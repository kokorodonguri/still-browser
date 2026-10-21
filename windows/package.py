#!/usr/bin/env python3
"""Build Still's Windows installers: Mozilla's Firefox ESR plus Still's launcher.

    python3 windows/package.py                     # x64 and arm64, latest Firefox ESR
    python3 windows/package.py --arch x64 --firefox 153.5.0esr
    python3 windows/package.py --arch x64 --firefox 153.5.0esr --gecko <zip>

Firefox comes unmodified from Mozilla: the full update package (complete.mar)
for Windows, checked against Mozilla's SHA256SUMS. With --gecko it is Still's
own build instead (windows/gecko/build.py), which does not update itself. The launcher is built with
cargo-xwin and the installer with makensis (see README.md). Each installer is
signed with the update key, as the Tauri build was:
TAURI_SIGNING_PRIVATE_KEY (or ~/.tauri/still-updater.key), password in
TAURI_SIGNING_PRIVATE_KEY_PASSWORD (default empty).

Writes artifacts/Still_<version>_<arch>-setup.exe, its .sig, and the release
record <installer>.release.json that binds the signature to the installer's
size and SHA-256 (see release_set.py). Each installer is built and signed in a
temporary directory inside artifacts/ and committed record-last, so
site/build.py never publishes a signature with the wrong installer, even while
this script runs.

    python3 windows/package.py --seal   # record existing installer + .sig pairs
"""
import argparse
import configparser
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import tempfile
from urllib.request import urlopen
import zipfile

WINDOWS = Path(__file__).resolve().parent
ROOT = WINDOWS.parent
ARTIFACTS = ROOT / "artifacts"
CACHE = WINDOWS / "cache"
BUILD = WINDOWS / "build"
LAUNCHER = WINDOWS / "launcher"
# Also works when this script is loaded with spec_from_file_location by tools/tests.
_mar_spec = importlib.util.spec_from_file_location("still_windows_mar", WINDOWS / "mar.py")
_mar = importlib.util.module_from_spec(_mar_spec)
_mar_spec.loader.exec_module(_mar)
_set_spec = importlib.util.spec_from_file_location("still_windows_release_set", WINDOWS / "release_set.py")
release_set = importlib.util.module_from_spec(_set_spec)
_set_spec.loader.exec_module(release_set)
ARCHES = {
    # arch: (Mozilla platform, Rust target)
    "x64": ("win64", "x86_64-pc-windows-msvc"),
    "arm64": ("win64-aarch64", "aarch64-pc-windows-msvc"),
}


def version() -> str:
    """Still's version: the launcher's, which the update check compares."""
    text = (LAUNCHER / "Cargo.toml").read_text()
    return re.search(r'^version = "([^"]+)"', text, re.M)[1]


def latest_esr() -> str:
    with urlopen("https://product-details.mozilla.org/1.0/firefox_versions.json", timeout=30) as response:
        release = json.load(response).get("FIREFOX_ESR")
    if not isinstance(release, str) or not re.fullmatch(r"\d+\.\d+(?:\.\d+)?esr", release):
        raise SystemExit("Mozilla's version list has no ESR release. Retry, or pass --firefox 153.5.0esr.")
    return release


def download(firefox: str, platform: str) -> Path:
    """The verified complete.mar, from the cache when it is already there."""
    relative = f"update/{platform}/ja/firefox-{firefox}.complete.mar"
    base = f"https://archive.mozilla.org/pub/firefox/releases/{firefox}/"
    with urlopen(base + "SHA256SUMS", timeout=30) as response:
        sums = dict(reversed(line.split(None, 1)) for line in response.read().decode().splitlines() if line.strip())
    expected = sums.get(relative)
    if expected is None:
        raise SystemExit(f"Mozilla's checksum list does not include {relative}.")
    target = CACHE / f"firefox-{firefox}-{platform}.complete.mar"
    if not (target.exists() and sha256(target) == expected):
        CACHE.mkdir(exist_ok=True)
        partial = target.with_suffix(".part")
        print(f"Downloading Firefox {firefox} ({platform}) from Mozilla…", flush=True)
        with urlopen(base + relative, timeout=60) as response, partial.open("wb") as output:
            shutil.copyfileobj(response, output)
        if sha256(partial) != expected:
            partial.unlink()
            raise SystemExit("Firefox checksum mismatch; nothing was packaged.")
        partial.replace(target)
    return target


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def extract_mar(mar: Path, destination: Path):
    """Unpacks a Mozilla update archive (MAR): an index of xz-compressed files."""
    try:
        _mar.extract_mar(mar, destination)
    except _mar.MarError as error:
        raise SystemExit(f"{mar.name}: {error}") from error


