use crate::article::Source;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub feeds: Vec<FeedConfig>,
}

#[derive(Debug, Deserialize, Clone)]
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

impl From<Platform> for Source {
    fn from(platform: Platform) -> Self {
        match platform {
            Platform::Medium => Self::Medium,
            Platform::Substack => Self::Substack,
            Platform::Other => Self::Other,
        }
    }
}

impl Platform {
    /// Returns the stable lowercase representation stored in SQLite.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Medium => "medium",
            Self::Substack => "substack",
            Self::Other => "other",
        }
    }
}

impl TryFrom<&str> for Platform {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "medium" => Ok(Self::Medium),
            "substack" => Ok(Self::Substack),
            "other" => Ok(Self::Other),
            _ => Err(format!("Unknown feed platform: {value}")),
        }
    }
}

/// Validates identifiers used to namespace articles from configured feeds.
///
/// # Errors
///
/// Returns an error when an identifier is blank or already used by another
/// configured feed.
fn validate_feed_ids(config: &Config) -> Result<()> {
    let mut feed_ids = HashSet::new();

    for feed in &config.feeds {
        if feed.id.trim().is_empty() {
            bail!("Feed id must not be blank");
        }

        if !feed_ids.insert(feed.id.as_str()) {
            bail!("Duplicate feed id: {}", feed.id);
        }
    }

    Ok(())
}

/// Loads and parses a configuration by using the supplied file-reading function.
///
/// The injected reader keeps parsing independent from the filesystem and makes
/// configuration loading straightforward to unit test.
///
/// # Errors
///
/// Returns an error when the reader cannot load the requested path or when the
/// resulting content is not valid TOML for [`Config`].
fn load_config_from_reader<F>(path: &Path, read_file: F) -> Result<Config>
where
    F: FnOnce(&Path) -> std::io::Result<String>,
{
    let content =
        read_file(path).with_context(|| format!("Impossible de lire {}", path.display()))?;

    let config = toml::from_str(&content)
        .with_context(|| format!("Invalid TOML format in {}", path.display()))?;

    validate_feed_ids(&config)?;

    Ok(config)
}

/// Loads the reader configuration from a TOML file.
///
/// # Errors
///
/// Returns an error when the file cannot be read or its content cannot be
/// deserialized into [`Config`].
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

    /// Verifies the exhaustive conversion from configured platforms to domain sources.
    #[test]
    fn convert_platforms_to_sources() {
        let cases = [
            (Platform::Medium, crate::article::Source::Medium),
            (Platform::Substack, crate::article::Source::Substack),
            (Platform::Other, crate::article::Source::Other),
        ];

        for (platform, expected_source) in cases {
            assert_eq!(crate::article::Source::from(platform), expected_source);
        }
    }

    /// Verifies every platform has a stable SQLite representation and round-trips.
    #[test]
    fn platform_round_trips_through_storage_value() {
        for platform in [Platform::Medium, Platform::Substack, Platform::Other] {
            assert_eq!(Platform::try_from(platform.as_str()), Ok(platform));
        }
    }

    /// Verifies corrupted or future platform values are not silently accepted.
    #[test]
    fn unknown_storage_platform_is_rejected() {
        assert_eq!(
            Platform::try_from("blog"),
            Err("Unknown feed platform: blog".to_string())
        );
    }

    /// Verifies that a valid TOML document produces the expected feed configuration.
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

    /// Verifies that an underlying file-reading error is returned to the caller.
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

    /// Verifies that malformed TOML produces a configuration parsing error.
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

    /// Verifies that empty and whitespace-only feed identifiers are rejected.
    #[test]
    fn reject_blank_feed_ids() {
        for id in ["", "   "] {
            let content = format!(
                r#"
                    [[feeds]]
                    id = "{id}"
                    platform = "substack"
                    url = "https://astronomy.example/feed"
                "#
            );
            let fake_reader = |_path: &Path| Ok(content);

            let error = load_config_from_reader(Path::new("blank-id.toml"), fake_reader)
                .expect_err("a blank feed id should be rejected");

            assert!(error.to_string().contains("Feed id must not be blank"));
        }
    }

    /// Verifies that two configured feeds cannot share the same identifier.
    #[test]
    fn reject_duplicate_feed_ids() {
        let content = r#"
            [[feeds]]
            id = "astronomy"
            platform = "substack"
            url = "https://astronomy.example/feed"

            [[feeds]]
            id = "astronomy"
            platform = "medium"
            url = "https://medium.com/feed/@astronomy"
        "#;
        let fake_reader = |_path: &Path| Ok(content.to_string());

        let error = load_config_from_reader(Path::new("duplicate-id.toml"), fake_reader)
            .expect_err("duplicate feed ids should be rejected");

        assert!(error.to_string().contains("Duplicate feed id: astronomy"));
    }
}
