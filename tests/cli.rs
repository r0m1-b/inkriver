use chrono::{TimeZone, Utc};
use reader::article::{Article, Source};
use reader::config::{FeedConfig, Platform};
use reader::storage::Storage;
use std::process::{Command, Output};

fn reader_command(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_reader"))
        .args(arguments)
        .output()
        .unwrap()
}

async fn create_cached_database(database_path: &std::path::Path) {
    let storage = Storage::open(database_path).await.unwrap();
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
            url: Some("https://astronomy.example/jupiter".to_string()),
            content: Some("<p>Jupiter reste visible hors ligne.</p>".to_string()),
            source: Source::Substack,
        }])
        .await
        .unwrap();
    storage.close().await;
}

/// Verifies the executable displays Clap help instead of refreshing implicitly.
#[test]
fn no_argument_displays_help() {
    let output = reader_command(&[]);
    let stderr = String::from_utf8(output.stderr).unwrap();

    assert!(!output.status.success());
    assert!(stderr.contains("Usage:"));
    assert!(stderr.contains("Commands:"));
}

/// Verifies the real executable lists cached articles without TOML or network.
#[tokio::test]
async fn list_reads_cached_articles_without_configuration() {
    let directory = tempfile::tempdir().unwrap();
    let database_path = directory.path().join("reader.db");
    let missing_config_path = directory.path().join("missing.toml");
    create_cached_database(&database_path).await;

    let output = reader_command(&[
        "--config",
        missing_config_path.to_str().unwrap(),
        "--database",
        database_path.to_str().unwrap(),
        "list",
    ]);

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Repérer Jupiter sans télescope"));
    assert!(stdout.contains("astronomy::jupiter"));
}
