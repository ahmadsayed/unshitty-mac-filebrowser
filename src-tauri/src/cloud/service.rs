//! Read-only Google Drive operations built on the REST client: listing,
//! metadata, downloading a local snapshot and thumbnails. Everything that
//! touches the network runs on a blocking thread; the frontend simply sees
//! `Entry` values with a `cloud` field.

use std::path::Path;

use chrono::DateTime;
use serde::Serialize;

use crate::cloud::CloudEntry;
use crate::cloud::CloudState;
use crate::cloud::accounts::DriveAccount;
use crate::cloud::drive::{API_BASE, DriveClient, DriveError, DriveFile};
use crate::cloud::location::{DriveLocation, Location, Segment};
use crate::cloud::oauth::TokenExchange;
use crate::cloud::token::Token;
use crate::fs_ops::{Entry, PathInfo};
use crate::properties::FileProperties;

/// Stop after this many children; a Drive listing is not streamed page by page
/// in the UI yet. Reported in the README as the large-folder limit.
pub const LIST_LIMIT: usize = 10_000;

pub struct Auth {
    pub account: DriveAccount,
    pub token: Token,
}

/// Exchanges a refresh token for a new access token. Tests use a fake.
type Refresh<'a> = &'a dyn Fn(&str) -> Result<Token, String>;

/// Refreshes with the user's OAuth client, the one that issued the token.
fn google_refresh(state: &CloudState) -> impl Fn(&str) -> Result<Token, String> + '_ {
    move |refresh_token| {
        // An account connected before clients were per-user has none stored.
        let client = state.oauth_client().ok_or(
            "Google Drive session expired. Disconnect Google Drive, then connect again with your own OAuth client.",
        )?;
        crate::cloud::oauth::GoogleExchange::new(&client).refresh(refresh_token)
    }
}

/// The account and access token for a Drive path, refreshing an expired token
/// once. Every operation starts here.
pub fn credentials(state: &CloudState, account_id: &str) -> Result<Auth, String> {
    credentials_with(state, account_id, &google_refresh(state))
}

fn credentials_with(
    state: &CloudState,
    account_id: &str,
    refresh: Refresh,
) -> Result<Auth, String> {
    let account = state
        .accounts
        .lock()
        .map_err(|_| "Google Drive state is unavailable.".to_string())?
        .get(account_id)
        .cloned()
        .ok_or_else(|| "Google Drive is not connected.".to_string())?;
    let token = state
        .token(account_id)?
        .ok_or_else(|| "Google Drive is not connected. Sign in again.".to_string())?;
    let token = if token.is_expired(crate::recent::now()) {
        renew(state, &account, &token, refresh)?
    } else {
        token
    };
    Ok(Auth { account, token })
}

/// Replace `stale` with a fresh token. Parallel callers wait on one lock; the
/// ones that arrive after a refresh reuse its result instead of refreshing
/// again.
fn renew(
    state: &CloudState,
    account: &DriveAccount,
    stale: &Token,
    refresh: Refresh,
) -> Result<Token, String> {
    let _refreshing = state
        .refreshing
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(current) = state.token(&account.id)? {
        if current.access_token != stale.access_token && !current.is_expired(crate::recent::now()) {
            return Ok(current);
        }
    }
    let refresh_token = stale
        .refresh_token()
        .ok_or_else(|| "Google Drive session expired. Sign in again.".to_string())?;
    let mut token = refresh(refresh_token)?;
    // Google's refresh response usually omits the refresh token and may omit
    // the scopes; both stay valid and must be kept.
    if token.refresh_token.is_none() {
        token.refresh_token = stale.refresh_token.clone();
    }
    if token.scopes.is_empty() {
        token.scopes = stale.scopes.clone();
    }
    state.save_token(&account.id, &token)?;
    Ok(token)
}

fn authorized<T>(
    state: &CloudState,
    account_id: &str,
    work: impl Fn(&DriveClient) -> Result<T, DriveError>,
) -> Result<T, String> {
    authorized_with(state, account_id, API_BASE, &google_refresh(state), work)
}

/// Run `work` with a client for the account; a 401 refreshes the token and
/// retries once.
fn authorized_with<T>(
    state: &CloudState,
    account_id: &str,
    base: &str,
    refresh: Refresh,
    work: impl Fn(&DriveClient) -> Result<T, DriveError>,
) -> Result<T, String> {
    let auth = credentials_with(state, account_id, refresh)?;
    let http = state.http()?;
    let client = DriveClient::new(http.clone(), base, auth.token.access_token.clone());
    match work(&client) {
        Ok(value) => Ok(value),
        Err(DriveError::Unauthorized) => {
            let token = renew(state, &auth.account, &auth.token, refresh)?;
            let client = DriveClient::new(http, base, token.access_token);
            work(&client).map_err(|error| error.message())
        }
        Err(error) => Err(error.message()),
    }
}

