//! Still's Firefox profile: the stylesheets and first-run preferences, shared
//! with the Linux build (`linux/profile/`), and what carries over from the
//! earlier Windows build.

use crate::{migrate, newtab};
use std::fs;
use std::io;
use std::path::Path;

const USER_JS: &str = include_str!("../../../linux/profile/user.js");
const SHARED_CHROME: &str = include_str!("../../../linux/profile/chrome/userChrome.css");
/// Windows palette shared by the browser frame and its privileged settings page.
const WINDOWS_TOKENS: &str = include_str!("../../firefox/still/content/tokens.css");
/// Windows only: Still's frame over the shared Firefox styling.
const WINDOWS_CHROME: &str = include_str!("../../profile/userChrome.css");
const WINDOWS_CONTENT: &str = include_str!("../../profile/userContent.css");
const USER_CONTENT: &str = include_str!("../../../linux/profile/chrome/userContent.css");
const STILL_SVG: &[u8] = include_bytes!("../../../linux/profile/chrome/still.svg");

/// First-run preferences that differ from Linux: a light theme by default.
/// The toolbar layout is applied
/// by windows/firefox/still.cfg, since Firefox rebuilds the toolbar when it
/// turns on vertical tabs.
const WINDOWS_PREFERENCES: &str = r#"
// Still starts light: still.cfg enables the light theme once and clears this.
user_pref("still.lightThemePending", true);
// The sidebar's bookmarks and history, like the original sidebar's sections.
user_pref("sidebar.main.tools", "bookmarks,history");
// No "drag tabs here to pin" card at the top of the sidebar.
user_pref("sidebar.verticalTabs.dragToPinPromo.dismissed", true);
// The extensions button appears once an extension is installed.
user_pref("extensions.unifiedExtensions.button.always_visible", false);
user_pref("browser.newtabpage.activity-stream.feeds.topsites", false);
user_pref("still.windowsRevision", 2);
"#;

/// Fresh profiles show vertical tabs and Still's start page. Kept separate
/// from migrations to preserve existing sidebar and homepage choices.
const FIRST_RUN_INTERFACE: &str = r#"
user_pref("sidebar.backupState", "{\"launcherVisible\":true,\"launcherExpanded\":true,\"panelOpen\":false,\"expandedLauncherWidth\":200}");
user_pref("browser.startup.homepage", "chrome://still/content/newtab.html");
"#;

/// Profiles started by 0.5.0, which shared Linux's dark theme. Until Firefox
/// has saved this revision, the dark settings are taken out of prefs.js and
/// [WINDOWS_PREFERENCES] are applied again; after that the user's choices stand.
const REVISION: &str = "\"still.windowsRevision\", 2);";
const DARK_PREFERENCES: [&str; 3] = ["\"ui.systemUsesDarkTheme\"", "\"browser.theme.toolbar-theme\"", "\"browser.theme.content-theme\""];

/// The shared first-run preferences with the Linux-only dark theme taken out,
/// followed by [WINDOWS_PREFERENCES].
fn first_run_preferences() -> String {
    let shared: String = USER_JS.lines().filter(|line| !DARK_PREFERENCES.iter().any(|pref| line.contains(pref))).map(|line| format!("{line}\n")).collect();
    format!("{shared}{WINDOWS_PREFERENCES}{FIRST_RUN_INTERFACE}")
}

/// Written on every later start. Other preferences belong to the user once
/// Firefox has saved them, as on Linux.
const KEPT_PREFERENCES: &str = "// Still interface requirements; other preferences are saved by Firefox.\n\
user_pref(\"toolkit.legacyUserProfileCustomizations.stylesheets\", true);\n\
user_pref(\"browser.newtabpage.activity-stream.improvesearch.handoffToAwesomebar\", false);\n";

/// The application version belongs to the installed launcher, not the profile.
const VERSION_PREFERENCE: &str = concat!("user_pref(\"still.version\", \"", env!("CARGO_PKG_VERSION"), "\");\n");

const LAYOUT: &str = r#"{"chrome://browser/content/browser.xhtml":{"main-window":{"width":"1440","height":"900","sizemode":"normal"},"sidebar-box":{"width":"280"}}}"#;

