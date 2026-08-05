import { useCallback, useEffect, useState } from "react";
import * as Icons from "./icons";
import { invoke } from "@tauri-apps/api/core";
import type { ProfilesInfo } from "./ProfilesPanel";

// Right-side slide-over for ONE project — SettingsSheet's twin (proposal §10).
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
export type ProjectConfigView = {
  path: string;
  exists: boolean;
  files: ProjectFileView[];
  ports: ProjectPortsView | null;
  compose: ProjectComposeView | null;
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
  onClose,
  onReport,
  onConfigWritten,
}: {
  open: boolean;
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
    refresh();
    // On sheet-open only: profiles_info does a filesystem probe per profile, so
    // it must never ride the 3s poll.
    invoke<ProfilesInfo | null>("profiles_info", { repo: root })
      .then((i) => setPinfo(i))
      .catch(() => {});
  }, [open, refresh, root]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

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

  return (
    <div className="scrim" onClick={onClose}>
      <aside className="settings-sheet project-sheet" onClick={(e) => e.stopPropagation()}>
        <header className="settings-h">
          <b>Project · {basename(root)}</b>
          <button className="icon-btn" title="close (Esc)" onClick={onClose}><Icons.X /></button>
        </header>

        <div className="settings-body">
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
                deleted \u2014 unbind and the old one comes back.
              </div>
            ) : null}
          </section>

          <section className="setting">
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
                  onClick={() => {
                    if (!armed) { setArmed(true); return; }
                    run("force", "relink --all --force", "relink", { repo: root, slug: null, force: true });
                  }}
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
