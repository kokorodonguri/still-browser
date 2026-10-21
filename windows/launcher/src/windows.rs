//! The launcher on Windows: prepares Still's profile, starts the bundled
//! Firefox, and installs Still's own updates.
//!
//! Layout (written by `windows/installer.nsi`):
//!   <install>\still.exe            this program
//!   <install>\firefox\firefox.exe  Mozilla's unmodified Firefox ESR
//!   %LOCALAPPDATA%\StillBrowser\Profile   Still's Firefox profile
//!   %LOCALAPPDATA%\StillBrowser\Update    a downloaded, verified installer
//!   %LOCALAPPDATA%\StillBrowser\Update.lock  the update lock (see staging.rs)

use crate::staging::{self, Outcome, StageError, UpdateLock};
use crate::{newtab, profile, update};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use windows_sys::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW};
use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDYES, MB_ICONINFORMATION, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MB_YESNO};

/// Firefox's own import wizard: Chrome, Edge, other Firefox profiles, a
/// passwords file (CSV) or a bookmarks file (HTML). The tab closes when done.
const IMPORT_PAGE: &str = "about:preferences#general-migrate-autoclose";
const SAVE_FAILED: &str = "プロファイルを保存できません。%LOCALAPPDATA%\\StillBrowser の書き込み権限と空き容量を確認してください。";
/// Where the installer records the installed version (`DisplayVersion`).
const UNINSTALL_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Still";
/// Where the installer registers Still as a browser (`RegisterBrowser`).
const REGISTERED_APPLICATIONS: &str = "Software\\RegisteredApplications";
/// How long the update check waits for another update process (a download
/// times out after 600 s). Still's start never waits.
const LOCK_WAIT: Duration = Duration::from_secs(15 * 60);

fn data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("StillBrowser")
}

fn profile_dir() -> PathBuf {
    data_dir().join("Profile")
}

fn update_dir() -> PathBuf {
    data_dir().join("Update")
}

fn install_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)).unwrap_or_default()
}

fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(Some(0)).collect()
}

fn message(text: &str, style: u32) -> i32 {
    #[cfg(feature = "e2e")]
    if let Some(answer) = e2e_message(text, style) {
        return answer;
    }
    let (text, caption) = (wide(OsStr::new(text)), wide(OsStr::new("Still")));
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), style | MB_SETFOREGROUND) }
}

fn tell(text: &str) {
    message(text, MB_OK | MB_ICONINFORMATION);
}

fn warn(text: &str) {
    message(text, MB_OK | MB_ICONWARNING);
}

/// `e2e` builds only: with `STILL_E2E_LOG` set, a dialog is appended to that
/// file as `<pid>\t<info|warning|question>\t<answer>\t<text, \n escaped>`
/// instead of being shown. Questions take `STILL_E2E_ANSWER` (`yes`, the default, or `no`).
#[cfg(feature = "e2e")]
fn e2e_message(text: &str, style: u32) -> Option<i32> {
    use std::io::Write;
    use windows_sys::Win32::UI::WindowsAndMessaging::{IDNO, IDOK};
    let log = std::env::var_os("STILL_E2E_LOG")?;
    let question = style & 0xF == MB_YESNO;
    let (kind, answer) = match () {
        _ if !question => (if style & 0xF0 == MB_ICONWARNING { "warning" } else { "info" }, IDOK),
        _ if std::env::var("STILL_E2E_ANSWER").is_ok_and(|a| a == "no") => ("question", IDNO),
        _ => ("question", IDYES),
    };
    let answer_name = match answer {
        IDYES => "yes",
        IDNO => "no",
        _ => "ok",
    };
    let line = format!("{}\t{kind}\t{answer_name}\t{}\n", std::process::id(), text.replace('\n', "\\n"));
    let _ = fs::OpenOptions::new().create(true).append(true).open(log).and_then(|mut f| f.write_all(line.as_bytes()));
    Some(answer)
}

/// Firefox holds `parent.lock` open without sharing while the profile is in
/// use (and deletes it on exit), so opening it fails with a sharing violation.
fn profile_running(profile: &Path) -> bool {
    const ERROR_SHARING_VIOLATION: i32 = 32;
    match fs::OpenOptions::new().read(true).share_mode(0).open(profile.join("parent.lock")) {
        Ok(_) => false,
        Err(error) => error.raw_os_error() == Some(ERROR_SHARING_VIOLATION),
    }
}

pub fn main() {
    let mut args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let first = args.first().map(|a| a.to_string_lossy().into_owned()).unwrap_or_default();
    match first.as_str() {
        "--check-update" => return check_update(false),
        "--update" => return check_update(true),
        "--newtab-image" => return choose_newtab_image(args.get(1).map(PathBuf::from)),
        "--remove-newtab-image" => return remove_newtab_image(),
        "--make-default" => return make_default(),
        "--prepare-only" => {
            if let Err(error) = profile::prepare(&profile_dir(), &data_dir()) {
                warn(&format!("{SAVE_FAILED}\n\n{error}"));
            }
            return;
        }
        "--import" => {
            if profile_running(&profile_dir()) {
                tell("Still のアドレス欄に about:preferences#general-migrate と入力すると、インポートの画面が開きます。");
                return;
            }
            args[0] = IMPORT_PAGE.into();
        }
        _ => {}
    }
    // Anything else is a page to open. Firefox options are not passed through.
    if args.iter().any(|a| a.to_string_lossy().starts_with('-')) {
        warn("使い方: still.exe [URL…] | --import | --update | --make-default | --newtab-image [画像] | --remove-newtab-image");
        return;
    }
    launch(&args);
}

