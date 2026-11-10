//! What the app learns about items besides a listing: their kind and
//! filesystem for drag and drop (`PathInfo`), their total size before a copy
//! (`Measure`), and everything the Properties dialog shows
//! (`FileProperties`). This computer and Google Drive both fill them.

use serde::Serialize;

#[derive(Debug, Serialize, PartialEq)]
pub struct PathInfo {
    pub is_dir: bool,
    /// Filesystem the path lives on; equal ids mean a move is a cheap rename.
    pub device: u64,
}

/// Sizes and item counts of `paths` before a copy, for the progress total.
#[derive(Serialize, Default, PartialEq, Eq, Debug, Clone, Copy)]
pub struct Measure {
    pub bytes: u64,
    pub items: u64,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct FileProperties {
    pub name: String,
    pub path: String,
    /// The containing folder, or empty for a filesystem root.
    pub parent: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    /// Where a symlink points, if it is one.
    pub symlink_target: Option<String>,
    pub size: u64,
    /// Direct children for folders; `None` for files.
    pub item_count: Option<u32>,
    pub modified: i64,
    pub accessed: i64,
    /// Birth time; 0 when the filesystem doesn't report one.
    pub created: i64,
    /// Permissions in `ls -l` form, e.g. "drwxr-xr-x".
    pub mode: String,
    /// The same permission bits in octal, e.g. "755".
    pub octal: String,
    pub owner: String,
    pub group: String,
    pub others: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_serialises_with_the_fields_the_frontend_reads() {
        let info = PathInfo { is_dir: true, device: 7 };
        assert_eq!(serde_json::to_string(&info).unwrap(), r#"{"is_dir":true,"device":7}"#);
        let measure = Measure { bytes: 10, items: 2 };
        assert_eq!(serde_json::to_string(&measure).unwrap(), r#"{"bytes":10,"items":2}"#);
        let props = FileProperties {
            name: "a".into(),
            path: "/a".into(),
            parent: "/".into(),
            is_dir: false,
            is_symlink: false,
            symlink_target: None,
            size: 1,
            item_count: None,
            modified: 2,
            accessed: 3,
            created: 0,
            mode: "-rw-r--r--".into(),
            octal: "644".into(),
            owner: "Read and write".into(),
            group: "Read-only".into(),
            others: "Read-only".into(),
        };
        assert_eq!(
            serde_json::to_string(&props).unwrap(),
            concat!(
                r#"{"name":"a","path":"/a","parent":"/","is_dir":false,"is_symlink":false,"symlink_target":null,"#,
                r#""size":1,"item_count":null,"modified":2,"accessed":3,"created":0,"mode":"-rw-r--r--","#,
                r#""octal":"644","owner":"Read and write","group":"Read-only","others":"Read-only"}"#
            )
        );
    }
}
