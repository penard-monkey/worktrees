// fullreply-check — a bubble the host shortened can be read whole.
//
// `agent_turns` cuts a prompt and a reply (turns.rs `TEXT_MAX`, and the reply's
// `text` steps share it: later ones are DROPPED), and says so with
// `truncated`. A turn with an `id` can be fetched whole with `agent_turn_text`
// (docs/proposals/ios-mvp.md): the chat offers "Show full reply" (and "Show
// full message" on a cut prompt), the whole turn replaces the short one BY ITS
// ID — so a re-read of `agent_turns` does not snap it back — and the newest
// turn, which may still be running, is read again with each re-read.
//
// Runs the REAL `itemsOf`, `showFull` and `loadTurns` from app/src/chat.ts
// with its imports stubbed (as chatsteps-check does), and reads ChatPane.tsx's
// source for the button's words and test id.
//
// FULLREPLY_SRC points the check at another copy of chat.ts, FULLREPLY_PANE at
// another ChatPane.tsx (to show it red).

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const here = (p) => path.join(path.dirname(fileURLToPath(import.meta.url)), p);
const SRC = process.env.FULLREPLY_SRC ?? here("../src/chat.ts");
const PANE = process.env.FULLREPLY_PANE ?? here("../src/ChatPane.tsx");

let failed = 0;
const fail = (m) => { failed++; console.log(`FAIL ${m}`); };
const ok = (m) => console.log(`ok   ${m}`);
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);

const src = fs.readFileSync(SRC, "utf8")
  .replace(/^import \{ useSyncExternalStore \} from "react";$/m, "const useSyncExternalStore = (_subscribe, snapshot) => snapshot();")
  .replace(/^import \{ invoke, windowOnHost \} from "\.\/ipc";$/m, "const invoke = (...args) => globalThis.__fullInvoke(...args); const windowOnHost = () => true;")
  .replace(/^import \{ convertPhoto, jpegName, photoDecision, photoKind, INPUT_MAX \} from "\.\/chat-image";$/m, "const INPUT_MAX = 40 * 1024 * 1024; const convertPhoto = () => undefined; const jpegName = () => undefined; const photoDecision = () => undefined; const photoKind = () => undefined;");
if (/^import /m.test(src)) fail("chat.ts gained an import this check does not stub — extend the stubs above");
const ipcws = (await transformWithEsbuild(fs.readFileSync(here("../src/ipcws.ts"), "utf8"), "ipcws.ts", { loader: "ts", format: "esm" })).code;
globalThis.__ipcws = await import("data:text/javascript;base64," + Buffer.from(ipcws).toString("base64"));
const js = (await transformWithEsbuild(src.replaceAll('await import("./ipcws")', "globalThis.__ipcws"), "chat.ts", { loader: "ts", format: "esm" })).code;
const C = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));
for (const name of ["itemsOf", "showFull", "loadTurns"]) if (typeof C[name] !== "function") fail(`chat.ts exports no ${name}`);
if (failed) { console.log(`\nfullreply-check: ${failed} failure(s)`); process.exit(1); }

const T0 = 1_793_470_000;
const MAX = C.HOST_TEXT_MAX;
const LAST = "One question before step 2: which branch?";
const cutUser = `${"p".repeat(MAX)}…`;
// What `agent_turns` answers for a long turn, and what `agent_turn_text` does.
const shortTurn = (id, extra = {}) => ({
  id, at: T0, user: "Survey, then ask me.", reply: `${"a".repeat(MAX)}…`, tools: [], truncated: true,
  steps: [{ kind: "text", at: T0 + 1, text: `${"a".repeat(MAX)}…` }], ...extra,
});
const wholeTurn = (id, extra = {}) => ({
  id, at: T0, user: "Survey, then ask me.", reply: "", tools: [], truncated: false,
  steps: [{ kind: "text", at: T0 + 1, text: "a".repeat(MAX + 500) }, { kind: "text", at: T0 + 2, text: LAST }], ...extra,
});
const replies = (items) => items.filter((it) => it.k === "reply");
const text = (items) => replies(items).map((it) => it.text).join("\n");

