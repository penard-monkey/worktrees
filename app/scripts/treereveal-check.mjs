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
// Every way into the viewer goes through `openDockFile`; a re-open of the SAME
// file changes no path, so only a token can tell the tree to scroll again.
const app = src("../src/App.tsx");
const odf = app.slice(app.indexOf("const openDockFile = useCallback"));
const body = odf.slice(0, odf.indexOf("}, [updateSettings]);"));
ok("openDockFile bumps the reveal token", /setRevealToken\(/.test(body));
ok("FilesPane is handed the reveal token", /<FilesPane[\s\S]{0,400}revealToken=\{revealToken\}/.test(app));
const fp = src("../src/FilesPane.tsx");
ok("FileTree passes the token on", /<FileTree[^>]*revealToken=\{revealToken\}/.test(fp));
// The tree compares in CANONICAL form — the raw prop never matched a row
// reached through a symlinked place path.
ok("FileTree normalises the open path with treePath", /treePath\(root,/.test(fp));
// Scroll the tree's own box only: `scrollIntoView` also scrolls every overflow
// ancestor, the shape AGENTS.md records for a hidden-overflow header.
ok("the reveal does not use scrollIntoView", !/\.scrollIntoView\(\s*\{\s*block:\s*"(nearest|center)"/.test(fp.slice(fp.indexOf("function TreeNode"), fp.indexOf("function FileTree"))));

console.log(failed ? `\n${failed} check(s) failed` : "all tree-reveal checks passed");
process.exit(failed ? 1 : 0);
