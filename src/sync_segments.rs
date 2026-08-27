use crate::storage::Storage;
use crate::sync::{SYNC_PROTOCOL_VERSION, SyncEvent, SyncImportReport};
use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

const SEGMENT_PAYLOAD_FORMAT: &str = "inkriver-sync-payload";
const SEGMENT_PAYLOAD_VERSION: u32 = 1;
const ENCRYPTED_SEGMENT_FORMAT: &str = "inkriver-encrypted-sync-segment";
const ENCRYPTED_SEGMENT_VERSION: u32 = 1;
const FORMAT_DIRECTORY: &str = "v2";
const GROUP_KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 24;
const MAX_SEGMENT_EVENTS: usize = 250;
const MAX_SEGMENT_BYTES: u64 = 2 * 1024 * 1024;
const MAX_DIRECTORY_SEGMENTS: usize = 1_000;
const MAX_EVENTS_PER_DIRECTORY_EXPORT: usize = 1_000;
const MAX_EVENTS_PER_DIRECTORY_IMPORT: usize = 1_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SegmentChecksumBody<'a> {
    format: &'a str,
    format_version: u32,
    protocol_version: i64,
    device_id: &'a str,
    first_sequence: i64,
    last_sequence: i64,
    events: &'a [SyncEvent],
}

/// Random shared secret used to encrypt one synchronization group.
///
/// The key is supplied by the caller and is never persisted by this module.
/// Secure platform storage and device pairing are deliberately deferred to
/// SYNC-008.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct SyncGroupKey([u8; GROUP_KEY_BYTES]);

impl SyncGroupKey {
    /// Generates a new key using the operating system random source.
    pub fn generate() -> Result<Self> {
        let mut bytes = [0_u8; GROUP_KEY_BYTES];
        getrandom::fill(&mut bytes).context("Impossible de générer la clé de synchronisation")?;
        Ok(Self(bytes))
    }

    /// Builds a group key from exactly 32 secret bytes.
    pub fn from_bytes(bytes: [u8; GROUP_KEY_BYTES]) -> Self {
        Self(bytes)
    }

    /// Stable, non-secret fingerprint used to separate current and future keys.
    pub fn key_id(&self) -> String {
        hex_digest(Sha256::digest(self.0))
    }
}

impl fmt::Debug for SyncGroupKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SyncGroupKey([REDACTED])")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SegmentFile {
    format: String,
    format_version: u32,
    protocol_version: i64,
    device_id: String,
    first_sequence: i64,
    last_sequence: i64,
    events: Vec<SyncEvent>,
    checksum_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SegmentAssociatedData<'a> {
    format: &'a str,
    format_version: u32,
    protocol_version: i64,
    key_id: &'a str,
    device_id: &'a str,
    first_sequence: i64,
    last_sequence: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EncryptedSegmentFile {
    format: String,
    format_version: u32,
    protocol_version: i64,
    key_id: String,
    device_id: String,
    first_sequence: i64,
    last_sequence: i64,
    nonce_base64: String,
    ciphertext_base64: String,
}

/// Counters returned after publishing local immutable segments.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SyncDirectoryExportReport {
    pub written_segments: usize,
    pub reused_segments: usize,
    pub exported_events: usize,
    pub last_exported_sequence: i64,
}

/// Counters returned after validating and importing one directory snapshot.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SyncDirectoryImportReport {
    pub segments: usize,
    pub received: usize,
    pub imported: usize,
    pub duplicates: usize,
    pub applied: usize,
    pub pending: usize,
}

/// Publishes local events not covered by the persistent export cursor.
///
/// Files are immutable and organized as
/// `v2/<key fingerprint>/<device UUID>/<range>.json`.
/// Existing identical files are reused to recover safely from a process crash
/// between the filesystem write and the SQLite cursor update.
///
/// # Errors
///
/// Returns an error without advancing the cursor when the directory cannot be
/// written, an existing path conflicts, or one event cannot fit in a segment.
pub async fn export_sync_directory(
    storage: &Storage,
    root: &Path,
    key: &SyncGroupKey,
) -> Result<SyncDirectoryExportReport> {
    let identity = storage.sync_identity().await?;
    if !identity.is_enabled {
        bail!("Synchronization must be enabled before exporting segments");
    }
    let key_id = key.key_id();
    let mut cursor = storage.local_sync_export_cursor(&key_id).await?;
    let mut report = SyncDirectoryExportReport {
        last_exported_sequence: cursor,
        ..SyncDirectoryExportReport::default()
    };
    let events = storage
        .local_sync_events_after(cursor, MAX_EVENTS_PER_DIRECTORY_EXPORT)
        .await?;
    if events.is_empty() {
        return Ok(report);
    }
    let device_directory = root
        .join(FORMAT_DIRECTORY)
        .join(&key_id)
        .join(&identity.device_id);
    fs::create_dir_all(&device_directory).with_context(|| {
        format!(
            "Impossible de créer le répertoire de segments {}",
            device_directory.display()
        )
    })?;

    let mut offset = 0;
    while offset < events.len() {
        if events[offset].sequence != cursor + 1 {
            bail!("The local synchronization journal contains a sequence gap");
        }
        let maximum_end = (offset + MAX_SEGMENT_EVENTS).min(events.len());
        let mut end = maximum_end;
        let (segment, bytes) = loop {
            let candidate = build_segment(&identity.device_id, &events[offset..end])?;
            let encrypted = encrypt_segment(&candidate, key)?;
            let bytes = serialize_encrypted_segment(&encrypted)?;
            if bytes.len() as u64 <= MAX_SEGMENT_BYTES {
                break (candidate, bytes);
            }
            if end == offset + 1 {
                bail!("One synchronization event exceeds the segment size limit");
            }
            end -= 1;
        };
        let outcome = publish_segment(&device_directory, &segment, key, &bytes)?;
        match outcome {
            PublishOutcome::Written => report.written_segments += 1,
            PublishOutcome::Reused => report.reused_segments += 1,
        }
        storage
            .mark_local_sync_events_exported(&key_id, cursor, segment.last_sequence)
            .await?;
        report.exported_events += segment.events.len();
        report.last_exported_sequence = segment.last_sequence;
        cursor = segment.last_sequence;
        offset = end;
    }
    Ok(report)
}

