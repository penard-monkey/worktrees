// Deterministic check that the mouse wheel scrolls a tmux pane's HISTORY when
// the program in it has not asked for the mouse.
//
// The main pane is xterm attached to a tmux CLIENT, and the client always draws
// on the alternate screen, so xterm itself never has scrollback. With no mouse
// mode on, xterm.js 5.5 does what every terminal does for an alternate screen:
// it turns each wheel notch into ESC [ A / ESC [ B ("Convert wheel events into
// up/down events", CoreBrowserTerminal.ts). A program on the pane's MAIN screen
// then receives arrow keys instead of a scroll — pi reads ↑ as editor history,
// zsh as history recall — and its real history, tmux's scrollback, is
// unreachable. Measured 2026-09-29 on a throwaway tmux server: a `seq 1 300;
// cat -v` pane under an attached client received `^[[A^[[A^[[A` for three
// notches. claude is unaffected because it turns the mouse on (tmux passes
// that through, and xterm then reports the wheel as mouse events).
//
// The fix routes those wheels to `term_wheel`, which scrolls tmux copy-mode (or
// sends the same arrows for an alternate-screen program — lib.rs's
// `wheel_plan`, unit-tested there). This pins the frontend half:
//
//   WHICH wheels go to tmux: no mouse mode AND the alternate buffer. A pane with
//   the mouse on must keep xterm's mouse reporting — xterm consults the custom
//   handler on that path too, so a handler that always cancels breaks claude.
//   A dock shell (owned pty, normal buffer, no `wheel` on its transport) keeps
//   xterm's own scrollback.
//
//   HOW MANY invokes: one in flight per pane, the rest summed into the next. A
//   trackpad fling is ~60 events a second and each flush is two tmux spawns.
//
// No suite can see this: the mock harness has no tmux, bats never reaches the
// frontend. Same shape as termreplay-check.mjs — the REAL source of
// TerminalPane.tsx is evaluated under stubs, so it fails on the version before
// the fix.
//
//   node termwheel-check.mjs [path/to/TerminalPane.tsx]   exits non-zero on failure
import fs from "node:fs";
import { transformWithEsbuild } from "vite";

const SRC = process.argv[2] ?? new URL("../src/TerminalPane.tsx", import.meta.url).pathname;
const raw = fs.readFileSync(SRC, "utf8");

