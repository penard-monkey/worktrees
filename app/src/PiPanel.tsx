// Settings → pi (pi-harness §5, §8, §9.4): which pi and node a pane will get,
// each model pi offers and why an unusable one is unusable — and, under
// Settings → Agent policy since the regroup (settings-reorg §3.2), the trust
// mode and the repos the user has allowed to load their own pi resources.
//
// The allowance is written only from here (and `worktrees trust pi`): it is the
// user's act. It lives in ~/.config/worktrees/config.toml, never in pi's own
// trust.json, and never in a repo.

import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "./ipc";
import type { PiProjectTrust, Settings } from "./settings";
import { ModelPicker, REASON_TEXT, modelArg, type ModelOption, type PiLaunchTrust, type PiStatus } from "./ModelPicker";
import { useArmed } from "./useArmed";
import { EnvLine } from "./EnvLine";

/** What decided the trust of a pi lane in the repo in focus, in words. */
export function launchTrustText(t: PiLaunchTrust, trustPath: string): string {
  const at = t.pi_entry?.path ?? "";
  switch (t.source) {
    case "shadowed": return "Launches with --no-approve: this repo's .pi/mcp.json defines its own worktrees server, which would replace the Worktrees tools. Nothing else overrides that.";
    case "allowance": return "Allowed below: launches with --approve.";
    case "pi-trusted": return `pi's own trust applies: ${trustPath} trusts ${at}, so lanes here launch with --approve.`;
    case "pi-untrusted": return `pi's own trust applies: ${trustPath} marks ${at} as not trusted, so lanes here launch with --no-approve. Allowing the repo below overrides it.`;
    case "ask": return "Neither worktrees nor pi's trust.json decides for this repo: pi will ask.";
    default: return "Neither worktrees nor pi's trust.json allows this repo: lanes launch with --no-approve.";
  }
}

/** `pi_status` for `repo`, and the allowance write. Only the newest read may
 *  land: a trust click (`PiTrustSection`) invalidates any read already in
 *  flight, so the panel ends on what the host stored after the last click.
 *  Once a trust click has succeeded (`held`), each landed read's trust is also
 *  copied into `Settings.pi_project_trust` for an older build (`settings.ts`)
 *  — not before, while `trust` may still be env or user config speaking. */
export function usePiStatus({ settings, repo, onChange, onReport, onFail }: {
  settings: Settings;
  repo: string | null;
  onChange: (patch: Partial<Settings>) => void;
  onReport: (msg: string) => void;
  /** A refused trust change: App's `fail`, so it is never swallowed. */
  onFail: (e: unknown) => void;
}) {
  const [status, setStatus] = useState<PiStatus | null>(null);
  const seq = useRef(0);
  const held = useRef(false);
  // A ref, so App's per-render `onChange` does not re-read on every render.
  const mirror = useRef<(t: PiProjectTrust) => void>(() => {});
  mirror.current = (t) => { if (settings.pi_project_trust !== t) onChange({ pi_project_trust: t }); };
  const readStatus = useCallback(() => {
    const n = ++seq.current;
    invoke<PiStatus>("pi_status", { repo })
      .then((v) => {
        if (seq.current !== n) return;
        setStatus(v);
        if (held.current) mirror.current(v.trust);
      })
      .catch((e) => onReport(`pi: ${String(e)}`));
  }, [repo, onReport]);
  useEffect(() => { readStatus(); }, [readStatus]);
  const allow = async (root: string, on: boolean) => {
    try {
      await invoke("set_pi_allowed", { repo: root, allow: on });
      readStatus();
    } catch (e) { onReport(`pi allowance: ${String(e)}`); }
  };
  return { status, setStatus, readStatus, seq, held, allow, onFail };
}
export type PiState = ReturnType<typeof usePiStatus>;

/** pi's catalog, read once per mount. */
export function usePiModels(onReport: (msg: string) => void): ModelOption[] | null {
  const [models, setModels] = useState<ModelOption[] | null>(null);
  useEffect(() => {
    invoke<ModelOption[]>("agent_models", { harness: "pi" }).then(setModels).catch((e) => { setModels([]); onReport(`pi models: ${String(e)}`); });
    // eslint-disable-next-line react-hooks/exhaustive-deps -- one read per mount
  }, []);
  return models;
}

