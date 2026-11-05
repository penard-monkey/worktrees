// Settings → Appearance (settings-reorg §3.2): how big, which theme, how
// dense. Nothing here polls — the usage meter's placement went to Layout and
// its per-provider switches to Agents.
import { clampRem, clampZoom, THEMES, ZOOM_STEPS, type ThemeId, type ThemeSetting } from "../settings";
import { Page, Sec, useSheet } from "./kit";

export function AppearancePage() {
  const { settings, onChange } = useSheet();
  return <Page page="appearance">
    {/* Overall size first: it is the knob that moves EVERYTHING, and the two
        below it are then a preference within it. The slider walks ZOOM_STEPS
        by index rather than taking a percentage directly, so it can only ever
        land on a legal step (browser zoom is multiplicative — a linear range
        gives useless 1% moves at 3×). */}
    <Sec id="zoom">
      <section className="setting">
        <label>Overall size <span className="val">{Math.round(clampZoom(settings.app_zoom) * 100)}%</span></label>
        <input
          type="range" min={0} max={ZOOM_STEPS.length - 1} step={1} aria-label="Overall size"
          value={Math.max(0, ZOOM_STEPS.indexOf(clampZoom(settings.app_zoom) as (typeof ZOOM_STEPS)[number]))}
          onChange={(e) => onChange({ app_zoom: ZOOM_STEPS[+e.currentTarget.value] })}
        />
        <div className="hint">⌘+ / ⌘− / ⌘0 anywhere in the app. Scales the whole window — chrome, icons, files and the terminal, which re-fits its tmux pane to the new size. (⌘⌥+ / ⌘⌥− is the markdown reader's own size.)</div>
      </section>
    </Sec>

    <Sec id="ui-font">
      <section className="setting">
        <label>UI font size <span className="val">{settings.ui_rem}px</span></label>
        <input
          type="range" min={13} max={22} step={1} value={settings.ui_rem} aria-label="UI font size"
          onChange={(e) => onChange({ ui_rem: clampRem(+e.currentTarget.value) })}
        />
        <div className="preview">The quick brown fox jumps</div>
        <div className="hint">The chrome's base size, before Overall size multiplies it. Does not touch the terminal — that has its own size under Terminal.</div>
      </section>
    </Sec>

    <Sec id="theme">
      <section className="setting">
        <label>Theme</label>
        <select value={settings.theme} aria-label="Theme" onChange={(e) => onChange({ theme: e.currentTarget.value as ThemeSetting })}>
          <option value="system">System (match macOS)</option>
          {THEMES.map((t) => (
            <option key={t.id} value={t.id}>
              {t.label} ({t.appearance})
            </option>
          ))}
        </select>
      </section>
    </Sec>

    <Sec id="theme-pair">
      <section className="setting">
        <label>Light ↔ dark pair</label>
        <div className="row2">
          <select value={settings.theme_light} aria-label="Light theme" onChange={(e) => onChange({ theme_light: e.currentTarget.value as ThemeId })}>
            {THEMES.filter((t) => t.appearance === "light").map((t) => (
              <option key={t.id} value={t.id}>{t.label}</option>
            ))}
          </select>
          <span className="times">↔</span>
          <select value={settings.theme_dark} aria-label="Dark theme" onChange={(e) => onChange({ theme_dark: e.currentTarget.value as ThemeId })}>
            {THEMES.filter((t) => t.appearance === "dark").map((t) => (
              <option key={t.id} value={t.id}>{t.label}</option>
            ))}
          </select>
        </div>
        <div className="hint">With the theme on System: this light theme by day, this dark theme in dark mode.</div>
      </section>
    </Sec>

    <Sec id="density">
      <section className="setting">
        <label>Density</label>
        <div className="seg">
          {(["comfortable", "compact"] as const).map((d) => (
            <button key={d} className={settings.density === d ? "on" : ""} onClick={() => onChange({ density: d })}>
              {d}
            </button>
          ))}
        </div>
      </section>
    </Sec>
  </Page>;
}
