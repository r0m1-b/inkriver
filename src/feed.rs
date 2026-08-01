use crate::article::Source;

pub struct Feed {
    pub title: String,
    pub link: String,
    pub description: String,
    pub source: Source,
    pub entries: Vec<feed_rs::model::Entry>,
}

impl Feed {
    /// Creates a feed from its metadata, source platform, and parsed entries.
    pub fn new(
        title: String,
        link: String,
        description: String,
        source: Source,
        entries: Vec<feed_rs::model::Entry>,
    ) -> Self {
        Self {
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
    /// Missing optional feed fields are replaced with explicit fallback values.
    pub fn article_from_entry(&self, entry: &feed_rs::model::Entry) -> crate::article::Article {
        crate::article::Article {
            id: entry.id.clone(),
            title: entry
                .title
                .as_ref()
                .map_or("No title".to_string(), |t| t.content.clone()),
            author: entry.authors.first().map(|a| a.name.clone()),
            published_at: entry.published,
            url: entry
                .links
                .first()
                .map_or_else(|| "".into(), |l| l.href.clone()),
            content: entry
                .content
                .as_ref()
                .map_or("No content".to_string(), |c| {
                    c.body.as_ref().map_or("No body".to_string(), |b| b.clone())
                }),
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
            "Test Feed".into(),
            "https://example.com".into(),
            "A simple test feed".into(),
            Source::Other,
            Vec::new(),
        );

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
            id: jupiter.id.clone(),
            title: jupiter
                .title
                .as_ref()
                .map_or("No title".to_string(), |t| t.content.clone()),
            author: jupiter.authors.first().map(|a| a.name.clone()),
            published_at: jupiter.published.clone(),
            url: jupiter
                .links
                .first()
                .map_or_else(|| "".into(), |l| l.href.clone()),
            content: jupiter
                .content
                .as_ref()
                .map_or("No content".to_string(), |c| {
                    c.body.as_ref().map_or("No body".to_string(), |b| b.clone())
                }),
            source: crate::article::Source::Substack,
        };
        assert_eq!(article.id, jupiter.id.clone());
        assert_eq!(
            article.title,
            jupiter
                .title
                .as_ref()
                .map_or("No title".to_string(), |t| t.content.clone())
        );
        assert_eq!(
            article.author,
            jupiter.authors.first().map(|a| a.name.clone())
        );
        assert_eq!(article.published_at, jupiter.published.clone());
        assert_eq!(
            article.url,
            jupiter
                .links
                .first()
                .map_or_else(|| "".into(), |l| l.href.clone())
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
        assert_eq!(article.id, "substack-astronomie-1");
        assert_eq!(article.title, "Repérer Jupiter sans télescope");
        assert_eq!(article.author, None);
        assert_eq!(article.published_at, entry.published.clone());
        assert_eq!(
            article.url,
            "https://carnet-du-ciel.example/p/reperer-jupiter"
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
            assert_eq!(article.id, entry.id);
            assert_eq!(
                article.title,
                entry
                    .title
                    .as_ref()
                    .map_or("No title".to_string(), |t| t.content.clone())
            );
            assert_eq!(
                article.author,
                entry.authors.first().map(|a| a.name.clone())
            );
            assert_eq!(article.published_at, entry.published.clone());
            assert_eq!(
                article.url,
                entry
                    .links
                    .first()
                    .map_or_else(|| "".into(), |l| l.href.clone())
            );
            assert_eq!(article.source, feed.source);
        }
    }
}
