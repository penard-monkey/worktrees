// The owner's side of group chat's shares (docs/proposals/group-chat.md §1,
// §3.3, §7 "David's view"): one store of what the host holds — every share
// (`shares_all`), the guests (`guests_list`) and the open share requests
// (`share_requests_list`) — re-read on every `shares:changed` and on a slow
// poll. Every row here is the CONSOLE's (ipc.ts CONSOLE_ROWS): a window that
// drives a paired host reads nothing and shows no share control.

import { useSyncExternalStore } from "react";
import { invoke, listen, notGranted } from "./ipc";
import type { ThreadAccess, ThreadEntry, ThreadPage } from "./thread";

export type Admission = "invited" | "asked" | "allowed";
export type ShareGuest = { principal: string; state: Admission; since: number };
export type Binding = {
  harness: string;
  session: string;
  transcript: string;
  bound_at: number;
  state: "active" | "ended";
  ended_offset?: number;
  ended_at?: number;
};
/** `serve::shares::Share`, as `shares_all` answers it. `project` is the
 *  registry key: the repo root a place's `repo` is. */
export type Share = {
  id: string;
  project: string;
  slug: string;
  created_at: number;
  guests: ShareGuest[];
  binding: Binding | null;
  ended: Binding[];
};
export type GuestDevice = { id: string; name: string; platform?: string; revoked_at?: number | null; last_seen?: number | null; [k: string]: unknown };
/** `devices::GuestView`. */
export type Guest = { id: string; name: string; created_at: number; devices: GuestDevice[] };
/** One open request (`share_requests_list`). */
export type ShareRequest = { id: string; principal: string; name: string | null; project: string | null; slug: string; note: string | null; at: number };

export type SharesState = {
  /** null until read, or when this window cannot (not the console). */
  shares: Share[] | null;
  guests: Guest[];
  requests: ShareRequest[];
  hidden: string[];
  /** The name guests see the owner by; "owner" until set. */
  owner: string;
  error: string | null;
  /** Bumped on every `shares:changed`: the thread panel re-reads on it. */
  tick: number;
  /** Join requests the owner has not looked at: share id + principal. */
  asked: { share: string; principal: string; since: number }[];
};

let state: SharesState = { shares: null, guests: [], requests: [], hidden: [], owner: "owner", error: null, tick: 0, asked: [] };
const subs = new Set<() => void>();

function set(patch: Partial<SharesState>): void {
  state = { ...state, ...patch };
  for (const f of subs) f();
}

export function useShares(): SharesState {
  return useSyncExternalStore(
    (f) => {
      subs.add(f);
      return () => subs.delete(f);
    },
    () => state,
  );
}

/** This window may act on shares: it is its own host's console. */
export function sharesHere(): boolean {
  return !notGranted("shares_all");
}

/** The owner's display name in the thread: "owner" (never set) reads as "You". */
export function ownerWords(s: SharesState): string {
  return s.owner && s.owner !== "owner" ? s.owner : "You";
}

/** The owner as the AGENT is told of them in *Use these*' frame: the name
 *  guests see (`owner_set_name`), or "the owner" until one is set, so the
 *  line never reads "You shares". */
export function ownerForAgent(s: SharesState): string {
  return s.owner && s.owner !== "owner" ? s.owner : "the owner";
}

/** Join requests the owner dismissed from the corner, by share, guest and
 *  the moment they asked: a later ask shows again. */
const seen = new Set<string>();
const askKey = (share: string, principal: string, since: number) => `${share}|${principal}|${since}`;

let reading: Promise<void> | null = null;
let again = false;

