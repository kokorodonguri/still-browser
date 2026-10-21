#!/usr/bin/env python3
"""Build the download site for Still.

The apps check site/public/update.json (Android, Linux) and
site/public/update/windows.json (the Tauri updater format, read by the Windows
launcher and by the earlier Tauri build) for new versions. One build publishes
one release generation from artifacts/:

1. Snapshot. Each release file is copied once into a private staging
   directory. Size, SHA-256, embedded versions and the Windows signature all
   come from that copy. A Windows installer is paired with the signature in its
   release record (windows/release_set.py) only when the record's size and
   SHA-256 match the snapshot; a rebuild racing this one causes a retry.
2. Immutable files. Snapshots go to dl/<version>/<sha256[:16]>/<name>, with
   <name>.sig beside a Windows installer. A published path is never rewritten:
   the same bytes keep the same URL and other bytes get another one.
3. Convenience links. dl/<name> is replaced by a hard link to the newest file,
   for people. Manifests never name these; they change with every release.
4. Manifests. update/windows.json, update.json and index.html are all written
   to synced temporary files, then each is renamed over its predecessor, after
   every file they name is in place. A failure before the renames leaves the
   previous manifests and the files they name untouched; a failed rename can
   leave some manifests new and some old, but each names only published files.
   update.json URLs are relative (both apps resolve them against update.json);
   update/windows.json URLs are absolute, as the Tauri format requires.
5. Retention. Immutable files of older generations are removed. Kept: the new
   generation, the previous distinct one, and every file that the replaced
   manifests named (clients may still be downloading them).

One build runs at a time (a lock in site/public/). Never write a published file
in place: immutable files and dl/<name> may share an inode.

    python3 site/build.py            # build only
    python3 site/build.py --serve    # build, then serve on port 8424 (public: https://still.dongurihub.com/)
    python3 site/build.py --artifacts DIR --out DIR --public-url http://127.0.0.1:8000/ --public-key KEY   # test site
"""
import argparse
from contextlib import contextmanager
from datetime import datetime, timezone
import html
import http.server
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile
import time
from urllib.parse import urljoin

try:
    import fcntl
except ImportError:  # Not POSIX: builds are not serialized.
    fcntl = None

ROOT = Path(__file__).resolve().parent.parent
ARTIFACTS = ROOT / "artifacts"
SITE = Path(__file__).resolve().parent
OUT = SITE / "public"
# The address the apps use to reach this site (see UPDATE_URL in each app).
PUBLIC_URL = "https://still.dongurihub.com/"
# The Windows installers carry the version in their file names (windows/package.py).
VERSION = re.search(r'^version = "([^"]+)"', (ROOT / "windows" / "launcher" / "Cargo.toml").read_text(encoding="utf-8"), re.M)[1]
# The updater key's public half; None reads desktop/src-tauri/tauri.conf.json.
PUBLIC_KEY = None

_spec = importlib.util.spec_from_file_location("still_windows_release_set", ROOT / "windows" / "release_set.py")
release_set = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(release_set)

LOCK = ".build.lock"
GENERATIONS = ".generations.json"
STAGING_PREFIX = ".staging-"
VERSION_DIR = re.compile(r"[0-9A-Za-z][0-9A-Za-z.+_-]{0,63}")
DIGEST_DIR = re.compile(r"[0-9a-f]{16}")
WINDOWS_TARGETS = {"x64": "windows-x86_64", "arm64": "windows-aarch64"}


def releases(version: str) -> list[tuple[str, str, str, str]]:
    """(platform id, file in artifacts/, label, note); the first file of a platform is its primary button."""
    return [
        ("windows", f"Still_{version}_x64-setup.exe", "インストーラー（x64）", "ほとんどのWindows PCはこちら。Firefox ESRを同梱。管理者権限は不要です"),
        ("windows", f"Still_{version}_arm64-setup.exe", "インストーラー（ARM64）", "Snapdragon搭載PCなど"),
        ("android", "StillBrowser-Android.apk", "APK（arm64）", "Android 10以上、64ビットARMの端末"),
        ("linux", "StillBrowser-linux-x64.tar.gz", "tar.gz（x86_64）", "Firefox ESRを同梱。展開して ./still を起動"),
    ]


RELEASES = releases(VERSION)

