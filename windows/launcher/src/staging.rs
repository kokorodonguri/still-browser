//! The staged update as a transaction, on any OS (Windows supplies the paths,
//! the HTTP body and the installer launch).
//!
//! `<data>\Update` holds one complete set: `Still-setup.exe`, its signature
//! `Still-setup.exe.sig` and `version`, which marks it as not yet tried. A new
//! set is written and verified in a sibling `Update.tmp-<pid>-<n>`, synced, and
//! committed by renaming the old set aside (`Update.old-<pid>-<n>`; Windows
//! cannot rename over a directory) and the new one into place. So `Update` is
//! absent, the previous complete set or the new one, whenever the process
//! stops. Leftover temporary and old directories are removed under the lock.
//!
//! Every writer holds `Update.lock` next to the directory (never inside it, as
//! the directory is renamed). The OS releases the lock when its process ends,
//! however it ends. Functions that change the directory take the held lock as
//! proof. `declined` (a version the user said no to) also lives in `Update`.

use crate::update::{self, Release};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const INSTALLER: &str = "Still-setup.exe";
pub const SIGNATURE: &str = "Still-setup.exe.sig";
/// The staged version; renamed to `version.tried` just before the installer runs.
pub const VERSION: &str = "version";
pub const TRIED: &str = "version.tried";
pub const DECLINED: &str = "declined";
const TEMPORARY: &str = ".tmp-";
const OLD: &str = ".old-";

/// `Update` with another suffix, in the same parent directory.
fn sibling(dir: &Path, suffix: &str) -> PathBuf {
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    dir.with_file_name(format!("{name}{suffix}"))
}

/// A name for a temporary or old directory that does not exist yet (a leftover
/// that could not be removed, say a file still open in it, keeps its name).
fn unused_sibling(dir: &Path, kind: &str) -> io::Result<PathBuf> {
    let pid = std::process::id();
    (0..1000)
        .map(|n| sibling(dir, &format!("{kind}{pid}-{n}")))
        .find(|path| fs::symlink_metadata(path).is_err())
        .ok_or_else(|| io::Error::new(io::ErrorKind::AlreadyExists, "too many leftover update directories"))
}

// ---------- Lock ----------

/// The cross-process update lock (`Update.lock` beside `Update`), held until dropped.
pub struct UpdateLock {
    _file: File,
}

impl UpdateLock {
    pub fn path(dir: &Path) -> PathBuf {
        sibling(dir, ".lock")
    }

    /// The lock if no other process holds it; never waits (Still's start uses this).
    pub fn try_acquire(dir: &Path) -> io::Result<Option<Self>> {
        let path = Self::path(dir);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        // The file stays: deleting it would let two writers lock different files.
        let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&path)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(UpdateLock { _file: file })),
            Err(fs::TryLockError::WouldBlock) => Ok(None),
            Err(fs::TryLockError::Error(error)) => Err(error),
        }
    }

    /// Waits up to `timeout` for the lock (the background check uses this).
    pub fn acquire(dir: &Path, timeout: Duration) -> io::Result<Option<Self>> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(lock) = Self::try_acquire(dir)? {
                return Ok(Some(lock));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

// ---------- Reading ----------

/// The staged version that has not been tried yet.
pub fn pending(dir: &Path) -> Option<String> {
    let version = fs::read_to_string(dir.join(VERSION)).ok()?;
    let version = version.trim();
    (!version.is_empty()).then(|| version.to_owned())
}

/// The version the user declined, if any.
pub fn declined(dir: &Path) -> Option<String> {
    fs::read_to_string(dir.join(DECLINED)).ok().map(|v| v.trim().to_owned())
}

/// Remembers a declined version, so the background check does not ask again.
pub fn decline(dir: &Path, _lock: &UpdateLock, version: &str) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    fs::write(dir.join(DECLINED), version)
}

#[derive(Debug, PartialEq)]
pub enum Plan {
    /// Download and stage the offered release.
    Download,
    /// This version, the same as or newer than the offered one, is staged already.
    AlreadyStaged(String),
    /// The offered release is not newer than the installed one.
    NotNewer,
}

