// Deterministic check of the docs viewer's client: the poll loop's routing and
// caching, the markdown block list, the mermaid click-directive rule, and the
// two places the page publishes a number the CSS depends on.
//
//   node app/scripts/docsviewer-check.mjs      # exits non-zero on failure
//
// WHY THIS FILE EXISTS. Nothing sliced `Viewer.tsx` or `blocks.tsx`, and that
// is exactly how the viewer shipped with "navigating back to a document you
// already read says `loading …` forever". Every gate was green: `tsc` has
// nothing to say about it, the Rust tests cover the server's 304 (which was
// CORRECT), the mock harness does not run this bundle at all, and the bug needs
// three navigations and a conditional request to appear. It is a state machine
// spread across a ref, a `useEffect` and a render guard — the shape this repo
// keeps writing `*-check.mjs` for.
//
// Like race-check.mjs and ctxmenu-check.mjs, it does NOT paraphrase the
// component. It strips the import lines out of the REAL source files, evaluates
// what is left under stubs, and drives it. Running it before and after an edit
// tests the edit itself, and it fails on the pre-fix sources — which is the
// only evidence that a new check is worth anything.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { marked } from "marked";
// vite's esbuild re-export — bare "esbuild" does not resolve under pnpm's
// strict layout, vite does (a direct dependency). Same reason as race-check.
import { transformWithEsbuild } from "vite";

const here = (p) => fileURLToPath(new URL(p, import.meta.url));
const VIEWER = process.argv[2] || here("../viewer/Viewer.tsx");
const BLOCKS = process.argv[3] || here("../viewer/blocks.tsx");
const MERMAID = process.argv[4] || here("../viewer/Mermaid.tsx");
const CHROME = process.argv[5] || here("../viewer/Chrome.tsx");
const INDEXVIEW = process.argv[6] || here("../viewer/IndexView.tsx");
const DOCSNAV = process.argv[7] || here("../viewer/DocsNav.tsx");
const CSS = process.argv[8] || here("../viewer/viewer.css");
// The OTHER half of the click contract, in Rust. Read, never written.
const DERIVE = process.argv[9] || here("../../crates/worktrees-core/src/derive.rs");

let failed = 0;
const fail = (msg) => { failed++; console.log(`not ok — ${msg}`); };
const ok = (msg) => console.log(`ok — ${msg}`);
/**
 * Evaluate a factory, reporting a missing binding as a FAILED ASSERTION rather
 * than an exception. A source that has not grown the function this file is
 * about (running it against the pre-fix tree, for one) otherwise aborts every
 * later section and buries the finding under a base64 data-URL stack trace.
 */
const safeBuild = (label, build, env) => {
  try {
    return build(env);
  } catch (e) {
    fail(`${label} could not be evaluated — ${String(e).split("\n")[0]}`);
    return null;
  }
};
const is = (got, want, msg) =>
  (JSON.stringify(got) === JSON.stringify(want) ? ok(msg) : fail(`${msg}\n     want ${JSON.stringify(want)}\n     got  ${JSON.stringify(got)}`));

// ── loading a real source file as a factory ──────────────────────────────────
/**
 * Strip the import statements (multi-line ones included — `Viewer.tsx` has a
 * four-line one) and the `export` keywords, wrap what is left in a factory that
 * receives every stripped binding through `env`, and evaluate it.
 *
 * Nothing between those two edits is touched, which is the whole point: the
 * code under test is the code that ships.
 */
async function load(path, bindings, returns) {
  const raw = fs.readFileSync(path, "utf8");
  const body = raw
    .replace(/^import\s[\s\S]*?\bfrom\s+"[^"]+";[^\n]*$/gm, "")
    // A bare `export type { X };` re-export would become `type { X };`, which
    // is a syntax error rather than a stripped keyword.
    .replace(/^export (type )?\{[^}]*\};[^\n]*$/gm, "")
    .replace(/^export (function|type|const|class|async)/gm, "$1");
  const src = `
export function build(env: any) {
  const { ${bindings.join(", ")} } = env;
${body}
  return { ${returns.join(", ")} };
}`;
  const js = (await transformWithEsbuild(src, "docsviewer-check.tsx", {
    loader: "tsx", format: "esm", jsx: "transform", jsxFactory: "h", jsxFragment: "F",
  })).code;
  const mod = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));
  return mod.build;
}

