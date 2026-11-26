/** A workspace's chats: what the rows answer, and the RULES every place that
 *  lists them follows (the nav's card, the Overview, the Chats tab, the chat's
 *  own menu), pure. `scripts/wschats-check.mjs` loads this file with desk.ts
 *  inlined. */
import { stateOf, type ConvState, type DeskEv, type Expect } from "./desk";

/** `workspacechats::Fold`. */
export type ChatFold = { asked?: string; asked_ms?: number; left?: number; state: string; commit?: string; files?: number; lines?: { added: number; removed: number }; reverted: boolean; why?: string; attempt?: string; before?: string;
  /** Its receipt is written and its session is still being ended. */
  closing?: boolean };
/** A fold that runs, or failed: the live word (`watch::FoldOf`). */
export type FoldNow = { state: string; why: string | null; left?: number | null };
/** `workspace_chats`' row. */
export type WsChat = {
  id: string; harness: string; session_id: string; title: string; started: string; last_activity: string; note: string;
  fold?: ChatFold; fold_now?: FoldNow | null; session: string; state: ConvState; note_exists: boolean;
  /** Whether a live chat still has what a launch now would give it. */
  access?: "current" | "stale" | "unknown" | null; access_changes?: AccessChange[];
};
/** One thing a launch now would do otherwise (`workspacechats::AccessChange`). */
export type AccessChange = { kind: string; name: string; path: string; change: string };
/** What a chat reads of one project, and why not. */
export type ProjectRead = { root: string; name: string | null; registered: boolean; private: boolean; reads: string; why?: string };
/** The workspace's plan and linked ideas, and whether a chat reads each. */
export type DocRead = { kind: "plan" | "idea"; id: string; live: string | null; title: string | null; path: string | null; reads: string; why?: string };
export type WsChats = { id: string; name: string; folder: string; chats: WsChat[]; projects: ProjectRead[]; documents?: DocRead[];
  /** What its rules stop and what they do not. */
  limits?: string };
/** `workspaces:sessions`. */
export type WsSessionsEv = DeskEv & {
  chat_of: Record<string, { workspace: string; chat: string }>;
  fold: Record<string, { workspace: string; chat: string } & FoldNow>;
  /** Each live chat's model, by session: its transcript's newest reply or
   *  `/model` switch. Absent from an older engine's push. */
  model?: Record<string, string>;
};

/** The model a chat's header names: a live chat's, as the push last said it. */
export const modelOf = (c: WsChat, ev: WsSessionsEv | null, live: boolean): string | null => (live ? ev?.model?.[c.session] ?? null : null);

/** What the workspaces' chats need a person for, from one push, as keys for
 *  notify.ts's set: `waiting|<session>` for a chat stopped on a prompt (its
 *  own, or its fold's: a fold runs Claude in the chat's session), and
 *  `failed|<session>` for a fold that did not finish. Sorted. */
export function needsYouKeys(ev: WsSessionsEv | null): string[] {
  if (!ev) return [];
  const waiting = new Set(ev.waiting);
  const failed = new Set<string>();
  for (const [s, f] of Object.entries(ev.fold ?? {})) {
    if (f.state === "waiting") waiting.add(s);
    else if (f.state === "failed") failed.add(s);
  }
  return [...[...failed].map((s) => `failed|${s}`), ...[...waiting].map((s) => `waiting|${s}`)].sort();
}

/** A key of [`needsYouKeys`] as a system notification. `names` gives a
 *  workspace's and a chat's names where this window knows them. */
export function needsYouWords(key: string, ev: WsSessionsEv, names: (ws: string, conv: string) => { ws: string | null; chat: string | null }): { title: string; body: string } {
  const i = key.indexOf("|");
  const kind = key.slice(0, i), s = key.slice(i + 1);
  const f = ev.fold?.[s];
  const of = ev.chat_of?.[s] ?? (f ? { workspace: f.workspace, chat: f.chat } : null);
  const n = of ? names(of.workspace, of.chat) : { ws: null, chat: null };
  const who = n.chat ? `“${n.chat}”${n.ws ? ` in ${n.ws}` : ""}` : null;
  if (kind === "failed") return { title: "A fold did not finish", body: `${who ?? "A chat of a workspace"}: ${f?.why ?? "no reason was given"}`.replace(/([^.!?])$/, "$1.") };
  const fold = !!f && (f.state === "waiting" || f.state === "folding");
  return { title: "A workspace chat is waiting for you", body: fold ? `The fold of ${who ?? "a chat of a workspace"} stopped on a question.` : `${who ?? "A chat of a workspace"} stopped on a question.` };
}

/** What a row shows: a chat's own state, or its fold's while there is one to say. */
export type RowState = ConvState | "folding" | "checking" | "fold-failed";
export type ChatRow = { chat: WsChat; state: RowState; live: boolean };

/** How a chat's fold stands: the list's own word first (it is read again on every
 *  push and after every act here), else the push's, which is up to a tick behind. */
