import { useCallback, useEffect, useRef, useState } from "react";
import type { Harness } from "./harness";
import { Terminal, type ILink } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { UnicodeGraphemesAddon } from "@xterm/addon-unicode-graphemes";
import { Channel, invoke } from "@tauri-apps/api/core";
import { FindBar, findColors } from "./Find";
import { findPaths, hitRange, logicalLine } from "./termlinks";
import { CtxMenu } from "./CtxMenu";
import { copyToClipboard } from "./clipboard";
import { relPath } from "./filekind";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import "@xterm/xterm/css/xterm.css";

// Two kinds of embedded terminal, one renderer.
//
//   TerminalPane — the place's canonical tmux session. Rust ATTACHES; tmux owns
//     the shell, the panes and the scrollback, and unmounting detaches. This is
//     where Claude runs, so it survives quitting the app.
//   ShellPane    — a dock scratch shell. Rust OWNS the PTY (no tmux), so
//     unmounting must DETACH, never kill: a tab flip or ⌘J can't be allowed to
//     take down a running build. The backend replays a ring buffer on re-attach.
//
// Font comes from the independent --term-* CSS vars (Settings), so UI zoom never
// disturbs the grid. Colors come from the active [data-theme]'s --term-*/--ansi-*
// vars (tokens.css) so the terminal repaints with the rest of the app on a theme
// switch.
const ANSI = [
  "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
  "brightBlack", "brightRed", "brightGreen", "brightYellow",
  "brightBlue", "brightMagenta", "brightCyan", "brightWhite",
] as const;

function termOptions() {
  const cs = getComputedStyle(document.documentElement);
  const v = (name: string, fallback: string) => cs.getPropertyValue(name).trim() || fallback;
  const family = v("--term-family", "Menlo, Monaco, monospace");
  const size = parseInt(cs.getPropertyValue("--term-size"), 10) || 13;
  const bg = v("--term-bg", "#0f0f16");
  const theme: Record<string, string> = {
    background: bg,
    foreground: v("--term-fg", "#c0caf5"),
    cursor: v("--term-cursor", "#c0caf5"),
    cursorAccent: bg,
    selectionBackground: v("--term-sel", "rgba(122, 162, 247, 0.3)"),
  };
  ANSI.forEach((name, i) => (theme[name] = v(`--ansi-${i}`, theme.foreground)));
  return { family, size, theme };
}

// ── cursor blink, gated on the window ──────────────────────────────────────
// A blinking cursor is a style recalc + paint twice a second, per mounted
// terminal, forever — and xterm 5.5 has no "stop blinking when idle" (that
// landed upstream in 7.0). xterm's own gate is the `.xterm-focus` class, which
// is useless here: `useTerm` force-focuses the pane on mount and on every
// re-entry, and element focus does NOT drop when the OS window deactivates. So
// the blink ran whenever the app was open, whether or not anyone was there.
//
// Every live terminal registers here and follows the window instead. Listeners
// are module-scope and deliberately never removed — they outlive any single
// pane and cost one function call per window event.
const liveTerms = new Set<Terminal>();
const blinkWanted = () => document.visibilityState !== "hidden" && document.hasFocus();
const applyBlink = () => {
  const on = blinkWanted();
  liveTerms.forEach((t) => { t.options.cursorBlink = on; });
};
window.addEventListener("focus", applyBlink);
window.addEventListener("blur", applyBlink);
document.addEventListener("visibilitychange", applyBlink);

/** How one pane talks to its backend. `close` is the unmount path and means
 * "stop streaming" for BOTH kinds — detach the tmux client, or drop the sink on
 * an owned shell. Neither ends the thing on the other side. */
type Transport = {
  /** Resolves once attached. `replay` is how many bytes of recorded output the
   *  backend pushed down `onBytes` FIRST — a dock shell's live ring on
   *  re-attach, or the scrollback it SAVED on a previous run when the shell is
   *  being spawned fresh — and 0 when nothing was (a brand-new tab; always, for
   *  tmux, which replays nothing of its own). See `useTerm` for why the pane
   *  must know.
   *
   *  `replayCols` is the grid those bytes were laid out for, or null when the
   *  backend could not say. Raw bytes do not re-wrap, so replaying them into a
   *  different width stacks every full line — see the reflow in `useTerm`. */
  open(cols: number, rows: number, onBytes: Channel<ArrayBuffer>): Promise<{ replay: number; replayCols: number | null }>;
  write(data: number[]): void;
  resize(cols: number, rows: number): void;
  close(): void;
  /** Scroll the backend's own history by `lines` (< 0 = up). Only tmux has one
   *  to scroll; a dock shell's history is xterm's, so it leaves this out and
   *  xterm keeps the wheel. See `wheelToTmux`. */
  wheel?(lines: number): Promise<unknown>;
};

const tmuxTransport = (session: string): Transport => {
  let id: number | null = null;
  return {
    async open(cols, rows, onBytes) {
      id = await invoke<number>("term_open", { session, cols, rows, onBytes });
      return { replay: 0, replayCols: null };
    },
    write: (data) => { if (id != null) invoke("term_write", { id, data }); },
    resize: (cols, rows) => { if (id != null) invoke("term_resize", { id, cols, rows }); },
    close: () => { if (id != null) invoke("term_close", { id }); id = null; },
    wheel: (lines) => (id != null ? invoke("term_wheel", { id, lines }) : Promise.resolve()),
  };
};

