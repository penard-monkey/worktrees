// Deterministic check of the dock viewer's ONE editable surface: a markdown
// file's Source view (FilesPane.tsx).
//
//   node mdedit-check.mjs [path/to/FilesPane.tsx]     exits non-zero on failure
//
// Everything here is invisible to every other suite. bats never loads the
// frontend; the unit tests never see a textarea; and the mock harness answers
// `write_file` in a microtask, so the two orderings that actually bite — a
// second save in one sitting, and a keystroke that lands while a save is in
// flight — cannot happen in it at all (CLAUDE.md, "the mock answers INSTANTLY").
//
// Like race-check.mjs and ctxmenu-check.mjs, it does NOT paraphrase the
// component: it evaluates the REAL source text of FilesPane.tsx under React/DOM
// stubs, so running it before and after an edit tests the edit itself. It fails
// on the pre-edit file, which is how it earns trust:
//
//   git show HEAD:app/src/FilesPane.tsx > /tmp/old.tsx && node mdedit-check.mjs /tmp/old.tsx
//
// The four rules it pins, and why each one is here:
//
//   1. A TRUNCATED read is never editable. The buffer is the first 1 MiB of the
//      file; saving it would silently delete everything past the cap. This is
//      the only rule in the file that can destroy data.
//   2. A save carries the mtime the edit STARTED from, and the ack's new mtime
//      re-bases the next one. Without the re-base the second ⌘S in a sitting is
//      refused as a conflict with the first — the compare-and-swap firing on
//      the one writer it is not there to stop. (`write_file` returns the saved
//      mtime for exactly this; it used to return nothing.)
//   3. A keystroke that lands WHILE a save is in flight is not thrown away.
//      Only the draft that was actually sent is cleared.
//   4. Overwriting after a refusal drops the guard (`expectedMtime: null`) —
//      and only after a refusal, and only on its own button. That is the one
//      path by which this pane can clobber what Claude wrote next door, so it
//      may never be reachable by accident.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
// vite's esbuild re-export — see race-check.mjs: bare "esbuild" does not
// resolve under pnpm's strict layout, vite does (direct dependency).
import { transformWithEsbuild } from "vite";

const SRC = process.argv[2] || fileURLToPath(new URL("../src/FilesPane.tsx", import.meta.url));
const raw = fs.readFileSync(SRC, "utf8");

let bad = 0;
const fail = (m) => { console.error(`FAIL ${m}`); bad++; };
const ok = (cond, m) => { if (!cond) fail(m); };
const eq = (got, want, what) => {
  if (!Object.is(got, want)) fail(`${what}: got ${JSON.stringify(got)}, want ${JSON.stringify(want)}`);
};

// ── the real modules, where they are pure ───────────────────────────────────
// `fileInfo` decides what "a markdown file" IS (`filekind.ts`), and that is
// half of rule 1 — stubbing it would leave the check agreeing with itself.
// filekind.ts has no imports at all, so it loads as-is.
const kindJs = (await transformWithEsbuild(
  fs.readFileSync(fileURLToPath(new URL("../src/filekind.ts", import.meta.url)), "utf8"),
  "filekind.ts", { loader: "ts", format: "esm" })).code;
const filekind = await import("data:text/javascript;base64," + Buffer.from(kindJs).toString("base64"));

// ── FilesPane.tsx, verbatim below its imports ───────────────────────────────
const body = raw
  .split("\n")
  .filter((l) => !/^\s*import\s/.test(l))
  .join("\n")
  .replace(/^export (function|type|const) /gm, "$1 ");
if (!body.includes("function FileView")) throw new Error("FileView not found in " + SRC);

const wrapped = `
export function build(env) {
  const {
    Component, useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore,
    invoke, openInDefaultApp, openUrl, revealItemInDir, Icons,
    CodeBlock, CtxMenu, DiffView, FindBar, useFileFind, Markdown,
    basename, fileInfo, humanSize, relPath,
    MD_ZOOM_MAX, MD_ZOOM_MIN, clampMdZoom, stepMdZoom,
    h, F,
  } = env;
${body}
  // typeof guards so the PRE-edit file, which has none of these, fails with a
  // sentence instead of a ReferenceError from inside a base64 data: URL.
  // (No backticks in here: this whole wrapper is a template literal.)
  return {
    FileView,
    SourceEditor: typeof SourceEditor === "undefined" ? null : SourceEditor,
    drafts: typeof drafts === "undefined" ? null : drafts,
    putDraft: typeof putDraft === "undefined" ? null : putDraft,
  };
}`;
const js = (await transformWithEsbuild(wrapped, "mdedit-check.tsx", {
  loader: "tsx", format: "esm", jsx: "transform", jsxFactory: "h", jsxFragment: "F",
})).code;
const { build } = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

