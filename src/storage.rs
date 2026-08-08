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

    fn feed(id: &str, platform: Platform, url: &str) -> FeedConfig {
        FeedConfig {
            id: id.to_string(),
            platform,
            url: url.to_string(),
        }
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
}
