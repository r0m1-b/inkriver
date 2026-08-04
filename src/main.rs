use reader::{config, service};
use std::path::Path;

const CONFIG_PATH: &str = "feeds.toml";

/// Loads the configured feeds and prints their articles from newest to oldest.
///
/// # Errors
///
/// Returns an error when the configuration or any configured feed cannot be
/// loaded.
fn main() -> Result<(), String> {
    let config = config::load_config(Path::new(CONFIG_PATH)).map_err(|error| error.to_string())?;
    let report = service::collect_articles(&config);

    for error in &report.errors {
        eprintln!("{error}");
    }

    for article in &report.articles {
        println!("{}", service::format_article_summary(article));
    }

    Ok(())
}
