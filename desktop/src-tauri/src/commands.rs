//! IPC for the interface. Only the local "chrome" webview may call these: pages
//! in tabs and popups get no capability, and every command checks the caller too.

use crate::shell::{self, with, CHROME};
use crate::updates::{self, UpdateStatus};
use still_core::session::{Bounds, Snapshot, TabId};
use still_core::BLANK;
use tauri::{AppHandle, Webview};

type CommandResult<T = ()> = Result<T, String>;

fn guard(caller: &Webview) -> CommandResult {
    if caller.label() == CHROME {
        Ok(())
    } else {
        Err("not allowed".into())
    }
}

#[tauri::command]
pub async fn ready(app: AppHandle, webview: Webview) -> CommandResult<Snapshot> {
    guard(&webview)?;
    Ok(with(&app, |s| s.snapshot(true)))
}

#[tauri::command]
pub async fn set_content_bounds(app: AppHandle, webview: Webview, bounds: Bounds) -> CommandResult {
    guard(&webview)?;
    if ![bounds.x, bounds.y, bounds.width, bounds.height].iter().all(|v| v.is_finite()) {
        return Err("invalid bounds".into());
    }
    with(&app, |s| s.set_bounds(bounds));
    shell::show_active(&app, false);
    Ok(())
}

#[tauri::command]
pub async fn navigate(app: AppHandle, webview: Webview, input: String) -> CommandResult {
    guard(&webview)?;
    shell::navigate(&app, &input);
    Ok(())
}

#[tauri::command]
pub async fn new_tab(app: AppHandle, webview: Webview) -> CommandResult {
    guard(&webview)?;
    shell::open_tab(&app, BLANK);
    Ok(())
}

#[tauri::command]
pub async fn close_tab(app: AppHandle, webview: Webview, id: TabId) -> CommandResult {
    guard(&webview)?;
    shell::close(&app, id);
    Ok(())
}

#[tauri::command]
pub async fn select_tab(app: AppHandle, webview: Webview, id: TabId) -> CommandResult {
    guard(&webview)?;
    shell::select(&app, id);
    Ok(())
}

#[tauri::command]
pub async fn cycle_tab(app: AppHandle, webview: Webview, forward: bool) -> CommandResult {
    guard(&webview)?;
    shell::run_shortcut(&app, if forward { shell::Shortcut::NextTab } else { shell::Shortcut::PreviousTab });
    Ok(())
}

#[tauri::command]
pub async fn go(app: AppHandle, webview: Webview, action: String) -> CommandResult {
    guard(&webview)?;
    shell::go(&app, &action);
    Ok(())
}

#[tauri::command]
pub async fn set_sidebar(app: AppHandle, webview: Webview, open: bool) -> CommandResult {
    guard(&webview)?;
    with(&app, |s| s.set_sidebar(open));
    shell::changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn toggle_bookmark(app: AppHandle, webview: Webview) -> CommandResult {
    guard(&webview)?;
    let _ = with(&app, |s| s.toggle_bookmark());
    shell::changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn remove_bookmark(app: AppHandle, webview: Webview, url: String) -> CommandResult {
    guard(&webview)?;
    with(&app, |s| s.remove_bookmark(&url));
    shell::changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn open_download_folder(app: AppHandle, webview: Webview, path: String) -> CommandResult {
    guard(&webview)?;
    shell::open_folder(&app, &path);
    Ok(())
}

#[tauri::command]
pub async fn dismiss_notice(app: AppHandle, webview: Webview) -> CommandResult {
    guard(&webview)?;
    with(&app, |s| s.dismiss_notice());
    shell::publish(&app);
    Ok(())
}

#[tauri::command]
pub async fn set_settings_open(app: AppHandle, webview: Webview, open: bool) -> CommandResult {
    guard(&webview)?;
    shell::set_settings_open(&app, open);
    Ok(())
}

/// `patch` holds only the changed fields (PascalCase, as saved).
#[tauri::command]
pub async fn update_settings(app: AppHandle, webview: Webview, patch: serde_json::Map<String, serde_json::Value>) -> CommandResult {
    guard(&webview)?;
    shell::update_settings(&app, &patch);
    Ok(())
}

#[tauri::command]
pub async fn pick_download_folder(app: AppHandle, webview: Webview) -> CommandResult {
    guard(&webview)?;
    shell::pick_download_folder(&app);
    Ok(())
}

/// The body is the picture file's raw bytes; an empty body removes the picture.
#[tauri::command]
pub async fn set_new_tab_image(app: AppHandle, webview: Webview, request: tauri::ipc::Request<'_>) -> CommandResult {
    guard(&webview)?;
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else { return Err("expected the picture's bytes".into()) };
    shell::set_new_tab_image(&app, bytes);
    Ok(())
}

/// Raw bytes of the current picture, empty when none is set.
#[tauri::command]
pub async fn new_tab_image(app: AppHandle, webview: Webview) -> CommandResult<tauri::ipc::Response> {
    guard(&webview)?;
    Ok(tauri::ipc::Response::new(shell::new_tab_image(&app)))
}

#[tauri::command]
pub async fn clear_browsing_data(app: AppHandle, webview: Webview, history: bool, site_data: bool, cache: bool) -> CommandResult {
    guard(&webview)?;
    shell::clear_browsing_data(&app, history, site_data, cache);
    Ok(())
}

#[tauri::command]
pub async fn check_update(app: AppHandle, webview: Webview) -> CommandResult<UpdateStatus> {
    guard(&webview)?;
    Ok(updates::check(&app).await)
}

#[tauri::command]
pub async fn install_update(app: AppHandle, webview: Webview) -> CommandResult {
    guard(&webview)?;
    updates::install(&app).await
}

/// Browser profiles on this computer that Still can import from.
#[tauri::command]
pub async fn import_sources(webview: Webview) -> CommandResult<Vec<shell::SourceView>> {
    guard(&webview)?;
    Ok(tauri::async_runtime::spawn_blocking(shell::import_sources).await.unwrap_or_default())
}

#[tauri::command]
pub async fn import_from(app: AppHandle, webview: Webview, id: String, bookmarks: bool, history: bool, cookies: bool) -> CommandResult {
    guard(&webview)?;
    let _ = tauri::async_runtime::spawn_blocking(move || shell::import_from(&app, &id, bookmarks, history, cookies)).await;
    Ok(())
}

/// The body is an exported file's raw bytes (bookmarks HTML, passwords CSV or
/// Google Takeout history JSON); Rust recognizes the format.
#[tauri::command]
pub async fn import_file(app: AppHandle, webview: Webview, request: tauri::ipc::Request<'_>) -> CommandResult {
    guard(&webview)?;
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else { return Err("expected the file's bytes".into()) };
    shell::import_file(&app, bytes);
    Ok(())
}

/// Fills a saved login into the active page; refused unless it belongs to that site.
#[tauri::command]
pub async fn fill_login(app: AppHandle, webview: Webview, index: usize) -> CommandResult {
    guard(&webview)?;
    shell::fill_login(&app, index);
    Ok(())
}

#[tauri::command]
pub async fn remove_login(app: AppHandle, webview: Webview, index: usize) -> CommandResult {
    guard(&webview)?;
    with(&app, |s| s.remove_login(index));
    shell::changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn clear_logins(app: AppHandle, webview: Webview) -> CommandResult {
    guard(&webview)?;
    with(&app, |s| s.clear_logins());
    shell::changed(&app);
    Ok(())
}
