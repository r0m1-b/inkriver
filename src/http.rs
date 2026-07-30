const SERVER_ERROR: &str = "Server error occurred";
const UNEXPECTED_ERROR: &str = "Unexpected error occurred";

fn validate_status(status: reqwest::StatusCode) -> Result<(), String> {
    if status.is_success() {
        Ok(())
    } else if status.is_server_error() {
        println!("server error!");
        Err(SERVER_ERROR.into())
    } else {
        println!("Something else happened. Status: {:?}", status);
        Err(UNEXPECTED_ERROR.into())
    }
}

pub fn check_feed_url(url: &str) -> Result<reqwest::blocking::Response, String> {
    let response = reqwest::blocking::get(url).map_err(|error| error.to_string())?;

    validate_status(response.status())?;

    Ok(response)
}

#[cfg(test)]
mod tests {

    use super::*;
    use reqwest::StatusCode;

    #[test]
    fn success_status_is_accepted() {
        let result = validate_status(StatusCode::OK);
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn server_error_is_rejected() {
        let result = validate_status(StatusCode::INTERNAL_SERVER_ERROR);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), SERVER_ERROR);
    }

    #[test]
    fn not_found_is_rejected_as_unexpected_status() {
        let result = validate_status(StatusCode::NOT_FOUND);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), UNEXPECTED_ERROR);
    }
}
