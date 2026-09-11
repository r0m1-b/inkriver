use std::future::Future;
use std::sync::LazyLock;

use crate::article::Article;
use crate::page_http::download_public_resource;
use crate::storage::Storage;
use anyhow::Result;
use futures_util::stream::{self, StreamExt};
use regex::{Captures, Regex};
use reqwest::Url;
use serde::Deserialize;

const MAX_X_CARDS_PER_ARTICLE: usize = 8;
const MAX_STORED_X_ARTICLES_PER_REFRESH: usize = 20;
const MAX_CONCURRENT_X_ARTICLES: usize = 2;
const MAX_CONCURRENT_X_REQUESTS: usize = 2;
const MAX_X_OEMBED_BYTES: usize = 128 * 1024;

static IFRAME_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<iframe\b(?P<attributes>[^>]*?)(?:\s*/>|>.*?</iframe\s*>)")
        .expect("the iframe pattern is valid")
});
static SRC_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)\bsrc\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+))"#)
        .expect("the iframe source pattern is valid")
});
static X_FALLBACK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?is)<blockquote>\s*<a\s+href="(https://x\.com/i/status/[A-Za-z0-9_-]+)"[^>]*>\s*Voir la publication sur X\s*</a>\s*</blockquote>"#,
    )
    .expect("the X fallback pattern is valid")
});
static BLOCKQUOTE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<blockquote\b[^>]*>(?P<body>.*?)</blockquote\s*>")
        .expect("the blockquote pattern is valid")
});
static LINK_HREF_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)<a\b[^>]*\bhref\s*=\s*"([^"]+)"[^>]*>"#).expect("the link pattern is valid")
});
static HTML_TAG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<[^>]*>").expect("the HTML tag pattern is valid"));
static HTML_SPACE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)&(?:nbsp|ZeroWidthSpace|#0*(?:160|173|8203|8204|8205|8288|65279)|#x0*(?:a0|ad|200b|200c|200d|2060|feff));",
    )
    .expect("the HTML space pattern is valid")
});

#[derive(Deserialize)]
struct XOEmbedResponse {
    html: String,
}

/// Keeps supported external media visible without retaining executable embeds.
///
/// YouTube and X iframes are converted to inert HTML before Ammonia removes
/// scripts, event handlers and every unsupported frame. The reader later styles
/// these safe fragments and delegates their HTTP(S) links to the system opener.
pub fn sanitize_article_html(html: &str) -> String {
    let expanded = IFRAME_RE.replace_all(html, |captures: &Captures<'_>| {
        let Some(source) = iframe_source(&captures["attributes"]) else {
            return captures[0].to_string();
        };
        if let Some(video_id) = youtube_video_id(source) {
            return youtube_card(&video_id);
        }
        if let Some(tweet_url) = x_status_url(source) {
            return x_card(&tweet_url);
        }
        captures[0].to_string()
    });
    normalize_empty_x_cards(&ammonia::clean(expanded.as_ref()))
}

fn normalize_empty_x_cards(html: &str) -> String {
    BLOCKQUOTE_RE
        .replace_all(html, |captures: &Captures<'_>| {
            let body = &captures["body"];
            if has_visible_text(body) {
                return captures[0].to_string();
            }
            LINK_HREF_RE
                .captures_iter(body)
                .filter_map(|link| link.get(1))
                .find_map(|href| x_status_url(href.as_str()))
                .map(|status_url| x_card(&status_url))
                .unwrap_or_else(|| captures[0].to_string())
        })
        .into_owned()
}

fn has_visible_text(html: &str) -> bool {
    let without_tags = HTML_TAG_RE.replace_all(html, "");
    HTML_SPACE_RE
        .replace_all(&without_tags, "")
        .chars()
        .any(|character| {
            !character.is_whitespace()
                && !matches!(
                    character,
                    '\u{00ad}' | '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{2060}' | '\u{feff}'
                )
        })
}