PLATFORMS = {
    "windows": ("Windows", "Windows 10 / 11", "Gecko（Firefox ESR）",
                "コード署名をしていないため、SmartScreenの警告が出たら「詳細情報」→「実行」を選んでください。以前のStillを使っている場合は、ブックマークと履歴を引き継ぎます。初回はFirefoxの利用規約の確認が出ます。"),
    "android": ("Android", "Android 10 以上（arm64）", "GeckoView（Firefox）",
                "ダウンロード後にAPKを開いてインストールします。「提供元不明のアプリ」の許可を求められたら、ダウンロードに使ったアプリに許可を与えてください。"),
    "linux": ("Linux", "x86_64 / Ubuntu 24.04 など", "Gecko（Firefox ESR）",
              "tar xzf StillBrowser-linux-x64.tar.gz で展開し、中の ./still を起動します。アプリメニューへの登録は python3 linux/install-desktop.py で行えます。"),
}

ICONS = {
    "windows": '<path d="M3 5.5 10 4.5v7H3zM11 4.4 21 3v8.5H11zM3 12.5h7v7l-7-1zM11 12.5h10V21l-10-1.4z"/>',
    "android": '<path d="M7.2 9.5h9.6a1 1 0 0 1 1 1v6.8a1 1 0 0 1-1 1h-1v2.4a1.1 1.1 0 0 1-2.2 0v-2.4h-3.2v2.4a1.1 1.1 0 0 1-2.2 0v-2.4h-1a1 1 0 0 1-1-1v-6.8a1 1 0 0 1 1-1zM4.6 9.6a1.1 1.1 0 0 1 1.1 1.1v4.8a1.1 1.1 0 0 1-2.2 0v-4.8a1.1 1.1 0 0 1 1.1-1.1zm14.8 0a1.1 1.1 0 0 1 1.1 1.1v4.8a1.1 1.1 0 0 1-2.2 0v-4.8a1.1 1.1 0 0 1 1.1-1.1zM7.3 8.6a4.8 4.8 0 0 1 9.4 0zm2.4-2.1a.6.6 0 1 0 0-1.2.6.6 0 0 0 0 1.2zm4.6 0a.6.6 0 1 0 0-1.2.6.6 0 0 0 0 1.2zM8.6 2.6l1.1 1.9m5.7-1.9-1.1 1.9" stroke="currentColor" stroke-width=".9" stroke-linecap="round"/>',
    "linux": '<path d="M12 2.5c-2.3 0-3.6 1.9-3.6 4.4 0 1.3.3 2.2-.6 3.6-1 1.5-2.6 3.5-2.6 6 0 .8.2 1.5.5 2.1-.8.5-1.6 1-1.4 1.7.3.9 2.4.6 3.6 1.2 1 .5 2 .2 2.5-.5h3.2c.5.7 1.5 1 2.5.5 1.2-.6 3.3-.3 3.6-1.2.2-.7-.6-1.2-1.4-1.7.3-.6.5-1.3.5-2.1 0-2.5-1.6-4.5-2.6-6-.9-1.4-.6-2.3-.6-3.6 0-2.5-1.3-4.4-3.6-4.4zm-1.3 3.3a.8.8 0 1 1 0 1.6.8.8 0 0 1 0-1.6zm2.6 0a.8.8 0 1 1 0 1.6.8.8 0 0 1 0-1.6zM12 8.2c.8 0 1.6.4 1.6.8s-.8.9-1.6.9-1.6-.5-1.6-.9.8-.8 1.6-.8z"/>',
}

SCREENSHOTS = {
    "hero-light.png": ARTIFACTS / "desktop-checks" / "light-1440-compact-sidebar.png",
    "hero-dark.png": ARTIFACTS / "desktop-checks" / "dark-1440-compact-sidebar.png",
}


class PublishError(Exception):
    """A release set that must not be published; the previous generation stays live."""


def human_size(n: int) -> str:
    for unit in ("B", "KB", "MB", "GB"):
        if n < 1024 or unit == "GB":
            return f"{n:.0f} {unit}" if unit in ("B", "KB") else f"{n:.1f} {unit}"
        n /= 1024
    return ""


def sha256(path: Path) -> str:
    return release_set.file_digest(path)[1]


def place(src: Path, dst: Path) -> None:
    """Copy `src` to `dst` as an independent, synced snapshot; a failure keeps the old `dst`."""
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=dst.parent, prefix=f".{dst.name}.", delete=False) as output:
            temporary = Path(output.name)
        shutil.copy2(src, temporary)
        release_set.fsync_file(temporary)
        temporary.replace(dst)
        release_set.fsync_dir(dst.parent)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def release_path(version: str, digest: str, name: str) -> str:
    """The immutable, site-relative path of a published file."""
    return f"dl/{version}/{digest[:16]}/{name}"