// ── stubs ───────────────────────────────────────────────────────────────────
/** A stub component renders as `<x-stub-Name>`, so the assertions below can ask
 *  "is the body the read-only renderer or the editor" in one vocabulary. */
const stub = (name) => Object.defineProperty((props) => h(`x-stub-${name}`, props), "name", { value: name });
class Component {}

const h = (type, props, ...children) => ({ type, props: props ?? {}, children: children.flat(Infinity) });
const F = "fragment";

/** Render every function node in a finished tree down to primitive tags. The
 *  parent's hooks have already run, so this cannot disturb the cursor —
 *  SourceEditor holds no state of its own, which is itself deliberate. */
function deep(node, depth = 0) {
  if (node == null || node === false || depth > 40) return node;
  if (Array.isArray(node)) return node.map((n) => deep(n, depth + 1));
  if (typeof node !== "object") return node;
  const { type, props, children } = node;
  if (typeof type === "function") {
    // A class component (the viewer's error boundary) is not callable.
    if (type.prototype instanceof Component || type === Component) return deep(children, depth + 1);
    return deep(type({ ...props, children }), depth + 1);
  }
  return { type, props, children: children.map((c) => deep(c, depth + 1)) };
}

function walk(node, out = []) {
  if (node == null || typeof node !== "object") return out;
  if (Array.isArray(node)) { node.forEach((n) => walk(n, out)); return out; }
  out.push(node);
  (node.children ?? []).forEach((c) => walk(c, out));
  return out;
}
const tags = (tree, name) => walk(tree).filter((n) => n.type === name);
function textOf(node) {
  if (node == null || typeof node === "boolean") return "";
  if (typeof node !== "object") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  return (node.children ?? []).map(textOf).join("");
}

// ── a hook runtime just big enough for FileView ─────────────────────────────
// The module is built ONCE and the hooks dispatch to whichever mount is
// rendering, the way React's own dispatcher does. Building it per mount would
// give each one a private copy of the draft store — and then rule 5, the whole
// point of which is that there is only one, would pass against two.
let current = null;
const sameDeps = (a, b) => a && b && a.length === b.length && a.every((v, n) => Object.is(v, b[n]));

const env = {
  Component,
  invoke: (cmd, args) => current.invoke(cmd, args),
    openInDefaultApp: () => Promise.resolve(),
    openUrl: () => Promise.resolve(),
    revealItemInDir: () => Promise.resolve(),
    Icons: new Proxy({}, { get: (_t, k) => stub(String(k)) }),
    CodeBlock: stub("CodeBlock"),
    CtxMenu: stub("CtxMenu"),
    DiffView: stub("DiffView"),
    FindBar: stub("FindBar"),
    Markdown: stub("Markdown"),
    useFileFind: () => ({
      query: "", setQuery: () => {}, index: 0, count: 0, capped: false,
      caseSensitive: false, setCaseSensitive: () => {}, next: () => {}, prev: () => {},
    }),
    basename: filekind.basename, fileInfo: filekind.fileInfo,
    humanSize: filekind.humanSize, relPath: filekind.relPath,
    // The zoom stepper is zoom-check.mjs's subject, not this one.
    MD_ZOOM_MAX: 200, MD_ZOOM_MIN: 70, clampMdZoom: (v) => v, stepMdZoom: (v) => v,
  h, F,

  useState(init) {
    // `ctx`, not `current`: a setter fires from a promise continuation (the
    // save's `.finally`), where no render is in progress and `current` is null.
    const ctx = current;
    const c = ctx.cell(() => ({ v: typeof init === "function" ? init() : init }));
    return [c.v, (n) => {
      const next = typeof n === "function" ? n(c.v) : n;
      if (!Object.is(next, c.v)) { c.v = next; ctx.dirty = true; }
    }];
  },
  useRef(init) { return current.cell(() => ({ current: init })); },
  useMemo(fn, deps) {
    const c = current.cell(() => ({ deps: null, v: undefined, first: true }));
    if (c.first || !sameDeps(c.deps, deps)) { c.v = fn(); c.deps = deps; c.first = false; }
    return c.v;
  },
  useCallback(fn, deps) { return env.useMemo(() => fn, deps); },
  useEffect(fn, deps) {
    const ctx = current;
    const c = ctx.cell(() => ({ deps: null, cleanup: null, first: true }));
    if (c.first || !sameDeps(c.deps, deps)) {
      ctx.pending.push(() => { c.cleanup?.(); c.cleanup = fn() ?? null; });
      c.deps = deps;
      c.first = false;
    }
  },
  useSyncExternalStore(sub, get) {
    const ctx = current;
    const c = ctx.cell(() => ({ subbed: false }));
    if (!c.subbed) { c.subbed = true; sub(() => { ctx.dirty = true; }); }
    return get();
  },
};

