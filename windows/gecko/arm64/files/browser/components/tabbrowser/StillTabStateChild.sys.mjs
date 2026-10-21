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
    }
  }

  receiveMessage({ name }) {
    if (name !== "Still:CanUnload") {
      return false;
    }
    try {
      const doc = this.document;
      if (this._edited || !doc || doc.hidden === false || doc.readyState !== "complete" ||
          doc.designMode === "on") {
        return false;
      }
      return this.safeRoot(doc);
    } catch (error) {
      console.error(error);
      return false;
    }
  }

  safeRoot(root) {
    for (const element of root.querySelectorAll("*")) {
      // Conservatively retain contenteditable, even when no input event was
      // observed (e.g. restored drafts or edits before actor registration).
      if (element.isContentEditable || element.isFormAssociatedCustomElement) {
        return false;
      }
      switch (element.localName) {
        case "input":
          if (this.changedInput(element)) {
            return false;
          }
          break;
        case "textarea":
          if (element.value !== element.defaultValue) {
            return false;
          }
          break;
        case "select":
          if (this.changedSelect(element)) {
            return false;
          }
          break;
        case "audio":
        case "video":
          if (!element.paused && !element.ended) {
            return false;
          }
          break;
      }
      // openOrClosedShadowRoot is a chrome-only API; closed editor roots need
      // the same protection as ordinary forms.
      const shadow = element.openOrClosedShadowRoot;
      // Native text controls have an internal editable UA shadow tree even
      // when pristine. Their value/checked state was checked above.
      if (shadow && !shadow.isUAWidget() && !this.safeRoot(shadow)) {
        return false;
      }
    }
    return true;
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