/// Writes `content` unless the file already holds it. The file is replaced in
/// one step, so Firefox never reads half of it.
pub fn write_atomic(path: &Path, content: &[u8]) -> io::Result<()> {
    if fs::read(path).is_ok_and(|current| current == content) {
        return Ok(());
    }
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temporary, content)?;
    fs::rename(&temporary, path).inspect_err(|_| {
        let _ = fs::remove_file(&temporary);
    })
}

/// Prepares `profile` before Firefox starts. `legacy` is the earlier Windows
/// build's data folder: on the first start its bookmarks, history and new tab
/// picture are handed to Firefox.
pub fn prepare(profile: &Path, legacy: &Path) -> io::Result<()> {
    let chrome = profile.join("chrome");
    fs::create_dir_all(&chrome)?;
    write_atomic(&chrome.join("userChrome.css"), format!("{SHARED_CHROME}{WINDOWS_TOKENS}{WINDOWS_CHROME}").as_bytes())?;
    write_atomic(&chrome.join("still.svg"), STILL_SVG)?;
    let first_run = !profile.join("prefs.js").exists();
    if first_run {
        migrate::stage(legacy, profile);
    }
    write_user_content(profile)?;
    let mut preferences = if first_run {
        first_run_preferences()
    } else if upgrade_from_dark(profile)? {
        format!("{KEPT_PREFERENCES}{WINDOWS_PREFERENCES}")
    } else {
        KEPT_PREFERENCES.to_owned()
    };
    preferences.push_str(VERSION_PREFERENCE);
    write_atomic(&profile.join("user.js"), preferences.as_bytes())?;
    // Seeded once; later window sizes and toolbar choices are Firefox's.
    let layout = profile.join("xulstore.json");
    if !layout.exists() {
        write_atomic(&layout, LAYOUT.as_bytes())?;
    }
    Ok(())
}

/// True for a profile that has not yet saved [REVISION]; its dark-theme
/// settings are removed from prefs.js (Firefox is not running here).
fn upgrade_from_dark(profile: &Path) -> io::Result<bool> {
    let saved = fs::read_to_string(profile.join("prefs.js"))?;
    if saved.contains(REVISION) {
        return Ok(false);
    }
    let kept: String = saved.lines().filter(|line| !DARK_PREFERENCES.iter().any(|pref| line.contains(pref))).map(|line| format!("{line}\n")).collect();
    write_atomic(&profile.join("prefs.js"), kept.as_bytes())?;
    Ok(true)
}

/// Firefox's own about: pages. about:blank and about:srcdoc are excluded, since
/// websites use them for frames inside their pages.
const BUILT_IN_PAGES: &str = "about:(?!blank|srcdoc)[^#?]*([#?].*)?";

