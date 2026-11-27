// Checks the navigation-history RULES — app/src/navHistory.ts — by evaluating
// the real module (no DOM, no React: it is pure on purpose).
//
//   node app/scripts/navhistory-check.mjs     # exits non-zero on failure
//
// What it pins: push / same-location no-op / the 600 ms coalesce (sliding) /
// amend keeps forward / back-then-push truncates / cap / skipping dead entries
// / prune moving the current entry / normalisation (file only on Files, shell
// only on Terminal), and the labels the buttons show.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const SRC = fileURLToPath(new URL("../src/navHistory.ts", import.meta.url));
const js = (await transformWithEsbuild(fs.readFileSync(SRC, "utf8"), "navHistory.ts", {
  loader: "ts", format: "esm",
})).code;
const H = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

let failed = 0;
const eq = (name, got, want) => {
  const g = JSON.stringify(got), w = JSON.stringify(want);
  if (g === w) return;
  failed++;
  console.log(`not ok — ${name}\n     got: ${g}\n    want: ${w}`);
};

const P = (slug, dock) => ({ place: { repo: "/r", slug }, ...(dock ? { dock } : {}) });
const HOME = { place: null };
const slugs = (h) => h.entries.map((e) => (e.place ? e.place.slug : "~") + (e.dock?.file ? ":" + e.dock.file.path : ""));
const W = H.COALESCE_MS;

// ── push ────────────────────────────────────────────────────────────────────
let h = H.empty();
eq("empty has no current", H.current(h), null);
h = H.record(h, HOME, 1000);
h = H.record(h, P("a"), 1000 + W);
h = H.record(h, P("b"), 1000 + 2 * W);
eq("pushes stack", [slugs(h), h.index], [["~", "a", "b"], 2]);
eq("same location twice is nothing", H.record(h, P("b"), 99_999), h);

// ── coalesce ────────────────────────────────────────────────────────────────
let c = H.record(h, P("b", { tab: "files", file: { path: "x.rs" } }), 1000 + 2 * W + 50);
eq("a change inside the window REPLACES the top", [slugs(c), c.index], [["~", "a", "b:x.rs"], 2]);
c = H.record(c, P("c"), 1000 + 2 * W + 50 + W - 1);
eq("the window slides (steady quick steps fold)", slugs(c), ["~", "a", "c"]);
c = H.record(c, P("a"), 1000 + 2 * W + 50 + W);
eq("coalescing back onto the previous entry merges with it", [slugs(c), c.index], [["~", "a"], 1]);
c = H.record(c, P("d"), 1_000_000);
eq("outside the window it pushes", slugs(c), ["~", "a", "d"]);

// ── back / forward ──────────────────────────────────────────────────────────
let b = H.step(h, -1);
eq("back moves the index", [b.index, H.current(b).place.slug], [1, "a"]);
eq("back clears the coalesce clock", b.at, 0);
eq("can go forward after back", H.canStep(b, 1), true);
eq("cannot go back past the start", H.canStep(H.step(b, -1), -1), false);
eq("step at an edge is a no-op", H.step(H.step(b, -1), -1).index, 0);
const b2 = H.record(b, P("z"), b.at + 1); // at=0 → never coalesces
eq("push after back truncates forward, never coalesces into the entry left", slugs(b2), ["~", "a", "z"]);
const am = H.record(b, P("a", { tab: "plan" }), 5, "amend");
eq("amend replaces the current entry and KEEPS forward", [slugs(am), am.index, am.entries[1].dock.tab], [["~", "a", "b"], 1, "plan"]);
const alive = (l) => !(l.place && l.place.slug === "a");
eq("back skips entries that are not alive", H.step(h, -1, alive).index, 0);
eq("reachable lists nearest first", H.reachable(h, -1), [1, 0]);
eq("canStep honours alive", H.canStep(H.goTo(h, 1), -1, (l) => l.place !== null), false);