// 1. A shortened reply offers its turn's id; a turn with no id offers nothing
//    (an older host: today's note, no button).
{
  const items = C.itemsOf([shortTurn("t1")], [], []);
  const r = replies(items)[0];
  if (!r?.truncated || r.turnId !== "t1") fail(`a shortened reply carries its turn's id — ${JSON.stringify(r && { truncated: r.truncated, turnId: r.turnId })}`);
  else ok("a shortened reply carries its turn's id");
  const old = replies(C.itemsOf([shortTurn(undefined)], [], []))[0];
  if (!old?.truncated || "turnId" in old) fail("a turn the host gave no id has no button — the note only");
  else ok("a turn with no id keeps today's note and offers nothing");
  const fine = replies(C.itemsOf([shortTurn("t2", { truncated: false })], [], []))[0];
  if (fine && ("turnId" in fine || fine.truncated)) fail("a turn that was not shortened offers nothing");
  else ok("a turn that was not shortened offers nothing");
}

// 2. A cut prompt offers it on the person's own bubble; an uncut one does not.
{
  const items = C.itemsOf([shortTurn("t1", { user: cutUser })], [], []);
  const u = items.find((it) => it.k === "user");
  if (u?.turnId !== "t1") fail(`a cut prompt carries the turn's id — ${JSON.stringify(u)}`);
  else ok("a cut prompt offers the whole message");
  const plain = C.itemsOf([shortTurn("t1")], [], []).find((it) => it.k === "user");
  if ("turnId" in plain) fail("an uncut prompt offers no button on the person's bubble");
  else ok("an uncut prompt offers none");
}

// 2b. Where the cut is, the note is: a turn `truncated` only for its prompt
//     leaves a whole reply alone; an older host's cut prompt gets the plain
//     note on its own bubble; a cut nothing accounts for stays the reply's.
{
  const whole = { steps: [{ kind: "text", at: T0 + 1, text: "Read it." }], reply: "Read it." };
  const items = C.itemsOf([shortTurn("t1", { user: cutUser, ...whole })], [], []);
  const u = items.find((it) => it.k === "user"), r = replies(items)[0];
  if (u?.turnId !== "t1" || u.cut !== true || r?.truncated || "turnId" in r) fail(`a cut prompt over a whole reply: the button on the prompt only — ${JSON.stringify({ u: u && { cut: u.cut, turnId: u.turnId }, r: r && { t: r.truncated, id: r.turnId } })}`);
  else ok("a turn cut only in its prompt marks the prompt, and leaves its whole reply without a note");
  const both = replies(C.itemsOf([shortTurn("t1", { user: cutUser })], [], [])).at(-1);
  if (both?.turnId !== "t1" || !both.truncated) fail("a turn whose prompt AND reply were cut keeps the button on the reply too");
  else ok("a turn whose prompt and reply were both cut keeps the button on both");
  const old = C.itemsOf([shortTurn(undefined, { user: cutUser, ...whole })], [], []);
  if (old.find((it) => it.k === "user")?.cut !== true || "turnId" in old.find((it) => it.k === "user") || replies(old)[0].truncated) fail("an older host's cut prompt carries the plain note on its own bubble");
  else ok("an older host's cut prompt carries the plain note on its own bubble");
  const plan = C.itemsOf([shortTurn("t1", { reply: "Read it.", steps: whole.steps })], [], []);
  if (replies(plan).at(-1)?.truncated !== true || replies(plan).at(-1).turnId !== "t1") fail("a cut nothing accounts for (a long plan) stays the reply's, with its button");
  else ok("a cut nothing accounts for stays the reply's, with its button");
  const exact = C.itemsOf([shortTurn("t1", { reply: "x", steps: [{ kind: "text", at: T0 + 1, text: "a".repeat(MAX) }] })], [], []);
  if (replies(exact).at(-1)?.turnId !== "t1") fail("a reply that used the whole budget may have lost later blocks: it offers the turn");
  else ok("a reply that used the whole budget (later blocks may be dropped) offers the turn");
  const bare = C.itemsOf([{ id: "t9", at: T0, user: "q", reply: `${"b".repeat(MAX)}…`, tools: [], truncated: true }], [], []);
  if (replies(bare)[0]?.turnId !== "t9") fail("a turn with no steps and a cut reply offers the turn");
  else ok("a turn with no steps and a cut reply offers the turn");
}

