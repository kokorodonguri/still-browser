//! Reading another browser's data so a user can move to Still: bookmarks and
//! history from Chromium-based browsers (Chrome, Edge, Brave, Vivaldi, Opera)
//! and Firefox, cookies from Firefox, and the files browsers export (bookmark
//! HTML, password CSV, Google Takeout history JSON).
//!
//! Chromium cookies and passwords are encrypted for that browser alone
//! (app-bound encryption), so they are not read here; passwords come from the
//! CSV file every browser can export instead.

use crate::passwords::Login;
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use url::Url;

/// Seconds between 1601-01-01 (Chromium's epoch) and 1970-01-01.
const WEBKIT_EPOCH_OFFSET: i64 = 11_644_473_600;
/// Longest title kept from another browser.
const TITLE_LIMIT: usize = 300;
/// Exported files larger than this are refused before parsing.
pub const FILE_LIMIT: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub url: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Visit {
    pub url: String,
    pub title: String,
    /// Unix seconds of the latest visit.
    pub visited_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SameSite {
    None,
    Lax,
    Strict,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    /// A leading dot marks a domain cookie; otherwise the cookie is host-only.
    pub domain: String,
    pub path: String,
    /// Unix seconds.
    pub expires: i64,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: SameSite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Chromium,
    Firefox,
}

/// A browser profile found on this computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub browser: String,
    pub profile: String,
    pub engine: Engine,
    pub dir: PathBuf,
}

impl Source {
    /// The interface sends this back to pick the source.
    pub fn id(&self) -> String {
        self.dir.to_string_lossy().into_owned()
    }
}

#[derive(Debug)]
pub enum ImportError {
    /// The file is not one of the formats Still reads.
    Unrecognized,
    TooLarge,
    /// The browser holds its files open or they could not be read.
    Io(std::io::Error),
    Database(rusqlite::Error),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unrecognized => f.write_str("unrecognized file"),
            Self::TooLarge => f.write_str("file too large"),
            Self::Io(e) => write!(f, "{e}"),
            Self::Database(e) => write!(f, "{e}"),
        }
    }
}

impl From<std::io::Error> for ImportError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<rusqlite::Error> for ImportError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Database(e)
    }
}

/// Only web pages are imported; `javascript:`, `file:`, `chrome:` and the
/// like are dropped.
fn web_url(raw: &str) -> Option<String> {
    let url = Url::parse(raw.trim()).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| url.into())
}

fn clean_title(title: &str) -> String {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    match title.char_indices().nth(TITLE_LIMIT) {
        Some((end, _)) => title[..end].to_owned(),
        None => title,
    }
}

fn link(url: &str, title: &str) -> Option<Link> {
    Some(Link { url: web_url(url)?, title: clean_title(title) })
}

/// Keeps the first link per address, in order.
fn unique_links(links: impl IntoIterator<Item = Link>) -> Vec<Link> {
    let mut seen = HashSet::new();
    links.into_iter().filter(|l| seen.insert(l.url.clone())).collect()
}

/// Newest first, one entry per address.
fn unique_visits(mut visits: Vec<Visit>) -> Vec<Visit> {
    visits.sort_by_key(|v| std::cmp::Reverse(v.visited_at));
    let mut seen = HashSet::new();
    visits.retain(|v| seen.insert(v.url.clone()));
    visits
}

// ---------- Finding browsers ----------

/// Chromium-based browsers whose data lives under `%LOCALAPPDATA%`.
const CHROMIUM_LOCAL: [(&str, &str); 5] = [
    ("Google Chrome", "Google/Chrome/User Data"),
    ("Microsoft Edge", "Microsoft/Edge/User Data"),
    ("Brave", "BraveSoftware/Brave-Browser/User Data"),
    ("Vivaldi", "Vivaldi/User Data"),
    ("Chromium", "Chromium/User Data"),
];
/// Opera keeps a single profile under `%APPDATA%`.
const OPERA_ROAMING: [(&str, &str); 2] = [("Opera", "Opera Software/Opera Stable"), ("Opera GX", "Opera Software/Opera GX Stable")];

