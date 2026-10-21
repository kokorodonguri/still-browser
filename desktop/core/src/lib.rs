//! Platform-independent browser logic: address normalization and the saved state.
//! The state format is shared with the earlier C# build, so existing data carries over.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

pub mod import;
pub mod new_tab_image;
pub mod passwords;
pub mod session;
pub mod settings;
pub mod persistence;
pub use persistence::{save_atomic, save_atomic_with};

pub use settings::{SearchEngine, Settings};

pub const BLANK: &str = "about:blank";
pub const HISTORY_LIMIT: usize = 500;
pub const DOWNLOAD_LIMIT: usize = 200;
/// Longest address accepted for navigation or as the home page.
pub const URL_LIMIT: usize = 32_000;
/// Version written by this build. Files without a version come from the C# build (1).
pub const SCHEMA_VERSION: u32 = 2;

/// Turns address-bar input into a URL: http(s) URLs pass through, bare hosts get
/// https://, and everything else becomes a Google search.
pub fn normalize(input: &str) -> String {
    normalize_with(input, SearchEngine::Google)
}

/// `normalize` with the search engine chosen in settings.
pub fn normalize_with(input: &str, engine: SearchEngine) -> String {
    let text = input.trim();
    if text.is_empty() {
        return BLANK.into();
    }
    if let Some(url) = address_url(text) {
        return url;
    }
    let query = percent_encoding::utf8_percent_encode(text, percent_encoding::NON_ALPHANUMERIC);
    engine.search_url(&query.to_string())
}

/// The web address `input` names (an http(s) URL or a bare host), if any.
pub fn address_url(input: &str) -> Option<String> {
    let text = input.trim();
    if let Ok(url) = Url::parse(text) {
        if matches!(url.scheme(), "http" | "https") {
            return Some(url.into());
        }
    }
    if !text.contains(char::is_whitespace) && text.contains('.') {
        if let Ok(url) = Url::parse(&format!("https://{text}")) {
            if url.host_str().is_some_and(|h| h.contains('.')) {
                return Some(url.into());
            }
        }
    }
    None
}

