// A workspace's chats: the pure rules (app/src/wschats.ts on desk.ts), run, and
// the one table of the two desk kinds' rows (app/src/chat.ts).
//
//   node app/scripts/wschats-check.mjs     # exits non-zero on failure
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const read = (rel) => fs.readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");
let failed = 0;
const eq = (name, got, want) => {
  const g = JSON.stringify(got), w = JSON.stringify(want);
  if (g === w) return;
  failed++;
  console.log(`not ok — ${name}\n     got: ${g}\n    want: ${w}`);
};
const strip = (s) => s.replace(/^import .*$/gm, "");
const js = (await transformWithEsbuild(strip(read("../src/desk.ts")) + strip(read("../src/wschats.ts")), "x.ts", { loader: "ts", format: "esm" })).code;
const W = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

const iso = (min) => new Date(Date.UTC(2026, 10, 24, 12, 0) - min * 60_000).toISOString();
const chat = (id, over = {}) => ({ id, harness: "claude", session_id: "s", title: `Chat ${id}`, started: iso(1000), last_activity: iso(10), note: "", session: `s~${id}`, state: "ended", note_exists: false, ...over });
const ev = (over = {}) => ({ live: [], busy: [], waiting: [], chat_of: {}, fold: {}, ...over });

// ── the order, and what the nav keeps ──
const chats = [
  chat("idle", { last_activity: iso(1) }), chat("busy", { last_activity: iso(500) }), chat("wait", { last_activity: iso(900) }),
  chat("fold", { last_activity: iso(2) }), chat("bad", { last_activity: iso(3) }),
  chat("c1", { last_activity: iso(100) }), chat("c2", { last_activity: iso(50) }), chat("c3", { last_activity: iso(200) }), chat("c4", { last_activity: iso(5) }),
];
const now = ev({ live: ["s~idle", "s~busy", "s~wait", "s~fold"], busy: ["s~busy"], waiting: ["s~wait"], fold: { "s~fold": { state: "folding" }, "s~bad": { state: "failed" } } });
const rows = W.chatRows(chats, now);
eq("waiting, then a failed fold, working, folding, open, closed (the newest first)", rows.map((r) => r.chat.id), ["wait", "bad", "busy", "fold", "idle", "c4", "c2", "c1", "c3"]);
eq("their states", rows.slice(0, 6).map((r) => r.state), ["waiting", "fold-failed", "busy", "folding", "running", "ended"]);
eq("the nav: all live, three closed, and a way to the rest", [W.navRows(rows).rows.map((r) => r.chat.id), W.navRows(rows).rest], [["wait", "bad", "busy", "fold", "idle", "c4", "c2", "c1"], true]);
eq("three closed are all of them", W.navRows(W.chatRows(chats.slice(5, 8), ev())).rest, false);
eq("a filter is every title that matches, closed or not", W.navRows(rows, "chat c").rows.map((r) => r.chat.id), ["c4", "c2", "c1", "c3"]);
eq("a fold that waits on the person is waiting", W.rowOf(chat("x"), ev({ live: ["s~x"], fold: { "s~x": { state: "waiting" } } }), null).state, "waiting");
eq("an unknown fold word changes nothing", W.rowOf(chat("x"), ev({ live: ["s~x"], fold: { "s~x": { state: "compacting" } } }), null).state, "running");
eq("the push wins over the list", W.rowOf(chat("x", { state: "busy" }), ev(), null).state, "ended");
eq("before any push the list is it", W.rowOf(chat("x", { state: "busy" }), null, null).state, "busy");
eq("…except for what this window just did", [W.rowOf(chat("x"), ev(), { conv: "x", live: true }).state, W.rowOf(chat("x"), ev({ live: ["s~x"] }), { conv: "x", live: false }).state], ["running", "ended"]);

// ── words ──
const r = (state, fold) => ({ chat: chat("x", fold ? { fold } : {}), state, live: state !== "ended" });
eq("the age gains a word when it needs somebody", ["waiting", "fold-failed", "folding", "busy", "ended"].map((s) => W.ageWords(r(s), "12m").text), ["waiting 12m", "fold failed", "folding…", "12m", "12m"]);
eq("…and reads strong then", ["waiting", "busy"].map((s) => W.ageWords(r(s), "1m").strong), [true, false]);
eq("closed, by its fold", [undefined, { state: "done", commit: "a", reverted: false }, { state: "done", reverted: false }, { state: "done", commit: "a", reverted: true }, { state: "failed", reverted: false }, { state: "???", reverted: false }].map((f) => W.stateWords(r("ended", f))),
  ["closed, not folded", "closed · folded into memory", "closed · no memory changes", "closed · its fold was undone", "closed, not folded", "closed, not folded"]);
