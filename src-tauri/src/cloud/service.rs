//! Read-only Google Drive operations built on the REST client: listing,
//! metadata, downloading a local snapshot and thumbnails. Everything that
//! touches the network runs on a blocking thread; the frontend simply sees
//! `Entry` values with a `cloud` field.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::DateTime;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::cloud::CloudEntry;
use crate::cloud::CloudState;
use crate::cloud::accounts::DriveAccount;
use crate::cloud::location::{DriveLocation, Location, Segment};
use crate::cloud::oauth::TokenExchange;
use crate::cloud::token::Token;
use crate::fs_ops::{Entry, PathInfo};
use crate::properties::FileProperties;

pub const API_BASE: &str = crate::cloud::drive::API_BASE;
/// Stop after this many children; a Drive listing is not streamed page by page
/// in the UI yet. Reported in the README as the large-folder limit.
pub const LIST_LIMIT: usize = 10_000;

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
}

impl std::fmt::Display for DriveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

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

#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    error: Option<ApiError>,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    message: Option<String>,
}

pub struct DriveClient {
    http: reqwest::blocking::Client,
    base: String,
    token: String,
}

impl DriveClient {
    pub fn new(token: String) -> Result<Self, String> {
        Self::with_base(API_BASE, token)
    }

    /// Tests point this at a local server; production always uses API_BASE.
    pub fn with_base(base: &str, token: String) -> Result<Self, String> {
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| format!("Could not create the Google Drive HTTP client: {error}"))?;
        Ok(Self {
            http,
            base: base.trim_end_matches('/').to_string(),
            token,
        })
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
        let mut response = self
            .http
            .get(format!("{}/files/{}", self.base, id))
            .query(&[("alt", "media"), ("supportsAllDrives", "true")])
            .bearer_auth(&self.token)
            .send()
            .map_err(|error| DriveError::Unavailable(error.to_string()))?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            let body = response.text().unwrap_or_default();
            return Err(DriveError::from_http(status, &body));
        }
        let mut file = std::fs::File::create(dest)
            .map_err(|error| DriveError::Unavailable(error.to_string()))?;
        std::io::copy(&mut response, &mut file)
            .map_err(|error| DriveError::Unavailable(error.to_string()))?;
        Ok(())
    }

    /// Fetch Drive's thumbnail and normalize it to PNG, which is what the
    /// frontend's thumbnail pipeline expects. A decode failure is "no
    /// thumbnail", not an error the user needs to see.
    pub fn thumbnail_png(&self, link: &str, size: u32) -> Result<Option<Vec<u8>>, DriveError> {
        // thumbnailLink is pre-signed; do not attach the bearer token to a
        // Google content host.
        let response = self
            .http
            .get(thumbnail_url(link, size))
            .send()
            .map_err(|error| DriveError::Unavailable(error.to_string()))?;
        if !response.status().is_success() {
            return Ok(None);
        }
        let bytes = response
            .bytes()
            .map_err(|error| DriveError::Unavailable(error.to_string()))?;
        let Ok(image) = image::load_from_memory(&bytes) else {
            return Ok(None);
        };
        let mut out = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
            .map_err(|error| DriveError::Parse(error.to_string()))?;
        Ok(Some(out))
    }

    fn get_json<T: DeserializeOwned>(
        &self,
        url: &str,
        query: &[(&str, &str)],
    ) -> Result<T, DriveError> {
        let response = self
            .http
            .get(url)
            .query(query)
            .bearer_auth(&self.token)
            .send()
            .map_err(|error| DriveError::Unavailable(error.to_string()))?;
        let status = response.status().as_u16();
        let body = response
            .text()
            .map_err(|error| DriveError::Parse(error.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(DriveError::from_http(status, &body));
        }
        serde_json::from_str(&body).map_err(|error| DriveError::Parse(error.to_string()))
    }
}