/// Validates every segment in a directory before atomically importing its events.
///
/// Dot-prefixed files created by synchronization tools are ignored. Every
/// other entry must follow the version-one directory layout.
///
/// # Errors
///
/// Returns an error without changing SQLite for unknown versions, malformed or
/// conflicting metadata, invalid checksums, excessive input, or merge errors.
pub async fn import_sync_directory(
    storage: &Storage,
    root: &Path,
    key: &SyncGroupKey,
    observed_at: DateTime<Utc>,
) -> Result<SyncDirectoryImportReport> {
    let paths = discover_segments(root, &key.key_id())?;
    let local_device_id = storage.sync_identity().await?.device_id;
    let mut cursors = HashMap::new();
    let mut events = Vec::new();
    for path in &paths {
        let segment = read_segment(path, root, key)?;
        if segment.device_id == local_device_id {
            continue;
        }
        let cursor = match cursors.get(&segment.device_id) {
            Some(cursor) => *cursor,
            None => {
                let cursor = storage.sync_import_cursor(&segment.device_id).await?;
                cursors.insert(segment.device_id.clone(), cursor);
                cursor
            }
        };
        for event in segment.events {
            if event.sequence > cursor && events.len() < MAX_EVENTS_PER_DIRECTORY_IMPORT {
                events.push(event);
            }
        }
    }
    let imported = storage.import_sync_events(&events, observed_at).await?;
    Ok(import_report(paths.len(), imported))
}

fn import_report(segments: usize, report: SyncImportReport) -> SyncDirectoryImportReport {
    SyncDirectoryImportReport {
        segments,
        received: report.received,
        imported: report.imported,
        duplicates: report.duplicates,
        applied: report.applied,
        pending: report.pending,
    }
}

fn build_segment(device_id: &str, events: &[SyncEvent]) -> Result<SegmentFile> {
    let first = events
        .first()
        .context("A segment cannot be empty")?
        .sequence;
    let last = events.last().context("A segment cannot be empty")?.sequence;
    for (index, event) in events.iter().enumerate() {
        if event.device_id != device_id
            || event.sequence != first + index as i64
            || event.protocol_version != SYNC_PROTOCOL_VERSION
        {
            bail!("A local segment must contain one contiguous device journal");
        }
    }
    let checksum = checksum_body(device_id, first, last, events)?;
    Ok(SegmentFile {
        format: SEGMENT_PAYLOAD_FORMAT.to_string(),
        format_version: SEGMENT_PAYLOAD_VERSION,
        protocol_version: SYNC_PROTOCOL_VERSION,
        device_id: device_id.to_string(),
        first_sequence: first,
        last_sequence: last,
        events: events.to_vec(),
        checksum_sha256: checksum,
    })
}

