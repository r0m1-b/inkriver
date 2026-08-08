use chrono::{DateTime, Utc};

#[derive(PartialEq, Debug, Clone, Copy)]
pub enum Source {
    Medium,
    Substack,
    Other,
}
pub struct Article {
    pub id: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub url: Option<String>,
    pub content: Option<String>,
    pub source: Source,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that an article retains every value supplied during construction.
    #[test]
    fn test_article_creation() {
        let article = Article {
            id: "1".to_string(),
            title: Some("Test Article".to_string()),
            author: None,
            published_at: None,
            url: Some("https://example.com/article".to_string()),
            content: Some("Test content".to_string()),
            source: Source::Other,
        };
        assert_eq!(article.id, "1");
        assert_eq!(article.title.as_deref(), Some("Test Article"));
        assert_eq!(article.author, None);
        assert_eq!(article.published_at, None);
        assert_eq!(article.url.as_deref(), Some("https://example.com/article"));
        assert_eq!(article.source, Source::Other);
        assert_eq!(article.content.as_deref(), Some("Test content"));
    }
}
