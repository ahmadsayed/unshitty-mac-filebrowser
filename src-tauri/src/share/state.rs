//! Tauri-facing sharing state: one lazily started iroh node, the host and the
//! guest on the same endpoint, plus the registry files. Plain async methods;
//! `lib.rs` only forwards commands here.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tokio::sync::OnceCell;

use crate::share::guest::{Guest, Status};
use crate::share::host::{Host, VersionsHint};
use crate::share::identity;
use crate::share::link::{LinkKind, ShareId, ShareLink};
use crate::share::location::{ShareLocation, safe_name};
use crate::share::registry::{AccessEntry, Received, ReceivedList, Registry, Share};

struct Node {
    host: Host,
    guest: Guest,
}

pub struct ShareState {
    config_dir: PathBuf,
    cache_dir: PathBuf,
    node: OnceCell<Node>,
}

#[derive(Debug, Serialize)]
pub struct ShareIdentity {
    pub name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AccessView {
    pub id: String,
    pub kind: &'static str,
    pub name: String,
    pub state: &'static str,
    pub visitors: usize,
}

#[derive(Debug, Serialize)]
pub struct ShareInfo {
    pub id: String,
    pub path: String,
    pub status: &'static str,
    pub access: Vec<AccessView>,
}

#[derive(Debug, Serialize)]
pub struct ShareSummary {
    pub id: String,
    pub path: String,
    pub name: String,
    pub status: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ReceivedShare {
    pub id: String,
    pub folder: String,
    pub sharer: String,
    pub status: &'static str,
}

#[derive(Debug, Serialize)]
pub struct LinkResult {
    pub link: String,
    pub entry: AccessView,
}

impl ShareState {
    pub fn new(config_dir: PathBuf, cache_dir: PathBuf) -> Self {
        Self {
            config_dir,
            cache_dir,
            node: OnceCell::new(),
        }
    }

    fn shares_path(&self) -> PathBuf {
        self.config_dir.join("shares.json")
    }

    fn received_path(&self) -> PathBuf {
        self.config_dir.join("received.json")
    }

    fn read_registry(&self) -> Result<Registry, String> {
        Registry::load(&self.shares_path())
    }

    fn read_received(&self) -> Result<ReceivedList, String> {
        ReceivedList::load(&self.received_path())
    }

    /// Start the node the first time it is actually needed. The host store and
    /// the guest store are separate; they share only the endpoint.
    async fn node(&self, versions_hint: VersionsHint) -> Result<&Node, String> {
        self.node
            .get_or_try_init(|| async {
                let identity = identity::load_or_create(&self.config_dir)?;
                #[cfg(test)]
                let preset = iroh::endpoint::presets::Minimal;
                #[cfg(not(test))]
                let preset = iroh::endpoint::presets::N0;
                let endpoint = iroh::Endpoint::builder(preset)
                    .secret_key(identity.key.clone())
                    .bind()
                    .await
                    .map_err(|e| e.to_string())?;
                let guest = Guest::new(
                    endpoint.clone(),
                    self.cache_dir.join("share-guest"),
                )
                .await?;
                let host = Host::start(
                    identity,
                    self.shares_path(),
                    self.cache_dir.join("share-host"),
                    endpoint,
                    versions_hint,
                )
                .await?;
                Ok(Node { host, guest })
            })
            .await
    }

    pub fn identity(&self) -> Result<ShareIdentity, String> {
        Ok(ShareIdentity {
            name: self.read_registry()?.name,
        })
    }

    /// Set the name visitors see. No node is started for a machine that has
    /// never shared anything.
    pub fn set_name(&self, name: Option<String>) -> Result<(), String> {
        let mut registry = self.read_registry()?;
        registry.name = name.filter(|name| !name.trim().is_empty());
        registry.save(&self.shares_path())?;
        if let Some(node) = self.node.get() {
            node.host.set_name(registry.name.clone())?;
        }
        Ok(())
    }

