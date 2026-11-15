// One pairing window, as both of its hosts draw it: Settings' pairing sheet
// (PairSheet: a device, or *Invite a guest*) and the share dialog's Pair step
// (LaneShare's ShareDialog, group chat start design §3.1). One implementation
// of everything that makes a window safe, so the two cannot drift:
//
// - the window is serve's (`pair_status`), read every second and at once on
//   every pairing step serve announces (remoteStore's `pairTick`);
// - one attempt; *Cancel pairing*; another window (here, another invite, the
//   CLI) closing this one reads as `superseded`;
// - closing the host while a window is open cancels it (`usePairWindow`'s
//   unmount): an open window nobody is watching is one a stranger could use;
// - Allow is the pairing consent and nothing else. This file never shares a
//   session: `share_create` is not in it (groupchat-start-check.mjs).
import { useEffect, useRef, useState } from "react";
import { invoke } from "./ipc";
import { copyToClipboard } from "./clipboard";
import { useRemote } from "./remoteStore";
import { RunsLabel, useNow, useUnprompted } from "./remoteui";
import {
  capsWords, codeSpaced, ENDED_WORDS, expiryWords, PLATFORM_LABEL, qrPath, scopeWords, secondsLeft,
  type Cap, type DeviceRecord, type PairBegin, type PairStatus,
} from "./devices";
import "./RemoteAccess.css";

