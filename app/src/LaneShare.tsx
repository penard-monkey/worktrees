// The owner's share controls on a lane (group chat §1, §3.3, §5, §7 "David's
// view"): *Share with…* on a running Claude or Codex session, the *Shared with
// <names>* marker while any share is attached (ended or not), *Share again*
// once the binding has ended, a join request's *Allow*, a guest's request as
// a nudge with *Share* / *Dismiss*, *Stop sharing* and taking a guest off,
// the thread beside the session, and *Use these*. Every row is the console's
// (shares.ts); a window that is not its host's console draws none of it.
import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { invoke, notGranted } from "./ipc";
import { useEscape } from "./useEscape";
import { ThreadPanel } from "./ThreadPanel";
import { laneLive, useLanes } from "./laneLive";
import {
  consoleThread, guestName, ownerForAgent, guestLive, namesOf, ownerWords, readShares, seenAsks, setThreadOpen, shareRefusal, sharesHere, sharesOf, shareWarning,
  useShares, useThreadOpen, type Guest, type Share, type ShareRequest, type SharesState,
} from "./shares";
import {
  asksForNote, authorsOf, checkRefusal, joinNames, lineProblem, NOTE_ASK, NOTE_PLACEHOLDER, PLAIN_TEXT, PREVIEW_PATH, sinceLastUse, useOutcome, useRefusal, useTheseFile, useTheseLine, useTheseName, utf8Bytes,
  LINE_MAX, type ShareCheck, type ThreadEntry, type UseSent,
} from "./thread";
import { openRemoteSettings, readStatus, remoteState, useRemote } from "./remoteStore";
import { DEFAULT_EXPIRY, expiryWords, pairingBlock, type Candidate, type DeviceRecord } from "./devices";
import { PairCountdown, PairOpenBody, PairOutcomeBody, PairPendingBody, usePairWindow, type PairWindow } from "./PairWindow";
import "./ThreadPanel.css";
import "./LaneShare.css";

const SHAREABLE = new Set(["claude", "codex"]);

/** The lane's share bar, under the agent's label. `harness` is the provider
 *  running there; `up` whether its session is live. */
export function LaneShareBar({ repo, slug, harness, up }: { repo: string; slug: string; harness: string; up: boolean }) {
  const s = useShares();
  const key = repo + "|" + slug;
  const threadOn = useThreadOpen(key);
  const [dialog, setDialog] = useState<{ mode: "create"; preset?: string[] } | { mode: "again"; share: Share } | { mode: "manage"; share: Share } | null>(null);
  if (!sharesHere() || s.shares === null) return null;
  const mine = sharesOf(s, repo, slug);
  const requests = s.requests.filter((r) => r.project === repo && r.slug === slug);
  const shareable = SHAREABLE.has(harness);

  return (
    <div className="lane-share" data-testid="lane-share">
      {mine.map((sh) => {
        const ended = sh.binding?.state !== "active";
        const asked = sh.guests.filter((g) => g.state === "asked");
        return (
          <div key={sh.id} className={"lane-share-row" + (ended ? " ended" : "")} data-share={sh.id}>
            <span className={"ls-dot" + (ended ? " ended" : "")} aria-hidden />
            <span className="ls-marker" data-testid="shared-marker">
              Shared with {joinNames(namesOf(s, sh))}
              {ended && <span className="ls-ended"> · ended</span>}
            </span>
            {asked.map((g) => (
              <span key={g.principal} className="ls-ask" data-testid="join-ask">
                {guestName(s, g.principal)} asks to join
                <button className="ctrl sm primary" onClick={() => void allow(sh.id, g.principal)}>Allow</button>
              </span>
            ))}
            {ended && (
              <button className="ctrl sm" data-track="share-again" disabled={!up} title={up ? undefined : "No agent is running in this lane"} onClick={() => setDialog({ mode: "again", share: sh })}>
                Share again
              </button>
            )}
            <button className={"ctrl sm" + (threadOn ? " on" : "")} aria-pressed={threadOn} onClick={() => setThreadOpen(key, !threadOn)}>
              Thread
            </button>
            <button className="ctrl sm" onClick={() => setDialog({ mode: "manage", share: sh })}>Sharing…</button>
          </div>
        );
      })}
      {mine.length === 0 && shareable && up && (
        <button className="ctrl sm ls-share" data-testid="share-with" onClick={() => setDialog({ mode: "create" })}>
          Share with…
        </button>
      )}
      {requests.map((r) => (
        <RequestNudge key={r.id} r={r} canShare={shareable && up && mine.length === 0} onShare={() => setDialog({ mode: "create", preset: [r.principal] })} />
      ))}
      {dialog?.mode === "create" && <ShareDialog s={s} repo={repo} slug={slug} preset={dialog.preset} onClose={() => setDialog(null)} onShared={() => setThreadOpen(key, true)} />}
      {dialog?.mode === "again" && <AgainDialog s={s} share={dialog.share} onClose={() => setDialog(null)} />}
      {dialog?.mode === "manage" && <ManageDialog s={s} share={dialog.share} onClose={() => setDialog(null)} />}
    </div>
  );
}

// *Share this session…* from outside the lane's bar (§3.1, F10): the header's
// ⋯ menu and the place's context menu ask `openShareDialog`, and the one
// `ShareDialogHost` App mounts (behind the beta) draws the dialog.
let shareAsk: { repo: string; slug: string } | null = null;
const askSubs = new Set<() => void>();
export function openShareDialog(repo: string, slug: string): void {
  shareAsk = { repo, slug };
  for (const f of [...askSubs]) f();
}
function useShareAsk() {
  return useSyncExternalStore((f) => { askSubs.add(f); return () => { askSubs.delete(f); }; }, () => shareAsk);
}
export function ShareDialogHost() {
  const ask = useShareAsk();
  const s = useShares();
  if (!ask || !sharesHere()) return null;
  return <ShareDialog key={ask.repo + "|" + ask.slug} s={s} repo={ask.repo} slug={ask.slug}
    onClose={() => { shareAsk = null; for (const f of [...askSubs]) f(); }} onShared={() => setThreadOpen(ask.repo + "|" + ask.slug, true)} />;
}

