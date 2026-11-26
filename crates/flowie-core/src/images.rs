//! Image files. Each bullet holds at most one image (`BlockKind::Image`,
//! the text is its caption). Files live beside the database in
//! `<database>.images/<ab>/<sha256>.<ext>`, named by their content, so each is written
//! once (temp file, fsync, rename) before any bullet refers to it, backups
//! copy it once, and identical images are stored once. The database keeps
//! each file's type and size (`blobs`), so a row's height is known without
//! decoding the image.
//!
//! The folder itself (`ImageDir`) needs a file system and comes with the `store` feature;
//! naming and sniffing images don't.

#[cfg(feature = "store")]
use std::collections::HashSet;
#[cfg(feature = "store")]
use std::io::Write;
#[cfg(feature = "store")]
use std::path::{Path, PathBuf};
#[cfg(feature = "store")]
use std::sync::Mutex;
#[cfg(feature = "store")]
use std::time::SystemTime;

use sha2::{Digest, Sha256};

/// Coordinates in-process `put` and `collect` metadata/unlink steps so launch
/// garbage collection can never delete an image file that is being reused or
/// written concurrently.
#[cfg(feature = "store")]
static FILE_OP_LOCK: Mutex<()> = Mutex::new(());

/// Largest image accepted.
pub const MAX_BYTES: usize = 50 << 20;

/// What the database knows about one image file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobInfo {
    /// `<sha256>.<ext>`.
    pub name: String,
    pub mime: String,
    pub width: u32,
    pub height: u32,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageError {
    /// Not PNG, JPEG, GIF or WebP.
    Unsupported,
    TooLarge,
    Io(String),
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageError::Unsupported => write!(f, "not a PNG, JPEG, GIF or WebP image"),
            ImageError::TooLarge => write!(f, "image larger than {} MB", MAX_BYTES >> 20),
            ImageError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ImageError {}

/// The file types Flowie keeps: (extension, MIME type).
const TYPES: [(&str, &str); 4] = [("png", "image/png"), ("jpg", "image/jpeg"), ("gif", "image/gif"), ("webp", "image/webp")];

/// Whether `name` is an image file name as stored: 64 hex digits and a known extension.
pub fn is_name(name: &str) -> bool {
    let Some((hash, ext)) = name.split_once('.') else { return false };
    hash.len() == 64 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) && TYPES.iter().any(|(e, _)| *e == ext)
}

/// The MIME type of a stored name's extension.
pub fn mime_of(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("");
    TYPES.iter().find(|(e, _)| *e == ext).map_or("application/octet-stream", |(_, m)| m)
}

fn be16(b: &[u8], at: usize) -> Option<u32> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?) as u32)
}

fn le16(b: &[u8], at: usize) -> Option<u32> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?) as u32)
}

fn le24(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at + 3)?;
    Some(s[0] as u32 | (s[1] as u32) << 8 | (s[2] as u32) << 16)
}

/// The type and pixel size of an image, read from its header.
pub fn sniff(b: &[u8]) -> Option<(&'static str, &'static str, u32, u32)> {
    let (ext, w, h) = if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        ("png", u32::from_be_bytes(b.get(16..20)?.try_into().ok()?), u32::from_be_bytes(b.get(20..24)?.try_into().ok()?))
    } else if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        ("gif", le16(b, 6)?, le16(b, 8)?)
    } else if b.starts_with(b"\xff\xd8") {
        let (w, h) = jpeg_size(b)?;
        ("jpg", w, h)
    } else if b.get(0..4) == Some(b"RIFF") && b.get(8..12) == Some(b"WEBP") {
        let (w, h) = match b.get(12..16)? {
            b"VP8 " => (le16(b, 26)? & 0x3fff, le16(b, 28)? & 0x3fff),
            b"VP8L" => {
                let s = b.get(21..25)?;
                (1 + (s[0] as u32 | (s[1] as u32 & 0x3f) << 8), 1 + (s[1] as u32 >> 6 | (s[2] as u32) << 2 | (s[3] as u32 & 0xf) << 10))
            }
            b"VP8X" => (1 + le24(b, 24)?, 1 + le24(b, 27)?),
            _ => return None,
        };
        ("webp", w, h)
    } else {
        return None;
    };
    let mime = TYPES.iter().find(|(e, _)| *e == ext)?.1;
    (w > 0 && h > 0).then_some((ext, mime, w, h))
}

