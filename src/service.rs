use crate::article::{Article, Source};
use crate::config::{Config, FeedConfig, Platform};
use crate::feed::Feed;
use crate::http;
use feed_rs::parser;
use std::fmt;

/// Identifies the operation that failed while loading a feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedLoadStage {
    HttpRequest,
    ResponseBody,
    FeedParsing,
    FeedMetadata,
}

impl fmt::Display for FeedLoadStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::HttpRequest => "HTTP request",
            Self::ResponseBody => "response body",
            Self::FeedParsing => "feed parsing",
            Self::FeedMetadata => "feed metadata",
        };

        formatter.write_str(label)
    }
}

/// Describes one failed operation while loading a feed.
#[derive(Debug, PartialEq, Eq)]
pub struct FeedLoadError {
    pub stage: FeedLoadStage,
    pub message: String,
}

impl fmt::Display for FeedLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.stage, self.message)
    }
}

/// Describes a configured feed that could not be collected.
#[derive(Debug, PartialEq, Eq)]
pub struct FeedCollectionError {
    pub feed_id: String,
    pub feed_url: String,
    pub error: FeedLoadError,
}

impl fmt::Display for FeedCollectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Feed {:?} ({}): {}",
            self.feed_id, self.feed_url, self.error
        )
    }
}

/// Contains all successfully collected articles and per-feed failures.
pub struct CollectionReport {
    pub articles: Vec<Article>,
    pub errors: Vec<FeedCollectionError>,
}

/// Builds the application feed model from data parsed by `feed-rs`.
///
/// # Errors
///
/// Returns an error when the parsed feed has no title or no primary link.
fn build_feed_from_data(
    raw_feed: feed_rs::model::Feed,
    feed_id: &str,
    platform: Platform,
) -> Result<Feed, String> {
    let title = raw_feed
        .title
        .map(|title| title.content)
        .ok_or_else(|| "This RSS feed has no title".to_string())?;

    let link = raw_feed
        .links
        .first()
        .map(|link| link.href.clone())
        .ok_or_else(|| "This RSS feed has no link".to_string())?;

    let description = raw_feed
        .description
        .map(|description| description.content)
        .unwrap_or_default();

    let source = match platform {
        Platform::Medium => Source::Medium,
        Platform::Substack => Source::Substack,
        Platform::Other => Source::Other,
    };

    Ok(Feed::new(
        feed_id.to_string(),
        title,
        link,
        description,
        source,
        raw_feed.entries,
    ))
}

/// Loads and parses one configured feed with an injected content loader.
///
/// Injecting at the response-content boundary exercises parsing and metadata
/// validation in tests without performing network requests.
fn load_feed_with_content_loader<F>(
    feed_config: &FeedConfig,
    load_content: F,
) -> Result<Feed, FeedLoadError>
where
    F: FnOnce(&FeedConfig) -> Result<String, FeedLoadError>,
{
    let content = load_content(feed_config)?;
    let raw_feed = parser::parse(content.as_bytes()).map_err(|error| FeedLoadError {
        stage: FeedLoadStage::FeedParsing,
        message: error.to_string(),
    })?;

    build_feed_from_data(raw_feed, &feed_config.id, feed_config.platform).map_err(|message| {
        FeedLoadError {
            stage: FeedLoadStage::FeedMetadata,
            message,
        }
    })
}

/// Downloads and parses one configured feed.
///
/// # Errors
///
/// Returns an error when the request, response reading, RSS parsing, or feed
/// model conversion fails.
fn load_feed_from_http(feed_config: &FeedConfig) -> Result<Feed, FeedLoadError> {
    load_feed_with_content_loader(feed_config, |feed_config| {
        let response = http::check_feed_url(&feed_config.url).map_err(|message| FeedLoadError {
            stage: FeedLoadStage::HttpRequest,
            message,
        })?;

        response.text().map_err(|error| FeedLoadError {
            stage: FeedLoadStage::ResponseBody,
            message: error.to_string(),
        })
    })
}

