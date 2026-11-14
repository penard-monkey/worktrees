// Which places have a live agent a share can bind (Claude or Codex), for the
// surfaces outside App that need it: the corner's share requests (LaneShare's
// `ShareAlerts`, in EngineGate's RemoteLayer), which say when a request is
// for a place with no live session rather than offer a Share that the host
// would refuse. App is the one writer (`publishLanes`, from each snapshot), as
// it is for betaLive.ts. Until App has a snapshot, every place reads unknown.
import { useSyncExternalStore } from "react";

/** `repo|slug` of every place with a live shareable agent; null: not known. */
let live: ReadonlySet<string> | null = null;
let stamp = "";
const subs = new Set<() => void>();

const keyOf = (repo: string, slug: string) => `${repo}|${slug}`;

/** App: the places whose shareable agent is up now, as `[repo, slug]`. */
export function publishLanes(places: readonly (readonly [string, string])[]): void {
  const keys = places.map(([r, s]) => keyOf(r, s)).sort();
  const next = keys.join("\n");
  if (live !== null && next === stamp) return;
  stamp = next;
  live = new Set(keys);
  for (const f of [...subs]) f();
}

/** Whether `repo`/`slug` has a live shareable agent: null while unknown. */
export function laneLive(set: ReadonlySet<string> | null, repo: string, slug: string): boolean | null {
  return set === null ? null : set.has(keyOf(repo, slug));
}

export function useLanes(): ReadonlySet<string> | null {
  return useSyncExternalStore((f) => { subs.add(f); return () => { subs.delete(f); }; }, () => live);
}
