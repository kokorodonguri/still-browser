import { backend, type LoginView, type Snapshot, type Update } from "./backend";
import { el, icon, setAttribute, setText } from "./dom";
import { TabList } from "./tabs";
import { applyAppearance, focusSettings, renderSettings } from "./settings";
import { checkForUpdate, installUpdate, onUpdateChange, updateState } from "./updates";
import { onPictureChange, pictureUrl, syncPicture } from "./picture";

type Section = "tabs" | "bookmarks" | "history" | "downloads";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const address = $<HTMLInputElement>("address");
const sidebar = $("sidebar");
const content = $("content");
const list = $("list");

let state: Snapshot | null = null;
let section: Section = "tabs";
let editing = false;
let renderQueued = false;
let renderedSection: Section | null = null;
let listData: unknown;
const tabIndices = new Map<number, number>();
const changedTabs = new Set<number>();
let tabStructureChanged = true;
let painted: Snapshot | null = null;
const tabList = new TabList(list, (id) => backend.selectTab(id), (id) => backend.closeTab(id), () => void newTab());
const lookupTab = (id: number) => { const index = tabIndices.get(id); return index === undefined ? undefined : state?.tabs[index]; };
let settingsWasOpen = false;

const SECTION_TITLES: Record<Section, string> = {
  tabs: "Tabs",
  bookmarks: "Bookmarks",
  history: "Recently visited",
  downloads: "Downloads",
};

const CLOSE_ICON = '<path d="M6 6l8 8M14 6l-8 8" />';

function hostOf(url: string) {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

function row(title: string, detail: string, onActivate: () => void, current = false) {
  const button = el("button", { className: "row", title: detail ? `${title}\n${detail}` : title });
  button.setAttribute("role", "listitem");
  if (current) button.setAttribute("aria-current", "true");
  const text = el("span", { className: "row-text" }, el("span", { className: "row-title", textContent: title }));
  if (detail) text.append(el("span", { className: "row-detail", textContent: detail }));
  button.append(text);
  button.addEventListener("click", onActivate);
  return button;
}

function trailing(label: string, onActivate: () => void) {
  const button = el("span", { className: "icon small trailing", title: label, tabIndex: 0 });
  button.setAttribute("role", "button");
  button.setAttribute("aria-label", label);
  button.append(icon(CLOSE_ICON));
  const activate = (event: Event) => {
    event.stopPropagation();
    onActivate();
  };
  button.addEventListener("click", activate);
  button.addEventListener("keydown", (event) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      activate(event);
    }
  });
  return button;
}

/** Only changed tab rows are touched; other sections follow their own list. */
function renderList() {
  if (!state || sidebar.hidden) return;
  const s = state;
  const switched = renderedSection !== section;
  if (switched) {
    renderedSection = section;
    listData = undefined;
    list.replaceChildren();
    tabList.clear();
    setText($("section-title"), SECTION_TITLES[section]);
    for (const tab of document.querySelectorAll<HTMLButtonElement>(".segmented button")) {
      setAttribute(tab, "aria-selected", String(tab.dataset.section === section));
    }
  }
  if (section === "tabs") {
    tabList.render(s.tabs, lookupTab, s.active, switched || tabStructureChanged ? null : changedTabs);
    tabStructureChanged = false;
    changedTabs.clear();
    return;
  }
  const data = s[section];
  if (data === listData) return;
  listData = data;
  const rows: HTMLElement[] = [];
  const empty = (text: string) => rows.push(el("p", { className: "empty", textContent: text }));

  if (section === "bookmarks") {
    for (const b of s.bookmarks) {
      const item = row(b.title || hostOf(b.url), b.url, () => backend.navigate(b.url));
      item.append(trailing("Remove bookmark", () => backend.removeBookmark(b.url)));
      rows.push(item);
    }
    if (!s.bookmarks.length) empty("No bookmarks yet. Use the star in the address bar to save a page.");
  } else if (section === "history") {
    for (const h of s.history) rows.push(row(h.title || hostOf(h.url), h.url, () => backend.navigate(h.url)));
    if (!s.history.length) empty("Pages you visit will appear here.");
  } else {
    for (const d of s.downloads) {
      const detail = d.state === "InProgress" ? "Downloading…" : d.state === "Completed" ? `${Math.max(1, Math.round(d.bytes / 1024))} KB · Show in folder` : d.state;
      const item = row(d.name || d.path, detail, () => backend.openDownloadFolder(d.path));
      if (d.state === "InProgress") item.prepend(el("span", { className: "dot" }));
      rows.push(item);
    }
    if (!s.downloads.length) empty("Files you download will appear here.");
  }
  list.replaceChildren(...rows);
}

