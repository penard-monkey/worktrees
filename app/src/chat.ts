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
//
// Memory only: turns, prompts, drafts and pending messages live for the
// window's life and are fetched again after a reload. Nothing is written.

import { useSyncExternalStore } from "react";
import { invoke, windowOnHost } from "./ipc";

// ── the rows' shapes ────────────────────────────────────────────────────────

/** `turns::Turn`. `user: null` when the window starts inside the turn. */
export type Turn = {
  at: number | null; user: string | null; reply: string; tools: string[]; truncated: boolean;
  /** The turn in order (`turns::Step`): absent from a host that predates it. */
  steps?: Step[];
};
/** `turns::Step`: the reply's blocks, the tools, and what a person was asked
 *  and answered on the way, each at its own time. */
export type Step =
  | { kind: "text"; at: number | null; text: string }
  | { kind: "tool"; at: number | null; name: string }
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
 *  note. `host_only: "text"` on a field means it is empty. */
export type PromptChoice = {
  n: number; label: string; more?: string[]; highlighted: boolean;
  host_only?: "widens" | "text"; checked?: boolean; text?: "field" | "amend";
};
/** What a prompt taller than the pane asks, whole: Claude's own tool call
 *  (its PermissionRequest hook) or a plan's own file. */
export type PromptDetail = { tool: string; input: Record<string, unknown>; from: "hook" | "plan-file" };
export type AgentPrompt =
  | { waiting: false }
  | {
    waiting: true; readable: true; harness: string; session: string; kind: "permission" | "question";
    fingerprint: string; lines: string[]; choices: PromptChoice[]; note: string; multi?: boolean; detail?: PromptDetail;
  }
  | { waiting: true; readable: false; harness: string | null; session: string | null; reason: string; lines?: string[]; note?: string };
export type ReadablePrompt = Extract<AgentPrompt, { readable: true }>;
export type AgentSent = { delivered: boolean; queued: boolean; harness: string; session: string; note: string };
export type AgentAnswer = { answered: true; confirmed: boolean; harness: string; session: string; choice: number; label: string };
/** A multi-select's free-text box, filled: nothing is sent yet. */
export type AgentFilled = { answered: false; harness: string; session: string; choice: number; note: string };
export type AgentAnsweredMany = { answered: true; confirmed: boolean; harness: string; session: string; choices: number[]; labels: string[] };

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
  noBox: "name at least one box to tick",
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
  if (err.startsWith(AGENT_ERR.noLanePrefix)) return { line: `No ${agent} is running in this place any more.` };
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

// ── the conversation ────────────────────────────────────────────────────────

/** A message `agent_send` confirmed, held until the transcript has it. */
export type Pending = { id: number; text: string; sentAt: number; queued: boolean };
/** An answer this window sent. */
/** An answer this window sent: the choice (a field's: its text), and the
 *  note that rode on it. */
export type Answered = { label: string; note?: string; at: number; confirmed: boolean };

export type Item =
  | { k: "sep"; key: string; at: number }
  | { k: "earlier"; key: string }
  | { k: "user"; key: string; text: string; at: number | null }
  | { k: "reply"; key: string; text: string; tools: string[]; truncated: boolean; at: number | null; label: boolean }
  | { k: "asked"; key: string; step: AskedStep; label: boolean }
  | { k: "answer"; key: string; step: AskedStep }
  | { k: "answered"; key: string; a: Answered }
  | { k: "pending"; key: string; p: Pending };

const GAP_S = 5 * 60;
const sameDay = (a: number, b: number) => new Date(a * 1000).toDateString() === new Date(b * 1000).toDateString();

/** Turns oldest first, with a time separator at a gap of more than five
 *  minutes or a new day, this window's answers where they fall in time, and
 *  the messages the transcript has not shown yet at the end. A turn with
 *  `steps` is drawn in their order: each reply block at its own time, the
 *  tools after the block they followed, and every question, permission and
 *  plan a person answered as what was asked and then the answer. */
