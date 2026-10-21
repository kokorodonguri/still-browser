// The picture shown behind new tabs. Rust keeps a copy in the data folder and
// names it; the interface fetches its bytes once per name and shows them from
// a blob: URL, shared by the new tab background and the settings preview.
import { backend } from "./backend";

/** Rust refuses anything larger; a bigger file is cut just past this so the refusal is cheap. */
const MAX_BYTES = 20 * 1024 * 1024;
const TYPES: Record<string, string> = {
  png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", gif: "image/gif", webp: "image/webp", avif: "image/avif", bmp: "image/bmp",
};

let name = "";
let url = "";
let sequence = 0;
const listeners = new Set<() => void>();

/** The blob: URL of the current picture, or "" when none is shown. */
export const pictureUrl = () => url;

export function onPictureChange(listener: () => void) {
  listeners.add(listener);
}

function show(next: string) {
  if (url) URL.revokeObjectURL(url);
  url = next;
  listeners.forEach((listener) => listener());
}

/** Called with every settings update; loads the bytes only when the name changes. */
export function syncPicture(next: string) {
  if (next === name) return;
  name = next;
  const request = ++sequence;
  if (!next) {
    show("");
    return;
  }
  backend.newTabImage().then((bytes) => {
    if (request !== sequence) return;
    const type = TYPES[next.slice(next.lastIndexOf(".") + 1)] ?? "";
    show(bytes.byteLength ? URL.createObjectURL(new Blob([bytes], { type })) : "");
  }).catch((error) => {
    console.error(error);
    if (request === sequence) show("");
  });
}

const input = Object.assign(document.createElement("input"), { type: "file", accept: Object.values(TYPES).join(","), hidden: true });
input.addEventListener("change", async () => {
  const file = input.files?.[0];
  input.value = "";
  if (!file) return;
  try {
    await backend.setNewTabImage(await file.slice(0, MAX_BYTES + 1).arrayBuffer());
  } catch (error) {
    console.error(error);
  }
});
document.body.append(input);

/** Opens the file picker; the choice is copied and checked by Rust. */
export function choosePicture() {
  input.click();
}

export function removePicture() {
  backend.setNewTabImage(null).catch(console.error);
}