/// Fetch the account details for a token that was just issued by sign-in.
pub fn about(
    state: &CloudState,
    access_token: &str,
) -> Result<crate::cloud::drive::AboutInfo, String> {
    DriveClient::new(state.http()?, API_BASE, access_token.to_string())
        .about()
        .map_err(|error| error.message())
}

pub const READONLY: &str =
    "Google Drive is connected read-only. Disconnect and connect again to allow changes.";

/// The location and account for a write, refused before any request when
/// the connection was made without write access.
pub fn writable(state: &CloudState, path: &str) -> Result<(DriveLocation, DriveAccount), String> {
    let drive = drive_location(path)?;
    let account = state
        .accounts
        .lock()
        .map_err(|_| "Google Drive state is unavailable.".to_string())?
        .get(&drive.account)
        .cloned()
        .ok_or_else(|| "Google Drive is not connected.".to_string())?;
    if !account
        .scopes
        .iter()
        .any(|scope| scope == crate::cloud::oauth::SCOPE_FULL)
    {
        return Err(READONLY.into());
    }
    Ok((drive, account))
}

pub fn drive_location(path: &str) -> Result<DriveLocation, String> {
    match crate::cloud::parse(path)? {
        Location::Drive(drive) => Ok(drive),
        Location::Local(_) => Err("Not a Google Drive location.".into()),
    }
}

pub fn list_dir(state: &CloudState, path: &str, _show_hidden: bool) -> Result<Vec<Entry>, String> {
    list_dir_with(state, path, API_BASE, &google_refresh(state))
}

fn list_dir_with(
    state: &CloudState,
    path: &str,
    base: &str,
    refresh: Refresh,
) -> Result<Vec<Entry>, String> {
    let drive = drive_location(path)?;
    let folder_id = drive
        .file_id()
        .ok_or_else(|| "Google Drive location has no folder id.".to_string())?
        .to_string();
    let files = authorized_with(state, &drive.account, base, refresh, |client| {
        list_all(client, &folder_id)
    })?;
    for file in &files {
        if let Some(link) = &file.thumbnail_link {
            state.remember_thumbnail_link(&drive.account, &file.id, link);
        }
    }
    Ok(files.iter().map(|file| entry_for(&drive, file)).collect())
}