fn checksum_body(
    device_id: &str,
    first_sequence: i64,
    last_sequence: i64,
    events: &[SyncEvent],
) -> Result<String> {
    let body = SegmentChecksumBody {
        format: SEGMENT_PAYLOAD_FORMAT,
        format_version: SEGMENT_PAYLOAD_VERSION,
        protocol_version: SYNC_PROTOCOL_VERSION,
        device_id,
        first_sequence,
        last_sequence,
        events,
    };
    let canonical = serde_json::to_vec(&body)
        .context("Impossible de sérialiser le contenu vérifiable du segment")?;
    Ok(hex_digest(Sha256::digest(canonical)))
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

fn serialize_encrypted_segment(segment: &EncryptedSegmentFile) -> Result<Vec<u8>> {
    let mut bytes =
        serde_json::to_vec(segment).context("Impossible de sérialiser le segment chiffré")?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn associated_data(segment: &EncryptedSegmentFile) -> Result<Vec<u8>> {
    serde_json::to_vec(&SegmentAssociatedData {
        format: &segment.format,
        format_version: segment.format_version,
        protocol_version: segment.protocol_version,
        key_id: &segment.key_id,
        device_id: &segment.device_id,
        first_sequence: segment.first_sequence,
        last_sequence: segment.last_sequence,
    })
    .context("Impossible de sérialiser les métadonnées authentifiées")
}

fn encrypt_segment(segment: &SegmentFile, key: &SyncGroupKey) -> Result<EncryptedSegmentFile> {
    let mut nonce = [0_u8; NONCE_BYTES];
    getrandom::fill(&mut nonce).context("Impossible de générer le nonce du segment")?;
    let mut encrypted = EncryptedSegmentFile {
        format: ENCRYPTED_SEGMENT_FORMAT.to_string(),
        format_version: ENCRYPTED_SEGMENT_VERSION,
        protocol_version: SYNC_PROTOCOL_VERSION,
        key_id: key.key_id(),
        device_id: segment.device_id.clone(),
        first_sequence: segment.first_sequence,
        last_sequence: segment.last_sequence,
        nonce_base64: BASE64.encode(nonce),
        ciphertext_base64: String::new(),
    };
    let plaintext = Zeroizing::new(
        serde_json::to_vec(segment)
            .context("Impossible de sérialiser le contenu privé du segment")?,
    );
    let aad = associated_data(&encrypted)?;
    let cipher = XChaCha20Poly1305::new((&key.0).into());
    let nonce = XNonce::from(nonce);
    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: &plaintext,
                aad: &aad,
            },
        )
        .map_err(|_| anyhow::anyhow!("Impossible de chiffrer le segment"))?;
    encrypted.ciphertext_base64 = BASE64.encode(ciphertext);
    Ok(encrypted)
}

fn decrypt_segment(segment: &EncryptedSegmentFile, key: &SyncGroupKey) -> Result<SegmentFile> {
    validate_encrypted_header(segment, &key.key_id())?;
    let nonce = BASE64
        .decode(&segment.nonce_base64)
        .context("Nonce de segment invalide")?;
    let nonce: [u8; NONCE_BYTES] = nonce
        .try_into()
        .map_err(|_| anyhow::anyhow!("Invalid synchronization segment nonce"))?;
    let ciphertext = BASE64
        .decode(&segment.ciphertext_base64)
        .context("Contenu chiffré du segment invalide")?;
    let aad = associated_data(segment)?;
    let cipher = XChaCha20Poly1305::new((&key.0).into());
    let nonce = XNonce::from(nonce);
    let plaintext = Zeroizing::new(
        cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: &ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| anyhow::anyhow!("Synchronization segment authentication failed"))?,
    );
    let decrypted: SegmentFile = serde_json::from_slice(&plaintext)
        .context("Malformed decrypted synchronization segment")?;
    validate_segment(&decrypted)?;
    if decrypted.protocol_version != segment.protocol_version
        || decrypted.device_id != segment.device_id
        || decrypted.first_sequence != segment.first_sequence
        || decrypted.last_sequence != segment.last_sequence
    {
        bail!("Encrypted segment metadata does not match its authenticated payload");
    }
    Ok(decrypted)
}

fn validate_encrypted_header(segment: &EncryptedSegmentFile, key_id: &str) -> Result<()> {
    if segment.format != ENCRYPTED_SEGMENT_FORMAT
        || segment.format_version != ENCRYPTED_SEGMENT_VERSION
    {
        bail!("Unsupported encrypted synchronization segment version");
    }
    if segment.protocol_version != SYNC_PROTOCOL_VERSION {
        bail!("Unsupported synchronization protocol version in segment");
    }
    if segment.key_id != key_id {
        bail!("Synchronization segment belongs to another group key");
    }
    if uuid::Uuid::parse_str(&segment.device_id).is_err()
        || segment.first_sequence <= 0
        || segment.last_sequence < segment.first_sequence
        || segment.last_sequence - segment.first_sequence >= MAX_SEGMENT_EVENTS as i64
    {
        bail!("Invalid encrypted synchronization segment metadata");
    }
    Ok(())
}

fn segment_file_name(first: i64, last: i64) -> String {
    format!("{first:020}-{last:020}.json")
}

enum PublishOutcome {
    Written,
    Reused,
}