const shellTransport = (repo: string, slug: string, index: number): Transport => {
  // The attach generation from shell_open. Detach presents it so a STALE
  // detach (StrictMode: unmount №1 resolving after mount №2 attached) is a
  // backend no-op instead of clearing the new attach's stream.
  let gen: number | null = null;
  return {
    async open(cols, rows, onBytes) {
      const at = await invoke<{ gen: number; replay: number; replay_cols: number | null }>(
        "shell_open", { repo, slug, index, cols, rows, onBytes });
      gen = at.gen;
      return { replay: at.replay, replayCols: at.replay_cols };
    },
    write: (data) => { invoke("shell_write", { repo, slug, index, data }); },
    // gated on the attach, like the tmux transport's `id` — before `shell_open`
    // has answered there is no shell to resize, and the reject would be silent
    resize: (cols, rows) => { if (gen != null) invoke("shell_resize", { repo, slug, index, cols, rows }); },
    close: () => { if (gen != null) invoke("shell_detach", { repo, slug, index, gen }); gen = null; },
  };
};

/** Has the pane been laid out yet? The fit addon needs a real box: given a host
 *  with none (`display:none`, or one not yet through a layout pass) it reads
 *  no usable width/height and returns without resizing, leaving the terminal on
 *  xterm's 80×24 default. Attaching there is not cosmetic — the backend sizes
 *  the PTY to whatever the attach asks for, and the ResizeObserver then corrects
 *  it, so the shell is walked real → 80×24 → real. Every one of those steps is a
 *  SIGWINCH, zsh answers each by redrawing the prompt and any half-typed line
 *  into the replay ring, and the ring is then rendered into an 80-column
 *  terminal it was never written for: the redraws stop overwriting each other
 *  and the pasted line stacks up, pane scrolled to the bottom. */
const measured = (host: HTMLElement) => host.clientWidth > 0 && host.clientHeight > 0;

/** How long to wait for that layout before attaching anyway. A shell that never
 *  attaches because its pane stayed hidden is worse than one attached at the
 *  wrong size, so the wait is bounded. In wall-clock, not frames: the retry
 *  rides `requestAnimationFrame` and the frame rate is not ours to predict
 *  (a headless Chromium runs it at ~200Hz, an occluded window barely at all). */
const ATTACH_WAIT_MS = 1000;

/** How long the pane must hold still before the backend is told its new size.
 *
 *  Every DISTINCT grid handed to the pty is a `TIOCSWINSZ`, so a SIGWINCH, and
 *  the shell answers each one by reprinting its prompt. A 240px drag of the
 *  pane walked the terminal through 17 different row counts and left 17 stacked
 *  prompt lines behind — one truncated `~/workspace/…` plus a full-width rule
 *  per step, in BOTH panes at once (screenshots, 2026-08-27). It also spent 120
 *  `term_resize` invokes to do it, 103 of them re-stating a size that had not
 *  changed at all: `fit()` skips `term.resize` when the grid is the same, but
 *  the invoke after it was unconditional.
 *
 *  A window drag is ONE intent, so it should cost one resize. Anything above a
 *  frame's worth of delay collapses a drag; 80ms is still well under where a
 *  discrete change (⌘B, ⌘J, dragging the dock's edge) reads as lag.
 *
 *  `fit()` waits with it on purpose. Refitting every frame while the pty keeps
 *  the old grid means tmux is painting a screen that no longer matches the
 *  canvas — garbled for the whole drag, instead of a strip of host background
 *  at the edge that closes when the drag stops. */
const RESIZE_SETTLE_MS = 80;

/** How many lines of scrollback xterm keeps.
 *
 *  Not a preference — a floor. The backend replays up to `SHELL_RING` (256K) of
 *  recorded output in one write, and xterm's default of 1000 lines silently
 *  drops whatever does not fit: ~3200 lines at 80 columns were being pushed into
 *  a buffer a third that size, so the OLDEST part of every replay — the build
 *  that failed, the error you flipped back to read — was gone before it could be
 *  scrolled to or found with ⌘F. `termresize-check.mjs` compares this against
 *  `SHELL_RING` in lib.rs, so raising the ring without raising this fails a gate
 *  rather than quietly truncating again. */
const TERM_SCROLLBACK = 5000;

/** Does this wheel event belong to tmux rather than to xterm?
 *
 *  xterm is attached to a tmux CLIENT, and the client always draws on the
 *  alternate screen — so xterm itself never has scrollback here. With no mouse
 *  mode on, xterm does what every terminal does for an alternate screen and
 *  turns each wheel notch into ↑/↓ keys. That is right for a program that owns
 *  the whole screen (less, vim) and wrong for one on the pane's MAIN screen —
 *  pi and a plain shell, whose history lives in tmux's scrollback: pi reads ↑
 *  as editor history, zsh as history recall. So every such wheel goes to the
 *  backend, which looks at the pane and scrolls tmux history, or sends the same
 *  arrows xterm would have (`wheel_plan` in lib.rs).
 *
 *  A pane that asked for the mouse (claude) is left alone: tmux passes that
 *  request through to xterm, which then reports the wheel as mouse events and
 *  the program scrolls itself. xterm consults this handler on that path too, so
 *  the mode check is what keeps it working: xterm 5.5 registers TWO wheel
 *  listeners, and while the one that makes arrows returns early when wheel
 *  reporting is on (`Terminal.ts:802`), the reporting one's `sendEvent` asks
 *  this handler first (`case 'wheel'`, `Terminal.ts:642`) — a `false` here
 *  would swallow claude's scroll. A normal buffer (the dock's owned
 *  shells) has scrollback of its own and keeps xterm's default. */
function wheelToTmux(mouseTrackingMode: string, bufferType: string): boolean {
  return mouseTrackingMode === "none" && bufferType === "alternate";
}

/** Whole lines in a wheel event, carrying the fraction to the next one (a
 *  trackpad sends many small pixel deltas). Mirrors xterm's own
 *  `Viewport.getLinesScrolled`, so a notch moves tmux history as far as it
 *  would have moved xterm's. */