/** Settings → pi → pi: which pi and node a pane gets. */
export function PiInstallSection({ status }: { status: PiStatus | null }) {
  const pf = status?.preflight;
  return (
    <section className="setting" data-testid="pi-install">
      <label>pi</label>
      {!status ? <div className="hint">reading…</div> : pf?.pi_path ? <div className="ver-rows">
        <div className="ver-row">pi <b>{pf.pi_version ?? "?"}</b> · <code>{pf.pi_path}</code></div>
        <div className="ver-row">node <b>{pf.node_version ?? "?"}</b> · <code>{pf.node_path ?? "(none)"}</code>
          {pf.node_floor && <> · pi needs ≥ {pf.node_floor}</>}</div>
      </div> : <div className="hint">pi is not installed for your shell. Install it with <code>curl -fsSL https://pi.dev/install.sh | sh</code> — the installer can set up a node for it.</div>}
      {pf?.problem && <div className="hint warn" data-testid="pi-problem">{pf.problem}</div>}
      <div className="hint">Measured the way a pane runs it (your login shell), since the node pi runs on is whatever that shell finds first.</div>
    </section>
  );
}

/** Settings → pi → Models: the catalog with each unusable model's reason, and
 *  the default (the same `default_models.pi` Settings → Agents shows). */
export function PiModelsSection({ settings, models, onChange }: {
  settings: Settings;
  models: ModelOption[] | null;
  onChange: (patch: Partial<Settings>) => void;
}) {
  return (
    <section className="setting" data-testid="pi-models">
      <label>Models</label>
      {models === null ? <div className="hint">reading pi's models…</div>
        : models.length === 0 ? <div className="hint">pi lists no models this build can read.</div>
        : <div className="ver-rows">{models.map((o) => (
          <div className="ver-row" key={modelArg(o)}>
            <span className={"pi-dot" + (o.ready ? " ok" : "")} aria-hidden="true" />
            <code>{modelArg(o)}</code>{o.reason && <> — {REASON_TEXT[o.reason]}</>}
          </div>
        ))}</div>}
      <label className="sub">Default model</label>
      <ModelPicker harness="pi" value={settings.default_models.pi ?? ""} options={models} testid="pi-default-model"
        onChange={(m) => onChange({ default_models: { ...settings.default_models, pi: m || undefined } })} />
      <div className="hint">Preselected when you start pi, in this window. pi's own default is never used: it may be a provider nobody signed in to. A model LM Studio serves but pi's models.json does not declare is not offered — pi cannot use it.</div>
    </section>
  );
}

/** Settings → Agent policy → pi: the trust mode and the allowed repos. A
 *  Revoke is two clicks (settings-reorg §6 item 3): a lane may be relying on
 *  the allowance it removes. */
export function PiTrustSection({ pi }: { pi: PiState }) {
  const { status, setStatus, readStatus, seq, held, onFail } = pi;
  const allow = (root: string, on: boolean) => void pi.allow(root, on);
  const arm = useArmed<string>();
  // Project trust is HOST policy (`host-policy.json`), read back through
  // `pi_status.trust` and changed only by the click below — never by this
  // panel or the app loading (hostpolicy-check.mjs). Optimistic, rolled back
  // on a refusal; the re-read after it also refreshes "This repo", which the
  // trust mode decides.
  const setTrust = (mode: PiProjectTrust) => {
    if (!status || status.trust === mode) return;
    const prev = status.trust;
    const n = ++seq.current;
    setStatus({ ...status, trust: mode });
    invoke("set_pi_trust", { mode }).then(() => { held.current = true; readStatus(); }, (e) => {
      if (seq.current === n) setStatus((s) => (s ? { ...s, trust: prev } : s));
      onFail(`pi trust: ${String(e)}`);
      readStatus();
    });
  };
  return (
    <section className="setting" data-testid="pi-trust">
      <label>pi</label>
      <label className="sub">Project trust</label>
      <div className="seg seg-plain">
        {([["never", "Skip repo resources"], ["ask", "Let pi ask"]] as const).map(([mode, label]) => (
          <button key={mode} className={status?.trust === mode ? "on" : ""} disabled={!status}
            onClick={() => setTrust(mode)}>{label}</button>
        ))}
      </div>
      <div className="hint">{status?.trust === "never" ? <b>Skip:</b> : "Skip:"} pi launches with --no-approve, so a repo's .pi/ extensions, settings and .agents/skills do not load. AGENTS.md and CLAUDE.md still do.</div>
      <div className="hint">{status?.trust === "ask" ? <b>Ask:</b> : "Ask:"} pi shows its own trust prompt, whose highlighted choice is Trust. The place's dot shows it as waiting, and nothing is typed into it for you.</div>
      <div className="hint">Either way, a folder pi itself already trusts (its own trust.json, e.g. from "Trust parent folder") launches with --approve, and one pi marks as not trusted with --no-approve.</div>
      <EnvLine origin={status?.trust_origin} outranks="control" />
      {status?.launch && <div className="hint" data-testid="pi-trust-source"><b>This repo:</b> {launchTrustText(status.launch, status.pi_trust_path ?? "trust.json")}</div>}
      <label className="sub">Allowed repos</label>
      {status?.allowed.length ? <div className="ver-rows">{status.allowed.map((r) => (
        <div className="ver-row" key={r}><code>{r}</code>
          <button className={"ctrl sm danger" + (arm.armed === r ? " armed" : "")} data-testid="pi-revoke"
            onClick={() => arm.confirm(r, () => allow(r, false))}>{arm.armed === r ? "Revoke — sure?" : "Revoke"}</button></div>
      ))}</div> : <div className="hint">None. pi lanes in an allowed repo launch with --approve and load its own resources — code from that repo runs inside pi.</div>}
      {arm.armed && <div className="hint">A pi lane in that repo launches with --no-approve from its next start.</div>}
      <div className="hint">Allowed means: the repo's pi extensions, skills and .pi/mcp.json servers run with no prompt when a lane starts. .pi/mcp.json comes with each BRANCH while the allowance covers the whole repo, so a branch you check out — a fork's PR included — brings its own. One that defines a worktrees server is never approved.</div>
      {status?.repo_root && !status.repo_allowed && (
        <div className="ver-actions">
          <button className="ctrl sm" data-testid="pi-allow-repo" onClick={() => allow(status.repo_root!, true)}>
            Allow {status.repo_root.split("/").pop()}
          </button>
        </div>
      )}
      <div className="hint">Kept in ~/.config/worktrees/config.toml as [trust] pi. pi's own trust.json is read, never written, so this does not reach a pi you run by hand. Also: <code>worktrees trust pi [--revoke]</code>.</div>
    </section>
  );
}
