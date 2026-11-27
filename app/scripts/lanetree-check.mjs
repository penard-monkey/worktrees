// The nav's lane tree (docs/proposals/lane-children.md §5): how one tier's rows
// are laid out from `lineage.parent`. Loads the REAL `app/src/lanetree.ts`
// and drives it with places shaped like the snapshot's. Run from the repo root.
//
// What it holds: depth to any level; a starter in another tier is a ghost
// (§5.4), a filtered-out one is a ghost in this tier (§5.6); a fold hides every
// depth and says what it hid; a typed filter ignores folds; siblings sort among
// themselves, a row by the newest activity under it; the depth stop (§5.2);
// a manual drop lands among siblings, its gap after the last one's subtree.

import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const src = fs.readFileSync(fileURLToPath(new URL("../src/lanetree.ts", import.meta.url)), "utf8");
const js = (await transformWithEsbuild(src, "lanetree.ts", { loader: "ts", format: "esm" })).code;
const { laneTree, indentOf, siblingSlot, subtreeOf, handoffLine, laneLines } = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

let bad = 0;
const eq = (name, got, want) => {
  const g = JSON.stringify(got), w = JSON.stringify(want);
  if (g !== w) { console.error(`FAIL ${name}\n  got  ${g}\n  want ${w}`); bad++; }
};

// A place: slug, its drawn parent, its tier, an activity time.
const P = (slug, parent, tier = "active", at = 0) => ({ slug, parent, tier, at });
const tree = (all, here, o = {}) => laneTree({
  rows: (o.rows ?? all).filter((p) => p.tier === here),
  all,
  slug: (p) => p.slug,
  parent: (p) => p.parent,
  tier: (p) => p.tier,
  here,
  order: o.order ?? ((sibs, latest) => [...sibs].sort((a, b) => latest(b) - latest(a) || a.slug.localeCompare(b.slug))),
  at: (p) => p.at,
  folded: (s) => !!o.folded?.[s],
  filtering: !!o.filtering,
});
const shape = (items) => items.map((i) => `${i.kind === "ghost" ? "~" : ""}${"  ".repeat(i.depth)}${i.slug}${i.collapsed ? `(${i.hidden.map((h) => h.slug).join(",")})` : ""}${i.kind === "ghost" ? `[${i.tier}]` : ""}`);

// 1. No lineage at all: today's flat list, in the order's order.
eq("flat", shape(tree([P("b", undefined, "active", 2), P("a", undefined, "active", 1)], "active")), ["b", "a"]);

// 2. Real depth: a grandchild sits under its parent, to any level.
const deep = [P("o"), P("d1", "o"), P("d2", "d1"), P("d3", "d2"), P("d4", "d3"), P("d5", "d4"), P("d6", "d5")];
eq("depth 6", tree(deep, "active").map((i) => [i.slug, i.depth, i.kids]),
  [["o", 0, true], ["d1", 1, true], ["d2", 2, true], ["d3", 3, true], ["d4", 4, true], ["d5", 5, true], ["d6", 6, false]]);

// 3. A closed middle: the starter is a ghost in ACTIVE, its own row in CLOSED.
const split = [P("chats", undefined, "active", 9), P("combined", "chats", "closed"), P("critique", "combined", "active", 5), P("kimi", "chats", "active", 3)];
eq("closed middle (active)", shape(tree(split, "active")), ["chats", "~  combined[closed]", "    critique", "  kimi"]);
eq("closed middle (closed)", shape(tree(split, "closed")), ["~chats[active]", "  combined"]);

// 4. A closed orchestrator with working lanes: ghost on top in ACTIVE.
const orch = [P("orch", undefined, "closed"), P("k1", "orch", "active", 2), P("k2", "orch", "active", 1), P("solo", undefined, "active", 0)];
eq("closed orchestrator", shape(tree(orch, "active")), ["~orch[closed]", "  k1", "  k2", "solo"]);
eq("a ghost is not counted as a row", tree(orch, "active").filter((i) => i.kind === "row").length, 3);

// 5. A pinned lane: in PINNED under a ghost of its starter; the starter's real
//    row does not roll it up (it does not hide it).
const pin = [P("s", undefined, "active"), P("k", "s", "pinned"), P("k2", "s", "active")];
eq("pinned lane", shape(tree(pin, "pinned")), ["~s[active]", "  k"]);
eq("starter folds only its own tier", shape(tree(pin, "active", { folded: { s: true } })), ["s(k2)"]);

// 6. A fold hides every depth below and says what it hid.
const fam = [P("a", undefined, "active", 1), P("b", "a"), P("c", "b"), P("d", "a"), P("x")];
eq("fold", shape(tree(fam, "active", { folded: { a: true } })), ["a(b,c,d)", "x"]);
eq("fold inside an open parent", shape(tree(fam, "active", { folded: { b: true } })), ["a", "  b(c)", "  d", "x"]);
eq("a fold on a childless row is nothing", shape(tree(fam, "active", { folded: { x: true } })), ["a", "  b", "    c", "  d", "x"]);
eq("a ghost folds and hides real rows only", shape(tree(orch, "active", { folded: { orch: true } })), ["~orch(k1,k2)[closed]", "solo"]);

// 7. A typed filter ignores folds (and the starter of a match shows dim).
const rows = fam.filter((p) => p.slug === "c");
eq("filter: starters as context, fold ignored", shape(tree(fam, "active", { rows, filtering: true, folded: { a: true } })), ["~a[active]", "~  b[active]", "    c"]);

