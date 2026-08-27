// Deterministic check of how TerminalPane talks to the pty about SIZE: that a
// resize gesture costs ONE resize rather than one per frame, and that no resize
// is ever lost to the coalescing.
//
// Two bugs live here. They are independent, and the fix for the first made the
// second permanent rather than causing it.
//
//   The storm. The ResizeObserver called `tx.resize()` on every callback. Every
//   distinct grid handed to the pty is a `TIOCSWINSZ`, so a SIGWINCH, and the
//   shell answers each one by reprinting its prompt — a 240px drag of the pane
//   walked the terminal through 17 row counts and left 17 stacked prompt lines
//   behind (a truncated `~/workspace/…` plus a full-width rule per step), in the
//   main pane and the dock shell at once. It spent 120 `term_resize` invokes to
//   do it, 103 of them re-stating a size that had not changed: `fit()` skips
//   `term.resize` when the grid is the same, but the invoke after it did not.
//   (_tmp screenshots, 2026-08-27.)
//
//   The lost resize. A size can be settled while `term_open` / `shell_open` is
//   STILL IN FLIGHT — and both transports gate `resize` on the attach having
//   answered, so that invoke is silently dropped. This PRE-DATES the coalescing:
//   run this check against the original per-frame version and test 5 fails there
//   too. What the per-frame version had was an accident that hid it — it re-sent
//   unconditionally on the next observer callback, so any later movement of the
//   pane papered over the drop. Coalescing removed the accident, and a dedup
//   made it permanent: whatever the attach records as "the size the backend has"
//   must therefore be the grid it PASSED to `open`, not the grid the terminal is
//   on by the time `open` resolves. Seeding it from the live terminal records a
//   size the pty never got and masks exactly the resize that was dropped — pty
//   at the opened size, canvas at another, tmux painting the wrong width until
//   the next grid-changing gesture. The window is real (`open` shells out to
//   tmux) and mounting while the window animates — entering full screen, say —
//   is how you land in it.
//
// No suite can see either one. The mock harness answers invokes in a microtask
// and mounts no ResizeObserver, the bats cases never reach the frontend, and
// every size the old code sent was CORRECT at the moment it was sent — the cost
// was the count, and nothing reads back a count. Same family as termfit-check.mjs
// (which guards the geometry of the same element) and race-check.mjs /
// ctxmenu-check.mjs, whose approach this borrows: it does NOT paraphrase the
// component, it evaluates the REAL source text of TerminalPane.tsx under stubs,
// so running it before and after an edit tests the edit itself.
//
// Time is VIRTUAL. The component's `setTimeout`/`clearTimeout` are free
// identifiers inside the `build(env)` wrapper, so `env` shadows them and a clock
// this file controls drives the settle exactly. An earlier version slept on real
// timers, which put the strict "one gesture, one resize" assertion at the mercy
// of the scheduler: one 80ms event-loop stall anywhere in a 120-await loop
// flushes mid-drag and fails with a message accusing the component. Rare on an
// idle laptop, occasional on a loaded shared runner — and the worst kind of
// flake, because it reads as a real finding.
//
//   node termresize-check.mjs [path/to/TerminalPane.tsx]   exits non-zero on failure
import fs from "node:fs";
import { fileURLToPath } from "node:url";
// vite's esbuild re-export — see race-check.mjs: bare "esbuild" does not resolve
// under pnpm's strict layout, vite does (direct dependency).
import { transformWithEsbuild } from "vite";

const SRC = process.argv[2] || fileURLToPath(new URL("../src/TerminalPane.tsx", import.meta.url));
const raw = fs.readFileSync(SRC, "utf8");

// Every import is replaced by an `env` binding (react hooks, xterm, the addons,
// the tauri bridge, the find bar) — including the bare CSS side-effect import.
// Everything else, module scope included, is verbatim.
//
// `export` is illegal inside a function body, so the module's own exports are
// un-exported — they still get evaluated, which is the point: a syntax or
// reference error anywhere in the file fails this check rather than sliding
// past it. Only the forms the file actually uses are rewritten; anything else
// (`export default`, `export { x }`) reaches esbuild as a nested `export` and
// throws, which is the loud failure we want rather than a silent skip.
const body = raw
  .split("\n")
  .filter((l) => !/^\s*import\s/.test(l))
  .join("\n")
  .replace(/^export (function|const|type|class) /gm, "$1 ");
