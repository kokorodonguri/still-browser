// Still: a minimal browser. The window holds one local "chrome" webview for the
// interface and one child webview per open page, so every control is drawn in
// HTML/CSS. See ../DESIGN.md for the architecture, data flow and edge cases.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod logger;
#[cfg(windows)]
mod platform;
mod shell;
mod updates;

use logger::{error, info};
use shell::{Shell, CHROME, WINDOW};
use tauri::{LogicalPosition, Manager, RunEvent, WebviewBuilder, WebviewUrl, WindowBuilder, WindowEvent};

fn data_dir() -> std::path::PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_DATA_HOME").map(std::path::PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share")));
    base.unwrap_or_else(std::env::temp_dir).join("StillBrowser")
}

fn main() {
    let base = data_dir();
    logger::init(base.join("logs"));
    info!("Still {} starting", env!("CARGO_PKG_VERSION"));

    #[cfg(windows)]
    if !platform::ensure_runtime() {
        error!("WebView2 Runtime missing; exiting");
        return;
    }

    let args: Vec<String> = std::env::args().collect();
    let app = tauri::Builder::default()
        // Must be the first plugin: a second launch hands over its URLs and exits,
        // so two processes never write the same state file.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| shell::second_instance(app, args)))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(updates::Pending::default())
        .invoke_handler(tauri::generate_handler![
            commands::ready,
            commands::set_content_bounds,
            commands::navigate,
            commands::new_tab,
            commands::close_tab,
            commands::select_tab,
            commands::cycle_tab,
            commands::go,
            commands::set_sidebar,
            commands::toggle_bookmark,
            commands::remove_bookmark,
            commands::open_download_folder,
            commands::dismiss_notice,
            commands::set_settings_open,
            commands::update_settings,
            commands::pick_download_folder,
            commands::set_new_tab_image,
            commands::new_tab_image,
            commands::clear_browsing_data,
            commands::check_update,
            commands::install_update,
            commands::import_sources,
            commands::import_from,
            commands::import_file,
            commands::fill_login,
            commands::remove_login,
            commands::clear_logins
        ])
        .setup(move |app| {
            let (state, workers) = Shell::load(&base, &args);
            app.manage(state);
            shell::start_workers(app.handle(), workers);
            #[cfg(windows)]
            platform::watch_memory(app.handle().clone());

            let window = WindowBuilder::new(app, WINDOW)
                .title("Still")
                .inner_size(1440.0, 900.0)
                .min_inner_size(732.0, 500.0)
                .decorations(false)
                .shadow(true)
                .visible(false)
                .build()?;
            shell::restore_placement(app.handle(), &window);

            // The interface uses its own WebView2 folder, so it survives a crash
            // of the page engine and can recreate the tabs.
            let chrome = WebviewBuilder::new(CHROME, WebviewUrl::App("index.html".into()))
                .data_directory(base.join("UI"))
                .auto_resize();
            window.add_child(chrome, LogicalPosition::new(0.0, 0.0), window.inner_size()?)?;
            window.show()?;

            let handle = app.handle().clone();
            window.on_window_event(move |event| {
                let app = handle.clone();
                match event {
                    WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
                        tauri::async_runtime::spawn(async move { shell::record_placement(&app) });
                    }
                    WindowEvent::ScaleFactorChanged { .. } => {
                        tauri::async_runtime::spawn(async move { shell::show_active(&app, true) });
                    }
                    _ => {}
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!());

    let app = match app {
        Ok(app) => app,
        Err(e) => {
            error!("startup failed: {e}");
            std::process::exit(1);
        }
    };
    app.run(|app, event| {
        if let RunEvent::Exit = event {
            shell::flush(app);
            info!("exit");
        }
    });
}
