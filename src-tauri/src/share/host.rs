//! The sharer's side (spec D5, D6, D7, D13): the control protocol, the
//! access gate in front of iroh-blobs, and serving files by reference.
//!
//! Everything a visitor may fetch is decided here. A blobs connection is
//! admitted only for a key with access to some share, and a get only for a
//! hash published by a share that key may open: one of its manifests, or a
//! file hash it returned from `Content` whose file is still unchanged.

use super::identity::Identity;
use super::link::{LinkKind, ShareId, ShareLink, Token};
use super::manifest::{ContentCache, Kind, Tree, stat};
use super::proto::{self, Reply, Request, read_msg, write_msg};
use super::registry::{AccessEntry, Claimed, Registry};
use iroh::PublicKey;
use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh_blobs::api::blobs::{AddPathOptions, ImportMode};
use iroh_blobs::provider::events::{
    AbortReason, ConnectMode, EventMask, EventSender, ObserveMode, ProviderMessage, RequestMode,
};
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::{BlobFormat, BlobsProtocol, Hash};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::watch;

/// Concurrent visitors (distinct keys with an open connection), D13.
pub const MAX_VISITORS: usize = 32;

const INVALID: &str = "This link is no longer valid.";
const NOT_FOUND: &str = "This item is no longer available.";
const KEY_MISMATCH: &str = "The sharing key doesn't match the network endpoint.";
const REVOKED: u32 = 1;

/// Keep Versions' hash for (path, size, mtime ns), when it has one.
pub type VersionsHint = Box<dyn Fn(&Path, u64, i64) -> Option<Hash> + Send + Sync>;

fn denied(why: &str) -> Reply {
    Reply::Denied(why.to_string())
}

fn unreadable(name: &str) -> Reply {
    Reply::Denied(format!("“{name}” can't be read."))
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())
}

// ---- state --------------------------------------------------------------

/// A file hash handed out by `Content`, with what the file looked like.
struct Published {
    path: PathBuf,
    size: u64,
    mtime: i64,
}

struct Served {
    tree: Tree,
    files: HashMap<Hash, Published>,
    root: watch::Sender<Hash>,
}

struct Conn {
    key: PublicKey,
    conn: Connection,
    /// Counted as a visitor (control: under the cap; blobs: passed the gate).
    admitted: bool,
}

/// A `Content` request resolved to a file of the sharer's own.
struct Job {
    share: ShareId,
    file: PathBuf,
    folder: PathBuf,
    name: String,
    size: u64,
    mtime: i64,
}

#[derive(Default)]
struct State {
    registry: Registry,
    served: HashMap<ShareId, Served>,
    conns: HashMap<u64, Conn>,
}

impl State {
    fn track(&mut self, id: u64, key: PublicKey, conn: Connection) {
        let admitted = false;
        self.conns.insert(
            id,
            Conn {
                key,
                conn,
                admitted,
            },
        );
    }

    fn online(&self) -> Vec<PublicKey> {
        let keys: HashSet<PublicKey> = self
            .conns
            .values()
            .filter(|c| c.admitted)
            .map(|c| c.key)
            .collect();
        keys.into_iter().collect()
    }

    /// Count the connection as a visitor unless that would pass the cap; a
    /// key that is already online never counts twice.
    fn admit(&mut self, id: u64, max: usize) -> bool {
        let online = self.online();
        let Some(c) = self.conns.get_mut(&id) else {
            return false;
        };
        c.admitted = c.admitted || online.contains(&c.key) || online.len() < max;
        c.admitted
    }

    /// Close every connection of these keys.
    fn drop_keys(&mut self, keys: &[PublicKey]) {
        self.conns.retain(|_, c| {
            let lost = keys.contains(&c.key);
            if lost {
                c.conn.close(REVOKED.into(), b"access removed");
            }
            !lost
        });
    }

    fn allowed_anywhere(&self, key: &PublicKey) -> bool {
        let r = &self.registry;
        r.shares.iter().any(|s| r.allowed(&s.id, key))
    }

    /// Whether `key` may fetch `hash`. A published file that changed since
    /// is unpublished (the next `Content` hashes it again).
    fn may_fetch(&mut self, key: &PublicKey, hash: &Hash) -> bool {
        let r = &self.registry;
        let shares: Vec<ShareId> = r
            .shares
            .iter()
            .filter(|s| r.allowed(&s.id, key))
            .map(|s| s.id)
            .collect();
        shares
            .iter()
            .any(|id| self.served.get_mut(id).is_some_and(|s| s.publishes(hash)))
    }

    fn folder(&self, share: &ShareId) -> Option<&Path> {
        let s = self.registry.shares.iter().find(|s| &s.id == share)?;
        Some(s.path.as_path())
    }

    /// The folder's name and the sharer's name, as visitors see them.
    fn labels(&self, share: &ShareId) -> (String, String) {
        let folder = self.folder(share).map(|p| {
            p.file_name()
                .unwrap_or(p.as_os_str())
                .to_string_lossy()
                .into_owned()
        });
        let sharer = self.registry.name.clone().unwrap_or_default();
        (folder.unwrap_or_default(), sharer)
    }

