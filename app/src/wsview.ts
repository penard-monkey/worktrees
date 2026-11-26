/** The workspace view's rules (docs/proposals/workspace-view-design.md §2, §3),
 *  pure: which tabs and chips a window draws, what of a workspace needs a
 *  person, and a card's dot. No runtime import but a type's own module:
 *  `scripts/wsview-check.mjs` loads this file alone. */
import type { WsTab } from "./navHistory";

/** The betas the view is drawn under. Each part asks its own (beta.ts): Plan
 *  and Ideas are Ideas', Memory and Chats are workspace chats'. */
export type WsBetas = { ideas: boolean; chats: boolean };

/** The frame is drawn with either on. Off with both: a workspace's name folds
 *  its card, as it always did. */
export const wsFrameOn = (b: WsBetas): boolean => b.ideas || b.chats;

/** The tabs, in order. Overview is always there: it always has Projects. */
export function wsTabs(b: WsBetas): WsTab[] {
  return ["overview", ...(b.chats ? ["memory" as const] : []), ...(b.ideas ? ["plan" as const, "ideas" as const] : []), ...(b.chats ? ["chats" as const] : [])];
}
/** The tabs of workspace `id`'s view: those of the betas, and the tab of a document of
 *  it whose text can be saved nowhere, whatever the betas say. */
export function tabsWith(base: readonly WsTab[], id: string, stuck: { ws: string; tab: WsTab } | null): readonly WsTab[] {
  if (!stuck || stuck.ws !== id || base.includes(stuck.tab)) return base;
  return wsTabs({ ideas: true, chats: true }).filter((t) => t === stuck.tab || base.includes(t));
}
/** The tab to show when `t` is asked for: itself, or Overview when it is not drawn. */
export const tabOr = (t: WsTab, tabs: readonly WsTab[]): WsTab => (tabs.includes(t) ? t : "overview");

/** The documents row in a workspace's nav card: the tabs that are documents. */
export const docChips = (b: WsBetas): ("memory" | "plan" | "ideas")[] =>
  wsTabs(b).filter((t): t is "memory" | "plan" | "ideas" => t === "memory" || t === "plan" || t === "ideas");

/** A card's dot for what is inside it: waiting on a person first, then working,
 *  then just finished. One working lane must not hide a waiting one. */
export type Rollup = "" | "waiting" | "busy" | "done";
export function rollupOf<T>(items: readonly T[], activity: (t: T) => string, justDone: (t: T) => boolean): Rollup {
  if (items.some((t) => activity(t) === "waiting")) return "waiting";
  if (items.some((t) => activity(t) === "busy")) return "busy";
  return items.some(justDone) ? "done" : "";
}

/** One place's mark from `sessions:busy`'s two sets. A place can be in both
 *  (a Codex working beside a Claude stopped on a prompt): waiting wins, as it
 *  does wherever several agents roll up into one mark. */
export function placeActivity(busy: ReadonlySet<string>, waiting: ReadonlySet<string>, path: string): "busy" | "waiting" | "" {
  return waiting.has(path) ? "waiting" : busy.has(path) ? "busy" : "";
}

/** A workspace chat's row state as its workspace's dot reads it: one that
 *  needs a person (waiting, or a fold that failed) is waiting; one that runs
 *  a turn or a fold is busy. */
export function chatActivity(state: string): "busy" | "waiting" | "" {
  return state === "waiting" || state === "fold-failed" ? "waiting" : state === "busy" || state === "folding" || state === "checking" ? "busy" : "";
}

/** One line of what needs a person, over a workspace's Overview. */
export type Attention =
  | { kind: "chat"; text: string; conv: string }
  | { kind: "unsaved"; text: string }
  | { kind: "plan"; text: string };

/** Only what needs somebody: a chat that waits (or whose fold failed), then a
 *  document whose text is saved nowhere, then a plan whose reader found problems (or that cannot be read). Nothing: []. */
