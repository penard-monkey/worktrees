// Settings → Hosts: which engine this window drives — this machine's own, or a
// host it is paired with — and pairing with one (Phase 2 §2.2, §4.4).
//
// Everything here is the shim's (src-tauri/src/remote.rs): the hosts are kept
// in this machine's config dir, shared with `worktrees remote`, and the page is
// told a summary of each — never its pin, never a key. Choosing reloads the
// window: the door is decided once, at boot (main.tsx), and a remote one never
// finds or starts a local serve.
//
// Pairing takes the LINK a host shows beside its pairing QR (§2.3: carried
// over Universal Clipboard, AirDrop or a password manager — a desktop has no
// camera to scan with). Its address is judged before anything is dialled; the
// six-digit code must match the host's screen before anyone presses Allow
// there.
import { useEffect, useRef, useState } from "react";
import { Channel, invoke, remoteHost } from "./ipc";
import type { HostSummary } from "./ipcremote";

type HostList = { chosen: string | null; hosts: HostSummary[]; this_name: string };
type PairEv = { dialling?: { host: string; addr: string }; code?: string };

/** "482 913": the comparison code as both screens show it. */
function spaced(code: string): string {
  return code.length === 6 ? `${code.slice(0, 3)} ${code.slice(3)}` : code;
}

function when(epoch: number): string {
  return new Date(epoch * 1000).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
}

export function HostsPanel({ onReport, onFail }: { onReport: (m: string) => void; onFail: (e: unknown) => void }) {
  const [list, setList] = useState<HostList | null>(null);
  const [listErr, setListErr] = useState<string | null>(null);
  const [link, setLink] = useState("");
  const [name, setName] = useState("");
  const [pairing, setPairing] = useState<{ host?: string; addr?: string; code?: string } | null>(null);
  const [pairErr, setPairErr] = useState<string | null>(null);
  const [armed, setArmed] = useState<string | null>(null);
  // Set on EVERY mount: StrictMode mounts, unmounts and mounts again, and a
  // flag only ever cleared would drop every answer after the first unmount.
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  const reload = () =>
    invoke<HostList>("remote_hosts")
      .then((h) => { if (mounted.current) { setList(h); setListErr(null); setName((n) => n || h.this_name); } })
      .catch((e) => { if (mounted.current) setListErr(String(e)); });
  useEffect(() => { void reload(); }, []);

  const drives = remoteHost();
  const choose = async (id: string | null) => {
    try {
      await invoke("remote_choose", { hostId: id });
      window.location.reload();
    } catch (e) { onFail(e); }
  };
  const forget = async (h: HostSummary) => {
    if (armed !== h.id) { setArmed(h.id); return; }
    setArmed(null);
    try {
      await invoke("remote_forget", { hostId: h.id });
      onReport(`Forgot ${h.name}. It still lists this machine until it is revoked there.`);
      await reload();
    } catch (e) { onFail(e); }
  };
  const pair = async () => {
    setPairErr(null);
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
      onReport(`Paired with ${h.name}. Choose it above to drive it from this window.`);
      await reload();
    } catch (e) {
      if (!mounted.current) return;
      setPairing(null);
      setPairErr(String(e));
    }
  };

  const chosen = list?.chosen ?? null;
  return (
    <>
      <section className="setting" data-testid="hosts-engine">
        <label>This window drives</label>
        <div className="hint">
          {drives
            ? <>This window is driving <b>{drives}</b>. Paths there are not paths here, so revealing, opening and choosing files are greyed.</>
            : <>This window is driving this machine's own engine.</>}
          {" "}Choosing another reloads the window.
        </div>
        {listErr && <pre className="update-log">{listErr}</pre>}
        <div className="hosts-list">
          <div className="hosts-row">
            <span className="hosts-name">This machine</span>
            <span className="hosts-meta">its own worktrees serve</span>
            <button className="ctrl sm" disabled={chosen === null} onClick={() => void choose(null)}>
              {chosen === null ? "Driving" : "Drive this machine"}
            </button>
          </div>
          {(list?.hosts ?? []).map((h) => (
            <div className="hosts-row" key={h.id} data-testid="hosts-row">
              <span className="hosts-name">{h.name}</span>
              <span className="hosts-meta">{h.addr} · paired {when(h.paired_at)}</span>
              <button className="ctrl sm" disabled={chosen === h.id} onClick={() => void choose(h.id)}>
                {chosen === h.id ? "Driving" : "Drive"}
              </button>
              <button className={"ctrl sm danger" + (armed === h.id ? " armed" : "")} onClick={() => void forget(h)}>
                {armed === h.id ? "Really forget?" : "Forget"}
              </button>
            </div>
          ))}
        </div>
      </section>
      <section className="setting" data-testid="hosts-pair">
        <label>Pair with a host</label>
        <div className="hint">
          On the host, open a pairing window (Settings → Devices → Pair a device, or <code>worktrees serve pair</code>) and
          copy the <code>worktrees-pair:v1?…</code> link it shows beside the QR — Universal Clipboard, AirDrop or a password
          manager. Only a private address (a LAN, Tailscale, a cable) is accepted, and it is checked before anything is dialled.
        </div>
        <input type="text" value={link} aria-label="Pairing link" placeholder="worktrees-pair:v1?addr=…"
          disabled={pairing !== null} onChange={(e) => setLink(e.currentTarget.value)} />
        <label className="sub">This machine's name, as the host will show it</label>
        <input type="text" value={name} aria-label="This machine's name" maxLength={64}
          disabled={pairing !== null} onChange={(e) => setName(e.currentTarget.value)} />
        <div className="ver-actions">
          <button className="ctrl sm" data-testid="hosts-pair-go" disabled={pairing !== null || !link.trim() || !name.trim()} onClick={() => void pair()}>
            Pair
          </button>
        </div>
        {pairing && !pairing.code && (
          <div className="hint" role="status">{pairing.host ? `Connecting to ${pairing.host} at ${pairing.addr}…` : "Reading the link…"}</div>
        )}
        {pairing?.code && (
          <div className="hosts-code" role="status">
            <div className="hosts-code-digits" data-testid="hosts-code">{spaced(pairing.code)}</div>
            <div className="hint">
              Check that {pairing.host ?? "the host"} shows this same code, then press Allow there. If it shows another code,
              press Deny: something else answered this link.
            </div>
          </div>
        )}
        {pairErr && <pre className="update-log" data-testid="hosts-pair-err">{pairErr}</pre>}
      </section>
    </>
  );
}
