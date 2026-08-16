use inkriver::content_extractor::{ContentExtractionError, extract_article_content};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CorpusPage {
    file: String,
    url: String,
    title: String,
    #[serde(rename = "rss")]
    _rss: String,
    expected_outcome: ExpectedOutcome,
    title_contains: String,
    must_contain: Vec<String>,
    must_not_contain: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ExpectedOutcome {
    Extractable,
    Rejected,
}

fn corpus_directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pages")
}

#[test]
#[ignore = "requires the local, untracked tests/pages corpus"]
fn local_blog_pages_match_their_extraction_expectations() {
    let directory = corpus_directory();
    let description_path = directory.join("description.json");
    let descriptions: Vec<CorpusPage> = serde_json::from_str(
        &fs::read_to_string(&description_path).unwrap_or_else(|error| {
            panic!(
                "cannot read the local corpus description {}: {error}",
                description_path.display()
            )
        }),
    )
    .expect("the local corpus description must be valid JSON");

    for page in descriptions {
        let html_path = directory.join(&page.file);
        let html = fs::read_to_string(&html_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", html_path.display()));
        let result = extract_article_content(&html, &page.url);

        match page.expected_outcome {
            ExpectedOutcome::Extractable => {
                let extracted = result.unwrap_or_else(|error| {
                    panic!("expected {:?} to be extractable: {error}", page.title)
                });
                assert!(
                    extracted
                        .title
                        .to_lowercase()
                        .contains(&page.title_contains.to_lowercase()),
                    "unexpected extracted title for {:?}: {:?}",
                    page.title,
                    extracted.title
                );
                for marker in page.must_contain {
                    assert!(
                        extracted.text.contains(&marker),
                        "missing expected marker {marker:?} in {:?}",
                        page.title
                    );
                }
                for marker in page.must_not_contain {
                    assert!(
                        !extracted.text.contains(&marker),
                        "unexpected marker {marker:?} in {:?}",
                        page.title
                    );
                }
            }
            ExpectedOutcome::Rejected => assert!(
                matches!(&result, Err(ContentExtractionError::TooShort { .. })),
                "expected {:?} to be rejected specifically as too short, got {result:?}",
                page.title
            ),
        }
    }
}
