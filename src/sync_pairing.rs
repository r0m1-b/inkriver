use crate::storage::{Storage, SyncConfiguration};
use crate::sync_secrets::{SyncSecretStore, SyncSecrets};
use crate::sync_segments::SyncGroupKey;
use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use qrcode::QrCode;
use qrcode::render::svg;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::fmt;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

const PAIRING_FORMAT: &str = "inkriver-device-pairing";
const PAIRING_VERSION: u32 = 1;
const PAIRING_URI_PREFIX: &str = "inkriver://pair/";
const MAX_PAIRING_URI_BYTES: usize = 8 * 1024;
const MAX_USERNAME_BYTES: usize = 512;
const MAX_DEVICE_NAME_BYTES: usize = 120;

/// Creates the first synchronization group on an already configured device.
pub async fn configure_new_sync_group<S: SyncSecretStore>(
    storage: &Storage,
    secret_store: &S,
    webdav_base_url: &str,
    webdav_username: &str,
    webdav_password: String,
    device_name: &str,
    observed_at: chrono::DateTime<chrono::Utc>,
) -> Result<SyncGroupKey> {
    if storage.sync_configuration().await?.is_some() || secret_store.load()?.is_some() {
        bail!("Une configuration de synchronisation existe déjà");
    }
    let base_url = normalize_webdav_base_url(webdav_base_url)?;
    validate_text(
        "nom d'utilisateur WebDAV",
        webdav_username,
        MAX_USERNAME_BYTES,
    )?;
    validate_text("nom de l'appareil", device_name, MAX_DEVICE_NAME_BYTES)?;
    let key = SyncGroupKey::generate()?;
    let secrets = SyncSecrets::new(key.clone(), webdav_password)?;
    secret_store.save(&secrets)?;
    let configuration = SyncConfiguration {
        webdav_base_url: base_url,
        webdav_username: webdav_username.to_string(),
        key_id: key.key_id(),
    };
    if let Err(error) = storage.save_sync_configuration(&configuration).await {
        let _ = secret_store.delete();
        return Err(error);
    }
    let local_id = storage.sync_identity().await?.device_id;
    if let Err(error) = storage
        .rename_sync_device(&local_id, device_name, observed_at)
        .await
    {
        let _ = storage.clear_sync_configuration().await;
        let _ = secret_store.delete();
        return Err(error);
    }
    if let Err(error) = storage.enable_sync().await {
        let _ = storage.clear_sync_configuration().await;
        let _ = secret_store.delete();
        return Err(error);
    }
    Ok(key)
}

/// Builds an invitation from the current non-secret settings and native vault.
pub async fn create_pairing_invitation<S: SyncSecretStore>(
    storage: &Storage,
    secret_store: &S,
) -> Result<PairingInvitation> {
    let configuration = storage
        .sync_configuration()
        .await?
        .context("La synchronisation n'est pas configurée")?;
    let secrets = secret_store
        .load()?
        .context("Les secrets de synchronisation sont absents")?;
    if secrets.group_key.key_id() != configuration.key_id {
        bail!("La clé sécurisée ne correspond pas à la configuration SQLite");
    }
    let identity = storage.sync_identity().await?;
    let local = storage
        .list_sync_devices()
        .await?
        .into_iter()
        .find(|device| device.device_id == identity.device_id && device.is_local)
        .context("L'appareil local n'est pas enregistré")?;
    Ok(PairingInvitation {
        webdav_base_url: configuration.webdav_base_url,
        webdav_username: configuration.webdav_username,
        group_key: secrets.group_key.clone(),
        inviter_device_id: identity.device_id,
        inviter_device_name: local.display_name,
    })
}