fn list_all(client: &DriveClient, folder_id: &str) -> Result<Vec<DriveFile>, DriveError> {
    let mut files = Vec::new();
    let mut page = None;
    loop {
        let response = client.list(folder_id, page.as_deref())?;
        files.extend(response.files);
        if files.len() >= LIST_LIMIT {
            break;
        }
        match response.next_page_token {
            Some(token) => page = Some(token),
            None => break,
        }
    }
    Ok(files)
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

/// Whether a Drive path names an existing, untrashed item. A pending path
/// (`…/~name`) exists when its parent has a child with that name.
pub fn path_exists(state: &CloudState, path: &str) -> bool {
    path_exists_with(state, path, API_BASE, &google_refresh(state))
}

fn path_exists_with(state: &CloudState, path: &str, base: &str, refresh: Refresh) -> bool {
    let Ok(drive) = drive_location(path) else {
        return false;
    };
    let result = if drive.is_pending() {
        let (Some(parent), Some(name)) = (drive.parent(), drive.name()) else {
            return false;
        };
        let Some(parent_id) = parent.file_id().map(str::to_string) else {
            return false;
        };
        authorized_with(state, &drive.account, base, refresh, |client| {
            client
                .find_children(&parent_id, name)
                .map(|found| !found.is_empty())
        })
    } else {
        let Some(id) = drive.file_id().map(str::to_string) else {
            return false;
        };
        authorized_with(
            state,
            &drive.account,
            base,
            refresh,
            |client| match client.get(&id) {
                Ok(file) => Ok(!file.trashed),
                Err(DriveError::NotFound) => Ok(false),
                Err(error) => Err(error),
            },
        )
    };
    result.unwrap_or(false)
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
    open_with(state, path, cache_dir, API_BASE, &google_refresh(state))
}

fn open_with(
    state: &CloudState,
    path: &str,
    cache_dir: &Path,
    base: &str,
    refresh: Refresh,
) -> Result<OpenTarget, String> {
    let drive = drive_location(path)?;
    let id = drive
        .file_id()
        .ok_or_else(|| "Google Drive location has no file id.".to_string())?
        .to_string();
    let file = authorized_with(state, &drive.account, base, refresh, |client| {
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
    let result = authorized_with(state, &drive.account, base, refresh, |client| {
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

/// PNG thumbnail bytes, empty when there is none. Uses the link remembered
/// from the listing first; links expire after a few hours, so a failure falls
/// back to fresh metadata.
pub fn thumbnail(state: &CloudState, path: &str, size: u32) -> Vec<u8> {
    thumbnail_with(state, path, size, API_BASE, &google_refresh(state))
}

fn thumbnail_with(
    state: &CloudState,
    path: &str,
    size: u32,
    base: &str,
    refresh: Refresh,
) -> Vec<u8> {
    let Ok(drive) = drive_location(path) else {
        return Vec::new();
    };
    let Some(id) = drive.file_id().map(str::to_string) else {
        return Vec::new();
    };
    let remembered = state.thumbnail_link(&drive.account, &id);
    let result = authorized_with(state, &drive.account, base, refresh, |client| {
        if let Some(png) = remembered
            .as_deref()
            .and_then(|link| client.thumbnail_png(link, size))
        {
            return Ok(Some(png));
        }
        let Some(link) = client.get(&id)?.thumbnail_link else {
            return Ok(None);
        };
        state.remember_thumbnail_link(&drive.account, &id, &link);
        Ok(client.thumbnail_png(&link, size))
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
    use crate::cloud::drive::Capabilities;
    use crate::cloud::oauth::SCOPE_READONLY;
    use crate::cloud::test_server::{json, serve};
    use crate::cloud::token::{FileTokenStore, TokenStore};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
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

    fn expired_token() -> Token {
        Token::new(
            "old".into(),
            Some("refresh".into()),
            crate::recent::now() - 10,
            vec![SCOPE_READONLY.into()],
        )
    }

    fn test_state(token: Token) -> (CloudState, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let mut accounts = Accounts::load(dir.path().join("accounts.json"));
        accounts.upsert(test_account()).unwrap();
        let store = FileTokenStore::new(dir.path().join("tokens"));
        store.save("acct", &token).unwrap();
        (
            CloudState::with_store(
                accounts,
                Box::new(store),
                crate::cloud::client::ClientStore::new(dir.path().join("google-client.json")),
            ),
            dir,
        )
    }

    /// Google's refresh response: a new access token, no refresh token, no
    /// scopes.
    fn fake_refresh(_refresh_token: &str) -> Result<Token, String> {
        Ok(Token::new(
            "refreshed".into(),
            None,
            crate::recent::now() + 3600,
            vec![],
        ))
    }

    #[test]
    fn credentials_reads_the_account_and_token() {
        let (state, _dir) = test_state(valid_token());
        let auth = credentials(&state, "acct").unwrap();
        assert_eq!(auth.account.email, "me@example.com");
        assert_eq!(auth.token.access_token, "access");
        assert!(credentials(&state, "missing").is_err());

        state.delete_token("acct").unwrap();
        assert!(credentials(&state, "acct").is_err());
    }

    #[test]
    fn the_token_is_read_from_storage_once() {
        let (state, _dir) = test_state(valid_token());
        credentials(&state, "acct").unwrap();
        // Storage changing behind our back does not matter: memory wins.
        state.tokens.delete("acct").unwrap();
        assert_eq!(
            credentials(&state, "acct").unwrap().token.access_token,
            "access"
        );
    }

    #[test]
    fn an_expired_token_is_refreshed_and_keeps_its_refresh_token() {
        let (state, _dir) = test_state(expired_token());
        let auth = credentials_with(&state, "acct", &fake_refresh).unwrap();
        assert_eq!(auth.token.access_token, "refreshed");
        assert_eq!(auth.token.refresh_token(), Some("refresh"));
        assert_eq!(auth.token.scopes, vec![SCOPE_READONLY.to_string()]);

        let stored = state.tokens.load("acct").unwrap().unwrap();
        assert_eq!(stored.access_token, "refreshed");
        assert_eq!(stored.refresh_token(), Some("refresh"));
    }

    #[test]
    fn parallel_requests_refresh_once() {
        let (state, _dir) = test_state(expired_token());
        let state = Arc::new(state);
        let calls = Arc::new(AtomicUsize::new(0));
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let state = Arc::clone(&state);
                let calls = Arc::clone(&calls);
                std::thread::spawn(move || {
                    let refresh = |token: &str| {
                        calls.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(std::time::Duration::from_millis(20));
                        fake_refresh(token)
                    };
                    credentials_with(&state, "acct", &refresh)
                        .unwrap()
                        .token
                        .access_token
                })
            })
            .collect();
        for thread in threads {
            assert_eq!(thread.join().unwrap(), "refreshed");
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn refreshing_without_an_oauth_client_says_to_reconnect() {
        let (state, _dir) = test_state(expired_token());
        let error = credentials(&state, "acct").err().unwrap();
        assert!(error.contains("Disconnect Google Drive"), "{error}");
    }

    #[test]
    fn a_session_without_a_refresh_token_must_sign_in_again() {
        let mut token = expired_token();
        token.refresh_token = None;
        let (state, _dir) = test_state(token);
        let error = credentials_with(&state, "acct", &fake_refresh)
            .err()
            .unwrap();
        assert!(error.contains("Sign in again"));
    }

    #[test]
    fn a_401_refreshes_and_retries_the_operation() {
        let (state, _dir) = test_state(valid_token());
        let calls = AtomicUsize::new(0);
        let result = authorized_with(&state, "acct", API_BASE, &fake_refresh, |client| {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(DriveError::Unauthorized)
            } else {
                let _ = client;
                Ok(7)
            }
        })
        .unwrap();
        assert_eq!(result, 7);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            state.token("acct").unwrap().unwrap().access_token,
            "refreshed"
        );
    }

    #[test]
    fn authorized_maps_other_errors() {
        let (state, _dir) = test_state(valid_token());
        let error = authorized_with(&state, "acct", API_BASE, &fake_refresh, |_client| {
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
            trashed: false,
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
    fn names_cannot_escape_the_cache() {
        let name = safe_name("../../etc/passwd");
        assert!(!name.contains('/'));
        assert!(!name.starts_with(".."));
        assert_eq!(safe_name(""), "item");
        assert_eq!(safe_component(".."), "item");
        assert!(!safe_name("a/b\\c").contains('/'));
    }

    fn full_reply(json_body: &str) -> (u16, Vec<u8>, &'static str) {
        (200, json(json_body), "application/json")
    }

    #[test]
    fn a_pending_path_exists_when_the_parent_has_that_name() {
        let (base, log, server) = crate::cloud::test_server::serve_logged(vec![full_reply(
            r#"{"files":[{"id":"f1","name":"a.txt","mimeType":"text/plain"}]}"#,
        )]);
        let (state, _dir) = test_state(valid_token());
        assert!(path_exists_with(
            &state,
            "gdrive://acct/root~My%20Drive/~a.txt",
            &base,
            &fake_refresh
        ));
        server.join().unwrap();
        let request = log.lock().unwrap()[0].clone();
        assert!(request.contains("name = 'a.txt'"), "{request}");
        assert!(request.contains("'root' in parents"), "{request}");
        assert!(request.contains("trashed = false"), "{request}");
    }

    #[test]
    fn a_pending_path_is_free_when_no_child_matches() {
        let (base, server) = serve(vec![full_reply(r#"{"files":[]}"#)]);
        let (state, _dir) = test_state(valid_token());
        assert!(!path_exists_with(
            &state,
            "gdrive://acct/root~My%20Drive/~a.txt",
            &base,
            &fake_refresh
        ));
        server.join().unwrap();
    }

    #[test]
    fn an_id_path_exists_unless_trashed() {
        let (base, server) = serve(vec![
            full_reply(r#"{"id":"f1","name":"a","mimeType":"text/plain"}"#),
            full_reply(r#"{"id":"f1","name":"a","mimeType":"text/plain","trashed":true}"#),
            (404, json("{}"), "application/json"),
        ]);
        let (state, _dir) = test_state(valid_token());
        let path = "gdrive://acct/root~My%20Drive/f1~a";
        assert!(path_exists_with(&state, path, &base, &fake_refresh));
        assert!(!path_exists_with(&state, path, &base, &fake_refresh));
        assert!(!path_exists_with(&state, path, &base, &fake_refresh));
        server.join().unwrap();
    }

    #[test]
    fn names_with_quotes_are_escaped_in_the_query() {
        let (base, log, server) =
            crate::cloud::test_server::serve_logged(vec![full_reply(r#"{"files":[]}"#)]);
        let (state, _dir) = test_state(valid_token());
        path_exists_with(
            &state,
            "gdrive://acct/root~My%20Drive/~it%27s",
            &base,
            &fake_refresh,
        );
        server.join().unwrap();
        let request = log.lock().unwrap()[0].clone();
        assert!(request.contains(r"name = 'it\'s'"), "{request}");
    }

    #[test]
    fn readonly_accounts_cannot_write() {
        let (state, _dir) = test_state(valid_token());
        let path = "gdrive://acct/root~My%20Drive/~New";
        assert_eq!(writable(&state, path).err().as_deref(), Some(READONLY));

        let mut account = test_account();
        account.scopes = vec![crate::cloud::oauth::SCOPE_FULL.into()];
        state.accounts.lock().unwrap().upsert(account).unwrap();
        let (location, account) = writable(&state, path).unwrap();
        assert!(location.is_pending());
        assert_eq!(account.id, "acct");
        assert!(writable(&state, "/Users/me").is_err());
    }

    #[test]
    fn list_all_follows_pages() {
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
        let client = DriveClient::new(
            crate::cloud::drive::http_client().unwrap(),
            &base,
            "token".into(),
        );
        let files = list_all(&client, "root").unwrap();
        assert_eq!(files.len(), 2);
        assert!(entry_for(&drive(), &files[0]).path.ends_with("f1~a.txt"));
        server.join().unwrap();
    }

    #[test]
    fn listing_maps_entries_and_remembers_thumbnail_links() {
        let (base, server) = serve(vec![(
            200,
            json(
                r#"{"files":[{"id":"f1","name":"a.png","mimeType":"image/png","thumbnailLink":"https://lh3/t=s220"},{"id":"f2","name":"b.txt","mimeType":"text/plain"}]}"#,
            ),
            "application/json",
        )]);
        let (state, _dir) = test_state(valid_token());
        let entries = list_dir_with(
            &state,
            "gdrive://acct/root~My%20Drive",
            &base,
            &fake_refresh,
        )
        .unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries[0].path.ends_with("f1~a.png"));
        assert_eq!(
            state.thumbnail_link("acct", "f1").as_deref(),
            Some("https://lh3/t=s220")
        );
        assert!(state.thumbnail_link("acct", "f2").is_none());
        server.join().unwrap();

        assert!(list_dir_with(&state, "/Users/me", &base, &fake_refresh).is_err());
    }

    fn png() -> Vec<u8> {
        let mut png = Vec::new();
        image::DynamicImage::new_rgb8(1, 1)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        png
    }

    #[test]
    fn a_remembered_thumbnail_link_needs_one_request() {
        let (base, server) = serve(vec![(200, png(), "image/png")]);
        let (state, _dir) = test_state(valid_token());
        state.remember_thumbnail_link("acct", "f1", &format!("{base}/t=s220"));
        let bytes = thumbnail_with(
            &state,
            "gdrive://acct/root~My%20Drive/f1~a.png",
            256,
            &base,
            &fake_refresh,
        );
        assert!(bytes.starts_with(b"\x89PNG"));
        server.join().unwrap();
    }

    #[test]
    fn an_expired_thumbnail_link_is_fetched_again() {
        let (images, image_server) = serve(vec![(200, png(), "image/png")]);
        let fresh = format!("{images}/fresh=s220");
        let (base, api_server) = serve(vec![
            (403, json("{}"), "application/json"),
            (
                200,
                json(&format!(
                    r#"{{"id":"f1","name":"a.png","mimeType":"image/png","thumbnailLink":"{fresh}"}}"#
                )),
                "application/json",
            ),
        ]);
        let (state, _dir) = test_state(valid_token());
        state.remember_thumbnail_link("acct", "f1", &format!("{base}/stale=s220"));
        let bytes = thumbnail_with(
            &state,
            "gdrive://acct/root~My%20Drive/f1~a.png",
            256,
            &base,
            &fake_refresh,
        );
        assert!(bytes.starts_with(b"\x89PNG"));
        assert_eq!(
            state.thumbnail_link("acct", "f1").as_deref(),
            Some(fresh.as_str())
        );
        api_server.join().unwrap();
        image_server.join().unwrap();
    }

    #[test]
    fn thumbnail_links_are_remembered_per_account() {
        let (state, _dir) = test_state(valid_token());
        state.remember_thumbnail_link("acct", "f1", "https://x/y=s220");
        assert_eq!(
            state.thumbnail_link("acct", "f1").as_deref(),
            Some("https://x/y=s220")
        );
        assert!(state.thumbnail_link("other", "f1").is_none());
        state.delete_token("acct").unwrap();
        assert!(state.thumbnail_link("acct", "f1").is_none());
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
            &fake_refresh,
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
            &fake_refresh,
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
            &fake_refresh,
        )
        .unwrap_err();
        assert!(error.contains("navigating"));
        server.join().unwrap();
    }
}
