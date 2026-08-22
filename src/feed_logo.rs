use crate::page_http::{DownloadedResource, download_public_resource};
use crate::storage::{FeedLogoCandidate, Storage};
use anyhow::Result;
use chrono::{DateTime, Utc};
use futures_util::stream::{self, StreamExt};
use image::{DynamicImage, GenericImageView, ImageFormat, RgbaImage};
use reqwest::Url;
use scraper::{Html, Selector};
use std::collections::HashSet;
use std::future::Future;
use std::io::Cursor;
use std::time::Duration;

pub const MAX_CONCURRENT_LOGO_DISCOVERIES: usize = 4;
pub const MAX_LOGO_BYTES: usize = 512 * 1024;
pub const MAX_SITE_HTML_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_HTML_ICON_CANDIDATES: usize = 5;
pub const LOGO_SIZE: u32 = 64;
pub const MAX_SOURCE_DIMENSION: u32 = 4_096;
pub const MAX_RASTER_ALLOCATION: u64 = 128 * 1024 * 1024;
pub const LOGO_SITE_TIMEOUT: Duration = Duration::from_secs(10);

/// Discovers and caches logos for successfully refreshed feeds that are due.
pub async fn enrich_feed_logos(
    storage: &Storage,
    successful_feed_ids: &[String],
    now: DateTime<Utc>,
) -> Result<()> {
    enrich_feed_logos_with_loader(
        storage,
        successful_feed_ids,
        now,
        |url, max_bytes| async move {
            download_public_resource(&url, max_bytes)
                .await
                .map_err(|error| error.to_string())
        },
    )
    .await
}

async fn enrich_feed_logos_with_loader<F, Fut>(
    storage: &Storage,
    successful_feed_ids: &[String],
    now: DateTime<Utc>,
    loader: F,
) -> Result<()>
where
    F: Fn(String, usize) -> Fut + Clone,
    Fut: Future<Output = std::result::Result<DownloadedResource, String>>,
{
    let candidates = storage
        .feed_logo_candidates(successful_feed_ids, now)
        .await?;
    let mut discoveries = stream::iter(candidates)
        .map(|candidate| {
            let loader = loader.clone();
            async move {
                let result =
                    tokio::time::timeout(LOGO_SITE_TIMEOUT, discover_logo(&candidate, loader))
                        .await
                        .map_err(|_| "logo discovery timed out".to_string())
                        .and_then(|result| result);
                (candidate, result)
            }
        })
        .buffer_unordered(MAX_CONCURRENT_LOGO_DISCOVERIES);

    while let Some((candidate, discovery)) = discoveries.next().await {
        match discovery {
            Ok(png) => {
                storage
                    .record_feed_logo_success(&candidate, &png, now)
                    .await?;
            }
            Err(error) => {
                storage
                    .record_feed_logo_failure(&candidate, &error, now)
                    .await?;
            }
        }
    }
    Ok(())
}

async fn discover_logo<F, Fut>(
    candidate: &FeedLogoCandidate,
    loader: F,
) -> std::result::Result<Vec<u8>, String>
where
    F: Fn(String, usize) -> Fut + Clone,
    Fut: Future<Output = std::result::Result<DownloadedResource, String>>,
{
    let site_url =
        Url::parse(&candidate.site_url).map_err(|_| "feed website URL is invalid".to_string())?;
    let mut attempted = HashSet::new();
    let mut errors = Vec::new();

    if let Some(declared) = candidate.declared_icon_url.as_deref()
        && let Ok(url) = site_url.join(declared)
        && is_http_url(&url)
        && let Some(png) = try_icon(&url, &loader, &mut attempted, &mut errors).await
    {
        return Ok(png);
    }

    let homepage = loader(candidate.site_url.clone(), MAX_SITE_HTML_BYTES).await;
    let mut final_site_url = site_url.clone();
    if let Ok(page) = homepage {
        if let Ok(page_url) = Url::parse(&page.final_url) {
            final_site_url = page_url;
        }
        if is_html(&page) {
            for url in html_icon_urls(&page.bytes, &final_site_url) {
                if let Some(png) = try_icon(&url, &loader, &mut attempted, &mut errors).await {
                    return Ok(png);
                }
            }
        } else {
            errors.push("website did not return HTML".to_string());
        }
    } else if let Err(error) = homepage {
        errors.push(error);
    }

    let mut fallback = final_site_url;
    fallback.set_path("/favicon.ico");
    fallback.set_query(None);
    fallback.set_fragment(None);
    if let Some(png) = try_icon(&fallback, &loader, &mut attempted, &mut errors).await {
        return Ok(png);
    }

    Err(errors
        .last()
        .cloned()
        .unwrap_or_else(|| "website exposes no usable icon".to_string()))
}

