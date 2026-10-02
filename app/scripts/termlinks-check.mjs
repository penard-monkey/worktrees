// Deterministic check for file-path links in the terminal (⌘-click → the dock's
// file viewer at the line).
//
// Three halves, all against the REAL source:
//
//   1. `findPaths` (termlinks.ts) over a fixture table — the forms agents and
//      tools print (relative, absolute, ~/, :line:col, (line,col), #L, ", line
//      N", " line N") and the false positives that must NOT become candidates
//      (URLs, version numbers, prose like "and/or", flags, directories).
//   2. `logicalLine` over stub xterm rows — a path WRAPPED across two rows is
//      one link, and a wide glyph before it does not shift its cells.
//   3. The link provider wired in TerminalPane.tsx's `useTerm`, evaluated under
//      stubs the way termwheel-check.mjs does it: only a candidate the resolver
//      says EXISTS becomes a link, the resolver is asked once per batch and then
//      cached (never once per mousemove), activation needs ⌘, and a ⌘-press on
//      a link is stopped before xterm's own mousedown listeners — which are
//      what would report the click to a mouse-mode program (claude) and start a
//      text selection.
//
// Run from the repo root, as CI does:
//   node app/scripts/termlinks-check.mjs [path/to/TerminalPane.tsx]   exits non-zero on failure
import fs from "node:fs";
import { transformWithEsbuild } from "vite";

const LINKS_SRC = new URL("../src/termlinks.ts", import.meta.url).pathname;
const PANE_SRC = process.argv[2] ?? new URL("../src/TerminalPane.tsx", import.meta.url).pathname;

let failed = 0;
const fail = (msg) => { failed++; console.log(`not ok — ${msg}`); };
const ok = (msg) => console.log(`ok — ${msg}`);
const check = (cond, msg) => (cond ? ok(msg) : fail(msg));
const tick = () => new Promise((r) => setTimeout(r, 0));

const linksJs = (await transformWithEsbuild(fs.readFileSync(LINKS_SRC, "utf8"), "termlinks.ts", { loader: "ts", format: "esm" })).code;
const L = await import("data:text/javascript;base64," + Buffer.from(linksJs).toString("base64"));

// ── 1. detection table ───────────────────────────────────────────────────────
// [text, expected: [path, line?, col?, clickable span]...]
const T = [
  ["crates/worktrees-core/src/ops.rs:1022", [["crates/worktrees-core/src/ops.rs", 1022, undefined, "crates/worktrees-core/src/ops.rs:1022"]]],
  ["  app/src/App.tsx", [["app/src/App.tsx"]]],
  ["error at src/foo.ts:42:7: bad", [["src/foo.ts", 42, 7, "src/foo.ts:42:7"]]],
  ["/Users/x/repo/.worktrees/a/f.rs:3", [["/Users/x/repo/.worktrees/a/f.rs", 3, undefined, "/Users/x/repo/.worktrees/a/f.rs:3"]]],
  ["see ~/notes/todo.md.", [["~/notes/todo.md"]]],
  ["src/a.ts(42,7): error TS2322", [["src/a.ts", 42, 7, "src/a.ts(42,7)"]]],
  ['  File "tools/x.py", line 88, in main', [["tools/x.py", 88, undefined, 'tools/x.py", line 88']]],
  ["README.md line 12", [["README.md", 12, undefined, "README.md line 12"]]],
  ["src/a.ts#L5", [["src/a.ts", 5, undefined, "src/a.ts#L5"]]],
  ["(see CLAUDE.md)", [["CLAUDE.md"]]],
  ["./bin/worktrees and ../x/y", [["./bin/worktrees"], ["../x/y"]]],
  ["test/helpers/common", [["test/helpers/common"]]],
  ["`.gitignore` and .nvmrc", [[".gitignore"], [".nvmrc"]]],
  ["M  app/src/termlinks.ts", [["app/src/termlinks.ts"]]],
  ["diff --git a/src/x.rs b/src/x.rs", [["a/src/x.rs"], ["b/src/x.rs"]]],
  ["node_modules/@xterm/xterm/src/browser/Linkifier.ts:229", [["node_modules/@xterm/xterm/src/browser/Linkifier.ts", 229, undefined, "node_modules/@xterm/xterm/src/browser/Linkifier.ts:229"]]],
  ["café/ñandú.md", [["café/ñandú.md"]]],
  // False positives — none of these may produce a candidate.
  ["https://github.com/x/y/blob/main/src/a.ts", []],
  ["open http://localhost:1420/src/App.css now", []],
  ["file:///etc/hosts", []],
  ["v0.33.0 and 1.2.3 and 3.5", []],
  ["client/server and/or read/write", []],
  ["src/", []],
  ["// comment", []],
  ["user@host.com pkg@1.2", []],
  ["--flag -- 12:30", []],
  ["e.g. 10:42", [["e.g"]]], // plausible by shape; the RESOLVER rejects it (no such file)
];
for (const [text, want] of T) {
  const got = L.findPaths(text);
  const shape = got.map((h) => [h.path, h.line, h.col, text.slice(h.start, h.end)]);
  const exp = want.map(([p, l, c, span]) => [p, l, c, span ?? p]);
  check(JSON.stringify(shape) === JSON.stringify(exp), `findPaths(${JSON.stringify(text)}) → ${JSON.stringify(shape)}${JSON.stringify(shape) === JSON.stringify(exp) ? "" : ` (want ${JSON.stringify(exp)})`}`);
}
check(L.findPaths("x.ts:0").every((h) => h.line === undefined), "line 0 is not a position");
check(L.findPaths("x.ts:99999999999").every((h) => h.line === undefined), "an absurd line is not a position");

