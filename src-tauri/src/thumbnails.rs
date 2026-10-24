//! File thumbnails for the grid and list views, cached as PNGs on disk.
//!
//! On macOS they come from Quick Look (`qlmanage`), which covers images
//! (HEIC included), video, PDF and office documents and runs out of process.
//! If Quick Look can't help, or on other systems, common image formats are
//! decoded directly — in this process, so decoding is serialized and large
//! files are skipped rather than allowed to balloon the app's memory.
//!
//! Cache entries are keyed on path, size, modification time and requested
//! size, so editing a file produces a fresh thumbnail. Files that have no
//! thumbnail leave an empty `.none` marker so they aren't retried each time.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

/// Stable 64-bit FNV-1a, so cache names survive Rust upgrades.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn cache_key(path: &Path, meta: &fs::Metadata, size: u32) -> String {
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let id = format!("{}\0{}\0{}\0{}", path.display(), mtime, meta.len(), size);
    format!("{:016x}-{}", fnv1a(id.as_bytes()), size)
}

/// Path of a PNG thumbnail no larger than `size`×`size` for `path`, creating
/// it in `cache_dir` if needed. `Ok(None)` means this file has no thumbnail.
pub fn thumbnail(path: &Path, size: u32, cache_dir: &Path) -> Result<Option<PathBuf>, String> {
    let meta = fs::metadata(path).map_err(|e| format!("Cannot read “{}”: {}", path.display(), e))?;
    if !meta.is_file() {
        return Ok(None);
    }
    let key = cache_key(path, &meta, size);
    let out = cache_dir.join(format!("{}.png", key));
    let none = cache_dir.join(format!("{}.none", key));
    if out.exists() {
        return Ok(Some(out));
    }
    if none.exists() {
        return Ok(None);
    }
    fs::create_dir_all(cache_dir).map_err(|e| format!("Cannot create thumbnail cache: {}", e))?;

    if (cfg!(target_os = "macos") && quick_look(path, size, &out)) || decode_image(path, size, &out) {
        return Ok(Some(out));
    }
    let _ = fs::write(&none, b"");
    Ok(None)
}

/// Ask Quick Look for a thumbnail. Gives up after a timeout: qlmanage can
/// hang on unusual files, and a thumbnail is never worth a stuck worker.
fn quick_look(path: &Path, size: u32, out: &Path) -> bool {
    let Some(name) = path.file_name() else { return false };
    let work = out.with_extension("ql");
    let _ = fs::remove_dir_all(&work);
    if fs::create_dir_all(&work).is_err() {
        return false;
    }
    let child = Command::new("/usr/bin/qlmanage")
        .arg("-t")
        .arg("-s")
        .arg(size.to_string())
        .arg("-o")
        .arg(&work)
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let ok = match child {
        Ok(mut child) => {
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break true,
                    Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
                    _ => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break false;
                    }
                }
            }
        }
        Err(_) => false,
    };
    // qlmanage writes "<file name>.png" into the output folder.
    let mut produced = name.to_os_string();
    produced.push(".png");
    let done = ok && fs::rename(work.join(produced), out).is_ok();
    let _ = fs::remove_dir_all(&work);
    done
}

/// Raster decoding is serialized. The UI can have four thumbnail requests in
/// flight, and four simultaneous full-size decodes of a large photo is where
/// the peak memory went: a 12-megapixel image is ~48 MB as RGBA, so four at
/// once cost ~190 MB. One at a time is free at thumbnail sizes.
static DECODE_LOCK: Mutex<()> = Mutex::new(());

/// Decodes running or waiting for `DECODE_LOCK`.
static DECODES_WAITING: AtomicUsize = AtomicUsize::new(0);

/// Largest image worth decoding for a thumbnail; 80 megapixels is ~320 MB as
/// RGBA, far past anything a 256 px thumbnail needs. Bigger files are treated
/// as thumbnailless instead of risking the app's memory.
const MAX_PIXELS: u64 = 80_000_000;

