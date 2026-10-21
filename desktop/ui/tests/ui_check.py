#!/usr/bin/env python3
"""Check the Still interface (built dist, mock backend) in a real browser engine."""
import functools, http.server, json, struct, sys, tempfile, threading, zlib
from pathlib import Path
from selenium import webdriver
from selenium.webdriver.common.by import By
from selenium.webdriver.common.keys import Keys
from selenium.webdriver.common.action_chains import ActionChains
from selenium.webdriver.firefox.options import Options
from selenium.webdriver.firefox.service import Service
from selenium.webdriver.support.ui import WebDriverWait

UI = Path(__file__).resolve().parents[1]
REPO = UI.parents[1]
OUT = REPO / "artifacts" / "desktop-checks"
OUT.mkdir(parents=True, exist_ok=True)

class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *a): pass
handler = functools.partial(Quiet, directory=str(UI / "dist"))
server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
threading.Thread(target=server.serve_forever, daemon=True).start()
URL = f"http://127.0.0.1:{server.server_port}/index.html"
results = []

def check(name, ok=True, detail=None):
    results.append({"check": name, "passed": bool(ok), "detail": detail})
    print("PASS" if ok else "FAIL", name, json.dumps(detail, ensure_ascii=False) if detail is not None else "", flush=True)

def driver_for(dark):
    o = Options()
    o.binary_location = str(REPO / "linux/runtime/firefox/firefox")
    o.add_argument("-headless")
    o.set_preference("ui.systemUsesDarkTheme", 1 if dark else 0)
    o.set_preference("layout.css.prefers-color-scheme.content-override", 0 if dark else 1)
    d = webdriver.Firefox(options=o, service=Service(str(REPO / ".tools/geckodriver"), log_output=str(OUT / "geckodriver.log")))
    d.set_window_size(1440, 900)
    return d

