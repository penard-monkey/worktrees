// Pair a device (Phase 2 §2.2–§2.4): the scope FIRST, with no timer running
// — Observe pre-selected, the project list empty so a scope is always chosen,
// 30 days unused — and only then a window: the QR (drawn here from the matrix
// `pair_begin` answers; no QR library), the same link to copy for a device
// with no camera, and the 120 s it stays open. When a device tries, its name,
// platform and address, and the six-digit comparison code large, with Allow
// and Deny. Allow is the consent act; nothing is enrolled without it.
//
// The window is serve's (`pair_status`), read every second and at once on
// every pairing step serve announces (remoteStore's `pairTick`). Closing the
// sheet while a window is open cancels it: an open window nobody is watching
// is one a stranger could use.
import { useEffect, useRef, useState } from "react";
import { invoke, notGranted } from "./ipc";
import { copyToClipboard } from "./clipboard";
import { useEscape } from "./useEscape";
import { useRemote } from "./remoteStore";
import { ExpiryEditor, RunsLabel, ScopeEditor, scopeProblem, useNow, useProjects, useUnprompted, type Scope } from "./remoteui";
import {
  capsWords, codeSpaced, DEFAULT_EXPIRY, ENDED_WORDS, expiryWords, PLATFORM_LABEL, qrPath, scopeWords, secondsLeft,
  type DeviceRecord, type Expiry, type PairBegin, type PairStatus,
} from "./devices";
import "./RemoteAccess.css";

