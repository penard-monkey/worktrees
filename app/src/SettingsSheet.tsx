import { useEffect } from "react";
import type { Settings } from "./settings";
import { clampNav, clampRem, clampTerm } from "./settings";

// Right-side slide-over. Presentational: App owns the Settings state and does the
// apply-live + persist + terminal-refit on each change. Esc / scrim closes.
export function SettingsSheet({
  open,
  settings,
  onChange,
  onClose,
}: {
  open: boolean;
  settings: Settings;
  onChange: (patch: Partial<Settings>) => void;
  onClose: () => void;
}) {
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
