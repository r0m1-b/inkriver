use feed_rs::parser;
use reader::article::Source;
use reader::config::Platform;
use reader::feed;
use reader::http;

const CONFIG_PATH: &str = "feeds.toml";

fn build_feed_from_data(raw_feed: feed_rs::model::Feed, platform: Platform) -> feed::Feed {
    let source = match platform {
        Platform::Medium => Source::Medium,
        Platform::Substack => Source::Substack,
    };
    feed::Feed::new(
        raw_feed.title.unwrap().content,
        raw_feed.links[0].href.clone(),
        raw_feed.description.unwrap().content,
        source,
        raw_feed.entries,
    )
}

fn get_articles_vector(config: reader::config::Config, articles: &mut Vec<reader::article::Article>) -> Result<(), String> {
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

        let feed = build_feed_from_data(raw_feed, feed_config.platform);

        articles.extend(feed.get_articles());
    };
    Ok(())
}

fn main() -> Result<(), String> {
    let config = reader::config::load_config(std::path::Path::new(CONFIG_PATH)).unwrap();

    let mut articles: Vec<reader::article::Article> = Vec::new();

    get_articles_vector(config, &mut articles)?;
    articles.sort_by_key(|article| article.published_at);

    Ok(())
}
