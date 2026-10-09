import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/** A project's planning level (`planning::Level`). */
export type PlanningLevel = "off" | "show" | "full";
export type PlanningScope = "place" | "main";
export type PlanningDefault = "unset" | "full" | "off";

/** `planning_status` (lib.rs): machine-level — the global default and every
 *  registered project's own choice. Nothing here is about the project in
 *  focus, which is what lets the `planning` offer stand without one. */
export type PlanningStatus = {
  default: PlanningDefault;
  /** core's `planning::VERSION` — the offer's fingerprint. */
  version: number;
  config_path: string;
  projects: {
    root: string;
    name: string;
    /** Null = inherit the default. */
    level: PlanningLevel | null;
    plan_path: string | null;
    plan_scope: PlanningScope | null;
    effective: PlanningLevel;
  }[];
  /** The CLI the plan hook runs, and this app's own version: two binaries,
   *  so two releases (owned-planning §7 item 1). `cli_version` null = no
   *  worktrees CLI with the plan hook was found. */
  cli_path: string | null;
  cli_version: string | null;
  app_version: string;
};

/** The words for "the hook and the tab may disagree", or null when they run
 *  the same release. Shared by this panel and the Plan tab. */
export function cliSkew(s: Pick<PlanningStatus, "cli_version" | "app_version"> | null): string | null {
  if (!s) return null;
  if (!s.cli_version) return "No worktrees CLI with the plan hook was found, so agents get no plan reminders. Install or update the CLI (Settings → Updates).";
  if (s.cli_version !== s.app_version)
    return `Agents' plan hook runs worktrees CLI v${s.cli_version}; this app is v${s.app_version}. On different releases the hook and the Plan tab can name different plans — update the older one.`;
  return null;
}

/** The global choice. Two buttons, not three: "choose per project" and "off"
 *  store the same value (`off`) — each project's own level is what differs —
 *  so a third button could never show as selected after a reload. */
const GLOBAL: { id: "full" | "off"; label: string; hint: string }[] = [
  { id: "full", label: "On for all projects", hint: "Every project uses full planning unless you turn it off (or to show only) below." },
  { id: "off", label: "Choose per project", hint: "Off unless you turn it on for a project below, or when you add one." },
];

const LEVEL_WORDS: Record<PlanningLevel, string> = { off: "off", show: "show only", full: "full" };

/** One project's row: inherit / off / show only / full, and show-only's path.
 *  At module scope (a component defined inside another remounts every render
 *  and loses the half-typed path). */
function ProjectRow({ p, dflt, busy, onSet }: {
  p: PlanningStatus["projects"][number];
  dflt: PlanningLevel;
  busy: boolean;
  onSet: (level: PlanningLevel | null, path: string | null, scope: PlanningScope | null) => Promise<string | null>;
}) {
  const [choice, setChoice] = useState<PlanningLevel | "inherit">(p.level ?? "inherit");
  const [path, setPath] = useState(p.plan_path ?? "");
  const [scope, setScope] = useState<PlanningScope>(p.plan_scope ?? "place");
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => { setChoice(p.level ?? "inherit"); setPath(p.plan_path ?? ""); setScope(p.plan_scope ?? "place"); }, [p.level, p.plan_path, p.plan_scope]);
  const apply = async (c: PlanningLevel | "inherit", pth: string, sc: PlanningScope) => {
    if (c === "show" && !pth.trim()) { setErr(null); return; }
    const e = await onSet(c === "inherit" ? null : c, c === "show" ? pth : null, c === "show" ? sc : null);
    setErr(e);
    // A refused full must not keep showing "full": the control says what is
    // stored. A refused show path stays, so it can be corrected in place.
    if (e && c !== "show") setChoice(p.level ?? "inherit");
  };
  const dirty = choice === "show" && (path !== (p.plan_path ?? "") || scope !== (p.plan_scope ?? "place") || p.level !== "show");
  return <div className="planning-row" data-project={p.name}>
    <div className="planning-row-head">
      <b>{p.name}</b>
      <select value={choice} disabled={busy} aria-label={`Planning in ${p.name}`}
        onChange={(e) => {
          const c = e.currentTarget.value as PlanningLevel | "inherit";
          setChoice(c);
          setErr(null);
          if (c !== "show") void apply(c, path, scope);
        }}>
        <option value="inherit">Your default ({LEVEL_WORDS[dflt]})</option>
        <option value="off">Off</option>
        <option value="show">Show only…</option>
        <option value="full">Full</option>
      </select>
    </div>
    {choice === "show" && <div className="planning-show">
      <input className="planning-path" value={path} placeholder="docs/plan or docs/plan/goals.md" spellCheck={false}
        aria-label={`Plan path in ${p.name}`} disabled={busy}
        onChange={(e) => { setPath(e.currentTarget.value); setErr(null); }}
        onKeyDown={(e) => { if (e.key === "Enter") void apply("show", path, scope); }} />
      <div className="seg" role="radiogroup" aria-label="Read the path from">
        {(["place", "main"] as const).map((s) => (
          <button key={s} role="radio" aria-checked={scope === s} className={scope === s ? "on" : ""} disabled={busy}
            onClick={() => setScope(s)}>{s === "place" ? "each place" : "main's copy"}</button>
        ))}
      </div>
      <button className="ver-btn" disabled={busy || !path.trim() || !dirty} onClick={() => void apply("show", path, scope)}>Apply</button>
    </div>}
    {choice === "show" && <div className="hint">A file is the plan; a folder is shown through its <code>task_plan.md</code>. "Main's copy" reads main's working tree for every place — right for a goals file the orchestrator edits. The path is checked against main when you apply it, whichever you choose: one missing from main is fine for "each place", but one that is a link in main is refused even if lanes have a real folder there.</div>}
    {err && <div className="hint planning-err" role="alert">{err}</div>}
  </div>;
}