/// Decode a common image format directly (respecting EXIF rotation) and
/// shrink it to fit `size`. Small images are kept at their own size.
fn decode_image(path: &Path, size: u32, out: &Path) -> bool {
    DECODES_WAITING.fetch_add(1, Ordering::SeqCst);
    let decoded = {
        let _guard = DECODE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        decode_image_now(path, size, out)
    };
    // Trim once the queue has drained, not after every decode: a trim walks
    // the whole heap, and while scrolling a photo folder the next decode is
    // already waiting. Outside the lock, so it never delays one either.
    if DECODES_WAITING.fetch_sub(1, Ordering::SeqCst) == 1 {
        release_free_memory();
    }
    decoded
}

fn decode_image_now(path: &Path, size: u32, out: &Path) -> bool {
    use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader};

    let Ok(reader) = ImageReader::open(path).and_then(|r| r.with_guessed_format()) else { return false };
    if reader.format().is_none() {
        return false;
    }
    let Ok(mut decoder) = reader.into_decoder() else { return false };
    // Read the header size first: a huge scan would allocate hundreds of
    // megabytes just to be thrown away.
    let (width, height) = decoder.dimensions();
    if !worth_decoding(width, height) {
        return false;
    }
    let orientation = decoder.orientation().ok();
    let Ok(mut img) = DynamicImage::from_decoder(decoder) else { return false };
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    if img.width() > size || img.height() > size {
        img = img.thumbnail(size, size);
    }
    let tmp = out.with_extension("tmp");
    if img.save_with_format(&tmp, ImageFormat::Png).is_err() {
        let _ = fs::remove_file(&tmp);
        return false;
    }
    fs::rename(&tmp, out).is_ok()
}

/// Whether an image is small enough that a thumbnail is worth the memory.
fn worth_decoding(width: u32, height: u32) -> bool {
    u64::from(width) * u64::from(height) <= MAX_PIXELS
}