if (!body.includes("function useTerm")) throw new Error("useTerm not found in " + SRC);

const wrapped = `
export function build(env: any) {
  const {
    useCallback, useEffect, useRef, useState,
    Terminal, FitAddon, SearchAddon, UnicodeGraphemesAddon, Channel, invoke,
    FindBar, findColors,
    window, document, getComputedStyle, performance,
    requestAnimationFrame, cancelAnimationFrame, setTimeout, clearTimeout,
    TextEncoder,
    h, F,
  } = env;
${body}
  return { useTerm };
}`;
const js = (await transformWithEsbuild(wrapped, "termresize-check.tsx", {
  loader: "tsx", format: "esm", jsx: "transform", jsxFactory: "h", jsxFragment: "F",
})).code;
const { build } = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

// ── virtual clock ────────────────────────────────────────────────────────────
const CELL_W = 8;
const CELL_H = 15;   // the harness measured 15px at --term-size: 13px

function makeClock() {
  let now = 0, seq = 0;
  const timers = new Map();
  return {
    now: () => now,
    setTimeout(fn, ms) { const id = ++seq; timers.set(id, { at: now + (ms || 0), fn }); return id; },
    clearTimeout(id) { timers.delete(id); },
    /** Run every timer due within `ms`, in time order, including ones armed
     *  while advancing. Each callback is removed before it runs, so it cannot
     *  re-fire ITSELF — but that is the only case covered: a callback that arms
     *  a NEW 0ms timer every time it runs would spin here forever. Nothing does
     *  (the settle never re-arms itself, the observer's delivery is one-shot,
     *  and raf is 16ms), so this is a note for whoever adds the first one. */
    advance(ms) {
      const target = now + ms;
      for (;;) {
        let next = null;
        for (const [id, t] of timers) if (t.at <= target && (!next || t.at < next.t.at)) next = { id, t };
        if (!next) break;
        timers.delete(next.id);
        now = next.t.at;
        next.t.fn();
      }
      now = target;
    },
  };
}
/** Let awaited promises resolve. The clock is virtual; microtasks are not. */
const tick = () => new Promise((r) => globalThis.setTimeout(r, 0));

