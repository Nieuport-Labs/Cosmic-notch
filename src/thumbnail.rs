// SPDX-License-Identifier: GPL-3.0-only

//! Square thumbnails for album art and shelf files, cached on disk.

use std::path::{Path, PathBuf};

use image::imageops::FilterType;

/// The island's cache directory, `~/.cache/cosmic-ext-island`.
pub fn cache_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .map(|c| c.join("cosmic-ext-island"))
}

/// FNV-1a, so cache file names stay the same across builds.
pub fn stable_hash(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Whether `path` looks like an image we can thumbnail.
pub fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| {
            ["png", "jpg", "jpeg", "webp", "gif", "bmp"]
                .contains(&e.to_ascii_lowercase().as_str())
        })
}

/// Center-crops `source` to a square of `size` pixels and caches it as PNG.
/// Blocking: run it off the UI thread.
pub fn square(source: &Path, size: u32) -> Option<PathBuf> {
    let modified = std::fs::metadata(source)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    let key = format!("{}:{modified}:{size}", source.display());
    let dir = cache_dir()?.join("thumbnails");
    let target = dir.join(format!("{:016x}.png", stable_hash(&key)));
    if target.exists() {
        return Some(target);
    }

    let image = image::ImageReader::open(source)
        .ok()?
        .with_guessed_format()
        .ok()?
        .decode()
        .inspect_err(|err| tracing::debug!("thumbnail {}: {err}", source.display()))
        .ok()?;
    let side = image.width().min(image.height());
    let cropped = image.crop_imm(
        (image.width() - side) / 2,
        (image.height() - side) / 2,
        side,
        side,
    );
    let thumbnail = cropped.resize_exact(size.min(side), size.min(side), FilterType::Triangle);
    std::fs::create_dir_all(&dir).ok()?;
    thumbnail.save(&target).ok()?;
    Some(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crops_wide_images_to_squares() {
        let dir = std::env::temp_dir().join(format!("island-thumb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("wide.png");
        image::RgbImage::new(320, 180).save(&source).unwrap();
        // Keep the test's cache out of the user's.
        unsafe { std::env::set_var("XDG_CACHE_HOME", &dir) };
        let thumb = square(&source, 64).unwrap();
        let out = image::open(thumb).unwrap();
        assert_eq!((out.width(), out.height()), (64, 64));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn recognizes_images() {
        assert!(is_image(Path::new("/a/Foto.JPG")));
        assert!(!is_image(Path::new("/a/faktura.pdf")));
    }
}
