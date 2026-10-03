//! Image previews: loading file versions from git / the working tree and decoding them.

use std::path::Path;

use crate::git::{self, cmd};

/// Larger images are downscaled before upload (GPU texture limits, memory).
const MAX_TEXTURE_SIDE: u32 = 4096;
/// Files bigger than this are not decoded.
const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "tif", "tiff"];

pub fn is_image_path(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, ext)| IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// Where a version of a file comes from.
#[derive(Debug, Clone)]
pub enum Source {
    /// The file in the working tree.
    Working(String),
    /// The staged version (`git show :path`).
    Index(String),
    /// The version in a commit (`git show <rev>:path`).
    Commit(String, String),
}

impl Source {
    /// The file's bytes, or `None` if that version does not exist.
    fn read(&self, repo: &Path) -> Option<Vec<u8>> {
        let (path, bytes) = match self {
            Source::Working(path) => (path, std::fs::read(repo.join(path)).ok()?),
            Source::Index(path) => (path, cmd::run_bytes(repo, &["show", &format!(":{path}")]).ok()?),
            Source::Commit(rev, path) => (path, git::file_at(repo, rev, path).ok()?),
        };
        // Commits and the index hold LFS pointers; fetch the real file for display.
        Some(git::lfs::resolve(repo, path, bytes).0)
    }
}

pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub bytes: usize,
    pub format: String,
    /// Pixels waiting to be uploaded; taken when the texture is created.
    pub pixels: Option<egui::ColorImage>,
    pub texture: Option<egui::TextureHandle>,
}

impl DecodedImage {
    pub fn texture(&mut self, ctx: &egui::Context, name: &str) -> Option<&egui::TextureHandle> {
        if self.texture.is_none() {
            let pixels = self.pixels.take()?;
            // Small images (icons) are magnified; keep their pixels crisp.
            let options = if self.width.max(self.height) <= 128 {
                egui::TextureOptions::NEAREST
            } else {
                egui::TextureOptions::LINEAR
            };
            self.texture = Some(ctx.load_texture(name, pixels, options));
        }
        self.texture.as_ref()
    }
}

/// One side of a preview: missing (file absent in that version), decoded, or undecodable.
pub type Side = Option<Result<DecodedImage, String>>;

pub struct ImagePair {
    pub old: Side,
    pub new: Side,
}

pub fn decode(bytes: &[u8]) -> Result<DecodedImage, String> {
    if git::lfs::is_pointer(bytes) {
        let size = git::lfs::pointer_size(bytes).map(|s| format!(" ({})", crate::format::human_size(s))).unwrap_or_default();
        return Err(format!("Stored in Git LFS{size}; the object is not available locally and could not be fetched"));
    }
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(git::diff::too_large_notice(bytes.len()));
    }
    let format = image::guess_format(bytes).map(|f| format!("{f:?}").to_uppercase()).unwrap_or_default();
    let img = image::load_from_memory(bytes).map_err(|e| format!("Cannot decode image: {e}"))?;
    let (width, height) = (img.width(), img.height());
    let img = if width > MAX_TEXTURE_SIDE || height > MAX_TEXTURE_SIDE {
        img.thumbnail(MAX_TEXTURE_SIDE, MAX_TEXTURE_SIDE)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    let pixels = egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
    Ok(DecodedImage { width, height, bytes: bytes.len(), format, pixels: Some(pixels), texture: None })
}

pub fn load_pair(repo: &Path, old: Option<Source>, new: Option<Source>) -> ImagePair {
    let side = |s: Option<Source>| s.and_then(|s| s.read(repo)).map(|bytes| decode(&bytes));
    ImagePair { old: side(old), new: side(new) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_image_extensions() {
        assert!(is_image_path("assets/icon-256.png"));
        assert!(is_image_path("Photo.JPG"));
        assert!(!is_image_path("src/main.rs"));
        assert!(!is_image_path("png"));
    }

    #[test]
    fn decodes_png() {
        let img = decode(include_bytes!("../assets/icon-256.png")).unwrap();
        assert_eq!((img.width, img.height, img.format.as_str()), (256, 256, "PNG"));
        assert!(decode(b"not an image").is_err());
    }
}