/** Settings → Planning (owned-planning §2.2). The explanation lives HERE, not
 *  in the offer row: the offer only carries you to it. Everything is user
 *  tier — a repo cannot set any of it. */
export function PlanningSection({ status, onStatus, onReport, offerPending = false, onSilenceOffer, "data-focus": focusId }: {
  status: PlanningStatus | null;
  /** Every read or write goes back to App: the offer follows it. */
  onStatus: (s: PlanningStatus) => void;
  onReport: (text: string) => void;
  offerPending?: boolean;
  onSilenceOffer?: () => void;
  "data-focus"?: string;
}) {
  const [busy, setBusy] = useState(false);
  // Re-read on open: `worktrees plan level` may have changed it since app start.
  useEffect(() => {
    invoke<PlanningStatus>("planning_status").then(onStatus).catch((e) => onReport(`planning_status: ${String(e)}`));
  }, []);
  const setDefault = async (d: PlanningDefault) => {
    setBusy(true);
    try { onStatus(await invoke<PlanningStatus>("set_planning_default", { default: d })); }
    catch (e) { onReport(`set_planning_default: ${String(e)}`); }
    finally { setBusy(false); }
  };
  /** Errors come back as the row's own text: a refused full or a bad path is
   *  something the user fixes right there. */
  const setProject = async (root: string, level: PlanningLevel | null, planPath: string | null, scope: PlanningScope | null) => {
    setBusy(true);
    try {
      onStatus(await invoke<PlanningStatus>("set_project_planning", { root, level, planPath, scope }));
      return null;
    } catch (e) {
      return String((e as { message?: string })?.message ?? e);
    } finally { setBusy(false); }
  };
  const dflt: PlanningLevel = status?.default === "full" ? "full" : "off";
  const skew = cliSkew(status);
  return <section className="setting" data-focus={focusId} data-testid="planning">
    <label>Planning</label>
    <div className="hint">Let Worktrees keep your agents' plans: one plan folder per place, the active plan always the one you see in the Plan tab, and agents reminded of it as they work.</div>
    <ul className="planning-how">
      <li>Each place keeps its plan in <code>.planning/&lt;topic&gt;/</code> — <code>task_plan.md</code>, <code>findings.md</code>, <code>progress.md</code>.</li>
      <li><code>.planning/.active_plan</code> says which plan is current, and Worktrees writes it when it creates the place.</li>
      <li>The Plan tab and your agents always see the same plan.</li>
      <li>Claude sessions Worktrees launches get the current phase at start and when the plan changes.</li>
      <li>Nothing is committed, and <code>.planning/</code> stays out of git.</li>
    </ul>
    <div className="hint"><b>What changes in a repo:</b> <code>.git/info/exclude</code> gains <code>/.planning/</code> if git does not already ignore it. It is local and never committed; no other file in your repo is written.</div>
    <div className="hint"><b>What is not changed:</b> your plans' content, your <code>.gitignore</code>, your <code>.claude/settings.json</code>, and any skill. Already have your own planning files or hooks? Choose <b>show only</b> for that project: the Plan tab shows your file, and nothing is injected or written.</div>
    {!status ? <div className="hint">Checking…</div> : <>
      <div className="seg" role="radiogroup" aria-label="Planning default">
        {GLOBAL.map((g) => {
          const on = status.default === g.id;
          return <button key={g.id} role="radio" aria-checked={on} className={on ? "on" : ""} disabled={busy} data-choice={g.id}
            onClick={() => !on && void setDefault(g.id)}>{g.label}</button>;
        })}
      </div>
      <div className="hint" data-testid="planning-default-hint">{status.default === "unset"
        ? "Not chosen yet — planning is off everywhere until you choose."
        : GLOBAL.find((g) => g.id === status.default)?.hint}</div>
      {status.projects.length === 0
        ? <div className="hint">No registered projects yet. Projects you add use your default, and the Add dialogs let you choose.</div>
        : <div className="planning-rows">{status.projects.map((p) => (
          <ProjectRow key={p.root} p={p} dflt={dflt} busy={busy}
            onSet={(level, path, scope) => setProject(p.root, level, path, scope)} />
        ))}</div>}
      <div className="hint"><b>Turning it off later</b> is a toggle: nothing is deleted. Plan folders and <code>.active_plan</code> stay and are read as before, and the exclude line stays. Running agents stop getting plan reminders on their next prompt; new launches carry no planning plugin.</div>
      <div className="hint" data-testid="planning-restart">A Claude session picks up the plan hooks when it is launched — restart running sessions to give them the hooks. Sessions created through an orchestrator&apos;s MCP server need that orchestrator restarted after an update.</div>
      {skew && <div className="hint planning-skew" data-testid="planning-skew">{skew}</div>}
      <div className="hint">Settings file: {status.config_path}</div>
      {offerPending && onSilenceOffer && <div className="ver-actions"><button className="mcp-dismiss" onClick={onSilenceOffer}>Stop suggesting this</button></div>}
    </>}
  </section>;
}