/// A JPEG's size from its first start-of-frame segment.
fn jpeg_size(b: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    loop {
        while *b.get(at)? != 0xff {
            at += 1;
        }
        while *b.get(at)? == 0xff {
            at += 1;
        }
        let marker = *b.get(at)?;
        at += 1;
        if matches!(marker, 0xd0..=0xd9 | 0x01) {
            continue;
        }
        let len = be16(b, at)? as usize;
        if matches!(marker, 0xc0..=0xcf) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
            return Some((be16(b, at + 5)?, be16(b, at + 3)?));
        }
        at += len.max(2);
    }
}

/// What storing `bytes` would record, without writing anything.
pub fn info(bytes: &[u8]) -> Result<BlobInfo, ImageError> {
    if bytes.len() > MAX_BYTES {
        return Err(ImageError::TooLarge);
    }
    let (ext, mime, width, height) = sniff(bytes).ok_or(ImageError::Unsupported)?;
    let hash: String = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
    Ok(BlobInfo { name: format!("{hash}.{ext}"), mime: mime.into(), width, height, size: bytes.len() as u64 })
}

/// The folder of image files beside a database.
#[cfg(feature = "store")]
#[derive(Debug, Clone)]
pub struct ImageDir {
    root: PathBuf,
}

#[cfg(feature = "store")]
impl ImageDir {
    pub fn new(root: PathBuf) -> ImageDir {
        ImageDir { root }
    }

