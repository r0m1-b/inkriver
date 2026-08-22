use crate::article::{ContentKind, Source};
use sha2::{Digest, Sha256};

/// Marks an identifier synthesized by the parser for an entry without GUID.
pub(crate) const MISSING_ENTRY_ID: &str = "__reader_missing_entry_id__";

fn canonicalize_entry_url(raw_url: &str) -> String {
    let trimmed_url = raw_url.trim();
    match reqwest::Url::parse(trimmed_url) {
        Ok(mut url) => {
            url.set_fragment(None);
            url.to_string()
        }
        Err(_) => trimmed_url.to_string(),
    }
}

fn fingerprint_entry(entry: &feed_rs::model::Entry) -> String {
    let mut hasher = Sha256::new();
    let mut has_stable_field = false;

    if let Some(published_at) = entry.published.or(entry.updated) {
        hasher.update(b"date\0");
        hasher.update(published_at.to_rfc3339().as_bytes());
        has_stable_field = true;
    }

    for author in &entry.authors {
        hasher.update(b"author\0");
        hasher.update(author.name.as_bytes());
        has_stable_field = true;
    }

    let body = entry
        .content
        .as_ref()
        .and_then(|content| content.body.as_deref())
        .or_else(|| {
            entry
                .summary
                .as_ref()
                .map(|summary| summary.content.as_str())
        });
    if let Some(body) = body {
        hasher.update(b"body\0");
        hasher.update(body.as_bytes());
        has_stable_field = true;
    }

    if !has_stable_field {
        hasher.update(b"title\0");
        if let Some(title) = &entry.title {
            hasher.update(title.content.as_bytes());
        }
    }

    format!("{:x}", hasher.finalize())
}

fn article_identity(entry: &feed_rs::model::Entry) -> String {
    if !entry.id.trim().is_empty() && entry.id != MISSING_ENTRY_ID {
        return entry.id.clone();
    }

    entry
        .links
        .first()
        .map(|link| format!("url::{}", canonicalize_entry_url(&link.href)))
        .unwrap_or_else(|| format!("fingerprint::{}", fingerprint_entry(entry)))
}

pub struct Feed {
    pub id: String,
    pub title: String,
    pub link: String,
    pub declared_icon_url: Option<String>,
    pub description: String,
    pub author: Option<String>,
    pub source: Source,
    pub entries: Vec<feed_rs::model::Entry>,
}

/// Contains the feed-level metadata retained after a successful collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedMetadata {
    pub id: String,
    pub title: String,
    pub description: String,
    pub author: Option<String>,
    pub site_url: String,
    pub declared_icon_url: Option<String>,
}

impl Feed {
    /// Creates a feed from its metadata, source platform, and parsed entries.
    pub fn new(
        id: String,
        title: String,
        link: String,
        description: String,
        author: Option<String>,
        source: Source,
        entries: Vec<feed_rs::model::Entry>,
    ) -> Self {
        Self {
            id,
            title,
            link,
            declared_icon_url: None,
            description,
            author,
            source,
            entries,
        }
    }

    /// Attaches the optional icon URL declared by RSS, Atom, or JSON Feed.
    pub fn with_declared_icon_url(mut self, declared_icon_url: Option<String>) -> Self {
        self.declared_icon_url = declared_icon_url;
        self
    }

