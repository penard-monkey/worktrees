// Disk usage: the engine's answers (`worktrees_core::diskusage`) and the pure
// rules the Disk view draws with. No DOM here: `disk-check.mjs` loads this
// file as a module and tests the layout, the trees and the cleanup defaults.

export type Cat = "build" | "deps" | "source" | "git";
export const CATS: Cat[] = ["build", "deps", "source", "git"];
export const CAT_LABEL: Record<Cat, string> = {
  build: "Build output",
  deps: "Dependencies",
  source: "Source and other files",
  git: "Git",
};
export type ByCat = Record<Cat, number>;
export type Cleanable = { path: string; cat: Cat; own: number; shared: number; files: number; id: string };
export type PlaceUsage = {
  slug: string; path: string; is_main: boolean; own: ByCat; shared: number; alloc: number;
  files: number; errors: number; cleanable: Cleanable[];
};
export type Volume = { free: number; total: number; snapshots: number | null };
export type ProjectUsage = { root: string; scanned_epoch: number; scan_ms: number; places: PlaceUsage[]; unique: number; volume: Volume };
/** `diskusage::Node`: `n` empty = small entries folded together; `id` is
 *  `"<dev>:<ino>:b<ns>"` (or `c<ns>`, a ctime), opaque here: the identity a
 *  delete is checked against (`diskusage::Ident`). */
export type DNode = { n: string; p?: string; own: number; shared: number; files: number; c: Cat; dir?: boolean; id?: string; k?: DNode[] };
export type PlaceTree = { usage: PlaceUsage; tree: DNode; scanned_epoch: number; scan_ms: number; volume: Volume };
export type Deleted = { path: string; bytes: number; files: number; dir: boolean };

export type DiskScope =
  | { kind: "all" }
  | { kind: "workspace"; id: string }
  | { kind: "project"; root: string }
  | { kind: "place"; repo: string; slug: string };

export const sameScope = (a: DiskScope | null, b: DiskScope | null): boolean => JSON.stringify(a) === JSON.stringify(b);

export const ownTotal = (o: ByCat): number => o.build + o.deps + o.source + o.git;

/** Decimal units, as Finder says them. */
export function gb(v: number): string {
  if (v >= 1e12) return `${(v / 1e12).toFixed(1)} TB`;
  if (v >= 1e11) return `${Math.round(v / 1e9)} GB`;
  if (v >= 1e9) return `${(v / 1e9).toFixed(1)} GB`;
  if (v >= 1e6) return `${Math.round(v / 1e6)} MB`;
  if (v > 0) return "<1 MB";
  return "0 MB";
}

export function ago(nowSec: number, epoch: number | null | undefined): string {
  if (!epoch) return "no commits";
  const d = Math.floor((nowSec - epoch) / 86400);
  return d < 1 ? "today" : d === 1 ? "1 day ago" : `${d} days ago`;
}