/// Imports an invitation and the separately supplied WebDAV password.
pub async fn accept_pairing_invitation<S: SyncSecretStore>(
    storage: &Storage,
    secret_store: &S,
    encoded_invitation: &str,
    webdav_password: String,
    local_device_name: &str,
    observed_at: chrono::DateTime<chrono::Utc>,
) -> Result<()> {
    if storage.sync_configuration().await?.is_some() || secret_store.load()?.is_some() {
        bail!("Une configuration de synchronisation existe déjà");
    }
    validate_text(
        "nom de l'appareil",
        local_device_name,
        MAX_DEVICE_NAME_BYTES,
    )?;
    let invitation = decode_pairing_invitation(encoded_invitation)?;
    let secrets = SyncSecrets::new(invitation.group_key.clone(), webdav_password)?;
    secret_store.save(&secrets)?;
    let configuration = SyncConfiguration {
        webdav_base_url: invitation.webdav_base_url.clone(),
        webdav_username: invitation.webdav_username.clone(),
        key_id: invitation.group_key.key_id(),
    };
    if let Err(error) = storage.save_sync_configuration(&configuration).await {
        let _ = secret_store.delete();
        return Err(error);
    }
    let local_id = storage.sync_identity().await?.device_id;
    if let Err(error) = storage
        .rename_sync_device(&local_id, local_device_name, observed_at)
        .await
    {
        let _ = storage.clear_sync_configuration().await;
        let _ = secret_store.delete();
        return Err(error);
    }
    if let Err(error) = storage
        .register_sync_device(
            &invitation.inviter_device_id,
            &invitation.inviter_device_name,
            observed_at,
        )
        .await
    {
        let _ = storage.clear_sync_configuration().await;
        let _ = secret_store.delete();
        return Err(error);
    }
    if let Err(error) = storage.enable_sync().await {
        let _ = storage.clear_sync_configuration().await;
        let _ = secret_store.delete();
        return Err(error);
    }
    Ok(())
}

/// Transient information transferred from an existing device to a new one.
///
/// The WebDAV password is deliberately absent. The group key is secret and the
/// encoded invitation must only be displayed during an explicit pairing flow.
#[derive(Clone, PartialEq, Eq)]
pub struct PairingInvitation {
    pub webdav_base_url: String,
    pub webdav_username: String,
    pub group_key: SyncGroupKey,
    pub inviter_device_id: String,
    pub inviter_device_name: String,
}

impl fmt::Debug for PairingInvitation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairingInvitation")
            .field("webdav_base_url", &self.webdav_base_url)
            .field("webdav_username", &self.webdav_username)
            .field("group_key", &"[REDACTED]")
            .field("inviter_device_id", &self.inviter_device_id)
            .field("inviter_device_name", &self.inviter_device_name)
            .finish()
    }
}

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PairingWire {
    format: String,
    version: u32,
    webdav_base_url: String,
    webdav_username: String,
    group_key_base64: String,
    inviter_device_id: String,
    inviter_device_name: String,
}

/// Encodes a versioned invitation suitable for QR-code transfer.
pub fn encode_pairing_invitation(invitation: &PairingInvitation) -> Result<String> {
    validate_invitation(invitation)?;
    let mut key_base64 = URL_SAFE_NO_PAD.encode(invitation.group_key.expose_bytes());
    let wire = PairingWire {
        format: PAIRING_FORMAT.to_string(),
        version: PAIRING_VERSION,
        webdav_base_url: invitation.webdav_base_url.clone(),
        webdav_username: invitation.webdav_username.clone(),
        group_key_base64: key_base64.clone(),
        inviter_device_id: invitation.inviter_device_id.clone(),
        inviter_device_name: invitation.inviter_device_name.clone(),
    };
    let json =
        Zeroizing::new(serde_json::to_vec(&wire).context("Impossible de sérialiser l'appairage")?);
    key_base64.zeroize();
    let encoded = URL_SAFE_NO_PAD.encode(json);
    let uri = format!("{PAIRING_URI_PREFIX}{encoded}");
    if uri.len() > MAX_PAIRING_URI_BYTES {
        bail!("La configuration d'appairage est trop volumineuse");
    }
    Ok(uri)
}

