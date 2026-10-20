// Settings → Kimi: which kimi a pane will get, whether its lanes get the
// Worktrees tools, the models kimi's config declares (and the one to preselect),
// and the repos whose kimi lanes start trusted.
//
// Nothing here writes kimi's own files. The MCP choice is worktrees' own
// `kimi_mcp` setting: each kimi launch then writes (or removes) the server in
// the place's `.kimi-code/mcp.json` (core `kimiplace`). The allowance is
// `[trust] kimi` in ~/.config/worktrees/config.toml — the user's act, from here
// or `worktrees trust kimi`.

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Settings } from "./settings";
import { ModelPicker, modelArg, type ModelOption } from "./ModelPicker";

export type KimiMcpMode = "off" | "read-only" | "mutations";

export type KimiMcpStatus = {
  state: "mutations" | "read-only" | "off" | "cli-missing" | "kimi-missing";
  mode: KimiMcpMode;
  kimi_bin: string | null;
  worktrees_bin: string | null;
  file: string;
  config_path: string;
};

export type KimiLaunchTrust = { env: [string, string] | null; source: "allowance" | "shadowed" | "ask"; allowed: boolean; shadowed: boolean };

export type KimiStatus = {
  kimi_bin: string | null;
  kimi_version: string | null;
  mcp: KimiMcpStatus;
  allowed: string[];
  repo_root: string | null;
  repo_allowed: boolean;
  launch: KimiLaunchTrust | null;
};

const MCP_VERDICT: Record<KimiMcpStatus["state"], string> = {
  mutations: "Connected — kimi lanes can report, wait for and message other places, and create or close worktrees.",
  "read-only": "Connected with read-only tools.",
  off: "Not connected to kimi.",
  "cli-missing": "Install the Worktrees CLI first (Settings → Updates).",
  "kimi-missing": "kimi is not installed.",
};

/** What decided the trust of a kimi lane in the repo in focus, in words. */
export function kimiTrustText(t: KimiLaunchTrust): string {
  switch (t.source) {
    case "allowance": return "Allowed below: kimi lanes here start trusted, for that launch only.";
    case "shadowed": return "kimi asks here even though the repo is allowed: a worktrees MCP server that is not Worktrees' own would answer a trusted lane (the repo's .mcp.json, or a .kimi-code/mcp.json it ships).";
    default: return "kimi asks, once per place: \"Trust this folder?\". The place's dot shows it as waiting until you answer it in the terminal.";
  }
}

export function KimiMcpSection({ onReport, onStatus, offerPending = false, onSilenceOffer, "data-focus": focusId }: {
  onReport: (text: string) => void;
  "data-focus"?: string;
  /** Every status read or handed back — App keeps the `kimi-mcp` offer in step. */
  onStatus?: (s: KimiMcpStatus) => void;
  offerPending?: boolean;
  onSilenceOffer?: () => void;
}) {
  const [status, setStatusRaw] = useState<KimiMcpStatus | null>(null);
  const setStatus = (s: KimiMcpStatus) => { setStatusRaw(s); onStatus?.(s); };
  const [busy, setBusy] = useState(false);
  const [mutations, setMutations] = useState(true);
  // eslint-disable-next-line react-hooks/exhaustive-deps -- one read per mount
  useEffect(() => { invoke<KimiMcpStatus>("kimi_mcp_status").then(setStatus).catch((e) => onReport(String(e))); }, []);
  const set = async (mode: KimiMcpMode) => {
    setBusy(true);
    try { setStatus(await invoke<KimiMcpStatus>("kimi_mcp_set", { mode })); }
    catch (e) { onReport(`kimi MCP: ${String(e)}`); }
    finally { setBusy(false); }
  };
  const on = status && (status.mode === "mutations" || status.mode === "read-only");
  return <section className="setting" data-focus={focusId} data-testid="kimi-mcp">
    <label>Worktrees tools</label>
    <div className="hint">Connect the Worktrees tools to kimi lanes, the same server Claude, Codex and pi use.</div>
    <div className="hint" data-testid="kimi-mcp-state">{status ? MCP_VERDICT[status.state] : "Checking…"}</div>
    {status && status.state !== "kimi-missing" && <>
      {!on && <label className="tier-toggle setting-check"><input type="checkbox" checked={mutations}
        onChange={(e) => setMutations(e.currentTarget.checked)} />Allow create and close tools</label>}
      <div className="ver-actions">
        {!on && <button className="ctrl sm" data-testid="kimi-mcp-install" disabled={busy || !status.worktrees_bin}
          onClick={() => set(mutations ? "mutations" : "read-only")}>{busy ? "Working…" : "Set up kimi"}</button>}
        {on && <button className="ctrl sm" disabled={busy} data-testid="kimi-mcp-toggle"
          onClick={() => set(status.mode === "mutations" ? "read-only" : "mutations")}>
          {status.mode === "mutations" ? "Make read-only" : "Allow create and close"}</button>}
        {on && <button className="ctrl sm danger" disabled={busy} data-testid="kimi-mcp-remove" onClick={() => set("off")}>Disconnect</button>}
        {offerPending && onSilenceOffer && <button className="mcp-dismiss" onClick={onSilenceOffer}>Stop suggesting this</button>}
      </div>
      <div className="hint">kimi has no setting Worktrees can add a server to without editing kimi's own files, so each kimi launch writes it into the place itself, as <code>{status.file}</code> — kept out of git, and never written over a <code>.kimi-code/mcp.json</code> the repo has. A running kimi picks a change up at its next launch, or <code>/new</code>.</div>
      <div className="hint">Saved as <code>kimi_mcp</code> in {status.config_path}. Also: <code>worktrees mcp --install --ai kimi [--read-only]</code>.</div>
    </>}
  </section>;
}