// ── 2. wrapped rows and wide glyphs ──────────────────────────────────────────
/** Stub rows from strings: each char one cell, except `W` markers in `wide`
 *  which become a 2-cell glyph followed by a width-0 continuation. */
function rows(lines, wrapped) {
  return lines.map((s, i) => {
    const cells = [];
    for (const ch of Array.from(s)) {
      const wide = /\p{Extended_Pictographic}/u.test(ch);
      cells.push({ getChars: () => ch, getWidth: () => (wide ? 2 : 1) });
      if (wide) cells.push({ getChars: () => "", getWidth: () => 0 });
    }
    return { isWrapped: !!wrapped[i], length: cells.length, getCell: (x) => cells[x] };
  });
}
{
  const R = rows(["see crates/worktrees-c", "ore/src/ops.rs:1022 ok"], [false, true]);
  const ll = L.logicalLine((y) => R[y], 1);
  check(ll.text === "see crates/worktrees-core/src/ops.rs:1022 ok", `wrapped rows read as one line (${JSON.stringify(ll.text)})`);
  const h = L.findPaths(ll.text)[0];
  const r = h && L.hitRange(h, ll.cells);
  check(r && r.start.x === 5 && r.start.y === 1 && r.end.y === 2 && r.end.x === 19,
    `a path wrapped across two rows is ONE link spanning both (${JSON.stringify(r)})`);
  const same = L.logicalLine((y) => R[y], 0);
  check(same.first === 0 && same.last === 1, "asking from the FIRST row finds the continuation too");
}
{
  const R = rows(["✅ src/a.ts:3"], [false]);
  const ll = L.logicalLine((y) => R[y], 0);
  const h = L.findPaths(ll.text)[0];
  const r = h && L.hitRange(h, ll.cells);
  // ✅ is cells 1-2 (1-based), the space cell 3, so the path starts at cell 4.
  check(r && r.start.x === 4 && r.end.x === 13, `a wide glyph before the path does not shift it (${JSON.stringify(r)})`);
}

// ── 3. the provider in TerminalPane ──────────────────────────────────────────
const raw = fs.readFileSync(PANE_SRC, "utf8");
const body = raw
  .split("\n")
  .filter((l) => !/^\s*import\s/.test(l))
  .join("\n")
  .replace(/^export (function|const|type|class) /gm, "$1 ");
