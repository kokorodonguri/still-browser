/* Any copyright is dedicated to the Public Domain.
   http://creativecommons.org/publicdomain/zero/1.0/ */

const { StillTabUnloader } = ChromeUtils.importESModule(
  "moz-src:///browser/components/tabbrowser/StillTabUnloader.sys.mjs"
);

add_setup(async function () {
  await SpecialPowers.pushPrefEnv({ set: [
    ["still.memory.autoUnload.enabled", true],
    ["still.memory.autoUnload.inactiveMinutes", 5],
  ] });
  StillTabUnloader.init();
});

async function backgroundTab() {
  const tab = await BrowserTestUtils.openNewForegroundTab(gBrowser, "https://example.com/");
  const selected = await BrowserTestUtils.openNewForegroundTab(gBrowser, "https://example.org/");
  tab.updateLastAccessed(Date.now() - 300_001);
  registerCleanupFunction(() => {
    BrowserTestUtils.removeTab(tab);
    BrowserTestUtils.removeTab(selected);
  });
  return tab;
}

add_task(async function discard_and_restore() {
  const tab = await backgroundTab();
  const uri = tab.linkedBrowser.currentURI.spec;
  await StillTabUnloader.tick();
  ok(tab.hasAttribute("pending"), "old background tab discarded");
  const loaded = BrowserTestUtils.browserLoaded(tab.linkedBrowser, false, uri);
  gBrowser.selectedTab = tab;
  await loaded;
  is(tab.linkedBrowser.currentURI.spec, uri, "selection restores the original page");
});

add_task(async function retain_cross_origin_editor() {
  const tab = await backgroundTab();
  await SpecialPowers.spawn(tab.linkedBrowser, [], async () => {
    const frame = content.document.createElement("iframe");
    frame.src = "https://example.org/";
    const loaded = new Promise(resolve => frame.onload = resolve);
    content.document.body.append(frame);
    await loaded;
  });
  const childBC = tab.linkedBrowser.browsingContext.children[0];
  await SpecialPowers.spawn(childBC, [], () => {
    const editor = content.document.createElement("textarea");
    editor.value = "unsaved frame draft";
    content.document.body.append(editor);
  });
  await StillTabUnloader.tick();
  ok(!tab.hasAttribute("pending"), "cross-origin frame draft retained");
});

add_task(async function retain_selection_during_flush() {
  const tab = await backgroundTab();
  const original = gBrowser.prepareDiscardBrowser;
  gBrowser.prepareDiscardBrowser = async function (candidate) {
    await original.call(this, candidate);
    if (candidate === tab) {
      gBrowser.selectedTab = tab;
    }
  };
  try {
    await StillTabUnloader.tick();
    ok(!tab.hasAttribute("pending"), "tab selected during state flush retained");
  } finally {
    gBrowser.prepareDiscardBrowser = original;
  }
});