    /// The share's tree while its folder still exists.
    fn present(&self, share: &ShareId) -> Option<&Served> {
        self.folder(share).filter(|p| p.is_dir())?;
        self.served.get(share)
    }

    fn root(&self, key: &PublicKey, share: &ShareId) -> Reply {
        if !self.registry.allowed(share, key) {
            return denied(INVALID);
        }
        let Some(served) = self.present(share) else {
            return Reply::Missing;
        };
        let (folder, sharer) = self.labels(share);
        Reply::Root {
            root: served.tree.root,
            folder,
            sharer,
        }
    }

    fn locate(
        &self,
        key: &PublicKey,
        share: ShareId,
        root: &Hash,
        names: &[String],
    ) -> Result<Job, Reply> {
        if !self.registry.allowed(&share, key) {
            return Err(denied(INVALID));
        }
        let served = self.present(&share).ok_or(Reply::Missing)?;
        let (file, entry) = served
            .tree
            .entry_path(root, names)
            .filter(|(_, e)| e.kind == Kind::File)
            .ok_or_else(|| denied(NOT_FOUND))?;
        Ok(Job {
            share,
            folder: self.folder(&share).ok_or(Reply::Missing)?.to_path_buf(),
            name: entry.name.clone(),
            size: entry.size,
            mtime: entry.mtime,
            file,
        })
    }

    fn publish(&mut self, job: Job, hash: Hash) {
        if let Some(s) = self.served.get_mut(&job.share) {
            let (path, size, mtime) = (job.file, job.size, job.mtime);
            s.files.insert(hash, Published { path, size, mtime });
        }
    }
}

impl Served {
    fn new(tree: Tree) -> Served {
        let root = watch::channel(tree.root).0;
        Served {
            tree,
            files: HashMap::new(),
            root,
        }
    }

    fn publishes(&mut self, hash: &Hash) -> bool {
        if self.tree.manifest(hash).is_some() {
            return true;
        }
        let Some(p) = self.files.get(hash) else {
            return false;
        };
        let same = stat(&p.path).ok() == Some((p.size, p.mtime));
        if !same {
            self.files.remove(hash);
        }
        same
    }
}

/// Whether `file`, with every symlink resolved now, is inside `folder`.
fn inside(file: &Path, folder: &Path) -> bool {
    match (file.canonicalize(), folder.canonicalize()) {
        (Ok(f), Ok(d)) => f.starts_with(d),
        _ => false,
    }
}

// ---- shared core --------------------------------------------------------

struct Inner {
    registry_path: PathBuf,
    store: FsStore,
    state: Mutex<State>,
    cache: Mutex<ContentCache>,
    versions_hint: VersionsHint,
    max_visitors: usize,
}

