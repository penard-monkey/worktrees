// Deterministic check of CtxMenu's viewport clamp — specifically that it
// re-runs when the menu RESIZES, not only when the cursor coords change.
//
// The bug this locks down: `x`/`y` are frozen for a menu's whole life, so a
// clamp keyed `[x, y]` never re-runs. A place menu whose "Remove worktree…"
// armed into two buttons grew by a row, and a menu already clamped flush to the
// bottom pushed its new last row off the screen — unreachable, with no
// scrollbar to say so. (_tmp screenshot, 2026-08-17.)
//
// Like race-check.mjs, it does NOT paraphrase the component: it evaluates the
// REAL source text of CtxMenu.tsx under React/DOM stubs, so running it before
// and after an edit tests the edit itself.
//
//   node ctxmenu-check.mjs [path/to/CtxMenu.tsx]      exits non-zero on failure
import fs from "node:fs";
import { fileURLToPath } from "node:url";
// vite's esbuild re-export — see race-check.mjs: bare "esbuild" does not resolve
// under pnpm's strict layout, vite does (direct dependency).
import { transformWithEsbuild } from "vite";

const SRC = process.argv[2] || fileURLToPath(new URL("../src/CtxMenu.tsx", import.meta.url));
const raw = fs.readFileSync(SRC, "utf8");

// Drop the react import (the hooks arrive through `env`) and un-export the
// component so it can be returned from the factory. Everything else is verbatim.
const body = raw
  .split("\n")
  .filter((l) => !/^\s*import\s.*from\s+"react"/.test(l))
  .join("\n")
  .replace(/export function CtxMenu/, "function CtxMenu");
if (!body.includes("function CtxMenu")) throw new Error("CtxMenu not found in " + SRC);

const wrapped = `
export function build(env: any) {
  const { useEffect, useLayoutEffect, useRef, useState, h, F } = env;
${body}
  return { CtxMenu };
}`;
const js = (await transformWithEsbuild(wrapped, "ctxmenu-check.tsx", {
  loader: "tsx", format: "esm", jsx: "transform", jsxFactory: "h", jsxFragment: "F",
})).code;
const { build } = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

// ── DOM / React stubs ────────────────────────────────────────────────────────
/** One mount of CtxMenu at (x, y) in a `vw`×`vh` viewport, starting `w`×`h`. */
function mount({ x, y, vw, vh, w, h: h0 }) {
  // Both measurement APIs, agreeing — the check is about WHEN the clamp runs,
  // not which probe it uses, and a stub that only answers one of them turns a
  // failed assertion into a TypeError (which reads like a broken harness).
  const el = {
    offsetWidth: w, offsetHeight: h0,
    getBoundingClientRect() { return { width: this.offsetWidth, height: this.offsetHeight }; },
  };
  let pos = { left: x, top: y };
  const observers = [];          // ResizeObserver callbacks watching `el`
  const winListeners = {};
  const cleanups = [];

  const prevRO = globalThis.ResizeObserver;
  const prevWin = globalThis.window;
  globalThis.ResizeObserver = class {
    constructor(cb) { this.cb = cb; }
    observe(target) { if (target === el) observers.push(this.cb); }
    disconnect() { const i = observers.indexOf(this.cb); if (i >= 0) observers.splice(i, 1); }
  };
  // Mutated in place, never reassigned: the component's clamp reads
  // `window.innerWidth` at call time, so shrinking the viewport has to happen on
  // the object it will look up.
  const win = {
    innerWidth: vw,
    innerHeight: vh,
    addEventListener: (t, fn) => { (winListeners[t] ||= []).push(fn); },
    removeEventListener: (t, fn) => {
      const a = winListeners[t]; if (!a) return;
      const i = a.indexOf(fn); if (i >= 0) a.splice(i, 1);
    },
  };
  globalThis.window = win;

  const env = {
    useRef: () => ({ current: el }),
    // The component's only state IS the clamped position; capturing the setter
    // is how the harness reads what the clamp decided.
    useState: (init) => [pos, (next) => { pos = typeof next === "function" ? next(pos) : next; }],
    useLayoutEffect: (fn) => { const c = fn(); if (c) cleanups.push(c); },
    useEffect: (fn) => { const c = fn(); if (c) cleanups.push(c); },
    h: (...a) => ({ tag: a[0] }),
    F: "fragment",
  };
  const { CtxMenu } = build(env);
  CtxMenu({ x, y, onClose: () => {}, children: null });

  return {
    el,
    pos: () => pos,
    /** Grow/shrink the menu the way arming an item does, then let the observer fire. */
    resize(nw, nh) {
      el.offsetWidth = nw;
      el.offsetHeight = nh;
      for (const cb of [...observers]) cb([{ target: el }]);
    },
    /** The window shrinking under an open menu — the other way the clamp's
     *  answer goes stale without the cursor moving. Fires the real listener the
     *  component registered, so removing that registration fails a test. */
    shrinkWindow(nvw, nvh) {
      win.innerWidth = nvw;
      win.innerHeight = nvh;
      for (const fn of [...(winListeners.resize ?? [])]) fn();
    },
    observed: () => observers.length,
    listeners: (t) => (winListeners[t] ?? []).length,
    restore() {
      for (const c of cleanups) c();
      globalThis.ResizeObserver = prevRO;
      globalThis.window = prevWin;
    },
  };
}

