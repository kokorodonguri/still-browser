//! still.exe: Still for Windows is Mozilla's Firefox ESR with Still's profile.
//! This launcher prepares that profile and starts the bundled Firefox.
#![cfg_attr(windows, windows_subsystem = "windows")]
// Most of the launcher runs only on Windows; Linux builds compile it for the tests.
#![cfg_attr(not(windows), allow(dead_code))]

mod migrate;
mod newtab;
mod profile;
mod staging;
mod update;
#[cfg(windows)]
mod windows;

fn main() {
    #[cfg(windows)]
    windows::main();
    #[cfg(not(windows))]
    {
        // Checking the Windows profile with Linux Firefox: still --prepare-profile DIR
        let args: Vec<String> = std::env::args().skip(1).collect();
        if let [flag, dir] = args.as_slice() {
            if flag == "--prepare-profile" {
                let dir = std::path::Path::new(dir);
                if let Err(error) = profile::prepare(dir, &dir.join("no-earlier-build")) {
                    eprintln!("{error}");
                    std::process::exit(1);
                }
                return;
            }
        }
        eprintln!("still.exe runs on Windows. On Linux, start linux/still (or --prepare-profile DIR to check the Windows profile).");
    }
}
