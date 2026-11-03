// The chat view of a place's agent (docs/proposals/chat-session-view.md): its
// state and its rules, apart from the component so the rules can be read in
// one place. Ported from the iOS app's View session (worktrees-ios
// `src/session.ts`, `src/answer.ts`, `docs/design/view-session.md`) and cut
// to what the desktop app is: the CONSOLE, so there is no step-up and no
// grant, and the terminal is one click away rather than on another machine.
//
// The rows are serve's (`worktrees-api/src/agents.rs`):
// - `agent_turns` (read): the transcript's last turns, never the pane;
// - `agent_prompt` (read): a waiting modal, whole, with its own choices, and
//   for one taller than the pane the whole of what it asks (`detail`);
// - `agent_answer` (answer): one of those choices, a compare-and-swap on the
//   fingerprint the person was shown;
// - `agent_answer_text` (instruct + answer): a choice with free text, a field
//   ("Type something.") or a note riding on it (Claude's `Tab to amend`);
// - `agent_answer_many` (answer): a multi-select's boxes, sent together;
// - `agent_send` (instruct): ONE line, pasted into an EMPTY composer. The only
//   path that types into Claude (AGENTS.md); the composer adds no second one.
// - `agent_interrupt` (instruct): stop the running turn — ONE Esc, pressed
//   only on a screen that shows the turn running, a key and never text. A
//   stop not seen to land is `unconfirmed` and is never pressed again.
// - `agent_attach` (instruct): a file for the agent, uploaded and kept in the
//   place; its answer is a path, which the message then names. Nothing types
//   it: the line still goes through `agent_send`
//   (docs/proposals/chat-attachments.md).
//
// Memory only: turns, prompts, drafts and pending messages live for the
// window's life and are fetched again after a reload. Nothing is written.

import { useSyncExternalStore } from "react";
import { invoke, windowOnHost } from "./ipc";
import { convertPhoto, jpegName, photoDecision, photoKind, INPUT_MAX } from "./chat-image";

// ── the rows' shapes ────────────────────────────────────────────────────────

/** `turns::Turn`. `user: null` when the window starts inside the turn. */
export type Turn = {
  at: number | null; user: string | null; reply: string; tools: string[]; truncated: boolean;
  /** Claude's own entry, never the person's message. Older hosts omit it. */
  notice?: Notice;
  /** The turn in order (`turns::Step`): absent from a host that predates it. */
  steps?: Step[];
};
export type Notice =
  | { kind: "task"; status: string; summary: string }
  | { kind: "peer_message"; from_name: string; text: string }
  | { kind: "command"; name: string; args: string; output?: string }
  | { kind: "shell"; command: string; output?: string; stderr?: string }
  | { kind: "interrupt" }
  | { kind: "summary"; text: string };
/** `turns::Step`: the reply's blocks, the tools, and what a person was asked
 *  and answered on the way, each at its own time. */
export type Step =
  | { kind: "text"; at: number | null; text: string }
  | { kind: "user"; at: number | null; text: string }
  | { kind: "tool"; at: number | null; name: string; summary?: string; failed?: boolean }
  | { kind: "question"; at: number | null; answered_at?: number; questions: AskedQuestion[]; answer?: { how: "picked" | "chat" | "declined"; answers?: QuestionAnswer[] } }
  | { kind: "permission"; at: number | null; answered_at?: number; tool: string; summary?: string; allowed: boolean; scope?: string; note?: string }
  | { kind: "plan"; at: number | null; answered_at?: number; plan: string; plan_file?: string; approved?: boolean; note?: string };
export type AskedQuestion = { header: string; question: string; multi: boolean; options: { label: string; description?: string }[] };
export type QuestionAnswer = { question: string; picked: string[]; typed?: string; note?: string };
export type AskedStep = Extract<Step, { kind: "question" | "permission" | "plan" }>;
/** `agent_turns`' answer; `reason` only when no agent is running. */
export type AgentTurns = { harness?: string; session?: string; turns: Turn[]; more: boolean; note: string; reason?: string };
/** `prompt::Choice`. `checked` marks a multi-select's box; `text` a choice
 *  that takes free text: `field` IS a text field, `amend` takes an optional
 *  note. `host_only: "text"` on a field means it is empty. `preview` is what
 *  Claude shows beside an option of a `previews` question, its lines and
 *  spacing kept; `preview_hidden` rows of it are cut off in the terminal. */
export type PromptChoice = {
  n: number; label: string; more?: string[]; highlighted: boolean;
  host_only?: "widens" | "text"; checked?: boolean; text?: "field" | "amend";
  preview?: string; preview_hidden?: number;
};
/** A `previews` question's ONE notes field (`prompt::Notes`): `open` while
 *  it has the keyboard in the terminal, `text` once someone typed there. */
export type PromptNotes = { open: boolean; text?: string };
/** What a prompt taller than the pane asks, whole: Claude's own tool call
 *  (its PermissionRequest hook) or a plan's own file. */
export type PromptDetail = { tool: string; input: Record<string, unknown>; from: "hook" | "plan-file" };
export type AgentPrompt =
  | { waiting: false }
  | {
    waiting: true; readable: true; harness: string; session: string; kind: "permission" | "question";
    fingerprint: string; lines: string[]; choices: PromptChoice[]; note: string; multi?: boolean; detail?: PromptDetail;
    /** Claude's own resume picker (`claude -r`): a question whose choices are
     *  the conversations on screen; `hidden` more are in the terminal's list. */
    picker?: "resume"; hidden?: number;
    /** Claude's question with a preview per option: `all` (every option's,
     *  from Claude's own call) or `shown` (only the one the terminal shows). */
    previews?: "all" | "shown"; notes?: PromptNotes;
  }
  | { waiting: true; readable: false; harness: string | null; session: string | null; reason: string; lines?: string[]; note?: string };
export type ReadablePrompt = Extract<AgentPrompt, { readable: true }>;
export type AgentSent = { delivered: boolean; queued: boolean; harness: string; session: string; note: string };
export type AgentAnswer = { answered: true; confirmed: boolean; harness: string; session: string; choice: number; label: string };
/** A multi-select's free-text box, filled: nothing is sent yet. */
export type AgentFilled = { answered: false; harness: string; session: string; choice: number; note: string };
export type AgentAnsweredMany = { answered: true; confirmed: boolean; harness: string; session: string; choices: number[]; labels: string[] };
/** `agent_interrupt`'s answer: stopped, or pressed and not seen to stop. */
export type AgentStopped = { interrupted: boolean; unconfirmed?: boolean; harness: string; session: string; note: string };
/** `agent_attach`'s answer: where the host kept the file, relative to the
 *  place root, and what its bytes are. `name` is the display name, cleaned. */
export type AgentAttached = { path: string; bytes: number; sha256: string; kind: "image" | "pdf" | "text"; name?: string };

/** agents.rs's refusal strings, which the words below key on. */
export const AGENT_ERR = {
  changed: "the prompt changed",
  noChoice: "no such choice",
  promptUp: "the agent is waiting on a prompt: answer it with agent_answer",
  midInput: "the agent's prompt has unsent text in it (someone is typing there): send nothing until it is empty",
  noComposer: "the agent is not at its prompt (starting, or a screen this host does not recognise): try again shortly",
  fieldHeld: "that choice's text field holds someone's text: answer it at the host, or choose it as it reads with agent_answer",
  multi: "a multi-select: tick its boxes with agent_answer_many",
  notMulti: "not a multi-select: answer it with agent_answer",
  notesOpen: "someone has this question's notes field open in the terminal: answer it at the host",
  noBox: "name at least one box to tick",
  /** `agent_send` / `agent_turns` naming a session no agent of the place runs in. */
  noLanePrefix: "no agent this place launched is running in ",
  /** `agent_turns` for pi and kimi. */
  turnsUnsupportedPrefix: "agent_turns reads Claude's and Codex's transcripts;",
  /** `agent_interrupt` with no turn running. */
  notBusy: "the agent is not working on a turn: there is nothing to stop",
  /** `agent_interrupt` while the terminal's input holds a draft. */
  stopDraft: "the agent's prompt has unsent text in it (someone is typing there), which hides whether a turn is running: stop it at the host",
  /** `agent_interrupt` for pi and kimi. */
  stopUnsupportedPrefix: "agent_interrupt stops Claude's and Codex's turns;",
  /** `agent_interrupt` in a place with no agent running. */
  noAgent: "no agent is running in this place",
  /** How `agent_turns` ends for a Claude with no session yet (`the claude in
   *  <session> has written no session file yet`): its resume picker is up,
   *  or it is starting. */
  noSessionYet: "has written no session file yet",
} as const;

/** `harness::LINE_MAX`. */
export const SEND_MAX = 4 * 1024;
/** `turns::MAX_TURNS`: the most one `agent_turns` answers. */
export const TURNS_MAX = 20;

// ── attachments (docs/proposals/chat-attachments.md) ────────────────────────

/** `attach::MAX_BYTES`: the most one uploaded file may hold. HEIC inputs may
 *  be larger; their converted JPEG must fit this cap before upload. */
