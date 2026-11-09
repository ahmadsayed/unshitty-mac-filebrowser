//! File thumbnails for the grid and list views, cached on disk.
//!
//! A thumbnail the desktop already made is used as it is: on Linux, the
//! shared freedesktop cache (`~/.cache/thumbnails`, which GNOME Files and
//! other apps fill) is read, never written, and an entry counts only while
//! its URI and modification time match the file's.
//!
//! On macOS they come from Quick Look (`qlmanage`), which covers images
//! (HEIC included), video, PDF and office documents and runs out of process.
//! If Quick Look can't help, or on other systems, common image formats are
//! decoded directly — in this process, so decoding is serialized and large
//! files are skipped rather than allowed to balloon the app's memory. A
//! decoded picture is kept as a small JPEG, or as PNG if it is transparent.
//!
//! Cache entries are keyed on path, size, modification time and requested
//! size, so editing a file produces a fresh thumbnail. Files that have no
//! thumbnail leave an empty `.none` marker so they aren't retried each time.
//! The cache is pruned at startup by age and kept under `MAX_CACHE_BYTES`.
//!
//! The view cancels requests for items it no longer shows: a cancelled
//! request kills its Quick Look helper or skips its decode, and leaves
//! nothing in the cache.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, Cursor, Seek};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

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

/// The error of a request cancelled before its thumbnail was made.
pub const CANCELLED: &str = "Cancelled.";

/// `thumbnail_unless` for a request that is never cancelled.
#[cfg(test)]
fn thumbnail(path: &Path, size: u32, cache_dir: &Path) -> Result<Option<PathBuf>, String> {
    thumbnail_unless(path, size, cache_dir, None, &|| false)
}

/// Path of a PNG thumbnail no larger than `size`×`size` for `path`, creating
/// it in `cache_dir` if needed. `Ok(None)` means this file has no thumbnail;
/// `Err(CANCELLED)` that `stop` said the view no longer wants it.
///
/// `shared` is the desktop's thumbnail cache (`~/.cache/thumbnails`), read
/// only; `None` where there is none (macOS).
pub fn thumbnail_unless(
    path: &Path,
    size: u32,
    cache_dir: &Path,
    shared: Option<&Path>,
    stop: &dyn Fn() -> bool,
) -> Result<Option<PathBuf>, String> {
    let meta =
        fs::metadata(path).map_err(|e| format!("Cannot read “{}”: {}", path.display(), e))?;
    if !meta.is_file() {
        return Ok(None);
    }
    let base = cache_dir.join(cache_key(path, &meta, size));
    // JPEG for most; PNG for transparent pictures, Quick Look's, and every
    // entry made before JPEG.
    for ext in ["jpg", "png"] {
        let out = base.with_extension(ext);
        if out.exists() {
            return Ok(Some(out));
        }
    }
    // Before the marker: the desktop may have made one since (a video).
    if let Some(found) = shared.and_then(|dir| shared_thumbnail(path, &meta, size, dir)) {
        return Ok(Some(found));
    }
    let none = base.with_extension("none");
    if none.exists() {
        return Ok(None);
    }
    fs::create_dir_all(cache_dir).map_err(|e| format!("Cannot create thumbnail cache: {}", e))?;

    let png = base.with_extension("png");
    if cfg!(target_os = "macos") && quick_look(path, size, &png, stop) {
        return Ok(Some(png));
    }
    if let Some(out) = decode_image(path, size, &base, stop) {
        return Ok(Some(out));
    }
    // A cancelled request says nothing about the file: don't remember it
    // as thumbnailless.
    if stop() {
        return Err(CANCELLED.into());
    }
    let _ = fs::write(&none, b"");
    Ok(None)
}

/// Ask Quick Look for a thumbnail. Gives up after a timeout: qlmanage can
/// hang on unusual files, and a thumbnail is never worth a stuck worker.
fn quick_look(path: &Path, size: u32, out: &Path, stop: &dyn Fn() -> bool) -> bool {
    quick_look_with(
        Path::new("/usr/bin/qlmanage"),
        path,
        size,
        out,
        Duration::from_secs(15),
        stop,
    )
}

