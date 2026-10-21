#!/usr/bin/env python3
"""Build Still's Firefox for Windows x64 or ARM64 from Mozilla's ESR source.

    python3 windows/gecko/build.py --firefox 153.5.0esr
    python3 windows/package.py --arch x64 --firefox 153.5.0esr --gecko <printed zip>

The source tarball is checked against Mozilla's SHA256SUMS, unpacked under
~/still-gecko (or $STILL_GECKO_HOME), patched with windows/gecko/patches/ and
given Still's branding, then cross-compiled with mach (mozconfig). The build
is repacked in Japanese from the firefox-l10n revision that Mozilla shipped
with the release. Prints the Japanese zip for package.py --gecko.

Needs about 50 GB free and swap on a 7 GB machine (see windows/README.md).
Every ESR security release has to be built and shipped again: this Firefox
does not update itself.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
from urllib.request import urlopen

GECKO = Path(__file__).resolve().parent
ROOT = GECKO.parent.parent
HOME = Path(os.environ.get("STILL_GECKO_HOME", Path.home() / "still-gecko"))
PATCHES = sorted((GECKO / "patches").glob("*.patch"))
ARM64_FILES = GECKO / "arm64" / "files"
ARCHES = {
    "x64": ("x86_64-pc-windows-msvc", "win64"),
    "arm64": ("aarch64-pc-windows-msvc", "win64-aarch64"),
}
L10N_REPOSITORY = "https://github.com/mozilla-l10n/firefox-l10n.git"
LOCALE = "ja"


def run(*command, cwd=None, env=None):
    print("+", " ".join(str(part) for part in command), flush=True)
    subprocess.run([str(part) for part in command], cwd=cwd, env=env, check=True)


def download(firefox: str) -> Path:
    name = f"firefox-{firefox}.source.tar.xz"
    base = f"https://archive.mozilla.org/pub/firefox/releases/{firefox}/"
    with urlopen(base + "SHA256SUMS", timeout=30) as response:
        sums = response.read().decode()
    expected = next((line.split()[0] for line in sums.splitlines() if line.endswith(f" source/{name}")), None)
    if not expected:
        raise SystemExit(f"Mozilla's SHA256SUMS has no source/{name}.")
    target = HOME / name
    if not target.exists() or sha256(target) != expected:
        print(f"Downloading the Firefox {firefox} source from Mozilla…", flush=True)
        partial = target.with_suffix(".part")
        with urlopen(base + "source/" + name, timeout=60) as response, partial.open("wb") as output:
            shutil.copyfileobj(response, output, 1 << 20)
        if sha256(partial) != expected:
            partial.unlink()
            raise SystemExit(f"{name} does not match Mozilla's SHA256SUMS.")
        partial.rename(target)
    return target


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def patches(arch: str) -> list[Path]:
    return PATCHES + (sorted((GECKO / "arm64").glob("*.patch")) if arch == "arm64" else [])


def patches_digest(arch: str = "x64") -> str:
    digest = hashlib.sha256()
    for patch in patches(arch):
        digest.update(patch.name.encode() + b"\0" + patch.read_bytes())
    if arch == "arm64":
        for path in sorted(ARM64_FILES.rglob("*")):
            if path.is_file():
                digest.update(path.relative_to(ARM64_FILES).as_posix().encode() + b"\0" + path.read_bytes())
    return digest.hexdigest()


def prepare_source(firefox: str, arch: str = "x64") -> Path:
    """An unpacked, patched source tree; unpacked again when the patches change."""
    source = HOME / (f"firefox-{firefox}" + ("-arm64" if arch == "arm64" else ""))
    marker = source / ".still-patches"
    if marker.exists() and marker.read_text() == patches_digest(arch):
        return source
    if source.exists():
        shutil.rmtree(source)
    archive = download(firefox)
    print(f"Unpacking {archive.name}…", flush=True)
    unpacked = HOME / ".unpacking"
    if unpacked.exists():
        shutil.rmtree(unpacked)
    with tarfile.open(archive) as tar:
        tar.extractall(unpacked, filter="data")
    (top,) = unpacked.iterdir()
    top.rename(source)
    unpacked.rmdir()
    for patch in patches(arch):
        try:
            run("git", "apply", "--check", patch, cwd=source)
        except subprocess.CalledProcessError:
            raise SystemExit(f"{patch.name} does not apply to Firefox {firefox}: update it for this release.")
        run("git", "apply", patch, cwd=source)
    if arch == "arm64":
        shutil.copytree(ARM64_FILES, source, dirs_exist_ok=True)
    brand(source)
    marker.write_text(patches_digest(arch))
    return source


def brand(source: Path):
    """browser/branding/still: Mozilla's unofficial branding named Still, with Still's icon."""
    branding = source / "browser" / "branding"
    still = branding / "still"
    shutil.copytree(branding / "unofficial", still)
    replace(still / "configure.sh", "MOZ_APP_DISPLAYNAME=Nightly", "MOZ_APP_DISPLAYNAME=Still")
    replace(still / "configure.sh", "MOZ_MACBUNDLE_ID=nightlyunofficial", "MOZ_MACBUNDLE_ID=still")
    for name in ("brand.properties", "brand.ftl"):
        path = still / "locales" / "en-US" / name
        path.write_text(path.read_text().replace("Nightly", "Still"))
    nsi = still / "branding.nsi"
    nsi.write_text(nsi.read_text().replace("Mozilla Developer Preview", "Still"))
    icon = ROOT / "desktop" / "src-tauri" / "icons" / "icon.ico"
    for name in ("firefox.ico", "firefox64.ico"):
        shutil.copyfile(icon, still / name)


def replace(path: Path, old: str, new: str):
    text = path.read_text()
    if old not in text:
        raise SystemExit(f"{path}: {old!r} is missing; Mozilla's branding changed.")
    path.write_text(text.replace(old, new))


def l10n(source: Path) -> Path:
    """firefox-l10n at the revision Mozilla shipped with this release, only LOCALE checked out."""
    revision = json.loads((source / "browser" / "locales" / "l10n-changesets.json").read_text())[LOCALE]["revision"]
    checkout = HOME / "firefox-l10n"
    if not (checkout / ".git").exists():
        run("git", "clone", "--filter=blob:none", "--no-checkout", L10N_REPOSITORY, checkout)
        run("git", "sparse-checkout", "set", LOCALE, cwd=checkout)
    run("git", "fetch", "--filter=blob:none", "origin", revision, cwd=checkout)
    run("git", "checkout", "--detach", revision, cwd=checkout)
    return checkout


def mozconfig(source: Path, objdir: Path, l10n_base: Path, safebrowsing: Path | None,
              arch: str = "x64", tests: bool = False) -> Path:
    text = (GECKO / "mozconfig").read_text()
    text = text.replace("--target=x86_64-pc-windows-msvc", f"--target={ARCHES[arch][0]}")
    if tests:
        text = text.replace("ac_add_options --disable-tests\n", "")
    text += f"\nmk_add_options MOZ_OBJDIR={objdir}\nac_add_options --with-l10n-base={l10n_base}\n"
    if safebrowsing:
        text += f"ac_add_options --with-google-safebrowsing-api-keyfile={safebrowsing.resolve()}\n"
    path = source / f"mozconfig.still-{arch}"
    path.write_text(text)
    return path


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--firefox", required=True, help="Firefox ESR release, such as 153.5.0esr")
    parser.add_argument("--arch", choices=ARCHES, default="x64")
    parser.add_argument("--enable-tests", action="store_true", help="include Gecko's browser tests")
    parser.add_argument("--safebrowsing-key", type=Path,
                        help="Google Safe Browsing API key file (required for ARM64; without it, x64 lists are not updated)")
    args = parser.parse_args()
    if not re.fullmatch(r"\d+\.\d+(?:\.\d+)?esr", args.firefox):
        raise SystemExit("--firefox takes an ESR release such as 153.5.0esr.")
    if args.arch == "arm64" and not args.safebrowsing_key:
        raise SystemExit("ARM64 builds require --safebrowsing-key to preserve phishing and malware list updates.")
    if args.safebrowsing_key and (not args.safebrowsing_key.is_file() or not args.safebrowsing_key.read_bytes().strip()):
        raise SystemExit("--safebrowsing-key must point to a non-empty API key file.")
    HOME.mkdir(parents=True, exist_ok=True)
    source = prepare_source(args.firefox, args.arch)
    objdir = HOME / f"obj-{args.firefox}-{args.arch}"
    environment = dict(os.environ, MOZCONFIG=str(mozconfig(source, objdir, l10n(source), args.safebrowsing_key,
                                                        args.arch, args.enable_tests)),
                       MOZBUILD_STATE_PATH=str(HOME / "mozbuild"))
    mach = source / "mach"
    run(mach, "--no-interactive", "build", cwd=source, env=environment)
    run(mach, "--no-interactive", "package", cwd=source, env=environment)
    run(mach, "--no-interactive", "build", f"installers-{LOCALE}", cwd=source, env=environment)
    built = sorted((objdir / "dist").glob(f"firefox-*.{LOCALE}.{ARCHES[args.arch][1]}.zip"))
    if not built:
        raise SystemExit(f"No {LOCALE} package in {objdir / 'dist'}.")
    print(built[-1])


if __name__ == "__main__":
    main()
