//! Keep Versions: content-addressed objects plus a per-folder SQLite index.
//!
//! Step 1 of the design (`docs/superpowers/specs/2026-10-24-keep-versions-design.md`)
//! is the storage layer only: `index` owns the schema and queries, `store`
//! owns the zstd object files. Both are plain functions so they can be tested
//! without Tauri. Paths are relative to a versioned folder and kept as raw
//! bytes; the display string is for the UI only.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub mod capture;
pub mod history;
pub mod index;
pub mod registry;
pub mod restore;
pub mod state;
pub mod store;
pub mod watch;

/// Written into the index's `meta` table and `.versions/meta.json`.
pub const APP_ID: &str = "unshitty-mac-filebrowser";

/// Schema version stored in both `meta.schema_version` and `PRAGMA user_version`.
pub const SCHEMA_VERSION: i64 = 1;

/// Files larger than this are not versioned (D5).
pub const MAX_FILE_SIZE: u64 = 1 << 30;

/// Deltas buffer base + new; this bounds both (D5).
pub const MAX_DELTA_SIZE: u64 = 128 << 20;

/// Deepest delta chain; a new delta would have depth 20 at the cap (D6).
pub const MAX_DELTA_DEPTH: i64 = 19;

/// A delta at least this fraction of the raw size is stored as a full copy (D6).
pub const FULL_COPY_RATIO_PERCENT: u64 = 90;

/// The hidden directory that holds a folder's history.
pub const VERSIONS_DIR: &str = ".versions";
pub const INDEX_FILE: &str = "index.sqlite";
pub const META_FILE: &str = "meta.json";
pub const LOCK_FILE: &str = "lock";
pub const OBJECTS_DIR: &str = "objects";
pub const TMP_DIR: &str = "tmp";

/// Errors from the storage layer. The command layer adds GNOME wording; these
/// strings are also used in tests, so keep them stable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The file changed while it was being read; capture should re-queue it.
    Changed,
    /// A stored object is missing, truncated, expands past its recorded size,
    /// or fails its hash check.
    Damaged,
    /// The index is missing tables, has a newer schema, or holds invalid rows.
    IndexDamaged(String),
    /// A `.versions/` directory without our marker was found.
    NotOurs(String),
    /// The history is open read-only because another instance holds the lock.
    ReadOnly,
    /// WAL mode is unavailable (network/cloud volume) (D18).
    NotLocal,
    /// The file is over the capture limit (D5).
    TooLarge,
    Io(String),
    Db(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Changed => write!(f, "The file changed while it was being read"),
            Error::Damaged => write!(f, "This version is damaged and can't be restored."),
            Error::IndexDamaged(detail) => {
                if detail.is_empty() {
                    write!(f, "This folder's version history is damaged.")
                } else {
                    write!(f, "This folder's version history is damaged: {detail}")
                }
            }
            Error::NotOurs(detail) => write!(f, "Not a version history this app owns: {detail}"),
            Error::ReadOnly => write!(
                f,
                "The version history of this folder is in use by another window."
            ),
            Error::NotLocal => write!(f, "Version history needs a folder on a local disk."),
            Error::TooLarge => write!(f, "The file is too large to keep versions of."),
            Error::Io(detail) => write!(f, "Could not read or write version history: {detail}"),
            Error::Db(detail) => write!(f, "Could not read or write the version index: {detail}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e.to_string())
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Db(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    Full,
    Delta,
}

impl ObjectKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ObjectKind::Full => "full",
            ObjectKind::Delta => "delta",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "full" => Some(ObjectKind::Full),
            "delta" => Some(ObjectKind::Delta),
            _ => None,
        }
    }
}

/// One row of `objects`, with no knowledge of paths or versions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectMeta {
    pub hash: String,
    pub kind: ObjectKind,
    pub base: Option<String>,
    pub depth: i64,
    pub size: u64,
    pub stored_size: u64,
}

