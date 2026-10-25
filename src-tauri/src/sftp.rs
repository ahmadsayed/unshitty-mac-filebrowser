//! A small, read-only SFTP client for Network > Connect to Server.
//! SSH keys and the agent are tried first; the UI supplies a password when
//! neither works. Host keys are checked against the system and app known-hosts
//! files; an unknown host is only accepted after an explicit trust decision.

use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::Serialize;
use ssh2::{CheckResult, HashType, KnownHostFileKind, Session};
use std::collections::HashMap;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use url::Url;

pub fn is_sftp(path: &str) -> bool {
    path.starts_with("sftp://")
}

#[derive(Debug, Clone, Serialize)]
pub struct ServerInfo {
    pub id: String,
    pub label: String,
    pub url: String,
    pub host: String,
    pub port: u16,
    pub user: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ConnectOutcome {
    Connected { server: ServerInfo },
    AuthNeeded { server: ServerInfo },
    HostKeyUnknown { server: ServerInfo, fingerprint: String },
}

struct Target {
    user: String,
    host: String,
    port: u16,
    path: String,
    id: String,
    label: String,
    url: String,
    root_url: String,
}

struct SftpConn {
    session: Session,
    info: ServerInfo,
}

enum HostDecision {
    Known,
    Unknown(String),
}

pub struct SftpState {
    known_hosts: PathBuf,
    home: PathBuf,
    cache_dir: PathBuf,
    conns: Mutex<HashMap<String, Arc<Mutex<SftpConn>>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn list_entry(base: &str, remote: &Path, stat: &ssh2::FileStat) -> Option<crate::fs_ops::Entry> {
    let name = remote.file_name()?.to_str()?;
    if name == "." || name == ".." {
        return None;
    }
    let encoded = utf8_percent_encode(name, NON_ALPHANUMERIC).to_string();
    Some(crate::fs_ops::Entry {
        name: name.to_string(),
        path: format!("{base}/{encoded}"),
        is_dir: stat.is_dir(),
        size: stat.size.unwrap_or(0),
        modified: stat.mtime.unwrap_or(0) as i64,
        is_hidden: name.starts_with('.'),
        is_symlink: stat.file_type().is_symlink(),
        cloud: None,
    })
}

fn parse_target(input: &str) -> Result<Target, String> {
    let text = input.trim();
    if text.is_empty() {
        return Err("Enter a server, for example user@host.".into());
    }
    let raw = if text.contains("://") {
        text.to_string()
    } else {
        format!("sftp://{text}")
    };
    let mut url = Url::parse(&raw).map_err(|_| "That isn't a valid server address.".to_string())?;
    if url.scheme() != "sftp" {
        return Err("Only sftp:// addresses are supported.".into());
    }
    let user = url.username().to_string();
    if user.is_empty() {
        return Err("Enter the SSH user, for example user@host.".into());
    }
    let host = url
        .host_str()
        .filter(|host| !host.is_empty())
        .ok_or_else(|| "Enter the SSH host, for example user@host.".to_string())?
        .to_string();
    let port = url.port().unwrap_or(22);
    let path = percent_encoding::percent_decode_str(url.path())
        .decode_utf8()
        .map_err(|_| "The server path isn't valid text.".to_string())?
        .to_string();
    let path = if path.is_empty() { "/".into() } else { path };
    let id = format!("{user}@{host}:{port}");
    let label = if port == 22 {
        format!("{user}@{host}")
    } else {
        format!("{user}@{host}:{port}")
    };
    url.set_path("/");
    let root_url = url.to_string();
    url.set_path(&path);
    let url = url.to_string();
    Ok(Target { user, host, port, path, id, label, url, root_url })
}

impl SftpState {
    pub fn new(known_hosts: PathBuf, home: PathBuf, cache_dir: PathBuf) -> Self {
        Self {
            known_hosts,
            home,
            cache_dir,
            conns: Mutex::new(HashMap::new()),
        }
    }