function render() {
  if (!state) return;
  const s = state;
  const previous = painted;
  if (!previous || previous.loading !== s.loading) {
    document.body.classList.toggle("loading", s.loading);
    const reload = $("reload");
    reload.title = reload.ariaLabel = s.loading ? "Stop" : "Reload";
  }
  if (!editing && address.value !== s.url) address.value = s.url;
  if (!previous || previous.canBack !== s.canBack) ($("back") as HTMLButtonElement).disabled = !s.canBack;
  if (!previous || previous.canForward !== s.canForward) ($("forward") as HTMLButtonElement).disabled = !s.canForward;
  if (!previous || !!previous.url !== !!s.url) {
    ($("reload") as HTMLButtonElement).disabled = !s.url;
    $("bookmark").hidden = !s.url;
  }
  if (!previous || previous.fullscreen !== s.fullscreen) document.body.classList.toggle("fullscreen", s.fullscreen);
  if (!previous || previous.bookmarked !== s.bookmarked) {
    const star = $("bookmark");
    setAttribute(star, "aria-pressed", String(s.bookmarked));
    star.title = star.ariaLabel = s.bookmarked ? "Remove bookmark" : "Bookmark this page";
  }
  renderLogins(s.logins, s.url);
  if (!previous || previous.downloading !== s.downloading) $("downloads-indicator").hidden = s.downloading === 0;
  if (!previous || previous.notice !== s.notice) {
    $("notice").hidden = !s.notice;
    setText($("notice-text"), s.notice ?? "");
    $("notice-text").title = s.notice ?? "";
  }
  setSidebar(s.sidebarOpen, false);
  renderList();
  applyAppearance(s.settings);
  syncPicture(s.settings.NewTabImage);
  showPicture();
  renderSettings(s.settings, s.settingsOpen, s.savedLogins);
  if (s.settingsOpen && !settingsWasOpen) focusSettings();
  settingsWasOpen = s.settingsOpen;
  painted = s;
}

// ---------- Saved passwords ----------
// The key in the address bar fills a saved login into the page. With several
// logins for the site, the choice covers the address field (a popup would sit
// under the page, which is drawn above the interface).

const picker = $("login-picker");
const fillButton = $<HTMLButtonElement>("fill-login");

let renderedLogins = "";
let loginsUrl = "";

/** Every update carries the page's logins; only a change touches the DOM. */
function renderLogins(logins: LoginView[], url: string) {
  const key = JSON.stringify(logins);
  if (key === renderedLogins && url === loginsUrl) return;
  const navigated = url !== loginsUrl;
  renderedLogins = key;
  loginsUrl = url;
  fillButton.hidden = !logins.length;
  if (!picker.hidden && (navigated || !logins.length)) closePicker(false);
  else if (!picker.hidden) openPicker();
}

function openPicker() {
  const logins = state?.logins ?? [];
  const items = logins.map((login) => {
    const item = el("button", { type: "button", className: "login-choice", title: login.host },
      el("span", { textContent: login.username || "No user name" }));
    item.setAttribute("role", "menuitem");
    item.addEventListener("click", () => {
      backend.fillLogin(login.index);
      closePicker(false);
    });
    return item;
  });
  const close = el("button", { type: "button", className: "icon small", title: "Cancel" }, icon(CLOSE_ICON));
  close.setAttribute("aria-label", "Cancel");
  close.addEventListener("click", () => closePicker(true));
  picker.replaceChildren(el("span", { className: "login-picker-label", textContent: "Fill in" }), el("div", { className: "login-choices" }, ...items), close);
  const wasHidden = picker.hidden;
  picker.hidden = false;
  setAttribute(fillButton, "aria-expanded", "true");
  if (wasHidden) items[0]?.focus();
}