impl ObjectMeta {
    /// Check the invariants the schema expresses plus the ones SQLite cannot:
    /// the base is a valid hash and depths are in range.
    pub fn validate(&self) -> Result<()> {
        if !is_valid_hash(&self.hash) {
            return Err(Error::IndexDamaged(
                "object hash is not 64 hex characters".into(),
            ));
        }
        match (self.kind, self.base.as_deref(), self.depth) {
            (ObjectKind::Full, None, 0) => {}
            (ObjectKind::Delta, Some(base), 1..=MAX_DELTA_DEPTH) if is_valid_hash(base) => {}
            _ => {
                return Err(Error::IndexDamaged(
                    "object kind, base or depth is inconsistent".into(),
                ));
            }
        }
        Ok(())
    }
}

/// One row of `versions`. `hash == None` is a tombstone (D17).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionRow {
    pub id: i64,
    pub path: Vec<u8>,
    pub display: String,
    pub hash: Option<String>,
    pub size: u64,
    pub mtime: i64,
    pub recorded_at: i64,
}

impl VersionRow {
    pub fn is_tombstone(&self) -> bool {
        self.hash.is_none()
    }
}

/// A deleted path, for the Deleted Files… dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletedRow {
    pub path: Vec<u8>,
    pub display: String,
    pub deleted_at: i64,
    /// Size of the last non-tombstone version.
    pub size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct HistoryInfo {
    /// Sum of `objects.stored_size` (compressed object files).
    pub bytes: u64,
    /// Rows that are not tombstones.
    pub versions: u64,
    /// Distinct paths with at least one non-tombstone version.
    pub files: u64,
}

/// One row of the Versions… list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VersionInfo {
    pub id: i64,
    pub size: u64,
    /// Unix seconds, for display.
    pub mtime: i64,
    pub recorded_at: i64,
    pub deleted: bool,
    /// The latest row and the file's current size/mtime agree (no hashing).
    pub current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VersionDiff {
    Text {
        lines: Vec<DiffLine>,
        truncated: bool,
    },
    Same,
    Binary,
    TooLarge,
    Deleted,
    NotOnDisk,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiffLine {
    /// ' ', '-' or '+'.
    pub tag: char,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeletedFile {
    pub path: String,
    pub deleted_at: i64,
    pub size: u64,
}

// ---------------------------------------------------------------------------
// Paths and names
// ---------------------------------------------------------------------------

/// A BLAKE3 hash in the form stored in object names: 64 lowercase hex.
pub fn is_valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A folder id is a 128-bit value in lowercase hex.
pub fn is_valid_folder_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Relative paths are `/`-separated bytes with no empty, `.`, `..` or
/// `.versions` component and no NUL. They are used for both the index and
/// object names, so validate before every write (D15).
pub fn validate_rel_path(path: &[u8]) -> Result<()> {
    if path.is_empty() {
        return Err(Error::IndexDamaged("path is empty".into()));
    }
    if path.contains(&0) {
        return Err(Error::IndexDamaged("path contains NUL".into()));
    }
    for part in path.split(|b| *b == b'/') {
        if part.is_empty() || part == b"." || part == b".." {
            return Err(Error::IndexDamaged(
                "path has an empty or dot component".into(),
            ));
        }
        if part == VERSIONS_DIR.as_bytes() {
            return Err(Error::IndexDamaged("path is inside .versions".into()));
        }
    }
    Ok(())
}

/// The path as the UI shows it. Files whose names are not UTF-8 are recorded
/// with exact bytes but cannot be addressed from the UI in v1 (D15).
pub fn display_path(path: &[u8]) -> String {
    String::from_utf8_lossy(path).into_owned()
}

/// Turn raw relative-path bytes into an OS path component sequence.
pub fn join_rel(root: &Path, rel: &[u8]) -> PathBuf {
    root.join(rel_path(rel))
}

/// Raw relative-path bytes as a standalone OS path.
pub fn rel_path(rel: &[u8]) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        PathBuf::from(std::ffi::OsString::from_vec(rel.to_vec()))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(String::from_utf8_lossy(rel).into_owned())
    }
}

/// True when any component is the reserved `.versions` directory (D10).
pub fn is_internal_path(path: &Path) -> bool {
    path.components()
        .any(|part| part.as_os_str().to_string_lossy() == VERSIONS_DIR)
}

