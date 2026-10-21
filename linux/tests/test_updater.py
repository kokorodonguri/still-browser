#!/usr/bin/env python3
"""Guard the Linux self-update: check, verified download, and the swap at launch."""
import hashlib
import io
import json
from pathlib import Path
import shutil
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch
from urllib.error import URLError

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "linux"))

import updater


def make_install(folder: Path, version: str | None, firefox: str = "140.17.0", marker: str = "old"):
    (folder / "runtime" / "firefox" / "distribution").mkdir(parents=True)
    (folder / "runtime" / "firefox" / "application.ini").write_text(f"[App]\nVersion={firefox}\n")
    (folder / "still").write_text(marker)
    (folder / "policies.json").write_text(json.dumps({"marker": marker}))
    (folder / "runtime" / "firefox" / "firefox").write_text(marker)
    (folder / "runtime" / "firefox" / "distribution" / "policies.json").write_text(json.dumps({"marker": marker}))
    if version:
        (folder / "release.json").write_text(json.dumps({"version": version}))


def make_archive(version: str, firefox: str = "140.17.0") -> bytes:
    with tempfile.TemporaryDirectory() as directory:
        folder = Path(directory) / "StillBrowser-linux-x64"
        make_install(folder, version, firefox, marker="new")
        buffer = io.BytesIO()
        with tarfile.open(fileobj=buffer, mode="w:gz") as tar:
            tar.add(folder, arcname=folder.name)
        return buffer.getvalue()


def manifest(version: str, archive: bytes, **overrides) -> bytes:
    entry = {"version": version, "url": "dl/still.tar.gz", "sha256": hashlib.sha256(archive).hexdigest(), "size": len(archive)}
    return json.dumps({"linux": {**entry, **overrides}}).encode()


