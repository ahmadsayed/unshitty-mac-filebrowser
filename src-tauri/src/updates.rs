//! Check for Updates: how this copy was installed, which failures the user
//! sees, and a guard against two installs at once. Plain functions, testable
//! without Tauri; `lib.rs` wraps `tauri-plugin-updater` around them.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;

/// How the running copy was installed, which decides whether it may replace
/// itself.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum InstallKind {
    AppImage,
    MacApp,
    /// A `.deb` or `.rpm`: the system's package manager owns the files.
    LinuxPackage,
    /// `npm run tauri dev`: never replace anything.
    DevBuild,
}

/// `os` as in `std::env::consts::OS`, `appimage` the `$APPIMAGE` variable.
/// An empty `$APPIMAGE` counts as a package: only the AppImage runtime sets
/// it, always to the image's path, so an empty one says nothing.
pub fn install_kind(debug: bool, os: &str, appimage: Option<&str>) -> InstallKind {
    if debug {
        InstallKind::DevBuild
    } else if os == "macos" {
        InstallKind::MacApp
    } else if appimage.is_some_and(|path| !path.is_empty()) {
        InstallKind::AppImage
    } else {
        InstallKind::LinuxPackage
    }
}

pub fn can_self_update(kind: InstallKind) -> bool {
    matches!(kind, InstallKind::AppImage | InstallKind::MacApp)
}

/// What went wrong, as the dialogs tell it apart.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    Offline,
    Unverified,
    NoPermission,
    Failed,
}

/// The error every update command returns; the frontend picks the wording
/// from `kind` and shows `detail` under it.
#[derive(Serialize, Debug)]
pub struct Failure {
    pub kind: FailureKind,
    pub detail: String,
}

/// The parts of an updater error that matter here, extracted by `lib.rs`
/// so this file needs no Tauri types.
pub enum FailureSource {
    Network { connect: bool },
    Signature,
    Io(std::io::ErrorKind),
    Other,
}

pub fn classify(source: FailureSource, detail: String) -> Failure {
    let kind = match source {
        FailureSource::Network { connect: true } => FailureKind::Offline,
        FailureSource::Signature => FailureKind::Unverified,
        FailureSource::Io(std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem) => {
            FailureKind::NoPermission
        }
        _ => FailureKind::Failed,
    };
    Failure { kind, detail }
}

/// One install at a time: a second Download and Install while one runs is
/// refused, and every way out of an install calls `finish`.
#[derive(Default)]
pub struct InstallGuard(AtomicBool);

impl InstallGuard {
    pub fn try_start(&self) -> bool {
        self.0.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }

    pub fn finish(&self) {
        self.0.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_kind_for_each_environment() {
        assert_eq!(install_kind(true, "linux", Some("/x.AppImage")), InstallKind::DevBuild);
        assert_eq!(install_kind(false, "macos", None), InstallKind::MacApp);
        assert_eq!(
            install_kind(false, "linux", Some("/home/me/.local/bin/file-browser.AppImage")),
            InstallKind::AppImage
        );
        assert_eq!(install_kind(false, "linux", Some("")), InstallKind::LinuxPackage);
        assert_eq!(install_kind(false, "linux", None), InstallKind::LinuxPackage);
    }

    #[test]
    fn only_appimage_and_mac_update_themselves() {
        assert!(can_self_update(InstallKind::AppImage));
        assert!(can_self_update(InstallKind::MacApp));
        assert!(!can_self_update(InstallKind::LinuxPackage));
        assert!(!can_self_update(InstallKind::DevBuild));
    }

    #[test]
    fn failures_are_classified() {
        let kind = |source| classify(source, "why".into()).kind;
        assert_eq!(kind(FailureSource::Network { connect: true }), FailureKind::Offline);
        assert_eq!(kind(FailureSource::Signature), FailureKind::Unverified);
        assert_eq!(kind(FailureSource::Io(std::io::ErrorKind::PermissionDenied)), FailureKind::NoPermission);
        assert_eq!(kind(FailureSource::Io(std::io::ErrorKind::ReadOnlyFilesystem)), FailureKind::NoPermission);
        assert_eq!(kind(FailureSource::Io(std::io::ErrorKind::NotFound)), FailureKind::Failed);
        assert_eq!(kind(FailureSource::Network { connect: false }), FailureKind::Failed);
        assert_eq!(kind(FailureSource::Other), FailureKind::Failed);
        assert_eq!(classify(FailureSource::Other, "exact text".into()).detail, "exact text");
    }

    #[test]
    fn the_guard_allows_one_install_at_a_time() {
        let guard = InstallGuard::default();
        assert!(guard.try_start());
        assert!(!guard.try_start());
        guard.finish();
        assert!(guard.try_start());
    }

    #[test]
    fn failure_serializes_for_the_frontend() {
        let failure = Failure { kind: FailureKind::Offline, detail: "x".into() };
        assert_eq!(serde_json::to_value(&failure).unwrap(), serde_json::json!({ "kind": "offline", "detail": "x" }));
        assert_eq!(serde_json::to_value(InstallKind::AppImage).unwrap(), serde_json::json!("app_image"));
        assert_eq!(serde_json::to_value(FailureKind::NoPermission).unwrap(), serde_json::json!("no_permission"));
    }
}
