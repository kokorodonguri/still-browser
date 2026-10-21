#!/usr/bin/env python3
"""Guard profile persistence and interrupted-install behavior."""
from concurrent.futures import ThreadPoolExecutor
import importlib.machinery
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]


def module(name, source):
    loader = importlib.machinery.SourceFileLoader(name, str(source))
    spec = importlib.util.spec_from_loader(name, loader)
    result = importlib.util.module_from_spec(spec)
    loader.exec_module(result)
    return result


sys.path.insert(0, str(ROOT / "linux"))

launcher = module("still_launcher", ROOT / "linux/still")
installer = module("still_installer", ROOT / "linux/install.py")


from profile_migration import decode_session, encode_session


class ProfileChecks(unittest.TestCase):
    def test_urlbar_policy_defaults_prioritize_browsing_history(self):
        policies = json.loads((ROOT / "linux/policies.json").read_text())["policies"]
        expected = {
            "browser.urlbar.showSearchSuggestionsFirst": False,
            "browser.urlbar.autoFill.adaptiveHistory.enabled": True,
            "browser.urlbar.suggest.trending": False,
            "browser.urlbar.suggest.engines": False,
            "browser.urlbar.suggest.weather": False,
            "browser.urlbar.suggest.yelp": False,
            "browser.urlbar.suggest.addons": False,
            "browser.urlbar.suggest.mdn": False,
            "browser.urlbar.suggest.fakespot": False,
        }
        for preference, value in expected.items():
            with self.subTest(preference=preference):
                self.assertEqual(policies["Preferences"][preference], {"Value": value, "Status": "default"})

    def test_parallel_profile_preparation_writes_complete_files(self):
        with tempfile.TemporaryDirectory() as directory:
            profile = Path(directory) / "profile"
            with ThreadPoolExecutor(max_workers=4) as pool:
                list(pool.map(lambda _: launcher.prepare_profile(profile), range(8)))
            self.assertTrue((profile / "user.js").read_text().endswith(");\n"))
            self.assertEqual((profile / "chrome/userChrome.css").read_bytes(), (ROOT / "linux/profile/chrome/userChrome.css").read_bytes())
            self.assertFalse(list(profile.glob("tmp*")))

    def test_existing_prefs_are_not_replaced_or_shortcuts_reset(self):
        with tempfile.TemporaryDirectory() as directory:
            profile = Path(directory)
            existing = ('user_pref("browser.newtabpage.pinned", "user-chosen-shortcuts");\n'
                        'user_pref("sidebar.main.tools", "history");\n'
                        'user_pref("still.uiRevision", 2);\n')
            (profile / "prefs.js").write_text(existing)
            launcher.prepare_profile(profile)
            self.assertEqual((profile / "prefs.js").read_text(), existing)
            self.assertNotIn('"browser.newtabpage.pinned"', (profile / "user.js").read_text())
            self.assertNotIn('"sidebar.main.tools"', (profile / "user.js").read_text())

    def test_migration_repeats_until_firefox_persists_revision(self):
        with tempfile.TemporaryDirectory() as directory:
            profile = Path(directory)
            original = 'user_pref("sidebar.visibility", "always-show");\n'
            (profile / "prefs.js").write_text(original)
            for _ in range(2):
                launcher.prepare_profile(profile)
                self.assertIn('user_pref("sidebar.visibility", "hide-sidebar");', (profile / "user.js").read_text())
                self.assertEqual((profile / "prefs.js").read_text(), original)
            (profile / "prefs.js").write_text(original + 'user_pref("still.uiRevision", 2);\n')
            launcher.prepare_profile(profile)
            self.assertNotIn('sidebar.visibility', (profile / "user.js").read_text())
            self.assertNotIn('browser.uiCustomization.state', (profile / "user.js").read_text())

    def test_search_suggestions_turn_on_once(self):
        with tempfile.TemporaryDirectory() as directory:
            profile = Path(directory)
            original = ('user_pref("browser.search.suggest.enabled", false);\n'
                        'user_pref("still.uiRevision", 2);\n')
            (profile / "prefs.js").write_text(original)
            launcher.prepare_profile(profile)
            self.assertIn('user_pref("browser.search.suggest.enabled", true);', (profile / "user.js").read_text())
            self.assertIn('user_pref("browser.urlbar.suggest.searches", true);', (profile / "user.js").read_text())
            (profile / "prefs.js").write_text(original + 'user_pref("still.suggestRevision", 1);\n')
            launcher.prepare_profile(profile)
            self.assertNotIn("suggest", (profile / "user.js").read_text())

    def test_sidebar_migration_preserves_tabs_and_backs_up_session(self):
        with tempfile.TemporaryDirectory() as directory:
            profile = Path(directory)
            (profile / "prefs.js").write_text('user_pref("sidebar.visibility", "always-show");')
            state = {"windows": [{"selected": 2, "tabs": [{"entries": [{"url": "https://example.org/"}]}, {"entries": [{"url": "https://mozilla.org/"}]}], "sidebar": {"launcherVisible": True, "panelOpen": True, "hidden": False}}]}
            original = encode_session(state)
            session = profile / "sessionstore.jsonlz4"
            session.write_bytes(original)
            launcher.prepare_profile(profile)
            result = decode_session(session.read_bytes())
            self.assertEqual(result['windows'][0]['tabs'], state['windows'][0]['tabs'])
            self.assertEqual(result['windows'][0]['selected'], 2)
            self.assertFalse(result['windows'][0]['sidebar']['launcherVisible'])
            self.assertEqual((profile / 'sessionstore.jsonlz4.before-minimal').read_bytes(), original)
            launcher.prepare_profile(profile)
            self.assertEqual((profile / 'sessionstore.jsonlz4.before-minimal').read_bytes(), original)

    def test_invalid_session_is_preserved_without_marking_migration(self):
        with tempfile.TemporaryDirectory() as directory:
            profile = Path(directory)
            session = profile / 'sessionstore.jsonlz4'
            session.write_bytes(b'incomplete')
            with self.assertRaisesRegex(SystemExit, '元のデータは保持'):
                launcher.prepare_profile(profile)
            self.assertEqual(session.read_bytes(), b'incomplete')
            self.assertFalse((profile / 'user.js').exists())

    def test_invalid_profile_path_has_actionable_error(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "not-a-directory"
            path.write_text("keep this file")
            result = subprocess.run([str(ROOT / "linux/still"), "--profile", str(path), "--prepare-only"], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("保存できません", result.stderr)
            self.assertNotIn("Traceback", result.stderr)
            self.assertEqual(path.read_text(), "keep this file")

    def test_import_opens_firefox_import_wizard(self):
        with tempfile.TemporaryDirectory() as directory, \
                patch.object(sys, "argv", ["still", "--profile", directory, "--import", "https://example.org/"]), \
                patch.object(launcher.updater, "apply_staged", return_value=False), \
                patch.object(launcher.updater, "release_version", return_value=None), \
                patch.object(launcher.os, "execv") as start:
            launcher.main()
            self.assertEqual(start.call_args.args[1][-2:], ["about:preferences#general-migrate-autoclose", "https://example.org/"])

    def test_import_while_running_explains_instead_of_starting(self):
        with tempfile.TemporaryDirectory() as directory, \
                patch.object(sys, "argv", ["still", "--profile", directory, "--import"]), \
                patch.object(launcher.updater, "apply_staged", return_value=False), \
                patch.object(launcher, "profile_running", return_value=True), \
                patch.object(launcher, "report") as told, \
                patch.object(launcher.os, "execv") as start:
            with self.assertRaises(SystemExit):
                launcher.main()
            start.assert_not_called()
            self.assertIn("about:preferences#general-migrate", told.call_args.args[0])

    def test_invalid_version_is_rejected_before_network_access(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(installer, "urlopen") as request:
            with self.assertRaisesRegex(SystemExit, "Invalid Firefox version"):
                installer.install(Path(directory), "../../unexpected")
            request.assert_not_called()

    def test_checksum_failure_keeps_existing_firefox(self):
        with tempfile.TemporaryDirectory() as directory:
            destination = Path(directory)
            binary = destination / "firefox/firefox"
            binary.parent.mkdir()
            binary.write_bytes(b"existing verified browser")
            checksums = ("0" * 64 + "  linux-x86_64/ja/firefox-140.17.0esr.tar.xz\n").encode()
            with patch.object(installer, "urlopen", side_effect=[io.BytesIO(checksums), io.BytesIO(b"incomplete download")]):
                with self.assertRaisesRegex(SystemExit, "checksum mismatch"):
                    installer.install(destination, "140.17.0esr")
            self.assertEqual(binary.read_bytes(), b"existing verified browser")
            self.assertFalse((destination / "firefox.previous").exists())

    def test_missing_release_metadata_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(installer, "urlopen", return_value=io.BytesIO(b'{}')):
            with self.assertRaisesRegex(SystemExit, "Invalid Firefox version"):
                installer.install(Path(directory))

    def test_empty_version_is_rejected_before_network_access(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(installer, "urlopen") as request:
            with self.assertRaisesRegex(SystemExit, "Invalid Firefox version"):
                installer.install(Path(directory), "")
            request.assert_not_called()

    def test_interrupted_install_recovers_previous_browser(self):
        with tempfile.TemporaryDirectory() as directory:
            destination = Path(directory)
            previous = destination / "firefox.previous/firefox"
            previous.parent.mkdir()
            previous.write_bytes(b"previous working browser")
            with patch.object(installer, "urlopen", return_value=io.BytesIO(b"\nno-matching-checksum\n")):
                with self.assertRaisesRegex(SystemExit, "checksum list"):
                    installer.install(destination, "140.17.0esr")
            self.assertEqual((destination / "firefox/firefox").read_bytes(), b"previous working browser")
            self.assertFalse((destination / "firefox.previous").exists())


if __name__ == "__main__":
    unittest.main(verbosity=2)
