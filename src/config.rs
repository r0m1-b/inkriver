use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub feeds: Vec<FeedConfig>,
}

#[derive(Debug, Deserialize)]
pub struct FeedConfig {
    pub id: String,
    pub platform: Platform,
    pub url: String,
}

#[derive(Debug, Deserialize, PartialEq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Medium,
    Substack,
    Other,
}

fn load_config_from_reader<F>(path: &Path, read_file: F) -> Result<Config>
where
    F: FnOnce(&Path) -> std::io::Result<String>,
{
    let content =
        read_file(path).with_context(|| format!("Impossible de lire {}", path.display()))?;

    let config = toml::from_str(&content)
        .with_context(|| format!("Invalid TOML format in {}", path.display()))?;

    Ok(config)
}

pub fn load_config(path: &Path) -> Result<Config> {
    load_config_from_reader(path, |path| fs::read_to_string(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::path::Path;

    const VALID_CONFIG: &str = r#"
        [[feeds]]
        id = "astronomy"
        platform = "substack"
        url = "https://astronomy.example/feed"

        [[feeds]]
        id = "bread"
        platform = "medium"
        url = "https://medium.com/feed/@bread"
    "#;

    #[test]
    fn load_valid_config_from_reader() {
        let fake_reader = |_path: &Path| Ok(VALID_CONFIG.to_string());

        let config: Config =
            load_config_from_reader(Path::new("unused.toml"), fake_reader).unwrap();

        assert_eq!(config.feeds.len(), 2);
        assert_eq!(config.feeds[0].id, "astronomy");
        assert_eq!(config.feeds[0].platform, Platform::Substack);
        assert_eq!(config.feeds[0].url, "https://astronomy.example/feed");
        assert_eq!(config.feeds[1].id, "bread");
        assert_eq!(config.feeds[1].platform, Platform::Medium);
        assert_eq!(config.feeds[1].url, "https://medium.com/feed/@bread");
    }

    #[test]
    fn return_error_for_invalid_file() {
        let fake_reader: fn(&Path) -> std::io::Result<String> = |_path: &Path| {
            Err::<String, std::io::Error>(io::Error::new(
                io::ErrorKind::NotFound,
                "Reading forbidden.",
            ))
        };

        let result = load_config_from_reader(Path::new("nonexistent.toml"), fake_reader);
        assert!(result.is_err());

        let error = result.unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("Reading forbidden."));
    }

    #[test]
    fn return_error_for_invalid_toml() {
        let fake_reader: fn(&Path) -> std::io::Result<String> = |_path: &Path| {
            Ok(r#"
                    [[feeds]]
                    id = "broken
                    platform = ???"
                "#
            .to_string())
        };

        let result = load_config_from_reader(Path::new("invalid.toml"), fake_reader);
        assert!(result.is_err());

        let error = result.unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("Invalid TOML format"));
    }
}
