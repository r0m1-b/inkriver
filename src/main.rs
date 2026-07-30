use feed_rs::parser;
use reader::feed;
use reader::http;

const FEED_URL: &str = "https://example.substack.com/feed";
fn main() -> Result<(), String> {
    let response = http::check_feed_url(FEED_URL)?;

    let content = match response.text() {
        Ok(text) => text,
        Err(error) => return Err(error.to_string()),
    };

    let raw_feed = match parser::parse(content.as_bytes()) {
        Ok(raw_feed) => raw_feed,
        Err(error) => return Err(error.to_string()),
    };

    let feed = feed::Feed::new(
        raw_feed.title.unwrap().content,
        raw_feed.links[0].href.clone(),
        raw_feed.description.unwrap().content,
        raw_feed.entries,
    );

    feed.display();

    Ok(())
}
