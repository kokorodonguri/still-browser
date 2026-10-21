//! Serialized by the shell's writer mutex, including the snapshot it writes.
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub struct StateWriter {
    path: PathBuf,
    /// Must succeed before the first replacement. Failure keeps this pending.
    backup: Option<PathBuf>,
}

impl StateWriter {
    pub fn new(path: PathBuf, backup: Option<PathBuf>) -> Self {
        Self { path, backup }
    }

    pub fn save(&mut self, contents: &str) -> io::Result<()> {
        self.save_with(|out| out.write_all(contents.as_bytes()))
    }

    /// Like `save`, but `write` streams the contents into the temporary file,
    /// so the whole document never has to exist in memory as one string.
    pub fn save_with(&mut self, write: impl FnOnce(&mut dyn Write) -> io::Result<()>) -> io::Result<()> {
        if let Some(base) = &self.backup {
            let mut source = File::open(&self.path)?;
            let (target, mut backup) = create_unique(base)?;
            let copied = io::copy(&mut source, &mut backup).and_then(|_| backup.sync_all());
            drop(backup);
            if let Err(error) = copied {
                let _ = fs::remove_file(target);
                return Err(error);
            }
            self.backup = None;
        }
        save_atomic_with(&self.path, write)
    }
}

/// Never truncates an existing backup or another writer's temporary file.
fn create_unique(base: &Path) -> io::Result<(PathBuf, File)> {
    let mut path = base.to_path_buf();
    loop {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let mut name = base.as_os_str().to_owned();
                name.push(format!(".{}-{}", std::process::id(), SEQUENCE.fetch_add(1, Ordering::Relaxed)));
                path = PathBuf::from(name);
            }
            Err(error) => return Err(error),
        }
    }
}

/// Sync the completed temporary file before atomically replacing the state.
/// Callers serialize writes to one path to preserve snapshot order.
pub fn save_atomic(path: &Path, contents: &str) -> io::Result<()> {
    save_atomic_with(path, |out| out.write_all(contents.as_bytes()))
}

/// `save_atomic` with streamed contents.
pub fn save_atomic_with(path: &Path, write: impl FnOnce(&mut dyn Write) -> io::Result<()>) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let (temporary, file) = create_unique(&path.with_extension("json.tmp"))?;
    let mut out = BufWriter::with_capacity(32 * 1024, file);
    // The file is closed on every path before it is renamed or removed:
    // Windows refuses both while a handle is open.
    let result = match write(&mut out) {
        Ok(()) => out.into_inner().map_err(io::IntoInnerError::into_error).and_then(|file| file.sync_all()),
        Err(error) => {
            drop(out);
            Err(error)
        }
    };
    let result = result.and_then(|_| fs::rename(&temporary, path));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("still-save-{}-{}", std::process::id(), SEQUENCE.fetch_add(1, Ordering::Relaxed)));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }

    #[test]
    fn failed_backup_blocks_replacement_and_retries() {
        let dir = Directory::new();
        let path = dir.0.join("state.json");
        let blocked = dir.0.join("blocked");
        fs::write(&path, "original newer-version data").unwrap();
        fs::write(&blocked, "not a directory").unwrap();
        let backup = blocked.join("backup");
        let mut writer = StateWriter::new(path.clone(), Some(backup.clone()));
        assert!(writer.save("replacement").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "original newer-version data");
        fs::remove_file(&blocked).unwrap();
        fs::create_dir(&blocked).unwrap();
        writer.save("replacement").unwrap();
        writer.save("latest").unwrap();
        assert_eq!(fs::read_to_string(backup).unwrap(), "original newer-version data");
        assert_eq!(fs::read_to_string(path).unwrap(), "latest");
    }

    #[test]
    fn existing_backup_does_not_skip_protecting_the_current_original() {
        let dir = Directory::new();
        let path = dir.0.join("state.json");
        let backup = dir.0.join("backup");
        fs::write(&path, "current original").unwrap();
        fs::write(&backup, "older original").unwrap();
        StateWriter::new(path.clone(), Some(backup.clone())).save("new state").unwrap();
        assert_eq!(fs::read_to_string(backup).unwrap(), "older original");
        assert!(fs::read_dir(&dir.0).unwrap().any(|p| fs::read_to_string(p.unwrap().path()).unwrap() == "current original"));
        assert_eq!(fs::read_to_string(path).unwrap(), "new state");
    }

    #[test]
    fn atomic_replacement_preserves_unrelated_temporary_files() {
        let dir = Directory::new();
        let path = dir.0.join("state.json");
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, "another write").unwrap();
        for text in ["first", "second"] {
            save_atomic(&path, text).unwrap();
            assert_eq!(fs::read_to_string(&path).unwrap(), text);
        }
        assert_eq!(fs::read_to_string(temporary).unwrap(), "another write");
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
    }

    #[test]
    fn streamed_writes_are_atomic_and_failures_keep_the_old_file() {
        let dir = Directory::new();
        let path = dir.0.join("state.json");
        let mut writer = StateWriter::new(path.clone(), None);
        writer.save_with(|out| serde_json::to_writer(out, &serde_json::json!({"Tabs": []})).map_err(io::Error::from)).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), r#"{"Tabs":[]}"#);
        let failed = writer.save_with(|out| {
            out.write_all(b"{\"partial")?;
            Err(io::Error::other("encoder failed"))
        });
        assert!(failed.is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), r#"{"Tabs":[]}"#, "a failed write never replaces the file");
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1, "the temporary file is removed");
    }

    #[test]
    fn failed_replace_cleans_up_its_temporary_file() {
        let dir = Directory::new();
        let path = dir.0.join("state.json");
        fs::create_dir(&path).unwrap();
        assert!(save_atomic(&path, "new").is_err());
        assert!(path.is_dir());
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    }
}