// See termresize-check.mjs for why the source is sliced this way.
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
  const pick = (name: string) => { try { return eval(name); } catch { return null; } };
  return { useTerm, tmuxTransport, wheelToTmux: pick("wheelToTmux"), wheelLines: pick("wheelLines") };
}`;
const js = (await transformWithEsbuild(wrapped, "termwheel-check.tsx", {
  loader: "tsx", format: "esm", jsx: "transform", jsxFactory: "h", jsxFragment: "F",
})).code;
const { build } = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

const tick = () => new Promise((r) => setTimeout(r, 0));

let failed = 0;
const fail = (msg) => { failed++; console.log(`not ok — ${msg}`); };
const ok = (msg) => console.log(`ok — ${msg}`);
const check = (cond, msg) => (cond ? ok(msg) : fail(msg));

/** Mount useTerm with a stub xterm that records the wheel handler, and a
 *  transport whose `wheel` invokes the test resolves by hand. */
function mount({ withWheel = true, invokes = null } = {}) {
  const host = { clientWidth: 800, clientHeight: 400 }; // 20 rows → 20px cells
  let term = null;
  const wheels = [];      // { lines, resolve }
  const written = [];     // what reached the pty as INPUT (tx.write)

  class TerminalStub {
    constructor(opts) {
      this.options = { ...opts }; this.unicode = { activeVersion: "" };
      this.cols = 100; this.rows = 20; this.element = {};
      this.modes = { mouseTrackingMode: "none" };
      this.buffer = { active: { type: "alternate" } };
      this.wheelHandler = null;
      term = this;
    }
    loadAddon(a) { a.activate?.(this); }
    open() {} writeln() {} focus() {} dispose() {} write() {}
    resize(c, r) { this.cols = c; this.rows = r; }
    onData(fn) { this.listener = fn; }
    attachCustomWheelEventHandler(fn) { this.wheelHandler = fn; }
    /** What xterm 5.5 does with a wheel event, reduced to the two outcomes that
     *  matter: the custom handler's `false` cancels it; otherwise, with no mouse
     *  mode and no scrollback (alternate buffer), it becomes arrow keys typed
     *  into the pty. */
    wheel(deltaY, deltaMode = 0) {
      const ev = { deltaY, deltaMode, prevented: false, preventDefault() { this.prevented = true; } };
      if (this.wheelHandler && this.wheelHandler(ev) === false) return ev;
      if (this.modes.mouseTrackingMode === "none" && this.buffer.active.type === "alternate") {
        const n = Math.trunc(Math.abs(deltaMode === 0 ? deltaY / 20 : deltaY));
        this.listener?.((deltaY < 0 ? "\x1b[A" : "\x1b[B").repeat(n));
      }
      return ev;
    }
  }
  const effects = [];
  const env = {
    useRef: (init) => ({ current: init }),
    useState: (init) => [typeof init === "function" ? init() : init, () => {}],
    useCallback: (fn) => fn,
    useEffect: (cb, deps) => { effects.push({ cb, deps }); },
    Terminal: TerminalStub,
    FitAddon: class { activate() {} dispose() {} fit() {} },
    SearchAddon: class { activate() {} dispose() {} onDidChangeResults() { return { dispose() {} }; } clearDecorations() {} findNext() {} findPrevious() {} },
    UnicodeGraphemesAddon: class { activate() {} dispose() {} },
    Channel: class { constructor() { this.onmessage = null; } },
    invoke: (cmd, args) => {
      invokes?.push({ cmd, args });
      return Promise.resolve(cmd === "term_open" ? 7 : null);
    },
    FindBar: () => null,
    findColors: () => ({ hit: "#000", on: "#fff" }),
    h: () => null, F: null,
    window: { addEventListener() {}, removeEventListener() {} },
    document: { addEventListener() {}, removeEventListener() {}, documentElement: {}, visibilityState: "visible", hasFocus: () => true },
    getComputedStyle: () => ({ getPropertyValue: () => "" }),
    performance: { now: () => Date.now() },
    requestAnimationFrame: (cb) => setTimeout(cb, 16),
    cancelAnimationFrame: (id) => clearTimeout(id),
    setTimeout, clearTimeout, TextEncoder,
  };
  const prevRO = globalThis.ResizeObserver;
  globalThis.ResizeObserver = class { observe() {} disconnect() {} };

  const built = build(env);
  const transport = () => {
    const tx = {
      open: () => Promise.resolve({ replay: 0, replayCols: null }),
      write: (data) => written.push(new TextDecoder().decode(new Uint8Array(data))),
      resize() {}, close() {},
    };
    if (withWheel) tx.wheel = (lines) => new Promise((resolve) => wheels.push({ lines, resolve }));
    return tx;
  };
  const api = built.useTerm(transport, "s", 0, 0);
  api.hostRef.current = host;
  const cleanups = effects.map(({ cb }) => cb()).filter((c) => typeof c === "function");
  return {
    built, term: () => term, wheels, written,
    dispose() { cleanups.forEach((c) => c()); globalThis.ResizeObserver = prevRO; },
  };
}

// 1. The routing rule, straight.
{
  const { built } = mount();
  const w = built.wheelToTmux;
  if (!w) fail("wheelToTmux not found — nothing decides which wheels belong to tmux");
  else {
    check(w("none", "alternate") === true, "no mouse, alternate screen (pi / a shell under tmux): tmux's wheel");
    check(w("any", "alternate") === false && w("vt200", "alternate") === false && w("drag", "alternate") === false,
      "mouse on (claude): xterm reports it, tmux is not asked");
    check(w("none", "normal") === false, "normal buffer (xterm has scrollback of its own): xterm's wheel");
  }
}

// 2. A main-screen pane: the wheel reaches the backend as a scroll, and NOT the
//    program as arrow keys.
{
  const m = mount();
  await tick();
  const ev = m.term().wheel(-100); // one notch up: 100px over 20px cells
  check(m.wheels.length === 1 && m.wheels[0].lines === -5, `wheel up → term_wheel(-5) (${JSON.stringify(m.wheels.map((w) => w.lines))})`);
  check(m.written.length === 0, `…and no ↑ reached the program (${JSON.stringify(m.written)})`);
  check(ev.prevented === true, "…and the page does not scroll with it");
  m.dispose();
}

// 3. The mouse on (claude): untouched — xterm keeps the event.
{
  const m = mount();
  await tick();
  m.term().modes.mouseTrackingMode = "any";
  const ev = m.term().wheel(-100);
  check(m.wheels.length === 0 && !ev.prevented, "mouse on: the handler lets xterm report the wheel itself");
  m.dispose();
}

// 4. A dock shell (no `wheel` on the transport): xterm's own scrollback.
{
  const m = mount({ withWheel: false });
  await tick();
  m.term().buffer.active.type = "normal";
  const ev = m.term().wheel(-100);
  check(!ev.prevented && m.wheels.length === 0, "dock shell: xterm keeps its own wheel");
  m.dispose();
}

// 5. A fling: one invoke in flight, the rest summed into the next — and a
//    reversal inside the window nets out.
{
  const m = mount();
  await tick();
  for (let i = 0; i < 12; i++) m.term().wheel(-10); // 12 × half a line
  check(m.wheels.length === 1, `fling: one invoke in flight (${m.wheels.length})`);
  m.wheels[0]?.resolve(); await tick(); await tick();
  const total = m.wheels.reduce((a, w) => a + w.lines, 0);
  check(m.wheels.length === 2 && total === -6, `…the rest arrive as ONE follow-up, nothing lost (${JSON.stringify(m.wheels.map((w) => w.lines))})`);
  m.wheels[1]?.resolve(); await tick(); await tick();
  m.term().wheel(-40); m.term().wheel(-40); m.term().wheel(40); // up 2 (in flight), then up 2 / down 2 queued
  m.wheels[2]?.resolve(); await tick(); await tick();
  check(m.wheels.length === 3, `up then down while busy: nets to nothing, no invoke (${JSON.stringify(m.wheels.map((w) => w.lines))})`);
  m.dispose();
}

// 6. Line and page deltas (a mouse on some systems, keyboard-driven scrolling).
{
  const w = mount().built.wheelLines;
  if (!w) fail("wheelLines not found");
  else {
    check(w(0, 3, 1, 20, 30).lines === 3, "DOM_DELTA_LINE: lines as given");
    check(w(0, -1, 2, 20, 30).lines === -30, "DOM_DELTA_PAGE: a page is the grid's rows");
    const a = w(0, 30, 0, 20, 30);
    const b = w(a.acc, 30, 0, 20, 30);
    check(a.lines === 1 && b.lines === 2, "pixel fractions carry to the next event");
  }
}

// 7. The real transport sends what lib.rs expects.
{
  const invokes = [];
  const { built, dispose } = mount({ invokes });
  const tx = built.tmuxTransport("valleos-todo-first~agent~pi");
  if (!tx.wheel) fail("tmuxTransport has no wheel");
  else {
    await tx.wheel(-3);
    check(!invokes.some((i) => i.cmd === "term_wheel"), "before the attach answers: nothing to scroll, no invoke");
    await tx.open(100, 20, {});
    await tx.wheel(-3);
    const w = invokes.find((i) => i.cmd === "term_wheel");
    check(w && w.args.id === 7 && w.args.lines === -3, `after it: term_wheel { id, lines } (${JSON.stringify(w?.args)})`);
  }
  dispose();
}

console.log(failed ? `\n${failed} failure(s)` : "\nall good");
process.exit(failed ? 1 : 0);
