// chat-announce-check — what the chat tells a screen reader (VoiceOver), and
// when. The iOS app announces an agent's new replies politely ("1 new
// message", worktrees-ios `src/screens/ViewSession.tsx`); the desktop chat
// does the same, and says when the agent starts waiting on a prompt.
//
// Announced: a new agent reply, and a prompt card appearing (readable or not).
// Never: the first read of a place's conversation, nor the one after it is
// shown again (a place switch, the window coming back); the person's own
// sends and answers, nor the transcript's echo of them; tool calls; a re-read
// that changes nothing (a pending message settling, the same transcript, the
// same prompt). Several within ANNOUNCE_MS are said once, and only while the
// chat is shown and the window is not hidden.
//
// Runs the REAL `hear`, `gather`, `announcement`, `promptKeyOf` and `itemsOf`
// from app/src/chat.ts, with only its two imports stubbed (React's store hook
// and the ipc door, which none of them touch), then reads ChatPane.tsx for the
// one live region and its wiring.
//
// CHATANNOUNCE_SRC / CHATANNOUNCE_PANE point the check at other copies of
// chat.ts / ChatPane.tsx (to show it red).

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const here = (p) => path.join(path.dirname(fileURLToPath(import.meta.url)), p);
const SRC = process.env.CHATANNOUNCE_SRC ?? here("../src/chat.ts");
const PANE = process.env.CHATANNOUNCE_PANE ?? here("../src/ChatPane.tsx");

let failed = 0;
const fail = (m) => { failed++; console.log(`FAIL ${m}`); };
const ok = (m) => console.log(`ok   ${m}`);
const check = (cond, m) => (cond ? ok(m) : fail(m));
const done = () => {
  console.log(failed ? `\nchat-announce-check: ${failed} failure(s)` : "\nchat-announce-check: all passed");
  process.exit(failed ? 1 : 0);
};

const src = fs.readFileSync(SRC, "utf8")
  .replace(/^import \{ useSyncExternalStore \} from "react";$/m, "const useSyncExternalStore = () => undefined;")
  .replace(/^import \{ invoke, windowOnHost \} from "\.\/ipc";$/m, "const invoke = async () => { throw new Error(\"no ipc here\"); }; const windowOnHost = () => true;");
if (/^import /m.test(src)) fail("chat.ts gained an import this check does not stub — extend the stubs above");
const js = (await transformWithEsbuild(src, "chat.ts", { loader: "ts", format: "esm" })).code;
const C = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

for (const name of ["hear", "gather", "announcement", "promptKeyOf", "itemsOf"]) {
  if (typeof C[name] !== "function") fail(`chat.ts exports no ${name}`);
}
if (typeof C.ANNOUNCE_MS !== "number") fail("chat.ts exports no ANNOUNCE_MS");
if (failed) done();

// ── fixtures: turns the way `agent_turns` answers them (turns::Turn/Step) ──

const T0 = 1_793_470_000;
const text = (at, t) => ({ kind: "text", at, text: t });
const tool = (at, name) => ({ kind: "tool", at, name });
const permission = (at, answered_at) => ({ kind: "permission", at, answered_at, tool: "Bash", summary: "touch probe.txt", allowed: true, scope: "once" });
const turn = (at, user, steps) => ({
  at, user, reply: steps.filter((s) => s.kind === "text").map((s) => s.text).join("\n\n"),
  tools: steps.filter((s) => s.kind === "tool").map((s) => s.name), truncated: false, steps,
});
/** A turn from a host that predates `steps`. */
const legacy = (at, user, reply, tools = []) => ({ at, user, reply, tools, truncated: false });

const SHOWN = 1_000_000; // when the chat was shown (ms)
const readable = (fp) => ({ waiting: true, readable: true, harness: "claude", session: "wt-x", kind: "permission", fingerprint: fp, lines: ["Bash command", "", " touch probe.txt"], choices: [{ n: 1, label: "Yes", highlighted: true }, { n: 2, label: "No", highlighted: false }], note: "" });
const unreadable = (lines) => ({ waiting: true, readable: false, harness: "claude", session: "wt-x", reason: "taller than its pane", lines });
/** A snapshot of the view, both reads landing `after` ms after it was shown. */
const now = (turns, o = {}) => ({
  items: C.itemsOf(turns, o.answered ?? [], o.pending ?? []),
  turnsAt: o.turnsAt ?? SHOWN + (o.after ?? 10),
  prompt: o.prompt ?? null,
  promptAt: o.promptAt ?? SHOWN + (o.after ?? 10),
  answering: o.answering ?? false,
});
/** Run `hear` over a sequence of snapshots; the news of each step. */
function walk(snaps, { visible = true, shownAt = SHOWN } = {}) {
  let heard = null;
  const news = [];
  for (const s of snaps) {
    const r = C.hear(heard, s, shownAt, typeof visible === "function" ? visible(news.length) : visible);
    heard = r.heard;
    news.push(r.news);
  }
  return { heard, news };
}
const replies = (n) => (n ? n.replies : 0);
const promptOf = (n) => (n ? n.prompt : null);
const silent = (n) => n === null || (n.replies === 0 && n.prompt === null);

