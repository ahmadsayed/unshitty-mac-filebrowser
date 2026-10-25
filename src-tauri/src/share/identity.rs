//! The sharing identity: a long-lived iroh secret key stored next to the
//! other settings, guarded by a lock so only one window shares at a time.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub struct Identity {
    pub key: iroh::SecretKey,
    _lock: File,
}

const BUSY: &str = "Sharing is active in another Files window.";

pub fn load_or_create(dir: &Path) -> Result<Identity, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join("share-key");
    let mut opts = OpenOptions::new();
    opts.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let mut file = opts.open(&path).map_err(|e| e.to_string())?;
    file.try_lock().map_err(|_| BUSY.to_string())?;

    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let key = if bytes.is_empty() {
        let mut raw = [0u8; 32];
        getrandom::fill(&mut raw).map_err(|e| e.to_string())?;
        file.write_all(&raw).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        iroh::SecretKey::from_bytes(&raw)
    } else {
        let raw: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| "The sharing key file is damaged.".to_string())?;
        iroh::SecretKey::from_bytes(&raw)
    };
    Ok(Identity { key, _lock: file })
}

/// Debug builds may redirect the config dir (for running two instances);
/// release builds never honour it.
pub fn config_dir_override() -> Option<PathBuf> {
    if cfg!(debug_assertions) {
        std::env::var_os("FILEBROWSER_CONFIG_DIR").map(PathBuf::from)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn the_key_is_created_once_and_reused() {
        let d = tempfile::tempdir().unwrap();
        let a = load_or_create(d.path()).unwrap().key.public();
        let b = load_or_create(d.path()).unwrap().key.public();
        assert_eq!(a, b);
    }

    #[test]
    fn the_key_file_is_private() {
        let d = tempfile::tempdir().unwrap();
        let _i = load_or_create(d.path()).unwrap();
        let mode = std::fs::metadata(d.path().join("share-key"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_second_instance_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let _first = load_or_create(d.path()).unwrap();
        let err = load_or_create(d.path()).err().unwrap();
        assert_eq!(err, "Sharing is active in another Files window.");
    }
}
