// The document's reading size, and the measure it is read at.
//
// WHY THE VIEWER OWNS A ZOOM AT ALL, rather than leaving it to the browser.
// Page zoom is the browser's, and on this page it is both unreliable and the
// wrong knob:
//
//   - It is not ours to bind. `⌘+` is a menu item in Safari, and what a page
//     gets is whatever the browser decides to leave it. The one reported
//     symptom is Safari zooming and then snapping back, which no line of this
//     bundle can cause and no line of it can fix.
//   - It would not survive anyway. The docs server binds an EPHEMERAL port
//     (`docserver.rs`: `SocketAddr::from((LOCALHOST, 0))`), so every app launch
//     is a new origin — and per-site page zoom, like `localStorage` below, is
//     keyed by origin. Nothing stored against `127.0.0.1:62347` is there at
//     `:51002` tomorrow.
//   - It scales the WHOLE page, which on this page is mostly not the document:
//     a 264px nav column and a sticky staleness header that has nothing to gain
//     from being larger. Measured on a 1672px window, the prose column was
//     645px of a 1393px page — page zoom grows the gutters along with the text.
//
// `--md-zoom` is the knob App.css already built for exactly this (`.md`, "ONE
// knob scales the rendered document"): unitless, 1 = normal, and every size in
// the block — headings in `em`, tables in `--fs-*`, fences off `--term-size`,
// the `--md-s*` spacing scale AND the 78ch measure, which tracks font-size — is
// expressed against it. So the viewer's zoom is one number written onto `.doc`,
// the nav and the header stay the size they should be, and the column WIDENS as
// it grows, which is the half of this that reclaims the gutter.
//
// PERSISTENCE IS DELIBERATELY SHALLOW. `localStorage` is keyed by origin
// including the port, so this setting lives exactly as long as the docs server
// does: it survives reloads, navigation and the back button, and it is gone
// when the app restarts on a fresh port. That is a known, accepted cost of not
// putting a write route on a read-only server — say so here rather than let the
// next reader discover it as a bug.
import { useCallback, useEffect, useState } from "react";

/**
 * Discrete stops, not a free number — the same reasoning as `MD_ZOOM_STEPS` in
 * `app/src/settings.ts`: a reading size is chosen by pressing a key until it
 * looks right, and 1%-at-a-time makes that a dozen presses. The steps widen as
 * they climb because 175 → 180 is invisible where 90 → 100 is not.
 *
 * It is NOT imported from `settings.ts`, and that is not an oversight. That
 * module's first line is `import { invoke } from "@tauri-apps/api/core"` — a
 * browser bundle that has no Tauri under it must not pull the IPC client in to
 * borrow an array. This is the kind of duplication the repo tolerates: a list
 * of stops is a PREFERENCE, not a decision, so drift shows up as a button that
 * steps differently, never as a wrong answer. (Contrast `dnd.ts::predictTier`,
 * which mirrors a rule in `store.rs` and therefore needs `dnd-check.mjs`.)
 *
 * It runs two stops past the dock's 200% on purpose: the dock's viewer lives in
 * a pane a few hundred pixels wide, this one has a whole browser window.
 */
export const ZOOM_STEPS = [70, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300] as const;
export const ZOOM_MIN = ZOOM_STEPS[0];
export const ZOOM_MAX = ZOOM_STEPS[ZOOM_STEPS.length - 1];
export const ZOOM_DEFAULT = 100;

/** What the viewer remembers: a size, and whether the measure is off. */
export type DocView = { zoom: number; wide: boolean };

/**
 * Snap to the nearest stop.
 *
 * Also the guard against a garbage persisted value: this reads a string that
 * anything with the origin's `localStorage` may have written, so `NaN`,
 * `Infinity` and `1e9` all have to land on something the buttons can step off
 * again. A zoom nobody can zoom back out of is the failure mode.
 */
export function clampZoom(v: unknown): number {
  const n = typeof v === "number" ? v : Number(v);
  if (!Number.isFinite(n)) return ZOOM_DEFAULT;
  const bound = Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, n));
  return ZOOM_STEPS.reduce(
    (best, s) => (Math.abs(s - bound) < Math.abs(best - bound) ? s : best),
    ZOOM_STEPS[0] as number,
  );
}

/** Next stop up (`1`) or down (`-1`); the same value at either end. */
export function stepZoom(v: number, dir: 1 | -1): number {
  const cur = clampZoom(v);
  const i = ZOOM_STEPS.indexOf(cur as (typeof ZOOM_STEPS)[number]);
  return ZOOM_STEPS[Math.max(0, Math.min(ZOOM_STEPS.length - 1, i + dir))];
}

