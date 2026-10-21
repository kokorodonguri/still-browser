#!/usr/bin/env python3
"""Check actual Firefox light/dark surfaces, focus visibility, and home readiness."""
import json
from pathlib import Path
import subprocess
import tempfile
from selenium.webdriver.common.by import By
from verify import ROOT, OUTPUT, start, chrome, content, navigate, wait_js, screenshot

results = []
for mode, value, background in (("light", 0, "rgb(245, 245, 247)"), ("dark", 1, "rgb(28, 28, 30)")):
    with tempfile.TemporaryDirectory(prefix="still-appearance-") as directory:
        profile = Path(directory)
        subprocess.run([str(ROOT / "linux/still"), "--profile", str(profile), "--prepare-only"], check=True, capture_output=True)
        with (profile / "user.js").open("a") as prefs:
            prefs.write(f'\nuser_pref("ui.systemUsesDarkTheme", {value});\nuser_pref("browser.theme.content-theme", {2 if value == 0 else 0});\nuser_pref("browser.theme.toolbar-theme", {2 if value == 0 else 0});\n')
        driver = None
        try:
            driver, _ = start(profile, str(ROOT / ".tools/geckodriver"), f"appearance-{mode}.log")
            navigate(driver, "about:home")
            content(driver)
            wait_js(driver, 'return document.readyState === "complete"', 60)
            state = driver.execute_script('''return {background:getComputedStyle(document.body).backgroundColor,
                image:getComputedStyle(document.body).backgroundImage,
                controls:[...document.querySelectorAll('input,button,a')].filter(e=>e.getClientRects().length).length,
                overflow:document.documentElement.scrollWidth>innerWidth};''')
            assert state['background'] == background, state
            assert state['image'] == 'none' and not state['overflow'] and state['controls'] == 0, state
            chrome(driver)
            driver.find_element(By.ID, "urlbar-input").click()
            focus = driver.execute_script('return getComputedStyle(document.getElementById("urlbar-background")).outlineWidth')
            assert focus == "2px", focus
            assert not driver.execute_script('return SidebarController.launcherVisible')
            material = driver.execute_script('''const s=getComputedStyle(document.getElementById('sidebar-main'));
                return {image:s.backgroundImage, blur:s.backdropFilter};''')
            assert material['image'] == 'none' and material['blur'] == 'blur(24px)', material
            screenshot(driver, f"minimal-{mode}-focus.png")
            content(driver)
            driver.execute_script('document.activeElement.blur()')
            screenshot(driver, f"minimal-{mode}.png")
            results.append({'mode':mode,'passed':True,'home':state,'sidebar':material})
        finally:
            if driver:
                driver.quit()
            (OUTPUT / 'minimal-appearance.json').write_text(json.dumps(results, indent=2))
print(json.dumps(results, indent=2))
