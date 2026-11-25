// A workspace's memory on screen: the pure rules (app/src/wsmemory.ts), run.
//
//   node app/scripts/wsmemory-check.mjs     # exits non-zero on failure
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const src = fs.readFileSync(fileURLToPath(new URL("../src/wsmemory.ts", import.meta.url)), "utf8");
let failed = 0;
const eq = (name, got, want) => {
  const g = JSON.stringify(got), w = JSON.stringify(want);
  if (g === w) return;
  failed++;
  console.log(`not ok — ${name}\n     got: ${g}\n    want: ${w}`);
};
eq("no runtime import", /^import (?!type )/m.test(src), false);
const M = await import("data:text/javascript;base64," + Buffer.from((await transformWithEsbuild(src.replace(/^import type .*$/gm, ""), "x.ts", { loader: "ts", format: "esm" })).code).toString("base64"));

const f = (path, over = {}) => ({ path, name: path.split("/").pop(), size: 10, mtime_ms: Date.UTC(2026, 9, 5), kind: "text", title: null, ...over });
const folder = (name, ...files) => ({ name, files });
const keys = (folders, q) => M.fileGroups(folders, q).map((g) => [g.key, g.open, g.files.map((x) => x.path)]);
// ── the list ──
eq("the index first, then notes (newest first), topics, files, others by name", keys([
  folder("zebra", f("zebra/a.md")), folder("files", f("files/r.png")), folder("topics", f("topics/n.md")), folder("archive", f("archive/a.md")),
  folder("notes", f("notes/2026-11-19-a.md"), f("notes/2026-11-24-b.md")), folder("", f("README.md"), f("MEMORY.md")),
]), [["", true, ["MEMORY.md", "README.md"]], ["notes", true, ["notes/2026-11-24-b.md", "notes/2026-11-19-a.md"]], ["topics", true, ["topics/n.md"]], ["files", true, ["files/r.png"]], ["archive", true, ["archive/a.md"]], ["zebra", true, ["zebra/a.md"]]]);
const many = Array.from({ length: 13 }, (_, i) => f(`notes/2026-${i < 5 ? "10" : "11"}-${String(i + 1).padStart(2, "0")}-n.md`));
eq("notes fold by month from 13, the newest month open", keys([folder("notes", ...many)]).map((g) => [g[0], g[1], g[2].length]), [["notes/2026-11", true, 8], ["notes/2026-10", false, 5]]);
eq("twelve are one group", keys([folder("notes", ...many.slice(0, 12))]).length, 1);
eq("a note with no date is of the month it changed", keys([folder("notes", ...many, f("notes/undated.md"))]).map((g) => [g[0], g[2].length]), [["notes/2026-11", 8], ["notes/2026-10", 6]]);
eq("two hundred files start folded", keys([folder("topics", ...Array.from({ length: 200 }, (_, i) => f(`topics/${i}.md`)))])[0][1], false);
eq("a filter opens what it finds, notes undivided, by path or heading", keys([folder("notes", ...many, f("notes/x.md", { title: "The RACK" })), folder("topics", f("topics/rack.md"), f("topics/y.md"))], "rack"), [["notes", true, ["notes/x.md"]], ["topics", true, ["topics/rack.md"]]]);
eq("a note is called by its heading, a topic by its file", [M.fileName(f("notes/a.md", { title: "Storage" })), M.fileName(f("topics/a.md", { title: "Storage" })), M.fileName(f("notes/a.md"))], ["Storage", "a.md", "a.md"]);
eq("sizes", [12, 4200, 23_552, 320_000, 4_300_000].map(M.sizeWords), ["12 B", "4.1 KB", "23 KB", "313 KB", "4.1 MB"]);
eq("a long index says so, with its size", [M.longIndex({ index: { over_budget: true, size: 23_552 } }), M.longIndex({ index: { over_budget: false, size: 1 } }), M.longIndex({ index: null })], ["The index is long (23 KB). Every chat reads all of it.", null, null]);

// ── changes ──
const c = (over) => ({ commit: "c1", at: 0, subject: "fold: Release order", kind: "fold", files: [], added: 0, removed: 0, reverts: null, reverted_by: null, ...over });
eq("who changed it", [c(), c({ kind: "undo", subject: "Undo: fold: Release order" }), c({ kind: "redo", subject: "Redo: fold: Release order" }), c({ kind: "edit", subject: "Edited: MEMORY.md" }), c({ kind: "edit", subject: "Chat closed: First look" }), c({ kind: "made" })].map(M.changedBy),
  ["Changed by the fold of “Release order”", "“Release order” was undone", "“Release order” was put back", "Edited by hand", "Changed: Chat closed: First look", "Made"]);
eq("Undo reverts it; undone, Redo reverts what undid it; what made it, neither", [M.revertOf(c()), M.revertOf(c({ reverted_by: "c9" })), M.revertOf(c({ kind: "made" }))], [{ commit: "c1", redo: false }, { commit: "c9", redo: true }, null]);
eq("a refused revert names the files", [M.conflictWords(["MEMORY.md"]), M.conflictWords(["a", "b", "c"])], ["It was not undone: MEMORY.md has changed since. Nothing was touched.", "It was not undone: a, b and c have changed since. Nothing was touched."]);
eq("the dot: changed since looked at; only made is nothing", [M.unseen(c(), null), M.unseen(c(), "c1"), M.unseen(c(), "c0"), M.unseen(c({ kind: "made" }), null), M.unseen(null, null)], [true, false, true, false, false]);
eq("the index is the first file of a change", M.changeFiles({ files: [{ path: "a.md" }, { path: "MEMORY.md" }] }).map((x) => x.path), ["MEMORY.md", "a.md"]);

// ── ‹ › ──
let t = M.trailStart();
for (const p of ["MEMORY.md", "topics/a.md", "topics/a.md", "notes/b.md"]) t = M.trailTo(t, p);
eq("each file once as it is read", t, { at: 2, paths: ["MEMORY.md", "topics/a.md", "notes/b.md"] });
const b = M.trailStep(t, -1);
eq("back, and forward", [b.paths[b.at], M.trailStep(b, 1), M.trailStep(t, 1), M.trailStep(M.trailStart(), -1)], ["topics/a.md", t, null, null]);
eq("a new file after Back drops what was forward", M.trailTo(b, "x.md").paths, ["MEMORY.md", "topics/a.md", "x.md"]);
eq("a chat's note, in the memory", M.notePath("memory/notes/2026-11-24-x.md"), "notes/2026-11-24-x.md");

if (failed) { console.log(`\nwsmemory-check: ${failed} failed`); process.exit(1); }
console.log("wsmemory-check: all ok");