/// Sorts articles from newest to oldest, placing undated articles last.
fn sort_articles_newest_first(articles: &mut [Article]) {
    articles.sort_by_key(|article| std::cmp::Reverse(article.published_at));
}

/// Collects articles with an injected feed loader.
///
/// The loader parameter keeps aggregation independent from HTTP and makes the
/// complete workflow deterministic in unit tests.
///
fn collect_articles_with_loader<F>(config: &Config, mut load_feed: F) -> CollectionReport
where
    F: FnMut(&FeedConfig) -> Result<Feed, FeedLoadError>,
{
    let mut articles = Vec::new();
    let mut errors = Vec::new();

    for feed_config in &config.feeds {
        match load_feed(feed_config) {
            Ok(feed) => articles.extend(feed.get_articles()),
            Err(error) => errors.push(FeedCollectionError {
                feed_id: feed_config.id.clone(),
                feed_url: feed_config.url.clone(),
                error,
            }),
        }
    }

    sort_articles_newest_first(&mut articles);
    CollectionReport { articles, errors }
}

/// Downloads all configured feeds and reports successes and failures separately.
pub fn collect_articles(config: &Config) -> CollectionReport {
    collect_articles_with_loader(config, load_feed_from_http)
}

/// Formats the compact article representation used by the current CLI.
pub fn format_article_summary(article: &Article) -> String {
    let published_at = article
        .published_at
        .map(|date| date.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "unknown date".to_string());

    format!(
        "{published_at} | {:?} | {} | {}",
        article.source, article.title, article.url
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::mock_feeds;

    const SIMPLE_RSS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
        <rss version="2.0" xmlns:content="http://purl.org/rss/1.0/modules/content/">
          <channel>
            <title>Test astronomy feed</title>
            <link>https://astronomy.example</link>
            <description>A test feed about the night sky.</description>
            <item>
              <guid>astronomy-1</guid>
              <title>Finding Mars</title>
              <link>https://astronomy.example/finding-mars</link>
              <pubDate>Wed, 29 Jul 2026 20:00:00 +0000</pubDate>
              <content:encoded><![CDATA[<p>Mars has a recognizable orange hue.</p>]]></content:encoded>
            </item>
          </channel>
        </rss>"#;

    const RSS_WITHOUT_TITLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
        <rss version="2.0">
          <channel>
            <link>https://astronomy.example</link>
            <description>A feed without a title.</description>
          </channel>
        </rss>"#;

    const RSS_WITHOUT_LINK: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
        <rss version="2.0">
          <channel>
            <title>Test astronomy feed</title>
            <description>A feed without a link.</description>
          </channel>
        </rss>"#;

    fn test_config() -> Config {
        Config {
            feeds: vec![
                FeedConfig {
                    id: "astronomy".to_string(),
                    platform: Platform::Substack,
                    url: "https://astronomy.example/feed".to_string(),
                },
                FeedConfig {
                    id: "bread".to_string(),
                    platform: Platform::Medium,
                    url: "https://medium.com/feed/@bread".to_string(),
                },
            ],
        }
    }

    /// Verifies conversion from a parsed feed into the common feed model.
    #[test]
    fn build_feed_from_parsed_data() {
        let raw_feed = parser::parse(SIMPLE_RSS.as_bytes()).unwrap();

        let feed = build_feed_from_data(raw_feed, "astronomy", Platform::Substack).unwrap();

        assert_eq!(feed.id, "astronomy");
        assert_eq!(feed.title, "Test astronomy feed");
        assert_eq!(feed.link, "https://astronomy.example/");
        assert_eq!(feed.description, "A test feed about the night sky.");
        assert_eq!(feed.source, Source::Substack);
        assert_eq!(feed.entries.len(), 1);
    }

    /// Verifies the complete loading pipeline from injected RSS content.
    #[test]
    fn load_feed_from_valid_injected_content() {
        let feed_config = &test_config().feeds[0];

        let feed =
            load_feed_with_content_loader(feed_config, |_feed_config| Ok(SIMPLE_RSS.to_string()))
                .unwrap();

        assert_eq!(feed.id, "astronomy");
        assert_eq!(feed.title, "Test astronomy feed");
        assert_eq!(feed.source, Source::Substack);
        assert_eq!(feed.entries.len(), 1);
    }

    /// Verifies that malformed injected XML is reported as a parsing failure.
    #[test]
    fn load_feed_rejects_invalid_injected_xml() {
        let feed_config = &test_config().feeds[0];

        let error = match load_feed_with_content_loader(feed_config, |_feed_config| {
            Ok("<rss><channel>".to_string())
        }) {
            Ok(_) => panic!("invalid XML should be rejected"),
            Err(error) => error,
        };

        assert_eq!(error.stage, FeedLoadStage::FeedParsing);
    }

    /// Verifies that a missing feed title is reported as invalid metadata.
    #[test]
    fn load_feed_rejects_injected_content_without_title() {
        let feed_config = &test_config().feeds[0];

        let error = match load_feed_with_content_loader(feed_config, |_feed_config| {
            Ok(RSS_WITHOUT_TITLE.to_string())
        }) {
            Ok(_) => panic!("a feed without a title should be rejected"),
            Err(error) => error,
        };

        assert_eq!(error.stage, FeedLoadStage::FeedMetadata);
        assert_eq!(error.message, "This RSS feed has no title");
    }

    /// Verifies that a missing primary link is reported as invalid metadata.
    #[test]
    fn load_feed_rejects_injected_content_without_link() {
        let feed_config = &test_config().feeds[0];

        let error = match load_feed_with_content_loader(feed_config, |_feed_config| {
            Ok(RSS_WITHOUT_LINK.to_string())
        }) {
            Ok(_) => panic!("a feed without a link should be rejected"),
            Err(error) => error,
        };

        assert_eq!(error.stage, FeedLoadStage::FeedMetadata);
        assert_eq!(error.message, "This RSS feed has no link");
    }

    /// Verifies that the HTTP loader rejects an invalid URL without network IO.
    #[test]
    fn load_feed_from_http_rejects_invalid_url() {
        let feed_config = FeedConfig {
            id: "invalid".to_string(),
            platform: Platform::Other,
            url: "not a valid URL".to_string(),
        };

        let result = load_feed_from_http(&feed_config);

        let error = match result {
            Ok(_) => panic!("an invalid URL should be rejected"),
            Err(error) => error,
        };
        assert_eq!(error.stage, FeedLoadStage::HttpRequest);
        assert!(error.to_string().starts_with("HTTP request:"));
    }

    /// Verifies chronological sorting and placement of the oldest article last.
    #[test]
    fn sort_articles_from_newest_to_oldest() {
        let mut articles: Vec<Article> = mock_feeds().iter().flat_map(Feed::get_articles).collect();

        sort_articles_newest_first(&mut articles);

        assert_eq!(
            articles.first().unwrap().id,
            "le-pain-patient::medium-pain-5"
        );
        assert_eq!(
            articles.last().unwrap().id,
            "carnet-du-ciel::substack-astronomie-1"
        );
        assert!(
            articles
                .windows(2)
                .all(|pair| pair[0].published_at >= pair[1].published_at)
        );
    }

    /// Verifies that missing publication dates are always placed after dated articles.
    #[test]
    fn sort_articles_places_missing_dates_last() {
        let mut articles = mock_feeds()[0].get_articles();
        articles[0].published_at = None;
        articles[3].published_at = None;

        sort_articles_newest_first(&mut articles);

        assert!(
            articles[..3]
                .iter()
                .all(|article| article.published_at.is_some())
        );
        assert!(
            articles[3..]
                .iter()
                .all(|article| article.published_at.is_none())
        );
    }

    /// Verifies aggregation of two injected feeds without accessing the network.
    #[test]
    fn collect_articles_from_two_loaded_feeds() {
        let config = test_config();
        let mut feeds = mock_feeds().into_iter();

        let report = collect_articles_with_loader(&config, |_feed_config| {
            feeds.next().ok_or_else(|| FeedLoadError {
                stage: FeedLoadStage::FeedMetadata,
                message: "Missing mock feed".to_string(),
            })
        });

        assert!(report.errors.is_empty());
        assert_eq!(report.articles.len(), 10);
        assert_eq!(
            report
                .articles
                .iter()
                .filter(|article| article.source == Source::Substack)
                .count(),
            5
        );
        assert_eq!(
            report
                .articles
                .iter()
                .filter(|article| article.source == Source::Medium)
                .count(),
            5
        );
        assert_eq!(
            report.articles.first().unwrap().id,
            "le-pain-patient::medium-pain-5"
        );
    }

    /// Verifies that one failed feed does not discard articles from later feeds.
    #[test]
    fn collect_articles_continues_after_feed_failure() {
        let config = test_config();
        let mut successful_feed = mock_feeds().pop();
        let mut loader_calls = 0;

        let report = collect_articles_with_loader(&config, |feed_config| {
            loader_calls += 1;

            if feed_config.id == "astronomy" {
                Err(FeedLoadError {
                    stage: FeedLoadStage::HttpRequest,
                    message: "Astronomy feed unavailable".to_string(),
                })
            } else {
                successful_feed.take().ok_or_else(|| FeedLoadError {
                    stage: FeedLoadStage::FeedMetadata,
                    message: "Missing successful mock feed".to_string(),
                })
            }
        });

        assert_eq!(loader_calls, 2);
        assert_eq!(report.articles.len(), 5);
        assert!(
            report
                .articles
                .iter()
                .all(|article| article.source == Source::Medium)
        );
        assert_eq!(
            report.errors,
            vec![FeedCollectionError {
                feed_id: "astronomy".to_string(),
                feed_url: "https://astronomy.example/feed".to_string(),
                error: FeedLoadError {
                    stage: FeedLoadStage::HttpRequest,
                    message: "Astronomy feed unavailable".to_string(),
                },
            }]
        );
    }

    /// Verifies that the public collector handles an empty configuration.
    #[test]
    fn collect_articles_from_empty_config() {
        let config = Config { feeds: Vec::new() };

        let report = collect_articles(&config);

        assert!(report.articles.is_empty());
        assert!(report.errors.is_empty());
    }

    /// Verifies that a feed collection error includes actionable context.
    #[test]
    fn format_feed_collection_error() {
        let error = FeedCollectionError {
            feed_id: "astronomy".to_string(),
            feed_url: "https://astronomy.example/feed".to_string(),
            error: FeedLoadError {
                stage: FeedLoadStage::HttpRequest,
                message: "Connection refused".to_string(),
            },
        };

        assert_eq!(
            error.to_string(),
            "Feed \"astronomy\" (https://astronomy.example/feed): HTTP request: Connection refused"
        );
    }

    /// Verifies the stable human-readable labels of every loading stage.
    #[test]
    fn format_feed_load_stages() {
        let cases = [
            (FeedLoadStage::HttpRequest, "HTTP request"),
            (FeedLoadStage::ResponseBody, "response body"),
            (FeedLoadStage::FeedParsing, "feed parsing"),
            (FeedLoadStage::FeedMetadata, "feed metadata"),
        ];

        for (stage, expected) in cases {
            assert_eq!(stage.to_string(), expected);
        }
    }

    /// Verifies that a loading error combines its stage and underlying message.
    #[test]
    fn format_feed_load_error() {
        let error = FeedLoadError {
            stage: FeedLoadStage::FeedParsing,
            message: "invalid XML".to_string(),
        };

        assert_eq!(error.to_string(), "feed parsing: invalid XML");
    }

    /// Verifies the CLI representation of an article.
    #[test]
    fn format_article_as_cli_summary() {
        let article = mock_feeds()[0].get_articles().remove(0);

        let summary = format_article_summary(&article);

        assert_eq!(
            summary,
            "2026-07-06 | Substack | Repérer Jupiter sans télescope | https://carnet-du-ciel.example/p/reperer-jupiter"
        );
    }
}
