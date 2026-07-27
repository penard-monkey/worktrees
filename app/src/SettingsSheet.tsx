import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { check as checkAppUpdate } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import type { Settings, ThemeId, ThemeSetting, UpdateInfo } from "./settings";
import { clampNav, clampRem, clampTerm, THEMES } from "./settings";

type CmdResult = { ok: boolean; code: number; output: string };

// Right-side slide-over. Presentational: App owns the Settings state and does the
// apply-live + persist + terminal-refit on each change. Esc / scrim closes.
// The Version section owns its own update-run state (log/progress) locally.
export function SettingsSheet({
  open,
  settings,
  onChange,
  onClose,
  update,
  cliStale,
  cliMissing,
  appStale,
  onCheckUpdate,
  onShowNotes,
  onReset,
}: {
  open: boolean;
  settings: Settings;
  onChange: (patch: Partial<Settings>) => void;
  onClose: () => void;
  update: UpdateInfo | null;
  cliStale: boolean;
  cliMissing: boolean;
  appStale: boolean;
  onCheckUpdate: () => Promise<void> | void;
  onShowNotes: () => void;
  onReset: () => void;
}) {
  const [updating, setUpdating] = useState(false);
  const [updateLog, setUpdateLog] = useState("");
  const [checkState, setCheckState] = useState<"" | "checking" | "done">("");
  const doCheck = async () => {
    setCheckState("checking");
    try { await onCheckUpdate(); } finally { setCheckState("done"); }
  };
  const doUpdate = async () => {
    if (!update?.latest || updating) return;
    setUpdating(true);
    setUpdateLog(`$ install.sh @ ${update.latest}\n`);
    try {
      const r = await invoke<CmdResult>("update_cli", { tag: update.latest });
      setUpdateLog((l) => l + r.output + (r.ok ? "\n✓ done" : `\n✗ failed (exit ${r.code})`));
    } catch (e) {
      setUpdateLog((l) => l + `\n✗ ${String(e)}`);
    } finally {
      // versions re-read BEFORE re-enabling the button — a stale-enabled button
      // in the re-check window would re-run the whole installer on a click
      await onCheckUpdate();
      setUpdating(false);
    }
  };
  const actionable = cliStale || cliMissing;

  // app self-update: signed bundle via tauri-plugin-updater (latest.json
  // endpoint on the release). Verify → download → swap → relaunch.
  const [appUpdating, setAppUpdating] = useState(false);
  const doAppUpdate = async () => {
    if (appUpdating) return;
    setAppUpdating(true);
    setUpdateLog("checking for a signed app update…\n");
    try {
      const up = await checkAppUpdate();
      if (!up) {
        setUpdateLog((l) => l + "no newer signed build published.");
        return;
      }
      setUpdateLog((l) => l + `downloading app ${up.version}…\n`);
      await up.downloadAndInstall();
      setUpdateLog((l) => l + "installed — relaunching…");
      await relaunch();
    } catch (e) {
      const m = `app update failed: ${String(e)}`;
      setUpdateLog((l) => l + `\n✗ ${m}`);
      invoke("log_event", { level: "error", msg: m }).catch(() => {});
    } finally {
      setAppUpdating(false);
    }
  };

  // logs (app.log — backend op results, frontend errors, panics)
  const [logPath, setLogPath] = useState("");
  const [logTail, setLogTail] = useState<string | null>(null);
  useEffect(() => {
    if (!open) return;
    invoke<{ dir: string; file: string }>("log_info").then((i) => setLogPath(i.file)).catch(() => {});
  }, [open]);
  // reveal (not open-path): opener:default only grants open-url +
  // reveal-item-in-dir — openPath was silently rejected by the capability
  // system. Reveal also highlights app.log in Finder. Failures are NEVER
  // swallowed: they land in the tail area AND the log itself.
  const openLogsDir = async () => {
    try {
      const i = await invoke<{ dir: string; file: string }>("log_info");
      await revealItemInDir(i.file);
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

  // Data section: settings file path (for reveal) + two-click "reset to defaults".
  const [settingsPath, setSettingsPath] = useState("");
  const [dataErr, setDataErr] = useState<string | null>(null);
  const [resetArmed, setResetArmed] = useState(false);
  useEffect(() => {
    if (!open) return;
    setResetArmed(false); // re-arm each time the sheet opens
    invoke<{ dir: string; file: string }>("settings_info").then((i) => setSettingsPath(i.file)).catch(() => {});
  }, [open]);
  // reveal (not open-path): opener:default grants reveal-item-in-dir only.
  // Failures are NEVER swallowed — they surface here AND land in app.log.
  const revealSettings = async () => {
    try {
      const i = await invoke<{ dir: string; file: string }>("settings_info");
      await revealItemInDir(i.file);
    } catch (e) {
      const m = `reveal settings file failed: ${String(e)}`;
      setDataErr(m);
      invoke("log_event", { level: "error", msg: m }).catch(() => {});
    }
  };
  const doReset = () => {
    if (!resetArmed) { setResetArmed(true); return; } // arm; second click confirms
    setResetArmed(false);
    onReset();
  };
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div className="scrim" onClick={onClose}>
      <aside className="settings-sheet" onClick={(e) => e.stopPropagation()}>
        <header className="settings-h">
          <b>Settings</b>
          <button className="icon-btn" title="close (Esc)" onClick={onClose}>✕</button>
        </header>

        <div className="settings-body">
          <section className="setting">
            <label>UI font size <span className="val">{settings.ui_rem}px</span></label>
            <input
              type="range" min={13} max={18} step={1} value={settings.ui_rem}
              onChange={(e) => onChange({ ui_rem: clampRem(+e.currentTarget.value) })}
            />
            <div className="preview">The quick brown fox jumps</div>
          </section>

          <section className="setting">
            <label>Terminal font</label>
            <input
              type="text" value={settings.term_family}
              onChange={(e) => onChange({ term_family: e.currentTarget.value })}
            />
            <label className="sub">Terminal size <span className="val">{settings.term_size}px</span></label>
            <input
              type="range" min={10} max={20} step={1} value={settings.term_size}
              onChange={(e) => onChange({ term_size: clampTerm(+e.currentTarget.value) })}
            />
          </section>

          <section className="setting">
            <label>Commands</label>
            <label className="sub">Editor command</label>
            <input
              type="text" value={settings.editor_cmd}
              onChange={(e) => onChange({ editor_cmd: e.currentTarget.value })}
            />
            <div className="hint">Used by right-click “Open in editor” and ⌘E (e.g. code, cursor, subl)</div>
            <label className="tier-toggle setting-check">
              <input
                type="checkbox"
                checked={settings.ai_auto_resume}
                onChange={(e) => onChange({ ai_auto_resume: e.currentTarget.checked })}
              />
              Resume Claude conversation on open
            </label>
            <div className="hint">Single-click Enter resumes the last conversation. Only applies when the AI command is claude.</div>
          </section>

          <section className="setting">
            <label>Theme</label>
            <select value={settings.theme} onChange={(e) => onChange({ theme: e.currentTarget.value as ThemeSetting })}>
              <option value="system">System (match macOS)</option>
              {THEMES.map((t) => (
                <option key={t.id} value={t.id}>
                  {t.label} ({t.appearance})
                </option>
              ))}
            </select>
            {settings.theme === "system" && (
              <>
                <label className="sub">Light ↔ dark pair</label>
                <div className="row2">
                  <select value={settings.theme_light} onChange={(e) => onChange({ theme_light: e.currentTarget.value as ThemeId })}>
                    {THEMES.filter((t) => t.appearance === "light").map((t) => (
                      <option key={t.id} value={t.id}>{t.label}</option>
                    ))}
                  </select>
                  <span className="times">↔</span>
                  <select value={settings.theme_dark} onChange={(e) => onChange({ theme_dark: e.currentTarget.value as ThemeId })}>
                    {THEMES.filter((t) => t.appearance === "dark").map((t) => (
                      <option key={t.id} value={t.id}>{t.label}</option>
                    ))}
                  </select>
                </div>
                <div className="hint">Follows macOS appearance: this light theme by day, this dark theme in dark mode.</div>
              </>
            )}
          </section>

          <section className="setting">
            <label>Density</label>
            <div className="seg">
              {(["comfortable", "compact"] as const).map((d) => (
                <button
                  key={d}
                  className={settings.density === d ? "on" : ""}
                  onClick={() => onChange({ density: d })}
                >
                  {d}
                </button>
              ))}
            </div>
          </section>

          <section className="setting">
            <label>Version{actionable ? <span className="upd-tag">{cliMissing ? "cli not installed" : "update available"}</span> : null}</label>
            <div className="ver-rows">
              <div className="ver-row">app <b>{update?.app_version ?? "…"}</b></div>
              <div className="ver-row">
                cli{" "}
                {update?.cli_version ? (
                  <><b>{update.cli_version}</b> <span className="ver-path" title={update.cli_path ?? ""}>{update.cli_path}</span></>
                ) : (
                  <i>not installed</i>
                )}
              </div>
              <div className="ver-row">latest {update?.latest ? <b>{update.latest}</b> : <i>unknown (offline?)</i>}</div>
            </div>
            <div className="ver-actions">
              <button className="ctrl sm" disabled={checkState === "checking"} onClick={doCheck}>
                {checkState === "checking" ? "Checking…" : "Check for updates"}
              </button>
              <button className="ctrl sm" onClick={onShowNotes}>Release notes</button>
              {actionable && update?.latest && (
                <button className="ctrl sm" disabled={updating || appUpdating} onClick={doUpdate}>
                  {updating ? "Updating…" : `${cliMissing ? "Install" : "Update"} CLI → ${update.latest}`}
                </button>
              )}
              {appStale && update?.latest && (
                <button className="ctrl sm" disabled={appUpdating || updating} onClick={doAppUpdate}>
                  {appUpdating ? "Updating app…" : `Update app → ${update.latest}`}
                </button>
              )}
            </div>
            {checkState === "done" && (
              <div className="hint">
                {update?.latest
                  ? actionable || appStale
                    ? `updates available (latest ${update.latest})`
                    : `✓ up to date (latest ${update.latest})`
                  : "✗ couldn't reach the release feed (offline?)"}
              </div>
            )}
            {updateLog && <pre className="update-log">{updateLog}</pre>}
          </section>

          <section className="setting">
            <label>Nav tiers</label>
            <div className="tier-toggles">
              {(["active", "idle", "dormant"] as const).map((t) => (
                <label key={t} className="tier-toggle">
                  <input
                    type="checkbox"
                    checked={!settings.hidden_tiers.includes(t)}
                    onChange={(e) => {
                      const show = e.currentTarget.checked;
                      onChange({
                        hidden_tiers: show
                          ? settings.hidden_tiers.filter((x) => x !== t)
                          : [...settings.hidden_tiers, t],
                      });
                    }}
                  />
                  {t}
                </label>
              ))}
            </div>
            <div className="hint">Pinned and (main) always show. Sort order lives in the nav header (⇅).</div>
          </section>

          <section className="setting">
            <label>Logs</label>
            <div className="ver-rows">
              <div className="ver-row"><span className="ver-path" title={logPath}>{logPath || "…"}</span></div>
            </div>
            <div className="ver-actions">
              <button className="ctrl sm" onClick={openLogsDir}>Open folder</button>
              <button className="ctrl sm" onClick={viewLogTail}>{logTail ? "Refresh tail" : "View tail"}</button>
            </div>
            {logTail && <pre className="update-log">{logTail}</pre>}
          </section>

          <section className="setting">
            <label>Nav width <span className="val">{settings.nav_width}px</span></label>
            <input
              type="range" min={220} max={460} step={10} value={settings.nav_width}
              onChange={(e) => onChange({ nav_width: clampNav(+e.currentTarget.value) })}
            />
          </section>

          <section className="setting">
            <label>Shortcuts</label>
            <div className="shortcuts">
              {[
                ["⌘B", "Toggle the nav"],
                ["⌘,", "Open Settings"],
                ["⌘1", "Home (briefing)"],
                ["⌘2", "Places"],
                ["⌘3", "Recent"],
                ["⌘4", "Attention"],
                ["⌘E", "Open selection in editor"],
                ["Esc", "Close sheets & menus"],
              ].map(([key, desc]) => (
                <div className="shortcut-row" key={key}>
                  <kbd>{key}</kbd>
                  <span>{desc}</span>
                </div>
              ))}
            </div>
          </section>

          <section className="setting">
            <label>Data</label>
            <div className="ver-rows">
              <div className="ver-row"><span className="ver-path" title={settingsPath}>{settingsPath || "…"}</span></div>
            </div>
            <div className="ver-actions">
              <button className="ctrl sm" onClick={revealSettings}>Reveal settings file</button>
              <button className={"ctrl sm danger" + (resetArmed ? " armed" : "")} onClick={doReset}>
                {resetArmed ? "Confirm reset?" : "Reset to defaults"}
              </button>
            </div>
            {resetArmed && <div className="hint">Restores every setting to its default (theme, fonts, nav, sort, tiers).</div>}
            {dataErr && <pre className="update-log">{dataErr}</pre>}
          </section>
        </div>
      </aside>
    </div>
  );
}
