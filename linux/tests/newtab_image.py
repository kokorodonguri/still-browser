#!/usr/bin/env python3
"""Check the new tab picture in actual Firefox: shown on home and new tabs, not elsewhere."""
import json
from pathlib import Path
import struct
import subprocess
import tempfile
import time
import zlib
from selenium.webdriver.common.keys import Keys
from verify import ROOT, OUTPUT, start, chrome, content, navigate, key, wait_js, screenshot

PICTURE = (200, 40, 120)


def png(color, size=48):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    rows = b"".join(b"\0" + bytes(color) * size for _ in range(size))
    header = struct.pack(">IIBBBBB", size, size, 8, 2, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")


def page_pixel(driver):
    """The rendered color in the middle of the selected tab's page."""
    chrome(driver)
    return driver.execute_async_script('''const done = arguments[arguments.length - 1];
      const browser = gBrowser.selectedBrowser;
      const [w, h] = [browser.clientWidth, browser.clientHeight];
      browser.browsingContext.currentWindowGlobal.drawSnapshot(new DOMRect(w / 2, h / 2, 1, 1), 1, "white").then(bitmap => {
        const canvas = document.createElementNS("http://www.w3.org/1999/xhtml", "canvas");
        canvas.width = canvas.height = 1;
        const context = canvas.getContext("2d");
        context.drawImage(bitmap, 0, 0);
        done([...context.getImageData(0, 0, 1, 1).data.slice(0, 3)]);
      }).catch(e => done(String(e)));''')


def settle(driver, url):
    chrome(driver)
    wait_js(driver, f'const b = gBrowser.selectedBrowser; return b.currentURI.spec === "{url}" && !b.webProgress.isLoadingDocument', 60)
    time.sleep(.5)


results = []
for mode, value, canvas in (("light", 0, [245, 245, 247]), ("dark", 1, [28, 28, 30])):
    with tempfile.TemporaryDirectory(prefix="still-newtab-image-") as directory:
        profile = Path(directory) / "profile"
        picture = Path(directory) / "picture.png"
        picture.write_bytes(png(PICTURE))
        subprocess.run([str(ROOT / "linux/still"), "--profile", str(profile), "--newtab-image", str(picture)], check=True, capture_output=True)
        subprocess.run([str(ROOT / "linux/still"), "--profile", str(profile), "--prepare-only"], check=True, capture_output=True)
        with (profile / "user.js").open("a") as prefs:
            prefs.write(f'\nuser_pref("ui.systemUsesDarkTheme", {value});\nuser_pref("browser.theme.content-theme", {2 if value == 0 else 0});\nuser_pref("browser.theme.toolbar-theme", {2 if value == 0 else 0});\n')
        driver = None
        try:
            driver, _ = start(profile, str(ROOT / ".tools/geckodriver"), f"newtab-image-{mode}.log")
            navigate(driver, "about:home")
            settle(driver, "about:home")
            home = page_pixel(driver)
            screenshot(driver, f"newtab-image-{mode}.png")
            key(driver, Keys.CONTROL, "t")
            settle(driver, "about:newtab")
            new_tab = page_pixel(driver)
            navigate(driver, "about:blank")
            settle(driver, "about:blank")
            blank = page_pixel(driver)
            state = {"mode": mode, "home": home, "newtab": new_tab, "blank": blank}
            assert home == list(PICTURE) and new_tab == list(PICTURE), state
            assert blank == canvas, state
            results.append({**state, "passed": True})
        finally:
            if driver:
                driver.quit()
            (OUTPUT / "newtab-image.json").write_text(json.dumps(results, indent=2))
print(json.dumps(results, indent=2))