// ── a minimal React ──────────────────────────────────────────────────────────
// Hooks in slots, effects flushed after each render, a setState that re-renders
// until quiescent. Enough to run a component that polls; not enough to be
// mistaken for React, which is why every assertion below is about data the
// component produced rather than about anything the runtime decided.
const sameDeps = (a, b) =>
  Array.isArray(a) && Array.isArray(b) && a.length === b.length && a.every((x, i) => Object.is(x, b[i]));

function runtime(render) {
  const slots = [];
  const cleanups = [];
  const fns = [];
  let i = 0, pending = [], dirty = false, inFlush = false, tree = null;

  const schedule = () => { dirty = true; if (!inFlush) flush(); };
  const hooks = {
    useState(init) {
      const k = i++;
      if (slots[k] === undefined) slots[k] = { v: typeof init === "function" ? init() : init };
      const s = slots[k];
      return [s.v, (next) => {
        const v = typeof next === "function" ? next(s.v) : next;
        if (Object.is(v, s.v)) return;   // React bails out on an identical value
        s.v = v;
        schedule();
      }];
    },
    useRef(init) {
      const k = i++;
      if (slots[k] === undefined) slots[k] = { current: init };
      return slots[k];
    },
    useMemo(fn, deps) {
      const k = i++;
      const s = slots[k];
      if (s === undefined || !sameDeps(s.deps, deps)) slots[k] = { deps, v: fn() };
      return slots[k].v;
    },
    useCallback(fn, deps) { return hooks.useMemo(() => fn, deps); },
    useEffect(fn, deps) {
      const k = i++;
      const s = slots[k];
      if (s !== undefined && sameDeps(s.deps, deps)) return;
      slots[k] = { deps };
      fns[k] = fn;
      pending.push(k);
    },
    useLayoutEffect(fn, deps) { hooks.useEffect(fn, deps); },
  };

  function flush() {
    inFlush = true;
    let guard = 0;
    do {
      if (++guard > 50) throw new Error("render loop did not settle — a setState is firing every render");
      dirty = false;
      i = 0;
      tree = render(hooks);
      const todo = pending;
      pending = [];
      for (const k of todo) {
        if (cleanups[k]) cleanups[k]();
        const c = fns[k]();
        cleanups[k] = typeof c === "function" ? c : null;
      }
    } while (dirty);
    inFlush = false;
  }

  return { hooks, flush, schedule, tree: () => tree, unmount: () => cleanups.forEach((c) => c && c()) };
}

// ── a JSX recorder ───────────────────────────────────────────────────────────
const h = (tag, props, ...kids) => ({ tag, props: props ?? {}, kids: kids.flat(Infinity).filter((k) => k != null) });
const F = "fragment";
/** Every node in a recorded tree, depth first. Strings and numbers included. */
function* walk(node) {
  if (node == null || node === false) return;
  if (typeof node !== "object") { yield node; return; }
  yield node;
  for (const k of node.kids ?? []) yield* walk(k);
  // A node passed as a prop (the Viewer hands `nav` to Chrome that way) is
  // still on screen, so it is part of the tree.
  for (const v of Object.values(node.props ?? {})) if (v && typeof v === "object" && "tag" in v) yield* walk(v);
}
const find = (tree, tag) => [...walk(tree)].find((n) => n && typeof n === "object" && n.tag === tag);
const findClass = (tree, cls) =>
  [...walk(tree)].find((n) => n && typeof n === "object" && n.props?.className === cls);
const textOf = (node) => (node ? [...walk(node)].filter((n) => typeof n === "string" || typeof n === "number").join("") : "");

