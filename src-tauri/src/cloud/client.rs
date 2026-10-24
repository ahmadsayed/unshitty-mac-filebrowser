//! The user's own Google OAuth client. File Browser ships no client: each user
//! creates a Desktop-app client in their Google Cloud project and gives the app
//! the JSON file Google lets them download (see `docs/google-drive.md`).
//!
//! Google treats a Desktop client's secret as non-confidential, so it is kept
//! in a `0600` file in the app config directory rather than the keychain. It
//! is never sent to the frontend.

use std::fmt;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OAuthClient {
    pub client_id: String,
    pub client_secret: String,
}

// Keep the secret out of logs and error messages.
impl fmt::Debug for OAuthClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthClient")
            .field("client_id", &self.client_id)
            .field("client_secret", &"<redacted>")
            .finish()
    }
}

/// Google's downloaded client file: `{"installed": {...}}` for a Desktop app,
/// `{"web": {...}}` for a web application.
#[derive(Deserialize)]
struct ClientFile {
    installed: Option<ClientFields>,
    web: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct ClientFields {
    client_id: Option<String>,
    client_secret: Option<String>,
}

/// Read the JSON file Google offers after creating an OAuth client.
pub fn parse(json: &str) -> Result<OAuthClient, String> {
    let file: ClientFile = serde_json::from_str(json.trim()).map_err(|_| {
        "This is not an OAuth client file. Download the JSON from the client's page in Google Cloud."
            .to_string()
    })?;
    let Some(fields) = file.installed else {
        return Err(if file.web.is_some() {
            "This is a web application client. Create a client of type Desktop app.".into()
        } else {
            "This is not an OAuth client file. Download the JSON from the client's page in Google Cloud."
                .into()
        });
    };
    let client_id = fields
        .client_id
        .map(|id| id.trim().to_string())
        .filter(|id| id.ends_with(".apps.googleusercontent.com"))
        .ok_or("The client file has no valid client ID.")?;
    let client_secret = fields
        .client_secret
        .map(|secret| secret.trim().to_string())
        .filter(|secret| !secret.is_empty())
        .ok_or("The client file has no client secret. Download it again from Google Cloud.")?;
    Ok(OAuthClient {
        client_id,
        client_secret,
    })
}

/// `google-client.json` in the app config directory.
pub struct ClientStore {
    path: PathBuf,
}

impl ClientStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> Option<OAuthClient> {
        let text = fs::read_to_string(&self.path).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self, client: &OAuthClient) -> Result<(), String> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)
                .map_err(|error| format!("Could not save the OAuth client: {error}"))?;
        }
        let json = serde_json::to_vec_pretty(client).map_err(|error| error.to_string())?;
        crate::cloud::token::write_private(&self.path, &json)
    }

    pub fn delete(&self) -> Result<(), String> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("Could not remove the OAuth client: {error}")),
        }
    }
}

#[cfg(test)]
pub(crate) fn test_client() -> OAuthClient {
    OAuthClient {
        client_id: "123-abc.apps.googleusercontent.com".into(),
        client_secret: "GOCSPX-test".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const DESKTOP: &str = r#"{"installed":{"client_id":"123-abc.apps.googleusercontent.com","project_id":"p","auth_uri":"https://accounts.google.com/o/oauth2/auth","token_uri":"https://oauth2.googleapis.com/token","client_secret":"GOCSPX-test","redirect_uris":["http://localhost"]}}"#;

    #[test]
    fn a_desktop_client_file_is_accepted() {
        assert_eq!(parse(DESKTOP).unwrap(), test_client());
        assert_eq!(parse(&format!("\n  {DESKTOP}\n")).unwrap(), test_client());
    }

    #[test]
    fn a_web_client_asks_for_a_desktop_client() {
        let error = parse(r#"{"web":{"client_id":"x.apps.googleusercontent.com"}}"#).unwrap_err();
        assert!(error.contains("Desktop app"));
    }

    #[test]
    fn other_json_is_not_a_client_file() {
        assert!(
            parse("not json")
                .unwrap_err()
                .contains("not an OAuth client")
        );
        assert!(parse("{}").unwrap_err().contains("not an OAuth client"));
    }

    #[test]
    fn missing_or_invalid_fields_are_refused() {
        let no_secret = r#"{"installed":{"client_id":"123-abc.apps.googleusercontent.com"}}"#;
        assert!(parse(no_secret).unwrap_err().contains("client secret"));
        let bad_id = r#"{"installed":{"client_id":"abc","client_secret":"s"}}"#;
        assert!(parse(bad_id).unwrap_err().contains("client ID"));
    }

    #[test]
    fn debug_output_hides_the_secret() {
        let text = format!("{:?}", test_client());
        assert!(text.contains("123-abc"));
        assert!(!text.contains("GOCSPX"));
    }

    #[test]
    fn the_store_saves_privately_and_deletes() {
        let dir = tempdir().unwrap();
        let store = ClientStore::new(dir.path().join("config").join("google-client.json"));
        assert!(store.load().is_none());
        store.save(&test_client()).unwrap();
        assert_eq!(store.load(), Some(test_client()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.path().join("config/google-client.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        store.delete().unwrap();
        assert!(store.load().is_none());
        store.delete().unwrap();
    }
}
