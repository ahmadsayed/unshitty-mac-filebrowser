//! The SQLite index inside `.versions/index.sqlite`.
//!
//! The index is derived data, but it is the authoritative record of which
//! objects make up a file's history. Opening is split into a writer (owner
//! process) and a reader (a second instance, D16). Nothing here touches Tauri.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use crate::versions::{
    APP_ID, DeletedRow, Error, HistoryInfo, ObjectKind, ObjectMeta, Result, SCHEMA_VERSION,
    VersionRow, child_range, display_path, is_valid_folder_id, is_valid_hash, validate_rel_path,
};

const BUSY_TIMEOUT: Duration = Duration::from_millis(5000);
const DEFAULT_VERSION_LIMIT: i64 = 1000;
const MAX_VERSION_LIMIT: i64 = 5000;

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS objects (
  hash        TEXT PRIMARY KEY CHECK (length(hash) = 64),
  kind        TEXT NOT NULL CHECK (kind IN ('full', 'delta')),
  base        TEXT REFERENCES objects(hash),
  depth       INTEGER NOT NULL,
  size        INTEGER NOT NULL,
  stored_size INTEGER NOT NULL,
  codec       TEXT NOT NULL DEFAULT 'zstd',
  CHECK ((kind = 'full'  AND base IS NULL     AND depth = 0) OR
         (kind = 'delta' AND base IS NOT NULL AND depth BETWEEN 1 AND 19))
);

CREATE TABLE IF NOT EXISTS versions (
  id          INTEGER PRIMARY KEY,
  path        BLOB NOT NULL,
  display     TEXT NOT NULL,
  hash        TEXT REFERENCES objects(hash),
  size        INTEGER NOT NULL,
  mtime       INTEGER NOT NULL,
  recorded_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS versions_by_path ON versions(path, id);
"#;

// ---------------------------------------------------------------------------
// Opening
// ---------------------------------------------------------------------------

fn configure(conn: &Connection) -> Result<()> {
    conn.busy_timeout(BUSY_TIMEOUT)?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(())
}

fn schema_version(conn: &Connection) -> Result<i64> {
    Ok(conn.pragma_query_value(None, "user_version", |row| row.get(0))?)
}

fn journal_mode(conn: &Connection) -> Result<String> {
    Ok(conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))?)
}

fn check_schema(conn: &Connection) -> Result<()> {
    let version = schema_version(conn)?;
    if version > SCHEMA_VERSION {
        return Err(Error::IndexDamaged(
            "This folder's history was made by a newer version of the app.".into(),
        ));
    }
    if version == 0 {
        return Err(Error::IndexDamaged("the index has no schema".into()));
    }
    // Only v1 exists; older versions would migrate here.
    Ok(())
}

fn check_wal(conn: &Connection) -> Result<()> {
    if journal_mode(conn)?.eq_ignore_ascii_case("wal") {
        Ok(())
    } else {
        Err(Error::NotLocal)
    }
}