const { FileView, SourceEditor, drafts, putDraft } = build(env);
if (!SourceEditor || !drafts || !putDraft) {
  fail(`${SRC} has no editor: SourceEditor / the draft store are missing, so the markdown Source view is read-only.`);
  console.error("\nmdedit-check: 1 failure(s)");
  process.exit(1);
}

/** One mounted FileView. `render()` is synchronous; the driver settles effects
 *  and microtasks between acts. */
function mount(props, { invoke }) {
  const cells = [];
  const self = {
    props, invoke, pending: [], dirty: true, i: 0, tree: null,
    drafts, putDraft,
    cell(make) { return (cells[self.i++] ??= make()); },
    render() {
      const prev = current;
      current = self;
      try {
        for (let pass = 0; pass < 20; pass++) {
          self.dirty = false;
          self.i = 0;
          self.tree = deep(FileView(self.props));
          while (self.pending.length) self.pending.shift()();
          if (!self.dirty) break;
        }
      } finally { current = prev; }
      return self.tree;
    },
    /** Run a handler as this mount — `invoke` dispatches to whoever is
     *  current, and a click is not a render. */
    act(fn) {
      const prev = current;
      current = self;
      try { return fn(); } finally { current = prev; self.render(); }
    },
    /** Let the read/save promises land, then re-render until stable. */
    async settle(n = 6) {
      for (let k = 0; k < n; k++) {
        await new Promise((r) => setTimeout(r, 0));
        self.render();
      }
      return self.tree;
    },
  };
  self.render();
  return self;
}

// ── the fixture ─────────────────────────────────────────────────────────────
const PATH = "/w/readme.md";
const DISK = "# hello\n\nworld\n";
const BASE_MTIME = 1000;

/** A backend whose every answer is explicit and recorded. */
function backend({ content = DISK, mtime = BASE_MTIME, truncated = false } = {}) {
  const calls = [];
  let file = { content, mtime, truncated };
  let held = null; // a save the test is holding open
  const api = {
    calls,
    file,
    hold() { return new Promise((res, rej) => { held = { res, rej }; }); },
    /** finish the save that is in flight */
    ack(newMtime) { held.res(newMtime); held = null; },
    reject(msg) { held.rej(msg); held = null; },
    invoke(cmd, args) {
      calls.push({ cmd, args });
      if (cmd === "read_file")
        return Promise.resolve({ content: file.content, truncated: file.truncated, binary: false, mtime: file.mtime, size: file.content.length });
      if (cmd === "write_file") {
        if (held !== null) throw new Error("two saves in flight — the test did not mean that");
        return new Promise((res, rej) => { held = { res, rej }; });
      }
      return Promise.resolve(null);
    },
  };
  return api;
}

const props = (over = {}) => ({
  path: PATH, reloadToken: 0,
  onOpenEditor: () => {}, onOpen: () => {}, onError: () => {},
  wrap: false, onWrap: () => {},
  mdSource: true, onMdSource: () => {},
  diff: false, onDiff: () => {},
  diffBase: "base", onDiffBase: () => {},
  mdZoom: 100, onMdZoom: () => {},
  expanded: false, onExpand: () => {},
  findOpen: false, findToken: 0, onFindClose: () => {},
  ...over,
});

const editorOf = (tree) => tags(tree, "textarea")[0] ?? null;
const saveBtn = (tree) => tags(tree, "button").find((b) => textOf(b).trim() === "Save" || textOf(b).trim() === "Saving…");
const named = (tree, label) => tags(tree, "button").find((b) => textOf(b).trim() === label);

