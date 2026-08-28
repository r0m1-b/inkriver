use crate::storage::{Storage, StoredSyncReport, SyncConfiguration};
use crate::sync_secrets::{SyncSecretStore, SyncSecrets};
use crate::sync_transport::{SegmentTransport, SyncTransportReport, synchronize_transport};
use crate::sync_webdav::{WebDavConfig, WebDavTransport};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};

const SYNC_ERROR_STAGE: &str = "Transport WebDAV";
const MAX_SYNC_ERROR_BYTES: usize = 4_096;

impl From<SyncTransportReport> for StoredSyncReport {
    fn from(report: SyncTransportReport) -> Self {
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

fn bounded_error_message(error: &anyhow::Error) -> String {
    let mut message = format!("{error:#}");
    if message.len() > MAX_SYNC_ERROR_BYTES {
        let mut boundary = MAX_SYNC_ERROR_BYTES;
        while !message.is_char_boundary(boundary) {
            boundary -= 1;
        }
        message.truncate(boundary);
    }
    message
}

async fn persist_sync_result(
    storage: &Storage,
    attempted_at: DateTime<Utc>,
    result: Result<SyncTransportReport>,
) -> Result<SyncTransportReport> {
    match result {
        Ok(report) => {
            storage
                .record_sync_success(attempted_at, report.into())
                .await?;
            Ok(report)
        }
        Err(error) => {
            let message = bounded_error_message(&error);
            if let Err(status_error) = storage
                .record_sync_failure(attempted_at, SYNC_ERROR_STAGE, &message)
                .await
            {
                return Err(anyhow::anyhow!(
                    "{message}; impossible de mémoriser l'échec : {status_error:#}"
                ));
            }
            Err(error)
        }
    }
}

async fn load_sync_material<S: SyncSecretStore>(
    storage: &Storage,
    secret_store: &S,
) -> Result<(SyncConfiguration, SyncSecrets)> {
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
    Ok((configuration, secrets))
}

/// Runs one complete upload, download and merge cycle using the configured
/// WebDAV endpoint and the native secret store.
pub async fn synchronize_configured<S: SyncSecretStore>(
    storage: &Storage,
    secret_store: &S,
    observed_at: DateTime<Utc>,
) -> Result<SyncTransportReport> {
    storage.record_sync_attempt(observed_at).await?;
    let result = async {
        let (configuration, secrets) = load_sync_material(storage, secret_store).await?;
        let webdav = WebDavConfig::new(
            &configuration.webdav_base_url,
            configuration.webdav_username,
            secrets.webdav_password.to_string(),
        )?;
        let transport = WebDavTransport::new(webdav)?;
        synchronize_transport(storage, &secrets.group_key, &transport, observed_at).await
    }
    .await;
    persist_sync_result(storage, observed_at, result).await
}

/// Testable transport-independent entry point that still enforces the local
/// configuration/secret consistency checks used by the WebDAV runtime.
pub async fn synchronize_configured_with_transport<S, T>(
    storage: &Storage,
    secret_store: &S,
    transport: &T,
    observed_at: DateTime<Utc>,
) -> Result<SyncTransportReport>
where
    S: SyncSecretStore,
    T: SegmentTransport,
{
    storage.record_sync_attempt(observed_at).await?;
    let result = async {
        let (_configuration, secrets) = load_sync_material(storage, secret_store).await?;
        synchronize_transport(storage, &secrets.group_key, transport, observed_at).await
    }
    .await;
    persist_sync_result(storage, observed_at, result).await
}

/// Removes the local synchronization configuration and native secrets. Local
/// content, event history and remote WebDAV files are deliberately preserved.
pub async fn remove_sync_configuration<S: SyncSecretStore>(
    storage: &Storage,
    secret_store: &S,
) -> Result<()> {
    secret_store
        .delete()
        .context("Impossible de supprimer les secrets de synchronisation")?;
    storage.remove_sync_metadata().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::SyncConfiguration;
    use crate::sync_secrets::SyncSecrets;
    use crate::sync_segments::SyncGroupKey;
    use crate::sync_transport::SegmentPublishOutcome;
    use anyhow::Result;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemorySecretStore(Mutex<Option<SyncSecrets>>);

    impl SyncSecretStore for MemorySecretStore {
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

    struct EmptyTransport;

    impl SegmentTransport for EmptyTransport {
        async fn ensure_layout(&self, _key_id: &str, _device_id: &str) -> Result<()> {
            Ok(())
        }

        async fn publish_immutable(
            &self,
            _relative_path: &str,
            _bytes: &[u8],
        ) -> Result<SegmentPublishOutcome> {
            unreachable!("an empty storage has no segment to publish")
        }

        async fn list_segments(&self, _key_id: &str) -> Result<Vec<String>> {
            Ok(Vec::new())
        }

        async fn download_segment(
            &self,
            _relative_path: &str,
            _max_bytes: usize,
        ) -> Result<Vec<u8>> {
            unreachable!("the transport exposes no segment")
        }
    }

    struct FailingTransport;

    impl SegmentTransport for FailingTransport {
        async fn ensure_layout(&self, _key_id: &str, _device_id: &str) -> Result<()> {
            anyhow::bail!("serveur indisponible")
        }

        async fn publish_immutable(
            &self,
            _relative_path: &str,
            _bytes: &[u8],
        ) -> Result<SegmentPublishOutcome> {
            unreachable!()
        }

        async fn list_segments(&self, _key_id: &str) -> Result<Vec<String>> {
            unreachable!()
        }

        async fn download_segment(
            &self,
            _relative_path: &str,
            _max_bytes: usize,
        ) -> Result<Vec<u8>> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn configured_runtime_validates_material_and_returns_transport_counters() {
        let storage = Storage::open_in_memory().await.unwrap();
        let key = SyncGroupKey::from_bytes([0x42; 32]);
        storage
            .save_sync_configuration(&SyncConfiguration {
                webdav_base_url: "https://cloud.example/dav/inkriver/".to_string(),
                webdav_username: "alice".to_string(),
                key_id: key.key_id(),
            })
            .await
            .unwrap();
        storage.enable_sync().await.unwrap();
        let secrets = MemorySecretStore::default();
        secrets
            .save(&SyncSecrets::new(key, "secret".to_string()).unwrap())
            .unwrap();

        let report =
            synchronize_configured_with_transport(&storage, &secrets, &EmptyTransport, Utc::now())
                .await
                .unwrap();
        assert_eq!(report, SyncTransportReport::default());
        let status = storage.sync_runtime_status().await.unwrap();
        assert!(status.last_attempt_at.is_some());
        assert!(status.last_success_at.is_some());
        assert_eq!(status.last_report, Some(StoredSyncReport::default()));
        assert!(status.last_error.is_none());
    }

    #[tokio::test]
    async fn configured_runtime_rejects_missing_or_mismatched_material() {
        let storage = Storage::open_in_memory().await.unwrap();
        let secrets = MemorySecretStore::default();
        let missing =
            synchronize_configured_with_transport(&storage, &secrets, &EmptyTransport, Utc::now())
                .await
                .unwrap_err();
        assert!(missing.to_string().contains("n'est pas configurée"));

        let expected_key = SyncGroupKey::from_bytes([0x24; 32]);
        storage
            .save_sync_configuration(&SyncConfiguration {
                webdav_base_url: "https://cloud.example/dav/inkriver/".to_string(),
                webdav_username: "alice".to_string(),
                key_id: expected_key.key_id(),
            })
            .await
            .unwrap();
        let missing_secrets =
            synchronize_configured_with_transport(&storage, &secrets, &EmptyTransport, Utc::now())
                .await
                .unwrap_err();
        assert!(missing_secrets.to_string().contains("secrets"));

        secrets
            .save(
                &SyncSecrets::new(SyncGroupKey::from_bytes([0x25; 32]), "secret".to_string())
                    .unwrap(),
            )
            .unwrap();
        let mismatch =
            synchronize_configured_with_transport(&storage, &secrets, &EmptyTransport, Utc::now())
                .await
                .unwrap_err();
        assert!(mismatch.to_string().contains("ne correspond pas"));
        let status = storage.sync_runtime_status().await.unwrap();
        assert!(status.last_success_at.is_none());
        assert!(
            status
                .last_error
                .unwrap()
                .message
                .contains("ne correspond pas")
        );
    }

    #[tokio::test]
    async fn removing_configuration_preserves_content_and_local_identity() {
        let storage = Storage::open_in_memory().await.unwrap();
        let key = SyncGroupKey::from_bytes([0x62; 32]);
        storage
            .save_sync_configuration(&SyncConfiguration {
                webdav_base_url: "https://cloud.example/dav/inkriver/".to_string(),
                webdav_username: "alice".to_string(),
                key_id: key.key_id(),
            })
            .await
            .unwrap();
        storage.enable_sync().await.unwrap();
        let identity = storage.sync_identity().await.unwrap();
        storage
            .register_sync_device(
                "00000000-0000-4000-8000-000000000123",
                "Téléphone",
                Utc::now(),
            )
            .await
            .unwrap();
        storage.record_sync_attempt(Utc::now()).await.unwrap();
        let secrets = MemorySecretStore::default();
        secrets
            .save(&SyncSecrets::new(key, "secret".to_string()).unwrap())
            .unwrap();

        remove_sync_configuration(&storage, &secrets).await.unwrap();

        assert!(secrets.load().unwrap().is_none());
        assert!(storage.sync_configuration().await.unwrap().is_none());
        assert!(
            storage
                .sync_runtime_status()
                .await
                .unwrap()
                .last_attempt_at
                .is_none()
        );
        let devices = storage.list_sync_devices().await.unwrap();
        assert_eq!(devices.len(), 1);
        assert!(devices[0].is_local);
        assert_eq!(devices[0].device_id, identity.device_id);
    }

    #[tokio::test]
    async fn failed_attempt_preserves_the_previous_success_and_detailed_error() {
        let storage = Storage::open_in_memory().await.unwrap();
        let key = SyncGroupKey::from_bytes([0x72; 32]);
        storage
            .save_sync_configuration(&SyncConfiguration {
                webdav_base_url: "https://cloud.example/dav/inkriver/".to_string(),
                webdav_username: "alice".to_string(),
                key_id: key.key_id(),
            })
            .await
            .unwrap();
        storage.enable_sync().await.unwrap();
        let secrets = MemorySecretStore::default();
        secrets
            .save(&SyncSecrets::new(key, "secret".to_string()).unwrap())
            .unwrap();
        let success_at = Utc::now();
        synchronize_configured_with_transport(&storage, &secrets, &EmptyTransport, success_at)
            .await
            .unwrap();

        let failure_at = success_at + chrono::Duration::minutes(5);
        let error = synchronize_configured_with_transport(
            &storage,
            &secrets,
            &FailingTransport,
            failure_at,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("préparer le transport"));
        let status = storage.sync_runtime_status().await.unwrap();
        assert_eq!(status.last_success_at, Some(success_at));
        assert_eq!(status.last_attempt_at, Some(failure_at));
        assert_eq!(status.last_report, Some(StoredSyncReport::default()));
        let stored_error = status.last_error.unwrap();
        assert_eq!(stored_error.stage, SYNC_ERROR_STAGE);
        assert!(stored_error.message.contains("serveur indisponible"));
    }
}