// ── cap ─────────────────────────────────────────────────────────────────────
let big = H.empty();
for (let i = 0; i < H.CAP + 7; i++) big = H.record(big, P("p" + i), i * 10 * W);
eq("capped at CAP, oldest dropped", [big.entries.length, big.entries[0].place.slug, big.index], [H.CAP, "p7", H.CAP - 1]);

// ── prune ───────────────────────────────────────────────────────────────────
let pr = H.record(h, P("a"), 10 * W); // ~ a b a
pr = H.goTo(pr, 2); // on b
const gone = H.prune(pr, (l) => l.place?.slug === "b");
eq("prune drops entries, merges the duplicates it makes, current → survivor before", [slugs(gone), gone.index], [["~", "a"], 1]);
const keepCur = H.prune(H.goTo(pr, 3), (l) => l.place === null);
eq("prune keeps the current entry where it survives", [slugs(keepCur), keepCur.index], [["a", "b", "a"], 2]);
eq("prune with nothing dead is identity", H.prune(pr, () => false), pr);
eq("prune of everything empties", H.prune(pr, () => true), { entries: [], index: -1, at: 0 });

// ── normalisation ───────────────────────────────────────────────────────────
eq("a file off the Files tab is not part of the location",
  H.locKey(P("a", { tab: "terminal", file: { path: "x" }, shell: 2 })), H.locKey(P("a", { tab: "terminal", shell: 2 })));
eq("a shell off the Terminal tab is not part of the location",
  H.locKey(P("a", { tab: "files", shell: 2 })), H.locKey(P("a", { tab: "files" })));
eq("a different line is a different location",
  H.locKey(P("a", { tab: "files", file: { path: "x", line: 3 } })) === H.locKey(P("a", { tab: "files", file: { path: "x", line: 4 } })), false);
eq("Home drops everything else", H.locKey({ place: null, agent: "claude", dock: { tab: "files" } }), H.locKey(HOME));

// ── labels ──────────────────────────────────────────────────────────────────
eq("label: home", H.label(HOME), "Home");
eq("label: file at line", H.label(P("a", { tab: "files", file: { path: "src/App.tsx", line: 12 } })), "a · App.tsx:12");
eq("label: shell", H.label(P("a", { tab: "terminal", shell: 2 })), "a · Terminal · sh 2");
eq("label: tab, with nameOf", H.label(P("a", { tab: "docs" }), () => "Alpha"), "Alpha · Docs");
eq("label: a bare Files tab is the default and goes unsaid", H.label(P("a", { tab: "files" })), "a");

// ── the workspace view ──────────────────────────────────────────────────────
const V = (tab, doc, id = "w1") => ({ place: null, ws: { id, tab, ...(doc ? { doc } : {}) } });
eq("a workspace is not Home", H.locKey(V("overview")) === H.locKey(HOME), false);
eq("two tabs of it are two locations", H.locKey(V("plan")) === H.locKey(V("overview")), false);
eq("…and two ideas on one tab", H.locKey(V("ideas", "a")) === H.locKey(V("ideas", "b")), false);
eq("a file on the Memory tab is its own location", H.locKey(V("memory", "a.md")) === H.locKey(V("memory", "b.md")), false);
eq("an idea means nothing off the Ideas tab", H.locKey(V("plan", "a")), H.locKey(V("plan")));
eq("a place drops a workspace left beside it", H.normalize({ ...P("a"), ws: { id: "w1", tab: "plan" } }), P("a"));
let v = H.record(H.empty(), HOME, 1000);
v = H.record(v, V("overview"), 1000 + W);
v = H.record(v, V("plan"), 1000 + 2 * W);
v = H.record(v, P("a"), 1000 + 3 * W);
eq("Home, Overview, Plan and a place stack", v.entries.length, 4);
eq("Back from the place lands on the Plan tab", H.current(H.step(v, -1)), V("plan"));
eq("…skipping a workspace that is gone", H.current(H.step(v, -1, (l) => !l.ws)), HOME);
eq("a removed workspace's entries are pruned", H.prune(v, (l) => l.ws?.id === "w1").entries, [HOME, P("a")]);
eq("where: a tab of the same workspace is the same where", H.whereKey(V("plan")) === H.whereKey(V("ideas", "a")), true);
eq("…another workspace, Home and a place are not", new Set([V("plan"), V("plan", null, "w2"), HOME, P("a")].map(H.whereKey)).size, 4);
eq("its label is the workspace's name and the tab", H.label(V("plan"), undefined, () => "Jellyfin"), "Jellyfin · Plan");
eq("…and never Home", H.label(V("overview")), "A workspace · Overview");
const C = (tab, chat) => ({ place: null, ws: { id: "w1", tab, chat } });
eq("a chat beside a tab is another location", H.locKey(C("plan", "c1")) === H.locKey(V("plan")), false);
eq("…and two chats are two", H.locKey(C("plan", "c1")) === H.locKey(C("plan", "c2")), false);
eq("…in the same where", H.whereKey(C("plan", "c1")), H.whereKey(V("ideas", "a")));
eq("its label names the chat", H.label(C("plan", "c1"), undefined, () => "Jellyfin", () => "Release order"), "Jellyfin · Release order · Plan");

