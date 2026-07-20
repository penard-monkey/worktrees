import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { TerminalPane } from "./TerminalPane";
import "./App.css";

// Mirrors the fields `worktrees ls --json` emits (schema v1). Only what the
// spike renders; the full typed model lands in a later phase.
type Place = {
  slug: string;
  path: string;
  is_main: boolean;
  registered: boolean;
  branch: string | null;
  detached: boolean | null;
  dirty: boolean | null;
  ahead: number | null;
  behind: number | null;
  tmux_session: { name: string; up: boolean };
  claude_session_present: boolean;
  lifecycle_effective: string;
};
type Snapshot = { repo: string; prefix: string; places: Place[] };

// Spike default; the repo picker is the real entry point.
const DEFAULT_REPO = "/Users/davidpena/workspace/worktrees";

function App() {
  const [repo, setRepo] = useState(DEFAULT_REPO);
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [err, setErr] = useState("");
  const [selected, setSelected] = useState<Place | null>(null);

  const refresh = useCallback(async () => {
    try {
      setErr("");
      setSnap(await invoke<Snapshot>("list_places", { repo }));
    } catch (e) {
      setErr(String(e));
      setSnap(null);
    }
  }, [repo]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  return (
    <div className="app">
      <aside className="nav">
        <div className="repo">
          <input
            value={repo}
            spellCheck={false}
            onChange={(e) => setRepo(e.currentTarget.value)}
            onKeyDown={(e) => e.key === "Enter" && refresh()}
          />
          <button onClick={refresh} title="refresh">↻</button>
        </div>
        {err && <div className="err">{err}</div>}
        <ul className="places">
          {snap?.places.map((p) => (
            <li
              key={p.slug}
              className={selected?.slug === p.slug ? "sel" : ""}
              onClick={() => setSelected(p)}
            >
              <span className={"dot " + (p.tmux_session.up ? "on" : "off")} />
              <span className="slug">
                {p.is_main ? "◆ " : ""}
                {p.slug}
              </span>
              <span className="branch">
                {p.branch ?? (p.detached ? "(detached)" : "—")}
              </span>
              {p.dirty ? <span className="tag dirty">dirty</span> : null}
              {p.claude_session_present ? <span className="tag ai">ai</span> : null}
            </li>
          ))}
        </ul>
      </aside>

      <main className="main">
        {selected ? (
          <>
            <header className="topbar">
              <b className="slug">
                {selected.is_main ? "◆ " : ""}
                {selected.slug}
              </b>
              <span className="branch">{selected.branch ?? "(detached)"}</span>
              {(selected.ahead || selected.behind) ? (
                <span className="ab">↑{selected.ahead ?? 0} ↓{selected.behind ?? 0}</span>
              ) : null}
              <span className="life">{selected.lifecycle_effective}</span>
            </header>
            {selected.tmux_session.up ? (
              <TerminalPane
                key={selected.tmux_session.name}
                session={selected.tmux_session.name}
              />
            ) : (
              <div className="empty">
                No live tmux session for <b>{selected.slug}</b>.
                <br />
                Start one: <code>worktrees open {selected.slug}</code>
              </div>
            )}
          </>
        ) : (
          <div className="empty">Select a place.</div>
        )}
      </main>
    </div>
  );
}

export default App;
