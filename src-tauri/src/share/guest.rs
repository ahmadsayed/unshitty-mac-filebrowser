//! The recipient's side (spec D5, D8, D10, D11, D13): claim a link, browse
//! a received share by walking its manifests, and download one file.
//!
//! Every name from a sharer's manifest is checked with `safe_name` before it
//! is shown; nothing here writes a sharer's name to disk (the caller picks
//! the staging path a download goes to).

use super::host::INVALID;
use super::link::ShareLink;
use super::location::{ShareLocation, safe_name};
use super::manifest::{Kind, MAX_MANIFEST, Manifest, ManifestEntry};
use super::proto::{self, Reply, Request, read_within, write_msg};
use super::registry::Received;
use crate::fs_ops::Entry;
use iroh::address_lookup::memory::MemoryLookup;
use iroh::endpoint::{
    ConnectError, ConnectingError, Connection, ConnectionError, TransportErrorCode,
};
use iroh::{EndpointAddr, EndpointId};
use iroh_blobs::Hash;
use iroh_blobs::api::downloader::Downloader;
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::util::connection_pool;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// How long reaching the sharer may take before it counts as offline (D13).
#[cfg(not(test))]
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(test)]
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);

/// How long the sharer may take to hash a file for `Content`.
const CONTENT_WAIT: Duration = Duration::from_secs(10 * 60);

/// TLS alert 120, `no_application_protocol`: the sharer speaks another ALPN.
const NO_APPLICATION_PROTOCOL: u8 = 120;

const CHANGED: &str = "The file changed while downloading. Try again.";
const MISSING: &str = "The shared folder was moved or deleted.";
const UNEXPECTED: &str = "The other side sent an unexpected reply.";
const BAD_MANIFEST: &str = "The shared folder's listing couldn't be read.";
const NOT_A_FILE: &str = "Only files can be downloaded.";
const EXISTS: &str = "A file with that name already exists.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Online,
    Offline,
    #[serde(rename = "no-access")]
    NoAccess,
    Missing,
}

/// Why a request to a sharer failed, before it is put into words.
#[derive(Debug)]
enum Fail {
    Offline,
    Version,
    Busy,
    Denied(String),
    Missing,
    Changed,
    /// A name on the way no longer exists.
    Gone(String),
    Other(String),
}

/// The words for a failure; `who` is the sharer's name.
fn describe(fail: Fail, who: &str) -> String {
    match fail {
        Fail::Offline => format!("{who} is offline."),
        Fail::Version => format!(
            "{who}'s Files app uses a different sharing version. \
             Both of you need the same version."
        ),
        Fail::Busy => format!("{who}'s computer is busy. Try again in a moment."),
        Fail::Denied(why) if why == INVALID => {
            format!("{who} stopped sharing this folder with you.")
        }
        Fail::Denied(why) | Fail::Other(why) => why,
        Fail::Missing => MISSING.into(),
        Fail::Changed => CHANGED.into(),
        Fail::Gone(name) => format!("“{name}” no longer exists."),
    }
}

/// A reply that isn't the one asked for.
fn refusal(reply: Reply) -> Fail {
    match reply {
        Reply::Missing => Fail::Missing,
        Reply::Denied(why) => Fail::Denied(why),
        Reply::Busy => Fail::Busy,
        Reply::Changed => Fail::Changed,
        _ => Fail::Other(UNEXPECTED.into()),
    }
}

/// The sharer's app refused our control protocol version.
fn refused_alpn(e: &ConnectError) -> bool {
    let closed = match e {
        ConnectError::Connecting {
            source: ConnectingError::ConnectionError { source, .. },
            ..
        } => source,
        ConnectError::Connection { source, .. } => source,
        _ => return false,
    };
    let alert = TransportErrorCode::crypto(NO_APPLICATION_PROTOCOL);
    matches!(closed, ConnectionError::ConnectionClosed(c) if c.error_code == alert)
}

fn connect_fail(e: &ConnectError) -> Fail {
    match refused_alpn(e) {
        true => Fail::Version,
        false => Fail::Offline,
    }
}

/// One request and its reply on a fresh stream.
async fn exchange(conn: &Connection, req: &Request, wait: Duration) -> Result<Reply, Fail> {
    let (mut send, mut recv) = conn.open_bi().await.map_err(|_| Fail::Offline)?;
    write_msg(&mut send, req).await.map_err(|_| Fail::Offline)?;
    send.finish().map_err(|_| Fail::Offline)?;
    read_within(&mut recv, wait)
        .await
        .map_err(|_| Fail::Offline)
}

