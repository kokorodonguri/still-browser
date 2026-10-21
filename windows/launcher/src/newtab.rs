//! The picture shown behind a new tab (about:home and about:newtab), as on
//! Linux (`linux/newtab_image.py`). The chosen file is copied into
//! `<profile>/chrome/newtab/` under a name Still generates, and
//! userContent.css points at that copy. Firefox reads userContent.css only at
//! startup, so a change made while Still runs appears after a restart.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Larger files are refused before anything is written.
pub const MAX_BYTES: u64 = 20 * 1024 * 1024;
const FOLDER: &str = "newtab";
const EXTENSIONS: [&str; 6] = ["png", "jpg", "gif", "webp", "avif", "bmp"];

pub const UNSUPPORTED: &str = "その画像は使えません。PNG、JPEG、GIF、WebP、AVIF、BMP（20MBまで）のファイルを選んでください。";

/// The format from the first bytes; the file name is not trusted.
pub fn sniff(data: &[u8]) -> Option<&'static str> {
    let starts = |magic: &[u8]| data.starts_with(magic);
    if starts(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if starts(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if starts(b"GIF87a") || starts(b"GIF89a") {
        Some("gif")
    } else if starts(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        Some("webp")
    } else if matches!(data.get(4..12), Some(b"ftypavif") | Some(b"ftypavis")) {
        Some("avif")
    } else if starts(b"BM") && data.len() > 26 {
        Some("bmp")
    } else {
        None
    }
}

/// The stamp of a name Still gave a stored picture (`newtab-<digits>.<ext>`).
fn stamp(name: &str) -> Option<u128> {
    let (stem, ext) = name.rsplit_once('.')?;
    let digits = stem.strip_prefix("newtab-")?;
    if digits.is_empty() || digits.len() > 20 || !EXTENSIONS.contains(&ext) {
        return None;
    }
    digits.parse().ok()
}

fn folder(profile: &Path) -> PathBuf {
    profile.join("chrome").join(FOLDER)
}

/// The stored picture, newest first if an interrupted change left two.
pub fn current(profile: &Path) -> Option<String> {
    fs::read_dir(folder(profile))
        .ok()?
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter_map(|name| Some((stamp(&name)?, name)))
        .max()
        .map(|(_, name)| name)
}

#[derive(Debug)]
pub enum StoreError {
    /// Not a supported picture, or too large.
    Unsupported,
    Io(io::Error),
}

/// Copies the picture `data` into the profile and makes it the only stored one.
pub fn store(profile: &Path, data: &[u8]) -> Result<String, StoreError> {
    if data.len() as u64 > MAX_BYTES {
        return Err(StoreError::Unsupported);
    }
    let extension = sniff(data).ok_or(StoreError::Unsupported)?;
    let directory = folder(profile);
    fs::create_dir_all(&directory).map_err(StoreError::Io)?;
    // A fresh name each time, so Firefox never shows a cached earlier picture.
    let mut stamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
    while directory.join(format!("newtab-{stamp}.{extension}")).exists() {
        stamp += 1;
    }
    let name = format!("newtab-{stamp}.{extension}");
    crate::profile::write_atomic(&directory.join(&name), data).map_err(StoreError::Io)?;
    remove_others(profile, &name);
    Ok(name)
}

/// Reads a picture file the user chose, refusing anything too large first.
pub fn read(source: &Path) -> Result<Vec<u8>, StoreError> {
    let size = fs::metadata(source).map_err(StoreError::Io)?.len();
    if size > MAX_BYTES {
        return Err(StoreError::Unsupported);
    }
    fs::read(source).map_err(StoreError::Io)
}

/// Deletes stored pictures other than `keep`; files Still did not name are left alone.
pub fn remove_others(profile: &Path, keep: &str) {
    let Ok(entries) = fs::read_dir(folder(profile)) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if stamp(&name).is_some() && name != keep {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// CSS appended to userContent.css; empty without a picture.
///
/// The root element's background fills the whole page, so the picture goes
/// there and the body is made transparent. about:blank keeps the plain color.
pub fn stylesheet(profile: &Path) -> String {
    let Some(name) = current(profile) else { return String::new() };
    format!(
        r#"
/* New tab picture, written by the launcher (still.exe --newtab-image). */
@-moz-document url("about:home"), url("about:newtab"), url("chrome://still/content/newtab.html") {{
  :root {{
    background-image: url("{FOLDER}/{name}") !important;
    background-position: center !important;
    background-size: cover !important;
    background-repeat: no-repeat !important;
    background-attachment: fixed !important;
  }}
  body {{ background: transparent !important; }}
  @media (forced-colors: active) {{
    :root {{ background-image: none !important; }}
  }}
}}
@-moz-document url("chrome://still/content/newtab.html") {{
  /* The picture shows around a translucent panel that keeps labels legible. */
  main {{ background: color-mix(in srgb, var(--still-canvas) 82%, transparent) !important; border-radius: 16px; backdrop-filter: blur(20px); }}
  header, footer {{ background: color-mix(in srgb, var(--still-canvas) 82%, transparent) !important; padding-inline: 16px; border-radius: 0 0 12px 12px; }}
  footer {{ border-radius: 12px 12px 0 0; }}
  @media (prefers-reduced-transparency: reduce) {{
    main, header, footer {{ background: var(--still-canvas) !important; backdrop-filter: none; }}
  }}
}}
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_one_picture_and_points_the_stylesheet_at_it() {
        let profile = std::env::temp_dir().join(format!("still-newtab-{}", std::process::id()));
        let _ = fs::remove_dir_all(&profile);
        assert_eq!(stylesheet(&profile), "");
        let png = b"\x89PNG\r\n\x1a\nrest";
        let first = store(&profile, png).unwrap();
        let second = store(&profile, b"GIF89a....").unwrap();
        assert_ne!(first, second);
        assert!(second.ends_with(".gif"));
        assert_eq!(current(&profile).as_deref(), Some(second.as_str()));
        assert!(!folder(&profile).join(&first).exists());
        assert!(stylesheet(&profile).contains(&format!("newtab/{second}")));
        assert!(matches!(store(&profile, b"text"), Err(StoreError::Unsupported)));
        fs::write(folder(&profile).join("mine.png"), png).unwrap();
        remove_others(&profile, "");
        assert_eq!(current(&profile), None);
        assert!(folder(&profile).join("mine.png").exists(), "files Still did not name stay");
        let _ = fs::remove_dir_all(&profile);
    }

    #[test]
    fn only_generated_names_count() {
        assert_eq!(stamp("newtab-12.png"), Some(12));
        assert_eq!(stamp("newtab-.png"), None);
        assert_eq!(stamp("newtab-1.exe"), None);
        assert_eq!(stamp("../newtab-1.png"), None);
    }
}
