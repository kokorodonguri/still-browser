const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");
const files = path.join(__dirname, "../gecko/common/files/browser/components/tabbrowser");

// Execute the shipped modules, replacing only Gecko's module imports/globals.
function moduleScope(name, globals, symbol) {
  const source = fs.readFileSync(path.join(files, name), "utf8")
    .replace(/^import .*;\n/gm, "").replace(/^export /gm, "");
  return vm.runInNewContext(`${source}\n;${symbol}`, globals, { filename: name });
}

function harness({ queryTimeoutMs = 2000 } = {}) {
  const prefs = new Map([["still.memory.autoUnload.enabled", true]]);
  const windows = [];
  const errors = [];
  const intervals = new Set();
  const observers = new Set();
  const actorNames = new Set();
  const clock = { now: 1_000_000 };
  const controller = moduleScope("StillTabUnloader.sys.mjs", {
    Date: { now: () => clock.now },
    setTimeout: (fn, ms) => setTimeout(fn, Math.min(ms, queryTimeoutMs)), clearTimeout,
    setInterval(fn) { intervals.add(fn); return fn; },
    clearInterval(fn) { intervals.delete(fn); },
    console: { error(error) { errors.push(error); } },
    Services: {
      prefs: {
        addObserver() {}, removeObserver() {},
        getBoolPref: (name, fallback) => prefs.get(name) ?? fallback,
        getIntPref: (name, fallback) => prefs.get(name) ?? fallback,
      },
      obs: { addObserver(o) { observers.add(o); }, removeObserver(o) { observers.delete(o); } },
      wm: { getEnumerator() { return windows; } },
    },
    ChromeUtils: {
      registerWindowActor(name) { assert.ok(!actorNames.has(name)); actorNames.add(name); },
      unregisterWindowActor(name) { actorNames.delete(name); },
    },
    PrivateBrowsingUtils: { isBrowserPrivate: browser => !!browser.private },
    webrtcUI: { browserHasStreams: browser => !!browser.sharing },
  }, "StillTabUnloader");
  function frame(safe = true) {
    return {
      hasActivePeerConnections: () => false,
      getActor() { return { sendQuery: async () => safe }; },
    };
  }
  function window() {
    const win = { closed: false };
    win.gBrowser = {
      ownerGlobal: win, tabs: [], discarded: [], flushed: [],
      getTabDialogBox: () => ({ _tabDialogManager: { _dialogs: [] } }),
      async prepareDiscardBrowser(tab) { this.flushed.push(tab); },
      discardBrowser(tab, force) {
        assert.equal(force, undefined);
        this.discarded.push(tab);
        tab.pending = true;
        return true;
      },
    };
    windows.push(win);
    return win;
  }
  function tab(win = windows[0] || window(), safe = true) {
    const tab = {
      isConnected: true, ownerDocument: { defaultView: win }, lastAccessed: clock.now - 300_000,
      hasAttribute(name) { return !!this[name]; },
      linkedBrowser: {
        isConnected: true, isRemoteBrowser: true,
        currentURI: { scheme: "https" }, webProgress: { isLoadingDocument: false },
        browsingContext: { currentWindowGlobal: frame(safe), children: [], mediaController: { isPlaying: false } },
      },
    };
    win.gBrowser.tabs.push(tab);
    return tab;
  }
  return { controller, prefs, windows, frame, window, tab, errors, clock, intervals, observers, actorNames };
}

test("five-minute boundary; oldest first across windows; at most one per tick", async () => {
  const h = harness();
  const young = h.tab();
  young.lastAccessed += 1;
  const first = h.windows[0];
  const second = h.window();
  const old = h.tab(second);
  old.lastAccessed -= 1000;
  const boundary = h.tab(first);
  await h.controller.tick();
  assert.deepEqual(second.gBrowser.discarded, [old]);
  assert.deepEqual(first.gBrowser.discarded, []);
  await h.controller.tick();
  assert.deepEqual(first.gBrowser.discarded, [boundary]);
  await h.controller.tick();
  assert.equal(first.gBrowser.discarded.length, 1);
});

