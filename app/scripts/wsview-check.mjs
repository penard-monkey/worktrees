// The workspace view's pure rules (app/src/wsview.ts), run: the card's dot, the
// attention line, a tab that is not drawn, and the document pane's steps
// (doc/useTier.ts, doc/docmode.ts).
//
//   node app/scripts/wsview-check.mjs     # exits non-zero on failure
//
// Which tabs each beta draws is beta-check.mjs's. What the screens do is
// workspace-view-browser.mjs's and documents-browser.mjs's.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const read = (rel) => fs.readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");
const load = async (src) => import("data:text/javascript;base64," + Buffer.from((await transformWithEsbuild(src, "x.ts", { loader: "ts", format: "esm" })).code).toString("base64"));
let failed = 0;
const eq = (name, got, want) => {
  const g = JSON.stringify(got), w = JSON.stringify(want);
  if (g === w) return;
  failed++;
  console.log(`not ok — ${name}\n     got: ${g}\n    want: ${w}`);
};
const src = read("../src/wsview.ts");
eq("wsview.ts has no runtime import", /^import (?!type )/m.test(src), false);
const V = await load(src.replace(/^import type .*$/gm, ""));

// ── the dot: waiting, then working, then just done ──
const roll = (...xs) => V.rollupOf(xs, (x) => x, (x) => x === "done");
eq("one waiting among working is waiting", roll("busy", "waiting", "busy"), "waiting");
eq("one working among done is busy", roll("done", "busy"), "busy");
eq("done", roll("", "done"), "done");
eq("nothing", roll("", ""), "");
eq("no items", roll(), "");

// ── one place's mark: its agents roll up, and one waiting beats one working ──
eq("a place with a working and a waiting agent is waiting", V.placeActivity(new Set(["/p"]), new Set(["/p"]), "/p"), "waiting");
eq("…working alone, waiting alone, neither", [V.placeActivity(new Set(["/p"]), new Set(), "/p"), V.placeActivity(new Set(), new Set(["/p"]), "/p"), V.placeActivity(new Set(["/q"]), new Set(["/q"]), "/p")], ["busy", "waiting", ""]);
// ── a chat's part in its workspace's dot ──
eq("a chat's row state as the card's dot reads it", ["waiting", "fold-failed", "busy", "folding", "checking", "running", "ended"].map(V.chatActivity), ["waiting", "waiting", "busy", "busy", "busy", "", ""]);
eq("a waiting chat under a working lane: the card is amber", roll(V.placeActivity(new Set(["/l"]), new Set(), "/l"), V.chatActivity("waiting")), "waiting");

// ── the attention line ──
const none = { unsaved: null, problems: [], planError: null };
eq("nothing needs anybody", V.attentionOf(none), []);
eq("an unsaved document here", V.attentionOf({ ...none, unsaved: { title: "Plan", elsewhere: null } }), [{ kind: "unsaved", text: "“Plan” has text that is not saved." }]);
eq("…in another workspace names it", V.attentionOf({ ...none, unsaved: { title: "Plan", elsewhere: "Jellyfin" } })[0].text, "“Plan”, in Jellyfin, has text that is not saved.");
eq("one problem", V.attentionOf({ ...none, problems: ["Line 3: x"] }), [{ kind: "plan", text: "The plan has a problem. Line 3: x" }]);
eq("more than one: the count and the first", V.attentionOf({ ...none, problems: ["Line 3: x", "Line 9: y"] })[0].text, "The plan has 2 problems. Line 3: x");
eq("a plan that cannot be read has no problems to count", V.attentionOf({ ...none, problems: ["Line 3: x"], planError: "it is gone" }), [{ kind: "plan", text: "The plan cannot be read: it is gone." }]);
eq("the document first", V.attentionOf({ unsaved: { title: "P", elsewhere: null }, problems: ["L"], planError: null }).map((a) => a.kind), ["unsaved", "plan"]);

eq("the last view, as anything may be stored", ['{"id":"w","tab":"ideas","doc":"i1","chat":"c1","x":1}', '{"id":"w","tab":"settings"}', '{"tab":"plan"}', "{", "null", null].map(V.lastView), [{ id: "w", tab: "ideas", doc: "i1", chat: "c1" }, null, null, null, null, null]);
eq("a tab that is not drawn is Overview", [V.tabOr("plan", ["overview", "memory"]), V.tabOr("memory", ["overview", "memory"])], ["overview", "memory"]);
eq("a chat that waits is first, one line each", V.attentionOf({ ...none, unsaved: { title: "P", elsewhere: null }, chats: [{ conv: "c1", title: "Who", failed: false }, { conv: "c2", title: "Sub", failed: true }] }).map((a) => a.text),
  ["“Who” is waiting for an answer.", "The fold of “Sub” did not finish.", "“P” has text that is not saved."]);
