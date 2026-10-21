# Windows hardening: progress

Goal: close the open Windows items in `ARCHITECTURE.md` to production grade,
verified end to end (Wine container on Linux, plus portable tests and CI).

| Track | Scope | Owner files | Status |
| --- | --- | --- | --- |
| A. Update transaction | Named mutex around staging/activation; stage into a temporary directory and commit a complete installer/signature/version set atomically; stream the installer to disk (no Vec) with a size limit; verify the signature from the file. | `windows/launcher/**` | done (portable tests; awaiting Track C's Wine E2E for the Windows-only paths) |
| B. Publication atomicity | Snapshot each Windows `.sig` with its installer; immutable, versioned download URLs; atomic manifest replacement so a manifest never names bytes that are not published. | `site/build.py`, `site/tests/**`, `windows/package.py`, `windows/release_set.py`, `windows/tests/test_package.py`, `windows/tests/signing_fixture.py` | done |
| C. Verification and CI | Wine-based E2E harness (silent install, launch, `--prepare-only`, staged-update activation); `cargo xwin` check/clippy for x64 and arm64 in CI; installer build smoke test. | `windows/e2e/**`, `.github/workflows/**` | in progress |

## Log

- 2026-10-16: baseline green — site 6, Windows Python 13, Node 10, Rust 12 (+1 ignored); `cargo xwin check` x64 clean with `~/.local/llvm18/bin` on PATH.
- 2026-10-16 (A): `staging.rs` holds the OS-neutral transaction: `Update.lock` file lock beside `Update` (launch try-locks and skips; the checker waits up to 15 min and re-plans), stream to `Update.tmp-<pid>-<n>` with the 400 MB limit, verify from the file (`verify_stream`, Tauri signatures are prehashed; legacy falls back to one bounded copy), sync, commit by rename-aside + rename, leftover cleanup, tried-once activation through a write/delete-denying handle. Installed version = max(still.exe, registry `DisplayVersion`). Off-by-default `e2e` feature: `STILL_UPDATE_MANIFEST_URL`, `STILL_UPDATE_PUBLIC_KEY`, `STILL_E2E_LOG`, `STILL_E2E_ANSWER`, loopback http. Rust 25 passed (+1 ignored); clippy `-D warnings` clean on Linux, x64 and arm64 (with and without `e2e`).
- 2026-10-16 (B): done. `windows/package.py` builds and signs in a hidden temp dir inside
  `artifacts/`, then `release_set.commit` renames installer, `.sig` (old one removed
  first), and last the record `<installer>.release.json` (size, SHA-256, signature).
  `site/build.py` snapshots, hashes, and accepts only a record matching the snapshot
  (retrying a racing rebuild), verifies the signature (Ed25519 with the shipped
  key when `cryptography` is installed; key ID and file name otherwise), and publishes
  `dl/<version>/<sha256[:16]>/<name>` (+ `.sig`) before replacing manifests. `dl/<name>`
  is a hard-linked convenience link. Retention: the new generation, the previous
  distinct one, and everything the replaced manifests named. Builds hold a `flock`.
  Existing installers without a record: `python3 windows/package.py --seal` once.
  Tests: site 17, Windows Python 21, Linux 52, Node 10. A served-site E2E (curl,
  three generations, real Linux updater) passed 94 checks.
