//! The Google Drive REST client: one shared HTTP client, typed responses,
//! classified errors, and retry with backoff for rate limits and server errors.
//! Higher-level operations (listing as entries, opening, thumbnails) live in
//! `service.rs`.

use std::io::Cursor;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use serde::de::DeserializeOwned;

pub const API_BASE: &str = "https://www.googleapis.com/drive/v3";

/// Attempts per request, counting the first. Only 429, 5xx and Drive's
/// rate-limit 403s are retried.
const ATTEMPTS: u32 = 3;
/// The longest `Retry-After` the client waits for before giving up.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(10);

/// The HTTP client every Drive request shares, so connections are reused.
/// Redirects are never followed: Drive API calls do not need them.
pub fn http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("Could not create the Google Drive HTTP client: {error}"))
}

// ---------------------------------------------------------------- errors

/// A failure from the Drive API, classified so the UI can offer the right next
/// step (sign in again, retry, or nothing).
#[derive(Debug)]
pub enum DriveError {
    Unauthorized,
    Forbidden(String),
    NotFound,
    RateLimited,
    Unavailable(String),
    Parse(String),
}

impl DriveError {
    pub fn message(&self) -> String {
        match self {
            DriveError::Unauthorized => "Google Drive sign-in expired. Sign in again.".into(),
            DriveError::Forbidden(message) if message.is_empty() => {
                "Google Drive refused the request.".into()
            }
            DriveError::Forbidden(message) => {
                format!("Google Drive refused the request: {message}")
            }
            DriveError::NotFound => "The Google Drive item no longer exists.".into(),
            DriveError::RateLimited => "Google Drive is temporarily unavailable. Try again.".into(),
            DriveError::Unavailable(message) => format!("Google Drive is unavailable: {message}"),
            DriveError::Parse(_) => "Invalid response from Google Drive.".into(),
        }
    }

    pub fn is_unauthorized(&self) -> bool {
        matches!(self, DriveError::Unauthorized)
    }

    fn is_retryable(&self) -> bool {
        matches!(self, DriveError::RateLimited | DriveError::Unavailable(_))
    }

    fn from_http(status: u16, body: &str) -> Self {
        let error = serde_json::from_str::<ErrorEnvelope>(body)
            .ok()
            .and_then(|envelope| envelope.error);
        // Drive reports quota exhaustion as a 403 with a rate-limit reason.
        let rate_limited = error.as_ref().is_some_and(|error| {
            error.errors.iter().any(|item| {
                matches!(
                    item.reason.as_deref(),
                    Some("rateLimitExceeded" | "userRateLimitExceeded")
                )
            })
        });
        let message = error
            .and_then(|error| error.message)
            .filter(|message| !message.is_empty());
        match status {
            401 => DriveError::Unauthorized,
            403 if rate_limited => DriveError::RateLimited,
            403 => DriveError::Forbidden(message.unwrap_or_default()),
            404 => DriveError::NotFound,
            429 => DriveError::RateLimited,
            500..=599 => {
                DriveError::Unavailable(message.unwrap_or_else(|| format!("server error {status}")))
            }
            _ => DriveError::Forbidden(
                message.unwrap_or_else(|| format!("request failed ({status})")),
            ),
        }
    }
}

impl std::fmt::Display for DriveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    error: Option<ApiError>,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    message: Option<String>,
    #[serde(default)]
    errors: Vec<ApiErrorItem>,
}

#[derive(Debug, Deserialize)]
struct ApiErrorItem {
    reason: Option<String>,
}

// ---------------------------------------------------------------- responses

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveFile {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub modified_time: Option<String>,
    #[serde(default)]
    pub web_view_link: Option<String>,
    #[serde(default)]
    pub thumbnail_link: Option<String>,
    #[serde(default)]
    pub capabilities: Option<Capabilities>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    #[serde(default)]
    pub can_download: bool,
}

impl DriveFile {
    pub fn is_dir(&self) -> bool {
        self.mime_type == "application/vnd.google-apps.folder"
    }

    pub fn is_shortcut(&self) -> bool {
        self.mime_type == "application/vnd.google-apps.shortcut"
    }

    /// Google Docs/Sheets/Slides/Drawings/Forms have no media bytes.
    pub fn is_google_doc(&self) -> bool {
        self.mime_type.starts_with("application/vnd.google-apps.")
            && !self.is_dir()
            && !self.is_shortcut()
    }

    pub fn can_download(&self) -> bool {
        !self.is_dir()
            && !self.is_google_doc()
            && self
                .capabilities
                .as_ref()
                .map(|capabilities| capabilities.can_download)
                .unwrap_or(true)
    }
}

