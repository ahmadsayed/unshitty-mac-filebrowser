//! What a listing returns, and what opening an item returns, the same for
//! every place.

use serde::Serialize;

/// One item in a listing, as the frontend receives it.
#[derive(Debug, Serialize)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: i64,
    pub is_hidden: bool,
    pub is_symlink: bool,
    /// Present only for an entry from Google Drive. Other entries omit the
    /// key entirely, so existing consumers see exactly the same JSON; the
    /// frontend treats any `cloud` key as a Drive item, so no other place
    /// may set it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cloud: Option<CloudEntry>,
    /// The item's other files, when it has some: today an Arbesk asset's
    /// `representations` list. Named for what it is, not for Arbesk, so
    /// another place with "other files of one item" can fill it. Omitted
    /// otherwise, so every other entry's JSON is unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub representations: Option<Representations>,
}

/// A summary of an item's other files, for the view: how many (valid
/// ones), how many are out of date, and their distinct formats (lower-case
/// extensions) in stored order.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Representations {
    pub count: u32,
    pub stale: u32,
    pub formats: Vec<String>,
}

/// Fields that only exist on an entry from a cloud provider. Omitted from the
/// serialized entry for local files, so existing frontend code is unchanged.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CloudEntry {
    pub account: String,
    pub id: String,
    pub mime_type: String,
    pub is_google_doc: bool,
    pub is_shortcut: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_url: Option<String>,
    pub can_download: bool,
}

/// What opening an item gives the app: a local file to hand to the default
/// app (the file itself, or a copy in the app cache), or a URL to open in
/// the browser instead (a Google Doc, an Arbesk asset).
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OpenTarget {
    File { path: String },
    Url { url: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, cloud: Option<CloudEntry>) -> Entry {
        Entry {
            name: "a.txt".into(),
            path: path.into(),
            is_dir: false,
            size: 3,
            modified: 1_700_000_000,
            is_hidden: false,
            is_symlink: false,
            cloud,
            representations: None,
        }
    }

    #[test]
    fn an_entry_without_cloud_data_has_no_cloud_key() {
        assert_eq!(
            serde_json::to_string(&entry("/home/me/a.txt", None)).unwrap(),
            r#"{"name":"a.txt","path":"/home/me/a.txt","is_dir":false,"size":3,"modified":1700000000,"is_hidden":false,"is_symlink":false}"#
        );
    }

    #[test]
    fn a_drive_entry_carries_its_cloud_fields_and_omits_a_missing_web_url() {
        let cloud = CloudEntry {
            account: "acct".into(),
            id: "f1".into(),
            mime_type: "text/plain".into(),
            is_google_doc: false,
            is_shortcut: false,
            web_url: None,
            can_download: true,
        };
        assert_eq!(
            serde_json::to_string(&entry("gdrive://acct/f1~a.txt", Some(cloud.clone()))).unwrap(),
            concat!(
                r#"{"name":"a.txt","path":"gdrive://acct/f1~a.txt","is_dir":false,"size":3,"modified":1700000000,"is_hidden":false,"is_symlink":false,"#,
                r#""cloud":{"account":"acct","id":"f1","mime_type":"text/plain","is_google_doc":false,"is_shortcut":false,"can_download":true}}"#
            )
        );
        let linked = CloudEntry { web_url: Some("https://drive/x".into()), ..cloud };
        assert!(serde_json::to_string(&linked).unwrap().contains(r#""web_url":"https://drive/x""#));
    }

    #[test]
    fn an_entry_with_other_files_carries_their_summary() {
        let mut with = entry("arbesk://0xab/1~Set/a1~Box.gltf", None);
        with.representations = Some(Representations {
            count: 3,
            stale: 1,
            formats: vec!["blend".into(), "gcode".into()],
        });
        assert_eq!(
            serde_json::to_string(&with).unwrap(),
            concat!(
                r#"{"name":"a.txt","path":"arbesk://0xab/1~Set/a1~Box.gltf","is_dir":false,"size":3,"modified":1700000000,"is_hidden":false,"is_symlink":false,"#,
                r#""representations":{"count":3,"stale":1,"formats":["blend","gcode"]}}"#
            )
        );
    }

    #[test]
    fn an_open_target_is_tagged_by_its_kind() {
        let file = OpenTarget::File { path: "/tmp/a.txt".into() };
        let url = OpenTarget::Url { url: "https://example.com/d".into() };
        assert_eq!(serde_json::to_string(&file).unwrap(), r#"{"kind":"file","path":"/tmp/a.txt"}"#);
        assert_eq!(serde_json::to_string(&url).unwrap(), r#"{"kind":"url","url":"https://example.com/d"}"#);
    }
}
