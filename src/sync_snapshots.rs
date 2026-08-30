use crate::storage::Storage;
use crate::sync::{SYNC_PROTOCOL_VERSION, SyncEvent, SyncImportReport};
use crate::sync_segments::SyncGroupKey;
use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

const SNAPSHOT_FORMAT: &str = "inkriver-sync-snapshot";
const LEGACY_SNAPSHOT_VERSION: u32 = 1;
const SNAPSHOT_VERSION: u32 = 2;
const ENCRYPTED_SNAPSHOT_FORMAT: &str = "inkriver-encrypted-sync-snapshot";
const ENCRYPTED_SNAPSHOT_VERSION: u32 = 1;
const NONCE_BYTES: usize = 24;
pub(crate) const SNAPSHOT_DIRECTORY: &str = "snapshots";
pub(crate) const MAX_SNAPSHOT_BYTES: usize = 8 * 1024 * 1024;
const MAX_SNAPSHOT_STATE_BYTES: usize = 5 * 1024 * 1024;
pub(crate) const MAX_SNAPSHOT_EVENTS: usize = 10_000;
pub(crate) const MAX_SNAPSHOT_DEVICES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SnapshotFrontier {
    device_id: String,
    contiguous_sequence: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SnapshotState {
    protocol_version: i64,
    frontiers: Vec<SnapshotFrontier>,
    events: Vec<SyncEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SnapshotDocument {
    format: String,
    format_version: u32,
    created_at: String,
    state: SnapshotState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EncryptedSnapshotDocument {
    format: String,
    format_version: u32,
    protocol_version: i64,
    key_id: String,
    creator_device_id: String,
    state_hash: String,
    nonce_base64: String,
    ciphertext_base64: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotAssociatedData<'a> {
    format: &'a str,
    format_version: u32,
    protocol_version: i64,
    key_id: &'a str,
    creator_device_id: &'a str,
    state_hash: &'a str,
}

pub(crate) struct PreparedSnapshot {
    pub relative_path: String,
    pub bytes: Vec<u8>,
    pub key_id: String,
    pub creator_device_id: String,
    pub state_hash: String,
    pub frontiers: Vec<(String, i64)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SnapshotUnavailableReason {
    TooManyRetainedEvents,
    StateTooLarge,
    EncryptedDocumentTooLarge,
}

pub(crate) enum SnapshotPreparation {
    Ready(PreparedSnapshot),
    ConfirmedUnchanged(PreparedSnapshot),
    Unavailable(SnapshotUnavailableReason),
}

#[cfg(test)]
impl SnapshotPreparation {
    fn expect_ready(self) -> PreparedSnapshot {
        match self {
            Self::Ready(snapshot) => snapshot,
            Self::ConfirmedUnchanged(_) => panic!("expected a new snapshot, got unchanged state"),
            Self::Unavailable(reason) => panic!("expected a new snapshot, got {reason:?}"),
        }
    }
}

#[derive(Clone, Copy)]
struct SnapshotLimits {
    events: usize,
    state_bytes: usize,
    encrypted_bytes: usize,
}

const DEFAULT_SNAPSHOT_LIMITS: SnapshotLimits = SnapshotLimits {
    events: MAX_SNAPSHOT_EVENTS,
    state_bytes: MAX_SNAPSHOT_STATE_BYTES,
    encrypted_bytes: MAX_SNAPSHOT_BYTES,
};

pub(crate) async fn prepare_snapshot(
    storage: &Storage,
    key: &SyncGroupKey,
    created_at: DateTime<Utc>,
) -> Result<SnapshotPreparation> {
    prepare_snapshot_with_limits(storage, key, created_at, DEFAULT_SNAPSHOT_LIMITS).await
}

async fn prepare_snapshot_with_limits(
    storage: &Storage,
    key: &SyncGroupKey,
    created_at: DateTime<Utc>,
    limits: SnapshotLimits,
) -> Result<SnapshotPreparation> {
    let key_id = key.key_id();
    let creator_device_id = storage.sync_identity().await?.device_id;
    let Some((frontiers, events)) = storage.sync_snapshot_material(limits.events).await? else {
        return Ok(SnapshotPreparation::Unavailable(
            SnapshotUnavailableReason::TooManyRetainedEvents,
        ));
    };
    let state = SnapshotState {
        protocol_version: SYNC_PROTOCOL_VERSION,
        frontiers: frontiers
            .into_iter()
            .map(|(device_id, contiguous_sequence)| SnapshotFrontier {
                device_id,
                contiguous_sequence,
            })
            .collect(),
        events,
    };
    validate_state(&state, SNAPSHOT_VERSION)?;
    let state_bytes =
        serde_json::to_vec(&state).context("Impossible de sérialiser l'état de l'instantané")?;
    if state_bytes.len() > limits.state_bytes {
        return Ok(SnapshotPreparation::Unavailable(
            SnapshotUnavailableReason::StateTooLarge,
        ));
    }
    let state_hash = hex_digest(Sha256::digest(&state_bytes));
    let unchanged = storage
        .sync_snapshot_publication_hash(&key_id, &creator_device_id)
        .await?
        .as_deref()
        == Some(&state_hash);
    let document = SnapshotDocument {
        format: SNAPSHOT_FORMAT.to_string(),
        format_version: SNAPSHOT_VERSION,
        created_at: created_at.to_rfc3339(),
        state,
    };
    let encrypted = encrypt_document(&document, &state_hash, key, &creator_device_id)?;
    let mut bytes =
        serde_json::to_vec(&encrypted).context("Impossible de sérialiser l'instantané chiffré")?;
    bytes.push(b'\n');
    if bytes.len() > limits.encrypted_bytes {
        return Ok(SnapshotPreparation::Unavailable(
            SnapshotUnavailableReason::EncryptedDocumentTooLarge,
        ));
    }
    let prepared = PreparedSnapshot {
        relative_path: snapshot_path(&key_id, &creator_device_id),
        bytes,
        key_id,
        creator_device_id,
        state_hash,
        frontiers: document
            .state
            .frontiers
            .iter()
            .map(|frontier| (frontier.device_id.clone(), frontier.contiguous_sequence))
            .collect(),
    };
    if unchanged {
        Ok(SnapshotPreparation::ConfirmedUnchanged(prepared))
    } else {
        Ok(SnapshotPreparation::Ready(prepared))
    }
}

pub(crate) fn verify_snapshot(
    relative_path: &str,
    bytes: &[u8],
    key: &SyncGroupKey,
    expected_state_hash: &str,
) -> Result<()> {
    let encrypted = read_encrypted(relative_path, bytes)?;
    validate_encrypted_header(&encrypted, &key.key_id())?;
    if encrypted.state_hash != expected_state_hash {
        bail!("L'instantané distant ne correspond pas à l'état confirmé");
    }
    let document = decrypt_document(&encrypted, key)?;
    validate_document(&document)?;
    let state_bytes = serde_json::to_vec(&document.state)
        .context("Impossible de vérifier l'état de l'instantané")?;
    if hex_digest(Sha256::digest(state_bytes)) != expected_state_hash {
        bail!("L'empreinte de l'instantané est incohérente");
    }
    Ok(())
}

pub(crate) async fn import_snapshot(
    storage: &Storage,
    key: &SyncGroupKey,
    relative_path: &str,
    bytes: &[u8],
    observed_at: DateTime<Utc>,
) -> Result<SyncImportReport> {
    let encrypted = read_encrypted(relative_path, bytes)?;
    validate_encrypted_header(&encrypted, &key.key_id())?;
    verify_snapshot(relative_path, bytes, key, &encrypted.state_hash)?;
    if storage
        .sync_snapshot_import_hash(&encrypted.key_id, &encrypted.creator_device_id)
        .await?
        .as_deref()
        == Some(encrypted.state_hash.as_str())
    {
        return Ok(SyncImportReport::default());
    }
    let document = decrypt_document(&encrypted, key)?;
    let frontiers = document
        .state
        .frontiers
        .iter()
        .map(|frontier| (frontier.device_id.clone(), frontier.contiguous_sequence))
        .collect::<Vec<_>>();
    let report = storage
        .import_sync_checkpoint_events(
            &document.state.events,
            &frontiers,
            observed_at,
            MAX_SNAPSHOT_EVENTS,
        )
        .await?;
    storage
        .record_sync_snapshot_import(
            &encrypted.key_id,
            &encrypted.creator_device_id,
            &encrypted.state_hash,
            observed_at,
        )
        .await?;
    Ok(report)
}

pub(crate) fn snapshot_path(key_id: &str, creator_device_id: &str) -> String {
    format!("v2/{key_id}/{SNAPSHOT_DIRECTORY}/{creator_device_id}.json")
}

pub(crate) fn snapshot_creator<'a>(relative_path: &'a str, key_id: &str) -> Result<&'a str> {
    let components = relative_path.split('/').collect::<Vec<_>>();
    if components.len() != 4
        || components[0] != "v2"
        || components[1] != key_id
        || components[2] != SNAPSHOT_DIRECTORY
    {
        bail!("Chemin d'instantané invalide");
    }
    let creator = components[3]
        .strip_suffix(".json")
        .context("Nom de fichier d'instantané invalide")?;
    uuid::Uuid::parse_str(creator).context("Créateur d'instantané invalide")?;
    Ok(creator)
}

fn read_encrypted(relative_path: &str, bytes: &[u8]) -> Result<EncryptedSnapshotDocument> {
    if bytes.len() > MAX_SNAPSHOT_BYTES {
        bail!("L'instantané chiffré dépasse la taille autorisée");
    }
    let encrypted: EncryptedSnapshotDocument =
        serde_json::from_slice(bytes).context("Instantané chiffré mal formé")?;
    if relative_path != snapshot_path(&encrypted.key_id, &encrypted.creator_device_id) {
        bail!("Le chemin de l'instantané ne correspond pas à ses métadonnées");
    }
    Ok(encrypted)
}

fn validate_document(document: &SnapshotDocument) -> Result<()> {
    if document.format != SNAPSHOT_FORMAT
        || !matches!(
            document.format_version,
            LEGACY_SNAPSHOT_VERSION | SNAPSHOT_VERSION
        )
    {
        bail!("Version d'instantané non prise en charge");
    }
    DateTime::parse_from_rfc3339(&document.created_at).context("Date d'instantané invalide")?;
    validate_state(&document.state, document.format_version)
}

fn validate_state(state: &SnapshotState, format_version: u32) -> Result<()> {
    if state.protocol_version != SYNC_PROTOCOL_VERSION
        || state.frontiers.is_empty()
        || state.frontiers.len() > MAX_SNAPSHOT_DEVICES
        || state.events.len() > MAX_SNAPSHOT_EVENTS
    {
        bail!("État d'instantané invalide");
    }
    let mut previous_device = None;
    for frontier in &state.frontiers {
        uuid::Uuid::parse_str(&frontier.device_id).context("Appareil d'instantané invalide")?;
        if frontier.contiguous_sequence < 0
            || previous_device.is_some_and(|device: &str| device >= frontier.device_id.as_str())
        {
            bail!("Frontières d'instantané invalides");
        }
        previous_device = Some(frontier.device_id.as_str());
    }
    if format_version == LEGACY_SNAPSHOT_VERSION {
        let mut event_index = 0;
        for frontier in &state.frontiers {
            for expected in 1..=frontier.contiguous_sequence {
                let event = state
                    .events
                    .get(event_index)
                    .context("L'instantané contient un trou de séquence")?;
                if event.device_id != frontier.device_id || event.sequence != expected {
                    bail!("L'instantané contient un trou de séquence");
                }
                event_index += 1;
            }
        }
        if event_index != state.events.len() {
            bail!("L'instantané contient des événements hors frontière");
        }
    } else {
        let frontier_by_device = state
            .frontiers
            .iter()
            .map(|frontier| (frontier.device_id.as_str(), frontier.contiguous_sequence))
            .collect::<std::collections::HashMap<_, _>>();
        let mut previous_event = None;
        for event in &state.events {
            let identity = (event.device_id.as_str(), event.sequence);
            if event.sequence <= 0
                || frontier_by_device
                    .get(event.device_id.as_str())
                    .is_none_or(|frontier| event.sequence > *frontier)
                || previous_event.is_some_and(|previous| previous >= identity)
            {
                bail!("Le checkpoint contient un événement hors frontière ou dupliqué");
            }
            previous_event = Some(identity);
        }
    }
    Ok(())
}

fn associated_data(document: &EncryptedSnapshotDocument) -> Result<Vec<u8>> {
    serde_json::to_vec(&SnapshotAssociatedData {
        format: &document.format,
        format_version: document.format_version,
        protocol_version: document.protocol_version,
        key_id: &document.key_id,
        creator_device_id: &document.creator_device_id,
        state_hash: &document.state_hash,
    })
    .context("Impossible de sérialiser les métadonnées de l'instantané")
}

fn encrypt_document(
    document: &SnapshotDocument,
    state_hash: &str,
    key: &SyncGroupKey,
    creator_device_id: &str,
) -> Result<EncryptedSnapshotDocument> {
    let mut nonce = [0_u8; NONCE_BYTES];
    getrandom::fill(&mut nonce).context("Impossible de générer le nonce de l'instantané")?;
    let mut encrypted = EncryptedSnapshotDocument {
        format: ENCRYPTED_SNAPSHOT_FORMAT.to_string(),
        format_version: ENCRYPTED_SNAPSHOT_VERSION,
        protocol_version: SYNC_PROTOCOL_VERSION,
        key_id: key.key_id(),
        creator_device_id: creator_device_id.to_string(),
        state_hash: state_hash.to_string(),
        nonce_base64: BASE64.encode(nonce),
        ciphertext_base64: String::new(),
    };
    let plaintext = Zeroizing::new(
        serde_json::to_vec(document).context("Impossible de sérialiser l'instantané")?,
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
            .map_err(|_| anyhow::anyhow!("Impossible de chiffrer l'instantané"))?,
    );
    Ok(encrypted)
}

fn decrypt_document(
    encrypted: &EncryptedSnapshotDocument,
    key: &SyncGroupKey,
) -> Result<SnapshotDocument> {
    let nonce: [u8; NONCE_BYTES] = BASE64
        .decode(&encrypted.nonce_base64)
        .context("Nonce d'instantané invalide")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("Taille de nonce d'instantané invalide"))?;
    let ciphertext = BASE64
        .decode(&encrypted.ciphertext_base64)
        .context("Contenu chiffré d'instantané invalide")?;
    let plaintext = Zeroizing::new(
        XChaCha20Poly1305::new((&key.expose_bytes()).into())
            .decrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: &ciphertext,
                    aad: &associated_data(encrypted)?,
                },
            )
            .map_err(|_| anyhow::anyhow!("Échec d'authentification de l'instantané"))?,
    );
    serde_json::from_slice(&plaintext).context("Instantané déchiffré mal formé")
}