/// `quick_look` with the program and timeout passed in, so it can be tested
/// with a stand-in for qlmanage on any platform.
fn quick_look_with(
    program: &Path,
    path: &Path,
    size: u32,
    out: &Path,
    timeout: Duration,
    stop: &dyn Fn() -> bool,
) -> bool {
    let Some(name) = path.file_name() else {
        return false;
    };
    if stop() {
        return false;
    }
    let work = out.with_extension("ql");
    let _ = fs::remove_dir_all(&work);
    if fs::create_dir_all(&work).is_err() {
        return false;
    }
    let mut command = Command::new(program);
    command
        .arg("-t")
        .arg("-s")
        .arg(size.to_string())
        .arg("-o")
        .arg(&work)
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = crate::platform::spawn_helper(&mut command);
    let ok = child
        .is_ok_and(|mut child| crate::platform::wait_or_kill_unless(&mut child, timeout, stop));
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
/// shrink it to fit `size`. Small images are kept at their own size. Writes
/// `base` with the extension of the format it chose, and returns that path.
/// A request cancelled while it waits for its turn is skipped.
fn decode_image(path: &Path, size: u32, base: &Path, stop: &dyn Fn() -> bool) -> Option<PathBuf> {
    one_decode_at_a_time(|| if stop() { None } else { decode_image_now(path, size, base) })
}

/// Runs `decode` once no other decode is running, so only one image is in
/// memory at a time.
fn one_decode_at_a_time<T>(decode: impl FnOnce() -> T) -> T {
    DECODES_WAITING.fetch_add(1, Ordering::SeqCst);
    let decoded = {
        let _guard = DECODE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        decode()
    };
    // Trim once the queue has drained, not after every decode: a trim walks
    // the whole heap, and while scrolling a photo folder the next decode is
    // already waiting. Outside the lock, so it never delays one either.
    if DECODES_WAITING.fetch_sub(1, Ordering::SeqCst) == 1 {
        release_free_memory();
    }
    decoded
}

/// JPEG quality of the thumbnails the app decodes: small (a 256 px photo is
/// ~15 KB, against ~120 KB as PNG) with no visible loss at icon sizes.
const JPEG_QUALITY: u8 = 80;

fn decode_image_now(path: &Path, size: u32, base: &Path) -> Option<PathBuf> {
    let img = image::ImageReader::open(path)
        .ok()
        .and_then(|reader| shrunk(reader, size))?;
    let tmp = base.with_extension("tmp");
    let (out, written) = if is_transparent(&img) {
        let out = base.with_extension("png");
        (out, img.save_with_format(&tmp, image::ImageFormat::Png).is_ok())
    } else {
        (base.with_extension("jpg"), save_jpeg(&img, &tmp))
    };
    if !written {
        let _ = fs::remove_file(&tmp);
        return None;
    }
    fs::rename(&tmp, &out).ok()?;
    Some(out)
}

/// Whether any pixel of `img` lets the background through: JPEG has no
/// alpha, and would show it black. A screenshot's alpha channel is usually
/// all opaque, which JPEG keeps as it is.
fn is_transparent(img: &image::DynamicImage) -> bool {
    img.color().has_alpha() && img.to_rgba8().pixels().any(|p| p.0[3] < 255)
}

fn save_jpeg(img: &image::DynamicImage, out: &Path) -> bool {
    let Ok(file) = fs::File::create(out) else {
        return false;
    };
    let mut writer = std::io::BufWriter::new(file);
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut writer, JPEG_QUALITY);
    img.to_rgb8().write_with_encoder(encoder).is_ok() && std::io::Write::flush(&mut writer).is_ok()
}

// ---------------------------------------------------------------------------
// The desktop's shared thumbnails, read only (freedesktop Thumbnail
// Managing Standard): `<dir>/<size folder>/<md5 of the file URI>.png`
// ---------------------------------------------------------------------------

/// The shared cache's folders and the largest side of their thumbnails.
const SHARED_SIZES: [(&str, u32); 4] =
    [("normal", 128), ("large", 256), ("x-large", 512), ("xx-large", 1024)];

