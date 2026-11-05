// chatsend-check — a message this window sent shows once: never as a pending
// bubble beside its own echo, whatever order the read and agent_send's answer
// land in, and whatever the transcript did to its text.
//
// Three things held a sent line on screen twice, each measured against David's
// audit log and transcripts on 2026-11-04 (hashes and lengths only):
// - Claude trims what it records. 14 of 50 sends ended in one space, and each
//   one's exact match against the transcript failed, on every read, for good.
// - The host clips a long message (`turns::finish`, TEXT_MAX and an ellipsis),
//   with the same result.
// - agent_send answered 533–922 ms after it started, and Claude stamps the
//   transcript inside that. A read landing in the gap brought the echo in
//   before the Pending existed, and only the next read settled it.
//
// The mock answers instantly and cannot express the order, so this runs the
// REAL sendDraft, loadTurns, settlePending and itemsOf from app/src/chat.ts on
// promises it resolves by hand and a clock it sets by hand. CHATSEND_SRC and
// CHATSEND_PANE point it at other copies (to show it red).

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const here = (p) => path.join(path.dirname(fileURLToPath(import.meta.url)), p);
const SRC = process.env.CHATSEND_SRC ?? here("../src/chat.ts");
const PANE = process.env.CHATSEND_PANE ?? here("../src/ChatPane.tsx");
const TURNS_RS = here("../../crates/worktrees-core/src/turns.rs");

let failed = 0;
const fail = (m) => { failed++; console.log(`not ok ${m}`); };
const ok = (m) => console.log(`ok     ${m}`);
const check = (cond, m, said = "") => (cond ? ok(m) : fail(said ? `${m}: ${said}` : m));

const src = fs.readFileSync(SRC, "utf8")
  .replace(/^import \{ useSyncExternalStore \} from "react";$/m, "const useSyncExternalStore = (_subscribe, snapshot) => snapshot();")
  .replace(/^import \{ invoke, windowOnHost \} from "\.\/ipc";$/m, "const invoke = (...args) => globalThis.__chatSendInvoke(...args); const windowOnHost = () => true;")
  .replace(/^import \{ convertPhoto, jpegName, photoDecision, photoKind, INPUT_MAX \} from "\.\/chat-image";$/m, "const INPUT_MAX = 40 * 1024 * 1024; const convertPhoto = () => undefined; const jpegName = () => undefined; const photoDecision = () => undefined; const photoKind = () => undefined;")
  // A `data:` module cannot resolve a relative import, and this one only names
  // an old host's refusal, which no case here is.
  .replaceAll('await import("./ipcws")', "({ unsupportedFeature: () => null })");
if (/^import /m.test(src)) {
  console.log("not ok chat.ts gained an import this check does not stub: extend the stubs above");
  process.exit(1);
}
const js = (await transformWithEsbuild(src, "chat.ts", { loader: "ts", format: "esm" })).code;
const C = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

// ── the harness ─────────────────────────────────────────────────────────────

// A mutation can leave an await pending for good, and node then exits 13 with
// nothing printed (AGENTS.md), so a stuck run says so and fails.
setTimeout(() => {
  console.log(`not ok chatsend-check stuck, ${failed} failure(s) before it`);
  process.exit(1);
}, 20_000);

// The own-send window is a real 20 s timer: this run holds such timers to fire
// by hand. `settle` and the rest pass through.
const realSetTimeout = globalThis.setTimeout;
const held = [];
globalThis.setTimeout = (f, ms, ...rest) => {
  if (ms >= 1000) { held.push({ f, ms }); return 0; }
  return realSetTimeout(f, ms, ...rest);
};

const T0 = 1_793_470_000;
const realNow = Date.now;
let nowSec = T0;
Date.now = () => nowSec * 1000;

/** Every invoke parks here until the test answers it, in the test's order. */
const calls = [];
globalThis.__chatSendInvoke = (cmd, args) => new Promise((resolve, reject) => { calls.push({ cmd, args, resolve, reject }); });
/** Let promise chains run, on a real timer the stubbed clock does not touch. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 5));
/** The oldest unanswered call of `cmd`: a missing one is a failure of ITS test,
 *  and answers nothing, so the rest of the run still reports. */