/// What to do about `offered`, decided again under the lock: a check that
/// waited must not replace a newer staged or installed version.
pub fn plan(dir: &Path, _lock: &UpdateLock, offered: &str, installed: &str) -> Plan {
    if !update::newer(offered, installed) {
        return Plan::NotNewer;
    }
    match staged_for(dir, offered, installed) {
        Some(staged) => Plan::AlreadyStaged(staged),
        None => Plan::Download,
    }
}

/// The untried staged version if it makes downloading `offered` unnecessary:
/// newer than the installed version and not older than `offered`. Without the
/// lock this is a hint only (the check skips its question); `plan` decides.
pub fn staged_for(dir: &Path, offered: &str, installed: &str) -> Option<String> {
    pending(dir).filter(|staged| update::newer(staged, installed) && !update::newer(offered, staged))
}

// ---------- Staging ----------

#[derive(Debug)]
pub enum StageError {
    /// Reading the download failed (the network).
    Download(io::Error),
    /// The download is over the size limit.
    TooLarge,
    /// The download does not carry the release's valid signature.
    BadSignature,
    /// Writing, syncing or committing the files failed (the disk).
    Save(io::Error),
}

impl fmt::Display for StageError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            StageError::Download(error) | StageError::Save(error) => error.fmt(f),
            StageError::TooLarge => write!(f, "larger than {} bytes", update::SIZE_LIMIT),
            StageError::BadSignature => f.write_str("bad signature"),
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Committed,
    AlreadyStaged(String),
    NotNewer,
}

/// Stages `release` unless `plan` says otherwise. `open` starts the download
/// and returns the declared length, if any, and the body. Nothing changes in
/// `dir` unless the complete, verified set is committed.
pub fn stage<R: Read>(
    dir: &Path,
    lock: &UpdateLock,
    release: &Release,
    installed: &str,
    public_key: &str,
    limit: u64,
    open: impl FnOnce() -> io::Result<(Option<u64>, R)>,
) -> Result<Outcome, StageError> {
    remove_leftovers(dir, lock);
    match plan(dir, lock, &release.version, installed) {
        Plan::Download => {}
        Plan::AlreadyStaged(version) => return Ok(Outcome::AlreadyStaged(version)),
        Plan::NotNewer => return Ok(Outcome::NotNewer),
    }
    let temporary = unused_sibling(dir, TEMPORARY).map_err(StageError::Save)?;
    let result = write_set(&temporary, release, public_key, limit, open).and_then(|()| commit(&temporary, dir).map_err(StageError::Save));
    if result.is_err() {
        let _ = fs::remove_dir_all(&temporary);
    }
    result.map(|()| Outcome::Committed)
}

fn write_set<R: Read>(
    temporary: &Path,
    release: &Release,
    public_key: &str,
    limit: u64,
    open: impl FnOnce() -> io::Result<(Option<u64>, R)>,
) -> Result<(), StageError> {
    fs::create_dir_all(temporary).map_err(StageError::Save)?;
    let (length, mut body) = open().map_err(StageError::Download)?;
    if length.is_some_and(|length| length > limit) {
        return Err(StageError::TooLarge);
    }
    let mut file =
        OpenOptions::new().read(true).write(true).create_new(true).open(temporary.join(INSTALLER)).map_err(StageError::Save)?;
    copy_limited(&mut body, &mut file, limit)?;
    file.sync_all().map_err(StageError::Save)?;
    // Checked from the file just written, through the same handle.
    file.rewind().map_err(StageError::Save)?;
    if !update::verify_reader(&mut file, &release.signature, public_key, limit).map_err(StageError::Save)? {
        return Err(StageError::BadSignature);
    }
    drop(file);
    write_synced(&temporary.join(SIGNATURE), release.signature.as_bytes()).map_err(StageError::Save)?;
    write_synced(&temporary.join(VERSION), release.version.as_bytes()).map_err(StageError::Save)
}

/// Copies `body` in chunks, so memory use does not grow with the installer.
fn copy_limited(body: &mut impl Read, file: &mut File, limit: u64) -> Result<(), StageError> {
    let mut buffer = vec![0; 64 * 1024];
    let mut total = 0u64;
    loop {
        let read = match body.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(StageError::Download(error)),
        };
        total += read as u64;
        if total > limit {
            return Err(StageError::TooLarge);
        }
        file.write_all(&buffer[..read]).map_err(StageError::Save)?;
    }
}

fn write_synced(path: &Path, data: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(data)?;
    file.sync_all()
}