    pub async fn share_folder<F>(
        &self,
        path: &Path,
        is_trash: F,
        versions_hint: VersionsHint,
    ) -> Result<ShareInfo, String>
    where
        F: Fn(&Path) -> bool + Send,
    {
        let node = self.node(versions_hint).await?;
        node.host.share_folder(path, is_trash).await?;
        let registry = self.read_registry()?;
        let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let share = registry
            .shares
            .iter()
            .find(|share| share.path == canonical)
            .ok_or_else(|| "The folder was not added to sharing.".to_string())?;
        Ok(info_for(share, &node.host))
    }

    pub async fn info(
        &self,
        path: &Path,
        versions_hint: VersionsHint,
    ) -> Result<Option<ShareInfo>, String> {
        let registry = self.read_registry()?;
        let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let Some(share) = registry.shares.iter().find(|share| share.path == canonical) else {
            return Ok(None);
        };
        let node = self.node(versions_hint).await?;
        Ok(Some(info_for(share, &node.host)))
    }

    pub async fn shares(&self, versions_hint: VersionsHint) -> Result<Vec<ShareSummary>, String> {
        let registry = self.read_registry()?;
        if registry.shares.is_empty() {
            return Ok(Vec::new());
        }
        // Start the host even though the summaries only describe the folders:
        // an existing share must be served from launch.
        self.node(versions_hint).await?;
        Ok(registry
            .shares
            .iter()
            .map(summary_for)
            .collect())
    }

    pub async fn create_link(
        &self,
        share: &str,
        kind: &str,
        person: Option<String>,
        versions_hint: VersionsHint,
    ) -> Result<LinkResult, String> {
        let id = parse_share_id(share)?;
        let kind = match kind {
            "anyone" => LinkKind::Anyone,
            "person" => LinkKind::Person,
            _ => return Err("Unknown link kind.".into()),
        };
        let node = self.node(versions_hint).await?;
        let (link, entry) = node.host.create_link(&id, kind, person).await?;
        let online = node.host.online();
        Ok(LinkResult {
            link,
            entry: access_view(&entry, &online),
        })
    }

    pub async fn link(
        &self,
        share: &str,
        entry: &str,
        versions_hint: VersionsHint,
    ) -> Result<String, String> {
        let id = parse_share_id(share)?;
        let registry = self.read_registry()?;
        let share = registry
            .shares
            .iter()
            .find(|share| share.id == id)
            .ok_or_else(|| "This folder is no longer shared.".to_string())?;
        let access = share
            .access
            .iter()
            .find(|access| access.id == entry)
            .ok_or_else(|| "This link no longer exists.".to_string())?;
        let node = self.node(versions_hint).await?;
        Ok(ShareLink {
            addr: node.host.addr(),
            share: id,
            token: access.token,
            kind: access.kind,
            sharer: registry.name.clone().unwrap_or_default(),
        }
        .encode())
    }

    pub async fn remove_access(
        &self,
        share: &str,
        entry: &str,
        versions_hint: VersionsHint,
    ) -> Result<(), String> {
        let share = parse_share_id(share)?;
        let node = self.node(versions_hint).await?;
        node.host.remove_access(&share, entry).await
    }

    pub async fn stop_sharing(
        &self,
        share: &str,
        versions_hint: VersionsHint,
    ) -> Result<(), String> {
        let share = parse_share_id(share)?;
        let node = self.node(versions_hint).await?;
        node.host.stop_sharing(&share).await
    }

    pub async fn add_received(
        &self,
        text: &str,
        versions_hint: VersionsHint,
    ) -> Result<ReceivedShare, String> {
        let link = ShareLink::decode(text)?;
        let mut list = self.read_received()?;
        if list
            .items
            .iter()
            .any(|r| r.link.addr.id == link.addr.id && r.link.share == link.share)
        {
            return Err("This folder is already in Shares.".into());
        }
        let node = self.node(versions_hint).await?;
        if link.addr.id == node.host.addr().id {
            return Err("This is your own shared folder.".into());
        }
        let (folder, sharer) = node.guest.claim(&link).await?;
        let item = list.add(link, folder, sharer, &node.host.addr().id)?;
        list.save(&self.received_path())?;
        Ok(received_view(&item, Status::Online))
    }

