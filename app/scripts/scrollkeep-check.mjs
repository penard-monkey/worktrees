// Deterministic check that a rendered document keeps its place when it refreshes.
//
// The report (2026-10-14, a user, verbatim): "the rendered markdown views
// refresh periodically and the view port for the pane resets to the top of the
// document instead of keeping place". The refresh is `places:changed` — the
// backend's poll / fs watch — which bumps App's `placesToken`, handed to every
// FileView as `reloadToken`.
//
// What actually reset it (measured in Playwright WebKit against the mock with
// `?slowread`): a refresh that read the file while an agent was writing it.
// The stub document clamps scrollTop, the clamp fires a scroll event, and when
// the whole file comes back on the next tick there is no position left to go
// back to. What did NOT reset it, although it looks like it should: FileView
// also flipped its `.scroll` box to a "loading…" note on EVERY refresh, but
// React reuses that div and both engines keep its offset through the flip.
// That frame is gone anyway — a re-render of the whole document per tick for
// nothing.
//
// Two halves, both against REAL source:
//
//   FileView (FilesPane.tsx, sliced the way termreplay-check.mjs slices
//   TerminalPane.tsx) runs under a minimal hooks runtime. Across a refresh that
//   re-reads the SAME bytes the scroller must never leave the tree and FileView
//   must not re-render at all; across a refresh with NEW bytes the scroller
//   must stay mounted, and the viewport hook must be told the content moved.
//
//   scrollkeep.ts runs under DOM stubs with real geometry and a box that
//   clamps and fires `scroll` late, as a browser does: an edit ABOVE the fold
//   keeps the block being read at the same offset, a stub read mid-write does
//   not lose the place, a remount comes back to where it was, another file in
//   the same box starts at its top, and an unchanged render touches nothing.
//
// The mock harness alone cannot see this — its read_file answers in a
// microtask — and bats never reaches the frontend. Half 1 fails on the
// FilesPane.tsx before the fix; the stub-read case fails on a hook that
// trusts every scroll event.
//
//   node scrollkeep-check.mjs [path/to/FilesPane.tsx]   exits non-zero on failure
import fs from "node:fs";
import { transformWithEsbuild } from "vite";

const SRC = process.argv[2] ?? new URL("../src/FilesPane.tsx", import.meta.url).pathname;
const KEEP = new URL("../src/scrollkeep.ts", import.meta.url).pathname;

let failed = 0;
const ok = (cond, msg) => {
  if (cond) console.log("  ok   " + msg);
  else { failed++; console.log("  FAIL " + msg); }
};

const strip = (raw) => raw
  .split("\n")
  .filter((l) => !/^\s*import\s/.test(l) && !/^\s*}\s*from\s+"/.test(l))
  .join("\n")
  .replace(/^export (function|const|type|class|async function) /gm, "$1 ");

async function load(code, names, ret) {
  const wrapped = `export function build(env: any) {\n  const { ${names.join(", ")} } = env;\n${code}\n  return { ${ret.join(", ")} };\n}\n`;
  const out = await transformWithEsbuild(wrapped, "x.tsx", { jsx: "transform", jsxFactory: "h", jsxFragment: "F", loader: "tsx" });
  const mod = await import("data:text/javascript;base64," + Buffer.from(out.code).toString("base64"));
  return mod.build;
}

// ── a hooks runtime small enough to read ────────────────────────────────
// One component instance. Effects run after each render; a state change marks
// it dirty and the driver renders again. setState with an identical value is a
// bail-out, as in React — which is exactly what "must not re-render" measures.
function runtime() {
  const slots = [];
  let i = 0, dirty = false;
  const pending = [];
  const depsMoved = (a, b) => !a || !b || a.length !== b.length || a.some((x, k) => !Object.is(x, b[k]));
  const R = {
    renders: 0,
    useState(init) {
      const k = i++;
      if (!(k in slots)) slots[k] = { v: typeof init === "function" ? init() : init };
      const s = slots[k];
      s.set ??= (nv) => {
        const v = typeof nv === "function" ? nv(s.v) : nv;
        if (!Object.is(v, s.v)) { s.v = v; dirty = true; }
      };
      return [s.v, s.set];
    },
    useRef(init) { const k = i++; if (!(k in slots)) slots[k] = { current: init }; return slots[k]; },
    useMemo(fn, deps) {
      const k = i++;
      if (!(k in slots) || depsMoved(slots[k].deps, deps)) slots[k] = { v: fn(), deps };
      return slots[k].v;
    },
    useCallback(fn, deps) { return R.useMemo(() => fn, deps); },
    useEffect(fn, deps) {
      const k = i++;
      const s = slots[k];
      if (!s || depsMoved(s.deps, deps)) {
        slots[k] = { deps, cleanup: s?.cleanup };
        pending.push(() => { slots[k].cleanup?.(); const c = fn(); slots[k].cleanup = typeof c === "function" ? c : undefined; });
      }
    },
    useSyncExternalStore(_sub, get) { return get(); },
    render(fn, props) {
      i = 0; dirty = false; R.renders++;
      const tree = fn(props);
      while (pending.length) pending.shift()();
      return tree;
    },
    get dirty() { return dirty; },
  };
  R.useLayoutEffect = R.useEffect;
  return R;
}