const take = (cmd) => {
  const i = calls.findIndex((c) => c.cmd === cmd);
  return i < 0 ? null : calls.splice(i, 1)[0];
};
async function answer(cmd, value) {
  const c = take(cmd);
  if (!c) { fail(`expected a ${cmd} call in flight`); return; }
  c.resolve(value);
  await settle();
}
const SENT = { delivered: true, queued: false, harness: "claude", session: "s", note: "Sent" };
const turnsOf = (...turns) => ({ turns, more: false, note: "" });
const turn = (at, user, steps = []) => ({ at, user, reply: "", tools: [], truncated: false, steps });

let places = 0;
const place = () => {
  const slug = `p${++places}`;
  return { repo: "/r", slug, path: `/r/${slug}`, session: "s" };
};
/** Answer every read a test left parked, so the next test starts clean. */
async function drain(cmdValue = turnsOf()) {
  for (let i = 0; i < 8 && calls.length; i++) {
    for (const c of calls.splice(0)) c.resolve(c.cmd === "agent_send" ? SENT : cmdValue);
    await settle();
  }
}
/** What the chat draws, in order, for the rows this check is about. */
const rows = (t) => {
  const v = C.useChat(t.path);
  return C.itemsOf(v.turns.data?.turns ?? [], v.answered, v.pending, false)
    .filter((it) => it.k === "user" || it.k === "pending")
    .map((it) => (it.k === "user" ? `user:${it.text}` : `pending:${it.p.text}`));
};
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
const shows = (t, want, m) => {
  const got = rows(t);
  check(same(got, want), m, `drew ${JSON.stringify(got)}, wanted ${JSON.stringify(want)}`);
};
/** Start sending `text`: returns with agent_send parked, and `done` wrapped,
 *  because an async function that returned it bare would wait for it. */
async function startSend(t, text) {
  C.setDraft(t.path, text);
  const done = C.sendDraft(t, "Claude");
  await settle();
  return { done };
}
/** One whole send that the transcript has echoed, ending idle. */
async function sendAndEcho(t, text, at, ...before) {
  const { done } = await startSend(t, text);
  await answer("agent_send", SENT);
  await done;
  await answer("agent_turns", turnsOf(...before, turn(at, text.trim())));
  await drain(turnsOf(...before, turn(at, text.trim())));
}

// ── 1. the order ────────────────────────────────────────────────────────────

// The echo read lands while agent_send is in flight. The Pending must never
// appear, including in the gap before the follow-up read answers.
{
  const t = place();
  const text = "make it a PR";
  const { done } = await startSend(t, text);
  void C.loadTurns(t);
  await settle();
  await answer("agent_turns", turnsOf(turn(T0, text)));
  await answer("agent_send", SENT);
  await done;
  shows(t, [`user:${text}`], "echo read lands BEFORE agent_send returns: no pending beside it, before the next read");
  await drain(turnsOf(turn(T0, text)));
  shows(t, [`user:${text}`], "… and still one after the follow-up read");
}

// The usual order: agent_send answers, then the read brings the echo.
{
  const t = place();
  const text = "make it a PR";
  const { done } = await startSend(t, text);
  await answer("agent_send", SENT);
  await done;
  shows(t, [`pending:${text}`], "echo read lands AFTER agent_send returns: pending until it does");
  await answer("agent_turns", turnsOf(turn(T0, text)));
  shows(t, [`user:${text}`], "… and the echo replaces it");
  await drain(turnsOf(turn(T0, text)));
}

// A read from before the echo answers during the send: the Pending is added,
// and the follow-up read settles it. Holding no echo must change nothing.
{
  const t = place();
  const text = "hold on";
  const { done } = await startSend(t, text);
  void C.loadTurns(t);
  await settle();
  await answer("agent_turns", turnsOf());
  await answer("agent_send", SENT);
  await done;
  shows(t, [`pending:${text}`], "a read without the echo during the send leaves the pending up");
  await answer("agent_turns", turnsOf(turn(T0, text)));
  shows(t, [`user:${text}`], "… until a read has it");
  await drain(turnsOf(turn(T0, text)));
}

// A read in flight when agent_send answers, joined by the follow-up (#596).
// It answers without the echo, so one fresh read must follow.
{
  const t = place();
  const text = "joined";
  void C.loadTurns(t);
  await settle();
  const { done } = await startSend(t, text);
  await answer("agent_send", SENT);
  await done;
  await answer("agent_turns", turnsOf());
  shows(t, [`pending:${text}`], "a joined read from before the echo leaves the pending up");
  await answer("agent_turns", turnsOf(turn(T0, text)));
  shows(t, [`user:${text}`], "… and the fresh read after it settles it");
  await drain(turnsOf(turn(T0, text)));
}

