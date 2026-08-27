// Deterministic check that a RESIZE GESTURE costs ONE pty resize, not one per
// frame.
//
// The bug this locks down: `TerminalPane`'s ResizeObserver called `tx.resize()`
// on every callback. Every distinct grid handed to the pty is a `TIOCSWINSZ`,
// so a SIGWINCH, and the shell answers each one by reprinting its prompt — a
// 240px drag of the pane walked the terminal through 17 row counts and left 17
// stacked prompt lines behind (one truncated `~/workspace/…` plus a full-width
// rule per step), in the main pane and the dock shell at the same time. It spent
// 120 `term_resize` invokes to do it, 103 of them re-stating a size that had not
// changed: `fit()` skips `term.resize` when the grid is the same, but the invoke
// after it did not. (_tmp screenshots, 2026-08-27.)
//
// No suite can see this. The mock harness answers invokes in a microtask and
// mounts no ResizeObserver, the bats cases never reach the frontend, and every
// size the old code sent was CORRECT at the moment it was sent — the cost was
// the count, and nothing reads back a count. Same family as termfit-check.mjs
// (which guards the geometry of the same element) and race-check.mjs /
// ctxmenu-check.mjs, whose approach this borrows: it does NOT paraphrase the
// component, it evaluates the REAL source text of TerminalPane.tsx under stubs,
// so running it before and after an edit tests the edit itself.
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
// `export` is illegal inside a function body, so the module's own exports
// (TermFindProps, TerminalPane, ShellPane) are un-exported — they still get
// evaluated, which is the point: a syntax or reference error anywhere in the
// file fails this check rather than sliding past it.
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
    requestAnimationFrame, cancelAnimationFrame, TextEncoder,
    h, F,
  } = env;