/// Puts the complete set in `temporary` in place of `dir`. If the second rename
/// fails, the previous set is renamed back.
fn commit(temporary: &Path, dir: &Path) -> io::Result<()> {
    let old = unused_sibling(dir, OLD)?;
    let had_previous = match fs::rename(dir, &old) {
        Ok(()) => true,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        // On Windows: a file in it is open, such as the installer while it runs.
        Err(error) => return Err(error),
    };
    if let Err(error) = fs::rename(temporary, dir) {
        if had_previous {
            let _ = fs::rename(&old, dir);
        }
        return Err(error);
    }
    sync_parent(dir);
    if had_previous {
        let _ = fs::remove_dir_all(&old);
    }
    Ok(())
}

/// Makes the renames durable where the OS allows syncing a directory (NTFS
/// journals them itself; opening a directory on Windows needs other flags).
fn sync_parent(dir: &Path) {
    #[cfg(unix)]
    if let Some(parent) = dir.parent() {
        let _ = File::open(parent).and_then(|d| d.sync_all());
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// Removes temporary and old directories left by a process that stopped.
/// Only lock holders create them, so any found under the lock are leftovers.
pub fn remove_leftovers(dir: &Path, _lock: &UpdateLock) {
    let (Some(parent), Some(name)) = (dir.parent(), dir.file_name()) else { return };
    let name = name.to_string_lossy();
    let prefixes = [format!("{name}{TEMPORARY}"), format!("{name}{OLD}")];
    let Ok(entries) = fs::read_dir(parent) else { return };
    for entry in entries.flatten() {
        let entry_name = entry.file_name();
        let entry_name = entry_name.to_string_lossy();
        if prefixes.iter().any(|prefix| entry_name.starts_with(prefix.as_str())) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// Drops the staged set: renamed aside first so `Update` never holds part of
/// one. If it cannot be moved (a file in it is open), at least `version` goes,
/// so the set is not tried again.
pub fn discard(dir: &Path, _lock: &UpdateLock) {
    match unused_sibling(dir, OLD).and_then(|old| fs::rename(dir, &old).map(|()| old)) {
        Ok(old) => {
            let _ = fs::remove_dir_all(old);
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => {
            let _ = fs::remove_file(dir.join(VERSION));
            let _ = fs::remove_dir_all(dir);
        }
    }
}

// ---------- Activation ----------

/// A verified installer, ready to run. The file stays open until this is
/// dropped; on Windows without write or delete sharing, so the checked bytes
/// are the ones that start.
#[derive(Debug)]
pub struct Activation {
    pub installer: PathBuf,
    _held: File,
}

/// The staged installer to run now, checked again (files on disk can change),
/// with `version` already renamed to `version.tried`: a staged update is tried
/// once. If Still is still older afterwards, the set is dropped here on a later
/// start and offered again by the next check. Anything not runnable is dropped.
pub fn activation(dir: &Path, lock: &UpdateLock, installed: &str, public_key: &str, limit: u64) -> Option<Activation> {
    remove_leftovers(dir, lock);
    let Some(version) = pending(dir) else {
        if dir.join(TRIED).exists() || dir.join(INSTALLER).exists() {
            discard(dir, lock);
        }
        return None;
    };
    let installer = dir.join(INSTALLER);
    let signature = fs::read_to_string(dir.join(SIGNATURE)).unwrap_or_default();
    let held = update::newer(&version, installed)
        .then(|| open_held(&installer).ok())
        .flatten()
        .filter(|file| file.metadata().is_ok_and(|m| m.len() <= limit))
        .filter(|mut file| update::verify_reader(&mut file, &signature, public_key, limit).unwrap_or(false));
    let Some(held) = held else {
        discard(dir, lock);
        return None;
    };
    if fs::rename(dir.join(VERSION), dir.join(TRIED)).is_err() {
        drop(held);
        discard(dir, lock);
        return None;
    }
    Some(Activation { installer, _held: held })
}

fn open_held(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_SHARE_READ only: the installer can start and read itself, but
        // nothing can change, rename or delete it until this handle closes.
        options.share_mode(0x1);
    }
    options.open(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::tests::{TEST_DATA, TEST_KEY, TEST_SIGNATURE};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn scratch(name: &str) -> PathBuf {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        let n = COUNT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("still-staging-{name}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root.join("Update")
    }

    fn release(version: &str) -> Release {
        Release { version: version.into(), url: "https://example.invalid/setup.exe".into(), signature: TEST_SIGNATURE.into() }
    }

    fn body(data: &'static [u8]) -> impl FnOnce() -> io::Result<(Option<u64>, &'static [u8])> {
        move || Ok((Some(data.len() as u64), data))
    }

    fn stage_test(dir: &Path, lock: &UpdateLock, version: &str, data: &'static [u8]) -> Result<Outcome, StageError> {
        stage(dir, lock, &release(version), "0.5.0", TEST_KEY, 1000, body(data))
    }

    /// The entries beside `Update`, apart from the lock file.
    fn siblings(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n != "Update.lock")
            .collect();
        names.sort();
        names
    }

    fn assert_complete_set(dir: &Path, version: &str) {
        assert_eq!(fs::read(dir.join(INSTALLER)).unwrap(), TEST_DATA);
        assert_eq!(fs::read_to_string(dir.join(SIGNATURE)).unwrap(), TEST_SIGNATURE);
        assert_eq!(fs::read_to_string(dir.join(VERSION)).unwrap(), version);
    }

    #[test]
    fn commits_a_complete_verified_set() {
        let dir = scratch("commit");
        let lock = UpdateLock::try_acquire(&dir).unwrap().unwrap();
        assert_eq!(stage_test(&dir, &lock, "0.6.0", TEST_DATA).unwrap(), Outcome::Committed);
        assert_complete_set(&dir, "0.6.0");
        assert_eq!(siblings(&dir), ["Update"], "no temporary or old directory is left");
        assert_eq!(pending(&dir).as_deref(), Some("0.6.0"));

        // A newer release replaces the set as a whole; `declined` goes with the old set.
        decline(&dir, &lock, "0.6.5").unwrap();
        assert_eq!(declined(&dir).as_deref(), Some("0.6.5"));
        assert_eq!(stage_test(&dir, &lock, "0.7.0", TEST_DATA).unwrap(), Outcome::Committed);
        assert_complete_set(&dir, "0.7.0");
        assert_eq!(declined(&dir), None);
        assert_eq!(siblings(&dir), ["Update"]);
    }

    #[test]
    fn a_failed_download_keeps_the_previous_set() {
        struct Interrupted(usize);
        impl Read for Interrupted {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                if self.0 == 0 {
                    return Err(io::Error::new(io::ErrorKind::ConnectionReset, "reset"));
                }
                self.0 -= 1;
                buffer[0] = b'x';
                Ok(1)
            }
        }
        let dir = scratch("interrupted");
        let lock = UpdateLock::try_acquire(&dir).unwrap().unwrap();
        stage_test(&dir, &lock, "0.6.0", TEST_DATA).unwrap();
        let result = stage(&dir, &lock, &release("0.7.0"), "0.5.0", TEST_KEY, 1000, || Ok((None, Interrupted(5))));
        assert!(matches!(result, Err(StageError::Download(_))), "{result:?}");
        let result = stage(&dir, &lock, &release("0.7.0"), "0.5.0", TEST_KEY, 1000, || -> io::Result<(Option<u64>, &[u8])> {
            Err(io::Error::other("no connection"))
        });
        assert!(matches!(result, Err(StageError::Download(_))), "{result:?}");
        assert_complete_set(&dir, "0.6.0");
        assert_eq!(siblings(&dir), ["Update"]);
    }

    #[test]
    fn a_stopped_process_leaves_only_leftovers_that_are_removed() {
        let dir = scratch("killed");
        let lock = UpdateLock::try_acquire(&dir).unwrap().unwrap();
        stage_test(&dir, &lock, "0.6.0", TEST_DATA).unwrap();
        // A process stopped while downloading, and another between the two renames.
        let partial = sibling(&dir, ".tmp-999-0");
        fs::create_dir_all(&partial).unwrap();
        fs::write(partial.join(INSTALLER), b"Still upd").unwrap();
        let old = sibling(&dir, ".old-998-0");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join(VERSION), "0.5.5").unwrap();
        assert_complete_set(&dir, "0.6.0");

        remove_leftovers(&dir, &lock);
        assert_eq!(siblings(&dir), ["Update"]);
        assert_complete_set(&dir, "0.6.0");

        // With `Update` absent (stopped between the renames) nothing is pending.
        fs::rename(&dir, &old).unwrap();
        assert_eq!(pending(&dir), None);
        assert!(activation(&dir, &lock, "0.5.0", TEST_KEY, 1000).is_none());
        assert_eq!(siblings(&dir), Vec::<String>::new(), "activation removes leftovers too");
    }

    #[test]
    fn rejects_oversized_downloads() {
        let dir = scratch("oversized");
        let lock = UpdateLock::try_acquire(&dir).unwrap().unwrap();
        stage_test(&dir, &lock, "0.6.0", TEST_DATA).unwrap();
        let limit = TEST_DATA.len() as u64 - 1;
        // Declared too large: refused before reading.
        let result = stage(&dir, &lock, &release("0.7.0"), "0.5.0", TEST_KEY, limit, body(TEST_DATA));
        assert!(matches!(result, Err(StageError::TooLarge)), "{result:?}");
        // No length declared: stopped once the limit is passed.
        let result = stage(&dir, &lock, &release("0.7.0"), "0.5.0", TEST_KEY, limit, || Ok((None, TEST_DATA)));
        assert!(matches!(result, Err(StageError::TooLarge)), "{result:?}");
        assert_complete_set(&dir, "0.6.0");
        assert_eq!(siblings(&dir), ["Update"]);
    }

    #[test]
    fn a_bad_signature_is_not_committed() {
        let dir = scratch("signature");
        let lock = UpdateLock::try_acquire(&dir).unwrap().unwrap();
        let result = stage_test(&dir, &lock, "0.6.0", b"Still update test!\n");
        assert!(matches!(result, Err(StageError::BadSignature)), "{result:?}");
        assert!(!dir.exists());
        stage_test(&dir, &lock, "0.6.0", TEST_DATA).unwrap();
        let result = stage(&dir, &lock, &release("0.7.0"), "0.5.0", crate::update::PUBLIC_KEY, 1000, body(TEST_DATA));
        assert!(matches!(result, Err(StageError::BadSignature)), "another key: {result:?}");
        assert_complete_set(&dir, "0.6.0");
        assert_eq!(siblings(&dir), ["Update"]);
    }

    #[test]
    fn never_replaces_a_newer_staged_or_installed_version() {
        let dir = scratch("older");
        let lock = UpdateLock::try_acquire(&dir).unwrap().unwrap();
        stage_test(&dir, &lock, "0.7.0", TEST_DATA).unwrap();
        let unused = || -> io::Result<(Option<u64>, &[u8])> { panic!("downloaded") };
        for older in ["0.6.0", "0.7.0"] {
            let result = stage(&dir, &lock, &release(older), "0.5.0", TEST_KEY, 1000, unused).unwrap();
            assert_eq!(result, Outcome::AlreadyStaged("0.7.0".into()));
        }
        assert_complete_set(&dir, "0.7.0");
        assert_eq!(staged_for(&dir, "0.6.0", "0.5.0").as_deref(), Some("0.7.0"));
        assert_eq!(staged_for(&dir, "0.8.0", "0.5.0"), None);
        // Installed meanwhile (a newer launcher ran the installer).
        assert_eq!(stage(&dir, &lock, &release("0.8.0"), "0.8.0", TEST_KEY, 1000, unused).unwrap(), Outcome::NotNewer);
        assert_eq!(plan(&dir, &lock, "0.8.0", "0.7.0"), Plan::Download);
        // A staged version that is installed already does not block a newer one.
        assert_eq!(plan(&dir, &lock, "0.8.0", "0.7.5"), Plan::Download);
        assert_eq!(plan(&dir, &lock, "0.7.0", "0.7.5"), Plan::NotNewer);
        // A tried set is not pending.
        fs::rename(dir.join(VERSION), dir.join(TRIED)).unwrap();
        assert_eq!(plan(&dir, &lock, "0.7.0", "0.5.0"), Plan::Download);
    }

    #[test]
    fn a_staged_update_is_tried_once() {
        let dir = scratch("tried");
        let lock = UpdateLock::try_acquire(&dir).unwrap().unwrap();
        stage_test(&dir, &lock, "0.6.0", TEST_DATA).unwrap();
        let run = activation(&dir, &lock, "0.5.0", TEST_KEY, 1000).expect("runnable");
        assert_eq!(run.installer, dir.join(INSTALLER));
        assert_eq!(fs::read_to_string(dir.join(TRIED)).unwrap(), "0.6.0");
        assert!(!dir.join(VERSION).exists() && dir.join(TRIED).exists());
        drop(run);
        // Still was not updated (or was): either way the set is dropped.
        assert!(activation(&dir, &lock, "0.5.0", TEST_KEY, 1000).is_none());
        assert!(!dir.exists());
        assert_eq!(siblings(&dir), Vec::<String>::new());
        assert!(activation(&dir, &lock, "0.5.0", TEST_KEY, 1000).is_none());
    }

    #[test]
    fn activation_checks_the_files_again() {
        let dir = scratch("activation");
        let lock = UpdateLock::try_acquire(&dir).unwrap().unwrap();
        // Altered after staging.
        stage_test(&dir, &lock, "0.6.0", TEST_DATA).unwrap();
        fs::write(dir.join(INSTALLER), b"Still update test!\n").unwrap();
        assert!(activation(&dir, &lock, "0.5.0", TEST_KEY, 1000).is_none());
        assert!(!dir.exists());
        // Not newer than the installed version.
        stage_test(&dir, &lock, "0.6.0", TEST_DATA).unwrap();
        assert!(activation(&dir, &lock, "0.6.0", TEST_KEY, 1000).is_none());
        assert!(!dir.exists());
        // Grown over the limit, or missing its signature.
        stage_test(&dir, &lock, "0.6.0", TEST_DATA).unwrap();
        assert!(activation(&dir, &lock, "0.5.0", TEST_KEY, 5).is_none());
        stage_test(&dir, &lock, "0.6.0", TEST_DATA).unwrap();
        fs::remove_file(dir.join(SIGNATURE)).unwrap();
        assert!(activation(&dir, &lock, "0.5.0", TEST_KEY, 1000).is_none());
        // Only `declined`: kept.
        decline(&dir, &lock, "0.6.0").unwrap();
        assert!(activation(&dir, &lock, "0.5.0", TEST_KEY, 1000).is_none());
        assert_eq!(declined(&dir).as_deref(), Some("0.6.0"));
    }

    #[test]
    fn the_lock_admits_one_holder_at_a_time() {
        let dir = scratch("lock");
        let first = UpdateLock::try_acquire(&dir).unwrap().expect("free");
        assert!(UpdateLock::try_acquire(&dir).unwrap().is_none(), "held");
        assert!(UpdateLock::acquire(&dir, Duration::from_millis(150)).unwrap().is_none(), "times out");
        let waiter = {
            let dir = dir.clone();
            std::thread::spawn(move || UpdateLock::acquire(&dir, Duration::from_secs(10)).unwrap().is_some())
        };
        std::thread::sleep(Duration::from_millis(200));
        drop(first);
        assert!(waiter.join().unwrap(), "acquired once released");
        assert!(UpdateLock::path(&dir).exists() && !dir.exists(), "the lock lives beside Update");
    }

    /// Another process holds the lock until it is killed; the OS then releases it.
    #[test]
    fn a_killed_holder_releases_the_lock() {
        if let Some(dir) = std::env::var_os("STILL_STAGING_LOCK_HOLDER") {
            let dir = PathBuf::from(dir);
            let _lock = UpdateLock::try_acquire(&dir).unwrap().expect("free in the child");
            fs::write(dir.with_file_name("held"), "").unwrap();
            std::thread::sleep(Duration::from_secs(60));
            return;
        }
        let dir = scratch("killed-holder");
        /// Kills the child even when an assertion fails.
        struct Holder(std::process::Child);
        impl Drop for Holder {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["staging::tests::a_killed_holder_releases_the_lock", "--exact", "--test-threads=1"])
            .env("STILL_STAGING_LOCK_HOLDER", &dir)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let holder = Holder(child);
        let deadline = Instant::now() + Duration::from_secs(30);
        while !dir.with_file_name("held").exists() {
            assert!(Instant::now() < deadline, "the child did not take the lock");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(UpdateLock::try_acquire(&dir).unwrap().is_none(), "held by the other process");
        drop(holder);
        assert!(UpdateLock::acquire(&dir, Duration::from_secs(10)).unwrap().is_some(), "released when the holder died");
    }
}