async fn try_icon<F, Fut>(
    url: &Url,
    loader: &F,
    attempted: &mut HashSet<String>,
    errors: &mut Vec<String>,
) -> Option<Vec<u8>>
where
    F: Fn(String, usize) -> Fut,
    Fut: Future<Output = std::result::Result<DownloadedResource, String>>,
{
    if !is_http_url(url) || !attempted.insert(url.to_string()) {
        return None;
    }
    match loader(url.to_string(), MAX_LOGO_BYTES).await {
        Ok(resource) => {
            let normalized = tokio::task::spawn_blocking(move || {
                normalize_logo(&resource.bytes, resource.content_type.as_deref())
            })
            .await
            .map_err(|error| format!("logo decoder failed: {error}"))
            .and_then(|result| result);
            match normalized {
                Ok(png) => Some(png),
                Err(error) => {
                    errors.push(error);
                    None
                }
            }
        }
        Err(error) => {
            errors.push(error);
            None
        }
    }
}

fn is_http_url(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.username().is_empty()
        && url.password().is_none()
}

fn is_html(resource: &DownloadedResource) -> bool {
    match resource.content_type.as_deref() {
        Some(content_type) => {
            let media_type = content_type
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            matches!(media_type.as_str(), "text/html" | "application/xhtml+xml")
        }
        None => {
            let prefix = String::from_utf8_lossy(&resource.bytes[..resource.bytes.len().min(512)])
                .to_ascii_lowercase();
            prefix.contains("<!doctype html") || prefix.contains("<html")
        }
    }
}

fn html_icon_urls(html: &[u8], base_url: &Url) -> Vec<Url> {
    let document = Html::parse_document(&String::from_utf8_lossy(html));
    let selector = Selector::parse("link[href]").expect("static selector is valid");
    let mut urls = Vec::new();
    let mut seen = HashSet::new();
    for element in document.select(&selector) {
        let rel = element
            .value()
            .attr("rel")
            .unwrap_or_default()
            .to_ascii_lowercase();
        let is_icon = rel
            .split_ascii_whitespace()
            .any(|token| token == "icon" || token == "shortcut" || token == "apple-touch-icon")
            || rel.contains("apple-touch-icon");
        if !is_icon {
            continue;
        }
        let Some(href) = element.value().attr("href") else {
            continue;
        };
        let Ok(url) = base_url.join(href) else {
            continue;
        };
        if is_http_url(&url) && seen.insert(url.to_string()) {
            urls.push(url);
            if urls.len() == MAX_HTML_ICON_CANDIDATES {
                break;
            }
        }
    }
    urls
}

fn normalize_logo(
    bytes: &[u8],
    content_type: Option<&str>,
) -> std::result::Result<Vec<u8>, String> {
    let looks_like_svg = content_type
        .is_some_and(|value| value.to_ascii_lowercase().contains("image/svg+xml"))
        || String::from_utf8_lossy(&bytes[..bytes.len().min(512)])
            .to_ascii_lowercase()
            .contains("<svg");
    if looks_like_svg {
        normalize_svg(bytes)
    } else {
        normalize_raster(bytes)
    }
}

fn normalize_raster(bytes: &[u8]) -> std::result::Result<Vec<u8>, String> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| format!("invalid logo image: {error}"))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_DIMENSION);
    limits.max_image_height = Some(MAX_SOURCE_DIMENSION);
    limits.max_alloc = Some(MAX_RASTER_ALLOCATION);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|error| format!("invalid logo image: {error}"))?;
    let (width, height) = image.dimensions();
    validate_dimensions(width, height)?;
    let resized = image.thumbnail(LOGO_SIZE, LOGO_SIZE).to_rgba8();
    let mut canvas = RgbaImage::new(LOGO_SIZE, LOGO_SIZE);
    let x = i64::from((LOGO_SIZE - resized.width()) / 2);
    let y = i64::from((LOGO_SIZE - resized.height()) / 2);
    image::imageops::overlay(&mut canvas, &resized, x, y);
    encode_png(DynamicImage::ImageRgba8(canvas))
}

