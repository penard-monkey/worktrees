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
import { useState } from "react";
import { notGranted } from "./ipc";
import { useEscape } from "./useEscape";
import { useRemote } from "./remoteStore";
import { ExpiryEditor, ScopeEditor, scopeProblem, useProjects, useUnprompted, type Scope } from "./remoteui";
import { DEFAULT_EXPIRY, expiryWords, capsWords, pairingBlock, PRESETS, scopeWords, type Expiry, type Preset } from "./devices";
import { PairCountdown, PairOpenBody, PairOutcomeBody, PairPendingBody, usePairWindow } from "./PairWindow";
import "./RemoteAccess.css";

/** `startScope`: the capabilities the sheet starts on — the policy preset's
 *  suggestion (settings-reorg §5.2: Operate under Autonomous), Observe else. */
/** `guest`: the sheet is Settings → Remote access's *Invite a guest* (group
 *  chat §1): no scope to choose (a guest's grant is `guest` and nothing
 *  else, the host's own rule), a name instead — a new guest's, typed here,
 *  or `{id, name}` for a guest the host has, pairing another device. */
export function PairSheet({ onClose, onRemote, startScope = "observe", guest }: {
  onClose: () => void; onRemote: () => void; startScope?: Preset; guest?: { id: string | null; name: string };
}) {
  const inviting = !!guest;
  const [guestName, setGuestName] = useState(guest?.name ?? "");
  const r = useRemote();
  const host = r.status?.host;
  const projects = useProjects();
  const unprompted = useUnprompted();
  const [scope, setScope] = useState<Scope>({ caps: [...PRESETS[startScope]], projects: [] });
  const [expiry, setExpiry] = useState<Expiry>(DEFAULT_EXPIRY);
  const w = usePairWindow();
  const remoteProblem = pairingBlock(r.status, false, r.statusErr);
  const { phase, busy } = w;

  const close = () => {
    if (busy) return;
    w.release();
    onClose();
  };
  const openRemote = () => { close(); onRemote(); };
  useEscape(close);

  // Unreachable from a remote window (Devices says why instead), and greyed
  // with the reason if it ever is.
  const problem = notGranted("pair_begin") ?? remoteProblem?.message ?? (inviting ? (guestName.trim() ? null : "Type the guest's name first.") : scopeProblem(scope));
  const begin = () => void (inviting
    ? w.begin({ invite: guest!.id ? { guest: guest!.id } : { name: guestName.trim() }, expiry })
    : w.begin({ caps: scope.caps, projects: scope.projects, expiry }));
  const who = guest?.id ? guest.name : guestName.trim();

  return (
    <div className="scrim scrim-center" onClick={close}>
      <div className="sync-modal ra-pair ra" role="dialog" aria-label={inviting ? "Invite a guest" : "Pair a device"} data-phase={phase === "idle" ? "scope" : phase} data-invite={inviting || undefined} onClick={(e) => e.stopPropagation()}>
        <header className="sync-h">
          <b>{inviting ? (guest!.id ? `Pair another device for ${guest!.name}` : "Invite a guest") : "Pair a device"}</b>
          <PairCountdown w={w} />
        </header>
        <div className="sync-body">
          {remoteProblem && remoteProblem.kind !== "loading" && (
            <div className="ra-ask" role="alert" data-testid={remoteProblem.kind === "paused" ? "pair-paused" : "pair-unavailable"}>
              <span className="ra-dot warn" aria-hidden />
              <div>
                <div>{remoteProblem.message}</div>
                <button className="ra-linkbtn" disabled={busy} onClick={openRemote}>Open Remote access</button>
              </div>
            </div>
          )}
          {phase === "idle" && inviting && (
            <>
              <div className="ra-line">
                A guest sees only the sessions you share with them, after you let them in, and the names of your places in projects you have not
                hidden. They can write in a share's thread; nothing they write reaches an agent unless you send it.
              </div>
              {guest!.id ? (
                <div className="ra-line">This pairs another device for <b>{guest!.name}</b>; their shares follow it.</div>
              ) : (
                <label className="ra-field">
                  <span className="ra-field-l">Their name, as you and the thread will see it</span>
                  <input autoFocus value={guestName} maxLength={64} placeholder="Bret" aria-label="Guest's name" onChange={(e) => setGuestName(e.target.value)}
                    onKeyDown={(e) => { if (e.key === "Enter" && !problem && !busy) void begin(); }} />
                </label>
              )}
              <div className="ra-note" data-testid="invite-one-window">
                One pairing window at a time: opening another (here, or with another invite or device) closes this one, and its link then fails with
                “someone else used this pairing code, or its window has closed”. Send the link only when you are ready to wait for it.
              </div>
              <ExpiryEditor value={expiry} onChange={setExpiry} />
            </>
          )}
          {phase === "idle" && !inviting && (
            <>
              <div className="ra-line">
                Choose what the device may do before anything opens. The code appears next, for two minutes and one attempt.
              </div>
              <ScopeEditor value={scope} onChange={setScope} projects={projects} unprompted={unprompted} host={host} />
              <ExpiryEditor value={expiry} onChange={setExpiry} />
            </>
          )}

          <PairOpenBody w={w} guest={inviting ? who : undefined} caps={inviting ? undefined : scope.caps}
            summary={`${inviting ? `A guest device for ${who}` : `${capsWords(scope.caps)} on ${scopeWords(scope.projects)}`} · ${expiryWords(expiry)}`} />
          <PairPendingBody w={w} guest={inviting ? who : undefined} />
          <PairOutcomeBody w={w}>
            {inviting
              ? <div className="ra-line">{w.paired?.name} is paired as {who}. They are listed under Guests, and you can share a session with them from the session itself.</div>
              : <div className="ra-line">{w.paired?.name} is paired. It is listed under Devices, where you can change or revoke it.</div>}
          </PairOutcomeBody>

          {w.err && <div className="ra-err" role="alert">{w.err}</div>}
        </div>
        <footer className="sync-foot">
          {phase === "idle" && (
            <>
              <span className="ra-foot-why">{problem ?? ""}</span>
              <button className="ctrl" onClick={close}>Cancel</button>
              <button className="enter-btn" data-track="pair-show-code" disabled={busy || !!problem} title={problem ?? undefined} onClick={begin}>{busy ? "Opening…" : "Show the pairing code"}</button>
            </>
          )}
          {phase === "open" && <button className="ctrl" onClick={() => void w.cancel()}>Cancel pairing</button>}
          {phase === "pending" && (
            <>
              <button className="ctrl" disabled={busy} onClick={() => void w.decide(false)}>Deny</button>
              <button className="enter-btn" disabled={busy} onClick={() => void w.decide(true)}>{busy ? "Working…" : "Allow"}</button>
            </>
          )}
          {phase === "closed" && (
            <>
              {!inviting && <button className="ctrl" onClick={w.reset}>Pair another</button>}
              <button className="enter-btn" autoFocus onClick={close}>Done</button>
            </>
          )}
        </footer>
      </div>
    </div>
  );
}
