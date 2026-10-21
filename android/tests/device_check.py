#!/usr/bin/env python3
"""Drives Still on an emulator or device over adb and records what works.

Usage: python3 android/tests/device_check.py path/to/app.apk [output-dir]

Needs a rooted target (an emulator image other than "Google Play"), because it
reads the saved state and ends the page renderer to test crash recovery. A local
web server serves tests/site through `adb reverse`; no outside site is needed.
"""
import http.server
import json
import os
import re
import statistics
import subprocess
import sys
import threading
import time
import xml.etree.ElementTree as ET
from pathlib import Path

PKG = "app.still.browser"
ADB = os.environ.get("ADB", str(Path.home() / "Android/Sdk/platform-tools/adb"))
PORT = 8765
BASE = f"http://127.0.0.1:{PORT}"
SITE = Path(__file__).resolve().parent / "site"
DOWNLOAD = bytes(range(256)) * 3584  # 917,504 bytes

requests: list[str] = []
results: list[dict] = []


class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(SITE), **kwargs)

    def do_GET(self):
        requests.append(self.path)
        if self.path.startswith("/file.bin"):
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Disposition", 'attachment; filename="file.bin"')
            self.send_header("Content-Length", str(len(DOWNLOAD)))
            self.end_headers()
            self.wfile.write(DOWNLOAD)
            return
        super().do_GET()

    def end_headers(self):
        # No caching, so every page load reaches the request log.
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def log_message(self, *args):
        pass


def adb(*args, check=True, text=True, timeout=60):
    out = subprocess.run([ADB, *args], capture_output=True, text=text, timeout=timeout)
    if check and out.returncode != 0:
        raise RuntimeError(f"adb {' '.join(args)}: {out.stderr or out.stdout}")
    return out.stdout


def sh(command, check=False):
    return adb("shell", command, check=check)


def nodes(accessible_only=False):
    """The view tree. With accessible_only, only what a screen reader can reach."""
    flags = ["--compressed"] if accessible_only else []
    for _ in range(5):
        xml = adb("exec-out", "uiautomator", "dump", *flags, "/dev/tty", check=False)
        start = xml.find("<?xml")
        end = xml.rfind("</hierarchy>")
        if start >= 0 and end > 0:
            root = ET.fromstring(xml[start:end + len("</hierarchy>")])
            return [n.attrib for n in root.iter("node")]
        time.sleep(0.5)
    return []


def center(node):
    x1, y1, x2, y2 = map(int, re.findall(r"\d+", node["bounds"]))
    return (x1 + x2) // 2, (y1 + y2) // 2


def find(text=None, desc=None, contains=None, ours=True):
    # Only what a screen reader can reach, so nothing hidden behind a panel matches.
    for n in nodes(accessible_only=True):
        if ours and n.get("package") != PKG:
            continue
        if text is not None and n.get("text") == text:
            return n
        if desc is not None and n.get("content-desc") == desc:
            return n
        if contains is not None and (contains in n.get("text", "") or contains in n.get("content-desc", "")):
            return n
    return None


def wait(predicate, timeout=15.0, step=0.5):
    end = time.time() + timeout
    while time.time() < end:
        value = predicate()
        if value:
            return value
        time.sleep(step)
    return predicate()


def tap(node_or_xy):
    x, y = center(node_or_xy) if isinstance(node_or_xy, dict) else node_or_xy
    sh(f"input tap {x} {y}")


def tap_found(timeout=15.0, **kw):
    node = wait(lambda: find(**kw), timeout)
    if not node:
        raise AssertionError(f"not found: {kw}")
    tap(node)
    return node


def type_text(text):
    sh("input text " + text.replace(" ", "%s").replace("&", "\\&").replace("?", "\\?"))


def key(code):
    sh(f"input keyevent {code}")


def state():
    raw = sh(f"cat /data/data/{PKG}/files/state.json")
    try:
        return json.loads(raw)
    except json.JSONDecodeError:
        return {}


