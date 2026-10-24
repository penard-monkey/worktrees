// The pieces Settings → Remote access, → Devices and the pairing sheet share:
// a confirm dialog, the scope editor (what a device may do, and where), the
// expiry editor, and two small hooks. The words are devices.ts's.
import { useEffect, useId, useState, type ReactNode } from "react";
import { invoke } from "./ipc";
import { useEscape } from "./useEscape";
import {
  CAP_WORDS, CAPS, DEFAULT_IDLE_DAYS, MAX_IDLE_DAYS, PRESET_LABEL, PRESETS, dayOf, normCaps, presetOf, runsCommands,
  runsCommandsLabel, untilEndOf, type Cap, type Expiry, type Preset, type Projects,
} from "./devices";

/** `Date.now()` at every render, with a render every `ms` while mounted: what
 *  a countdown reads. The tick only schedules renders. A clock kept from the
 *  last tick is up to `ms` old in a render something else caused, and the
 *  pairing window's answer is one: read a second boundary late, 120 s opened
 *  at "2:01". */
export function useNow(ms = 250): number {
  const [, setTick] = useState(0);
  useEffect(() => {
    const id = window.setInterval(() => setTick((t) => t + 1), ms);
    return () => window.clearInterval(id);
  }, [ms]);
  return Date.now();
}

/** Host policy lets agents act unprompted (Codex permissions other than
 *  `ask`): what §1.1's label adds `instruct` alone for. `false` until read. */
export function useUnprompted(): boolean {
  const [mode, setMode] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    invoke<string>("get_codex_permissions").then((m) => live && setMode(m), () => {});
    return () => { live = false; };
  }, []);
  return mode !== null && mode !== "ask";
}

export type ProjectRef = { root: string; name: string };

/** The registered projects, by registry key: what a device's scope names. */
export function useProjects(): ProjectRef[] | null {
  const [list, setList] = useState<ProjectRef[] | null>(null);
  useEffect(() => {
    let live = true;
    invoke<{ projects: { root: string; name?: string | null }[] }>("list_workspace").then(
      (ws) => live && setList(ws.projects.map((p) => ({ root: p.root, name: p.name || p.root.split("/").pop() || p.root }))),
      () => live && setList([]),
    );
    return () => { live = false; };
  }, []);
  return list;
}

/** A question with one consequential answer, over whatever is open. */
export function ConfirmDialog({ title, children, confirm, danger, busy, error, onConfirm, onClose }: {
  title: string;
  children: ReactNode;
  confirm: string;
  /** Hard to undo: Cancel, not the act, has the focus. */
  danger?: boolean;
  busy?: boolean;
  error?: string | null;
  onConfirm: () => void;
  onClose: () => void;
}) {
  useEscape(() => { if (!busy) onClose(); });
  return (
    <div className="scrim scrim-center" onClick={() => !busy && onClose()}>
      <div className="sync-modal ra-confirm ra" role="dialog" aria-label={title} onClick={(e) => e.stopPropagation()}>
        <header className="sync-h"><b>{title}</b></header>
        <div className="sync-body">
          {children}
          {error && <div className="ra-err" role="alert">{error}</div>}
        </div>
        <footer className="sync-foot">
          {/* Enter takes the safe answer of a destructive question. */}
          <button className="ctrl" autoFocus={danger} disabled={busy} onClick={onClose}>Cancel</button>
          <button className="enter-btn" autoFocus={!danger} disabled={busy} onClick={onConfirm}>
            {busy ? "Working…" : confirm}
          </button>
        </footer>
      </div>
    </div>
  );
}

/** The "can run commands on <host> as you" line, when §1.1 says it applies. */
export function RunsLabel({ caps, unprompted, host }: { caps: readonly Cap[]; unprompted: boolean; host?: string }) {
  if (!runsCommands(caps, unprompted)) return null;
  return (
    <div className="ra-runs" data-testid="runs-label">
      <span className="ra-dot warn" aria-hidden />
      <span>This device {runsCommandsLabel(host)}.</span>
    </div>
  );
}

export type Scope = { caps: Cap[]; projects: Projects };

/** What a device may do (a preset, or a custom set, with host admin on top)
 *  and where (every project, or a list that starts EMPTY, §2.2 step 1: a
 *  scope is always chosen, never inherited). */