// ── 2. the same text twice ──────────────────────────────────────────────────

// The first copy's echo is already on screen when the second is sent. It is
// inside SKEW_S, so only knowing it was already there keeps it from settling
// the second copy, in either order.
{
  const t = place();
  const text = "yes";
  await sendAndEcho(t, text, T0);
  nowSec = T0 + 30;
  const first = turn(T0, text);
  const { done } = await startSend(t, text);
  void C.loadTurns(t);
  await settle();
  await answer("agent_turns", turnsOf(first));
  await answer("agent_send", SENT);
  await done;
  shows(t, [`user:${text}`, `pending:${text}`], "same text again, a read holding only the FIRST echo during the send: the second still shows");
  await answer("agent_turns", turnsOf(first));
  shows(t, [`user:${text}`, `pending:${text}`], "… and a read after it holding only the first still leaves it");
  await answer("agent_turns", turnsOf(first, turn(T0 + 30, text)));
  shows(t, [`user:${text}`, `user:${text}`], "… and its own echo settles it: two, no pending");
  await drain(turnsOf(first, turn(T0 + 30, text)));
  nowSec = T0;
}

// The second copy's own echo lands during its send.
{
  const t = place();
  const text = "again";
  await sendAndEcho(t, text, T0);
  nowSec = T0 + 30;
  const first = turn(T0, text);
  const { done } = await startSend(t, text);
  void C.loadTurns(t);
  await settle();
  await answer("agent_turns", turnsOf(first, turn(T0 + 30, text)));
  await answer("agent_send", SENT);
  await done;
  shows(t, [`user:${text}`, `user:${text}`], "same text again, its own echo read during the send: two, no pending");
  await drain(turnsOf(first, turn(T0 + 30, text)));
  nowSec = T0;
}

// #596's rule, sent while neither echo has been read: one record settles one
// copy, whichever read brings it.
{
  const t = place();
  const text = "twice";
  const one = await startSend(t, text);
  await answer("agent_send", SENT);
  await one.done;
  await answer("agent_turns", turnsOf());
  await answer("agent_turns", turnsOf());
  nowSec = T0 + 5;
  const two = await startSend(t, text);
  await answer("agent_send", SENT);
  await two.done;
  shows(t, [`pending:${text}`, `pending:${text}`], "two identical sends with no echo yet: two pending");
  await answer("agent_turns", turnsOf(turn(T0, text)));
  check(rows(t).filter((r) => r === `pending:${text}`).length === 1, "… one record settles one copy", JSON.stringify(rows(t)));
  check(rows(t).filter((r) => r === `user:${text}`).length === 1, "… beside its one echo", JSON.stringify(rows(t)));
  await drain(turnsOf(turn(T0, text)));
  void C.loadTurns(t);
  await settle();
  await answer("agent_turns", turnsOf(turn(T0, text), turn(T0 + 5, text)));
  shows(t, [`user:${text}`, `user:${text}`], "… and the second record settles the other");
  await drain(turnsOf(turn(T0, text), turn(T0 + 5, text)));
  nowSec = T0;
}

// A message held that is much older (past SKEW_S) was never a candidate, and
// remembering it must not change that.
{
  const old = turn(T0 - 3600, "yes");
  const p = { id: 1, text: "yes", sentAt: T0 * 1000, queued: false };
  check(C.settlePending([p], [old]).length === 1, "an old copy past the skew does not settle a new send");
  check(C.settlePending([p], [old, turn(T0, "yes")]).length === 0, "its own echo beside that old copy does");
}

// ── 3. what the transcript does to the text ─────────────────────────────────

