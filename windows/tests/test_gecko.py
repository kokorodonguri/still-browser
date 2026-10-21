"""Offline checks for architecture-specific source/configuration selection."""
import importlib.util
from pathlib import Path
import tempfile
import subprocess
import os
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("still_gecko", ROOT / "windows/gecko/build.py")
gecko = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gecko)


class GeckoChecks(unittest.TestCase):
    def test_target_configs_and_test_opt_in(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)
            for arch, (target, _) in gecko.ARCHES.items():
                for tests in (True, False):
                    config = gecko.mozconfig(source, source / f"obj-{arch}", source / "l10n",
                                              source / "key", arch, tests).read_text()
                    self.assertEqual(config.count("ac_add_options --target="), 1)
                    self.assertIn(f"--target={target}", config)
                    self.assertIn(f"MOZ_OBJDIR={source}/obj-{arch}", config)
                    self.assertEqual("--disable-tests" in config, not tests)
                    self.assertIn(f"--with-google-safebrowsing-api-keyfile={source}/key", config)
                    self.assertIn("--disable-updater", config)

    def test_arm64_patches_do_not_change_x64(self):
        self.assertEqual(gecko.patches("x64"), gecko.PATCHES)
        self.assertEqual(len(gecko.patches("arm64")), len(gecko.PATCHES) + 1)
        self.assertNotEqual(gecko.patches_digest("arm64"), gecko.patches_digest("x64"))
        with tempfile.TemporaryDirectory() as directory:
            overlay = Path(directory)
            (overlay / "actor.sys.mjs").write_text("old")
            with patch.object(gecko, "COMMON_FILES", overlay):
                arm = gecko.patches_digest("arm64")
                x64 = gecko.patches_digest("x64")
                (overlay / "actor.sys.mjs").write_text("new")
                self.assertNotEqual(gecko.patches_digest("arm64"), arm)
                self.assertNotEqual(gecko.patches_digest("x64"), x64)

    def test_build_variants_are_explicit(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)
            for arch in gecko.ARCHES:
                config = gecko.mozconfig(source, source / "obj", source / "l10n", None,
                    arch, False, "thin", "use", source / "profile.profdata", 32).read_text()
                self.assertIn("--enable-lto=thin", config)
                self.assertIn("--enable-profile-use", config)
                self.assertIn(f"--with-pgo-profile-path={source}/profile.profdata", config)
                self.assertIn("-DSTILL_DIRTY_MAX_PAGES=32", config)
                baseline = gecko.mozconfig(source, source / "obj", source / "l10n", None, arch).read_text()
                self.assertIn("--disable-lto", baseline)
                self.assertNotIn("--enable-profile-", baseline)

    def test_existing_compiler_flags_survive_shell_expansion(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)
            config = gecko.mozconfig(source, source / 'obj', source / 'l10n', None, dirty_pages=32).read_text()
            exports = '\n'.join(line for line in config.splitlines() if line.startswith(('export CFLAGS=', 'export CXXFLAGS=')))
            for original in ('', '-O2 -DNAME="a b"'):
                output = subprocess.check_output(['sh', '-c', exports + '\nprintf "%s\\n" "$CFLAGS" "$CXXFLAGS"'],
                    env=dict(os.environ, CFLAGS=original, CXXFLAGS=original), text=True).splitlines()
                self.assertEqual(output, [(original + ' ' if original else '') + '-DSTILL_DIRTY_MAX_PAGES=32'] * 2)

    def test_pgo_validation_precedes_download(self):
        for flags in (["--pgo", "use"], ["--pgo", "generate", "--lto", "thin"]):
            with patch("sys.argv", ["build.py", "--firefox", "153.5.0esr", *flags]), \
                    patch.object(gecko, "prepare_source", side_effect=AssertionError("must not download")):
                with self.assertRaises(SystemExit):
                    gecko.main()

    def test_arm64_requires_safe_browsing_key_before_build_or_download(self):
        with patch("sys.argv", ["build.py", "--arch", "arm64", "--firefox", "153.5.0esr"]), \
                patch.object(gecko, "prepare_source", side_effect=AssertionError("must not build")):
            with self.assertRaisesRegex(SystemExit, "require --safebrowsing-key"):
                gecko.main()

    def test_package_pattern_selects_only_requested_architecture(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            for arch, (_, platform) in gecko.ARCHES.items():
                objdir = base / f"obj-153.5.0esr-{arch}"
                (objdir / "dist").mkdir(parents=True)
                archive = objdir / "dist" / f"firefox-153.5.0.ja.{platform}.zip"
                archive.touch()
                # Wrong platform and non-Japanese archives must not be selected.
                (objdir / "dist" / "firefox-999.en-US.win64.zip").touch()
                args = ["build.py", "--firefox", "153.5.0esr", "--arch", arch, "--safebrowsing-key", str(base / "key")]
                (base / "key").write_text("test-key")
                with patch("sys.argv", args), patch.object(gecko, "HOME", base), \
                        patch.object(gecko, "prepare_source", return_value=base) as prepare, \
                        patch.object(gecko, "l10n", return_value=base / "l10n"), \
                        patch.object(gecko, "run"), patch("builtins.print") as output:
                    gecko.main()
                prepare.assert_called_once_with("153.5.0esr", arch)
                output.assert_called_with(archive)


if __name__ == "__main__":
    unittest.main()
