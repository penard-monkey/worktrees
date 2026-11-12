// Ideas relate, the non-drag paths (app/companion/ideasrelate.ts, §6.1): a
// check cannot drive a real drag (navdrag.ts's header), so it holds the
// rules every path goes through and the calls they end in.
//
//   node app/scripts/ideas-relate-check.mjs     # from the repo root; exits non-zero on failure
//
// It loads the REAL module (as a `data:` module, which is why ideasrelate.ts
// may have no runtime import) and holds:
//   - the drop rules: an idea onto itself, onto a row with a problem or onto
//     nothing is no target (no highlight, nothing happens); Combine wants two
//     or more ideas; the chooser's default is Relate, which never asks, and
//     Merge asks;
//   - each act's call names a row serve has (dispatch.rs), with arguments
//     that row reads — so the drag, the menu and a later agent's proposal are
//     the same call (§7.1);
//   - in Companion.tsx the drop and its menu path end in the same function
//     (doRelate / doMerge / doCombine / doSplit), and no act row is invoked
//     except through those builders;
//   - `[[`: the query, the matches, and the link it inserts, which `linkAt`
//     (⌘-click), the Read view (`readableLinks`/`ideaHref`) and core's
//     `links_in` all read back, escaped brackets included; core's own split
//     link has the same shape;
//   - selection by ⌘/Shift-click.
//
// Override, so a mutation can be shown red without touching the tree:
//   IDEASRELATE_SRC=<copy of ideasrelate.ts>  COMPANION_SRC=<copy of Companion.tsx>
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const here = (p) => fileURLToPath(new URL(p, import.meta.url));
const SRC = process.env.IDEASRELATE_SRC ?? here("../companion/ideasrelate.ts");
const COMP = process.env.COMPANION_SRC ?? here("../companion/Companion.tsx");
let failed = 0;
const check = (c, m) => { if (!c) failed++; console.log(`${c ? "ok" : "not ok"} — ${m}`); };

const src = fs.readFileSync(SRC, "utf8");
check(!/^\s*import\s(?!type\b)/m.test(src), "ideasrelate.ts has no runtime import (this check loads it as a data: module)");
const js = (await transformWithEsbuild(src, "ideasrelate.ts", { loader: "ts" })).code;
const R = await import(`data:text/javascript;base64,${Buffer.from(js).toString("base64")}`);

const A = "01JC8Z4K3M9QX2T7VBN5R6WYHA", B = "01JC8Z4K3M9QX2T7VBN5R6WYHB", C = "01JC8Z4K3M9QX2T7VBN5R6WYHC";

// ── the drop rules (§6.1) ───────────────────────────────────────────────────
check(R.ideaDrop(A, { id: B, title: "B" }) === B, "an idea onto another idea is a target");
check(R.ideaDrop(A, { id: A, title: "A" }) === null, "an idea onto itself is no target");
check(R.ideaDrop(A, { id: null, title: "hand", problem: "no-id" }) === null, "a hand-made file (no id) is no target");
check(R.ideaDrop(A, { id: B, title: "B", problem: "duplicate-id" }) === null, "a row with a problem is no target");
check(R.ideaDrop(A, null) === null, "nothing under the pointer is no target");
check(R.combineDrop([A]) === null, "one idea onto New idea combines nothing");
check(JSON.stringify(R.combineDrop([A, B, A, "x"])) === JSON.stringify([A, B]), "Combine takes each idea once, ids only");
check(R.CHOICES[0] === "relate" && R.CHOICES.includes("merge") && R.CHOICES.length === 2, "the chooser offers Relate (first: Enter) and Merge into this");
check(R.ASKS.relate === false && R.ASKS.merge === true, "relate never asks; merge asks or offers Undo");

