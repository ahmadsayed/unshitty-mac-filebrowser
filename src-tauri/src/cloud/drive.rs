//! The Google Drive REST client: one shared HTTP client, typed responses,
//! classified errors, and retry with backoff for rate limits and server errors.
//! Higher-level operations (listing as entries, opening, thumbnails) live in
//! `service.rs`.

use std::io::Cursor;
use std::path::Path;
use std::time::Duration;

use crate::progress::{CANCELLED, Progress, copy_counted, is_cancel};

use serde::Deserialize;
use serde::de::DeserializeOwned;

pub const API_BASE: &str = "https://www.googleapis.com/drive/v3";
pub const UPLOAD_BASE: &str = "https://www.googleapis.com/upload/drive/v3";

/// Upload chunk size: Drive wants a multiple of 256 KiB.
pub const CHUNK: usize = 8 * 1024 * 1024;

/// The metadata every single-file request asks for.
const FILE_FIELDS: &str = "id,name,mimeType,size,modifiedTime,webViewLink,thumbnailLink,capabilities(canDownload),trashed";

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
    /// The operation was cancelled (spec: operation progress D2).
    Cancelled,
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
            DriveError::Cancelled => CANCELLED.into(),
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
    #[serde(default)]
    pub trashed: bool,
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

    /// Has bytes to download: not a folder, Google Doc or shortcut, and the
    /// owner allows downloads.
    pub fn can_download(&self) -> bool {
        !self.is_dir()
            && !self.is_google_doc()
            && !self.is_shortcut()
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

/// A metadata change for `DriveClient::update`. Parents travel in the query,
/// the rest in the JSON body; `None` fields are left alone.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct FileChange {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trashed: Option<bool>,
    #[serde(skip)]
    pub add_parent: Option<String>,
    #[serde(skip)]
    pub remove_parent: Option<String>,
}

impl FileChange {
    pub fn trash(trashed: bool) -> Self {
        Self {
            trashed: Some(trashed),
            ..Self::default()
        }
    }

    pub fn rename(name: &str) -> Self {
        Self {
            name: Some(name.to_string()),
            ..Self::default()
        }
    }
}

/// Fetches a new access token; see `DriveClient::with_refresher`.
type Refresher<'a> = Box<dyn Fn() -> Result<String, String> + 'a>;

pub struct DriveClient<'a> {
    http: reqwest::blocking::Client,
    base: String,
    upload_base: String,
    token: std::cell::RefCell<String>,
    refresher: Option<Refresher<'a>>,
    /// The first retry waits about this long; later ones double it.
    backoff: Duration,
}

impl<'a> DriveClient<'a> {
    /// `base` is API_BASE in production; tests point it at a local server.
    pub fn new(http: reqwest::blocking::Client, base: &str, token: String) -> Self {
        let base = base.trim_end_matches('/').to_string();
        let upload_base = if base == API_BASE {
            UPLOAD_BASE.to_string()
        } else {
            format!("{base}/upload")
        };
        Self {
            http,
            base,
            upload_base,
            token: std::cell::RefCell::new(token),
            refresher: None,
            backoff: Duration::from_millis(500),
        }
    }

    /// Refresh the token when a request comes back 401 and send that one
    /// request again. Multi-step writes outlive an access token (about an
    /// hour); resending the request that was refused repeats nothing.
    pub fn with_refresher(mut self, refresh: impl Fn() -> Result<String, String> + 'a) -> Self {
        self.refresher = Some(Box::new(refresh));
        self
    }