/** What a project-making dialog asks: the level, and show-only's path. */
export type PlanningPick = { level: PlanningLevel; path: string; scope: PlanningScope };

/** The default a dialog starts from: the global default, or full when the
 *  repo suggests it (`[plan]` in its `.worktrees.toml` — Add existing only,
 *  where the repo is on disk when the dialog shows). A suggestion pre-sets the
 *  choice; it never applies itself. */
export function initialPick(dflt: PlanningDefault | undefined, suggested: boolean): PlanningPick {
  return { level: suggested || dflt === "full" ? "full" : "off", path: "", scope: "place" };
}

/** Whether a pick needs writing: only when it differs from the default, so
 *  "inherit" stays the common case and changing the default later still
 *  means something (owned-planning §2.3). */
export function pickToWrite(p: PlanningPick, dflt: PlanningDefault | undefined): PlanningPick | null {
  const inherited: PlanningLevel = dflt === "full" ? "full" : "off";
  return p.level === inherited ? null : p;
}

/** The "Planning in this project" row of Add existing / New project / Clone.
 *  Module scope: it owns a text input inside dialogs App re-renders. */
export function PlanningChoice({ value, onChange, dflt, suggested = false, disabled = false }: {
  value: PlanningPick;
  onChange: (p: PlanningPick) => void;
  dflt: PlanningDefault | undefined;
  suggested?: boolean;
  disabled?: boolean;
}) {
  const set = (patch: Partial<PlanningPick>) => onChange({ ...value, ...patch });
  const why = [`your default: ${dflt === "full" ? "full" : "off"}`, suggested ? "this repo uses worktrees planning" : ""].filter(Boolean).join(" · ");
  return <div className="np-field planning-choice" data-testid="planning-choice">
    <span className="np-label">Planning in this project</span>
    <div className="seg" role="radiogroup" aria-label="Planning in this project">
      {(["off", "show", "full"] as const).map((l) => (
        <button key={l} role="radio" aria-checked={value.level === l} className={value.level === l ? "on" : ""} disabled={disabled}
          data-level={l} onClick={() => set({ level: l })}>{l === "show" ? "show only" : l}</button>
      ))}
    </div>
    {value.level === "show" && <div className="np-row">
      <input className="np-input" value={value.path} placeholder="docs/plan" spellCheck={false} disabled={disabled}
        aria-label="Plan path" data-testid="planning-choice-path" onChange={(e) => set({ path: e.currentTarget.value })} />
      <div className="seg" role="radiogroup" aria-label="Read the path from">
        {(["place", "main"] as const).map((s) => (
          <button key={s} role="radio" aria-checked={value.scope === s} className={value.scope === s ? "on" : ""} disabled={disabled}
            onClick={() => set({ scope: s })}>{s === "place" ? "this place" : "main's copy"}</button>
        ))}
      </div>
    </div>}
    <span className="hint">({why}) — see Settings → Planning</span>
  </div>;
}