// ── each call is a row serve has, with arguments it reads ───────────────────
const dispatch = fs.readFileSync(here("../../crates/worktrees-cli/src/serve/dispatch.rs"), "utf8");
const camel = (k) => k.replace(/_([a-z])/g, (_, c) => c.toUpperCase());
/** The arguments a row's body reads (`a.get("…")`), camelCase as the page sends them. */
const rowArgs = (cmd) => {
  const line = dispatch.split("\n").find((l) => l.trimStart().startsWith(`${cmd}: `));
  return line ? [...line.matchAll(/a\.get\("([a-z_]+)"\)/g)].map((m) => camel(m[1])) : null;
};
const calls = {
  relate: R.relateCall(A, B),
  merge: R.mergeCall(A, B, { into: 1, from: 2 }),
  mergeBare: R.mergeCall(A, B),
  combine: R.combineCall([A, B], "  Both "),
  split: R.splitCall(A, 3, 9, true, 42, " T "),
  splitMove: R.splitCall(A, 3, 9, false, 42),
};
for (const [name, c] of Object.entries(calls)) {
  const want = rowArgs(c.cmd);
  const extra = want ? Object.keys(c.args).filter((k) => !want.includes(k)) : null;
  check(want && extra.length === 0, `${name}: ${c.cmd} is a row, and reads every argument the page sends (${want ? (extra.length ? `unread: ${extra}` : Object.keys(c.args).join(", ")) : "no such row"})`);
  check(/HostConsole/.test(dispatch.split("\n").find((l) => l.trimStart().startsWith(`${c.cmd}: `)) ?? ""), `${c.cmd} is a console row (desktop only)`);
}
for (const cmd of ["idea_related", "idea_versions", "idea_restore"]) check(rowArgs(cmd) !== null, `${cmd} is a row`);
check(calls.relate.args.id === A && calls.relate.args.other === B, "relate(a, b): a is the idea whose related: is written");
check(calls.merge.args.into === A && calls.merge.args.from === B && calls.merge.args.expectedInto === 1 && calls.merge.args.expectedFrom === 2, "merge(into, from) with the open idea's mtime");
check(!("expectedInto" in calls.mergeBare.args) && !("text" in calls.mergeBare.args), "merge sends no mtime it does not have, and no text (the default append)");
check(calls.combine.args.title === "Both" && calls.combine.args.ids.length === 2, "combine(ids, title), the title trimmed");
check(R.combineCall([A, B], "  ").args.title === "Combined idea", "an empty combine title is a name, not nothing");
check(calls.split.args.extract === true && calls.split.args.title === "T" && calls.split.args.expectedMtime === 42, "split with ⌥ (or the box) extracts; the mtime names the text the offsets count in");
check(!("extract" in calls.splitMove.args) && !("title" in calls.splitMove.args), "a plain split moves the text, its title from the first line");

// ── Companion.tsx: one function per act, and no second door ────────────────
const comp = fs.readFileSync(COMP, "utf8");
for (const cmd of ["idea_relate", "idea_merge", "idea_combine", "idea_split"]) {
  check(!comp.includes(`"${cmd}"`), `Companion.tsx invokes ${cmd} only through ideasrelate.ts's builder`);
}
/** The text of the call site that handles `name`. */
const choose = comp.slice(comp.indexOf("const choose = "), comp.indexOf("const choose = ") + 400);
check(/c === "relate"\) void doRelate\(ch\.from, ch\.onto\)/.test(choose) && /void doMerge\(ch\.onto, ch\.from\)/.test(choose),
  "the drop's chooser: Relate relates the dragged idea to the target; Merge merges the dragged one INTO the target");
check(/p\.act === "relate"\) void doRelate\(p\.id, r\.id!\); else void doMerge\(r\.id!, p\.id\)/.test(comp),
  "the menu path (Relate to… / Merge into…) ends in the same doRelate / doMerge, the same way round");
