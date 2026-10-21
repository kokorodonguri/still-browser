//! Executes `Session` decisions with Tauri: webviews, windows, threads, disk.
//! Rule: never call a window/webview API while holding a lock. Those calls wait
//! for the main thread, which may itself be waiting for the lock in a handler.

use crate::logger::{error, info, warning};
use serde::Serialize;
use still_core::import::{self, Engine, ImportError, ImportedFile, Source};
use still_core::passwords::Vault;
use still_core::persistence::StateWriter;
use still_core::session::{notices, Bounds, LoadOutcome, ProcessFailure, Session, ShowAction, TabId};
use still_core::{WindowPlacement, BLANK};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Mutex;
use std::time::Duration;
use tauri::webview::{DownloadEvent, NewWindowResponse, PageLoadEvent};
use tauri::{
    AppHandle, Emitter, EventTarget, LogicalPosition, LogicalSize, Manager, PhysicalPosition, PhysicalSize, Rect, Url,
    Webview, WebviewBuilder, WebviewUrl, WebviewWindowBuilder, Window, Wry,
};

pub const WINDOW: &str = "main";
pub const CHROME: &str = "chrome";
const SCROLLBARS: &str = include_str!("scrollbars.js");
/// Bursts of page events collapse into one UI update per frame and one disk
/// write per interval.
const PUBLISH_DELAY: Duration = Duration::from_millis(16);
const SAVE_DELAY: Duration = Duration::from_millis(800);
/// A saved window must overlap a monitor by this much to be restored there.
const MIN_VISIBLE: i64 = 64;

pub struct Shell {
    session: Mutex<Session>,
    views: Mutex<HashMap<TabId, Webview>>,
    writer: Mutex<StateWriter>,
    pub profile_dir: PathBuf,
    /// Holds the new tab picture (see `still_core::new_tab_image`).
    new_tab_dir: PathBuf,
    publish_pending: AtomicBool,
    publish_wake: SyncSender<()>,
    save_wake: SyncSender<()>,
    popup_seq: AtomicU64,
    /// Hidden page webview that exists only while site data is being cleared
    /// or cookies imported with no page open.
    cleaner: Mutex<Option<Webview>>,
    /// Saved logins, encrypted for the Windows user.
    vault_file: PathBuf,
    /// False when an unreadable vault could not be set aside, so it is never
    /// overwritten.
    vault_writable: AtomicBool,
    /// Copies of other browsers' databases live here only during an import.
    import_dir: PathBuf,
    importing: AtomicBool,
}

pub struct Workers {
    publish: Receiver<()>,
    save: Receiver<()>,
}

impl Shell {
    /// Loads saved state, backing up files that must not be overwritten.
    pub fn load(base: &Path, args: &[String]) -> (Self, Workers) {
        let data_file = base.join("state.json");
        let saved = match std::fs::read_to_string(&data_file) {
            Ok(text) => Some(Ok(text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                error!("cannot read {}: {e}", data_file.display());
                Some(Err(()))
            }
        };
        let (session, outcome) = Session::load(saved, args);
        let backup = match outcome {
            LoadOutcome::Corrupt => Some(format!("json.invalid-{}", still_core::now_iso8601().replace(':', ""))),
            LoadOutcome::Newer(version) => Some(format!("json.v{version}-backup")),
            LoadOutcome::Unreadable => Some("json.unreadable-backup".into()),
            _ => None,
        };
        let backup = backup.map(|extension| data_file.with_extension(extension));
        if backup.is_some() {
            warning!("state file {outcome:?}; backup required before saving");
        }
        info!("session loaded: {outcome:?}, {} tabs", session.tabs().len());
        let new_tab_dir = base.join("NewTab");
        // A picture replaced just before a crash or failed save is left over.
        let removed = still_core::new_tab_image::remove_others(&new_tab_dir, &session.settings().new_tab_image);
        if removed > 0 {
            info!("removed {removed} unused new tab picture(s)");
        }
        let mut session = session;
        let vault_file = base.join("passwords.dat");
        let vault_writable = load_vault(&vault_file, &mut session);
        let (publish_wake, publish) = sync_channel(1);
        let (save_wake, save) = sync_channel(1);
        let shell = Self {
            session: Mutex::new(session),
            views: Mutex::new(HashMap::new()),
            writer: Mutex::new(StateWriter::new(data_file, backup)),
            profile_dir: base.join("WebView2"),
            new_tab_dir,
            publish_pending: AtomicBool::new(false),
            publish_wake,
            save_wake,
            popup_seq: AtomicU64::new(1),
            cleaner: Mutex::new(None),
            vault_file,
            vault_writable: AtomicBool::new(vault_writable),
            import_dir: base.join("Import"),
            importing: AtomicBool::new(false),
        };
        // Left behind if Still stopped during an import; holds copied history.
        let _ = std::fs::remove_dir_all(&shell.import_dir);
        (shell, Workers { publish, save })
    }
}

/// Reads the saved logins. One that cannot be decrypted or read (another
/// Windows user, a damaged file) is renamed so nothing overwrites it; returns
/// false when even that failed.
fn load_vault(file: &Path, session: &mut Session) -> bool {
    let bytes = match std::fs::read(file) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return true,
        Err(e) => {
            error!("cannot read {}: {e}", file.display());
            session.set_notice(notices::PASSWORDS_UNREADABLE);
            return false;
        }
    };
    let vault = unprotect(&bytes).and_then(|plain| String::from_utf8(plain).ok()).and_then(|text| Vault::parse(&text).ok());
    if let Some(vault) = vault {
        info!("{} saved login(s)", vault.len());
        session.set_vault(vault);
        return true;
    }
    let backup = file.with_extension(format!("dat.unreadable-{}", still_core::now_iso8601().replace(':', "")));
    warning!("saved logins are unreadable; moving them to {}", backup.display());
    session.set_notice(notices::PASSWORDS_UNREADABLE);
    std::fs::rename(file, &backup).map_err(|e| error!("keeping unreadable logins failed: {e}")).is_ok()
}

