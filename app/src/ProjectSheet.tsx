import { useCallback, useEffect, useRef, useState } from "react";
import { useEscape } from "./useEscape";
import * as Icons from "./icons";
import { invoke } from "@tauri-apps/api/core";
import type { ProfilesInfo } from "./ProfilesPanel";
import { AgentFixButton, AgentSetupSection, openPr, useAgentSetup, type AgentSetupStatus } from "./AgentSetup";
import { projectTodos, type Remedies, type Todo, type TodoHealth } from "./projectTodos";

// Right-side slide-over for ONE project (proposal §10). It stays a sheet:
// unlike Settings, it is about the thing selected in the tree behind it.
//
// Why a sheet and not a pane: the app's selection model is `{repo, slug}` and the
// main pane is binary (place | briefing). A real project pane means touching ~30
// `sel?.repo` read sites plus three subtle effects, and is only worth it if the
// project view has to host a terminal. A sheet is one new App state, one new
// context-menu item, and it reuses .scrim / .settings-sheet / .setting /
// .ver-rows / .ver-actions / .update-log wholesale.
//
// ⚠ The file is NOT named Project.tsx on purpose: macOS's filesystem is
// case-insensitive and `Settings.tsx` collided with `settings.ts` once already.

type CmdResult = { ok: boolean; code: number; output: string; slug?: string | null; warnings?: string[] };

export type ProjectFileView = { path: string; mode: string };
export type ProjectPortsView = { stride: number; max_slots: number; base: [string, number][] };
/** `files` is docker's `-f` list, in docker's order — see lib.rs. */
export type ProjectComposeView = { files: string[]; project: string };
/** `[docs]` — which paths the Docs tab walks, and where reading starts.
 *  Shown here because its EFFECT is on another surface entirely, which makes
 *  it the section easiest to write wrong and never notice. */
export type ProjectDocsView = { paths: string[]; index: string | null };
export type ProjectConfigView = {
  path: string;
  exists: boolean;
  files: ProjectFileView[];
  ports: ProjectPortsView | null;
  compose: ProjectComposeView | null;
  docs: ProjectDocsView | null;
  error: string | null;
  warnings: string[];
};

export type DoctorFinding = {
  severity: "info" | "warn" | "error";
  code: string;
  place: string | null;
  path: string | null;
  message: string;
};
export type DoctorReport = { code: number; schema_version: number; findings: DoctorFinding[]; error: string | null };

export type SuggestedFile = { path: string; credential: boolean };
/** `model::Stray` — a worktree git registers outside `.worktrees/`. */
export type Stray = { path: string; branch: string | null; slug: string };
export type InitSuggestion = {
  path: string;
  exists: boolean;
  qualifies: boolean;
  files: SuggestedFile[];
  credentials: number;
  ports: boolean;
  compose: boolean;
  stale_places: string[];
  truncated: boolean;
  toml: string;
  hash: string;
};

const basename = (p: string) => p.replace(/\/+$/, "").split("/").pop() || p;

/** Slugs a report blames — the row-decoration set behind the drift glyph.
 * `info` findings are deliberately excluded: a bound port is almost always this
 * place's OWN running stack, and a drifted copy is the expected steady state
 * after a script rewrote it (§7). Badging those would train people to ignore
 * the glyph. */
export function driftedSlugs(r: DoctorReport | null | undefined): Set<string> {
  const out = new Set<string>();
  for (const f of r?.findings ?? []) if (f.place && f.severity !== "info") out.add(f.place);
  return out;
}

/** Every actionable finding — the per-place ones PLUS the ones attached to no
 * place (an `unknown-key` warn, a project-wide slot conflict). This is the ONE
 * count in the app: the sheet's Health badge and the project context menu both
 * use it, so they can no longer disagree by the width of a placeless finding.
 * The nav GLYPH is necessarily narrower (it decorates a row, so it can only show
 * findings that name a row) — that asymmetry is real, and the two are labelled
 * differently because of it. */
export function issueCount(r: DoctorReport | null | undefined): number {
  return (r?.findings ?? []).filter((f) => f.severity !== "info").length;
}

