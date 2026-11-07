//! Which kind of place a path names, and what the backend allows there: the
//! backend twin of `placeOf` (`src/paths.ts`) and `src/locations.ts`.
//!
//! A narrow gate, not a router. A command that works on this computer only
//! takes its path from [`local`] (or [`local_write`]), a write command asks
//! [`writable`] first, and a batch command with nothing to say about remote
//! paths answers through [`local_only`]. So a `gdrive://`, `share://` or
//! `sftp://` path, well formed or not, never reaches `std::fs`, and neither
//! does a relative path, which would be read from the app's working
//! directory. Commands that serve remote paths pick their handler with
//! [`kind`]; each handler parses its own scheme (`cloud::location`,
//! `share::location`, `sftp`) and refuses a malformed path.
//!
//! Plain functions, no Tauri.

use std::path::{Path, PathBuf};

use crate::{cloud, share, sftp};

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
}

/// The kind of place `path` names, from its scheme alone: a malformed
/// remote path still has its scheme's kind, so its own parser refuses it.
pub fn kind(path: &str) -> Kind {
    if cloud::is_cloud(path) {
        Kind::Drive
    } else if share::location::is_share(path) {
        Kind::Share
    } else if sftp::is_sftp(path) {
        Kind::Sftp
    } else {
        Kind::Local
    }
}

// ---- Refusals ----------------------------------------------------------------

/// `path` as a file on this computer, or why it isn't one.
pub fn local(path: &str) -> Result<&Path, String> {
    let local = Path::new(path);
    match kind(path) {
        Kind::Local if local.is_absolute() => Ok(local),
        Kind::Local => Err(format!("“{path}” is not a local file.")),
        Kind::Drive => Err(cloud::service::unsupported()),
        Kind::Share => Err("This action is not available for shared folders.".into()),
        Kind::Sftp => Err("This action is not available for SSH servers.".into()),
    }
}

/// Whether anything may be written at `path`: received shares and SSH
/// servers are read only.
pub fn writable(path: &str) -> Result<(), String> {
    match kind(path) {
        Kind::Sftp => Err("SSH servers are read only for now.".into()),
        Kind::Share => Err("Shared folders are read only.".into()),
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

    #[test]
    fn malformed_remote_paths_are_refused_by_their_own_parser() {
        // `local` keeps them from std::fs; the handler they go to refuses them.
        for path in ["gdrive://", "gdrive://../root~x", "gdrive://acct/root~bad%zz"] {
            assert_eq!(kind(path), Kind::Drive);
            assert!(cloud::parse(path).is_err(), "{path}");
        }
        for path in ["share://", "share://r1/%2E%2E", "share://r1//x"] {
            assert_eq!(kind(path), Kind::Share);
            assert!(share::location::ShareLocation::parse(path).is_none(), "{path}");
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
        let paths: Vec<String> = ["/a", "gdrive://acct/root~x", "/b", "share://r1", "sftp://me@h/", "rel", "/c"]
            .map(String::from)
            .to_vec();
        let mut calls = 0;
        let out = local_only(&paths, |local| {
            calls += 1;
            assert_eq!(local, ["/a", "/b", "/c"]);
            local.into_iter().map(|p| (p != "/b").then(|| p.len())).collect()
        });
        assert_eq!(calls, 1);
        assert_eq!(out, vec![Some(2), None, None, None, None, None, Some(2)]);
        let none: Vec<Option<u8>> = local_only(&[], |local| {
            assert!(local.is_empty());
            Vec::new()
        });
        assert!(none.is_empty());
    }
}
