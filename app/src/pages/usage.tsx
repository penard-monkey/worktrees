// Settings → Data & privacy → Usage record (Expert, collapsed; it was the
// Usage category until the regroup, settings-reorg §3.2).
import { useEffect, useState } from "react";
import { invoke } from "../ipc";
import { hostOnly, revealOnHost } from "../onhost";
import { useArmed } from "../useArmed";

// ── Settings → Usage ────────────────────────────────────────────────────────
// What this Mac's copy of the app has recorded about itself: where the
// foreground hours went, and which controls got clicked. It exists to find the
// DEAD ones — a row of zeros is the finding, not a gap in the data.
//
// Nothing here identifies anything. An event is a control key (a constant in
// the source), a surface (a fixed enum) and a timestamp; `usage.ts` decides
// what may become a key and `valid_token` in lib.rs refuses to store the rest.
// The file never leaves the machine.

type UiUsage = {
  days: string[];
  acts: { key: string; total: number; per_day: number[] }[];
  dwell: { surface: string; ms_total: number; per_day: number[] }[];
  file: string;
};

const USAGE_RANGES = [7, 14, 30] as const;
/** Past this the table stops being readable; the tail folds into one line. */
const USAGE_ROWS = 40;

/** ms → the shortest true reading: `4h 12m`, `12m`, `48s`. */
function dur(ms: number): string {
  const s = Math.round(ms / 1000);
  if (s < 90) return `${s}s`;
  const m = Math.round(s / 60);
  if (m < 90) return `${m}m`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

/** The five-step ramp, as a share of `--accent` mixed into the panel.
 *
 *  Thresholds are FIXED (1 / 3 / 7 / 16) once a day in the range has 16+
 *  actions, so the colour of a cell means the same thing week to week. Below
 *  that the same five steps are spread over the observed maximum instead —
 *  otherwise a light week paints entirely in the first bucket and the map says
 *  nothing at all. Sequential single hue, so the only thing the colour encodes
 *  is "more". */
function usageBuckets(max: number): number[] {
  if (max >= 16) return [1, 3, 7, 16];
  const q = (f: number) => Math.max(1, Math.ceil(max * f));
  const steps = [1, q(0.25), q(0.5), q(0.75)];
  // dedupe upward so two buckets can never share a threshold
  return steps.map((v, i) => (i === 0 ? v : Math.max(v, steps[i - 1] + 1)));
}
const USAGE_MIX = [0, 25, 45, 70, 100];
function usageLevel(n: number, cuts: number[]): number {
  let lvl = 0;
  for (const c of cuts) if (n >= c) lvl++;
  return lvl;
}
const usageCell = (lvl: number) => `color-mix(in srgb, var(--accent) ${USAGE_MIX[lvl]}%, var(--bg-elev))`;

export function UsagePanel() {
  const [days, setDays] = useState<number>(14);
  const [data, setData] = useState<UiUsage | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const arm = useArmed<"clear">();
  const clearArmed = arm.armed === "clear";

  // On open and on range change only — an interval here would make the panel
  // measure itself.
  const load = async (n: number) => {
    try {
      // The backend has no timezone database, and the heatmap's columns are the
      // user's days, not UTC's. `getTimezoneOffset` is minutes WEST of UTC, so
      // the sign flips on the way out.
      const tzOffsetMin = -new Date().getTimezoneOffset();
      setData(await invoke<UiUsage>("ui_usage", { days: n, tzOffsetMin }));
      setErr(null);
    } catch (e) {
      setErr(String(e));
    }
  };
  useEffect(() => { void load(days); }, [days]);

  const doClear = () => arm.confirm("clear", async () => {
    try {
      await invoke("ui_events_clear");
      await load(days);
    } catch (e) {
      const m = `clear usage events failed: ${String(e)}`;
      setErr(m);
      invoke("log_event", { level: "error", msg: m }).catch(() => {});
    }
  });
  const reveal = async () => {
    try {
      if (data?.file) await revealOnHost(data.file);
    } catch (e) {
      const m = `reveal usage file failed: ${String(e)}`;
      setErr(m);
      invoke("log_event", { level: "error", msg: m }).catch(() => {});
    }
  };

  const dwellTotal = (data?.dwell ?? []).reduce((a, d) => a + d.ms_total, 0);
  const max = Math.max(0, ...(data?.acts ?? []).flatMap((a) => a.per_day));
  const cuts = usageBuckets(max);
  const rows = (data?.acts ?? []).slice(0, USAGE_ROWS);
  const hidden = (data?.acts ?? []).length - rows.length;
  const empty = !!data && data.acts.length === 0 && data.dwell.length === 0;

  return (
    <>
      <section className="setting">
        <label>Usage</label>
        <div className="um-head">
          <span className="hint">last {days} days · stays on this Mac</span>
          <select value={days} onChange={(e) => setDays(+e.currentTarget.value)}>
            {USAGE_RANGES.map((n) => <option key={n} value={n}>{n} days</option>)}
          </select>
        </div>
      </section>

      {empty && (
        <section className="setting">
          <div className="hint">Nothing recorded yet — use the app for a day and come back.</div>
        </section>
      )}

      {!!data && data.dwell.length > 0 && (
        <section className="setting">
          <label className="sub">Foreground time by surface</label>
          <div className="um-bars">
            {data.dwell.map((d) => {
              const pct = dwellTotal ? Math.round((d.ms_total / dwellTotal) * 100) : 0;
              return (
                <div className="um-bar-row" key={d.surface}>
                  <span className="um-bar-label">{d.surface}</span>
                  <span className="um-bar-track" title={`${d.surface} · ${dur(d.ms_total)}`}>
                    <span className="um-bar-fill" style={{ width: `${pct}%` }} />
                  </span>
                  <span className="um-bar-val">{dur(d.ms_total)}</span>
                </div>
              );
            })}
          </div>
        </section>
      )}

      {!!data && data.acts.length > 0 && (
        <section className="setting">
          <label className="sub">Actions per day</label>
          {/* Its own scroller: a month of columns is wider than the modal, and
              the body must never scroll sideways as a whole. */}
          <div className="um-grid-scroll">
            <div className="um-grid">
              <div className="um-row um-head-row">
                <span className="um-key" />
                {data.days.map((d) => <span className="um-col" key={d} title={d}>{d.slice(8)}</span>)}
                <span className="um-total">total</span>
              </div>
              {rows.map((a) => (
                <div className={"um-row" + (a.total === 0 ? " um-dead" : "")} key={a.key}>
                  <span className="um-key" title={a.key}>{a.key}</span>
                  {a.per_day.map((n, i) => (
                    <span
                      key={i}
                      className="um-cell"
                      style={{ background: usageCell(usageLevel(n, cuts)) }}
                      title={`${a.key} · ${data.days[i]} · ${n}`}
                    />
                  ))}
                  <span className="um-total">{a.total}</span>
                </div>
              ))}
            </div>
          </div>
          {hidden > 0 && <div className="hint">… {hidden} more</div>}
          <div className="um-legend">
            fewer
            {USAGE_MIX.map((_, i) => <span className="um-cell" key={i} style={{ background: usageCell(i) }} />)}
            more
          </div>
        </section>
      )}

      <section className="setting">
        <div className="ver-rows">
          <div className="ver-row"><span className="ver-path" title={data?.file ?? ""}>{data?.file || "…"}</span></div>
        </div>
        <div className="hint">Control names and surfaces only — never a place name, a path, or anything you typed. Never sent anywhere.</div>
        <div className="ver-actions">
          <button className="ctrl sm" onClick={reveal} {...hostOnly("Reveal in Finder")}>Reveal file</button>
          <button className={"ctrl sm danger" + (clearArmed ? " armed" : "")} onClick={doClear}>
            {clearArmed ? "Clear — sure?" : "Clear"}
          </button>
        </div>
        {err && <pre className="update-log">{err}</pre>}
      </section>
    </>
  );
}
