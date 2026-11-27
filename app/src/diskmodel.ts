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
 *  `"<dev>:<ino>"`, the identity a delete is checked against. */
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
export function placeTM(repo: string, u: PlaceUsage, withShared: boolean): TM {
  const kids: TM[] = CATS.filter((c) => u.own[c] > 0).map((c) => ({
    key: `${repo}|${u.slug}|${c}`, label: CAT_LABEL[c], value: u.own[c], kind: "leaf", cat: c,
  }));
  if (withShared && u.shared > 0) kids.push({ key: `${repo}|${u.slug}|shared`, label: "Shared", value: u.shared, kind: "leaf", cat: "source", shared: true });
  return { key: `${repo}|${u.slug}`, label: u.slug, value: 0, kind: "place", kids, ref: { repo, slug: u.slug } };
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
): BulkRow[] {
  const rows: BulkRow[] = [];
  for (const [repo, u] of Object.entries(usage)) {
    for (const p of u.places) {
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