/// Short label for a tab whose page has not reported a title yet.
pub fn host_label(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_else(|| "New tab".into())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase", default)]
pub struct SavedTab {
    pub url: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase", default)]
pub struct HistoryEntry {
    pub url: String,
    /// ISO 8601, as written by .NET's DateTimeOffset.
    pub visited_at: String,
    pub title: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase", default)]
pub struct SavedDownload {
    pub path: String,
    pub state: String,
    pub bytes: i64,
    pub total: i64,
    /// The tab whose page started it; only known while this run lasts.
    #[serde(skip)]
    pub tab: Option<u64>,
}

/// Outer window position and inner size in physical pixels.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase", default)]
pub struct WindowPlacement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

/// Read straight from the file text: .NET writes null for empty lists, and
/// non-object list entries are skipped, without first expanding the whole
/// document into a `serde_json::Value` tree.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase", default)]
pub struct BrowserState {
    #[serde(deserialize_with = "lenient::or_default")]
    pub version: u32,
    #[serde(deserialize_with = "lenient::or_default")]
    pub sidebar_open: bool,
    #[serde(deserialize_with = "lenient::objects")]
    pub tabs: Vec<SavedTab>,
    #[serde(deserialize_with = "lenient::index")]
    pub active_index: usize,
    #[serde(deserialize_with = "lenient::or_default")]
    pub bookmarks: Vec<String>,
    /// Titles of bookmarks imported from another browser, used until a visit
    /// gives the page a title of its own. Absent in the C# build's files.
    #[serde(deserialize_with = "lenient::titles", skip_serializing_if = "BTreeMap::is_empty")]
    pub bookmark_titles: BTreeMap<String, String>,
    #[serde(deserialize_with = "lenient::objects")]
    pub history: Vec<HistoryEntry>,
    #[serde(deserialize_with = "lenient::objects")]
    pub downloads: Vec<SavedDownload>,
    pub window: Option<WindowPlacement>,
    #[serde(deserialize_with = "lenient::or_default")]
    pub settings: Settings,
}

/// Field readers that keep the old build's tolerance for null and stray values.
mod lenient {
    use serde::de::value::MapAccessDeserializer;
    use serde::de::{Deserializer, Error, IgnoredAny, MapAccess, SeqAccess, Visitor};
    use serde::Deserialize;
    use std::fmt;
    use std::marker::PhantomData;

    /// Text values only; anything else is dropped rather than failing the file.
    pub fn titles<'de, D: Deserializer<'de>>(d: D) -> Result<std::collections::BTreeMap<String, String>, D::Error> {
        let serde_json::Value::Object(map) = serde_json::Value::deserialize(d)? else { return Ok(Default::default()) };
        Ok(map.into_iter().filter_map(|(k, v)| v.as_str().map(|t| (k, t.to_owned()))).collect())
    }

    pub fn or_default<'de, D: Deserializer<'de>, T: Deserialize<'de> + Default>(d: D) -> Result<T, D::Error> {
        Option::<T>::deserialize(d).map(Option::unwrap_or_default)
    }

    /// Negative indexes become 0; null is 0; a fraction or text is an error.
    pub fn index<'de, D: Deserializer<'de>>(d: D) -> Result<usize, D::Error> {
        match Option::<serde_json::Number>::deserialize(d)? {
            None => Ok(0),
            Some(n) => match (n.as_i64(), n.as_u64()) {
                (Some(i), _) => Ok(usize::try_from(i.max(0)).unwrap_or(usize::MAX)),
                (None, Some(u)) => Ok(usize::try_from(u).unwrap_or(usize::MAX)),
                _ => Err(D::Error::custom("ActiveIndex is not an integer")),
            },
        }
    }

    /// A list of objects. Null is empty and entries that are not objects are
    /// skipped; an object with a mistyped field is still an error.
    pub fn objects<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Vec<T>, D::Error> {
        struct Items<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for Items<T> {
            type Value = Vec<T>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a list or null")
            }
            fn visit_unit<E>(self) -> Result<Vec<T>, E> {
                Ok(Vec::new())
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<T>, A::Error> {
                let mut items = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(1024));
                while let Some(Entry(item)) = seq.next_element::<Entry<T>>()? {
                    items.extend(item);
                }
                Ok(items)
            }
        }
        d.deserialize_any(Items(PhantomData))
    }

    struct Entry<T>(Option<T>);

    impl<'de, T: Deserialize<'de>> Deserialize<'de> for Entry<T> {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct One<T>(PhantomData<T>);
            impl<'de, T: Deserialize<'de>> Visitor<'de> for One<T> {
                type Value = Entry<T>;
                fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                    f.write_str("a list entry")
                }
                fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Entry<T>, A::Error> {
                    T::deserialize(MapAccessDeserializer::new(map)).map(|item| Entry(Some(item)))
                }
                fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Entry<T>, A::Error> {
                    while seq.next_element::<IgnoredAny>()?.is_some() {}
                    Ok(Entry(None))
                }
                fn visit_unit<E>(self) -> Result<Entry<T>, E> {
                    Ok(Entry(None))
                }
                fn visit_bool<E>(self, _: bool) -> Result<Entry<T>, E> {
                    Ok(Entry(None))
                }
                fn visit_i64<E>(self, _: i64) -> Result<Entry<T>, E> {
                    Ok(Entry(None))
                }
                fn visit_u64<E>(self, _: u64) -> Result<Entry<T>, E> {
                    Ok(Entry(None))
                }
                fn visit_f64<E>(self, _: f64) -> Result<Entry<T>, E> {
                    Ok(Entry(None))
                }
                fn visit_str<E>(self, _: &str) -> Result<Entry<T>, E> {
                    Ok(Entry(None))
                }
            }
            d.deserialize_any(One(PhantomData))
        }
    }
}

impl BrowserState {
    /// Parses saved JSON and drops entries the old build could also have rejected.
    pub fn parse(json: &str) -> serde_json::Result<Self> {
        let mut state: Self = serde_json::from_str(json)?;
        state.version = state.version.max(1);
        state.window = state.window.filter(|w| w.width >= 200 && w.height >= 150);
        let mut seen = std::collections::HashSet::new();
        let keep: Vec<bool> = state.bookmarks.iter().map(|b| !b.trim().is_empty() && seen.insert(b.as_str())).collect();
        let mut keep = keep.into_iter();
        state.bookmarks.retain(|_| keep.next().unwrap_or(false));
        let bookmarks: std::collections::HashSet<&str> = state.bookmarks.iter().map(String::as_str).collect();
        state.bookmark_titles.retain(|url, title| bookmarks.contains(url.as_str()) && !title.is_empty());
        state.history.retain(|h| !h.url.trim().is_empty());
        trim_history(&mut state.history);
        state.downloads.retain(|d| !d.path.trim().is_empty() && !d.path.contains('\0'));
        trim_downloads(&mut state.downloads);
        for d in &mut state.downloads {
            if d.state == "InProgress" {
                d.state = "Interrupted".into();
            }
        }
        state.tabs.retain(|t| !t.url.trim().is_empty());
        state.active_index = state.active_index.min(state.tabs.len().saturating_sub(1));
        Ok(state)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("state serializes")
    }
}