if (!body.includes("function useTerm")) throw new Error("useTerm not found in " + PANE_SRC);
const wrapped = `
export function build(env: any) {
  const {
    useCallback, useEffect, useRef, useState,
    Terminal, FitAddon, SearchAddon, UnicodeGraphemesAddon, Channel, invoke,
    FindBar, findColors,
    findPaths, logicalLine, hitRange,
    window, document, getComputedStyle, performance, navigator,
    requestAnimationFrame, cancelAnimationFrame, setTimeout, clearTimeout,
    TextEncoder,
    h, F,
  } = env;
${body}
  const pick = (name: string) => { try { return eval(name); } catch { return null; } };
  return { useTerm, tmuxTransport, termLinkProvider: pick("termLinkProvider") };
}`;
const js = (await transformWithEsbuild(wrapped, "termlinks-check.tsx", {
  loader: "tsx", format: "esm", jsx: "transform", jsxFactory: "h", jsxFragment: "F",
})).code;
const { build } = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

/** One evaluation of TerminalPane.tsx with a stub xterm whose buffer is `lines`
 *  and an `invoke` whose `resolve_term_paths` answers from `exists`. */
function harness(lines, exists) {
  const calls = [];
  const listeners = [];
  const screen = {
    addEventListener: (type, fn, opts) => listeners.push({ type, fn, opts }),
    // 80×24 cells of 10×10px, for `linkAt`'s own hit-test.
    getBoundingClientRect: () => ({ left: 0, top: 0, width: 800, height: 240 }),
    removeEventListener: () => {},
  };
  const host = { clientWidth: 800, clientHeight: 400, title: "" };
  const R = rows(lines, lines.map(() => false));
  let provider = null;
  class Terminal {
    constructor() {
      this.cols = 80; this.rows = 24; this.options = {};
      this.modes = { mouseTrackingMode: "none" };
      this.buffer = { active: { type: "alternate", viewportY: 0, getLine: (y) => R[y] } };
      this.unicode = {};
      this.element = { querySelector: (sel) => (sel === ".xterm-screen" ? screen : null) };
    }
    loadAddon() {} open() {} focus() {} write() {} writeln() {} resize() {} dispose() {}
    onData() { return { dispose() {} }; }
    attachCustomWheelEventHandler() {}
    registerLinkProvider(p) { provider = p; return { dispose() {} }; }
  }
  const invoke = async (cmd, args) => {
    if (cmd === "resolve_term_paths") {
      calls.push(args);
      return args.paths.map((p) => exists[p] ?? null);
    }
    if (cmd === "term_open") return 1;
    return undefined;
  };
  const effects = [];
  const refs = [];
  let ri = 0;
  const env = {
    useCallback: (f) => f,
    useEffect: (f) => effects.push(f),
    useRef: (v) => (refs[ri] ??= { current: v }, refs[ri++]),
    useState: (v) => [v, () => {}],
    Terminal, FitAddon: class { fit() {} }, SearchAddon: class {}, UnicodeGraphemesAddon: class {},
    Channel: class { onmessage = null; }, invoke,
    FindBar: () => null, findColors: () => ({}),
    findPaths: L.findPaths, logicalLine: L.logicalLine, hitRange: L.hitRange,
    window: { addEventListener() {} }, document: { documentElement: {}, visibilityState: "visible", hasFocus: () => true, addEventListener() {} },
    getComputedStyle: () => ({ getPropertyValue: () => "" }),
    performance: { now: () => 0 }, navigator: { platform: "MacIntel" },
    requestAnimationFrame: () => 0, cancelAnimationFrame() {}, setTimeout, clearTimeout,
    TextEncoder, h: () => null, F: null,
  };
  globalThis.ResizeObserver = class { observe() {} disconnect() {} };
  const mod = build(env);
  return { mod, env, calls, listeners, host, effects, get provider() { return provider; }, refs };
}