function wheelLines(acc: number, deltaY: number, deltaMode: number, cellHeight: number, rows: number) {
  let amount = deltaY;
  if (deltaMode === 0) amount /= cellHeight > 0 ? cellHeight : 1; // DOM_DELTA_PIXEL
  else if (deltaMode === 2) amount *= rows;                        // DOM_DELTA_PAGE
  amount += acc;
  const lines = Math.trunc(amount);
  return { lines, acc: amount - lines };
}

/** At most one wheel invoke in flight; what arrives meanwhile is summed into
 *  the next. A trackpad fling is ~60 events a second and each flush is two tmux
 *  spawns, so an invoke per event would queue for seconds after the fingers
 *  stop — the view still scrolling long after the gesture ended. Up and down
 *  in the same window cancel, as they should. */
function wheelPump(send: (lines: number) => Promise<unknown>) {
  let pending = 0;
  let busy = false;
  // A failure repeats for every flush of the gesture that hit it (a session
  // that went away, a mode tmux cannot scroll), so it is logged once, until a
  // flush succeeds again. A warning: the cost is one wheel that did nothing.
  let warned = false;
  const flush = () => {
    if (busy || pending === 0) return;
    const n = pending;
    pending = 0;
    busy = true;
    send(n)
      .then(() => { warned = false; })
      .catch((e) => {
        if (warned) return;
        warned = true;
        invoke("log_event", { level: "warn", msg: `terminal wheel: ${e}` }).catch(() => {});
      })
      .finally(() => { busy = false; flush(); });
  };
  return (lines: number) => { pending += lines; flush(); };
}

// ── file-path links (⌘-click → the dock's file viewer) ──────────────────────

/** What a pane needs to turn a path into a link: the directory a relative path
 *  is relative to, and what to do with one. A dock shell also names itself, so
 *  the backend can read its LIVE working directory — the user may have `cd`'d
 *  since the tab opened, and `ls` prints names relative to wherever that is. */
export type TermLinks = {
  root: string;
  shell?: { repo: string; slug: string; index: number };
  /** The tmux session a pane is attached to: its active pane's cwd is read
   *  live, for the same reason as a dock shell's. */
  session?: string;
  onOpen: (path: string, line?: number, col?: number) => void;
  /** Right-click on a link: the pane's own menu (`TermSurface` supplies it). */
  onMenu?: (m: LinkMenu) => void;
  onError?: (e: unknown) => void;
};

/** A right-clicked link: where the menu goes and what it is about. */
export type LinkMenu = { x: number; y: number; path: string; line?: number; col?: number };

/** How long an answer about one path is trusted. Short, because files appear
 *  (an agent writes one and prints its name in the same breath) and a shell's
 *  cwd moves; long enough that sweeping the mouse over a screen of output asks
 *  the filesystem once per path rather than once per row crossed. */
const LINK_TTL_MS = 10_000;
const linkCache = new Map<string, { abs: string | null; at: number }>();

/** ⌘ on macOS, Ctrl elsewhere — the gesture iTerm, Terminal and VS Code all
 *  use. NOT a plain click: in a terminal a click focuses, starts a selection,
 *  and in a mouse-mode program (claude) is the program's own input, so a plain
 *  click that also opened a file would hijack all three. */
const isMac = () => /Mac|iPhone|iPad/.test(navigator.platform);
const isOpenGesture = (e: { metaKey: boolean; ctrlKey: boolean; button?: number }) =>
  (e.button ?? 0) === 0 && (isMac() ? e.metaKey : e.ctrlKey);
/** A press that will become a `contextmenu`: the right button, or — macOS
 *  only — Ctrl with the left one. */
const isMenuPress = (e: { button: number; ctrlKey: boolean }) =>
  e.button === 2 || (isMac() && e.button === 0 && e.ctrlKey);

const LINK_HINT = /Mac|iPhone|iPad/.test(navigator.platform)
  ? "⌘-click to open in the file viewer · right-click for more"
  : "Ctrl-click to open in the file viewer · right-click for more";

/** xterm link provider for file paths. `provideLinks` is asked per ROW as the
 *  mouse crosses it; the row's logical line (wrapped rows joined) is scanned
 *  by `findPaths`, and every candidate not already cached is resolved in ONE
 *  `resolve_term_paths` call — a stat per path in the backend, no spawn. Only
 *  the ones it answers with an absolute path become links.
 *
 *  The ⌘-press on a link is stopped at `.xterm-screen`. xterm's own mousedown
 *  listeners live on the PARENT (`.xterm`): one reports the press to a program
 *  that turned the mouse on (claude would see a click at that cell), the other
 *  starts a text selection. The linkifier listens on `.xterm-screen` itself,
 *  and listeners on ONE element all run, in registration order, whatever any
 *  of them does to propagation — so ours, added after it, can stop the press
 *  from reaching `.xterm` while the linkifier still records the press it needs
 *  to activate on release. A plain press is never
 *  touched, so selection and claude's own mouse work exactly as before. */
