// Settings → Agents (settings-reorg §3.2): the overview the pi and
// cross-project proposals both named "Settings → Agents" and never got —
// which agent and model a new place starts on, how it opens, whether each one
// has the Worktrees tools, and which plans are polled.
//
// This window's (§9 Q7, the plan's default): a paired device keeps its own
// default agent and models, as every blob key is per device today.
import { useEffect, useState } from "react";
import { invoke, notGranted } from "../ipc";
import { hostOnly, revealOnHost } from "../onhost";
import { HARNESSES, HARNESS_LABEL, type Harness } from "../harness";
import { ModelPicker, useAgentModels } from "../ModelPicker";
import type { CodexMcpStatus } from "../CodexMcpPanel";
import type { PiMcpStatus } from "../PiMcpPanel";
import type { KimiMcpStatus } from "../KimiPanel";
import { askNotifications, notificationsAllowed } from "../notifyhost";
import { DONE_AFTER_HIDDEN_MS } from "../notify";
import { Page, Sec, useSheet } from "./kit";

type AiConfig = { ai_cmd: string; ai_resume_arg: string; path: string; exists: boolean };

/** What `*_mcp_status` says, in the table's words. */
export function agentSetupLabel(state: string | null): string {
  if (!state) return "Checking…";
  return ({
    installed: "Connected",
    "read-only": "Connected (read only)",
    elsewhere: "Connected in this project",
    absent: "Not connected",
    stale: "Needs repair",
    foreign: "Name in use",
    "cli-missing": "Worktrees CLI missing",
    disabled: "Disabled in pi",
    unreadable: "Config unreadable",
    "pi-missing": "pi not installed",
    "not-applicable": "Unavailable",
    // kimi's states are its `kimi_mcp` setting, prefixed so they cannot
    // collide with another harness's words.
    "kimi-mutations": "Connected",
    "kimi-read-only": "Connected (read only)",
    "kimi-off": "Not connected",
    "kimi-cli-missing": "Worktrees CLI missing",
    "kimi-kimi-missing": "kimi not installed",
  } as Record<string, string>)[state] ?? state;
}

/** What the CLI does for a model instead of this window's choice: two stores
 *  for one fact (§2.4), so the row says which one a terminal launch reads.
 *  Claude and Codex read no `[model]`: the CLI passes no model unless given
 *  `--model` (core harness.rs reads `config::default_model` for pi and kimi
 *  only). */
function cliLine(h: Harness): string {
  if (h === "pi" || h === "kimi") return `From a terminal, worktrees reads [model] ${h} in config.toml instead.`;
  return `From a terminal, worktrees passes no model unless you give --model, so ${HARNESS_LABEL[h]} uses its own default.`;
}

/** Module scope: one picker row, fed by its own catalog read. */
function ModelRow({ harness }: { harness: Harness }) {
  const { settings, onChange, onReport } = useSheet();
  const opts = useAgentModels(harness, (e) => onReport(`${harness} models: ${String(e)}`));
  return (
    <div className="model-row" data-testid={`default-model-${harness}`}>
      <span className="model-row-name">{HARNESS_LABEL[harness]}</span>
      <ModelPicker harness={harness} value={settings.default_models[harness] ?? ""} options={opts} testid={`default-model-pick-${harness}`}
        onChange={(m) => onChange({ default_models: { ...settings.default_models, [harness]: m || undefined } })} />
      <span className="hint model-row-cli">{cliLine(harness)}</span>
    </div>
  );
}

/** Settings → Agents → Notifications (settings-reorg §6 item 9). The system
 *  is asked for permission only from here, by the button — never on load. */