const base = [
  turn(T0, "Hello", [text(T0 + 2, "Hi there.")]),
  turn(T0 + 100, "Read the README", [tool(T0 + 101, "Read"), text(T0 + 105, "It says hello.")]),
];
const plusReply = [...base, turn(T0 + 200, "Thanks", [text(T0 + 201, "You're welcome.")])];

// ── 1. the first read, and a stale one, are history ─────────────────────────
{
  const { news } = walk([now(base, { prompt: readable("A") })]);
  check(silent(news[0]), "the first read of a conversation (and the prompt already up) is not announced");
}
{
  // The store held a read made before the chat was shown (another visit):
  // it is not the baseline, the first read landing after it is.
  const { news } = walk([now(base, { turnsAt: SHOWN - 5000, promptAt: SHOWN - 5000 }), now(plusReply, { prompt: readable("A") })]);
  check(news.every(silent), "a place switched to: what its first fresh read holds is history, not news (the store's older read is no baseline)");
}
{
  const { news } = walk([now(base), now(plusReply, { after: 20 })], { visible: false });
  const { news: after } = walk([now(base)].concat([now(plusReply, { after: 20 })]));
  check(news.every(silent) && replies(after[1]) === 1, "a reply read while the window is hidden is not announced (the same reply, shown, is)");
}
{
  // Hidden, then shown again: the read made while hidden is the baseline and
  // nothing it held is said later.
  const { news } = walk([now(base), now(plusReply, { after: 20 }), now(plusReply, { after: 30 })], { visible: (i) => i !== 1 });
  check(news.every(silent), "nothing read while hidden is replayed once the window is shown again");
}

// ── 2. a new reply, once ────────────────────────────────────────────────────
{
  const { news } = walk([now(base), now(plusReply, { after: 20 }), now(plusReply, { after: 30 })]);
  check(replies(news[1]) === 1, "a new agent reply is announced");
  check(silent(news[2]), "the same transcript read again announces nothing");
}
{
  // Two replies in one read: one announcement of two.
  const more = [...plusReply, turn(T0 + 300, "And?", [text(T0 + 301, "One."), tool(T0 + 302, "Bash"), text(T0 + 303, "Two.")])];
  const { news } = walk([now(plusReply), now(more, { after: 20 })]);
  check(replies(news[1]) === 2, "two reply blocks in one read count as two");
}
{
  // Two identical replies (the same words in the same second) are two.
  const twice = [...base, turn(T0 + 200, "Again", [text(T0 + 201, "Done."), text(T0 + 201, "Done.")])];
  const { news } = walk([now(base), now(twice, { after: 20 })]);
  check(replies(news[1]) === 2, "two replies alike are counted as two, not one");
}
{
  // The 20-turn window slides: the oldest turn goes as the newest comes.
  const slid = [...plusReply.slice(1), turn(T0 + 300, "More", [text(T0 + 301, "Sure.")])];
  const { news } = walk([now(plusReply), now(plusReply.slice(1), { after: 20 }), now(slid, { after: 30 })]);
  check(silent(news[1]), "the window dropping its oldest turn announces nothing");
  check(replies(news[2]) === 1, "the window sliding by one turn announces exactly the new reply");
  // In one read: as many replies as before, one of them new.
  const { news: one } = walk([now(plusReply), now(slid, { after: 20 })]);
  check(replies(one[1]) === 1, "a read where the oldest turn went and a new one came announces the new reply (counted by reply, not by length)");
}
{
  // A host that predates `steps`: the turn's reply grows as blocks land.
  const a = [legacy(T0, "Go", "")];
  const b = [legacy(T0, "Go", "Started.")];
  const c = [legacy(T0, "Go", "Started.", ["Bash"])];
  const { news } = walk([now(a), now(b, { after: 20 }), now(c, { after: 30 })]);
  check(replies(news[1]) === 1 && silent(news[2]), "an older host's reply is announced when it arrives, not when a tool joins it");
}