def png(width, height):
    """A diagonal gradient, so a stretched or misplaced picture is visible in screenshots."""
    rows = b"".join(b"\0" + bytes(v for x in range(width) for v in (40 + 180 * x // width, 90 + 120 * y // height, 200)) for y in range(height))
    chunk = lambda kind, data: struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")

FILES = Path(tempfile.mkdtemp(prefix="still-ui-"))
(FILES / "wallpaper.png").write_bytes(png(320, 180))
(FILES / "notes.png").write_text("not a picture")
(FILES / "passwords.csv").write_text("name,url,username,password,note\nx,https://imported.test/,imported,secret,\n")

def js(d, s, *a): return d.execute_script(s, *a)
def wait(d, s, t=10): return WebDriverWait(d, t).until(lambda x: x.execute_script(s))
def ctrl(d, key, shift=False):
    a = ActionChains(d).key_down(Keys.CONTROL)
    if shift: a = a.key_down(Keys.SHIFT)
    a = a.send_keys(key)
    if shift: a = a.key_up(Keys.SHIFT)
    a.key_up(Keys.CONTROL).perform()

for mode in ("light", "dark"):
    d = driver_for(mode == "dark")
    try:
        d.get(URL)
        wait(d, "return window.stillMock && window.stillMock.bounds")
        bg = js(d, "return getComputedStyle(document.body).backgroundColor")
        check(f"[{mode}] canvas color", bg == ("rgb(28, 28, 30)" if mode == "dark" else "rgb(245, 245, 247)"), bg)
        b = js(d, "return window.stillMock.bounds")
        r = js(d, "const r=document.getElementById('content').getBoundingClientRect();return {x:Math.round(r.left),y:Math.round(r.top),width:Math.round(r.width),height:Math.round(r.height)}")
        check(f"[{mode}] page area bounds sent to Rust match the content box", b == r and b["x"] >= 4 and b["y"] >= 40, b)
        hidden = js(d, "return ['downloads-indicator','notice'].map(id=>getComputedStyle(document.getElementById(id)).display)")
        check(f"[{mode}] idle indicators are not rendered", hidden == ["none", "none"], hidden)
        check(f"[{mode}] sidebar starts closed", js(d, "return document.getElementById('sidebar').hidden"))
        check(f"[{mode}] address shows the active page", js(d, "return document.getElementById('address').value") == "https://example.org/")
        d.save_screenshot(str(OUT / f"{mode}-page.png"))

        hidden_work = js(d, """let n = 0; const mo = new MutationObserver(() => n++);
          mo.observe(document.getElementById('list'), {childList: true, subtree: true});
          window.stillMock.tabs[0].title = 'Hidden update'; window.stillMock.publish();
          return new Promise(r => requestAnimationFrame(() => requestAnimationFrame(() => { mo.disconnect(); r(n); })));""")
        check(f"[{mode}] closed sidebar is not re-rendered", hidden_work == 0, hidden_work)
        js(d, "window.stillMock.tabs[0].title = 'Example Domain'; window.stillMock.publish()")
        d.find_element(By.ID, "sidebar-toggle").click()
        wait(d, "return !document.getElementById('sidebar').hidden")
        b2 = js(d, "return window.stillMock.bounds")
        edge = js(d, "return Math.round(document.getElementById('sidebar').getBoundingClientRect().right)")
        check(f"[{mode}] opening sidebar aligns the page with its edge", b2["x"] == edge and b2["x"] > b["x"] and b2["width"] == b["width"] - (b2["x"] - b["x"]), b2)
        rows = js(d, "return [...document.querySelectorAll('#list .row-title')].map(e=>e.textContent)")
        check(f"[{mode}] tab list", rows == ["Example Domain", "New tab", "New tab"], rows)
        d.save_screenshot(str(OUT / f"{mode}-sidebar.png"))

        for sec, expect in (("bookmarks", "Example Domain"), ("history", "Example Domain"), ("downloads", "report.pdf")):
            d.find_element(By.ID, f"seg-{sec}").click()
            got = js(d, "return [...document.querySelectorAll('#list .row-title')].map(e=>e.textContent)")
            check(f"[{mode}] {sec} section", expect in got, got)
            d.save_screenshot(str(OUT / f"{mode}-{sec}.png"))
        d.find_element(By.ID, "seg-tabs").click()

        rebuilds = js(d, """const done = arguments[0];
          let roots = 0, texts = 0;
          const list = document.getElementById('list');
          const row = list.querySelector('.row');
          const mo = new MutationObserver(records => { for (const m of records) { if (m.target === list) roots++; else texts++; } });
          mo.observe(list, {childList: true, subtree: true});
          for (let i = 0; i < 50; i++) { window.stillMock.tabs[0].title = 'Title ' + i; window.stillMock.publish(); }
          return new Promise(r => requestAnimationFrame(() => requestAnimationFrame(() => { mo.disconnect(); r([roots, texts, row === list.querySelector('.row'), row.querySelector('.row-title').textContent]); })));""")
        check(f"[{mode}] 50 updates change one title once without replacing rows", rebuilds == [0, 1, True, "Title 49"], rebuilds)
        same = js(d, """let n = 0; const mo = new MutationObserver(() => n++);
          mo.observe(document.getElementById('list'), {childList: true, subtree: true});
          window.stillMock.state.notice = null; window.stillMock.publish();
          return new Promise(r => requestAnimationFrame(() => requestAnimationFrame(() => { mo.disconnect(); r(n); })));""")
        check(f"[{mode}] unchanged list is not rebuilt", same == 0, same)

        d.find_element(By.ID, "bookmark").click()
        check(f"[{mode}] star toggles bookmark", js(d, "return document.getElementById('bookmark').getAttribute('aria-pressed')") == "false")

        ctrl(d, "t")
        wait(d, "return window.stillMock.tabs.length === 3 && document.activeElement.id === 'address'")
        check(f"[{mode}] Ctrl+T opens a tab and focuses the address")
        check(f"[{mode}] blank tab hides the bookmark star", wait(d, "return getComputedStyle(document.getElementById('bookmark')).display === 'none'"))
        d.find_element(By.ID, "address").send_keys("still browser", Keys.ENTER)
        wait(d, "return window.stillMock.snapshot().url.includes('google.com/search?q=still%20browser')")
        check(f"[{mode}] address bar submits searches")
        ctrl(d, Keys.TAB)
        a1 = js(d, "return window.stillMock.state.active")
        ctrl(d, Keys.TAB, shift=True)
        a2 = js(d, "return window.stillMock.state.active")
        check(f"[{mode}] Ctrl+Tab / Ctrl+Shift+Tab cycle tabs", a1 != a2, [a1, a2])
        ctrl(d, "w")
        check(f"[{mode}] Ctrl+W closes the tab", js(d, "return window.stillMock.tabs.length") == 2)
        ctrl(d, "l")
        check(f"[{mode}] Ctrl+L focuses the address", js(d, "return document.activeElement.id") == "address")
        d.find_element(By.ID, "address").send_keys(Keys.ESCAPE)

        js(d, "window.stillMock.state.notice='Changes could not be saved. Check available disk space.';window.stillMock.publish()")
        check(f"[{mode}] notice shows inline", wait(d, "return !document.getElementById('notice').hidden"))
        d.save_screenshot(str(OUT / f"{mode}-notice.png"))
        d.find_element(By.ID, "notice-dismiss").click()
        check(f"[{mode}] notice dismisses", wait(d, "return document.getElementById('notice').hidden"))

        for w in (732, 1024, 1440):
            d.set_window_size(w, 700)
            over = js(d, "return document.documentElement.scrollWidth > innerWidth || document.querySelector('.toolbar').scrollWidth > document.querySelector('.toolbar').clientWidth")
            check(f"[{mode}] no horizontal overflow at {w}px", not over)
        d.save_screenshot(str(OUT / f"{mode}-narrow.png"))

        js(d, "window.stillMock.tabs[0].crashed = true; window.stillMock.publish()")
        d.find_element(By.ID, "seg-tabs").click()
        crashed = wait(d, "const r = document.querySelector('#list .row.crashed'); return r && r.querySelector('.row-detail').textContent")
        check(f"[{mode}] crashed tab is marked with a recovery hint", crashed == "Stopped working · Reload", crashed)
        js(d, "window.stillMock.tabs[0].crashed = false; window.stillMock.publish()")

        # ---------- Settings ----------
        d.set_window_size(1440, 900)
        S = lambda key: d.find_element(By.CSS_SELECTOR, f'[data-key="{key}"]')
        mock_settings = lambda: js(d, "return window.stillMock.state.settings")
        settle = lambda: js(d, "return new Promise(r => requestAnimationFrame(() => requestAnimationFrame(() => r(true))))")
        d.find_element(By.ID, "settings-toggle").click()
        wait(d, "return !document.getElementById('settings').hidden")
        check(f"[{mode}] gear button opens settings", js(d, "return window.stillMock.state.settingsOpen && document.getElementById('settings-toggle').getAttribute('aria-pressed')") == "true")
        d.save_screenshot(str(OUT / f"{mode}-settings-general.png"))

        S("Startup:Home").click()
        wait(d, "return !!document.querySelector('[data-key=home]')")
        check(f"[{mode}] choosing the home page shows its address field", mock_settings()["Startup"] == "Home")
        S("home").send_keys("just words", Keys.ENTER)
        settle()
        check(f"[{mode}] search text is rejected as a home page", js(d, "return document.querySelector('[data-key=home]').getAttribute('aria-invalid')") == "true" and mock_settings()["HomePage"] == "")
        S("home").send_keys(Keys.CONTROL, "a")
        S("home").send_keys("example.com", Keys.ENTER)
        wait(d, "return window.stillMock.state.settings.HomePage === 'https://example.com/'")
        check(f"[{mode}] home page is saved as a web address", S("home").get_attribute("value") == "https://example.com/")
        S("Startup:Restore").click()
        S("folder").click()
        wait(d, "return !!document.querySelector('[data-key=folder-reset]')")
        check(f"[{mode}] chosen download folder is shown", "Desktop" in d.find_element(By.ID, "settings-body").text)
        S("folder-reset").click()
        wait(d, "return !document.querySelector('[data-key=folder-reset]')")
        S("ask").click()
        check(f"[{mode}] ask-where-to-save switch", wait(d, "return window.stillMock.state.settings.AskDownloadFolder") and S("ask").get_attribute("aria-checked") == "true")
        S("ask").click()

        d.find_element(By.ID, "pane-appearance").click()
        check(f"[{mode}] appearance pane", d.find_element(By.CSS_SELECTOR, ".settings-title").text == "Appearance")
        d.save_screenshot(str(OUT / f"{mode}-settings-appearance.png"))
        canvas = "return getComputedStyle(document.body).backgroundColor"
        S("Theme:Dark").click()
        wait(d, "return document.documentElement.dataset.theme === 'dark'")
        check(f"[{mode}] Dark theme applies", js(d, canvas) == "rgb(28, 28, 30)", js(d, canvas))
        S("Theme:Light").click()
        wait(d, "return document.documentElement.dataset.theme === 'light'")
        check(f"[{mode}] Light theme applies", js(d, canvas) == "rgb(245, 245, 247)", js(d, canvas))
        S("Theme:System").click()
        wait(d, "return !document.documentElement.dataset.theme")
        S("Theme:System").send_keys(Keys.ARROW_RIGHT)
        wait(d, "return window.stillMock.state.settings.Theme === 'Light'")
        settle()
        check(f"[{mode}] arrow keys move through the theme choices and keep focus", js(d, "return document.activeElement.dataset.key") == "Theme:Light")
        S("Theme:System").click()
        S("Accent color:Green").click()
        wait(d, "return document.documentElement.dataset.accent === 'green'")
        accent = js(d, "const cs = getComputedStyle(document.documentElement); return [cs.getPropertyValue('--accent').trim(), cs.getPropertyValue('--accent-green').trim()]")
        check(f"[{mode}] accent color applies", accent[0] == accent[1] and accent[0] != "", accent)
        S("Accent color:Blue").click()
        wait(d, "return !document.documentElement.dataset.accent")
        S("Sidebar at startup:Open").click()
        check(f"[{mode}] sidebar startup choice", wait(d, "return window.stillMock.state.settings.Sidebar === 'Open'"))
        S("Sidebar at startup:Remember").click()

        # New tab picture: chosen with the file picker, checked by the backend.
        picker = lambda: d.find_element(By.CSS_SELECTOR, "input[type=file][accept*=image]")
        check(f"[{mode}] new tab picture starts unset", S("picture").text == "Choose…" and not js(d, "return !!document.querySelector('[data-key=picture-remove]')"))
        picker().send_keys(str(FILES / "wallpaper.png"))
        wait(d, "return /^newtab-\\d+\\.png$/.test(window.stillMock.state.settings.NewTabImage)")
        thumb = wait(d, "const t = document.querySelector('.picture-thumb'); return t && t.style.backgroundImage.includes('blob:') && t.style.backgroundImage")
        check(f"[{mode}] chosen picture is previewed in settings", S("picture").text == "Change…" and bool(thumb), thumb)
        d.save_screenshot(str(OUT / f"{mode}-settings-newtab-picture.png"))
        check(f"[{mode}] settings over a new tab hide the picture", js(d, "return window.stillMock.snapshot().url === '' || !document.getElementById('content').classList.contains('picture')"))
        was_active = js(d, "return window.stillMock.state.active")
        js(d, "const m = window.stillMock; m.state.active = m.tabs.find(t => t.url === 'about:blank').id; m.state.settingsOpen = false; m.publish()")
        shown = wait(d, "const c = document.getElementById('content'); return c.classList.contains('picture') && getComputedStyle(c).backgroundImage")
        check(f"[{mode}] new tab shows the picture behind the content area", "blob:" in shown and js(d, "return getComputedStyle(document.getElementById('content')).backgroundSize") == "cover", shown)
        d.save_screenshot(str(OUT / f"{mode}-newtab-picture.png"))
        js(d, "const m = window.stillMock; m.state.active = m.tabs.find(t => t.url !== 'about:blank').id; m.publish()")
        check(f"[{mode}] a page tab does not show the picture", wait(d, "return !document.getElementById('content').classList.contains('picture')"))
        js(d, "const m = window.stillMock; m.state.active = arguments[0]; m.state.settingsOpen = true; m.publish()", was_active)
        wait(d, "return !document.getElementById('settings').hidden")
        before = js(d, "return window.stillMock.state.settings.NewTabImage")
        picker().send_keys(str(FILES / "notes.png"))
        notice = wait(d, "return !document.getElementById('notice').hidden && document.getElementById('notice-text').textContent")
        check(f"[{mode}] a file that is not a picture is refused with a notice", "cannot be used" in notice and js(d, "return window.stillMock.state.settings.NewTabImage") == before, notice)
        js(d, "window.stillMock.state.notice = null; window.stillMock.publish()")
        S("picture-remove").click()
        wait(d, "return window.stillMock.state.settings.NewTabImage === ''")
        check(f"[{mode}] removing the picture clears the preview", wait(d, "const t = document.querySelector('.picture-thumb'); return t && !t.style.backgroundImage") and S("picture").text == "Choose…")

        d.find_element(By.ID, "pane-search").click()
        S("Search engine:DuckDuckGo").click()
        wait(d, "return window.stillMock.state.settings.Search === 'DuckDuckGo'")
        d.find_element(By.ID, "address").send_keys("still browser", Keys.ENTER)
        wait(d, "return window.stillMock.snapshot().url.startsWith('https://duckduckgo.com/?q=still%20browser')")
        check(f"[{mode}] searches use the chosen engine and leave settings", not js(d, "return window.stillMock.state.settingsOpen"))
        d.find_element(By.ID, "settings-toggle").click()
        wait(d, "return !document.getElementById('settings').hidden")
        check(f"[{mode}] settings reopen on the last pane", d.find_element(By.CSS_SELECTOR, ".settings-title").text == "Search")
        S("Search engine:Google").click()

        d.find_element(By.ID, "pane-privacy").click()
        S("keep-history").click()
        check(f"[{mode}] history switch", wait(d, "return window.stillMock.state.settings.KeepHistory === false"))
        S("keep-history").click()
        S("clear-site").click()
        S("clear").click()
        wait(d, "return !!document.querySelector('[data-key=clear-confirm]')")
        check(f"[{mode}] clearing asks for confirmation", js(d, "return document.activeElement.dataset.key") == "clear-cancel")
        d.save_screenshot(str(OUT / f"{mode}-settings-privacy.png"))
        ActionChains(d).send_keys(Keys.ESCAPE).perform()
        check(f"[{mode}] Escape cancels the confirmation first", wait(d, "return !document.querySelector('[data-key=clear-confirm]')") and js(d, "return window.stillMock.state.settingsOpen"))
        S("clear").click()
        S("clear-confirm").click()
        cleared = wait(d, "return window.stillMock.state.cleared")
        check(f"[{mode}] chosen data kinds are cleared", cleared == {"history": True, "siteData": True, "cache": True}, cleared)
        js(d, "window.stillMock.state.notice = null; window.stillMock.publish()")
        # Software update (the startup check found nothing: the mock site offers no version yet).
        d.find_element(By.ID, "pane-general").click()
        check(f"[{mode}] no update button without an update", js(d, "return document.getElementById('update-button').hidden"))
        S("update-check").click()
        check(f"[{mode}] manual check reports up to date", wait(d, "return document.body.textContent.includes('Still is up to date.')"))
        js(d, "window.stillMock.state.update = 'fail'")
        S("update-check").click()
        check(f"[{mode}] a failed manual check says so", wait(d, "return document.body.textContent.includes('Could not check for updates')"))
        js(d, "window.stillMock.state.update = '0.3.0'")
        S("update-check").click()
        wait(d, "return !!document.querySelector('[data-key=update-install]')")
        check(f"[{mode}] a newer version shows in settings and the toolbar",
              "Version 0.3.0 is available." in js(d, "return document.getElementById('settings-body').textContent")
              and js(d, "return !document.getElementById('update-button').hidden && document.getElementById('update-button').title") == "Install Still 0.3.0 and restart")
        d.save_screenshot(str(OUT / f"{mode}-update-available.png"))
        js(d, "window.stillMock.state.installing = 'fail'")
        d.find_element(By.ID, "update-button").click()
        check(f"[{mode}] a failed install offers a retry", wait(d, "return document.getElementById('update-label').textContent === 'Retry update'"))
        js(d, "window.stillMock.state.installing = null")
        S("update-install").click()
        check(f"[{mode}] installing hands over to the backend",
              wait(d, "return window.stillMock.state.installing === '0.3.0' && document.getElementById('update-button').disabled"))
        d.find_element(By.ID, "pane-privacy").click()
        for key in ("clear-history", "clear-site", "clear-cache"):
            S(key).click()
        check(f"[{mode}] Clear is disabled with nothing selected", not S("clear").is_enabled())

        # Saved passwords: sites and user names only, removable one by one.
        d.find_element(By.ID, "pane-passwords").click()
        labels = js(d, "return [...document.querySelectorAll('#settings-body .setting-label')].map(e => e.textContent)")
        check(f"[{mode}] saved passwords list their sites", labels[:3] == ["example.org", "example.org", "other.test"], labels)
        S("remove-login:other.test:you").click()
        check(f"[{mode}] a saved password can be removed", wait(d, "return window.stillMock.state.logins.length === 2 && !document.querySelector('[data-key=\"remove-login:other.test:you\"]')"))
        d.save_screenshot(str(OUT / f"{mode}-settings-passwords.png"))

        # Import from a browser found on this PC, and from an exported file.
        d.find_element(By.ID, "pane-import").click()
        wait(d, "return !!document.querySelector('[data-key=import-start]')")
        check(f"[{mode}] Chromium browsers offer no cookies", not js(d, "return !!document.querySelector('[data-key=import-cookies]')"))
        js(d, "[...document.querySelectorAll('[data-key]')].find(e => e.dataset.key === arguments[0]).click()", "Browser:C:\\Firefox\\abc")
        wait(d, "return !!document.querySelector('[data-key=import-cookies]')")
        S("import-cookies").click()
        d.save_screenshot(str(OUT / f"{mode}-settings-import.png"))
        S("import-start").click()
        imported = wait(d, "return window.stillMock.state.imported")
        check(f"[{mode}] import sends the browser and the chosen kinds", imported == {"id": "C:\\Firefox\\abc", "bookmarks": True, "history": True, "cookies": True}, imported)
        notice = wait(d, "return !document.getElementById('notice').hidden && document.getElementById('notice-text').textContent")
        check(f"[{mode}] the result is reported", "from Firefox" in notice, notice)
        d.find_element(By.CSS_SELECTOR, "input[type=file][accept*=csv]").send_keys(str(FILES / "passwords.csv"))
        notice = wait(d, "const t = document.getElementById('notice-text').textContent; return window.stillMock.state.logins.length === 3 && t.includes('password') && t")
        check(f"[{mode}] a passwords file is imported", notice == "Imported 1 password.", notice)
        for key in ("import-bookmarks", "import-history", "import-cookies"):
            S(key).click()
        check(f"[{mode}] Import is disabled with nothing selected", not S("import-start").is_enabled())
        S("import-bookmarks").click()
        S("import-history").click()
        js(d, "window.stillMock.state.notice = null; window.stillMock.publish()")

        # The key in the address bar fills a saved login; several logins become choices.
        def go(url):
            field = d.find_element(By.ID, "address")
            field.click()
            settle()  # Focus selects the whole address a frame later; typing then replaces it.
            field.send_keys(url, Keys.ENTER)
        go("https://example.org/")
        wait(d, "return !document.getElementById('fill-login').hidden")
        settle()
        d.find_element(By.ID, "fill-login").click()
        wait(d, "return !document.getElementById('login-picker').hidden")
        choices = js(d, "return [...document.querySelectorAll('.login-choice')].map(e => e.textContent)")
        check(f"[{mode}] several logins are offered by user name", choices == ["me@example.org", "work@example.org"], choices)
        check(f"[{mode}] the first login has focus", js(d, "return document.activeElement.textContent") == "me@example.org")
        d.save_screenshot(str(OUT / f"{mode}-login-picker.png"))
        ActionChains(d).send_keys(Keys.ESCAPE).perform()
        check(f"[{mode}] Escape closes the choice", wait(d, "return document.getElementById('login-picker').hidden && document.activeElement.id === 'fill-login'"))
        d.find_element(By.ID, "fill-login").click()
        ActionChains(d).send_keys(Keys.ARROW_RIGHT, Keys.ENTER).perform()
        check(f"[{mode}] the chosen login is filled", wait(d, "return window.stillMock.state.filled === 1 && document.getElementById('login-picker').hidden"))
        go("https://nologin.test/")
        check(f"[{mode}] no key on a site without logins", wait(d, "return document.getElementById('fill-login').hidden"))
        d.find_element(By.ID, "settings-toggle").click()
        wait(d, "return !document.getElementById('settings').hidden")

        d.find_element(By.ID, "pane-shortcuts").click()
        keys = js(d, "return document.querySelectorAll('#settings-body .keys').length")
        check(f"[{mode}] shortcut list", keys >= 6, keys)
        d.find_element(By.ID, "pane-shortcuts").send_keys(Keys.ARROW_UP)
        check(f"[{mode}] arrow keys move between categories", js(d, "return document.activeElement.id") == "pane-import")
        ActionChains(d).send_keys(Keys.ESCAPE).perform()
        check(f"[{mode}] Escape closes settings", wait(d, "return document.getElementById('settings').hidden && !window.stillMock.state.settingsOpen"))
        ctrl(d, ",")
        check(f"[{mode}] Ctrl+, opens settings", wait(d, "return !document.getElementById('settings').hidden"))
        ctrl(d, ",")
        check(f"[{mode}] Ctrl+, closes settings", wait(d, "return document.getElementById('settings').hidden"))
        d.find_element(By.ID, "settings-toggle").click()
        wait(d, "return !document.getElementById('settings').hidden")
        js(d, "document.querySelector('#list .row').click()")
        check(f"[{mode}] choosing a tab leaves settings", wait(d, "return document.getElementById('settings').hidden"))

        d.find_element(By.ID, "settings-toggle").click()
        d.find_element(By.ID, "pane-general").click()
        S("Startup:Home").click()
        for w in (732, 1024):
            d.set_window_size(w, 700)
            settle()
            over = js(d, "const b = document.getElementById('settings-body'); return document.documentElement.scrollWidth > innerWidth || b.scrollWidth > b.clientWidth")
            check(f"[{mode}] settings fit at {w}px with the sidebar open", not over)
        d.save_screenshot(str(OUT / f"{mode}-settings-narrow.png"))
        S("Startup:Restore").click()
        d.set_window_size(1440, 900)
        d.find_element(By.ID, "settings-toggle").click()

        before_url = js(d, "return document.getElementById('address').value")
        js(d, "const old = window.stillMock.snapshot(); old.revision = 0; old.url = 'https://stale.test/'; old.tabs = []; window.stillMock.emit(old)")
        settle()
        check(f"[{mode}] stale delivery cannot roll back the current session", js(d, "return document.getElementById('address').value") == before_url and js(d, "return document.querySelectorAll('#list [data-tab-id]').length") > 0)

        # Stable DOM and constant work for a one-row update with many tabs.
        perf = js(d, """const mock = window.stillMock;
          for (let i = 0; i < 1000; i++) mock.tabs.push({id: 10000+i, title:'Background '+i, url:'https://example.org/'+i, loading:false, crashed:false});
          mock.publish();
          return new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(() => {
            const list = document.getElementById('list');
            const before = [...list.children];
            const focused = list.querySelector('.trailing'); focused.focus();
            let roots=0, texts=0, themes=0;
            const mo = new MutationObserver(records => { for (const m of records) { if(m.target===list) roots++; else texts++; } });
            mo.observe(list, {childList:true, subtree:true});
            const theme = new MutationObserver(records => themes += records.length);
            theme.observe(document.documentElement, {attributes:true});
            const start = performance.now();
            mock.tabs[mock.tabs.length-1].title = 'Updated background'; mock.publish();
            requestAnimationFrame(() => requestAnimationFrame(() => {
              mo.disconnect(); theme.disconnect();
              resolve({roots,texts,themes, stable:before.every((node,i)=>node===list.children[i]),
                focused:document.activeElement===focused, rows:list.children.length, frameMs:performance.now()-start});
            }));
          })));""")
        check(f"[{mode}] 1000 background tabs update one text node, retaining rows and focus", perf["roots"] == 0 and perf["texts"] == 1 and perf["themes"] == 0 and perf["stable"] and perf["focused"], perf)
        js(d, "window.stillMock.tabs.splice(-1000); window.stillMock.publish()")
        settle()
        check(f"[{mode}] closing background tabs releases their rows", js(d, "return document.querySelectorAll('#list [data-tab-id]').length === window.stillMock.tabs.length"))

        icons = js(d, "return [...document.querySelectorAll('button')].filter(b=>!b.textContent.trim() && !(b.getAttribute('aria-label')||b.title)).map(b=>b.id)")
        check(f"[{mode}] every icon button has an accessible name", not icons, icons)

        # Exercise the compact layout with real content and every optional toolbar control.
        js(d, """const m = window.stillMock;
          m.tabs[0].title = 'A very long page title — 長いページタイトル '.repeat(8);
          m.state.notice = 'A long notification that must not push window controls off screen. '.repeat(5);
          m.state.settings.Startup = 'Home';
          m.state.settings.DownloadFolder = 'C:/Users/Example/Downloads/' + 'Long folder name/'.repeat(8);
          m.publish();""")
        settle()
        for width, height in ((1440, 900), (1024, 768), (732, 500)):
            d.set_window_size(width, height)
            for sidebar_open in (False, True):
                if js(d, "return !document.getElementById('sidebar').hidden") != sidebar_open:
                    d.find_element(By.ID, "sidebar-toggle").click()
                settle()
                tag = f"[{mode} {width}x{height} sidebar={sidebar_open}]"
                # Force the download indicator on as well, independently of mock download state.
                js(d, "document.getElementById('downloads-indicator').hidden = false")
                layout = js(d, """const bar = document.querySelector('.toolbar');
                  const nodes = [...bar.children]; const rects = nodes.map(e=>e.getBoundingClientRect());
                  const r = document.getElementById('content').getBoundingClientRect();
                  return {fits: rects.every((r,i)=>r.left>=0 && r.right<=innerWidth && (!i || r.left>=rects[i-1].right)),
                    bounds: JSON.stringify(window.stillMock.bounds) === JSON.stringify({x:Math.round(r.x),y:Math.round(r.y),width:Math.round(r.width),height:Math.round(r.height)}),
                    tooltip: document.querySelector('#list [data-tab-id]')?.title.includes('長いページタイトル')};""")
                check(f"{tag} toolbar fits and page bounds stay synchronized", layout["fits"] and layout["bounds"], layout)
                if sidebar_open:
                    check(f"{tag} long tab title remains available in tooltip", layout["tooltip"])
                if js(d, "return document.getElementById('settings').hidden"):
                    d.find_element(By.ID, "settings-toggle").click()
                for pane in ("general", "appearance", "search", "privacy", "shortcuts"):
                    d.find_element(By.ID, f"pane-{pane}").click()
                    settle()
                    overflow = js(d, """const body = document.getElementById('settings-body');
                      return [...body.querySelectorAll('.setting-row, .field, .theme-cards, .choice, .keys')]
                        .filter(e=>e.scrollWidth>e.clientWidth+1).map(e=>e.className);""")
                    check(f"{tag} {pane} controls fit without horizontal overflow", not overflow, overflow)
                    if sidebar_open and pane in ("general", "appearance"):
                        d.save_screenshot(str(OUT / f"{mode}-{width}-compact-{pane}.png"))
                d.find_element(By.ID, "settings-toggle").click()
            d.save_screenshot(str(OUT / f"{mode}-{width}-compact-sidebar.png"))
    finally:
        d.quit()

(OUT / "ui-results.json").write_text(json.dumps(results, indent=2, ensure_ascii=False))
failed = [r for r in results if not r["passed"]]
print(f"{len(results) - len(failed)}/{len(results)} passed")
sys.exit(1 if failed else 0)
