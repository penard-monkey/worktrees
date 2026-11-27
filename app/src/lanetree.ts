// Lanes started by a lane, drawn (docs/proposals/lane-children.md §5): one
// tier's rows as a tree. The RULE of who hangs from whom is core's
// (`lineage.parent`, resolved on every snapshot row); this module only lays
// out a tier's rows by it. No import: `lanetree-check.mjs` loads it as a
// `data:` module.
//
// One rule at every depth (§5.4): a row is drawn in the tier it is really in,
// with its starters above it, all the way up. A starter that is not a row of
// this tier is drawn as a GHOST: in another tier (its real row is there), or
// in this one but hidden by the filter (§5.6: shown dim, as context).

/** What one tier draws, in render order. */
export type LaneItem<P> = {
  kind: "row" | "ghost";
  place: P;
  slug: string;
  /** The drawn parent's slug, null at the top of the tier. */
  parent: string | null;
  /** 0 at the top of the tier. */
  depth: number;
  /** Rows (real or ghost) are drawn under it in this tier. */
  kids: boolean;
  /** It has kids and they are folded away. Never while a filter is typed. */
  collapsed: boolean;
  /** Collapsed only: the REAL rows it hides in this tier, at every depth. */
  hidden: P[];
  /** Ghost only: the tier its real row is in. */
  tier?: string;
};

export type LaneTreeIn<P> = {
  /** This tier's rows, as the nav shows them (filter applied). */
  rows: readonly P[];
  /** Every place of the project, unfiltered: where a ghost comes from. */
  all: readonly P[];
  slug: (p: P) => string;
  /** `lineage.parent`; undefined at the top. */
  parent: (p: P) => string | undefined;
  /** The tier a place is really in (`bucketOf`). */
  tier: (p: P) => string;
  /** This tier's key, as `tier` answers it. */
  here: string;
  /** Orders one set of siblings. `latest` is the newest activity in the
   *  subtree drawn under each (itself included), for the last-used sort. */
  order: (siblings: P[], latest: (p: P) => number) => P[];
  /** Its own activity time, for `latest`. */
  at: (p: P) => number;
  /** The frontend's fold map: is this slug folded. */
  folded: (slug: string) => boolean;
  /** A filter is typed: fold is ignored (and not written). */
  filtering: boolean;
};

export function laneTree<P>(t: LaneTreeIn<P>): LaneItem<P>[] {
  const bySlug = new Map<string, P>();
  for (const p of t.all) bySlug.set(t.slug(p), p);
  const real = new Set(t.rows.map(t.slug));
  // The parent a row is drawn under: one that exists in this project. A name
  // with no place behind it (the snapshot moved under us) is the top level.
  const raw = (s: string): string | null => {
    const p = bySlug.get(s);
    const u = p ? t.parent(p) : undefined;
    return u && u !== s && bySlug.has(u) ? u : null;
  };
  // Core forbids a cycle; a bad snapshot that has one draws its rows at the
  // top rather than losing them (a row in a loop has no root to hang from).
  const up = (s: string): string | null => {
    const u = raw(s);
    const seen = new Set<string>([s]);
    for (let cur = u; cur; cur = raw(cur)) {
      if (seen.has(cur)) return cur === s ? null : u;
      seen.add(cur);
    }
    return u;
  };
  // Every row and its starters, all the way up.
  const nodes = new Set<string>();
  for (const s of real) {
    const seen = new Set<string>();
    for (let cur: string | null = s; cur && !seen.has(cur); cur = up(cur)) {
      seen.add(cur);
      nodes.add(cur);
    }
  }
  const parentOf = (s: string): string | null => {
    const u = up(s);
    return u && nodes.has(u) ? u : null;
  };
  const kids = new Map<string | null, string[]>();
  for (const s of nodes) {
    const k = parentOf(s);
    (kids.get(k) ?? kids.set(k, []).get(k)!).push(s);
  }
  const latestMemo = new Map<string, number>();
  const latest = (s: string, seen = new Set<string>()): number => {
    const m = latestMemo.get(s);
    if (m !== undefined) return m;
    if (seen.has(s)) return 0;
    seen.add(s);
    let v = real.has(s) ? t.at(bySlug.get(s)!) : 0;
    for (const k of kids.get(s) ?? []) v = Math.max(v, latest(k, seen));
    latestMemo.set(s, v);
    return v;
  };
  const sorted = (parent: string | null): string[] => {
    const sibs = (kids.get(parent) ?? []).map((s) => bySlug.get(s)!);
    return t.order(sibs, (p) => latest(t.slug(p))).map(t.slug);
  };
  const realUnder = (s: string, seen = new Set<string>()): P[] => {
    const out: P[] = [];
    for (const k of kids.get(s) ?? []) {
      if (seen.has(k)) continue;
      seen.add(k);
      if (real.has(k)) out.push(bySlug.get(k)!);
      out.push(...realUnder(k, seen));
    }
    return out;
  };
  const out: LaneItem<P>[] = [];
  const walk = (parent: string | null, depth: number, seen: Set<string>) => {
    for (const s of sorted(parent)) {
      if (seen.has(s)) continue;
      seen.add(s);
      const p = bySlug.get(s)!;
      const has = (kids.get(s)?.length ?? 0) > 0;
      const collapsed = has && !t.filtering && t.folded(s);
      const ghost = !real.has(s);
      out.push({
        kind: ghost ? "ghost" : "row",
        place: p,
        slug: s,
        parent,
        depth,
        kids: has,
        collapsed,
        hidden: collapsed ? realUnder(s) : [],
        ...(ghost ? { tier: t.tier(p) } : {}),
      });
      if (!collapsed) walk(s, depth + 1, seen);
    }
  };
  walk(null, 0, new Set());
  return out;
}