impl fmt::Debug for Inner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Inner").finish_non_exhaustive()
    }
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ---- the blobs gate

    fn gate_connect(&self, id: u64, key: Option<PublicKey>) -> Result<(), AbortReason> {
        let mut st = self.lock();
        if !key.is_some_and(|k| st.allowed_anywhere(&k)) {
            return Err(AbortReason::Permission);
        }
        match st.admit(id, self.max_visitors) {
            true => Ok(()),
            false => Err(AbortReason::RateLimited),
        }
    }

    fn gate_get(&self, id: u64, hash: &Hash, single_blob: bool) -> Result<(), AbortReason> {
        let mut st = self.lock();
        let key = st.conns.get(&id).filter(|c| c.admitted).map(|c| c.key);
        match key {
            Some(k) if single_blob && st.may_fetch(&k, hash) => Ok(()),
            _ => Err(AbortReason::Permission),
        }
    }

    // ---- control requests, one function each

    fn claim(&self, key: PublicKey, share: &ShareId, token: &Token) -> Reply {
        let mut st = self.lock();
        let path = &self.registry_path;
        match st.registry.claim(share, token, key, |r| r.save(path)) {
            Claimed::Ok => {
                let (folder, sharer) = st.labels(share);
                Reply::Ok { folder, sharer }
            }
            Claimed::Denied(why) => denied(why),
        }
    }

    fn root(&self, key: PublicKey, share: &ShareId) -> Reply {
        self.lock().root(&key, share)
    }

    async fn content(
        self: &Arc<Self>,
        key: PublicKey,
        share: ShareId,
        root: Hash,
        names: Vec<String>,
    ) -> Reply {
        let job = match self.lock().locate(&key, share, &root, &names) {
            Ok(job) => job,
            Err(reply) => return reply,
        };
        let me = self.clone();
        let (job, found) = match blocking(move || (me.hash_file(&job), job)).await {
            Ok((found, job)) => (job, found),
            Err(_) => return Reply::Changed,
        };
        let (hash, size) = match found {
            Ok(found) => found,
            Err(reply) => return reply,
        };
        if let Err(reply) = self.import(&job, hash).await {
            return reply;
        }
        self.lock().publish(job, hash);
        Reply::Content { hash, size }
    }

    /// Hash the file behind a `Content` request, refusing anything that is
    /// no longer what the manifest describes or now leads out of the folder.
    fn hash_file(&self, job: &Job) -> Result<(Hash, u64), Reply> {
        if stat(&job.file).ok() != Some((job.size, job.mtime)) {
            return Err(Reply::Changed);
        }
        if !inside(&job.file, &job.folder) {
            return Err(denied(NOT_FOUND));
        }
        let hint = (self.versions_hint)(&job.file, job.size, job.mtime);
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        match cache.hash(&job.file, hint) {
            Ok(Some(found)) => Ok(found),
            Ok(None) => Err(Reply::Changed),
            Err(_) => Err(unreadable(&job.name)),
        }
    }

    /// Make the file servable in place (by reference, not copied).
    async fn import(&self, job: &Job, hash: Hash) -> Result<(), Reply> {
        let blobs = self.store.blobs();
        if blobs.has(hash).await.unwrap_or(false) {
            return Ok(());
        }
        let opts = AddPathOptions {
            path: job.file.clone(),
            format: BlobFormat::Raw,
            mode: ImportMode::TryReference,
        };
        match blobs.add_path_with_opts(opts).await {
            Ok(tag) if tag.hash == hash => Ok(()),
            Ok(_) => Err(Reply::Changed),
            Err(_) => Err(unreadable(&job.name)),
        }
    }

    /// Send the root now and again whenever it changes, until the stream or
    /// the visitor's access ends.
    async fn watch(&self, key: PublicKey, share: &ShareId, send: &mut SendStream) {
        let mut rx = self.lock().served.get(share).map(|s| s.root.subscribe());
        loop {
            let reply = self.root(key, share);
            let go_on = matches!(reply, Reply::Root { .. });
            if write_msg(send, &reply).await.is_err() || !go_on {
                return;
            }
            let Some(rx) = rx.as_mut() else { return };
            if rx.changed().await.is_err() {
                return;
            }
        }
    }

    // ---- shares

    /// Build the share's tree on a blocking thread and put its manifests in
    /// the store.
    async fn serve(&self, share: ShareId, folder: PathBuf) -> Result<(), String> {
        let tree = blocking(move || Tree::build(&folder)).await??;
        let blobs = self.store.blobs();
        for h in tree.manifests() {
            let bytes = tree.manifest(&h).unwrap_or_default().to_vec();
            blobs.add_bytes(bytes).await.map_err(|e| e.to_string())?;
        }
        self.lock().served.insert(share, Served::new(tree));
        Ok(())
    }

    /// Apply a change that takes access away: it holds at once, even if it
    /// can't be saved, and the visitors who lost access are disconnected.
    fn revoke(&self, change: impl FnOnce(&mut State) -> Vec<PublicKey>) -> Result<(), String> {
        let mut st = self.lock();
        let lost = change(&mut st);
        st.drop_keys(&lost);
        st.registry.save(&self.registry_path)
    }
}

// ---- protocol handlers --------------------------------------------------

#[derive(Debug, Clone)]
struct Control(Arc<Inner>);

impl ProtocolHandler for Control {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let id = conn.stable_id() as u64;
        let key = conn.remote_id();
        let admitted = {
            let mut st = self.0.lock();
            st.track(id, key, conn.clone());
            st.admit(id, self.0.max_visitors)
        };
        while let Ok((send, recv)) = conn.accept_bi().await {
            tokio::spawn(answer(self.0.clone(), key, admitted, send, recv));
        }
        self.0.lock().conns.remove(&id);
        Ok(())
    }
}

/// One request, one reply (a `Watch` keeps replying).
async fn answer(
    inner: Arc<Inner>,
    key: PublicKey,
    admitted: bool,
    mut send: SendStream,
    mut recv: RecvStream,
) {
    let reply = match read_msg::<Request>(&mut recv).await {
        Err(_) => return,
        Ok(_) if !admitted => Reply::Busy,
        Ok(Request::Claim { share, token }) => inner.claim(key, &share, &token),
        Ok(Request::Root { share }) => inner.root(key, &share),
        Ok(Request::Content { share, root, names }) => inner.content(key, share, root, names).await,
        Ok(Request::Watch { share }) => {
            inner.watch(key, &share, &mut send).await;
            send.finish().ok();
            return;
        }
    };
    if write_msg(&mut send, &reply).await.is_ok() {
        send.finish().ok();
    }
}

/// iroh-blobs, with each connection tracked so revoking can close it.
#[derive(Debug, Clone)]
struct Blobs {
    inner: Arc<Inner>,
    proto: BlobsProtocol,
}

impl ProtocolHandler for Blobs {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let id = conn.stable_id() as u64;
        self.inner.lock().track(id, conn.remote_id(), conn.clone());
        let result = self.proto.accept(conn).await;
        self.inner.lock().conns.remove(&id);
        result
    }

    async fn shutdown(&self) {
        self.proto.shutdown().await;
    }
}

