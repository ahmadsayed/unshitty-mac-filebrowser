//! Compiles the Google OAuth desktop client from the git-ignored `.env` into
//! the binary. The secret is not source code; it never appears in the repo.
//! A build without `.env` still compiles; Google Drive sign-in then reports
//! that it is not configured.

use std::path::Path;

#[derive(serde::Deserialize)]
struct Installed {
    client_id: String,
    client_secret: String,
}

#[derive(serde::Deserialize)]
struct Env {
    installed: Installed,
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../.env");

    if let Ok(text) = std::fs::read_to_string(Path::new("../.env")) {
        if let Some((client_id, client_secret)) = parse_client(&text) {
            println!("cargo:rustc-env=FILEBROWSER_GOOGLE_CLIENT_ID={client_id}");
            println!("cargo:rustc-env=FILEBROWSER_GOOGLE_CLIENT_SECRET={client_secret}");
        }
    }

    tauri_build::build()
}

fn parse_client(text: &str) -> Option<(String, String)> {
    let env: Env = serde_json::from_str(text).ok()?;
    Some((env.installed.client_id, env.installed.client_secret))
}
