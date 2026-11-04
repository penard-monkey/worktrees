// Settings → Layout (settings-reorg §3.2): the window's shape — the sidebar,
// which edge each half owns, widths, the dock's lower slot, where the usage
// meter sits, and the nav's tiers, guides and afterglow.
import { useEffect, useRef, useState } from "react";
import { LAYOUT_KEYS } from "../arrange";
import { clampNav, DEFAULTS, LAYOUT_TILES, railsMerged, railsWidth, type LayoutTile, type Settings } from "../settings";
import { clampSteps, doneBounds, DONE_FIRST_SECS, DONE_HORIZONS, DONE_STEPS_MAX, DONE_STEPS_MIN, fmtSecs, snapHorizon } from "../afterglow";
import { Page, Sec, useSheet } from "./kit";

/** What the tile captions call each region — the names the rest of the UI
 *  uses, not the code's (`nav` is "Places" everywhere a person reads it). */
const REGION_LABEL: Record<LayoutTile["order"][number], string> = { nav: "Places", agent: "Agent", reference: "Reference" };

/** A radio group with a roving tabindex: Tab lands on the current tile,
 *  arrows move AND select (the WAI-ARIA radio pattern), so the keyboard
 *  reaches every layout without a separate commit key. */
function LayoutTiles({ places, dock, onPick }: {
  places: Settings["places_side"]; dock: Settings["dock_side"];
  onPick: (places: Settings["places_side"], dock: Settings["dock_side"]) => void;
}) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const cur = LAYOUT_TILES.findIndex((t) => t.places === places && t.dock === dock);
  const move = (i: number) => {
    const n = (i + LAYOUT_TILES.length) % LAYOUT_TILES.length;
    onPick(LAYOUT_TILES[n].places, LAYOUT_TILES[n].dock);
    refs.current[n]?.focus();
  };
  return (
    <div className="layout-tiles" role="radiogroup" aria-labelledby="layout-label">
      {LAYOUT_TILES.map((t, i) => {
        const on = i === cur;
        const merged = railsMerged({ places_side: t.places, dock_side: t.dock });
        return (
          <button
            key={t.name}
            ref={(el) => { refs.current[i] = el; }}
            className={"layout-tile" + (on ? " on" : "")}
            role="radio"
            aria-checked={on}
            tabIndex={on || (cur < 0 && i === 0) ? 0 : -1}
            data-testid={`layout-${t.places}-${t.dock}`}
            onClick={() => onPick(t.places, t.dock)}
            onKeyDown={(e) => {
              if (e.key === "ArrowRight" || e.key === "ArrowDown") { e.preventDefault(); move(i + 1); }
              else if (e.key === "ArrowLeft" || e.key === "ArrowUp") { e.preventDefault(); move(i - 1); }
            }}
          >
            {/* The schematic: rails as slivers (one when merged), then the
                regions in window order. Decorative — the caption says it. */}
            <span className="layout-sch" aria-hidden="true">
              {(t.places === "left" || !merged) && <i className="sch-rail" />}
              {t.order.map((r) => <i key={r} className={"sch-" + r} />)}
              {(t.places === "right" || !merged) && <i className="sch-rail" />}
            </span>
            <span className="layout-name">{t.name}</span>
            <span className="layout-order">{t.order.map((r) => REGION_LABEL[r]).join(" · ")}</span>
          </button>
        );
      })}
    </div>
  );
}