    pub fn connect(
        &self,
        input: &str,
        password: Option<String>,
        trust_host: bool,
    ) -> Result<ConnectOutcome, String> {
        let target = parse_target(input)?;
        let addr = (target.host.as_str(), target.port);
        let tcp = TcpStream::connect(addr)
            .map_err(|e| format!("Could not reach {}: {e}", target.label))?;
        tcp.set_read_timeout(Some(Duration::from_secs(20))).ok();
        tcp.set_write_timeout(Some(Duration::from_secs(20))).ok();
        let mut session = Session::new().map_err(|e| e.to_string())?;
        session.set_tcp_stream(tcp);
        session.handshake().map_err(|e| format!("SSH handshake failed: {e}"))?;

        let mut server = ServerInfo {
            id: target.id.clone(),
            label: target.label.clone(),
            url: target.root_url.clone(),
            host: target.host.clone(),
            port: target.port,
            user: target.user.clone(),
        };
        match self.check_host_key(&session, &target, trust_host)? {
            HostDecision::Unknown(fingerprint) => {
                return Ok(ConnectOutcome::HostKeyUnknown { server, fingerprint });
            }
            HostDecision::Known => {}
        }
        if !self.authenticate(&session, &target.user, password.as_deref()) {
            return Ok(ConnectOutcome::AuthNeeded { server });
        }
        // Start in the user's home folder unless the URL named a path.
        server.url = start_url(&session, &target, &server.url);
        let conn = Arc::new(Mutex::new(SftpConn {
            session,
            info: server.clone(),
        }));
        lock(&self.conns).insert(server.id.clone(), conn);
        Ok(ConnectOutcome::Connected { server })
    }

    /// `Ok(Some(fingerprint))` means the key is unknown and the caller should
    /// ask; `Ok(None)` means the connection can proceed.
    fn check_host_key(
        &self,
        session: &Session,
        target: &Target,
        trust: bool,
    ) -> Result<HostDecision, String> {
        let (key, kind) = session
            .host_key()
            .ok_or_else(|| "The server didn't present a host key.".to_string())?;
        let mut known = self.known_hosts(session)?;
        let host = known_host_name(&target.host, target.port);
        match known.check_port(&target.host, target.port, key) {
            CheckResult::Match => Ok(HostDecision::Known),
            CheckResult::Mismatch => Err(
                "The server's host key changed. This could be an attack; connection refused.".into(),
            ),
            CheckResult::NotFound if !trust => {
                Ok(HostDecision::Unknown(fingerprint(session)))
            }
            CheckResult::NotFound => {
                self.save_host_key(&mut known, &host, key, kind)?;
                Ok(HostDecision::Known)
            }
            CheckResult::Failure => Err("Could not check the server's host key.".into()),
        }
    }

    fn known_hosts(&self, session: &Session) -> Result<ssh2::KnownHosts, String> {
        let mut known = session.known_hosts().map_err(|e| e.to_string())?;
        if let Some(system) = self.system_known_hosts() {
            let _ = known.read_file(&system, KnownHostFileKind::OpenSSH);
        }
        if self.known_hosts.exists() {
            let _ = known.read_file(&self.known_hosts, KnownHostFileKind::OpenSSH);
        }
        Ok(known)
    }

    fn save_host_key(
        &self,
        known: &mut ssh2::KnownHosts,
        host: &str,
        key: &[u8],
        kind: ssh2::HostKeyType,
    ) -> Result<(), String> {
        if let Some(parent) = self.known_hosts.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        known
            .add(host, key, "File Browser", kind.into())
            .map_err(|e| e.to_string())?;
        known
            .write_file(&self.known_hosts, KnownHostFileKind::OpenSSH)
            .map_err(|e| e.to_string())
    }

    fn system_known_hosts(&self) -> Option<PathBuf> {
        let file = self.home.join(".ssh/known_hosts");
        file.is_file().then_some(file)
    }

    fn authenticate(&self, session: &Session, user: &str, password: Option<&str>) -> bool {
        self.try_agent(session, user)
            || self.try_keys(session, user, password)
            || self.try_password(session, user, password)
    }

    fn try_agent(&self, session: &Session, user: &str) -> bool {
        session.userauth_agent(user).is_ok() && session.authenticated()
    }

    fn try_keys(&self, session: &Session, user: &str, passphrase: Option<&str>) -> bool {
        for name in ["id_ed25519", "id_ecdsa", "id_rsa"] {
            if self.try_key(session, user, name, passphrase) {
                return true;
            }
        }
        false
    }

    fn try_key(
        &self,
        session: &Session,
        user: &str,
        name: &str,
        passphrase: Option<&str>,
    ) -> bool {
        let private = self.home.join(".ssh").join(name);
        if !private.is_file() {
            return false;
        }
        let public = PathBuf::from(format!("{}.pub", private.display()));
        let public = public.is_file().then_some(public);
        for passphrase in [None, passphrase.filter(|pass| !pass.is_empty())] {
            if session
                .userauth_pubkey_file(user, public.as_deref(), &private, passphrase)
                .is_ok()
                && session.authenticated()
            {
                return true;
            }
        }
        false
    }

