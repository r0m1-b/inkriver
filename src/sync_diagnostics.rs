use crate::storage::{Storage, StoredSyncReport};
use crate::sync::SYNC_PROTOCOL_VERSION;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;

const DIAGNOSTIC_FORMAT: &str = "inkriver-sync-diagnostic";
const DIAGNOSTIC_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SyncDiagnosticCounters {
    pub local_events: i64,
    pub remote_events: i64,
    pub pending_events: i64,
    pub import_streams: i64,
    pub acknowledgements: i64,
    pub published_snapshots: i64,
    pub imported_snapshots: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncDiagnostic {
    format: &'static str,
    format_version: u32,
    application_version: &'static str,
    sync_protocol_version: i64,
    generated_at: String,
    configured: bool,
    enabled: bool,
    local_journal_sequence: i64,
    known_devices: usize,
    active_devices: usize,
    revoked_devices: usize,
    local_events: i64,
    remote_events: i64,
    pending_events: i64,
    import_streams: i64,
    acknowledgements: i64,
    published_snapshots: i64,
    imported_snapshots: i64,
    last_attempt_at: Option<String>,
    last_success_at: Option<String>,
    last_error_stage: Option<String>,
    last_report: Option<SyncDiagnosticReport>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SyncDiagnosticReport {
    uploaded_segments: usize,
    reused_segments: usize,
    exported_events: usize,
    downloaded_segments: usize,
    received_events: usize,
    imported_events: usize,
    duplicate_events: usize,
    applied_events: usize,
    pending_events: usize,
}

impl From<StoredSyncReport> for SyncDiagnosticReport {
    fn from(report: StoredSyncReport) -> Self {
        Self {
            uploaded_segments: report.uploaded_segments,
            reused_segments: report.reused_segments,
            exported_events: report.exported_events,
            downloaded_segments: report.downloaded_segments,
            received_events: report.received_events,
            imported_events: report.imported_events,
            duplicate_events: report.duplicate_events,
            applied_events: report.applied_events,
            pending_events: report.pending_events,
        }
    }
}

/// Builds a read-only support report without credentials, remote locations,
/// device identities, subscription metadata or article data.
pub async fn build_sync_diagnostic(
    storage: &Storage,
    generated_at: DateTime<Utc>,
) -> Result<SyncDiagnostic> {
    let configured = storage.sync_configuration().await?.is_some();
    let identity = storage.sync_identity().await?;
    let devices = storage.list_sync_devices().await?;
    let status = storage.sync_runtime_status().await?;
    let counters = storage.sync_diagnostic_counters().await?;
    let revoked_devices = devices
        .iter()
        .filter(|device| device.revoked_at.is_some())
        .count();

    Ok(SyncDiagnostic {
        format: DIAGNOSTIC_FORMAT,
        format_version: DIAGNOSTIC_VERSION,
        application_version: env!("CARGO_PKG_VERSION"),
        sync_protocol_version: SYNC_PROTOCOL_VERSION,
        generated_at: generated_at.to_rfc3339(),
        configured,
        enabled: identity.is_enabled,
        local_journal_sequence: identity.next_sequence - 1,
        known_devices: devices.len(),
        active_devices: devices.len() - revoked_devices,
        revoked_devices,
        local_events: counters.local_events,
        remote_events: counters.remote_events,
        pending_events: counters.pending_events,
        import_streams: counters.import_streams,
        acknowledgements: counters.acknowledgements,
        published_snapshots: counters.published_snapshots,
        imported_snapshots: counters.imported_snapshots,
        last_attempt_at: status.last_attempt_at.map(|date| date.to_rfc3339()),
        last_success_at: status.last_success_at.map(|date| date.to_rfc3339()),
        last_error_stage: status.last_error.map(|error| error.stage),
        last_report: status.last_report.map(Into::into),
    })
}

pub async fn export_sync_diagnostic_json(
    storage: &Storage,
    generated_at: DateTime<Utc>,
) -> Result<String> {
    let diagnostic = build_sync_diagnostic(storage, generated_at).await?;
    serde_json::to_string_pretty(&diagnostic)
        .context("Impossible de sérialiser le diagnostic de synchronisation")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::SyncConfiguration;

    #[tokio::test]
    async fn exported_diagnostic_excludes_secrets_and_personal_metadata() {
        let storage = Storage::open_in_memory().await.unwrap();
        storage.enable_sync().await.unwrap();
        storage
            .save_sync_configuration(&SyncConfiguration {
                webdav_base_url: "https://cloud.example/private/romain/".to_string(),
                webdav_username: "personal-user".to_string(),
                key_id: "ab".repeat(32),
            })
            .await
            .unwrap();
        storage
            .register_sync_device(
                "00000000-0000-4000-8000-000000000099",
                "Romain's private phone",
                Utc::now(),
            )
            .await
            .unwrap();

        let json = export_sync_diagnostic_json(&storage, Utc::now())
            .await
            .unwrap();

        assert!(json.contains("inkriver-sync-diagnostic"));
        let key_id = "ab".repeat(32);
        for forbidden in [
            "cloud.example",
            "romain",
            "personal-user",
            &key_id,
            "00000000-0000-4000-8000-000000000099",
            "private phone",
        ] {
            assert!(!json.to_lowercase().contains(&forbidden.to_lowercase()));
        }
    }
}
