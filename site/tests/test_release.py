"""Exercise release packaging and publication using tiny local fixtures."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import re
import tempfile
import threading
import unittest
from unittest.mock import patch
from urllib.parse import urljoin

ROOT = Path(__file__).resolve().parents[2]


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


package = module("still_linux_package", ROOT / "linux/package.py")
site = module("still_release_site", ROOT / "site/build.py")
fixture = module("still_signing_fixture", ROOT / "windows/tests/signing_fixture.py")
release_set = site.release_set


class ReleaseChecks(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.folder = Path(self.directory.name)
        self.source = self.folder / "source"
        self.source.mkdir()
        for name in package.RELEASE_PATHS:
            destination = self.source / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            if name == "runtime/firefox":
                destination.mkdir()
                (destination / "firefox").write_bytes(b"fixture runtime")
            elif (ROOT / "linux" / name).is_dir():
                shutil.copytree(ROOT / "linux" / name, destination)
            else:
                shutil.copy2(ROOT / "linux" / name, destination)
        self.artifacts = self.folder / "artifacts"
        self.output = self.artifacts / "StillBrowser-linux-x64.tar.gz"

    def test_archive_allowlist_and_extracted_launcher_dependencies(self):
        for name in ("tests/private.txt", "runtime/firefox.previous/firefox", "runtime/firefox/updates/pending.mar", "unrelated.txt"):
            path = self.source / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"local machine state")
        package.package(self.source, self.output, "0.3.0")
        self.assertEqual(self.output.stat().st_mode & 0o777, 0o644)
        with tarfile.open(self.output) as archive:
            first = archive.next()
            self.assertEqual(first.name, f"{package.ARCHIVE_ROOT}/release.json")
            self.assertEqual(json.load(archive.extractfile(first)), {"version": "0.3.0"})
            names = archive.getnames()
            self.assertFalse(any("private.txt" in n or "firefox.previous" in n or "/updates/" in n or "unrelated.txt" in n for n in names))
            archive.extractall(self.folder / "unpacked", filter="data")
        extracted = self.folder / "unpacked" / package.ARCHIVE_ROOT
        result = subprocess.run(
            [sys.executable, str(extracted / "still"), "--prepare-only", "--profile", str(self.folder / "profile")],
            capture_output=True, text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.folder / "profile/user.js").is_file())

    def test_rebuild_does_not_modify_existing_hardlinked_download(self):
        self.artifacts.mkdir()
        self.output.write_bytes(b"previous release")
        published = self.folder / "published.tar.gz"
        os.link(self.output, published)
        package.package(self.source, self.output, "0.3.0")
        self.assertEqual(published.read_bytes(), b"previous release")
        self.assertEqual(site.linux_version(self.output), "0.3.0")

    def test_failed_archive_preserves_previous_output_and_cleans_temporary_files(self):
        self.artifacts.mkdir()
        self.output.write_bytes(b"previous release")
        with patch.object(tarfile.TarFile, "add", side_effect=OSError("disk full")):
            with self.assertRaises(OSError):
                package.package(self.source, self.output, "0.3.0")
        self.assertEqual(self.output.read_bytes(), b"previous release")
        self.assertEqual(list(self.artifacts.iterdir()), [self.output])

    def test_published_snapshot_survives_source_overwrite(self):
        source, published = self.folder / "artifact", self.folder / "download"
        source.write_bytes(b"published bytes")
        site.place(source, published)
        source.write_bytes(b"next build")
        self.assertEqual(published.read_bytes(), b"published bytes")

    def test_failed_copy_keeps_the_served_snapshot(self):
        source, published = self.folder / "artifact", self.folder / "download"
        source.write_bytes(b"new bytes")
        published.write_bytes(b"published bytes")
        def fail(source, destination):
            destination.write_bytes(b"partial")
            raise OSError("disk full")
        with patch.object(site.shutil, "copy2", side_effect=fail):
            with self.assertRaises(OSError):
                site.place(source, published)
        self.assertEqual(published.read_bytes(), b"published bytes")
        self.assertEqual({p.name for p in self.folder.iterdir()}, {"source", "artifact", "download"})

    def test_manifest_metadata_comes_from_the_published_snapshot(self):
        package.package(self.source, self.output, "0.3.0")
        expected = self.output.read_bytes()
        out = self.folder / "public"
        place = site.place
        def publish_then_rebuild(source, destination):
            place(source, destination)
            source.write_bytes(b"next incomplete build")
        with patch.object(site, "OUT", out), patch.object(site, "ARTIFACTS", self.artifacts), \
                patch.object(site, "VERSION", "0.3.0"), patch.object(site, "SCREENSHOTS", {}), \
                patch.object(site, "place", side_effect=publish_then_rebuild):
            site.build()
        manifest = json.loads((out / "update.json").read_text())["linux"]
        self.assertEqual(manifest["version"], "0.3.0")
        self.assertEqual(manifest["size"], len(expected))
        self.assertEqual(manifest["sha256"], hashlib.sha256(expected).hexdigest())
        self.assertEqual((out / manifest["url"]).read_bytes(), expected)


VERSION = "0.9.0"
URL = "https://still.example.test/"
IMMUTABLE = re.compile(r"dl/[0-9A-Za-z.+_-]+/[0-9a-f]{16}/[^/]+")


def linux_archive(path, version, payload):
    """A tiny archive shaped like linux/package.py output: release.json first."""
    import io
    with tarfile.open(path, "w:gz") as archive:
        for name, data in ((f"{package.ARCHIVE_ROOT}/release.json", json.dumps({"version": version}).encode()),
                           (f"{package.ARCHIVE_ROOT}/still", payload)):
            info = tarfile.TarInfo(name)
            info.size = len(data)
            archive.addfile(info, io.BytesIO(data))


class PublicationChecks(unittest.TestCase):
    """One build publishes one generation: immutable files first, then manifests."""

    def setUp(self):
        directory = fixture.fast_temporary_directory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.artifacts = self.root / "artifacts"
        self.artifacts.mkdir()
        self.out = self.root / "public"
        self.signer = fixture.Signer()
        self.issued = {}  # sha256 of installer bytes -> signatures made for them
        self.linux(b"linux one")
        (self.artifacts / "StillBrowser-Android.apk").write_bytes(b"apk one")
        self.windows("x64", b"MZ x64 one")
        self.windows("arm64", b"MZ arm64 one")

    # Fixtures ---------------------------------------------------------------

    def linux(self, payload, version=VERSION):
        linux_archive(self.artifacts / "StillBrowser-linux-x64.tar.gz", version, payload)

    def signed(self, folder, arch, data, signer=None):
        name = f"Still_{VERSION}_{arch}-setup.exe"
        built = (signer or self.signer).build(folder, name, data)
        self.issued.setdefault(hashlib.sha256(data).hexdigest(), set()).add(built.with_name(name + ".sig").read_text())
        return built

    def windows(self, arch, data):
        """What windows/package.py does: build and sign in a temporary directory, then commit."""
        with tempfile.TemporaryDirectory(dir=self.artifacts, prefix=".build-") as work:
            release_set.commit(self.signed(Path(work), arch, data), self.artifacts, self.signer.public_key)

    def partial_commit(self, arch, data):
        """A rebuild interrupted after the installer and .sig, before its record."""
        with tempfile.TemporaryDirectory(dir=self.artifacts, prefix=".build-") as work:
            built = self.signed(Path(work), arch, data)
            os.replace(built.with_name(built.name + ".sig"), self.artifacts / (built.name + ".sig"))
            os.replace(built, self.artifacts / built.name)

    def build(self, **options):
        with patch.object(site, "android_version", return_value=(VERSION, 90)), \
                patch.object(site, "apk_abis", return_value=["arm64-v8a"]):
            return site.build(self.artifacts, self.out, version=VERSION, public_url=URL, screenshots={},
                              public_key=self.signer.public_key, retry_delay=0, **options)

    def served(self):
        return {str(p.relative_to(self.out)): p.read_bytes() for p in sorted(self.out.rglob("*"))
                if p.is_file() and p.name != site.LOCK}

    def manifests(self):
        names = ("update.json", "update/windows.json", "index.html")
        return {name: (self.out / name).read_bytes() for name in names if (self.out / name).exists()}

    def assert_consistent(self):
        """Every URL the live manifests name serves the bytes they describe."""
        update = json.loads((self.out / "update.json").read_text())
        for platform, entry in update.items():
            self.assertRegex(entry["url"], IMMUTABLE)
            absolute = urljoin(URL + "update.json", entry["url"])  # as linux/updater.py and Android resolve it
            data = (self.out / absolute[len(URL):]).read_bytes()
            self.assertEqual((len(data), hashlib.sha256(data).hexdigest()), (entry["size"], entry["sha256"]), platform)
        windows = self.out / "update/windows.json"
        if windows.exists():
            for target, entry in json.loads(windows.read_text())["platforms"].items():
                self.assertTrue(entry["url"].startswith(URL), target)
                path = self.out / entry["url"][len(URL):]
                self.assertRegex(entry["url"][len(URL):], IMMUTABLE)
                self.assertIn(entry["signature"], self.issued[hashlib.sha256(path.read_bytes()).hexdigest()], target)
                release_set.verify_signature(path, entry["signature"], self.signer.public_key)
                self.assertEqual(release_set.signature_path(path).read_text(), entry["signature"])
        for href in re.findall(r'href="(dl/[^"]+)"', (self.out / "index.html").read_text()):
            self.assertRegex(href, IMMUTABLE)
            self.assertTrue((self.out / href).is_file(), href)

    def url_of(self, target):
        return json.loads((self.out / "update/windows.json").read_text())["platforms"][target]["url"]

    # Checks -----------------------------------------------------------------

    def test_manifests_name_immutable_paths_in_their_formats(self):
        self.build()
        self.assert_consistent()
        windows = json.loads((self.out / "update/windows.json").read_text())
        self.assertEqual(set(windows), {"version", "pub_date", "platforms"})
        self.assertEqual(windows["version"], VERSION)
        self.assertRegex(windows["pub_date"], r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$")
        self.assertEqual(set(windows["platforms"]), {"windows-x86_64", "windows-aarch64"})
        for target, entry in windows["platforms"].items():
            self.assertEqual(set(entry), {"signature", "url"})
        x64 = self.url_of("windows-x86_64")
        digest = hashlib.sha256(b"MZ x64 one").hexdigest()
        self.assertEqual(x64, f"{URL}dl/{VERSION}/{digest[:16]}/Still_{VERSION}_x64-setup.exe")
        update = json.loads((self.out / "update.json").read_text())
        self.assertEqual(set(update), {"android", "linux"})
        self.assertEqual(set(update["linux"]), {"version", "url", "sha256", "size"})
        self.assertEqual(set(update["android"]), {"version", "code", "url", "sha256", "size", "abis"})
        self.assertEqual((update["android"]["code"], update["android"]["abis"]), (90, ["arm64-v8a"]))
        # Fixed names stay for people and hold the newest bytes.
        for name, data in ((f"Still_{VERSION}_x64-setup.exe", b"MZ x64 one"), ("StillBrowser-Android.apk", b"apk one")):
            self.assertEqual((self.out / "dl" / name).read_bytes(), data)
        self.assertEqual([p.name for p in self.out.glob(f"{site.STAGING_PREFIX}*")], [])

    def test_rebuilds_keep_immutable_paths_and_the_previous_generation(self):
        self.build()
        first = json.loads((self.out / "update.json").read_text())["linux"]["url"]
        first_x64 = self.url_of("windows-x86_64")
        inode = (self.out / first).stat().st_ino
        self.build()  # unchanged artifacts: same paths, files untouched
        self.assertEqual(json.loads((self.out / "update.json").read_text())["linux"]["url"], first)
        self.assertEqual((self.out / first).stat().st_ino, inode)

        self.linux(b"linux two")
        self.windows("x64", b"MZ x64 two")
        self.build()
        self.assert_consistent()
        second = json.loads((self.out / "update.json").read_text())["linux"]["url"]
        self.assertNotEqual(second, first)
        self.assertNotEqual(self.url_of("windows-x86_64"), first_x64)
        # The previous generation still serves its own bytes for clients mid-download.
        with tarfile.open(self.out / first) as archive:
            self.assertEqual(archive.extractfile(f"{package.ARCHIVE_ROOT}/still").read(), b"linux one")
        old_x64 = self.out / first_x64[len(URL):]
        self.assertEqual(old_x64.read_bytes(), b"MZ x64 one")
        release_set.verify_signature(old_x64, release_set.signature_path(old_x64).read_text(), self.signer.public_key)

        self.build()  # a repeated build does not age the previous generation out
        self.assertTrue((self.out / first).exists())
        self.linux(b"linux three")
        self.build()
        self.assert_consistent()
        self.assertFalse((self.out / first).exists())
        self.assertTrue((self.out / second).exists())
        self.assertFalse(old_x64.exists(), "x64 one is two generations old")

    def test_live_manifest_files_survive_a_lost_generation_record(self):
        self.build()
        first = json.loads((self.out / "update.json").read_text())["linux"]["url"]
        (self.out / site.GENERATIONS).unlink()
        self.linux(b"linux two")
        self.build()
        self.assertTrue((self.out / first).exists(), "named by the manifest that was just replaced")

    def test_signature_follows_the_snapshot_through_a_concurrent_rebuild(self):
        place, raced = site.place, []
        def rebuild_after_snapshot(source, destination):
            place(source, destination)
            if source.name.endswith("x64-setup.exe") and not raced:
                raced.append(source)
                self.windows("x64", b"MZ x64 two")  # commits after the snapshot, before the record is read
        with patch.object(site, "place", side_effect=rebuild_after_snapshot):
            self.build()
        self.assertTrue(raced)
        self.assert_consistent()
        self.assertEqual((self.out / self.url_of("windows-x86_64")[len(URL):]).read_bytes(), b"MZ x64 two")

    def test_unfinished_commit_is_retried_then_refused(self):
        self.build()
        before = self.manifests()
        self.partial_commit("x64", b"MZ x64 two")  # new installer and .sig; the record still names "one"
        with self.assertRaises(site.PublishError):
            self.build(attempts=3)
        self.assertEqual(self.manifests(), before)
        self.assert_consistent()
        place, snapshots = site.place, []
        def finish_during_retry(source, destination):
            place(source, destination)
            if source.name.endswith("x64-setup.exe"):
                snapshots.append(source)
                if len(snapshots) == 2:
                    self.windows("x64", b"MZ x64 two")  # the rebuild's record lands
        with patch.object(site, "place", side_effect=finish_during_retry):
            self.build(attempts=3)
        self.assertGreaterEqual(len(snapshots), 2)
        self.assert_consistent()
        self.assertEqual((self.out / self.url_of("windows-x86_64")[len(URL):]).read_bytes(), b"MZ x64 two")

    def test_threaded_rebuilds_always_publish_matching_pairs(self):
        source = self.artifacts / f"Still_{VERSION}_x64-setup.exe"
        stop = threading.Event()
        def rebuild():
            for round in range(60):
                if stop.is_set():
                    return
                self.windows("x64", b"MZ x64 %d" % (round % 3))
        writer = threading.Thread(target=rebuild)
        writer.start()
        try:
            staging = self.root / "staging"
            staging.mkdir()
            reads = 0
            while writer.is_alive() or reads < 20:
                size, digest, signature = site.snapshot_signed(
                    source, staging / source.name, self.signer.public_key, attempts=1000, delay=0)
                data = (staging / source.name).read_bytes()
                self.assertEqual((size, digest), (len(data), hashlib.sha256(data).hexdigest()))
                self.assertIn(signature, self.issued[digest])
                release_set.verify_signature(staging / source.name, signature, self.signer.public_key, source.name)
                reads += 1
        finally:
            stop.set()
            writer.join()

    def test_failed_builds_keep_the_previous_manifests_and_their_files(self):
        self.build()
        previous_manifests = self.manifests()
        previous_files = self.served()
        real_replace = os.replace
        def fail_publishing(source, destination, *args, **kwargs):
            if IMMUTABLE.search(str(destination)) and "/dl/" in str(destination):
                raise OSError("disk full")
            return real_replace(source, destination, *args, **kwargs)
        def fail_copy(source, destination):
            Path(destination).write_bytes(b"partial")
            raise OSError("disk full")
        real_prepare = release_set.prepare
        def fail_index(path, *args, **kwargs):
            if path.name == "index.html":  # after windows.json and update.json were prepared
                raise OSError("disk full")
            return real_prepare(path, *args, **kwargs)
        failures = {
            "snapshot copy": patch.object(site.shutil, "copy2", side_effect=fail_copy),
            "immutable rename": patch.object(site.os, "replace", side_effect=fail_publishing),
            "manifest rendering": patch.object(site, "render", side_effect=RuntimeError("template")),
            "manifest preparation": patch.object(release_set, "prepare", side_effect=fail_index),
        }
        for label, failure in failures.items():
            with self.subTest(label):
                self.linux(b"linux next " + label.encode())
                self.windows("x64", b"MZ x64 next " + label.encode())
                with failure, self.assertRaises(Exception):
                    self.build()
                self.assertEqual(self.manifests(), previous_manifests)
                served = self.served()
                for name, data in previous_files.items():
                    if IMMUTABLE.fullmatch(name.removesuffix(".sig")):
                        self.assertEqual(served[name], data, name)
                self.assert_consistent()
                self.assertEqual([p.name for p in self.out.glob(f"{site.STAGING_PREFIX}*")], [])
        self.build()  # the next good build publishes and cleans up
        self.assert_consistent()
        leftovers = [p for p in self.out.rglob(".*") if p.name not in (site.LOCK, site.GENERATIONS)]
        self.assertEqual(leftovers, [])

    def test_failed_manifest_rename_leaves_each_manifest_consistent(self):
        self.build()
        self.linux(b"linux two")
        self.windows("x64", b"MZ x64 two")
        real_replace = os.replace
        def fail_update_json(source, destination, *args, **kwargs):
            if Path(destination) == self.out / "update.json":
                raise OSError("I/O error")
            return real_replace(source, destination, *args, **kwargs)
        with patch.object(site.os, "replace", side_effect=fail_update_json), self.assertRaises(OSError):
            self.build()
        self.assert_consistent()  # windows.json may be new; update.json and index.html are the previous ones
        self.build()
        self.assert_consistent()

    def test_signature_from_another_key_is_refused(self):
        self.build()
        before = self.manifests()
        with tempfile.TemporaryDirectory(dir=self.artifacts, prefix=".build-") as work:
            other = fixture.Signer()
            built = self.signed(Path(work), "x64", b"MZ x64 forged", signer=other)
            release_set.commit(built, self.artifacts, other.public_key)
        with self.assertRaises(site.PublishError):
            self.build()
        self.assertEqual(self.manifests(), before)

    def test_served_site_updates_linux_through_relative_immutable_urls(self):
        import http.server
        from urllib.request import urlopen
        handler_class = type("Quiet", (http.server.SimpleHTTPRequestHandler,), {"log_message": lambda *a: None})
        handler = lambda *a, **kw: handler_class(*a, directory=str(self.out), **kw)
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        base = f"http://127.0.0.1:{server.server_address[1]}/"
        self.build()
        first = json.loads((self.out / "update.json").read_text())["linux"]
        self.linux(b"linux two")
        self.build()
        second = json.loads((self.out / "update.json").read_text())["linux"]
        with urlopen(urljoin(base + "update.json", first["url"])) as response:  # the old URL still serves old bytes
            self.assertEqual(hashlib.sha256(response.read()).hexdigest(), first["sha256"])
        # The real Linux updater, installed as an older version, resolves and verifies the new URL.
        install = self.root / "install" / package.ARCHIVE_ROOT
        install.mkdir(parents=True)
        for name in ("updater.py", "update_swap.py"):
            shutil.copy2(ROOT / "linux" / name, install / name)
        (install / "release.json").write_text(json.dumps({"version": "0.0.1"}))
        script = ("import json, updater; entry = updater.check(); updater.download(entry); "
                  "print(json.dumps([entry['url'], updater.release_version(updater.STAGED)]))")
        result = subprocess.run([sys.executable, "-c", script], cwd=install, capture_output=True, text=True,
                                env=dict(os.environ, STILL_UPDATE_URL=base + "update.json"), timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout), [second["url"], VERSION])
        with tarfile.open(self.out / second["url"]) as archive:
            staged = (install.with_name(install.name + ".update") / "still").read_bytes()
            self.assertEqual(staged, archive.extractfile(f"{package.ARCHIVE_ROOT}/still").read())

    def test_legacy_signature_without_record_is_refused_but_unsigned_installers_publish(self):
        self.build()
        before = self.manifests()
        record = release_set.record_path(self.artifacts / f"Still_{VERSION}_x64-setup.exe")
        record.unlink()
        with self.assertRaises(site.PublishError) as raised:
            self.build(attempts=2)
        self.assertIn("--seal", str(raised.exception))
        self.assertEqual(self.manifests(), before)
        for arch in ("x64", "arm64"):
            installer = self.artifacts / f"Still_{VERSION}_{arch}-setup.exe"
            release_set.signature_path(installer).unlink()
            release_set.record_path(installer).unlink(missing_ok=True)
        self.build()
        self.assertFalse((self.out / "update/windows.json").exists())
        self.assert_consistent()
        self.assertIn(f"Still_{VERSION}_x64-setup.exe", (self.out / "index.html").read_text())


if __name__ == "__main__":
    unittest.main()
