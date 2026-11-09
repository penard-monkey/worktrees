// The Ideas companion's one view in phase 0: empty, and connected. Documents,
// the editor and their store are later phases (docs/proposals/
// idea-board-phases.md §6); until then this says "No ideas yet" and, only while
// the engine is not up, why not.
import { useEffect, useSyncExternalStore } from "react";
import { engineState, invoke, onEngineState, type EngineState } from "../src/ipc";
import { applySettings, loadSettings } from "../src/settings";
import "../src/tokens.css";
import "./companion.css";

function useEngine(): EngineState {
  return useSyncExternalStore(onEngineState, engineState);
}

/** What a line in serve.log says about this window's connection: which socket
 *  (gen), from which origin, with which crypto. Read by a person after a
 *  hand test, since nothing may click this window for them. */
function connectLine(gen: number): string {
  const subtle = typeof window.crypto?.subtle?.verify === "function" ? "present" : "MISSING";
  return `companion: engine up (gen ${gen}) origin=${window.location.origin} crypto.subtle=${subtle}`;
}

/** Why the engine is not up, in words for the one muted line. */
function waitingWords(e: EngineState): string | null {
  switch (e.kind) {
    case "up":
      return null;
    case "starting":
      return "Connecting to the engine…";
    case "waiting":
    case "hidden":
      return "Waiting for the engine. The main window restarts it; this window reconnects on its own.";
    case "down":
      return `The engine is not running: ${e.reason}`;
  }
}

export function Companion() {
  const engine = useEngine();
  const up = engine.kind === "up";
  const gen = engine.gen;
  // Once per socket: the window's theme from the shared settings (read only —
  // the main window owns ui-state.json's whole-blob save), and the connect
  // line. gen 0 is a door with no engine (the mock): nothing to log to.
  useEffect(() => {
    if (!up) return;
    loadSettings().then(applySettings).catch(() => {});
    if (gen > 0) invoke("log_event", { level: "info", msg: connectLine(gen) }).catch(() => {});
  }, [up, gen]);
  const words = waitingWords(engine);
  return (
    <main className="cmp">
      <div className="cmp-empty">
        <p className="cmp-title">No ideas yet</p>
        {words && <p className="cmp-engine" data-testid="companion-engine">{words}</p>}
      </div>
    </main>
  );
}
