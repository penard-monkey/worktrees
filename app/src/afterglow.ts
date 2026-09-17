// ── afterglow: the arithmetic of the purple dot ──────────────────────────────
// "Claude finished a task here", decaying out. The dot keys off `workedAt(p)`
// and nothing else — never an open, never a commit — so the nav answers "what
// moved recently" at a glance after the busy dot has gone.
//
// It also owns the UNREAD rule. An afterglow decays by wall clock whether or
// not anyone looked, which is the right answer for "how long ago" and the wrong
// one for "is there something here for me". `doneTierSeen` holds a finish the
// user has not seen at tier 1 — past the horizon, forever if need be — and the
// moment it IS seen the dot drops to its REAL age rather than restarting a
// clock. Mail semantics, and the arithmetic lives here with everything else the
// dot is made of.
//
// This module is PURE and React-free on purpose: App.tsx renders the tiers, the
// Settings sheet prints the boundaries it is about to write, and
// `scripts/afterglow-check.mjs` drives the real functions under node. Three
// consumers of one rule; a paraphrase in any of them is a dot that lies.
//
// The decay is DISCRETE, not a continuous fade, for three reasons that survive
// the boundaries becoming a setting: absolute opacity is unreadable on its own
// (only the contrast BETWEEN rows carries), a CSS animation long enough to
// cover a horizon of hours is frozen at frame 0 by `prefers-reduced-motion`
// (tokens.css) — full brightness forever, the exact inverse of the signal — and
// steps are assertable with getComputedStyle instead of racing an in-flight
// animation.
//
// The spacing is GEOMETRIC. Linear steps over a 12h horizon put every boundary
// in the "hours ago" region and leave the first quarter-hour — the only window
// in which the dot means "go look now" — sharing a step with lunch. A constant
// ratio gives the fresh end the resolution and lets the tail be coarse.

/** The first boundary is PINNED, not derived. Tier 1 is the "go look" signal:
 *  it wears the ring, and it is the only tier the project-folder rollup lights
 *  on (App.tsx). Both of those are statements about a quarter of an hour, so a
 *  user stretching the horizon to a week must not silently stretch them too.
 *
 *  Tier 1 is reached two ways, and that is deliberate: a finish is "go look"
 *  while it is FRESH, and also while it is UNSEEN, however old it is (see
 *  `doneTierSeen`). Age alone was the whole rule until unread existed. */
export const DONE_FIRST_SECS = 15 * 60;

/** The horizon slider's stops. A TABLE, not a linear range, for the same reason
 *  `ZOOM_STEPS` is one: the useful moves are at the short end, and a linear
 *  range over a week spends most of its travel on differences nobody can see. */
export const DONE_HORIZONS = [
  3600, // 1h
  2 * 3600,
  4 * 3600,
  8 * 3600,
  12 * 3600, // the default — one working day, overnight
  24 * 3600,
  48 * 3600,
  72 * 3600,
  7 * 86400,
] as const;

export const DONE_STEPS_MIN = 2;
export const DONE_STEPS_MAX = 6;

/** Nearest legal horizon. Also the guard against a hand-edited ui-state.json:
 *  a NaN reaching `doneBounds` is a nav full of `opacity: NaN` dots. */
export function snapHorizon(v: number): number {
  const def = DONE_HORIZONS[4] as number;
  if (!Number.isFinite(v)) return def;
  const lo = DONE_HORIZONS[0] as number;
  const hi = DONE_HORIZONS[DONE_HORIZONS.length - 1] as number;
  const bound = Math.max(lo, Math.min(hi, v));
  return DONE_HORIZONS.reduce(
    (best, s) => (Math.abs(s - bound) < Math.abs(best - bound) ? s : best),
    DONE_HORIZONS[0] as number,
  );
}

/** Steps, clamped to a count the geometry can actually express. Below 2 there
 *  is no ratio; above 6 the opacity difference between neighbours drops under
 *  what the eye separates on an 8px dot. */
export function clampSteps(v: number): number {
  if (!Number.isFinite(v)) return 3;
  return Math.max(DONE_STEPS_MIN, Math.min(DONE_STEPS_MAX, Math.round(v)));
}

/** The tier boundaries, in seconds, ascending: `b[i]` is the age at which the
 *  dot leaves tier `i + 1`. Always `steps` long, always starting at
 *  `DONE_FIRST_SECS` and ending at the horizon, geometric in between:
 *
 *      b[i] = FIRST * (H / FIRST) ^ (i / (steps - 1))
 *
 *  Inputs are snapped/clamped rather than trusted, so garbage in ui-state.json
 *  yields the default curve instead of a zero-length array or a NaN. */