/// Create a brand-new history index. Refuses to touch an existing file, so a
/// partial or foreign `.versions/index.sqlite` is never adopted (D14).
pub fn create_index(path: &Path) -> Result<Connection> {
    if path.exists() {
        return Err(Error::NotOurs("the index already exists".into()));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    configure(&conn)?;
    // Journal mode cannot be changed inside a transaction.
    if !journal_mode(&conn)?.eq_ignore_ascii_case("wal") {
        // `PRAGMA journal_mode=WAL` returns the resulting mode, unlike the
        // no-assignment form which only reports it.
        let mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Err(Error::NotLocal);
        }
    }
    {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(SCHEMA_SQL)?;
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        tx.commit()?;
    }
    // A real write/read transaction, so a read-only or broken filesystem is
    // rejected at creation rather than at the first capture (D18).
    {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch("CREATE TABLE _probe (x INTEGER); INSERT INTO _probe VALUES (1);")?;
        let n: i64 = tx.query_row("SELECT COUNT(*) FROM _probe", [], |row| row.get(0))?;
        if n != 1 {
            return Err(Error::NotLocal);
        }
        tx.execute_batch("DROP TABLE _probe;")?;
        tx.commit()?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(conn)
}

/// Open an existing index for writing (the owner process).
pub fn open_writer(path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    configure(&conn)?;
    check_schema(&conn)?;
    check_wal(&conn)?;
    Ok(conn)
}

/// Open an existing index read-only (a second app instance, D16). Never runs
/// migrations, never writes, never clears `tmp/`.
pub fn open_reader(path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    conn.pragma_update(None, "query_only", "ON")?;
    check_schema(&conn)?;
    check_wal(&conn)?;
    Ok(conn)
}

// ---------------------------------------------------------------------------
// Ownership
// ---------------------------------------------------------------------------

/// Write the ownership marker. Called only after the schema and the WAL test
/// have passed, so a failed creation leaves an inert directory (D14, R6).
pub fn write_meta(conn: &Connection, folder_id: &str) -> Result<()> {
    if !is_valid_folder_id(folder_id) {
        return Err(Error::NotOurs("folder id is not 128-bit hex".into()));
    }
    let created_at = crate::versions::now_seconds().to_string();
    let schema_version = SCHEMA_VERSION.to_string();
    let tx = conn.unchecked_transaction()?;
    for (key, value) in [
        ("app", APP_ID),
        ("folder_id", folder_id),
        ("schema_version", schema_version.as_str()),
        ("created_at", created_at.as_str()),
    ] {
        tx.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub fn read_meta(conn: &Connection) -> Result<HashMap<String, String>> {
    let mut stmt = conn.prepare("SELECT key, value FROM meta")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut out = HashMap::new();
    for row in rows {
        let (k, v) = row?;
        out.insert(k, v);
    }
    Ok(out)
}

/// The index belongs to this app and is structurally usable.
pub fn verify_owner(conn: &Connection) -> Result<()> {
    let meta = read_meta(conn)?;
    if meta.get("app").map(String::as_str) != Some(APP_ID) {
        return Err(Error::NotOurs("missing app marker".into()));
    }
    let Some(folder_id) = meta.get("folder_id") else {
        return Err(Error::NotOurs("missing folder id".into()));
    };
    if !is_valid_folder_id(folder_id) {
        return Err(Error::NotOurs("invalid folder id".into()));
    }
    match meta
        .get("schema_version")
        .and_then(|value| value.parse::<i64>().ok())
    {
        Some(SCHEMA_VERSION) => Ok(()),
        Some(other) => Err(Error::IndexDamaged(format!(
            "unexpected schema_version {other}"
        ))),
        None => Err(Error::IndexDamaged("missing schema_version".into())),
    }
}

// ---------------------------------------------------------------------------
// Objects
// ---------------------------------------------------------------------------

pub fn get_object(conn: &Connection, hash: &str) -> Result<Option<ObjectMeta>> {
    if !is_valid_hash(hash) {
        return Err(Error::IndexDamaged(
            "object hash is not 64 hex characters".into(),
        ));
    }
    let row: Option<(String, String, Option<String>, i64, i64, i64)> = conn
        .query_row(
            "SELECT hash, kind, base, depth, size, stored_size FROM objects WHERE hash = ?1",
            params![hash],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()?;
    let Some((hash, kind, base, depth, size, stored_size)) = row else {
        return Ok(None);
    };
    let kind = ObjectKind::parse(&kind)
        .ok_or_else(|| Error::IndexDamaged(format!("unknown object kind {kind}")))?;
    let object = ObjectMeta {
        hash,
        kind,
        base,
        depth,
        size: to_u64(size)?,
        stored_size: to_u64(stored_size)?,
    };
    object.validate()?;
    Ok(Some(object))
}

pub fn insert_object(conn: &Connection, object: &ObjectMeta) -> Result<()> {
    object.validate()?;
    conn.execute(
        "INSERT OR IGNORE INTO objects
           (hash, kind, base, depth, size, stored_size)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            object.hash,
            object.kind.as_str(),
            object.base,
            object.depth,
            to_i64(object.size)?,
            to_i64(object.stored_size)?,
        ],
    )?;
    // A hash is content-addressed: an existing row must describe the same
    // bytes. Its storage kind may differ (full vs delta), which is fine.
    let stored = get_object(conn, &object.hash)?
        .ok_or_else(|| Error::IndexDamaged("object insert vanished".into()))?;
    if stored.size != object.size {
        return Err(Error::IndexDamaged(
            "object size does not match its hash".into(),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Versions
// ---------------------------------------------------------------------------

fn to_u64(value: i64) -> Result<u64> {
    u64::try_from(value).map_err(|_| Error::IndexDamaged("negative size in index".into()))
}

fn to_i64(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| Error::IndexDamaged("size does not fit in SQLite".into()))
}

fn version_from_tuple(
    id: i64,
    path: Vec<u8>,
    display: String,
    hash: Option<String>,
    size: i64,
    mtime: i64,
    recorded_at: i64,
) -> Result<VersionRow> {
    if let Some(hash) = &hash {
        if !is_valid_hash(hash) {
            return Err(Error::IndexDamaged(
                "version hash is not 64 hex characters".into(),
            ));
        }
    }
    Ok(VersionRow {
        id,
        path,
        display,
        hash,
        size: to_u64(size)?,
        mtime,
        recorded_at,
    })
}

type VersionTuple = (i64, Vec<u8>, String, Option<String>, i64, i64, i64);

fn tuple_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<VersionTuple> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
    ))
}

/// Insert a version row. A `hash` of `None` records a tombstone (D17).
pub fn insert_version(
    conn: &Connection,
    path: &[u8],
    hash: Option<&str>,
    size: u64,
    mtime: i64,
    recorded_at: i64,
) -> Result<i64> {
    validate_rel_path(path)?;
    if let Some(hash) = hash {
        if !is_valid_hash(hash) {
            return Err(Error::IndexDamaged(
                "version hash is not 64 hex characters".into(),
            ));
        }
    }
    let display = display_path(path);
    conn.execute(
        "INSERT INTO versions (path, display, hash, size, mtime, recorded_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![path, display, hash, to_i64(size)?, mtime, recorded_at],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn insert_tombstone(conn: &Connection, path: &[u8], recorded_at: i64) -> Result<i64> {
    insert_version(conn, path, None, 0, 0, recorded_at)
}

pub fn latest_version(conn: &Connection, path: &[u8]) -> Result<Option<VersionRow>> {
    let row: Option<VersionTuple> = conn
        .query_row(
            "SELECT id, path, display, hash, size, mtime, recorded_at
             FROM versions WHERE path = ?1 ORDER BY id DESC LIMIT 1",
            params![path],
            tuple_from_row,
        )
        .optional()?;
    row.map(|t| version_from_tuple(t.0, t.1, t.2, t.3, t.4, t.5, t.6))
        .transpose()
}

pub fn get_version(conn: &Connection, id: i64) -> Result<Option<VersionRow>> {
    let row: Option<VersionTuple> = conn
        .query_row(
            "SELECT id, path, display, hash, size, mtime, recorded_at
             FROM versions WHERE id = ?1",
            params![id],
            tuple_from_row,
        )
        .optional()?;
    row.map(|t| version_from_tuple(t.0, t.1, t.2, t.3, t.4, t.5, t.6))
        .transpose()
}

/// The newest row for `path` that is not a tombstone.
pub fn last_live_version(conn: &Connection, path: &[u8]) -> Result<Option<VersionRow>> {
    let row: Option<VersionTuple> = conn
        .query_row(
            "SELECT id, path, display, hash, size, mtime, recorded_at
             FROM versions
             WHERE path = ?1 AND hash IS NOT NULL
             ORDER BY id DESC LIMIT 1",
            params![path],
            tuple_from_row,
        )
        .optional()?;
    row.map(|t| version_from_tuple(t.0, t.1, t.2, t.3, t.4, t.5, t.6))
        .transpose()
}

/// Newest first. The UI renders at most 1,000; the command layer passes a
/// limit so a file with a very long history cannot flood the IPC channel (R8).
pub fn list_versions(
    conn: &Connection,
    path: &[u8],
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<Vec<VersionRow>> {
    let limit = limit.map_or(DEFAULT_VERSION_LIMIT, |n| i64::from(n));
    let limit = limit.clamp(1, MAX_VERSION_LIMIT);
    let offset = i64::from(offset.unwrap_or(0)).max(0);
    let mut stmt = conn.prepare(
        "SELECT id, path, display, hash, size, mtime, recorded_at
         FROM versions WHERE path = ?1 ORDER BY id DESC LIMIT ?2 OFFSET ?3",
    )?;
    let rows = stmt.query_map(params![path, limit, offset], tuple_from_row)?;
    let mut out = Vec::new();
    for row in rows {
        let t = row?;
        out.push(version_from_tuple(t.0, t.1, t.2, t.3, t.4, t.5, t.6)?);
    }
    Ok(out)
}

pub fn count_versions(conn: &Connection, path: &[u8]) -> Result<u64> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM versions WHERE path = ?1",
        params![path],
        |row| row.get(0),
    )?;
    to_u64(count)
}

/// The latest row for every path under `dir` (or the whole history when
/// `dir` is `None`), newest first. `dir` uses the indexed BLOB range (R7).
pub fn latest_versions_under(conn: &Connection, dir: Option<&[u8]>) -> Result<Vec<VersionRow>> {
    let tuples: Vec<VersionTuple> = match dir {
        Some(dir) => {
            validate_rel_path(dir)?;
            let (lower, upper) = child_range(dir);
            let mut stmt = conn.prepare(
                "SELECT v.id, v.path, v.display, v.hash, v.size, v.mtime, v.recorded_at
                 FROM versions v
                 JOIN (SELECT path, MAX(id) AS id FROM versions
                       WHERE path >= ?1 AND path < ?2 GROUP BY path) latest
                   ON v.id = latest.id
                 ORDER BY v.id DESC",
            )?;
            let rows = stmt.query_map(params![lower, upper], tuple_from_row)?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            out
        }
        None => {
            let mut stmt = conn.prepare(
                "SELECT v.id, v.path, v.display, v.hash, v.size, v.mtime, v.recorded_at
                 FROM versions v
                 JOIN (SELECT path, MAX(id) AS id FROM versions GROUP BY path) latest
                   ON v.id = latest.id
                 ORDER BY v.id DESC",
            )?;
            let rows = stmt.query_map([], tuple_from_row)?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            out
        }
    };
    tuples
        .into_iter()
        .map(|t| version_from_tuple(t.0, t.1, t.2, t.3, t.4, t.5, t.6))
        .collect()
}

fn last_non_tombstone_size(conn: &Connection, path: &[u8]) -> Result<u64> {
    let size: Option<i64> = conn
        .query_row(
            "SELECT size FROM versions
             WHERE path = ?1 AND hash IS NOT NULL ORDER BY id DESC LIMIT 1",
            params![path],
            |row| row.get(0),
        )
        .optional()?;
    size.map_or(Ok(0), to_u64)
}

/// Tombstoned paths under `dir` whose latest row is a tombstone, with the size
/// of the last real version. The caller decides whether something occupies the
/// path now.
pub fn deleted_files(conn: &Connection, dir: Option<&[u8]>) -> Result<Vec<DeletedRow>> {
    let mut out = Vec::new();
    for row in latest_versions_under(conn, dir)? {
        if !row.is_tombstone() {
            continue;
        }
        out.push(DeletedRow {
            path: row.path.clone(),
            display: row.display,
            deleted_at: row.recorded_at,
            size: last_non_tombstone_size(conn, &row.path)?,
        });
    }
    Ok(out)
}

pub fn update_mtime(conn: &Connection, id: i64, mtime: i64) -> Result<()> {
    conn.execute(
        "UPDATE versions SET mtime = ?1 WHERE id = ?2",
        params![mtime, id],
    )?;
    Ok(())
}

/// D20: object bytes, non-tombstone rows, distinct paths with real content.
pub fn history_info(conn: &Connection) -> Result<HistoryInfo> {
    let bytes: i64 = conn.query_row(
        "SELECT COALESCE(SUM(stored_size), 0) FROM objects",
        [],
        |row| row.get(0),
    )?;
    let versions: i64 = conn.query_row(
        "SELECT COUNT(*) FROM versions WHERE hash IS NOT NULL",
        [],
        |row| row.get(0),
    )?;
    let files: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT path) FROM versions WHERE hash IS NOT NULL",
        [],
        |row| row.get(0),
    )?;
    Ok(HistoryInfo {
        bytes: to_u64(bytes)?,
        versions: to_u64(versions)?,
        files: to_u64(files)?,
    })
}

// ---------------------------------------------------------------------------
// Maintenance and transfer
// ---------------------------------------------------------------------------

/// A consistent snapshot for a copy (transfer). Objects are immutable and
/// copied separately; only the index needs SQLite's backup API (R2/R3).
/// Copies currently close the history instead (spec revision 4), so only
/// tests use this; it stays ready for an online-copy path.
#[cfg(test)]
pub fn backup_to(conn: &Connection, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut target = Connection::open(dest)?;
    {
        let backup = rusqlite::backup::Backup::new(conn, &mut target)?;
        backup.run_to_completion(100, Duration::from_millis(5), None)?;
    }
    target.close().map_err(|(_, e)| Error::Db(e.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub fn integrity_check(conn: &Connection) -> Result<bool> {
    let result: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    Ok(result == "ok")
}

/// Fold the WAL back into the index; non-fatal if a reader blocks it.
pub fn checkpoint(conn: &Connection) -> Result<()> {
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::versions::now_seconds;
    use tempfile::tempdir;

    const FOLDER_ID: &str = "0123456789abcdef0123456789abcdef";

    fn new_index() -> (tempfile::TempDir, Connection) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("index.sqlite");
        let conn = create_index(&path).unwrap();
        write_meta(&conn, FOLDER_ID).unwrap();
        (dir, conn)
    }

    fn object(hash: &str, size: u64) -> ObjectMeta {
        ObjectMeta {
            hash: hash.to_string(),
            kind: ObjectKind::Full,
            base: None,
            depth: 0,
            size,
            stored_size: 10,
        }
    }

    fn add_version(conn: &Connection, path: &[u8], hash: &str, size: u64) -> i64 {
        insert_object(conn, &object(hash, size)).unwrap();
        insert_version(conn, path, Some(hash), size, 1, now_seconds()).unwrap()
    }

    #[test]
    fn creates_the_schema_in_wal_mode_and_reopens() {
        let (dir, conn) = new_index();
        assert_eq!(journal_mode(&conn).unwrap().to_ascii_lowercase(), "wal");
        assert_eq!(schema_version(&conn).unwrap(), SCHEMA_VERSION);
        verify_owner(&conn).unwrap();
        drop(conn);
        let reopened = open_writer(&dir.path().join("index.sqlite")).unwrap();
        verify_owner(&reopened).unwrap();
        assert!(integrity_check(&reopened).unwrap());
    }

    #[test]
    fn refuses_to_create_over_an_existing_index() {
        let (dir, _conn) = new_index();
        let err = create_index(&dir.path().join("index.sqlite")).unwrap_err();
        assert!(matches!(err, Error::NotOurs(_)));
    }

    #[test]
    fn refuses_a_newer_or_missing_schema() {
        let (dir, conn) = new_index();
        conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        drop(conn);
        let err = open_writer(&dir.path().join("index.sqlite")).unwrap_err();
        assert!(matches!(err, Error::IndexDamaged(_)));

        let empty = dir.path().join("empty.sqlite");
        let conn = Connection::open(&empty).unwrap();
        drop(conn);
        let err = open_reader(&empty).unwrap_err();
        assert!(matches!(err, Error::IndexDamaged(_)));
    }

    #[test]
    fn reader_is_read_only() {
        let (dir, conn) = new_index();
        add_version(&conn, b"a.txt", &"a".repeat(64), 5);
        let reader = open_reader(&dir.path().join("index.sqlite")).unwrap();
        assert_eq!(latest_version(&reader, b"a.txt").unwrap().unwrap().size, 5);
        let err = insert_tombstone(&reader, b"a.txt", now_seconds()).unwrap_err();
        assert!(matches!(err, Error::Db(_)));
    }

    #[test]
    fn meta_must_name_this_app() {
        let (dir, conn) = new_index();
        conn.execute("UPDATE meta SET value = 'other' WHERE key = 'app'", [])
            .unwrap();
        let err = verify_owner(&conn).unwrap_err();
        assert!(matches!(err, Error::NotOurs(_)));
        drop(conn);
        // The schema still opens; ownership is a separate gate.
        assert!(open_writer(&dir.path().join("index.sqlite")).is_ok());
    }

    #[test]
    fn writes_and_reads_objects_and_versions() {
        let (_dir, conn) = new_index();
        let hash = "b".repeat(64);
        insert_object(&conn, &object(&hash, 11)).unwrap();
        let id = insert_version(&conn, b"docs/a.txt", Some(&hash), 11, 42, 100).unwrap();
        assert_eq!(id, 1);

        let latest = latest_version(&conn, b"docs/a.txt").unwrap().unwrap();
        assert_eq!(latest.id, 1);
        assert_eq!(latest.display, "docs/a.txt");
        assert_eq!(latest.hash.as_deref(), Some(hash.as_str()));
        assert_eq!(latest.size, 11);
        assert_eq!(latest.recorded_at, 100);

        update_mtime(&conn, id, 99).unwrap();
        assert_eq!(
            latest_version(&conn, b"docs/a.txt").unwrap().unwrap().mtime,
            99
        );
    }

    #[test]
    fn lists_newest_first_and_pages() {
        let (_dir, conn) = new_index();
        let hash = "c".repeat(64);
        insert_object(&conn, &object(&hash, 1)).unwrap();
        for i in 0..10 {
            insert_version(&conn, b"a.txt", Some(&hash), 1, i, 100 + i).unwrap();
        }
        let all = list_versions(&conn, b"a.txt", None, None).unwrap();
        assert_eq!(all.len(), 10);
        assert_eq!(all[0].id, 10, "newest first");
        let page = list_versions(&conn, b"a.txt", Some(3), Some(2)).unwrap();
        assert_eq!(page.iter().map(|v| v.id).collect::<Vec<_>>(), [8, 7, 6]);
        assert_eq!(count_versions(&conn, b"a.txt").unwrap(), 10);
        // The limit is clamped to the UI's maximum rather than unbounded.
        assert_eq!(
            list_versions(&conn, b"a.txt", Some(0), None).unwrap().len(),
            1
        );
    }

    #[test]
    fn latest_under_uses_a_blob_range_and_excludes_siblings() {
        let (_dir, conn) = new_index();
        let hash = "d".repeat(64);
        insert_object(&conn, &object(&hash, 1)).unwrap();
        for path in [
            &b"foo/a"[..],
            b"foo/z",
            b"foo/!x",
            b"foo/0x",
            b"foo/space name",
            b"foobar",
            b"foo-other",
        ] {
            insert_version(&conn, path, Some(&hash), 1, 1, 100).unwrap();
        }
        // A second, newer row for foo/a must collapse to one latest row.
        insert_version(&conn, b"foo/a", Some(&hash), 2, 2, 200).unwrap();

        let under = latest_versions_under(&conn, Some(b"foo")).unwrap();
        let mut displays: Vec<_> = under.into_iter().map(|v| v.display).collect();
        displays.sort();
        assert_eq!(
            displays,
            ["foo/!x", "foo/0x", "foo/a", "foo/space name", "foo/z"]
        );
        assert_eq!(latest_version(&conn, b"foo/a").unwrap().unwrap().size, 2);

        let root = latest_versions_under(&conn, None).unwrap();
        assert_eq!(root.len(), 7, "one latest row per path");
    }

    #[test]
    fn tombstones_and_deleted_files_keep_the_last_size() {
        let (_dir, conn) = new_index();
        let hash = "e".repeat(64);
        add_version(&conn, b"draft.md", &hash, 120);
        insert_tombstone(&conn, b"draft.md", 500).unwrap();
        // Repeating the event must not add a second tombstone.
        insert_tombstone(&conn, b"draft.md", 501).unwrap();

        let latest = latest_version(&conn, b"draft.md").unwrap().unwrap();
        assert!(latest.is_tombstone());
        let deleted = deleted_files(&conn, None).unwrap();
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0].display, "draft.md");
        assert_eq!(deleted[0].size, 120);
        assert_eq!(deleted[0].deleted_at, 501);
    }

    #[test]
    fn history_info_dedupes_shared_objects_and_skips_tombstones() {
        let (_dir, conn) = new_index();
        let hash = "f".repeat(64);
        insert_object(&conn, &object(&hash, 100)).unwrap();
        insert_version(&conn, b"a.txt", Some(&hash), 100, 1, 100).unwrap();
        insert_version(&conn, b"b.txt", Some(&hash), 100, 1, 100).unwrap();
        insert_tombstone(&conn, b"c.txt", 100).unwrap();

        let info = history_info(&conn).unwrap();
        assert_eq!(info.bytes, 10, "one object, counted once");
        assert_eq!(info.versions, 2);
        assert_eq!(info.files, 2);
    }

    #[test]
    fn backup_makes_a_usable_snapshot() {
        let (dir, conn) = new_index();
        let hash = "1".repeat(64);
        add_version(&conn, b"a.txt", &hash, 5);
        let dest = dir.path().join("copy").join("index.sqlite");
        backup_to(&conn, &dest).unwrap();

        let copy = open_writer(&dest).unwrap();
        assert!(integrity_check(&copy).unwrap());
        assert_eq!(latest_version(&copy, b"a.txt").unwrap().unwrap().size, 5);
    }

    #[test]
    fn schema_rejects_inconsistent_objects() {
        let (_dir, conn) = new_index();
        let err = conn
            .execute(
                "INSERT INTO objects (hash, kind, base, depth, size, stored_size)
                 VALUES (?1, 'delta', NULL, 1, 1, 1)",
                params!["2".repeat(64)],
            )
            .unwrap_err();
        assert!(err.to_string().contains("CHECK"), "{err}");
    }

    #[test]
    fn validation_rejects_bad_paths_and_hashes() {
        let (_dir, conn) = new_index();
        assert!(insert_version(&conn, b"../x", None, 0, 0, 1).is_err());
        assert!(insert_version(&conn, b"/abs", None, 0, 0, 1).is_err());
        assert!(insert_version(&conn, b".versions/x", None, 0, 0, 1).is_err());
        assert!(insert_version(&conn, b"a.txt", Some("bad"), 0, 0, 1).is_err());
        assert!(insert_tombstone(&conn, b"a.txt", 1).is_ok());
    }

    #[test]
    fn invalid_folder_id_is_refused() {
        let dir = tempdir().unwrap();
        let conn = create_index(&dir.path().join("index.sqlite")).unwrap();
        assert!(write_meta(&conn, "short").is_err());
        assert!(write_meta(&conn, &"A".repeat(32)).is_err());
        assert!(read_meta(&conn).unwrap().is_empty());
    }
}