// ── one mount of useTerm, with a host whose size we drive by hand ────────────
function mount({ w = 1000, h = 600, openDelayMs = 0 } = {}) {
  const clock = makeClock();
  // A font change moves the grid by changing the CELL, not the box — which is
  // why it is the `termVersion` effect's job and not the observer's.
  const cell = { w: CELL_W, h: CELL_H };
  const invokes = [];       // what the backend was actually told
  const dropped = [];       // resizes made before the attach answered
  const host = { clientWidth: w, clientHeight: h, _w: w, _h: h };
  let roCallback = null;

  class TerminalStub {
    constructor(opts) {
      this.options = { ...opts };
      this.unicode = { activeVersion: "" };
      this.cols = 80;
      this.rows = 24;
      this.element = {};
    }
    loadAddon(a) { a.activate?.(this); }
    open() {}
    write() {}
    writeln() {}
    focus() {}
    onData() {}
    dispose() {}
    resize(cols, rows) { this.cols = cols; this.rows = rows; }
  }
  class FitAddonStub {
    activate(term) { this.term = term; }
    dispose() {}
    fit() {
      // The real addon's arithmetic, minus the CSS reading (termfit-check.mjs is
      // the guard for that): whole cells only, the same `Math.max` floors, and
      // `term.resize` skipped when the grid has not changed — which is why the
      // old code's redundant invokes were invisible to `term.cols`/`rows`.
      const cols = Math.max(2, Math.floor(host._w / cell.w));
      const rows = Math.max(1, Math.floor(host._h / cell.h));
      if (cols !== this.term.cols || rows !== this.term.rows) this.term.resize(cols, rows);
    }
  }

  const effects = [];
  const env = {
    // ── react ──
    useRef: (init) => ({ current: init }),
    useState: (init) => [typeof init === "function" ? init() : init, () => {}],
    useCallback: (fn) => fn,
    useEffect: (cb, deps) => { effects.push({ cb, deps }); },
    // ── xterm ──
    Terminal: TerminalStub,
    FitAddon: FitAddonStub,
    SearchAddon: class { activate() {} dispose() {} onDidChangeResults() { return { dispose() {} }; } clearDecorations() {} findNext() {} findPrevious() {} },
    UnicodeGraphemesAddon: class { activate() {} dispose() {} },
    // ── tauri ──
    Channel: class { constructor() { this.onmessage = null; } },
    invoke: () => Promise.resolve(1),
    // ── the rest of the module's surface ──
    FindBar: () => null,
    findColors: () => ({ hit: "#000", on: "#fff" }),
    h: () => null,
    F: null,
    window: { addEventListener() {}, removeEventListener() {} },
    document: {
      addEventListener() {}, removeEventListener() {},
      documentElement: {},
      visibilityState: "visible",
      hasFocus: () => true,
    },
    getComputedStyle: () => ({ getPropertyValue: () => "" }),
    performance: { now: () => clock.now() },
    requestAnimationFrame: (cb) => clock.setTimeout(cb, 16),
    cancelAnimationFrame: (id) => clock.clearTimeout(id),
    setTimeout: (fn, ms) => clock.setTimeout(fn, ms),
    clearTimeout: (id) => clock.clearTimeout(id),
    TextEncoder,
  };

  const prevRO = globalThis.ResizeObserver;
  globalThis.ResizeObserver = class {
    constructor(cb) { roCallback = cb; }
    // A real ResizeObserver delivers once, asynchronously, on observe — which is
    // what arms the settle on every mount, and so part of the attach window
    // under test.
    observe() { clock.setTimeout(() => roCallback?.([], this), 0); }
    disconnect() { roCallback = null; }
  };

  // The transport, faithful on the one point that matters: `resize` before the
  // attach has answered goes NOWHERE. Both real transports gate it on the id /
  // attach generation `open` resolves with (TerminalPane.tsx, tmuxTransport and
  // shellTransport), so the invoke is never made and nothing reports it.
  let attached = false;
  const transport = () => ({
    open: (cols, rows) => {
      invokes.push({ cmd: "term_open", args: { cols, rows } });
      if (!openDelayMs) { attached = true; return Promise.resolve(1); }
      // The real one shells out to tmux; a slow attach is the whole point here.
      return new Promise((res) => clock.setTimeout(() => { attached = true; res(1); }, openDelayMs));
    },
    write: () => {},
    resize: (cols, rows) => {
      if (!attached) { dropped.push({ cols, rows }); return; }
      invokes.push({ cmd: "term_resize", args: { cols, rows } });
    },
    close: () => {},
  });

  const { useTerm } = build(env);
  // termVersion is a RECOGNISABLE value, so the effect that owns it can be found
  // by its deps rather than by an index that shifts when an effect is added.
  const TERM_VERSION = 7;
  const api = useTerm(transport, "session-1", TERM_VERSION, 0);
  // React attaches refs during commit, BEFORE effects run.
  api.hostRef.current = host;
  const cleanups = effects.map(({ cb }) => cb()).filter((c) => typeof c === "function");

  return {
    clock, invokes, dropped,
    resizes: () => invokes.filter((i) => i.cmd === "term_resize"),
    open: () => invokes.find((i) => i.cmd === "term_open"),
    /** The grid the pty is actually on: what `open` asked for, then every
     *  resize that was not dropped. */
    ptyGrid() {
      const last = this.resizes().at(-1) ?? this.open();
      return last ? { cols: last.args.cols, rows: last.args.rows } : null;
    },
    /** Re-fit for a new cell size and re-run the effect that owns [termVersion]
     *  — exactly what a Settings font change does. */
    fontChange(cw, ch) {
      cell.w = cw; cell.h = ch;
      const e = effects.find((x) => x.deps && x.deps.length === 1 && x.deps[0] === TERM_VERSION);
      if (!e) throw new Error("no effect keyed on termVersion — has useTerm been restructured?");
      const cleanup = e.cb();
      if (typeof cleanup === "function") cleanups.push(cleanup);
    },
    /** Move the pane and deliver the observer callback, as the browser would. */
    setSize(nw, nh) {
      host._w = host.clientWidth = nw;
      host._h = host.clientHeight = nh;
      roCallback?.([], null);
    },
    dispose() {
      cleanups.forEach((c) => c());
      globalThis.ResizeObserver = prevRO;
    },
  };
}