test("all protected parent states are retained", async () => {
  const protectors = [
    t => { t.selected = true; }, t => { t.pinned = true; }, t => { t.closing = true; },
    t => { t.undiscardable = true; }, t => { t.pending = true; }, t => { t.busy = true; },
    t => { t.soundPlaying = true; }, t => { t.pictureinpicture = true; },
    t => { t.linkedBrowser.private = true; }, t => { t.linkedBrowser.sharing = true; },
    t => { t.linkedBrowser.hasBeforeUnload = true; },
    t => { t.linkedBrowser.isRemoteBrowser = false; },
    t => { t.linkedBrowser.currentURI.scheme = "moz-extension"; },
    t => { t.linkedBrowser.webProgress.isLoadingDocument = true; },
    t => { t.linkedBrowser.browsingContext.mediaController.isPlaying = true; },
    t => { t.linkedBrowser.browsingContext.currentWindowGlobal.hasActivePeerConnections = () => true; },
    (t, h) => { h.windows[0].gBrowser.getTabDialogBox = () => ({ _tabDialogManager: { _dialogs: [{}] } }); },
    (t, h) => { h.prefs.set("still.memory.autoUnload.enabled", false); },
    (t, h) => { h.prefs.set("still.memory.autoUnload.inactiveMinutes", 15); },
  ];
  for (const protect of protectors) {
    const h = harness();
    protect(h.tab(), h);
    await h.controller.tick();
    assert.equal(h.windows[0].gBrowser.discarded.length, 0, String(protect));
  }
});

test("frame refusal, unavailable globals and actor errors fail closed", async () => {
  for (const mode of ["edited", "missing", "error"]) {
    const h = harness();
    const t = h.tab();
    const global = mode === "missing" ? null : h.frame(false);
    if (mode === "error") global.getActor = () => { throw new Error("gone"); };
    t.linkedBrowser.browsingContext.children.push({ currentWindowGlobal: global, children: [] });
    await h.controller.tick();
    assert.equal(h.windows[0].gBrowser.discarded.length, 0);
  }
});

test("unresponsive frame times out without blocking future ticks", async () => {
  const h = harness({ queryTimeoutMs: 5 });
  const tab = h.tab();
  const global = tab.linkedBrowser.browsingContext.currentWindowGlobal;
  global.getActor = () => ({ sendQuery: () => new Promise(() => {}) });
  await h.controller.tick();
  assert.equal(h.windows[0].gBrowser.discarded.length, 0);
  assert.equal(h.controller._running, false);
  tab.linkedBrowser.browsingContext.currentWindowGlobal = h.frame();
  await h.controller.tick();
  assert.equal(h.windows[0].gBrowser.discarded.length, 1);
});

test("overlapping ticks share one discard attempt", async () => {
  const h = harness();
  h.tab();
  let release;
  h.windows[0].gBrowser.prepareDiscardBrowser = () => new Promise(resolve => { release = resolve; });
  const tick = h.controller.tick();
  while (!release) await new Promise(setImmediate);
  await h.controller.tick();
  release();
  await tick;
  assert.equal(h.windows[0].gBrowser.discarded.length, 1);
});

test("session-flush races recheck selection, edits, frames, media and preferences", async () => {
  for (const change of [
    t => { t.selected = true; },
    (t, h) => { t.linkedBrowser.browsingContext.currentWindowGlobal = h.frame(); },
    t => { t.linkedBrowser.browsingContext.currentWindowGlobal.getActor = () => ({ sendQuery: async () => false }); },
    t => { t.linkedBrowser.browsingContext.mediaController.isPlaying = true; },
    (t, h) => { h.prefs.set("still.memory.autoUnload.enabled", false); },
  ]) {
    const h = harness();
    const t = h.tab();
    h.windows[0].gBrowser.prepareDiscardBrowser = async () => change(t, h);
    await h.controller.tick();
    assert.equal(h.windows[0].gBrowser.discarded.length, 0, String(change));
  }
});

test("failed discard/flush does not block another candidate or future ticks", async () => {
  const h = harness();
  const failing = h.tab();
  const good = h.tab();
  const g = h.windows[0].gBrowser;
  g.prepareDiscardBrowser = async tab => { if (tab === failing) throw new Error("flush failed"); };
  await h.controller.tick();
  assert.deepEqual(g.discarded, [good]);
  assert.equal(h.controller._running, false);
  const next = h.tab();
  await h.controller.tick();
  assert.deepEqual(g.discarded, [good, next]);
});

test("one timer across repeated initialization and cleanup on shutdown", async () => {
  const h = harness();
  h.controller.init();
  h.controller.init();
  assert.equal(h.intervals.size, 1);
  assert.equal(h.observers.size, 1);
  h.tab();
  h.controller.observe();
  await h.controller.tick();
  assert.equal(h.intervals.size, 0);
  assert.equal(h.observers.size, 0);
  assert.equal(h.actorNames.size, 0);
  assert.equal(h.windows[0].gBrowser.discarded.length, 0);
});