console.log("── the poll loop: routing, the payload cache and the banner ──");

// ── the viewer harness ───────────────────────────────────────────────────────
const buildViewer = await load(
  VIEWER,
  ["useCallback", "useEffect", "useMemo", "useRef", "useState",
   "Chrome", "DocBody", "DocsNav", "IndexView",
   "apiBase", "basename", "fetchDoc", "fetchIndex", "h", "F"],
  ["Viewer", "parseRoute", "routeHash"],
);

const meta = {
  place: "live-docs", branch: "main", behind: 0, base: "origin/main",
  dirty: 0, subject: "x", derived_epoch: 1, status_epoch: 1,
};
const payload = (path, text) => ({ meta, blocks: [{ id: "b0", md: `# ${path}\n\n${text}` }] });

function mountViewer(startHash) {
  const realSetTimeout = globalThis.setTimeout;
  const prev = {
    window: globalThis.window, document: globalThis.document, location: globalThis.location,
    setTimeout: globalThis.setTimeout, clearTimeout: globalThis.clearTimeout,
  };
  const listeners = {};
  let hash = startHash;
  // Keyed by id, because the component CANCELS one on every route change
  // (`clearTimeout` in the effect's cleanup) and a harness that ignored that
  // would later fire a poll belonging to a document nobody is looking at.
  const timers = new Map();
  let tid = 0;
  const requests = [];               // every fetchDoc call, oldest first

  globalThis.location = {
    href: "http://127.0.0.1:6391/tok/p/live-docs/",
    get hash() { return hash; },
    set hash(v) {
      if (v === hash) return;
      hash = v;
      for (const fn of listeners.hashchange ?? []) fn();
    },
  };
  globalThis.window = {
    addEventListener: (t, fn) => { (listeners[t] ||= []).push(fn); },
    removeEventListener: (t, fn) => {
      const a = listeners[t]; if (!a) return;
      const j = a.indexOf(fn); if (j >= 0) a.splice(j, 1);
    },
    scrollTo: () => {}, scrollBy: () => {},
  };
  globalThis.document = {
    title: "",
    documentElement: { dataset: {}, style: { setProperty: () => {} } },
    getElementById: () => null,
  };
  // Captured, never run on their own: the chained `setTimeout(tick, POLL_MS)`
  // is the poll, and a check that let it run would race its own assertions.
  globalThis.setTimeout = (fn) => { timers.set(++tid, fn); return tid; };
  globalThis.clearTimeout = (id) => { timers.delete(id); };

  let indexServed = false;
  const env = {
    Chrome: "Chrome", DocBody: "DocBody", DocsNav: "DocsNav", IndexView: "IndexView",
    apiBase: () => new URL("http://127.0.0.1:6391/tok/p/live-docs/"),
    basename: (p) => p.split("/").filter(Boolean).pop() ?? p,
    fetchDoc: (_base, path, etag) => {
      const rec = { path, etag };
      rec.p = new Promise((r) => { rec.settle = r; });
      requests.push(rec);
      return rec.p;
    },
    fetchIndex: () => {
      if (indexServed) return Promise.resolve({ kind: "same" });
      indexServed = true;
      return Promise.resolve({
        kind: "data", etag: '"i1"',
        data: { meta, entries: [{ path: "docs/a.md", title: "A", group: "docs" }] },
      });
    },
    h, F,
  };

  const rt = runtime(() => api.Viewer());
  Object.assign(env, rt.hooks);
  const api = buildViewer(env);
  rt.flush();

  return {
    api,
    tree: rt.tree,
    /** The document currently painted, or null when the page says "loading". */
    shown: () => find(rt.tree(), "DocBody")?.props?.blocks?.[0]?.md ?? null,
    loading: () => textOf(findClass(rt.tree(), "loading")) || null,
    banner: () => textOf(findClass(rt.tree(), "err")) || null,
    gone: () => Boolean(findClass(rt.tree(), "gone")),
    go: (h2) => { globalThis.location.hash = h2; },
    /** Answer the LATEST unanswered `doc` request — the one the route that is
     *  on screen is waiting for. Requests abandoned by a navigation stay
     *  unanswered, exactly as an in-flight fetch does when you click away. */
    answer: async (r) => {
      const rec = [...requests].reverse().find((x) => !x.answered);
      if (!rec) throw new Error("nothing asked for a document");
      rec.answered = true;
      rec.settle(r);
      for (let n = 0; n < 8; n++) await new Promise((res) => realSetTimeout(res, 0));
      return rec;
    },
    /** What the latest unanswered request carried — the conditional-GET half. */
    pending: () => [...requests].reverse().find((x) => !x.answered) ?? null,
    requests: () => requests,
    tick: () => { const t = [...timers.values()]; timers.clear(); for (const fn of t) fn(); },
    restore: () => {
      rt.unmount();
      Object.assign(globalThis, prev);
    },
  };
}