// ── Disk ────────────────────────────────────────────────────────────────────
const D = (scope, path, under = HOME) => ({ ...under, disk: { scope, ...(path ? { path } : {}) } });
const DP = { kind: "place", repo: "/r", slug: "b" };
eq("Disk over a place is not the place", H.locKey(D(DP, null, P("a"))) === H.locKey(P("a")), false);
eq("…nor another place's Disk over it", H.whereKey(D(DP, null, P("a"))) === H.whereKey(D({ kind: "place", repo: "/r", slug: "c" }, null, P("a"))), false);
eq("a picked folder is a step in the same where", H.whereKey(D(DP, "target")), H.whereKey(D(DP)));
eq("…and a location of its own", H.locKey(D(DP, "target")) === H.locKey(D(DP)), false);
eq("only a place's treemap picks: a path on a project's Disk is dropped",
  H.normalize(D({ kind: "project", root: "/r" }, "x")), D({ kind: "project", root: "/r" }));
eq("stray scope fields are dropped", H.normalize({ place: null, disk: { scope: { kind: "project", root: "/r", junk: 1 } } }), D({ kind: "project", root: "/r" }));
{
  let h = H.empty();
  for (const [i, l] of [HOME, D({ kind: "project", root: "/r" }), D(DP), D(DP, "target/debug")].entries()) h = H.record(h, l, 10_000 * (i + 1));
  eq("the owner's walk is four entries", h.entries.length, 4);
  h = H.step(h, -1);
  eq("Back from the folder: the place's treemap", H.current(h), D(DP));
  h = H.step(h, -1);
  eq("…then the project's Disk it was opened from", H.current(h), D({ kind: "project", root: "/r" }));
  h = H.step(h, -1);
  eq("…then Home, where Disk was opened", H.current(h), HOME);
  eq("…and Forward goes the other way", H.current(H.step(h, 1)), D({ kind: "project", root: "/r" }));
}
eq("Disk labels: a place", H.label(D(DP, "target/debug"), () => "Bravo"), "Disk · Bravo · debug");
eq("…a project", H.label(D({ kind: "project", root: "/w/worktrees" })), "Disk · worktrees");
eq("…a workspace", H.label(D({ kind: "workspace", id: "w1" }), undefined, () => "Jellyfin"), "Disk · Jellyfin");
eq("…everything", H.label(D({ kind: "all" })), "Disk · All projects");

