import type { TabView } from "./backend";
import { el, icon, setAttribute, setText } from "./dom";

const CLOSE = '<path d="M6 6l8 8M14 6l-8 8" />';
const PLUS = '<path d="M10 4.5v11M4.5 10h11" />';

interface TabRow {
  node: HTMLButtonElement;
  title: HTMLSpanElement;
  detail: HTMLSpanElement;
  dot: HTMLSpanElement;
  data?: TabView;
}

/** Stable rows preserve focus, scroll and event handlers during page events. */
export class TabList {
  private rows = new Map<number, TabRow>();
  private selected: number | undefined;
  private add: HTMLButtonElement;

  constructor(private list: HTMLElement, private select: (id: number) => void, private close: (id: number) => void, add: () => void) {
    this.add = el("button", { className: "row add", title: "New tab" }, icon(PLUS),
      el("span", { className: "row-text" }, el("span", { className: "row-title", textContent: "New tab" })));
    this.add.setAttribute("role", "listitem");
    this.add.addEventListener("click", add);
  }

  clear() {
    this.rows.clear();
    this.selected = undefined;
  }

  private create(id: number): TabRow {
    const title = el("span", { className: "row-title" });
    const detail = el("span", { className: "row-detail", hidden: true });
    const dot = el("span", { className: "dot", hidden: true });
    const close = el("span", { className: "icon small trailing", title: "Close tab", tabIndex: 0 }, icon(CLOSE));
    close.setAttribute("role", "button");
    close.setAttribute("aria-label", "Close tab");
    const activate = (event: Event) => { event.stopPropagation(); this.close(id); };
    close.addEventListener("click", activate);
    close.addEventListener("keydown", (event) => {
      if (event.key === "Enter" || event.key === " ") { event.preventDefault(); activate(event); }
    });
    const node = el("button", { className: "row" }, dot, el("span", { className: "row-text" }, title, detail), close);
    node.dataset.tabId = String(id);
    node.setAttribute("role", "listitem");
    node.addEventListener("click", () => this.select(id));
    return { node, title, detail, dot };
  }

  private update(row: TabRow, tab: TabView) {
    const previous = row.data;
    if (previous === tab) return;
    setText(row.title, tab.title);
    if (!previous || previous.url !== tab.url || previous.crashed !== tab.crashed || previous.title !== tab.title) {
      let detail = "";
      if (tab.crashed) detail = "Stopped working · Reload";
      else if (tab.url !== "about:blank") {
        try { detail = new URL(tab.url).host; } catch { detail = tab.url; }
      }
      setText(row.detail, detail);
      if (row.detail.hidden !== !detail) row.detail.hidden = !detail;
      const title = detail ? `${tab.title}\n${detail}` : tab.title;
      if (row.node.title !== title) row.node.title = title;
    }
    if (!previous || previous.crashed !== tab.crashed) row.node.classList.toggle("crashed", tab.crashed);
    const hidden = !tab.loading || tab.crashed;
    if (row.dot.hidden !== hidden) row.dot.hidden = hidden;
    row.data = tab;
  }

  /** null = structural update; otherwise visit only changed IDs and selection. */
  render(tabs: TabView[], lookup: (id: number) => TabView | undefined, active: number, changes: ReadonlySet<number> | null) {
    if (changes === null) {
      for (const [id, row] of this.rows) {
        if (!lookup(id)) { row.node.remove(); this.rows.delete(id); }
      }
      let cursor = this.list.firstChild;
      for (const tab of tabs) {
        let row = this.rows.get(tab.id);
        if (!row) { row = this.create(tab.id); this.rows.set(tab.id, row); }
        this.update(row, tab);
        if (cursor === row.node) cursor = cursor.nextSibling;
        else this.list.insertBefore(row.node, cursor);
      }
      if (cursor !== this.add) this.list.insertBefore(this.add, cursor);
    } else {
      for (const id of changes) {
        const tab = lookup(id), row = this.rows.get(id);
        if (tab && row) this.update(row, tab);
      }
    }
    if (this.selected !== active) {
      if (this.selected !== undefined) this.rows.get(this.selected)?.node.removeAttribute("aria-current");
      const selected = this.rows.get(active);
      if (selected) setAttribute(selected.node, "aria-current", "true");
      this.selected = active;
    }
  }
}
