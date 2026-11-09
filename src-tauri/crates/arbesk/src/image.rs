//! A thumbnail from the bytes a remote place hands over, decoded in this
//! process. Decoding is serialized and oversized pictures are skipped rather
//! than allowed to balloon the browser's memory.

use std::io::{BufRead, Cursor, Seek};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

/// Decodes run one at a time: a listing's thumbnails must not decode in
/// parallel and hold several full-size images at once. (The app serializes
/// its own local thumbnails with a separate lock in `thumbnails.rs`.)
static DECODE_LOCK: Mutex<()> = Mutex::new(());

/// Decodes running or waiting for `DECODE_LOCK`.
static DECODES_WAITING: AtomicUsize = AtomicUsize::new(0);

/// The most pixels worth decoding for one thumbnail.
pub(crate) const MAX_PIXELS: u64 = 80_000_000;

fn one_decode_at_a_time<T>(decode: impl FnOnce() -> T) -> T {
    DECODES_WAITING.fetch_add(1, Ordering::SeqCst);
    let decoded = {
        let _guard: MutexGuard<'_, ()> = DECODE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        decode()
    };
    // Trim once the queue has drained, not after every decode, as the app's
    // own thumbnail decoder does.
    if DECODES_WAITING.fetch_sub(1, Ordering::SeqCst) == 1 {
        release_free_memory();
    }
    decoded
}

/// Give freed heap back to the OS after a decode.
///
/// glibc keeps released heap in per-arena pools instead of returning it.
/// `malloc_trim` hands those pages back; only glibc has it.
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

/// A thumbnail of the image in `bytes` as PNG, at most `size` pixels square
/// (a smaller image keeps its size); `None` when the bytes aren't an image
/// this crate decodes, or one too large to be worth it.
pub(crate) fn png_from_bytes(bytes: &[u8], size: u32, stop: &dyn Fn() -> bool) -> Option<Vec<u8>> {
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

/// The image `reader` holds, turned upright (EXIF orientation) and shrunk to
/// fit `size`.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(w: u32, h: u32, format: image::ImageFormat) -> Vec<u8> {
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(w, h, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
        }));
        let mut bytes = Vec::new();
        img.write_to(&mut Cursor::new(&mut bytes), format)
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
    }
}