/** What each actionable finding needs, by the ONE command (or the lack of
 * one) that clears it. The To do list turns this into a row per remedy, and it
 * has to be right, because a row's button is a promise: `issueCount` under a
 * Relink button claimed a plain relink would clear a `copy-stale` it
 * deliberately leaves alone (materialize::plan_one), a `no-slot` that only
 * provision allocates, and an `unknown-key` nothing but an edit removes.
 *
 * Same population as `issueCount` (non-info), so the four always sum to it.
 * The codes are `diag::Code`, kebab-cased. `dangling-link` is NOT relink's:
 * it travels with `missing-source` (the file is gone from main), which no
 * command here can conjure. Anything unknown — a code added to core after this
 * list — lands in `manual`, the one bucket that promises no button. */
export function remedies(r: DoctorReport | null | undefined): Remedies {
  const out: Remedies = { relink: 0, force: 0, provision: 0, manual: 0 };
  for (const f of r?.findings ?? []) {
    if (f.severity === "info") continue;
    switch (f.code) {
      case "not-linked":
      case "wrong-mode":
        out.relink++; break;
      case "shadowed":
      case "copy-stale":
        out.force++; break;
      case "no-slot":
      case "missing-port":
        out.provision++; break;
      default:
        out.manual++;
    }
  }
  return out;
}

/** A report that did not RUN — `cmd_doctor` exited on a guard (an unreadable
 * `.worktrees.toml`, a bad worktree name) before it could emit any JSON, so
 * `findings: []` here means "nothing was measured", never "nothing is wrong".
 * Every consumer has to branch on this BEFORE it reads `findings`. */
export function reportFailed(r: DoctorReport | null | undefined): boolean {
  return !r || r.error !== null || r.code === 1;
}

/** What `relink --force` would rewrite: a copy that differs from main, and a real
 * file shadowing a declared link. Core backs each one up to `.bak`/`.bak.2`
 * before it touches it (§7) — but the displaced content may be the only copy, so
 * the button that calls this is armed, counted, and never the default. */
const forcible = (r: DoctorReport | null) =>
  (r?.findings ?? []).filter((f) => f.code === "copy-stale" || f.code === "shadowed");

