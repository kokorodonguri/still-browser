//! Still's own updates. The download page publishes `update/windows.json` in
//! the Tauri updater format, so the earlier Tauri build updates to this one
//! with the same manifest. Installers are signed with the same minisign key.

use base64::Engine;
use minisign_verify::{PublicKey, Signature};
use serde_json::Value;
use std::io::{self, Read};

pub const MANIFEST_URL: &str = "https://still.dongurihub.com/update/windows.json";
/// `plugins.updater.pubkey` in the earlier build's tauri.conf.json.
pub const PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEJDREFENjI0OThBMDkzQkUKUldTK2s2Q1lKTmJhdktSQVdtTTRsQjJtNDRVdjFqemNwRDVVV0ZyZGJtU1ZIN1V1YmJaQXFscHkK";

pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// Installers are about 80 MB; anything far larger is not ours. Applies to the
/// download, the staged file and the signature check.
pub const SIZE_LIMIT: u64 = 400 * 1024 * 1024;

/// The manifest to read. Only builds with the off-by-default `e2e` feature (an
/// end-to-end test harness) take another from `STILL_UPDATE_MANIFEST_URL`.
pub fn manifest_url() -> String {
    #[cfg(feature = "e2e")]
    if let Ok(url) = std::env::var("STILL_UPDATE_MANIFEST_URL") {
        return url;
    }
    MANIFEST_URL.to_owned()
}

/// The key installers must be signed with. Only `e2e` builds take a test key
/// from `STILL_UPDATE_PUBLIC_KEY` (base64 of the minisign public key file).
pub fn public_key() -> String {
    #[cfg(feature = "e2e")]
    if let Ok(key) = std::env::var("STILL_UPDATE_PUBLIC_KEY") {
        return key;
    }
    PUBLIC_KEY.to_owned()
}

/// The manifest key for this build's processor.
pub fn target() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "windows-aarch64"
    } else {
        "windows-x86_64"
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: String,
    pub url: String,
    /// Base64 of the minisign signature file, as `tauri signer sign` writes it.
    pub signature: String,
}

/// The release offered for `target`, if the manifest names one that is newer.
pub fn offered(manifest: &str, target: &str, current: &str) -> Option<Release> {
    let manifest: Value = serde_json::from_str(manifest).ok()?;
    let version = manifest.get("version")?.as_str()?.trim_start_matches('v').to_owned();
    let platform = manifest.get("platforms")?.get(target)?;
    let url = platform.get("url")?.as_str()?.to_owned();
    let signature = platform.get("signature")?.as_str()?.to_owned();
    (download_allowed(&url) && newer(&version, current)).then_some(Release { version, url, signature })
}

/// HTTPS only. `e2e` builds also accept a plain-HTTP server on the loopback address.
fn download_allowed(url: &str) -> bool {
    url.starts_with("https://")
        || (cfg!(feature = "e2e") && ["http://127.0.0.1:", "http://127.0.0.1/", "http://localhost:", "http://localhost/"].iter().any(|p| url.starts_with(p)))
}

