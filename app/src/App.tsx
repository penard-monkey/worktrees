import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
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
type ProjectView = { root: string; ok: boolean; error: string | null; snapshot: Snapshot | null };
type Workspace = { projects: ProjectView[] };
type CmdResult = { ok: boolean; code: number; output: string };

const GROUPS = ["pinned", "active", "idle", "saved", "closed", "archived", "abandoned"] as const;
const GROUP_LABEL: Record<string, string> = {
  pinned: "Pinned", active: "Active", idle: "Idle", saved: "Saved",
  closed: "Closed", archived: "Archived", abandoned: "Abandoned",
};
const SETTABLE = [
  { label: "Close", value: "closed" },
  { label: "Save", value: "saved" },
  { label: "Archive", value: "archived" },
  { label: "Abandon", value: "abandoned" },
];
const basename = (p: string) => p.replace(/\/+$/, "").split("/").pop() || p;

function App() {
  const [ws, setWs] = useState<Workspace | null>(null);
  const [err, setErr] = useState("");
  const [sel, setSel] = useState<{ repo: string; slug: string } | null>(null);
  const [filter, setFilter] = useState("");
  const [newFor, setNewFor] = useState<string | null>(null); // project root the new-form targets
  const [newBranch, setNewBranch] = useState("");
  const [newName, setNewName] = useState("");
  const [switchTo, setSwitchTo] = useState("");
  const [confirmRm, setConfirmRm] = useState<string | null>(null); // "repo|slug"

  const refresh = useCallback(async () => {
    try {
      setErr("");
      setWs(await invoke<Workspace>("list_workspace"));
    } catch (e) {
      setErr(String(e));
    }
  }, []);
  useEffect(() => {
    refresh();
  }, [refresh]);

  const selected: Place | null =
    (sel && ws?.projects.find((p) => p.root === sel.repo)?.snapshot?.places.find((pl) => pl.slug === sel.slug)) || null;

  const mutate = async (p: Promise<unknown>) => {
    try {
      await p;
      await refresh();
    } catch (e) {
      setErr(String(e));
    }
  };
  const runCmd = async (name: string, args: Record<string, unknown>): Promise<boolean> => {
    try {
      setErr("");
      const r = await invoke<CmdResult>(name, args);
      if (!r.ok) setErr(r.output || `${name} failed (exit ${r.code})`);
      await refresh();
      return r.ok;
    } catch (e) {
      setErr(String(e));
      return false;
    }
  };

  const addProject = async () => {
    try {
      const dir = await open({ directory: true, title: "Add a git project" });
      if (typeof dir === "string") {
        setErr("");
        setWs(await invoke<Workspace>("add_project", { dir }));
      }
    } catch (e) {
      setErr(String(e));
    }
  };
  const removeProject = async (root: string) => {
    try {
      setWs(await invoke<Workspace>("remove_project", { root }));
      if (sel?.repo === root) setSel(null);
    } catch (e) {
      setErr(String(e));
    }
  };

  const openPlace = (repo: string, p: Place) => {
    setSel({ repo, slug: p.slug });
    mutate(invoke("touch_place", { repo, slug: p.slug }));
  };
  const createPlace = async (repo: string) => {
    const branch = newBranch.trim();
    if (!branch) return;
    const name = newName.trim();
    if (await runCmd("new_place", { repo, branch, base: null, name: name || null })) {
      setSel({ repo, slug: (name || branch).replace(/\//g, "-") });
      setNewFor(null);
      setNewBranch("");
      setNewName("");
    }
  };
  const doSwitch = async () => {
    if (!sel) return;
    const b = switchTo.trim();
    if (!b) return;
    if (await runCmd("switch_place", { repo: sel.repo, slug: sel.slug, branch: b, base: null })) setSwitchTo("");
  };
  const doRemove = async () => {
    if (!sel) return;
    const key = `${sel.repo}|${sel.slug}`;
    if (confirmRm !== key) {
      setConfirmRm(key);
      return;
    }
    setConfirmRm(null);
    if (await runCmd("remove_place", { repo: sel.repo, slug: sel.slug, del_branch: false, force: false })) setSel(null);
  };

  const q = filter.trim().toLowerCase();
  const matchPlace = (p: Place) =>
    !q ||
    p.slug.toLowerCase().includes(q) ||
    (p.branch ?? "").toLowerCase().includes(q) ||
    (p.declared?.note ?? "").toLowerCase().includes(q);

  const Row = ({ repo, p }: { repo: string; p: Place }) => (
    <li
      className={sel?.repo === repo && sel?.slug === p.slug ? "sel" : ""}
      onClick={() => openPlace(repo, p)}
    >
      <span className={"dot " + (p.tmux_session.up ? "on" : "off")} />
      <span className="slug">
        {p.is_main ? "◆ " : ""}
        {p.declared?.pinned ? "★ " : ""}
        {p.slug}
      </span>
      <span className="branch">{p.branch ?? (p.detached ? "(detached)" : "—")}</span>
      {p.dirty ? <span className="tag dirty">dirty</span> : null}
      {p.claude_session_present ? <span className="tag ai">ai</span> : null}
    </li>
  );

  const ProjectTree = ({ pv }: { pv: ProjectView }) => {
    const places = (pv.snapshot?.places ?? []).filter(matchPlace);
    const main = places.find((p) => p.is_main) ?? null;
    const buckets: Record<string, Place[]> = {};
    for (const p of places) {
      if (p.is_main) continue;
      const key = p.declared?.pinned ? "pinned" : p.lifecycle_effective;
      (buckets[key] ??= []).push(p);
    }
    return (
      <div className="project">
        <div className="project-h">
          <span className="pname" title={pv.root}>{basename(pv.root)}</span>
          <button className="mini" title="new worktree" onClick={() => setNewFor(newFor === pv.root ? null : pv.root)}>＋</button>
          <button className="mini" title="remove project" onClick={() => removeProject(pv.root)}>×</button>
        </div>
        {newFor === pv.root && (
          <div className="newform">
            <input
              placeholder="branch (e.g. feat/x)"
              value={newBranch}
              autoFocus
              onChange={(e) => setNewBranch(e.currentTarget.value)}
              onKeyDown={(e) => e.key === "Enter" && createPlace(pv.root)}
            />
            <input
              placeholder="name (optional)"
              value={newName}
              onChange={(e) => setNewName(e.currentTarget.value)}
              onKeyDown={(e) => e.key === "Enter" && createPlace(pv.root)}
            />
            <button onClick={() => createPlace(pv.root)} disabled={!newBranch.trim()}>Create</button>
          </div>
        )}
        {!pv.ok ? (
          <div className="project-err">{pv.error ?? "unavailable"}</div>
        ) : (
          <>
            {main && (
              <ul className="places">
                <Row repo={pv.root} p={main} />
              </ul>
            )}
            {GROUPS.filter((g) => buckets[g]?.length).map((g) => (
              <div key={g} className="group">
                <div className="group-h">{GROUP_LABEL[g]}</div>
                <ul className="places">
                  {buckets[g].map((p) => (
                    <Row key={p.slug} repo={pv.root} p={p} />
                  ))}
                </ul>
              </div>
            ))}
          </>
        )}
      </div>
    );
  };

  return (
    <div className="app">
      <aside className="nav">
        <div className="repo">
          <b className="title">Projects</b>
          <button onClick={addProject} title="add a git project">＋ add</button>
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
          {ws && ws.projects.length === 0 && (
            <div className="empty small">No projects yet.<br />Click <b>＋ add</b> to open one.</div>
          )}
          {ws?.projects.map((pv) => <ProjectTree key={pv.root} pv={pv} />)}
        </div>
      </aside>

      <main className="main">
        {selected && sel ? (
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
                  onClick={() => mutate(invoke("set_pin", { repo: sel.repo, slug: sel.slug, on: !selected.declared?.pinned }))}
                >
                  ★
                </button>
                {SETTABLE.map((s) => (
                  <button
                    key={s.value}
                    className={selected.declared?.lifecycle === s.value ? "on" : ""}
                    onClick={() => mutate(invoke("set_lifecycle", { repo: sel.repo, slug: sel.slug, label: s.value }))}
                  >
                    {s.label}
                  </button>
                ))}
                {!selected.is_main && (
                  <>
                    <input
                      className="switchto"
                      placeholder="switch to branch…"
                      value={switchTo}
                      onChange={(e) => setSwitchTo(e.currentTarget.value)}
                      onKeyDown={(e) => e.key === "Enter" && doSwitch()}
                    />
                    <button onClick={doSwitch} disabled={!switchTo.trim()}>Switch</button>
                    <button
                      className={confirmRm === `${sel.repo}|${sel.slug}` ? "danger" : ""}
                      onClick={doRemove}
                      onBlur={() => setConfirmRm(null)}
                    >
                      {confirmRm === `${sel.repo}|${sel.slug}` ? "Confirm?" : "Remove"}
                    </button>
                  </>
                )}
              </div>
            </header>
            <input
              className="note"
              placeholder="note…"
              defaultValue={selected.declared?.note ?? ""}
              key={sel.repo + sel.slug + (selected.declared?.note ?? "")}
              onBlur={(e) => mutate(invoke("set_note", { repo: sel.repo, slug: sel.slug, note: e.currentTarget.value }))}
            />
            {selected.tmux_session.up ? (
              <TerminalPane key={selected.tmux_session.name} session={selected.tmux_session.name} />
            ) : (
              <div className="empty">
                No live tmux session for <b>{selected.slug}</b>.
                <br />
                Open it: create/switch spins one up, or run <code>worktrees open {selected.slug}</code>.
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
