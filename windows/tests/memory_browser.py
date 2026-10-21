#!/usr/bin/env python3
"""Exercise the shipped Gecko modules in an isolated real Firefox runtime.

Runs on Linux with Selenium; verifies Gecko APIs, not ARM64 memory consumption.
Only the moz-src URI of our actor is relocated in the temporary runtime.
"""
import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import shutil
import tempfile
from threading import Thread

from selenium import webdriver
from selenium.webdriver.common.by import By
from selenium.webdriver.firefox.options import Options
from selenium.webdriver.firefox.service import Service
from selenium.webdriver.support.ui import WebDriverWait

ROOT = Path(__file__).resolve().parents[2]
MODULES = ROOT / "windows/gecko/common/files/browser/components/tabbrowser"
PREFIX = "moz-src:///browser/components/tabbrowser/"
RELOCATED = "resource://still-memory/"


class Pages(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_GET(self):
        body = '<!doctype html><title>Memory test</title><input id="draft"><input type="checkbox"><input type="range"><select><option>A</option><option>B</option></select><p>Plain page</p>'
        if self.path == "/frame":
            body = f'<!doctype html><title>Frame test</title><iframe src="http://localhost:{self.server.server_port}/plain"></iframe>'
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.end_headers()
        self.wfile.write(body.encode())


def run(binary, geckodriver, output):
    output.mkdir(parents=True, exist_ok=True)
    checks = []
    server = ThreadingHTTPServer(("127.0.0.1", 0), Pages)
    thread = Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix="still-memory-browser-") as directory:
            work = Path(directory)
            runtime = work / "firefox"
            shutil.copytree(binary.parent, runtime)
            modules = runtime / "still-memory"
            modules.mkdir()
            for name in ("StillTabUnloader.sys.mjs", "StillTabStateChild.sys.mjs"):
                (modules / name).write_text((MODULES / name).read_text().replace(PREFIX, RELOCATED))
            options = Options()
            options.binary_location = str(runtime / binary.name)
            options.add_argument("-headless")
            options.set_preference("browser.shell.checkDefaultBrowser", False)
            options.set_preference("still.memory.autoUnload.enabled", True)
            options.set_preference("still.memory.autoUnload.inactiveMinutes", 5)
            service = Service(str(geckodriver), log_output=str(output / "geckodriver.log"),
                              env=dict(os.environ, MOZ_REMOTE_ALLOW_SYSTEM_ACCESS="1"))
            driver = webdriver.Firefox(options=options, service=service)
            driver.set_script_timeout(20)
            try:
                driver.set_context("chrome")
                driver.execute_script("""
                  const dir = Components.classes['@mozilla.org/file/local;1'].createInstance(Components.interfaces.nsIFile);
                  dir.initWithPath(arguments[0]);
                  Services.io.getProtocolHandler('resource').QueryInterface(Components.interfaces.nsIResProtocolHandler)
                    .setSubstitution('still-memory', Services.io.newFileURI(dir));
                  window.stillMemory = ChromeUtils.importESModule('resource://still-memory/StillTabUnloader.sys.mjs').StillTabUnloader;
                  stillMemory.init();
                """, str(modules))

                def content():
                    driver.set_context("content")

                def chrome(script, *args):
                    driver.set_context("chrome")
                    return driver.execute_script(script, *args)

                def passed(name):
                    checks.append(name)
                    print("PASS", name, flush=True)

                def candidate(route="plain", edit=None):
                    content()
                    driver.switch_to.new_window("tab")
                    handle = driver.current_window_handle
                    driver.get(f"http://127.0.0.1:{server.server_port}/{route}")
                    if edit:
                        edit()
                    chrome("window.memoryCandidate = gBrowser.selectedTab;")
                    content()
                    driver.switch_to.new_window("tab")
                    driver.get("about:blank")
                    chrome("memoryCandidate.updateLastAccessed(Date.now() - 300001);")
                    return handle

                def tick():
                    driver.set_context("chrome")
                    result = driver.execute_async_script("""
                      const done = arguments[arguments.length - 1];
                      stillMemory.tick().then(() => done({pending: memoryCandidate.hasAttribute('pending')}),
                        error => done({error: String(error)}));
                    """)
                    assert "error" not in result, result
                    return result["pending"]

                def remove_candidate():
                    chrome("gBrowser.removeTab(memoryCandidate);")
                    content()
                    driver.switch_to.window(driver.window_handles[0])

                handle = candidate()
                if not tick():
                    details = chrome("""
                      const b = memoryCandidate.linkedBrowser;
                      let eligible, error;
                      try { eligible = stillMemory.eligible(memoryCandidate, gBrowser); } catch (e) { error = String(e); }
                      return {eligible, error, selected: memoryCandidate.selected, age: Date.now()-memoryCandidate.lastAccessed,
                        beforeUnload: b.hasBeforeUnload, remote: b.isRemoteBrowser, attrs: memoryCandidate.getAttributeNames(),
                        frames: stillMemory.frames(b.browsingContext).length};
                    """)
                    driver.set_context("chrome")
                    details["actor"] = driver.execute_async_script("""
                      const done = arguments[arguments.length-1];
                      const global = memoryCandidate.linkedBrowser.browsingContext.currentWindowGlobal;
                      global.getActor('StillTabState').sendQuery('Still:PassesRetentionChecks').then(done, e => done(String(e)));
                    """)
                    raise AssertionError(f"plain inactive tab was not discarded: {details}")
                passed("plain background tab discarded through real Gecko APIs")
                content()
                driver.switch_to.window(handle)
                WebDriverWait(driver, 20).until(lambda d: d.find_elements(By.ID, "draft"))
                passed("discarded tab restores on selection")
                remove_candidate()

                candidate(edit=lambda: driver.execute_script("document.body.append(document.createElement('canvas'))"))
                assert not tick(), "canvas page was discarded"
                passed("canvas page retained")
                remove_candidate()

                candidate(edit=lambda: driver.find_element(By.ID, "draft").send_keys("unsaved draft"))
                assert not tick(), "edited form was discarded"
                passed("trusted form edits retained")
                remove_candidate()

                def edit_frame():
                    driver.switch_to.frame(0)
                    driver.find_element(By.ID, "draft").send_keys("frame draft")
                    driver.switch_to.default_content()

                candidate("frame", edit_frame)
                assert not tick(), "cross-origin frame draft was discarded"
                passed("cross-origin frame draft retained")
                remove_candidate()

                def editor():
                    driver.execute_script("const host = document.createElement('div'); document.body.append(host); host.attachShadow({mode:'closed'}).innerHTML='<div contenteditable>draft</div>';")

                candidate(edit=editor)
                assert not tick(), "closed shadow editor was discarded"
                passed("closed shadow editor retained")
                remove_candidate()

                candidate()
                chrome("memoryCandidate.updateLastAccessed(Date.now() - 299000);")
                assert not tick(), "tab younger than five minutes discarded"
                passed("five-minute retention boundary")
                chrome("memoryCandidate.updateLastAccessed(Date.now() - 300001); Services.prefs.setBoolPref('still.memory.autoUnload.enabled', false);")
                assert not tick(), "disabled feature discarded a tab"
                passed("disable preference takes effect immediately")
                chrome("Services.prefs.setBoolPref('still.memory.autoUnload.enabled', true);")
                chrome("window.originalFlush = gBrowser.prepareDiscardBrowser; gBrowser.prepareDiscardBrowser = async function(tab) { await originalFlush.call(this, tab); if (tab == memoryCandidate) this.selectedTab = tab; };")
                assert not tick(), "tab selected during flush discarded"
                chrome("gBrowser.prepareDiscardBrowser = originalFlush;")
                passed("selection during session flush retained")
            finally:
                driver.quit()
    finally:
        server.shutdown()
        server.server_close()
        thread.join()
        (output / "report.json").write_text(json.dumps({"passed": checks}, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--firefox", type=Path, default=ROOT / "linux/runtime/firefox/firefox")
    parser.add_argument("--geckodriver", type=Path, default=ROOT / ".tools/geckodriver")
    parser.add_argument("--output", type=Path, default=ROOT / "artifacts/windows-memory-checks")
    args = parser.parse_args()
    run(args.firefox.resolve(), args.geckodriver.resolve(), args.output.resolve())
