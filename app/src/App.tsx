import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { TerminalPane } from "./TerminalPane";
import "./App.css";

type Declared = {
  lifecycle?: string;
  pinned?: boolean;
  note?: string;
  last_opened_epoch?: number;
  up_cmd?: string | null;
} | null;

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
  declared: Declared;
  lifecycle_effective: string;
};
type Snapshot = { repo: string; prefix: string; places: Place[] };

const DEFAULT_REPO = "/Users/davidpena/workspace/worktrees";

// Nav section order. A pinned place floats to "pinned" regardless of state.
const GROUPS = ["pinned", "active", "idle", "saved", "closed", "archived", "abandoned"] as const;
const GROUP_LABEL: Record<string, string> = {
  pinned: "Pinned",
  active: "Active",
  idle: "Idle",
  saved: "Saved",
  closed: "Closed",
  archived: "Archived",
  abandoned: "Abandoned",
};
// Lifecycle a user can set (active/idle are derived, never written).
const SETTABLE: { label: string; value: string }[] = [
  { label: "Close", value: "closed" },
  { label: "Save", value: "saved" },
  { label: "Archive", value: "archived" },
  { label: "Abandon", value: "abandoned" },
];

function App() {
  const [repo, setRepo] = useState(DEFAULT_REPO);
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [err, setErr] = useState("");
  const [selectedSlug, setSelectedSlug] = useState<string | null>(null);
  const [filter, setFilter] = useState("");

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

  const selected = snap?.places.find((p) => p.slug === selectedSlug) ?? null;

  const mutate = async (p: Promise<unknown>) => {
    try {
      await p;
      await refresh();
    } catch (e) {
      setErr(String(e));
    }
  };

  const openPlace = (p: Place) => {
    setSelectedSlug(p.slug);
    mutate(invoke("touch_place", { repo, slug: p.slug })); // stamp last-opened
  };

  // Filter + group the non-main places; main is always rendered first.
  const { main, groups } = useMemo(() => {
    const q = filter.trim().toLowerCase();
    const match = (p: Place) =>
      !q ||
      p.slug.toLowerCase().includes(q) ||
      (p.branch ?? "").toLowerCase().includes(q) ||
      (p.declared?.note ?? "").toLowerCase().includes(q);
    const places = (snap?.places ?? []).filter(match);
    const main = places.find((p) => p.is_main) ?? null;
    const buckets: Record<string, Place[]> = {};
    for (const p of places) {
      if (p.is_main) continue;
      const key = p.declared?.pinned ? "pinned" : p.lifecycle_effective;
      (buckets[key] ??= []).push(p);
    }
    return { main, groups: buckets };
  }, [snap, filter]);

  const Row = ({ p }: { p: Place }) => (
    <li
      className={selectedSlug === p.slug ? "sel" : ""}
      onClick={() => openPlace(p)}
    >
      <span className={"dot " + (p.tmux_session.up ? "on" : "off")} />
      <span className="slug">
        {p.is_main ? "◆ " : ""}
        {p.declared?.pinned ? "★ " : ""}
        {p.slug}
      </span>
      <span className="branch">
        {p.branch ?? (p.detached ? "(detached)" : "—")}
      </span>
      {p.dirty ? <span className="tag dirty">dirty</span> : null}
      {p.claude_session_present ? <span className="tag ai">ai</span> : null}
    </li>
  );

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
        <input
          className="search"
          placeholder="search places…"
          value={filter}
          onChange={(e) => setFilter(e.currentTarget.value)}
        />
        {err && <div className="err">{err}</div>}
        <div className="scroll">
          {main && (
            <ul className="places">
              <Row p={main} />
            </ul>
          )}
          {GROUPS.filter((g) => groups[g]?.length).map((g) => (
            <div key={g} className="group">
              <div className="group-h">{GROUP_LABEL[g]}</div>
              <ul className="places">
                {groups[g].map((p) => (
                  <Row key={p.slug} p={p} />
                ))}
              </ul>
            </div>
          ))}
        </div>
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
              {selected.ahead || selected.behind ? (
                <span className="ab">↑{selected.ahead ?? 0} ↓{selected.behind ?? 0}</span>
              ) : null}
              <span className="life">{selected.lifecycle_effective}</span>
              <div className="actions">
                <button
                  className={selected.declared?.pinned ? "on" : ""}
                  title={selected.declared?.pinned ? "unpin" : "pin"}
                  onClick={() =>
                    mutate(invoke("set_pin", { repo, slug: selected.slug, on: !selected.declared?.pinned }))
                  }
                >
                  ★
                </button>
                {SETTABLE.map((s) => (
                  <button
                    key={s.value}
                    className={selected.declared?.lifecycle === s.value ? "on" : ""}
                    onClick={() =>
                      mutate(invoke("set_lifecycle", { repo, slug: selected.slug, label: s.value }))
                    }
                  >
                    {s.label}
                  </button>
                ))}
              </div>
            </header>
            <input
              className="note"
              placeholder="note…"
              defaultValue={selected.declared?.note ?? ""}
              key={selected.slug + (selected.declared?.note ?? "")}
              onBlur={(e) =>
                mutate(invoke("set_note", { repo, slug: selected.slug, note: e.currentTarget.value }))
              }
            />
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
