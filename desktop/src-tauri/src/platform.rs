//! WebView2 hooks that Tauri does not expose: browser shortcuts while a page has
//! focus, real back/forward state, crash and fullscreen events, background-tab
//! throttling, releasing background pages when Windows runs low on memory,
//! page color scheme, clearing site data, the save and folder dialogs, the
//! startup check for the WebView2 Runtime, saving new passwords, encrypting
//! imported logins (DPAPI), filling them in, and importing cookies.

use crate::logger::{error, warning};
use crate::shell::{self, Shortcut};
use still_core::import::{Cookie, SameSite};
use still_core::session::ProcessFailure;
use still_core::settings::Theme;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, Webview};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use std::time::Duration;
use webview2_com::{
    take_pwstr, AcceleratorKeyPressedEventHandler, ClearBrowsingDataCompletedHandler,
    ContainsFullScreenElementChangedEventHandler, DownloadStartingEventHandler, ExecuteScriptCompletedHandler,
    HistoryChangedEventHandler, ProcessFailedEventHandler, TrySuspendCompletedHandler,
};
use windows::core::{w, Interface, BOOL, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, LocalFree, HLOCAL, HWND, WAIT_OBJECT_0};
use windows::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER};
use windows::Win32::System::Memory::{CreateMemoryResourceNotification, LowMemoryResourceNotification};
use windows::Win32::System::Threading::{WaitForSingleObject, INFINITE};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VK_CONTROL, VK_L, VK_MENU, VK_OEM_COMMA, VK_SHIFT, VK_T, VK_TAB, VK_W,
};
use windows::Win32::UI::Shell::{
    FileOpenDialog, FileSaveDialog, IFileDialog, IFileOpenDialog, IFileSaveDialog, IShellItem,
    SHCreateItemFromParsingName, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDYES, MB_ICONWARNING, MB_YESNO};

const RUNTIME_DOWNLOAD: &str = "https://go.microsoft.com/fwlink/p/?LinkId=2124703";

/// Returns false (after telling the user) when the WebView2 Runtime is missing.
pub fn ensure_runtime() -> bool {
    let mut version = PWSTR::null();
    let found = unsafe { GetAvailableCoreWebView2BrowserVersionString(PCWSTR::null(), &mut version) }.is_ok()
        && !version.is_null()
        && !take_pwstr(version).is_empty();
    if !found {
        let answer = unsafe {
            MessageBoxW(
                None,
                w!("Still needs the Microsoft Edge WebView2 Runtime, which is not installed on this PC.\n\nOpen the download page now?"),
                w!("Still"),
                MB_YESNO | MB_ICONWARNING,
            )
        };
        if answer == IDYES {
            let _ = std::process::Command::new("explorer").arg(RUNTIME_DOWNLOAD).spawn();
        }
    }
    found
}

fn pressed(key: u16) -> bool {
    unsafe { GetKeyState(key as i32) < 0 }
}

fn shortcut(key: u32) -> Option<Shortcut> {
    if !pressed(VK_CONTROL.0) || pressed(VK_MENU.0) {
        return None;
    }
    let shift = pressed(VK_SHIFT.0);
    match key as u16 {
        k if k == VK_L.0 && !shift => Some(Shortcut::FocusAddress),
        k if k == VK_T.0 && !shift => Some(Shortcut::NewTab),
        k if k == VK_W.0 && !shift => Some(Shortcut::CloseTab),
        k if k == VK_TAB.0 => Some(if shift { Shortcut::PreviousTab } else { Shortcut::NextTab }),
        k if k == VK_OEM_COMMA.0 && !shift => Some(Shortcut::Settings),
        _ => None,
    }
}

