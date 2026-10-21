const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const source = fs.readFileSync(path.join(__dirname, "../firefox/still/content/settings.js"), "utf8");

class Element {
  constructor() {
    this.value = "";
    this.checked = false;
    this.disabled = true;
    this.hidden = true;
    this.valid = true;
    this.listeners = new Map();
    this.children = [];
    this.attributes = new Map();
    this.textContent = "";
  }
  addEventListener(name, callback) { this.listeners.set(name, callback); }
  setAttribute(name, value) { this.attributes.set(name, value); }
  removeAttribute(name) { this.attributes.delete(name); }
  reportValidity() { return this.valid; }
  focus() { this.focused = true; }
  replaceChildren(...children) {
    this.children = children;
    this.value = (children.find((child) => child.selected) || children[0])?.value || "";
  }
  async send(name, properties = {}) {
    const event = { ...properties, prevented: false, preventDefault() { this.prevented = true; } };
    assert.ok(this.listeners.has(name), `Missing ${name} handler`);
    await this.listeners.get(name)(event);
    return event;
  }
}

async function loadPage(initialPrefs = {}, { launcher = "C:/Still/still.exe", failSearch = false, enableTheme = async () => {} } = {}) {
  const elements = new Map();
  const groups = Array.from({ length: 6 }, () => new Element());
  const values = new Map(Object.entries(initialPrefs));
  const calls = { saved: 0, native: [], launches: [], themes: [], search: [] };
  const errors = [];
  const listeners = new Map();
  const feedback = { bottom: 60 };
  const headings = ["一般 起動時に開くもの", "外観 テーマ", "検索 検索候補", "プライバシー 履歴", "引き継ぎ ブックマーク", "情報 アップデート"];
  const keywords = ["home", "theme 壁紙", "search", "history", "import お気に入り", "version"];
  const sections = ["general", "appearance", "search", "privacy", "import", "information"].map((id, index) => Object.assign(new Element(), {
    id, hidden: false, textContent: headings[index], dataset: { keywords: keywords[index] },
    top: 200 + index * 300, getBoundingClientRect() { return { top: this.top }; },
  }));
  const links = sections.map(({ id }) => Object.assign(new Element(), { hash: `#${id}` }));
  let themesInstalled = false;
  const engines = [{ id: "google", name: "Google" }, { id: "duck", name: "DuckDuckGo" }];
  const element = (id) => {
    if (!elements.has(id)) {
      const control = new Element();
      control.tagName = ["theme", "engine", "startup"].includes(id) ? "SELECT" : ["default-browser", "choose-image", "remove-image", "update", "clear-data", "passwords", "import-browser"].includes(id) ? "BUTTON" : "INPUT";
      control.disabled = id == "engine";
      elements.set(id, control);
    }
    return elements.get(id);
  };
  class File {
    constructor(name) { this.path = name; }
    get parent() { return new File(this.path.slice(0, this.path.lastIndexOf("/"))); }
    clone() { return new File(this.path); }
    append(name) { this.path += `/${name}`; }
    exists() { return this.path === launcher; }
    isFile() { return this.exists(); }
  }
  const browser = {
    switchToTabHavingURI(uri, openNew) { calls.native.push(["tab", uri, openNew]); },
    openPreferences(pane) { calls.native.push(["preferences", pane]); },
  };
  const viewport = {
    browsingContext: { topChromeWindow: browser },
    innerHeight: 800, scrollY: 0,
    addEventListener(name, action) { listeners.set(name, action); },
    requestAnimationFrame(action) { action(); },
  };
  const get = (name, fallback) => values.has(name) ? values.get(name) : fallback;
  const modules = {
    "moz-src:///toolkit/components/search/SearchService.sys.mjs": {
      SearchService: {
        CHANGE_REASON: { USER: "user" },
        async init() { if (failSearch) throw new Error("Search unavailable"); },
        async getVisibleEngines() { return engines; },
        async getDefault() { return engines[1]; },
        async setDefault(engine, reason) { calls.search.push([engine.id, reason]); },
      },
    },
    "resource:///modules/Sanitizer.sys.mjs": {
      Sanitizer: { async showUI(parent) { assert.equal(parent, browser); calls.native.push(["sanitize"]); } },
    },
    "resource:///modules/BuiltInThemes.sys.mjs": {
      BuiltInThemes: { async ensureBuiltInThemes() { themesInstalled = true; } },
    },
    "resource://gre/modules/AddonManager.sys.mjs": {
      AddonManager: {
        async getAddonByID(id) {
          // A fresh profile has only the active theme installed initially.
          if (id !== "default-theme@mozilla.org" && !themesInstalled) return null;
          return { async enable() { calls.themes.push(id); await enableTheme(); } };
        },
      },
    },
  };
  vm.runInNewContext(source, {
    document: {
      getElementById: element,
      createElement() { return new Element(); },
      querySelectorAll(selector) { return selector === "fieldset" ? groups : selector === "nav a" ? links : selector === "main section" ? sections : []; },
      querySelector() { return { getBoundingClientRect() { return feedback; } }; },
      documentElement: { scrollHeight: 2400 },
    },
    window: viewport,
    location: { hash: "" },
    console: { error(error) { errors.push(error); } },
    ChromeUtils: {
      importESModule(uri) {
        assert.ok(modules[uri], `Unexpected module: ${uri}`);
        return modules[uri];
      },
    },
    Components: {
      interfaces: {},
      classes: {
        "@mozilla.org/process/util;1": {
          createInstance() {
            let executable;
            return {
              init(file) { executable = file.path; },
              runwAsync(args, count) { calls.launches.push({ executable, args: Array.from(args), count }); },
            };
          },
        },
      },
    },
    Services: {
      appinfo: { version: "153.5.0" },
      dirsvc: { get() { return new File("C:/Still/firefox"); } },
      prefs: {
        getIntPref: get, getStringPref: get, getBoolPref: get,
        setIntPref(name, value) { values.set(name, value); },
        setStringPref(name, value) { values.set(name, value); },
        setBoolPref(name, value) { values.set(name, value); },
        clearUserPref(name) { values.delete(name); },
        savePrefFile() { calls.saved += 1; },
      },
    },
  }, { filename: "settings.js" });
  // loadSearch runs independently of the synchronous page initialization.
  await new Promise(setImmediate);
  return { element, groups, values, calls, errors, listeners, sections, links, viewport, feedback };
}

