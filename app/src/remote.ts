// Which page on the origin remote a place or a project opens at. Pure on
// purpose (no React, no invoke): `app/scripts/remote-check.mjs` imports it the
// way dnd-check.mjs imports dnd.ts. The backend (`remote_url` in lib.rs) only
// turns the remote SPEC into an https base; this is the one place that decides
// what to append to it.
//
// The rule uses `Place.upstream`, never the local branch name: the snapshot
// sets `upstream` only when its remote-tracking ref RESOLVES (project.rs), so
// `origin/x` means the branch exists on origin and `/tree/x` is a page. The
// old `github_url` built `/tree/<local branch>` unconditionally and 404'd on
// every branch that had not been pushed yet — the repo home is the better
// answer there. Only github.com gets the `/tree/` path; other hosts spell it
// differently and get the home.

/** A `Place`'s remote-facing fields, narrowed so the check script can build
 *  one without the whole record. */
export type RemotePlace = { upstream?: string | null };

/** The web page for `place` on a repo whose origin normalises to `base`. */
export function remoteWebUrl(base: string, place: RemotePlace | null): string {
  const up = place?.upstream ?? null;
  if (!up || !base.startsWith("https://github.com/")) return base;
  const branch = up.startsWith("origin/") ? up.slice("origin/".length) : null;
  if (!branch) return base;
  return `${base}/tree/${branch.split("/").map(encodeURIComponent).join("/")}`;
}

/** What to call the host in a menu item: "GitHub" for github.com, else the bare
 *  host (`gitlab.internal`, `gitea.local`). `null` base → "GitHub", the item's
 *  default before the remote has been read. */
export function remoteHostLabel(base: string | null | undefined): string {
  if (!base) return "GitHub";
  const host = /^https?:\/\/([^/]+)/.exec(base)?.[1] ?? "";
  if (host === "github.com") return "GitHub";
  return host || "remote";
}

/** Tooltip text: the URL without its scheme, which is what a person recognises. */
export function remoteTitle(url: string): string {
  return url.replace(/^https?:\/\//, "");
}