fn normalize_svg(bytes: &[u8]) -> std::result::Result<Vec<u8>, String> {
    let options = resvg::usvg::Options {
        resources_dir: None,
        image_href_resolver: resvg::usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..resvg::usvg::Options::default()
    };
    let tree = resvg::usvg::Tree::from_data(bytes, &options)
        .map_err(|error| format!("invalid SVG logo: {error}"))?;
    let source = tree.size();
    let width = source.width().ceil() as u32;
    let height = source.height().ceil() as u32;
    validate_dimensions(width, height)?;
    let scale = (LOGO_SIZE as f32 / source.width()).min(LOGO_SIZE as f32 / source.height());
    let x = (LOGO_SIZE as f32 - source.width() * scale) / 2.0;
    let y = (LOGO_SIZE as f32 - source.height() * scale) / 2.0;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(LOGO_SIZE, LOGO_SIZE)
        .ok_or_else(|| "cannot allocate normalized logo".to_string())?;
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale).post_translate(x, y);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    pixmap
        .encode_png()
        .map_err(|error| format!("cannot encode normalized SVG: {error}"))
}

fn validate_dimensions(width: u32, height: u32) -> std::result::Result<(), String> {
    if width == 0
        || height == 0
        || width > MAX_SOURCE_DIMENSION
        || height > MAX_SOURCE_DIMENSION
        || u64::from(width) * u64::from(height)
            > u64::from(MAX_SOURCE_DIMENSION) * u64::from(MAX_SOURCE_DIMENSION)
    {
        return Err("logo dimensions are invalid or excessive".to_string());
    }
    Ok(())
}

