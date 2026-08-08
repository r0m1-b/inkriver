use crate::article::{Article, Source};
use crate::config::{FeedConfig, Platform};
use anyhow::{Context, Result};
use sqlx::SqlitePool;
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::path::Path;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// Owns the SQLite connection pool used by the reader core.
pub struct Storage {
    pool: SqlitePool,
}

/// Represents one subscription as persisted by the application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFeed {
    pub id: String,
    pub platform: Platform,
    pub url: String,
    pub is_active: bool,
}

/// Combines remote article data with reader-specific local state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredArticle {
    pub article: Article,
    pub is_read: bool,
    pub is_favorite: bool,
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

        Ok(Self { pool })
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
        let rows: Vec<(String, String, String, bool)> =
            sqlx::query_as("SELECT id, platform, url, is_active FROM feeds ORDER BY id ASC")
                .fetch_all(&self.pool)
                .await
                .context("Impossible de charger les abonnements")?;

        rows.into_iter()
            .map(|(id, platform, url, is_active)| {
                let platform = Platform::try_from(platform.as_str()).map_err(anyhow::Error::msg)?;
                Ok(StoredFeed {
                    id,
                    platform,
                    url,
                    is_active,
                })
            })
            .collect()
    }

    /// Inserts new articles and refreshes existing remote metadata.
    ///
    /// Missing incoming values do not erase data previously stored, and local
    /// read/favorite flags are intentionally absent from the conflict update.
    ///
    /// # Errors
    ///
    /// Returns an error when the complete article transaction cannot be applied.
    pub async fn upsert_articles(&self, articles: &[Article]) -> Result<()> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("Impossible de commencer l'enregistrement des articles")?;

        for article in articles {
            sqlx::query(
                r#"
                    INSERT INTO articles (
                        id, feed_id, title, author, published_at, url, content, source
                    )
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                    ON CONFLICT(id) DO UPDATE SET
                        feed_id = excluded.feed_id,
                        title = COALESCE(excluded.title, articles.title),
                        author = COALESCE(excluded.author, articles.author),
                        published_at = COALESCE(excluded.published_at, articles.published_at),
                        url = COALESCE(excluded.url, articles.url),
                        content = COALESCE(excluded.content, articles.content),
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
            .bind(article.source.as_str())
            .execute(&mut *transaction)
            .await
            .with_context(|| format!("Impossible d'enregistrer l'article {:?}", article.id))?;
        }

        transaction
            .commit()
            .await
            .context("Impossible de valider l'enregistrement des articles")
    }

    /// Lists all retained articles from newest to oldest, with undated entries last.
    ///
    /// # Errors
    ///
    /// Returns an error when rows cannot be loaded or contain an unknown source.
    pub async fn list_articles(&self) -> Result<Vec<StoredArticle>> {
        type ArticleRow = (
            String,
            String,
            Option<String>,
            Option<String>,
            Option<chrono::DateTime<chrono::Utc>>,
            Option<String>,
            Option<String>,
            String,
            bool,
            bool,
        );

        let rows: Vec<ArticleRow> = sqlx::query_as(
            r#"
                SELECT id, feed_id, title, author, published_at, url, content,
                       source, is_read, is_favorite
                FROM articles
                ORDER BY published_at IS NULL ASC, published_at DESC, id ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .context("Impossible de charger les articles")?;

        rows.into_iter()
            .map(
                |(
                    id,
                    feed_id,
                    title,
                    author,
                    published_at,
                    url,
                    content,
                    source,
                    is_read,
                    is_favorite,
                )| {
                    let source = Source::try_from(source.as_str()).map_err(anyhow::Error::msg)?;
                    Ok(StoredArticle {
                        article: Article {
                            id,
                            feed_id,
                            title,
                            author,
                            published_at,
                            url,
                            content,
                            source,
                        },
                        is_read,
                        is_favorite,
                    })
                },
            )
            .collect()
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

    fn article(id: &str, feed_id: &str, published_at: Option<chrono::DateTime<Utc>>) -> Article {
        Article {
            id: id.to_string(),
            feed_id: feed_id.to_string(),
            title: Some(format!("Title for {id}")),
            author: Some("Test Author".to_string()),
            published_at,
            url: Some(format!("https://articles.example/{id}")),
            content: Some(format!("Readable content for {id}")),
            source: Source::Substack,
        }
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

    /// Verifies a file database is created with migrations and foreign keys enabled.
    #[tokio::test]
    async fn open_creates_database_and_applies_migrations() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("reader.db");

        let storage = Storage::open(&database_path).await.unwrap();

        assert!(database_path.is_file());

        let table_names: Vec<String> =
            sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
                .fetch_all(&storage.pool)
                .await
                .unwrap();
        assert!(table_names.contains(&"feeds".to_string()));
        assert!(table_names.contains(&"articles".to_string()));

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

        assert_eq!(migration_count, 1);

        storage.close().await;
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
                StoredFeed {
                    id: "astronomy".to_string(),
                    platform: Platform::Substack,
                    url: "https://astronomy.example/feed".to_string(),
                    is_active: true,
                },
                StoredFeed {
                    id: "bread".to_string(),
                    platform: Platform::Medium,
                    url: "https://medium.com/feed/@bread".to_string(),
                    is_active: true,
                },
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
                StoredFeed {
                    id: "astronomy".to_string(),
                    platform: Platform::Other,
                    url: "https://astronomy.example/new-feed".to_string(),
                    is_active: true,
                },
                StoredFeed {
                    id: "bread".to_string(),
                    platform: Platform::Medium,
                    url: "https://medium.com/feed/@bread".to_string(),
                    is_active: false,
                },
            ]
        );
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
            vec![StoredFeed {
                id: original.id,
                platform: original.platform,
                url: original.url,
                is_active: true,
            }]
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
                article: expected,
                is_read: false,
                is_favorite: false,
            }]
        );
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
        storage.upsert_articles(&[update]).await.unwrap();

        let stored = storage.list_articles().await.unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(
            stored[0].article.title.as_deref(),
            Some("Jupiter after opposition")
        );
        assert_eq!(stored[0].article.author, original.author);
        assert_eq!(stored[0].article.url, original.url);
        assert_eq!(stored[0].article.content, original.content);
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

    /// Verifies article batches are atomic when one row references an unknown feed.
    #[tokio::test]
    async fn upsert_articles_is_atomic() {
        let storage = storage_with_feed().await;
        let valid = article("astronomy::valid", "astronomy", None);
        let invalid = article("missing::invalid", "missing", None);

        assert!(storage.upsert_articles(&[valid, invalid]).await.is_err());
        assert!(storage.list_articles().await.unwrap().is_empty());
    }
}