def wait_state(predicate, timeout=15.0):
    return wait(lambda: (lambda s: s if s and predicate(s) else None)(state()), timeout)


def app_pid():
    return sh(f"pidof {PKG}").strip()


def renderer_pids():
    """GeckoView's page (content) processes: app.still.browser:tab…"""
    out = sh("ps -A -o PID,NAME")
    return sorted(re.findall(rf"^\s*(\d+)\s+{re.escape(PKG)}:tab", out, re.M))


def engine_pids():
    """Every GeckoView child process: pages, GPU, crash helper."""
    out = sh("ps -A -o PID,NAME")
    return sorted(re.findall(rf"^\s*(\d+)\s+{re.escape(PKG)}:", out, re.M))


def pss_kb():
    """Proportional set size of the app and every engine process, in KB."""
    total = 0
    for pid in [app_pid(), *engine_pids()]:
        if not pid:
            continue
        out = sh(f"dumpsys meminfo {pid}")
        m = re.search(r"TOTAL PSS:\s+(\d+)", out) or re.search(r"^\s*TOTAL\s+(\d+)", out, re.M)
        if m:
            total += int(m.group(1))
    return total


def web_top():
    for n in nodes():
        if is_page(n):
            return list(map(int, re.findall(r"\d+", n["bounds"])))
    raise AssertionError("no page is showing")


def is_page(node):
    # Gecko's accessibility bridge can identify the document as a WebView.
    return node.get("package") == PKG and node.get("class") in {
        "android.webkit.WebView", "org.mozilla.geckoview.GeckoView",
    }


def open_menu_item(text):
    tap_found(desc="Menu")
    tap_found(text=text)


def tabs_open():
    return find(text="Tabs") and find(text="New tab")


def open_tabs():
    button = wait(lambda: next((n for n in nodes(accessible_only=True)
                               if n.get("package") == PKG
                               and re.fullmatch(r"\d+ tabs?", n.get("content-desc", ""))), None))
    expect(button, "tab counter missing")
    tap(button)
    expect(wait(tabs_open), "tab counter did not open the tab overview")


def page_tap(row):
    """Taps row `row` of the test page (rows are 64 CSS px tall, 72 apart)."""
    x1, y1, x2, _ = web_top()
    density = int(re.search(r"(\d+)", sh("wm density").splitlines()[-1]).group(1)) / 160
    sh(f"input tap {(x1 + x2) // 2} {int(y1 + (row * 72 + 32) * density)}")


def titles():
    return [e["Title"] for e in state().get("History", [])]


def wait_title(title, timeout=15.0):
    """Page titles reach the saved history through onTitleChange."""
    return wait(lambda: title in titles(), timeout)


def tab_titles():
    """Titles shown in the tab overview (closes it again)."""
    open_tabs()
    time.sleep(0.8)
    shown = [n.get("content-desc", "").split(": ", 1)[1]
             for n in nodes(accessible_only=True) if n.get("package") == PKG
             and n.get("content-desc", "").startswith(("Tab preview: ", "Tab placeholder: "))]
    key(4)
    expect(wait(lambda: not tabs_open()), "Back did not close the tab overview")
    return shown


def screenshot(out_dir, name):
    data = subprocess.run([ADB, "exec-out", "screencap", "-p"], capture_output=True, timeout=30).stdout
    (out_dir / f"{name}.png").write_bytes(data)


def pixel(x, y):
    """The screen's color at (x, y), from a raw screencap (RGBA after a small header)."""
    raw = subprocess.run([ADB, "exec-out", "screencap"], capture_output=True, timeout=30).stdout
    width, height = int.from_bytes(raw[0:4], "little"), int.from_bytes(raw[4:8], "little")
    offset = len(raw) - width * height * 4 + (y * width + x) * 4
    return tuple(raw[offset:offset + 3])


def page_center():
    """Sample below HomeView's upper 44%, clear of the logo and shortcut tiles."""
    x1, _, x2, y2 = map(int, re.findall(r"\d+", address_node()["bounds"]))
    return (x1 + x2) // 2, y2 // 2