#[cfg(windows)]
use crate::platform::{protect, unprotect};

/// Without DPAPI (development builds on Linux) logins stay in memory only.
#[cfg(not(windows))]
fn protect(_: &[u8]) -> Option<Vec<u8>> {
    None
}

#[cfg(not(windows))]
fn unprotect(_: &[u8]) -> Option<Vec<u8>> {
    None
}

fn shell(app: &AppHandle) -> tauri::State<'_, Shell> {
    app.state::<Shell>()
}

pub fn with<T>(app: &AppHandle, f: impl FnOnce(&mut Session) -> T) -> T {
    let shell = shell(app);
    let mut session = shell.session.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut session)
}

fn view(app: &AppHandle, id: TabId) -> Option<Webview> {
    shell(app).views.lock().unwrap_or_else(|e| e.into_inner()).get(&id).cloned()
}

fn take_view(app: &AppHandle, id: TabId) -> Option<Webview> {
    shell(app).views.lock().unwrap_or_else(|e| e.into_inner()).remove(&id)
}

pub fn window(app: &AppHandle) -> Option<Window> {
    app.get_window(WINDOW)
}

// ---------- Background workers ----------

pub fn start_workers(app: &AppHandle, workers: Workers) {
    let handle = app.clone();
    std::thread::Builder::new()
        .name("still-publish".into())
        .spawn(move || {
            while workers.publish.recv().is_ok() {
                std::thread::sleep(PUBLISH_DELAY);
                shell(&handle).publish_pending.store(false, Ordering::Release);
                let snapshot = with(&handle, |s| s.snapshot(false));
                let _ = handle.emit_to(EventTarget::webview(CHROME), "state", snapshot);
            }
        })
        .expect("spawn publisher");
    let handle = app.clone();
    std::thread::Builder::new()
        .name("still-save".into())
        .spawn(move || {
            while workers.save.recv().is_ok() {
                std::thread::sleep(SAVE_DELAY);
                flush(&handle);
            }
        })
        .expect("spawn saver");
}

/// Requests a coalesced UI update.
pub fn publish(app: &AppHandle) {
    let shell = shell(app);
    if !shell.publish_pending.swap(true, Ordering::AcqRel) {
        let _ = shell.publish_wake.try_send(());
    }
}

/// Requests a coalesced save (the session tracks whether anything changed)
/// and a UI update.
pub fn changed(app: &AppHandle) {
    let _ = shell(app).save_wake.try_send(());
    publish(app);
}

/// Serialize save transactions, including the snapshot: shutdown waits for an
/// in-flight save and cannot overwrite a newer snapshot with an older one.
/// JSON encoding, backup and disk I/O run outside the session lock.
pub fn flush(app: &AppHandle) {
    let shell = shell(app);
    let mut writer = shell.writer.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(state) = with(app, |s| s.take_state()) {
        // Encoded straight into the file instead of a full JSON string first.
        let saved = writer.save_with(|out| serde_json::to_writer(out, &state).map_err(std::io::Error::from));
        drop(state);
        if let Err(e) = saved {
            error!("save failed: {e}");
            with(app, |s| s.save_failed());
            publish(app);
        }
    }
    if let Some(json) = with(app, |s| s.take_vault()) {
        if !shell.vault_writable.load(Ordering::Acquire) {
            warning!("saved logins are not written: the unreadable file is still in place");
            return;
        }
        let saved = match protect(json.as_bytes()) {
            Some(sealed) => still_core::save_atomic_with(&shell.vault_file, |out| out.write_all(&sealed)),
            None => Err(std::io::Error::other("encryption is unavailable")),
        };
        if let Err(e) = saved {
            error!("saving logins failed: {e}");
            #[cfg(windows)]
            {
                with(app, |s| s.vault_save_failed());
                publish(app);
            }
        }
    }
}