function termLinkProvider(term: Terminal, linksRef: { current: TermLinks | null }, host: HTMLElement) {
  // The link under the pointer, as `hover`/`leave` report it. Null = none, and
  // then nothing below touches an event.
  let hovered: { path: string; line?: number; col?: number } | null = null;
  const screen = term.element?.querySelector<HTMLElement>(".xterm-screen") ?? null;
  // A press that belongs to the link — ⌘ to open, right / Ctrl for the menu —
  // stops here. A right-press is reported to a mouse-mode program just like a
  // left one, so it needs the same treatment as the ⌘-press.
  const scopeOf = (ctx: TermLinks) =>
    `${ctx.root}\0${ctx.shell ? `${ctx.shell.repo}|${ctx.shell.slug}|${ctx.shell.index}` : ctx.session ?? ""}\0`;
  /** The link under a mouse event, worked out HERE rather than taken from
   *  `hover`. xterm's linkifier re-asks only when the pointer reaches a
   *  DIFFERENT cell than the last one it saw, and it keeps that cell across a
   *  `mouseleave` — so after a menu or a tooltip covers the pane and goes
   *  away, the pointer can be back on the link with no `hover` ever fired
   *  (right-click, Escape, right-click again without moving: WebKit's own menu
   *  came up instead of ours). Synchronous, from the resolve cache only: a
   *  path that was never resolved was never underlined either. */
  const linkAt = (e: MouseEvent): typeof hovered => {
    const ctx = linksRef.current;
    if (!ctx || !screen || term.cols < 1 || term.rows < 1) return null;
    const r = screen.getBoundingClientRect();
    const x = Math.floor((e.clientX - r.left) / (r.width / term.cols));
    const row = Math.floor((e.clientY - r.top) / (r.height / term.rows));
    if (x < 0 || x >= term.cols || row < 0 || row >= term.rows) return null;
    const buf = term.buffer.active;
    const y = buf.viewportY + row;
    const ll = logicalLine((ry) => buf.getLine(ry), y);
    const scope = scopeOf(ctx);
    for (const h of findPaths(ll.text)) {
      const a = ll.cells[h.start], b = ll.cells[h.end - 1];
      if (!a || !b) continue;
      const inside = (y > a.y || (y === a.y && x >= a.x)) && (y < b.y || (y === b.y && x < b.x + b.w));
      const abs = inside ? linkCache.get(scope + h.path)?.abs : null;
      if (abs) return { path: abs, line: h.line, col: h.col };
    }
    return null;
  };
  const onDown = (e: MouseEvent) => {
    if (!(isOpenGesture(e) || isMenuPress(e))) return;
    const at = hovered ?? linkAt(e);
    if (at) {
      // Not hovered means xterm has no current link, so its `activate` will
      // never run for this press: a ⌘-press found only by `linkAt` is opened
      // here, or stopping it would swallow it.
      if (!hovered && isOpenGesture(e)) linksRef.current?.onOpen(at.path, at.line, at.col);
      e.stopPropagation();
      e.preventDefault();
    }
  };
  // The menu itself. Off a link this returns without touching the event, so
  // WebKit's own menu (and whatever xterm does with it) is exactly as before.
  const onMenu = (e: MouseEvent) => {
    const ctx = linksRef.current;
    const at = hovered ?? linkAt(e);
    if (!at || !ctx?.onMenu) return;
    e.preventDefault();
    e.stopPropagation();
    ctx.onMenu({ x: e.clientX, y: e.clientY, ...at });
  };
  screen?.addEventListener("mousedown", onDown);
  screen?.addEventListener("contextmenu", onMenu);

  const resolve = async (ctx: TermLinks, paths: string[]): Promise<Map<string, string | null>> => {
    const scope = scopeOf(ctx);
    const now = performance.now();
    const out = new Map<string, string | null>();
    const ask: string[] = [];
    for (const p of new Set(paths)) {
      const c = linkCache.get(scope + p);
      if (c && now - c.at < LINK_TTL_MS) out.set(p, c.abs);
      else ask.push(p);
    }
    if (ask.length) {
      try {
        const got = await invoke<(string | null)[]>("resolve_term_paths", {
          root: ctx.root, shell: ctx.shell ?? null, session: ctx.session ?? null, paths: ask,
        });
        ask.forEach((p, i) => {
          const abs = got[i] ?? null;
          linkCache.set(scope + p, { abs, at: now });
          out.set(p, abs);
        });
      } catch (e) {
        // No links is the safe failure; say so once per batch in the log
        // rather than painting anything (CLAUDE.md: never swallow errors).
        invoke("log_event", { level: "warn", msg: `terminal links: ${e}` }).catch(() => {});
      }
      // The cache only ever needs what is on screen; drop the stale tail so a
      // long session does not accumulate every path it ever printed.
      if (linkCache.size > 2000) for (const [k, v] of linkCache) if (now - v.at >= LINK_TTL_MS) linkCache.delete(k);
    }
    return out;
  };

  return {
    provideLinks(bufferLineNumber: number, callback: (links: ILink[] | undefined) => void) {
      const ctx = linksRef.current;
      const buf = term.buffer.active;
      const y = bufferLineNumber - 1;
      if (!ctx) return callback(undefined);
      const ll = logicalLine((ry) => buf.getLine(ry), y);
      // Only the candidates that touch the hovered row: xterm asks per row,
      // and the rest of a wrapped line is answered when the mouse gets there.
      const hits = findPaths(ll.text).filter((h) => {
        const a = ll.cells[h.start];
        const b = ll.cells[h.end - 1];
        return a && b && a.y <= y && b.y >= y;
      });
      if (!hits.length) return callback(undefined);
      void resolve(ctx, hits.map((h) => h.path)).then((abs) => {
        const links: ILink[] = [];
        for (const h of hits) {
          const path = abs.get(h.path);
          if (!path) continue;
          links.push({
            range: hitRange(h, ll.cells),
            text: ll.text.slice(h.start, h.end),
            decorations: { pointerCursor: true, underline: true },
            activate: (e: MouseEvent) => {
              if (!isOpenGesture(e)) return;
              linksRef.current?.onOpen(path, h.line, h.col);
            },
            hover: () => { hovered = { path, line: h.line, col: h.col }; host.title = LINK_HINT; },
            leave: () => { hovered = null; host.title = ""; },
          });
        }
        callback(links.length ? links : undefined);
      });
    },
    dispose() {
      screen?.removeEventListener("mousedown", onDown);
      screen?.removeEventListener("contextmenu", onMenu);
      hovered = null;
    },
  };
}