    fn token(&self) -> String {
        self.token.borrow().clone()
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
        let fields = "nextPageToken,files(id,name,mimeType,size,modifiedTime,webViewLink,thumbnailLink,capabilities(canDownload),trashed)";
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

    /// The untrashed children of `parent_id` named exactly `name`. Drive
    /// allows several, so this is a list.
    pub fn find_children(&self, parent_id: &str, name: &str) -> Result<Vec<DriveFile>, DriveError> {
        let query = format!(
            "name = '{}' and '{}' in parents and trashed = false",
            escape_query(name),
            escape_query(parent_id)
        );
        let fields = format!("files({FILE_FIELDS})");
        let response: ListResponse = self.get_json(
            &format!("{}/files", self.base),
            &[
                ("q", query.as_str()),
                ("fields", fields.as_str()),
                ("supportsAllDrives", "true"),
                ("includeItemsFromAllDrives", "true"),
            ],
        )?;
        Ok(response.files)
    }

    pub fn get(&self, id: &str) -> Result<DriveFile, DriveError> {
        let fields = FILE_FIELDS;
        self.get_json(
            &format!("{}/files/{}", self.base, id),
            &[("fields", fields), ("supportsAllDrives", "true")],
        )
    }

    /// Stream a file's bytes to `dest`, counting them and stopping between
    /// slices when `progress` is cancelled.
    pub fn download(&self, id: &str, dest: &Path, progress: &Progress) -> Result<(), DriveError> {
        let url = format!("{}/files/{}", self.base, id);
        let mut response = self.send(|| {
            self.http
                .get(&url)
                .query(&[("alt", "media"), ("supportsAllDrives", "true")])
                .bearer_auth(self.token())
        })?;
        let mut file = std::fs::File::create(dest)
            .map_err(|error| DriveError::Unavailable(error.to_string()))?;
        copy_counted(&mut response, &mut file, progress).map_err(|error| {
            if is_cancel(&error) {
                DriveError::Cancelled
            } else {
                DriveError::Unavailable(error.to_string())
            }
        })?;
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

    /// Every page of a folder's untrashed children.
    pub fn list_all(&self, folder_id: &str) -> Result<Vec<DriveFile>, DriveError> {
        let mut files = Vec::new();
        let mut page: Option<String> = None;
        loop {
            let response = self.list(folder_id, page.as_deref())?;
            files.extend(response.files);
            match response.next_page_token {
                Some(token) => page = Some(token),
                None => return Ok(files),
            }
        }
    }

    pub fn create_folder(&self, parent_id: &str, name: &str) -> Result<DriveFile, DriveError> {
        let body = serde_json::json!({
            "name": name,
            "mimeType": "application/vnd.google-apps.folder",
            "parents": [parent_id],
        });
        self.json_request(
            reqwest::Method::POST,
            &format!("{}/files", self.base),
            &[],
            &body,
        )
    }

    pub fn update(&self, id: &str, change: &FileChange) -> Result<DriveFile, DriveError> {
        let mut query = Vec::new();
        if let Some(parent) = &change.add_parent {
            query.push(("addParents", parent.as_str()));
        }
        if let Some(parent) = &change.remove_parent {
            query.push(("removeParents", parent.as_str()));
        }
        let body = serde_json::to_value(change).map_err(|e| DriveError::Parse(e.to_string()))?;
        self.json_request(
            reqwest::Method::PATCH,
            &format!("{}/files/{}", self.base, id),
            &query,
            &body,
        )
    }

    /// A server-side copy of a file (Drive cannot copy folders).
    pub fn copy_file(
        &self,
        id: &str,
        parent_id: &str,
        name: &str,
    ) -> Result<DriveFile, DriveError> {
        let body = serde_json::json!({ "name": name, "parents": [parent_id] });
        self.json_request(
            reqwest::Method::POST,
            &format!("{}/files/{}/copy", self.base, id),
            &[],
            &body,
        )
    }

    /// Upload a local file with Drive's resumable protocol, in `CHUNK`s.
    /// Drive creates the file only when the last chunk arrives, so a failed
    /// upload leaves nothing behind.
    pub fn upload(
        &self,
        parent_id: &str,
        name: &str,
        local: &Path,
        progress: &Progress,
    ) -> Result<DriveFile, DriveError> {
        let io = |error: std::io::Error| DriveError::Unavailable(error.to_string());
        let size = std::fs::metadata(local).map_err(io)?.len();
        let metadata = serde_json::json!({ "name": name, "parents": [parent_id] });
        let url = format!("{}/files", self.upload_base);
        let started = self.send(|| {
            self.http
                .post(&url)
                .query(&[
                    ("uploadType", "resumable"),
                    ("supportsAllDrives", "true"),
                    ("fields", FILE_FIELDS),
                ])
                .bearer_auth(self.token())
                .header("X-Upload-Content-Length", size)
                .json(&metadata)
        })?;
        let session = started
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| DriveError::Parse("Drive did not start an upload session".into()))?
            .to_string();

        let mut file = std::fs::File::open(local).map_err(io)?;
        let mut offset = 0u64;
        let mut stalls = 0;
        loop {
            // A cancel abandons the session; Drive creates nothing.
            if progress.is_cancelled() {
                return Err(DriveError::Cancelled);
            }
            let end = (offset + CHUNK as u64).min(size);
            let chunk = read_chunk(&mut file, offset, (end - offset) as usize).map_err(io)?;
            let range = if size == 0 {
                "bytes */0".to_string()
            } else {
                format!("bytes {offset}-{}/{size}", end - 1)
            };
            let response = self.send_accepting(
                || {
                    self.http
                        .put(&session)
                        .header(reqwest::header::CONTENT_RANGE, &range)
                        .body(chunk.clone())
                },
                |status| status == 308,
            )?;
            if response.status().as_u16() != 308 {
                progress.add_bytes(size.saturating_sub(offset));
                let body = response
                    .text()
                    .map_err(|error| DriveError::Parse(error.to_string()))?;
                return serde_json::from_str(&body)
                    .map_err(|error| DriveError::Parse(error.to_string()));
            }
            let next = response
                .headers()
                .get(reqwest::header::RANGE)
                .and_then(|value| value.to_str().ok())
                .and_then(next_offset)
                .unwrap_or(0);
            // Progress or give up: a server that never accepts bytes must not
            // keep us looping.
            stalls = if next > offset { 0 } else { stalls + 1 };
            progress.add_bytes(next.saturating_sub(offset));
            if stalls >= ATTEMPTS {
                return Err(DriveError::Unavailable(
                    "the upload made no progress".into(),
                ));
            }
            offset = next;
        }
    }

    fn json_request<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        url: &str,
        query: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<T, DriveError> {
        let response = self.send(|| {
            self.http
                .request(method.clone(), url)
                .query(query)
                .query(&[("supportsAllDrives", "true"), ("fields", FILE_FIELDS)])
                .bearer_auth(self.token())
                .json(body)
        })?;
        let text = response
            .text()
            .map_err(|error| DriveError::Parse(error.to_string()))?;
        serde_json::from_str(&text).map_err(|error| DriveError::Parse(error.to_string()))
    }

    fn get_json<T: DeserializeOwned>(
        &self,
        url: &str,
        query: &[(&str, &str)],
    ) -> Result<T, DriveError> {
        let response = self.send(|| self.http.get(url).query(query).bearer_auth(self.token()))?;
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
        self.send_accepting(request, |_| false)
    }

    /// `send`, also accepting the statuses `accept` approves (an upload
    /// chunk's `308 Resume Incomplete`).
    fn send_accepting(
        &self,
        request: impl Fn() -> reqwest::blocking::RequestBuilder,
        accept: impl Fn(u16) -> bool,
    ) -> Result<reqwest::blocking::Response, DriveError> {
        let mut attempt = 0;
        let mut refreshed = false;
        loop {
            attempt += 1;
            let (error, retry_after) = match request().send() {
                Ok(response)
                    if response.status().is_success() || accept(response.status().as_u16()) =>
                {
                    return Ok(response);
                }
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
            if error.is_unauthorized() && !refreshed {
                if let Some(refresh) = &self.refresher {
                    refreshed = true;
                    attempt -= 1;
                    *self.token.borrow_mut() = refresh().map_err(|_| DriveError::Unauthorized)?;
                    continue;
                }
            }
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

/// Read up to `length` bytes at `offset`.
fn read_chunk(file: &mut std::fs::File, offset: u64, length: usize) -> std::io::Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    file.seek(SeekFrom::Start(offset))?;
    let mut chunk = Vec::with_capacity(length);
    file.take(length as u64).read_to_end(&mut chunk)?;
    Ok(chunk)
}

/// The next byte to send after a `308`'s `Range: bytes=0-N` (that is, N+1).
fn next_offset(range: &str) -> Option<u64> {
    let last = range.trim().strip_prefix("bytes=")?.split('-').nth(1)?;
    last.trim().parse::<u64>().ok().map(|last| last + 1)
}

/// Quote a value for a Drive search query (`'…'`): backslash and quote are
/// escaped with a backslash.
fn escape_query(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

/// Drive thumbnail links end in a `=s<size>` suffix; ask for the size we need.
pub fn thumbnail_url(link: &str, size: u32) -> String {
    let base = link.split("=s").next().unwrap_or(link);
    format!("{base}=s{}", size.clamp(32, 1024))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::Progress;
    use crate::cloud::test_server::{json, serve};

    fn client(base: &str) -> DriveClient<'_> {
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
        client(&base).download("f1", &dest, &Progress::default()).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello drive");
        server.join().unwrap();
    }

    #[test]
    fn a_401_refreshes_the_token_once_and_resends_that_request() {
        let (base, log, server) = serve_replies(vec![
            Reply::json(401, r#"{"error":{"message":"expired"}}"#),
            Reply::json(200, r#"{"files":[]}"#),
        ]);
        let calls = std::cell::Cell::new(0);
        let refresh = || {
            calls.set(calls.get() + 1);
            Ok("fresh".to_string())
        };
        let client = client(&base).with_refresher(&refresh);
        client.list("root", None).unwrap();
        server.join().unwrap();
        assert_eq!(calls.get(), 1);
        let log = log.lock().unwrap();
        assert!(
            log[0].head.contains("authorization: Bearer token"),
            "{}",
            log[0].head
        );
        assert!(
            log[1].head.contains("authorization: Bearer fresh"),
            "{}",
            log[1].head
        );
    }

    #[test]
    fn a_second_401_after_refreshing_is_unauthorized() {
        let (base, server) = serve(vec![
            (401, json("{}"), "application/json"),
            (401, json("{}"), "application/json"),
        ]);
        let refresh = || Ok("fresh".to_string());
        let error = client(&base)
            .with_refresher(&refresh)
            .list("root", None)
            .unwrap_err();
        server.join().unwrap();
        assert!(error.is_unauthorized());
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

    use crate::cloud::test_server::{Reply, serve_replies};

    const FILE: &str = r#"{"id":"new","name":"n","mimeType":"text/plain","size":"5"}"#;

    #[test]
    fn create_folder_posts_the_folder_mime_type_and_parent() {
        let (base, log, server) = serve_replies(vec![Reply::json(200, FILE)]);
        let file = client(&base).create_folder("p1", "Docs").unwrap();
        server.join().unwrap();
        assert_eq!(file.id, "new");
        let request = log.lock().unwrap()[0].clone();
        assert!(request.head.starts_with("POST /files?"), "{}", request.head);
        let body = request.body_text();
        assert!(
            body.contains("application/vnd.google-apps.folder"),
            "{body}"
        );
        assert!(body.contains(r#""parents":["p1"]"#), "{body}");
        assert!(body.contains(r#""name":"Docs""#), "{body}");
    }

    #[test]
    fn update_moves_with_add_and_remove_parents() {
        let (base, log, server) = serve_replies(vec![Reply::json(200, FILE)]);
        let change = FileChange {
            add_parent: Some("p2".into()),
            remove_parent: Some("p1".into()),
            ..FileChange::default()
        };
        client(&base).update("f1", &change).unwrap();
        server.join().unwrap();
        let head = log.lock().unwrap()[0].head.clone();
        assert!(head.starts_with("PATCH /files/f1?"), "{head}");
        assert!(head.contains("addParents=p2"), "{head}");
        assert!(head.contains("removeParents=p1"), "{head}");
    }

    #[test]
    fn update_trashes_and_untrashes() {
        let (base, log, server) =
            serve_replies(vec![Reply::json(200, FILE), Reply::json(200, FILE)]);
        let client = client(&base);
        client.update("f1", &FileChange::trash(true)).unwrap();
        client.update("f1", &FileChange::trash(false)).unwrap();
        server.join().unwrap();
        let log = log.lock().unwrap();
        assert_eq!(log[0].body_text(), r#"{"trashed":true}"#);
        assert_eq!(log[1].body_text(), r#"{"trashed":false}"#);
    }

    #[test]
    fn copy_file_names_the_copy_and_its_parent() {
        let (base, log, server) = serve_replies(vec![Reply::json(200, FILE)]);
        client(&base).copy_file("f1", "p2", "a (copy).txt").unwrap();
        server.join().unwrap();
        let request = log.lock().unwrap()[0].clone();
        assert!(
            request.head.starts_with("POST /files/f1/copy?"),
            "{}",
            request.head
        );
        assert!(request.body_text().contains(r#""name":"a (copy).txt""#));
        assert!(request.body_text().contains(r#""parents":["p2"]"#));
    }

    fn local_file(bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("up.bin");
        std::fs::write(&path, bytes).unwrap();
        (dir, path)
    }

    #[test]
    fn upload_starts_a_session_and_sends_the_bytes() {
        let (base, log, server) = serve_replies(vec![
            Reply::json(200, "").header("Location", "{base}/session/1"),
            Reply::json(200, FILE),
        ]);
        let (_dir, path) = local_file(b"hello");
        let file = client(&base).upload("p1", "up.bin", &path, &Progress::default()).unwrap();
        server.join().unwrap();
        assert_eq!(file.id, "new");
        let log = log.lock().unwrap();
        assert!(
            log[0].head.starts_with("POST /upload/files?"),
            "{}",
            log[0].head
        );
        assert!(log[0].head.contains("uploadType=resumable"));
        assert!(log[0].body_text().contains(r#""parents":["p1"]"#));
        assert!(log[1].head.starts_with("PUT /session/1"), "{}", log[1].head);
        assert!(
            log[1].head.contains("content-range: bytes 0-4/5"),
            "{}",
            log[1].head
        );
        assert_eq!(log[1].body, b"hello");
    }

    #[test]
    fn upload_resumes_after_a_308() {
        let size = CHUNK + 10;
        let bytes: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        let (base, log, server) = serve_replies(vec![
            Reply::json(200, "").header("Location", "{base}/session/1"),
            Reply::json(308, "").header("Range", "bytes=0-262143"),
            Reply::json(200, FILE),
        ]);
        let (_dir, path) = local_file(&bytes);
        client(&base).upload("p1", "up.bin", &path, &Progress::default()).unwrap();
        server.join().unwrap();
        let log = log.lock().unwrap();
        assert!(
            log[1]
                .head
                .contains(&format!("content-range: bytes 0-{}/{size}", CHUNK - 1))
        );
        assert!(
            log[2]
                .head
                .contains(&format!("content-range: bytes 262144-{}/{size}", size - 1)),
            "{}",
            log[2].head
        );
        assert_eq!(log[2].body, bytes[262144..]);
    }

    #[test]
    fn upload_of_an_empty_file_sends_no_chunk_range() {
        let (base, log, server) = serve_replies(vec![
            Reply::json(200, "").header("Location", "{base}/session/1"),
            Reply::json(200, FILE),
        ]);
        let (_dir, path) = local_file(b"");
        client(&base).upload("p1", "empty", &path, &Progress::default()).unwrap();
        server.join().unwrap();
        assert!(
            log.lock().unwrap()[1]
                .head
                .contains("content-range: bytes */0")
        );
    }

    #[test]
    fn a_failed_upload_session_is_an_error() {
        let (base, server) = {
            let (base, _log, server) = serve_replies(vec![
                Reply::json(200, "").header("Location", "{base}/session/1"),
                Reply::json(400, r#"{"error":{"message":"bad"}}"#),
            ]);
            (base, server)
        };
        let (_dir, path) = local_file(b"hello");
        assert!(client(&base).upload("p1", "up.bin", &path, &Progress::default()).is_err());
        server.join().unwrap();
    }

    #[test]
    fn list_all_follows_every_page() {
        let (base, server) = serve(vec![
            (
                200,
                json(r#"{"files":[{"id":"a","name":"a","mimeType":"x"}],"nextPageToken":"2"}"#),
                "application/json",
            ),
            (
                200,
                json(r#"{"files":[{"id":"b","name":"b","mimeType":"x"}]}"#),
                "application/json",
            ),
        ]);
        let files = client(&base).list_all("p1").unwrap();
        server.join().unwrap();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn thumbnails_get_one_size_suffix() {
        assert_eq!(thumbnail_url("https://x/y=s220", 256), "https://x/y=s256");
        assert_eq!(thumbnail_url("https://x/y", 2000), "https://x/y=s1024");
    }

    #[test]
    fn download_counts_bytes() {
        let body = vec![5u8; 2 * 1024 * 1024 + 3];
        let (base, server) = serve(vec![(200, body.clone(), "application/octet-stream")]);
        let dir = tempfile::tempdir().unwrap();
        let progress = Progress::default();
        client(&base)
            .download("f1", &dir.path().join("out.bin"), &progress)
            .unwrap();
        server.join().unwrap();
        assert_eq!(progress.snapshot().bytes, body.len() as u64);
    }

    #[test]
    fn upload_counts_chunks() {
        let size = CHUNK + 10;
        let (base, _log, server) = serve_replies(vec![
            Reply::json(200, "").header("Location", "{base}/session/1"),
            Reply::json(308, "").header("Range", "bytes=0-262143"),
            Reply::json(200, FILE),
        ]);
        let (_dir, path) = local_file(&vec![1u8; size]);
        let progress = Progress::default();
        client(&base).upload("p1", "up.bin", &path, &progress).unwrap();
        server.join().unwrap();
        assert_eq!(progress.snapshot().bytes, size as u64);
    }
}