// 3. The whole turn replaces the short one BY ID, and stays through a re-read.
{
  const items = C.itemsOf([shortTurn("t1")], [], [], false, { t1: { state: "done", turn: wholeTurn("t1") } });
  const t = text(items);
  if (!t.includes(LAST) || replies(items).some((r) => r.truncated || "turnId" in r)) fail(`the whole turn is drawn, no note, no button — ${JSON.stringify(replies(items).map((r) => r.text.slice(-30)))}`);
  else ok("the whole turn is drawn in place: the dropped last block is there, with no note");
  const again = C.itemsOf([shortTurn("t1"), shortTurn("t2", { at: T0 + 100 })], [], [], false, { t1: { state: "done", turn: wholeTurn("t1") } });
  const first = replies(again)[0], second = replies(again).find((r) => r.turnId === "t2");
  if (!first.text.includes("a".repeat(MAX + 500)) || !second?.truncated || replies(again).filter((r) => r.turnId).length !== 1) fail("another turn is not swapped, and keeps its own button");
  else ok("only the turn asked for is swapped; its neighbour keeps its button");
  const loading = C.itemsOf([shortTurn("t1")], [], [], false, { t1: { state: "loading" } });
  if (replies(loading)[0].turnId !== "t1" || text(loading).includes(LAST)) fail("while loading the short turn stays, with its button (disabled by the pane)");
  else ok("while loading the short turn stays");
  // A whole turn the host cut again at ITS cap says so and offers nothing more.
  const cap = C.itemsOf([shortTurn("t1")], [], [], false, { t1: { state: "done", turn: wholeTurn("t1", { truncated: true }) } });
  if (!replies(cap).at(-1).truncated || "turnId" in replies(cap).at(-1)) fail("a turn cut again at the host's cap keeps the note and no button");
  else ok("a turn cut at the host's own cap keeps the note, with no button");
}