/// Every callback leaves the WebView2 event first (spawn) before touching
/// other webviews, so no COM re-entrancy reaches the shell.
pub fn attach(app: AppHandle, view: &Webview, id: u64, generation: u64) {
    let _ = view.with_webview(move |platform| unsafe {
        let controller = platform.controller();
        let mut token = 0i64;

        let keys = app.clone();
        let _ = controller.add_AcceleratorKeyPressed(
            &AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut kind = COREWEBVIEW2_KEY_EVENT_KIND::default();
                args.KeyEventKind(&mut kind)?;
                if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN && kind != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN {
                    return Ok(());
                }
                let mut key = 0u32;
                args.VirtualKey(&mut key)?;
                if let Some(action) = shortcut(key) {
                    args.SetHandled(true)?;
                    let app = keys.clone();
                    tauri::async_runtime::spawn(async move { shell::run_shortcut(&app, action) });
                }
                Ok(())
            })),
            &mut token,
        );

        let Ok(core) = controller.CoreWebView2() else { return };
        set_color_scheme(&core, shell::with(&app, |s| s.settings().theme));
        // WebView2 offers to save passwords typed into sign-in forms (including
        // ones Still filled in) and fills them in next time on its own.
        if let Ok(settings) = core.Settings().and_then(|s| s.cast::<ICoreWebView2Settings4>()) {
            let _ = settings.SetIsPasswordAutosaveEnabled(true);
            let _ = settings.SetIsGeneralAutofillEnabled(true);
        }

        // Runs after wry's own handler, which has already applied the folder
        // from settings. The dialog waits for the handlers to return.
        if let Ok(core) = core.cast::<ICoreWebView2_4>() {
            let downloads = app.clone();
            let _ = core.add_DownloadStarting(
                &DownloadStartingEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()) };
                    if !shell::with(&downloads, |s| s.settings().ask_download_folder) {
                        return Ok(());
                    }
                    let pending = MainThreadOnly((args.clone(), args.GetDeferral()?));
                    let app = downloads.clone();
                    tauri::async_runtime::spawn(async move {
                        let task = app.clone();
                        let _ = app.run_on_main_thread(move || {
                            let (args, deferral) = pending.into_inner();
                            ask_where_to_save(&task, &args);
                            let _ = deferral.Complete();
                        });
                    });
                    Ok(())
                })),
                &mut token,
            );
        }

        let history = app.clone();
        let _ = core.add_HistoryChanged(
            &HistoryChangedEventHandler::create(Box::new(move |sender, _| {
                let Some(core) = sender else { return Ok(()) };
                let (mut back, mut forward) = (BOOL::default(), BOOL::default());
                core.CanGoBack(&mut back)?;
                core.CanGoForward(&mut forward)?;
                let mut source = PWSTR::null();
                core.Source(&mut source)?;
                let url = take_pwstr(source);
                let app = history.clone();
                tauri::async_runtime::spawn(async move { shell::history_changed(&app, id, generation, back.as_bool(), forward.as_bool(), url) });
                Ok(())
            })),
            &mut token,
        );

        let crashes = app.clone();
        let _ = core.add_ProcessFailed(
            &ProcessFailedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                args.ProcessFailedKind(&mut kind)?;
                let failure = match kind {
                    COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED => ProcessFailure::RendererExited,
                    COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE => ProcessFailure::RendererUnresponsive,
                    COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED => ProcessFailure::BrowserExited,
                    // GPU/utility/frame processes restart on their own.
                    _ => return Ok(()),
                };
                let app = crashes.clone();
                tauri::async_runtime::spawn(async move { shell::process_failed(&app, id, generation, failure) });
                Ok(())
            })),
            &mut token,
        );

        let fullscreen = app.clone();
        let _ = core.add_ContainsFullScreenElementChanged(
            &ContainsFullScreenElementChangedEventHandler::create(Box::new(move |sender, _| {
                let Some(core) = sender else { return Ok(()) };
                let mut on = BOOL::default();
                core.ContainsFullScreenElement(&mut on)?;
                let app = fullscreen.clone();
                tauri::async_runtime::spawn(async move { shell::page_fullscreen(&app, id, generation, on.as_bool()) });
                Ok(())
            })),
            &mut token,
        );
    });
}

/// Evaluates to `true` only when nothing on the page would be lost by
/// closing it: no field differs from its initial value, no editable region,
/// no unload warning, and no frame it cannot inspect (cross-origin frames
/// might hold edits). Any doubt, including an exception, keeps the page.
const EDIT_CHECK: &str = r#"(() => {
  const edited = (doc) => {
    if (doc.designMode === "on" || doc.querySelector('[contenteditable]:not([contenteditable="false"])')) return true;
    for (const el of doc.querySelectorAll("input, textarea, select")) {
      if (el.disabled) continue;
      if (el.tagName === "SELECT") { for (const o of el.options) if (o.selected !== o.defaultSelected) return true; }
      else if (el.type === "checkbox" || el.type === "radio") { if (el.checked !== el.defaultChecked) return true; }
      else if (el.type === "file") { if (el.files && el.files.length) return true; }
      else if (el.type !== "hidden" && el.value !== el.defaultValue) return true;
    }
    for (const frame of doc.querySelectorAll("iframe, frame")) {
      let inner = null;
      try { inner = frame.contentDocument; } catch (_) {}
      if (!inner || edited(inner)) return true;
    }
    return false;
  };
  try { return !window.onbeforeunload && !edited(document); } catch (_) { return false; }
})()"#;

