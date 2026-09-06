// Local usage metrics — what gets clicked, and where the hours go.
//
// The point is to find the controls nobody uses, so the UI can shed weight.
// Everything stays on this Mac (`ui-events.jsonl` in the app config dir);
// nothing is sent anywhere, ever.
//
// ⚠ THE RULE: an event is a control KEY, a SURFACE and a timestamp. It is never
// a place, a slug, a path, a branch, a title, a note or a filter — nothing a
// person typed and nothing that names their work. Every key in the file is a
// constant that lives in this repo's source. The three belts, outermost first:
//
//   1. `data-track="…"` attributes on the controls whose label carries user
//      text (every place row, every project header, the file tree). Those are
//      literals in the TSX.
//   2. `titleKey()` below, for everything else — it reduces a FIXED English
//      title to a key, and refuses anything that looks like it was typed.
//   3. `valid_token` in lib.rs, which refuses to store a key that is not a
//      hand-written identifier.
//
// Nothing here may ever throw into the UI or raise a toast: a metric that
// changes the behaviour it measures is worse than no metric.

import { invoke } from "@tauri-apps/api/core";

/** Where the user was when it happened. A FIXED enum — never a place name. */
export type Surface =
  | "main"          // the place's terminal
  | "dock.files"
  | "dock.terminal"
  | "read"          // reading mode (a rendered file over the whole pane)
  | "home"          // the briefing
  | "nav"           // sidebar / rails
  | "settings"
  | "status"        // the status check + sheets
  | "project"       // the Project sheet
  | "other";

type Kind = "act" | "dwell";
type Event = { t: number; k: Kind; key: string; s: Surface; ms?: number };

/** Flush cadence. 15s is short enough that a crash loses almost nothing and
 *  long enough that a busy minute is one invoke, not sixty. */
const FLUSH_MS = 15_000;
/** …or sooner, if the buffer fills. */
const FLUSH_AT = 50;
/** Dwell resolution. One second is finer than any question this answers, and
 *  it is what makes "attribute nothing while hidden" exact rather than a guess. */
const DWELL_MS = 1000;
/** A key longer than this is not a name anyone wrote deliberately. */
const KEY_MAX = 40;

// ── the key of a control ────────────────────────────────────────────────────

/** Reduce a control's `title` to a stable key, or `null` for "do not record".
 *
 *  Titles in this app are `"<what it is> — <why / how>"`, `"Layout: auto — …"`,
 *  `"Places — pinned; click to unpin (⌘B)"`. The part before the first
 *  separator is the control; everything after it is state, a hint or a chord,
 *  and folding those in would make one button several rows in the heatmap.
 *
 *  The refusals are the interesting half. A title that interpolates a place
 *  name, a slug or a path must never become a key — such a control gets an
 *  explicit `data-track` instead (see the audit in the PR), and this is the
 *  belt for the one that gets added next year and forgotten:
 *
 *   - non-ASCII, `/`, `\`, `~` — a path, a branch, or a name someone typed;
 *   - a hyphenated token with no spaces around it (`standup-and-daily-work`) —
 *     that is a slug's shape, and no fixed title in this app has one;
 *   - anything over 40 characters, which no button label is.
 *
 *  It cannot tell a one-word slug from a one-word label, and it is not trying
 *  to: belt 1 is what actually covers those.
 */
export function titleKey(title: string | null | undefined): string | null {
  if (!title) return null;
  // the control, not its state — cut at the first of the app's separators
  let head = title;
  for (const sep of [" — ", ";", "(", ":", "·", "\n"]) {
    const i = head.indexOf(sep);
    if (i > 0) head = head.slice(0, i);
  }
  head = head.trim();
  if (!head) return null;
  // a path, a branch or a name with an accent in it — not a label we wrote
  if (/[^\x20-\x7e]|[/\\~]/.test(head)) return null;
  // slug shape: hyphens or underscores, and no spaces to say it is a sentence
  if (/[-_]/.test(head) && !/\s/.test(head)) return null;
  const key = head.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
  if (!key || key.length > KEY_MAX) return null;
  return key;
}

/** The key for one click, or `null`. Order matters: an explicit `data-track`
 *  beats a `data-testid` (some of which interpolate a project root, and all of
 *  which exist for the tests, not for this), which beats the title. */