function closePicker(refocus: boolean) {
  if (picker.hidden) return;
  picker.hidden = true;
  setAttribute(fillButton, "aria-expanded", "false");
  if (refocus) fillButton.focus();
}

fillButton.addEventListener("click", () => {
  const logins = state?.logins ?? [];
  if (logins.length === 1) backend.fillLogin(logins[0].index);
  else if (logins.length > 1) openPicker();
});
picker.addEventListener("keydown", (event) => {
  const items = [...picker.querySelectorAll<HTMLButtonElement>(".login-choice")];
  const at = items.indexOf(document.activeElement as HTMLButtonElement);
  if (event.key === "Escape") {
    event.preventDefault();
    event.stopPropagation();
    closePicker(true);
  } else if ((event.key === "ArrowRight" || event.key === "ArrowLeft") && items.length) {
    event.preventDefault();
    items[(at + (event.key === "ArrowRight" ? 1 : items.length - 1)) % items.length].focus();
  }
});
picker.addEventListener("focusout", (event) => {
  if (!picker.contains(event.relatedTarget as Node | null)) closePicker(false);
});

/** A new tab has no page webview, so its picture is the content area's background. */
let shownPicture = "";
function showPicture() {
  const url = state && !state.url && !state.settingsOpen ? pictureUrl() : "";
  if (url === shownPicture) return;
  shownPicture = url;
  content.classList.toggle("picture", !!url);
  if (url) content.style.setProperty("--picture", `url("${url}")`);
  else content.style.removeProperty("--picture");
}
onPictureChange(showPicture);

function setSidebar(open: boolean, persist = true) {
  if (sidebar.hidden === !open && !persist) return;
  sidebar.hidden = !open;
  const toggle = $("sidebar-toggle");
  toggle.setAttribute("aria-pressed", String(open));
  toggle.title = toggle.ariaLabel = open ? "Hide sidebar" : "Show sidebar";
  if (persist) backend.setSidebar(open);
  if (open) renderList();
}

function showSection(next: Section) {
  section = next;
  if (sidebar.hidden) setSidebar(true);
  renderList();
}

function focusAddress() {
  address.focus();
  address.select();
}

async function newTab() {
  await backend.newTab();
  focusAddress();
}

// Keep the page webviews exactly over the content area.
let lastBounds = "";
function sendBounds() {
  const r = content.getBoundingClientRect();
  const bounds = { x: Math.round(r.left), y: Math.round(r.top), width: Math.round(r.width), height: Math.round(r.height) };
  const key = JSON.stringify(bounds);
  if (key === lastBounds) return;
  lastBounds = key;
  backend.setContentBounds(bounds);
}
new ResizeObserver(sendBounds).observe(content);
window.addEventListener("resize", sendBounds);

// ---------- Wiring ----------
$("address-form").addEventListener("submit", (event) => {
  event.preventDefault();
  const text = address.value.trim();
  if (!text) return;
  editing = false;
  address.blur();
  backend.navigate(text);
});
address.addEventListener("focus", () => {
  editing = true;
  requestAnimationFrame(() => address.select());
});
address.addEventListener("blur", () => {
  editing = false;
  render();
});
address.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    editing = false;
    address.value = state?.url ?? "";
    address.blur();
  }
});

