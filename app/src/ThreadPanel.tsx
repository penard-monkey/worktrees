// The thread beside a shared session (group chat §4): one reusable panel,
// mounted by the owner beside the session's chat (PR D) and by a guest over
// the guest connection (PR E). Its data access is INJECTED (`access`: a
// `{read, post}` pair, thread.ts' `ThreadAccess`), so this file imports no
// door: the owner passes the engine door's `thread_read`/`thread_post`, a
// guest its own socket's. Nothing here reaches an agent; the owner's *Use
// these* is a separate component given the entries through `footer`.
//
// Reading: `read(after)` from the newest seq it holds, on mount, every
// `pollMs` while mounted, and whenever `refresh` changes (the owner bumps it
// on `shares:changed`). Posting: each draft takes a client id once, kept
// until the post lands, so Retry sends the SAME id and the host answers with
// the original entry rather than a second one.

import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { eventWords, mergeEntries, newPostId, postProblem, whoOf, type ThreadAccess, type ThreadEntry } from "./thread";
import "./ThreadPanel.css";

export type ThreadPanelProps = {
  /** The share's id: the panel re-reads from scratch when it changes. */
  share: string;
  access: ThreadAccess;
  /** The viewer's principal id (`"owner"` for the console): their own posts
   *  sit on the right. */
  me: string;
  /** The owner's display name, for events and the owner's posts. */
  ownerName: string;
  /** Bump to re-read now (a `shares:changed` for this share). */
  refresh?: number;
  /** Re-read this often while mounted, ms. 0: only on `refresh`. */
  pollMs?: number;
  /** Posting is off with these words in place of the field (a guest not
   *  let in yet, a host offline). */
  readOnly?: string | null;
  /** A line above the entries: "Ended", "waiting for David", … */
  banner?: ReactNode;
  /** The owner's *Use these*: a checkbox per post, owned by the caller. */
  select?: { picked: ReadonlySet<number>; toggle(seq: number): void };
  /** Under the entries, above the field, given the entries as they stand. */
  footer?: (entries: readonly ThreadEntry[]) => ReactNode;
  /** A heading; default "Thread". */
  title?: ReactNode;
  onClose?: () => void;
};

type Pending = { id: string; text: string; state: "posting" | "failed"; error?: string };

const PAGE = 200;

function clock(at: number): string {
  const d = new Date(at * 1000);
  const today = new Date();
  const hm = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  return d.toDateString() === today.toDateString() ? hm : `${d.toLocaleDateString([], { month: "short", day: "numeric" })} ${hm}`;
}

/** The thread's entries and its post, for one share through `access`. */
export function useThread(share: string, access: ThreadAccess, refresh: number, pollMs: number) {
  const [entries, setEntries] = useState<ThreadEntry[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);
  const lastRef = useRef(0);
  const flight = useRef<Promise<void> | null>(null);
  const accessRef = useRef(access);
  accessRef.current = access;
  const shareRef = useRef(share);
  shareRef.current = share;

  // A new share starts from nothing.
  useEffect(() => {
    lastRef.current = 0;
    flight.current = null;
    setEntries([]);
    setLoaded(false);
    setError(null);
  }, [share]);

  const load = useCallback((): Promise<void> => {
    if (flight.current) return flight.current;
    const at = share;
    const run = (async () => {
      try {
        for (let i = 0; i < 50; i++) {
          const page = await accessRef.current.read(lastRef.current, PAGE);
          if (at !== shareRef.current) return;
          if (page.entries.length) {
            lastRef.current = Math.max(lastRef.current, ...page.entries.map((e) => e.seq));
            setEntries((cur) => mergeEntries(cur, page.entries));
          }
          if (!page.more) break;
        }
        setError(null);
      } catch (e) {
        if (at === shareRef.current) setError(String(e));
      } finally {
        if (at === shareRef.current) setLoaded(true);
      }
    })();
    flight.current = run.finally(() => {
      flight.current = null;
    });
    return flight.current;
  }, [share]);

  useEffect(() => {
    void load();
  }, [load, refresh]);

  useEffect(() => {
    if (!pollMs) return;
    const id = window.setInterval(() => {
      if (document.visibilityState !== "hidden") void load();
    }, pollMs);
    return () => window.clearInterval(id);
  }, [load, pollMs]);

  /** Post `text` under `id`; the landed entry joins the list. */
  const post = useCallback(
    async (id: string, text: string): Promise<ThreadEntry> => {
      const e = await accessRef.current.post(id, text);
      lastRef.current = Math.max(lastRef.current, e.seq);
      setEntries((cur) => mergeEntries(cur, [e]));
      return e;
    },
    [],
  );

  return { entries, error, loaded, load, post };
}

