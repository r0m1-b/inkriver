use chrono::{DateTime, Utc};

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum Source {
    Medium,
    Substack,
    Other,
}

/// Describes the origin and completeness of the stored article content.
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum ContentKind {
    Full,
    Extracted,
    Excerpt,
    Missing,
    Unknown,
}

impl ContentKind {
    /// Returns the stable lowercase representation stored in SQLite.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Extracted => "extracted",
            Self::Excerpt => "excerpt",
            Self::Missing => "missing",
            Self::Unknown => "unknown",
        }
    }
}

impl TryFrom<&str> for ContentKind {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "full" => Ok(Self::Full),
            "extracted" => Ok(Self::Extracted),
            "excerpt" => Ok(Self::Excerpt),
            "missing" => Ok(Self::Missing),
            "unknown" => Ok(Self::Unknown),
            _ => Err(format!("Unknown article content kind: {value}")),
        }
    }
}

impl Source {
    /// Returns the stable lowercase representation stored in SQLite.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Medium => "medium",
            Self::Substack => "substack",
            Self::Other => "other",
        }
    }
}

impl TryFrom<&str> for Source {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "medium" => Ok(Self::Medium),
            "substack" => Ok(Self::Substack),
            "other" => Ok(Self::Other),
            _ => Err(format!("Unknown article source: {value}")),
        }
    }
}

#[derive(PartialEq, Eq, Debug, Clone)]
pub struct Article {
    pub id: String,
    pub feed_id: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub url: Option<String>,
    pub content: Option<String>,
    pub content_kind: ContentKind,
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
            feed_id: "test-feed".to_string(),
            title: Some("Test Article".to_string()),
            author: None,
            published_at: None,
            url: Some("https://example.com/article".to_string()),
            content: Some("Test content".to_string()),
            content_kind: ContentKind::Full,
            source: Source::Other,
        };
        assert_eq!(article.id, "1");
        assert_eq!(article.feed_id, "test-feed");
        assert_eq!(article.title.as_deref(), Some("Test Article"));
        assert_eq!(article.author, None);
        assert_eq!(article.published_at, None);
        assert_eq!(article.url.as_deref(), Some("https://example.com/article"));
        assert_eq!(article.source, Source::Other);
        assert_eq!(article.content.as_deref(), Some("Test content"));
        assert_eq!(article.content_kind, ContentKind::Full);
    }

    #[test]
    fn content_kinds_round_trip_through_storage_values() {
        for kind in [
            ContentKind::Full,
            ContentKind::Extracted,
            ContentKind::Excerpt,
            ContentKind::Missing,
            ContentKind::Unknown,
        ] {
            assert_eq!(ContentKind::try_from(kind.as_str()), Ok(kind));
        }
    }

    /// Verifies every source has a stable SQLite representation and round-trips.
    #[test]
    fn source_round_trips_through_storage_value() {
        for source in [Source::Medium, Source::Substack, Source::Other] {
            assert_eq!(Source::try_from(source.as_str()), Ok(source));
        }
    }

    /// Verifies corrupted or future source values are not silently accepted.
    #[test]
    fn unknown_storage_source_is_rejected() {
        assert_eq!(
            Source::try_from("newsletter"),
            Err("Unknown article source: newsletter".to_string())
        );
    }
}