/// Browser profiles with bookmarks or history, given `%LOCALAPPDATA%` and
/// `%APPDATA%`.
pub fn discover(local: &Path, roaming: &Path) -> Vec<Source> {
    let mut sources = Vec::new();
    for (browser, relative) in CHROMIUM_LOCAL {
        sources.extend(chromium_profiles(browser, &local.join(relative)));
    }
    for (browser, relative) in OPERA_ROAMING {
        let dir = roaming.join(relative);
        if has_chromium_data(&dir) {
            sources.push(Source { browser: browser.into(), profile: String::new(), engine: Engine::Chromium, dir });
        }
    }
    sources.extend(firefox_profiles(&roaming.join("Mozilla/Firefox")));
    sources
}

fn has_chromium_data(dir: &Path) -> bool {
    dir.join("Bookmarks").is_file() || dir.join("History").is_file()
}

#[derive(Deserialize, Default)]
struct LocalState {
    #[serde(default)]
    profile: ProfileInfo,
}

#[derive(Deserialize, Default)]
struct ProfileInfo {
    #[serde(default)]
    info_cache: std::collections::BTreeMap<String, ProfileEntry>,
}

#[derive(Deserialize, Default)]
struct ProfileEntry {
    #[serde(default)]
    name: String,
}

/// Profiles named in `Local State`, or the usual folder names when it is missing.
fn chromium_profiles(browser: &str, user_data: &Path) -> Vec<Source> {
    let named: Vec<(String, String)> = std::fs::read_to_string(user_data.join("Local State"))
        .ok()
        .and_then(|text| serde_json::from_str::<LocalState>(&text).ok())
        .map(|state| state.profile.info_cache.into_iter().map(|(dir, entry)| (dir, entry.name)).collect())
        .unwrap_or_default();
    let mut folders = named;
    if folders.is_empty() {
        let mut found: Vec<String> = std::fs::read_dir(user_data)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name == "Default" || name.starts_with("Profile "))
            .collect();
        found.sort();
        folders = found.into_iter().map(|dir| (dir.clone(), dir)).collect();
    }
    folders
        .into_iter()
        .filter(|(dir, _)| !dir.contains(['/', '\\']) && dir != ".." && dir != ".")
        .map(|(dir, name)| (user_data.join(&dir), name))
        .filter(|(dir, _)| has_chromium_data(dir))
        .map(|(dir, profile)| Source { browser: browser.into(), profile, engine: Engine::Chromium, dir })
        .collect()
}

/// Profiles listed in Firefox's `profiles.ini` that hold `places.sqlite`.
fn firefox_profiles(root: &Path) -> Vec<Source> {
    let Ok(ini) = std::fs::read_to_string(root.join("profiles.ini")) else { return Vec::new() };
    let mut sources = Vec::new();
    let mut section: Option<(String, String, bool)> = None;
    let mut finish = |entry: Option<(String, String, bool)>| {
        let Some((name, path, relative)) = entry else { return };
        if path.is_empty() {
            return;
        }
        let dir = if relative { root.join(path.replace('\\', "/")) } else { PathBuf::from(&path) };
        if dir.join("places.sqlite").is_file() {
            sources.push(Source { browser: "Firefox".into(), profile: name, engine: Engine::Firefox, dir });
        }
    };
    for line in ini.lines().map(str::trim) {
        if line.starts_with('[') {
            finish(section.take());
            if line.to_ascii_lowercase().starts_with("[profile") {
                section = Some((String::new(), String::new(), true));
            }
        } else if let (Some(entry), Some((key, value))) = (section.as_mut(), line.split_once('=')) {
            match key.trim() {
                "Name" => entry.0 = value.trim().into(),
                "Path" => entry.1 = value.trim().into(),
                "IsRelative" => entry.2 = value.trim() != "0",
                _ => {}
            }
        }
    }
    finish(section.take());
    sources
}

