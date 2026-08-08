use crate::article::Source;

pub struct Feed {
    pub id: String,
    pub title: String,
    pub link: String,
    pub description: String,
    pub source: Source,
    pub entries: Vec<feed_rs::model::Entry>,
}

impl Feed {
    /// Creates a feed from its metadata, source platform, and parsed entries.
    pub fn new(
        id: String,
        title: String,
        link: String,
        description: String,
        source: Source,
        entries: Vec<feed_rs::model::Entry>,
    ) -> Self {
        Self {
            id,
            title,
            link,
            description,
            source,
            entries,
        }
    }

    /// Finds an entry by its feed-specific identifier.
    ///
    /// Returns `None` when the feed does not contain a matching entry.
    pub fn entry_from_id(&self, id: &str) -> Option<&feed_rs::model::Entry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    /// Converts a parsed feed entry into the application's common article model.
    ///
    /// The feed entry ID is namespaced by the configured feed ID, with the
    /// entry URL as a fallback. Missing optional display fields remain `None`;
    /// a summary is used only when the full content body is unavailable.
    pub fn article_from_entry(&self, entry: &feed_rs::model::Entry) -> crate::article::Article {
        let entry_identity = if entry.id.trim().is_empty() {
            entry
                .links
                .first()
                .map(|link| format!("url::{}", link.href))
                .unwrap_or_else(|| "unidentified".to_string())
        } else {
            entry.id.clone()
        };

        crate::article::Article {
            id: format!("{}::{entry_identity}", self.id),
            title: entry.title.as_ref().map(|title| title.content.clone()),
            author: entry.authors.first().map(|a| a.name.clone()),
            published_at: entry.published,
            url: entry.links.first().map(|link| link.href.clone()),
            content: entry
                .content
                .as_ref()
                .and_then(|content| content.body.clone())
                .or_else(|| {
                    entry
                        .summary
                        .as_ref()
                        .map(|summary| summary.content.clone())
                })
                .map(|content| ammonia::clean(&content)),
            source: self.source,
        }
    }

    /// Converts every parsed entry in this feed into an application article.
    pub fn get_articles(&self) -> Vec<crate::article::Article> {
        self.entries
            .iter()
            .map(|entry| self.article_from_entry(entry))
            .collect()
    }

    /// Prints the title, primary link, and publication date of every feed entry.
    pub fn display_entries(&self) {
        for entry in &self.entries {
            println!(
                "Title: {}",
                entry.title.as_ref().map_or("No title", |t| &t.content)
            );
            println!(
                "Link: {}",
                entry.links.first().map_or("No link", |l| &l.href)
            );
            println!(
                "Published: {}",
                entry
                    .published
                    .map_or("No published date".to_string(), |p| p.to_string())
            );
            println!("-----------------------------");
        }
    }

    /// Prints the feed metadata and an entries heading to standard output.
    pub fn display_feed_info(&self) {
        println!("Title: {}", self.title);
        println!("Link: {}", self.link);
        println!("Description: {}", self.description);
        println!("Entries:");
    }

