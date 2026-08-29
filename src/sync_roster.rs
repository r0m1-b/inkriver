use crate::storage::{Storage, SyncRosterMember};
use crate::sync::SYNC_PROTOCOL_VERSION;
use crate::sync_segments::SyncGroupKey;
use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

const ROSTER_FORMAT: &str = "inkriver-sync-roster";
const ROSTER_VERSION: u32 = 1;
const ENCRYPTED_ROSTER_FORMAT: &str = "inkriver-encrypted-sync-roster";
const ENCRYPTED_ROSTER_VERSION: u32 = 1;
const NONCE_BYTES: usize = 24;
const MAX_ROSTER_MEMBERS: usize = 256;
pub(crate) const ROSTER_DIRECTORY: &str = "rosters";
pub(crate) const MAX_ROSTER_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RosterMemberDocument {
    device_id: String,
    revoked_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RosterDocument {
    format: String,
    format_version: u32,
    protocol_version: i64,
    publisher_device_id: String,
    observed_at: String,
    members: Vec<RosterMemberDocument>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EncryptedRosterDocument {
    format: String,
    format_version: u32,
    protocol_version: i64,
    key_id: String,
    publisher_device_id: String,
    nonce_base64: String,
    ciphertext_base64: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RosterAssociatedData<'a> {
    format: &'a str,
    format_version: u32,
    protocol_version: i64,
    key_id: &'a str,
    publisher_device_id: &'a str,
}

pub(crate) struct PreparedRoster {
    pub relative_path: String,
    pub bytes: Vec<u8>,
}

pub(crate) struct ReadRoster {
    pub members: Vec<SyncRosterMember>,
}

pub(crate) async fn prepare_roster(
    storage: &Storage,
    key: &SyncGroupKey,
    observed_at: DateTime<Utc>,
) -> Result<PreparedRoster> {
    let key_id = key.key_id();
    storage.seed_sync_roster(&key_id, observed_at).await?;
    let identity = storage.sync_identity().await?;
    let members = storage.sync_roster_members(&key_id).await?;
    let document = RosterDocument {
        format: ROSTER_FORMAT.to_string(),
        format_version: ROSTER_VERSION,
        protocol_version: SYNC_PROTOCOL_VERSION,
        publisher_device_id: identity.device_id.clone(),
        observed_at: observed_at.to_rfc3339(),
        members: members
            .into_iter()
            .map(|member| RosterMemberDocument {
                device_id: member.device_id,
                revoked_at: member.revoked_at.map(|date| date.to_rfc3339()),
            })
            .collect(),
    };
    validate_document(&document)?;
    let encrypted = encrypt_document(&document, key)?;
    let mut bytes =
        serde_json::to_vec(&encrypted).context("Impossible de sérialiser le registre chiffré")?;
    bytes.push(b'\n');
    if bytes.len() > MAX_ROSTER_BYTES {
        bail!("Le registre chiffré dépasse la taille autorisée");
    }
    Ok(PreparedRoster {
        relative_path: roster_path(&key_id, &identity.device_id),
        bytes,
    })
}

pub(crate) fn read_roster(
    relative_path: &str,
    bytes: &[u8],
    key: &SyncGroupKey,
) -> Result<ReadRoster> {
    if bytes.len() > MAX_ROSTER_BYTES {
        bail!("Le registre chiffré dépasse la taille autorisée");
    }
    let encrypted: EncryptedRosterDocument =
        serde_json::from_slice(bytes).context("Registre chiffré mal formé")?;
    validate_encrypted_header(&encrypted, &key.key_id())?;
    if relative_path != roster_path(&encrypted.key_id, &encrypted.publisher_device_id) {
        bail!("Le chemin du registre ne correspond pas à ses métadonnées");
    }
    let document = decrypt_document(&encrypted, key)?;
    validate_document(&document)?;
    if document.publisher_device_id != encrypted.publisher_device_id {
        bail!("L'auteur du registre chiffré est incohérent");
    }
    Ok(ReadRoster {
        members: document
            .members
            .into_iter()
            .map(|member| {
                Ok(SyncRosterMember {
                    device_id: member.device_id,
                    revoked_at: member
                        .revoked_at
                        .map(|value| {
                            DateTime::parse_from_rfc3339(&value)
                                .map(|date| date.with_timezone(&Utc))
                                .context("Date de révocation du registre invalide")
                        })
                        .transpose()?,
                })
            })
            .collect::<Result<Vec<_>>>()?,
    })
}

pub(crate) fn roster_path(key_id: &str, publisher_device_id: &str) -> String {
    format!("v2/{key_id}/{ROSTER_DIRECTORY}/{publisher_device_id}.json")
}

fn validate_document(document: &RosterDocument) -> Result<()> {
    if document.format != ROSTER_FORMAT
        || document.format_version != ROSTER_VERSION
        || document.protocol_version != SYNC_PROTOCOL_VERSION
        || uuid::Uuid::parse_str(&document.publisher_device_id).is_err()
        || document.members.is_empty()
        || document.members.len() > MAX_ROSTER_MEMBERS
    {
        bail!("Registre de synchronisation invalide");
    }
    DateTime::parse_from_rfc3339(&document.observed_at)
        .context("Date d'observation du registre invalide")?;
    let mut previous = None;
    let mut contains_publisher = false;
    for member in &document.members {
        uuid::Uuid::parse_str(&member.device_id)
            .context("Identifiant d'appareil du registre invalide")?;
        if previous.is_some_and(|value: &str| value >= member.device_id.as_str()) {
            bail!("Liste d'appareils du registre invalide");
        }
        if let Some(revoked_at) = &member.revoked_at {
            DateTime::parse_from_rfc3339(revoked_at)
                .context("Date de révocation du registre invalide")?;
        }
        contains_publisher |= member.device_id == document.publisher_device_id;
        previous = Some(member.device_id.as_str());
    }
    if !contains_publisher {
        bail!("L'auteur est absent du registre de synchronisation");
    }
    Ok(())
}

fn associated_data(document: &EncryptedRosterDocument) -> Result<Vec<u8>> {
    serde_json::to_vec(&RosterAssociatedData {
        format: &document.format,
        format_version: document.format_version,
        protocol_version: document.protocol_version,
        key_id: &document.key_id,
        publisher_device_id: &document.publisher_device_id,
    })
    .context("Impossible de sérialiser les métadonnées du registre")
}

fn encrypt_document(
    document: &RosterDocument,
    key: &SyncGroupKey,
) -> Result<EncryptedRosterDocument> {
    let mut nonce = [0_u8; NONCE_BYTES];
    getrandom::fill(&mut nonce).context("Impossible de générer le nonce du registre")?;
    let mut encrypted = EncryptedRosterDocument {
        format: ENCRYPTED_ROSTER_FORMAT.to_string(),
        format_version: ENCRYPTED_ROSTER_VERSION,
        protocol_version: SYNC_PROTOCOL_VERSION,
        key_id: key.key_id(),
        publisher_device_id: document.publisher_device_id.clone(),
        nonce_base64: BASE64.encode(nonce),
        ciphertext_base64: String::new(),
    };
    let plaintext = Zeroizing::new(
        serde_json::to_vec(document).context("Impossible de sérialiser le registre")?,
    );
    encrypted.ciphertext_base64 = BASE64.encode(
        XChaCha20Poly1305::new((&key.expose_bytes()).into())
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: &plaintext,
                    aad: &associated_data(&encrypted)?,
                },
            )
            .map_err(|_| anyhow::anyhow!("Impossible de chiffrer le registre"))?,
    );
    Ok(encrypted)
}

