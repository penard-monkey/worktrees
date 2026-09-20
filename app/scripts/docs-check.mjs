// Deterministic check of the Docs tab's TREE — specifically that it never
// re-orders what the backend handed it, and that it nests on `group` rather
// than on the path.
//
// Why this one earns a script when the flat list did not. `docs::index_with`
// controls order on purpose, and part of that order is the REPO's: `[docs]
// paths = ["b", "a"]` lists b before a because the project said so, and the
// walk refuses to sort it (`declared_paths_keep_one_contiguous_run_per_group`).
// A `.sort()` added here to tidy the display would overrule that silently —
// nothing throws, nothing fails, the list is simply not what the project asked
// for. That is the same shape as `dnd.ts::predictTier` drifting from
// `store::reconcile`, and the reason this file exists rather than a comment.
//
// Like race-check.mjs and ctxmenu-check.mjs, it does NOT paraphrase the
// component: it evaluates the REAL source text of `tree()` out of DocsPane.tsx,
// so running it before and after an edit tests the edit itself.
//
// `tree()` moved out of `DocsPane.tsx` into `app/src/doctree.ts` when the
// browser viewer's persistent nav became a second renderer of the same tree —
// ONE implementation, two surfaces. This path had to move with it: a slice that
// cannot find its function throws, but a check pointed at a file that no longer
// holds the rule would simply stop guarding, and a guard that stopped guarding
// looks exactly like a guard that passed.
//
//   node docs-check.mjs [path/to/doctree.ts]           exits non-zero on failure
import fs from "node:fs";
import { fileURLToPath } from "node:url";
// vite's esbuild re-export — bare "esbuild" does not resolve under pnpm's
// strict layout, vite does (a direct dependency).
import { transformWithEsbuild } from "vite";

const SRC = process.argv[2] || fileURLToPath(new URL("../src/doctree.ts", import.meta.url));
const raw = fs.readFileSync(SRC, "utf8");

let failures = 0;
const fail = (msg) => {
  failures++;
  console.error("docs-check: FAIL — " + msg);
};

/** The text of a top-level declaration, brace-matched from its header line.
 *  Template literals in the body balance their own braces, so a plain counter
 *  is enough here and stays readable. */
function slice(header) {
  const at = raw.indexOf(header);
  if (at < 0) throw new Error(`${header} not found in ${SRC} — renamed? this check reads the real source`);
  let depth = 0;
  for (let i = raw.indexOf("{", at); i < raw.length; i++) {
    if (raw[i] === "{") depth++;
    else if (raw[i] === "}" && --depth === 0) return raw.slice(at, i + 1);
  }
  throw new Error(`unbalanced braces after ${header}`);
}

const typeAt = raw.indexOf("export type DocNode");
if (typeAt < 0) throw new Error("DocNode is gone — renamed?");
// To the first `;` OUTSIDE a brace — the union's own members end in one, and
// stopping at the first would cut the type in half and report the truncation as
// a syntax error somewhere else entirely.
const nodeType = (() => {
  let depth = 0;
  for (let i = typeAt; i < raw.length; i++) {
    if (raw[i] === "{") depth++;
    else if (raw[i] === "}") depth--;
    else if (raw[i] === ";" && depth === 0) return raw.slice(typeAt, i + 1);
  }
  throw new Error("DocNode's declaration never ends");
})();

const wrapped = `
type DocEntry = { path: string; rel: string; title: string; group: string; mtime_ms: number };
${nodeType}
${slice("export function tree(")}
`;
const js = (await transformWithEsbuild(wrapped, "docs-check.ts", { loader: "ts", format: "esm" })).code;
const { tree } = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

// ── fixtures ─────────────────────────────────────────────────────────────────

/** A row as `docs::index_with` emits one. `group` is the backend's own
 *  decision and is NOT always `rel`'s directory — see the brief. */
const doc = (rel, group = "") => ({ path: "/p/" + rel, rel, title: rel, group, mtime_ms: 0 });

const kidsOf = (nodes, path) => {
  const seg = path.split("/");
  let list = nodes;
  for (const s of seg) {
    const hit = list.find((n) => n.kind === "dir" && n.name === s);
    if (!hit) return null;
    list = hit.kids;
  }
  return list;
};
const names = (nodes) => nodes.map((n) => (n.kind === "dir" ? n.name + "/" : n.entry.rel));

