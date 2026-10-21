// When the page tries to reach its engine again, and whether a try may START
// one. The engine door (`ipc.ts` `engineDoor`) owns the sockets; this module
// owns the policy, and nothing else does: the shim only obeys `start: false`
// (it never spawns or replaces under it) and reports two facts this side
// cannot see — whether the serve it found is the app's, and whether the call
// started one (app/src-tauri/src/engine.rs `Endpoint`, `ConnectError`).
//
// The rules:
// - A retry waits `min(30 s, 0.5 s · 2^n)`, jittered DOWN to anywhere in
//   [d/2, d], so the countdown shown is an upper bound.
// - Reconnecting to a serve that is alive is always allowed.
// - Starting one is allowed only when the serve that was lost was this app's
//   (started or adopted — or there was none yet, at launch), and at most
//   `CRASH_LOOP.starts` times in any `CRASH_LOOP.windowMs`. Past that, a try
//   that finds nothing to reconnect to STOPS the retries: the banner says why
//   and names serve.log, and only a press of Reconnect now goes on. From then
//   until the engine is up again no automatic try starts anything, however
//   long ago the starts were.
// - A serve someone else started is never restarted: the retries go on,
//   reconnect-only, until it is back.
// - A press of Reconnect now cancels the timer, tries at once, and may start
//   one serve — a person asked — which the ledger records. A press never
//   re-arms automatic starts: after a stop, one press is one start, and if
//   that serve does not come up the next automatic try stops again.
// - One try in flight, ever. A retry that falls due while the window is
//   hidden waits for it to be seen: nobody is looking, a crash loop would
//   spend its budget unseen, and a serve that is not coming back would be
//   polled (and logged) all night.
//
// Pure: the clock, the random source, the window's visibility and the one
// connector are injected, and reconnect-check.mjs drives it on a virtual
// clock. No runtime imports — the check loads it as a `data:` module, and
// transport-check.mjs inlines it ahead of ipc.ts.

/** The backoff: 0.5, 1, 2, 4, 8, 16, then 30 s for good. The first try is not
 *  at once because the close usually arrives while the serve that closed it is
 *  still on its way out; the cap is the longest someone who fixed something by
 *  hand should wait for the app to notice, at a cost of one look twice a
 *  minute. */
export const BACKOFF = { baseMs: 500, capMs: 30_000 } as const;

/** At most this many starts in any window this long, the launch's own
 *  included. A start costs up to 20 s of the shim's time and a login-shell
 *  probe, and a serve that has died three times in a minute will not fix
 *  itself. */
export const CRASH_LOOP = { starts: 3, windowMs: 60_000 } as const;

/** The nominal wait before retry `n` (0 is the first after a loss). */
export function nominalDelay(n: number): number {
  return Math.min(BACKOFF.capMs, BACKOFF.baseMs * 2 ** n);
}

/** Retry `n`'s wait for a random draw `r` in [0, 1): in (d/2, d] — never later
 *  than nominal ("equal jitter"). One app per host makes a herd unlikely, but
 *  a second client of the same serve should not reconnect in lockstep. */
export function jittered(n: number, r: number): number {
  const d = nominalDelay(n);
  return d - (d / 2) * Math.min(Math.max(r, 0), 1);
}

/** One try, as the connector gets it. `reason` is why the engine is not up
 *  (null at launch), for the shim's log; `attempt` numbers the tries since
 *  the engine was last up, from 1. */
export type Try = { start: boolean; manual: boolean; reason: string | null; attempt: number };

/** What a try came to. `started`: the shim reached its start, whatever came
 *  of it. `declined`: nothing was there to reconnect to, and the try could
 *  not start one. */
export type Outcome =
  | { ok: true; owned: boolean; started: boolean }
  | { ok: false; reason: string; started: boolean; declined: boolean };

/** Where the retries stand, for the banner. `at` is on `env.now()`'s clock. */
export type RetryView =
  | { kind: "trying"; manual: boolean; reason: string | null; phase: string | null }
  | { kind: "up" }
  | { kind: "waiting"; reason: string; at: number; notOurs: boolean }
  | { kind: "hidden"; reason: string; notOurs: boolean }
  | { kind: "stopped"; reason: string };

export type RetryEnv = {
  now: () => number;
  setTimeout: (fn: () => void, ms: number) => unknown;
  clearTimeout: (handle: unknown) => void;
  /** In [0, 1). */
  random: () => number;
  /** Is the window being seen? `document.visibilityState !== "hidden"`. */
  visible: () => boolean;
};

