import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { TerminalPane } from "./TerminalPane";
import { SettingsSheet } from "./SettingsSheet";
import { applySettings, clampNav, DEFAULTS, loadSettings, saveSettings, type Settings } from "./settings";
import "./tokens.css";
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
  dirty_files?: number | null;
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
type Lens = "places" | "recent" | "attention";

const LIVE_TIERS = ["pinned", "active", "idle"] as const;
const DORMANT_TIERS = ["saved", "closed", "archived", "abandoned"] as const;
const GROUP_LABEL: Record<string, string> = {
  pinned: "Pinned", active: "Active", idle: "Idle",
  saved: "Saved", closed: "Closed", archived: "Archived", abandoned: "Abandoned",
};
const SETTABLE = [
  { label: "Close", value: "closed" },
  { label: "Save", value: "saved" },
  { label: "Archive", value: "archived" },
  { label: "Abandon", value: "abandoned" },
];
const DOT_COLOR: Record<string, string> = {
  active: "var(--ok)", idle: "var(--idle)",
  saved: "var(--sticky)", closed: "var(--sticky)", archived: "var(--sticky)", abandoned: "var(--sticky)",
};

const basename = (p: string) => p.replace(/\/+$/, "").split("/").pop() || p;
const bucketOf = (p: Place) => (p.declared?.pinned ? "pinned" : p.lifecycle_effective);
const isLive = (p: Place) => p.tmux_session.up || p.claude_session_present;
const hasAttention = (p: Place) => !!p.dirty || !!p.ahead || !!p.behind;

function ago(epoch?: number): string {
  if (!epoch) return "";
  const s = Math.floor(Date.now() / 1000) - epoch;
  if (s < 60) return "now";
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}

// fixed-order signal glyphs; geometry (3-col row grid) guarantees no collision.
function glyphs(p: Place) {
  const g: { cls: string; text: string; title: string }[] = [];
  if (p.claude_session_present) g.push({ cls: "g-ai", text: "✦", title: "AI session" });
  if (p.dirty) g.push({ cls: "g-dirty", text: `●${p.dirty_files ?? ""}`, title: `${p.dirty_files ?? "dirty"} uncommitted` });
  if (p.ahead) g.push({ cls: "g-ahead", text: `↑${p.ahead}`, title: `${p.ahead} ahead` });
  if (p.behind) g.push({ cls: "g-behind", text: `↓${p.behind}`, title: `${p.behind} behind` });
  if (p.detached) g.push({ cls: "g-det", text: "⑂", title: "detached HEAD" });
  const MAX = 4;
  if (g.length > MAX) return [...g.slice(0, MAX), { cls: "g-more", text: `+${g.length - MAX}`, title: "more" }];
  return g;
}

