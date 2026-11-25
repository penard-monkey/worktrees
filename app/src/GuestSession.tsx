// A shared session, read as a GUEST (group chat §3, §7 "Bret's session
// view"): the owner's session from its first turn, read-only, beside the
// share's thread.
//
// Read-only by construction, not by hiding: this module and everything it
// imports reach the owner's host only through the guest connection
// (guestdoor.ts), which sends the guest rows and nothing else. It draws no
// composer, Stop, prompt controls or slash menu, and imports neither chat.ts
// nor ChatPane.tsx (only their TYPES, erased at build), so no route to
// `agent_send`, `agent_answer*`, `agent_interrupt` or `agent_command` exists
// in its module graph. guestview-check.mjs walks that graph and fails on any.
//
// Reading: `shared_turns` pages BACKWARDS from the newest (`before` is the
// cursor), so the first page is the latest turns and "Earlier turns" asks for
// the page before. A poll asks for the newest page again and replaces the
// turns it covers: the host keeps an index of the transcript and reads only
// the bytes past it, so a poll never re-reads the file (§3.2). Polling runs
// while the binding is active, the connection is up and the window is seen,
// every 10 s and on every `shares:changed`. An ENDED binding is frozen: read
// once, marked ended, never polled; the thread stays open (§3.3).
//
// A live permission prompt is the owner's alone (D8): a guest sees "waiting
// for David" where an unanswered question or plan sits in the transcript.
// David's host offline: the last page stays, in memory only (§3.4, §6).
import { openUrl } from "@tauri-apps/plugin-opener";
import { useCallback, useEffect, useRef, useState } from "react";
import type { Notice, Step } from "./chat";
import { chatLinkAction } from "./chatlink";
import { closeShare, guestLinkOf, joinShare, useGuests, type GuestHostView, type GuestShare } from "./guest";
import { leavesGap, mergeNewest, mergeOlder, piecesOf, removedWords, standing, type SharedPage, type SharedTurn } from "./guestview";
import type { GuestLink } from "./guestdoor";
import { Markdown } from "./markdown";
import { ThreadPanel } from "./ThreadPanel";
import type { ThreadAccess, ThreadEntry, ThreadPage } from "./thread";
import "./ChatPane.css";
import "./GuestSession.css";

/** The chat view's busy poll (ChatPane's `BUSY_POLL_MS`, §3.2). */
export const SHARED_POLL_MS = 10_000;
/** Turns per page asked for (the host's default; at most 50). */
export const SHARED_PAGE = 20;

type CallDetail = { tool: string; input: unknown; truncated: boolean; bytes: number };

function useSharedTurns(link: GuestLink | undefined, share: string, binding: number | undefined, live: boolean, allowed: boolean, bump: number) {
  const [turns, setTurns] = useState<SharedTurn[]>([]);
  const [more, setMore] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [older, setOlder] = useState(false);
  const before = useRef(0);
  const key = `${share}|${binding ?? "now"}`;
  const keyRef = useRef(key);
  keyRef.current = key;

  useEffect(() => {
    setTurns([]); setMore(false); setError(null); setLoaded(false); before.current = 0;
  }, [key]);

  const newest = useCallback(async () => {
    if (!link || !allowed) return;
    const k = key;
    try {
      const p = await link.call<SharedPage>("shared_turns", { share, ...(binding === undefined ? {} : { binding }), limit: SHARED_PAGE });
      if (keyRef.current !== k) return;
      setTurns((cur) => {
        // The first page sets the cursor; later polls only refresh the tail,
        // unless more turns came than a page holds (`leavesGap`).
        if (!cur.length || leavesGap(cur, p.turns)) { before.current = p.next_before; setMore(p.more); return mergeNewest([], p.turns); }
        return mergeNewest(cur, p.turns);
      });
      setError(null);
      setLoaded(true);
    } catch (e) {
      if (keyRef.current === k) setError(String(e));
    }
  }, [link, allowed, share, binding, key]);

  const earlier = async () => {
    if (!link || older) return;
    const k = key;
    setOlder(true);
    try {
      const p = await link.call<SharedPage>("shared_turns", { share, ...(binding === undefined ? {} : { binding }), before: before.current, limit: SHARED_PAGE });
      if (keyRef.current !== k) return;
      before.current = p.next_before;
      setMore(p.more);
      setTurns((cur) => mergeOlder(cur, p.turns));
    } catch (e) {
      if (keyRef.current === k) setError(String(e));
    } finally {
      setOlder(false);
    }
  };

  // Once on open (an ended binding is read only then), and on every change.
  useEffect(() => { void newest(); }, [newest, bump]);
  useEffect(() => {
    if (!live) return;
    const id = window.setInterval(() => { if (document.visibilityState === "visible") void newest(); }, SHARED_POLL_MS);
    return () => window.clearInterval(id);
  }, [live, newest]);

  return { turns, more, error, loaded, older, earlier };
}

