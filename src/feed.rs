pub struct Feed {
    pub title: String,
    pub link: String,
    pub description: String,
    pub entries: Vec<feed_rs::model::Entry>,
}

impl Feed {
    pub fn new(
        title: String,
        link: String,
        description: String,
        entries: Vec<feed_rs::model::Entry>,
    ) -> Self {
        Self {
            title,
            link,
            description,
            entries,
        }
    }

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

    pub fn display_feed_info(&self) {
        println!("Title: {}", self.title);
        println!("Link: {}", self.link);
        println!("Description: {}", self.description);
        println!("Entries:");
    }

    pub fn display(&self) {
        self.display_feed_info();
        self.display_entries();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::mock_feeds;

    #[test]
    fn test_feed_creation() {
        let feed = Feed::new(
            "Test Feed".into(),
            "https://example.com".into(),
            "A simple test feed".into(),
            Vec::new(),
        );

        assert_eq!(feed.title, "Test Feed");
        assert_eq!(feed.link, "https://example.com");
        assert_eq!(feed.description, "A simple test feed");
    }

    #[test]
    fn mock_feeds_contain_two_sources_with_five_entries_each() {
        let feeds = mock_feeds();

        assert_eq!(feeds.len(), 2);
        assert_eq!(feeds[0].title, "Carnet du ciel — Substack");
        assert_eq!(feeds[0].entries.len(), 5);
        assert_eq!(feeds[1].title, "Le pain patient — Medium");
        assert_eq!(feeds[1].entries.len(), 5);
    }

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
}
