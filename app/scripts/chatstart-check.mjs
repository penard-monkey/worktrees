// chatstart-check — a chat opened on a Claude that is still starting says so,
// and reads again until the Claude has written its session file.
//
// The owner, 2026-11-06, with a screenshot: every new session opened in the
// chat view showed "Couldn't read the conversation: the claude in <session>
// has written no session file yet" and a Try again button, and kept it. A
// Claude writes its probe file about a second after its pane opens (2.1.318:
// the pane at 06:36:20, the file at 06:36:21); the chat read once, on open,
// in that gap, and nothing re-reads an idle agent until a turn starts.
//
// This runs the REAL loadTurns, rereadTurns and startingUp from
// app/src/chat.ts on invokes it answers by hand and timers it fires by hand,
// and reads ChatPane.tsx for the branch that shows them. CHATSTART_SRC and
// CHATSTART_PANE point it at other copies (to show it red).

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const here = (p) => path.join(path.dirname(fileURLToPath(import.meta.url)), p);
const SRC = process.env.CHATSTART_SRC ?? here("../src/chat.ts");
const PANE = process.env.CHATSTART_PANE ?? here("../src/ChatPane.tsx");

let failed = 0;
const fail = (m) => { failed++; console.log(`not ok ${m}`); };
const ok = (m) => console.log(`ok     ${m}`);
const check = (cond, m, said = "") => (cond ? ok(m) : fail(said ? `${m}: ${said}` : m));

const src = fs.readFileSync(SRC, "utf8")
  .replace(/^import \{ useSyncExternalStore \} from "react";$/m, "const useSyncExternalStore = (_subscribe, snapshot) => snapshot();")
  .replace(/^import \{ invoke, windowOnHost \} from "\.\/ipc";$/m, "const invoke = (...args) => globalThis.__chatStartInvoke(...args); const windowOnHost = () => true;")
  .replace(/^import \{ convertPhoto, jpegName, photoDecision, photoKind, INPUT_MAX \} from "\.\/chat-image";$/m, "const INPUT_MAX = 40 * 1024 * 1024; const convertPhoto = () => undefined; const jpegName = () => undefined; const photoDecision = () => undefined; const photoKind = () => undefined;")
  .replaceAll('await import("./ipcws")', "({ unsupportedFeature: () => null })");
if (/^import /m.test(src)) {
  console.log("not ok chat.ts gained an import this check does not stub: extend the stubs above");
  process.exit(1);
}

// A mutation can leave an await pending for good, and node then exits 13 with
// nothing printed (AGENTS.md), so a stuck run says so and fails.
const realSetTimeout = globalThis.setTimeout;
realSetTimeout(() => {
  console.log(`not ok chatstart-check stuck, ${failed} failure(s) before it`);
  process.exit(1);
}, 20_000).unref?.();

// Every timer chat.ts sets is held, to be fired by hand.
const held = [];
globalThis.setTimeout = (f, ms) => { held.push({ f, ms }); return held.length; };

const js = (await transformWithEsbuild(src, "chat.ts", { loader: "ts", format: "esm" })).code;
const C = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

/** Every invoke parks here until the test answers it. */
const calls = [];
globalThis.__chatStartInvoke = (cmd, args) => new Promise((resolve, reject) => { calls.push({ cmd, args, resolve, reject }); });
const settle = () => new Promise((resolve) => realSetTimeout(resolve, 5));
const take = (cmd) => {
  const i = calls.findIndex((c) => c.cmd === cmd);
  return i < 0 ? null : calls.splice(i, 1)[0];
};
const view = (t) => C.useChat(t.path);
// A copy from before the fix has none of these: its cases fail, not the run.
const startingUp = C.startingUp ?? (() => false);
const rereadTurns = C.rereadTurns ?? C.loadTurns;
const NO_SESSION = (s) => `the claude in ${s} has written no session file yet`;
const TURNS = (s) => ({ harness: "claude", session: s, more: false, note: "", turns: [] });

/** Answer the pending agent_turns read with `how`, and let it land. */
async function answer(name, how) {
  const c = take("agent_turns");
  if (!c) { fail(`${name}: no agent_turns read was asked`); return false; }
  if (how.err) c.reject(how.err); else c.resolve(how.ok);
  await settle();
  return true;
}
/** Fire the held timers, oldest first, and let what they start run. */
async function fire() {
  const due = held.splice(0);
  for (const h of due) h.f();
  await settle();
  return due.map((h) => h.ms);
}

const R = C.START_REREAD_MS ?? [];
check(Array.isArray(R) && R.length >= 3 && R.every((ms, i) => i === 0 || ms >= R[i - 1]), "START_REREAD_MS is a growing schedule", JSON.stringify(R));
const total = R.reduce((a, b) => a + b, 0);
check(total >= 10_000 && total <= 60_000, "the re-reads cover a slow start and then give up", `${total} ms`);