def extract_gecko(archive: Path, destination: Path):
    """Still's own Firefox build: the firefox/ folder of mach's package zip."""
    with zipfile.ZipFile(archive) as bundle:
        for entry in bundle.infolist():
            parts = Path(entry.filename).parts
            if parts[0] != "firefox" or ".." in parts:
                raise SystemExit(f"{archive.name}: unexpected entry {entry.filename}")
            if len(parts) == 1 or entry.is_dir():
                continue
            target = destination.joinpath(*parts[1:])
            target.parent.mkdir(parents=True, exist_ok=True)
            with bundle.open(entry) as source, target.open("wb") as output:
                shutil.copyfileobj(source, output)


def check_machine(executable: Path, platform: str):
    """Check the PE header rather than trusting an archive's filename."""
    expected = {"win64": 0x8664, "win64-aarch64": 0xAA64}[platform]
    try:
        with executable.open("rb") as file:
            if file.read(2) != b"MZ":
                raise ValueError("missing DOS header")
            file.seek(0x3C)
            offset = struct.unpack("<I", file.read(4))[0]
            if offset < 64:
                raise ValueError("invalid PE offset")
            file.seek(offset)
            if file.read(4) != b"PE\0\0":
                raise ValueError("missing PE signature")
            machine = struct.unpack("<H", file.read(2))[0]
            if machine != expected:
                raise ValueError(f"machine 0x{machine:04x}, expected 0x{expected:04x}")
    except (OSError, ValueError, struct.error) as error:
        raise SystemExit(f"{executable.name}: not a {platform} executable ({error}).") from error


def stage_firefox(firefox: str, platform: str, destination: Path, gecko: Path | None = None) -> str:
    """Firefox with Still's additive settings and icons; returns its version."""
    if destination.exists():
        shutil.rmtree(destination)
    if gecko:
        extract_gecko(gecko, destination)
    else:
        extract_mar(download(firefox, platform), destination)
    for name in ("updatev2.manifest", "updatev3.manifest"):
        (destination / name).unlink(missing_ok=True)
    if not (destination / "firefox.exe").exists():
        raise SystemExit("The update archive did not contain firefox.exe.")
    if gecko:
        check_machine(destination / "firefox.exe", platform)
        # Gecko lives in xul.dll; reject mixed executable/engine packages too.
        check_machine(destination / "xul.dll", platform)
    application = configparser.ConfigParser()
    application.read(destination / "application.ini", encoding="utf-8")
    if gecko and application["App"]["Version"] != firefox.removesuffix("esr"):
        raise SystemExit(f"{gecko.name} is Firefox {application['App']['Version']}, not {firefox}.")
    if gecko and (destination / "updater.exe").exists():
        # The installer distinguishes our runtime from Mozilla's by this file.
        raise SystemExit(f"{gecko.name} contains updater.exe; rebuild with --disable-updater.")
    policies = json.loads((ROOT / "linux" / "policies.json").read_text())
    # Windows only: Firefox's background task that asks to become the default browser.
    policies["policies"]["DisableDefaultBrowserAgent"] = True
    if gecko:
        # Mozilla's updates would replace Still's modified Firefox; Still's installer updates it.
        policies["policies"]["DisableAppUpdate"] = True
    (destination / "distribution").mkdir(exist_ok=True)
    (destination / "distribution" / "policies.json").write_text(json.dumps(policies, indent=2) + "\n")
    shutil.copyfile(WINDOWS / "firefox" / "still.cfg", destination / "still.cfg")
    shutil.copyfile(WINDOWS / "firefox" / "defaults" / "pref" / "still-autoconfig.js",
                    destination / "defaults" / "pref" / "still-autoconfig.js")
    shutil.copytree(WINDOWS / "firefox" / "still", destination / "still")
    if not gecko:
        # Still's own build carries the icon in its branding.
        icons = destination / "browser" / "chrome" / "icons" / "default"
        icons.mkdir(parents=True, exist_ok=True)
        for name in ("main-window.ico", "default.ico"):
            shutil.copyfile(ROOT / "desktop" / "src-tauri" / "icons" / "icon.ico", icons / name)
    return application["App"]["Version"]


def build_launcher(target: str) -> Path:
    environment = dict(os.environ)
    llvm = Path.home() / ".local" / "llvm18" / "bin"
    if llvm.is_dir():
        environment["PATH"] = f"{llvm}{os.pathsep}{environment['PATH']}"
    subprocess.run(["cargo", "xwin", "build", "--release", "--target", target], cwd=LAUNCHER, env=environment, check=True)
    return LAUNCHER / "target" / target / "release" / "still.exe"


def makensis() -> str:
    found = shutil.which("makensis") or str(Path.home() / ".local" / "nsis" / "bin" / "makensis")
    if not Path(found).exists():
        raise SystemExit("makensis was not found. Install NSIS (see windows/README.md).")
    return found