/// Raw bytes of a relative path, for storing in `versions.path`.
#[cfg(unix)]
pub fn path_bytes(rel: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    rel.as_os_str().as_bytes().to_vec()
}

#[cfg(not(unix))]
pub fn path_bytes(rel: &Path) -> Vec<u8> {
    rel.to_string_lossy().as_bytes().to_vec()
}

/// The half-open BLOB range of a directory's children, for the
/// `versions_by_path` index. `dir` must be a non-empty relative path; use
/// `None` for the root.
///
/// Children are `dir + '/' + rest`. The lower bound is `dir + '/'` and the
/// upper bound is `dir + '0'`: every child compares below it because `'/'`
/// (0x2F) is less than `'0'` (0x30), regardless of `rest`. A sibling such as
/// `dir-other` is outside the range.
pub fn child_range(dir: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut lower = Vec::with_capacity(dir.len() + 1);
    lower.extend_from_slice(dir);
    lower.push(b'/');
    let mut upper = Vec::with_capacity(dir.len() + 1);
    upper.extend_from_slice(dir);
    upper.push(b'0');
    (lower, upper)
}

/// The `.versions` directory of a versioned root.
pub fn history_dir(root: &Path) -> PathBuf {
    root.join(VERSIONS_DIR)
}

pub fn meta_path(root: &Path) -> PathBuf {
    history_dir(root).join(META_FILE)
}

// ---------------------------------------------------------------------------
// Ownership marker
// ---------------------------------------------------------------------------

/// The cheap ownership gate in `.versions/meta.json` (D14, R1). It mirrors
/// the `meta` table so `versioned_root` never has to open SQLite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnerMarker {
    pub app: String,
    pub folder_id: String,
    pub schema_version: i64,
}

pub fn read_marker(root: &Path) -> Result<OwnerMarker> {
    let path = meta_path(root);
    let text =
        std::fs::read_to_string(&path).map_err(|_| Error::NotOurs("missing meta.json".into()))?;
    let marker: OwnerMarker =
        serde_json::from_str(&text).map_err(|_| Error::NotOurs("invalid meta.json".into()))?;
    if marker.app != APP_ID {
        return Err(Error::NotOurs("meta.json names another app".into()));
    }
    if !is_valid_folder_id(&marker.folder_id) {
        return Err(Error::NotOurs("meta.json has an invalid folder id".into()));
    }
    if marker.schema_version > SCHEMA_VERSION {
        return Err(Error::IndexDamaged(
            "This folder's history was made by a newer version of the app.".into(),
        ));
    }
    if marker.schema_version != SCHEMA_VERSION {
        return Err(Error::IndexDamaged("unsupported meta.json schema".into()));
    }
    Ok(marker)
}

pub fn write_marker(root: &Path, folder_id: &str) -> Result<()> {
    if !is_valid_folder_id(folder_id) {
        return Err(Error::NotOurs("folder id is not 128-bit hex".into()));
    }
    let dir = history_dir(root);
    std::fs::create_dir_all(&dir)?;
    let marker = OwnerMarker {
        app: APP_ID.to_string(),
        folder_id: folder_id.to_string(),
        schema_version: SCHEMA_VERSION,
    };
    let json = serde_json::to_vec_pretty(&marker)
        .map_err(|e| Error::Io(format!("could not encode meta.json: {e}")))?;
    let tmp = dir.join(format!("{META_FILE}.tmp"));
    std::fs::write(&tmp, json)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, meta_path(root))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Limits and helpers
// ---------------------------------------------------------------------------

/// zstd level: 19 for small files (good ratio, still fast), 6 above (D6).
pub fn compression_level(size: u64) -> i32 {
    if size <= 1 << 20 { 19 } else { 6 }
}

/// Window log that covers `total` bytes, capped at 28 (256 MiB).
pub fn window_log_for(total: u64) -> u32 {
    let mut log = 10;
    while log < 28 && (1u64 << log) < total {
        log += 1;
    }
    log
}