/// While Windows reports low memory, one checked background page is released
/// per interval, least recently shown first.
const PRESSURE_INTERVAL: Duration = Duration::from_secs(5);

fn set_memory_target(core: &ICoreWebView2, level: COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL) {
    if let Ok(core) = core.cast::<ICoreWebView2_19>() {
        let _ = unsafe { core.SetMemoryUsageTargetLevel(level) };
    }
}

fn suspend(core: &ICoreWebView2) {
    if let Ok(core) = core.cast::<ICoreWebView2_3>() {
        let _ = unsafe { core.TrySuspend(&TrySuspendCompletedHandler::create(Box::new(|_, _| Ok(())))) };
    }
}

/// A hidden tab asks WebView2 to release caches and, unless it is playing
/// audio, records whether the page holds user edits and then suspends script
/// timers and rendering. A suspended page cannot change, so the record stays
/// true until it is shown again; it resumes by itself, keeping its scroll
/// position and form state.
pub fn rest(app: &AppHandle, view: &Webview, id: u64, generation: u64) {
    let app = app.clone();
    let _ = view.with_webview(move |platform| unsafe {
        let Ok(core) = platform.controller().CoreWebView2() else { return };
        set_memory_target(&core, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW);
        let mut audible = BOOL::default();
        if let Ok(core) = core.cast::<ICoreWebView2_8>() {
            let _ = core.IsDocumentPlayingAudio(&mut audible);
        }
        if audible.as_bool() {
            // Keeps playing, and is never released.
            return;
        }
        let target = core.clone();
        let checked = core.ExecuteScript(
            &HSTRING::from(EDIT_CHECK),
            &ExecuteScriptCompletedHandler::create(Box::new(move |result, json| {
                let releasable = result.is_ok() && json == "true";
                tauri::async_runtime::spawn(async move { shell::hidden_page_checked(&app, id, generation, releasable) });
                suspend(&target);
                Ok(())
            })),
        );
        if checked.is_err() {
            suspend(&core);
        }
    });
}

/// The page is shown again: back to the normal memory target.
pub fn wake(view: &Webview) {
    let _ = view.with_webview(|platform| unsafe {
        if let Ok(core) = platform.controller().CoreWebView2() {
            set_memory_target(&core, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL);
        }
    });
}

/// Waits in the kernel for Windows' low-memory notification (set when
/// available physical memory is low), so it costs no CPU in normal use.
pub fn watch_memory(app: AppHandle) {
    let spawned = std::thread::Builder::new().name("still-memory".into()).spawn(move || {
        let handle = match unsafe { CreateMemoryResourceNotification(LowMemoryResourceNotification) } {
            Ok(handle) => handle,
            Err(e) => {
                warning!("low-memory notifications are unavailable: {e}");
                return;
            }
        };
        while unsafe { WaitForSingleObject(handle, INFINITE) } == WAIT_OBJECT_0 {
            shell::relieve_memory_pressure(&app);
            std::thread::sleep(PRESSURE_INTERVAL);
        }
        let _ = unsafe { CloseHandle(handle) };
    });
    if let Err(e) = spawned {
        error!("memory watcher could not start: {e}");
    }
}

/// COM objects that are only touched again on the main thread, where they
/// were created; the closure that carries them there must be `Send`.
struct MainThreadOnly<T>(T);
unsafe impl<T> Send for MainThreadOnly<T> {}

impl<T> MainThreadOnly<T> {
    /// A method call moves the whole wrapper into closures (field capture would not).
    fn into_inner(self) -> T {
        self.0
    }
}

fn main_hwnd(app: &AppHandle) -> Option<HWND> {
    app.get_window(shell::WINDOW).and_then(|w| w.hwnd().ok()).map(|h| HWND(h.0))
}

