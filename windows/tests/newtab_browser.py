#!/usr/bin/env python3
"""Exercise Windows' AutoConfig in an isolated real Firefox runtime.

On Linux, for example:
  xvfb-run -a .venv-linux/bin/python windows/tests/newtab_browser.py

Uses selenium from linux/tests/requirements.txt and .tools/geckodriver.
The runtime is copied before adding Windows' AutoConfig; existing browser
files and profiles are never modified. This checks Gecko UI behavior, not
Windows installer execution.
"""
import argparse
from contextlib import suppress
import json
import os
from pathlib import Path
import shutil
import tempfile
import zipfile

from selenium import webdriver
from selenium.webdriver import ActionChains
from selenium.webdriver.common.by import By
from selenium.webdriver.common.keys import Keys
from selenium.webdriver.firefox.options import Options
from selenium.webdriver.firefox.service import Service
from selenium.webdriver.support.ui import WebDriverWait

ROOT = Path(__file__).resolve().parents[2]
START = "chrome://still/content/newtab.html"


def wait(driver, script):
    return WebDriverWait(driver, 30).until(lambda d: d.execute_script(script))


def new_tab(driver):
    driver.set_context("content")
    handles = set(driver.window_handles)
    driver.set_context("chrome")
    count = driver.execute_script("return gBrowser.tabs.length")
    ActionChains(driver).key_down(Keys.CONTROL).send_keys("t").key_up(Keys.CONTROL).perform()
    wait(driver, f"return gBrowser.tabs.length === {count + 1} && gBrowser.currentURI.spec !== 'about:blank' && !gBrowser.selectedTab.hasAttribute('busy')")
    driver.set_context("content")
    handle = WebDriverWait(driver, 30).until(lambda d: set(d.window_handles) - handles)
    driver.switch_to.window(handle.pop())
    driver.set_context("chrome")
    return driver.execute_script("return gBrowser.currentURI.spec")


def choose_mode(driver, enabled):
    driver.set_context("chrome")
    driver.execute_script("openTrustedLinkIn('about:preferences#home', 'current')")
    driver.set_context("content")
    menu = WebDriverWait(driver, 30).until(lambda d: d.find_element(By.ID, "newTabMode"))
    WebDriverWait(driver, 30).until(lambda d: menu.is_displayed() and menu.is_enabled())
    menu.click()
    menu.send_keys(Keys.HOME if enabled else Keys.END, Keys.ENTER)
    driver.set_context("chrome")
    wait(driver, f"return Services.prefs.getBoolPref('browser.newtabpage.enabled') === {str(enabled).lower()}")