// ── 0. the Source view of a markdown file is typeable ───────────────────────
{
  const be = backend();
  const m = mount(props(), { invoke: be.invoke });
  await m.settle();
  ok(editorOf(m.tree), "markdown Source is not editable — no <textarea> in the body (this is the whole feature)");
  eq(editorOf(m.tree)?.props.value, DISK, "the editor opens on the file's contents");
  ok(saveBtn(m.tree), "no Save control in the header");
  eq(saveBtn(m.tree)?.props.disabled, true, "Save is enabled on a clean buffer");

  // …and Preview is still the renderer, not a second editor
  const prev = mount(props({ mdSource: false }), { invoke: backend().invoke });
  await prev.settle();
  ok(!editorOf(prev.tree), "Preview grew a textarea — only Source is editable");
  ok(tags(prev.tree, "x-stub-Markdown").length === 1, "Preview no longer renders Markdown");
}

// ── 1. a truncated read is never editable ───────────────────────────────────
// The one rule here that can destroy data: the buffer holds the first 1 MiB,
// so saving it would delete the rest of the file.
{
  const be = backend({ truncated: true });
  const m = mount(props(), { invoke: be.invoke });
  await m.settle();
  ok(!editorOf(m.tree), "a TRUNCATED markdown file is editable — saving it would delete everything past the 1 MiB cap");
  ok(!saveBtn(m.tree), "a truncated file offers Save");
}

// A code file keeps the read-only renderer (the narrowness is deliberate —
// a .rs textarea would lose highlighting, the gutter and ⌘F's painting).
{
  const be = backend();
  const m = mount(props({ path: "/w/lib.rs" }), { invoke: be.invoke });
  await m.settle();
  ok(!editorOf(m.tree), "a .rs file is editable — the exception was supposed to be markdown only");
  ok(tags(m.tree, "x-stub-CodeBlock").length === 1, "a .rs file no longer renders CodeBlock");
}

// ── 2. save carries the base mtime, and the ack re-bases the next save ──────
{
  const be = backend();
  const m = mount(props(), { invoke: be.invoke });
  await m.settle();

  m.act(() => editorOf(m.tree).props.onChange({ currentTarget: { value: "# hello\n\nworld!\n" } }));
  ok(m.drafts.get(PATH), "typing left no draft");
  eq(saveBtn(m.tree)?.props.disabled, false, "Save is still disabled after an edit");
  ok(named(m.tree, "Discard"), "no Discard control beside a dirty buffer");
  ok(walk(m.tree).some((n) => textOf(n).trim() === "unsaved"), "the header does not say the buffer is unsaved");

  m.act(() => saveBtn(m.tree).props.onClick());
  await new Promise((r) => setTimeout(r, 0));
  const first = be.calls.filter((c) => c.cmd === "write_file").pop();
  eq(first.args.expectedMtime, BASE_MTIME, "the save did not carry the mtime the edit started from");
  eq(first.args.content, "# hello\n\nworld!\n", "the save did not carry what was typed");

  be.ack(2000); // the backend's NEW mtime for the file it just wrote
  await m.settle();
  ok(!m.drafts.get(PATH), "a saved draft was not cleared");
  ok(!walk(m.tree).some((n) => textOf(n).trim() === "unsaved"), "the header still says unsaved after a successful save");

  // …and the second ⌘S of the sitting must not be refused as a conflict with
  // the first. This is the assertion `write_file`'s return value exists for.
  m.act(() => editorOf(m.tree).props.onChange({ currentTarget: { value: "# hello\n\nagain\n" } }));
  m.act(() => saveBtn(m.tree).props.onClick());
  await new Promise((r) => setTimeout(r, 0));
  const second = be.calls.filter((c) => c.cmd === "write_file").pop();
  eq(second.args.expectedMtime, 2000, "the second save re-sent the FIRST save's mtime — every save after the first would be refused as a conflict with itself");
  be.ack(3000);
  await m.settle();
}

// ── 3. a keystroke during an in-flight save is not thrown away ──────────────
{
  const be = backend();
  const m = mount(props(), { invoke: be.invoke });
  await m.settle();
  m.act(() => editorOf(m.tree).props.onChange({ currentTarget: { value: "one\n" } }));
  m.act(() => saveBtn(m.tree).props.onClick());
  await new Promise((r) => setTimeout(r, 0));
  // …the user keeps typing while the write is in flight
  m.act(() => editorOf(m.tree).props.onChange({ currentTarget: { value: "one two\n" } }));
  be.ack(2000);
  await m.settle();
  eq(m.drafts.get(PATH)?.text, "one two\n", "the save cleared a draft it never sent — typing during a write is lost");
  eq(editorOf(m.tree)?.props.value, "one two\n", "the editor lost what was typed during the save");
}