export function keyForTarget(el: Element | null): string | null {
  if (!el) return null;
  const tracked = el.closest<HTMLElement>("[data-track]");
  if (tracked?.dataset.track) return tracked.dataset.track;
  const tested = el.closest<HTMLElement>("[data-testid]");
  const testid = tested?.getAttribute("data-testid");
  // A `data-testid` is only a key if it is a CONSTANT. Two of them carry the
  // project root (`sync-mini|/Users/…`), and those controls have their own
  // `data-track`; this refuses the shape outright so the next one is covered.
  if (testid && /^[a-z0-9.\-_]+$/i.test(testid)) return testid;
  // …otherwise a title, but only on something that is actually a control.
  const btn = el.closest<HTMLElement>("button, [role=button]");
  return btn ? titleKey(btn.getAttribute("title")) : null;
}

/** `cmd-b`, `cmd-shift-t`, `cmd-1`, `cmd-equal` — or `null` when this is not an
 *  app chord.
 *
 *  Built from `e.code` ALONE, and that is not a style choice. `e.key` is the
 *  composed character: on macOS ⌥- arrives as "–" and ⌥= as "≠" (the reason
 *  `zoomDir` falls back to `e.code`), and on a plain keypress it is literally
 *  the character typed — which is text, and text is the one thing that may not
 *  reach this file. `e.code` is the physical key, drawn from a fixed vocabulary.
 *
 *  A modifier is required for the same reason: without one, every keystroke in
 *  a focused terminal would arrive here, and xterm calls `preventDefault` on
 *  most of them.
 */
export function comboName(e: KeyboardEvent): string | null {
  if (!e.metaKey && !e.ctrlKey) return null;
  const code = e.code || "";
  let base: string | null = null;
  if (/^Key[A-Z]$/.test(code)) base = code.slice(3).toLowerCase();
  else if (/^Digit[0-9]$/.test(code)) base = code.slice(5);
  else if (/^[A-Za-z]+$/.test(code)) base = code.toLowerCase(); // Equal, Minus, Comma, Slash, Enter…
  if (!base || base === "escape") return null;
  const mods = [e.metaKey && "cmd", e.ctrlKey && "ctrl", e.altKey && "alt", e.shiftKey && "shift"].filter(Boolean);
  return [...mods, base].join("-");
}

// ── the buffer ──────────────────────────────────────────────────────────────

let buf: Event[] = [];
let surface: Surface = "main";
/** ms accrued on each surface since the last flush; emitted as one line each. */
let dwell: Partial<Record<Surface, number>> = {};
let warned = false;
let timers: number[] = [];

/** One warning per session, into app.log, then silence — see the header rule. */
function quiet(e: unknown) {
  if (warned) return;
  warned = true;
  invoke("log_event", { level: "warn", msg: `usage metrics off: ${String(e)}` }).catch(() => {});
}

export function setSurface(s: Surface) {
  surface = s;
}
export function currentSurface(): Surface {
  return surface;
}

/** Record one act. `key` MUST be a literal from this repo's source. */
export function track(key: string, s: Surface = surface) {
  if (!key) return;
  buf.push({ t: Date.now(), k: "act", key, s });
  if (buf.length >= FLUSH_AT) void flushUsage();
}

export function flushUsage(): Promise<void> {
  const events: Event[] = buf;
  buf = [];
  for (const [s, ms] of Object.entries(dwell)) {
    // Whole seconds only, and `key` is a constant: the surface it belongs to
    // travels in `s`, where the aggregation reads it.
    if (ms && ms >= DWELL_MS) events.push({ t: Date.now(), k: "dwell", key: "dwell", s: s as Surface, ms });
  }
  dwell = {};
  if (!events.length) return Promise.resolve();
  return invoke("ui_events_append", { events }).then(() => {}, quiet);
}

/** Which surface a pointerdown landed on. Same `closest` spirit as App's
 *  `lastSurface`, widened to the enum this file records. Ordered
 *  most-specific-first: the dock and the reader sit INSIDE `.main`'s box in
 *  some layouts, so a `.main` test has to come after them. */
export function surfaceOf(t: Element): Surface | null {
  if (t.closest(".settings-modal")) return "settings";
  if (t.closest(".project-sheet")) return "project";
  // both hosts of the health check: the slide-over, and the panel a
  // session-less worktree shows in the main pane
  if (t.closest(".status-sheet, .term-status")) return "status";
  if (t.closest(".reading")) return "read";
  if (t.closest(".dock")) return t.closest(".termtabs, .term-host") ? "dock.terminal" : "dock.files";
  if (t.closest(".rail, .nav")) return "nav";
  // `.briefing` and `.term-status` live INSIDE `.main`, so `.main` is the
  // fallthrough — the terminal and its chrome, and nothing more specific.
  if (t.closest(".briefing")) return "home";
  if (t.closest(".main")) return "main";
  return null;
}