def sign(installer: Path):
    key = os.environ.get("TAURI_SIGNING_PRIVATE_KEY")
    if not key:
        path = Path.home() / ".tauri" / "still-updater.key"
        if not path.exists():
            raise SystemExit("No update signing key: set TAURI_SIGNING_PRIVATE_KEY (see README.md).")
        key = path.read_text()
    tauri = ROOT / "desktop" / "ui" / "node_modules" / ".bin" / "tauri"
    if not tauri.exists():
        raise SystemExit("The Tauri CLI signs installers: run (cd desktop/ui && npm ci) first.")
    environment = dict(os.environ, TAURI_SIGNING_PRIVATE_KEY=key)
    environment.setdefault("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", "")
    subprocess.run([str(tauri), "signer", "sign", str(installer)], env=environment, check=True,
                   stdout=subprocess.DEVNULL)
    if not installer.with_name(installer.name + ".sig").exists():
        raise SystemExit(f"Signing {installer.name} wrote no .sig file.")


def package(arch: str, firefox: str, gecko: Path | None = None):
    platform, target = ARCHES[arch]
    still = version()
    stage = BUILD / arch
    firefox_version = stage_firefox(firefox, platform, stage / "firefox", gecko)
    # Still's own build replaces any other Firefox build, even of the same version.
    own_build = []
    if gecko:
        application = configparser.ConfigParser()
        application.read(stage / "firefox" / "application.ini", encoding="utf-8")
        own_build = [f"-DFIREFOX_BUILDID={application['App']['BuildID']}"]
    shutil.copyfile(build_launcher(target), stage / "still.exe")
    name = f"Still_{still}_{arch}-setup.exe"
    ARTIFACTS.mkdir(exist_ok=True)
    print(f"Building {name} (Firefox {firefox_version})…", flush=True)
    # Build and sign under a temporary directory on the artifacts filesystem;
    # only a complete, signed set is renamed into place (release_set.commit).
    with tempfile.TemporaryDirectory(dir=ARTIFACTS, prefix=f".{name}.") as work:
        built = Path(work) / name
        subprocess.run([makensis(), "-V2", "-INPUTCHARSET", "UTF8",
                        f"-DVERSION={still}", f"-DARCH={arch}", f"-DFIREFOX_VERSION={firefox_version}", *own_build,
                        f"-DSTAGE={stage}", f"-DOUTFILE={built}", f"-DICON={ROOT / 'desktop/src-tauri/icons/icon.ico'}",
                        str(WINDOWS / "installer.nsi")], check=True)
        sign(built)
        release_set.commit(built, ARTIFACTS, release_set.updater_public_key(ROOT))
    print(ARTIFACTS / name)


def seal(artifacts: Path, still: str, public_key: str) -> list[dict]:
    """Records existing installer + .sig pairs of version `still` (built before records existed)."""
    sealed = []
    for arch in ARCHES:
        installer = artifacts / f"Still_{still}_{arch}-setup.exe"
        if installer.exists() and release_set.signature_path(installer).exists():
            try:
                sealed.append(release_set.seal(installer, public_key))
            except release_set.SignatureError as error:
                raise SystemExit(f"Not sealed: {error}") from error
    return sealed


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--arch", choices=[*ARCHES, "all"], default="all")
    parser.add_argument("--firefox", help="Firefox ESR release, such as 153.5.0esr (default: Mozilla's latest ESR)")
    parser.add_argument("--gecko", type=Path,
                        help="Still's own Firefox build matching --arch (the zip printed by windows/gecko/build.py)")
    parser.add_argument("--seal", action="store_true",
                        help="build nothing; write the release record for this version's existing installer + .sig pairs")
    args = parser.parse_args()
    if args.seal:
        sealed = seal(ARTIFACTS, version(), release_set.updater_public_key(ROOT))
        for item in sealed:
            print(f"sealed {item['file']} ({item['size']} bytes, sha256 {item['sha256']}"
                  + ("" if item["verified"] else "; Ed25519 not checked: python3 cryptography is missing") + ")")
        if not sealed:
            raise SystemExit(f"No signed installers of {version()} in {ARTIFACTS}.")
        return
    firefox = args.firefox or latest_esr()
    if not re.fullmatch(r"\d+\.\d+(?:\.\d+)?esr", firefox):
        raise SystemExit("--firefox takes an ESR release such as 153.5.0esr.")
    if args.gecko and args.arch == "all":
        raise SystemExit("--gecko requires one architecture: pass --arch x64 or --arch arm64.")
    for arch in ARCHES if args.arch == "all" else [args.arch]:
        package(arch, firefox, args.gecko)


if __name__ == "__main__":
    main()
