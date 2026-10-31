// chatsteps-check — the chat view draws a turn in the order it happened
// (`turns::Step`, worktrees-core/src/turns.rs): each reply block at its own
// time, the tools after the block they followed, and every question,
// permission and plan a person answered as what was asked and then the
// answer. And the line this window adds for an answer it sent ("You
// answered …") goes once the transcript carries an answer, whoever gave it.
//
// Runs the REAL `itemsOf` and `settleAnswered` from app/src/chat.ts, with
// only its two imports stubbed (React's store hook and the ipc door, neither
// of which these two functions touch). The steps below use the field names
// the Rust tests pin on a real Claude 2.1.316 transcript
// (`claude_steps_carry_what_was_asked_and_answered`).
//
// CHATSTEPS_SRC points the check at another copy of chat.ts (to show it red).

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
  .replace(/^import \{ useSyncExternalStore \} from "react";$/m, "const useSyncExternalStore = () => undefined;")
  .replace(/^import \{ invoke, windowOnHost \} from "\.\/ipc";$/m, "const invoke = async () => { throw new Error(\"no ipc here\"); }; const windowOnHost = () => true;");
if (/^import /m.test(src)) fail("chat.ts gained an import this check does not stub — extend the stubs above");
const js = (await transformWithEsbuild(src, "chat.ts", { loader: "ts", format: "esm" })).code;
const C = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

for (const name of ["itemsOf", "settleAnswered"]) {
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

console.log(failed ? `\nchatsteps-check: ${failed} failure(s)` : "\nchatsteps-check: all good");
process.exit(failed ? 1 : 0);