/// Shows the save dialog for a starting download. Main thread only.
fn ask_where_to_save(app: &AppHandle, args: &ICoreWebView2DownloadStartingEventArgs) {
    let mut path = PWSTR::null();
    if unsafe { args.ResultFilePath(&mut path) }.is_err() {
        return;
    }
    let suggested = PathBuf::from(take_pwstr(path));
    let from = suggested.to_string_lossy().into_owned();
    match save_dialog(main_hwnd(app), &suggested) {
        Some(chosen) => {
            if unsafe { args.SetResultFilePath(&HSTRING::from(chosen.as_path())) }.is_ok() {
                shell::with(app, |s| s.download_redirected(&from, chosen.to_string_lossy().into_owned()));
            }
        }
        None => {
            let _ = unsafe { args.SetCancel(true) };
            shell::with(app, |s| s.download_cancelled(&from));
        }
    }
    shell::changed(app);
}

/// Runs a shell file dialog; `None` when cancelled. Main thread only (STA).
unsafe fn run_dialog(dialog: &IFileDialog, owner: Option<HWND>, folder: Option<&Path>) -> Option<PathBuf> {
    let options = dialog.GetOptions().ok()?;
    dialog.SetOptions(options | FOS_FORCEFILESYSTEM).ok()?;
    if let Some(folder) = folder.filter(|f| f.is_dir()) {
        if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(folder), None) {
            let _ = dialog.SetFolder(&item);
        }
    }
    dialog.Show(owner).ok()?;
    let name = dialog.GetResult().ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
    let path = name.to_string().ok().map(PathBuf::from);
    CoTaskMemFree(Some(name.0 as _));
    path
}

fn save_dialog(owner: Option<HWND>, suggested: &Path) -> Option<PathBuf> {
    unsafe {
        let dialog: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        if let Some(name) = suggested.file_name() {
            let _ = dialog.SetFileName(&HSTRING::from(name));
        }
        run_dialog(&dialog, owner, suggested.parent())
    }
}

/// The folder picker for the download folder setting. Main thread only.
pub fn pick_folder(app: &AppHandle, current: &str) -> Option<PathBuf> {
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        dialog.SetOptions(dialog.GetOptions().ok()? | FOS_PICKFOLDERS).ok()?;
        run_dialog(&dialog, main_hwnd(app), (!current.is_empty()).then(|| Path::new(current)))
    }
}

/// Pages follow the theme chosen in settings (`prefers-color-scheme`).
/// The setting belongs to the profile, so one page applies it to all.
fn set_color_scheme(core: &ICoreWebView2, theme: Theme) {
    let scheme = match theme {
        Theme::System => COREWEBVIEW2_PREFERRED_COLOR_SCHEME_AUTO,
        Theme::Light => COREWEBVIEW2_PREFERRED_COLOR_SCHEME_LIGHT,
        Theme::Dark => COREWEBVIEW2_PREFERRED_COLOR_SCHEME_DARK,
    };
    unsafe {
        if let Ok(profile) = core.cast::<ICoreWebView2_13>().and_then(|c| c.Profile()) {
            let _ = profile.SetPreferredColorScheme(scheme);
        }
    }
}

pub fn apply_color_scheme(view: &Webview, theme: Theme) {
    let _ = view.with_webview(move |platform| unsafe {
        if let Ok(core) = platform.controller().CoreWebView2() {
            set_color_scheme(&core, theme);
        }
    });
}

/// Clears cookies and site storage and/or the disk cache of the page profile.
/// Reports the result through `shell::browsing_data_cleared`.
pub fn clear_browsing_data(app: AppHandle, view: &Webview, site_data: bool, cache: bool) {
    let mut kinds = COREWEBVIEW2_BROWSING_DATA_KINDS(0);
    if site_data {
        kinds.0 |= COREWEBVIEW2_BROWSING_DATA_KINDS_ALL_SITE.0;
    }
    if cache {
        kinds.0 |= COREWEBVIEW2_BROWSING_DATA_KINDS_DISK_CACHE.0;
    }
    let report = app.clone();
    let sent = view.with_webview(move |platform| unsafe {
        let profile = platform
            .controller()
            .CoreWebView2()
            .and_then(|c| c.cast::<ICoreWebView2_13>())
            .and_then(|c| c.Profile())
            .and_then(|p| p.cast::<ICoreWebView2Profile2>());
        let done = app.clone();
        let started = profile.and_then(|profile| {
            profile.ClearBrowsingData(
                kinds,
                &ClearBrowsingDataCompletedHandler::create(Box::new(move |result| {
                    tauri::async_runtime::spawn(async move { shell::browsing_data_cleared(&done, result.is_ok()) });
                    Ok(())
                })),
            )
        });
        if started.is_err() {
            tauri::async_runtime::spawn(async move { shell::browsing_data_cleared(&app, false) });
        }
    });
    if sent.is_err() {
        shell::browsing_data_cleared(&report, false);
    }
}