test("initial controls reflect saved Firefox preferences and installed versions", async () => {
  const page = await loadPage({
    "browser.startup.page": 1,
    "browser.startup.homepage": "https://example.org/",
    "browser.theme.toolbar-theme": 0,
    "browser.search.suggest.enabled": false,
    "places.history.enabled": false,
    "still.version": "0.5.2",
  });
  assert.deepEqual(page.errors, []);
  assert.equal(page.element("startup").value, "home");
  assert.equal(page.element("homepage").value, "https://example.org/");
  assert.equal(page.element("theme").value, "0");
  assert.equal(page.element("suggestions").checked, false);
  assert.equal(page.element("history").checked, false);
  assert.equal(page.element("still-version").textContent, "0.5.2");
  assert.equal(page.element("firefox-version").textContent, "153.5.0");
  assert.ok(page.groups.every((group) => !group.disabled));
  assert.equal(page.calls.saved, 0);
});

test("startup choices and homepage edits save Firefox's startup preferences", async () => {
  const page = await loadPage({ "browser.startup.page": 3, "browser.startup.homepage": "https://saved.test/" });
  assert.equal(page.element("startup").value, "session");
  for (const [choice, startup, homepage] of [
    ["home", 1, "https://saved.test/"],
    ["newtab", 1, "chrome://still/content/newtab.html"],
    ["session", 3, "chrome://still/content/newtab.html"],
    ["home", 1, "about:home"],
  ]) {
    page.element("startup").value = choice;
    await page.element("startup").send("change");
    assert.equal(page.values.get("browser.startup.page"), startup);
    assert.equal(page.values.get("browser.startup.homepage"), homepage);
  }
  page.element("homepage").value = " https://new.test/ ";
  await page.element("homepage").send("change");
  assert.equal(page.values.get("browser.startup.homepage"), "https://new.test/");
  assert.equal(page.calls.saved, 5);
  page.element("homepage").valid = false;
  page.element("homepage").value = "invalid";
  await page.element("homepage").send("change");
  assert.equal(page.values.get("browser.startup.homepage"), "https://new.test/");
  assert.equal(page.calls.saved, 5);
  const reopened = await loadPage(Object.fromEntries(page.values));
  assert.equal(reopened.element("homepage").value, "https://new.test/");
});