/// Resolves fallback X cards in several articles without blocking collection
/// when a post is private, deleted, rate-limited or temporarily unavailable.
pub async fn enrich_x_cards_in_articles(articles: &mut [Article]) {
    stream::iter(articles.iter_mut())
        .for_each_concurrent(MAX_CONCURRENT_X_ARTICLES, |article| async {
            if let Some(content) = article.content.as_mut()
                && content.contains("Voir la publication sur X")
            {
                *content = enrich_x_cards(content).await;
            }
        })
        .await;
}

/// Resolves the text of X frames that contained only a post identifier.
pub async fn enrich_x_cards(html: &str) -> String {
    enrich_x_cards_with_loader(html, load_x_oembed).await
}

/// Repairs already cached fallback cards and stores their static text so it
/// remains readable during later offline sessions.
pub async fn enrich_stored_x_cards(storage: &Storage) -> Result<()> {
    let candidates = storage
        .unresolved_x_embed_articles(MAX_STORED_X_ARTICLES_PER_REFRESH)
        .await?;
    let mut resolutions = stream::iter(candidates)
        .map(|(article_id, content)| async move {
            let enriched = enrich_x_cards(&content).await;
            (article_id, content, enriched)
        })
        .buffer_unordered(MAX_CONCURRENT_X_ARTICLES);

    while let Some((article_id, content, enriched)) = resolutions.next().await {
        if enriched != content {
            storage
                .replace_article_content_if_current(&article_id, &content, &enriched)
                .await?;
        }
    }
    Ok(())
}

async fn load_x_oembed(status_url: String) -> Result<String, String> {
    let endpoint = x_oembed_url(&status_url)?;
    let resource = download_public_resource(endpoint.as_str(), MAX_X_OEMBED_BYTES)
        .await
        .map_err(|error| error.to_string())?;
    let response = serde_json::from_slice::<XOEmbedResponse>(&resource.bytes)
        .map_err(|error| format!("invalid X oEmbed response: {error}"))?;
    Ok(response.html)
}

async fn enrich_x_cards_with_loader<F, Fut>(html: &str, loader: F) -> String
where
    F: Fn(String) -> Fut + Clone,
    Fut: Future<Output = Result<String, String>>,
{
    let normalized = normalize_empty_x_cards(html);
    let mut urls = X_FALLBACK_RE
        .captures_iter(&normalized)
        .filter_map(|captures| captures.get(1).map(|value| value.as_str().to_string()))
        .collect::<Vec<_>>();
    urls.sort();
    urls.dedup();
    urls.truncate(MAX_X_CARDS_PER_ARTICLE);

    let resolutions = stream::iter(urls)
        .map(|status_url| {
            let loader = loader.clone();
            async move {
                let resolved = loader(status_url.clone())
                    .await
                    .ok()
                    .map(|embed| sanitize_article_html(&embed))
                    .filter(|embed| !embed.trim().is_empty());
                (status_url, resolved)
            }
        })
        .buffer_unordered(MAX_CONCURRENT_X_REQUESTS)
        .collect::<Vec<_>>()
        .await;

    resolutions
        .into_iter()
        .fold(normalized, |result, (url, resolved)| {
            let Some(embed) = resolved else {
                return result;
            };
            X_FALLBACK_RE
                .replace_all(&result, |captures: &Captures<'_>| {
                    if captures.get(1).is_some_and(|value| value.as_str() == url) {
                        embed.clone()
                    } else {
                        captures[0].to_string()
                    }
                })
                .into_owned()
        })
}

fn x_oembed_url(status_url: &str) -> Result<Url, String> {
    let mut endpoint = Url::parse("https://publish.x.com/oembed")
        .map_err(|error| format!("invalid X oEmbed endpoint: {error}"))?;
    endpoint
        .query_pairs_mut()
        .append_pair("url", status_url)
        .append_pair("omit_script", "true")
        .append_pair("dnt", "true");
    Ok(endpoint)
}