// ── 4. a refusal offers Overwrite, and only Overwrite drops the guard ───────
{
  const be = backend();
  const errors = [];
  const m = mount(props({ onError: (e) => errors.push(String(e)) }), { invoke: be.invoke });
  await m.settle();
  m.act(() => editorOf(m.tree).props.onChange({ currentTarget: { value: "mine\n" } }));

  ok(!named(m.tree, "Overwrite"), "Overwrite is offered BEFORE anything was refused — the one control that can clobber Claude's write must not be reachable by accident");

  m.act(() => saveBtn(m.tree).props.onClick());
  await new Promise((r) => setTimeout(r, 0));
  be.reject("file changed on disk since you opened it — reload to see the latest");
  await m.settle();

  ok(errors.length === 1, "a refused save was swallowed — it never reached onError (toast + app.log)");
  ok(walk(m.tree).some((n) => textOf(n).trim() === "save refused"), "the header does not show that the save was refused");
  eq(m.drafts.get(PATH)?.text, "mine\n", "a refused save threw the edit away");
  const ow = named(m.tree, "Overwrite");
  ok(ow, "no Overwrite control after a refusal — the edit is trapped");

  m.act(() => ow.props.onClick());
  await new Promise((r) => setTimeout(r, 0));
  const forced = be.calls.filter((c) => c.cmd === "write_file").pop();
  eq(forced.args.expectedMtime, null, "Overwrite kept the compare-and-swap, so it cannot do what it says");
  be.ack(4000);
  await m.settle();
  ok(!m.drafts.get(PATH), "the forced save left the draft behind");
}

// ── 5. one draft, however many views are mounted ────────────────────────────
// App mounts the dock's viewer and the reading overlay at the same time. Two
// component-held buffers would fork into two answers to "what have I typed".
{
  const a = mount(props(), { invoke: backend().invoke });
  const b = mount(props(), { invoke: backend().invoke });
  await a.settle();
  await b.settle();
  a.act(() => editorOf(a.tree).props.onChange({ currentTarget: { value: "typed in the dock\n" } }));
  b.render();
  eq(editorOf(b.tree)?.props.value, "typed in the dock\n", "the reading overlay did not see what was typed in the dock — the draft is not shared");
  a.putDraft(PATH, null);
}

// ── 6. Find suspends editing rather than searching a textarea ──────────────
// ⌘F paints through the CSS Custom Highlight API, which cannot reach inside a
// textarea. An open Find falls back to the renderer — showing the DRAFT.
{
  const be = backend();
  const m = mount(props({ findOpen: true }), { invoke: be.invoke });
  await m.settle();
  ok(!editorOf(m.tree), "the editor is still a textarea with Find open — every match would go unpainted");
  ok(tags(m.tree, "x-stub-CodeBlock").length === 1, "Find did not fall back to the read-only renderer");
  ok(saveBtn(m.tree), "Save disappeared while Find is open — a dirty buffer would have no way to land");
}

// ── 7. the editor itself: ⌘S saves, and ⌥ does not ──────────────────────────
{
  const be = backend();
  const m = mount(props(), { invoke: be.invoke });
  await m.settle();
  let saved = 0;
  const key = (o) => {
    let prevented = false;
    return { metaKey: false, ctrlKey: false, altKey: false, key: "", preventDefault: () => { prevented = true; }, stopPropagation: () => {}, get prevented() { return prevented; }, ...o };
  };
  const el = SourceEditor({ text: "x", wrap: false, onChange: () => {}, onSave: () => { saved++; } });
  const press = (o) => { const e = key(o); el.props.onKeyDown(e); return e; };
  press({ metaKey: true, key: "s" });
  eq(saved, 1, "⌘S in the editor did not save");
  press({ ctrlKey: true, key: "S" });
  eq(saved, 2, "Ctrl+S / ⇧ variant did not save");
  press({ key: "s" });
  eq(saved, 2, "a bare `s` saved the file — that is a character, not a chord");
  press({ metaKey: true, altKey: true, key: "s" });
  eq(saved, 2, "⌘⌥S saved — that is a different chord");
  // `wrap="off"` is what makes a textarea scroll instead of soft-wrapping; the
  // CSS white-space alone does not reach the control.
  eq(el.props.wrap, "off", "the unwrapped editor soft-wraps anyway");
  eq(SourceEditor({ text: "x", wrap: true, onChange: () => {}, onSave: () => {} }).props.wrap, "soft", "Wrap does not reach the editor");
}

if (bad) {
  console.error(`\nmdedit-check: ${bad} failure(s)`);
  process.exit(1);
}
console.log("mdedit-check: ok");
