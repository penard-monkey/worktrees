/** Drag-and-drop semantics for the nav — the PURE half.
 *
 *  Everything here is a function of its arguments: no React, no DOM, no clock
 *  of its own. `app/scripts/dnd-check.mjs` imports this file directly and
 *  checks the rules below, which is only possible because dropping a place on
 *  a tier is decided here rather than inside an event handler.
 *
 *  The one fact the whole feature turns on: **half the tier groups are derived,
 *  not declared.** `store::reconcile` (crates/worktrees-core/src/store.rs:101)
 *  computes `lifecycle_effective` from the declared label AND live tmux state,
 *  so "Active" and "Idle" are readings, not labels — there is nothing to write
 *  when a row is dropped on them. `predictTier` mirrors that function so a drag
 *  can say, BEFORE it commits, which group the row will actually land in. When
 *  that differs from the group under the pointer we open the gap at the real
 *  destination instead of lying about where the row is going. */

/** store.rs:15 — how long after `last_opened_epoch` a place still reads idle. */
export const IDLE_WINDOW_SECS = 7 * 24 * 3600;

export type Tier =
  | "pinned" | "active" | "idle"
  | "saved" | "closed" | "archived" | "abandoned";

/** Labels core's `set_lifecycle` accepts (lib.rs `LIFECYCLE_LABELS`). */
export const LIFECYCLE_LABELS = ["closed", "saved", "archived", "abandoned"] as const;
/** Declared labels reconcile treats as sticky — they win over live tmux. */
const STICKY = new Set(["archived", "abandoned", "saved"]);

export const TIER_LABEL: Record<string, string> = {
  pinned: "Pinned", active: "Active", idle: "Idle",
  saved: "Saved", closed: "Closed", archived: "Archived", abandoned: "Abandoned",
};

export type DeclaredLike = {
  lifecycle?: string;
  pinned?: boolean;
  title?: string;
  last_opened_epoch?: number;
} | null | undefined;

export type PlaceLike = {
  slug: string;
  is_main?: boolean;
  declared?: DeclaredLike;
  tmux_session?: { up: boolean };
};

/** A declared-state edit. `lifecycle: null` CLEARS the label (back to derived);
 *  an absent key means "leave it alone". */
export type DeclPatch = { pinned?: boolean; lifecycle?: string | null };

/** Client-side mirror of `store::reconcile` + `bucketOf` — which group a place
 *  WOULD render in once `patch` lands. Used for the drop preview only; the
 *  rendered tier always remains the server's `lifecycle_effective`. */
export function predictTier(p: PlaceLike, patch: DeclPatch, now: number): Tier {
  const d = p.declared ?? {};
  const pinned = patch.pinned !== undefined ? patch.pinned : !!d.pinned;
  if (pinned) return "pinned";
  const label = patch.lifecycle !== undefined ? patch.lifecycle : d.lifecycle ?? null;
  if (label && STICKY.has(label)) return label as Tier;
  if (p.tmux_session?.up) return "active";
  const t = d.last_opened_epoch;
  if (t && now - t < IDLE_WINDOW_SECS) return "idle";
  return "closed";
}

export type DropPlan =
  /** Nothing to write — `hint` is shown instead of a gap. */
  | { ok: false; hint: string }
  /** `patch` is what to write; `lands` is where the row will REALLY end up,
   *  which is not always `target` (see the module header). */
  | { ok: true; patch: DeclPatch; lands: Tier };

/** What dropping `p` on the `target` group means. `current` is the group it is
 *  rendered in now (`bucketOf`), which decides whether this is a tier change at
 *  all — a drop inside the row's own group is a pure re-order and writes
 *  nothing, so it stays legal even for the derived groups a row cannot be
 *  moved INTO. Reordering the Active group was possible before drag could
 *  change tiers, and losing it would be a regression paid for a new feature. */
export function dropIntent(p: PlaceLike, target: Tier, now: number, current: Tier): DropPlan {
  if (p.is_main) return { ok: false, hint: "the main worktree stays where it is" };
  if (target === current) return { ok: true, patch: {}, lands: target };
  if (target === "active")
    return { ok: false, hint: "Active means a live tmux session — press Enter to open" };
  // Every non-pinned target also UNPINS: `pinned` outranks the lifecycle label
  // in bucketOf, so dropping a pinned row on Archived without clearing the pin
  // would leave it sitting in Pinned — a drop that visibly did nothing.
  const patch: DeclPatch =
    target === "pinned" ? { pinned: true }
      : target === "idle" ? { pinned: false, lifecycle: null }
        : { pinned: false, lifecycle: target };
  return { ok: true, patch, lands: predictTier(p, patch, now) };
}

