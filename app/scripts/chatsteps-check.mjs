// chatsteps-check — the chat view draws a turn in the order it happened
// (`turns::Step`, worktrees-core/src/turns.rs): each reply block at its own
// time, the tools after the block they followed, and every question,
// permission and plan a person answered as what was asked and then the
// answer. And the line this window adds for an answer it sent ("You
// answered …") goes once the transcript carries an answer, whoever gave it.
// A tool step's line and failure reach the reply's calls, which fold past a
// few, and the place's root comes off their paths; ChatPane draws them as
// text.
//
// Runs the REAL `itemsOf` and `settleAnswered` from app/src/chat.ts, with
// only its two imports stubbed (React's store hook and the ipc door, neither
// of which these two functions touch). The steps below use the field names
// the Rust tests pin on a real Claude 2.1.316 transcript
// (`claude_steps_carry_what_was_asked_and_answered`).
//
// CHATSTEPS_SRC points the check at another copy of chat.ts, CHATSTEPS_PANE
// at another ChatPane.tsx (to show it red).

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const here = (p) => path.join(path.dirname(fileURLToPath(import.meta.url)), p);
const SRC = process.env.CHATSTEPS_SRC ?? here("../src/chat.ts");

let failed = 0;
const fail = (m) => { failed++; console.log(`FAIL ${m}`); };
const ok = (m) => console.log(`ok   ${m}`);
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);

const src = fs.readFileSync(SRC, "utf8")
  .replace(/^import \{ useSyncExternalStore \} from "react";$/m, "const useSyncExternalStore = (_subscribe, snapshot) => snapshot();")
  .replace(/^import \{ invoke, windowOnHost \} from "\.\/ipc";$/m, "const invoke = (...args) => globalThis.__chatStepsInvoke(...args); const windowOnHost = () => true;");
if (/^import /m.test(src)) fail("chat.ts gained an import this check does not stub — extend the stubs above");
const js = (await transformWithEsbuild(src, "chat.ts", { loader: "ts", format: "esm" })).code;
const C = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

for (const name of ["itemsOf", "settlePending", "settleAnswered", "foldCalls", "callsWords", "callsSay", "underRoot"]) {
  if (typeof C[name] !== "function") fail(`chat.ts exports no ${name}`);
}
if (failed) {
  console.log(`\nchatsteps-check: ${failed} failure(s)`);
  process.exit(1);
}

const T0 = 1_793_470_000;
const question = (at, answered_at, picked) => ({
  kind: "question", at, ...(answered_at ? { answered_at } : {}),
  questions: [{ header: "Fruit", question: "Which fruit?", multi: false, options: [{ label: "Apple" }, { label: "Banana" }] }],
  ...(answered_at ? { answer: { how: "picked", answers: [{ question: "Which fruit?", picked }] } } : {}),
});
const turn = (at, user, steps, extra = {}) => ({ at, user, reply: "", tools: [], truncated: false, steps, ...extra });
const kinds = (items) => items.map((it) => it.k);

// 1. Asked, answered, then the reply — the reply at ITS time, not the prompt's.
{
  const items = C.itemsOf([turn(T0, "Ask me", [question(T0 + 3, T0 + 8, ["Banana"]), { kind: "text", at: T0 + 10, text: "A haiku." }])], [], []);
  const shape = kinds(items).filter((k) => k !== "sep");
  if (!eq(shape, ["user", "asked", "answer", "reply"])) fail(`a question is drawn asked, answered, then the reply — got ${JSON.stringify(shape)}`);
  else ok("a question is drawn as the question, then the answer, then the reply");
  const reply = items.find((it) => it.k === "reply");
  if (reply?.at !== T0 + 10) fail(`the reply after an answer carries its own time (${T0 + 10}), got ${reply?.at}`);
  else ok("the reply after an answer is labelled with its own time");
  const asked = items.find((it) => it.k === "asked");
  if (!asked?.label) fail("the first agent item after the user's message carries the agent's name");
  const after = items.find((it) => it.k === "reply");
  if (!after?.label) fail("a reply after the user's answer carries the agent's name again");
  else ok("the agent's name comes back after the answer");
}

// 2. Not answered yet: the live card shows it, the history does not.
{
  const items = C.itemsOf([turn(T0, "Ask me", [{ kind: "text", at: T0 + 1, text: "Asking." }, question(T0 + 3, null, [])])], [], []);
  if (items.some((it) => it.k === "asked" || it.k === "answer")) fail("an unanswered question is drawn in the history (the live card is where it belongs)");
  else ok("an unanswered question stays out of the history");
}

