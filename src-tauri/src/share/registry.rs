//! The share registry: which folders are shared, who holds which link, and
//! the claim rules (design D6, D7). Plain functions, no Tauri types.

use super::link::{random_id, random_token, LinkKind, ShareId, ShareLink, Token};
use crate::config_store::write_json;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::path::{Path, PathBuf};

const ONLY_LOCAL: &str = "Only folders on this computer can be shared.";
const INVALID: &str = "This link is no longer valid.";
const USED: &str = "This link was already used on another computer.";
const NOT_SAVED: &str = "Sharing settings couldn't be saved.";

// ---- hex serde helpers -------------------------------------------------

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex<const N: usize>(s: &str) -> Option<[u8; N]> {
    if s.len() != N * 2 || !s.is_ascii() {
        return None;
    }
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn hex_bytes<const N: usize, S: Serializer>(v: &[u8; N], s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&to_hex(v))
}

fn unhex<'de, const N: usize, D: Deserializer<'de>>(d: D) -> Result<[u8; N], D::Error> {
    let s = String::deserialize(d)?;
    from_hex::<N>(&s).ok_or_else(|| serde::de::Error::custom("expected lowercase hex"))
}

fn ser_id<S: Serializer>(v: &ShareId, s: S) -> Result<S::Ok, S::Error> {
    hex_bytes(v, s)
}
fn de_id<'de, D: Deserializer<'de>>(d: D) -> Result<ShareId, D::Error> {
    unhex::<16, D>(d)
}
fn ser_token<S: Serializer>(v: &Token, s: S) -> Result<S::Ok, S::Error> {
    hex_bytes(v, s)
}
fn de_token<'de, D: Deserializer<'de>>(d: D) -> Result<Token, D::Error> {
    unhex::<32, D>(d)
}

fn key_to_hex(k: &iroh::PublicKey) -> String {
    to_hex(k.as_bytes())
}

fn key_from_hex(s: &str) -> Option<iroh::PublicKey> {
    iroh::PublicKey::from_bytes(&from_hex::<32>(s)?).ok()
}

fn ser_opt_key<S: Serializer>(v: &Option<iroh::PublicKey>, s: S) -> Result<S::Ok, S::Error> {
    v.as_ref().map(key_to_hex).serialize(s)
}
fn de_opt_key<'de, D: Deserializer<'de>>(d: D) -> Result<Option<iroh::PublicKey>, D::Error> {
    match Option::<String>::deserialize(d)? {
        None => Ok(None),
        Some(s) => key_from_hex(&s)
            .map(Some)
            .ok_or_else(|| serde::de::Error::custom("bad key")),
    }
}
fn ser_keys<S: Serializer>(v: &[iroh::PublicKey], s: S) -> Result<S::Ok, S::Error> {
    v.iter().map(key_to_hex).collect::<Vec<_>>().serialize(s)
}
fn de_keys<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<iroh::PublicKey>, D::Error> {
    Vec::<String>::deserialize(d)?
        .iter()
        .map(|s| key_from_hex(s).ok_or_else(|| serde::de::Error::custom("bad key")))
        .collect()
}

fn short_id() -> String {
    to_hex(&random_id()[..8])
}

/// Constant-time token comparison.
fn same_token(a: &Token, b: &Token) -> bool {
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Write JSON, then make the file readable by its owner only.
fn write_private<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    write_json(path, value)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("Failed to save settings: {e}"))?;
    }
    Ok(())
}

fn read_or_default<T: Default + for<'de> Deserialize<'de>>(path: &Path) -> T {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

// ---- types -------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessEntry {
    pub id: String,
    pub kind: LinkKind,
    pub name: String,
    #[serde(serialize_with = "ser_token", deserialize_with = "de_token")]
    pub token: Token,
    /// Person links: the one computer this link is tied to.
    #[serde(
        default,
        serialize_with = "ser_opt_key",
        deserialize_with = "de_opt_key"
    )]
    pub bound: Option<iroh::PublicKey>,
    /// Anyone links: every computer that opened it.
    #[serde(default, serialize_with = "ser_keys", deserialize_with = "de_keys")]
    pub visitors: Vec<iroh::PublicKey>,
}