// The size chords, keyed on BOTH faces of each key.
//
// `e.key` covers the characters: ⌘+ is ⌘⇧= on a US layout ("+"), ⌘− is "-" but
// "_" when shifted, and the numeric keypad sends "+"/"-" unshifted. `e.code` is
// the fallback and it is what the ⌥ variants NEED — macOS composes Option with
// the layout, so ⌥- arrives as "–" (an en dash) and ⌥= as "≠", and a handler
// keyed on the character alone is silently dead on every US Mac (CLAUDE.md,
// "An ⌥ chord cannot be matched on `e.key`"). ⌥ is accepted rather than
// required or refused because the dock's markdown viewer binds ⌘⌥±/⌘⌥0 for the
// same job; there is only one thing to zoom on this page, so both chords should
// reach it.
const BY_KEY: Record<string, 1 | -1 | 0> = { "+": 1, "=": 1, "-": -1, "_": -1, "0": 0 };
const BY_CODE: Record<string, 1 | -1 | 0> = {
  Equal: 1, NumpadAdd: 1, Minus: -1, NumpadSubtract: -1, Digit0: 0, Numpad0: 0,
};

/**
 * `1` bigger, `-1` smaller, `0` reset, `undefined` when this is not a size
 * chord at all.
 *
 * `??`, never `||`: 0 is a legal direction (reset) and would fall straight
 * through `||` to the code table, where `Digit0` happens to answer 0 as well —
 * so the bug would be invisible here and would bite the first time the two
 * tables disagreed.
 */
export function zoomDir(e: KeyboardEvent): 1 | -1 | 0 | undefined {
  if (!e.metaKey && !e.ctrlKey) return undefined;
  return BY_KEY[e.key] ?? BY_CODE[e.code];
}

/**
 * One key, one JSON blob, both halves together — the same reason `Viewer.tsx`
 * keeps an ETag with the payload it describes: two keys can disagree, one
 * cannot.
 *
 * EVERY ACCESS IS WRAPPED. `localStorage` is not a plain object: the getter
 * itself throws in a Safari private window and wherever site data is blocked,
 * and a throw here is a throw inside a `useState` initialiser — a blank page,
 * not a lost preference. The document must render at 100% and keep working
 * when there is nowhere to remember anything.
 */
const KEY = "worktrees.docs.view";

export function readView(): DocView {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return { zoom: ZOOM_DEFAULT, wide: false };
    const v = JSON.parse(raw) as Partial<DocView> | null;
    return { zoom: clampZoom(v?.zoom), wide: v?.wide === true };
  } catch {
    return { zoom: ZOOM_DEFAULT, wide: false };
  }
}

export function writeView(v: DocView): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(v));
  } catch {
    /* no store, or a full one: the setting is simply not remembered */
  }
}

export type DocViewApi = {
  view: DocView;
  /** A direction, as `zoomDir` reports it: `1`, `-1`, or `0` for reset. */
  nudge: (dir: 1 | -1 | 0) => void;
  setWide: (wide: boolean) => void;
};

/**
 * The reading size and its chords.
 *
 * `enabled` is the DOCUMENT route. The handler calls `preventDefault`, which is
 * what stops the browser zooming the whole page underneath our own — so on the
 * index screen, where there is no `.md` block for `--md-zoom` to reach, it must
 * not be installed at all: swallowing ⌘+ to do nothing is worse than not
 * binding it.
 *
 * CAPTURE phase, like the `/` handler in `Viewer.tsx`, so the chord works while
 * the caret is in the nav's filter box.
 */
export function useDocView(enabled: boolean): DocViewApi {
  const [view, setView] = useState<DocView>(readView);

  // Persisted from an effect rather than from inside the updater: StrictMode
  // double-invokes a reducer, and a `setItem` in there is a side effect in a
  // function React is allowed to call twice and throw one result away.
  useEffect(() => { writeView(view); }, [view]);

  const nudge = useCallback((dir: 1 | -1 | 0) => {
    setView((v) => {
      const zoom = dir === 0 ? ZOOM_DEFAULT : stepZoom(v.zoom, dir);
      return zoom === v.zoom ? v : { ...v, zoom };
    });
  }, []);

  const setWide = useCallback((wide: boolean) => {
    setView((v) => (v.wide === wide ? v : { ...v, wide }));
  }, []);

  useEffect(() => {
    if (!enabled) return;
    const onKey = (e: KeyboardEvent) => {
      const dir = zoomDir(e);
      if (dir === undefined) return;
      e.preventDefault();
      nudge(dir);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [enabled, nudge]);

  return { view, nudge, setWide };
}
