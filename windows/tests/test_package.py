#!/usr/bin/env python3
"""Check Windows staging and releases offline with throwaway signing keys.

STILL_TEST_WINE also enables compiling and running the real NSIS installer.
"""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import struct
import tempfile
import unittest
from unittest.mock import patch
import zipfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("still_windows_package", ROOT / "windows/package.py")
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)
release_set = package.release_set
fixture_spec = importlib.util.spec_from_file_location("still_signing_fixture", ROOT / "windows/tests/signing_fixture.py")
fixture = importlib.util.module_from_spec(fixture_spec)
fixture_spec.loader.exec_module(fixture)
NAME = "Still_0.9.0_x64-setup.exe"


# Include the executable, archives and branding files that Still must leave
# byte-for-byte intact. The real MAR extractor is replaced, as is downloading.
def pe_fixture(machine=0x8664):
    header = bytearray(70)
    header[:2] = b"MZ"
    struct.pack_into("<I", header, 0x3C, 64)
    header[64:68] = b"PE\0\0"
    struct.pack_into("<H", header, 68, machine)
    return bytes(header)


MOZILLA_FILES = {
    "firefox.exe": pe_fixture(),
    "xul.dll": pe_fixture(),
    "application.ini": b"[App]\nName=Firefox\nVersion=153.5.0\n",
    "omni.ja": b"Mozilla platform resources\0\xff",
    "browser/omni.ja": b"Mozilla branding and browser resources\0\xff",
    "defaults/pref/channel-prefs.js": b'pref("app.update.channel", "esr");\n',
    "browser/chrome/icons/default/mozilla.ico": b"Mozilla original icon\0\xff",
    "updater.exe": b"Mozilla updater\0\xff",
}


def extract_fixture(_archive, destination):
    for name, content in MOZILLA_FILES.items():
        path = destination / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)
    for name in ("updatev2.manifest", "updatev3.manifest"):
        (destination / name).write_bytes(b"MAR extraction instructions")