// ---------- Pages ----------

fn rect(bounds: Bounds) -> Rect {
    Rect {
        position: LogicalPosition::new(bounds.x, bounds.y).into(),
        size: LogicalSize::new(bounds.width.max(1.0), bounds.height.max(1.0)).into(),
    }
}

/// Makes the active tab visible. Only the previously shown page is hidden, so
/// the cost does not grow with the number of tabs. `force` re-applies geometry.
pub fn show_active(app: &AppHandle, force: bool) {
    let plan = with(app, |s| s.plan_show(force));
    if let Some((id, previous)) = plan.hide.and_then(|id| view(app, id).map(|page| (id, page))) {
        let _ = previous.hide();
        #[cfg(windows)]
        {
            let generation = with(app, |s| s.tab(id).map_or(0, |t| t.view_generation));
            crate::platform::rest(app, &previous, id, generation);
        }
        #[cfg(not(windows))]
        let _ = id;
    }
    match plan.action {
        ShowAction::None => {}
        ShowAction::Create { id, generation, url, bounds } => {
            if let Err(e) = create_page(app, id, generation, &url, bounds) {
                error!("creating page for tab {id} failed: {e}");
                with(app, |s| { if s.view_is_current(id, generation) { s.view_failed(id, &e.to_string()); } });
                publish(app);
            }
        }
        ShowAction::Resize { id, bounds } => {
            if let Some(page) = view(app, id) {
                let _ = page.set_bounds(rect(bounds));
            }
        }
        ShowAction::Reveal { id, bounds } => {
            if let Some(page) = view(app, id) {
                #[cfg(windows)]
                crate::platform::wake(&page);
                let _ = page.set_bounds(rect(bounds));
                let _ = page.show();
            }
        }
    }
}

fn create_page(app: &AppHandle, id: TabId, generation: u64, url: &str, bounds: Bounds) -> tauri::Result<()> {
    let window = window(app).ok_or(tauri::Error::WindowNotFound)?;
    let target: Url = url.parse().map_err(|_| tauri::Error::InvalidWebviewUrl("invalid page address"))?;
    let label = format!("tab-{id}-{}", shell(app).popup_seq.fetch_add(1, Ordering::Relaxed));

    let on_title = app.clone();
    let on_load = app.clone();
    let on_popup = app.clone();
    let on_download = app.clone();
    let builder = WebviewBuilder::new(label, WebviewUrl::External(target))
        .data_directory(shell(app).profile_dir.clone())
        .initialization_script_for_all_frames(SCROLLBARS)
        .on_document_title_changed(move |_, title| {
            with(&on_title, |s| { if s.view_is_current(id, generation) { s.title_changed(id, &title); } });
            changed(&on_title);
        })
        .on_page_load(move |_, payload| {
            let url = payload.url().as_str();
            with(&on_load, |s| {
                if s.view_is_current(id, generation) {
                    match payload.event() {
                        PageLoadEvent::Started => s.load_started(id, url),
                        PageLoadEvent::Finished => s.load_finished(id, url),
                    }
                }
            });
            changed(&on_load);
        })
        .on_new_window(move |url, features| new_window(&on_popup, url, features))
        .on_download(move |_, event| {
            match event {
                DownloadEvent::Requested { url, destination } => {
                    info!("download started from {}", url.host_str().unwrap_or("unknown origin"));
                    use_download_folder(&on_download, destination);
                    with(&on_download, |s| s.download_started(destination.to_string_lossy().into_owned(), Some(id)));
                }
                DownloadEvent::Finished { path, success, .. } => {
                    let path = path.map(|p| p.to_string_lossy().into_owned());
                    with(&on_download, |s| {
                        s.download_finished(path, success, |p| std::fs::metadata(p).ok().map(|m| m.len() as i64))
                    });
                }
                _ => {}
            }
            changed(&on_download);
            true
        });
    let page = window.add_child(builder, LogicalPosition::new(bounds.x, bounds.y), LogicalSize::new(bounds.width.max(1.0), bounds.height.max(1.0)))?;
    // The tab may have closed or its engine may have restarted while add_child
    // waited for the UI thread. Commit ownership atomically with that decision.
    let visible = with(app, |s| {
        if !s.view_is_current(id, generation) { return None; }
        shell(app).views.lock().unwrap_or_else(|e| e.into_inner()).insert(id, page.clone());
        Some(s.active() == id && !s.settings_open())
    });
    let Some(visible) = visible else { let _ = page.close(); return Ok(()) };
    #[cfg(windows)]
    crate::platform::attach(app.clone(), &page, id, generation);
    if visible {
        let _ = page.set_focus();
    } else {
        let _ = page.hide();
    }
    Ok(())
}

