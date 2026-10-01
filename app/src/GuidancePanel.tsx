import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/** `guidance::Status` (core), as `agent_guidance_status` returns it. */
export type GuidanceDelivery =
  | { state: "on"; flags: string[] }
  | { state: "off" }
  | { state: "skipped"; reason: string };
export type GuidanceHarness = { id: string; label: string; installed: boolean } & GuidanceDelivery;
export type GuidanceStatus = {
  /** Bumped when what agents are told changes enough to ask again (the offer's fingerprint). */
  version: number;
  settings: { enabled: boolean; guard: boolean };
  settings_path: string;
  dir: string | null;
  error: string | null;
  harnesses: GuidanceHarness[];
  skill: string;
  rules: string;
};

function deliveryLine(h: GuidanceHarness): string {
  if (!h.installed) return "not installed";
  switch (h.state) {
    case "on":
      return h.id === "claude" ? "gets the worktrees plugin (skill)" : h.id === "pi" ? "gets the skill and the rule" : "gets the rule";
    case "off":
      return "off";
    case "skipped":
      return `not given it: ${h.reason}`;
  }
}

/** Settings → Agent guidance (agent-guidance proposal §4.5, decisions Q2/Q8).
 *  Machine-level: nothing here is about the project in focus. */
export function GuidanceSection({ status, onStatus, onReport, offerPending = false, onSilenceOffer, "data-focus": focusId }: {
  /** App's startup probe; null = not read yet or failed. */
  status: GuidanceStatus | null;
  /** Every status read or written here goes back to App, so the offer follows it. */
  onStatus: (s: GuidanceStatus) => void;
  onReport: (text: string) => void;
  offerPending?: boolean;
  onSilenceOffer?: () => void;
  "data-focus"?: string;
}) {
  const [busy, setBusy] = useState(false);
  // Re-read on open: the Codex half depends on Codex's own config, which can
  // change between app start and now.
  useEffect(() => {
    invoke<GuidanceStatus>("agent_guidance_status").then(onStatus).catch((e) => onReport(`agent_guidance_status: ${String(e)}`));
  }, []);
  const set = async (patch: Partial<GuidanceStatus["settings"]>) => {
    if (!status) return;
    setBusy(true);
    try {
      const next = { ...status.settings, ...patch };
      onStatus(await invoke<GuidanceStatus>("set_agent_guidance", { enabled: next.enabled, guard: next.guard }));
    } catch (e) { onReport(`set_agent_guidance: ${String(e)}`); }
    finally { setBusy(false); }
  };
  return <section className="setting" data-focus={focusId} data-testid="agent-guidance">
    <label>Agent guidance</label>
    <div className="hint">In a worktrees-managed repo, agents launched from Worktrees are told to do their branch work in a place, and get a <code>worktrees</code> skill for the rest: handing work to another agent, messaging between places, finishing and releasing. The text ships with Worktrees; nothing is written to any agent's own settings.</div>
    {!status ? <div className="hint">Checking…</div> : <>
      <label className="tier-toggle setting-check"><input type="checkbox" checked={status.settings.enabled} disabled={busy}
        onChange={(e) => set({ enabled: e.currentTarget.checked })} />Give agents the guidance at launch</label>
      <label className="tier-toggle setting-check"><input type="checkbox" checked={status.settings.guard} disabled={busy || !status.settings.enabled}
        onChange={(e) => set({ guard: e.currentTarget.checked })} />Stop Claude from <code>git worktree add</code> and new branches in (main)</label>
      <div className="hint">The guard refuses those two commands and tells Claude to use a place. An agent moving its own place to another branch is never refused. Off by default.</div>
      <div className="guidance-rows">
        {status.harnesses.map((h) => (
          <div className="hint" key={h.id} data-harness={h.id}><b>{h.label}:</b> {status.settings.enabled ? deliveryLine(h) : "off"}</div>
        ))}
      </div>
      {status.error && <div className="hint">The guidance files could not be written: {status.error}</div>}
      <div className="hint">Applies to agents launched from now on; a session already running keeps what it started with. Every connected session also reads the rule through the Worktrees MCP server.</div>
      <details className="guidance-text">
        <summary>What agents are told</summary>
        <div className="hint">The rule (pi and Codex get it as a prompt; every session sees it through the MCP server):</div>
        <pre className="update-log">{status.rules}</pre>
        <div className="hint">The skill (Claude and pi):</div>
        <pre className="update-log">{status.skill}</pre>
      </details>
      <div className="hint">Settings file: {status.settings_path}</div>
      {offerPending && onSilenceOffer && <div className="ver-actions"><button className="mcp-dismiss" onClick={onSilenceOffer}>Stop suggesting this</button></div>}
    </>}
  </section>;
}
