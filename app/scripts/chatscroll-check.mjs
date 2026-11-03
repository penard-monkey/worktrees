// chatscroll-check — the chat's scroll (docs/proposals/chat-session-view.md
// §7): at the bottom it follows whatever arrives, scrolled up it holds the
// row being read still, and a reply below a reader who scrolled up is
// offered as "New messages".
//
// The bug it pins (2026-11-02): "when I get a response in chat view, the view
// jumps up to roughly 3 messages ago". Measured in WebKit
// (chat-scroll-browser.mjs): a send slid `agent_turns`' 20-turn window, and
// scrollTop went 4129 → 3226 with the reader at the bottom, ending 805 px up.
// Three things made it:
//   - `itemsOf` keyed rows by window INDEX, so a slide rewrote every row in
//     place, and the browser's scroll anchoring held a node that now said
//     something else;
//   - anchoring moves scrollTop, and the scroll event that move fires is
//     dispatched BEFORE the ResizeObserver's follow, so the hook took it for
//     the reader scrolling up;
//   - the hook followed only when the item COUNT grew, and a slide (or a
//     pending message settling) does not grow it.
//
// It runs the REAL hook (app/src/stickToBottom.ts) and the REAL `itemsOf`
// and `settlePending` (app/src/chat.ts) under a fake React and a fake DOM.
// The fake DOM does what WebKit was measured to do (chat-scroll-browser.mjs,
// and a probe that wrapped the scroller's scrollTop setter):
//   - a commit that rewrites rows is laid out ONCE with those rows torn down
//     and the new ones not yet in, and scrollTop is clamped to that shorter
//     column; no script writes it (4143 → 2010 with every row rewritten,
//     4143 → 3836 with stable keys; Chromium only clamps to the final height);
//   - native scroll anchoring on the first row in view (Playwright WebKit
//     27.2, Chromium 156 and the system WKWebView on macOS 27.0 all do it),
//     unless ChatPane.css gives `.chat-scroll` `overflow-anchor: none`, and
//     not after that clamp (the rows' offsets moved with it);
//   - every scroll event dispatched at the next frame, before that frame's
//     ResizeObserver delivery (the HTML event loop's order).
// And something in the tree forces a layout at the end of every commit (the
// composer's `fit` measures its field in a layout effect), because the hook
// must hold whether or not anything does.
//
//   node chatscroll-check.mjs        exits non-zero on failure
// CHATSCROLL_HOOK (a stickToBottom.ts, or a ChatPane.tsx from before the fix,
// whose `useStickToBottom` is sliced out), CHATSCROLL_CHAT (chat.ts),
// CHATSCROLL_CSS (ChatPane.css) and CHATSCROLL_PANE (ChatPane.tsx) point it at
// other copies, to show it red.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const here = (p) => path.join(path.dirname(fileURLToPath(import.meta.url)), p);
const HOOK = process.env.CHATSCROLL_HOOK ?? here("../src/stickToBottom.ts");
const CHAT = process.env.CHATSCROLL_CHAT ?? here("../src/chat.ts");
const CSS = process.env.CHATSCROLL_CSS ?? here("../src/ChatPane.css");
const PANE = process.env.CHATSCROLL_PANE ?? here("../src/ChatPane.tsx");

let failed = 0;
const check = (cond, msg) => { if (cond) console.log(`ok   ${msg}`); else { failed++; console.log(`FAIL ${msg}`); } };

// ── the real sources ────────────────────────────────────────────────────────

const chatSrc = fs.readFileSync(CHAT, "utf8")
  .replace(/^import \{ useSyncExternalStore \} from "react";$/m, "const useSyncExternalStore = (_s, snap) => snap();")
  .replace(/^import \{ invoke, windowOnHost \} from "\.\/ipc";$/m, "const invoke = () => { throw new Error('no ipc here'); }; const windowOnHost = () => true;")
  .replace(/^import \{ convertPhoto, jpegName, photoDecision, photoKind, INPUT_MAX \} from "\.\/chat-image";$/m, "const INPUT_MAX = 1; const convertPhoto = () => undefined; const jpegName = () => undefined; const photoDecision = () => undefined; const photoKind = () => undefined;");