fn iframe_source(attributes: &str) -> Option<&str> {
    let captures = SRC_RE.captures(attributes)?;
    captures
        .get(1)
        .or_else(|| captures.get(2))
        .or_else(|| captures.get(3))
        .map(|value| value.as_str().trim())
        .filter(|value| !value.is_empty())
}

fn parse_web_url(raw_url: &str) -> Option<Url> {
    let decoded = raw_url.replace("&amp;", "&");
    let absolute = if decoded.starts_with("//") {
        format!("https:{decoded}")
    } else {
        decoded
    };
    let url = Url::parse(&absolute).ok()?;
    matches!(url.scheme(), "http" | "https").then_some(url)
}

fn is_host(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}

fn valid_media_id(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')))
    .then(|| value.to_string())
}

fn youtube_video_id(raw_url: &str) -> Option<String> {
    let url = parse_web_url(raw_url)?;
    let host = url.host_str()?.trim_end_matches('.').to_ascii_lowercase();
    let segments = url
        .path_segments()?
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();

    if is_host(&host, "youtu.be") {
        return segments.first().and_then(|value| valid_media_id(value));
    }
    if !is_host(&host, "youtube.com") && !is_host(&host, "youtube-nocookie.com") {
        return None;
    }
    if matches!(segments.first().copied(), Some("embed" | "shorts" | "live")) {
        return segments.get(1).and_then(|value| valid_media_id(value));
    }
    if segments.first().copied() == Some("watch") {
        return url
            .query_pairs()
            .find(|(name, _)| name == "v")
            .and_then(|(_, value)| valid_media_id(value.as_ref()));
    }
    None
}

fn x_status_url(raw_url: &str) -> Option<String> {
    let url = parse_web_url(raw_url)?;
    let host = url.host_str()?.trim_end_matches('.').to_ascii_lowercase();
    if is_host(&host, "platform.twitter.com") {
        let id = url
            .query_pairs()
            .find(|(name, _)| name == "id")
            .and_then(|(_, value)| valid_media_id(value.as_ref()))?;
        return Some(format!("https://x.com/i/status/{id}"));
    }
    if !is_host(&host, "x.com") && !is_host(&host, "twitter.com") {
        return None;
    }
    let segments = url
        .path_segments()?
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let status_index = segments.iter().position(|part| *part == "status")?;
    let id = segments
        .get(status_index + 1)
        .and_then(|value| valid_media_id(value))?;
    Some(format!("https://x.com/i/status/{id}"))
}

fn youtube_card(video_id: &str) -> String {
    format!(
        r#"<figure><a href="https://www.youtube.com/watch?v={video_id}"><img src="https://i.ytimg.com/vi/{video_id}/hqdefault.jpg" alt="Miniature de la vidéo YouTube"></a><figcaption><a href="https://www.youtube.com/watch?v={video_id}">Voir la vidéo sur YouTube</a></figcaption></figure>"#
    )
}

