//! A small file log at `<data>/logs/still.log`, rotated at 1 MB, plus a panic
//! hook. Release builds abort on panic, so the hook is the only trace we get.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const ROTATE_AT: u64 = 1024 * 1024;

struct Log {
    path: PathBuf,
    file: Option<File>,
}

static LOG: OnceLock<Mutex<Log>> = OnceLock::new();

pub fn init(dir: PathBuf) {
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("still.log");
    let file = OpenOptions::new().create(true).append(true).open(&path).ok();
    let _ = LOG.set(Mutex::new(Log { path, file }));
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        write("PANIC", &info.to_string());
        previous(info);
    }));
}

pub fn write(level: &str, message: &str) {
    #[cfg(debug_assertions)]
    eprintln!("[{level}] {message}");
    let Some(log) = LOG.get() else { return };
    // Never panic while logging (the panic hook calls this).
    let Ok(mut log) = log.lock() else { return };
    if fs::metadata(&log.path).map(|m| m.len() > ROTATE_AT).unwrap_or(false) {
        log.file = None;
        let _ = fs::rename(&log.path, log.path.with_file_name("still.1.log"));
        log.file = OpenOptions::new().create(true).append(true).open(&log.path).ok();
    }
    if let Some(file) = log.file.as_mut() {
        let _ = writeln!(file, "{} {level} {message}", still_core::now_iso8601());
    }
}

macro_rules! info {
    ($($arg:tt)*) => { $crate::logger::write("INFO", &format!($($arg)*)) };
}
macro_rules! warning {
    ($($arg:tt)*) => { $crate::logger::write("WARN", &format!($($arg)*)) };
}
macro_rules! error {
    ($($arg:tt)*) => { $crate::logger::write("ERROR", &format!($($arg)*)) };
}
pub(crate) use {error, info, warning};
