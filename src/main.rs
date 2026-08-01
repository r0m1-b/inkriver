use feed_rs::parser;
use reader::article::Source;
use reader::config::Platform;
use reader::feed;
use reader::http;

const CONFIG_PATH: &str = "feeds.toml";

/// Maps a configured platform to the corresponding article source.
fn get_source_from(platform: Platform) -> Source {
    match platform {
        Platform::Medium => Source::Medium,
        Platform::Substack => Source::Substack,
        Platform::Other => Source::Other,
    }
}

/// Extracts the primary link from a parsed feed.
///
/// # Errors
///
/// Returns an error when the feed does not contain any link.
fn get_link_from(raw_feed: &feed_rs::model::Feed) -> Result<String, String> {
    let link = raw_feed
        .links
        .first()
        .map(|l| l.href.clone())
        .ok_or_else(|| "This RSS feed has no link".to_string())?;
    Ok(link)
}

/// Extracts the title from a parsed feed.
///
/// # Errors
///
/// Returns an error when the feed does not contain a title.
fn get_title_from(raw_feed: &feed_rs::model::Feed) -> Result<String, String> {
    let title: &str = raw_feed
        .title
        .as_ref()
        .map(|t| t.content.as_ref())
        .ok_or_else(|| "This RSS feed has no title".to_string())?;
    Ok(title.to_string())
}

/// Extracts the feed description or returns an empty string when it is absent.
fn get_description_from(raw_feed: &feed_rs::model::Feed) -> String {
    raw_feed
        .description
        .as_ref()
        .map(|d| d.content.clone())
        .unwrap_or_default()
}

/// Builds the application's feed model from a parsed feed and configured platform.
///
/// # Errors
///
/// Returns an error when required feed metadata, such as the title or primary
/// link, is missing.
fn build_feed_from_data(
    raw_feed: feed_rs::model::Feed,
    platform: Platform,
) -> Result<feed::Feed, String> {
    Ok(feed::Feed::new(
        get_title_from(&raw_feed)?,
        get_link_from(&raw_feed)?,
        get_description_from(&raw_feed),
        get_source_from(platform),
        raw_feed.entries,
    ))
}

/// Downloads every configured feed and appends its converted articles to a vector.
///
/// # Errors
///
/// Returns an error when a request fails, a response body cannot be read, an RSS
/// document cannot be parsed, or required feed metadata is missing.
fn get_articles_vector(
    config: reader::config::Config,
    articles: &mut Vec<reader::article::Article>,
) -> Result<(), String> {
    let _: () = for feed_config in &config.feeds {
        println!("Feed ID: {}", feed_config.id);
        println!("Platform: {:?}", feed_config.platform);
        println!("URL: {}", feed_config.url);

        let response = http::check_feed_url(&feed_config.url)?;

        let content = match response.text() {
            Ok(text) => text,
            Err(error) => return Err(error.to_string()),
        };

        let raw_feed = match parser::parse(content.as_bytes()) {
            Ok(raw_feed) => raw_feed,
            Err(error) => return Err(error.to_string()),
        };

        match build_feed_from_data(raw_feed, feed_config.platform) {
            Ok(feed) => articles.extend(feed.get_articles()),
            Err(error) => return Err(error.to_string()),
        };
    };
    Ok(())
}

/// Loads the feed configuration, collects all articles, and sorts them newest first.
///
/// # Errors
///
/// Returns an error when configuration loading or article collection fails.
fn main() -> Result<(), String> {
    let config = reader::config::load_config(std::path::Path::new(CONFIG_PATH))
        .map_err(|error| error.to_string())?;

    let mut articles: Vec<reader::article::Article> = Vec::new();

    get_articles_vector(config, &mut articles)?;
    articles.sort_by_key(|a| std::cmp::Reverse(a.published_at));

    Ok(())
}
