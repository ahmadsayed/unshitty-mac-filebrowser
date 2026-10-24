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
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
    spotlight_with(Path::new("/usr/bin/mdfind"), home, days, Duration::from_secs(5))
}

/// `spotlight` with the program and timeout passed in, so it can be tested
/// with a stand-in for mdfind on any platform.
fn spotlight_with(program: &Path, home: &Path, days: i64, timeout: Duration) -> Vec<(String, i64)> {
    let child = Command::new(program)
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
    crate::platform::wait_or_kill(&mut child, timeout);
    let out = reader.join().unwrap_or_default();
    out.lines().filter_map(parse_mdfind_line).collect()
}

/// Files that were used recently in any app, newest first, as
/// `(path, unix seconds)`. macOS asks Spotlight; Linux reads the GTK
/// recently-used list.
pub fn system_recent(home: &Path, days: i64) -> Vec<(String, i64)> {
    if cfg!(target_os = "macos") {
        return spotlight(home, days);
    }
    if cfg!(target_os = "linux") {
        return recently_used(home, days);
    }
    Vec::new()
}

/// Linux's answer to Spotlight: `~/.local/share/recently-used.xbel`, the list
/// GTK apps add to when they open a file. Returns only files used within the
/// last `days` days, newest first.
pub fn recently_used(home: &Path, days: i64) -> Vec<(String, i64)> {
    let list = crate::platform::data_home(home).join("recently-used.xbel");
    read_recently_used(&list, days)
}

/// `recently_used` with the file named directly, so it can be tested without
/// touching the environment.
pub fn read_recently_used(list: &Path, days: i64) -> Vec<(String, i64)> {
    let Some(text) = crate::platform::read_text(list) else { return Vec::new() };
    let cutoff = now() - days * 86_400;
    let mut found: Vec<(String, i64)> =
        parse_xbel(&text).into_iter().filter(|(_, t)| *t >= cutoff).collect();
    found.sort_unstable_by(|a, b| b.1.cmp(&a.1));
    found
}

/// One `<bookmark href="file:///…" modified="…">` element per remembered file.
/// Parsed by hand: the format is flat enough that an XML dependency would cost
/// more than it saves.
fn parse_xbel(text: &str) -> Vec<(String, i64)> {
    const OPEN: &str = "<bookmark ";
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        let after = &rest[start + OPEN.len()..];
        let Some(end) = after.find('>') else { break };
        let tag = &after[..end];
        // `modified` is what GTK updates; `visited` is the fallback.
        let when = attr(tag, "modified").or_else(|| attr(tag, "visited"));
        if let (Some(href), Some(when)) = (attr(tag, "href"), when) {
            if let (Some(path), Some(time)) = (crate::platform::file_uri_to_path(&href), parse_iso(&when)) {
                out.push((path.to_string_lossy().to_string(), time));
            }
        }
        rest = &after[end..];
    }
    out
}

/// The value of `name="…"` inside an XML tag, with XML escapes decoded.
fn attr(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{}=\"", name);
    let start = tag.find(&needle)? + needle.len();
    let rest = &tag[start..];
    Some(xml_unescape(&rest[..rest.find('"')?]))
}

/// Decode the five predefined XML entities and `&#…;` character references.
/// GTK escapes `&`, `<`, `>`, `"` and `'` that are left unescaped by URI
/// encoding, so a file named `Tom & Jerry.odt` arrives as `Tom%20&amp;%20…`.
/// Anything unrecognised is kept as written.
fn xml_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest.find(';') else { break };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// `2026-10-23T08:12:44Z` (fractional seconds and numeric zones allowed) →
/// unix seconds. No zone is read as UTC.
fn parse_iso(s: &str) -> Option<i64> {
    let (date, time) = s.split_once('T')?;
    let ymd: Vec<i64> = date.split('-').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    let [y, m, d] = ymd[..] else { return None };
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }

    let (clock, offset) = match time.strip_suffix('Z') {
        Some(clock) => (clock, 0),
        None => match time.rfind(['+', '-']) {
            Some(at) => (&time[..at], parse_offset(&time[at..])?),
            None => (time, 0),
        },
    };
    let hms: Vec<i64> = clock
        .split(':')
        .take(3)
        .map(|p| p.chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok())
        .collect::<Option<_>>()?;
    let [hh, mm, ss] = hms[..] else { return None };
    Some(days_from_civil(y, m, d) * 86_400 + hh * 3600 + mm * 60 + ss - offset)
}