/// A listing entry for a manifest entry, unless its name isn't safe.
fn to_entry(loc: &ShareLocation, e: &ManifestEntry) -> Option<Entry> {
    let name = safe_name(&e.name)?;
    Some(Entry {
        name: name.to_string(),
        path: loc.child(name).to_path(),
        is_dir: e.kind == Kind::Folder,
        size: e.size,
        modified: e.mtime.div_euclid(1_000_000_000),
        is_hidden: name.starts_with('.'),
        is_symlink: false,
        cloud: None,
    })
}

/// Remove the folder if it exists, then create it empty.
fn empty_dir(dir: &Path) -> Result<(), String> {
    match std::fs::remove_dir_all(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
        _ => {}
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct Guest {
    endpoint: iroh::Endpoint,
    store: FsStore,
    downloader: Downloader,
    lookup: MemoryLookup,
    /// At most one control connection per sharer.
    conns: Mutex<HashMap<EndpointId, Connection>>,
    manifests: Mutex<HashMap<Hash, Arc<Manifest>>>,
    /// The sharer's name from the latest `Root` reply, by received id.
    sharers: Mutex<HashMap<String, String>>,
}

impl Guest {
    /// The blob store lives in `cache_dir/share-store`, emptied here (D11).
    pub async fn new(endpoint: iroh::Endpoint, cache_dir: PathBuf) -> Result<Guest, String> {
        let dir = cache_dir.join("share-store");
        empty_dir(&dir)?;
        let store = FsStore::load(&dir).await.map_err(|e| e.to_string())?;
        let lookup = MemoryLookup::new();
        let lookups = endpoint.address_lookup().map_err(|e| e.to_string())?;
        lookups.add(lookup.clone());
        let pool = connection_pool::Options {
            connect_timeout: CONNECT_TIMEOUT,
            ..Default::default()
        };
        let downloader = Downloader::new_with_opts(&store, &endpoint, pool);
        Ok(Guest {
            endpoint,
            store,
            downloader,
            lookup,
            conns: Mutex::default(),
            manifests: Mutex::default(),
            sharers: Mutex::default(),
        })
    }

    /// Claim the link; returns the folder's and the sharer's names.
    pub async fn claim(&self, link: &ShareLink) -> Result<(String, String), String> {
        let req = Request::Claim {
            share: link.share,
            token: link.token,
        };
        let fail = match self.ask(&link.addr, &req, CONNECT_TIMEOUT).await {
            Ok(Reply::Ok { folder, sharer }) => return Ok((folder, sharer)),
            Ok(Reply::Denied(why)) => return Err(why),
            Ok(other) => refusal(other),
            Err(fail) => fail,
        };
        Err(describe(fail, &link.sharer))
    }

    pub async fn status(&self, r: &Received) -> Status {
        match self.root(r).await {
            Ok(_) | Err(Fail::Busy) => Status::Online,
            Err(Fail::Missing) => Status::Missing,
            Err(Fail::Denied(_)) => Status::NoAccess,
            Err(_) => Status::Offline,
        }
    }

    pub async fn list(
        &self,
        r: &Received,
        loc: &ShareLocation,
        show_hidden: bool,
    ) -> Result<Vec<Entry>, String> {
        let found = self.browse(r, loc).await.map_err(|f| self.say(r, f))?;
        let entries = found.entries.iter().filter_map(|e| to_entry(loc, e));
        Ok(entries.filter(|e| show_hidden || !e.is_hidden).collect())
    }

    /// Download the file at `loc` to `dest`, which must not exist yet. On
    /// failure nothing is left at `dest`.
    pub async fn fetch(
        &self,
        r: &Received,
        loc: &ShareLocation,
        dest: &Path,
    ) -> Result<(), String> {
        if dest.symlink_metadata().is_ok() {
            return Err(EXISTS.into());
        }
        self.download(r, loc, dest)
            .await
            .map_err(|f| self.say(r, f))
    }

    // ---- requests

    fn say(&self, r: &Received, fail: Fail) -> String {
        let latest = lock(&self.sharers).get(&r.id).cloned();
        describe(fail, latest.as_deref().unwrap_or(&r.sharer))
    }

    /// The cached control connection to the sharer, or a new one; `true`
    /// when it was cached.
    async fn connection(&self, addr: &EndpointAddr) -> Result<(Connection, bool), Fail> {
        let cached = lock(&self.conns).get(&addr.id).cloned();
        if let Some(c) = cached.filter(|c| c.close_reason().is_none()) {
            return Ok((c, true));
        }
        self.lookup.add_endpoint_info(addr.clone());
        let conn = self.endpoint.connect(addr.clone(), proto::ALPN).await;
        let conn = conn.map_err(|e| connect_fail(&e))?;
        lock(&self.conns).insert(addr.id, conn.clone());
        Ok((conn, false))
    }

    /// Ask the sharer, giving up (as offline) after `wait`.
    async fn ask(&self, addr: &EndpointAddr, req: &Request, wait: Duration) -> Result<Reply, Fail> {
        let asked = tokio::time::timeout(wait, self.ask_now(addr, req, wait)).await;
        let reply = asked.unwrap_or(Err(Fail::Offline));
        if matches!(reply, Err(Fail::Offline)) {
            lock(&self.conns).remove(&addr.id);
        }
        reply
    }

    /// A cached connection that fails is dialled again once.
    async fn ask_now(
        &self,
        addr: &EndpointAddr,
        req: &Request,
        wait: Duration,
    ) -> Result<Reply, Fail> {
        let (conn, cached) = self.connection(addr).await?;
        match exchange(&conn, req, wait).await {
            Err(_) if cached => {
                lock(&self.conns).remove(&addr.id);
                let (conn, _) = self.connection(addr).await?;
                exchange(&conn, req, wait).await
            }
            reply => reply,
        }
    }

    /// The share's current root, noting the sharer's current name.
    async fn root(&self, r: &Received) -> Result<Hash, Fail> {
        let req = Request::Root {
            share: r.link.share,
        };
        match self.ask(&r.link.addr, &req, CONNECT_TIMEOUT).await? {
            Reply::Root { root, sharer, .. } => {
                if !sharer.is_empty() {
                    lock(&self.sharers).insert(r.id.clone(), sharer);
                }
                Ok(root)
            }
            other => Err(refusal(other)),
        }
    }

    // ---- manifests and content

    /// A manifest by hash: from memory, or downloaded and decoded.
    async fn manifest(&self, addr: &EndpointAddr, hash: Hash) -> Result<Arc<Manifest>, Fail> {
        if let Some(m) = lock(&self.manifests).get(&hash).cloned() {
            return Ok(m);
        }
        let got = self.downloader.download(hash, Some(addr.id)).await;
        got.map_err(|_| Fail::Offline)?;
        let bytes = self.store.blobs().get_bytes(hash).await;
        let bytes = bytes.map_err(|_| Fail::Other(BAD_MANIFEST.into()))?;
        if bytes.len() > MAX_MANIFEST {
            return Err(Fail::Other(BAD_MANIFEST.into()));
        }
        let m = Arc::new(Manifest::decode(&bytes).map_err(|_| Fail::Other(BAD_MANIFEST.into()))?);
        lock(&self.manifests).insert(hash, m.clone());
        Ok(m)
    }

    /// The manifest of the folder `names` lead to from `root`.
    async fn folder_at(
        &self,
        addr: &EndpointAddr,
        root: Hash,
        names: &[String],
    ) -> Result<Arc<Manifest>, Fail> {
        let mut m = self.manifest(addr, root).await?;
        for name in names {
            let entry = m.entries.iter().find(|e| e.name == *name);
            let next = entry
                .filter(|_| safe_name(name).is_some())
                .and_then(|e| e.folder);
            m = self
                .manifest(addr, next.ok_or_else(|| Fail::Gone(name.clone()))?)
                .await?;
        }
        Ok(m)
    }

    /// The manifest of the folder at `loc`, from the current root.
    async fn browse(&self, r: &Received, loc: &ShareLocation) -> Result<Arc<Manifest>, Fail> {
        let root = self.root(r).await?;
        self.folder_at(&r.link.addr, root, &loc.names).await
    }

    /// Resolve the file from the current root, ask for its content, fetch it
    /// verified and export it to `dest`.
    async fn download(&self, r: &Received, loc: &ShareLocation, dest: &Path) -> Result<(), Fail> {
        let (name, parents) = loc
            .names
            .split_last()
            .ok_or(Fail::Other(NOT_A_FILE.into()))?;
        let addr = &r.link.addr;
        let root = self.root(r).await?;
        let folder = self.folder_at(addr, root, parents).await?;
        let found = folder.entries.iter().find(|e| e.name == *name);
        found
            .filter(|e| e.kind == Kind::File)
            .ok_or_else(|| Fail::Gone(name.clone()))?;
        let req = Request::Content {
            share: r.link.share,
            root,
            names: loc.names.clone(),
        };
        let hash = match self.ask(addr, &req, CONTENT_WAIT).await? {
            Reply::Content { hash, .. } => hash,
            other => return Err(refusal(other)),
        };
        if self.downloader.download(hash, Some(addr.id)).await.is_err() {
            // Still reachable: the sharer stopped serving it, so it changed.
            return Err(self.root(r).await.map_or_else(|f| f, |_| Fail::Changed));
        }
        self.export(hash, dest).await
    }

    /// Copy a complete blob out to `dest`, removing a partial copy.
    async fn export(&self, hash: Hash, dest: &Path) -> Result<(), Fail> {
        match self.store.blobs().export(hash, dest).await {
            Ok(_) => Ok(()),
            Err(e) => {
                std::fs::remove_file(dest).ok();
                Err(Fail::Other(e.to_string()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::share::host::Host;
    use crate::share::identity::load_or_create;
    use crate::share::link::{LinkKind, ShareId};
    use crate::share::manifest::{Kind, Manifest, ManifestEntry};
    use crate::share::registry::{AccessEntry, Registry};
    use crate::share::{test_endpoint, test_endpoint_for};
    use std::time::{Duration, Instant};

    const LIMIT: Duration = Duration::from_secs(20);

    async fn within<F: std::future::Future>(f: F) -> F::Output {
        tokio::time::timeout(LIMIT, f).await.expect("timed out")
    }

    /// A host whose sharer is called Ahmed, speaking `alpn`.
    async fn host_with(dir: &Path, alpn: Option<&'static [u8]>) -> Host {
        let config = dir.join("config");
        let identity = load_or_create(&config).unwrap();
        let registry = config.join("shares.json");
        if !registry.exists() {
            let named = Registry {
                name: Some("Ahmed".into()),
                ..Registry::default()
            };
            named.save(&registry).unwrap();
        }
        let ep = test_endpoint_for(identity.key.clone()).await;
        let store = dir.join("store");
        match alpn {
            None => Host::start(identity, registry, store, ep, Box::new(|_, _, _| None)).await,
            Some(alpn) => Host::start_with_alpn(identity, registry, store, ep, alpn).await,
        }
        .unwrap()
    }

    fn folder(dir: &Path, name: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let f = dir.join(name);
        for (n, body) in files {
            let p = f.join(n);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
        std::fs::create_dir_all(&f).unwrap();
        f
    }

    async fn guest(dir: &Path) -> Guest {
        Guest::new(test_endpoint().await, dir.join("guest-cache"))
            .await
            .unwrap()
    }

    async fn link(host: &Host, share: ShareId) -> (ShareLink, AccessEntry) {
        let (text, entry) = host
            .create_link(&share, LinkKind::Person, Some("Bob".into()))
            .await
            .unwrap();
        (ShareLink::decode(&text).unwrap(), entry)
    }

    fn received(link: ShareLink) -> Received {
        Received {
            id: "r1".into(),
            folder: "Photos".into(),
            sharer: link.sharer.clone(),
            link,
        }
    }

    /// A guest that claimed a fresh person link on the folder.
    struct Setup {
        _tmp: tempfile::TempDir,
        host: Host,
        share: ShareId,
        folder: PathBuf,
        guest: Guest,
        r: Received,
        entry: AccessEntry,
    }

    async fn setup(files: &[(&str, &[u8])]) -> Setup {
        let tmp = tempfile::tempdir().unwrap();
        let folder = folder(tmp.path(), "Photos", files);
        let host = host_with(tmp.path(), None).await;
        let share = host.share_folder(&folder, |_| false).await.unwrap();
        let (l, entry) = link(&host, share).await;
        let guest = guest(tmp.path()).await;
        let got = guest.claim(&l).await;
        assert_eq!(got, Ok(("Photos".to_string(), "Ahmed".to_string())));
        Setup {
            host,
            share,
            folder,
            guest,
            r: received(l),
            entry,
            _tmp: tmp,
        }
    }

    fn top() -> ShareLocation {
        ShareLocation::parse("share://r1").unwrap()
    }

    fn names(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    fn secs(p: &Path) -> i64 {
        let t = std::fs::metadata(p).unwrap().modified().unwrap();
        t.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
    }

    #[tokio::test]
    async fn listing_shows_the_shared_folder() {
        within(async {
            let s = setup(&[("a.txt", b"hi"), ("sub/b.txt", b"yo!")]).await;
            let list = s.guest.list(&s.r, &top(), false).await.unwrap();
            assert_eq!(names(&list), vec!["a.txt", "sub"]);
            let (a, sub) = (&list[0], &list[1]);
            assert_eq!((a.size, a.is_dir, a.is_hidden), (2, false, false));
            assert_eq!(a.path, "share://r1/a.txt");
            assert_eq!(a.modified, secs(&s.folder.join("a.txt")));
            assert!(!a.is_symlink && a.cloud.is_none());
            assert!(sub.is_dir);
            assert_eq!(sub.path, "share://r1/sub");

            let inside = ShareLocation::parse(&sub.path).unwrap();
            let list = s.guest.list(&s.r, &inside, false).await.unwrap();
            assert_eq!(names(&list), vec!["b.txt"]);
            assert_eq!(list[0].size, 3);
            assert_eq!(list[0].path, "share://r1/sub/b.txt");

            let gone = top().child("nope");
            assert_eq!(
                s.guest.list(&s.r, &gone, false).await.err(),
                Some("“nope” no longer exists.".to_string())
            );
            assert_eq!(s.guest.status(&s.r).await, Status::Online);
            s.host.shutdown().await;
        })
        .await
    }

    #[tokio::test]
    async fn hidden_entries_follow_show_hidden() {
        within(async {
            let s = setup(&[("a.txt", b"hi"), (".secret", b"psst")]).await;
            let shown = s.guest.list(&s.r, &top(), false).await.unwrap();
            assert_eq!(names(&shown), vec!["a.txt"]);
            let all = s.guest.list(&s.r, &top(), true).await.unwrap();
            assert_eq!(names(&all), vec![".secret", "a.txt"]);
            assert!(all[0].is_hidden);
            assert!(!all[1].is_hidden);
            s.host.shutdown().await;
        })
        .await
    }

    fn file_entry(name: &str) -> ManifestEntry {
        ManifestEntry {
            name: name.into(),
            kind: Kind::File,
            size: 1,
            mtime: 0,
            folder: None,
        }
    }

    #[tokio::test]
    async fn a_malicious_manifest_name_is_dropped() {
        within(async {
            let s = setup(&[("a.txt", b"hi")]).await;
            let evil = Manifest {
                entries: ["../evil", "..", ".", "", "a\0b", "ok"]
                    .iter()
                    .map(|n| file_entry(n))
                    .collect(),
            };
            s.host.publish_root(&s.share, evil).await;
            let list = s.guest.list(&s.r, &top(), true).await.unwrap();
            assert_eq!(names(&list), vec!["ok"]);
            let through = top().child("..");
            assert!(s.guest.list(&s.r, &through, true).await.is_err());
            s.host.shutdown().await;
        })
        .await
    }

    fn big_data() -> Vec<u8> {
        (0..3_000_000u32)
            .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
            .collect()
    }

    #[tokio::test]
    async fn fetch_writes_a_verified_copy() {
        within(async {
            let data = big_data();
            let s = setup(&[("big.bin", &data), ("a.txt", b"hi")]).await;
            let out = s._tmp.path().join("out");
            std::fs::create_dir_all(&out).unwrap();
            let dest = out.join(".big.bin.part");
            assert!(!dest.exists());
            let loc = top().child("big.bin");
            s.guest.fetch(&s.r, &loc, &dest).await.unwrap();
            assert_eq!(std::fs::read(&dest).unwrap(), data);

            let missing = out.join(".nope.part");
            let got = s.guest.fetch(&s.r, &top().child("nope"), &missing).await;
            assert_eq!(got, Err("“nope” no longer exists.".to_string()));
            assert!(!missing.exists());

            std::fs::write(s.folder.join("a.txt"), b"hello, changed").unwrap();
            let changed = out.join(".a.txt.part");
            let got = s.guest.fetch(&s.r, &top().child("a.txt"), &changed).await;
            assert_eq!(
                got,
                Err("The file changed while downloading. Try again.".to_string())
            );
            assert!(!changed.exists());
            s.host.shutdown().await;
        })
        .await
    }

    #[tokio::test]
    async fn a_stopped_host_is_offline_within_the_timeout() {
        within(async {
            let s = setup(&[("a.txt", b"hi")]).await;
            assert_eq!(s.guest.status(&s.r).await, Status::Online);
            s.host.shutdown().await;
            let started = Instant::now();
            assert_eq!(s.guest.status(&s.r).await, Status::Offline);
            assert!(started.elapsed() < Duration::from_secs(3));
            assert_eq!(
                s.guest.list(&s.r, &top(), false).await.err(),
                Some("Ahmed is offline.".to_string())
            );
        })
        .await
    }

    #[tokio::test]
    async fn a_revoked_guest_sees_no_access() {
        within(async {
            let s = setup(&[("a.txt", b"hi")]).await;
            assert_eq!(s.guest.status(&s.r).await, Status::Online);
            s.host.remove_access(&s.share, &s.entry.id).await.unwrap();
            assert_eq!(s.guest.status(&s.r).await, Status::NoAccess);
            let stopped = Some("Ahmed stopped sharing this folder with you.".to_string());
            assert_eq!(s.guest.list(&s.r, &top(), false).await.err(), stopped);
            s.host.shutdown().await;
        })
        .await
    }

    #[tokio::test]
    async fn a_vanished_folder_is_missing() {
        within(async {
            let s = setup(&[("a.txt", b"hi")]).await;
            std::fs::rename(&s.folder, s._tmp.path().join("Elsewhere")).unwrap();
            assert_eq!(s.guest.status(&s.r).await, Status::Missing);
            assert_eq!(
                s.guest.list(&s.r, &top(), false).await.err(),
                Some("The shared folder was moved or deleted.".to_string())
            );
            s.host.shutdown().await;
        })
        .await
    }

    #[tokio::test]
    async fn a_used_person_link_is_refused_on_another_computer() {
        within(async {
            let s = setup(&[("a.txt", b"hi")]).await;
            let (l, _) = link(&s.host, s.share).await;
            assert!(s.guest.claim(&l).await.is_ok());
            assert!(s.guest.claim(&l).await.is_ok());
            let other = guest(&s._tmp.path().join("other")).await;
            assert_eq!(
                other.claim(&l).await,
                Err("This link was already used on another computer.".to_string())
            );
            s.host.shutdown().await;
        })
        .await
    }

    #[tokio::test]
    async fn a_different_protocol_version_is_explained() {
        within(async {
            let tmp = tempfile::tempdir().unwrap();
            let f = folder(tmp.path(), "Photos", &[("a.txt", b"hi")]);
            let host = host_with(tmp.path(), Some(b"filebrowser/share/2")).await;
            let share = host.share_folder(&f, |_| false).await.unwrap();
            let (l, _) = link(&host, share).await;
            let g = guest(tmp.path()).await;
            let mismatch = "Ahmed's Files app uses a different sharing version. \
                            Both of you need the same version.";
            assert_eq!(g.claim(&l).await, Err(mismatch.to_string()));
            let r = received(l);
            assert_eq!(
                g.list(&r, &top(), false).await.err(),
                Some(mismatch.to_string())
            );
            assert_eq!(g.status(&r).await, Status::Offline);
            host.shutdown().await;
        })
        .await
    }

    #[tokio::test]
    async fn the_store_is_emptied_on_start() {
        within(async {
            let tmp = tempfile::tempdir().unwrap();
            let left = tmp.path().join("cache/share-store/leftover");
            std::fs::create_dir_all(left.parent().unwrap()).unwrap();
            std::fs::write(&left, b"old").unwrap();
            let _g = Guest::new(test_endpoint().await, tmp.path().join("cache"))
                .await
                .unwrap();
            assert!(!left.exists());
            assert!(tmp.path().join("cache/share-store").is_dir());
        })
        .await
    }

    #[test]
    fn status_serializes_like_the_frontend_expects() {
        let s = |v: Status| serde_json::to_string(&v).unwrap();
        assert_eq!(s(Status::Online), "\"online\"");
        assert_eq!(s(Status::Offline), "\"offline\"");
        assert_eq!(s(Status::NoAccess), "\"no-access\"");
        assert_eq!(s(Status::Missing), "\"missing\"");
    }
}
