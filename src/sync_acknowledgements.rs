use crate::storage::{Storage, SyncAcknowledgement};
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

const ACKNOWLEDGEMENT_FORMAT: &str = "inkriver-sync-acknowledgement";
const ACKNOWLEDGEMENT_VERSION: u32 = 1;
const ENCRYPTED_ACKNOWLEDGEMENT_FORMAT: &str = "inkriver-encrypted-sync-acknowledgement";
const ENCRYPTED_ACKNOWLEDGEMENT_VERSION: u32 = 1;
const NONCE_BYTES: usize = 24;
const MAX_ACKNOWLEDGED_SOURCES: usize = 256;
pub(crate) const ACKNOWLEDGEMENT_DIRECTORY: &str = "acknowledgements";
pub(crate) const MAX_ACKNOWLEDGEMENT_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AcknowledgedJournal {
    source_device_id: String,
    contiguous_sequence: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AcknowledgementDocument {
    format: String,
    format_version: u32,
    protocol_version: i64,
    observer_device_id: String,
    observed_at: String,
    journals: Vec<AcknowledgedJournal>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EncryptedAcknowledgementDocument {
    format: String,
    format_version: u32,
    protocol_version: i64,
    key_id: String,
    observer_device_id: String,
    nonce_base64: String,
    ciphertext_base64: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AcknowledgementAssociatedData<'a> {
    format: &'a str,
    format_version: u32,
    protocol_version: i64,
    key_id: &'a str,
    observer_device_id: &'a str,
}

pub(crate) struct PreparedAcknowledgement {
    pub relative_path: String,
    pub bytes: Vec<u8>,
}

pub(crate) async fn prepare_acknowledgement(
    storage: &Storage,
    key: &SyncGroupKey,
    observed_at: DateTime<Utc>,
) -> Result<PreparedAcknowledgement> {
    let key_id = key.key_id();
    let acknowledgements = storage
        .local_sync_acknowledgement_snapshot(&key_id, observed_at)
        .await?;
    let observer_device_id = storage.sync_identity().await?.device_id;
    let document = AcknowledgementDocument {
        format: ACKNOWLEDGEMENT_FORMAT.to_string(),
        format_version: ACKNOWLEDGEMENT_VERSION,
        protocol_version: SYNC_PROTOCOL_VERSION,
        observer_device_id: observer_device_id.clone(),
        observed_at: observed_at.to_rfc3339(),
        journals: acknowledgements
            .into_iter()
            .map(|acknowledgement| AcknowledgedJournal {
                source_device_id: acknowledgement.source_device_id,
                contiguous_sequence: acknowledgement.contiguous_sequence,
            })
            .collect(),
    };
    validate_document(&document)?;
    let encrypted = encrypt_document(&document, key)?;
    let mut bytes = serde_json::to_vec(&encrypted)
        .context("Impossible de sérialiser l'accusé de réception chiffré")?;
    bytes.push(b'\n');
    if bytes.len() > MAX_ACKNOWLEDGEMENT_BYTES {
        bail!("L'accusé de réception chiffré dépasse la taille autorisée");
    }
    Ok(PreparedAcknowledgement {
        relative_path: acknowledgement_path(&key_id, &observer_device_id),
        bytes,
    })
}

pub(crate) fn read_acknowledgement(
    relative_path: &str,
    bytes: &[u8],
    key: &SyncGroupKey,
) -> Result<Vec<SyncAcknowledgement>> {
    if bytes.len() > MAX_ACKNOWLEDGEMENT_BYTES {
        bail!("L'accusé de réception chiffré dépasse la taille autorisée");
    }
    let encrypted: EncryptedAcknowledgementDocument =
        serde_json::from_slice(bytes).context("Accusé de réception chiffré mal formé")?;
    validate_encrypted_header(&encrypted, &key.key_id())?;
    if relative_path != acknowledgement_path(&encrypted.key_id, &encrypted.observer_device_id) {
        bail!("Le chemin de l'accusé ne correspond pas à ses métadonnées");
    }
    let document = decrypt_document(&encrypted, key)?;
    validate_document(&document)?;
    if document.observer_device_id != encrypted.observer_device_id {
        bail!("L'observateur de l'accusé chiffré est incohérent");
    }
    let observed_at = DateTime::parse_from_rfc3339(&document.observed_at)
        .context("Date d'accusé de réception invalide")?
        .with_timezone(&Utc);
    Ok(document
        .journals
        .into_iter()
        .map(|journal| SyncAcknowledgement {
            key_id: encrypted.key_id.clone(),
            observer_device_id: document.observer_device_id.clone(),
            source_device_id: journal.source_device_id,
            contiguous_sequence: journal.contiguous_sequence,
            observed_at,
        })
        .collect())
}

pub(crate) fn acknowledgement_path(key_id: &str, observer_device_id: &str) -> String {
    format!("v2/{key_id}/{ACKNOWLEDGEMENT_DIRECTORY}/{observer_device_id}.json")
}

fn validate_document(document: &AcknowledgementDocument) -> Result<()> {
    if document.format != ACKNOWLEDGEMENT_FORMAT
        || document.format_version != ACKNOWLEDGEMENT_VERSION
        || document.protocol_version != SYNC_PROTOCOL_VERSION
        || uuid::Uuid::parse_str(&document.observer_device_id).is_err()
        || document.journals.is_empty()
        || document.journals.len() > MAX_ACKNOWLEDGED_SOURCES
    {
        bail!("Accusé de réception de synchronisation invalide");
    }
    DateTime::parse_from_rfc3339(&document.observed_at)
        .context("Date d'accusé de réception invalide")?;
    let mut previous = None;
    for journal in &document.journals {
        uuid::Uuid::parse_str(&journal.source_device_id)
            .context("Identifiant de journal acquitté invalide")?;
        if journal.contiguous_sequence < 0
            || previous.is_some_and(|value: &str| value >= journal.source_device_id.as_str())
        {
            bail!("Liste de journaux acquittés invalide");
        }
        previous = Some(journal.source_device_id.as_str());
    }
    Ok(())
}

fn associated_data(document: &EncryptedAcknowledgementDocument) -> Result<Vec<u8>> {
    serde_json::to_vec(&AcknowledgementAssociatedData {
        format: &document.format,
        format_version: document.format_version,
        protocol_version: document.protocol_version,
        key_id: &document.key_id,
        observer_device_id: &document.observer_device_id,
    })
    .context("Impossible de sérialiser les métadonnées de l'accusé")
}

fn encrypt_document(
    document: &AcknowledgementDocument,
    key: &SyncGroupKey,
) -> Result<EncryptedAcknowledgementDocument> {
    let mut nonce = [0_u8; NONCE_BYTES];
    getrandom::fill(&mut nonce).context("Impossible de générer le nonce de l'accusé")?;
    let mut encrypted = EncryptedAcknowledgementDocument {
        format: ENCRYPTED_ACKNOWLEDGEMENT_FORMAT.to_string(),
        format_version: ENCRYPTED_ACKNOWLEDGEMENT_VERSION,
        protocol_version: SYNC_PROTOCOL_VERSION,
        key_id: key.key_id(),
        observer_device_id: document.observer_device_id.clone(),
        nonce_base64: BASE64.encode(nonce),
        ciphertext_base64: String::new(),
    };
    let plaintext =
        Zeroizing::new(serde_json::to_vec(document).context("Impossible de sérialiser l'accusé")?);
    let cipher = XChaCha20Poly1305::new((&key.expose_bytes()).into());
    encrypted.ciphertext_base64 = BASE64.encode(
        cipher
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: &plaintext,
                    aad: &associated_data(&encrypted)?,
                },
            )
            .map_err(|_| anyhow::anyhow!("Impossible de chiffrer l'accusé"))?,
    );
    Ok(encrypted)
}