pub fn push_history(history: &mut Vec<HistoryEntry>, url: &str, title: &str) {
    history.push(HistoryEntry { url: url.into(), visited_at: now_iso8601(), title: title.into() });
    trim_history(history);
}

/// Keeps the newest entries; in-progress downloads are never dropped.
pub fn trim_downloads(downloads: &mut Vec<SavedDownload>) {
    let mut excess = downloads.len().saturating_sub(DOWNLOAD_LIMIT);
    downloads.retain(|d| {
        if excess > 0 && d.state != "InProgress" {
            excess -= 1;
            false
        } else {
            true
        }
    });
}

fn trim_history(history: &mut Vec<HistoryEntry>) {
    if history.len() > HISTORY_LIMIT {
        history.drain(..history.len() - HISTORY_LIMIT);
    }
}

/// `folder/name`, or `folder/stem (n).ext` with the first free `n`, so a
/// download never overwrites an existing file.
pub fn unique_path(folder: &Path, name: &str, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let candidate = folder.join(name);
    if !exists(&candidate) {
        return candidate;
    }
    let (stem, extension) = match name.rfind('.') {
        Some(dot) if dot > 0 => name.split_at(dot),
        _ => (name, ""),
    };
    (1..10_000)
        .map(|n| folder.join(format!("{stem} ({n}){extension}")))
        .find(|p| !exists(p))
        .unwrap_or(candidate)
}