// 1 ── a document arrives and is rendered.
{
  const v = mountViewer("#/docs/a.md");
  const first = v.pending();
  is(first?.path, "docs/a.md", "the route's path is what the first request asks for");
  is(first?.etag, null, "the first request for an uncached document is unconditional");
  is(v.shown(), null, "before the answer, the page says loading");
  await v.answer({ kind: "data", etag: '"a1"', data: payload("docs/a.md", "alpha") });
  is(v.shown(), "# docs/a.md\n\nalpha", "the document renders once the payload arrives");
  is(v.banner(), null, "no banner on a good response");

  // 2 ── THE BUG. A → B → A, where the server correctly answers the third
  // request with 304 because nothing in the place changed.
  v.go("#/docs/b.md");
  is(v.shown(), null, "switching to an unvisited document shows loading (nothing is cached)");
  await v.answer({ kind: "data", etag: '"b1"', data: payload("docs/b.md", "beta") });
  is(v.shown(), "# docs/b.md\n\nbeta", "the second document renders");

  v.go("#/docs/a.md");
  is(
    v.shown(), "# docs/a.md\n\nalpha",
    "BACK TO A RENDERS A IMMEDIATELY, from the cache, before any request is answered\n"
    + "     (pre-fix: the single `doc` slot still held B, so this was `loading docs/a.md…`)",
  );
  const back = v.pending();
  is(back?.path, "docs/a.md", "returning to A re-asks for A");
  is(back?.etag, '"a1"', "and it asks CONDITIONALLY — the cached ETag went back out");
  await v.answer({ kind: "same" });
  is(
    v.shown(), "# docs/a.md\n\nalpha",
    "A IS STILL ON SCREEN AFTER THE 304\n"
    + "     (pre-fix: the 304 branch wrote nothing and the page said `loading docs/a.md…` at 1 Hz, forever)",
  );
  is(v.loading(), null, "and nothing anywhere is claiming to be loading");

  // 3 ── the banner clears when the server comes back, which is a 304.
  v.tick();
  await v.answer({ kind: "error", message: "cannot reach the docs server — TypeError" });
  is(v.banner(), "cannot reach the docs server — TypeError", "a failed poll raises the banner");
  is(v.shown(), "# docs/a.md\n\nalpha", "and the document already on screen stays on screen");
  v.tick();
  await v.answer({ kind: "same" });
  is(
    v.banner(), null,
    "A 304 AFTER A FAILURE CLEARS THE BANNER\n"
    + "     (pre-fix: only a 200 cleared it, and a recovery is a 304 — the red band never left)",
  );

  // 4 ── a fresh document still updates in place.
  v.tick();
  await v.answer({ kind: "data", etag: '"a2"', data: payload("docs/a.md", "alpha, edited") });
  is(v.shown(), "# docs/a.md\n\nalpha, edited", "an edit to the open document still lands");
  v.go("#/docs/b.md");
  is(v.shown(), "# docs/b.md\n\nbeta", "and B is still cached too");
  v.go("#/docs/a.md");
  is(v.shown(), "# docs/a.md\n\nalpha, edited", "…while A now shows the EDITED text, not the stale cached one");

  // 5 ── 410 replaces the page.
  v.tick();
  await v.answer({ kind: "gone" });
  is(v.gone(), true, "410 Gone replaces the whole page");
  v.restore();
}