fn launch(urls: &[OsString]) {
    let profile = profile_dir();
    let running = profile_running(&profile);
    if !running && apply_staged_update() {
        return;
    }
    // A running Firefox owns prefs.js; the second start only hands it the URLs.
    if !running {
        if let Err(error) = profile::prepare(&profile, &data_dir()) {
            warn(&format!("{SAVE_FAILED}\n\n{error}"));
            return;
        }
    }
    let firefox = install_dir().join("firefox").join("firefox.exe");
    if let Err(error) = Command::new(&firefox).arg("-profile").arg(&profile).args(urls).spawn() {
        warn(&format!("Firefox を起動できません（{}）。Still をインストールし直してください。\n\n{error}", firefox.display()));
        return;
    }
    if !running {
        // Asks only when there is a new version; runs on after this process exits.
        if let Ok(exe) = std::env::current_exe() {
            let _ = Command::new(exe).arg("--check-update").spawn();
        }
    }
}

// ---------- Updates ----------

fn agent() -> ureq::Agent {
    use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(600)))
        .user_agent(format!("Still/{}", update::CURRENT))
        .tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build())
        .build()
        .into()
}

/// The installed version: this program's, or the one the installer recorded if
/// that is newer (this process started before a newer Still was installed).
fn installed_version() -> String {
    match registry_string(UNINSTALL_KEY, "DisplayVersion") {
        Some(recorded) if update::newer(recorded.trim(), update::CURRENT) => recorded.trim().to_owned(),
        _ => update::CURRENT.to_owned(),
    }
}

fn registry_string(key: &str, value: &str) -> Option<String> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
    let (key, value) = (wide(OsStr::new(key)), wide(OsStr::new(value)));
    let mut buffer = vec![0u16; 256];
    let mut size = (buffer.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buffer.as_mut_ptr().cast(), &mut size)
    };
    if status != 0 {
        return None;
    }
    let text = &buffer[..(size as usize / 2).min(buffer.len())];
    let end = text.iter().position(|&c| c == 0).unwrap_or(text.len());
    Some(String::from_utf16_lossy(&text[..end]))
}

/// Asks before downloading. `explicit` (still.exe --update) also reports that
/// Still is up to date, and asks again about a version declined earlier.
///
/// The download, verification and commit run under the update lock (see
/// staging.rs), which this waits for; dialogs are shown without holding it.
fn check_update(explicit: bool) {
    let agent = agent();
    let manifest = agent.get(update::manifest_url()).call().and_then(|mut r| r.body_mut().read_to_string());
    let manifest = match manifest {
        Ok(text) => text,
        Err(error) => {
            if explicit {
                warn(&format!("アップデートを確認できませんでした。インターネット接続を確認してください。\n\n{error}"));
            }
            return;
        }
    };
    let installed = installed_version();
    let Some(release) = update::offered(&manifest, update::target(), &installed) else {
        if explicit {
            tell(&format!("Still {installed} は最新です。"));
        }
        return;
    };
    let dir = update_dir();
    if !explicit && staging::declined(&dir).is_some_and(|v| v == release.version) {
        return;
    }
    // Downloaded already (another check, while an update could not start): no question.
    if staging::staged_for(&dir, &release.version, &installed).is_none() {
        let question = format!(
            "Still {} が利用できます（現在 {installed}）。\n\nダウンロードしますか？ 次に Still を起動したときに更新します。",
            release.version
        );
        if message(&question, MB_YESNO | MB_ICONINFORMATION) != IDYES {
            if let Ok(Some(lock)) = UpdateLock::acquire(&dir, Duration::from_secs(5)) {
                let _ = staging::decline(&dir, &lock, &release.version);
            }
            return;
        }
    }
    let lock = match UpdateLock::acquire(&dir, LOCK_WAIT) {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            if explicit {
                warn("別のアップデートの処理が終わりません。しばらくしてから、もう一度確認してください。");
            }
            return;
        }
        Err(error) => return warn(&format!("アップデートを保存できませんでした。空き容量を確認してください。\n\n{error}")),
    };
    let download = || {
        let response = agent.get(&release.url).call().map_err(std::io::Error::other)?;
        Ok((response.body().content_length(), response.into_body().into_reader()))
    };
    let staged = staging::stage(&dir, &lock, &release, &installed, &update::public_key(), update::SIZE_LIMIT, download);
    match staged {
        Ok(Outcome::Committed | Outcome::AlreadyStaged(_)) => {
            if !profile_running(&profile_dir()) {
                run_staged(lock);
                return;
            }
            drop(lock);
            tell("アップデートの準備ができました。Still を閉じて、もう一度起動すると更新します。");
        }
        Ok(Outcome::NotNewer) => {
            drop(lock);
            if explicit {
                tell(&format!("Still {} は最新です。", installed_version()));
            }
        }
        Err(error) => {
            drop(lock);
            warn(&match error {
                StageError::Download(error) => format!("アップデートをダウンロードできませんでした。\n\n{error}"),
                StageError::TooLarge => "ダウンロードしたアップデートが大きすぎます。インストールしません。".to_owned(),
                StageError::BadSignature => "ダウンロードしたアップデートの署名を確認できませんでした。インストールしません。".to_owned(),
                StageError::Save(error) => format!("アップデートを保存できませんでした。空き容量を確認してください。\n\n{error}"),
            });
        }
    }
}

