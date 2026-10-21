/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

import { setInterval, clearInterval, setTimeout, clearTimeout } from "resource://gre/modules/Timer.sys.mjs";
import { PrivateBrowsingUtils } from "resource://gre/modules/PrivateBrowsingUtils.sys.mjs";
import { webrtcUI } from "resource:///modules/webrtcUI.sys.mjs";

const ENABLED = "still.memory.autoUnload.enabled";
const MINUTES = "still.memory.autoUnload.inactiveMinutes";
const INTERVAL = 30_000;
const QUERY_TIMEOUT = 2_000;

// A single controller for all windows. Uses normal session restore/discard,
// never force-discard, synthetic memory pressure or working-set trimming.
export const StillTabUnloader = {
  init() {
    if (this._initialized) {
      return;
    }
    this._initialized = true;
    ChromeUtils.registerWindowActor("StillTabState", {
      child: {
        esModuleURI: "moz-src:///browser/components/tabbrowser/StillTabStateChild.sys.mjs",
        events: { input: { capture: true }, change: { capture: true },
          pointerdown: { capture: true }, keydown: { capture: true } },
      },
      allFrames: true,
    });
    Services.prefs.addObserver(ENABLED, this);
    this.updateTimer();
    Services.obs.addObserver(this, "quit-application-granted");
  },

  updateTimer() {
    clearInterval(this._timer);
    this._timer = Services.prefs.getBoolPref(ENABLED, false)
      ? setInterval(() => this.tick().catch(console.error), INTERVAL) : null;
  },

  observe(_subject, topic) {
    if (topic === "nsPref:changed") {
      this.updateTimer();
      return;
    }
    Services.prefs.removeObserver(ENABLED, this);
    clearInterval(this._timer);
    this._timer = null;
    this._stopped = true;
    Services.obs.removeObserver(this, "quit-application-granted");
    ChromeUtils.unregisterWindowActor("StillTabState");
  },

  eligible(tab, gBrowser) {
    const browser = tab.linkedBrowser;
    const minutes = Math.max(1, Math.min(1440, Services.prefs.getIntPref(MINUTES, 5)));
    if (
      this._stopped || !Services.prefs.getBoolPref(ENABLED, false) ||
      !tab.isConnected || tab.selected || tab.closing || tab.pinned || tab.undiscardable ||
      tab.hasAttribute("pending") || tab.hasAttribute("busy") ||
      tab.soundPlaying || tab.pictureinpicture ||
      !Number.isFinite(tab.lastAccessed) || Date.now() - tab.lastAccessed < minutes * 60_000 ||
      !browser?.isConnected || !browser.isRemoteBrowser || browser.hasBeforeUnload ||
      tab.ownerDocument.defaultView.closed || PrivateBrowsingUtils.isBrowserPrivate(browser) ||
      !["http", "https", "file"].includes(browser.currentURI.scheme) ||
      browser.webProgress.isLoadingDocument || webrtcUI.browserHasStreams(browser)
    ) {
      return false;
    }
    const bc = browser.browsingContext;
    if (!bc?.currentWindowGlobal || bc.mediaController?.isPlaying ||
        bc.currentWindowGlobal.hasActivePeerConnections()) {
      return false;
    }
    return !gBrowser.getTabDialogBox(browser)._tabDialogManager._dialogs.length;
  },

  // Snapshot every frame, including Fission's out-of-process subframes.
  frames(bc) {
    const result = [];
    const visit = context => {
      const global = context.currentWindowGlobal;
      if (!global) {
        throw new Error("Frame state unavailable");
      }
      result.push(global);
      for (const child of context.children) {
        visit(child);
      }
    };
    visit(bc);
    return result;
  },

  async passesRetentionChecks(frames) {
    let timeout;
    try {
      return await Promise.race([
        Promise.all(frames.map(global =>
          global.getActor("StillTabState").sendQuery("Still:PassesRetentionChecks")
        )).then(results => results.every(result => result === true)),
        new Promise(resolve => { timeout = setTimeout(() => resolve(false), QUERY_TIMEOUT); }),
      ]);
    } catch (error) {
      // Navigation, crashed frames and unavailable actors must be retained.
      return false;
    } finally {
      clearTimeout(timeout);
    }
  },

  async tick() {
    if (this._running || this._stopped || !Services.prefs.getBoolPref(ENABLED, false)) {
      return;
    }
    this._running = true;
    try {
      const candidates = [];
      for (const win of Services.wm.getEnumerator("navigator:browser")) {
        if (!win.closed && win.gBrowser) {
          for (const tab of win.gBrowser.tabs) {
            candidates.push({ tab, gBrowser: win.gBrowser });
          }
        }
      }
      candidates.sort((a, b) => a.tab.lastAccessed - b.tab.lastAccessed);
      for (const { tab, gBrowser } of candidates) {
        try {
          if (!this.eligible(tab, gBrowser)) {
            continue;
          }
          const browser = tab.linkedBrowser;
          const frames = this.frames(browser.browsingContext);
          if (!await this.passesRetentionChecks(frames)) {
            continue;
          }
          await gBrowser.prepareDiscardBrowser(tab);
          // Recheck after the asynchronous session flush: a user may select,
          // edit, navigate, start media, or change the preference while waiting.
          if (!this.eligible(tab, gBrowser) || browser !== tab.linkedBrowser ||
              !await this.passesRetentionChecks(frames) || !this.eligible(tab, gBrowser)) {
            continue;
          }
          const current = this.frames(browser.browsingContext);
          if (current.length !== frames.length || current.some((global, i) => global !== frames[i])) {
            continue;
          }
          if (gBrowser.discardBrowser(tab)) {
            break; // At most one successful discard across all windows per tick.
          }
        } catch (error) {
          // One failed tab must not disable later attempts or other windows.
          console.error(error);
        }
      }
    } finally {
      this._running = false;
    }
  },
};
