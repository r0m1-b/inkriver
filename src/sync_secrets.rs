use crate::sync_segments::SyncGroupKey;
use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};
use std::fmt;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

const SECRET_SERVICE: &str = "io.github.r0m1-b.inkriver.sync";
const SECRET_USER: &str = "default";
const SECRET_FORMAT: &str = "inkriver-sync-secrets";
const SECRET_VERSION: u32 = 1;

/// Secrets required to open one synchronization group and its WebDAV transport.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct SyncSecrets {
    pub group_key: SyncGroupKey,
    pub webdav_password: Zeroizing<String>,
}

impl SyncSecrets {
    pub fn new(group_key: SyncGroupKey, webdav_password: String) -> Result<Self> {
        if webdav_password.is_empty() {
            bail!("Le secret WebDAV ne peut pas être vide");
        }
        Ok(Self {
            group_key,
            webdav_password: Zeroizing::new(webdav_password),
        })
    }
}

impl fmt::Debug for SyncSecrets {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SyncSecrets([REDACTED])")
    }
}

/// Platform-independent interface used by pairing and synchronization services.
pub trait SyncSecretStore: Send + Sync {
    fn save(&self, secrets: &SyncSecrets) -> Result<()>;
    fn load(&self) -> Result<Option<SyncSecrets>>;
    fn delete(&self) -> Result<()>;
}

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SecretWire {
    format: String,
    version: u32,
    group_key_base64: String,
    webdav_password: String,
}

fn encode_secrets(secrets: &SyncSecrets) -> Result<Zeroizing<String>> {
    let wire = SecretWire {
        format: SECRET_FORMAT.to_string(),
        version: SECRET_VERSION,
        group_key_base64: BASE64.encode(secrets.group_key.expose_bytes()),
        webdav_password: secrets.webdav_password.to_string(),
    };
    Ok(Zeroizing::new(
        serde_json::to_string(&wire).context("Impossible de sérialiser les secrets")?,
    ))
}

fn decode_secrets(encoded: &[u8]) -> Result<SyncSecrets> {
    let mut wire: SecretWire =
        serde_json::from_slice(encoded).context("Secrets de synchronisation invalides")?;
    if wire.format != SECRET_FORMAT || wire.version != SECRET_VERSION {
        wire.group_key_base64.zeroize();
        wire.webdav_password.zeroize();
        bail!("Version des secrets de synchronisation non prise en charge");
    }
    let decoded = BASE64
        .decode(&wire.group_key_base64)
        .context("Clé de groupe stockée invalide")?;
    wire.group_key_base64.zeroize();
    let key: [u8; 32] = decoded
        .try_into()
        .map_err(|_| anyhow::anyhow!("La clé de groupe stockée doit contenir 32 octets"))?;
    let password = std::mem::take(&mut wire.webdav_password);
    SyncSecrets::new(SyncGroupKey::from_bytes(key), password)
}

/// Native credential store backed by Secret Service on Linux and Android Keystore on Android.
#[cfg(any(target_os = "linux", target_os = "android"))]
#[derive(Debug, Default, Clone, Copy)]
pub struct PlatformSyncSecretStore;

#[cfg(any(target_os = "linux", target_os = "android"))]
impl PlatformSyncSecretStore {
    pub fn initialize() -> Result<Self> {
        initialize_native_store()?;
        Ok(Self)
    }

    fn entry(&self) -> Result<keyring_core::Entry> {
        keyring_core::Entry::new(SECRET_SERVICE, SECRET_USER)
            .context("Impossible d'accéder au coffre de secrets InkRiver")
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
impl SyncSecretStore for PlatformSyncSecretStore {
    fn save(&self, secrets: &SyncSecrets) -> Result<()> {
        let encoded = encode_secrets(secrets)?;
        self.entry()?
            .set_secret(encoded.as_bytes())
            .context("Impossible d'enregistrer les secrets de synchronisation")
    }

    fn load(&self) -> Result<Option<SyncSecrets>> {
        match self.entry()?.get_secret() {
            Ok(mut secret) => {
                let decoded = decode_secrets(&secret).map(Some);
                secret.zeroize();
                decoded
            }
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(error) => Err(error).context("Impossible de lire les secrets de synchronisation"),
        }
    }

    fn delete(&self) -> Result<()> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(error) => {
                Err(error).context("Impossible de supprimer les secrets de synchronisation")
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn initialize_native_store() -> Result<()> {
    let store = zbus_secret_service_keyring_store::Store::new()
        .context("Secret Service n'est pas disponible")?;
    keyring_core::set_default_store(store);
    Ok(())
}

#[cfg(target_os = "android")]
fn initialize_native_store() -> Result<()> {
    let store = android_native_keyring_store::Store::new()
        .context("Android Keystore n'est pas disponible")?;
    keyring_core::set_default_store(store);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemorySecretStore(Mutex<Option<Vec<u8>>>);

    impl SyncSecretStore for MemorySecretStore {
        fn save(&self, secrets: &SyncSecrets) -> Result<()> {
            *self.0.lock().unwrap() = Some(encode_secrets(secrets)?.as_bytes().to_vec());
            Ok(())
        }

        fn load(&self) -> Result<Option<SyncSecrets>> {
            self.0
                .lock()
                .unwrap()
                .as_deref()
                .map(decode_secrets)
                .transpose()
        }

        fn delete(&self) -> Result<()> {
            self.0.lock().unwrap().take();
            Ok(())
        }
    }

    #[test]
    fn secret_bundle_round_trips_and_is_redacted() {
        let store = MemorySecretStore::default();
        let secrets = SyncSecrets::new(
            SyncGroupKey::from_bytes([0x31; 32]),
            "webdav-secret".to_string(),
        )
        .unwrap();
        store.save(&secrets).unwrap();

        assert_eq!(store.load().unwrap(), Some(secrets.clone()));
        assert_eq!(format!("{secrets:?}"), "SyncSecrets([REDACTED])");
        store.delete().unwrap();
        assert_eq!(store.load().unwrap(), None);
    }

    #[test]
    fn malformed_and_empty_secrets_are_rejected() {
        assert!(SyncSecrets::new(SyncGroupKey::from_bytes([0; 32]), String::new()).is_err());
        assert!(decode_secrets(br#"{"format":"wrong"}"#).is_err());
    }
}