function NotificationsSection() {
  const { settings, onChange, host } = useSheet();
  const [allowed, setAllowed] = useState<boolean | null | undefined>(undefined);
  useEffect(() => { void notificationsAllowed().then(setAllowed); }, []);
  const mode = settings.notify_hidden;
  return (
    <section className="setting">
      <label>Notifications</label>
      <div className="seg seg-plain" data-testid="notify-mode">
        {([["off", "Off"], ["waiting", "When an agent waits"], ["both", "When one waits or finishes"]] as const).map(([v, label]) => (
          <button key={v} className={mode === v ? "on" : ""} data-mode={v} onClick={() => onChange({ notify_hidden: v })}>{label}</button>
        ))}
      </div>
      <div className="hint">
        Only while this window is hidden — minimised, on another Space, or fully covered: an agent stopped on a question, and — with the
        second — one that finishes after you have been away {Math.round(DONE_AFTER_HIDDEN_MS / 60_000)} minutes.
        {host ? ` This window shows ${host}'s agents, on this computer.` : ""}
      </div>
      {mode !== "off" && allowed === false && (
        <div className="ver-actions">
          <span className="hint">This computer has not allowed the app's notifications yet.</span>
          <button type="button" className="ctrl sm" data-testid="notify-allow" onClick={() => void askNotifications().then(setAllowed)}>Allow notifications</button>
        </div>
      )}
      {mode !== "off" && allowed === null && <div className="hint">Notifications are not available in this window.</div>}
    </section>
  );
}

