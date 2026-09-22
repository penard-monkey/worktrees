import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type Status = {
  state: "installed" | "read-only" | "stale" | "foreign" | "absent" | "cli-missing";
  codex_bin: string | null;
  worktrees_bin: string | null;
  entry: { command: string; mutations: boolean } | null;
  command: string | null;
  config_path: string;
};
type Outcome = { ok: boolean; output: string; status: Status };

export function CodexMcpSection({ onReport }: { onReport: (text: string) => void }) {
  const [status, setStatus] = useState<Status | null>(null);
  const [busy, setBusy] = useState(false);
  const [mutations, setMutations] = useState(true);
  const [output, setOutput] = useState("");
  useEffect(() => { invoke<Status>("codex_mcp_status").then(setStatus).catch((e) => onReport(String(e))); }, []);
  const act = async (remove: boolean) => {
    setBusy(true);
    try {
      const result = await invoke<Outcome>(remove ? "codex_mcp_uninstall" : "codex_mcp_install", remove ? {} : { mutations });
      setStatus(result.status);
      setOutput(result.output);
      if (!result.ok) onReport(result.output || "Codex MCP setup did not complete.");
    } catch (e) { onReport(String(e)); }
    finally { setBusy(false); }
  };
  return <section className="setting">
    <label>Codex MCP server</label>
    <div className="hint">Worktrees uses Codex's ChatGPT account sign-in. Run <code>codex login</code> in a terminal to complete the browser flow, then <code>codex login status</code> to check it. Worktrees does not ask for an API key.</div>
    <div className="hint">Connect Worktrees tools to Codex. Claude's MCP setup is separate.</div>
    <div className="hint">{status ? {
      installed: "Connected — Codex can create, inspect, and close worktrees.",
      "read-only": "Connected with read-only tools.",
      stale: `Configured, but ${status.entry?.command ?? "the server"} is unavailable.`,
      foreign: "Another server uses the worktrees name. Worktrees will leave it untouched.",
      absent: "Not connected to Codex.",
      "cli-missing": "Install the Worktrees CLI first.",
    }[status.state] : "Checking…"}</div>
    {status && status.state !== "foreign" && <>
      <label className="tier-toggle setting-check"><input type="checkbox" checked={mutations}
        onChange={(e) => setMutations(e.currentTarget.checked)} />Allow create and close tools</label>
      <div className="ver-actions">
        <button className="ctrl sm" disabled={busy || !status.codex_bin || !status.worktrees_bin}
          onClick={() => act(false)}>{busy ? "Working…" : status.entry ? "Repair or update" : "Set up Codex"}</button>
        {status.entry && <button className="ctrl sm danger" disabled={busy} onClick={() => act(true)}>Remove from Codex</button>}
      </div>
      {status.command && <div className="hint"><code>{status.command}</code></div>}
    </>}
    {status && <div className="hint">Codex configuration: {status.config_path}</div>}
    {output && <pre className="update-log">{output}</pre>}
  </section>;
}