impl DriveError {
    fn from_http(status: u16, body: &str) -> Self {
        let message = serde_json::from_str::<ErrorEnvelope>(body)
            .ok()
            .and_then(|envelope| envelope.error)
            .and_then(|error| error.message)
            .filter(|message| !message.is_empty());
        match status {
            401 => DriveError::Unauthorized,
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

pub struct Auth {
    pub account: DriveAccount,
    pub token: Token,
}

/// The account and access token for a Drive path, refreshing an expired token
/// once. Every operation starts here.
pub fn credentials(state: &CloudState, account_id: &str) -> Result<Auth, String> {
    credentials_with(state, account_id, refresh)
}

fn credentials_with(
    state: &CloudState,
    account_id: &str,
    refresh: impl Fn(&CloudState, &DriveAccount, &Token) -> Result<Token, String>,
) -> Result<Auth, String> {
    let account = state
        .accounts
        .lock()
        .map_err(|_| "Google Drive state is unavailable.".to_string())?
        .get(account_id)
        .cloned()
        .ok_or_else(|| "Google Drive is not connected.".to_string())?;
    let token = state
        .tokens
        .load(account_id)?
        .ok_or_else(|| "Google Drive is not connected. Sign in again.".to_string())?;
    if token.is_expired(crate::recent::now()) {
        let token = refresh(state, &account, &token)?;
        Ok(Auth { account, token })
    } else {
        Ok(Auth { account, token })
    }
}

fn refresh(state: &CloudState, account: &DriveAccount, token: &Token) -> Result<Token, String> {
    let refresh = token
        .refresh_token()
        .ok_or_else(|| "Google Drive session expired. Sign in again.".to_string())?;
    let token = crate::cloud::oauth::GoogleExchange::new().refresh(refresh)?;
    state.tokens.save(&account.id, &token)?;
    Ok(token)
}

fn authorized<T>(
    state: &CloudState,
    account_id: &str,
    work: impl Fn(&DriveClient) -> Result<T, DriveError>,
) -> Result<T, String> {
    authorized_with(state, account_id, API_BASE, refresh, work)
}

fn authorized_with<T>(
    state: &CloudState,
    account_id: &str,
    base: &str,
    refresh: impl Fn(&CloudState, &DriveAccount, &Token) -> Result<Token, String>,
    work: impl Fn(&DriveClient) -> Result<T, DriveError>,
) -> Result<T, String> {
    let auth = credentials_with(state, account_id, &refresh)?;
    let client = DriveClient::with_base(base, auth.token.access_token.clone())?;
    match work(&client) {
        Ok(value) => Ok(value),
        Err(DriveError::Unauthorized) => {
            let token = refresh(state, &auth.account, &auth.token)?;
            let client = DriveClient::with_base(base, token.access_token)?;
            work(&client).map_err(|error| error.message())
        }
        Err(error) => Err(error.message()),
    }
}

pub fn drive_location(path: &str) -> Result<DriveLocation, String> {
    match crate::cloud::parse(path)? {
        Location::Drive(drive) => Ok(drive),
        Location::Local(_) => Err("Not a Google Drive location.".into()),
    }
}

pub fn list_dir(state: &CloudState, path: &str, _show_hidden: bool) -> Result<Vec<Entry>, String> {
    let drive = drive_location(path)?;
    let folder_id = drive
        .file_id()
        .ok_or_else(|| "Google Drive location has no folder id.".to_string())?
        .to_string();
    authorized(state, &drive.account, |client| {
        list_all(client, &drive, &folder_id)
    })
}

fn list_all(
    client: &DriveClient,
    drive: &DriveLocation,
    folder_id: &str,
) -> Result<Vec<Entry>, DriveError> {
    let mut entries = Vec::new();
    let mut page = None;
    loop {
        let response = client.list(folder_id, page.as_deref())?;
        entries.extend(response.files.iter().map(|file| entry_for(drive, file)));
        if entries.len() >= LIST_LIMIT {
            break;
        }
        match response.next_page_token {
            Some(token) => page = Some(token),
            None => break,
        }
    }
    Ok(entries)
}

fn entry_for(parent: &DriveLocation, file: &DriveFile) -> Entry {
    let mut segments = parent.segments.clone();
    segments.push(Segment {
        id: file.id.clone(),
        name: file.name.clone(),
    });
    let path = DriveLocation {
        account: parent.account.clone(),
        segments,
    }
    .to_uri();
    let is_dir = file.is_dir();
    let is_google_doc = file.is_google_doc();
    let size = file
        .size
        .as_deref()
        .and_then(|size| size.parse::<u64>().ok())
        .unwrap_or(0);
    Entry {
        name: file.name.clone(),
        path,
        is_dir,
        size: if is_dir { 0 } else { size },
        modified: file
            .modified_time
            .as_deref()
            .and_then(parse_time)
            .unwrap_or(0),
        is_hidden: false,
        is_symlink: false,
        cloud: Some(CloudEntry {
            account: parent.account.clone(),
            id: file.id.clone(),
            mime_type: file.mime_type.clone(),
            is_google_doc,
            is_shortcut: file.is_shortcut(),
            web_url: file.web_view_link.clone(),
            can_download: file.can_download(),
        }),
    }
}

pub fn path_info(state: &CloudState, path: &str) -> Result<PathInfo, String> {
    let drive = drive_location(path)?;
    let id = drive
        .file_id()
        .ok_or_else(|| "Google Drive location has no file id.".to_string())?
        .to_string();
    let file = authorized(state, &drive.account, |client| client.get(&id))?;
    Ok(PathInfo {
        is_dir: file.is_dir(),
        device: 0,
    })
}

pub fn file_properties(
    state: &CloudState,
    path: &str,
    _show_hidden: bool,
) -> Result<FileProperties, String> {
    let drive = drive_location(path)?;
    let id = drive
        .file_id()
        .ok_or_else(|| "Google Drive location has no file id.".to_string())?
        .to_string();
    let owner = state
        .accounts
        .lock()
        .map_err(|_| "Google Drive state is unavailable.".to_string())?
        .get(&drive.account)
        .map(|account| account.email.clone())
        .unwrap_or_default();
    let file = authorized(state, &drive.account, |client| client.get(&id))?;
    Ok(properties_from(path, &drive, &file, &owner))
}

fn properties_from(
    path: &str,
    drive: &DriveLocation,
    file: &DriveFile,
    owner: &str,
) -> FileProperties {
    let modified = file
        .modified_time
        .as_deref()
        .and_then(parse_time)
        .unwrap_or(0);
    FileProperties {
        name: file.name.clone(),
        path: path.to_string(),
        parent: parent_uri(drive),
        is_dir: file.is_dir(),
        is_symlink: false,
        symlink_target: None,
        size: file
            .size
            .as_deref()
            .and_then(|size| size.parse().ok())
            .unwrap_or(0),
        item_count: None,
        modified,
        accessed: modified,
        created: 0,
        mode: String::new(),
        octal: String::new(),
        owner: owner.to_string(),
        group: String::new(),
        others: String::new(),
    }
}

fn parent_uri(drive: &DriveLocation) -> String {
    if drive.segments.len() <= 1 {
        return String::new();
    }
    let mut parent = drive.clone();
    parent.segments.pop();
    parent.to_uri()
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OpenTarget {
    File { path: String },
    Url { url: String },
}

/// Materialize a read-only snapshot for the default app, or return the web
/// URL for a Google-native document.
pub fn open(state: &CloudState, path: &str, cache_dir: &Path) -> Result<OpenTarget, String> {
    open_with(state, path, cache_dir, API_BASE, refresh)
}

fn open_with(
    state: &CloudState,
    path: &str,
    cache_dir: &Path,
    base: &str,
    refresh: impl Fn(&CloudState, &DriveAccount, &Token) -> Result<Token, String>,
) -> Result<OpenTarget, String> {
    let drive = drive_location(path)?;
    let id = drive
        .file_id()
        .ok_or_else(|| "Google Drive location has no file id.".to_string())?
        .to_string();
    let file = authorized_with(state, &drive.account, base, &refresh, |client| {
        client.get(&id)
    })?;
    if file.is_dir() {
        return Err("Folders open by navigating into them.".into());
    }
    if file.is_google_doc() || !file.can_download() {
        return file
            .web_view_link
            .map(|url| OpenTarget::Url { url })
            .ok_or_else(|| {
                "This Google Drive item can only be opened in the browser.".to_string()
            });
    }

    let dir = cache_dir
        .join("drive")
        .join(safe_component(&drive.account))
        .join(safe_component(&file.id));
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("Could not create the Google Drive cache: {error}"))?;
    let stamp: String = file
        .modified_time
        .as_deref()
        .unwrap_or("unknown")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let dest = dir.join(format!("{stamp}-{}", safe_name(&file.name)));
    if std::fs::metadata(&dest).is_ok_and(|meta| meta.len() > 0) {
        return Ok(OpenTarget::File {
            path: dest.to_string_lossy().to_string(),
        });
    }

    let part = dest.with_extension("part");
    let _ = std::fs::remove_file(&part);
    let result = authorized_with(state, &drive.account, base, &refresh, |client| {
        client.download(&id, &part)
    });
    if let Err(error) = result {
        let _ = std::fs::remove_file(&part);
        return Err(error);
    }
    std::fs::rename(&part, &dest)
        .map_err(|error| format!("Could not finish the Google Drive download: {error}"))?;
    Ok(OpenTarget::File {
        path: dest.to_string_lossy().to_string(),
    })
}

pub fn thumbnail(state: &CloudState, path: &str, size: u32) -> Vec<u8> {
    let Ok(drive) = drive_location(path) else {
        return Vec::new();
    };
    let Some(id) = drive.file_id().map(str::to_string) else {
        return Vec::new();
    };
    let result = authorized(state, &drive.account, |client| {
        let file = client.get(&id)?;
        match file.thumbnail_link {
            Some(link) => client.thumbnail_png(&link, size),
            None => Ok(None),
        }
    });
    result.ok().flatten().unwrap_or_default()
}

pub fn unsupported() -> String {
    "This action is not available for Google Drive items yet.".into()
}

fn parse_time(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.timestamp())
}

fn thumbnail_url(link: &str, size: u32) -> String {
    let base = link.split("=s").next().unwrap_or(link);
    format!("{base}=s{}", size.clamp(32, 1024))
}

fn safe_component(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        "item".into()
    } else {
        cleaned.chars().take(180).collect()
    }
}

fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c == '\0' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('.').trim();
    if trimmed.is_empty() {
        "item".into()
    } else {
        trimmed.chars().take(180).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::accounts::Accounts;
    use crate::cloud::oauth::SCOPE_READONLY;
    use crate::cloud::token::FileTokenStore;
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Mutex;
    use std::thread;
    use tempfile::tempdir;

    fn test_account() -> DriveAccount {
        DriveAccount::new(
            "acct".into(),
            "me@example.com".into(),
            "Me".into(),
            "root".into(),
            vec![SCOPE_READONLY.into()],
            0,
        )
    }

    fn valid_token() -> Token {
        Token::new(
            "access".into(),
            Some("refresh".into()),
            crate::recent::now() + 3600,
            vec![SCOPE_READONLY.into()],
        )
    }

    fn test_state(token: Token) -> (CloudState, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let mut accounts = Accounts::load(dir.path().join("accounts.json"));
        accounts.upsert(test_account()).unwrap();
        let state = CloudState {
            accounts: Mutex::new(accounts),
            tokens: Box::new(FileTokenStore::new(dir.path().join("tokens"))),
            flows: Mutex::new(HashMap::new()),
        };
        state.tokens.save("acct", &token).unwrap();
        (state, dir)
    }

    fn fake_refresh(
        _state: &CloudState,
        _account: &DriveAccount,
        _token: &Token,
    ) -> Result<Token, String> {
        Ok(Token::new(
            "refreshed".into(),
            Some("refresh-2".into()),
            crate::recent::now() + 3600,
            vec![SCOPE_READONLY.into()],
        ))
    }

    #[test]
    fn credentials_reads_the_account_and_token() {
        let (state, _dir) = test_state(valid_token());
        let auth = credentials(&state, "acct").unwrap();
        assert_eq!(auth.account.email, "me@example.com");
        assert_eq!(auth.token.access_token, "access");
        assert!(credentials(&state, "missing").is_err());

        state.tokens.delete("acct").unwrap();
        assert!(credentials(&state, "acct").is_err());
    }

    #[test]
    fn an_expired_token_is_refreshed_once() {
        let expired = Token::new(
            "old".into(),
            Some("refresh".into()),
            crate::recent::now() - 10,
            vec![],
        );
        let (state, _dir) = test_state(expired);
        let auth = credentials_with(&state, "acct", fake_refresh).unwrap();
        assert_eq!(auth.token.access_token, "refreshed");
    }

    #[test]
    fn a_401_refreshes_and_retries_the_operation() {
        let (state, _dir) = test_state(valid_token());
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let result = authorized_with(&state, "acct", API_BASE, fake_refresh, |_client| {
            if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                Err(DriveError::Unauthorized)
            } else {
                Ok(7)
            }
        })
        .unwrap();
        assert_eq!(result, 7);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn authorized_maps_other_errors() {
        let (state, _dir) = test_state(valid_token());
        let error = authorized_with(&state, "acct", API_BASE, fake_refresh, |_client| {
            Err::<(), _>(DriveError::NotFound)
        })
        .unwrap_err();
        assert!(error.contains("no longer exists"));
    }

    fn drive() -> DriveLocation {
        DriveLocation {
            account: "acct".into(),
            segments: vec![Segment {
                id: "root".into(),
                name: "My Drive".into(),
            }],
        }
    }

    fn file(mime: &str) -> DriveFile {
        DriveFile {
            id: "file-id".into(),
            name: "report.docx".into(),
            mime_type: mime.into(),
            size: Some("1234".into()),
            modified_time: Some("2026-10-24T17:52:03.123Z".into()),
            web_view_link: Some("https://docs.google.com/x".into()),
            thumbnail_link: Some("https://lh3.googleusercontent.com/x=s220".into()),
            capabilities: Some(Capabilities { can_download: true }),
        }
    }

    #[test]
    fn entries_are_id_anchored_and_carry_cloud_metadata() {
        let entry = entry_for(&drive(), &file("application/pdf"));
        assert_eq!(entry.name, "report.docx");
        assert!(entry.path.starts_with("gdrive://acct/root~My%20Drive/"));
        assert!(entry.path.ends_with("file-id~report.docx"));
        assert!(!entry.is_dir);
        assert_eq!(entry.size, 1234);
        assert!(entry.modified > 0);
        let cloud = entry.cloud.unwrap();
        assert_eq!(cloud.id, "file-id");
        assert!(!cloud.is_google_doc);
        assert!(cloud.can_download);
    }

    #[test]
    fn folders_and_google_docs_are_not_downloadable() {
        let folder = entry_for(&drive(), &file("application/vnd.google-apps.folder"));
        assert!(folder.is_dir);
        assert_eq!(folder.size, 0);
        assert!(!folder.cloud.unwrap().can_download);

        let doc = entry_for(&drive(), &file("application/vnd.google-apps.document"));
        assert!(!doc.is_dir);
        let cloud = doc.cloud.unwrap();
        assert!(cloud.is_google_doc);
        assert!(!cloud.can_download);
    }

    #[test]
    fn a_shortcut_is_not_a_google_doc() {
        let entry = entry_for(&drive(), &file("application/vnd.google-apps.shortcut"));
        let cloud = entry.cloud.unwrap();
        assert!(cloud.is_shortcut);
        assert!(!cloud.is_google_doc);
    }

    #[test]
    fn missing_size_and_time_are_zero() {
        let mut f = file("text/plain");
        f.size = None;
        f.modified_time = None;
        let entry = entry_for(&drive(), &f);
        assert_eq!(entry.size, 0);
        assert_eq!(entry.modified, 0);
    }

    #[test]
    fn properties_drop_the_last_segment_for_the_parent() {
        let loc = DriveLocation {
            account: "acct".into(),
            segments: vec![
                Segment {
                    id: "root".into(),
                    name: "My Drive".into(),
                },
                Segment {
                    id: "f1".into(),
                    name: "folder".into(),
                },
                Segment {
                    id: "f2".into(),
                    name: "file.txt".into(),
                },
            ],
        };
        let props = properties_from(&loc.to_uri(), &loc, &file("text/plain"), "me@example.com");
        assert!(props.parent.ends_with("f1~folder"));
        assert_eq!(props.owner, "me@example.com");
        assert_eq!(props.size, 1234);
    }

    #[test]
    fn the_root_has_no_parent() {
        assert!(parent_uri(&drive()).is_empty());
    }

    #[test]
    fn thumbnails_get_one_size_suffix() {
        assert_eq!(thumbnail_url("https://x/y=s220", 256), "https://x/y=s256");
        assert_eq!(thumbnail_url("https://x/y", 2000), "https://x/y=s1024");
    }

    #[test]
    fn names_cannot_escape_the_cache() {
        let name = safe_name("../../etc/passwd");
        assert!(!name.contains('/'));
        assert!(!name.starts_with(".."));
        assert_eq!(safe_name(""), "item");
        assert_eq!(safe_component(".."), "item");
        assert!(!safe_name("a/b\\c").contains('/'));
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
        let forbidden = DriveError::from_http(403, r#"{"error":{"message":"nope"}}"#);
        assert!(forbidden.message().contains("nope"));
    }

    /// Serve one response per call, in order, on a local port.
    fn serve(responses: Vec<(u16, Vec<u8>, &'static str)>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            for (status, body, content_type) in responses {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut buffer = [0u8; 4096];
                let _ = stream.read(&mut buffer);
                let reason = if status == 200 { "OK" } else { "Error" };
                let head = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        (format!("http://{addr}"), handle)
    }

    fn json(value: &str) -> Vec<u8> {
        value.as_bytes().to_vec()
    }

    #[test]
    fn listing_follows_pages_and_maps_entries() {
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
        let client = DriveClient::with_base(&base, "token".into()).unwrap();
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
    fn list_all_follows_pages_and_builds_paths() {
        let (base, server) = serve(vec![
            (
                200,
                json(
                    r#"{"files":[{"id":"f1","name":"a.txt","mimeType":"text/plain","size":"12"}],"nextPageToken":"p2"}"#,
                ),
                "application/json",
            ),
            (
                200,
                json(r#"{"files":[{"id":"f2","name":"b.txt","mimeType":"text/plain"}]}"#),
                "application/json",
            ),
        ]);
        let client = DriveClient::with_base(&base, "token".into()).unwrap();
        let entries = list_all(&client, &drive(), "root").unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries[0].path.ends_with("f1~a.txt"));
        assert!(entries[1].cloud.is_some());
        server.join().unwrap();
    }

    #[test]
    fn open_returns_the_web_url_for_a_google_doc() {
        let (base, server) = serve(vec![(
            200,
            json(
                r#"{"id":"f1","name":"Plan","mimeType":"application/vnd.google-apps.document","webViewLink":"https://docs.google.com/document/d/f1"}"#,
            ),
            "application/json",
        )]);
        let cache = tempdir().unwrap();
        let (state, _dir) = test_state(valid_token());
        let target = open_with(
            &state,
            "gdrive://acct/root~My%20Drive/f1~Plan",
            cache.path(),
            &base,
            fake_refresh,
        )
        .unwrap();
        match target {
            OpenTarget::Url { url } => assert_eq!(url, "https://docs.google.com/document/d/f1"),
            OpenTarget::File { .. } => panic!("a Google Doc must not be downloaded"),
        }
        server.join().unwrap();
    }

    #[test]
    fn open_downloads_a_file_snapshot() {
        let (base, server) = serve(vec![
            (
                200,
                json(
                    r#"{"id":"f1","name":"hello.txt","mimeType":"text/plain","size":"5","modifiedTime":"2026-10-24T17:52:03Z","capabilities":{"canDownload":true}}"#,
                ),
                "application/json",
            ),
            (200, b"hello".to_vec(), "application/octet-stream"),
        ]);
        let cache = tempdir().unwrap();
        let (state, _dir) = test_state(valid_token());
        let target = open_with(
            &state,
            "gdrive://acct/root~My%20Drive/f1~hello.txt",
            cache.path(),
            &base,
            fake_refresh,
        )
        .unwrap();
        match target {
            OpenTarget::File { path } => {
                assert_eq!(std::fs::read(path).unwrap(), b"hello");
            }
            OpenTarget::Url { .. } => panic!("a regular file must be downloaded"),
        }
        server.join().unwrap();
    }

    #[test]
    fn open_refuses_a_folder() {
        let (base, server) = serve(vec![(
            200,
            json(r#"{"id":"f1","name":"Docs","mimeType":"application/vnd.google-apps.folder"}"#),
            "application/json",
        )]);
        let cache = tempdir().unwrap();
        let (state, _dir) = test_state(valid_token());
        let error = open_with(
            &state,
            "gdrive://acct/root~My%20Drive/f1~Docs",
            cache.path(),
            &base,
            fake_refresh,
        )
        .unwrap_err();
        assert!(error.contains("navigating"));
        server.join().unwrap();
    }

    #[test]
    fn a_download_writes_the_media_bytes() {
        let (base, server) = serve(vec![(
            200,
            b"hello drive".to_vec(),
            "application/octet-stream",
        )]);
        let dir = tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        let client = DriveClient::with_base(&base, "token".into()).unwrap();
        client.download("f1", &dest).unwrap();
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
        let client = DriveClient::with_base(&base, "token".into()).unwrap();
        let error = client.list("root", None).unwrap_err();
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
        let client = DriveClient::with_base(&base, "token".into()).unwrap();
        let bytes = client
            .thumbnail_png(&format!("{base}/thumb"), 256)
            .unwrap()
            .unwrap();
        assert!(bytes.starts_with(b"\x89PNG"));
        server.join().unwrap();
    }
}