/** The xterm instance + wiring. `key` re-creates everything when it changes. */
function useTerm(makeTransport: () => Transport, key: string, termVersion: number, focusToken: number, focusEnabled: boolean, links?: TermLinks) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  // Read through a ref by the link provider, which lives as long as the xterm:
  // `onOpen` is a new closure every render and the root can change under a
  // mounted pane, and neither is a reason to rebuild the terminal.
  const linksRef = useRef<TermLinks | null>(links ?? null);
  linksRef.current = links ?? null;
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const txRef = useRef<Transport | null>(null);
  const searchRef = useRef<SearchAddon | null>(null);
  // The grid the backend has been told about, and the baseline the coalescing
  // below dedups against. A REF rather than an effect-local because the
  // termVersion effect resizes on its own too, and a baseline only one of the
  // two writers can update is a baseline that lies — it would suppress a resize
  // the pty actually needs. `{ 0, 0 }` means "nothing known", and no real grid
  // can collide with it: the fit addon floors at 2 cols and 1 row.
  const sentRef = useRef({ cols: 0, rows: 0 });
  // Bumped whenever the terminal below is re-created. Refs cannot wake an
  // effect, so anything that has to re-subscribe to THIS xterm (the find bar's
  // results event) depends on this instead.
  const [epoch, setEpoch] = useState(0);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    let disposed = false;
    // A new terminal knows nothing about any backend yet, and the ref outlives
    // the effect that re-created it.
    sentRef.current = { cols: 0, rows: 0 };

    const { family, size, theme } = termOptions();
    // allowProposedApi: the search addon paints its match highlights through
    // `registerDecoration`, and the graphemes addon loads through the
    // `term.unicode` getter — xterm 5.5 gates both behind this flag. Search
    // throws only once you actually ⌘F; unicode throws at load.
    const term = new Terminal({
      fontFamily: family, fontSize: size, cursorBlink: blinkWanted(), theme,
      allowProposedApi: true,
      // xterm's default is 1000 lines, and a full 256K ring is ~3200 lines at 80
      // columns — so the top of every replay was already being dropped on the
      // floor before it could be scrolled to or searched. `TERM_SCROLLBACK` is
      // checked against the backend's `SHELL_RING` by termresize-check.mjs, so
      // raising one without the other fails a gate instead of silently
      // truncating again.
      scrollback: TERM_SCROLLBACK,
    });
    liveTerms.add(term);
    const fit = new FitAddon();
    term.loadAddon(fit);
    // tmux (utf8proc) lays out emoji as 2 cells; xterm's default Unicode 6 tables
    // say 1 — the mismatch garbles every tmux partial repaint. Match tmux.
    term.loadAddon(new UnicodeGraphemesAddon());
    term.unicode.activeVersion = "15-graphemes";
    term.open(host);
    // ⌘F. Loaded AFTER open(): the addon subscribes to the render service, and
    // activating it against a terminal that has not been opened yet leaves the
    // viewport syncing against dimensions that do not exist. `highlightLimit`
    // caps how many matches get a decoration; past it the addon reports
    // resultIndex -1 rather than lying about where you are.
    const search = new SearchAddon({ highlightLimit: 2000 });
    term.loadAddon(search);
    // File-path links, for the panes that were given somewhere to send them.
    // After open(): the provider hangs a listener on `.xterm-screen`, which
    // does not exist before it.
    const linkProv = linksRef.current ? termLinkProvider(term, linksRef, host) : null;
    if (linkProv) term.registerLinkProvider(linkProv);
    const safeFit = () => { try { fit.fit(); } catch { /* renderer not measured yet */ } };
    safeFit();
    termRef.current = term;
    fitRef.current = fit;
    searchRef.current = search;
    setEpoch((v) => v + 1);

    const tx = makeTransport();
    txRef.current = tx;

    // The wheel, for the panes whose history is tmux's (see `wheelToTmux`).
    if (tx.wheel) {
      const pump = wheelPump(tx.wheel);
      let acc = 0;
      term.attachCustomWheelEventHandler((e) => {
        if (!wheelToTmux(term.modes.mouseTrackingMode, term.buffer.active.type)) return true;
        e.preventDefault();
        // The cell height from the GRID: `.xterm-screen` is exactly rows × cell,
        // where the host also carries its content-box padding (termfit-check).
        const grid = term.element?.querySelector<HTMLElement>(".xterm-screen")?.clientHeight || host.clientHeight;
        const r = wheelLines(acc, e.deltaY, e.deltaMode, grid / Math.max(term.rows, 1), term.rows);
        acc = r.acc;
        if (r.lines !== 0) pump(r.lines);
        return false;
      });
    }

    let settle: ReturnType<typeof setTimeout> | undefined;
    const applySize = () => {
      settle = undefined;
      if (!measured(host)) return;
      try {
        fit.fit();
      } catch {
        /* host detached mid-resize */
      }
      const sent = sentRef.current;
      if (term.cols === sent.cols && term.rows === sent.rows) return;
      sentRef.current = { cols: term.cols, rows: term.rows };
      tx.resize(term.cols, term.rows);
    };

    // A re-attached dock shell starts with a REPLAY: the backend's ring, every
    // byte the shell ever wrote, pushed as the channel's first message. It is a
    // recording, and xterm must not answer it. Any terminal query in there —
    // vim's startup burst (two cursor-position reports, DA2, then the colour
    // and cursor-blink queries once it hears back), left behind by every
    // `git commit` without `-m` — is re-issued to this brand-new xterm, which
    // replies down the pty as INPUT, and zsh echoes the printable tail of each
    // reply onto the prompt: `2RR0;276;0c11;rgb:0f0f/0f0f/1616…`, on every
    // place switch and dock re-open until 256K of later output rolls the query
    // out of the ring (screenshot, 2026-09-11). So `onData` is muted while the
    // replay is being parsed. The mute is exact: xterm runs a write's callback
    // synchronously once THAT chunk is parsed and before the next one, so a live
    // chunk queued behind the replay is answered normally.
    //
    // Which message is the replay: the FIRST on the channel, whenever `open`
    // reports one (Tauri delivers a channel's messages in send order, and the
    // backend sends the snapshot before installing the live sink). It cannot be
    // "whatever arrived before `open` resolved" — a payload this size reaches
    // the page through a separate fetch that can land AFTER the invoke's own
    // response. And it may land BEFORE, when `replay` is still unknown; then the
    // first message is presumed a replay, which on a fresh shell costs only the
    // replies to a query in its first chunk of output, and zsh makes none.
    let replay: number | null = null; // bytes the backend replayed; null until `open` answers
    let replayCols: number | null = null; // the grid they were laid out for, if known
    let first = true;
    let parsingReplay = false;
    const onBytes = new Channel<ArrayBuffer>();
    onBytes.onmessage = (msg) => {
      const bytes = new Uint8Array(msg);
      const isReplay = first && replay !== 0;
      first = false;
      if (!isReplay) {
        term.write(bytes);
        return;
      }
      parsingReplay = true;
      // A recording is raw bytes laid out for the grid it was RECORDED at, and
      // nothing in them re-wraps. Replayed narrower, every line that was full
      // stacks a fragment underneath instead of continuing, and the
      // cursor-relative redraws in it address columns that are not there —
      // ROADMAP's "no byte log replays faithfully across a width change".
      //
      // What makes it fixable here is that this terminal is BRAND NEW: the
      // replay is the first thing ever written to it, so nothing else can be
      // damaged by moving the grid. Hand the recording the width it was written
      // for, then put the real one back and let xterm's own buffer reflow do the
      // conversion. It only recovers lines xterm itself wrapped — full-screen
      // output (vim, htop) still cannot be re-laid-out by anyone — so this
      // narrows the parked defect rather than closing it.
      //
      // Skipped when `replayCols` is unknown, which is only when the payload
      // beat `open`'s own response. That degrades in the right direction: a
      // payload big enough to take Tauri's separate-fetch path is one whose
      // invoke has almost certainly landed already, and one small enough to
      // arrive inline is a short recording with little to misplace.
      const rc =
        replayCols != null && replayCols >= 2 && replayCols <= 2000 && replayCols !== term.cols
          ? replayCols
          : null;
      const was = { cols: term.cols, rows: term.rows };
      if (rc != null) term.resize(rc, was.rows);
      term.write(bytes, () => {
        parsingReplay = false;
        if (rc == null) return;
        // Back to the pane's own grid. The backend was never told about the
        // temporary width — `tx.resize` is not called and `sentRef` is
        // deliberately not touched — so the pty never took a SIGWINCH for a size
        // nobody was looking at, and the baseline still says what it always
        // said. `applySize` after it is what corrects the grid if the host
        // genuinely moved while this was parsing.
        term.resize(was.cols, was.rows);
        applySize();
      });
    };

    // Attach at the pane's REAL grid, never at xterm's default (see `measured`).
    // Re-fit each frame while we wait, so the size we finally hand over is the
    // one the host settled on — and give up eventually rather than strand a
    // shell that can never be measured.
    const giveUp = performance.now() + ATTACH_WAIT_MS;
    let raf = 0;

    const attach = () => {
      if (disposed) return;
      safeFit();
      if (!measured(host) && performance.now() < giveUp) {
        raf = requestAnimationFrame(attach);
        return;
      }
      void (async () => {
        try {
          // Captured BEFORE the await: this is the grid the pty is sized to, and
          // it is the only honest baseline. Reading `term.cols/rows` again after
          // the await reads whatever the terminal drifted to meanwhile — and
          // recording THAT as the backend's size masks the very resize the
          // transport dropped while there was no attach to carry it, leaving the
          // pty on the opened grid and the canvas on another, with nothing to
          // correct it until the next gesture. `open` shells out to tmux, so the
          // window is wide enough to hit by mounting while the window animates.
          const opened = { cols: term.cols, rows: term.rows };
          const at = await tx.open(opened.cols, opened.rows, onBytes);
          replay = at.replay;
          replayCols = at.replayCols;
          if (disposed) {
            tx.close();
            return;
          }
          sentRef.current = opened;
          // Settle once more now that there IS an attach: this re-sends anything
          // that moved while `open` was in flight, and is a no-op if the grid is
          // still what we opened at.
          applySize();
          term.onData((data) => {
            if (parsingReplay) return; // a reply to the recording, not to a program
            tx.write(Array.from(new TextEncoder().encode(data)));
          });
          term.focus();
        } catch (e) {
          term.writeln(`\r\n\x1b[31m[worktrees] ${e}\x1b[0m\r\n`);
        }
      })();
    };
    attach();

    const ro = new ResizeObserver(() => {
      // An unmeasured host has nothing true to report — it is the pane on its
      // way out (unmount detaches the node before this observer is disconnected)
      // or on its way in. Reporting it anyway walks the PTY through a size
      // nobody is looking at, and the shell redraws for each one.
      if (!measured(host)) return;
      // Coalesce: one settled size per gesture, not one per frame. See
      // RESIZE_SETTLE_MS for what the per-frame version cost.
      clearTimeout(settle);
      settle = setTimeout(applySize, RESIZE_SETTLE_MS);
    });
    ro.observe(host);

    return () => {
      disposed = true;
      cancelAnimationFrame(raf);
      clearTimeout(settle);
      ro.disconnect();
      tx.close(); // detach, not kill — for either backend
      liveTerms.delete(term);
      linkProv?.dispose();
      term.dispose(); // disposes the loaded addons (and link providers) with it
      termRef.current = null;
      fitRef.current = null;
      searchRef.current = null;
      txRef.current = null;
    };
    // makeTransport is re-created every render; `key` is the real identity
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);

  // Re-grab keyboard focus when the user re-enters the place (clicking any
  // chrome — rows, pin, popovers — moves focus there and nothing else returns
  // it; xterm only self-focuses on a click inside its own canvas).
  useEffect(() => {
    if (focusEnabled) termRef.current?.focus();
  }, [focusToken, focusEnabled]);

  // live re-fit when Settings change the terminal font or theme
  useEffect(() => {
    const term = termRef.current;
    const fit = fitRef.current;
    if (!term || !fit) return;
    const { family, size, theme } = termOptions();
    term.options.fontFamily = family;
    term.options.fontSize = size;
    term.options.theme = theme;
    try {
      fit.fit();
    } catch {
      /* ignore */
    }
    txRef.current?.resize(term.cols, term.rows);
    // INVALIDATE the coalescing baseline rather than writing this size into it.
    // Writing would claim the backend has a grid this send may never have
    // reached — the transports drop a resize made before the attach answers —
    // and an over-claiming baseline suppresses a resize the pty needs. Clearing
    // it can only ever cost one redundant resize on the next gesture, and the
    // attach overwrites it with the truth if one is still in flight.
    sentRef.current = { cols: 0, rows: 0 };
  }, [termVersion]);

  return { hostRef, termRef, searchRef, epoch };
}