const clock = (at: number | null) => (at ? new Date(at * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }) : "");
const HARNESS: Record<string, string> = { claude: "Claude", codex: "Codex" };

/** A guest has no Files tab, and a local path names a file on the HOST's
 *  machine, so only http(s) opens; every other link is refused in words
 *  (`say`), never silently (the same rule as ChatPane's, chat.ts). */
function guestLink(href: string, say: (words: string) => void) {
  const a = chatLinkAction(href, "");
  if (a.kind === "url") openUrl(a.url).catch((e) => console.warn(`[guest] open ${href}: ${String(e)}`));
  else if (a.kind === "refuse") say(`Refused to open a ${a.scheme}: link.`);
  else if (a.kind === "path") say("That link names a file on the host’s computer, which a shared session cannot open.");
}

function noticeWords(n: Notice): string {
  switch (n.kind) {
    case "task": return `Background task ${n.status}: ${n.summary}`;
    case "peer_message": return `Message from ${n.from_name}: ${n.text}`;
    case "command": return `/${n.name}${n.args ? ` ${n.args}` : ""}`;
    case "shell": return `! ${n.command}`;
    case "interrupt": return "Interrupted";
    case "summary": return "The conversation was compacted";
  }
}

function Call(props: { link: GuestLink | undefined; share: string; binding: number | undefined; name: string; call_id?: string; summary?: string; failed?: boolean; detail?: boolean }) {
  const [open, setOpen] = useState(false);
  const [read, setRead] = useState<{ text?: string; cut?: boolean; err?: string } | null>(null);
  const fetches = !!props.detail && !!props.call_id;
  const toggle = async () => {
    const next = !open;
    setOpen(next);
    if (!next || read?.text !== undefined || !props.link || !props.call_id) return;
    setRead({});
    try {
      const c = await props.link.call<CallDetail>("shared_tool_call", { share: props.share, ...(props.binding === undefined ? {} : { binding: props.binding }), call_id: props.call_id });
      setRead({ text: typeof c.input === "string" ? c.input : JSON.stringify(c.input, null, 2), cut: c.truncated });
    } catch (e) {
      setRead({ err: String(e) });
    }
  };
  const head = (
    <span className="chat-call">
      <span className="chat-tool chat-call-name">{props.name}</span>
      {props.summary && <span className="chat-call-sum" title={props.summary}>{props.summary}</span>}
      {props.failed && <span className="chat-call-failed"><span className="chat-call-dot" aria-hidden="true" />failed</span>}
    </span>
  );
  return (
    <li className={"chat-call-row" + (props.failed ? " failed" : "")} data-testid="guest-call">
      {fetches ? <button type="button" className="chat-call-btn" aria-expanded={open} onClick={() => void toggle()}>{head}</button> : <div className="chat-call">{head}</div>}
      {open && (
        <div className="chat-call-full">
          {read?.text !== undefined && <pre className="chat-call-text">{read.text}</pre>}
          {read && read.text === undefined && !read.err && <p className="chat-call-note" role="status">Reading the call…</p>}
          {read?.cut && <p className="chat-call-note">Cut by the host.</p>}
          {read?.err && <p className="chat-call-note" role="alert">Couldn’t read this call: {read.err}</p>}
        </div>
      )}
    </li>
  );
}

/** One step a person was asked: answered, as what was asked and the answer in
 *  a line; not answered yet, as the owner's to answer (D8). */
function Asked({ st, owner }: { st: Extract<Step, { kind: "question" | "permission" | "plan" }>; owner: string }) {
  const what = st.kind === "question" ? st.questions.map((q) => q.question).join(" · ") : st.kind === "permission" ? `${st.tool}${st.summary ? `: ${st.summary}` : ""}` : "A plan";
  if (st.answered_at === undefined) {
    return <li className="chat-note guest-waiting" data-testid="guest-waiting">Waiting for {owner}: {what}</li>;
  }
  const answer = st.kind === "permission" ? (st.allowed ? "allowed" : "refused") : st.kind === "plan" ? (st.approved ? "approved" : "not approved") : "answered";
  return <li className="chat-note" data-testid="guest-asked">{what} · {owner} {answer}</li>;
}