export function ScopeEditor({ value, onChange, projects, unprompted, host }: {
  value: Scope;
  onChange: (s: Scope) => void;
  projects: ProjectRef[] | null;
  unprompted: boolean;
  host?: string;
}) {
  const preset = presetOf(value.caps);
  const [custom, setCustom] = useState(preset === "custom");
  const admin = value.caps.includes("admin");
  const pick = (p: Preset | "custom") => {
    if (p === "custom") { setCustom(true); return; }
    setCustom(false);
    onChange({ ...value, caps: normCaps([...PRESETS[p], ...(admin ? (["admin"] as Cap[]) : [])]) });
  };
  const toggle = (c: Cap, on: boolean) => onChange({ ...value, caps: normCaps(on ? [...value.caps, c] : value.caps.filter((x) => x !== c)) });
  const shown = custom ? "custom" : preset;
  const list = value.projects === "*" ? null : value.projects;
  const group = useId();
  return (
    <div className="ra-scope">
      <div className="ra-field">
        <span className="ra-field-l">What it may do</span>
        <div className="seg" role="radiogroup" aria-label="What it may do">
          {(["observe", "operate", "drive", "custom"] as const).map((p) => (
            <button key={p} className={"seg-b" + (shown === p ? " on" : "")} role="radio" aria-checked={shown === p} onClick={() => pick(p)}>
              {PRESET_LABEL[p]}
            </button>
          ))}
        </div>
        {shown !== "custom" && (
          <div className="ra-note">{PRESETS[shown].map((c) => CAP_WORDS[c]).join("; ")}.</div>
        )}
        {shown === "custom" && (
          <div className="ra-caps">
            {CAPS.filter((c) => c !== "admin").map((c) => (
              <label key={c} className="ra-check">
                <input type="checkbox" checked={value.caps.includes(c)} onChange={(e) => toggle(c, e.currentTarget.checked)} />
                <span><b>{c}</b> — {CAP_WORDS[c]}</span>
              </label>
            ))}
          </div>
        )}
        <label className="ra-check">
          <input type="checkbox" checked={admin} onChange={(e) => toggle("admin", e.currentTarget.checked)} />
          <span><b>Host admin</b> — {CAP_WORDS.admin}</span>
        </label>
        <RunsLabel caps={value.caps} unprompted={unprompted} host={host} />
      </div>
      <div className="ra-field">
        <span className="ra-field-l">Where</span>
        <label className="ra-check">
          <input type="radio" name={group} checked={value.projects === "*"} onChange={() => onChange({ ...value, projects: "*" })} />
          <span>Every project, including ones added later</span>
        </label>
        <label className="ra-check">
          <input type="radio" name={group} checked={value.projects !== "*"} onChange={() => onChange({ ...value, projects: list ?? [] })} />
          <span>Only these projects</span>
        </label>
        {list && (
          <div className="ra-projects">
            {projects === null && <div className="ra-note">Reading the projects…</div>}
            {projects?.length === 0 && <div className="ra-note">No projects are registered.</div>}
            {projects?.map((p) => (
              <label key={p.root} className="ra-check" title={p.root}>
                <input type="checkbox" checked={list.includes(p.root)}
                  onChange={(e) => {
                    const on = e.currentTarget.checked;
                    onChange({ ...value, projects: on ? [...list, p.root] : list.filter((r) => r !== p.root) });
                  }} />
                <span>{p.name}</span>
              </label>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

/** A scope serve would take: some capability, and a project list that is
 *  not empty. The reason when it is not. */
export function scopeProblem(s: Scope): string | null {
  if (s.caps.length === 0) return "Choose at least one thing it may do.";
  if (s.projects !== "*" && s.projects.length === 0) return "Choose its projects, or every project.";
  return null;
}

type ExpiryKind = "idle" | "until" | "never";

/** When a device stops being served (§2.5): N days unused (default 30), a
 *  date, or never. */
export function ExpiryEditor({ value, onChange }: { value: Expiry; onChange: (e: Expiry) => void }) {
  const group = useId();
  const kind: ExpiryKind = value === "never" ? "never" : "idle_days" in value ? "idle" : "until";
  const [days, setDays] = useState(kind === "idle" ? (value as { idle_days: number }).idle_days : DEFAULT_IDLE_DAYS);
  const [day, setDay] = useState(kind === "until" ? dayOf((value as { until: string }).until) : "");
  const set = (k: ExpiryKind, d = days, on = day) => {
    if (k === "never") onChange("never");
    else if (k === "idle") onChange({ idle_days: d });
    else {
      const until = untilEndOf(on);
      if (until) onChange({ until });
    }
  };
  return (
    <div className="ra-field">
      <span className="ra-field-l">Expiry</span>
      <label className="ra-check mid">
        <input type="radio" name={group} checked={kind === "idle"} onChange={() => set("idle")} />
        <span>
          Disabled after{" "}
          <input className="ra-days" type="number" min={1} max={MAX_IDLE_DAYS} value={days} aria-label="Days unused"
            onChange={(e) => {
              const n = Math.min(MAX_IDLE_DAYS, Math.max(1, Math.floor(+e.currentTarget.value || 1)));
              setDays(n);
              if (kind === "idle") onChange({ idle_days: n });
            }} />{" "}
          days unused
        </span>
      </label>
      <label className="ra-check mid">
        <input type="radio" name={group} checked={kind === "until"} onChange={() => { if (day) set("until"); else { const t = new Date(Date.now() + 7 * 86400_000); const d = dayOf(t.toISOString()); setDay(d); set("until", days, d); } }} />
        <span>
          Until{" "}
          <input className="ra-date" type="date" value={day} aria-label="Expiry date"
            onChange={(e) => { const d = e.currentTarget.value; setDay(d); if (d) set("until", days, d); }} />
        </span>
      </label>
      <label className="ra-check">
        <input type="radio" name={group} checked={kind === "never"} onChange={() => set("never")} />
        <span>Never — the device list marks it</span>
      </label>
    </div>
  );
}
