//! Icons from the operating system for files and folders: the ones Finder
//! shows on macOS (NSWorkspace), the user's icon theme on Linux (GIO names
//! the icon, the freedesktop icon theme lookup finds its file). Nothing is
//! bundled: the OS draws them for the running app.
//!
//! Most files share their icon with every file of the same type, so the
//! frontend first asks for each path's icon *id* (cheap, no image work) and
//! then for each id's image once. Only ids handed out here can be drawn, so
//! `image` never reads a path the frontend made up.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Where an icon's image comes from.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// A type macOS draws an icon for: a filename extension or a UTI.
    OsType(String),
    /// A path macOS draws its own icon for: apps, special folders, volumes,
    /// items with a custom icon.
    OsPath(PathBuf),
    /// Icon names to look up in the icon theme, best first (Linux).
    ThemeNames(Vec<String>),
    /// An image file set as this item's icon (Linux: GIO's custom icon).
    IconFile(PathBuf),
}

impl Source {
    /// The id the frontend caches the image under: equal sources share it.
    pub fn id(&self) -> String {
        match self {
            Source::OsType(kind) => format!("type:{kind}"),
            Source::OsPath(path) => format!("path:{}", path.display()),
            Source::ThemeNames(names) => format!("names:{}", names.join(",")),
            Source::IconFile(path) => format!("file:{}", path.display()),
        }
    }
}

/// The ids handed out so far, and the icon themes read so far.
pub struct Icons {
    home: Option<PathBuf>,
    theme: String,
    sources: Mutex<HashMap<String, Source>>,
    themes: Mutex<HashMap<String, Option<Arc<theme::Theme>>>>,
}

impl Icons {
    /// `theme` is the icon theme the desktop uses (Linux only).
    pub fn new(home: Option<PathBuf>, theme: Option<String>) -> Self {
        Icons {
            home,
            theme: theme.unwrap_or_else(|| "Adwaita".into()),
            sources: Mutex::new(HashMap::new()),
            themes: Mutex::new(HashMap::new()),
        }
    }

    /// The icon id for `path`, or None when the OS has none for it and the
    /// app draws its own.
    pub fn id(&self, path: &Path) -> Option<String> {
        let source = source_for(path, self.home.as_deref())?;
        let id = source.id();
        self.sources
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(id.clone())
            .or_insert(source);
        Some(id)
    }

    /// The image for an id from `id`, about `size` pixels square: PNG, or SVG
    /// from an icon theme. None for an unknown id or when drawing fails.
    pub fn image(&self, id: &str, size: u32) -> Option<Vec<u8>> {
        let source = self
            .sources
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()?;
        match &source {
            Source::ThemeNames(names) => {
                let bases = theme::search_paths();
                let file = theme::find(&|name| self.load_theme(name, &bases), &self.theme, names, size, &bases)?;
                std::fs::read(file).ok()
            }
            Source::IconFile(file) => theme::is_icon_file(file).then(|| std::fs::read(file).ok()).flatten(),
            _ => render(&source, size),
        }
    }

    fn load_theme(&self, name: &str, bases: &[PathBuf]) -> Option<Arc<theme::Theme>> {
        let mut themes = self.themes.lock().unwrap_or_else(|e| e.into_inner());
        themes
            .entry(name.to_string())
            .or_insert_with(|| theme::Theme::load(name, bases).map(Arc::new))
            .clone()
    }
}

// ---------------------------------------------------------------------------
// macOS: which icon Finder shows
// ---------------------------------------------------------------------------

/// The icon source for a path on macOS. Folders and files of a type share
/// one icon; items that can have their own get one per path: packages such
/// as apps, the folders Finder marks specially (home and what's in it, the
/// disk root, volumes) and anything with a custom icon.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn mac_source(path: &Path, is_dir: bool, executable: bool, custom_icon: bool, home: Option<&Path>) -> Source {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| !e.is_empty())
        .map(str::to_lowercase);
    if custom_icon || (is_dir && (extension.is_some() || is_special_folder(path, home))) {
        return Source::OsPath(path.to_path_buf());
    }
    if is_dir {
        return Source::OsType("public.folder".into());
    }
    Source::OsType(match extension {
        Some(extension) => extension,
        None if executable => "public.unix-executable".into(),
        None => "public.data".into(),
    })
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn is_special_folder(path: &Path, home: Option<&Path>) -> bool {
    let parent = path.parent();
    parent.is_none()
        || parent == Some(Path::new("/"))
        || parent == Some(Path::new("/Volumes"))
        || home.is_some_and(|home| path == home || parent == Some(home))
}

