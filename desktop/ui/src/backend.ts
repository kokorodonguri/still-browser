// The interface talks to Rust through this module. Outside Tauri (a plain browser
// during UI tests) it falls back to an in-memory mock with the same behavior.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

export interface TabView { id: number; title: string; url: string; loading: boolean; crashed: boolean }
export interface LinkView { url: string; title: string }
export interface DownloadView { name: string; path: string; state: string; bytes: number }
/** A saved login as the interface sees it: never the password. */
export interface LoginView { index: number; host: string; username: string }
/** A browser profile on this computer that Still can import from. */
export interface ImportSource { id: string; browser: string; profile: string; cookies: boolean }
export interface ImportRequest { bookmarks: boolean; history: boolean; cookies: boolean }
/** Saved preferences. Keys are PascalCase, as in state.json. */
export interface Settings {
  Theme: "System" | "Light" | "Dark";
  Accent: "Blue" | "Purple" | "Pink" | "Orange" | "Green" | "Graphite";
  Sidebar: "Remember" | "Open" | "Closed";
  Search: "Google" | "Bing" | "DuckDuckGo" | "YahooJapan";
  Startup: "Restore" | "NewTab" | "Home";
  HomePage: string;
  /** Empty = the user's Downloads folder. */
  DownloadFolder: string;
  AskDownloadFolder: boolean;
  KeepHistory: boolean;
  /** File name of the new tab picture Rust stored; empty = none. Changed only by setNewTabImage. */
  NewTabImage: string;
}
export interface ClearRequest { history: boolean; siteData: boolean; cache: boolean }
export interface Snapshot {
  revision: number;
  tabs: TabView[];
  active: number;
  url: string;
  loading: boolean;
  canBack: boolean;
  canForward: boolean;
  bookmarked: boolean;
  sidebarOpen: boolean;
  fullscreen: boolean;
  bookmarks: LinkView[];
  history: LinkView[];
  downloads: DownloadView[];
  /** Saved logins for the active page. */
  logins: LoginView[];
  /** Every saved login, for settings. */
  savedLogins: LoginView[];
  downloading: number;
  notice: string | null;
  settingsOpen: boolean;
  settings: Settings;
}
/** Updates omit the lists and settings when they did not change since the last one. */
type Omittable = "tabs" | "bookmarks" | "history" | "downloads" | "savedLogins" | "settings";
export type Update = Omit<Snapshot, Omittable> & Partial<Pick<Snapshot, Omittable>> & { tabChanges?: TabView[] };
export interface Bounds { x: number; y: number; width: number; height: number }
export interface UpdateStatus { current: string; available: string | null; failed: boolean }
type WindowAction = "minimize" | "toggleMaximize" | "close";

interface Backend {
  ready(): Promise<Snapshot>;
  onState(handler: (s: Update) => void): Promise<() => void>;
  onFocusAddress(handler: () => void): Promise<() => void>;
  setContentBounds(bounds: Bounds): void;
  navigate(input: string): void;
  newTab(): Promise<void>;
  closeTab(id: number): void;
  selectTab(id: number): void;
  cycleTab(forward: boolean): void;
  go(action: "back" | "forward" | "reload" | "stop"): void;
  setSidebar(open: boolean): void;
  toggleBookmark(): void;
  removeBookmark(url: string): void;
  openDownloadFolder(path: string): void;
  dismissNotice(): void;
  window(action: WindowAction): void;
  setSettingsOpen(open: boolean): void;
  /** Sends only the changed fields. */
  updateSettings(patch: Partial<Settings>): void;
  pickDownloadFolder(): void;
  /** Stores a copy of a picture file (null removes it). Rust checks the format and size. */
  setNewTabImage(bytes: ArrayBuffer | null): Promise<void>;
  /** The current picture's bytes; empty when there is none. */
  newTabImage(): Promise<ArrayBuffer>;
  clearBrowsingData(request: ClearRequest): void;
  checkUpdate(): Promise<UpdateStatus>;
  /** Downloads, verifies and runs the installer; Still exits when it starts. */
  installUpdate(): Promise<void>;
  importSources(): Promise<ImportSource[]>;
  /** The result arrives as a notice. */
  importFrom(id: string, request: ImportRequest): void;
  /** An exported file's bytes; Rust recognizes bookmarks HTML, passwords CSV or Takeout history JSON. */
  importFile(bytes: ArrayBuffer): Promise<void>;
  fillLogin(index: number): void;
  removeLogin(index: number): void;
  clearLogins(): void;
}