/// Sized `window.open` calls (sign-in popups) become real popup windows that
/// share the page's WebView2 environment, so `window.opener` keeps working.
/// Plain new-window links open as tabs.
fn new_window(app: &AppHandle, url: Url, features: tauri::webview::NewWindowFeatures) -> NewWindowResponse<Wry> {
    if features.size().is_none() {
        let app = app.clone();
        tauri::async_runtime::spawn(async move { open_tab(&app, url.as_str()) });
        return NewWindowResponse::Deny;
    }
    let label = format!("popup-{}", shell(app).popup_seq.fetch_add(1, Ordering::Relaxed));
    let tabs = app.clone();
    let built = WebviewWindowBuilder::new(app, label, WebviewUrl::External(url.clone()))
        .window_features(features)
        .title("Still")
        .min_inner_size(320.0, 240.0)
        .initialization_script_for_all_frames(SCROLLBARS)
        .on_new_window(move |url, _| {
            let app = tabs.clone();
            tauri::async_runtime::spawn(async move { open_tab(&app, url.as_str()) });
            NewWindowResponse::Deny
        })
        .build();
    match built {
        Ok(window) => {
            info!("popup opened: {}", url.host_str().unwrap_or_default());
            NewWindowResponse::Create { window }
        }
        Err(e) => {
            error!("popup failed, opening as tab: {e}");
            let app = app.clone();
            tauri::async_runtime::spawn(async move { open_tab(&app, url.as_str()) });
            NewWindowResponse::Deny
        }
    }
}

/// Moves a starting download into the folder chosen in settings.
fn use_download_folder(app: &AppHandle, destination: &mut PathBuf) {
    let folder = with(app, |s| s.settings().download_folder.clone());
    if folder.is_empty() {
        return;
    }
    let folder = PathBuf::from(folder);
    if !folder.is_dir() {
        warning!("download folder {} is missing; using the default", folder.display());
        with(app, |s| s.set_notice(notices::DOWNLOAD_FOLDER_GONE));
        return;
    }
    if let Some(name) = destination.file_name() {
        *destination = still_core::unique_path(&folder, &name.to_string_lossy(), |p| p.exists());
    }
}

// ---------- Tab operations ----------

/// Shared tail of every operation that may change the active tab.
fn after_tab_change(app: &AppHandle) {
    if let Some(previous) = with(app, |s| s.leave_fullscreen_if_inactive()) {
        if let Some(window) = window(app) {
            let _ = window.set_fullscreen(false);
        }
        if let Some(page) = view(app, previous) {
            let _ = page.eval("document.fullscreenElement && document.exitFullscreen()");
        }
    }
    show_active(app, false);
    changed(app);
}

pub fn open_tab(app: &AppHandle, url: &str) {
    with(app, |s| s.open_tab(url));
    after_tab_change(app);
}

pub fn select(app: &AppHandle, id: TabId) {
    if with(app, |s| s.select(id)) {
        after_tab_change(app);
    }
}

pub fn close(app: &AppHandle, id: TabId) {
    let Some(had_view) = with(app, |s| s.close(id)) else { return };
    if had_view {
        if let Some(page) = take_view(app, id) {
            let _ = page.close();
        }
    }
    after_tab_change(app);
}

pub fn navigate(app: &AppHandle, input: &str) {
    use still_core::session::Navigation;
    match with(app, |s| s.navigate(input)) {
        Ok(Some(Navigation::Existing { id, url })) => {
            if let Some(page) = view(app, id) {
                if let Ok(url) = url.parse() {
                    let _ = page.navigate(url);
                }
                let _ = page.set_focus();
            }
        }
        Ok(Some(Navigation::Create)) => show_active(app, false),
        Ok(None) | Err(_) => {}
    }
    changed(app);
}