export function ThreadPanel(p: ThreadPanelProps) {
  const { entries, error, loaded, post } = useThread(p.share, p.access, p.refresh ?? 0, p.pollMs ?? 10_000);
  const [draft, setDraft] = useState("");
  // The draft's client id: made once per draft, so a retry is the same post.
  const draftId = useRef<string | null>(null);
  const [pending, setPending] = useState<Pending[]>([]);
  const scroller = useRef<HTMLDivElement>(null);
  const stick = useRef(true);

  useEffect(() => {
    setPending([]);
    setDraft("");
    draftId.current = null;
  }, [p.share]);

  // Follow the newest entry while the reader is at the bottom.
  useEffect(() => {
    const el = scroller.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [entries.length, pending.length]);

  const problem = draft.trim() ? postProblem(draft) : null;

  const send = async (id: string, text: string) => {
    setPending((cur) => [...cur.filter((x) => x.id !== id), { id, text, state: "posting" }]);
    try {
      await post(id, text);
      setPending((cur) => cur.filter((x) => x.id !== id));
    } catch (e) {
      setPending((cur) => cur.map((x) => (x.id === id ? { ...x, state: "failed", error: String(e) } : x)));
    }
  };

  const submit = () => {
    if (!draft.trim() || problem || p.readOnly) return;
    const id = draftId.current ?? newPostId();
    const text = draft.replace(/\r\n?/g, "\n");
    draftId.current = null;
    setDraft("");
    void send(id, text);
  };

  return (
    <section className="thread" aria-label="Thread">
      <header className="thread-head">
        <span className="thread-title">{p.title ?? "Thread"}</span>
        <span className="thread-sub">The agent sees none of this.</span>
        {p.onClose && (
          <button className="thread-x" aria-label="Close the thread" title="Close the thread" onClick={p.onClose}>
            ×
          </button>
        )}
      </header>
      {p.banner && <div className="thread-banner">{p.banner}</div>}
      <div
        className="thread-list"
        ref={scroller}
        onScroll={(ev) => {
          const el = ev.currentTarget;
          stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
        }}
      >
        {!loaded && <div className="thread-note">Reading the thread…</div>}
        {loaded && entries.length === 0 && pending.length === 0 && !error && <div className="thread-note">No messages yet.</div>}
        {entries.map((e) => {
          const ev = eventWords(e, p.ownerName);
          if (ev !== null) {
            return (
              <div key={e.seq} className={`thread-event k-${e.kind}`} data-seq={e.seq}>
                <span>{ev}</span>
                <time>{clock(e.at)}</time>
              </div>
            );
          }
          const mine = e.principal === p.me;
          const picked = p.select?.picked.has(e.seq) ?? false;
          return (
            <div key={e.seq} className={"thread-post" + (mine ? " mine" : "") + (picked ? " picked" : "")} data-seq={e.seq}>
              {p.select && (
                <input
                  type="checkbox"
                  className="thread-pick"
                  checked={picked}
                  aria-label={`Use message ${e.seq}`}
                  onChange={() => p.select!.toggle(e.seq)}
                />
              )}
              <div className="thread-bubble">
                <div className="thread-meta">
                  <span className="thread-who">{whoOf(e, p.ownerName)}</span>
                  {e.device_name && <span className="thread-dev">{e.device_name}</span>}
                  <span className="thread-seq">#{e.seq}</span>
                  <time>{clock(e.at)}</time>
                </div>
                <div className="thread-text">{e.text}</div>
              </div>
            </div>
          );
        })}
        {pending.map((x) => (
          <div key={x.id} className={"thread-post mine pending" + (x.state === "failed" ? " failed" : "")}>
            <div className="thread-bubble">
              <div className="thread-meta">
                <span className="thread-who">{x.state === "posting" ? "Posting…" : "Not posted"}</span>
              </div>
              <div className="thread-text">{x.text}</div>
              {x.state === "failed" && (
                <div className="thread-fail">
                  <span>{x.error}</span>
                  <button className="ctrl sm" onClick={() => void send(x.id, x.text)}>
                    Retry
                  </button>
                  <button
                    className="ctrl sm"
                    onClick={() => {
                      setPending((cur) => cur.filter((y) => y.id !== x.id));
                      setDraft(x.text);
                      draftId.current = x.id;
                    }}
                  >
                    Edit
                  </button>
                </div>
              )}
            </div>
          </div>
        ))}
      </div>
      {error && <div className="thread-error" role="alert">{error}</div>}
      {p.footer?.(entries)}
      {p.readOnly ? (
        <div className="thread-note thread-ro">{p.readOnly}</div>
      ) : (
        <form
          className="thread-compose"
          onSubmit={(ev) => {
            ev.preventDefault();
            submit();
          }}
        >
          <textarea
            className="thread-field"
            rows={2}
            value={draft}
            placeholder="Write in the thread"
            aria-label="Write in the thread"
            onChange={(ev) => {
              setDraft(ev.target.value);
              if (!ev.target.value) draftId.current = null;
              else if (!draftId.current) draftId.current = newPostId();
            }}
            onKeyDown={(ev) => {
              if (ev.key === "Enter" && !ev.shiftKey && !ev.nativeEvent.isComposing) {
                ev.preventDefault();
                submit();
              }
            }}
          />
          <button className="ctrl sm primary" type="submit" disabled={!draft.trim() || !!problem}>
            Post
          </button>
          {problem && <div className="thread-problem">{problem}</div>}
        </form>
      )}
    </section>
  );
}
