# Still for Windows — design

Still is a minimal browser. Every control it draws is HTML/CSS, so no Windows-native widgets appear in the browser chrome. Pages render in WebView2.

## 1. Requirements

| # | Requirement | Notes |
|---|---|---|
| R1 | Browse with tabs: address/search bar, back/forward/reload, new/close/switch tabs | Text that is not a URL becomes a Google search |
| R2 | Bookmarks, history (latest 500), downloads list | |
| R3 | Restore the previous session; restored background tabs load lazily | |
| R4 | No Windows-native UI in the browser chrome | The page area is WebView2. The download folder picker and the "ask where to save" dialog are Windows' own file dialogs, like WebView2's prompts |
| R5 | Never lose user data | Single instance, atomic writes, backups for corrupt or newer files |
| R6 | Recover from failures | Missing WebView2, renderer crash, browser-process crash, unwritable disk |
| R7 | Sign-in popups (OAuth) work | `window.open` with a size keeps `window.opener` |
| R8 | HTML fullscreen (video) fills the screen | |
| R9 | Behave like a desktop app | Command-line URLs, remembered window placement, per-monitor DPI |
| R10 | Operable | Log file, panic log, installer, x64 + ARM64, CI |
| R11 | Settings | Theme, accent color, sidebar at startup, search engine, startup tabs, home page, download folder, ask where to save, keep history, clear browsing data |
| R12 | Move from another browser | Bookmarks and history from Chrome, Edge, Brave, Vivaldi, Opera and Firefox; Firefox cookies; exported files (bookmarks HTML, passwords CSV, Google Takeout history JSON) |
| R13 | Saved passwords | Imported logins are encrypted for the Windows user and filled in on request; WebView2 offers to save new ones |

