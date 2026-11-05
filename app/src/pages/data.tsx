// Settings → Data & privacy (settings-reorg §3.2): feedback, the logs, where
// this window's settings live, Reset with a scope (§6 item 7), and — collapsed,
// Expert — the usage record that used to be its own category.
import { useEffect, useState } from "react";
import { FeedbackSection } from "../FeedbackSection";
import { invoke, windowOnHost } from "../ipc";
import { copyToClipboard } from "../clipboard";
import { hostOnly, revealOnHost } from "../onhost";
import { clampZoom, DEFAULTS } from "../settings";
import { useArmed } from "../useArmed";
import { MODE_LABEL, RESET_SCOPES, resetPatch, type ResetScope } from "../settingsMap";
import { UsagePanel } from "./usage";
import { Page, Sec, useSheet } from "./kit";

function Logs() {
  const { open, settings } = useSheet();
  // logs (serve.log — backend op results, frontend errors, panics)
  const [logPath, setLogPath] = useState("");
  const [logTail, setLogTail] = useState<string | null>(null);
  const [diagCopied, setDiagCopied] = useState(false);
  useEffect(() => {
    if (!open) return;
    invoke<{ dir: string; file: string }>("log_info").then((i) => setLogPath(i.file)).catch(() => {});
  }, [open]);
  // reveal (not open-path): opener:default only grants open-url +
  // reveal-item-in-dir — openPath was silently rejected by the capability
  // system. Reveal also highlights the log in Finder. Failures are NEVER
  // swallowed: they land in the tail area AND the log itself.
  const openLogsDir = async () => {
    try {
      const i = await invoke<{ dir: string; file: string }>("log_info");
      await revealOnHost(i.file);
    } catch (e) {
      const m = `open logs folder failed: ${String(e)}`;
      setLogTail(m);
      invoke("log_event", { level: "error", msg: m }).catch(() => {});
    }
  };
  const viewLogTail = async () => {
    try {
      setLogTail((await invoke<string>("log_tail", { lines: 200 })) || "(empty)");
    } catch (e) {
      setLogTail(String(e));
    }
  };
  // Copy diagnostics: the backend assembles the plaintext block (app/cli/PATH/
  // git/tmux/core-config/log-tail); the frontend-known UI bits go after it.
  const copyDiagnostics = async () => {
    try {
      // `gh`: the Pull requests switch reaches the backend, so "off stops
      // every gh call" holds for this button too (`gh auth status` is a
      // network check, ~0.5s).
      const back = await invoke<string>("diagnostics", { gh: settings.pull_requests });
      const ui =
        `\nUI settings\n-----------\n` +
        `theme   : ${settings.theme}\n` +
        `density : ${settings.density}\n` +
        `ui_rem  : ${settings.ui_rem}px\n` +
        `app_zoom: ${Math.round(clampZoom(settings.app_zoom) * 100)}%\n` +
        `term    : ${settings.term_size}px\n` +
        `mode    : ${settings.settings_mode}\n`;
      await copyToClipboard(back + ui);
      setDiagCopied(true);
      setTimeout(() => setDiagCopied(false), 2000);
    } catch (e) {
      const m = `copy diagnostics failed: ${String(e)}`;
      setLogTail(m);
      invoke("log_event", { level: "error", msg: m }).catch(() => {});
    }
  };
  return (
    <section className="setting">
      <label>Logs</label>
      <div className="ver-rows">
        <div className="ver-row"><span className="ver-path" title={logPath}>{logPath || "…"}</span></div>
      </div>
      <div className="ver-actions">
        <button className="ctrl sm" onClick={openLogsDir} {...hostOnly("Reveal in Finder")}>Open folder</button>
        <button className="ctrl sm" onClick={viewLogTail}>{logTail ? "Refresh tail" : "View tail"}</button>
        <button className="ctrl sm" onClick={copyDiagnostics}>{diagCopied ? "Copied ✓" : "Copy diagnostics"}</button>
      </div>
      {logTail && <pre className="update-log log-tail">{logTail}</pre>}
    </section>
  );
}

/** Where THIS window's settings are written. On a window driving a paired
 *  host that is this machine's `remote-ui-state.json`, not the host's file —
 *  `settings_info` would answer with the host's path (§2.4). */
