#!/usr/bin/env python3
"""Review empty/error/loading states, keyboard use and window sizes in Gecko."""
from http.server import ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
from urllib.parse import urlparse

from verify import *

LOAD_CONTINUE = threading.Event()
REPORT = []


class ReviewPages(Pages):
    def do_GET(self):
        if urlparse(self.path).path == "/slow":
            LOAD_CONTINUE.wait(20)
        if urlparse(self.path).path == "/long":
            body = ('<!doctype html><title>' + 'Very long page title — ' * 60 + '</title><h1>Long title</h1>').encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        super().do_GET()


def record(name, details=None):
    REPORT.append({"check": name, "passed": True, "details": details})
    print("PASS", name, json.dumps(details, ensure_ascii=False) if details else "", flush=True)


def memory(process_id):
    parents = {}
    for status in Path("/proc").glob("[0-9]*/status"):
        try:
            parent = next(int(line.split()[1]) for line in status.read_text().splitlines() if line.startswith("PPid:"))
            parents[int(status.parent.name)] = parent
        except (OSError, StopIteration):
            continue
    family = {process_id}
    while children := {pid for pid, parent in parents.items() if parent in family and pid not in family}:
        family |= children
    pss = 0
    for pid in family:
        try:
            pss += next(int(line.split()[1]) for line in Path(f"/proc/{pid}/smaps_rollup").read_text().splitlines() if line.startswith("Pss:"))
        except (OSError, StopIteration):
            continue
    return {"processes": len(family), "pssMiB": round(pss / 1024, 1), "baseline": "one about:blank tab; Firefox processes only; Xvfb and geckodriver excluded"}


def main():
    temporary = tempfile.TemporaryDirectory(prefix="still-states-")
    profile = Path(temporary.name) / "profile"
    subprocess.run([str(ROOT / "linux/still"), "--profile", str(profile), "--prepare-only"], check=True, capture_output=True)
    with (profile / "user.js").open("a") as preferences:
        preferences.write('\nuser_pref("browser.startup.homepage", "about:blank");\n')
    server = ThreadingHTTPServer(("127.0.0.1", 0), ReviewPages)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{server.server_port}"
    driver = None
    try:
        driver, warnings = start(profile, str(ROOT / ".tools/geckodriver"), "ui-review.log")
        assert driver.find_element(By.ID, "back-button").get_attribute("disabled") == "true"
        record("Initial back button is disabled")
        navigate(driver, "about:blank")
        wait_js(driver, 'return gBrowser.currentURI.spec === "about:blank" && !gBrowser.selectedTab.hasAttribute("busy")')
        record("Blank tab memory measurement", memory(driver.capabilities["moz:processID"]))

        driver.execute_async_script('''const done=arguments[arguments.length-1];PlacesUtils.bookmarks.eraseEverything().then(()=>done(true));''')
        key(driver, Keys.CONTROL, "b")
        wait_js(driver, 'return SidebarController.currentID === "viewBookmarksSidebar"')
        screenshot(driver, "bookmarks-empty.png")
        key(driver, Keys.CONTROL, "b")
        record("Empty bookmark sidebar opens and closes")

        navigate(driver, "http://127.0.0.1:1/unreachable")
        content(driver)
        WebDriverWait(driver, 30).until(lambda d: d.execute_script('return document.documentURI.startsWith("about:neterror")'))
        WebDriverWait(driver, 30).until(lambda d: d.find_elements(By.CSS_SELECTOR, "h1") and d.find_element(By.CSS_SELECTOR, "h1").text)
        screenshot(driver, "error.png")
        navigate(driver, base + "/one")
        content(driver)
        WebDriverWait(driver, 30).until(lambda d: d.title == "First page")
        record("Error page gives guidance and navigation recovers")

        navigate(driver, base + "/slow")
        wait_js(driver, 'return gBrowser.selectedTab.hasAttribute("busy")')
        screenshot(driver, "loading.png")
        LOAD_CONTINUE.set()
        wait_js(driver, 'return !gBrowser.selectedTab.hasAttribute("busy") && gBrowser.currentURI.spec.endsWith("/slow")')
        record("Loading indicator clears after the page finishes")

        for path in ("/one", "/two", "/one", "/other"):
            navigate(driver, base + path)
        wait_js(driver, 'return gBrowser.currentURI.spec.endsWith("/other") && gBrowser.selectedBrowser.contentTitle === "Another tab"')
        assert driver.find_element(By.ID, "urlbar-input").get_attribute("value") == base + "/other"
        record("Rapid navigation keeps the address and selected page consistent")

        if not driver.execute_script("return SidebarController.launcherVisible"):
            driver.find_element(By.ID, "sidebar-button").click()
        wait_js(driver, "return SidebarController.launcherVisible")
        navigate(driver, base + "/long")
        wait_js(driver, 'return gBrowser.selectedTab.label.length > 500')
        screenshot(driver, "long-title.png")
        record("Long page title truncates in the native tab")
        driver.execute_script('''const url=arguments[0];for(let i=0;i<24;i++)gBrowser.addTab(url+"?tab="+i,{inBackground:true,triggeringPrincipal:Services.scriptSecurityManager.getSystemPrincipal()});''', base + "/long")
        wait_js(driver, 'return gBrowser.tabs.length === 25')
        screenshot(driver, "many-tabs.png")
        selected_before = driver.execute_script("return gBrowser.selectedTab._tPos")
        key(driver, Keys.CONTROL, Keys.TAB)
        wait_js(driver, f"return gBrowser.selectedTab._tPos !== {selected_before}")
        record("Many tabs scroll and Ctrl+Tab changes the selected tab", {"tabs": driver.execute_script("return gBrowser.tabs.length")})
        # Close the fixture tabs before reviewing the home layout.
        driver.execute_script('for(const t of [...gBrowser.tabs].slice(1))gBrowser.removeTab(t,{animate:false})')
        content(driver)
        driver.switch_to.window(driver.window_handles[0])
        navigate(driver, "about:home")
        content(driver)
        wait_js(driver, 'return document.readyState === "complete"')

        for width, height in ((320, 640), (390, 844), (768, 1024), (1280, 800), (1440, 900), (2560, 1440)):
            driver.set_window_size(width, height)
            chrome(driver)
            expanded = driver.execute_script('return SidebarController.launcherVisible')
            if expanded != (width >= 900):
                driver.find_element(By.ID, "sidebar-button").click()
                wait_js(driver, 'return SidebarController.launcherVisible === ' + str(width >= 900).lower())
            content(driver)
            geometry = driver.execute_script('''return {width:innerWidth,scrollWidth:document.documentElement.scrollWidth,
              controls:[...document.querySelectorAll("input,button,a")].filter(e=>e.getClientRects().length).length};''')
            screenshot(driver, f"width-{width}.png")
            record(f"Window {width}×{height}", {"actualWindow": driver.get_window_size(), "content": geometry})
            assert geometry["scrollWidth"] <= geometry["width"] + 1, geometry
            assert geometry["controls"] == 0, geometry
        record("Home layout has no horizontal overflow at reviewed sizes")
    except Exception as error:
        REPORT.append({"passed": False, "error": str(error)})
        if driver:
            screenshot(driver, "review-failure.png")
        raise
    finally:
        LOAD_CONTINUE.set()
        if driver:
            driver.quit()
        server.shutdown()
        temporary.cleanup()
        (OUTPUT / "ui-review.json").write_text(json.dumps(REPORT, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
