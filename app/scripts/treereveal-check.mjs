// Checks the Files tree's "reveal the open file" — the path rules in
// app/src/filekind.ts, and the wiring in FilesPane.tsx / App.tsx that makes
// every way into the viewer reach the tree.
//
//   node app/scripts/treereveal-check.mjs        # exits non-zero on failure
//
// Before this, a file opened from a terminal ⌘-click or MCP `show_doc` showed
// in the viewer while its folders stayed shut: the row was "selected" by an
// equality test on a row that was never mounted. The rules worth pinning are
// the ones that fail SILENTLY — a path in the wrong form simply never matches,
// and the tree looks exactly as it did before the fix.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const src = (p) => fs.readFileSync(fileURLToPath(new URL(p, import.meta.url)), "utf8");
const js = (await transformWithEsbuild(src("../src/filekind.ts"), "filekind.ts", { loader: "ts", format: "esm" })).code;
const F = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

let failed = 0;
const eq = (name, got, want) => {
  const g = JSON.stringify(got), w = JSON.stringify(want);
  if (g === w) return;
  failed++;
  console.log(`not ok — ${name}\n     got: ${g}\n    want: ${w}`);
};
const ok = (name, cond) => { if (!cond) { failed++; console.log(`not ok — ${name}`); } };

const has = (name) => typeof F[name] === "function";
ok("filekind exports treePath", has("treePath"));
ok("filekind exports revealsThrough", has("revealsThrough"));

if (has("treePath")) {
  // macOS: /tmp is /private/tmp. The place path is what the user registered;
  // every row the backend lists is canonical.
  const ROOT = "/tmp/w/feature", CANON = "/private/tmp/w/feature";
  eq("a path under the place path moves onto the canonical root",
    F.treePath(ROOT, CANON, `${ROOT}/docs/a.md`), `${CANON}/docs/a.md`);
  eq("an already canonical path is untouched",
    F.treePath(ROOT, CANON, `${CANON}/docs/a.md`), `${CANON}/docs/a.md`);
  eq("a sibling sharing the root as a name prefix is not rewritten",
    F.treePath(ROOT, CANON, `${ROOT}-2/a.md`), `${ROOT}-2/a.md`);
  eq("a canonical place needs no rewrite", F.treePath(CANON, CANON, `${CANON}/a.md`), `${CANON}/a.md`);
  eq("no canonical root yet: unchanged", F.treePath(ROOT, "", `${ROOT}/a.md`), `${ROOT}/a.md`);
  eq("a trailing slash on either root is ignored",
    F.treePath(`${ROOT}/`, `${CANON}/`, `${ROOT}/a.md`), `${CANON}/a.md`);
}
if (has("revealsThrough")) {
  const R = "/r";
  ok("an ancestor directory is opened", F.revealsThrough(`${R}/app`, `${R}/app/src/x.ts`));
  ok("the direct parent is opened", F.revealsThrough(`${R}/app/src`, `${R}/app/src/x.ts`));
  ok("a sibling with the dir as a name prefix is NOT opened", !F.revealsThrough(`${R}/app`, `${R}/app-old/x.ts`));
  ok("the file itself is not a directory to open", !F.revealsThrough(`${R}/app/x.ts`, `${R}/app/x.ts`));
  ok("an unrelated directory stays shut", !F.revealsThrough(`${R}/crates`, `${R}/app/x.ts`));
}

// ── wiring ────────────────────────────────────────────────────────────────
// Source assertions on the REAL files (paths overridable, so a mutant copy can
// be checked: `node treereveal-check.mjs <FilesPane.tsx> <App.tsx>`). Each one
// names a single line whose loss leaves the tree looking exactly as it did
// before the fix — no error, no crash, just a row that never appears.
const fpPath = process.argv[2] ?? fileURLToPath(new URL("../src/FilesPane.tsx", import.meta.url));
const appPath = process.argv[3] ?? fileURLToPath(new URL("../src/App.tsx", import.meta.url));
const fp = fs.readFileSync(fpPath, "utf8");
const app = fs.readFileSync(appPath, "utf8");
const between = (txt, a, b) => {
  const i = txt.indexOf(a);
  if (i < 0) return "";
  const j = txt.indexOf(b, i + a.length);
  return j < 0 ? txt.slice(i) : txt.slice(i, j);
};