if (/^import /m.test(chatSrc)) throw new Error("chat.ts gained an import this check does not stub — extend the stubs above");
const C = await import("data:text/javascript;base64," + Buffer.from((await transformWithEsbuild(chatSrc, "chat.ts", { loader: "ts", format: "esm" })).code).toString("base64"));

const hookRaw = fs.readFileSync(HOOK, "utf8");
// Before the fix the hook lived inside ChatPane.tsx: slice it out alone.
let hookBody;
if (/from "\.\/chat"/.test(hookRaw)) {
  const start = hookRaw.indexOf("function useStickToBottom(");
  if (start < 0) throw new Error(`no useStickToBottom in ${HOOK}`);
  let i = hookRaw.indexOf(") {", start) + 2, depth = 0;
  for (; i < hookRaw.length; i++) {
    if (hookRaw[i] === "{") depth++;
    else if (hookRaw[i] === "}" && --depth === 0) break;
  }
  hookBody = hookRaw.slice(start, i + 1);
} else {
  hookBody = hookRaw.split("\n").filter((l) => !/^\s*import\s/.test(l)).join("\n").replace(/^export (function|const|type|class) /gm, "$1 ");
}
const OLD_SIGNATURE = /function useStickToBottom\([^)]*count: number/.test(hookBody);
const hookJs = (await transformWithEsbuild(`
export function build(env) {
  const { useEffect, useLayoutEffect, useRef, useState, useCallback, Component, ResizeObserver } = env;
${hookBody}
  return { useStickToBottom, BeforeCommit: typeof BeforeCommit === "undefined" ? null : BeforeCommit };
}`, "chatscroll-check.ts", { loader: "ts", format: "esm" })).code;
const { build } = await import("data:text/javascript;base64," + Buffer.from(hookJs).toString("base64"));

const cssRule = /\.chat-scroll\s*\{([^}]*)\}/.exec(fs.readFileSync(CSS, "utf8"))?.[1] ?? "";
const NATIVE_ANCHORING = !/overflow-anchor:\s*none/.test(cssRule);
console.log(`(hook: ${path.basename(HOOK)}${OLD_SIGNATURE ? ", the pre-fix signature" : ""}; native anchoring ${NATIVE_ANCHORING ? "ON" : "off"} per ${path.basename(CSS)})`);

// ── a fake React: one component, hooks by call order ────────────────────────

function makeReact() {
  const slots = [];
  let i = 0, layoutQ = [], passiveQ = [], dirty = false;
  const effect = (q) => (fn, deps) => {
    const k = i++;
    const prev = slots[k];
    const changed = !prev || !deps || !prev.deps || deps.length !== prev.deps.length || deps.some((d, j) => !Object.is(d, prev.deps[j]));
    if (!prev) slots[k] = { deps, cleanup: undefined };
    if (changed) q.push(() => { slots[k].cleanup?.(); slots[k].deps = deps; const c = fn(); slots[k].cleanup = typeof c === "function" ? c : undefined; });
  };
  const env = {
    useRef(v) { const k = i++; if (!slots[k]) slots[k] = { current: v }; return slots[k]; },
    useState(v) {
      const k = i++;
      if (!slots[k]) slots[k] = { v };
      const s = slots[k];
      return [s.v, (nv) => { const x = typeof nv === "function" ? nv(s.v) : nv; if (!Object.is(x, s.v)) { s.v = x; dirty = true; } }];
    },
    useCallback: (fn) => fn,
    useLayoutEffect: effect({ push: (f) => layoutQ.push(f) }),
    useEffect: effect({ push: (f) => passiveQ.push(f) }),
    Component: class { constructor(props) { this.props = props; } },
  };
  return {
    env,
    render(fn) { i = 0; layoutQ = []; passiveQ = []; dirty = false; return fn(); },
    runLayout() { for (const f of layoutQ) f(); },
    runPassive() { for (const f of passiveQ) f(); },
    get dirty() { return dirty; },
  };
}

