//! User preferences, saved in state.json under "Settings".
//! Reading is lenient per field: an unknown or mistyped value falls back to its
//! default instead of making the whole state file unreadable.

use crate::{address_url, new_tab_image, URL_LIMIT};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

/// Longest download folder path accepted.
const FOLDER_LIMIT: usize = 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Accent {
    #[default]
    Blue,
    Purple,
    Pink,
    Orange,
    Green,
    Graphite,
}

/// Whether the sidebar opens at startup.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SidebarStartup {
    /// As it was when the browser closed.
    #[default]
    Remember,
    Open,
    Closed,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchEngine {
    #[default]
    Google,
    Bing,
    DuckDuckGo,
    YahooJapan,
}

impl SearchEngine {
    /// Results page for an already percent-encoded query.
    pub fn search_url(self, encoded_query: &str) -> String {
        let base = match self {
            Self::Google => "https://www.google.com/search?q=",
            Self::Bing => "https://www.bing.com/search?q=",
            Self::DuckDuckGo => "https://duckduckgo.com/?q=",
            Self::YahooJapan => "https://search.yahoo.co.jp/search?p=",
        };
        format!("{base}{encoded_query}")
    }
}

/// What opens when the browser starts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Startup {
    /// The tabs from last time.
    #[default]
    Restore,
    /// One blank tab.
    NewTab,
    /// The home page (a blank tab when none is set).
    Home,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Settings {
    pub theme: Theme,
    pub accent: Accent,
    pub sidebar: SidebarStartup,
    pub search: SearchEngine,
    pub startup: Startup,
    pub home_page: String,
    /// Empty = WebView2's default (the user's Downloads folder).
    pub download_folder: String,
    pub ask_download_folder: bool,
    pub keep_history: bool,
    /// File name of the new tab picture in `<data>/NewTab/`; empty = none.
    pub new_tab_image: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            accent: Accent::default(),
            sidebar: SidebarStartup::default(),
            search: SearchEngine::default(),
            startup: Startup::default(),
            home_page: String::new(),
            download_folder: String::new(),
            ask_download_folder: false,
            keep_history: true,
            new_tab_image: String::new(),
        }
    }
}

impl Settings {
    /// Applies the fields present in `patch` (PascalCase keys) over these
    /// settings. Invalid fields are ignored, so a mistyped home page keeps the
    /// previous one; an empty one clears it.
    pub fn merge(&mut self, patch: &Map<String, Value>) {
        fn field<T: DeserializeOwned>(patch: &Map<String, Value>, key: &str, slot: &mut T) {
            if let Some(value) = patch.get(key).and_then(|v| T::deserialize(v).ok()) {
                *slot = value;
            }
        }
        field(patch, "Theme", &mut self.theme);
        field(patch, "Accent", &mut self.accent);
        field(patch, "Sidebar", &mut self.sidebar);
        field(patch, "Search", &mut self.search);
        field(patch, "Startup", &mut self.startup);
        field(patch, "AskDownloadFolder", &mut self.ask_download_folder);
        field(patch, "KeepHistory", &mut self.keep_history);
        if let Some(home) = patch.get("HomePage").and_then(Value::as_str).map(str::trim) {
            if home.is_empty() {
                self.home_page.clear();
            } else if let Some(url) = address_url(home).filter(|u| u.len() <= URL_LIMIT) {
                self.home_page = url;
            }
        }
        if let Some(name) = patch.get("NewTabImage").and_then(Value::as_str) {
            if name.is_empty() || new_tab_image::is_stored_name(name) {
                self.new_tab_image = name.into();
            }
        }
        if let Some(folder) = patch.get("DownloadFolder").and_then(Value::as_str).map(str::trim) {
            if folder.len() <= FOLDER_LIMIT && !folder.contains('\0') {
                self.download_folder = folder.into();
            }
        }
    }
}

impl<'de> Deserialize<'de> for Settings {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut settings = Settings::default();
        if let Value::Object(map) = Value::deserialize(deserializer)? {
            settings.merge(&map);
        }
        Ok(settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unknown_or_mistyped_values_fall_back_per_field() {
        let s: Settings = serde_json::from_value(json!({
            "Theme": "Neon", "Accent": "Pink", "Search": 3, "KeepHistory": "yes", "Startup": "Home", "Extra": true
        }))
        .unwrap();
        assert_eq!(s, Settings { accent: Accent::Pink, startup: Startup::Home, ..Settings::default() });
        let s: Settings = serde_json::from_value(json!("not an object")).unwrap();
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn merge_sanitizes_the_home_page_and_folder() {
        let mut s = Settings::default();
        s.merge(json!({"HomePage": " example.org ", "DownloadFolder": "  D:\\Files  "}).as_object().unwrap());
        assert_eq!((s.home_page.as_str(), s.download_folder.as_str()), ("https://example.org/", "D:\\Files"));
        s.merge(json!({"HomePage": "just words"}).as_object().unwrap());
        assert_eq!(s.home_page, "https://example.org/", "search text is not a home page; the old one stays");
        s.merge(json!({"HomePage": "javascript:alert(1)", "DownloadFolder": "a\u{0}b"}).as_object().unwrap());
        assert_eq!((s.home_page.as_str(), s.download_folder.as_str()), ("https://example.org/", "D:\\Files"));
        s.merge(json!({"Theme": "Dark"}).as_object().unwrap());
        assert_eq!(s.theme, Theme::Dark, "absent fields are untouched");
        s.merge(json!({"HomePage": " ", "DownloadFolder": ""}).as_object().unwrap());
        assert_eq!((s.home_page.as_str(), s.download_folder.as_str()), ("", ""), "empty clears");
        let s: Settings = serde_json::from_value(json!({"HomePage": "not a url"})).unwrap();
        assert_eq!(s.home_page, "", "invalid saved value falls back to the default");
    }

    #[test]
    fn new_tab_image_accepts_only_stored_picture_names() {
        let mut s = Settings::default();
        s.merge(json!({"NewTabImage": "newtab-1760400000000.jpg"}).as_object().unwrap());
        assert_eq!(s.new_tab_image, "newtab-1760400000000.jpg");
        for bad in ["C:\\Users\\me\\a.jpg", "../state.json", "newtab-1.exe"] {
            s.merge(json!({"NewTabImage": bad}).as_object().unwrap());
            assert_eq!(s.new_tab_image, "newtab-1760400000000.jpg", "{bad} is rejected");
        }
        s.merge(json!({"NewTabImage": ""}).as_object().unwrap());
        assert_eq!(s.new_tab_image, "", "empty removes the picture");
    }

    #[test]
    fn search_engines_build_result_urls() {
        assert_eq!(SearchEngine::Bing.search_url("a%20b"), "https://www.bing.com/search?q=a%20b");
        assert_eq!(SearchEngine::DuckDuckGo.search_url("a"), "https://duckduckgo.com/?q=a");
        assert_eq!(SearchEngine::YahooJapan.search_url("a"), "https://search.yahoo.co.jp/search?p=a");
    }
}
