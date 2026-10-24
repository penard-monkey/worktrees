// Settings → Remote access (Phase 2 §4.1, §4.7, §9.3): whether other devices
// can reach this computer's engine, on which address, and the one-click way
// back to off. Every act is a console row (`remote_enable`, `remote_disable`,
// `remote_advertise`, `remote_candidates`); the state shown is `remote_status`
// as serve last answered it (remoteStore.ts), never a guess.
import { useEffect, useId, useState } from "react";
import { invoke, notGranted } from "./ipc";
import { copyToClipboard } from "./clipboard";
import { ConfirmDialog } from "./remoteui";
import { GrantNote } from "./GrantNote";
import { notConsole, readStatus, useRemote } from "./remoteStore";
import type { Candidate, RemoteStatus } from "./devices";
import "./RemoteAccess.css";

/** What each address class is, for the picker. */
const CLASS_WORDS: Record<Candidate["class"], string> = {
  rfc1918: "home or office network",
  cgnat: "shared address space",
  ula: "unique-local IPv6",
  "link-local": "link-local, a cable between two computers",
};

function candidateWords(c: Candidate): string {
  if (c.mode === "overlay") return c.class === "cgnat" ? "overlay (Tailscale or a VPN tunnel)" : `overlay, ${CLASS_WORDS[c.class]}`;
  return CLASS_WORDS[c.class];
}

/** The headline and its dot, by state. */
function headline(st: RemoteStatus): { dot: "off" | "on" | "warn" | "err"; text: string } {
  switch (st.state) {
    case "off":
      return { dot: "off", text: "Off. No other device can reach this computer." };
    case "unreadable":
      return { dot: "err", text: `Off: its settings file cannot be read (${st.error ?? "unknown error"}). Turning it off removes the file.` };
    case "listening":
      return { dot: "on", text: `On, listening on ${st.at ?? `${st.address}:${st.port}`} (${st.iface}, ${st.mode === "overlay" ? "overlay" : "LAN"} mode).` };
    case "paused":
      return { dot: "warn", text: `Paused: ${st.paused ?? "this computer's address changed"}.` };
    case "error":
      return { dot: "err", text: `On, but not listening: ${st.error ?? "unknown error"}.` };
    case "starting":
      return { dot: "warn", text: `On, starting on ${st.address} (${st.iface}).` };
  }
}

function advertWords(st: RemoteStatus): string {
  if (st.advertise === false) return "Not announced on the network: switched off below.";
  if (st.advert?.state === "registered") return `Devices on this network can find it as ${st.advert.host}.`;
  if (st.advert?.state === "not-advertised") return `Not announced on the network: ${st.advert.why ?? "not registered"}. A device that moves follows with the address code.`;
  return "Not announced on the network.";
}