/** What ⌘F can and cannot see here. xterm only holds what the app RECEIVED:
 *  attaching to tmux replays the visible screen, not tmux's scrollback, so a
 *  fresh attach starts with almost nothing to search. Said out loud in the
 *  field's tooltip rather than left to be discovered. */
const TERM_HINT =
  "Searches this terminal's output since it was attached (not tmux's own history)";

/** Run a search-addon call without letting it take the pane down with it. The
 *  addon throws for reasons that have nothing to do with the terminal being
 *  usable (a proposed-API gate, a dispose landing mid-search), and an
 *  unhandled throw inside an effect unmounts the whole surface — a blank
 *  terminal because a search failed. Nothing is swallowed: it goes to the app
 *  log the same way `fail()` does on the frontend. */
function guard<T>(what: string, fn: () => T): T | undefined {
  try {
    return fn();
  } catch (e) {
    invoke("log_event", { level: "error", msg: `terminal find (${what}): ${e}` }).catch(() => {});
    return undefined;
  }
}

export type TermFindProps = {
  /** the find bar is showing on THIS pane — App keeps it exclusive */
  findOpen?: boolean;
  /** bumps on every ⌘F, so a second press re-selects the field */
  findToken?: number;
  onFindClose?: () => void;
};

/** The rendered pane: xterm plus its find bar. Both kinds of terminal share it,
 *  so find behaves identically in the main pane and in a dock shell tab. */
