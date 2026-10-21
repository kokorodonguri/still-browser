const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");
const source = fs.readFileSync(path.join(__dirname, "../firefox/still/content/newtab.js"), "utf8");

async function page({ modern = false, failure = false, privateWindow = false, legacyDownloads = false } = {}) {
  const elements = new Map();
  const calls = [];
  const errors = [];
  const element = (id) => {
    if (!elements.has(id)) elements.set(id, {
      value: "", disabled: true, hidden: true, textContent: "", listeners: {},
      addEventListener(type, listener) { this.listeners[type] = listener; },
      focus() { this.focused = true; },
    });
    return elements.get(id);
  };
  const search = {
    async init() { if (failure) throw new Error("Search unavailable"); },
    async getDefault() { return search.engine; },
    async getDefaultPrivate() { return search.privateEngine; },
    privateEngine: {
      name: "Private engine",
      getSubmission(query) { return { uri: { scheme: "https", spec: `https://private.test/?q=${encodeURIComponent(query)}` }, postData: null }; },
    },
    engine: {
      name: "Test engine",
      getSubmission(query) { return { uri: { scheme: "https", spec: `https://search.test/?q=${encodeURIComponent(query)}` }, postData: null }; },
    },
  };
  const browser = {
    openTrustedLinkIn(url, target, options) { calls.push({ url, target, options }); },
    SidebarController: { toggle(id) { calls.push(id); } },
  };
  if (legacyDownloads) browser.BrowserDownloadsUI = () => calls.push("downloads");
  else browser.BrowserCommands = { downloadsUI() { calls.push("downloads"); } };
  vm.runInNewContext(source, {
    document: { getElementById: element },
    Services: modern ? {} : { search },
    ChromeUtils: { importESModule() { return { SearchService: search }; } },
    window: { browsingContext: { topChromeWindow: browser, usePrivateBrowsing: privateWindow } },
    console: { error(error) { errors.push(error); } },
  });
  await new Promise(setImmediate);
  return { element, search, calls, errors, async submit(value) {
    element("query").value = value;
    let prevented = false;
    await element("search-form").listeners.submit({ preventDefault() { prevented = true; } });
    assert.ok(prevented);
  } };
}

test("new tab uses Firefox's current default engine, including POST submissions", async () => {
  for (const modern of [false, true]) {
    const p = await page({ modern });
    assert.equal(p.element("query").disabled, false);
    assert.equal(p.element("search-status").textContent, "Test engine で検索");
    await p.submit(" 日本語 & <script> ");
    assert.equal(p.calls[0].url, `https://search.test/?q=${encodeURIComponent("日本語 & <script>")}`);
    const postData = { stream: "query body" };
    p.search.engine = { getSubmission() { return { uri: { scheme: "https", spec: "https://other.test/search" }, postData }; } };
    await p.submit("new engine");
    assert.equal(p.calls[1].url, "https://other.test/search");
    assert.equal(p.calls[1].target, "current");
    assert.equal(p.calls[1].options.postData, postData);
  }
});

test("private windows search with the private default engine", async () => {
  const p = await page({ privateWindow: true });
  assert.equal(p.element("search-status").textContent, "Private engine で検索");
  await p.submit("秘密");
  assert.equal(p.calls[0].url, `https://private.test/?q=${encodeURIComponent("秘密")}`);
});

test("downloads open through Firefox 153's command and older Firefox's function", async () => {
  for (const legacyDownloads of [false, true]) {
    const p = await page({ legacyDownloads });
    p.element("downloads").listeners.click();
    await new Promise(setImmediate);
    assert.deepEqual(p.calls, ["downloads"]);
    assert.equal(p.element("error").hidden, true);
  }
});

test("empty searches stay on the page and unsupported search schemes never navigate", async () => {
  const p = await page();
  await p.submit("   ");
  assert.equal(p.element("query").focused, true);
  for (const scheme of ["javascript", "chrome", "file", "data"]) {
    p.search.engine = { getSubmission() { return { uri: { scheme, spec: `${scheme}:payload` } }; } };
    await p.submit("query");
  }
  assert.equal(p.calls.length, 0);
  assert.equal(p.element("error").hidden, false);
});

test("failed search initialization leaves controls disabled with a readable fallback", async () => {
  const p = await page({ failure: true });
  assert.equal(p.element("query").disabled, true);
  assert.equal(p.element("submit-search").disabled, true);
  assert.match(p.element("search-status").textContent, /アドレス欄/);
});

test("library shortcuts call the browser's bookmarks, history and downloads", async () => {
  const p = await page();
  for (const id of ["bookmarks", "history", "downloads"]) {
    p.element(id).listeners.click();
    await new Promise(setImmediate);
  }
  assert.deepEqual(p.calls, ["viewBookmarksSidebar", "viewHistorySidebar", "downloads"]);
});
