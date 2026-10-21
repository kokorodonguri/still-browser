// Privileged chrome page; preferences remain Firefox's, not a separate store.
const { classes: Cc, interfaces: Ci } = Components;
const $ = (id) => document.getElementById(id);
const prefs = Services.prefs;
const NEWTAB = "chrome://still/content/newtab.html";
const NEWTAB_PAGES = ["about:newtab", NEWTAB];

function browserWindow() {
  return window.browsingContext.topChromeWindow;
}

// The control whose failure is shown; only its own later success clears it,
// so another control's "saved" never hides a change that did not apply.
let failedControl = null;

function report(error, control = null) {
  console.error(error);
  failedControl = control;
  $("status").textContent = "";
  $("status").removeAttribute("data-saved");
  $("error").textContent = "設定を変更できませんでした。Firefox の詳細設定も確認してください。";
  $("error").hidden = false;
}

// Buttons and selects are locked while their action runs, and repeats are
// dropped. Checkboxes and inputs stay usable, so their changes run in order.
function bind(id, event, action) {
  const control = $(id);
  const lockControl = control.tagName == "BUTTON" || control.tagName == "SELECT";
  let pending = false;
  let queue = Promise.resolve();
  const run = async (e) => {
    const disabled = control.disabled;
    if (lockControl) control.disabled = true;
    try {
      await action(e);
      if (failedControl == control) {
        failedControl = null;
        $("error").hidden = true;
      }
    } catch (error) {
      report(error, control);
    } finally {
      if (lockControl) control.disabled = disabled;
    }
  };
  control.addEventListener(event, (e) => {
    if (lockControl) {
      if (pending) return undefined;
      pending = true;
      return run(e).finally(() => { pending = false; });
    }
    queue = queue.then(() => run(e));
    return queue;
  });
}

function save() {
  prefs.savePrefFile(null);
  $("status").textContent = "変更を保存しました。";
  $("status").setAttribute("data-saved", "");
}

function launcherAction(argument, message) {
  runLauncher(argument);
  $("status").removeAttribute("data-saved");
  $("status").textContent = message;
}

function startupChoice() {
  if (prefs.getIntPref("browser.startup.page", 3) == 3) {
    return "session";
  }
  return NEWTAB_PAGES.includes(prefs.getStringPref("browser.startup.homepage", "about:home")) ? "newtab" : "home";
}

// Use the launcher for images and updates, so file checks and storage match
// its command-line operations. Firefox is installed one folder below it.
function runLauncher(argument) {
  const firefox = Services.dirsvc.get("GreD", Ci.nsIFile);
  const candidates = [firefox.parent, firefox];
  const exe = candidates.map((directory) => {
    const file = directory.clone();
    file.append("still.exe");
    return file;
  }).find((file) => file.exists() && file.isFile());
  if (!exe) {
    throw new Error("still.exe が見つかりません。");
  }
  const process = Cc["@mozilla.org/process/util;1"].createInstance(Ci.nsIProcess);
  process.init(exe);
  process.runwAsync([argument], 1);
}

// Firefox 153 keeps the search service in a module; chrome pages lack Services.search.
function searchService() {
  return Services.search || ChromeUtils.importESModule("moz-src:///toolkit/components/search/SearchService.sys.mjs").SearchService;
}

async function loadSearch() {
  try {
    const search = searchService();
    await search.init();
    const engines = await search.getVisibleEngines();
    const current = await search.getDefault();
    $("engine").replaceChildren(...engines.map((engine) => {
      const option = document.createElement("option");
      option.value = engine.id;
      option.textContent = engine.name;
      option.selected = engine.id == current?.id;
      return option;
    }));
    $("engine").disabled = !engines.length;
    $("search-status").textContent = engines.length ? "" : "検索エンジンがありません。Firefox の詳細設定から追加できます。";
    bind("engine", "change", async () => {
      const engine = engines.find((item) => item.id == $("engine").value);
      await search.setDefault(engine, search.CHANGE_REASON.USER);
      save();
    });
  } catch (error) {
    console.error(error);
    $("search-status").textContent = "検索エンジンを読み込めませんでした。画面を開き直してください。";
  }
}

