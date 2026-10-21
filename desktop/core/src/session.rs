//! The browser's state machine. `Session` decides; the Tauri shell executes the
//! returned plans (create/show/hide webviews) after releasing its lock.

use crate::import::{Link, Visit};
use crate::passwords::{Login, LoginView, Vault};
use crate::settings::{SidebarStartup, Startup};
use crate::{
    host_label, iso8601_utc, normalize_with, parse_iso8601, push_history, trim_downloads, BrowserState, HistoryEntry,
    SavedDownload, Settings, SavedTab, WindowPlacement, BLANK, HISTORY_LIMIT, SCHEMA_VERSION, URL_LIMIT,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

pub type TabId = u64;

const TITLE_LIMIT: usize = 300;
const BOOKMARKS: u8 = 1;
const HISTORY: u8 = 2;
const DOWNLOADS: u8 = 4;
const PASSWORDS: u8 = 8;
const ALL_LIBRARY: u8 = BOOKMARKS | HISTORY | DOWNLOADS | PASSWORDS;

#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    pub id: TabId,
    pub url: String,
    pub title: String,
    pub loading: bool,
    pub can_back: bool,
    pub can_forward: bool,
    /// A webview exists for this tab (created lazily on first show).
    pub has_view: bool,
    /// Identifies this view creation across crash/retry cycles.
    pub view_generation: u64,
    /// The renderer crashed or hung; Reload recovers.
    pub crashed: bool,
    /// When the page was last shown (a counter, larger is more recent).
    pub last_shown: u64,
    /// The hidden page was checked and holds nothing that releasing it would
    /// lose (no edited fields, no unload warning). Cleared whenever it is shown.
    pub releasable: bool,
}

impl Tab {
    fn new(id: TabId, url: String) -> Self {
        let title = if url == BLANK { "New tab".into() } else { host_label(&url) };
        Self {
            id, url, title, loading: false, can_back: false, can_forward: false, has_view: false, view_generation: 0,
            crashed: false, last_shown: 0, releasable: false,
        }
    }
}

