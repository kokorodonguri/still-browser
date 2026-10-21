#!/usr/bin/env python3
"""Exercise the actual Firefox chrome and content UI on a Linux display."""
import argparse
from collections import Counter
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import time
from urllib.parse import parse_qs, urlparse

from selenium import webdriver
from selenium.webdriver import ActionChains
from selenium.webdriver.common.by import By
from selenium.webdriver.common.keys import Keys
from selenium.webdriver.firefox.options import Options
from selenium.webdriver.firefox.service import Service
from selenium.webdriver.support.ui import WebDriverWait

ROOT = Path(__file__).resolve().parents[2]
OUTPUT = ROOT / "artifacts" / "linux-checks"
COUNTS = Counter()
RESULTS = []
PAYLOAD = b"Still download verification\n" * 32768
DOWNLOAD_CONTINUE = threading.Event()


class Pages(BaseHTTPRequestHandler):
    def do_GET(self):
        route = urlparse(self.path).path
        COUNTS[route] += 1
        if route == "/download":
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Disposition", 'attachment; filename="still-test.txt"')
            self.send_header("Content-Length", str(len(PAYLOAD)))
            self.end_headers()
            for start in range(0, len(PAYLOAD), 32768):
                self.wfile.write(PAYLOAD[start:start + 32768])
                self.wfile.flush()
                if start == 7 * 32768:
                    DOWNLOAD_CONTINUE.wait(40)
                time.sleep(.05)
            return
        title = {"/one": "First page", "/two": "Second page", "/other": "Another tab"}.get(route, "Still test")
        body = f'''<!doctype html><meta charset="utf-8"><title>{title}</title>
        <style>body{{margin:0;background:#f2f1e9;color:#263b32;font:16px sans-serif;padding:80px}}h1{{font:42px Georgia,serif}}a{{color:#3a6451;margin-right:25px}}p{{color:#536658;line-height:1.8}}</style>
        <small>STILL · BROWSER CHECK</small><h1>{title}</h1><p>A real page, rendered by Firefox / Gecko.</p>
        <a id="next" href="/two">Next page</a><a id="download" href="/download">Download test file</a>'''.encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Cache-Control", "no-store")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_):
        pass


def check(name, details=None):
    RESULTS.append({"check": name, "passed": True, "details": details})
    print("PASS", name, json.dumps(details, ensure_ascii=False) if details else "", flush=True)


def chrome(driver):
    driver.set_context("chrome")


def content(driver):
    driver.set_context("content")


def wait_js(driver, script, timeout=30):
    return WebDriverWait(driver, timeout).until(lambda d: d.execute_script(script))


def screenshot(driver, name):
    chrome(driver)
    driver.save_screenshot(str(OUTPUT / name))


def navigate(driver, text):
    chrome(driver)
    address = driver.find_element(By.ID, "urlbar-input")
    address.click()
    address.send_keys(Keys.CONTROL, "a")
    address.send_keys(text)
    address.send_keys(Keys.ENTER)


def key(driver, *keys):
    chrome(driver)
    actions = ActionChains(driver)
    for k in keys[:-1]:
        actions.key_down(k)
    actions.send_keys(keys[-1])
    for k in reversed(keys[:-1]):
        actions.key_up(k)
    actions.perform()


def wait_google_query(driver, query):
    # Google may redirect this test host to a CAPTCHA. Validate the exact
    # requested search URL carried by that redirect; never solve/bypass it.
    def submitted(d):
        current = d.execute_script('return gBrowser.currentURI.spec')
        parsed = urlparse(current)
        challenged = parsed.hostname == "www.google.com" and parsed.path.startswith("/sorry/")
        target = parse_qs(parsed.query).get("continue", [""])[0] if challenged else current
        request = urlparse(target)
        if request.hostname == "www.google.com" and request.path == "/search" and parse_qs(request.query).get("q") == [query]:
            return target, challenged
        return False
    target, challenged = WebDriverWait(driver, 30).until(submitted)
    if challenged:
        RESULTS.append({"environmentWarnings": ["Google CAPTCHA: query submission verified; search results unavailable for " + query]})
    return target