export function RemoteAccessSection({ onReport }: { onReport: (msg: string) => void }) {
  const r = useRemote();
  const st = r.status;
  const [picking, setPicking] = useState(false);
  const [confirmOff, setConfirmOff] = useState(false);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [copied, setCopied] = useState("");

  // A window that drives a paired host is not that host's console: it does
  // not ask (ipc.ts CONSOLE_ROWS), and serve's `console-only` says the same.
  if (notGranted("remote_status") || notConsole(r)) {
    return <GrantNote cmd="remote_status" label="Remote access" focus="remote-access" refused />;
  }
  if (!st) {
    return (
      <section className="setting ra" data-focus="remote-access">
        <label>Remote access</label>
        <div className="ra-note">{r.statusErr ? `Remote access could not be read: ${r.statusErr}` : "Reading remote access…"}</div>
      </section>
    );
  }

  const on = st.state !== "off" && st.state !== "unreadable";
  const h = headline(st);
  const enable = async (address: string, iface?: string) => {
    setBusy(true);
    setErr(null);
    try {
      await invoke("remote_enable", { address, iface: iface ?? null });
      setPicking(false);
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
      void readStatus();
    }
  };
  const disable = async () => {
    setBusy(true);
    setErr(null);
    try {
      await invoke("remote_disable");
      setConfirmOff(false);
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
      void readStatus();
    }
  };
  const copy = (what: string, text: string) =>
    copyToClipboard(text).then(() => { setCopied(what); window.setTimeout(() => setCopied(""), 2000); }, (e) => onReport(String(e)));

  return (
    <>
      <section className="setting ra" data-focus="remote-access">
        <label>Remote access</label>
        <div className="ra-status" role="status" data-state={st.state}>
          <span className={`ra-dot ${h.dot}`} aria-hidden />
          <span>{h.text}</span>
        </div>
        {st.question && (
          <div className="ra-ask" data-testid="remote-question">
            <span className="ra-dot warn" aria-hidden />
            <div>
              <div>{st.question.text}</div>
              <div className="ver-actions">
                {st.question.use_address && (
                  <button className="ctrl sm" disabled={busy} onClick={() => enable(st.question!.use_address)}>Use this address</button>
                )}
                <button className="ctrl sm" disabled={busy} onClick={() => setConfirmOff(true)}>Keep it off</button>
              </div>
            </div>
          </div>
        )}
        {st.warning && (
          <div className="ra-ask"><span className="ra-dot warn" aria-hidden /><div>{st.warning}</div></div>
        )}
        {st.state === "listening" && <div className="ra-line">{advertWords(st)}</div>}
        {on && st.owner === "app" && (
          <div className="ra-line">This app started its engine, so quitting Worktrees turns remote access off. Run <code>worktrees serve</code> yourself to keep it on without the app.</div>
        )}
        {err && <div className="ra-err" role="alert">{err}</div>}
        <div className="ver-actions">
          {!on && !picking && <button className="ctrl sm" onClick={() => { setErr(null); setPicking(true); }}>Turn on…</button>}
          {on && !picking && <button className="ctrl sm" onClick={() => { setErr(null); setPicking(true); }}>Use another address…</button>}
          {on && <button className="ctrl sm ra-danger" onClick={() => { setErr(null); setConfirmOff(true); }}>Turn off</button>}
          {st.state === "unreadable" && <button className="ctrl sm ra-danger" onClick={() => setConfirmOff(true)}>Remove the file</button>}
        </div>
        {picking && <AddressPicker busy={busy} current={on ? st.address : undefined} onPick={(c) => enable(c.address, c.iface)} onCancel={() => setPicking(false)} />}
        {st.state === "listening" && st.pin && (
          <details className="ra-ident">
            <summary>This computer's identity</summary>
            <div className="ra-kv"><span>Pin</span><code>{st.pin}</code></div>
            {st.address_code && (
              <div className="ra-kv">
                <span>Address code</span>
                <code>{st.address_code}</code>
                <button className="ctrl sm" onClick={() => copy("code", st.address_code!)}>{copied === "code" ? "Copied" : "Copy"}</button>
              </div>
            )}
            <div className="ra-note">A paired device that lost this computer after its address moved takes the address code instead of pairing again. It carries no secret.</div>
          </details>
        )}
      </section>
      <AdvertiseSection st={st} onFail={setErr} />
      {confirmOff && (
        <ConfirmDialog title="Turn off remote access?" confirm="Turn off" danger busy={busy} error={err}
          onConfirm={disable} onClose={() => setConfirmOff(false)}>
          <div className="ra-line">
            Every device's connection closes now, along with the listener and any pairing window. Devices stay paired, and
            reconnect when you turn remote access on again.
          </div>
        </ConfirmDialog>
      )}
    </>
  );
}

/** §4.7's switch: on until the host turns it off, remembered, and honoured
 *  before remote access is ever on (off then means nothing is announced, not
 *  for an instant). */