// ── 1. a starting Claude: re-read on the schedule, say so, then show the turns
{
  const s = "sbx-(main)";
  const t = { path: "/p/one", repo: "/p", slug: "(main)", session: s };
  void C.loadTurns(t);
  await settle();
  await answer("start", { err: NO_SESSION(s) });
  check(startingUp(view(t)), "a read that finds no session file yet says the agent is starting");
  check(held.length === 1 && held[0].ms === R[0], "…and reads again after the first delay", JSON.stringify(held.map((h) => h.ms)));
  let fired = await fire();
  check(calls.some((c) => c.cmd === "agent_turns"), "the delay's end reads again", JSON.stringify(fired));
  await answer("start 2", { err: NO_SESSION(s) });
  check(startingUp(view(t)) && held.length === 1 && held[0].ms === R[1], "a second miss waits the next delay", JSON.stringify(held.map((h) => h.ms)));
  fired = await fire();
  await answer("start 3", { ok: TURNS(s) });
  const v = view(t);
  check(!startingUp(v) && v.turns.error === null && v.turns.data?.session === s, "once the file is written the turns show");
  check(v.startMisses === 0, "…and the misses are forgotten", String(v.startMisses));
  check(held.length === 0, "…and nothing more is scheduled", JSON.stringify(held.map((h) => h.ms)));
}

// ── 2. a Claude that never writes one: the re-reads run out, then the failure
{
  const s = "sbx-two";
  const t = { path: "/p/two", repo: "/p", slug: "two", session: s };
  void C.loadTurns(t);
  await settle();
  const waits = [];
  for (let i = 0; i <= R.length; i++) {
    if (!(await answer(`miss ${i}`, { err: NO_SESSION(s) }))) break;
    waits.push(...(await fire()));
  }
  check(JSON.stringify(waits) === JSON.stringify([...R]), "every delay is waited once, in order", JSON.stringify(waits));
  const v = view(t);
  check(!startingUp(v) && C.noSessionYet(v.turns.error), "after the last delay the failure is shown, not 'starting'");
  check(calls.length === 0 && held.length === 0, "…and nothing more is read", `${calls.length} read(s), ${held.length} timer(s)`);
  // Try again starts the schedule over.
  void rereadTurns(t);
  await settle();
  await answer("try again", { err: NO_SESSION(s) });
  check(startingUp(view(t)) && held.length === 1 && held[0].ms === R[0], "Try again says 'starting' and re-reads from the first delay", JSON.stringify(held.map((h) => h.ms)));
  held.splice(0);
}

// ── 3. any other failure is shown at once and never re-read
{
  const t = { path: "/p/three", repo: "/p", slug: "three", session: "sbx-three" };
  void C.loadTurns(t);
  await settle();
  await answer("other", { err: "no agent is running in this place" });
  const v = view(t);
  check(!startingUp(v) && v.turns.error === "no agent is running in this place", "another failure is not 'starting'");
  check(held.length === 0, "…and is not re-read on the schedule", JSON.stringify(held.map((h) => h.ms)));
}

// ── 4. a re-read that comes due after another read already succeeded does nothing
{
  const s = "sbx-four";
  const t = { path: "/p/four", repo: "/p", slug: "four", session: s };
  void C.loadTurns(t);
  await settle();
  await answer("four miss", { err: NO_SESSION(s) });
  void C.loadTurns(t); // the place's activity moved: a read of its own
  await settle();
  await answer("four ok", { ok: TURNS(s) });
  await fire();
  check(calls.length === 0, "a due re-read after a good read asks nothing", `${calls.length} read(s)`);
}

// ── 5. the pane shows it: 'starting' before the failure, Try again from the top
{
  const pane = fs.readFileSync(PANE, "utf8");
  const starting = pane.search(/\} else if \(startingUp\(v\)\) \{/);
  const failure = pane.search(/\} else if \(v\.turns\.error\) \{/);
  check(starting >= 0 && failure > starting, "ChatPane shows 'starting' ahead of the failure", `starting at ${starting}, failure at ${failure}`);
  const branch = starting >= 0 ? pane.slice(starting, pane.indexOf("} else if", starting + 10)) : "";
  check(/role="status"/.test(branch) && !/role="alert"/.test(branch) && /data-testid="chat-starting"/.test(branch), "…as a status note, not an alert");
  const refresh = /const refresh = \(\) => \{([^}]*)\}/.exec(pane)?.[1] ?? "";
  check(/rereadTurns\(target\)/.test(refresh), "Try again (refresh) re-reads from the first delay", refresh.trim());
}

console.log(failed ? `\n${failed} failure(s)` : "\nall ok");
process.exit(failed ? 1 : 0);