function clock(s: number): string {
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

export function PairSheet({ onClose }: { onClose: () => void }) {
  const r = useRemote();
  const host = r.status?.host;
  const projects = useProjects();
  const unprompted = useUnprompted();
  const [scope, setScope] = useState<Scope>({ caps: ["read"], projects: [] });
  const [expiry, setExpiry] = useState<Expiry>(DEFAULT_EXPIRY);
  const [win, setWin] = useState<PairBegin | null>(null);
  const [st, setSt] = useState<PairStatus | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [paired, setPaired] = useState<DeviceRecord | null>(null);
  const [copied, setCopied] = useState(false);
  const now = useNow(250);

  // The window as serve has it, while one is ours.
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
  const phase: "scope" | "open" | "pending" | "closed" =
    !win ? "scope" : replaced ? "closed" : st?.state === "pending" ? "pending" : st?.state === "closed" ? "closed" : "open";
  const live = phase === "open" || phase === "pending";

  const liveRef = useRef(live);
  liveRef.current = live;
  // Unmounting with a window open (Settings closed under it): cancel it.
  useEffect(() => () => { if (liveRef.current) void invoke("pair_cancel").catch(() => {}); }, []);

  const close = () => {
    if (busy) return;
    if (live) void invoke("pair_cancel").catch(() => {});
    liveRef.current = false;
    onClose();
  };
  useEscape(close);

  // Unreachable from a remote window (Devices says why instead), and greyed
  // with the reason if it ever is.
  const problem = notGranted("pair_begin") ?? scopeProblem(scope);
  const begin = async () => {
    setBusy(true);
    setErr(null);
    setPaired(null);
    setSt(null);
    try {
      const b = await invoke<PairBegin>("pair_begin", { caps: scope.caps, projects: scope.projects, expiry });
      setWin(b);
      setSt({ state: "open", uri: b.uri, expires_at: b.expires_at });
    } catch (e) {
      setErr(String(e));
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
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
      void invoke<PairStatus>("pair_status").then(setSt, () => {});
    }
  };
  const cancel = async () => {
    await invoke("pair_cancel").catch((e) => setErr(String(e)));
    void invoke<PairStatus>("pair_status").then(setSt, () => {});
  };
  const again = () => {
    setWin(null);
    setSt(null);
    setErr(null);
    setPaired(null);
  };

  return (
    <div className="scrim scrim-center" onClick={close}>
      <div className="sync-modal ra-pair ra" role="dialog" aria-label="Pair a device" data-phase={phase} onClick={(e) => e.stopPropagation()}>
        <header className="sync-h">
          <b>Pair a device</b>
          {phase === "open" && st && "expires_at" in st && (
            <span className="ra-countdown" data-testid="pair-countdown">Closes in {clock(secondsLeft(st.expires_at, now))}</span>
          )}
        </header>
        <div className="sync-body">
          {phase === "scope" && (
            <>
              <div className="ra-line">
                Choose what the device may do before anything opens. The code appears next, for two minutes and one attempt.
              </div>
              <ScopeEditor value={scope} onChange={setScope} projects={projects} unprompted={unprompted} host={host} />
              <ExpiryEditor value={expiry} onChange={setExpiry} />
            </>
          )}

          {phase === "open" && win && (
            <div className="ra-window">
              <Qr size={win.qr.size} modules={win.qr.modules} />
              <div className="ra-window-txt">
                <div className="ra-line">
                  Scan this with the device, or paste the link on a device with no camera:
                  <code className="ra-cmd">worktrees remote pair '&lt;link&gt;'</code>
                </div>
                <div className="ra-link">
                  <input readOnly value={win.uri} aria-label="Pairing link" onFocus={(e) => e.currentTarget.select()} />
                  <button className="ctrl sm" onClick={() => copyToClipboard(win.uri).then(() => { setCopied(true); window.setTimeout(() => setCopied(false), 2000); }, (e) => setErr(String(e)))}>
                    {copied ? "Copied" : "Copy"}
                  </button>
                </div>
                <div className="ra-summary">
                  {capsWords(scope.caps)} on {scopeWords(scope.projects)} · {expiryWords(expiry)}
                </div>
                <RunsLabel caps={scope.caps} unprompted={unprompted} host={host} />
                <div className="ra-status" role="status">
                  <span className="ra-dot warn" aria-hidden />
                  {st?.state === "attempt" ? <span>A device is connecting from {st.peer}…</span> : <span>Waiting for a device. One attempt; any failure closes the window.</span>}
                </div>
              </div>
            </div>
          )}

          {phase === "pending" && st?.state === "pending" && (
            <div className="ra-request" data-testid="pair-request">
              <div className="ra-req-who">
                <b className="ra-req-name">{st.request.name}</b>
                <span className="ra-req-meta">{PLATFORM_LABEL[st.request.platform] ?? st.request.platform} · from {st.request.peer}</span>
              </div>
              <div className="ra-code-l">Comparison code</div>
              <div className="ra-code" data-testid="pair-code" aria-label={`Code ${st.request.code.split("").join(" ")}`}>{codeSpaced(st.request.code)}</div>
              <div className="ra-line"><b>Allow only if this code is on your device right now.</b> The name is the device's own claim; the code is what shows it is yours.</div>
              <div className="ra-summary">
                {capsWords(st.request.grant.capabilities)} on {scopeWords(st.request.grant.projects)} · {expiryWords(st.request.expiry)}
              </div>
              <RunsLabel caps={st.request.grant.capabilities} unprompted={unprompted} host={host} />
              <div className="ra-note">Unanswered in {clock(secondsLeft(st.decide_by, now))}, it is denied.</div>
            </div>
          )}

          {phase === "closed" && (
            <div className="ra-outcome" role="status" data-testid="pair-outcome">
              <span className={`ra-dot ${paired || (st?.state === "closed" && st.ended === "paired") ? "on" : "off"}`} aria-hidden />
              <div>
                {replaced ? ENDED_WORDS.superseded : st?.state === "closed" ? ENDED_WORDS[st.ended] ?? st.ended : "The window is closed."}
                {paired && <div className="ra-line">{paired.name} is paired. It is listed under Devices, where you can change or revoke it.</div>}
              </div>
            </div>
          )}

          {err && <div className="ra-err" role="alert">{err}</div>}
        </div>
        <footer className="sync-foot">
          {phase === "scope" && (
            <>
              <span className="ra-foot-why">{problem ?? ""}</span>
              <button className="ctrl" onClick={close}>Cancel</button>
              <button className="enter-btn" data-track="pair-show-code" disabled={busy || !!problem} title={problem ?? undefined} onClick={begin}>{busy ? "Opening…" : "Show the pairing code"}</button>
            </>
          )}
          {phase === "open" && <button className="ctrl" onClick={cancel}>Cancel pairing</button>}
          {phase === "pending" && (
            <>
              <button className="ctrl" disabled={busy} onClick={() => decide(false)}>Deny</button>
              <button className="enter-btn" disabled={busy} onClick={() => decide(true)}>{busy ? "Working…" : "Allow"}</button>
            </>
          )}
          {phase === "closed" && (
            <>
              <button className="ctrl" onClick={again}>Pair another</button>
              <button className="enter-btn" autoFocus onClick={close}>Done</button>
            </>
          )}
        </footer>
      </div>
    </div>
  );
}
