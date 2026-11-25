/** A workspace's chats: what the rows answer, and the RULES every place that
 *  lists them follows (the nav's card, the Overview, the Chats tab, the chat's
 *  own menu), pure. `scripts/wschats-check.mjs` loads this file with desk.ts
 *  inlined. */
import { stateOf, type ConvState, type DeskEv, type Expect } from "./desk";

/** `workspacechats::Fold`. */
export type ChatFold = { asked?: string; state: string; commit?: string; files?: number; lines?: { added: number; removed: number }; reverted: boolean; why?: string };
/** `workspace_chats`' row. */
export type WsChat = {
  id: string; harness: string; session_id: string; title: string; started: string; last_activity: string; note: string;
  fold?: ChatFold; session: string; state: ConvState; note_exists: boolean;
};
/** What a chat reads of one project, and why not. */
export type ProjectRead = { root: string; name: string | null; registered: boolean; private: boolean; reads: string; why?: string };
export type WsChats = { id: string; name: string; folder: string; chats: WsChat[]; projects: ProjectRead[] };
/** `workspaces:sessions`. */
export type WsSessionsEv = DeskEv & {
  chat_of: Record<string, { workspace: string; chat: string }>;
  fold: Record<string, { workspace: string; chat: string; state: string; why: string | null }>;
};

/** What a row shows: a chat's own state, or its fold's while there is one to say. */
export type RowState = ConvState | "folding" | "fold-failed";
export type ChatRow = { chat: WsChat; state: RowState; live: boolean };

export function rowOf(c: WsChat, ev: WsSessionsEv | null, expect: Expect): ChatRow {
  const s = stateOf(c, ev, expect);
  const f = ev?.fold?.[c.session]?.state;
  // Waiting on the person comes before anything a fold says.
  const state: RowState = s === "waiting" || f === "waiting" ? "waiting" : f === "failed" ? "fold-failed" : f === "folding" ? "folding" : s;
  return { chat: c, state, live: s !== "ended" };
}

const RANK: Record<RowState, number> = { waiting: 0, "fold-failed": 1, busy: 2, folding: 3, running: 4, ended: 5 };
const at = (iso: string) => { const t = Date.parse(iso); return Number.isNaN(t) ? 0 : t; };
/** When a chat last did something, as epoch seconds. */
export const activeAt = (c: WsChat): number => Math.floor(Math.max(at(c.last_activity), at(c.started)) / 1000);

/** Every chat in the order a list shows them: what needs a person, then what
 *  runs, then the closed ones; the newest first within each. */
export function chatRows(chats: readonly WsChat[], ev: WsSessionsEv | null, expect: (conv: string) => Expect = () => null): ChatRow[] {
  return chats.map((c) => rowOf(c, ev, expect(c.id)))
    .sort((a, b) => RANK[a.state] - RANK[b.state] || activeAt(b.chat) - activeAt(a.chat) || (a.chat.id < b.chat.id ? 1 : -1));
}

/** The closed chats a list keeps beside the live ones. */
export const CLOSED_SHOWN = 3;
/** What the nav's card lists: every live one (and one whose fold has something
 *  to say), then three closed. `rest`: there are more than these. With a filter
 *  it is every chat whose title matches, and no more. */
export function navRows(rows: readonly ChatRow[], q = ""): { rows: ChatRow[]; rest: boolean } {
  if (q) return { rows: rows.filter((r) => r.chat.title.toLowerCase().includes(q)), rest: false };
  const open = rows.filter((r) => r.state !== "ended"), closed = rows.filter((r) => r.state === "ended");
  return { rows: [...open, ...closed.slice(0, CLOSED_SHOWN)], rest: closed.length > CLOSED_SHOWN };
}

/** The age column: a word before it when the chat needs somebody, or folds. */
export function ageWords(r: ChatRow, age: string): { text: string; strong: boolean } {
  switch (r.state) {
    case "waiting": return { text: `waiting ${age}`.trim(), strong: true };
    case "fold-failed": return { text: "fold failed", strong: true };
    case "folding": return { text: "folding…", strong: true };
    default: return { text: age, strong: false };
  }
}