/// Give freed heap back to the OS after a decode.
///
/// glibc keeps released heap in per-arena pools instead of returning it, so a
/// folder of large photos left this process tens of megabytes above where it
/// started long after the thumbnails were written. `malloc_trim` hands those
/// pages back. Only glibc has it, which is what the cfg selects.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn release_free_memory() {
    unsafe extern "C" {
        fn malloc_trim(pad: usize) -> i32;
    }
    // SAFETY: malloc_trim only releases memory the allocator already considers
    // free; it cannot affect live allocations.
    unsafe {
        malloc_trim(0);
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn release_free_memory() {}

/// Delete cache entries older than `max_age`; run once at startup.
pub fn prune(cache_dir: &Path, max_age: Duration) {
    let Ok(entries) = fs::read_dir(cache_dir) else { return };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age > max_age);
        if old {
            let p = entry.path();
            let _ = if p.is_dir() { fs::remove_dir_all(&p) } else { fs::remove_file(&p) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_png(path: &Path, w: u32, h: u32) {
        let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, 128]));
        img.save(path).unwrap();
    }

    #[test]
    fn large_image_is_shrunk_to_fit() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("photo.png");
        write_png(&src, 800, 400);
        let cache = dir.path().join("cache");

        let thumb = thumbnail(&src, 128, &cache).unwrap().expect("thumbnail");
        let (w, h) = image::image_dimensions(&thumb).unwrap();
        assert_eq!((w, h), (128, 64));
    }

    #[test]
    fn oversized_images_are_skipped_instead_of_decoded() {
        // Exactly at the budget is fine; one pixel past it is not.
        assert!(worth_decoding(8000, 10000));
        assert!(worth_decoding(10, 10));
        assert!(!worth_decoding(8001, 10000));
        assert!(!worth_decoding(9000, 9000));
        assert!(!worth_decoding(u32::MAX, u32::MAX), "must not overflow");

        // The size check doesn't get in the way of ordinary images.
        let dir = tempdir().unwrap();
        let src = dir.path().join("scan.png");
        write_png(&src, 400, 400);
        let cache = dir.path().join("cache");
        let thumb = thumbnail(&src, 64, &cache).unwrap().expect("small image still works");
        assert!(thumb.exists());
    }

    #[test]
    fn decoding_one_at_a_time_still_produces_every_thumbnail() {
        // The decode lock serializes work; concurrent requests must all
        // succeed, just in sequence.
        let dir = tempdir().unwrap();
        let cache = dir.path().join("cache");
        let paths: Vec<PathBuf> = (0..4)
            .map(|i| {
                let p = dir.path().join(format!("photo-{i}.png"));
                write_png(&p, 600, 600);
                p
            })
            .collect();

        let handles: Vec<_> = paths
            .into_iter()
            .map(|p| {
                let cache = cache.clone();
                std::thread::spawn(move || thumbnail(&p, 96, &cache).map(|t| t.is_some()))
            })
            .collect();
        for h in handles {
            assert_eq!(h.join().unwrap(), Ok(true));
        }
    }

    #[test]
    fn free_memory_release_is_harmless() {
        // No-op on most platforms; must never touch live allocations.
        release_free_memory();
    }

    #[test]
    fn small_image_keeps_its_size() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("icon.png");
        write_png(&src, 40, 30);

        let thumb = thumbnail(&src, 256, &dir.path().join("cache")).unwrap().unwrap();
        assert_eq!(image::image_dimensions(&thumb).unwrap(), (40, 30));
    }

    #[test]
    fn second_request_is_served_from_cache() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("photo.png");
        write_png(&src, 300, 300);
        let cache = dir.path().join("cache");

        let first = thumbnail(&src, 128, &cache).unwrap().unwrap();
        let stamp = fs::metadata(&first).unwrap().modified().unwrap();
        std::thread::sleep(Duration::from_millis(20));
        let second = thumbnail(&src, 128, &cache).unwrap().unwrap();
        assert_eq!(first, second);
        assert_eq!(fs::metadata(&second).unwrap().modified().unwrap(), stamp);
    }

    #[test]
    fn changing_the_file_changes_the_cache_entry() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("photo.png");
        write_png(&src, 300, 300);
        let cache = dir.path().join("cache");
        let first = thumbnail(&src, 128, &cache).unwrap().unwrap();

        write_png(&src, 300, 200);
        let second = thumbnail(&src, 128, &cache).unwrap().unwrap();
        assert_ne!(first, second);
        let (w, h) = image::image_dimensions(&second).unwrap();
        assert_eq!(w, 128);
        assert!((85..=86).contains(&h), "height {}", h);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn non_image_has_no_thumbnail_and_is_remembered() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("notes.txt");
        fs::write(&src, "hello").unwrap();
        let cache = dir.path().join("cache");

        assert!(thumbnail(&src, 128, &cache).unwrap().is_none());
        let markers = fs::read_dir(&cache).unwrap().filter(|e| e.as_ref().unwrap().path().extension().is_some_and(|x| x == "none")).count();
        assert_eq!(markers, 1);
        assert!(thumbnail(&src, 128, &cache).unwrap().is_none());
    }

    #[test]
    fn folders_and_missing_files() {
        let dir = tempdir().unwrap();
        assert!(thumbnail(dir.path(), 128, &dir.path().join("cache")).unwrap().is_none());
        assert!(thumbnail(&dir.path().join("gone.png"), 128, &dir.path().join("cache")).is_err());
    }

    #[test]
    fn prune_removes_only_old_entries() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("fresh.png"), b"x").unwrap();
        prune(dir.path(), Duration::from_secs(3600));
        assert!(dir.path().join("fresh.png").exists());
        std::thread::sleep(Duration::from_millis(20));
        prune(dir.path(), Duration::from_millis(1));
        assert!(!dir.path().join("fresh.png").exists());
    }
}