check(/if \(target\.kind === "new"\) \{ setCombineAsk\(target\.ids\)/.test(comp) && /onCombine=\{\(\) => setCombineAsk\(selected\)\}/.test(comp),
  "the drop on New idea and the Combine button both open the same ask (Combine asks first)");
check(/void doSplit\(sel, extract\)/.test(comp) && /void doSplit\(splitAsk, o\.extract, o\.title\)/.test(comp),
  "the text drop and the Split… menu both end in doSplit");
check(/setUndo\(\{ label: "Undo merge"/.test(comp) && /setUndo\(\{ label: "Undo combine"/.test(comp) && /setUndo\(\{ label: `Undo \$\{extract/.test(comp),
  "merge, combine and split each offer Undo");
check(!/setUndo\(\{[^}]*relate/i.test(comp), "relate offers no Undo prompt (it never asks; Versions undoes it)");

// ── `[[` and the links ──────────────────────────────────────────────────────
check(JSON.stringify(R.pickerQuery("see [[jel")) === JSON.stringify({ from: 4, query: "jel" }), "`[[jel` before the cursor opens the picker on `jel`");
check(R.pickerQuery("see [[jel]] x") === null && R.pickerQuery("see [jel") === null, "no picker after a closed `]]` or a single `[`");
const rows = [{ id: A, title: "Jellyfin sharing" }, { id: B, title: "Home jellyfin" }, { id: C, title: "Other" }, { id: null, title: "Jelly hand", problem: "no-id" }];
const m = R.pickerMatches(rows, "jel", null).map((r) => r.id);
check(JSON.stringify(m) === JSON.stringify([A, B]), "the picker matches titles, a title that starts with the query first, never a row with no id");
check(!R.pickerMatches(rows, "", A).some((r) => r.id === A), "the open idea is never offered");
const link = R.ideaLink("Plan [v2] \\ draft", B);
check(link === `[Plan \\[v2\\] \\\\ draft](idea:${B})`, `the link is by id, its brackets escaped (${link})`);
check(R.linkAt(`see ${link} now`, 6) === B && R.linkAt(`see ${link} now`, 4 + link.length - 2) === B, "⌘-click anywhere on the link finds its id");
check(R.linkAt(`see ${link} now`, 1) === null, "outside the link, nothing");
check(R.linkAt(`bare idea:${C}.`, 7) === C, "a bare idea:<id> is a link too");
const body = `x ${link} y`;
const read = R.readableLinks(body);
check(read.includes(`](#idea:${B})`) && R.ideaHref(`#idea:${B}`) === B && R.ideaHref("https://x") === null,
  "the Read view turns idea: links into #idea: ones the renderer links, and reads them back");
// Core reads the same shape: links_in finds `idea:` + a ULID; split writes `[label](idea:id)`.
const ideasRs = fs.readFileSync(here("../../crates/worktrees-core/src/ideas.rs"), "utf8");
check(/\{lead\}\[\{label\}\]\(idea:\{new_id\}\)\{trail\}/.test(ideasRs) && /match_indices\("idea:"\)/.test(ideasRs),
  "core's split writes `[label](idea:<id>)` and links_in reads `idea:<id>` — the shape ideaLink inserts");
check(/replace\('\\\\', "\\\\\\\\"\)\.replace\('\[', "\\\\\["\)\.replace\('\]', "\\\\\]"\)/.test(ideasRs),
  "core escapes a split link's label as ideaLink does (\\ [ ])");
check(R.mergedNote({ id: B, title: "Beta", date: "2026-11-12" }, "Alpha") === "“Beta” was merged into “Alpha” on 2026-11-12.", "an absorbed id lands with its note");

// ── selection ───────────────────────────────────────────────────────────────
const order = [A, B, C];
check(JSON.stringify(R.nextSelection([A], C, { toggle: true, range: false }, order, A)) === JSON.stringify([A, C]), "⌘-click adds a row");
check(JSON.stringify(R.nextSelection([A, C], C, { toggle: true, range: false }, order, A)) === JSON.stringify([A]), "⌘-click on a selected row drops it");
check(JSON.stringify(R.nextSelection([A], C, { toggle: false, range: true }, order, A)) === JSON.stringify([A, B, C]), "Shift-click selects the run");

console.log(failed ? `\nideas-relate-check: ${failed} failure(s)` : "\nideas-relate-check: all ok");
process.exit(failed ? 1 : 0);