// 3. Tools join the block they followed; one before any text gets its own row.
{
  const items = C.itemsOf([turn(T0, "Go", [
    { kind: "tool", at: T0 + 1, name: "Write" },
    { kind: "text", at: T0 + 2, text: "Done." },
    { kind: "tool", at: T0 + 3, name: "Bash" },
    { kind: "tool", at: T0 + 4, name: "Read" },
  ])], [], []);
  const replies = items.filter((it) => it.k === "reply").map((r) => [r.text, r.tools]);
  if (!eq(replies, [["", ["Write"]], ["Done.", ["Bash", "Read"]]])) fail(`tool chips follow their block — got ${JSON.stringify(replies)}`);
  else ok("tool chips follow the block they came after");
}

// 4. A host that sends no steps: the turn is drawn as before.
{
  const items = C.itemsOf([{ at: T0, user: "Go", reply: "Done.", tools: ["Bash"], truncated: false }], [], []);
  const r = items.find((it) => it.k === "reply");
  if (!eq(kinds(items).filter((k) => k !== "sep"), ["user", "reply"]) || r.text !== "Done." || !eq(r.tools, ["Bash"])) fail("a turn without steps is not drawn as before");
  else ok("a turn without steps is drawn from reply and tools, as before");
}

// 5. A permission and a plan are asked-and-answered too.
{
  const items = C.itemsOf([turn(T0, "Go", [
    { kind: "permission", at: T0 + 1, answered_at: T0 + 4, tool: "Bash", summary: "touch b", allowed: false, note: "use c" },
    { kind: "plan", at: T0 + 5, answered_at: T0 + 9, plan: "# Plan", approved: true },
  ])], [], []);
  const shape = kinds(items).filter((k) => k !== "sep");
  if (!eq(shape, ["user", "asked", "answer", "asked", "answer"])) fail(`a permission and a plan are drawn asked then answered — got ${JSON.stringify(shape)}`);
  else ok("a permission and a plan are drawn as asked, then answered");
}

// 6. This window's own answer line: kept until the transcript has an answer
//    given after it was sent (less the host's clock skew), one for one.
{
  const sent = (sec) => ({ label: "Banana", at: sec * 1000, confirmed: true });
  const withStep = (answeredAt) => [turn(T0, "Ask", [question(T0 + 1, answeredAt, ["Banana"])])];
  const kept = C.settleAnswered([sent(T0 + 5)], [turn(T0, "Ask", [])]);
  if (kept.length !== 1) fail("a local answer line went before the transcript had any answer");
  else ok("a local answer line stays until the transcript has the answer");
  const gone = C.settleAnswered([sent(T0 + 5)], withStep(T0 + 6));
  if (gone.length !== 0) fail("a local answer line stayed after the transcript carried the answer");
  else ok("a local answer line goes once the transcript carries the answer");
  const old = C.settleAnswered([sent(T0 + 500)], withStep(T0 + 6));
  if (old.length !== 1) fail("an answer the transcript recorded long BEFORE this one settled it");
  else ok("an older answer in the transcript does not settle a newer local one");
  const two = C.settleAnswered([sent(T0 + 5), sent(T0 + 7)], withStep(T0 + 8));
  if (two.length !== 1) fail(`two local answers and one recorded answer leave one line, got ${two.length}`);
  else ok("each recorded answer settles one local line");
}

// 7. A tool step's line and outcome reach the reply's calls; a host that
//    sends neither leaves the bare names, drawn as chips.
{
  const items = C.itemsOf([turn(T0, "Go", [
    { kind: "text", at: T0 + 1, text: "Running it." },
    { kind: "tool", at: T0 + 2, name: "Bash", summary: "make test", failed: true },
    { kind: "tool", at: T0 + 3, name: "mcp__github__get_issue" },
  ])], [], []);
  const r = items.find((it) => it.k === "reply");
  if (!eq(r?.calls, [{ name: "Bash", summary: "make test", failed: true }, { name: "mcp__github__get_issue" }])) fail(`a tool step's summary and failure reach the reply — got ${JSON.stringify(r?.calls)}`);
  else ok("a tool step's summary and failure reach the reply's calls");
  if (!eq(r?.tools, ["Bash", "mcp__github__get_issue"])) fail("the reply's tool names are kept beside its calls");
  if (!C.callsSay(r.calls)) fail("a call with a line is drawn as a row");
  const old = C.itemsOf([{ at: T0, user: "Go", reply: "Done.", tools: ["Bash", "Read"], truncated: false }], [], []).find((it) => it.k === "reply");
  if (!eq(old?.calls, [{ name: "Bash" }, { name: "Read" }]) || C.callsSay(old.calls)) fail("a host without steps gets name chips, as before");
  else ok("a host without steps (or summaries) falls back to the name chips");
}

