use inkriver::config::Platform;
use inkriver::refresh::{self, RefreshReport};
use inkriver::storage::{
    ArticleSummary, DeleteFeedResult, Storage, StoredArticle, StoredFeed, SubscriptionError,
};
use serde::Serialize;
use std::path::Path;
use tauri::{Manager, State};

const DATABASE_FILE_NAME: &str = "inkriver.db";

pub struct AppState {
    storage: Storage,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredFeedErrorDto {
    pub stage: String,
    pub message: String,
    pub occurred_at: String,
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
    pub errors: Vec<FeedRefreshErrorDto>,
}

impl From<RefreshReport> for RefreshReportDto {
    fn from(report: RefreshReport) -> Self {
        Self {
            active_feeds: report.active_feeds,
            collected_articles: report.collected_articles,
            inserted_articles: report.inserted_articles,
            updated_articles: report.updated_articles,
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
        SubscriptionError::Database(message) => ApiError::new("storage", message),
    }
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
    refresh::refresh_active(&state.storage)
        .await
        .map(Into::into)
        .map_err(ApiError::storage)
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
async fn set_article_favorite(
    state: State<'_, AppState>,
    article_id: String,
    is_favorite: bool,
) -> Result<(), ApiError> {
    set_article_favorite_in(&state.storage, &article_id, is_favorite).await
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

fn open_storage(database_path: &Path) -> Result<Storage, Box<dyn std::error::Error>> {
    Ok(tauri::async_runtime::block_on(Storage::open(
        database_path,
    ))?)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_data_dir)?;
            let storage = open_storage(&app_data_dir.join(DATABASE_FILE_NAME))?;
            app.manage(AppState { storage });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_articles,
            get_article,
            refresh_feeds,
            set_article_read,
            set_article_favorite,
            list_feeds,
            add_feed,
            set_feed_active,
            delete_feed,
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

    #[test]
    fn refresh_report_keeps_partial_feed_errors() {
        use inkriver::service::{FeedCollectionError, FeedLoadError, FeedLoadStage};
        let dto = RefreshReportDto::from(RefreshReport {
            active_feeds: 1,
            collected_articles: 0,
            inserted_articles: 0,
            updated_articles: 0,
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
    }
}