const p = (reads, why) => ({ root: "/r", name: "r", registered: true, private: false, reads, ...(why ? { why } : {}) });
eq("what it reads", [W.readsWords([p("yes"), p("yes"), p("no", "private")]), W.readsWords([])], ["reads 2 of 3", "reads no project"]);
eq("the first line", [W.introWords([p("yes"), p("no")]), W.introWords([p("yes")]), W.introWords([])], [
  "It reads the memory index first, and it can read 1 of 2 projects. It is told not to write in a project. Claude's file tools are blocked there; programs it runs are not.",
  "It reads the memory index first, and it can read its project. It is told not to write in a project. Claude's file tools are blocked there; programs it runs are not.",
  "It reads the memory index first, and the workspace has no projects to read. It is told not to write in a project. Claude's file tools are blocked there; programs it runs are not."]);
eq("a read the engine does not call `yes` is not one", W.readsCount([p("partly")]).read, 0);

// ── the fold ──
const fc = (fold, fold_now) => chat("x", { ...(fold ? { fold } : {}), ...(fold_now !== undefined ? { fold_now } : {}) });
const evFold = ev({ live: ["s~x"], fold: { "s~x": { state: "folding" } } });
eq("the list's live word is drawn from first, the push's when it has none", [W.foldOf(fc(null, { state: "waiting", why: null }), evFold)?.state, W.foldOf(fc(null, null), evFold), W.foldOf(fc(), evFold)?.state], ["waiting", null, "folding"]);
eq("what runs", ["folding", "waiting", "checking", "failed"].map((s) => W.foldRuns({ state: s, why: null })).concat(W.foldRuns(null)), [true, true, true, false, false]);
eq("a checking one is said so", [W.rowOf(fc(null, { state: "checking", why: null }), ev({ live: ["s~x"] }), null).state, W.ageWords(r("checking"), "1m").text], ["checking", "checking…"]);
const end = (fold, gone = false) => { const e = W.endedLine(fc(fold), gone); return [e.text, e.foldNow, e.resume]; };
eq("an ended chat's line, Fold now and Resume", [end(null), end({ state: "done", commit: "a", reverted: false }), end({ state: "done", reverted: false }), end({ state: "done", commit: "a", reverted: true }), end({ state: "none", reverted: false }), end({ state: "failed", reverted: false }), end({ state: "done", commit: "a", reverted: false }, true)], [
  ["Closed, not folded.", true, true], ["Closed and folded into memory.", false, true], ["Closed. No memory changes.", false, true], ["Closed. Its fold was undone.", true, true],
  ["Closed, not folded.", true, true], ["Closed, not folded.", true, true], ["Closed. Its transcript is gone.", false, false]]);
eq("a failed fold is the engine's words, and no more", [W.failedWords({ state: "failed", why: "it wrote no note in five minutes. Memory is unchanged." }), W.failedWords({ state: "failed", why: "its Claude exited" })], ["The fold did not finish: it wrote no note in five minutes. Memory is unchanged.", "The fold did not finish: its Claude exited."]);
eq("a transcript that is gone, in the engine's words", [W.transcriptGone("this conversation's transcript is no longer on disk"), W.transcriptGone("this conversation has written no transcript yet"), W.transcriptGone(undefined)], [true, false, false]);