// 8. The fold: up to CALLS_SHOWN + 1 show whole; past it, the first few and
//    a "+N more" that counts the failures it hides.
{
  const n = (k, failedAt = []) => Array.from({ length: k }, (_, i) => ({ name: "Bash", summary: `c${i}`, ...(failedAt.includes(i) ? { failed: true } : {}) }));
  const four = C.foldCalls(n(C.CALLS_SHOWN + 1), false);
  if (four.folds || four.shown.length !== C.CALLS_SHOWN + 1) fail(`${C.CALLS_SHOWN + 1} calls fold (a fold would hide one row)`);
  else ok(`${C.CALLS_SHOWN + 1} calls show whole: a fold never hides a single row`);
  const seven = C.foldCalls(n(7, [1, 5, 6]), false);
  if (!seven.folds || seven.shown.length !== C.CALLS_SHOWN || seven.hidden !== 7 - C.CALLS_SHOWN || seven.hiddenFailed !== 2) fail(`seven calls fold to ${C.CALLS_SHOWN} and name the two failures hidden — got ${JSON.stringify({ ...seven, shown: seven.shown.length })}`);
  else ok("seven calls fold to the first few, and the fold counts the failures it hides");
  const open = C.foldCalls(n(7), true);
  if (open.shown.length !== 7 || open.hidden !== 0) fail("an opened fold shows every call");
  if (C.callsWords(n(3, [0, 2])) !== "Used Bash. 2 calls failed.") fail(`the group's label counts failures — got ${C.callsWords(n(3, [0, 2]))}`);
  else if (C.callsWords([{ name: "Read" }, { name: "Bash", failed: true }]) !== "Used Read and Bash. One call failed.") fail("one failure is said as one");
  else if (C.callsWords([{ name: "Read" }, { name: "Edit" }]) !== "Used Read and Edit.") fail("no failure says none");
  else ok("the group's label names the tools and how many calls failed");
}

// 9. The place's root comes off a path where a path starts, and nowhere else.
{
  const root = "/Users/u/repo/.worktrees/feat-x";
  const cases = [
    [`${root}/app/src/App.css`, "app/src/App.css"],
    [`nav-age in ${root}/app/src`, "nav-age in app/src"],
    [`sed -n '1,80p' ${root}/src/a.ts`, "sed -n '1,80p' src/a.ts"],
    [`edit ${root}/a.rs, ${root}/b.rs; delete ${root}/c.rs`, "edit a.rs, b.rs; delete c.rs"],
    [`cat '${root}/x' --file=${root}/y`, "cat 'x' --file=y"],
    [`/elsewhere${root}/a.rs`, `/elsewhere${root}/a.rs`],
    [`${root}-next/a.rs`, `${root}-next/a.rs`],
    [`cd ${root} && make`, `cd ${root} && make`],
  ];
  const bad = cases.filter(([a, want]) => C.underRoot(a, root) !== want || C.underRoot(a, root + "/") !== want);
  if (bad.length) fail(`underRoot: ${JSON.stringify(bad.map(([a, want]) => [a, C.underRoot(a, root), want]))}`);
  else ok("the place's root comes off each path that starts with it, and nothing else");
}

// 10. ChatPane draws a summary as TEXT: no markdown, no link, no raw HTML.
{
  const pane = fs.readFileSync(process.env.CHATSTEPS_PANE ?? here("../src/ChatPane.tsx"), "utf8");
  const at = pane.indexOf("function ToolCalls(");
  const body = at < 0 ? "" : pane.slice(at, pane.indexOf("\nfunction ", at + 1));
  if (!body) fail("ChatPane.tsx has no ToolCalls component where this check looks");
  else if (/<Markdown|dangerouslySetInnerHTML|<a[\s>]|href=|onLink|pathLinks/.test(body)) fail("ToolCalls renders a summary through markup or a link — a summary is transcript text");
  else if (!/\{underRoot\(c\.summary, props\.root\)\}/.test(body) || !/title=\{c\.summary\}/.test(body)) fail("ToolCalls draws the line as text with the whole of it in the title");
  else ok("a summary is drawn as text, the whole line in its title");
}