fn encode_png(image: DynamicImage) -> std::result::Result<Vec<u8>, String> {
    let mut output = Cursor::new(Vec::new());
    image
        .write_to(&mut output, ImageFormat::Png)
        .map_err(|error| format!("cannot encode normalized logo: {error}"))?;
    Ok(output.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::article::{Article, ContentKind, Source};
    use crate::config::{FeedConfig, Platform};
    use crate::feed::FeedMetadata;
    use chrono::TimeZone;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    fn test_png() -> Vec<u8> {
        let image = DynamicImage::new_rgba8(32, 24);
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    fn resource(url: &str, bytes: Vec<u8>, content_type: &str) -> DownloadedResource {
        DownloadedResource {
            bytes,
            content_type: Some(content_type.to_string()),
            final_url: url.to_string(),
        }
    }

    #[test]
    fn html_icon_discovery_resolves_supported_relations_and_limits_candidates() {
        let html = r#"<html><head>
            <link rel="stylesheet" href="ignore.css">
            <link rel="apple-touch-icon" href="/apple.png">
            <link rel="shortcut icon" href="icons/favicon.ico">
            <link rel="icon" href="https://cdn.example/icon.png">
        </head></html>"#;
        let urls = html_icon_urls(
            html.as_bytes(),
            &Url::parse("https://site.example/blog/").unwrap(),
        );
        assert_eq!(urls[0].as_str(), "https://site.example/apple.png");
        assert_eq!(
            urls[1].as_str(),
            "https://site.example/blog/icons/favicon.ico"
        );
        assert_eq!(urls[2].as_str(), "https://cdn.example/icon.png");
    }

    #[test]
    fn raster_and_svg_are_normalized_to_safe_png_dimensions() {
        let raster = DynamicImage::new_rgba8(32, 32);
        for (format, content_type) in [
            (ImageFormat::Png, "image/png"),
            (ImageFormat::Jpeg, "image/jpeg"),
            (ImageFormat::Gif, "image/gif"),
            (ImageFormat::WebP, "image/webp"),
            (ImageFormat::Ico, "image/x-icon"),
        ] {
            let mut raster_bytes = Cursor::new(Vec::new());
            raster.write_to(&mut raster_bytes, format).unwrap();
            let png = normalize_logo(&raster_bytes.into_inner(), Some(content_type)).unwrap();
            assert_eq!(
                image::load_from_memory(&png).unwrap().dimensions(),
                (64, 64)
            );
        }

        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="60"><rect width="120" height="60" fill="#ff6719"/></svg>"##;
        let png = normalize_logo(svg, Some("image/svg+xml")).unwrap();
        assert_eq!(
            image::load_from_memory(&png).unwrap().dimensions(),
            (64, 64)
        );
    }

    #[test]
    fn invalid_or_excessive_images_are_rejected() {
        assert!(normalize_logo(b"not an image", Some("image/png")).is_err());
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="5000" height="10"/>"#;
        assert!(normalize_logo(svg, Some("image/svg+xml")).is_err());
    }

    #[tokio::test]
    async fn declared_icon_is_resolved_and_tried_before_the_website() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let candidate = FeedLogoCandidate {
            feed_id: "feed".to_string(),
            site_url: "https://site.example/articles/".to_string(),
            declared_icon_url: Some("../declared.png".to_string()),
        };
        let png = test_png();
        let result = discover_logo(&candidate, {
            let calls = Arc::clone(&calls);
            move |url, _| {
                let calls = Arc::clone(&calls);
                let png = png.clone();
                async move {
                    calls.lock().unwrap().push(url.clone());
                    Ok(resource(&url, png, "image/png"))
                }
            }
        })
        .await
        .unwrap();

        assert_eq!(
            image::load_from_memory(&result).unwrap().dimensions(),
            (64, 64)
        );
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            ["https://site.example/declared.png"]
        );
    }

    #[tokio::test]
    async fn html_icons_are_tried_in_order_before_origin_favicon() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let candidate = FeedLogoCandidate {
            feed_id: "feed".to_string(),
            site_url: "https://site.example/blog/".to_string(),
            declared_icon_url: None,
        };
        let png = test_png();
        let result = discover_logo(&candidate, {
            let calls = Arc::clone(&calls);
            move |url, _| {
                let calls = Arc::clone(&calls);
                let png = png.clone();
                async move {
                    calls.lock().unwrap().push(url.clone());
                    match url.as_str() {
                        "https://site.example/blog/" => Ok(resource(
                            &url,
                            br#"<html><head><link rel="icon" href="bad.bin"><link rel="apple-touch-icon" href="good.png"></head></html>"#.to_vec(),
                            "text/html",
                        )),
                        "https://site.example/blog/bad.bin" => {
                            Ok(resource(&url, b"bad".to_vec(), "application/octet-stream"))
                        }
                        "https://site.example/blog/good.png" => {
                            Ok(resource(&url, png, "image/png"))
                        }
                        _ => Err("unexpected request".to_string()),
                    }
                }
            }
        })
        .await
        .unwrap();

        assert_eq!(
            image::load_from_memory(&result).unwrap().dimensions(),
            (64, 64)
        );
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            [
                "https://site.example/blog/",
                "https://site.example/blog/bad.bin",
                "https://site.example/blog/good.png"
            ]
        );
    }

    #[tokio::test]
    async fn missing_html_icon_falls_back_to_origin_favicon() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let candidate = FeedLogoCandidate {
            feed_id: "feed".to_string(),
            site_url: "https://site.example/blog/article".to_string(),
            declared_icon_url: None,
        };
        let png = test_png();
        discover_logo(&candidate, {
            let calls = Arc::clone(&calls);
            move |url, _| {
                let calls = Arc::clone(&calls);
                let png = png.clone();
                async move {
                    calls.lock().unwrap().push(url.clone());
                    if url == "https://site.example/blog/article" {
                        Ok(resource(
                            &url,
                            b"<html><head></head></html>".to_vec(),
                            "text/html",
                        ))
                    } else if url == "https://site.example/favicon.ico" {
                        Ok(resource(&url, png, "image/png"))
                    } else {
                        Err("unexpected request".to_string())
                    }
                }
            }
        })
        .await
        .unwrap();

        assert_eq!(
            calls.lock().unwrap().as_slice(),
            [
                "https://site.example/blog/article",
                "https://site.example/favicon.ico"
            ]
        );
    }

    #[tokio::test]
    async fn discovery_failure_is_non_blocking_and_preserves_feed_and_articles() {
        let storage = Storage::open_in_memory().await.unwrap();
        storage
            .import_feeds(&[FeedConfig {
                id: "feed".to_string(),
                platform: Platform::Other,
                url: "https://site.example/feed".to_string(),
            }])
            .await
            .unwrap();
        let now = Utc.with_ymd_and_hms(2026, 8, 22, 12, 0, 0).unwrap();
        storage
            .record_feed_refreshes(
                &[FeedMetadata {
                    id: "feed".to_string(),
                    title: "Site".to_string(),
                    description: "Kept description".to_string(),
                    author: None,
                    site_url: "https://site.example/".to_string(),
                    declared_icon_url: None,
                }],
                &[],
                now,
            )
            .await
            .unwrap();
        storage
            .upsert_articles(&[Article {
                id: "feed::article".to_string(),
                feed_id: "feed".to_string(),
                title: Some("Kept article".to_string()),
                author: None,
                published_at: Some(now),
                url: Some("https://site.example/article".to_string()),
                content: Some("<p>Kept body</p>".to_string()),
                content_kind: ContentKind::Full,
                source: Source::Other,
            }])
            .await
            .unwrap();

        enrich_feed_logos_with_loader(&storage, &["feed".to_string()], now, |_, _| async {
            Err("logo unavailable".to_string())
        })
        .await
        .unwrap();

        let feed = storage.list_feeds().await.unwrap().remove(0);
        assert_eq!(feed.title.as_deref(), Some("Site"));
        assert_eq!(feed.description.as_deref(), Some("Kept description"));
        assert!(feed.last_error.is_none());
        assert!(feed.logo_png.is_none());
        let article = storage.list_articles().await.unwrap().remove(0);
        assert_eq!(article.article.title.as_deref(), Some("Kept article"));
        assert_eq!(article.article.content.as_deref(), Some("<p>Kept body</p>"));
        assert!(
            storage
                .feed_logo_candidates(&["feed".to_string()], now + chrono::Duration::days(6))
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn enrichment_never_exceeds_four_concurrent_sites() {
        let storage = Storage::open_in_memory().await.unwrap();
        let feeds = (0..5)
            .map(|index| FeedConfig {
                id: format!("feed-{index}"),
                platform: Platform::Other,
                url: format!("https://site-{index}.example/feed"),
            })
            .collect::<Vec<_>>();
        storage.import_feeds(&feeds).await.unwrap();
        let now = Utc.with_ymd_and_hms(2026, 8, 22, 12, 0, 0).unwrap();
        let metadata = feeds
            .iter()
            .map(|feed| FeedMetadata {
                id: feed.id.clone(),
                title: feed.id.clone(),
                description: String::new(),
                author: None,
                site_url: feed.url.replace("/feed", "/"),
                declared_icon_url: Some("/icon.png".to_string()),
            })
            .collect::<Vec<_>>();
        storage
            .record_feed_refreshes(&metadata, &[], now)
            .await
            .unwrap();
        let successful = feeds.iter().map(|feed| feed.id.clone()).collect::<Vec<_>>();
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let calls = Arc::new(AtomicUsize::new(0));
        let png = test_png();

        enrich_feed_logos_with_loader(&storage, &successful, now, {
            let active = Arc::clone(&active);
            let maximum = Arc::clone(&maximum);
            let calls = Arc::clone(&calls);
            move |url, _| {
                let active = Arc::clone(&active);
                let maximum = Arc::clone(&maximum);
                let calls = Arc::clone(&calls);
                let png = png.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                    maximum.fetch_max(current, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    Ok(resource(&url, png, "image/png"))
                }
            }
        })
        .await
        .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 5);
        assert_eq!(
            maximum.load(Ordering::SeqCst),
            MAX_CONCURRENT_LOGO_DISCOVERIES
        );
        assert!(
            storage
                .list_feeds()
                .await
                .unwrap()
                .iter()
                .all(|feed| feed.logo_png.is_some())
        );
    }
}