const flush = () => new Promise((r) => setTimeout(r, 0));

// ── half 1: FileView across refreshes ───────────────────────────────────
const MarkdownStub = function Markdown() { return null; };
const keepCalls = [];
const rt = runtime();
const disk = { content: "# Title\n\npara one\n\npara two\n", mtime: 1 };
let reads = 0;

const raw = fs.readFileSync(SRC, "utf8");
const body = strip(raw);
if (!body.includes("function FileView(")) throw new Error("FileView not found in " + SRC);
const build = await load(body, [
  "Component", "Fragment", "useCallback", "useEffect", "useLayoutEffect", "useMemo", "useRef", "useState", "useSyncExternalStore",
  "invoke", "copyToClipboard", "openInDefaultApp", "openUrl", "revealItemInDir", "Icons", "CodeBlock", "CtxMenu", "DiffView",
  "FindBar", "useFileFind", "Markdown", "useDocPathLinks", "basename", "fileInfo", "humanSize", "relPath", "revealsThrough",
  "treePath", "MD_ZOOM_MAX", "MD_ZOOM_MIN", "clampMdZoom", "stepMdZoom", "applyMd", "spliceRange", "useScrollKeep",
  "h", "F", "CSS", "document",
], ["FileView"]);

const env = {
  Component: class {}, Fragment: "Fragment",
  useCallback: (...a) => rt.useCallback(...a), useEffect: (...a) => rt.useEffect(...a),
  useLayoutEffect: (...a) => rt.useLayoutEffect(...a), useMemo: (...a) => rt.useMemo(...a),
  useRef: (...a) => rt.useRef(...a), useState: (...a) => rt.useState(...a),
  useSyncExternalStore: (...a) => rt.useSyncExternalStore(...a),
  invoke: async (cmd) => {
    if (cmd !== "read_file") throw new Error("unexpected invoke " + cmd);
    reads++;
    return { content: disk.content, truncated: false, binary: false, mtime: disk.mtime, size: disk.content.length };
  },
  copyToClipboard() {}, openInDefaultApp() {}, openUrl: async () => {}, revealItemInDir: async () => {},
  Icons: new Proxy({}, { get: () => () => null }),
  CodeBlock: function CodeBlock() { return null; }, CtxMenu: () => null, DiffView: () => null,
  FindBar: () => null, useFileFind: () => ({}), Markdown: MarkdownStub,
  useDocPathLinks: () => ({ pathLinks: undefined, menu: null }),
  basename: (p) => p.slice(p.lastIndexOf("/") + 1),
  fileInfo: () => ({ kind: "markdown", label: "Markdown", mime: "text/markdown", lang: "" }),
  humanSize: (n) => `${n} B`, relPath: (p) => p, revealsThrough: () => false, treePath: (p) => p,
  MD_ZOOM_MAX: 200, MD_ZOOM_MIN: 50, clampMdZoom: (z) => z, stepMdZoom: (z) => z,
  applyMd: () => ({}), spliceRange: () => ({}),
  // Recorded, not run: half 2 tests the hook itself.
  useScrollKeep: (ref, key, content, blocks) => keepCalls.push({ ref, key, content, blocks }),
  h: (type, props, ...children) => ({ type, props: { ...(props ?? {}), children } }), F: "Fragment",
  CSS: { escape: (s) => s }, document: {},
};
const { FileView } = build(env);