// ── assertions ───────────────────────────────────────────────────────────────
let failed = 0;
const ok = (name, cond, detail = "") => {
  console.log(`${cond ? "  ok  " : "FAIL  "}${name}${detail && !cond ? " — " + detail : ""}`);
  if (!cond) failed++;
};
/** The invariant the bug broke: the whole menu is inside the viewport. */
const fits = (m, vw, vh) => {
  const { left, top } = m.pos();
  return left >= 4 && top >= 4
    && left + m.el.offsetWidth <= vw - 4
    && top + m.el.offsetHeight <= vh - 4;
};

console.log(`[${SRC.split("/").pop()}] ${raw.split("\n").length}L`);

// 1 — the plain case: a menu that fits opens exactly at the cursor.
{
  const m = mount({ x: 300, y: 200, vw: 1440, vh: 900, w: 260, h: 300 });
  ok("opens at the cursor when it fits", m.pos().left === 300 && m.pos().top === 200,
    JSON.stringify(m.pos()));
  m.restore();
}

// 2 — opened near the bottom edge: clamped up so the last item is reachable.
{
  const m = mount({ x: 200, y: 1000, vw: 1440, vh: 1080, w: 260, h: 470 });
  ok("clamps a bottom-edge menu into the viewport", fits(m, 1440, 1080), JSON.stringify(m.pos()));
  m.restore();
}

// 3 — THE REGRESSION. Menu opens clamped flush to the bottom, then grows by one
//     row (Remove worktree… → two armed buttons). Without a resize-driven clamp
//     the extra row lands off-screen: top stays at the old height's answer.
{
  const VW = 478, VH = 1080;                    // the screenshot's window
  const m = mount({ x: 240, y: 610, vw: VW, vh: VH, w: 236, h: 470 });
  const before = m.pos().top;
  m.resize(236, 470 + 28);                      // one .pop-item taller
  ok("re-clamps when the menu grows after opening", fits(m, VW, VH),
    `top ${before} → ${m.pos().top}, bottom ${m.pos().top + m.el.offsetHeight} > ${VH - 4}`);
  ok("the grown menu is observed at all", m.observed() > 0, "no ResizeObserver on the menu element");
  m.restore();
}

// 4 — growth away from the edge must NOT drag the menu around: a menu with room
//     below it stays where the cursor put it.
{
  const m = mount({ x: 100, y: 100, vw: 1440, vh: 1080, w: 260, h: 300 });
  m.resize(260, 340);
  ok("a menu with room below does not move when it grows",
    m.pos().left === 100 && m.pos().top === 100, JSON.stringify(m.pos()));
  m.restore();
}

// 5 — the WINDOW shrinking under an open menu: same staleness, other direction.
//     The menu fits when it opens, then the window gets shorter than its bottom.
{
  const m = mount({ x: 200, y: 500, vw: 1440, vh: 900, w: 260, h: 380 });
  ok("registers a window resize listener", m.listeners("resize") > 0, "none registered");
  m.shrinkWindow(700, 600);                       // 500 + 380 = 880 > 600
  ok("re-clamps when the window shrinks under it", fits(m, 700, 600),
    `${JSON.stringify(m.pos())}, bottom ${m.pos().top + m.el.offsetHeight} > ${600 - 4}`);
  m.restore();
}

// 6 — teardown must not leak: BOTH the observer and the window listener.
{
  const m = mount({ x: 10, y: 10, vw: 1440, vh: 900, w: 260, h: 200 });
  const hadListener = m.listeners("resize") > 0;
  m.restore();
  ok("disconnects its observer on unmount", m.observed() === 0, `${m.observed()} left`);
  ok("removes its window listener on unmount", hadListener && m.listeners("resize") === 0,
    `${m.listeners("resize")} left`);
}

console.log(failed ? `\n${failed} FAILED` : "\nall ok");
process.exit(failed ? 1 : 0);
