// The open idea's conversations (docs/proposals/idea-board-phases.md §7.3,
// phase S): a header with the conversation's title and a menu of the idea's
// conversations, and the main window's chat view driving the `idea_*` rows.
//
// A conversation is Claude in its own tmux session (`worktrees~idea~<conv>`)
// with the ideas folder as its working directory, never a place. Its state
// comes from the `ideas:sessions` push (by session name); the list row
// (`idea_conversations`) carries its record: title, start, the history copy
// it started from. An ended conversation reads read-only, with Resume.
import { useCallback, useEffect, useMemo, useState } from "react";
import { ChatPane } from "../src/ChatPane";
import { ideaChatKey } from "../src/chat";
import { invoke } from "../src/ipc";
import { revealOnHost } from "../src/onhost";
import { useArmed } from "../src/useArmed";
import { useEscape } from "../src/useEscape";

/** `ideas:sessions`' payload (`worktrees_api::watch::IdeaSessions`). */
export type IdeaSessionsEv = { live: string[]; busy: string[]; waiting: string[]; idea_of: Record<string, string> };
export const NO_SESSIONS: IdeaSessionsEv = { live: [], busy: [], waiting: [], idea_of: {} };

export type ConvState = "running" | "busy" | "waiting" | "ended";
export type Conv = {
  id: string;
  harness: string;
  session_id: string;
  title: string;
  started: string;
  started_from: string;
  session: string;
  state: ConvState;
};

/** What this window just did to a conversation, until a push agrees: a
 *  start or resume answers once its session exists, and the poll's next push
 *  is up to a tick away, so the last push still says "ended" (measured on a
 *  real serve: one tick of a Resume bar over a conversation that was starting). */
export type Expect = { conv: string; live: boolean } | null;

/** A conversation's state now: the push wins over the list's, which is as
 *  old as its last read — except while `expect` says what this window just
 *  did and the push has not caught up. */
export function stateOf(c: Pick<Conv, "session" | "state"> & { id?: string }, ev: IdeaSessionsEv | null, expect: Expect = null): ConvState {
  if (expect && c.id === expect.conv && ev && ev.live.includes(c.session) !== expect.live) return expect.live ? "running" : "ended";
  if (!ev) return c.state;
  if (ev.waiting.includes(c.session)) return "waiting";
  if (ev.busy.includes(c.session)) return "busy";
  if (ev.live.includes(c.session)) return "running";
  return "ended";
}

/** The dot an idea gets in the list: amber while a conversation waits on the
 *  person, green while one works, a ring while one is merely open. */
export function ideaDots(ev: IdeaSessionsEv | null): Map<string, ConvState> {
  const out = new Map<string, ConvState>();
  if (!ev) return out;
  const rank: Record<ConvState, number> = { waiting: 3, busy: 2, running: 1, ended: 0 };
  for (const [session, idea] of Object.entries(ev.idea_of ?? {})) {
    const s = stateOf({ session, state: "ended" }, ev);
    if (rank[s] > rank[out.get(idea) ?? "ended"]) out.set(idea, s);
  }
  return out;
}

const STATE_WORDS: Record<ConvState, string> = { running: "running", busy: "working", waiting: "waiting on you", ended: "ended" };

function startedWords(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString([], { year: "numeric", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}

const errText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));

