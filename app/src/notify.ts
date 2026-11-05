// Notify while the window is hidden (settings-reorg §6 item 9): a system
// notification when an agent starts WAITING on you, and — in "both" — when
// one FINISHES after the window has been away a while. The afterglow dot only
// works if you are looking; this is for when you are not.
//
// The DECISION, pure and import-free (`notify-check.mjs` loads it as a data:
// module): which paths a `sessions:busy` snapshot or a `sessions:done` event
// should announce. Sending is App's (the notification plugin), and so is
// asking macOS for permission — only from the Settings control, an act.
//
// Never twice for one thing, the rule a REMOTE window needs most:
// - the first snapshot a window sees is a baseline, never news: a reload, or
//   the snapshot serve sends a socket the moment it declares the event
//   (`scope::SNAPSHOT`), would otherwise announce everything already waiting;
// - the last waiting set is kept across a reconnect, so the replayed snapshot
//   a new socket gets is diffed against it, not against nothing;
// - a place is announced at most once per kind in `RENOTIFY_MS`, so a set that
//   flickers (a socket that drops and comes back) cannot repeat it.
// Each window decides for itself, on the machine it runs on: a window driving
// a paired host notifies on THIS machine, about that host's agents.

export const NOTIFY_MODES = ["off", "waiting", "both"] as const;
export type NotifyMode = (typeof NOTIFY_MODES)[number];
export const isNotifyMode = (v: unknown): v is NotifyMode => typeof v === "string" && (NOTIFY_MODES as readonly string[]).includes(v);

/** How long the window must have been hidden before a FINISHED turn is news:
 *  a glance at another app and back is not being away. */
export const DONE_AFTER_HIDDEN_MS = 2 * 60_000;
/** One notification per place and kind in this long. */
export const RENOTIFY_MS = 10 * 60_000;

export type Notifier = {
  /** The waiting set the last snapshot held. */
  waiting: Set<string>;
  /** A snapshot has been seen (the baseline). */
  primed: boolean;
  /** When the window was last hidden; null while visible. */
  hiddenSince: number | null;
  /** `kind|path` → when it was last announced. */
  last: Map<string, number>;
};

export const newNotifier = (): Notifier => ({ waiting: new Set(), primed: false, hiddenSince: null, last: new Map() });

/** The window was hidden or shown at `now`. */
export function onVisibility(n: Notifier, hidden: boolean, now: number): void {
  n.hiddenSince = hidden ? (n.hiddenSince ?? now) : null;
}

const due = (n: Notifier, key: string, now: number): boolean => {
  const t = n.last.get(key);
  if (t !== undefined && now - t < RENOTIFY_MS) return false;
  n.last.set(key, now);
  return true;
};

/** A `sessions:busy` snapshot's waiting set: the paths that just STARTED
 *  waiting, to announce — none while the window is visible, none in "off",
 *  none from the baseline. */
export function onWaitingSnapshot(n: Notifier, waiting: readonly string[], now: number, mode: NotifyMode, hidden: boolean): string[] {
  const next = new Set(waiting);
  const fresh = [...next].filter((p) => !n.waiting.has(p));
  n.waiting = next;
  if (!n.primed) {
    n.primed = true;
    return [];
  }
  if (mode === "off" || !hidden) return [];
  return fresh.filter((p) => due(n, `waiting|${p}`, now));
}

/** A `sessions:done` event: announce it? Only in "both", only while hidden,
 *  and only once the window has been hidden `DONE_AFTER_HIDDEN_MS`. */
export function onDone(n: Notifier, path: string, now: number, mode: NotifyMode, hidden: boolean): boolean {
  if (mode !== "both" || !hidden || n.hiddenSince === null || now - n.hiddenSince < DONE_AFTER_HIDDEN_MS) return false;
  return due(n, `done|${path}`, now);
}
