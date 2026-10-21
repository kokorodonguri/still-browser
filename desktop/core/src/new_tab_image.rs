//! The picture shown behind a new tab. The shell copies the chosen file into
//! `<data>/NewTab/` under a name it generates, and settings keep only that name,
//! so the interface can never point the shell at another file.

use std::fs;
use std::io;
use std::path::Path;

/// Larger files are refused before anything is written.
pub const MAX_BYTES: usize = 20 * 1024 * 1024;
/// Picture formats accepted, by the extension the shell gives them.
pub const EXTENSIONS: [&str; 7] = ["png", "jpg", "jpeg", "webp", "gif", "avif", "bmp"];

/// True for the names the shell gives stored pictures (`newtab-<digits>.<ext>`).
/// Anything else, including a path, is not a picture the shell stored.
pub fn is_stored_name(name: &str) -> bool {
    let Some((stem, ext)) = name.rsplit_once('.') else { return false };
    let Some(digits) = stem.strip_prefix("newtab-") else { return false };
    !digits.is_empty() && digits.len() <= 20 && digits.bytes().all(|b| b.is_ascii_digit()) && EXTENSIONS.contains(&ext)
}

/// The format of a picture from its first bytes; the file name is not trusted.
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    let starts = |magic: &[u8]| bytes.starts_with(magic);
    if starts(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if starts(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if starts(b"GIF87a") || starts(b"GIF89a") {
        Some("gif")
    } else if starts(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("webp")
    } else if matches!(bytes.get(4..12), Some(b"ftypavif") | Some(b"ftypavis")) {
        Some("avif")
    } else if starts(b"BM") && bytes.len() > 26 {
        Some("bmp")
    } else {
        None
    }
}

#[derive(Debug)]
pub enum StoreError {
    /// Not a supported picture, or too large.
    Unsupported,
    Io(io::Error),
}

/// Writes `bytes` into `dir` under a fresh stored name and returns that name.
/// The file is complete before the name is returned; nothing else is touched.
pub fn store(dir: &Path, bytes: &[u8], now_ms: u128) -> Result<String, StoreError> {
    if bytes.len() > MAX_BYTES {
        return Err(StoreError::Unsupported);
    }
    let ext = sniff(bytes).ok_or(StoreError::Unsupported)?;
    fs::create_dir_all(dir).map_err(StoreError::Io)?;
    // Two pictures chosen within the same millisecond still get distinct names.
    let mut stamp = now_ms;
    let name = loop {
        let name = format!("newtab-{stamp}.{ext}");
        if !dir.join(&name).exists() {
            break name;
        }
        stamp += 1;
    };
    crate::save_atomic_with(&dir.join(&name), |out| out.write_all(bytes)).map_err(StoreError::Io)?;
    Ok(name)
}

/// Deletes stored pictures other than `keep` (empty keeps none). Files the
/// shell did not name are left alone. Returns how many were removed.
pub fn remove_others(dir: &Path, keep: &str) -> usize {
    let Ok(entries) = fs::read_dir(dir) else { return 0 };
    entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_str().is_some_and(|name| is_stored_name(name) && name != keep))
        .filter(|entry| fs::remove_file(entry.path()).is_ok())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("still-newtab-{}-{}", std::process::id(), SEQUENCE.fetch_add(1, Ordering::Relaxed)));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }

    #[test]
    fn stored_names_are_only_the_ones_the_shell_generates() {
        assert!(is_stored_name("newtab-1760400000000.jpg"));
        for bad in ["C:\\Users\\me\\a.jpg", "../state.json", "newtab-1.exe", "newtab-.png", "newtab-1a.png", "other-1.png", "newtab-1.png/x", "newtab-1"] {
            assert!(!is_stored_name(bad), "{bad}");
        }
    }

    #[test]
    fn formats_come_from_the_contents() {
        let mut webp = b"RIFF\0\0\0\0WEBPVP8 ".to_vec();
        assert_eq!(sniff(PNG), Some("png"));
        assert_eq!(sniff(&[0xff, 0xd8, 0xff, 0xe0]), Some("jpg"));
        assert_eq!(sniff(b"GIF89a..."), Some("gif"));
        assert_eq!(sniff(&webp), Some("webp"));
        assert_eq!(sniff(b"\0\0\0\x1cftypavif\0\0"), Some("avif"));
        assert_eq!(sniff(&[b"BM".as_slice(), &[0; 40]].concat()), Some("bmp"));
        webp[8..12].copy_from_slice(b"WAVE");
        for other in [&webp[..], b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>", b"MZ\x90\0", b"BM", b""] {
            assert_eq!(sniff(other), None);
        }
    }

    #[test]
    fn store_writes_a_new_file_and_refuses_other_files() {
        let dir = Directory::new();
        let first = store(&dir.0, PNG, 1000).unwrap();
        assert_eq!(first, "newtab-1000.png");
        assert_eq!(fs::read(dir.0.join(&first)).unwrap(), PNG);
        assert_eq!(store(&dir.0, PNG, 1000).unwrap(), "newtab-1001.png", "same millisecond, new name");
        assert!(matches!(store(&dir.0, b"#!/bin/sh", 2000), Err(StoreError::Unsupported)));
        let mut huge = PNG.to_vec();
        huge.resize(MAX_BYTES + 1, 0);
        assert!(matches!(store(&dir.0, &huge, 3000), Err(StoreError::Unsupported)));
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2, "refused files leave nothing behind");
    }

    #[test]
    fn remove_others_keeps_the_current_picture_and_unrelated_files() {
        let dir = Directory::new();
        for name in ["newtab-1.png", "newtab-2.jpg", "notes.txt"] {
            fs::write(dir.0.join(name), "x").unwrap();
        }
        assert_eq!(remove_others(&dir.0, "newtab-2.jpg"), 1);
        let mut left: Vec<_> = fs::read_dir(&dir.0).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left, ["newtab-2.jpg", "notes.txt"]);
        assert_eq!(remove_others(&dir.0, ""), 1);
        assert_eq!(remove_others(&dir.0.join("missing"), ""), 0);
    }
}