/** Whether *Share this session…* is offered for a place: this window is its
 *  host's console, the place runs a live Claude or Codex (`laneLive`, the
 *  rule the lane's bar applies), and it is not shared already (its bar's
 *  *Sharing…* manages a share). */
export function useShareStart(): (repo: string, slug: string) => boolean {
  const s = useShares();
  const lanes = useLanes();
  return (repo, slug) => sharesHere() && s.shares !== null && laneLive(lanes, repo, slug) === true && sharesOf(s, repo, slug).length === 0;
}

async function allow(share: string, principal: string): Promise<void> {
  try {
    await invoke("share_allow", { share, principal });
    seenAsks(share);
  } finally {
    void readShares();
  }
}

function RequestNudge({ r, canShare, onShare }: { r: ShareRequest; canShare: boolean; onShare: () => void }) {
  const [busy, setBusy] = useState(false);
  return (
    <div className="lane-share-row ls-request" data-testid="share-request" data-request={r.id}>
      <span className="ls-dot ask" aria-hidden />
      <span className="ls-marker">
        {r.name ?? "A guest"} asks to see this session{r.note ? <>: <q>{r.note}</q></> : "."}
      </span>
      <button className="ctrl sm primary" data-track="share-request-share" disabled={!canShare} title={canShare ? undefined : "Start the agent here first, or the lane is shared already"} onClick={onShare}>
        Share
      </button>
      <button className="ctrl sm" disabled={busy}
        onClick={() => { setBusy(true); void invoke("share_request_dismiss", { request: r.id }).finally(() => { setBusy(false); void readShares(); }); }}>
        Dismiss
      </button>
    </div>
  );
}

function Modal({ title, onClose, children, foot }: { title: string; onClose: () => void; children: React.ReactNode; foot: React.ReactNode }) {
  useEscape(onClose);
  return (
    <div className="scrim scrim-center" onClick={onClose}>
      <div className="sync-modal ls-modal" role="dialog" aria-label={title} onClick={(e) => e.stopPropagation()}>
        <header className="sync-h"><b>{title}</b></header>
        <div className="sync-body">{children}</div>
        <footer className="sync-foot">{foot}</footer>
      </div>
    </div>
  );
}

/** Two guest names that are one person's name to a reader: the host refuses
 *  a new guest whose name another LIVE guest has, in any case (F7,
 *  `pairing.rs` `Invite::from_value`), and the dialog says so before asking.
 *  A removed guest's name re-admits that guest; the dialog says that too. */
export function sameGuestName(a: string, b: string): boolean {
  return a.trim().toLowerCase() === b.trim().toLowerCase();
}

/** The guest a typed name means, as the host's `Invite::from_value` reads
 *  it, the exact name first (so "Bret" beside an older "bret" offers Bret's
 *  own devices): `clash`, a LIVE guest, whose name is taken; else
 *  `returning`, a removed one (no device left, `live` false), whom an invite
 *  by the name re-admits as the same person. */
export function nameTwin<G extends { name: string }>(guests: readonly G[], name: string, live: (g: G) => boolean): { clash: G | null; returning: G | null } {
  const twin = (of: G[]) => (name.trim() ? of.find((g) => g.name === name.trim()) ?? of.find((g) => sameGuestName(g.name, name)) ?? null : null);
  const clash = twin(guests.filter(live));
  return { clash, returning: clash ? null : twin(guests.filter((g) => !live(g))) };
}

function when(at: number | null | undefined): string {
  if (!at) return "never";
  return new Date(at * 1000).toLocaleDateString([], { month: "short", day: "numeric", year: "numeric" });
}

/** "2 devices · paired Nov 12, 2026": what tells two guests apart (F7). */
export function guestMeta(g: Guest): string {
  const n = g.devices.filter((d) => !d.revoked_at).length;
  return `${n} ${n === 1 ? "device" : "devices"} · paired ${when(g.created_at)}`;
}

/** The address the reach block preselects (§3.1, S6): the first overlay
 *  candidate (Tailscale reaches a guest anywhere on the tailnet), else the
 *  first LAN one; link-local never, since a cable is rarely the answer. */
export function reachDefault(list: readonly Candidate[]): Candidate | null {
  const usable = list.filter((c) => c.class !== "link-local");
  return usable.find((c) => c.mode === "overlay") ?? usable.find((c) => c.mode === "lan") ?? usable[0] ?? null;
}

function reachWords(c: Candidate): string {
  if (c.mode === "overlay") return c.class === "cgnat" ? "Tailscale or a VPN tunnel: works wherever both computers are on it" : "an overlay network";
  return "this network only (home or office)";
}

type Invitee = { id: string | null; name: string };
type Step = "who" | "pair" | "share";

/** *Share…* (group chat start design A, §3.1): **Who** — the people paired
 *  already, or *Someone new* with a name; **Pair** — for someone new, the
 *  invite window (`PairWindow`, the one implementation Settings' sheet uses
 *  too), with remote access turned on inline when it is off; **Share** — v3's
 *  warning and *Share with <name>*. Allow (pairing) and Share (reading) stay
 *  two screens and two presses: `PairStep` never shares, and `ShareStep` is
 *  the one place `share_create` is called (groupchat-start-check.mjs). With
 *  a guest already paired it is today's two acts: tick, Share. Closing the
 *  dialog cancels a window still open. */