// Module scope with props (CLAUDE.md): a component defined inside App() gets a
// fresh identity every render, remounting its DOM and dropping input focus.
export function ProjectSheet({
  open,
  root,
  editorCmd,
  suggestion,
  strays = [],
  onClose,
  onReport,
  onConfigWritten,
  todoFocus = false,
  health = null,
  agentStatus = null,
  onAgentLoaded,
}: {
  open: boolean;
  /** Opened from "Repair / upgrade…" or the header's to-do badge: bring the
   *  To do list into view (it is the first section, so this is belt-and-braces
   *  for a sheet that was already scrolled). */
  todoFocus?: boolean;
  /** App's last sweep for this root — what the To do list shows until the
   *  sheet's own doctor run and agent probe land, so it is never empty for the
   *  second those take. */
  health?: TodoHealth | null;
  agentStatus?: AgentSetupStatus | null;
  /** Every agent-setup read this sheet makes (on open, Re-check, after a
   *  Fix) — App stores it, so the header badge and the menu agree with the
   *  sheet at once instead of at the next 5-minute sweep. */
  onAgentLoaded: (root: string, s: AgentSetupStatus) => void;
  /** Worktrees registered outside `.worktrees/` (from the snapshot's `strays`). */
  strays?: Stray[];
  root: string;
  editorCmd: string;
  /** What `init` would suggest here (App probes it once per project). */
  suggestion: InitSuggestion | null;
  onClose: () => void;
  /** Hand the fresh report back so the nav's drift glyphs track this sheet. */
  onReport: (root: string, r: DoctorReport | null) => void;
  /** A config was just written — App re-probes so the banner retires. */
  onConfigWritten: (root: string) => void;
}) {
  const [cfg, setCfg] = useState<ProjectConfigView | null>(null);
  const [report, setReport] = useState<DoctorReport | null>(null);
  const [checking, setChecking] = useState(false);
  const [running, setRunning] = useState<"" | "relink" | "force" | "provision" | "init">("");
  const [log, setLog] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [pinfo, setPinfo] = useState<ProfilesInfo | null>(null);
  const [showToml, setShowToml] = useState(false);
  // Arm-then-confirm for the one destructive control in this sheet (`--force`),
  // same two-click shape as .pop-item.danger / .ctrl.sm.danger elsewhere. It is
  // disarmed by every state change below — an armed button surviving a re-check
  // would fire against findings it was never aimed at.
  const [armed, setArmed] = useState(false);

  // Errors are NEVER swallowed: they surface in this sheet's own error area AND
  // land in app.log (ProjectSheet has no fail() — same shape as SettingsSheet).
  const note = useCallback((m: string) => {
    setErr(m);
    invoke("log_event", { level: "error", msg: m }).catch(() => {});
  }, []);

  // For a section that shows its OWN error inline (Agent setup): log it, but do
  // not also paint it into the Health section's error area.
  const logError = useCallback((m: string) => {
    invoke("log_event", { level: "error", msg: m }).catch(() => {});
  }, []);

  // The agent-setup state, owned here so the To do row and the Agent setup
  // section press ONE Fix with one arm (see `useAgentSetup`).
  const agent = useAgentSetup(root, open, onAgentLoaded, logError);
  // Set when a To do row ran something, so that row's output shows beside it
  // instead of only in the section further down.
  const [todoRan, setTodoRan] = useState(false);
  const bodyRef = useRef<HTMLDivElement | null>(null);
  const todoRef = useRef<HTMLElement | null>(null);

  // One re-read of everything this sheet shows. Also the `finally` step of every
  // action below — see the hazard note on `run`.
  const refresh = useCallback(async () => {
    setChecking(true);
    setArmed(false);
    try {
      // The mock harness returns null for an unmocked command, so every typed
      // invoke here has to tolerate null rather than assume a shape.
      const c = await invoke<ProjectConfigView | null>("project_config_read", { repo: root });
      setCfg(c ?? null);
    } catch (e) {
      note(`read ${root} config failed: ${String(e)}`);
    }
    try {
      const r = await invoke<DoctorReport | null>("doctor", { repo: root, slug: null });
      setReport(r ?? null);
      onReport(root, r ?? null);
      if (r?.error) setErr(r.error);
    } catch (e) {
      note(`doctor ${root} failed: ${String(e)}`);
    } finally {
      setChecking(false);
    }
  }, [root, note, onReport]);

  useEffect(() => {
    if (!open) return;
    setLog("");
    setErr(null);
    setShowToml(false);
    setArmed(false);
    setTodoRan(false);
    refresh();
    // On sheet-open only: profiles_info does a filesystem probe per profile, so
    // it must never ride the 3s poll.
    invoke<ProfilesInfo | null>("profiles_info", { repo: root })
      .then((i) => setPinfo(i))
      // Swallowing this would leave the section rendering "(use the default) /
      // Sessions here launch unprofiled" — which may be false, in the direction
      // that matters most for a restrictive profile.
      .catch((e) => note(`could not read AI profiles: ${e}`));
  }, [open, refresh, root]);

  useEscape(onClose, open);

  useEffect(() => {
    if (open && todoFocus) todoRef.current?.scrollIntoView({ block: "nearest" });
  }, [open, todoFocus]);

  // Every action shares this shape, copied from the CLI-update block in
  // SettingsSheet INCLUDING the hazard it already solved: state is re-read in
  // `finally` BEFORE the buttons come back, because a stale-enabled button in
  // the re-check window would re-run the whole repair on a click.
  const run = async (kind: "relink" | "force" | "provision" | "init", cmdline: string, cmd: string, args: Record<string, unknown>) => {
    if (running) return;
    setRunning(kind);
    setArmed(false);
    setErr(null);
    setLog(`$ worktrees ${cmdline}\n`);
    try {
      const r = await invoke<CmdResult | null>(cmd, args);
      const out = r?.output?.trim() ? r.output : "(no output)";
      const tail = r?.ok ? "\n✓ done" : `\n✗ exit ${r?.code ?? "?"}`;
      // `warnings` is the Warn subset of the SAME lines `output` already carries
      // (CaptureUi::push writes both), so echo only what the output missed —
      // otherwise every stale-copy warning prints twice. Still never swallowed:
      // a warning absent from output is appended exactly as before.
      const missed = (r?.warnings ?? []).filter((w) => !out.includes(w));
      const warn = missed.length ? "\n" + missed.map((w) => `! ${w}`).join("\n") : "";
      setLog((l) => l + out + warn + tail);
      if (kind === "init" && r?.ok) onConfigWritten(root);
    } catch (e) {
      setLog((l) => l + `\n✗ ${String(e)}`);
      invoke("log_event", { level: "error", msg: `${cmd} ${root}: ${String(e)}` }).catch(() => {});
    } finally {
      await refresh();
      setRunning("");
    }
  };

  // Route through open_editor, NEVER openPath: the opener capability grants
  // open-url + reveal-item-in-dir only, and a missing permission rejects the
  // invoke SILENTLY (CLAUDE.md).
  const openConfig = () => {
    if (!cfg?.exists) return;
    invoke("open_editor", { path: cfg.path, cmd: editorCmd }).catch((e) =>
      note(`open ${cfg.path} failed: ${String(e)}`),
    );
  };

  if (!open) return null;

  // A report that failed to RUN is not a report of zero problems. Every read of
  // `report.findings` below is gated on this.
  const broken = report !== null && reportFailed(report);
  const issues = broken ? 0 : issueCount(report);
  const busy = running !== "";
  const canSuggest = !!suggestion?.qualifies && !cfg?.exists;
  // Force is offered only when something force would actually repair is present
  // at warn/error. The COUNT includes the info-level drifted copies, because
  // `--force` re-seeds those too — the confirm must name what it will rewrite,
  // not what it was triggered by.
  const forceable = broken ? [] : forcible(report);
  const offerForce = forceable.some((f) => f.severity !== "info");
  // The sheet's own reads win once they exist; App's sweep fills the gap.
  const todoHealth: TodoHealth | null = report
    ? { issues, error: broken ? (report.error ?? "doctor could not run") : null, remedies: remedies(broken ? null : report) }
    : health;
  const canProvision = !!cfg?.ports && !cfg?.error;
  // The Health section's --force arm, shared: one armed state, whichever of
  // the two buttons armed it.
  const reseed = () => {
    if (!armed) { setArmed(true); return; }
    run("force", "relink --all --force", "relink", { repo: root, slug: null, force: true });
  };
  const todos = projectTodos(todoHealth, agent.status ?? agentStatus);
  const canRelink = !!cfg?.exists && !cfg?.error;
  const show = (section: Todo["section"]) => {
    const sel = section === "health" ? '[data-section="health"]' : '[data-testid="agent-setup"]';
    bodyRef.current?.querySelector<HTMLElement>(sel)?.scrollIntoView({ block: "start", behavior: "smooth" });
  };

  return (
    <div className="scrim" onClick={onClose}>
      <aside className="settings-sheet project-sheet" onClick={(e) => e.stopPropagation()}>
        <header className="settings-h">
          <b>Project · {basename(root)}</b>
          <button className="icon-btn" title="close (Esc)" onClick={onClose}><Icons.X /></button>
        </header>

        <div className="settings-body" ref={bodyRef}>
          {(todos.length > 0 || todoFocus) && (
            <section className="setting" ref={todoRef} data-testid="project-todos">
              <label>To do</label>
              {todos.length === 0 ? (
                <div className="hint">✓ Nothing to repair or upgrade here.</div>
              ) : (
                <div className="todo-list">
                  {todos.map((t, i) => {
                    // The Fix PR carries the dirs AND the skills: its button
                    // shows once, on the first row that uses it, and the other
                    // row's label already says "same PR".
                    const firstFix = t.action === "agent-fix" && todos.findIndex((x) => x.action === "agent-fix") === i;
                    return (
                      <div className="todo-row" key={t.id} data-todo={t.id}>
                        <span className={"todo-dot " + t.sev} />
                        <span className="todo-label">{t.label}</span>
                        <span className="todo-acts">
                          {t.action === "relink" && (
                            <button className="ctrl sm" data-testid="todo-relink" disabled={busy || !canRelink}
                              title={canRelink ? "worktrees relink --all" : "no readable .worktrees.toml — see Health"}
                              onClick={() => { setTodoRan(true); run("relink", "relink --all", "relink", { repo: root, slug: null, force: false }); }}>
                              {running === "relink" ? "Relinking…" : "Relink"}
                            </button>
                          )}
                          {t.action === "force" && (
                            <button className={"ctrl sm danger" + (armed ? " armed" : "")} data-testid="todo-force"
                              disabled={busy || !canRelink}
                              title="re-seed declared copies from main and move a shadowing file aside as .bak"
                              onClick={() => { setTodoRan(true); reseed(); }}>
                              {running === "force" ? "Re-seeding…" : armed ? `Overwrite ${forceable.length} file${forceable.length === 1 ? "" : "s"}?` : "Re-seed…"}
                            </button>
                          )}
                          {t.action === "provision" && (
                            <button className="ctrl sm" data-testid="todo-provision" disabled={busy || !canProvision}
                              title={canProvision ? "worktrees provision --all" : "no [ports] in a readable .worktrees.toml — see Health"}
                              onClick={() => { setTodoRan(true); run("provision", "provision --all", "provision", { repo: root, slug: null }); }}>
                              {running === "provision" ? "Provisioning…" : "Provision"}
                            </button>
                          )}
                          {firstFix && <AgentFixButton ctl={agent} onPress={() => setTodoRan(true)} />}
                          {t.id === "agent-pending" && agent.outcome?.pr_url && (
                            <button className="ctrl sm" onClick={() => openPr(agent.outcome!.pr_url!, logError)}>
                              Open PR <Icons.ExternalLink size={12} />
                            </button>
                          )}
                          <button className="ctrl sm" data-testid={`todo-show|${t.id}`} onClick={() => show(t.section)}>Details</button>
                        </span>
                      </div>
                    );
                  })}
                </div>
              )}
              {todoRan && log && <pre className="update-log">{log}</pre>}
              {todoRan && agent.log && <pre className="update-log">{agent.log}</pre>}
            </section>
          )}

          <section className="setting">
            <label>
              Config
              {cfg?.error ? <span className="upd-tag warn">unreadable</span> : !cfg?.exists ? <span className="upd-tag warn">none</span> : null}
            </label>
            <div className="ver-rows">
              <div className="ver-row"><span className="ver-path" title={cfg?.path ?? root}>{cfg?.path ?? "…"}</span></div>
              {cfg && !cfg.exists && (
                <div className="ver-row"><i>no .worktrees.toml — this project behaves exactly as it does today</i></div>
              )}
              {cfg?.exists && (
                <>
                  <div className="ver-row">
                    files <b>{cfg.files.length}</b>
                    {cfg.ports ? <> · ports <b>{cfg.ports.base.length}</b> (stride {cfg.ports.stride}, ≤{cfg.ports.max_slots} slots)</> : null}
                    {cfg.compose ? <> · compose <b>{cfg.compose.files.join(" + ")}</b></> : null}
                    {cfg.docs ? <> · docs <b>{cfg.docs.paths.length || "convention"}</b></> : null}
                  </div>
                  {cfg.files.map((f) => (
                    <div className="ver-row dx-file" key={f.path}>
                      <span className={"dx-mode " + f.mode}>{f.mode}</span>
                      <span className="ver-path" title={f.path}>{f.path}</span>
                    </div>
                  ))}
                  {cfg.compose && (
                    <div className="ver-row">project name <b>{cfg.compose.project}</b></div>
                  )}
                  {cfg.docs?.index && (
                    <div className="ver-row">docs open on <b>{cfg.docs.index}</b></div>
                  )}
                  {cfg.docs?.paths.map((p) => (
                    <div className="ver-row dx-file" key={p}>
                      <span className="dx-mode link">docs</span>
                      <span className="ver-path" title={p}>{p}</span>
                    </div>
                  ))}
                </>
              )}
            </div>
            <div className="ver-actions">
              <button className="ctrl sm" onClick={openConfig} disabled={!cfg?.exists}>Open .worktrees.toml</button>
              <button className="ctrl sm" onClick={refresh} disabled={checking || busy}>
                {checking ? "Checking…" : "Re-check"}
              </button>
            </div>
            <div className="hint">
              Committed project structure, shared with the CLI. Read-only here — edit the file.
            </div>
            {cfg?.error && <pre className="update-log">{cfg.error}</pre>}
            {cfg?.warnings.map((w, i) => <div className="hint" key={i}>! {w}</div>)}
          </section>

          {strays.length > 0 && (
            <section className="setting">
              <label>
                Worktrees outside .worktrees/
                <span className="upd-tag warn">{strays.length}</span>
              </label>
              <div className="ver-rows">
                {strays.map((s) => (
                  <div className="ver-row" key={s.path}>
                    <span className="ver-path" title={s.path}>{s.path}</span>
                    {" "}<b>{s.branch ?? "(detached)"}</b>
                  </div>
                ))}
              </div>
              <div className="hint">
                Git registers these for this repo, but they live outside <code>.worktrees/</code> — made
                by hand or by another tool — so nothing here can see them. Adopting one is a move;
                nothing is moved for you, because a tmux session or an editor may be sitting in it:
              </div>
              <pre className="update-log">{strays.map((s) => `git -C ${root} worktree move ${s.path} ${root}/.worktrees/${s.slug}`).join("\n")}</pre>
            </section>
          )}

          <section className="setting">
            <label>AI profile</label>
            <div className="row2">
              <select
                value={pinfo?.assigned_id ?? ""}
                disabled={!!pinfo?.env_override}
                onChange={async (e) => {
                  const id = e.target.value || null;
                  try {
                    await invoke("set_project_profile", { repo: root, id });
                    setPinfo(await invoke<ProfilesInfo | null>("profiles_info", { repo: root }));
                  } catch (err) { note(String(err)); }
                }}
              >
                <option value="">(use the default)</option>
                {(pinfo?.profiles ?? []).map((p) => (
                  <option key={p.id} value={p.id}>{p.name}</option>
                ))}
              </select>
            </div>
            <div className="hint">
              A project profile REPLACES the global default for this repo — the two do not merge.
              {pinfo?.effective_id
                ? ` Sessions here launch with \u201c${(pinfo.profiles ?? []).find((p) => p.id === pinfo.effective_id)?.name ?? pinfo.effective_id}\u201d.`
                : " Sessions here launch unprofiled."}
            </div>
            {pinfo?.env_override ? (
              <div className="hint"><code>WORKTREES_PROFILE={pinfo.env_override}</code> overrides this choice.</div>
            ) : null}
            {pinfo?.repo_has_unprofiled_history && !pinfo?.effective_id ? (
              <div className="hint">
                Heads up: this repo already has Claude conversations under <code>~/.claude</code>. Binding a
                profile starts a FRESH conversation, because history lives with the profile. Nothing is
                deleted — unbind and the old one comes back.
              </div>
            ) : null}
          </section>

          <section className="setting" data-section="health">
            <label>
              Health
              {broken ? <span className="upd-tag warn">unchecked</span> : null}
              {issues > 0 ? <span className="upd-tag warn">{issues} {issues === 1 ? "issue" : "issues"}</span> : null}
            </label>
            {report === null ? (
              <div className="hint">doctor hasn't run yet.</div>
            ) : broken ? (
              // The guard message itself is in the `err` pre at the bottom of the
              // section; this says what its ABSENCE of findings means, which is
              // the thing that used to read as "✓ clean".
              <div className="dx-list">
                <div className="dx-row">
                  <span className="dx-sev error">✗</span>
                  <span className="dx-msg">
                    doctor could not run here — nothing was checked. Until the config parses,
                    every command in this project refuses, and the drift shown in the nav is
                    whatever was last measured.
                    <span className="dx-code">unchecked</span>
                  </span>
                </div>
              </div>
            ) : report.findings.length === 0 ? (
              <div className="hint">✓ clean — every declared file is materialized.</div>
            ) : (
              <div className="dx-list">
                {report.findings.map((f, i) => (
                  <div className="dx-row" key={i}>
                    <span className={"dx-sev " + f.severity}>{f.severity === "error" ? "✗" : f.severity === "warn" ? "▸" : "·"}</span>
                    <span className="dx-msg">
                      {f.place ? <b className="dx-place">{f.place}</b> : null}
                      {f.place ? " " : null}
                      {f.message}
                      <span className="dx-code">{f.code}</span>
                    </span>
                  </div>
                ))}
              </div>
            )}
            <div className="ver-actions">
              {/* An unreadable config makes both of these refuse in core anyway —
                  offering them would just print the parse error twice. */}
              <button className="ctrl sm" disabled={busy || !cfg?.exists || !!cfg?.error}
                onClick={() => run("relink", "relink --all", "relink", { repo: root, slug: null, force: false })}>
                {running === "relink" ? "Relinking…" : "Relink files"}
              </button>
              <button className="ctrl sm" disabled={busy || !cfg?.ports || !!cfg?.error}
                onClick={() => run("provision", "provision --all", "provision", { repo: root, slug: null })}>
                {running === "provision" ? "Provisioning…" : "Provision ports"}
              </button>
              {offerForce && (
                <button
                  className={"ctrl sm danger" + (armed ? " armed" : "")}
                  disabled={busy}
                  title="re-seed declared copies from main and move a shadowing file aside as .bak"
                  onClick={reseed}
                >
                  {running === "force"
                    ? "Re-seeding…"
                    : armed
                      ? `Overwrite ${forceable.length} file${forceable.length === 1 ? "" : "s"}?`
                      : "Re-seed from main…"}
                </button>
              )}
            </div>
            <div className="hint">
              Relink re-applies the file plan to every worktree; a real file that shadows a declared
              link, and a copy that has drifted, are reported and left ALONE. Provision allocates or
              adopts a port slot and writes .worktree.env.
            </div>
            {offerForce && (
              // Without this the warn is un-repairable from the app: the plain
              // Relink above takes no action on it by design, and the finding's
              // own fix text names a flag only the CLI had.
              <div className="hint">
                Re-seed is the <b>--force</b> pass, and it is the only way to clear a stale copy: it
                rewrites every declared copy from main and moves a shadowing file aside first, as
                .bak (then .bak.2, never overwriting a backup). The content it displaces may be the
                only copy of it.
              </div>
            )}
            {log && <pre className="update-log">{log}</pre>}
            {err && <pre className="update-log">{err}</pre>}
          </section>

          <AgentSetupSection ctl={agent} onError={logError} />

          {canSuggest && suggestion && (
            <section className="setting">
              <label>Suggested config<span className="upd-tag warn">init</span></label>
              <div className="ver-rows">
                <div className="ver-row">
                  {suggestion.files.length} file{suggestion.files.length === 1 ? "" : "s"}
                  {suggestion.credentials > 0 ? <> · <b>{suggestion.credentials} credential</b></> : null}
                  {suggestion.ports ? " · ports" : ""}
                  {suggestion.compose ? " · compose" : ""}
                </div>
                {suggestion.stale_places.length > 0 && (
                  <div className="ver-row">
                    <i>{suggestion.stale_places.length} existing worktree(s) already missing a file: {suggestion.stale_places.join(", ")}</i>
                  </div>
                )}
              </div>
              <div className="ver-actions">
                <button className="ctrl sm" onClick={() => setShowToml((v) => !v)}>
                  {showToml ? "Hide preview" : "Preview"}
                </button>
                <button className="ctrl sm" disabled={busy}
                  onClick={() => run("init", "init", "init_write", { repo: root })}>
                  {running === "init" ? "Writing…" : "Write .worktrees.toml"}
                </button>
              </div>
              <div className="hint">
                Nothing is written until you press Write. Credential files fail SILENTLY when a
                worktree is missing them — that is what this file is for.
                {suggestion.truncated ? " (The search stopped early — add anything it missed by hand.)" : ""}
              </div>
              {showToml && <pre className="update-log">{suggestion.toml}</pre>}
            </section>
          )}
        </div>
      </aside>
    </div>
  );
}

