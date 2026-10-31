// The chat view of a place's agent (docs/proposals/chat-session-view.md): its
// state and its rules, apart from the component so the rules can be read in
// one place. Ported from the iOS app's View session (worktrees-ios
// `src/session.ts`, `src/answer.ts`, `docs/design/view-session.md`) and cut
// to what the desktop app is: the CONSOLE, so there is no step-up and no
// grant, and the terminal is one click away rather than on another machine.
//
// The four rows are serve's (`worktrees-api/src/agents.rs`):
// - `agent_turns` (read): the transcript's last turns, never the pane;
// - `agent_prompt` (read): a waiting modal, whole, with its own choices;
// - `agent_answer` (answer): one of those choices, a compare-and-swap on the
//   fingerprint the person was shown;
// - `agent_send` (instruct): ONE line, pasted into an EMPTY composer. The only
//   path that types into Claude (AGENTS.md); the composer adds no second one.
//
// Memory only: turns, prompts, drafts and pending messages live for the
// window's life and are fetched again after a reload. Nothing is written.

import { useSyncExternalStore } from "react";
import { invoke, windowOnHost } from "./ipc";

// ── the rows' shapes ────────────────────────────────────────────────────────

/** `turns::Turn`. `user: null` when the window starts inside the turn. */
export type Turn = { at: number | null; user: string | null; reply: string; tools: string[]; truncated: boolean };
/** `agent_turns`' answer; `reason` only when no agent is running. */
export type AgentTurns = { harness?: string; session?: string; turns: Turn[]; more: boolean; note: string; reason?: string };
/** `prompt::Choice`. */
export type PromptChoice = { n: number; label: string; more?: string[]; highlighted: boolean; host_only?: "widens" | "text" };
export type AgentPrompt =
  | { waiting: false }
  | { waiting: true; readable: true; harness: string; session: string; kind: "permission" | "question"; fingerprint: string; lines: string[]; choices: PromptChoice[]; note: string }
  | { waiting: true; readable: false; harness: string | null; session: string | null; reason: string };
export type ReadablePrompt = Extract<AgentPrompt, { readable: true }>;
export type AgentSent = { delivered: boolean; queued: boolean; harness: string; session: string; note: string };
export type AgentAnswer = { answered: true; confirmed: boolean; harness: string; session: string; choice: number; label: string };

/** agents.rs's refusal strings, which the words below key on. */
export const AGENT_ERR = {
  changed: "the prompt changed",
  noChoice: "no such choice",
  promptUp: "the agent is waiting on a prompt: answer it with agent_answer",
  midInput: "the agent's prompt has unsent text in it (someone is typing there): send nothing until it is empty",
  noComposer: "the agent is not at its prompt (starting, or a screen this host does not recognise): try again shortly",
  /** `agent_send` / `agent_turns` naming a session no agent of the place runs in. */
  noLanePrefix: "no agent this place launched is running in ",
  /** `agent_turns` for pi and kimi. */
  turnsUnsupportedPrefix: "agent_turns reads Claude's and Codex's transcripts;",
} as const;

/** `harness::LINE_MAX`. */
export const SEND_MAX = 4 * 1024;
/** `turns::MAX_TURNS`: the most one `agent_turns` answers. */
export const TURNS_MAX = 20;

// ── the composer's rule ─────────────────────────────────────────────────────

export const bytes = (text: string) => new TextEncoder().encode(text).length;

/** Why `agent_send` would refuse a draft, in words, or null. The same rule as
 *  `harness::typed_line_ok` (MCP `send` shares it), checked before the line
 *  leaves so a refusal is explained where it is typed. A pasted draft is never
 *  changed to make it pass. */
export function draftProblem(text: string): string | null {
  if (!text.trim()) return null;
  if (/\p{Cc}/u.test(text)) return "Use one line: a message can’t hold line breaks, tabs or other control characters.";
  if (/[\p{Cf}\p{Zl}\p{Zp}]/u.test(text)) return "Remove the invisible formatting characters: the agent would read something other than what you see.";
  if (bytes(text) > SEND_MAX) return `Too long: ${bytes(text).toLocaleString()} of ${SEND_MAX.toLocaleString()} bytes.`;
  const lead = text.trimStart()[0];
  if ("/@!#".includes(lead)) {
    return lead === "/"
      ? "Slash commands run in the terminal: a message can’t start with /."
      : `A message can’t start with ${lead}: at the agent’s prompt that runs something, it is not a message.`;
  }
  return null;
}