export function ShareDialog({ s, repo, slug, preset, onClose, onShared }: { s: SharesState; repo: string; slug: string; preset?: string[]; onClose: () => void; onShared: () => void }) {
  const live = s.guests.filter(guestLive);
  const [step, setStep] = useState<Step>("who");
  const [someoneNew, setSomeoneNew] = useState(live.length === 0 && !preset?.length);
  const [name, setName] = useState("");
  const [picked, setPicked] = useState<Set<string>>(() => new Set(preset ?? (live.length === 1 ? [live[0].id] : [])));
  const [invitee, setInvitee] = useState<Invitee | null>(null);
  const [paired, setPaired] = useState<{ principal: string; device: string; name: string } | null>(null);
  const w = usePairWindow();
  // The guest list as it is now: a guest paired a minute ago is offered.
  useEffect(() => {
    void readShares();
  }, []);
  // Someone new appears once guests are read, if there are none.
  useEffect(() => { if (live.length === 0 && !preset?.length) setSomeoneNew(true); }, [live.length]); // eslint-disable-line react-hooks/exhaustive-deps
  const close = () => {
    w.release();
    onClose();
  };
  // The guest the name means: the exact one first, so "Bret" beside an older
  // "bret" offers Bret's own devices (both were on the test VMs, 2026-11-14).
  const { clash, returning } = someoneNew ? nameTwin(s.guests, name, guestLive) : { clash: null, returning: null };
  const invite = (who: Invitee) => {
    setInvitee(who);
    setStep("pair");
  };
  const steps = someoneNew || step !== "who" ? ["Who", "Pair", "Share"] : null;
  const stepAt = step === "who" ? 0 : step === "pair" ? 1 : 2;

  return (
    <ShareModal title={`Share ${slug}`} steps={steps} step={stepAt} onClose={close} countdown={step === "pair" ? <PairCountdown w={w} /> : null}>
      {step === "who" && (
        <WhoStep s={s} live={live} picked={picked} setPicked={setPicked} someoneNew={someoneNew} setSomeoneNew={setSomeoneNew}
          name={name} setName={setName} clash={clash} returning={returning} onInvite={invite} onClose={close}
          share={!someoneNew ? <ShareStep s={s} repo={repo} slug={slug} picked={picked} onClose={close} onShared={onShared} /> : null} />
      )}
      {step === "pair" && invitee && (
        <PairStep w={w} invitee={invitee} onClose={close}
          onPaired={(rec) => {
            setPicked(new Set([rec.principal]));
            setPaired({ principal: rec.principal, device: rec.name, name: invitee.name });
            void readShares();
            setStep("share");
          }} />
      )}
      {step === "share" && (
        <ShareStep s={s} repo={repo} slug={slug} picked={picked} setPicked={setPicked} paired={paired} onClose={close} onShared={onShared} />
      )}
    </ShareModal>
  );
}

/** The dialog's frame: its steps named in the header, and the body each
 *  step draws (each step draws its own footer, as `.sync-foot`). */
function ShareModal({ title, steps, step, countdown, onClose, children }: {
  title: string; steps: string[] | null; step: number; countdown: React.ReactNode; onClose: () => void; children: React.ReactNode;
}) {
  useEscape(onClose);
  return (
    <div className="scrim scrim-center" onClick={onClose}>
      <div className="sync-modal ls-modal ra" role="dialog" aria-label={title} data-testid="share-dialog" data-step={steps ? steps[step].toLowerCase() : "who"} onClick={(e) => e.stopPropagation()}>
        <header className="sync-h">
          <b>{title}</b>
          {steps && <span className="ls-steps" aria-label={`Step ${step + 1} of ${steps.length}`}>{steps.map((x, i) => <span key={x} className={i === step ? "on" : ""}>{i + 1}. {x}</span>)}</span>}
          {countdown}
        </header>
        {children}
      </div>
    </div>
  );
}

function Body({ children }: { children: React.ReactNode }) {
  return <div className="sync-body">{children}</div>;
}
function Foot({ children }: { children: React.ReactNode }) {
  return <footer className="sync-foot">{children}</footer>;
}

/** Step 1, who: the people paired already (each with their devices and when,
 *  so two of one name are told apart, F7), or *Someone new*. */
function WhoStep({ s, live, picked, setPicked, someoneNew, setSomeoneNew, name, setName, clash, returning, onInvite, onClose, share }: {
  s: SharesState; live: Guest[]; picked: Set<string>; setPicked: (p: Set<string>) => void;
  someoneNew: boolean; setSomeoneNew: (on: boolean) => void; name: string; setName: (n: string) => void;
  clash: Guest | null; returning: Guest | null; onInvite: (who: Invitee) => void; onClose: () => void; share: React.ReactNode;
}) {
  const problem = notGranted("pair_begin") ?? (!name.trim() ? "Type their name first." : clash ? `A guest named ${clash.name} exists.` : null);
  const pick = (
    <div className="ls-pick" role="group" aria-label="Guests">
      {live.length === 0 && <div className="ra-note">{s.shares === null ? "Reading who is paired…" : "Nobody is paired with this computer yet."}</div>}
      {live.map((g) => (
        <label key={g.id} className="tier-toggle setting-check" data-guest={g.id}>
          <input type="checkbox" checked={!someoneNew && picked.has(g.id)}
            onChange={(e) => { const n = new Set(picked); if (e.currentTarget.checked) n.add(g.id); else n.delete(g.id); setPicked(n); setSomeoneNew(false); }} />
          {g.name} <span className="ra-note ls-guest-meta">{guestMeta(g)}</span>
        </label>
      ))}
      <label className="tier-toggle setting-check ls-new">
        <input type="checkbox" data-testid="share-someone-new" checked={someoneNew} onChange={(e) => setSomeoneNew(e.currentTarget.checked)} />
        ＋ Someone new
      </label>
    </div>
  );
  if (!someoneNew) {
    return <>
      <Body>
        <div className="ra-line">Who should read this session?</div>
        {pick}
      </Body>
      {share}
    </>;
  }
  return <>
    <Body>
      <div className="ra-line">Who should read this session?</div>
      {pick}
      <label className="ra-field">
        <span className="ra-field-l">Their name, as you and the thread will see it</span>
        <input autoFocus value={name} maxLength={64} placeholder="Bret" aria-label="Their name" data-testid="share-new-name" onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter" && !problem) onInvite({ id: null, name: name.trim() }); }} />
      </label>
      {clash ? (
        <div className="ra-ask" role="note" data-testid="share-name-taken">
          <span className="ra-dot warn" aria-hidden />
          <div>
            <div>A guest named <b>{clash.name}</b> exists ({guestMeta(clash)}). Pair another device for them, or use another name.</div>
            <button className="ra-linkbtn" data-testid="share-pair-another" onClick={() => onInvite({ id: clash.id, name: clash.name })}>Pair another device for {clash.name}</button>
          </div>
        </div>
      ) : returning ? (
        <div className="ra-note" data-testid="share-name-returning">
          {returning.name} was a guest here before and was removed. Pairing them again makes them {returning.name} again: the threads show their old
          posts and new ones as one person. They are on no share until you choose one.
        </div>
      ) : (
        <div className="ra-note">They pair their Mac with yours once, with a link you send them and a code you compare. Then you choose what they read.</div>
      )}
    </Body>
    <Foot>
      <span className="ra-foot-why">{problem ?? ""}</span>
      <button className="ctrl" onClick={onClose}>Cancel</button>
      <button className="enter-btn" data-testid="share-invite" disabled={!!problem} title={problem ?? undefined} onClick={() => onInvite({ id: null, name: name.trim() })}>
        Invite {name.trim() || "them"}
      </button>
    </Foot>
  </>;
}