export function itemsOf(turns: Turn[], answered: Answered[], pending: Pending[]): Item[] {
  const out: Item[] = [];
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
  turns.forEach((t, i) => {
    if (t.at !== null) flushAnswered(t.at);
    sep(t.at, `s${i}`);
    if (t.user === null) out.push({ k: "earlier", key: `e${i}` });
    else {
      out.push({ k: "user", key: `u${i}`, text: t.user, at: t.at });
      lastWasAgent = false;
    }
    if (!t.steps?.length) {
      if (t.reply || t.tools.length || t.truncated) {
        out.push({ k: "reply", key: `r${i}`, text: t.reply, tools: t.tools, truncated: t.truncated, at: t.at, label: !lastWasAgent });
        lastWasAgent = true;
      }
      return;
    }
    let reply: Extract<Item, { k: "reply" }> | null = null;
    t.steps.forEach((st, j) => {
      const key = `${i}.${j}`;
      if (st.kind === "text" || st.kind === "tool") {
        if (st.kind === "text" || !reply) {
          if (st.at !== null) { flushAnswered(st.at); sep(st.at, `s${key}`); }
          reply = { k: "reply", key: `r${key}`, text: st.kind === "text" ? st.text : "", tools: [], truncated: false, at: st.at, label: !lastWasAgent };
          out.push(reply);
          lastWasAgent = true;
        }
        if (st.kind === "tool") reply.tools.push(st.name);
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
  for (const p of pending) out.push({ k: "pending", key: `p${p.id}`, p });
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

/** The parts of a `KeyboardEvent` the card's keys read. */
export type CardKeyEvent = { key: string; metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; isComposing: boolean; repeat: boolean };
export type CardKeyState = {
  focus: KeyFocus;
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
 *  - `hold`: an Enter with nothing to send, kept from clicking a focused choice. */
export type CardKeyAct =
  | { act: "select"; n: number; choice: number | null; field: boolean }
  | { act: "tick"; n: number }
  | { act: "send" }
  | { act: "hold" };

/** The card's answer to a key, or null when the key is not the card's (and
 *  must not be prevented). Never while an answer is in flight, a modal is up,
 *  a modifier is held, an IME is composing, or the keyboard is in a text field
 *  or a terminal. Enter only from the card or from nowhere: another control's
 *  Enter is its own. */
export function cardKey(e: CardKeyEvent, s: CardKeyState, p: ReadablePrompt): CardKeyAct | null {
  if (e.metaKey || e.ctrlKey || e.altKey || e.shiftKey || e.isComposing || e.repeat) return null;
  if (!s.shown || s.escapeDepth > 0 || s.answering) return null;
  if (s.focus === "composer" || s.focus === "editable" || s.focus === "xterm") return null;
  if (e.key === "Enter") {
    if (s.focus !== "card" && s.focus !== "none") return null;
    return s.canSend ? { act: "send" } : { act: "hold" };
  }
  if (!/^[1-9]$/.test(e.key)) return null;
  const n = Number(e.key);
  const c = p.choices.find((x) => x.n === n);
  // A choice the card draws disabled is not there for a key either.
  if (!c || !choiceOpen(c).open) return null;
  if (p.multi && c.checked !== undefined) return { act: "tick", n };
  const on = s.selected !== n;
  return { act: "select", n, choice: on ? n : null, field: on && choiceOpen(c).how !== "plain" };
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
  const ns = p.choices.filter((c) => c.n >= 1 && c.n <= 9 && choiceOpen(c).open).map((c) => c.n);
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
};

const blank = (): ChatView => ({
  turns: { data: null, error: null, loading: false, at: null },
  prompt: { data: null, error: null, loading: false, at: null },
  pending: [], answered: [], draft: "", sending: false, sendWords: null, followUp: null,
  selected: null, answerText: "", ticks: null, boxText: "", answering: false, answerWords: null,
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
        sending: false, followUp: null,
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
  if (!c) return;
  const way = choiceOpen(c);
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
    put(t.path, {
      ...NO_CHOICE, answering: false, answered: [...cur.answered, a],
      followUp: tellsAfter(c) ? `Now tell ${agent} what to do differently: type it here and press Enter.` : null,
    });
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