export const ATTACH_MAX = 8 * 1024 * 1024;
/** The longest `path` `agent_attach` answers:
 *  `.worktrees-attachments/<yyyymmdd>-<16 hex>.<ext>`, `webp` the longest
 *  extension. A message is measured with every path this long BEFORE any
 *  upload, so a line the host would refuse never leaves files behind. */
export const ATTACH_PATH_MAX = 53;
/** The most files one message carries. A place holds 50. */
export const ATTACH_PER_MESSAGE = 10;
/** `agent_attach`'s refusal tokens (`attach::ERR_*`, `budget::ATTACH_BUDGET`):
 *  each is followed by `: <detail>`, except `unsafe`, which stands alone. */
export const ATTACH_ERR = {
  type: "attach-type", size: "attach-size", files: "attach-files", bytes: "attach-bytes",
  budget: "attach-budget", unsafe: "attach-unsafe",
} as const;

/** The lead-in of a message that is only attachments. */
export const attachLead = (n: number) => (n === 1 ? "Look at the attached file." : "Look at the attached files.");

/** The ONE line a message with attachments is sent as (the iOS app's shape
 *  too): the text, or the lead-in when there is none, then
 *  ` [attached: <path>, <path>]`, the paths in upload order. */
export function attachLine(text: string, paths: string[]): string {
  if (!paths.length) return text;
  const lead = text.trim() ? text.replace(/\s+$/u, "") : attachLead(paths.length);
  return `${lead} [attached: ${paths.join(", ")}]`;
}

const ATTACHED = /^(.*) \[attached: (\.worktrees-attachments\/\d{8}-[0-9a-f]{16}\.[a-z]{2,4}(?:, \.worktrees-attachments\/\d{8}-[0-9a-f]{16}\.[a-z]{2,4})*)\]$/su;

/** A sent line's attachments, split off its end so they can be drawn as
 *  files: what came before ` [attached: …]`, and the paths. Only the host's
 *  own path shape counts; anything else is all text. */
export function splitAttached(line: string): { text: string; paths: string[] } {
  const m = ATTACHED.exec(line);
  return m ? { text: m[1], paths: m[2].split(", ") } : { text: line, paths: [] };
}

/** Why the message would be refused, checked BEFORE anything is uploaded:
 *  the text by today's rule, then the line it becomes with `n` attachments,
 *  every path at its longest. */
export function attachProblem(text: string, n: number): string | null {
  const own = draftProblem(text);
  if (own || !n) return own;
  if (n > ATTACH_PER_MESSAGE) return `At most ${ATTACH_PER_MESSAGE} files in one message.`;
  const line = attachLine(text, Array.from({ length: n }, () => "x".repeat(ATTACH_PATH_MAX)));
  if (bytes(line) > SEND_MAX) {
    return `Too long with ${n === 1 ? "the attachment" : `${n} attachments`}: ${bytes(line).toLocaleString()} of ${SEND_MAX.toLocaleString()} bytes.`;
  }
  return draftProblem(line);
}