/// Whether Finder's flags (the `com.apple.FinderInfo` attribute) say the item
/// has a custom icon: `kHasCustomIcon` in the big-endian flags at byte 8.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn finder_info_has_custom_icon(info: &[u8]) -> bool {
    info.get(8).is_some_and(|flags| flags & 0x04 != 0)
}

#[cfg(target_os = "macos")]
fn source_for(path: &Path, home: Option<&Path>) -> Option<Source> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(path).ok()?;
    let custom_icon = xattr::get(path, "com.apple.FinderInfo")
        .ok()
        .flatten()
        .is_some_and(|info| finder_info_has_custom_icon(&info));
    let executable = meta.permissions().mode() & 0o111 != 0;
    Some(mac_source(path, meta.is_dir(), executable, custom_icon, home))
}

/// PNG of the icon macOS shows for `source`, `size` pixels square.
#[cfg(target_os = "macos")]
fn render(source: &Source, size: u32) -> Option<Vec<u8>> {
    use objc2::AnyThread;
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::{
        NSBitmapImageFileType, NSBitmapImageRep, NSCompositingOperation, NSDeviceRGBColorSpace,
        NSGraphicsContext, NSWorkspace,
    };
    use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};

    autoreleasepool(|_| {
        let workspace = NSWorkspace::sharedWorkspace();
        let image = match source {
            // Takes an extension or a UTI; its replacement needs a UTType.
            #[allow(deprecated)]
            Source::OsType(kind) => workspace.iconForFileType(&NSString::from_str(kind)),
            Source::OsPath(path) => workspace.iconForFile(&NSString::from_str(path.to_str()?)),
            _ => return None,
        };
        let px = size as isize;
        // SAFETY: a null `planes` makes the rep allocate its own pixels.
        let rep = unsafe {
            NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
                NSBitmapImageRep::alloc(),
                std::ptr::null_mut(),
                px,
                px,
                8,
                4,
                true,
                false,
                NSDeviceRGBColorSpace,
                0,
                0,
            )
        }?;
        // Drawing into the rep's own context: NSImage picks the
        // representation that best fits the pixel size.
        let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep)?;
        NSGraphicsContext::saveGraphicsState_class();
        NSGraphicsContext::setCurrentContext(Some(&context));
        let side = f64::from(size);
        let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(side, side));
        image.drawInRect_fromRect_operation_fraction(rect, NSRect::ZERO, NSCompositingOperation::Copy, 1.0);
        context.flushGraphics();
        NSGraphicsContext::restoreGraphicsState_class();
        // SAFETY: an empty properties dictionary is valid for PNG.
        let png = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }?;
        Some(png.to_vec())
    })
}

// ---------------------------------------------------------------------------
// Linux: GIO names the icon (by content type, special folder or a custom
// icon), the icon theme finds its file
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
fn source_for(path: &Path, _home: Option<&Path>) -> Option<Source> {
    use gio::prelude::*;
    let info = gio::File::for_path(path)
        .query_info("standard::icon", gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE)
        .ok()?;
    let icon = info.icon()?;
    if let Some(themed) = icon.downcast_ref::<gio::ThemedIcon>() {
        return theme_source(themed.names().iter().map(|n| n.to_string()));
    }
    let file = icon.downcast_ref::<gio::FileIcon>()?.file().path()?;
    theme::is_icon_file(&file).then_some(Source::IconFile(file))
}

/// Full-colour names only: GIO lists symbolic ones too.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn theme_source(names: impl Iterator<Item = String>) -> Option<Source> {
    let names: Vec<String> = names.filter(|n| !n.ends_with("-symbolic")).collect();
    (!names.is_empty()).then_some(Source::ThemeNames(names))
}