// ── a fake DOM: one scroller, its column, the list's rows ──────────────────

const TOP = 100;        // the scroller's top in the window: geometry must be relative
const PAD = 16, GAP = 8, HEADER = 40, TYPING = 30;

/** What a row SAYS, which is what a reader reads (not its key). */
const sayOf = (it) => it.k === "sep" ? `sep ${it.at}` : it.k === "pending" ? `pending: ${it.p.text}` : it.k === "user" ? `you: ${it.text}` : it.k === "reply" ? `agent: ${it.text}` : it.k;
const heightOf = (it) => it.k === "sep" ? 20 : it.k === "earlier" ? 24
  : 30 + 18 * Math.ceil(Math.max(1, (it.text ?? it.p?.text ?? "").length) / 64) + (it.calls?.length ? 26 : 0);

function makeWorld({ ch = 600 } = {}) {
  const w = { st: 0, ch, layoutDirty: true, pendingScroll: false, listeners: new Set(), observers: [], painted: [], anchor: null };
  const block = (tag, extra = {}) => Object.assign({ tagName: tag, isConnected: true, y: 0, h: 0,
    getBoundingClientRect() { w.layout(); return { top: TOP + this.y - w.st, bottom: TOP + this.y + this.h - w.st, height: this.h }; },
    get offsetHeight() { w.layout(); return this.h; } }, extra);
  w.header = block("P", { say: "header", h: HEADER });
  w.list = block("OL", { children: [] });
  w.typing = block("P", { say: "typing", h: TYPING });
  w.card = null;
  w.showTyping = false;
  w.col = block("DIV");
  Object.defineProperty(w.col, "children", { get: () => [w.header, w.list, ...(w.showTyping ? [w.typing] : []), ...(w.card ? [w.card] : [])] });
  w.byKey = new Map();
  w.contentH = 0;

  w.rows = () => w.col.children.flatMap((c) => c.tagName === "OL" ? c.children : [c]);
  w.max = () => Math.max(0, w.contentH - w.ch);
  w.setSt = (v, why) => {
    const nv = Math.max(0, Math.min(w.max(), Math.round(v)));
    if (nv !== w.st) { w.st = nv; w.pendingScroll = true; }
    return why;
  };
  w.layout = () => {
    if (!w.layoutDirty) return;
    w.layoutDirty = false;
    let y = PAD;
    for (const c of w.col.children) {
      c.y = y;
      if (c.tagName === "OL") {
        let ry = y;
        c.children.forEach((r, j) => { r.y = ry; ry += r.h + (j < c.children.length - 1 ? GAP : 0); });
        c.h = ry - y;
      }
      y += c.h + GAP;
    }
    w.contentH = y - GAP + PAD;
    // Native anchoring: the row that was first in view keeps its offset.
    if (NATIVE_ANCHORING && w.anchor && w.anchor.node.isConnected) w.setSt(w.anchor.node.y - w.anchor.off);
    w.anchor = null;
    if (w.st > w.max()) w.setSt(w.max());   // the clamp
  };
  w.pickAnchor = () => {
    w.layout();
    const node = w.rows().find((r) => r.y + r.h > w.st && r.y < w.st + w.ch);
    w.anchor = node ? { node, off: node.y - w.st } : null;
  };
  /** React's mutation: rows reconciled BY KEY, as React does. */
  w.mutate = ({ items, typing = w.showTyping, card = w.card ? { key: w.card.key, h: w.card.h } : null }) => {
    // WebKit's torn layout: only the rows that keep their key AND what they
    // say are laid out; rewritten, removed and new rows count for nothing.
    w.layout();
    const says = new Map(items.map((it) => [it.key, sayOf(it)]));
    const kept = w.list.children.filter((r) => says.get(r.key) === r.say);
    const torn = PAD + HEADER + GAP + kept.reduce((n, r) => n + r.h + GAP, 0) + (w.showTyping ? TYPING + GAP : 0) + (w.card ? w.card.h + GAP : 0) + PAD;
    if (w.st > Math.max(0, torn - w.ch)) {
      w.st = Math.max(0, torn - w.ch);
      w.pendingScroll = true;
      w.anchor = null;
    }
    const live = new Set();
    w.list.children = items.map((it) => {
      let r = w.byKey.get(it.key);
      if (!r) { r = block("LI", { key: it.key }); w.byKey.set(it.key, r); }
      r.say = sayOf(it); r.h = heightOf(it); r.isConnected = true;
      live.add(it.key);
      return r;
    });
    for (const [k, r] of w.byKey) if (!live.has(k)) { r.isConnected = false; w.byKey.delete(k); }
    w.showTyping = typing;
    if (card && (!w.card || w.card.key !== card.key)) { if (w.card) w.card.isConnected = false; w.card = block("DIV", { key: card.key, say: `card ${card.key}`, h: card.h }); }
    else if (card) w.card.h = card.h;
    else if (w.card) { w.card.isConnected = false; w.card = null; }
    w.layoutDirty = true;
  };
  w.el = {
    tagName: "DIV",
    get scrollTop() { w.layout(); return w.st; },
    set scrollTop(v) { w.layout(); w.setSt(v); },
    get scrollHeight() { w.layout(); return Math.max(w.contentH, w.ch); },
    get clientHeight() { return w.ch; },
    get firstElementChild() { return w.col; },
    getBoundingClientRect() { return { top: TOP, bottom: TOP + w.ch, height: w.ch }; },
    querySelector(sel) { return sel === ".chat-card" ? w.card : null; },
    addEventListener(t, f) { if (t === "scroll") w.listeners.add(f); },
    removeEventListener(t, f) { if (t === "scroll") w.listeners.delete(f); },
  };
  class RO {
    constructor(cb) { this.cb = cb; this.els = []; this.seen = new Map(); w.observers.push(this); }
    observe(el) { this.els.push(el); }
    disconnect() { this.els = []; w.observers = w.observers.filter((o) => o !== this); }
  }
  w.RO = RO;
  const sizeOf = (el) => el === w.el ? `${w.ch}` : `${(w.layout(), w.contentH)}`;
  /** One rendering update, in the HTML event loop's order. */
  w.frame = () => {
    if (w.pendingScroll) { w.pendingScroll = false; for (const f of [...w.listeners]) f(); }   // run the scroll steps
    w.layout();
    for (let round = 0; round < 4; round++) {                                                   // broadcast resize observations
      let any = false;
      for (const o of [...w.observers]) {
        const moved = o.els.filter((el) => o.seen.get(el) !== sizeOf(el));
        if (!moved.length) continue;
        for (const el of moved) o.seen.set(el, sizeOf(el));
        any = true;
        o.cb(moved.map((target) => ({ target })));
        w.layout();
      }
      if (!any) break;
    }
    w.painted.push(w.view());                                                                    // paint
  };
  /** What the reader sees. */
  w.view = () => {
    w.layout();
    const top = w.rows().find((r) => r.y + r.h > w.st);
    return { st: w.st, fromBottom: w.max() - w.st, top: top?.say ?? null, topOff: top ? top.y - w.st : null };
  };
  w.userScroll = (dy) => { w.layout(); w.setSt(w.st + dy); };
  return w;
}