fn decrypt_document(
    encrypted: &EncryptedAcknowledgementDocument,
    key: &SyncGroupKey,
) -> Result<AcknowledgementDocument> {
    let nonce: [u8; NONCE_BYTES] = BASE64
        .decode(&encrypted.nonce_base64)
        .context("Nonce d'accusé invalide")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("Taille de nonce d'accusé invalide"))?;
    let ciphertext = BASE64
        .decode(&encrypted.ciphertext_base64)
        .context("Contenu chiffré d'accusé invalide")?;
    let plaintext = Zeroizing::new(
        XChaCha20Poly1305::new((&key.expose_bytes()).into())
            .decrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: &ciphertext,
                    aad: &associated_data(encrypted)?,
                },
            )
            .map_err(|_| anyhow::anyhow!("Échec d'authentification de l'accusé"))?,
    );
    serde_json::from_slice(&plaintext).context("Accusé de réception déchiffré mal formé")
}

fn validate_encrypted_header(
    document: &EncryptedAcknowledgementDocument,
    key_id: &str,
) -> Result<()> {
    if document.format != ENCRYPTED_ACKNOWLEDGEMENT_FORMAT
        || document.format_version != ENCRYPTED_ACKNOWLEDGEMENT_VERSION
        || document.protocol_version != SYNC_PROTOCOL_VERSION
        || document.key_id != key_id
        || uuid::Uuid::parse_str(&document.observer_device_id).is_err()
    {
        bail!("En-tête d'accusé de réception chiffré invalide");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn encrypted_acknowledgement_round_trips_without_exposing_identifiers() {
        let storage = Storage::open_in_memory().await.unwrap();
        storage.enable_sync().await.unwrap();
        let key = SyncGroupKey::from_bytes([0x91; 32]);
        let observed_at = DateTime::parse_from_rfc3339("2026-08-29T08:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let prepared = prepare_acknowledgement(&storage, &key, observed_at)
            .await
            .unwrap();
        let identity = storage.sync_identity().await.unwrap();
        let visible = String::from_utf8_lossy(&prepared.bytes);
        assert!(!visible.contains(&observed_at.to_rfc3339()));

        let acknowledgements =
            read_acknowledgement(&prepared.relative_path, &prepared.bytes, &key).unwrap();
        assert_eq!(acknowledgements.len(), 1);
        assert_eq!(acknowledgements[0].observer_device_id, identity.device_id);
        assert_eq!(acknowledgements[0].source_device_id, identity.device_id);
        assert_eq!(acknowledgements[0].contiguous_sequence, 0);
    }

    #[tokio::test]
    async fn acknowledgement_rejects_another_key_and_a_mismatched_path() {
        let storage = Storage::open_in_memory().await.unwrap();
        storage.enable_sync().await.unwrap();
        let key = SyncGroupKey::from_bytes([0x92; 32]);
        let prepared = prepare_acknowledgement(&storage, &key, Utc::now())
            .await
            .unwrap();
        assert!(
            read_acknowledgement(
                &prepared.relative_path,
                &prepared.bytes,
                &SyncGroupKey::from_bytes([0x93; 32]),
            )
            .is_err()
        );
        assert!(read_acknowledgement("v2/wrong/path.json", &prepared.bytes, &key).is_err());
    }
}
