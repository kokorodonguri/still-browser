#!/usr/bin/env python3
"""Guard the new tab picture: accepted formats, storage, and the stylesheet."""
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "linux"))

import newtab_image

PNG = b"\x89PNG\r\n\x1a\n" + b"\0" * 32
JPEG = b"\xff\xd8\xff\xe0" + b"\0" * 32


def still(*args):
    # Not a terminal, so results would be notifications; keep them on stdout for the test.
    env = {"PATH": "/usr/bin:/bin", "HOME": str(Path.home())}
    return subprocess.run([sys.executable, str(ROOT / "linux/still"), *args], capture_output=True, text=True, env=env)


class Formats(unittest.TestCase):
    def test_sniff_reads_the_content_not_the_name(self):
        samples = {
            "png": PNG, "jpg": JPEG, "gif": b"GIF89a" + b"\0" * 8, "webp": b"RIFF\0\0\0\0WEBPVP8 ",
            "avif": b"\0\0\0\x1cftypavif\0\0", "bmp": b"BM" + b"\0" * 30,
        }
        for extension, data in samples.items():
            self.assertEqual(newtab_image.sniff(data), extension)
        for data in (b"", b"<svg xmlns='http://www.w3.org/2000/svg'/>", b"BM", b"RIFF\0\0\0\0WAVE"):
            self.assertIsNone(newtab_image.sniff(data), data)


class Storage(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.profile = Path(self.directory.name) / "profile"
        self.source = Path(self.directory.name) / "holiday.JPEG"
        self.source.write_bytes(JPEG)

    def tearDown(self):
        self.directory.cleanup()

    def test_store_copies_under_a_generated_name_and_replaces_the_previous(self):
        first = newtab_image.store(self.profile, self.source)
        self.assertRegex(first.name, r"^newtab-\d+\.jpg$")
        self.assertEqual(first.parent, self.profile / "chrome/newtab")
        self.assertEqual(first.read_bytes(), JPEG)
        unrelated = first.parent / "notes.txt"
        unrelated.write_text("not Still's")
        png = Path(self.directory.name) / "b.png"
        png.write_bytes(PNG)
        second = newtab_image.store(self.profile, png)
        self.assertNotEqual(first.name, second.name)
        self.assertEqual(newtab_image.current(self.profile), second)
        self.assertEqual(sorted(p.name for p in first.parent.iterdir()), sorted([second.name, "notes.txt"]))

    def test_unsupported_or_large_files_change_nothing(self):
        kept = newtab_image.store(self.profile, self.source)
        text = Path(self.directory.name) / "picture.png"
        text.write_text("not a picture")
        large = Path(self.directory.name) / "large.png"
        large.write_bytes(PNG + b"\0" * newtab_image.MAX_BYTES)
        for path in (text, large):
            with self.assertRaisesRegex(newtab_image.ImageError, "使えません"):
                newtab_image.store(self.profile, path)
        with self.assertRaisesRegex(newtab_image.ImageError, "読み込めません"):
            newtab_image.store(self.profile, Path(self.directory.name) / "missing.png")
        self.assertEqual(list(kept.parent.iterdir()), [kept])

    def test_stylesheet_points_at_the_stored_copy_only_on_new_tabs(self):
        self.assertEqual(newtab_image.stylesheet(self.profile), "")
        stored = newtab_image.store(self.profile, self.source)
        css = newtab_image.stylesheet(self.profile)
        self.assertIn(f'url("newtab/{stored.name}")', css)
        self.assertIn('@-moz-document url("about:home"), url("about:newtab")', css)
        self.assertNotIn("about:blank", css)
        self.assertNotIn(str(self.source), css)


class Launcher(unittest.TestCase):
    def test_set_survives_launch_preparation_and_remove_clears_it(self):
        with tempfile.TemporaryDirectory() as directory:
            profile = Path(directory) / "profile"
            source = Path(directory) / "picture.png"
            source.write_bytes(PNG)
            result = still("--profile", str(profile), "--newtab-image", str(source))
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("設定しました", result.stdout)
            stored = newtab_image.current(profile)
            css = profile / "chrome/userContent.css"
            self.assertIn(stored.name, css.read_text())
            # Each launch rewrites userContent.css from the repository copy; the picture stays.
            self.assertEqual(still("--profile", str(profile), "--prepare-only").returncode, 0)
            self.assertIn(stored.name, css.read_text())
            self.assertTrue(css.read_text().startswith((ROOT / "linux/profile/chrome/userContent.css").read_text()))
            result = still("--profile", str(profile), "--remove-newtab-image")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("外しました", result.stdout)
            self.assertIsNone(newtab_image.current(profile))
            self.assertEqual(css.read_text(), (ROOT / "linux/profile/chrome/userContent.css").read_text())

    def test_rejected_file_reports_and_keeps_the_current_picture(self):
        with tempfile.TemporaryDirectory() as directory:
            profile = Path(directory) / "profile"
            good = Path(directory) / "good.png"
            good.write_bytes(PNG)
            self.assertEqual(still("--profile", str(profile), "--newtab-image", str(good)).returncode, 0)
            before = newtab_image.current(profile)
            bad = Path(directory) / "document.png"
            bad.write_text("text")
            result = still("--profile", str(profile), "--newtab-image", str(bad))
            self.assertEqual(result.returncode, 1)
            self.assertIn("使えません", result.stdout)
            self.assertNotIn("Traceback", result.stderr)
            self.assertEqual(newtab_image.current(profile), before)
            self.assertIn(before.name, (profile / "chrome/userContent.css").read_text())

    def test_picture_chosen_in_the_dialog_is_stored(self):
        with tempfile.TemporaryDirectory() as directory:
            picture = Path(directory) / "chosen picture.png"
            picture.write_bytes(PNG)
            zenity = Path(directory) / "zenity"
            zenity.write_text(f"#!/bin/sh\nprintf '%s\\n' '{picture}'\n")
            zenity.chmod(0o755)
            env = {"PATH": f"{directory}:/usr/bin:/bin", "HOME": directory}  # no display, so GTK is skipped
            profile = Path(directory) / "p"
            result = subprocess.run([sys.executable, str(ROOT / "linux/still"), "--profile", str(profile), "--newtab-image"],
                                    capture_output=True, text=True, env=env)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(newtab_image.current(profile).read_bytes(), PNG)

    def test_cancelled_dialog_changes_nothing(self):
        with tempfile.TemporaryDirectory() as directory:
            zenity = Path(directory) / "zenity"
            zenity.write_text("#!/bin/sh\nexit 1\n")
            zenity.chmod(0o755)
            profile = Path(directory) / "p"
            result = subprocess.run([sys.executable, str(ROOT / "linux/still"), "--profile", str(profile), "--newtab-image"],
                                    capture_output=True, text=True, env={"PATH": f"{directory}:/usr/bin:/bin", "HOME": directory})
            self.assertEqual((result.returncode, result.stdout), (0, ""), result.stderr)
            self.assertFalse(profile.exists())

    def test_no_dialog_available_explains_the_terminal_option(self):
        with tempfile.TemporaryDirectory() as directory:
            env = {"PATH": directory, "HOME": directory}  # no zenity/kdialog, and no display for GTK
            result = subprocess.run([sys.executable, str(ROOT / "linux/still"), "--profile", str(Path(directory) / "p"),
                                     "--newtab-image"], capture_output=True, text=True, env=env)
            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertIn("--newtab-image 画像のパス", result.stdout)


if __name__ == "__main__":
    unittest.main(verbosity=2)