// ── 3. the person's own acts, and their echo ────────────────────────────────
{
  const pending = [{ id: 1, text: "Thanks", sentAt: (T0 + 199) * 1000, queued: false }];
  const echoed = [...base, turn(T0 + 200, "Thanks", [])];
  const { news } = walk([now(base), now(base, { pending, after: 20 }), now(echoed, { after: 30 })]);
  check(news.every(silent), "a send, and the transcript's echo of it (the pending message settling), are silent");
}
{
  const answered = [{ label: "Yes", at: (T0 + 150) * 1000, confirmed: true }];
  const echoed = [base[0], turn(T0 + 100, "Read the README", [tool(T0 + 101, "Read"), text(T0 + 105, "It says hello."), permission(T0 + 140, T0 + 150)])];
  const { news } = walk([
    now(base, { prompt: readable("A") }),
    now(base, { prompt: readable("A"), answering: true, after: 20 }),
    now(base, { answered, after: 30 }),
    now(echoed, { after: 40 }),
  ]);
  check(news.every(silent), "an answer, its line, and the transcript's asked-and-answered echo are silent");
}
{
  // A multi-select's free-text box filled from here moves the fingerprint
  // while the answer is in flight: that move is the person's own.
  const { news } = walk([
    now(base, { prompt: readable("A") }),
    now(base, { prompt: readable("A2"), answering: true, after: 20 }),
    now(base, { prompt: readable("A2"), after: 30 }),
  ]);
  check(news.every(silent), "a prompt that moves under the person's own answer is not announced");
}

// ── 4. tool calls ───────────────────────────────────────────────────────────
{
  const chip = [...base, turn(T0 + 200, "Run it", [tool(T0 + 201, "Bash")])];
  const chips = [...base, turn(T0 + 200, "Run it", [tool(T0 + 201, "Bash"), tool(T0 + 202, "Read")])];
  const joined = [base[0], turn(T0 + 100, "Read the README", [tool(T0 + 101, "Read"), text(T0 + 105, "It says hello."), tool(T0 + 106, "Grep")])];
  const { news } = walk([now(base), now(chip, { after: 20 }), now(chips, { after: 30 })]);
  check(news.every(silent), "a tool call drawn as a chip of its own is not announced");
  const { news: j } = walk([now(base), now(joined, { after: 20 })]);
  check(silent(j[1]), "a tool call joining an existing reply is not announced");
}

// ── 5. the prompt ───────────────────────────────────────────────────────────
{
  const { news } = walk([
    now(base),
    now(base, { prompt: readable("A"), after: 20 }),
    now(base, { prompt: readable("A"), after: 30 }),
    now(base, { prompt: readable("B"), after: 40 }),
    now(base, { after: 50 }),
    now(base, { prompt: unreadable(["Edit file", "  1. Yes"]), after: 60 }),
  ]);
  check(promptOf(news[1])?.key === C.promptKeyOf(readable("A")) && promptOf(news[1])?.readable === true, "a prompt appearing is announced, keyed as the card keys it");
  check(silent(news[2]), "the same prompt read again is not announced again");
  check(promptOf(news[3])?.key === C.promptKeyOf(readable("B")), "a different prompt (a new fingerprint) is announced");
  check(silent(news[4]), "a prompt going away is not announced");
  check(promptOf(news[5])?.readable === false, "an unreadable prompt appearing is announced, as one");
}
{
  // The card's rule: a prompt that went and came back is a new prompt (the
  // same command asked again after an answer in the terminal).
  const { news } = walk([now(base), now(base, { prompt: readable("A"), after: 20 }), now(base, { after: 30 }), now(base, { prompt: readable("A"), after: 40 })]);
  check(promptOf(news[3]) !== null, "a prompt that went and came back is announced again, as the card scrolls to it again");
}
{
  // After an answer the agent asks the next question: that is news.
  const { news } = walk([
    now(base, { prompt: readable("Q1") }),
    now(base, { prompt: readable("Q1"), answering: true, after: 20 }),
    now(base, { prompt: readable("Q2"), after: 30 }),
  ]);
  check(promptOf(news[2])?.key === C.promptKeyOf(readable("Q2")), "the next prompt after an answer is announced");
}
check(C.promptKeyOf(null) === null && C.promptKeyOf({ waiting: false }) === null, "no prompt has no key");
check(C.promptKeyOf(readable("fp1")) === "fp1", "a readable prompt's key is its fingerprint");
check(C.promptKeyOf(unreadable(["a"])) !== C.promptKeyOf(unreadable(["b"])), "two unreadable prompts are told apart by their rows, as the card does");

