//! Compiles the Google OAuth desktop client from the git-ignored `.env` into
//! the binary. The secret is not source code; it never appears in the repo.
//! CI may set `FILEBROWSER_GOOGLE_CLIENT_ID`/`_SECRET` instead. A build with
//! neither still compiles, with a warning; Google Drive sign-in then reports
//! that it is not configured. A malformed `.env` fails the build.

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
    // CI can set these instead of writing `.env`; `option_env!` reads them.
    println!("cargo:rerun-if-env-changed=FILEBROWSER_GOOGLE_CLIENT_ID");
    println!("cargo:rerun-if-env-changed=FILEBROWSER_GOOGLE_CLIENT_SECRET");

    match std::fs::read_to_string(Path::new("../.env")) {
        Ok(text) => {
            // A broken `.env` is a mistake to fix, not a build without Drive.
            let (client_id, client_secret) = parse_client(&text).unwrap_or_else(|| {
                panic!(
                    "../.env is not the Google OAuth client JSON \
                     ({{\"installed\": {{\"client_id\", \"client_secret\", ...}}}})"
                )
            });
            println!("cargo:rustc-env=FILEBROWSER_GOOGLE_CLIENT_ID={client_id}");
            println!("cargo:rustc-env=FILEBROWSER_GOOGLE_CLIENT_SECRET={client_secret}");
        }
        Err(_) if has_env_client() => {}
        Err(_) => println!(
            "cargo:warning=No Google OAuth client (.env or FILEBROWSER_GOOGLE_CLIENT_ID/_SECRET); \
             Google Drive sign-in is disabled in this build."
        ),
    }

    tauri_build::build()
}

fn has_env_client() -> bool {
    [
        "FILEBROWSER_GOOGLE_CLIENT_ID",
        "FILEBROWSER_GOOGLE_CLIENT_SECRET",
    ]
    .iter()
    .all(|key| std::env::var(key).is_ok_and(|value| !value.is_empty()))
}

fn parse_client(text: &str) -> Option<(String, String)> {
    let env: Env = serde_json::from_str(text).ok()?;
    Some((env.installed.client_id, env.installed.client_secret))
}