${body}
  return { useTerm };
}`;
const js = (await transformWithEsbuild(wrapped, "termresize-check.tsx", {
  loader: "tsx", format: "esm", jsx: "transform", jsxFactory: "h", jsxFragment: "F",
})).code;
const { build } = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

// ── stubs ────────────────────────────────────────────────────────────────────
const CELL_W = 8;
const CELL_H = 15;   // the harness measured 15px at --term-size: 13px

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** One mount of useTerm against a host whose size we drive by hand. */
function mount({ w = 1000, h = 600 } = {}) {
  const invokes = [];                 // every backend call, in order
  const host = {
    clientWidth: w, clientHeight: h,
    // the fit stub reads these; the real addon reads them through
    // getComputedStyle, which termfit-check.mjs is the guard for
    _w: w, _h: h,
  };
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
      // The real addon's arithmetic, minus the CSS reading: whole cells only,
      // and `term.resize` is skipped when the grid has not changed (xterm's
      // fit addon does exactly this, and it is why the old code's redundant
      // invokes were invisible to `term.cols`/`rows`).
      const cols = Math.max(2, Math.floor(host._w / CELL_W));
      const rows = Math.max(1, Math.floor(host._h / CELL_H));
      if (cols !== this.term.cols || rows !== this.term.rows) this.term.resize(cols, rows);
    }
  }
  const noopAddon = { activate() {}, dispose() {} };

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
    invoke: (cmd, args) => {
      invokes.push({ cmd, args });
      // term_open / shell_open answer with an id / attach generation, and both
      // transports gate every later call on having one.
      return Promise.resolve(1);
    },
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
    performance: { now: () => Date.now() },
    requestAnimationFrame: (cb) => setTimeout(cb, 0),
    cancelAnimationFrame: (id) => clearTimeout(id),
    TextEncoder,
  };

  const effects = [];
  const prevRO = globalThis.ResizeObserver;
  globalThis.ResizeObserver = class {
    constructor(cb) { roCallback = cb; }
    observe() {}
    disconnect() { roCallback = null; }
  };

  const { useTerm } = build(env);
  const transport = () => ({
    open: (cols, rows) => env.invoke("term_open", { cols, rows }),
    write: () => {},
    resize: (cols, rows) => env.invoke("term_resize", { cols, rows }),
    close: () => {},
  });
  const api = useTerm(transport, "session-1", 0, 0);
  // React attaches refs during commit, BEFORE effects run.
  api.hostRef.current = host;
  const cleanups = effects.map(({ cb }) => cb()).filter((c) => typeof c === "function");

  return {
    invokes,
    resizes: () => invokes.filter((i) => i.cmd === "term_resize"),
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

// ── assertions ───────────────────────────────────────────────────────────────
let failed = 0;
const fail = (msg) => { failed++; console.log(`not ok — ${msg}`); };
const ok = (msg) => console.log(`ok — ${msg}`);

// The settle window the component declares. Read from the source rather than
// hardcoded: a change to it should not silently retune this check's waits.
const settleMs = Number(/RESIZE_SETTLE_MS\s*=\s*(\d+)/.exec(raw)?.[1] ?? NaN);
if (!Number.isFinite(settleMs)) {
  fail("no RESIZE_SETTLE_MS in the source — resizes are not coalesced at all");
} else {
  ok(`RESIZE_SETTLE_MS = ${settleMs}ms`);
}
const wait = Number.isFinite(settleMs) ? settleMs * 3 + 60 : 300;

// ── 1. one gesture, one resize ───────────────────────────────────────────────
// 120 frames of drag, 240px of travel — the measured shape of the bug. Frames
// 5ms apart: closer together than any sane settle, and slow enough that a
// per-frame implementation cannot be excused as "it was all one tick".
{
  const m = mount({ w: 1000, h: 900 });
  await sleep(wait);                       // let the attach settle first
  const before = m.resizes().length;
  for (let i = 0; i < 120; i++) {
    m.setSize(1000, 900 - i * 2);
    await sleep(5);
  }
  await sleep(wait);
  const n = m.resizes().length - before;
  const last = m.resizes().at(-1);
  if (n === 1) ok(`a 120-frame / 240px drag costs ONE pty resize (was 120)`);
  else fail(`a 120-frame drag sent ${n} pty resizes — every distinct one is a SIGWINCH, and the shell reprints its prompt for each`);
  // Coalescing that loses the FINAL size would be worse than the storm.
  const wantRows = Math.floor((900 - 119 * 2) / CELL_H);
  if (last && last.args.rows === wantRows) ok(`the size that lands is the one the drag ended on (${last.args.cols}x${last.args.rows})`);
  else fail(`the drag ended at rows=${wantRows} but the backend was last told ${last ? last.args.rows : "nothing"}`);
  m.dispose();
}

// ── 2. coalescing is not swallowing ──────────────────────────────────────────
// Two gestures with a pause between them are two resizes. Without this a check
// for "few resizes" would pass a component that never resized at all.
{
  const m = mount({ w: 1000, h: 900 });
  await sleep(wait);
  const before = m.resizes().length;
  m.setSize(1000, 800);
  await sleep(wait);
  m.setSize(1000, 700);
  await sleep(wait);
  const n = m.resizes().length - before;
  if (n === 2) ok("two separate gestures are two resizes (coalescing, not swallowing)");
  else fail(`two gestures ${wait}ms apart produced ${n} resizes, expected 2`);
  m.dispose();
}

// ── 3. a sub-cell move tells the backend nothing ─────────────────────────────
// The grid is whole cells, so a 1px nudge changes no grid. The old code sent
// one anyway — 103 of the drag's 120 invokes were this.
{
  const m = mount({ w: 1000, h: 900 });
  await sleep(wait);
  const before = m.resizes().length;
  m.setSize(1001, 901);                    // same cols/rows, different box
  await sleep(wait);
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
  const m = mount({ w: 1000, h: 900 });
  await sleep(wait);
  const open = m.invokes.find((i) => i.cmd === "term_open");
  const wantCols = Math.floor(1000 / CELL_W), wantRows = Math.floor(900 / CELL_H);
  if (open && open.args.cols === wantCols && open.args.rows === wantRows)
    ok(`the attach opens at the pane's real grid (${wantCols}x${wantRows}), not 80x24`);
  else fail(`term_open asked for ${open ? `${open.args.cols}x${open.args.rows}` : "nothing"}, expected ${wantCols}x${wantRows}`);
  m.dispose();
}

console.log(failed ? `\n${failed} failure(s)` : "\nall good");
process.exit(failed ? 1 : 0);
