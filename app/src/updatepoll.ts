// The release poll, and the rule for the update bubble.
//
// It used to be one check ~3s after launch, so an app left open for a week
// never heard of a release made on day two. Now it is a schedule:
//
//   - first check UPDATE_FIRST_MS after start (off the startup path, as before);
//   - then hourly, measured from when the previous check FINISHED;
//   - a failed check (offline, GitHub down) retries sooner and backs off —
//     5, 10, 20, 40 min, then hourly — so waking up on a new network finds a
//     release within minutes without a dead network costing more than the
//     steady cadence does;
//   - never while the window is hidden. A check that comes due while hidden is
//     HELD, and runs the moment the window is visible again — so "back after a
//     night away" checks at once, and flapping visibility inside the hour
//     checks nothing;
//   - never two at once. `check_update` shells out to curl with a 6s cap plus a
//     CLI probe; a slow one must not be joined by the timer or a visibility
//     change asking again.
//
// The check itself is the existing `check_update`: a HEAD of
// github.com/<repo>/releases/latest and a read of the redirect — no API, no
// token, no rate-limit bucket. Once an hour is far inside anything GitHub
// throttles for an unauthenticated page.
//
// IMPORT-FREE on purpose: `scripts/updatepoll-check.mjs` evaluates this file
// as a data: URL module on a virtual clock, and a data: URL has no base to
// resolve `./x` against. Everything time- or DOM-shaped comes in through `env`.

export const UPDATE_FIRST_MS = 3_000;
export const UPDATE_EVERY_MS = 60 * 60_000;
export const UPDATE_RETRY_MS = 5 * 60_000;

export type PollEnv = {
  now: () => number;
  setTimeout: (fn: () => void, ms: number) => unknown;
  clearTimeout: (t: unknown) => void;
  visible: () => boolean;
  /** Subscribe to visibility changes; returns the unsubscribe. */
  onVisible: (fn: () => void) => () => void;
};

/** Delay before the next check after `failures` consecutive failures (0 = the last one worked). */
export function nextDelay(failures: number): number {
  if (failures <= 0) return UPDATE_EVERY_MS;
  return Math.min(UPDATE_RETRY_MS * 2 ** (failures - 1), UPDATE_EVERY_MS);
}

/** Start polling. `check` resolves true when it reached the release feed.
 *  Returns stop(): no timer, listener or late answer survives it. */
export function startUpdatePoll(check: () => Promise<boolean>, env: PollEnv): () => void {
  let stopped = false;
  let inflight = false;
  let failures = 0;
  let timer: unknown = null;
  let dueAt = env.now() + UPDATE_FIRST_MS;

  const arm = () => {
    if (timer !== null) env.clearTimeout(timer);
    timer = env.setTimeout(fire, Math.max(0, dueAt - env.now()));
  };
  const run = async () => {
    if (inflight || stopped) return;
    inflight = true;
    let reached = false;
    try { reached = await check(); } catch { reached = false; }
    inflight = false;
    if (stopped) return;
    failures = reached ? 0 : failures + 1;
    dueAt = env.now() + nextDelay(failures);
    arm();
  };
  // Due while hidden → do nothing; the visibility handler owns the catch-up.
  function fire() {
    timer = null;
    if (env.visible()) void run();
  }
  const unsubscribe = env.onVisible(() => {
    if (stopped || inflight || !env.visible()) return;
    if (env.now() >= dueAt) {
      if (timer !== null) { env.clearTimeout(timer); timer = null; }
      void run();
    } else if (timer === null) {
      arm();
    }
  });
  arm();
  return () => {
    stopped = true;
    if (timer !== null) env.clearTimeout(timer);
    timer = null;
    unsubscribe();
  };
}

/** The browser's env. Only touched when App starts the poll. */
export function browserPollEnv(): PollEnv {
  return {
    now: () => Date.now(),
    setTimeout: (fn, ms) => window.setTimeout(fn, ms),
    clearTimeout: (t) => window.clearTimeout(t as number),
    visible: () => document.visibilityState !== "hidden",
    onVisible: (fn) => {
      document.addEventListener("visibilitychange", fn);
      return () => document.removeEventListener("visibilitychange", fn);
    },
  };
}

/** Which release the bubble announces, or null. Dismissal is per VERSION:
 *  the tag you closed stays closed and a newer release asks again. `newer`
 *  means the app or the installed CLI is behind `latest` — a CLI that is
 *  merely not installed is a setup state the gear's dot already carries, not
 *  news about a release. */
export function bubbleTag(a: { latest: string | null | undefined; newer: boolean; dismissed: string | undefined }): string | null {
  if (!a.latest || !a.newer) return null;
  return a.latest === a.dismissed ? null : a.latest;
}
