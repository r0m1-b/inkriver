use crate::content_extractor::extract_article_content;
use crate::page_http::{DownloadedPage, download_article_page};
use crate::storage::{ExtractionCandidate, Storage};
use anyhow::Result;
use chrono::{DateTime, Utc};
use futures_util::stream::{self, StreamExt};
use std::future::Future;

pub const MAX_CONCURRENT_EXTRACTIONS: usize = 4;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ExtractionReport {
    pub extracted: usize,
    pub failed: usize,
    pub skipped: usize,
}

/// Downloads and extracts every article page currently due for enrichment.
pub async fn enrich_articles(storage: &Storage, now: DateTime<Utc>) -> Result<ExtractionReport> {
    enrich_articles_with_loader(storage, now, |url| async move {
        download_article_page(&url)
            .await
            .map_err(|error| error.to_string())
    })
    .await
}

async fn enrich_articles_with_loader<F, Fut>(
    storage: &Storage,
    now: DateTime<Utc>,
    loader: F,
) -> Result<ExtractionReport>
where
    F: Fn(String) -> Fut + Clone,
    Fut: Future<Output = Result<DownloadedPage, String>>,
{
    let selection = storage.extraction_candidates(now).await?;
    let mut report = ExtractionReport {
        skipped: selection.skipped,
        ..ExtractionReport::default()
    };
    let mut results = stream::iter(selection.candidates)
        .map(|candidate| {
            let loader = loader.clone();
            async move {
                let result = loader(candidate.url.clone()).await;
                (candidate, result)
            }
        })
        .buffer_unordered(MAX_CONCURRENT_EXTRACTIONS);

    while let Some((candidate, download)) = results.next().await {
        store_download_result(storage, now, candidate, download, &mut report).await?;
    }

    Ok(report)
}

