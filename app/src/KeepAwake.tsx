import { useEffect, useRef, useSyncExternalStore } from "react";
import { engineState, invoke, listen, notGranted, onEngineState, remoteHost } from "./ipc";
import * as Icons from "./icons";
import { SECTION } from "./settingsMap";

// Keep awake: the HOST's idle sleep, held off by the engine (worktrees-api
// `keepawake`). Two controls on one state: the rail's cup and Settings →
// General → Keep awake. The rail is in every layout, so its button is the one
// that watches; Settings reads the same store.
//
// Host policy (hostpolicy-check.mjs): loading a window only READS. The set is
// sent by the person's own click, and what either control shows is what the
// host answered, never this window's guess. `keep_awake:changed` carries the
// whole state, so a toggle in another window or on a phone lands here too.

/** `keepawake::State`. */
export type KeepAwake = {
  supported: boolean;
  on: boolean;
  held: boolean;
  mechanism: string | null;
  os: string;
  error: string | null;
};

let state: KeepAwake | null = null;
/** An act in flight: the controls wait for it. */
let busy = false;
/** Newest news wins: every read, act and event takes a number, and an answer
 *  overtaken by a later one is dropped. */
let seq = 0;
const subs = new Set<() => void>();
const tell = () => { for (const f of [...subs]) f(); };
const subscribe = (f: () => void) => { subs.add(f); return () => { subs.delete(f); }; };
const land = (s: KeepAwake | null) => { state = s; tell(); };
const snapshot = () => state;
const snapshotBusy = () => busy;

export const useKeepAwake = () => ({
  state: useSyncExternalStore(subscribe, snapshot),
  busy: useSyncExternalStore(subscribe, snapshotBusy),
});

function read(onFail: (e: unknown) => void) {
  const n = ++seq;
  invoke<KeepAwake>("get_keep_awake").then(
    (s) => { if (seq === n) land(s); },
    (e) => {
      if (seq !== n) return;
      land(null);
      // A host from before the feature has no such row: it has no button
      // either, which is all there is to say.
      if (!/unknown command/.test(String(e))) onFail(`Keep awake: ${String(e)}`);
    },
  );
}

/** Turn it. Called by a click or a switch and by nothing else. */
function turn(on: boolean, onFail: (e: unknown) => void) {
  if (busy) return;
  const n = ++seq;
  busy = true;
  tell();
  invoke<KeepAwake>("set_keep_awake", { on }).then(
    (s) => { busy = false; if (seq === n) land(s); else tell(); },
    (e) => { busy = false; tell(); onFail(`Keep awake: ${String(e)}`); read(onFail); },
  );
}

/** Whose computer: this one, or the paired host this window drives. */
function computer(s: KeepAwake): string {
  return remoteHost() ?? (s.os === "macos" ? "this Mac" : "this computer");
}

export function keepAwakeTitle(s: KeepAwake, who: string): string {
  if (!s.on) return `Keep ${who} awake, so agents can keep working`;
  if (!s.held) return `Not keeping ${who} awake: ${s.error ?? "the hold was lost"} — click to turn it off`;
  return `Keeping ${who} awake, so agents can keep working — click to let it sleep`;
}

/** The rail's cup. Renders nothing until the host has answered, and nothing
 *  at all where the host has no way to hold sleep off. Module scope: a
 *  component declared inside App remounts on every render. */
export function KeepAwakeRailButton({ onFail }: { onFail: (e: unknown) => void }) {
  const { state: s, busy: waiting } = useKeepAwake();
  const gen = useSyncExternalStore(onEngineState, engineGen);
  // A ref, so a new `onFail` identity does not read again.
  const fail = useRef(onFail);
  fail.current = onFail;
  useEffect(() => {
    const un = listen<KeepAwake>("keep_awake:changed", (e) => { seq++; land(e.payload); });
    return () => { un.then((f) => f()).catch(() => {}); };
  }, []);
  // At mount, and again on every new engine: what happened while the socket
  // was down was announced to nobody here.
  useEffect(() => {
    if (gen !== null) read((e) => fail.current(e));
  }, [gen]);
  if (!s?.supported) return null;
  // The row the Settings section names as its write, so the two cannot differ.
  const refused = notGranted(SECTION["keep-awake"].write!);
  const title = keepAwakeTitle(s, computer(s));
  return <button
    className={"rail-icon keep-awake" + (s.held ? " lit" : "") + (s.on && !s.held ? " rail-waiting" : "")}
    data-track="keep-awake"
    data-testid="keep-awake-rail"
    aria-label="Keep awake"
    aria-pressed={s.on}
    disabled={!!refused || waiting}
    title={refused ? `${title} (${refused})` : title}
    onClick={() => turn(!s.on, onFail)}
  ><Icons.Coffee size={17} /></button>;
}

/** The engine generation while it is up, or null: a number to re-read on. */
function engineGen(): number | null {
  const e = engineState();
  return e.kind === "up" ? e.gen : null;
}

/** Settings → General → Keep awake. */
export function KeepAwakeSection({ onFail }: { onFail: (e: unknown) => void }) {
  const { state: s, busy: waiting } = useKeepAwake();
  const who = s ? computer(s) : "this computer";
  return (
    <section className="setting">
      <label>Keep awake</label>
      <label className="tier-toggle setting-check">
        <input
          type="checkbox"
          data-testid="keep-awake-setting"
          checked={s?.on ?? false}
          disabled={!s?.supported || waiting}
          onChange={(e) => turn(e.currentTarget.checked, onFail)}
        />
        Keep {who} awake while Worktrees runs
      </label>
      <div className="hint">
        {s && !s.supported
          ? "Not available here: it needs systemd and systemd-inhibit on Linux."
          : <>
            Holds off idle sleep with {s?.os === "linux" ? "systemd-inhibit" : "a macOS power assertion, as caffeinate does"}, whether
            or not an agent is busy, until you turn it off or Worktrees quits. The screen may still turn off, and closing the lid still sleeps.
          </>}
      </div>
      {s?.on && !s.held && <div className="hint" role="alert">Not held: {s.error ?? "the hold was lost"}.</div>}
    </section>
  );
}