// ---------- Reading profiles ----------

/// Bookmarks from a Chromium `Bookmarks` file, in folder order.
pub fn chromium_bookmarks(json: &str) -> Result<Vec<Link>, ImportError> {
    #[derive(Deserialize)]
    struct Node {
        #[serde(default, rename = "type")]
        kind: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        url: String,
        #[serde(default)]
        children: Vec<Node>,
    }
    #[derive(Deserialize)]
    struct File {
        roots: std::collections::BTreeMap<String, serde_json::Value>,
    }
    fn walk(node: &Node, out: &mut Vec<Link>) {
        if node.kind == "url" {
            out.extend(link(&node.url, &node.name));
        }
        for child in &node.children {
            walk(child, out);
        }
    }
    let file: File = serde_json::from_str(json).map_err(|_| ImportError::Unrecognized)?;
    let mut links = Vec::new();
    // The bar first, then the other folders, as the browser shows them.
    for key in ["bookmark_bar", "other", "synced"] {
        if let Some(node) = file.roots.get(key).and_then(|v| Node::deserialize(v).ok()) {
            walk(&node, &mut links);
        }
    }
    Ok(unique_links(links))
}

/// Copies a database (and its write-ahead log) out of a running browser's
/// profile, so it can be read without touching or locking the original.
fn open_copy(db: &Path, scratch: &Path) -> Result<rusqlite::Connection, ImportError> {
    std::fs::create_dir_all(scratch)?;
    let name = db.file_name().ok_or_else(|| ImportError::Io(std::io::ErrorKind::NotFound.into()))?;
    let copy = scratch.join(name);
    std::fs::copy(db, &copy)?;
    for suffix in ["-wal", "-journal"] {
        let mut side = db.as_os_str().to_owned();
        side.push(suffix);
        let mut target = copy.as_os_str().to_owned();
        target.push(suffix);
        if Path::new(&side).is_file() {
            std::fs::copy(&side, &target)?;
        }
    }
    Ok(rusqlite::Connection::open(&copy)?)
}

/// The newest `limit` pages from a Chromium `History` database.
pub fn chromium_history(db: &Path, scratch: &Path, limit: usize) -> Result<Vec<Visit>, ImportError> {
    let conn = open_copy(db, scratch)?;
    let mut query = conn.prepare(
        "SELECT url, title, last_visit_time FROM urls WHERE hidden = 0 AND last_visit_time > 0 ORDER BY last_visit_time DESC LIMIT ?1",
    )?;
    let rows = query.query_map([(limit * 4) as i64], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?.unwrap_or_default(), row.get::<_, i64>(2)?))
    })?;
    let mut visits = Vec::new();
    for row in rows {
        let (url, title, time) = row?;
        if let Some(l) = link(&url, &title) {
            visits.push(Visit { url: l.url, title: l.title, visited_at: time / 1_000_000 - WEBKIT_EPOCH_OFFSET });
        }
    }
    let mut visits = unique_visits(visits);
    visits.truncate(limit);
    Ok(visits)
}

/// Bookmarks from Firefox's `places.sqlite`, oldest first like its menu.
pub fn firefox_bookmarks(db: &Path, scratch: &Path) -> Result<Vec<Link>, ImportError> {
    let conn = open_copy(db, scratch)?;
    let mut query = conn.prepare(
        "SELECT p.url, COALESCE(b.title, p.title, '') FROM moz_bookmarks b JOIN moz_places p ON p.id = b.fk \
         WHERE b.type = 1 ORDER BY b.parent, b.position",
    )?;
    let rows = query.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
    let mut links = Vec::new();
    for row in rows {
        let (url, title) = row?;
        links.extend(link(&url, &title));
    }
    Ok(unique_links(links))
}