class StagingChecks(unittest.TestCase):
    def test_custom_build_architecture_is_checked_from_both_pe_headers(self):
        for platform, machine in (("win64", 0x8664), ("win64-aarch64", 0xAA64)):
            for mismatch in (None, "firefox.exe", "xul.dll"):
                with self.subTest(platform=platform, mismatch=mismatch), tempfile.TemporaryDirectory() as directory:
                    archive = Path(directory) / "arbitrary-name.zip"
                    with zipfile.ZipFile(archive, "w") as bundle:
                        for name, content in MOZILLA_FILES.items():
                            if name == "updater.exe":
                                continue
                            if name in ("firefox.exe", "xul.dll"):
                                content = pe_fixture((0x8664 if machine == 0xAA64 else 0xAA64) if name == mismatch else machine)
                            bundle.writestr(f"firefox/{name}", content)
                    if mismatch:
                        with self.assertRaisesRegex(SystemExit, "not a .* executable"):
                            package.stage_firefox("153.5.0esr", platform, Path(directory) / "stage", archive)
                    else:
                        self.assertEqual(package.stage_firefox("153.5.0esr", platform, Path(directory) / "stage", archive), "153.5.0")

    def test_invalid_pe_headers_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            exe = Path(directory) / "firefox.exe"
            for header in (b"", b"MZ", bytes(100), pe_fixture()[:69]):
                exe.write_bytes(header)
                with self.assertRaises(SystemExit):
                    package.check_machine(exe, "win64-aarch64")

    def test_adds_still_files_without_changing_mozilla_or_skipping_terms(self):
        for platform in ("win64", "win64-aarch64"):
            with self.subTest(platform=platform), tempfile.TemporaryDirectory() as directory:
                destination = Path(directory) / "firefox"
                destination.mkdir()
                (destination / "stale-file").write_text("earlier staging")
                archive = Path(directory) / "fixture.mar"
                with patch.object(package, "urlopen", side_effect=AssertionError("network forbidden")), \
                        patch.object(package, "download", return_value=archive) as download, \
                        patch.object(package, "extract_mar", side_effect=extract_fixture) as extract:
                    version = package.stage_firefox("153.5.0esr", platform, destination)
                self.assertEqual(version, "153.5.0")
                download.assert_called_once_with("153.5.0esr", platform)
                extract.assert_called_once_with(archive, destination)
                self.assertFalse((destination / "stale-file").exists())
                for name, content in MOZILLA_FILES.items():
                    self.assertEqual((destination / name).read_bytes(), content, name)

                sources = {
                    "still.cfg": ROOT / "windows/firefox/still.cfg",
                    "defaults/pref/still-autoconfig.js": ROOT / "windows/firefox/defaults/pref/still-autoconfig.js",
                }
                for name in ("chrome.manifest", "content/tokens.css", "content/settings.html", "content/settings.css", "content/settings.js", "content/newtab.html", "content/newtab.css", "content/newtab.js"):
                    sources[f"still/{name}"] = ROOT / "windows/firefox/still" / name
                for name in ("main-window.ico", "default.ico"):
                    sources[f"browser/chrome/icons/default/{name}"] = ROOT / "desktop/src-tauri/icons/icon.ico"
                for name, source in sources.items():
                    self.assertEqual((destination / name).read_bytes(), source.read_bytes(), name)

                policies = json.loads((destination / "distribution/policies.json").read_text())
                expected = json.loads((ROOT / "linux/policies.json").read_text())
                expected["policies"]["DisableDefaultBrowserAgent"] = True
                self.assertEqual(policies, expected)
                self.assertNotIn("SkipTermsOfUse", policies["policies"])
                self.assertNotIn("termsofuse", json.dumps(policies).lower())
                expected_files = set(MOZILLA_FILES) | set(sources) | {"distribution/policies.json"}
                self.assertEqual({str(path.relative_to(destination)) for path in destination.rglob("*") if path.is_file()}, expected_files)


    def test_own_build_turns_off_mozilla_updates_and_keeps_its_branding(self):
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / "firefox-153.5.0.ja.win64.zip"
            with zipfile.ZipFile(archive, "w") as bundle:
                for name, content in MOZILLA_FILES.items():
                    if name != "updater.exe":
                        bundle.writestr(f"firefox/{name}", content)
            destination = Path(directory) / "firefox"
            with patch.object(package, "urlopen", side_effect=AssertionError("network forbidden")), \
                    patch.object(package, "download", side_effect=AssertionError("Mozilla's build not used")):
                version = package.stage_firefox("153.5.0esr", "win64", destination, archive)
            self.assertEqual(version, "153.5.0")
            for name, content in MOZILLA_FILES.items():
                if name != "updater.exe":
                    self.assertEqual((destination / name).read_bytes(), content, name)
            self.assertFalse((destination / "updater.exe").exists())
            policies = json.loads((destination / "distribution/policies.json").read_text())["policies"]
            self.assertIs(policies["DisableAppUpdate"], True)
            self.assertFalse((destination / "browser/chrome/icons/default/main-window.ico").exists())
            with self.assertRaises(SystemExit):
                package.stage_firefox("153.6.0esr", "win64", destination, archive)

    def test_rejects_own_build_that_would_be_mistaken_for_mozilla(self):
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / "own-build.zip"
            with zipfile.ZipFile(archive, "w") as bundle:
                for name, content in MOZILLA_FILES.items():
                    bundle.writestr(f"firefox/{name}", content)
            with self.assertRaisesRegex(SystemExit, "rebuild with --disable-updater"):
                package.stage_firefox("153.5.0esr", "win64", Path(directory) / "firefox", archive)


