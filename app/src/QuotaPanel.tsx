import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/** `quota_settings` (lib.rs): the user's `[quota]` in config.toml, which the
 *  launch gate (`worktrees_core::quota`) reads every time an agent starts. */
export type QuotaSettings = {
  gate: boolean;
  /** `null` = the provider's own grade, which is the shipped behaviour. */
  weekly_warn_pct: number | null;
  /** What a hand edit got wrong; the gate ignored it and said so here. */
  problems: string[];
  config_path: string;
};

/** The thresholds offered. Must stay inside `quota::PCT_MIN..=PCT_MAX` —
 *  `app/scripts/quota-check.mjs` checks this list against quota.rs. A value
 *  set by hand that is not listed is still shown (see `options`). */
export const QUOTA_PCTS = [50, 60, 70, 75, 80, 85, 90, 95, 100];

/** Settings → Behavior → Plan limits. The refusal message names this path, so
 *  renaming the section means changing `quota::SETTINGS_PATH` too (quota-check.mjs). */
export function QuotaSection({ onReport, "data-focus": focusId }: {
  onReport: (text: string) => void;
  "data-focus"?: string;
}) {
  const [s, setS] = useState<QuotaSettings | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    invoke<QuotaSettings>("quota_settings").then(setS).catch((e) => onReport(`quota_settings: ${String(e)}`));
  }, []);
  const save = async (gate: boolean, pct: number | null) => {
    setBusy(true);
    try { setS(await invoke<QuotaSettings>("set_quota_settings", { gate, weeklyWarnPct: pct })); }
    catch (e) { onReport(`set_quota_settings: ${String(e)}`); }
    finally { setBusy(false); }
  };
  const pct = s?.weekly_warn_pct ?? null;
  const options = pct !== null && !QUOTA_PCTS.includes(pct) ? [...QUOTA_PCTS, pct].sort((a, b) => a - b) : QUOTA_PCTS;
  return <section className="setting" data-focus={focusId} data-testid="quota-settings">
    <label>Plan limits</label>
    {!s ? <div className="hint">Checking…</div> : <>
      <label className="tier-toggle setting-check">
        <input type="checkbox" data-testid="quota-gate" checked={s.gate} disabled={busy}
          onChange={(e) => save(e.currentTarget.checked, pct)} />
        Don't start an agent on a nearly spent plan
      </label>
      <div className="hint">
        Before starting Claude or Codex, worktrees reads the plan's usage and refuses the launch when a
        window is nearly spent. The worktree and its brief are still created, and Launch anyway overrides it once.
      </div>
      <label className="sub">Weekly window: refuse at</label>
      <select data-testid="quota-pct" value={pct === null ? "" : String(pct)} disabled={busy || !s.gate}
        onChange={(e) => save(s.gate, e.currentTarget.value === "" ? null : +e.currentTarget.value)}>
        <option value="">The provider's warning (default)</option>
        {options.map((n) => <option key={n} value={String(n)}>{n}% used</option>)}
      </select>
      <div className="hint">
        Applies to weekly windows. The 5-hour window always uses the provider's own warning; turn the check
        off to launch past it. The usage meter's colours always show the provider's reading.
      </div>
      {/* Hue in the glyph only, words in --txt-dim: `.hint`'s --txt-mute is
          2.2:1 in tokyo-day, too faint for the one line saying an edit did nothing. */}
      {s.problems.length > 0 && <div className="quota-problems" data-testid="quota-problems">
        {s.problems.map((p) => <div key={p}><span className="quota-problem-mark" aria-hidden>⚠</span> {p}</div>)}
      </div>}
      <div className="hint">Takes effect at the next launch — agents already running are not affected.</div>
      <div className="hint">Settings file: {s.config_path} (<code>[quota]</code>)</div>
    </>}
  </section>;
}