/// The access gate (D7) as iroh-blobs provider events.
fn gate(inner: Arc<Inner>) -> EventSender {
    let mask = EventMask {
        connected: ConnectMode::Intercept,
        get: RequestMode::Intercept,
        get_many: RequestMode::Disabled,
        observe: ObserveMode::Intercept,
        ..EventMask::DEFAULT
    };
    let (tx, mut rx) = EventSender::channel(32, mask);
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            decide(&inner, msg).await;
        }
    });
    tx
}

async fn decide(inner: &Inner, msg: ProviderMessage) {
    match msg {
        ProviderMessage::ClientConnected(m) => {
            let verdict = inner.gate_connect(m.connection_id, m.endpoint_id);
            m.tx.send(verdict).await.ok();
        }
        ProviderMessage::GetRequestReceived(m) => {
            let single = m.request.ranges.is_blob();
            let verdict = inner.gate_get(m.connection_id, &m.request.hash, single);
            m.tx.send(verdict).await.ok();
        }
        ProviderMessage::ObserveRequestReceived(m) => {
            let verdict = inner.gate_get(m.connection_id, &m.request.hash, true);
            m.tx.send(verdict).await.ok();
        }
        // Anything else that asks for a verdict is refused by dropping it.
        _ => {}
    }
}

// ---- host ---------------------------------------------------------------

pub struct Host {
    router: Router,
    inner: Arc<Inner>,
    _identity: Identity,
}

impl Host {
    pub async fn start(
        identity: Identity,
        registry_path: PathBuf,
        store_dir: PathBuf,
        endpoint: iroh::Endpoint,
        versions_hint: VersionsHint,
    ) -> Result<Host, String> {
        let parts = (identity, registry_path, store_dir, endpoint, versions_hint);
        Self::start_inner(parts, MAX_VISITORS).await
    }

    #[cfg(test)]
    pub(crate) async fn start_with_max_visitors(
        identity: Identity,
        registry_path: PathBuf,
        store_dir: PathBuf,
        endpoint: iroh::Endpoint,
        versions_hint: VersionsHint,
        max_visitors: usize,
    ) -> Result<Host, String> {
        let parts = (identity, registry_path, store_dir, endpoint, versions_hint);
        Self::start_inner(parts, max_visitors).await
    }

    async fn start_inner(
        (identity, registry_path, store_dir, endpoint, versions_hint): (
            Identity,
            PathBuf,
            PathBuf,
            iroh::Endpoint,
            VersionsHint,
        ),
        max_visitors: usize,
    ) -> Result<Host, String> {
        if endpoint.id() != identity.key.public() {
            return Err(KEY_MISMATCH.into());
        }
        let registry = Registry::load(&registry_path)?;
        let store = FsStore::load(&store_dir).await.map_err(|e| e.to_string())?;
        let shares: Vec<_> = registry
            .shares
            .iter()
            .map(|s| (s.id, s.path.clone()))
            .collect();
        let state = State {
            registry,
            ..State::default()
        };
        let inner = Arc::new(Inner {
            registry_path,
            store: store.clone(),
            state: Mutex::new(state),
            cache: Mutex::new(ContentCache::default()),
            versions_hint,
            max_visitors,
        });
        for (id, path) in shares {
            // A folder that is gone (or can't be walked) answers `Missing`.
            inner.serve(id, path).await.ok();
        }
        let blobs = Blobs {
            inner: inner.clone(),
            proto: BlobsProtocol::new(&store, Some(gate(inner.clone()))),
        };
        let router = Router::builder(endpoint)
            .accept(proto::ALPN, Control(inner.clone()))
            .accept(iroh_blobs::ALPN, blobs)
            .spawn();
        Ok(Host {
            router,
            inner,
            _identity: identity,
        })
    }

    pub fn addr(&self) -> iroh::EndpointAddr {
        self.router.endpoint().addr()
    }

    /// Register the folder (saved once its tree is built) and serve it.
    pub async fn share_folder(&self, path: &Path) -> Result<ShareId, String> {
        let (id, folder, new) = {
            let mut st = self.inner.lock();
            let before = st.registry.shares.len();
            // The Trash is refused by the caller (it knows where it is).
            let id = st.registry.share_folder(path, |_| false)?;
            let folder = st.folder(&id).map(Path::to_path_buf).unwrap_or_default();
            let new = st.registry.shares.len() > before;
            if !new && st.served.contains_key(&id) {
                return Ok(id);
            }
            (id, folder, new)
        };
        let mut done = self.inner.serve(id, folder).await;
        if done.is_ok() {
            done = self.inner.lock().registry.save(&self.inner.registry_path);
        }
        if done.is_err() && new {
            let mut st = self.inner.lock();
            st.registry.stop_sharing(&id);
            st.served.remove(&id);
        }
        done.map(|()| id)
    }

    /// A new link (the Anyone link is reused), saved before it is returned.
    pub async fn create_link(
        &self,
        share: &ShareId,
        kind: LinkKind,
        person: Option<String>,
    ) -> Result<(String, AccessEntry), String> {
        let (entry, sharer) = {
            let mut st = self.inner.lock();
            let mut next = st.registry.clone();
            let entry = next.create_link(share, kind, person.as_deref())?;
            next.save(&self.inner.registry_path)?;
            st.registry = next;
            (entry, st.registry.name.clone().unwrap_or_default())
        };
        let link = ShareLink {
            addr: self.addr(),
            share: *share,
            token: entry.token,
            kind,
            sharer,
        };
        Ok((link.encode(), entry))
    }