// ---------- Logins and cookies from other browsers ----------

/// Encrypts `data` so only this Windows user can read it (DPAPI).
pub fn protect(data: &[u8]) -> Option<Vec<u8>> {
    crypt(data, true)
}

pub fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
    crypt(data, false)
}

fn crypt(data: &[u8], seal: bool) -> Option<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB { cbData: u32::try_from(data.len()).ok()?, pbData: data.as_ptr() as *mut u8 };
    let mut output = CRYPT_INTEGER_BLOB::default();
    let done = unsafe {
        if seal {
            CryptProtectData(&input, w!("Still logins"), None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut output)
        } else {
            CryptUnprotectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut output)
        }
    };
    if let Err(e) = done {
        error!("{} failed: {e}", if seal { "encrypting logins" } else { "decrypting logins" });
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
    unsafe {
        let _ = LocalFree(Some(HLOCAL(output.pbData as _)));
    }
    Some(bytes)
}

/// Runs a login fill script in the page; says so when the page had no
/// sign-in field.
pub fn fill(app: AppHandle, view: &Webview, script: String) {
    let _ = view.with_webview(move |platform| unsafe {
        let Ok(core) = platform.controller().CoreWebView2() else { return };
        let _ = core.ExecuteScript(
            &HSTRING::from(script),
            &ExecuteScriptCompletedHandler::create(Box::new(move |result, json| {
                if result.is_err() || json != "true" {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move { shell::fill_found_nothing(&app) });
                }
                Ok(())
            })),
        );
    });
}

/// Adds cookies to the page profile and reports how many WebView2 accepted.
pub fn import_cookies(app: AppHandle, view: &Webview, cookies: Vec<Cookie>, done: impl FnOnce(&AppHandle, usize) + Send + 'static) {
    let report = app.clone();
    let done = std::sync::Arc::new(std::sync::Mutex::new(Some(done)));
    let finish = done.clone();
    let sent = view.with_webview(move |platform| unsafe {
        let manager = platform.controller().CoreWebView2().and_then(|c| c.cast::<ICoreWebView2_2>()).and_then(|c| c.CookieManager());
        let mut added = 0;
        match manager {
            Ok(manager) => {
                for cookie in &cookies {
                    if add_cookie(&manager, cookie).is_ok() {
                        added += 1;
                    }
                }
            }
            Err(e) => error!("cookie manager unavailable: {e}"),
        }
        tauri::async_runtime::spawn(async move {
            if let Some(done) = finish.lock().unwrap_or_else(|e| e.into_inner()).take() {
                done(&app, added);
            }
        });
    });
    if sent.is_err() {
        if let Some(done) = done.lock().unwrap_or_else(|e| e.into_inner()).take() {
            done(&report, 0);
        }
    }
}

unsafe fn add_cookie(manager: &ICoreWebView2CookieManager, cookie: &Cookie) -> windows::core::Result<()> {
    let made = manager.CreateCookie(
        &HSTRING::from(cookie.name.as_str()),
        &HSTRING::from(cookie.value.as_str()),
        &HSTRING::from(cookie.domain.as_str()),
        &HSTRING::from(cookie.path.as_str()),
    )?;
    made.SetExpires(cookie.expires as f64)?;
    made.SetIsSecure(cookie.secure)?;
    made.SetIsHttpOnly(cookie.http_only)?;
    // Browsers refuse SameSite=None without Secure.
    made.SetSameSite(match cookie.same_site {
        SameSite::None if cookie.secure => COREWEBVIEW2_COOKIE_SAME_SITE_KIND_NONE,
        SameSite::Strict => COREWEBVIEW2_COOKIE_SAME_SITE_KIND_STRICT,
        _ => COREWEBVIEW2_COOKIE_SAME_SITE_KIND_LAX,
    })?;
    manager.AddOrUpdateCookie(&made)
}