{
  const opened = [];
  const H = harness(
    ["  M src/real.ts:12:3 and src/ghost.ts and and/or", "README.md"],
    { "src/real.ts": "/repo/src/real.ts", "README.md": "/repo/README.md" },
  );
  if (!H.mod.termLinkProvider) {
    fail("TerminalPane.tsx has no `termLinkProvider` — the provider is not wired");
  } else {
    const links = { root: "/repo", onOpen: (p, line, col) => opened.push([p, line, col]) };
    const term = new (H.env.Terminal)();
    const prov = H.mod.termLinkProvider(term, { current: links }, H.host);
    const ask = (y) => new Promise((r) => prov.provideLinks(y, (ls) => r(ls ?? [])));
    const got = await ask(1);
    check(got.length === 1 && got[0].text === "src/real.ts:12:3",
      `only the path that EXISTS is a link — ghost.ts and and/or are not (${JSON.stringify(got.map((l) => l.text))})`);
    check(H.calls.length === 1 && H.calls[0].paths.length === 2,
      `one batched resolve for the row's candidates (${JSON.stringify(H.calls.map((c) => c.paths))})`);
    await ask(1); await ask(1);
    check(H.calls.length === 1, `hovering the same row again is answered from the cache (${H.calls.length} resolves)`);
    await ask(2);
    check(H.calls.length === 2, "a new row with a new candidate asks once more");
    const link = got[0];
    link.activate({ metaKey: false, ctrlKey: false }, link.text);
    check(opened.length === 0, "a PLAIN click does not open — it is the terminal's (focus, selection, claude's mouse)");
    link.activate({ metaKey: true, ctrlKey: false }, link.text);
    check(JSON.stringify(opened) === JSON.stringify([["/repo/src/real.ts", 12, 3]]), `⌘-click opens the resolved path at line:col (${JSON.stringify(opened)})`);
    check(link.range.start.x === 5 && link.range.end.x === 20 && link.range.start.y === 1, `the range covers "src/real.ts:12:3" (${JSON.stringify(link.range)})`);

    // The ⌘-press must not reach xterm's own mousedown listeners. Those sit on
    // `.xterm` (the parent): one reports the press to a mouse-mode program, the
    // other starts a selection. Stopping propagation at `.xterm-screen` — the
    // linkifier's own element — leaves the linkifier's listener running.
    const down = H.listeners.find((l) => l.type === "mousedown");
    check(!!down, "a mousedown listener is installed on .xterm-screen");
    if (down) {
      const ev = (meta) => ({ metaKey: meta, ctrlKey: false, stopped: false, prevented: false,
        stopPropagation() { this.stopped = true; }, preventDefault() { this.prevented = true; } });
      link.hover?.({}, link.text);
      const plain = ev(false); down.fn(plain);
      check(!plain.stopped, "a plain press on a link still reaches xterm (selection, claude's mouse)");
      const meta = ev(true); down.fn(meta);
      check(meta.stopped, "a ⌘-press on a link is stopped before xterm reports it or starts a selection");
      link.leave?.({}, link.text);
      const off = ev(true); down.fn(off);
      check(!off.stopped, "a ⌘-press with no link under the pointer is left alone");
      check(H.host.title === "", "leaving the link clears its hint");
    }
  }
}