    pub async fn received(
        &self,
        versions_hint: VersionsHint,
    ) -> Result<Vec<ReceivedShare>, String> {
        let list = self.read_received()?;
        if list.items.is_empty() {
            return Ok(Vec::new());
        }
        let node = self.node(versions_hint).await?;
        let mut out = Vec::with_capacity(list.items.len());
        for item in &list.items {
            out.push(received_view(item, node.guest.status(item).await));
        }
        Ok(out)
    }

    pub fn remove_received(&self, id: &str) -> Result<(), String> {
        let mut list = self.read_received()?;
        list.remove(id);
        list.save(&self.received_path())
    }

    pub async fn list_dir(
        &self,
        path: &str,
        show_hidden: bool,
        versions_hint: VersionsHint,
    ) -> Result<Vec<crate::fs_ops::Entry>, String> {
        let location = ShareLocation::parse(path)
            .ok_or_else(|| "This isn't a share location.".to_string())?;
        let list = self.read_received()?;
        let received = list
            .items
            .iter()
            .find(|r| r.id == location.received)
            .ok_or_else(|| "This folder is no longer in Shares.".to_string())?;
        let node = self.node(versions_hint).await?;
        node.guest.list(received, &location, show_hidden).await
    }

    /// Fetch a file into the app cache and return the local path to open.
    pub async fn open(
        &self,
        path: &str,
        versions_hint: VersionsHint,
    ) -> Result<String, String> {
        let location = ShareLocation::parse(path)
            .ok_or_else(|| "This isn't a share location.".to_string())?;
        let list = self.read_received()?;
        let received = list
            .items
            .iter()
            .find(|r| r.id == location.received)
            .ok_or_else(|| "This folder is no longer in Shares.".to_string())?;
        let name = location
            .names
            .last()
            .and_then(|name| safe_name(name))
            .ok_or_else(|| "Only files can be opened.".to_string())?;
        let node = self.node(versions_hint).await?;
        let dir = self
            .cache_dir
            .join("share-open")
            .join(unique_dir());
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let dest = dir.join(name);
        if let Err(error) = node.guest.fetch(received, &location, &dest).await {
            let _ = fs::remove_dir_all(&dir);
            return Err(error);
        }
        Ok(dest.to_string_lossy().into_owned())
    }
}

fn info_for(share: &Share, host: &Host) -> ShareInfo {
    let online = host.online();
    ShareInfo {
        id: share_hex(&share.id),
        path: share.path.to_string_lossy().into_owned(),
        status: if share.path.is_dir() { "ok" } else { "missing" },
        access: share
            .access
            .iter()
            .map(|entry| access_view(entry, &online))
            .collect(),
    }
}

fn summary_for(share: &Share) -> ShareSummary {
    ShareSummary {
        id: share_hex(&share.id),
        path: share.path.to_string_lossy().into_owned(),
        name: share
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        status: if share.path.is_dir() { "ok" } else { "missing" },
    }
}

fn access_view(entry: &AccessEntry, online: &[iroh::PublicKey]) -> AccessView {
    let keys: &[iroh::PublicKey] = match entry.kind {
        LinkKind::Anyone => &entry.visitors,
        LinkKind::Person => entry.bound.as_ref().map(std::slice::from_ref).unwrap_or(&[]),
    };
    let state = if keys.is_empty() {
        "unused"
    } else if keys.iter().any(|key| online.contains(key)) {
        "online"
    } else {
        "offline"
    };
    AccessView {
        id: entry.id.clone(),
        kind: match entry.kind {
            LinkKind::Anyone => "anyone",
            LinkKind::Person => "person",
        },
        name: entry.name.clone(),
        state,
        visitors: entry.visitors.len(),
    }
}

fn received_view(item: &Received, status: Status) -> ReceivedShare {
    ReceivedShare {
        id: item.id.clone(),
        folder: item.folder.clone(),
        sharer: item.sharer.clone(),
        status: match status {
            Status::Online => "online",
            Status::Offline => "offline",
            Status::NoAccess => "no-access",
            Status::Missing => "missing",
        },
    }
}

fn share_hex(id: &ShareId) -> String {
    data_encoding::HEXLOWER.encode(id)
}

fn parse_share_id(text: &str) -> Result<ShareId, String> {
    let bytes = data_encoding::HEXLOWER
        .decode(text.as_bytes())
        .map_err(|_| "This folder is no longer shared.".to_string())?;
    bytes
        .try_into()
        .map_err(|_| "This folder is no longer shared.".to_string())
}

fn unique_dir() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{now:x}-{n:x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn no_hint() -> VersionsHint {
        Box::new(|_, _, _| None)
    }

    #[tokio::test]
    async fn the_node_starts_only_when_needed() {
        let tmp = tempfile::tempdir().unwrap();
        let state = ShareState::new(tmp.path().join("config"), tmp.path().join("cache"));

        state.identity().unwrap();
        assert!(state.shares(no_hint()).await.unwrap().is_empty());
        assert!(state.received(no_hint()).await.unwrap().is_empty());
        assert!(!tmp.path().join("config/share-key").exists());
    }

    #[tokio::test]
    async fn a_folder_is_shared_and_received_end_to_end() {
        tokio::time::timeout(Duration::from_secs(30), async {
            let tmp = tempfile::tempdir().unwrap();
            let folder = tmp.path().join("Shared");
            std::fs::create_dir(&folder).unwrap();
            std::fs::write(folder.join("a.txt"), b"hi").unwrap();

            let host = ShareState::new(
                tmp.path().join("host-config"),
                tmp.path().join("host-cache"),
            );
            let info = host
                .share_folder(&folder, |_| false, no_hint())
                .await
                .unwrap();
            assert_eq!(info.status, "ok");
            let link = host
                .create_link(&info.id, "anyone", None, no_hint())
                .await
                .unwrap();
            assert!(link.link.starts_with("fbshare1:"));
            assert_eq!(
                host.link(&info.id, &link.entry.id, no_hint()).await.unwrap(),
                link.link
            );
            assert!(host.info(&folder, no_hint()).await.unwrap().is_some());
            assert_eq!(host.shares(no_hint()).await.unwrap().len(), 1);

            let guest = ShareState::new(
                tmp.path().join("guest-config"),
                tmp.path().join("guest-cache"),
            );
            let received = guest.add_received(&link.link, no_hint()).await.unwrap();
            assert_eq!(received.folder, "Shared");
            assert_eq!(
                guest.received(no_hint()).await.unwrap()[0].status,
                "online"
            );
            let root = guest
                .list_dir(&format!("share://{}", received.id), false, no_hint())
                .await
                .unwrap();
            assert_eq!(
                root.iter().map(|entry| entry.name.as_str()).collect::<Vec<_>>(),
                vec!["a.txt"]
            );
            let opened = guest
                .open(&format!("share://{}/a.txt", received.id), no_hint())
                .await
                .unwrap();
            assert_eq!(std::fs::read(&opened).unwrap(), b"hi");

            guest.remove_received(&received.id).unwrap();
            assert!(guest.received(no_hint()).await.unwrap().is_empty());
            host.stop_sharing(&info.id, no_hint()).await.unwrap();
            assert!(host.shares(no_hint()).await.unwrap().is_empty());
        })
        .await
        .expect("timed out");
    }

    #[test]
    fn access_states_follow_the_visitor_keys() {
        let key = iroh::SecretKey::from_bytes(&[7; 32]).public();
        let entry = |kind, bound, visitors| AccessEntry {
            id: "e1".into(),
            kind,
            name: "Sara".into(),
            token: [0; 32],
            bound,
            visitors,
        };

        assert_eq!(access_view(&entry(LinkKind::Anyone, None, vec![]), &[]).state, "unused");
        assert_eq!(
            access_view(&entry(LinkKind::Anyone, None, vec![key]), &[key]).state,
            "online"
        );
        assert_eq!(
            access_view(&entry(LinkKind::Person, Some(key), vec![]), &[]).state,
            "offline"
        );
        assert_eq!(
            access_view(&entry(LinkKind::Person, Some(key), vec![]), &[key]).state,
            "online"
        );
    }

    #[test]
    fn share_ids_are_hex_and_validated() {
        let id = [0xab; 16];
        assert_eq!(parse_share_id(&share_hex(&id)).unwrap(), id);
        assert!(parse_share_id("nope").is_err());
        assert!(parse_share_id("00").is_err());
    }
}