    /// Copies the feed-level fields that must remain available after refresh.
    pub fn metadata(&self) -> FeedMetadata {
        FeedMetadata {
            id: self.id.clone(),
            title: self.title.clone(),
            description: self.description.clone(),
            author: self.author.clone(),
            site_url: self.link.clone(),
            declared_icon_url: self.declared_icon_url.clone(),
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
    /// The feed entry ID is namespaced by the configured feed ID. A publisher
    /// GUID is preferred, followed by the canonical entry URL and a stable
    /// content fingerprint. Missing optional display fields remain `None`; a
    /// summary is used only when the full content body is unavailable.
    pub fn article_from_entry(&self, entry: &feed_rs::model::Entry) -> crate::article::Article {
        let entry_identity = article_identity(entry);
        let (content, content_kind) = match entry
            .content
            .as_ref()
            .and_then(|content| content.body.as_deref())
            .filter(|body| !body.trim().is_empty())
        {
            Some(body) => (Some(ammonia::clean(body)), ContentKind::Full),
            None => match entry
                .summary
                .as_ref()
                .map(|summary| summary.content.as_str())
                .filter(|summary| !summary.trim().is_empty())
            {
                Some(summary) => (Some(ammonia::clean(summary)), ContentKind::Excerpt),
                None => (None, ContentKind::Missing),
            },
        };

        crate::article::Article {
            id: format!("{}::{entry_identity}", self.id),
            feed_id: self.id.clone(),
            title: entry.title.as_ref().map(|title| title.content.clone()),
            author: entry.authors.first().map(|a| a.name.clone()),
            published_at: entry.published,
            url: entry.links.first().map(|link| link.href.clone()),
            content,
            content_kind,
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
            Some("Test Author".into()),
            Source::Other,
            Vec::new(),
        );

        assert_eq!(feed.id, "test-feed");
        assert_eq!(feed.title, "Test Feed");
        assert_eq!(feed.link, "https://example.com");
        assert_eq!(feed.description, "A simple test feed");
        assert_eq!(feed.author.as_deref(), Some("Test Author"));
        assert_eq!(feed.metadata().title, "Test Feed");
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
            feed_id: feeds[0].id.clone(),
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
            content_kind: ContentKind::Full,
            source: crate::article::Source::Substack,
        };
        assert_eq!(article.id, format!("{}::{}", feeds[0].id, jupiter.id));
        assert_eq!(article.feed_id, feeds[0].id);
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
        assert_eq!(article.content_kind, ContentKind::Full);
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
            None,
            Source::Substack,
            vec![duplicate_entry.clone()],
        );
        let second_feed = Feed::new(
            "second".to_string(),
            "Second feed".to_string(),
            "https://second.example".to_string(),
            String::new(),
            None,
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

    /// Verifies a real publisher GUID takes precedence over the entry URL.
    #[test]
    fn article_id_prefers_publisher_guid() {
        let feed = &mock_feeds()[0];
        let entry = &feed.entries[0];

        let article = feed.article_from_entry(entry);

        assert_eq!(article.id, "carnet-du-ciel::substack-astronomie-1");
    }

    /// Verifies URL fallback ignores fragments and remains stable across title changes.
    #[test]
    fn article_url_identity_is_canonical_and_title_independent() {
        let feed = &mock_feeds()[0];
        let mut first_entry = feed.entries[0].clone();
        first_entry.id = MISSING_ENTRY_ID.to_string();
        first_entry.links[0].href =
            "https://carnet-du-ciel.example/p/reperer-jupiter#comments".to_string();
        let mut renamed_entry = first_entry.clone();
        renamed_entry.title.as_mut().unwrap().content = "Un nouveau titre".to_string();

        let first_article = feed.article_from_entry(&first_entry);
        let renamed_article = feed.article_from_entry(&renamed_entry);

        assert_eq!(first_article.id, renamed_article.id);
        assert_eq!(
            first_article.id,
            "carnet-du-ciel::url::https://carnet-du-ciel.example/p/reperer-jupiter"
        );
    }

    /// Verifies the last-resort fingerprint does not depend on the article title.
    #[test]
    fn article_fingerprint_identity_is_title_independent() {
        let feed = &mock_feeds()[0];
        let mut first_entry = feed.entries[0].clone();
        first_entry.id = MISSING_ENTRY_ID.to_string();
        first_entry.links.clear();
        let mut renamed_entry = first_entry.clone();
        renamed_entry.title.as_mut().unwrap().content = "Un nouveau titre".to_string();

        let first_article = feed.article_from_entry(&first_entry);
        let renamed_article = feed.article_from_entry(&renamed_entry);

        assert_eq!(first_article.id, renamed_article.id);
        assert!(
            first_article
                .id
                .starts_with("carnet-du-ciel::fingerprint::")
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
        assert_eq!(article.content_kind, ContentKind::Excerpt);
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
        assert_eq!(article.content_kind, ContentKind::Excerpt);
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
        assert_eq!(article.content_kind, ContentKind::Missing);
    }
}
