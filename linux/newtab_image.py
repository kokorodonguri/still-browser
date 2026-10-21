"""The picture shown behind a new tab (about:home and about:newtab).

The chosen file is copied into <profile>/chrome/newtab/ under a name Still
generates, and userContent.css points at that copy, so the stylesheet never
refers to a file outside the profile. Firefox reads userContent.css only at
startup: a picture chosen while Still is running appears after a restart.
Same formats and size limit as the Windows build (desktop/core/src/new_tab_image.rs).
"""
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time

# Larger files are refused before anything is written.
MAX_BYTES = 20 * 1024 * 1024
FOLDER = "newtab"
STORED = re.compile(r"newtab-(\d{1,20})\.(png|jpg|gif|webp|avif|bmp)")
TITLE = "新しいタブの画像を選ぶ"
PATTERNS = ("*.png", "*.jpg", "*.jpeg", "*.gif", "*.webp", "*.avif", "*.bmp")


class ImageError(Exception):
    """A picture that cannot be used or saved, worded for the user."""


UNSUPPORTED = "その画像は使えません。PNG、JPEG、GIF、WebP、AVIF、BMP（20MBまで）のファイルを選んでください。"


def sniff(data: bytes) -> str | None:
    """The format from the first bytes; the file name is not trusted."""
    if data.startswith(b"\x89PNG\r\n\x1a\n"):
        return "png"
    if data.startswith(b"\xff\xd8\xff"):
        return "jpg"
    if data.startswith((b"GIF87a", b"GIF89a")):
        return "gif"
    if data.startswith(b"RIFF") and data[8:12] == b"WEBP":
        return "webp"
    if data[4:12] in (b"ftypavif", b"ftypavis"):
        return "avif"
    if data.startswith(b"BM") and len(data) > 26:
        return "bmp"
    return None


def folder(profile: Path) -> Path:
    return profile / "chrome" / FOLDER


def current(profile: Path) -> Path | None:
    """The stored picture, newest first if an interrupted change left two."""
    try:
        names = [entry.name for entry in folder(profile).iterdir() if STORED.fullmatch(entry.name) and entry.is_file()]
    except OSError:
        return None
    if not names:
        return None
    return folder(profile) / max(names, key=lambda name: int(STORED.fullmatch(name)[1]))


def store(profile: Path, source: Path) -> Path:
    """Copies `source` into the profile and makes it the only stored picture."""
    try:
        with source.open("rb") as file:
            data = file.read(MAX_BYTES + 1)
    except OSError as error:
        raise ImageError(f"画像を読み込めません: {source}（{error.strerror or error}）")
    extension = sniff(data) if len(data) <= MAX_BYTES else None
    if extension is None:
        raise ImageError(UNSUPPORTED)
    directory = folder(profile)
    try:
        directory.mkdir(parents=True, exist_ok=True)
        # A fresh name each time, so Firefox never shows a cached earlier picture.
        stamp = time.time_ns() // 1_000_000
        while (directory / f"newtab-{stamp}.{extension}").exists():
            stamp += 1
        target = directory / f"newtab-{stamp}.{extension}"
        with tempfile.NamedTemporaryFile(dir=directory, prefix=".newtab-", delete=False) as temporary:
            temporary.write(data)
            temporary_path = Path(temporary.name)
        try:
            temporary_path.chmod(0o644)
            temporary_path.replace(target)
        finally:
            temporary_path.unlink(missing_ok=True)
    except OSError:
        raise ImageError("画像を保存できません。空き容量とプロファイルの書き込み権限を確認してください。")
    remove_others(profile, keep=target.name)
    return target


def remove_others(profile: Path, keep: str = "") -> int:
    """Deletes stored pictures other than `keep`; files Still did not name are left alone."""
    removed = 0
    try:
        entries = list(folder(profile).iterdir())
    except OSError:
        return 0
    for entry in entries:
        if STORED.fullmatch(entry.name) and entry.name != keep:
            try:
                entry.unlink()
                removed += 1
            except OSError:
                pass
    return removed


def stylesheet(profile: Path) -> str:
    """CSS appended to userContent.css; empty without a picture.

    The root element's background fills the whole page, so the picture goes
    there and the body is made transparent. about:blank keeps the plain color.
    """
    picture = current(profile)
    if picture is None:
        return ""
    return f'''
/* New tab picture, written by the launcher (./still --newtab-image). */
@-moz-document url("about:home"), url("about:newtab") {{
  :root {{
    background-image: url("{FOLDER}/{picture.name}") !important;
    background-position: center !important;
    background-size: cover !important;
    background-repeat: no-repeat !important;
    background-attachment: fixed !important;
  }}
  body {{ background: transparent !important; }}
  @media (forced-colors: active) {{
    :root {{ background-image: none !important; }}
  }}
}}
'''


def choose_file() -> Path | None:
    """Asks with the desktop's file dialog. None when cancelled.

    Tries GTK (through the desktop portal where there is one), then zenity and
    kdialog; raises ImageError when none of them can show a dialog.
    """
    try:
        return _choose_with_gtk()
    except (ImportError, ValueError, RuntimeError):
        pass
    if shutil.which("zenity"):
        result = subprocess.run(["zenity", "--file-selection", f"--title={TITLE}",
                                 f"--file-filter=画像 | {' '.join(PATTERNS)}"], capture_output=True, text=True)
        return _chosen(result)
    if shutil.which("kdialog"):
        result = subprocess.run(["kdialog", "--title", TITLE, "--getopenfilename", str(Path.home()),
                                 f"画像 ({' '.join(PATTERNS)})"], capture_output=True, text=True)
        return _chosen(result)
    raise ImageError("ファイルを選ぶ画面を開けません。端末で ./still --newtab-image 画像のパス を実行してください。")


def _chosen(result: subprocess.CompletedProcess) -> Path | None:
    path = result.stdout.strip()
    return Path(path) if result.returncode == 0 and path else None


def _choose_with_gtk() -> Path | None:
    import gi
    gi.require_version("Gtk", "3.0")
    from gi.repository import Gtk
    ready = Gtk.init_check()
    if not (ready[0] if isinstance(ready, tuple) else ready):
        raise RuntimeError("no display")
    dialog = Gtk.FileChooserNative.new(TITLE, None, Gtk.FileChooserAction.OPEN, "選択", "キャンセル")
    pictures = Gtk.FileFilter()
    pictures.set_name("画像")
    for pattern in PATTERNS:
        pictures.add_pattern(pattern)
        pictures.add_pattern(pattern.upper())
    dialog.add_filter(pictures)
    try:
        if dialog.run() != Gtk.ResponseType.ACCEPT:
            return None
        path = dialog.get_filename()
        return Path(path) if path else None
    finally:
        dialog.destroy()
        while Gtk.events_pending():
            Gtk.main_iteration()