// ── 1. first appearance, at every level ──────────────────────────────────────
// The order a repo declared (`[docs] paths = ["b", "a"]`) and the fixed reading
// order of the root files both arrive as sequence and nothing else. Sorting any
// level throws that away, and the fixture is deliberately in anti-alphabetical
// order so a `.sort()` cannot pass by luck.
{
  const t = tree([
    doc("README.md"),
    doc("CLAUDE.md"),
    doc("zeta.md"),
    doc("b/one.md", "b"),
    doc("b/zzz.md", "b"),
    doc("b/alpha.md", "b"),
    doc("a/two.md", "a"),
  ]);
  const want = ["README.md", "CLAUDE.md", "zeta.md", "b/", "a/"];
  if (JSON.stringify(names(t)) !== JSON.stringify(want)) {
    fail(`root order is ${JSON.stringify(names(t))}, not the backend's ${JSON.stringify(want)}`);
  }
  const b = kidsOf(t, "b");
  const wantB = ["b/one.md", "b/zzz.md", "b/alpha.md"];
  if (JSON.stringify(names(b)) !== JSON.stringify(wantB)) {
    fail(`inside a directory the order is ${JSON.stringify(names(b))}, not ${JSON.stringify(wantB)}`);
  }
}

// ── 2. files before subdirectories, as the walk emits them ───────────────────
// `index_with` lists a directory's own files and then descends. Losing that
// puts `docs/index.md` — the landing page — below three directories of detail.
{
  const t = tree([
    doc("docs/index.md", "docs"),
    doc("docs/adr/0001.md", "docs/adr"),
    doc("docs/later.md", "docs"),
  ]);
  const got = names(kidsOf(t, "docs"));
  if (got[0] !== "docs/index.md" || got[1] !== "adr/") {
    fail(`docs/ children are ${JSON.stringify(got)} — the walk's file-then-directory order is gone`);
  }
}

// ── 3. the nesting comes from `group`, never from `rel` ──────────────────────
// The brief is the row where they differ: it lives at `.planning/brief.md` and
// is grouped with the ROOT files on purpose (`DocEntry::group`), because a group
// of one under a gitignored directory name reads as an accident. Deriving the
// parent from the path would file it under a `.planning/` the walk does not
// show, and every other `""`-grouped row with a slash in it goes the same way.
{
  const t = tree([doc("README.md"), doc(".planning/brief.md"), doc("docs/a.md", "docs")]);
  if (names(t)[1] !== ".planning/brief.md") {
    fail(`the brief was nested under a directory: ${JSON.stringify(names(t))}`);
  }
  if (t.some((n) => n.kind === "dir" && n.name === ".planning")) {
    fail("a `.planning` directory node was invented from the brief's path");
  }
}

// ── 4. a parent nobody grouped still exists ──────────────────────────────────
// `docs/rfc/2026` can be the only group in a place — nothing sits directly in
// `docs/rfc`. The chain has to be built from the path's segments; built from
// the rows instead, the node is missing and its documents surface at the root
// looking like README files.
{
  const t = tree([doc("docs/rfc/2026/one.md", "docs/rfc/2026")]);
  const kids = kidsOf(t, "docs/rfc/2026");
  if (!kids || names(kids)[0] !== "docs/rfc/2026/one.md") {
    fail("a directory that only appears inside a deeper group was not created");
  }
  if (t.length !== 1 || t[0].kind !== "dir" || t[0].name !== "docs") {
    fail(`the root should be one "docs" node, got ${JSON.stringify(names(t))}`);
  }
}

// ── 5. a directory's count is its whole subtree ──────────────────────────────
// It is what a COLLAPSED row says, and it is the only thing telling the reader
// what is behind it. A count of direct children reads "1" over 31 documents.
{
  const t = tree([
    doc("docs/index.md", "docs"),
    doc("docs/adr/0001.md", "docs/adr"),
    doc("docs/adr/0002.md", "docs/adr"),
    doc("docs/adr/old/0000.md", "docs/adr/old"),
  ]);
  const docsNode = t.find((n) => n.kind === "dir" && n.name === "docs");
  const adr = docsNode.kids.find((n) => n.kind === "dir" && n.name === "adr");
  if (docsNode.count !== 4) fail(`docs/ counts ${docsNode.count} documents, not 4 — is it counting direct children?`);
  if (adr.count !== 3) fail(`docs/adr counts ${adr.count}, not 3`);
}

// ── 6. every key the tree renders is unique ──────────────────────────────────
// React answers a duplicate key by duplicating a sibling and omitting its
// children, which is how the flat list's group headers broke once
// (`every_group_is_one_contiguous_run`). Directory keys are paths and document
// keys are absolute paths, so this holds by construction — asserted anyway,
// because "by construction" is what it was last time.
{
  const t = tree([
    doc("README.md"),
    doc("docs/a.md", "docs"),
    doc("docs/sub/a.md", "docs/sub"),
    doc("other/a.md", "other"),
  ]);
  const keys = [];
  const walk = (nodes) => {
    for (const n of nodes) {
      if (n.kind === "dir") {
        keys.push("dir:" + n.path);
        walk(n.kids);
      } else keys.push(n.entry.path);
    }
  };
  walk(t);
  if (new Set(keys).size !== keys.length) fail(`duplicate React keys in the tree: ${JSON.stringify(keys)}`);
}

if (failures) {
  console.error(`docs-check: ${failures} failure(s)`);
  process.exit(1);
}
console.log("docs-check: ok — first-appearance order at every level, files before directories, grouping by `group` not by path, synthesised parents, subtree counts, unique keys");