/** What a refused or unconfirmed send says under the field. `terminal` offers
 *  the way back; `prompt` re-reads the prompt the refusal says is up. */
export type SendWords = { line: string; terminal?: boolean; prompt?: boolean };

export function sendWords(err: string, agent: string): SendWords {
  if (err === AGENT_ERR.promptUp) return { line: `${agent} is waiting on the prompt above. Answer it first.`, prompt: true };
  if (err === AGENT_ERR.midInput) {
    return { line: `${agent}’s input already has text in it, typed in the terminal. Send it or clear it there, then try again.`, terminal: true };
  }
  if (err === AGENT_ERR.noComposer) return { line: `${agent} isn’t at its prompt yet (starting up, or a screen this app doesn’t know). Try again in a moment.`, terminal: true };
  if (err.startsWith(AGENT_ERR.noLanePrefix)) return { line: `No ${agent} is running in this place any more.` };
  // typed_line_ok's own words, should the client's check ever drift from it.
  if (err.startsWith("text ")) return { line: `Not sent: ${err}.` };
  return { line: `Not sent: ${err}`, terminal: true };
}

/** The words for a send that typed the line and could not see it submitted. */
export const UNCONFIRMED = "Typed, but not confirmed. Look at the terminal before sending it again.";

// ── the conversation ────────────────────────────────────────────────────────

/** A message `agent_send` confirmed, held until the transcript has it. */
export type Pending = { id: number; text: string; sentAt: number; queued: boolean };
/** An answer this window sent. */
export type Answered = { label: string; at: number; confirmed: boolean };

export type Item =
  | { k: "sep"; key: string; at: number }
  | { k: "earlier"; key: string }
  | { k: "user"; key: string; text: string; at: number | null }
  | { k: "reply"; key: string; text: string; tools: string[]; truncated: boolean; at: number | null; label: boolean }
  | { k: "answered"; key: string; a: Answered }
  | { k: "pending"; key: string; p: Pending };

const GAP_S = 5 * 60;
const sameDay = (a: number, b: number) => new Date(a * 1000).toDateString() === new Date(b * 1000).toDateString();

/** Turns oldest first, with a time separator at a gap of more than five
 *  minutes or a new day, this window's answers where they fall in time, and
 *  the messages the transcript has not shown yet at the end. */
export function itemsOf(turns: Turn[], answered: Answered[], pending: Pending[]): Item[] {
  const out: Item[] = [];
  let prevAt: number | null = null;
  let lastWasReply = false;
  const queue = [...answered];
  const flushAnswered = (beforeSec: number | null) => {
    while (queue.length && (beforeSec === null || queue[0].at / 1000 < beforeSec)) {
      const a = queue.shift()!;
      out.push({ k: "answered", key: `a${a.at}`, a });
      lastWasReply = false;
    }
  };
  turns.forEach((t, i) => {
    if (t.at !== null) flushAnswered(t.at);
    if (t.at !== null && (prevAt === null || t.at - prevAt > GAP_S || !sameDay(t.at, prevAt))) out.push({ k: "sep", key: `s${i}`, at: t.at });
    if (t.at !== null) prevAt = t.at;
    if (t.user === null) out.push({ k: "earlier", key: `e${i}` });
    else {
      out.push({ k: "user", key: `u${i}`, text: t.user, at: t.at });
      lastWasReply = false;
    }
    if (t.reply || t.tools.length || t.truncated) {
      out.push({ k: "reply", key: `r${i}`, text: t.reply, tools: t.tools, truncated: t.truncated, at: t.at, label: !lastWasReply });
      lastWasReply = true;
    }
  });
  flushAnswered(null);
  for (const p of pending) out.push({ k: "pending", key: `p${p.id}`, p });
  return out;
}

/** How far the host's clock may run behind when a pending message is matched. */
const SKEW_S = 120;

/** Drop the pending messages the transcript now holds: newest first, each
 *  taking the newest unmatched user turn with its text written after it was
 *  sent, so two identical messages resolve to two turns. */