/** Re-read all three; one read in flight, with one more after it at most. */
export function readShares(): Promise<void> {
  if (!sharesHere()) return Promise.resolve();
  if (reading) {
    again = true;
    return reading;
  }
  reading = (async () => {
    try {
      const [all, guests, requests] = await Promise.all([
        invoke<{ shares: Share[]; hidden_projects: string[]; owner?: string | null }>("shares_all"),
        invoke<Guest[]>("guests_list"),
        invoke<ShareRequest[]>("share_requests_list"),
      ]);
      // Join requests: a guest whose state is `asked` on a share.
      const asked = all.shares.flatMap((s) =>
        s.guests.filter((g) => g.state === "asked" && !seen.has(askKey(s.id, g.principal, g.since))).map((g) => ({ share: s.id, principal: g.principal, since: g.since })),
      );
      set({ shares: all.shares, hidden: all.hidden_projects ?? [], owner: all.owner || "owner", guests, requests, asked, error: null });
    } catch (e) {
      set({ error: String(e) });
    }
  })().finally(() => {
    reading = null;
    if (again) {
      again = false;
      void readShares();
    }
  });
  return reading;
}

const POLL_MS = 30_000;

/** Watch `shares:changed` while the engine is up (RemoteLayer mounts it, keyed
 *  by the engine's gen). Returns the teardown. */
export function watchShares(): () => void {
  if (!sharesHere()) return () => {};
  let live = true;
  void readShares();
  const un = listen<Record<string, unknown>>("shares:changed", () => {
    if (!live) return;
    set({ tick: state.tick + 1 });
    void readShares();
  });
  const id = window.setInterval(() => {
    if (live && document.visibilityState !== "hidden") void readShares();
  }, POLL_MS);
  return () => {
    live = false;
    window.clearInterval(id);
    void un.then((f) => f());
  };
}

/** The shares attached to one lane. */
export function sharesOf(s: SharesState, repo: string, slug: string): Share[] {
  return (s.shares ?? []).filter((x) => x.project === repo && x.slug === slug);
}

export function guestName(s: SharesState, principal: string): string {
  return s.guests.find((g) => g.id === principal)?.name ?? "a removed guest";
}

/** "Bret", "Bret and Eve": the guests a share names. */
export function namesOf(s: SharesState, share: Share): string[] {
  return share.guests.map((g) => guestName(s, g.principal));
}

/** A guest with a device that can still connect. One without is REMOVED
 *  (serve's `GuestView::removed`): listed nowhere, its name free, and an
 *  invite by that name re-admits the same person. */
export function guestLive(g: Guest): boolean {
  return g.devices.some((d) => !d.revoked_at);
}

/** §1's warning, exact, for the guests about to read the session. */
export function shareWarning(names: readonly string[]): string {
  const who = names.length <= 1 ? names[0] ?? "This guest" : names.slice(0, -1).join(", ") + " and " + names[names.length - 1];
  return `${who} can read this whole session from its first turn, including anything it printed. This can't be recalled.`;
}

/** `share_create`/`share_again`'s refusal, in words. The one measured trap:
 *  a fresh or reopened Codex lane has no rollout until its first turn, so the
 *  host cannot name its session yet. */
export function shareRefusal(err: string): string {
  const e = String(err);
  if (e.includes("no agent session is running in this place") || e.includes("has not named its session yet")) {
    return "The agent has not started a session the host can name yet: send the agent something first, then share.";
  }
  return e;
}

/** The thread's access through the engine door, as the console. */
export function consoleThread(share: string): ThreadAccess {
  return {
    read: (after, limit) => invoke<ThreadPage>("thread_read", { share, after, limit }),
    post: (id, text) => invoke<ThreadEntry>("thread_post", { share, id, text }),
  };
}

/** Mark the join requests of `share` as seen (the corner alert goes). */
export function seenAsks(share: string): void {
  for (const a of state.asked) if (a.share === share) seen.add(askKey(a.share, a.principal, a.since));
  set({ asked: state.asked.filter((a) => a.share !== share) });
}

/** The thread panel open beside a lane, per place key (`repo|slug`). */
let threadOpen = new Set<string>();
const threadSubs = new Set<() => void>();
export function useThreadOpen(key: string): boolean {
  return useSyncExternalStore(
    (f) => {
      threadSubs.add(f);
      return () => threadSubs.delete(f);
    },
    () => threadOpen.has(key),
  );
}
export function setThreadOpen(key: string, on: boolean): void {
  const next = new Set(threadOpen);
  if (on) next.add(key);
  else next.delete(key);
  threadOpen = next;
  for (const f of threadSubs) f();
}
