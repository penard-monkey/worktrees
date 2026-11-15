// *Join with a link* (group chat start design A, S4, F5): the guest's way in.
// A host's *Share…* (or *Invite a guest*) shows a `worktrees-pair:v1?…` link;
// the guest pastes it here, reads the six-digit code to the host, and the host
// presses Allow. One form, two homes: Home's *Join with a link…* dialog and
// Settings → Remote → Join with a link (`JoinHostSection`). The call is the
// shim's `remote_pair`, the same one "This window drives" used (HostsPanel),
// with the link judged there before anything is dialled.
//
// With Group chat's beta off the join is refused in words before any call
// (S5, F6): a guest's pairing would succeed and show nothing, since the guest
// connections start only with the beta on (main.tsx).
import { useEffect, useRef, useState } from "react";
import { Channel, invoke } from "./ipc";
import type { HostSummary } from "./ipcremote";
import { refreshGuestHosts } from "./guest";
import { useEscape } from "./useEscape";

type PairEv = { dialling?: { host: string; addr: string }; code?: string };

/** What a join with the beta off is told, before anything is dialled. */
export const JOIN_BETA_OFF = "Turn on Group chat in Settings → General → Beta features first. Without it nothing a host shares with you can show here.";

/** "482 913": the comparison code as both screens show it. */
function spaced(code: string): string {
  return code.length === 6 ? `${code.slice(0, 3)} ${code.slice(3)}` : code;
}

/** What a finished join says. */
export function joinedWords(h: Pick<HostSummary, "name" | "guest">): string {
  return h.guest
    ? `Joined ${h.name} as a guest. When they share a session with you, it shows in the sidebar.`
    : `Paired with ${h.name} to drive it: choose it in Settings → Remote → This window drives.`;
}

/** The paste, the name, Join, and the code to read to the host. */
export function JoinForm({ groupChat, onJoined, onOpenBeta, autoFocus = false }: {
  /** `groupChatBeta(settings)`. */
  groupChat: boolean;
  onJoined: (h: HostSummary) => void;
  /** Where the beta-off refusal sends the person, when the host can. */
  onOpenBeta?: () => void;
  autoFocus?: boolean;
}) {
  const [link, setLink] = useState("");
  const [name, setName] = useState("");
  const [pairing, setPairing] = useState<{ host?: string; addr?: string; code?: string } | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [refused, setRefused] = useState(false);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);
  useEffect(() => {
    void invoke<{ this_name: string }>("remote_hosts").then((h) => { if (mounted.current) setName((n) => n || h.this_name); }, () => {});
  }, []);
  useEffect(() => { if (groupChat) setRefused(false); }, [groupChat]);

  const join = async () => {
    setErr(null);
    // S5: refused in words, before any call.
    if (!groupChat) { setRefused(true); return; }
    setPairing({});
    const ch = new Channel<PairEv>((ev) => {
      if (!mounted.current) return;
      if (ev.dialling) setPairing({ host: ev.dialling.host, addr: ev.dialling.addr });
      if (ev.code) setPairing((p) => ({ ...p, code: ev.code }));
    });
    try {
      const h = await invoke<HostSummary>("remote_pair", { link: link.trim(), name: name.trim(), onEvent: ch });
      if (!mounted.current) return;
      setPairing(null);
      setLink("");
      void refreshGuestHosts();
      onJoined(h);
    } catch (e) {
      if (!mounted.current) return;
      setPairing(null);
      setErr(String(e));
    }
  };
  const busy = pairing !== null;
  return (
    <div className="setting join-form" data-testid="join-form">
      <label className="sub">The link they sent you</label>
      <input type="text" value={link} aria-label="Pairing link" placeholder="worktrees-pair:v1?addr=…" autoFocus={autoFocus} data-testid="join-link"
        disabled={busy} onChange={(e) => { setLink(e.currentTarget.value); setRefused(false); }}
        onKeyDown={(e) => { if (e.key === "Enter" && link.trim() && name.trim() && !busy) void join(); }} />
      <label className="sub">This computer's name, as they will see it</label>
      <input type="text" value={name} aria-label="This computer's name" maxLength={64} disabled={busy} onChange={(e) => setName(e.currentTarget.value)} />
      <div className="hint">
        Joining pairs this computer as a guest: you read only what they share, after they let you in, and their computer is never driven from
        here. Only a private address (your network, Tailscale) is accepted, and it is checked before anything is dialled.
      </div>
      <div className="ver-actions">
        <button className="ctrl sm primary" data-testid="join-go" disabled={busy || !link.trim() || !name.trim()} onClick={() => void join()}>Join</button>
      </div>
      {refused && (
        <div className="hint join-refused" role="alert" data-testid="join-beta-off">
          {JOIN_BETA_OFF}
          {onOpenBeta && <> <button className="ra-linkbtn" onClick={onOpenBeta}>Open Beta features</button></>}
        </div>
      )}
      {pairing && !pairing.code && (
        <div className="hint" role="status">{pairing.host ? `Connecting to ${pairing.host} at ${pairing.addr}…` : "Reading the link…"}</div>
      )}
      {pairing?.code && (
        <div className="hosts-code" role="status">
          <div className="hosts-code-digits" data-testid="join-code">{spaced(pairing.code)}</div>
          <div className="hint">
            Read this code to whoever sent the link. They press Allow when {pairing.host ?? "their computer"} shows the same; if it shows
            another code, they press Deny: something else answered this link.
          </div>
        </div>
      )}
      {err && <pre className="update-log" data-testid="join-err">{err}</pre>}
    </div>
  );
}

/** Settings → Remote → Join with a link. */
export function JoinHostSection({ groupChat, onReport, onOpenBeta }: { groupChat: boolean; onReport: (m: string) => void; onOpenBeta: () => void }) {
  return (
    <section className="setting" data-testid="join-host">
      <label>Join with a link</label>
      <div className="hint">
        Someone sent you a <code>worktrees-pair:v1?…</code> link? They made it with <b>Share…</b> on a session (or <b>Invite a guest</b> in their
        Settings → Remote → Guests). Paste it here to read what they share with you.
      </div>
      <JoinForm groupChat={groupChat} onOpenBeta={onOpenBeta} onJoined={(h) => onReport(joinedWords(h))} />
    </section>
  );
}

/** Home's *Join with a link…* (shown only with the beta on). */
export function JoinDialog({ groupChat, onClose }: { groupChat: boolean; onClose: () => void }) {
  const [done, setDone] = useState<HostSummary | null>(null);
  useEscape(onClose);
  return (
    <div className="scrim scrim-center" onClick={onClose}>
      <div className="sync-modal ra" role="dialog" aria-label="Join with a link" data-testid="join-dialog" onClick={(e) => e.stopPropagation()}>
        <header className="sync-h"><b>Join with a link</b></header>
        <div className="sync-body">
          {done
            ? <div className="ra-line" role="status" data-testid="join-done">{joinedWords(done)}</div>
            : <JoinForm groupChat={groupChat} autoFocus onJoined={setDone} />}
        </div>
        <footer className="sync-foot">
          <button className={done ? "enter-btn" : "ctrl"} onClick={onClose}>{done ? "Done" : "Cancel"}</button>
        </footer>
      </div>
    </div>
  );
}
