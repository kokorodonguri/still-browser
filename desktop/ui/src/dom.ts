// Small DOM helpers shared by the toolbar, sidebar and settings.

export function el<K extends keyof HTMLElementTagNameMap>(tag: K, props: Partial<HTMLElementTagNameMap[K]> = {}, ...children: (Node | string)[]) {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children);
  return node;
}

export function icon(path: string) {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 20 20");
  svg.innerHTML = path;
  return svg;
}

/** Avoid invalidating layout/style or replacing text nodes for unchanged data. */
export function setText(node: HTMLElement, text: string) {
  if (node.textContent !== text) node.textContent = text;
}

export function setAttribute(node: HTMLElement, name: string, value: string) {
  if (node.getAttribute(name) !== value) node.setAttribute(name, value);
}