def start(profile, geckodriver, log):
    env = dict(os.environ, MOZ_REMOTE_ALLOW_SYSTEM_ACCESS="1")
    options = Options()
    options.binary_location = str(ROOT / "linux" / "runtime" / "firefox" / "firefox")
    options.add_argument("-profile")
    options.add_argument(str(profile))
    # geckodriver otherwise forces a blank startup in reused profiles.
    # Exercise the app's default restore behavior instead of that harness default.
    options.set_preference("browser.startup.page", 3)
    options.accept_insecure_certs = False
    service = Service(geckodriver, log_output=str(OUTPUT / log), env=env)
    driver = webdriver.Firefox(options=options, service=service)
    driver.set_script_timeout(60)
    driver.set_window_size(1440, 900)
    chrome(driver)
    driver.execute_async_script('''const done=arguments[arguments.length-1];
      Services.search.init().then(()=>done(true)).catch(e=>done(String(e)));''')
    wait_js(driver, 'return document.getElementById("sidebar-main") && gBrowser.tabs.length>0')
    # The host disallows user namespaces. Record Firefox's warning, then dismiss it
    # through its native notification API for an unobstructed UI screenshot.
    warnings = driver.execute_script('''return [...document.querySelectorAll("notification-message")].map(n=>n.textContent.trim())''')
    driver.execute_script('''for(const n of document.querySelectorAll("notification-message")){n.close?.();}''')
    return driver, warnings


