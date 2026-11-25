/** The state of a DESK chat — Claude in a session of its own that belongs to no
 *  place: an idea's conversation (the Ideas window) or a workspace's chat (the
 *  main window). Pure, and shared so both say the same thing of the same push. */

/** What `ideas:sessions` and `workspaces:sessions` both carry: tmux sessions. */
export type DeskEv = { live: string[]; busy: string[]; waiting: string[] };
export type ConvState = "running" | "busy" | "waiting" | "ended";

/** What this window just did to a chat, until a push agrees: a start or resume
 *  answers once its session exists, and the poll's next push is up to a tick
 *  away, so the last push still says "ended" (measured on a real serve: one
 *  tick of a Resume bar over a conversation that was starting). */
export type Expect = { conv: string; live: boolean } | null;

/** A chat's state now: the push wins over the list's, which is as old as its
 *  last read — except while `expect` says what this window just did and the
 *  push has not caught up. */
export function stateOf(c: { session: string; state: ConvState; id?: string }, ev: DeskEv | null, expect: Expect = null): ConvState {
  if (expect && c.id === expect.conv && ev && ev.live.includes(c.session) !== expect.live) return expect.live ? "running" : "ended";
  if (!ev) return c.state;
  if (ev.waiting.includes(c.session)) return "waiting";
  if (ev.busy.includes(c.session)) return "busy";
  if (ev.live.includes(c.session)) return "running";
  return "ended";
}

export const STATE_WORDS: Record<ConvState, string> = { running: "running", busy: "working", waiting: "waiting on you", ended: "ended" };
