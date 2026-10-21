//! Updates from the download site (`site/build.py`). The updater plugin reads
//! `update/windows.json`, checks the installer's signature against the public
//! key in tauri.conf.json, then runs the installer and exits; the installer
//! starts Still again with the same arguments.

use crate::logger::{error, info};
use crate::shell;
use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    current: String,
    /// The newer version on the site, if any.
    available: Option<String>,
    /// The site could not be reached or its answer was not valid.
    failed: bool,
}

/// The update found by the last check, kept until the user installs it.
#[derive(Default)]
pub struct Pending(Mutex<Option<Update>>);

fn pending(app: &AppHandle) -> std::sync::MutexGuard<'_, Option<Update>> {
    app.state::<Pending>().inner().0.lock().unwrap_or_else(|e| e.into_inner())
}

pub async fn check(app: &AppHandle) -> UpdateStatus {
    let current = app.package_info().version.to_string();
    match async { app.updater()?.check().await }.await {
        Ok(update) => {
            let available = update.as_ref().map(|u| u.version.clone());
            *pending(app) = update;
            UpdateStatus { current, available, failed: false }
        }
        Err(e) => {
            error!("update check failed: {e}");
            UpdateStatus { current, available: None, failed: true }
        }
    }
}

/// Downloads and verifies the pending update, saves the session and starts the
/// installer. On Windows this does not return when it succeeds.
pub async fn install(app: &AppHandle) -> Result<(), String> {
    let update = pending(app).clone().ok_or("no update to install")?;
    info!("updating to {}", update.version);
    let fail = |e: tauri_plugin_updater::Error| {
        error!("update to {} failed: {e}", update.version);
        "update failed".to_string()
    };
    let bytes = update.download(|_, _| {}, || {}).await.map_err(fail)?;
    // The plugin exits the process right after starting the installer, so
    // RunEvent::Exit never comes: save now.
    shell::flush(app);
    update.install(bytes).map_err(fail)
}