function TermSurface({ makeTransport, tkey, termVersion, focusToken, focusEnabled = true, drop, links, findOpen = false, findToken = 0, onFindClose }: {
  makeTransport: () => Transport; tkey: string; termVersion: number; focusToken: number; focusEnabled?: boolean;
  /** ⌘-clickable file paths, or absent for none (see `termLinkProvider`). */
  links?: TermLinks;
  /** `data-drop` for the nav drag's hit-test, or absent. Passed IN rather than
   *  set here because this component is shared: every dock shell tab renders it
   *  too, and only the place's own tmux pane is somewhere a worktree reference
   *  can be dropped. */
  drop?: string;
} & TermFindProps) {
  // The right-click menu on a link. The provider only reports it; the menu is
  // the app's shared `CtxMenu` (clamping, Escape through `useEscape`, and an
  // outside click that lands on its own `.menu-catch`, never on the menu).
  const [menu, setMenu] = useState<LinkMenu | null>(null);
  const withMenu = links ? { ...links, onMenu: setMenu } : undefined;
  const { hostRef, termRef, searchRef, epoch } = useTerm(makeTransport, tkey, termVersion, focusToken, focusEnabled, withMenu);
  // Closing hands the keyboard back to the terminal: the menu took it, and a
  // pane you have to click again before typing is a pane that ate a keystroke.
  const closeMenu = useCallback(() => { setMenu(null); termRef.current?.focus(); }, [termRef]);
  // Every verb closes FIRST and reports through `onError`: an opener invoke
  // that a missing permission rejects does so silently (AGENTS.md).
  const act = (f: () => Promise<unknown> | void) => {
    closeMenu();
    Promise.resolve().then(f).catch((e) => links?.onError?.(e));
  };
  const [query, setQuery] = useState("");
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [res, setRes] = useState({ index: 0, count: 0 });

  // Search options, colours read live so they always match the current theme.
  // Held in a REF as well: the effects below must fire on what actually changed
  // (the query, the case flag, the theme) and not merely because this function
  // has a new identity — every `findNext` moves the selection on, so an effect
  // that re-runs for no reason silently advances the user a match.
  const opts = useCallback(() => {
    const c = findColors();
    return {
      caseSensitive,
      decorations: {
        matchBackground: c.hit,
        matchOverviewRuler: c.hit,
        activeMatchBackground: c.on,
        activeMatchColorOverviewRuler: c.on,
      },
    };
  }, [caseSensitive]);
  const optsRef = useRef(opts);
  optsRef.current = opts;

  useEffect(() => {
    const s = searchRef.current;
    if (!s) return;
    // resultIndex is -1 when the match count blew past the highlight limit.
    const d = s.onDidChangeResults(({ resultIndex, resultCount }) =>
      setRes({ index: resultIndex < 0 ? 0 : resultIndex + 1, count: resultCount }));
    return () => d.dispose();
  }, [searchRef, epoch]);

  // Live search as the query changes. `incremental` keeps the selection anchored
  // on the hit you are already on while you keep typing, instead of jumping.
  useEffect(() => {
    const s = searchRef.current;
    if (!s) return;
    if (!findOpen || !query) {
      guard("clear", () => s.clearDecorations());
      setRes({ index: 0, count: 0 });
      return;
    }
    guard("type", () => s.findNext(query, { incremental: true, ...optsRef.current() }));
  }, [searchRef, findOpen, query, caseSensitive, epoch]);

  // Repaint the matches on a theme switch. Handing `findNext` the new colours is
  // NOT enough: the addon re-highlights only when the TERM or
  // case/regex/wholeWord changed — `_didOptionsChange` never looks at
  // `decorations` — so every non-active match would keep the old theme's hex on
  // the new theme's background. `clearDecorations()` drops the cached term,
  // which forces the next search to lay them all down again. It also makes
  // `_findNextAndSelect` measure from the selection's START rather than its
  // end, so the user stays on the match they were on. Skipped on the first run:
  // nothing is painted yet.
  const themeSeen = useRef(false);
  useEffect(() => {
    if (!themeSeen.current) { themeSeen.current = true; return; }
    const s = searchRef.current;
    if (!s || !findOpen || !query) return;
    guard("theme", () => { s.clearDecorations(); s.findNext(query, optsRef.current()); });
    // Only the theme belongs here; the effect above owns query/case changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [termVersion]);

  const next = useCallback(() => {
    if (query) guard("next", () => searchRef.current?.findNext(query, optsRef.current()));
  }, [searchRef, query]);
  const prev = useCallback(() => {
    if (query) guard("prev", () => searchRef.current?.findPrevious(query, optsRef.current()));
  }, [searchRef, query]);
  const close = useCallback(() => {
    guard("clear", () => searchRef.current?.clearDecorations());
    onFindClose?.();
    termRef.current?.focus();
  }, [searchRef, termRef, onFindClose]);

  return (
    <div className="term-wrap" data-drop={drop}>
      <div ref={hostRef} className="term-host" />
      {findOpen && (
        <FindBar
          query={query} onQuery={setQuery}
          // index 0 with matches present means the addon gave up placing you
          // (more matches than `highlightLimit`) — `capped` renders that as
          // "2000+" instead of a nonsensical "0/2000".
          index={res.index} count={res.count} capped={res.count > 0 && res.index === 0}
          caseSensitive={caseSensitive} onCaseSensitive={setCaseSensitive}
          onNext={next} onPrev={prev} onClose={close}
          focusToken={findToken} hint={TERM_HINT}
        />
      )}
      {menu && links && (
        <CtxMenu x={menu.x} y={menu.y} onClose={closeMenu}>
          <div className="pop-hint path" title={menu.path}>{relPath(links.root, menu.path)}{menu.line ? `:${menu.line}${menu.col ? `:${menu.col}` : ""}` : ""}</div>
          <button className="pop-item" onClick={() => act(() => links.onOpen(menu.path, menu.line, menu.col))}>Open in viewer</button>
          <button className="pop-item" onClick={() => act(() => revealItemInDir(menu.path))}>Reveal in Finder</button>
          <div className="ctx-sep" />
          {/* Both, as the Files tree's menu offers: the absolute path is what
              the link RESOLVED to (unambiguous anywhere you paste it), the
              relative one is what you would type in this place. A path
              outside the place root keeps its absolute form (`relPath`). */}
          <button className="pop-item" onClick={() => act(() => copyToClipboard(menu.path))}>Copy path</button>
          <button className="pop-item" onClick={() => act(() => copyToClipboard(relPath(links.root, menu.path)))}>Copy relative path</button>
        </CtxMenu>
      )}
    </div>
  );
}

/** Where a pane's paths resolve and what opening one does. Optional for both
 *  panes: no `onOpenPath` means no links. */
export type PaneLinkProps = {
  /** The place's directory — what a relative path in its output is relative to. */
  root?: string;
  onOpenPath?: (path: string, line?: number, col?: number) => void;
  /** Where the link menu's failures go (a refused reveal, a failed copy). */
  onError?: (e: unknown) => void;
};

export function TerminalPane({ session, provider = "claude", termVersion = 0, focusToken = 0, focusEnabled = true, root, onOpenPath, onError, ...find }: {
  session: string; provider?: Harness; termVersion?: number; focusToken?: number; focusEnabled?: boolean;
} & PaneLinkProps & TermFindProps) {
  // The tmux pane resolves against its active pane's LIVE cwd, then the place
  // root (where claude, codex and pi start). The backend reads the cwd.
  const links = root && onOpenPath ? { root, session, onOpen: onOpenPath, onError } : undefined;
  return (
    <TermSurface makeTransport={() => tmuxTransport(session)} tkey={session}
      termVersion={termVersion} focusToken={focusToken} focusEnabled={focusEnabled} drop={provider === "claude" ? "mention" : undefined}
      links={links} {...find} />
  );
}

export function ShellPane({ repo, slug, index, termVersion = 0, focusToken = 0, root, onOpenPath, onError, ...find }: {
  repo: string; slug: string; index: number; termVersion?: number; focusToken?: number;
} & PaneLinkProps & TermFindProps) {
  const links = root && onOpenPath ? { root, shell: { repo, slug, index }, onOpen: onOpenPath, onError } : undefined;
  return (
    <TermSurface makeTransport={() => shellTransport(repo, slug, index)}
      tkey={`${repo}|${slug}|${index}`}
      termVersion={termVersion} focusToken={focusToken} links={links} {...find} />
  );
}