/// The newest `limit` pages from Firefox's `places.sqlite`.
pub fn firefox_history(db: &Path, scratch: &Path, limit: usize) -> Result<Vec<Visit>, ImportError> {
    let conn = open_copy(db, scratch)?;
    let mut query = conn.prepare(
        "SELECT url, COALESCE(title, ''), last_visit_date FROM moz_places \
         WHERE last_visit_date IS NOT NULL AND hidden = 0 ORDER BY last_visit_date DESC LIMIT ?1",
    )?;
    let rows = query.query_map([(limit * 4) as i64], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?)))?;
    let mut visits = Vec::new();
    for row in rows {
        let (url, title, time) = row?;
        if let Some(l) = link(&url, &title) {
            visits.push(Visit { url: l.url, title: l.title, visited_at: time / 1_000_000 });
        }
    }
    let mut visits = unique_visits(visits);
    visits.truncate(limit);
    Ok(visits)
}

/// Unexpired cookies from Firefox's `cookies.sqlite`. Cookies kept for a
/// container or partitioned under another site are left out: Still has
/// neither, so they would leak into the normal profile.
pub fn firefox_cookies(db: &Path, scratch: &Path, now: i64) -> Result<Vec<Cookie>, ImportError> {
    let conn = open_copy(db, scratch)?;
    let mut query = conn.prepare(
        "SELECT name, value, host, path, expiry, isSecure, isHttpOnly, sameSite FROM moz_cookies WHERE originAttributes = ''",
    )?;
    let rows = query.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, i64>(5)?,
            row.get::<_, i64>(6)?,
            row.get::<_, i64>(7)?,
        ))
    })?;
    let mut cookies = Vec::new();
    for row in rows {
        let (name, value, domain, path, expiry, secure, http_only, same_site) = row?;
        // Newer Firefox stores milliseconds.
        let expires = if expiry > 100_000_000_000 { expiry / 1000 } else { expiry };
        if expires <= now || domain.trim_start_matches('.').is_empty() || domain.contains(['/', ' ']) {
            continue;
        }
        let same_site = match same_site {
            0 => SameSite::None,
            2 => SameSite::Strict,
            _ => SameSite::Lax,
        };
        let path = if path.starts_with('/') { path } else { "/".into() };
        cookies.push(Cookie { name, value, domain, path, expires, secure: secure != 0, http_only: http_only != 0, same_site });
    }
    Ok(cookies)
}

// ---------- Exported files ----------

/// What an exported file turned out to hold.
#[derive(Debug, PartialEq)]
pub enum ImportedFile {
    Bookmarks(Vec<Link>),
    History(Vec<Visit>),
    Passwords(Vec<Login>),
}

/// Recognizes a bookmark HTML file, a password CSV file or a Google Takeout
/// `History.json`, by its contents rather than its name.
pub fn read_file(bytes: &[u8]) -> Result<ImportedFile, ImportError> {
    if bytes.len() > FILE_LIMIT {
        return Err(ImportError::TooLarge);
    }
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim_start_matches('\u{feff}').trim_start();
    let head: String = text.chars().take(512).collect::<String>().to_ascii_lowercase();
    if head.starts_with('{') {
        return takeout_history(text).map(ImportedFile::History);
    }
    if head.contains("netscape-bookmark") || head.starts_with("<!doctype") || head.starts_with("<html") || head.starts_with("<dl") || head.starts_with("<meta") || head.starts_with("<title") {
        let links = bookmarks_html(text);
        return if links.is_empty() { Err(ImportError::Unrecognized) } else { Ok(ImportedFile::Bookmarks(links)) };
    }
    passwords_csv(text).map(ImportedFile::Passwords)
}

