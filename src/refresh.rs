use crate::config::Config;
use crate::service::{self, CollectionReport, FeedCollectionError};
use crate::storage::{FeedRefreshFailure, Storage, UpsertStats};
use anyhow::Result;
use chrono::Utc;
#[cfg(test)]
use std::future::Future;

/// Summarizes one feed refresh and its persistence effects.
#[derive(Debug, PartialEq, Eq)]
pub struct RefreshReport {
    pub active_feeds: usize,
    pub collected_articles: usize,
    pub inserted_articles: usize,
    pub updated_articles: usize,
    pub auto_archived_articles: usize,
    pub errors: Vec<FeedCollectionError>,
}

/// Imports configured feeds, collects them, and persists every successful article.
///
/// Per-feed collection errors are returned in the report and do not prevent
/// successful feeds from being stored.
///
/// # Errors
///
/// Returns an error when subscriptions or collected articles cannot be persisted.
pub async fn refresh(storage: &Storage, config: &Config) -> Result<RefreshReport> {
    storage.import_feeds(&config.feeds).await?;
    let report = service::collect_articles(config).await;
    store_collection(storage, config, report).await
}

/// Refreshes only subscriptions currently active in SQLite.
///
/// Unlike [`refresh`], this never imports or synchronizes a TOML configuration.
pub async fn refresh_active(storage: &Storage) -> Result<RefreshReport> {
    let config = Config {
        feeds: storage.active_feed_config().await?,
    };
    let report = service::collect_articles(&config).await;
    store_collection(storage, &config, report).await
}

#[cfg(test)]
async fn refresh_with_collector<F, Fut>(
    storage: &Storage,
    config: &Config,
    collect: F,
) -> Result<RefreshReport>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = CollectionReport>,
{
    storage.import_feeds(&config.feeds).await?;
    store_collection(storage, config, collect().await).await
}