impl AccessEntry {
    fn keys(&self) -> Vec<iroh::PublicKey> {
        match self.kind {
            LinkKind::Anyone => self.visitors.clone(),
            LinkKind::Person => self.bound.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Share {
    #[serde(serialize_with = "ser_id", deserialize_with = "de_id")]
    pub id: ShareId,
    pub path: PathBuf,
    #[serde(default)]
    pub access: Vec<AccessEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub shares: Vec<Share>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Claimed {
    Ok,
    Denied(&'static str),
}

// ---- registry ----------------------------------------------------------

/// Whether a path may be shared at all: a real folder on this computer.
fn is_local_folder(path: &Path, is_trash: &impl Fn(&Path) -> bool) -> bool {
    let text = path.to_string_lossy();
    if text.starts_with("share://") || text.starts_with("gdrive://") {
        return false;
    }
    if !path.is_absolute() || is_trash(path) {
        return false;
    }
    if path
        .components()
        .any(|c| c.as_os_str() == crate::versions::VERSIONS_DIR)
    {
        return false;
    }
    // A symlink to a folder is not a folder here: shares point at real ones.
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}

impl Registry {
    pub fn load(path: &Path) -> Registry {
        read_or_default(path)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        write_private(path, self)
    }

    fn share_mut(&mut self, id: &ShareId) -> Option<&mut Share> {
        self.shares.iter_mut().find(|s| &s.id == id)
    }

    pub fn share_folder(
        &mut self,
        path: &Path,
        is_trash: impl Fn(&Path) -> bool,
    ) -> Result<ShareId, String> {
        if !is_local_folder(path, &is_trash) {
            return Err(ONLY_LOCAL.into());
        }
        if let Some(s) = self.shares.iter().find(|s| s.path == path) {
            return Ok(s.id);
        }
        let id = random_id();
        self.shares.push(Share {
            id,
            path: path.to_path_buf(),
            access: Vec::new(),
        });
        Ok(id)
    }

    pub fn create_link(
        &mut self,
        share: &ShareId,
        kind: LinkKind,
        person: Option<&str>,
    ) -> Result<AccessEntry, String> {
        let s = self.share_mut(share).ok_or("This folder isn't shared.")?;
        if kind == LinkKind::Anyone {
            if let Some(e) = s.access.iter().find(|e| e.kind == LinkKind::Anyone) {
                return Ok(e.clone());
            }
        }
        let name = match kind {
            LinkKind::Anyone => "Anyone with the link".to_string(),
            LinkKind::Person => match person.map(str::trim) {
                Some(n) if !n.is_empty() => n.to_string(),
                _ => return Err("Enter a name for this person.".into()),
            },
        };
        let entry = AccessEntry {
            id: short_id(),
            kind,
            name,
            token: random_token(),
            bound: None,
            visitors: Vec::new(),
        };
        s.access.push(entry.clone());
        Ok(entry)
    }

    /// Apply the claim rules (D6). The change is made on a copy and only
    /// kept once `persist` succeeded, so a crash or a failed save never
    /// leaves a binding that was not written down.
    pub fn claim(
        &mut self,
        share: &ShareId,
        token: &Token,
        visitor: iroh::PublicKey,
        persist: impl FnOnce(&Registry) -> Result<(), String>,
    ) -> Claimed {
        let mut next = self.clone();
        let Some(entry) = next
            .share_mut(share)
            .and_then(|s| s.access.iter_mut().find(|e| same_token(&e.token, token)))
        else {
            return Claimed::Denied(INVALID);
        };
        let changed = match entry.kind {
            LinkKind::Anyone => {
                let new = !entry.visitors.contains(&visitor);
                if new {
                    entry.visitors.push(visitor);
                }
                new
            }
            LinkKind::Person => match entry.bound {
                Some(k) if k == visitor => false,
                Some(_) => return Claimed::Denied(USED),
                None => {
                    entry.bound = Some(visitor);
                    true
                }
            },
        };
        if changed {
            if persist(&next).is_err() {
                return Claimed::Denied(NOT_SAVED);
            }
            *self = next;
        }
        Claimed::Ok
    }

    pub fn allowed(&self, share: &ShareId, visitor: &iroh::PublicKey) -> bool {
        self.shares
            .iter()
            .filter(|s| &s.id == share)
            .flat_map(|s| &s.access)
            .any(|e| e.keys().contains(visitor))
    }

    /// Delete one link; returns the keys that no longer have access.
    pub fn remove_access(&mut self, share: &ShareId, entry: &str) -> Vec<iroh::PublicKey> {
        let Some(s) = self.share_mut(share) else {
            return Vec::new();
        };
        let Some(i) = s.access.iter().position(|e| e.id == entry) else {
            return Vec::new();
        };
        let removed = s.access.remove(i);
        let rest: Vec<_> = s.access.iter().flat_map(AccessEntry::keys).collect();
        removed
            .keys()
            .into_iter()
            .filter(|k| !rest.contains(k))
            .collect()
    }

    /// Delete a whole share; returns every key that had access.
    pub fn stop_sharing(&mut self, share: &ShareId) -> Vec<iroh::PublicKey> {
        let Some(i) = self.shares.iter().position(|s| &s.id == share) else {
            return Vec::new();
        };
        let gone = self.shares.remove(i);
        let mut keys: Vec<iroh::PublicKey> =
            gone.access.iter().flat_map(AccessEntry::keys).collect();
        keys.dedup();
        keys
    }

    /// A folder was moved or renamed: shares at or under `from` follow it.
    pub fn follow_move(&mut self, from: &Path, to: &Path) {
        for s in &mut self.shares {
            if let Ok(rest) = s.path.strip_prefix(from) {
                s.path = to.join(rest);
            }
        }
    }
}

// ---- received shares ---------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReceivedWire {
    id: String,
    link: String,
    folder: String,
    sharer: String,
}

#[derive(Debug, Clone)]
pub struct Received {
    pub id: String,
    pub link: ShareLink,
    pub folder: String,
    pub sharer: String,
}

#[derive(Debug, Clone, Default)]
pub struct ReceivedList {
    pub items: Vec<Received>,
}

impl ReceivedList {
    pub fn load(path: &Path) -> ReceivedList {
        let wire: Vec<ReceivedWire> = read_or_default(path);
        let items = wire
            .into_iter()
            .filter_map(|w| {
                Some(Received {
                    link: ShareLink::decode(&w.link).ok()?,
                    id: w.id,
                    folder: w.folder,
                    sharer: w.sharer,
                })
            })
            .collect();
        ReceivedList { items }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let wire: Vec<ReceivedWire> = self
            .items
            .iter()
            .map(|r| ReceivedWire {
                id: r.id.clone(),
                link: r.link.encode(),
                folder: r.folder.clone(),
                sharer: r.sharer.clone(),
            })
            .collect();
        write_private(path, &wire)
    }

    pub fn add(
        &mut self,
        link: ShareLink,
        folder: String,
        sharer: String,
        own_key: &iroh::PublicKey,
    ) -> Result<Received, String> {
        if &link.addr.id == own_key {
            return Err("This is your own shared folder.".into());
        }
        if self
            .items
            .iter()
            .any(|r| r.link.addr.id == link.addr.id && r.link.share == link.share)
        {
            return Err("This folder is already in Network.".into());
        }
        let item = Received {
            id: short_id(),
            link,
            folder,
            sharer,
        };
        self.items.push(item.clone());
        Ok(item)
    }

    pub fn remove(&mut self, id: &str) {
        self.items.retain(|r| r.id != id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u8) -> iroh::PublicKey {
        iroh::SecretKey::from_bytes(&[n; 32]).public()
    }

    fn setup() -> (Registry, ShareId) {
        let mut r = Registry::default();
        r.share_folder(Path::new("/"), |_| false).unwrap();
        let id = r.shares[0].id;
        (r, id)
    }

    fn ok(_: &Registry) -> Result<(), String> {
        Ok(())
    }

    fn link_for(n: u8, share: u8) -> ShareLink {
        ShareLink {
            addr: iroh::EndpointAddr::new(key(n)),
            share: [share; 16],
            token: [3; 32],
            kind: LinkKind::Person,
            sharer: "S".into(),
        }
    }

    #[test]
    fn an_unused_person_link_binds_to_the_first_visitor() {
        let (mut r, id) = setup();
        let e = r.create_link(&id, LinkKind::Person, Some("Sara")).unwrap();
        assert_eq!(r.claim(&id, &e.token, key(1), ok), Claimed::Ok);
        assert!(r.allowed(&id, &key(1)));
        assert!(!r.allowed(&id, &key(2)));
    }

    #[test]
    fn the_same_visitor_can_claim_again() {
        let (mut r, id) = setup();
        let e = r.create_link(&id, LinkKind::Person, Some("Sara")).unwrap();
        r.claim(&id, &e.token, key(1), ok);
        assert_eq!(r.claim(&id, &e.token, key(1), ok), Claimed::Ok);
    }

    #[test]
    fn a_used_person_link_refuses_another_computer() {
        let (mut r, id) = setup();
        let e = r.create_link(&id, LinkKind::Person, Some("Sara")).unwrap();
        r.claim(&id, &e.token, key(1), ok);
        assert_eq!(
            r.claim(&id, &e.token, key(2), ok),
            Claimed::Denied("This link was already used on another computer.")
        );
        assert!(!r.allowed(&id, &key(2)));
    }

    #[test]
    fn a_removed_link_is_no_longer_valid() {
        let (mut r, id) = setup();
        let e = r.create_link(&id, LinkKind::Person, Some("Sara")).unwrap();
        r.remove_access(&id, &e.id);
        assert_eq!(
            r.claim(&id, &e.token, key(1), ok),
            Claimed::Denied("This link is no longer valid.")
        );
    }

    #[test]
    fn a_wrong_token_is_no_longer_valid() {
        let (mut r, id) = setup();
        r.create_link(&id, LinkKind::Person, Some("Sara")).unwrap();
        assert_eq!(
            r.claim(&id, &[0; 32], key(1), ok),
            Claimed::Denied("This link is no longer valid.")
        );
        assert_eq!(
            r.claim(&[9; 16], &[0; 32], key(1), ok),
            Claimed::Denied("This link is no longer valid.")
        );
    }

    #[test]
    fn the_binding_is_saved_before_the_reply() {
        let (mut r, id) = setup();
        let e = r.create_link(&id, LinkKind::Person, Some("Sara")).unwrap();
        let mut seen = None;
        let res = r.claim(&id, &e.token, key(1), |reg| {
            seen = Some(reg.shares[0].access[0].bound);
            Ok(())
        });
        assert_eq!(res, Claimed::Ok);
        assert_eq!(seen, Some(Some(key(1))));

        let (mut r, id) = setup();
        let e = r.create_link(&id, LinkKind::Person, Some("Sara")).unwrap();
        let res = r.claim(&id, &e.token, key(1), |_| Err("disk".into()));
        assert_eq!(res, Claimed::Denied("Sharing settings couldn't be saved."));
        assert!(r.shares[0].access[0].bound.is_none());
        assert!(!r.allowed(&id, &key(1)));
    }

    #[test]
    fn anyone_links_record_each_visitor() {
        let (mut r, id) = setup();
        let e = r.create_link(&id, LinkKind::Anyone, None).unwrap();
        let again = r.create_link(&id, LinkKind::Anyone, None).unwrap();
        assert_eq!(e.id, again.id);
        r.claim(&id, &e.token, key(1), ok);
        r.claim(&id, &e.token, key(2), ok);
        r.claim(&id, &e.token, key(2), ok);
        assert_eq!(r.shares[0].access[0].visitors, vec![key(1), key(2)]);
    }

    #[test]
    fn removing_the_anyone_link_cuts_off_its_visitors() {
        let (mut r, id) = setup();
        let e = r.create_link(&id, LinkKind::Anyone, None).unwrap();
        r.claim(&id, &e.token, key(1), ok);
        r.claim(&id, &e.token, key(2), ok);
        let gone = r.remove_access(&id, &e.id);
        assert_eq!(gone, vec![key(1), key(2)]);
        assert!(!r.allowed(&id, &key(1)));
        assert!(!r.allowed(&id, &key(2)));
    }

    #[test]
    fn removing_one_person_keeps_the_others() {
        let (mut r, id) = setup();
        let a = r.create_link(&id, LinkKind::Person, Some("A")).unwrap();
        let b = r.create_link(&id, LinkKind::Person, Some("B")).unwrap();
        r.claim(&id, &a.token, key(1), ok);
        r.claim(&id, &b.token, key(2), ok);
        assert_eq!(r.remove_access(&id, &a.id), vec![key(1)]);
        assert!(!r.allowed(&id, &key(1)));
        assert!(r.allowed(&id, &key(2)));
        assert_eq!(r.stop_sharing(&id), vec![key(2)]);
        assert!(r.shares.is_empty());
    }

    #[test]
    fn only_local_folders_can_be_shared() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("f.txt");
        std::fs::write(&file, "x").unwrap();
        let trash = tmp.path().join("Trash");
        std::fs::create_dir(&trash).unwrap();
        let ver = tmp.path().join(".versions").join("x");
        std::fs::create_dir_all(&ver).unwrap();
        let link = tmp.path().join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(tmp.path(), &link).unwrap();
        let mut r = Registry::default();
        let is_trash = |p: &Path| p == trash;
        let msg = Err("Only folders on this computer can be shared.".to_string());
        for bad in [
            file.as_path(),
            trash.as_path(),
            ver.as_path(),
            Path::new("share://a/b"),
            Path::new("gdrive://a/b"),
            Path::new("relative"),
            Path::new("/no/such/folder"),
            #[cfg(unix)]
            link.as_path(),
        ] {
            assert_eq!(r.share_folder(bad, is_trash), msg, "{bad:?}");
        }
        assert!(r.shares.is_empty());
        assert!(r.share_folder(tmp.path(), is_trash).is_ok());
    }

    #[test]
    fn sharing_a_folder_twice_gives_the_same_share() {
        let tmp = tempfile::tempdir().unwrap();
        let mut r = Registry::default();
        let a = r.share_folder(tmp.path(), |_| false).unwrap();
        let b = r.share_folder(tmp.path(), |_| false).unwrap();
        assert_eq!(a, b);
        assert_eq!(r.shares.len(), 1);
    }

    #[test]
    fn a_moved_folder_keeps_its_share() {
        let mut r = Registry::default();
        r.shares.push(Share {
            id: [1; 16],
            path: "/a/photos".into(),
            access: vec![],
        });
        r.shares.push(Share {
            id: [2; 16],
            path: "/other".into(),
            access: vec![],
        });
        r.follow_move(Path::new("/a/photos"), Path::new("/a/pics"));
        assert_eq!(r.shares[0].path, Path::new("/a/pics"));
        r.follow_move(Path::new("/a"), Path::new("/b"));
        assert_eq!(r.shares[0].path, Path::new("/b/pics"));
        assert_eq!(r.shares[1].path, Path::new("/other"));
    }

    #[test]
    fn the_registry_file_is_private() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("shares.json");
        let (mut r, id) = setup();
        let e = r.create_link(&id, LinkKind::Person, Some("Sara")).unwrap();
        r.claim(&id, &e.token, key(1), ok);
        r.name = Some("Me".into());
        r.save(&file).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let back = Registry::load(&file);
        assert_eq!(back.name.as_deref(), Some("Me"));
        assert_eq!(back.shares[0].access[0].token, e.token);
        assert!(back.allowed(&id, &key(1)));
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains(&to_hex(&e.token)));
    }

    #[test]
    fn the_same_link_twice_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let mut l = ReceivedList::default();
        let me = key(9);
        let first = l.add(link_for(1, 1), "Photos".into(), "Sara".into(), &me);
        assert!(first.is_ok());
        assert_eq!(
            l.add(link_for(1, 1), "Photos".into(), "Sara".into(), &me)
                .unwrap_err(),
            "This folder is already in Network."
        );
        assert!(l
            .add(link_for(1, 2), "B".into(), "Sara".into(), &me)
            .is_ok());
        let file = tmp.path().join("received.json");
        l.save(&file).unwrap();
        let mut back = ReceivedList::load(&file);
        assert_eq!(back.items.len(), 2);
        let id = back.items[0].id.clone();
        back.remove(&id);
        assert_eq!(back.items.len(), 1);
    }

    #[test]
    fn your_own_link_is_refused() {
        let mut l = ReceivedList::default();
        assert_eq!(
            l.add(link_for(9, 1), "P".into(), "Me".into(), &key(9))
                .unwrap_err(),
            "This is your own shared folder."
        );
    }
}
