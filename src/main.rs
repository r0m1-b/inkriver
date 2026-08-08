use reader::{config, service};
use std::path::PathBuf;

const CONFIG_FILE_NAME: &str = "feeds.toml";

/// Returns the development configuration path anchored to the Cargo project.
fn default_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(CONFIG_FILE_NAME)
}

/// Loads the configured feeds and prints their articles from newest to oldest.
///
/// # Errors
///
/// Returns an error when the configuration or any configured feed cannot be
/// loaded.
fn main() -> Result<(), String> {
    let config = config::load_config(&default_config_path()).map_err(|error| error.to_string())?;
    let report = service::collect_articles(&config);

    for error in &report.errors {
        eprintln!("{error}");
    }

    for article in &report.articles {
        println!("{}", service::format_article_summary(article));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