test("history, suggestions and search engine changes persist through Firefox APIs", async () => {
  const page = await loadPage();
  for (const [id, pref] of [["suggestions", "browser.search.suggest.enabled"], ["history", "places.history.enabled"]]) {
    page.element(id).checked = false;
    await page.element(id).send("change");
    assert.equal(page.values.get(pref), false);
  }
  assert.equal(page.element("engine").value, "duck");
  assert.equal(page.element("engine").disabled, false);
  assert.deepEqual(page.element("engine").children.map((option) => option.textContent), ["Google", "DuckDuckGo"]);
  page.element("engine").value = "google";
  await page.element("engine").send("change");
  assert.deepEqual(page.calls.search, [["google", "user"]]);
  assert.equal(page.calls.saved, 3);
});

test("theme changes install builtins before enabling them and clear the old forced theme", async () => {
  const page = await loadPage({ "ui.systemUsesDarkTheme": 1 });
  for (const [value, id] of [["1", "firefox-compact-light@mozilla.org"], ["0", "firefox-compact-dark@mozilla.org"], ["2", "default-theme@mozilla.org"]]) {
    page.element("theme").value = value;
    await page.element("theme").send("change");
    assert.deepEqual(page.errors, []);
    assert.equal(page.calls.themes.at(-1), id);
    assert.equal(page.values.get("browser.theme.toolbar-theme"), Number(value));
    assert.equal(page.values.get("browser.theme.content-theme"), Number(value));
    assert.equal(page.values.get("layout.css.prefers-color-scheme.content-override"), 2);
    assert.equal(page.values.has("ui.systemUsesDarkTheme"), false);
  }
  assert.equal(page.calls.saved, 3);
});

test("native actions route to Firefox's dialogs and pages", async () => {
  const page = await loadPage();
  for (const id of ["clear-data", "passwords", "import-browser", "advanced", "license"]) {
    const event = await page.element(id).send("click");
    if (["advanced", "license"].includes(id)) assert.equal(event.prevented, true);
  }
  assert.deepEqual(page.calls.native, [
    ["sanitize"], ["tab", "about:logins", true],
    ["preferences", "general-migrate-autoclose"],
    ["tab", "about:preferences", true], ["tab", "about:license", true],
  ]);
  assert.deepEqual(page.errors, []);
});

test("images and updates invoke fixed launcher flags from the parent installation", async () => {
  const page = await loadPage();
  for (const id of ["default-browser", "choose-image", "remove-image", "update"]) await page.element(id).send("click");
  assert.deepEqual(page.calls.launches, ["--make-default", "--newtab-image", "--remove-newtab-image", "--update"].map((flag) => ({
    executable: "C:/Still/still.exe", args: [flag], count: 1,
  })));
  const fallback = await loadPage({}, { launcher: "C:/Still/firefox/still.exe" });
  await fallback.element("update").send("click");
  assert.equal(fallback.calls.launches[0].executable, "C:/Still/firefox/still.exe");
  const missing = await loadPage({}, { launcher: null });
  await missing.element("update").send("click");
  assert.equal(missing.calls.launches.length, 0);
  assert.equal(missing.element("error").hidden, false);
});

test("failed search initialization leaves the engine control disabled and other settings usable", async () => {
  const page = await loadPage({}, { failSearch: true });
  assert.equal(page.element("engine").disabled, true);
  assert.match(page.element("search-status").textContent, /読み込めません/);
  page.element("history").checked = false;
  await page.element("history").send("change");
  assert.equal(page.values.get("places.history.enabled"), false);
});

test("save and launcher feedback describe the completed step and errors clear success", async () => {
  const page = await loadPage({}, { launcher: null });
  page.element("history").checked = false;
  await page.element("history").send("change");
  assert.match(page.element("status").textContent, /保存しました/);
  assert.equal(page.element("status").attributes.has("data-saved"), true);
  await page.element("choose-image").send("click");
  assert.equal(page.element("status").textContent, "");
  assert.equal(page.element("status").attributes.has("data-saved"), false);
  assert.equal(page.element("choose-image").disabled, false);
  const available = await loadPage();
  await available.element("choose-image").send("click");
  assert.match(available.element("status").textContent, /画像を選んだ後.*再起動/);
  await available.element("update").send("click");
  assert.match(available.element("status").textContent, /確認を開始/);
});

test("an error stays visible until the failed control succeeds", async () => {
  const page = await loadPage({}, { launcher: null });
  await page.element("update").send("click");
  assert.equal(page.element("error").hidden, false);
  page.element("history").checked = false;
  await page.element("history").send("change");
  assert.equal(page.element("error").hidden, false, "another control's success does not hide the failure");
  const retry = await loadPage();
  await retry.element("passwords").send("click");
  assert.equal(retry.element("error").hidden, true);
});