export function settlePending(pending: Pending[], turns: Turn[]): Pending[] {
  const taken = new Set<number>();
  const keep: Pending[] = [];
  for (const p of [...pending].sort((a, b) => b.sentAt - a.sentAt)) {
    const after = Math.floor(p.sentAt / 1000) - SKEW_S;
    let hit = -1;
    for (let i = turns.length - 1; i >= 0; i--) {
      const t = turns[i];
      if (!taken.has(i) && t.user === p.text && (t.at ?? 0) >= after) { hit = i; break; }
    }
    if (hit >= 0) taken.add(hit);
    else keep.push(p);
  }
  return keep.sort((a, b) => a.sentAt - b.sentAt);
}

const clock = (ms: number) => new Date(ms).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
export const clockOf = (sec: number) => clock(sec * 1000);
export const clockMs = clock;

/** "Today 9:36", "Yesterday 18:02", or the date. */
export function dayLabel(at: number, now = Date.now()): string {
  const d = new Date(at * 1000);
  const t = clockOf(at);
  if (d.toDateString() === new Date(now).toDateString()) return `Today ${t}`;
  if (d.toDateString() === new Date(now - 86400_000).toDateString()) return `Yesterday ${t}`;
  return `${d.toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short" })} ${t}`;
}

/** The rows above a prompt's choices: every row down to its first choice,
 *  their shared indent dropped. The choices are the buttons, and what follows
 *  them is the harness's key hints. A drawing this cannot place shows whole. */
export function promptBody(p: ReadablePrompt): string[] {
  const first = p.choices[0];
  const at = first ? p.lines.findIndex((l) => new RegExp(`^\\s*(?:[❯›▶→]\\s*)?${first.n}\\.\\s`).test(l)) : -1;
  const rows = at > 0 ? p.lines.slice(0, at) : [...p.lines];
  while (rows.length && !rows[rows.length - 1].trim()) rows.pop();
  const indent = Math.min(...rows.filter((l) => l.trim()).map((l) => l.length - l.trimStart().length));
  return rows.map((l) => (Number.isFinite(indent) ? l.slice(indent) : l));
}

/** Whether a choice can be made from this window. The console may make a
 *  `widens` choice (agents.rs lets `is_console()` through); a window driving
 *  a paired host may not. A `text` choice opens a field inside the modal,
 *  which the chat cannot fill, so it is the terminal's everywhere. */
export function choiceOpen(c: PromptChoice): { open: boolean; note?: string } {
  if (c.host_only === "text") return { open: false, note: "Opens a text field: answer this one in the terminal" };
  if (c.host_only === "widens") {
    return windowOnHost()
      ? { open: true, note: "Changes what the agent may do from now on" }
      : { open: false, note: "Answer this one on the host" };
  }
  return { open: true };
}

// ── the store: one view per place, keyed by its path ────────────────────────

export type Target = { repo: string; slug: string; path: string; session: string };
type Load<T> = { data: T | null; error: string | null; loading: boolean; at: number | null };

export type ChatView = {
  turns: Load<AgentTurns>;
  prompt: Load<AgentPrompt>;
  pending: Pending[];
  answered: Answered[];
  draft: string;
  sending: boolean;
  sendWords: SendWords | null;
  /** The selected choice: an answer takes a click on it, then a second on
   *  "Send answer", because an answer can run a command. */
  selected: number | null;
  answering: boolean;
  answerLine: string | null;
};

const blank = (): ChatView => ({
  turns: { data: null, error: null, loading: false, at: null },
  prompt: { data: null, error: null, loading: false, at: null },
  pending: [], answered: [], draft: "", sending: false, sendWords: null,
  selected: null, answering: false, answerLine: null,
});
const EMPTY = blank();
let views = new Map<string, ChatView>();
const subs = new Set<() => void>();
const viewOf = (path: string) => views.get(path) ?? EMPTY;
function put(path: string, patch: Partial<ChatView>) {
  views = new Map(views).set(path, { ...viewOf(path), ...patch });
  for (const f of [...subs]) f();
}

export function useChat(path: string): ChatView {
  return useSyncExternalStore(
    (f) => { subs.add(f); return () => { subs.delete(f); }; },
    () => viewOf(path),
  );
}

const flights = new Map<string, Promise<void>>();
/** One request of a kind per place in flight; a second ask joins it. */
function once(key: string, run: () => Promise<void>): Promise<void> {
  const f = flights.get(key);
  if (f) return f;
  const p = run().finally(() => flights.delete(key));
  flights.set(key, p);
  return p;
}

