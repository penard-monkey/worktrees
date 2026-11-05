import { useEffect, useState } from "react";
import { invoke } from "./ipc";
import { HARNESS_LABEL, isHarness } from "./harness";

/** `limits_settings` (worktrees-api ops): the user's `[limits]` in
 *  config.toml, which the launch guard (`worktrees_core::guard`) reads every
 *  time an agent starts. */
export type LimitsSettings = {
  /** What the file sets; null / absent = the default. */
  explicit: {
    max_live_agents: number | null;
    max_live: Record<string, number>;
    min_free_memory_pct: number | null;
    max_swap_used_pct: number | null;
  };
  /** What the guard applies now. null = no cap. */
  max_live_agents: number | null;
  max_live: Record<string, number>;
  min_free_memory_pct: number;
  max_swap_used_pct: number;
  default_cap: number | null;
  ram_gib: number | null;
  default_min_free_pct: number;
  /** Agents alive on this machine now, by provider (counts only). */
  live: Record<string, number>;
  providers: string[];
  max_agents: number;
  problems: string[];
  config_path: string;
};

/** The memory floors offered; 0 is off. A value set by hand is shown too. */
const FREE_PCTS = [0, 10, 15, 20, 25, 30, 40, 50];
const SWAP_PCTS = [0, 50, 60, 70, 80, 90, 100];

const providerName = (p: string) => (isHarness(p) ? HARNESS_LABEL[p] : p);

/** A count with − / + and a way back to the default. Module scope (a
 *  component defined inside another remounts every render). */
function CapLine({ label, value, explicit, max, live, fallback, disabled, testid, onSet }: {
  label: string;
  /** The cap applied now; null = none. */
  value: number | null;
  /** The file sets it. */
  explicit: boolean;
  max: number;
  live: number;
  /** What Reset leaves, in words. */
  fallback: string;
  disabled: boolean;
  testid: string;
  onSet: (n: number | null) => void;
}) {
  const shown = value ?? 0;
  return <div className="slot-row" data-testid={`limit-${testid}`}>
    <div className="slot-main">
      <span className="slot-name">{label}</span>
      <span className="slot-meta">
        {live > 0 && <span className="slot-dot" aria-hidden />}
        <span>{value === null ? "no cap" : `at most ${value}`}{explicit ? "" : ` (${fallback})`}</span>
        {live > 0 && <span> · {live} live now</span>}
      </span>
    </div>
    <div className="slot-step">
      <button type="button" data-testid={`limit-dec-${testid}`} aria-label={`One fewer for ${label}`}
        disabled={disabled || shown <= 1} onClick={() => onSet(shown - 1)}>−</button>
      <span className="slot-cap" data-testid={`limit-cap-${testid}`}>{value === null ? "—" : value}</span>
      <button type="button" data-testid={`limit-inc-${testid}`} aria-label={`One more for ${label}`}
        disabled={disabled || shown >= max} onClick={() => onSet(shown + 1)}>+</button>
    </div>
    {explicit && <button type="button" className="slot-reset" data-testid={`limit-reset-${testid}`} disabled={disabled}
      onClick={() => onSet(null)}>Back to {fallback}</button>}
  </div>;
}

/** Settings → Agent policy → Limits (settings-reorg §6 item 1). The guard
 *  refuses a launch past these and says so; Launch anyway overrides it once.
 *  The panel only ever writes a value the guard reads (`set_limits` refuses
 *  the rest), and shows what the file got wrong. */