async fn store_collection(
    storage: &Storage,
    config: &Config,
    report: CollectionReport,
) -> Result<RefreshReport> {
    let CollectionReport {
        feeds,
        articles,
        errors,
    } = report;
    let collected_articles = articles.len();
    let UpsertStats { inserted, updated } = storage.upsert_articles(&articles).await?;
    let failures = errors
        .iter()
        .map(|error| FeedRefreshFailure {
            feed_id: error.feed_id.clone(),
            stage: error.error.stage.to_string(),
            message: error.error.message.clone(),
        })
        .collect::<Vec<_>>();
    let refreshed_at = Utc::now();
    storage
        .record_feed_refreshes(&feeds, &failures, refreshed_at)
        .await?;
    let auto_archived_articles = storage.archive_expired_read_articles(refreshed_at).await?;

    Ok(RefreshReport {
        active_feeds: config.feeds.len(),
        collected_articles,
        inserted_articles: inserted,
        updated_articles: updated,
        auto_archived_articles,
        errors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::article::{Article, ContentKind, Source};
    use crate::config::{FeedConfig, Platform};
    use crate::feed::FeedMetadata;
    use crate::service::{FeedCollectionError, FeedLoadError, FeedLoadStage};
    use chrono::{TimeZone, Utc};
    use std::future::Future;

    fn assert_future<T: Future>(_future: T) {}

    fn feed(id: &str, platform: Platform) -> FeedConfig {
        FeedConfig {
            id: id.to_string(),
            platform,
            url: format!("https://feeds.example/{id}"),
        }
    }

    fn article(id: &str, feed_id: &str, source: Source) -> Article {
        Article {
            id: id.to_string(),
            feed_id: feed_id.to_string(),
            title: Some(format!("Readable title for {id}")),
            author: Some("Test Author".to_string()),
            published_at: Some(Utc.with_ymd_and_hms(2026, 8, 8, 12, 0, 0).unwrap()),
            url: Some(format!("https://articles.example/{id}")),
            content: Some(format!("Coherent body for {id}")),
            content_kind: ContentKind::Full,
            source,
        }
    }

    fn collection_error(feed_id: &str) -> FeedCollectionError {
        FeedCollectionError {
            feed_id: feed_id.to_string(),
            feed_url: format!("https://feeds.example/{feed_id}"),
            error: FeedLoadError {
                stage: FeedLoadStage::HttpRequest,
                message: "network unavailable".to_string(),
            },
        }
    }

    /// Verifies the public refresh API remains awaitable for CLI and Tauri callers.
    #[tokio::test]
    async fn public_refresh_api_is_async() {
        let storage = Storage::open_in_memory().await.unwrap();
        let config = Config { feeds: Vec::new() };

        assert_future(refresh(&storage, &config));
    }

    /// Verifies successful articles are stored while per-feed errors remain visible.
    #[tokio::test]
    async fn refresh_persists_successes_and_returns_collection_errors() {
        let storage = Storage::open_in_memory().await.unwrap();
        let config = Config {
            feeds: vec![
                feed("astronomy", Platform::Substack),
                feed("bread", Platform::Medium),
            ],
        };
        let report = CollectionReport {
            feeds: vec![FeedMetadata {
                id: "astronomy".to_string(),
                title: "Night sky notes".to_string(),
                description: "Practical astronomy".to_string(),
                author: Some("Claire".to_string()),
            }],
            articles: vec![article("astronomy::jupiter", "astronomy", Source::Substack)],
            errors: vec![collection_error("bread")],
        };

        let result = refresh_with_collector(&storage, &config, || async { report })
            .await
            .unwrap();

        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].feed_id, "bread");
        assert_eq!(result.active_feeds, 2);
        assert_eq!(result.collected_articles, 1);
        assert_eq!(result.inserted_articles, 1);
        assert_eq!(result.updated_articles, 0);
        assert_eq!(storage.list_articles().await.unwrap().len(), 1);
        let feeds = storage.list_feeds().await.unwrap();
        assert!(feeds.iter().all(|feed| feed.is_active));
        let astronomy = feeds.iter().find(|feed| feed.id == "astronomy").unwrap();
        assert_eq!(astronomy.title.as_deref(), Some("Night sky notes"));
        assert_eq!(astronomy.author.as_deref(), Some("Claire"));
        assert!(astronomy.last_success_at.is_some());
        assert!(astronomy.last_error.is_none());
        let bread = feeds.iter().find(|feed| feed.id == "bread").unwrap();
        assert_eq!(bread.last_error.as_ref().unwrap().stage, "HTTP request");
        assert_eq!(
            bread.last_error.as_ref().unwrap().message,
            "network unavailable"
        );
    }

    /// Verifies an entirely failed collection leaves previously cached articles intact.
    #[tokio::test]
    async fn refresh_keeps_cached_articles_when_network_collection_fails() {
        let storage = Storage::open_in_memory().await.unwrap();
        let config = Config {
            feeds: vec![feed("astronomy", Platform::Substack)],
        };
        let initial = CollectionReport {
            feeds: Vec::new(),
            articles: vec![article("astronomy::jupiter", "astronomy", Source::Substack)],
            errors: Vec::new(),
        };
        refresh_with_collector(&storage, &config, || async { initial })
            .await
            .unwrap();

        let offline = CollectionReport {
            feeds: Vec::new(),
            articles: Vec::new(),
            errors: vec![collection_error("astronomy")],
        };
        let result = refresh_with_collector(&storage, &config, || async { offline })
            .await
            .unwrap();

        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.collected_articles, 0);
        assert_eq!(result.inserted_articles, 0);
        assert_eq!(result.updated_articles, 0);
        assert_eq!(storage.list_articles().await.unwrap().len(), 1);
    }

    /// Verifies refresh, local state, deduplication, and feed removal work together.
    #[tokio::test]
    async fn refresh_preserves_history_and_local_state_when_feed_is_removed() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("inkriver.db");
        let storage = Storage::open(&database_path).await.unwrap();
        let complete_config = Config {
            feeds: vec![
                feed("astronomy", Platform::Substack),
                feed("bread", Platform::Medium),
            ],
        };
        let astronomy = article("astronomy::jupiter", "astronomy", Source::Substack);
        let bread = article("bread::sourdough", "bread", Source::Medium);
        let first_collection = CollectionReport {
            feeds: Vec::new(),
            articles: vec![astronomy.clone(), bread.clone()],
            errors: Vec::new(),
        };
        refresh_with_collector(&storage, &complete_config, || async { first_collection })
            .await
            .unwrap();
        storage.set_read(&bread.id, true).await.unwrap();
        storage.set_favorite(&bread.id, true).await.unwrap();
        storage.close().await;

        let storage = Storage::open(&database_path).await.unwrap();
        let second_collection = CollectionReport {
            feeds: Vec::new(),
            articles: vec![astronomy, bread.clone()],
            errors: Vec::new(),
        };
        let second_result =
            refresh_with_collector(&storage, &complete_config, || async { second_collection })
                .await
                .unwrap();
        assert_eq!(second_result.inserted_articles, 0);
        assert_eq!(second_result.updated_articles, 2);
        assert_eq!(storage.list_articles().await.unwrap().len(), 2);
        storage.close().await;

        let storage = Storage::open(&database_path).await.unwrap();
        let reduced_config = Config {
            feeds: vec![feed("astronomy", Platform::Substack)],
        };
        let offline_collection = CollectionReport {
            feeds: Vec::new(),
            articles: Vec::new(),
            errors: vec![collection_error("astronomy")],
        };
        let result =
            refresh_with_collector(&storage, &reduced_config, || async { offline_collection })
                .await
                .unwrap();

        assert_eq!(result.errors.len(), 1);
        let feeds = storage.list_feeds().await.unwrap();
        assert!(
            feeds
                .iter()
                .any(|feed| feed.id == "bread" && !feed.is_active)
        );
        let articles = storage.list_articles().await.unwrap();
        assert_eq!(articles.len(), 2);
        let stored_bread = articles
            .iter()
            .find(|stored| stored.article.id == bread.id)
            .unwrap();
        assert!(stored_bread.is_read);
        assert!(stored_bread.is_favorite);
        storage.close().await;
    }

    #[tokio::test]
    async fn refresh_active_collects_only_active_sqlite_subscriptions() {
        let storage = Storage::open_in_memory().await.unwrap();
        let config = Config {
            feeds: vec![
                feed("astronomy", Platform::Substack),
                feed("bread", Platform::Medium),
            ],
        };
        storage.import_feeds(&config.feeds).await.unwrap();
        storage.set_feed_active("bread", false).await.unwrap();

        let active_config = Config {
            feeds: storage.active_feed_config().await.unwrap(),
        };
        assert_eq!(active_config.feeds.len(), 1);
        assert_eq!(active_config.feeds[0].id, "astronomy");
        let report = store_collection(
            &storage,
            &active_config,
            CollectionReport {
                feeds: Vec::new(),
                articles: vec![article("astronomy::mars", "astronomy", Source::Substack)],
                errors: Vec::new(),
            },
        )
        .await
        .unwrap();

        assert_eq!(report.active_feeds, 1);
        assert_eq!(report.inserted_articles, 1);
        let feeds = storage.list_feeds().await.unwrap();
        assert!(
            feeds
                .iter()
                .any(|feed| feed.id == "bread" && !feed.is_active)
        );
    }

    #[tokio::test]
    async fn refresh_applies_retention_and_never_restores_the_tombstone() {
        let storage = Storage::open_in_memory().await.unwrap();
        let config = Config {
            feeds: vec![feed("astronomy", Platform::Substack)],
        };
        storage.import_feeds(&config.feeds).await.unwrap();
        let old_article = Article {
            published_at: Some(Utc.with_ymd_and_hms(2020, 1, 1, 12, 0, 0).unwrap()),
            ..article("astronomy::old", "astronomy", Source::Substack)
        };
        storage
            .upsert_articles(std::slice::from_ref(&old_article))
            .await
            .unwrap();
        storage.set_read(&old_article.id, true).await.unwrap();

        let first = refresh_with_collector(&storage, &config, || async {
            CollectionReport {
                feeds: Vec::new(),
                articles: vec![old_article.clone()],
                errors: Vec::new(),
            }
        })
        .await
        .unwrap();
        assert_eq!(first.updated_articles, 1);
        assert_eq!(first.auto_archived_articles, 1);
        assert!(storage.list_articles().await.unwrap().is_empty());

        let second = refresh_with_collector(&storage, &config, || async {
            CollectionReport {
                feeds: Vec::new(),
                articles: vec![old_article],
                errors: Vec::new(),
            }
        })
        .await
        .unwrap();
        assert_eq!(second.inserted_articles, 0);
        assert_eq!(second.updated_articles, 0);
        assert_eq!(second.auto_archived_articles, 0);
        assert!(storage.list_articles().await.unwrap().is_empty());
    }
}