/** The depth stop (§5.2): how far a row is indented, the step, and the level
 *  number a row past the stop shows before its name. */
export const WIDE_NAV = 280;
export function indentOf(depth: number, navWidth: number): { lvl: number; step: number; label: number | null } {
  const wide = navWidth >= WIDE_NAV;
  const stop = wide ? 4 : 3;
  return { lvl: Math.min(depth, stop), step: wide ? 14 : 8, label: depth > stop ? depth : null };
}

/** Where a manual reorder may land a row (§5.6): among its siblings only. The
 *  siblings are the items with the same drawn parent, in render order, minus
 *  the dragged row. `index` is the pointer's slot among them. Answers the
 *  sibling it goes before (for `manual_order`, null = after the last), and the
 *  item the gap is drawn in front of (null = the tier's end): after the last
 *  sibling's whole subtree, not straight under it. */
export function siblingSlot<P>(
  items: readonly LaneItem<P>[], dragged: string, parent: string | null, index: number,
): { before: string | null; gap: string | null } {
  const sibs = items.filter((i) => i.parent === parent && i.slug !== dragged);
  if (index < sibs.length) return { before: sibs[index].slug, gap: sibs[index].slug };
  if (sibs.length === 0) {
    // No sibling left: straight under the parent, or the tier's top.
    const at = parent === null ? -1 : items.findIndex((i) => i.slug === parent);
    const next = items.slice(at + 1).find((i) => i.slug !== dragged);
    return { before: null, gap: next?.slug ?? null };
  }
  const last = sibs[sibs.length - 1];
  const from = items.indexOf(last);
  // The subtree ends at the first later item no deeper than the sibling.
  const end = items.slice(from + 1).find((i) => i.depth <= last.depth && i.slug !== dragged);
  return { before: null, gap: end?.slug ?? null };
}

/** The items a dragged row's subtree occupies, itself first: they travel with it. */
export function subtreeOf<P>(items: readonly LaneItem<P>[], slug: string): string[] {
  const at = items.findIndex((i) => i.slug === slug);
  if (at < 0) return [];
  const out = [slug];
  for (const i of items.slice(at + 1)) {
    if (i.depth <= items[at].depth) break;
    out.push(i.slug);
  }
  return out;
}

/** The remove dialog's sentence (§7): core's `ops::handoff_sentence`, word for
 *  word (`lanetree-check.mjs` reads ops.rs for it). `lanes` is
 *  `lineage.lanes`, `to` is `lineage.up`, else `(main)`. */
export function handoffLine(lanes: readonly string[], to: string): string | null {
  if (lanes.length === 0) return null;
  if (lanes.length === 1) return `1 lane hangs from it (${lanes[0]}). It moves up to ${to}; it is not closed or removed.`;
  return `${lanes.length} lanes hang from it (${lanes.join(", ")}). They move up to ${to}; none is closed or removed.`;
}

/** What a place's header says about its line (§8's table), from the
 *  snapshot's `lineage` and `declared.under`. Structural types, so this file
 *  keeps no import. */
type LinkLike = { place: string; at: number; by?: string };
type StarterLike = { at: number; elsewhere: true } | { place: string; at: number; project?: string; removed?: boolean };
export function laneLines(
  lin: { starter?: StarterLike; under?: LinkLike; handed_to?: LinkLike; parent?: string; reports_to?: string; started?: string[] } | undefined,
  closed: (slug: string) => boolean,
  date: (at: number) => string,
): string[] {
  if (!lin) return [];
  const out: string[] = [];
  const s = lin.starter;
  if (s && "elsewhere" in s) out.push("started in another project");
  else if (s?.project) out.push(`from ${s.project}: ${s.place}`);
  else if (s) out.push(`started by ${s.place}${s.removed ? " (removed)" : closed(s.place) ? " (closed)" : ""}`);
  const u = lin.under;
  if (u) {
    const who = u.by === "owner" ? "the owner" : u.by ?? "the owner";
    out.push(u.place === "(main)" ? `moved to top level by ${who}, ${date(u.at)}` : `moved under ${u.place} by ${who}, ${date(u.at)}`);
  }
  const h = lin.handed_to;
  if (h && !u) out.push(h.place === "(main)" ? "now at the top level" : `now under ${h.place}`);
  if (lin.reports_to && lin.reports_to !== (lin.parent ?? "(main)")) out.push(`reports to ${lin.reports_to}`);
  const n = lin.started?.length ?? 0;
  if (n) out.push(`started ${n} lane${n === 1 ? "" : "s"}`);
  return out;
}
