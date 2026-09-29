// Settings → pi (pi-harness §5, §8, §9.4): which pi and node a pane will get,
// each model pi offers and why an unusable one is unusable, the trust mode, and
// the repos the user has allowed to load their own pi resources.
//
// The allowance is written only from here (and `worktrees trust pi`): it is the
// user's act. It lives in ~/.config/worktrees/config.toml, never in pi's own
// trust.json, and never in a repo.

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Settings } from "./settings";
import { ModelPicker, REASON_TEXT, modelArg, type ModelOption, type PiStatus } from "./ModelPicker";

export default function PiPanel({ settings, repo, onChange, onReport }: {
  settings: Settings;
  repo: string | null;
  onChange: (patch: Partial<Settings>) => void;
  onReport: (msg: string) => void;
}) {
  const [status, setStatus] = useState<PiStatus | null>(null);
  const [models, setModels] = useState<ModelOption[] | null>(null);
  const reload = useCallback(() => {
    invoke<PiStatus>("pi_status", { repo }).then(setStatus).catch((e) => onReport(`pi: ${String(e)}`));
    setModels(null);
    invoke<ModelOption[]>("agent_models", { harness: "pi" }).then(setModels).catch((e) => { setModels([]); onReport(`pi models: ${String(e)}`); });
  }, [repo, onReport]);
  useEffect(() => { reload(); }, [reload]);

  const allow = async (root: string, on: boolean) => {
    try {
      await invoke("set_pi_allowed", { repo: root, allow: on });
      reload();
    } catch (e) { onReport(`pi allowance: ${String(e)}`); }
  };

  const pf = status?.preflight;
  return <>
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
      <div className="hint">Preselected when you start pi. pi's own default is never used: it may be a provider nobody signed in to. A model LM Studio serves but pi's models.json does not declare is not offered — pi cannot use it.</div>
    </section>

    <section className="setting" data-testid="pi-trust">
      <label>Project trust</label>
      <div className="seg seg-plain">
        {([["never", "Skip repo resources"], ["ask", "Let pi ask"]] as const).map(([mode, label]) => (
          <button key={mode} className={settings.pi_project_trust === mode ? "on" : ""}
            onClick={() => onChange({ pi_project_trust: mode })}>{label}</button>
        ))}
      </div>
      <div className="hint">{settings.pi_project_trust === "never" ? <b>Skip:</b> : "Skip:"} pi launches with --no-approve, so a repo's .pi/ extensions, settings and .agents/skills do not load. AGENTS.md and CLAUDE.md still do.</div>
      <div className="hint">{settings.pi_project_trust === "ask" ? <b>Ask:</b> : "Ask:"} pi shows its own trust prompt, whose highlighted choice is Trust. The place's dot shows it as waiting, and nothing is typed into it for you.</div>
      <label className="sub">Allowed repos</label>
      {status?.allowed.length ? <div className="ver-rows">{status.allowed.map((r) => (
        <div className="ver-row" key={r}><code>{r}</code>
          <button className="ctrl sm" data-testid="pi-revoke" onClick={() => allow(r, false)}>Revoke</button></div>
      ))}</div> : <div className="hint">None. pi lanes in an allowed repo launch with --approve and load its own resources — code from that repo runs inside pi.</div>}
      {status?.repo_root && !status.repo_allowed && (
        <div className="ver-actions">
          <button className="ctrl sm" data-testid="pi-allow-repo" onClick={() => allow(status.repo_root!, true)}>
            Allow {status.repo_root.split("/").pop()}
          </button>
        </div>
      )}
      <div className="hint">Kept in ~/.config/worktrees/config.toml as [trust] pi. pi's own trust.json is never written, so this does not reach a pi you run by hand. Also: <code>worktrees trust pi [--revoke]</code>.</div>
    </section>
  </>;
}
