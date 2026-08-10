use crate::article::{Article, ContentKind, Source};
use crate::config::{FeedConfig, FeedUrlError, Platform, detect_platform, normalize_feed_url};
use anyhow::{Context, Result};
use sqlx::SqlitePool;
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::path::Path;
use std::{error::Error, fmt};

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
    Database(String),
}

impl fmt::Display for SubscriptionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl(error) => error.fmt(formatter),
            Self::DuplicateActiveUrl(url) => write!(formatter, "Feed URL is already active: {url}"),
            Self::NotFound(id) => write!(formatter, "Feed not found: {id}"),
            Self::Database(message) => write!(formatter, "SQLite subscription error: {message}"),
        }
    }
}

impl Error for SubscriptionError {}

/// Combines remote article data with reader-specific local state.
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
        let existing: Option<(String, bool)> = sqlx::query_as(
            "SELECT id, is_active FROM feeds WHERE url = ? ORDER BY is_active DESC, id LIMIT 1",
        )
        .bind(&url)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| SubscriptionError::Database(error.to_string()))?;

        if let Some((id, is_active)) = existing {
            if is_active {
                return Err(SubscriptionError::DuplicateActiveUrl(url));
            }
            sqlx::query("UPDATE feeds SET platform = ?, is_active = 1 WHERE id = ?")
                .bind(platform.as_str())
                .bind(&id)
                .execute(&self.pool)
                .await
                .map_err(|error| SubscriptionError::Database(error.to_string()))?;
            return Ok(StoredFeed {
                id,
                platform,
                url,
                is_active: true,
            });
        }

        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO feeds (id, platform, url, is_active) VALUES (?, ?, ?, 1)")
            .bind(&id)
            .bind(platform.as_str())
            .bind(&url)
            .execute(&self.pool)
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

        Ok(StoredFeed {
            id,
            platform,
            url,
            is_active: true,
        })
    }

    /// Activates or deactivates a retained subscription without deleting history.
    pub async fn set_feed_active(
        &self,
        feed_id: &str,
        is_active: bool,
    ) -> std::result::Result<StoredFeed, SubscriptionError> {
        let feed: Option<(String, String, String, bool)> =
            sqlx::query_as("SELECT id, platform, url, is_active FROM feeds WHERE id = ?")
                .bind(feed_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|error| SubscriptionError::Database(error.to_string()))?;
        let (id, platform, url, _) =
            feed.ok_or_else(|| SubscriptionError::NotFound(feed_id.to_string()))?;

        if is_active {
            let duplicate: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM feeds WHERE url = ? AND is_active = 1 AND id <> ?)",
            )
            .bind(&url)
            .bind(&id)
            .fetch_one(&self.pool)
            .await
            .map_err(|error| SubscriptionError::Database(error.to_string()))?;
            if duplicate {
                return Err(SubscriptionError::DuplicateActiveUrl(url));
            }
        }

        sqlx::query("UPDATE feeds SET is_active = ? WHERE id = ?")
            .bind(is_active)
            .bind(&id)
            .execute(&self.pool)
            .await
            .map_err(|error| SubscriptionError::Database(error.to_string()))?;
        let platform =
            Platform::try_from(platform.as_str()).map_err(SubscriptionError::Database)?;
        Ok(StoredFeed {
            id,
            platform,
            url,
            is_active,
        })
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

        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM feeds WHERE id = ?)")
            .bind(feed_id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(|error| SubscriptionError::Database(error.to_string()))?;
        if !exists {
            return Err(SubscriptionError::NotFound(feed_id.to_string()));
        }

        let deleted_articles = sqlx::query("DELETE FROM articles WHERE feed_id = ?")
            .bind(feed_id)
            .execute(&mut *transaction)
            .await
            .map_err(|error| SubscriptionError::Database(error.to_string()))?
            .rows_affected() as usize;

        sqlx::query("DELETE FROM feeds WHERE id = ?")
            .bind(feed_id)
            .execute(&mut *transaction)
            .await
            .map_err(|error| SubscriptionError::Database(error.to_string()))?;

        transaction
            .commit()
            .await
            .map_err(|error| SubscriptionError::Database(error.to_string()))?;

        Ok(DeleteFeedResult {
            feed_id: feed_id.to_string(),
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
            let already_exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM articles WHERE id = ?)")
                    .bind(&article.id)
                    .fetch_one(&mut *transaction)
                    .await
                    .with_context(|| {
                        format!("Impossible de rechercher l'article {:?}", article.id)
                    })?;

            sqlx::query(
                r#"
                    INSERT INTO articles (
                        id, feed_id, title, author, published_at, url, content,
                        content_kind, source
                    )
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                    ON CONFLICT(id) DO UPDATE SET
                        feed_id = excluded.feed_id,
                        title = COALESCE(excluded.title, articles.title),
                        author = COALESCE(excluded.author, articles.author),
                        published_at = COALESCE(excluded.published_at, articles.published_at),
                        url = COALESCE(excluded.url, articles.url),
                        content = COALESCE(excluded.content, articles.content),
                        content_kind = CASE
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
            .execute(&mut *transaction)
            .await
            .with_context(|| format!("Impossible d'enregistrer l'article {:?}", article.id))?;

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
                WHERE id = ?
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
        let result = sqlx::query("UPDATE articles SET is_read = ? WHERE id = ?")
            .bind(is_read)
            .bind(article_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("Impossible de modifier l'état lu de {article_id:?}"))?;

        Ok(result.rows_affected() == 1)
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
        let result = sqlx::query("UPDATE articles SET is_favorite = ? WHERE id = ?")
            .bind(is_favorite)
            .bind(article_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("Impossible de modifier le favori {article_id:?}"))?;

        Ok(result.rows_affected() == 1)
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
            content_kind: ContentKind::Full,
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

        assert_eq!(migration_count, 2);

        storage.close().await;
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
            .add_feed("https://medium.com/feed/@reader", Some(Platform::Other))
            .await
            .unwrap();
        assert_eq!(stored.platform, Platform::Other);

        let error = storage
            .add_feed("https://medium.com/feed/@reader#fragment", None)
            .await
            .unwrap_err();
        assert_eq!(
            error,
            SubscriptionError::DuplicateActiveUrl("https://medium.com/feed/@reader".to_string())
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

        let result = storage.delete_feed("astronomy").await.unwrap();

        assert_eq!(
            result,
            DeleteFeedResult {
                feed_id: "astronomy".to_string(),
                deleted_articles: 2,
            }
        );
        assert_eq!(
            storage.list_feeds().await.unwrap(),
            vec![StoredFeed {
                id: "bread".to_string(),
                platform: Platform::Medium,
                url: "https://medium.com/feed/@bread".to_string(),
                is_active: true,
            }]
        );
        let articles = storage.list_articles().await.unwrap();
        assert_eq!(articles.len(), 1);
        assert_eq!(articles[0].article.id, "bread::starter");
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