export function doneBounds(horizonSecs: number, steps: number): number[] {
  const h = snapHorizon(horizonSecs);
  const n = clampSteps(steps);
  const out: number[] = [];
  for (let i = 0; i < n; i++) {
    out.push(Math.round(DONE_FIRST_SECS * Math.pow(h / DONE_FIRST_SECS, i / (n - 1))));
  }
  // The ends are exact by definition, not by floating-point luck: the rollup
  // and the horizon label both compare against them.
  out[0] = DONE_FIRST_SECS;
  out[n - 1] = h;
  return out;
}

/** Which tier an epoch is in at `nowSec`: 0 = off (no stamp, or older than the
 *  last bound), otherwise 1..steps — the index of the first bound the age is
 *  under. A negative age (clock skew, a stamp from the future) lands in tier 1,
 *  which is what it did before this was configurable. */
export function doneTier(epoch: number, nowSec: number, bounds: number[]): number {
  if (!epoch || !bounds.length) return 0;
  const age = nowSec - epoch;
  for (let i = 0; i < bounds.length; i++) if (age < bounds[i]) return i + 1;
  return 0;
}

/** The moment the user last SAW this place, from the two stamps that can say so.
 *  `last_opened_epoch` is the FALLBACK, not a second answer: it predates
 *  `last_seen_epoch` by a release, so on first launch after the upgrade a place
 *  entered since its last finish reads as seen instead of shouting. Entering is
 *  also a strictly stronger "I looked" than selecting, so it can never be wrong
 *  to count it — only too generous, and only once. */
export function seenEpoch(seen?: number, opened?: number): number {
  return Math.max(seen ?? 0, opened ?? 0);
}

/** "Claude finished here and the user has not looked since." Needs a real
 *  `worked` — a place that never worked has nothing to be unread ABOUT, and
 *  `0 > 0` would otherwise be the only thing stopping every empty place from
 *  lighting up. Equality is SEEN: the ack stamps at or after the completion it
 *  acknowledges, so `>` is what makes the ack idempotent. */
export function isUnread(worked: number, seen: number): boolean {
  return worked > 0 && worked > seen;
}

/** The tier the dot actually renders: `doneTier`, except that an unread finish
 *  is pinned to 1. Note what this does NOT do — it does not touch `worked`. A
 *  finish seen for the first time three hours late lands on the tier its real
 *  age earns (dim, maybe off), not at the top of a restarted decay. The signal
 *  that was waiting to be delivered is spent by being read, and what is left is
 *  the honest "how long ago". */
export function doneTierSeen(
  worked: number,
  seen: number,
  nowSec: number,
  bounds: number[],
): number {
  return isUnread(worked, seen) ? 1 : doneTier(worked, nowSec, bounds);
}

/** The dot's opacity for a tier. Linear from 1 down to 0.3 across the steps —
 *  with the default 3 steps that is exactly the 1 / .65 / .3 the CSS used to
 *  hardcode. Rounded to 3dp so the value is a number a check script and a
 *  computed style can both state (1 - 0.7 is 0.30000000000000004 in IEEE). */
export function doneOpacity(tier: number, steps: number): number {
  const n = clampSteps(steps);
  if (n < 2) return 1;
  const t = Math.max(1, Math.min(n, Math.round(tier)));
  return Math.round((1 - (0.7 * (t - 1)) / (n - 1)) * 1000) / 1000;
}

/** A compact duration for the UI: "15m", "1h 44m", "12h", "2d", "7d". Zero
 *  parts are dropped, and the smallest shown unit is ROUNDED, not truncated —
 *  a boundary at 6235s reads as "1h 44m", which is what it is nearer to. */
export function fmtSecs(s: number): string {
  if (!Number.isFinite(s) || s <= 0) return "0m";
  const secs = Math.round(s);
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) {
    const m = Math.round(secs / 60);
    return m === 60 ? "1h" : `${m}m`; // 3599s rounds up out of its own unit
  }
  if (secs < 86400) {
    let h = Math.floor(secs / 3600);
    let m = Math.round((secs - h * 3600) / 60);
    if (m === 60) { h += 1; m = 0; }
    if (h === 24) return "1d"; // …and so can 86399s
    return m ? `${h}h ${m}m` : `${h}h`;
  }
  let d = Math.floor(secs / 86400);
  let h = Math.round((secs - d * 86400) / 3600);
  if (h === 24) { d += 1; h = 0; }
  return h ? `${d}d ${h}h` : `${d}d`;
}