def scroll_settings_up():
    scroll = next((n for n in nodes(accessible_only=True)
                   if n.get("package") == PKG and n.get("class") == "android.widget.ScrollView"), None)
    expect(scroll, "settings scroll area missing")
    x1, y1, x2, y2 = map(int, re.findall(r"\d+", scroll["bounds"]))
    x = (x1 + x2) // 2
    sh(f"input swipe {x} {int(y1 + (y2 - y1) * 0.8)} {x} {int(y1 + (y2 - y1) * 0.3)} 300")
    time.sleep(1)


PICTURE_COLOR = (200, 40, 120)


def picture_png(width=600, height=1000):
    import struct
    import zlib

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    rows = b"".join(b"\0" + bytes(PICTURE_COLOR) * width for _ in range(height))
    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")


def check(name, fn):
    try:
        details = fn()
        results.append({"check": name, "passed": True, "details": details})
        print(f"PASS {name} {json.dumps(details, ensure_ascii=False) if details is not None else ''}")
    except Exception as e:  # noqa: BLE001 - every failure is reported, then the run continues
        results.append({"check": name, "passed": False, "details": f"{type(e).__name__}: {e}"})
        print(f"FAIL {name}: {type(e).__name__}: {e}")


def expect(condition, message):
    if not condition:
        raise AssertionError(message)


def launch(intent_args=""):
    out = sh(f"am start -W -n {PKG}/.MainActivity {intent_args}")
    m = re.search(r"TotalTime: (\d+)", out)
    return int(m.group(1)) if m else None


def open_address(text):
    tap_found(contains="Search or enter address") if find(contains="Search or enter address") else tap(address_node())
    time.sleep(0.6)
    type_text(text)
    key(66)


def address_node():
    for n in nodes():
        if n.get("package") == PKG and n.get("class") == "android.widget.EditText":
            return n
    raise AssertionError("address bar not found")


def address_swipe_checks():
    """Starts and ends on tab 0 of a two-tab session, with the editor unfocused."""
    density = int(re.search(r"(\d+)", sh("wm density").splitlines()[-1]).group(1)) / 160

    def swipe(dx, dy=0):
        node = address_node()
        x, y = center(node)
        half = int(abs(dx) * density / 2)
        start = x + half if dx < 0 else x - half
        end = x - half if dx < 0 else x + half
        sh(f"input swipe {start} {y} {end} {int(y + dy * density)} 300")
        time.sleep(1)

    def unchanged(index):
        expect(state().get("ActiveIndex") == index, "unexpected tab switch")
        expect(address_node().get("focused") == "false", "gesture opened URL editing")

    swipe(70)
    unchanged(0)  # First tab does not wrap.
    swipe(-70)
    expect(wait_state(lambda s: s["ActiveIndex"] == 1), "left swipe did not select next tab")
    unchanged(1)
    swipe(-70)
    unchanged(1)  # Last tab does not wrap.
    swipe(70)
    expect(wait_state(lambda s: s["ActiveIndex"] == 0), "right swipe did not select previous tab")
    for dx, dy in [(-30, 0), (0, 60), (-60, -60)]:
        swipe(dx, dy)
        unchanged(0)
    swipe(0, -70)
    expect(wait(tabs_open, 5), "up swipe did not open the tab list")
    expect(state().get("ActiveIndex") == 0, "opening the tab list switched tabs")
    key(4)
    expect(wait(lambda: not tabs_open(), 5), "Back did not close the tab list")
    unchanged(0)
    tap(address_node())
    expect(address_node().get("focused") == "true", "tap did not open URL editing")
    swipe(-70)
    expect(state().get("ActiveIndex") == 0, "editing swipe switched tabs")
    expect(address_node().get("focused") == "true", "editing swipe lost focus")
    key(4)  # Hide IME, then leave editing if predictive Back only hid the IME.
    if address_node().get("focused") == "true":
        key(4)
    unchanged(0)


