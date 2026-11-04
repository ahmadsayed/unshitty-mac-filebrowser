//! The release workflow signs updater packages with `scripts/sign-updater.sh`
//! (plain minisign, outside the build jobs). Installed copies check those
//! signatures with tauri-plugin-updater, which uses `minisign-verify` and,
//! with `requireSignedVersion`, the version in the trusted comment. A format
//! mistake here would leave every installed copy unable to update, so this
//! signs with a throwaway key through the real script and verifies the result
//! exactly as the updater does. Skipped when `minisign` isn't installed.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use minisign_verify::{PublicKey, Signature};
use std::path::{Path, PathBuf};
use std::process::Command;

const PASSWORD: &str = "throwaway password";

fn have_minisign() -> bool {
    Command::new("minisign").arg("-v").output().is_ok()
}

fn script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/sign-updater.sh")
}

/// A Tauri-style key pair: each file of a minisign pair, base64-encoded.
fn throwaway_keys(dir: &Path) -> (String, String) {
    let (public, secret) = (dir.join("key.pub"), dir.join("key.sec"));
    let mut generate = Command::new("minisign")
        .args(["-G", "-f", "-p"])
        .arg(&public)
        .arg("-s")
        .arg(&secret)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    write!(generate.stdin.take().unwrap(), "{PASSWORD}\n{PASSWORD}\n").unwrap();
    assert!(generate.wait().unwrap().success());
    let encode = |path: &Path| STANDARD.encode(std::fs::read(path).unwrap());
    (encode(&secret), encode(&public))
}

/// Run the release script on `files`.
fn sign(secret: &str, password: &str, public: &str, version: &str, files: &[&Path]) -> std::process::Output {
    Command::new("bash")
        .arg(script())
        .arg(version)
        .arg(public)
        .args(files)
        .env("TAURI_SIGNING_PRIVATE_KEY", secret)
        .env("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", password)
        .output()
        .unwrap()
}

/// What tauri-plugin-updater 2's verify_signature does with a `.sig` file.
fn updater_accepts(public: &str, sig_file: &Path, data: &[u8], version: &str) -> Result<(), String> {
    let decode = |b64: &str| String::from_utf8(STANDARD.decode(b64.trim()).map_err(|e| e.to_string())?).map_err(|e| e.to_string());
    let key = PublicKey::decode(&decode(public)?).map_err(|e| e.to_string())?;
    let signature = Signature::decode(&decode(&std::fs::read_to_string(sig_file).unwrap())?).map_err(|e| e.to_string())?;
    key.verify(data, &signature, true).map_err(|e| e.to_string())?;
    let signed = signature
        .trusted_comment()
        .split('\t')
        .find_map(|pair| pair.strip_prefix("version:"))
        .ok_or("no version in the trusted comment")?;
    if signed == version { Ok(()) } else { Err(format!("signed for {signed}, announced {version}")) }
}

#[test]
fn signed_packages_pass_the_updaters_check() {
    if !have_minisign() {
        eprintln!("minisign isn't installed: skipping the release signing test");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let (secret, public) = throwaway_keys(tmp.path());
    let appimage = tmp.path().join("File Browser_0.7.4_amd64.AppImage");
    let archive = tmp.path().join("File Browser.app.tar.gz");
    std::fs::write(&appimage, b"appimage bytes").unwrap();
    std::fs::write(&archive, b"archive bytes").unwrap();

    let out = sign(&secret, PASSWORD, &public, "0.7.4", &[&appimage, &archive]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    for (file, bytes) in [(&appimage, &b"appimage bytes"[..]), (&archive, &b"archive bytes"[..])] {
        let sig = PathBuf::from(format!("{}.sig", file.display()));
        assert_eq!(updater_accepts(&public, &sig, bytes, "0.7.4"), Ok(()));
        // The updater refuses another version, and changed bytes.
        assert!(updater_accepts(&public, &sig, bytes, "0.7.5").is_err());
        assert!(updater_accepts(&public, &sig, b"tampered", "0.7.4").is_err());
    }
    // The key never stays on disk next to the packages.
    let left: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(left.len(), 6, "only the key pair, 2 packages and 2 signatures: {left:?}");
}

#[test]
fn a_key_that_does_not_match_the_apps_public_key_is_refused() {
    if !have_minisign() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let (secret, _) = throwaway_keys(tmp.path());
    let other = tempfile::tempdir().unwrap();
    let (_, other_public) = throwaway_keys(other.path());
    let file = tmp.path().join("pkg.AppImage");
    std::fs::write(&file, b"bytes").unwrap();
    let out = sign(&secret, PASSWORD, &other_public, "0.7.4", &[&file]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("does not verify"), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(!tmp.path().join("pkg.AppImage.sig").exists(), "no signature is left behind");
}

#[test]
fn a_wrong_password_or_missing_key_fails_without_signatures() {
    if !have_minisign() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let (secret, public) = throwaway_keys(tmp.path());
    let file = tmp.path().join("pkg.AppImage");
    std::fs::write(&file, b"bytes").unwrap();
    assert!(!sign(&secret, "wrong", &public, "0.7.4", &[&file]).status.success());
    assert!(!sign("", PASSWORD, &public, "0.7.4", &[&file]).status.success());
    assert!(!sign(&secret, PASSWORD, &public, "", &[&file]).status.success());
    assert!(!tmp.path().join("pkg.AppImage.sig").exists());
}
