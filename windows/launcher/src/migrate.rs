//! Hands the earlier Windows build's bookmarks, history and new tab picture to
//! Firefox. That build kept them in `%LOCALAPPDATA%\StillBrowser\state.json`
//! (format: `desktop/core/src/lib.rs`, `BrowserState`). The launcher writes them
//! to `still-import.json` in the new profile, and `windows/firefox/still.cfg`
//! adds them to Firefox's library once, after the first window opens.
//! Saved passwords and sign-ins do not carry over.

use crate::newtab;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub const IMPORT_FILE: &str = "still-import.json";
/// What still.cfg renames the import file to once it is done, so a profile is
/// never given the same library twice.
pub const DONE_FILE: &str = "still-import.done";

fn web(url: &str) -> bool {
    (url.starts_with("https://") || url.starts_with("http://")) && url.len() <= 32_000
}

/// The import file's contents, or None when there is nothing to import.
pub fn library(state: &Value) -> Option<Value> {
    let text = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
    let list = |key: &str| state.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
    let mut titles: HashMap<String, String> = HashMap::new();
    let mut history = Vec::new();
    for entry in list("History") {
        let url = text(&entry, "Url");
        if !web(&url) {
            continue;
        }
        let title = text(&entry, "Title");
        if !title.is_empty() {
            titles.insert(url.clone(), title.clone());
        }
        let Some(seconds) = parse_iso8601(&text(&entry, "VisitedAt")) else { continue };
        history.push(json!({ "url": url, "title": title, "visited": seconds * 1000 }));
    }
    let imported = state.get("BookmarkTitles").and_then(Value::as_object).cloned().unwrap_or_else(Map::new);
    // The earlier build showed a bookmark with the newest title from history,
    // else the one imported with it.
    let bookmarks: Vec<Value> = list("Bookmarks")
        .iter()
        .filter_map(Value::as_str)
        .filter(|url| web(url))
        .map(|url| {
            let title = titles.get(url).cloned().or_else(|| imported.get(url).and_then(Value::as_str).map(str::to_owned));
            json!({ "url": url, "title": title.unwrap_or_default() })
        })
        .collect();
    (!bookmarks.is_empty() || !history.is_empty()).then(|| json!({ "bookmarks": bookmarks, "history": history }))
}

/// Stages what the earlier build saved in `legacy` for the new `profile`.
/// Anything unreadable is skipped: the new profile starts regardless.
pub fn stage(legacy: &Path, profile: &Path) {
    if profile.join(IMPORT_FILE).exists() || profile.join(DONE_FILE).exists() {
        return;
    }
    let Ok(text) = fs::read_to_string(legacy.join("state.json")) else { return };
    let Ok(state) = serde_json::from_str::<Value>(&text) else { return };
    if let Some(library) = library(&state) {
        let _ = crate::profile::write_atomic(&profile.join(IMPORT_FILE), library.to_string().as_bytes());
    }
    let picture = state.pointer("/Settings/NewTabImage").and_then(Value::as_str).unwrap_or_default();
    // Only a bare file name, as the earlier build stored it.
    if !picture.is_empty() && !picture.contains(['/', '\\', ':']) && newtab::current(profile).is_none() {
        if let Ok(data) = newtab::read(&legacy.join("NewTab").join(picture)) {
            let _ = newtab::store(profile, &data);
        }
    }
}

/// Unix seconds of an ISO 8601 time with an offset, as the earlier build saved
/// visits (`2026-10-09T05:00:00+09:00`, fractions and `Z` allowed). Same rules
/// as `parse_iso8601` in `desktop/core/src/lib.rs`.
pub fn parse_iso8601(text: &str) -> Option<i64> {
    let text = text.trim();
    let num = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hour, minute, second) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if text.get(4..5)? != "-" || text.get(7..8)? != "-" || !matches!(text.get(10..11)?, "T" | "t" | " ") || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let rest = text[19..].trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let offset = match rest {
        "Z" | "z" | "" => 0,
        _ => {
            let sign = match rest.get(..1)? {
                "+" => 1,
                "-" => -1,
                _ => return None,
            };
            let hours = rest.get(1..3)?.parse::<i64>().ok()?;
            let minutes = rest.get(4..6).or_else(|| rest.get(3..5)).and_then(|m| m.parse::<i64>().ok()).unwrap_or(0);
            sign * (hours * 3600 + minutes * 60)
        }
    };
    // Days-from-civil (Howard Hinnant).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carries_bookmarks_and_visits_over() {
        let state = json!({
            "Bookmarks": ["https://example.org/", "file:///C:/secret.txt", "https://imported.test/"],
            "BookmarkTitles": { "https://imported.test/": "Imported" },
            "History": [
                { "Url": "https://example.org/", "VisitedAt": "2026-10-09T05:00:00.1234567+09:00", "Title": "Example" },
                { "Url": "about:blank", "VisitedAt": "2026-10-09T05:00:00Z", "Title": "" },
                { "Url": "https://broken.test/", "VisitedAt": "yesterday", "Title": "Broken" },
                null
            ],
            "Settings": { "NewTabImage": "" }
        });
        let library = library(&state).unwrap();
        assert_eq!(library["bookmarks"], json!([
            { "url": "https://example.org/", "title": "Example" },
            { "url": "https://imported.test/", "title": "Imported" }
        ]));
        assert_eq!(library["history"], json!([
            { "url": "https://example.org/", "title": "Example", "visited": 1_791_489_600_000_i64 }
        ]));
        assert_eq!(super::library(&json!({ "Bookmarks": null, "History": [] })), None);
    }

    #[test]
    fn stages_the_import_file_and_picture() {
        let dir = std::env::temp_dir().join(format!("still-migrate-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let (legacy, profile) = (dir.join("StillBrowser"), dir.join("StillBrowser/Profile"));
        fs::create_dir_all(legacy.join("NewTab")).unwrap();
        fs::write(legacy.join("NewTab/newtab-5.png"), b"\x89PNG\r\n\x1a\nrest").unwrap();
        fs::write(legacy.join("state.json"), r#"{"Bookmarks":["https://example.org/"],"Settings":{"NewTabImage":"newtab-5.png"}}"#).unwrap();
        crate::profile::prepare(&profile, &legacy).unwrap();
        let staged: Value = serde_json::from_str(&fs::read_to_string(profile.join(IMPORT_FILE)).unwrap()).unwrap();
        assert_eq!(staged["bookmarks"][0]["url"], "https://example.org/");
        assert!(newtab::current(&profile).unwrap().ends_with(".png"));
        assert!(fs::read_to_string(profile.join("chrome/userContent.css")).unwrap().contains("newtab/newtab-"));
        // Once imported, never again, even before Firefox has saved prefs.js.
        fs::rename(profile.join(IMPORT_FILE), profile.join(DONE_FILE)).unwrap();
        crate::profile::prepare(&profile, &legacy).unwrap();
        assert!(!profile.join(IMPORT_FILE).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_or_broken_state_is_skipped() {
        let dir = std::env::temp_dir().join(format!("still-migrate-broken-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("state.json"), "{not json").unwrap();
        stage(&dir, &dir.join("Profile"));
        assert!(!dir.join("Profile").join(IMPORT_FILE).exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