// ── one chat: the hook, rendered and committed against a world ─────────────

function makeChat(opts) {
  const w = makeWorld(opts);
  const R = makeReact();
  const { useStickToBottom, BeforeCommit } = build({ ...R.env, ResizeObserver: w.RO });
  const ref = { current: w.el };
  let api = null, snapshotter = null;
  const chat = { w, get fresh() { return api.fresh; }, api: () => api, BeforeCommit };
  /** A render and its commit, with React's phases in React's order. */
  chat.commit = (state) => {
    const { items, promptKey = null, loaded = true } = state;
    const count = items.length;
    const replies = items.filter((it) => it.k === "reply").length;
    const lastReply = [...items].reverse().find((it) => it.k === "reply")?.key ?? null;
    for (let pass = 0; pass < 5; pass++) {
      api = R.render(() => OLD_SIGNATURE ? useStickToBottom(ref, count, replies, loaded, promptKey) : useStickToBottom(ref, loaded, promptKey, lastReply));
      // before mutation: <BeforeCommit take={take} />'s getSnapshotBeforeUpdate
      if (BeforeCommit && api.take) {
        if (!snapshotter) snapshotter = new BeforeCommit({ take: api.take });
        else { snapshotter.props = { take: api.take }; snapshotter.getSnapshotBeforeUpdate(); snapshotter.componentDidUpdate?.(); }
      }
      if (pass === 0) { w.pickAnchor(); w.mutate(state); }
      R.runLayout();
      w.layout();   // a layout effect elsewhere in the tree reads geometry (the composer's fit)
      R.runPassive();
      if (!R.dirty) break;
    }
  };
  chat.frames = (n = 4) => { for (let i = 0; i < n; i++) w.frame(); };
  chat.step = (state, n = 4) => { const from = w.painted.length; chat.commit(state); chat.frames(n); return w.painted.slice(from); };
  return chat;
}