function initNavigation() {
  const sections = [...document.querySelectorAll("main section")];
  const links = [...document.querySelectorAll("nav a")];
  const query = $("settings-query");
  const normalize = (text) => text.normalize("NFKC").toLowerCase();
  // Index labels and stable keywords, rather than values of private settings.
  const index = sections.map((section) => normalize(`${section.textContent} ${section.dataset.keywords || ""}`));
  const markCurrent = (hash) => {
    for (const link of links) {
      if (link.hash == hash) {
        link.setAttribute("aria-current", "location");
      } else {
        link.removeAttribute("aria-current");
      }
    }
  };
  const navigation = () => markCurrent(location.hash || "#general");
  window.addEventListener("hashchange", navigation);
  navigation();
  const filterSettings = () => {
    const terms = normalize(query.value).trim().split(/\s+/).filter(Boolean);
    sections.forEach((section, i) => {
      section.hidden = !terms.every((term) => index[i].includes(term));
    });
    const visible = sections.filter((section) => !section.hidden);
    $("clear-settings-query").hidden = !query.value;
    $("settings-empty").hidden = visible.length > 0;
    $("settings-search-status").hidden = !terms.length;
    $("settings-search-status").textContent = terms.length ? `${visible.length} 件の設定項目が見つかりました。` : "";
    if (terms.length) markCurrent(visible.length ? `#${visible[0].id}` : "");
    else navigation();
  };
  const resetSearch = () => {
    query.value = "";
    filterSettings();
    query.focus();
  };
  query.addEventListener("input", filterSettings);
  query.addEventListener("keydown", (event) => {
    if (event.key == "Escape" && query.value) {
      event.preventDefault();
      resetSearch();
    }
  });
  $("clear-settings-query").addEventListener("click", resetSearch);
  $("reset-settings-query").addEventListener("click", resetSearch);
  for (const link of links) {
    link.addEventListener("click", () => {
      query.value = "";
      filterSettings();
      markCurrent(link.hash);
    });
  }
  query.disabled = false;
  filterSettings();
  let scrollPending = false;
  window.addEventListener("scroll", () => {
    if (scrollPending) return;
    scrollPending = true;
    window.requestAnimationFrame(() => {
      scrollPending = false;
      const visible = sections.filter((section) => !section.hidden);
      // Match the sticky feedback bar; mark the last heading above it.
      const threshold = Math.max(24, document.querySelector(".feedback").getBoundingClientRect().bottom + 24);
      const current = visible.filter((section) => section.getBoundingClientRect().top <= threshold).at(-1) || visible[0];
      const atBottom = window.innerHeight + window.scrollY >= document.documentElement.scrollHeight - 2;
      if (current) markCurrent(`#${atBottom ? visible.at(-1).id : current.id}`);
    });
  }, { passive: true });
}

