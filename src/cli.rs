use crate::config;
use crate::refresh;
use crate::refresh::RefreshReport;
use crate::storage::{ArticleSummary, Storage, StoredArticle};
use clap::{Parser, Subcommand};
use std::fmt;
use std::path::PathBuf;

const CONFIG_FILE_NAME: &str = "feeds.toml";
const DATABASE_FILE_NAME: &str = "inkriver.db";
const CONTENT_WIDTH: usize = 88;

/// Lecteur unifié de flux RSS et Atom.
#[derive(Debug, Parser, PartialEq, Eq)]
#[command(version, about, arg_required_else_help = true)]
pub struct Cli {
    /// Utiliser un autre fichier de configuration des flux.
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Utiliser un autre fichier de base SQLite.
    #[arg(long, global = true, value_name = "PATH")]
    pub database: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

/// Opérations proposées par le CLI.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum Command {
    /// Télécharger les flux configurés et actualiser SQLite.
    Refresh,
    /// Lister les articles stockés sans utiliser le réseau.
    List,
    /// Afficher un article stocké et le marquer comme lu.
    Show { selector: String },
    /// Marquer un article stocké comme lu.
    MarkRead { selector: String },
    /// Marquer un article stocké comme non lu.
    MarkUnread { selector: String },
    /// Ajouter un article stocké aux favoris.
    Favorite { selector: String },
    /// Retirer un article stocké des favoris.
    Unfavorite { selector: String },
}

/// Text and process status produced by one successfully handled command.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: u8,
}

/// User-facing categories for fatal CLI failures.
#[derive(Debug, PartialEq, Eq)]
pub enum CliError {
    Configuration(String),
    Database(String),
    ArticleNotFound(String),
    Rendering(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration(message) => {
                write!(formatter, "Erreur de configuration : {message}")
            }
            Self::Database(message) => write!(formatter, "Erreur SQLite : {message}"),
            Self::ArticleNotFound(selector) => {
                write!(formatter, "Article introuvable : {selector}")
            }
            Self::Rendering(message) => write!(formatter, "Erreur de rendu : {message}"),
        }
    }
}

impl std::error::Error for CliError {}

/// Returns the development configuration path anchored to the Cargo project.
pub fn default_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(CONFIG_FILE_NAME)
}

/// Returns the development database path anchored to the Cargo project.
pub fn default_database_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(DATABASE_FILE_NAME)
}

fn format_summary(index: usize, summary: &ArticleSummary) -> String {
    let read = if summary.is_read { "lu" } else { "non lu" };
    let favorite = if summary.is_favorite { "favori" } else { "-" };
    let published_at = summary
        .published_at
        .map(|date| date.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "date inconnue".to_string());
    let title = summary.title.as_deref().unwrap_or("(sans titre)");

    format!(
        "{index} | {read}, {favorite} | {published_at} | {:?} | {title} | {}",
        summary.source, summary.id
    )
}

fn resolve_selector<'a>(summaries: &'a [ArticleSummary], selector: &str) -> Option<&'a str> {
    summaries
        .iter()
        .find(|summary| summary.id == selector)
        .map(|summary| summary.id.as_str())
        .or_else(|| {
            selector
                .parse::<usize>()
                .ok()
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| summaries.get(index))
                .map(|summary| summary.id.as_str())
        })
}

fn render_content(content: Option<&str>) -> Result<String, CliError> {
    match content {
        Some(content) => html2text::from_read(content.as_bytes(), CONTENT_WIDTH)
            .map(|text| text.trim().to_string())
            .map_err(|error| CliError::Rendering(error.to_string())),
        None => Ok("(aucun contenu stocké)".to_string()),
    }
}

fn format_article_detail(stored: &StoredArticle, content: &str) -> String {
    let article = &stored.article;
    let title = article.title.as_deref().unwrap_or("(sans titre)");
    let author = article.author.as_deref().unwrap_or("auteur inconnu");
    let published_at = article
        .published_at
        .map(|date| date.to_rfc3339())
        .unwrap_or_else(|| "date inconnue".to_string());
    let url = article.url.as_deref().unwrap_or("(aucune URL)");

    format!(
        "{title}\nAuteur : {author}\nDate : {published_at}\nSource : {:?}\nURL : {url}\n\n{content}\n",
        article.source
    )
}

fn format_refresh_report(report: &RefreshReport) -> CommandOutput {
    let stderr = report
        .errors
        .iter()
        .map(|error| format!("{error}\n"))
        .collect();
    let exit_code = if report.errors.is_empty() { 0 } else { 2 };

    CommandOutput {
        stdout: format!(
            "Flux actifs : {}\nArticles reçus : {}\nNouveaux articles : {}\nArticles mis à jour : {}\nFlux en erreur : {}\n",
            report.active_feeds,
            report.collected_articles,
            report.inserted_articles,
            report.updated_articles,
            report.errors.len()
        ),
        stderr,
        exit_code,
    }
}