export function IdeaChat(props: {
  ideaId: string;
  /** The ideas folder (`ideas_list`'s `dir`): what a reply's paths are under,
   *  and where "started from this version" is. */
  dir: string | null;
  sessions: IdeaSessionsEv | null;
  terminalCmd: string;
  uiRem: number;
  onNotice(msg: string): void;
}) {
  const { ideaId, sessions } = props;
  const [convs, setConvs] = useState<Conv[] | null>(null);
  const [sel, setSel] = useState<string | null>(null);
  const [menu, setMenu] = useState(false);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [expect, setExpect] = useState<Expect>(null);
  const arm = useArmed<"remove">();
  useEscape(() => setMenu(false), menu);

  const refresh = useCallback(async () => {
    try {
      const r = await invoke<{ conversations: Conv[] }>("idea_conversations", { id: ideaId });
      setConvs(r.conversations);
      setSel((s) => (s && r.conversations.some((c) => c.id === s) ? s : r.conversations[0]?.id ?? null));
    } catch (e) {
      props.onNotice(`Could not list this idea's conversations: ${errText(e)}`);
      setConvs([]);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ideaId]);
  useEffect(() => { setConvs(null); setSel(null); setMenu(false); setRenaming(null); void refresh(); }, [refresh]);
  // A conversation that started or ended elsewhere (a terminal, a quit):
  // re-read the list when the live set moves.
  const liveKey = (sessions?.live ?? []).join(",");
  useEffect(() => { void refresh(); }, [liveKey, refresh]);

  // A push that agrees with what this window did ends the expectation.
  useEffect(() => {
    if (!expect || !sessions) return;
    const c = convs?.find((x) => x.id === expect.conv);
    if (!c || sessions.live.includes(c.session) === expect.live) setExpect(null);
  }, [sessions, expect, convs]);
  const cur = useMemo(() => convs?.find((c) => c.id === sel) ?? null, [convs, sel]);
  const state = cur ? stateOf(cur, sessions, expect) : "ended";

  const act = async (what: string, run: () => Promise<unknown>) => {
    setBusy(true);
    try { await run(); } catch (e) { props.onNotice(`Could not ${what}: ${errText(e)}`); }
    finally { setBusy(false); void refresh(); }
  };
  const start = () => act("start a conversation", async () => {
    setMenu(false);
    const r = await invoke<{ conversation: Conv }>("idea_conversation_start", { id: ideaId });
    setExpect({ conv: r.conversation.id, live: true });
    setSel(r.conversation.id);
  });
  const resume = () => cur && act("resume it", async () => {
    await invoke("idea_conversation_resume", { id: ideaId, conv: cur.id });
    setExpect({ conv: cur.id, live: true });
  });
  const close = () => cur && act("close it", async () => {
    await invoke("idea_conversation_close", { id: ideaId, conv: cur.id });
    setExpect({ conv: cur.id, live: false });
  });
  const remove = () => cur && arm.confirm("remove", () => void act("remove it", async () => {
    await invoke("idea_conversation_remove", { id: ideaId, conv: cur.id });
    setSel(null);
  }));
  const rename = (title: string) => {
    setRenaming(null);
    if (!cur || !title.trim() || title.trim() === cur.title) return;
    void act("rename it", () => invoke("idea_conversation_rename", { id: ideaId, conv: cur.id, title: title.trim() }));
  };
  const terminal = () => {
    if (!cur) return;
    if (!props.terminalCmd) { props.onNotice(`Set a terminal command in Settings to open it there, or attach by hand: tmux attach -t '=${cur.session}'`); return; }
    invoke("open_terminal", { cmd: props.terminalCmd, session: cur.session }).catch((e) => props.onNotice(`Could not open it in a terminal: ${errText(e)}`));
  };
  const startedFrom = () => {
    if (!cur || !props.dir) return;
    revealOnHost(`${props.dir}/${cur.started_from}`).catch((e) => props.onNotice(`Could not show it: ${errText(e)}`));
  };

  if (convs === null) return <aside className="ix-side ix-chat" data-testid="idea-chat"><p className="ix-quiet">Loading conversations…</p></aside>;

  return (
    <aside className="ix-side ix-chat" data-testid="idea-chat" aria-label="Conversations">
      <header className="ix-chat-head">
        {renaming !== null && cur ? (
          <input className="ix-rename" data-testid="idea-conv-rename-input" autoFocus value={renaming}
            onChange={(e) => setRenaming(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") rename(renaming); if (e.key === "Escape") setRenaming(null); }}
            onBlur={() => setRenaming(null)} />
        ) : (
          <button className="ix-chat-title" data-testid="idea-conv-menu" aria-expanded={menu} aria-haspopup="menu" onClick={() => setMenu((m) => !m)}>
            {/* The row is an inner span: WebKit shrink-wraps a <button> that is
                itself the flex container (AGENTS.md). */}
            <span className="ix-chat-title-in">
              {cur ? <><span className={`ix-dot ix-dot-${state}`} aria-hidden="true" /><span className="ix-chat-name">{cur.title}</span><span className="ix-quiet ix-nowrap">· {STATE_WORDS[state]}</span></> : <span className="ix-chat-name">Conversations</span>}
              <span aria-hidden="true">▾</span>
            </span>
          </button>
        )}
        <div className="ix-tools">
          {cur && state !== "ended" && <button className="ix-btn" data-testid="idea-conv-terminal" onClick={terminal} title="Attach to this conversation in your terminal app">Open in terminal</button>}
          {cur && state !== "ended" && <button className="ix-btn" data-testid="idea-conv-close" disabled={busy} onClick={() => void close()}>Close</button>}
          {cur && state === "ended" && <button className="ix-btn" data-testid="idea-conv-resume" disabled={busy} onClick={() => void resume()}>Resume</button>}
        </div>
      </header>
      {menu && (
        <div className="ix-chat-menu" role="menu" data-testid="idea-conv-list">
          <button role="menuitem" className="ix-btn ix-btn-primary" data-testid="idea-conv-new" disabled={busy} onClick={() => void start()}>New conversation</button>
          {convs.map((c) => {
            const s = stateOf(c, sessions, expect);
            return (
              <button key={c.id} role="menuitem" className={`ix-chat-row${c.id === sel ? " on" : ""}`} data-conv={c.id} data-state={s}
                onClick={() => { setSel(c.id); setMenu(false); arm.disarm(); }}>
                <span className="ix-chat-row-in">
                  <span className={`ix-dot ix-dot-${s}`} aria-hidden="true" />
                  <span className="ix-row-title">{c.title}</span>
                  <span className="ix-quiet ix-nowrap">{STATE_WORDS[s]} · {startedWords(c.started)}</span>
                </span>
              </button>
            );
          })}
          {cur && (
            <div className="ix-chat-menu-foot">
              <button className="ix-btn ix-btn-quiet" data-testid="idea-conv-rename" onClick={() => { setMenu(false); setRenaming(cur.title); }}>Rename</button>
              {props.dir && <button className="ix-btn ix-btn-quiet" data-testid="idea-conv-started" onClick={startedFrom} title={cur.started_from}>Started from this version</button>}
              <button className={`ix-btn ix-btn-quiet${arm.armed === "remove" ? " ix-btn-armed" : ""}`} data-testid="idea-conv-remove" disabled={busy} onClick={remove}
                title="Forgets this conversation here. Claude's own transcript is left as it is.">
                {arm.armed === "remove" ? "Remove this conversation?" : "Remove"}
              </button>
            </div>
          )}
        </div>
      )}
      {cur ? (
        <div className="ix-chat-body">
          <ChatPane key={cur.id} repo="" slug="" path={ideaChatKey({ id: ideaId, conv: cur.id })} root={props.dir ?? undefined}
            idea={{ id: ideaId, conv: cur.id }} provider="claude" session={cur.session}
            activity={state === "busy" ? "busy" : state === "waiting" ? "waiting" : ""}
            doneAt={0} paneDraft={null} uiRem={props.uiRem}
            onTerminal={terminal} onOpenPath={() => {}} reloadToken={0} onError={props.onNotice}
            readOnly={state === "ended" ? (
              <div className="ix-chat-ended" data-testid="idea-conv-ended">
                <span>This conversation has ended. Started {startedWords(cur.started)}.</span>
                <button className="ix-btn ix-btn-primary" disabled={busy} onClick={() => void resume()}>Resume</button>
              </div>
            ) : undefined} />
        </div>
      ) : (
        <div className="ix-none">
          <p className="ix-none-title">No conversation yet</p>
          <p className="ix-quiet">Talk this idea through with Claude. The idea stays as it is unless you ask for a change.</p>
          <button className="ix-btn ix-btn-primary" data-testid="idea-conv-first" disabled={busy} onClick={() => void start()}>New conversation</button>
        </div>
      )}
    </aside>
  );
}