/** A size in words: `512 B`, `34 KB`, `2.4 MB`. */
export function sizeWords(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`;
  return `${(n / (1024 * 1024)).toFixed(n < 10 * 1024 * 1024 ? 1 : 0).replace(/\.0$/, "")} MB`;
}

/** What a refused upload says under the field: `name` is the file's. */
export function attachWords(err: string, name: string): SendWords {
  const detail = (tok: string) => err.slice(tok.length + 2);
  const shown = `“${name}”`;
  if (err.startsWith(`${ATTACH_ERR.type}: `)) {
    const what = detail(ATTACH_ERR.type).replace(/ \(accepted: [^)]*\)$/, "");
    const convert = /^(HEIC|HEIF|AVIF)$/.test(what) ? " Export it as JPEG or PNG first." : "";
    return { line: `Not attached: ${shown} is ${what}. Attach PNG, JPEG, GIF, WebP, PDF or text.${convert}` };
  }
  if (err.startsWith(`${ATTACH_ERR.size}: `)) return { line: `Not attached: ${shown} is over the ${sizeWords(Number(detail(ATTACH_ERR.size)))} limit.` };
  if (err.startsWith(`${ATTACH_ERR.files}: `)) {
    return { line: `Not attached: this place already holds ${detail(ATTACH_ERR.files)} attachments. Each is removed after 7 days.` };
  }
  if (err.startsWith(`${ATTACH_ERR.bytes}: `)) {
    return { line: `Not attached: this place’s attachments already take ${sizeWords(Number(detail(ATTACH_ERR.bytes)))}. Each is removed after 7 days.` };
  }
  if (err.startsWith(`${ATTACH_ERR.budget}: `)) return { line: `Not attached: too much was uploaded too quickly. Try again in ${detail(ATTACH_ERR.budget)} s.` };
  if (err === ATTACH_ERR.unsafe) {
    return { line: "Not attached: this place’s .worktrees-attachments folder isn’t safe to write to (a link, or a folder git would show).", terminal: true };
  }
  return { line: `Not attached: ${err}` };
}

/** A file picked, pasted or dropped, until its message is sent. `path` once
 *  the host has it: a retry after a failed send does not upload it again. */
export type Attachment = { id: number; name: string; size: number; file: Blob; state: "preparing" | "ready" | "uploading" | "attached" | "failed"; path?: string; converted?: boolean };

// ── the composer's rule ─────────────────────────────────────────────────────

export const bytes = (text: string) => new TextEncoder().encode(text).length;

/** Why `agent_send` (or `agent_answer_text`, which shares the rule) would
 *  refuse `text`, in words, or null. The same rule as `harness::typed_line_ok`
 *  (MCP `send` shares it), checked before the line leaves so a refusal is
 *  explained where it is typed. A pasted draft is never changed to make it
 *  pass. `what` names the thing typed: a message, an answer, a note. */
export function draftProblem(text: string, what: "message" | "answer" | "note" = "message"): string | null {
  if (!text.trim()) return null;
  const one = `${what === "answer" ? "an" : "a"} ${what}`;
  if (/\p{Cc}/u.test(text)) return `Use one line: ${one} can’t hold line breaks, tabs or other control characters.`;
  if (/[\p{Cf}\p{Zl}\p{Zp}]/u.test(text)) return "Remove the invisible formatting characters: the agent would read something other than what you see.";
  if (bytes(text) > SEND_MAX) return `Too long: ${bytes(text).toLocaleString()} of ${SEND_MAX.toLocaleString()} bytes.`;
  const lead = text.trimStart()[0];
  if ("/@!#".includes(lead)) {
    return lead === "/"
      ? `Slash commands run in the terminal: ${one} can’t start with /.`
      : `${one[0].toUpperCase()}${one.slice(1)} can’t start with ${lead}: at the agent’s prompt that runs something, it is not ${one}.`;
  }
  return null;
}

/** What a refused or unconfirmed send says under the field. `terminal` offers
 *  the way back; `prompt` re-reads the prompt the refusal says is up. */
export type SendWords = { line: string; terminal?: boolean; prompt?: boolean };
/** What a refused or unconfirmed answer says on the card. */
export type Words = SendWords;

export function sendWords(err: string, agent: string): SendWords {
  if (err === AGENT_ERR.promptUp) return { line: `${agent} is waiting on the prompt above. Answer it first.`, prompt: true };
  if (err === AGENT_ERR.midInput) {
    return { line: `${agent}’s input already has text in it, typed in the terminal. Send it or clear it there, then try again.`, terminal: true };
  }
  if (err === AGENT_ERR.noComposer) return { line: `${agent} isn’t at its prompt yet (starting up, or a screen this app doesn’t know). Try again in a moment.`, terminal: true };
  if (err.startsWith(AGENT_ERR.noLanePrefix)) return { line: "No agent is running in this place." };
  // typed_line_ok's own words, should the client's check ever drift from it.
  if (err.startsWith("text ")) return { line: `Not sent: ${err}.` };
  return { line: `Not sent: ${err}`, terminal: true };
}

/** What a refused answer says on the card. `prompt` re-reads the prompt: the
 *  refusal says it is not the one the card shows. The `…answer it at the
 *  host` family typed nothing, or nothing confirmed, and so says to look. */
export function answerWords(err: string): Words {
  if (err === AGENT_ERR.changed || err === AGENT_ERR.noChoice || / is not one of the boxes$/.test(err)) return { line: CHANGED, prompt: true };
  if (err === AGENT_ERR.fieldHeld) {
    return { line: "Someone has typed into this choice’s field in the terminal. Answer it there, or pick the choice as it reads now.", terminal: true, prompt: true };
  }
  if (err === AGENT_ERR.multi) return { line: "This prompt is a multi-select: tick its boxes, then send them.", prompt: true };
  if (err === AGENT_ERR.notMulti) return { line: "This prompt is no longer a multi-select. It has been read again.", prompt: true };
  if (err === AGENT_ERR.notesOpen) return { line: "Someone is typing a note into this question in the terminal. Answer it there.", terminal: true, prompt: true };
  if (err === AGENT_ERR.noBox) return { line: "Tick at least one box." };
  if (/ takes no text: /.test(err)) return { line: "This choice takes no note. Send it without one.", prompt: true };
  if (/ is the free-text box: /.test(err)) return { line: "Type your own answer in its box before sending it." };
  if (/ text field did not open; /.test(err)) return { line: "The choice’s text field didn’t open, so nothing was typed. Answer it in the terminal.", terminal: true, prompt: true };
  if (/ could not be seen there; /.test(err)) {
    return { line: "The text was typed into the field but couldn’t be seen there, so nothing was sent. Look at the terminal before answering again.", terminal: true, prompt: true };
  }
  if (/ can change what the agent may do from now on, so only the host can make it$/.test(err)) return { line: "Only the host can make this choice: it changes what the agent may do from now on.", terminal: true };
  if (/ opens a text field in the prompt — answer it at the host$/.test(err)) return { line: "This choice opens a text field the app can’t fill. Answer it in the terminal.", terminal: true };
  if (/; nothing was (sent|confirmed|typed) — answer it at the host$/.test(err) || / — answer it at the host$/.test(err)) {
    return { line: `Nothing was answered: ${err.replace(/ — answer it at the host$/, "")}. Answer it in the terminal.`, terminal: true, prompt: true };
  }
  if (err.startsWith("text ")) return { line: `Not sent: ${err}.` };
  return { line: `Not confirmed: ${err}. Check the terminal before answering again.`, terminal: true, prompt: true };
}

/** The words for a send that typed the line and could not see it submitted. */
export const UNCONFIRMED = "Typed, but not confirmed. Look at the terminal before sending it again.";

/** What a stop says under the field: done, refused, or pressed and not
 *  seen to land. `terminal` offers the way back, where it can be stopped by
 *  hand; `prompt` re-reads the prompt the refusal says is up. */
export function stopWords(err: string, agent: string): SendWords {
  if (err === AGENT_ERR.notBusy) return { line: `${agent} isn’t working on a turn, so there was nothing to stop.` };
  if (err === AGENT_ERR.promptUp) return { line: `${agent} is waiting on the prompt above, and stopping would cancel it. Answer it instead.`, prompt: true };
  if (err === AGENT_ERR.stopDraft) {
    return { line: `${agent}’s input has text in it, typed in the terminal, which hides whether it is working. Stop it in the terminal.`, terminal: true };
  }
  if (err === AGENT_ERR.noComposer) return { line: `${agent} isn’t at its prompt (starting up, or a screen this app doesn’t know). Stop it in the terminal.`, terminal: true };
  if (err.startsWith(AGENT_ERR.stopUnsupportedPrefix)) return { line: `${agent} can’t be stopped from here yet. Stop it in the terminal.`, terminal: true };
  if (err === AGENT_ERR.noAgent) return { line: "No agent is running in this place." };
  return { line: `Not stopped: ${err}`, terminal: true };
}

/** A stop that pressed Esc and could not see the turn end. Never retried. */
export const STOP_UNCONFIRMED = (agent: string) => `Esc was sent once, but ${agent} wasn’t seen to stop. Look at the terminal before stopping it again.`;

// ── the conversation ────────────────────────────────────────────────────────

/** A message `agent_send` confirmed, held until the transcript has it. */
export type Pending = { id: number; text: string; sentAt: number; queued: boolean };
/** An answer this window sent: the choice (a field's: its text), and the
 *  note that rode on it. `resumed`: a conversation picked in Claude's resume
 *  picker, with the picker's row about it (age · branch · size) — no
 *  transcript ever records that pick, so this line stays. */
export type Answered = { label: string; note?: string; at: number; confirmed: boolean; resumed?: string };

/** One tool call as the chat draws it: its name, what it did in a line
 *  (`turns::Step`'s `summary`: absent from an older host, for an MCP tool,
 *  and for a call whose line the host left out to stay in budget), and
 *  whether its result says it failed. Text only, never markup. */
export type ToolCall = { name: string; summary?: string; failed?: boolean };

export type Item =
  | { k: "sep"; key: string; at: number }
  | { k: "earlier"; key: string }
  | { k: "user"; key: string; text: string; at: number | null; midTurn?: boolean }
  | { k: "notice"; key: string; notice: Notice; at: number | null }
  | { k: "reply"; key: string; text: string; tools: string[]; calls: ToolCall[]; truncated: boolean; at: number | null; label: boolean }
  | { k: "asked"; key: string; step: AskedStep; label: boolean }
  | { k: "answer"; key: string; step: AskedStep }
  | { k: "answered"; key: string; a: Answered }
  | { k: "pending"; key: string; p: Pending; caughtUp: boolean };

const GAP_S = 5 * 60;
const sameDay = (a: number, b: number) => new Date(a * 1000).toDateString() === new Date(b * 1000).toDateString();

/** Turns oldest first, with a time separator at a gap of more than five
 *  minutes or a new day, this window's answers where they fall in time, and
 *  the messages the transcript has not shown yet at their send time. A turn with
 *  `steps` is drawn in their order: each reply block at its own time, the
 *  tools after the block they followed, and every question, permission and
 *  plan a person answered as what was asked and then the answer.
 *
 *  Keys name what an item IS (its turn's time, its step's time), never where
 *  it sits in the window: `agent_turns` answers the latest `TURNS_MAX`, so a
 *  new turn pushes the oldest off the top, and a key by position handed every
 *  row the next turn's content. React rewrote the whole list in place, and
 *  the browser's scroll anchoring held a row that now said something else
 *  (docs/proposals/chat-session-view.md §7). A second thing at the same
 *  second gets `~1`, `~2`. */
export function itemsOf(turns: Turn[], answered: Answered[], pending: Pending[], busy = false): Item[] {
  const out: Item[] = [];
  const taken = new Map<string, number>();
  const idOf = (base: string) => {
    const n = taken.get(base) ?? 0;
    taken.set(base, n + 1);
    return n ? `${base}~${n}` : base;
  };
  let prevAt: number | null = null;
  let lastWasAgent = false;
  const queue = [...answered];
  const flushAnswered = (beforeSec: number | null) => {
    while (queue.length && (beforeSec === null || queue[0].at / 1000 < beforeSec)) {
      const a = queue.shift()!;
      out.push({ k: "answered", key: `a${a.at}`, a });
      lastWasAgent = false;
    }
  };
  const sep = (at: number | null, key: string) => {
    if (at === null) return;
    if (prevAt === null || at - prevAt > GAP_S || !sameDay(at, prevAt)) out.push({ k: "sep", key, at });
    prevAt = at;
  };
  turns.forEach((t) => {
    const i = idOf(`${t.at ?? "-"}`);
    if (t.at !== null) flushAnswered(t.at);
    sep(t.at, `s${i}`);
    if (t.notice) {
      out.push({ k: "notice", key: `n${i}`, notice: t.notice, at: t.at });
      lastWasAgent = t.notice.kind === "peer_message";
    } else if (t.user === null) out.push({ k: "earlier", key: `e${i}` });
    else {
      out.push({ k: "user", key: `u${i}`, text: t.user, at: t.at });
      lastWasAgent = false;
    }
    if (!t.steps?.length) {
      if (t.reply || t.tools.length || t.truncated) {
        out.push({ k: "reply", key: `r${i}`, text: t.reply, tools: t.tools, calls: t.tools.map((name) => ({ name })), truncated: t.truncated, at: t.at, label: !lastWasAgent });
        lastWasAgent = true;
      }
      return;
    }
    let reply: Extract<Item, { k: "reply" }> | null = null;
    t.steps.forEach((st) => {
      // By the step's time, not its index: a turn the host's byte window
      // starts inside loses its first steps as the transcript grows.
      const key = idOf(`${i}.${st.at ?? "-"}`);
      if (st.kind === "text" || st.kind === "tool") {
        if (st.kind === "text" || !reply) {
          if (st.at !== null) { flushAnswered(st.at); sep(st.at, `s${key}`); }
          reply = { k: "reply", key: `r${key}`, text: st.kind === "text" ? st.text : "", tools: [], calls: [], truncated: false, at: st.at, label: !lastWasAgent };
          out.push(reply);
          lastWasAgent = true;
        }
        if (st.kind === "tool") {
          reply.tools.push(st.name);
          reply.calls.push({ name: st.name, ...(st.summary ? { summary: st.summary } : {}), ...(st.failed ? { failed: true } : {}) });
        }
        return;
      }
      if (st.kind === "user") {
        reply = null;
        if (st.at !== null) { flushAnswered(st.at); sep(st.at, `s${key}`); }
        out.push({ k: "user", key: `u${key}`, text: st.text, at: st.at, midTurn: true });
        lastWasAgent = false;
        return;
      }
      reply = null;
      // Not answered yet: the live card is where it is shown.
      if (st.answered_at === undefined) return;
      if (st.at !== null) sep(st.at, `s${key}`);
      out.push({ k: "asked", key: `q${key}`, step: st, label: !lastWasAgent });
      out.push({ k: "answer", key: `a${key}`, step: st });
      lastWasAgent = false;
    });
    if (t.truncated) {
      const last = [...out].reverse().find((it) => it.k === "reply") as Extract<Item, { k: "reply" }> | undefined;
      if (last) last.truncated = true;
    }
  });
  flushAnswered(null);
  // A pending fallback is still a message at its send time. Insert it before
  // the next timed item, including that item's day separator, so a transcript
  // that never records it cannot push it below hours of later conversation.
  const itemAt = (it: Item): number | null => {
    switch (it.k) {
      case "sep": case "user": case "reply": case "notice": return it.at;
      case "asked": return it.step.at;
      case "answer": return it.step.answered_at ?? it.step.at;
      case "answered": return it.a.at / 1000;
      case "pending": return it.p.sentAt / 1000;
      case "earlier": return null;
    }
  };
  for (const p of [...pending].sort((a, b) => a.sentAt - b.sentAt)) {
    // A later turn is known to have ended once another turn starts, or once
    // the agent is idle. A partial reply on a busy turn proves neither.
    const caughtUp = turns.some((t, i) => t.at !== null && t.at >= Math.floor(p.sentAt / 1000)
      && (i < turns.length - 1 || !busy));
    const at = p.sentAt / 1000;
    const next = out.findIndex((it) => { const time = itemAt(it); return time !== null && time > at; });
    out.splice(next < 0 ? out.length : next, 0, { k: "pending", key: `p${p.id}`, p, caughtUp });
  }
  return out;
}

/** This window's answers the transcript has not shown yet: newest first,
 *  each taking the newest answered step not already taken that was
 *  answered after it was sent (less the host's clock skew). Whatever
 *  answered a prompt, the transcript's record of it is the one shown. */
export function settleAnswered(answered: Answered[], turns: Turn[]): Answered[] {
  const times = turns.flatMap((t) => (t.steps ?? []).flatMap((s) => ("answered_at" in s && s.answered_at !== undefined ? [s.answered_at] : [])));
  times.sort((a, b) => b - a);
  const taken = new Set<number>();
  const keep: Answered[] = [];
  for (const a of [...answered].sort((x, y) => y.at - x.at)) {
    const after = Math.floor(a.at / 1000) - SKEW_S;
    const hit = times.findIndex((t, i) => !taken.has(i) && t >= after);
    if (hit >= 0) taken.add(hit);
    else keep.push(a);
  }
  return keep.sort((x, y) => x.at - y.at);
}

/** How a question was answered, in a line. */
export function questionAnswerWords(st: Extract<Step, { kind: "question" }>): string {
  const a = st.answer;
  if (!a) return "Answered";
  if (a.how === "chat") return "Chat about this";
  if (a.how === "declined") return "Declined to answer";
  const parts = (a.answers ?? []).map((x) => [...x.picked, ...(x.typed ? [x.typed] : [])].join(", "));
  return parts.filter(Boolean).join(" · ") || "Answered";
}

/** How a permission was answered, in a line. */
export function permissionAnswerWords(st: Extract<Step, { kind: "permission" }>): string {
  if (!st.allowed) return "No";
  return st.scope && st.scope !== "once" ? `Yes, for this ${st.scope === "session" ? "session" : st.scope}` : "Yes";
}

/** Whether a turn's calls are drawn as rows (a name and its line) rather
 *  than the bare name chips an older host's turns get: any call says what
 *  it did, or that it failed. */
export const callsSay = (calls: ToolCall[]) => calls.some((c) => !!c.summary || !!c.failed);

/** The rows a turn shows before "+N more". A fold never hides one row:
 *  up to one more than this, every call shows. */
export const CALLS_SHOWN = 3;

/** The calls shown, folded or not, and what the fold hides: how many, and
 *  how many of those failed, so a failure is never folded out of sight. */
export function foldCalls(calls: ToolCall[], open: boolean): { shown: ToolCall[]; hidden: number; hiddenFailed: number; folds: boolean } {
  const folds = calls.length > CALLS_SHOWN + 1;
  const shown = folds && !open ? calls.slice(0, CALLS_SHOWN) : calls;
  const rest = calls.slice(shown.length);
  return { shown, hidden: rest.length, hiddenFailed: rest.filter((c) => c.failed).length, folds };
}

/** A turn's calls in words, for its group's label: "Used Bash, Read and
 *  Edit." and, when any failed, how many. */
export function callsWords(calls: ToolCall[]): string {
  const names = [...new Set(calls.map((c) => c.name))];
  if (!names.length) return "";
  const used = names.length === 1 ? `Used ${names[0]}.` : `Used ${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}.`;
  const failed = calls.filter((c) => c.failed).length;
  return failed ? `${used} ${failed === 1 ? "One call" : `${failed} calls`} failed.` : used;
}

/** `text` with the place's root taken off each path that starts with it:
 *  at the start, as the permission card shows its path, or after a space,
 *  a quote, `=`, `(`, `,` or `;`, where a command, `<pattern> in <path>` or
 *  a patch's file list puts one. The title keeps the line as sent. */
export function underRoot(text: string, root: string): string {
  const r = root.replace(/\/+$/, "");
  if (!r) return text;
  const esc = `${r}/`.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return text.replace(new RegExp(`(^|[\\s"'=(,;])${esc}`, "g"), "$1");
}

/** How far the host's clock may run behind when a pending message is matched. */
const SKEW_S = 120;

/** Drop the pending messages the transcript now holds: newest first, each
 *  taking the newest unmatched normal user turn or absorbed mid-turn message
 *  with its text written after the send began. One record settles one copy. */
export function settlePending(pending: Pending[], turns: Turn[]): Pending[] {
  const messages = turns.flatMap((t) => [
    ...(t.user === null ? [] : [{ text: t.user, at: t.at }]),
    ...(t.steps ?? []).flatMap((s) => s.kind === "user" ? [{ text: s.text, at: s.at }] : []),
  ]);
  const taken = new Set<number>();
  const keep: Pending[] = [];
  for (const p of [...pending].sort((a, b) => b.sentAt - a.sentAt)) {
    const after = Math.floor(p.sentAt / 1000) - SKEW_S;
    let hit = -1;
    for (let i = messages.length - 1; i >= 0; i--) {
      const m = messages[i];
      if (!taken.has(i) && m.text === p.text && (m.at ?? 0) >= after) { hit = i; break; }
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

/** A prompt shown beside its `detail`: the rows from the last rule down (the
 *  question itself), and the clipped part above it, which the detail shows
 *  whole. No rule, and every row is the question. */
export function splitAtRule(rows: string[]): { above: string[]; question: string[] } {
  let at = -1;
  rows.forEach((l, i) => { if (/^\s*[╌─━▔┄]{8,}\s*$/.test(l)) at = i; });
  const question = rows.slice(at + 1);
  return at < 0 || !question.some((l) => l.trim()) ? { above: [], question: rows } : { above: rows.slice(0, at + 1), question };
}

/** How a choice is answered from this window, and whether it can be:
 *  - `plain`: `agent_answer`. A field someone has typed into reads as its
 *    text, and is sent as it reads.
 *  - `field`: an empty text field ("Type something."): `agent_answer_text`,
 *    and only with text, because the empty choice answers nothing.
 *  - `amend`: plain, or with a note through `agent_answer_text`.
 *  The console may make a `widens` choice (agents.rs lets `is_console()`
 *  through); a window driving a paired host may not. A field the reader
 *  cannot fill (`host_only: text` and no `text`, kimi's) is the terminal's. */
export type ChoiceWay = { open: boolean; how: "plain" | "field" | "amend"; note?: string };
export function choiceOpen(c: PromptChoice): ChoiceWay {
  if (c.host_only === "text" && c.text !== "field") return { open: false, how: "plain", note: "Opens a text field: answer this one in the terminal" };
  const how = c.text === "field" && c.host_only === "text" ? "field" : c.text === "amend" ? "amend" : "plain";
  if (c.host_only === "widens") {
    return windowOnHost()
      ? { open: true, how, note: "Changes what the agent may do from now on" }
      : { open: false, how, note: "Answer this one on the host" };
  }
  if (c.text === "field" && how === "plain") return { open: true, how, note: "Typed in the terminal: sent as it reads" };
  return { open: true, how };
}

/** `choiceOpen`, in its prompt: a `previews` question's ONE notes field
 *  decides for every option. Open in the terminal, someone is typing there
 *  and the rows refuse every answer; holding their note, it goes with
 *  whichever choice is sent, and no other note can be added. */
export function choiceWay(p: ReadablePrompt, c: PromptChoice): ChoiceWay {
  if (c.checked !== undefined) return { open: false, how: "plain", note: "Tick this box, then send the choices together" };
  const way = choiceOpen(c);
  if (!p.notes) return way;
  if (p.notes.open) return { open: false, how: "plain", note: "Someone is typing a note in the terminal: answer it there" };
  return p.notes.text && way.how === "amend" ? { ...way, how: "plain" } : way;
}

/** The preview a `previews` card shows, and for which choice: the selected
 *  option's when every one is known (none when it has none), else the
 *  highlighted option's, else the first's; with the screen's alone, the one
 *  the terminal shows, whichever is selected. */
export function previewOf(p: ReadablePrompt, selected: number | null): { c: PromptChoice; known: "all" | "shown" } | null {
  if (!p.previews) return null;
  const options = p.choices.filter((c) => c.text === "amend" || c.preview !== undefined);
  if (p.previews === "shown") {
    const c = options.find((x) => x.preview !== undefined);
    return c ? { c, known: "shown" } : null;
  }
  const c = options.find((x) => x.n === selected) ?? (selected === null ? options.find((x) => x.highlighted) ?? options[0] : undefined);
  return c ? { c, known: "all" } : null;
}

/** What a text field or a note on choice `c` asks for, as a placeholder. */
export function fieldWords(c: PromptChoice, harness: string, agent: string): string {
  if (c.text === "amend") {
    if (harness !== "claude") return "Add a note (optional)";
    if (/^no\b/i.test(c.label)) return `No, and tell ${agent} what to do differently (optional)`;
    if (/^yes\b/i.test(c.label)) return `Yes, and tell ${agent} what to do next (optional)`;
    return "Add a note (optional)";
  }
  return /^type something\.?$/i.test(c.label.trim()) ? "Type your answer" : c.label.replace(/\.$/, "");
}

/** Codex's approval "No, and tell Codex what to do differently" opens no
 *  field: it ends the turn, and the line goes through the composer. */
export const tellsAfter = (c: PromptChoice) => !c.text && /\btell \w+ what to do differently\b/i.test(c.label);

/** The boxes of a multi-select, and its free-text box when that is still
 *  empty (filled, it is a box like the rest, its label the text). */
export function boxesOf(p: ReadablePrompt): { boxes: PromptChoice[]; freeBox: PromptChoice | null } {
  const boxes = p.choices.filter((c) => c.checked !== undefined);
  return { boxes, freeBox: boxes.find((c) => c.text === "field" && c.host_only === "text") ?? null };
}

// ── the card's keys ─────────────────────────────────────────────────────────
// A digit does what a click on that choice does, and Enter is the card's Send
// button: two keys, as two clicks, because an answer can run a command. Claude's
// own terminal answers on the digit alone; this card never does.

/** Where the keyboard is, as the card's keys see it:
 *  - `composer`: the chat's own message field. A text field like any other,
 *    named so that an EMPTY one can hand the keyboard to a new prompt;
 *  - `editable`: any other text input, textarea, select or contenteditable;
 *  - `xterm`: a terminal's helper textarea (the dock shell): its digits are
 *    the shell's;
 *  - `card`: the prompt card itself, or one of its choices or boxes, where a
 *    native Enter would click the focused choice AGAIN (Chromium focuses a
 *    clicked button; WebKit focuses the card around it);
 *  - `control`: any other button, link or widget, whose Enter is its own;
 *  - `none`: nothing that takes keys (the body, text). */
export type KeyFocus = "composer" | "editable" | "xterm" | "card" | "control" | "none";

const NOT_TEXT = new Set(["button", "checkbox", "color", "file", "image", "radio", "range", "reset", "submit"]);
const TEXT_ROLES = new Set(["textbox", "searchbox", "combobox", "spinbutton"]);
const CONTROL_ROLES = new Set(["button", "link", "checkbox", "radio", "switch", "tab", "menuitem", "menuitemcheckbox", "menuitemradio", "option", "treeitem", "slider", "separator", "gridcell", "row"]);

/** What `el` (the focused element, read through any shadow root) is to the
 *  card's keys. `card` is the card's root element. */
export function keyFocusOf(el: Element | null, card: Element | null): KeyFocus {
  if (!el || el.tagName === "BODY" || el.tagName === "HTML") return "none";
  if (el.classList.contains("xterm-helper-textarea") || el.closest(".xterm")) return "xterm";
  if (el.classList.contains("chat-field")) return "composer";
  const tag = el.tagName;
  const type = tag === "INPUT" ? (el.getAttribute("type") ?? "text").toLowerCase() : "";
  const role = el.getAttribute("role") ?? "";
  if (tag === "TEXTAREA" || tag === "SELECT" || (tag === "INPUT" && !NOT_TEXT.has(type)) || TEXT_ROLES.has(role) || (el as HTMLElement).isContentEditable) return "editable";
  if (card && (el === card || (card.contains(el) && (el.classList.contains("chat-choice") || (type === "checkbox" && !!el.closest(".chat-box")))))) return "card";
  if (tag === "BUTTON" || tag === "SUMMARY" || tag === "INPUT" || (tag === "A" && el.hasAttribute("href")) || CONTROL_ROLES.has(role)) return "control";
  if (el.hasAttribute("tabindex") && Number(el.getAttribute("tabindex")) >= 0) return "control";
  return "none";
}

/** How long after a card takes the keyboard on its own every key stays the
 *  composer's: a key pressed that soon was aimed before the card was seen
 *  (Firefox holds its security dialogs' buttons the same way, for 1 s). */
export const CARD_GRACE_MS = 1000;

/** The parts of a `KeyboardEvent` the card's keys read. */
export type CardKeyEvent = { key: string; metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; isComposing: boolean; repeat: boolean };
export type CardKeyState = {
  focus: KeyFocus;
  /** ms since this card took the keyboard from the composer on its own
   *  (`cardTakesFocus`); null when it did not. */
  sinceTook: number | null;
  /** `escapeDepth()`: a modal, menu, sheet or popover is up. */
  escapeDepth: number;
  /** The card is on screen: mounted, laid out, not inert, not under the reader. */
  shown: boolean;
  /** `ChatView.answering`. */
  answering: boolean;
  /** `ChatView.selected`. */
  selected: number | null;
  /** The card's Send button is there and enabled. */
  canSend: boolean;
};
/** What a key does on the card:
 *  - `select`: `selectChoice(path, choice)`, as a click on choice `n` does
 *    (a click on the selected choice unselects it, so `choice` is null), and
 *    `field` focuses the text the newly selected choice takes;
 *  - `tick`: `toggleTick(path, n)`, as a click on box `n` does;
 *  - `send`: what the Send button does;
 *  - `hold`: an Enter with nothing to send, kept from clicking a focused choice;
 *  - `composer`: the person was typing a message: the composer gets the
 *    keyboard back, and the key goes on to it, unprevented. */
export type CardKeyAct =
  | { act: "select"; n: number; choice: number | null; field: boolean }
  | { act: "tick"; n: number }
  | { act: "send" }
  | { act: "hold" }
  | { act: "composer" };

/** The card's answer to a key, or null when the key is not the card's (and
 *  must not be prevented). Never while a modal is up, a modifier is held, an
 *  IME is composing, or the keyboard is in a text field or a terminal; no
 *  answer while one is in flight. Enter only from the card or from nowhere:
 *  another control's Enter is its own.
 *
 *  Typing at the card is typing a message, so the composer gets it: a letter
 *  (or any key in the first `CARD_GRACE_MS` after the card took the keyboard
 *  on its own). Without that, "2 things⏎" begun in an empty composer as a
 *  prompt arrived would select choice 2, drop the words, and SEND it. */
export function cardKey(e: CardKeyEvent, s: CardKeyState, p: ReadablePrompt): CardKeyAct | null {
  if (e.metaKey || e.ctrlKey || e.altKey || e.isComposing || e.repeat) return null;
  if (!s.shown || s.escapeDepth > 0) return null;
  if (s.focus === "composer" || s.focus === "editable" || s.focus === "xterm") return null;
  if (s.focus === "card") {
    const typed = e.key.length === 1 && e.key !== " ";
    if (s.sinceTook !== null && s.sinceTook < CARD_GRACE_MS && (typed || e.key === "Enter")) return { act: "composer" };
    if (typed && !/^[0-9]$/.test(e.key)) return { act: "composer" };
  }
  if (e.shiftKey || s.answering) return null;
  if (e.key === "Enter") {
    if (s.focus !== "card" && s.focus !== "none") return null;
    return s.canSend ? { act: "send" } : { act: "hold" };
  }
  if (!/^[1-9]$/.test(e.key)) return null;
  const n = Number(e.key);
  const c = p.choices.find((x) => x.n === n);
  // A choice the card draws disabled is not there for a key either.
  if (!c) return null;
  if (p.multi && c.checked !== undefined) return choiceOpen(c).open ? { act: "tick", n } : null;
  if (!choiceWay(p, c).open) return null;
  const on = s.selected !== n;
  return { act: "select", n, choice: on ? n : null, field: on && choiceWay(p, c).how !== "plain" };
}

/** Whether a card takes the keyboard as it appears: from the chat's own
 *  composer while it is EMPTY, never from anywhere else (the dock shell, a
 *  find field, a sheet), and once per prompt, so a person who clicks back into
 *  the composer is not pulled out again by the next read of the same prompt.
 *  The composer cannot send while a prompt waits, so an empty one loses
 *  nothing; one holding text keeps the keyboard. */
export function cardTakesFocus(a: { fingerprint: string | null; seen: string | null; focus: KeyFocus; draft: string; escapeDepth: number; shown: boolean }): boolean {
  return !!a.fingerprint && a.fingerprint !== a.seen && a.focus === "composer" && a.draft === "" && a.escapeDepth === 0 && a.shown;
}

/** The digits that choose on this card, as its hint says them: "1–3", "1", or
 *  null when no choice is open. */
export function cardDigits(p: ReadablePrompt): string | null {
  const ns = p.choices.filter((c) => c.n >= 1 && c.n <= 9 && (p.multi && c.checked !== undefined ? choiceOpen(c).open : choiceWay(p, c).open)).map((c) => c.n);
  if (!ns.length) return null;
  const lo = Math.min(...ns);
  const hi = Math.max(...ns);
  return lo === hi ? String(lo) : `${lo}–${hi}`;
}

/** One row of a diff between two texts. */
export type DiffLine = { k: " " | "-" | "+"; text: string };
/** The most cells the line LCS may fill before it gives up on alignment. */
const DIFF_BUDGET = 400_000;

/** `old` → `new` as rows: the shared head and tail as context, the middle
 *  aligned by a line LCS, or (past the budget) all of `old` out and all of
 *  `new` in, which is still the whole of both. An Edit's `old_string` and
 *  `new_string` are fragments, so every row of both is shown. */
export function lineDiff(a: string, b: string): DiffLine[] {
  const A = a.split("\n");
  const B = b.split("\n");
  let lo = 0;
  while (lo < A.length && lo < B.length && A[lo] === B[lo]) lo++;
  let ea = A.length;
  let eb = B.length;
  while (ea > lo && eb > lo && A[ea - 1] === B[eb - 1]) { ea--; eb--; }
  const out: DiffLine[] = A.slice(0, lo).map((text) => ({ k: " ", text }));
  const mA = A.slice(lo, ea);
  const mB = B.slice(lo, eb);
  if ((mA.length + 1) * (mB.length + 1) > DIFF_BUDGET) {
    out.push(...mA.map((text) => ({ k: "-" as const, text })), ...mB.map((text) => ({ k: "+" as const, text })));
  } else {
    const m = mB.length + 1;
    const dp = new Int32Array((mA.length + 1) * m);
    for (let i = mA.length - 1; i >= 0; i--) {
      for (let j = mB.length - 1; j >= 0; j--) {
        dp[i * m + j] = mA[i] === mB[j] ? dp[(i + 1) * m + j + 1] + 1 : Math.max(dp[(i + 1) * m + j], dp[i * m + j + 1]);
      }
    }
    let i = 0;
    let j = 0;
    while (i < mA.length || j < mB.length) {
      if (i < mA.length && j < mB.length && mA[i] === mB[j]) { out.push({ k: " ", text: mA[i] }); i++; j++; }
      else if (j < mB.length && (i === mA.length || dp[i * m + j + 1] >= dp[(i + 1) * m + j])) { out.push({ k: "+", text: mB[j] }); j++; }
      else { out.push({ k: "-", text: mA[i] }); i++; }
    }
    // Within a change, removals before additions, as git shows them.
    for (let s = 0; s < out.length; s++) {
      if (out[s].k === " ") continue;
      let e = s;
      while (e < out.length && out[e].k !== " ") e++;
      const run = out.slice(s, e);
      out.splice(s, e - s, ...run.filter((r) => r.k === "-"), ...run.filter((r) => r.k === "+"));
      s = e;
    }
  }
  out.push(...A.slice(ea).map((text) => ({ k: " " as const, text })));
  return out;
}

// ── what a screen reader is told (VoiceOver) ────────────────────────────────
//
// The iOS app's View session announces the agent's new replies politely,
// never by moving focus ("1 new message", `ViewSession.tsx`). This chat does
// the same, and also says when the agent starts waiting on a prompt, in the
// iOS status's spoken words ("Claude needs input"). Never announced: what was
// there when the chat was shown, the person's own sends and answers and the
// transcript's echo of them, tool calls, and a re-read that changes nothing.
// The reply's own text is never said: the conversation is one move away.

/** How the card tells one prompt from the next: a readable one by its
 *  fingerprint, an unreadable one (which has none) by the rows it shows. */
export function promptKeyOf(p: AgentPrompt | null): string | null {
  if (!p || !p.waiting) return null;
  return p.readable ? p.fingerprint : `unreadable|${(p.lines ?? []).join("\n")}`;
}

/** What the chat holds now, as far as a screen reader is concerned. */
export type ChatNow = {
  items: Item[];
  /** When the turns shown were read (`turns.at`), null before any. */
  turnsAt: number | null;
  /** The prompt read (`prompt.data`), and when (`prompt.at`). */
  prompt: AgentPrompt | null;
  promptAt: number | null;
  /** An answer of this window's is in flight: a prompt that moves under it
   *  (a multi-select's box filled) is the person's own doing. */
  answering: boolean;
};
/** What has been accounted for: each reply by its time and words, counted (two
 *  alike are two), and the prompt's key. A part is null until a read that
 *  landed after the chat was shown: what the store held before is history. */
export type Heard = { replies: Map<string, number> | null; prompt: { key: string | null } | null };
/** What is new since the last `Heard`. */
export type News = { replies: number; prompt: { key: string; readable: boolean } | null };

/** The agent's replies that say something: a tool call drawn on its own (a
 *  reply item with no words) is a chip, never a message. */
function repliesOf(items: Item[]): Map<string, number> {
  const m = new Map<string, number>();
  for (const it of items) {
    let id: string;
    if (it.k === "reply" && it.text.trim()) id = `${it.at ?? ""}\n${it.text}`;
    else if (it.k === "notice" && it.notice.kind === "peer_message") id = `peer\n${it.at ?? ""}\n${it.notice.from_name}\n${it.notice.text}`;
    else continue;
    m.set(id, (m.get(id) ?? 0) + 1);
  }
  return m;
}

/** The decision: given what was heard, the chat now, when it was shown, and
 *  whether the window is visible, what is new (null for nothing) and what is
 *  heard from here. Replies are compared as a count per reply, so the window
 *  sliding past its oldest turn, a pending message settling into the
 *  transcript, an answer's echo, a tool joining a reply or the same transcript
 *  read again add nothing; a prompt is news when its key differs from the last
 *  one read, as the card scrolls to it, unless an answer from here is in
 *  flight. Hidden, nothing is news, and what was read is heard all the same,
 *  so nothing is said for it later. */
export function hear(prev: Heard | null, now: ChatNow, shownAt: number, visible: boolean): { heard: Heard; news: News | null } {
  const turnsFresh = now.turnsAt !== null && now.turnsAt >= shownAt;
  const promptFresh = now.promptAt !== null && now.promptAt >= shownAt;
  const replies = turnsFresh ? repliesOf(now.items) : null;
  const key = promptKeyOf(now.prompt);
  const heard: Heard = { replies, prompt: promptFresh ? { key } : null };
  let fresh = 0;
  if (replies && prev?.replies) {
    for (const [id, n] of replies) fresh += Math.max(0, n - (prev.replies.get(id) ?? 0));
  }
  const p = promptFresh && prev?.prompt && key !== null && key !== prev.prompt.key && !now.answering && now.prompt?.waiting
    ? { key, readable: now.prompt.readable }
    : null;
  if (!visible || (!fresh && !p)) return { heard, news: null };
  return { heard, news: { replies: fresh, prompt: p } };
}

/** News inside this long of the first is said once, when it ends. A change
 *  of the place's state reads the turns and the prompt together, and the two
 *  land apart (a reply written just before a prompt, then the prompt); a send
 *  or an answer reads both again. 1.5 s takes those in, slow reads over a
 *  paired host included, and stays under the host's 3 s activity poll, so
 *  news from two of its ticks is two announcements. The window opens with the
 *  first news and never slides, so a steady trickle cannot hold it open. */
export const ANNOUNCE_MS = 1500;

/** News waiting to be said, due when its window ends. */
export type Bundle = { due: number; replies: number; prompt: { key: string; readable: boolean } | null };

/** Add news to the bundle (a new one opens at `now`). */
export function gather(b: Bundle | null, news: News, now: number): Bundle {
  return { due: b ? b.due : now + ANNOUNCE_MS, replies: (b?.replies ?? 0) + news.replies, prompt: news.prompt ?? b?.prompt ?? null };
}

/** What is said when a bundle falls due: the iOS app's words for replies, and
 *  for a prompt its spoken status, only if that prompt is still the one up
 *  (`promptNow`). Null for nothing, and always when the window is hidden. */
export function announcement(b: Bundle, agent: string, promptNow: string | null, visible: boolean): string | null {
  if (!visible) return null;
  const parts: string[] = [];
  if (b.replies > 0) parts.push(b.replies === 1 ? "1 new message" : `${b.replies} new messages`);
  if (b.prompt && b.prompt.key === promptNow) parts.push(b.prompt.readable ? `${agent} needs input` : `${agent} needs input in the terminal`);
  return parts.length ? parts.join(". ") : null;
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
  /** What the composer asks for next: Codex's "No, and tell Codex what to
   *  do differently" ends the turn and the line is a message. */
  followUp: string | null;
  /** The selected choice: an answer takes a click on it, then a second on
   *  "Send answer", because an answer can run a command. */
  selected: number | null;
  /** The selected choice's text: a field's answer, or the note on an amend. */
  answerText: string;
  /** A multi-select's ticks; null while they are the prompt's own. */
  ticks: number[] | null;
  /** A multi-select's free-text box. */
  boxText: string;
  answering: boolean;
  answerWords: Words | null;
  /** A stop in flight, and what the last one said. */
  stopping: boolean;
  stopWords: SendWords | null;
  /** A stop was SEEN to land: the turn is over, whatever the activity push
   *  (a poll behind) still says, until it next moves. */
  stopped: boolean;
  /** The files the next message carries, in order. */
  attachments: Attachment[];
  /** What the last refused upload or pick said. */
  attachWords: SendWords | null;
  /** Words when this host has no `agent_attach` (an older one): the
   *  paperclip is off, with them as its reason. The host's, so every place's. */
  attachOff: string | null;
};

/** Set once a host answers `unknown command: agent_attach`. */
let attachOff: string | null = null;
const blank = (): ChatView => ({
  turns: { data: null, error: null, loading: false, at: null },
  prompt: { data: null, error: null, loading: false, at: null },
  pending: [], answered: [], draft: "", sending: false, sendWords: null, followUp: null,
  selected: null, answerText: "", ticks: null, boxText: "", answering: false, answerWords: null,
  stopping: false, stopWords: null, stopped: false,
  attachments: [], attachWords: null, attachOff,
});
const EMPTY = blank();
let views = new Map<string, ChatView>();
const subs = new Set<() => void>();
/** EMPTY once the host is known to lack `agent_attach`: one object, as
 *  `useSyncExternalStore` needs the same snapshot until something changes. */
let emptyOff: ChatView | null = null;
const viewOf = (path: string) => views.get(path) ?? (attachOff ? (emptyOff ??= { ...EMPTY, attachOff }) : EMPTY);
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
      put(t.path, {
        turns: { data: r, error: null, loading: false, at: Date.now() },
        pending: settlePending(cur.pending, r.turns), answered: settleAnswered(cur.answered, r.turns),
      });
    } catch (e) {
      put(t.path, { turns: { ...viewOf(t.path).turns, loading: false, error: String(e) } });
    }
  });
}

export const turnsUnsupported = (err: string | null) => !!err && err.startsWith(AGENT_ERR.turnsUnsupportedPrefix);
/** `agent_turns` failed only because the place's Claude has no session yet. */
export const noSessionYet = (err: string | null) => !!err && err.endsWith(AGENT_ERR.noSessionYet);
/** After a conversation is picked in the resume picker, Claude writes its
 *  session file a moment later (measured: under a second, and not by the read
 *  straight after the answer), so the turns are read again on these delays
 *  while they still fail that way. */
export const RESUME_REREAD_MS = [500, 1500, 3500] as const;

export const CHANGED = "Not sent: the prompt changed before the answer reached it. Someone may have answered it in the terminal.";

const fpOf = (p: AgentPrompt | null) => (p && p.waiting && p.readable ? p.fingerprint : null);
/** The card's choices for one prompt: a choice, its text, the ticks. */
const NO_CHOICE = { selected: null, answerText: "", ticks: null, boxText: "" } as const;

/** The place's prompt. One that is not the prompt a choice was made on
 *  clears the choice, and says so; a choice never carries over. Not while an
 *  answer is in flight: filling a multi-select's box moves its fingerprint,
 *  and that answer reads the prompt again itself. */
export function loadPrompt(t: Target): Promise<void> {
  return once(`p|${t.path}`, async () => {
    put(t.path, { prompt: { ...viewOf(t.path).prompt, loading: true } });
    try {
      const r = await invoke<AgentPrompt>("agent_prompt", { repo: t.repo, slug: t.slug });
      const cur = viewOf(t.path);
      const chose = cur.selected !== null || cur.ticks !== null || !!cur.answerText || !!cur.boxText;
      const moved = !cur.answering && chose && fpOf(cur.prompt.data) !== fpOf(r);
      put(t.path, {
        prompt: { data: r, error: null, loading: false, at: Date.now() },
        ...(moved ? { ...NO_CHOICE, answerWords: cur.answerWords ?? { line: CHANGED } } : {}),
      });
    } catch (e) {
      put(t.path, { prompt: { ...viewOf(t.path).prompt, loading: false, error: String(e) } });
    }
  });
}

export function setDraft(path: string, draft: string): void {
  put(path, { draft, sendWords: null, stopWords: null });
}

let nextAttachment = 1;

/** Add picked, pasted or dropped files to the next message. All three paths
 *  go through the same byte sniff and photo preparation before Send can run. */
export function addAttachments(path: string, files: Blob[]): void {
  const v = viewOf(path);
  if (v.sending || v.attachOff || !files.length) return;
  const nameOf = (f: Blob) => ("name" in f && typeof f.name === "string" && f.name) || (f.type.startsWith("image/") ? "pasted image" : "pasted file");
  const add: Attachment[] = [];
  const big: string[] = [];
  const empty: string[] = [];
  let over = 0;
  for (const f of files) {
    const name = nameOf(f);
    if (f.size > INPUT_MAX) big.push(name);
    else if (f.size === 0) empty.push(name);
    else if (v.attachments.length + add.length >= ATTACH_PER_MESSAGE) over++;
    else add.push({ id: nextAttachment++, name, size: f.size, file: f, state: "preparing" });
  }
  const said: string[] = [];
  const quote = (ns: string[]) => ns.map((n) => `“${n}”`).join(", ");
  if (big.length) said.push(`${quote(big)} ${big.length === 1 ? "is" : "are"} over the ${sizeWords(INPUT_MAX)} input limit.`);
  if (empty.length) said.push(`${quote(empty)} ${empty.length === 1 ? "is" : "are"} empty.`);
  if (over) said.push(`At most ${ATTACH_PER_MESSAGE} files in one message: ${over} left out.`);
  put(path, {
    attachments: [...v.attachments, ...add], sendWords: null,
    attachWords: said.length ? { line: `Not attached: ${said.join(" ")}` } : null,
  });
  for (const a of add) void prepareAttachment(path, a);
}

async function prepareAttachment(path: string, a: Attachment): Promise<void> {
  try {
    const header = new Uint8Array(await a.file.slice(0, 12).arrayBuffer());
    const kind = photoKind(header);
    const decision = photoDecision(kind, a.file.size, ATTACH_MAX);
    if (decision === "input-large") throw new Error(`is over the ${sizeWords(INPUT_MAX)} input limit.`);
    if (decision === "upload-large") throw new Error(`is over the ${sizeWords(ATTACH_MAX)} attachment limit.`);
    if (decision === "convert" && kind) {
      const jpeg = await convertPhoto(a.file, ATTACH_MAX, kind);
      setAttachment(path, a.id, { name: jpegName(a.name), size: jpeg.size, file: jpeg, converted: true, state: "ready" });
    } else {
      setAttachment(path, a.id, { state: "ready" });
    }
  } catch (error) {
    const v = viewOf(path);
    if (!v.attachments.some((item) => item.id === a.id)) return;
    const detail = String(error).replace(/^Error: /, "");
    put(path, {
      attachments: v.attachments.filter((item) => item.id !== a.id),
      attachWords: { line: `Not attached: “${a.name}”${detail.startsWith("is ") ? " " : ": "}${detail}` },
    });
  }
}

/** Take a file off the next message. Not while it is being sent. */
export function removeAttachment(path: string, id: number): void {
  const v = viewOf(path);
  if (v.sending) return;
  put(path, { attachments: v.attachments.filter((a) => a.id !== id), attachWords: null });
}

function setAttachment(path: string, id: number, patch: Partial<Attachment>): void {
  put(path, { attachments: viewOf(path).attachments.map((a) => (a.id === id ? { ...a, ...patch } : a)) });
}

/** A file's bytes as base64, read in 32 KiB slices (`fromCharCode` takes
 *  them as arguments). */
async function base64Of(f: Blob): Promise<string> {
  const b = new Uint8Array(await f.arrayBuffer());
  let s = "";
  for (let i = 0; i < b.length; i += 0x8000) s += String.fromCharCode(...b.subarray(i, i + 0x8000));
  return btoa(s);
}

/** Upload each file the host does not have yet, in order. The paths, or
 *  null when one was refused: then nothing is sent, and the chips say which. */
async function uploadAll(t: Target): Promise<string[] | null> {
  const paths: string[] = [];
  for (const a of viewOf(t.path).attachments) {
    if (a.path) { paths.push(a.path); continue; }
    setAttachment(t.path, a.id, { state: "uploading" });
    try {
      const r = await invoke<AgentAttached>("agent_attach", { repo: t.repo, slug: t.slug, name: a.name, data: await base64Of(a.file) });
      setAttachment(t.path, a.id, { state: "attached", path: r.path });
      paths.push(r.path);
    } catch (e) {
      setAttachment(t.path, a.id, { state: "failed" });
      const { unsupportedFeature } = await import("./ipcws");
      const old = unsupportedFeature(e, "attachments");
      if (old) {
        attachOff = `${old}: update Worktrees on it to attach files`;
        views = new Map([...views].map(([k, w]) => [k, { ...w, attachOff }]));
      }
      put(t.path, { sending: false, attachWords: old ? { line: `${old}. Nothing was sent.` } : attachWords(String(e), a.name) });
      return null;
    }
  }
  return paths;
}

/** The place's activity moved: a stop's "seen to land" has served its turn,
 *  and once a new turn starts, so has "Stopped.". */
export function activityMoved(path: string, busy: boolean): void {
  const v = viewOf(path);
  const done = busy && !!v.stopWords && !v.stopWords.terminal && !v.stopWords.prompt;
  if (v.stopped || done) put(path, { stopped: false, ...(done ? { stopWords: null } : {}) });
}

/** Stop the running turn: ONE `agent_interrupt`, never repeated by itself.
 *  A host from before the row answers `unknown command`, and the words say
 *  this host can't stop a turn (`unsupportedFeature`). */
export async function stopTurn(t: Target, agent: string): Promise<void> {
  if (viewOf(t.path).stopping) return;
  put(t.path, { stopping: true, stopWords: null, sendWords: null });
  try {
    const r = await invoke<AgentStopped>("agent_interrupt", { repo: t.repo, slug: t.slug });
    put(t.path, {
      stopping: false, stopped: r.interrupted,
      stopWords: r.interrupted ? { line: `Stopped. ${agent} is waiting for your next message.` } : { line: STOP_UNCONFIRMED(agent), terminal: true },
    });
  } catch (e) {
    // ipcws.ts is never imported statically (main.tsx's dynamic imports
    // decide what a build carries); destructured, so the build tree-shakes it.
    const { unsupportedFeature } = await import("./ipcws");
    const old = unsupportedFeature(e, "stopping a turn");
    const w = old ? { line: `${old}. Stop it in the terminal.`, terminal: true } : stopWords(String(e), agent);
    put(t.path, { stopping: false, stopWords: w });
    if (w.prompt) void loadPrompt(t);
  }
  void loadTurns(t);
}

let nextPending = 1;

/** Send the draft, and its attachments: each file is uploaded first, in
 *  order, and only once every one is kept is the line sent, naming them
 *  (`attachLine`). A refused upload sends nothing. The draft and the files
 *  clear only on a confirmed delivery. */
export async function sendDraft(t: Target, agent: string): Promise<void> {
  const v = viewOf(t.path);
  const draft = v.draft;
  const n = v.attachments.length;
  if (v.sending || v.attachments.some((a) => a.state === "preparing") || (!draft.trim() && !n) || attachProblem(draft, n)) return;
  put(t.path, { sending: true, sendWords: null, stopWords: null, attachWords: null });
  const paths = n ? await uploadAll(t) : [];
  if (!paths) return;
  const text = attachLine(draft, paths);
  const sent = new Set(viewOf(t.path).attachments.map((a) => a.id));
  try {
    // Claude stamps the transcript before agent_send's confirmation returns.
    const sentAt = Date.now();
    const r = await invoke<AgentSent>("agent_send", { repo: t.repo, slug: t.slug, session: t.session, text });
    const cur = viewOf(t.path);
    let sentId: number | null = null;
    if (!r.delivered) {
      put(t.path, { sending: false, sendWords: { line: UNCONFIRMED, terminal: true } });
    } else {
      sentId = nextPending++;
      put(t.path, {
        sending: false, followUp: null,
        // Only if nothing new was typed meanwhile.
        draft: cur.draft === draft ? "" : cur.draft,
        attachments: cur.attachments.filter((a) => !sent.has(a.id)),
        pending: [...cur.pending, { id: sentId, text, sentAt, queued: r.queued }],
      });
    }
    // This call can join a read started before the send. If that response
    // still leaves this message pending, start one fresh read after the
    // joined flight ends; an idle place will not get a busy poll to do it.
    void loadTurns(t).then(() => {
      if (sentId !== null && viewOf(t.path).pending.some((p) => p.id === sentId)) return loadTurns(t);
    });
  } catch (e) {
    const w = sendWords(String(e), agent);
    put(t.path, { sending: false, sendWords: w });
    if (w.prompt) void loadPrompt(t);
  }
}

export function selectChoice(path: string, n: number | null): void {
  const v = viewOf(path);
  if (v.answering) return;
  put(path, { selected: n, answerText: n === v.selected ? v.answerText : "", answerWords: null });
}

export function setAnswerText(path: string, text: string): void {
  put(path, { answerText: text, answerWords: null });
}

/** The boxes ticked now: the person's, else the prompt's own. */
export function ticksOf(v: ChatView, p: ReadablePrompt): number[] {
  return v.ticks ?? p.choices.filter((c) => c.checked).map((c) => c.n);
}

/** Tick or untick box `n`. A box is the first act, as a selection is. */
export function toggleTick(path: string, n: number): void {
  const v = viewOf(path);
  const d = v.prompt.data;
  if (v.answering || !d || !d.waiting || !d.readable || !d.multi) return;
  const on = new Set(ticksOf(v, d));
  if (on.has(n)) on.delete(n);
  else on.add(n);
  put(path, { ticks: [...on].sort((a, b) => a - b), selected: null, answerText: "", answerWords: null });
}

/** The free-text box's answer. Typing into it ticks it, as Claude does. */
export function setBoxText(path: string, text: string): void {
  const v = viewOf(path);
  const d = v.prompt.data;
  if (!d || !d.waiting || !d.readable || !d.multi) return;
  const { freeBox } = boxesOf(d);
  const ticks = ticksOf(v, d);
  const tick = freeBox && text.trim() && !ticks.includes(freeBox.n);
  put(path, { boxText: text, answerWords: null, ...(tick ? { ticks: [...ticks, freeBox.n].sort((a, b) => a - b), selected: null } : {}) });
}

/** Answer with the selected choice, against the fingerprint on screen: a
 *  plain choice, a field's text, or a choice with its note. */
export async function sendAnswer(t: Target, agent: string): Promise<void> {
  const v = viewOf(t.path);
  const d = v.prompt.data;
  if (v.answering || v.selected === null || !d || !d.waiting || !d.readable) return;
  const c = d.choices.find((x) => x.n === v.selected);
  if (!c || c.checked !== undefined) return;
  const way = choiceWay(d, c);
  const text = way.how === "plain" ? "" : v.answerText;
  if (!way.open || (way.how === "field" && !text.trim()) || draftProblem(text, way.how === "field" ? "answer" : "note")) return;
  put(t.path, { answering: true, answerWords: null });
  try {
    const args = { repo: t.repo, slug: t.slug, choice: c.n, fingerprint: d.fingerprint };
    const r = text.trim()
      ? await invoke<AgentAnswer>("agent_answer_text", { ...args, text })
      : await invoke<AgentAnswer>("agent_answer", args);
    const cur = viewOf(t.path);
    const a: Answered = way.how === "field" ? { label: text, at: Date.now(), confirmed: r.confirmed } : { label: r.label, note: text.trim() ? text : undefined, at: Date.now(), confirmed: r.confirmed };
    if (d.picker === "resume") a.resumed = (c.more ?? []).join(" · ");
    put(t.path, {
      ...NO_CHOICE, answering: false, answered: [...cur.answered, a],
      followUp: tellsAfter(c) ? `Now tell ${agent} what to do differently: type it here and press Enter.` : null,
    });
    if (d.picker === "resume") {
      for (const ms of RESUME_REREAD_MS) {
        setTimeout(() => { if (noSessionYet(viewOf(t.path).turns.error)) void loadTurns(t); }, ms);
      }
    }
  } catch (e) {
    put(t.path, { answering: false, answerWords: answerWords(String(e)) });
  }
  void loadPrompt(t);
  void loadTurns(t);
}

/** Send a multi-select's ticks. Its free-text box, when ticked, is filled
 *  first (`agent_answer_text`, which ticks it and sends nothing); that moves
 *  the prompt's fingerprint, so the prompt is read again and the boxes go
 *  against the new one, only if it still shows that box ticked. */
export async function sendMany(t: Target): Promise<void> {
  const v = viewOf(t.path);
  const d = v.prompt.data;
  if (v.answering || !d || !d.waiting || !d.readable || !d.multi) return;
  const { freeBox } = boxesOf(d);
  const want = ticksOf(v, d);
  const fill = !!freeBox && want.includes(freeBox.n);
  if (!want.length || (fill && (!v.boxText.trim() || draftProblem(v.boxText, "answer")))) return;
  put(t.path, { answering: true, answerWords: null });
  try {
    let fingerprint = d.fingerprint;
    if (fill) {
      await invoke<AgentFilled>("agent_answer_text", { repo: t.repo, slug: t.slug, choice: freeBox.n, fingerprint, text: v.boxText });
      const now = await invoke<AgentPrompt>("agent_prompt", { repo: t.repo, slug: t.slug });
      put(t.path, { prompt: { data: now, error: null, loading: false, at: Date.now() } });
      if (!now.waiting || !now.readable || !now.multi || !now.choices.some((c) => c.n === freeBox.n && c.checked)) {
        put(t.path, { answering: false, answerWords: { line: "Your answer was typed into its box, then the prompt changed, so nothing was sent. Check the boxes and send again.", terminal: true } });
        void loadTurns(t);
        return;
      }
      fingerprint = now.fingerprint;
    }
    const r = await invoke<AgentAnsweredMany>("agent_answer_many", { repo: t.repo, slug: t.slug, choices: want, fingerprint });
    const cur = viewOf(t.path);
    put(t.path, { ...NO_CHOICE, answering: false, answered: [...cur.answered, { label: r.labels.join(", "), at: Date.now(), confirmed: r.confirmed }] });
  } catch (e) {
    put(t.path, { answering: false, answerWords: answerWords(String(e)) });
  }
  void loadPrompt(t);
  void loadTurns(t);
}
