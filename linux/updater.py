#!/usr/bin/env python3
"""Update a packaged Still from the download site.

A check reads update.json from UPDATE_URL. A newer archive is downloaded,
checked against the manifest's SHA-256 and size, and unpacked next to the
installation as "<folder>.update". The launcher swaps it in at the next start,
before Firefox runs, so a running browser never has its files replaced.

Only packaged copies update themselves: linux/package.py adds release.json, and
a source checkout without it is never touched.

    python3 updater.py --background   # what the launcher starts: check, then notify
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
from urllib.error import URLError
from urllib.parse import urljoin
from urllib.request import urlopen
import update_swap

ROOT = Path(__file__).resolve().parent
UPDATE_URL = os.environ.get("STILL_UPDATE_URL", "https://still.dongurihub.com/update.json")
STAGED = ROOT.with_name(ROOT.name + ".update")
PREVIOUS = ROOT.with_name(ROOT.name + ".previous")
# A larger download than this is not a Still archive.
SIZE_LIMIT = 1 << 30


class UpdateError(Exception):
    """A failed check or download, worded for the user."""


def version_key(version: str) -> tuple[int, ...]:
    """'0.10.2' > '0.9.9' and '0.2' == '0.2.0'. Trailing letters ('140.17.0esr') are ignored."""
    parts = [int(re.match(r"\d*", part)[0] or 0) for part in version.split(".")]
    while parts and parts[-1] == 0:
        parts.pop()
    return tuple(parts)


def release_version(folder: Path) -> str | None:
    try:
        version = json.loads((folder / "release.json").read_text()).get("version")
    except (OSError, ValueError, AttributeError):
        return None
    return version if isinstance(version, str) and version else None


def firefox_version(folder: Path) -> str:
    try:
        for line in (folder / "runtime" / "firefox" / "application.ini").read_text().splitlines():
            if line.startswith("Version="):
                return line.split("=", 1)[1].strip()
    except OSError:
        pass
    return "0"


def firefox_running(folder: Path) -> bool:
    runtime = (folder / "runtime" / "firefox").resolve()
    for executable in Path("/proc").glob("[0-9]*/exe"):
        try:
            if executable.resolve(strict=True).is_relative_to(runtime):
                return True
        except (OSError, RuntimeError):
            continue
    return False


def check() -> dict | None:
    """The manifest entry when a newer version is available, else None."""
    installed = release_version(ROOT)
    if installed is None:
        raise UpdateError("ソースから起動しているため、アップデートは使えません。")
    try:
        with urlopen(UPDATE_URL, timeout=10) as response:
            manifest = json.loads(response.read(1 << 20))
    except (URLError, TimeoutError, OSError):
        raise UpdateError("アップデートを確認できませんでした。ネットワークに接続しているか確認してください。")
    except ValueError:
        raise UpdateError("アップデートの情報を読み取れませんでした。")
    entry = manifest.get("linux") if isinstance(manifest, dict) else None
    if not isinstance(entry, dict) or not all(isinstance(entry.get(k), t) for k, t in (("version", str), ("url", str), ("sha256", str), ("size", int))):
        return None
    if version_key(entry["version"]) <= version_key(installed):
        return None
    if STAGED.exists() and release_version(STAGED) == entry["version"]:
        entry = {**entry, "staged": True}
    return entry


def download(entry: dict) -> None:
    """Downloads, verifies and unpacks the archive into STAGED."""
    try:
        with update_swap.locked(ROOT):
            _download(entry)
    except update_swap.BusyError:
        raise UpdateError("別のアップデートを処理しています。完了してから、もう一度お試しください。")


def _download(entry: dict) -> None:
    if entry.get("staged"):
        return
    if not 0 < entry["size"] <= SIZE_LIMIT:
        raise UpdateError("アップデートの情報が正しくありません。")
    url = urljoin(UPDATE_URL, entry["url"])
    try:
        with tempfile.TemporaryDirectory(dir=ROOT.parent, prefix=f".{ROOT.name}.download-") as temporary:
            archive = Path(temporary) / "update.tar.gz"
            digest = hashlib.sha256()
            received = 0
            with urlopen(url, timeout=30) as response, archive.open("wb") as output:
                while chunk := response.read(1 << 20):
                    received += len(chunk)
                    if received > entry["size"]:
                        break
                    digest.update(chunk)
                    output.write(chunk)
            if received != entry["size"] or digest.hexdigest() != entry["sha256"].lower():
                raise UpdateError("ダウンロードしたファイルが壊れています。もう一度お試しください。")
            unpacked = Path(temporary) / "unpacked"
            with tarfile.open(archive, "r:gz") as tar:
                tar.extractall(unpacked, filter="data")
            folders = list(unpacked.iterdir())
            if len(folders) != 1 or not (folders[0] / "still").is_file() or release_version(folders[0]) != entry["version"]:
                raise UpdateError("アップデートのファイルが正しくありません。")
            # A result from an earlier check must not replace a newer download
            # or installation committed while that check was in flight.
            installed = release_version(ROOT)
            pending = release_version(STAGED)
            if installed is None:
                raise UpdateError("ソースから起動しているため、アップデートは使えません。")
            if version_key(entry["version"]) <= version_key(installed):
                return
            if pending and version_key(entry["version"]) <= version_key(pending):
                return
            if STAGED.exists():
                shutil.rmtree(STAGED)
            folders[0].rename(STAGED)
    except (URLError, TimeoutError):
        raise UpdateError("アップデートをダウンロードできませんでした。ネットワークを確認してから、もう一度お試しください。")
    except (OSError, tarfile.TarError):
        raise UpdateError(f"アップデートを保存できませんでした。{ROOT.parent} の書き込み権限と空き容量を確認してください。")


def apply_staged() -> bool:
    """Serialize recovery and activation; another updater never blocks launch."""
    if release_version(ROOT) is None and not PREVIOUS.exists():
        return False
    try:
        with update_swap.locked(ROOT):
            return _apply_staged()
    except update_swap.BusyError:
        return False


def _apply_staged() -> bool:
    """Swaps STAGED in for ROOT while Firefox is closed. True when files changed.

    The installed Firefox keeps updating itself, so it is kept unless the
    archive brings a newer one. Restore an uncommitted backup before retrying.
    """
    update_swap.recover(ROOT, STAGED, PREVIOUS)
    if not STAGED.exists() or release_version(ROOT) is None:
        return False
    staged = release_version(STAGED)
    if staged is None or not (STAGED / "still").is_file() or version_key(staged) <= version_key(release_version(ROOT)):
        shutil.rmtree(STAGED, ignore_errors=True)
        return False
    if firefox_running(ROOT):
        return False
    keep_runtime = version_key(firefox_version(ROOT)) >= version_key(firefox_version(STAGED)) and (ROOT / "runtime" / "firefox").exists()
    update_swap.activate(ROOT, STAGED, PREVIOUS, keep_runtime=keep_runtime)
    return True


def notify(title: str, body: str = "", action: str | None = None) -> bool:
    """Shows a desktop notification. With an action, waits and returns whether it was chosen."""
    if not shutil.which("notify-send"):
        return False
    command = ["notify-send", "--app-name=Still", "--icon=still-browser"]
    if action:
        command += [f"--action=update={action}", "--wait"]
    try:
        result = subprocess.run([*command, title, body], capture_output=True, text=True)
    except OSError:
        return False
    return result.stdout.strip() == "update"


def run(mode: str) -> int:
    """Checks, and when there is a newer version, downloads it after the user agrees.

    mode: "terminal" asks on stdin; "manual" (the desktop menu action) and
    "background" (each launch) ask with a notification. Only "background" stays
    quiet when there is nothing new or the check fails.
    """
    interactive = mode == "terminal"
    say = print if interactive else (lambda text: notify("Still", text))
    try:
        entry = check()
        if entry is None:
            if mode != "background":
                say(f"Still {release_version(ROOT)} は最新です。")
            return 0
        if interactive:
            answer = input(f"Still {entry['version']} があります（現在 {release_version(ROOT)}）。更新しますか？ [Y/n] ").strip().lower()
            if answer not in ("", "y", "yes"):
                return 0
        elif not notify(f"Still {entry['version']} があります", f"現在のバージョンは {release_version(ROOT)} です。", "更新"):
            return 0
        download(entry)
        if not firefox_running(ROOT) and apply_staged():
            say(f"Still {entry['version']} に更新しました。")
        else:
            say(f"Still {entry['version']} の準備ができました。Stillを終了して起動し直すと更新されます。")
        return 0
    except UpdateError as error:
        if mode != "background":
            say(str(error))
        return 1
    except OSError:
        if mode != "background":
            say(f"アップデートを適用できませんでした。{ROOT.parent} の書き込み権限と空き容量を確認してください。")
        return 1


def main_with(argv: list[str]):
    parser = argparse.ArgumentParser(description="Update Still from the download site")
    parser.add_argument("--background", action="store_true", help="check quietly; notify only when an update exists")
    parser.add_argument("--manual", action="store_true", help="report the result as notifications (desktop menu action)")
    args = parser.parse_args(argv)
    sys.exit(run("background" if args.background else "manual" if args.manual or not sys.stdin.isatty() else "terminal"))


if __name__ == "__main__":
    main_with(sys.argv[1:])