/// Still's stylesheet plus the new tab picture, if one is chosen.
pub fn write_user_content(profile: &Path) -> io::Result<()> {
    let style = format!("{USER_CONTENT}\n@-moz-document regexp(\"{BUILT_IN_PAGES}\"), url-prefix(\"chrome://still/content/\") {{\n{WINDOWS_TOKENS}\n}}\n{WINDOWS_CONTENT}{}", newtab::stylesheet(profile));
    write_atomic(&profile.join("chrome").join("userContent.css"), style.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("still-launcher-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn first_run_seeds_preferences_and_later_runs_keep_the_users() {
        let dir = temp("prefs");
        let profile = dir.join("Profile");
        prepare(&profile, &dir.join("none")).unwrap();
        let prefs = fs::read_to_string(profile.join("user.js")).unwrap();
        assert!(prefs.contains(VERSION_PREFERENCE));
        assert!(prefs.contains("\"browser.search.suggest.enabled\", true"));
        assert!(prefs.contains("\"sidebar.verticalTabs\", true"));
        let sidebar_pref = prefs.lines().find_map(|line| line.strip_prefix("user_pref(\"sidebar.backupState\", ").and_then(|value| value.strip_suffix(");"))).unwrap();
        let sidebar_json: String = serde_json::from_str(sidebar_pref).unwrap();
        let sidebar: serde_json::Value = serde_json::from_str(&sidebar_json).unwrap();
        assert_eq!(sidebar["launcherVisible"], true);
        assert_eq!(sidebar["launcherExpanded"], true);
        assert_eq!(sidebar["panelOpen"], false);
        assert_eq!(sidebar["expandedLauncherWidth"], 200);
        assert!(!prefs.contains("ui.systemUsesDarkTheme"), "Windows uses the chosen light theme");
        assert!(prefs.contains("\"still.lightThemePending\", true"));
        assert!(!prefs.contains("browser.theme.toolbar-theme"), "Firefox derives the theme preferences");
        let style = fs::read_to_string(profile.join("chrome/userChrome.css")).unwrap();
        assert!(style.starts_with(SHARED_CHROME) && style.ends_with(WINDOWS_CHROME));
        assert!(style.contains(WINDOWS_TOKENS));
        assert!(profile.join("xulstore.json").exists());
        // A profile from 0.5.0 loses the forced dark theme once.
        fs::write(profile.join("prefs.js"), "user_pref(\"sidebar.visibility\", \"always-show\");\nuser_pref(\"ui.systemUsesDarkTheme\", 1);\n").unwrap();
        fs::write(profile.join("xulstore.json"), "{}").unwrap();
        prepare(&profile, &dir.join("none")).unwrap();
        assert_eq!(fs::read_to_string(profile.join("prefs.js")).unwrap(), "user_pref(\"sidebar.visibility\", \"always-show\");\n");
        let prefs = fs::read_to_string(profile.join("user.js")).unwrap();
        assert!(prefs.contains("\"still.lightThemePending\", true"));
        assert!(!prefs.contains("sidebar.backupState"), "upgrading the theme keeps the user's sidebar state");
        assert!(prefs.contains(VERSION_PREFERENCE));
        // Once Firefox has saved the revision, the user's choices stand.
        fs::write(profile.join("prefs.js"), "user_pref(\"ui.systemUsesDarkTheme\", 1);\nuser_pref(\"still.windowsRevision\", 2);\n").unwrap();
        prepare(&profile, &dir.join("none")).unwrap();
        assert_eq!(fs::read_to_string(profile.join("user.js")).unwrap(), format!("{KEPT_PREFERENCES}{VERSION_PREFERENCE}"));
        assert!(fs::read_to_string(profile.join("prefs.js")).unwrap().contains("ui.systemUsesDarkTheme"));
        assert_eq!(fs::read_to_string(profile.join("xulstore.json")).unwrap(), "{}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn updates_the_application_version_without_changing_saved_preferences() {
        let dir = temp("version");
        let profile = dir.join("Profile");
        fs::create_dir_all(&profile).unwrap();
        let saved = "user_pref(\"still.windowsRevision\", 2);\n\
user_pref(\"still.version\", \"0.1.0\");\n\
user_pref(\"browser.theme.toolbar-theme\", 1);\n\
user_pref(\"sidebar.visibility\", \"hide-sidebar\");\n\
user_pref(\"sidebar.backupState\", \"{\\\"launcherVisible\\\":false,\\\"expandedLauncherWidth\\\":312}\");\n\
user_pref(\"browser.startup.homepage\", \"https://example.org/\");\n";
        fs::write(profile.join("prefs.js"), saved).unwrap();
        fs::write(profile.join("user.js"), "user_pref(\"still.version\", \"0.1.0\");\n").unwrap();

        prepare(&profile, &dir.join("none")).unwrap();

        let preferences = fs::read_to_string(profile.join("user.js")).unwrap();
        assert_eq!(preferences.matches("still.version").count(), 1);
        assert!(preferences.contains(&format!("user_pref(\"still.version\", \"{}\");", env!("CARGO_PKG_VERSION"))));
        assert!(!preferences.contains("browser.theme.toolbar-theme"));
        assert!(!preferences.contains("browser.startup.homepage"));
        assert_eq!(fs::read_to_string(profile.join("prefs.js")).unwrap(), saved);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_write_leaves_no_temporary_file() {
        let dir = temp("atomic");
        let file = dir.join("a.css");
        write_atomic(&file, b"one").unwrap();
        write_atomic(&file, b"two").unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"two");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        let _ = fs::remove_dir_all(&dir);
    }
}
