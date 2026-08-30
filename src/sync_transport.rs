use crate::storage::{MAX_SYNC_EVENTS_COMPACTED_PER_CYCLE, Storage};
use crate::sync_acknowledgements::{
    MAX_ACKNOWLEDGEMENT_BYTES, prepare_acknowledgement, read_acknowledgement,
};
use crate::sync_roster::{MAX_ROSTER_BYTES, prepare_roster, read_roster, roster_path};
use crate::sync_segments::{
    MAX_DIRECTORY_SEGMENTS, MAX_SEGMENT_BYTES, SyncDirectoryImportReport, SyncGroupKey,
    confirm_sync_segment_export, import_sync_segment_blobs, prepare_sync_export,
    verify_prepared_segment_bytes,
};
use crate::sync_snapshots::{
    MAX_SNAPSHOT_BYTES, MAX_SNAPSHOT_DEVICES, PreparedSnapshot, SnapshotPreparation,
    import_snapshot, prepare_snapshot, snapshot_creator, verify_snapshot,
};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use futures_util::stream::{self, StreamExt, TryStreamExt};
use std::collections::{HashMap, HashSet};

const MAX_CONCURRENT_DOWNLOADS: usize = 4;
const MAX_SEGMENTS_PER_SYNC: usize = 20;
const MAX_SNAPSHOTS_PER_SYNC: usize = 8;
const MAX_SEGMENT_DELETIONS_PER_SYNC: usize = 20;

/// Outcome of publishing one immutable encrypted segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentPublishOutcome {
    Created,
    AlreadyExists,
}

/// Transport contract for immutable encrypted synchronization segments.
///
/// Implementations own remote layout, atomic publication and bounded reads.
/// They never receive a group key or decrypted business event.
pub trait SegmentTransport: Sync {
    /// Ensures the version, key and local-device collections exist.
    fn ensure_layout(
        &self,
        key_id: &str,
        device_id: &str,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Publishes bytes without replacing an existing immutable path.
    fn publish_immutable(
        &self,
        relative_path: &str,
        bytes: &[u8],
    ) -> impl Future<Output = Result<SegmentPublishOutcome>> + Send;

    /// Lists all segment paths for the selected key, without downloading them.
    fn list_segments(&self, key_id: &str) -> impl Future<Output = Result<Vec<String>>> + Send;

    /// Downloads one segment and rejects a response larger than `max_bytes`.
    fn download_segment(
        &self,
        relative_path: &str,
        max_bytes: usize,
    ) -> impl Future<Output = Result<Vec<u8>>> + Send;

    /// Idempotently deletes one validated immutable segment path. A missing
    /// remote file is considered a successful cleanup.
    fn delete_segment(&self, relative_path: &str) -> impl Future<Output = Result<()>> + Send;

    /// Atomically replaces the local device's encrypted acknowledgement.
    fn publish_acknowledgement(
        &self,
        relative_path: &str,
        bytes: &[u8],
    ) -> impl Future<Output = Result<()>> + Send;

    /// Lists the encrypted acknowledgement documents for one group key.
    fn list_acknowledgements(
        &self,
        key_id: &str,
    ) -> impl Future<Output = Result<Vec<String>>> + Send;

    /// Downloads one bounded encrypted acknowledgement document.
    fn download_acknowledgement(
        &self,
        relative_path: &str,
        max_bytes: usize,
    ) -> impl Future<Output = Result<Vec<u8>>> + Send;

    /// Atomically replaces the local device's encrypted recovery snapshot.
    fn publish_snapshot(
        &self,
        relative_path: &str,
        bytes: &[u8],
    ) -> impl Future<Output = Result<()>> + Send;

    /// Lists the encrypted per-device recovery snapshots for one group.
    fn list_snapshots(&self, key_id: &str) -> impl Future<Output = Result<Vec<String>>> + Send;

    /// Downloads one bounded encrypted recovery snapshot.
    fn download_snapshot(
        &self,
        relative_path: &str,
        max_bytes: usize,
    ) -> impl Future<Output = Result<Vec<u8>>> + Send;

    fn publish_roster(
        &self,
        relative_path: &str,
        bytes: &[u8],
    ) -> impl Future<Output = Result<()>> + Send;

    fn list_rosters(&self, key_id: &str) -> impl Future<Output = Result<Vec<String>>> + Send;

    fn download_roster(
        &self,
        relative_path: &str,
        max_bytes: usize,
    ) -> impl Future<Output = Result<Vec<u8>>> + Send;
}

/// Non-sensitive counters from one upload/download/merge cycle.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SyncTransportReport {
    pub uploaded_segments: usize,
    pub reused_segments: usize,
    pub exported_events: usize,
    pub downloaded_segments: usize,
    pub received_events: usize,
    pub imported_events: usize,
    pub duplicate_events: usize,
    pub applied_events: usize,
    pub pending_events: usize,
    pub compacted_events: usize,
    pub deleted_segments: usize,
    pub deferred_segment_deletions: usize,
}

