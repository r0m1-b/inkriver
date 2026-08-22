use reqwest::Url;
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE, LOCATION};
use std::error::Error;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

pub const PAGE_TIMEOUT: Duration = Duration::from_secs(10);
pub const MAX_PAGE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_REDIRECTS: usize = 5;

#[derive(Debug, Clone, Copy)]
struct DownloadLimits {
    timeout: Duration,
    max_bytes: usize,
    max_redirects: usize,
    allow_private_addresses: bool,
}

const PRODUCTION_LIMITS: DownloadLimits = DownloadLimits {
    timeout: PAGE_TIMEOUT,
    max_bytes: MAX_PAGE_BYTES,
    max_redirects: MAX_REDIRECTS,
    allow_private_addresses: false,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadedPage {
    pub html: String,
    pub final_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadedResource {
    pub bytes: Vec<u8>,
    pub content_type: Option<String>,
    pub final_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageDownloadError(String);

impl PageDownloadError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for PageDownloadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for PageDownloadError {}

/// Downloads one public HTML page with strict resource and redirect limits.
pub async fn download_article_page(url: &str) -> Result<DownloadedPage, PageDownloadError> {
    download_article_page_with_limits(url, PRODUCTION_LIMITS).await
}

/// Downloads one public HTTP(S) resource with the article network protections.
pub async fn download_public_resource(
    url: &str,
    max_bytes: usize,
) -> Result<DownloadedResource, PageDownloadError> {
    let limits = DownloadLimits {
        max_bytes,
        ..PRODUCTION_LIMITS
    };
    tokio::time::timeout(limits.timeout, download_public_resource_inner(url, limits))
        .await
        .map_err(|_| PageDownloadError::new("resource download timed out"))?
}

async fn download_article_page_with_limits(
    url: &str,
    limits: DownloadLimits,
) -> Result<DownloadedPage, PageDownloadError> {
    tokio::time::timeout(limits.timeout, download_article_page_inner(url, limits))
        .await
        .map_err(|_| PageDownloadError::new("article page timed out"))?
}

async fn download_article_page_inner(
    url: &str,
    limits: DownloadLimits,
) -> Result<DownloadedPage, PageDownloadError> {
    let resource = download_public_resource_inner(url, limits).await?;
    validate_html_content(resource.content_type.as_deref(), &resource.bytes)?;
    Ok(DownloadedPage {
        html: String::from_utf8_lossy(&resource.bytes).into_owned(),
        final_url: resource.final_url,
    })
}

async fn download_public_resource_inner(
    url: &str,
    limits: DownloadLimits,
) -> Result<DownloadedResource, PageDownloadError> {
    let mut current = validate_page_url(url, limits.allow_private_addresses)?;

    for redirect_count in 0..=limits.max_redirects {
        let (client, resolved_addresses) =
            client_for_url(&current, limits.allow_private_addresses, limits.timeout).await?;
        let mut response =
            client.get(current.clone()).send().await.map_err(|error| {
                PageDownloadError::new(format!("article request failed: {error}"))
            })?;

        if let Some(remote) = response.remote_addr()
            && ((!limits.allow_private_addresses && !is_public_ip(remote.ip()))
                || !resolved_addresses
                    .iter()
                    .any(|address| address.ip() == remote.ip()))
        {
            return Err(PageDownloadError::new(
                "article server resolved to a forbidden network address",
            ));
        }

        if response.status().is_redirection() {
            if redirect_count == limits.max_redirects {
                return Err(PageDownloadError::new("too many article redirects"));
            }
            let location = response
                .headers()
                .get(LOCATION)
                .ok_or_else(|| PageDownloadError::new("article redirect has no Location header"))?
                .to_str()
                .map_err(|_| PageDownloadError::new("article redirect Location is invalid"))?;
            current = validate_page_url(
                current
                    .join(location)
                    .map_err(|_| PageDownloadError::new("article redirect URL is invalid"))?
                    .as_str(),
                limits.allow_private_addresses,
            )?;
            continue;
        }

        if !response.status().is_success() {
            return Err(PageDownloadError::new(format!(
                "article returned HTTP status {}",
                response.status()
            )));
        }

        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        if let Some(length) = response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            && length > limits.max_bytes as u64
        {
            return Err(PageDownloadError::new(
                "article page exceeds the size limit",
            ));
        }

        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| PageDownloadError::new(format!("cannot read article page: {error}")))?
        {
            if body.len().saturating_add(chunk.len()) > limits.max_bytes {
                return Err(PageDownloadError::new(
                    "article page exceeds the size limit",
                ));
            }
            body.extend_from_slice(&chunk);
        }

        return Ok(DownloadedResource {
            bytes: body,
            content_type,
            final_url: current.into(),
        });
    }

    unreachable!("the redirect loop always returns or continues within its bound")
}

#[cfg(test)]
pub(crate) async fn download_test_resource(
    url: &str,
    max_bytes: usize,
) -> Result<DownloadedResource, PageDownloadError> {
    let limits = DownloadLimits {
        timeout: Duration::from_millis(250),
        max_bytes,
        max_redirects: MAX_REDIRECTS,
        allow_private_addresses: true,
    };
    tokio::time::timeout(limits.timeout, download_public_resource_inner(url, limits))
        .await
        .map_err(|_| PageDownloadError::new("resource download timed out"))?
}

async fn client_for_url(
    url: &Url,
    allow_private_addresses: bool,
    timeout: Duration,
) -> Result<(reqwest::Client, Vec<SocketAddr>), PageDownloadError> {
    let host = url
        .host_str()
        .ok_or_else(|| PageDownloadError::new("article URL has no host"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| PageDownloadError::new("article URL has no usable port"))?;
    let literal_ip = host.trim_matches(['[', ']']).parse::<IpAddr>().ok();
    let addresses = if let Some(ip) = literal_ip {
        vec![SocketAddr::new(ip, port)]
    } else {
        tokio::net::lookup_host((host, port))
            .await
            .map_err(|error| PageDownloadError::new(format!("article DNS failed: {error}")))?
            .collect::<Vec<_>>()
    };
    if addresses.is_empty()
        || (!allow_private_addresses && addresses.iter().any(|address| !is_public_ip(address.ip())))
    {
        return Err(PageDownloadError::new(
            "article host resolves to a forbidden network address",
        ));
    }

    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .no_proxy()
        .user_agent("InkRiver/0.1 (+https://github.com/r0m1-b/inkriver)");
    if literal_ip.is_none() {
        builder = builder.resolve_to_addrs(host, &addresses);
    }
    let client = builder
        .build()
        .map_err(|error| PageDownloadError::new(format!("cannot build HTTP client: {error}")))?;
    Ok((client, addresses))
}

fn validate_page_url(
    raw_url: &str,
    allow_private_addresses: bool,
) -> Result<Url, PageDownloadError> {
    let url = Url::parse(raw_url).map_err(|_| PageDownloadError::new("article URL is invalid"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(PageDownloadError::new("article URL must use HTTP or HTTPS"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(PageDownloadError::new(
            "article URL must not contain credentials",
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| PageDownloadError::new("article URL has no host"))?;
    let lowercase_host = host.to_ascii_lowercase();
    if !allow_private_addresses
        && (lowercase_host == "localhost"
            || lowercase_host.ends_with(".localhost")
            || lowercase_host.ends_with(".local"))
    {
        return Err(PageDownloadError::new("article host is not public"));
    }
    if !allow_private_addresses
        && let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>()
        && !is_public_ip(ip)
    {
        return Err(PageDownloadError::new("article host is not public"));
    }
    Ok(url)
}

fn validate_html_content(content_type: Option<&str>, body: &[u8]) -> Result<(), PageDownloadError> {
    if let Some(content_type) = content_type {
        let media_type = content_type
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        if !matches!(media_type.as_str(), "text/html" | "application/xhtml+xml") {
            return Err(PageDownloadError::new(format!(
                "article response is not HTML: {media_type}"
            )));
        }
    } else {
        let prefix = String::from_utf8_lossy(&body[..body.len().min(512)]).to_ascii_lowercase();
        let prefix = prefix.trim_start_matches(['\u{feff}', ' ', '\t', '\r', '\n']);
        if !prefix.starts_with("<!doctype html") && !prefix.starts_with("<html") {
            return Err(PageDownloadError::new(
                "article response has no HTML content type or signature",
            ));
        }
    }
    Ok(())
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_ipv4(ip),
        IpAddr::V6(ip) => is_public_ipv6(ip),
    }
}

fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_multicast()
        || ip.is_broadcast()
        || a == 0
        || a >= 224
        || (a == 100 && (64..=127).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113))
}

fn is_public_ipv6(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();
    if let Some(ipv4) = ip.to_ipv4_mapped() {
        return is_public_ipv4(ipv4);
    }
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] == 0x2001 && segments[1] == 0x0db8))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn local_server(
        responses: Vec<(Duration, String)>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            for (delay, response) in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0_u8; 2_048];
                let _ = socket.read(&mut request).await.unwrap();
                tokio::time::sleep(delay).await;
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        (format!("http://{address}"), task)
    }

    fn test_limits() -> DownloadLimits {
        DownloadLimits {
            timeout: Duration::from_secs(1),
            max_bytes: 1_024,
            max_redirects: 5,
            allow_private_addresses: true,
        }
    }

    #[test]
    fn accepts_only_public_http_urls_without_credentials() {
        assert!(validate_page_url("https://journal.example/article", false).is_ok());
        for url in [
            "file:///etc/passwd",
            "https://user:secret@example.com/article",
            "http://localhost/article",
            "http://news.local/article",
            "http://127.0.0.1/article",
            "http://10.1.2.3/article",
            "http://[::1]/article",
        ] {
            assert!(
                validate_page_url(url, false).is_err(),
                "unexpectedly accepted {url}"
            );
        }
    }

    #[test]
    fn accepts_html_types_and_sniffs_missing_content_types() {
        assert!(validate_html_content(Some("text/html; charset=utf-8"), b"body").is_ok());
        assert!(validate_html_content(Some("application/xhtml+xml"), b"body").is_ok());
        assert!(validate_html_content(None, b"  <!doctype html><html></html>").is_ok());
        assert!(validate_html_content(Some("application/json"), b"{}").is_err());
        assert!(validate_html_content(None, b"plain text").is_err());
    }

    #[test]
    fn rejects_reserved_ipv4_and_ipv6_ranges() {
        for ip in [
            "0.0.0.0",
            "100.64.0.1",
            "192.0.2.1",
            "198.51.100.2",
            "203.0.113.3",
            "224.0.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
        ] {
            assert!(
                !is_public_ip(ip.parse().unwrap()),
                "unexpectedly public: {ip}"
            );
        }
        assert!(is_public_ip("1.1.1.1".parse().unwrap()));
        assert!(is_public_ip("2606:4700:4700::1111".parse().unwrap()));
    }

    #[tokio::test]
    async fn follows_controlled_redirects_and_returns_the_final_html() {
        let redirect = "HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        let body = "<!doctype html><html><body>Complete article</body></html>";
        let success = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let (base_url, server) = local_server(vec![
            (Duration::ZERO, redirect.to_string()),
            (Duration::ZERO, success),
        ])
        .await;

        let page = download_article_page_with_limits(&format!("{base_url}/start"), test_limits())
            .await
            .unwrap();

        assert_eq!(page.final_url, format!("{base_url}/final"));
        assert_eq!(page.html, body);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn generic_resource_download_preserves_bytes_type_and_limits() {
        let response = "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 4\r\nConnection: close\r\n\r\nlogo";
        let (base_url, server) = local_server(vec![(Duration::ZERO, response.to_string())]).await;

        let resource = download_test_resource(&format!("{base_url}/logo"), 4)
            .await
            .unwrap();
        server.await.unwrap();
        assert_eq!(resource.bytes, b"logo");
        assert_eq!(resource.content_type.as_deref(), Some("image/png"));

        let oversized = "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\n12345";
        let (base_url, server) = local_server(vec![(Duration::ZERO, oversized.to_string())]).await;
        assert!(download_test_resource(&base_url, 4).await.is_err());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn rejects_redirects_beyond_the_configured_limit() {
        let redirect = "HTTP/1.1 302 Found\r\nLocation: /again\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        let (url, server) = local_server(vec![
            (Duration::ZERO, redirect.to_string()),
            (Duration::ZERO, redirect.to_string()),
        ])
        .await;

        let error = download_article_page_with_limits(
            &url,
            DownloadLimits {
                max_redirects: 1,
                ..test_limits()
            },
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("too many"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn rejects_controlled_non_html_and_oversized_responses() {
        let json = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}";
        let (json_url, json_server) = local_server(vec![(Duration::ZERO, json.to_string())]).await;
        let error = download_article_page_with_limits(&json_url, test_limits())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("not HTML"));
        json_server.await.unwrap();

        let oversized = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n<html>{}</html>",
            "x".repeat(50)
        );
        let (oversized_url, oversized_server) =
            local_server(vec![(Duration::ZERO, oversized)]).await;
        let error = download_article_page_with_limits(
            &oversized_url,
            DownloadLimits {
                max_bytes: 20,
                ..test_limits()
            },
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("size limit"));
        oversized_server.await.unwrap();
    }

    #[tokio::test]
    async fn times_out_a_controlled_slow_response() {
        let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 13\r\nConnection: close\r\n\r\n<html></html>";
        let (url, server) =
            local_server(vec![(Duration::from_millis(100), response.to_string())]).await;

        let error = download_article_page_with_limits(
            &url,
            DownloadLimits {
                timeout: Duration::from_millis(20),
                ..test_limits()
            },
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("timed out"));
        server.abort();
    }
}
