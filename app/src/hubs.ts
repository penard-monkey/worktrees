// The hub dashboard's data (`sync_hubs`, core's `synchub::Dashboard`) and the
// pure rules the dashboard and the sync modals share: which hubs a push goes
// to by default, which copy a pull takes, and whether a project changed here
// since its last push. No imports, so a check script can load it as is.

export type HubKind = "removable" | "internal" | "network" | "folder";
/** One project on one hub; `here` is the root of this machine's project of
 *  the same name (the hub slot is the root's basename). */
export type HubProj = { name: string; local_root: string; pushed_at: number; host: string; here: string | null };
export type Hub = {
  /** `uuid:<VOLUME-UUID>`, `smb://host/share`, or `path:<p>` — the ONLY way
   *  the backend accepts a hub; a path is never sent. */
  id: string;
  name: string;
  kind: HubKind;
  /** Mounted path; null when not connected. */
  path: string | null;
  connected: boolean;
  /** Position in the chosen list (0 = primary), or null. */
  chosen: number | null;
  /** `[sync] hub` / `$WORKTREES_SYNC_HUB` — outranks the choice. */
  configured: boolean;
  /** What the CLI would auto-detect with nothing chosen. */
  auto: boolean;
  last_seen: number | null;
  free_bytes: number | null;
  total_bytes: number | null;
  projects: HubProj[];
  /** `projects` is the last-known list (not connected). */
  stale: boolean;
};
export type LocalProj = {
  name: string;
  root: string;
  branch: string | null;
  dirty: number | null;
  head_at: number | null;
  /** Non-null ⇒ this checkout arrived on a hub: it is never pushed. */
  hub_copy: string | null;
};
export type HubDash = { hubs: Hub[]; local: LocalProj[]; default_error: string | null };
/** One step of a batch: a project (by `repo` here, by `name` for an import)
 *  and the hub, by id. */
export type SyncJob = { repo: string | null; name: string | null; hub: string; confirmed?: boolean };
/** A project as the batch addresses it: a root here, or only a name on a hub. */
export type SyncItem = { name: string; repo: string | null };

export const connectedHubs = (d: HubDash | null): Hub[] => (d?.hubs ?? []).filter((h) => h.connected);

/** The chosen ids in chosen order (connected or not). */
export function chosenIds(d: HubDash | null): string[] {
  return (d?.hubs ?? [])
    .filter((h) => h.chosen !== null)
    .sort((a, b) => (a.chosen ?? 0) - (b.chosen ?? 0))
    .map((h) => h.id);
}

/** A push's default targets: every CHOSEN hub that is connected; with none
 *  chosen, the one auto-detect would take, or the only connected hub. Never a
 *  guess between several unchosen ones — the user ticks those. */
export function defaultPushHubs(d: HubDash | null): string[] {
  const conn = connectedHubs(d);
  const chosen = chosenIds(d).filter((id) => conn.some((h) => h.id === id));
  if (chosen.length) return chosen;
  const auto = conn.find((h) => h.auto) ?? (conn.length === 1 ? conn[0] : undefined);
  return auto ? [auto.id] : [];
}

/** Connected hubs that hold `name`, newest push first. */
export function holders(d: HubDash | null, name: string): { hub: Hub; p: HubProj }[] {
  const out: { hub: Hub; p: HubProj }[] = [];
  for (const hub of connectedHubs(d)) {
    const p = hub.projects.find((x) => x.name === name);
    if (p) out.push({ hub, p });
  }
  return out.sort((a, b) => b.p.pushed_at - a.p.pushed_at);
}

/** A pull's default source: the NEWEST connected copy. */
export const newestHub = (d: HubDash | null, name: string): string | null => holders(d, name)[0]?.hub.id ?? null;

/** The newest push of `name` on any hub (connected or remembered), or 0. */
export function lastPush(d: HubDash | null, name: string): number {
  let at = 0;
  for (const h of d?.hubs ?? []) for (const p of h.projects) if (p.name === name && p.pushed_at > at) at = p.pushed_at;
  return at;
}

/** Changed here since its newest push: dirty paths, or a commit after it.
 *  Null when it was never pushed (there is nothing to be behind). */
export function changedSincePush(d: HubDash | null, l: LocalProj): boolean | null {
  const at = lastPush(d, l.name);
  if (!at) return null;
  return (l.dirty ?? 0) > 0 || (l.head_at ?? 0) > at;
}

/** Every project name the dashboard shows: this machine's, then hub-only ones. */
export function projectNames(d: HubDash | null): string[] {
  const local = (d?.local ?? []).map((l) => l.name);
  const hub = new Set<string>();
  for (const h of d?.hubs ?? []) for (const p of h.projects) if (!local.includes(p.name)) hub.add(p.name);
  return [...local, ...[...hub].sort()];
}

/** `free_bytes` as a person reads it. */
export function bytes(n: number | null): string {
  if (n == null) return "";
  if (n >= 1e12) return `${(n / 1e12).toFixed(1)} TB`;
  if (n >= 1e9) return `${Math.round(n / 1e9)} GB`;
  return `${Math.round(n / 1e6)} MB`;
}

/** The batch's jobs. A push: every item × every ticked hub (an item with no
 *  root here cannot be pushed and is left out). A pull: every item from its
 *  picked hub (an item with none is left out). */
export function batchJobs(dir: "push" | "pull", items: SyncItem[], pushHubs: string[], pullFrom: Record<string, string>): SyncJob[] {
  if (dir === "push") {
    return items.filter((i) => i.repo).flatMap((i) => pushHubs.map((hub) => ({ repo: i.repo, name: null, hub })));
  }
  return items.filter((i) => pullFrom[i.name]).map((i) => ({ repo: i.repo, name: i.repo ? null : i.name, hub: pullFrom[i.name] }));
}
