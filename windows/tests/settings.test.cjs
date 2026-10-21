const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const config = fs.readFileSync(path.join(__dirname, "../firefox/still.cfg"), "utf8");
const settingsURI = "chrome://still/content/settings.html";
const startupTopic = "browser-delayed-startup-finished";

function loadConfig({ prefs: initial = {}, overridden = false } = {}) {
  const observers = new Map();
  const registrations = [];
  const locked = new Map();
  const errors = [];
  const notify = (topic, data) => {
    for (const observer of [...(observers.get(topic) || [])]) observer(null, topic, data);
  };
  const newTab = {
    _url: overridden ? "moz-extension://test/newtab.html" : "about:newtab",
    newTabURLOverridden: overridden,
    get newTabURL() { return this._url; },
    set newTabURL(url) {
      if (url == "about:newtab") return this.resetNewTabURL();
      this._url = url;
      this.newTabURLOverridden = true;
      notify("newtab-url-changed", url);
    },
    resetNewTabURL() {
      this._url = "about:newtab";
      this.newTabURLOverridden = false;
      notify("newtab-url-changed", this._url);
    },
  };
  const prefObservers = new Map();
  const defaults = new Map();
  const values = new Map(Object.entries(initial));
  const shell = {};
  const launches = [];
  const file = (parts) => ({
    parts,
    append(part) { this.parts.push(part); },
    clone() { return file([...this.parts]); },
    get parent() { return file(this.parts.slice(0, -1)); },
  });
  const context = vm.createContext({
    ChromeUtils: {
      importESModule(uri) {
        if (uri == "moz-src:///browser/components/shell/ShellService.sys.mjs") {
          return { ShellService: shell };
        }
        assert.equal(uri, "resource:///modules/AboutNewTab.sys.mjs");
        return { AboutNewTab: newTab };
      },
    },
    Services: {
      dirsvc: {
        get() {
          return file(["firefox"]);
        },
      },
      io: { newFileURI(file) { return `file:///${file.parts.join("/")}`; } },
      prefs: {
        getIntPref(name) { return values.has(name) ? values.get(name) : 1; },
        setIntPref(name, value) { values.set(name, value); },
        getBoolPref(name, fallback) { return values.has(name) ? values.get(name) : fallback; },
        getDefaultBranch() { return { setStringPref(name, value) { defaults.set(name, value); } }; },
        addObserver(name, observer) { prefObservers.set(name, observer); },
      },
      obs: {
        addObserver(observer, topic) {
          const entries = observers.get(topic) || new Set();
          entries.add(observer);
          observers.set(topic, entries);
        },
        removeObserver(observer, topic) { observers.get(topic)?.delete(observer); },
      },
    },
    Components: {
      interfaces: {},
      classes: {
        "@mozilla.org/addons/addon-manager-startup;1": {
          getService() {
            return {
              registerChrome(uri, entries) {
                const registration = { uri, entries };
                registrations.push(registration);
                return registration;
              },
            };
          },
        },
        "@mozilla.org/process/util;1": {
          createInstance() {
            return {
              init(exe) { this.exe = exe.parts.join("/"); },
              runw(blocking, args) { launches.push({ exe: this.exe, blocking, args: Array.from(args) }); },
            };
          },
        },
      },
    },
    dump(error) { errors.push(error); },
    lockPref(name, value) { locked.set(name, value); },
  });
  vm.runInContext(config, context, { filename: "still.cfg" });
  return {
    registrations,
    locked,
    newTab,
    defaults,
    errors,
    shell,
    launches,
    values,
    setPref(name, value) {
      values.set(name, value);
      prefObservers.get(name)?.(null, "nsPref:changed", name);
    },
    async openWindow(window) {
      for (const observer of [...(observers.get(startupTopic) || [])]) {
        observer(window);
      }
      // Window setup waits for the document's localization promise.
      await window.document.l10n.ready;
      await Promise.resolve();
    },
  };
}