async fn resolve_from_storage(storage: &Storage, selector: &str) -> Result<String, CliError> {
    let summaries = storage
        .list_article_summaries()
        .await
        .map_err(|error| CliError::Database(format!("{error:#}")))?;
    resolve_selector(&summaries, selector)
        .map(str::to_string)
        .ok_or_else(|| CliError::ArticleNotFound(selector.to_string()))
}

async fn run_with_storage(
    storage: &Storage,
    command: &Command,
    config_path: &std::path::Path,
) -> Result<CommandOutput, CliError> {
    match command {
        Command::Refresh => {
            let config = config::load_config(config_path)
                .map_err(|error| CliError::Configuration(format!("{error:#}")))?;
            let report = refresh::refresh(storage, &config)
                .await
                .map_err(|error| CliError::Database(format!("{error:#}")))?;
            Ok(format_refresh_report(&report))
        }
        Command::List => {
            let summaries = storage
                .list_article_summaries()
                .await
                .map_err(|error| CliError::Database(format!("{error:#}")))?;
            let stdout = summaries
                .iter()
                .enumerate()
                .map(|(index, summary)| format!("{}\n", format_summary(index + 1, summary)))
                .collect();
            Ok(CommandOutput {
                stdout,
                ..CommandOutput::default()
            })
        }
        Command::Show { selector } => {
            let article_id = resolve_from_storage(storage, selector).await?;
            let stored = storage
                .get_article(&article_id)
                .await
                .map_err(|error| CliError::Database(format!("{error:#}")))?
                .ok_or_else(|| CliError::ArticleNotFound(selector.clone()))?;
            let content = render_content(stored.article.content.as_deref())?;
            storage
                .set_read(&article_id, true)
                .await
                .map_err(|error| CliError::Database(format!("{error:#}")))?;
            Ok(CommandOutput {
                stdout: format_article_detail(&stored, &content),
                ..CommandOutput::default()
            })
        }
        Command::MarkRead { selector } | Command::MarkUnread { selector } => {
            let article_id = resolve_from_storage(storage, selector).await?;
            let is_read = matches!(command, Command::MarkRead { .. });
            storage
                .set_read(&article_id, is_read)
                .await
                .map_err(|error| CliError::Database(format!("{error:#}")))?;
            let label = if is_read { "lu" } else { "non lu" };
            Ok(CommandOutput {
                stdout: format!("Article {article_id} marqué comme {label}.\n"),
                ..CommandOutput::default()
            })
        }
        Command::Favorite { selector } | Command::Unfavorite { selector } => {
            let article_id = resolve_from_storage(storage, selector).await?;
            let is_favorite = matches!(command, Command::Favorite { .. });
            storage
                .set_favorite(&article_id, is_favorite)
                .await
                .map_err(|error| CliError::Database(format!("{error:#}")))?;
            let label = if is_favorite {
                "ajouté aux favoris"
            } else {
                "retiré des favoris"
            };
            Ok(CommandOutput {
                stdout: format!("Article {article_id} {label}.\n"),
                ..CommandOutput::default()
            })
        }
    }
}