/// Decodes and strictly validates an invitation scanned or imported by a new device.
pub fn decode_pairing_invitation(uri: &str) -> Result<PairingInvitation> {
    if uri.len() > MAX_PAIRING_URI_BYTES {
        bail!("La configuration d'appairage est trop volumineuse");
    }
    let encoded = uri
        .strip_prefix(PAIRING_URI_PREFIX)
        .context("Format d'appairage InkRiver invalide")?;
    let json = Zeroizing::new(
        URL_SAFE_NO_PAD
            .decode(encoded)
            .context("Encodage d'appairage invalide")?,
    );
    let mut wire: PairingWire =
        serde_json::from_slice(&json).context("Configuration d'appairage invalide")?;
    if wire.format != PAIRING_FORMAT || wire.version != PAIRING_VERSION {
        wire.group_key_base64.zeroize();
        bail!("Version d'appairage InkRiver non prise en charge");
    }
    let decoded_key = URL_SAFE_NO_PAD
        .decode(&wire.group_key_base64)
        .context("Clé de groupe invalide")?;
    wire.group_key_base64.zeroize();
    let key_bytes: [u8; 32] = decoded_key
        .try_into()
        .map_err(|_| anyhow::anyhow!("La clé de groupe doit contenir exactement 32 octets"))?;
    let invitation = PairingInvitation {
        webdav_base_url: std::mem::take(&mut wire.webdav_base_url),
        webdav_username: std::mem::take(&mut wire.webdav_username),
        group_key: SyncGroupKey::from_bytes(key_bytes),
        inviter_device_id: std::mem::take(&mut wire.inviter_device_id),
        inviter_device_name: std::mem::take(&mut wire.inviter_device_name),
    };
    validate_invitation(&invitation)?;
    Ok(invitation)
}

/// Renders an invitation as a self-contained SVG QR code without network access.
pub fn render_pairing_qr_svg(uri: &str) -> Result<String> {
    decode_pairing_invitation(uri)?;
    let code = QrCode::new(uri.as_bytes()).context("Impossible de générer le QR code")?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(320, 320)
        .dark_color(svg::Color("#17352f"))
        .light_color(svg::Color("#ffffff"))
        .build())
}

fn validate_invitation(invitation: &PairingInvitation) -> Result<()> {
    if invitation.webdav_base_url != normalize_webdav_base_url(&invitation.webdav_base_url)? {
        bail!("L'URL WebDAV d'appairage n'est pas normalisée");
    }
    validate_text(
        "nom d'utilisateur WebDAV",
        &invitation.webdav_username,
        MAX_USERNAME_BYTES,
    )?;
    validate_text(
        "nom de l'appareil",
        &invitation.inviter_device_name,
        MAX_DEVICE_NAME_BYTES,
    )?;
    uuid::Uuid::parse_str(&invitation.inviter_device_id)
        .context("Identifiant d'appareil invalide")?;
    Ok(())
}

fn normalize_webdav_base_url(value: &str) -> Result<String> {
    let mut url = Url::parse(value).context("URL WebDAV invalide")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!(
            "L'URL WebDAV doit être HTTP(S) et ne contenir ni identifiants, ni requête, ni fragment"
        );
    }
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url.to_string())
}