class UpdaterChecks(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name) / "StillBrowser-linux-x64"
        self.lock_name = "." + self.root.name + ".update.lock"
        self.patches = [
            patch.object(updater, "ROOT", self.root),
            patch.object(updater, "STAGED", self.root.with_name(self.root.name + ".update")),
            patch.object(updater, "PREVIOUS", self.root.with_name(self.root.name + ".previous")),
            patch.object(updater, "firefox_running", lambda folder: False),
            patch.object(updater, "UPDATE_URL", "https://updates.example/update.json"),
            patch.object(updater, "urlopen", side_effect=AssertionError("unexpected network request")),
        ]
        for p in self.patches:
            p.start()

    def tearDown(self):
        for p in self.patches:
            p.stop()
        self.directory.cleanup()

    def serve(self, version: str, archive: bytes, **overrides):
        # Exercise the real manifest/stream/checksum/extraction path without a
        # socket. Each response is a fresh binary stream, just like urlopen().
        responses = {
            updater.UPDATE_URL: manifest(version, archive, **overrides),
            "https://updates.example/dl/still.tar.gz": archive,
        }

        def respond(url, *, timeout):
            self.assertIn(timeout, (10, 30))
            return io.BytesIO(responses[url])

        return patch.object(updater, "urlopen", side_effect=respond)

    def test_versions_compare_numerically(self):
        self.assertGreater(updater.version_key("0.10.0"), updater.version_key("0.9.9"))
        self.assertEqual(updater.version_key("0.2"), updater.version_key("0.2.0"))
        self.assertGreater(updater.version_key("141.0esr"), updater.version_key("140.17.0esr"))

    def test_source_checkout_never_updates(self):
        make_install(self.root, None)
        with self.assertRaises(updater.UpdateError):
            updater.check()
        self.assertFalse(updater.apply_staged())

    def test_same_or_older_version_is_not_offered(self):
        make_install(self.root, "0.3.0")
        archive = make_archive("0.3.0")
        with self.serve("0.3.0", archive):
            self.assertIsNone(updater.check())
        with self.serve("0.2.9", archive):
            self.assertIsNone(updater.check())

    def test_newer_version_is_downloaded_and_swapped_in_keeping_firefox(self):
        make_install(self.root, "0.2.0", firefox="140.18.0")
        archive = make_archive("0.3.0", firefox="140.17.0")
        with self.serve("0.3.0", archive):
            entry = updater.check()
            self.assertEqual(entry["version"], "0.3.0")
            updater.download(entry)
            self.assertTrue(updater.check()["staged"], "a staged update is not downloaded again")
        self.assertEqual(updater.release_version(self.root), "0.2.0", "nothing changes before the next launch")
        self.assertTrue(updater.apply_staged())
        self.assertEqual(updater.release_version(self.root), "0.3.0")
        self.assertEqual((self.root / "still").read_text(), "new")
        self.assertEqual(updater.firefox_version(self.root), "140.18.0", "the self-updated Firefox is newer and stays")
        self.assertEqual(json.loads((self.root / "runtime/firefox/distribution/policies.json").read_text()), {"marker": "new"})
        self.assertEqual(sorted(p.name for p in self.root.parent.iterdir()), [self.lock_name, self.root.name], "only installation and persistent lock remain")

    def test_newer_firefox_in_the_archive_replaces_the_installed_one(self):
        make_install(self.root, "0.2.0", firefox="128.0")
        archive = make_archive("0.3.0", firefox="140.17.0")
        with self.serve("0.3.0", archive):
            updater.download(updater.check())
        self.assertTrue(updater.apply_staged())
        self.assertEqual(updater.firefox_version(self.root), "140.17.0")

    def test_corrupt_download_is_rejected_and_nothing_is_staged(self):
        make_install(self.root, "0.2.0")
        archive = make_archive("0.3.0")
        with self.serve("0.3.0", archive, sha256="0" * 64):
            with self.assertRaises(updater.UpdateError):
                updater.download(updater.check())
        with self.serve("0.3.0", archive, size=len(archive) - 1):
            with self.assertRaises(updater.UpdateError):
                updater.download(updater.check())
        self.assertFalse(updater.STAGED.exists())
        self.assertFalse(updater.apply_staged())
        self.assertEqual(sorted(p.name for p in self.root.parent.iterdir()), [self.lock_name, self.root.name], "temporary downloads are removed")

    def test_archive_with_a_different_version_than_the_manifest_is_rejected(self):
        make_install(self.root, "0.2.0")
        archive = make_archive("0.2.5")
        with self.serve("0.3.0", archive):
            with self.assertRaises(updater.UpdateError):
                updater.download(updater.check())

    def test_running_firefox_postpones_the_swap(self):
        make_install(self.root, "0.2.0")
        archive = make_archive("0.3.0")
        with self.serve("0.3.0", archive):
            updater.download(updater.check())
        with patch.object(updater, "firefox_running", lambda folder: True):
            self.assertFalse(updater.apply_staged())
        self.assertEqual(updater.release_version(self.root), "0.2.0")
        self.assertTrue(updater.apply_staged())

    def test_interrupted_swap_is_recovered(self):
        make_install(self.root, "0.2.0")
        self.root.rename(updater.PREVIOUS)
        self.assertFalse(updater.apply_staged())
        self.assertEqual(updater.release_version(self.root), "0.2.0")

    def test_unreachable_server_reports_an_error(self):
        make_install(self.root, "0.2.0")
        with patch.object(updater, "urlopen", side_effect=URLError("unreachable")):
            with self.assertRaises(updater.UpdateError):
                updater.check()

    def assert_original_intact(self):
        self.assertEqual(updater.release_version(self.root), "0.2.0")
        self.assertEqual(updater.firefox_version(self.root), "140.18.0")
        self.assertEqual((self.root / "still").read_text(), "old")
        self.assertEqual((self.root / "runtime/firefox/firefox").read_text(), "old")
        distribution = self.root / "runtime/firefox/distribution"
        self.assertEqual(json.loads((distribution / "policies.json").read_text()), {"marker": "old"})
        self.assertEqual([p.name for p in distribution.iterdir()], ["policies.json"])
        self.assertFalse(updater.PREVIOUS.exists())

    def stage_update(self, firefox="140.17.0"):
        make_install(self.root, "0.2.0", firefox="140.18.0")
        make_install(updater.STAGED, "0.3.0", firefox=firefox, marker="new")

    def test_failed_directory_renames_roll_back_and_allow_retry(self):
        # Both the retained runtime path and the replacement runtime path must
        # leave a usable old installation after any rename fails.
        for firefox in ("140.17.0", "141.0"):
            for boundary in ("backup", "runtime", "commit"):
                if firefox == "141.0" and boundary == "runtime":
                    continue
                with self.subTest(firefox=firefox, boundary=boundary):
                    self.stage_update(firefox)
                    rename = Path.rename
                    sources = {"backup": self.root, "runtime": updater.PREVIOUS / "runtime", "commit": updater.STAGED}

                    def fail_once(path, target):
                        if path == sources[boundary]:
                            raise OSError("injected rename failure")
                        return rename(path, target)

                    with patch.object(Path, "rename", fail_once):
                        with self.assertRaises(OSError):
                            updater.apply_staged()
                    self.assert_original_intact()
                    self.assertTrue(updater.apply_staged())
                    self.assertEqual(updater.release_version(self.root), "0.3.0")
                    expected = "140.18.0" if firefox == "140.17.0" else "141.0"
                    self.assertEqual(updater.firefox_version(self.root), expected)
                    shutil.rmtree(self.root)

    def test_failed_policy_replace_rolls_back_and_removes_temporary_file(self):
        self.stage_update()
        replace = Path.replace

        def fail_policy(path, target):
            if target == updater.STAGED / "runtime/firefox/distribution/policies.json":
                raise OSError("injected policy failure")
            return replace(path, target)

        with patch.object(Path, "replace", fail_policy):
            with self.assertRaises(OSError):
                updater.apply_staged()
        self.assert_original_intact()
        self.assertTrue(updater.apply_staged())

    def test_keyboard_interrupt_during_commit_rolls_back(self):
        self.stage_update()
        rename = Path.rename

        def interrupt_commit(path, target):
            if path == updater.STAGED:
                raise KeyboardInterrupt
            return rename(path, target)

        with patch.object(Path, "rename", interrupt_commit):
            with self.assertRaises(KeyboardInterrupt):
                updater.apply_staged()
        self.assert_original_intact()

    def test_policy_allocation_failure_leaves_the_original_installation_in_place(self):
        self.stage_update()
        with patch.object(updater.update_swap.tempfile, "NamedTemporaryFile", side_effect=OSError("disk full")):
            with self.assertRaises(OSError):
                updater.apply_staged()
        self.assert_original_intact()
        self.assertTrue(updater.apply_staged())

    def test_rollback_preserves_actual_runtime_policies_including_absence(self):
        for original in (b'{"custom": true}', None):
            with self.subTest(original=original):
                self.stage_update()
                policy = self.root / "runtime/firefox/distribution/policies.json"
                if original is None:
                    policy.unlink()
                else:
                    policy.write_bytes(original)
                rename = Path.rename

                def fail_commit(path, target):
                    if path == updater.STAGED:
                        raise OSError("injected commit failure")
                    return rename(path, target)

                with patch.object(Path, "rename", fail_commit):
                    with self.assertRaises(OSError):
                        updater.apply_staged()
                self.assertEqual(policy.read_bytes() if policy.exists() else None, original)
                self.assertEqual(updater.release_version(self.root), "0.2.0")
                self.assertTrue(updater.apply_staged())
                shutil.rmtree(self.root)

    def test_recovers_runtime_and_old_policies_after_an_interruption(self):
        self.stage_update()
        # Model interruption after moving Firefox and installing new policies,
        # before the final directory rename. Drop the update version so recovery
        # must also happen before stale staging can be removed.
        rename = Path.rename

        def terminate_before_commit(path, target):
            if path == updater.STAGED:
                raise SystemExit("simulate unhandled termination")
            return rename(path, target)

        with patch.object(Path, "rename", terminate_before_commit):
            with self.assertRaises(SystemExit):
                updater.apply_staged()
        (updater.STAGED / "release.json").write_text('{"version": "0.1.0"}')
        self.assertFalse(updater.apply_staged())
        self.assert_original_intact()
        self.assertFalse(updater.STAGED.exists())

    def test_recovery_can_retry_after_another_filesystem_error(self):
        self.stage_update()
        rename = Path.rename

        def fail_commit_and_restore(path, target):
            if path in (updater.STAGED, updater.PREVIOUS):
                raise OSError("filesystem unavailable")
            return rename(path, target)

        with patch.object(Path, "rename", fail_commit_and_restore):
            with self.assertRaises(OSError):
                updater.apply_staged()
        self.assertFalse(self.root.exists())
        self.assertTrue((updater.PREVIOUS / "runtime/firefox/firefox").is_file())
        self.assertTrue(updater.apply_staged())
        self.assertEqual(updater.release_version(self.root), "0.3.0")
        self.assertEqual(updater.firefox_version(self.root), "140.18.0")

    def test_cleanup_after_commit_keeps_the_new_installation(self):
        self.stage_update()
        self.root.rename(updater.PREVIOUS)
        updater.STAGED.rename(self.root)
        self.assertFalse(updater.apply_staged())
        self.assertEqual(updater.release_version(self.root), "0.3.0")
        self.assertFalse(updater.PREVIOUS.exists())

    def test_staging_without_a_launcher_is_not_activated(self):
        self.stage_update()
        (updater.STAGED / "still").unlink()
        self.assertFalse(updater.apply_staged())
        self.assert_original_intact()

    def test_manual_activation_error_is_reported(self):
        with patch.object(updater, "check", return_value={"version": "0.3.0"}), \
                patch.object(updater, "download"), patch.object(updater, "apply_staged", side_effect=OSError("disk full")), \
                patch.object(updater, "notify") as notify:
            self.assertEqual(updater.run("manual"), 1)
        self.assertIn("アップデートを適用できませんでした", notify.call_args.args[1])

    def test_busy_update_cannot_download_or_activate_over_another_writer(self):
        self.stage_update()
        with updater.update_swap.locked(self.root):
            self.assertFalse(updater.apply_staged())
            with self.assertRaises(updater.UpdateError):
                updater.download({"version": "0.4.0"})
        self.assert_original_intact()
        self.assertEqual(updater.release_version(updater.STAGED), "0.3.0")
        self.assertTrue(updater.apply_staged())

    def test_delayed_download_cannot_replace_a_newer_staged_update(self):
        make_install(self.root, "0.2.0")
        archive = make_archive("0.3.0")
        with self.serve("0.3.0", archive):
            entry = updater.check()
            make_install(updater.STAGED, "0.4.0", marker="latest")
            updater.download(entry)
        self.assertEqual(updater.release_version(updater.STAGED), "0.4.0")
        self.assertEqual((updater.STAGED / "still").read_text(), "latest")

    def test_delayed_download_is_discarded_after_the_update_was_installed(self):
        make_install(self.root, "0.2.0")
        archive = make_archive("0.3.0")
        with self.serve("0.3.0", archive):
            entry = updater.check()
            (self.root / "release.json").write_text('{"version": "0.3.0"}')
            updater.download(entry)
        self.assertFalse(updater.STAGED.exists())
        self.assertEqual(updater.release_version(self.root), "0.3.0")


if __name__ == "__main__":
    unittest.main()