/** Step 2, pair: reach (remote access, inline, only while it is off), then
 *  the invite window, the code and Allow. Allow pairs and shares nothing:
 *  this step never calls `share_create`. */
function PairStep({ w, invitee, onClose, onPaired }: {
  w: PairWindow; invitee: Invitee; onClose: () => void; onPaired: (rec: DeviceRecord) => void;
}) {
  const r = useRemote();
  const off = r.status?.state === "off";
  const problem = off ? null : pairingBlock(r.status, false, r.statusErr);
  const args = { invite: invitee.id ? { guest: invitee.id } : { name: invitee.name }, expiry: DEFAULT_EXPIRY };
  const [cands, setCands] = useState<Candidate[] | null>(null);
  const [chosen, setChosen] = useState<string | null>(null);
  const [turning, setTurning] = useState(false);
  const started = useRef(false);
  const key = (c: Candidate) => `${c.address}%${c.iface}`;

  // Remote access listening already: the window opens at once (Invite was
  // the press). Off: nothing opens until the reach block's own press.
  useEffect(() => {
    if (started.current || w.phase !== "idle" || r.status?.state !== "listening" || turning) return;
    started.current = true;
    void w.begin(args);
  }, [r.status?.state, turning]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    if (!off || cands) return;
    let alive = true;
    invoke<Candidate[]>("remote_candidates").then(
      (l) => { if (!alive) return; setCands(l); const d = reachDefault(l); setChosen((c) => c ?? (d ? key(d) : null)); },
      (e) => alive && w.setErr(String(e)),
    );
    return () => { alive = false; };
  }, [off]); // eslint-disable-line react-hooks/exhaustive-deps

  const pick = cands?.find((c) => key(c) === chosen) ?? null;
  /** S6: turned on by this press only, on the one address named above it. */
  const turnOn = async () => {
    if (!pick) return;
    setTurning(true);
    w.setErr(null);
    try {
      await invoke("remote_enable", { address: pick.address, iface: pick.iface });
      for (let i = 0; i < 40; i++) {
        await readStatus();
        if (remoteState().status?.state === "listening") break;
        await new Promise((res) => window.setTimeout(res, 250));
      }
      if (remoteState().status?.state !== "listening") throw new Error("remote access did not start listening: open Settings → Remote to see why");
      started.current = true;
      await w.begin(args);
    } catch (e) {
      w.setErr(String(e));
    } finally {
      setTurning(false);
    }
  };
  const allow = async () => {
    const rec = await w.decide(true);
    if (rec) onPaired(rec);
  };
  const usable = cands?.filter((c) => c.class !== "link-local") ?? [];

  return <>
    <Body>
      {problem && problem.kind !== "loading" && (
        <div className="ra-ask" role="alert" data-testid="pair-unavailable">
          <span className="ra-dot warn" aria-hidden />
          <div>
            <div>{problem.message}</div>
            <button className="ra-linkbtn" onClick={() => { onClose(); openRemoteSettings("remote"); }}>Open Remote access</button>
          </div>
        </div>
      )}
      {w.phase === "idle" && off && (
        <div className="ls-reach" data-testid="share-reach">
          <div className="ra-line"><b>{invitee.name}'s Mac has to reach this one.</b> Remote access is off. Turning it on listens on one private address, and only a key paired with this computer gets in.</div>
          {!cands && <div className="ra-note">Reading this computer's addresses…</div>}
          {cands && usable.length === 0 && <div className="ra-note">This computer holds no private address remote access may use. Join a network (or Tailscale), then try again.</div>}
          {usable.length > 0 && (
            <select aria-label="Address" data-testid="share-reach-address" value={chosen ?? ""} onChange={(e) => setChosen(e.currentTarget.value)}>
              {usable.map((c) => <option key={key(c)} value={key(c)}>{c.address} · {reachWords(c)}</option>)}
            </select>
          )}
          {pick?.warning && <div className="ra-cand-warn">{pick.warning}</div>}
          <div className="ra-note">Change it or turn it off any time in Settings → Remote. Quitting worktrees turns it off.</div>
        </div>
      )}
      {w.phase === "idle" && !off && !problem && <div className="ra-note">Opening the pairing window…</div>}
      <PairOpenBody w={w} guest={invitee.name} summary={`A guest device for ${invitee.name} · ${expiryWords(DEFAULT_EXPIRY)}`} />
      <PairPendingBody w={w} guest={invitee.name} />
      <PairOutcomeBody w={w} />
      {w.err && <div className="ra-err" role="alert" data-testid="share-pair-error">{w.err}</div>}
    </Body>
    <Foot>
      {w.phase === "idle" && off && (
        <>
          <button className="ctrl" onClick={onClose}>Cancel</button>
          <button className="enter-btn" data-testid="share-reach-on" disabled={turning || !pick || !!notGranted("remote_enable")} onClick={() => void turnOn()}>
            {turning ? "Turning on…" : "Turn on and show the link"}
          </button>
        </>
      )}
      {w.phase === "idle" && !off && (
        <>
          <button className="ctrl" onClick={onClose}>Cancel</button>
          {w.err && !problem && <button className="enter-btn" onClick={() => void w.begin(args)}>Try again</button>}
        </>
      )}
      {w.phase === "open" && <button className="ctrl" data-testid="share-pair-cancel" onClick={() => void w.cancel()}>Cancel pairing</button>}
      {w.phase === "pending" && (
        <>
          <button className="ctrl" disabled={w.busy} onClick={() => void w.decide(false)}>Deny</button>
          <button className="enter-btn" data-testid="share-pair-allow" disabled={w.busy} onClick={() => void allow()}>{w.busy ? "Working…" : "Allow"}</button>
        </>
      )}
      {w.phase === "closed" && (
        <>
          <button className="ctrl" onClick={onClose}>Close</button>
          <button className="enter-btn" data-testid="share-pair-again" onClick={() => { started.current = false; w.reset(); }}>Open another window</button>
        </>
      )}
    </Foot>
  </>;
}