const props = {
  path: "/repo/.worktrees/a/docs/NOTES.md", reloadToken: 0,
  onOpenEditor() {}, onOpen() {}, onError: (e) => { throw e; },
  wrap: false, onWrap() {}, mdSource: false, onMdSource() {},
  diff: false, onDiff() {}, diffBase: "base", onDiffBase() {},
  mdZoom: 100, onMdZoom() {}, expanded: false, onExpand() {},
};

function find(node, pred) {
  if (!node || typeof node !== "object") return null;
  if (Array.isArray(node)) { for (const n of node) { const f = find(n, pred); if (f) return f; } return null; }
  if (pred(node)) return node;
  return find(node.props?.children, pred);
}
const scrollerOf = (tree) => find(tree, (n) => typeof n.props?.className === "string" && n.props.className.split(" ").includes("scroll"));
const mdIn = (n) => find(n, (x) => x.type === MarkdownStub);

/** Render until stable, recording every tree on the way. A state change an
 *  effect makes SYNCHRONOUSLY is rendered before any invoke can answer: the
 *  real read is IPC plus a disk read, so React always paints that frame first
 *  — and it is the frame the bug lived in. Letting the stub's promise resolve
 *  ahead of it would hide the "loading…" render entirely (it did, in the first
 *  cut of this check). */
async function settle(p) {
  const trees = [rt.render(FileView, p)];
  for (let n = 0; n < 20; n++) {
    while (rt.dirty) trees.push(rt.render(FileView, p));
    await flush();
    if (!rt.dirty) break;
  }
  return trees;
}

console.log("FileView across a refresh");
let trees = await settle(props);
const first = trees.at(-1);
ok(!!scrollerOf(first) && !!mdIn(scrollerOf(first)), "the document renders inside a .scroll box");

// Refresh, same bytes on disk.
let before = rt.renders;
const readsBefore = reads;
trees = await settle({ ...props, reloadToken: 1 });
ok(reads === readsBefore + 1, "a refresh re-reads the file");
ok(trees.every((t) => scrollerOf(t)), "the scroller never leaves the tree on a refresh (no 'loading…' frame)");
ok(rt.renders - before === 1, `unchanged bytes cost no extra render (${rt.renders - before} renders, want 1: the prop change itself)`);

// Refresh, new bytes on disk.
disk.content = "# Title\n\nINSERTED\n\npara one\n\npara two\n";
disk.mtime = 2;
trees = await settle({ ...props, reloadToken: 2 });
ok(trees.every((t) => scrollerOf(t)), "the scroller never leaves the tree on a refresh that changed the file");
ok(mdIn(scrollerOf(trees.at(-1)))?.props.src === disk.content, "the new content is what is drawn");
const last = keepCalls.at(-1);
ok(!!last && last.content === disk.content, "the viewport hook is given the drawn content");
ok(!!last && typeof last.key === "string" && last.key.includes(props.path), "the viewport hook's memory is keyed by the file's path");
ok(!!last && last.blocks === ".md > *", "rendered markdown anchors on its top-level blocks");
ok(!!last && scrollerOf(trees.at(-1))?.props.ref === last.ref, "the hook's ref is the .scroll box's own ref");

// ── half 2: scrollkeep.ts under DOM stubs ───────────────────────────────
// A box 100px tall at y=0; each block is `h` px tall and laid out in order.
// getBoundingClientRect answers from the current scrollTop, as the DOM does.
console.log("scrollkeep.ts");
const keepSrc = fs.readFileSync(KEEP, "utf8");
const kbuild = await load(strip(keepSrc), ["useLayoutEffect", "useRef"],
  ["captureAnchor", "restoreAnchor", "rememberAnchor", "recallAnchor", "useScrollKeep"]);