/** The place's latest turns, at the row's maximum. */
export function loadTurns(t: Target): Promise<void> {
  return once(`t|${t.path}`, async () => {
    put(t.path, { turns: { ...viewOf(t.path).turns, loading: true } });
    try {
      const r = await invoke<AgentTurns>("agent_turns", { repo: t.repo, slug: t.slug, session: t.session, n: TURNS_MAX });
      const cur = viewOf(t.path);
      put(t.path, { turns: { data: r, error: null, loading: false, at: Date.now() }, pending: settlePending(cur.pending, r.turns) });
    } catch (e) {
      put(t.path, { turns: { ...viewOf(t.path).turns, loading: false, error: String(e) } });
    }
  });
}

export const turnsUnsupported = (err: string | null) => !!err && err.startsWith(AGENT_ERR.turnsUnsupportedPrefix);

export const CHANGED = "Not sent: the prompt changed before the answer reached it. Someone may have answered it in the terminal.";

/** The place's prompt. One that is not the prompt a selection was made on
 *  clears the selection, and says so; a choice never carries over. */
export function loadPrompt(t: Target): Promise<void> {
  return once(`p|${t.path}`, async () => {
    const before = viewOf(t.path);
    put(t.path, { prompt: { ...before.prompt, loading: true } });
    try {
      const r = await invoke<AgentPrompt>("agent_prompt", { repo: t.repo, slug: t.slug });
      const cur = viewOf(t.path);
      const fp = (p: AgentPrompt | null) => (p && p.waiting && p.readable ? p.fingerprint : null);
      const moved = cur.selected !== null && fp(cur.prompt.data) !== fp(r);
      put(t.path, {
        prompt: { data: r, error: null, loading: false, at: Date.now() },
        ...(moved ? { selected: null, answerLine: CHANGED } : {}),
      });
    } catch (e) {
      put(t.path, { prompt: { ...viewOf(t.path).prompt, loading: false, error: String(e) } });
    }
  });
}

export function setDraft(path: string, draft: string): void {
  put(path, { draft, sendWords: null });
}

let nextPending = 1;

/** Send the draft. It clears only on a confirmed delivery. */
export async function sendDraft(t: Target, agent: string): Promise<void> {
  const v = viewOf(t.path);
  const text = v.draft;
  if (v.sending || !text.trim() || draftProblem(text)) return;
  put(t.path, { sending: true, sendWords: null });
  try {
    const r = await invoke<AgentSent>("agent_send", { repo: t.repo, slug: t.slug, session: t.session, text });
    const cur = viewOf(t.path);
    if (!r.delivered) {
      put(t.path, { sending: false, sendWords: { line: UNCONFIRMED, terminal: true } });
    } else {
      put(t.path, {
        sending: false,
        // Only if nothing new was typed meanwhile.
        draft: cur.draft === text ? "" : cur.draft,
        pending: [...cur.pending, { id: nextPending++, text, sentAt: Date.now(), queued: r.queued }],
      });
    }
    void loadTurns(t);
  } catch (e) {
    const w = sendWords(String(e), agent);
    put(t.path, { sending: false, sendWords: w });
    if (w.prompt) void loadPrompt(t);
  }
}

export function selectChoice(path: string, n: number | null): void {
  const v = viewOf(path);
  if (v.answering) return;
  put(path, { selected: n, answerLine: null });
}

/** Answer with the selected choice, against the fingerprint on screen. */
export async function sendAnswer(t: Target): Promise<void> {
  const v = viewOf(t.path);
  const d = v.prompt.data;
  if (v.answering || v.selected === null || !d || !d.waiting || !d.readable) return;
  const c = d.choices.find((x) => x.n === v.selected);
  if (!c || !choiceOpen(c).open) return;
  put(t.path, { answering: true, answerLine: null });
  try {
    const r = await invoke<AgentAnswer>("agent_answer", { repo: t.repo, slug: t.slug, choice: c.n, fingerprint: d.fingerprint });
    const cur = viewOf(t.path);
    put(t.path, { answering: false, selected: null, answered: [...cur.answered, { label: r.label, at: Date.now(), confirmed: r.confirmed }] });
  } catch (e) {
    const err = String(e);
    const changed = err === AGENT_ERR.changed || err === AGENT_ERR.noChoice;
    put(t.path, {
      answering: false, selected: null,
      answerLine: changed ? CHANGED : `Not confirmed: ${err}. Check the terminal before answering again.`,
    });
  }
  void loadPrompt(t);
  void loadTurns(t);
}