pub fn go(app: &AppHandle, action: &str) {
    let active = with(app, |s| s.active());
    if action == "reload" {
        match with(app, |s| s.reload()).and_then(|id| view(app, id)) {
            Some(page) => {
                let _ = page.reload();
            }
            None => show_active(app, false),
        }
        changed(app);
        return;
    }
    let Some(page) = view(app, active) else { return };
    let script = match action {
        "back" => "history.back()",
        "forward" => "history.forward()",
        "stop" => "window.stop()",
        _ => return,
    };
    let _ = page.eval(script);
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shortcut {
    FocusAddress,
    NewTab,
    CloseTab,
    NextTab,
    PreviousTab,
    Settings,
}

pub fn run_shortcut(app: &AppHandle, shortcut: Shortcut) {
    match shortcut {
        Shortcut::FocusAddress => {
            if let Some(chrome) = app.get_webview(CHROME) {
                let _ = chrome.set_focus();
            }
            let _ = app.emit_to(EventTarget::webview(CHROME), "focus-address", ());
        }
        Shortcut::NewTab => {
            open_tab(app, BLANK);
            run_shortcut(app, Shortcut::FocusAddress);
        }
        Shortcut::CloseTab => {
            let active = with(app, |s| s.active());
            close(app, active);
        }
        Shortcut::NextTab | Shortcut::PreviousTab => {
            with(app, |s| s.cycle(shortcut == Shortcut::NextTab));
            after_tab_change(app);
        }
        Shortcut::Settings => {
            let open = with(app, |s| s.settings_open());
            set_settings_open(app, !open);
            if let Some(chrome) = app.get_webview(CHROME) {
                let _ = chrome.set_focus();
            }
        }
    }
}

// ---------- Settings ----------

pub fn set_settings_open(app: &AppHandle, open: bool) {
    with(app, |s| s.set_settings_open(open));
    after_tab_change(app);
}

/// Any open page (they share one profile), or a hidden helper when none is.
/// Call `release_helper` when done.
fn profile_view(app: &AppHandle) -> Option<Webview> {
    if let Some(page) = shell(app).views.lock().unwrap_or_else(|e| e.into_inner()).values().next().cloned() {
        return Some(page);
    }
    let window = window(app)?;
    let label = format!("cleaner-{}", shell(app).popup_seq.fetch_add(1, Ordering::Relaxed));
    let builder = WebviewBuilder::new(label, WebviewUrl::External(BLANK.parse().ok()?)).data_directory(shell(app).profile_dir.clone());
    match window.add_child(builder, LogicalPosition::new(0.0, 0.0), LogicalSize::new(1.0, 1.0)) {
        Ok(page) => {
            let _ = page.hide();
            *shell(app).cleaner.lock().unwrap_or_else(|e| e.into_inner()) = Some(page.clone());
            Some(page)
        }
        Err(e) => {
            error!("helper webview for clearing data failed: {e}");
            None
        }
    }
}

pub fn update_settings(app: &AppHandle, patch: &serde_json::Map<String, serde_json::Value>) {
    // The picture changes only through `set_new_tab_image`, which owns its file.
    let mut patch = patch.clone();
    patch.remove("NewTabImage");
    let patch = &patch;
    let (previous, theme) = with(app, |s| (s.update_settings(patch), s.settings().theme));
    if previous.theme != theme {
        // The color scheme belongs to the page profile, so one page is enough.
        #[cfg(windows)]
        if let Some(page) = shell(app).views.lock().unwrap_or_else(|e| e.into_inner()).values().next().cloned() {
            crate::platform::apply_color_scheme(&page, theme);
        }
    }
    changed(app);
}

/// Copies the chosen picture into the data folder and shows it behind new tabs.
/// Empty `bytes` removes the picture.
pub fn set_new_tab_image(app: &AppHandle, bytes: &[u8]) {
    use still_core::new_tab_image::{remove_others, store, StoreError};
    let dir = &shell(app).new_tab_dir;
    let name = if bytes.is_empty() {
        String::new()
    } else {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
        match store(dir, bytes, now) {
            Ok(name) => name,
            Err(e) => {
                let notice = match e {
                    StoreError::Unsupported => notices::IMAGE_UNSUPPORTED,
                    StoreError::Io(e) => {
                        error!("saving the new tab picture failed: {e}");
                        notices::IMAGE_FAILED
                    }
                };
                with(app, |s| s.set_notice(notice));
                publish(app);
                return;
            }
        }
    };
    let mut patch = serde_json::Map::new();
    patch.insert("NewTabImage".into(), name.clone().into());
    with(app, |s| s.update_settings(&patch));
    remove_others(dir, &name);
    info!("new tab picture {}", if name.is_empty() { "removed" } else { "set" });
    changed(app);
}

/// The current picture's bytes; empty when there is none or it is unreadable.
pub fn new_tab_image(app: &AppHandle) -> Vec<u8> {
    let name = with(app, |s| s.settings().new_tab_image.clone());
    if name.is_empty() {
        return Vec::new();
    }
    std::fs::read(shell(app).new_tab_dir.join(&name)).unwrap_or_else(|e| {
        warning!("new tab picture {name} is unreadable: {e}");
        Vec::new()
    })
}

/// Shows the folder picker on the main thread and stores the choice.
pub fn pick_download_folder(app: &AppHandle) {
    #[cfg(windows)]
    {
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            let current = with(&handle, |s| s.settings().download_folder.clone());
            if let Some(folder) = crate::platform::pick_folder(&handle, &current) {
                let mut patch = serde_json::Map::new();
                patch.insert("DownloadFolder".into(), folder.to_string_lossy().into_owned().into());
                update_settings(&handle, &patch);
            }
        });
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        warning!("the folder picker is only available on Windows");
    }
}

