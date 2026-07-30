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
}

pub fn load_config(path: &Path) -> Result<Config> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Impossible de lire {}", path.display()))?;

    let config = toml::from_str(&content)
        .with_context(|| format!("Configuration TOML invalide dans {}", path.display()))?;

    Ok(config)
}
