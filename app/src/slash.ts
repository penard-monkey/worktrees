// Slash commands in the chat's composer (docs/proposals/chat-slash-commands.md):
// the rules the menu and the composer follow, apart from the component and
// with no import, so `slashmenu-check.mjs` runs them as they are.
//
// The host is the authority: `agent_commands` lists what the session has, and
// `agent_command` refuses anything else. These only decide what the composer
// OFFERS and when it asks twice. (Its one import is a type: none at runtime.)

import type { SlashCommand } from "./chat";

/** The draft as a command line: `/name` or `/name args` (`args` is null
 *  until a space is typed). Null for anything that is not one — a line that
 *  starts with a space is a message. */
export type SlashDraft = { name: string; args: string | null };
export function slashDraft(draft: string): SlashDraft | null {
  if (!draft.startsWith("/")) return null;
  const rest = draft.slice(1);
  const sp = rest.search(/\s/);
  return sp < 0 ? { name: rest, args: null } : { name: rest.slice(0, sp), args: rest.slice(sp + 1) };
}

/** The menu shows while a name is being typed: `/`, then no space yet. */
export const menuFor = (draft: string): string | null => {
  const d = slashDraft(draft);
  return d && d.args === null ? d.name : null;
};

export const MENU_MAX = 50;

/** The menu's rows for `query` (the name typed so far, no `/`), case-
 *  insensitively: names that start with it (shortest first, so an exact name
 *  is on top), then names that contain it, then — from three letters on —
 *  descriptions that do. */
export function filterCommands(list: SlashCommand[], query: string): SlashCommand[] {
  const q = query.toLowerCase();
  const by = (a: SlashCommand, b: SlashCommand) => a.name.length - b.name.length || a.name.localeCompare(b.name);
  const starts = list.filter((c) => c.name.toLowerCase().startsWith(q)).sort(by);
  const inName = list.filter((c) => !c.name.toLowerCase().startsWith(q) && c.name.toLowerCase().includes(q)).sort(by);
  // A description is searched from three letters on: before that nearly
  // every description holds the query, and the menu would list them all.
  const inText = q.length >= 3 ? list.filter((c) => !c.name.toLowerCase().includes(q) && c.description.toLowerCase().includes(q)) : [];
  return [...starts, ...inName, ...inText].slice(0, MENU_MAX);
}

/** Where a command comes from, in words. */
export function sourceWord(c: SlashCommand): string {
  if (c.source === "plugin") return c.plugin ? `Plugin · ${c.plugin}` : "Plugin";
  return { "built-in": "Built-in", user: "Yours", project: "This repository" }[c.source] ?? c.source;
}

/** The words a command is confirmed with — a sensitive one, or one that
 *  replaces or ends the conversation (`/clear`, Codex's `/new`) — or null. */
export function confirmWords(c: SlashCommand): string | null {
  if (c.confirm) return c.confirm;
  return c.tier === "sensitive" ? (c.why ? `A sensitive command: ${c.why}.` : "A sensitive command.") : null;
}

/** A command waiting for its second press: the line, until when. */
export type Armed = { line: string; until: number };
/** How long a first press waits for the second. */
export const ARM_MS = 4000;

/** One press of Enter (or Run) on `line`, a run of `c`: run it now, or arm
 *  it and wait for a second press of the SAME line within `ARM_MS`. A command
 *  with no confirm words runs at once. */
export function confirmStep(c: SlashCommand, line: string, armed: Armed | null, now: number): { run: boolean; armed: Armed | null } {
  if (!confirmWords(c)) return { run: true, armed: null };
  if (armed && armed.line === line && now <= armed.until) return { run: true, armed: null };
  return { run: false, armed: { line, until: now + ARM_MS } };
}

/** Why the composer will not run the draft as a command, in words, or null.
 *  `argProblem` is the line rule for the argument (`draftProblem`, the
 *  client's mirror of `typed_line_ok`), passed in so this stays import-free. */
export function commandProblem(list: SlashCommand[] | null, draft: string, argProblem: (s: string) => string | null): string | null {
  const d = slashDraft(draft);
  if (!d) return null;
  if (!d.name) return "Type a command’s name, or pick one from the list.";
  if (!list) return "Reading the agent’s commands…";
  const c = list.find((x) => x.name === d.name);
  if (!c) return `/${d.name} is not one of this agent’s commands.`;
  const args = (d.args ?? "").trim();
  if (!args) return null;
  if (!c.args) return `/${c.name} takes no argument from the chat: send /${c.name} and pick from what it opens (this session only).`;
  if (args.includes("@")) return "An argument may not contain “@”: the agent would open its file list.";
  const p = argProblem(args);
  return p ? `Its argument: ${p.charAt(0).toLowerCase()}${p.slice(1)}` : null;
}

/** `agent_command`'s refusals, in words: which ones point at the terminal,
 *  and which at a prompt that is up. */
export function commandWords(err: string, agent: string): { line: string; terminal?: boolean; prompt?: boolean } {
  if (err.startsWith("unknown command: agent_command")) return { line: "This host runs no slash commands from the chat yet: update Worktrees on it." };
  if (err.startsWith("denied: this command is in the sensitive tier")) return { line: "A sensitive command: this device needs the admin permission to run it." };
  if (err.startsWith("not a command this session has")) return { line: `${err.replace("not a command this session has:", "Not one of this agent’s commands:")}.` };
  if (err.startsWith("a line starting with '!'")) return { line: "“!” runs a shell in the agent’s terminal; it is not a command, and the chat does not send it." };
  if (err.startsWith("a line starting with '#'")) return { line: "“#” writes to the agent’s memory; it is not a command, and the chat does not send it." };
  if (err.includes("takes no argument from the chat")) return { line: err.charAt(0).toUpperCase() + err.slice(1) + "." };
  if (err.startsWith("the agent's / menu lit another command")) return { line: `${agent}’s menu chose another command, so nothing ran; the line is in its input — clear it in the terminal.`, terminal: true };
  if (err.startsWith("the agent is waiting on a prompt")) return { line: "A prompt is waiting: answer it first.", prompt: true };
  if (err.startsWith("the agent's prompt has unsent text")) return { line: `${agent}’s input holds unsent text: send or clear it in the terminal first.`, terminal: true };
  if (err.startsWith("the agent is not at its prompt")) return { line: `${agent} is not at its prompt yet; try again shortly.` };
  if (err.startsWith("slash commands are read for Claude and Codex")) return { line: `The chat runs ${agent}’s commands only for Claude and Codex.` };
  return { line: err.charAt(0).toUpperCase() + err.slice(1), terminal: /terminal|at the host/.test(err) };
}