function SettingsFile() {
  const { open } = useSheet();
  const [path, setPath] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const onHost = windowOnHost();
  useEffect(() => {
    if (!open || !onHost) return;
    invoke<{ dir: string; file: string }>("settings_info").then((i) => setPath(i.file)).catch(() => {});
  }, [open, onHost]);
  // reveal (not open-path): opener:default grants reveal-item-in-dir only.
  // Failures are NEVER swallowed — they surface here AND land in the log.
  const reveal = async () => {
    try {
      const i = await invoke<{ dir: string; file: string }>("settings_info");
      await revealOnHost(i.file);
    } catch (e) {
      const m = `reveal settings file failed: ${String(e)}`;
      setErr(m);
      invoke("log_event", { level: "error", msg: m }).catch(() => {});
    }
  };
  return (
    <section className="setting">
      <label>Settings file</label>
      <div className="ver-rows">
        <div className="ver-row" data-testid="settings-file-path">
          {onHost
            ? <span className="ver-path" title={path}>{path || "…"}</span>
            : <span className="ver-path">remote-ui-state.json, in this app's settings folder on this computer</span>}
        </div>
      </div>
      <div className="ver-actions">
        <button className="ctrl sm" onClick={reveal} {...hostOnly("Reveal in Finder")}>Reveal settings file</button>
      </div>
      {!onHost && <div className="hint">This window drives another computer, and keeps its own settings here — one file for every host it drives.</div>}
      {err && <pre className="update-log">{err}</pre>}
    </section>
  );
}

/** Reset, with a scope (§6 item 7): "reset to defaults" used to wipe forty
 *  places' memory and every silenced suggestion to reset a theme (§2.6). The
 *  scope says what goes; the default is the narrowest that matches the old
 *  hint's promise. Two clicks, and the arm expires. */
function Reset() {
  const { onReset, mode, vis } = useSheet();
  const [scope, setScope] = useState<ResetScope>("look");
  const arm = useArmed<"reset">();
  const scopes = RESET_SCOPES.filter((s) => s.id !== "above" || mode !== "expert");
  const cur = RESET_SCOPES.find((s) => s.id === scope)!;
  const go = () => arm.confirm("reset", () => onReset(resetPatch(scope, { mode, installed: vis.installed, defaults: DEFAULTS }), scope));
  return (
    <section className="setting">
      <label>Reset</label>
      <div className="reset-scopes" role="radiogroup" aria-label="What to reset" data-testid="reset-scopes">
        {scopes.map((s) => (
          <label key={s.id} className="tier-toggle setting-check">
            <input type="radio" name="reset-scope" checked={scope === s.id} data-scope={s.id}
              onChange={() => { setScope(s.id); arm.disarm(); }} />
            {s.id === "above" ? `Settings above ${MODE_LABEL[mode]}` : s.label}
          </label>
        ))}
      </div>
      <div className="hint" data-testid="reset-hint">{cur.hint}</div>
      <div className="ver-actions">
        <button className={"ctrl sm danger" + (arm.armed ? " armed" : "")} data-testid="reset-go" onClick={go}>
          {arm.armed ? "Confirm reset?" : `Reset ${scope === "everything" ? "everything" : cur.label.toLowerCase()}`}
        </button>
      </div>
      <div className="hint">This window's settings only. What this computer keeps for every window — agent policy, terminal history, profiles — is never reset from here.</div>
    </section>
  );
}

export function DataPage() {
  return <Page page="data">
    <Sec id="feedback"><FeedbackSection /></Sec>
    <Sec id="logs"><Logs /></Sec>
    <Sec id="settings-file"><SettingsFile /></Sec>
    <Sec id="reset"><Reset /></Sec>
    <Sec id="usage">
      {/* Collapsed: a record, not a setting (§2.7), and the panel reads only
          once it is opened. */}
      <UsageRecord />
    </Sec>
  </Page>;
}

function UsageRecord() {
  // Open when a link named it (the old Usage category, `LEGACY_CATS.usage`).
  const { focus } = useSheet();
  const [open, setOpen] = useState(focus === "usage");
  return (
    <details className="usage-record" open={open} onToggle={(e) => setOpen(e.currentTarget.open)}>
      <summary>Usage record</summary>
      {open && <UsagePanel />}
    </details>
  );
}