@contextmanager
def build_lock(out: Path):
    """Serializes builds; a concurrent cleanup could otherwise remove another build's files."""
    out.mkdir(parents=True, exist_ok=True)
    if fcntl is None:
        yield
        return
    with open(out / LOCK, "a") as handle:
        try:
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            print("waiting for another site build to finish…", flush=True)
            fcntl.flock(handle, fcntl.LOCK_EX)
        yield


def remove_leftovers(out: Path) -> None:
    """Temporary files of an interrupted build (only called while holding the lock)."""
    for path in out.glob(f"{STAGING_PREFIX}*"):
        shutil.rmtree(path, ignore_errors=True)
    temporary = {out: (".update.json.", ".index.html.", f"{GENERATIONS}."), out / "update": (".windows.json.",),
                 out / "dl": (".",), out / "img": (".",)}
    for folder, prefixes in temporary.items():
        if folder.is_dir():
            for path in folder.iterdir():
                if path.name.startswith(prefixes) and path.is_file():
                    path.unlink(missing_ok=True)


def snapshot_signed(src: Path, staged: Path, public_key: str, attempts: int, delay: float) -> tuple[int, str, str | None]:
    """(size, SHA-256, signature) of a Windows installer snapshot and the signature
    that belongs to exactly those bytes; None when the installer is unsigned."""
    problem = ""
    for attempt in range(attempts):
        if attempt:
            time.sleep(delay)
        place(src, staged)
        size, digest = release_set.file_digest(staged)
        try:
            record = release_set.read_record(src)
        except release_set.SignatureError as error:
            problem = str(error)
            continue
        if record is None:
            if not release_set.signature_path(src).exists():
                return size, digest, None
            problem = (f"{src.name} has a .sig but no release record: rebuild it with windows/package.py, "
                       "or run python3 windows/package.py --seal")
            continue
        if (record["size"], record["sha256"].lower()) != (size, digest):
            problem = f"{src.name} changed while it was read, or its release record is for other bytes"
            continue
        try:
            release_set.verify_signature(staged, record["signature"], public_key, src.name)
        except release_set.SignatureError as error:
            raise PublishError(f"{src.name}: {error}") from error
        return size, digest, record["signature"].strip()
    raise PublishError(problem + "; nothing was published")


def collect(artifacts: Path, staging: Path, version: str, public_key: str, attempts: int, delay: float) -> list[dict]:
    """Snapshots and metadata of the release files present in `artifacts`."""
    items = []
    for platform, name, label, note in releases(version):
        source = artifacts / name
        if not source.exists():
            print(f"skip (missing): {name}")
            continue
        staged = staging / name
        signature = None
        if platform == "windows":
            size, digest, signature = snapshot_signed(source, staged, public_key, attempts, delay)
        else:
            place(source, staged)
            size, digest = release_set.file_digest(staged)
        item = {"platform": platform, "name": name, "label": label, "note": note,
                "bytes": size, "size": human_size(size), "sha": digest, "staged": staged,
                "signature": signature, "embedded": None, "date": ""}
        if platform == "android" and (found := android_version(staged)):
            item["embedded"], item["code"], item["abis"] = found[0], found[1], apk_abis(staged)
        elif platform == "linux":
            item["embedded"] = linux_version(staged)
        # The version the file declares names its folder; the release version otherwise.
        folder = item["embedded"] if isinstance(item["embedded"], str) and VERSION_DIR.fullmatch(item["embedded"]) else version
        item["path"] = release_path(folder, digest, name)
        items.append(item)
    return items


def publish_file(out: Path, item: dict, public_key: str) -> None:
    """Moves a snapshot to its immutable path, keeping identical bytes already there."""
    target = out / item["path"]
    target.parent.mkdir(parents=True, exist_ok=True)
    release_set.fsync_dir(target.parent.parent)
    staged = item.pop("staged")
    if target.is_file() and release_set.file_digest(target) == (item["bytes"], item["sha"]):
        staged.unlink()
    else:
        if target.exists():
            print(f"repairing {item['path']}: its bytes did not match its address")
        os.replace(staged, target)
        release_set.fsync_dir(target.parent)
    item["date"] = datetime.fromtimestamp(target.stat().st_mtime).strftime("%Y-%m-%d")
    if item["signature"]:
        signature_file = release_set.signature_path(target)
        try:
            existing = signature_file.read_text(encoding="utf-8").strip()
            release_set.verify_signature(target, existing, public_key, item["name"])
        except (OSError, release_set.SignatureError):
            release_set.write_atomic(signature_file, item["signature"].encode())
        else:
            # A valid published signature of these bytes stays; the manifest names it.
            item["signature"] = existing


