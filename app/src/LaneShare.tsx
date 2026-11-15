// The owner's share controls on a lane (group chat §1, §3.3, §5, §7 "David's
// view"): *Share with…* on a running Claude or Codex session, the *Shared with
// <names>* marker while any share is attached (ended or not), *Share again*
// once the binding has ended, a join request's *Allow*, a guest's request as
// a nudge with *Share* / *Dismiss*, *Stop sharing* and taking a guest off,
// the thread beside the session, and *Use these*. Every row is the console's
// (shares.ts); a window that is not its host's console draws none of it.
import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "./ipc";
import { useEscape } from "./useEscape";
import { ThreadPanel } from "./ThreadPanel";
import { laneLive, useLanes } from "./laneLive";
import {
  consoleThread, guestName, ownerForAgent, guestLive, namesOf, ownerWords, readShares, seenAsks, setThreadOpen, shareRefusal, sharesHere, sharesOf, shareWarning,
  useShares, useThreadOpen, type Share, type ShareRequest, type SharesState,
} from "./shares";
import {
  asksForNote, authorsOf, checkRefusal, joinNames, lineProblem, NOTE_ASK, NOTE_PLACEHOLDER, PLAIN_TEXT, PREVIEW_PATH, sinceLastUse, useOutcome, useRefusal, useTheseFile, useTheseLine, useTheseName, utf8Bytes,
  LINE_MAX, type ShareCheck, type ThreadEntry, type UseSent,
} from "./thread";
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

/** *Share with…*: pick guests, read §1's warning, share. */
function ShareDialog({ s, repo, slug, preset, onClose, onShared }: { s: SharesState; repo: string; slug: string; preset?: string[]; onClose: () => void; onShared: () => void }) {
  const live = s.guests.filter(guestLive);
  const [picked, setPicked] = useState<Set<string>>(() => new Set(preset ?? (live.length === 1 ? [live[0].id] : [])));
  // The guest list as it is now: a guest paired a minute ago is offered.
  useEffect(() => {
    void readShares();
  }, []);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const names = live.filter((g) => picked.has(g.id)).map((g) => g.name);
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
  return (
    <Modal title={`Share ${slug}`} onClose={onClose} foot={<>
      <button className="ctrl" onClick={onClose}>Cancel</button>
      <button className="enter-btn" data-testid="share-confirm" disabled={busy || picked.size === 0} onClick={go}>{busy ? "Sharing…" : "Share"}</button>
    </>}>
      {live.length === 0 ? (
        <div className="ra-line">No guests yet. Invite one in Settings → Remote → Guests.</div>
      ) : (
        <div className="ls-pick" role="group" aria-label="Guests">
          {live.map((g) => (
            <label key={g.id} className="tier-toggle setting-check">
              <input type="checkbox" checked={picked.has(g.id)}
                onChange={(e) => { const n = new Set(picked); if (e.currentTarget.checked) n.add(g.id); else n.delete(g.id); setPicked(n); }} />
              {g.name}
            </label>
          ))}
        </div>
      )}
      {names.length > 0 && <div className="ls-warn" data-testid="share-warning" role="note">{shareWarning(names)}</div>}
      <div className="ra-note">They ask to join when they open it, and you let them in. What they write sits in a thread beside the session; the agent sees none of it unless you send it.</div>
      {err && <div className="ra-err" role="alert" data-testid="share-error">{err}</div>}
    </Modal>
  );
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
