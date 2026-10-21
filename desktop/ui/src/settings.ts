// The settings view: a category list on the left and grouped controls on the
// right, drawn over the content area while the page is hidden. Every control
// is HTML (no <select>, whose popup would be a native widget).
import { backend, type ClearRequest, type ImportRequest, type ImportSource, type LoginView, type Settings } from "./backend";
import { el, icon, setAttribute } from "./dom";
import { checkForUpdate, installUpdate, onUpdateChange, updateState } from "./updates";
import { choosePicture, onPictureChange, pictureUrl, removePicture } from "./picture";

type Pane = "general" | "appearance" | "search" | "privacy" | "passwords" | "import" | "shortcuts";

const root = document.getElementById("settings")!;
const body = document.getElementById("settings-body")!;
const tabs = [...document.querySelectorAll<HTMLButtonElement>("#settings-tabs [role=tab]")];

let pane: Pane = "general";
let settings: Settings | null = null;
let logins: LoginView[] = [];
/** Browsers found on this computer; null while looking. Refreshed when the pane opens. */
let sources: ImportSource[] | null = null;
let source = "";
const kinds: ImportRequest = { bookmarks: true, history: true, cookies: false };
let renderedKey = "";
/** Data kinds ticked under "Clear browsing data" (not saved). */
const clear: ClearRequest = { history: true, siteData: false, cache: true };
let confirming = false;
/** Home page text that is not a web address, kept so it can be corrected. */
let rejectedHome = "";

/** Mirrors the Rust rule (an http(s) URL or a bare host); Rust has the final say. */
const looksLikeAddress = (text: string) => /^https?:\/\/\S+$/i.test(text) || (!/\s/.test(text) && text.includes("."));

const CHECK_ICON = '<path d="M5 10.5l3 3 7-7" />';
const PANE_TITLES: Record<Pane, string> = {
  general: "General",
  appearance: "Appearance",
  search: "Search",
  privacy: "Privacy",
  passwords: "Passwords",
  import: "Import",
  shortcuts: "Shortcuts",
};

const update = (patch: Partial<Settings>) => backend.updateSettings(patch);

// ---------- Building blocks ----------

function group(title: string, ...rows: Node[]) {
  return el("section", { className: "settings-group" }, el("h3", { textContent: title }), el("div", { className: "settings-card" }, ...rows));
}

function text(label: string, detail?: string) {
  const node = el("span", { className: "setting-text" }, el("span", { className: "setting-label", textContent: label }));
  if (detail) node.append(el("span", { className: "setting-detail", textContent: detail }));
  return node;
}

function row(label: string, detail: string | undefined, ...trailing: Node[]) {
  return el("div", { className: "setting-row" }, text(label, detail), ...trailing);
}

/** Tags a control so focus can return to it after the pane is rebuilt. */
function control<T extends HTMLElement>(node: T, key: string) {
  node.dataset.key = key;
  return node;
}

function toggle(key: string, label: string, detail: string | undefined, checked: boolean, onChange: (on: boolean) => void, role: "switch" | "checkbox" = "switch") {
  const button = control(el("button", { className: `setting-row ${role}` }, text(label, detail)), key);
  button.setAttribute("role", role);
  button.setAttribute("aria-checked", String(checked));
  button.append(role === "switch" ? el("span", { className: "switch-track" }, el("span", { className: "switch-thumb" })) : el("span", { className: "checkmark" }, icon(CHECK_ICON)));
  button.addEventListener("click", () => onChange(!checked));
  return button;
}

/** A single-choice group with roving focus and arrow keys, like native radios. */
function radios<T extends string>(label: string, className: string, options: { value: T; build: (button: HTMLButtonElement) => void }[], selected: T, onPick: (value: T) => void) {
  const groupNode = el("div", { className });
  groupNode.setAttribute("role", "radiogroup");
  groupNode.setAttribute("aria-label", label);
  const buttons = options.map(({ value, build }) => {
    const button = control(el("button"), `${label}:${value}`);
    button.setAttribute("role", "radio");
    button.setAttribute("aria-checked", String(value === selected));
    button.tabIndex = value === selected ? 0 : -1;
    build(button);
    button.addEventListener("click", () => value !== selected && onPick(value));
    return button;
  });
  groupNode.addEventListener("keydown", (event) => {
    const step = { ArrowDown: 1, ArrowRight: 1, ArrowUp: -1, ArrowLeft: -1 }[event.key];
    if (!step) return;
    event.preventDefault();
    const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const next = buttons[(at + step + buttons.length) % buttons.length];
    next.focus();
    next.click();
  });
  groupNode.append(...buttons);
  return groupNode;
}

