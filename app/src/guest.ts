// What this window knows as a GUEST (group chat §6, §7): for each host it is
// a guest of, the connection (guestdoor.ts), what is shared with it
// (`shares_list`), the owner's place names (`guest_places`), the badge and
// the toasts. The sidebar (GuestNav.tsx) and the read-only session view
// (GuestSession.tsx) read it; nothing else does.
//
// Reading, never polling the lists: `shares_list` and `guest_places` are read
// on every (re)connect and on `shares:changed`, which is how a closed app's
// news arrives (§6: nothing is queued while the app is closed). A post in a
// thread is a `shares:changed` too; it bumps that share's thread, not the
// lists.
//
// The badge: a share this window has not opened yet, or one whose standing or
// binding moved since it was last opened. "Seen" is kept per host in
// localStorage — a per-viewer convenience, wrapped, so a window without
// storage only badges again after a restart.
//
// No runtime imports but React's store hook and the guest door: no engine
// door, no chat module (guestview-check.mjs walks the graph).
import { useSyncExternalStore } from "react";
import { guestLink, type GuestDial, type GuestEnv, type GuestLink, type GuestState, type SharesChanged } from "./guestdoor";
import type { HostSummary } from "./ipcremote";
import { newsOf, shareMark, unseenOf, type GuestProject, type GuestShare } from "./guestview";

export type { GuestBinding, GuestProject, GuestShare } from "./guestview";

/** What a guest host's window shows. */
export type GuestHostView = {
  host: HostSummary;
  link: GuestState;
  /** The owner's display name (`owner_set_name`), or the host's name. */
  owner: string;
  shares: GuestShare[] | null;
  projects: GuestProject[] | null;
  /** The last read's refusal, while the lists may be stale. */
  error: string | null;
  /** Share ids the badge counts. */
  unseen: string[];
  /** Bumped per share on a post, so its open thread re-reads. */
  threadBump: Record<string, number>;
  /** Bumped on any change, so an open session view re-reads its page. */
  bump: number;
  /** Requests sent from this window this run, `project_id|slug`. */
  asked: string[];
  /** The project names the last read held, kept per host: where an offline
   *  host's group goes before this run has read it (`placed`). */
  known: string[] | null;
};

export type Toast = { id: number; host: string; text: string; share?: string };

/** Which share the window shows in place of a place, if any. */
export type GuestOpen = { host: string; share: string; binding?: number } | null;

type State = { hosts: GuestHostView[]; toasts: Toast[]; open: GuestOpen };

export type GuestSource = {
  /** The hosts this machine is paired with (the shim's `remote_hosts`). */
  hosts(): Promise<HostSummary[]>;
  dial(host: HostSummary): GuestDial;
  env: GuestEnv;
  /** A per-viewer key-value store (localStorage), or null. */
  storage: { get(k: string): string | null; set(k: string, v: string): void } | null;
};

let state: State = { hosts: [], toasts: [], open: null };
const subs = new Set<() => void>();
const set = (f: (s: State) => State) => {
  state = f(state);
  for (const s of [...subs]) s();
};
const links = new Map<string, GuestLink>();
let source: GuestSource | null = null;
let toastN = 0;

const seenKey = (host: string) => `wt-guest-seen:${host}`;
function readSeen(host: string): Record<string, string> {
  try {
    const raw = source?.storage?.get(seenKey(host));
    const v = raw ? JSON.parse(raw) : {};
    return v && typeof v === "object" ? (v as Record<string, string>) : {};
  } catch {
    return {};
  }
}
function writeSeen(host: string, seen: Record<string, string>) {
  try { source?.storage?.set(seenKey(host), JSON.stringify(seen)); } catch { /* storage is a convenience */ }
}
const knownKey = (host: string) => `wt-guest-projects:${host}`;
function readKnown(host: string): string[] | null {
  try {
    const raw = source?.storage?.get(knownKey(host));
    const v = raw ? JSON.parse(raw) : null;
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : null;
  } catch {
    return null;
  }
}
function writeKnown(host: string, names: string[]) {
  try { source?.storage?.set(knownKey(host), JSON.stringify(names)); } catch { /* storage is a convenience */ }
}

const patch = (id: string, f: (h: GuestHostView) => GuestHostView) =>
  set((s) => ({ ...s, hosts: s.hosts.map((h) => (h.host.id === id ? f(h) : h)) }));
const viewOf = (id: string) => state.hosts.find((h) => h.host.id === id);

function toast(host: string, text: string, share?: string) {
  const t: Toast = { id: ++toastN, host, text, ...(share ? { share } : {}) };
  set((s) => ({ ...s, toasts: [...s.toasts, t].slice(-4) }));
  source?.env.setTimeout(() => dismissToast(t.id), 8000);
}

export function dismissToast(id: number) {
  set((s) => ({ ...s, toasts: s.toasts.filter((t) => t.id !== id) }));
}

