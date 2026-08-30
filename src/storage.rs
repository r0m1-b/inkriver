use crate::article::{Article, ContentKind, Source};
use crate::config::{FeedConfig, FeedUrlError, Platform, detect_platform, normalize_feed_url};
use crate::feed::FeedMetadata;
use crate::sync::{
    HybridLogicalClock, SYNC_PROTOCOL_VERSION, SyncArticleRef, SyncEvent, SyncEventId,
    SyncEventPayload, SyncIdentity, SyncImportReport,
};
use crate::sync_diagnostics::SyncDiagnosticCounters;
use crate::sync_merge;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Sqlite, SqlitePool, Transaction};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::{error::Error, fmt};

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");
pub const ARTICLE_RETENTION_DAYS: i64 = 30;
pub const EXTRACTION_RETRY_DAYS: i64 = 7;
pub const MAX_EXTRACTION_ATTEMPTS_PER_REFRESH: usize = 20;
pub const LOGO_RETRY_DAYS: i64 = 7;
pub const MAX_LOGO_ATTEMPTS_PER_REFRESH: usize = 20;
pub const MAX_SYNC_EVENTS_PER_READ: usize = 1_000;
pub const MAX_SYNC_EVENTS_COMPACTED_PER_CYCLE: usize = 1_000;

fn unique_article_ids(article_ids: &[String]) -> Vec<&str> {
    let mut seen = HashSet::new();
    article_ids
        .iter()
        .map(String::as_str)
        .filter(|article_id| seen.insert(*article_id))
        .collect()
}

fn article_entry_key<'a>(article_id: &'a str, feed_id: &str) -> &'a str {
    article_id
        .strip_prefix(feed_id)
        .and_then(|suffix| suffix.strip_prefix("::"))
        .unwrap_or(article_id)
}

fn validate_sync_configuration(configuration: &SyncConfiguration) -> Result<()> {
    let url = reqwest::Url::parse(&configuration.webdav_base_url)
        .context("URL WebDAV de synchronisation invalide")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.path().ends_with('/')
        || url.as_str() != configuration.webdav_base_url
        || configuration.webdav_username.trim().is_empty()
        || configuration.webdav_username != configuration.webdav_username.trim()
        || configuration.webdav_username.len() > 512
        || configuration.key_id.len() != 64
        || !configuration
            .key_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        anyhow::bail!("Configuration de synchronisation invalide");
    }
    Ok(())
}

fn validate_sync_device(device_id: &str, display_name: &str) -> Result<()> {
    uuid::Uuid::parse_str(device_id).context("Identifiant d'appareil invalide")?;
    if display_name.trim().is_empty()
        || display_name != display_name.trim()
        || display_name.len() > 120
    {
        anyhow::bail!("Nom d'appareil invalide");
    }
    Ok(())
}

fn validate_sync_acknowledgement(acknowledgement: &SyncAcknowledgement) -> Result<()> {
    if acknowledgement.key_id.len() != 64
        || !acknowledgement
            .key_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        anyhow::bail!("Empreinte de clé d'accusé de réception invalide");
    }
    uuid::Uuid::parse_str(&acknowledgement.observer_device_id)
        .context("Identifiant d'appareil observateur invalide")?;
    uuid::Uuid::parse_str(&acknowledgement.source_device_id)
        .context("Identifiant d'appareil source invalide")?;
    if acknowledgement.contiguous_sequence < 0 {
        anyhow::bail!("Une séquence acquittée ne peut pas être négative");
    }
    Ok(())
}

fn validate_sync_key_and_device(key_id: &str, device_id: &str) -> Result<()> {
    validate_sync_key_id(key_id)?;
    uuid::Uuid::parse_str(device_id).context("Identifiant d'appareil invalide")?;
    Ok(())
}

fn validate_sync_key_id(key_id: &str) -> Result<()> {
    if key_id.len() != 64
        || !key_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        anyhow::bail!("Empreinte de clé de synchronisation invalide");
    }
    Ok(())
}

fn validate_sync_snapshot_identity(key_id: &str, device_id: &str, state_hash: &str) -> Result<()> {
    validate_sync_key_and_device(key_id, device_id)?;
    if state_hash.len() != 64
        || !state_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        anyhow::bail!("Empreinte d'instantané invalide");
    }
    Ok(())
}

/// Owns the SQLite connection pool used by the InkRiver core.
pub struct Storage {
    pool: SqlitePool,
}

/// Non-sensitive synchronization settings persisted alongside local data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncConfiguration {
    pub webdav_base_url: String,
    pub webdav_username: String,
    pub key_id: String,
}

/// User-facing metadata for one known synchronization device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncDevice {
    pub device_id: String,
    pub display_name: String,
    pub is_local: bool,
    pub revoked_at: Option<DateTime<Utc>>,
}

/// One member of the monotonic, group-scoped synchronization roster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncRosterMember {
    pub device_id: String,
    pub revoked_at: Option<DateTime<Utc>>,
}

/// One device's durable claim that it has consumed a contiguous prefix of
/// another device's immutable synchronization journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncAcknowledgement {
    pub key_id: String,
    pub observer_device_id: String,
    pub source_device_id: String,
    pub contiguous_sequence: i64,
    pub observed_at: DateTime<Utc>,
}

/// Conservative compaction boundary calculated for an explicit, authoritative
/// set of devices that must all be able to recover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncCompactionFrontier {
    pub source_device_id: String,
    pub safe_through_sequence: i64,
    pub required_observer_count: usize,
    pub blocking_observer_device_ids: Vec<String>,
}

/// Non-sensitive counters retained from the last successful synchronization.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StoredSyncReport {
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

/// Persisted user-facing state of the manual synchronization runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSyncRuntimeStatus {
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_error: Option<StoredSyncRuntimeError>,
    pub last_report: Option<StoredSyncReport>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSyncRuntimeError {
    pub stage: String,
    pub message: String,
    pub occurred_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct SyncRuntimeStatusRow {
    last_attempt_at: Option<String>,
    last_success_at: Option<String>,
    last_error_stage: Option<String>,
    last_error_message: Option<String>,
    last_error_at: Option<String>,
    uploaded_segments: i64,
    reused_segments: i64,
    exported_events: i64,
    downloaded_segments: i64,
    received_events: i64,
    imported_events: i64,
    duplicate_events: i64,
    applied_events: i64,
    pending_events: i64,
    compacted_events: i64,
    deleted_segments: i64,
    deferred_segment_deletions: i64,
}

/// Represents one subscription as persisted by the application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFeed {
    pub id: String,
    pub platform: Platform,
    pub url: String,
    pub is_active: bool,
    pub title: Option<String>,
    pub description: Option<String>,
    pub author: Option<String>,
    pub last_published_at: Option<DateTime<Utc>>,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_error: Option<StoredFeedError>,
    pub logo_png: Option<Vec<u8>>,
}

/// Describes the most recent failed refresh retained for a subscription.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFeedError {
    pub stage: String,
    pub message: String,
    pub occurred_at: DateTime<Utc>,
}

/// Contains one failed feed result ready to be persisted after collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedRefreshFailure {
    pub feed_id: String,
    pub stage: String,
    pub message: String,
}

/// Summarizes a permanent subscription deletion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteFeedResult {
    pub feed_id: String,
    pub deleted_articles: usize,
}

/// Errors produced while changing the installed application's subscriptions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscriptionError {
    InvalidUrl(FeedUrlError),
    DuplicateActiveUrl(String),
    NotFound(String),
    Inactive(String),
    Database(String),
}

impl fmt::Display for SubscriptionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl(error) => error.fmt(formatter),
            Self::DuplicateActiveUrl(url) => write!(formatter, "Feed URL is already active: {url}"),
            Self::NotFound(id) => write!(formatter, "Feed not found: {id}"),
            Self::Inactive(id) => write!(formatter, "Feed is inactive: {id}"),
            Self::Database(message) => write!(formatter, "SQLite subscription error: {message}"),
        }
    }
}

impl Error for SubscriptionError {}

/// Combines remote article data with InkRiver-specific local state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredArticle {
    pub article: Article,
    pub is_read: bool,
    pub is_favorite: bool,
}

/// Contains the lightweight fields required to render an article list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArticleSummary {
    pub id: String,
    pub feed_id: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    pub url: Option<String>,
    pub source: Source,
    pub is_read: bool,
    pub is_favorite: bool,
}

/// Counts rows inserted and rows refreshed by one article batch.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct UpsertStats {
    pub inserted: usize,
    pub updated: usize,
}

/// One visible article whose original page may provide a complete body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionCandidate {
    pub article_id: String,
    pub url: String,
}

/// Candidates due now and the number deliberately deferred to a later refresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionSelection {
    pub candidates: Vec<ExtractionCandidate>,
    pub skipped: usize,
}

/// One successfully refreshed feed whose website logo is due for discovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedLogoCandidate {
    pub feed_id: String,
    pub site_url: String,
    pub declared_icon_url: Option<String>,
}

fn same_site_domain(left: &str, right: &str) -> bool {
    let parsed = reqwest::Url::parse(left)
        .ok()
        .zip(reqwest::Url::parse(right).ok());
    parsed
        .and_then(|(left, right)| {
            Some(
                left.host_str()?.eq_ignore_ascii_case(right.host_str()?)
                    && left.port_or_known_default() == right.port_or_known_default(),
            )
        })
        .unwrap_or(left == right)
}

type StoredFeedRow = (
    String,
    String,
    String,
    bool,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<DateTime<Utc>>,
    Option<String>,
    Option<String>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    Option<Vec<u8>>,
);

fn stored_feed_from_row(row: StoredFeedRow) -> Result<StoredFeed> {
    let (
        id,
        platform,
        url,
        is_active,
        title,
        description,
        author,
        last_success_at,
        last_error_stage,
        last_error_message,
        last_error_at,
        last_published_at,
        logo_png,
    ) = row;
    let platform = Platform::try_from(platform.as_str()).map_err(anyhow::Error::msg)?;
    let last_error = match (last_error_stage, last_error_message, last_error_at) {
        (Some(stage), Some(message), Some(occurred_at)) => Some(StoredFeedError {
            stage,
            message,
            occurred_at,
        }),
        _ => None,
    };

    Ok(StoredFeed {
        id,
        platform,
        url,
        is_active,
        title,
        description,
        author,
        last_published_at,
        last_success_at,
        last_error,
        logo_png,
    })
}

type StoredArticleRow = (
    String,
    String,
    Option<String>,
    Option<String>,
    Option<chrono::DateTime<chrono::Utc>>,
    Option<String>,
    Option<String>,
    String,
    String,
    bool,
    bool,
);

type SyncEventRow = (String, i64, i64, i64, i64, String, String);
type SyncArticleRefRow = (
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<DateTime<Utc>>,
);

fn sync_article_ref_from_row(row: SyncArticleRefRow) -> SyncArticleRef {
    let (subscription_id, entry_key, title, url, author, published_at) = row;
    SyncArticleRef {
        subscription_id,
        entry_key,
        title,
        url,
        author,
        published_at: published_at.map(|date| date.to_rfc3339()),
    }
}

async fn canonical_sync_subscription_id_in(
    transaction: &mut Transaction<'_, Sqlite>,
    subscription_id: &str,
) -> Result<String> {
    Ok(
        sqlx::query_scalar("SELECT canonical_id FROM sync_subscription_aliases WHERE alias_id = ?")
            .bind(subscription_id)
            .fetch_optional(&mut **transaction)
            .await
            .context("Impossible de résoudre l'identité canonique de l'abonnement")?
            .unwrap_or_else(|| subscription_id.to_string()),
    )
}

async fn store_local_sync_version_in(
    transaction: &mut Transaction<'_, Sqlite>,
    entity_kind: &str,
    entity_key: &str,
    field_name: &str,
    event: &SyncEvent,
) -> Result<()> {
    sqlx::query(
        r#"
            INSERT INTO sync_entity_versions (
                entity_kind, entity_key, field_name,
                event_device_id, event_sequence
            ) VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(entity_kind, entity_key, field_name) DO UPDATE SET
                event_device_id = excluded.event_device_id,
                event_sequence = excluded.event_sequence
        "#,
    )
    .bind(entity_kind)
    .bind(entity_key)
    .bind(field_name)
    .bind(&event.device_id)
    .bind(event.sequence)
    .execute(&mut **transaction)
    .await
    .context("Impossible d'enregistrer la version locale synchronisée")?;
    Ok(())
}

async fn store_local_sync_tombstone_in(
    transaction: &mut Transaction<'_, Sqlite>,
    entity_kind: &str,
    entity_key: &str,
    event: &SyncEvent,
) -> Result<()> {
    sqlx::query(
        r#"
            INSERT INTO sync_tombstones (
                entity_kind, entity_key, event_device_id, event_sequence
            ) VALUES (?, ?, ?, ?)
            ON CONFLICT(entity_kind, entity_key) DO UPDATE SET
                event_device_id = excluded.event_device_id,
                event_sequence = excluded.event_sequence
        "#,
    )
    .bind(entity_kind)
    .bind(entity_key)
    .bind(&event.device_id)
    .bind(event.sequence)
    .execute(&mut **transaction)
    .await
    .context("Impossible d'enregistrer la pierre tombale locale")?;
    Ok(())
}

fn sync_event_from_row(row: SyncEventRow) -> Result<SyncEvent> {
    let (
        device_id,
        sequence,
        physical_milliseconds,
        logical_counter,
        protocol_version,
        kind,
        payload_json,
    ) = row;
    let payload: SyncEventPayload = serde_json::from_str(&payload_json)
        .with_context(|| format!("Invalid JSON in synchronization event {device_id}:{sequence}"))?;
    if payload.kind() != kind {
        anyhow::bail!(
            "Synchronization event kind mismatch for {device_id}:{sequence}: column={kind}, payload={}",
            payload.kind()
        );
    }

    Ok(SyncEvent {
        device_id,
        sequence,
        clock: HybridLogicalClock {
            physical_milliseconds,
            logical_counter,
        },
        protocol_version,
        kind,
        payload,
    })
}

fn stored_article_from_row(row: StoredArticleRow) -> Result<StoredArticle> {
    let (
        id,
        feed_id,
        title,
        author,
        published_at,
        url,
        content,
        content_kind,
        source,
        is_read,
        is_favorite,
    ) = row;
    let source = Source::try_from(source.as_str()).map_err(anyhow::Error::msg)?;
    let content_kind = ContentKind::try_from(content_kind.as_str()).map_err(anyhow::Error::msg)?;

    Ok(StoredArticle {
        article: Article {
            id,
            feed_id,
            title,
            author,
            published_at,
            url,
            content,
            content_kind,
            source,
        },
        is_read,
        is_favorite,
    })
}

