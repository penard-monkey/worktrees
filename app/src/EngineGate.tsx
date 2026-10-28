// The app's view of its engine — this host's `worktrees serve`.
//
// Before the engine has answered once, the app is not mounted at all: App
// hydrates its settings on mount, and against a dead engine `loadSettings`
// answers DEFAULTS, which a later whole-blob save would write over the user's
// real ones. So the gate shows what is happening instead — starting, or why it
// could not, with the log that says more and a way to try again.
//
// After that, losing the engine must not look like a closed place or a blank
// app (proposal §III.7, "a dropped socket must not look like a closed place"):
// the app stays mounted, showing what it last knew, under a banner that says
// the engine is gone and that the app is reconnecting — on its own, on a
// backoff (reconnect.ts), with "Reconnect now" to skip the wait. Reconnecting
// re-runs the shim's find-or-start, re-subscribes every listener, and bumps
// the engine's `gen`, which the terminals carry in their keys — so each
// re-attaches through the same remount it always had (a tmux pane redraws; a
// dock shell replays its ring behind the `── restored ──` seam). The banner
// says "Disconnected" only once the retries have stopped (the crash-loop
// limit), with the reason and the log that says more.
import { useEffect, useState, useSyncExternalStore, type ReactNode } from "react";
import { engineState, invoke, notGranted, onEngineState, reconnectEngine, remoteHost, type EngineState } from "./ipc";
import { dismissAlert, labelOf, openRemoteSettings, useRemote, watchRemote } from "./remoteStore";
import { alertText, remoteNoticeWords, type RemoteStatus } from "./devices";
import "./tokens.css";
import "./EngineGate.css";
import "./RemoteAccess.css";

/** Where serve logs: the place that says why it would not start. */
const SERVE_LOG = "~/.cache/worktrees/serve.log";

/** The engine state, live. `gen` 0 everywhere but the engine door. */
export function useEngine(): EngineState {
  return useSyncExternalStore(onEngineState, engineState);
}

/** Whole seconds until `at` (on `performance.now()`'s clock, the one main.tsx
 *  hands the retries), re-rendered while it counts. Rounded UP: the retry is
 *  never later than nominal, so "in 3 s" is a promise the jitter keeps. */
function useSecondsUntil(at: number | null): number {
  const [now, setNow] = useState(() => performance.now());
  useEffect(() => {
    if (at === null) return;
    setNow(performance.now());
    const id = window.setInterval(() => setNow(performance.now()), 250);
    return () => window.clearInterval(id);
  }, [at]);
  return at === null ? 0 : Math.max(0, Math.ceil((at - now) / 1000));
}

/** Back to this machine's own engine, from a window that drives a paired host
 *  it cannot reach: the choice is the shim's, and the door is decided at boot. */
async function driveThisMachine() {
  try {
    await invoke("remote_choose", { hostId: null });
  } finally {
    window.location.reload();
  }
}

/** What a try in flight is doing, in words: the shim's phase, if it sent one. */
function phaseText(phase: string | null, first: boolean): string {
  const host = remoteHost();
  if (host) return first ? `Connecting to ${host}…` : `Reconnecting to ${host}…`;
  if (phase === "waiting-old") return "Waiting for the old engine to stop…";
  if (phase === "starting" || first) return "Starting the engine…";
  return "Reconnecting to the engine…";
}

export function EngineGate({ children }: { children: ReactNode }) {
  const st = useEngine();
  if (st.gen === 0 && st.kind !== "up") return <EngineBoot st={st} />;
  return (
    <>
      {children}
      {st.kind !== "up" && <EngineBanner st={st} />}
      {st.kind === "up" && <RemoteLayer key={st.gen} />}
    </>
  );
}

/** Remote access's surfaces outside App: the watch on serve's console
 *  events, and one corner (bottom-right, clear of the bottom band and a
 *  right rail) holding §5.3's alerts. The notice that remote access is on is
 *  not here: it is App's, in the bottom bar (RemoteNotice below). Inside the
 *  gate while the engine is up, keyed by its `gen` so a reconnect re-reads
 *  and re-listens; main.tsx mounts it beside an ungated App (the harness).
 *  The left edge is the nav's and the rail's, whose bottom holds the
 *  Settings gear an alert there would cover. */
