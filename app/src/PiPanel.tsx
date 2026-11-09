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

/** pi's catalog: read once per mount, and again after a hosts edit. */
export function usePiModels(onReport: (msg: string) => void) {
  const [models, setModels] = useState<ModelOption[] | null>(null);
  const seq = useRef(0);
  const reload = useCallback(() => {
    const n = ++seq.current;
    invoke<ModelOption[]>("agent_models", { harness: "pi" })
      .then((v) => { if (seq.current === n) setModels(v); })
      .catch((e) => { if (seq.current === n) setModels([]); onReport(`pi models: ${String(e)}`); });
    // eslint-disable-next-line react-hooks/exhaustive-deps -- onReport is per-render
  }, []);
  useEffect(() => { reload(); }, [reload]);
  return { models, reload };
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

/** `pi_hosts`: every host a pi worktrees starts can use (pimodels::Hosts). */
export type PiHostModel = { id: string; name: string | null; context_window: number | null; max_tokens: number | null; reasoning: boolean | null; images: boolean | null };
export type PiHost = {
  backend: string; base_url: string | null; api: string | null; has_key: boolean; models: PiHostModel[];
  origin: "pi" | "worktrees"; overridden: boolean;
  reach: "served" | "up" | "down" | "unprobed"; served_undeclared: string[];
};
export type PiHosts = {
  path: string; file: "missing" | "ok" | "unreadable"; problem: string | null; load_error: string | null;
  own_path: string; own_problem: string | null; hosts: PiHost[];
};
/** `pi_host_edit`'s argument (api `PiHostEdit`). No variant carries a key. */
export type PiHostEdit =
  | { op: "add"; id: string; base_url: string; api?: string }
  | { op: "set_url"; id: string; base_url: string }
  | { op: "remove"; id: string }
  | { op: "override"; id: string }
  | { op: "declare"; id: string; model: string; context_window?: number; max_tokens?: number }
  | { op: "update_model"; id: string; model: string; context_window?: number; max_tokens?: number }
  | { op: "remove_model"; id: string; model: string };

export const PI_HOST_APIS = ["openai-completions", "openai-responses"] as const;

/** `pi_hosts`, and the one write. Only the newest read lands; every write is
 *  followed by a re-read here and of the catalog (`onChanged`), so the dots
 *  and the default-model picker follow without a restart. */
export function usePiHosts(onChanged: () => void, onFail: (e: unknown) => void) {
  const [hosts, setHosts] = useState<PiHosts | null>(null);
  const [busy, setBusy] = useState(false);
  const seq = useRef(0);
  const read = useCallback(() => {
    const n = ++seq.current;
    invoke<PiHosts>("pi_hosts")
      .then((v) => { if (seq.current === n) setHosts(v); })
      .catch((e) => { if (seq.current === n) onFail(`pi hosts: ${String(e)}`); });
    // eslint-disable-next-line react-hooks/exhaustive-deps -- onFail is per-render
  }, []);
  useEffect(() => { read(); }, [read]);
  const edit = async (e: PiHostEdit): Promise<boolean> => {
    setBusy(true);
    try {
      await invoke("pi_host_edit", { edit: e });
      return true;
    } catch (err) {
      onFail(`pi hosts: ${String(err)}`);
      return false;
    } finally {
      setBusy(false);
      read();
      onChanged();
    }
  };
  return { hosts, busy, edit, read };
}

const fmtTokens = (n: number | null, dflt: number) => {
  const v = n ?? dflt;
  const s = v >= 1000 ? `${Math.round(v / 100) / 10}K` : String(v);
  return n == null ? `${s} (pi's default)` : s;
};
const REACH_TEXT: Record<PiHost["reach"], string> = {
  served: "answering", up: "answering (its model list is not readable)", down: "not answering", unprobed: "pi's built-in endpoint",
};
const toCount = (s: string): number | undefined => {
  const n = Number(s.trim());
  return s.trim() === "" || !Number.isInteger(n) || n <= 0 ? undefined : n;
};

/** Settings → pi → Models: the hosts (worktrees' own, editable; pi's own,
 *  read-only), the models each declares and the ones it serves undeclared,
 *  and the default (the same `default_models.pi` Settings → Agents shows). */
export function PiModelsSection({ settings, models, onChange, reloadModels, onFail }: {
  settings: Settings;
  models: ModelOption[] | null;
  onChange: (patch: Partial<Settings>) => void;
  reloadModels: () => void;
  onFail: (e: unknown) => void;
}) {
  const { hosts, busy, edit } = usePiHosts(reloadModels, onFail);
  const arm = useArmed<string>();
  const [adding, setAdding] = useState({ id: "", url: "http://127.0.0.1:1234/v1", api: "openai-completions" });
  const [urlEdit, setUrlEdit] = useState<{ id: string; url: string } | null>(null);
  const [modelEdit, setModelEdit] = useState<{ id: string; model: string; ctx: string; max: string } | null>(null);
  const dflt = settings.default_models.pi ?? "";
  const row = (backend: string, model: string) => models?.find((o) => o.model.backend === backend && o.model.model === model);
  const shown = new Set((hosts?.hosts ?? []).filter((h) => !(h.origin === "pi" && h.overridden)).map((h) => h.backend));
  const others = (models ?? []).filter((o) => !shown.has(o.model.backend ?? ""));
  const usesDefault = (id: string, model?: string) => dflt !== "" && (model ? dflt === `${id}/${model}` : dflt.startsWith(`${id}/`));
  const host = (h: PiHost) => {
    const ours = h.origin === "worktrees";
    const key = `${h.origin}:${h.backend}`;
    const shadow = h.origin === "pi" && h.overridden;
    return (
      <div className="pi-host" key={key} data-testid="pi-host" data-origin={h.origin}>
        <div className="pi-host-head">
          <span className={"pi-dot" + (h.reach === "served" || h.reach === "up" ? " ok" : h.reach === "down" ? " bad" : "")} aria-hidden="true" />
          <b>{h.backend}</b>
          <span className="pi-host-tag">{ours ? (h.overridden ? "overridden in worktrees" : "worktrees") : shadow ? "pi's config · replaced by worktrees" : "pi's config"}</span>
          {!shadow && <span className="pi-host-state">{REACH_TEXT[h.reach]}</span>}
        </div>
        {urlEdit?.id === h.backend && ours ? (
          <div className="pi-host-form">
            <input type="text" aria-label={`${h.backend} base URL`} value={urlEdit.url} onChange={(e) => setUrlEdit({ ...urlEdit, url: e.target.value })} />
            <button className="ctrl sm" disabled={busy} onClick={() => void edit({ op: "set_url", id: h.backend, base_url: urlEdit.url }).then((ok) => ok && setUrlEdit(null))}>Save</button>
            <button className="ctrl sm" onClick={() => setUrlEdit(null)}>Cancel</button>
          </div>
        ) : h.base_url && <div className="ver-row"><code>{h.base_url}</code>{h.api && <> · {h.api}</>}</div>}
        {!shadow && <div className="ver-rows">
          {h.models.length === 0 && <div className="hint">No models declared{ours ? " yet: pi will not list this host until one is." : "."}</div>}
          {h.models.map((m) => {
            const o = row(h.backend, m.id);
            const mk = `${h.backend}/${m.id}`;
            if (ours && modelEdit?.id === h.backend && modelEdit.model === m.id) return (
              <div className="pi-host-form" key={mk}>
                <code>{m.id}</code>
                <input type="number" min={1} aria-label="context window" placeholder="context (128000)" value={modelEdit.ctx} onChange={(e) => setModelEdit({ ...modelEdit, ctx: e.target.value })} />
                <input type="number" min={1} aria-label="max output" placeholder="max out (16384)" value={modelEdit.max} onChange={(e) => setModelEdit({ ...modelEdit, max: e.target.value })} />
                <button className="ctrl sm" disabled={busy} onClick={() => void edit({ op: "update_model", id: h.backend, model: m.id, context_window: toCount(modelEdit.ctx), max_tokens: toCount(modelEdit.max) }).then((ok) => ok && setModelEdit(null))}>Save</button>
                <button className="ctrl sm" onClick={() => setModelEdit(null)}>Cancel</button>
              </div>
            );
            return (
              <div className="ver-row pi-host-model" key={mk} data-testid="pi-host-model">
                <span className={"pi-dot" + (o?.ready ? " ok" : "")} aria-hidden="true" />
                <code>{m.id}</code>
                <span className="pi-host-meta">{fmtTokens(m.context_window, 128000)} context · {fmtTokens(m.max_tokens, 16384)} out{o?.reason && <> — {REASON_TEXT[o.reason]}</>}</span>
                {ours && <span className="pi-host-acts">
                  <button className="ctrl sm" onClick={() => setModelEdit({ id: h.backend, model: m.id, ctx: m.context_window ? String(m.context_window) : "", max: m.max_tokens ? String(m.max_tokens) : "" })}>Edit</button>
                  <button className={"ctrl sm danger" + (arm.armed === `m:${mk}` ? " armed" : "")} disabled={busy} data-testid="pi-model-remove"
                    onClick={() => arm.confirm(`m:${mk}`, () => void edit({ op: "remove_model", id: h.backend, model: m.id }))}>
                    {arm.armed === `m:${mk}` ? "Remove — sure?" : "Remove"}</button>
                </span>}
              </div>
            );
          })}
          {h.served_undeclared.map((id) => (
            <div className="ver-row pi-host-model undeclared" key={`u:${h.backend}/${id}`} data-testid="pi-served-undeclared">
              <span className="pi-dot" aria-hidden="true" />
              <code>{id}</code><span className="pi-host-meta">served, not declared</span>
              {ours && <span className="pi-host-acts"><button className="ctrl sm" disabled={busy} data-testid="pi-declare"
                onClick={() => void edit({ op: "declare", id: h.backend, model: id })}>Declare</button></span>}
            </div>
          ))}
          {!ours && h.served_undeclared.length > 0 && <div className="hint">Override this host in worktrees to declare one.</div>}
        </div>}
        <div className="ver-actions">
          {ours && urlEdit?.id !== h.backend && <button className="ctrl sm" onClick={() => setUrlEdit({ id: h.backend, url: h.base_url ?? "" })}>Edit URL</button>}
          {ours && <button className={"ctrl sm danger" + (arm.armed === `h:${h.backend}` ? " armed" : "")} disabled={busy} data-testid="pi-host-remove"
            onClick={() => arm.confirm(`h:${h.backend}`, () => void edit({ op: "remove", id: h.backend }))}>
            {arm.armed === `h:${h.backend}` ? (h.overridden ? "Remove override — sure?" : "Remove host — sure?") : (h.overridden ? "Remove override" : "Remove host")}</button>}
          {!ours && !h.overridden && h.base_url && <button className="ctrl sm" disabled={busy} data-testid="pi-host-override"
            onClick={() => void edit({ op: "override", id: h.backend })}>Override in worktrees</button>}
        </div>
        {arm.armed === `h:${h.backend}` && <div className="hint">{h.overridden
          ? `pi worktrees starts goes back to pi's own ${h.backend} from its next start.`
          : `Its models are gone from pi worktrees starts from their next start.`}{usesDefault(h.backend) && ` Your default model, ${dflt}, is on this host.`}</div>}
        {arm.armed?.startsWith(`m:${h.backend}/`) && usesDefault(h.backend, arm.armed.slice(`m:${h.backend}/`.length)) &&
          <div className="hint">{dflt} is your default model.</div>}
        {ours && h.overridden && <div className="hint">This host's models come from worktrees now, and later edits to pi's own config for it do not reach pi worktrees starts. Its key is still pi's: worktrees never copies one.</div>}
      </div>
    );
  };
  return (
    <section className="setting" data-testid="pi-models">
      <label>Models</label>
      <div className="hint">The hosts worktrees adds here exist only for pi started by worktrees: a pi you start yourself reads only {hosts?.path ?? "~/.pi/agent/models.json"}, and a session on one of these models, resumed there, starts on another model without a warning. A pi that is already running picks up a change when it is restarted.</div>
      {hosts === null ? <div className="hint">reading pi's hosts…</div> : <>
        {hosts.own_problem && <div className="hint warn" data-testid="pi-own-problem">worktrees cannot use {hosts.own_path}: {hosts.own_problem}</div>}
        {(hosts.problem || hosts.load_error) && <div className="hint warn" data-testid="pi-load-error">pi cannot load {hosts.path}: {hosts.load_error ?? hosts.problem}. Every host it declares is unusable until it is fixed.</div>}
        {hosts.hosts.length === 0 && <div className="hint">No model hosts yet.</div>}
        <div className="pi-hosts">{hosts.hosts.map(host)}</div>
        <label className="sub">Add a host</label>
        <div className="pi-host-form">
          <input type="text" aria-label="host name" placeholder="name (e.g. lm-studio-local)" value={adding.id} onChange={(e) => setAdding({ ...adding, id: e.target.value })} />
          <input type="text" aria-label="base URL" placeholder="http://127.0.0.1:1234/v1" value={adding.url} onChange={(e) => setAdding({ ...adding, url: e.target.value })} />
          <select aria-label="API" value={adding.api} onChange={(e) => setAdding({ ...adding, api: e.target.value })}>
            {PI_HOST_APIS.map((a) => <option key={a} value={a}>{a}</option>)}
          </select>
          <button className="ctrl sm" data-testid="pi-host-add" disabled={busy || !adding.id.trim() || !adding.url.trim()}
            onClick={() => void edit({ op: "add", id: adding.id.trim(), base_url: adding.url.trim(), api: adding.api }).then((ok) => ok && setAdding({ ...adding, id: "" }))}>Add</button>
        </div>
        <div className="hint">For a local server that needs no key (LM Studio, Ollama, vLLM, llama.cpp). Then declare the models it serves. A host that needs a key belongs in pi's own config (<code>pi auth</code>); override it here to change its models.</div>
      </>}
      {others.length > 0 && <>
        <label className="sub">Other providers</label>
        <div className="ver-rows">{others.map((o) => (
          <div className="ver-row" key={modelArg(o)}>
            <span className={"pi-dot" + (o.ready ? " ok" : "")} aria-hidden="true" />
            <code>{modelArg(o)}</code>{o.reason && <> — {REASON_TEXT[o.reason]}</>}
          </div>
        ))}</div>
        <div className="hint">pi's own providers and sign-in: <code>pi auth</code>.</div>
      </>}
      <label className="sub">Default model</label>
      <ModelPicker harness="pi" value={dflt} options={models} testid="pi-default-model"
        onChange={(m) => onChange({ default_models: { ...settings.default_models, pi: m || undefined } })} />
      <div className="hint">Preselected when you start pi, in this window. pi's own default is never used: it may be a provider nobody signed in to.</div>
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