    /// Prints the feed metadata followed by all of its entries.
    pub fn display(&self) {
        self.display_feed_info();
        self.display_entries();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::mock_feeds;

    /// Verifies that the feed constructor retains all supplied metadata.
    #[test]
    fn test_feed_creation() {
        let feed = Feed::new(
            "test-feed".into(),
            "Test Feed".into(),
            "https://example.com".into(),
            "A simple test feed".into(),
            Source::Other,
            Vec::new(),
        );

        assert_eq!(feed.id, "test-feed");
        assert_eq!(feed.title, "Test Feed");
        assert_eq!(feed.link, "https://example.com");
        assert_eq!(feed.description, "A simple test feed");
        assert_eq!(feed.source, Source::Other);
    }

    /// Verifies that the mock dataset contains two feeds with five entries each.
    #[test]
    fn mock_feeds_contain_two_sources_with_five_entries_each() {
        let feeds = mock_feeds();

        assert_eq!(feeds.len(), 2);
        assert_eq!(feeds[0].title, "Carnet du ciel — Substack");
        assert_eq!(feeds[0].entries.len(), 5);
        assert_eq!(feeds[1].title, "Le pain patient — Medium");
        assert_eq!(feeds[1].entries.len(), 5);
    }

    /// Verifies that a mock article title is consistent with its body content.
    #[test]
    fn mock_article_title_matches_its_content() {
        let feeds = mock_feeds();
        let jupiter = &feeds[0].entries[0];

        assert_eq!(
            jupiter.title.as_ref().unwrap().content,
            "Repérer Jupiter sans télescope"
        );
        assert!(
            jupiter
                .content
                .as_ref()
                .unwrap()
                .body
                .as_ref()
                .unwrap()
                .contains("Jupiter ressemble à une étoile très brillante")
        );
    }

    /// Verifies manual conversion of a mock feed entry into an article model.
    #[test]
    fn build_article_from_mock_feed() {
        let feeds = mock_feeds();
        let jupiter = &feeds[0].entries[0];

        let article = crate::article::Article {
            id: format!("{}::{}", feeds[0].id, jupiter.id),
            title: jupiter.title.as_ref().map(|title| title.content.clone()),
            author: jupiter.authors.first().map(|a| a.name.clone()),
            published_at: jupiter.published,
            url: jupiter.links.first().map(|link| link.href.clone()),
            content: jupiter
                .content
                .as_ref()
                .and_then(|content| content.body.clone())
                .or_else(|| {
                    jupiter
                        .summary
                        .as_ref()
                        .map(|summary| summary.content.clone())
                }),
            source: crate::article::Source::Substack,
        };
        assert_eq!(article.id, format!("{}::{}", feeds[0].id, jupiter.id));
        assert_eq!(
            article.title.as_deref(),
            jupiter.title.as_ref().map(|title| title.content.as_str())
        );
        assert_eq!(
            article.author,
            jupiter.authors.first().map(|a| a.name.clone())
        );
        assert_eq!(article.published_at, jupiter.published.clone());
        assert_eq!(
            article.url.as_deref(),
            jupiter.links.first().map(|link| link.href.as_str())
        );
        assert_eq!(article.source, crate::article::Source::Substack);
    }

    /// Verifies that an existing entry can be found by its identifier.
    #[test]
    fn test_entry_from_id() {
        let feeds = mock_feeds();
        let feed = &feeds[0];
        let entry = feed.entry_from_id(&feed.entries[0].id).unwrap();
        assert_eq!(entry.id, feed.entries[0].id);
    }

    /// Verifies that `article_from_entry` maps the expected entry fields.
    #[test]
    fn test_article_from_entry() {
        let feeds = mock_feeds();
        let feed = &feeds[0];
        let entry = feed.entry_from_id(&feed.entries[0].id).unwrap();

        let article = feed.article_from_entry(entry);
        assert_eq!(article.id, "carnet-du-ciel::substack-astronomie-1");
        assert_eq!(
            article.title.as_deref(),
            Some("Repérer Jupiter sans télescope")
        );
        assert_eq!(article.author, None);
        assert_eq!(article.published_at, entry.published.clone());
        assert_eq!(
            article.url.as_deref(),
            Some("https://carnet-du-ciel.example/p/reperer-jupiter")
        );
        assert_eq!(
            article.content.as_deref(),
            entry
                .content
                .as_ref()
                .and_then(|content| content.body.as_deref())
        );
        assert_eq!(article.source, crate::article::Source::Substack);
    }

    /// Verifies that all feed entries are converted into corresponding articles.
    #[test]
    fn test_get_articles() {
        let feeds = mock_feeds();
        let feed = &feeds[0];
        let articles = feed.get_articles();

        assert_eq!(articles.len(), feed.entries.len());
        for (article, entry) in articles.iter().zip(feed.entries.iter()) {
            assert_eq!(article.id, format!("{}::{}", feed.id, entry.id));
            assert_eq!(
                article.title.as_deref(),
                entry.title.as_ref().map(|title| title.content.as_str())
            );
            assert_eq!(
                article.author,
                entry.authors.first().map(|a| a.name.clone())
            );
            assert_eq!(article.published_at, entry.published.clone());
            assert_eq!(
                article.url.as_deref(),
                entry.links.first().map(|link| link.href.as_str())
            );
            assert_eq!(article.source, feed.source);
        }
    }

    /// Verifies that equal entry identifiers from different feeds stay unique.
    #[test]
    fn article_ids_are_unique_between_feeds() {
        let duplicate_entry = mock_feeds()[0].entries[0].clone();
        let first_feed = Feed::new(
            "first".to_string(),
            "First feed".to_string(),
            "https://first.example".to_string(),
            String::new(),
            Source::Substack,
            vec![duplicate_entry.clone()],
        );
        let second_feed = Feed::new(
            "second".to_string(),
            "Second feed".to_string(),
            "https://second.example".to_string(),
            String::new(),
            Source::Medium,
            vec![duplicate_entry],
        );

        let first_article = first_feed.get_articles().remove(0);
        let second_article = second_feed.get_articles().remove(0);

        assert_eq!(first_article.id, "first::substack-astronomie-1");
        assert_eq!(second_article.id, "second::substack-astronomie-1");
        assert_ne!(first_article.id, second_article.id);
    }

    /// Verifies that an entry URL provides a stable identity when its ID is absent.
    #[test]
    fn article_id_falls_back_to_entry_url() {
        let feed = &mock_feeds()[0];
        let mut entry = feed.entries[0].clone();
        entry.id.clear();

        let article = feed.article_from_entry(&entry);

        assert_eq!(
            article.id,
            "carnet-du-ciel::url::https://carnet-du-ciel.example/p/reperer-jupiter"
        );
    }

    /// Verifies that the entry summary is used when full content is unavailable.
    #[test]
    fn article_uses_summary_when_content_is_missing() {
        let feed = &mock_feeds()[0];
        let mut entry = feed.entries[0].clone();
        let expected_summary = entry.summary.as_ref().unwrap().content.clone();
        entry.content = None;

        let article = feed.article_from_entry(&entry);

        assert_eq!(article.content, Some(expected_summary));
    }

    /// Verifies that the entry summary replaces content with no inline body.
    #[test]
    fn article_uses_summary_when_content_body_is_missing() {
        let feed = &mock_feeds()[0];
        let mut entry = feed.entries[0].clone();
        let expected_summary = entry.summary.as_ref().unwrap().content.clone();
        entry.content.as_mut().unwrap().body = None;

        let article = feed.article_from_entry(&entry);

        assert_eq!(article.content, Some(expected_summary));
    }

    /// Verifies that unsafe HTML is removed from a full article body.
    #[test]
    fn article_sanitizes_full_html_content() {
        let feed = &mock_feeds()[0];
        let mut entry = feed.entries[0].clone();
        entry.content.as_mut().unwrap().body = Some(
            r#"<p onclick="alert('event')">Safe <strong>text</strong><script>alert('script')</script><a href="javascript:alert('url')">bad link</a><a href="https://example.com/read">read</a></p>"#
                .to_string(),
        );

        let article = feed.article_from_entry(&entry);
        let content = article.content.unwrap();

        assert!(!content.contains("<script"));
        assert!(!content.contains("onclick"));
        assert!(!content.contains("javascript:"));
        assert!(content.contains("<strong>text</strong>"));
        assert!(content.contains("href=\"https://example.com/read\""));
    }

    /// Verifies that fallback summaries receive the same HTML sanitization.
    #[test]
    fn article_sanitizes_summary_fallback() {
        let feed = &mock_feeds()[0];
        let mut entry = feed.entries[0].clone();
        entry.content = None;
        entry.summary.as_mut().unwrap().content =
            r#"<p>Safe summary<img src="invalid" onerror="alert('image')"><style>body { display: none; }</style></p>"#
                .to_string();

        let article = feed.article_from_entry(&entry);
        let content = article.content.unwrap();

        assert!(!content.contains("onerror"));
        assert!(!content.contains("<style"));
        assert!(content.contains("Safe summary"));
    }

    /// Verifies that absent display fields remain explicit in the article model.
    #[test]
    fn article_preserves_missing_display_fields() {
        let feed = &mock_feeds()[0];
        let mut entry = feed.entries[0].clone();
        entry.title = None;
        entry.links.clear();
        entry.content = None;
        entry.summary = None;

        let article = feed.article_from_entry(&entry);

        assert_eq!(article.title, None);
        assert_eq!(article.url, None);
        assert_eq!(article.content, None);
    }
}