/// Content-area rectangle in logical (CSS) pixels.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// What the shell must do to make the active tab visible.
#[derive(Debug, Clone, PartialEq)]
pub struct ShowPlan {
    /// Previously visible page to hide (and let rest).
    pub hide: Option<TabId>,
    pub action: ShowAction,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ShowAction {
    /// Nothing to show (blank tab or no geometry yet).
    None,
    Create { id: TabId, generation: u64, url: String, bounds: Bounds },
    /// Already visible: only the geometry may have changed.
    Resize { id: TabId, bounds: Bounds },
    Reveal { id: TabId, bounds: Bounds },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Navigation {
    /// Navigate the existing webview.
    Existing { id: TabId, url: String },
    /// The tab has no webview yet; run `plan_show` to create it.
    Create,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProcessFailure {
    RendererExited,
    RendererUnresponsive,
    /// WebView2's browser process died: every page webview is gone.
    BrowserExited,
}

/// How the saved state was obtained, so the shell can back up and log.
#[derive(Debug, Clone, PartialEq)]
pub enum LoadOutcome {
    Fresh,
    Restored,
    /// Unparseable: back up the file before it is overwritten.
    Corrupt,
    /// Written by a newer build: back up before overwriting.
    Newer(u32),
    Unreadable,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct TabView {
    pub id: TabId,
    pub title: String,
    pub url: String,
    pub loading: bool,
    pub crashed: bool,
}

impl From<&Tab> for TabView {
    fn from(tab: &Tab) -> Self {
        Self { id: tab.id, title: tab.title.clone(), url: tab.url.clone(), loading: tab.loading, crashed: tab.crashed }
    }
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct LinkView {
    pub url: String,
    pub title: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct DownloadView {
    pub name: String,
    pub path: String,
    pub state: String,
    pub bytes: i64,
}

/// Everything the interface renders. Lists are omitted when unchanged.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub revision: u64,
    /// Full ordered list only for initial state or a structural change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tabs: Option<Vec<TabView>>,
    /// Otherwise only the changed rows, keyed by stable tab ID.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tab_changes: Vec<TabView>,
    pub active: TabId,
    pub url: String,
    pub loading: bool,
    pub can_back: bool,
    pub can_forward: bool,
    pub bookmarked: bool,
    pub sidebar_open: bool,
    pub fullscreen: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bookmarks: Option<Vec<LinkView>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history: Option<Vec<LinkView>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub downloads: Option<Vec<DownloadView>>,
    /// Saved logins for the active page (never the passwords).
    pub logins: Vec<LoginView>,
    /// Every saved login, for settings; omitted when unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_logins: Option<Vec<LoginView>>,
    pub downloading: usize,
    pub notice: Option<String>,
    pub settings_open: bool,
    /// Omitted when unchanged since the last update.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings: Option<Settings>,
}

pub mod notices {
    pub const CORRUPT: &str = "Saved browser data could not be read. A backup is required before saving new data.";
    pub const NEWER: &str = "Saved data came from a newer version of Still. A backup is required before saving.";
    pub const UNREADABLE: &str = "Saved browser data could not be read. Check folder permissions.";
    pub const SAVE_FAILED: &str = "Changes could not be saved. Check available disk space and folder permissions.";
    pub const BAD_ADDRESS: &str = "The address or search text is too long or invalid. Shorten it and try again.";
    pub const NO_PAGE: &str = "Open a page before adding a bookmark.";
    pub const CRASHED: &str = "This page stopped working. Reload to try again.";
    pub const UNRESPONSIVE: &str = "This page is not responding. Reload to try again.";
    pub const RESTARTED: &str = "The page engine restarted. Your tabs were kept.";
    pub const FOLDER_GONE: &str = "The download folder is no longer available.";
    pub const FOLDER_FAILED: &str = "The download folder could not open.";
    pub const DOWNLOAD_FOLDER_GONE: &str = "The chosen download folder is not available. The file was saved to Downloads instead.";
    pub const CLEARED: &str = "Browsing data was cleared.";
    pub const CLEAR_FAILED: &str = "Some browsing data could not be cleared. Try again.";
    pub const IMAGE_UNSUPPORTED: &str = "That picture cannot be used. Choose a PNG, JPEG, GIF, WebP, AVIF or BMP file up to 20 MB.";
    pub const IMAGE_FAILED: &str = "The picture could not be saved. Check available disk space and folder permissions.";
    pub const IMPORT_FAILED: &str = "Nothing could be imported. Close that browser and try again.";
    pub const IMPORT_UNRECOGNIZED: &str = "That file cannot be imported. Choose a bookmarks file (HTML), a passwords file (CSV) or a Google Takeout history file (JSON).";
    pub const IMPORT_TOO_LARGE: &str = "That file is too large to import.";
    pub const IMPORT_NOTHING: &str = "There was nothing new to import.";
    pub const PASSWORDS_UNREADABLE: &str = "Saved passwords could not be read. A backup was kept; import them again.";
    pub const PASSWORDS_NOT_SAVED: &str = "Passwords could not be saved on this computer.";
    pub const FILL_NONE: &str = "No sign-in field was found on this page.";

    /// "Imported 3 bookmarks and 1 password from Firefox." Kinds with no
    /// items are left out; `None` when nothing was imported at all.
    pub fn imported(counts: &[(usize, &str, &str)], from: Option<&str>) -> Option<String> {
        let parts: Vec<String> = counts
            .iter()
            .filter(|(n, _, _)| *n > 0)
            .map(|(n, one, many)| format!("{n} {}", if *n == 1 { one } else { many }))
            .collect();
        let list = match parts.as_slice() {
            [] => return None,
            [one] => one.clone(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        };
        Some(match from {
            Some(from) => format!("Imported {list} from {from}."),
            None => format!("Imported {list}."),
        })
    }
}

#[derive(Debug)]
pub struct Session {
    tabs: Vec<Tab>,
    /// Ordered storage plus O(1) lookup; closing repairs the shifted suffix.
    tab_indices: HashMap<TabId, usize>,
    tabs_reordered: bool,
    changed_tabs: HashSet<TabId>,
    snapshot_revision: u64,
    active: TabId,
    next_id: TabId,
    show_count: u64,
    /// Page currently visible, so switching hides only that one.
    shown: Option<TabId>,
    bounds: Option<Bounds>,
    /// Whole-window rectangle, used while a page is fullscreen.
    window_bounds: Option<Bounds>,
    fullscreen: Option<TabId>,
    pub sidebar_open: bool,
    settings: Settings,
    /// The settings page covers the content area; the page is hidden meanwhile.
    settings_open: bool,
    bookmarks: Vec<String>,
    bookmark_titles: BTreeMap<String, String>,
    history: Vec<HistoryEntry>,
    downloads: Vec<SavedDownload>,
    /// Logins imported from other browsers; saved separately and encrypted.
    vault: Vault,
    vault_dirty: bool,
    window: Option<WindowPlacement>,
    notice: Option<String>,
    library_dirty: u8,
    settings_dirty: bool,
    dirty: bool,
}

impl Session {
    /// Builds a session from the saved file's contents (`None` = file missing).
    /// `extra_urls` (command line) open as new tabs after the restored ones.
    pub fn load(saved: Option<Result<String, ()>>, extra_urls: &[String]) -> (Self, LoadOutcome) {
        let (state, outcome) = match saved {
            None => (BrowserState::default(), LoadOutcome::Fresh),
            Some(Err(())) => (BrowserState::default(), LoadOutcome::Unreadable),
            Some(Ok(text)) => match BrowserState::parse(&text) {
                Ok(state) if state.version > SCHEMA_VERSION => {
                    let version = state.version;
                    (state, LoadOutcome::Newer(version))
                }
                Ok(state) => (state, LoadOutcome::Restored),
                Err(_) => (BrowserState::default(), LoadOutcome::Corrupt),
            },
        };
        let notice = match outcome {
            LoadOutcome::Corrupt => Some(notices::CORRUPT),
            LoadOutcome::Newer(_) => Some(notices::NEWER),
            LoadOutcome::Unreadable => Some(notices::UNREADABLE),
            _ => None,
        };
        let mut session = Self {
            tabs: Vec::new(),
            tab_indices: HashMap::new(),
            tabs_reordered: true,
            changed_tabs: HashSet::new(),
            snapshot_revision: 0,
            active: 0,
            next_id: 1,
            show_count: 0,
            shown: None,
            bounds: None,
            window_bounds: None,
            fullscreen: None,
            sidebar_open: match state.settings.sidebar {
                SidebarStartup::Remember => state.sidebar_open,
                SidebarStartup::Open => true,
                SidebarStartup::Closed => false,
            },
            settings: state.settings,
            settings_open: false,
            bookmarks: state.bookmarks,
            bookmark_titles: state.bookmark_titles,
            history: state.history,
            downloads: state.downloads,
            vault: Vault::default(),
            vault_dirty: false,
            window: state.window,
            notice: notice.map(Into::into),
            library_dirty: ALL_LIBRARY,
            settings_dirty: true,
            dirty: false,
        };
        match session.settings.startup {
            Startup::Restore => {
                for saved in state.tabs {
                    let url = if saved.url == BLANK { saved.url } else { session.normalize(&saved.url) };
                    session.push_tab(url);
                }
            }
            Startup::NewTab => {}
            Startup::Home if session.settings.home_page.is_empty() => {}
            Startup::Home => {
                session.push_tab(session.settings.home_page.clone());
            }
        }
        if session.tabs.is_empty() {
            session.push_tab(BLANK.into());
        }
        let restored = if session.settings.startup == Startup::Restore { state.active_index } else { 0 };
        session.active = session.tabs[restored.min(session.tabs.len() - 1)].id;
        let mut opened = false;
        for url in extra_urls.iter().filter_map(|u| launch_url(u)) {
            session.active = session.push_tab(url);
            opened = true;
        }
        if opened || matches!(outcome, LoadOutcome::Newer(_)) {
            session.dirty = true;
        }
        (session, outcome)
    }

    fn normalize(&self, input: &str) -> String {
        normalize_with(input, self.settings.search)
    }

    fn push_tab(&mut self, url: String) -> TabId {
        let id = self.next_id;
        self.next_id += 1;
        self.tab_indices.insert(id, self.tabs.len());
        self.tabs.push(Tab::new(id, url));
        self.tabs_reordered = true;
        id
    }

    pub fn tab(&self, id: TabId) -> Option<&Tab> {
        self.tab_indices.get(&id).and_then(|&index| self.tabs.get(index))
    }

    fn tab_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        let index = *self.tab_indices.get(&id)?;
        self.tabs.get_mut(index)
    }

    pub fn active(&self) -> TabId {
        self.active
    }

    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    pub fn window_placement(&self) -> Option<WindowPlacement> {
        self.window
    }

    pub fn is_fullscreen(&self) -> bool {
        self.fullscreen.is_some()
    }

    fn changed(&mut self) {
        self.dirty = true;
    }

    fn library_changed(&mut self, lists: u8) {
        self.library_dirty |= lists;
        self.dirty = true;
    }

    pub fn set_notice(&mut self, text: impl Into<String>) {
        self.notice = Some(text.into());
    }

    pub fn dismiss_notice(&mut self) {
        self.notice = None;
    }

    // ---------- Tabs ----------

    pub fn open_tab(&mut self, url: &str) -> TabId {
        let url = if url == BLANK { BLANK.into() } else { self.normalize(url) };
        self.active = self.push_tab(url);
        self.settings_open = false;
        self.changed();
        self.active
    }

    /// Opens URLs handed over by a second launch. Returns whether any opened.
    pub fn open_launch_urls(&mut self, args: &[String]) -> bool {
        let mut opened = false;
        for url in args.iter().filter_map(|a| launch_url(a)) {
            self.active = self.push_tab(url);
            opened = true;
        }
        if opened {
            self.settings_open = false;
            self.changed();
        }
        opened
    }

    pub fn select(&mut self, id: TabId) -> bool {
        if self.tab(id).is_none() {
            return false;
        }
        self.active = id;
        self.settings_open = false;
        self.changed();
        true
    }

    pub fn cycle(&mut self, forward: bool) -> TabId {
        let len = self.tabs.len() as i64;
        let at = self.tab_indices.get(&self.active).copied().unwrap_or(0) as i64;
        let id = self.tabs[(at + if forward { 1 } else { -1 }).rem_euclid(len) as usize].id;
        self.select(id);
        id
    }

    /// Removes a tab. Returns whether it had a webview the shell must close.
    pub fn close(&mut self, id: TabId) -> Option<bool> {
        let index = self.tab_indices.remove(&id)?;
        let tab = self.tabs.remove(index);
        for (offset, tab) in self.tabs[index..].iter().enumerate() {
            self.tab_indices.insert(tab.id, index + offset);
        }
        self.changed_tabs.remove(&id);
        self.tabs_reordered = true;
        if self.shown == Some(id) {
            self.shown = None;
        }
        if self.fullscreen == Some(id) {
            self.fullscreen = None;
        }
        if self.tabs.is_empty() {
            self.push_tab(BLANK.into());
        }
        if self.active == id {
            self.active = self.tabs[index.min(self.tabs.len() - 1)].id;
        }
        self.release_spare_capacity();
        self.changed();
        Some(tab.has_view)
    }

    /// After many tabs close, the tab tables keep their peak capacity. Halve it
    /// once they are three-quarters empty (amortized, so closing stays O(1)).
    fn release_spare_capacity(&mut self) {
        const MIN: usize = 64;
        let len = self.tabs.len();
        if self.tabs.capacity() > MIN && len * 4 <= self.tabs.capacity() {
            let keep = (len * 2).max(MIN);
            self.tabs.shrink_to(keep);
            self.tab_indices.shrink_to(keep);
            self.changed_tabs.shrink_to(keep);
        }
    }

    pub fn navigate(&mut self, input: &str) -> Result<Option<Navigation>, &'static str> {
        if input.trim().len() > URL_LIMIT {
            self.set_notice(notices::BAD_ADDRESS);
            return Err(notices::BAD_ADDRESS);
        }
        let url = self.normalize(input);
        if url == BLANK {
            return Ok(None);
        }
        if url.len() > URL_LIMIT || url::Url::parse(&url).is_err() {
            self.set_notice(notices::BAD_ADDRESS);
            return Err(notices::BAD_ADDRESS);
        }
        let id = self.active;
        let Some(tab) = self.tab_mut(id) else { return Ok(None) };
        tab.url = url.clone();
        tab.loading = true;
        tab.crashed = false;
        let has_view = tab.has_view;
        self.changed_tabs.insert(id);
        self.settings_open = false;
        self.changed();
        Ok(Some(if has_view { Navigation::Existing { id, url } } else { Navigation::Create }))
    }

    // ---------- Geometry ----------

    pub fn set_bounds(&mut self, bounds: Bounds) {
        self.bounds = Some(bounds);
    }

    pub fn set_window_bounds(&mut self, bounds: Bounds) {
        self.window_bounds = Some(bounds);
    }

    pub fn set_window_placement(&mut self, placement: WindowPlacement) {
        if self.window != Some(placement) {
            self.window = Some(placement);
            self.changed();
        }
    }

    /// Plans showing the active page. Webviews are created lazily on first show.
    /// `force` re-applies geometry to an already visible page (DPI change).
    pub fn plan_show(&mut self, force: bool) -> ShowPlan {
        if self.settings_open {
            return ShowPlan { hide: self.shown.take(), action: ShowAction::None };
        }
        let active = self.active;
        let hide = self.shown.filter(|&id| id != active);
        let already = self.shown == Some(active) && !force;
        let bounds = if self.fullscreen == Some(active) { self.window_bounds.or(self.bounds) } else { self.bounds };
        self.show_count += 1;
        let show_count = self.show_count;
        let Some(tab) = self.tab_mut(active) else {
            self.shown = None;
            return ShowPlan { hide, action: ShowAction::None };
        };
        // A visible page can be edited, so it must be checked again when hidden.
        tab.releasable = false;
        tab.last_shown = show_count;
        let action = match bounds {
            Some(bounds) if tab.url != BLANK => {
                if !tab.has_view {
                    tab.has_view = true;
                    tab.view_generation += 1;
                    ShowAction::Create { id: active, generation: tab.view_generation, url: tab.url.clone(), bounds }
                } else if already {
                    ShowAction::Resize { id: active, bounds }
                } else {
                    ShowAction::Reveal { id: active, bounds }
                }
            }
            _ => ShowAction::None,
        };
        self.shown = match action {
            ShowAction::None => None,
            _ => Some(active),
        };
        ShowPlan { hide, action }
    }

    /// Reject a late creation or callback from a closed/replaced webview.
    pub fn view_is_current(&self, id: TabId, generation: u64) -> bool {
        self.tab(id).is_some_and(|tab| tab.has_view && tab.view_generation == generation)
    }

    /// The shell could not create the webview.
    pub fn view_failed(&mut self, id: TabId, error: &str) {
        if let Some(tab) = self.tab_mut(id) {
            tab.has_view = false;
            tab.loading = false;
            self.changed_tabs.insert(id);
        }
        if self.shown == Some(id) {
            self.shown = None;
        }
        self.set_notice(format!("The page could not open: {error}"));
    }

    // ---------- Page events ----------

    pub fn title_changed(&mut self, id: TabId, title: &str) {
        let title: String = title.trim().chars().take(TITLE_LIMIT).collect();
        let Some(tab) = self.tab_mut(id) else { return };
        let display = if title.is_empty() { host_label(&tab.url) } else { title.clone() };
        let tab_changed = tab.title != display;
        if tab_changed { tab.title = display; }
        let url = tab.url.clone();
        if tab_changed { self.changed_tabs.insert(id); }
        if let Some(last) = self.history.last_mut().filter(|h| h.url == url && h.title != title) {
            last.title = title;
            let lists = self.history_lists(&url, None);
            self.library_changed(lists);
        }
    }

    /// History changed for `url`, and `dropped` (the oldest visit) may have
    /// left it. Bookmark rows show the newest non-empty title from history, so
    /// that list is rebuilt and resent only when a bookmarked row can change.
    fn history_lists(&self, url: &str, dropped: Option<&HistoryEntry>) -> u8 {
        let bookmarked = |u: &str| self.bookmarks.iter().any(|b| b == u);
        // A dropped visit mattered only if it was that page's last titled one.
        let lost_title = dropped.is_some_and(|d| {
            !d.title.is_empty() && bookmarked(&d.url) && !self.history.iter().any(|h| h.url == d.url && !h.title.is_empty())
        });
        if lost_title || bookmarked(url) { BOOKMARKS | HISTORY } else { HISTORY }
    }

    pub fn load_started(&mut self, id: TabId, url: &str) {
        if let Some(tab) = self.tab_mut(id) {
            if !tab.loading || tab.crashed || tab.url != url {
                tab.loading = true;
                tab.crashed = false;
                if tab.url != url { tab.url = url.into(); }
                self.changed_tabs.insert(id);
            }
        }
    }

    pub fn load_finished(&mut self, id: TabId, url: &str) {
        let Some(tab) = self.tab_mut(id) else { return };
        let tab_changed = tab.loading || tab.url != url;
        tab.loading = false;
        if tab.url != url { tab.url = url.into(); }
        let title = tab.title.clone();
        if tab_changed { self.changed_tabs.insert(id); }
        if self.settings.keep_history && (url.starts_with("http://") || url.starts_with("https://")) {
            let dropped = (self.history.len() >= crate::HISTORY_LIMIT).then(|| self.history[0].clone());
            push_history(&mut self.history, url, &title);
            let lists = self.history_lists(url, dropped.as_ref());
            self.library_changed(lists);
        } else {
            self.changed();
        }
    }

    pub fn history_changed(&mut self, id: TabId, can_back: bool, can_forward: bool, url: &str) {
        let Some(tab) = self.tab_mut(id) else { return };
        tab.can_back = can_back;
        tab.can_forward = can_forward;
        if !url.is_empty() && tab.url != url {
            tab.url = url.into();
            self.changed_tabs.insert(id);
            self.changed();
        }
    }

    /// Returns the tab ids whose webviews the shell must discard.
    pub fn process_failed(&mut self, id: TabId, failure: ProcessFailure) -> Vec<TabId> {
        match failure {
            ProcessFailure::RendererExited | ProcessFailure::RendererUnresponsive => {
                if let Some(tab) = self.tab_mut(id) {
                    tab.crashed = true;
                    tab.loading = false;
                    self.changed_tabs.insert(id);
                }
                if id == self.active {
                    self.set_notice(if failure == ProcessFailure::RendererExited { notices::CRASHED } else { notices::UNRESPONSIVE });
                }
                Vec::new()
            }
            ProcessFailure::BrowserExited => {
                let lost: Vec<TabId> = self.tabs.iter().filter(|t| t.has_view).map(|t| t.id).collect();
                for tab in &mut self.tabs {
                    if tab.loading || tab.crashed { self.changed_tabs.insert(tab.id); }
                    tab.has_view = false;
                    tab.loading = false;
                    tab.crashed = false;
                }
                self.shown = None;
                self.fullscreen = None;
                if !lost.is_empty() {
                    self.set_notice(notices::RESTARTED);
                }
                lost
            }
        }
    }

    /// Reload clears the crash flag; the shell reloads the webview.
    pub fn reload(&mut self) -> Option<TabId> {
        let id = self.active;
        let tab = self.tab_mut(id)?;
        let has_view = tab.has_view;
        if tab.crashed {
            tab.crashed = false;
            self.changed_tabs.insert(id);
        }
        has_view.then_some(id)
    }

    /// A page entered or left HTML fullscreen. Returns whether the window
    /// fullscreen state must change.
    pub fn fullscreen_changed(&mut self, id: TabId, on: bool) -> bool {
        let before = self.fullscreen.is_some();
        if on && id == self.active {
            self.fullscreen = Some(id);
        } else if self.fullscreen == Some(id) {
            self.fullscreen = None;
        }
        before != self.fullscreen.is_some()
    }

    /// Switching tabs or opening settings ends a page's fullscreen. Returns the page to tell.
    pub fn leave_fullscreen_if_inactive(&mut self) -> Option<TabId> {
        let id = self.fullscreen.filter(|&id| id != self.active || self.settings_open)?;
        self.fullscreen = None;
        Some(id)
    }

    // ---------- Memory ----------

    /// The shell checked a page as it was hidden. Ignored for a page that is
    /// visible again, or for a view that has since been replaced.
    pub fn hidden_page_checked(&mut self, id: TabId, generation: u64, releasable: bool) {
        if self.shown == Some(id) || !self.view_is_current(id, generation) {
            return;
        }
        if let Some(tab) = self.tab_mut(id) {
            tab.releasable = releasable;
        }
    }

    /// The least recently shown page that may be released under memory
    /// pressure: hidden, checked as having nothing to lose, not the active or
    /// fullscreen tab, and not the source of a running download.
    pub fn release_candidate(&self) -> Option<(TabId, u64)> {
        self.tabs
            .iter()
            .filter(|t| self.may_release(t))
            .min_by_key(|t| t.last_shown)
            .map(|t| (t.id, t.view_generation))
    }

    fn may_release(&self, tab: &Tab) -> bool {
        tab.has_view
            && tab.releasable
            && !tab.crashed
            && tab.id != self.active
            && self.shown != Some(tab.id)
            && self.fullscreen != Some(tab.id)
            && !self.downloads.iter().any(|d| d.state == "InProgress" && d.tab == Some(tab.id))
    }

    /// Forgets the tab's view; the shell then closes it. The tab keeps its
    /// address and title and loads again when selected. Back/forward history
    /// is lost, as WebView2 cannot hand it to a new view.
    pub fn release(&mut self, id: TabId, generation: u64) -> bool {
        let allowed = self.view_is_current(id, generation) && self.tab(id).is_some_and(|t| self.may_release(t));
        let Some(tab) = self.tab_mut(id).filter(|_| allowed) else { return false };
        tab.has_view = false;
        tab.releasable = false;
        tab.can_back = false;
        tab.can_forward = false;
        if tab.loading {
            tab.loading = false;
            self.changed_tabs.insert(id);
        }
        true
    }

    // ---------- Library ----------

    pub fn toggle_bookmark(&mut self) -> Result<bool, &'static str> {
        let Some(url) = self.tab(self.active).map(|t| t.url.clone()).filter(|u| u != BLANK) else {
            self.set_notice(notices::NO_PAGE);
            return Err(notices::NO_PAGE);
        };
        let added = match self.bookmarks.iter().position(|x| x == &url) {
            Some(i) => {
                self.bookmarks.remove(i);
                self.bookmark_titles.remove(&url);
                false
            }
            None => {
                self.bookmarks.push(url);
                true
            }
        };
        self.library_changed(BOOKMARKS);
        Ok(added)
    }

    pub fn remove_bookmark(&mut self, url: &str) {
        let before = self.bookmarks.len();
        self.bookmarks.retain(|x| x != url);
        self.bookmark_titles.remove(url);
        if self.bookmarks.len() != before {
            self.library_changed(BOOKMARKS);
        }
    }

    /// `tab` is the page that started it, if known: that page is kept while it runs.
    pub fn download_started(&mut self, path: String, tab: Option<TabId>) {
        self.downloads.push(SavedDownload { path, state: "InProgress".into(), bytes: 0, total: 0, tab });
        trim_downloads(&mut self.downloads);
        self.library_changed(DOWNLOADS);
    }

    /// `size` reads the finished file's length (the shell passes a filesystem lookup).
    pub fn download_finished(&mut self, path: Option<String>, success: bool, size: impl Fn(&str) -> Option<i64>) {
        let index = self
            .downloads
            .iter()
            .rposition(|d| d.state == "InProgress" && path.as_ref().is_none_or(|p| &d.path == p))
            .or_else(|| self.downloads.iter().rposition(|d| d.state == "InProgress"));
        if let Some(d) = index.map(|i| &mut self.downloads[i]) {
            if let Some(p) = path {
                d.path = p;
            }
            d.state = if success { "Completed" } else { "Failed" }.into();
            d.bytes = size(&d.path).unwrap_or(d.bytes);
            d.total = d.bytes;
            // Active downloads may exceed the history cap. As they finish,
            // release old entries without waiting for another download/startup.
            trim_downloads(&mut self.downloads);
            self.library_changed(DOWNLOADS);
        }
    }

    /// The save dialog chose another file for a starting download.
    pub fn download_redirected(&mut self, from: &str, to: String) {
        if let Some(d) = self.downloads.iter_mut().rev().find(|d| d.state == "InProgress" && d.path == from) {
            d.path = to;
            self.library_changed(DOWNLOADS);
        }
    }

    /// The save dialog was cancelled, so the download never started.
    pub fn download_cancelled(&mut self, path: &str) {
        if let Some(i) = self.downloads.iter().rposition(|d| d.state == "InProgress" && d.path == path) {
            self.downloads.remove(i);
            self.library_changed(DOWNLOADS);
        }
    }

    /// Adds bookmarks and history from another browser. Bookmarks keep their
    /// order after the existing ones. Visits merge into history by time (one
    /// per page, skipping pages already visited as recently), and only the
    /// newest `HISTORY_LIMIT` entries remain. History is left alone while it
    /// is turned off. Returns how many bookmarks and visits were added.
    pub fn import_library(&mut self, links: Vec<Link>, visits: Vec<Visit>) -> (usize, usize) {
        let mut known: HashSet<String> = self.bookmarks.iter().cloned().collect();
        let mut bookmarks = 0;
        let mut titled = false;
        for link in links {
            if link.url.len() > URL_LIMIT {
                continue;
            }
            if known.insert(link.url.clone()) {
                self.bookmarks.push(link.url.clone());
                bookmarks += 1;
            }
            if !link.title.is_empty() && !self.bookmark_titles.contains_key(&link.url) {
                self.bookmark_titles.insert(link.url, link.title);
                titled = true;
            }
        }
        let visits = if self.settings.keep_history { visits } else { Vec::new() };
        let mut latest: HashMap<&str, i64> = HashMap::new();
        let mut time = 0;
        let mut dated: Vec<(i64, bool, HistoryEntry)> = Vec::with_capacity(self.history.len() + visits.len());
        for entry in &self.history {
            // An unreadable time keeps the entry where it was.
            time = parse_iso8601(&entry.visited_at).unwrap_or(time);
            let newest = latest.entry(entry.url.as_str()).or_insert(time);
            *newest = (*newest).max(time);
            dated.push((time, false, entry.clone()));
        }
        let fresh: Vec<(i64, bool, HistoryEntry)> = visits
            .into_iter()
            .filter(|v| v.url.len() <= URL_LIMIT && latest.get(v.url.as_str()).is_none_or(|&t| t < v.visited_at))
            .map(|v| {
                let at = v.visited_at.max(0);
                (at, true, HistoryEntry { url: v.url, visited_at: iso8601_utc(at as u64), title: v.title })
            })
            .collect();
        drop(latest);
        dated.extend(fresh);
        dated.sort_by_key(|(at, _, _)| *at);
        let start = dated.len().saturating_sub(HISTORY_LIMIT);
        let history = dated.iter().skip(start).filter(|(_, imported, _)| *imported).count();
        if history > 0 {
            self.history = dated.into_iter().skip(start).map(|(_, _, entry)| entry).collect();
            self.library_changed(BOOKMARKS | HISTORY);
        } else if bookmarks > 0 || titled {
            self.library_changed(BOOKMARKS);
        }
        (bookmarks, history)
    }

    // ---------- Saved logins ----------

    /// The logins read from `passwords.dat` at startup.
    pub fn set_vault(&mut self, vault: Vault) {
        self.vault = vault;
        self.library_dirty |= PASSWORDS;
    }

    /// Returns how many logins were added or changed.
    pub fn import_logins(&mut self, logins: Vec<Login>) -> usize {
        let count = self.vault.merge(logins);
        if count > 0 {
            self.vault_changed();
        }
        count
    }

    pub fn remove_login(&mut self, index: usize) {
        if self.vault.remove(index) {
            self.vault_changed();
        }
    }

    pub fn clear_logins(&mut self) {
        if self.vault.clear() {
            self.vault_changed();
        }
    }

    fn vault_changed(&mut self) {
        self.vault_dirty = true;
        self.library_dirty |= PASSWORDS;
    }

    /// The fill script for login `index`, only when it belongs on the active
    /// page; returns the tab it must run in.
    pub fn fill_login(&self, index: usize) -> Option<(TabId, String)> {
        let tab = self.tab(self.active)?;
        let page = url::Url::parse(&tab.url).ok()?;
        let login = self.vault.get(index).filter(|l| l.matches(&page))?;
        Some((tab.id, crate::passwords::fill_script(page.host_str()?, login)))
    }

    /// The vault to encrypt and write, when it changed since the last call.
    pub fn take_vault(&mut self) -> Option<String> {
        std::mem::take(&mut self.vault_dirty).then(|| self.vault.to_json())
    }

    /// Writing the vault failed; try again with the next save.
    pub fn vault_save_failed(&mut self) {
        self.vault_dirty = true;
        self.set_notice(notices::PASSWORDS_NOT_SAVED);
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
        self.library_changed(BOOKMARKS | HISTORY);
    }

    /// Only folders of recorded downloads may be opened.
    pub fn download_folder(&self, path: &str) -> Option<std::path::PathBuf> {
        self.downloads.iter().find(|d| d.path == path)?;
        Path::new(path).parent().map(Path::to_path_buf)
    }

    // ---------- Output ----------

    /// Full reads do not consume pending event deltas: a `ready` request must
    /// not swallow a change before the publisher has sent it to the interface.
    pub fn snapshot(&mut self, full: bool) -> Snapshot {
        self.snapshot_revision += 1;
        let library = if full { ALL_LIBRARY } else { std::mem::take(&mut self.library_dirty) };
        let send_settings = full || std::mem::take(&mut self.settings_dirty);
        let ordered = full || self.tabs_reordered;
        let tabs = ordered.then(|| self.tabs.iter().map(TabView::from).collect());
        let tab_changes = if ordered {
            Vec::new()
        } else {
            self.changed_tabs.iter().filter_map(|id| self.tab(*id)).map(TabView::from).collect()
        };
        if !full {
            self.tabs_reordered = false;
            self.changed_tabs.clear();
        }
        let active = self.tab(self.active);
        let url = active.map(|t| t.url.clone()).filter(|u| u != BLANK).unwrap_or_default();
        let bookmarks = (library & BOOKMARKS != 0).then(|| {
            let titles: HashMap<&str, &str> = self.history.iter().filter(|h| !h.title.is_empty())
                .map(|h| (h.url.as_str(), h.title.as_str())).collect();
            self.bookmarks.iter().rev().map(|u| LinkView {
                url: u.clone(),
                title: titles.get(u.as_str()).copied().or_else(|| self.bookmark_titles.get(u).map(String::as_str)).unwrap_or_default().to_owned(),
            }).collect()
        });
        let history = (library & HISTORY != 0).then(|| {
            let mut seen = HashSet::new();
            self.history.iter().rev().filter(|h| seen.insert(h.url.as_str())).take(30)
                .map(|h| LinkView { url: h.url.clone(), title: h.title.clone() }).collect()
        });
        let downloads = (library & DOWNLOADS != 0).then(|| {
            self.downloads.iter().rev().map(|d| DownloadView {
                name: Path::new(&d.path.replace('\\', "/")).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                path: d.path.clone(), state: d.state.clone(), bytes: d.bytes,
            }).collect()
        });
        let logins = if url.is_empty() || self.vault.is_empty() { Vec::new() } else { self.vault.for_page(&url) };
        Snapshot {
            revision: self.snapshot_revision,
            tabs,
            tab_changes,
            active: self.active,
            loading: active.is_some_and(|t| t.loading),
            can_back: active.is_some_and(|t| t.can_back),
            can_forward: active.is_some_and(|t| t.can_forward),
            bookmarked: !url.is_empty() && self.bookmarks.contains(&url),
            url,
            sidebar_open: self.sidebar_open,
            fullscreen: self.fullscreen.is_some(),
            bookmarks,
            history,
            downloads,
            logins,
            saved_logins: (library & PASSWORDS != 0).then(|| self.vault.views()),
            downloading: self.downloads.iter().filter(|d| d.state == "InProgress").count(),
            notice: self.notice.clone(),
            settings_open: self.settings_open,
            settings: send_settings.then(|| self.settings.clone()),
        }
    }

    // ---------- Settings ----------

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn settings_open(&self) -> bool {
        self.settings_open
    }

    pub fn set_settings_open(&mut self, open: bool) {
        self.settings_open = open;
    }

    /// Applies the fields in `patch`; returns the previous settings so the
    /// shell can tell what changed.
    pub fn update_settings(&mut self, patch: &serde_json::Map<String, serde_json::Value>) -> Settings {
        let previous = self.settings.clone();
        self.settings.merge(patch);
        if self.settings != previous {
            self.settings_dirty = true;
            self.changed();
        }
        previous
    }

    pub fn set_sidebar(&mut self, open: bool) {
        if self.sidebar_open != open {
            self.sidebar_open = open;
            self.changed();
        }
    }

    /// Compatibility helper for tests and consumers that want serialized state.
    pub fn take_save(&mut self) -> Option<String> {
        self.take_state().map(|state| serde_json::to_string(&state).expect("state serializes"))
    }

    /// Capture owned data under the session lock; encode JSON after releasing it.
    pub fn take_state(&mut self) -> Option<BrowserState> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        Some(BrowserState {
            version: SCHEMA_VERSION,
            sidebar_open: self.sidebar_open,
            tabs: self.tabs.iter().map(|t| SavedTab { url: t.url.clone() }).collect(),
            active_index: self.tab_indices.get(&self.active).copied().unwrap_or(0),
            bookmarks: self.bookmarks.clone(),
            bookmark_titles: self.bookmark_titles.clone(),
            history: self.history.clone(),
            downloads: self.downloads.clone(),
            window: self.window,
            settings: self.settings.clone(),
        })
    }

    pub fn save_failed(&mut self) {
        self.dirty = true;
        self.set_notice(notices::SAVE_FAILED);
    }

}

/// Only http(s) URLs from the command line open; flags and other schemes are ignored.
pub fn launch_url(arg: &str) -> Option<String> {
    let parsed = url::Url::parse(arg.trim()).ok()?;
    matches!(parsed.scheme(), "http" | "https").then(|| parsed.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: Bounds = Bounds { x: 8.0, y: 48.0, width: 1000.0, height: 700.0 };

    fn session(json: &str) -> Session {
        Session::load(Some(Ok(json.into())), &[]).0
    }

    fn fresh() -> Session {
        Session::load(None, &[]).0
    }

    #[test]
    fn deltas_reconstruct_the_full_tab_list_through_lifecycle_events() {
        let mut s = fresh();
        let mut rows = s.snapshot(false).tabs.unwrap();
        for i in 0..500 {
            match i % 10 {
                0 => { s.open_tab("https://example.org/"); }
                1 => { s.navigate("https://example.org/page").unwrap(); }
                2 => s.load_started(s.active(), "https://example.org/page"),
                3 => s.title_changed(s.active(), "Changed title"),
                4 => s.load_finished(s.active(), "https://example.org/page"),
                5 => { s.process_failed(s.active(), ProcessFailure::RendererExited); }
                6 => { s.reload(); }
                7 => { s.cycle(false); s.history_changed(s.active(), true, false, "https://example.org/#new"); }
                8 => { s.close(s.active()); }
                _ => { s.process_failed(s.active(), ProcessFailure::BrowserExited); }
            }
            // A concurrent full read must not consume the pending event delta.
            let full = s.snapshot(true);
            let delta = s.snapshot(false);
            assert!(delta.revision > full.revision);
            if let Some(tabs) = delta.tabs { rows = tabs; }
            for tab in delta.tab_changes {
                let row = rows.iter_mut().find(|row| row.id == tab.id).expect("existing row");
                *row = tab;
            }
            assert_eq!(rows, full.tabs.unwrap(), "event {i}");
            for (index, tab) in s.tabs.iter().enumerate() {
                assert_eq!(s.tab_indices.get(&tab.id), Some(&index));
                assert_eq!(s.tab(tab.id), Some(tab));
            }
            assert_eq!(s.tab_indices.len(), s.tabs.len());
        }
    }

    #[test]
    fn one_title_update_is_bounded_even_with_ten_thousand_tabs() {
        let mut s = fresh();
        for _ in 1..10_000 { s.open_tab(BLANK); }
        s.snapshot(false);
        let id = s.active();
        for _ in 0..50 { s.title_changed(id, "Latest title"); }
        let update = s.snapshot(false);
        assert!(update.tabs.is_none());
        assert_eq!(update.tab_changes.len(), 1);
        assert!(serde_json::to_vec(&update).unwrap().len() < 1_000);
        s.title_changed(id, "Latest title");
        s.cycle(false);
        let update = s.snapshot(false);
        assert!(update.tabs.is_none() && update.tab_changes.is_empty());
        assert!(update.bookmarks.is_none() && update.history.is_none() && update.downloads.is_none());
    }

    #[test]
    fn page_loads_resend_bookmarks_only_when_a_bookmarked_page_changes() {
        let mut s = session(r#"{"Tabs":[{"Url":"https://a.test/"}],"Bookmarks":["https://saved.test/"]}"#);
        s.snapshot(false);
        let id = s.active();
        s.load_finished(id, "https://other.test/");
        s.title_changed(id, "Other");
        let update = s.snapshot(false);
        assert!(update.history.is_some() && update.bookmarks.is_none(), "an unbookmarked page leaves bookmarks alone");
        s.load_finished(id, "https://saved.test/");
        assert!(s.snapshot(false).bookmarks.is_some(), "a bookmarked page may change its row title");
        s.title_changed(id, "Saved page");
        assert_eq!(s.snapshot(false).bookmarks.unwrap()[0].title, "Saved page");
        for i in 0..crate::HISTORY_LIMIT - 1 {
            s.load_finished(id, &format!("https://filler.test/{i}"));
        }
        s.snapshot(false);
        s.load_finished(id, "https://filler.test/last");
        let update = s.snapshot(false);
        assert_eq!(update.bookmarks.unwrap()[0].title, "", "dropping the bookmark's last visit resends its row");

        let mut s = session(r#"{"Tabs":[{"Url":"https://a.test/"}],"Bookmarks":["https://saved.test/"],
            "History":[{"Url":"https://saved.test/","Title":"Old"},{"Url":"https://saved.test/","Title":"New"}]}"#);
        for i in 0..crate::HISTORY_LIMIT - 2 {
            s.load_finished(s.active(), &format!("https://filler.test/{i}"));
        }
        s.snapshot(false);
        s.load_finished(s.active(), "https://filler.test/last");
        assert!(s.snapshot(false).bookmarks.is_none(), "another titled visit still names the bookmark");
    }

    #[test]
    fn settings_are_sent_only_when_they_change() {
        let mut s = fresh();
        assert!(s.snapshot(false).settings.is_some(), "the first update carries settings");
        assert!(s.snapshot(false).settings.is_none());
        assert!(s.snapshot(true).settings.is_some(), "full snapshots always carry settings");
        s.update_settings(&patch(serde_json::json!({"Theme": "Light"})));
        assert_eq!(s.snapshot(false).settings.unwrap().theme, crate::settings::Theme::Light);
        s.update_settings(&patch(serde_json::json!({"Theme": "Light"})));
        assert!(s.snapshot(false).settings.is_none(), "an unchanged patch sends nothing");
    }

    #[test]
    fn closing_most_tabs_releases_table_capacity() {
        let mut s = fresh();
        let ids: Vec<_> = (0..5000).map(|i| s.open_tab(&format!("https://t{i}.test/"))).collect();
        s.snapshot(false);
        let peak = s.tabs.capacity();
        for id in &ids[10..] {
            s.close(*id);
        }
        assert!(s.tabs.capacity() < peak / 8, "{} of {peak}", s.tabs.capacity());
        assert!(s.tab_indices.capacity() <= 256);
        for (index, tab) in s.tabs.iter().enumerate() {
            assert_eq!(s.tab_indices.get(&tab.id), Some(&index));
        }
    }

    /// Shows each tab once (a, b, c), leaving c active and a, b hidden with views.
    fn three_shown_tabs() -> (Session, [TabId; 3]) {
        let mut s = session(r#"{"Tabs":[{"Url":"https://a.test/"},{"Url":"https://b.test/"},{"Url":"https://c.test/"}],"ActiveIndex":0}"#);
        let ids = [s.tabs()[0].id, s.tabs()[1].id, s.tabs()[2].id];
        s.set_bounds(B);
        for id in ids {
            s.select(id);
            s.plan_show(false);
        }
        (s, ids)
    }

    fn generation(s: &Session, id: TabId) -> u64 {
        s.tab(id).unwrap().view_generation
    }

    #[test]
    fn only_checked_hidden_pages_are_released_least_recent_first() {
        let (mut s, [a, b, c]) = three_shown_tabs();
        assert_eq!(s.release_candidate(), None, "nothing is releasable until checked");
        s.hidden_page_checked(b, generation(&s, b), true);
        s.hidden_page_checked(a, generation(&s, a), true);
        s.hidden_page_checked(c, generation(&s, c), true);
        assert!(!s.tab(c).unwrap().releasable, "the visible page ignores a late check");
        assert_eq!(s.release_candidate(), Some((a, generation(&s, a))), "a was shown longest ago");
        assert!(s.release(a, generation(&s, a)));
        assert!(!s.tab(a).unwrap().has_view);
        assert_eq!(s.release_candidate().map(|c| c.0), Some(b));
        assert!(!s.release(a, generation(&s, a)), "already released");
        // Selecting a released tab creates a fresh view at its address.
        s.select(a);
        let plan = s.plan_show(false);
        assert!(matches!(plan.action, ShowAction::Create { id, ref url, .. } if id == a && url == "https://a.test/"));
        assert_eq!(plan.hide, Some(c));
        assert!(!s.tab(a).unwrap().releasable, "shown again: must be re-checked");
    }

    #[test]
    fn active_edited_downloading_and_fullscreen_pages_are_kept() {
        let (mut s, [a, b, c]) = three_shown_tabs();
        s.hidden_page_checked(a, generation(&s, a), false);
        s.hidden_page_checked(b, generation(&s, b), true);
        assert_eq!(s.release_candidate().map(|c| c.0), Some(b), "a holds edits");
        s.download_started("big.iso".into(), Some(b));
        assert_eq!(s.release_candidate(), None, "b is downloading");
        s.download_finished(Some("big.iso".into()), true, |_| Some(1));
        assert_eq!(s.release_candidate().map(|c| c.0), Some(b));
        let old = generation(&s, b);
        s.select(b);
        s.plan_show(false);
        assert_eq!(s.release_candidate(), None, "b is visible and c was never checked");
        assert!(!s.release(b, old), "the active page is never released");
        s.hidden_page_checked(c, generation(&s, c), true);
        s.fullscreen_changed(b, true);
        s.select(c);
        assert_eq!(s.release_candidate(), None, "b is still the fullscreen page until the shell ends it");
        s.leave_fullscreen_if_inactive();
        s.plan_show(false);
        assert_eq!(s.release_candidate().map(|c| c.0), None, "b was shown again, so it is unchecked");
        s.process_failed(c, ProcessFailure::BrowserExited);
        assert_eq!(s.release_candidate(), None, "no views left");
    }

    #[test]
    fn checks_from_a_replaced_view_are_ignored() {
        let (mut s, [a, ..]) = three_shown_tabs();
        let old = generation(&s, a);
        s.process_failed(a, ProcessFailure::BrowserExited);
        s.hidden_page_checked(a, old, true);
        assert!(!s.tab(a).unwrap().releasable);
        assert!(!s.release(a, old));
    }

    #[test]
    fn library_updates_do_not_resend_unrelated_lists() {
        let mut s = fresh();
        s.snapshot(false);
        s.download_started("file.zip".into(), None);
        let update = s.snapshot(false);
        assert!(update.downloads.is_some() && update.bookmarks.is_none() && update.history.is_none());
        s.navigate("https://example.org").unwrap();
        s.toggle_bookmark().unwrap();
        let update = s.snapshot(false);
        assert!(update.bookmarks.is_some() && update.history.is_none() && update.downloads.is_none());
        s.load_finished(s.active(), "https://example.org/");
        let update = s.snapshot(false);
        assert!(update.history.is_some() && update.downloads.is_none());
    }

    #[test]
    fn save_capture_is_owned_and_new_changes_remain_pending() {
        let mut s = fresh();
        s.open_tab("https://first.test/");
        let saved = s.take_state().unwrap();
        s.navigate("https://second.test/").unwrap();
        assert_eq!(saved.tabs.last().unwrap().url, "https://first.test/");
        let latest = s.take_state().unwrap();
        assert_eq!(latest.tabs.last().unwrap().url, "https://second.test/");
        assert!(s.take_state().is_none());
        s.save_failed();
        assert_eq!(s.take_state().unwrap(), latest);
    }

    #[test]
    fn closed_or_replaced_views_cannot_deliver_late_events() {
        let mut s = fresh();
        s.navigate("https://example.org/").unwrap();
        s.set_bounds(B);
        let ShowAction::Create { id, generation, .. } = s.plan_show(false).action else { panic!("create") };
        assert!(s.view_is_current(id, generation));
        s.process_failed(id, ProcessFailure::BrowserExited);
        assert!(!s.view_is_current(id, generation));
        let ShowAction::Create { generation: next, .. } = s.plan_show(false).action else { panic!("recreate") };
        assert!(s.view_is_current(id, next));
        assert!(!s.view_is_current(id, generation), "old process events must not destroy the replacement");
        s.close(id);
        assert!(!s.view_is_current(id, next), "late creation must be closed, not retained");
    }

    #[test]
    fn fresh_session_has_one_blank_tab_and_nothing_to_show() {
        let mut s = fresh();
        assert_eq!(s.tabs().len(), 1);
        s.set_bounds(B);
        assert_eq!(s.plan_show(false), ShowPlan { hide: None, action: ShowAction::None });
        assert_eq!(s.take_save(), None, "nothing changed yet");
    }

    #[test]
    fn restore_is_lazy_and_switching_hides_only_the_previous_page() {
        let mut s = session(r#"{"Tabs":[{"Url":"https://a.test/"},{"Url":"https://b.test/"},{"Url":"https://c.test/"}],"ActiveIndex":1}"#);
        let ids: Vec<TabId> = s.tabs().iter().map(|t| t.id).collect();
        assert_eq!(s.active(), ids[1]);
        assert_eq!(s.plan_show(false).action, ShowAction::None, "no geometry yet");
        s.set_bounds(B);
        assert_eq!(s.plan_show(false), ShowPlan { hide: None, action: ShowAction::Create { id: ids[1], generation: 1, url: "https://b.test/".into(), bounds: B } });
        assert!(!s.tab(ids[0]).unwrap().has_view && !s.tab(ids[2]).unwrap().has_view, "background tabs stay unloaded");
        assert_eq!(s.plan_show(false).action, ShowAction::Resize { id: ids[1], bounds: B });
        s.select(ids[2]);
        let plan = s.plan_show(false);
        assert_eq!(plan.hide, Some(ids[1]));
        assert!(matches!(plan.action, ShowAction::Create { id, .. } if id == ids[2]));
        s.select(ids[1]);
        assert_eq!(s.plan_show(false), ShowPlan { hide: Some(ids[2]), action: ShowAction::Reveal { id: ids[1], bounds: B } });
        assert_eq!(s.plan_show(true).action, ShowAction::Reveal { id: ids[1], bounds: B }, "force re-applies geometry");
    }

    #[test]
    fn closing_picks_the_neighbour_and_never_leaves_zero_tabs() {
        let mut s = session(r#"{"Tabs":[{"Url":"https://a.test/"},{"Url":"https://b.test/"}],"ActiveIndex":1}"#);
        let [a, b] = [s.tabs()[0].id, s.tabs()[1].id];
        s.set_bounds(B);
        s.plan_show(false);
        assert_eq!(s.close(b), Some(true), "b had a webview");
        assert_eq!(s.active(), a);
        let plan = s.plan_show(false);
        assert_eq!(plan.hide, None, "closed page is not hidden twice");
        assert!(matches!(plan.action, ShowAction::Create { id, .. } if id == a), "neighbour loads lazily now");
        assert_eq!(s.close(a), Some(true));
        assert_eq!(s.tabs().len(), 1);
        assert_eq!(s.tabs()[0].url, BLANK);
        assert_eq!(s.close(999), None);
    }

    #[test]
    fn cycling_wraps_both_ways() {
        let mut s = fresh();
        let first = s.active();
        let second = s.open_tab(BLANK);
        assert_eq!(s.cycle(true), first);
        assert_eq!(s.cycle(false), second);
    }

    #[test]
    fn navigation_validates_and_routes_to_existing_or_new_webview() {
        let mut s = fresh();
        assert_eq!(s.navigate("   "), Ok(None));
        assert_eq!(s.navigate("example.org"), Ok(Some(Navigation::Create)));
        s.set_bounds(B);
        assert!(matches!(s.plan_show(false).action, ShowAction::Create { .. }));
        let id = s.active();
        assert_eq!(s.navigate("rust lang"), Ok(Some(Navigation::Existing { id, url: "https://www.google.com/search?q=rust%20lang".into() })));
        assert_eq!(s.navigate(&"x".repeat(40_000)), Err(notices::BAD_ADDRESS));
        assert_eq!(s.snapshot(false).notice.as_deref(), Some(notices::BAD_ADDRESS));
    }

    #[test]
    fn page_events_update_tab_and_history() {
        let mut s = fresh();
        s.navigate("https://a.test/").unwrap();
        let id = s.active();
        s.load_started(id, "https://a.test/");
        s.title_changed(id, &"T".repeat(5000));
        assert_eq!(s.tab(id).unwrap().title.chars().count(), TITLE_LIMIT);
        s.load_finished(id, "https://a.test/");
        s.title_changed(id, "A page");
        let snap = s.snapshot(true);
        assert!(!snap.loading);
        assert_eq!(snap.history.unwrap()[0], LinkView { url: "https://a.test/".into(), title: "A page".into() });
        s.title_changed(id, "  ");
        assert_eq!(s.tab(id).unwrap().title, "a.test", "empty titles fall back to the host");
        s.load_finished(id, "about:blank");
        assert_eq!(s.snapshot(true).history.unwrap().len(), 1, "non-web URLs are not history");
        s.history_changed(id, true, false, "https://a.test/#x");
        assert!(s.snapshot(false).can_back);
    }

    #[test]
    fn library_lists_are_sent_only_when_changed() {
        let mut s = fresh();
        assert!(s.snapshot(false).bookmarks.is_some(), "first update carries everything");
        assert!(s.snapshot(false).bookmarks.is_none());
        assert_eq!(s.toggle_bookmark(), Err(notices::NO_PAGE));
        s.navigate("https://a.test/").unwrap();
        assert_eq!(s.toggle_bookmark(), Ok(true));
        let snap = s.snapshot(false);
        assert!(snap.bookmarked && snap.bookmarks.is_some());
        assert!(s.snapshot(true).bookmarks.is_some(), "full snapshots always carry lists");
        assert_eq!(s.toggle_bookmark(), Ok(false));
        s.remove_bookmark("https://never.test/");
    }

    #[test]
    fn downloads_match_by_path_and_folders_are_restricted() {
        let mut s = fresh();
        s.download_started(r"C:\Users\me\Downloads\a.zip".into(), None);
        s.download_started(r"C:\Users\me\Downloads\b.zip".into(), None);
        s.download_finished(Some(r"C:\Users\me\Downloads\a.zip".into()), true, |_| Some(2048));
        let list = s.snapshot(true).downloads.unwrap();
        assert_eq!((list[1].name.as_str(), list[1].state.as_str(), list[1].bytes), ("a.zip", "Completed", 2048));
        assert_eq!(list[0].state, "InProgress");
        s.download_finished(None, false, |_| None);
        assert_eq!(s.snapshot(true).downloads.unwrap()[0].state, "Failed");
        assert!(s.download_folder(r"C:\Users\me\Downloads\a.zip").is_some());
        assert!(s.download_folder(r"C:\Windows\System32\x.exe").is_none());
    }

    #[test]
    fn completed_download_bursts_return_to_the_history_limit() {
        let mut s = fresh();
        for i in 0..crate::DOWNLOAD_LIMIT + 50 { s.download_started(format!("file-{i}.zip"), None); }
        assert_eq!(s.downloads.len(), crate::DOWNLOAD_LIMIT + 50, "keep every active download");
        for i in 0..crate::DOWNLOAD_LIMIT + 50 { s.download_finished(Some(format!("file-{i}.zip")), true, |_| Some(10)); }
        assert_eq!(s.downloads.len(), crate::DOWNLOAD_LIMIT);
        assert!(s.downloads.iter().all(|d| d.state == "Completed"));
    }

    #[test]
    fn renderer_crash_marks_the_tab_and_reload_recovers() {
        let mut s = fresh();
        s.navigate("https://a.test/").unwrap();
        s.set_bounds(B);
        s.plan_show(false);
        let id = s.active();
        assert!(s.process_failed(id, ProcessFailure::RendererExited).is_empty());
        let snap = s.snapshot(false);
        assert!(snap.tabs.unwrap()[0].crashed);
        assert_eq!(snap.notice.as_deref(), Some(notices::CRASHED));
        assert_eq!(s.reload(), Some(id));
        assert!(!s.tab(id).unwrap().crashed);
    }

    #[test]
    fn browser_process_exit_discards_every_view_and_recreates_the_active_one() {
        let mut s = session(r#"{"Tabs":[{"Url":"https://a.test/"},{"Url":"https://b.test/"}]}"#);
        s.set_bounds(B);
        s.plan_show(false);
        s.cycle(true);
        s.plan_show(false);
        let lost = s.process_failed(s.active(), ProcessFailure::BrowserExited);
        assert_eq!(lost.len(), 2);
        assert!(matches!(s.plan_show(false), ShowPlan { hide: None, action: ShowAction::Create { .. } }));
    }

    #[test]
    fn fullscreen_uses_the_window_and_ends_when_switching() {
        let mut s = fresh();
        s.navigate("https://video.test/").unwrap();
        s.set_bounds(B);
        let window = Bounds { x: 0.0, y: 0.0, width: 1440.0, height: 900.0 };
        s.set_window_bounds(window);
        s.plan_show(false);
        let video = s.active();
        assert!(s.fullscreen_changed(video, true));
        assert_eq!(s.plan_show(true).action, ShowAction::Reveal { id: video, bounds: window });
        assert!(!s.fullscreen_changed(video, true), "no change when already fullscreen");
        s.open_tab(BLANK);
        assert_eq!(s.leave_fullscreen_if_inactive(), Some(video));
        assert!(!s.is_fullscreen());
        assert!(!s.fullscreen_changed(video, false));
        let other = s.active();
        assert!(!s.fullscreen_changed(other, true) || s.is_fullscreen());
    }

    #[test]
    fn view_creation_failure_is_reported_and_retried_on_next_show() {
        let mut s = fresh();
        s.navigate("https://a.test/").unwrap();
        s.set_bounds(B);
        let id = s.active();
        s.plan_show(false);
        s.view_failed(id, "boom");
        assert!(s.snapshot(false).notice.unwrap().contains("boom"));
        assert!(matches!(s.plan_show(false).action, ShowAction::Create { .. }));
    }

    #[test]
    fn load_outcomes_and_launch_urls() {
        assert_eq!(Session::load(None, &[]).1, LoadOutcome::Fresh);
        assert_eq!(Session::load(Some(Err(())), &[]).1, LoadOutcome::Unreadable);
        let (s, outcome) = Session::load(Some(Ok("{oops".into())), &[]);
        assert_eq!(outcome, LoadOutcome::Corrupt);
        assert_eq!(s.notice.as_deref(), Some(notices::CORRUPT));
        let (mut s, outcome) = Session::load(Some(Ok(r#"{"Version":9,"Tabs":[{"Url":"https://a.test/"}]}"#.into())), &[]);
        assert_eq!(outcome, LoadOutcome::Newer(9));
        assert!(s.take_save().unwrap().contains("\"Version\":2"));

        let args = ["--flag".to_string(), "file:///c:/x".into(), "https://open.test/".into(), "javascript:alert(1)".into()];
        let (s, _) = Session::load(None, &args);
        assert_eq!(s.tabs().len(), 2);
        assert_eq!(s.tab(s.active()).unwrap().url, "https://open.test/");
        let mut s = fresh();
        assert!(!s.open_launch_urls(&["still.exe".into()]));
        assert!(s.open_launch_urls(&["still.exe".into(), "http://x.test".into()]));
    }

    #[test]
    fn saves_round_trip_through_the_parser_and_track_window_placement() {
        let mut s = session(r#"{"Tabs":[{"Url":"https://a.test/"}],"Bookmarks":["https://a.test/"]}"#);
        let placement = WindowPlacement { x: -1200, y: 40, width: 1280, height: 800, maximized: true };
        s.set_window_placement(placement);
        s.set_sidebar(true);
        let json = s.take_save().expect("dirty");
        assert_eq!(s.take_save(), None, "taken once");
        let state = BrowserState::parse(&json).unwrap();
        assert_eq!(state.version, SCHEMA_VERSION);
        assert_eq!(state.window, Some(placement));
        assert!(state.sidebar_open);
        assert_eq!(state.bookmarks, vec!["https://a.test/"]);
        s.save_failed();
        assert!(s.take_save().is_some(), "failed saves are retried");
    }

    fn patch(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn startup_setting_chooses_the_first_tabs() {
        let tabs = r#""Tabs":[{"Url":"https://a.test/"},{"Url":"https://b.test/"}],"ActiveIndex":1"#;
        let s = session(&format!(r#"{{{tabs},"Settings":{{"Startup":"Restore"}}}}"#));
        assert_eq!((s.tabs().len(), s.tab(s.active()).unwrap().url.as_str()), (2, "https://b.test/"));
        let s = session(&format!(r#"{{{tabs},"Settings":{{"Startup":"NewTab"}}}}"#));
        assert_eq!(s.tabs().iter().map(|t| t.url.as_str()).collect::<Vec<_>>(), [BLANK]);
        let s = session(&format!(r#"{{{tabs},"Settings":{{"Startup":"Home","HomePage":"https://home.test/"}}}}"#));
        assert_eq!(s.tabs().iter().map(|t| t.url.as_str()).collect::<Vec<_>>(), ["https://home.test/"]);
        let s = session(&format!(r#"{{{tabs},"Settings":{{"Startup":"Home"}}}}"#));
        assert_eq!(s.tabs()[0].url, BLANK, "no home page set");
        let json = format!(r#"{{{tabs},"Settings":{{"Startup":"NewTab"}}}}"#);
        let (s, _) = Session::load(Some(Ok(json)), &["still.exe".into(), "https://open.test/".into()]);
        assert_eq!(s.tabs().iter().map(|t| t.url.as_str()).collect::<Vec<_>>(), [BLANK, "https://open.test/"]);
        assert_eq!(s.tab(s.active()).unwrap().url, "https://open.test/", "launch URLs still open");
    }

    #[test]
    fn sidebar_startup_overrides_the_remembered_state() {
        assert!(session(r#"{"SidebarOpen":true}"#).sidebar_open);
        assert!(!session(r#"{"SidebarOpen":true,"Settings":{"Sidebar":"Closed"}}"#).sidebar_open);
        assert!(session(r#"{"SidebarOpen":false,"Settings":{"Sidebar":"Open"}}"#).sidebar_open);
    }

    #[test]
    fn search_engine_and_history_settings_apply_to_navigation() {
        let mut s = fresh();
        let previous = s.update_settings(&patch(serde_json::json!({"Search": "DuckDuckGo", "KeepHistory": false})));
        assert_eq!(previous, Settings::default());
        assert!(s.take_save().unwrap().contains(r#""Search":"DuckDuckGo""#));
        s.update_settings(&patch(serde_json::json!({"Search": "DuckDuckGo"})));
        assert_eq!(s.take_save(), None, "unchanged settings are not saved again");
        s.navigate("still browser").unwrap();
        let id = s.active();
        assert_eq!(s.tab(id).unwrap().url, "https://duckduckgo.com/?q=still%20browser");
        s.load_finished(id, "https://duckduckgo.com/?q=still%20browser");
        assert!(s.snapshot(true).history.unwrap().is_empty(), "history is off");
        s.update_settings(&patch(serde_json::json!({"KeepHistory": true})));
        s.load_finished(id, "https://a.test/");
        assert_eq!(s.snapshot(true).history.unwrap().len(), 1);
        s.clear_history();
        assert!(s.snapshot(true).history.unwrap().is_empty());
    }

    #[test]
    fn import_notices_name_what_arrived() {
        let kinds = |b, h, c| [(b, "bookmark", "bookmarks"), (h, "history entry", "history entries"), (c, "cookie", "cookies")];
        assert_eq!(notices::imported(&kinds(3, 1, 0), Some("Firefox")).unwrap(), "Imported 3 bookmarks and 1 history entry from Firefox.");
        assert_eq!(notices::imported(&kinds(1, 2, 3), None).unwrap(), "Imported 1 bookmark, 2 history entries and 3 cookies.");
        assert_eq!(notices::imported(&kinds(0, 0, 0), None), None);
    }

    #[test]
    fn imports_bookmarks_and_merges_history_by_time() {
        let mut s = session(r#"{"Bookmarks":["https://a.test/"],"History":[
            {"Url":"https://old.test/","VisitedAt":"2026-01-01T09:00:00+09:00","Title":"Old"},
            {"Url":"https://a.test/","VisitedAt":"2026-03-01T00:00:00+00:00","Title":""}]}"#);
        s.snapshot(false);
        let link = |url: &str, title: &str| Link { url: url.into(), title: title.into() };
        let visit = |url: &str, at: i64| Visit { url: url.into(), title: format!("T {url}"), visited_at: at };
        let jan = parse_iso8601("2026-01-15T00:00:00Z").unwrap();
        let (bookmarks, history) = s.import_library(
            vec![link("https://a.test/", "A named"), link("https://b.test/", "B"), link("https://b.test/", "B again")],
            vec![visit("https://between.test/", jan), visit("https://a.test/", jan), visit("https://new.test/", jan + 90 * 86_400)],
        );
        assert_eq!((bookmarks, history), (1, 2), "a.test was visited more recently in Still");
        let snap = s.snapshot(false);
        let titles: Vec<_> = snap.bookmarks.unwrap().into_iter().map(|b| b.title).collect();
        assert_eq!(titles, vec!["B", "A named"], "imported titles fill in until a visit names the page");
        let urls: Vec<_> = snap.history.unwrap().into_iter().map(|h| h.url).collect();
        assert_eq!(urls, vec!["https://new.test/", "https://a.test/", "https://between.test/", "https://old.test/"]);
        let saved = BrowserState::parse(&s.take_save().unwrap()).unwrap();
        assert_eq!(saved.bookmark_titles.get("https://b.test/").map(String::as_str), Some("B"));
        assert_eq!(s.import_library(vec![link("https://b.test/", "B")], vec![visit("https://new.test/", jan)]), (0, 0), "importing twice adds nothing");
        assert_eq!(s.take_save(), None);
        s.remove_bookmark("https://b.test/");
        assert!(!s.take_save().unwrap().contains("b.test"), "a removed bookmark forgets its title");
    }

    #[test]
    fn imported_history_keeps_the_newest_entries_and_respects_the_setting() {
        let mut s = fresh();
        s.update_settings(&patch(serde_json::json!({"KeepHistory": false})));
        let visits: Vec<Visit> = (0..600).map(|i| Visit { url: format!("https://e.test/{i}"), title: String::new(), visited_at: 1_700_000_000 + i }).collect();
        assert_eq!(s.import_library(Vec::new(), visits.clone()), (0, 0), "history is off");
        s.update_settings(&patch(serde_json::json!({"KeepHistory": true})));
        assert_eq!(s.import_library(Vec::new(), visits), (0, HISTORY_LIMIT));
        let state = BrowserState::parse(&s.take_save().unwrap()).unwrap();
        assert_eq!(state.history.len(), HISTORY_LIMIT);
        assert_eq!(state.history[0].url, "https://e.test/100");
        assert_eq!(state.history.last().unwrap().url, "https://e.test/599");
    }

    #[test]
    fn logins_are_offered_on_their_site_and_filled_only_there() {
        let mut s = fresh();
        let login = |url: &str, user: &str| Login::new(url, user, "secret").unwrap();
        assert_eq!(s.import_logins(vec![login("https://accounts.example.com/x", "me"), login("https://other.test", "you")]), 2);
        let snap = s.snapshot(false);
        assert_eq!(snap.saved_logins.unwrap().len(), 2);
        assert!(snap.logins.is_empty(), "a blank tab offers nothing");
        assert!(s.take_vault().unwrap().contains("secret"));
        assert_eq!(s.take_vault(), None);

        s.navigate("https://accounts.example.com/signin").unwrap();
        let snap = s.snapshot(false);
        assert_eq!(snap.logins.len(), 1);
        assert!(snap.saved_logins.is_none(), "unchanged");
        assert!(!serde_json::to_string(&snap).unwrap().contains("secret"), "passwords never reach the interface");
        let (tab, script) = s.fill_login(snap.logins[0].index).unwrap();
        assert_eq!(tab, s.active());
        assert!(script.contains("\"accounts.example.com\""));
        let other = s.vault.logins.iter().position(|l| l.username == "you").unwrap();
        assert_eq!(s.fill_login(other), None, "another site's login is refused");
        assert_eq!(s.fill_login(99), None);

        s.remove_login(other);
        assert_eq!(s.snapshot(false).saved_logins.unwrap().len(), 1);
        s.clear_logins();
        assert!(s.snapshot(false).logins.is_empty());
        assert_eq!(s.take_vault().as_deref(), Some(r#"{"Logins":[]}"#));
    }

    #[test]
    fn settings_page_hides_the_page_until_a_tab_is_chosen() {
        let mut s = session(r#"{"Tabs":[{"Url":"https://a.test/"},{"Url":"https://b.test/"}]}"#);
        let [a, b] = [s.tabs()[0].id, s.tabs()[1].id];
        s.set_bounds(B);
        s.plan_show(false);
        s.set_settings_open(true);
        assert_eq!(s.plan_show(false), ShowPlan { hide: Some(a), action: ShowAction::None });
        assert_eq!(s.plan_show(false), ShowPlan { hide: None, action: ShowAction::None });
        assert!(s.snapshot(false).settings_open);
        s.set_settings_open(false);
        assert_eq!(s.plan_show(false).action, ShowAction::Reveal { id: a, bounds: B });
        s.set_settings_open(true);
        s.plan_show(false);
        s.select(b);
        assert!(!s.snapshot(false).settings_open, "choosing a tab leaves settings");
        assert!(matches!(s.plan_show(false).action, ShowAction::Create { id, .. } if id == b));
        s.set_settings_open(true);
        s.open_tab(BLANK);
        assert!(!s.snapshot(false).settings_open);
        s.set_settings_open(true);
        s.navigate("https://c.test/").unwrap();
        assert!(!s.snapshot(false).settings_open);
    }

    #[test]
    fn opening_settings_ends_page_fullscreen() {
        let mut s = fresh();
        s.navigate("https://video.test/").unwrap();
        s.set_bounds(B);
        s.plan_show(false);
        let video = s.active();
        s.fullscreen_changed(video, true);
        s.set_settings_open(true);
        assert_eq!(s.leave_fullscreen_if_inactive(), Some(video));
    }

    #[test]
    fn save_dialog_redirects_or_cancels_a_download() {
        let mut s = fresh();
        s.download_started(r"C:\Downloads\a.zip".into(), None);
        s.download_redirected(r"C:\Downloads\a.zip", r"D:\Keep\a.zip".into());
        assert_eq!(s.snapshot(true).downloads.unwrap()[0].path, r"D:\Keep\a.zip");
        s.download_started(r"C:\Downloads\b.zip".into(), None);
        s.download_cancelled(r"C:\Downloads\b.zip");
        let list = s.snapshot(true).downloads.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(s.snapshot(false).downloading, 1);
    }

    #[test]
    fn settings_round_trip_through_the_saved_file() {
        let mut s = fresh();
        s.update_settings(&patch(serde_json::json!({"Theme": "Dark", "Accent": "Green", "HomePage": "home.test", "AskDownloadFolder": true})));
        let state = BrowserState::parse(&s.take_save().unwrap()).unwrap();
        assert_eq!(&state.settings, s.settings());
        assert_eq!(state.settings.home_page, "https://home.test/");
    }
}