def run(geckodriver):
    OUTPUT.mkdir(parents=True, exist_ok=True)
    server = ThreadingHTTPServer(("127.0.0.1", 0), Pages)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{server.server_port}"
    driver = None
    temporary = tempfile.TemporaryDirectory(prefix="still-ui-check-")
    directory = temporary.name
    try:
        profile = Path(directory) / "profile"
        subprocess.run([str(ROOT / "linux" / "still"), "--profile", str(profile), "--prepare-only"], check=True, capture_output=True)
        downloads = Path(directory) / "downloads"
        downloads.mkdir()
        with (profile / "user.js").open("a") as prefs:
            prefs.write(f'\nuser_pref("browser.download.dir", {json.dumps(str(downloads))});\n')
            prefs.write('user_pref("browser.download.folderList", 2);\n')
        driver, warnings = start(profile, geckodriver, "geckodriver.log")
        info = driver.execute_script('''return {version:Services.appinfo.version, engine:Services.search.defaultEngine.name,
          vertical:Services.prefs.getBoolPref("sidebar.verticalTabs"),
          lazy:Services.prefs.getBoolPref("browser.sessionstore.restore_on_demand"),
          sidebarWidth:document.getElementById("sidebar-main").getBoundingClientRect().width,
          color:getComputedStyle(document.documentElement).getPropertyValue("--still-canvas").trim()};''')
        assert info["vertical"] and info["lazy"] and info["engine"] == "Google"
        assert info["color"] == "#1c1c1e" and info["sidebarWidth"] == 0
        check("Firefox / Gecko launches with styled vertical tabs", info)
        RESULTS.append({"environmentWarnings": warnings})

        navigate(driver, "about:home")
        content(driver)
        wait_js(driver, 'return document.readyState === "complete"')
        # about:home may commit after the previous document reports complete.
        wait_js(driver, 'return location.href === "about:home" && [...document.querySelectorAll("input,button,a")].every(e=>!e.getClientRects().length)', 15)
        screenshot(driver, "minimal-home.png")
        check("Empty home has no visible or focusable page controls")
        key(driver, Keys.CONTROL, "t")
        wait_js(driver, 'return gBrowser.tabs.length === 2 && document.activeElement.closest("#urlbar") !== null')
        check("New tab focuses the address bar with sidebar closed")
        key(driver, Keys.CONTROL, "w")
        wait_js(driver, 'return gBrowser.tabs.length === 1')
        driver.find_element(By.ID, "sidebar-button").click()
        wait_js(driver, 'return SidebarController.launcherVisible && document.getElementById("sidebar-main").getBoundingClientRect().width >= 224')
        check("Sidebar toggle exposes vertical tabs")

        navigate(driver, "still browser test")
        target = wait_google_query(driver, "still browser test")
        assert parse_qs(urlparse(target).query).get("q") == ["still browser test"]
        check("Address bar sends keywords to Google", target)

        navigate(driver, "https://example.org/")
        content(driver)
        WebDriverWait(driver, 45).until(lambda d: d.title == "Example Domain" and "documentation examples" in d.find_element(By.TAG_NAME, "body").text)
        check("External HTTPS webpage renders", driver.current_url)

        navigate(driver, base + "/one")
        content(driver)
        WebDriverWait(driver, 30).until(lambda d: d.title == "First page")
        driver.find_element(By.ID, "next").click()
        WebDriverWait(driver, 30).until(lambda d: d.title == "Second page")
        chrome(driver)
        driver.find_element(By.ID, "back-button").click()
        wait_js(driver, 'return gBrowser.currentURI.spec.endsWith("/one")')
        driver.find_element(By.ID, "forward-button").click()
        wait_js(driver, 'return gBrowser.currentURI.spec.endsWith("/two")')
        before = COUNTS["/two"]
        driver.find_element(By.ID, "reload-button").click()
        WebDriverWait(driver, 30).until(lambda _: COUNTS["/two"] > before)
        check("Back, forward and reload buttons work")

        content(driver)
        previous_handles = set(driver.window_handles)
        chrome(driver)
        count = driver.execute_script("return gBrowser.tabs.length")
        driver.find_element(By.ID, "new-tab-button").click()
        wait_js(driver, f"return gBrowser.tabs.length === {count+1}")
        navigate(driver, base + "/other")
        wait_js(driver, 'return gBrowser.selectedBrowser.contentTitle === "Another tab"')
        content(driver)
        new_handle = next(handle for handle in driver.window_handles if handle not in previous_handles)
        driver.switch_to.window(new_handle)
        WebDriverWait(driver, 30).until(lambda d: d.title == "Another tab")
        chrome(driver)
        driver.find_elements(By.CSS_SELECTOR, ".tabbrowser-tab")[0].click()
        wait_js(driver, 'return gBrowser.currentURI.spec.endsWith("/two")')
        driver.find_elements(By.CSS_SELECTOR, ".tabbrowser-tab")[1].click()
        wait_js(driver, 'return gBrowser.currentURI.spec.endsWith("/other")')
        check("New tab button and vertical tab switching work")
        screenshot(driver, "tabs.png")

        driver.find_element(By.ID, "star-button-box").click()
        wait_js(driver, 'return document.getElementById("editBookmarkPanel")?.state === "open"')
        driver.find_element(By.ID, "editBookmarkPanelDoneButton").click()
        saved = driver.execute_async_script('''const done=arguments[arguments.length-1];
          PlacesUtils.bookmarks.fetch({url:arguments[0]}).then(x=>done(x && x.title));''', base + "/other")
        assert saved == "Another tab"
        check("Bookmark star saves the current page", saved)

        key(driver, Keys.CONTROL, "h")
        wait_js(driver, 'return SidebarController.currentID === "viewHistorySidebar"')
        visited = driver.execute_async_script('''const done=arguments[arguments.length-1];
          PlacesUtils.history.hasVisits(arguments[0]).then(done);''', base + "/two")
        assert visited
        screenshot(driver, "history.png")
        key(driver, Keys.CONTROL, "h")
        check("History panel and stored visits work")

        content(driver)
        ActionChains(driver).click(driver.find_element(By.ID, "download")).perform()
        chrome(driver)
        last_download = None
        def download_progress(d):
            nonlocal last_download
            entries = d.execute_async_script('''const done=arguments[arguments.length-1];
              const {Downloads}=ChromeUtils.importESModule("resource://gre/modules/Downloads.sys.mjs");
              Downloads.getList(Downloads.ALL).then(l=>l.getAll()).then(x=>done(x.map(d=>({target:d.target.path,total:d.totalBytes,current:d.currentBytes,succeeded:d.succeeded}))));''')
            if entries != last_download:
                print("Download state", json.dumps(entries), flush=True)
                last_download = entries
            return entries if entries and entries[-1]["current"] > 0 else False
        download_start = WebDriverWait(driver, 25).until(download_progress)
        assert download_start and not download_start[-1]["succeeded"] and download_start[-1]["total"] > 0
        if not driver.execute_script('return document.getElementById("downloadsPanel")?.state === "open"'):
            driver.find_element(By.ID, "downloads-button").click()
        wait_js(driver, 'return document.getElementById("downloadsPanel")?.state === "open"')
        screenshot(driver, "downloading.png")
        key(driver, Keys.ESCAPE)
        DOWNLOAD_CONTINUE.set()
        WebDriverWait(driver, 30).until(lambda _: (downloads / "still-test.txt").exists() and (downloads / "still-test.txt").read_bytes() == PAYLOAD)
        key(driver, Keys.CONTROL, Keys.SHIFT, "y")
        wait_js(driver, 'return !!Services.wm.getMostRecentWindow("Places:Organizer")')
        driver.execute_script('Services.wm.getMostRecentWindow("Places:Organizer").close()')
        check("Download progress, Downloads UI and saved file work", {"bytes": len(PAYLOAD), "observed": download_start})

        chrome(driver)
        # Save /two as the foreground tab and /other as an unloaded restore candidate.
        driver.find_elements(By.CSS_SELECTOR, ".tabbrowser-tab")[0].click()
        wait_js(driver, 'return gBrowser.currentURI.spec.endsWith("/two")')
        driver.execute_async_script('''const done=arguments[arguments.length-1];
          const {TabStateFlusher}=ChromeUtils.importESModule("resource:///modules/sessionstore/TabStateFlusher.sys.mjs");
          Promise.all([...gBrowser.browsers].map(b=>TabStateFlusher.flush(b))).then(()=>done(true));''')
        driver.execute_script('Services.prefs.setStringPref("sidebar.main.tools","history")')
        driver.quit()
        driver = None
        assert (profile / "sessionstore.jsonlz4").exists(), "Firefox did not save its session"
        before_other = COUNTS["/other"]
        subprocess.run([str(ROOT / "linux" / "still"), "--profile", str(profile), "--prepare-only"], check=True, capture_output=True)
        driver, _ = start(profile, geckodriver, "geckodriver-restart.log")
        assert driver.execute_script('return Services.prefs.getStringPref("sidebar.main.tools")') == "history"
        assert driver.execute_script('return SidebarController.launcherVisible')
        check("Launcher keeps the user's sidebar setting and open state after restart")
        wait_js(driver, 'return gBrowser.tabs.length === 2 && gBrowser.currentURI.spec.endsWith("/two")', 45)
        pending = driver.execute_script('return [...gBrowser.tabs].map(t=>({label:t.label,pending:t.hasAttribute("pending")}))')
        assert pending[1]["pending"], pending
        assert COUNTS["/other"] == before_other, "Background restored tab loaded before selection"
        bookmark_restored = driver.execute_async_script('''const done=arguments[arguments.length-1];
          PlacesUtils.bookmarks.fetch({url:arguments[0]}).then(x=>done(x && x.title));''', base + "/other")
        visits_restored = driver.execute_async_script('''const done=arguments[arguments.length-1];
          PlacesUtils.history.hasVisits(arguments[0]).then(done);''', base + "/two")
        assert bookmark_restored == "Another tab" and visits_restored
        check("Restart restores selected tab, bookmarks and history", pending)
        time.sleep(2)
        assert COUNTS["/other"] == before_other
        driver.find_elements(By.CSS_SELECTOR, ".tabbrowser-tab")[1].click()
        WebDriverWait(driver, 30).until(lambda _: COUNTS["/other"] > before_other)
        wait_js(driver, 'return gBrowser.currentURI.spec.endsWith("/other")')
        check("Restored background tab loads only when selected", {"requestsBefore": before_other, "requestsAfter": COUNTS["/other"]})
        screenshot(driver, "restored.png")
        # Simulate upgrading a legacy open-sidebar session, using Firefox's own
        # compressed session file rather than only a synthetic JSON fixture.
        driver.execute_script('Services.prefs.setIntPref("still.uiRevision", 1)')
        driver.quit()
        driver = None
        subprocess.run([str(ROOT / "linux/still"), "--profile", str(profile), "--prepare-only"], check=True, capture_output=True)
        assert (profile / "sessionstore.jsonlz4.before-minimal").exists()
        driver, _ = start(profile, geckodriver, "geckodriver-migration.log")
        wait_js(driver, 'return gBrowser.tabs.length === 2 && gBrowser.currentURI.spec.endsWith("/other")', 45)
        assert not driver.execute_script('return SidebarController.launcherVisible')
        assert driver.execute_script('return Services.prefs.getIntPref("still.uiRevision")') == 2
        check("Legacy compressed session migrates closed without losing tabs")
        screenshot(driver, "minimal-migrated.png")

    except Exception as error:
        RESULTS.append({"passed": False, "error": str(error)})
        if driver:
            try:
                screenshot(driver, "failure.png")
            except Exception:
                pass
        raise
    finally:
        DOWNLOAD_CONTINUE.set()
        if driver:
            driver.quit()
        temporary.cleanup()
        server.shutdown()
        (OUTPUT / "results.json").write_text(json.dumps(RESULTS, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--geckodriver", default=shutil.which("geckodriver") or str(ROOT / ".tools" / "geckodriver"))
    args = parser.parse_args()
    run(args.geckodriver)