/// On start: runs a staged installer if there is one and no other process is
/// updating (it never waits; Still then starts as usual and the next start
/// tries again). True when the installer started; it starts Still when done.
fn apply_staged_update() -> bool {
    let dir = update_dir();
    if !dir.exists() {
        return false;
    }
    match UpdateLock::try_acquire(&dir) {
        Ok(Some(lock)) => run_staged(lock),
        _ => false,
    }
}

/// Runs the staged installer, checked again from the file (see
/// `staging::activation`: it is tried once). The lock and the installer file
/// are held until the installer has started.
fn run_staged(lock: UpdateLock) -> bool {
    let dir = update_dir();
    let Some(activation) = staging::activation(&dir, &lock, &installed_version(), &update::public_key(), update::SIZE_LIMIT) else {
        return false;
    };
    // /P shows progress without questions; /UPDATE starts Still afterwards.
    match Command::new(&activation.installer).args(["/P", "/UPDATE"]).spawn() {
        Ok(_) => true,
        Err(error) => {
            drop(activation);
            staging::discard(&dir, &lock);
            drop(lock);
            warn(&format!("アップデートを開始できませんでした。\n\n{error}"));
            false
        }
    }
}

// ---------- Default browser ----------

/// Windows lets only the user choose the default browser: this opens Settings
/// at Still's entry in Default apps (Windows 11), or at Default apps (Windows 10).
fn make_default() {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    if registry_string(REGISTERED_APPLICATIONS, "Still").is_none() {
        return warn("Still が Windows に登録されていません。Still をインストールし直してください。");
    }
    let open = |page: &str| {
        let (verb, page) = (wide(OsStr::new("open")), wide(OsStr::new(page)));
        let result = unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), page.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) };
        result as isize > 32
    };
    if !open("ms-settings:defaultapps?registeredAppUser=Still") && !open("ms-settings:defaultapps") {
        warn("Windows の設定を開けませんでした。設定 › アプリ › 既定のアプリ で Still を選んでください。");
    }
}

// ---------- New tab picture ----------

fn pick_picture() -> Option<PathBuf> {
    let filter: Vec<u16> = "画像 (PNG, JPEG, GIF, WebP, AVIF, BMP)\0*.png;*.jpg;*.jpeg;*.gif;*.webp;*.avif;*.bmp\0\0".encode_utf16().collect();
    let title = wide(OsStr::new("新しいタブの画像を選ぶ"));
    let mut file = vec![0u16; 32_768];
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        lpstrFilter: filter.as_ptr(),
        lpstrFile: file.as_mut_ptr(),
        nMaxFile: file.len() as u32,
        lpstrTitle: title.as_ptr(),
        Flags: OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
        ..Default::default()
    };
    if unsafe { GetOpenFileNameW(&mut dialog) } == 0 {
        return None;
    }
    let length = file.iter().position(|&c| c == 0).unwrap_or(file.len());
    Some(PathBuf::from(OsString::from_wide(&file[..length])))
}

fn choose_newtab_image(source: Option<PathBuf>) {
    let Some(source) = source.or_else(pick_picture) else { return };
    let profile = profile_dir();
    let stored = newtab::read(&source).and_then(|data| newtab::store(&profile, &data));
    match stored {
        Err(newtab::StoreError::Unsupported) => return warn(newtab::UNSUPPORTED),
        Err(newtab::StoreError::Io(error)) => return warn(&format!("画像を保存できません。\n\n{error}")),
        Ok(_) => {}
    }
    if fs::create_dir_all(profile.join("chrome")).and_then(|_| profile::write_user_content(&profile)).is_err() {
        return warn(SAVE_FAILED);
    }
    let restart = if profile_running(&profile) { "Still を再起動すると反映されます。" } else { "" };
    tell(&format!("新しいタブの画像を設定しました。{restart}"));
}

fn remove_newtab_image() {
    let profile = profile_dir();
    newtab::remove_others(&profile, "");
    if fs::create_dir_all(profile.join("chrome")).and_then(|_| profile::write_user_content(&profile)).is_err() {
        return warn(SAVE_FAILED);
    }
    let restart = if profile_running(&profile) { "Still を再起動すると反映されます。" } else { "" };
    tell(&format!("新しいタブの画像を外しました。{restart}"));
}