/** A chat's state in words, where there is room for them. */
export function stateWords(r: ChatRow): string {
  switch (r.state) {
    case "waiting": return "waiting on you";
    case "busy": return "working";
    case "running": return "open";
    case "folding": return "folding";
    case "fold-failed": return "the fold failed";
    default: {
      const f = r.chat.fold;
      if (!f || f.state !== "done") return "closed";
      if (f.reverted) return "closed · its fold was undone";
      return f.commit ? "closed · folded into memory" : "closed · no memory changes";
    }
  }
}

/** How many of its projects a chat reads: "2 of 3", or null with none. */
export function readsCount(projects: readonly ProjectRead[]): { read: number; of: number } {
  return { read: projects.filter((p) => p.reads === "yes").length, of: projects.length };
}
export function readsWords(projects: readonly ProjectRead[]): string {
  const { read, of } = readsCount(projects);
  return of === 0 ? "reads no project" : `reads ${read} of ${of}`;
}
/** What a chat is, said once over its transcript. */
export function introWords(projects: readonly ProjectRead[]): string {
  const { read, of } = readsCount(projects);
  const can = of === 0 ? "the workspace has no projects to read" : read === 0 ? "it can read none of its projects" : read === of ? `it can read its ${of === 1 ? "project" : `${of} projects`}` : `it can read ${read} of ${of} projects`;
  return `It reads the memory index first, and ${can}. It never writes in a project.`;
}

/** The engine's own name for a chat nobody has named. */
export const isDefaultTitle = (t: string): boolean => /^Chat \d+$/.test(t);
/** A chat's name from the person's first message: its first line, cut at a word. */
export function titleFrom(message: string): string | null {
  const line = message.split(/\r?\n/).map((l) => l.trim()).find(Boolean)?.replace(/\s+/g, " ") ?? "";
  if (!line) return null;
  if (line.length <= 40) return line;
  const cut = line.slice(0, 40);
  const sp = cut.lastIndexOf(" ");
  return `${(sp > 20 ? cut.slice(0, sp) : cut).replace(/[\s.,;:—-]+$/, "")}…`;
}
/** The person's first message among a chat's turns: the first that is theirs.
 *  The opener is the engine's, and a notice is Claude's own entry. */
export function firstMessage(turns: readonly { user: string | null; notice?: unknown }[], conv: string): string | null {
  const t = turns.find((x) => x.user && !x.notice && !x.user.startsWith(`Read chats/${conv}/`));
  return t?.user ?? null;
}

/** What the engine said to start a chat (or a fold), which is not the person's:
 *  a quiet line in the transcript. `null`: a message like any other. */
export function engineLine(text: string, conv: string): string | null {
  const t = text.trim();
  if (t === `Read chats/${conv}/start.md and begin.`) return "The chat was started.";
  return t === `Read chats/${conv}/fold.md and do what it says.` || t === `Read chats/${conv}/fold.md and begin.` ? "The chat was asked to fold into memory." : null;
}
/** A prompt that asks whether the workspace's own folder is trusted. */
export const asksTrust = (lines: readonly string[] | undefined): boolean => !!lines && /\btrust\b/i.test(lines.join(" ")) && /\bfolder\b|workspaces\//i.test(lines.join(" "));
export const TRUST_WORDS = "This is the workspace's own folder. Worktrees made it, and it holds this workspace's chats and memory. Claude asks once.";

/** The months the Chats tab groups by: "November 2026". */
export function monthOf(c: WsChat): string {
  const t = activeAt(c);
  return t ? new Date(t * 1000).toLocaleString([], { month: "long", year: "numeric" }) : "Undated";
}
/** A day and time, for a person. */
export function whenWords(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString([], { year: "numeric", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}