export function RemoteLayer() {
  const { alerts } = useRemote();
  // The alerts are about THIS machine's own engine: a window that drives a
  // paired host is not on it, and `remote_status` is the console's (ipc.ts
  // CONSOLE_ROWS). Its watch still names devices.
  const onConsole = !notGranted("remote_status");
  const shown = onConsole && alerts.length > 0;
  const at = useClearCorner(shown);
  return (
    <>
      <RemoteWatch />
      {shown && (
        <div className="remote-corner" style={{ bottom: at.bottom, right: at.right }}>
          <RemoteAlerts />
        </div>
      )}
    </>
  );
}

function RemoteWatch() {
  useEffect(() => watchRemote(), []);
  return null;
}

/** The status App's bottom bar shows while remote access is on, or null for
 *  no notice: only on the console (a window that drives a paired host is not
 *  on this machine's engine, and `remote_status` is the console's), only
 *  while the engine is up (RemoteLayer's watch is what keeps the status
 *  current, and it runs only then), and never for off or unreadable
 *  (`remoteNoticeWords`). App's bar for it exists whenever this is not null
 *  (`remoteNoticeHost`). */
export function useRemoteNotice(): RemoteStatus | null {
  const { status } = useRemote();
  const up = useEngine().kind === "up";
  return up && !notGranted("remote_status") && remoteNoticeWords(status) !== null ? status : null;
}

/** While remote access is on, the window says so, persistently (§4.1), in
 *  the bottom bar the status chip uses (App's `remoteNoticeHost`: the
 *  window-wide `.usage-strip`, or a place's `.statusbar` when the usage meter
 *  lives there), on a place and on Home. It does not say
 *  what quitting does: an app-owned serve still stops with the app (§9.3),
 *  and Settings → Remote access says so. A click opens that section, which
 *  is also the one-click way back to off. `remote_status` comes from
 *  remoteStore.ts, re-read on every `remote:changed` (the console's alone). */
export function RemoteNotice({ st }: { st: RemoteStatus }) {
  return (
    <button className="remote-notice" title="Settings → Remote access" onClick={() => openRemoteSettings("remote")}>
      <span className="remote-notice-in">
        <span className={`engine-dot remote${st.state === "listening" ? "" : " starting"}`} aria-hidden />
        <span className="remote-notice-txt">{remoteNoticeWords(st)}</span>
      </span>
    </button>
  );
}

/** §5.3's alerts, for the person at this computer: a calm notice per alert,
 *  one click to dismiss and one to look. Never a modal. */
function RemoteAlerts() {
  const alerts = useRemote().alerts;
  return (
    <div className="ra-alerts ra" aria-live="polite">
      {alerts.map((a) => (
        <div key={a.key} className="ra-alert" role="status" data-alert={a.key}>
          <span className="ra-dot warn" aria-hidden />
          <span className="ra-alert-txt">{a.payload.alert === "question" ? String(a.payload.text) : alertText(a, labelOf)}</span>
          <span className="ra-alert-acts">
            <button className="ctrl sm" onClick={() => { dismissAlert(a.key); openRemoteSettings(a.cat); }}>Review</button>
            <button className="ra-alert-x" aria-label="Dismiss" title="Dismiss" onClick={() => dismissAlert(a.key)}>×</button>
          </span>
        </div>
      ))}
    </div>
  );
}

/** Where the alerts sit clear of what is already in the bottom-right
 *  corner: above the bottom band (the usage strip, or a place's footer) and
 *  left of a rail on the right, re-measured as those resize.
 *  A fixed card over either would hide the meter's last reading, the remote
 *  notice or a rail button. */
