use crate::config::Config;
use crate::service::{self, CollectionReport, FeedCollectionError};
use crate::storage::{Storage, UpsertStats};
use anyhow::Result;
use std::future::Future;

/// Summarizes one feed refresh and its persistence effects.
#[derive(Debug, PartialEq, Eq)]
pub struct RefreshReport {
    pub active_feeds: usize,
    pub collected_articles: usize,
    pub inserted_articles: usize,
    pub updated_articles: usize,
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
    refresh_with_collector(storage, config, || service::collect_articles(config)).await
}

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
    let CollectionReport { articles, errors } = collect().await;
    let collected_articles = articles.len();
    let UpsertStats { inserted, updated } = storage.upsert_articles(&articles).await?;

    Ok(RefreshReport {
        active_feeds: config.feeds.len(),
        collected_articles,
        inserted_articles: inserted,
        updated_articles: updated,
        errors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::article::{Article, Source};
    use crate::config::{FeedConfig, Platform};
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
        assert!(
            storage
                .list_feeds()
                .await
                .unwrap()
                .iter()
                .all(|f| f.is_active)
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
            articles: vec![article("astronomy::jupiter", "astronomy", Source::Substack)],
            errors: Vec::new(),
        };
        refresh_with_collector(&storage, &config, || async { initial })
            .await
            .unwrap();

        let offline = CollectionReport {
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
        let database_path = directory.path().join("reader.db");
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
}