function init() {
  const memoryAvailable = prefs.getBoolPref("still.memory.autoUnload.available", false);
  $("memory-settings").hidden = !memoryAvailable;
  if (memoryAvailable) {
    $("auto-unload").checked = prefs.getBoolPref("still.memory.autoUnload.enabled", false);
    $("unload-minutes").value = String(prefs.getIntPref("still.memory.autoUnload.inactiveMinutes", 5));
    $("unload-minutes").disabled = !$("auto-unload").checked;
    bind("auto-unload", "change", () => {
      prefs.setBoolPref("still.memory.autoUnload.enabled", $("auto-unload").checked);
      $("unload-minutes").disabled = !$("auto-unload").checked;
      save();
    });
    bind("unload-minutes", "change", () => {
      const minutes = Number($("unload-minutes").value);
      if (!$("unload-minutes").reportValidity() || !Number.isInteger(minutes) || minutes < 1 || minutes > 1440) {
        return;
      }
      prefs.setIntPref("still.memory.autoUnload.inactiveMinutes", minutes);
      save();
    });
  }
  $("startup").value = startupChoice();
  $("homepage").value = prefs.getStringPref("browser.startup.homepage", "about:home");
  bind("startup", "change", () => {
    const choice = $("startup").value;
    prefs.setIntPref("browser.startup.page", choice == "session" ? 3 : 1);
    if (choice == "newtab") {
      prefs.setStringPref("browser.startup.homepage", NEWTAB);
      $("homepage").value = NEWTAB;
    } else if (choice == "home" && NEWTAB_PAGES.includes($("homepage").value)) {
      prefs.setStringPref("browser.startup.homepage", "about:home");
      $("homepage").value = "about:home";
    }
    save();
  });
  bind("homepage", "change", () => {
    if (!$("homepage").reportValidity()) {
      return;
    }
    prefs.setStringPref("browser.startup.homepage", $("homepage").value.trim() || "about:home");
    $("startup").value = startupChoice();
    save();
  });

  $("theme").value = String(prefs.getIntPref("browser.theme.toolbar-theme", 2));
  bind("theme", "change", async () => {
    const { AddonManager } = ChromeUtils.importESModule("resource://gre/modules/AddonManager.sys.mjs");
    const { BuiltInThemes } = ChromeUtils.importESModule("resource:///modules/BuiltInThemes.sys.mjs");
    const themes = ["firefox-compact-dark@mozilla.org", "firefox-compact-light@mozilla.org", "default-theme@mozilla.org"];
    const choice = Number($("theme").value);
    await BuiltInThemes.ensureBuiltInThemes();
    const theme = await AddonManager.getAddonByID(themes[choice]);
    if (!theme) {
      throw new Error("テーマが見つかりません。");
    }
    prefs.clearUserPref("ui.systemUsesDarkTheme");
    await theme.enable();
    prefs.setIntPref("browser.theme.toolbar-theme", choice);
    prefs.setIntPref("browser.theme.content-theme", choice);
    prefs.setIntPref("layout.css.prefers-color-scheme.content-override", 2);
    save();
  });
  bind("default-browser", "click", () => launcherAction("--make-default", "Windows の設定を開きました。Still を選ぶと、既定のブラウザになります。"));
  bind("choose-image", "click", () => launcherAction("--newtab-image", "画像の選択画面を開きました。画像を選んだ後、Still を再起動すると反映されます。"));
  bind("remove-image", "click", () => launcherAction("--remove-newtab-image", "背景を外す処理を開始しました。Still を再起動すると反映されます。"));

  for (const [id, pref] of [["suggestions", "browser.search.suggest.enabled"], ["history", "places.history.enabled"]]) {
    $(id).checked = prefs.getBoolPref(pref, true);
    bind(id, "change", () => {
      prefs.setBoolPref(pref, $(id).checked);
      save();
    });
  }
  bind("clear-data", "click", async () => {
    const { Sanitizer } = ChromeUtils.importESModule("resource:///modules/Sanitizer.sys.mjs");
    await Sanitizer.showUI(browserWindow());
  });
  bind("passwords", "click", () => browserWindow().switchToTabHavingURI("about:logins", true));
  bind("import-browser", "click", () => browserWindow().openPreferences("general-migrate-autoclose"));
  $("still-version").textContent = prefs.getStringPref("still.version", "不明");
  $("firefox-version").textContent = Services.appinfo.version;
  bind("update", "click", () => launcherAction("--update", "アップデートの確認を開始しました。結果は別のダイアログでお知らせします。"));
  for (const [id, uri] of [["advanced", "about:preferences"], ["license", "about:license"]]) {
    bind(id, "click", (event) => {
      event.preventDefault();
      browserWindow().switchToTabHavingURI(uri, true);
    });
  }
  document.querySelectorAll("fieldset").forEach((group) => { group.disabled = false; });
  initNavigation();
  loadSearch();
}

try {
  init();
} catch (error) {
  report(error);
}
