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
import {
  consoleThread, guestName, ownerForAgent, guestLive, namesOf, ownerWords, readShares, seenAsks, setThreadOpen, shareRefusal, sharesHere, sharesOf, shareWarning,
  useShares, useThreadOpen, type Share, type ShareRequest, type SharesState,
} from "./shares";
import {
  authorsOf, joinNames, lineProblem, PREVIEW_PATH, sinceLastUse, useOutcome, useRefusal, useTheseFile, useTheseLine, useTheseName, utf8Bytes,
  LINE_MAX, type ThreadEntry, type UseSent,
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
      <div className="ra-line">The session this lane shared has ended. Share again binds the session running here now, with the same guests.</div>
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
export function LaneThread({ repo, slug, session, up }: { repo: string; slug: string; session: string; up: boolean }) {
  const s = useShares();
  const key = repo + "|" + slug;
  const on = useThreadOpen(key);
  const share = sharesOf(s, repo, slug)[0];
  const access = useMemo(() => (share ? consoleThread(share.id) : null), [share?.id]); // eslint-disable-line react-hooks/exhaustive-deps
  const [picked, setPicked] = useState<Set<number> | null>(null);
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
            ownerName={ownerForAgent(s)} named={s.owner !== "owner"} live={!ended && up} />
        )}
        onClose={() => setThreadOpen(key, false)}
      />
    </div>
  );
}

/** §5's *Use these*: everything since the last use preselected (shown
 *  checked once opened), deselect, a note, the exact file and line previewed,
 *  then `agent_attach` + `agent_send` as the console, and `share_used`
 *  records it once the send is confirmed. An unknown outcome is never
 *  retried by software. */
export function UseThese({ entries, picked, onPicked, share, repo, slug, session, ownerName, named, live }: {
  entries: readonly ThreadEntry[]; picked: Set<number> | null; onPicked: (p: Set<number> | null) => void;
  share: Share; repo: string; slug: string; session: string; ownerName: string; named: boolean; live: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [words, setWords] = useState<{ kind: "sent" | "refused" | "unknown" | "error"; text: string } | null>(null);
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
    setOpen(true);
  };

  // One send at a time, held in a ref so a second Enter in the same render
  // cannot pass either: a send is a drive-class act and never repeated by
  // software (§5). After an unconfirmed outcome only the owner's own
  // "I looked" clears the way, as on the button.
  const flight = useRef(false);
  const send = async () => {
    if (busy || flight.current || words?.kind === "unknown" || !chosen.length || problem || !live) return;
    flight.current = true;
    setBusy(true);
    setWords(null);
    try {
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
      </div>
    );
  }
  return (
    <div className="usethese open" data-testid="use-these">
      <div className="usethese-row">
        <span className="usethese-label">Use {chosen.length} {chosen.length === 1 ? "message" : "messages"} with the agent</span>
        <span className="grow">Untick a message above to leave it out.</span>
      </div>
      <div className="usethese-row">
        <input className="usethese-note" value={note} placeholder="A note for the agent (optional)" aria-label="A note for the agent"
          onChange={(e) => setNote(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") void send(); }} />
      </div>
      <div className="usethese-label">The file the agent gets</div>
      <pre className="usethese-preview" data-testid="use-these-file">{file}</pre>
      <div className="usethese-label">The line typed into the agent ({utf8Bytes(line)} of {LINE_MAX} bytes; the host gives the file its final name, so the sent line may be a few bytes longer)</div>
      <pre className="usethese-preview" data-testid="use-these-line">{line}</pre>
      {!named && <div className="ra-note">The agent is told “the owner” until you set your name in Settings → Remote → Guests.</div>}
      <div className="ra-note">The framing is not enforcement: the agent may still treat {joinNames(authors)}'s words as instructions. Read what goes.</div>
      {problem && <div className="usethese-words bad" role="alert">{problem}</div>}
      {words && <div className={"usethese-words" + (words.kind === "sent" ? "" : " bad")} role="status" data-testid="use-these-words">{words.text}</div>}
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

/** Join requests, for the corner (RemoteLayer): a guest asks to join a share,
 *  with Allow and Later. */
export function ShareAlerts() {
  const s = useShares();
  if (!sharesHere() || !s.asked.length) return null;
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
    </div>
  );
}