export function LimitsSection({ onReport, onProblems }: {
  onReport: (text: string) => void;
  /** Rule 3: the page draws this section, whatever the mode, while the file
   *  holds something the guard ignores. */
  onProblems?: (on: boolean) => void;
}) {
  const [s, setS] = useState<LimitsSettings | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    invoke<LimitsSettings>("limits_settings").then(setS).catch((e) => onReport(`limits_settings: ${String(e)}`));
    // eslint-disable-next-line react-hooks/exhaustive-deps -- one read per mount
  }, []);
  useEffect(() => { if (s) onProblems?.(s.problems.length > 0); }, [s, onProblems]);
  const set = async (key: string, value: number | null) => {
    setBusy(true);
    try { setS(await invoke<LimitsSettings>("set_limits", { key, value })); }
    catch (e) { onReport(`set_limits: ${String(e)}`); }
    finally { setBusy(false); }
  };
  const total = s ? Object.values(s.live).reduce((a, b) => a + b, 0) : 0;
  const machine = s?.default_cap != null
    ? `machine default ${s.default_cap}, from ${s.ram_gib} GiB`
    : "no cap: this machine's memory could not be read";
  const pctOptions = (list: number[], cur: number) => (list.includes(cur) ? list : [...list, cur].sort((a, b) => a - b));
  return <section className="setting" data-testid="limits-settings">
    <label>Limits</label>
    {!s ? <div className="hint">Checking…</div> : <>
      <div className="hint">
        How many agents may be live on this machine at once, across every project, and how much memory must be
        free before another starts. Worktrees refuses a launch past them and says which; the place and its brief
        are still created, and Launch anyway overrides it once.
      </div>
      <div className="slot-rows">
        <CapLine label="Every agent" value={s.max_live_agents} explicit={s.explicit.max_live_agents !== null}
          max={s.max_agents} live={total} fallback={machine} disabled={busy} testid="all"
          onSet={(n) => void set("max_live_agents", n)} />
        {s.providers.map((p) => (
          <CapLine key={p} label={providerName(p)} value={s.max_live[p] ?? null} explicit={p in s.explicit.max_live}
            max={s.max_agents} live={s.live[p] ?? 0} fallback="no cap of its own" disabled={busy} testid={p}
            onSet={(n) => void set(`max_live.${p}`, n)} />
        ))}
      </div>
      <label className="sub">Free memory needed to start one</label>
      <select data-testid="limit-free" value={s.min_free_memory_pct} disabled={busy}
        onChange={(e) => void set("min_free_memory_pct", +e.currentTarget.value)}>
        {pctOptions(FREE_PCTS, s.min_free_memory_pct).map((n) => (
          <option key={n} value={n}>{n === 0 ? "Off" : `${n}% free`}{n === s.default_min_free_pct ? " (default)" : ""}</option>
        ))}
      </select>
      <label className="sub">Swap in use that stops one</label>
      <select data-testid="limit-swap" value={s.max_swap_used_pct} disabled={busy}
        onChange={(e) => void set("max_swap_used_pct", +e.currentTarget.value)}>
        {pctOptions(SWAP_PCTS, s.max_swap_used_pct).map((n) => (
          <option key={n} value={n}>{n === 0 ? "Off (default)" : `${n}% used`}</option>
        ))}
      </select>
      <div className="hint">macOS grows swap as it needs it, so a swap limit is noisy there; the free-memory floor is the steadier signal.</div>
      {(s.explicit.min_free_memory_pct !== null || s.explicit.max_swap_used_pct !== null) && <div className="ver-actions">
        <button type="button" className="slot-reset" data-testid="limit-reset-memory" disabled={busy} onClick={async () => {
          if (s.explicit.min_free_memory_pct !== null) await set("min_free_memory_pct", null);
          if (s.explicit.max_swap_used_pct !== null) await set("max_swap_used_pct", null);
        }}>Back to the memory defaults</button>
      </div>}
      {s.problems.length > 0 && <div className="quota-problems" data-testid="limits-problems">
        {s.problems.map((p) => <div key={p}><span className="quota-problem-mark" aria-hidden>⚠</span> {p}</div>)}
      </div>}
      <div className="hint">Takes effect at the next launch — agents already running are not stopped.</div>
      <div className="hint">Settings file: {s.config_path} (<code>[limits]</code>)</div>
    </>}
  </section>;
}