    /// Delete one link; whoever loses access is disconnected at once. An
    /// error means the change holds now but couldn't be saved.
    pub async fn remove_access(&self, share: &ShareId, entry: &str) -> Result<(), String> {
        self.inner
            .revoke(|st| st.registry.remove_access(share, entry))
    }

    /// Stop sharing the folder; every visitor of it is disconnected at once.
    pub async fn stop_sharing(&self, share: &ShareId) -> Result<(), String> {
        self.inner.revoke(|st| {
            st.served.remove(share);
            st.registry.stop_sharing(share)
        })
    }

    pub fn online(&self) -> Vec<iroh::PublicKey> {
        self.inner.lock().online()
    }

    pub async fn shutdown(self) {
        self.router.shutdown().await.ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::share::identity::load_or_create;
    use crate::share::link::ShareLink;
    use crate::share::proto::{read_msg, write_msg};
    use crate::share::{test_endpoint, test_endpoint_for};
    use iroh::address_lookup::memory::MemoryLookup;
    use iroh::endpoint::Connection;
    use std::time::Duration;

    const LIMIT: Duration = Duration::from_secs(20);

    async fn start(dir: &Path, max_visitors: usize) -> Host {
        let identity = load_or_create(&dir.join("config")).unwrap();
        let ep = test_endpoint_for(identity.key.clone()).await;
        Host::start_with_max_visitors(
            identity,
            dir.join("config/shares.json"),
            dir.join("store"),
            ep,
            Box::new(|_, _, _| None),
            max_visitors,
        )
        .await
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

    struct Visitor {
        ep: iroh::Endpoint,
        lookup: MemoryLookup,
    }

    impl Visitor {
        async fn new() -> Visitor {
            let ep = test_endpoint().await;
            let lookup = MemoryLookup::new();
            ep.address_lookup().unwrap().add(lookup.clone());
            Visitor { ep, lookup }
        }

        async fn connect(&self, host: &Host, alpn: &[u8]) -> Result<Connection, String> {
            self.lookup.add_endpoint_info(host.addr());
            self.ep
                .connect(host.addr(), alpn)
                .await
                .map_err(|e| e.to_string())
        }

        async fn control(&self, host: &Host) -> Connection {
            self.connect(host, proto::ALPN).await.unwrap()
        }

        async fn ask(&self, host: &Host, req: Request) -> Reply {
            let conn = self.control(host).await;
            let reply = ask_on(&conn, req).await;
            conn.close(0u32.into(), b"done");
            reply
        }

        async fn root(&self, host: &Host, share: ShareId) -> Hash {
            match self.ask(host, Request::Root { share }).await {
                Reply::Root { root, .. } => root,
                r => panic!("expected a root, got {r:?}"),
            }
        }

        async fn content(&self, host: &Host, share: ShareId, name: &str) -> Reply {
            let root = self.root(host, share).await;
            let names = vec![name.to_string()];
            self.ask(host, Request::Content { share, root, names })
                .await
        }

        async fn download(&self, host: &Host, hash: Hash) -> Result<Vec<u8>, String> {
            let conn = self.connect(host, iroh_blobs::ALPN).await?;
            let bytes = iroh_blobs::get::request::get_blob(conn, hash)
                .bytes()
                .await
                .map_err(|e| e.to_string())?;
            Ok(bytes.to_vec())
        }
    }

    async fn ask_on(conn: &Connection, req: Request) -> Reply {
        let (mut send, mut recv) = conn.open_bi().await.unwrap();
        write_msg(&mut send, &req).await.unwrap();
        send.finish().unwrap();
        read_msg(&mut recv).await.unwrap()
    }

    async fn link(host: &Host, share: ShareId) -> (ShareLink, AccessEntry) {
        let (text, entry) = host
            .create_link(&share, LinkKind::Person, Some("Bob".into()))
            .await
            .unwrap();
        (ShareLink::decode(&text).unwrap(), entry)
    }

    /// A new visitor who claimed a fresh person link on `share`.
    async fn claimed(host: &Host, share: ShareId) -> (Visitor, AccessEntry) {
        let (link, entry) = link(host, share).await;
        let v = Visitor::new().await;
        let reply = v
            .ask(
                host,
                Request::Claim {
                    share,
                    token: link.token,
                },
            )
            .await;
        assert!(matches!(reply, Reply::Ok { .. }), "{reply:?}");
        (v, entry)
    }

    fn content_hash(reply: Reply) -> (Hash, u64) {
        match reply {
            Reply::Content { hash, size } => (hash, size),
            r => panic!("expected content, got {r:?}"),
        }
    }

    fn big_data() -> Vec<u8> {
        (0..3_000_000u32)
            .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
            .collect()
    }

    async fn within<F: std::future::Future>(f: F) -> F::Output {
        tokio::time::timeout(LIMIT, f).await.expect("timed out")
    }

    #[tokio::test]
    async fn claim_then_root_returns_the_tree_root() {
        within(async {
            let tmp = tempfile::tempdir().unwrap();
            let f = folder(
                tmp.path(),
                "Photos",
                &[("a.txt", b"hi"), ("sub/b.txt", b"yo")],
            );
            let host = start(tmp.path(), MAX_VISITORS).await;
            let share = host.share_folder(&f).await.unwrap();
            let (v, _) = claimed(&host, share).await;
            let expected = Tree::build(&f).unwrap().root;
            match v.ask(&host, Request::Root { share }).await {
                Reply::Root { root, folder, .. } => {
                    assert_eq!(root, expected);
                    assert_eq!(folder, "Photos");
                }
                r => panic!("{r:?}"),
            }
            host.shutdown().await;
        })
        .await
    }

    #[tokio::test]
    async fn root_without_a_claim_is_denied() {
        within(async {
            let tmp = tempfile::tempdir().unwrap();
            let f = folder(tmp.path(), "F", &[("a.txt", b"hi")]);
            let host = start(tmp.path(), MAX_VISITORS).await;
            let share = host.share_folder(&f).await.unwrap();
            let v = Visitor::new().await;
            assert_eq!(
                v.ask(&host, Request::Root { share }).await,
                Reply::Denied("This link is no longer valid.".into())
            );
            let bad = Request::Claim {
                share,
                token: [9; 32],
            };
            assert_eq!(
                v.ask(&host, bad).await,
                Reply::Denied("This link is no longer valid.".into())
            );
            host.shutdown().await;
        })
        .await
    }

    #[tokio::test]
    async fn content_hashes_a_file_and_it_downloads() {
        within(async {
            let tmp = tempfile::tempdir().unwrap();
            let data = big_data();
            let f = folder(tmp.path(), "F", &[("big.bin", &data)]);
            let host = start(tmp.path(), MAX_VISITORS).await;
            let share = host.share_folder(&f).await.unwrap();
            let (v, _) = claimed(&host, share).await;
            let (hash, size) = content_hash(v.content(&host, share, "big.bin").await);
            assert_eq!(size, data.len() as u64);
            assert_eq!(hash, Hash::new(&data));
            assert_eq!(v.download(&host, hash).await.unwrap(), data);
            host.shutdown().await;
        })
        .await
    }

    #[tokio::test]
    async fn content_refuses_names_outside_the_manifest() {
        within(async {
            let tmp = tempfile::tempdir().unwrap();
            let f = folder(tmp.path(), "F", &[("a.txt", b"hi"), ("sub/b.txt", b"yo")]);
            std::fs::write(tmp.path().join("x"), b"outside").unwrap();
            let host = start(tmp.path(), MAX_VISITORS).await;
            let share = host.share_folder(&f).await.unwrap();
            let (v, _) = claimed(&host, share).await;
            let root = v.root(&host, share).await;
            let cases: Vec<(Hash, Vec<&str>)> = vec![
                (root, vec!["nope"]),
                (root, vec!["../x"]),
                (root, vec!["..", "x"]),
                (root, vec!["sub"]),
                (root, vec![]),
                (Hash::new(b"another root"), vec!["a.txt"]),
            ];
            for (root, names) in cases {
                let names: Vec<String> = names.iter().map(|s| s.to_string()).collect();
                let reply = v
                    .ask(
                        &host,
                        Request::Content {
                            share,
                            root,
                            names: names.clone(),
                        },
                    )
                    .await;
                assert!(matches!(reply, Reply::Denied(_)), "{names:?}: {reply:?}");
            }
            host.shutdown().await;
        })
        .await
    }

    #[tokio::test]
    async fn content_answers_changed_after_a_modification() {
        within(async {
            let tmp = tempfile::tempdir().unwrap();
            let f = folder(tmp.path(), "F", &[("a.txt", b"hi")]);
            let host = start(tmp.path(), MAX_VISITORS).await;
            let share = host.share_folder(&f).await.unwrap();
            let (v, _) = claimed(&host, share).await;
            let root = v.root(&host, share).await;
            std::fs::write(f.join("a.txt"), b"hello, changed").unwrap();
            let names = vec!["a.txt".to_string()];
            assert_eq!(
                v.ask(&host, Request::Content { share, root, names }).await,
                Reply::Changed
            );
            host.shutdown().await;
        })
        .await
    }

    #[tokio::test]
    async fn an_unpublished_hash_is_not_served() {
        within(async {
            let tmp = tempfile::tempdir().unwrap();
            let f = folder(
                tmp.path(),
                "F",
                &[("a.txt", b"hi"), ("secret.txt", b"psst")],
            );
            let host = start(tmp.path(), MAX_VISITORS).await;
            let share = host.share_folder(&f).await.unwrap();
            let (v, _) = claimed(&host, share).await;
            let (hash, _) = content_hash(v.content(&host, share, "a.txt").await);
            assert_eq!(v.download(&host, hash).await.unwrap(), b"hi");
            // The store has the secret file, but it was never published.
            host.inner
                .store
                .blobs()
                .add_path(f.join("secret.txt"))
                .await
                .unwrap();
            assert!(v.download(&host, Hash::new(b"psst")).await.is_err());
            host.shutdown().await;
        })
        .await
    }

    #[tokio::test]
    async fn share_a_hashes_are_denied_to_a_share_b_visitor() {
        within(async {
            let tmp = tempfile::tempdir().unwrap();
            let fa = folder(tmp.path(), "A", &[("a.txt", b"from a")]);
            let fb = folder(tmp.path(), "B", &[("b.txt", b"from b")]);
            let host = start(tmp.path(), MAX_VISITORS).await;
            let a = host.share_folder(&fa).await.unwrap();
            let b = host.share_folder(&fb).await.unwrap();
            let (va, _) = claimed(&host, a).await;
            let (vb, _) = claimed(&host, b).await;
            let (hash, _) = content_hash(va.content(&host, a, "a.txt").await);
            let manifest = va.root(&host, a).await;
            assert!(vb.download(&host, hash).await.is_err());
            assert!(vb.download(&host, manifest).await.is_err());
            assert!(matches!(
                vb.ask(&host, Request::Root { share: a }).await,
                Reply::Denied(_)
            ));
            assert_eq!(va.download(&host, hash).await.unwrap(), b"from a");
            host.shutdown().await;
        })
        .await
    }

    #[tokio::test]
    async fn removing_access_drops_the_connection() {
        within(async {
            let tmp = tempfile::tempdir().unwrap();
            let f = folder(tmp.path(), "F", &[("a.txt", b"hi")]);
            let host = start(tmp.path(), MAX_VISITORS).await;
            let share = host.share_folder(&f).await.unwrap();
            let (v, entry) = claimed(&host, share).await;
            let conn = v.control(&host).await;
            assert!(matches!(
                ask_on(&conn, Request::Root { share }).await,
                Reply::Root { .. }
            ));
            assert!(host.online().contains(&v.ep.id()));
            host.remove_access(&share, &entry.id).await.unwrap();
            tokio::time::timeout(Duration::from_secs(5), conn.closed())
                .await
                .expect("the connection stays open");
            assert!(!host.online().contains(&v.ep.id()));
            assert_eq!(
                v.ask(&host, Request::Root { share }).await,
                Reply::Denied("This link is no longer valid.".into())
            );
            host.shutdown().await;
        })
        .await
    }

    #[tokio::test]
    async fn the_thirty_third_visitor_is_busy() {
        within(async {
            let tmp = tempfile::tempdir().unwrap();
            let f = folder(tmp.path(), "F", &[("a.txt", b"hi")]);
            let host = start(tmp.path(), 1).await;
            let share = host.share_folder(&f).await.unwrap();
            let (l1, _) = link(&host, share).await;
            let (l2, _) = link(&host, share).await;
            let v1 = Visitor::new().await;
            let held = v1.control(&host).await;
            let claim = |token| Request::Claim { share, token };
            assert!(matches!(
                ask_on(&held, claim(l1.token)).await,
                Reply::Ok { .. }
            ));
            let v2 = Visitor::new().await;
            assert_eq!(v2.ask(&host, claim(l2.token)).await, Reply::Busy);
            // The first visitor's further connections are still served.
            assert!(matches!(
                v1.ask(&host, Request::Root { share }).await,
                Reply::Root { .. }
            ));
            host.shutdown().await;
        })
        .await
    }

    #[tokio::test]
    async fn links_survive_a_restart() {
        within(async {
            let tmp = tempfile::tempdir().unwrap();
            let f = folder(tmp.path(), "F", &[("a.txt", b"hi")]);
            let host = start(tmp.path(), MAX_VISITORS).await;
            let id = host.addr().id;
            let share = host.share_folder(&f).await.unwrap();
            let (v, _) = claimed(&host, share).await;
            host.shutdown().await;

            let host = start(tmp.path(), MAX_VISITORS).await;
            assert_eq!(host.addr().id, id);
            assert!(matches!(
                v.ask(&host, Request::Root { share }).await,
                Reply::Root { .. }
            ));
            host.shutdown().await;
        })
        .await
    }

    #[tokio::test]
    async fn a_stranger_gets_no_blobs_at_all() {
        within(async {
            let tmp = tempfile::tempdir().unwrap();
            let f = folder(tmp.path(), "F", &[("a.txt", b"hi")]);
            let host = start(tmp.path(), MAX_VISITORS).await;
            let share = host.share_folder(&f).await.unwrap();
            let (v, _) = claimed(&host, share).await;
            let root = v.root(&host, share).await;
            assert!(v.download(&host, root).await.is_ok());
            let stranger = Visitor::new().await;
            assert!(stranger.download(&host, root).await.is_err());
            host.shutdown().await;
        })
        .await
    }

    #[tokio::test]
    async fn a_file_changed_after_content_is_not_served() {
        within(async {
            let tmp = tempfile::tempdir().unwrap();
            let f = folder(tmp.path(), "F", &[("a.txt", b"hi")]);
            let host = start(tmp.path(), MAX_VISITORS).await;
            let share = host.share_folder(&f).await.unwrap();
            let (v, _) = claimed(&host, share).await;
            let (hash, _) = content_hash(v.content(&host, share, "a.txt").await);
            let later = std::time::SystemTime::now() + Duration::from_secs(60);
            let file = std::fs::File::options().write(true).open(f.join("a.txt"));
            file.unwrap().set_modified(later).unwrap();
            assert!(v.download(&host, hash).await.is_err());
            host.shutdown().await;
        })
        .await
    }

    #[tokio::test]
    async fn a_symlink_swapped_in_after_the_walk_is_refused() {
        within(async {
            let tmp = tempfile::tempdir().unwrap();
            let f = folder(tmp.path(), "F", &[("a.txt", b"hi")]);
            let outside = tmp.path().join("outside.txt");
            std::fs::write(&outside, b"no").unwrap();
            let host = start(tmp.path(), MAX_VISITORS).await;
            let share = host.share_folder(&f).await.unwrap();
            let (v, _) = claimed(&host, share).await;
            let root = v.root(&host, share).await;
            // Same size and mtime as the manifest entry, but outside.
            let mtime = std::fs::metadata(f.join("a.txt"))
                .unwrap()
                .modified()
                .unwrap();
            std::fs::File::options()
                .write(true)
                .open(&outside)
                .unwrap()
                .set_modified(mtime)
                .unwrap();
            std::fs::remove_file(f.join("a.txt")).unwrap();
            std::os::unix::fs::symlink(&outside, f.join("a.txt")).unwrap();
            let names = vec!["a.txt".to_string()];
            assert!(matches!(
                v.ask(&host, Request::Content { share, root, names }).await,
                Reply::Denied(_)
            ));
            host.shutdown().await;
        })
        .await
    }

    #[tokio::test]
    async fn an_unreadable_file_says_so() {
        use std::os::unix::fs::PermissionsExt;
        within(async {
            let tmp = tempfile::tempdir().unwrap();
            let f = folder(tmp.path(), "F", &[("a.txt", b"hi")]);
            let host = start(tmp.path(), MAX_VISITORS).await;
            let share = host.share_folder(&f).await.unwrap();
            let (v, _) = claimed(&host, share).await;
            let a = f.join("a.txt");
            std::fs::set_permissions(&a, std::fs::Permissions::from_mode(0o000)).unwrap();
            if std::fs::read(&a).is_ok() {
                return; // running as root: nothing is unreadable
            }
            assert_eq!(
                v.content(&host, share, "a.txt").await,
                Reply::Denied("“a.txt” can't be read.".into())
            );
            host.shutdown().await;
        })
        .await
    }

    #[tokio::test]
    async fn a_vanished_folder_is_missing() {
        within(async {
            let tmp = tempfile::tempdir().unwrap();
            let f = folder(tmp.path(), "F", &[("a.txt", b"hi")]);
            let host = start(tmp.path(), MAX_VISITORS).await;
            let share = host.share_folder(&f).await.unwrap();
            let (v, _) = claimed(&host, share).await;
            let root = v.root(&host, share).await;
            std::fs::rename(&f, tmp.path().join("G")).unwrap();
            assert_eq!(v.ask(&host, Request::Root { share }).await, Reply::Missing);
            let names = vec!["a.txt".to_string()];
            assert_eq!(
                v.ask(&host, Request::Content { share, root, names }).await,
                Reply::Missing
            );
            host.shutdown().await;
        })
        .await
    }

    #[tokio::test]
    async fn stop_sharing_disconnects_and_denies() {
        within(async {
            let tmp = tempfile::tempdir().unwrap();
            let f = folder(tmp.path(), "F", &[("a.txt", b"hi")]);
            let host = start(tmp.path(), MAX_VISITORS).await;
            let share = host.share_folder(&f).await.unwrap();
            let (v, _) = claimed(&host, share).await;
            let conn = v.control(&host).await;
            assert!(matches!(
                ask_on(&conn, Request::Root { share }).await,
                Reply::Root { .. }
            ));
            host.stop_sharing(&share).await.unwrap();
            tokio::time::timeout(Duration::from_secs(5), conn.closed())
                .await
                .expect("the connection stays open");
            assert_eq!(
                v.ask(&host, Request::Root { share }).await,
                Reply::Denied("This link is no longer valid.".into())
            );
            let saved = Registry::load(&tmp.path().join("config/shares.json")).unwrap();
            assert!(saved.shares.is_empty());
            host.shutdown().await;
        })
        .await
    }

    #[tokio::test]
    async fn the_endpoint_must_carry_the_identity_key() {
        within(async {
            let tmp = tempfile::tempdir().unwrap();
            let identity = load_or_create(tmp.path()).unwrap();
            let other = test_endpoint().await;
            let started = Host::start(
                identity,
                tmp.path().join("shares.json"),
                tmp.path().join("store"),
                other,
                Box::new(|_, _, _| None),
            )
            .await;
            assert_eq!(
                started.err(),
                Some("The sharing key doesn't match the network endpoint.".to_string())
            );
        })
        .await
    }
}