/// Exchanges all currently available encrypted segments through one transport.
///
/// Every successfully published segment advances its local export cursor only
/// after the transport confirms creation or an existing file decrypts to the
/// same immutable events. Downloads are fully authenticated before the merge
/// transaction starts.
pub async fn synchronize_transport<T: SegmentTransport>(
    storage: &Storage,
    key: &SyncGroupKey,
    transport: &T,
    observed_at: DateTime<Utc>,
) -> Result<SyncTransportReport> {
    let prepared = prepare_sync_export(storage, key).await?;
    transport
        .ensure_layout(&prepared.key_id, &prepared.device_id)
        .await
        .context("Impossible de préparer le transport de synchronisation")?;

    storage
        .seed_sync_roster(&prepared.key_id, observed_at)
        .await?;
    let local_roster_path = roster_path(&prepared.key_id, &prepared.device_id);
    let mut roster_paths = transport
        .list_rosters(&prepared.key_id)
        .await
        .context("Impossible de lister les registres des appareils")?;
    roster_paths.sort();
    roster_paths.dedup();
    if roster_paths.len() > 256 {
        bail!("Le transport expose trop de registres d'appareils");
    }
    for relative_path in roster_paths {
        if relative_path == local_roster_path {
            continue;
        }
        let bytes = transport
            .download_roster(&relative_path, MAX_ROSTER_BYTES)
            .await
            .with_context(|| format!("Impossible de télécharger {relative_path}"))?;
        let roster = read_roster(&relative_path, &bytes, key)?;
        storage
            .merge_sync_roster(&prepared.key_id, &roster.members, observed_at)
            .await?;
    }
    let active_roster = storage
        .active_sync_roster_device_ids(&prepared.key_id)
        .await?;
    if !active_roster.contains(&prepared.device_id) {
        bail!("Cet appareil a été révoqué du groupe de synchronisation");
    }

    let mut report = SyncTransportReport::default();
    let mut cursor = prepared.initial_cursor;
    for segment in &prepared.segments {
        let outcome = transport
            .publish_immutable(&segment.relative_path, &segment.bytes)
            .await
            .with_context(|| {
                format!(
                    "Impossible de publier le segment {}-{}",
                    segment.first_sequence, segment.last_sequence
                )
            })?;
        match outcome {
            SegmentPublishOutcome::Created => report.uploaded_segments += 1,
            SegmentPublishOutcome::AlreadyExists => {
                let existing = transport
                    .download_segment(&segment.relative_path, MAX_SEGMENT_BYTES as usize)
                    .await
                    .context("Impossible de vérifier le segment distant existant")?;
                verify_prepared_segment_bytes(&existing, segment, key)?;
                report.reused_segments += 1;
            }
        }
        confirm_sync_segment_export(storage, &prepared.key_id, cursor, segment.last_sequence)
            .await?;
        cursor = segment.last_sequence;
        report.exported_events += segment.event_count;
    }
    let local_roster = prepare_roster(storage, key, observed_at).await?;
    transport
        .publish_roster(&local_roster.relative_path, &local_roster.bytes)
        .await
        .context("Impossible de publier le registre des appareils")?;

    let mut paths = transport
        .list_segments(&prepared.key_id)
        .await
        .context("Impossible de lister les segments distants")?;
    paths.sort();
    paths.dedup();
    if paths.len() > MAX_DIRECTORY_SEGMENTS {
        bail!(
            "A synchronization transport cannot expose more than {MAX_DIRECTORY_SEGMENTS} segments"
        );
    }
    let mut cursors = HashMap::new();
    let mut expected_sequences = HashMap::new();
    let mut gap_devices = HashSet::new();
    let mut local_segments = Vec::new();
    for path in &paths {
        let (device_id, first_sequence, last_sequence) =
            segment_path_identity(path, &prepared.key_id)?;
        if device_id == prepared.device_id {
            local_segments.push((path.clone(), first_sequence, last_sequence));
            continue;
        }
        if device_is_revoked(storage, &prepared.key_id, device_id).await? {
            continue;
        }
        let cursor = match cursors.get(device_id) {
            Some(cursor) => *cursor,
            None => {
                let cursor = storage.sync_import_cursor(device_id).await?;
                cursors.insert(device_id.to_string(), cursor);
                cursor
            }
        };
        if last_sequence <= cursor {
            continue;
        }
        let expected = expected_sequences
            .entry(device_id.to_string())
            .or_insert(cursor + 1);
        if first_sequence > *expected {
            gap_devices.insert(device_id.to_string());
        }
        *expected = (*expected).max(last_sequence + 1);
    }

    let mut snapshot_paths = transport
        .list_snapshots(&prepared.key_id)
        .await
        .context("Impossible de lister les instantanés")?;
    snapshot_paths.sort();
    snapshot_paths.dedup();
    if snapshot_paths.len() > MAX_SNAPSHOT_DEVICES {
        bail!("Le transport expose trop d'instantanés de récupération");
    }
    let mut snapshot_candidates = Vec::new();
    for relative_path in snapshot_paths {
        let creator = snapshot_creator(&relative_path, &prepared.key_id)?;
        if creator == prepared.device_id
            || device_is_revoked(storage, &prepared.key_id, creator).await?
        {
            continue;
        }
        let never_imported = storage
            .sync_snapshot_import_hash(&prepared.key_id, creator)
            .await?
            .is_none();
        if !never_imported && gap_devices.is_empty() {
            continue;
        }
        let priority = if gap_devices.contains(creator) {
            0
        } else if never_imported {
            1
        } else {
            2
        };
        snapshot_candidates.push((priority, relative_path));
    }
    snapshot_candidates.sort();
    for (_, relative_path) in snapshot_candidates.into_iter().take(MAX_SNAPSHOTS_PER_SYNC) {
        let bytes = transport
            .download_snapshot(&relative_path, MAX_SNAPSHOT_BYTES)
            .await
            .with_context(|| format!("Impossible de télécharger {relative_path}"))?;
        let imported = import_snapshot(storage, key, &relative_path, &bytes, observed_at).await?;
        accumulate_snapshot_import_report(&mut report, imported);
    }

    cursors.clear();
    let mut missing_paths = Vec::new();
    for path in paths {
        let (device_id, _first_sequence, last_sequence) =
            segment_path_identity(&path, &prepared.key_id)?;
        if device_id == prepared.device_id
            || device_is_revoked(storage, &prepared.key_id, device_id).await?
        {
            continue;
        }
        let cursor = match cursors.get(device_id) {
            Some(cursor) => *cursor,
            None => {
                let cursor = storage.sync_import_cursor(device_id).await?;
                cursors.insert(device_id.to_string(), cursor);
                cursor
            }
        };
        if last_sequence > cursor && missing_paths.len() < MAX_SEGMENTS_PER_SYNC {
            missing_paths.push(path);
        }
    }
    let blobs = stream::iter(missing_paths.into_iter().map(|relative_path| async move {
        let bytes = transport
            .download_segment(&relative_path, MAX_SEGMENT_BYTES as usize)
            .await
            .with_context(|| format!("Impossible de télécharger {relative_path}"))?;
        Ok::<_, anyhow::Error>((relative_path, bytes))
    }))
    .buffer_unordered(MAX_CONCURRENT_DOWNLOADS)
    .try_collect::<Vec<_>>()
    .await?;
    report.downloaded_segments = blobs.len();

    let imported = import_sync_segment_blobs(storage, key, blobs, observed_at).await?;
    accumulate_directory_import_report(&mut report, imported);

    let local_acknowledgement = prepare_acknowledgement(storage, key, observed_at).await?;
    transport
        .publish_acknowledgement(
            &local_acknowledgement.relative_path,
            &local_acknowledgement.bytes,
        )
        .await
        .context("Impossible de publier l'accusé de réception")?;
    let mut acknowledgement_paths = transport
        .list_acknowledgements(&prepared.key_id)
        .await
        .context("Impossible de lister les accusés de réception")?;
    acknowledgement_paths.sort();
    acknowledgement_paths.dedup();
    if acknowledgement_paths.len() > 256 {
        bail!("Le transport expose trop d'accusés de réception");
    }
    for relative_path in acknowledgement_paths {
        if relative_path == local_acknowledgement.relative_path {
            continue;
        }
        let bytes = transport
            .download_acknowledgement(&relative_path, MAX_ACKNOWLEDGEMENT_BYTES)
            .await
            .with_context(|| format!("Impossible de télécharger {relative_path}"))?;
        let acknowledgements = read_acknowledgement(&relative_path, &bytes, key)?;
        if device_is_revoked(
            storage,
            &prepared.key_id,
            &acknowledgements[0].observer_device_id,
        )
        .await?
        {
            continue;
        }
        storage
            .record_sync_acknowledgements(&acknowledgements)
            .await?;
    }

    let checkpoint_frontiers =
        ensure_recovery_checkpoint(storage, key, transport, observed_at).await?;
    report.deferred_segment_deletions = local_segments.len();
    if let Some(checkpoint_frontiers) = checkpoint_frontiers {
        report.compacted_events = storage
            .compact_sync_events(
                &prepared.key_id,
                &checkpoint_frontiers,
                MAX_SYNC_EVENTS_COMPACTED_PER_CYCLE,
            )
            .await?;
        delete_confirmed_local_segments(
            storage,
            &prepared.key_id,
            &prepared.device_id,
            &checkpoint_frontiers,
            &local_segments,
            transport,
            &mut report,
        )
        .await?;
    }
    Ok(report)
}