// The §9 passive nudge, in the nav under a qualifying project's header. Box
// treatment copied from .nav-newform (the app has no banner component, and
// inventing a second card idiom for one line of text is not worth it).
//
// Dismissal is keyed by the suggestion's CONTENT HASH, not a boolean, so a repo
// that later gains a credential file re-suggests (§9). It persists in
// ui-state.json's `init_dismissed`, alongside `collapsed` / `manual_order` —
// which are also per-project-root maps.
/** Worktrees git registers for this repo that live outside `.worktrees/` —
 *  made by hand, or by another tool (Claude Code's own worktree feature puts
 *  them in `.claude/worktrees/`). Every command here is keyed on the place dir,
 *  so a stray is invisible to `ls`, to the nav and to this app, while holding a
 *  branch and often uncommitted work.
 *
 *  The data has always been in the snapshot (`ls --json`'s `strays`) and the
 *  adopt commands have always been in the project sheet. What was missing was a
 *  signal anyone could read: a bare `⊟` in the header strip, explained only in
 *  a `title`. Eight of them sat unnoticed in a real project, four with
 *  uncommitted changes and one twelve commits ahead.
 *
 *  NOT an offer, and deliberately not built on `offers.ts`. That registry is
 *  for after-update suggestions, its context may "ask about the MACHINE, never
 *  about which screen you are on" (a stray is per-PROJECT), and every offer is
 *  dismissible — which its own doc rules out for this class: a problem "belongs
 *  in Settings where it cannot be silenced". Hence no dismiss button here. The
 *  banner leaves when the strays do, which is the only thing that should
 *  silence it.
 *
 *  Borrows `.init-banner`'s box, amber edge included: unlike the MCP card this
 *  IS something being wrong. */