const gridFor = (w, h) => ({ cols: Math.floor(w / CELL_W), rows: Math.floor(h / CELL_H) });
const same = (a, b) => !!a && !!b && a.cols === b.cols && a.rows === b.rows;
const show = (g) => (g ? `${g.cols}x${g.rows}` : "nothing");

// ── assertions ───────────────────────────────────────────────────────────────
let failed = 0;
const fail = (msg) => { failed++; console.log(`not ok — ${msg}`); };
const ok = (msg) => console.log(`ok — ${msg}`);

// The settle window the component declares. Read from the source rather than
// hardcoded: a change to it should not silently retune this check's waits.
const settleMs = Number(/RESIZE_SETTLE_MS\s*=\s*(\d+)/.exec(raw)?.[1] ?? NaN);
if (!Number.isFinite(settleMs)) fail("no RESIZE_SETTLE_MS in the source — resizes are not coalesced at all");
else ok(`RESIZE_SETTLE_MS = ${settleMs}ms`);
const SETTLE = Number.isFinite(settleMs) ? settleMs : 80;
const FRAME = Math.max(1, Math.floor(SETTLE / 8));   // frames closer together than the settle
const PAST = SETTLE * 2 + 20;                        // comfortably past it

/** Mount, let the attach and its mount-time settle finish, and hand back a
 *  baseline resize count. */
async function settled(opts) {
  const m = mount(opts);
  m.clock.advance(PAST);
  await tick();
  m.clock.advance(PAST);
  await tick();
  return m;
}

// ── 1. one gesture, one resize ───────────────────────────────────────────────
// 120 frames of drag, 240px of travel — the measured shape of the bug.
{
  const m = await settled({ w: 1000, h: 900 });
  const before = m.resizes().length;
  for (let i = 0; i < 120; i++) {
    m.setSize(1000, 900 - i * 2);
    m.clock.advance(FRAME);
  }
  const during = m.resizes().length - before;
  m.clock.advance(PAST);
  await tick();
  const n = m.resizes().length - before;
  if (during === 0) ok("nothing is sent mid-drag");
  else fail(`${during} pty resize(s) sent DURING the drag — the settle is not holding them`);
  if (n === 1) ok("a 120-frame / 240px drag costs ONE pty resize (was 120)");
  else fail(`a 120-frame drag sent ${n} pty resizes — every distinct one is a SIGWINCH, and the shell reprints its prompt for each`);
  // Coalescing that loses the FINAL size would be worse than the storm.
  const want = gridFor(1000, 900 - 119 * 2);
  if (same(m.ptyGrid(), want)) ok(`the size that lands is the one the drag ended on (${show(want)})`);
  else fail(`the drag ended at ${show(want)} but the pty is on ${show(m.ptyGrid())}`);
  m.dispose();
}

// ── 2. coalescing is not swallowing ──────────────────────────────────────────
// Two gestures with a pause between them are two resizes. Without this a check
// for "few resizes" would pass a component that never resized at all.
{
  const m = await settled({ w: 1000, h: 900 });
  const before = m.resizes().length;
  m.setSize(1000, 800);
  m.clock.advance(PAST);
  await tick();
  m.setSize(1000, 700);
  m.clock.advance(PAST);
  await tick();
  const n = m.resizes().length - before;
  if (n === 2) ok("two separate gestures are two resizes (coalescing, not swallowing)");
  else fail(`two gestures ${PAST}ms apart produced ${n} resizes, expected 2`);
  m.dispose();
}

// ── 3. a sub-cell move tells the backend nothing ─────────────────────────────
// The grid is whole cells, so a 1px nudge changes no grid. The old code sent one
// anyway — 103 of the drag's 120 invokes were this.
{
  const m = await settled({ w: 1000, h: 900 });
  const before = m.resizes().length;
  m.setSize(1001, 901);
  m.clock.advance(PAST);
  await tick();
  const n = m.resizes().length - before;
  if (n === 0) ok("a sub-cell resize sends nothing (the grid did not change)");
  else fail(`a 1px nudge sent ${n} pty resize(s) for a grid that did not change`);
  m.dispose();
}