async fn ensure_recovery_checkpoint<T: SegmentTransport>(
    storage: &Storage,
    key: &SyncGroupKey,
    transport: &T,
    observed_at: DateTime<Utc>,
) -> Result<Option<Vec<(String, i64)>>> {
    match prepare_snapshot(storage, key, observed_at).await? {
        SnapshotPreparation::Ready(snapshot) => {
            publish_and_verify_snapshot(&snapshot, key, transport, "publier").await?;
            storage
                .record_sync_snapshot_publication(
                    &snapshot.key_id,
                    &snapshot.creator_device_id,
                    &snapshot.state_hash,
                    observed_at,
                )
                .await?;
            Ok(Some(snapshot.frontiers))
        }
        SnapshotPreparation::ConfirmedUnchanged(snapshot) => {
            let remote_is_valid = match transport
                .download_snapshot(&snapshot.relative_path, MAX_SNAPSHOT_BYTES)
                .await
            {
                Ok(bytes) => {
                    verify_snapshot(&snapshot.relative_path, &bytes, key, &snapshot.state_hash)
                        .is_ok()
                }
                Err(_) => false,
            };
            if !remote_is_valid {
                publish_and_verify_snapshot(&snapshot, key, transport, "réparer").await?;
                storage
                    .record_sync_snapshot_publication(
                        &snapshot.key_id,
                        &snapshot.creator_device_id,
                        &snapshot.state_hash,
                        observed_at,
                    )
                    .await?;
            }
            Ok(Some(snapshot.frontiers))
        }
        SnapshotPreparation::Unavailable(_reason) => Ok(None),
    }
}

async fn publish_and_verify_snapshot<T: SegmentTransport>(
    snapshot: &PreparedSnapshot,
    key: &SyncGroupKey,
    transport: &T,
    action: &str,
) -> Result<()> {
    transport
        .publish_snapshot(&snapshot.relative_path, &snapshot.bytes)
        .await
        .with_context(|| format!("Impossible de {action} l'instantané de récupération"))?;
    let remote = transport
        .download_snapshot(&snapshot.relative_path, MAX_SNAPSHOT_BYTES)
        .await
        .context("Impossible de vérifier l'instantané publié")?;
    verify_snapshot(&snapshot.relative_path, &remote, key, &snapshot.state_hash)
        .context("L'instantané publié ne peut pas être authentifié")
}

async fn delete_confirmed_local_segments<T: SegmentTransport>(
    storage: &Storage,
    key_id: &str,
    local_device_id: &str,
    checkpoint_frontiers: &[(String, i64)],
    local_segments: &[(String, i64, i64)],
    transport: &T,
    report: &mut SyncTransportReport,
) -> Result<()> {
    let checkpoint_frontier = checkpoint_frontiers
        .iter()
        .find_map(|(device_id, sequence)| (device_id == local_device_id).then_some(*sequence))
        .context("Le checkpoint confirmé ne couvre pas le journal local")?;
    let safe = storage
        .authoritative_sync_compaction_frontier(key_id, local_device_id, checkpoint_frontier)
        .await?;
    let mut candidates = local_segments
        .iter()
        .filter(|(_, _, last_sequence)| *last_sequence <= safe.safe_through_sequence)
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        left.2
            .cmp(&right.2)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.0.cmp(&right.0))
    });
    for (relative_path, _, _) in candidates.into_iter().take(MAX_SEGMENT_DELETIONS_PER_SYNC) {
        if transport.delete_segment(relative_path).await.is_err() {
            break;
        }
        report.deleted_segments += 1;
        report.deferred_segment_deletions -= 1;
    }
    Ok(())
}

async fn device_is_revoked(storage: &Storage, key_id: &str, device_id: &str) -> Result<bool> {
    Ok(storage.sync_device_is_revoked(device_id).await?
        || storage
            .sync_roster_device_is_revoked(key_id, device_id)
            .await?)
}