impl Storage {
    /// Opens or creates a SQLite database and applies all embedded migrations.
    ///
    /// # Errors
    ///
    /// Returns an error when the database cannot be opened or migrated.
    pub async fn open(path: &Path) -> Result<Self> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true);

        Self::connect(options, 5)
            .await
            .with_context(|| format!("Impossible d'ouvrir la base SQLite {}", path.display()))
    }

    async fn connect(options: SqliteConnectOptions, max_connections: u32) -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(max_connections)
            .connect_with(options)
            .await
            .context("Impossible de créer le pool SQLite")?;

        MIGRATOR
            .run(&pool)
            .await
            .context("Impossible d'appliquer les migrations SQLite")?;

        let generated_device_id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            r#"
                INSERT OR IGNORE INTO sync_local_state (
                    singleton, device_id, next_sequence,
                    hlc_physical_ms, hlc_counter
                ) VALUES (1, ?, 1, 0, 0)
            "#,
        )
        .bind(generated_device_id)
        .execute(&pool)
        .await
        .context("Impossible d'initialiser l'identité de synchronisation")?;

        let (device_id,): (String,) =
            sqlx::query_as("SELECT device_id FROM sync_local_state WHERE singleton = 1")
                .fetch_one(&pool)
                .await
                .context("Impossible de relire l'identité de synchronisation")?;
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"
                INSERT OR IGNORE INTO sync_devices (
                    device_id, display_name, is_local, created_at, updated_at
                ) VALUES (?, 'Cet appareil', 1, ?, ?)
            "#,
        )
        .bind(device_id)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .context("Impossible d'initialiser le nom de l'appareil")?;

        Ok(Self { pool })
    }

    /// Saves the non-secret part of the current synchronization configuration.
    pub async fn save_sync_configuration(&self, configuration: &SyncConfiguration) -> Result<()> {
        validate_sync_configuration(configuration)?;
        sqlx::query(
            r#"
                INSERT INTO sync_configuration (
                    singleton, webdav_base_url, webdav_username, key_id
                ) VALUES (1, ?, ?, ?)
                ON CONFLICT(singleton) DO UPDATE SET
                    webdav_base_url = excluded.webdav_base_url,
                    webdav_username = excluded.webdav_username,
                    key_id = excluded.key_id
            "#,
        )
        .bind(&configuration.webdav_base_url)
        .bind(&configuration.webdav_username)
        .bind(&configuration.key_id)
        .execute(&self.pool)
        .await
        .context("Impossible d'enregistrer la configuration de synchronisation")?;
        Ok(())
    }

    /// Loads the non-secret synchronization configuration, if one exists.
    pub async fn sync_configuration(&self) -> Result<Option<SyncConfiguration>> {
        let row: Option<(String, String, String)> = sqlx::query_as(
            "SELECT webdav_base_url, webdav_username, key_id FROM sync_configuration WHERE singleton = 1",
        )
        .fetch_optional(&self.pool)
        .await
        .context("Impossible de charger la configuration de synchronisation")?;
        Ok(row.map(
            |(webdav_base_url, webdav_username, key_id)| SyncConfiguration {
                webdav_base_url,
                webdav_username,
                key_id,
            },
        ))
    }

    /// Removes only non-secret synchronization settings from SQLite.
    pub async fn clear_sync_configuration(&self) -> Result<()> {
        sqlx::query("DELETE FROM sync_configuration WHERE singleton = 1")
            .execute(&self.pool)
            .await
            .context("Impossible de supprimer la configuration de synchronisation")?;
        Ok(())
    }

    /// Returns the persisted outcome of the most recent manual sync attempts.
    pub async fn sync_runtime_status(&self) -> Result<StoredSyncRuntimeStatus> {
        let row: Option<SyncRuntimeStatusRow> = sqlx::query_as(
            r#"
                SELECT last_attempt_at, last_success_at,
                       last_error_stage, last_error_message, last_error_at,
                       uploaded_segments, reused_segments, exported_events,
                       downloaded_segments, received_events, imported_events,
                       duplicate_events, applied_events, pending_events,
                       compacted_events, deleted_segments,
                       deferred_segment_deletions
                FROM sync_runtime_status WHERE singleton = 1
            "#,
        )
        .fetch_optional(&self.pool)
        .await
        .context("Impossible de charger l'état de synchronisation")?;
        let Some(row) = row else {
            return Ok(StoredSyncRuntimeStatus {
                last_attempt_at: None,
                last_success_at: None,
                last_error: None,
                last_report: None,
            });
        };
        let parse_date = |value: String| -> Result<DateTime<Utc>> {
            Ok(DateTime::parse_from_rfc3339(&value)
                .context("Date d'état de synchronisation invalide")?
                .with_timezone(&Utc))
        };
        let last_attempt_at = row.last_attempt_at.map(&parse_date).transpose()?;
        let last_success_at = row.last_success_at.map(&parse_date).transpose()?;
        let last_error = match (
            row.last_error_stage,
            row.last_error_message,
            row.last_error_at,
        ) {
            (Some(stage), Some(message), Some(occurred_at)) => Some(StoredSyncRuntimeError {
                stage,
                message,
                occurred_at: parse_date(occurred_at)?,
            }),
            (None, None, None) => None,
            _ => anyhow::bail!("État d'erreur de synchronisation incohérent"),
        };
        let last_report = last_success_at.map(|_| StoredSyncReport {
            uploaded_segments: row.uploaded_segments as usize,
            reused_segments: row.reused_segments as usize,
            exported_events: row.exported_events as usize,
            downloaded_segments: row.downloaded_segments as usize,
            received_events: row.received_events as usize,
            imported_events: row.imported_events as usize,
            duplicate_events: row.duplicate_events as usize,
            applied_events: row.applied_events as usize,
            pending_events: row.pending_events as usize,
            compacted_events: row.compacted_events as usize,
            deleted_segments: row.deleted_segments as usize,
            deferred_segment_deletions: row.deferred_segment_deletions as usize,
        });
        Ok(StoredSyncRuntimeStatus {
            last_attempt_at,
            last_success_at,
            last_error,
            last_report,
        })
    }

    pub(crate) async fn sync_diagnostic_counters(&self) -> Result<SyncDiagnosticCounters> {
        let local_device_id: String =
            sqlx::query_scalar("SELECT device_id FROM sync_local_state WHERE singleton = 1")
                .fetch_one(&self.pool)
                .await
                .context("Impossible de lire l'identité du diagnostic")?;
        let row: (i64, i64, i64, i64, i64, i64, i64) = sqlx::query_as(
            r#"
                SELECT
                    (SELECT COUNT(*) FROM sync_events WHERE device_id = ?),
                    (SELECT COUNT(*) FROM sync_events WHERE device_id <> ?),
                    (SELECT COUNT(*) FROM sync_pending_events),
                    (SELECT COUNT(*) FROM sync_import_cursors),
                    (SELECT COUNT(*) FROM sync_acknowledgements),
                    (SELECT COUNT(*) FROM sync_snapshot_publications),
                    (SELECT COUNT(*) FROM sync_snapshot_imports)
            "#,
        )
        .bind(&local_device_id)
        .bind(&local_device_id)
        .fetch_one(&self.pool)
        .await
        .context("Impossible de calculer le diagnostic de synchronisation")?;
        Ok(SyncDiagnosticCounters {
            local_events: row.0,
            remote_events: row.1,
            pending_events: row.2,
            import_streams: row.3,
            acknowledgements: row.4,
            published_snapshots: row.5,
            imported_snapshots: row.6,
        })
    }

    pub async fn record_sync_attempt(&self, attempted_at: DateTime<Utc>) -> Result<()> {
        sqlx::query(
            r#"
                INSERT INTO sync_runtime_status (singleton, last_attempt_at)
                VALUES (1, ?)
                ON CONFLICT(singleton) DO UPDATE SET
                    last_attempt_at = excluded.last_attempt_at
            "#,
        )
        .bind(attempted_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .context("Impossible d'enregistrer la tentative de synchronisation")?;
        Ok(())
    }

    pub async fn record_sync_success(
        &self,
        succeeded_at: DateTime<Utc>,
        report: StoredSyncReport,
    ) -> Result<()> {
        sqlx::query(
            r#"
                INSERT INTO sync_runtime_status (
                    singleton, last_attempt_at, last_success_at,
                    uploaded_segments, reused_segments, exported_events,
                    downloaded_segments, received_events, imported_events,
                    duplicate_events, applied_events, pending_events,
                    compacted_events, deleted_segments,
                    deferred_segment_deletions
                ) VALUES (1, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(singleton) DO UPDATE SET
                    last_attempt_at = excluded.last_attempt_at,
                    last_success_at = excluded.last_success_at,
                    last_error_stage = NULL,
                    last_error_message = NULL,
                    last_error_at = NULL,
                    uploaded_segments = excluded.uploaded_segments,
                    reused_segments = excluded.reused_segments,
                    exported_events = excluded.exported_events,
                    downloaded_segments = excluded.downloaded_segments,
                    received_events = excluded.received_events,
                    imported_events = excluded.imported_events,
                    duplicate_events = excluded.duplicate_events,
                    applied_events = excluded.applied_events,
                    pending_events = excluded.pending_events,
                    compacted_events = excluded.compacted_events,
                    deleted_segments = excluded.deleted_segments,
                    deferred_segment_deletions = excluded.deferred_segment_deletions
            "#,
        )
        .bind(succeeded_at.to_rfc3339())
        .bind(succeeded_at.to_rfc3339())
        .bind(report.uploaded_segments as i64)
        .bind(report.reused_segments as i64)
        .bind(report.exported_events as i64)
        .bind(report.downloaded_segments as i64)
        .bind(report.received_events as i64)
        .bind(report.imported_events as i64)
        .bind(report.duplicate_events as i64)
        .bind(report.applied_events as i64)
        .bind(report.pending_events as i64)
        .bind(report.compacted_events as i64)
        .bind(report.deleted_segments as i64)
        .bind(report.deferred_segment_deletions as i64)
        .execute(&self.pool)
        .await
        .context("Impossible d'enregistrer le succès de synchronisation")?;
        Ok(())
    }

    pub async fn record_sync_failure(
        &self,
        failed_at: DateTime<Utc>,
        stage: &str,
        message: &str,
    ) -> Result<()> {
        if stage.trim().is_empty()
            || stage != stage.trim()
            || stage.len() > 120
            || message.trim().is_empty()
            || message.len() > 4_096
        {
            anyhow::bail!("Erreur de synchronisation invalide");
        }
        sqlx::query(
            r#"
                INSERT INTO sync_runtime_status (
                    singleton, last_attempt_at, last_error_stage,
                    last_error_message, last_error_at
                ) VALUES (1, ?, ?, ?, ?)
                ON CONFLICT(singleton) DO UPDATE SET
                    last_attempt_at = excluded.last_attempt_at,
                    last_error_stage = excluded.last_error_stage,
                    last_error_message = excluded.last_error_message,
                    last_error_at = excluded.last_error_at
            "#,
        )
        .bind(failed_at.to_rfc3339())
        .bind(stage)
        .bind(message)
        .bind(failed_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .context("Impossible d'enregistrer l'échec de synchronisation")?;
        Ok(())
    }

    /// Clears local pairing/runtime metadata while preserving subscriptions,
    /// articles, the local device identity, journal and remote WebDAV data.
    pub async fn remove_sync_metadata(&self) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("DELETE FROM sync_configuration WHERE singleton = 1")
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM sync_runtime_status WHERE singleton = 1")
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM sync_export_cursors")
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM sync_import_cursors")
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM sync_pending_events")
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM sync_acknowledgements")
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM sync_snapshot_publications")
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM sync_snapshot_imports")
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM sync_roster_members")
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM sync_devices WHERE is_local = 0")
            .execute(&mut *transaction)
            .await?;
        transaction
            .commit()
            .await
            .context("Impossible de supprimer les métadonnées de synchronisation")?;
        Ok(())
    }

    /// Lists local and paired devices, including logically revoked entries.
    pub async fn list_sync_devices(&self) -> Result<Vec<SyncDevice>> {
        let rows: Vec<(String, String, bool, Option<String>)> = sqlx::query_as(
            r#"
                SELECT device_id, display_name, is_local, revoked_at
                FROM sync_devices
                ORDER BY is_local DESC, display_name COLLATE NOCASE, device_id
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .context("Impossible de charger les appareils synchronisés")?;
        rows.into_iter()
            .map(|(device_id, display_name, is_local, revoked_at)| {
                Ok(SyncDevice {
                    device_id,
                    display_name,
                    is_local,
                    revoked_at: revoked_at
                        .map(|value| {
                            DateTime::parse_from_rfc3339(&value)
                                .map(|date| date.with_timezone(&Utc))
                                .context("Date de révocation d'appareil invalide")
                        })
                        .transpose()?,
                })
            })
            .collect()
    }

    /// Registers a device learned through a validated pairing invitation.
    pub async fn register_sync_device(
        &self,
        device_id: &str,
        display_name: &str,
        observed_at: DateTime<Utc>,
    ) -> Result<()> {
        validate_sync_device(device_id, display_name)?;
        let timestamp = observed_at.to_rfc3339();
        sqlx::query(
            r#"
                INSERT INTO sync_devices (
                    device_id, display_name, is_local, created_at, updated_at
                ) VALUES (?, ?, 0, ?, ?)
                ON CONFLICT(device_id) DO UPDATE SET
                    display_name = excluded.display_name,
                    updated_at = excluded.updated_at
                WHERE sync_devices.is_local = 0
                  AND sync_devices.revoked_at IS NULL
            "#,
        )
        .bind(device_id)
        .bind(display_name)
        .bind(&timestamp)
        .bind(&timestamp)
        .execute(&self.pool)
        .await
        .context("Impossible d'enregistrer l'appareil appairé")?;
        Ok(())
    }

    /// Renames a known device without changing its immutable identifier.
    pub async fn rename_sync_device(
        &self,
        device_id: &str,
        display_name: &str,
        observed_at: DateTime<Utc>,
    ) -> Result<bool> {
        validate_sync_device(device_id, display_name)?;
        let result = sqlx::query(
            "UPDATE sync_devices SET display_name = ?, updated_at = ? WHERE device_id = ?",
        )
        .bind(display_name)
        .bind(observed_at.to_rfc3339())
        .bind(device_id)
        .execute(&self.pool)
        .await
        .context("Impossible de renommer l'appareil")?;
        Ok(result.rows_affected() == 1)
    }

    /// Logically revokes a remote device while retaining its historical events.
    pub async fn revoke_sync_device(
        &self,
        device_id: &str,
        observed_at: DateTime<Utc>,
    ) -> Result<bool> {
        uuid::Uuid::parse_str(device_id).context("Identifiant d'appareil invalide")?;
        let timestamp = observed_at.to_rfc3339();
        let result = sqlx::query(
            r#"
                UPDATE sync_devices
                SET revoked_at = ?, updated_at = ?
                WHERE device_id = ? AND is_local = 0 AND revoked_at IS NULL
            "#,
        )
        .bind(&timestamp)
        .bind(&timestamp)
        .bind(device_id)
        .execute(&self.pool)
        .await
        .context("Impossible de révoquer l'appareil")?;
        Ok(result.rows_affected() == 1)
    }

    pub(crate) async fn sync_device_is_revoked(&self, device_id: &str) -> Result<bool> {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sync_devices WHERE device_id = ? AND revoked_at IS NOT NULL)",
        )
        .bind(device_id)
        .fetch_one(&self.pool)
        .await
        .context("Impossible de vérifier la révocation de l'appareil")
    }

    /// Seeds the group roster from locally known pairing metadata. Membership
    /// and revocation are both monotonic: an old observation can never remove a
    /// member or reactivate a revoked device.
    pub(crate) async fn seed_sync_roster(
        &self,
        key_id: &str,
        observed_at: DateTime<Utc>,
    ) -> Result<()> {
        validate_sync_key_id(key_id)?;
        let devices = self.list_sync_devices().await?;
        self.merge_sync_roster(
            key_id,
            &devices
                .into_iter()
                .map(|device| SyncRosterMember {
                    device_id: device.device_id,
                    revoked_at: device.revoked_at,
                })
                .collect::<Vec<_>>(),
            observed_at,
        )
        .await
    }

    pub(crate) async fn merge_sync_roster(
        &self,
        key_id: &str,
        members: &[SyncRosterMember],
        observed_at: DateTime<Utc>,
    ) -> Result<()> {
        validate_sync_key_id(key_id)?;
        if members.is_empty() || members.len() > 256 {
            anyhow::bail!("Liste d'appareils de synchronisation invalide");
        }
        let mut unique = HashSet::new();
        for member in members {
            uuid::Uuid::parse_str(&member.device_id)
                .context("Identifiant d'appareil du registre invalide")?;
            if !unique.insert(member.device_id.as_str()) {
                anyhow::bail!("Appareil dupliqué dans le registre de synchronisation");
            }
        }
        let timestamp = observed_at.to_rfc3339();
        let mut transaction = self.pool.begin().await?;
        for member in members {
            sqlx::query(
                r#"
                    INSERT INTO sync_roster_members (
                        key_id, device_id, revoked_at,
                        first_observed_at, last_observed_at
                    ) VALUES (?, ?, ?, ?, ?)
                    ON CONFLICT(key_id, device_id) DO UPDATE SET
                        revoked_at = COALESCE(
                            sync_roster_members.revoked_at,
                            excluded.revoked_at
                        ),
                        last_observed_at = MAX(
                            sync_roster_members.last_observed_at,
                            excluded.last_observed_at
                        )
                "#,
            )
            .bind(key_id)
            .bind(&member.device_id)
            .bind(member.revoked_at.map(|date| date.to_rfc3339()))
            .bind(&timestamp)
            .bind(&timestamp)
            .execute(&mut *transaction)
            .await?;
            if let Some(revoked_at) = member.revoked_at {
                sqlx::query(
                    r#"
                        UPDATE sync_devices
                        SET revoked_at = COALESCE(revoked_at, ?),
                            updated_at = MAX(updated_at, ?)
                        WHERE device_id = ?
                    "#,
                )
                .bind(revoked_at.to_rfc3339())
                .bind(&timestamp)
                .bind(&member.device_id)
                .execute(&mut *transaction)
                .await?;
            }
        }
        transaction
            .commit()
            .await
            .context("Impossible de fusionner le registre des appareils")?;
        Ok(())
    }

    pub(crate) async fn sync_roster_members(&self, key_id: &str) -> Result<Vec<SyncRosterMember>> {
        validate_sync_key_id(key_id)?;
        let rows: Vec<(String, Option<String>)> = sqlx::query_as(
            r#"
                SELECT device_id, revoked_at
                FROM sync_roster_members
                WHERE key_id = ?
                ORDER BY device_id
            "#,
        )
        .bind(key_id)
        .fetch_all(&self.pool)
        .await
        .context("Impossible de charger le registre des appareils")?;
        rows.into_iter()
            .map(|(device_id, revoked_at)| {
                Ok(SyncRosterMember {
                    device_id,
                    revoked_at: revoked_at
                        .map(|value| {
                            DateTime::parse_from_rfc3339(&value)
                                .map(|date| date.with_timezone(&Utc))
                                .context("Date de révocation du registre invalide")
                        })
                        .transpose()?,
                })
            })
            .collect()
    }

    pub(crate) async fn active_sync_roster_device_ids(&self, key_id: &str) -> Result<Vec<String>> {
        validate_sync_key_id(key_id)?;
        sqlx::query_scalar(
            r#"
                SELECT device_id FROM sync_roster_members
                WHERE key_id = ? AND revoked_at IS NULL
                ORDER BY device_id
            "#,
        )
        .bind(key_id)
        .fetch_all(&self.pool)
        .await
        .context("Impossible de charger les appareils actifs du registre")
    }

    pub(crate) async fn sync_roster_device_is_revoked(
        &self,
        key_id: &str,
        device_id: &str,
    ) -> Result<bool> {
        validate_sync_key_and_device(key_id, device_id)?;
        sqlx::query_scalar(
            r#"
                SELECT EXISTS(
                    SELECT 1 FROM sync_roster_members
                    WHERE key_id = ? AND device_id = ? AND revoked_at IS NOT NULL
                )
            "#,
        )
        .bind(key_id)
        .bind(device_id)
        .fetch_one(&self.pool)
        .await
        .context("Impossible de vérifier la révocation distribuée")
    }

    /// Returns the stable identity and journal allocation state of this installation.
    ///
    /// # Errors
    ///
    /// Returns an error when the singleton synchronization state is missing or
    /// cannot be read.
    pub async fn sync_identity(&self) -> Result<SyncIdentity> {
        let (device_id, next_sequence, physical_milliseconds, logical_counter, is_enabled): (
            String,
            i64,
            i64,
            i64,
            bool,
        ) = sqlx::query_as(
            r#"
                SELECT device_id, next_sequence, hlc_physical_ms, hlc_counter,
                       is_enabled
                FROM sync_local_state
                WHERE singleton = 1
            "#,
        )
        .fetch_one(&self.pool)
        .await
        .context("Impossible de charger l'identité de synchronisation")?;

        Ok(SyncIdentity {
            device_id,
            next_sequence,
            clock: HybridLogicalClock {
                physical_milliseconds,
                logical_counter,
            },
            is_enabled,
        })
    }

    async fn sync_is_enabled_in(transaction: &mut Transaction<'_, Sqlite>) -> Result<bool> {
        sqlx::query_scalar("SELECT is_enabled FROM sync_local_state WHERE singleton = 1")
            .fetch_one(&mut **transaction)
            .await
            .context("Impossible de lire l'activation de la synchronisation")
    }

    async fn append_local_sync_event_in(
        transaction: &mut Transaction<'_, Sqlite>,
        payload: &SyncEventPayload,
        observed_at: DateTime<Utc>,
    ) -> Result<SyncEvent> {
        let kind = payload.kind();
        let payload_json = serde_json::to_string(payload)
            .context("Impossible de sérialiser l'événement de synchronisation")?;
        let wall_milliseconds = observed_at.timestamp_millis().max(0);

        let (device_id, sequence, physical_milliseconds, logical_counter): (String, i64, i64, i64) =
            sqlx::query_as(
                r#"
                    UPDATE sync_local_state
                    SET next_sequence = next_sequence + 1,
                        hlc_counter = CASE
                            WHEN ? > hlc_physical_ms THEN 0
                            ELSE hlc_counter + 1
                        END,
                        hlc_physical_ms = MAX(hlc_physical_ms, ?)
                    WHERE singleton = 1
                    RETURNING device_id, next_sequence - 1,
                              hlc_physical_ms, hlc_counter
                "#,
            )
            .bind(wall_milliseconds)
            .bind(wall_milliseconds)
            .fetch_one(&mut **transaction)
            .await
            .context("Impossible d'allouer la version de l'événement de synchronisation")?;

        sqlx::query(
            r#"
                INSERT INTO sync_events (
                    device_id, sequence, hlc_physical_ms, hlc_counter,
                    protocol_version, event_kind, payload_json
                ) VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&device_id)
        .bind(sequence)
        .bind(physical_milliseconds)
        .bind(logical_counter)
        .bind(SYNC_PROTOCOL_VERSION)
        .bind(kind)
        .bind(&payload_json)
        .execute(&mut **transaction)
        .await
        .context("Impossible d'ajouter l'événement au journal de synchronisation")?;

        let event = SyncEvent {
            device_id,
            sequence,
            clock: HybridLogicalClock {
                physical_milliseconds,
                logical_counter,
            },
            protocol_version: SYNC_PROTOCOL_VERSION,
            kind: kind.to_string(),
            payload: payload.clone(),
        };
        Self::record_local_sync_projection_metadata(transaction, &event).await?;
        Ok(event)
    }

    async fn record_local_sync_projection_metadata(
        transaction: &mut Transaction<'_, Sqlite>,
        event: &SyncEvent,
    ) -> Result<()> {
        match &event.payload {
            SyncEventPayload::SubscriptionCreated {
                subscription_id, ..
            } => {
                let canonical =
                    canonical_sync_subscription_id_in(transaction, subscription_id).await?;
                store_local_sync_version_in(
                    transaction,
                    "subscription",
                    &canonical,
                    "active",
                    event,
                )
                .await?;
                store_local_sync_version_in(
                    transaction,
                    "subscription",
                    &canonical,
                    "platform",
                    event,
                )
                .await?;
            }
            SyncEventPayload::SubscriptionActiveSet {
                subscription_id, ..
            } => {
                let canonical =
                    canonical_sync_subscription_id_in(transaction, subscription_id).await?;
                store_local_sync_version_in(
                    transaction,
                    "subscription",
                    &canonical,
                    "active",
                    event,
                )
                .await?;
            }
            SyncEventPayload::SubscriptionPlatformSet {
                subscription_id, ..
            } => {
                let canonical =
                    canonical_sync_subscription_id_in(transaction, subscription_id).await?;
                store_local_sync_version_in(
                    transaction,
                    "subscription",
                    &canonical,
                    "platform",
                    event,
                )
                .await?;
            }
            SyncEventPayload::SubscriptionDeleted { subscription_id } => {
                let canonical =
                    canonical_sync_subscription_id_in(transaction, subscription_id).await?;
                store_local_sync_tombstone_in(transaction, "subscription", &canonical, event)
                    .await?;
            }
            SyncEventPayload::ArticleReadSet { article, .. } => {
                let canonical =
                    canonical_sync_subscription_id_in(transaction, &article.subscription_id)
                        .await?;
                let key = serde_json::to_string(&(&canonical, &article.entry_key))
                    .context("Impossible de sérialiser l'identité logique de l'article")?;
                store_local_sync_version_in(transaction, "article", &key, "read", event).await?;
            }
            SyncEventPayload::ArticleFavoriteSet { article, .. } => {
                let canonical =
                    canonical_sync_subscription_id_in(transaction, &article.subscription_id)
                        .await?;
                let key = serde_json::to_string(&(&canonical, &article.entry_key))
                    .context("Impossible de sérialiser l'identité logique de l'article")?;
                store_local_sync_version_in(transaction, "article", &key, "favorite", event)
                    .await?;
            }
            SyncEventPayload::ArticleArchived { article } => {
                let canonical =
                    canonical_sync_subscription_id_in(transaction, &article.subscription_id)
                        .await?;
                let key = serde_json::to_string(&(&canonical, &article.entry_key))
                    .context("Impossible de sérialiser l'identité logique de l'article")?;
                store_local_sync_tombstone_in(transaction, "article", &key, event).await?;
            }
        }
        Ok(())
    }

    /// Atomically allocates a sequence and hybrid time, then appends one local event.
    ///
    /// The allocation and insert share one SQLite transaction. A failed insert
    /// therefore consumes neither a sequence nor a logical-clock tick.
    ///
    /// # Errors
    ///
    /// Returns an error when serialization or SQLite persistence fails.
    pub async fn append_local_sync_event(
        &self,
        payload: &SyncEventPayload,
        observed_at: DateTime<Utc>,
    ) -> Result<SyncEvent> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("Impossible de démarrer l'écriture du journal de synchronisation")?;
        let event =
            Self::append_local_sync_event_in(&mut transaction, payload, observed_at).await?;

        transaction
            .commit()
            .await
            .context("Impossible de valider l'événement de synchronisation")?;

        Ok(event)
    }

    /// Enables synchronization and snapshots the current replicated state once.
    ///
    /// Existing feeds and article states are journaled in the same transaction
    /// that flips the activation flag. Repeated calls are idempotent.
    ///
    /// # Errors
    ///
    /// Returns an error when the complete bootstrap cannot be committed.
    pub async fn enable_sync(&self) -> Result<bool> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("Impossible de démarrer l'activation de la synchronisation")?;
        let claimed: Option<bool> = sqlx::query_scalar(
            r#"
                UPDATE sync_local_state
                SET is_enabled = 1
                WHERE singleton = 1 AND is_enabled = 0
                RETURNING is_enabled
            "#,
        )
        .fetch_optional(&mut *transaction)
        .await
        .context("Impossible d'activer la synchronisation")?;
        if claimed.is_none() {
            transaction
                .commit()
                .await
                .context("Impossible de terminer l'activation de la synchronisation")?;
            return Ok(false);
        }

        let observed_at = Utc::now();
        let feeds: Vec<(String, String, String, bool)> =
            sqlx::query_as("SELECT id, platform, url, is_active FROM feeds ORDER BY id")
                .fetch_all(&mut *transaction)
                .await
                .context("Impossible de charger les abonnements pour le bootstrap")?;
        for (id, platform, url, is_active) in feeds {
            let platform_hint =
                Platform::try_from(platform.as_str()).map_err(anyhow::Error::msg)?;
            Self::append_local_sync_event_in(
                &mut transaction,
                &SyncEventPayload::SubscriptionCreated {
                    subscription_id: id.clone(),
                    normalized_url: url.clone(),
                    platform_hint,
                    is_active,
                    parent_tombstone: None,
                },
                observed_at,
            )
            .await?;
            sqlx::query(
                r#"
                    INSERT OR IGNORE INTO sync_subscription_aliases (
                        alias_id, canonical_id, normalized_url
                    ) VALUES (?, ?, ?)
                "#,
            )
            .bind(&id)
            .bind(&id)
            .bind(&url)
            .execute(&mut *transaction)
            .await
            .context("Impossible d'initialiser l'identité de l'abonnement")?;
        }

        type BootstrapArticleRow = (
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<DateTime<Utc>>,
            bool,
            bool,
            bool,
            Option<String>,
        );
        let articles: Vec<BootstrapArticleRow> = sqlx::query_as(
            r#"
                SELECT id, feed_id, entry_key, title, url, author, published_at,
                       is_read, is_favorite, is_archived, archive_reason
                FROM articles
                ORDER BY feed_id, entry_key
            "#,
        )
        .fetch_all(&mut *transaction)
        .await
        .context("Impossible de charger les articles pour le bootstrap")?;
        for (
            article_id,
            subscription_id,
            entry_key,
            title,
            url,
            author,
            published_at,
            is_read,
            is_favorite,
            is_archived,
            archive_reason,
        ) in articles
        {
            sqlx::query(
                r#"
                    INSERT INTO sync_article_identities (
                        subscription_id, entry_key, article_id
                    ) VALUES (?, ?, ?)
                    ON CONFLICT(subscription_id, entry_key) DO UPDATE SET
                        article_id = excluded.article_id
                "#,
            )
            .bind(&subscription_id)
            .bind(&entry_key)
            .bind(&article_id)
            .execute(&mut *transaction)
            .await
            .context("Impossible d'initialiser l'identité logique de l'article")?;
            let article = SyncArticleRef {
                subscription_id,
                entry_key,
                title,
                url,
                author,
                published_at: published_at.map(|date| date.to_rfc3339()),
            };
            Self::append_local_sync_event_in(
                &mut transaction,
                &SyncEventPayload::ArticleReadSet {
                    article: article.clone(),
                    is_read,
                },
                observed_at,
            )
            .await?;
            Self::append_local_sync_event_in(
                &mut transaction,
                &SyncEventPayload::ArticleFavoriteSet {
                    article: article.clone(),
                    is_favorite,
                },
                observed_at,
            )
            .await?;
            if is_archived && archive_reason.as_deref() == Some("manual") {
                Self::append_local_sync_event_in(
                    &mut transaction,
                    &SyncEventPayload::ArticleArchived { article },
                    observed_at,
                )
                .await?;
            }
        }

        transaction
            .commit()
            .await
            .context("Impossible de valider le bootstrap de synchronisation")?;
        Ok(true)
    }

    /// Reads this installation's immutable journal after an exclusive sequence.
    ///
    /// Results are ordered by sequence and one call is capped at 1,000 rows so
    /// future transports cannot accidentally load an unbounded journal.
    ///
    /// # Errors
    ///
    /// Returns an error when rows cannot be read or contain invalid JSON.
    pub async fn local_sync_events_after(
        &self,
        after_sequence: i64,
        limit: usize,
    ) -> Result<Vec<SyncEvent>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = limit.min(MAX_SYNC_EVENTS_PER_READ) as i64;
        let rows: Vec<SyncEventRow> = sqlx::query_as(
            r#"
                SELECT events.device_id, events.sequence,
                       events.hlc_physical_ms, events.hlc_counter,
                       events.protocol_version, events.event_kind,
                       events.payload_json
                FROM sync_events AS events
                INNER JOIN sync_local_state AS local
                    ON local.device_id = events.device_id
                WHERE local.singleton = 1
                  AND events.sequence > ?
                ORDER BY events.sequence ASC
                LIMIT ?
            "#,
        )
        .bind(after_sequence.max(0))
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .context("Impossible de lire le journal local de synchronisation")?;

        rows.into_iter().map(sync_event_from_row).collect()
    }

    pub(crate) async fn local_sync_export_cursor(&self, key_id: &str) -> Result<i64> {
        Ok(sqlx::query_scalar(
            "SELECT last_exported_sequence FROM sync_export_cursors WHERE key_id = ?",
        )
        .bind(key_id)
        .fetch_optional(&self.pool)
        .await
        .context("Impossible de lire le curseur d'export de synchronisation")?
        .unwrap_or(0))
    }

    pub(crate) async fn mark_local_sync_events_exported(
        &self,
        key_id: &str,
        expected_cursor: i64,
        exported_through: i64,
    ) -> Result<()> {
        if exported_through < expected_cursor {
            anyhow::bail!("Le curseur d'export ne peut pas reculer");
        }
        let result = sqlx::query(
            r#"
                INSERT INTO sync_export_cursors (key_id, last_exported_sequence)
                SELECT ?, ?
                WHERE (
                        ? = 0
                        OR EXISTS (
                            SELECT 1 FROM sync_export_cursors
                            WHERE key_id = ? AND last_exported_sequence = ?
                        )
                      )
                  AND ? < (SELECT next_sequence FROM sync_local_state WHERE singleton = 1)
                ON CONFLICT(key_id) DO UPDATE
                SET last_exported_sequence = excluded.last_exported_sequence
                WHERE sync_export_cursors.last_exported_sequence = ?
                  AND excluded.last_exported_sequence
                      < (SELECT next_sequence FROM sync_local_state WHERE singleton = 1)
            "#,
        )
        .bind(key_id)
        .bind(exported_through)
        .bind(expected_cursor)
        .bind(key_id)
        .bind(expected_cursor)
        .bind(exported_through)
        .bind(expected_cursor)
        .execute(&self.pool)
        .await
        .context("Impossible d'avancer le curseur d'export de synchronisation")?;
        if result.rows_affected() != 1 {
            anyhow::bail!("Le journal local a changé pendant son export");
        }
        Ok(())
    }

    pub(crate) async fn sync_import_cursor(&self, remote_device_id: &str) -> Result<i64> {
        Ok(sqlx::query_scalar(
            "SELECT contiguous_sequence FROM sync_import_cursors WHERE remote_device_id = ?",
        )
        .bind(remote_device_id)
        .fetch_optional(&self.pool)
        .await
        .context("Impossible de lire le curseur d'un appareil distant")?
        .unwrap_or(0))
    }

    /// Reads a transactionally consistent compact checkpoint. Journal
    /// frontiers remain contiguous, while the payload retains only creation
    /// events, current LWW winners, tombstones and unresolved dependencies.
    pub(crate) async fn sync_snapshot_material(
        &self,
        maximum_events: usize,
    ) -> Result<Option<(Vec<(String, i64)>, Vec<SyncEvent>)>> {
        if maximum_events == 0 {
            anyhow::bail!("Un instantané doit autoriser au moins un événement");
        }
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("Impossible de démarrer la lecture de l'instantané")?;
        let (local_device_id, local_next_sequence): (String, i64) = sqlx::query_as(
            "SELECT device_id, next_sequence FROM sync_local_state WHERE singleton = 1",
        )
        .fetch_one(&mut *transaction)
        .await
        .context("Impossible de lire la frontière locale de l'instantané")?;
        let mut frontiers: Vec<(String, i64)> = sqlx::query_as(
            r#"
                SELECT remote_device_id, contiguous_sequence
                FROM sync_import_cursors
                ORDER BY remote_device_id
            "#,
        )
        .fetch_all(&mut *transaction)
        .await
        .context("Impossible de lire les frontières distantes de l'instantané")?;
        frontiers.push((local_device_id, local_next_sequence - 1));
        frontiers.sort_by(|left, right| left.0.cmp(&right.0));
        if frontiers.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            anyhow::bail!("Les frontières de l'instantané contiennent un appareil dupliqué");
        }
        let frontier_by_device = frontiers.iter().cloned().collect::<HashMap<_, _>>();
        let maximum_rows = i64::try_from(maximum_events)
            .context("La limite d'événements de checkpoint est hors limites")?
            .checked_add(1)
            .context("La limite d'événements de checkpoint déborde")?;
        let rows: Vec<SyncEventRow> = sqlx::query_as(
            r#"
                WITH retained(device_id, sequence) AS (
                    SELECT event_device_id, event_sequence FROM sync_entity_versions
                    UNION
                    SELECT event_device_id, event_sequence FROM sync_tombstones
                    UNION
                    SELECT device_id, sequence FROM sync_pending_events
                    UNION
                    SELECT device_id, sequence FROM sync_events
                    WHERE event_kind = 'subscription_created'
                )
                SELECT event.device_id, event.sequence,
                       event.hlc_physical_ms, event.hlc_counter,
                       event.protocol_version, event.event_kind,
                       event.payload_json
                FROM retained
                INNER JOIN sync_events AS event
                    ON event.device_id = retained.device_id
                   AND event.sequence = retained.sequence
                ORDER BY event.device_id, event.sequence
                LIMIT ?
            "#,
        )
        .bind(maximum_rows)
        .fetch_all(&mut *transaction)
        .await
        .context("Impossible de lire les événements compacts du checkpoint")?;
        if rows.len() > maximum_events {
            transaction
                .rollback()
                .await
                .context("Impossible de terminer la lecture de l'instantané")?;
            return Ok(None);
        }
        let events = rows
            .into_iter()
            .map(sync_event_from_row)
            .collect::<Result<Vec<_>>>()?;
        if events.iter().any(|event| {
            frontier_by_device
                .get(&event.device_id)
                .is_none_or(|frontier| event.sequence > *frontier)
        }) {
            transaction
                .rollback()
                .await
                .context("Impossible de terminer la lecture de l'instantané")?;
            return Ok(None);
        }
        transaction
            .commit()
            .await
            .context("Impossible de terminer la lecture de l'instantané")?;
        Ok(Some((frontiers, events)))
    }

    pub(crate) async fn sync_snapshot_publication_hash(
        &self,
        key_id: &str,
        creator_device_id: &str,
    ) -> Result<Option<String>> {
        validate_sync_key_and_device(key_id, creator_device_id)?;
        sqlx::query_scalar(
            "SELECT state_hash FROM sync_snapshot_publications WHERE key_id = ? AND creator_device_id = ?",
        )
        .bind(key_id)
        .bind(creator_device_id)
        .fetch_optional(&self.pool)
        .await
        .context("Impossible de lire l'état de publication de l'instantané")
    }

    pub(crate) async fn record_sync_snapshot_publication(
        &self,
        key_id: &str,
        creator_device_id: &str,
        state_hash: &str,
        published_at: DateTime<Utc>,
    ) -> Result<()> {
        validate_sync_snapshot_identity(key_id, creator_device_id, state_hash)?;
        sqlx::query(
            r#"
                INSERT INTO sync_snapshot_publications (
                    key_id, creator_device_id, state_hash, published_at
                ) VALUES (?, ?, ?, ?)
                ON CONFLICT(key_id, creator_device_id) DO UPDATE SET
                    state_hash = excluded.state_hash,
                    published_at = excluded.published_at
            "#,
        )
        .bind(key_id)
        .bind(creator_device_id)
        .bind(state_hash)
        .bind(published_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .context("Impossible d'enregistrer la publication de l'instantané")?;
        Ok(())
    }

    pub(crate) async fn sync_snapshot_import_hash(
        &self,
        key_id: &str,
        creator_device_id: &str,
    ) -> Result<Option<String>> {
        validate_sync_key_and_device(key_id, creator_device_id)?;
        sqlx::query_scalar(
            "SELECT state_hash FROM sync_snapshot_imports WHERE key_id = ? AND creator_device_id = ?",
        )
        .bind(key_id)
        .bind(creator_device_id)
        .fetch_optional(&self.pool)
        .await
        .context("Impossible de lire l'état d'import de l'instantané")
    }

    pub(crate) async fn record_sync_snapshot_import(
        &self,
        key_id: &str,
        creator_device_id: &str,
        state_hash: &str,
        imported_at: DateTime<Utc>,
    ) -> Result<()> {
        validate_sync_snapshot_identity(key_id, creator_device_id, state_hash)?;
        sqlx::query(
            r#"
                INSERT INTO sync_snapshot_imports (
                    key_id, creator_device_id, state_hash, imported_at
                ) VALUES (?, ?, ?, ?)
                ON CONFLICT(key_id, creator_device_id) DO UPDATE SET
                    state_hash = excluded.state_hash,
                    imported_at = excluded.imported_at
            "#,
        )
        .bind(key_id)
        .bind(creator_device_id)
        .bind(state_hash)
        .bind(imported_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .context("Impossible d'enregistrer l'import de l'instantané")?;
        Ok(())
    }

    /// Records a monotonic acknowledgement learned from one synchronization
    /// device. Older or identical observations never move the durable state
    /// backwards.
    pub async fn record_sync_acknowledgement(
        &self,
        acknowledgement: &SyncAcknowledgement,
    ) -> Result<bool> {
        validate_sync_acknowledgement(acknowledgement)?;
        let result = sqlx::query(
            r#"
                INSERT INTO sync_acknowledgements (
                    key_id, observer_device_id, source_device_id,
                    contiguous_sequence, observed_at
                ) VALUES (?, ?, ?, ?, ?)
                ON CONFLICT(key_id, observer_device_id, source_device_id) DO UPDATE SET
                    contiguous_sequence = excluded.contiguous_sequence,
                    observed_at = excluded.observed_at
                WHERE excluded.contiguous_sequence
                          > sync_acknowledgements.contiguous_sequence
                   OR (
                        excluded.contiguous_sequence
                            = sync_acknowledgements.contiguous_sequence
                        AND excluded.observed_at > sync_acknowledgements.observed_at
                      )
            "#,
        )
        .bind(&acknowledgement.key_id)
        .bind(&acknowledgement.observer_device_id)
        .bind(&acknowledgement.source_device_id)
        .bind(acknowledgement.contiguous_sequence)
        .bind(acknowledgement.observed_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .context("Impossible d'enregistrer l'accusé de réception")?;
        Ok(result.rows_affected() == 1)
    }

    /// Atomically records one authenticated acknowledgement document.
    pub async fn record_sync_acknowledgements(
        &self,
        acknowledgements: &[SyncAcknowledgement],
    ) -> Result<usize> {
        if acknowledgements.is_empty() {
            anyhow::bail!("Un document d'accusé de réception ne peut pas être vide");
        }
        for acknowledgement in acknowledgements {
            validate_sync_acknowledgement(acknowledgement)?;
        }
        let first = &acknowledgements[0];
        if acknowledgements.iter().any(|acknowledgement| {
            acknowledgement.key_id != first.key_id
                || acknowledgement.observer_device_id != first.observer_device_id
        }) {
            anyhow::bail!(
                "Un document d'accusé doit appartenir à un seul observateur et une seule clé"
            );
        }
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("Impossible de démarrer l'enregistrement des accusés de réception")?;
        let mut changed = 0;
        for acknowledgement in acknowledgements {
            changed += sqlx::query(
                r#"
                    INSERT INTO sync_acknowledgements (
                        key_id, observer_device_id, source_device_id,
                        contiguous_sequence, observed_at
                    ) VALUES (?, ?, ?, ?, ?)
                    ON CONFLICT(key_id, observer_device_id, source_device_id) DO UPDATE SET
                        contiguous_sequence = excluded.contiguous_sequence,
                        observed_at = excluded.observed_at
                    WHERE excluded.contiguous_sequence
                              > sync_acknowledgements.contiguous_sequence
                       OR (
                            excluded.contiguous_sequence
                                = sync_acknowledgements.contiguous_sequence
                            AND excluded.observed_at > sync_acknowledgements.observed_at
                          )
                "#,
            )
            .bind(&acknowledgement.key_id)
            .bind(&acknowledgement.observer_device_id)
            .bind(&acknowledgement.source_device_id)
            .bind(acknowledgement.contiguous_sequence)
            .bind(acknowledgement.observed_at.to_rfc3339())
            .execute(&mut *transaction)
            .await
            .context("Impossible d'enregistrer un document d'accusé de réception")?
            .rows_affected() as usize;
        }
        transaction
            .commit()
            .await
            .context("Impossible de valider les accusés de réception")?;
        Ok(changed)
    }

    /// Lists the durable acknowledgements for one source journal.
    pub async fn sync_acknowledgements_for_source(
        &self,
        key_id: &str,
        source_device_id: &str,
    ) -> Result<Vec<SyncAcknowledgement>> {
        if key_id.len() != 64
            || !key_id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            anyhow::bail!("Empreinte de clé d'accusé de réception invalide");
        }
        uuid::Uuid::parse_str(source_device_id)
            .context("Identifiant d'appareil source invalide")?;
        let rows: Vec<(String, String, String, i64, String)> = sqlx::query_as(
            r#"
                SELECT key_id, observer_device_id, source_device_id,
                       contiguous_sequence, observed_at
                FROM sync_acknowledgements
                WHERE key_id = ? AND source_device_id = ?
                ORDER BY observer_device_id
            "#,
        )
        .bind(key_id)
        .bind(source_device_id)
        .fetch_all(&self.pool)
        .await
        .context("Impossible de charger les accusés de réception")?;
        rows.into_iter()
            .map(
                |(
                    key_id,
                    observer_device_id,
                    source_device_id,
                    contiguous_sequence,
                    observed_at,
                )| {
                    Ok(SyncAcknowledgement {
                        key_id,
                        observer_device_id,
                        source_device_id,
                        contiguous_sequence,
                        observed_at: DateTime::parse_from_rfc3339(&observed_at)
                            .context("Date d'accusé de réception invalide")?
                            .with_timezone(&Utc),
                    })
                },
            )
            .collect()
    }

    /// Builds the local device's current acknowledgement vector. The local
    /// journal and every contiguous remote import cursor are included in a
    /// deterministic order, including sequence zero for a fresh device.
    pub async fn local_sync_acknowledgement_snapshot(
        &self,
        key_id: &str,
        observed_at: DateTime<Utc>,
    ) -> Result<Vec<SyncAcknowledgement>> {
        if key_id.len() != 64
            || !key_id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            anyhow::bail!("Empreinte de clé d'accusé de réception invalide");
        }
        let identity = self.sync_identity().await?;
        let rows: Vec<(String, i64)> = sqlx::query_as(
            r#"
                SELECT remote_device_id, contiguous_sequence
                FROM sync_import_cursors
                ORDER BY remote_device_id
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .context("Impossible de charger les curseurs à acquitter")?;
        let mut positions = rows
            .into_iter()
            .map(
                |(source_device_id, contiguous_sequence)| SyncAcknowledgement {
                    key_id: key_id.to_string(),
                    observer_device_id: identity.device_id.clone(),
                    source_device_id,
                    contiguous_sequence,
                    observed_at,
                },
            )
            .collect::<Vec<_>>();
        positions.push(SyncAcknowledgement {
            key_id: key_id.to_string(),
            observer_device_id: identity.device_id.clone(),
            source_device_id: identity.device_id,
            contiguous_sequence: identity.next_sequence - 1,
            observed_at,
        });
        positions.sort_by(|left, right| left.source_device_id.cmp(&right.source_device_id));
        positions.dedup_by(|left, right| left.source_device_id == right.source_device_id);
        Ok(positions)
    }

    /// Computes the highest prefix that every explicitly required observer has
    /// consumed. Missing acknowledgements deliberately contribute zero.
    ///
    /// The caller must provide the complete authoritative active-device roster;
    /// the current pairing metadata is not yet a distributed membership list.
    pub async fn sync_compaction_frontier(
        &self,
        key_id: &str,
        source_device_id: &str,
        source_max_sequence: i64,
        required_observer_device_ids: &[String],
    ) -> Result<SyncCompactionFrontier> {
        uuid::Uuid::parse_str(source_device_id)
            .context("Identifiant d'appareil source invalide")?;
        if source_max_sequence < 0 {
            anyhow::bail!("La dernière séquence source ne peut pas être négative");
        }
        let mut required = required_observer_device_ids
            .iter()
            .map(|device_id| {
                uuid::Uuid::parse_str(device_id)
                    .context("Identifiant d'appareil requis invalide")?;
                Ok(device_id.clone())
            })
            .collect::<Result<Vec<_>>>()?;
        required.sort();
        required.dedup();
        if required.is_empty() {
            anyhow::bail!("Le calcul de compaction exige au moins un appareil");
        }

        let local = self.sync_identity().await?;
        let stored = self
            .sync_acknowledgements_for_source(key_id, source_device_id)
            .await?
            .into_iter()
            .map(|acknowledgement| {
                (
                    acknowledgement.observer_device_id,
                    acknowledgement.contiguous_sequence,
                )
            })
            .collect::<HashMap<_, _>>();
        let mut positions = Vec::with_capacity(required.len());
        for observer_device_id in &required {
            let sequence = if observer_device_id == &local.device_id {
                if source_device_id == local.device_id {
                    local.next_sequence - 1
                } else {
                    self.sync_import_cursor(source_device_id).await?
                }
            } else {
                stored.get(observer_device_id).copied().unwrap_or(0)
            };
            positions.push((
                observer_device_id.clone(),
                sequence.min(source_max_sequence),
            ));
        }
        let safe_through_sequence = positions
            .iter()
            .map(|(_, sequence)| *sequence)
            .min()
            .unwrap_or(0);
        let blocking_observer_device_ids = if safe_through_sequence < source_max_sequence {
            positions
                .into_iter()
                .filter_map(|(device_id, sequence)| {
                    (sequence == safe_through_sequence).then_some(device_id)
                })
                .collect()
        } else {
            Vec::new()
        };
        Ok(SyncCompactionFrontier {
            source_device_id: source_device_id.to_string(),
            safe_through_sequence,
            required_observer_count: required.len(),
            blocking_observer_device_ids,
        })
    }

    /// Computes a compaction boundary from the complete active distributed
    /// roster. This is the only frontier suitable for a future destructive
    /// operation; callers cannot omit a lagging member.
    pub async fn authoritative_sync_compaction_frontier(
        &self,
        key_id: &str,
        source_device_id: &str,
        source_max_sequence: i64,
    ) -> Result<SyncCompactionFrontier> {
        let required = self.active_sync_roster_device_ids(key_id).await?;
        let local_device_id = self.sync_identity().await?.device_id;
        if !required
            .iter()
            .any(|device_id| device_id == &local_device_id)
        {
            anyhow::bail!("L'appareil local est absent du registre actif");
        }
        self.sync_compaction_frontier(key_id, source_device_id, source_max_sequence, &required)
            .await
    }

    /// Removes a bounded set of synchronization events made redundant by the
    /// compact checkpoint. The authoritative roster and every acknowledgement
    /// are read in the same transaction as the deletions so a newly observed
    /// active device can never be omitted from the safety boundary.
    ///
    /// Current projection winners, tombstones, unresolved dependencies and
    /// subscription creation events are retained. The latter are deliberately
    /// conservative because they carry incarnation ancestry required when a
    /// checkpoint is restored without the original journal.
    pub(crate) async fn compact_sync_events(
        &self,
        key_id: &str,
        checkpoint_frontiers: &[(String, i64)],
        maximum_events: usize,
    ) -> Result<usize> {
        if key_id.len() != 64
            || !key_id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            anyhow::bail!("Identifiant de clé de compaction invalide");
        }
        if maximum_events == 0 {
            return Ok(0);
        }
        let mut checkpoint_by_device = HashMap::new();
        for (device_id, sequence) in checkpoint_frontiers {
            uuid::Uuid::parse_str(device_id)
                .context("Identifiant d'appareil du checkpoint invalide")?;
            if *sequence < 0
                || checkpoint_by_device
                    .insert(device_id.clone(), *sequence)
                    .is_some()
            {
                anyhow::bail!("Frontières du checkpoint invalides");
            }
        }
        if checkpoint_by_device.is_empty() {
            anyhow::bail!("Le checkpoint ne contient aucune frontière");
        }
        let maximum_events = maximum_events.min(MAX_SYNC_EVENTS_COMPACTED_PER_CYCLE);
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("Impossible de démarrer la compaction de synchronisation")?;
        let (local_device_id, local_max_sequence): (String, i64) = sqlx::query_as(
            "SELECT device_id, next_sequence - 1 FROM sync_local_state WHERE singleton = 1",
        )
        .fetch_one(&mut *transaction)
        .await
        .context("Impossible de lire le journal local pour la compaction")?;
        let required_observers: Vec<String> = sqlx::query_scalar(
            r#"
                SELECT device_id
                FROM sync_roster_members
                WHERE key_id = ? AND revoked_at IS NULL
                ORDER BY device_id
            "#,
        )
        .bind(key_id)
        .fetch_all(&mut *transaction)
        .await
        .context("Impossible de lire le registre autoritaire pour la compaction")?;
        if required_observers.is_empty()
            || !required_observers
                .iter()
                .any(|device_id| device_id == &local_device_id)
        {
            anyhow::bail!("L'appareil local est absent du registre actif");
        }

        let remote_frontiers: Vec<(String, i64)> = sqlx::query_as(
            r#"
                SELECT remote_device_id, contiguous_sequence
                FROM sync_import_cursors
                ORDER BY remote_device_id
            "#,
        )
        .fetch_all(&mut *transaction)
        .await
        .context("Impossible de lire les frontières importées pour la compaction")?;
        let mut source_frontiers = remote_frontiers.iter().cloned().collect::<HashMap<_, _>>();
        source_frontiers.insert(local_device_id.clone(), local_max_sequence);
        source_frontiers.retain(|device_id, sequence| {
            checkpoint_by_device
                .get(device_id)
                .is_some_and(|checkpoint| {
                    *sequence = (*sequence).min(*checkpoint);
                    true
                })
        });
        let acknowledgements: Vec<(String, String, i64)> = sqlx::query_as(
            r#"
                SELECT observer_device_id, source_device_id, contiguous_sequence
                FROM sync_acknowledgements
                WHERE key_id = ?
            "#,
        )
        .bind(key_id)
        .fetch_all(&mut *transaction)
        .await
        .context("Impossible de lire les accusés pour la compaction")?;
        let acknowledged = acknowledgements
            .into_iter()
            .map(|(observer, source, sequence)| ((observer, source), sequence))
            .collect::<HashMap<_, _>>();

        let mut sources = source_frontiers.into_iter().collect::<Vec<_>>();
        sources.sort_by(|left, right| left.0.cmp(&right.0));
        let mut candidates = Vec::new();
        for (source_device_id, source_frontier) in sources {
            if candidates.len() == maximum_events {
                break;
            }
            let safe_through = required_observers
                .iter()
                .map(|observer_device_id| {
                    if observer_device_id == &local_device_id {
                        source_frontier
                    } else {
                        acknowledged
                            .get(&(observer_device_id.clone(), source_device_id.clone()))
                            .copied()
                            .unwrap_or(0)
                            .min(source_frontier)
                    }
                })
                .min()
                .unwrap_or(0);
            if safe_through == 0 {
                continue;
            }
            let remaining = i64::try_from(maximum_events - candidates.len())
                .context("Limite de compaction hors limites")?;
            let mut source_candidates: Vec<(String, i64)> = sqlx::query_as(
                r#"
                    SELECT event.device_id, event.sequence
                    FROM sync_events AS event
                    WHERE event.device_id = ?
                      AND event.sequence <= ?
                      AND event.event_kind <> 'subscription_created'
                      AND NOT EXISTS (
                          SELECT 1 FROM sync_entity_versions AS version
                          WHERE version.event_device_id = event.device_id
                            AND version.event_sequence = event.sequence
                      )
                      AND NOT EXISTS (
                          SELECT 1 FROM sync_tombstones AS tombstone
                          WHERE tombstone.event_device_id = event.device_id
                            AND tombstone.event_sequence = event.sequence
                      )
                      AND NOT EXISTS (
                          SELECT 1 FROM sync_pending_events AS pending
                          WHERE pending.device_id = event.device_id
                            AND pending.sequence = event.sequence
                      )
                    ORDER BY event.sequence
                    LIMIT ?
                "#,
            )
            .bind(&source_device_id)
            .bind(safe_through)
            .bind(remaining)
            .fetch_all(&mut *transaction)
            .await
            .context("Impossible de sélectionner les événements à compacter")?;
            candidates.append(&mut source_candidates);
        }

        let mut deleted = 0usize;
        for (device_id, sequence) in candidates {
            let result =
                sqlx::query("DELETE FROM sync_events WHERE device_id = ? AND sequence = ?")
                    .bind(device_id)
                    .bind(sequence)
                    .execute(&mut *transaction)
                    .await
                    .context("Impossible de compacter un événement de synchronisation")?;
            deleted += result.rows_affected() as usize;
        }
        transaction
            .commit()
            .await
            .context("Impossible de valider la compaction de synchronisation")?;
        Ok(deleted)
    }

    /// Validates and atomically imports remote synchronization events.
    ///
    /// Remote application writes projections directly and therefore never
    /// creates outgoing local events.
    ///
    /// # Errors
    ///
    /// Returns an error without changing the database when validation or any
    /// SQLite operation fails.
    pub async fn import_sync_events(
        &self,
        events: &[SyncEvent],
        observed_at: DateTime<Utc>,
    ) -> Result<SyncImportReport> {
        sync_merge::import_sync_events(&self.pool, events, observed_at).await
    }

    pub(crate) async fn import_sync_checkpoint_events(
        &self,
        events: &[SyncEvent],
        frontiers: &[(String, i64)],
        observed_at: DateTime<Utc>,
        maximum: usize,
    ) -> Result<SyncImportReport> {
        sync_merge::import_sync_checkpoint_events(
            &self.pool,
            events,
            frontiers,
            observed_at,
            maximum,
        )
        .await
    }

    /// Imports the configured subscriptions as the active feed set.
    ///
    /// Feeds absent from the imported set are marked inactive. Existing rows
    /// are retained so their article history can remain available.
    ///
    /// # Errors
    ///
    /// Returns an error when the complete import transaction cannot be applied.
    pub async fn import_feeds(&self, feeds: &[FeedConfig]) -> Result<()> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("Impossible de commencer l'import des abonnements")?;

        sqlx::query("UPDATE feeds SET is_active = 0")
            .execute(&mut *transaction)
            .await
            .context("Impossible de désactiver les anciens abonnements")?;

        for feed in feeds {
            sqlx::query(
                r#"
                    INSERT INTO feeds (id, platform, url, is_active)
                    VALUES (?, ?, ?, 1)
                    ON CONFLICT(id) DO UPDATE SET
                        platform = excluded.platform,
                        url = excluded.url,
                        is_active = 1
                "#,
            )
            .bind(&feed.id)
            .bind(feed.platform.as_str())
            .bind(&feed.url)
            .execute(&mut *transaction)
            .await
            .with_context(|| format!("Impossible d'importer l'abonnement {:?}", feed.id))?;
        }

        transaction
            .commit()
            .await
            .context("Impossible de valider l'import des abonnements")
    }

    /// Lists every persisted feed, including inactive subscriptions.
    ///
    /// # Errors
    ///
    /// Returns an error when rows cannot be loaded or contain an unknown platform.
    pub async fn list_feeds(&self) -> Result<Vec<StoredFeed>> {
        let rows: Vec<StoredFeedRow> = sqlx::query_as(
            r#"
                SELECT feeds.id, feeds.platform, feeds.url, feeds.is_active,
                       feeds.title, feeds.description,
                       COALESCE(
                           feeds.author,
                           (
                               SELECT articles.author
                               FROM articles
                               WHERE articles.feed_id = feeds.id
                                 AND articles.author IS NOT NULL
                                 AND TRIM(articles.author) <> ''
                               ORDER BY articles.published_at IS NULL ASC,
                                        articles.published_at DESC,
                                        articles.id ASC
                               LIMIT 1
                           )
                       ) AS author,
                       feeds.last_success_at, feeds.last_error_stage,
                       feeds.last_error_message, feeds.last_error_at,
                       MAX(articles.published_at) AS last_published_at,
                       feeds.logo_png
                FROM feeds
                LEFT JOIN articles ON articles.feed_id = feeds.id
                GROUP BY feeds.id
                ORDER BY feeds.is_active DESC,
                         COALESCE(feeds.title, feeds.url) COLLATE NOCASE ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .context("Impossible de charger les abonnements")?;

        rows.into_iter().map(stored_feed_from_row).collect()
    }

    /// Adds a subscription or reactivates a previously disabled matching URL.
    ///
    /// A generated UUID remains stable for the lifetime of a subscription.
    pub async fn add_feed(
        &self,
        raw_url: &str,
        platform_override: Option<Platform>,
    ) -> std::result::Result<StoredFeed, SubscriptionError> {
        let url = normalize_feed_url(raw_url).map_err(SubscriptionError::InvalidUrl)?;
        let platform = platform_override.unwrap_or_else(|| detect_platform(&url));
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| SubscriptionError::Database(error.to_string()))?;
        let existing: Option<(String, bool, String)> = sqlx::query_as(
            r#"
                SELECT id, is_active, platform
                FROM feeds
                WHERE url = ?
                ORDER BY is_active DESC, id
                LIMIT 1
            "#,
        )
        .bind(&url)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| SubscriptionError::Database(error.to_string()))?;

        if let Some((id, is_active, previous_platform)) = existing {
            if is_active {
                return Err(SubscriptionError::DuplicateActiveUrl(url));
            }
            sqlx::query("UPDATE feeds SET platform = ?, is_active = 1 WHERE id = ?")
                .bind(platform.as_str())
                .bind(&id)
                .execute(&mut *transaction)
                .await
                .map_err(|error| SubscriptionError::Database(error.to_string()))?;
            if Self::sync_is_enabled_in(&mut transaction)
                .await
                .map_err(|error| SubscriptionError::Database(error.to_string()))?
            {
                let observed_at = Utc::now();
                if previous_platform != platform.as_str() {
                    Self::append_local_sync_event_in(
                        &mut transaction,
                        &SyncEventPayload::SubscriptionPlatformSet {
                            subscription_id: id.clone(),
                            platform_hint: platform,
                        },
                        observed_at,
                    )
                    .await
                    .map_err(|error| SubscriptionError::Database(error.to_string()))?;
                }
                Self::append_local_sync_event_in(
                    &mut transaction,
                    &SyncEventPayload::SubscriptionActiveSet {
                        subscription_id: id.clone(),
                        is_active: true,
                    },
                    observed_at,
                )
                .await
                .map_err(|error| SubscriptionError::Database(error.to_string()))?;
            }
            transaction
                .commit()
                .await
                .map_err(|error| SubscriptionError::Database(error.to_string()))?;
            return self
                .list_feeds()
                .await
                .map_err(|error| SubscriptionError::Database(error.to_string()))?
                .into_iter()
                .find(|feed| feed.id == id)
                .ok_or(SubscriptionError::NotFound(id));
        }

        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO feeds (id, platform, url, is_active) VALUES (?, ?, ?, 1)")
            .bind(&id)
            .bind(platform.as_str())
            .bind(&url)
            .execute(&mut *transaction)
            .await
            .map_err(|error| {
                if error.as_database_error().is_some_and(|database_error| {
                    database_error.message().contains("feeds.url")
                        || database_error.message().contains("feeds_unique_active_url")
                }) {
                    SubscriptionError::DuplicateActiveUrl(url.clone())
                } else {
                    SubscriptionError::Database(error.to_string())
                }
            })?;

        if Self::sync_is_enabled_in(&mut transaction)
            .await
            .map_err(|error| SubscriptionError::Database(error.to_string()))?
        {
            let parent_tombstone: Option<(String, i64)> = sqlx::query_as(
                r#"
                    SELECT tombstone.event_device_id, tombstone.event_sequence
                    FROM sync_subscription_aliases AS alias
                    INNER JOIN sync_tombstones AS tombstone
                        ON tombstone.entity_kind = 'subscription'
                       AND tombstone.entity_key = alias.canonical_id
                    INNER JOIN sync_events AS event
                        ON event.device_id = tombstone.event_device_id
                       AND event.sequence = tombstone.event_sequence
                    WHERE alias.normalized_url = ?
                    ORDER BY event.hlc_physical_ms DESC,
                             event.hlc_counter DESC,
                             event.device_id DESC,
                             event.sequence DESC
                    LIMIT 1
                "#,
            )
            .bind(&url)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|error| SubscriptionError::Database(error.to_string()))?;
            let parent_tombstone = parent_tombstone.map(|(device_id, sequence)| SyncEventId {
                device_id,
                sequence,
            });
            Self::append_local_sync_event_in(
                &mut transaction,
                &SyncEventPayload::SubscriptionCreated {
                    subscription_id: id.clone(),
                    normalized_url: url.clone(),
                    platform_hint: platform,
                    is_active: true,
                    parent_tombstone: parent_tombstone.clone(),
                },
                Utc::now(),
            )
            .await
            .map_err(|error| SubscriptionError::Database(error.to_string()))?;
            sqlx::query(
                r#"
                    INSERT INTO sync_subscription_aliases (
                        alias_id, canonical_id, normalized_url,
                        parent_tombstone_device_id, parent_tombstone_sequence
                    ) VALUES (?, ?, ?, ?, ?)
                "#,
            )
            .bind(&id)
            .bind(&id)
            .bind(&url)
            .bind(parent_tombstone.as_ref().map(|event| &event.device_id))
            .bind(parent_tombstone.as_ref().map(|event| event.sequence))
            .execute(&mut *transaction)
            .await
            .map_err(|error| SubscriptionError::Database(error.to_string()))?;
        }

        transaction
            .commit()
            .await
            .map_err(|error| SubscriptionError::Database(error.to_string()))?;

        Ok(StoredFeed {
            id,
            platform,
            url,
            is_active: true,
            title: None,
            description: None,
            author: None,
            last_published_at: None,
            last_success_at: None,
            last_error: None,
            logo_png: None,
        })
    }

    /// Activates or deactivates a retained subscription without deleting history.
    pub async fn set_feed_active(
        &self,
        feed_id: &str,
        is_active: bool,
    ) -> std::result::Result<StoredFeed, SubscriptionError> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| SubscriptionError::Database(error.to_string()))?;
        let feed: Option<(String, String)> =
            sqlx::query_as("SELECT id, url FROM feeds WHERE id = ?")
                .bind(feed_id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(|error| SubscriptionError::Database(error.to_string()))?;
        let (id, url) = feed.ok_or_else(|| SubscriptionError::NotFound(feed_id.to_string()))?;

        if is_active {
            let duplicate: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM feeds WHERE url = ? AND is_active = 1 AND id <> ?)",
            )
            .bind(&url)
            .bind(&id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(|error| SubscriptionError::Database(error.to_string()))?;
            if duplicate {
                return Err(SubscriptionError::DuplicateActiveUrl(url));
            }
        }

        sqlx::query("UPDATE feeds SET is_active = ? WHERE id = ?")
            .bind(is_active)
            .bind(&id)
            .execute(&mut *transaction)
            .await
            .map_err(|error| SubscriptionError::Database(error.to_string()))?;
        if Self::sync_is_enabled_in(&mut transaction)
            .await
            .map_err(|error| SubscriptionError::Database(error.to_string()))?
        {
            Self::append_local_sync_event_in(
                &mut transaction,
                &SyncEventPayload::SubscriptionActiveSet {
                    subscription_id: id.clone(),
                    is_active,
                },
                Utc::now(),
            )
            .await
            .map_err(|error| SubscriptionError::Database(error.to_string()))?;
        }
        transaction
            .commit()
            .await
            .map_err(|error| SubscriptionError::Database(error.to_string()))?;
        self.list_feeds()
            .await
            .map_err(|error| SubscriptionError::Database(error.to_string()))?
            .into_iter()
            .find(|feed| feed.id == id)
            .ok_or_else(|| SubscriptionError::NotFound(feed_id.to_string()))
    }

    /// Permanently deletes a subscription and all of its cached articles.
    ///
    /// The operation is atomic: article state is stored on the article rows, so
    /// deleting them also removes read and favorite state. Deactivation remains
    /// available when the user wants to retain that history.
    pub async fn delete_feed(
        &self,
        feed_id: &str,
    ) -> std::result::Result<DeleteFeedResult, SubscriptionError> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| SubscriptionError::Database(error.to_string()))?;

        let feed: Option<(String, String)> =
            sqlx::query_as("SELECT id, url FROM feeds WHERE id = ?")
                .bind(feed_id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(|error| SubscriptionError::Database(error.to_string()))?;
        let (feed_id, url) =
            feed.ok_or_else(|| SubscriptionError::NotFound(feed_id.to_string()))?;

        if Self::sync_is_enabled_in(&mut transaction)
            .await
            .map_err(|error| SubscriptionError::Database(error.to_string()))?
        {
            sqlx::query(
                r#"
                    INSERT OR IGNORE INTO sync_subscription_aliases (
                        alias_id, canonical_id, normalized_url
                    ) VALUES (?, ?, ?)
                "#,
            )
            .bind(&feed_id)
            .bind(&feed_id)
            .bind(&url)
            .execute(&mut *transaction)
            .await
            .map_err(|error| SubscriptionError::Database(error.to_string()))?;
            Self::append_local_sync_event_in(
                &mut transaction,
                &SyncEventPayload::SubscriptionDeleted {
                    subscription_id: feed_id.clone(),
                },
                Utc::now(),
            )
            .await
            .map_err(|error| SubscriptionError::Database(error.to_string()))?;
        }

        let deleted_articles = sqlx::query("DELETE FROM articles WHERE feed_id = ?")
            .bind(&feed_id)
            .execute(&mut *transaction)
            .await
            .map_err(|error| SubscriptionError::Database(error.to_string()))?
            .rows_affected() as usize;

        sqlx::query("DELETE FROM feeds WHERE id = ?")
            .bind(&feed_id)
            .execute(&mut *transaction)
            .await
            .map_err(|error| SubscriptionError::Database(error.to_string()))?;

        transaction
            .commit()
            .await
            .map_err(|error| SubscriptionError::Database(error.to_string()))?;

        Ok(DeleteFeedResult {
            feed_id,
            deleted_articles,
        })
    }

    /// Returns active subscriptions in the configuration shape used by collection.
    pub async fn active_feed_config(&self) -> Result<Vec<FeedConfig>> {
        self.list_feeds()
            .await?
            .into_iter()
            .filter(|feed| feed.is_active)
            .map(|feed| {
                Ok(FeedConfig {
                    id: feed.id,
                    platform: feed.platform,
                    url: feed.url,
                })
            })
            .collect()
    }

    /// Returns one active subscription in the configuration shape used by collection.
    pub async fn active_feed_config_for(
        &self,
        feed_id: &str,
    ) -> std::result::Result<FeedConfig, SubscriptionError> {
        let feed = self
            .list_feeds()
            .await
            .map_err(|error| SubscriptionError::Database(error.to_string()))?
            .into_iter()
            .find(|feed| feed.id == feed_id)
            .ok_or_else(|| SubscriptionError::NotFound(feed_id.to_string()))?;
        if !feed.is_active {
            return Err(SubscriptionError::Inactive(feed_id.to_string()));
        }
        Ok(FeedConfig {
            id: feed.id,
            platform: feed.platform,
            url: feed.url,
        })
    }

    /// Persists metadata and the latest success or failure for each attempted feed.
    ///
    /// A successful result clears the previous error. A failed result retains the
    /// last successful metadata and publication history.
    pub async fn record_feed_refreshes(
        &self,
        successful_feeds: &[FeedMetadata],
        failures: &[FeedRefreshFailure],
        refreshed_at: DateTime<Utc>,
    ) -> Result<()> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("Impossible de commencer l'enregistrement des états de flux")?;

        for feed in successful_feeds {
            let description = (!feed.description.trim().is_empty()).then_some(&feed.description);
            let author = feed
                .author
                .as_ref()
                .filter(|author| !author.trim().is_empty());
            let previous_site: Option<String> =
                sqlx::query_scalar("SELECT site_url FROM feeds WHERE id = ?")
                    .bind(&feed.id)
                    .fetch_optional(&mut *transaction)
                    .await
                    .with_context(|| {
                        format!("Impossible de charger le site du flux {:?}", feed.id)
                    })?
                    .flatten();
            let site_changed = previous_site
                .as_deref()
                .is_some_and(|site| !same_site_domain(site, &feed.site_url));
            sqlx::query(
                r#"
                    UPDATE feeds
                    SET title = ?, description = ?, author = ?, last_success_at = ?,
                        logo_png = CASE WHEN ? THEN NULL ELSE logo_png END,
                        logo_site_url = CASE WHEN ? THEN NULL ELSE logo_site_url END,
                        logo_attempted_at = CASE WHEN ? THEN NULL ELSE logo_attempted_at END,
                        logo_attempted_site_url = CASE WHEN ? THEN NULL ELSE logo_attempted_site_url END,
                        logo_attempted_declared_url = CASE WHEN ? THEN NULL ELSE logo_attempted_declared_url END,
                        logo_last_error = CASE WHEN ? THEN NULL ELSE logo_last_error END,
                        site_url = ?, declared_icon_url = ?,
                        last_error_stage = NULL, last_error_message = NULL,
                        last_error_at = NULL
                    WHERE id = ?
                "#,
            )
            .bind(&feed.title)
            .bind(description)
            .bind(author)
            .bind(refreshed_at)
            .bind(site_changed)
            .bind(site_changed)
            .bind(site_changed)
            .bind(site_changed)
            .bind(site_changed)
            .bind(site_changed)
            .bind(&feed.site_url)
            .bind(&feed.declared_icon_url)
            .bind(&feed.id)
            .execute(&mut *transaction)
            .await
            .with_context(|| format!("Impossible d'enregistrer le succès du flux {:?}", feed.id))?;
        }

        for failure in failures {
            sqlx::query(
                r#"
                    UPDATE feeds
                    SET last_error_stage = ?, last_error_message = ?, last_error_at = ?
                    WHERE id = ?
                "#,
            )
            .bind(&failure.stage)
            .bind(&failure.message)
            .bind(refreshed_at)
            .bind(&failure.feed_id)
            .execute(&mut *transaction)
            .await
            .with_context(|| {
                format!(
                    "Impossible d'enregistrer l'échec du flux {:?}",
                    failure.feed_id
                )
            })?;
        }

        transaction
            .commit()
            .await
            .context("Impossible de valider les états de rafraîchissement des flux")
    }

    /// Selects logo discoveries due for the feeds that refreshed successfully.
    pub async fn feed_logo_candidates(
        &self,
        successful_feed_ids: &[String],
        now: DateTime<Utc>,
    ) -> Result<Vec<FeedLogoCandidate>> {
        type LogoRow = (
            String,
            String,
            Option<String>,
            Option<Vec<u8>>,
            Option<String>,
            Option<DateTime<Utc>>,
            Option<String>,
            Option<String>,
        );
        let successful = successful_feed_ids
            .iter()
            .collect::<std::collections::HashSet<_>>();
        let retry_cutoff = now - chrono::Duration::days(LOGO_RETRY_DAYS);
        let rows: Vec<LogoRow> = sqlx::query_as(
            r#"
                SELECT id, site_url, declared_icon_url, logo_png, logo_site_url,
                       logo_attempted_at, logo_attempted_site_url,
                       logo_attempted_declared_url
                FROM feeds
                WHERE is_active = 1
                  AND platform = 'other'
                  AND site_url IS NOT NULL
                  AND TRIM(site_url) <> ''
                ORDER BY last_success_at DESC, id ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .context("Impossible de sélectionner les logos de flux")?;

        Ok(rows
            .into_iter()
            .filter(|(id, ..)| successful.contains(id))
            .filter(
                |(
                    _,
                    site_url,
                    declared_icon_url,
                    logo_png,
                    logo_site_url,
                    attempted_at,
                    attempted_site_url,
                    attempted_declared_url,
                )| {
                    if logo_png.is_some() {
                        return logo_site_url
                            .as_deref()
                            .is_none_or(|logo_site| !same_site_domain(logo_site, site_url));
                    }
                    attempted_at.is_none()
                        || attempted_site_url.as_deref().is_none_or(|attempted_site| {
                            !same_site_domain(attempted_site, site_url)
                        })
                        || attempted_declared_url != declared_icon_url
                        || attempted_at.is_some_and(|attempted| attempted <= retry_cutoff)
                },
            )
            .take(MAX_LOGO_ATTEMPTS_PER_REFRESH)
            .map(
                |(feed_id, site_url, declared_icon_url, ..)| FeedLogoCandidate {
                    feed_id,
                    site_url,
                    declared_icon_url,
                },
            )
            .collect())
    }

    /// Stores a normalized feed logo only if the feed still targets the same site.
    pub async fn record_feed_logo_success(
        &self,
        candidate: &FeedLogoCandidate,
        png: &[u8],
        attempted_at: DateTime<Utc>,
    ) -> Result<bool> {
        let result = sqlx::query(
            r#"
                UPDATE feeds
                SET logo_png = ?, logo_site_url = ?, logo_attempted_at = ?,
                    logo_attempted_site_url = ?, logo_attempted_declared_url = ?,
                    logo_last_error = NULL
                WHERE id = ? AND site_url = ? AND platform = 'other'
            "#,
        )
        .bind(png)
        .bind(&candidate.site_url)
        .bind(attempted_at)
        .bind(&candidate.site_url)
        .bind(&candidate.declared_icon_url)
        .bind(&candidate.feed_id)
        .bind(&candidate.site_url)
        .execute(&self.pool)
        .await
        .context("Impossible d’enregistrer le logo du flux")?;
        Ok(result.rows_affected() == 1)
    }

    /// Records a non-blocking logo discovery failure for retry cooldown.
    pub async fn record_feed_logo_failure(
        &self,
        candidate: &FeedLogoCandidate,
        message: &str,
        attempted_at: DateTime<Utc>,
    ) -> Result<bool> {
        let bounded_message = message.chars().take(1_000).collect::<String>();
        let result = sqlx::query(
            r#"
                UPDATE feeds
                SET logo_attempted_at = ?, logo_attempted_site_url = ?,
                    logo_attempted_declared_url = ?, logo_last_error = ?
                WHERE id = ? AND site_url = ? AND platform = 'other'
            "#,
        )
        .bind(attempted_at)
        .bind(&candidate.site_url)
        .bind(&candidate.declared_icon_url)
        .bind(bounded_message)
        .bind(&candidate.feed_id)
        .bind(&candidate.site_url)
        .execute(&self.pool)
        .await
        .context("Impossible d’enregistrer l’échec du logo du flux")?;
        Ok(result.rows_affected() == 1)
    }

    /// Inserts new articles and refreshes existing remote metadata.
    ///
    /// Missing incoming values do not erase data previously stored, and local
    /// read/favorite flags are intentionally absent from the conflict update.
    ///
    /// # Errors
    ///
    /// Returns an error when the complete article transaction cannot be applied.
    pub async fn upsert_articles(&self, articles: &[Article]) -> Result<UpsertStats> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("Impossible de commencer l'enregistrement des articles")?;
        let mut stats = UpsertStats::default();

        for article in articles {
            let entry_key = article_entry_key(&article.id, &article.feed_id);
            let sync_subscription_id =
                canonical_sync_subscription_id_in(&mut transaction, &article.feed_id).await?;
            let existing_archived: Option<bool> =
                sqlx::query_scalar("SELECT is_archived FROM articles WHERE id = ?")
                    .bind(&article.id)
                    .fetch_optional(&mut *transaction)
                    .await
                    .with_context(|| {
                        format!("Impossible de rechercher l'article {:?}", article.id)
                    })?;

            if existing_archived == Some(true) {
                continue;
            }
            let already_exists = existing_archived.is_some();

            sqlx::query(
                r#"
                    INSERT INTO articles (
                        id, feed_id, title, author, published_at, url, content,
                        content_kind, source, entry_key
                    )
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                    ON CONFLICT(id) DO UPDATE SET
                        feed_id = excluded.feed_id,
                        entry_key = COALESCE(articles.entry_key, excluded.entry_key),
                        title = COALESCE(excluded.title, articles.title),
                        author = COALESCE(excluded.author, articles.author),
                        published_at = COALESCE(excluded.published_at, articles.published_at),
                        url = COALESCE(excluded.url, articles.url),
                        content = CASE
                            WHEN excluded.content_kind = 'full' AND excluded.content IS NOT NULL THEN excluded.content
                            WHEN articles.content_kind IN ('full', 'extracted') THEN articles.content
                            WHEN excluded.content IS NOT NULL THEN excluded.content
                            ELSE articles.content
                        END,
                        content_kind = CASE
                            WHEN excluded.content_kind = 'full' AND excluded.content IS NOT NULL THEN excluded.content_kind
                            WHEN articles.content_kind IN ('full', 'extracted') THEN articles.content_kind
                            WHEN excluded.content IS NOT NULL THEN excluded.content_kind
                            WHEN articles.content IS NULL THEN excluded.content_kind
                            ELSE articles.content_kind
                        END,
                        source = excluded.source
                "#,
            )
            .bind(&article.id)
            .bind(&article.feed_id)
            .bind(&article.title)
            .bind(&article.author)
            .bind(article.published_at)
            .bind(&article.url)
            .bind(&article.content)
            .bind(article.content_kind.as_str())
            .bind(article.source.as_str())
            .bind(entry_key)
            .execute(&mut *transaction)
            .await
            .with_context(|| format!("Impossible d'enregistrer l'article {:?}", article.id))?;
            sqlx::query(
                r#"
                    INSERT INTO sync_article_identities (
                        subscription_id, entry_key, article_id
                    ) VALUES (?, ?, ?)
                    ON CONFLICT(subscription_id, entry_key) DO UPDATE SET
                        article_id = excluded.article_id
                "#,
            )
            .bind(&sync_subscription_id)
            .bind(entry_key)
            .bind(&article.id)
            .execute(&mut *transaction)
            .await
            .with_context(|| {
                format!(
                    "Impossible d'enregistrer l'identité logique de l'article {:?}",
                    article.id
                )
            })?;

            if already_exists {
                stats.updated += 1;
            } else {
                stats.inserted += 1;
            }
        }

        transaction
            .commit()
            .await
            .context("Impossible de valider l'enregistrement des articles")?;

        Ok(stats)
    }

    /// Selects the newest extraction candidates that are due for a network attempt.
    ///
    /// Articles in their retry cooldown and due articles beyond the per-refresh
    /// limit are reported as skipped. Rows without an URL are not candidates.
    pub async fn extraction_candidates(&self, now: DateTime<Utc>) -> Result<ExtractionSelection> {
        self.extraction_candidates_scoped(now, None).await
    }

    /// Selects due extraction candidates belonging to one active feed.
    pub async fn extraction_candidates_for_feed(
        &self,
        now: DateTime<Utc>,
        feed_id: &str,
    ) -> Result<ExtractionSelection> {
        self.extraction_candidates_scoped(now, Some(feed_id)).await
    }

    async fn extraction_candidates_scoped(
        &self,
        now: DateTime<Utc>,
        feed_id: Option<&str>,
    ) -> Result<ExtractionSelection> {
        type CandidateRow = (String, String);

        let retry_cutoff = now - chrono::Duration::days(EXTRACTION_RETRY_DAYS);
        let total: i64 = sqlx::query_scalar(
            r#"
                SELECT COUNT(*)
                FROM articles
                INNER JOIN feeds ON feeds.id = articles.feed_id
                WHERE feeds.is_active = 1
                  AND feeds.platform = 'other'
                  AND articles.is_archived = 0
                  AND articles.content_kind IN ('excerpt', 'missing')
                  AND articles.url IS NOT NULL
                  AND TRIM(articles.url) <> ''
                  AND (? IS NULL OR articles.feed_id = ?)
            "#,
        )
        .bind(feed_id)
        .bind(feed_id)
        .fetch_one(&self.pool)
        .await
        .context("Impossible de compter les candidats à l'extraction")?;
        let rows: Vec<CandidateRow> = sqlx::query_as(
            r#"
                SELECT articles.id, articles.url
                FROM articles
                INNER JOIN feeds ON feeds.id = articles.feed_id
                WHERE feeds.is_active = 1
                  AND feeds.platform = 'other'
                  AND articles.is_archived = 0
                  AND articles.content_kind IN ('excerpt', 'missing')
                  AND articles.url IS NOT NULL
                  AND TRIM(articles.url) <> ''
                  AND (? IS NULL OR articles.feed_id = ?)
                  AND (
                    articles.extraction_attempted_at IS NULL
                    OR articles.extraction_attempted_url IS NULL
                    OR articles.extraction_attempted_url <> articles.url
                    OR articles.extraction_attempted_at <= ?
                  )
                ORDER BY articles.published_at IS NULL ASC,
                         articles.published_at DESC,
                         articles.id ASC
                LIMIT ?
            "#,
        )
        .bind(feed_id)
        .bind(feed_id)
        .bind(retry_cutoff)
        .bind(MAX_EXTRACTION_ATTEMPTS_PER_REFRESH as i64)
        .fetch_all(&self.pool)
        .await
        .context("Impossible de sélectionner les candidats à l'extraction")?;
        let candidates = rows
            .into_iter()
            .map(|(article_id, url)| ExtractionCandidate { article_id, url })
            .collect::<Vec<_>>();

        Ok(ExtractionSelection {
            skipped: (total as usize).saturating_sub(candidates.len()),
            candidates,
        })
    }

    /// Persists one successful extraction without touching local article state.
    pub async fn record_extraction_success(
        &self,
        article_id: &str,
        attempted_url: &str,
        html: &str,
        attempted_at: DateTime<Utc>,
    ) -> Result<bool> {
        let result = sqlx::query(
            r#"
                UPDATE articles
                SET content = ?,
                    content_kind = 'extracted',
                    extraction_attempted_at = ?,
                    extraction_attempted_url = ?,
                    extraction_attempt_count = extraction_attempt_count + 1,
                    extraction_last_error = NULL
                WHERE id = ?
                  AND is_archived = 0
                  AND content_kind IN ('excerpt', 'missing')
            "#,
        )
        .bind(html)
        .bind(attempted_at)
        .bind(attempted_url)
        .bind(article_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("Impossible d'enregistrer l'extraction de {article_id:?}"))?;

        Ok(result.rows_affected() == 1)
    }

    /// Memorizes one failed extraction while preserving the RSS fallback.
    pub async fn record_extraction_failure(
        &self,
        article_id: &str,
        attempted_url: &str,
        error: &str,
        attempted_at: DateTime<Utc>,
    ) -> Result<bool> {
        let error = error.chars().take(1_000).collect::<String>();
        let result = sqlx::query(
            r#"
                UPDATE articles
                SET extraction_attempted_at = ?,
                    extraction_attempted_url = ?,
                    extraction_attempt_count = extraction_attempt_count + 1,
                    extraction_last_error = ?
                WHERE id = ?
                  AND is_archived = 0
                  AND content_kind IN ('excerpt', 'missing')
            "#,
        )
        .bind(attempted_at)
        .bind(attempted_url)
        .bind(error)
        .bind(article_id)
        .execute(&self.pool)
        .await
        .with_context(|| {
            format!("Impossible d'enregistrer l'échec d'extraction de {article_id:?}")
        })?;

        Ok(result.rows_affected() == 1)
    }

    /// Lists all retained articles from newest to oldest, with undated entries last.
    ///
    /// # Errors
    ///
    /// Returns an error when rows cannot be loaded or contain an unknown source.
    pub async fn list_articles(&self) -> Result<Vec<StoredArticle>> {
        let rows: Vec<StoredArticleRow> = sqlx::query_as(
            r#"
                SELECT id, feed_id, title, author, published_at, url, content,
                       content_kind, source, is_read, is_favorite
                FROM articles
                WHERE is_archived = 0
                ORDER BY published_at IS NULL ASC, published_at DESC, id ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .context("Impossible de charger les articles")?;

        rows.into_iter().map(stored_article_from_row).collect()
    }

    /// Lists lightweight article summaries without loading their HTML bodies.
    ///
    /// # Errors
    ///
    /// Returns an error when rows cannot be loaded or contain an unknown source.
    pub async fn list_article_summaries(&self) -> Result<Vec<ArticleSummary>> {
        type SummaryRow = (
            String,
            String,
            Option<String>,
            Option<String>,
            Option<chrono::DateTime<chrono::Utc>>,
            Option<String>,
            String,
            bool,
            bool,
        );

        let rows: Vec<SummaryRow> = sqlx::query_as(
            r#"
                SELECT id, feed_id, title, author, published_at, url, source,
                       is_read, is_favorite
                FROM articles
                WHERE is_archived = 0
                ORDER BY published_at IS NULL ASC, published_at DESC, id ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .context("Impossible de charger les résumés d'articles")?;

        rows.into_iter()
            .map(
                |(id, feed_id, title, author, published_at, url, source, is_read, is_favorite)| {
                    let source = Source::try_from(source.as_str()).map_err(anyhow::Error::msg)?;
                    Ok(ArticleSummary {
                        id,
                        feed_id,
                        title,
                        author,
                        published_at,
                        url,
                        source,
                        is_read,
                        is_favorite,
                    })
                },
            )
            .collect()
    }

    /// Loads one complete article and its local state.
    ///
    /// # Errors
    ///
    /// Returns an error when the row cannot be loaded or contains an unknown source.
    pub async fn get_article(&self, article_id: &str) -> Result<Option<StoredArticle>> {
        let row: Option<StoredArticleRow> = sqlx::query_as(
            r#"
                SELECT id, feed_id, title, author, published_at, url, content,
                       content_kind, source, is_read, is_favorite
                FROM articles
                WHERE id = ? AND is_archived = 0
            "#,
        )
        .bind(article_id)
        .fetch_optional(&self.pool)
        .await
        .with_context(|| format!("Impossible de charger l'article {article_id:?}"))?;

        row.map(stored_article_from_row).transpose()
    }

    /// Changes the read state of an article.
    ///
    /// Returns `true` when the article exists and was targeted, even if its
    /// stored value was already identical.
    ///
    /// # Errors
    ///
    /// Returns an error when SQLite cannot execute the update.
    pub async fn set_read(&self, article_id: &str, is_read: bool) -> Result<bool> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("Impossible de démarrer la modification de l'état lu")?;
        let row: Option<SyncArticleRefRow> = sqlx::query_as(
            r#"
                UPDATE articles
                SET is_read = ?
                WHERE id = ? AND is_archived = 0
                RETURNING feed_id, COALESCE(entry_key, id), title, url, author,
                          published_at
            "#,
        )
        .bind(is_read)
        .bind(article_id)
        .fetch_optional(&mut *transaction)
        .await
        .with_context(|| format!("Impossible de modifier l'état lu de {article_id:?}"))?;
        let Some(row) = row else {
            transaction
                .rollback()
                .await
                .context("Impossible d'annuler la modification de l'état lu")?;
            return Ok(false);
        };
        if Self::sync_is_enabled_in(&mut transaction).await? {
            Self::append_local_sync_event_in(
                &mut transaction,
                &SyncEventPayload::ArticleReadSet {
                    article: sync_article_ref_from_row(row),
                    is_read,
                },
                Utc::now(),
            )
            .await?;
        }
        transaction
            .commit()
            .await
            .context("Impossible de valider la modification de l'état lu")?;
        Ok(true)
    }

    /// Changes the read state of several visible articles atomically.
    ///
    /// Duplicate identifiers are ignored. Returns `false` without changing any
    /// article when the list is empty or one identifier is missing or archived.
    pub async fn set_read_many(&self, article_ids: &[String], is_read: bool) -> Result<bool> {
        let article_ids = unique_article_ids(article_ids);
        if article_ids.is_empty() {
            return Ok(false);
        }

        let mut transaction = self
            .pool
            .begin()
            .await
            .context("Impossible de démarrer la modification groupée de l'état lu")?;
        let sync_enabled = Self::sync_is_enabled_in(&mut transaction).await?;
        let observed_at = Utc::now();
        for article_id in article_ids {
            let row: Option<SyncArticleRefRow> = sqlx::query_as(
                r#"
                    UPDATE articles
                    SET is_read = ?
                    WHERE id = ? AND is_archived = 0
                    RETURNING feed_id, COALESCE(entry_key, id), title, url,
                              author, published_at
                "#,
            )
            .bind(is_read)
            .bind(article_id)
            .fetch_optional(&mut *transaction)
            .await
            .with_context(|| format!("Impossible de modifier l'état lu de {article_id:?}"))?;
            let Some(row) = row else {
                transaction
                    .rollback()
                    .await
                    .context("Impossible d'annuler la modification groupée de l'état lu")?;
                return Ok(false);
            };
            if sync_enabled {
                Self::append_local_sync_event_in(
                    &mut transaction,
                    &SyncEventPayload::ArticleReadSet {
                        article: sync_article_ref_from_row(row),
                        is_read,
                    },
                    observed_at,
                )
                .await?;
            }
        }
        transaction
            .commit()
            .await
            .context("Impossible de valider la modification groupée de l'état lu")?;
        Ok(true)
    }

    /// Changes the favorite state of an article.
    ///
    /// Returns `true` when the article exists and was targeted, even if its
    /// stored value was already identical.
    ///
    /// # Errors
    ///
    /// Returns an error when SQLite cannot execute the update.
    pub async fn set_favorite(&self, article_id: &str, is_favorite: bool) -> Result<bool> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("Impossible de démarrer la modification du favori")?;
        let row: Option<SyncArticleRefRow> = sqlx::query_as(
            r#"
                UPDATE articles
                SET is_favorite = ?
                WHERE id = ? AND is_archived = 0
                RETURNING feed_id, COALESCE(entry_key, id), title, url, author,
                          published_at
            "#,
        )
        .bind(is_favorite)
        .bind(article_id)
        .fetch_optional(&mut *transaction)
        .await
        .with_context(|| format!("Impossible de modifier le favori {article_id:?}"))?;
        let Some(row) = row else {
            transaction
                .rollback()
                .await
                .context("Impossible d'annuler la modification du favori")?;
            return Ok(false);
        };
        if Self::sync_is_enabled_in(&mut transaction).await? {
            Self::append_local_sync_event_in(
                &mut transaction,
                &SyncEventPayload::ArticleFavoriteSet {
                    article: sync_article_ref_from_row(row),
                    is_favorite,
                },
                Utc::now(),
            )
            .await?;
        }
        transaction
            .commit()
            .await
            .context("Impossible de valider la modification du favori")?;
        Ok(true)
    }

    /// Archives one visible article and releases its cached body.
    ///
    /// Returns `false` when the article is missing or already archived.
    pub async fn archive_article(
        &self,
        article_id: &str,
        archived_at: DateTime<Utc>,
    ) -> Result<bool> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("Impossible de démarrer l'archivage de l'article")?;
        let row: Option<SyncArticleRefRow> = sqlx::query_as(
            r#"
                UPDATE articles
                SET is_archived = 1,
                    archived_at = ?,
                    archive_reason = 'manual',
                    content = NULL,
                    content_kind = 'missing'
                WHERE id = ? AND is_archived = 0
                RETURNING feed_id, COALESCE(entry_key, id), title, url, author,
                          published_at
            "#,
        )
        .bind(archived_at)
        .bind(article_id)
        .fetch_optional(&mut *transaction)
        .await
        .with_context(|| format!("Impossible d'archiver l'article {article_id:?}"))?;
        let Some(row) = row else {
            transaction
                .rollback()
                .await
                .context("Impossible d'annuler l'archivage de l'article")?;
            return Ok(false);
        };
        if Self::sync_is_enabled_in(&mut transaction).await? {
            let article = sync_article_ref_from_row(row);
            Self::append_local_sync_event_in(
                &mut transaction,
                &SyncEventPayload::ArticleArchived { article },
                archived_at,
            )
            .await?;
        }
        transaction
            .commit()
            .await
            .context("Impossible de valider l'archivage de l'article")?;
        Ok(true)
    }

    /// Archives one article using the current UTC time.
    pub async fn archive_article_now(&self, article_id: &str) -> Result<bool> {
        self.archive_article(article_id, Utc::now()).await
    }

    /// Archives several visible articles atomically using one timestamp.
    ///
    /// Duplicate identifiers are ignored. Returns `false` without changing any
    /// article when the list is empty or one identifier is missing or archived.
    pub async fn archive_articles_now(&self, article_ids: &[String]) -> Result<bool> {
        let article_ids = unique_article_ids(article_ids);
        if article_ids.is_empty() {
            return Ok(false);
        }

        let archived_at = Utc::now();
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("Impossible de démarrer l'archivage groupé")?;
        let sync_enabled = Self::sync_is_enabled_in(&mut transaction).await?;
        for article_id in article_ids {
            let row: Option<SyncArticleRefRow> = sqlx::query_as(
                r#"
                    UPDATE articles
                    SET is_archived = 1,
                        archived_at = ?,
                        archive_reason = 'manual',
                        content = NULL,
                        content_kind = 'missing'
                    WHERE id = ? AND is_archived = 0
                    RETURNING feed_id, COALESCE(entry_key, id), title, url,
                              author, published_at
                "#,
            )
            .bind(archived_at)
            .bind(article_id)
            .fetch_optional(&mut *transaction)
            .await
            .with_context(|| format!("Impossible d'archiver l'article {article_id:?}"))?;
            let Some(row) = row else {
                transaction
                    .rollback()
                    .await
                    .context("Impossible d'annuler l'archivage groupé")?;
                return Ok(false);
            };
            if sync_enabled {
                let article = sync_article_ref_from_row(row);
                Self::append_local_sync_event_in(
                    &mut transaction,
                    &SyncEventPayload::ArticleArchived { article },
                    archived_at,
                )
                .await?;
            }
        }
        transaction
            .commit()
            .await
            .context("Impossible de valider l'archivage groupé")?;
        Ok(true)
    }

    /// Archives old read articles that are not favorites and releases their bodies.
    pub async fn archive_expired_read_articles(&self, now: DateTime<Utc>) -> Result<usize> {
        self.archive_expired_read_articles_scoped(now, None).await
    }

    /// Archives expired read articles belonging to one subscription only.
    pub async fn archive_expired_read_articles_for_feed(
        &self,
        now: DateTime<Utc>,
        feed_id: &str,
    ) -> Result<usize> {
        self.archive_expired_read_articles_scoped(now, Some(feed_id))
            .await
    }

    async fn archive_expired_read_articles_scoped(
        &self,
        now: DateTime<Utc>,
        feed_id: Option<&str>,
    ) -> Result<usize> {
        let cutoff = now - chrono::Duration::days(ARTICLE_RETENTION_DAYS);
        let result = sqlx::query(
            r#"
                UPDATE articles
                SET is_archived = 1,
                    archived_at = ?,
                    archive_reason = 'retention',
                    content = NULL,
                    content_kind = 'missing'
                WHERE is_archived = 0
                  AND is_read = 1
                  AND is_favorite = 0
                  AND published_at IS NOT NULL
                  AND published_at < ?
                  AND (? IS NULL OR feed_id = ?)
            "#,
        )
        .bind(now)
        .bind(cutoff)
        .bind(feed_id)
        .bind(feed_id)
        .execute(&self.pool)
        .await
        .context("Impossible d'archiver les anciens articles lus")?;

        Ok(result.rows_affected() as usize)
    }

    /// Applies the fixed retention policy using the current UTC time.
    pub async fn apply_article_retention(&self) -> Result<usize> {
        self.archive_expired_read_articles(Utc::now()).await
    }

    #[cfg(test)]
    pub(crate) async fn open_in_memory() -> Result<Self> {
        let options = SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);

        Self::connect(options, 1).await
    }

    /// Closes every pooled connection and waits for in-flight operations.
    pub async fn close(self) {
        self.pool.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn feed(id: &str, platform: Platform, url: &str) -> FeedConfig {
        FeedConfig {
            id: id.to_string(),
            platform,
            url: url.to_string(),
        }
    }

    fn stored_feed(id: &str, platform: Platform, url: &str, is_active: bool) -> StoredFeed {
        StoredFeed {
            id: id.to_string(),
            platform,
            url: url.to_string(),
            is_active,
            title: None,
            description: None,
            author: None,
            last_published_at: None,
            last_success_at: None,
            last_error: None,
            logo_png: None,
        }
    }

    fn article(id: &str, feed_id: &str, published_at: Option<chrono::DateTime<Utc>>) -> Article {
        Article {
            id: id.to_string(),
            feed_id: feed_id.to_string(),
            title: Some(format!("Title for {id}")),
            author: Some("Test Author".to_string()),
            published_at,
            url: Some(format!("https://articles.example/{id}")),
            content: Some(format!("Readable content for {id}")),
            content_kind: ContentKind::Full,
            source: Source::Substack,
        }
    }

    fn sync_article_ref(id: &str) -> SyncArticleRef {
        SyncArticleRef {
            subscription_id: "astronomy".to_string(),
            entry_key: id.to_string(),
            title: Some(format!("Title for {id}")),
            url: Some(format!("https://articles.example/{id}")),
            author: Some("Test Author".to_string()),
            published_at: None,
        }
    }

    fn remote_event(
        device_id: &str,
        sequence: i64,
        physical_milliseconds: i64,
        payload: SyncEventPayload,
    ) -> SyncEvent {
        SyncEvent {
            device_id: device_id.to_string(),
            sequence,
            clock: HybridLogicalClock {
                physical_milliseconds,
                logical_counter: 0,
            },
            protocol_version: SYNC_PROTOCOL_VERSION,
            kind: payload.kind().to_string(),
            payload,
        }
    }

    fn remote_subscription_created(
        device_id: &str,
        sequence: i64,
        physical_milliseconds: i64,
        subscription_id: &str,
    ) -> SyncEvent {
        remote_event(
            device_id,
            sequence,
            physical_milliseconds,
            SyncEventPayload::SubscriptionCreated {
                subscription_id: subscription_id.to_string(),
                normalized_url: "https://sync.example/feed".to_string(),
                platform_hint: Platform::Other,
                is_active: true,
                parent_tombstone: None,
            },
        )
    }

    fn event_permutations(events: &[SyncEvent]) -> Vec<Vec<SyncEvent>> {
        fn visit(
            events: &[SyncEvent],
            used: &mut [bool],
            current: &mut Vec<SyncEvent>,
            result: &mut Vec<Vec<SyncEvent>>,
        ) {
            if current.len() == events.len() {
                result.push(current.clone());
                return;
            }
            for index in 0..events.len() {
                if !used[index] {
                    used[index] = true;
                    current.push(events[index].clone());
                    visit(events, used, current, result);
                    current.pop();
                    used[index] = false;
                }
            }
        }

        let mut result = Vec::new();
        visit(
            events,
            &mut vec![false; events.len()],
            &mut Vec::new(),
            &mut result,
        );
        result
    }

    async fn storage_with_feed() -> Storage {
        let storage = Storage::open_in_memory().await.unwrap();
        storage
            .import_feeds(&[feed(
                "astronomy",
                Platform::Substack,
                "https://astronomy.example/feed",
            )])
            .await
            .unwrap();
        storage
    }

    #[tokio::test]
    async fn sync_pairing_metadata_supports_rename_and_logical_revocation() {
        let storage = Storage::open_in_memory().await.unwrap();
        let observed_at = DateTime::parse_from_rfc3339("2026-08-28T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let local_id = storage.sync_identity().await.unwrap().device_id;
        let remote_id = "00000000-0000-4000-8000-000000000088";
        let configuration = SyncConfiguration {
            webdav_base_url: "https://dav.example.test/inkriver/".to_string(),
            webdav_username: "romain".to_string(),
            key_id: "42".repeat(32),
        };

        storage
            .save_sync_configuration(&configuration)
            .await
            .unwrap();
        assert_eq!(
            storage.sync_configuration().await.unwrap(),
            Some(configuration)
        );
        assert!(
            storage
                .rename_sync_device(&local_id, "Laptop Linux", observed_at)
                .await
                .unwrap()
        );
        storage
            .register_sync_device(remote_id, "Téléphone", observed_at)
            .await
            .unwrap();
        assert!(
            storage
                .rename_sync_device(remote_id, "Pixel", observed_at)
                .await
                .unwrap()
        );
        assert!(!storage.sync_device_is_revoked(remote_id).await.unwrap());
        assert!(
            storage
                .revoke_sync_device(remote_id, observed_at)
                .await
                .unwrap()
        );
        assert!(storage.sync_device_is_revoked(remote_id).await.unwrap());
        assert!(
            !storage
                .revoke_sync_device(remote_id, observed_at)
                .await
                .unwrap()
        );
        assert!(
            !storage
                .revoke_sync_device(&local_id, observed_at)
                .await
                .unwrap()
        );
        let devices = storage.list_sync_devices().await.unwrap();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].display_name, "Laptop Linux");
        assert_eq!(devices[1].display_name, "Pixel");
        assert!(devices[1].revoked_at.is_some());

        storage.clear_sync_configuration().await.unwrap();
        assert_eq!(storage.sync_configuration().await.unwrap(), None);
    }

    #[tokio::test]
    async fn sync_pairing_metadata_rejects_invalid_values() {
        let storage = Storage::open_in_memory().await.unwrap();
        let invalid_config = SyncConfiguration {
            webdav_base_url: "https://user:secret@example.test/dav/".to_string(),
            webdav_username: "user".to_string(),
            key_id: "42".repeat(32),
        };
        assert!(
            storage
                .save_sync_configuration(&invalid_config)
                .await
                .is_err()
        );
        assert!(
            storage
                .register_sync_device("not-a-uuid", "Phone", Utc::now())
                .await
                .is_err()
        );
        assert!(
            storage
                .rename_sync_device(
                    &storage.sync_identity().await.unwrap().device_id,
                    "  ",
                    Utc::now(),
                )
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn sync_acknowledgements_are_monotonic_and_round_trip() {
        let storage = Storage::open_in_memory().await.unwrap();
        let key_id = "42".repeat(32);
        let observer = "00000000-0000-4000-8000-000000000111";
        let source = "00000000-0000-4000-8000-000000000222";
        let first_at = Utc.with_ymd_and_hms(2026, 8, 28, 10, 0, 0).unwrap();
        let newer_at = Utc.with_ymd_and_hms(2026, 8, 28, 10, 5, 0).unwrap();

        assert!(
            storage
                .record_sync_acknowledgement(&SyncAcknowledgement {
                    key_id: key_id.clone(),
                    observer_device_id: observer.to_string(),
                    source_device_id: source.to_string(),
                    contiguous_sequence: 8,
                    observed_at: first_at,
                })
                .await
                .unwrap()
        );
        assert!(
            !storage
                .record_sync_acknowledgement(&SyncAcknowledgement {
                    key_id: key_id.clone(),
                    observer_device_id: observer.to_string(),
                    source_device_id: source.to_string(),
                    contiguous_sequence: 7,
                    observed_at: newer_at,
                })
                .await
                .unwrap()
        );
        assert!(
            storage
                .record_sync_acknowledgement(&SyncAcknowledgement {
                    key_id: key_id.clone(),
                    observer_device_id: observer.to_string(),
                    source_device_id: source.to_string(),
                    contiguous_sequence: 8,
                    observed_at: newer_at,
                })
                .await
                .unwrap()
        );
        let other_key_id = "24".repeat(32);
        assert!(
            storage
                .record_sync_acknowledgement(&SyncAcknowledgement {
                    key_id: other_key_id.clone(),
                    observer_device_id: observer.to_string(),
                    source_device_id: source.to_string(),
                    contiguous_sequence: 99,
                    observed_at: newer_at,
                })
                .await
                .unwrap()
        );

        assert_eq!(
            storage
                .sync_acknowledgements_for_source(&key_id, source)
                .await
                .unwrap(),
            vec![SyncAcknowledgement {
                key_id: key_id.clone(),
                observer_device_id: observer.to_string(),
                source_device_id: source.to_string(),
                contiguous_sequence: 8,
                observed_at: newer_at,
            }]
        );
        assert_eq!(
            storage
                .sync_acknowledgements_for_source(&other_key_id, source)
                .await
                .unwrap()[0]
                .contiguous_sequence,
            99
        );
    }

    #[tokio::test]
    async fn acknowledgement_documents_are_recorded_atomically() {
        let storage = Storage::open_in_memory().await.unwrap();
        let key_id = "42".repeat(32);
        let observer = "00000000-0000-4000-8000-000000000111";
        let valid_source = "00000000-0000-4000-8000-000000000222";
        let observed_at = Utc.with_ymd_and_hms(2026, 8, 29, 10, 0, 0).unwrap();
        let acknowledgements = vec![
            SyncAcknowledgement {
                key_id: key_id.clone(),
                observer_device_id: observer.to_string(),
                source_device_id: valid_source.to_string(),
                contiguous_sequence: 4,
                observed_at,
            },
            SyncAcknowledgement {
                key_id: key_id.clone(),
                observer_device_id: observer.to_string(),
                source_device_id: "invalid".to_string(),
                contiguous_sequence: 2,
                observed_at,
            },
        ];

        assert!(
            storage
                .record_sync_acknowledgements(&acknowledgements)
                .await
                .is_err()
        );
        assert!(
            storage
                .sync_acknowledgements_for_source(&key_id, valid_source)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn compaction_frontier_requires_every_explicit_observer() {
        let storage = Storage::open_in_memory().await.unwrap();
        let key_id = "42".repeat(32);
        storage.enable_sync().await.unwrap();
        let feed = storage
            .add_feed("https://compaction.example/feed", None)
            .await
            .unwrap();
        storage.set_feed_active(&feed.id, false).await.unwrap();
        let local_id = storage.sync_identity().await.unwrap().device_id;
        let source_max_sequence = storage.sync_identity().await.unwrap().next_sequence - 1;
        assert!(source_max_sequence >= 2);
        let first_remote = "00000000-0000-4000-8000-000000000111".to_string();
        let second_remote = "00000000-0000-4000-8000-000000000222".to_string();
        let required = vec![
            second_remote.clone(),
            local_id.clone(),
            first_remote.clone(),
            second_remote.clone(),
        ];

        let blocked = storage
            .sync_compaction_frontier(&key_id, &local_id, source_max_sequence, &required)
            .await
            .unwrap();
        assert_eq!(blocked.safe_through_sequence, 0);
        assert_eq!(blocked.required_observer_count, 3);
        assert_eq!(
            blocked.blocking_observer_device_ids,
            vec![first_remote.clone(), second_remote.clone()]
        );

        let observed_at = Utc.with_ymd_and_hms(2026, 8, 28, 11, 0, 0).unwrap();
        storage
            .record_sync_acknowledgement(&SyncAcknowledgement {
                key_id: key_id.clone(),
                observer_device_id: first_remote.clone(),
                source_device_id: local_id.clone(),
                contiguous_sequence: 1,
                observed_at,
            })
            .await
            .unwrap();
        storage
            .record_sync_acknowledgement(&SyncAcknowledgement {
                key_id: key_id.clone(),
                observer_device_id: second_remote.clone(),
                source_device_id: local_id.clone(),
                contiguous_sequence: source_max_sequence + 100,
                observed_at,
            })
            .await
            .unwrap();

        let partial = storage
            .sync_compaction_frontier(&key_id, &local_id, source_max_sequence, &required)
            .await
            .unwrap();
        assert_eq!(partial.safe_through_sequence, 1);
        assert_eq!(partial.blocking_observer_device_ids, vec![first_remote]);
    }

    #[tokio::test]
    async fn authoritative_frontier_uses_active_roster_and_excludes_revoked_members() {
        let storage = Storage::open_in_memory().await.unwrap();
        storage.enable_sync().await.unwrap();
        storage
            .add_feed("https://frontier.example/feed", None)
            .await
            .unwrap();
        let local_id = storage.sync_identity().await.unwrap().device_id;
        let active_remote = "00000000-0000-4000-8000-000000000061";
        let revoked_remote = "00000000-0000-4000-8000-000000000062";
        let key_id = "ef".repeat(32);
        let observed_at = Utc.with_ymd_and_hms(2026, 8, 29, 15, 0, 0).unwrap();
        storage
            .merge_sync_roster(
                &key_id,
                &[
                    SyncRosterMember {
                        device_id: local_id.clone(),
                        revoked_at: None,
                    },
                    SyncRosterMember {
                        device_id: active_remote.to_string(),
                        revoked_at: None,
                    },
                    SyncRosterMember {
                        device_id: revoked_remote.to_string(),
                        revoked_at: Some(observed_at),
                    },
                ],
                observed_at,
            )
            .await
            .unwrap();

        let blocked = storage
            .authoritative_sync_compaction_frontier(&key_id, &local_id, 1)
            .await
            .unwrap();
        assert_eq!(blocked.safe_through_sequence, 0);
        assert_eq!(blocked.required_observer_count, 2);
        assert_eq!(
            blocked.blocking_observer_device_ids,
            vec![active_remote.to_string()]
        );

        storage
            .record_sync_acknowledgement(&SyncAcknowledgement {
                key_id: key_id.clone(),
                observer_device_id: active_remote.to_string(),
                source_device_id: local_id.clone(),
                contiguous_sequence: 1,
                observed_at,
            })
            .await
            .unwrap();
        let ready = storage
            .authoritative_sync_compaction_frontier(&key_id, &local_id, 1)
            .await
            .unwrap();
        assert_eq!(ready.safe_through_sequence, 1);
        assert!(ready.blocking_observer_device_ids.is_empty());
    }

    #[tokio::test]
    async fn local_compaction_is_bounded_idempotent_and_preserves_checkpoint_state() {
        let storage = Storage::open_in_memory().await.unwrap();
        storage.enable_sync().await.unwrap();
        let feed = storage
            .add_feed("https://local-compaction.example/feed", None)
            .await
            .unwrap();
        for index in 0..25 {
            storage
                .set_feed_active(&feed.id, index % 2 == 0)
                .await
                .unwrap();
        }
        let key_id = "ac".repeat(32);
        let observed_at = Utc.with_ymd_and_hms(2026, 8, 29, 16, 0, 0).unwrap();
        storage
            .seed_sync_roster(&key_id, observed_at)
            .await
            .unwrap();
        let checkpoint_before = storage
            .sync_snapshot_material(MAX_SYNC_EVENTS_PER_READ)
            .await
            .unwrap()
            .unwrap();
        let feeds_before = storage.list_feeds().await.unwrap();

        assert_eq!(
            storage
                .compact_sync_events(&key_id, &checkpoint_before.0, 7)
                .await
                .unwrap(),
            7
        );
        assert_eq!(
            storage
                .compact_sync_events(&key_id, &checkpoint_before.0, MAX_SYNC_EVENTS_PER_READ,)
                .await
                .unwrap(),
            17
        );
        assert_eq!(
            storage
                .compact_sync_events(&key_id, &checkpoint_before.0, MAX_SYNC_EVENTS_PER_READ,)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            storage
                .sync_snapshot_material(MAX_SYNC_EVENTS_PER_READ)
                .await
                .unwrap()
                .unwrap(),
            checkpoint_before
        );
        assert_eq!(storage.list_feeds().await.unwrap(), feeds_before);
        assert_eq!(
            storage.local_sync_events_after(0, 100).await.unwrap().len(),
            2
        );
    }

    #[tokio::test]
    async fn local_compaction_waits_for_every_active_roster_member() {
        let storage = Storage::open_in_memory().await.unwrap();
        storage.enable_sync().await.unwrap();
        let feed = storage
            .add_feed("https://blocked-compaction.example/feed", None)
            .await
            .unwrap();
        storage.set_feed_active(&feed.id, false).await.unwrap();
        storage.set_feed_active(&feed.id, true).await.unwrap();
        let local = storage.sync_identity().await.unwrap();
        let remote = "00000000-0000-4000-8000-000000000063";
        let key_id = "bd".repeat(32);
        let observed_at = Utc.with_ymd_and_hms(2026, 8, 29, 16, 30, 0).unwrap();
        storage
            .merge_sync_roster(
                &key_id,
                &[
                    SyncRosterMember {
                        device_id: local.device_id.clone(),
                        revoked_at: None,
                    },
                    SyncRosterMember {
                        device_id: remote.to_string(),
                        revoked_at: None,
                    },
                ],
                observed_at,
            )
            .await
            .unwrap();

        let checkpoint_frontiers = vec![(local.device_id.clone(), local.next_sequence - 1)];
        assert_eq!(
            storage
                .compact_sync_events(&key_id, &checkpoint_frontiers, 100)
                .await
                .unwrap(),
            0
        );
        storage
            .record_sync_acknowledgement(&SyncAcknowledgement {
                key_id: key_id.clone(),
                observer_device_id: remote.to_string(),
                source_device_id: local.device_id,
                contiguous_sequence: local.next_sequence - 1,
                observed_at,
            })
            .await
            .unwrap();
        assert_eq!(
            storage
                .compact_sync_events(&key_id, &checkpoint_frontiers, 100)
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn synchronization_roster_is_additive_and_revocation_never_regresses() {
        let storage = Storage::open_in_memory().await.unwrap();
        let key_id = "cd".repeat(32);
        let first = "00000000-0000-4000-8000-000000000041";
        let second = "00000000-0000-4000-8000-000000000042";
        let revoked_at = Utc.with_ymd_and_hms(2026, 8, 29, 14, 0, 0).unwrap();

        storage
            .merge_sync_roster(
                &key_id,
                &[
                    SyncRosterMember {
                        device_id: first.to_string(),
                        revoked_at: None,
                    },
                    SyncRosterMember {
                        device_id: second.to_string(),
                        revoked_at: Some(revoked_at),
                    },
                ],
                revoked_at,
            )
            .await
            .unwrap();
        storage
            .merge_sync_roster(
                &key_id,
                &[SyncRosterMember {
                    device_id: second.to_string(),
                    revoked_at: None,
                }],
                revoked_at - chrono::Duration::days(1),
            )
            .await
            .unwrap();

        let members = storage.sync_roster_members(&key_id).await.unwrap();
        assert_eq!(members.len(), 2);
        assert_eq!(members[1].revoked_at, Some(revoked_at));
        assert_eq!(
            storage
                .active_sync_roster_device_ids(&key_id)
                .await
                .unwrap(),
            vec![first]
        );
    }

    #[tokio::test]
    async fn synchronization_roster_merge_is_atomic_on_invalid_input() {
        let storage = Storage::open_in_memory().await.unwrap();
        let key_id = "de".repeat(32);
        let valid = "00000000-0000-4000-8000-000000000051";
        let result = storage
            .merge_sync_roster(
                &key_id,
                &[
                    SyncRosterMember {
                        device_id: valid.to_string(),
                        revoked_at: None,
                    },
                    SyncRosterMember {
                        device_id: "invalid".to_string(),
                        revoked_at: None,
                    },
                ],
                Utc::now(),
            )
            .await;

        assert!(result.is_err());
        assert!(
            storage
                .sync_roster_members(&key_id)
                .await
                .unwrap()
                .is_empty()
        );
    }

    /// Verifies a file database is created with migrations and foreign keys enabled.
    #[tokio::test]
    async fn open_creates_database_and_applies_migrations() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("inkriver.db");

        let storage = Storage::open(&database_path).await.unwrap();

        assert!(database_path.is_file());

        let table_names: Vec<String> =
            sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
                .fetch_all(&storage.pool)
                .await
                .unwrap();
        assert!(table_names.contains(&"feeds".to_string()));
        assert!(table_names.contains(&"articles".to_string()));
        assert!(table_names.contains(&"sync_local_state".to_string()));
        assert!(table_names.contains(&"sync_events".to_string()));
        assert!(table_names.contains(&"sync_pending_events".to_string()));
        assert!(table_names.contains(&"sync_acknowledgements".to_string()));
        assert!(table_names.contains(&"sync_snapshot_publications".to_string()));
        assert!(table_names.contains(&"sync_snapshot_imports".to_string()));
        assert!(table_names.contains(&"sync_roster_members".to_string()));

        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&storage.pool)
            .await
            .unwrap();
        assert_eq!(foreign_keys, 1);

        storage.close().await;
    }

    /// Verifies the test-only in-memory constructor applies the same migrations.
    #[tokio::test]
    async fn open_in_memory_applies_migrations() {
        let storage = Storage::open_in_memory().await.unwrap();

        let migration_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
            .fetch_one(&storage.pool)
            .await
            .unwrap();

        assert_eq!(migration_count, 17);

        storage.close().await;
    }

    /// Verifies that one generated device identity and its journal clock survive
    /// closing and reopening the same SQLite database.
    #[tokio::test]
    async fn synchronization_identity_and_clock_survive_restart() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("inkriver.db");
        let storage = Storage::open(&database_path).await.unwrap();
        let initial = storage.sync_identity().await.unwrap();
        assert!(uuid::Uuid::parse_str(&initial.device_id).is_ok());
        assert_eq!(initial.next_sequence, 1);
        assert!(!initial.is_enabled);
        assert_eq!(
            initial.clock,
            HybridLogicalClock {
                physical_milliseconds: 0,
                logical_counter: 0,
            }
        );

        let first_time = Utc.timestamp_millis_opt(2_000).single().unwrap();
        let first = storage
            .append_local_sync_event(
                &SyncEventPayload::SubscriptionActiveSet {
                    subscription_id: "feed".to_string(),
                    is_active: true,
                },
                first_time,
            )
            .await
            .unwrap();
        storage.close().await;

        let storage = Storage::open(&database_path).await.unwrap();
        let reopened = storage.sync_identity().await.unwrap();
        assert_eq!(reopened.device_id, initial.device_id);
        assert_eq!(reopened.next_sequence, 2);
        assert_eq!(reopened.clock, first.clock);

        let older_wall_time = Utc.timestamp_millis_opt(1_000).single().unwrap();
        let second = storage
            .append_local_sync_event(
                &SyncEventPayload::SubscriptionActiveSet {
                    subscription_id: "feed".to_string(),
                    is_active: false,
                },
                older_wall_time,
            )
            .await
            .unwrap();
        assert_eq!(second.sequence, 2);
        assert_eq!(second.clock.physical_milliseconds, 2_000);
        assert_eq!(second.clock.logical_counter, 1);
    }

    /// Verifies ordered, bounded journal reads and hybrid-clock progression.
    #[tokio::test]
    async fn local_sync_journal_allocates_unique_sequences_and_reads_after_cursor() {
        let storage = Storage::open_in_memory().await.unwrap();
        let times = [1_000, 900, 1_000, 1_100];
        let expected_clocks = [(1_000, 0), (1_000, 1), (1_000, 2), (1_100, 0)];

        for (index, time) in times.into_iter().enumerate() {
            let event = storage
                .append_local_sync_event(
                    &SyncEventPayload::ArticleReadSet {
                        article: sync_article_ref(&index.to_string()),
                        is_read: index % 2 == 0,
                    },
                    Utc.timestamp_millis_opt(time).single().unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(event.sequence, index as i64 + 1);
            assert_eq!(
                (
                    event.clock.physical_milliseconds,
                    event.clock.logical_counter
                ),
                expected_clocks[index]
            );
            assert_eq!(event.protocol_version, SYNC_PROTOCOL_VERSION);
        }

        let page = storage.local_sync_events_after(1, 2).await.unwrap();
        assert_eq!(
            page.iter().map(|event| event.sequence).collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert_eq!(
            page[0].payload,
            SyncEventPayload::ArticleReadSet {
                article: sync_article_ref("1"),
                is_read: false,
            }
        );
        assert!(
            storage
                .local_sync_events_after(0, 0)
                .await
                .unwrap()
                .is_empty()
        );

        let duplicate = sqlx::query(
            r#"
                INSERT INTO sync_events (
                    device_id, sequence, hlc_physical_ms, hlc_counter,
                    protocol_version, event_kind, payload_json
                ) VALUES (?, 1, 0, 0, 1, 'duplicate', '{}')
            "#,
        )
        .bind(&page[0].device_id)
        .execute(&storage.pool)
        .await;
        assert!(duplicate.is_err());
    }

    /// Verifies concurrent writers cannot allocate the same local sequence.
    #[tokio::test]
    async fn concurrent_sync_appends_allocate_unique_contiguous_sequences() {
        let directory = tempfile::tempdir().unwrap();
        let storage = Storage::open(&directory.path().join("inkriver.db"))
            .await
            .unwrap();
        let observed_at = Utc.timestamp_millis_opt(10_000).single().unwrap();
        let appends = (0..16).map(|index| {
            let payload = SyncEventPayload::SubscriptionActiveSet {
                subscription_id: format!("feed-{index}"),
                is_active: true,
            };
            let storage = &storage;
            async move { storage.append_local_sync_event(&payload, observed_at).await }
        });

        let mut sequences = futures_util::future::join_all(appends)
            .await
            .into_iter()
            .map(Result::unwrap)
            .map(|event| event.sequence)
            .collect::<Vec<_>>();
        sequences.sort_unstable();
        assert_eq!(sequences, (1..=16).collect::<Vec<_>>());
        assert_eq!(storage.sync_identity().await.unwrap().next_sequence, 17);
    }

    /// Verifies that a failed journal insert rolls back both sequence and clock.
    #[tokio::test]
    async fn failed_sync_append_does_not_consume_sequence_or_clock() {
        let storage = Storage::open_in_memory().await.unwrap();
        sqlx::raw_sql(
            r#"
                CREATE TRIGGER reject_test_sync_event
                BEFORE INSERT ON sync_events
                WHEN NEW.event_kind = 'subscription_deleted'
                BEGIN
                    SELECT RAISE(ABORT, 'rejected by test');
                END;
            "#,
        )
        .execute(&storage.pool)
        .await
        .unwrap();

        let time = Utc.timestamp_millis_opt(5_000).single().unwrap();
        assert!(
            storage
                .append_local_sync_event(
                    &SyncEventPayload::SubscriptionDeleted {
                        subscription_id: "feed".to_string(),
                    },
                    time,
                )
                .await
                .is_err()
        );
        let after_failure = storage.sync_identity().await.unwrap();
        assert_eq!(after_failure.next_sequence, 1);
        assert_eq!(after_failure.clock.physical_milliseconds, 0);
        assert_eq!(after_failure.clock.logical_counter, 0);

        let accepted = storage
            .append_local_sync_event(
                &SyncEventPayload::SubscriptionActiveSet {
                    subscription_id: "feed".to_string(),
                    is_active: true,
                },
                time,
            )
            .await
            .unwrap();
        assert_eq!(accepted.sequence, 1);
        assert_eq!(accepted.clock.physical_milliseconds, 5_000);
        assert_eq!(accepted.clock.logical_counter, 0);
    }

    /// Verifies a corrupted discriminator cannot be silently deserialized.
    #[tokio::test]
    async fn sync_journal_reader_rejects_a_kind_payload_mismatch() {
        let storage = Storage::open_in_memory().await.unwrap();
        let identity = storage.sync_identity().await.unwrap();
        let payload = serde_json::to_string(&SyncEventPayload::SubscriptionDeleted {
            subscription_id: "feed".to_string(),
        })
        .unwrap();
        sqlx::query(
            r#"
                INSERT INTO sync_events (
                    device_id, sequence, hlc_physical_ms, hlc_counter,
                    protocol_version, event_kind, payload_json
                ) VALUES (?, 1, 1000, 0, 1, 'subscription_active_set', ?)
            "#,
        )
        .bind(identity.device_id)
        .bind(payload)
        .execute(&storage.pool)
        .await
        .unwrap();

        assert!(storage.local_sync_events_after(0, 10).await.is_err());
    }

    /// Verifies existing behavior remains unchanged and silent before a sync
    /// configuration explicitly enables journaling.
    #[tokio::test]
    async fn business_mutations_do_not_journal_before_sync_is_enabled() {
        let storage = Storage::open_in_memory().await.unwrap();
        let feed = storage
            .add_feed("https://example.com/feed", None)
            .await
            .unwrap();
        storage
            .upsert_articles(&[article("article", &feed.id, None)])
            .await
            .unwrap();
        storage.set_read("article", true).await.unwrap();
        storage.set_favorite("article", true).await.unwrap();
        storage.archive_article_now("article").await.unwrap();
        storage.set_feed_active(&feed.id, false).await.unwrap();

        assert!(
            storage
                .local_sync_events_after(0, 100)
                .await
                .unwrap()
                .is_empty()
        );
        let identity = storage.sync_identity().await.unwrap();
        assert_eq!(identity.next_sequence, 1);
        assert!(!identity.is_enabled);
    }

    /// Verifies enabling sync snapshots retained feeds and article state once,
    /// excluding automatic-retention tombstones.
    #[tokio::test]
    async fn enabling_sync_bootstraps_existing_state_once() {
        let storage = storage_with_feed().await;
        let now = Utc.with_ymd_and_hms(2026, 8, 26, 12, 0, 0).unwrap();
        let old_date = now - chrono::Duration::days(60);
        storage
            .upsert_articles(&[
                article("astronomy::manual", "astronomy", Some(old_date)),
                article("astronomy::retention", "astronomy", Some(old_date)),
            ])
            .await
            .unwrap();
        storage.set_read("astronomy::manual", true).await.unwrap();
        storage
            .set_favorite("astronomy::manual", true)
            .await
            .unwrap();
        storage
            .archive_article("astronomy::manual", now)
            .await
            .unwrap();
        storage
            .set_read("astronomy::retention", true)
            .await
            .unwrap();
        assert_eq!(storage.archive_expired_read_articles(now).await.unwrap(), 1);

        assert!(storage.enable_sync().await.unwrap());
        assert!(!storage.enable_sync().await.unwrap());
        let events = storage.local_sync_events_after(0, 100).await.unwrap();
        assert_eq!(
            events
                .iter()
                .map(|event| event.kind.as_str())
                .collect::<Vec<_>>(),
            vec![
                "subscription_created",
                "article_read_set",
                "article_favorite_set",
                "article_archived",
                "article_read_set",
                "article_favorite_set",
            ]
        );
        assert!(matches!(
            &events[0].payload,
            SyncEventPayload::SubscriptionCreated {
                subscription_id,
                normalized_url,
                platform_hint: Platform::Substack,
                is_active: true,
                parent_tombstone: None,
            } if subscription_id == "astronomy"
                && normalized_url == "https://astronomy.example/feed"
        ));
        assert!(matches!(
            &events[3].payload,
            SyncEventPayload::ArticleArchived { article }
                if article.entry_key == "manual" && article.subscription_id == "astronomy"
        ));
        assert!(!events.iter().any(|event| {
            matches!(
                &event.payload,
                SyncEventPayload::ArticleArchived { article }
                    if article.entry_key == "retention"
            )
        }));
        let identity = storage.sync_identity().await.unwrap();
        assert!(identity.is_enabled);
        assert_eq!(identity.next_sequence, 7);
        let aliases: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_subscription_aliases")
            .fetch_one(&storage.pool)
            .await
            .unwrap();
        let article_identities: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sync_article_identities")
                .fetch_one(&storage.pool)
                .await
                .unwrap();
        assert_eq!((aliases, article_identities), (1, 2));
    }

    /// Verifies subscription lifecycle events, including explicit re-creation
    /// after a synchronized deletion tombstone.
    #[tokio::test]
    async fn subscription_mutations_produce_typed_events_and_parent_tombstones() {
        let storage = Storage::open_in_memory().await.unwrap();
        storage.enable_sync().await.unwrap();

        let original = storage
            .add_feed("https://example.com/feed", None)
            .await
            .unwrap();
        storage.set_feed_active(&original.id, false).await.unwrap();
        let reactivated = storage
            .add_feed("https://example.com/feed", Some(Platform::Medium))
            .await
            .unwrap();
        assert_eq!(reactivated.id, original.id);
        storage.delete_feed(&original.id).await.unwrap();
        let replacement = storage
            .add_feed("https://example.com/feed", None)
            .await
            .unwrap();
        assert_ne!(replacement.id, original.id);

        let events = storage.local_sync_events_after(0, 100).await.unwrap();
        assert_eq!(
            events
                .iter()
                .map(|event| event.kind.as_str())
                .collect::<Vec<_>>(),
            vec![
                "subscription_created",
                "subscription_active_set",
                "subscription_platform_set",
                "subscription_active_set",
                "subscription_deleted",
                "subscription_created",
            ]
        );
        let deletion = &events[4];
        assert!(matches!(
            &events[5].payload,
            SyncEventPayload::SubscriptionCreated {
                subscription_id,
                parent_tombstone: Some(parent),
                ..
            } if subscription_id == &replacement.id
                && parent.device_id == deletion.device_id
                && parent.sequence == deletion.sequence
        ));
        let tombstones: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sync_tombstones WHERE entity_kind = 'subscription'",
        )
        .fetch_one(&storage.pool)
        .await
        .unwrap();
        assert_eq!(tombstones, 1);
    }

    /// Verifies explicit article mutations emit one event per logical article,
    /// while RSS upserts remain local cache operations.
    #[tokio::test]
    async fn article_mutations_produce_typed_events_but_upserts_do_not() {
        let storage = storage_with_feed().await;
        storage.enable_sync().await.unwrap();
        let baseline = storage.sync_identity().await.unwrap().next_sequence - 1;
        storage
            .upsert_articles(&[article("astronomy::mars", "astronomy", None)])
            .await
            .unwrap();
        assert!(
            storage
                .local_sync_events_after(baseline, 10)
                .await
                .unwrap()
                .is_empty()
        );

        storage.set_read("astronomy::mars", true).await.unwrap();
        storage.set_favorite("astronomy::mars", true).await.unwrap();
        storage
            .archive_article(
                "astronomy::mars",
                Utc.with_ymd_and_hms(2026, 8, 26, 18, 0, 0).unwrap(),
            )
            .await
            .unwrap();

        let events = storage.local_sync_events_after(baseline, 10).await.unwrap();
        assert_eq!(
            events
                .iter()
                .map(|event| event.kind.as_str())
                .collect::<Vec<_>>(),
            vec![
                "article_read_set",
                "article_favorite_set",
                "article_archived",
            ]
        );
        for event in &events {
            let article = match &event.payload {
                SyncEventPayload::ArticleReadSet { article, .. }
                | SyncEventPayload::ArticleFavoriteSet { article, .. }
                | SyncEventPayload::ArticleArchived { article } => article,
                _ => panic!("unexpected subscription event"),
            };
            assert_eq!(article.subscription_id, "astronomy");
            assert_eq!(article.entry_key, "mars");
            assert_eq!(article.title.as_deref(), Some("Title for astronomy::mars"));
        }
        let tombstones: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sync_tombstones WHERE entity_kind = 'article'",
        )
        .fetch_one(&storage.pool)
        .await
        .unwrap();
        assert_eq!(tombstones, 1);
    }

    /// Verifies automatic cache retention and CLI configuration imports never
    /// masquerade as synchronized user intent.
    #[tokio::test]
    async fn retention_and_cli_import_remain_unjournaled_after_sync_is_enabled() {
        let storage = storage_with_feed().await;
        storage.enable_sync().await.unwrap();
        let now = Utc.with_ymd_and_hms(2026, 8, 27, 12, 0, 0).unwrap();
        storage
            .upsert_articles(&[article(
                "astronomy::old",
                "astronomy",
                Some(now - chrono::Duration::days(60)),
            )])
            .await
            .unwrap();
        storage.set_read("astronomy::old", true).await.unwrap();
        let baseline = storage.sync_identity().await.unwrap().next_sequence - 1;

        assert_eq!(storage.archive_expired_read_articles(now).await.unwrap(), 1);
        storage
            .import_feeds(&[feed("bread", Platform::Other, "https://bread.example/feed")])
            .await
            .unwrap();

        assert!(
            storage
                .local_sync_events_after(baseline, 10)
                .await
                .unwrap()
                .is_empty()
        );
        let retention_tombstones: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sync_tombstones WHERE entity_kind = 'article'",
        )
        .fetch_one(&storage.pool)
        .await
        .unwrap();
        assert_eq!(retention_tombstones, 0);
    }

    /// Verifies grouped mutations deduplicate IDs, create one event per article,
    /// and roll back every state and event when one target is missing.
    #[tokio::test]
    async fn grouped_mutations_are_atomically_journaled_per_article() {
        let storage = storage_with_feed().await;
        storage
            .upsert_articles(&[
                article("astronomy::mars", "astronomy", None),
                article("astronomy::venus", "astronomy", None),
            ])
            .await
            .unwrap();
        storage.enable_sync().await.unwrap();
        let baseline = storage.sync_identity().await.unwrap().next_sequence - 1;

        assert!(
            storage
                .set_read_many(
                    &[
                        "astronomy::mars".to_string(),
                        "astronomy::mars".to_string(),
                        "astronomy::venus".to_string(),
                    ],
                    true,
                )
                .await
                .unwrap()
        );
        let read_events = storage.local_sync_events_after(baseline, 10).await.unwrap();
        assert_eq!(read_events.len(), 2);
        assert!(
            read_events
                .iter()
                .all(|event| event.kind == "article_read_set")
        );

        let before_failure = storage.sync_identity().await.unwrap().next_sequence;
        assert!(
            !storage
                .set_read_many(
                    &["astronomy::mars".to_string(), "missing".to_string(),],
                    false,
                )
                .await
                .unwrap()
        );
        assert_eq!(
            storage.sync_identity().await.unwrap().next_sequence,
            before_failure
        );
        assert!(
            storage
                .get_article("astronomy::mars")
                .await
                .unwrap()
                .unwrap()
                .is_read
        );

        assert!(
            storage
                .archive_articles_now(&[
                    "astronomy::mars".to_string(),
                    "astronomy::venus".to_string(),
                ])
                .await
                .unwrap()
        );
        let all_events = storage.local_sync_events_after(baseline, 10).await.unwrap();
        assert_eq!(all_events.len(), 4);
        assert_eq!(
            all_events
                .iter()
                .filter(|event| event.kind == "article_archived")
                .count(),
            2
        );
    }

    /// Verifies a journal failure rolls the corresponding business write back.
    #[tokio::test]
    async fn business_state_and_event_roll_back_together() {
        let storage = storage_with_feed().await;
        storage
            .upsert_articles(&[article("astronomy::mars", "astronomy", None)])
            .await
            .unwrap();
        storage.enable_sync().await.unwrap();
        let before = storage.sync_identity().await.unwrap();
        sqlx::raw_sql(
            r#"
                CREATE TRIGGER reject_favorite_sync_event
                BEFORE INSERT ON sync_events
                WHEN NEW.event_kind = 'article_favorite_set'
                BEGIN
                    SELECT RAISE(ABORT, 'simulated journal failure');
                END;
            "#,
        )
        .execute(&storage.pool)
        .await
        .unwrap();

        assert!(storage.set_favorite("astronomy::mars", true).await.is_err());
        let article = storage
            .get_article("astronomy::mars")
            .await
            .unwrap()
            .unwrap();
        assert!(!article.is_favorite);
        let after = storage.sync_identity().await.unwrap();
        assert_eq!(after.next_sequence, before.next_sequence);
        assert_eq!(after.clock, before.clock);
    }

    /// Verifies a state event can arrive before its dependencies, is retried,
    /// advances the contiguous cursor, and never creates a local outgoing event.
    #[tokio::test]
    async fn sync_import_retries_pending_dependencies_without_echoing_events() {
        const DEVICE: &str = "00000000-0000-4000-8000-00000000000a";
        let storage = Storage::open_in_memory().await.unwrap();
        storage.enable_sync().await.unwrap();
        let article = SyncArticleRef {
            subscription_id: "remote-feed".to_string(),
            entry_key: "first-post".to_string(),
            title: Some("A remote article".to_string()),
            url: Some("https://sync.example/first-post".to_string()),
            author: Some("Remote Author".to_string()),
            published_at: Some("2026-08-27T12:00:00Z".to_string()),
        };
        let state = remote_event(
            DEVICE,
            2,
            2_000,
            SyncEventPayload::ArticleReadSet {
                article: article.clone(),
                is_read: true,
            },
        );
        let first = storage
            .import_sync_events(&[state], Utc.timestamp_millis_opt(3_000).unwrap())
            .await
            .unwrap();
        assert_eq!(
            first,
            SyncImportReport {
                received: 1,
                imported: 1,
                duplicates: 0,
                applied: 0,
                pending: 1,
            }
        );
        assert!(storage.list_articles().await.unwrap().is_empty());

        let create = remote_subscription_created(DEVICE, 1, 1_000, "remote-feed");
        let second = storage
            .import_sync_events(&[create], Utc.timestamp_millis_opt(3_100).unwrap())
            .await
            .unwrap();
        assert_eq!(second.imported, 1);
        assert_eq!(second.applied, 2);
        assert_eq!(second.pending, 0);
        let stored = storage.list_articles().await.unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].article.title.as_deref(), Some("A remote article"));
        assert!(stored[0].is_read);
        assert!(
            storage
                .local_sync_events_after(0, 10)
                .await
                .unwrap()
                .is_empty()
        );
        let cursor: i64 = sqlx::query_scalar(
            "SELECT contiguous_sequence FROM sync_import_cursors WHERE remote_device_id = ?",
        )
        .bind(DEVICE)
        .fetch_one(&storage.pool)
        .await
        .unwrap();
        assert_eq!(cursor, 2);
    }

    /// Verifies independent registers use total event versions and converge for
    /// every arrival order of the same event set.
    #[tokio::test]
    async fn sync_import_converges_for_every_register_event_permutation() {
        const DEVICE_A: &str = "00000000-0000-4000-8000-00000000000a";
        const DEVICE_B: &str = "00000000-0000-4000-8000-00000000000b";
        let article = SyncArticleRef {
            subscription_id: "shared-feed".to_string(),
            entry_key: "entry".to_string(),
            title: Some("Convergent article".to_string()),
            url: None,
            author: None,
            published_at: None,
        };
        let events = vec![
            remote_subscription_created(DEVICE_A, 1, 100, "shared-feed"),
            remote_event(
                DEVICE_A,
                2,
                200,
                SyncEventPayload::ArticleReadSet {
                    article: article.clone(),
                    is_read: true,
                },
            ),
            remote_event(
                DEVICE_B,
                1,
                300,
                SyncEventPayload::ArticleReadSet {
                    article: article.clone(),
                    is_read: false,
                },
            ),
            remote_event(
                DEVICE_A,
                3,
                250,
                SyncEventPayload::ArticleFavoriteSet {
                    article,
                    is_favorite: true,
                },
            ),
        ];

        for permutation in event_permutations(&events) {
            let storage = Storage::open_in_memory().await.unwrap();
            storage.enable_sync().await.unwrap();
            for event in permutation {
                storage
                    .import_sync_events(&[event], Utc.timestamp_millis_opt(1_000).unwrap())
                    .await
                    .unwrap();
            }
            let articles = storage.list_articles().await.unwrap();
            assert_eq!(articles.len(), 1);
            assert!(!articles[0].is_read);
            assert!(articles[0].is_favorite);
            assert_eq!(
                articles[0].article.title.as_deref(),
                Some("Convergent article")
            );
        }
    }

    /// Verifies a manual archive tombstone dominates article registers in all
    /// arrival orders, even when the archive is received before the article.
    #[tokio::test]
    async fn sync_import_archive_tombstone_converges_for_every_permutation() {
        const DEVICE_A: &str = "00000000-0000-4000-8000-00000000000a";
        const DEVICE_B: &str = "00000000-0000-4000-8000-00000000000b";
        let article = SyncArticleRef {
            subscription_id: "shared-feed".to_string(),
            entry_key: "archived-entry".to_string(),
            title: Some("Archived everywhere".to_string()),
            url: None,
            author: None,
            published_at: None,
        };
        let events = vec![
            remote_subscription_created(DEVICE_A, 1, 100, "shared-feed"),
            remote_event(
                DEVICE_A,
                2,
                200,
                SyncEventPayload::ArticleReadSet {
                    article: article.clone(),
                    is_read: false,
                },
            ),
            remote_event(
                DEVICE_B,
                1,
                150,
                SyncEventPayload::ArticleArchived {
                    article: article.clone(),
                },
            ),
            remote_event(
                DEVICE_A,
                3,
                300,
                SyncEventPayload::ArticleFavoriteSet {
                    article,
                    is_favorite: true,
                },
            ),
        ];

        for permutation in event_permutations(&events) {
            let storage = Storage::open_in_memory().await.unwrap();
            storage.enable_sync().await.unwrap();
            for event in permutation {
                storage
                    .import_sync_events(&[event], Utc.timestamp_millis_opt(1_000).unwrap())
                    .await
                    .unwrap();
            }
            assert!(storage.list_articles().await.unwrap().is_empty());
            let archived: (bool, Option<String>, Option<String>) =
                sqlx::query_as("SELECT is_archived, archive_reason, content FROM articles")
                    .fetch_one(&storage.pool)
                    .await
                    .unwrap();
            assert_eq!(archived, (true, Some("manual".to_string()), None));
        }
    }

    /// Verifies local mutations participate in the same version registers as
    /// imported events, while remote application creates no outgoing echo.
    #[tokio::test]
    async fn sync_import_compares_remote_events_with_local_register_versions() {
        const DEVICE: &str = "00000000-0000-4000-8000-00000000000a";
        let storage = storage_with_feed().await;
        storage
            .upsert_articles(&[article("astronomy::mars", "astronomy", None)])
            .await
            .unwrap();
        storage.enable_sync().await.unwrap();
        let baseline = storage.sync_identity().await.unwrap();
        let reference = sync_article_ref("mars");
        let older = remote_event(
            DEVICE,
            1,
            1,
            SyncEventPayload::ArticleReadSet {
                article: reference.clone(),
                is_read: true,
            },
        );
        storage
            .import_sync_events(
                &[older],
                Utc.timestamp_millis_opt(baseline.clock.physical_milliseconds)
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            !storage
                .get_article("astronomy::mars")
                .await
                .unwrap()
                .unwrap()
                .is_read
        );

        let newer = remote_event(
            DEVICE,
            2,
            baseline.clock.physical_milliseconds + 10_000,
            SyncEventPayload::ArticleReadSet {
                article: reference,
                is_read: true,
            },
        );
        storage
            .import_sync_events(
                &[newer],
                Utc.timestamp_millis_opt(baseline.clock.physical_milliseconds)
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            storage
                .get_article("astronomy::mars")
                .await
                .unwrap()
                .unwrap()
                .is_read
        );
        assert_eq!(
            storage.local_sync_events_after(0, 100).await.unwrap().len(),
            baseline.next_sequence as usize - 1
        );
    }

    /// Verifies concurrent additions of one URL collapse to the smallest alias
    /// and produce only one subscription projection regardless of arrival order.
    #[tokio::test]
    async fn sync_import_deduplicates_concurrent_subscription_additions() {
        const DEVICE_A: &str = "00000000-0000-4000-8000-00000000000a";
        const DEVICE_B: &str = "00000000-0000-4000-8000-00000000000b";
        let events = [
            remote_subscription_created(DEVICE_A, 1, 100, "subscription-a"),
            remote_subscription_created(DEVICE_B, 1, 200, "subscription-b"),
        ];

        for order in [events.to_vec(), vec![events[1].clone(), events[0].clone()]] {
            let storage = Storage::open_in_memory().await.unwrap();
            storage.enable_sync().await.unwrap();
            storage
                .import_sync_events(&order, Utc.timestamp_millis_opt(1_000).unwrap())
                .await
                .unwrap();
            let feeds = storage.list_feeds().await.unwrap();
            assert_eq!(feeds.len(), 1);
            assert_eq!(feeds[0].url, "https://sync.example/feed");
            let aliases: Vec<(String, String)> = sqlx::query_as(
                "SELECT alias_id, canonical_id FROM sync_subscription_aliases ORDER BY alias_id",
            )
            .fetch_all(&storage.pool)
            .await
            .unwrap();
            assert_eq!(
                aliases,
                vec![
                    ("subscription-a".to_string(), "subscription-a".to_string()),
                    ("subscription-b".to_string(), "subscription-a".to_string()),
                ]
            );
        }
    }

    /// Verifies a deletion dominates later register events and an explicit
    /// re-addition linked to its tombstone creates a fresh incarnation.
    #[tokio::test]
    async fn sync_import_applies_permanent_deletion_and_explicit_readdition() {
        const DEVICE: &str = "00000000-0000-4000-8000-00000000000a";
        let create = remote_subscription_created(DEVICE, 1, 100, "old-feed");
        let deletion = remote_event(
            DEVICE,
            2,
            200,
            SyncEventPayload::SubscriptionDeleted {
                subscription_id: "old-feed".to_string(),
            },
        );
        let stale_activation = remote_event(
            DEVICE,
            3,
            300,
            SyncEventPayload::SubscriptionActiveSet {
                subscription_id: "old-feed".to_string(),
                is_active: true,
            },
        );
        let replacement = remote_event(
            DEVICE,
            4,
            400,
            SyncEventPayload::SubscriptionCreated {
                subscription_id: "new-feed".to_string(),
                normalized_url: "https://sync.example/feed".to_string(),
                platform_hint: Platform::Other,
                is_active: true,
                parent_tombstone: Some(SyncEventId {
                    device_id: DEVICE.to_string(),
                    sequence: 2,
                }),
            },
        );
        let storage = Storage::open_in_memory().await.unwrap();
        storage.enable_sync().await.unwrap();
        storage
            .import_sync_events(
                &[stale_activation, replacement, deletion, create],
                Utc.timestamp_millis_opt(1_000).unwrap(),
            )
            .await
            .unwrap();

        let feeds = storage.list_feeds().await.unwrap();
        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds[0].id, "new-feed");
        assert!(feeds[0].is_active);
        let old_tombstone: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sync_tombstones WHERE entity_kind = 'subscription' AND entity_key = 'old-feed'",
        )
        .fetch_one(&storage.pool)
        .await
        .unwrap();
        assert_eq!(old_tombstone, 1);
    }

    /// Verifies re-additions referencing concurrent deletions are regrouped
    /// under the winning deletion tombstone.
    #[tokio::test]
    async fn sync_import_recanonicalizes_concurrent_deletion_parents() {
        const DEVICE_A: &str = "00000000-0000-4000-8000-00000000000a";
        const DEVICE_B: &str = "00000000-0000-4000-8000-00000000000b";
        let create = remote_subscription_created(DEVICE_A, 1, 100, "old-feed");
        let deletion_a = remote_event(
            DEVICE_A,
            2,
            200,
            SyncEventPayload::SubscriptionDeleted {
                subscription_id: "old-feed".to_string(),
            },
        );
        let deletion_b = remote_event(
            DEVICE_B,
            1,
            300,
            SyncEventPayload::SubscriptionDeleted {
                subscription_id: "old-feed".to_string(),
            },
        );
        let readd_a = remote_event(
            DEVICE_A,
            3,
            250,
            SyncEventPayload::SubscriptionCreated {
                subscription_id: "readd-a".to_string(),
                normalized_url: "https://sync.example/feed".to_string(),
                platform_hint: Platform::Other,
                is_active: true,
                parent_tombstone: Some(SyncEventId {
                    device_id: DEVICE_A.to_string(),
                    sequence: 2,
                }),
            },
        );
        let readd_b = remote_event(
            DEVICE_B,
            2,
            350,
            SyncEventPayload::SubscriptionCreated {
                subscription_id: "readd-b".to_string(),
                normalized_url: "https://sync.example/feed".to_string(),
                platform_hint: Platform::Other,
                is_active: true,
                parent_tombstone: Some(SyncEventId {
                    device_id: DEVICE_B.to_string(),
                    sequence: 1,
                }),
            },
        );
        let storage = Storage::open_in_memory().await.unwrap();
        storage.enable_sync().await.unwrap();
        for event in [create, deletion_a, readd_a, deletion_b, readd_b] {
            storage
                .import_sync_events(&[event], Utc.timestamp_millis_opt(1_000).unwrap())
                .await
                .unwrap();
        }

        let feeds = storage.list_feeds().await.unwrap();
        assert_eq!(feeds.len(), 1);
        let readds: Vec<(String, String, String, i64)> = sqlx::query_as(
            r#"
                SELECT alias_id, canonical_id, parent_tombstone_device_id,
                       parent_tombstone_sequence
                FROM sync_subscription_aliases
                WHERE alias_id LIKE 'readd-%'
                ORDER BY alias_id
            "#,
        )
        .fetch_all(&storage.pool)
        .await
        .unwrap();
        assert_eq!(
            readds,
            vec![
                (
                    "readd-a".to_string(),
                    "readd-a".to_string(),
                    DEVICE_B.to_string(),
                    1,
                ),
                (
                    "readd-b".to_string(),
                    "readd-a".to_string(),
                    DEVICE_B.to_string(),
                    1,
                ),
            ]
        );
    }

    /// Verifies duplicates are idempotent and a conflicting identity rolls the
    /// complete batch back without advancing the local clock.
    #[tokio::test]
    async fn sync_import_is_idempotent_and_rejects_identity_collisions_atomically() {
        const DEVICE: &str = "00000000-0000-4000-8000-00000000000a";
        let storage = Storage::open_in_memory().await.unwrap();
        storage.enable_sync().await.unwrap();
        let create = remote_subscription_created(DEVICE, 1, 100, "remote-feed");
        storage
            .import_sync_events(
                std::slice::from_ref(&create),
                Utc.timestamp_millis_opt(1_000).unwrap(),
            )
            .await
            .unwrap();
        let duplicate = storage
            .import_sync_events(
                std::slice::from_ref(&create),
                Utc.timestamp_millis_opt(2_000).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(duplicate.duplicates, 1);
        assert_eq!(duplicate.imported, 0);
        assert_eq!(storage.list_feeds().await.unwrap().len(), 1);

        let before = storage.sync_identity().await.unwrap();
        let extra = remote_event(
            DEVICE,
            2,
            150,
            SyncEventPayload::SubscriptionActiveSet {
                subscription_id: "remote-feed".to_string(),
                is_active: false,
            },
        );
        let collision = remote_subscription_created(DEVICE, 1, 100, "different-feed");
        assert!(
            storage
                .import_sync_events(
                    &[extra, collision],
                    Utc.timestamp_millis_opt(3_000).unwrap(),
                )
                .await
                .is_err()
        );
        let after = storage.sync_identity().await.unwrap();
        assert_eq!(after.clock, before.clock);
        let imported_extra: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sync_events WHERE device_id = ? AND sequence = 2",
        )
        .bind(DEVICE)
        .fetch_one(&storage.pool)
        .await
        .unwrap();
        assert_eq!(imported_extra, 0);
    }

    /// Verifies malformed envelopes and imports attempted before activation
    /// leave both the journal and business projections untouched.
    #[tokio::test]
    async fn sync_import_validates_before_changing_state() {
        const DEVICE: &str = "00000000-0000-4000-8000-00000000000a";
        let storage = Storage::open_in_memory().await.unwrap();
        let event = remote_subscription_created(DEVICE, 1, 100, "remote-feed");
        assert!(
            storage
                .import_sync_events(
                    std::slice::from_ref(&event),
                    Utc.timestamp_millis_opt(1_000).unwrap(),
                )
                .await
                .is_err()
        );
        storage.enable_sync().await.unwrap();
        let mut invalid = event;
        invalid.kind = "article_read_set".to_string();
        assert!(
            storage
                .import_sync_events(&[invalid], Utc.timestamp_millis_opt(1_000).unwrap())
                .await
                .is_err()
        );
        assert!(storage.list_feeds().await.unwrap().is_empty());
        let remote_events: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sync_events WHERE device_id = ?")
                .bind(DEVICE)
                .fetch_one(&storage.pool)
                .await
                .unwrap();
        assert_eq!(remote_events, 0);
    }

    /// Verifies feed metadata and refresh errors survive reloads and a later
    /// success clears only the obsolete error.
    #[tokio::test]
    async fn record_feed_refreshes_persists_details_and_clears_recovered_errors() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("inkriver.db");
        let storage = Storage::open(&database_path).await.unwrap();
        storage
            .import_feeds(&[feed(
                "astronomy",
                Platform::Substack,
                "https://astronomy.example/feed",
            )])
            .await
            .unwrap();
        let published_at = Utc.with_ymd_and_hms(2026, 8, 10, 18, 0, 0).unwrap();
        storage
            .upsert_articles(&[article("astronomy::mars", "astronomy", Some(published_at))])
            .await
            .unwrap();
        let failure_at = Utc.with_ymd_and_hms(2026, 8, 11, 9, 0, 0).unwrap();
        storage
            .record_feed_refreshes(
                &[],
                &[FeedRefreshFailure {
                    feed_id: "astronomy".to_string(),
                    stage: "HTTP request".to_string(),
                    message: "network unavailable".to_string(),
                }],
                failure_at,
            )
            .await
            .unwrap();

        storage.close().await;
        let storage = Storage::open(&database_path).await.unwrap();

        let failed = storage.list_feeds().await.unwrap().remove(0);
        assert_eq!(failed.author.as_deref(), Some("Test Author"));
        assert_eq!(failed.last_published_at, Some(published_at));
        assert_eq!(failed.last_error.as_ref().unwrap().occurred_at, failure_at);

        let success_at = Utc.with_ymd_and_hms(2026, 8, 12, 10, 30, 0).unwrap();
        storage
            .record_feed_refreshes(
                &[FeedMetadata {
                    id: "astronomy".to_string(),
                    title: "Night sky notes".to_string(),
                    description: "A practical guide to astronomy".to_string(),
                    author: Some("Claire du Ciel".to_string()),
                    site_url: "https://astronomy.example".to_string(),
                    declared_icon_url: None,
                }],
                &[],
                success_at,
            )
            .await
            .unwrap();

        let recovered = storage.list_feeds().await.unwrap().remove(0);
        assert_eq!(recovered.title.as_deref(), Some("Night sky notes"));
        assert_eq!(
            recovered.description.as_deref(),
            Some("A practical guide to astronomy")
        );
        assert_eq!(recovered.author.as_deref(), Some("Claire du Ciel"));
        assert_eq!(recovered.last_success_at, Some(success_at));
        assert_eq!(recovered.last_published_at, Some(published_at));
        assert!(recovered.last_error.is_none());
    }

    #[tokio::test]
    async fn content_kind_migration_classifies_legacy_rows_conservatively() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .in_memory(true)
                    .foreign_keys(true),
            )
            .await
            .unwrap();
        sqlx::raw_sql(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/migrations/202608080001_initial_schema.sql"
        )))
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO feeds (id, platform, url) VALUES ('feed', 'other', 'https://example.com/feed')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO articles (id, feed_id, content, source) VALUES ('with-content', 'feed', '<p>legacy</p>', 'other'), ('without-content', 'feed', NULL, 'other')")
            .execute(&pool)
            .await
            .unwrap();

        sqlx::raw_sql(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/migrations/202608080002_article_content_kind.sql"
        )))
        .execute(&pool)
        .await
        .unwrap();

        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT id, content_kind FROM articles ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            rows,
            vec![
                ("with-content".to_string(), "unknown".to_string()),
                ("without-content".to_string(), "missing".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn archiving_migration_keeps_existing_articles_visible_by_default() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .in_memory(true)
                    .foreign_keys(true),
            )
            .await
            .unwrap();
        for migration in [
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/migrations/202608080001_initial_schema.sql"
            )),
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/migrations/202608080002_article_content_kind.sql"
            )),
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/migrations/202608120001_feed_details.sql"
            )),
        ] {
            sqlx::raw_sql(migration).execute(&pool).await.unwrap();
        }
        sqlx::query("INSERT INTO feeds (id, platform, url) VALUES ('feed', 'other', 'https://example.com/feed')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO articles (id, feed_id, content, source) VALUES ('legacy', 'feed', '<p>legacy</p>', 'other')")
            .execute(&pool)
            .await
            .unwrap();

        sqlx::raw_sql(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/migrations/202608140001_article_archiving.sql"
        )))
        .execute(&pool)
        .await
        .unwrap();

        let state: (bool, Option<DateTime<Utc>>, Option<String>) = sqlx::query_as(
            "SELECT is_archived, archived_at, archive_reason FROM articles WHERE id = 'legacy'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, (false, None, None));
    }

    #[tokio::test]
    async fn extraction_migration_preserves_articles_and_local_state() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .in_memory(true)
                    .foreign_keys(true),
            )
            .await
            .unwrap();
        for migration in [
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/migrations/202608080001_initial_schema.sql"
            )),
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/migrations/202608080002_article_content_kind.sql"
            )),
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/migrations/202608120001_feed_details.sql"
            )),
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/migrations/202608140001_article_archiving.sql"
            )),
        ] {
            sqlx::raw_sql(migration).execute(&pool).await.unwrap();
        }
        sqlx::query("INSERT INTO feeds (id, platform, url) VALUES ('feed', 'other', 'https://example.com/feed')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            r#"
                INSERT INTO articles (
                    id, feed_id, title, content, source, is_read, is_favorite,
                    content_kind, is_archived, archived_at, archive_reason
                ) VALUES (
                    'legacy', 'feed', 'Legacy title', '<p>legacy excerpt</p>',
                    'other', 1, 1, 'excerpt', 1, '2026-08-15T12:00:00Z', 'manual'
                )
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::raw_sql(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/migrations/202608160001_article_extraction.sql"
        )))
        .execute(&pool)
        .await
        .unwrap();

        type ExtractionMigrationRow = (
            String,
            String,
            bool,
            bool,
            bool,
            Option<DateTime<Utc>>,
            Option<String>,
            Option<String>,
            Option<String>,
            i64,
        );
        let row: ExtractionMigrationRow = sqlx::query_as(
            r#"
                    SELECT title, content_kind, is_read, is_favorite, is_archived,
                           archived_at, archive_reason, extraction_attempted_at,
                           extraction_last_error, extraction_attempt_count
                    FROM articles WHERE id = 'legacy'
                "#,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            row,
            (
                "Legacy title".to_string(),
                "excerpt".to_string(),
                true,
                true,
                true,
                Some(Utc.with_ymd_and_hms(2026, 8, 15, 12, 0, 0).unwrap()),
                Some("manual".to_string()),
                None,
                None,
                0,
            )
        );
        sqlx::query(
            "INSERT INTO articles (id, feed_id, source, content_kind) VALUES ('extracted', 'feed', 'other', 'extracted')",
        )
        .execute(&pool)
        .await
        .unwrap();
    }

    /// Verifies the feed-logo migration leaves subscriptions, articles, and
    /// their local state untouched while initializing an empty logo cache.
    #[tokio::test]
    async fn feed_logo_migration_preserves_existing_data() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .in_memory(true)
                    .foreign_keys(true),
            )
            .await
            .unwrap();
        for migration in [
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/migrations/202608080001_initial_schema.sql"
            )),
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/migrations/202608080002_article_content_kind.sql"
            )),
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/migrations/202608120001_feed_details.sql"
            )),
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/migrations/202608140001_article_archiving.sql"
            )),
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/migrations/202608160001_article_extraction.sql"
            )),
        ] {
            sqlx::raw_sql(migration).execute(&pool).await.unwrap();
        }
        sqlx::query(
            "INSERT INTO feeds (id, platform, url, title) VALUES ('feed', 'other', 'https://example.com/feed', 'Example')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            r#"
                INSERT INTO articles (
                    id, feed_id, title, content, source, content_kind,
                    is_read, is_favorite, is_archived, archived_at, archive_reason
                ) VALUES (
                    'article', 'feed', 'Kept article', '<p>kept</p>', 'other',
                    'full', 1, 1, 1, '2026-08-20T12:00:00Z', 'manual'
                )
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::raw_sql(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/migrations/202608220001_feed_logos.sql"
        )))
        .execute(&pool)
        .await
        .unwrap();

        let feed_state: (String, Option<String>, Option<Vec<u8>>) =
            sqlx::query_as("SELECT title, site_url, logo_png FROM feeds WHERE id = 'feed'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(feed_state, ("Example".to_string(), None, None));
        let article_state: (String, bool, bool, bool, Option<String>) = sqlx::query_as(
            "SELECT title, is_read, is_favorite, is_archived, archive_reason FROM articles WHERE id = 'article'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            article_state,
            (
                "Kept article".to_string(),
                true,
                true,
                true,
                Some("manual".to_string())
            )
        );
    }

    /// Verifies the synchronization migration preserves all existing feed and
    /// article state while deriving a feed-independent entry key.
    #[tokio::test]
    async fn synchronization_migration_preserves_existing_data_and_backfills_entry_keys() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .in_memory(true)
                    .foreign_keys(true),
            )
            .await
            .unwrap();
        for migration in [
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/migrations/202608080001_initial_schema.sql"
            )),
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/migrations/202608080002_article_content_kind.sql"
            )),
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/migrations/202608120001_feed_details.sql"
            )),
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/migrations/202608140001_article_archiving.sql"
            )),
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/migrations/202608160001_article_extraction.sql"
            )),
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/migrations/202608220001_feed_logos.sql"
            )),
        ] {
            sqlx::raw_sql(migration).execute(&pool).await.unwrap();
        }
        sqlx::query(
            r#"
                INSERT INTO feeds (
                    id, platform, url, is_active, title, description, author,
                    logo_png
                ) VALUES (
                    'feed%id', 'other', 'https://example.com/feed', 0,
                    'Existing feed', 'Existing description', 'Existing author',
                    X'89504E47'
                )
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            r#"
                INSERT INTO articles (
                    id, feed_id, title, author, published_at, url, content,
                    source, is_read, is_favorite, content_kind, is_archived,
                    archived_at, archive_reason, extraction_attempt_count
                ) VALUES (
                    'feed%id::publisher-guid', 'feed%id', 'Existing article',
                    'Existing author', '2026-08-20T12:00:00Z',
                    'https://example.com/article', '<p>Existing body</p>',
                    'other', 1, 1, 'full', 1, '2026-08-21T12:00:00Z',
                    'manual', 3
                )
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::raw_sql(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/migrations/202608260001_sync_journal.sql"
        )))
        .execute(&pool)
        .await
        .unwrap();

        type PreservedSyncMigrationRow = (
            String,
            String,
            String,
            bool,
            bool,
            bool,
            Option<String>,
            i64,
        );
        let row: PreservedSyncMigrationRow = sqlx::query_as(
            r#"
                SELECT id, feed_id, entry_key, is_read, is_favorite,
                       is_archived, archive_reason, extraction_attempt_count
                FROM articles
            "#,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            row,
            (
                "feed%id::publisher-guid".to_string(),
                "feed%id".to_string(),
                "publisher-guid".to_string(),
                true,
                true,
                true,
                Some("manual".to_string()),
                3,
            )
        );

        let feed_state: (String, bool, Option<Vec<u8>>) =
            sqlx::query_as("SELECT title, is_active, logo_png FROM feeds")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            feed_state,
            (
                "Existing feed".to_string(),
                false,
                Some(vec![0x89, 0x50, 0x4e, 0x47]),
            )
        );

        let sync_tables: i64 = sqlx::query_scalar(
            r#"
                SELECT COUNT(*) FROM sqlite_master
                WHERE type = 'table' AND name IN (
                    'sync_local_state', 'sync_events', 'sync_import_cursors',
                    'sync_pending_events', 'sync_subscription_aliases',
                    'sync_entity_versions', 'sync_tombstones',
                    'sync_article_identities'
                )
            "#,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(sync_tables, 8);

        sqlx::query(
            r#"
                INSERT INTO sync_local_state (
                    singleton, device_id, next_sequence,
                    hlc_physical_ms, hlc_counter
                ) VALUES (1, 'existing-device', 2, 1234, 0)
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            r#"
                INSERT INTO sync_events (
                    device_id, sequence, hlc_physical_ms, hlc_counter,
                    protocol_version, event_kind, payload_json
                ) VALUES (
                    'existing-device', 1, 1234, 0, 1,
                    'article_read_set', '{}'
                )
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::raw_sql(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/migrations/202608260002_sync_event_production.sql"
        )))
        .execute(&pool)
        .await
        .unwrap();

        let migrated_sync_state: (String, i64, i64, bool) = sqlx::query_as(
            r#"
                SELECT device_id, next_sequence, hlc_physical_ms, is_enabled
                FROM sync_local_state
                WHERE singleton = 1
            "#,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            migrated_sync_state,
            ("existing-device".to_string(), 2, 1234, false)
        );
        let preserved_events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_events")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(preserved_events, 1);

        sqlx::raw_sql(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/migrations/202608270001_sync_segments.sql"
        )))
        .execute(&pool)
        .await
        .unwrap();
        let segment_state: (String, i64, i64) = sqlx::query_as(
            r#"
                SELECT device_id, next_sequence, last_exported_sequence
                FROM sync_local_state
                WHERE singleton = 1
            "#,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(segment_state, ("existing-device".to_string(), 2, 0));

        sqlx::raw_sql(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/migrations/202608270002_encrypted_sync_segments.sql"
        )))
        .execute(&pool)
        .await
        .unwrap();
        let encrypted_export_cursors: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sync_export_cursors")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(encrypted_export_cursors, 0);

        let duplicate_entry_key = sqlx::query(
            r#"
                INSERT INTO articles (
                    id, feed_id, entry_key, source, content_kind
                ) VALUES (
                    'different-id', 'feed%id', 'publisher-guid', 'other', 'missing'
                )
            "#,
        )
        .execute(&pool)
        .await;
        assert!(duplicate_entry_key.is_err());
    }

    /// Verifies logo discovery is scoped to successful Other feeds, observes
    /// the retry delay, persists success permanently, and invalidates a logo
    /// when the website changes.
    #[tokio::test]
    async fn feed_logo_candidates_observe_scope_retry_and_site_changes() {
        let storage = Storage::open_in_memory().await.unwrap();
        storage
            .import_feeds(&[
                feed("other", Platform::Other, "https://example.com/feed"),
                feed(
                    "substack",
                    Platform::Substack,
                    "https://newsletter.substack.com/feed",
                ),
                feed(
                    "untouched",
                    Platform::Other,
                    "https://untouched.example/feed",
                ),
            ])
            .await
            .unwrap();
        let now = Utc.with_ymd_and_hms(2026, 8, 22, 12, 0, 0).unwrap();
        storage
            .record_feed_refreshes(
                &[
                    FeedMetadata {
                        id: "other".to_string(),
                        title: "Example".to_string(),
                        description: String::new(),
                        author: None,
                        site_url: "https://example.com/articles/".to_string(),
                        declared_icon_url: Some("/icon.png".to_string()),
                    },
                    FeedMetadata {
                        id: "substack".to_string(),
                        title: "Newsletter".to_string(),
                        description: String::new(),
                        author: None,
                        site_url: "https://newsletter.substack.com/".to_string(),
                        declared_icon_url: Some("/favicon.ico".to_string()),
                    },
                    FeedMetadata {
                        id: "untouched".to_string(),
                        title: "Untouched".to_string(),
                        description: String::new(),
                        author: None,
                        site_url: "https://untouched.example/".to_string(),
                        declared_icon_url: None,
                    },
                ],
                &[],
                now,
            )
            .await
            .unwrap();

        let successful = vec!["other".to_string(), "substack".to_string()];
        let candidates = storage
            .feed_logo_candidates(&successful, now)
            .await
            .unwrap();
        assert_eq!(
            candidates,
            vec![FeedLogoCandidate {
                feed_id: "other".to_string(),
                site_url: "https://example.com/articles/".to_string(),
                declared_icon_url: Some("/icon.png".to_string()),
            }]
        );

        storage
            .record_feed_logo_failure(&candidates[0], "not found", now)
            .await
            .unwrap();
        assert!(
            storage
                .feed_logo_candidates(&successful, now + chrono::Duration::days(6))
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            storage
                .feed_logo_candidates(&successful, now + chrono::Duration::days(LOGO_RETRY_DAYS))
                .await
                .unwrap()
                .len(),
            1
        );

        let changed_icon_at = now + chrono::Duration::hours(1);
        storage
            .record_feed_refreshes(
                &[FeedMetadata {
                    id: "other".to_string(),
                    title: "Example".to_string(),
                    description: String::new(),
                    author: None,
                    site_url: "https://example.com/articles/".to_string(),
                    declared_icon_url: Some("/new-icon.png".to_string()),
                }],
                &[],
                changed_icon_at,
            )
            .await
            .unwrap();
        let changed_icon = storage
            .feed_logo_candidates(&successful, changed_icon_at)
            .await
            .unwrap();
        assert_eq!(
            changed_icon[0].declared_icon_url.as_deref(),
            Some("/new-icon.png")
        );

        storage
            .record_feed_logo_success(&changed_icon[0], b"normalized png", changed_icon_at)
            .await
            .unwrap();
        assert_eq!(
            storage.list_feeds().await.unwrap()[0].logo_png.as_deref(),
            Some(b"normalized png".as_slice())
        );
        assert!(
            storage
                .feed_logo_candidates(&successful, now + chrono::Duration::days(365))
                .await
                .unwrap()
                .is_empty()
        );

        let changed_path_at = changed_icon_at + chrono::Duration::minutes(30);
        storage
            .record_feed_refreshes(
                &[FeedMetadata {
                    id: "other".to_string(),
                    title: "Example moved within its site".to_string(),
                    description: String::new(),
                    author: None,
                    site_url: "https://example.com/new-home/".to_string(),
                    declared_icon_url: None,
                }],
                &[],
                changed_path_at,
            )
            .await
            .unwrap();
        let stored = storage
            .list_feeds()
            .await
            .unwrap()
            .into_iter()
            .find(|feed| feed.id == "other")
            .unwrap();
        assert_eq!(
            stored.logo_png.as_deref(),
            Some(b"normalized png".as_slice())
        );
        assert!(
            storage
                .feed_logo_candidates(&successful, changed_path_at)
                .await
                .unwrap()
                .is_empty()
        );

        let changed_site_at = changed_icon_at + chrono::Duration::hours(1);
        storage
            .record_feed_refreshes(
                &[FeedMetadata {
                    id: "other".to_string(),
                    title: "Example moved".to_string(),
                    description: String::new(),
                    author: None,
                    site_url: "https://new.example/".to_string(),
                    declared_icon_url: None,
                }],
                &[],
                changed_site_at,
            )
            .await
            .unwrap();
        let moved = storage
            .feed_logo_candidates(&successful, changed_site_at)
            .await
            .unwrap();
        assert_eq!(moved[0].site_url, "https://new.example/");
        let stored = storage
            .list_feeds()
            .await
            .unwrap()
            .into_iter()
            .find(|feed| feed.id == "other")
            .unwrap();
        assert!(stored.logo_png.is_none());
    }

    /// Verifies one refresh never schedules more website-logo downloads than
    /// the configured bound.
    #[tokio::test]
    async fn feed_logo_candidates_are_limited_to_twenty() {
        let storage = Storage::open_in_memory().await.unwrap();
        let feeds = (0..21)
            .map(|index| {
                feed(
                    &format!("feed-{index:02}"),
                    Platform::Other,
                    &format!("https://site-{index:02}.example/feed"),
                )
            })
            .collect::<Vec<_>>();
        storage.import_feeds(&feeds).await.unwrap();
        let metadata = feeds
            .iter()
            .map(|feed| FeedMetadata {
                id: feed.id.clone(),
                title: feed.id.clone(),
                description: String::new(),
                author: None,
                site_url: feed.url.replace("/feed", "/"),
                declared_icon_url: None,
            })
            .collect::<Vec<_>>();
        let now = Utc.with_ymd_and_hms(2026, 8, 22, 12, 0, 0).unwrap();
        storage
            .record_feed_refreshes(&metadata, &[], now)
            .await
            .unwrap();
        let successful = feeds.iter().map(|feed| feed.id.clone()).collect::<Vec<_>>();

        let candidates = storage
            .feed_logo_candidates(&successful, now)
            .await
            .unwrap();

        assert_eq!(candidates.len(), MAX_LOGO_ATTEMPTS_PER_REFRESH);
        assert_eq!(candidates.first().unwrap().feed_id, "feed-00");
        assert_eq!(candidates.last().unwrap().feed_id, "feed-19");
    }

    /// Verifies a feed import persists every configured value as active.
    #[tokio::test]
    async fn import_feeds_inserts_active_subscriptions() {
        let storage = Storage::open_in_memory().await.unwrap();
        let feeds = vec![
            feed(
                "astronomy",
                Platform::Substack,
                "https://astronomy.example/feed",
            ),
            feed("bread", Platform::Medium, "https://medium.com/feed/@bread"),
        ];

        storage.import_feeds(&feeds).await.unwrap();

        assert_eq!(
            storage.list_feeds().await.unwrap(),
            vec![
                stored_feed(
                    "astronomy",
                    Platform::Substack,
                    "https://astronomy.example/feed",
                    true,
                ),
                stored_feed(
                    "bread",
                    Platform::Medium,
                    "https://medium.com/feed/@bread",
                    true,
                ),
            ]
        );
    }

    /// Verifies a later import updates retained feeds and only deactivates missing ones.
    #[tokio::test]
    async fn import_feeds_updates_and_deactivates_without_deleting() {
        let storage = Storage::open_in_memory().await.unwrap();
        storage
            .import_feeds(&[
                feed(
                    "astronomy",
                    Platform::Substack,
                    "https://astronomy.example/feed",
                ),
                feed("bread", Platform::Medium, "https://medium.com/feed/@bread"),
            ])
            .await
            .unwrap();

        storage
            .import_feeds(&[feed(
                "astronomy",
                Platform::Other,
                "https://astronomy.example/new-feed",
            )])
            .await
            .unwrap();

        assert_eq!(
            storage.list_feeds().await.unwrap(),
            vec![
                stored_feed(
                    "astronomy",
                    Platform::Other,
                    "https://astronomy.example/new-feed",
                    true,
                ),
                stored_feed(
                    "bread",
                    Platform::Medium,
                    "https://medium.com/feed/@bread",
                    false,
                ),
            ]
        );
    }

    #[tokio::test]
    async fn add_feed_normalizes_url_detects_platform_and_generates_uuid() {
        let storage = Storage::open_in_memory().await.unwrap();

        let stored = storage
            .add_feed(" https://letters.substack.com/feed#latest ", None)
            .await
            .unwrap();

        assert!(uuid::Uuid::parse_str(&stored.id).is_ok());
        assert_eq!(stored.url, "https://letters.substack.com/feed");
        assert_eq!(stored.platform, Platform::Substack);
        assert!(stored.is_active);
    }

    #[tokio::test]
    async fn add_feed_uses_platform_override_and_rejects_active_duplicate() {
        let storage = Storage::open_in_memory().await.unwrap();
        let stored = storage
            .add_feed("https://medium.com/feed/@inkriver", Some(Platform::Other))
            .await
            .unwrap();
        assert_eq!(stored.platform, Platform::Other);

        let error = storage
            .add_feed("https://medium.com/feed/@inkriver#fragment", None)
            .await
            .unwrap_err();
        assert_eq!(
            error,
            SubscriptionError::DuplicateActiveUrl("https://medium.com/feed/@inkriver".to_string())
        );
    }

    #[tokio::test]
    async fn add_feed_reactivates_retained_subscription_with_same_uuid() {
        let storage = Storage::open_in_memory().await.unwrap();
        let original = storage
            .add_feed("https://example.com/feed", None)
            .await
            .unwrap();
        storage.set_feed_active(&original.id, false).await.unwrap();

        let reactivated = storage
            .add_feed("https://example.com/feed", Some(Platform::Medium))
            .await
            .unwrap();

        assert_eq!(reactivated.id, original.id);
        assert_eq!(reactivated.platform, Platform::Medium);
        assert!(reactivated.is_active);
    }

    #[tokio::test]
    async fn set_feed_active_preserves_articles_and_reports_unknown_ids() {
        let storage = storage_with_feed().await;
        let cached = article("astronomy::mars", "astronomy", None);
        storage.upsert_articles(&[cached]).await.unwrap();

        let inactive = storage.set_feed_active("astronomy", false).await.unwrap();

        assert!(!inactive.is_active);
        assert_eq!(storage.list_articles().await.unwrap().len(), 1);
        assert!(storage.active_feed_config().await.unwrap().is_empty());
        assert_eq!(
            storage.set_feed_active("missing", true).await.unwrap_err(),
            SubscriptionError::NotFound("missing".to_string())
        );
    }

    #[tokio::test]
    async fn delete_feed_removes_its_articles_and_local_states_only() {
        let storage = Storage::open_in_memory().await.unwrap();
        storage
            .import_feeds(&[
                feed(
                    "astronomy",
                    Platform::Substack,
                    "https://astronomy.example/feed",
                ),
                feed("bread", Platform::Medium, "https://medium.com/feed/@bread"),
            ])
            .await
            .unwrap();
        storage
            .upsert_articles(&[
                article("astronomy::mars", "astronomy", None),
                article("astronomy::venus", "astronomy", None),
                article("bread::starter", "bread", None),
            ])
            .await
            .unwrap();
        storage.set_read("astronomy::mars", true).await.unwrap();
        storage
            .set_favorite("astronomy::venus", true)
            .await
            .unwrap();
        storage
            .archive_article(
                "astronomy::mars",
                Utc.with_ymd_and_hms(2026, 8, 12, 12, 0, 0).unwrap(),
            )
            .await
            .unwrap();

        let result = storage.delete_feed("astronomy").await.unwrap();

        assert_eq!(
            result,
            DeleteFeedResult {
                feed_id: "astronomy".to_string(),
                deleted_articles: 2,
            }
        );
        let retained_feeds = storage.list_feeds().await.unwrap();
        assert_eq!(retained_feeds.len(), 1);
        assert_eq!(retained_feeds[0].id, "bread");
        assert_eq!(retained_feeds[0].author.as_deref(), Some("Test Author"));
        let articles = storage.list_articles().await.unwrap();
        assert_eq!(articles.len(), 1);
        assert_eq!(articles[0].article.id, "bread::starter");
        let astronomy_articles: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM articles WHERE feed_id = 'astronomy'")
                .fetch_one(&storage.pool)
                .await
                .unwrap();
        assert_eq!(astronomy_articles, 0);
    }

    #[tokio::test]
    async fn delete_feed_accepts_inactive_feeds_and_reports_missing_ids() {
        let storage = storage_with_feed().await;
        storage.set_feed_active("astronomy", false).await.unwrap();

        assert_eq!(
            storage.delete_feed("missing").await.unwrap_err(),
            SubscriptionError::NotFound("missing".to_string())
        );
        assert_eq!(storage.list_feeds().await.unwrap().len(), 1);

        let result = storage.delete_feed("astronomy").await.unwrap();
        assert_eq!(result.deleted_articles, 0);
        assert!(storage.list_feeds().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn delete_feed_rolls_back_article_deletion_when_feed_deletion_fails() {
        let storage = storage_with_feed().await;
        storage
            .upsert_articles(&[article("astronomy::mars", "astronomy", None)])
            .await
            .unwrap();
        storage.enable_sync().await.unwrap();
        let before = storage.sync_identity().await.unwrap();
        sqlx::query(
            r#"
                CREATE TRIGGER reject_feed_deletion
                BEFORE DELETE ON feeds
                BEGIN
                    SELECT RAISE(ABORT, 'simulated deletion failure');
                END
            "#,
        )
        .execute(&storage.pool)
        .await
        .unwrap();

        assert!(matches!(
            storage.delete_feed("astronomy").await.unwrap_err(),
            SubscriptionError::Database(message) if message.contains("simulated deletion failure")
        ));
        assert_eq!(storage.list_feeds().await.unwrap().len(), 1);
        assert_eq!(storage.list_articles().await.unwrap().len(), 1);
        let after = storage.sync_identity().await.unwrap();
        assert_eq!(after.next_sequence, before.next_sequence);
        assert_eq!(after.clock, before.clock);
        let tombstones: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sync_tombstones WHERE entity_kind = 'subscription'",
        )
        .fetch_one(&storage.pool)
        .await
        .unwrap();
        assert_eq!(tombstones, 0);
    }

    #[tokio::test]
    async fn deleted_feed_url_can_be_added_again_with_a_new_uuid() {
        let storage = Storage::open_in_memory().await.unwrap();
        let original = storage
            .add_feed("https://letters.substack.com/feed", None)
            .await
            .unwrap();

        storage.delete_feed(&original.id).await.unwrap();
        let replacement = storage
            .add_feed("https://letters.substack.com/feed", None)
            .await
            .unwrap();

        assert_ne!(replacement.id, original.id);
        assert_eq!(replacement.url, original.url);
    }

    /// Verifies a failed import rolls back its preliminary deactivation.
    #[tokio::test]
    async fn import_feeds_is_atomic() {
        let storage = Storage::open_in_memory().await.unwrap();
        let original = feed(
            "astronomy",
            Platform::Substack,
            "https://astronomy.example/feed",
        );
        storage
            .import_feeds(std::slice::from_ref(&original))
            .await
            .unwrap();

        let duplicate_url = vec![
            feed("one", Platform::Other, "https://duplicate.example/feed"),
            feed("two", Platform::Other, "https://duplicate.example/feed"),
        ];
        assert!(storage.import_feeds(&duplicate_url).await.is_err());

        assert_eq!(
            storage.list_feeds().await.unwrap(),
            vec![stored_feed(
                &original.id,
                original.platform,
                &original.url,
                true,
            )]
        );
    }

    /// Verifies every remote and local article field can be read after insertion.
    #[tokio::test]
    async fn upsert_articles_inserts_and_round_trips_article() {
        let storage = storage_with_feed().await;
        let expected = article(
            "astronomy::jupiter",
            "astronomy",
            Some(Utc.with_ymd_and_hms(2026, 8, 8, 10, 30, 0).unwrap()),
        );

        storage
            .upsert_articles(std::slice::from_ref(&expected))
            .await
            .unwrap();

        assert_eq!(
            storage.list_articles().await.unwrap(),
            vec![StoredArticle {
                article: expected.clone(),
                is_read: false,
                is_favorite: false,
            }]
        );
        let entry_key: String = sqlx::query_scalar("SELECT entry_key FROM articles WHERE id = ?")
            .bind(&expected.id)
            .fetch_one(&storage.pool)
            .await
            .unwrap();
        assert_eq!(entry_key, "jupiter");
    }

    /// Verifies repeated collection updates one row without erasing richer old values.
    #[tokio::test]
    async fn upsert_articles_updates_without_duplicates_or_data_loss() {
        let storage = storage_with_feed().await;
        let original = article("astronomy::jupiter", "astronomy", None);
        storage
            .upsert_articles(std::slice::from_ref(&original))
            .await
            .unwrap();

        let update = Article {
            title: Some("Jupiter after opposition".to_string()),
            author: None,
            url: None,
            content: None,
            ..original.clone()
        };
        let stats = storage.upsert_articles(&[update]).await.unwrap();

        let stored = storage.list_articles().await.unwrap();
        assert_eq!(
            stats,
            UpsertStats {
                inserted: 0,
                updated: 1,
            }
        );
        assert_eq!(stored.len(), 1);
        assert_eq!(
            stored[0].article.title.as_deref(),
            Some("Jupiter after opposition")
        );
        assert_eq!(stored[0].article.author, original.author);
        assert_eq!(stored[0].article.url, original.url);
        assert_eq!(stored[0].article.content, original.content);
        assert_eq!(stored[0].article.content_kind, ContentKind::Full);
    }

    #[tokio::test]
    async fn upsert_replaces_unknown_content_kind_after_refresh() {
        let storage = storage_with_feed().await;
        let mut legacy = article("astronomy::legacy", "astronomy", None);
        legacy.content_kind = ContentKind::Unknown;
        storage.upsert_articles(&[legacy.clone()]).await.unwrap();
        let refreshed = Article {
            content: Some("A complete refreshed body".to_string()),
            content_kind: ContentKind::Full,
            ..legacy
        };

        storage.upsert_articles(&[refreshed]).await.unwrap();

        let stored = storage
            .get_article("astronomy::legacy")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.article.content_kind, ContentKind::Full);
    }

    #[tokio::test]
    async fn upsert_preserves_extracted_content_until_a_full_body_arrives() {
        let storage = storage_with_feed().await;
        let mut extracted = article("astronomy::extracted", "astronomy", None);
        extracted.content = Some("Extracted web body".to_string());
        extracted.content_kind = ContentKind::Extracted;
        storage.upsert_articles(&[extracted.clone()]).await.unwrap();

        let excerpt = Article {
            content: Some("Short RSS excerpt".to_string()),
            content_kind: ContentKind::Excerpt,
            ..extracted.clone()
        };
        storage.upsert_articles(&[excerpt]).await.unwrap();
        let stored = storage.get_article(&extracted.id).await.unwrap().unwrap();
        assert_eq!(
            stored.article.content.as_deref(),
            Some("Extracted web body")
        );
        assert_eq!(stored.article.content_kind, ContentKind::Extracted);

        let full = Article {
            content: Some("Publisher supplied full body".to_string()),
            content_kind: ContentKind::Full,
            ..extracted
        };
        storage.upsert_articles(&[full]).await.unwrap();
        let stored = storage
            .get_article("astronomy::extracted")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored.article.content.as_deref(),
            Some("Publisher supplied full body")
        );
        assert_eq!(stored.article.content_kind, ContentKind::Full);
    }

    #[tokio::test]
    async fn extraction_candidates_respect_platform_state_kind_archive_retry_and_limit() {
        let storage = Storage::open_in_memory().await.unwrap();
        storage
            .import_feeds(&[
                feed("other", Platform::Other, "https://other.example/feed"),
                feed("medium", Platform::Medium, "https://medium.example/feed"),
                feed("inactive", Platform::Other, "https://inactive.example/feed"),
            ])
            .await
            .unwrap();
        storage.set_feed_active("inactive", false).await.unwrap();
        let now = Utc.with_ymd_and_hms(2026, 8, 16, 12, 0, 0).unwrap();
        let mut articles = Vec::new();
        for index in 0..=MAX_EXTRACTION_ATTEMPTS_PER_REFRESH {
            let mut candidate = article(
                &format!("other::{index:02}"),
                "other",
                Some(now - chrono::Duration::minutes(index as i64)),
            );
            candidate.source = Source::Other;
            if index == MAX_EXTRACTION_ATTEMPTS_PER_REFRESH {
                candidate.content = None;
                candidate.content_kind = ContentKind::Missing;
            } else {
                candidate.content_kind = ContentKind::Excerpt;
            }
            articles.push(candidate);
        }
        for (id, feed_id, source, kind) in [
            (
                "medium::article",
                "medium",
                Source::Medium,
                ContentKind::Excerpt,
            ),
            (
                "inactive::article",
                "inactive",
                Source::Other,
                ContentKind::Missing,
            ),
            ("other::full", "other", Source::Other, ContentKind::Full),
        ] {
            let mut excluded = article(id, feed_id, Some(now));
            excluded.source = source;
            excluded.content_kind = kind;
            articles.push(excluded);
        }
        let mut archived = article("other::archived", "other", Some(now));
        archived.source = Source::Other;
        archived.content_kind = ContentKind::Excerpt;
        articles.push(archived);
        let mut without_url = article("other::without-url", "other", Some(now));
        without_url.source = Source::Other;
        without_url.content_kind = ContentKind::Missing;
        without_url.content = None;
        without_url.url = None;
        articles.push(without_url);
        storage.upsert_articles(&articles).await.unwrap();
        storage
            .archive_article("other::archived", now)
            .await
            .unwrap();

        let selection = storage.extraction_candidates(now).await.unwrap();
        assert_eq!(
            selection.candidates.len(),
            MAX_EXTRACTION_ATTEMPTS_PER_REFRESH
        );
        assert_eq!(selection.candidates[0].article_id, "other::00");
        assert_eq!(selection.candidates[19].article_id, "other::19");
        assert_eq!(selection.skipped, 1);

        storage
            .record_extraction_failure(
                "other::00",
                "https://articles.example/other::00",
                "temporary failure",
                now,
            )
            .await
            .unwrap();
        let cooldown = storage.extraction_candidates(now).await.unwrap();
        assert!(
            !cooldown
                .candidates
                .iter()
                .any(|candidate| candidate.article_id == "other::00")
        );
        assert_eq!(cooldown.skipped, 1);

        sqlx::query("UPDATE articles SET url = ? WHERE id = 'other::00'")
            .bind("https://articles.example/changed")
            .execute(&storage.pool)
            .await
            .unwrap();
        let changed = storage.extraction_candidates(now).await.unwrap();
        assert_eq!(changed.candidates[0].article_id, "other::00");

        sqlx::query("UPDATE articles SET url = ? WHERE id = 'other::00'")
            .bind("https://articles.example/other::00")
            .execute(&storage.pool)
            .await
            .unwrap();
        let retried = storage
            .extraction_candidates(now + chrono::Duration::days(EXTRACTION_RETRY_DAYS))
            .await
            .unwrap();
        assert_eq!(retried.candidates[0].article_id, "other::00");
    }

    #[tokio::test]
    async fn extraction_results_preserve_fallbacks_and_local_state() {
        let storage = Storage::open_in_memory().await.unwrap();
        storage
            .import_feeds(&[feed("other", Platform::Other, "https://other.example/feed")])
            .await
            .unwrap();
        let now = Utc.with_ymd_and_hms(2026, 8, 16, 12, 0, 0).unwrap();
        let mut success = article("other::success", "other", Some(now));
        success.source = Source::Other;
        success.content_kind = ContentKind::Excerpt;
        success.content = Some("RSS fallback".to_string());
        let mut failure = success.clone();
        failure.id = "other::failure".to_string();
        failure.url = Some("https://articles.example/failure".to_string());
        storage.upsert_articles(&[success, failure]).await.unwrap();
        storage.set_read("other::success", true).await.unwrap();
        storage.set_favorite("other::success", true).await.unwrap();

        assert!(
            storage
                .record_extraction_success(
                    "other::success",
                    "https://articles.example/other::success",
                    "<p>Complete extracted body</p>",
                    now,
                )
                .await
                .unwrap()
        );
        assert!(
            storage
                .record_extraction_failure(
                    "other::failure",
                    "https://articles.example/failure",
                    &"x".repeat(1_500),
                    now,
                )
                .await
                .unwrap()
        );

        let success = storage
            .get_article("other::success")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(success.article.content_kind, ContentKind::Extracted);
        assert_eq!(
            success.article.content.as_deref(),
            Some("<p>Complete extracted body</p>")
        );
        assert!(success.is_read);
        assert!(success.is_favorite);
        let failure = storage
            .get_article("other::failure")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(failure.article.content_kind, ContentKind::Excerpt);
        assert_eq!(failure.article.content.as_deref(), Some("RSS fallback"));
        let attempt: (i64, Option<String>, Option<DateTime<Utc>>) = sqlx::query_as(
            "SELECT extraction_attempt_count, extraction_last_error, extraction_attempted_at FROM articles WHERE id = 'other::failure'",
        )
        .fetch_one(&storage.pool)
        .await
        .unwrap();
        assert_eq!(attempt.0, 1);
        assert_eq!(attempt.1.unwrap().chars().count(), 1_000);
        assert_eq!(attempt.2, Some(now));
    }

    /// Verifies an article batch reports inserted and updated rows independently.
    #[tokio::test]
    async fn upsert_articles_reports_insert_and_update_counts() {
        let storage = storage_with_feed().await;
        let existing = article("astronomy::existing", "astronomy", None);
        storage
            .upsert_articles(std::slice::from_ref(&existing))
            .await
            .unwrap();
        let new = article("astronomy::new", "astronomy", None);

        let stats = storage.upsert_articles(&[existing, new]).await.unwrap();

        assert_eq!(
            stats,
            UpsertStats {
                inserted: 1,
                updated: 1,
            }
        );
    }

    /// Verifies dated articles are newest-first and undated articles are last.
    #[tokio::test]
    async fn list_articles_orders_newest_first_with_undated_last() {
        let storage = storage_with_feed().await;
        let older = article(
            "astronomy::older",
            "astronomy",
            Some(Utc.with_ymd_and_hms(2026, 8, 6, 12, 0, 0).unwrap()),
        );
        let newer = article(
            "astronomy::newer",
            "astronomy",
            Some(Utc.with_ymd_and_hms(2026, 8, 8, 12, 0, 0).unwrap()),
        );
        let undated = article("astronomy::undated", "astronomy", None);

        storage
            .upsert_articles(&[older, undated, newer])
            .await
            .unwrap();

        let ids: Vec<String> = storage
            .list_articles()
            .await
            .unwrap()
            .into_iter()
            .map(|stored| stored.article.id)
            .collect();
        assert_eq!(
            ids,
            ["astronomy::newer", "astronomy::older", "astronomy::undated"]
        );
    }

    /// Verifies the lightweight timeline preserves metadata, state, and ordering.
    #[tokio::test]
    async fn list_article_summaries_returns_metadata_without_article_bodies() {
        let storage = storage_with_feed().await;
        let older = article(
            "astronomy::older",
            "astronomy",
            Some(Utc.with_ymd_and_hms(2026, 8, 6, 12, 0, 0).unwrap()),
        );
        let newer = article(
            "astronomy::newer",
            "astronomy",
            Some(Utc.with_ymd_and_hms(2026, 8, 8, 12, 0, 0).unwrap()),
        );
        storage
            .upsert_articles(&[older, newer.clone()])
            .await
            .unwrap();
        storage.set_read(&newer.id, true).await.unwrap();
        storage.set_favorite(&newer.id, true).await.unwrap();

        let summaries = storage.list_article_summaries().await.unwrap();

        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].id, newer.id);
        assert_eq!(summaries[0].title, newer.title);
        assert_eq!(summaries[0].author, newer.author);
        assert_eq!(summaries[0].published_at, newer.published_at);
        assert_eq!(summaries[0].url, newer.url);
        assert!(summaries[0].is_read);
        assert!(summaries[0].is_favorite);
        assert_eq!(summaries[1].id, "astronomy::older");
    }

    /// Verifies full article detail is loaded by ID and missing IDs return `None`.
    #[tokio::test]
    async fn get_article_returns_full_detail_or_none() {
        let storage = storage_with_feed().await;
        let expected = article("astronomy::jupiter", "astronomy", None);
        storage
            .upsert_articles(std::slice::from_ref(&expected))
            .await
            .unwrap();
        storage.set_favorite(&expected.id, true).await.unwrap();

        let stored = storage.get_article(&expected.id).await.unwrap().unwrap();

        assert_eq!(stored.article, expected);
        assert!(!stored.is_read);
        assert!(stored.is_favorite);
        assert!(storage.get_article("missing").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn manual_archive_hides_article_releases_content_and_blocks_reimport() {
        let storage = storage_with_feed().await;
        let original = article(
            "astronomy::jupiter",
            "astronomy",
            Some(Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap()),
        );
        storage
            .upsert_articles(std::slice::from_ref(&original))
            .await
            .unwrap();
        storage.set_read(&original.id, true).await.unwrap();
        let archived_at = Utc.with_ymd_and_hms(2026, 8, 14, 12, 0, 0).unwrap();

        assert!(
            storage
                .archive_article(&original.id, archived_at)
                .await
                .unwrap()
        );
        assert!(
            !storage
                .archive_article(&original.id, archived_at)
                .await
                .unwrap()
        );
        assert!(storage.list_articles().await.unwrap().is_empty());
        assert!(storage.list_article_summaries().await.unwrap().is_empty());
        assert!(storage.get_article(&original.id).await.unwrap().is_none());
        assert!(!storage.set_read(&original.id, false).await.unwrap());
        assert!(!storage.set_favorite(&original.id, true).await.unwrap());

        type TombstoneRow = (
            bool,
            Option<DateTime<Utc>>,
            Option<String>,
            Option<String>,
            String,
            Option<String>,
        );
        let tombstone: TombstoneRow = sqlx::query_as(
            r#"
                SELECT is_archived, archived_at, archive_reason, content,
                       content_kind, title
                FROM articles WHERE id = ?
            "#,
        )
        .bind(&original.id)
        .fetch_one(&storage.pool)
        .await
        .unwrap();
        assert_eq!(
            tombstone,
            (
                true,
                Some(archived_at),
                Some("manual".to_string()),
                None,
                "missing".to_string(),
                original.title.clone(),
            )
        );

        let stats = storage.upsert_articles(&[original]).await.unwrap();
        assert_eq!(stats, UpsertStats::default());
        let content: Option<String> =
            sqlx::query_scalar("SELECT content FROM articles WHERE id = 'astronomy::jupiter'")
                .fetch_one(&storage.pool)
                .await
                .unwrap();
        assert!(content.is_none());
    }

    #[tokio::test]
    async fn retention_archives_only_old_read_non_favorite_dated_articles() {
        let storage = storage_with_feed().await;
        let now = Utc.with_ymd_and_hms(2026, 8, 14, 12, 0, 0).unwrap();
        let cutoff = now - chrono::Duration::days(ARTICLE_RETENTION_DAYS);
        let old = cutoff - chrono::Duration::seconds(1);
        let recent = cutoff + chrono::Duration::seconds(1);
        let articles = [
            article("astronomy::eligible", "astronomy", Some(old)),
            article("astronomy::unread", "astronomy", Some(old)),
            article("astronomy::favorite", "astronomy", Some(old)),
            article("astronomy::recent", "astronomy", Some(recent)),
            article("astronomy::undated", "astronomy", None),
            article("astronomy::boundary", "astronomy", Some(cutoff)),
        ];
        storage.upsert_articles(&articles).await.unwrap();
        for id in [
            "astronomy::eligible",
            "astronomy::favorite",
            "astronomy::recent",
            "astronomy::undated",
            "astronomy::boundary",
        ] {
            storage.set_read(id, true).await.unwrap();
        }
        storage
            .set_favorite("astronomy::favorite", true)
            .await
            .unwrap();

        assert_eq!(storage.archive_expired_read_articles(now).await.unwrap(), 1);
        let visible_ids: Vec<String> = storage
            .list_article_summaries()
            .await
            .unwrap()
            .into_iter()
            .map(|article| article.id)
            .collect();
        assert!(!visible_ids.contains(&"astronomy::eligible".to_string()));
        for retained in ["unread", "favorite", "recent", "undated", "boundary"] {
            assert!(visible_ids.contains(&format!("astronomy::{retained}")));
        }

        let archived: (bool, Option<String>, Option<String>, String) = sqlx::query_as(
            "SELECT is_archived, archive_reason, content, content_kind FROM articles WHERE id = 'astronomy::eligible'",
        )
        .fetch_one(&storage.pool)
        .await
        .unwrap();
        assert_eq!(
            archived,
            (
                true,
                Some("retention".to_string()),
                None,
                "missing".to_string()
            )
        );
        assert_eq!(storage.archive_expired_read_articles(now).await.unwrap(), 0);
    }

    /// Verifies article batches are atomic when one row references an unknown feed.
    #[tokio::test]
    async fn upsert_articles_is_atomic() {
        let storage = storage_with_feed().await;
        let valid = article("astronomy::valid", "astronomy", None);
        let invalid = article("missing::invalid", "missing", None);

        assert!(storage.upsert_articles(&[valid, invalid]).await.is_err());
        assert!(storage.list_articles().await.unwrap().is_empty());
    }

    /// Verifies read state can be enabled, disabled, and detects an unknown article.
    #[tokio::test]
    async fn set_read_updates_existing_article_only() {
        let storage = storage_with_feed().await;
        let article = article("astronomy::jupiter", "astronomy", None);
        storage.upsert_articles(&[article]).await.unwrap();

        assert!(storage.set_read("astronomy::jupiter", true).await.unwrap());
        assert!(storage.list_articles().await.unwrap()[0].is_read);

        assert!(storage.set_read("astronomy::jupiter", false).await.unwrap());
        assert!(!storage.list_articles().await.unwrap()[0].is_read);
        assert!(!storage.set_read("missing", true).await.unwrap());
    }

    #[tokio::test]
    async fn set_read_many_is_atomic_and_ignores_duplicate_ids() {
        let storage = storage_with_feed().await;
        let first = article("astronomy::jupiter", "astronomy", None);
        let second = article("astronomy::saturn", "astronomy", None);
        storage.upsert_articles(&[first, second]).await.unwrap();

        assert!(
            storage
                .set_read_many(
                    &[
                        "astronomy::jupiter".to_string(),
                        "astronomy::jupiter".to_string(),
                        "astronomy::saturn".to_string(),
                    ],
                    true,
                )
                .await
                .unwrap()
        );
        assert!(
            storage
                .list_articles()
                .await
                .unwrap()
                .iter()
                .all(|article| article.is_read)
        );

        assert!(
            !storage
                .set_read_many(
                    &["astronomy::jupiter".to_string(), "missing".to_string()],
                    false,
                )
                .await
                .unwrap()
        );
        assert!(
            storage
                .list_articles()
                .await
                .unwrap()
                .iter()
                .all(|article| article.is_read)
        );
        assert!(!storage.set_read_many(&[], false).await.unwrap());
    }

    #[tokio::test]
    async fn archive_articles_now_is_atomic_and_releases_content() {
        let storage = storage_with_feed().await;
        let first = article("astronomy::jupiter", "astronomy", None);
        let second = article("astronomy::saturn", "astronomy", None);
        storage.upsert_articles(&[first, second]).await.unwrap();

        assert!(
            !storage
                .archive_articles_now(&["astronomy::jupiter".to_string(), "missing".to_string(),])
                .await
                .unwrap()
        );
        assert_eq!(storage.list_articles().await.unwrap().len(), 2);

        assert!(
            storage
                .archive_articles_now(&[
                    "astronomy::jupiter".to_string(),
                    "astronomy::saturn".to_string(),
                    "astronomy::saturn".to_string(),
                ])
                .await
                .unwrap()
        );
        assert!(storage.list_articles().await.unwrap().is_empty());
        for id in ["astronomy::jupiter", "astronomy::saturn"] {
            let row: (bool, Option<String>, Option<String>, String) = sqlx::query_as(
                "SELECT is_archived, archive_reason, content, content_kind FROM articles WHERE id = ?",
            )
            .bind(id)
            .fetch_one(&storage.pool)
            .await
            .unwrap();
            assert_eq!(
                row,
                (
                    true,
                    Some("manual".to_string()),
                    None,
                    "missing".to_string()
                )
            );
        }
        assert!(!storage.archive_articles_now(&[]).await.unwrap());
    }

    /// Verifies favorite state can be enabled, disabled, and detects an unknown article.
    #[tokio::test]
    async fn set_favorite_updates_existing_article_only() {
        let storage = storage_with_feed().await;
        let article = article("astronomy::jupiter", "astronomy", None);
        storage.upsert_articles(&[article]).await.unwrap();

        assert!(
            storage
                .set_favorite("astronomy::jupiter", true)
                .await
                .unwrap()
        );
        assert!(storage.list_articles().await.unwrap()[0].is_favorite);

        assert!(
            storage
                .set_favorite("astronomy::jupiter", false)
                .await
                .unwrap()
        );
        assert!(!storage.list_articles().await.unwrap()[0].is_favorite);
        assert!(!storage.set_favorite("missing", true).await.unwrap());
    }

    /// Verifies refreshed remote data never resets either local state flag.
    #[tokio::test]
    async fn upsert_articles_preserves_read_and_favorite_states() {
        let storage = storage_with_feed().await;
        let original = article("astronomy::jupiter", "astronomy", None);
        storage
            .upsert_articles(std::slice::from_ref(&original))
            .await
            .unwrap();
        storage.set_read("astronomy::jupiter", true).await.unwrap();
        storage
            .set_favorite("astronomy::jupiter", true)
            .await
            .unwrap();

        let refreshed = Article {
            title: Some("A refreshed title".to_string()),
            ..original
        };
        storage.upsert_articles(&[refreshed]).await.unwrap();

        let stored = &storage.list_articles().await.unwrap()[0];
        assert_eq!(stored.article.title.as_deref(), Some("A refreshed title"));
        assert!(stored.is_read);
        assert!(stored.is_favorite);
    }
}