export default function KimiPanel({ settings, repo, onChange, onReport, onMcpStatus, mcpOfferPending, onSilenceMcpOffer }: {
  settings: Settings;
  repo: string | null;
  onChange: (patch: Partial<Settings>) => void;
  onReport: (msg: string) => void;
  onMcpStatus?: (s: KimiMcpStatus) => void;
  mcpOfferPending?: boolean;
  onSilenceMcpOffer?: () => void;
}) {
  const [status, setStatus] = useState<KimiStatus | null>(null);
  const [models, setModels] = useState<ModelOption[] | null>(null);
  const reload = useCallback(() => {
    invoke<KimiStatus>("kimi_status", { repo }).then(setStatus).catch((e) => onReport(`kimi: ${String(e)}`));
    setModels(null);
    invoke<ModelOption[]>("agent_models", { harness: "kimi" }).then(setModels).catch((e) => { setModels([]); onReport(`kimi models: ${String(e)}`); });
  }, [repo, onReport]);
  useEffect(() => { reload(); }, [reload]);

  const allow = async (root: string, on: boolean) => {
    try {
      await invoke("set_kimi_allowed", { repo: root, allow: on });
      reload();
    } catch (e) { onReport(`kimi allowance: ${String(e)}`); }
  };

  return <>
    {/* FIRST: the `kimi-mcp` offer's deep-link target; the sections below
        fill in asynchronously and would push it off screen. */}
    <KimiMcpSection data-focus="kimi-mcp" onReport={onReport} onStatus={onMcpStatus}
      offerPending={mcpOfferPending} onSilenceOffer={onSilenceMcpOffer} />

    <section className="setting" data-testid="kimi-install">
      <label>kimi</label>
      {!status ? <div className="hint">reading…</div> : status.kimi_bin ? <div className="ver-rows">
        <div className="ver-row">kimi <b>{status.kimi_version ?? "?"}</b> · <code>{status.kimi_bin}</code></div>
      </div> : <div className="hint">Kimi Code is not installed. Install it from kimi.com/code, then sign in with <code>kimi login</code>.</div>}
      <div className="hint">kimi takes no prompt when it starts, so a brief's opener is typed into kimi once its prompt is empty — never over a question or into text you are typing — and given up on after ten minutes. <code>worktrees doctor --kimi</code> says how the last one went.</div>
    </section>

    <section className="setting" data-testid="kimi-models">
      <label>Models</label>
      {models === null ? <div className="hint">reading kimi's models…</div>
        : models.length === 0 ? <div className="hint">kimi's config declares no models this build can read.</div>
        : <div className="ver-rows">{models.map((o) => (
          <div className="ver-row" key={modelArg(o)}>
            <code>{modelArg(o)}</code>{o.model.label && <> — {o.model.label}</>}{o.meta.context && <> · {o.meta.context}</>}
          </div>
        ))}</div>}
      <label className="sub">Default model</label>
      <ModelPicker harness="kimi" value={settings.default_models.kimi ?? ""} options={models} testid="kimi-default-model"
        onChange={(m) => onChange({ default_models: { ...settings.default_models, kimi: m || undefined } })} />
      <div className="hint">Preselected when you start kimi. Left on the CLI's default, kimi uses its own <code>default_model</code>. Only the model list is read from kimi's config, never the rest of it.</div>
    </section>

    <section className="setting" data-testid="kimi-trust">
      <label>Folder trust</label>
      <div className="hint">kimi asks to trust every folder it has not seen, and every place is a new folder. Answering "Trust and continue" lets the repo's <code>.mcp.json</code> start its servers inside kimi.</div>
      {status?.launch && <div className="hint" data-testid="kimi-trust-source"><b>This repo:</b> {kimiTrustText(status.launch)}</div>}
      <label className="sub">Allowed repos</label>
      {status?.allowed.length ? <div className="ver-rows">{status.allowed.map((r) => (
        <div className="ver-row" key={r}><code>{r}</code>
          <button className="ctrl sm" data-testid="kimi-revoke" onClick={() => allow(r, false)}>Revoke</button></div>
      ))}</div> : <div className="hint">None. kimi lanes in an allowed repo start trusted, with no prompt — code from that repo's MCP servers runs inside kimi.</div>}
      {status?.repo_root && !status.repo_allowed && (
        <div className="ver-actions">
          <button className="ctrl sm" data-testid="kimi-allow-repo" onClick={() => allow(status.repo_root!, true)}>
            Allow {status.repo_root.split("/").pop()}
          </button>
        </div>
      )}
      <div className="hint">Kept in ~/.config/worktrees/config.toml as [trust] kimi. It trusts each launch only and is never written into kimi's own trust records, so it does not reach a kimi you run by hand. Also: <code>worktrees trust kimi [--revoke]</code>.</div>
    </section>
  </>;
}