def link_fixed(out: Path, item: dict) -> None:
    """dl/<name>: the newest bytes for people. Replaced each release; never in a manifest."""
    target, fixed = out / item["path"], out / "dl" / item["name"]
    if fixed.exists() and os.path.samefile(target, fixed):
        return
    temporary = fixed.with_name(f".{fixed.name}.{os.getpid()}.link")
    temporary.unlink(missing_ok=True)
    try:
        try:
            os.link(target, temporary)
        except OSError:
            shutil.copy2(target, temporary)
            release_set.fsync_file(temporary)
        os.replace(temporary, fixed)
    finally:
        temporary.unlink(missing_ok=True)


def android_version(apk: Path) -> tuple[str, int] | None:
    """versionName and versionCode, read from the APK with the SDK's aapt2."""
    sdk = Path(os.environ.get("ANDROID_HOME", Path.home() / "Android" / "Sdk"))
    tools = sorted((sdk / "build-tools").glob("*/aapt2"))
    if not tools:
        return None
    out = subprocess.run([str(tools[-1]), "dump", "badging", str(apk)], capture_output=True, text=True).stdout
    match = re.search(r"versionCode='(\d+)' versionName='([^']+)'", out)
    return (match[2], int(match[1])) if match else None


def apk_abis(apk: Path) -> list[str]:
    """Processors the APK's native code runs on (lib/<abi>/); the app skips updates it cannot install."""
    import zipfile
    with zipfile.ZipFile(apk) as archive:
        return sorted({name.split("/")[1] for name in archive.namelist() if name.startswith("lib/") and name.count("/") >= 2})


def linux_version(archive: Path) -> str | None:
    """The version in release.json, which linux/package.py writes first into the archive."""
    with tarfile.open(archive, "r:gz") as tar:
        for member in tar:
            if member.name.endswith("/release.json") and member.name.count("/") == 1:
                return json.load(tar.extractfile(member)).get("version")
            if member.isfile():
                return None  # an archive from before release.json existed


def update_manifests(items: list[dict], version: str, public_url: str) -> tuple[dict, dict | None]:
    """update.json for Android and Linux; update/windows.json (Tauri updater format), or None."""
    by_name = {item["name"]: item for item in items}
    # Relative URLs: Android (URL(base, url)) and Linux (urljoin) resolve them against update.json.
    entry = lambda item: {"url": item["path"], "sha256": item["sha"], "size": item["bytes"]}
    manifest = {}
    apk = by_name.get("StillBrowser-Android.apk")
    if apk and apk.get("code") is not None:
        manifest["android"] = {"version": apk["embedded"], "code": apk["code"], **entry(apk), "abis": apk["abis"]}
    elif apk:
        print("update.json: Android left out (aapt2 not found in the Android SDK)")
    tgz = by_name.get("StillBrowser-linux-x64.tar.gz")
    if tgz and tgz["embedded"]:
        manifest["linux"] = {"version": tgz["embedded"], **entry(tgz)}
    elif tgz:
        print("update.json: Linux left out (the archive has no release.json; rebuild it with linux/package.py)")
    # Every platform takes Windows' version; a different one means a stale build.
    for platform, item in manifest.items():
        if item["version"] != version:
            print(f"update.json: {platform} is {item['version']}, not {version} (Windows); rebuild it")

    # Only signed updates are installed: each installer needs the release record
    # that windows/package.py writes with TAURI_SIGNING_PRIVATE_KEY (the Tauri key).
    platforms = {}
    for arch, target in WINDOWS_TARGETS.items():
        item = by_name.get(f"Still_{version}_{arch}-setup.exe")
        if item and item["signature"]:
            platforms[target] = {"signature": item["signature"], "url": f"{public_url}{item['path']}"}
    if not platforms:
        return manifest, None
    return manifest, {
        "version": version,
        "pub_date": datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
        "platforms": platforms,
    }