    fn try_password(&self, session: &Session, user: &str, password: Option<&str>) -> bool {
        let Some(password) = password.filter(|password| !password.is_empty()) else {
            return false;
        };
        session.userauth_password(user, password).is_ok() && session.authenticated()
    }

    pub fn servers(&self) -> Vec<ServerInfo> {
        let conns = lock(&self.conns);
        let mut out: Vec<ServerInfo> = conns
            .values()
            .map(|conn| lock(conn).info.clone())
            .collect();
        out.sort_by(|a, b| a.label.cmp(&b.label));
        out
    }

    pub fn disconnect(&self, id: &str) {
        lock(&self.conns).remove(id);
    }

    pub fn list(&self, path: &str, show_hidden: bool) -> Result<Vec<crate::fs_ops::Entry>, String> {
        let target = parse_target(path)?;
        let conn = self.connection(&target.id)?;
        let conn = lock(&conn);
        let sftp = conn.session.sftp().map_err(|e| e.to_string())?;
        let entries = sftp
            .readdir(Path::new(&target.path))
            .map_err(|e| format!("Could not read {}: {e}", target.path))?;
        let base = target.url.trim_end_matches('/');
        Ok(entries
            .into_iter()
            .filter_map(|(remote, stat)| list_entry(base, &remote, &stat))
            .filter(|entry| show_hidden || !entry.is_hidden)
            .collect())
    }

    pub fn open(&self, path: &str) -> Result<String, String> {
        let target = parse_target(path)?;
        let conn = self.connection(&target.id)?;
        let conn = lock(&conn);
        self.download(&conn.session, &target)
    }

    fn download(&self, session: &Session, target: &Target) -> Result<String, String> {
        let name = target
            .path
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty() && *name != "." && *name != "..")
            .ok_or_else(|| "Only files can be opened.".to_string())?;
        let sftp = session.sftp().map_err(|e| e.to_string())?;
        let mut remote = sftp
            .open(Path::new(&target.path))
            .map_err(|e| format!("Could not open the file: {e}"))?;
        let dir = self.cache_dir.join("sftp-open").join(unique_dir());
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let dest = dir.join(name);
        let mut local = std::fs::File::create(&dest).map_err(|e| e.to_string())?;
        if let Err(e) = std::io::copy(&mut remote, &mut local) {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(format!("Could not download the file: {e}"));
        }
        Ok(dest.to_string_lossy().to_string())
    }

    fn connection(&self, id: &str) -> Result<Arc<Mutex<SftpConn>>, String> {
        lock(&self.conns)
            .get(id)
            .cloned()
            .ok_or_else(|| "Connect to this server first.".to_string())
    }
}

fn known_host_name(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_string()
    } else {
        format!("[{host}]:{port}")
    }
}

fn fingerprint(session: &Session) -> String {
    session
        .host_key_hash(HashType::Sha256)
        .map(|hash| format!("SHA256:{}", data_encoding::BASE64.encode(hash)))
        .unwrap_or_else(|| "unknown".into())
}

fn start_url(session: &Session, target: &Target, root: &str) -> String {
    let path = if target.path == "/" {
        session
            .sftp()
            .and_then(|sftp| sftp.realpath(Path::new(".")))
            .map(|home| home.to_string_lossy().to_string())
            .unwrap_or_else(|_| "/".into())
    } else {
        target.path.clone()
    };
    with_path(root, &path)
}

fn with_path(root: &str, path: &str) -> String {
    match Url::parse(root) {
        Ok(mut url) => {
            url.set_path(path);
            url.to_string()
        }
        Err(_) => root.to_string(),
    }
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

    #[test]
    fn sftp_addresses_are_parsed_with_or_without_a_scheme() {
        let target = parse_target("user@host").unwrap();
        assert_eq!((target.user.as_str(), target.host.as_str(), target.port), ("user", "host", 22));
        assert_eq!(target.path, "/");
        assert_eq!(target.id, "user@host:22");
        assert_eq!(target.root_url, "sftp://user@host/");

        let target = parse_target("sftp://me@example.com:2222/docs").unwrap();
        assert_eq!(target.port, 2222);
        assert_eq!(target.path, "/docs");
        assert_eq!(target.id, "me@example.com:2222");
    }

    #[test]
    fn sftp_addresses_need_a_user_and_the_right_scheme() {
        assert!(parse_target("host").is_err());
        assert!(parse_target("ssh://user@host").is_err());
        assert!(parse_target("").is_err());
    }

    #[test]
    fn host_key_names_include_nonstandard_ports() {
        assert_eq!(known_host_name("host", 22), "host");
        assert_eq!(known_host_name("host", 2222), "[host]:2222");
    }
}
