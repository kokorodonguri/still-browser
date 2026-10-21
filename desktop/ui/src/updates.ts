// Software updates: a quiet check at startup, a toolbar button when the
// download site has a newer version, and Settings > General > Software update.
import { backend } from "./backend";

export type UpdatePhase = "idle" | "checking" | "current" | "available" | "installing" | "check-failed" | "install-failed";
export interface UpdateState { phase: UpdatePhase; current: string; available: string | null }

let state: UpdateState = { phase: "idle", current: "", available: null };
const listeners = new Set<() => void>();

export const updateState = () => state;
export const onUpdateChange = (listener: () => void) => void listeners.add(listener);

function set(next: Partial<UpdateState>) {
  state = { ...state, ...next };
  listeners.forEach((listener) => listener());
}

/** A failed startup check stays quiet; a manual one says so. */
export async function checkForUpdate(manual: boolean) {
  if (state.phase === "checking" || state.phase === "installing") return;
  const before = state.phase;
  set({ phase: "checking" });
  try {
    const status = await backend.checkUpdate();
    const failed = status.failed ? (manual ? "check-failed" : before) : null;
    set({ current: status.current, available: status.available, phase: failed ?? (status.available ? "available" : "current") });
  } catch {
    set({ phase: manual ? "check-failed" : before });
  }
}

export async function installUpdate() {
  if (state.phase !== "available" && state.phase !== "install-failed") return;
  set({ phase: "installing" });
  try {
    await backend.installUpdate();
  } catch {
    set({ phase: "install-failed" });
  }
}