fn decrypt_document(
    encrypted: &EncryptedRosterDocument,
    key: &SyncGroupKey,
) -> Result<RosterDocument> {
    let nonce: [u8; NONCE_BYTES] = BASE64
        .decode(&encrypted.nonce_base64)
        .context("Nonce de registre invalide")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("Taille de nonce de registre invalide"))?;
    let ciphertext = BASE64
        .decode(&encrypted.ciphertext_base64)
        .context("Contenu chiffré de registre invalide")?;
    let plaintext = Zeroizing::new(
        XChaCha20Poly1305::new((&key.expose_bytes()).into())
            .decrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: &ciphertext,
                    aad: &associated_data(encrypted)?,
                },
            )
            .map_err(|_| anyhow::anyhow!("Échec d'authentification du registre"))?,
    );
    serde_json::from_slice(&plaintext).context("Registre déchiffré mal formé")
}

fn validate_encrypted_header(document: &EncryptedRosterDocument, key_id: &str) -> Result<()> {
    if document.format != ENCRYPTED_ROSTER_FORMAT
        || document.format_version != ENCRYPTED_ROSTER_VERSION
        || document.protocol_version != SYNC_PROTOCOL_VERSION
        || document.key_id != key_id
        || uuid::Uuid::parse_str(&document.publisher_device_id).is_err()
    {
        bail!("En-tête de registre chiffré invalide");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn encrypted_roster_round_trips_without_exposing_members() {
        let storage = Storage::open_in_memory().await.unwrap();
        storage.enable_sync().await.unwrap();
        let remote = "00000000-0000-4000-8000-000000000031";
        storage
            .register_sync_device(remote, "Téléphone privé", Utc::now())
            .await
            .unwrap();
        let key = SyncGroupKey::from_bytes([0xa1; 32]);
        let prepared = prepare_roster(&storage, &key, Utc::now()).await.unwrap();
        let visible = String::from_utf8_lossy(&prepared.bytes);
        assert!(!visible.contains(remote));
        assert!(!visible.contains("Téléphone privé"));

        let read = read_roster(&prepared.relative_path, &prepared.bytes, &key).unwrap();
        assert_eq!(read.members.len(), 2);
        let local_id = storage.sync_identity().await.unwrap().device_id;
        assert!(
            read.members
                .iter()
                .any(|member| member.device_id == local_id)
        );
    }

    #[tokio::test]
    async fn roster_rejects_wrong_key_corruption_and_mismatched_path() {
        let storage = Storage::open_in_memory().await.unwrap();
        storage.enable_sync().await.unwrap();
        let key = SyncGroupKey::from_bytes([0xa2; 32]);
        let prepared = prepare_roster(&storage, &key, Utc::now()).await.unwrap();
        assert!(
            read_roster(
                &prepared.relative_path,
                &prepared.bytes,
                &SyncGroupKey::from_bytes([0xa3; 32])
            )
            .is_err()
        );
        assert!(read_roster("v2/wrong/rosters/device.json", &prepared.bytes, &key).is_err());
        let mut corrupted = prepared.bytes;
        let index = corrupted.len() / 2;
        corrupted[index] ^= 1;
        assert!(read_roster(&prepared.relative_path, &corrupted, &key).is_err());
    }
}