function TurnItems({ t, agent, owner, link, share, binding, onLink }: { onLink: (href: string) => void; t: SharedTurn; agent: string; owner: string; link: GuestLink | undefined; share: string; binding: number | undefined }) {
  return (
    <>
      {piecesOf(t).map((p, n) => {
        switch (p.k) {
          case "notice": {
            // §3.1: a `!` command's output and a slash command's are shown.
            const out = p.notice.kind === "shell" ? [p.notice.output, p.notice.stderr].filter(Boolean).join("\n") : p.notice.kind === "command" ? p.notice.output ?? "" : "";
            return (
              <li key={n} className="chat-note" data-testid="guest-notice">
                {noticeWords(p.notice)}
                {out && <details className="chat-notice-details"><summary>Output</summary><pre>{out}</pre></details>}
              </li>
            );
          }
          case "user":
            return (
              <li key={n} className="chat-row chat-row-user">
                <p className="chat-name" aria-hidden="true">{owner}</p>
                <div className="chat-bubble chat-bubble-user" role="group" aria-label={owner} data-testid="guest-user">
                  <Markdown src={p.text} onLink={onLink} />
                  {p.mid && <p className="chat-bubble-note chat-midturn-caption">Sent while {agent} was working</p>}
                </div>
              </li>
            );
          case "asked":
            return <Asked key={n} st={p.st} owner={owner} />;
          case "agent":
            return (
              <li key={n} className="chat-row chat-row-agent">
                <p className="chat-name" aria-hidden="true">{agent}{p.at ? <span className="chat-at"> · {clock(p.at)}</span> : null}</p>
                <div className="chat-group" role="group" aria-label={agent} data-testid="guest-agent">
                  {p.text && <div className="chat-bubble chat-bubble-agent"><Markdown src={p.text} onLink={onLink} /></div>}
                  {p.calls.length > 0 && (
                    <ul className="chat-calls">
                      {p.calls.map((c, j) => <Call key={c.call_id ?? `${j}`} link={link} share={share} binding={binding} name={c.name} call_id={c.call_id} summary={c.summary} failed={c.failed} detail={c.detail} />)}
                    </ul>
                  )}
                </div>
              </li>
            );
        }
      })}
      {t.truncated && <li className="chat-note" data-testid="guest-cut">Shortened by the host{t.chars ? `: ${t.kept ?? "?"} of ${t.chars} characters shown` : ""}.</li>}
    </>
  );
}

function threadAccess(link: GuestLink | undefined, share: string): ThreadAccess {
  return {
    read: (after, limit) => (link ? link.call<ThreadPage>("thread_read", { share, after, ...(limit ? { limit } : {}) }) : Promise.reject("not connected")),
    post: (id, text) => (link ? link.call<ThreadEntry>("thread_post", { share, id, text }) : Promise.reject("not connected")),
  };
}