/// True when dotted version `a` is above `b` (`0.10.0` > `0.9.1`).
pub fn newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| -> Option<Vec<u64>> { v.split('.').map(|p| p.parse().ok()).collect() };
    match (parts(a), parts(b)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// Checks the bytes read from `data` against a base64 signature (as `tauri
/// signer sign` writes it) made with `public_key`. Errors are read errors only;
/// a malformed key or signature, or anything over `limit` bytes, is Ok(false).
///
/// Tauri signs a BLAKE2b-512 prehash ("ED" signatures), which is checked as the
/// data streams through: memory stays constant. Legacy minisign signatures
/// ("Ed") sign the raw bytes, and minisign-verify 0.2 needs them in one buffer,
/// so those read at most `limit` bytes into memory once.
pub fn verify_reader(mut data: impl Read, signature: &str, public_key: &str, limit: u64) -> io::Result<bool> {
    let decode = |text: &str| {
        let bytes = base64::engine::general_purpose::STANDARD.decode(text.trim()).ok()?;
        String::from_utf8(bytes).ok()
    };
    let (Some(key), Some(signature)) = (decode(public_key), decode(signature)) else { return Ok(false) };
    let (Ok(key), Ok(signature)) = (PublicKey::decode(&key), Signature::decode(&signature)) else { return Ok(false) };
    match key.verify_stream(&signature) {
        Ok(mut stream) => {
            let mut buffer = vec![0; 64 * 1024];
            let mut total = 0u64;
            loop {
                let read = match data.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => read,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(error),
                };
                total += read as u64;
                if total > limit {
                    return Ok(false);
                }
                stream.update(&buffer[..read]);
            }
            Ok(stream.finalize().is_ok())
        }
        Err(minisign_verify::Error::UnsupportedLegacyMode) => {
            let mut bytes = Vec::new();
            data.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
            Ok(bytes.len() as u64 <= limit && key.verify(&bytes, &signature, true).is_ok())
        }
        Err(_) => Ok(false),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn verify_with(data: &[u8], signature: &str, public_key: &str) -> bool {
        verify_reader(data, signature, public_key, SIZE_LIMIT).unwrap_or(false)
    }

    /// Checks `data` against a base64 signature made with Still's update key.
    fn verify(data: &[u8], signature: &str) -> bool {
        verify_with(data, signature, PUBLIC_KEY)
    }

    #[test]
    fn compares_versions_by_number() {
        assert!(newer("0.10.0", "0.9.1"));
        assert!(newer("0.5.0", "0.4.0"));
        assert!(!newer("0.5.0", "0.5.0"));
        assert!(!newer("0.4.9", "0.5.0"));
        assert!(!newer("0.5.0-beta", "0.4.0"));
    }

    #[test]
    fn reads_the_tauri_manifest() {
        let manifest = r#"{"version":"0.6.0","pub_date":"2026-10-14T00:00:00Z","platforms":{
            "windows-x86_64":{"signature":"c2ln","url":"https://still.dongurihub.com/dl/Still_0.6.0_x64-setup.exe"}}}"#;
        let release = offered(manifest, "windows-x86_64", "0.5.0").unwrap();
        assert_eq!(release.version, "0.6.0");
        assert_eq!(release.signature, "c2ln");
        assert_eq!(offered(manifest, "windows-aarch64", "0.5.0"), None);
        assert_eq!(offered(manifest, "windows-x86_64", "0.6.0"), None);
        assert_eq!(offered(&manifest.replace("https://still", "http://still"), "windows-x86_64", "0.5.0"), None);
        assert_eq!(offered("<html>", "windows-x86_64", "0.5.0"), None);
    }

    /// Made with `tauri signer generate` / `tauri signer sign` and a throwaway key.
    pub(crate) const TEST_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEFFQ0VCQjlCQzUyQzE3NDEKUldSQkZ5ekZtN3ZPcnNpTitmOWNEWHUvR2dXcThrazAzOEsySnNrVnk5alozcWxHU04rVmwvQmUK";
    pub(crate) const TEST_SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVSQkZ5ekZtN3ZPcnZJcmhSaDBXWHpWN3VSUitFY1hORjFjbUdwNE5DMGFTQ1ZIcWdlRVpQdlJZUnZoMk16VUdnUWRZV1VhWUdERlJLQ3htMW9wdGlBbFFOUS83cko4dGdBPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkxOTg5MjU3CWZpbGU6ZGF0YS5iaW4KS2s2MjNVNFBzU2pwa0t6WG5UTzdRZWh4WTJ3a1NRUnpROVdIbmd0TGdxQXJKMlppNU5lL1lIcUhZaDA5b1BDaWNrT0F2eEJROUtsZ1dEQVkxM1pzQmc9PQo=";
    /// The bytes TEST_SIGNATURE signs.
    pub(crate) const TEST_DATA: &[u8] = b"Still update test\n";
    /// A legacy (non-prehashed, "Ed") minisign signature of TEST_DATA from another
    /// throwaway key, made with Ed25519 directly; Tauri itself signs prehashed.
    const LEGACY_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IHRocm93YXdheSBsZWdhY3kgdGVzdCBrZXkKUldTY1VjblhUaWRyd2x3cURZclkzamU2TDZWVWFoSWphREhZdkJDVklTVlloZVlpRGVnVHZWTDEK";
    const LEGACY_SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IGxlZ2FjeSBzaWduYXR1cmUKUldTY1VjblhUaWRyd2pHc09oMThBOG1RM2k4NkZmQ2h4ZXBGM25kSGVpemVkQ0kyWFJlNVI4R3ZOSGJ1d0l2bUlaZEtVRFJzZktGSFI2dHY1VEtUWFdkVXd3czBPOUJDalEwPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkxOTg5MjU3CWZpbGU6ZGF0YS5iaW4KMk1VSW5UQTZ2NmNFVndEOEdQNk1kWlJoalhvSDJJMkxzZEV5ZHFLakFVdUV4ZFlzK3Y5WWNnZ1FBZE1wZ2NoaDhOY2JoRUJDZlVxQzBhTVlZVmlIQ3c9PQo=";

    #[test]
    fn accepts_a_tauri_signature_and_nothing_else() {
        assert!(verify_with(b"Still update test\n", TEST_SIGNATURE, TEST_KEY));
        assert!(!verify_with(b"Still update test!\n", TEST_SIGNATURE, TEST_KEY));
        assert!(!verify(b"Still update test\n", TEST_SIGNATURE), "another key's signature");
    }

    /// A reader that hands out a few bytes per call, like a network body.
    struct Trickle<'a>(&'a [u8]);
    impl Read for Trickle<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let count = self.0.len().min(buffer.len()).min(3);
            buffer[..count].copy_from_slice(&self.0[..count]);
            self.0 = &self.0[count..];
            Ok(count)
        }
    }

    #[test]
    fn verifies_prehashed_signatures_as_a_stream() {
        assert!(verify_reader(Trickle(TEST_DATA), TEST_SIGNATURE, TEST_KEY, 100).unwrap());
        assert!(!verify_reader(Trickle(b"Still update test!\n"), TEST_SIGNATURE, TEST_KEY, 100).unwrap());
        assert!(!verify_reader(Trickle(TEST_DATA), TEST_SIGNATURE, TEST_KEY, 10).unwrap(), "over the limit");
        assert!(!verify_reader(Trickle(TEST_DATA), TEST_SIGNATURE, LEGACY_KEY, 100).unwrap(), "another key");
    }

    #[test]
    fn verifies_legacy_signatures_from_one_bounded_buffer() {
        assert!(verify_reader(Trickle(TEST_DATA), LEGACY_SIGNATURE, LEGACY_KEY, 100).unwrap());
        assert!(!verify_reader(Trickle(b"Still update test!\n"), LEGACY_SIGNATURE, LEGACY_KEY, 100).unwrap());
        assert!(!verify_reader(Trickle(TEST_DATA), LEGACY_SIGNATURE, LEGACY_KEY, 10).unwrap(), "over the limit");
        assert!(!verify(TEST_DATA, LEGACY_SIGNATURE), "another key's signature");
    }

    #[test]
    fn reports_read_errors() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("disk"))
            }
        }
        assert!(verify_reader(Broken, TEST_SIGNATURE, TEST_KEY, 100).is_err());
    }

    /// A built installer: STILL_INSTALLER=artifacts/Still_<v>_x64-setup.exe cargo test -- --ignored
    #[test]
    #[ignore]
    fn built_installer_carries_a_valid_signature() {
        let path = std::path::PathBuf::from(std::env::var("STILL_INSTALLER").expect("STILL_INSTALLER"));
        let signature = std::fs::read_to_string(format!("{}.sig", path.display())).unwrap();
        let file = std::fs::File::open(&path).unwrap();
        assert!(verify_reader(file, &signature, PUBLIC_KEY, SIZE_LIMIT).unwrap());
    }

    #[test]
    fn rejects_unsigned_or_altered_files() {
        assert!(!verify(b"installer", ""));
        assert!(!verify(b"installer", "bm90IGEgc2lnbmF0dXJl"));
    }
}
