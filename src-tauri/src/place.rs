//! Which kind of place a path names, and what the backend allows there: the
//! backend twin of `placeOf` (`src/paths.ts`) and `src/locations.ts`.
//!
//! A narrow gate, not a router. A command that works on this computer only
//! takes its path from [`local`] (or [`local_write`]), a write command asks
//! [`writable`] first (a copy, move or rename asks [`transfer`]), and a
//! batch command with nothing to say about remote paths answers through
//! [`local_only`]. A command that asks never hands `std::fs` a `gdrive://`,
//! `share://` or `sftp://` path, well formed or not, nor a relative path,
//! which would be read from the app's working directory. The program and
//! disk-image actions and dragging out refuse anything but an absolute path
//! themselves. Listing, existence, path info, thumbnails and opening ask
//! the place's provider in the `places::Registry` (`providers.rs`), whose
//! prefix rule matches [`kind`]'s; the write commands that serve remote
//! paths (Drive's copies, say) still pick their handler with [`kind`]. Each
//! handler parses its own scheme (`places_gdrive::location`, `share::location`,
//! `places_sftp`) and refuses a malformed path.
//!
//! Plain functions, no Tauri.

use std::path::{Path, PathBuf};

use crate::share;

// ---- Kinds -------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Anything without a remote scheme. [`local`] still refuses one that
    /// isn't an absolute path.
    Local,
    /// Google Drive (`gdrive://`).
    Drive,
    /// A received share (`share://`).
    Share,
    /// An SSH server (`sftp://`).
    Sftp,
    /// An Arbesk account (`arbesk://`), read only.
    Arbesk,
}

/// The kind of place `path` names, from its scheme alone: a malformed
/// remote path still has its scheme's kind, so its own parser refuses it.
pub fn kind(path: &str) -> Kind {
    if arbesk::is_arbesk(path) {
        Kind::Arbesk
    } else if places_gdrive::is_cloud(path) {
        Kind::Drive
    } else if share::location::is_share(path) {
        Kind::Share
    } else if places_sftp::is_sftp(path) {
        Kind::Sftp
    } else {
        Kind::Local
    }
}

// ---- Refusals ----------------------------------------------------------------

/// `path` as a file on this computer, or why it isn't one.
/// This computer's own words come from its provider (`places_local::local`).
pub fn local(path: &str) -> Result<&Path, String> {
    match kind(path) {
        Kind::Local => places_local::local(path),
        Kind::Drive => Err(places_gdrive::service::unsupported()),
        Kind::Share => Err(share::provider::LOCAL_ONLY.into()),
        Kind::Sftp => Err(places_sftp::LOCAL_ONLY.into()),
        Kind::Arbesk => Err("This action is not available for Arbesk.".into()),
    }
}

/// Whether anything may be written at `path`: received shares, SSH servers
/// and Arbesk are read only.
pub fn writable(path: &str) -> Result<(), String> {
    match kind(path) {
        Kind::Sftp => Err(places_sftp::READ_ONLY.into()),
        Kind::Share => Err(share::provider::READ_ONLY.into()),
        Kind::Arbesk => Err("Arbesk is read only.".into()),
        Kind::Local | Kind::Drive => Ok(()),
    }
}

/// [`writable`] for both sides of a copy, move or rename; `from` first.
pub fn writable_both(from: &str, to: &str) -> Result<(), String> {
    writable(from).and_then(|()| writable(to))
}

/// `path` as a file on this computer that may be written: the read-only
/// refusal comes first, then [`local`]'s.
pub fn local_write(path: &str) -> Result<&Path, String> {
    writable(path)?;
    local(path)
}

// ---- Copy, move and rename ---------------------------------------------------

/// Where a copy, move or rename runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// Through Drive: at least one side is in Drive.
    Drive,
    /// On this computer: both sides are local paths.
    Local,
    /// Into an Arbesk collection: only a copy of a local model file
    /// ([`copy`] alone answers it).
    IntoArbesk,
}