// 11. A message absorbed by Claude is a user step in the running turn. It
// settles one pending copy, and a later normally delivered message settles
// the other. A pending fallback stays where it was sent in time.
{
  const absorbed = "Layer these images [attached: .worktrees-attachments/a.png, .worktrees-attachments/b.png]";
  const normal = "Give me more options.";
  const history = [
    turn(T0, "Start", [{ kind: "text", at: T0 + 1, text: "Working." }, { kind: "user", at: T0 + 10, text: absorbed }, { kind: "text", at: T0 + 20, text: "Layered." }]),
    turn(T0 + 30, normal, [{ kind: "text", at: T0 + 35, text: "Here are options." }]),
  ];
  const pending = [
    { id: 1, text: absorbed, sentAt: (T0 + 9) * 1000, queued: true },
    { id: 2, text: normal, sentAt: (T0 + 29) * 1000, queued: false },
  ];
  if (C.settlePending(pending, history).length !== 0) fail("absorbed and normal messages each settle their pending copy");
  else ok("absorbed and normal messages each settle their pending copy");
  const fallback = [{ id: 3, text: "Unrecorded", sentAt: (T0 + 12) * 1000, queued: true }];
  const items = C.itemsOf(history, [], fallback, false);
  const shape = kinds(items).filter((k) => k !== "sep");
  if (!eq(shape, ["user", "reply", "user", "pending", "reply", "user", "reply"])) fail(`a fallback is not in time order among steps: ${JSON.stringify(shape)}`);
  else ok("an unmatched message stays at its send time among transcript steps");
  if (!items.find((it) => it.k === "pending")?.caughtUp) fail("a fallback still says waiting after a later turn finished");
  else ok("a fallback stops saying waiting after a later turn finished");
}

// 12. The pending clock starts when Send begins, before an agent_send that
// may take over two minutes to confirm. The transcript's user entry may be
// stamped while that request is still in flight.
{
  const oldNow = Date.now;
  let confirm;
  const gate = new Promise((resolve) => { confirm = resolve; });
  globalThis.__chatStepsInvoke = (cmd) => cmd === "agent_send"
    ? gate
    : Promise.resolve({ turns: [], more: false, note: "" });
  const target = { repo: "/r", slug: "s", path: "/r/s", session: "s" };
  Date.now = () => T0 * 1000;
  C.setDraft(target.path, "Give me more options.");
  const sent = C.sendDraft(target, "Claude");
  await Promise.resolve();
  Date.now = () => (T0 + 180) * 1000;
  confirm({ delivered: true, queued: false, harness: "claude", session: "s", note: "Sent" });
  await sent;
  await new Promise((resolve) => setTimeout(resolve, 10));
  const stamped = C.useChat(target.path).pending[0]?.sentAt;
  Date.now = oldNow;
  delete globalThis.__chatStepsInvoke;
  if (stamped !== T0 * 1000) fail(`send time is ${stamped}, after the host confirmation instead of before it`);
  else ok("a slow agent_send cannot make its normal user entry too old to settle");
}

// 13. A send must not let its refresh join an old read and then stop. That
// old read may have captured the transcript before Claude received the line;
// when it finishes, a fresh read must settle the pending copy even if the
// place is now idle and will not be polled again.
{
  const target = { repo: "/r", slug: "race", path: "/r/race", session: "race" };
  const text = "A normal follow-up.";
  let finishOldRead;
  const oldRead = new Promise((resolve) => { finishOldRead = resolve; });
  let reads = 0;
  globalThis.__chatStepsInvoke = (cmd) => {
    if (cmd === "agent_send") return Promise.resolve({ delivered: true, queued: false, harness: "claude", session: "race", note: "Sent" });
    if (cmd === "agent_turns") {
      reads++;
      return reads === 1 ? oldRead : Promise.resolve({ turns: [turn(Math.floor(Date.now() / 1000), text, [])], more: false, note: "" });
    }
    throw new Error(`unexpected ${cmd}`);
  };
  const loading = C.loadTurns(target);
  C.setDraft(target.path, text);
  await C.sendDraft(target, "Claude");
  finishOldRead({ turns: [], more: false, note: "" });
  await loading;
  await new Promise((resolve) => setTimeout(resolve, 10));
  const left = C.useChat(target.path).pending.length;
  delete globalThis.__chatStepsInvoke;
  if (reads !== 2 || left !== 0) fail(`a send joined an old read and left ${left} pending copy (reads: ${reads})`);
  else ok("a send refreshes again after joining a stale in-flight read");
}

console.log(failed ? `\nchatsteps-check: ${failed} failure(s)` : "\nchatsteps-check: all good");
process.exit(failed ? 1 : 0);
