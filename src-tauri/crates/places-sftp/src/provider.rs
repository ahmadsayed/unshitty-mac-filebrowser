//! SSH servers as a `places::Provider`: [`SftpState`] answers for every
//! `sftp://` path. Reads only: listing a connected server's folder and
//! opening a file (a download into the app cache, with progress); nothing
//! is looked up for a paste, the preview refuses a server's file as any
//! remote item, and every write is refused as read only.

use std::path::PathBuf;

use places::{
    Capabilities, Cx, Entry, Error, Fetch, Fetched, ListOptions, NewKind, PathInfo, Provider, Result, Trashed,
};

use crate::SftpState;

/// The scheme of SSH servers' paths, without "://".
pub const SCHEME: &str = "sftp";

/// Every write's refusal: nothing is written on a server.
pub const READ_ONLY: &str = "SSH servers are read only for now.";

/// The local-only gate's words (`place::local`), for the features only
/// items on this computer have; the preview and looking an item up refuse
/// with them too.
pub const LOCAL_ONLY: &str = "This action is not available for SSH servers.";

/// Today's errors are words; they keep them, as `Other`, so a command
/// returns exactly the same text.
fn words(message: String) -> Error {
    Error::Other(message)
}

fn read_only<T>() -> Result<T> {
    Err(Error::Unsupported(READ_ONLY.into()))
}

fn local_only<T>() -> Result<T> {
    Err(Error::Unsupported(LOCAL_ONLY.into()))
}

impl Provider for SftpState {
    fn scheme(&self) -> &'static str {
        SCHEME
    }
    fn capabilities(&self, _: &str) -> Capabilities {
        Capabilities::read_only_place(READ_ONLY, LOCAL_ONLY)
    }
    fn list(&self, dir: &str, opts: &ListOptions, _: &Cx) -> Result<Vec<Entry>> {
        SftpState::list(self, dir, opts.show_hidden).map_err(words)
    }
    /// Nothing is pasted on a server, so nothing is looked up there.
    fn stat(&self, _: &str, _: &Cx) -> Result<Option<Entry>> {
        local_only()
    }
    fn lookup(&self, _: &str, _: &Cx) -> Result<Vec<Entry>> {
        local_only()
    }
    /// Nothing is looked up on a server: nothing is pasted there.
    fn exists(&self, _: &str, _: &Cx) -> bool {
        false
    }
    fn path_info(&self, paths: &[String], _: &Cx) -> Vec<Option<PathInfo>> {
        paths.iter().map(|_| None).collect()
    }
    /// The file, downloaded into the app cache; the preview refuses it.
    fn fetch(&self, path: &str, purpose: Fetch, cx: &Cx) -> Result<Fetched> {
        if purpose == Fetch::Preview {
            return local_only();
        }
        let file = self.open(path, &cx.progress).map_err(words)?;
        Ok(Fetched::File(PathBuf::from(file)))
    }
    fn create(&self, _: &str, _: &str, _: NewKind, _: &Cx) -> Result<String> {
        read_only()
    }
    fn rename(&self, _: &str, _: &str, _: &Cx) -> Result<String> {
        read_only()
    }
    fn trash(&self, paths: &[&str], _: &Cx) -> Vec<Trashed> {
        paths.iter().map(|path| Trashed { path: path.to_string(), result: read_only() }).collect()
    }
    fn restore(&self, _: &str, _: &Cx) -> Result<String> {
        read_only()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use places::{Operation, Replace};

    fn state(dir: &std::path::Path) -> SftpState {
        SftpState::new(dir.join("known_hosts"), dir.into(), dir.join("sftp"))
    }

    #[test]
    fn a_server_is_read_only_with_its_own_words() {
        let dir = tempfile::tempdir().unwrap();
        let caps = state(dir.path()).capabilities("sftp://me@host/home");
        assert!(caps.read_only && caps.knows_sizes);
        assert!(!(caps.can_create || caps.can_rename || caps.can_trash || caps.can_copy_out));
        assert!(!(caps.knows_counts || caps.searchable || caps.has_thumbnails));
        assert_eq!((caps.rename_atomic, caps.replace, caps.names_unique, caps.put_is_whole), (false, Replace::Trash, true, false));
        let words = |operation| caps.refused.iter().find(|r| r.operation == operation).map(|r| r.message.as_str());
        for operation in [Operation::Create, Operation::Rename, Operation::Trash, Operation::CopyIn, Operation::CopyOut] {
            assert_eq!(words(operation), Some(READ_ONLY), "{operation:?}");
        }
        assert_eq!(words(Operation::LocalOnly), Some(LOCAL_ONLY));
    }

    #[test]
    fn every_write_is_refused_as_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let (sftp, cx) = (state(dir.path()), Cx::default());
        let refused = Error::Unsupported(READ_ONLY.into());
        assert_eq!(sftp.create("sftp://me@host/a", "b", NewKind::Folder, &cx).unwrap_err(), refused);
        assert_eq!(sftp.rename("sftp://me@host/a", "b", &cx).unwrap_err(), refused);
        assert_eq!(sftp.restore("sftp://me@host/a", &cx).unwrap_err(), refused);
        let trashed = sftp.trash(&["sftp://me@host/a"], &cx);
        assert_eq!(trashed, [Trashed { path: "sftp://me@host/a".into(), result: Err(refused) }]);
        assert!(places::with_transfer(&sftp, &cx, |_| ()).is_none());
    }

    #[test]
    fn reads_need_a_connection_and_the_preview_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let (sftp, cx) = (state(dir.path()), Cx::default());
        let first = Error::Other("Connect to this server first.".into());
        assert_eq!(Provider::list(&sftp, "sftp://me@host/home", &ListOptions::default(), &cx).unwrap_err(), first);
        assert_eq!(sftp.fetch("sftp://me@host/a.txt", Fetch::Open, &cx).unwrap_err(), first);
        let malformed = Provider::list(&sftp, "sftp://", &ListOptions::default(), &cx).unwrap_err();
        assert!(matches!(malformed, Error::Other(_)), "{malformed:?}");
        let preview = sftp.fetch("sftp://me@host/a.txt", Fetch::Preview, &cx).unwrap_err();
        assert_eq!(preview, Error::Unsupported(LOCAL_ONLY.into()));
        assert!(!sftp.exists("sftp://me@host/a.txt", &cx));
        assert_eq!(sftp.path_info(&["sftp://me@host/a".into(), "sftp://".into()], &cx), vec![None, None]);
        assert!(sftp.fetch_reports_progress());
        assert_eq!(sftp.scheme(), SCHEME);
    }
}