pub fn now_iso8601() -> String {
    iso8601_utc(now_secs())
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Unix seconds of an ISO 8601 time with an offset, as saved in history
/// (`2026-10-09T05:00:00+09:00`, fractions and `Z` allowed).
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
            let sign = match rest.get(..1)? { "+" => 1, "-" => -1, _ => return None };
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

pub fn iso8601_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil-from-days (Howard Hinnant), valid for the Unix era.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}+00:00", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_addresses_like_the_previous_build() {
        assert_eq!(normalize("  "), BLANK);
        assert_eq!(normalize("https://example.org"), "https://example.org/");
        assert_eq!(normalize("example.org/path"), "https://example.org/path");
        assert_eq!(normalize("still browser"), "https://www.google.com/search?q=still%20browser");
        assert_eq!(normalize("日本語"), "https://www.google.com/search?q=%E6%97%A5%E6%9C%AC%E8%AA%9E");
        assert_eq!(normalize("javascript:alert(1)"), "https://www.google.com/search?q=javascript%3Aalert%281%29");
        assert!(normalize("file:///etc/passwd").starts_with("https://www.google.com/"));
        assert_eq!(normalize_with("still browser", SearchEngine::DuckDuckGo), "https://duckduckgo.com/?q=still%20browser");
        assert_eq!(normalize_with("example.org", SearchEngine::Bing), "https://example.org/");
    }

    #[test]
    fn reads_state_written_by_the_csharp_build() {
        let json = r#"{
          "SidebarOpen": true,
          "Tabs": [{"Url":"https://example.org/"}, null, {"Url":"https://a.test/"}],
          "ActiveIndex": 7,
          "Bookmarks": ["https://example.org/", "", "https://example.org/"],
          "History": [{"Url":"https://example.org/","VisitedAt":"2026-10-09T05:00:00+09:00","Title":"Example"}],
          "Downloads": [{"Path":"C:\\Users\\a\\Downloads\\x.zip","State":"InProgress","Bytes":10,"Total":20}]
        }"#;
        let state = BrowserState::parse(json).unwrap();
        assert_eq!(state.version, 1, "files without a version come from the C# build");
        assert!(state.sidebar_open);
        assert_eq!(state.tabs.len(), 2);
        assert_eq!(state.active_index, 1);
        assert_eq!(state.bookmarks, vec!["https://example.org/"]);
        assert_eq!(state.history[0].title, "Example");
        assert_eq!(state.downloads[0].state, "Interrupted");
        let round = BrowserState::parse(&state.to_json()).unwrap();
        assert_eq!(round, state);
        assert!(state.to_json().contains("\"SidebarOpen\": true"));
    }

    #[test]
    fn tolerates_nulls_missing_fields_and_negative_index() {
        let state = BrowserState::parse(r#"{"Tabs":null,"ActiveIndex":-3,"History":null,"Window":{"Width":10}}"#).unwrap();
        assert_eq!(state, BrowserState { version: 1, ..BrowserState::default() });
        assert!(BrowserState::parse("not json").is_err());
    }

    #[test]
    fn list_entries_keep_the_old_leniency() {
        let state = BrowserState::parse(r#"{"Tabs":[1,"x",true,[{"Url":"https://in.list/"}],{"Url":"https://a.test/"}],"ActiveIndex":18446744073709551615}"#).unwrap();
        assert_eq!(state.tabs, vec![SavedTab { url: "https://a.test/".into() }], "non-objects are skipped");
        assert_eq!(state.active_index, 0, "an out-of-range index is clamped");
        assert!(BrowserState::parse(r#"{"Tabs":[{"Url":5}]}"#).is_err(), "a mistyped object is still corrupt");
        assert!(BrowserState::parse(r#"{"Tabs":"https://a.test/"}"#).is_err(), "a list that is not a list is corrupt");
        assert!(BrowserState::parse(r#"{"ActiveIndex":1.5}"#).is_err());
        assert!(BrowserState::parse(r#"{"Bookmarks":[1]}"#).is_err(), "bookmarks must be text, as before");
        let state = BrowserState::parse(r#"{"Version":null,"SidebarOpen":null,"Bookmarks":null,"Settings":null,"Window":null,"Extra":{"a":[1,2]}}"#).unwrap();
        assert_eq!(state, BrowserState { version: 1, ..BrowserState::default() });
    }

    #[test]
    fn history_keeps_the_latest_500() {
        let mut history = Vec::new();
        for i in 0..520 {
            push_history(&mut history, &format!("https://e.test/{i}"), "");
        }
        assert_eq!(history.len(), HISTORY_LIMIT);
        assert_eq!(history[0].url, "https://e.test/20");
    }

    #[test]
    fn downloads_are_capped_without_dropping_active_ones() {
        let mut downloads: Vec<SavedDownload> = (0..DOWNLOAD_LIMIT + 5)
            .map(|i| SavedDownload { path: format!("f{i}"), state: if i == 0 { "InProgress" } else { "Completed" }.into(), ..Default::default() })
            .collect();
        trim_downloads(&mut downloads);
        assert_eq!(downloads.len(), DOWNLOAD_LIMIT);
        assert_eq!(downloads[0].path, "f0");
        assert_eq!(downloads[1].path, "f6");
    }

    #[test]
    fn download_names_never_overwrite() {
        let taken = ["/d/a.zip", "/d/a (1).zip", "/d/README"];
        let exists = |p: &Path| taken.iter().any(|t| Path::new(t) == p);
        assert_eq!(unique_path(Path::new("/d"), "b.zip", exists), Path::new("/d/b.zip"));
        assert_eq!(unique_path(Path::new("/d"), "a.zip", exists), Path::new("/d/a (2).zip"));
        assert_eq!(unique_path(Path::new("/d"), "README", exists), Path::new("/d/README (1)"));
    }

    #[test]
    fn formats_utc_timestamps() {
        assert_eq!(iso8601_utc(0), "1970-01-01T00:00:00+00:00");
        assert_eq!(iso8601_utc(1_791_511_239), "2026-10-09T02:00:39+00:00");
    }

    #[test]
    fn parses_saved_timestamps() {
        assert_eq!(parse_iso8601("2026-10-09T02:00:39+00:00"), Some(1_791_511_239));
        assert_eq!(parse_iso8601("2026-10-09T11:00:39.1234567+09:00"), Some(1_791_511_239), ".NET writes fractions and local offsets");
        assert_eq!(parse_iso8601("2026-10-09T02:00:39Z"), Some(1_791_511_239));
        assert_eq!(parse_iso8601("1970-01-01T00:00:00+00:00"), Some(0));
        assert_eq!(parse_iso8601("2024-02-29T00:00:00-0130"), Some(1_709_170_200));
        for bad in ["", "yesterday", "2026-13-01T00:00:00Z", "2026-10-09"] {
            assert_eq!(parse_iso8601(bad), None, "{bad}");
        }
    }

    #[test]
    fn imported_bookmark_titles_are_kept_for_saved_bookmarks_only() {
        let state = BrowserState::parse(r#"{"Bookmarks":["https://a.test/"],"BookmarkTitles":{"https://a.test/":"A","https://gone.test/":"G","https://a.test/x":5}}"#).unwrap();
        assert_eq!(state.bookmark_titles.len(), 1);
        assert!(state.to_json().contains("\"BookmarkTitles\""));
        assert!(!BrowserState::default().to_json().contains("BookmarkTitles"), "the C# build's layout is unchanged without imports");
        assert!(BrowserState::parse(r#"{"BookmarkTitles":"x"}"#).is_ok());
    }
}