// ── 6. a burst is said once ─────────────────────────────────────────────────
check(C.ANNOUNCE_MS >= 1000 && C.ANNOUNCE_MS <= 2000, `the window is about 1–2 s (${C.ANNOUNCE_MS} ms)`);
{
  const W = C.ANNOUNCE_MS;
  let b = C.gather(null, { replies: 1, prompt: null }, 0);
  b = C.gather(b, { replies: 1, prompt: null }, 400);
  b = C.gather(b, { replies: 2, prompt: null }, W - 1);
  check(b.due === W && b.replies === 4, "news inside the window joins the first one's announcement, due when the window ends");
  check(C.announcement(b, "Claude", null, true) === "4 new messages", "a burst is one announcement: \"4 new messages\"");
  const after = C.gather(null, { replies: 1, prompt: null }, W + 1);
  check(after.due === 2 * W + 1, "news after the window opens a new one");
  // The window does not slide: a steady trickle cannot hold it open.
  let t = C.gather(null, { replies: 1, prompt: null }, 0);
  for (let ms = 100; ms < W; ms += 100) t = C.gather(t, { replies: 1, prompt: null }, ms);
  check(t.due === W, "a steady trickle does not push the announcement back");
}

// ── 7. the words ────────────────────────────────────────────────────────────
{
  const one = C.gather(null, { replies: 1, prompt: null }, 0);
  check(C.announcement(one, "Claude", null, true) === "1 new message", "one reply: \"1 new message\" (the iOS app's words)");
  const pk = C.promptKeyOf(readable("A"));
  const both = C.gather(one, { replies: 0, prompt: { key: pk, readable: true } }, 200);
  check(C.announcement(both, "Claude", pk, true) === "1 new message. Claude needs input", "a reply then a prompt: \"1 new message. Claude needs input\"");
  const pOnly = C.gather(null, { replies: 0, prompt: { key: pk, readable: true } }, 0);
  check(C.announcement(pOnly, "Codex", pk, true) === "Codex needs input", "a prompt: \"Codex needs input\" (the iOS app's spoken status)");
  const uk = C.promptKeyOf(unreadable(["x"]));
  const u = C.gather(null, { replies: 0, prompt: { key: uk, readable: false } }, 0);
  check(C.announcement(u, "Claude", uk, true) === "Claude needs input in the terminal", "an unreadable prompt: \"Claude needs input in the terminal\"");
  check(C.announcement(both, "Claude", null, true) === "1 new message", "a prompt answered before the window ended is not announced");
  check(C.announcement(pOnly, "Claude", C.promptKeyOf(readable("B")), true) === null, "a prompt replaced before the window ended is not announced as the old one");
  check(C.announcement(both, "Claude", pk, false) === null, "nothing is said when the window is hidden as the announcement falls due");
  // The words never carry what the agent wrote.
  const secret = [...base, turn(T0 + 200, "x", [text(T0 + 201, "SECRET-REPLY-TEXT")])];
  const r = walk([now(base), now(secret, { after: 20 })]).news[1];
  const said = C.announcement(C.gather(null, r, 0), "Claude", null, true) ?? "";
  check(said === "1 new message" && !JSON.stringify(r).includes("SECRET"), "the reply's own text stays out of the announcement");
}

// ── 8. the live region, in ChatPane.tsx ─────────────────────────────────────
{
  const pane = fs.readFileSync(PANE, "utf8");
  const regions = [...pane.matchAll(/<[a-z]+\b[^>]*\baria-live=/g)];
  check(regions.length === 1, `ChatPane has exactly one aria-live region (found ${regions.length})`);
  // The root may carry more attributes (onKeyDown: Esc stops a turn); the region is still its first child.
  const root = pane.match(/<div className="chat" data-testid="chat"[^>]*>\s*(<div\b[^>]*>)/);
  const tag = root?.[1] ?? "";
  check(/\baria-live="polite"/.test(tag) && /\baria-atomic="true"/.test(tag),
    "the region is the chat's first child, aria-live=\"polite\" aria-atomic=\"true\": mounted with the chat, before its first words");
  check(/\bclassName="sr-only"/.test(tag), "the region is visually hidden with the app's own .sr-only");
  const css = fs.readFileSync(here("../src/App.css"), "utf8");
  check(/^\.sr-only\s*\{/m.test(css), "App.css still defines .sr-only");
  check(/\bhear\(/.test(pane) && /document\.visibilityState === "visible"/.test(pane) && /document\.addEventListener\("visibilitychange"/.test(pane),
    "ChatPane decides with hear(), told whether the window is visible, and hears the window hide and come back");
  check(/promptKeyOf\(prompt\)/.test(pane), "the card scrolls to a prompt by the same key the announcement uses (promptKeyOf)");
  // The Terminal view: the chat, and so its region, is not mounted.
  const app = fs.readFileSync(here("../src/App.tsx"), "utf8");
  check(/chatOn && \(\s*<ChatPane /.test(app), "App mounts ChatPane (and its region) only while Chat is shown");
}

done();