function AdvertiseSection({ st, onFail }: { st: RemoteStatus; onFail: (e: string) => void }) {
  const [busy, setBusy] = useState(false);
  const on = st.advertise !== false;
  const flip = async (next: boolean) => {
    setBusy(true);
    try {
      await invoke("remote_advertise", { on: next });
    } catch (e) {
      onFail(String(e));
    } finally {
      setBusy(false);
      void readStatus();
    }
  };
  return (
    <section className="setting ra" data-focus="remote-advertise">
      <label>Announce on the network</label>
      <label className="tier-toggle setting-check">
        <input type="checkbox" checked={on} disabled={busy || st.state === "unreadable"} onChange={(e) => flip(e.currentTarget.checked)} />
        Let devices on this network find this computer by itself
      </label>
      <div className="ra-note">
        In LAN mode, a paired device whose host moved to a new address finds it again over mDNS, as {st.advert?.state === "registered" ? st.advert.host : "wt-<id>.local"}.
        The announcement names no person, key or project. Overlay addresses and shared address space are never announced. Off, a
        moved device needs the address code.
      </div>
    </section>
  );
}

/** The addresses `remote_enable` would take now (`remote_candidates`), in
 *  serve's order; link-local folded away, since a cable is rarely the
 *  answer. */
function AddressPicker({ busy, current, onPick, onCancel }: {
  busy: boolean;
  current?: string;
  onPick: (c: Candidate) => void;
  onCancel: () => void;
}) {
  const [list, setList] = useState<Candidate[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [chosen, setChosen] = useState<string | null>(null);
  const [showLL, setShowLL] = useState(false);
  const group = useId();
  const key = (c: Candidate) => `${c.address}%${c.iface}`;
  useEffect(() => {
    let live = true;
    invoke<Candidate[]>("remote_candidates").then(
      (l) => {
        if (!live) return;
        setList(l);
        const first = l.find((c) => c.address !== current && c.class !== "link-local");
        setChosen((cur) => cur ?? (first ? key(first) : null));
      },
      (e) => live && setErr(String(e)),
    );
    return () => { live = false; };
    // Read once per opening: the picker is a question about NOW.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const ll = list?.filter((c) => c.class === "link-local") ?? [];
  const rest = list?.filter((c) => c.class !== "link-local") ?? [];
  const pick = list?.find((c) => key(c) === chosen) ?? null;
  const row = (c: Candidate) => (
    <label key={key(c)} className="ra-cand" data-address={c.address}>
      <input type="radio" name={group} checked={pick === c} onChange={() => setChosen(key(c))} />
      <span className="ra-cand-txt">
        <span className="ra-cand-addr">{c.address}</span>
        <span className="ra-cand-meta">{c.iface} · {candidateWords(c)}{c.address === current ? " · in use now" : ""}</span>
        {c.warning && <span className="ra-cand-warn">{c.warning}</span>}
      </span>
    </label>
  );
  return (
    <div className="ra-picker" data-testid="address-picker">
      <div className="ra-field-l">Listen on</div>
      {err && <div className="ra-err" role="alert">{err}</div>}
      {!list && !err && <div className="ra-note">Reading this computer's addresses…</div>}
      {list && list.length === 0 && (
        <div className="ra-note">This computer holds no private address remote access may use. Join a network (or Tailscale), then try again.</div>
      )}
      {rest.map(row)}
      {ll.length > 0 && !showLL && (
        <button className="ra-linkbtn" onClick={() => setShowLL(true)}>Show link-local addresses ({ll.length})</button>
      )}
      {showLL && ll.map(row)}
      <div className="ra-note">
        Only private addresses are offered: a home or office network, Tailscale, or a cable. Devices connect with a key they get by pairing,
        never with this address alone. In LAN mode a DHCP reservation on your router keeps the address from moving.
      </div>
      <div className="ver-actions">
        <button className="enter-btn sm" disabled={busy || !pick} onClick={() => pick && onPick(pick)}>{busy ? "Turning on…" : "Turn on"}</button>
        <button className="ctrl sm" disabled={busy} onClick={onCancel}>Cancel</button>
      </div>
    </div>
  );
}