function choiceRows<T extends string>(label: string, choices: [T, string, string?][], selected: T, onPick: (value: T) => void) {
  return radios(label, "radio-rows", choices.map(([value, title, detail]) => ({
    value,
    build: (b) => {
      b.className = "setting-row";
      b.append(text(title, detail), el("span", { className: "checkmark" }, icon(CHECK_ICON)));
    },
  })), selected, onPick);
}

function segmented<T extends string>(label: string, choices: [T, string][], selected: T, onPick: (value: T) => void) {
  return radios(label, "choice", choices.map(([value, title]) => ({ value, build: (b) => void (b.textContent = title) })), selected, onPick);
}

function textButton(key: string, label: string, onClick: () => void, extra = "") {
  const button = control(el("button", { className: `text-button ${extra}`.trim(), type: "button", textContent: label }), key);
  button.addEventListener("click", onClick);
  return button;
}

// ---------- Panes ----------

function general(s: Settings) {
  const startup = choiceRows("Startup", [
    ["Restore", "Open the tabs from last time"],
    ["NewTab", "Open a new tab"],
    ["Home", "Open the home page"],
  ], s.Startup, (Startup) => update({ Startup }));
  const startupGroup = group("When Still starts", startup);
  if (s.Startup === "Home") {
    const invalid = !!rejectedHome;
    const input = document.createElement("input");
    Object.assign(input, { className: "field", type: "text", spellcheck: false, placeholder: "example.com", value: invalid ? rejectedHome : s.HomePage });
    control(input, "home");
    input.setAttribute("aria-label", "Home page address");
    if (invalid) input.setAttribute("aria-invalid", "true");
    input.addEventListener("change", () => {
      const value = input.value.trim();
      rejectedHome = value && !looksLikeAddress(value) ? value : "";
      if (rejectedHome) render(true);
      else update({ HomePage: value });
    });
    input.addEventListener("keydown", (event) => {
      if (event.key === "Enter") input.blur();
      if (event.key === "Escape") {
        event.stopPropagation();
        input.value = s.HomePage;
        input.blur();
      }
    });
    const home = el("div", { className: "setting-row" }, text("Home page", invalid ? "Enter a web address, such as example.com." : s.HomePage ? undefined : "A blank tab opens until you set one."), input);
    if (invalid) home.classList.add("invalid");
    startupGroup.lastElementChild!.append(home);
  }

  const folder = row("Save files to", s.DownloadFolder || "Downloads", textButton("folder", "Change…", () => backend.pickDownloadFolder()));
  if (s.DownloadFolder) folder.append(textButton("folder-reset", "Reset", () => update({ DownloadFolder: "" })));
  return [
    startupGroup,
    group("Downloads", folder, toggle("ask", "Ask where to save each file", "Choose the folder and name for every download.", s.AskDownloadFolder, (AskDownloadFolder) => update({ AskDownloadFolder }))),
    softwareUpdate(),
  ];
}

function softwareUpdate() {
  const { phase, current, available } = updateState();
  const detail = {
    idle: undefined,
    checking: "Checking for updates…",
    current: "Still is up to date.",
    available: `Version ${available} is available.`,
    installing: "Downloading the update. Still will restart.",
    "check-failed": "Could not check for updates. Check the network connection.",
    "install-failed": "Could not install the update. Try again later.",
  }[phase];
  const canInstall = phase === "available" || phase === "install-failed" || (phase === "installing" && !!available);
  const button = canInstall
    ? textButton("update-install", "Install and Restart", () => void installUpdate(), "accent")
    : textButton("update-check", "Check for Updates", () => void checkForUpdate(true));
  button.disabled = phase === "checking" || phase === "installing";
  return group("Software update", row(current ? `Still ${current}` : "Still", detail, button));
}

const ACCENTS: Settings["Accent"][] = ["Blue", "Purple", "Pink", "Orange", "Green", "Graphite"];