// 4. The store: a click asks once, lands, survives re-reads; a running turn
//    is read again; gone / old / failed read as they should.
{
  const target = { repo: "/r", slug: "full", path: "/r/full", session: "full" };
  const view = () => C.useChat(target.path);
  let asked = [];
  let turnsNow = { turns: [shortTurn("t1")], more: false, note: "" };
  let whole = wholeTurn("t1");
  let textAnswer = () => Promise.resolve({ turn_id: "t1", turn: whole, harness: "claude", session: "full", note: "" });
  globalThis.__fullInvoke = (cmd, args) => {
    asked.push([cmd, args]);
    return cmd === "agent_turns" ? Promise.resolve(turnsNow) : textAnswer(args.turn_id);
  };
  await C.loadTurns(target);
  await C.showFull(target, "t1");
  if (!eq(asked.filter(([c]) => c === "agent_turn_text"), [["agent_turn_text", { repo: "/r", slug: "full", turn_id: "t1" }]])) fail(`the click asks for that turn — ${JSON.stringify(asked)}`);
  else if (view().full.t1?.state !== "done") fail(`the answer lands — ${JSON.stringify(view().full)}`);
  else ok("a click asks the host for that turn, and it lands");
  const shown = () => C.itemsOf(view().turns.data.turns, [], [], false, view().full);
  asked = [];
  await C.loadTurns(target);
  if (!text(shown()).includes(LAST)) fail("a re-read of the short turns snapped the whole one back");
  else ok("a re-read of agent_turns keeps it expanded");
  const rereads = asked.filter(([c]) => c === "agent_turn_text").length;
  if (rereads !== 1) fail(`the newest turn, expanded, is read again with the poll — asked ${rereads}`);
  else ok("the newest turn, once expanded, is read again with each poll (it may still be running)");
  // It grew: the next poll brings the new words in.
  whole = wholeTurn("t1", { steps: [...wholeTurn("t1").steps, { kind: "text", at: T0 + 9, text: "A block written after it was expanded." }] });
  await C.loadTurns(target);
  if (!text(shown()).includes("written after it was expanded")) fail("a running turn froze at the text it had when expanded");
  else ok("a running turn is not frozen at the text it had when it was expanded");
  // An older turn (no longer the newest) is complete and is not asked for again.
  turnsNow = { turns: [shortTurn("t1"), shortTurn("t3", { at: T0 + 50 })], more: false, note: "" };
  asked = [];
  await C.loadTurns(target);
  if (asked.some(([c]) => c === "agent_turn_text")) fail("a finished turn is read again with every poll");
  else ok("a turn that is no longer the newest is not read again");
  // Gone after being shown: what is on screen stays.
  turnsNow = { turns: [shortTurn("t1")], more: false, note: "" };
  textAnswer = () => Promise.reject("gone");
  await C.loadTurns(target);
  if (view().full.t1?.state !== "done") fail("a turn that went `gone` after being shown lost its words");
  else ok("a turn that goes `gone` after it was shown keeps its words");

  // States for turns not read yet.
  const outcome = async (id, error, state) => {
    textAnswer = () => Promise.reject(error);
    await C.showFull(target, id);
    const got = view().full[id];
    if ((got?.state ?? "off") !== state) fail(`${JSON.stringify(error)} reads as ${state} — got ${JSON.stringify(got)}`);
    else ok(`${JSON.stringify(error)} reads as ${state}`);
    return got;
  };
  await outcome("g", "gone", "gone");
  const failedRead = await outcome("f", "the claude here has a session file worktrees can't read", "failed");
  if (!/session file/.test(failedRead.words)) fail("a failure says what the host said");
  asked = [];
  textAnswer = () => Promise.resolve({ turn_id: "f", turn: wholeTurn("f"), harness: "claude", session: "full", note: "" });
  await C.showFull(target, "f");
  if (view().full.f?.state !== "done" || asked.length !== 1) fail("a failed read is asked again on the next click");
  else ok("a failed read is asked again on the next click");
  asked = [];
  await C.showFull(target, "g");
  await C.showFull(target, "f");
  if (asked.length) fail("`gone` and a read turn are not asked for again");
  else ok("`gone` and a read turn are not asked for again");
  await outcome("o", "unknown command: agent_turn_text", "off");
  if (!/This host doesn't support/.test(view().fullOff ?? "")) fail(`an older host is named — ${view().fullOff}`);
  else ok("an older host is named, for the whole view (no bubble offers the button)");
  delete globalThis.__fullInvoke;
}

// 5. The pane: the words, and the one button.
{
  const pane = fs.readFileSync(PANE, "utf8");
  const need = [
    ["Show full ${what}", "the button says Show full reply / message"],
    ["data-testid=\"chat-show-full\"", "the button has its test id"],
    ["showFull(target, id)", "the button calls showFull"],
    ["Shortened by the host: the full ${what} is in the terminal.", "an older host keeps today's note, in its words"],
    ["has left what the host reads", "gone says the text is in the terminal"],
    ["v.full)", "the pane draws from the whole turns"],
  ];
  for (const [needle, why] of need) {
    if (!pane.includes(needle)) fail(`ChatPane.tsx: ${why} (missing ${needle})`);
    else ok(`ChatPane.tsx: ${why}`);
  }
}

console.log(failed ? `\nfullreply-check: ${failed} failure(s)` : "\nfullreply-check: all good");
process.exit(failed ? 1 : 0);