/// Where a copy, move or rename from `from` to `to` runs, or why it can't:
/// both sides must be [`writable`] (`from` first), and a side outside Drive
/// must be a [`local`] path, so Drive's uploads and downloads don't get a
/// relative one either.
pub fn transfer(from: &str, to: &str) -> Result<Route, String> {
    writable_both(from, to)?;
    let in_drive = |path: &str| kind(path) == Kind::Drive;
    for side in [from, to] {
        if !in_drive(side) {
            local(side)?;
        }
    }
    Ok(if in_drive(from) || in_drive(to) { Route::Drive } else { Route::Local })
}

/// Where a copy runs, and whether its source is an Arbesk asset to build first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CopyRoute {
    pub route: Route,
    /// The source is an Arbesk asset: its file is built in the app cache and
    /// copied from there (`Client::export`).
    pub from_arbesk: bool,
}

/// [`transfer`] for a copy, which may also take an Arbesk asset out to this
/// computer or Drive, or a local file into an Arbesk collection. A move never
/// does: Arbesk is read only. Collections can't be copied yet.
pub fn copy(from: &str, to: &str) -> Result<CopyRoute, String> {
    if kind(to) == Kind::Arbesk && kind(from) != Kind::Arbesk {
        return into_arbesk(from, to);
    }
    if kind(from) != Kind::Arbesk {
        return transfer(from, to).map(|route| CopyRoute { route, from_arbesk: false });
    }
    writable(to)?;
    if !matches!(arbesk::parse(from)?, arbesk::Location::Asset { .. }) {
        return Err("Copying Arbesk collections comes next.".into());
    }
    let route = if kind(to) == Kind::Drive {
        Route::Drive
    } else {
        local(to)?;
        Route::Local
    };
    Ok(CopyRoute { route, from_arbesk: true })
}

/// A copy of the local file `from` to the pending name `to` in an Arbesk
/// collection. What the file is, is checked where it is read.
fn into_arbesk(from: &str, to: &str) -> Result<CopyRoute, String> {
    if kind(from) != Kind::Local {
        return Err("Only .gltf, .glb and .3mf files on this computer can be copied to Arbesk.".into());
    }
    local(from)?;
    arbesk::parse_target(to)?;
    Ok(CopyRoute { route: Route::IntoArbesk, from_arbesk: false })
}

/// Every path as a file on this computer, or the first refusal.
pub fn all_local(paths: &[String]) -> Result<Vec<PathBuf>, String> {
    paths
        .iter()
        .map(|path| local(path).map(Path::to_path_buf))
        .collect()
}

// ---- Batches -----------------------------------------------------------------