/** Why a landing tier differs from the group the pointer was over, in words the
 *  row can be flashed with. `null` when they agree. */
export function landingNote(target: Tier, lands: Tier): string | null {
  if (target === lands) return null;
  if (lands === "active")
    return `still Active — the tmux session is running. Close it to reach ${TIER_LABEL[target]}.`;
  if (target === "idle")
    return `Idle is derived from when a place was last opened — this one reads ${TIER_LABEL[lands]}.`;
  return `landed in ${TIER_LABEL[lands]}.`;
}

// ── landing position ─────────────────────────────────────────────────────────
// Where the gap opens inside the target group. Each sort mode answers with the
// position that mode WILL produce, so the gap is a promise the list keeps:
// manual reads the pointer, alpha and recent compute the slot from the value
// they sort by (a gap that follows the pointer in A–Z mode would be a lie).

/** First index whose existing key sorts strictly AFTER `key` — equal keys are
 *  passed over, so an insert lands after the run of its equals, which is where
 *  a stable sort puts a newcomer. */
export function sortedIndex<T>(keys: T[], key: T, cmp: (a: T, b: T) => number): number {
  let i = 0;
  while (i < keys.length && cmp(keys[i], key) <= 0) i++;
  return i;
}

export function alphaIndex(names: string[], name: string, desc: boolean): number {
  const cmp = (a: string, b: string) => (desc ? -1 : 1) * a.localeCompare(b);
  return sortedIndex(names, name, cmp);
}

export function recentIndex(acts: number[], act: number, asc: boolean): number {
  const cmp = (a: number, b: number) => (asc ? a - b : b - a);
  return sortedIndex(acts, act, cmp);
}

/** Pointer → slot. `rects` are the rows of the target group WITHOUT the dragged
 *  row, top-to-bottom, and already put back into their NATURAL positions by
 *  `naturalTop` — see below for why that matters. */
export function pointerIndex(rects: { top: number; height: number }[], y: number): number {
  let i = 0;
  while (i < rects.length && y > rects[i].top + rects[i].height / 2) i++;
  return i;
}

/** The open gap displaces everything below it, so measuring the live DOM asks
 *  "where would this row be if the gap were not there". Without this the gap
 *  chases the pointer: a pointer resting INSIDE the gap sits past the following
 *  row's displaced midpoint, which computes the next slot down, which moves the
 *  gap, which moves the pointer back inside it — a one-frame oscillation.
 *
 *  Pointer and rows are compensated the same way, with the inside-the-gap case
 *  clamped to the gap's own top so the answer there is exactly "here", stable. */
export function naturalTop(top: number, gapTop: number, gapH: number): number {
  return top >= gapTop ? Math.max(gapTop, top - gapH) : top;
}


// ── manual order ─────────────────────────────────────────────────────────────

/** `settings.manual_order[repo]` is ONE flat slug array per repo and every tier
 *  group sorts by it, so a within-group position is just a position in this
 *  array relative to that group's other slugs — inserting before a group's row
 *  k lands at k inside the group whatever else the array holds, and inserting
 *  at the end lands last in the group for the same reason. */
export function normalizeOrder(existing: string[], allSlugs: string[]): string[] {
  const known = new Set(allSlugs);
  const seen = new Set<string>();
  const kept: string[] = [];
  for (const s of existing) {
    if (!known.has(s) || seen.has(s)) continue;
    seen.add(s);
    kept.push(s);
  }
  return [...kept, ...allSlugs.filter((s) => !seen.has(s))];
}

/** Move `moved` directly before `beforeSlug` (or to the end when null). */
export function spliceOrder(
  existing: string[], allSlugs: string[], moved: string, beforeSlug: string | null,
): string[] {
  const order = normalizeOrder(existing, allSlugs);
  // Dropping a row on itself is a no-op, not a move to the end: removing it
  // first makes `indexOf(beforeSlug)` miss, and the -1 fallback is "append".
  if (beforeSlug === moved) return order;
  const from = order.indexOf(moved);
  if (from >= 0) order.splice(from, 1);
  const to = beforeSlug === null ? -1 : order.indexOf(beforeSlug);
  order.splice(to < 0 ? order.length : to, 0, moved);
  return order;
}

/** Generic list move — projects (by root) reuse it. */
export function moveBefore<T>(list: T[], moved: T, before: T | null): T[] {
  const out = list.filter((x) => x !== moved);
  const to = before === null ? -1 : out.indexOf(before);
  out.splice(to < 0 ? out.length : to, 0, moved);
  return out;
}
