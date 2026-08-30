use base64::Engine;
use inkriver::config::Platform;
use inkriver::refresh::{self, RefreshReport};
use inkriver::storage::{
    ArticleSummary, DeleteFeedResult, Storage, StoredArticle, StoredFeed, StoredSyncReport,
    StoredSyncRuntimeError, SubscriptionError, SyncDevice,
};
use inkriver::sync_diagnostics::export_sync_diagnostic_json;
use inkriver::sync_pairing::{
    accept_pairing_invitation, configure_new_sync_group, create_pairing_invitation,
    encode_pairing_invitation, render_pairing_qr_svg,
};
use inkriver::sync_runtime;
use inkriver::sync_secrets::{PlatformSyncSecretStore, SyncSecretStore};
use inkriver::sync_transport::SyncTransportReport;
use serde::Serialize;
use std::path::Path;
use tauri::{Manager, State};
use tokio::sync::{Mutex, MutexGuard};

const DATABASE_FILE_NAME: &str = "inkriver.db";

pub struct AppState {
    storage: Storage,
    refresh_lock: Mutex<()>,
    sync_lock: Mutex<()>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

impl ApiError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }

    fn storage(error: impl std::fmt::Display) -> Self {
        Self::new("storage", error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArticleSummaryDto {
    pub id: String,
    pub feed_id: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub published_at: Option<String>,
    pub url: Option<String>,
    pub source: String,
    pub is_read: bool,
    pub is_favorite: bool,
}

impl From<ArticleSummary> for ArticleSummaryDto {
    fn from(summary: ArticleSummary) -> Self {
        Self {
            id: summary.id,
            feed_id: summary.feed_id,
            title: summary.title,
            author: summary.author,
            published_at: summary.published_at.map(|date| date.to_rfc3339()),
            url: summary.url,
            source: summary.source.as_str().to_string(),
            is_read: summary.is_read,
            is_favorite: summary.is_favorite,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArticleDetailDto {
    pub id: String,
    pub feed_id: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub published_at: Option<String>,
    pub url: Option<String>,
    pub content: Option<String>,
    pub content_kind: String,
    pub source: String,
    pub is_read: bool,
    pub is_favorite: bool,
}

impl From<StoredArticle> for ArticleDetailDto {
    fn from(stored: StoredArticle) -> Self {
        Self {
            id: stored.article.id,
            feed_id: stored.article.feed_id,
            title: stored.article.title,
            author: stored.article.author,
            published_at: stored.article.published_at.map(|date| date.to_rfc3339()),
            url: stored.article.url,
            content: stored.article.content,
            content_kind: stored.article.content_kind.as_str().to_string(),
            source: stored.article.source.as_str().to_string(),
            is_read: stored.is_read,
            is_favorite: stored.is_favorite,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedDto {
    pub id: String,
    pub platform: String,
    pub url: String,
    pub is_active: bool,
    pub title: Option<String>,
    pub description: Option<String>,
    pub author: Option<String>,
    pub last_published_at: Option<String>,
    pub last_success_at: Option<String>,
    pub last_error: Option<StoredFeedErrorDto>,
    pub logo_data_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredFeedErrorDto {
    pub stage: String,
    pub message: String,
    pub occurred_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncDeviceDto {
    pub device_id: String,
    pub display_name: String,
    pub is_local: bool,
    pub revoked_at: Option<String>,
}

impl From<SyncDevice> for SyncDeviceDto {
    fn from(device: SyncDevice) -> Self {
        Self {
            device_id: device.device_id,
            display_name: device.display_name,
            is_local: device.is_local,
            revoked_at: device.revoked_at.map(|date| date.to_rfc3339()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPairingStatusDto {
    pub configured: bool,
    pub webdav_base_url: Option<String>,
    pub webdav_username: Option<String>,
    pub key_id: Option<String>,
    pub devices: Vec<SyncDeviceDto>,
    pub last_attempt_at: Option<String>,
    pub last_success_at: Option<String>,
    pub last_error: Option<SyncRuntimeErrorDto>,
    pub last_report: Option<SyncTransportReportDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRuntimeErrorDto {
    pub stage: String,
    pub message: String,
    pub occurred_at: String,
}

impl From<StoredSyncRuntimeError> for SyncRuntimeErrorDto {
    fn from(error: StoredSyncRuntimeError) -> Self {
        Self {
            stage: error.stage,
            message: error.message,
            occurred_at: error.occurred_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingInvitationDto {
    pub invitation: String,
    pub qr_code_data_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncTransportReportDto {
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

impl From<SyncTransportReport> for SyncTransportReportDto {
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
            compacted_events: report.compacted_events,
            deleted_segments: report.deleted_segments,
            deferred_segment_deletions: report.deferred_segment_deletions,
        }
    }
}

impl From<StoredSyncReport> for SyncTransportReportDto {
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
            compacted_events: report.compacted_events,
            deleted_segments: report.deleted_segments,
            deferred_segment_deletions: report.deferred_segment_deletions,
        }
    }
}

impl From<StoredFeed> for FeedDto {
    fn from(feed: StoredFeed) -> Self {
        Self {
            id: feed.id,
            platform: feed.platform.as_str().to_string(),
            url: feed.url,
            is_active: feed.is_active,
            title: feed.title,
            description: feed.description,
            author: feed.author,
            last_published_at: feed.last_published_at.map(|date| date.to_rfc3339()),
            last_success_at: feed.last_success_at.map(|date| date.to_rfc3339()),
            last_error: feed.last_error.map(|error| StoredFeedErrorDto {
                stage: error.stage,
                message: error.message,
                occurred_at: error.occurred_at.to_rfc3339(),
            }),
            logo_data_url: feed.logo_png.map(|png| {
                format!(
                    "data:image/png;base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(png)
                )
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteFeedResultDto {
    pub feed_id: String,
    pub deleted_articles: usize,
}

impl From<DeleteFeedResult> for DeleteFeedResultDto {
    fn from(result: DeleteFeedResult) -> Self {
        Self {
            feed_id: result.feed_id,
            deleted_articles: result.deleted_articles,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedRefreshErrorDto {
    pub feed_id: String,
    pub feed_url: String,
    pub stage: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshReportDto {
    pub active_feeds: usize,
    pub collected_articles: usize,
    pub inserted_articles: usize,
    pub updated_articles: usize,
    pub auto_archived_articles: usize,
    pub extracted_articles: usize,
    pub extraction_failed_articles: usize,
    pub extraction_skipped_articles: usize,
    pub errors: Vec<FeedRefreshErrorDto>,
}

impl From<RefreshReport> for RefreshReportDto {
    fn from(report: RefreshReport) -> Self {
        Self {
            active_feeds: report.active_feeds,
            collected_articles: report.collected_articles,
            inserted_articles: report.inserted_articles,
            updated_articles: report.updated_articles,
            auto_archived_articles: report.auto_archived_articles,
            extracted_articles: report.extracted_articles,
            extraction_failed_articles: report.extraction_failed_articles,
            extraction_skipped_articles: report.extraction_skipped_articles,
            errors: report
                .errors
                .into_iter()
                .map(|error| FeedRefreshErrorDto {
                    feed_id: error.feed_id,
                    feed_url: error.feed_url,
                    stage: error.error.stage.to_string(),
                    message: error.error.message,
                })
                .collect(),
        }
    }
}

fn parse_platform(platform: Option<&str>) -> Result<Option<Platform>, ApiError> {
    platform
        .map(|value| {
            Platform::try_from(value).map_err(|message| ApiError::new("invalid_platform", message))
        })
        .transpose()
}

fn subscription_error(error: SubscriptionError) -> ApiError {
    match error {
        SubscriptionError::InvalidUrl(error) => ApiError::new("invalid_url", error.to_string()),
        SubscriptionError::DuplicateActiveUrl(url) => {
            ApiError::new("duplicate_feed", format!("Ce flux est déjà actif : {url}"))
        }
        SubscriptionError::NotFound(id) => {
            ApiError::new("feed_not_found", format!("Abonnement introuvable : {id}"))
        }
        SubscriptionError::Inactive(id) => ApiError::new(
            "feed_inactive",
            format!("Réactivez l’abonnement avant de l’actualiser : {id}"),
        ),
        SubscriptionError::Database(message) => ApiError::new("storage", message),
    }
}

fn refresh_error(error: anyhow::Error) -> ApiError {
    if let Some(error) = error.downcast_ref::<SubscriptionError>() {
        return match error {
            SubscriptionError::NotFound(id) => {
                ApiError::new("feed_not_found", format!("Abonnement introuvable : {id}"))
            }
            SubscriptionError::Inactive(id) => ApiError::new(
                "feed_inactive",
                format!("Réactivez l’abonnement avant de l’actualiser : {id}"),
            ),
            SubscriptionError::Database(message) => ApiError::new("storage", message),
            SubscriptionError::InvalidUrl(error) => ApiError::new("invalid_url", error.to_string()),
            SubscriptionError::DuplicateActiveUrl(url) => {
                ApiError::new("duplicate_feed", format!("Ce flux est déjà actif : {url}"))
            }
        };
    }
    ApiError::storage(error)
}

fn acquire_refresh_lock(lock: &Mutex<()>) -> Result<MutexGuard<'_, ()>, ApiError> {
    lock.try_lock().map_err(|_| {
        ApiError::new(
            "refresh_in_progress",
            "Une actualisation est déjà en cours.",
        )
    })
}

async fn refresh_feed_from(storage: &Storage, feed_id: &str) -> Result<RefreshReportDto, ApiError> {
    refresh::refresh_feed(storage, feed_id)
        .await
        .map(Into::into)
        .map_err(refresh_error)
}

async fn list_articles_from(storage: &Storage) -> Result<Vec<ArticleSummaryDto>, ApiError> {
    storage
        .list_article_summaries()
        .await
        .map(|articles| articles.into_iter().map(Into::into).collect())
        .map_err(ApiError::storage)
}

async fn get_article_from(
    storage: &Storage,
    article_id: &str,
) -> Result<ArticleDetailDto, ApiError> {
    storage
        .get_article(article_id)
        .await
        .map_err(ApiError::storage)?
        .map(Into::into)
        .ok_or_else(|| {
            ApiError::new(
                "article_not_found",
                format!("Article introuvable : {article_id}"),
            )
        })
}

async fn set_article_read_in(
    storage: &Storage,
    article_id: &str,
    is_read: bool,
) -> Result<(), ApiError> {
    if storage
        .set_read(article_id, is_read)
        .await
        .map_err(ApiError::storage)?
    {
        Ok(())
    } else {
        Err(ApiError::new(
            "article_not_found",
            format!("Article introuvable : {article_id}"),
        ))
    }
}

async fn set_articles_read_in(
    storage: &Storage,
    article_ids: &[String],
    is_read: bool,
) -> Result<(), ApiError> {
    if article_ids.is_empty() {
        return Err(ApiError::new(
            "invalid_request",
            "Sélection d'articles vide",
        ));
    }
    if storage
        .set_read_many(article_ids, is_read)
        .await
        .map_err(ApiError::storage)?
    {
        Ok(())
    } else {
        Err(ApiError::new(
            "article_not_found",
            "Au moins un article est introuvable ou déjà archivé",
        ))
    }
}

async fn set_article_favorite_in(
    storage: &Storage,
    article_id: &str,
    is_favorite: bool,
) -> Result<(), ApiError> {
    if storage
        .set_favorite(article_id, is_favorite)
        .await
        .map_err(ApiError::storage)?
    {
        Ok(())
    } else {
        Err(ApiError::new(
            "article_not_found",
            format!("Article introuvable : {article_id}"),
        ))
    }
}

async fn archive_article_in(storage: &Storage, article_id: &str) -> Result<(), ApiError> {
    if storage
        .archive_article_now(article_id)
        .await
        .map_err(ApiError::storage)?
    {
        Ok(())
    } else {
        Err(ApiError::new(
            "article_not_found",
            format!("Article introuvable : {article_id}"),
        ))
    }
}

async fn archive_articles_in(storage: &Storage, article_ids: &[String]) -> Result<(), ApiError> {
    if article_ids.is_empty() {
        return Err(ApiError::new(
            "invalid_request",
            "Sélection d'articles vide",
        ));
    }
    if storage
        .archive_articles_now(article_ids)
        .await
        .map_err(ApiError::storage)?
    {
        Ok(())
    } else {
        Err(ApiError::new(
            "article_not_found",
            "Au moins un article est introuvable ou déjà archivé",
        ))
    }
}

async fn list_feeds_from(storage: &Storage) -> Result<Vec<FeedDto>, ApiError> {
    storage
        .list_feeds()
        .await
        .map(|feeds| feeds.into_iter().map(Into::into).collect())
        .map_err(ApiError::storage)
}

async fn add_feed_to(
    storage: &Storage,
    url: &str,
    platform: Option<&str>,
) -> Result<FeedDto, ApiError> {
    let platform = parse_platform(platform)?;
    storage
        .add_feed(url, platform)
        .await
        .map(Into::into)
        .map_err(subscription_error)
}

async fn set_feed_active_in(
    storage: &Storage,
    feed_id: &str,
    is_active: bool,
) -> Result<FeedDto, ApiError> {
    storage
        .set_feed_active(feed_id, is_active)
        .await
        .map(Into::into)
        .map_err(subscription_error)
}

async fn delete_feed_from(
    storage: &Storage,
    feed_id: &str,
) -> Result<DeleteFeedResultDto, ApiError> {
    storage
        .delete_feed(feed_id)
        .await
        .map(Into::into)
        .map_err(subscription_error)
}

async fn sync_pairing_status_from(storage: &Storage) -> Result<SyncPairingStatusDto, ApiError> {
    let configuration = storage
        .sync_configuration()
        .await
        .map_err(ApiError::storage)?;
    let devices = storage
        .list_sync_devices()
        .await
        .map_err(ApiError::storage)?
        .into_iter()
        .map(Into::into)
        .collect();
    let runtime = storage
        .sync_runtime_status()
        .await
        .map_err(ApiError::storage)?;
    let last_attempt_at = runtime.last_attempt_at.map(|date| date.to_rfc3339());
    let last_success_at = runtime.last_success_at.map(|date| date.to_rfc3339());
    let last_error = runtime.last_error.map(Into::into);
    let last_report = runtime.last_report.map(Into::into);
    Ok(match configuration {
        Some(configuration) => SyncPairingStatusDto {
            configured: true,
            webdav_base_url: Some(configuration.webdav_base_url),
            webdav_username: Some(configuration.webdav_username),
            key_id: Some(configuration.key_id),
            devices,
            last_attempt_at,
            last_success_at,
            last_error,
            last_report,
        },
        None => SyncPairingStatusDto {
            configured: false,
            webdav_base_url: None,
            webdav_username: None,
            key_id: None,
            devices,
            last_attempt_at,
            last_success_at,
            last_error,
            last_report,
        },
    })
}

async fn export_sync_diagnostic_from(
    storage: &Storage,
    generated_at: chrono::DateTime<chrono::Utc>,
) -> Result<String, ApiError> {
    export_sync_diagnostic_json(storage, generated_at)
        .await
        .map_err(ApiError::storage)
}

fn pairing_error(error: anyhow::Error) -> ApiError {
    let message = error.to_string();
    let code = if message.contains("existe déjà") {
        "sync_already_configured"
    } else if message.contains("appairage") || message.contains("invitation") {
        "invalid_pairing"
    } else if message.contains("coffre")
        || message.contains("Secret Service")
        || message.contains("Keystore")
    {
        "secret_store"
    } else {
        "sync_pairing"
    };
    ApiError::new(code, message)
}

fn platform_secret_store() -> Result<PlatformSyncSecretStore, ApiError> {
    PlatformSyncSecretStore::initialize().map_err(pairing_error)
}

fn sync_runtime_error(error: anyhow::Error) -> ApiError {
    let message = format!("{error:#}");
    let code = if message.contains("n'est pas configurée") {
        "sync_not_configured"
    } else if message.contains("secrets")
        || message.contains("clé sécurisée")
        || message.contains("coffre")
    {
        "sync_secrets"
    } else {
        "sync_failed"
    };
    ApiError::new(code, message)
}

async fn delete_sync_configuration_with<S: SyncSecretStore>(
    storage: &Storage,
    secret_store: &S,
) -> Result<SyncPairingStatusDto, ApiError> {
    sync_runtime::remove_sync_configuration(storage, secret_store)
        .await
        .map_err(sync_runtime_error)?;
    sync_pairing_status_from(storage).await
}

fn acquire_sync_lock(lock: &Mutex<()>) -> Result<MutexGuard<'_, ()>, ApiError> {
    lock.try_lock()
        .map_err(|_| ApiError::new("sync_in_progress", "Une synchronisation est déjà en cours."))
}

async fn configure_sync_group_with<S: SyncSecretStore>(
    storage: &Storage,
    secret_store: &S,
    webdav_base_url: &str,
    webdav_username: &str,
    webdav_password: String,
    device_name: &str,
) -> Result<SyncPairingStatusDto, ApiError> {
    configure_new_sync_group(
        storage,
        secret_store,
        webdav_base_url,
        webdav_username,
        webdav_password,
        device_name,
        chrono::Utc::now(),
    )
    .await
    .map_err(pairing_error)?;
    sync_pairing_status_from(storage).await
}

async fn create_pairing_invitation_with<S: SyncSecretStore>(
    storage: &Storage,
    secret_store: &S,
) -> Result<PairingInvitationDto, ApiError> {
    let invitation = create_pairing_invitation(storage, secret_store)
        .await
        .map_err(pairing_error)?;
    let encoded = encode_pairing_invitation(&invitation).map_err(pairing_error)?;
    let svg = render_pairing_qr_svg(&encoded).map_err(pairing_error)?;
    Ok(PairingInvitationDto {
        invitation: encoded,
        qr_code_data_url: format!(
            "data:image/svg+xml;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(svg)
        ),
    })
}

async fn accept_pairing_invitation_with<S: SyncSecretStore>(
    storage: &Storage,
    secret_store: &S,
    invitation: &str,
    webdav_password: String,
    device_name: &str,
) -> Result<SyncPairingStatusDto, ApiError> {
    accept_pairing_invitation(
        storage,
        secret_store,
        invitation,
        webdav_password,
        device_name,
        chrono::Utc::now(),
    )
    .await
    .map_err(pairing_error)?;
    sync_pairing_status_from(storage).await
}

#[tauri::command]
async fn list_articles(state: State<'_, AppState>) -> Result<Vec<ArticleSummaryDto>, ApiError> {
    list_articles_from(&state.storage).await
}

#[tauri::command]
async fn get_article(
    state: State<'_, AppState>,
    article_id: String,
) -> Result<ArticleDetailDto, ApiError> {
    get_article_from(&state.storage, &article_id).await
}

#[tauri::command]
async fn refresh_feeds(state: State<'_, AppState>) -> Result<RefreshReportDto, ApiError> {
    let _guard = acquire_refresh_lock(&state.refresh_lock)?;
    refresh::refresh_active(&state.storage)
        .await
        .map(Into::into)
        .map_err(ApiError::storage)
}

#[tauri::command]
async fn refresh_feed(
    state: State<'_, AppState>,
    feed_id: String,
) -> Result<RefreshReportDto, ApiError> {
    let _guard = acquire_refresh_lock(&state.refresh_lock)?;
    refresh_feed_from(&state.storage, &feed_id).await
}

#[tauri::command]
async fn set_article_read(
    state: State<'_, AppState>,
    article_id: String,
    is_read: bool,
) -> Result<(), ApiError> {
    set_article_read_in(&state.storage, &article_id, is_read).await
}

#[tauri::command]
async fn set_articles_read(
    state: State<'_, AppState>,
    article_ids: Vec<String>,
    is_read: bool,
) -> Result<(), ApiError> {
    set_articles_read_in(&state.storage, &article_ids, is_read).await
}

#[tauri::command]
async fn set_article_favorite(
    state: State<'_, AppState>,
    article_id: String,
    is_favorite: bool,
) -> Result<(), ApiError> {
    set_article_favorite_in(&state.storage, &article_id, is_favorite).await
}

#[tauri::command]
async fn archive_article(state: State<'_, AppState>, article_id: String) -> Result<(), ApiError> {
    archive_article_in(&state.storage, &article_id).await
}

#[tauri::command]
async fn archive_articles(
    state: State<'_, AppState>,
    article_ids: Vec<String>,
) -> Result<(), ApiError> {
    archive_articles_in(&state.storage, &article_ids).await
}

#[tauri::command]
async fn list_feeds(state: State<'_, AppState>) -> Result<Vec<FeedDto>, ApiError> {
    list_feeds_from(&state.storage).await
}

#[tauri::command]
async fn add_feed(
    state: State<'_, AppState>,
    url: String,
    platform: Option<String>,
) -> Result<FeedDto, ApiError> {
    add_feed_to(&state.storage, &url, platform.as_deref()).await
}

#[tauri::command]
async fn set_feed_active(
    state: State<'_, AppState>,
    feed_id: String,
    is_active: bool,
) -> Result<FeedDto, ApiError> {
    set_feed_active_in(&state.storage, &feed_id, is_active).await
}

#[tauri::command]
async fn delete_feed(
    state: State<'_, AppState>,
    feed_id: String,
) -> Result<DeleteFeedResultDto, ApiError> {
    delete_feed_from(&state.storage, &feed_id).await
}

#[tauri::command]
async fn sync_pairing_status(state: State<'_, AppState>) -> Result<SyncPairingStatusDto, ApiError> {
    sync_pairing_status_from(&state.storage).await
}

#[tauri::command]
async fn configure_sync_group(
    state: State<'_, AppState>,
    webdav_base_url: String,
    webdav_username: String,
    webdav_password: String,
    device_name: String,
) -> Result<SyncPairingStatusDto, ApiError> {
    let secret_store = platform_secret_store()?;
    configure_sync_group_with(
        &state.storage,
        &secret_store,
        &webdav_base_url,
        &webdav_username,
        webdav_password,
        &device_name,
    )
    .await
}

#[tauri::command]
async fn pairing_invitation(state: State<'_, AppState>) -> Result<PairingInvitationDto, ApiError> {
    let secret_store = platform_secret_store()?;
    create_pairing_invitation_with(&state.storage, &secret_store).await
}

#[tauri::command]
async fn join_sync_group(
    state: State<'_, AppState>,
    invitation: String,
    webdav_password: String,
    device_name: String,
) -> Result<SyncPairingStatusDto, ApiError> {
    let secret_store = platform_secret_store()?;
    accept_pairing_invitation_with(
        &state.storage,
        &secret_store,
        &invitation,
        webdav_password,
        &device_name,
    )
    .await
}

#[tauri::command]
async fn rename_sync_device(
    state: State<'_, AppState>,
    device_id: String,
    display_name: String,
) -> Result<SyncPairingStatusDto, ApiError> {
    if !state
        .storage
        .rename_sync_device(&device_id, &display_name, chrono::Utc::now())
        .await
        .map_err(ApiError::storage)?
    {
        return Err(ApiError::new(
            "sync_device_not_found",
            "Appareil de synchronisation introuvable",
        ));
    }
    sync_pairing_status_from(&state.storage).await
}

#[tauri::command]
async fn revoke_sync_device(
    state: State<'_, AppState>,
    device_id: String,
) -> Result<SyncPairingStatusDto, ApiError> {
    if !state
        .storage
        .revoke_sync_device(&device_id, chrono::Utc::now())
        .await
        .map_err(ApiError::storage)?
    {
        return Err(ApiError::new(
            "sync_device_not_revocable",
            "Cet appareil est local, introuvable ou déjà révoqué",
        ));
    }
    sync_pairing_status_from(&state.storage).await
}

#[tauri::command]
async fn synchronize_now(state: State<'_, AppState>) -> Result<SyncTransportReportDto, ApiError> {
    let _guard = acquire_sync_lock(&state.sync_lock)?;
    let secret_store = platform_secret_store()?;
    sync_runtime::synchronize_configured(&state.storage, &secret_store, chrono::Utc::now())
        .await
        .map(Into::into)
        .map_err(sync_runtime_error)
}

#[tauri::command]
async fn export_sync_diagnostic(state: State<'_, AppState>) -> Result<String, ApiError> {
    export_sync_diagnostic_from(&state.storage, chrono::Utc::now()).await
}

#[tauri::command]
async fn delete_sync_configuration(
    state: State<'_, AppState>,
) -> Result<SyncPairingStatusDto, ApiError> {
    let _guard = acquire_sync_lock(&state.sync_lock)?;
    let secret_store = platform_secret_store()?;
    delete_sync_configuration_with(&state.storage, &secret_store).await
}

fn open_storage(database_path: &Path) -> Result<Storage, Box<dyn std::error::Error>> {
    let storage = tauri::async_runtime::block_on(Storage::open(database_path))?;
    tauri::async_runtime::block_on(storage.apply_article_retention())?;
    Ok(storage)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init());
    #[cfg(any(target_os = "android", target_os = "ios"))]
    let builder = builder.plugin(tauri_plugin_barcode_scanner::init());
    builder
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_data_dir)?;
            let storage = open_storage(&app_data_dir.join(DATABASE_FILE_NAME))?;
            app.manage(AppState {
                storage,
                refresh_lock: Mutex::new(()),
                sync_lock: Mutex::new(()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_articles,
            get_article,
            refresh_feeds,
            refresh_feed,
            set_article_read,
            set_articles_read,
            set_article_favorite,
            archive_article,
            archive_articles,
            list_feeds,
            add_feed,
            set_feed_active,
            delete_feed,
            sync_pairing_status,
            configure_sync_group,
            pairing_invitation,
            join_sync_group,
            rename_sync_device,
            revoke_sync_device,
            synchronize_now,
            export_sync_diagnostic,
            delete_sync_configuration,
        ])
        .run(tauri::generate_context!())
        .expect("error while running InkRiver");
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use inkriver::article::{Article, ContentKind, Source};
    use inkriver::config::FeedConfig;
    use inkriver::feed::FeedMetadata;
    use inkriver::storage::FeedRefreshFailure;
    use inkriver::sync_secrets::SyncSecrets;
    use std::sync::Mutex as StdMutex;

    #[derive(Default)]
    struct MemorySecretStore(StdMutex<Option<SyncSecrets>>);

    impl SyncSecretStore for MemorySecretStore {
        fn save(&self, secrets: &SyncSecrets) -> anyhow::Result<()> {
            *self.0.lock().unwrap() = Some(secrets.clone());
            Ok(())
        }

        fn load(&self) -> anyhow::Result<Option<SyncSecrets>> {
            Ok(self.0.lock().unwrap().clone())
        }

        fn delete(&self) -> anyhow::Result<()> {
            self.0.lock().unwrap().take();
            Ok(())
        }
    }

    async fn test_storage() -> (tempfile::TempDir, Storage) {
        let directory = tempfile::tempdir().unwrap();
        let storage = Storage::open(&directory.path().join("inkriver.db"))
            .await
            .unwrap();
        (directory, storage)
    }

    async fn storage_with_article() -> (tempfile::TempDir, Storage) {
        let (directory, storage) = test_storage().await;
        storage
            .import_feeds(&[FeedConfig {
                id: "space".to_string(),
                platform: Platform::Substack,
                url: "https://space.substack.com/feed".to_string(),
            }])
            .await
            .unwrap();
        storage
            .upsert_articles(&[Article {
                id: "space::mars".to_string(),
                feed_id: "space".to_string(),
                title: Some("Observer Mars".to_string()),
                author: Some("Claire".to_string()),
                published_at: None,
                url: Some("https://space.example/mars".to_string()),
                content: Some("<p>Mars est orangée.</p>".to_string()),
                content_kind: ContentKind::Excerpt,
                source: Source::Substack,
            }])
            .await
            .unwrap();
        (directory, storage)
    }

    #[tokio::test]
    async fn pairing_adapters_configure_invite_and_join_without_webdav_secret_in_qr() {
        let (_linux_directory, linux) = test_storage().await;
        let linux_secrets = MemorySecretStore::default();
        let initial = sync_pairing_status_from(&linux).await.unwrap();
        assert!(!initial.configured);

        let configured = configure_sync_group_with(
            &linux,
            &linux_secrets,
            "https://cloud.example/dav/inkriver",
            "alice",
            "webdav-secret".to_string(),
            "Linux",
        )
        .await
        .unwrap();
        assert!(configured.configured);
        assert_eq!(configured.devices.len(), 1);
        assert!(configured.devices[0].is_local);

        let invitation = create_pairing_invitation_with(&linux, &linux_secrets)
            .await
            .unwrap();
        assert!(invitation.invitation.starts_with("inkriver://pair/"));
        assert!(!invitation.invitation.contains("webdav-secret"));
        assert!(
            invitation
                .qr_code_data_url
                .starts_with("data:image/svg+xml;base64,")
        );

        let (_android_directory, android) = test_storage().await;
        let android_secrets = MemorySecretStore::default();
        let joined = accept_pairing_invitation_with(
            &android,
            &android_secrets,
            &invitation.invitation,
            "webdav-secret".to_string(),
            "Android",
        )
        .await
        .unwrap();
        assert!(joined.configured);
        assert_eq!(joined.webdav_base_url, configured.webdav_base_url);
        assert_eq!(joined.webdav_username.as_deref(), Some("alice"));
        assert_eq!(joined.devices.len(), 2);
        assert!(joined.devices.iter().any(|device| device.is_local));
    }

    #[tokio::test]
    async fn pairing_status_persists_runtime_details_and_deletion_preserves_articles() {
        let (_directory, storage) = storage_with_article().await;
        let secrets = MemorySecretStore::default();
        configure_sync_group_with(
            &storage,
            &secrets,
            "https://cloud.example/dav/inkriver",
            "alice",
            "webdav-secret".to_string(),
            "Linux",
        )
        .await
        .unwrap();
        let synchronized_at = Utc.with_ymd_and_hms(2026, 8, 28, 12, 30, 0).unwrap();
        storage
            .record_sync_success(
                synchronized_at,
                StoredSyncReport {
                    exported_events: 2,
                    applied_events: 3,
                    pending_events: 1,
                    compacted_events: 4,
                    deleted_segments: 5,
                    deferred_segment_deletions: 6,
                    ..StoredSyncReport::default()
                },
            )
            .await
            .unwrap();

        let status = sync_pairing_status_from(&storage).await.unwrap();
        assert_eq!(
            status.last_success_at.as_deref(),
            Some("2026-08-28T12:30:00+00:00")
        );
        assert_eq!(status.last_report.as_ref().unwrap().exported_events, 2);
        assert_eq!(status.last_report.as_ref().unwrap().pending_events, 1);
        assert_eq!(status.last_report.as_ref().unwrap().compacted_events, 4);
        assert_eq!(status.last_report.as_ref().unwrap().deleted_segments, 5);
        assert_eq!(
            status
                .last_report
                .as_ref()
                .unwrap()
                .deferred_segment_deletions,
            6
        );

        let removed = delete_sync_configuration_with(&storage, &secrets)
            .await
            .unwrap();
        assert!(!removed.configured);
        assert!(removed.last_attempt_at.is_none());
        assert!(secrets.load().unwrap().is_none());
        assert!(storage.get_article("space::mars").await.unwrap().is_some());
        assert_eq!(
            removed
                .devices
                .iter()
                .filter(|device| device.is_local)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn diagnostic_adapter_exports_redacted_camel_case_json() {
        let (_directory, storage) = storage_with_article().await;
        let generated_at = Utc.with_ymd_and_hms(2026, 8, 30, 10, 0, 0).unwrap();
        storage
            .record_sync_success(
                generated_at,
                StoredSyncReport {
                    compacted_events: 7,
                    deleted_segments: 2,
                    ..StoredSyncReport::default()
                },
            )
            .await
            .unwrap();

        let json = export_sync_diagnostic_from(&storage, generated_at)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(value["format"], "inkriver-sync-diagnostic");
        assert_eq!(value["generatedAt"], "2026-08-30T10:00:00+00:00");
        assert_eq!(value["lastReport"]["compactedEvents"], 7);
        assert_eq!(value["lastReport"]["deletedSegments"], 2);
        for forbidden in [
            "space.example",
            "Observer Mars",
            "Claire",
            "Mars est orangée",
        ] {
            assert!(!json.contains(forbidden));
        }
    }

    #[test]
    fn parse_platform_accepts_override_and_rejects_unknown_value() {
        assert_eq!(
            parse_platform(Some("medium")).unwrap(),
            Some(Platform::Medium)
        );
        assert_eq!(parse_platform(None).unwrap(), None);
        assert_eq!(
            parse_platform(Some("blog")).unwrap_err().code,
            "invalid_platform"
        );
    }

    #[tokio::test]
    async fn list_and_get_map_storage_models_to_camel_case_ready_dtos() {
        let (_directory, storage) = storage_with_article().await;
        let summaries = list_articles_from(&storage).await.unwrap();
        let detail = get_article_from(&storage, "space::mars").await.unwrap();

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].source, "substack");
        assert_eq!(detail.content_kind, "excerpt");
        assert_eq!(detail.content.as_deref(), Some("<p>Mars est orangée.</p>"));
        let summary_value = serde_json::to_value(&summaries[0]).unwrap();
        let detail_value = serde_json::to_value(&detail).unwrap();
        assert!(summary_value.get("logoDataUrl").is_none());
        assert!(detail_value.get("logoDataUrl").is_none());
    }

    #[test]
    fn extracted_content_kind_is_exposed_by_the_article_dto() {
        let dto = ArticleDetailDto::from(StoredArticle {
            article: Article {
                id: "journal::article".to_string(),
                feed_id: "journal".to_string(),
                title: Some("Extracted article".to_string()),
                author: None,
                published_at: None,
                url: Some("https://journal.example/article".to_string()),
                content: Some("<p>Complete web content</p>".to_string()),
                content_kind: ContentKind::Extracted,
                source: Source::Other,
            },
            is_read: false,
            is_favorite: false,
        });

        assert_eq!(dto.content_kind, "extracted");
    }

    #[tokio::test]
    async fn missing_article_returns_structured_error() {
        let (_directory, storage) = test_storage().await;
        let error = get_article_from(&storage, "missing").await.unwrap_err();
        assert_eq!(error.code, "article_not_found");
    }

    #[tokio::test]
    async fn state_updates_work_and_reject_missing_articles() {
        let (_directory, storage) = storage_with_article().await;
        set_article_read_in(&storage, "space::mars", true)
            .await
            .unwrap();
        set_article_favorite_in(&storage, "space::mars", true)
            .await
            .unwrap();
        let article = storage.get_article("space::mars").await.unwrap().unwrap();
        assert!(article.is_read);
        assert!(article.is_favorite);
        set_article_read_in(&storage, "space::mars", false)
            .await
            .unwrap();
        let article = storage.get_article("space::mars").await.unwrap().unwrap();
        assert!(!article.is_read);
        assert!(article.is_favorite);
        assert_eq!(
            set_article_read_in(&storage, "missing", true)
                .await
                .unwrap_err()
                .code,
            "article_not_found"
        );
    }

    #[tokio::test]
    async fn grouped_read_adapter_is_atomic_and_rejects_empty_selection() {
        let (_directory, storage) = storage_with_article().await;
        storage
            .upsert_articles(&[Article {
                id: "space::venus".to_string(),
                feed_id: "space".to_string(),
                title: Some("Observer Vénus".to_string()),
                author: Some("Claire".to_string()),
                published_at: None,
                url: Some("https://space.example/venus".to_string()),
                content: Some("<p>Vénus est brillante.</p>".to_string()),
                content_kind: ContentKind::Full,
                source: Source::Substack,
            }])
            .await
            .unwrap();

        set_articles_read_in(
            &storage,
            &["space::mars".to_string(), "space::venus".to_string()],
            true,
        )
        .await
        .unwrap();
        assert!(
            list_articles_from(&storage)
                .await
                .unwrap()
                .iter()
                .all(|article| article.is_read)
        );

        let error = set_articles_read_in(
            &storage,
            &["space::mars".to_string(), "missing".to_string()],
            false,
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "article_not_found");
        assert!(
            list_articles_from(&storage)
                .await
                .unwrap()
                .iter()
                .all(|article| article.is_read)
        );
        assert_eq!(
            set_articles_read_in(&storage, &[], false)
                .await
                .unwrap_err()
                .code,
            "invalid_request"
        );
    }

    #[tokio::test]
    async fn grouped_archive_adapter_is_atomic() {
        let (_directory, storage) = storage_with_article().await;
        let error = archive_articles_in(
            &storage,
            &["space::mars".to_string(), "missing".to_string()],
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "article_not_found");
        assert_eq!(list_articles_from(&storage).await.unwrap().len(), 1);

        archive_articles_in(
            &storage,
            &["space::mars".to_string(), "space::mars".to_string()],
        )
        .await
        .unwrap();
        assert!(list_articles_from(&storage).await.unwrap().is_empty());
        assert_eq!(
            archive_articles_in(&storage, &[]).await.unwrap_err().code,
            "invalid_request"
        );
    }

    #[tokio::test]
    async fn archive_adapter_hides_article_and_maps_missing_ids() {
        let (_directory, storage) = storage_with_article().await;

        archive_article_in(&storage, "space::mars").await.unwrap();

        assert!(list_articles_from(&storage).await.unwrap().is_empty());
        assert_eq!(
            get_article_from(&storage, "space::mars").await.unwrap_err(),
            ApiError::new("article_not_found", "Article introuvable : space::mars")
        );
        assert_eq!(
            archive_article_in(&storage, "space::mars")
                .await
                .unwrap_err()
                .code,
            "article_not_found"
        );
    }

    #[test]
    fn opening_app_storage_applies_retention_before_first_list() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("inkriver.db");
        tauri::async_runtime::block_on(async {
            let storage = Storage::open(&path).await.unwrap();
            storage
                .import_feeds(&[FeedConfig {
                    id: "space".to_string(),
                    platform: Platform::Substack,
                    url: "https://space.substack.com/feed".to_string(),
                }])
                .await
                .unwrap();
            storage
                .upsert_articles(&[Article {
                    id: "space::old".to_string(),
                    feed_id: "space".to_string(),
                    title: Some("Ancien article".to_string()),
                    author: None,
                    published_at: Some(Utc.with_ymd_and_hms(2020, 1, 1, 12, 0, 0).unwrap()),
                    url: Some("https://space.example/old".to_string()),
                    content: Some("Old body".to_string()),
                    content_kind: ContentKind::Full,
                    source: Source::Substack,
                }])
                .await
                .unwrap();
            storage.set_read("space::old", true).await.unwrap();
            storage.close().await;
        });

        let storage = open_storage(&path).unwrap();
        assert!(
            tauri::async_runtime::block_on(storage.list_article_summaries())
                .unwrap()
                .is_empty()
        );
        tauri::async_runtime::block_on(storage.close());
    }

    #[test]
    fn refresh_report_keeps_partial_feed_errors() {
        use inkriver::service::{FeedCollectionError, FeedLoadError, FeedLoadStage};
        let dto = RefreshReportDto::from(RefreshReport {
            active_feeds: 1,
            collected_articles: 0,
            inserted_articles: 0,
            updated_articles: 0,
            auto_archived_articles: 2,
            extracted_articles: 3,
            extraction_failed_articles: 1,
            extraction_skipped_articles: 4,
            errors: vec![FeedCollectionError {
                feed_id: "space".to_string(),
                feed_url: "https://space.example/feed".to_string(),
                error: FeedLoadError {
                    stage: FeedLoadStage::HttpRequest,
                    message: "offline".to_string(),
                },
            }],
        });
        assert_eq!(dto.errors[0].stage, "HTTP request");
        assert_eq!(dto.errors[0].message, "offline");
        assert_eq!(dto.auto_archived_articles, 2);
        assert_eq!(dto.extracted_articles, 3);
        assert_eq!(dto.extraction_failed_articles, 1);
        assert_eq!(dto.extraction_skipped_articles, 4);
    }

    #[tokio::test]
    async fn feed_adapters_add_list_and_deactivate_subscriptions() {
        let (_directory, storage) = test_storage().await;
        let added = add_feed_to(&storage, " https://letters.substack.com/feed#latest ", None)
            .await
            .unwrap();
        assert_eq!(added.platform, "substack");
        assert_eq!(added.url, "https://letters.substack.com/feed");
        assert_eq!(
            list_feeds_from(&storage).await.unwrap(),
            vec![added.clone()]
        );

        let inactive = set_feed_active_in(&storage, &added.id, false)
            .await
            .unwrap();
        assert!(!inactive.is_active);
        assert_eq!(
            add_feed_to(&storage, "file:///tmp/feed", None)
                .await
                .unwrap_err()
                .code,
            "invalid_url"
        );
    }

    #[tokio::test]
    async fn feed_adapter_exposes_persisted_metadata_and_error_details() {
        let (_directory, storage) = storage_with_article().await;
        let refreshed_at = Utc.with_ymd_and_hms(2026, 8, 12, 18, 30, 0).unwrap();
        storage
            .record_feed_refreshes(
                &[FeedMetadata {
                    id: "space".to_string(),
                    title: "Carnet du ciel".to_string(),
                    description: "Observer les planètes".to_string(),
                    author: Some("Claire".to_string()),
                    site_url: "https://space.example".to_string(),
                    declared_icon_url: None,
                }],
                &[],
                refreshed_at,
            )
            .await
            .unwrap();
        let failed_at = Utc.with_ymd_and_hms(2026, 8, 12, 19, 0, 0).unwrap();
        storage
            .record_feed_refreshes(
                &[],
                &[FeedRefreshFailure {
                    feed_id: "space".to_string(),
                    stage: "HTTP request".to_string(),
                    message: "offline".to_string(),
                }],
                failed_at,
            )
            .await
            .unwrap();

        let feed = list_feeds_from(&storage).await.unwrap().remove(0);
        assert_eq!(feed.title.as_deref(), Some("Carnet du ciel"));
        assert_eq!(feed.description.as_deref(), Some("Observer les planètes"));
        assert_eq!(
            feed.last_success_at.as_deref(),
            Some("2026-08-12T18:30:00+00:00")
        );
        assert_eq!(feed.last_error.as_ref().unwrap().stage, "HTTP request");
        assert_eq!(feed.last_error.as_ref().unwrap().message, "offline");
        assert_eq!(
            feed.last_error.as_ref().unwrap().occurred_at,
            "2026-08-12T19:00:00+00:00"
        );
    }

    #[tokio::test]
    async fn delete_feed_adapter_removes_articles_and_maps_missing_feed_error() {
        let (_directory, storage) = storage_with_article().await;

        let result = delete_feed_from(&storage, "space").await.unwrap();
        assert_eq!(
            result,
            DeleteFeedResultDto {
                feed_id: "space".to_string(),
                deleted_articles: 1,
            }
        );
        assert!(list_feeds_from(&storage).await.unwrap().is_empty());
        assert!(list_articles_from(&storage).await.unwrap().is_empty());
        assert_eq!(
            delete_feed_from(&storage, "missing").await.unwrap_err(),
            ApiError::new("feed_not_found", "Abonnement introuvable : missing")
        );
    }

    #[test]
    fn feed_dto_embeds_cached_png_as_a_data_url() {
        let dto = FeedDto::from(StoredFeed {
            id: "feed-id".to_string(),
            platform: Platform::Other,
            url: "https://example.com/feed".to_string(),
            is_active: true,
            title: Some("Example".to_string()),
            description: None,
            author: None,
            last_published_at: None,
            last_success_at: None,
            last_error: None,
            logo_png: Some(vec![0x89, b'P', b'N', b'G']),
        });

        assert_eq!(
            dto.logo_data_url.as_deref(),
            Some("data:image/png;base64,iVBORw==")
        );
        let value = serde_json::to_value(dto).unwrap();
        assert_eq!(value["logoDataUrl"], "data:image/png;base64,iVBORw==");
    }

    #[test]
    fn serialized_dtos_use_camel_case_fields() {
        let value = serde_json::to_value(FeedDto {
            id: "feed-id".to_string(),
            platform: "medium".to_string(),
            url: "https://medium.com/feed/@inkriver".to_string(),
            is_active: true,
            title: Some("InkRiver on Medium".to_string()),
            description: None,
            author: Some("InkRiver".to_string()),
            last_published_at: None,
            last_success_at: None,
            last_error: None,
            logo_data_url: None,
        })
        .unwrap();
        assert_eq!(value["isActive"], true);
        assert!(value.get("is_active").is_none());

        let deletion = serde_json::to_value(DeleteFeedResultDto {
            feed_id: "feed-id".to_string(),
            deleted_articles: 3,
        })
        .unwrap();
        assert_eq!(deletion["feedId"], "feed-id");
        assert_eq!(deletion["deletedArticles"], 3);
        assert!(deletion.get("deleted_articles").is_none());

        let refresh = serde_json::to_value(RefreshReportDto {
            active_feeds: 2,
            collected_articles: 5,
            inserted_articles: 3,
            updated_articles: 2,
            auto_archived_articles: 4,
            extracted_articles: 1,
            extraction_failed_articles: 2,
            extraction_skipped_articles: 3,
            errors: Vec::new(),
        })
        .unwrap();
        assert_eq!(refresh["autoArchivedArticles"], 4);
        assert_eq!(refresh["extractedArticles"], 1);
        assert_eq!(refresh["extractionFailedArticles"], 2);
        assert_eq!(refresh["extractionSkippedArticles"], 3);
        assert!(refresh.get("auto_archived_articles").is_none());

        let sync = serde_json::to_value(SyncTransportReportDto::from(SyncTransportReport {
            uploaded_segments: 1,
            reused_segments: 2,
            exported_events: 3,
            downloaded_segments: 4,
            received_events: 5,
            imported_events: 6,
            duplicate_events: 7,
            applied_events: 8,
            pending_events: 9,
            compacted_events: 10,
            deleted_segments: 11,
            deferred_segment_deletions: 12,
        }))
        .unwrap();
        assert_eq!(sync["uploadedSegments"], 1);
        assert_eq!(sync["appliedEvents"], 8);
        assert_eq!(sync["pendingEvents"], 9);
        assert_eq!(sync["compactedEvents"], 10);
        assert_eq!(sync["deletedSegments"], 11);
        assert_eq!(sync["deferredSegmentDeletions"], 12);
        assert!(sync.get("uploaded_segments").is_none());
    }

    #[tokio::test]
    async fn refresh_lock_rejects_a_second_concurrent_operation() {
        let lock = Mutex::new(());
        let first = acquire_refresh_lock(&lock).unwrap();
        let error = acquire_refresh_lock(&lock).unwrap_err();
        assert_eq!(error.code, "refresh_in_progress");
        drop(first);
        assert!(acquire_refresh_lock(&lock).is_ok());
    }

    #[tokio::test]
    async fn sync_lock_rejects_a_second_concurrent_operation() {
        let lock = Mutex::new(());
        let first = acquire_sync_lock(&lock).unwrap();
        let error = acquire_sync_lock(&lock).unwrap_err();
        assert_eq!(error.code, "sync_in_progress");
        drop(first);
        assert!(acquire_sync_lock(&lock).is_ok());
    }

    #[test]
    fn sync_runtime_failures_are_structured_without_hiding_details() {
        assert_eq!(
            sync_runtime_error(anyhow::anyhow!("La synchronisation n'est pas configurée")).code,
            "sync_not_configured"
        );
        assert_eq!(
            sync_runtime_error(anyhow::anyhow!(
                "Les secrets de synchronisation sont absents"
            ))
            .code,
            "sync_secrets"
        );
        let error = sync_runtime_error(anyhow::anyhow!("PROPFIND returned HTTP status 503"));
        assert_eq!(error.code, "sync_failed");
        assert_eq!(error.message, "PROPFIND returned HTTP status 503");
    }

    #[tokio::test]
    async fn targeted_refresh_maps_missing_and_inactive_feed_errors() {
        let (_directory, storage) = test_storage().await;
        assert_eq!(
            refresh_feed_from(&storage, "missing")
                .await
                .unwrap_err()
                .code,
            "feed_not_found"
        );
        storage
            .import_feeds(&[FeedConfig {
                id: "space".to_string(),
                platform: Platform::Substack,
                url: "https://space.substack.com/feed".to_string(),
            }])
            .await
            .unwrap();
        storage.set_feed_active("space", false).await.unwrap();
        assert_eq!(
            refresh_feed_from(&storage, "space").await.unwrap_err().code,
            "feed_inactive"
        );
    }

    #[test]
    fn inactive_subscription_maps_to_a_structured_error() {
        assert_eq!(
            subscription_error(SubscriptionError::Inactive("space".to_string())),
            ApiError::new(
                "feed_inactive",
                "Réactivez l’abonnement avant de l’actualiser : space"
            )
        );
    }
}