eq("live chats are counted", [V.wsCounts(3, 2), V.wsCounts(1, 1), V.wsCounts(2, 0)], ["3 projects · 2 chats live", "1 project · 1 chat live", "2 projects"]);
// ── a text that is not saved keeps its tab ──
const bare = V.wsTabs({ ideas: false, chats: false }), chats = V.wsTabs({ ideas: false, chats: true });
eq("its workspace's, in its place, and no other's", [V.tabsWith(bare, "A", { ws: "A", tab: "memory" }), V.tabsWith(chats, "A", { ws: "A", tab: "ideas" }), V.tabsWith(chats, "B", { ws: "A", tab: "ideas" }), V.tabsWith(chats, "A", { ws: "A", tab: "memory" }), V.tabsWith(bare, "A", null)],
  [["overview", "memory"], ["overview", "memory", "ideas", "chats"], chats, chats, ["overview"]]);
// ── a place's workspace ──
const WS = [{ id: "a", projects: [{ root: "/p" }, { root: "/q" }] }, { id: "b", projects: [{ root: "/p" }] }, { id: "c", projects: [] }];
const holds = (r) => V.placeWorkspaces(WS, r);
eq("the workspaces that hold its project", [holds("/p").map((w) => w.id), holds("/q").map((w) => w.id), holds("/z")], [["a", "b"], ["a"], []]);
eq("the card it was clicked in; else the only one; else nobody has said", [V.placeWorkspace(holds("/p"), "b")?.id, V.placeWorkspace(holds("/q"), null)?.id, V.placeWorkspace(holds("/p"), null), V.placeWorkspace(holds("/z"), "a")], ["b", "a", null, null]);
eq("a card that no longer holds it is not it", [V.placeWorkspace(holds("/q"), "b")?.id, V.placeWorkspace(holds("/p"), "gone")], ["a", null]);
eq("what is stored, as anything may be", ['{"x":"a","y":1,"z":""}', "[]", "{", null, "7"].map(V.placeWsRead), [{ x: "a" }, {}, {}, {}, {}]);
const full = Object.fromEntries(Array.from({ length: V.PLACE_WS_MAX }, (_, i) => [`k${i}`, "a"]));
const more = V.placeWsSet(full, "new", "b");
eq("bounded: the oldest goes", [Object.keys(more).length, "k0" in more, more.new], [V.PLACE_WS_MAX, false, "b"]);
eq("from All projects it is forgotten", V.placeWsSet({ x: "a", y: "b" }, "x", null), { y: "b" });
eq("counts", [V.wsCounts(0), V.wsCounts(1), V.wsCounts(3)], ["0 projects", "1 project", "3 projects"]);

// ── the pane's header by its own width (§4.1) ──
const T = await load(read("../src/doc/useTier.ts").replace(/^import .*$/gm, "const useState = () => [], useLayoutEffect = () => {};"));
const pane = read("../src/doc/DocumentPane.tsx");
const breaks = [Number(pane.match(/export const NARROW = (\d+);/)?.[1]), Number(pane.match(/export const FULL = (\d+);/)?.[1])];
eq("the steps are the design's", breaks, [420, 640]);
eq("the header's steps", [340, 419, 420, 639, 640, 1052].map((w) => T.tierOf(w, breaks)), [0, 0, 1, 1, 2, 2]);
eq("the pane uses them, and its own width", /const BREAKS = \[NARROW, FULL\] as const;/.test(pane) && /useTier\(head, BREAKS\)/.test(pane), true);

// ── Edit or Read, per kind ──
globalThis.window = undefined;
const M = await load(read("../src/doc/docmode.ts"));
eq("with no storage: ideas in Edit, plans and memory in Read", ["idea", "plan", "memory"].map(M.startMode), ["edit", "read", "read"]);
const kept = new Map();
globalThis.window = { localStorage: { getItem: (k) => kept.get(k) ?? null, setItem: (k, v) => kept.set(k, v) } };
M.rememberMode("plan", "edit");
eq("a choice is its kind's alone", ["idea", "plan", "memory"].map(M.startMode), ["edit", "edit", "read"]);
kept.set("wt.doc.mode.idea", "junk");
eq("junk in storage is the default", M.startMode("idea"), "edit");
globalThis.window = { get localStorage() { throw new Error("blocked"); } };
M.rememberMode("idea", "read");
eq("blocked storage is the default, and no error", M.startMode("plan"), "read");

if (failed) { console.log(`\nwsview-check: ${failed} failed`); process.exit(1); }
console.log("wsview-check: all ok");