pub fn clear_browsing_data(app: &AppHandle, history: bool, site_data: bool, cache: bool) {
    if history {
        with(app, |s| s.clear_history());
    }
    if !site_data && !cache {
        if history {
            browsing_data_cleared(app, true);
        }
        return;
    }
    let Some(page) = profile_view(app) else {
        browsing_data_cleared(app, false);
        return;
    };
    #[cfg(windows)]
    crate::platform::clear_browsing_data(app.clone(), &page, site_data, cache);
    #[cfg(not(windows))]
    browsing_data_cleared(app, page.clear_all_browsing_data().is_ok());
}

pub fn browsing_data_cleared(app: &AppHandle, ok: bool) {
    if ok {
        info!("browsing data cleared");
    } else {
        error!("clearing browsing data failed");
    }
    release_helper(app);
    with(app, |s| s.set_notice(if ok { notices::CLEARED } else { notices::CLEAR_FAILED }));
    changed(app);
}

fn release_helper(app: &AppHandle) {
    let helper = shell(app).cleaner.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(helper) = helper {
        let _ = helper.close();
    }
}

// ---------- Moving from another browser ----------

#[derive(Serialize)]
pub struct SourceView {
    id: String,
    browser: String,
    profile: String,
    /// Firefox cookies are readable; Chromium encrypts them for itself.
    cookies: bool,
}

fn sources() -> Vec<Source> {
    let dir = |name| std::env::var_os(name).map(PathBuf::from);
    match (dir("LOCALAPPDATA"), dir("APPDATA")) {
        (Some(local), Some(roaming)) => import::discover(&local, &roaming),
        _ => Vec::new(),
    }
}

/// Browser profiles on this computer that Still can import from.
pub fn import_sources() -> Vec<SourceView> {
    sources()
        .into_iter()
        .map(|s| SourceView { id: s.id(), cookies: s.engine == Engine::Firefox, browser: s.browser, profile: s.profile })
        .collect()
}

/// Ends an import: shows `notice` and allows the next one.
fn import_done(app: &AppHandle, notice: String) {
    let _ = std::fs::remove_dir_all(&shell(app).import_dir);
    shell(app).importing.store(false, Ordering::Release);
    with(app, |s| s.set_notice(notice));
    changed(app);
}

/// Imports from a browser profile found by `import_sources`. Its databases
/// are copied first, so the browser may stay open. Runs off the main thread.
pub fn import_from(app: &AppHandle, id: &str, bookmarks: bool, history: bool, cookies: bool) {
    let Some(source) = sources().into_iter().find(|s| s.id() == id) else {
        warning!("import source is gone");
        with(app, |s| s.set_notice(notices::IMPORT_FAILED));
        publish(app);
        return;
    };
    if shell(app).importing.swap(true, Ordering::AcqRel) {
        return;
    }
    let from = if source.profile.is_empty() { source.browser.clone() } else { format!("{} ({})", source.browser, source.profile) };
    with(app, |s| s.set_notice(format!("Importing from {from}…")));
    publish(app);
    let scratch = shell(app).import_dir.clone();
    let dir = &source.dir;
    let mut failed = 0;
    let mut part = |name: &str, result: Result<(), ImportError>| {
        if let Err(e) = result {
            warning!("importing {name} from {} failed: {e}", source.browser);
            failed += 1;
        }
    };
    let (mut links, mut visits, mut jar) = (Vec::new(), Vec::new(), Vec::new());
    let limit = still_core::HISTORY_LIMIT;
    match source.engine {
        Engine::Chromium => {
            if bookmarks {
                let read = std::fs::read_to_string(dir.join("Bookmarks")).map_err(ImportError::from);
                part("bookmarks", read.and_then(|t| import::chromium_bookmarks(&t)).map(|found| links = found));
            }
            if history {
                part("history", import::chromium_history(&dir.join("History"), &scratch.join("history"), limit).map(|found| visits = found));
            }
        }
        Engine::Firefox => {
            let places = dir.join("places.sqlite");
            if bookmarks {
                part("bookmarks", import::firefox_bookmarks(&places, &scratch.join("bookmarks")).map(|found| links = found));
            }
            if history {
                part("history", import::firefox_history(&places, &scratch.join("history"), limit).map(|found| visits = found));
            }
            if cookies {
                let now = still_core::now_secs() as i64;
                part("cookies", import::firefox_cookies(&dir.join("cookies.sqlite"), &scratch.join("cookies"), now).map(|found| jar = found));
            }
        }
    }
    let requested = [bookmarks, history, cookies && source.engine == Engine::Firefox].iter().filter(|r| **r).count();
    let (added_bookmarks, added_history) = with(app, |s| s.import_library(links, visits));
    info!("imported {added_bookmarks} bookmark(s) and {added_history} history entries from {}", source.browser);
    let finish = move |app: &AppHandle, added_cookies: usize| {
        let counts = [
            (added_bookmarks, "bookmark", "bookmarks"),
            (added_history, "history entry", "history entries"),
            (added_cookies, "cookie", "cookies"),
        ];
        let notice = notices::imported(&counts, Some(&from)).unwrap_or_else(|| {
            if requested > 0 && failed == requested { notices::IMPORT_FAILED } else { notices::IMPORT_NOTHING }.to_owned()
        });
        import_done(app, notice);
    };
    if jar.is_empty() {
        finish(app, 0);
        return;
    }
    let Some(page) = profile_view(app) else {
        finish(app, 0);
        return;
    };
    #[cfg(windows)]
    crate::platform::import_cookies(app.clone(), &page, jar, move |app, added| {
        info!("imported {added} cookie(s)");
        release_helper(app);
        finish(app, added);
    });
    #[cfg(not(windows))]
    {
        let _ = page;
        release_helper(app);
        finish(app, 0);
    }
}