export type Retrier = {
  /** The launch: one try, now, allowed to start (there is no serve yet to
   *  have been someone else's). Not deferred for a hidden window. */
  boot(): Promise<void>;
  /** The live socket died: retry on the backoff. */
  lost(reason: string): void;
  /** Reconnect now: cancel the timer and try at once, allowed to start. */
  now(): Promise<void>;
  /** The window's visibility changed: a retry that fell due runs now. */
  visibility(): void;
};

export function retrier(
  env: RetryEnv,
  connect: (t: Try, phase: (p: string) => void) => Promise<Outcome>,
  report: (v: RetryView) => void,
): Retrier {
  /** The last serve reached: the app's? null before the first. */
  let owned: boolean | null = null;
  /** When each start this page asked for was reported, oldest first. */
  const starts: number[] = [];
  /** Retries scheduled since the engine was last up: the backoff step. */
  let step = 0;
  /** Tries since the engine was last up, for the shim's log. */
  let tries = 0;
  let up = false;
  let timer: unknown = null;
  let flight: Promise<void> | null = null;
  /** A retry fell due while the window was hidden. */
  let due = false;
  /** The live socket died while a try was still settling (see `lost`). */
  let lostInFlight: string | null = null;
  /** Why the engine is not up. */
  let why: string | null = null;
  /** The crash-loop limit stopped the retries, and the engine has not been
   *  up since: no automatic try may start one. */
  let tripped = false;
  /** Why the last try that started a serve failed: what a stop names. */
  let startFailure: string | null = null;

  const recentStarts = () => {
    const since = env.now() - CRASH_LOOP.windowMs;
    while (starts.length && starts[0] <= since) starts.shift();
    return starts.length;
  };
  const mayStart = (manual: boolean) => manual || (!tripped && owned !== false && recentStarts() < CRASH_LOOP.starts);

  function cancel() {
    if (timer !== null) env.clearTimeout(timer);
    timer = null;
    due = false;
  }

  function schedule(notOurs: boolean) {
    const reason = why ?? "the engine is not connected";
    const ms = jittered(step++, env.random());
    const at = env.now() + ms;
    timer = env.setTimeout(() => {
      timer = null;
      if (env.visible()) void run(false);
      else {
        due = true;
        report({ kind: "hidden", reason, notOurs });
      }
    }, ms);
    report({ kind: "waiting", reason, at, notOurs });
  }

  function settle(start: boolean, r: Outcome) {
    if (r.started) starts.push(env.now());
    if (r.ok) {
      owned = r.owned;
      tripped = false;
      startFailure = null;
      up = true;
      step = 0;
      tries = 0;
      why = null;
      report({ kind: "up" });
      const gone = lostInFlight;
      lostInFlight = null;
      if (gone !== null) lost(gone);
      return;
    }
    lostInFlight = null;
    why = r.reason;
    if (r.started) startFailure = r.reason;
    if (r.declined && !start) {
      if (owned === false) {
        schedule(true); // someone else's serve: wait for it to come back
        return;
      }
      const again = tripped;
      tripped = true;
      const n = recentStarts();
      const last = startFailure ?? r.reason;
      report({
        kind: "stopped",
        reason: again
          ? `The serve started on request did not stay up either, so the app is still not starting one. Last: ${last}`
          : `worktrees serve was started ${n} times in the last ${CRASH_LOOP.windowMs / 1000} s and did not stay up, ` +
            `so the app has stopped starting it. Last: ${last}`,
      });
      return;
    }
    schedule(false);
  }

  function run(manual: boolean): Promise<void> {
    if (flight) return flight;
    cancel();
    const start = mayStart(manual);
    const t: Try = { start, manual, reason: why, attempt: ++tries };
    report({ kind: "trying", manual, reason: why, phase: null });
    // A phase is news only while its try is the one in flight.
    let live = true;
    const phase = (p: string) => {
      if (live) report({ kind: "trying", manual, reason: t.reason, phase: p });
    };
    flight = (async () => {
      let r: Outcome;
      try {
        r = await connect(t, phase);
      } catch (e) {
        r = { ok: false, reason: String(e), started: false, declined: false };
      }
      live = false;
      flight = null;
      settle(start, r);
    })();
    return flight;
  }

  function lost(reason: string) {
    if (flight) {
      // The socket a try has just made live died before the try reported
      // back: count it once that try has said "up".
      lostInFlight = reason;
      return;
    }
    if (!up) return;
    up = false;
    why = reason;
    step = 0;
    schedule(false);
  }

  return {
    boot: () => run(false),
    lost,
    now: () => (up ? Promise.resolve() : run(true)),
    visibility() {
      if (due && !flight && env.visible()) void run(false);
    },
  };
}