// Right-click on a link: the app's own menu, and the press is the link's — a
// mouse-mode program must not hear it. Off a link, nothing changes.
{
  const menus = [];
  const H = harness(["  M src/real.ts:12:3"], { "src/real.ts": "/repo/src/real.ts" });
  if (H.mod.termLinkProvider) {
    let openedMenuCase = 0;
    const links = { root: "/repo", session: "cdv-x", onOpen() { openedMenuCase++; }, onMenu: (m) => menus.push(m) };
    const term = new (H.env.Terminal)();
    const prov = H.mod.termLinkProvider(term, { current: links }, H.host);
    const [link] = await new Promise((r) => prov.provideLinks(1, (ls) => r(ls ?? [])));
    const on = (type) => H.listeners.find((l) => l.type === type);
    const ev = (o) => ({ metaKey: false, ctrlKey: false, button: 0, clientX: 40, clientY: 50, stopped: false, prevented: false,
      stopPropagation() { this.stopped = true; }, preventDefault() { this.prevented = true; }, ...o });
    check(!!on("contextmenu"), "a contextmenu listener is installed on .xterm-screen");
    check(H.calls[0]?.session === "cdv-x", `a tmux pane's session reaches the resolver, for its live cwd (${JSON.stringify(H.calls[0])})`);
    if (on("contextmenu") && link) {
      const offDown = ev({ button: 2 }); on("mousedown").fn(offDown);
      const offCtx = ev({ button: 2 }); on("contextmenu").fn(offCtx);
      check(!offDown.stopped && !offCtx.stopped && !offCtx.prevented && menus.length === 0,
        "a right-click OFF a link is untouched — it reaches xterm and the program, and opens no menu");
      link.hover?.({}, link.text);
      const down = ev({ button: 2 }); on("mousedown").fn(down);
      check(down.stopped, "a right-press ON a link is stopped before xterm reports it to a mouse-mode program");
      // ⌘ held on a RIGHT press is still the menu, never an open: xterm's
      // linkifier activates on any button's release, so this is the case
      // that would otherwise open the file underneath its own menu.
      const before = menus.length;
      const mdown = ev({ button: 2, metaKey: true }); on("mousedown").fn(mdown);
      const mctx = ev({ button: 2, metaKey: true }); on("contextmenu").fn(mctx);
      link.activate({ button: 2, metaKey: true, ctrlKey: false }, link.text);
      check(mdown.stopped && mctx.prevented && menus.length === before + 1 && openedMenuCase === 0,
        `⌘-RIGHT-click on a link opens the MENU and never the file (menus +${menus.length - before}, opened ${openedMenuCase})`);
      menus.length = before;
      const cdown = ev({ button: 0, ctrlKey: true }); on("mousedown").fn(cdown);
      check(cdown.stopped, "so is a macOS Ctrl-click on a link (it IS the right-click there)");
      const ctx = ev({ button: 2, clientX: 41, clientY: 52 }); on("contextmenu").fn(ctx);
      check(ctx.prevented && ctx.stopped, "the contextmenu on a link is the app's, not WebKit's");
      check(menus.length === 1 && menus[0].path === "/repo/src/real.ts" && menus[0].line === 12 && menus[0].col === 3 && menus[0].x === 41 && menus[0].y === 52,
        `the menu gets the resolved path, line:col and the pointer (${JSON.stringify(menus[0])})`);
      let opened = 0;
      links.onOpen = () => opened++;
      link.activate({ metaKey: true, ctrlKey: false, button: 2 }, link.text);
      check(opened === 0, "releasing a RIGHT button on a link never opens it, ⌘ held or not");

      // No `hover` (xterm skips re-asking for the cell it saw last, even after
      // a mouseleave): the provider finds the link under the event itself.
      // Row 0 is "  M src/real.ts:12:3", the link spanning cols 4..19.
      link.leave?.({}, link.text);
      menus.length = 0;
      const ctx2 = ev({ button: 2, clientX: 65, clientY: 5 }); on("contextmenu").fn(ctx2);
      check(ctx2.prevented && menus.length === 1 && menus[0].path === "/repo/src/real.ts",
        `with no hover, a right-click on the link still gets the app's menu (${JSON.stringify(menus)})`);
      const ctx3 = ev({ button: 2, clientX: 15, clientY: 5 }); on("contextmenu").fn(ctx3);
      check(!ctx3.prevented && menus.length === 1, "…and one beside it still does not");
      const cmd = ev({ metaKey: true, clientX: 65, clientY: 5 }); on("mousedown").fn(cmd);
      check(cmd.stopped && opened === 1, `with no hover, a ⌘-press on the link opens it here rather than being swallowed (${opened})`);
    }
  }
}

// And `useTerm` registers it — for a pane given links, and not for one without.
for (const withLinks of [true, false]) {
  const H = harness(["x"], {});
  let registered = 0;
  H.env.Terminal.prototype.registerLinkProvider = function () { registered++; return { dispose() {} }; };
  const host = { clientWidth: 800, clientHeight: 400 };
  H.mod.useTerm(() => H.mod.tmuxTransport("s"), "k", 0, 0, true,
    withLinks ? { root: "/repo", onOpen() {} } : undefined);
  H.refs[0].current = host; // hostRef
  H.effects[0]();
  check(registered === (withLinks ? 1 : 0), `useTerm ${withLinks ? "registers" : "does not register"} a link provider ${withLinks ? "when given links" : "without them"} (${registered})`);
}

if (failed) {
  console.log(`\n${failed} failed`);
  process.exit(1);
}
console.log("\nall ok");