/// Imports an exported file: bookmarks (HTML), passwords (CSV) or Google
/// Takeout history (JSON).
pub fn import_file(app: &AppHandle, bytes: &[u8]) {
    let notice = match import::read_file(bytes) {
        Ok(ImportedFile::Bookmarks(links)) => {
            let (added, _) = with(app, |s| s.import_library(links, Vec::new()));
            notices::imported(&[(added, "bookmark", "bookmarks")], None)
        }
        Ok(ImportedFile::History(visits)) => {
            let (_, added) = with(app, |s| s.import_library(Vec::new(), visits));
            notices::imported(&[(added, "history entry", "history entries")], None)
        }
        Ok(ImportedFile::Passwords(logins)) => {
            let added = with(app, |s| s.import_logins(logins));
            info!("imported {added} login(s)");
            notices::imported(&[(added, "password", "passwords")], None)
        }
        Err(e) => {
            warning!("import file refused: {e}");
            Some(match e { ImportError::TooLarge => notices::IMPORT_TOO_LARGE, _ => notices::IMPORT_UNRECOGNIZED }.to_owned())
        }
    };
    with(app, |s| s.set_notice(notice.unwrap_or_else(|| notices::IMPORT_NOTHING.to_owned())));
    changed(app);
}

/// Fills saved login `index` into the active page, if it belongs there.
pub fn fill_login(app: &AppHandle, index: usize) {
    let Some((id, script)) = with(app, |s| s.fill_login(index)) else { return };
    let Some(page) = view(app, id) else { return };
    #[cfg(windows)]
    crate::platform::fill(app.clone(), &page, script);
    #[cfg(not(windows))]
    let _ = page.eval(script);
}

pub fn fill_found_nothing(app: &AppHandle) {
    with(app, |s| s.set_notice(notices::FILL_NONE));
    publish(app);
}

// ---------- Page engine events (Windows hooks) ----------