function useClearCorner(on: boolean): { bottom: string; right: string } {
  const [at, setAt] = useState({ bottom: 0, right: 0 });
  useEffect(() => {
    if (!on) return;
    const measure = () => {
      const band = document.querySelector(".usage-strip, .statusbar");
      const bottom = band ? Math.max(0, window.innerHeight - band.getBoundingClientRect().top) : 0;
      const rails = [...document.querySelectorAll(".rail")].map((r) => r.getBoundingClientRect()).filter((b) => b.left > window.innerWidth / 2);
      const right = rails.length ? Math.max(0, window.innerWidth - Math.min(...rails.map((b) => b.left))) : 0;
      setAt((cur) => (cur.bottom === bottom && cur.right === right ? cur : { bottom, right }));
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(document.body);
    for (const el of document.querySelectorAll(".usage-strip, .statusbar, .rail")) ro.observe(el);
    const id = window.setInterval(measure, 2000);
    return () => {
      ro.disconnect();
      window.clearInterval(id);
    };
  }, [on]);
  return { bottom: `calc(${at.bottom}px + var(--s2))`, right: `calc(${at.right}px + var(--s3))` };
}

/** When the next try is, what the one in flight is doing, or why it is not
 *  counting down. */
function NextTry({ st }: { st: EngineState }) {
  const secs = useSecondsUntil(st.kind === "waiting" ? st.at : null);
  if (st.kind === "waiting") return <>Next try in {secs} s.</>;
  if (st.kind === "hidden") return <>The next try waits until this window is visible.</>;
  if (st.kind === "starting") return <>{phaseText(st.phase, false)}</>;
  return null;
}

function EngineBoot({ st }: { st: EngineState }) {
  // The launch's own try, before anything has failed. Once one has, the card
  // keeps saying why, and only its last line follows the retries — rather
  // than flipping between two cards on every try.
  if (st.kind === "up" || (st.kind === "starting" && st.reason === null)) {
    return (
      <div className="engine-boot">
        <div className="engine-boot-card" role="status">
          <div className="engine-title">
            <span className="engine-dot starting" aria-hidden /> {phaseText(st.kind === "starting" ? st.phase : null, true)}
          </div>
        </div>
      </div>
    );
  }
  const down = st.kind === "down";
  const host = remoteHost();
  return (
    <div className="engine-boot">
      <div className="engine-boot-card" role={down ? "alert" : "status"}>
        <div className="engine-title">
          <span className={`engine-dot ${down ? "down" : "starting"}`} aria-hidden />{" "}
          {host ? `worktrees could not reach ${host} yet` : down ? "worktrees could not start its engine" : "worktrees could not reach its engine yet"}
        </div>
        <p className="engine-why">{st.reason ?? ""}</p>
        {!down && (
          <p className="engine-log">
            <NextTry st={st} />
          </p>
        )}
        <p className="engine-log">
          {host ? <>The engine’s log is on {host}: <code>{SERVE_LOG}</code></> : <>The engine’s log: <code>{SERVE_LOG}</code></>}
        </p>
        <button className="enter-btn" autoFocus onClick={() => void reconnectEngine()}>
          Reconnect now
        </button>
        {host && (
          <button className="enter-btn" onClick={() => void driveThisMachine()}>
            Drive this machine instead
          </button>
        )}
      </div>
    </div>
  );
}

function EngineBanner({ st }: { st: EngineState }) {
  const down = st.kind === "down";
  const trying = st.kind === "starting";
  const retrying = st.kind === "waiting" || st.kind === "hidden";
  const notOurs = retrying && st.notOurs;
  return (
    <div className={`engine-banner${down ? " down" : ""}`} role={down ? "alert" : "status"}>
      <span className={`engine-dot ${down ? "down" : "starting"}`} aria-hidden />
      <span className="engine-banner-txt">
        {down ? (
          <>
            <b>Disconnected from the engine.</b> {st.reason} Its log: <code>{SERVE_LOG}</code>
          </>
        ) : trying ? (
          <b>{phaseText(st.phase, false)}</b>
        ) : notOurs ? (
          <>
            <b>The engine was started outside this app, and it is not running.</b> The app reconnects when it is back
            and does not start one in its place; Reconnect now starts the app’s own. <NextTry st={st} />
          </>
        ) : (
          <>
            <b>Reconnecting…</b> <NextTry st={st} />
            {retrying && <span className="engine-banner-why">{st.reason}</span>}
          </>
        )}
      </span>
      {/* Focused when it appears: the terminal that had the keyboard is
          talking to nothing now, and Enter here is the one key that helps.
          Never disabled: a press during a try joins it (one at a time,
          reconnect.ts), and disabling a focused button drops the focus. */}
      <button className="enter-btn sm" autoFocus onClick={() => void reconnectEngine()}>
        Reconnect now
      </button>
    </div>
  );
}
