//! The Google Drive REST client. This file starts with the one call the
//! sign-in flow needs (`about`); the listing and file operations land next.

use std::time::Duration;

use serde::Deserialize;

pub const API_BASE: &str = "https://www.googleapis.com/drive/v3";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AboutInfo {
    pub email: String,
    pub name: String,
    pub root_id: String,
}

#[derive(Debug, Deserialize)]
struct AboutResponse {
    #[serde(default)]
    user: Option<AboutUser>,
    #[serde(rename = "rootFolderId", default)]
    root_folder_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AboutUser {
    #[serde(rename = "displayName", default)]
    display_name: Option<String>,
    #[serde(rename = "emailAddress", default)]
    email_address: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    error: Option<ApiError>,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    message: Option<String>,
}

/// Fetch the account's email, display name and root folder id.
pub fn about(access_token: &str) -> Result<AboutInfo, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("Could not create the Google Drive client: {error}"))?;
    let response = client
        .get(format!("{API_BASE}/about"))
        .query(&[("fields", "user(displayName,emailAddress),rootFolderId")])
        .bearer_auth(access_token)
        .send()
        .map_err(|error| format!("Google Drive is unavailable: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .map_err(|error| format!("Could not read the Google Drive response: {error}"))?;
    if !status.is_success() {
        return Err(http_error(status.as_u16(), &body));
    }
    parse_about(&body)
}

pub fn parse_about(json: &str) -> Result<AboutInfo, String> {
    let about: AboutResponse = serde_json::from_str(json)
        .map_err(|error| format!("Invalid Google Drive response: {error}"))?;
    let user = about
        .user
        .ok_or("Google did not return the account information.")?;
    let email = user
        .email_address
        .filter(|email| !email.is_empty())
        .ok_or("Google did not return an email address for this account.")?;
    let name = user
        .display_name
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| {
            email
                .split('@')
                .next()
                .unwrap_or("Google Drive")
                .to_string()
        });
    let root_id = about
        .root_folder_id
        .filter(|id| !id.is_empty())
        .ok_or("Google did not return the Drive root folder.")?;
    Ok(AboutInfo {
        email,
        name,
        root_id,
    })
}

fn http_error(status: u16, body: &str) -> String {
    let message = serde_json::from_str::<ErrorEnvelope>(body)
        .ok()
        .and_then(|envelope| envelope.error)
        .and_then(|error| error.message)
        .filter(|message| !message.is_empty());

    match status {
        401 => "Google Drive sign-in expired. Sign in again.".into(),
        403 => match message {
            Some(message) => format!("Google Drive refused the request: {message}"),
            None => "Google Drive refused the request.".into(),
        },
        404 => "The Google Drive item no longer exists.".into(),
        _ => match message {
            Some(message) => format!("Google Drive request failed ({status}): {message}"),
            None => format!("Google Drive request failed ({status})."),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn about_parses_the_user_and_root() {
        let info = parse_about(
            r#"{
                "user": {"displayName": "Ahmed", "emailAddress": "me@example.com"},
                "rootFolderId": "root-123"
            }"#,
        )
        .unwrap();
        assert_eq!(
            info,
            AboutInfo {
                email: "me@example.com".into(),
                name: "Ahmed".into(),
                root_id: "root-123".into(),
            }
        );
    }

    #[test]
    fn about_falls_back_to_the_email_local_part() {
        let info =
            parse_about(r#"{"user": {"emailAddress": "me@example.com"}, "rootFolderId": "root"}"#)
                .unwrap();
        assert_eq!(info.name, "me");
    }

    #[test]
    fn about_rejects_missing_pieces() {
        assert!(parse_about("{}").is_err());
        assert!(parse_about(r#"{"user": {}, "rootFolderId": "root"}"#).is_err());
        assert!(parse_about(r#"{"user": {"emailAddress": "a@b"}, "rootFolderId": ""}"#).is_err());
    }

    #[test]
    fn errors_map_to_actionable_messages() {
        assert!(http_error(401, "").contains("Sign in again"));
        assert!(
            http_error(403, r#"{"error":{"message":"No permission"}}"#).contains("No permission")
        );
        assert!(http_error(500, "").contains("500"));
    }
}