export const foldOf = (c: WsChat, ev: WsSessionsEv | null): FoldNow | null => (c.fold_now !== undefined ? c.fold_now : ev?.fold?.[c.session] ?? null);
/** A fold that has not ended: nothing is sent to its chat meanwhile. */
export const foldRuns = (f: FoldNow | null): boolean => !!f && (f.state === "folding" || f.state === "waiting" || f.state === "checking");

export function rowOf(c: WsChat, ev: WsSessionsEv | null, expect: Expect): ChatRow {
  const f = foldOf(c, ev)?.state;
  // A fold that has landed ends its session a moment later: it is drawn as landed.
  const s = c.fold?.closing && !f ? "ended" : stateOf(c, ev, expect);
  // Waiting on the person comes before anything a fold says.
  const state: RowState = s === "waiting" || f === "waiting" ? "waiting" : f === "failed" ? "fold-failed" : f === "folding" ? "folding" : f === "checking" ? "checking" : s;
  return { chat: c, state, live: s !== "ended" };
}

const RANK: Record<RowState, number> = { waiting: 0, "fold-failed": 1, busy: 2, folding: 3, checking: 3, running: 4, ended: 5 };
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
    case "checking": return { text: "checking…", strong: true };
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
    case "checking": return "checking its fold";
    case "fold-failed": return "the fold failed";
    default: {
      const f = r.chat.fold;
      if (!f || f.state !== "done") return "closed, not folded";
      if (f.reverted) return "closed · its fold was undone";
      return f.commit ? "closed · folded into memory" : "closed · no memory changes";
    }
  }
}

/** An ended chat's line, and whether it offers to fold now. */
export function endedLine(c: WsChat, transcriptGone: boolean): { text: string; foldNow: boolean; resume: boolean } {
  if (transcriptGone) return { text: "Closed. Its transcript is gone.", foldNow: false, resume: false };
  const f = c.fold;
  if (!f || f.state !== "done") return { text: "Closed, not folded.", foldNow: true, resume: true };
  if (f.reverted) return { text: "Closed. Its fold was undone.", foldNow: true, resume: true };
  return { text: f.commit ? "Closed and folded into memory." : "Closed. No memory changes.", foldNow: false, resume: true };
}
export const transcriptGone = (reason: string | undefined): boolean => !!reason && /no longer on disk/.test(reason);
/** A fold that failed, in the engine's own words. It never says more than they do. */
export const failedWords = (f: FoldNow): string => `The fold did not finish: ${f.why ?? "no reason was given"}`.replace(/([^.!?])$/, "$1.");

/** How many of its projects a chat reads: "2 of 3", or null with none. */
export function readsCount(projects: readonly ProjectRead[]): { read: number; of: number } {
  return { read: projects.filter((p) => p.reads === "yes").length, of: projects.length };
}
export function readsWords(projects: readonly ProjectRead[]): string {
  const { read, of } = readsCount(projects);
  return of === 0 ? "reads no project" : `reads ${read} of ${of}`;
}
/** What a chat is, said once over its transcript. */
export function introWords(projects: readonly ProjectRead[], limits?: string): string {
  const { read, of } = readsCount(projects);
  const can = of === 0 ? "the workspace has no projects to read" : read === 0 ? "it can read none of its projects" : read === of ? `it can read its ${of === 1 ? "project" : `${of} projects`}` : `it can read ${read} of ${of} projects`;
  return `It reads the memory index first, and ${can}. It is told not to write in a project. ${limits ?? "Claude's file tools are blocked there; programs it runs are not."}`;
}

const CHANGES: Record<string, (n: string) => string> = {
  private: (n) => `${n} is private now`, removed: (n) => `${n} was taken out of the workspace`, unregistered: (n) => `${n} is not registered any more`,
  unreadable: (n) => `${n} cannot be read any more`, readable: (n) => `${n} can be read now`, added: (n) => `${n} was added`,
  unlinked: (n) => `“${n}” is not linked any more`, gone: (n) => `“${n}” is gone`, moved: (n) => `“${n}” was moved`, linked: (n) => `“${n}” was linked`,
};
/** What a launch now would do otherwise, in words. */
export const accessWords = (c: AccessChange): string =>
  c.kind === "rules" ? "its other rules changed" : (CHANGES[c.change] ?? ((n: string) => `${n}: ${c.change}`))(c.name || c.path.slice(c.path.lastIndexOf("/") + 1) || "something");

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
type Said = { user: string | null; notice?: unknown; engine?: "start" | "fold" };
/** The person's first message among a chat's turns: not one the engine sent (it says
 *  so; its words are never looked at), nor Claude's own entry. */
export const firstMessage = (turns: readonly Said[]): string | null => turns.find((x) => x.user && !x.notice && !x.engine)?.user ?? null;
/** What a turn the engine sent is in the transcript: a quiet line. */
export const quietLine = (t: Pick<Said, "engine">): string | null =>
  t.engine === "start" ? "The chat was started." : t.engine === "fold" ? "The chat was asked to fold into memory." : null;
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