function appearance(s: Settings) {
  const themes = radios("Theme", "theme-cards", (["System", "Light", "Dark"] as const).map((value) => ({
    value,
    build: (b) => {
      b.className = "theme-card";
      // Automatic shows a light window with a dark one over its lower right.
      const win = (shade: string) => el("span", { className: `pv-win ${shade}` }, el("span", { className: "pv-side" }), el("span", { className: "pv-page" }));
      const preview = el("span", { className: "preview" }, ...(value === "System" ? [win("light"), win("dark")] : [win(value.toLowerCase())]));
      if (value === "System") preview.classList.add("system");
      b.append(preview, el("span", { textContent: value === "System" ? "Automatic" : value }));
    },
  })), s.Theme, (Theme) => update({ Theme }));
  const accents = radios("Accent color", "swatches", ACCENTS.map((value) => ({
    value,
    build: (b) => {
      b.className = "swatch";
      b.dataset.accent = value.toLowerCase();
      b.title = value;
      b.setAttribute("aria-label", value);
      b.append(icon(CHECK_ICON));
    },
  })), s.Accent, (Accent) => update({ Accent }));
  return [
    group("Theme", el("div", { className: "setting-row stacked" }, text("Window and pages", "Automatic follows the Windows setting."), themes)),
    group("Accent color", el("div", { className: "setting-row" }, text("Buttons, focus and progress"), accents)),
    group("New tab", newTabPicture(s)),
    group("Sidebar", el("div", { className: "setting-row" }, text("When Still starts"), segmented("Sidebar at startup", [["Remember", "As last time"], ["Open", "Open"], ["Closed", "Closed"]], s.Sidebar, (Sidebar) => update({ Sidebar })))),
  ];
}

function newTabPicture(s: Settings) {
  const url = pictureUrl();
  const thumb = el("span", { className: "picture-thumb" });
  thumb.setAttribute("aria-hidden", "true");
  if (url) thumb.style.backgroundImage = `url("${url}")`;
  const detail = !s.NewTabImage ? "PNG, JPEG, GIF, WebP, AVIF or BMP, up to 20 MB." : url ? "Shown behind every new tab." : "Loading…";
  const picture = row("Background picture", detail, thumb, textButton("picture", s.NewTabImage ? "Change…" : "Choose…", choosePicture));
  if (s.NewTabImage) picture.append(textButton("picture-remove", "Remove", removePicture));
  return picture;
}

function search(s: Settings) {
  return [
    group("Search engine", choiceRows("Search engine", [
      ["Google", "Google", "google.com"],
      ["Bing", "Bing", "bing.com"],
      ["DuckDuckGo", "DuckDuckGo", "duckduckgo.com · does not track searches"],
      ["YahooJapan", "Yahoo! JAPAN", "search.yahoo.co.jp"],
    ], s.Search, (Search) => update({ Search }))),
    el("p", { className: "settings-note", textContent: "Text in the address bar that is not a web address is searched with this engine." }),
  ];
}

function privacy(s: Settings) {
  const kinds = [
    toggle("clear-history", "Browsing history", "The list under Recently visited.", clear.history, (on) => setClear({ history: on }), "checkbox"),
    toggle("clear-site", "Cookies and site data", "Signs you out of most sites.", clear.siteData, (on) => setClear({ siteData: on }), "checkbox"),
    toggle("clear-cache", "Cached images and files", "Pages may load slower the next time.", clear.cache, (on) => setClear({ cache: on }), "checkbox"),
  ];
  const nothing = !clear.history && !clear.siteData && !clear.cache;
  const action = confirming
    ? el("div", { className: "setting-row confirm" },
        text("Clear the selected data?", "This cannot be undone."),
        textButton("clear-cancel", "Cancel", () => setConfirming(false)),
        textButton("clear-confirm", "Clear", () => {
          backend.clearBrowsingData({ ...clear });
          setConfirming(false);
        }, "danger"))
    : el("div", { className: "setting-row" }, el("span", { className: "setting-text" }), (() => {
        const button = textButton("clear", "Clear…", () => setConfirming(true));
        button.disabled = nothing;
        return button;
      })());
  return [
    group("History", toggle("keep-history", "Save browsing history", "Pages you visit appear under Recently visited.", s.KeepHistory, (KeepHistory) => update({ KeepHistory }))),
    group("Clear browsing data", ...kinds, action),
  ];
}

function passwords() {
  if (!logins.length) {
    return [
      group("Saved passwords", row("No saved passwords", "Import the passwords you saved in another browser.", textButton("to-import", "Import…", () => selectPane("import")))),
      el("p", { className: "settings-note", textContent: "Passwords you type into sign-in forms can also be saved when the page offers to." }),
    ];
  }
  const rows = logins.map((login) => row(login.host, login.username || "No user name", textButton(`remove-login:${login.host}:${login.username}`, "Remove", () => backend.removeLogin(login.index))));
  const removeAll = confirming
    ? el("div", { className: "setting-row confirm" },
        text(`Remove all ${logins.length} saved passwords?`, "This cannot be undone."),
        textButton("clear-cancel", "Cancel", () => setConfirming(false)),
        textButton("clear-confirm", "Remove All", () => {
          backend.clearLogins();
          setConfirming(false);
        }, "danger"))
    : el("div", { className: "setting-row" }, el("span", { className: "setting-text" }), textButton("clear", "Remove All…", () => setConfirming(true)));
  return [
    group(`Saved passwords (${logins.length})`, ...rows),
    group("Remove", removeAll),
    el("p", { className: "settings-note", textContent: "Passwords are encrypted for your Windows account. On a sign-in page, press the key in the address bar to fill one in." }),
  ];
}