export function StrayBanner({ count, onOpen }: { count: number; onOpen: () => void }) {
  return (
    <div className="init-banner">
      <div className="init-banner-h">
        <span className="init-banner-i">⊟</span>
        {count} outside <code>.worktrees/</code>
      </div>
      <p>
        Git registers {count === 1 ? "this worktree" : `these ${count} worktrees`} for this project, but
        {count === 1 ? " it lives" : " they live"} outside <code>.worktrees/</code> — so nothing here can
        see {count === 1 ? "it" : "them"}, and {count === 1 ? "it is" : "they are"} not listed above.
      </p>
      <div className="ver-actions">
        <button className="ctrl sm" data-track="nav.project.strays.adopt" onClick={onOpen}>Adopt…</button>
      </div>
    </div>
  );
}

export function InitBanner({ suggestion, onOpen, onDismiss }: {
  suggestion: InitSuggestion;
  onOpen: () => void;
  onDismiss: () => void;
}) {
  const n = suggestion.files.length;
  const c = suggestion.credentials;
  return (
    <div className="init-banner">
      <div className="init-banner-h">
        <span className="init-banner-i">⚑</span>
        Not configured
        <button className="mini" title="dismiss (until this project changes)" onClick={onDismiss}><Icons.X size={13} /></button>
      </div>
      <p>
        {n > 0
          ? `${n} gitignored file${n === 1 ? "" : "s"}${c > 0 ? ` (${c} credential${c === 1 ? "" : "s"})` : ""} in main ${n === 1 ? "is" : "are"} not linked into this project's worktrees.`
          : "This project publishes host ports with no per-worktree isolation."}
      </p>
      <div className="ver-actions">
        <button className="ctrl sm" onClick={onOpen}>Review…</button>
      </div>
    </div>
  );
}
