// Escape for anything modal: dialogs, sheets, the ⌘K palette, context menus,
// the branch combobox's popover. One CAPTURE-phase listener on `window` and a
// stack of the surfaces currently up; the key goes to the TOP entry only.
//
// Why capture, and why a stack, instead of the `window.addEventListener(
// "keydown", …)` in each component this replaces:
//
//  - xterm calls `stopPropagation()` on every keydown it handles (`cancel(e,
//    true)` in its keydown path), so a bubble-phase listener on `window` never
//    hears a key pressed while the terminal has focus — and in WKWebView a
//    click on a button does NOT move focus, so a dialog opened from a menu
//    while the terminal held the keyboard was un-dismissable: Escape went to
//    tmux (which showed the ESC) and the dialog stayed. Only dialogs that
//    focus an input of their own on mount ever worked. A capture listener on
//    `window` runs before the target sees the event; stopping it there keeps
//    the ESC out of the pty too.
//  - Stacked surfaces (What's new over Settings, the combobox popover inside
//    the New-worktree dialog) each want the key. The stack is ordered by
//    ACTIVATION, so the last surface to appear is the one that closes, and no
//    listener has to know what is on top of it (the `.modal-scrim.stacked` DOM
//    test and the combobox's `stopPropagation` did that job before).
//
// `fn` is read through a ref, so a caller may pass a fresh closure every
// render without re-registering — re-registering would move it to the top of
// the stack, which is exactly the wrong thing when a surface UNDER the top
// one re-renders. A surface that must swallow Escape without acting (a dialog
// mid-apply) keeps its entry and makes `fn` a no-op; dropping the entry would
// hand the key to whatever is beneath.
import { useEffect, useRef } from "react";

type Entry = { fn: () => void };

const stack: Entry[] = [];

function onKeyCapture(e: KeyboardEvent) {
  if (e.key !== "Escape" || e.repeat || e.isComposing) return;
  const top = stack[stack.length - 1];
  if (!top) return;
  e.preventDefault();
  e.stopPropagation();
  top.fn();
}

/** Run `fn` on Escape while `active` — and while active, own the key: nothing
 *  underneath (a terminal, a lower dialog, the app's own chord handler) sees
 *  it. Call it unconditionally, like any hook; gate with `active`. */
export function useEscape(fn: () => void, active = true) {
  const ref = useRef(fn);
  ref.current = fn;
  useEffect(() => {
    if (!active) return;
    const entry: Entry = { fn: () => ref.current() };
    stack.push(entry);
    if (stack.length === 1) window.addEventListener("keydown", onKeyCapture, true);
    return () => {
      const i = stack.indexOf(entry);
      if (i >= 0) stack.splice(i, 1);
      if (stack.length === 0) window.removeEventListener("keydown", onKeyCapture, true);
    };
  }, [active]);
}

/** Test seam: how many surfaces currently own Escape. */
export const escapeDepth = () => stack.length;
