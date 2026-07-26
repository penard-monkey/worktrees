import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import type { Settings, UpdateInfo } from "./settings";
import { clampNav, clampRem, clampTerm } from "./settings";

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
}) {
  const [updating, setUpdating] = useState(false);
  const [updateLog, setUpdateLog] = useState("");
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
            <label>Editor command</label>
            <input
              type="text" value={settings.editor_cmd}
              onChange={(e) => onChange({ editor_cmd: e.currentTarget.value })}
            />
            <div className="hint">Used by right-click “Open in editor” (e.g. code, cursor, subl)</div>
          </section>

          <section className="setting">
            <label>Theme</label>
            <select value={settings.theme} onChange={(e) => onChange({ theme: e.currentTarget.value as "dark" })}>
              <option value="dark">Tokyo Night (dark)</option>
            </select>
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
            {appStale && update?.latest && (
              <div className="hint">app {update.app_version} · latest {update.latest} — rebuild/download to update the app itself</div>
            )}
            <div className="ver-actions">
              <button className="ctrl sm" onClick={onCheckUpdate}>Check for updates</button>
              {actionable && update?.latest && (
                <button className="ctrl sm" disabled={updating} onClick={doUpdate}>
                  {updating ? "Updating…" : `${cliMissing ? "Install" : "Update"} CLI → ${update.latest}`}
                </button>
              )}
            </div>
            {updateLog && <pre className="update-log">{updateLog}</pre>}
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
            <label>Window default</label>
            <div className="row2">
              <input
                type="number" min={900} value={settings.window_w}
                onChange={(e) => onChange({ window_w: +e.currentTarget.value })}
              />
              <span className="times">×</span>
              <input
                type="number" min={560} value={settings.window_h}
                onChange={(e) => onChange({ window_h: +e.currentTarget.value })}
              />
            </div>
            <label className="sub">Nav width <span className="val">{settings.nav_width}px</span></label>
            <input
              type="range" min={220} max={460} step={10} value={settings.nav_width}
              onChange={(e) => onChange({ nav_width: clampNav(+e.currentTarget.value) })}
            />
          </section>
        </div>
      </aside>
    </div>
  );
}