def main():
    apk = sys.argv[1]
    out_dir = Path(sys.argv[2] if len(sys.argv) > 2 else "artifacts/android-checks")
    out_dir.mkdir(parents=True, exist_ok=True)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", PORT), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()

    adb("root", check=False)
    time.sleep(2)
    adb("wait-for-device")
    adb("reverse", f"tcp:{PORT}", f"tcp:{PORT}")
    adb("install", "-r", apk, timeout=180)
    sh(f"pm clear {PKG}")
    sh("cmd uimode night no")
    sh("settings put system accelerometer_rotation 0; settings put system user_rotation 0")
    sh("rm -f /sdcard/Download/file*.bin")
    env = {
        "device": sh("getprop ro.product.model").strip(),
        "android": sh("getprop ro.build.version.release").strip(),
        "engine": "GeckoView",
        "abi": sh("getprop ro.product.cpu.abi").strip(),
        "apk_bytes": Path(apk).stat().st_size,
    }
    print("Environment", json.dumps(env))
    results.append({"environment": env})

    def cold_start():
        times = []
        for _ in range(5):
            sh(f"am force-stop {PKG}")
            time.sleep(1)
            times.append(launch())
        return {"runs_ms": times, "median_ms": statistics.median(times)}
    check("Cold start to first frame (am start -W, blank session)", cold_start)

    def blank_home():
        expect(find(contains="Search or enter address"), "address hint missing")
        expect(find(text="Still"), "the new-tab home wordmark is missing")
        expect(not find(text="Shortcuts"), "a fresh profile has unexpected shortcuts")
        expect(not any(is_page(n) for n in nodes()), "a blank tab created a page")
        screenshot(out_dir, "light-blank")
    check("Blank start shows only the address bar and toolbar, no page", blank_home)

    def menu_sheet():
        tap_found(desc="Menu")
        for text in ("New tab", "Bookmarks", "History", "Downloads", "Settings"):
            expect(find(text=text), f"menu lacks {text}")
        for desc in ("Add bookmark", "Share", "Reload"):
            action = find(desc=desc) or find(text=desc)
            expect(action and action.get("enabled") == "false", f"{desc} is enabled on a blank tab")
        expect(not any(is_page(n) for n in nodes(accessible_only=True)), "blank menu exposes a page")
        screenshot(out_dir, "light-menu-blank")
        key(4)
        expect(wait(lambda: not find(text="Settings")), "Back did not dismiss the menu")
        tap_found(desc="Menu")
        scrim = find(desc="Dismiss")
        expect(scrim, "the menu scrim is missing")
        x1, y1, x2, y2 = map(int, re.findall(r"\d+", scrim["bounds"]))
        # Sheets occupy at most 90% of the available height.
        tap(((x1 + x2) // 2, y1 + (y2 - y1) // 20))
        expect(wait(lambda: not find(text="Settings")), "tapping the scrim did not dismiss the menu")
    check("Menu sheet: actions are disabled on blank tabs; Back and the scrim dismiss it", menu_sheet)

    def navigate():
        open_address(f"{BASE}/index.html")
        expect(wait_title("Home light"), "test page did not render")
        s = wait_state(lambda s: s.get("Tabs") and s["Tabs"][0]["Url"].startswith(BASE))
        expect(s, "state.json does not record the tab")
        return s["Tabs"][0]["Url"]
    check("Address bar opens a URL and the tab is saved", navigate)

    def no_auto_popup():
        time.sleep(2)
        s = state()
        expect(len(s.get("Tabs", [])) == 1, f"script popup without a gesture opened a tab: {s.get('Tabs')}")
        expect("/popup.html?auto" not in requests, "the blocked popup was requested")
    check("window.open without a user gesture is blocked", no_auto_popup)

    def light_scheme():
        expect(wait_title("Home light"), f"page does not report prefers-color-scheme: light ({titles()[-3:]})")
        screenshot(out_dir, "light-page")
    check("Pages see prefers-color-scheme: light in the light theme", light_scheme)

    def links_and_back():
        page_tap(0)
        expect(wait_state(lambda s: s["Tabs"][0]["Url"].endswith("/second.html")), "link did not navigate")
        tap_found(desc="Back")
        expect(wait_state(lambda s: s["Tabs"][0]["Url"].endswith("/index.html")), "Back did not return")
        h = wait_state(lambda s: any(e["Title"] == "Second page" for e in s.get("History", [])))
        expect(h, "history lacks the second page title")
        return [(e["Url"].replace(BASE, ""), e["Title"]) for e in h["History"]]
    check("Links, Back button and history with titles", links_and_back)

    def bookmark():
        tap_found(desc="Menu")
        tap_found(desc="Add bookmark")
        s = wait_state(lambda s: any(b.endswith("/index.html") for b in s.get("Bookmarks", [])))
        expect(s, "bookmark not saved")
        tap_found(desc="Menu")
        expect(find(desc="Remove bookmark"), "menu does not show the saved bookmark state")
        expect(not any(is_page(n) for n in nodes(accessible_only=True)), "the page behind the menu is still exposed to accessibility")
        screenshot(out_dir, "light-menu-page")
        tap_found(text="Bookmarks")
        expect(wait(lambda: find(text="Home light")), "bookmark list lacks the page title")
        expect(not any(is_page(n) for n in nodes(accessible_only=True)), "the page behind the panel is still exposed to accessibility")
        screenshot(out_dir, "light-bookmarks")
        tap_found(desc="Back")
    check("Star saves a bookmark; Bookmarks list shows its title; the page is hidden from screen readers under the panel", bookmark)

    def target_blank():
        page_tap(1)
        s = wait_state(lambda s: len(s.get("Tabs", [])) == 2 and s["ActiveIndex"] == 1)
        expect(s, "target=_blank did not open an active second tab")
        expect(wait_state(lambda s: s["Tabs"][1]["Url"].endswith("/second.html?blank")), "new tab did not load")
        key(4)  # system back: no history in the new tab, so it closes and returns to its opener
        s = wait_state(lambda s: len(s.get("Tabs", [])) == 1)
        expect(s and s["Tabs"][0]["Url"].endswith("/index.html"), "back did not close the new tab")
    check("target=_blank opens a tab; Back closes it and returns to the opener", target_blank)

    def popup_with_opener():
        page_tap(2)
        expect(wait_state(lambda s: len(s.get("Tabs", [])) == 2), "popup tab not opened")
        expect(wait_title("Popup opener yes"), f"popup lacks window.opener ({titles()[-3:]})")
        time.sleep(0.5)
        page_tap(0)
        expect(wait_state(lambda s: len(s.get("Tabs", [])) == 1), "window.close() did not close the tab")
    check("window.open from a tap keeps opener (sign-in popups); window.close closes it", popup_with_opener)

    def file_link_blocked():
        page_tap(5)
        time.sleep(1.5)
        expect(state()["Tabs"][0]["Url"].endswith("/index.html"), "file: link navigated")
        expect(PKG in sh("dumpsys activity activities | grep -m1 topResumedActivity"), "left the app")
    check("file: links are refused", file_link_blocked)

    def external_link():
        page_tap(4)
        # A mail app can take several seconds to come up on a cold emulator.
        def outcome():
            top = sh("dumpsys activity activities | grep -m1 topResumedActivity")
            notice = find(contains="No app can open this link")
            return (top, notice) if notice or (top.strip() and PKG not in top) else None
        top, notice = wait(outcome, 10) or (sh("dumpsys activity activities | grep -m1 topResumedActivity"), None)
        alive = bool(app_pid())
        left = bool(top.strip()) and PKG not in top
        if left:
            key(4)
            time.sleep(1)
            launch()
            time.sleep(1)
        expect(alive, "app died")
        expect(notice or left, "mailto: neither opened an app nor explained why")
        return "notice shown" if notice else "opened " + top.strip().split()[-2]
    check("mailto: opens another app or shows a notice", external_link)

    def download():
        page_tap(3)
        s = wait_state(lambda s: s.get("Downloads") and s["Downloads"][-1]["State"] == "Completed", timeout=40)
        expect(s, f"download did not complete: {state().get('Downloads')}")
        name = s["Downloads"][-1]["Path"]
        size = sh(f"stat -c %s /sdcard/Download/{name}").strip()
        expect(size == str(len(DOWNLOAD)), f"saved size {size}")
        expect(state()["Tabs"][0]["Url"].endswith("/index.html"), "the download replaced the page")
        open_menu_item("Downloads")
        expect(wait(lambda: find(contains="Completed")), "Downloads panel does not show the file")
        screenshot(out_dir, "light-downloads")
        tap_found(desc="Back")
        return {"file": name, "bytes": int(size)}
    check("Download is saved to Downloads (MediaStore)", download)

    def tabs_panel():
        open_tabs()
        tap_found(text="New tab")
        time.sleep(1)
        expect(address_node().get("focused") == "true", "new tab did not focus the address bar")
        screenshot(out_dir, "light-keyboard")
        type_text(f"{BASE}/second.html")
        key(66)
        expect(wait_state(lambda s: len(s.get("Tabs", [])) == 2 and s["ActiveIndex"] == 1), "second tab not saved")
        expect(wait_title("Second page"), "second tab did not finish loading")
        open_tabs()
        expect(wait(lambda: find(desc="Tab preview: Home light")), "tab list lacks the first tab's captured preview")
        active_card = wait(lambda: find(desc="Tab preview: Second page"))
        expect(active_card, "the loaded active tab lacks a captured page preview")
        expect(active_card.get("selected") == "true", "the active tab card is not exposed as selected")
        expect(find(desc="Tab preview: Home light").get("selected") != "true", "the inactive tab card is exposed as selected")
        expect(not any(is_page(n) for n in nodes(accessible_only=True)), "the page behind the tab overview is still exposed to accessibility")
        screenshot(out_dir, "light-tabs")
        tap_found(desc="Tab preview: Home light")
        expect(wait_state(lambda s: s["ActiveIndex"] == 0), "selecting a tab did not switch")
    check("Tabs panel: new tab, switching", tabs_panel)

    def suggestions():
        tap(address_node())
        time.sleep(0.6)
        type_text("second")
        # The page opened with target=_blank earlier is in history only; second.html is also an open tab.
        row = wait(lambda: find(contains="127.0.0.1:8765/second.html?blank"), 10)
        expect(row, "typing did not suggest the visited page")
        expect(find(contains="Switch to tab · 127.0.0.1:8765/second.html"), "the open tab is not suggested")
        screenshot(out_dir, "light-suggestions")
        searches = [n.get("content-desc") for n in nodes() if n.get("package") == PKG and n.get("content-desc", "").startswith("second ")]
        tap(row)
        expect(wait_state(lambda s: s["Tabs"][s["ActiveIndex"]]["Url"].endswith("/second.html?blank")), "choosing the suggestion did not open it")
        expect(address_node().get("focused") == "false", "the address bar stayed in editing")
        tap_found(desc="Back")
        expect(wait_state(lambda s: s["Tabs"][s["ActiveIndex"]]["Url"].endswith("/index.html")), "Back did not return")
        return {"search_suggestions": searches[:4]}
    check("Address bar suggests visited pages and search phrases while typing", suggestions)
    check("Address swipes: direction, boundaries, up for the tab list, accidental drags and URL editing", address_swipe_checks)

    def dark_theme():
        open_menu_item("Settings")
        tap_found(text="Theme")
        tap_found(text="Dark")
        expect(wait_state(lambda s: s["Settings"]["Theme"] == "Dark"), "theme not saved")
        time.sleep(1)
        screenshot(out_dir, "dark-settings")
        tap_found(desc="Back")
        time.sleep(2)
        live = "Home dark" in tab_titles()
        if not live:
            tap_found(desc="Menu")
            tap_found(desc="Reload")
            time.sleep(2)
        shown = tab_titles()
        expect("Home dark" in shown, f"page does not see prefers-color-scheme: dark ({shown})")
        time.sleep(1)
        screenshot(out_dir, "dark-page")
        return {"applied_without_reload": bool(live)}
    check("Dark theme reaches the chrome and the page's prefers-color-scheme", dark_theme)

    def rotation_keeps_pages():
        pid = app_pid()
        renderers = renderer_pids()
        sh("settings put system user_rotation 1")
        time.sleep(2.5)
        screenshot(out_dir, "dark-landscape")
        sh("settings put system user_rotation 0")
        time.sleep(2)
        expect(app_pid() == pid, "process restarted")
        expect(renderer_pids() == renderers, "page renderer restarted")
        expect(any(is_page(n) for n in nodes()), "page was lost on rotation")
    check("Rotation keeps the page (no activity recreation)", rotation_keeps_pages)

    def renderer_killed():
        # GeckoView reports a page process ended from outside (Android's
        # low-memory killer, or kill here) as killed, not crashed: the page
        # comes back by itself with its history. A crash inside the engine
        # shows a notice with Reload instead; it cannot be caused from adb.
        pids = renderer_pids()
        expect(pids, "no page process found")
        pid = app_pid()
        before = len(requests)
        for r in pids:
            sh(f"kill -9 {r}")
        expect(wait(lambda: any("index.html" in r for r in requests[before:]), 30), "the page did not come back")
        expect(app_pid() == pid, "the app process died with the page process")
        expect(wait(lambda: renderer_pids() and not set(renderer_pids()) & set(pids)), "no new page process")
        screenshot(out_dir, "dark-recovered")
        return {"killed": pids, "new": renderer_pids()}
    check("Page process killed: the app survives and the page loads again", renderer_killed)

    def restart_restores_lazily():
        sh(f"am force-stop {PKG}")
        before = len(requests)
        launch()
        expect(wait(lambda: any("index.html" in r for r in requests[before:])), "active tab not restored")
        time.sleep(2)
        lazy = [r for r in requests[before:] if "second" in r]
        expect(not lazy, f"background tab loaded at startup: {lazy}")
        open_tabs()
        placeholder = find(desc="Tab placeholder: Second page") or find(desc="Tab placeholder: 127.0.0.1")
        expect(placeholder, "the restored background tab should have a placeholder until loaded")
        expect(not find(desc="Tab preview: Second page") and not find(desc="Tab preview: 127.0.0.1"),
               "a background thumbnail unexpectedly survived process restart")
        screenshot(out_dir, "dark-tabs-restored")
        tap(placeholder)
        expect(wait(lambda: any("second" in r for r in requests[before:])), "background tab did not load when selected")
        return {"requests_after_restart": requests[before:]}
    check("Restart restores tabs; background tabs load only when selected", restart_restores_lazily)

    def home_shortcuts():
        tap_found(desc="New tab")
        key(4)  # Hide the keyboard to expose the home content.
        if address_node().get("focused") == "true":
            key(4)
        expect(wait(lambda: find(text="Still")), "the new-tab home wordmark is missing")
        expect(find(text="Shortcuts"), "saved bookmarks and visits did not produce home shortcuts")
        shortcut = find(text="Home light") or find(text="Home dark")
        expect(shortcut, "the bookmarked home page is missing from shortcuts")
        screenshot(out_dir, "dark-home-shortcuts")
        tap(shortcut)
        expect(wait_state(lambda s: s["Tabs"][s["ActiveIndex"]]["Url"].endswith("/index.html")), "home shortcut did not open its page")
    check("Home shortcuts show bookmarks and visits and open their saved URL", home_shortcuts)

    def new_tab_picture():
        source = out_dir / "still-picture.png"
        source.write_bytes(picture_png())
        adb("push", str(source), "/sdcard/Pictures/still-picture.png")
        sh("content call --uri content://media/external/file --method scan_file --arg /storage/emulated/0/Pictures/still-picture.png")
        open_menu_item("Settings")
        for _ in range(4):
            if find(text="Choose…"):
                break
            scroll_settings_up()
        tap_found(text="Choose…")
        # The system photo picker (Android 13+) lists the pushed picture.
        photo = wait(lambda: next((n for n in nodes() if n.get("content-desc", "").startswith("Photo taken")), None), 15)
        expect(photo, "the photo picker does not show the picture")
        tap(photo)
        s = wait_state(lambda s: s["Settings"].get("NewTabImage", "").startswith("newtab-"))
        expect(s, "picture not saved in settings")
        name = s["Settings"]["NewTabImage"]
        expect(name in sh(f"ls /data/data/{PKG}/files/NewTab"), "picture file missing")
        expect(wait(lambda: find(text="Remove")), "no Remove button after choosing")
        screenshot(out_dir, "settings-picture")
        tap_found(desc="Back")

        def new_tab_color():
            tap_found(desc="New tab")
            key(4)  # close the keyboard a new tab opens with
            time.sleep(1.5)
            return pixel(*page_center())

        new_tab = new_tab_color()
        screenshot(out_dir, "newtab-picture")
        expect(new_tab == PICTURE_COLOR, f"new tab shows {new_tab}, not the picture")
        open_address(f"{BASE}/second.html?picture")
        expect(wait(lambda: any("picture" in r for r in requests)), "page did not load")
        time.sleep(2)
        page = pixel(*page_center())
        expect(page != PICTURE_COLOR, "the picture shows behind a web page")
        sh(f"am force-stop {PKG}")
        launch()
        time.sleep(2)
        restarted = new_tab_color()
        expect(restarted == PICTURE_COLOR, f"after a restart the new tab shows {restarted}")
        open_menu_item("Settings")
        for _ in range(4):
            if find(text="Remove"):
                break
            scroll_settings_up()
        tap_found(text="Remove")
        expect(wait_state(lambda s: s["Settings"].get("NewTabImage") == ""), "removal not saved")
        expect(wait(lambda: name not in sh(f"ls /data/data/{PKG}/files/NewTab")), "picture file not deleted")
        tap_found(desc="Back")
        time.sleep(1)
        removed = pixel(*page_center())
        expect(removed in ((249, 249, 251), (28, 27, 34)), f"after Remove the new tab shows {removed}, not the canvas")
        sh("rm -f /sdcard/Pictures/still-picture.png")
        return {"file": name, "new_tab": new_tab, "page": page, "after_restart": restarted, "after_remove": removed}
    check("New tab picture: chosen in the photo picker, shown only on new tabs", new_tab_picture)

    def memory():
        sh(f"am force-stop {PKG}")
        sh(f"pm clear {PKG}")
        launch(f"-a android.intent.action.VIEW -d {BASE}/second.html?0")
        time.sleep(4)
        samples = {"1": pss_kb()}
        for i in range(1, 20):
            launch(f"-a android.intent.action.VIEW -d {BASE}/second.html?{i}")
            time.sleep(1.5)
            if i + 1 in (5, 20):
                time.sleep(3)
                samples[str(i + 1)] = pss_kb()
        s = wait_state(lambda s: len(s.get("Tabs", [])) == 20)
        expect(s, f"expected 20 tabs, got {len(state().get('Tabs', []))}")
        return {"pss_kb_by_tabs": samples}
    check("Memory with 1, 5 and 20 tabs (PSS of app + engine processes)", memory)

    (out_dir / "results.json").write_text(json.dumps(results, indent=2, ensure_ascii=False))
    passed = sum(1 for r in results if r.get("passed"))
    total = sum(1 for r in results if "passed" in r)
    print(f"{passed}/{total} passed")
    server.shutdown()
    return 0 if passed == total else 1


if __name__ == "__main__":
    sys.exit(main())