/// Links from a bookmark file in the Netscape format, which Chrome, Edge,
/// Firefox and Safari all export.
pub fn bookmarks_html(text: &str) -> Vec<Link> {
    let lower = text.to_ascii_lowercase();
    let mut links = Vec::new();
    let mut at = 0;
    while let Some(start) = lower[at..].find("<a ").map(|i| at + i) {
        let Some(tag_end) = lower[start..].find('>').map(|i| start + i) else { break };
        let close = lower[tag_end..].find("</a>").map_or(lower.len(), |i| tag_end + i);
        let href = attribute(&text[start..tag_end], &lower[start..tag_end], "href");
        if let Some(href) = href {
            links.extend(link(&decode_entities(&href), &decode_entities(&strip_tags(&text[tag_end + 1..close]))));
        }
        at = close.max(tag_end + 1);
    }
    unique_links(links)
}

/// The value of `name` in a tag (`tag` and its lowercase copy `lower`).
fn attribute(tag: &str, lower: &str, name: &str) -> Option<String> {
    let mut search = 0;
    while let Some(i) = lower[search..].find(name).map(|i| search + i) {
        search = i + name.len();
        let before = lower[..i].chars().last();
        if !before.is_some_and(char::is_whitespace) {
            continue;
        }
        let rest = lower[search..].trim_start();
        if !rest.starts_with('=') {
            continue;
        }
        let offset = tag.len() - rest.len() + 1;
        let value = tag[offset..].trim_start();
        return Some(match value.chars().next() {
            Some(quote @ ('"' | '\'')) => value[1..].split(quote).next().unwrap_or_default().to_owned(),
            _ => value.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or_default().to_owned(),
        });
    }
    None
}

fn strip_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut inside = false;
    for c in text.chars() {
        match c {
            '<' => inside = true,
            '>' => inside = false,
            _ if !inside => out.push(c),
            _ => {}
        }
    }
    out
}

fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// `Chrome/History.json` (or `BrowserHistory.json`) from Google Takeout,
/// the only way to take history out of Chrome on a phone.
pub fn takeout_history(json: &str) -> Result<Vec<Visit>, ImportError> {
    #[derive(Deserialize)]
    struct Entry {
        #[serde(default)]
        url: String,
        #[serde(default)]
        title: String,
        #[serde(default)]
        time_usec: i64,
    }
    #[derive(Deserialize)]
    struct File {
        #[serde(rename = "Browser History")]
        history: Vec<Entry>,
    }
    let file: File = serde_json::from_str(json).map_err(|_| ImportError::Unrecognized)?;
    Ok(unique_visits(
        file.history
            .into_iter()
            .filter_map(|e| link(&e.url, &e.title).map(|l| Visit { url: l.url, title: l.title, visited_at: e.time_usec / 1_000_000 }))
            .collect(),
    ))
}

/// Logins from a password CSV file exported by Chrome, Edge, Firefox, Safari
/// or a password manager. Rows for apps (`android://`) are skipped.
pub fn passwords_csv(text: &str) -> Result<Vec<Login>, ImportError> {
    let mut rows = csv_rows(text).into_iter();
    let header: Vec<String> = rows.next().unwrap_or_default().into_iter().map(|h| h.trim().to_ascii_lowercase()).collect();
    let column = |names: &[&str]| header.iter().position(|h| names.contains(&h.as_str()));
    let (Some(url), Some(username), Some(password)) = (
        column(&["url", "login_uri", "website", "origin", "web site"]),
        column(&["username", "login_username", "login", "user", "email"]),
        column(&["password", "login_password"]),
    ) else {
        return Err(ImportError::Unrecognized);
    };
    let mut logins = Vec::new();
    for row in rows {
        let field = |i: usize| row.get(i).map(String::as_str).unwrap_or_default();
        if field(password).is_empty() {
            continue;
        }
        if let Some(login) = Login::new(field(url), field(username), field(password)) {
            logins.push(login);
        }
    }
    Ok(logins)
}

