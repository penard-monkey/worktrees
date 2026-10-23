import { useEffect, useRef, useState } from "react";
import { invoke } from "./ipc";

/** One row of `slots_settings` (worktrees-api `ops::SlotRow`). */
export type SlotRow = {
  name: string;
  capacity: number;
  /** `config [slots] gate`, `machine (10 cores, 36 GiB)` or `built-in`. */
  source: string;
  /** `[slots] <name>` is set, so Reset has something to remove. */
  explicit: boolean;
  /** What Reset leaves, and where that comes from. */
  fallback: number;
  fallback_source: string;
  fallback_kind: "config" | "machine" | "built-in";
  busy: number;
  waiting: number;
};

/** `slots_settings`: the user's `[slots]` in config.toml, which every
 *  `worktrees run --slot <name>` reads when it starts (`worktrees_core::slots`). */
export type SlotsSettings = {
  /** `gate`, every name `[slots]` lists, then `default`. */
  slots: SlotRow[];
  /** `null` when the machine could not be read (the built-in default). */
  machine: { cores: number; ram_gib: number; capacity: number } | null;
  /** The highest capacity a slot takes (`slots::CAPACITY_MAX`), so this
   *  panel holds no copy of the rule. */
  max: number;
  problems: string[];
  config_path: string;
};

/** How often the busy counts refresh while the section is on screen. */
const POLL_MS = 5000;

/** What Reset puts the slot back on, in words. */
function resetLabel(r: SlotRow): string {
  if (r.fallback_kind === "machine") return `Reset to machine default (${r.fallback})`;
  if (r.fallback_kind === "config") return `Reset to default (${r.fallback})`;
  return `Reset to built-in (${r.fallback})`;
}

function busyText(r: SlotRow): string {
  const parts = [];
  if (r.busy > 0) parts.push(`${r.busy} running`);
  if (r.waiting > 0) parts.push(`${r.waiting} waiting`);
  return parts.join(", ");
}

/** Module scope (AGENTS.md: a component defined inside another remounts on
 *  every render and loses the button's focus mid-click). */
function SlotLine({ row, max, disabled, onSet }: {
  row: SlotRow;
  max: number;
  disabled: boolean;
  onSet: (n: number | null) => void;
}) {
  const busy = busyText(row);
  return <div className="slot-row" data-testid={`slot-${row.name}`}>
    <div className="slot-main">
      <span className="slot-name">{row.name === "default" ? "default (any other name)" : row.name}</span>
      <span className="slot-meta">
        {busy && <span className="slot-dot" aria-hidden />}
        <span data-testid={`slot-source-${row.name}`}>{row.source}</span>
        {busy && <span> · {busy}</span>}
      </span>
    </div>
    <div className="slot-step">
      <button type="button" data-track="slots-fewer" data-testid={`slot-dec-${row.name}`} aria-label={`One fewer ${row.name} slot`}
        disabled={disabled || row.capacity <= 1} onClick={() => onSet(row.capacity - 1)}>−</button>
      <span className="slot-cap" data-testid={`slot-cap-${row.name}`}>{row.capacity}</span>
      <button type="button" data-track="slots-more" data-testid={`slot-inc-${row.name}`} aria-label={`One more ${row.name} slot`}
        disabled={disabled || row.capacity >= max} onClick={() => onSet(row.capacity + 1)}>+</button>
    </div>
    {row.explicit && <button type="button" className="slot-reset" data-track="slots-reset" data-testid={`slot-reset-${row.name}`} disabled={disabled}
      title={`Remove [slots] ${row.name} from config.toml: ${row.fallback} (${row.fallback_source})`}
      onClick={() => onSet(null)}>{resetLabel(row)}</button>}
  </div>;
}

/** Settings → Behavior → Heavy jobs. */
export function SlotsSection({ onReport, "data-focus": focusId }: {
  onReport: (text: string) => void;
  "data-focus"?: string;
}) {
  const [s, setS] = useState<SlotsSettings | null>(null);
  const [saving, setSaving] = useState(false);
  // A poll answered after a save STARTED may have read the file before the
  // save wrote it, so it is dropped: `gen` moves on every save.
  const gen = useRef(0);
  const savingRef = useRef(false);
  useEffect(() => {
    let live = true;
    const pull = () => {
      if (document.visibilityState === "hidden" || savingRef.current) return;
      const at = gen.current;
      invoke<SlotsSettings>("slots_settings")
        .then((v) => { if (live && at === gen.current && !savingRef.current) setS(v); })
        .catch((e) => onReport(`slots_settings: ${String(e)}`));
    };
    pull();
    const t = setInterval(pull, POLL_MS);
    return () => { live = false; clearInterval(t); };
  }, []);
  const save = async (name: string, capacity: number | null) => {
    gen.current++;
    savingRef.current = true;
    setSaving(true);
    try { setS(await invoke<SlotsSettings>("set_slot_capacity", { name, capacity })); }
    catch (e) { onReport(`set_slot_capacity: ${String(e)}`); }
    finally { savingRef.current = false; setSaving(false); }
  };
  return <section className="setting" data-focus={focusId} data-testid="slots-settings">
    <label>Heavy jobs</label>
    {!s ? <div className="hint">Checking…</div> : <>
      <div className="hint">
        How many heavy jobs — gates, builds, full test runs — may run at once on this machine, across every
        project. A lane wraps one in <code>worktrees run --slot gate -- …</code>, and the next one waits for a free slot.
      </div>
      <div className="slot-rows">
        {s.slots.map((r) => <SlotLine key={r.name} row={r} max={s.max} disabled={saving}
          onSet={(n) => save(r.name, n)} />)}
      </div>
      {s.machine
        ? <div className="hint" data-testid="slots-machine">
            Sized from this machine when <code>[slots]</code> sets nothing: {s.machine.cores} cores and{" "}
            {s.machine.ram_gib} GiB give {s.machine.capacity}.
          </div>
        : <div className="hint" data-testid="slots-machine">This machine could not be read, so the built-in default applies.</div>}
      {s.problems.length > 0 && <div className="quota-problems" data-testid="slots-problems">
        {s.problems.map((p) => <div key={p}><span className="quota-problem-mark" aria-hidden>⚠</span> {p}</div>)}
      </div>}
      <div className="hint">Takes effect at the next <code>worktrees run</code> — a job already running or waiting keeps the number it started with.</div>
      <div className="hint">Settings file: {s.config_path} (<code>[slots]</code>)</div>
    </>}
  </section>;
}