// Every way into the viewer goes through `openDockFile`; a re-open of the SAME
// file changes no path, so only a token can tell the tree to scroll again.
const odf = between(app, "const openDockFile = useCallback", "}, [updateSettings]);");
ok("openDockFile bumps the reveal token", /setRevealToken\(/.test(odf));
ok("FilesPane is handed the reveal token", /<FilesPane[\s\S]{0,400}revealToken=\{revealToken\}/.test(app));
ok("FileTree is handed the reveal token", /<FileTree[^>]*revealToken=\{revealToken\}/.test(fp));

const node = between(fp, "function TreeNode(", "function FileTree(");
const tree = between(fp, "function FileTree(", "// ── image");
const scrollFn = between(fp, "function scrollRowIntoBox(", "\n}\n") + "\n}";
ok("slices found", node.length > 0 && tree.length > 0 && scrollFn.includes("box"));

// Directory half: without it nothing opens and the row is never mounted.
ok("a directory on the path opens itself (revealsThrough → setOpen(true))",
  /if \(revealsThrough\(entry\.path, reveal\.path\)\) setOpen\(true\)/.test(node));
// Row half: the scroll lives in the layout effect that consumes the request.
const rowFx = between(node, "useLayoutEffect(", "}, [reveal");
ok("the row's layout effect scrolls it (scrollRowIntoBox(box, row))", /scrollRowIntoBox\(box, row\)/.test(rowFx));
// The request has to reach every level, not just the top one.
ok("the recursive TreeNode passes reveal={reveal}", /<TreeNode[^>]*depth=\{depth \+ 1\}[^>]*reveal=\{reveal\}/.test(node));
// Scroll the tree's own box only: `scrollIntoView` (any form) also scrolls
// every overflow ancestor — the dock, the space body.
ok("no scrollIntoView in TreeNode", !/scrollIntoView\s*\(/.test(node));
ok("no scrollIntoView in scrollRowIntoBox", !/scrollIntoView\s*\(/.test(scrollFn));
// A new request per (open path, token) — the token is the ONLY thing that
// changes on a re-open, so losing it kills re-scroll and nothing else.
ok("the request is keyed on the open path", /revealRef\.current\?\.openPath !== openPath/.test(tree));
ok("the request is keyed on the reveal token", /revealRef\.current\?\.revealToken !== revealToken/.test(tree));
ok("the tree compares in canonical form (treePath)", /treePath\(root,/.test(tree));
// Only a toggle ON THE PATH means the user took over. Expanding an unrelated
// folder while a lazy cascade is still loading must not strand it halfway.
const toggle = between(node, "const toggle = () => {", "\n  };");
ok("only a toggle on the revealed path abandons the reveal",
  /if \(reveal && revealsThrough\(entry\.path, reveal\.path\)\) reveal\.done = true/.test(toggle)
  && !/if \(reveal\) reveal\.done = true/.test(toggle));

// ── scrollRowIntoBox, evaluated ───────────────────────────────────────────
// The real function under a DOM stub: only `box.scrollTop` may move.
if (scrollFn.length > 10) {
  const fjs = (await transformWithEsbuild(scrollFn, "s.ts", { loader: "ts" })).code;
  const scrollRowIntoBox = new Function(`${fjs}; return scrollRowIntoBox;`)();
  const mk = (top, h) => {
    const touched = [];
    const el = { scrollTop: 0, scrollLeft: 0, getBoundingClientRect: () => ({ top, bottom: top + h, height: h, left: 0, right: 100, width: 100 }),
      scrollIntoView: () => touched.push("scrollIntoView"), scrollTo: () => touched.push("scrollTo"), scrollBy: () => touched.push("scrollBy") };
    return { el, touched };
  };
  const run = (rowTop) => {
    const box = mk(100, 200), row = mk(rowTop, 20), parent = mk(0, 1000);
    box.el.parentElement = parent.el; row.el.parentElement = box.el;
    box.el.scrollTop = 50;
    scrollRowIntoBox(box.el, row.el);
    return { delta: box.el.scrollTop - 50, rowMoved: row.el.scrollTop, parentMoved: parent.el.scrollTop,
      calls: [...box.touched, ...row.touched, ...parent.touched] };
  };
  eq("a row in view leaves the tree alone", run(150), { delta: 0, rowMoved: 0, parentMoved: 0, calls: [] });
  // centre of row (510) − centre of box (200) = 310
  eq("a row below the fold is centred, by box.scrollTop only", run(500), { delta: 310, rowMoved: 0, parentMoved: 0, calls: [] });
  eq("a row above the fold is centred, by box.scrollTop only", run(20), { delta: -170, rowMoved: 0, parentMoved: 0, calls: [] });
  eq("a row cut by the bottom edge counts as out of view", run(290), { delta: 100, rowMoved: 0, parentMoved: 0, calls: [] });
}

console.log(failed ? `\n${failed} check(s) failed` : "all tree-reveal checks passed");
process.exit(failed ? 1 : 0);