// The message in the screenshot, for real: "make it a PR " went out, Claude
// recorded "make it a PR". Before the fix, every read kept this pending.
{
  const t = place();
  const { done } = await startSend(t, "make it a PR ");
  await answer("agent_send", SENT);
  await done;
  await answer("agent_turns", turnsOf(turn(T0, "make it a PR")));
  shows(t, ["user:make it a PR"], "a draft ending in a space settles against what Claude recorded");
  await drain(turnsOf(turn(T0, "make it a PR")));
}
{
  const t = place();
  const { done } = await startSend(t, "make it a PR ");
  void C.loadTurns(t);
  await settle();
  await answer("agent_turns", turnsOf(turn(T0, "make it a PR")));
  await answer("agent_send", SENT);
  await done;
  shows(t, ["user:make it a PR"], "… and with its echo read during the send");
  await drain(turnsOf(turn(T0, "make it a PR")));
}

// Sent while Claude works: the record is a step inside the running turn.
{
  const t = place();
  const { done } = await startSend(t, "and the tests ");
  await answer("agent_send", { ...SENT, queued: true });
  await done;
  check(C.useChat(t.path).pending[0]?.queued === true, "a queued send is marked queued");
  await answer("agent_turns", turnsOf(turn(T0 - 5, "start", [{ kind: "user", at: T0, text: "and the tests" }])));
  check(C.useChat(t.path).pending.length === 0, "a queued message settles against its mid-turn step, trailing space and all");
  await drain(turnsOf(turn(T0 - 5, "start", [{ kind: "user", at: T0, text: "and the tests" }])));
}

// `turns::finish` cuts a user message at TEXT_MAX characters with an ellipsis,
// and `clip_steps` a mid-turn one the same way.
{
  const product = /pub const TEXT_MAX: usize = (\d+);/.exec(fs.readFileSync(TURNS_RS, "utf8"));
  check(!!product, "turns.rs still names TEXT_MAX");
  const MAX = Number(product?.[1]);
  check(C.HOST_TEXT_MAX === MAX, "HOST_TEXT_MAX mirrors turns.rs", `${C.HOST_TEXT_MAX} against ${MAX}`);
  const long = "x".repeat(MAX + 100);
  const clipped = `${"x".repeat(MAX)}…`;
  const p = { id: 1, text: long, sentAt: T0 * 1000, queued: false };
  check(C.settlePending([p], [turn(T0, clipped)]).length === 0, "a message over the host's clip settles against its clipped record");
  check(C.settlePending([p], [turn(T0 - 5, "start", [{ kind: "user", at: T0, text: clipped }])]).length === 0, "… as a mid-turn step too");
  check(C.settlePending([p], [turn(T0, `${"y".repeat(MAX)}…`)]).length === 1, "another message clipped at the same length does not");
  // The host counts `chars()`: code points, where JS's length counts UTF-16.
  const astral = "😀".repeat(MAX + 10);
  check(C.settlePending([{ ...p, text: astral }], [turn(T0, `${"😀".repeat(MAX)}…`)]).length === 0, "the clip is counted in code points, as the host counts");
  // A person's own ellipsis is not the host's cut. A read holding it must not
  // settle a later line that merely starts the same way.
  const kept = (sent, record, m) => check(C.settlePending([{ id: 3, text: sent, sentAt: T0 * 1000, queued: false }], [turn(T0, record)]).length === 1, m);
  kept("ok, go ahead", "ok…", "a typed “ok…” does not settle “ok, go ahead”");
  kept("wait for it", "wait…", "a typed “wait…” does not settle “wait for it”");
  kept(long, `${"x".repeat(MAX - 1)}…`, "a record one short of the cut is not a clip");
  kept(long, `${"x".repeat(MAX + 1)}…`, "nor one past it");
  const short = { id: 2, text: "wait", sentAt: T0 * 1000, queued: false };
  check(C.settlePending([short], [turn(T0, "wait…")]).length === 1, "a sent line is no copy of a longer one that ends in an ellipsis");
  check(C.settlePending([{ ...short, text: "wait for it" }], [turn(T0, "wait")]).length === 1, "nor is a prefix without the ellipsis");
  check(C.settlePending([short], [turn(T0, "…")]).length === 1, "nor a record that is only an ellipsis");
}

// An attachment line, as the host and Claude both keep it.
{
  const line = "look at this [attached: .worktrees-attachments/20261104-0123456789abcdef.png]";
  const p = { id: 1, text: line, sentAt: T0 * 1000, queued: false };
  check(C.settlePending([p], [turn(T0, line)]).length === 0, "an attachment line settles against itself");
}

// ── 4. a send that did not land ─────────────────────────────────────────────