// The box clamps scrollTop to its content and fires `scroll` LATER, as a
// browser does — the clamp's own event is the one that used to be mistaken
// for the reader moving.
function fakeBox(texts, h = 30) {
  let top = 0;
  const box = {
    listeners: [],
    blocks: [],
    get scrollTop() { return top; },
    set scrollTop(v) {
      const max = Math.max(0, box.blocks.length * h - 100);
      const nv = Math.max(0, Math.min(max, v));
      if (nv === top) return;
      top = nv;
      setTimeout(() => box.listeners.forEach((f) => f()), 0);
    },
    set(ts) {
      box.blocks = ts.map((t, i) => ({
        textContent: t,
        getBoundingClientRect: () => ({ top: i * h - top, bottom: (i + 1) * h - top }),
      }));
      // Layout: a shorter document clamps the box, and says so with an event.
      box.scrollTop = top;
    },
    getBoundingClientRect: () => ({ top: 0, bottom: 100 }),
    querySelectorAll: (sel) => { if (sel !== ".md > *") throw new Error("selector " + sel); return box.blocks; },
    addEventListener: (_t, fn) => box.listeners.push(fn),
    removeEventListener: (_t, fn) => { box.listeners = box.listeners.filter((f) => f !== fn); },
  };
  box.set(texts);
  return box;
}
/** The reader scrolls: set, then let the event land. */
const scrollTo = async (box, y) => { box.scrollTop = y; await flush(); };
const topOf = (box, text) => box.blocks.find((b) => b.textContent === text)?.getBoundingClientRect().top;

const krt = runtime();
const K = kbuild({ useLayoutEffect: (...a) => krt.useLayoutEffect(...a), useRef: (...a) => krt.useRef(...a) });

const doc = Array.from({ length: 40 }, (_, i) => `block ${i}`);
const box = fakeBox(doc);
const ref = { current: box };
let key = "k1";
const Host = ({ content }) => { K.useScrollKeep(ref, key, content, ".md > *"); return null; };
/** A commit: the DOM changes, then the layout effect runs, then events land. */
const commit = async (texts, content) => { if (texts) ref.current.set(texts); krt.render(Host, { content }); await flush(); };

await commit(null, "v1");
await scrollTo(box, 20 * 30 + 12); // reading block 20, 12px of it scrolled past
const a = K.captureAnchor(box, ".md > *");
ok(a.idx === 20 && a.sig === "block 20" && a.off === -12, `anchor is the top-visible block and its offset (${JSON.stringify(a)})`);

const t0 = box.scrollTop;
await commit(null, "v1");
ok(box.scrollTop === t0, "an unchanged render moves nothing");

// Three blocks inserted ABOVE the fold: React rewrites nodes in place by index.
let texts = ["new a", "new b", "new c", ...doc];
await commit(texts, "v2");
ok(topOf(box, "block 20") === -12, `an edit above the fold keeps the block being read where it was (block 20 at ${topOf(box, "block 20")}px, want -12)`);

// The case that reached a user: a refresh reads the file mid-write and draws a
// stub, which clamps the box to the top; then the whole file is back.
await commit(["new a", "partial"], "stub");
ok(box.scrollTop === 0, "a stub document clamps the box (the browser's doing, not ours)");
await commit(texts, "v2-again");
ok(topOf(box, "block 20") === -12, `a stub read mid-write does not lose the place: the full file comes back to it (block 20 at ${topOf(box, "block 20")}px, want -12)`);

// The anchored block itself was edited away: fall back to the pixel.
await scrollTo(box, 10 * 30);
const t1 = box.scrollTop;
const edited = box.blocks.map((b) => b.textContent);
edited[10] = "rewritten";
await commit(edited, "v3");
ok(box.scrollTop === t1, "when the block being read is the one edited, scrollTop holds");

// Remount: a fresh scroller element under the same key comes back to the spot.
await scrollTo(box, 25 * 30 + 5);
const box2 = fakeBox(box.blocks.map((b) => b.textContent));
ref.current = box2;
await commit(null, "v3");
ok(topOf(box2, box2.blocks[25].textContent) === -5, `a remounted scroller restores its position (block 25 at ${box2.blocks[25].getBoundingClientRect().top}px, want -5)`);
ok(box.listeners.length === 0, "the old scroller's listener is removed");

// Another document in the SAME element (the Files tab opening a second file):
// it starts at its top, not at the first file's offset.
key = "k2";
await commit(Array.from({ length: 40 }, (_, i) => `other ${i}`), "other");
ok(box2.scrollTop === 0, `a different document in a reused box starts at the top (scrollTop ${box2.scrollTop})`);
// …and going back to the first restores it.
key = "k1";
await commit(box.blocks.map((b) => b.textContent), "v3");
ok(box2.blocks[25].getBoundingClientRect().top === -5, "switching back to the first document restores its place");

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("\nall ok");