    /// A private image folder for this database, including its full filename.
    pub fn beside(db: &Path) -> ImageDir {
        let mut path = db.as_os_str().to_owned();
        path.push(".images");
        ImageDir::new(path.into())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Where the file `name` lives (`None` for a name that isn't one).
    pub fn path(&self, name: &str) -> Option<PathBuf> {
        is_name(name).then(|| self.root.join(&name[..2]).join(name))
    }

    pub fn has(&self, name: &str) -> bool {
        self.path(name).is_some_and(|p| p.is_file())
    }

    pub fn read(&self, name: &str) -> std::io::Result<Vec<u8>> {
        let path = self.path(name).ok_or_else(|| std::io::Error::other("not an image name"))?;
        std::fs::read(path)
    }

    /// Store an image (once: the same bytes give the same file).
    /// Reusing an existing file refreshes its modification time so concurrent
    /// garbage collection does not treat it as an old unused file.
    pub fn put(&self, bytes: &[u8]) -> Result<BlobInfo, ImageError> {
        let info = info(bytes)?;
        let path = self.path(&info.name).expect("a stored name");
        let io = |e: std::io::Error| ImageError::Io(e.to_string());
        let dir = path.parent().expect("inside the image folder");
        let (tmp, mut file) = {
            let _guard = FILE_OP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            if let Ok(existing) = std::fs::OpenOptions::new().write(true).open(&path) {
                if existing.set_modified(SystemTime::now()).is_ok() {
                    return Ok(info);
                }
            }
            std::fs::create_dir_all(dir).map_err(io)?;
            loop {
                let tmp = dir.join(format!(".{}.tmp-{}", info.name, uuid::Uuid::new_v4().simple()));
                match std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp) {
                    Ok(file) => break (tmp, file),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => return Err(io(error)),
                }
            }
        };
        let written = (|| {
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            let _guard = FILE_OP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            match std::fs::rename(&tmp, &path) {
                Ok(()) => Ok(()),
                Err(error) => std::fs::OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .and_then(|existing| existing.set_modified(SystemTime::now()))
                    .map_err(|_| error),
            }
        })();
        let _ = std::fs::remove_file(&tmp);
        written.map_err(io)?;
        Ok(info)
    }

    /// Delete image files not in `keep` and older than `min_age` (younger
    /// ones may have just been added); returns the names removed.
    pub fn collect(&self, keep: &HashSet<String>, min_age: std::time::Duration) -> Vec<String> {
        let mut removed = Vec::new();
        let Ok(dirs) = std::fs::read_dir(&self.root) else { return removed };
        for dir in dirs.flatten() {
            if !dir.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            let Ok(files) = std::fs::read_dir(dir.path()) else { continue };
            for f in files.flatten() {
                let name = f.file_name().to_string_lossy().into_owned();
                let path = f.path();
                let _guard = FILE_OP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
                // Leftover temp files too, but only old ones: one may be in the middle of a write.
                let age =
                    std::fs::symlink_metadata(&path).and_then(|metadata| metadata.modified()).ok().and_then(|time| time.elapsed().ok());
                let Some(age) = age else { continue };
                let stale_tmp = name.starts_with('.') && age.as_secs() > 3600;
                let unused = is_name(&name) && !keep.contains(&name) && age >= min_age;
                if (unused || stale_tmp) && std::fs::remove_file(&path).is_ok() && unused {
                    removed.push(name);
                }
            }
            let _guard = FILE_OP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let _ = std::fs::remove_dir(dir.path()); // only if now empty
        }
        removed
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A 3×2 PNG.
    pub fn png() -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        b.extend(3u32.to_be_bytes());
        b.extend(2u32.to_be_bytes());
        b.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
        b
    }

    #[test]
    fn sniffs_sizes() {
        assert_eq!(sniff(&png()), Some(("png", "image/png", 3, 2)));
        let gif = b"GIF89a\x05\x00\x07\x00rest";
        assert_eq!(sniff(gif), Some(("gif", "image/gif", 5, 7)));
        // SOI, an APP0 segment, then SOF0 with height 10, width 20.
        let jpeg = [0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0, 0xff, 0xc0, 0, 11, 8, 0, 10, 0, 20, 3, 0, 0, 0];
        assert_eq!(sniff(&jpeg), Some(("jpg", "image/jpeg", 20, 10)));
        let mut webp = b"RIFF\0\0\0\0WEBPVP8X\0\0\0\0\0\0\0\0".to_vec();
        webp.extend([99, 0, 0, 49, 0, 0]);
        assert_eq!(sniff(&webp), Some(("webp", "image/webp", 100, 50)));
        assert_eq!(sniff(b"hello"), None);
        assert_eq!(info(b"<svg/>"), Err(ImageError::Unsupported));
    }

    #[cfg(feature = "store")]
    #[test]
    fn collecting_one_database_leaves_its_neighbours_images_alone() {
        let dir = std::env::temp_dir().join(format!("flowie-images-{}", crate::NodeId::random()));
        let first = ImageDir::beside(&dir.join("notes.sqlite"));
        let second = ImageDir::beside(&dir.join("notes.db"));
        let image = first.put(&png()).unwrap();
        second.put(&png()).unwrap();
        assert_eq!(first.collect(&HashSet::new(), std::time::Duration::ZERO), std::slice::from_ref(&image.name));
        assert_eq!(second.read(&image.name).unwrap(), png());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(feature = "store")]
    #[test]
    fn stores_once_and_collects_unused() {
        let dir = std::env::temp_dir().join(format!("flowie-images-{}", crate::tree::NodeId::random()));
        let images = ImageDir::new(dir.clone());
        let a = images.put(&png()).unwrap();
        assert!(is_name(&a.name) && a.name.ends_with(".png"));
        assert_eq!((a.width, a.height, a.size), (3, 2, png().len() as u64));
        assert_eq!(images.put(&png()).unwrap(), a);
        assert_eq!(images.read(&a.name).unwrap(), png());
        assert_eq!(images.path("../etc/passwd"), None);
        let mut other = png();
        other.push(0);
        let b = images.put(&other).unwrap();
        assert_eq!(images.collect(&[a.name.clone()].into_iter().collect(), std::time::Duration::ZERO), std::slice::from_ref(&b.name));
        assert!(images.has(&a.name) && !images.has(&b.name));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(feature = "store")]
    #[test]
    fn reusing_an_old_image_refreshes_mtime_so_collection_keeps_it() {
        let dir = std::env::temp_dir().join(format!("flowie-images-reuse-{}", crate::tree::NodeId::random()));
        let images = ImageDir::new(dir.clone());
        let a = images.put(&png()).unwrap();
        let path = images.path(&a.name).unwrap();
        // Backdate the file so an unused-image sweep would normally delete it.
        let old = SystemTime::now() - std::time::Duration::from_secs(7200);
        std::fs::OpenOptions::new().write(true).open(&path).unwrap().set_modified(old).unwrap();

        // Re-pasting the same image reuses the file and refreshes its mtime.
        assert_eq!(images.put(&png()).unwrap(), a);
        assert!(images.collect(&HashSet::new(), std::time::Duration::from_secs(3600)).is_empty());
        assert!(images.has(&a.name));
        assert_eq!(images.read(&a.name).unwrap(), png());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