const report = (error: unknown) => console.error(error);

const tauri: Backend = {
  ready: () => invoke<Snapshot>("ready"),
  onState: (handler) => listen<Update>("state", (e) => handler(e.payload)),
  onFocusAddress: (handler) => listen("focus-address", () => handler()),
  setContentBounds: (bounds) => void invoke("set_content_bounds", { bounds }).catch(report),
  navigate: (input) => void invoke("navigate", { input }).catch(report),
  newTab: () => invoke("new_tab"),
  closeTab: (id) => void invoke("close_tab", { id }).catch(report),
  selectTab: (id) => void invoke("select_tab", { id }).catch(report),
  cycleTab: (forward) => void invoke("cycle_tab", { forward }).catch(report),
  go: (action) => void invoke("go", { action }).catch(report),
  setSidebar: (open) => void invoke("set_sidebar", { open }).catch(report),
  toggleBookmark: () => void invoke("toggle_bookmark").catch(report),
  removeBookmark: (url) => void invoke("remove_bookmark", { url }).catch(report),
  openDownloadFolder: (path) => void invoke("open_download_folder", { path }).catch(report),
  dismissNotice: () => void invoke("dismiss_notice").catch(report),
  window: (action) => void getCurrentWindow()[action]().catch(report),
  setSettingsOpen: (open) => void invoke("set_settings_open", { open }).catch(report),
  updateSettings: (patch) => void invoke("update_settings", { patch }).catch(report),
  pickDownloadFolder: () => void invoke("pick_download_folder").catch(report),
  setNewTabImage: (bytes) => invoke("set_new_tab_image", new Uint8Array(bytes ?? new ArrayBuffer(0))),
  newTabImage: () => invoke<ArrayBuffer>("new_tab_image"),
  clearBrowsingData: (request) => void invoke("clear_browsing_data", { ...request }).catch(report),
  checkUpdate: () => invoke<UpdateStatus>("check_update"),
  installUpdate: () => invoke("install_update"),
  importSources: () => invoke<ImportSource[]>("import_sources"),
  importFrom: (id, request) => void invoke("import_from", { id, ...request }).catch(report),
  importFile: (bytes) => invoke("import_file", new Uint8Array(bytes)),
  fillLogin: (index) => void invoke("fill_login", { index }).catch(report),
  removeLogin: (index) => void invoke("remove_login", { index }).catch(report),
  clearLogins: () => void invoke("clear_logins").catch(report),
};

export const DEFAULT_SETTINGS: Settings = {
  Theme: "System",
  Accent: "Blue",
  Sidebar: "Remember",
  Search: "Google",
  Startup: "Restore",
  HomePage: "",
  DownloadFolder: "",
  AskDownloadFolder: false,
  KeepHistory: true,
  NewTabImage: "",
};

const SEARCH_URLS: Record<Settings["Search"], string> = {
  Google: "https://www.google.com/search?q=",
  Bing: "https://www.bing.com/search?q=",
  DuckDuckGo: "https://duckduckgo.com/?q=",
  YahooJapan: "https://search.yahoo.co.jp/search?p=",
};

