import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "./ipc";
import type { Settings } from "./settings";

// Settings → Behavior → Auto-fetch origin and Settings → Terminal → "What a
// shell tab remembers": HOST policy (`hostpolicy` in worktrees-api), like
// Codex permissions and pi trust. The host keeps them in host-policy.json and
// applies them to objects only it has — the one loop that fetches for the
// whole host, and the dock shells every window that opens a tab shares — so
// these controls render what the host answers, and only the user's own act on
// them changes it. Nothing here runs on load but a READ.
//
// The shape is CodexPermissionsSection's (SettingsSheet.tsx):
// - every set is followed by a fresh read, and only the newest read may land
//   (`seq`): two quick changes can reach serve out of order, and the control
//   must end on what the host STORED, not on whichever reply came back last;
// - a refusal rolls the control back and is reported through `onFail`;
// - once a set has been ACCEPTED, each read that lands is copied into
//   `Settings` for an older build to find after a downgrade (`settings.ts`).
//   Not before, and only for what was set: the getters answer the EFFECTIVE
//   value (the default while the host holds nothing), which is not a record of
//   what the host holds.

type Props = {
  settings: Settings;
  onChange: (patch: Partial<Settings>) => void;
  onFail: (e: unknown) => void;
};

const FETCH_CHOICES = [
  [0, "Off"],
  [5, "Every 5 min"],
  [15, "Every 15 min"],
  [60, "Every 60 min"],
] as const;

export function FetchIntervalSection({ settings, onChange, onFail }: Props) {
  const [mins, setMins] = useState<number | null>(null);
  const seq = useRef(0);
  const held = useRef(false);
  // A ref, so App's per-render `onChange` does not re-read on every render.
  const mirror = useRef<(m: number) => void>(() => {});
  mirror.current = (m) => { if (settings.fetch_interval_min !== m) onChange({ fetch_interval_min: m }); };
  const readHost = useCallback(() => {
    const n = ++seq.current;
    invoke<number>("get_fetch_interval")
      .then((m) => {
        if (seq.current !== n) return;
        setMins(m);
        if (held.current) mirror.current(m);
      })
      .catch((e) => onFail(`Auto-fetch: ${String(e)}`));
  }, [onFail]);
  useEffect(() => {
    readHost();
    return () => { seq.current++; };
  }, [readHost]);
  const pick = (next: number) => {
    if (mins === null || next === mins) return;
    const prev = mins;
    const n = ++seq.current;
    setMins(next);
    invoke("set_fetch_interval", { mins: next }).then(() => { held.current = true; readHost(); }, (e) => {
      if (seq.current === n) setMins(prev);
      onFail(`Auto-fetch: ${String(e)}`);
      readHost();
    });
  };
  return (
    <section className="setting">
      <label>Git</label>
      <label className="sub">Auto-fetch origin</label>
      <select value={mins ?? ""} disabled={mins === null} onChange={(e) => pick(+e.currentTarget.value)}>
        {mins === null && <option value="">…</option>}
        {FETCH_CHOICES.map(([m, label]) => <option key={m} value={m}>{label}</option>)}
      </select>
      <div className="hint">
        Keeps the divergence counts fresh. Fetches each project's origin in the background.
        Kept by this machine rather than this window.
      </div>
    </section>
  );
}

type TermOpts = { persist_scrollback: boolean; per_tab_history: boolean };

export function TermHistorySection({ settings, onChange, onFail }: Props) {
  const [opts, setOpts] = useState<TermOpts | null>(null);
  const seq = useRef(0);
  // Per switch: a set names one, so only that one is known to be in the
  // host's file; the other may still be answering with its default.
  const held = useRef(new Set<keyof TermOpts>());
  const mirror = useRef<(o: TermOpts) => void>(() => {});
  mirror.current = (o) => {
    const h = held.current;
    if (h.has("persist_scrollback") && settings.term_persist_scrollback !== o.persist_scrollback) {
      onChange({ term_persist_scrollback: o.persist_scrollback });
    }
    if (h.has("per_tab_history") && settings.term_per_tab_history !== o.per_tab_history) {
      onChange({ term_per_tab_history: o.per_tab_history });
    }
  };
  const readHost = useCallback(() => {
    const n = ++seq.current;
    invoke<TermOpts>("get_term_history_opts")
      .then((o) => {
        if (seq.current !== n) return;
        setOpts(o);
        mirror.current(o);
      })
      .catch((e) => onFail(`Terminal history: ${String(e)}`));
  }, [onFail]);
  useEffect(() => {
    readHost();
    return () => { seq.current++; };
  }, [readHost]);
  // Each switch sends ONLY itself (the setter is a patch), so changing one
  // never re-sends this window's copy of the other over another window's
  // change to it.
  const flip = (key: keyof TermOpts, on: boolean) => {
    if (!opts || opts[key] === on) return;
    const prev = opts[key];
    const n = ++seq.current;
    setOpts({ ...opts, [key]: on });
    invoke("set_term_history_opts", key === "persist_scrollback" ? { persistScrollback: on } : { perTabHistory: on })
      .then(() => { held.current.add(key); readHost(); }, (e) => {
        if (seq.current === n) setOpts((o) => (o ? { ...o, [key]: prev } : o));
        onFail(`Terminal history: ${String(e)}`);
        readHost();
      });
  };
  return (
    <section className="setting">
      <label>What a shell tab remembers</label>
      <label className="tier-toggle setting-check">
        <input
          type="checkbox"
          checked={opts?.persist_scrollback ?? false}
          disabled={opts === null}
          onChange={(e) => flip("persist_scrollback", e.currentTarget.checked)}
        />
        Keep each tab's output between restarts
      </label>
      <div className="hint">
        A dock shell dies with the app, so its tab used to reopen blank. Its last
        256K of output is saved and replayed instead. This is the only thing that
        writes a terminal's output to disk — whatever a command printed, secrets
        included — under Application Support. Clear it under Data &amp; Logs.
      </div>
      <label className="tier-toggle setting-check">
        <input
          type="checkbox"
          checked={opts?.per_tab_history ?? false}
          disabled={opts === null}
          onChange={(e) => flip("per_tab_history", e.currentTarget.checked)}
        />
        Give each tab its own command history
      </label>
      <div className="hint">
        Arrow-up recalls what this tab ran, rather than the one history every
        terminal on the Mac shares. zsh and bash only; your own rc files are read
        as usual and never modified. Turn it off if a shell setup of yours depends
        on $ZDOTDIR.
      </div>
      <div className="hint">
        Both are kept by this machine rather than this window: every window that
        opens a tab shares its shell.
      </div>
    </section>
  );
}