fn publish_segment(
    directory: &Path,
    segment: &SegmentFile,
    key: &SyncGroupKey,
    bytes: &[u8],
) -> Result<PublishOutcome> {
    let destination = directory.join(segment_file_name(
        segment.first_sequence,
        segment.last_sequence,
    ));
    if destination.exists() {
        verify_existing_segment(&destination, segment, key)?;
        return Ok(PublishOutcome::Reused);
    }

    let temporary = directory.join(format!(
        ".{}.{}.tmp",
        destination
            .file_name()
            .and_then(OsStr::to_str)
            .context("Nom de segment invalide")?,
        uuid::Uuid::new_v4()
    ));
    let write_result = (|| -> Result<PublishOutcome> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .with_context(|| format!("Impossible de créer {}", temporary.display()))?;
        file.write_all(bytes)
            .context("Impossible d'écrire le segment temporaire")?;
        file.sync_all()
            .context("Impossible de synchroniser le segment temporaire")?;
        drop(file);

        match fs::hard_link(&temporary, &destination) {
            Ok(()) => {
                fs::remove_file(&temporary)
                    .context("Impossible de supprimer le segment temporaire")?;
                Ok(PublishOutcome::Written)
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                fs::remove_file(&temporary)
                    .context("Impossible de supprimer le segment temporaire concurrent")?;
                verify_existing_segment(&destination, segment, key)?;
                Ok(PublishOutcome::Reused)
            }
            Err(_) => {
                match OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&destination)
                {
                    Ok(mut destination_file) => {
                        let publish_result = destination_file
                            .write_all(bytes)
                            .and_then(|()| destination_file.sync_all());
                        drop(destination_file);
                        if let Err(error) = publish_result {
                            let _ = fs::remove_file(&destination);
                            return Err(error)
                                .context("Impossible de publier le segment sans écrasement");
                        }
                        fs::remove_file(&temporary)
                            .context("Impossible de supprimer le segment temporaire")?;
                        Ok(PublishOutcome::Written)
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        fs::remove_file(&temporary)
                            .context("Impossible de supprimer le segment temporaire concurrent")?;
                        verify_existing_segment(&destination, segment, key)?;
                        Ok(PublishOutcome::Reused)
                    }
                    Err(error) => {
                        Err(error).context("Impossible de publier le segment sans écrasement")
                    }
                }
            }
        }
    })();
    if write_result.is_err() && temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    write_result
}

fn verify_existing_segment(path: &Path, expected: &SegmentFile, key: &SyncGroupKey) -> Result<()> {
    let existing = decrypt_segment(&read_encrypted_segment_file(path)?, key)?;
    if existing != *expected {
        bail!("An immutable synchronization segment already exists with different content");
    }
    Ok(())
}

fn discover_segments(root: &Path, key_id: &str) -> Result<Vec<PathBuf>> {
    let metadata = fs::symlink_metadata(root)
        .with_context(|| format!("Impossible de lire {}", root.display()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("The synchronization root must be a real directory");
    }
    let mut version_directory = None;
    for entry in read_directory(root)? {
        let name = entry.file_name();
        if is_dot_entry(&name) {
            continue;
        }
        if name != OsStr::new(FORMAT_DIRECTORY) {
            bail!("Unknown synchronization directory version or unexpected entry");
        }
        require_real_directory(&entry.path())?;
        version_directory = Some(entry.path());
    }
    let Some(version_directory) = version_directory else {
        return Ok(Vec::new());
    };

    let mut key_directory = None;
    for entry in read_directory(&version_directory)? {
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("Invalid synchronization key directory name"))?;
        if is_dot_name(&name) {
            continue;
        }
        if name != key_id {
            bail!("Synchronization directory contains segments for another group key");
        }
        require_real_directory(&entry.path())?;
        key_directory = Some(entry.path());
    }
    let Some(key_directory) = key_directory else {
        return Ok(Vec::new());
    };

    let mut paths = Vec::new();
    for device in read_directory(&key_directory)? {
        let name = device
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("Invalid device directory name"))?;
        if is_dot_name(&name) {
            continue;
        }
        if uuid::Uuid::parse_str(&name).is_err() {
            bail!("Invalid synchronization device directory");
        }
        require_real_directory(&device.path())?;
        for entry in read_directory(&device.path())? {
            let file_name = entry.file_name();
            if is_dot_entry(&file_name) {
                continue;
            }
            let metadata = fs::symlink_metadata(entry.path())?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                bail!("A device directory contains a non-segment entry");
            }
            if entry.path().extension() != Some(OsStr::new("json")) {
                bail!("A device directory contains an unknown segment format");
            }
            paths.push(entry.path());
            if paths.len() > MAX_DIRECTORY_SEGMENTS {
                bail!("A directory import cannot exceed {MAX_DIRECTORY_SEGMENTS} segments");
            }
        }
    }
    paths.sort();
    Ok(paths)
}

fn read_directory(path: &Path) -> Result<Vec<fs::DirEntry>> {
    fs::read_dir(path)
        .with_context(|| format!("Impossible de lire le répertoire {}", path.display()))?
        .collect::<std::io::Result<Vec<_>>>()
        .with_context(|| format!("Impossible de parcourir le répertoire {}", path.display()))
}

fn require_real_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("The synchronization layout contains an invalid directory");
    }
    Ok(())
}