export function attentionOf(a: {
  /** A document in the main window that is not saved, and the workspace it
   *  waits in when that is another one. */
  unsaved: { title: string; elsewhere: string | null } | null;
  problems: readonly string[];
  planError: string | null;
  chats?: readonly { conv: string; title: string; failed: boolean }[];
}): Attention[] {
  const out: Attention[] = (a.chats ?? []).map((c) => ({ kind: "chat", conv: c.conv, text: c.failed ? `The fold of “${c.title}” did not finish.` : `“${c.title}” is waiting for an answer.` }));
  if (a.unsaved) out.push({ kind: "unsaved", text: `“${a.unsaved.title}”${a.unsaved.elsewhere ? `, in ${a.unsaved.elsewhere},` : ""} has text that is not saved.` });
  if (a.planError) out.push({ kind: "plan", text: `The plan cannot be read: ${a.planError}.` });
  else if (a.problems.length) out.push({ kind: "plan", text: a.problems.length === 1 ? `The plan has a problem. ${a.problems[0]}` : `The plan has ${a.problems.length} problems. ${a.problems[0]}` });
  return out;
}

/** The workspace view as it was at the last quit, for the launch restore (the
 *  same switch a place's is: Settings' "restore last"). Browser storage, read as
 *  anything may be in it. */
export const LAST_VIEW_KEY = "wt.last.view";
export type LastView = { id: string; tab: WsTab; doc?: string; chat?: string };
const TABS: readonly string[] = ["overview", "memory", "plan", "ideas", "chats"];
export function lastView(raw: string | null): LastView | null {
  try {
    const v = JSON.parse(raw ?? "null") as Record<string, unknown> | null;
    if (!v || typeof v.id !== "string" || !v.id || typeof v.tab !== "string" || !TABS.includes(v.tab)) return null;
    return { id: v.id, tab: v.tab as WsTab, ...(typeof v.doc === "string" && v.doc ? { doc: v.doc } : {}), ...(typeof v.chat === "string" && v.chat ? { chat: v.chat } : {}) };
  } catch { return null; }
}

/** The workspaces that hold a project, in their order. */
export const placeWorkspaces = <T extends { id: string; projects: readonly { root: string }[] }>(all: readonly T[], root: string): T[] => all.filter((w) => w.projects.some((p) => p.root === root));
/** Which of them a place is in: the card it was clicked in, while that still holds
 *  its project; else the only one. `null`: none, or a person has to say. */
export const placeWorkspace = <T extends { id: string }>(holds: readonly T[], from: string | null | undefined): T | null =>
  holds.find((w) => w.id === from) ?? (holds.length === 1 ? holds[0] : null);
/** Which card each place was last clicked in, as stored: bounded, and read as
 *  anything may be in it. */
export const PLACE_WS_KEY = "wt.place.ws";
export function placeWsRead(raw: string | null): Record<string, string> {
  try {
    const v = JSON.parse(raw ?? "null") as Record<string, unknown> | null;
    return v && typeof v === "object" && !Array.isArray(v) ? Object.fromEntries(Object.entries(v).filter((e): e is [string, string] => typeof e[1] === "string" && !!e[1])) : {};
  } catch { return {}; }
}
export const PLACE_WS_MAX = 200;
export function placeWsSet(cur: Record<string, string>, key: string, ws: string | null): Record<string, string> {
  const { [key]: _, ...rest } = cur;
  if (!ws) return rest;
  return Object.fromEntries([...Object.entries(rest), [key, ws] as const].slice(-PLACE_WS_MAX));
}

/** The documents' share of the room beside a chat, between both floors. */
export const DOCS_FLOOR = 340, CHAT_FLOOR = 420;
export function splitBounds(width: number): { min: number; max: number } {
  if (!(width > 0)) return { min: 0.2, max: 0.8 };
  const min = DOCS_FLOOR / width, max = 1 - CHAT_FLOOR / width;
  return min <= max ? { min, max } : { min: 0.5, max: 0.5 };
}
export const splitClamp = (share: number, width: number): number => {
  const b = splitBounds(width);
  return Math.min(b.max, Math.max(b.min, share));
};

/** The counts beside a workspace's name. */
export const wsCounts = (projects: number, liveChats = 0): string =>
  `${projects} project${projects === 1 ? "" : "s"}${liveChats ? ` · ${liveChats} chat${liveChats === 1 ? "" : "s"} live` : ""}`;
