// The page's half of the macOS menu bar (src-tauri/src/winmenu.rs has the
// items). Choosing an item never runs a second copy of an action: it replays
// the item's CHORD as a keydown, so the handler that owns the chord does it
// with every guard it has (a dialog is up, nothing is selected), or for an item
// with no chord it calls the window's own function for that id.
//
// One press is one act. macOS gives a ⌘ key to the page first and sends it on
// to the menu only if the page did not `preventDefault` it. A press that
// reaches the menu that way is one the page already saw and declined (the
// repeats of a held ⌘B, a chord behind a dialog), so it is dropped here.
import { listenMenu, type MenuChosen } from "./ipc";

/** How long after the page's own keydown a menu message is that same press.
 *  The round trip is milliseconds. A person cannot press a chord and then open
 *  a menu and click its item inside this. */
export const SAME_PRESS_MS = 500;

export type Seen = { key: string; code: string; alt: boolean; at: number };

/** Is `m` the press the page last saw? Shift is not compared: the menu's "+"
 *  is pressed both with it and without. */
export function samePress(m: MenuChosen, seen: Seen | null, now: number): boolean {
  if (!m.chord || !seen || now - seen.at > SAME_PRESS_MS || seen.alt !== m.chord.alt) return false;
  return seen.code === m.chord.code || seen.key.toLowerCase() === m.chord.key;
}

/** Wire this window's page to the menu bar. `act` runs an item that has no
 *  chord. Returns the teardown. */
export function wireMenu(label: string, act: (id: string) => void): () => void {
  let seen: Seen | null = null;
  // Capture, so a handler that stops the key (an editor's, the find bar's)
  // does not hide it from here. Trusted only: the replay below is not a press.
  const onKey = (e: KeyboardEvent) => {
    if (e.isTrusted && e.metaKey) seen = { key: e.key, code: e.code, alt: e.altKey, at: performance.now() };
  };
  window.addEventListener("keydown", onKey, true);
  const un = listenMenu(label, (m) => {
    if (samePress(m, seen, performance.now())) return;
    if (!m.chord) { act(m.id); return; }
    const { key, code, shift, alt, focus } = m.chord;
    const at = (focus && document.activeElement) || document.body;
    at.dispatchEvent(new KeyboardEvent("keydown", { key, code, metaKey: true, shiftKey: shift, altKey: alt, bubbles: true, cancelable: true, composed: true }));
  });
  return () => {
    window.removeEventListener("keydown", onKey, true);
    void un.then((f) => f());
  };
}