@unittest.skipUnless(os.environ.get("STILL_TEST_WINE"), "set STILL_TEST_WINE to run NSIS installers under Wine")
class InstallerChecks(unittest.TestCase):
    """Compile the real installer and verify runtime migration in an isolated Wine prefix."""

    @classmethod
    def setUpClass(cls):
        work = tempfile.TemporaryDirectory(prefix="still-installer-test-")
        cls.addClassCleanup(work.cleanup)
        cls.work = Path(work.name)
        cls.environment = dict(os.environ, WINEPREFIX=str(cls.work / "wine"), WINEDEBUG="-all")
        cls.wine = os.environ["STILL_TEST_WINE"]
        cls.installers = {}
        cls.payloads = {}
        for kind in ("mozilla", "still"):
            stage = cls.work / f"stage-{kind}"
            runtime = stage / "firefox"
            if kind == "mozilla":
                with patch.object(package, "download", return_value=cls.work / "fixture.mar"), \
                        patch.object(package, "extract_mar", side_effect=extract_fixture):
                    package.stage_firefox("153.5.0esr", "win64", runtime)
            else:
                archive = cls.work / "own-build.zip"
                with zipfile.ZipFile(archive, "w") as bundle:
                    for name, content in MOZILLA_FILES.items():
                        if name != "updater.exe":
                            bundle.writestr(f"firefox/{name}", content)
                package.stage_firefox("153.5.0esr", "win64", runtime, archive)
            (runtime / "application.ini").write_text("[App]\nVersion=153.5.0\nBuildID=20261021000000\n")
            (stage / "still.exe").write_bytes(b"launcher fixture")
            installer = cls.work / f"{kind}-setup.exe"
            command = [package.makensis(), "-V2", "-INPUTCHARSET", "UTF8", "-DVERSION=0.9.0",
                       "-DARCH=x64", "-DFIREFOX_VERSION=153.5.0", f"-DSTAGE={stage}",
                       f"-DOUTFILE={installer}", f"-DICON={ROOT / 'desktop/src-tauri/icons/icon.ico'}"]
            if kind == "still":
                command.append("-DFIREFOX_BUILDID=20261021000000")
            subprocess.run([*command, str(ROOT / "windows/installer.nsi")], check=True,
                           capture_output=True, text=True, timeout=60)
            cls.installers[kind] = installer
            cls.payloads[kind] = runtime

    def test_runtime_migrations_refresh_still_files_and_restore_update_support(self):
        # installer, existing runtime, existing version, BuildID, preserve runtime
        cases = [
            ("mozilla", None, "", "", False),
            ("mozilla", "mozilla", "153.4.0", "old", False),
            ("mozilla", "mozilla", "153.5.0", "same", True),
            ("mozilla", "mozilla", "153.6.0", "newer", True),
            ("mozilla", "still", "153.4.0", "old", False),
            ("mozilla", "still", "153.5.0", "20261021000000", False),
            ("mozilla", "still", "153.6.0", "newer", False),
            ("still", None, "", "", False),
            ("still", "mozilla", "153.5.0", "20261021000000", False),
            ("still", "still", "153.5.0", "20261021000000", True),
            ("still", "still", "153.5.0", "different", False),
            ("still", "missing-executable", "153.5.0", "20261021000000", False),
            ("mozilla", "missing-executable", "153.6.0", "newer", False),
        ]
        for index, (kind, existing, version, build, preserve) in enumerate(cases):
            with self.subTest(installer=kind, existing=existing, version=version, build=build):
                destination = self.work / f"install-{index}"
                runtime = destination / "firefox"
                if existing:
                    runtime.mkdir(parents=True)
                    (runtime / "application.ini").write_text(f"[App]\nVersion={version}\nBuildID={build}\n")
                    if existing != "missing-executable":
                        (runtime / "firefox.exe").write_bytes(b"existing runtime")
                    if existing in ("mozilla", "missing-executable"):
                        (runtime / "updater.exe").write_bytes(b"existing updater")
                    (runtime / "stale-runtime-file").write_bytes(b"existing")
                    for name in ("still.cfg", "still/content/settings.js", "distribution/policies.json"):
                        target = runtime / name
                        target.parent.mkdir(parents=True, exist_ok=True)
                        target.write_bytes(b"old Still file")
                windows_destination = "Z:" + str(destination).replace("/", "\\")
                result = subprocess.run([self.wine, str(self.installers[kind]), "/S", f"/D={windows_destination}"],
                                        env=self.environment, capture_output=True, text=True, timeout=90)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual((runtime / "firefox.exe").read_bytes(),
                                 b"existing runtime" if preserve else MOZILLA_FILES["firefox.exe"])
                self.assertEqual((runtime / "stale-runtime-file").exists(), preserve)
                self.assertEqual((runtime / "updater.exe").exists(), kind == "mozilla")
                for name in ("still.cfg", "defaults/pref/still-autoconfig.js", "still/content/settings.js",
                             "distribution/policies.json"):
                    self.assertEqual((runtime / name).read_bytes(), (self.payloads[kind] / name).read_bytes(), name)
                policies = json.loads((runtime / "distribution/policies.json").read_text())["policies"]
                self.assertEqual(policies.get("DisableAppUpdate", False), kind == "still")


