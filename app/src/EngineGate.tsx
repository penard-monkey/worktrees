// The app's view of its engine — this host's `worktrees serve`.
//
// Before the engine has answered once, the app is not mounted at all: App
// hydrates its settings on mount, and against a dead engine `loadSettings`
// answers DEFAULTS, which a later whole-blob save would write over the user's
// real ones. So the gate shows what is happening instead — starting, or why it
// could not, with the log that says more and a way to try again.
//
// After that, losing the engine must not look like a closed place or a blank
// app (proposal §III.7, "a dropped socket must not look like a closed place"):
// the app stays mounted, showing what it last knew, under a banner that says
// the engine is gone and offers Reconnect. Reconnecting re-runs the shim's
// find-or-start, re-subscribes every listener, and bumps the engine's `gen`,
// which the terminals carry in their keys — so each re-attaches through the
// same remount it always had (a tmux pane redraws; a dock shell replays its
// ring behind the `── restored ──` seam).
import { useSyncExternalStore, type ReactNode } from "react";
import { engineState, onEngineState, reconnectEngine, type EngineState } from "./ipc";
import "./tokens.css";
import "./EngineGate.css";

/** Where serve logs: the place that says why it would not start. */
const SERVE_LOG = "~/.cache/worktrees/serve.log";

/** The engine state, live. `gen` 0 everywhere but the engine door. */
export function useEngine(): EngineState {
  return useSyncExternalStore(onEngineState, engineState);
}

export function EngineGate({ children }: { children: ReactNode }) {
  const st = useEngine();
  if (st.gen === 0 && st.kind !== "up") return <EngineBoot st={st} />;
  return (
    <>
      {children}
      {st.kind !== "up" && <EngineBanner st={st} />}
    </>
  );
}

function EngineBoot({ st }: { st: EngineState }) {
  return (
    <div className="engine-boot">
      <div className="engine-boot-card" role={st.kind === "down" ? "alert" : "status"}>
        {st.kind === "down" ? (
          <>
            <div className="engine-title">
              <span className="engine-dot down" aria-hidden /> worktrees could not reach its engine
            </div>
            <p className="engine-why">{st.reason}</p>
            <p className="engine-log">
              The engine’s log: <code>{SERVE_LOG}</code>
            </p>
            <button className="enter-btn" autoFocus onClick={() => void reconnectEngine()}>
              Try again
            </button>
          </>
        ) : (
          <div className="engine-title">
            <span className="engine-dot starting" aria-hidden /> Starting the engine…
          </div>
        )}
      </div>
    </div>
  );
}

function EngineBanner({ st }: { st: EngineState }) {
  const down = st.kind === "down";
  return (
    <div className="engine-banner" role="alert">
      <span className={`engine-dot ${down ? "down" : "starting"}`} aria-hidden />
      <span className="engine-banner-txt">
        {down ? (
          <>
            <b>Disconnected from the engine.</b> {st.kind === "down" ? st.reason : ""} Its log:{" "}
            <code>{SERVE_LOG}</code>
          </>
        ) : (
          <b>Reconnecting to the engine…</b>
        )}
      </span>
      {/* Focused when it appears: the terminal that had the keyboard is
          talking to nothing now, and Enter here is the one key that helps. */}
      <button className="enter-btn sm" autoFocus disabled={!down} onClick={() => void reconnectEngine()}>
        Reconnect
      </button>
    </div>
  );
}