function child(elements = []) {
  const Child = moduleScope("StillTabStateChild.sys.mjs", { JSWindowActorChild: class {} }, "StillTabStateChild");
  const actor = new Child();
  actor.contentWindow = { MutationObserver: class {
    observe() {} disconnect() {} takeRecords() { return []; }
  } };
  actor.document = { readyState: "complete", createTreeWalker() {
    const copy = elements.map(e => ({ localName: "div", ...e }));
    return { nextNode: () => copy.shift() };
  } };
  return actor;
}

test("child protects modified forms, custom editors, closed shadow roots and silent video", () => {
  const protectedElements = [
    { localName: "input", value: "draft", defaultValue: "", cloneNode: () => ({ defaultValue: "" }) },
    { localName: "input", type: "checkbox", checked: true, defaultChecked: false },
    { localName: "input", type: "file", files: [{}] },
    { localName: "textarea", value: "draft", defaultValue: "" },
    { localName: "select", options: [{ selected: true, defaultSelected: false }] },
    { isContentEditable: true }, { isFormAssociatedCustomElement: true },
    { localName: "video", paused: false, ended: false },
    { openOrClosedShadowRoot: { isUAWidget: () => false, querySelectorAll: () => [{ isContentEditable: true }] } },
  ];
  for (const element of protectedElements) {
    assert.equal(child([element]).receiveMessage({ name: "Still:PassesRetentionChecks" }), false, JSON.stringify(element));
  }
  assert.equal(child([{ localName: "p" }]).receiveMessage({ name: "Still:PassesRetentionChecks" }), true);
  assert.equal(child([
    { localName: "input", type: "checkbox", checked: false, defaultChecked: false, value: "on", defaultValue: "" },
    { localName: "select", size: 0, selectedIndex: 0, options: [{ selected: true, defaultSelected: false }] },
    { localName: "input", value: "", cloneNode: () => ({ defaultValue: "" }) },
    { openOrClosedShadowRoot: { isUAWidget: () => true, querySelectorAll: () => [{ isContentEditable: true }] } },
  ]).receiveMessage({ name: "Still:PassesRetentionChecks" }), true);
});

test("trusted edits are sticky; incomplete/unknown content is retained", () => {
  const actor = child();
  actor.handleEvent({ isTrusted: false });
  assert.equal(actor.receiveMessage({ name: "Still:PassesRetentionChecks" }), true);
  actor.handleEvent({ isTrusted: true });
  assert.equal(actor.receiveMessage({ name: "Still:PassesRetentionChecks" }), false);
  for (const doc of [null, { readyState: "loading" }, { hidden: false }, { readyState: "complete", designMode: "on" }]) {
    const a = child();
    a.document = doc;
    assert.equal(a.receiveMessage({ name: "Still:PassesRetentionChecks" }), false);
  }
});

 test("default off has no timer or discard; preference starts/stops timer", async () => {
  const h = harness();
  h.prefs.clear();
  h.controller.init();
  h.tab();
  await h.controller.tick();
  assert.equal(h.intervals.size, 0);
  assert.equal(h.windows[0].gBrowser.discarded.length, 0);
  h.prefs.set("still.memory.autoUnload.enabled", true);
  h.controller.observe(null, "nsPref:changed");
  assert.equal(h.intervals.size, 1);
  h.prefs.set("still.memory.autoUnload.enabled", false);
  h.controller.observe(null, "nsPref:changed");
  assert.equal(h.intervals.size, 0);
});

test("bounded one-time walk, canvas retention and post-scan mutations", () => {
  assert.equal(child([{localName: "canvas"}]).receiveMessage({name: "Still:PassesRetentionChecks"}), false);
  assert.equal(child(Array(5001).fill({localName: "p"})).receiveMessage({name: "Still:PassesRetentionChecks"}), false);
  const a = child();
  assert.equal(a.receiveMessage({name: "Still:PassesRetentionChecks"}), true);
  a.document.createTreeWalker = () => { throw Error("unexpected repeat walk"); };
  assert.equal(a.receiveMessage({name: "Still:PassesRetentionChecks"}), true);
  a._observer.takeRecords = () => [{}];
  assert.equal(a.receiveMessage({name: "Still:PassesRetentionChecks"}), false);
});

test("failed initial scan remains retained on later queries", () => {
  const a = child();
  a.document.createTreeWalker = () => { throw Error("unavailable"); };
  assert.equal(a.receiveMessage({name: "Still:PassesRetentionChecks"}), false);
  assert.equal(a.receiveMessage({name: "Still:PassesRetentionChecks"}), false);
});