/// Whether a delta at `stored` bytes is worth keeping over a full copy of
/// `raw` bytes. A delta at 90 % or more is replaced by a full copy (D6).
pub fn delta_is_worthwhile(stored: u64, raw: u64) -> bool {
    raw > 0 && stored.saturating_mul(100) < raw.saturating_mul(FULL_COPY_RATIO_PERCENT)
}

pub fn unix_seconds(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn now_seconds() -> i64 {
    unix_seconds(SystemTime::now())
}

/// File modification time in Unix nanoseconds (0 when unavailable).
pub fn mtime_nanos(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_and_folder_ids_are_strict() {
        assert!(is_valid_hash(&"a".repeat(64)));
        assert!(is_valid_hash(&"0123456789abcdef".repeat(4)));
        assert!(!is_valid_hash(&"A".repeat(64)));
        assert!(!is_valid_hash(&"g".repeat(64)));
        assert!(!is_valid_hash(&"a".repeat(63)));
        assert!(is_valid_folder_id(&"0".repeat(32)));
        assert!(!is_valid_folder_id(&"0".repeat(31)));
    }

    #[test]
    fn relative_paths_reject_traversal_and_internal_dirs() {
        assert!(validate_rel_path(b"notes.txt").is_ok());
        assert!(validate_rel_path(b"docs/notes.txt").is_ok());
        assert!(validate_rel_path(b".versions.txt").is_ok());
        assert!(validate_rel_path(b"").is_err());
        assert!(validate_rel_path(b"/abs").is_err());
        assert!(validate_rel_path(b"a//b").is_err());
        assert!(validate_rel_path(b"a/./b").is_err());
        assert!(validate_rel_path(b"a/../b").is_err());
        assert!(validate_rel_path(b"a/.versions/b").is_err());
        assert!(validate_rel_path(b"a\0b").is_err());
        // Non-UTF-8 names are valid bytes, not lossy strings.
        assert!(validate_rel_path(&[0x66, 0x80]).is_ok());
    }

    #[test]
    fn child_range_covers_every_child_but_not_siblings() {
        let (lower, upper) = child_range(b"foo");
        assert_eq!(lower, b"foo/");
        assert_eq!(upper, b"foo0");
        for child in [&b"foo/a"[..], b"foo/z", b"foo/!x", b"foo/0x", b"foo/ "] {
            assert!(child >= &lower[..] && child < &upper[..], "{child:?}");
        }
        for outside in [&b"foo"[..], b"foobar", b"foo-other", b"fo"] {
            assert!(
                !(outside >= &lower[..] && outside < &upper[..]),
                "{outside:?}"
            );
        }
    }

    #[test]
    fn window_and_level_are_bounded() {
        assert_eq!(compression_level(1 << 20), 19);
        assert_eq!(compression_level((1 << 20) + 1), 6);
        assert_eq!(window_log_for(0), 10);
        assert_eq!(window_log_for(1024), 10);
        assert_eq!(window_log_for(1 << 20), 20);
        assert_eq!(window_log_for(u64::MAX), 28);
    }

    #[test]
    fn delta_ratio_is_ninety_percent() {
        assert!(delta_is_worthwhile(89, 100));
        assert!(!delta_is_worthwhile(90, 100));
        assert!(!delta_is_worthwhile(100, 100));
        assert!(!delta_is_worthwhile(0, 0));
    }

    #[test]
    fn object_metadata_enforces_the_chain_shape() {
        let full = ObjectMeta {
            hash: "a".repeat(64),
            kind: ObjectKind::Full,
            base: None,
            depth: 0,
            size: 10,
            stored_size: 8,
        };
        assert!(full.validate().is_ok());
        let delta = ObjectMeta {
            hash: "b".repeat(64),
            kind: ObjectKind::Delta,
            base: Some("a".repeat(64)),
            depth: 1,
            size: 10,
            stored_size: 8,
        };
        assert!(delta.validate().is_ok());
        assert!(
            ObjectMeta {
                depth: 20,
                ..delta.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            ObjectMeta {
                base: None,
                ..delta.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            ObjectMeta {
                hash: "x".into(),
                ..full
            }
            .validate()
            .is_err()
        );
    }
}