function browserWindow({ privateMode = false, permanent = false, title = "" } = {}) {
  const attributes = new Map();
  const listeners = new Map();
  const errors = [];
  const tabs = [];
  const originalBrand = privateMode && !permanent ? "Mozilla Firefox プライベートブラウジング" : "Mozilla Firefox";
  const window = {
    closed: false,
    console: { error(error) { errors.push(error); } },
    document: {
      title: "",
      documentElement: {
        setAttribute(name, value) { attributes.set(name, value); },
        getAttribute(name) { return name == "privatebrowsingmode" && permanent ? "permanent" : attributes.get(name); },
      },
      l10n: { ready: Promise.resolve() },
      getElementById(id) {
        const labels = {
          mainWindowTitle: "Mozilla Firefox",
          privateWindowTitle: "Mozilla Firefox プライベートブラウジング",
        };
        return id in labels ? { textContent: labels[id] } : null;
      },
    },
    gBrowser: {
      selectedBrowser: { contentTitle: title },
      tabs: [],
      tabContainer: { addEventListener(type, listener) { listeners.set(`tab:${type}`, { listener }); } },
      addTabsProgressListener(listener) { listeners.set("progress", { listener }); },
      getTabForBrowser(linked) { return this.tabs.find((tab) => tab.linkedBrowser === linked); },
      addTrustedTab(uri) {
        const tab = { uri };
        tabs.push(tab);
        return tab;
      },
      // Firefox 153 caches localized branding separately from root attributes.
      getWindowTitleForBrowser(browser) {
        return [browser.contentTitle, originalBrand].filter(Boolean).join(" — ");
      },
      updateTitlebar() {
        window.document.title = this.getWindowTitleForBrowser(this.selectedBrowser);
      },
    },
    PanelUI: { hidden: 0, hide() { this.hidden += 1; } },
    addEventListener(type, listener, capture) { listeners.set(type, { listener, capture }); },
    switchToTabHavingURI(uri, openNew) {
      const existing = tabs.find((tab) => tab.uri === uri);
      if (existing) {
        window.gBrowser.selectedTab = existing;
        return true;
      }
      if (openNew) {
        const tab = { uri };
        tabs.push(tab);
        window.gBrowser.selectedTab = tab;
      }
      return false;
    },
  };
  return {
    window, attributes, listeners, errors, tabs,
    command(id) {
      const event = {
        target: { closest(selector) { return selector.split(", ").includes(`#${id}`) ? { id } : null; } },
        prevented: false,
        stopped: false,
        preventDefault() { this.prevented = true; },
        stopImmediatePropagation() { this.stopped = true; },
      };
      listeners.get("command").listener(event);
      return event;
    },
  };
}

test("settings chrome is registered on every application start", () => {
  for (let start = 0; start < 2; start += 1) {
    const loaded = loadConfig();
    assert.deepEqual(loaded.errors, []);
    assert.equal(loaded.registrations.length, 1);
    assert.equal(loaded.registrations[0].uri, "file:///firefox/still/chrome.manifest");
    assert.deepEqual(Array.from(loaded.registrations[0].entries[0]), ["content", "still", "content/"]);
    assert.equal(loaded.newTab.newTabURL, "chrome://still/content/newtab.html");
    assert.equal(loaded.defaults.get("browser.startup.homepage"), "chrome://still/content/newtab.html");
    // Firefox's own button would make firefox.exe the default, without Still's profile.
    assert.equal(loaded.locked.get("pref.general.disable_button.default_browser"), true);
  }
});

test("Firefox's own make-default paths open Windows' Default apps at Still", async () => {
  const loaded = loadConfig();
  await loaded.shell.setDefaultBrowser(false);
  loaded.shell.setAsDefault();
  assert.deepEqual(loaded.launches, [
    { exe: "still.exe", blocking: false, args: ["--make-default"] },
    { exe: "still.exe", blocking: false, args: ["--make-default"] },
  ]);
});

test("Firefox's blank new tab and extension overrides are kept", () => {
  assert.equal(loadConfig({ prefs: { "browser.newtabpage.enabled": false } }).newTab.newTabURL, "about:newtab");
  assert.equal(loadConfig({ overridden: true }).newTab.newTabURL, "moz-extension://test/newtab.html");
});

test("changing the blank-page preference updates new tabs without restarting", () => {
  for (const enabled of [true, false]) {
    const loaded = loadConfig({ prefs: { "browser.newtabpage.enabled": enabled } });
    for (const value of [false, true, false, true]) {
      loaded.setPref("browser.newtabpage.enabled", value);
      assert.equal(loaded.newTab.newTabURL, value ? "chrome://still/content/newtab.html" : "about:newtab");
      assert.equal(loaded.newTab.newTabURLOverridden, value);
    }
    assert.deepEqual(loaded.errors, []);
  }
});

