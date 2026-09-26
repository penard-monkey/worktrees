import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";

const INSTALL_URL = "https://developers.openai.com/codex/cli/";

export type CodexMcpStatus = {
  state: "installed" | "read-only" | "stale" | "foreign" | "absent" | "cli-missing";
  codex_bin: string | null;
  worktrees_bin: string | null;
  entry: { command: string; mutations: boolean } | null;
  command: string | null;
  config_path: string;
};
type Outcome = { ok: boolean; output: string; status: CodexMcpStatus };

export function CodexMcpSection({ onReport, onStatus, offerPending = false, onSilenceOffer }: {
  onReport: (text: string) => void;
  /** Every status this panel reads or is handed back — App keeps the
   *  `codex-mcp` offer in step with it, so installing here retires the offer. */
  onStatus?: (s: CodexMcpStatus) => void;
  /** Is `codex-mcp` still an open suggestion? Like the Claude panel, this is
   *  the on-demand way to end it (the band appears once per version). */
  offerPending?: boolean;
  onSilenceOffer?: () => void;
}) {
  const [status, setStatusRaw] = useState<CodexMcpStatus | null>(null);
  const setStatus = (s: CodexMcpStatus) => { setStatusRaw(s); onStatus?.(s); };
  const [busy, setBusy] = useState(false);
  const [mutations, setMutations] = useState(true);
  const [output, setOutput] = useState("");
  useEffect(() => { invoke<CodexMcpStatus>("codex_mcp_status").then(setStatus).catch((e) => onReport(String(e))); }, []);
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
  return <><section className="setting" data-focus="codex-mcp">
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
    {status && !status.codex_bin && <div className="hint">Worktrees could not find the Codex CLI. Install it or add it to your PATH. Run <code>curl -fsSL https://chatgpt.com/codex/install.sh | sh</code>, then <code>codex login</code> to sign in with ChatGPT.</div>}
    {status && !status.worktrees_bin && <div className="hint">Worktrees CLI was not found. Install it under Updates, then reopen Settings.</div>}
    {status && status.state !== "foreign" && <>
      <label className="tier-toggle setting-check"><input type="checkbox" checked={mutations}
        onChange={(e) => setMutations(e.currentTarget.checked)} />Allow create and close tools</label>
      <div className="ver-actions">
        {!status.codex_bin ? <button className="ctrl sm" onClick={() => openUrl(INSTALL_URL).catch((e) => onReport(String(e)))}>Install Codex CLI</button>
          : <button className="ctrl sm" disabled={busy || !status.worktrees_bin}
            onClick={() => act(false)}>{busy ? "Working…" : status.entry ? "Repair or update" : "Set up Codex"}</button>}
        {status.entry && <button className="ctrl sm danger" disabled={busy} onClick={() => act(true)}>Remove from Codex</button>}
        {offerPending && onSilenceOffer && <button className="mcp-dismiss" onClick={onSilenceOffer}>Stop suggesting this</button>}
      </div>
      {status.command && <div className="hint"><code>{status.command}</code></div>}
    </>}
    {status && <div className="hint">Codex configuration: {status.config_path}</div>}
    {output && <pre className="update-log">{output}</pre>}
  </section><CodexMcpMigration onReport={onReport} codexAvailable={!!status?.codex_bin} /></>;
}

export type MigrationRow = {
  name: string;
  transport: string;
  status: "copy" | "copy_needs_login" | "copy_literal_env" | "exists" | "differs" | "unsupported";
  reason: string;
};
export type MigrationOutcome = { name: string; ok: boolean; output: string; needs_login: boolean };
const migrationLabels: Record<MigrationRow["status"], string> = {
  copy_literal_env: "Literal env values",
  copy: "Ready to copy", copy_needs_login: "Sign-in needed", exists: "Already in Codex",
  differs: "Differs", unsupported: "Can't copy",
};

function CodexMcpMigration({ onReport, codexAvailable }: { onReport: (text: string) => void; codexAvailable: boolean }) {
  const [rows, setRows] = useState<MigrationRow[] | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [outcomes, setOutcomes] = useState<MigrationOutcome[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const refresh = async () => {
    setError("");
    try {
      const plan = await invoke<MigrationRow[]>("codex_mcp_migration_plan");
      setRows(plan);
      // A refreshed literal-value warning requires an explicit selection again.
      setSelected((current) => current.filter((name) => plan.some((r) => r.name === name && (r.status === "copy" || r.status === "copy_needs_login"))));
    } catch (e) { setError(String(e)); onReport(String(e)); }
  };
  useEffect(() => { void refresh(); }, []);
  const apply = async () => {
    if (busy || !selected.length) return;
    setBusy(true);
    setError("");
    try {
      const results = await invoke<MigrationOutcome[]>("codex_mcp_migrate", { names: selected });
      setOutcomes(results);
      if (results.some((r) => !r.ok)) onReport("Some servers were not copied. See the per-server results.");
      await refresh();
    } catch (e) { setError(String(e)); onReport(String(e)); }
    finally { setBusy(false); }
  };
  return <section className="setting mcp-migration" data-focus="mcp-migration">
    <label>Copy servers from Claude</label>
    <div className="hint">Choose servers from Claude's user configuration to add to Codex. Existing names are skipped. Worktrees' own server is managed above. Worktrees serializes its own migrations, but cannot guard against another tool writing Codex configuration at the same moment.</div>
    {error && <div className="hint" role="alert">{error}</div>}
    {!rows && !error && <div className="hint">Checking Claude servers…</div>}
    {rows?.length === 0 && <div className="hint">No Claude user-scope servers to copy.</div>}
    {rows?.map((row) => {
      const copyable = row.status === "copy" || row.status === "copy_needs_login" || row.status === "copy_literal_env";
      const result = outcomes.find((o) => o.name === row.name);
      return <div className="mcp-migration-row" key={row.name}>
        <div className="mcp-migration-heading">
          {copyable ? <label className="tier-toggle setting-check">
            <input type="checkbox" aria-label={`Copy ${row.name}`} checked={selected.includes(row.name)} disabled={busy || !codexAvailable}
              onChange={(e) => { const checked = e.currentTarget.checked; setSelected((current) => checked ? [...current, row.name] : current.filter((n) => n !== row.name)); }} />
            <span className="mcp-migration-name">{row.name}</span>
          </label> : <span className="mcp-migration-name">{row.name}</span>}
          <span className={`mcp-migration-chip ${row.status}`}><i aria-hidden="true" />{migrationLabels[row.status]}</span>
        </div>
        <div className="hint">{row.transport} · {row.reason}</div>
        {result && <div className="hint" role="status">{result.ok ? "Copied" : "Not completed"}: {result.output}</div>}
        {(row.status === "copy_needs_login" || result?.needs_login) && <div className="hint">Sign in from a terminal: <code>codex mcp login {row.name}</code></div>}
      </div>;
    })}
    <div className="ver-actions">
      <button className="ctrl sm" disabled={busy || !codexAvailable || !selected.length} onClick={apply}>{busy ? "Copying…" : `Apply${selected.length ? ` (${selected.length})` : ""}`}</button>
      <button className="ctrl sm" disabled={busy} onClick={refresh}>Refresh servers</button>
    </div>
    {!codexAvailable && <div className="hint">Install the Codex CLI to copy servers.</div>}
  </section>;
}