$("sidebar-toggle").addEventListener("click", () => setSidebar(sidebar.hidden));
$("back").addEventListener("click", () => backend.go("back"));
$("forward").addEventListener("click", () => backend.go("forward"));
$("reload").addEventListener("click", () => backend.go(state?.loading ? "stop" : "reload"));
$("bookmark").addEventListener("click", () => backend.toggleBookmark());
$("new-tab").addEventListener("click", () => newTab());
$("notice-dismiss").addEventListener("click", () => backend.dismissNotice());
$("downloads-indicator").addEventListener("click", () => showSection("downloads"));
$("update-button").addEventListener("click", () => void installUpdate());
onUpdateChange(() => {
  const { phase, available } = updateState();
  const button = $<HTMLButtonElement>("update-button");
  button.hidden = phase !== "available" && phase !== "installing" && phase !== "install-failed";
  button.disabled = phase === "installing";
  setText($("update-label"), phase === "installing" ? "Updating…" : phase === "install-failed" ? "Retry update" : "Update");
  button.title = phase === "installing" ? "Downloading the update. Still will restart." : `Install Still ${available} and restart`;
});
$("win-min").addEventListener("click", () => backend.window("minimize"));
$("win-max").addEventListener("click", () => backend.window("toggleMaximize"));
$("win-close").addEventListener("click", () => backend.window("close"));
for (const button of document.querySelectorAll<HTMLButtonElement>(".segmented button")) {
  button.addEventListener("click", () => showSection(button.dataset.section as Section));
}
document.querySelector(".segmented")!.addEventListener("keydown", (event) => {
  const e = event as KeyboardEvent;
  if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
  const order: Section[] = ["tabs", "bookmarks", "history", "downloads"];
  const next = order[(order.indexOf(section) + (e.key === "ArrowRight" ? 1 : 3)) % 4];
  showSection(next);
  $(`seg-${next}`).focus();
});

window.addEventListener("keydown", (event) => {
  if (!event.ctrlKey || event.altKey || event.metaKey) return;
  const key = event.key.toLowerCase();
  if (key === "l" && !event.shiftKey) focusAddress();
  else if (key === "t" && !event.shiftKey) newTab();
  else if (key === "w" && !event.shiftKey) state && backend.closeTab(state.active);
  else if (key === "tab") backend.cycleTab(!event.shiftKey);
  else if (key === "," && !event.shiftKey) backend.setSettingsOpen(!state?.settingsOpen);
  else return;
  event.preventDefault();
});

function receive(next: Update) {
  if (!state || next.revision <= state.revision) return;
  const blankBefore = !state.url;
  if (next.tabs) {
    tabIndices.clear();
    next.tabs.forEach((tab, index) => tabIndices.set(tab.id, index));
    tabStructureChanged = true;
    changedTabs.clear();
  } else {
    for (const tab of next.tabChanges ?? []) {
      const index = tabIndices.get(tab.id);
      if (index !== undefined) { state.tabs[index] = tab; changedTabs.add(tab.id); }
    }
  }
  state = {
    ...next,
    tabs: next.tabs ?? state.tabs,
    bookmarks: next.bookmarks ?? state.bookmarks,
    history: next.history ?? state.history,
    downloads: next.downloads ?? state.downloads,
    savedLogins: next.savedLogins ?? state.savedLogins,
    settings: next.settings ?? state.settings,
  };
  if (!renderQueued) {
    renderQueued = true;
    requestAnimationFrame(() => { renderQueued = false; render(); });
  }
  if (!next.url && !blankBefore && !editing) focusAddress();
}

// Subscribe before requesting the initial snapshot. Revision numbers prevent a
// delayed ready response or an older event from replacing newer state.
async function initialize() {
  const pending: Update[] = [];
  const unsubscribe = await Promise.all([
    backend.onState((next) => state ? receive(next) : void pending.push(next)),
    backend.onFocusAddress(focusAddress),
  ]);
  try {
    const initial = await backend.ready();
    state = initial;
    initial.tabs.forEach((tab, index) => tabIndices.set(tab.id, index));
    for (const next of pending) receive(next);
    pending.length = 0;
    render();
    sendBounds();
    if (!state.url) focusAddress();
  } catch (error) {
    // A failed startup must not retain every later state event in the buffer.
    unsubscribe.forEach((stop) => stop());
    pending.length = 0;
    throw error;
  }
}
void initialize().then(() => checkForUpdate(false)).catch(console.error);