/// One answer per path where only local paths can have one: `answer` gets
/// the local paths, in order, in a single call; every other path gets
/// `None`.
pub fn local_only<T>(
    paths: &[String],
    answer: impl FnOnce(Vec<String>) -> Vec<Option<T>>,
) -> Vec<Option<T>> {
    let (positions, local): (Vec<usize>, Vec<String>) = paths
        .iter()
        .enumerate()
        .filter(|(_, path)| local(path).is_ok())
        .map(|(index, path)| (index, path.clone()))
        .unzip();
    let mut out: Vec<Option<T>> = paths.iter().map(|_| None).collect();
    for (index, value) in positions.into_iter().zip(answer(local)) {
        out[index] = value;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRIVE_UNSUPPORTED: &str = "This action is not available for Google Drive items yet.";
    const SHARE_LOCAL: &str = "This action is not available for shared folders.";
    const SFTP_LOCAL: &str = "This action is not available for SSH servers.";
    const SHARE_READ_ONLY: &str = "Shared folders are read only.";
    const SFTP_READ_ONLY: &str = "SSH servers are read only for now.";
    const ARBESK_LOCAL: &str = "This action is not available for Arbesk.";
    const ARBESK_READ_ONLY: &str = "Arbesk is read only.";
    const ARBESK_COLLECTION: &str = "Copying Arbesk collections comes next.";
    const ARBESK: &str = "arbesk://0x00000000000000000000000000000000000000ab/12~Set";

    /// Each path: its kind, `local`'s answer (None: the path itself),
    /// `writable`'s refusal.
    fn table() -> Vec<(&'static str, Kind, Option<String>, Option<&'static str>)> {
        let not_local = |p: &str| Some(format!("“{p}” is not a local file."));
        vec![
            ("/Users/me/a.txt", Kind::Local, None, None),
            ("/", Kind::Local, None, None),
            ("/Users/me/gdrive://x", Kind::Local, None, None),
            ("Documents/a.txt", Kind::Local, not_local("Documents/a.txt"), None),
            ("", Kind::Local, not_local(""), None),
            ("pseudo:recent", Kind::Local, not_local("pseudo:recent"), None),
            ("file:///Users/me", Kind::Local, not_local("file:///Users/me"), None),
            ("smb://host/share", Kind::Local, not_local("smb://host/share"), None),
            // Near-misses of the remote schemes are no scheme at all.
            ("GDRIVE://acct/root~x", Kind::Local, not_local("GDRIVE://acct/root~x"), None),
            ("gdrive:/acct", Kind::Local, not_local("gdrive:/acct"), None),
            ("share:/r1", Kind::Local, not_local("share:/r1"), None),
            ("sftp:/me@host", Kind::Local, not_local("sftp:/me@host"), None),
            ("gdrive://acct/root~My%20Drive", Kind::Drive, Some(DRIVE_UNSUPPORTED.into()), None),
            ("gdrive://acct/root~x/~New%20Folder", Kind::Drive, Some(DRIVE_UNSUPPORTED.into()), None),
            ("gdrive://", Kind::Drive, Some(DRIVE_UNSUPPORTED.into()), None),
            ("gdrive://../root~x", Kind::Drive, Some(DRIVE_UNSUPPORTED.into()), None),
            ("share://r1/Photos", Kind::Share, Some(SHARE_LOCAL.into()), Some(SHARE_READ_ONLY)),
            ("share://", Kind::Share, Some(SHARE_LOCAL.into()), Some(SHARE_READ_ONLY)),
            ("share://r1/%2E%2E", Kind::Share, Some(SHARE_LOCAL.into()), Some(SHARE_READ_ONLY)),
            ("sftp://me@host/home", Kind::Sftp, Some(SFTP_LOCAL.into()), Some(SFTP_READ_ONLY)),
            ("sftp://", Kind::Sftp, Some(SFTP_LOCAL.into()), Some(SFTP_READ_ONLY)),
            (ARBESK, Kind::Arbesk, Some(ARBESK_LOCAL.into()), Some(ARBESK_READ_ONLY)),
            ("arbesk://", Kind::Arbesk, Some(ARBESK_LOCAL.into()), Some(ARBESK_READ_ONLY)),
            ("arbesk:/0xab", Kind::Local, not_local("arbesk:/0xab"), None),
        ]
    }

    #[test]
    fn each_path_has_a_kind_a_local_answer_and_a_write_answer() {
        for (path, expected_kind, local_refusal, write_refusal) in table() {
            assert_eq!(kind(path), expected_kind, "{path}");
            match &local_refusal {
                None => assert_eq!(local(path), Ok(Path::new(path)), "{path}"),
                Some(refusal) => assert_eq!(local(path).unwrap_err(), *refusal, "{path}"),
            }
            assert_eq!(writable(path).err().as_deref(), write_refusal, "{path}");
            let expected_write = write_refusal.map(String::from).or(local_refusal);
            assert_eq!(local_write(path).err(), expected_write, "{path}");
        }
    }

    /// The local-only gate's words are what each provider will give as its
    /// `Operation::LocalOnly` refusal (D10), unchanged.
    #[test]
    fn the_local_only_gate_fits_each_providers_refusal() {
        let cases = [
            ("gdrive://acct/root~x", DRIVE_UNSUPPORTED),
            ("share://r1/Photos", SHARE_LOCAL),
            ("sftp://me@host/home", SFTP_LOCAL),
            (ARBESK, ARBESK_LOCAL),
        ];
        for (path, words) in cases {
            let refused = places::Refused {
                operation: places::Operation::LocalOnly,
                message: local(path).unwrap_err(),
            };
            assert_eq!(refused.message, words, "{path}");
            let json = serde_json::to_value(&refused).unwrap();
            assert_eq!(json["operation"], "local_only", "{path}");
        }
    }

    #[test]
    fn malformed_remote_paths_are_refused_by_their_own_parser() {
        // `local` keeps them from std::fs; the handler they go to refuses them.
        for path in ["gdrive://", "gdrive://../root~x", "gdrive://acct/root~bad%zz"] {
            assert_eq!(kind(path), Kind::Drive);
            assert!(places_gdrive::parse(path).is_err(), "{path}");
        }
        for path in ["share://", "share://r1/%2E%2E", "share://r1//x"] {
            assert_eq!(kind(path), Kind::Share);
            assert!(share::location::ShareLocation::parse(path).is_none(), "{path}");
        }
        for path in ["sftp://", "sftp://host/home", "sftp://me@/home", "sftp://me@host/%FF"] {
            assert_eq!(kind(path), Kind::Sftp);
            assert!(places_sftp::parse_target(path).is_err(), "{path}");
        }
        for path in ["arbesk://", "arbesk://0xab", "arbesk://0x00000000000000000000000000000000000000ab/x~Set"] {
            assert_eq!(kind(path), Kind::Arbesk);
            assert!(arbesk::parse(path).is_err(), "{path}");
        }
    }

    #[test]
    fn a_write_needs_both_sides_writable() {
        let local = "/Users/me/a.txt";
        assert_eq!(writable_both(local, "/Users/me/b.txt"), Ok(()));
        assert_eq!(writable_both(local, "gdrive://acct/root~x/~b.txt"), Ok(()));
        assert_eq!(writable_both("share://r1/a", local).unwrap_err(), SHARE_READ_ONLY);
        assert_eq!(writable_both(local, "sftp://me@host/b").unwrap_err(), SFTP_READ_ONLY);
        assert_eq!(writable_both("sftp://me@host/a", "share://r1/b").unwrap_err(), SFTP_READ_ONLY);
    }

    #[test]
    fn a_transfer_runs_through_drive_or_between_local_paths() {
        let (a, b) = ("/Users/me/a.txt", "/Users/me/b.txt");
        let drive = "gdrive://acct/root~x/~b.txt";
        assert_eq!(transfer(a, b), Ok(Route::Local));
        assert_eq!(transfer(a, drive), Ok(Route::Drive));
        assert_eq!(transfer(drive, a), Ok(Route::Drive));
        assert_eq!(transfer(drive, "gdrive://acct/root~y"), Ok(Route::Drive));
        // The read-only refusals come first, `from`'s before `to`'s.
        assert_eq!(transfer("share://r1/a", "rel").unwrap_err(), SHARE_READ_ONLY);
        assert_eq!(transfer("rel", "sftp://me@host/b").unwrap_err(), SFTP_READ_ONLY);
        assert_eq!(transfer(ARBESK, a).unwrap_err(), ARBESK_READ_ONLY);
        assert_eq!(transfer(a, ARBESK).unwrap_err(), ARBESK_READ_ONLY);
        // A side outside Drive is a path on this computer, whichever way it goes.
        for (from, to, bad) in [
            ("rel", b, "rel"),
            (a, "rel/b.txt", "rel/b.txt"),
            ("a.txt", "b.txt", "a.txt"),
            (drive, "b.txt", "b.txt"),
            ("a.txt", drive, "a.txt"),
            ("", b, ""),
        ] {
            assert_eq!(transfer(from, to).unwrap_err(), format!("“{bad}” is not a local file."), "{from} -> {to}");
        }
    }

    #[test]
    fn a_copy_may_take_an_arbesk_asset_out_to_this_computer_or_drive() {
        let (a, b) = ("/Users/me/a.txt", "/Users/me/b.txt");
        let drive = "gdrive://acct/root~x/~Box.gltf";
        let asset = "arbesk://0x00000000000000000000000000000000000000ab/12~Set/a1~Box.gltf";
        let plain = |route| Ok(CopyRoute { route, from_arbesk: false });
        let built = |route| Ok(CopyRoute { route, from_arbesk: true });
        assert_eq!(copy(a, b), plain(Route::Local));
        assert_eq!(copy(a, drive), plain(Route::Drive));
        assert_eq!(copy(drive, a), plain(Route::Drive));
        assert_eq!(copy(asset, "/Users/me/Box.gltf"), built(Route::Local));
        assert_eq!(copy(asset, drive), built(Route::Drive));
        // Anything else stays as `transfer` answers, and only a copy takes an asset out.
        for (from, to, refusal) in [
            (asset, asset, ARBESK_READ_ONLY.to_string()),
            (asset, "share://r1/b", SHARE_READ_ONLY.into()),
            (asset, "sftp://me@host/b", SFTP_READ_ONLY.into()),
            (asset, "rel/Box.gltf", "“rel/Box.gltf” is not a local file.".into()),
            (ARBESK, b, ARBESK_COLLECTION.into()),
            ("arbesk://0x00000000000000000000000000000000000000ab", b, ARBESK_COLLECTION.into()),
            ("share://r1/a", b, SHARE_READ_ONLY.into()),
        ] {
            assert_eq!(copy(from, to).unwrap_err(), refusal, "{from} -> {to}");
        }
        assert!(copy("arbesk://0xab", b).is_err());
        assert_eq!(transfer(a, &format!("{ARBESK}/~Box.gltf")).unwrap_err(), ARBESK_READ_ONLY);
        assert_eq!(transfer(asset, b).unwrap_err(), ARBESK_READ_ONLY);
    }

    #[test]
    fn a_copy_may_take_a_local_file_into_an_arbesk_collection() {
        let (a, pending) = ("/Users/me/Box.glb", format!("{ARBESK}/~Box.glb"));
        let account = "arbesk://0x00000000000000000000000000000000000000ab";
        assert_eq!(copy(a, &pending), Ok(CopyRoute { route: Route::IntoArbesk, from_arbesk: false }));
        let only = "Only .gltf, .glb and .3mf files on this computer can be copied to Arbesk.";
        let paste = "Paste into a collection.";
        for (from, to, refusal) in [
            (a, ARBESK.to_string(), paste),
            (a, format!("{account}/~Box.glb"), paste),
            ("gdrive://acct/root~x", pending.clone(), only),
            ("share://r1/a", pending.clone(), only),
            ("sftp://me@host/a", pending.clone(), only),
        ] {
            assert_eq!(copy(from, &to).unwrap_err(), refusal, "{from} -> {to}");
        }
        assert_eq!(copy("rel/Box.glb", &pending).unwrap_err(), "“rel/Box.glb” is not a local file.");
        assert_eq!(transfer(a, &pending).unwrap_err(), ARBESK_READ_ONLY);
    }

    #[test]
    fn all_local_refuses_a_batch_with_any_other_path() {
        let paths = |list: &[&str]| list.iter().map(|p| p.to_string()).collect::<Vec<_>>();
        assert_eq!(
            all_local(&paths(&["/a", "/b c"])),
            Ok(vec![PathBuf::from("/a"), PathBuf::from("/b c")])
        );
        assert_eq!(all_local(&[]), Ok(vec![]));
        assert_eq!(all_local(&paths(&["/a", "share://r1/x"])).unwrap_err(), SHARE_LOCAL);
        assert_eq!(all_local(&paths(&["/a", "relative"])).unwrap_err(), "“relative” is not a local file.");
    }

    #[test]
    fn local_only_answers_local_paths_in_one_call_and_others_with_none() {
        let paths: Vec<String> = ["/a", "gdrive://acct/root~x", "/b", "share://r1", "sftp://me@h/", "rel", "/c", ARBESK]
            .map(String::from)
            .to_vec();
        let mut calls = 0;
        let out = local_only(&paths, |local| {
            calls += 1;
            assert_eq!(local, ["/a", "/b", "/c"]);
            local.into_iter().map(|p| (p != "/b").then(|| p.len())).collect()
        });
        assert_eq!(calls, 1);
        assert_eq!(out, vec![Some(2), None, None, None, None, None, Some(2), None]);
        let none: Vec<Option<u8>> = local_only(&[], |local| {
            assert!(local.is_empty());
            Vec::new()
        });
        assert!(none.is_empty());
    }
}
