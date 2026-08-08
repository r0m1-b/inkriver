use reader::service::CollectionReport;
use reader::{config, refresh, service, storage::Storage};
use std::io::Write;
use std::path::PathBuf;

const CONFIG_FILE_NAME: &str = "feeds.toml";
const DATABASE_FILE_NAME: &str = "reader.db";

/// Returns the development configuration path anchored to the Cargo project.
fn default_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(CONFIG_FILE_NAME)
}

/// Returns the development database path anchored to the Cargo project.
fn default_database_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(DATABASE_FILE_NAME)
}

/// Writes collection errors and every retained article to their respective streams.
async fn print_stored_results<Out, Err>(
    storage: &Storage,
    report: &CollectionReport,
    stdout: &mut Out,
    stderr: &mut Err,
) -> Result<(), String>
where
    Out: Write,
    Err: Write,
{
    for error in &report.errors {
        writeln!(stderr, "{error}").map_err(|error| error.to_string())?;
    }

    let articles = storage
        .list_articles()
        .await
        .map_err(|error| error.to_string())?;
    for stored in articles {
        writeln!(
            stdout,
            "{}",
            service::format_article_summary(&stored.article)
        )
        .map_err(|error| error.to_string())?;
    }

    Ok(())
}

/// Loads the configured feeds and prints their articles from newest to oldest.
///
/// # Errors
///
/// Returns an error when the configuration or any configured feed cannot be
/// loaded.
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), String> {
    let config = config::load_config(&default_config_path()).map_err(|error| error.to_string())?;
    let storage = Storage::open(&default_database_path())
        .await
        .map_err(|error| error.to_string())?;
    let report = refresh::refresh(&storage, &config)
        .await
        .map_err(|error| error.to_string())?;
    print_stored_results(
        &storage,
        &report,
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    )
    .await?;
    storage.close().await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use reader::article::{Article, Source};
    use reader::config::{FeedConfig, Platform};
    use reader::service::{FeedCollectionError, FeedLoadError, FeedLoadStage};
    use std::path::Path;

    /// Verifies that the default configuration path never depends on the process CWD.
    #[test]
    fn default_config_path_is_anchored_to_manifest_directory() {
        let path = default_config_path();

        assert!(path.is_absolute());
        assert_eq!(path.file_name().unwrap(), "feeds.toml");
        assert_eq!(
            path.parent().unwrap(),
            Path::new(env!("CARGO_MANIFEST_DIR"))
        );
    }

    /// Verifies the development database remains local to the Cargo project.
    #[test]
    fn default_database_path_is_anchored_to_manifest_directory() {
        let path = default_database_path();

        assert!(path.is_absolute());
        assert_eq!(path.file_name().unwrap(), "reader.db");
        assert_eq!(
            path.parent().unwrap(),
            Path::new(env!("CARGO_MANIFEST_DIR"))
        );
    }

    /// Verifies offline errors and previously stored articles are both displayed.
    #[tokio::test]
    async fn print_stored_results_keeps_cached_articles_visible_on_failure() {
        let directory = tempfile::tempdir().unwrap();
        let storage = Storage::open(&directory.path().join("reader.db"))
            .await
            .unwrap();
        storage
            .import_feeds(&[FeedConfig {
                id: "astronomy".to_string(),
                platform: Platform::Substack,
                url: "https://feeds.example/astronomy".to_string(),
            }])
            .await
            .unwrap();
        storage
            .upsert_articles(&[Article {
                id: "astronomy::jupiter".to_string(),
                feed_id: "astronomy".to_string(),
                title: Some("Repérer Jupiter sans télescope".to_string()),
                author: Some("Claire du Ciel".to_string()),
                published_at: Some(Utc.with_ymd_and_hms(2026, 8, 8, 12, 0, 0).unwrap()),
                url: Some("https://articles.example/jupiter".to_string()),
                content: Some("Jupiter reste visible malgré la panne réseau.".to_string()),
                source: Source::Substack,
            }])
            .await
            .unwrap();
        let report = CollectionReport {
            articles: Vec::new(),
            errors: vec![FeedCollectionError {
                feed_id: "astronomy".to_string(),
                feed_url: "https://feeds.example/astronomy".to_string(),
                error: FeedLoadError {
                    stage: FeedLoadStage::HttpRequest,
                    message: "network unavailable".to_string(),
                },
            }],
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        print_stored_results(&storage, &report, &mut stdout, &mut stderr)
            .await
            .unwrap();

        let stdout = String::from_utf8(stdout).unwrap();
        let stderr = String::from_utf8(stderr).unwrap();
        assert!(stdout.contains("Repérer Jupiter sans télescope"));
        assert!(stderr.contains("network unavailable"));
    }
}