{
  const t = place();
  const { done } = await startSend(t, "not confirmed");
  await answer("agent_send", { ...SENT, delivered: false });
  await done;
  shows(t, [], "an unconfirmed send adds no pending");
  check(C.useChat(t.path).draft === "not confirmed", "… and keeps the draft");
  await drain();
}

// ── 5. the pane-draft notice ────────────────────────────────────────────────

// serve samples the composer every 15 s, so a sample can land in the ~600 ms
// between the paste and Enter (serve.log: `drafts: 1` at 22:30:03, `drafts: 0`
// at 22:30:18). For those 15 s the notice said a message Claude had already
// taken was unsent. A send that did NOT land has to leave the notice telling
// the truth: there the text may really be in the composer.
// ── a message of several lines ──────────────────────────────────────────────
{
  const lines = { lineBreaks: true, maxBytes: 16 * 1024 };
  const t = place();
  C.setDraft(t.path, "one\ntwo\n\nfour  \n");
  const done = C.sendDraft(t, "Claude", lines);
  await settle();
  const sending = calls.find((c) => c.cmd === "agent_send");
  check(sending?.args.text === "one\ntwo\n\nfour  \n", "the draft goes as typed: the host normalises and trims nothing the person sees", JSON.stringify(sending?.args.text));
  const own = C.draftIsOwnSend, v = () => C.useChat(t.path);
  check(own("one two", v()), "a sample cut at the first blank row is its own send");
  check(own("four", v()), "so is one showing only the end of a tall draft");
  check(own("[Pasted text #12 +3 lines]", v()), "so is the placeholder Claude collapsed it to");
  check(own("[Pasted text #12]", v()), "… with no line count too");
  check(!own("one two TYPED", v()), "words of someone's still count as a draft");
  check(!own("[Pasted text #12] hello", v()), "… beside a placeholder too");
  await answer("agent_send", SENT);
  await done;
  check(C.useChat(t.path).pending.length === 1, "it is pending until the transcript has it");
  // Claude trims the ends and keeps the lines (measured 2.1.318); the host has
  // unwrapped a collapsed paste by the time it answers.
  const at = T0 + 1;
  const after = C.settlePending(C.useChat(t.path).pending, [turn(at, "one\ntwo\n\nfour")]);
  check(after.length === 0, "its record settles it", JSON.stringify(after));
  const crlf = C.settlePending([{ id: 1, text: "a\r\nb\rc", sentAt: T0 * 1000 }], [turn(at, "a\nb\nc")]);
  check(crlf.length === 0, "CRLF and CR settle against the LF the host typed");
  const other = C.settlePending(C.useChat(t.path).pending, [turn(at, "one two four")]);
  check(other.length === 1, "the same words with the lines flattened are not its record");
  const long = "line\n".repeat(1500);
  const cut = C.settlePending([{ id: 2, text: long, sentAt: T0 * 1000 }], [turn(at, `${[...long.trim()].slice(0, C.HOST_TEXT_MAX).join("")}…`)]);
  check(cut.length === 0, "a long one settles against the host's cut");
  await drain();

  // A send that stopped: said in the host's words, the whole draft kept.
  const u = place();
  C.setDraft(u.path, "one\ntwo");
  const stopped = C.sendDraft(u, "Claude", lines);
  await settle();
  const note = "Not sent: a prompt opened part way, after about 512 of 2048 bytes.";
  await answer("agent_send", { delivered: false, queued: false, harness: "claude", session: "s", note, stopped: true });
  await stopped;
  const w = C.useChat(u.path);
  check(w.sendWords?.line === note && w.sendWords?.terminal === true, "a stopped send says what the host said and offers the terminal", JSON.stringify(w.sendWords));
  check(w.draft === "one\ntwo" && w.pending.length === 0 && w.sentLine === null, "the whole draft stays, nothing claims to be on its way");
  await drain();
  // An older host's refusal is a way out, not a dead end.
  const old = C.sendWords("text contains a control character (newline, tab, escape…); only ONE line is typed", "Claude");
  check(old.join === true && /one line/.test(old.line), "an older host's refusal offers to join", JSON.stringify(old));
}
if (typeof C.draftIsOwnSend !== "function") {
  fail("chat.ts exports no draftIsOwnSend");
} else {
  const own = C.draftIsOwnSend;
  const t = place();
  const text = "make it a PR ";
  held.length = 0;
  const { done } = await startSend(t, text);
  const during = C.useChat(t.path);
  check(own("make it a PR", during), "a sample of the line being sent is no draft");
  check(!own("something else", during), "… other text in the composer still is");
  check(held.length === 0, "… and its window does not start until the send lands");
  await answer("agent_send", SENT);
  await done;
  const after = C.useChat(t.path);
  check(own("make it a PR", after), "the same sample after the send landed is none either");
  // The push is change-gated (host.rs), so if the person really typed these
  // words, nothing would redraw a hidden notice. The window ends in the STORE,
  // which is what redraws the composer.
  check(held.length === 1 && held[0].ms > 15_000, "… and one timer outlasts the 15 s sample", JSON.stringify(held.map((h) => h.ms)));
  held.splice(0).forEach((h) => h.f());
  const expired = C.useChat(t.path);
  check(expired !== after, "the window's end moves the store, so the composer redraws");
  check(!own("make it a PR", expired), "… and the same words after it are someone's draft");
  await drain();

  // A later send of the same words gets its whole window: the first's timer
  // belongs to the first.
  {
    const r = place();
    const one = await startSend(r, "same");
    await answer("agent_send", SENT);
    await one.done;
    const two = await startSend(r, "same");
    await answer("agent_send", SENT);
    await two.done;
    const timers = held.splice(0);
    check(timers.length === 2, "two landed sends, two timers", String(timers.length));
    timers[0]?.f();
    check(own("same", C.useChat(r.path)), "the first send's timer does not end the second's window");
    timers[1]?.f();
    check(!own("same", C.useChat(r.path)), "… its own does");
    await drain();
  }

  // A window still open when a later send begins vouches only for the later
  // line, from its first step: an upload can take a while, and may be refused.
  {
    const r = place();
    const one = await startSend(r, "stale");
    await answer("agent_send", SENT);
    await one.done;
    await drain();
    held.length = 0;
    C.setDraft(r.path, "stale");
    C.addAttachments(r.path, [new Blob(["hello"], { type: "text/plain" })]);
    await settle();
    const sending = C.sendDraft(r, "Claude");
    await settle();
    check(C.useChat(r.path).sending === true, "a send with a file is uploading");
    check(!own("stale", C.useChat(r.path)), "an earlier line is not vouched for while a later send uploads");
    take("agent_attach")?.reject("attach-unsafe");
    await sending;
    check(!own("stale", C.useChat(r.path)), "… nor after that upload is refused");
    await drain();
    held.length = 0;
  }

  for (const [name, reply] of [["unconfirmed", { ...SENT, delivered: false }], ["refused", null]]) {
    const r = place();
    const { done: sending } = await startSend(r, "stuck");
    if (reply) await answer("agent_send", reply);
    else { take("agent_send")?.reject("the agent's prompt has unsent text in it"); await settle(); }
    await sending;
    check(!own("stuck", C.useChat(r.path)), `a send ${name}: the notice stays, the text may really be there`);
    await drain();
  }

  // The sample's own shape (`agent::draft_from_screen`): wrapped rows joined by
  // a space, runs of space collapsed, cut at 160 characters with an ellipsis.
  const w = place();
  const wrapped = `${"word ".repeat(60)}end`;
  const { done: sendingW } = await startSend(w, wrapped);
  const v = C.useChat(w.path);
  const shown = `${wrapped.split(/\s+/).join(" ").slice(0, 160)}…`;
  check(own(shown, v), "a long line, as the sample cuts it, is still this send");
  check(own("word word wo rd word", { ...v, sentLine: { text: "word word word word" } }), "a row break inside a word does not hide it");
  await answer("agent_send", SENT);
  await sendingW;
  await drain();

  check(!own("anything", { sentLine: null }), "nothing sent, nothing to discount");
}

const pane = fs.readFileSync(PANE, "utf8");
check(/props\.paneDraft && !props\.waiting && !draftIsOwnSend\(props\.paneDraft\.text, v\)/.test(pane),
  "the composer's notice goes through draftIsOwnSend");

Date.now = realNow;
globalThis.setTimeout = realSetTimeout;
delete globalThis.__chatSendInvoke;
if (failed) {
  console.log(`\nchatsend-check: ${failed} failure(s)`);
  process.exit(1);
}
console.log("\nchatsend-check: all passed");
process.exit(0);