function importPane(s: Settings) {
  const chosen = sources?.find((x) => x.id === source) ?? sources?.[0];
  let browserGroup: Node;
  if (sources === null) {
    browserGroup = group("From a browser on this PC", row("Looking for browsers…", undefined));
  } else if (!chosen) {
    browserGroup = group("From a browser on this PC", row("No other browsers were found", "Chrome, Edge, Brave, Vivaldi, Opera and Firefox are looked for."));
  } else {
    const picker = choiceRows("Browser", sources.map((x): [string, string, string?] => [x.id, x.browser, x.profile || undefined]), chosen.id, (id) => {
      source = id;
      render(true);
    });
    const history = s.KeepHistory
      ? toggle("import-history", "History", "Your latest 500 pages.", kinds.history, (on) => setKinds({ history: on }), "checkbox")
      : row("History", "Turn on Save browsing history under Privacy to import it.");
    const what = [
      toggle("import-bookmarks", "Bookmarks", undefined, kinds.bookmarks, (on) => setKinds({ bookmarks: on }), "checkbox"),
      history,
    ];
    if (chosen.cookies) {
      what.push(toggle("import-cookies", "Cookies", "Stay signed in to sites. Some, such as Google, may still ask you to sign in again.", kinds.cookies, (on) => setKinds({ cookies: on }), "checkbox"));
    }
    const request = { bookmarks: kinds.bookmarks, history: kinds.history && s.KeepHistory, cookies: kinds.cookies && chosen.cookies };
    const start = textButton("import-start", "Import", () => backend.importFrom(chosen.id, request), "accent");
    start.disabled = !request.bookmarks && !request.history && !request.cookies;
    browserGroup = el("div", {},
      group("From a browser on this PC", picker),
      group("What to import", ...what, el("div", { className: "setting-row" }, el("span", { className: "setting-text" }), start)));
  }
  return [
    browserGroup,
    group("From a file", row("Bookmarks, passwords or history", "A bookmarks file (HTML) or passwords file (CSV) exported from any browser, or History.json from Google Takeout.", textButton("import-file", "Choose File…", chooseImportFile))),
    group("Moving from Chrome or Edge",
      row("Passwords", "In Chrome or Edge, open Settings › Passwords, export your passwords, and choose that file above. Delete the exported file afterwards: anyone can read it."),
      row("Signing in", "Chrome and Edge lock their cookies to themselves, so sign in to Google and other sites again in Still. Imported passwords fill in with the key in the address bar.")),
  ];
}

function setKinds(patch: Partial<ImportRequest>) {
  Object.assign(kinds, patch);
  render(true);
}

/** Rust refuses larger files; a bigger one is cut just past this so the refusal is cheap. */
const IMPORT_LIMIT = 64 * 1024 * 1024;
const importInput = Object.assign(document.createElement("input"), { type: "file", accept: ".html,.htm,.csv,.json,text/html,text/csv,application/json", hidden: true });
importInput.addEventListener("change", async () => {
  const file = importInput.files?.[0];
  importInput.value = "";
  if (!file) return;
  try {
    await backend.importFile(await file.slice(0, IMPORT_LIMIT + 1).arrayBuffer());
  } catch (error) {
    console.error(error);
  }
});
document.body.append(importInput);

function chooseImportFile() {
  importInput.click();
}

function loadSources() {
  sources = null;
  backend.importSources().then((found) => {
    sources = found;
    if (!found.some((x) => x.id === source)) source = found[0]?.id ?? "";
    render(true);
  }).catch((error) => {
    console.error(error);
    sources = [];
    render(true);
  });
}

const SHORTCUTS: [string, string[]][] = [
  ["New tab", ["Ctrl", "T"]],
  ["Close tab", ["Ctrl", "W"]],
  ["Next tab", ["Ctrl", "Tab"]],
  ["Previous tab", ["Ctrl", "Shift", "Tab"]],
  ["Go to the address bar", ["Ctrl", "L"]],
  ["Open or close settings", ["Ctrl", ","]],
  ["Close settings", ["Esc"]],
];