/// `+02:00` or `-0130` → seconds east of UTC.
fn parse_offset(s: &str) -> Option<i64> {
    let sign = match s.chars().next()? {
        '+' => 1,
        '-' => -1,
        _ => return None,
    };
    let digits: String = s[1..].chars().filter(char::is_ascii_digit).collect();
    let (h, m): (i64, i64) = match digits.len() {
        2 => (digits.parse().ok()?, 0),
        4 => (digits[..2].parse().ok()?, digits[2..].parse().ok()?),
        _ => return None,
    };
    Some(sign * (h * 3600 + m * 60))
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

    #[cfg(unix)]
    fn fake_mdfind(dir: &Path, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("mdfind");
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn spotlight_parses_what_mdfind_prints_and_gives_up_on_a_stuck_one() {
        let dir = tempfile::tempdir().unwrap();
        let mdfind = fake_mdfind(
            dir.path(),
            r#"echo "/Users/me/a.txt   kMDItemLastUsedDate = 2026-10-23 08:12:44 +0000"
echo "not a result line""#,
        );
        let found = spotlight_with(&mdfind, Path::new("/Users/me"), 30, Duration::from_secs(5));
        assert_eq!(found, vec![("/Users/me/a.txt".to_string(), 1_792_743_164)]);

        let stuck = fake_mdfind(dir.path(), "exec sleep 10");
        assert!(spotlight_with(&stuck, Path::new("/"), 30, Duration::from_millis(100)).is_empty());
        let missing = dir.path().join("nope");
        assert!(spotlight_with(&missing, Path::new("/"), 30, Duration::from_secs(1)).is_empty());
    }
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

    // --- recently-used.xbel (Linux) --------------------------------------

    /// A trickle of what GTK writes: two bookmarks and the surrounding noise.
    const XBEL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<xbel version="1.0" xmlns:bookmark="http://www.freedesktop.org/standards/desktop-bookmarks">
  <bookmark href="file:///home/me/My%20Notes.txt" added="2026-10-20T08:00:00Z" modified="2026-10-23T08:12:44Z" visited="2026-10-23T08:12:44Z">
    <info><metadata owner="http://freedesktop.org">
      <mime:mime-type type="text/plain"/>
    </metadata></info>
  </bookmark>
  <bookmark href="file:///home/me/Music/song.mp3" added="2026-01-01T00:00:00Z" modified="2026-01-02T00:00:00Z"/>
</xbel>
"#;

    #[test]
    fn parses_the_recently_used_list() {
        let mut found = parse_xbel(XBEL);
        found.sort_unstable_by(|a, b| b.1.cmp(&a.1));
        assert_eq!(
            found,
            vec![
                ("/home/me/My Notes.txt".to_string(), 1_792_743_164),
                ("/home/me/Music/song.mp3".to_string(), 1_767_312_000),
            ]
        );
        // Comments, unknown elements and missing attributes are skipped.
        assert!(parse_xbel("<xbel><bookmark href=\"file:///x\"/></xbel>").is_empty());
        assert!(parse_xbel("not xml at all").is_empty());
        assert!(parse_xbel("<bookmark").is_empty());
    }

    #[test]
    fn xml_escapes_in_hrefs_are_decoded() {
        let xbel = r#"<bookmark href="file:///home/me/Tom%20&amp;%20Jerry&apos;s%20&lt;1&gt;.odt" modified="2026-10-23T08:12:44Z"/>"#;
        assert_eq!(parse_xbel(xbel), vec![("/home/me/Tom & Jerry's <1>.odt".to_string(), 1_792_743_164)]);
        assert_eq!(xml_unescape("a&#38;b&#x27;c"), "a&b'c");
        assert_eq!(xml_unescape("plain"), "plain");
        // Stray or unknown ampersands are kept rather than dropped.
        assert_eq!(xml_unescape("a & b &bogus; c&"), "a & b &bogus; c&");
    }

    #[test]
    fn iso_dates_ignore_fractions_and_offsets() {
        assert_eq!(parse_iso("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso("2026-10-23T08:12:44Z"), Some(1_792_743_164));
        assert_eq!(parse_iso("2026-10-23T08:12:44.123456Z"), Some(1_792_743_164));
        assert_eq!(parse_iso("2026-10-23T10:12:44+02:00"), Some(1_792_743_164));
        assert_eq!(parse_iso("2026-13-01T00:00:00Z"), None);
        assert_eq!(parse_iso("yesterday"), None);
    }

    #[test]
    fn recently_used_reads_the_xbel_file_and_honours_the_cutoff() {
        let dir = tempdir().unwrap();
        let list = dir.path().join("recently-used.xbel");
        fs::write(
            &list,
            r#"<xbel>
              <bookmark href="file:///home/me/recent.txt" modified="2026-10-23T08:12:44Z"/>
              <bookmark href="file:///home/me/ancient.txt" modified="2000-01-01T00:00:00Z"/>
            </xbel>"#,
        )
        .unwrap();

        let all = read_recently_used(&list, 100_000);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].0, "/home/me/recent.txt", "newest first");

        // The 2000 entry falls outside a 30-day window.
        let recent = read_recently_used(&list, 30);
        assert_eq!(recent, vec![("/home/me/recent.txt".to_string(), 1_792_743_164)]);

        assert!(read_recently_used(&dir.path().join("missing.xbel"), 30).is_empty());
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