function createMock(): Backend {
  let nextId = 3;
  const tabs: TabView[] = [
    { id: 1, title: "Example Domain", url: "https://example.org/", loading: false, crashed: false },
    { id: 2, title: "New tab", url: "about:blank", loading: false, crashed: false },
  ];
  const s = {
    active: 1,
    sidebarOpen: false,
    bookmarks: [{ url: "https://example.org/", title: "Example Domain" }] as LinkView[],
    history: [{ url: "https://example.org/", title: "Example Domain" }] as LinkView[],
    downloads: [{ name: "report.pdf", path: "C:\\Users\\me\\Downloads\\report.pdf", state: "Completed", bytes: 248_000 }] as DownloadView[],
    notice: null as string | null,
    settingsOpen: false,
    settings: { ...DEFAULT_SETTINGS },
    cleared: null as ClearRequest | null,
    /** The version the mock site offers; "fail" makes the check or install fail. */
    update: null as string | null,
    installing: null as string | null,
    picture: new ArrayBuffer(0),
    logins: [{ url: "https://example.org", username: "me@example.org" }, { url: "https://example.org", username: "work@example.org" }, { url: "https://other.test", username: "you" }],
    sources: [
      { id: "C:\\Chrome\\Default", browser: "Google Chrome", profile: "Personal", cookies: false },
      { id: "C:\\Firefox\\abc", browser: "Firefox", profile: "default-release", cookies: true },
    ] as ImportSource[],
    imported: null as (ImportRequest & { id: string }) | null,
    filled: null as number | null,
  };
  const loginViews = () => s.logins.map((l, index) => ({ index, host: new URL(l.url).host, username: l.username }));
  const listeners = new Set<(s: Update) => void>();
  let revision = 0;
  let previous: Snapshot | undefined;
  const snapshot = (): Snapshot => {
    const tab = tabs.find((t) => t.id === s.active)!;
    const url = tab.url === "about:blank" ? "" : tab.url;
    return {
      revision: ++revision,
      tabs: tabs.map((t) => ({ ...t })),
      active: s.active,
      url,
      loading: tab.loading,
      canBack: !!url,
      canForward: false,
      bookmarked: s.bookmarks.some((b) => b.url === url),
      sidebarOpen: s.sidebarOpen,
      fullscreen: false,
      bookmarks: [...s.bookmarks],
      history: [...s.history],
      downloads: [...s.downloads],
      logins: url ? loginViews().filter((l) => l.host === new URL(url).host) : [],
      savedLogins: loginViews(),
      downloading: s.downloads.filter((d) => d.state === "InProgress").length,
      notice: s.notice,
      settingsOpen: s.settingsOpen,
      settings: { ...s.settings },
    };
  };
  // The mock emits the same full-list / row-delta protocol as Rust. Diffing
  // here is test-only; production tracks changed IDs directly in the session.
  const publish = () => {
    const next = snapshot();
    const update: Update = { ...next };
    if (previous) {
      if (next.tabs.length === previous.tabs.length && next.tabs.every((t, i) => t.id === previous!.tabs[i].id)) {
        delete update.tabs;
        update.tabChanges = next.tabs.filter((t, i) => {
          const old = previous!.tabs[i];
          return t.title !== old.title || t.url !== old.url || t.loading !== old.loading || t.crashed !== old.crashed;
        });
      }
      for (const key of ["bookmarks", "history", "downloads", "savedLogins", "settings"] as const) {
        if (JSON.stringify(next[key]) === JSON.stringify(previous[key])) delete update[key];
      }
    }
    previous = next;
    listeners.forEach((listener) => listener(update));
  };
  const active = () => tabs.find((t) => t.id === s.active)!;
  (window as unknown as { stillMock: unknown }).stillMock = {
    tabs, state: s, publish, snapshot, bounds: null as Bounds | null,
    // Exercise delivery order separately from normal mock state mutations.
    emit: (update: Update) => listeners.forEach((listener) => listener(update)),
  };
  const mock = (window as unknown as { stillMock: { bounds: Bounds | null } }).stillMock;
  return {
    ready: async () => { previous = snapshot(); return previous; },
    onState: async (h) => { listeners.add(h); return () => { listeners.delete(h); }; },
    onFocusAddress: async () => () => {},
    setContentBounds: (b) => void (mock.bounds = b),
    navigate: (input) => {
      s.settingsOpen = false;
      const url = /^https?:\/\//.test(input) ? input : input.includes(".") && !input.includes(" ") ? `https://${input}/` : `${SEARCH_URLS[s.settings.Search]}${encodeURIComponent(input)}`;
      Object.assign(active(), { url, title: new URL(url).host });
      s.history.unshift({ url, title: new URL(url).host });
      publish();
    },
    newTab: async () => {
      s.settingsOpen = false;
      tabs.push({ id: nextId, title: "New tab", url: "about:blank", loading: false, crashed: false });
      s.active = nextId++;
      publish();
    },
    closeTab: (id) => {
      const i = tabs.findIndex((t) => t.id === id);
      if (i < 0) return;
      tabs.splice(i, 1);
      if (!tabs.length) tabs.push({ id: nextId++, title: "New tab", url: "about:blank", loading: false, crashed: false });
      if (s.active === id) s.active = tabs[Math.min(i, tabs.length - 1)].id;
      publish();
    },
    selectTab: (id) => {
      s.active = id;
      s.settingsOpen = false;
      publish();
    },
    cycleTab: (forward) => {
      const i = tabs.findIndex((t) => t.id === s.active);
      s.active = tabs[(i + (forward ? 1 : tabs.length - 1)) % tabs.length].id;
      publish();
    },
    go: () => {},
    setSidebar: (open) => {
      s.sidebarOpen = open;
      publish();
    },
    toggleBookmark: () => {
      const url = active().url;
      const i = s.bookmarks.findIndex((b) => b.url === url);
      if (i >= 0) s.bookmarks.splice(i, 1);
      else s.bookmarks.unshift({ url, title: active().title });
      publish();
    },
    removeBookmark: (url) => {
      s.bookmarks = s.bookmarks.filter((b) => b.url !== url);
      publish();
    },
    openDownloadFolder: () => {},
    dismissNotice: () => {
      s.notice = null;
      publish();
    },
    window: () => {},
    setSettingsOpen: (open) => {
      s.settingsOpen = open;
      publish();
    },
    updateSettings: (patch) => {
      // Like Rust: a home page that is not a web address keeps the previous one.
      if (patch.HomePage !== undefined) {
        const text = patch.HomePage.trim();
        const valid = !text || /^https?:\/\//.test(text) || (!text.includes(" ") && text.includes("."));
        patch = { ...patch, HomePage: !valid ? s.settings.HomePage : !text || /^https?:\/\//.test(text) ? text : `https://${text}/` };
      }
      Object.assign(s.settings, patch);
      publish();
    },
    pickDownloadFolder: () => {
      s.settings.DownloadFolder = "C:\\Users\\me\\Desktop";
      publish();
    },
    setNewTabImage: async (bytes) => {
      // Like Rust: the format comes from the first bytes, and large files are refused.
      const head = Array.from(new Uint8Array(bytes ?? new ArrayBuffer(0), 0, Math.min(12, bytes?.byteLength ?? 0)), (b) => String.fromCharCode(b)).join("");
      const ext = !bytes?.byteLength ? "" : head.startsWith("\x89PNG") ? "png" : head.startsWith("\xff\xd8\xff") ? "jpg" : head.startsWith("GIF8") ? "gif" : head.startsWith("RIFF") && head.slice(8) === "WEBP" ? "webp" : null;
      if (ext === null || (bytes && bytes.byteLength > 20 * 1024 * 1024)) {
        s.notice = "That picture cannot be used. Choose a PNG, JPEG, GIF, WebP, AVIF or BMP file up to 20 MB.";
      } else {
        s.picture = bytes ?? new ArrayBuffer(0);
        s.settings.NewTabImage = ext ? `newtab-${Date.now()}.${ext}` : "";
      }
      publish();
    },
    newTabImage: async () => s.picture,
    clearBrowsingData: (request) => {
      s.cleared = request;
      if (request.history) s.history = [];
      s.notice = "Browsing data was cleared.";
      publish();
    },
    checkUpdate: async () => ({ current: "0.2.0", available: s.update === "fail" ? null : s.update, failed: s.update === "fail" }),
    installUpdate: async () => {
      if (s.installing === "fail") throw new Error("update failed");
      s.installing = s.update;
    },
    importSources: async () => s.sources.map((source) => ({ ...source })),
    importFrom: (id, request) => {
      s.imported = { id, ...request };
      const source = s.sources.find((x) => x.id === id);
      if (request.bookmarks) s.bookmarks.unshift({ url: "https://imported.test/", title: "Imported" });
      s.notice = `Imported 1 bookmark from ${source?.browser} (${source?.profile}).`;
      publish();
    },
    importFile: async (bytes) => {
      // Like Rust: the contents decide the kind.
      const text = new TextDecoder().decode(bytes).trimStart();
      if (/^<!doctype netscape-bookmark/i.test(text)) {
        s.bookmarks.unshift({ url: "https://from-file.test/", title: "From file" });
        s.notice = "Imported 1 bookmark.";
      } else if (/^[^\n]*\bpassword\b/i.test(text)) {
        s.logins.push({ url: "https://imported.test", username: "imported" });
        s.notice = "Imported 1 password.";
      } else {
        s.notice = "That file cannot be imported. Choose a bookmarks file (HTML), a passwords file (CSV) or a Google Takeout history file (JSON).";
      }
      publish();
    },
    fillLogin: (index) => void (s.filled = index),
    removeLogin: (index) => {
      s.logins.splice(index, 1);
      publish();
    },
    clearLogins: () => {
      s.logins = [];
      publish();
    },
  };
}

const inTauri = "__TAURI_INTERNALS__" in window;
export const backend: Backend = inTauri ? tauri : createMock();