def referenced(out: Path, public_url: str) -> set[str]:
    """Site-relative paths under dl/ named by the manifests currently in `out`."""
    paths = set()
    base = urljoin(public_url, "update.json")
    def add(url):
        if isinstance(url, str):
            absolute = urljoin(base, url)
            if absolute.startswith(public_url):
                path = absolute[len(public_url):].split("#")[0].split("?")[0]
                if path.startswith("dl/"):
                    paths.add(path)
    for name in ("update.json", "update/windows.json"):
        try:
            manifest = json.loads((out / name).read_text(encoding="utf-8"))
        except (OSError, ValueError):
            continue
        if not isinstance(manifest, dict):
            continue
        entries = manifest.get("platforms", {}) if name == "update/windows.json" else manifest
        for entry in entries.values() if isinstance(entries, dict) else ():
            if isinstance(entry, dict):
                add(entry.get("url"))
    try:
        for href in re.findall(r'href="(dl/[^"]+)"', (out / "index.html").read_text(encoding="utf-8")):
            add(html.unescape(href))
    except OSError:
        pass
    return paths


def retain(out: Path, current: set[str], kept_before: set[str]) -> None:
    """Records the generation and removes immutable files nobody can still be fetching."""
    try:
        state = json.loads((out / GENERATIONS).read_text(encoding="utf-8"))
        recorded, previous = set(state.get("current", [])), set(state.get("previous", []))
    except (OSError, ValueError, AttributeError, TypeError):
        recorded, previous = set(), set()
    if recorded != current:
        previous = recorded
    release_set.write_atomic(out / GENERATIONS, (json.dumps(
        {"current": sorted(current), "previous": sorted(previous)}, indent=2) + "\n").encode())
    keep = {path.rsplit("/", 1)[0] for path in current | previous | kept_before}
    dl = out / "dl"
    names = {path.rsplit("/", 1)[1] for path in current}
    for entry in dl.iterdir():
        if entry.name.startswith("."):
            continue
        if entry.is_dir() and not entry.is_symlink():
            for folder in entry.iterdir():
                if folder.is_dir() and DIGEST_DIR.fullmatch(folder.name) and f"dl/{entry.name}/{folder.name}" not in keep:
                    shutil.rmtree(folder)
            try:
                entry.rmdir()  # only when no generation uses this version any more
            except OSError:
                pass
        elif entry.name not in names and f"dl/{entry.name}" not in kept_before:
            entry.unlink()


def replace_manifests(out: Path, contents: dict) -> None:
    """Writes every manifest to a synced temporary file first, then renames them
    in order with nothing that can fail in between but the renames themselves.
    None removes a manifest. Each manifest only names files already published."""
    prepared = {}
    try:
        for path, text in contents.items():
            if text is not None:
                prepared[path] = release_set.prepare(path, text.encode())
        for path, text in contents.items():
            if text is None:
                path.unlink(missing_ok=True)
            else:
                os.replace(prepared.pop(path), path)
            release_set.fsync_dir(path.parent)
    finally:
        for temporary in prepared.values():
            temporary.unlink(missing_ok=True)


def build(artifacts: Path | None = None, out: Path | None = None, *, version: str | None = None,
          public_url: str | None = None, screenshots: dict | None = None, public_key: str | None = None,
          attempts: int = 5, retry_delay: float = 1.0) -> dict:
    """Publishes one generation; see the module docstring. Returns the manifests written."""
    artifacts = ARTIFACTS if artifacts is None else artifacts
    out = OUT if out is None else out
    version = VERSION if version is None else version
    public_url = PUBLIC_URL if public_url is None else public_url
    screenshots = SCREENSHOTS if screenshots is None else screenshots
    public_key = public_key or PUBLIC_KEY or release_set.updater_public_key(ROOT)
    with build_lock(out):
        remove_leftovers(out)
        for folder in ("dl", "img", "update"):
            (out / folder).mkdir(exist_ok=True)
        live = referenced(out, public_url)
        staging = Path(tempfile.mkdtemp(dir=out, prefix=STAGING_PREFIX))
        try:
            items = collect(artifacts, staging, version, public_key, attempts, retry_delay)
            # Every file first; manifests only once all they name is in place.
            for item in items:
                publish_file(out, item, public_key)
            for item in items:
                link_fixed(out, item)
            for name, source in screenshots.items():
                if source.exists():
                    place(source, out / "img" / name)
            update, windows = update_manifests(items, version, public_url)
            replace_manifests(out, {
                out / "update" / "windows.json": json.dumps(windows, indent=2) + "\n" if windows else None,
                out / "update.json": json.dumps(update, indent=2) + "\n",
                out / "index.html": render(items, version),
            })
            if not windows:
                print("update/windows.json: left out (no signed installer; see README)")
        finally:
            shutil.rmtree(staging, ignore_errors=True)
        print(f"wrote {out / 'index.html'} ({len(items)} files)")
        print(f"wrote update.json ({', '.join(update) or 'empty'})"
              + (f", update/windows.json ({', '.join(windows['platforms'])})" if windows else ""))
        retain(out, {item["path"] for item in items}, live)
    return {"update": update, "windows": windows, "files": [item["path"] for item in items]}