// 8. Last used: a busy grandchild lifts its family; siblings sort among themselves.
const lift = [P("old", undefined, "active", 1), P("mid", undefined, "active", 5), P("kid", "old", "active", 2), P("gk", "kid", "active", 10)];
eq("subtree activity lifts a family", shape(tree(lift, "active")), ["old", "  kid", "    gk", "mid"]);

// 9. A name with no place behind it, and a hand-made loop in a bad snapshot:
//    top level, and no hang.
eq("dangling parent", shape(tree([P("a", "gone")], "active")), ["a"]);
eq("a loop draws every row, at the top", shape(tree([P("a", "b"), P("b", "a"), P("c")], "active")), ["a", "b", "c"]);

// 10. The depth stop (§5.2): 14 px to level 4 at 280 px and wider; 8 px to
//     level 3 under it; past the stop the level shows as a number.
eq("stop wide 4", indentOf(4, 300), { lvl: 4, step: 14, label: null });
eq("stop wide 5", indentOf(5, 300), { lvl: 4, step: 14, label: 5 });
eq("stop at 280 is wide", indentOf(5, 280), { lvl: 4, step: 14, label: 5 });
eq("stop narrow 3", indentOf(3, 240), { lvl: 3, step: 8, label: null });
eq("stop narrow 4", indentOf(4, 220), { lvl: 3, step: 8, label: 4 });
eq("top level", indentOf(0, 220), { lvl: 0, step: 8, label: null });

// 11. Manual drop among siblings: before the sibling at the slot; past the
//     last one, the gap goes after its whole subtree.
const items = tree([P("a", undefined, "active", 3), P("b", "a", "active", 2), P("c", "b"), P("d", "a", "active", 1), P("e", undefined, "active", 0)], "active");
eq("render order", shape(items), ["a", "  b", "    c", "  d", "e"]);
eq("slot among siblings", siblingSlot(items, "d", "a", 0), { before: "b", gap: "b" });
eq("slot after the last sibling's subtree", siblingSlot(items, "d", "a", 1), { before: null, gap: "e" });
eq("slot after b's subtree, b being last", siblingSlot(items, "zzz", "a", 2), { before: null, gap: "e" });
eq("top-level slot at the end", siblingSlot(items, "a", null, 1), { before: null, gap: null });
eq("only child: straight under its parent", siblingSlot(items, "c", "b", 0), { before: null, gap: "d" });
eq("a subtree travels", subtreeOf(items, "a"), ["a", "b", "c", "d"]);
eq("a leaf's subtree is itself", subtreeOf(items, "c"), ["c"]);

// 12. The remove dialog says what `rm` will do, in core's own words: read
//     `handoff_sentence` out of ops.rs and fill its format strings.
const ops = fs.readFileSync(fileURLToPath(new URL("../../crates/worktrees-core/src/ops.rs", import.meta.url)), "utf8");
const fn = ops.slice(ops.indexOf("pub fn handoff_sentence"), ops.indexOf("\n}\n", ops.indexOf("pub fn handoff_sentence")));
const fmts = [...fn.matchAll(/format!\("([^"]+)"/g)].map((m) => m[1]);
if (fmts.length !== 2) { console.error(`FAIL found ${fmts.length} format strings in ops::handoff_sentence, expected 2`); bad++; }
else {
  eq("one lane, core's words", handoffLine(["x"], "y"), fmts[0].replace("{}", "x").replace("{to}", "y"));
  eq("two lanes, core's words", handoffLine(["a", "b"], "(main)"), fmts[1].replace("{n}", "2").replace("{}", "a, b").replace("{to}", "(main)"));
}
eq("no lanes, no sentence", handoffLine([], "y"), null);

// 13. The header lines (§8).
const d = () => "25 Nov";
eq("adopted lane", laneLines({ starter: { place: "combined", at: 1 }, under: { place: "(main)", at: 2, by: "(main)" }, reports_to: "combined", started: ["probe"] }, () => false, d),
  ["started by combined", "moved to top level by (main), 25 Nov", "reports to combined", "started 1 lane"]);
eq("closed starter, reports further up", laneLines({ starter: { place: "s", at: 1 }, parent: "s", reports_to: "(main)" }, (x) => x === "s", d),
  ["started by s (closed)", "reports to (main)"]);
eq("handed lane", laneLines({ starter: { place: "gone", at: 1, removed: true }, handed_to: { place: "a", at: 2 }, parent: "a", reports_to: "a" }, () => false, d),
  ["started by gone (removed)", "now under a"]);
eq("from another project", laneLines({ starter: { place: "chats", at: 1, project: "worktrees" }, reports_to: "(main)" }, () => false, d), ["from worktrees: chats"]);
eq("a starter the caller may not see", laneLines({ starter: { at: 1, elsewhere: true }, reports_to: "(main)" }, () => false, d), ["started in another project"]);
eq("moved under by the owner", laneLines({ starter: { place: "s", at: 1 }, under: { place: "x", at: 2, by: "owner" }, parent: "x", reports_to: "s" }, () => false, d),
  ["started by s", "moved under x by the owner, 25 Nov", "reports to s"]);
eq("moved AND handed: it is drawn where it was moved, so no \"now under\"", laneLines({ starter: { place: "gone", at: 1, removed: true }, under: { place: "x", at: 3, by: "owner" }, handed_to: { place: "a", at: 2 }, parent: "x", reports_to: "a" }, () => false, d),
  ["started by gone (removed)", "moved under x by the owner, 25 Nov", "reports to a"]);
eq("plain top-level place", laneLines({ reports_to: "(main)" }, () => false, d), []);

if (bad) { console.error(`lanetree-check: ${bad} failure(s)`); process.exit(1); }
console.log("lanetree-check: the nav lays out lanes by lineage, ghosts, folds, the depth stop and sibling drops as §5 says");
