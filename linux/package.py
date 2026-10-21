#!/usr/bin/env python3
"""Build a portable Linux archive from the verified local runtime."""
import io
import json
import os
from pathlib import Path
import re
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parent
ARCHIVE_ROOT = "StillBrowser-linux-x64"
# Include runtime dependencies explicitly. Tests, backups, and unrelated local
# files must not become part of a user's installation.
RELEASE_PATHS = (
    "still", "updater.py", "update_swap.py", "newtab_image.py",
    "profile_migration.py", "profile", "policies.json", "install.py",
    "install-desktop.py", "still-browser.desktop", "README.md", "runtime/firefox",
)


def include(info):
    parts = Path(info.name).parts
    if "__pycache__" in parts or ".install.lock" in parts:
        return None
    # Firefox's own update downloads, and an update it has staged but not yet
    # applied (a second copy of Firefox), belong to this machine only.
    if parts[:3] == (ARCHIVE_ROOT, "runtime", "firefox") and parts[3:4] in (("updates",), ("updated",)):
        return None
    return info


def package(source: Path, output: Path, version: str) -> None:
    """Publish only a closed, synced archive; preserve the old one on failure."""
    if not (source / "runtime/firefox/firefox").is_file():
        raise SystemExit("Install the Mozilla runtime first: python3 linux/install.py")
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=output.parent, prefix=f".{output.name}.", delete=False) as stream:
            temporary = Path(stream.name)
            with tarfile.open(fileobj=stream, mode="w:gz") as archive:
                # First so site/build.py can read the version without scanning Firefox.
                release = json.dumps({"version": version}).encode()
                info = tarfile.TarInfo(f"{ARCHIVE_ROOT}/release.json")
                info.size, info.mode = len(release), 0o644
                archive.addfile(info, io.BytesIO(release))
                for name in RELEASE_PATHS:
                    archive.add(source / name, arcname=f"{ARCHIVE_ROOT}/{name}", filter=include)
            stream.flush()
            os.fsync(stream.fileno())
            os.fchmod(stream.fileno(), 0o644)
        # Replace the inode instead of truncating any existing published hard link.
        temporary.replace(output)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def main() -> None:
    # Windows sets the version for every platform; raise it there per release.
    version = re.search(r'^version = "([^"]+)"', (ROOT.parent / "windows/launcher/Cargo.toml").read_text(), re.M)[1]
    output = ROOT.parent / f"artifacts/{ARCHIVE_ROOT}.tar.gz"
    package(ROOT, output, version)
    print(output)


if __name__ == "__main__":
    main()
