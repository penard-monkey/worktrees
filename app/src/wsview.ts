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

/** One line of what needs a person, over a workspace's Overview. */
export type Attention =
  | { kind: "unsaved"; text: string }
  | { kind: "plan"; text: string };

/** Only what needs somebody: a document whose text is saved nowhere, then a
 *  plan whose reader found problems (or that cannot be read). Nothing: []. */
export function attentionOf(a: {
  /** A document in the main window that is not saved, and the workspace it
   *  waits in when that is another one. */
  unsaved: { title: string; elsewhere: string | null } | null;
  problems: readonly string[];
  planError: string | null;
}): Attention[] {
  const out: Attention[] = [];
  if (a.unsaved) out.push({ kind: "unsaved", text: `“${a.unsaved.title}”${a.unsaved.elsewhere ? `, in ${a.unsaved.elsewhere},` : ""} has text that is not saved.` });
  if (a.planError) out.push({ kind: "plan", text: `The plan cannot be read: ${a.planError}.` });
  else if (a.problems.length) out.push({ kind: "plan", text: a.problems.length === 1 ? `The plan has a problem. ${a.problems[0]}` : `The plan has ${a.problems.length} problems. ${a.problems[0]}` });
  return out;
}

/** The counts beside a workspace's name. */
export const wsCounts = (projects: number): string => `${projects} project${projects === 1 ? "" : "s"}`;
