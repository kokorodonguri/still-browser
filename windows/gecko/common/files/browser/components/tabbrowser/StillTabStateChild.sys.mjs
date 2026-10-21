/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

// Only a boolean crosses IPC. Never collect or send form values.
export class StillTabStateChild extends JSWindowActorChild {
  handleEvent(event) {
    if (event.isTrusted) {
      // Sticky for this document, including custom editors and shadow DOM.
      // Keeping a submitted form alive is preferable to losing an unsaved draft.
      this._edited = true;
      this.retain();
    }
  }

  receiveMessage({ name }) {
    if (name !== "Still:PassesRetentionChecks") {
      return false;
    }
    try {
      const doc = this.document;
      if (this._edited || this._retained || !doc || doc.hidden === false || doc.readyState !== "complete" ||
          doc.designMode === "on") {
        return false;
      }
      // This is a retention heuristic, never a guarantee that JS state is saved.
      if (!this._candidates) {
        this.collectCandidates(doc);
      }
      if (this._observer?.takeRecords().length) {
        this.retain();
      }
      return !this._retained && this._candidates.every(element => this.unchanged(element));
    } catch (error) {
      this.retain();
      console.error(error);
      return false;
    }
  }

  collectCandidates(doc) {
    this._candidates = [];
    // One bounded walk per document, without allocating an all-elements list.
    // Any later DOM mutation retains the document rather than scanning again.
    this._observer = new this.contentWindow.MutationObserver(() => this.retain());
    this._observer.observe(doc, { subtree: true, childList: true, attributes: true, characterData: true });
    const walker = doc.createTreeWalker(doc, 1 /* SHOW_ELEMENT */);
    let element;
    let visited = 0;
    while ((element = walker.nextNode())) {
      const shadow = element.openOrClosedShadowRoot;
      if (++visited > 5000 || element.localName === "canvas" ||
          element.isContentEditable || element.isFormAssociatedCustomElement ||
          element.localName.includes("-") || (shadow && !shadow.isUAWidget())) {
        this._retained = true;
        break;
      }
      if (["input", "textarea", "select", "audio", "video"].includes(element.localName)) {
        this._candidates.push(element);
        if (this._candidates.length > 256) {
          this._retained = true;
          break;
        }
      }
    }
    if (this._retained) {
      this.retain();
    }
  }

  retain() {
    this._retained = true;
    this._observer?.disconnect();
    this._candidates = [];
  }

  didDestroy() {
    this._observer?.disconnect();
    this._candidates = null;
  }

  unchanged(element) {
    switch (element.localName) {
      case "input": return !this.changedInput(element);
      case "textarea": return element.value === element.defaultValue;
      case "select": return !this.changedSelect(element);
      case "audio":
      case "video": return element.paused || element.ended;
      default: return false;
    }
  }

  changedInput(element) {
    switch (element.type) {
      case "file":
        return !!element.files?.length;
      case "checkbox":
      case "radio":
        return element.checked !== element.defaultChecked;
      default:
        // Range/color/date/number values can be normalized from the HTML
        // attribute. A detached native clone gives the pristine value without
        // resetting or modifying the live form.
        const pristine = element.cloneNode(false);
        pristine.value = pristine.defaultValue;
        return element.value !== pristine.value;
    }
  }

  changedSelect(element) {
    const options = [...element.options];
    if (element.multiple) {
      return options.some(option => option.selected !== option.defaultSelected);
    }
    let expected = options.findLastIndex(option => option.defaultSelected);
    if (expected === -1 && element.size <= 1) {
      expected = options.findIndex(option => !option.disabled &&
        !(option.parentElement?.localName === "optgroup" && option.parentElement.disabled));
    }
    return element.selectedIndex !== expected;
  }
}