#[cfg_attr(not(windows), allow(dead_code))]
pub fn history_changed(app: &AppHandle, id: TabId, generation: u64, can_back: bool, can_forward: bool, url: String) {
    with(app, |s| { if s.view_is_current(id, generation) { s.history_changed(id, can_back, can_forward, &url); } });
    changed(app);
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn process_failed(app: &AppHandle, id: TabId, generation: u64, failure: ProcessFailure) {
    warning!("page process failure on tab {id}: {failure:?}");
    let (recreate, lost) = with(app, |s| {
        if !s.view_is_current(id, generation) { return (false, Vec::new()); }
        let ids = s.process_failed(id, failure);
        // Remove ownership before another thread can commit replacement views.
        let shell = shell(app);
        let mut views = shell.views.lock().unwrap_or_else(|e| e.into_inner());
        let pages: Vec<_> = ids.iter().filter_map(|id| views.remove(id)).collect();
        (!ids.is_empty(), pages)
    });
    for page in lost {
        let _ = page.close();
    }
    if recreate {
        if let Some(window) = window(app) {
            let _ = window.set_fullscreen(false);
        }
        show_active(app, false);
    }
    changed(app);
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn page_fullscreen(app: &AppHandle, id: TabId, generation: u64, on: bool) {
    let Some(window) = window(app) else { return };
    let screen = window.current_monitor().ok().flatten().map(|m| {
        let size = m.size().to_logical::<f64>(m.scale_factor());
        Bounds { x: 0.0, y: 0.0, width: size.width, height: size.height }
    });
    let toggled = with(app, |s| {
        if !s.view_is_current(id, generation) { return false; }
        if let Some(screen) = screen {
            s.set_window_bounds(screen);
        }
        s.fullscreen_changed(id, on)
    });
    if toggled {
        let _ = window.set_fullscreen(on);
        show_active(app, true);
        changed(app);
    }
}

// ---------- Memory pressure ----------

#[cfg_attr(not(windows), allow(dead_code))]
pub fn hidden_page_checked(app: &AppHandle, id: TabId, generation: u64, releasable: bool) {
    with(app, |s| s.hidden_page_checked(id, generation, releasable));
}

/// Closes the view of the least recently shown background page that holds
/// nothing to lose (see `Session::release_candidate`). The tab stays in the
/// list and loads again when selected. Returns whether a page was released.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn relieve_memory_pressure(app: &AppHandle) -> bool {
    let released = with(app, |s| {
        let (id, generation) = s.release_candidate()?;
        if !s.release(id, generation) {
            return None;
        }
        // Drop ownership in the same lock, before another thread can recreate it.
        let page = shell(app).views.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        Some((id, page))
    });
    let Some((id, page)) = released else { return false };
    info!("memory is low: released the page of background tab {id}");
    if let Some(page) = page {
        let _ = page.close();
    }
    publish(app);
    true
}

// ---------- Window placement ----------

fn overlaps(placement: &WindowPlacement, monitor: &tauri::Monitor) -> bool {
    let (mx, my) = (monitor.position().x as i64, monitor.position().y as i64);
    let (mw, mh) = (monitor.size().width as i64, monitor.size().height as i64);
    let (x, y, w, h) = (placement.x as i64, placement.y as i64, placement.width as i64, placement.height as i64);
    let overlap_x = (x + w).min(mx + mw) - x.max(mx);
    let overlap_y = (y + h).min(my + mh) - y.max(my);
    overlap_x >= MIN_VISIBLE && overlap_y >= MIN_VISIBLE
}

/// Restores the saved placement when it is still on a connected monitor;
/// otherwise centers the window.
pub fn restore_placement(app: &AppHandle, window: &Window) {
    let saved = with(app, |s| s.window_placement());
    let monitors = window.available_monitors().unwrap_or_default();
    match saved.filter(|p| monitors.iter().any(|m| overlaps(p, m))) {
        Some(p) => {
            let _ = window.set_size(PhysicalSize::new(p.width, p.height));
            let _ = window.set_position(PhysicalPosition::new(p.x, p.y));
            if p.maximized {
                let _ = window.maximize();
            }
        }
        None => {
            if saved.is_some() {
                info!("saved window position is off-screen; centering");
            }
            let _ = window.center();
        }
    }
}

pub fn record_placement(app: &AppHandle) {
    let Some(window) = window(app) else { return };
    if window.is_minimized().unwrap_or(false) || window.is_fullscreen().unwrap_or(false) {
        return;
    }
    let maximized = window.is_maximized().unwrap_or(false);
    let previous = with(app, |s| s.window_placement());
    let placement = match (maximized, previous) {
        // Keep the restored geometry while maximized, like Windows does.
        (true, Some(p)) => WindowPlacement { maximized: true, ..p },
        _ => {
            let (Ok(position), Ok(size)) = (window.outer_position(), window.inner_size()) else { return };
            WindowPlacement { x: position.x, y: position.y, width: size.width, height: size.height, maximized }
        }
    };
    with(app, |s| s.set_window_placement(placement));
    let _ = shell(app).save_wake.try_send(());
}

/// A second launch forwards its arguments here.
pub fn second_instance(app: &AppHandle, args: Vec<String>) {
    info!("second launch with {} argument(s)", args.len().saturating_sub(1));
    if with(app, |s| s.open_launch_urls(&args)) {
        after_tab_change(app);
    }
    if let Some(window) = window(app) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub fn open_folder(app: &AppHandle, path: &str) {
    let folder = with(app, |s| s.download_folder(path)).filter(|f| f.is_dir());
    let Some(folder) = folder else {
        with(app, |s| s.set_notice(notices::FOLDER_GONE));
        publish(app);
        return;
    };
    #[cfg(windows)]
    let opened = std::process::Command::new("explorer").arg(&folder).spawn();
    #[cfg(not(windows))]
    let opened = std::process::Command::new("xdg-open").arg(&folder).spawn();
    if let Err(e) = opened {
        error!("opening {} failed: {e}", folder.display());
        with(app, |s| s.set_notice(notices::FOLDER_FAILED));
        publish(app);
    }
}