#[derive(Debug)]
pub struct DrivePage {
    pub files: Vec<DriveFile>,
    pub next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListResponse {
    #[serde(default)]
    files: Vec<DriveFile>,
    #[serde(default)]
    next_page_token: Option<String>,
}

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
}

#[derive(Debug, Deserialize)]
struct AboutUser {
    #[serde(rename = "displayName", default)]
    display_name: Option<String>,
    #[serde(rename = "emailAddress", default)]
    email_address: Option<String>,
}

fn about_from(about: AboutResponse) -> Result<AboutInfo, String> {
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
    // Drive v3's `about` does not expose rootFolderId; "root" is the API's
    // stable alias for the My Drive folder.
    Ok(AboutInfo {
        email,
        name,
        root_id: "root".into(),
    })
}

// ---------------------------------------------------------------- client

pub struct DriveClient {
    http: reqwest::blocking::Client,
    base: String,
    token: String,
    /// The first retry waits about this long; later ones double it.
    backoff: Duration,
}

impl DriveClient {
    /// `base` is API_BASE in production; tests point it at a local server.
    pub fn new(http: reqwest::blocking::Client, base: &str, token: String) -> Self {
        Self {
            http,
            base: base.trim_end_matches('/').to_string(),
            token,
            backoff: Duration::from_millis(500),
        }
    }

    /// Fetch the account's email, display name and root folder id.
    pub fn about(&self) -> Result<AboutInfo, DriveError> {
        let about: AboutResponse = self.get_json(
            &format!("{}/about", self.base),
            &[("fields", "user(displayName,emailAddress)")],
        )?;
        about_from(about).map_err(DriveError::Parse)
    }

    pub fn list(&self, folder_id: &str, page: Option<&str>) -> Result<DrivePage, DriveError> {
        let query = format!("'{folder_id}' in parents and trashed = false");
        let fields = "nextPageToken,files(id,name,mimeType,size,modifiedTime,webViewLink,thumbnailLink,capabilities(canDownload))";
        let mut params: Vec<(&str, &str)> = vec![
            ("q", query.as_str()),
            ("pageSize", "1000"),
            ("orderBy", "name_natural"),
            ("fields", fields),
            ("supportsAllDrives", "true"),
            ("includeItemsFromAllDrives", "true"),
        ];
        if let Some(page) = page {
            params.push(("pageToken", page));
        }
        let response: ListResponse = self.get_json(&format!("{}/files", self.base), &params)?;
        Ok(DrivePage {
            files: response.files,
            next_page_token: response.next_page_token,
        })
    }

    pub fn get(&self, id: &str) -> Result<DriveFile, DriveError> {
        let fields = "id,name,mimeType,size,modifiedTime,webViewLink,thumbnailLink,capabilities(canDownload)";
        self.get_json(
            &format!("{}/files/{}", self.base, id),
            &[("fields", fields), ("supportsAllDrives", "true")],
        )
    }

    pub fn download(&self, id: &str, dest: &Path) -> Result<(), DriveError> {
        let url = format!("{}/files/{}", self.base, id);
        let mut response = self.send(|| {
            self.http
                .get(&url)
                .query(&[("alt", "media"), ("supportsAllDrives", "true")])
                .bearer_auth(&self.token)
        })?;
        let mut file = std::fs::File::create(dest)
            .map_err(|error| DriveError::Unavailable(error.to_string()))?;
        std::io::copy(&mut response, &mut file)
            .map_err(|error| DriveError::Unavailable(error.to_string()))?;
        Ok(())
    }

    /// Fetch Drive's thumbnail and normalize it to PNG, which is what the
    /// frontend's thumbnail pipeline expects. Any failure, including an
    /// expired link, is "no thumbnail", not an error the user needs to see.
    pub fn thumbnail_png(&self, link: &str, size: u32) -> Option<Vec<u8>> {
        let url = thumbnail_url(link, size);
        // thumbnailLink is pre-signed; do not attach the bearer token to a
        // Google content host.
        let response = self.send(|| self.http.get(&url)).ok()?;
        let bytes = response.bytes().ok()?;
        let image = image::load_from_memory(&bytes).ok()?;
        let mut out = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
            .ok()?;
        Some(out)
    }

    fn get_json<T: DeserializeOwned>(
        &self,
        url: &str,
        query: &[(&str, &str)],
    ) -> Result<T, DriveError> {
        let response = self.send(|| self.http.get(url).query(query).bearer_auth(&self.token))?;
        let body = response
            .text()
            .map_err(|error| DriveError::Parse(error.to_string()))?;
        serde_json::from_str(&body).map_err(|error| DriveError::Parse(error.to_string()))
    }

