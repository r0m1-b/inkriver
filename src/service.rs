use crate::article::{Article, Source};
use crate::config::{Config, FeedConfig, Platform};
use crate::feed::Feed;
use crate::http;
use feed_rs::parser;
use std::fmt;

/// Describes a configured feed that could not be collected.
#[derive(Debug, PartialEq, Eq)]
pub struct FeedCollectionError {
    pub feed_id: String,
    pub feed_url: String,
    pub message: String,
}

impl fmt::Display for FeedCollectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Feed {:?} ({}): {}",
            self.feed_id, self.feed_url, self.message
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
        title,
        link,
        description,
        source,
        raw_feed.entries,
    ))
}

/// Downloads and parses one configured feed.
///
/// # Errors
///
/// Returns an error when the request, response reading, RSS parsing, or feed
/// model conversion fails.
fn load_feed_from_http(feed_config: &FeedConfig) -> Result<Feed, String> {
    let response = http::check_feed_url(&feed_config.url)?;
    let content = response.text().map_err(|error| error.to_string())?;
    let raw_feed = parser::parse(content.as_bytes()).map_err(|error| error.to_string())?;

    build_feed_from_data(raw_feed, feed_config.platform)
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
    F: FnMut(&FeedConfig) -> Result<Feed, String>,
{
    let mut articles = Vec::new();
    let mut errors = Vec::new();

    for feed_config in &config.feeds {
        match load_feed(feed_config) {
            Ok(feed) => articles.extend(feed.get_articles()),
            Err(message) => errors.push(FeedCollectionError {
                feed_id: feed_config.id.clone(),
                feed_url: feed_config.url.clone(),
                message,
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

        let feed = build_feed_from_data(raw_feed, Platform::Substack).unwrap();

        assert_eq!(feed.title, "Test astronomy feed");
        assert_eq!(feed.link, "https://astronomy.example/");
        assert_eq!(feed.description, "A test feed about the night sky.");
        assert_eq!(feed.source, Source::Substack);
        assert_eq!(feed.entries.len(), 1);
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

        assert!(result.is_err());
    }

    /// Verifies chronological sorting and placement of the oldest article last.
    #[test]
    fn sort_articles_from_newest_to_oldest() {
        let mut articles: Vec<Article> = mock_feeds().iter().flat_map(Feed::get_articles).collect();

        sort_articles_newest_first(&mut articles);

        assert_eq!(articles.first().unwrap().id, "medium-pain-5");
        assert_eq!(articles.last().unwrap().id, "substack-astronomie-1");
        assert!(
            articles
                .windows(2)
                .all(|pair| pair[0].published_at >= pair[1].published_at)
        );
    }

    /// Verifies aggregation of two injected feeds without accessing the network.
    #[test]
    fn collect_articles_from_two_loaded_feeds() {
        let config = test_config();
        let mut feeds = mock_feeds().into_iter();

        let report = collect_articles_with_loader(&config, |_feed_config| {
            feeds.next().ok_or_else(|| "Missing mock feed".to_string())
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
        assert_eq!(report.articles.first().unwrap().id, "medium-pain-5");
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
                Err("Astronomy feed unavailable".to_string())
            } else {
                successful_feed
                    .take()
                    .ok_or_else(|| "Missing successful mock feed".to_string())
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
                message: "Astronomy feed unavailable".to_string(),
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
            message: "Connection refused".to_string(),
        };

        assert_eq!(
            error.to_string(),
            "Feed \"astronomy\" (https://astronomy.example/feed): Connection refused"
        );
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