class CommitChecks(unittest.TestCase):
    """artifacts/ never presents a committed installer with another installer's signature."""

    def setUp(self):
        directory = fixture.fast_temporary_directory()
        self.addCleanup(directory.cleanup)
        self.artifacts = Path(directory.name) / "artifacts"
        self.artifacts.mkdir()
        self.signer = fixture.Signer()
        self.signatures = {}
        self.installer = self.artifacts / NAME

    def commit(self, data, signer=None, key=None):
        with tempfile.TemporaryDirectory(dir=self.artifacts, prefix=".build-") as work:
            built = (signer or self.signer).build(Path(work), NAME, data)
            self.signatures[data] = release_set.signature_path(built).read_text()
            return release_set.commit(built, self.artifacts, key or self.signer.public_key)

    def state(self):
        read = lambda path: path.read_bytes() if path.exists() else None
        record = release_set.read_record(self.installer) if release_set.record_path(self.installer).exists() else None
        return read(self.installer), read(release_set.signature_path(self.installer)), record

    def test_commit_writes_a_bound_set(self):
        result = self.commit(b"MZ one")
        self.assertEqual(sorted(p.name for p in self.artifacts.iterdir()),
                         [NAME, NAME + ".release.json", NAME + ".sig"])
        installer, signature, record = self.state()
        self.assertEqual(record, {"file": NAME, "size": 6, "sha256": hashlib.sha256(b"MZ one").hexdigest(),
                                  "signature": self.signatures[b"MZ one"]})
        self.assertEqual(result, record)
        self.assertEqual(signature.decode(), self.signatures[b"MZ one"])
        self.assertEqual(self.installer.stat().st_mode & 0o777, 0o644)

    def test_every_intermediate_state_is_safe_for_both_kinds_of_reader(self):
        self.commit(b"MZ one")
        seen = []
        real_replace = os.replace
        def observe(source, destination, *args, **kwargs):
            result = real_replace(source, destination, *args, **kwargs)
            if Path(destination).parent == self.artifacts:
                seen.append(self.state())
            return result
        with patch.object(release_set.os, "replace", side_effect=observe):
            self.commit(b"MZ two")
        self.assertEqual(len(seen), 3)
        for installer, signature, record in seen:
            # A reader of the plain pair sees a matching signature or none.
            if signature is not None:
                self.assertEqual(signature.decode(), self.signatures[installer])
            # A reader of the record accepts only bytes the record describes.
            if record["sha256"] == hashlib.sha256(installer).hexdigest():
                self.assertEqual(record["signature"], self.signatures[installer])
        self.assertEqual(seen[-1][2]["sha256"], hashlib.sha256(b"MZ two").hexdigest(), "the record lands last")
        self.assertEqual(seen[0][2]["sha256"], hashlib.sha256(b"MZ one").hexdigest())

    def test_interrupted_commit_keeps_the_previous_record(self):
        self.commit(b"MZ one")
        real_replace = os.replace
        def fail_signature(source, destination, *args, **kwargs):
            if Path(destination) == release_set.signature_path(self.installer):
                raise OSError("power lost")
            return real_replace(source, destination, *args, **kwargs)
        with patch.object(release_set.os, "replace", side_effect=fail_signature), self.assertRaises(OSError):
            self.commit(b"MZ two")
        installer, signature, record = self.state()
        self.assertEqual(installer, b"MZ two")
        self.assertIsNone(signature, "never the old signature beside the new installer")
        self.assertNotEqual(record["sha256"], hashlib.sha256(installer).hexdigest(), "readers refuse the half set")
        self.commit(b"MZ two")
        self.assertEqual(self.state()[2]["sha256"], hashlib.sha256(b"MZ two").hexdigest())

    def test_commit_refuses_a_signature_from_another_key(self):
        self.commit(b"MZ one")
        before = self.state()
        with self.assertRaises(release_set.SignatureError):
            self.commit(b"MZ forged", signer=fixture.Signer(), key=self.signer.public_key)
        self.assertEqual(self.state(), before)

    @unittest.skipUnless(fixture.CRYPTOGRAPHY, "Ed25519 verification needs the cryptography package")
    def test_signature_of_other_bytes_is_rejected(self):
        path = self.artifacts / NAME
        path.write_bytes(b"MZ real")
        with self.assertRaises(release_set.SignatureError):
            release_set.verify_signature(path, self.signer.sign(b"MZ other", NAME), self.signer.public_key)
        self.assertTrue(release_set.verify_signature(path, self.signer.sign(b"MZ real", NAME), self.signer.public_key))

    def test_signature_for_another_file_name_is_rejected(self):
        path = self.artifacts / NAME
        path.write_bytes(b"MZ real")
        with self.assertRaises(release_set.SignatureError):
            release_set.verify_signature(path, self.signer.sign(b"MZ real", "Still_0.1.0_x64-setup.exe"), self.signer.public_key)

    def test_seal_records_an_existing_pair(self):
        self.installer.write_bytes(b"MZ legacy")
        release_set.signature_path(self.installer).write_text(self.signer.sign(b"MZ legacy", NAME))
        sealed = package.seal(self.artifacts, "0.9.0", self.signer.public_key)
        self.assertEqual([item["file"] for item in sealed], [NAME])
        self.assertEqual(release_set.read_record(self.installer)["sha256"], hashlib.sha256(b"MZ legacy").hexdigest())
        release_set.signature_path(self.installer).write_text(fixture.Signer().sign(b"MZ legacy", NAME))
        with self.assertRaises(SystemExit):
            package.seal(self.artifacts, "0.9.0", self.signer.public_key)

    def test_package_builds_and_signs_under_temporary_names(self):
        build = Path(self.artifacts.parent) / "build"
        outfiles = []
        def makensis(arguments, check):
            outfile = Path(next(a for a in arguments if a.startswith("-DOUTFILE="))[len("-DOUTFILE="):])
            outfiles.append(outfile)
            self.assertFalse(self.installer.exists(), "nothing is written under the final name")
            outfile.write_bytes(b"MZ built")
        def sign(installer):
            release_set.signature_path(installer).write_text(self.signer.sign(installer.read_bytes(), installer.name))
        def stage(firefox, platform, destination, gecko=None):
            destination.mkdir(parents=True)
            return "153.5.0"
        launcher = Path(self.artifacts.parent) / "still.exe"
        launcher.write_bytes(b"launcher")
        with patch.object(package, "ARTIFACTS", self.artifacts), patch.object(package, "BUILD", build), \
                patch.object(package, "version", return_value="0.9.0"), \
                patch.object(package, "stage_firefox", side_effect=stage), \
                patch.object(package, "build_launcher", return_value=launcher), \
                patch.object(package, "makensis", return_value="makensis"), \
                patch.object(package.subprocess, "run", side_effect=makensis), \
                patch.object(package, "sign", side_effect=sign), \
                patch.object(release_set, "updater_public_key", return_value=self.signer.public_key):
            package.package("x64", "153.5.0esr")
        self.assertEqual(outfiles[0].parent.parent, self.artifacts)
        self.assertTrue(outfiles[0].parent.name.startswith("."))
        self.assertEqual(sorted(p.name for p in self.artifacts.iterdir()),
                         [NAME, NAME + ".release.json", NAME + ".sig"])
        self.assertEqual(release_set.read_record(self.installer)["sha256"], hashlib.sha256(b"MZ built").hexdigest())


if __name__ == "__main__":
    unittest.main(verbosity=2)