function ShareView({ hv, share, binding }: { hv: GuestHostView; share: GuestShare; binding: number | undefined }) {
  const link = guestLinkOf(hv.host.id);
  const b = binding === undefined ? share.binding : share.ended[binding] ?? null;
  const st = standing(share, b, hv.link, hv.owner);
  const allowed = share.state === "allowed" && !!b;
  const agent = HARNESS[b?.harness ?? ""] ?? "The agent";
  const { turns, more, error, loaded, older, earlier } = useSharedTurns(link, share.id, binding, st.live, allowed, hv.bump);
  const [joinErr, setJoinErr] = useState<string | null>(null);
  const [linkNote, setLinkNote] = useState<string | null>(null);
  const onLink = useCallback((href: string) => { setLinkNote(null); guestLink(href, setLinkNote); }, []);

  // §1 step 4: opening an invited share asks to join.
  useEffect(() => {
    if (share.state !== "invited" || hv.link.kind !== "up") return;
    joinShare(hv.host.id, share.id).catch((e) => setJoinErr(String(e)));
  }, [share.id, share.state, hv.host.id, hv.link.kind]);

  // The thread's access is the guest connection's, stable per share.
  const access = useRef<{ key: string; a: ThreadAccess } | null>(null);
  if (access.current?.key !== `${hv.host.id}|${share.id}` || !link) access.current = { key: `${hv.host.id}|${share.id}`, a: threadAccess(link, share.id) };

  const name = share.title || share.slug;
  const scroll = useRef<HTMLDivElement>(null);
  const atEnd = useRef(true);
  useEffect(() => {
    const el = scroll.current;
    if (el && atEnd.current) el.scrollTop = el.scrollHeight;
  }, [turns.length, turns[turns.length - 1]?.seq]);

  return (
    <div className="guest-session" data-testid="guest-session">
      <div className="guest-main">
        <header className="guest-head">
          <span className="guest-title"><span className="guest-owner">{hv.owner}’s</span> <b>{name}</b></span>
          <span className="guest-proj">{share.project}{share.title ? ` · ${share.slug}` : ""}</span>
          {b?.state === "ended" && <span className="guest-chip ended" data-testid="guest-ended">Ended</span>}
          {st.live && <span className="guest-chip live">Live</span>}
          <span className="guest-ro">Read-only: only {hv.owner} talks to the agent</span>
          {share.ended.length > 0 && (
            <span className="guest-bindings" role="group" aria-label="Sessions in this share">
              {share.binding && <button className={"ctrl sm" + (binding === undefined ? " on" : "")} onClick={() => setBinding(hv.host.id, share.id, undefined)}>Current</button>}
              {share.ended.map((_, i) => (
                <button key={i} className={"ctrl sm" + (binding === i ? " on" : "")} onClick={() => setBinding(hv.host.id, share.id, i)} data-testid="guest-binding">Earlier {i + 1}</button>
              ))}
            </span>
          )}
          <button className="ctrl sm" onClick={closeShare} title="Back to your own places">Close</button>
        </header>
        {st.words && <div className={"guest-banner" + (hv.link.kind === "offline" || hv.link.kind === "removed" ? " offline" : "")} role="status" data-testid="guest-banner">{st.words}{(hv.link.kind === "offline" || hv.link.kind === "removed") && turns.length ? ". Showing the last page read." : "."}</div>}
        {joinErr && <div className="guest-banner" role="alert">Couldn’t ask to join: {joinErr}</div>}
        <div className="chat-scroll guest-scroll" ref={scroll} onScroll={(e) => { const el = e.currentTarget; atEnd.current = el.scrollHeight - el.scrollTop - el.clientHeight < 32; }}>
          <div className="chat-col">
            {allowed && more && (
              <button className="chat-linkbtn guest-earlier" disabled={older} onClick={() => void earlier()} data-testid="guest-earlier">{older ? "Reading…" : "Earlier turns"}</button>
            )}
            {allowed && !loaded && !error && <p className="chat-note" role="status">Reading the session…</p>}
            {error && <p className="chat-note" role="alert" data-testid="guest-error">Couldn’t read the session: {error}</p>}
            {allowed && loaded && turns.length === 0 && <p className="chat-note">No messages in this session yet.</p>}
            <ol className="chat-list" data-testid="guest-list">
              {turns.map((t, i) => <TurnItems key={`${t.seq}|${t.at ?? "-"}|${i}`} t={t} agent={agent} owner={hv.owner} link={link} share={share.id} binding={binding} onLink={onLink} />)}
            </ol>
            {linkNote && <p className="chat-note" role="alert" data-testid="guest-link-note">{linkNote}</p>}
          </div>
        </div>
      </div>
      <div className="guest-thread">
        {share.state === "allowed" ? (
          <ThreadPanel
            share={share.id}
            access={access.current.a}
            me={hv.host.principal_id ?? ""}
            ownerName={hv.owner}
            refresh={(hv.threadBump[share.id] ?? 0) + hv.bump}
            pollMs={hv.link.kind === "up" ? 30_000 : 0}
            readOnly={hv.link.kind === "removed" ? `${removedWords(hv.link.by, hv.owner)}: posting is closed.` : hv.link.kind === "offline" ? `${hv.owner}'s host is offline: posting waits until it is back.` : null}
            banner={b?.state === "ended" ? "The session ended. The thread is still open." : undefined}
          />
        ) : (
          <div className="guest-thread-wait thread-note">The thread opens when {hv.owner} lets you in.</div>
        )}
      </div>
    </div>
  );
}

/** Which ended binding each open share shows; module state, per window. */
const chosenBinding = new Map<string, number | undefined>();
let bindingTick = 0;
const bindingSubs = new Set<() => void>();
function setBinding(host: string, share: string, n: number | undefined) {
  chosenBinding.set(`${host}|${share}`, n);
  bindingTick += 1;
  for (const f of [...bindingSubs]) f();
}
function useBindingChoice(host: string, share: string): number | undefined {
  const [, force] = useState(0);
  useEffect(() => {
    const f = () => force(bindingTick);
    bindingSubs.add(f);
    return () => { bindingSubs.delete(f); };
  }, []);
  return chosenBinding.get(`${host}|${share}`);
}

/** The space's content while a share is open (App.tsx mounts it in place of
 *  the place's workbench). */
export function GuestSpace() {
  const g = useGuests();
  const open = g.open;
  const hv = open ? g.hosts.find((h) => h.host.id === open.host) : undefined;
  const share = open ? hv?.shares?.find((s) => s.id === open.share) : undefined;
  const chosen = useBindingChoice(open?.host ?? "", open?.share ?? "");
  if (!open) return null;
  if (!hv || !share) {
    return (
      <div className="guest-session guest-gone" data-testid="guest-gone">
        <p className="chat-note">This share is no longer there: it was stopped, or you were removed from it.</p>
        <button className="ctrl sm" onClick={closeShare}>Back</button>
      </div>
    );
  }
  // No current binding but ended ones: the newest ended one.
  const binding = chosen ?? open.binding ?? (share.binding ? undefined : share.ended.length ? share.ended.length - 1 : undefined);
  return <ShareView key={`${hv.host.id}|${share.id}`} hv={hv} share={share} binding={binding} />;
}