async fn store_download_result(
    storage: &Storage,
    now: DateTime<Utc>,
    candidate: ExtractionCandidate,
    download: Result<DownloadedPage, String>,
    report: &mut ExtractionReport,
) -> Result<()> {
    let extracted = download.and_then(|page| {
        extract_article_content(&page.html, &page.final_url).map_err(|error| error.to_string())
    });
    match extracted {
        Ok(content) => {
            if storage
                .record_extraction_success(
                    &candidate.article_id,
                    &candidate.url,
                    &content.html,
                    now,
                )
                .await?
            {
                report.extracted += 1;
            } else {
                report.skipped += 1;
            }
        }
        Err(error) => {
            if storage
                .record_extraction_failure(&candidate.article_id, &candidate.url, &error, now)
                .await?
            {
                report.failed += 1;
            } else {
                report.skipped += 1;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::article::{Article, ContentKind, Source};
    use crate::config::{FeedConfig, Platform};
    use crate::storage::MAX_EXTRACTION_ATTEMPTS_PER_REFRESH;
    use chrono::TimeZone;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn article(id: &str, published_at: DateTime<Utc>) -> Article {
        Article {
            id: id.to_string(),
            feed_id: "journal".to_string(),
            title: Some(format!("Title for {id}")),
            author: Some("Test Author".to_string()),
            published_at: Some(published_at),
            url: Some(format!("https://journal.example/{id}")),
            content: Some("RSS fallback".to_string()),
            content_kind: ContentKind::Excerpt,
            source: Source::Other,
        }
    }

    async fn storage_with_other_feed() -> Storage {
        let storage = Storage::open_in_memory().await.unwrap();
        storage
            .import_feeds(&[FeedConfig {
                id: "journal".to_string(),
                platform: Platform::Other,
                url: "https://journal.example/feed".to_string(),
            }])
            .await
            .unwrap();
        storage
    }

    fn readable_page() -> String {
        let paragraph = "A calm feed reader preserves useful context while presenting every article in a focused and readable stream. ";
        format!(
            "<!doctype html><html><head><title>Complete article</title></head><body><main><article><h1>Complete article</h1><p>{}</p></article></main></body></html>",
            paragraph.repeat(30)
        )
    }

    #[tokio::test]
    async fn successful_extraction_replaces_the_fallback_and_failure_enters_cooldown() {
        let storage = storage_with_other_feed().await;
        let now = Utc.with_ymd_and_hms(2026, 8, 16, 12, 0, 0).unwrap();
        storage
            .upsert_articles(&[
                article("success", now),
                article("failure", now - chrono::Duration::minutes(1)),
                article("short", now - chrono::Duration::minutes(2)),
            ])
            .await
            .unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let loader_calls = Arc::clone(&calls);
        let report = enrich_articles_with_loader(&storage, now, move |url| {
            let loader_calls = Arc::clone(&loader_calls);
            async move {
                loader_calls.fetch_add(1, Ordering::SeqCst);
                if url.ends_with("success") {
                    Ok(DownloadedPage {
                        html: readable_page(),
                        final_url: url,
                    })
                } else if url.ends_with("short") {
                    Ok(DownloadedPage {
                        html: "<!doctype html><html><body><article><p>Short teaser only.</p></article></body></html>".to_string(),
                        final_url: url,
                    })
                } else {
                    Err("temporary download failure".to_string())
                }
            }
        })
        .await
        .unwrap();

        assert_eq!(
            report,
            ExtractionReport {
                extracted: 1,
                failed: 2,
                skipped: 0
            }
        );
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        let success = storage.get_article("success").await.unwrap().unwrap();
        assert_eq!(success.article.content_kind, ContentKind::Extracted);
        assert!(
            success
                .article
                .content
                .unwrap()
                .contains("calm feed reader")
        );
        let failure = storage.get_article("failure").await.unwrap().unwrap();
        assert_eq!(failure.article.content_kind, ContentKind::Excerpt);
        assert_eq!(failure.article.content.as_deref(), Some("RSS fallback"));
        let short = storage.get_article("short").await.unwrap().unwrap();
        assert_eq!(short.article.content_kind, ContentKind::Excerpt);
        assert_eq!(short.article.content.as_deref(), Some("RSS fallback"));

        let second_calls = Arc::clone(&calls);
        let second = enrich_articles_with_loader(&storage, now, move |_| {
            let second_calls = Arc::clone(&second_calls);
            async move {
                second_calls.fetch_add(1, Ordering::SeqCst);
                Err("must not be called during cooldown".to_string())
            }
        })
        .await
        .unwrap();
        assert_eq!(
            second,
            ExtractionReport {
                extracted: 0,
                failed: 0,
                skipped: 2
            }
        );
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn extraction_never_exceeds_four_concurrent_downloads_or_twenty_attempts() {
        let storage = storage_with_other_feed().await;
        let now = Utc.with_ymd_and_hms(2026, 8, 16, 12, 0, 0).unwrap();
        let articles = (0..25)
            .map(|index| {
                article(
                    &format!("article-{index:02}"),
                    now - chrono::Duration::minutes(index),
                )
            })
            .collect::<Vec<_>>();
        storage.upsert_articles(&articles).await.unwrap();
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let calls = Arc::new(AtomicUsize::new(0));

        let report = enrich_articles_with_loader(&storage, now, {
            let active = Arc::clone(&active);
            let maximum = Arc::clone(&maximum);
            let calls = Arc::clone(&calls);
            move |_| {
                let active = Arc::clone(&active);
                let maximum = Arc::clone(&maximum);
                let calls = Arc::clone(&calls);
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let in_flight = active.fetch_add(1, Ordering::SeqCst) + 1;
                    maximum.fetch_max(in_flight, Ordering::SeqCst);
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    Err("controlled failure".to_string())
                }
            }
        })
        .await
        .unwrap();

        assert_eq!(
            calls.load(Ordering::SeqCst),
            MAX_EXTRACTION_ATTEMPTS_PER_REFRESH
        );
        assert!(maximum.load(Ordering::SeqCst) <= MAX_CONCURRENT_EXTRACTIONS);
        assert_eq!(report.failed, MAX_EXTRACTION_ATTEMPTS_PER_REFRESH);
        assert_eq!(report.skipped, 5);
    }

    #[tokio::test]
    async fn archived_articles_are_not_downloaded() {
        let storage = storage_with_other_feed().await;
        let now = Utc.with_ymd_and_hms(2026, 8, 16, 12, 0, 0).unwrap();
        storage
            .upsert_articles(&[article("archived", now)])
            .await
            .unwrap();
        storage.archive_article("archived", now).await.unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let loader_calls = Arc::clone(&calls);

        let report = enrich_articles_with_loader(&storage, now, move |_| {
            let loader_calls = Arc::clone(&loader_calls);
            async move {
                loader_calls.fetch_add(1, Ordering::SeqCst);
                Err("must not be called".to_string())
            }
        })
        .await
        .unwrap();

        assert_eq!(report, ExtractionReport::default());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}
