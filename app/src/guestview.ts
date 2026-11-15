// The guest views' rules, as data (group chat §7): which turns a page
// replaces, how a turn reads, where a guest row goes, what a re-read says,
// what the badge counts. No runtime imports, on purpose: guestview-check.mjs
// loads this file as a `data:` module and runs the real rules. guest.ts,
// GuestNav.tsx and GuestSession.tsx draw with them.
import type { Notice, Step, Turn } from "./chat";
import type { GuestState } from "./guestdoor";

/** A share's binding as a guest sees it (`Binding::guest_view`). */
export type GuestBinding = { harness: string; state: "active" | "ended"; bound_at: number; ended_at: number | null };

/** One entry of `shares_list` (`Shares::guest_shares`). */
export type GuestShare = {
  id: string;
  project: string;
  project_id: string;
  slug: string;
  title: string | null;
  /** Where this guest stands: named, asked to join, or let in. */
  state: "invited" | "asked" | "allowed";
  binding: GuestBinding | null;
  /** Bindings a Share again replaced, oldest first: `shared_turns`' `binding: n`. */
  ended: GuestBinding[];
  created_at: number;
};

/** `guest_places`: the owner's unhidden projects, names only. */
export type GuestProject = { id: string; name: string; places: { slug: string; title: string | null }[] };

/** What a guest host's lists are, for `placed`. `known`: the project names
 *  its last read held (kept per host by guest.ts), for a run that has not
 *  read the lists yet — a host offline since the app started. */
export type GuestLists = { shares: GuestShare[] | null; projects: GuestProject[] | null; known?: readonly string[] | null };

/** A turn as `shared_turns` answers it: `turns::Turn`, plus its index seq and,
 *  when cut, its counts. Several turns can share one seq. */
export type SharedTurn = Turn & { seq: number; chars?: number; kept?: number };
export type SharedPage = {
  harness: string;
  state: "active" | "ended";
  turns: SharedTurn[];
  total: number;
  next_before: number;
  more: boolean;
};
/** Turns oldest first, after `page` (newest first) is merged into `held`:
 *  every held turn at or past the page's oldest seq is replaced by the page's.
 *  Pure, for the check. */
export function mergeNewest(held: SharedTurn[], page: SharedTurn[]): SharedTurn[] {
  if (!page.length) return held;
  const fresh = [...page].reverse();
  const from = fresh[0].seq;
  return [...held.filter((t) => t.seq < from), ...fresh];
}

/** A poll's page does not reach back to what is held: more turns came than
 *  one page holds, so joining them would hide the ones between. The page
 *  then stands alone, and "Earlier turns" pages back from it. */
export function leavesGap(held: SharedTurn[], page: SharedTurn[]): boolean {
  if (!held.length || !page.length) return false;
  return page[page.length - 1].seq > held[held.length - 1].seq + 1;
}

/** `page` (newest first) is the page BEFORE `held`'s oldest: prepend it. */
export function mergeOlder(held: SharedTurn[], page: SharedTurn[]): SharedTurn[] {
  const until = held.length ? held[0].seq : Infinity;
  return [...[...page].reverse().filter((t) => t.seq < until), ...held];
}

/** The name a guest calls the host's owner by: the name the host answers
 *  (`shares_list`, then `guest_places`), else the host's own name as this
 *  machine paired it. A host that never set one answers `null`; one from
 *  v0.52.0 answers its record's placeholder `"owner"`, which is not a name
 *  either ("OWNER'S PLACES", found in the three-VM run). */
export function ownerOf(listed: string | null | undefined, places: string | null | undefined, hostName: string): string {
  const named = (n: string | null | undefined) => (n && n.trim() && n !== "owner" ? n : null);
  return named(listed) ?? named(places) ?? hostName;
}

/** What the view says about where this guest and this session stand. */
export function standing(share: GuestShare, binding: GuestBinding | null, link: GuestState, owner: string): { live: boolean; words: string | null } {
  if (link.kind === "offline") return { live: false, words: `${owner}'s host is offline` };
  if (share.state === "invited") return { live: false, words: `Asking ${owner} to let you in…` };
  if (share.state === "asked") return { live: false, words: `Waiting for ${owner} to let you in` };
  if (!binding) return { live: false, words: "Nothing is shared in this place right now" };
  if (binding.state === "ended") return { live: false, words: "Ended: this session moved on, and is shown as it was" };
  return { live: link.kind === "up", words: null };
}