function shortcuts() {
  return [group("Keyboard", ...SHORTCUTS.map(([label, keys]) => row(label, undefined, el("span", { className: "keys" }, ...keys.map((k) => el("kbd", { textContent: k }))))))];
}

// ---------- Rendering ----------

function setClear(patch: Partial<ClearRequest>) {
  Object.assign(clear, patch);
  confirming = false;
  render(true);
}

function setConfirming(on: boolean) {
  confirming = on;
  render(true);
  document.querySelector<HTMLElement>(`[data-key="${on ? "clear-cancel" : "clear"}"]`)?.focus();
}

function selectPane(next: Pane, focusTab = false) {
  pane = next;
  confirming = false;
  if (next === "import") loadSources();
  for (const tab of tabs) {
    const on = tab.dataset.pane === next;
    tab.setAttribute("aria-selected", String(on));
    tab.tabIndex = on ? 0 : -1;
    if (on) {
      body.setAttribute("aria-labelledby", tab.id);
      if (focusTab) tab.focus();
    }
  }
  render(true);
  body.scrollTop = 0;
}

/** Rebuilds the pane only when what it shows changed; focus survives rebuilds. */
function render(force = false) {
  if (!settings || root.hidden) return;
  const key = JSON.stringify([pane, settings, clear, confirming, rejectedHome, updateState(), pictureUrl(), pane === "passwords" ? logins : null, pane === "import" ? [sources, source, kinds] : null]);
  if (key === renderedKey && !force) return;
  renderedKey = key;
  const focused = (document.activeElement as HTMLElement | null)?.dataset?.key;
  const s = settings;
  const content = { general, appearance, search, privacy, passwords, import: importPane, shortcuts }[pane](s);
  body.replaceChildren(el("h2", { className: "settings-title", textContent: PANE_TITLES[pane] }), ...content);
  if (focused) body.querySelector<HTMLElement>(`[data-key="${CSS.escape(focused)}"]`)?.focus();
}

/** Called for every state update. */
export function renderSettings(next: Settings, open: boolean, saved: LoginView[]) {
  const changed = !settings || (Object.keys(next) as (keyof Settings)[]).some((key) => settings![key] !== next[key]) || (pane === "passwords" && saved !== logins);
  settings = next;
  logins = saved;
  const wasHidden = root.hidden;
  if (root.hidden !== !open) root.hidden = !open;
  const toggleButton = document.getElementById("settings-toggle")!;
  setAttribute(toggleButton, "aria-pressed", String(open));
  if (!open) {
    confirming = false;
    return;
  }
  if (wasHidden && pane === "import") loadSources();
  if (wasHidden || changed) render(wasHidden);
}

/** Moves focus into settings when nothing in the interface has it. */
export function focusSettings() {
  if (!root.hidden && (document.activeElement === document.body || !document.activeElement)) {
    tabs.find((t) => t.dataset.pane === pane)?.focus();
  }
}

/** Theme and accent apply to the whole interface. */
export function applyAppearance(s: Settings) {
  const html = document.documentElement;
  const theme = s.Theme === "System" ? undefined : s.Theme.toLowerCase();
  const accent = s.Accent === "Blue" ? undefined : s.Accent.toLowerCase();
  if (html.dataset.theme !== theme) {
    if (theme === undefined) delete html.dataset.theme;
    else html.dataset.theme = theme;
  }
  if (html.dataset.accent !== accent) {
    if (accent === undefined) delete html.dataset.accent;
    else html.dataset.accent = accent;
  }
}

// ---------- Wiring ----------

for (const tab of tabs) tab.addEventListener("click", () => selectPane(tab.dataset.pane as Pane));
document.getElementById("settings-tabs")!.addEventListener("keydown", (event) => {
  const step = { ArrowDown: 1, ArrowUp: -1 }[event.key];
  if (!step) return;
  event.preventDefault();
  const at = tabs.findIndex((t) => t.dataset.pane === pane);
  selectPane(tabs[(at + step + tabs.length) % tabs.length].dataset.pane as Pane, true);
});
window.addEventListener("keydown", (event) => {
  // The address bar handles its own Escape.
  if (event.key !== "Escape" || root.hidden || (event.target as HTMLElement).id === "address") return;
  event.preventDefault();
  if (confirming) setConfirming(false);
  else backend.setSettingsOpen(false);
});
document.getElementById("settings-toggle")!.addEventListener("click", () => backend.setSettingsOpen(root.hidden));
onUpdateChange(() => render());
onPictureChange(() => render());