fn is_dot_entry(name: &OsStr) -> bool {
    name.to_str().is_some_and(is_dot_name)
}

fn is_dot_name(name: &str) -> bool {
    name.starts_with('.')
}

fn read_segment(path: &Path, root: &Path, key: &SyncGroupKey) -> Result<SegmentFile> {
    let encrypted = read_encrypted_segment_file(path)?;
    let segment = decrypt_segment(&encrypted, key)?;
    let relative = path
        .strip_prefix(root)
        .context("Le segment n'appartient pas au répertoire de synchronisation")?;
    let components = relative
        .components()
        .map(|component| component.as_os_str())
        .collect::<Vec<_>>();
    if components.len() != 4
        || components[0] != OsStr::new(FORMAT_DIRECTORY)
        || components[1] != OsStr::new(&encrypted.key_id)
        || components[2] != OsStr::new(&segment.device_id)
        || components[3]
            != OsStr::new(&segment_file_name(
                segment.first_sequence,
                segment.last_sequence,
            ))
    {
        bail!("Segment metadata does not match its immutable path");
    }
    Ok(segment)
}

fn read_encrypted_segment_file(path: &Path) -> Result<EncryptedSegmentFile> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("Impossible de lire {}", path.display()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!("A synchronization segment must be a regular file");
    }
    if metadata.len() > MAX_SEGMENT_BYTES {
        bail!("A synchronization segment exceeds the size limit");
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(path)
        .with_context(|| format!("Impossible d'ouvrir {}", path.display()))?
        .take(MAX_SEGMENT_BYTES + 1)
        .read_to_end(&mut bytes)
        .context("Impossible de lire le segment")?;
    if bytes.len() as u64 > MAX_SEGMENT_BYTES {
        bail!("A synchronization segment grew beyond the size limit");
    }
    serde_json::from_slice(&bytes).context("Malformed encrypted synchronization segment")
}

