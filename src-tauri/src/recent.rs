//! "Recent": files the user opened lately, newest first.
//!
//! Two sources are merged. On macOS, Spotlight remembers when any app last
//! opened a file (`kMDItemLastUsedDate`), so Recent also shows files opened
//! from the Dock, Finder or other apps. On top of that the app keeps its own
//! list in `recent.json` of files opened from here, which covers places
//! Spotlight doesn't index (external drives, Spotlight turned off).
//!
//! "Remove from Recent" can't change Spotlight's dates, so it records when the
//! file was removed and hides it until it is opened again.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::fs_ops::{entry_for, Entry};

/// Files used longer ago than this drop out of Recent (GNOME's default too).
pub const MAX_AGE_DAYS: i64 = 30;
/// Most files Recent shows.
const MAX_SHOWN: usize = 300;
/// Most files the app's own list remembers.
const MAX_REMEMBERED: usize = 500;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct RecentStore {
    /// Path → when it was last opened from this app (unix seconds).
    #[serde(default)]
    pub opened: HashMap<String, i64>,
    /// Path → when it was removed from Recent.
    #[serde(default)]
    pub removed: HashMap<String, i64>,
}

#[derive(Debug, Serialize)]
pub struct RecentEntry {
    #[serde(flatten)]
    pub entry: Entry,
    /// When the file was last used (unix seconds).
    pub used: i64,
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn read_store(path: &Path) -> RecentStore {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn oldest_kept(now: i64) -> i64 {
    now - MAX_AGE_DAYS * 86_400
}

/// Remember that `path` was opened at `now`.
pub fn record_open(store: &mut RecentStore, path: &str, now: i64) {
    store.opened.insert(path.to_string(), now);
    store.removed.remove(path);
    let cutoff = oldest_kept(now);
    store.opened.retain(|_, t| *t >= cutoff);
    if store.opened.len() > MAX_REMEMBERED {
        let mut times: Vec<i64> = store.opened.values().copied().collect();
        times.sort_unstable_by(|a, b| b.cmp(a));
        let keep_from = times[MAX_REMEMBERED - 1];
        store.opened.retain(|_, t| *t >= keep_from);
    }
}

/// Hide `paths` from Recent until they are next opened.
pub fn forget(store: &mut RecentStore, paths: &[String], now: i64) {
    for p in paths {
        store.opened.remove(p);
        store.removed.insert(p.clone(), now);
    }
    // Anything removed before the cutoff is too old to come back anyway.
    let cutoff = oldest_kept(now);
    store.removed.retain(|_, t| *t >= cutoff);
}

/// Things that shouldn't appear in Recent even if they were opened: hidden
/// files (unless shown), app data in ~/Library, and the insides of bundles.
fn excluded(path: &str, home: &Path, show_hidden: bool) -> bool {
    let library = format!("{}/Library/", home.display());
    if path.starts_with(&library) {
        return true;
    }
    let below_home = path.strip_prefix(&format!("{}/", home.display())).unwrap_or(path);
    let parts: Vec<&str> = below_home.split('/').filter(|p| !p.is_empty()).collect();
    let Some((last, parents)) = parts.split_last() else { return true };
    if parents.iter().any(|p| p.ends_with(".app")) || last.ends_with(".app") {
        return true;
    }
    !show_hidden && parts.iter().any(|p| p.starts_with('.'))
}

/// Merge the app's list with Spotlight's `(path, last used)` pairs into the
/// Recent listing: existing files only, newest first.
pub fn collect(store: &RecentStore, spotlight: Vec<(String, i64)>, home: &Path, show_hidden: bool, now: i64) -> Vec<RecentEntry> {
    let cutoff = oldest_kept(now);
    let mut used: HashMap<String, i64> = HashMap::new();
    let sources = store.opened.iter().map(|(p, t)| (p.clone(), *t)).chain(spotlight);
    for (path, t) in sources {
        let slot = used.entry(path).or_insert(t);
        *slot = (*slot).max(t);
    }

    let mut candidates: Vec<(String, i64)> = used
        .into_iter()
        .filter(|(p, t)| *t >= cutoff && store.removed.get(p).is_none_or(|r| *r < *t))
        .filter(|(p, _)| !excluded(p, home, show_hidden))
        .collect();
    candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    candidates
        .into_iter()
        .filter_map(|(p, t)| entry_for(Path::new(&p)).map(|entry| RecentEntry { entry, used: t }))
        .take(MAX_SHOWN)
        .collect()
}

/// Files under `home` that Spotlight saw used in the last `days` days.
/// Empty if Spotlight is unavailable, off, or too slow to answer.
pub fn spotlight(home: &Path, days: i64) -> Vec<(String, i64)> {
    if !cfg!(target_os = "macos") {
        return Vec::new();
    }
    let child = Command::new("/usr/bin/mdfind")
        .arg("-onlyin")
        .arg(home)
        .arg("-attr")
        .arg("kMDItemLastUsedDate")
        .arg(format!("kMDItemLastUsedDate >= $time.today(-{})", days))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else { return Vec::new() };

    // Read on a thread so a stuck mdfind can be killed after a deadline.
    let mut stdout = child.stdout.take().expect("piped stdout");
    let reader = std::thread::spawn(move || {
        let mut out = String::new();
        let _ = std::io::Read::read_to_string(&mut stdout, &mut out);
        out
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
    let out = reader.join().unwrap_or_default();
    out.lines().filter_map(parse_mdfind_line).collect()
}

/// One line of `mdfind -attr kMDItemLastUsedDate`:
/// `/path/to/file   kMDItemLastUsedDate = 2026-10-23 08:12:44 +0000`.
fn parse_mdfind_line(line: &str) -> Option<(String, i64)> {
    const MARK: &str = "   kMDItemLastUsedDate = ";
    let (path, date) = line.rsplit_once(MARK)?;
    if !path.starts_with('/') {
        return None;
    }
    Some((path.to_string(), parse_date(date.trim())?))
}

/// `2026-10-23 08:12:44 +0000` → unix seconds.
fn parse_date(s: &str) -> Option<i64> {
    let mut parts = s.split(' ');
    let date = parts.next()?;
    let time = parts.next()?;
    let zone = parts.next().unwrap_or("+0000");

    let ymd: Vec<i64> = date.split('-').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    let hms: Vec<i64> = time.split(':').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    let [y, m, d] = ymd[..] else { return None };
    let [hh, mm, ss] = hms[..] else { return None };
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || zone.len() != 5 {
        return None;
    }
    let sign = match &zone[..1] {
        "+" => 1,
        "-" => -1,
        _ => return None,
    };
    let zh: i64 = zone[1..3].parse().ok()?;
    let zm: i64 = zone[3..5].parse().ok()?;
    let offset = sign * (zh * 3600 + zm * 60);
    Some(days_from_civil(y, m, d) * 86_400 + hh * 3600 + mm * 60 + ss - offset)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const NOW: i64 = 1_800_000_000;

    fn names(list: &[RecentEntry]) -> Vec<&str> {
        list.iter().map(|e| e.entry.name.as_str()).collect()
    }

    #[test]
    fn parses_mdfind_output() {
        assert_eq!(parse_date("1970-01-01 00:00:00 +0000"), Some(0));
        assert_eq!(parse_date("2026-10-23 08:12:44 +0000"), Some(1_792_743_164));
        assert_eq!(parse_date("2026-10-23 10:12:44 +0200"), Some(1_792_743_164));
        assert_eq!(parse_date("2024-02-29 12:00:00 -0130"), Some(1_709_213_400));
        assert_eq!(parse_date("yesterday"), None);

        let line = "/Users/me/My Notes.txt   kMDItemLastUsedDate = 2026-10-23 08:12:44 +0000";
        assert_eq!(parse_mdfind_line(line), Some(("/Users/me/My Notes.txt".into(), 1_792_743_164)));
        assert_eq!(parse_mdfind_line("/Users/me/x   kMDItemLastUsedDate = (null)"), None);
        assert_eq!(parse_mdfind_line("garbage"), None);
    }

    #[test]
    fn merges_sources_newest_first_and_skips_missing_files() {
        let dir = tempdir().unwrap();
        let home = dir.path();
        for name in ["a.txt", "b.txt", "c.txt"] {
            fs::write(home.join(name), "x").unwrap();
        }
        let p = |n: &str| home.join(n).to_string_lossy().to_string();

        let mut store = RecentStore::default();
        record_open(&mut store, &p("a.txt"), NOW - 50);
        record_open(&mut store, &p("gone.txt"), NOW - 10);
        let spotlight = vec![(p("b.txt"), NOW - 100), (p("c.txt"), NOW - 20), (p("a.txt"), NOW - 500)];

        let list = collect(&store, spotlight, home, false, NOW);
        assert_eq!(names(&list), ["c.txt", "a.txt", "b.txt"]);
        // The newer of the two times wins.
        assert_eq!(list[1].used, NOW - 50);
    }

    #[test]
    fn old_hidden_and_library_files_are_left_out() {
        let dir = tempdir().unwrap();
        let home = dir.path();
        fs::create_dir_all(home.join("Library/Prefs")).unwrap();
        fs::create_dir_all(home.join(".config")).unwrap();
        fs::create_dir_all(home.join("Tool.app/Contents")).unwrap();
        for f in ["old.txt", "new.txt", ".zshrc", ".config/x.json", "Library/Prefs/p.plist", "Tool.app/Contents/Info.plist"] {
            fs::write(home.join(f), "x").unwrap();
        }
        let p = |n: &str| home.join(n).to_string_lossy().to_string();
        let spotlight = vec![
            (p("old.txt"), NOW - (MAX_AGE_DAYS + 1) * 86_400),
            (p("new.txt"), NOW - 60),
            (p(".zshrc"), NOW - 60),
            (p(".config/x.json"), NOW - 60),
            (p("Library/Prefs/p.plist"), NOW - 60),
            (p("Tool.app"), NOW - 60),
            (p("Tool.app/Contents/Info.plist"), NOW - 60),
        ];
        let store = RecentStore::default();

        assert_eq!(names(&collect(&store, spotlight.clone(), home, false, NOW)), ["new.txt"]);
        let mut with_hidden = names(&collect(&store, spotlight, home, true, NOW)).into_iter().map(String::from).collect::<Vec<_>>();
        with_hidden.sort();
        assert_eq!(with_hidden, [".zshrc", "new.txt", "x.json"]);
    }

    #[test]
    fn removed_files_stay_hidden_until_used_again() {
        let dir = tempdir().unwrap();
        let home = dir.path();
        fs::write(home.join("a.txt"), "x").unwrap();
        let a = home.join("a.txt").to_string_lossy().to_string();

        let mut store = RecentStore::default();
        record_open(&mut store, &a, NOW - 100);
        forget(&mut store, std::slice::from_ref(&a), NOW - 50);
        assert!(store.opened.is_empty());
        let spotlight = vec![(a.clone(), NOW - 100)];
        assert!(collect(&store, spotlight, home, false, NOW).is_empty());

        // Opened again in another app, after the removal.
        let spotlight = vec![(a.clone(), NOW - 10)];
        assert_eq!(collect(&store, spotlight, home, false, NOW).len(), 1);

        // Opened again from this app.
        record_open(&mut store, &a, NOW);
        assert!(store.removed.is_empty());
        assert_eq!(collect(&store, vec![], home, false, NOW).len(), 1);
    }

    #[test]
    fn remembered_list_is_bounded() {
        let mut store = RecentStore::default();
        record_open(&mut store, "/ancient", NOW - (MAX_AGE_DAYS + 5) * 86_400);
        for i in 0..(MAX_REMEMBERED as i64 + 20) {
            record_open(&mut store, &format!("/f{}", i), NOW - 1000 + i);
        }
        assert_eq!(store.opened.len(), MAX_REMEMBERED);
        assert!(!store.opened.contains_key("/ancient"));
        assert!(!store.opened.contains_key("/f0"));
        assert!(store.opened.contains_key(&format!("/f{}", MAX_REMEMBERED + 19)));
    }

    #[test]
    fn store_round_trips_and_tolerates_bad_files() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("recent.json");
        assert!(read_store(&path).opened.is_empty());
        fs::write(&path, "not json").unwrap();
        assert!(read_store(&path).opened.is_empty());

        let mut store = RecentStore::default();
        record_open(&mut store, "/x", NOW);
        fs::write(&path, serde_json::to_string(&store).unwrap()).unwrap();
        assert_eq!(read_store(&path).opened.get("/x"), Some(&NOW));
    }
}