function App() {
  const [ws, setWs] = useState<Workspace | null>(null);
  const [err, setErr] = useState("");
  const [sel, setSel] = useState<{ repo: string; slug: string } | null>(null);
  const [filter, setFilter] = useState("");
  const [lens, setLens] = useState<Lens>("places");
  const [newFor, setNewFor] = useState<string | null>(null);
  const [newBranch, setNewBranch] = useState("");
  const [newName, setNewName] = useState("");
  const [switchTo, setSwitchTo] = useState("");
  const [confirmRm, setConfirmRm] = useState<string | null>(null);
  const [menu, setMenu] = useState<"life" | "more" | null>(null);
  const [groupOpen, setGroupOpen] = useState<Record<string, boolean>>({});
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const [settings, setSettings] = useState<Settings>(DEFAULTS);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [termVersion, setTermVersion] = useState(0);
  const searchRef = useRef<HTMLInputElement | null>(null);

  const refresh = useCallback(async () => {
    try {
      setErr("");
      setWs(await invoke<Workspace>("list_workspace"));
    } catch (e) {
      setErr(String(e));
    }
  }, []);
  useEffect(() => { refresh(); }, [refresh]);

  // live refresh: backend emits "places:changed" (poll/fs-watch) → re-pull
  useEffect(() => {
    const un = listen("places:changed", () => refresh());
    return () => { un.then((f) => f()).catch(() => {}); };
  }, [refresh]);

  // hydrate persisted settings BEFORE first meaningful paint
  useLayoutEffect(() => {
    (async () => {
      const s = await loadSettings();
      applySettings(s);
      setSettings(s);
      setLens(s.lens);
      setCollapsed(s.collapsed ?? {});
    })();
  }, []);

  const updateSettings = (patch: Partial<Settings>) => {
    setSettings((prev) => {
      const next = { ...prev, ...patch };
      applySettings(next);
      saveSettings(next);
      return next;
    });
    if (patch.term_family !== undefined || patch.term_size !== undefined) setTermVersion((v) => v + 1);
  };

  const selected: Place | null =
    (sel && ws?.projects.find((p) => p.root === sel.repo)?.snapshot?.places.find((pl) => pl.slug === sel.slug)) || null;

  const mutate = async (p: Promise<unknown>) => {
    try { await p; await refresh(); } catch (e) { setErr(String(e)); }
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
      if (typeof dir === "string") { setErr(""); setWs(await invoke<Workspace>("add_project", { dir })); }
    } catch (e) { setErr(String(e)); }
  };
  const removeProject = async (root: string) => {
    try {
      setWs(await invoke<Workspace>("remove_project", { root }));
      if (sel?.repo === root) setSel(null);
    } catch (e) { setErr(String(e)); }
  };

  // THE primary verb: inhabit a place — stamp recency, ensure its session, select it.
  const enterPlace = (repo: string, p: Place) => {
    setSel({ repo, slug: p.slug });
    setMenu(null);
    (async () => {
      await invoke("touch_place", { repo, slug: p.slug }).catch(() => {});
      await runCmd("open_place", { repo, slug: p.slug });
    })();
  };

  const createPlace = async (repo: string) => {
    const branch = newBranch.trim();
    if (!branch) return;
    const name = newName.trim();
    if (await runCmd("new_place", { repo, branch, base: null, name: name || null })) {
      setSel({ repo, slug: (name || branch).replace(/\//g, "-") });
      setNewFor(null); setNewBranch(""); setNewName("");
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
    if (confirmRm !== key) { setConfirmRm(key); return; }
    setConfirmRm(null);
    setMenu(null);
    if (await runCmd("remove_place", { repo: sel.repo, slug: sel.slug, del_branch: false, force: false })) setSel(null);
  };

  const toggleProject = (root: string) => {
    setCollapsed((c) => {
      const next = { ...c, [root]: !c[root] };
      updateSettings({ collapsed: next });
      return next;
    });
  };
  const isOpen = (key: string, def: boolean) => groupOpen[key] ?? def;
  const toggleGroup = (key: string, def: boolean) =>
    setGroupOpen((g) => ({ ...g, [key]: !(g[key] ?? def) }));
  const changeLens = (l: Lens) => { setLens(l); updateSettings({ lens: l }); };

  const q = filter.trim().toLowerCase();
  const matchPlace = (p: Place) =>
    !q ||
    p.slug.toLowerCase().includes(q) ||
    (p.branch ?? "").toLowerCase().includes(q) ||
    (p.declared?.note ?? "").toLowerCase().includes(q);

  // workspace-wide stats for the Briefing cockpit
  const allPlaces = useMemo(
    () => (ws?.projects ?? []).flatMap((pv) => (pv.snapshot?.places ?? []).map((p) => ({ pv, p }))),
    [ws],
  );
  const stats = useMemo(() => {
    let live = 0, dirty = 0, ai = 0;
    for (const { p } of allPlaces) { if (p.tmux_session.up) live++; if (p.dirty) dirty++; if (p.claude_session_present) ai++; }
    return { live, dirty, ai };
  }, [allPlaces]);
  const resume = useMemo(
    () => allPlaces
      .filter(({ p }) => !p.is_main)
      .sort((a, b) => (b.p.declared?.last_opened_epoch ?? 0) - (a.p.declared?.last_opened_epoch ?? 0))
      .slice(0, 6),
    [allPlaces],
  );

  // ── nav resizer (drag the nav's right edge) ──
  const onResize = (e: React.MouseEvent) => {
    e.preventDefault();
    const startX = e.clientX;
    const startW = settings.nav_width;
    const move = (ev: MouseEvent) => updateSettings({ nav_width: clampNav(startW + (ev.clientX - startX)) });
    const up = () => { window.removeEventListener("mousemove", move); window.removeEventListener("mouseup", up); };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  // ── row ──
  const PlaceRow = ({ repo, p, showProject }: { repo: string; p: Place; showProject?: boolean }) => {
    const divergent = !p.is_main && !p.detached && p.branch && p.branch !== p.slug;
    return (
      <li
        className={"row" + (sel?.repo === repo && sel?.slug === p.slug ? " sel" : "")}
        onClick={() => enterPlace(repo, p)}
        title={p.slug}
      >
        <span
          className={"status-dot" + (isLive(p) ? " live" : "")}
          style={{ background: DOT_COLOR[p.lifecycle_effective] ?? "var(--sticky)" }}
        />
        <span className="row-id">
          <span className="row-name">
            {p.is_main ? "◆ " : p.declared?.pinned ? "★ " : ""}
            {p.slug}
            {showProject ? <span className="row-proj">{basename(repo)}</span> : null}
          </span>
          {divergent ? <span className="row-branch">↗ {p.branch}</span> : null}
        </span>
        <span className="glyphs">
          {glyphs(p).map((g, i) => (
            <span key={i} className={"g " + g.cls} title={g.title}>{g.text}</span>
          ))}
        </span>
      </li>
    );
  };

  const GroupHeader = ({ gkey, label, count, open, onToggle }: { gkey: string; label: string; count: number; open: boolean; onToggle: () => void }) => (
    <div className="group-h" key={gkey} onClick={onToggle}>
      <span className="caret">{open ? "▾" : "▸"}</span>
      {label}
      <span className="count">{count}</span>
    </div>
  );

  const ProjectNode = ({ pv }: { pv: ProjectView }) => {
    const open = !collapsed[pv.root];
    const places = (pv.snapshot?.places ?? []).filter(matchPlace);
    const main = places.find((p) => p.is_main) ?? null;
    const rollupLive = places.some((p) => p.tmux_session.up);
    const buckets: Record<string, Place[]> = {};
    for (const p of places) { if (p.is_main) continue; (buckets[bucketOf(p)] ??= []).push(p); }
    const dormant = DORMANT_TIERS.flatMap((t) => buckets[t] ?? []);

    return (
      <div className="project">
        <div className="project-h">
          <span className="caret" onClick={() => toggleProject(pv.root)}>{open ? "▾" : "▸"}</span>
          {pv.ok
            ? <span className={"rollup " + (rollupLive ? "on" : "off")} />
            : <span className="rollup broken" title="repo gone">⊘</span>}
          <span className="pname" title={pv.root} onClick={() => toggleProject(pv.root)}>{basename(pv.root)}</span>
          {pv.ok ? <span className="pcount">{places.length}</span> : <span className="pgone">repo gone</span>}
          <button className="mini" title="new worktree" onClick={() => setNewFor(newFor === pv.root ? null : pv.root)}>＋</button>
          <button className="mini" title="remove project" onClick={() => removeProject(pv.root)}>✕</button>
        </div>

        {newFor === pv.root && (
          <div className="newform">
            <input placeholder="branch (e.g. feat/x)" value={newBranch} autoFocus
              onChange={(e) => setNewBranch(e.currentTarget.value)}
              onKeyDown={(e) => e.key === "Enter" && createPlace(pv.root)} />
            <input placeholder="name (optional)" value={newName}
              onChange={(e) => setNewName(e.currentTarget.value)}
              onKeyDown={(e) => e.key === "Enter" && createPlace(pv.root)} />
            <button onClick={() => createPlace(pv.root)} disabled={!newBranch.trim()}>Create</button>
          </div>
        )}

        {open && pv.ok && (
          <>
            {main && <ul className="places"><PlaceRow repo={pv.root} p={main} /></ul>}
            {LIVE_TIERS.filter((g) => buckets[g]?.length).map((g) => {
              const key = `${pv.root}|${g}`;
              const opened = isOpen(key, g !== "idle"); // idle collapsed by default
              return (
                <div className="group" key={key}>
                  <GroupHeader gkey={key} label={GROUP_LABEL[g]} count={buckets[g].length} open={opened} onToggle={() => toggleGroup(key, g !== "idle")} />
                  {opened && <ul className="places">{buckets[g].map((p) => <PlaceRow key={p.slug} repo={pv.root} p={p} />)}</ul>}
                </div>
              );
            })}
            {dormant.length > 0 && (() => {
              const key = `${pv.root}|dormant`;
              const opened = isOpen(key, false);
              return (
                <div className="group dormant" key={key}>
                  <div className="group-h dormant-h" onClick={() => toggleGroup(key, false)}>
                    <span className="caret">{opened ? "▾" : "▸"}</span>
                    Dormant<span className="count">{dormant.length}</span>
                  </div>
                  {opened && DORMANT_TIERS.filter((t) => buckets[t]?.length).map((t) => (
                    <div className="subgroup" key={t}>
                      <div className="subdiv">{GROUP_LABEL[t]}</div>
                      <ul className="places">{buckets[t].map((p) => <PlaceRow key={p.slug} repo={pv.root} p={p} />)}</ul>
                    </div>
                  ))}
                </div>
              );
            })()}
          </>
        )}
      </div>
    );
  };

  // flat lens (recent / attention) across all projects
  const FlatLens = ({ items }: { items: { pv: ProjectView; p: Place }[] }) => (
    <ul className="places flat">
      {items.length === 0 && <li className="flat-empty">Nothing here.</li>}
      {items.map(({ pv, p }) => <PlaceRow key={pv.root + p.slug} repo={pv.root} p={p} showProject />)}
    </ul>
  );

  const recentItems = useMemo(
    () => allPlaces.filter(({ p }) => matchPlace(p) && !p.is_main)
      .sort((a, b) => (b.p.declared?.last_opened_epoch ?? 0) - (a.p.declared?.last_opened_epoch ?? 0)),
    [allPlaces, q],
  );
  const attentionItems = useMemo(
    () => allPlaces.filter(({ p }) => matchPlace(p) && hasAttention(p)),
    [allPlaces, q],
  );

  const RAIL = [
    { key: "places" as Lens, icon: "▤", title: "Places — the full tree" },
    { key: "recent" as Lens, icon: "◷", title: "Recent — resurface dormant places" },
    { key: "attention" as Lens, icon: "⚠", title: "Attention — dirty / ahead-behind / broken" },
  ];

  return (
    <div className="app" style={{ gridTemplateColumns: `var(--rail-w) ${settings.nav_width}px 1fr` }}>
      {/* ── activity rail ── */}
      <nav className="rail">
        {RAIL.map((r) => (
          <button key={r.key} className={"rail-icon" + (lens === r.key ? " active" : "")} title={r.title} onClick={() => changeLens(r.key)}>
            {r.icon}
          </button>
        ))}
        <div className="rail-spacer" />
        <button className="rail-icon" title="add project" onClick={addProject}>＋</button>
        <button className="rail-icon" title="settings (⌘,)" onClick={() => setSettingsOpen(true)}>⚙</button>
      </nav>

      {/* ── nav ── */}
      <aside className="nav">
        <div className="nav-head">
          <span className="nav-title">{lens === "places" ? "PLACES" : lens === "recent" ? "RECENT" : "ATTENTION"}</span>
          <button className="icon-btn" title="focus search" onClick={() => searchRef.current?.focus()}>⌕</button>
        </div>
        <input ref={searchRef} className="search" placeholder="filter places…" value={filter} onChange={(e) => setFilter(e.currentTarget.value)} />
        {err && <div className="err">{err}</div>}
        <div className="nav-scroll">
          {ws && ws.projects.length === 0 && <div className="empty small">No projects yet.<br />Click ＋ to add one.</div>}
          {lens === "places" && ws?.projects.map((pv) => <ProjectNode key={pv.root} pv={pv} />)}
          {lens === "recent" && <FlatLens items={recentItems} />}
          {lens === "attention" && (
            <>
              <FlatLens items={attentionItems} />
              {ws?.projects.filter((pv) => !pv.ok).map((pv) => (
                <div className="project broken-flat" key={pv.root}><span className="rollup broken">⊘</span> {basename(pv.root)} <span className="pgone">repo gone</span></div>
              ))}
            </>
          )}
        </div>
        <button className="add-footer" onClick={addProject}>＋ Add project</button>
        <div className="nav-resizer" onMouseDown={onResize} />
      </aside>

      {/* ── main ── */}
      <main className="main">
        {selected && sel ? (
          <>
            <header className="topbar">
              <div className="identity">
                <b className="slug">{selected.is_main ? "◆ " : ""}{selected.slug}</b>
                {selected.branch && (
                  <span className={"branch" + (!selected.is_main && selected.branch !== selected.slug ? " hi" : "")}>
                    {!selected.is_main && selected.branch !== selected.slug ? "↗ " : ""}{selected.branch}
                  </span>
                )}
                <span className="status-cluster">
                  {selected.tmux_session.up && <span className="s ok" title="tmux live"><span className="status-dot live" style={{ background: "var(--ok)" }} /> live</span>}
                  {selected.claude_session_present && <span className="s ai" title="AI session">✦ ai</span>}
                  {selected.dirty && <span className="s dirty">● {selected.dirty_files ?? ""}</span>}
                  {(selected.ahead || selected.behind) && <span className="s ab">↑{selected.ahead ?? 0} ↓{selected.behind ?? 0}</span>}
                  <span className={"life " + selected.lifecycle_effective}>{selected.lifecycle_effective}</span>
                </span>
              </div>

              <div className="controls">
                {selected.tmux_session.up ? (
                  <span className="live-badge" title="session live"><span className="status-dot live" style={{ background: "var(--ok)" }} /> live</span>
                ) : (
                  <button className="enter-btn" onClick={() => enterPlace(sel.repo, selected)}>Enter ▸</button>
                )}
                <button className={"icon-btn" + (selected.declared?.pinned ? " on" : "")} title={selected.declared?.pinned ? "unpin" : "pin"}
                  onClick={() => mutate(invoke("set_pin", { repo: sel.repo, slug: sel.slug, on: !selected.declared?.pinned }))}>★</button>

                <div className="menu-wrap">
                  <button className="ctrl" onClick={() => setMenu(menu === "life" ? null : "life")}>Lifecycle ▾</button>
                  {menu === "life" && (
                    <div className="popover right">
                      <div className="pop-hint">active / idle are derived</div>
                      {SETTABLE.map((s) => (
                        <button key={s.value} className="pop-item" onClick={() => { mutate(invoke("set_lifecycle", { repo: sel.repo, slug: sel.slug, label: s.value })); setMenu(null); }}>
                          <span className="check">{selected.declared?.lifecycle === s.value ? "✓" : ""}</span>{s.label}
                        </button>
                      ))}
                    </div>
                  )}
                </div>

                {!selected.is_main && (
                  <div className="menu-wrap">
                    <button className="ctrl" onClick={() => setMenu(menu === "more" ? null : "more")}>⋯</button>
                    {menu === "more" && (
                      <div className="popover right">
                        <button className="pop-item" onClick={() => { navigator.clipboard?.writeText(selected.path).catch(() => {}); setMenu(null); }}>Copy path</button>
                        <button className={"pop-item danger" + (confirmRm === `${sel.repo}|${sel.slug}` ? " armed" : "")} onClick={doRemove}>
                          {confirmRm === `${sel.repo}|${sel.slug}` ? "Confirm remove?" : "Remove place…"}
                        </button>
                      </div>
                    )}
                  </div>
                )}
              </div>
            </header>

            <input className="note-strip" placeholder="note…" defaultValue={selected.declared?.note ?? ""}
              key={sel.repo + sel.slug + (selected.declared?.note ?? "")}
              onBlur={(e) => mutate(invoke("set_note", { repo: sel.repo, slug: sel.slug, note: e.currentTarget.value }))} />

            {selected.tmux_session.up ? (
              <TerminalPane key={selected.tmux_session.name} session={selected.tmux_session.name} termVersion={termVersion} />
            ) : (
              <div className="term-empty">
                <div className="term-empty-card">
                  <div className="te-title">No live session for <b>{selected.slug}</b></div>
                  <button className="enter-btn big" onClick={() => enterPlace(sel.repo, selected)}>Enter ▸ to start</button>
                </div>
              </div>
            )}

            <footer className="statusbar">
              <div className="switch-wrap">
                {!selected.is_main && (
                  <>
                    <span className="sb-label">⎇</span>
                    <input className="switchto" placeholder="switch branch…" value={switchTo}
                      onChange={(e) => setSwitchTo(e.currentTarget.value)}
                      onKeyDown={(e) => e.key === "Enter" && doSwitch()} />
                    <button className="ctrl sm" onClick={doSwitch} disabled={!switchTo.trim()}>Switch</button>
                  </>
                )}
              </div>
              <div className="sb-facts">
                {selected.tmux_session.up ? <>tmux <span className="ok">●</span> up · {selected.tmux_session.name}</> : <>tmux <span className="off">○</span> down</>}
                {selected.claude_session_present ? " · pane0 claude" : ""}
              </div>
            </footer>
          </>
        ) : (
          <div className="briefing">
            <h1>Welcome back.</h1>
            <div className="chips">
              <span className="chip"><span className="dot" style={{ background: "var(--ok)" }} /> {stats.live} live</span>
              <span className="chip"><span className="dot" style={{ background: "var(--dirty)" }} /> {stats.dirty} dirty</span>
              <span className="chip"><span className="dot" style={{ background: "var(--ai)" }} /> {stats.ai} AI</span>
            </div>
            <div className="resume-h">RESUME WHERE YOU LEFT OFF</div>
            <div className="resume">
              {resume.length === 0 && <div className="empty small">No places yet — ＋ add a project.</div>}
              {resume.map(({ pv, p }) => (
                <div className="resume-row" key={pv.root + p.slug} onClick={() => enterPlace(pv.root, p)}>
                  <span className="status-dot" style={{ background: DOT_COLOR[p.lifecycle_effective] ?? "var(--sticky)" }} />
                  <span className="rr-name">{p.declared?.pinned ? "★ " : ""}{p.slug}</span>
                  <span className="rr-proj">{basename(pv.root)}</span>
                  <span className="rr-life">{p.lifecycle_effective}</span>
                  <span className="rr-age">{ago(p.declared?.last_opened_epoch)}</span>
                  <button className="enter-btn sm">Enter ▸</button>
                </div>
              ))}
            </div>
            <div className="briefing-foot">＋ Add a project to get started</div>
          </div>
        )}
      </main>

      <SettingsSheet open={settingsOpen} settings={settings} onChange={updateSettings} onClose={() => setSettingsOpen(false)} />
      {menu && <div className="menu-catch" onClick={() => setMenu(null)} />}
    </div>
  );
}

export default App;