fn segment_path_identity<'a>(relative_path: &'a str, key_id: &str) -> Result<(&'a str, i64, i64)> {
    let components = relative_path.split('/').collect::<Vec<_>>();
    if components.len() != 4 || components[0] != "v2" || components[1] != key_id {
        bail!("Transport returned an invalid synchronization segment path");
    }
    uuid::Uuid::parse_str(components[2])
        .context("Transport returned an invalid device identifier")?;
    let range = components[3]
        .strip_suffix(".json")
        .context("Transport returned an invalid segment file name")?;
    let (first, last) = range
        .split_once('-')
        .context("Transport returned an invalid segment sequence range")?;
    if first.len() != 20
        || last.len() != 20
        || !first.bytes().all(|byte| byte.is_ascii_digit())
        || !last.bytes().all(|byte| byte.is_ascii_digit())
    {
        bail!("Transport returned an invalid segment sequence range");
    }
    let first_sequence = first
        .parse::<i64>()
        .context("Transport returned an excessive segment sequence")?;
    let last_sequence = last
        .parse::<i64>()
        .context("Transport returned an excessive segment sequence")?;
    if first_sequence <= 0 || last_sequence < first_sequence {
        bail!("Transport returned an invalid segment sequence range");
    }
    Ok((components[2], first_sequence, last_sequence))
}

fn accumulate_directory_import_report(
    report: &mut SyncTransportReport,
    imported: SyncDirectoryImportReport,
) {
    report.received_events += imported.received;
    report.imported_events += imported.imported;
    report.duplicate_events += imported.duplicates;
    report.applied_events += imported.applied;
    report.pending_events = imported.pending;
}