/** How long ago a scan was taken, in words a header can carry. */
export function scannedAgo(nowSec: number, epoch: number): string {
  const s = Math.max(0, nowSec - epoch);
  if (s < 60) return "just now";
  const m = Math.floor(s / 60);
  if (m < 60) return m === 1 ? "1 minute ago" : `${m} minutes ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return h === 1 ? "1 hour ago" : `${h} hours ago`;
  const d = Math.floor(h / 24);
  return d === 1 ? "1 day ago" : `${d} days ago`;
}

// ── what a kept scan no longer matches ─────────────────────────────────────
//
// A scan is kept until Rescan (`diskusage::project_usage`). It records, per
// project, WHEN (`scanned_epoch`) and WHICH places (`places[].slug`); the live
// workspace snapshot says which places exist now. Comparing the two names is
// certain and costs nothing: a slug in the scan and not in the snapshot was
// removed after it, a slug in the snapshot and not in the scan was added. What
// it cannot see: a place removed and made again under the same name, and bytes
// that changed inside a place that is still there (a build, a delete by hand).
// Those only a walk can find, and nothing here walks.

/** A project's kept scan against the places that exist now. */
export type StaleProject = { root: string; gone: string[]; added: string[] };

export function staleProject(u: ProjectUsage, live: { slug: string }[]): StaleProject {
  const now = new Set(live.map((p) => p.slug));
  const had = new Set(u.places.map((p) => p.slug));
  return {
    root: u.root,
    gone: u.places.filter((p) => !now.has(p.slug)).map((p) => p.slug),
    added: live.filter((p) => !had.has(p.slug)).map((p) => p.slug),
  };
}

/** One screen's staleness: every scanned project's, plus the projects in
 *  scope that have no scan. */
export type Stale = { gone: { root: string; slug: string }[]; added: { root: string; slug: string }[]; unscanned: string[] };

export function staleOf(
  roots: string[],
  usage: Record<string, ProjectUsage>,
  live: (root: string) => { slug: string }[] | null,
): Stale {
  const out: Stale = { gone: [], added: [], unscanned: [] };
  for (const r of roots) {
    const u = usage[r];
    if (!u) { out.unscanned.push(r); continue; }
    const now = live(r);
    if (!now) continue;
    const s = staleProject(u, now);
    out.gone.push(...s.gone.map((slug) => ({ root: r, slug })));
    out.added.push(...s.added.map((slug) => ({ root: r, slug })));
  }
  return out;
}

export const isGone = (st: Stale, root: string, slug: string): boolean => st.gone.some((g) => g.root === root && g.slug === slug);

/** The notice's sentences, or none when the scan still matches. Short
 *  sentences, no dashes; a place is named while there is one. */
export function staleWords(st: Stale, projectName: (root: string) => string): string[] {
  const w: string[] = [];
  if (st.gone.length === 1) w.push(`${st.gone[0].slug} was removed after this scan. These numbers still include it.`);
  else if (st.gone.length > 1) w.push(`${st.gone.length} places were removed after this scan. These numbers still include them.`);
  if (st.added.length === 1) w.push(`${st.added[0].slug} was added after this scan and is not counted.`);
  else if (st.added.length > 1) w.push(`${st.added.length} places were added after this scan and are not counted.`);
  if (st.unscanned.length === 1) w.push(`${projectName(st.unscanned[0])} has not been scanned and is not counted.`);
  else if (st.unscanned.length > 1) w.push(`${st.unscanned.length} projects have not been scanned and are not counted.`);
  return w;
}

/** The oldest and newest scan among `us`, as one header phrase. */
export function scansAgo(nowSec: number, us: ProjectUsage[]): string | null {
  if (!us.length) return null;
  const eps = us.map((u) => u.scanned_epoch);
  const old = scannedAgo(nowSec, Math.min(...eps));
  const neu = scannedAgo(nowSec, Math.max(...eps));
  return old === neu ? `scanned ${old}` : `scanned ${old} to ${neu}`;
}

// ── the treemap's tree ─────────────────────────────────────────────────────

/** One rectangle's worth: a group (workspace or project), a place, a folder or
 *  a leaf. `box` draws a dashed outline (a workspace, (main)'s places). */
export type TM = {
  key: string;
  label: string;
  value: number;
  kind: "group" | "place" | "dir" | "leaf";
  cat?: Cat;
  shared?: boolean;
  box?: boolean;
  /** A place removed after the scan: drawn, but not opened. */
  gone?: boolean;
  kids?: TM[];
  ref?: { repo?: string; slug?: string; path?: string; id?: string; dir?: boolean; root?: string };
};

export const tmValue = (t: TM): number => (t.kids ? t.kids.reduce((a, k) => a + tmValue(k), 0) : t.value);

/** The node a picked path names, or null once it is gone (deleted, rescanned
 *  away). The pick lives in the history location as a path, not a node, so
 *  Back finds it again in whatever tree is drawn now. */
export function findPath(t: TM, path: string): TM | null {
  if (t.ref?.path === path) return t;
  for (const k of t.kids ?? []) { const f = findPath(k, path); if (f) return f; }
  return null;
}

/** A place as category leaves: its own bytes, and (with `shared`) the bytes
 *  it shares as one hatched leaf. */
export function placeTM(repo: string, u: PlaceUsage, withShared: boolean, gone = false): TM {
  const kids: TM[] = CATS.filter((c) => u.own[c] > 0).map((c) => ({
    key: `${repo}|${u.slug}|${c}`, label: CAT_LABEL[c], value: u.own[c], kind: "leaf", cat: c,
  }));
  if (withShared && u.shared > 0) kids.push({ key: `${repo}|${u.slug}|shared`, label: "Shared", value: u.shared, kind: "leaf", cat: "source", shared: true });
  return { key: `${repo}|${u.slug}`, label: u.slug, value: 0, kind: "place", kids, ref: { repo, slug: u.slug }, ...(gone ? { gone } : {}) };
}

/** The engine's pruned tree as treemap nodes: a folder's children, or one
 *  leaf per (category, shared) for what was folded. */
export function treeTM(n: DNode, withShared: boolean, key = ""): TM {
  if (n.dir && n.k && n.k.length) {
    const kids = n.k
      .map((c, i) => treeTM(c, withShared, `${key}/${c.n || "·" + i}`))
      .filter((t) => tmValue(t) > 0);
    return { key: key || "/", label: n.n, value: 0, kind: "dir", cat: n.c, kids, ref: { path: n.p, id: n.id, dir: true } };
  }
  const v = n.own + (withShared ? n.shared : 0);
  return {
    key: key || "/", label: n.n, value: v, kind: "leaf", cat: n.c, shared: n.own === 0 && n.shared > 0,
    ref: n.p ? { path: n.p, id: n.id, dir: !!n.dir } : undefined,
  };
}

/** Squarified layout (Bruls, Huizing, van Wijk 2000): rows whose worst aspect
 *  ratio stops improving are closed. Returns [index, x, y, w, h] per item with
 *  a value above zero, in the order given (sort largest first for squares). */
export function squarify(values: number[], x: number, y: number, w: number, h: number): [number, number, number, number, number][] {
  const out: [number, number, number, number, number][] = [];
  const items = values.map((v, i) => [i, v] as const).filter(([, v]) => v > 0);
  const total = items.reduce((a, [, v]) => a + v, 0);
  if (!total || w <= 0 || h <= 0) return out;
  const scale = (w * h) / total;
  let i = 0;
  while (i < items.length) {
    const side = Math.min(w, h);
    const row: (readonly [number, number])[] = [];
    let sum = 0;
    let best = Infinity;
    while (i < items.length) {
      const a = items[i][1] * scale;
      const ns = sum + a;
      const areas = [...row.map((r) => r[1] * scale), a];
      const mx = Math.max(...areas), mn = Math.min(...areas);
      const worst = Math.max((side * side * mx) / (ns * ns), (ns * ns) / (side * side * mn));
      if (worst > best) break;
      best = worst;
      row.push(items[i++]);
      sum = ns;
    }
    const thick = sum / side;
    let off = 0;
    for (const [idx, v] of row) {
      const len = (v * scale) / thick;
      out.push(w >= h ? [idx, x, y + off, thick, len] : [idx, x + off, y, len, thick]);
      off += len;
    }
    if (w >= h) { x += thick; w -= thick; } else { y += thick; h -= thick; }
  }
  return out;
}

// ── cleanup ────────────────────────────────────────────────────────────────

/** A place untouched this long, with no agent working, is ticked by default. */
export const IDLE_DAYS = 14;

export type BulkRow = {
  repo: string; project: string; slug: string; path: string; is_main: boolean;
  last: number | null; busy: boolean; items: Cleanable[]; frees: number;
};

/** The places with build output worth offering, largest first: every
 *  `build` folder the scan named, never dependencies (a reinstall is slower
 *  and can change versions), never git or source. */
export function bulkRows(
  usage: Record<string, ProjectUsage>,
  names: Record<string, string>,
  info: (repo: string, slug: string) => { last: number | null; busy: boolean },
  min = 100e6,
  gone: (repo: string, slug: string) => boolean = () => false,
): BulkRow[] {
  const rows: BulkRow[] = [];
  for (const [repo, u] of Object.entries(usage)) {
    for (const p of u.places) {
      // A place removed after the scan has nothing left to delete.
      if (gone(repo, p.slug)) continue;
      const items = p.cleanable.filter((c) => c.cat === "build" && c.own > 0);
      const frees = items.reduce((a, c) => a + c.own, 0);
      if (frees < min) continue;
      const { last, busy } = info(repo, p.slug);
      rows.push({ repo, project: names[repo] ?? repo.split("/").pop() ?? repo, slug: p.slug, path: p.path, is_main: p.is_main, last, busy, items, frees });
    }
  }
  return rows.sort((a, b) => b.frees - a.frees);
}

/** Ticked by default: idle for `IDLE_DAYS` and no agent working. (main)
 *  follows the same rule; its build output is as rebuildable as a lane's. */
export function bulkDefault(r: { last: number | null; busy: boolean }, nowSec: number): boolean {
  if (r.busy) return false;
  if (r.last == null) return false;
  return (nowSec - r.last) / 86400 >= IDLE_DAYS;
}
