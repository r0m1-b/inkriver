use std::time::Duration;

const FEED_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const FEED_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const USER_AGENT: &str = "InkRiver/0.3 (+https://github.com/r0m1-b/inkriver)";

/// Creates a request client builder configured for the current platform.
///
/// Android cannot use reqwest's platform verifier until its JVM bridge has
/// been initialized. InkRiver instead embeds Mozilla's public roots there,
/// avoiding a TLS panic while keeping the desktop system trust store intact.
pub(crate) fn client_builder() -> Result<reqwest::ClientBuilder, String> {
    let builder = reqwest::Client::builder();

    #[cfg(target_os = "android")]
    {
        let roots = webpki_root_certs::TLS_SERVER_ROOT_CERTS
            .iter()
            .map(|certificate| {
                reqwest::Certificate::from_der(certificate.as_ref())
                    .map_err(|error| format!("cannot load an embedded TLS root: {error}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(builder.tls_certs_only(roots));
    }

    #[cfg(not(target_os = "android"))]
    Ok(builder)
}

/// Accepts successful HTTP statuses and preserves unsuccessful status details.
///
/// # Errors
///
/// Returns an error containing the exact status code and reason for every
/// non-successful response.
fn validate_status(status: reqwest::StatusCode) -> Result<(), String> {
    if status.is_success() {
        Ok(())
    } else {
        Err(format!("HTTP status {status}"))
    }
}

/// Asynchronously downloads a feed URL and returns its successful response.
///
/// # Errors
///
/// Returns an error when the request cannot be completed or when the server
/// responds with a non-successful HTTP status.
pub async fn check_feed_url(url: &str) -> Result<reqwest::Response, String> {
    let client = client_builder()?
        .connect_timeout(FEED_CONNECT_TIMEOUT)
        .timeout(FEED_REQUEST_TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|error| format!("cannot build HTTP client: {error}"))?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| error.to_string())?;

    validate_status(response.status())?;

    Ok(response)
}

#[cfg(test)]
mod tests {

    use super::*;
    use reqwest::StatusCode;

    /// Verifies that the platform-specific client configuration is usable.
    #[test]
    fn configured_client_can_be_built() {
        assert!(
            client_builder()
                .and_then(|builder| builder.build().map_err(|error| error.to_string()))
                .is_ok()
        );
    }

    /// Verifies that a successful HTTP status is accepted.
    #[test]
    fn success_status_is_accepted() {
        let result = validate_status(StatusCode::OK);
        assert_eq!(result, Ok(()));
    }

    /// Verifies that a server-side HTTP failure preserves its exact status.
    #[test]
    fn server_error_is_rejected() {
        let result = validate_status(StatusCode::INTERNAL_SERVER_ERROR);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "HTTP status 500 Internal Server Error");
    }

    /// Verifies that a missing resource preserves its exact status.
    #[test]
    fn not_found_is_rejected_as_unexpected_status() {
        let result = validate_status(StatusCode::NOT_FOUND);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "HTTP status 404 Not Found");
    }
}