    /// Send a request, retrying rate limits and server errors with exponential
    /// backoff (or the server's `Retry-After`). Only a 2xx response is `Ok`.
    fn send(
        &self,
        request: impl Fn() -> reqwest::blocking::RequestBuilder,
    ) -> Result<reqwest::blocking::Response, DriveError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let (error, retry_after) = match request().send() {
                Ok(response) if response.status().is_success() => return Ok(response),
                Ok(response) => {
                    let status = response.status().as_u16();
                    let retry_after = response
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_string);
                    let body = response.text().unwrap_or_default();
                    (DriveError::from_http(status, &body), retry_after)
                }
                Err(error) => (DriveError::Unavailable(error.to_string()), None),
            };
            if attempt >= ATTEMPTS || !error.is_retryable() {
                return Err(error);
            }
            match retry_delay(attempt, retry_after.as_deref(), self.backoff) {
                Some(delay) => std::thread::sleep(delay),
                None => return Err(error),
            }
        }
    }
}

/// How long to wait before retry number `attempt` (1-based): the server's
/// `Retry-After` seconds when given, else `backoff * 2^(attempt-1)` plus up to
/// 50% jitter. `None` when the server asks for a longer wait than we accept.
fn retry_delay(attempt: u32, retry_after: Option<&str>, backoff: Duration) -> Option<Duration> {
    if let Some(seconds) = retry_after.and_then(|value| value.trim().parse::<u64>().ok()) {
        let delay = Duration::from_secs(seconds);
        return (delay <= MAX_RETRY_AFTER).then_some(delay);
    }
    let base = backoff * 2u32.pow(attempt.saturating_sub(1).min(8));
    let jitter = getrandom::u32().unwrap_or(0) % 1000;
    Some(base + base * jitter / 2000)
}