export function AgentsPage() {
  const api = useSheet();
  const { settings, onChange, onReport, mcpStatus, goto, vis, mode } = api;
  const [codexMcp, setCodexMcp] = useState<CodexMcpStatus | null>(null);
  const [piMcp, setPiMcp] = useState<PiMcpStatus | null>(null);
  const [kimiMcp, setKimiMcp] = useState<KimiMcpStatus | null>(null);
  const [aiConfig, setAiConfig] = useState<AiConfig | null>(null);
  useEffect(() => {
    let alive = true;
    // A remote window that may not ask says so in the row, not as an error
    // notice on every visit.
    const read = <T,>(cmd: string, set: (v: T) => void) => notGranted(cmd) ? Promise.resolve()
      : invoke<T>(cmd).then((v) => { if (alive) set(v); }).catch((e) => { if (alive) onReport(String(e)); });
    void read<CodexMcpStatus>("codex_mcp_status", setCodexMcp);
    void read<PiMcpStatus>("pi_mcp_status", setPiMcp);
    void read<KimiMcpStatus>("kimi_mcp_status", setKimiMcp);
    invoke<AiConfig>("get_ai_config").then((c) => { if (alive) setAiConfig(c); }).catch(() => {});
    return () => { alive = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- probe on entry, not every App render
  }, []);
  // Reveal the config file — but revealItemInDir REJECTS a path that does not
  // exist (the plugin canonicalizes), so a missing file reveals its folder.
  const revealConfig = async () => {
    if (!aiConfig) return;
    try {
      await revealOnHost(aiConfig.exists ? aiConfig.path : aiConfig.path.replace(/\/[^/]*$/, "") || "/");
    } catch (e) {
      const m = `reveal AI config failed: ${String(e)}`;
      onReport(m);
      invoke("log_event", { level: "error", msg: m }).catch(() => {});
    }
  };
  // Beginner lists the harnesses the host has (rule 4's reading of
  // "installed"); Advanced and up list all four.
  const shown = HARNESSES.filter((h) => mode !== "beginner" || vis.installed[h] !== false);
  const statusCmd: Record<Harness, string> = { claude: "mcp_status", codex: "codex_mcp_status", pi: "pi_mcp_status", kimi: "kimi_mcp_status" };
  const states: Record<Harness, string | null> = {
    claude: mcpStatus?.state ?? null,
    codex: codexMcp?.state ?? null,
    pi: piMcp?.state ?? null,
    kimi: kimiMcp ? `kimi-${kimiMcp.state}` : null,
  };
  const stateWords = (h: Harness) => notGranted(statusCmd[h]) ? `Set up at ${api.host ?? "the host"}` : agentSetupLabel(states[h]);
  const focusOf: Record<Harness, string> = { claude: "mcp-server", codex: "codex-mcp", pi: "pi-mcp", kimi: "kimi-mcp" };
  return <Page page="agents">
    <Sec id="default-agent">
      <section className="setting">
        <label>Default agent</label>
        <div className="seg">
          {HARNESSES.map((provider) => (
            <button key={provider} className={settings.default_provider === provider ? "on" : ""}
              onClick={() => onChange({ default_provider: provider })}>
              {HARNESS_LABEL[provider]}
            </button>
          ))}
        </div>
        <div className="hint">Preselected for new worktrees when more than one agent is installed. Entering a place keeps its running agent; this default applies when none is running. Switching agents closes the current session first.</div>
      </section>
    </Sec>

    <Sec id="default-models">
      <section className="setting">
        <label>Default model</label>
        <div className="model-rows">{shown.map((h) => <ModelRow key={h} harness={h} />)}</div>
        <div className="hint">Preselected in New worktree and Switch agent. The CLI's default leaves the choice to the agent. pi always needs one, or it asks each time.</div>
      </section>
    </Sec>

    <Sec id="auto-resume">
      <section className="setting">
        <label>Resume on open</label>
        <label className="tier-toggle setting-check">
          <input type="checkbox" checked={settings.ai_auto_resume}
            onChange={(e) => onChange({ ai_auto_resume: e.currentTarget.checked })} />
          Resume agent conversation on open
        </label>
        <div className="hint">Single-click Enter resumes the selected agent's last conversation when available.</div>
      </section>
    </Sec>

    <Sec id="chat-view">
      <section className="setting">
        <label>Chat view</label>
        <label className="tier-toggle setting-check">
          <input type="checkbox" checked={settings.chat_view} data-testid="setting-chat-view"
            onChange={(e) => onChange({ chat_view: e.currentTarget.checked })} />
          Show a Chat | Terminal switch on each agent (preview)
        </label>
        <div className="hint">
          Chat is the default for Claude and Codex; pi and kimi open on Terminal. Each place
          remembers your choice. Chat shows recent turns, waiting prompts, and a message box.
          Terminal is one click away for tool output, diffs, and slash commands.
        </div>
      </section>
    </Sec>

    <Sec id="notifications">
      <NotificationsSection />
    </Sec>

    <Sec id="connections">
      <section className="setting">
        <label>Worktrees tools for agents</label>
        <div className="hint">Every agent can be connected at the same time. Their setup is independent of the default agent.</div>
        <div className="conn-rows" data-testid="agent-connections">
          {shown.map((h) => (
            <div className="conn-row" key={h} data-harness={h}>
              <span className="conn-name">{HARNESS_LABEL[h]}</span>
              <span className="conn-state">{stateWords(h)}</span>
              <button type="button" className="mcp-dismiss" data-testid={`configure-${h}`} onClick={() => goto(h, focusOf[h])}>Configure ›</button>
            </div>
          ))}
        </div>
      </section>
    </Sec>

    <Sec id="plan-usage">
      <section className="setting">
        <label>Plan usage</label>
        <label className="usage-setting-toggle"><input type="checkbox" checked={settings.usage_claude}
          onChange={(e) => onChange({ usage_claude: e.target.checked })} /> Claude plan usage</label>
        <label className="usage-setting-toggle"><input type="checkbox" checked={settings.usage_codex}
          onChange={(e) => onChange({ usage_codex: e.target.checked })} /> Codex plan usage</label>
        <label className="usage-setting-toggle"><input type="checkbox" checked={settings.usage_kimi}
          onChange={(e) => onChange({ usage_kimi: e.target.checked })} /> Kimi plan usage</label>
        <div className="hint">Account limits use your existing CLI sign-ins. Turning a provider off stops its usage checks. Kimi's are read by starting kimi's own local server (<code>kimi web</code>) for one request, at most every two minutes. Where the meter sits is under Layout.</div>
      </section>
    </Sec>

    <Sec id="ai-command">
      <section className="setting">
        <label>AI command (CLI)</label>
        <div className="ver-rows">
          <div className="ver-row">
            AI command: <b>{aiConfig?.ai_cmd ?? "…"}</b>
            {aiConfig && <> · resume: <b>{aiConfig.ai_resume_arg}</b></>}
          </div>
        </div>
        <div className="ver-actions">
          <button className="ctrl sm" onClick={revealConfig} disabled={!aiConfig} {...hostOnly("Reveal in Finder")}>Reveal config file</button>
        </div>
        <div className="hint">The CLI uses this command when you omit --ai, and <code>[model] pi</code> / <code>[model] kimi</code> in the same file for those two. The app uses Default agent and Default model above. Shell env vars (WORKTREES_AI_CMD) don't reach the GUI.</div>
      </section>
    </Sec>
  </Page>;
}
