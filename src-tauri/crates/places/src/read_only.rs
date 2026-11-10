//! What every read-only place (an SSH server, a received share) answers
//! alike, so a change to how such a place refuses is made once.

/// The [`Provider`](crate::Provider) methods a read-only place answers
/// alike, for a place refusing writes with `$read_only`'s words and the
/// features of items on this computer with `$local_only`'s: its
/// capabilities ([`Capabilities::read_only_place`](crate::Capabilities::read_only_place)),
/// `stat` and `lookup` refused as local only (nothing is pasted there, so
/// nothing is looked up), `exists` false, no path info, and every write
/// (`create`, `rename`, `trash`, `restore`) refused as read only. The place
/// writes `scheme`, `list` and `fetch` itself; its `fetch` refuses the
/// preview with [`refuse_preview`](crate::refuse_preview).
#[macro_export]
macro_rules! read_only_place {
    ($read_only:expr, $local_only:expr) => {
        fn capabilities(&self, _: &str) -> $crate::Capabilities {
            $crate::Capabilities::read_only_place($read_only, $local_only)
        }
        /// Nothing is pasted in a read-only place, so nothing is looked up.
        fn stat(&self, _: &str, _: &$crate::Cx) -> $crate::Result<Option<$crate::Entry>> {
            Err($crate::Error::Unsupported($local_only.into()))
        }
        fn lookup(&self, _: &str, _: &$crate::Cx) -> $crate::Result<Vec<$crate::Entry>> {
            Err($crate::Error::Unsupported($local_only.into()))
        }
        fn exists(&self, _: &str, _: &$crate::Cx) -> bool {
            false
        }
        fn path_info(&self, paths: &[String], _: &$crate::Cx) -> Vec<Option<$crate::PathInfo>> {
            paths.iter().map(|_| None).collect()
        }
        fn create(&self, _: &str, _: &str, _: $crate::NewKind, _: &$crate::Cx) -> $crate::Result<String> {
            Err($crate::Error::Unsupported($read_only.into()))
        }
        fn rename(&self, _: &str, _: &str, _: &$crate::Cx) -> $crate::Result<String> {
            Err($crate::Error::Unsupported($read_only.into()))
        }
        fn trash(&self, paths: &[&str], _: &$crate::Cx) -> Vec<$crate::Trashed> {
            let refused = |path: &&str| $crate::Trashed {
                path: path.to_string(),
                result: Err($crate::Error::Unsupported($read_only.into())),
            };
            paths.iter().map(refused).collect()
        }
        fn restore(&self, _: &str, _: &$crate::Cx) -> $crate::Result<String> {
            Err($crate::Error::Unsupported($read_only.into()))
        }
    };
}

/// A read-only place's preview refusal: its file isn't on this computer,
/// so the preview refuses it as any remote item, with `local_only`'s words.
/// `None` for any other purpose, which the place fetches itself.
pub fn refuse_preview(purpose: crate::Fetch, local_only: &str) -> Option<crate::Error> {
    (purpose == crate::Fetch::Preview).then(|| crate::Error::Unsupported(local_only.into()))
}

#[cfg(test)]
mod tests {
    use crate::*;

    const READ_ONLY: &str = "Read only here.";
    const LOCAL_ONLY: &str = "Not available here.";

    struct Place;

    impl Provider for Place {
        fn scheme(&self) -> &'static str {
            "test"
        }
        fn list(&self, _: &str, _: &ListOptions, _: &Cx) -> Result<Vec<Entry>> {
            Ok(Vec::new())
        }
        fn fetch(&self, _: &str, purpose: Fetch, _: &Cx) -> Result<Fetched> {
            match refuse_preview(purpose, LOCAL_ONLY) {
                Some(refused) => Err(refused),
                None => Ok(Fetched::Url("https://example.org/".into())),
            }
        }
        read_only_place!(READ_ONLY, LOCAL_ONLY);
    }

    #[test]
    fn a_read_only_place_refuses_every_write_and_looks_nothing_up() {
        let (place, cx) = (Place, Cx::default());
        let read_only = Error::Unsupported(READ_ONLY.into());
        let local_only = Error::Unsupported(LOCAL_ONLY.into());
        assert_eq!(place.capabilities("test://a"), Capabilities::read_only_place(READ_ONLY, LOCAL_ONLY));
        assert_eq!(place.stat("test://a", &cx).unwrap_err(), local_only);
        assert_eq!(place.lookup("test://~a", &cx).unwrap_err(), local_only);
        assert!(!place.exists("test://a", &cx));
        assert_eq!(place.path_info(&["test://a".into(), "test://".into()], &cx), vec![None, None]);
        assert_eq!(place.create("test://a", "b", NewKind::Folder, &cx).unwrap_err(), read_only);
        assert_eq!(place.rename("test://a", "b", &cx).unwrap_err(), read_only);
        assert_eq!(place.restore("test://a", &cx).unwrap_err(), read_only);
        let trashed = place.trash(&["test://a", "test://b"], &cx);
        assert_eq!(trashed.iter().map(|t| t.path.as_str()).collect::<Vec<_>>(), ["test://a", "test://b"]);
        assert!(trashed.iter().all(|t| t.result == Err(read_only.clone())));
        assert_eq!(place.fetch("test://a", Fetch::Preview, &cx).unwrap_err(), local_only);
        assert!(place.fetch("test://a", Fetch::Open, &cx).is_ok());
    }
}