def render(files: list[dict], version: str = VERSION) -> str:
    e = html.escape
    by_platform = {}
    for item in files:
        by_platform.setdefault(item["platform"], []).append(item)
    cards, hero_buttons = [], []
    for pid, (title, req, engine, howto) in PLATFORMS.items():
        items = by_platform.get(pid, [])
        if not items:
            continue
        primary, rest = items[0], items[1:]
        icon = f'<svg viewBox="0 0 24 24" aria-hidden="true">{ICONS[pid]}</svg>'
        hero_buttons.append(
            f'<a class="btn primary" data-os="{pid}" href="{e(primary["path"])}" download>'
            f'{icon}<span>{e(title)}版をダウンロード</span></a>')
        rows = "".join(
            f'<li><a href="{e(f["path"])}" download><span class="label">{e(f["label"])}</span>'
            f'<span class="meta">{e(f["size"])}</span></a><p class="note">{e(f["note"])}</p>'
            f'<details><summary>SHA-256</summary><code>{e(f["sha"])}</code></details></li>'
            for f in rest)
        cards.append(f"""
      <article class="card" id="{pid}" data-os="{pid}">
        <header>{icon}<div><h3>{e(title)}</h3><p>{e(req)} · {e(engine)}</p></div></header>
        <a class="btn primary wide" href="{e(primary["path"])}" download>
          <span>{e(primary["label"])}</span><span class="meta">{e(primary["size"])}</span></a>
        <p class="note">{e(primary["note"])}</p>
        <details><summary>SHA-256</summary><code>{e(primary["sha"])}</code></details>
        {f'<ul class="more">{rows}</ul>' if rows else ''}
        <p class="howto">{e(howto)}</p>
      </article>""")
    updated = max((f["date"] for f in files), default="")
    return TEMPLATE.format(version=version, updated=updated,
                           hero_buttons="\n        ".join(hero_buttons), cards="".join(cards))