fn validate_text(label: &str, value: &str, max_bytes: usize) -> Result<()> {
    if value.trim().is_empty() || value != value.trim() || value.len() > max_bytes {
        bail!("{label} invalide");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemorySecrets(Mutex<Option<SyncSecrets>>);

    impl SyncSecretStore for MemorySecrets {
        fn save(&self, secrets: &SyncSecrets) -> Result<()> {
            *self.0.lock().unwrap() = Some(secrets.clone());
            Ok(())
        }

        fn load(&self) -> Result<Option<SyncSecrets>> {
            Ok(self.0.lock().unwrap().clone())
        }

        fn delete(&self) -> Result<()> {
            self.0.lock().unwrap().take();
            Ok(())
        }
    }

    fn invitation() -> PairingInvitation {
        PairingInvitation {
            webdav_base_url: "https://dav.example.test/inkriver/".to_string(),
            webdav_username: "romain".to_string(),
            group_key: SyncGroupKey::from_bytes([0x42; 32]),
            inviter_device_id: "00000000-0000-4000-8000-000000000042".to_string(),
            inviter_device_name: "Laptop Linux".to_string(),
        }
    }

    #[test]
    fn invitation_round_trips_without_webdav_password() {
        let invitation = invitation();
        let encoded = encode_pairing_invitation(&invitation).unwrap();
        let decoded = decode_pairing_invitation(&encoded).unwrap();

        assert_eq!(decoded, invitation);
        assert!(!encoded.contains("password"));
        assert!(!format!("{decoded:?}").contains("QkJC"));
    }

    #[test]
    fn qr_svg_is_self_contained() {
        let encoded = encode_pairing_invitation(&invitation()).unwrap();
        let svg = render_pairing_qr_svg(&encoded).unwrap();

        assert!(svg.starts_with("<?xml"));
        assert!(svg.contains("<svg"));
        assert!(!svg.contains("<image"));
        assert!(!svg.contains("href="));
    }

    #[test]
    fn invalid_or_oversized_payloads_are_rejected() {
        assert!(decode_pairing_invitation("https://example.test").is_err());
        assert!(
            decode_pairing_invitation(&format!("{PAIRING_URI_PREFIX}{}", "a".repeat(9_000)))
                .is_err()
        );

        let mut invalid = invitation();
        invalid.webdav_base_url = "https://user:secret@example.test/dav/".to_string();
        assert!(encode_pairing_invitation(&invalid).is_err());
    }

    #[test]
    fn unknown_fields_and_versions_are_rejected() {
        let encoded = encode_pairing_invitation(&invitation()).unwrap();
        let json = URL_SAFE_NO_PAD
            .decode(encoded.strip_prefix(PAIRING_URI_PREFIX).unwrap())
            .unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&json).unwrap();
        value["version"] = serde_json::json!(99);
        let unsupported = format!(
            "{PAIRING_URI_PREFIX}{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&value).unwrap())
        );
        assert!(decode_pairing_invitation(&unsupported).is_err());

        value["version"] = serde_json::json!(1);
        value["unexpected"] = serde_json::json!(true);
        let unknown = format!(
            "{PAIRING_URI_PREFIX}{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&value).unwrap())
        );
        assert!(decode_pairing_invitation(&unknown).is_err());
    }

    #[tokio::test]
    async fn fresh_device_joins_from_an_invitation_with_a_separate_password() {
        let observed_at = chrono::DateTime::parse_from_rfc3339("2026-08-28T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let linux = Storage::open_in_memory().await.unwrap();
        let linux_secrets = MemorySecrets::default();
        let key = configure_new_sync_group(
            &linux,
            &linux_secrets,
            "https://dav.example.test/inkriver",
            "romain",
            "linux-password".to_string(),
            "Laptop Linux",
            observed_at,
        )
        .await
        .unwrap();
        let invitation = create_pairing_invitation(&linux, &linux_secrets)
            .await
            .unwrap();
        let encoded = encode_pairing_invitation(&invitation).unwrap();

        let android = Storage::open_in_memory().await.unwrap();
        let android_secrets = MemorySecrets::default();
        accept_pairing_invitation(
            &android,
            &android_secrets,
            &encoded,
            "phone-password".to_string(),
            "Téléphone Android",
            observed_at,
        )
        .await
        .unwrap();

        let config = android.sync_configuration().await.unwrap().unwrap();
        assert_eq!(config.webdav_base_url, "https://dav.example.test/inkriver/");
        assert_eq!(config.webdav_username, "romain");
        assert_eq!(config.key_id, key.key_id());
        let stored = android_secrets.load().unwrap().unwrap();
        assert_eq!(stored.group_key, key);
        assert_eq!(stored.webdav_password.as_str(), "phone-password");
        assert!(android.sync_identity().await.unwrap().is_enabled);
        let devices = android.list_sync_devices().await.unwrap();
        assert_eq!(devices.len(), 2);
        assert!(
            devices
                .iter()
                .any(|device| { device.is_local && device.display_name == "Téléphone Android" })
        );
        assert!(
            devices
                .iter()
                .any(|device| !device.is_local && device.display_name == "Laptop Linux")
        );
    }

    #[tokio::test]
    async fn pairing_refuses_to_replace_an_existing_configuration() {
        let observed_at = chrono::Utc::now();
        let storage = Storage::open_in_memory().await.unwrap();
        let secrets = MemorySecrets::default();
        configure_new_sync_group(
            &storage,
            &secrets,
            "https://dav.example.test/root/",
            "user",
            "password".to_string(),
            "Linux",
            observed_at,
        )
        .await
        .unwrap();

        assert!(
            accept_pairing_invitation(
                &storage,
                &secrets,
                &encode_pairing_invitation(&invitation()).unwrap(),
                "replacement".to_string(),
                "Other",
                observed_at,
            )
            .await
            .is_err()
        );
        assert_eq!(
            secrets.load().unwrap().unwrap().webdav_password.as_str(),
            "password"
        );
    }
}
