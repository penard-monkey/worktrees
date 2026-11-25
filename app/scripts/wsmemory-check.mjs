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
eq("an edit with chats live says while, never by; a deleted chat is one", [M.changedBy(c({ kind: "edit", subject: "Edited: a", live: ["c1"] }), (id) => (id === "c1" ? "Release order" : null)), M.changedBy(c({ kind: "edit", subject: "Edited: a", live: ["c1", "gone"] }), (id) => (id === "c1" ? "Release order" : null))],
  ["Edited while “Release order” was live", "Edited while “Release order” and a chat that was deleted were live"]);
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

eq("commits that fail, in git's words; the files are not lost", M.failingWords({ error: "git commit failed: fatal: no space left on device.", since: 1, times: 4 }, "14:31"),
  "Changes are not reaching this memory's history: git commit failed: fatal: no space left on device. The files are saved. Tried 4 times since 14:31.");
// ── what a note is called ──
{
  const view = { folders: [folder("notes", f("notes/2026-11-24-release-order-000904.md", { title: "Release order: the role first" }), f("notes/2026-11-24-x.md"))] };
  eq("a note by its heading, anything else by its path", ["notes/2026-11-24-release-order-000904.md", "topics/nas.md", "MEMORY.md"].map((p) => M.nameAt(view, p)), ["Release order: the role first", "topics/nas.md", "MEMORY.md"]);
  eq("one that is gone, or has no heading, by its words; never its path or an id", ["notes/2026-11-24-who-can-see-what-000905.md", "notes/2026-11-24-x.md", "notes/2026-11-24-01JC8Z4K3M9QX2T7VB00000905.md"].map((p) => M.nameAt(view, p)), ["Who can see what", "X", "A note"]);
  eq("with no view yet too", M.nameAt(null, "notes/2026-11-24-storage-layout-000903.md"), "Storage layout");
  eq("a chat has a note once its first fold wrote it", [{ note: "", note_exists: false }, { note: "", note_exists: true }, { note: "memory/notes/a.md", note_exists: false }, { note: "memory/notes/a.md", note_exists: true }].map(M.chatNote), [null, null, null, "notes/a.md"]);
}

// ── editing ──
const link = (from, to) => `${from}->${to}`;
const v = { folders: [folder("", f("MEMORY.md", { title: "Memory" })), folder("topics", f("topics/nas.md", { title: "NAS [layout]" }), f("topics/audiences.md")), folder("notes", f("notes/2026-11-24-a.md", { title: "Storage layout" })), folder("files", f("files/rack.png", { kind: "image" }))] };
const ideas = [{ id: "I1", title: "Jellyfin sharing" }];
eq("[[ offers this memory's files first, then linked ideas, never the file it is in", M.memoryPicks("", "topics/nas.md", v, ideas, link).map((p) => p.hint).sort().join() + "|" + M.memoryPicks("", "topics/nas.md", v, ideas, link).map((p) => p.hint).indexOf("idea"), "file,idea,image,note,topic|4");
eq("…its ideas last", M.memoryPicks("", "topics/nas.md", v, ideas, link).at(-1), { id: "I1", title: "Jellyfin sharing", hint: "idea", text: "[Jellyfin sharing](idea:I1)" });
eq("…and not itself", M.memoryPicks("nas", "topics/nas.md", v, ideas, link), []);
eq("a plain link, its words escaped; an image is drawn", M.memoryPicks("nas", "MEMORY.md", v, [], link).concat(M.memoryPicks("rack", "MEMORY.md", v, [], link)).map((p) => p.text), ["[NAS \\[layout\\]](MEMORY.md->topics/nas.md)", "![rack](MEMORY.md->files/rack.png)"]);
eq("by heading or path, the best first", M.memoryPicks("stor", "MEMORY.md", v, [], link).map((p) => p.id), ["notes/2026-11-24-a.md"]);
eq("at most ten", M.memoryPicks("", "x", { folders: [folder("topics", ...Array.from({ length: 30 }, (_, i) => f(`topics/${i}.md`)))] }, ideas, link).length, 10);
const topic = (t) => `topics/${t.toLowerCase().replace(/ /g, "-")}.md`;
eq("a new file: under topics, or where its name says, always a text", ["NAS Layout", "notes/x", "plans/y.csv", " /a/b ", "", "../x", "a//b", ".git", "a/.b", "x\\y"].map((n) => M.newFilePath(n, topic)), ["topics/nas-layout.md", "notes/x.md", "plans/y.csv", "a/b.md", null, null, null, null, null, null]);
eq("its heading", M.titleOf("topics/nas-layout.md"), "Nas layout");
eq("4 MB, and said", [M.tooBig("a.png", 4 * 1024 * 1024), M.tooBig("a.png", 5 * 1024 * 1024)], [null, "a.png is 5.0 MB: a memory takes files up to 4 MB."]);
const at = new Date("2026-11-24T10:20:30Z");
eq("a pasted image has a name of its own; a path and a dot are not kept", [M.putName("image.png", "image/png", at), M.putName("", "image/jpeg", at), M.putName("a/b.png", "image/png", at), M.putName("..hidden", "", at)], ["pasted-2026-11-24-10-20-30.png", "pasted-2026-11-24-10-20-30.jpg", "a-b.png", "hidden"]);
eq("who may edit it too", [[], ["A"], ["A", "B"]].map(M.mayEdit), [null, "“A” is live and may edit this file too.", "2 chats are live and may edit this file too."]);
eq("a change under you names who is live, never who did it", [[], ["A"], ["A", "B"]].map(M.changedUnderWords), ["Changed on disk.", "Changed on disk. “A” is live.", "Changed on disk. 2 chats are live."]);
const d = (a, b) => M.lineDiff(a, b).map((x) => ({ ctx: " ", del: "-", add: "+", gap: "~" })[x.kind] + x.text);
eq("a line put in", d("a\nb\nc", "a\nx\nb\nc"), [" a", "+x", " b", " c"]);
eq("one changed", d("a\nb\nc", "a\nB\nc"), [" a", "-b", "+B", " c"]);
eq("the same text is no lines, whatever its line ends", [d("a\nb", "a\nb"), d("a\r\nb", "a\nb")], [[], []]);
eq("what is far apart has a gap", d(["1", ..."abcdefghijkl", "2"].join("\n"), ["9", ..."abcdefghijkl", "8"].join("\n")), ["-1", "+9", " a", " b", " c", "~", " j", " k", " l", "-2", "+8"]);
eq("its numbers are the side's own", M.lineDiff("a\nb", "a\nx\nb").map((x) => x.no), [1, 2, 3]);
const big = M.lineDiff(Array.from({ length: 1500 }, (_, i) => `a${i}`).join("\n"), Array.from({ length: 1500 }, (_, i) => `b${i}`).join("\n"));
eq("a block too large is all removed, all added", [big.filter((x) => x.kind === "del").length, big.filter((x) => x.kind === "add").length], [1500, 1500]);

if (failed) { console.log(`\nwsmemory-check: ${failed} failed`); process.exit(1); }
console.log("wsmemory-check: all ok");
