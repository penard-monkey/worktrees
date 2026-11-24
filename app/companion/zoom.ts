// The Ideas window's own size: ⌘+ / ⌘− / ⌘0 (and View → Zoom In / Out / Actual
// Size) zoom the window that has focus, on the same steps as the main window's.
// The factor is this window's, kept by the shim beside its frame
// (window-state.json) and not in ui-state.json, which the main window writes
// whole. Until the person zooms this window it takes the app's size
// (`app_zoom`), and taking it keeps nothing, so it goes on inheriting.
import { invoke } from "../src/ipc";
import { applyZoom, clampZoom, DEFAULTS, stepZoom } from "../src/settings";
import { zoomDir } from "../src/zoomkeys";

type Key = Pick<KeyboardEvent, "key" | "code" | "metaKey" | "ctrlKey" | "altKey" | "preventDefault">;

/** This window's zoom. `own`: the person set it, so the app's no longer applies. */
export class WindowZoom {
  private cur = DEFAULTS.app_zoom;
  private own = false;

  /** The app's size, from the shared settings. */
  inherit(appZoom: number): void {
    if (this.own) return;
    this.cur = clampZoom(appZoom);
    applyZoom(this.cur);
  }

  /** What the shim kept for this window, null for nothing. */
  restore(kept: number | null): void {
    if (this.own || typeof kept !== "number") return;
    this.own = true;
    this.cur = clampZoom(kept);
    applyZoom(this.cur);
  }

  /** A keydown: true when it was a size chord and has been dealt with. */
  key(e: Key): boolean {
    if (!e.metaKey || e.ctrlKey || e.altKey) return false;
    const dir = zoomDir(e);
    if (dir === undefined) return false;
    e.preventDefault();
    const next = dir === 0 ? DEFAULTS.app_zoom : stepZoom(this.cur, dir);
    if (next !== this.cur || !this.own) {
      this.own = true;
      this.cur = next;
      applyZoom(next, true);
    }
    return true;
  }

  /** Read what the shim kept. Before the settings can answer, so a kept size
   *  is never flashed over by the app's. */
  async load(): Promise<void> {
    try { this.restore(await invoke<number | null>("window_zoom")); } catch { /* no shim (the harness): the app's size */ }
  }
}