Out of scope for the MVP:
- Code signing (Authenticode; updates are signed separately, see §8)
- Extensions, sync, private windows
- Custom permission and error-page UI (WebView2's own prompts are used)

## 2. Edge cases

| Area | Case | Handling |
|---|---|---|
| Startup | WebView2 Runtime missing | A message box explains the problem and offers the download page, then the app exits. (A native box is acceptable here: the app cannot run at all.) |
| Startup | Second launch, possibly with URLs | The single-instance plugin forwards the arguments. The running window opens them as tabs and comes to the front. |
| Startup | Command-line argument is not http(s) (`file:`, `javascript:`, flags) | Ignored |
| State | File missing | Fresh session |
| State | File corrupt | Fresh session and notice; backup to `state.json.invalid-<time>` must succeed before saving |
| State | File from a newer schema version | Backup to `state.json.v<N>-backup` must succeed before the first write; existing backups are preserved under unique names |
| State | File unreadable or disk full | Notice and log entry; the next change retries the save |
| State | Written by the old C# build (no version, nulls, PascalCase) | Read as version 1 |
| State | A setting has an unknown or mistyped value | That field falls back to its default; the rest of the file is kept |
| State | Process killed | At most `SAVE_DELAY` (0.8 s) of changes lost. Writes are atomic (temp file + rename), so the file is never left half-written. |
| Window | Saved position is off-screen (monitor unplugged) | Window is centered |
| Window | Moved to a monitor with a different DPI | Page bounds are re-applied on `ScaleFactorChanged` |
| Window | Minimized (content box is 0×0) | Size is clamped to at least 1 px |
| Tabs | Closing the last tab | A blank tab replaces it |
| Tabs | Closing or switching while a page is still being created | A generation check rejects stale creations and callbacks; closed tabs cannot retain a late webview |
| Tabs | Page title is 1 MB or empty | Truncated to 300 characters; empty title falls back to the host name |
| Tabs | Thousands of tabs | Only the visible tab is touched on switch or resize; others are suspended |
| Memory | Windows reports low physical memory | Every 5 s while it lasts, the least recently shown background page that was checked as holding nothing to lose is closed. The tab keeps its address and title and loads again when selected (its back/forward history is lost). Never released: the active, visible or fullscreen page, a page playing audio, the source of a running download, a page with edited fields, an editable region, an unload warning or a frame it cannot inspect, a page whose check failed |
| Pages | Renderer crashed or unresponsive | Tab is marked, a notice is shown, and Reload recovers |
| Pages | Browser process exited | All page webviews are discarded and the active one is recreated |
| Pages | `window.open` with a size (OAuth) | Real popup window sharing the environment, so `opener` works |
| Pages | `target=_blank` / `window.open` without a size | Opens as a new tab |
| Pages | HTML fullscreen | Window goes fullscreen and the page covers it. Switching tabs exits fullscreen. |
| Pages | Page tries to call the app over IPC | No capability grants it, and every command also checks the caller's label |
| Downloads | Finished event without a path | Matched to the newest in-progress download |
| Downloads | List grows forever | Capped at 200; the oldest finished entries are dropped |
| Downloads | "Show in folder" with an unknown or removed path | Only recorded paths are accepted, and a notice is shown if the folder is gone |
| Address bar | Over 32k characters or invalid | Notice; nothing is navigated |
| Settings | Opened while a page is fullscreen | Fullscreen ends; the page is hidden while settings are shown |
| Settings | Home page entry is not a web address | The interface shows an error and keeps the text; Rust also ignores it and keeps the previous home page |
| Settings | "Open the home page" with no home page | A blank tab opens |
| Settings | Clearing site data with no page open | A hidden helper webview on the page profile does the work, then closes |
| Downloads | Chosen download folder was removed | Saved to the default folder, with a notice |
| Downloads | Name already exists in the chosen folder | `name (1).ext`, `name (2).ext`, … |
| Downloads | "Ask where to save" dialog cancelled | The download is cancelled and removed from the list |
| Panic | Release build (`panic = "abort"`) | The panic hook writes the message and location to the log first |
| Import | The other browser is running and holds its databases | Each database (and its `-wal`/`-journal`) is copied to `Import\` first and read there; the folder is removed afterwards and at startup |
| Import | Chrome or Edge cookies and passwords | Not read: they are encrypted for that browser alone (app-bound encryption). The Import pane says to export passwords as CSV and to sign in again |
| Import | Imported twice | Bookmarks already saved and visits no newer than one in history are skipped; logins with the same site and user name take the imported password |
| Import | More than 500 history entries | Visits merge with history by time; the newest 500 remain |
| Import | Save browsing history is off | History is not imported |
| Import | Firefox container or partitioned cookies | Skipped: Still has neither, so they would leak into the normal profile |
| Import | A file that is not HTML bookmarks, CSV passwords or Takeout JSON, or over 64 MB | Refused with a notice |
| Passwords | `passwords.dat` cannot be decrypted (another Windows user, damaged) | Renamed to `passwords.dat.unreadable-<time>` with a notice; if renaming fails, the vault is never written over it |
| Passwords | Page navigated between offering a login and filling it | The fill script checks `location.hostname` first and does nothing |
| Passwords | A login offered on another site | Only the same host or a parent/child domain matches (a dotless host never matches a subdomain); https logins are never offered on http pages |

## 3. Architecture

```
┌──────────────────────── Window "main" (undecorated) ────────────────────────┐
│ ┌─ webview "chrome" (ui/dist, local, IPC allowed) ────────────────────────┐ │
│ │ toolbar · sidebar · notices — HTML/CSS                                  │ │
│ │       ┌─ child webview "tab-<id>" (remote page, no IPC) ───────────────┐│ │
│ │       │  positioned over the content box; one visible at a time       ││ │
│ │       └────────────────────────────────────────────────────────────────┘│ │
│ └─────────────────────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────────────────┘
  popup windows "popup-<n>" (OAuth): native frame, shared WebView2 environment
```

| Layer | Crate / folder | Responsibility | Can be tested on Linux |
|---|---|---|---|
| Domain | `core/` (`still-core`) | Address normalization, state schema and migrations, the `Session` state machine (tabs, view plan, library, notices, snapshot) | Yes — pure Rust |
| Shell | `src-tauri/` | Windows, webviews, threads, file I/O, logging, single instance, WebView2 COM hooks | Compile-checked with `cargo xwin clippy` |
| Interface | `ui/` | Renders snapshots; sends intents | Yes — Selenium against a mock backend |

The design rule: `Session` decides, the shell executes. Every mutation runs under one mutex and returns a plan, for example `ShowPlan { hide, action }`. The shell carries out the plan only after releasing the lock, because webview calls block on the main thread.

## 4. Data flow

```
UI intent ──invoke──▶ command (checks caller label)
                          │ lock
                          ▼
                   Session::method()  ──▶ plan / notice / dirty flag
                          │ unlock
                          ▼
             shell executes the plan (create / show / hide / navigate a webview)
                          │
     WebView2 events ─────┘ (title, load, history, download, crash, fullscreen)
                          │ lock → Session::on_* → unlock
                          ▼
   publish()  ─ coalesced to one per 16 ms ─▶ "state" event → UI (one render per frame)
   save()     ─ coalesced to one per 800 ms ─▶ serialized writer → state.json (atomic)
```

## 5. Persistence schema

File: `%LOCALAPPDATA%\StillBrowser\state.json`. It is JSON with PascalCase keys, compatible with the C# build.

```jsonc
{
  "Version": 2,                       // absent = 1 (C# build)
  "SidebarOpen": false,
  "Tabs": [{ "Url": "https://…" }],
  "ActiveIndex": 0,
  "Bookmarks": ["https://…"],
  "BookmarkTitles": { "https://…": "…" },  // imported titles; written only when not empty
  "History": [{ "Url": "…", "VisitedAt": "2026-10-09T05:00:00+00:00", "Title": "…" }],   // ≤ 500
  "Downloads": [{ "Path": "C:\\…", "State": "Completed|Failed|Interrupted|InProgress", "Bytes": 0, "Total": 0 }], // ≤ 200
  "Window": { "X": 100, "Y": 80, "Width": 1440, "Height": 900, "Maximized": false }, // physical px
  "Settings": {                       // absent = defaults; read leniently per field
    "Theme": "System|Light|Dark",
    "Accent": "Blue|Purple|Pink|Orange|Green|Graphite",
    "Sidebar": "Remember|Open|Closed",
    "Search": "Google|Bing|DuckDuckGo|YahooJapan",
    "Startup": "Restore|NewTab|Home",
    "HomePage": "https://…",          // empty = none
    "DownloadFolder": "D:\\…",         // empty = the user's Downloads folder
    "AskDownloadFolder": false,
    "KeepHistory": true
  }
}
```

Other on-disk data:
- `%LOCALAPPDATA%\StillBrowser\WebView2\` — cookies, cache and logins (WebView2 user data)
- `%LOCALAPPDATA%\StillBrowser\logs\still.log` — rotated at 1 MB to `still.1.log`
- `%LOCALAPPDATA%\StillBrowser\passwords.dat` — imported logins, `{"Logins":[{"Url":"https://host","Username":"…","Password":"…"}]}` encrypted with DPAPI (`CryptProtectData`, current user)

## 6. IPC API (chrome webview only)

| Command | Args | Effect |
|---|---|---|
| `ready` | — | Full `Snapshot` |
| `set_content_bounds` | `{x,y,width,height}` CSS px | Place the active page |
| `navigate` | `input` | Normalize, then navigate the active tab |
| `new_tab` / `close_tab` / `select_tab` / `cycle_tab` | `id` / `forward` | Tab management |
| `go` | `back` \| `forward` \| `reload` \| `stop` | Reload also recovers crashed tabs |
| `set_sidebar` | `open` | |
| `toggle_bookmark` / `remove_bookmark` | — / `url` | |
| `open_download_folder` | `path` (must be a recorded download) | |
| `dismiss_notice` | — | |
| `set_settings_open` | `open` | Hides the page while settings are shown |
| `update_settings` | `patch` (only the changed fields, PascalCase) | A theme change also sets the pages' `prefers-color-scheme` |
| `pick_download_folder` | — | Folder dialog on the main thread; stores the choice |
| `clear_browsing_data` | `history`, `siteData`, `cache` | Notice when done |
| `check_update` | — | `{current, available, failed}` from the download site; keeps the found update |
| `install_update` | — | Download, verify, save the session, run the installer and exit |
| `import_sources` | — | Browser profiles found under `%LOCALAPPDATA%` / `%APPDATA%`: `{id, browser, profile, cookies}` |
| `import_from` | `id`, `bookmarks`, `history`, `cookies` | Copies and reads the profile's databases off the main thread; the result is a notice |
| `import_file` | raw body (the file's bytes) | Recognized by content: bookmarks HTML, passwords CSV or Takeout history JSON |
| `fill_login` | `index` | Runs the fill script in the active page if the login matches its site |
| `remove_login` / `clear_logins` | `index` / — | |

Events to the chrome webview:
- `state` — a `Snapshot` with monotonic `revision`. `logins` lists the saved logins for the active page and `savedLogins` all of them (omitted when unchanged); both hold the site and user name only, never a password. `tabs` is a complete ordered list on initial/structural updates; otherwise `tabChanges` contains only changed rows. Each library list and `settings` are independently omitted when unchanged. The bookmark list is resent after a page load only when a bookmarked row's title can change. The interface subscribes before `ready` and ignores older revisions.
- `focus-address`

## 7. Performance

- **Saves:** coalesced. A writer mutex covers capture through replacement so shutdown waits for in-flight saves. Capture owns its data; JSON encoding, mandatory backups and synced writes happen outside the session lock. Failed backups block replacement and are retried.
- **UI updates:** coalesced to one per frame. Library lists are sent only when they change.
- **Tab switching and resizing:** indexed tab lookup and O(1) webview calls. Title/loading/crash events transmit only changed rows. Full ordered lists are still needed when tabs are added or removed; closing repairs the vector's shifted index suffix.
- **Hidden tabs:** WebView2's low memory target plus `TrySuspend`. Pages playing audio are left running. Before suspending, a script records whether the page holds anything that closing it would lose; a suspended page cannot change, so the record holds until it is shown again.
- **Memory pressure (Windows):** a thread waits in the kernel on the low-memory resource notification and then releases checked background pages, least recently shown first (see §2). Without the notification nothing is released.
- **Rust heap:** the saved file is read straight into typed structures (no whole-document JSON tree) and saves are encoded straight into the temporary file. Tab tables give back capacity after most tabs close. Heap measurements: `core/examples/memory.rs`.
- **Restore:** lazy. A tab gets a webview only when first shown.
- **UI rendering:** at most one render per frame. Tab rows are keyed by ID and reused; a title change updates one text node. Selection touches the previous and next row. Hidden sidebar updates are accumulated until shown. Unchanged toolbar/theme attributes are not written.

Measurements, test coverage and remaining native-runtime checks: [PERFORMANCE.md](PERFORMANCE.md).

## 8. Updates

`tauri-plugin-updater` reads `https://still.dongurihub.com/update/windows.json` (written by `site/build.py`). The interface checks once at startup and from Settings > General > Software update; a newer version shows an **Update** button in the toolbar. Nothing is downloaded until the user presses it.

| Case | Handling |
|---|---|
| Site unreachable or answer invalid | A startup check stays quiet; a manual check says so |
| Installer altered in transit (the site is plain HTTP) | Rejected: every installer must carry a minisign signature that matches `plugins.updater.pubkey` |
| Same or older version on the site | Not offered (semver comparison) |
| Unsaved session at install time | `install_update` saves before the plugin exits the process (`RunEvent::Exit` does not run) |
| Installed copy | NSIS runs passively with `/UPDATE /R` and starts Still again with the same arguments |
| Portable EXE | The installer installs Still per user; the portable file is left as it is. Data is shared (`%LOCALAPPDATA%\StillBrowser`) |

The private key is `~/.tauri/still-updater.key` on the build machine (never in the repository); `tauri build` signs with it through `TAURI_SIGNING_PRIVATE_KEY`. Losing it means installed copies can no longer update: they only accept the public key they were built with.

## 9. Moving from another browser

Settings > Import lists the browser profiles on this PC and imports from the chosen one; Settings > Passwords lists the saved logins.

| Source | Bookmarks | History | Cookies | Passwords |
|---|---|---|---|---|
| Chrome, Edge, Brave, Vivaldi, Chromium (`%LOCALAPPDATA%\…\User Data\<profile>`), Opera (`%APPDATA%\Opera Software\…`) | `Bookmarks` (JSON) | `History` (SQLite) | No: app-bound encryption | Export CSV in that browser |
| Firefox (`%APPDATA%\Mozilla\Firefox\profiles.ini`) | `places.sqlite` | `places.sqlite` | `cookies.sqlite`, added through WebView2's cookie manager | Export CSV in Firefox |
| File | Netscape bookmarks HTML (every browser exports it) | Google Takeout `Chrome/History.json` | — | CSV from Chrome, Edge, Firefox, Safari or a password manager |

Parsing lives in `core/src/import.rs` (SQLite through `rusqlite`, bundled) and merging in `Session::import_library` / `import_logins`, so both are tested on Linux. Bookmark titles from the other browser are kept in `BookmarkTitles` until a visit gives the page a title.

Filling a login: when the vault holds logins for the active page, a key appears in the address bar. With one login, pressing it fills that login; with several, the choices cover the address field (a popup would sit under the page webview). Rust checks again that the login matches the page, then evaluates `passwords::fill_script` in the top frame. The script re-checks `location.hostname`, fills only visible, enabled fields, and dispatches `input`/`change` so page scripts notice. Sign-in forms inside cross-origin frames and popup windows are not filled. WebView2's own password saving (`IsPasswordAutosaveEnabled`) is turned on, so passwords typed or filled into a form can be saved and later filled by WebView2 itself.