/// RFC 4180 rows: quoted fields may hold commas, quotes ("") and line breaks.
fn csv_rows(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            (true, '"') => quoted = false,
            (true, c) => field.push(c),
            (false, '"') if field.is_empty() => quoted = true,
            (false, ',') => row.push(std::mem::take(&mut field)),
            (false, '\r') => {}
            (false, '\n') => {
                row.push(std::mem::take(&mut field));
                if row.iter().any(|f| !f.is_empty()) {
                    rows.push(std::mem::take(&mut row));
                } else {
                    row.clear();
                }
            }
            (false, c) => field.push(c),
        }
    }
    row.push(field);
    if row.iter().any(|f| !f.is_empty()) {
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("still-import-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reads_exported_bookmark_html() {
        let html = r#"<!DOCTYPE NETSCAPE-Bookmark-file-1>
<META HTTP-EQUIV="Content-Type" CONTENT="text/html; charset=UTF-8">
<TITLE>Bookmarks</TITLE>
<DL><p>
    <DT><H3 ADD_DATE="1">Bookmarks bar</H3>
    <DL><p>
        <DT><A HREF="https://example.org/a?x=1&amp;y=2" ADD_DATE="1" ICON="data:image/png;base64,AAA">Example &amp; &#12354;</A>
        <DT><a href='https://example.org/b'>  Two
           lines </a>
        <DT><A HREF="javascript:alert(1)">Bookmarklet</A>
        <DT><A HREF="https://example.org/a?x=1&amp;y=2">Duplicate</A>
        <DT><A ADD_DATE="2" HREF=https://unquoted.test/>Unquoted</A>
    </DL><p>
</DL><p>"#;
        let ImportedFile::Bookmarks(links) = read_file(html.as_bytes()).unwrap() else { panic!("not bookmarks") };
        assert_eq!(
            links,
            vec![
                Link { url: "https://example.org/a?x=1&y=2".into(), title: "Example & あ".into() },
                Link { url: "https://example.org/b".into(), title: "Two lines".into() },
                Link { url: "https://unquoted.test/".into(), title: "Unquoted".into() },
            ]
        );
    }

    #[test]
    fn reads_password_csv_from_each_browser() {
        let chrome = "\u{feff}name,url,username,password,note\n\
            example.org,https://example.org/login,me@example.org,\"p,a\"\"ss\nword\",\n\
            app,android://abc@com.example/,me,secret,\n\
            empty,https://empty.test/,me,,\n";
        let ImportedFile::Passwords(logins) = read_file(chrome.as_bytes()).unwrap() else { panic!("not passwords") };
        assert_eq!(logins.len(), 1);
        assert_eq!((logins[0].url.as_str(), logins[0].username.as_str(), logins[0].password.as_str()), ("https://example.org", "me@example.org", "p,a\"ss\nword"));

        let firefox = "\"url\",\"username\",\"password\",\"httpRealm\",\"formActionOrigin\",\"guid\",\"timeCreated\",\"timeLastUsed\",\"timePasswordChanged\"\r\n\
            \"https://a.test:8443\",\"\",\"pw\",,\"https://a.test:8443\",\"{x}\",\"1\",\"1\",\"1\"\r\n";
        let logins = passwords_csv(firefox).unwrap();
        assert_eq!((logins[0].url.as_str(), logins[0].username.as_str()), ("https://a.test:8443", ""));

        let safari = "Title,URL,Username,Password,Notes,OTPAuth\nA,https://b.test/,u,p,,\n";
        assert_eq!(passwords_csv(safari).unwrap()[0].url, "https://b.test");
        assert!(matches!(read_file(b"just,some,columns\n1,2,3"), Err(ImportError::Unrecognized)));
    }

    #[test]
    fn reads_takeout_history() {
        let json = r#"{"Browser History":[
            {"title":"Old","url":"https://a.test/","time_usec":1700000000000000},
            {"title":"New","url":"https://a.test/","time_usec":1800000000000000},
            {"title":"B","url":"https://b.test/","time_usec":1750000000000000},
            {"title":"Settings","url":"chrome://settings/","time_usec":1900000000000000}]}"#;
        let ImportedFile::History(visits) = read_file(json.as_bytes()).unwrap() else { panic!("not history") };
        assert_eq!(visits.iter().map(|v| (v.title.as_str(), v.visited_at)).collect::<Vec<_>>(), vec![("New", 1_800_000_000), ("B", 1_750_000_000)]);
        assert!(matches!(read_file(b"{\"other\":1}"), Err(ImportError::Unrecognized)));
    }

    #[test]
    fn reads_chromium_bookmarks_in_folder_order() {
        let json = r#"{"roots":{
            "other":{"type":"folder","children":[{"type":"url","name":"Other","url":"https://other.test/"}]},
            "bookmark_bar":{"type":"folder","children":[
                {"type":"url","name":"Bar","url":"https://bar.test/"},
                {"type":"folder","name":"Nested","children":[{"type":"url","name":"Deep","url":"https://deep.test/"}]},
                {"type":"url","name":"Local","url":"file:///C:/x.html"}]},
            "synced":{"type":"folder","children":[]}},"version":1}"#;
        let links = chromium_bookmarks(json).unwrap();
        assert_eq!(links.iter().map(|l| l.title.as_str()).collect::<Vec<_>>(), vec!["Bar", "Deep", "Other"]);
        assert!(chromium_bookmarks("[]").is_err());
    }

    #[test]
    fn reads_chromium_history_database() {
        let dir = scratch("chromium");
        let db = dir.join("History");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE urls(id INTEGER PRIMARY KEY, url TEXT, title TEXT, visit_count INTEGER, typed_count INTEGER, last_visit_time INTEGER, hidden INTEGER);
             INSERT INTO urls VALUES (1,'https://a.test/','A',1,0,13400000000000000,0);
             INSERT INTO urls VALUES (2,'https://hidden.test/','H',1,0,13400000000000001,1);
             INSERT INTO urls VALUES (3,'chrome://newtab/','N',1,0,13400000000000002,0);
             INSERT INTO urls VALUES (4,'https://b.test/',NULL,1,0,13300000000000000,0);",
        )
        .unwrap();
        drop(conn);
        let visits = chromium_history(&db, &dir.join("copy"), 500).unwrap();
        assert_eq!(visits.len(), 2);
        assert_eq!((visits[0].url.as_str(), visits[0].visited_at), ("https://a.test/", 13_400_000_000 - WEBKIT_EPOCH_OFFSET));
        assert_eq!(visits[1].title, "");
        assert_eq!(chromium_history(&db, &dir.join("copy2"), 1).unwrap().len(), 1);
    }

    #[test]
    fn reads_firefox_places_and_cookies() {
        let dir = scratch("firefox");
        let places = dir.join("places.sqlite");
        let conn = rusqlite::Connection::open(&places).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE moz_places(id INTEGER PRIMARY KEY, url TEXT, title TEXT, last_visit_date INTEGER, hidden INTEGER DEFAULT 0);
             CREATE TABLE moz_bookmarks(id INTEGER PRIMARY KEY, type INTEGER, fk INTEGER, parent INTEGER, position INTEGER, title TEXT);
             INSERT INTO moz_places VALUES (1,'https://a.test/','A page',1700000000000000,0);
             INSERT INTO moz_places VALUES (2,'place:sort=8','Recent',NULL,0);
             INSERT INTO moz_places VALUES (3,'https://b.test/','B page',NULL,0);
             INSERT INTO moz_bookmarks VALUES (10,1,3,5,1,'B named');
             INSERT INTO moz_bookmarks VALUES (11,1,1,5,0,NULL);
             INSERT INTO moz_bookmarks VALUES (12,1,2,5,2,'Smart');
             INSERT INTO moz_bookmarks VALUES (13,2,NULL,5,3,'Folder');",
        )
        .unwrap();
        // Left open: the newest rows are still only in the write-ahead log.
        let links = firefox_bookmarks(&places, &dir.join("b")).unwrap();
        assert_eq!(links, vec![Link { url: "https://a.test/".into(), title: "A page".into() }, Link { url: "https://b.test/".into(), title: "B named".into() }]);
        let visits = firefox_history(&places, &dir.join("h"), 500).unwrap();
        assert_eq!(visits, vec![Visit { url: "https://a.test/".into(), title: "A page".into(), visited_at: 1_700_000_000 }]);
        drop(conn);

        let cookies = dir.join("cookies.sqlite");
        let conn = rusqlite::Connection::open(&cookies).unwrap();
        conn.execute_batch(
            "CREATE TABLE moz_cookies(id INTEGER PRIMARY KEY, originAttributes TEXT NOT NULL DEFAULT '', name TEXT, value TEXT, host TEXT, path TEXT, expiry INTEGER, isSecure INTEGER, isHttpOnly INTEGER, sameSite INTEGER);
             INSERT INTO moz_cookies VALUES (1,'','sid','abc','.example.org','/',2000000000,1,1,1);
             INSERT INTO moz_cookies VALUES (2,'','ms','v','host.test','',2000000000000,0,0,2);
             INSERT INTO moz_cookies VALUES (3,'','old','v','old.test','/',1000,0,0,0);
             INSERT INTO moz_cookies VALUES (4,'^userContextId=1','box','v','box.test','/',2000000000,0,0,0);",
        )
        .unwrap();
        let found = firefox_cookies(&cookies, &dir.join("c"), 1_800_000_000).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0], Cookie { name: "sid".into(), value: "abc".into(), domain: ".example.org".into(), path: "/".into(), expires: 2_000_000_000, secure: true, http_only: true, same_site: SameSite::Lax });
        assert_eq!((found[1].expires, found[1].path.as_str(), found[1].same_site), (2_000_000_000, "/", SameSite::Strict));
    }

    #[test]
    fn discovers_installed_browsers() {
        let dir = scratch("discover");
        let (local, roaming) = (dir.join("Local"), dir.join("Roaming"));
        let chrome = local.join("Google/Chrome/User Data");
        for profile in ["Default", "Profile 2", "Profile 3"] {
            std::fs::create_dir_all(chrome.join(profile)).unwrap();
        }
        std::fs::write(chrome.join("Default/Bookmarks"), "{}").unwrap();
        std::fs::write(chrome.join("Profile 2/History"), "").unwrap();
        std::fs::write(chrome.join("Local State"), r#"{"profile":{"info_cache":{"Default":{"name":"Personal"},"Profile 2":{"name":"Work"},"Profile 3":{"name":"Empty"},"../x":{"name":"Escape"}}}}"#).unwrap();
        let edge = local.join("Microsoft/Edge/User Data/Default");
        std::fs::create_dir_all(&edge).unwrap();
        std::fs::write(edge.join("History"), "").unwrap();
        let firefox = roaming.join("Mozilla/Firefox");
        std::fs::create_dir_all(firefox.join("Profiles/abc.default-release")).unwrap();
        std::fs::write(firefox.join("Profiles/abc.default-release/places.sqlite"), "").unwrap();
        std::fs::write(firefox.join("profiles.ini"), "[General]\nStartWithLastProfile=1\n\n[Profile1]\nName=default\nIsRelative=1\nPath=Profiles/missing\n\n[Profile0]\nName=default-release\nIsRelative=1\nPath=Profiles/abc.default-release\nDefault=1\n\n[Install1]\nDefault=Profiles/abc.default-release\n").unwrap();

        let found: Vec<(String, String, Engine)> = discover(&local, &roaming).into_iter().map(|s| (s.browser, s.profile, s.engine)).collect();
        assert_eq!(
            found,
            vec![
                ("Google Chrome".into(), "Personal".into(), Engine::Chromium),
                ("Google Chrome".into(), "Work".into(), Engine::Chromium),
                ("Microsoft Edge".into(), "Default".into(), Engine::Chromium),
                ("Firefox".into(), "default-release".into(), Engine::Firefox),
            ]
        );
        assert!(discover(&dir.join("none"), &dir.join("none")).is_empty());
    }
}