export function LayoutPage() {
  const { settings, onChange, onResetLayout, open } = useSheet();
  // Reset layout's way back, for as long as the sheet stays open.
  const [resetUndo, setResetUndo] = useState<(() => void) | null>(null);
  useEffect(() => { if (!open) setResetUndo(null); }, [open]);
  return <Page page="layout">
    {/* One control for what used to be two booleans and a rail button. The
        pair it writes is `nav_pinned` + `nav_hover_reveal`, and only the middle
        state uses both — "Hidden" is unpinned with the pointer trigger off,
        which is why it still opens on ⌘B. */}
    <Sec id="sidebar">
      <section className="setting">
        <label>Sidebar</label>
        <div className="seg seg-plain">
          {([
            ["pinned", "Pinned", { nav_pinned: true }],
            ["auto", "Auto-hide", { nav_pinned: false, nav_hover_reveal: true }],
            ["hidden", "Hidden", { nav_pinned: false, nav_hover_reveal: false }],
          ] as const).map(([mode, label, patch]) => {
            const cur = settings.nav_pinned ? "pinned" : settings.nav_hover_reveal ? "auto" : "hidden";
            return (
              <button key={mode} className={cur === mode ? "on" : ""} onClick={() => onChange(patch)}>
                {label}
              </button>
            );
          })}
        </div>
        <div className="hint">
          ⌘B and the Places icon switch between pinned and hidden. With auto-hide, the sidebar
          slides over the terminal when the pointer reaches the rail or on ⌘B, and closes on Esc
          or when you open a place.
        </div>
      </section>
    </Sec>

    {/* Both halves' sides as ONE control: four tiles that show the result,
        rather than two segmented controls that make you compose it in your
        head. See `places_side` / `dock_side` in settings.ts. */}
    <Sec id="layout">
      <section className="setting">
        <label id="layout-label">Layout</label>
        <LayoutTiles places={settings.places_side} dock={settings.dock_side}
          onPick={(places, dock) => onChange({ places_side: places, dock_side: dock })} />
        <div className="hint">
          Applies to all places. Places and the reference panels (Files, Terminal, Docs, Plan) each
          take an edge; on the same edge their rails become one. Widths, ⌘B and ⌘J are unchanged.
          Tab still walks the panels in their classic order. You can also drag a section by the
          grip in its header, or click the grip for a menu.
        </div>
        <div style={{ display: "flex", gap: "var(--s2)", marginTop: "var(--s2)" }}>
          <button className="ctrl sm" data-testid="layout-reset"
            disabled={LAYOUT_KEYS.every((k) => settings[k] === DEFAULTS[k])}
            onClick={() => { const u = onResetLayout(); setResetUndo(() => u); }}>
            Reset layout
          </button>
          {resetUndo && (
            <button className="ctrl sm" data-testid="layout-reset-undo"
              onClick={() => { resetUndo(); setResetUndo(null); }}>
              Undo
            </button>
          )}
        </div>
      </section>
    </Sec>

    <Sec id="nav-width">
      <section className="setting">
        <label>Nav width <span className="val">{settings.nav_width}px</span></label>
        <input
          type="range" min={220} max={460} step={10} value={settings.nav_width} aria-label="Nav width"
          onChange={(e) => onChange({ nav_width: clampNav(+e.currentTarget.value, 0, undefined, railsWidth(settings)) })}
        />
      </section>
    </Sec>

    <Sec id="nav-tiers">
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
        <div className="hint">Pinned and (main) always show. Sort order lives in the nav header.</div>
      </section>
    </Sec>

    {/* The reference column's lower slot. Terminal only — see `dock_lower`
        in settings.ts for why the other sections are not offered here yet. */}
    <Sec id="dock-lower">
      <section className="setting">
        <label>Terminal placement</label>
        <div className="seg seg-plain">
          {([
            [null, "In the panel's tabs"],
            ["terminal", "Below the reference panel"],
          ] as const).map(([lower, label]) => (
            <button
              key={label}
              data-testid={`dock-lower-${lower ?? "tab"}`}
              className={settings.dock_lower === lower ? "on" : ""}
              onClick={() => onChange({ dock_lower: lower })}
            >
              {label}
            </button>
          ))}
        </div>
        <div className="hint">
          Below the panel, your shells stay on screen under Files, Docs or Plan — the same shells
          as the Terminal tab. Drag the line between them to resize; ⌘J hides both.
        </div>
      </section>
    </Sec>

    {/* The placement IS layout; whether each provider is polled at all is the
        Agents page's (Plan usage), beside the agent it reads. */}
    <Sec id="usage-meter">
      <section className="setting">
        <label>Usage meter</label>
        <div className="seg seg-plain">
          {([
            ["strip", "Strip"],
            ["footer", "Footer"],
            ["rail", "Rail"],
            ["off", "Off"],
          ] as const).map(([v, label]) => (
            <button key={v} className={settings.usage_place === v ? "on" : ""} onClick={() => onChange({ usage_place: v })}>
              {label}
            </button>
          ))}
        </div>
        <div className="hint">
          Where account plan limits live. Strip is a row across the bottom of the window, so it
          stays put through ⌘B and is the only one that also shows on Home. Footer puts them
          under the terminal, on a place only. Rail is a tile beside the ＋ and ⚙ icons. Hover
          any of them for the full windows and how long until each resets. Off stops the checks.
        </div>
      </section>
    </Sec>

    <Sec id="tree-guides">
      <section className="setting">
        <label>Tree guide lines</label>
        <label className="tier-toggle setting-check">
          <input type="checkbox" checked={settings.nav_guides}
            onChange={(e) => onChange({ nav_guides: e.currentTarget.checked })} />
          Draw the vertical guides
        </label>
        <div className="hint">The 1px rails that connect a project to its places. Off leaves the indentation alone.</div>
      </section>
    </Sec>

    {/* The purple dot's decay. Horizon walks DONE_HORIZONS by index for the
        same reason Overall size walks ZOOM_STEPS: a linear range over 1h..7d
        spends most of its travel on moves nobody can see. The hint prints the
        boundaries from `doneBounds` itself rather than a hand-written example,
        so it cannot go stale when either knob or the geometry changes. */}
    <Sec id="afterglow">
      <section className="setting">
        <label>Afterglow</label>
        <label className="sub">Fade out over <span className="val">{fmtSecs(snapHorizon(settings.done_horizon_secs))}</span></label>
        <input
          type="range" min={0} max={DONE_HORIZONS.length - 1} step={1} aria-label="Fade out over"
          value={Math.max(0, DONE_HORIZONS.indexOf(snapHorizon(settings.done_horizon_secs) as (typeof DONE_HORIZONS)[number]))}
          onChange={(e) => onChange({ done_horizon_secs: DONE_HORIZONS[+e.currentTarget.value] })}
        />
        <label className="sub">Steps <span className="val">{clampSteps(settings.done_steps)}</span></label>
        <input
          type="range" min={DONE_STEPS_MIN} max={DONE_STEPS_MAX} step={1} value={clampSteps(settings.done_steps)} aria-label="Steps"
          onChange={(e) => onChange({ done_steps: clampSteps(+e.currentTarget.value) })}
        />
        <div className="hint">
          The purple dot marks a place where Claude finished, fading in steps:{" "}
          {doneBounds(settings.done_horizon_secs, settings.done_steps).map(fmtSecs).join(" · ")}.
          The first step is always {fmtSecs(DONE_FIRST_SECS)} and is the one that lights the project folder.
          A finish you have not looked at yet holds the first step, however old it is, until you select the place.
        </div>
      </section>
    </Sec>
  </Page>;
}