/** Re-read the lists of one host. */
async function reread(id: string, news: boolean) {
  const link = links.get(id);
  if (!link) return;
  try {
    const [sl, gp] = await Promise.all([
      link.call<{ owner: string; shares: GuestShare[] }>("shares_list"),
      link.call<{ owner: string; projects: GuestProject[] }>("guest_places"),
    ]);
    const before = viewOf(id)?.shares ?? null;
    const owner = sl.owner || gp.owner || link.host.name;
    const said = news ? newsOf(owner, before, sl.shares) : [];
    // The share on screen is seen as it changes: it never badges itself.
    let seen = readSeen(id);
    const shown = state.open?.host === id ? sl.shares.find((x) => x.id === state.open?.share) : undefined;
    if (shown && seen[shown.id] !== shareMark(shown)) {
      seen = { ...seen, [shown.id]: shareMark(shown) };
      writeSeen(id, seen);
    }
    const known = [...new Set([...gp.projects.map((p) => p.name), ...sl.shares.map((x) => x.project)])];
    writeKnown(id, known);
    patch(id, (h) => ({
      ...h, owner, shares: sl.shares, projects: gp.projects, known, error: null,
      unseen: unseenOf(sl.shares, seen), bump: h.bump + 1,
    }));
    for (const s of said) toast(id, s);
  } catch (e) {
    patch(id, (h) => ({ ...h, error: String(e) }));
  }
}

function heard(id: string, p: SharesChanged) {
  if (p.what === "post" && p.share) {
    const share = p.share;
    patch(id, (h) => ({ ...h, threadBump: { ...h.threadBump, [share]: (h.threadBump[share] ?? 0) + 1 } }));
    return;
  }
  // A connect reads quietly (its badge says what is new); a push may toast.
  void reread(id, p.what !== "connected");
}

function adopt(host: HostSummary) {
  if (links.has(host.id) || !source) return;
  const link = guestLink(host, source.dial(host), source.env);
  links.set(host.id, link);
  set((s) => ({
    ...s,
    hosts: [...s.hosts, { host, link: link.state(), owner: host.name, shares: null, projects: null, error: null, unseen: [], threadBump: {}, bump: 0, asked: [], known: readKnown(host.id) }],
  }));
  link.onState(() => patch(host.id, (h) => ({ ...h, link: link.state() })));
  link.onChanged((p) => heard(host.id, p));
}

function drop(id: string) {
  links.get(id)?.close();
  links.delete(id);
  set((s) => ({ ...s, hosts: s.hosts.filter((h) => h.host.id !== id), open: s.open?.host === id ? null : s.open }));
}

/** Read the paired hosts again, and connect to each one this machine is a
 *  guest of (a pairing just made, a host just forgotten). */
export async function refreshGuestHosts(): Promise<void> {
  const src = source;
  if (!src) return;
  let hosts: HostSummary[];
  try {
    hosts = (await src.hosts()).filter((h) => h.guest === true);
  } catch {
    return; // Settings → Hosts says why the list cannot be read
  }
  // Stopped (group chat switched off), or stopped and started again, while
  // the list was read: this answer is not the current source's to act on.
  if (source !== src) return;
  for (const h of hosts) adopt(h);
  for (const id of [...links.keys()]) if (!hosts.some((h) => h.id === id)) drop(id);
}

/** Start, with the app's or the harness's source: main.tsx, each time group
 *  chat's beta switch goes on (betaLive). A second start is a no-op. */
export function startGuests(src: GuestSource): void {
  if (source) return;
  source = src;
  void refreshGuestHosts();
}

/** The window came back into view: a link waiting on a hidden window tries. */
export function guestsVisible(): void {
  for (const l of links.values()) l.retry();
}

export function useGuests(): State {
  return useSyncExternalStore((f) => { subs.add(f); return () => { subs.delete(f); }; }, () => state);
}

export function guestLinkOf(host: string): GuestLink | undefined {
  return links.get(host);
}

/** Show `share` in place of a place; it counts as seen from now. */
export function openShare(host: string, share: string, binding?: number) {
  const h = viewOf(host);
  const s = h?.shares?.find((x) => x.id === share);
  if (h && s) {
    const seen = { ...readSeen(host), [share]: shareMark(s) };
    writeSeen(host, seen);
    patch(host, (v) => ({ ...v, unseen: v.unseen.filter((x) => x !== share) }));
  }
  set((st) => ({ ...st, open: { host, share, ...(binding === undefined ? {} : { binding }) } }));
}

/** Back to the window's own places. */
export function closeShare() {
  if (state.open) set((s) => ({ ...s, open: null }));
}

/** Open asks to join (§1 step 4): `share_join`, then the lists again. */
export async function joinShare(host: string, share: string): Promise<string> {
  const link = links.get(host);
  if (!link) throw new Error("not connected");
  const r = await link.call<{ state: string }>("share_join", { share });
  await reread(host, false);
  return r.state;
}

/** *Ask David to share* (§2.2): grants nothing; the owner's app gets a nudge.
 *  The host answers the same for every place, so this window only says it
 *  sent the ask. */
export async function requestShare(host: string, projectId: string, slug: string, note: string | null): Promise<void> {
  const link = links.get(host);
  if (!link) throw new Error("not connected");
  await link.call<null>("share_request", { project: projectId, slug, ...(note ? { note } : {}) });
  patch(host, (h) => ({ ...h, asked: [...h.asked, `${projectId}|${slug}`] }));
}

/** Stop: group chat's beta switch went off. Every guest connection closes,
 *  the source is forgotten (so the next on starts afresh), and an open guest
 *  view, the hosts' lists and the toasts go, told to every subscriber. The
 *  pairings stay: Settings → Hosts still lists them, and nothing is deleted. */
export function stopGuests(): void {
  for (const l of links.values()) l.close();
  links.clear();
  source = null;
  set(() => ({ hosts: [], toasts: [], open: null }));
}

/** Tests only: forget every host and the source. */
export function resetGuestsForTest() {
  stopGuests();
}
