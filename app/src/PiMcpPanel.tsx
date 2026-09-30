// Settings → pi → Worktrees tools (pi-harness §4.3): the CodexMcpSection shape.
//
// Status is READ from pi's own mcp.json (`pimcp::status`) — never `pi mcp list`,
// which starts every server the user has. Every write is `pi mcp add/remove`,
// run by the backend; this app never edits pi's file.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export type PiMcpStatus = {
  state: "installed" | "read-only" | "disabled" | "stale" | "foreign" | "unreadable" | "absent" | "cli-missing" | "pi-missing";
  pi_bin: string | null;
  worktrees_bin: string | null;
  entry: { command: string; mutations: boolean; exposure: string | null; enabled: boolean } | null;
  command: string | null;
  config_path: string;
};
type Outcome = { ok: boolean; output: string; status: PiMcpStatus };

const VERDICT: Record<PiMcpStatus["state"], string> = {
  installed: "Connected — pi lanes can report, wait for and message other places, and create or close worktrees.",
  "read-only": "Connected with read-only tools.",
  disabled: "In pi's config but disabled (from pi's /mcp). Repair turns it back on.",
  stale: "Configured, but the worktrees binary it names is gone.",
  foreign: "Another server uses the worktrees name in pi's config. Worktrees leaves it untouched.",
  unreadable: "pi's mcp.json is not valid JSON — pi cannot read it either. Fix it by hand, then come back.",
  absent: "Not connected to pi.",
  "cli-missing": "Install the Worktrees CLI first (Settings → Updates).",
  "pi-missing": "pi is not installed.",
};

export function PiMcpSection({ onReport, onStatus, offerPending = false, onSilenceOffer, "data-focus": focusId }: {
  onReport: (text: string) => void;
  "data-focus"?: string;
  /** Every status read or handed back — App keeps the `pi-mcp` offer in step. */
  onStatus?: (s: PiMcpStatus) => void;
  offerPending?: boolean;
  onSilenceOffer?: () => void;
}) {
  const [status, setStatusRaw] = useState<PiMcpStatus | null>(null);
  const setStatus = (s: PiMcpStatus) => { setStatusRaw(s); onStatus?.(s); };
  const [busy, setBusy] = useState(false);
  const [mutations, setMutations] = useState(true);
  const [output, setOutput] = useState("");
  useEffect(() => { invoke<PiMcpStatus>("pi_mcp_status").then(setStatus).catch((e) => onReport(String(e))); }, []);
  const act = async (remove: boolean) => {
    setBusy(true);
    try {
      const result = await invoke<Outcome>(remove ? "pi_mcp_uninstall" : "pi_mcp_install", remove ? {} : { mutations });
      setStatus(result.status);
      setOutput(result.output);
      if (!result.ok) onReport(result.output || "pi MCP setup did not complete.");
    } catch (e) { onReport(String(e)); }
    finally { setBusy(false); }
  };
  const canAct = status && !["foreign", "unreadable", "pi-missing"].includes(status.state);
  const exposure = status?.entry ? status.entry.exposure ?? "codemode (pi's default)" : null;
  return <section className="setting" data-focus={focusId} data-testid="pi-mcp">
    <label>Worktrees tools</label>
    <div className="hint">Connect the Worktrees tools to pi, the same server Claude and Codex use. Claude's and Codex's setup is separate.</div>
    <div className="hint" data-testid="pi-mcp-state">{status ? VERDICT[status.state] : "Checking…"}</div>
    {exposure && <div className="hint">Exposure: <code>{exposure}</code>{exposure !== "direct" && " — worktrees installs it as direct, so a model calls the tools like its own."}</div>}
    {canAct && <>
      <label className="tier-toggle setting-check"><input type="checkbox" checked={mutations}
        onChange={(e) => setMutations(e.currentTarget.checked)} />Allow create and close tools</label>
      <div className="ver-actions">
        <button className="ctrl sm" data-testid="pi-mcp-install" disabled={busy || !status.worktrees_bin}
          onClick={() => act(false)}>{busy ? "Working…" : status.entry ? "Repair or update" : "Set up pi"}</button>
        {status.entry && <button className="ctrl sm danger" disabled={busy} onClick={() => act(true)}>Remove from pi</button>}
        {offerPending && onSilenceOffer && <button className="mcp-dismiss" onClick={onSilenceOffer}>Stop suggesting this</button>}
      </div>
      {status.entry && <div className="hint">Repair replaces the whole entry, including any per-tool exposure or timeout you added to it by hand.</div>}
      {status.command && <div className="hint"><code>{status.command}</code></div>}
    </>}
    {status && <div className="hint">pi configuration: {status.config_path}. Running pi sessions pick a change up on <code>/reload</code> or their next launch.</div>}
    {output && <pre className="update-log">{output}</pre>}
  </section>;
}
