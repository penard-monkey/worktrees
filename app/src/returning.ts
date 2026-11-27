// When a window's owner has come back after being away. Pure and import-free:
// the caller supplies both the epoch clock and the local calendar day, and
// returning-check.mjs drives the real module on a scripted clock.

export const RETURN_AFTER_MS = 6 * 60 * 60 * 1000;

export type ReturnEvent = "input" | "hidden" | "visible" | "focus" | "unload";

export type ReturningState = {
  /** The later of the last input and the last hide. Null on a first run. */
  lastActive: number | null;
  /** The local day the panel last opened, not merely the last digest request. */
  shownDay: string | null;
};

export type ReturnStep = {
  state: ReturningState;
  isReturn: boolean;
  /** Only these transitions reach the settings blob; input stays in memory. */
  persist: boolean;
};

export function reduceReturning(state: ReturningState, event: ReturnEvent, now: number, today: string): ReturnStep {
  const isReturn = (event === "visible" || event === "focus" || event === "input")
    && state.lastActive !== null
    && now - state.lastActive > RETURN_AFTER_MS
    && state.shownDay !== today;
  const lastActive = event === "input" || event === "hidden"
    ? Math.max(state.lastActive ?? now, now)
    : state.lastActive;
  return {
    state: { lastActive, shownDay: state.shownDay },
    isReturn,
    persist: event === "hidden" || event === "unload",
  };
}

/** Call only after the panel actually opened. An unchanged digest leaves the
 *  day unmarked, so a later qualifying return can still offer the panel. */
export function markShown(state: ReturningState, today: string): ReturningState {
  return state.shownDay === today ? state : { ...state, shownDay: today };
}
