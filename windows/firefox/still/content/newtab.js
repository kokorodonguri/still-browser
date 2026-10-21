// A local, privileged page. Only the browser's search service builds search URLs;
// user input is never treated as markup or as a chrome/javascript URI.
const $ = (id) => document.getElementById(id);
const browserWindow = () => window.browsingContext.topChromeWindow;

function report(error) {
  console.error(error);
  $("error").textContent = "開けませんでした。上のアドレス欄やメニューからも操作できます。";
  $("error").hidden = false;
}

// Older ESR exposes Services.search; newer Firefox exposes SearchService.
function searchService() {
  return Services.search || ChromeUtils.importESModule("moz-src:///toolkit/components/search/SearchService.sys.mjs").SearchService;
}

// Private windows open this page too, so they use the private default engine.
function defaultEngine(search) {
  return window.browsingContext.usePrivateBrowsing ? search.getDefaultPrivate() : search.getDefault();
}

async function initSearch() {
  try {
    const search = searchService();
    await search.init();
    const engine = await defaultEngine(search);
    if (!engine) throw new Error("No default search engine");
    $("search-status").textContent = `${engine.name} で検索`;
    $("query").disabled = false;
    $("submit-search").disabled = false;
    $("search-form").addEventListener("submit", async (event) => {
      event.preventDefault();
      const query = $("query").value.trim();
      if (!query) { $("query").focus(); return; }
      try {
        const current = await defaultEngine(search);
        const submission = current?.getSubmission(query, null, "newtab");
        if (!submission || !["https", "http"].includes(submission.uri.scheme)) {
          throw new Error("Unsupported search submission");
        }
        browserWindow().openTrustedLinkIn(submission.uri.spec, "current", { postData: submission.postData });
      } catch (error) { report(error); }
    });
  } catch (error) {
    $("search-status").textContent = "検索を準備できませんでした。上のアドレス欄から検索できます。";
    report(error);
  }
}

for (const [id, action] of [
  ["bookmarks", () => browserWindow().SidebarController.toggle("viewBookmarksSidebar")],
  ["history", () => browserWindow().SidebarController.toggle("viewHistorySidebar")],
  // Firefox 153 moved BrowserDownloadsUI() into BrowserCommands.
  ["downloads", () => {
    const win = browserWindow();
    return win.BrowserCommands?.downloadsUI ? win.BrowserCommands.downloadsUI() : win.BrowserDownloadsUI();
  }],
]) {
  $(id).addEventListener("click", () => {
    $("error").hidden = true;
    Promise.resolve().then(action).catch(report);
  });
}
initSearch();