/// Drive thumbnail links end in a `=s<size>` suffix; ask for the size we need.
pub fn thumbnail_url(link: &str, size: u32) -> String {
    let base = link.split("=s").next().unwrap_or(link);
    format!("{base}=s{}", size.clamp(32, 1024))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::test_server::{json, serve};

    fn client(base: &str) -> DriveClient {
        let mut client = DriveClient::new(http_client().unwrap(), base, "token".into());
        client.backoff = Duration::from_millis(1);
        client
    }

    fn about(json: &str) -> Result<AboutInfo, String> {
        about_from(serde_json::from_str(json).unwrap())
    }

    #[test]
    fn about_parses_the_user_and_root() {
        let info = about(
            r#"{
                "user": {"displayName": "Ahmed", "emailAddress": "me@example.com"}
            }"#,
        )
        .unwrap();
        assert_eq!(
            info,
            AboutInfo {
                email: "me@example.com".into(),
                name: "Ahmed".into(),
                root_id: "root".into(),
            }
        );
    }

    #[test]
    fn about_falls_back_to_the_email_local_part() {
        let info = about(r#"{"user": {"emailAddress": "me@example.com"}}"#).unwrap();
        assert_eq!(info.name, "me");
    }

    #[test]
    fn about_rejects_missing_pieces() {
        assert!(about("{}").is_err());
        assert!(about(r#"{"user": {}}"#).is_err());
    }

    #[test]
    fn about_is_fetched_with_the_client() {
        let (base, server) = serve(vec![(
            200,
            json(r#"{"user":{"displayName":"Me","emailAddress":"me@example.com"}}"#),
            "application/json",
        )]);
        let info = client(&base).about().unwrap();
        assert_eq!(info.email, "me@example.com");
        server.join().unwrap();
    }

    #[test]
    fn http_errors_are_classified() {
        assert!(matches!(
            DriveError::from_http(401, ""),
            DriveError::Unauthorized
        ));
        assert!(matches!(
            DriveError::from_http(404, ""),
            DriveError::NotFound
        ));
        assert!(matches!(
            DriveError::from_http(429, ""),
            DriveError::RateLimited
        ));
        assert!(matches!(
            DriveError::from_http(503, ""),
            DriveError::Unavailable(_)
        ));
        let forbidden = DriveError::from_http(403, r#"{"error":{"message":"nope"}}"#);
        assert!(forbidden.message().contains("nope"));
        assert!(!forbidden.is_retryable());
    }

    #[test]
    fn a_rate_limit_403_is_rate_limited() {
        let body = r#"{"error":{"code":403,"message":"slow down","errors":[{"reason":"userRateLimitExceeded"}]}}"#;
        assert!(matches!(
            DriveError::from_http(403, body),
            DriveError::RateLimited
        ));
    }

    #[test]
    fn retry_delays_grow_and_honour_retry_after() {
        let backoff = Duration::from_millis(100);
        let first = retry_delay(1, None, backoff).unwrap();
        let second = retry_delay(2, None, backoff).unwrap();
        assert!(first >= backoff && first <= backoff * 3 / 2);
        assert!(second >= backoff * 2 && second <= backoff * 3);
        assert_eq!(
            retry_delay(1, Some("2"), backoff),
            Some(Duration::from_secs(2))
        );
        assert_eq!(retry_delay(1, Some("3600"), backoff), None);
    }

    #[test]
    fn rate_limits_and_server_errors_are_retried() {
        let (base, server) = serve(vec![
            (429, json("{}"), "application/json"),
            (503, json("{}"), "application/json"),
            (200, json(r#"{"files":[]}"#), "application/json"),
        ]);
        let page = client(&base).list("root", None).unwrap();
        assert!(page.files.is_empty());
        server.join().unwrap();
    }

    #[test]
    fn retries_give_up_after_three_attempts() {
        let (base, server) = serve(vec![
            (503, json("{}"), "application/json"),
            (503, json("{}"), "application/json"),
            (503, json("{}"), "application/json"),
        ]);
        let error = client(&base).list("root", None).unwrap_err();
        assert!(matches!(error, DriveError::Unavailable(_)));
        server.join().unwrap();
    }

    #[test]
    fn a_404_is_not_retried() {
        let (base, server) = serve(vec![(404, json("{}"), "application/json")]);
        let error = client(&base).get("gone").unwrap_err();
        assert!(matches!(error, DriveError::NotFound));
        server.join().unwrap();
    }

    #[test]
    fn listing_follows_pages() {
        let (base, server) = serve(vec![
            (
                200,
                json(
                    r#"{"files":[{"id":"f1","name":"a.txt","mimeType":"text/plain","size":"12","modifiedTime":"2026-10-24T17:52:03Z","capabilities":{"canDownload":true}}],"nextPageToken":"p2"}"#,
                ),
                "application/json",
            ),
            (
                200,
                json(
                    r#"{"files":[{"id":"f2","name":"b.txt","mimeType":"text/plain","size":"34"}]}"#,
                ),
                "application/json",
            ),
        ]);
        let client = client(&base);
        let first = client.list("root", None).unwrap();
        assert_eq!(first.files.len(), 1);
        assert_eq!(first.next_page_token.as_deref(), Some("p2"));
        let second = client
            .list("root", first.next_page_token.as_deref())
            .unwrap();
        assert_eq!(second.files[0].name, "b.txt");
        assert!(second.next_page_token.is_none());
        server.join().unwrap();
    }

    #[test]
    fn a_download_writes_the_media_bytes() {
        let (base, server) = serve(vec![(
            200,
            b"hello drive".to_vec(),
            "application/octet-stream",
        )]);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        client(&base).download("f1", &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello drive");
        server.join().unwrap();
    }

    #[test]
    fn a_401_is_unauthorized() {
        let (base, server) = serve(vec![(
            401,
            json(r#"{"error":{"message":"invalid credentials"}}"#),
            "application/json",
        )]);
        let error = client(&base).list("root", None).unwrap_err();
        assert!(error.is_unauthorized());
        server.join().unwrap();
    }

    #[test]
    fn thumbnails_are_normalized_to_png() {
        let mut png = Vec::new();
        image::DynamicImage::new_rgb8(1, 1)
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let (base, server) = serve(vec![(200, png, "image/png")]);
        let bytes = client(&base)
            .thumbnail_png(&format!("{base}/thumb"), 256)
            .unwrap();
        assert!(bytes.starts_with(b"\x89PNG"));
        server.join().unwrap();
    }

    #[test]
    fn a_missing_thumbnail_is_none() {
        let (base, server) = serve(vec![(404, json("{}"), "application/json")]);
        assert!(
            client(&base)
                .thumbnail_png(&format!("{base}/thumb"), 256)
                .is_none()
        );
        server.join().unwrap();
    }

    #[test]
    fn thumbnails_get_one_size_suffix() {
        assert_eq!(thumbnail_url("https://x/y=s220", 256), "https://x/y=s256");
        assert_eq!(thumbnail_url("https://x/y", 2000), "https://x/y=s1024");
    }
}
