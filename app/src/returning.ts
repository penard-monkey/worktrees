// When a window's owner has come back after being away. Pure and import-free:
// the caller supplies both the epoch clock and the local calendar day, and
// returning-check.mjs drives the real module on a scripted clock.

export const RETURN_AFTER_MS = 6 * 60 * 60 * 1000;
export const INPUT_PERSIST_MS = 10 * 60 * 1000;

export type ReturnEvent = "input" | "hidden" | "visible" | "focus" | "unload";

export type ReturningState = {
  /** The last input, hide or return. Null on a first run. */
  lastActive: number | null;
  /** The local day the panel last opened, not merely the last digest request. */
  shownDay: string | null;
  /** In memory only: the lastActive value most recently handed to persistence. */
  lastPersisted: number | null;
};

export type ReturnStep = {
  state: ReturningState;
  isReturn: boolean;
  /** Hide/unload writes, plus an input only once the persistence gap passes. */
  persist: boolean;
};

export function reduceReturning(state: ReturningState, event: ReturnEvent, now: number, today: string): ReturnStep {
  const isReturn = (event === "visible" || event === "focus" || event === "input")
    && state.lastActive !== null
    && now - state.lastActive > RETURN_AFTER_MS
    && state.shownDay !== today;
  const lastActive = event === "input" || event === "hidden" || isReturn
    ? Math.max(state.lastActive ?? now, now)
    : state.lastActive;
  const persist = event === "hidden" || event === "unload"
    || (event === "input" && (state.lastPersisted === null || now - state.lastPersisted > INPUT_PERSIST_MS));
  return {
    state: { lastActive, shownDay: state.shownDay, lastPersisted: persist ? lastActive : state.lastPersisted },
    isReturn,
    persist,
  };
}

/** Call only after the panel actually opened. An unchanged digest leaves the
 *  day unmarked, so a later qualifying return can still offer the panel. */
export function markShown(state: ReturningState, today: string): ReturningState {
  return state.shownDay === today ? state : { ...state, shownDay: today };
}