fn x_card(status_url: &str) -> String {
    format!(r#"<blockquote><a href="{status_url}">Voir la publication sur X</a></blockquote>"#)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn replaces_youtube_iframes_with_static_safe_cards() {
        let html = sanitize_article_html(
            r#"<p>Avant</p><iframe src="https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?autoplay=1" allow="autoplay"></iframe><p>Après</p>"#,
        );

        assert!(!html.contains("iframe"));
        assert!(!html.contains("autoplay"));
        assert!(html.contains("<figure>"));
        assert!(html.contains("<figcaption>"));
        assert!(html.contains("https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg"));
        assert!(html.contains("https://www.youtube.com/watch?v=dQw4w9WgXcQ"));
        assert!(html.contains("Voir la vidéo sur YouTube"));
    }

    #[test]
    fn accepts_common_youtube_url_shapes() {
        for url in [
            "https://youtu.be/dQw4w9WgXcQ",
            "https://youtube.com/watch?v=dQw4w9WgXcQ&t=3",
            "https://www.youtube.com/shorts/dQw4w9WgXcQ",
            "//www.youtube.com/embed/dQw4w9WgXcQ",
        ] {
            assert_eq!(youtube_video_id(url).as_deref(), Some("dQw4w9WgXcQ"));
        }
    }

    #[test]
    fn replaces_x_iframes_with_static_links() {
        let html = sanitize_article_html(
            r#"<iframe src="https://platform.twitter.com/embed/Tweet.html?id=1840000000000000000"><script>bad()</script></iframe>"#,
        );

        assert!(!html.contains("iframe"));
        assert!(!html.contains("script"));
        assert!(html.contains("Voir la publication sur X"));
        assert!(html.contains("https://x.com/i/status/1840000000000000000"));
    }

    #[test]
    fn turns_empty_x_blockquotes_into_resolvable_fallbacks() {
        let input = concat!(
            r#"<blockquote class="twitter-tweet"><a href="https://twitter.com/example/status/1840000000000000000?ref_src=embed" rel="noopener noreferrer">"#,
            "\n\u{200b}",
            "</a></blockquote>",
        );
        let html = sanitize_article_html(input);

        assert_eq!(
            html,
            r#"<blockquote><a href="https://x.com/i/status/1840000000000000000">Voir la publication sur X</a></blockquote>"#
        );
    }

    #[test]
    fn keeps_x_blockquotes_that_already_contain_the_post_text() {
        let html = sanitize_article_html(
            r#"<blockquote class="twitter-tweet"><p>Le contenu est déjà présent.</p><a href="https://x.com/example/status/1840000000000000000">Date</a></blockquote>"#,
        );

        assert!(html.contains("Le contenu est déjà présent."));
        assert!(html.contains(">Date</a>"));
        assert!(!html.contains("Voir la publication sur X"));
    }

    #[test]
    fn leaves_unknown_frames_for_the_sanitizer_to_remove() {
        let html = sanitize_article_html(
            r#"<p>Visible</p><iframe src="https://tracker.example/frame"></iframe><iframe src="https://youtube.com.attacker.example/embed/dQw4w9WgXcQ"></iframe>"#,
        );

        assert_eq!(html, "<p>Visible</p>");
    }

    #[tokio::test]
    async fn enriches_x_fallbacks_with_static_oembed_html() {
        let fallback = sanitize_article_html(
            r#"<iframe src="https://platform.twitter.com/embed/Tweet.html?id=1840000000000000000"></iframe>"#,
        );
        assert!(X_FALLBACK_RE.is_match(&fallback), "{fallback}");
        let enriched = enrich_x_cards_with_loader(&fallback, |_url| async {
            Ok(r#"<blockquote class="twitter-tweet"><p>Le texte retrouvé.</p>&mdash; Camille <a href="https://x.com/camille/status/1840000000000000000">7 septembre 2026</a></blockquote><script>remote()</script>"#.to_string())
        })
        .await;

        assert!(enriched.contains("Le texte retrouvé."));
        assert!(enriched.contains("Camille"));
        assert!(enriched.contains("https://x.com/camille/status/1840000000000000000"));
        assert!(!enriched.contains("Voir la publication sur X"));
        assert!(!enriched.contains("script"));
    }

    #[tokio::test]
    async fn keeps_x_fallbacks_when_oembed_is_unavailable_and_deduplicates_requests() {
        let fallback = sanitize_article_html(
            r#"<iframe src="https://platform.twitter.com/embed/Tweet.html?id=1840000000000000000"></iframe>"#,
        );
        let repeated = format!("{fallback}{fallback}");
        let calls = Arc::new(AtomicUsize::new(0));
        let loader_calls = Arc::clone(&calls);
        let enriched = enrich_x_cards_with_loader(&repeated, move |_url| {
            let loader_calls = Arc::clone(&loader_calls);
            async move {
                loader_calls.fetch_add(1, Ordering::SeqCst);
                Err("post unavailable".to_string())
            }
        })
        .await;

        assert_eq!(enriched, repeated);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
