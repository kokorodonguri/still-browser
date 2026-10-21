#!/usr/bin/env python3
"""Download an unmodified Mozilla Firefox ESR build and verify its checksum."""
import argparse
import fcntl
import hashlib
import json
from pathlib import Path
import platform
import re
import shutil
import tarfile
import tempfile
from urllib.request import urlopen
from urllib.error import URLError

ROOT = Path(__file__).resolve().parent


def install(destination: Path, version: str | None = None):
    if platform.system() != "Linux" or platform.machine() not in ("x86_64", "amd64"):
        raise SystemExit("This package currently supports Linux x86_64.")
    destination = destination.resolve()
    target = destination / "firefox"
    backup = destination / "firefox.previous"
    installed_binaries = {target / "firefox-bin", backup / "firefox-bin"}
    for executable in Path("/proc").glob("[0-9]*/exe"):
        try:
            if executable.resolve(strict=True) in installed_binaries:
                raise SystemExit("Close Still before updating its Firefox runtime, then run the installer again.")
        except (OSError, RuntimeError):
            continue
    if version is None:
        with urlopen("https://product-details.mozilla.org/1.0/firefox_versions.json", timeout=30) as response:
            metadata = json.load(response)
            version = metadata.get("FIREFOX_ESR") if isinstance(metadata, dict) else None
    if not isinstance(version, str) or not re.fullmatch(r"\d+\.\d+(?:\.\d+)?esr", version):
        raise SystemExit("Invalid Firefox version. Retry or specify a release such as --version 140.17.0esr.")
    # Recover a previous install if interruption happened between the directory renames.
    if backup.exists() and not target.exists():
        backup.rename(target)
    relative = f"linux-x86_64/ja/firefox-{version}.tar.xz"
    base = f"https://archive.mozilla.org/pub/firefox/releases/{version}/"
    with urlopen(base + "SHA256SUMS", timeout=30) as response:
        checksums = response.read().decode()
    entries = [line.split() for line in checksums.splitlines()]
    expected = next((entry[0] for entry in entries if len(entry) == 2 and entry[1] == relative), None)
    if expected is None:
        raise SystemExit("Mozilla's checksum list does not include this build. Retry with the latest ESR.")
    destination.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=destination) as temporary:
        archive = Path(temporary) / "firefox.tar.xz"
        print(f"Downloading Firefox ESR {version} from Mozilla…", flush=True)
        with urlopen(base + relative, timeout=30) as response, archive.open("wb") as output:
            shutil.copyfileobj(response, output)
        with archive.open("rb") as downloaded:
            digest = hashlib.file_digest(downloaded, "sha256").hexdigest()
        if digest != expected:
            raise SystemExit("Firefox checksum mismatch; installation aborted.")
        with tarfile.open(archive) as tar:
            tar.extractall(temporary, filter="data")
        distribution = Path(temporary) / "firefox" / "distribution"
        distribution.mkdir(exist_ok=True)
        shutil.copyfile(ROOT / "policies.json", distribution / "policies.json")
        staged_version = Path(temporary) / "firefox-version.txt"
        staged_version.write_text(version + "\n")
        if backup.exists():
            shutil.rmtree(backup)
        if target.exists():
            target.rename(backup)
        installed_new = False
        try:
            (Path(temporary) / "firefox").rename(target)
            installed_new = True
            staged_version.replace(destination / "firefox-version.txt")
        except (OSError, KeyboardInterrupt):
            if installed_new:
                shutil.rmtree(target)
            if backup.exists():
                backup.rename(target)
            raise
        if backup.exists():
            shutil.rmtree(backup)
    print(f"Installed and SHA-256 verified: {target}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--destination", type=Path, default=ROOT / "runtime")
    parser.add_argument("--version")
    args = parser.parse_args()
    destination = args.destination.resolve()
    try:
        destination.mkdir(parents=True, exist_ok=True)
        with (destination / ".install.lock").open("a") as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                raise SystemExit("Another installation is running. Wait until it finishes.")
            install(destination, args.version)
    except (URLError, TimeoutError):
        raise SystemExit("Could not download Firefox. Check the network connection and retry; the existing installation is retained.")
    except (OSError, tarfile.TarError):
        raise SystemExit("Could not install Firefox. Check the destination's permissions and free space, then retry.")
    except (json.JSONDecodeError, UnicodeError):
        raise SystemExit("Could not read Mozilla's release information. Retry later or specify a release with --version.")