/** The chord that just fired, if it did anything.
 *
 *  A chord that DID something called `preventDefault` on its way out, and
 *  App's `onKey` branches all `return` — so there is no end-of-function line to
 *  hang this on. It is registered by App's OWN effect, immediately after
 *  `onKey`: two bubble-phase listeners on the same target fire in registration
 *  order, which is the only thing that guarantees `defaultPrevented` is already
 *  set when we read it. Installed from here it would race the effect order.
 *
 *  Escape is not a chord, and `comboName` refuses an unmodified key.
 *
 *  ⚠ But "unmodified" is not enough on its own, because the embedded terminal
 *  is full of MODIFIED keys that are not the app's. xterm calls
 *  `preventDefault` on Ctrl+C, Ctrl+R, Ctrl+D and whatever the tmux prefix is,
 *  and hands them to the pty — so every one of them arrives here already
 *  `defaultPrevented` and reads exactly like an app chord that did something.
 *  Recording those does not leak anything (a chord name is a physical key, not
 *  a character), but it turns the heatmap into a typing meter: `chord.ctrl-c`
 *  outranks every real control in the app within a day of normal use, and the
 *  one question this feature exists to answer — which of OUR controls are dead
 *  — is drowned by it.
 *
 *  So a ctrl-only chord landing inside `.term-host` belongs to the terminal and
 *  is dropped. ⌘ chords still count everywhere: the webview keeps those for
 *  itself and never gives them to the pty, so a ⌘ chord is the app's by
 *  construction — including one pressed with the terminal focused, which is
 *  where most of them are pressed. */
export function trackChord(e: KeyboardEvent) {
  if (!e.defaultPrevented) return;
  // ctrl-only, inside the terminal → the pty's keystroke, not our control
  if (!e.metaKey && e.target instanceof Element && e.target.closest(".term-host")) return;
  const name = comboName(e);
  if (name) track("chord." + name);
}

/** Install the listeners. Returns a teardown (App's effect uses it). */
export function installUsage(): () => void {
  // Capture phase: a handler that calls `stopPropagation` (the combobox, the
  // context menus) must not make its own control invisible.
  const onClick = (e: MouseEvent) => {
    const t = e.target;
    if (!(t instanceof Element)) return;
    const key = keyForTarget(t);
    if (key) track(key);
  };
  const onDown = (e: PointerEvent) => {
    const t = e.target;
    if (t instanceof Element) {
      const s = surfaceOf(t);
      if (s) surface = s;
    }
  };
  // Never attribute time to a window nobody is looking at. `hasFocus` is the
  // second half: `visibilitychange` does not fire on plain focus loss (a
  // visible-but-unfocused window is still being read — but it is not being
  // USED, which is the thing this measures).
  const tick = () => {
    if (document.visibilityState !== "visible" || !document.hasFocus()) return;
    dwell[surface] = (dwell[surface] ?? 0) + DWELL_MS;
  };
  const onVis = () => {
    if (document.visibilityState === "hidden") void flushUsage();
  };
  const onBye = () => {
    void flushUsage();
  };

  document.addEventListener("click", onClick, true);
  document.addEventListener("pointerdown", onDown, true);
  document.addEventListener("visibilitychange", onVis);
  window.addEventListener("beforeunload", onBye);
  timers = [window.setInterval(tick, DWELL_MS), window.setInterval(() => void flushUsage(), FLUSH_MS)];

  return () => {
    document.removeEventListener("click", onClick, true);
    document.removeEventListener("pointerdown", onDown, true);
    document.removeEventListener("visibilitychange", onVis);
    window.removeEventListener("beforeunload", onBye);
    for (const id of timers) clearInterval(id);
    timers = [];
    void flushUsage();
  };
}

// Harness only (VITE_MOCK=1): the mock answers in a microtask, so a Playwright
// run has no way to wait for the 15s flush. Never present in a real build.
if (import.meta.env.VITE_MOCK) {
  (window as unknown as Record<string, unknown>).__usage = { track, flushUsage, currentSurface, titleKey, comboName };
}