// ── the transcript ──────────────────────────────────────────────────────────

const T0 = 1_793_470_000;
const SHAPES = [
  (n) => `Short reply ${n}.`,
  (n) => `Reply ${n}: ` + "the reader keeps the place it was reading while the window moves. ".repeat(4),
  (n) => `Reply ${n} with a list:\n\n1. one\n2. two\n3. three\n\n` + "and a fence below it. ".repeat(3),
];
const turnOf = (n) => ({ at: T0 + n * 120, user: `Message ${n}: do step ${n}.`, reply: SHAPES[n % 3](n), tools: n % 3 === 2 ? ["Read", "Edit"] : [], truncated: false });
const ALL = Array.from({ length: 24 }, (_, i) => turnOf(i + 1));
const windowOf = (turns) => turns.slice(-C.TURNS_MAX);
const itemsOf = (turns, pending = []) => C.itemsOf(windowOf(turns), [], C.settlePending(pending, windowOf(turns)));
const pendingOf = (text, n) => ({ id: n, text, sentAt: (T0 + n * 120) * 1000, queued: false });

/** Open on the 24 turns, at the bottom. */
function opened(opts) {
  const chat = makeChat(opts);
  chat.commit({ items: [], loaded: false });   // mounted before the read lands: effects attach
  chat.frames(2);
  chat.step({ items: itemsOf(ALL) });
  return chat;
}
/** The send, the read that settles it (the window slides), the reply. */
function sendAndReply(chat, text, extra = {}) {
  const n = 25;
  const p = pendingOf(text, n);
  const frames = [];
  frames.push(...chat.step({ items: itemsOf(ALL, [p]), ...extra }));
  const sent = [...ALL, { at: T0 + n * 120, user: text, reply: "", tools: [], truncated: false }];
  frames.push(...chat.step({ items: itemsOf(sent, [p]), ...extra }));
  const replied = [...ALL, { at: T0 + n * 120, user: text, reply: "On it. ".repeat(30), tools: ["Read"], truncated: false }];
  frames.push(...chat.step({ items: itemsOf(replied, [p]), ...extra }));
  return { frames, replied };
}
const at = (f) => f.fromBottom <= 1;
const fmt = (f) => `st=${f.st} fromBottom=${f.fromBottom} top=${JSON.stringify((f.top ?? "").slice(0, 28))}@${f.topOff}`;

// ── the keys ────────────────────────────────────────────────────────────────

{
  const a = new Map(itemsOf(ALL).map((it) => [it.key, sayOf(it)]));
  const b = itemsOf([...ALL, turnOf(25)]);
  const moved = b.filter((it) => a.has(it.key) && a.get(it.key) !== sayOf(it));
  check(moved.length === 0, `a slid window keeps every surviving row's key on the same content (${moved.length} keys now name other content${moved.length ? `, e.g. ${moved[0].key}` : ""})`);
}

// ── the cases ───────────────────────────────────────────────────────────────

