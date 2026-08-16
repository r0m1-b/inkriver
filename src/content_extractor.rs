use std::error::Error;
use std::fmt;

/// Minimum readable text required before web content can replace an RSS fallback.
pub const MIN_EXTRACTED_TEXT_CHARS: usize = 2_000;

/// Limits pathological documents before Legible scores their content candidates.
const MAX_DOCUMENT_ELEMENTS: usize = 50_000;

/// Main article content extracted from one complete HTML document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedContent {
    pub title: String,
    pub byline: Option<String>,
    pub html: String,
    pub text: String,
    pub character_count: usize,
}

/// Explains why a downloaded page cannot safely replace an RSS fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentExtractionError {
    Extraction(String),
    TooShort {
        extracted_chars: usize,
        minimum_chars: usize,
    },
}

impl fmt::Display for ContentExtractionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Extraction(message) => write!(formatter, "article extraction failed: {message}"),
            Self::TooShort {
                extracted_chars,
                minimum_chars,
            } => write!(
                formatter,
                "extracted article is too short: {extracted_chars} characters (minimum: {minimum_chars})"
            ),
        }
    }
}

impl Error for ContentExtractionError {}

/// Extracts and sanitizes the main article from a complete HTML document.
///
/// The base URL must be absolute. It allows Legible to resolve relative links
/// and images before Ammonia applies InkRiver's HTML safety policy.
pub fn extract_article_content(
    html: &str,
    base_url: &str,
) -> Result<ExtractedContent, ContentExtractionError> {
    extract_article_content_with_minimum(html, base_url, MIN_EXTRACTED_TEXT_CHARS)
}

fn extract_article_content_with_minimum(
    html: &str,
    base_url: &str,
    minimum_chars: usize,
) -> Result<ExtractedContent, ContentExtractionError> {
    let options = legible::Options::new()
        .max_elems_to_parse(MAX_DOCUMENT_ELEMENTS)
        .char_threshold(minimum_chars);
    let article = legible::parse(html, Some(base_url), Some(options))
        .map_err(|error| ContentExtractionError::Extraction(error.to_string()))?;
    let text = article.text_content.trim().to_string();
    let character_count = text.chars().count();
    if character_count < minimum_chars {
        return Err(ContentExtractionError::TooShort {
            extracted_chars: character_count,
            minimum_chars,
        });
    }

    Ok(ExtractedContent {
        title: article.title,
        byline: article.byline,
        html: ammonia::clean(&article.content),
        text,
        character_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXTRACTABLE_PAGE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/pages/extractable_article.html"
    ));
    const SHORT_TEASER: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/pages/short_teaser.html"
    ));

    #[test]
    fn extracts_main_content_resolves_links_and_sanitizes_html() {
        let extracted = extract_article_content_with_minimum(
            EXTRACTABLE_PAGE,
            "https://journal.example/notes/rust-reader",
            250,
        )
        .unwrap();

        assert_eq!(extracted.title, "Building a calm feed reader");
        assert_eq!(extracted.byline.as_deref(), Some("Camille Martin"));
        assert!(extracted.text.contains("A useful reader starts"));
        assert!(!extracted.text.contains("Pricing Documentation Sign in"));
        assert!(
            extracted
                .html
                .contains("https://journal.example/notes/next-step")
        );
        assert!(!extracted.html.contains("<script"));
        assert!(!extracted.html.contains("onclick"));
        assert!(!extracted.html.contains("javascript:"));
        assert!(extracted.character_count >= 250);
    }

    #[test]
    fn rejects_a_readable_but_incomplete_teaser() {
        let error = extract_article_content(SHORT_TEASER, "https://newspaper.example/paid-story")
            .unwrap_err();

        assert!(matches!(
            error,
            ContentExtractionError::TooShort {
                minimum_chars: MIN_EXTRACTED_TEXT_CHARS,
                ..
            }
        ));
    }

    #[test]
    fn rejects_an_invalid_base_url() {
        let error =
            extract_article_content_with_minimum(EXTRACTABLE_PAGE, "not a URL", 250).unwrap_err();

        assert!(matches!(error, ContentExtractionError::Extraction(_)));
        assert!(error.to_string().contains("Invalid URL"));
    }
}