test("checkbox changes made while an earlier one runs are applied in order", async () => {
  const page = await loadPage();
  const control = page.element("history");
  control.checked = false;
  const first = control.send("change");
  control.checked = true;
  const second = control.send("change");
  await Promise.all([first, second]);
  assert.equal(page.values.get("places.history.enabled"), true);
  assert.equal(page.calls.saved, 2);
});

test("a pending theme change cannot start a second conflicting operation", async () => {
  let finish;
  const ready = new Promise((resolve) => { finish = resolve; });
  const page = await loadPage({}, { enableTheme: () => ready });
  page.element("theme").value = "1";
  const first = page.element("theme").send("change");
  assert.equal(page.element("theme").disabled, true);
  await page.element("theme").send("change");
  finish();
  await first;
  assert.deepEqual(page.calls.themes, ["firefox-compact-light@mozilla.org"]);
  assert.equal(page.calls.saved, 1);
  assert.equal(page.element("theme").disabled, false);
});

test("navigation follows the visible settings section without changing the URL", async () => {
  const page = await loadPage();
  assert.equal(page.links[0].attributes.get("aria-current"), "location");
  page.sections.forEach((section) => { section.top -= 750; });
  page.listeners.get("scroll")();
  assert.equal(page.links[1].attributes.has("aria-current"), false);
  assert.equal(page.links[2].attributes.get("aria-current"), "location");
  // On narrow windows the feedback bar scrolls away instead of covering controls.
  page.feedback.bottom = -650;
  page.sections.forEach((section) => { section.top -= 50; });
  page.listeners.get("scroll")();
  assert.equal(page.links[2].attributes.get("aria-current"), "location");
  page.listeners.get("hashchange")();
  assert.equal(page.links[0].attributes.get("aria-current"), "location");
});

test("settings search matches normalized words and aliases without saving preferences", async () => {
  const page = await loadPage();
  assert.equal(page.element("settings-query").disabled, false);
  page.element("settings-query").value = "ＴＨＥＭＥ　壁紙";
  await page.element("settings-query").send("input");
  assert.deepEqual(page.sections.filter((section) => !section.hidden).map((section) => section.id), ["appearance"]);
  assert.match(page.element("settings-search-status").textContent, /1 件/);
  assert.equal(page.element("settings-empty").hidden, true);
  assert.equal(page.links[1].attributes.get("aria-current"), "location");
  // At the bottom of filtered results, navigation must not mark a hidden section.
  page.viewport.scrollY = 1600;
  page.listeners.get("scroll")();
  assert.equal(page.links[1].attributes.get("aria-current"), "location");
  page.element("settings-query").value = "お気に入り";
  await page.element("settings-query").send("input");
  assert.deepEqual(page.sections.filter((section) => !section.hidden).map((section) => section.id), ["import"]);
  assert.equal(page.calls.saved, 0);
});

test("empty results can be cleared with Escape or the reset button and restore search focus", async () => {
  const page = await loadPage();
  for (const action of ["escape", "reset", "clear"]) {
    page.element("settings-query").value = "見つからない設定";
    await page.element("settings-query").send("input");
    assert.ok(page.sections.every((section) => section.hidden));
    assert.equal(page.element("settings-empty").hidden, false);
    assert.ok(page.links.every((link) => !link.attributes.has("aria-current")));
    page.listeners.get("scroll")();
    if (action == "escape") {
      const event = await page.element("settings-query").send("keydown", { key: "Escape" });
      assert.equal(event.prevented, true);
    } else {
      await page.element(action == "reset" ? "reset-settings-query" : "clear-settings-query").send("click");
    }
    assert.equal(page.element("settings-query").value, "");
    assert.equal(page.element("settings-query").focused, true);
    assert.ok(page.sections.every((section) => !section.hidden));
    assert.equal(page.element("settings-empty").hidden, true);
    assert.equal(page.element("clear-settings-query").hidden, true);
    assert.equal(page.element("settings-search-status").textContent, "");
  }
});

test("category navigation reveals its destination while a settings search is active", async () => {
  const page = await loadPage();
  page.element("settings-query").value = "theme";
  await page.element("settings-query").send("input");
  assert.equal(page.sections[3].hidden, true);
  const event = await page.links[3].send("click");
  assert.equal(event.prevented, false);
  assert.equal(page.element("settings-query").value, "");
  assert.ok(page.sections.every((section) => !section.hidden));
  assert.equal(page.links[3].attributes.get("aria-current"), "location");
});