// ── 4. the attach still opens at the pane's real size ────────────────────────
// The coalescing must not delay the ATTACH: opening at xterm's 80x24 default
// walks the shell real -> 80x24 -> real, and the replay ring is then rendered
// into a terminal it was never written for (see `measured`'s docstring).
{
  const m = await settled({ w: 1000, h: 900 });
  const want = gridFor(1000, 900);
  if (same(m.open() && { cols: m.open().args.cols, rows: m.open().args.rows }, want))
    ok(`the attach opens at the pane's real grid (${show(want)}), not 80x24`);
  else fail(`term_open asked for ${m.open() ? `${m.open().args.cols}x${m.open().args.rows}` : "nothing"}, expected ${show(want)}`);
  m.dispose();
}

// ── 5. a resize that lands DURING the attach is not lost ─────────────────────
// The regression the coalescing introduced. `open` shells out to tmux, so it can
// easily outlast a settle; the resize that fires meanwhile is dropped by the
// transport (no id yet), and whatever the attach then records as the backend's
// size decides whether anything ever corrects it.
{
  const m = mount({ w: 1000, h: 900, openDelayMs: SETTLE * 4 });
  await tick();
  m.clock.advance(1);                 // the observer's mount-time delivery
  m.setSize(1000, 600);               // the pane moves while `open` is in flight
  m.clock.advance(PAST);              // the settle fires: canvas moves, resize dropped
  await tick();
  if (m.dropped.length) ok(`a resize during the attach is dropped by the transport (${m.dropped.length}) — as the real ones do`);
  else console.log("ok — (no resize was dropped; the settle did not fire inside the attach window)");
  m.clock.advance(SETTLE * 6);        // `open` resolves
  await tick();
  m.clock.advance(PAST);
  await tick();
  const want = gridFor(1000, 600);
  if (same(m.ptyGrid(), want))
    ok(`the pty ends up on the grid the pane actually has (${show(want)})`);
  else
    fail(`the pane settled at ${show(want)} while the attach was in flight, but the pty is stuck on ${show(m.ptyGrid())} — the dropped resize was masked, so tmux paints the wrong size until the next gesture`);
  m.dispose();
}

// ── 6. a font change does not poison the dedup ───────────────────────────────
// The `termVersion` effect re-fits and resizes on its OWN, outside the
// observer's closure. If the dedup's baseline is private to that closure, the
// effect's send goes unrecorded and the baseline becomes a lie — a later gesture
// that lands back on the pre-font-change grid is then suppressed as a duplicate
// while the pty is somewhere else entirely.
{
  const m = await settled({ w: 1000, h: 900 });
  const before = m.ptyGrid();                       // 125x60 at the 8x15 cell

  // Settings: a bigger font. Same box, bigger cell, so the grid shrinks and the
  // termVersion effect resizes the pty to it.
  m.fontChange(10, 20);
  await tick();
  const afterFont = m.ptyGrid();
  if (!same(afterFont, before)) ok(`a font change resizes the pty on its own (${show(before)} -> ${show(afterFont)})`);
  else fail(`the font change did not reach the pty at all (still ${show(before)})`);

  // Now drag the pane to a box that lands the NEW cell exactly back on the grid
  // the pty had before the font change. Nothing about that grid is stale: the
  // pty is on `afterFont` and genuinely needs to be told.
  m.setSize(before.cols * 10, before.rows * 20);
  m.clock.advance(PAST);
  await tick();
  if (same(m.ptyGrid(), before))
    ok("a grid the pty held BEFORE a font change is re-sent when a gesture returns to it");
  else
    fail(`the pane settled at ${show(before)} but the pty is still on ${show(m.ptyGrid())} — the font change's resize was never recorded, so the dedup suppressed a resize the pty needed`);
  m.dispose();
}

console.log(failed ? `\n${failed} failure(s)` : "\nall good");
process.exit(failed ? 1 : 0);
