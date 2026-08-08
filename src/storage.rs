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
}