// 1. At the bottom, a reply grows its turn and the count stays the same.
{
  const chat = opened();
  check(at(chat.w.view()), "opens at the bottom");
  const grown = [...ALL.slice(0, -1), { ...ALL[23], reply: ALL[23].reply + " And more. ".repeat(40), tools: ["Bash", "Edit"] }];
  const before = itemsOf(ALL).length;
  const frames = chat.step({ items: itemsOf(grown) });
  check(itemsOf(grown).length === before && frames.every(at), `at the bottom, a reply grows with the count unchanged (${before}): every frame stays at the bottom (${fmt(frames.at(-1))})`);
}

// 2. At the bottom, a send slides the window, the pending one settles, and
//    the reply lands: the case the owner saw.
{
  const chat = opened();
  const { frames } = sendAndReply(chat, "Slide the window, please.");
  const worst = frames.reduce((m, f) => (f.fromBottom > m.fromBottom ? f : m), frames[0]);
  check(frames.every(at), `at the bottom, the window slides and a reply lands: every frame stays at the bottom (worst ${fmt(worst)})`);
  check(!chat.fresh, "…and offers no \"New messages\" (the reader is following)");
  const next = chat.step({ items: itemsOf([...ALL, { at: T0 + 25 * 120, user: "Slide the window, please.", reply: "On it. ".repeat(30) + " Done.".repeat(30), tools: ["Read"], truncated: false }]) });
  check(next.every(at), "…and still follows the next change");
}

// 3. Scrolled up, the window slides (turns drop off the top) and the reply
//    lands below: the row being read does not move, and "New messages".
{
  const chat = opened();
  chat.w.userScroll(-1500);
  chat.frames(2);
  const read = chat.w.view();
  const { frames } = sendAndReply(chat, "Slide it while I read.");
  const off = frames.filter((f) => f.top !== read.top || Math.abs(f.topOff - read.topOff) > 1);
  check(off.length === 0, `scrolled up, turns drop off the top: the row being read stays put on every frame (${fmt(read)}${off.length ? ` → ${fmt(off[0])}` : ""})`);
  check(chat.fresh, "…and the reply that landed below is offered as \"New messages\"");
}

// 3b. A reader 120px up (not following) when the window slides: the clamp
//     inside the commit must not leak into where they are.
{
  const chat = opened();
  chat.w.userScroll(-120);
  chat.frames(2);
  const read = chat.w.view();
  const { frames } = sendAndReply(chat, "Slide it while I am near the end.");
  const off = frames.filter((f) => f.top !== read.top || Math.abs(f.topOff - read.topOff) > 1);
  check(off.length === 0, `120px up, the window slides: the clamp inside the commit does not move the reader (${fmt(read)}${off.length ? ` → ${fmt(off[0])}` : ""})`);
}

// 4. Scrolled up, a reply lands below in the last turn (no slide).
{
  const chat = opened();
  chat.w.userScroll(-900);
  chat.frames(2);
  const read = chat.w.view();
  const more = [...ALL];
  more[23] = { ...ALL[23], steps: [{ kind: "text", at: ALL[23].at, text: ALL[23].reply }, { kind: "text", at: ALL[23].at + 30, text: "A second reply, below the reader." }] };
  const frames = chat.step({ items: itemsOf(more) });
  check(frames.every((f) => f.top === read.top && Math.abs(f.topOff - read.topOff) <= 1), `scrolled up, a reply lands below: the view holds (${fmt(read)} → ${fmt(frames.at(-1))})`);
  check(chat.fresh, "…and offers \"New messages\"");
}

// 5. A new prompt is brought into view; one taller than the view opens at its top.
{
  const chat = opened();
  const items = itemsOf(ALL);
  chat.step({ items, card: { key: "fits", h: 260 }, promptKey: "fits" });
  const c = chat.w.card;
  check(c.y >= chat.w.st && c.y + c.h <= chat.w.st + chat.w.ch, "at the bottom, a prompt that fits comes up in full view");
  chat.w.userScroll(-1200);
  chat.frames(2);
  chat.step({ items, card: { key: "tall", h: 1400 }, promptKey: "tall" });
  const t = chat.w.card;
  check(t.y - chat.w.st >= 0 && t.y - chat.w.st <= 16, `scrolled up, a prompt taller than the view opens at its top (${t.y - chat.w.st}px down)`);
}