fn validate_segment(segment: &SegmentFile) -> Result<()> {
    if segment.format != SEGMENT_PAYLOAD_FORMAT || segment.format_version != SEGMENT_PAYLOAD_VERSION
    {
        bail!("Unsupported synchronization segment version");
    }
    if segment.protocol_version != SYNC_PROTOCOL_VERSION {
        bail!("Unsupported synchronization protocol version in segment");
    }
    if uuid::Uuid::parse_str(&segment.device_id).is_err() {
        bail!("Invalid segment device identity");
    }
    if segment.events.is_empty() || segment.events.len() > MAX_SEGMENT_EVENTS {
        bail!("Invalid number of events in synchronization segment");
    }
    if segment.first_sequence <= 0
        || segment.last_sequence != segment.first_sequence + segment.events.len() as i64 - 1
    {
        bail!("Invalid synchronization segment sequence range");
    }
    for (index, event) in segment.events.iter().enumerate() {
        if event.device_id != segment.device_id
            || event.sequence != segment.first_sequence + index as i64
            || event.protocol_version != segment.protocol_version
            || event.kind != event.payload.kind()
        {
            bail!("Synchronization segment contains an inconsistent event");
        }
    }
    let expected = checksum_body(
        &segment.device_id,
        segment.first_sequence,
        segment.last_sequence,
        &segment.events,
    )?;
    if segment.checksum_sha256 != expected {
        bail!("Synchronization segment checksum mismatch");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::article::{Article, ContentKind, Source};
    use crate::storage::StoredArticle;

    fn group_key() -> SyncGroupKey {
        SyncGroupKey::from_bytes([0x42; GROUP_KEY_BYTES])
    }

    fn cached_article(feed_id: &str, entry_key: &str) -> Article {
        Article {
            id: format!("{feed_id}::{entry_key}"),
            feed_id: feed_id.to_string(),
            title: Some("A synchronized article".to_string()),
            author: Some("InkRiver Test".to_string()),
            published_at: Some(
                DateTime::parse_from_rfc3339("2026-08-27T18:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            ),
            url: Some("https://sync.example/article".to_string()),
            content: Some("Local cached body that must not be exported".to_string()),
            content_kind: ContentKind::Full,
            source: Source::Other,
        }
    }

    fn article_state(article: &StoredArticle) -> (bool, bool, Option<&str>) {
        (
            article.is_read,
            article.is_favorite,
            article.article.title.as_deref(),
        )
    }

    fn segment_paths(root: &Path) -> Vec<PathBuf> {
        let mut result = Vec::new();
        let version = root.join(FORMAT_DIRECTORY);
        if !version.exists() {
            return result;
        }
        for key in fs::read_dir(version).unwrap() {
            let key = key.unwrap();
            for device in fs::read_dir(key.path()).unwrap() {
                let device = device.unwrap();
                for entry in fs::read_dir(device.path()).unwrap() {
                    let path = entry.unwrap().path();
                    if path.extension() == Some(OsStr::new("json")) {
                        result.push(path);
                    }
                }
            }
        }
        result.sort();
        result
    }

    #[test]
    fn group_keys_are_random_identifiable_and_redacted() {
        let first = SyncGroupKey::generate().unwrap();
        let second = SyncGroupKey::generate().unwrap();
        assert_ne!(first, second);
        assert_ne!(first.key_id(), second.key_id());
        assert_eq!(first.key_id().len(), 64);
        assert_eq!(format!("{first:?}"), "SyncGroupKey([REDACTED])");
    }

    #[tokio::test]
    async fn export_writes_only_new_events_and_recovers_from_an_immutable_collision() {
        let storage = Storage::open_in_memory().await.unwrap();
        storage.enable_sync().await.unwrap();
        let feed = storage
            .add_feed("https://sync.example/feed", None)
            .await
            .unwrap();
        let identity = storage.sync_identity().await.unwrap();
        let key = group_key();
        let directory = tempfile::tempdir().unwrap();
        let device_directory = directory
            .path()
            .join(FORMAT_DIRECTORY)
            .join(key.key_id())
            .join(&identity.device_id);
        fs::create_dir_all(&device_directory).unwrap();
        let collision = device_directory.join(segment_file_name(1, 1));
        fs::write(&collision, b"{}\n").unwrap();

        assert!(
            export_sync_directory(&storage, directory.path(), &key)
                .await
                .is_err()
        );
        fs::remove_file(collision).unwrap();
        let events = storage.local_sync_events_after(0, 10).await.unwrap();
        let segment = build_segment(&identity.device_id, &events).unwrap();
        let encrypted = encrypt_segment(&segment, &key).unwrap();
        let bytes = serialize_encrypted_segment(&encrypted).unwrap();
        assert!(matches!(
            publish_segment(&device_directory, &segment, &key, &bytes).unwrap(),
            PublishOutcome::Written
        ));
        let first = export_sync_directory(&storage, directory.path(), &key)
            .await
            .unwrap();
        assert_eq!(first.written_segments, 0);
        assert_eq!(first.reused_segments, 1);
        assert_eq!(first.exported_events, 1);
        assert_eq!(first.last_exported_sequence, 1);
        let second = export_sync_directory(&storage, directory.path(), &key)
            .await
            .unwrap();
        assert_eq!(second.exported_events, 0);
        assert_eq!(segment_paths(directory.path()).len(), 1);
        let bytes = fs::read(&segment_paths(directory.path())[0]).unwrap();
        let visible = String::from_utf8_lossy(&bytes);
        assert!(!visible.contains("Local cached body"));
        assert!(!visible.contains("A synchronized article"));
        assert!(!visible.contains("https://sync.example"));
        assert!(!visible.contains("subscription_created"));
        assert_eq!(storage.list_feeds().await.unwrap()[0].id, feed.id);
    }

    #[tokio::test]
    async fn each_encryption_uses_a_unique_nonce_and_each_key_reexports_history() {
        let storage = Storage::open_in_memory().await.unwrap();
        storage.enable_sync().await.unwrap();
        let feed = storage
            .add_feed("https://sync.example/feed", None)
            .await
            .unwrap();
        let article = cached_article(&feed.id, "entry");
        storage
            .upsert_articles(std::slice::from_ref(&article))
            .await
            .unwrap();
        for index in 0..MAX_SEGMENT_EVENTS {
            storage.set_read(&article.id, index % 2 == 0).await.unwrap();
        }

        let first_key = group_key();
        let first_directory = tempfile::tempdir().unwrap();
        let first = export_sync_directory(&storage, first_directory.path(), &first_key)
            .await
            .unwrap();
        assert_eq!(first.exported_events, MAX_SEGMENT_EVENTS + 1);
        let envelopes = segment_paths(first_directory.path())
            .iter()
            .map(|path| read_encrypted_segment_file(path).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(envelopes.len(), 2);
        assert_ne!(envelopes[0].nonce_base64, envelopes[1].nonce_base64);

        let second_key = SyncGroupKey::from_bytes([0x24; GROUP_KEY_BYTES]);
        let second_directory = tempfile::tempdir().unwrap();
        let second = export_sync_directory(&storage, second_directory.path(), &second_key)
            .await
            .unwrap();
        assert_eq!(second.exported_events, MAX_SEGMENT_EVENTS + 1);
        assert_ne!(first_key.key_id(), second_key.key_id());
    }

    #[tokio::test]
    async fn wrong_key_and_authenticated_header_tampering_have_no_side_effect() {
        let source = Storage::open_in_memory().await.unwrap();
        source.enable_sync().await.unwrap();
        source
            .add_feed("https://private.example/feed", None)
            .await
            .unwrap();
        let key = group_key();
        let directory = tempfile::tempdir().unwrap();
        export_sync_directory(&source, directory.path(), &key)
            .await
            .unwrap();

        let wrong_target = Storage::open_in_memory().await.unwrap();
        wrong_target.enable_sync().await.unwrap();
        let wrong_key = SyncGroupKey::from_bytes([0x99; GROUP_KEY_BYTES]);
        assert!(
            import_sync_directory(&wrong_target, directory.path(), &wrong_key, Utc::now())
                .await
                .is_err()
        );
        assert!(wrong_target.list_feeds().await.unwrap().is_empty());

        let path = segment_paths(directory.path()).pop().unwrap();
        let mut encrypted = read_encrypted_segment_file(&path).unwrap();
        encrypted.device_id = "00000000-0000-4000-8000-0000000000aa".to_string();
        fs::write(&path, serialize_encrypted_segment(&encrypted).unwrap()).unwrap();
        let tampered_target = Storage::open_in_memory().await.unwrap();
        tampered_target.enable_sync().await.unwrap();
        assert!(
            import_sync_directory(&tampered_target, directory.path(), &key, Utc::now())
                .await
                .is_err()
        );
        assert!(tampered_target.list_feeds().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn export_splits_large_journals_into_bounded_contiguous_segments() {
        let key = group_key();
        let storage = Storage::open_in_memory().await.unwrap();
        storage.enable_sync().await.unwrap();
        let feed = storage
            .add_feed("https://sync.example/feed", None)
            .await
            .unwrap();
        let article = cached_article(&feed.id, "entry");
        storage
            .upsert_articles(std::slice::from_ref(&article))
            .await
            .unwrap();
        for index in 0..MAX_SEGMENT_EVENTS {
            storage.set_read(&article.id, index % 2 == 0).await.unwrap();
        }
        let directory = tempfile::tempdir().unwrap();
        let report = export_sync_directory(&storage, directory.path(), &key)
            .await
            .unwrap();
        assert_eq!(report.exported_events, MAX_SEGMENT_EVENTS + 1);
        assert_eq!(report.written_segments, 2);
        let paths = segment_paths(directory.path());
        assert_eq!(paths.len(), 2);
        for path in paths {
            let metadata = fs::metadata(&path).unwrap();
            assert!(metadata.len() <= MAX_SEGMENT_BYTES);
            let encrypted = read_encrypted_segment_file(&path).unwrap();
            validate_segment(&decrypt_segment(&encrypted, &key).unwrap()).unwrap();
        }
    }

    #[tokio::test]
    async fn repeated_directory_calls_progress_through_more_than_one_import_batch() {
        let key = group_key();
        let source = Storage::open_in_memory().await.unwrap();
        source.enable_sync().await.unwrap();
        let feed = source
            .add_feed("https://sync.example/feed", None)
            .await
            .unwrap();
        let article = cached_article(&feed.id, "entry");
        source
            .upsert_articles(std::slice::from_ref(&article))
            .await
            .unwrap();
        for index in 0..MAX_EVENTS_PER_DIRECTORY_IMPORT {
            source.set_read(&article.id, index % 2 == 0).await.unwrap();
        }
        let directory = tempfile::tempdir().unwrap();
        let first_export = export_sync_directory(&source, directory.path(), &key)
            .await
            .unwrap();
        let second_export = export_sync_directory(&source, directory.path(), &key)
            .await
            .unwrap();
        assert_eq!(
            first_export.exported_events,
            MAX_EVENTS_PER_DIRECTORY_EXPORT
        );
        assert_eq!(second_export.exported_events, 1);

        let target = Storage::open_in_memory().await.unwrap();
        target.enable_sync().await.unwrap();
        let first_import = import_sync_directory(&target, directory.path(), &key, Utc::now())
            .await
            .unwrap();
        let second_import = import_sync_directory(&target, directory.path(), &key, Utc::now())
            .await
            .unwrap();
        assert_eq!(first_import.imported, MAX_EVENTS_PER_DIRECTORY_IMPORT);
        assert_eq!(second_import.imported, 1);
        assert_eq!(second_import.pending, 0);
    }

    #[tokio::test]
    async fn directory_import_validates_every_file_before_touching_sqlite() {
        let key = group_key();
        let source = Storage::open_in_memory().await.unwrap();
        source.enable_sync().await.unwrap();
        source
            .add_feed("https://sync.example/feed", None)
            .await
            .unwrap();
        let directory = tempfile::tempdir().unwrap();
        export_sync_directory(&source, directory.path(), &key)
            .await
            .unwrap();
        let valid_path = segment_paths(directory.path()).pop().unwrap();
        let malformed = valid_path.parent().unwrap().join(segment_file_name(2, 2));
        fs::write(&malformed, b"{}\n").unwrap();

        let target = Storage::open_in_memory().await.unwrap();
        target.enable_sync().await.unwrap();
        assert!(
            import_sync_directory(&target, directory.path(), &key, Utc::now())
                .await
                .is_err()
        );
        assert!(target.list_feeds().await.unwrap().is_empty());
        let remote_events = target
            .local_sync_events_after(0, MAX_EVENTS_PER_DIRECTORY_IMPORT)
            .await
            .unwrap();
        assert!(remote_events.is_empty());
    }

    #[tokio::test]
    async fn directory_import_rejects_unknown_versions_and_oversized_files() {
        let key = group_key();
        let target = Storage::open_in_memory().await.unwrap();
        target.enable_sync().await.unwrap();
        let unknown = tempfile::tempdir().unwrap();
        fs::create_dir(unknown.path().join("v3")).unwrap();
        assert!(
            import_sync_directory(&target, unknown.path(), &key, Utc::now())
                .await
                .is_err()
        );

        let oversized = tempfile::tempdir().unwrap();
        let device = "00000000-0000-4000-8000-00000000000a";
        let directory = oversized
            .path()
            .join(FORMAT_DIRECTORY)
            .join(key.key_id())
            .join(device);
        fs::create_dir_all(&directory).unwrap();
        let file = directory.join(segment_file_name(1, 1));
        let handle = File::create(&file).unwrap();
        handle.set_len(MAX_SEGMENT_BYTES + 1).unwrap();
        assert!(
            import_sync_directory(&target, oversized.path(), &key, Utc::now())
                .await
                .is_err()
        );
        assert!(target.list_feeds().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn directory_import_rejects_corrupted_ciphertext_without_side_effects() {
        let key = group_key();
        let source = Storage::open_in_memory().await.unwrap();
        source.enable_sync().await.unwrap();
        source
            .add_feed("https://sync.example/feed", None)
            .await
            .unwrap();
        let directory = tempfile::tempdir().unwrap();
        export_sync_directory(&source, directory.path(), &key)
            .await
            .unwrap();
        let path = segment_paths(directory.path()).pop().unwrap();
        let mut segment = read_encrypted_segment_file(&path).unwrap();
        let replacement = if segment.ciphertext_base64.starts_with('A') {
            "B"
        } else {
            "A"
        };
        segment.ciphertext_base64.replace_range(..1, replacement);
        fs::write(&path, serialize_encrypted_segment(&segment).unwrap()).unwrap();

        let target = Storage::open_in_memory().await.unwrap();
        target.enable_sync().await.unwrap();
        assert!(
            import_sync_directory(&target, directory.path(), &key, Utc::now())
                .await
                .is_err()
        );
        assert!(target.list_feeds().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn repeated_linux_android_linux_directory_exchange_converges() {
        let key = group_key();
        let directory = tempfile::tempdir().unwrap();
        let linux = Storage::open_in_memory().await.unwrap();
        linux.enable_sync().await.unwrap();
        let feed = linux
            .add_feed("https://sync.example/feed", None)
            .await
            .unwrap();
        let article = cached_article(&feed.id, "entry");
        linux
            .upsert_articles(std::slice::from_ref(&article))
            .await
            .unwrap();
        linux.set_read(&article.id, true).await.unwrap();
        export_sync_directory(&linux, directory.path(), &key)
            .await
            .unwrap();
        assert!(segment_paths(directory.path()).iter().all(|path| {
            let visible = String::from_utf8_lossy(&fs::read(path).unwrap()).into_owned();
            !visible.contains("Local cached body")
                && !visible.contains("A synchronized article")
                && !visible.contains("https://sync.example")
                && !visible.contains("article_read_set")
        }));

        let android = Storage::open_in_memory().await.unwrap();
        android.enable_sync().await.unwrap();
        let first_import = import_sync_directory(&android, directory.path(), &key, Utc::now())
            .await
            .unwrap();
        assert_eq!(first_import.imported, 2);
        let android_article = android.list_articles().await.unwrap().pop().unwrap();
        assert_eq!(
            article_state(&android_article),
            (true, false, Some("A synchronized article"))
        );
        android
            .set_read(&android_article.article.id, false)
            .await
            .unwrap();
        android
            .set_favorite(&android_article.article.id, true)
            .await
            .unwrap();
        export_sync_directory(&android, directory.path(), &key)
            .await
            .unwrap();

        import_sync_directory(&linux, directory.path(), &key, Utc::now())
            .await
            .unwrap();
        import_sync_directory(&android, directory.path(), &key, Utc::now())
            .await
            .unwrap();
        let linux_article = linux.list_articles().await.unwrap().pop().unwrap();
        let android_article = android.list_articles().await.unwrap().pop().unwrap();
        assert_eq!(
            article_state(&linux_article),
            article_state(&android_article)
        );
        assert_eq!(
            article_state(&linux_article),
            (false, true, Some("A synchronized article"))
        );

        let repeated = import_sync_directory(&linux, directory.path(), &key, Utc::now())
            .await
            .unwrap();
        assert_eq!(repeated.imported, 0);
        assert_eq!(repeated.received, 0);
        assert!(!directory.path().join("reader.db").exists());
        assert!(!directory.path().join("reader.db-wal").exists());
        assert!(!directory.path().join("reader.db-shm").exists());
    }
}