// ── its name ──
eq("the engine's own names", ["Chat 3", "Chat three", "Chat 3b", "Release order"].map(W.isDefaultTitle), [true, false, false, false]);
eq("from the first line, whole", W.titleFrom("\n  Which   ships first?\nand why"), "Which ships first?");
eq("cut at a word", W.titleFrom("What order should we ship the Jellyfin role and the config repository in, please?"), "What order should we ship the Jellyfin…");
eq("forty characters is whole", W.titleFrom("x".repeat(40)).length, 40);
const OPENER = "Read /Users/d/.local/share/worktrees/workspaces/01M/chats/c1/fold.md and do what it says. This is fold 01JDWSFOLD0000000000000001.";
eq("a turn the engine sent is a quiet line, whatever its words", [{ user: OPENER, engine: "fold" }, { user: "Anything", engine: "start" }, { user: OPENER }, { user: "Hello" }].map(W.quietLine), ["The chat was asked to fold into memory.", "The chat was started.", null, null]);
eq("the folder-trust question", [W.asksTrust(["Accessing workspace:", "/x/workspaces/01M", "Yes, I trust this folder"]), W.asksTrust(["Edit file", "memory/MEMORY.md"]), W.asksTrust(undefined)], [true, false, false]);
eq("a blank message names nothing", W.titleFrom(" \n "), null);
const t = (user, notice) => ({ user, ...(notice ? { notice } : {}) });
eq("the person's first: not the engine's, not Claude's notice, not a turn of its own", W.firstMessage([t(null), { user: "Read chats/c1/start.md and begin.", engine: "start" }, t("x", { kind: "task" }), { user: OPENER, engine: "fold" }, t("Hello"), t("Later")]), "Hello");
eq("the same words from a person are theirs", W.firstMessage([{ user: OPENER, engine: "fold" }, t(OPENER)]), OPENER);
eq("nothing said yet", W.firstMessage([{ user: "Read chats/c1/start.md and begin.", engine: "start" }]), null);

// ── the desk rows: two kinds, the same acts, and no place's row in either ──
const chatTs = read("../src/chat.ts");
const table = chatTs.slice(chatTs.indexOf("const DESK_ROWS = {"), chatTs.indexOf("} as const;", chatTs.indexOf("const DESK_ROWS = {")));
const kind = (k) => [...(table.match(new RegExp(`${k}: \\{([\\s\\S]*?)\\}`))?.[1] ?? "").matchAll(/(\w+): "(\w+)"/g)].map((m) => [m[1], m[2]]);
eq("both kinds name the same acts", kind("idea").map((x) => x[0]), kind("ws").map((x) => x[0]));
eq("…each on its own rows", [kind("idea").every(([a, row]) => row === `idea_${a}`), kind("ws").every(([a, row]) => row === `workspace_${a}`), kind("ws").length], [true, true, 9]);
const dispatch = read("../../crates/worktrees-cli/src/serve/dispatch.rs");
for (const [, row] of kind("ws")) eq(`${row} is a row, console only, on the chat`, new RegExp(`^\\s+${row}: HostConsole \\[console, workspace_chat\\(conv, id\\)\\]`, "m").test(dispatch), true);
eq("no other idea_* or workspace_* chat row is invoked from chat.ts", (chatTs.replace(table, "").match(/"(idea|workspace)_[a-z_]+"/g) ?? []), []);
eq("a desk chat sends no file and no command", [(chatTs.match(/if \(isDesk\(t\)\) return/g) ?? []).length], [3]);

// ── its rules ──
eq("the intro's limits are the engine's words", W.introWords([], "The rules are set when the chat starts."), "It reads the memory index first, and the workspace has no projects to read. It is told not to write in a project. The rules are set when the chat starts.");
const chg = (kind, change, name = "valleos", path = "/w/valleos") => W.accessWords({ kind, name, path, change });
eq("what changed since it started", ["private", "removed", "unregistered", "unreadable", "readable", "added"].map((c) => chg("project", c)), ["valleos is private now", "valleos was taken out of the workspace", "valleos is not registered any more", "valleos cannot be read any more", "valleos can be read now", "valleos was added"]);
eq("…a document, the rules, and what this build does not know", [...["unlinked", "gone", "moved", "linked"].map((c) => chg("document", c, "Plan")), chg("rules", "other", "", ""), chg("project", "frozen"), chg("project", "added", "")], ["“Plan” is not linked any more", "“Plan” is gone", "“Plan” was moved", "“Plan” was linked", "its other rules changed", "valleos: frozen", "valleos was added"]);
{
  const c = { id: "C", session: "s", title: "t", started: "", last_activity: "", state: "running", note: "n", note_exists: true, fold_now: null };
  const done = { state: "done", commit: "abc", reverted: false };
  eq("a fold that landed and is ending its session is drawn as landed", [W.rowOf({ ...c, fold: { ...done, closing: true } }, null, null), W.rowOf({ ...c, fold: done }, null, null), W.rowOf({ ...c, fold: { ...done, closing: true }, fold_now: { state: "folding", why: null } }, null, null)].map((r) => [r.state, r.live]), [["ended", false], ["running", true], ["folding", true]]);
}

if (failed) { console.log(`\nwschats-check: ${failed} failed`); process.exit(1); }
console.log("wschats-check: all ok");