/// Executes one parsed command and closes its SQLite pool before returning.
///
/// # Errors
///
/// Returns a categorized error for invalid configuration, SQLite failures,
/// unknown articles, or HTML rendering failures.
pub async fn run(cli: Cli) -> Result<CommandOutput, CliError> {
    let config_path = cli.config.unwrap_or_else(default_config_path);
    let database_path = cli.database.unwrap_or_else(default_database_path);
    let storage = Storage::open(&database_path)
        .await
        .map_err(|error| CliError::Database(format!("{error:#}")))?;
    let result = run_with_storage(&storage, &cli.command, &config_path).await;
    storage.close().await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::article::{Article, ContentKind, Source};
    use crate::config::{FeedConfig, Platform};
    use crate::service::{FeedCollectionError, FeedLoadError, FeedLoadStage};
    use chrono::{TimeZone, Utc};
    use std::path::Path;

    fn article(id: &str, title: &str, content: Option<&str>) -> Article {
        Article {
            id: id.to_string(),
            feed_id: "astronomy".to_string(),
            title: Some(title.to_string()),
            author: Some("Claire du Ciel".to_string()),
            published_at: Some(Utc.with_ymd_and_hms(2026, 8, 8, 12, 0, 0).unwrap()),
            url: Some("https://articles.example/jupiter".to_string()),
            content: content.map(str::to_string),
            content_kind: if content.is_some() {
                ContentKind::Full
            } else {
                ContentKind::Missing
            },
            source: Source::Substack,
        }
    }

    async fn populated_database() -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("inkriver.db");
        let storage = Storage::open(&database_path).await.unwrap();
        storage
            .import_feeds(&[FeedConfig {
                id: "astronomy".to_string(),
                platform: Platform::Substack,
                url: "https://feeds.example/astronomy".to_string(),
            }])
            .await
            .unwrap();
        storage
            .upsert_articles(&[
                article(
                    "astronomy::jupiter",
                    "Repérer Jupiter",
                    Some("<p>Jupiter est <strong>très brillante</strong>.</p>"),
                ),
                article("astronomy::mars", "Observer Mars", None),
            ])
            .await
            .unwrap();
        storage.close().await;
        (directory, database_path)
    }

    fn offline_cli(database: PathBuf, command: Command) -> Cli {
        Cli {
            config: Some(PathBuf::from("definitely-missing.toml")),
            database: Some(database),
            command,
        }
    }

    /// Verifies Clap exposes every agreed command and global path option.
    #[test]
    fn parse_all_cli_commands() {
        let cases = [
            ("refresh", Command::Refresh),
            ("list", Command::List),
            (
                "show",
                Command::Show {
                    selector: "article-id".to_string(),
                },
            ),
            (
                "mark-read",
                Command::MarkRead {
                    selector: "article-id".to_string(),
                },
            ),
            (
                "mark-unread",
                Command::MarkUnread {
                    selector: "article-id".to_string(),
                },
            ),
            (
                "favorite",
                Command::Favorite {
                    selector: "article-id".to_string(),
                },
            ),
            (
                "unfavorite",
                Command::Unfavorite {
                    selector: "article-id".to_string(),
                },
            ),
        ];

        for (name, expected) in cases {
            let mut arguments = vec![
                "inkriver",
                "--database",
                "custom.db",
                "--config",
                "custom.toml",
                name,
            ];
            if !matches!(expected, Command::Refresh | Command::List) {
                arguments.push("article-id");
            }
            let cli = Cli::try_parse_from(arguments).unwrap();
            assert_eq!(cli.command, expected);
            assert_eq!(cli.database, Some(PathBuf::from("custom.db")));
            assert_eq!(cli.config, Some(PathBuf::from("custom.toml")));
        }
    }

    /// Verifies an invocation without a subcommand produces the help text.
    #[test]
    fn no_command_displays_help() {
        let error = Cli::try_parse_from(["inkriver"]).unwrap_err();
        assert!(error.to_string().contains("Usage:"));
        assert!(error.to_string().contains("Commands:"));
    }

    /// Verifies development paths never depend on the process working directory.
    #[test]
    fn default_paths_are_anchored_to_manifest_directory() {
        for (path, file_name) in [
            (default_config_path(), "feeds.toml"),
            (default_database_path(), "inkriver.db"),
        ] {
            assert!(path.is_absolute());
            assert_eq!(path.file_name().unwrap(), file_name);
            assert_eq!(
                path.parent().unwrap(),
                Path::new(env!("CARGO_MANIFEST_DIR"))
            );
        }
    }

    /// Verifies article IDs and one-based list positions are both accepted.
    #[test]
    fn resolve_selector_accepts_id_or_one_based_index() {
        let summaries = vec![ArticleSummary {
            id: "astronomy::jupiter".to_string(),
            feed_id: "astronomy".to_string(),
            title: None,
            author: None,
            published_at: None,
            url: None,
            source: Source::Substack,
            is_read: false,
            is_favorite: false,
        }];

        assert_eq!(
            resolve_selector(&summaries, "astronomy::jupiter"),
            Some("astronomy::jupiter")
        );
        assert_eq!(
            resolve_selector(&summaries, "1"),
            Some("astronomy::jupiter")
        );
        assert_eq!(resolve_selector(&summaries, "0"), None);
        assert_eq!(resolve_selector(&summaries, "2"), None);
    }

    /// Verifies sanitized HTML becomes readable plain terminal text.
    #[test]
    fn render_content_converts_html_to_text() {
        let rendered =
            render_content(Some("<p>Une planète <strong>très brillante</strong>.</p>")).unwrap();

        assert!(rendered.contains("Une planète"));
        assert!(rendered.contains("très brillante"));
        assert!(!rendered.contains('<'));
        assert_eq!(render_content(None).unwrap(), "(aucun contenu stocké)");
    }

    /// Verifies partial refreshes preserve their counts and return exit code 2.
    #[test]
    fn format_partial_refresh_report_uses_nonzero_exit_code() {
        let report = RefreshReport {
            active_feeds: 2,
            collected_articles: 5,
            inserted_articles: 2,
            updated_articles: 3,
            errors: vec![FeedCollectionError {
                feed_id: "bread".to_string(),
                feed_url: "https://feeds.example/bread".to_string(),
                error: FeedLoadError {
                    stage: FeedLoadStage::HttpRequest,
                    message: "network unavailable".to_string(),
                },
            }],
        };

        let output = format_refresh_report(&report);

        assert_eq!(output.exit_code, 2);
        assert!(output.stdout.contains("Flux actifs : 2"));
        assert!(output.stdout.contains("Articles reçus : 5"));
        assert!(output.stdout.contains("Nouveaux articles : 2"));
        assert!(output.stdout.contains("Articles mis à jour : 3"));
        assert!(output.stderr.contains("network unavailable"));
    }

    /// Verifies a fully successful refresh keeps exit code zero.
    #[test]
    fn format_successful_refresh_report_uses_zero_exit_code() {
        let output = format_refresh_report(&RefreshReport {
            active_feeds: 1,
            collected_articles: 2,
            inserted_articles: 2,
            updated_articles: 0,
            errors: Vec::new(),
        });

        assert_eq!(output.exit_code, 0);
        assert!(output.stderr.is_empty());
    }

    /// Verifies list works without configuration or network access.
    #[tokio::test]
    async fn list_is_fully_offline() {
        let (_directory, database_path) = populated_database().await;

        let output = run(offline_cli(database_path, Command::List))
            .await
            .unwrap();

        assert_eq!(output.exit_code, 0);
        assert!(output.stdout.contains("Repérer Jupiter"));
        assert!(output.stdout.contains("astronomy::jupiter"));
        assert!(output.stderr.is_empty());
    }

    /// Verifies show accepts a list number, renders HTML, and marks the article read.
    #[tokio::test]
    async fn show_by_number_is_offline_and_marks_article_read() {
        let (_directory, database_path) = populated_database().await;

        let output = run(offline_cli(
            database_path.clone(),
            Command::Show {
                selector: "1".to_string(),
            },
        ))
        .await
        .unwrap();

        assert!(output.stdout.contains("Jupiter est"));
        assert!(output.stdout.contains("très brillante"));
        assert!(!output.stdout.contains("<strong>"));
        let storage = Storage::open(&database_path).await.unwrap();
        assert!(
            storage
                .get_article("astronomy::jupiter")
                .await
                .unwrap()
                .unwrap()
                .is_read
        );
    }

    /// Verifies state commands accept stable IDs and one-based positions offline.
    #[tokio::test]
    async fn state_commands_update_persisted_flags_offline() {
        let (_directory, database_path) = populated_database().await;

        run(offline_cli(
            database_path.clone(),
            Command::Favorite {
                selector: "astronomy::jupiter".to_string(),
            },
        ))
        .await
        .unwrap();
        run(offline_cli(
            database_path.clone(),
            Command::MarkRead {
                selector: "1".to_string(),
            },
        ))
        .await
        .unwrap();

        let storage = Storage::open(&database_path).await.unwrap();
        let stored = storage
            .get_article("astronomy::jupiter")
            .await
            .unwrap()
            .unwrap();
        assert!(stored.is_read);
        assert!(stored.is_favorite);
        storage.close().await;

        run(offline_cli(
            database_path.clone(),
            Command::Unfavorite {
                selector: "1".to_string(),
            },
        ))
        .await
        .unwrap();
        run(offline_cli(
            database_path.clone(),
            Command::MarkUnread {
                selector: "astronomy::jupiter".to_string(),
            },
        ))
        .await
        .unwrap();

        let storage = Storage::open(&database_path).await.unwrap();
        let stored = storage
            .get_article("astronomy::jupiter")
            .await
            .unwrap()
            .unwrap();
        assert!(!stored.is_read);
        assert!(!stored.is_favorite);
    }

    /// Verifies an unknown selector produces a categorized fatal error.
    #[tokio::test]
    async fn unknown_article_returns_clear_error() {
        let (_directory, database_path) = populated_database().await;

        let error = run(offline_cli(
            database_path,
            Command::Show {
                selector: "99".to_string(),
            },
        ))
        .await
        .unwrap_err();

        assert_eq!(error, CliError::ArticleNotFound("99".to_string()));
        assert_eq!(error.to_string(), "Article introuvable : 99");
    }

    /// Verifies refresh reports a missing TOML file as a configuration error.
    #[tokio::test]
    async fn refresh_categorizes_configuration_errors_before_network_access() {
        let directory = tempfile::tempdir().unwrap();
        let cli = Cli {
            config: Some(directory.path().join("missing.toml")),
            database: Some(directory.path().join("inkriver.db")),
            command: Command::Refresh,
        };

        let error = run(cli).await.unwrap_err();

        assert!(matches!(error, CliError::Configuration(_)));
        assert!(error.to_string().starts_with("Erreur de configuration :"));
    }
}