export function clock(s: number): string {
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** The QR as SVG, black on white whatever the theme: a camera reads dark on
 *  light, and a theme's inversion would cost a scan. Four modules of quiet
 *  zone, as the standard asks. */
function Qr({ size, modules }: { size: number; modules: string }) {
  const q = 4;
  return (
    <svg className="ra-qr" viewBox={`${-q} ${-q} ${size + 2 * q} ${size + 2 * q}`} role="img" aria-label="Pairing QR code" shapeRendering="crispEdges">
      <rect x={-q} y={-q} width={size + 2 * q} height={size + 2 * q} fill="#ffffff" />
      <path d={qrPath(size, modules)} fill="#000000" />
    </svg>
  );
}

export type PairPhase = "idle" | "open" | "pending" | "closed";

export type PairWindow = {
  phase: PairPhase;
  /** open or pending: a window serve holds for us. */
  live: boolean;
  win: PairBegin | null;
  st: PairStatus | null;
  /** The device Allow enrolled, once it did. */
  paired: DeviceRecord | null;
  replaced: boolean;
  listenerStopped: boolean;
  busy: boolean;
  err: string | null;
  setErr: (e: string | null) => void;
  /** `pair_begin` with these arguments; the window is ours once it answers. */
  begin: (args: Record<string, unknown>) => Promise<boolean>;
  /** Allow or Deny; answers the enrolled device on Allow. */
  decide: (allow: boolean) => Promise<DeviceRecord | null>;
  cancel: () => Promise<void>;
  /** Back to idle, to open another. */
  reset: () => void;
  /** The host is closing: cancel a window still open. */
  release: () => void;
};

/** The window's state, polling while one is ours, cancelling it on unmount. */
export function usePairWindow(): PairWindow {
  const r = useRemote();
  const [win, setWin] = useState<PairBegin | null>(null);
  const [st, setSt] = useState<PairStatus | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [paired, setPaired] = useState<DeviceRecord | null>(null);

  useEffect(() => {
    if (!win) return;
    let live = true;
    const read = () => invoke<PairStatus>("pair_status").then((s) => live && setSt(s), (e) => live && setErr(String(e)));
    void read();
    const id = window.setInterval(read, 1000);
    return () => { live = false; window.clearInterval(id); };
  }, [win, r.pairTick]);

  // Another window (`worktrees serve pair`, say) replaced ours.
  const replaced = !!win && st?.state === "open" && st.uri !== win.uri;
  const listenerStopped = !!win && !!r.status && r.status.state !== "listening";
  const phase: PairPhase =
    !win ? "idle" : listenerStopped ? "closed" : replaced ? "closed" : st?.state === "pending" ? "pending" : st?.state === "closed" ? "closed" : "open";
  const live = phase === "open" || phase === "pending";

  const liveRef = useRef(live);
  liveRef.current = live;
  // Unmounting with a window open (the sheet or dialog closed under it): cancel it.
  useEffect(() => () => { if (liveRef.current) void invoke("pair_cancel").catch(() => {}); }, []);

  const begin = async (args: Record<string, unknown>) => {
    setBusy(true);
    setErr(null);
    setPaired(null);
    setSt(null);
    try {
      const b = await invoke<PairBegin>("pair_begin", args);
      setWin(b);
      setSt({ state: "open", uri: b.uri, expires_at: b.expires_at });
      return true;
    } catch (e) {
      setErr(String(e));
      return false;
    } finally {
      setBusy(false);
    }
  };
  const decide = async (allow: boolean) => {
    setBusy(true);
    setErr(null);
    try {
      const rec = await invoke<DeviceRecord | null>("pair_decide", { allow });
      if (rec) setPaired(rec);
      return rec;
    } catch (e) {
      setErr(String(e));
      return null;
    } finally {
      setBusy(false);
      void invoke<PairStatus>("pair_status").then(setSt, () => {});
    }
  };
  const cancel = async () => {
    await invoke("pair_cancel").catch((e) => setErr(String(e)));
    void invoke<PairStatus>("pair_status").then(setSt, () => {});
  };
  const reset = () => {
    setWin(null);
    setSt(null);
    setErr(null);
    setPaired(null);
  };
  const release = () => {
    if (liveRef.current) void invoke("pair_cancel").catch(() => {});
    liveRef.current = false;
  };
  return { phase, live, win, st, paired, replaced, listenerStopped, busy, err, setErr, begin, decide, cancel, reset, release };
}

/** The header's countdown while the window is open. */
export function PairCountdown({ w }: { w: PairWindow }) {
  const now = useNow(250);
  if (w.phase !== "open" || !w.st || !("expires_at" in w.st)) return null;
  return <span className="ra-countdown" data-testid="pair-countdown">Closes in {clock(secondsLeft(w.st.expires_at, now))}</span>;
}

/** The open window: the QR, the link and Copy, and who it is for. `guest`:
 *  an invite's, named, whose words speak to a person on a Mac (F9) rather
 *  than to a phone and the CLI. */
export function PairOpenBody({ w, guest, summary, caps }: {
  w: PairWindow; guest?: string; summary: string; caps?: readonly Cap[];
}) {
  const [copied, setCopied] = useState(false);
  const unprompted = useUnprompted();
  const host = useRemote().status?.host;
  if (w.phase !== "open" || !w.win) return null;
  const win = w.win;
  return (
    <div className="ra-window">
      <Qr size={win.qr.size} modules={win.qr.modules} />
      <div className="ra-window-txt">
        {guest !== undefined ? (
          <div className="ra-line" data-testid="pair-send-link">
            Send {guest || "them"} this link. In worktrees they choose <b>Join with a link</b> and paste it.
          </div>
        ) : (
          <div className="ra-line">
            Scan this with the device, or paste the link on a device with no camera:
            <code className="ra-cmd">worktrees remote pair '&lt;link&gt;'</code>
          </div>
        )}
        <div className="ra-link">
          <input readOnly value={win.uri} aria-label="Pairing link" onFocus={(e) => e.currentTarget.select()} />
          <button className="ctrl sm" data-testid="pair-copy" onClick={() => copyToClipboard(win.uri).then(() => { setCopied(true); window.setTimeout(() => setCopied(false), 2000); }, (e) => w.setErr(String(e)))}>
            {copied ? "Copied" : "Copy"}
          </button>
        </div>
        <div className="ra-summary">{summary}</div>
        {caps && <RunsLabel caps={caps} unprompted={unprompted} host={host} />}
        {guest !== undefined && <div className="ra-note">One pairing window at a time: opening another closes this one, and this link then stops working. Closing this dialog cancels it.</div>}
        <div className="ra-status" role="status">
          <span className="ra-dot warn" aria-hidden />
          {w.st?.state === "attempt"
            ? <span>A device is connecting from {w.st.peer}…</span>
            : guest !== undefined
              ? <span>Waiting for {guest || "them"}. One attempt; any failure closes the window.</span>
              : <span>Waiting for a device. One attempt; any failure closes the window.</span>}
        </div>
      </div>
    </div>
  );
}

/** A device's request: its claimed name, the comparison code large, and what
 *  Allow would grant. `guest`: an invite's, whose Allow shares nothing. */
export function PairPendingBody({ w, guest }: { w: PairWindow; guest?: string }) {
  const now = useNow(250);
  const unprompted = useUnprompted();
  const host = useRemote().status?.host;
  if (w.phase !== "pending" || w.st?.state !== "pending") return null;
  const req = w.st.request;
  return (
    <div className="ra-request" data-testid="pair-request">
      <div className="ra-req-who">
        <b className="ra-req-name">{req.name}</b>
        <span className="ra-req-meta">{PLATFORM_LABEL[req.platform] ?? req.platform} · from {req.peer}</span>
      </div>
      <div className="ra-code-l">Comparison code</div>
      <div className="ra-code" data-testid="pair-code" aria-label={`Code ${req.code.split("").join(" ")}`}>{codeSpaced(req.code)}</div>
      {guest !== undefined ? (
        <div className="ra-line"><b>Allow only if {guest} reads you this same code from their screen.</b> The name is the device's own claim; the code is what shows it is {guest}.</div>
      ) : (
        <div className="ra-line"><b>Allow only if this code is on your device right now.</b> The name is the device's own claim; the code is what shows it is yours.</div>
      )}
      {req.invite ? (
        <div className="ra-summary" data-testid="pair-as-guest">As <b>{req.invite.name}</b>, a guest · {expiryWords(req.expiry)}</div>
      ) : (
        <>
          <div className="ra-summary">
            {capsWords(req.grant.capabilities)} on {scopeWords(req.grant.projects)} · {expiryWords(req.expiry)}
          </div>
          <RunsLabel caps={req.grant.capabilities} unprompted={unprompted} host={host} />
        </>
      )}
      {req.invite && <div className="ra-note" data-testid="pair-shares-nothing">Allow pairs this device as {req.invite.name}, a guest. It shares nothing yet.</div>}
      <div className="ra-note">Unanswered in {clock(secondsLeft(w.st.decide_by, now))}, it is denied.</div>
    </div>
  );
}

/** The closed window: how it ended, and `children` for what pairing did. */
export function PairOutcomeBody({ w, children }: { w: PairWindow; children?: React.ReactNode }) {
  if (w.phase !== "closed") return null;
  const st = w.st;
  return (
    <div className="ra-outcome" role="status" data-testid="pair-outcome">
      <span className={`ra-dot ${w.paired || (st?.state === "closed" && st.ended === "paired") ? "on" : "off"}`} aria-hidden />
      <div>
        {w.listenerStopped && !w.paired
          ? "The pairing window closed when remote access stopped listening."
          : w.replaced ? ENDED_WORDS.superseded : st?.state === "closed" ? ENDED_WORDS[st.ended] ?? st.ended : "The window is closed."}
        {w.paired && children}
      </div>
    </div>
  );
}
