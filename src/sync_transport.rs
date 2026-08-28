use crate::storage::Storage;
use crate::sync_segments::{
    MAX_DIRECTORY_SEGMENTS, MAX_SEGMENT_BYTES, SyncDirectoryImportReport, SyncGroupKey,
    confirm_sync_segment_export, import_sync_segment_blobs, prepare_sync_export,
    verify_prepared_segment_bytes,
};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use futures_util::stream::{self, StreamExt, TryStreamExt};
use std::collections::HashMap;

const MAX_CONCURRENT_DOWNLOADS: usize = 4;
const MAX_SEGMENTS_PER_SYNC: usize = 20;

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
    let mut missing_paths = Vec::new();
    for path in paths {
        let (device_id, last_sequence) = segment_path_identity(&path, &prepared.key_id)?;
        if device_id == prepared.device_id {
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
    apply_import_report(&mut report, imported);
    Ok(report)
}

fn segment_path_identity<'a>(relative_path: &'a str, key_id: &str) -> Result<(&'a str, i64)> {
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
    Ok((components[2], last_sequence))
}

fn apply_import_report(report: &mut SyncTransportReport, imported: SyncDirectoryImportReport) {
    report.received_events = imported.received;
    report.imported_events = imported.imported;
    report.duplicate_events = imported.duplicates;
    report.applied_events = imported.applied;
    report.pending_events = imported.pending;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

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
}