fn validate_encrypted_header(document: &EncryptedSnapshotDocument, key_id: &str) -> Result<()> {
    if document.format != ENCRYPTED_SNAPSHOT_FORMAT
        || document.format_version != ENCRYPTED_SNAPSHOT_VERSION
        || document.protocol_version != SYNC_PROTOCOL_VERSION
        || document.key_id != key_id
        || uuid::Uuid::parse_str(&document.creator_device_id).is_err()
        || document.state_hash.len() != 64
        || !document
            .state_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        bail!("En-tête d'instantané chiffré invalide");
    }
    Ok(())
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = bytes.as_ref();
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::article::{Article, ContentKind, Source};
    use crate::sync_segments::{export_sync_directory, import_sync_directory};

    const SNAPSHOT_V1_FIXTURE: &str = include_str!("../tests/fixtures/sync/snapshot-v1.json");
    const SNAPSHOT_V2_FIXTURE: &str = include_str!("../tests/fixtures/sync/snapshot-v2.json");

    fn fixture_document(contents: &str) -> SnapshotDocument {
        let document: SnapshotDocument = serde_json::from_str(contents).unwrap();
        validate_document(&document).unwrap();
        document
    }

    fn encrypted_fixture(
        document: &SnapshotDocument,
        key: &SyncGroupKey,
        creator_device_id: &str,
    ) -> Vec<u8> {
        let state_bytes = serde_json::to_vec(&document.state).unwrap();
        let state_hash = hex_digest(Sha256::digest(state_bytes));
        serde_json::to_vec(
            &encrypt_document(document, &state_hash, key, creator_device_id).unwrap(),
        )
        .unwrap()
    }

    async fn source_with_state() -> (Storage, String) {
        let storage = Storage::open_in_memory().await.unwrap();
        storage.enable_sync().await.unwrap();
        let feed = storage
            .add_feed("https://snapshot.example/feed", None)
            .await
            .unwrap();
        let article = Article {
            id: format!("{}::entry", feed.id),
            feed_id: feed.id,
            title: Some("Private snapshot title".to_string()),
            author: Some("Snapshot author".to_string()),
            published_at: Some(Utc::now()),
            url: Some("https://snapshot.example/article".to_string()),
            content: Some("Cached body must never be synchronized".to_string()),
            content_kind: ContentKind::Full,
            source: Source::Other,
        };
        storage
            .upsert_articles(std::slice::from_ref(&article))
            .await
            .unwrap();
        storage.set_read(&article.id, true).await.unwrap();
        storage.set_favorite(&article.id, true).await.unwrap();
        (storage, article.id)
    }

    #[tokio::test]
    async fn encrypted_snapshot_reconstructs_projections_without_cached_content() {
        let (source, _article_id) = source_with_state().await;
        let key = SyncGroupKey::from_bytes([0xb1; 32]);
        let observed_at = DateTime::parse_from_rfc3339("2026-08-29T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let prepared = prepare_snapshot(&source, &key, observed_at)
            .await
            .unwrap()
            .expect_ready();
        let visible = String::from_utf8_lossy(&prepared.bytes);
        assert!(!visible.contains("snapshot.example"));
        assert!(!visible.contains("Private snapshot title"));
        assert!(!visible.contains("Cached body"));

        let target = Storage::open_in_memory().await.unwrap();
        target.enable_sync().await.unwrap();
        let report = import_snapshot(
            &target,
            &key,
            &prepared.relative_path,
            &prepared.bytes,
            observed_at,
        )
        .await
        .unwrap();
        assert_eq!(report.imported, 3);
        assert_eq!(target.list_feeds().await.unwrap().len(), 1);
        let article = target.list_articles().await.unwrap().pop().unwrap();
        assert!(article.is_read);
        assert!(article.is_favorite);
        assert!(article.article.content.is_none());

        let repeated = import_snapshot(
            &target,
            &key,
            &prepared.relative_path,
            &prepared.bytes,
            observed_at,
        )
        .await
        .unwrap();
        assert_eq!(repeated, SyncImportReport::default());
    }

    #[tokio::test]
    async fn unchanged_snapshot_is_not_republished_after_confirmation() {
        let (storage, _) = source_with_state().await;
        let key = SyncGroupKey::from_bytes([0xb2; 32]);
        let observed_at = Utc::now();
        let prepared = prepare_snapshot(&storage, &key, observed_at)
            .await
            .unwrap()
            .expect_ready();
        storage
            .record_sync_snapshot_publication(
                &prepared.key_id,
                &prepared.creator_device_id,
                &prepared.state_hash,
                observed_at,
            )
            .await
            .unwrap();
        assert!(matches!(
            prepare_snapshot(&storage, &key, observed_at).await.unwrap(),
            SnapshotPreparation::ConfirmedUnchanged(_)
        ));
    }

    #[tokio::test]
    async fn oversized_snapshot_is_skipped_without_blocking_the_journal() {
        let (storage, _) = source_with_state().await;
        assert!(storage.sync_snapshot_material(1).await.unwrap().is_none());
        assert!(matches!(
            prepare_snapshot_with_limits(
                &storage,
                &SyncGroupKey::from_bytes([0xb8; 32]),
                Utc::now(),
                SnapshotLimits {
                    events: 1,
                    ..DEFAULT_SNAPSHOT_LIMITS
                },
            )
            .await
            .unwrap(),
            SnapshotPreparation::Unavailable(SnapshotUnavailableReason::TooManyRetainedEvents)
        ));
        assert!(matches!(
            prepare_snapshot_with_limits(
                &storage,
                &SyncGroupKey::from_bytes([0xb9; 32]),
                Utc::now(),
                SnapshotLimits {
                    state_bytes: 1,
                    ..DEFAULT_SNAPSHOT_LIMITS
                },
            )
            .await
            .unwrap(),
            SnapshotPreparation::Unavailable(SnapshotUnavailableReason::StateTooLarge)
        ));
        assert!(matches!(
            prepare_snapshot_with_limits(
                &storage,
                &SyncGroupKey::from_bytes([0xba; 32]),
                Utc::now(),
                SnapshotLimits {
                    encrypted_bytes: 1,
                    ..DEFAULT_SNAPSHOT_LIMITS
                },
            )
            .await
            .unwrap(),
            SnapshotPreparation::Unavailable(SnapshotUnavailableReason::EncryptedDocumentTooLarge)
        ));
        assert_eq!(
            storage.local_sync_events_after(0, 10).await.unwrap().len(),
            3
        );
    }

    #[tokio::test]
    async fn checkpoint_keeps_only_winning_events_and_advances_sparse_frontier() {
        let source = Storage::open_in_memory().await.unwrap();
        source.enable_sync().await.unwrap();
        let feed = source
            .add_feed("https://compact.example/feed", None)
            .await
            .unwrap();
        for index in 0..50 {
            source
                .set_feed_active(&feed.id, index % 2 != 0)
                .await
                .unwrap();
        }
        let identity = source.sync_identity().await.unwrap();
        let frontier = identity.next_sequence - 1;
        assert_eq!(frontier, 51);
        let (_, compact_events) = source
            .sync_snapshot_material(MAX_SNAPSHOT_EVENTS)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(compact_events.len(), 2);

        let key = SyncGroupKey::from_bytes([0xb6; 32]);
        let prepared = prepare_snapshot(&source, &key, Utc::now())
            .await
            .unwrap()
            .expect_ready();
        let target = Storage::open_in_memory().await.unwrap();
        target.enable_sync().await.unwrap();
        let report = import_snapshot(
            &target,
            &key,
            &prepared.relative_path,
            &prepared.bytes,
            Utc::now(),
        )
        .await
        .unwrap();

        assert_eq!(report.imported, 2);
        assert_eq!(
            target
                .sync_import_cursor(&identity.device_id)
                .await
                .unwrap(),
            frontier
        );
        assert!(target.list_feeds().await.unwrap()[0].is_active);
    }

    #[tokio::test]
    async fn legacy_contiguous_snapshot_remains_importable() {
        let document = fixture_document(SNAPSHOT_V1_FIXTURE);
        let creator_device_id = document.state.frontiers[0].device_id.clone();
        let key = SyncGroupKey::from_bytes([0xb7; 32]);
        let bytes = encrypted_fixture(&document, &key, &creator_device_id);
        let target = Storage::open_in_memory().await.unwrap();
        target.enable_sync().await.unwrap();

        let report = import_snapshot(
            &target,
            &key,
            &snapshot_path(&key.key_id(), &creator_device_id),
            &bytes,
            Utc::now(),
        )
        .await
        .unwrap();
        assert_eq!(report.imported, 1);
        let feeds = target.list_feeds().await.unwrap();
        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds[0].url, "https://fixture.example/feed.xml");
        assert!(feeds[0].is_active);
    }

    #[tokio::test]
    async fn compact_v2_fixture_remains_importable() {
        let document = fixture_document(SNAPSHOT_V2_FIXTURE);
        let creator_device_id = document.state.frontiers[0].device_id.clone();
        let key = SyncGroupKey::from_bytes([0xbb; 32]);
        let bytes = encrypted_fixture(&document, &key, &creator_device_id);
        let target = Storage::open_in_memory().await.unwrap();
        target.enable_sync().await.unwrap();

        let report = import_snapshot(
            &target,
            &key,
            &snapshot_path(&key.key_id(), &creator_device_id),
            &bytes,
            Utc::now(),
        )
        .await
        .unwrap();

        assert_eq!(report.imported, 2);
        assert_eq!(
            target.sync_import_cursor(&creator_device_id).await.unwrap(),
            3
        );
        let feeds = target.list_feeds().await.unwrap();
        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds[0].url, "https://fixture.example/compact.xml");
        assert!(!feeds[0].is_active);
    }

    #[tokio::test]
    async fn future_snapshot_version_is_rejected_without_sqlite_side_effect() {
        let valid_document = fixture_document(SNAPSHOT_V2_FIXTURE);
        let creator_device_id = valid_document.state.frontiers[0].device_id.clone();
        let key = SyncGroupKey::from_bytes([0xbc; 32]);
        let target = Storage::open_in_memory().await.unwrap();
        target.enable_sync().await.unwrap();
        import_snapshot(
            &target,
            &key,
            &snapshot_path(&key.key_id(), &creator_device_id),
            &encrypted_fixture(&valid_document, &key, &creator_device_id),
            Utc::now(),
        )
        .await
        .unwrap();
        let import_hash_before = target
            .sync_snapshot_import_hash(&key.key_id(), &creator_device_id)
            .await
            .unwrap();

        let mut future_document = valid_document;
        future_document.format_version = SNAPSHOT_VERSION + 1;
        let bytes = encrypted_fixture(&future_document, &key, &creator_device_id);

        let result = import_snapshot(
            &target,
            &key,
            &snapshot_path(&key.key_id(), &creator_device_id),
            &bytes,
            Utc::now(),
        )
        .await;

        assert!(result.is_err());
        let feeds = target.list_feeds().await.unwrap();
        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds[0].url, "https://fixture.example/compact.xml");
        assert!(!feeds[0].is_active);
        assert_eq!(
            target.sync_import_cursor(&creator_device_id).await.unwrap(),
            3
        );
        assert_eq!(
            target
                .sync_snapshot_import_hash(&key.key_id(), &creator_device_id)
                .await
                .unwrap(),
            import_hash_before
        );
    }

    #[tokio::test]
    async fn corrupted_or_wrong_key_snapshot_has_no_side_effect() {
        let (source, _) = source_with_state().await;
        let key = SyncGroupKey::from_bytes([0xb3; 32]);
        let prepared = prepare_snapshot(&source, &key, Utc::now())
            .await
            .unwrap()
            .expect_ready();
        let target = Storage::open_in_memory().await.unwrap();
        target.enable_sync().await.unwrap();
        assert!(
            import_snapshot(
                &target,
                &SyncGroupKey::from_bytes([0xb4; 32]),
                &prepared.relative_path,
                &prepared.bytes,
                Utc::now(),
            )
            .await
            .is_err()
        );
        let mut corrupted = prepared.bytes.clone();
        let index = corrupted.len() / 2;
        corrupted[index] ^= 1;
        assert!(
            import_snapshot(
                &target,
                &key,
                &prepared.relative_path,
                &corrupted,
                Utc::now(),
            )
            .await
            .is_err()
        );
        assert!(target.list_feeds().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn restored_snapshot_then_applies_following_segments() {
        let source = Storage::open_in_memory().await.unwrap();
        source.enable_sync().await.unwrap();
        let feed = source
            .add_feed("https://catchup.example/feed", None)
            .await
            .unwrap();
        let key = SyncGroupKey::from_bytes([0xb5; 32]);
        let snapshot = prepare_snapshot(&source, &key, Utc::now())
            .await
            .unwrap()
            .expect_ready();
        source.set_feed_active(&feed.id, false).await.unwrap();
        let directory = tempfile::tempdir().unwrap();
        export_sync_directory(&source, directory.path(), &key)
            .await
            .unwrap();

        let target = Storage::open_in_memory().await.unwrap();
        target.enable_sync().await.unwrap();
        let restored = import_snapshot(
            &target,
            &key,
            &snapshot.relative_path,
            &snapshot.bytes,
            Utc::now(),
        )
        .await
        .unwrap();
        assert_eq!(restored.imported, 1);
        assert!(target.list_feeds().await.unwrap()[0].is_active);

        let caught_up = import_sync_directory(&target, directory.path(), &key, Utc::now())
            .await
            .unwrap();
        assert_eq!(caught_up.imported, 1);
        assert!(!target.list_feeds().await.unwrap()[0].is_active);
    }
}