// ── the Docs tab's chosen document ──────────────────────────────────────────
// Docs and Files behave the same: what is chosen in the list is a part of the
// location, so Back to the list shows the one you chose and Forward opens it.
const DOC = (doc) => P("a", { tab: "docs", ...(doc ? { doc } : {}) });
eq("a document chosen on Docs is its own location", H.locKey(DOC("README.md")) === H.locKey(DOC()), false);
eq("…and two documents are two", H.locKey(DOC("README.md")) === H.locKey(DOC("docs/a.md")), false);
eq("…in the same where", H.whereKey(DOC("README.md")), H.whereKey(DOC()));
eq("a document means nothing off the Docs tab", H.locKey(P("a", { tab: "files", doc: "README.md" })), H.locKey(P("a", { tab: "files" })));
eq("…nor a file on Docs", H.locKey(P("a", { tab: "docs", file: { path: "x" } })), H.locKey(DOC()));
eq("its label names the document", H.label(DOC("docs/adr/0001.md")), "a · Docs · 0001.md");
{
  // Docs → choose README (the entry is amended: the list now shows it chosen,
  // and the file opens in Files) → Docs again → choose CLAUDE → Back ×3.
  let h = H.empty();
  h = H.record(h, DOC(), 10_000);
  h = H.record(h, DOC("README.md"), 20_000, "amend");
  h = H.record(h, P("a", { tab: "files", file: { path: "/a/README.md" } }), 20_050);
  h = H.record(h, DOC("README.md"), 30_000);
  h = H.record(h, DOC("CLAUDE.md"), 40_000, "amend");
  h = H.record(h, P("a", { tab: "files", file: { path: "/a/CLAUDE.md" } }), 40_050);
  eq("the walk is four entries", h.entries.map((e) => H.label(e)), ["a · Docs · README.md", "a · README.md", "a · Docs · CLAUDE.md", "a · CLAUDE.md"]);
  eq("Back from the file: the list, with it chosen", H.current(H.step(h, -1)), DOC("CLAUDE.md"));
  eq("…Back again: the document before", H.current(H.step(H.step(h, -1), -1)).dock.file.path, "/a/README.md");
}

// ── a Memory file's History ─────────────────────────────────────────────────
const M = (doc, hist) => ({ place: null, ws: { id: "w1", tab: "memory", doc, ...(hist ? { hist } : {}) } });
eq("History open is its own location", H.locKey(M("a.md", {})) === H.locKey(M("a.md")), false);
eq("…and one change in it another", H.locKey(M("a.md", { at: "c1" })) === H.locKey(M("a.md", {})), false);
eq("…and two changes are two", H.locKey(M("a.md", { at: "c1" })) === H.locKey(M("a.md", { at: "c2" })), false);
eq("…all in the same where", new Set([M("a.md"), M("a.md", {}), M("a.md", { at: "c1" })].map(H.whereKey)).size, 1);
eq("History means nothing off the Memory tab", H.locKey({ place: null, ws: { id: "w1", tab: "plan", hist: { at: "c1" } } }), H.locKey(V("plan")));
eq("stray History fields are dropped", H.normalize(M("a.md", { at: "", junk: 1 })), M("a.md", {}));
eq("labels: History", H.label(M("a.md", {}), undefined, () => "Jellyfin"), "Jellyfin · Memory · History");
eq("…and a change in it", H.label(M("a.md", { at: "c1" }), undefined, () => "Jellyfin"), "Jellyfin · Memory · A change in History");
{
  let h = H.empty();
  for (const [i, l] of [M("a.md"), M("a.md", {}), M("a.md", { at: "c1" })].entries()) h = H.record(h, l, 10_000 * (i + 1));
  eq("the file, its History, a change: three entries", h.entries.length, 3);
  h = H.step(h, -1);
  eq("Back from a change: the History list", H.current(h), M("a.md", {}));
  h = H.step(h, -1);
  eq("…Back again: the file you were reading", H.current(h), M("a.md"));
  eq("…and Forward the change again", H.current(H.step(H.step(h, 1), 1)), M("a.md", { at: "c1" }));
  eq("a change that is gone is skipped", H.current(H.step(H.goTo(h, 2), -1, (l) => !l.ws?.hist || !!l.ws.hist.at)), M("a.md"));
}

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("navhistory-check: all ok");