fn accumulate_snapshot_import_report(
    report: &mut SyncTransportReport,
    imported: crate::sync::SyncImportReport,
) {
    report.received_events += imported.received;
    report.imported_events += imported.imported;
    report.duplicate_events += imported.duplicates;
    report.applied_events += imported.applied;
    report.pending_events = imported.pending;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[derive(Clone, Default)]
    struct MemoryTransport {
        files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
        reject_snapshots: bool,
        reject_deletions: bool,
        fail_deletion_after: Option<usize>,
        deleted_paths: Arc<Mutex<Vec<String>>>,
    }

    impl SegmentTransport for MemoryTransport {
        async fn ensure_layout(&self, _key_id: &str, _device_id: &str) -> Result<()> {
            Ok(())
        }

        async fn publish_immutable(
            &self,
            relative_path: &str,
            bytes: &[u8],
        ) -> Result<SegmentPublishOutcome> {
            let mut files = self.files.lock().unwrap();
            match files.get(relative_path) {
                Some(existing) if existing == bytes => Ok(SegmentPublishOutcome::AlreadyExists),
                Some(_) => bail!("immutable collision"),
                None => {
                    files.insert(relative_path.to_string(), bytes.to_vec());
                    Ok(SegmentPublishOutcome::Created)
                }
            }
        }

        async fn list_segments(&self, key_id: &str) -> Result<Vec<String>> {
            let prefix = format!("v2/{key_id}/");
            Ok(self
                .files
                .lock()
                .unwrap()
                .keys()
                .filter(|path| {
                    path.starts_with(&prefix)
                        && !path.contains("/acknowledgements/")
                        && !path.contains("/snapshots/")
                        && !path.contains("/rosters/")
                })
                .cloned()
                .collect())
        }

        async fn download_segment(&self, relative_path: &str, max_bytes: usize) -> Result<Vec<u8>> {
            let bytes = self
                .files
                .lock()
                .unwrap()
                .get(relative_path)
                .cloned()
                .context("missing memory transport file")?;
            if bytes.len() > max_bytes {
                bail!("memory transport file too large");
            }
            Ok(bytes)
        }

        async fn delete_segment(&self, relative_path: &str) -> Result<()> {
            if self.reject_deletions
                || self
                    .fail_deletion_after
                    .is_some_and(|maximum| self.deleted_paths.lock().unwrap().len() >= maximum)
            {
                bail!("segment deletion rejected for test");
            }
            self.files.lock().unwrap().remove(relative_path);
            self.deleted_paths
                .lock()
                .unwrap()
                .push(relative_path.to_string());
            Ok(())
        }

        async fn publish_acknowledgement(&self, relative_path: &str, bytes: &[u8]) -> Result<()> {
            self.files
                .lock()
                .unwrap()
                .insert(relative_path.to_string(), bytes.to_vec());
            Ok(())
        }

        async fn list_acknowledgements(&self, key_id: &str) -> Result<Vec<String>> {
            let prefix = format!("v2/{key_id}/acknowledgements/");
            Ok(self
                .files
                .lock()
                .unwrap()
                .keys()
                .filter(|path| path.starts_with(&prefix))
                .cloned()
                .collect())
        }

        async fn download_acknowledgement(
            &self,
            relative_path: &str,
            max_bytes: usize,
        ) -> Result<Vec<u8>> {
            self.download_segment(relative_path, max_bytes).await
        }

        async fn publish_snapshot(&self, relative_path: &str, bytes: &[u8]) -> Result<()> {
            if self.reject_snapshots {
                bail!("snapshot publication rejected for test");
            }
            self.files
                .lock()
                .unwrap()
                .insert(relative_path.to_string(), bytes.to_vec());
            Ok(())
        }

        async fn list_snapshots(&self, key_id: &str) -> Result<Vec<String>> {
            let prefix = format!("v2/{key_id}/snapshots/");
            Ok(self
                .files
                .lock()
                .unwrap()
                .keys()
                .filter(|path| path.starts_with(&prefix))
                .cloned()
                .collect())
        }

        async fn download_snapshot(
            &self,
            relative_path: &str,
            max_bytes: usize,
        ) -> Result<Vec<u8>> {
            self.download_segment(relative_path, max_bytes).await
        }

        async fn publish_roster(&self, relative_path: &str, bytes: &[u8]) -> Result<()> {
            self.files
                .lock()
                .unwrap()
                .insert(relative_path.to_string(), bytes.to_vec());
            Ok(())
        }

        async fn list_rosters(&self, key_id: &str) -> Result<Vec<String>> {
            let prefix = format!("v2/{key_id}/rosters/");
            Ok(self
                .files
                .lock()
                .unwrap()
                .keys()
                .filter(|path| path.starts_with(&prefix))
                .cloned()
                .collect())
        }

        async fn download_roster(&self, relative_path: &str, max_bytes: usize) -> Result<Vec<u8>> {
            self.download_segment(relative_path, max_bytes).await
        }
    }

    struct ConcurrentDownloadTransport {
        active: AtomicUsize,
        maximum: AtomicUsize,
        completed: AtomicUsize,
    }

    impl SegmentTransport for ConcurrentDownloadTransport {
        async fn ensure_layout(&self, _key_id: &str, _device_id: &str) -> Result<()> {
            Ok(())
        }

        async fn publish_immutable(
            &self,
            _relative_path: &str,
            _bytes: &[u8],
        ) -> Result<SegmentPublishOutcome> {
            unreachable!("the test storage has no local event")
        }

        async fn list_segments(&self, key_id: &str) -> Result<Vec<String>> {
            Ok((0..24)
                .map(|index| {
                    let sequence = index + 1;
                    format!(
                        "v2/{key_id}/00000000-0000-4000-8000-000000000001/{sequence:020}-{sequence:020}.json"
                    )
                })
                .collect())
        }

        async fn download_segment(
            &self,
            _relative_path: &str,
            _max_bytes: usize,
        ) -> Result<Vec<u8>> {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.maximum.fetch_max(active, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            self.active.fetch_sub(1, Ordering::SeqCst);
            self.completed.fetch_add(1, Ordering::SeqCst);
            Ok(b"{}".to_vec())
        }

        async fn delete_segment(&self, _relative_path: &str) -> Result<()> {
            Ok(())
        }

        async fn publish_acknowledgement(&self, _relative_path: &str, _bytes: &[u8]) -> Result<()> {
            Ok(())
        }

        async fn list_acknowledgements(&self, _key_id: &str) -> Result<Vec<String>> {
            Ok(Vec::new())
        }

        async fn download_acknowledgement(
            &self,
            _relative_path: &str,
            _max_bytes: usize,
        ) -> Result<Vec<u8>> {
            unreachable!()
        }

        async fn publish_snapshot(&self, _relative_path: &str, _bytes: &[u8]) -> Result<()> {
            Ok(())
        }

        async fn list_snapshots(&self, _key_id: &str) -> Result<Vec<String>> {
            Ok(Vec::new())
        }

        async fn download_snapshot(
            &self,
            _relative_path: &str,
            _max_bytes: usize,
        ) -> Result<Vec<u8>> {
            unreachable!()
        }

        async fn publish_roster(&self, _relative_path: &str, _bytes: &[u8]) -> Result<()> {
            Ok(())
        }

        async fn list_rosters(&self, _key_id: &str) -> Result<Vec<String>> {
            Ok(Vec::new())
        }

        async fn download_roster(
            &self,
            _relative_path: &str,
            _max_bytes: usize,
        ) -> Result<Vec<u8>> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn remote_segment_downloads_are_limited_to_four_concurrent_requests() {
        let storage = Storage::open_in_memory().await.unwrap();
        storage.enable_sync().await.unwrap();
        let transport = ConcurrentDownloadTransport {
            active: AtomicUsize::new(0),
            maximum: AtomicUsize::new(0),
            completed: AtomicUsize::new(0),
        };
        let key = SyncGroupKey::from_bytes([0x55; 32]);
        assert!(
            synchronize_transport(&storage, &key, &transport, Utc::now())
                .await
                .is_err()
        );
        assert_eq!(transport.maximum.load(Ordering::SeqCst), 4);
        assert_eq!(transport.active.load(Ordering::SeqCst), 0);
        assert_eq!(transport.completed.load(Ordering::SeqCst), 20);
        assert!(storage.list_feeds().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn encrypted_acknowledgements_cross_devices_and_unlock_the_safe_frontier() {
        let linux = Storage::open_in_memory().await.unwrap();
        let android = Storage::open_in_memory().await.unwrap();
        linux.enable_sync().await.unwrap();
        android.enable_sync().await.unwrap();
        linux
            .add_feed("https://ack.example/feed", None)
            .await
            .unwrap();
        let linux_id = linux.sync_identity().await.unwrap().device_id;
        let android_id = android.sync_identity().await.unwrap().device_id;
        let key = SyncGroupKey::from_bytes([0xa1; 32]);
        let key_id = key.key_id();
        let transport = MemoryTransport::default();

        synchronize_transport(&linux, &key, &transport, Utc::now())
            .await
            .unwrap();
        synchronize_transport(&android, &key, &transport, Utc::now())
            .await
            .unwrap();
        synchronize_transport(&linux, &key, &transport, Utc::now())
            .await
            .unwrap();

        let acknowledgements = linux
            .sync_acknowledgements_for_source(&key_id, &linux_id)
            .await
            .unwrap();
        assert!(acknowledgements.iter().any(|acknowledgement| {
            acknowledgement.observer_device_id == android_id
                && acknowledgement.contiguous_sequence == 1
        }));
        let frontier = linux
            .sync_compaction_frontier(&key_id, &linux_id, 1, &[linux_id.clone(), android_id])
            .await
            .unwrap();
        assert_eq!(frontier.safe_through_sequence, 1);
        assert!(frontier.blocking_observer_device_ids.is_empty());
    }

    #[tokio::test]
    async fn confirmed_checkpoint_triggers_bounded_local_compaction() {
        let storage = Storage::open_in_memory().await.unwrap();
        storage.enable_sync().await.unwrap();
        let feed = storage
            .add_feed("https://transport-compaction.example/feed", None)
            .await
            .unwrap();
        for index in 0..12 {
            storage
                .set_feed_active(&feed.id, index % 2 == 0)
                .await
                .unwrap();
        }
        let key = SyncGroupKey::from_bytes([0xa2; 32]);
        let transport = MemoryTransport::default();

        let first = synchronize_transport(&storage, &key, &transport, Utc::now())
            .await
            .unwrap();
        assert_eq!(first.compacted_events, 11);
        assert_eq!(
            storage.local_sync_events_after(0, 100).await.unwrap().len(),
            2
        );

        let second = synchronize_transport(&storage, &key, &transport, Utc::now())
            .await
            .unwrap();
        assert_eq!(second.compacted_events, 0);
    }

    #[tokio::test]
    async fn failed_checkpoint_publication_prevents_local_compaction() {
        let storage = Storage::open_in_memory().await.unwrap();
        storage.enable_sync().await.unwrap();
        let feed = storage
            .add_feed("https://unconfirmed-compaction.example/feed", None)
            .await
            .unwrap();
        storage.set_feed_active(&feed.id, false).await.unwrap();
        storage.set_feed_active(&feed.id, true).await.unwrap();
        let before = storage.local_sync_events_after(0, 100).await.unwrap();
        let transport = MemoryTransport {
            reject_snapshots: true,
            ..MemoryTransport::default()
        };

        let error = synchronize_transport(
            &storage,
            &SyncGroupKey::from_bytes([0xa3; 32]),
            &transport,
            Utc::now(),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("instantané"));
        assert_eq!(
            storage.local_sync_events_after(0, 100).await.unwrap(),
            before
        );
    }

    #[tokio::test]
    async fn remote_cleanup_waits_for_full_acknowledgement_of_a_segment() {
        let storage = Storage::open_in_memory().await.unwrap();
        storage.enable_sync().await.unwrap();
        let feed = storage
            .add_feed("https://segment-frontier.example/feed", None)
            .await
            .unwrap();
        storage.set_feed_active(&feed.id, false).await.unwrap();
        storage.set_feed_active(&feed.id, true).await.unwrap();
        let local = storage.sync_identity().await.unwrap();
        let remote_id = "00000000-0000-4000-8000-000000000064";
        let observed_at = Utc::now();
        storage
            .register_sync_device(remote_id, "Remote", observed_at)
            .await
            .unwrap();
        let key = SyncGroupKey::from_bytes([0xa4; 32]);
        let transport = MemoryTransport::default();

        let blocked = synchronize_transport(&storage, &key, &transport, observed_at)
            .await
            .unwrap();
        assert_eq!(blocked.deleted_segments, 0);
        assert_eq!(blocked.deferred_segment_deletions, 1);
        storage
            .record_sync_acknowledgement(&crate::storage::SyncAcknowledgement {
                key_id: key.key_id(),
                observer_device_id: remote_id.to_string(),
                source_device_id: local.device_id.clone(),
                contiguous_sequence: 2,
                observed_at,
            })
            .await
            .unwrap();

        let partial = synchronize_transport(&storage, &key, &transport, observed_at)
            .await
            .unwrap();
        assert_eq!(partial.deleted_segments, 0);
        assert_eq!(partial.deferred_segment_deletions, 1);
        storage
            .record_sync_acknowledgement(&crate::storage::SyncAcknowledgement {
                key_id: key.key_id(),
                observer_device_id: remote_id.to_string(),
                source_device_id: local.device_id.clone(),
                contiguous_sequence: local.next_sequence - 1,
                observed_at,
            })
            .await
            .unwrap();

        let cleaned = synchronize_transport(&storage, &key, &transport, observed_at)
            .await
            .unwrap();
        assert_eq!(cleaned.deleted_segments, 1);
        assert_eq!(cleaned.deferred_segment_deletions, 0);
        assert!(
            transport
                .deleted_paths
                .lock()
                .unwrap()
                .iter()
                .all(|path| path.contains(&format!("/{}/", local.device_id)))
        );
    }

    #[tokio::test]
    async fn remote_cleanup_failure_is_non_blocking_and_retried_later() {
        let storage = Storage::open_in_memory().await.unwrap();
        storage.enable_sync().await.unwrap();
        let feed = storage
            .add_feed("https://deferred-cleanup.example/feed", None)
            .await
            .unwrap();
        storage.set_feed_active(&feed.id, false).await.unwrap();
        let failing = MemoryTransport {
            reject_deletions: true,
            ..MemoryTransport::default()
        };
        let key = SyncGroupKey::from_bytes([0xa5; 32]);

        let deferred = synchronize_transport(&storage, &key, &failing, Utc::now())
            .await
            .unwrap();
        assert_eq!(deferred.deleted_segments, 0);
        assert_eq!(deferred.deferred_segment_deletions, 1);
        let succeeding = MemoryTransport {
            files: Arc::clone(&failing.files),
            deleted_paths: Arc::clone(&failing.deleted_paths),
            ..MemoryTransport::default()
        };
        let cleaned = synchronize_transport(&storage, &key, &succeeding, Utc::now())
            .await
            .unwrap();
        assert_eq!(cleaned.deleted_segments, 1);
        assert_eq!(cleaned.deferred_segment_deletions, 0);
    }

    #[tokio::test]
    async fn remote_cleanup_never_deletes_another_device_journal() {
        let local = Storage::open_in_memory().await.unwrap();
        let remote = Storage::open_in_memory().await.unwrap();
        local.enable_sync().await.unwrap();
        remote.enable_sync().await.unwrap();
        local
            .add_feed("https://local-owner.example/feed", None)
            .await
            .unwrap();
        remote
            .add_feed("https://remote-owner.example/feed", None)
            .await
            .unwrap();
        let key = SyncGroupKey::from_bytes([0xa7; 32]);
        let local_export = prepare_sync_export(&local, &key).await.unwrap();
        let remote_export = prepare_sync_export(&remote, &key).await.unwrap();
        let local_path = local_export.segments[0].relative_path.clone();
        let remote_path = remote_export.segments[0].relative_path.clone();
        let transport = MemoryTransport::default();
        {
            let mut files = transport.files.lock().unwrap();
            files.insert(remote_path.clone(), remote_export.segments[0].bytes.clone());
        }

        let report = synchronize_transport(&local, &key, &transport, Utc::now())
            .await
            .unwrap();
        assert_eq!(report.deleted_segments, 1);
        let files = transport.files.lock().unwrap();
        assert!(!files.contains_key(&local_path));
        assert!(files.contains_key(&remote_path));
    }

    #[tokio::test]
    async fn remote_cleanup_is_limited_to_twenty_segments_per_cycle() {
        let storage = Storage::open_in_memory().await.unwrap();
        storage.enable_sync().await.unwrap();
        let feed = storage
            .add_feed("https://bounded-cleanup.example/feed", None)
            .await
            .unwrap();
        for index in 0..24 {
            storage
                .set_feed_active(&feed.id, index % 2 == 0)
                .await
                .unwrap();
        }
        let local = storage.sync_identity().await.unwrap();
        let key = SyncGroupKey::from_bytes([0xa6; 32]);
        let key_id = key.key_id();
        storage.seed_sync_roster(&key_id, Utc::now()).await.unwrap();
        let segments = (1..local.next_sequence)
            .map(|sequence| {
                (
                    format!(
                        "v2/{key_id}/{}/{sequence:020}-{sequence:020}.json",
                        local.device_id
                    ),
                    sequence,
                    sequence,
                )
            })
            .collect::<Vec<_>>();
        let transport = MemoryTransport::default();
        let mut report = SyncTransportReport {
            deferred_segment_deletions: segments.len(),
            ..SyncTransportReport::default()
        };

        delete_confirmed_local_segments(
            &storage,
            &key_id,
            &local.device_id,
            &[(local.device_id.clone(), local.next_sequence - 1)],
            &segments,
            &transport,
            &mut report,
        )
        .await
        .unwrap();
        assert_eq!(report.deleted_segments, 20);
        assert_eq!(report.deferred_segment_deletions, 5);
    }

    #[tokio::test]
    async fn restart_between_checkpoint_and_local_compaction_preserves_recovery_state() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("interrupted.db");
        let storage = Storage::open(&database_path).await.unwrap();
        storage.enable_sync().await.unwrap();
        let feed = storage
            .add_feed("https://checkpoint-interruption.example/feed", None)
            .await
            .unwrap();
        storage.set_feed_active(&feed.id, false).await.unwrap();
        storage.set_feed_active(&feed.id, true).await.unwrap();
        let key = SyncGroupKey::from_bytes([0xa8; 32]);
        let key_id = key.key_id();
        let observed_at = Utc::now();
        storage
            .seed_sync_roster(&key_id, observed_at)
            .await
            .unwrap();
        let transport = MemoryTransport::default();
        let checkpoint_frontiers =
            ensure_recovery_checkpoint(&storage, &key, &transport, observed_at)
                .await
                .unwrap()
                .unwrap();
        let before = storage.local_sync_events_after(0, 100).await.unwrap();
        storage.close().await;

        let reopened = Storage::open(&database_path).await.unwrap();
        assert_eq!(
            reopened.local_sync_events_after(0, 100).await.unwrap(),
            before
        );
        assert_eq!(
            reopened
                .compact_sync_events(&key_id, &checkpoint_frontiers, 100)
                .await
                .unwrap(),
            1
        );
        assert!(reopened.list_feeds().await.unwrap()[0].is_active);
    }

    #[tokio::test]
    async fn interruption_between_remote_deletions_resumes_from_remaining_segments() {
        let storage = Storage::open_in_memory().await.unwrap();
        storage.enable_sync().await.unwrap();
        let feed = storage
            .add_feed("https://delete-interruption.example/feed", None)
            .await
            .unwrap();
        for index in 0..504 {
            storage
                .set_feed_active(&feed.id, index % 2 == 0)
                .await
                .unwrap();
        }
        let key = SyncGroupKey::from_bytes([0xa9; 32]);
        let interrupted = MemoryTransport {
            fail_deletion_after: Some(1),
            ..MemoryTransport::default()
        };

        let first = synchronize_transport(&storage, &key, &interrupted, Utc::now())
            .await
            .unwrap();
        assert_eq!(first.deleted_segments, 1);
        assert_eq!(first.deferred_segment_deletions, 2);
        let resumed = MemoryTransport {
            files: Arc::clone(&interrupted.files),
            deleted_paths: Arc::clone(&interrupted.deleted_paths),
            ..MemoryTransport::default()
        };
        let second = synchronize_transport(&storage, &key, &resumed, Utc::now())
            .await
            .unwrap();
        assert_eq!(second.deleted_segments, 2);
        assert_eq!(second.deferred_segment_deletions, 0);
    }

    #[tokio::test]
    async fn missing_checkpoint_is_repaired_before_any_newly_safe_segment_is_deleted() {
        let storage = Storage::open_in_memory().await.unwrap();
        storage.enable_sync().await.unwrap();
        let feed = storage
            .add_feed("https://repair-before-delete.example/feed", None)
            .await
            .unwrap();
        storage.set_feed_active(&feed.id, false).await.unwrap();
        let remote_id = "00000000-0000-4000-8000-000000000065";
        storage
            .register_sync_device(remote_id, "Remote", Utc::now())
            .await
            .unwrap();
        let local = storage.sync_identity().await.unwrap();
        let key = SyncGroupKey::from_bytes([0xaa; 32]);
        let transport = MemoryTransport::default();
        let first = synchronize_transport(&storage, &key, &transport, Utc::now())
            .await
            .unwrap();
        assert_eq!(first.deferred_segment_deletions, 1);
        storage
            .record_sync_acknowledgement(&crate::storage::SyncAcknowledgement {
                key_id: key.key_id(),
                observer_device_id: remote_id.to_string(),
                source_device_id: local.device_id,
                contiguous_sequence: local.next_sequence - 1,
                observed_at: Utc::now(),
            })
            .await
            .unwrap();
        let snapshot_path = transport
            .files
            .lock()
            .unwrap()
            .keys()
            .find(|path| path.contains("/snapshots/"))
            .unwrap()
            .clone();
        transport.files.lock().unwrap().remove(&snapshot_path);
        let failed_repair = MemoryTransport {
            files: Arc::clone(&transport.files),
            deleted_paths: Arc::clone(&transport.deleted_paths),
            reject_snapshots: true,
            ..MemoryTransport::default()
        };

        assert!(
            synchronize_transport(&storage, &key, &failed_repair, Utc::now())
                .await
                .is_err()
        );
        assert!(failed_repair.deleted_paths.lock().unwrap().is_empty());
        let repaired = synchronize_transport(&storage, &key, &transport, Utc::now())
            .await
            .unwrap();
        assert!(transport.files.lock().unwrap().contains_key(&snapshot_path));
        assert_eq!(repaired.deleted_segments, 1);
    }

    #[tokio::test]
    async fn fresh_database_recovers_after_all_covered_segments_are_gone() {
        let source = Storage::open_in_memory().await.unwrap();
        source.enable_sync().await.unwrap();
        let feed = source
            .add_feed("https://checkpoint-only.example/feed", None)
            .await
            .unwrap();
        source.set_feed_active(&feed.id, false).await.unwrap();
        let key = SyncGroupKey::from_bytes([0xab; 32]);
        let key_id = key.key_id();
        let transport = MemoryTransport::default();
        let published = synchronize_transport(&source, &key, &transport, Utc::now())
            .await
            .unwrap();
        assert_eq!(published.deleted_segments, 1);
        assert!(transport.list_segments(&key_id).await.unwrap().is_empty());

        let recovered = Storage::open_in_memory().await.unwrap();
        recovered.enable_sync().await.unwrap();
        let report = synchronize_transport(&recovered, &key, &transport, Utc::now())
            .await
            .unwrap();
        assert_eq!(report.downloaded_segments, 0);
        assert!(report.imported_events >= 1);
        let feeds = recovered.list_feeds().await.unwrap();
        assert_eq!(feeds.len(), 1);
        assert!(!feeds[0].is_active);
    }

    #[tokio::test]
    async fn distributed_roster_propagates_irreversible_device_revocation() {
        let key = SyncGroupKey::from_bytes([0x83; 32]);
        let transport = MemoryTransport::default();
        let linux = Storage::open_in_memory().await.unwrap();
        let android = Storage::open_in_memory().await.unwrap();
        linux.enable_sync().await.unwrap();
        android.enable_sync().await.unwrap();
        let linux_id = linux.sync_identity().await.unwrap().device_id;
        let android_id = android.sync_identity().await.unwrap().device_id;
        let observed_at = Utc::now();
        linux
            .register_sync_device(&android_id, "Android", observed_at)
            .await
            .unwrap();
        android
            .register_sync_device(&linux_id, "Linux", observed_at)
            .await
            .unwrap();

        synchronize_transport(&linux, &key, &transport, observed_at)
            .await
            .unwrap();
        synchronize_transport(&android, &key, &transport, observed_at)
            .await
            .unwrap();
        assert!(
            linux
                .revoke_sync_device(&android_id, observed_at)
                .await
                .unwrap()
        );
        synchronize_transport(&linux, &key, &transport, observed_at)
            .await
            .unwrap();

        let error = synchronize_transport(&android, &key, &transport, observed_at)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("révoqué"));
        assert!(
            android
                .sync_roster_device_is_revoked(&key.key_id(), &android_id)
                .await
                .unwrap()
        );
        android
            .merge_sync_roster(
                &key.key_id(),
                &[crate::storage::SyncRosterMember {
                    device_id: android_id.clone(),
                    revoked_at: None,
                }],
                observed_at + chrono::Duration::days(1),
            )
            .await
            .unwrap();
        assert!(
            android
                .sync_roster_device_is_revoked(&key.key_id(), &android_id)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn unchanged_checkpoint_is_verified_and_repaired_remotely() {
        let storage = Storage::open_in_memory().await.unwrap();
        storage.enable_sync().await.unwrap();
        storage
            .add_feed("https://checkpoint-repair.example/feed", None)
            .await
            .unwrap();
        let key = SyncGroupKey::from_bytes([0x84; 32]);
        let transport = MemoryTransport::default();
        synchronize_transport(&storage, &key, &transport, Utc::now())
            .await
            .unwrap();
        let snapshot_path = transport
            .files
            .lock()
            .unwrap()
            .keys()
            .find(|path| path.contains("/snapshots/"))
            .unwrap()
            .clone();

        transport.files.lock().unwrap().remove(&snapshot_path);
        synchronize_transport(&storage, &key, &transport, Utc::now())
            .await
            .unwrap();
        assert!(transport.files.lock().unwrap().contains_key(&snapshot_path));

        {
            let mut files = transport.files.lock().unwrap();
            let bytes = files.get_mut(&snapshot_path).unwrap();
            let index = bytes.len() / 2;
            bytes[index] ^= 1;
        }
        synchronize_transport(&storage, &key, &transport, Utc::now())
            .await
            .unwrap();

        let restored = Storage::open_in_memory().await.unwrap();
        restored.enable_sync().await.unwrap();
        let bytes = transport
            .files
            .lock()
            .unwrap()
            .get(&snapshot_path)
            .unwrap()
            .clone();
        crate::sync_snapshots::import_snapshot(&restored, &key, &snapshot_path, &bytes, Utc::now())
            .await
            .unwrap();
        assert_eq!(restored.list_feeds().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn lagging_device_repairs_a_missing_segment_from_the_latest_snapshot() {
        let source = Storage::open_in_memory().await.unwrap();
        let lagging = Storage::open_in_memory().await.unwrap();
        source.enable_sync().await.unwrap();
        lagging.enable_sync().await.unwrap();
        let feed = source
            .add_feed("https://repair.example/feed", None)
            .await
            .unwrap();
        let source_id = source.sync_identity().await.unwrap().device_id;
        let key = SyncGroupKey::from_bytes([0xa2; 32]);
        let transport = MemoryTransport::default();

        synchronize_transport(&source, &key, &transport, Utc::now())
            .await
            .unwrap();
        synchronize_transport(&lagging, &key, &transport, Utc::now())
            .await
            .unwrap();
        assert_eq!(lagging.sync_import_cursor(&source_id).await.unwrap(), 1);

        source.set_feed_active(&feed.id, false).await.unwrap();
        synchronize_transport(&source, &key, &transport, Utc::now())
            .await
            .unwrap();
        source.set_feed_active(&feed.id, true).await.unwrap();
        synchronize_transport(&source, &key, &transport, Utc::now())
            .await
            .unwrap();
        let missing_suffix = "/00000000000000000002-00000000000000000002.json";
        transport
            .files
            .lock()
            .unwrap()
            .retain(|path, _| !path.ends_with(missing_suffix));

        let repaired = synchronize_transport(&lagging, &key, &transport, Utc::now())
            .await
            .unwrap();
        assert_eq!(repaired.imported_events, 1);
        assert_eq!(repaired.downloaded_segments, 0);
        assert_eq!(lagging.sync_import_cursor(&source_id).await.unwrap(), 3);
        assert!(lagging.list_feeds().await.unwrap()[0].is_active);
    }
}