// 6. The reader scrolls up and an update lands before that scroll's event
//    does: the update must not pull them back down.
{
  const chat = opened();
  chat.w.userScroll(-400);                  // no frame: the event is still on its way
  const read = chat.w.view();
  const more = [...ALL];
  more[23] = { ...ALL[23], steps: [{ kind: "text", at: ALL[23].at, text: ALL[23].reply }, { kind: "text", at: ALL[23].at + 30, text: "More. ".repeat(60) }] };
  const frames = chat.step({ items: itemsOf(more) });
  check(frames.every((f) => f.top === read.top && Math.abs(f.topOff - read.topOff) <= 1), `a scroll up whose event has not arrived yet is not undone by a reply landing (${fmt(read)} → ${fmt(frames.at(-1))})`);
}

// 6b. At the bottom, a reply lands, and before the next frame it grows
//     outside any commit (an image, a font): the scroll event this hook's own
//     follow fired reads "200px up" when it arrives, and must not count.
{
  const chat = opened();
  const more = [...ALL];
  more[23] = { ...ALL[23], steps: [{ kind: "text", at: ALL[23].at, text: ALL[23].reply }, { kind: "text", at: ALL[23].at + 30, text: "A picture below." }] };
  chat.commit({ items: itemsOf(more) });
  const last = chat.w.list.children.at(-1);
  last.h += 200;
  chat.w.layoutDirty = true;
  chat.frames(4);
  check(at(chat.w.view()), `at the bottom, a reply grows outside a commit before its follow's scroll event lands: still followed (${fmt(chat.w.view())})`);
}

// 7. At the bottom, the composer grows (the view shrinks): the bottom stays.
{
  const chat = opened();
  chat.w.ch -= 96;
  chat.frames(3);
  check(at(chat.w.view()), `at the bottom, the composer grows: the bottom stays in view (${fmt(chat.w.view())})`);
}

// 8. A reader 80px up on a busy turn: the turn ends, the typing row goes and
//    the bottom comes to them; the next reply is followed.
{
  const chat = opened();
  chat.step({ items: itemsOf(ALL), typing: true });
  chat.w.userScroll(-80);
  chat.frames(2);
  const done = [...ALL.slice(0, -1), { ...ALL[23], reply: ALL[23].reply }];
  chat.step({ items: itemsOf(done), typing: false });
  const more = [...ALL.slice(0, -1), { ...ALL[23], reply: ALL[23].reply + " Then a long addendum.".repeat(30) }];
  const frames = chat.step({ items: itemsOf(more), typing: false });
  check(at(frames.at(-1)), `the bottom came to a reader 80px up (the typing row went): the next reply is followed (${fmt(frames.at(-1))})`);
}

// 9. Back at the bottom after "New messages": following resumes.
{
  const chat = opened();
  chat.w.userScroll(-1000);
  chat.frames(2);
  sendAndReply(chat, "One while I read.");
  check(chat.fresh, "scrolled up, a reply arrives: \"New messages\"");
  chat.api().dismiss(); chat.api().toBottom();
  chat.frames(2);
  const more = [...ALL, { at: T0 + 25 * 120, user: "One while I read.", reply: "On it. ".repeat(30) + "And done. ".repeat(40), tools: ["Read"], truncated: false }];
  const frames = chat.step({ items: itemsOf(more) });
  check(frames.every(at) && !chat.fresh, `after "New messages" takes the reader down, the next reply is followed (${fmt(frames.at(-1))})`);
}

// The hook reads the DOM before each commit only if ChatPane renders the
// component that calls it.
if (!OLD_SIGNATURE) {
  const pane = fs.readFileSync(PANE, "utf8");
  check(/<BeforeCommit take=\{take\} \/>/.test(pane), "ChatPane renders <BeforeCommit take={take} /> (the snapshot before every commit)");
}

if (failed) { console.log(`\nchatscroll-check: ${failed} failure(s)`); process.exit(1); }
console.log("\nchatscroll-check: all passed");
