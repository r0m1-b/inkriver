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

/// Downloads a feed URL and returns its response when the HTTP status is successful.
///
/// # Errors
///
/// Returns an error when the request cannot be completed or when the server
/// responds with a non-successful HTTP status.
pub fn check_feed_url(url: &str) -> Result<reqwest::blocking::Response, String> {
    let response = reqwest::blocking::get(url).map_err(|error| error.to_string())?;

    validate_status(response.status())?;

    Ok(response)
}

#[cfg(test)]
mod tests {

    use super::*;
    use reqwest::StatusCode;

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