export type Piece =
  | { k: "user"; text: string; mid: boolean }
  | { k: "notice"; notice: Notice }
  | { k: "agent"; text: string; calls: Extract<Step, { kind: "tool" }>[]; at: number | null }
  | { k: "asked"; st: Extract<Step, { kind: "question" | "permission" | "plan" }> };

/** One turn as pieces, in its steps' order: a reply block with the tools
 *  that followed it, a message sent mid-turn, what a person was asked. A turn
 *  without steps (an older host) is its reply and its tools. Pure. */
export function piecesOf(t: Turn): Piece[] {
  const out: Piece[] = [];
  if (t.notice) out.push({ k: "notice", notice: t.notice });
  else if (t.user !== null) out.push({ k: "user", text: t.user, mid: false });
  const steps: Step[] = t.steps?.length ? t.steps : [{ kind: "text", at: t.at, text: t.reply }, ...t.tools.map((name) => ({ kind: "tool" as const, at: t.at, name }))];
  let agent: Extract<Piece, { k: "agent" }> | null = null;
  for (const st of steps) {
    if (st.kind === "text" || st.kind === "tool") {
      if (st.kind === "text" || !agent) {
        agent = { k: "agent", text: st.kind === "text" ? st.text : "", calls: [], at: st.at };
        out.push(agent);
      }
      if (st.kind === "tool") agent.calls.push(st);
      continue;
    }
    agent = null;
    out.push(st.kind === "user" ? { k: "user", text: st.text, mid: true } : { k: "asked", st });
  }
  return out.filter((p) => p.k !== "agent" || p.text || p.calls.length);
}

/** A share's fingerprint for the badge: its standing and its bindings. */
export const shareMark = (s: GuestShare): string => `${s.state}|${s.binding?.state ?? "-"}|${s.binding?.bound_at ?? 0}|${s.ended.length}`;

/** The shares the badge counts: never opened, or moved since. */
export function unseenOf(shares: GuestShare[], seen: Record<string, string>): string[] {
  return shares.filter((s) => seen[s.id] !== shareMark(s)).map((s) => s.id);
}

/** The toast a re-read earns: a share that was not there before (or was, and
 *  was shared again), in the words §6 gives. Pure, for the check. */
export function newsOf(owner: string, before: GuestShare[] | null, after: GuestShare[]): string[] {
  if (before === null) return [];
  const was = new Map(before.map((s) => [s.id, s]));
  const out: string[] = [];
  for (const s of after) {
    const b = was.get(s.id);
    const name = s.title || s.slug;
    if (!b) out.push(`${owner} shared ${name} with you`);
    else if (b.state !== "allowed" && s.state === "allowed") out.push(`${owner} let you in on ${name}`);
    else if ((b.binding?.bound_at ?? 0) !== (s.binding?.bound_at ?? 0) && s.binding?.state === "active") out.push(`${owner} shared ${name} again`);
  }
  return out;
}

/** A row's standing in words, and whether it is live. */
export function shareWords(s: GuestShare, owner: string): { words: string; cls: string } {
  if (s.state === "invited") return { words: "Open to ask to join", cls: "invited" };
  if (s.state === "asked") return { words: `Waiting for ${owner}`, cls: "asked" };
  if (!s.binding) return { words: "Nothing bound", cls: "ended" };
  if (s.binding.state === "ended") return { words: "Ended", cls: "ended" };
  return { words: "Shared", cls: "live" };
}

/** Which of a host's projects and shares go under the local project `name`
 *  (null: the ones that match no local project), and `status`: whether the
 *  host's group belongs HERE when it has no rows (offline, connecting), so it
 *  says so in one place. That is where its rows went: under each local
 *  project its names match, else under Shared with me. The names are the
 *  lists it holds or, before any read this run, the ones it last held. An
 *  offline host was listed under its project AND as an empty *Shared with
 *  me* group in the 2026-11-14 rehearsal. Pure, for the check. */
export function placed(hv: GuestLists, name: string | null, localNames: readonly string[]): { shares: GuestShare[]; projects: GuestProject[]; status: boolean } {
  const mine = (p: string) => (name === null ? !localNames.includes(p) : p === name);
  const names = hv.projects || hv.shares
    ? [...(hv.projects ?? []).map((p) => p.name), ...(hv.shares ?? []).map((s) => s.project)]
    : [...(hv.known ?? [])];
  const matched = names.filter((n) => localNames.includes(n));
  return {
    shares: (hv.shares ?? []).filter((s) => mine(s.project)),
    projects: (hv.projects ?? []).filter((p) => mine(p.name)),
    status: name === null ? matched.length === 0 : matched.includes(name),
  };
}