#[cfg(not(target_os = "macos"))]
fn render(_source: &Source, _size: u32) -> Option<Vec<u8>> {
    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn source_for(_path: &Path, _home: Option<&Path>) -> Option<Source> {
    None
}

/// The freedesktop icon theme lookup: a theme's `index.theme` lists folders
/// of icons by size; a name is looked up in the theme, then in the themes it
/// inherits from, then in hicolor, taking the folder whose size is closest.
pub mod theme {
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    const EXTENSIONS: [&str; 2] = ["svg", "png"];

    /// Folders that hold icon themes, most personal first.
    pub fn search_paths() -> Vec<PathBuf> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| home.as_ref().map(|h| h.join(".local/share")));
        let data_dirs = std::env::var("XDG_DATA_DIRS")
            .ok()
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        let mut paths: Vec<PathBuf> = home.map(|h| h.join(".icons")).into_iter().collect();
        paths.extend(data_home.map(|d| d.join("icons")));
        paths.extend(data_dirs.split(':').filter(|d| d.starts_with('/')).map(|d| Path::new(d).join("icons")));
        paths
    }

    /// Only images a web view can show safely and that themes use.
    pub fn is_icon_file(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Kind {
        Fixed,
        Scalable,
        Threshold,
    }

    #[derive(Debug)]
    struct Dir {
        path: String,
        size: u32,
        scale: u32,
        kind: Kind,
        min: u32,
        max: u32,
        threshold: u32,
    }

    impl Dir {
        /// How far this folder's icons are from `px` pixels (0: a match).
        fn distance(&self, px: u32) -> u32 {
            let (low, high) = match self.kind {
                Kind::Fixed => (self.size, self.size),
                Kind::Scalable => (self.min, self.max),
                Kind::Threshold => (self.size.saturating_sub(self.threshold), self.size + self.threshold),
            };
            let (low, high) = (low * self.scale, high * self.scale);
            if px < low {
                low - px
            } else {
                px.saturating_sub(high)
            }
        }
    }

    #[derive(Debug)]
    pub struct Theme {
        roots: Vec<PathBuf>,
        dirs: Vec<Dir>,
        inherits: Vec<String>,
    }

    impl Theme {
        /// The theme `name` from the first `index.theme` found; its icons may
        /// be spread over every search path.
        pub fn load(name: &str, bases: &[PathBuf]) -> Option<Theme> {
            if name.is_empty() || name.contains('/') {
                return None;
            }
            let roots: Vec<PathBuf> = bases.iter().map(|b| b.join(name)).filter(|r| r.is_dir()).collect();
            let index = roots.iter().find_map(|r| std::fs::read_to_string(r.join("index.theme")).ok())?;
            let (dirs, inherits) = parse(&index);
            Some(Theme { roots, dirs, inherits })
        }

        /// The file for `name` in this theme closest to `px` pixels.
        fn lookup(&self, name: &str, px: u32) -> Option<PathBuf> {
            let mut best: Option<(u32, PathBuf)> = None;
            for dir in &self.dirs {
                let distance = dir.distance(px);
                if best.as_ref().is_some_and(|(d, _)| *d <= distance) {
                    continue;
                }
                let found = self.roots.iter().find_map(|root| {
                    EXTENSIONS
                        .iter()
                        .map(|ext| root.join(&dir.path).join(format!("{name}.{ext}")))
                        .find(|file| file.is_file())
                });
                if let Some(file) = found {
                    if distance == 0 {
                        return Some(file);
                    }
                    best = Some((distance, file));
                }
            }
            best.map(|(_, file)| file)
        }
    }

    /// The folders and inherited themes an `index.theme` lists.
    fn parse(index: &str) -> (Vec<Dir>, Vec<String>) {
        let mut section = String::new();
        let mut listed: Vec<String> = Vec::new();
        let mut inherits = Vec::new();
        let mut keys: std::collections::HashMap<String, Vec<(String, String)>> = Default::default();
        for line in index.lines().map(str::trim) {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = name.to_string();
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            let (key, value) = (key.trim(), value.trim());
            if section == "Icon Theme" {
                match key {
                    "Directories" | "ScaledDirectories" => {
                        listed.extend(value.split(',').map(str::trim).filter(|d| !d.is_empty()).map(String::from))
                    }
                    "Inherits" => inherits = value.split(',').map(str::trim).filter(|t| !t.is_empty()).map(String::from).collect(),
                    _ => {}
                }
            } else {
                keys.entry(section.clone()).or_default().push((key.to_string(), value.to_string()));
            }
        }
        let mut seen = HashSet::new();
        let dirs = listed
            .into_iter()
            .filter(|d| seen.insert(d.clone()))
            .filter_map(|path| {
                let entries = keys.get(&path)?;
                let get = |k: &str| entries.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
                let num = |k: &str| get(k).and_then(|v| v.parse::<u32>().ok());
                let size = num("Size")?;
                let kind = match get("Type") {
                    Some("Fixed") => Kind::Fixed,
                    Some("Scalable") => Kind::Scalable,
                    _ => Kind::Threshold,
                };
                Some(Dir {
                    size,
                    scale: num("Scale").unwrap_or(1).max(1),
                    kind,
                    min: num("MinSize").unwrap_or(size),
                    max: num("MaxSize").unwrap_or(size),
                    threshold: num("Threshold").unwrap_or(2),
                    path,
                })
            })
            .collect();
        (dirs, inherits)
    }

    /// The best file for the first of `names` found: in `theme`, then the
    /// themes it inherits from, then hicolor, then loose in a search path
    /// (`/usr/share/pixmaps` style).
    pub fn find(
        load: &dyn Fn(&str) -> Option<Arc<Theme>>,
        theme: &str,
        names: &[String],
        px: u32,
        bases: &[PathBuf],
    ) -> Option<PathBuf> {
        let mut chain = Vec::new();
        let mut seen = HashSet::new();
        let mut queue = vec![theme.to_string()];
        while let Some(name) = queue.pop() {
            if !seen.insert(name.clone()) {
                continue;
            }
            if let Some(loaded) = load(&name) {
                // Inherited themes are searched in the order listed.
                queue.extend(loaded.inherits.iter().rev().cloned());
                chain.push(loaded);
            }
            if queue.is_empty() && !seen.contains("hicolor") {
                queue.push("hicolor".into());
            }
        }
        for theme in &chain {
            if let Some(file) = names.iter().find_map(|name| theme.lookup(name, px)) {
                return Some(file);
            }
        }
        let loose = bases.iter().map(|b| b.parent().unwrap_or(b).join("pixmaps"));
        loose.chain(bases.iter().cloned()).find_map(|dir| {
            names.iter().find_map(|name| {
                EXTENSIONS.iter().map(|ext| dir.join(format!("{name}.{ext}"))).find(|f| f.is_file())
            })
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn write(path: &Path, text: &str) {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }

        const INDEX: &str = "[Icon Theme]\nName=Test\nInherits=Base\nDirectories=16x16/mimetypes,48x48/mimetypes,scalable/mimetypes\n\n\
            [16x16/mimetypes]\nSize=16\nType=Fixed\n\n[48x48/mimetypes]\nSize=48\nType=Fixed\n\n\
            [scalable/mimetypes]\nSize=128\nType=Scalable\nMinSize=64\nMaxSize=512\n";

        fn fixture() -> (tempfile::TempDir, Vec<PathBuf>) {
            let dir = tempfile::tempdir().unwrap();
            let icons = dir.path().join("icons");
            write(&icons.join("Test/index.theme"), INDEX);
            write(&icons.join("Test/16x16/mimetypes/text-x-generic.png"), "16");
            write(&icons.join("Test/48x48/mimetypes/text-x-generic.png"), "48");
            write(&icons.join("Test/scalable/mimetypes/text-x-generic.svg"), "<svg/>");
            write(&icons.join("Base/index.theme"), "[Icon Theme]\nDirectories=48x48/places\n[48x48/places]\nSize=48\n");
            write(&icons.join("Base/48x48/places/folder.png"), "base folder");
            write(&icons.join("hicolor/index.theme"), "[Icon Theme]\nDirectories=48x48/apps\n[48x48/apps]\nSize=48\n");
            write(&icons.join("hicolor/48x48/apps/my-app.png"), "app");
            write(&dir.path().join("pixmaps/old-app.png"), "pixmap");
            (dir, vec![icons])
        }

        fn find_in(bases: &[PathBuf], names: &[&str], px: u32) -> Option<String> {
            let load = |name: &str| Theme::load(name, bases).map(Arc::new);
            let names: Vec<String> = names.iter().map(|n| n.to_string()).collect();
            find(&load, "Test", &names, px, bases).map(|f| std::fs::read_to_string(f).unwrap())
        }

        #[test]
        fn the_closest_size_wins_and_scalable_covers_its_range() {
            let (_dir, bases) = fixture();
            assert_eq!(find_in(&bases, &["text-x-generic"], 16).as_deref(), Some("16"));
            assert_eq!(find_in(&bases, &["text-x-generic"], 48).as_deref(), Some("48"));
            assert_eq!(find_in(&bases, &["text-x-generic"], 40).as_deref(), Some("48"));
            assert_eq!(find_in(&bases, &["text-x-generic"], 256).as_deref(), Some("<svg/>"));
        }

        #[test]
        fn names_are_tried_in_order_then_inherited_themes_then_hicolor_and_pixmaps() {
            let (_dir, bases) = fixture();
            // The specific name is missing: the generic one after it is used.
            assert_eq!(find_in(&bases, &["application-x-thing", "text-x-generic"], 48).as_deref(), Some("48"));
            assert_eq!(find_in(&bases, &["folder"], 48).as_deref(), Some("base folder"));
            assert_eq!(find_in(&bases, &["my-app"], 48).as_deref(), Some("app"));
            assert_eq!(find_in(&bases, &["old-app"], 48).as_deref(), Some("pixmap"));
            assert_eq!(find_in(&bases, &["nothing-like-it"], 48), None);
        }

        #[test]
        fn a_missing_or_odd_theme_name_falls_back_to_hicolor() {
            let (_dir, bases) = fixture();
            let load = |name: &str| Theme::load(name, &bases).map(Arc::new);
            let names = vec!["my-app".to_string()];
            assert!(find(&load, "Gone", &names, 48, &bases).is_some());
            assert!(Theme::load("../Test", &bases).is_none());
            assert!(Theme::load("", &bases).is_none());
        }

        #[test]
        fn inheritance_loops_end() {
            let dir = tempfile::tempdir().unwrap();
            let icons = dir.path().join("icons");
            write(&icons.join("A/index.theme"), "[Icon Theme]\nInherits=B\nDirectories=\n");
            write(&icons.join("B/index.theme"), "[Icon Theme]\nInherits=A\nDirectories=\n");
            let bases = vec![icons];
            let load = |name: &str| Theme::load(name, &bases).map(Arc::new);
            assert_eq!(find(&load, "A", &["x".to_string()], 48, &bases), None);
        }

        #[test]
        fn index_files_parse_sizes_scales_and_kinds() {
            let (dirs, inherits) = parse(
                "# comment\n[Icon Theme]\nInherits=One, Two\nDirectories=a,b,a\nScaledDirectories=c\n\
                 [a]\nSize=32\nScale=2\n[b]\nSize=24\nType=Threshold\nThreshold=4\n[c]\nSize=16\nType=Scalable\nMinSize=8\nMaxSize=64\n[d]\nSize=1\n",
            );
            assert_eq!(inherits, ["One", "Two"]);
            assert_eq!(dirs.iter().map(|d| d.path.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);
            assert_eq!(dirs[0].distance(64), 0);
            assert_eq!(dirs[0].distance(32), 28);
            assert_eq!(dirs[1].distance(27), 0);
            assert_eq!(dirs[1].distance(30), 2);
            assert_eq!(dirs[2].distance(40), 0);
            assert_eq!(dirs[2].distance(4), 4);
        }

        #[test]
        fn only_svg_and_png_files_are_served() {
            assert!(is_icon_file(Path::new("/x/folder.svg")));
            assert!(is_icon_file(Path::new("/x/folder.PNG")));
            assert!(!is_icon_file(Path::new("/x/folder.xpm")));
            assert!(!is_icon_file(Path::new("/etc/passwd")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_files_share_an_icon_by_type() {
        let home = Path::new("/Users/me");
        let pdf = mac_source(Path::new("/Users/me/Docs/A.PDF"), false, false, false, Some(home));
        assert_eq!(pdf, Source::OsType("pdf".into()));
        assert_eq!(pdf.id(), mac_source(Path::new("/tmp/b.pdf"), false, false, false, Some(home)).id());
        assert_eq!(mac_source(Path::new("/tmp/run"), false, true, false, None), Source::OsType("public.unix-executable".into()));
        assert_eq!(mac_source(Path::new("/tmp/notes"), false, false, false, None), Source::OsType("public.data".into()));
        assert_eq!(mac_source(Path::new("/tmp/trailing."), false, false, false, None), Source::OsType("public.data".into()));
        assert_eq!(mac_source(Path::new("/tmp/Projects"), true, true, false, Some(home)), Source::OsType("public.folder".into()));
    }

    #[test]
    fn mac_apps_special_folders_and_custom_icons_get_their_own() {
        let home = Path::new("/Users/me");
        let own = |path: &str, is_dir: bool, custom: bool| {
            mac_source(Path::new(path), is_dir, false, custom, Some(home)) == Source::OsPath(PathBuf::from(path))
        };
        assert!(own("/Applications/Safari.app", true, false));
        assert!(own("/Users/me", true, false));
        assert!(own("/Users/me/Downloads", true, false));
        assert!(own("/Applications", true, false));
        assert!(own("/Volumes/Backup", true, false));
        assert!(own("/", true, false));
        assert!(own("/Users/me/Work/Logo folder", true, true));
        assert!(own("/Users/me/Work/notes.txt", false, true));
        assert!(!own("/Users/me/Work/Logo folder", true, false));
    }

    #[test]
    fn the_custom_icon_flag_is_read_from_finder_info() {
        let mut info = [0u8; 32];
        assert!(!finder_info_has_custom_icon(&info));
        info[8] = 0x04;
        assert!(finder_info_has_custom_icon(&info));
        assert!(!finder_info_has_custom_icon(&[0u8; 4]));
    }

    #[test]
    fn theme_names_drop_symbolic_ones() {
        let names = ["application-pdf", "application-pdf-symbolic", "x-office-document"].map(String::from);
        assert_eq!(
            theme_source(names.into_iter()),
            Some(Source::ThemeNames(vec!["application-pdf".into(), "x-office-document".into()]))
        );
        assert_eq!(theme_source(["folder-symbolic".to_string()].into_iter()), None);
    }

    #[test]
    fn ids_are_registered_and_unknown_ids_draw_nothing() {
        let icons = Icons::new(None, None);
        assert_eq!(icons.image("type:pdf", 64), None);
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "hi").unwrap();
        let id = icons.id(&file);
        if cfg!(any(target_os = "macos", target_os = "linux")) {
            let id = id.expect("an icon id for a text file");
            assert!(icons.sources.lock().unwrap().contains_key(&id));
        }
        assert_eq!(icons.id(&dir.path().join("missing.txt")), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_names_icons_by_content_type_and_special_folder() {
        let dir = tempfile::tempdir().unwrap();
        let pdf = dir.path().join("report.pdf");
        std::fs::write(&pdf, "%PDF-1.4\n").unwrap();
        let Some(Source::ThemeNames(names)) = source_for(&pdf, None) else { panic!("no theme names for a PDF") };
        assert!(names.iter().any(|n| n == "application-pdf"), "{names:?}");
        let Some(Source::ThemeNames(names)) = source_for(dir.path(), None) else { panic!("no theme names for a folder") };
        assert!(names.iter().any(|n| n.starts_with("folder")), "{names:?}");
    }
}