TEMPLATE = """<!doctype html>
<html lang="ja">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Still Browser</title>
<meta name="description" content="縦タブのシンプルなブラウザ Still のダウンロード">
<style>
:root {{
  --canvas: #f5f5f7; --surface: #fff; --label: #1d1d1f; --secondary: #6e6e73;
  --line: rgb(0 0 0 / 8%); --hover: rgb(0 0 0 / 5%); --accent: #007aff; --accent-hover: #0068d9;
  --shadow: 0 1px 2px rgb(0 0 0 / 6%), 0 8px 24px rgb(0 0 0 / 6%);
  color-scheme: light dark;
}}
@media (prefers-color-scheme: dark) {{
  :root {{
    --canvas: #1c1c1e; --surface: #2c2c2e; --label: #f5f5f7; --secondary: #98989d;
    --line: rgb(255 255 255 / 10%); --hover: rgb(255 255 255 / 7%); --accent: #0a84ff; --accent-hover: #3d9bff;
    --shadow: 0 1px 2px rgb(0 0 0 / 30%), 0 8px 24px rgb(0 0 0 / 30%);
  }}
}}
* {{ box-sizing: border-box; }}
body {{
  margin: 0; background: var(--canvas); color: var(--label);
  font: 15px/1.6 system-ui, -apple-system, "Segoe UI", "Hiragino Sans", "Noto Sans JP", sans-serif;
  -webkit-font-smoothing: antialiased;
}}
a {{ color: inherit; }}
.wrap {{ max-width: 1040px; margin: 0 auto; padding: 0 20px; }}
nav {{ display: flex; align-items: center; justify-content: space-between; height: 56px; }}
.brand {{ display: flex; align-items: center; gap: 8px; font-weight: 600; text-decoration: none; }}
.mark {{ width: 22px; height: 22px; border-radius: 6px; background: var(--accent); display: grid; place-items: center; }}
.mark::before {{ content: ""; width: 4px; height: 12px; border-radius: 2px; background: #fff; transform: translateX(-4px); }}
nav .links a {{ color: var(--secondary); text-decoration: none; font-size: 14px; margin-left: 16px; }}
nav .links a:hover {{ color: var(--label); }}

.hero {{ padding: 56px 0 32px; text-align: center; }}
.hero h1 {{ font-size: clamp(32px, 6vw, 52px); line-height: 1.15; letter-spacing: -0.02em; margin: 0 0 14px; font-weight: 700; }}
.hero .lead {{ color: var(--secondary); font-size: clamp(15px, 2.2vw, 18px); margin: 0 auto 28px; max-width: 34em; }}
.actions {{ display: flex; flex-wrap: wrap; gap: 10px; justify-content: center; }}
.hero .btn[data-os] {{ display: none; }}
.hero[data-detected="none"] .btn[data-os],
.hero .btn[data-os].detected {{ display: inline-flex; }}
.sub {{ color: var(--secondary); font-size: 13px; margin-top: 12px; }}
.sub a {{ color: var(--accent); text-decoration: none; }}

.btn {{
  display: inline-flex; align-items: center; justify-content: center; gap: 8px;
  height: 40px; padding: 0 18px; border-radius: 10px; border: 1px solid var(--line);
  background: var(--surface); text-decoration: none; font-weight: 500; font-size: 14px;
  transition: background .15s, transform .15s;
}}
.btn:hover {{ background: var(--hover); }}
.btn:active {{ transform: scale(.98); }}
.btn.primary {{ background: var(--accent); border-color: transparent; color: #fff; }}
.btn.primary:hover {{ background: var(--accent-hover); }}
.btn svg {{ width: 18px; height: 18px; fill: currentColor; flex: none; }}
.btn.wide {{ display: flex; width: 100%; justify-content: space-between; }}
.btn .meta {{ opacity: .8; font-weight: 400; font-variant-numeric: tabular-nums; }}

.shot {{ margin: 40px auto 0; border-radius: 14px; overflow: hidden; border: 1px solid var(--line); box-shadow: var(--shadow); background: var(--surface); }}
.shot img {{ display: block; width: 100%; height: auto; }}

section {{ padding: 56px 0; }}
h2 {{ font-size: 24px; letter-spacing: -0.01em; margin: 0 0 6px; }}
.section-lead {{ color: var(--secondary); margin: 0 0 24px; }}

.features {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 12px; }}
.feature {{ background: var(--surface); border: 1px solid var(--line); border-radius: 12px; padding: 18px; }}
.feature h3 {{ font-size: 15px; margin: 0 0 4px; }}
.feature p {{ margin: 0; color: var(--secondary); font-size: 14px; }}

.cards {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(290px, 1fr)); gap: 12px; align-items: start; }}
.card {{ background: var(--surface); border: 1px solid var(--line); border-radius: 14px; padding: 18px; scroll-margin-top: 16px; }}
.card.detected {{ border-color: var(--accent); box-shadow: 0 0 0 1px var(--accent); }}
.card header {{ display: flex; gap: 12px; align-items: center; margin-bottom: 14px; }}
.card header svg {{ width: 28px; height: 28px; fill: var(--label); flex: none; }}
.card h3 {{ margin: 0; font-size: 17px; }}
.card header p {{ margin: 0; color: var(--secondary); font-size: 13px; }}
.note {{ color: var(--secondary); font-size: 13px; margin: 6px 0 0; }}
details {{ margin-top: 4px; font-size: 12px; color: var(--secondary); }}
summary {{ cursor: pointer; width: max-content; }}
summary:hover {{ color: var(--label); }}
code {{ display: block; word-break: break-all; font: 11px/1.5 ui-monospace, "Cascadia Mono", Menlo, monospace; padding: 6px 8px; margin-top: 4px; border-radius: 6px; background: var(--hover); color: var(--label); }}
.more {{ list-style: none; margin: 14px 0 0; padding: 0; border-top: 1px solid var(--line); }}
.more li {{ padding: 10px 0; border-bottom: 1px solid var(--line); }}
.more a {{ display: flex; justify-content: space-between; gap: 8px; text-decoration: none; color: var(--accent); font-weight: 500; font-size: 14px; }}
.more a:hover .label {{ text-decoration: underline; }}
.more .meta {{ color: var(--secondary); font-weight: 400; font-variant-numeric: tabular-nums; }}
.more .note {{ margin-top: 2px; }}
.howto {{ margin: 14px 0 0; padding: 10px 12px; border-radius: 8px; background: var(--hover); font-size: 13px; color: var(--secondary); }}

footer {{ padding: 32px 0 48px; color: var(--secondary); font-size: 13px; border-top: 1px solid var(--line); }}
:focus-visible {{ outline: 2px solid var(--accent); outline-offset: 2px; border-radius: 6px; }}
@media (prefers-reduced-motion: reduce) {{ .btn {{ transition: none; }} }}
</style>
</head>
<body>
<div class="wrap">
  <nav>
    <a class="brand" href="#"><span class="mark" aria-hidden="true"></span>Still</a>
    <div class="links"><a href="#features">特長</a><a href="#download">ダウンロード</a></div>
  </nav>

  <header class="hero" data-detected="none">
    <h1>静かに、速く。<br>縦タブのブラウザ。</h1>
    <p class="lead">余計な装飾を削ぎ落とし、ページを広く使えるように設計したシンプルなブラウザです。Windows・Android・Linuxに対応しています。</p>
    <div class="actions">
        {hero_buttons}
    </div>
    <p class="sub">バージョン {version} · <a href="#download">ほかのプラットフォーム・形式</a></p>
    <div class="shot">
      <picture>
        <source srcset="img/hero-dark.png" media="(prefers-color-scheme: dark)">
        <img src="img/hero-light.png" width="1440" height="766" alt="Stillのウィンドウ。左に縦タブのサイドバー、上部にアドレス欄が並ぶ">
      </picture>
    </div>
  </header>

  <section id="features">
    <h2>特長</h2>
    <p class="section-lead">毎日使う道具だから、目立たず、邪魔をしないことを大切にしました。</p>
    <div class="features">
      <div class="feature"><h3>縦タブ</h3><p>タブを左に並べ、長いタイトルも読みやすく。サイドバーはワンクリックで開閉できます。</p></div>
      <div class="feature"><h3>Firefoxのエンジン</h3><p>Windows・Linux版はFirefox ESR、Android版はGeckoView。どの端末でも同じGeckoでページを表示します。</p></div>
      <div class="feature"><h3>入力しながら候補</h3><p>アドレス欄に入力すると、履歴・ブックマーク・開いているタブと検索エンジンの候補を出します。</p></div>
      <div class="feature"><h3>ライト／ダーク</h3><p>システム設定に合わせて自動で切り替え。アクセントカラーも選べます。</p></div>
      <div class="feature"><h3>データを失わない</h3><p>前回のタブを復元し、設定は安全に保存。壊れたファイルもバックアップしてから復旧します。</p></div>
    </div>
  </section>

  <section id="download">
    <h2>ダウンロード</h2>
    <p class="section-lead">最終更新 {updated}。ファイルの改ざんが気になる場合はSHA-256で確認できます。</p>
    <div class="cards">{cards}
    </div>
  </section>
</div>
<footer>
  <div class="wrap">Still Browser {version} · コード署名なしの開発版です。</div>
</footer>
<script>
(() => {{
  const ua = navigator.userAgent;
  const os = /Android/i.test(ua) ? "android" : /Windows/i.test(ua) ? "windows" : /Linux|X11/i.test(ua) ? "linux" : null;
  if (!os) return;
  const hero = document.querySelector(".hero");
  const button = hero.querySelector(`.btn[data-os="${{os}}"]`);
  if (!button) return;
  hero.dataset.detected = os;
  button.classList.add("detected");
  document.querySelector(`.card[data-os="${{os}}"]`)?.classList.add("detected");
}})();
</script>
</body>
</html>
"""


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--serve", action="store_true", help="serve the site after building")
    parser.add_argument("--port", type=int, default=8424)
    parser.add_argument("--host", default="0.0.0.0", help="address to listen on (default: every interface)")
    parser.add_argument("--artifacts", type=Path, default=ARTIFACTS, help="release files to publish (default: artifacts/)")
    parser.add_argument("--out", type=Path, default=OUT, help="site directory to build (default: site/public/)")
    parser.add_argument("--public-url", default=PUBLIC_URL, help=f"site address in update/windows.json (default: {PUBLIC_URL})")
    parser.add_argument("--public-key", help="updater public key to check signatures with, for test sites "
                                             "(default: plugins.updater.pubkey in desktop/src-tauri/tauri.conf.json)")
    args = parser.parse_args()
    if not args.public_url.endswith("/"):
        parser.error("--public-url must end with /")
    try:
        build(args.artifacts, args.out, public_url=args.public_url, public_key=args.public_key)
    except (PublishError, release_set.SignatureError) as error:
        raise SystemExit(f"Not published (the previous release stays live): {error}")
    if args.serve:
        handler = lambda *a, **kw: http.server.SimpleHTTPRequestHandler(*a, directory=str(args.out), **kw)
        print(f"serving on {args.host}:{args.port} — {args.public_url}  (Ctrl+C to stop)")
        http.server.ThreadingHTTPServer((args.host, args.port), handler).serve_forever()


if __name__ == "__main__":
    main()