def run(binary, geckodriver, output):
    output.mkdir(parents=True, exist_ok=True)
    (output / "report.json").unlink(missing_ok=True)
    checks = []

    def passed(name):
        checks.append(name)
        print("PASS", name, flush=True)

    with tempfile.TemporaryDirectory(prefix="still-newtab-browser-") as directory:
        work = Path(directory)
        runtime = work / "firefox"
        shutil.copytree(binary.parent, runtime)
        shutil.copyfile(ROOT / "windows/firefox/still.cfg", runtime / "still.cfg")
        shutil.copyfile(ROOT / "windows/firefox/defaults/pref/still-autoconfig.js",
                        runtime / "defaults/pref/still-autoconfig.js")
        shutil.copytree(ROOT / "windows/firefox/still", runtime / "still", dirs_exist_ok=True)
        extension = work / "newtab-test.xpi"
        with zipfile.ZipFile(extension, "w") as bundle:
            bundle.writestr("manifest.json", json.dumps({
                "manifest_version": 2, "name": "Still new tab regression fixture", "version": "1.0",
                "browser_specific_settings": {"gecko": {"id": "newtab-regression@still.test"}},
                "chrome_url_overrides": {"newtab": "newtab.html"},
            }))
            bundle.writestr("newtab.html", "<!doctype html><title>Extension new tab</title><h1>Extension page</h1>")

        for initial in (True, False):
            for name in (f"failure-{initial}.html", f"failure-{initial}.png", f"geckodriver-{initial}.log"):
                (output / name).unlink(missing_ok=True)
            profile = work / f"profile-{initial}"
            profile.mkdir()
            options = Options()
            options.binary_location = str(runtime / binary.name)
            options.add_argument("-profile")
            options.add_argument(str(profile))
            options.set_preference("browser.newtabpage.enabled", initial)
            options.set_preference("browser.shell.checkDefaultBrowser", False)
            options.set_preference("browser.startup.homepage_override.mstone", "ignore")
            # Keep Firefox's legacy Home dropdown available in newer ESRs.
            options.set_preference("browser.settings-redesign.enabled", False)
            options.set_preference("still.windowsLayout", 1)
            service = Service(str(geckodriver), log_output=str(output / f"geckodriver-{initial}.log"),
                              env=dict(os.environ, MOZ_REMOTE_ALLOW_SYSTEM_ACCESS="1"))
            driver = webdriver.Firefox(options=options, service=service)
            try:
                driver.set_window_size(1280, 900)
                driver.set_context("chrome")
                wait(driver, "return gBrowser && gBrowser.tabs.length > 0")
                assert new_tab(driver) == (START if initial else "about:newtab")
                passed(f"startup with enabled={initial}")

                for enabled in (False, True, False, True):
                    choose_mode(driver, enabled)
                    actual = new_tab(driver)
                    assert actual == (START if enabled else "about:newtab"), (enabled, actual)
                    driver.set_context("content")
                    if enabled:
                        WebDriverWait(driver, 30).until(lambda d: d.find_element(By.ID, "search-form"))
                    else:
                        assert driver.execute_script("return document.body.children.length") == 0
                    driver.save_screenshot(str(output / f"initial-{initial}-enabled-{enabled}.png"))
                    passed(f"click setting then Ctrl+T: initial={initial}, enabled={enabled}")

                for removal_enabled in (True, False):
                    addon = driver.install_addon(str(extension), temporary=True)
                    driver.set_context("chrome")
                    wait(driver, "return AboutNewTab.newTabURL.startsWith('moz-extension://')")
                    assert new_tab(driver).startswith("moz-extension://")
                    # Change the pref directly while the extension controls its UI.
                    for enabled in (False, True, removal_enabled):
                        driver.execute_script("Services.prefs.setBoolPref('browser.newtabpage.enabled', arguments[0])", enabled)
                        assert new_tab(driver).startswith("moz-extension://")
                    driver.uninstall_addon(addon)
                    expected = START if removal_enabled else "about:newtab"
                    wait(driver, f"return AboutNewTab.newTabURL === '{expected}'")
                    assert new_tab(driver) == expected
                    driver.execute_script("Services.prefs.setBoolPref('browser.newtabpage.enabled', true)")
                    assert new_tab(driver) == START
                    passed(f"extension preserves control; uninstall honors enabled={removal_enabled}: initial={initial}")
            except Exception as error:
                (output / "report.json").write_text(json.dumps({"passed": checks, "failed": str(error)}, indent=2) + "\n")
                with suppress(Exception):
                    driver.set_context("content")
                    (output / f"failure-{initial}.html").write_text(driver.page_source)
                    driver.save_screenshot(str(output / f"failure-{initial}.png"))
                raise
            finally:
                driver.quit()

    (output / "report.json").write_text(json.dumps({"passed": checks}, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--firefox", type=Path, default=ROOT / "linux/runtime/firefox/firefox")
    parser.add_argument("--geckodriver", type=Path, default=ROOT / ".tools/geckodriver")
    parser.add_argument("--output", type=Path, default=ROOT / "artifacts/windows-newtab-checks")
    args = parser.parse_args()
    run(args.firefox.resolve(), args.geckodriver.resolve(), args.output.resolve())