// 6 ── the router survives a `%` that is not an escape.
{
  const v = mountViewer("#/");
  const { parseRoute, routeHash } = v.api;
  let threw = null;
  try { parseRoute("#/100%.md"); } catch (e) { threw = String(e); }
  is(
    threw, null,
    "parseRoute does not throw on a stray `%`\n"
    + "     (pre-fix: URIError out of a useState initialiser — a blank page, and a frozen route from the listener)",
  );
  is(parseRoute("#/docs/a.md"), { kind: "doc", path: "docs/a.md", anchor: null }, "a plain fragment route parses");
  is(parseRoute("#/docs/a.md?h=install"), { kind: "doc", path: "docs/a.md", anchor: "install" }, "…and its `?h=` anchor");
  is(parseRoute("#"), { kind: "index" }, "a bare hash is the index");
  is(
    parseRoute(routeHash({ kind: "doc", path: "100%.md", anchor: null })),
    { kind: "doc", path: "100%.md", anchor: null },
    "routeHash → parseRoute round-trips a name with a `%` in it",
  );
  v.restore();
}

console.log("── the mermaid click rule ──");

// ── stripClickDirectives ─────────────────────────────────────────────────────
{
  const build = await load(
    MERMAID,
    ["memo", "useEffect", "useRef", "useState", "mermaid", "h", "F"],
    ["stripClickDirectives"],
  );
  const built = safeBuild("Mermaid.tsx", build, {
    memo: (f) => f, useEffect: () => {}, useRef: () => ({ current: null }),
    useState: (v) => [v, () => {}], mermaid: { initialize: () => {}, render: () => {} }, h, F,
  });
  const { stripClickDirectives } = built ?? { stripClickDirectives: () => ({ src: "", stripped: -1 }) };

  const run = (lines) => stripClickDirectives(lines.join("\n"));

  // The tool's own shape — a SAME-PAGE fragment, which is what the drill-down
  // emits now that the loopback URL is gone.
  const toolOnly = run([
    "flowchart TD",
    "  A[docs/a.md] --> B[docs/b.md]",
    '  click A href "#/docs/a.md"',
    '  click B href "#/docs/adr/0001-one%20engine.md"',
  ]);
  is(
    toolOnly.stripped, 0,
    "a tool-shaped `click X href \"#/<rel>\"` is NOT stripped\n"
    + "     (pre-fix: every click line went, so the drill-down was dead)",
  );
  is(
    toolOnly.src.split("\n").filter((l) => l.includes("click")).length, 2,
    "…and both of them are still in the source handed to mermaid",
  );

  const authored = run([
    "flowchart TD",
    '  click A "javascript:alert(1)"',
    '  click B href "http://evil.example/steal"',
    "  click C call danger()",
    '  click D href "#not-a-route"',
    '  click E href "#/ok.md" %% trailing directive',
    '  click F href "#/a.md" ; click G href "http://evil"',
    "\tclick H href \"#/tab-indented.md\"",
  ]);
  is(authored.stripped, 6, "every author shape is still removed, including the near misses");
  is(
    authored.src.split("\n").filter((l) => l.includes("click")).map((l) => l.trim()),
    ['click H href "#/tab-indented.md"'],
    "only the exactly-shaped line survives — leading whitespace is fine, a trailing anything is not",
  );

  // The count drives a note that tells the reader an author was overruled, so
  // it has to be what was actually removed.
  const mixed = run([
    "flowchart TD",
    '  click A href "#/docs/a.md"',
    '  click B href "https://evil.example"',
  ]);
  is(mixed.stripped, 1, "the `N author click directives removed` note counts only what WAS removed");

  // And the survivor has to be a route the page reads. `svgFromString` keeps an
  // independent rule that an href must start with `#`; what makes `#/…` useful
  // is that changing `location.hash` is exactly what the router listens to.
  const v = mountViewer("#/");
  const target = /click\s+\S+\s+href\s+"([^"]+)"/.exec(toolOnly.src)?.[1];
  is(target, "#/docs/a.md", "the surviving href is a fragment");
  is(
    // `?? "#"` so a source that stripped the directive reports a failed
    // assertion rather than crashing the rest of the file.
    v.api.parseRoute(target ?? "#"),
    { kind: "doc", path: "docs/a.md", anchor: null },
    "…and the page's own router resolves it to that document, so clicking it navigates",
  );
  is(
    v.api.parseRoute('#/docs/adr/0001-one%20engine.md'),
    { kind: "doc", path: "docs/adr/0001-one engine.md", anchor: null },
    "…escaped segments included",
  );
  // Not just parsed — NAVIGATED. Clicking an `<a href="#/x">` inside the SVG
  // sets `location.hash`, and the router's `hashchange` listener is what turns
  // that into a route. This drives the same event on the mounted component.
  v.go(target ?? "#");
  is(
    v.tree()?.props?.className, "shell",
    "the page is still the shell after following the fragment (it did not fall over)",
  );
  is(
    textOf(findClass(v.tree(), "nav-path")), "docs/a.md",
    "…and the header now names the drilled-into document, so the fragment really did navigate",
  );
  v.restore();

  // ── the contract's other half ──────────────────────────────────────────────
  // `derive.rs` writes the directive; `stripClickDirectives` decides whether it
  // survives. Two files, two languages, one shape — the drift-check pattern this
  // repo uses for `dnd.ts::predictTier` and for `place_url` vs `routeHash`.
  const rust = fs.readFileSync(DERIVE, "utf8");
  const emit = /format!\("([^"\\]*(?:\\.[^"\\]*)*click[^"\\]*(?:\\.[^"\\]*)*)"\)/.exec(rust)?.[1];
  if (!emit) {
    fail("no `format!(\"…click…\")` in derive.rs — that is the line this strip rule exists to spare.\n"
      + "     If the emitter moved, re-point this check at it; the two halves must not drift apart.");
  } else {
    const line = emit
      .replace(/\\"/g, '"')
      .replace("{indent}", "  ")
      .replace("{id}", "Node_1")
      .replace("{url}", "#/docs/adr/0001.md");
    const r = stripClickDirectives(`flowchart TD\n${line}`);
    is(
      r.stripped, 0,
      `the line derive.rs actually emits survives this strip rule\n     (emitted: ${JSON.stringify(line)})`,
    );
  }
}

console.log("── the block list: cross-block link definitions ──");

// ── blocks.tsx ───────────────────────────────────────────────────────────────
{
  const build = await load(
    BLOCKS,
    ["Component", "memo", "useLayoutEffect", "useMemo", "useRef", "marked",
     "Markdown", "Mermaid", "onLink", "renderImage", "sanitizeHtml", "h", "F"],
    ["DocBody", "collectDefs", "blockKeys", "mermaidSource"],
  );
  const rt = runtime(() => null);
  const api = safeBuild("blocks.tsx", build, {
    Component: class { constructor(p) { this.props = p; } },
    memo: (f) => f,
    marked,
    Markdown: "Markdown", Mermaid: "Mermaid",
    onLink: () => {}, renderImage: () => null, sanitizeHtml: () => null,
    h, F,
    ...rt.hooks,
  });
  if (!api) {
    fail("blocks.tsx has no `collectDefs` — nothing collects a document's link definitions across blocks,\n"
      + "     so `[ci][badge]` in one block cannot see `[badge]: …` in another and renders as literal brackets");
  } else {

  // A README's badge row: the reference and the definition are separate blocks,
  // which is exactly how the server splits a document.
  const blocks = [
    { id: "b0", md: "# Title" },
    { id: "b1", md: "[![ci][badge]][runs] and [the docs][d]." },
    { id: "b2", md: "Some prose in between." },
    { id: "b3", md: '[badge]: https://img.example/ci.svg "CI"\n[runs]: https://ci.example/runs\n[d]: docs/a.md' },
    { id: "b4", md: "```mermaid\nflowchart TD\n  A --> B\n```" },
  ];

  const defs = api.collectDefs(blocks);
  is(defs.split("\n").length, 3, "collectDefs finds every definition in the document");
  is(
    defs.includes('[badge]: https://img.example/ci.svg "CI"'), true,
    "…keeping the title, which marked gives back unwrapped",
  );

  // Render for real and read what each block was actually handed.
  const srcOf = (tree) => {
    const out = [];
    for (const n of walk(tree)) {
      // `dataKey` identifies BlockView; the boundary wrapping it is a CLASS,
      // and calling a class without `new` throws.
      if (n && typeof n === "object" && typeof n.tag === "function" && n.props?.dataKey !== undefined) {
        const inner = n.tag(n.props);           // BlockView, un-memo'd above
        const md = find(inner, "Markdown");
        out.push(md ? md.props.src : { diagram: Boolean(find(inner, "Mermaid")) });
      }
    }
    return out;
  };
  const shown = srcOf(api.DocBody({ blocks, ctx: { base: new URL("http://x/"), dir: "" } }));
  is(shown.length, 5, "every block is rendered");
  is(
    shown[1].includes("[badge]: https://img.example/ci.svg"), true,
    "THE DEFINITIONS ARE APPENDED TO THE BLOCK THAT REFERENCES THEM\n"
    + "     (pre-fix: each block was lexed alone, so `[ci][badge]` rendered as literal brackets)",
  );
  is(shown[1].startsWith("[![ci][badge]][runs] and [the docs][d]."), true, "…after the block's own text, not before it");
  is(shown[4], { diagram: true }, "A MERMAID BLOCK IS STILL A DIAGRAM — the defs must not reach it, or one token becomes two");

  // The proof that the appending is what makes the reference resolve, using the
  // same lexer the renderer uses.
  const html = (md) => marked.parse(md, { gfm: true, async: false });
  is(html(blocks[1].md).includes("<a href="), false, "the referencing block alone produces no link at all");
  is(html(shown[1]).includes('href="https://ci.example/runs"'), true, "…and with the defs appended it produces the link");

  // Keys are the reconciler's identity. Folding the defs into them would
  // re-key — and so replace the DOM of — every block in the document whenever
  // one unrelated definition line is edited, which is the selection-and-scroll
  // churn blocks.tsx exists to prevent.
  const edited = blocks.map((b) => (b.id === "b3" ? { ...b, md: `${b.md}\n[extra]: docs/b.md` } : b));
  const before = api.blockKeys(blocks);
  const after = api.blockKeys(edited);
  is(
    before.filter((k, j) => k !== after[j]).length, 1,
    "editing one definition block changes exactly ONE key — blockKeys stays on the ORIGINAL md",
  );
  const shownAfter = srcOf(api.DocBody({ blocks: edited, ctx: { base: new URL("http://x/"), dir: "" } }));
  is(shownAfter[1] !== shown[1], true, "…while what the referencing block RENDERS does follow the new definition");
  }
}

console.log("── the sticky header and the anchors under it ──");

// ── `--chrome-h` and scroll-margin-top ───────────────────────────────────────
{
  const build = await load(
    CHROME, ["useCallback", "useEffect", "useRef", "useState", "h", "F"], ["usePublishedHeight"],
  );
  const prevDoc = globalThis.document;
  const prevRO = globalThis.ResizeObserver;
  const set = {};
  let observed = null, disconnected = 0, onResize = null;
  globalThis.document = { documentElement: { style: { setProperty: (k, v) => { set[k] = v; } } } };
  globalThis.ResizeObserver = class {
    constructor(cb) { onResize = cb; }
    observe(n) { observed = n; }
    disconnect() { disconnected++; }
  };
  const chrome = safeBuild("Chrome.tsx", build, {
    useCallback: (f) => f, useEffect: () => {}, useRef: (v) => ({ current: v }),
    useState: (v) => [v, () => {}], h, F,
  });
  const ref = chrome?.usePublishedHeight?.() ?? (() => {});
  let height = 71.4;
  const node = { getBoundingClientRect: () => ({ height }) };
  ref(node);
  is(set["--chrome-h"], "71px", "the header publishes its MEASURED height as --chrome-h");
  is(observed, node, "…and observes itself, so a wrapped subject or a split timestamp re-publishes it");
  // The band grows a row (the timestamps split apart, a long subject wraps).
  height = 96;
  onResize?.();
  is(set["--chrome-h"], "96px", "…and a re-measure re-publishes it, so the anchor offset follows the band");
  ref(null);
  is(disconnected > 0, true, "detaching disconnects the observer");
  globalThis.document = prevDoc;
  globalThis.ResizeObserver = prevRO;

  const css = fs.readFileSync(CSS, "utf8");
  const rule = /\.doc\s*\[id\]\s*\{([^}]*)\}/.exec(css)?.[1] ?? "";
  if (!/scroll-margin-top/.test(rule)) {
    fail("no `scroll-margin-top` on `.doc [id]` in viewer.css — every `[x](#section)` lands under the sticky .chrome");
  } else if (!/--chrome-h/.test(rule)) {
    fail("`.doc [id]`'s scroll-margin-top does not use --chrome-h — a constant is wrong the moment the band rewraps");
  } else {
    ok("an anchored heading clears the sticky header, by the header's own measured height");
  }
}

console.log("── one filter shortcut, and the index's order ──");

// ── the `/` handler, registered once ─────────────────────────────────────────
{
  const files = { Viewer: VIEWER, IndexView: INDEXVIEW, DocsNav: DOCSNAV };
  const hits = Object.entries(files)
    .filter(([, p]) => /e\.key !== "\/"/.test(fs.readFileSync(p, "utf8")))
    .map(([n]) => n);
  is(
    hits, ["Viewer"],
    "`/` is bound in exactly one place\n"
    + "     (pre-fix: DocsNav and IndexView each bound it in CAPTURE phase with preventDefault, and both are\n"
    + "      mounted on the index route — the later registration won and the nav's filter was unreachable)",
  );
}

// ── IndexView groups follow the server's order ───────────────────────────────
{
  const raw = fs.readFileSync(INDEXVIEW, "utf8");
  const a = raw.indexOf("const groups = useMemo(");
  const b = raw.indexOf("}, [entries, q]);", a);
  if (a < 0 || b < 0) {
    fail("the `groups` useMemo is gone from IndexView.tsx — re-point this check at whatever orders the index");
  } else {
    const slice = raw.slice(a, b + "}, [entries, q]);".length);
    const js = (await transformWithEsbuild(
      `export function build(env: any) { const { useMemo, matches, entries, q } = env;\n${slice}\n return groups; }`,
      "indexview-check.ts", { loader: "ts", format: "esm" },
    )).code;
    const { build } = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));
    // The declared order, deliberately not alphabetical — this is the shape
    // `docs::index_with` produces for `[docs] paths = ["b", "a"]`.
    const entries = [
      { path: "zeta.md", title: "Zeta", group: "" },
      { path: "docs/b.md", title: "B", group: "docs" },
      { path: "adr/one.md", title: "One", group: "adr" },
    ];
    const groups = build({ useMemo: (f) => f(), matches: () => true, entries, q: "" });
    is(
      groups.map((g) => g[0]), [".", "docs", "adr"],
      "the index lists groups in the SERVER's order\n"
      + "     (pre-fix: a localeCompare sorted them, so the index and the nav — both on screen on this very\n"
      + "      route — showed the same documents in different orders, and nothing failed)",
    );
  }
}

console.log(failed === 0 ? "\nall good" : `\n${failed} failing`);
process.exit(failed === 0 ? 0 : 1);