test("preference changes preserve extension pages and removing an override restores Still", () => {
  const loaded = loadConfig({ overridden: true });
  for (const value of [false, true]) {
    loaded.setPref("browser.newtabpage.enabled", value);
    assert.equal(loaded.newTab.newTabURL, "moz-extension://test/newtab.html");
  }
  loaded.newTab.resetNewTabURL();
  assert.equal(loaded.newTab.newTabURL, "chrome://still/content/newtab.html");
  loaded.newTab.newTabURL = "moz-extension://second/newtab.html";
  loaded.setPref("browser.newtabpage.enabled", false);
  loaded.newTab.resetNewTabURL();
  assert.equal(loaded.newTab.newTabURL, "about:newtab");
  loaded.setPref("browser.newtabpage.enabled", true);
  assert.equal(loaded.newTab.newTabURL, "chrome://still/content/newtab.html");
  assert.deepEqual(loaded.errors, []);
});

test("every ordinary and private window receives Still branding", async () => {
  const loaded = loadConfig();
  for (const [privateMode, permanent] of [[false, false], [true, false], [true, true], [false, false]]) {
    const browser = browserWindow({ privateMode, permanent, title: "Mozilla Firefox — guide" });
    await loaded.openWindow(browser.window);
    assert.deepEqual(browser.errors, []);
    assert.equal(browser.attributes.get("data-title-default"), "Still");
    assert.equal(browser.attributes.get("data-title-private"), "Still プライベートブラウジング");
    const suffix = privateMode ? "Still プライベートブラウジング" : "Still";
    assert.equal(browser.window.document.title, `Mozilla Firefox — guide — ${suffix}`);
    browser.window.gBrowser.selectedBrowser.contentTitle = "次のページ";
    browser.window.gBrowser.updateTitlebar();
    assert.equal(browser.window.document.title, `次のページ — ${suffix}`);
    browser.window.gBrowser.selectedBrowser.contentTitle = "";
    browser.window.gBrowser.updateTitlebar();
    assert.equal(browser.window.document.title, suffix);
  }
});

test("sidebar, toolbar and menu commands open one settings tab and then focus it", async () => {
  const loaded = loadConfig();
  const browser = browserWindow();
  await loaded.openWindow(browser.window);
  assert.equal(browser.listeners.get("command").capture, true);

  const unrelated = browser.command("back-button");
  assert.equal(unrelated.prevented, false);
  assert.equal(unrelated.stopped, false);
  assert.equal(browser.tabs.length, 0);

  for (const id of ["still-settings-button", "appMenu-settings-button", "preferences-button"]) {
    browser.window.gBrowser.selectedTab = { uri: "https://example.org/" };
    const event = browser.command(id);
    assert.equal(event.prevented, true);
    assert.equal(event.stopped, true);
    assert.equal(browser.tabs.length, 1);
    assert.equal(browser.window.gBrowser.selectedTab.uri, settingsURI);
  }
  assert.equal(browser.window.PanelUI.hidden, 3);
});

test("moving Settings preserves an existing toolbar and runs only once", async () => {
  const loaded = loadConfig({ prefs: { "still.settingsPosition": 0, "still.windowsLayout": 1 } });
  const removed = [];
  for (let i = 0; i < 2; i += 1) {
    const browser = browserWindow();
    browser.window.CustomizableUI = { removeWidgetFromArea(id) { removed.push(id); } };
    await loaded.openWindow(browser.window);
    assert.deepEqual(browser.errors, []);
  }
  assert.deepEqual(removed, ["preferences-button"]);
  assert.equal(loaded.values.get("still.settingsPosition"), 1);
  assert.equal(loaded.values.get("still.windowsLayout"), 1);
});

test("each tab shows its web site's host under the title", async () => {
  const loaded = loadConfig();
  const browser = browserWindow();
  const tab = (spec) => {
    const attributes = new Map();
    const container = {
      attributes,
      setAttribute(name, value) { attributes.set(name, value); },
      removeAttribute(name) { attributes.delete(name); },
    };
    const linkedBrowser = { currentURI: null };
    const result = { linkedBrowser, container, querySelector: () => container };
    result.go = (next) => {
      const [scheme, rest = ""] = next.split("://");
      linkedBrowser.currentURI = { schemeIs: (name) => name == scheme, host: rest.split("/")[0] };
    };
    result.go(spec);
    return result;
  };
  const first = tab("https://www.example.org/page");
  browser.window.gBrowser.tabs.push(first);
  await loaded.openWindow(browser.window);
  assert.equal(first.container.attributes.get("still-host"), "example.org");

  first.go("about:newtab");
  browser.listeners.get("progress").listener.onLocationChange(first.linkedBrowser);
  assert.equal(first.container.attributes.has("still-host"), false);

  const opened = tab("http://nologin.test/");
  browser.listeners.get("tab:TabOpen").listener({ target: opened });
  assert.equal(opened.container.attributes.get("still-host"), "nologin.test");
  assert.deepEqual(browser.errors, []);
});