/// The desktop's thumbnail of `path`, at least `size` pixels, if it is
/// current: made for this file's URI at its present modification time.
/// Only the PNG's text is read; nothing in `shared` is written.
fn shared_thumbnail(path: &Path, meta: &fs::Metadata, size: u32, shared: &Path) -> Option<PathBuf> {
    let uri = crate::platform::path_to_file_uri(path);
    let name = format!("{}.png", md5_hex(&uri));
    let mtime = meta.modified().ok()?.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    SHARED_SIZES
        .iter()
        .filter(|(_, side)| *side >= size)
        .map(|(folder, _)| shared.join(folder).join(&name))
        .find(|candidate| shared_is_current(candidate, &uri, mtime))
}

fn md5_hex(text: &str) -> String {
    use md5::{Digest, Md5};
    Md5::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether the shared thumbnail at `candidate` names `uri` and `mtime`
/// (`Thumb::URI`, `Thumb::MTime`); missing or unreadable is not current.
fn shared_is_current(candidate: &Path, uri: &str, mtime: Duration) -> bool {
    let Some(text) = png_text(candidate) else {
        return false;
    };
    let field = |key: &str| text.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
    field("Thumb::URI") == Some(uri) && field("Thumb::MTime").is_some_and(|t| same_mtime(t, mtime))
}

/// A `Thumb::MTime` against the file's: whole seconds, as GNOME writes it,
/// or with a fraction, compared to the microsecond.
fn same_mtime(text: &str, mtime: Duration) -> bool {
    let (secs, fraction) = text.trim().split_once('.').unwrap_or((text.trim(), ""));
    if secs.parse::<u64>().ok() != Some(mtime.as_secs()) {
        return false;
    }
    if fraction.is_empty() {
        return true;
    }
    let micros: String = fraction.chars().chain(std::iter::repeat('0')).take(6).collect();
    micros.parse::<u32>().ok() == Some(mtime.subsec_micros())
}

/// The text chunks before a PNG's image data, as keyword and text.
fn png_text(path: &Path) -> Option<Vec<(String, String)>> {
    let file = fs::File::open(path).ok()?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_ignore_text_chunk(false);
    let reader = decoder.read_info().ok()?;
    let info = reader.info();
    let mut text: Vec<(String, String)> = info
        .uncompressed_latin1_text
        .iter()
        .map(|t| (t.keyword.clone(), t.text.clone()))
        .collect();
    text.extend(info.utf8_text.iter().filter_map(|t| Some((t.keyword.clone(), t.get_text().ok()?))));
    Some(text)
}

/// A thumbnail of the image in `bytes` as PNG, at most `size` pixels square
/// (a smaller image keeps its size); `None` when the bytes aren't an image
/// the app decodes, or one too large to be worth it. For the pictures a
/// remote place hands over: an Arbesk asset's thumbnail. A request
/// cancelled while it waits for its turn is skipped.
pub fn png_from_bytes(bytes: &[u8], size: u32, stop: &dyn Fn() -> bool) -> Option<Vec<u8>> {
    let img = one_decode_at_a_time(|| {
        if stop() {
            return None;
        }
        shrunk(image::ImageReader::new(Cursor::new(bytes)), size)
    })?;
    let mut png = Vec::new();
    img.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(png)
}

/// The image `reader` holds, turned upright (EXIF orientation) and shrunk
/// to fit `size`.
fn shrunk<R: BufRead + Seek>(reader: image::ImageReader<R>, size: u32) -> Option<image::DynamicImage> {
    use image::ImageDecoder;

    let reader = reader.with_guessed_format().ok()?;
    reader.format()?;
    let mut decoder = reader.into_decoder().ok()?;
    // Read the header size first: a huge scan would allocate hundreds of
    // megabytes just to be thrown away.
    let (width, height) = decoder.dimensions();
    if !worth_decoding(width, height) {
        return None;
    }
    let orientation = decoder.orientation().ok();
    let mut img = image::DynamicImage::from_decoder(decoder).ok()?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    if img.width() > size || img.height() > size {
        img = img.thumbnail(size, size);
    }
    Some(img)
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

/// The most the thumbnail cache keeps on disk; past it, `prune` deletes
/// the oldest entries.
pub const MAX_CACHE_BYTES: u64 = 100 * 1024 * 1024;

/// Delete cache entries older than `max_age`, then the oldest files until
/// the rest fit in `max_bytes`; run once at startup.
pub fn prune(cache_dir: &Path, max_age: Duration, max_bytes: u64) {
    let Ok(entries) = fs::read_dir(cache_dir) else {
        return;
    };
    let now = SystemTime::now();
    let mut kept: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let modified = meta.modified().unwrap_or(now);
        let p = entry.path();
        if now.duration_since(modified).is_ok_and(|age| age > max_age) {
            let _ = if meta.is_dir() {
                fs::remove_dir_all(&p)
            } else {
                fs::remove_file(&p)
            };
        } else if meta.is_file() {
            kept.push((modified, meta.len(), p));
        }
    }
    let mut total: u64 = kept.iter().map(|(_, len, _)| len).sum();
    kept.sort();
    for (_, len, p) in kept {
        if total <= max_bytes {
            break;
        }
        if fs::remove_file(&p).is_ok() {
            total -= len;
        }
    }
}

// ---------------------------------------------------------------------------
// Requests in progress, so a window can cancel the ones it no longer shows
// ---------------------------------------------------------------------------

type Running = HashMap<(String, u64), Arc<AtomicBool>>;

/// Thumbnail requests running now, keyed by window and the window's ticket.
#[derive(Clone, Default)]
pub struct Requests {
    running: Arc<Mutex<Running>>,
}

/// One running request; it is forgotten when this is dropped.
pub struct Request {
    requests: Requests,
    key: (String, u64),
    stop: Arc<AtomicBool>,
}

impl Requests {
    fn lock(&self) -> std::sync::MutexGuard<'_, Running> {
        self.running.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn start(&self, window: &str, ticket: u64) -> Request {
        let key = (window.to_string(), ticket);
        let stop = Arc::new(AtomicBool::new(false));
        self.lock().insert(key.clone(), stop.clone());
        Request { requests: self.clone(), key, stop }
    }

    /// Cancel `window`'s requests with these tickets; finished ones are ignored.
    pub fn cancel(&self, window: &str, tickets: &[u64]) {
        let running = self.lock();
        for ticket in tickets {
            if let Some(stop) = running.get(&(window.to_string(), *ticket)) {
                stop.store(true, Ordering::SeqCst);
            }
        }
    }

    #[cfg(test)]
    fn running(&self) -> usize {
        self.lock().len()
    }
}

impl Request {
    #[cfg(test)]
    fn cancelled(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// The flag to poll from the thread doing the work.
    pub fn stop_flag(&self) -> Arc<AtomicBool> {
        self.stop.clone()
    }
}

impl Drop for Request {
    fn drop(&mut self) {
        self.requests.lock().remove(&self.key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// A stand-in program: a shell script with `body`, made executable.
    #[cfg(unix)]
    fn fake_program(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fake");
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn quick_look_takes_the_png_qlmanage_writes() {
        let dir = tempdir().unwrap();
        // Like qlmanage: -t -s SIZE -o DIR FILE writes DIR/<file name>.png.
        let ql = fake_program(dir.path(), r#"echo png > "$5/$(basename "$6").png""#);
        let src = dir.path().join("photo.heic");
        fs::write(&src, b"x").unwrap();
        let out = dir.path().join("thumb.png");
        assert!(quick_look_with(
            &ql,
            &src,
            256,
            &out,
            Duration::from_secs(5),
            &|| false
        ));
        assert_eq!(fs::read_to_string(&out).unwrap(), "png\n");
        assert!(
            !dir.path().join("thumb.ql").exists(),
            "the work folder is cleaned up"
        );
    }

    #[cfg(unix)]
    #[test]
    fn quick_look_fails_cleanly_when_qlmanage_fails_hangs_or_is_missing() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("doc.pages");
        fs::write(&src, b"x").unwrap();
        let out = dir.path().join("thumb.png");
        let silent = fake_program(dir.path(), "exit 0"); // writes nothing
        assert!(!quick_look_with(
            &silent,
            &src,
            256,
            &out,
            Duration::from_secs(5),
            &|| false
        ));
        let stuck = fake_program(dir.path(), "exec sleep 10");
        assert!(!quick_look_with(
            &stuck,
            &src,
            256,
            &out,
            Duration::from_millis(100),
            &|| false
        ));
        assert!(!quick_look_with(
            &dir.path().join("missing"),
            &src,
            256,
            &out,
            Duration::from_secs(1),
            &|| false
        ));
        assert!(
            !quick_look_with(&silent, Path::new("/"), 256, &out, Duration::from_secs(1), &|| false),
            "no file name"
        );
        assert!(!out.exists());
        assert!(!dir.path().join("thumb.ql").exists());
    }

    fn write_png(path: &Path, w: u32, h: u32) {
        let img = image::RgbImage::from_fn(w, h, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
        });
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

    fn encoded(w: u32, h: u32, format: image::ImageFormat) -> Vec<u8> {
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(w, h, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
        }));
        let mut bytes = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut bytes), format)
            .unwrap();
        bytes
    }

    fn png_size(png: &[u8]) -> (u32, u32) {
        use image::GenericImageView;
        assert_eq!(image::guess_format(png).unwrap(), image::ImageFormat::Png);
        image::load_from_memory(png).unwrap().dimensions()
    }

    #[test]
    fn image_bytes_become_a_png_that_fits_the_size() {
        // Arbesk keeps its thumbnails as WebP.
        let webp = encoded(800, 400, image::ImageFormat::WebP);
        assert_eq!(png_size(&png_from_bytes(&webp, 128, &|| false).unwrap()), (128, 64));
        // Smaller than asked: kept at its own size.
        let small = encoded(40, 30, image::ImageFormat::Png);
        assert_eq!(png_size(&png_from_bytes(&small, 128, &|| false).unwrap()), (40, 30));
    }

    #[test]
    fn bytes_that_are_not_an_image_have_no_thumbnail() {
        assert!(png_from_bytes(b"", 128, &|| false).is_none());
        assert!(png_from_bytes(br#"{"not":"an image"}"#, 128, &|| false).is_none());
        let png = encoded(64, 64, image::ImageFormat::Png);
        assert!(png_from_bytes(&png[..png.len() / 2], 128, &|| false).is_none(), "cut short");
    }

    #[test]
    fn a_cancelled_picture_is_not_decoded() {
        let png = encoded(64, 64, image::ImageFormat::Png);
        assert!(png_from_bytes(&png, 128, &|| true).is_none());
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
        let thumb = thumbnail(&src, 64, &cache)
            .unwrap()
            .expect("small image still works");
        assert!(thumb.exists());
    }

    #[test]
    fn decoding_one_at_a_time_still_produces_every_thumbnail() {
        // The decode lock serializes work; concurrent requests must all
        // succeed, just in sequence. Call the decoder directly: thumbnail()
        // can take Quick Look on macOS and never exercise this lock.
        let dir = tempdir().unwrap();
        let cache = dir.path().join("cache");
        fs::create_dir(&cache).unwrap();
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
                std::thread::spawn(move || {
                    let out = decode_image(&p, 96, &cache.join(p.file_stem().unwrap()), &|| false)
                        .expect("decoded");
                    assert_eq!(image::image_dimensions(&out).unwrap(), (96, 96));
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
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

        let thumb = thumbnail(&src, 256, &dir.path().join("cache"))
            .unwrap()
            .unwrap();
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
        let markers = fs::read_dir(&cache)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|x| x == "none")
            })
            .count();
        assert_eq!(markers, 1);
        assert!(thumbnail(&src, 128, &cache).unwrap().is_none());
    }

    #[test]
    fn folders_and_missing_files() {
        let dir = tempdir().unwrap();
        assert!(
            thumbnail(dir.path(), 128, &dir.path().join("cache"))
                .unwrap()
                .is_none()
        );
        assert!(thumbnail(&dir.path().join("gone.png"), 128, &dir.path().join("cache")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn cancelling_quick_look_stops_the_helper_at_once() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("movie.mov");
        fs::write(&src, b"x").unwrap();
        let out = dir.path().join("thumb.png");
        let stuck = fake_program(dir.path(), "exec sleep 10");
        let started = std::time::Instant::now();
        let stop = || started.elapsed() > Duration::from_millis(100);
        assert!(!quick_look_with(&stuck, &src, 256, &out, Duration::from_secs(15), &stop));
        assert!(started.elapsed() < Duration::from_secs(2), "took {:?}", started.elapsed());
        assert!(!out.exists());
        assert!(!dir.path().join("thumb.ql").exists());
    }

    #[test]
    fn a_decode_cancelled_while_waiting_its_turn_is_skipped() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("photo.png");
        write_png(&src, 300, 300);
        let out = dir.path().join("thumb");
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let turn = DECODE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let waiting = {
            let (src, out, stop) = (src.clone(), out.clone(), stop.clone());
            std::thread::spawn(move || decode_image(&src, 128, &out, &|| stop.load(Ordering::SeqCst)))
        };
        std::thread::sleep(Duration::from_millis(50));
        stop.store(true, Ordering::SeqCst);
        drop(turn);
        assert!(waiting.join().unwrap().is_none());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1, "nothing but the photo");
    }

    #[test]
    fn a_cancelled_thumbnail_is_not_remembered_as_missing() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("photo.png");
        write_png(&src, 300, 300);
        let cache = dir.path().join("cache");
        assert_eq!(thumbnail_unless(&src, 128, &cache, None, &|| true), Err(CANCELLED.to_string()));
        let left = fs::read_dir(&cache).map(|d| d.count()).unwrap_or(0);
        assert_eq!(left, 0, "no thumbnail and no .none marker");
        // Asked again later, it is made as usual.
        assert!(thumbnail(&src, 128, &cache).unwrap().is_some());
    }

    #[test]
    fn requests_are_cancelled_per_window_and_forgotten_when_done() {
        let requests = Requests::default();
        let a1 = requests.start("main", 1);
        let a2 = requests.start("main", 2);
        let b1 = requests.start("other", 1);
        requests.cancel("main", &[1, 7]);
        assert!(a1.cancelled());
        assert!(!a2.cancelled());
        assert!(!b1.cancelled(), "another window's ticket 1 is a different request");
        drop((a1, a2, b1));
        assert_eq!(requests.running(), 0);
        // Cancelling a finished request does nothing and keeps nothing.
        requests.cancel("main", &[2]);
        assert_eq!(requests.running(), 0);
    }

    #[test]
    fn prune_removes_only_old_entries() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("fresh.png"), b"x").unwrap();
        prune(dir.path(), Duration::from_secs(3600), u64::MAX);
        assert!(dir.path().join("fresh.png").exists());
        std::thread::sleep(Duration::from_millis(20));
        prune(dir.path(), Duration::from_millis(1), u64::MAX);
        assert!(!dir.path().join("fresh.png").exists());
    }

    // ---- The desktop's shared thumbnails (read only) and the cache's formats

    /// A thumbnail in the desktop's shared cache, as GNOME's thumbnailer
    /// writes it: `<shared>/<folder>/<md5 of the URI>.png`, with the URI and
    /// modification time as text. `None` leaves that text out.
    fn write_shared(
        shared: &Path,
        folder: &str,
        uri: &str,
        mtime: Option<&str>,
        uri_text: Option<&str>,
    ) -> PathBuf {
        use md5::{Digest, Md5};
        let dir = shared.join(folder);
        fs::create_dir_all(&dir).unwrap();
        let name: String = Md5::digest(uri.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
        let out = dir.join(format!("{name}.png"));
        let mut encoder = png::Encoder::new(fs::File::create(&out).unwrap(), 4, 3);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.add_text_chunk("Software".into(), "GNOME::ThumbnailFactory".into()).unwrap();
        if let Some(mtime) = mtime {
            encoder.add_text_chunk("Thumb::MTime".into(), mtime.into()).unwrap();
        }
        if let Some(uri) = uri_text {
            encoder.add_text_chunk("Thumb::URI".into(), uri.into()).unwrap();
        }
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[200; 4 * 3 * 3]).unwrap();
        out
    }

    fn mtime_secs(path: &Path) -> String {
        let modified = fs::metadata(path).unwrap().modified().unwrap();
        modified.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs().to_string()
    }

    /// Every file under `dir` with its length and modification time.
    fn snapshot(dir: &Path) -> Vec<(PathBuf, u64, SystemTime)> {
        let mut found = Vec::new();
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let meta = entry.metadata().unwrap();
            if meta.is_dir() {
                found.extend(snapshot(&entry.path()));
            } else {
                found.push((entry.path(), meta.len(), meta.modified().unwrap()));
            }
        }
        found.sort();
        found
    }

    #[test]
    fn a_current_desktop_thumbnail_is_used_as_it_is() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("My photo #1.jpg");
        write_png(&src, 300, 200);
        let (shared, cache) = (dir.path().join("thumbnails"), dir.path().join("cache"));
        let uri = crate::platform::path_to_file_uri(&src);
        let large = write_shared(&shared, "large", &uri, Some(&mtime_secs(&src)), Some(&uri));

        let found = thumbnail_unless(&src, 256, &cache, Some(&shared), &|| false).unwrap();
        assert_eq!(found, Some(large.clone()));
        // A smaller request takes the larger one when its own size is missing.
        let small = thumbnail_unless(&src, 128, &cache, Some(&shared), &|| false).unwrap();
        assert_eq!(small, Some(large));
        assert!(snapshot(&cache).is_empty(), "nothing decoded, nothing copied");
    }

    #[test]
    fn a_stale_foreign_or_smaller_desktop_thumbnail_is_not_used() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("photo.png");
        write_png(&src, 300, 200);
        let uri = crate::platform::path_to_file_uri(&src);
        let now = mtime_secs(&src);
        let older = (now.parse::<u64>().unwrap() - 60).to_string();
        let cases: [(&str, Option<&str>, Option<&str>); 5] = [
            ("large", Some(&older), Some(&uri)), // the file changed since
            ("large", Some(&now), Some("file:///elsewhere.png")), // another file's
            ("large", None, Some(&uri)),         // no time to check
            ("large", Some(&now), None),         // no URI to check
            ("normal", Some(&now), Some(&uri)),  // smaller than asked
        ];
        for (i, (folder, mtime, uri_text)) in cases.into_iter().enumerate() {
            let shared = dir.path().join(format!("thumbnails-{i}"));
            let shared_file = write_shared(&shared, folder, &uri, mtime, uri_text);
            let cache = dir.path().join(format!("cache-{i}"));
            let got = thumbnail_unless(&src, 256, &cache, Some(&shared), &|| false)
                .unwrap()
                .expect("decoded instead");
            assert_ne!(got, shared_file, "case {i}");
            assert!(got.starts_with(&cache), "case {i}");
        }
    }

    #[test]
    fn a_desktop_thumbnail_is_found_for_a_file_the_app_cannot_decode() {
        // A video: the app has no decoder for it, the desktop made one.
        let dir = tempdir().unwrap();
        let src = dir.path().join("clip.mp4");
        fs::write(&src, b"not a picture").unwrap();
        let (shared, cache) = (dir.path().join("thumbnails"), dir.path().join("cache"));
        if cfg!(not(target_os = "macos")) {
            let none = thumbnail_unless(&src, 256, &cache, Some(&shared), &|| false).unwrap();
            assert_eq!(none, None);
        }
        // Made later (GNOME Files opened the folder): used despite the marker.
        let uri = crate::platform::path_to_file_uri(&src);
        let made = write_shared(&shared, "x-large", &uri, Some(&mtime_secs(&src)), Some(&uri));
        let found = thumbnail_unless(&src, 256, &cache, Some(&shared), &|| false).unwrap();
        assert_eq!(found, Some(made));
    }

    #[test]
    fn a_desktop_thumbnail_time_matches_to_the_second_or_microsecond() {
        let t = Duration::new(1_790_758_491, 500_000);
        assert!(same_mtime("1790758491", t), "whole seconds, as GNOME writes it");
        assert!(same_mtime(" 1790758491 ", t));
        assert!(same_mtime("1790758491.0005", t));
        assert!(same_mtime("1790758491.000500999", t));
        assert!(!same_mtime("1790758491.5", t));
        assert!(!same_mtime("1790758490", t));
        assert!(!same_mtime("soon", t));
        assert!(!same_mtime("", t));
    }

    #[test]
    fn the_desktop_cache_is_never_written() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("photo.png");
        write_png(&src, 300, 200);
        let shared = dir.path().join("thumbnails");
        let other = "file:///other.png";
        write_shared(&shared, "large", other, Some("1"), Some(other));
        let before = snapshot(&shared);
        std::thread::sleep(Duration::from_millis(20));
        let cache = dir.path().join("cache");
        let made = thumbnail_unless(&src, 256, &cache, Some(&shared), &|| false).unwrap().unwrap();
        assert!(made.starts_with(&cache));
        assert_eq!(snapshot(&shared), before);
        // A missing shared cache is not created either.
        let absent = dir.path().join("no-thumbnails");
        thumbnail_unless(&src, 128, &cache, Some(&absent), &|| false).unwrap();
        assert!(!absent.exists());
    }

    #[test]
    fn a_decoded_photo_is_kept_as_a_small_jpeg() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("photo.png");
        write_png(&src, 800, 400);
        let thumb = thumbnail(&src, 256, &dir.path().join("cache")).unwrap().unwrap();
        assert_eq!(thumb.extension().unwrap(), "jpg");
        let bytes = fs::read(&thumb).unwrap();
        assert_eq!(image::guess_format(&bytes).unwrap(), image::ImageFormat::Jpeg);
        assert_eq!(image::image_dimensions(&thumb).unwrap(), (256, 128));
        // An opaque picture with an alpha channel (a screenshot) is a JPEG too.
        let shot = dir.path().join("shot.png");
        image::RgbaImage::from_pixel(300, 200, image::Rgba([10, 20, 30, 255])).save(&shot).unwrap();
        let thumb = thumbnail(&shot, 128, &dir.path().join("cache")).unwrap().unwrap();
        assert_eq!(thumb.extension().unwrap(), "jpg");
    }

    #[test]
    fn a_picture_with_transparency_stays_png() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("logo.png");
        let alpha = |x: u32| if x < 150 { 0 } else { 255 };
        image::RgbaImage::from_fn(300, 200, |x, _| image::Rgba([200, 0, 0, alpha(x)]))
            .save(&src)
            .unwrap();
        let thumb = thumbnail(&src, 128, &dir.path().join("cache")).unwrap().unwrap();
        assert_eq!(thumb.extension().unwrap(), "png");
        assert!(image::open(&thumb).unwrap().color().has_alpha());
    }

    #[test]
    fn a_png_thumbnail_cached_before_is_still_used() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("photo.png");
        write_png(&src, 300, 200);
        let cache = dir.path().join("cache");
        fs::create_dir(&cache).unwrap();
        let key = cache_key(&src, &fs::metadata(&src).unwrap(), 128);
        let old = cache.join(format!("{key}.png"));
        write_png(&old, 128, 85);
        assert_eq!(thumbnail(&src, 128, &cache).unwrap(), Some(old));
        assert_eq!(snapshot(&cache).len(), 1, "not made again");
    }

    #[test]
    fn pruning_keeps_the_cache_under_its_size_oldest_first() {
        let dir = tempdir().unwrap();
        let now = SystemTime::now();
        let names = ["oldest.jpg", "older.png", "newer.jpg", "newest.none"];
        for (i, name) in names.iter().enumerate() {
            let path = dir.path().join(name);
            let len = if name.ends_with("none") { 0 } else { 100 };
            fs::write(&path, vec![0u8; len]).unwrap();
            let age = Duration::from_secs(3600 * (4 - i as u64));
            let file = fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(now - age).unwrap();
        }
        prune(dir.path(), Duration::from_secs(86_400), 250);
        let left: Vec<String> = snapshot(dir.path())
            .into_iter()
            .map(|(p, _, _)| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, ["newer.jpg", "newest.none", "older.png"]);
        prune(dir.path(), Duration::from_secs(86_400), 100);
        assert!(!dir.path().join("older.png").exists());
        assert!(dir.path().join("newer.jpg").exists());
    }
}
