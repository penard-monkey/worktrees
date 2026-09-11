// Deterministic check that a dock shell's REPLAY is never answered.
//
// `shell_open` on a live shell pushes the backend's ring — every byte the shell
// ever wrote — down the channel before anything live. It is a recording, but
// xterm parses it like output, and any terminal query in it is re-issued to a
// brand-new xterm on every re-attach. vim leaves exactly such a burst (two
// cursor-position reports, DA2, then colour and cursor-blink queries), and every
// `git commit` without `-m` runs vim. xterm answered each one down the pty as
// INPUT, and zsh echoed the printable tail onto the prompt as if typed:
// `2RR0;276;0c11;rgb:0f0f/0f0f/1616…`, on every place switch and dock re-open
// until 256K of later output rolled the query out (_tmp screenshot, 2026-09-11).
//
// The fix mutes `onData` while the replay chunk is being parsed. Two things
// make that precise, and this check pins both:
//
//   Which chunk is the replay. NOT "whatever arrived before `open` resolved":
//   Tauri hands a payload this size to the page through a separate fetch that
//   can land after the invoke's own response. The channel IS ordered, so the
//   replay is its first message whenever `open` reports one — and when the
//   first message beats `open`, it is presumed a replay.
//
//   When the mute lifts. xterm runs a write's callback synchronously once THAT
//   chunk is parsed, before the next one, so a live chunk queued behind the
//   replay is answered normally — a program the user left running in the tab
//   (vim itself, say) still gets its replies.
//
// No suite can see this. The mock harness models no queries and answers every
// invoke in a microtask; bats never reaches the frontend. Same shape as
// termresize-check.mjs: the REAL source of TerminalPane.tsx is evaluated under
// stubs, so running it before and after an edit tests the edit itself — and it
// fails on the version before the fix.
//
//   node termreplay-check.mjs [path/to/TerminalPane.tsx]   exits non-zero on failure
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const here = fileURLToPath(new URL(".", import.meta.url));
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
  return { useTerm };
}`;
const js = (await transformWithEsbuild(wrapped, "termreplay-check.tsx", {
  loader: "tsx", format: "esm", jsx: "transform", jsxFactory: "h", jsxFragment: "F",
})).code;
const { build } = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));
void here;

const tick = () => new Promise((r) => setTimeout(r, 0));
const enc = new TextEncoder();
const QUERY = "\x1b[>c";                 // DA2 request, as vim sends it
const REPLY = "\x1b[>0;276;0c";           // what xterm 5.5 answers

/** Mount useTerm against a transport whose `open` the test resolves by hand,
 *  and an xterm stub whose parsing the test drives chunk by chunk. */
function mount(replay) {
  const host = { clientWidth: 800, clientHeight: 600 };
  const sent = [];        // what reached the pty (tx.write)
  let channel = null;     // the Channel the component handed to `open`
  let resolveOpen = null;
  let term = null;

  class TerminalStub {
    constructor(opts) {
      this.options = { ...opts }; this.unicode = { activeVersion: "" };
      this.cols = 100; this.rows = 40; this.element = {};
      this.queue = []; this.listener = null;
      term = this;
    }
    loadAddon(a) { a.activate?.(this); }
    open() {} writeln() {} focus() {} dispose() {}
    resize(c, r) { this.cols = c; this.rows = r; }
    onData(fn) { this.listener = fn; }
    // Bytes are queued, as xterm's WriteBuffer queues them; nothing is parsed
    // until the test says so.
    write(bytes, cb) { this.queue.push({ bytes, cb }); }
    /** Parse the next queued chunk the way xterm would: a query in it fires a
     *  reply through onData DURING the parse, and the chunk's callback runs
     *  synchronously right after, before any later chunk. */
    parseNext() {
      const w = this.queue.shift();
      if (!w) throw new Error("parseNext: nothing queued");
      const text = new TextDecoder().decode(w.bytes);
      if (text.includes(QUERY)) this.listener?.(REPLY);
      w.cb?.();
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
    invoke: () => Promise.resolve(1),
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

  const transport = () => ({
    open: (_c, _r, ch) => { channel = ch; return new Promise((res) => { resolveOpen = res; }); },
    write: (data) => sent.push(new TextDecoder().decode(new Uint8Array(data))),
    resize() {}, close() {},
  });
  const { useTerm } = build(env);
  const api = useTerm(transport, "shell|1", 0, 0);
  api.hostRef.current = host;
  const cleanups = effects.map(({ cb }) => cb()).filter((c) => typeof c === "function");
  if (!channel) throw new Error("open was not called synchronously on mount");

  return {
    term: () => term,
    sent,
    deliver: (text) => channel.onmessage(enc.encode(text).buffer),
    // A pre-fix transport resolved with the bare generation; the fixed one with
    // `{ replay }`. Resolving with the object is what the real one now does.
    async resolveOpen() { resolveOpen({ replay }); await tick(); },
    dispose() { cleanups.forEach((c) => c()); globalThis.ResizeObserver = prevRO; },
  };
}

let failed = 0;
const fail = (msg) => { failed++; console.log(`not ok — ${msg}`); };
const ok = (msg) => console.log(`ok — ${msg}`);
const check = (cond, msg) => (cond ? ok(msg) : fail(msg));

const RING = `~/x (main) » vim notes.md\r\n${QUERY}\x1b[6n\r\n~/x (main) » `;

// 1. The replay lands BEFORE `open` resolves (small payload, eval path).
{
  const m = mount(RING.length);
  m.deliver(RING);
  await m.resolveOpen();
  m.term().parseNext();
  check(m.sent.length === 0, `replay delivered before open resolves: query in the ring is not answered (${m.sent.length} sent)`);
  m.deliver(`${QUERY}`);            // vim, live, asks again
  m.term().parseNext();
  check(m.sent.length === 1 && m.sent[0] === REPLY, "…and a live query after it IS answered");
  m.dispose();
}

// 2. The replay lands AFTER `open` resolves (large payload, fetch path).
{
  const m = mount(RING.length);
  await m.resolveOpen();
  m.deliver(RING);
  m.term().parseNext();
  check(m.sent.length === 0, `replay delivered after open resolves: still not answered (${m.sent.length} sent)`);
  m.deliver(`${QUERY}`);
  m.term().parseNext();
  check(m.sent.length === 1, "…and a live query after it IS answered");
  m.dispose();
}

// 3. A live chunk queued BEHIND the replay, parsed before anything else lands:
//    the mute lifts with the replay's own callback, not later.
{
  const m = mount(RING.length);
  m.deliver(RING);
  m.deliver(QUERY);
  await m.resolveOpen();
  m.term().parseNext();
  check(m.sent.length === 0, "queued behind the replay: the replay itself is silent");
  m.term().parseNext();
  check(m.sent.length === 1, "…and the live chunk right behind it is answered");
  m.dispose();
}

// 4. A FRESH shell (nothing replayed) whose first output arrives after `open`:
//    that output is live and its queries must be answered.
{
  const m = mount(0);
  await m.resolveOpen();
  m.deliver(QUERY);
  m.term().parseNext();
  check(m.sent.length === 1, "fresh shell, first chunk after open: a query in it is answered");
  m.dispose();
}

// 5. The tmux transport reports no replay; tmux's own attach-time queries are
//    real and want their answers.
{
  const m = mount(0);
  await m.resolveOpen();
  m.deliver("\x1b[c\x1b[>c\x1b]10;?\x1b\\");
  m.term().parseNext();
  check(m.sent.length === 1, "tmux attach burst (no replay): answered");
  m.dispose();
}

console.log(failed ? `\n${failed} failure(s)` : "\nall good");
process.exit(failed ? 1 : 0);