/** Step 3, share — and Who's own footer when the guests are paired already:
 *  v3's warning, alone, and the press that shares. The one `share_create`. */
function ShareStep({ s, repo, slug, picked, setPicked, paired, onClose, onShared }: {
  s: SharesState; repo: string; slug: string; picked: Set<string>; setPicked?: (p: Set<string>) => void;
  paired?: { principal: string; device: string; name: string } | null; onClose: () => void; onShared: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const live = s.guests.filter(guestLive);
  const nameOf = (id: string) => s.guests.find((g) => g.id === id)?.name ?? (paired?.principal === id ? paired.name : id);
  const names = [...picked].map(nameOf);
  const go = async () => {
    setBusy(true);
    setErr(null);
    try {
      await invoke("share_create", { repo, slug, guests: [...picked] });
      onShared();
      onClose();
    } catch (e) {
      setErr(shareRefusal(String(e)));
    } finally {
      setBusy(false);
      void readShares();
    }
  };
  const rows = paired && !live.some((g) => g.id === paired.principal) ? [...live, { id: paired.principal, name: paired.name, created_at: 0, devices: [] } as Guest] : live;
  return <>
    {paired && (
      <Body>
        <div className="ra-status" data-testid="share-paired"><span className="ra-dot on" aria-hidden />{paired.device} is paired as {paired.name}, a guest.</div>
        {setPicked && (
          <div className="ls-pick" role="group" aria-label="Guests">
            {rows.map((g) => (
              <label key={g.id} className="tier-toggle setting-check">
                <input type="checkbox" checked={picked.has(g.id)}
                  onChange={(e) => { const n = new Set(picked); if (e.currentTarget.checked) n.add(g.id); else n.delete(g.id); setPicked(n); }} />
                {g.name} <span className="ra-note ls-guest-meta">{g.id === paired.principal ? "just paired" : guestMeta(g)}</span>
              </label>
            ))}
          </div>
        )}
      </Body>
    )}
    <div className="sync-body ls-share-body">
      {names.length > 0 && <div className="ls-warn" data-testid="share-warning" role="note">{shareWarning(names)}</div>}
      <div className="ra-note">They ask to join when they open it, and you let them in. What they write sits in a thread beside the session; the agent sees none of it unless you send it.</div>
      {err && <div className="ra-err" role="alert" data-testid="share-error">{err}</div>}
    </div>
    <Foot>
      <button className="ctrl" onClick={onClose}>{paired ? "Not now" : "Cancel"}</button>
      <button className="enter-btn" data-testid="share-confirm" disabled={busy || picked.size === 0} onClick={go}>
        {busy ? "Sharing…" : names.length === 1 ? `Share with ${names[0]}` : "Share"}
      </button>
    </Foot>
  </>;
}

/** *Share again*: bind the lane's new session, the share's guests kept. */
function AgainDialog({ s, share, onClose }: { s: SharesState; share: Share; onClose: () => void }) {
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const go = async () => {
    setBusy(true);
    setErr(null);
    try {
      await invoke("share_again", { share: share.id });
      onClose();
    } catch (e) {
      setErr(shareRefusal(String(e)));
    } finally {
      setBusy(false);
      void readShares();
    }
  };
  return (
    <Modal title={`Share ${share.slug} again`} onClose={onClose} foot={<>
      <button className="ctrl" onClick={onClose}>Cancel</button>
      <button className="enter-btn" disabled={busy} onClick={go}>{busy ? "Sharing…" : "Share again"}</button>
    </>}>
      <div className="ra-line">The session this lane shared is not the one running here now. Share again binds the session running here now, with the same guests.</div>
      <div className="ls-warn" role="note">{shareWarning(namesOf(s, share))}</div>
      <div className="ra-note">The ended session stays readable beside it, as it was when it ended.</div>
      {err && <div className="ra-err" role="alert">{err}</div>}
    </Modal>
  );
}

/** Who is on the share, letting a guest in or taking one off, Stop sharing. */
function ManageDialog({ s, share: first, onClose }: { s: SharesState; share: Share; onClose: () => void }) {
  const share = (s.shares ?? []).find((x) => x.id === first.id) ?? first;
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [stopping, setStopping] = useState(false);
  const act = async (f: () => Promise<unknown>, close = false) => {
    setBusy(true);
    setErr(null);
    try {
      await f();
      if (close) onClose();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
      void readShares();
    }
  };
  const STATE = { invited: "not opened yet", asked: "asks to join", allowed: "let in" } as const;
  return (
    <Modal title={`Sharing ${share.slug}`} onClose={onClose} foot={<>
      {stopping ? (
        <>
          <span className="ra-foot-why">Their access ends now, and the thread is deleted. What they read can't be recalled.</span>
          <button className="ctrl" onClick={() => setStopping(false)}>Keep sharing</button>
          <button className="enter-btn danger" data-testid="stop-confirm" disabled={busy} onClick={() => act(() => invoke("share_stop", { share: share.id }), true)}>Stop sharing</button>
        </>
      ) : (
        <>
          <button className="ctrl danger" data-testid="stop-sharing" onClick={() => setStopping(true)}>Stop sharing…</button>
          <button className="enter-btn" onClick={onClose}>Done</button>
        </>
      )}
    </>}>
      <div className="ra-line">
        {share.binding?.state === "active"
          ? `Shared since ${new Date(share.binding.bound_at * 1000).toLocaleString()}. Everything this session prints is readable to the guests let in.`
          : "The session ended: guests read it as it was, with no new turns. Share again on the lane binds the session running now."}
        {share.ended.length > 0 && ` ${share.ended.length} earlier ${share.ended.length === 1 ? "session stays" : "sessions stay"} readable too.`}
      </div>
      <ul className="ls-guests">
        {share.guests.map((g) => (
          <li key={g.principal}>
            <b>{guestName(s, g.principal)}</b> <span className="ra-note">{STATE[g.state]}</span>
            <span className="ls-guest-acts">
              {g.state === "asked" && <button className="ctrl sm primary" disabled={busy} onClick={() => act(() => allow(share.id, g.principal))}>Allow</button>}
              <button className="ctrl sm" disabled={busy} onClick={() => act(() => invoke("share_remove_guest", { share: share.id, principal: g.principal }))}>Remove from this share</button>
            </span>
          </li>
        ))}
      </ul>
      {err && <div className="ra-err" role="alert">{err}</div>}
    </Modal>
  );
}

/** The thread beside a lane, with *Use these* — for the lane's first share. */
export function LaneThread({ repo, slug, session, up, askNote, onNeverAsk }: {
  repo: string; slug: string; session: string; up: boolean;
  /** Settings' `use_these_ask_note`, and its "Don't ask again" (App owns settings). */
  askNote: boolean; onNeverAsk: () => void;
}) {
  const s = useShares();
  const key = repo + "|" + slug;
  const on = useThreadOpen(key);
  const share = sharesOf(s, repo, slug)[0];
  const access = useMemo(() => (share ? consoleThread(share.id) : null), [share?.id]); // eslint-disable-line react-hooks/exhaustive-deps
  const [picked, setPicked] = useState<Set<number> | null>(null);
  const [again, setAgain] = useState(false);
  useEffect(() => setPicked(null), [share?.id]);
  if (!on || !share || !access) return null;
  const ended = share.binding?.state !== "active";
  const owner = ownerWords(s);
  return (
    <div className="lane-thread" data-testid="lane-thread">
      <ThreadPanel
        share={share.id}
        access={access}
        me="owner"
        ownerName={owner}
        refresh={s.tick}
        pollMs={10_000}
        title={<>Thread · {joinNames(namesOf(s, share))}</>}
        banner={ended ? "The shared session ended. Guests read it as it was; Share again on the lane to share the new one." : null}
        select={{
          picked: picked ?? new Set(),
          toggle: (seq) => setPicked((cur) => {
            const n = new Set(cur ?? []);
            if (n.has(seq)) n.delete(seq);
            else n.add(seq);
            return n;
          }),
        }}
        footer={(entries) => (
          <UseThese entries={entries} picked={picked} onPicked={setPicked} share={share} repo={repo} slug={slug} session={session}
            ownerName={ownerForAgent(s)} named={s.owner !== "owner"} live={!ended && up} askNote={askNote} onNeverAsk={onNeverAsk}
            onShareAgain={() => setAgain(true)} />
        )}
        onClose={() => setThreadOpen(key, false)}
      />
      {again && <AgainDialog s={s} share={share} onClose={() => setAgain(false)} />}
    </div>
  );
}

/** §5's *Use these*: everything since the last use preselected (shown
 *  checked once opened), deselect, a note, the exact file and line previewed,
 *  then `agent_attach` + `agent_send` as the console, and `share_used`
 *  records it once the send is confirmed. An unknown outcome is never
 *  retried by software. Before anything is attached, `share_check` must say
 *  the lane's live session IS the bound one: a Codex reopened after a reboot
 *  names no session until its first turn, so the binding still reads active
 *  while the lane runs an unshared session (the test VMs, 2026-11-14). */
export function UseThese({ entries, picked, onPicked, share, repo, slug, session, ownerName, named, live, askNote, onNeverAsk, onShareAgain }: {
  entries: readonly ThreadEntry[]; picked: Set<number> | null; onPicked: (p: Set<number> | null) => void;
  share: Share; repo: string; slug: string; session: string; ownerName: string; named: boolean; live: boolean;
  /** `use_these_ask_note`: ask before a send with no note. */
  askNote: boolean; onNeverAsk: () => void;
  /** *Share again*, offered when the check finds another session live. */
  onShareAgain: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [note, setNote] = useState("");
  // The ask before a send with no note (`asksForNote`), and its "Don't ask
  // again". `noteOk` lets the send the ask answered through, once.
  const [asking, setAsking] = useState(false);
  const [never, setNever] = useState(false);
  const noteOk = useRef(false);
  const noteRef = useRef<HTMLInputElement | null>(null);
  const [busy, setBusy] = useState(false);
  const [words, setWords] = useState<{ kind: "sent" | "refused" | "unknown" | "error"; text: string; again?: boolean } | null>(null);
  const fresh = useMemo(() => sinceLastUse(entries), [entries]);
  const sel = picked ?? new Set(fresh);
  const chosen = entries.filter((e) => e.kind === "post" && sel.has(e.seq));
  const authors = authorsOf(chosen, ownerName);
  const file = chosen.length ? useTheseFile(chosen, ownerName, slug) : "";
  const line = chosen.length ? useTheseLine(chosen.length, authors, ownerName, PREVIEW_PATH.replace("<file>", useTheseName(chosen)), note) : "";
  const problem = chosen.length ? lineProblem(line, note) : null;

  const begin = () => {
    // Opening materialises the preselection, so the boxes show it.
    if (picked === null) onPicked(new Set(fresh));
    setWords(null);
    setAsking(false);
    setOpen(true);
  };

  // One send at a time, held in a ref so a second Enter in the same render
  // cannot pass either: a send is a drive-class act and never repeated by
  // software (§5). After an unconfirmed outcome only the owner's own
  // "I looked" clears the way, as on the button.
  const flight = useRef(false);
  const send = async () => {
    if (busy || flight.current || words?.kind === "unknown" || !chosen.length || problem || !live) return;
    if (asksForNote(note, askNote) && !noteOk.current) {
      setAsking(true);
      return;
    }
    noteOk.current = false;
    setAsking(false);
    flight.current = true;
    setBusy(true);
    setWords(null);
    try {
      // The proof first: nothing is attached or typed unless the session
      // live in the lane now is the one this share bound.
      const c = await invoke<ShareCheck>("share_check", { share: share.id });
      const why = checkRefusal(c);
      if (why) {
        setWords({ kind: "refused", text: why + ". Nothing was sent; the selection stays.", again: c.live !== null });
        return;
      }
      const bytes = new TextEncoder().encode(file);
      let bin = "";
      for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
      const att = await invoke<{ path: string }>("agent_attach", { repo, slug, name: useTheseName(chosen), data: btoa(bin) });
      const real = useTheseLine(chosen.length, authors, ownerName, att.path, note);
      const again = lineProblem(real, note);
      if (again) {
        setWords({ kind: "refused", text: again + " Nothing was sent." });
        return;
      }
      let r: UseSent;
      try {
        r = await invoke<UseSent>("agent_send", { repo, slug, session, text: real });
      } catch (e) {
        setWords({ kind: "refused", text: useRefusal(String(e)) });
        return;
      }
      const out = useOutcome(r);
      if (out.kind === "sent") {
        const seqs = chosen.map((e) => e.seq);
        try {
          await invoke("share_used", { share: share.id, first: Math.min(...seqs), last: Math.max(...seqs) });
        } catch (e) {
          setWords({ kind: "error", text: `Sent to the agent, but the thread could not record it: ${e}` });
          onPicked(null);
          setOpen(false);
          return;
        }
        onPicked(null);
        setNote("");
        setOpen(false);
      }
      setWords({ kind: out.kind, text: out.words });
    } catch (e) {
      setWords({ kind: "refused", text: `${e}. Nothing was sent; the selection stays.` });
    } finally {
      flight.current = false;
      setBusy(false);
    }
  };

  if (!open) {
    return (
      <div className="usethese" data-testid="use-these">
        <div className="usethese-row">
          <span className="grow">
            {fresh.length === 0 && picked === null ? "No new messages since the last use." : `${(picked ?? new Set(fresh)).size} selected for the agent.`}
          </span>
          <button className="ctrl sm" data-track="use-these-open" disabled={!live || chosen.length === 0} title={live ? undefined : "No live shared session: Share again first"} onClick={begin}>
            Use these…
          </button>
        </div>
        {words && <div className={"usethese-words" + (words.kind === "sent" ? "" : " bad")} role="status">{words.text}</div>}
        {words?.again && <ShareAgainOffer onShareAgain={onShareAgain} />}
      </div>
    );
  }
  return (
    <div className="usethese open" data-testid="use-these">
      {/* The note first, and focused: it is the instruction. The frame tells
          the agent the comments are NOT instructions, so with no note it has
          nothing to do (2026-11-14: "the plan is unchanged"). */}
      <label className="usethese-label" htmlFor={`usethese-note-${share.id}`}>What should the agent do with them?</label>
      <div className="usethese-row">
        <input id={`usethese-note-${share.id}`} ref={noteRef} className="usethese-note" data-testid="use-these-note" value={note} placeholder={NOTE_PLACEHOLDER} aria-label="What the agent should do with them"
          autoFocus {...PLAIN_TEXT}
          onChange={(e) => { setNote(e.target.value); setAsking(false); }} onKeyDown={(e) => { if (e.key === "Enter") void send(); }} />
      </div>
      {asking && (
        <div className="usethese-ask" role="alertdialog" aria-label={NOTE_ASK} data-testid="use-these-ask">
          <div className="usethese-row"><span className="grow">{NOTE_ASK}</span></div>
          <div className="usethese-row">
            <label className="usethese-never"><input type="checkbox" checked={never} onChange={(e) => setNever(e.target.checked)} data-testid="use-these-never" /> Don’t ask again</label>
            <span className="grow" />
            <button className="ctrl sm" onClick={() => { setAsking(false); noteRef.current?.focus(); }}>Write a note</button>
            <button className="ctrl sm primary" data-testid="use-these-send-anyway" disabled={busy}
              onClick={() => { if (never) onNeverAsk(); noteOk.current = true; void send(); }}>Send without a note</button>
          </div>
        </div>
      )}
      <div className="usethese-row">
        <span className="usethese-label">Use {chosen.length} {chosen.length === 1 ? "message" : "messages"} with the agent</span>
        <span className="grow">Untick a message above to leave it out.</span>
      </div>
      <div className="usethese-label">The file the agent gets</div>
      <pre className="usethese-preview" data-testid="use-these-file">{file}</pre>
      <div className="usethese-label">The line typed into the agent ({utf8Bytes(line)} of {LINE_MAX} bytes; the host gives the file its final name, so the sent line may be a few bytes longer)</div>
      <pre className="usethese-preview" data-testid="use-these-line">{line}</pre>
      {!named && <div className="ra-note">The agent is told “the owner” until you set your name in Settings → Remote → Guests.</div>}
      <div className="ra-note">The framing is not enforcement: the agent may still treat {joinNames(authors)}'s words as instructions. Read what goes.</div>
      {problem && <div className="usethese-words bad" role="alert">{problem}</div>}
      {words && <div className={"usethese-words" + (words.kind === "sent" ? "" : " bad")} role="status" data-testid="use-these-words">{words.text}</div>}
      {words?.again && <ShareAgainOffer onShareAgain={onShareAgain} />}
      <div className="usethese-row">
        <span className="grow" />
        <button className="ctrl sm" disabled={busy} onClick={() => { setOpen(false); setWords(null); }}>Cancel</button>
        <button className="ctrl sm primary" data-testid="use-these-send" disabled={busy || !live || !chosen.length || !!problem || words?.kind === "unknown"}
          title={words?.kind === "unknown" ? "Look at the session first: the last send was not confirmed" : undefined} onClick={send}>
          {busy ? "Sending…" : "Send to the agent"}
        </button>
        {words?.kind === "unknown" && <button className="ctrl sm" onClick={() => setWords(null)}>I looked: send again is allowed</button>}
      </div>
    </div>
  );
}

function ShareAgainOffer({ onShareAgain }: { onShareAgain: () => void }) {
  return (
    <div className="usethese-row">
      <span className="grow" />
      <button className="ctrl sm primary" data-testid="use-these-share-again" onClick={onShareAgain}>Share again</button>
    </div>
  );
}

/** The corner (RemoteLayer): join asks, with Allow and Later, and share
 *  requests (§2.2), with *Share…* (the share dialog for that place, the asker
 *  ticked) and *Dismiss*. A request used to show only in its lane's share
 *  bar, so it went unseen unless the owner happened to look at that lane
 *  (2026-11-14 rehearsal). One for a place with no live Claude or Codex says
 *  so (`laneLive`) instead of offering a Share the host would refuse. */
export function ShareAlerts() {
  const s = useShares();
  const lanes = useLanes();
  const [dialog, setDialog] = useState<{ repo: string; slug: string; preset: string[] } | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  if (!sharesHere() || (!s.asked.length && !s.requests.length && !dialog)) return null;
  return (
    <div className="ra-alerts ra" aria-live="polite" data-testid="share-alerts">
      {s.asked.map((a) => {
        const sh = (s.shares ?? []).find((x) => x.id === a.share);
        return (
          <div key={a.share + a.principal} className="ra-alert" role="status">
            <span className="ra-dot warn" aria-hidden />
            <span className="ra-alert-txt">{guestName(s, a.principal)} asks to join {sh?.slug ?? "a shared session"}.</span>
            <span className="ra-alert-acts">
              <button className="ctrl sm" onClick={() => void allow(a.share, a.principal)}>Allow</button>
              <button className="ra-alert-x" aria-label="Later" title="Later" onClick={() => seenAsks(a.share)}>×</button>
            </span>
          </div>
        );
      })}
      {s.requests.map((r) => {
        const w = requestWords(r, r.project === null ? null : laneLive(lanes, r.project, r.slug), sharesOf(s, r.project ?? "", r.slug).length > 0);
        return (
          <div key={r.id} className="ra-alert" role="status" data-testid="share-request-alert" data-request={r.id}>
            <span className="ra-dot warn" aria-hidden />
            <span className="ra-alert-txt">
              {r.name ?? "A guest"} asks to see {r.slug}{r.note ? <>: <q>{r.note}</q></> : "."}
              {w.why && <span className="ra-alert-why" data-testid="share-request-why"> {w.why}</span>}
            </span>
            <span className="ra-alert-acts">
              {w.share && r.project !== null && (
                <button className="ctrl sm primary" data-track="share-request-share" data-testid="share-request-share"
                  onClick={() => setDialog({ repo: r.project!, slug: r.slug, preset: [r.principal] })}>
                  Share…
                </button>
              )}
              <button className="ctrl sm" data-testid="share-request-dismiss" disabled={busy === r.id}
                onClick={() => { setBusy(r.id); void invoke("share_request_dismiss", { request: r.id }).finally(() => { setBusy(null); void readShares(); }); }}>
                Dismiss
              </button>
            </span>
          </div>
        );
      })}
      {dialog && <ShareDialog s={s} repo={dialog.repo} slug={dialog.slug} preset={dialog.preset} onClose={() => setDialog(null)}
        onShared={() => setThreadOpen(dialog.repo + "|" + dialog.slug, true)} />}
    </div>
  );
}

/** What a share request in the corner says beside its words, and whether it
 *  offers *Share…*: a place this host does not resolve, one already shared
 *  (its lane's bar manages it), one with no live session. `live` null: not
 *  known yet (no snapshot), so the Share is offered and the host decides. */
export function requestWords(r: Pick<ShareRequest, "project">, live: boolean | null, shared: boolean): { why: string | null; share: boolean } {
  if (r.project === null) return { why: "It is not a place this host shows.", share: false };
  if (shared) return { why: "It is shared already: Sharing… on its lane adds a guest.", share: false };
  if (live === false) return { why: "It has no live session: start its agent there, then share it.", share: false };
  return { why: null, share: true };
}
