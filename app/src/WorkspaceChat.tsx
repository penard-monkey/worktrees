/** A workspace's chats on screen (docs/proposals/workspace-view-design.md §2, §5):
 *  the rows in its nav card, the column a chat is in (the middle of the view),
 *  and the lists of them. The chat itself is the place's ChatPane on the
 *  `workspace_*` rows; the rules are wschats.ts's. Behind workspace chats' beta.
 *  Everything here is at module scope: a component with state inside App
 *  remounts on every render. */
import { useEffect, useMemo, useRef, useState } from "react";
import { ChatPane } from "./ChatPane";
import { useChat, wsChatKey } from "./chat";
import { CtxMenu } from "./CtxMenu";
import * as Icons from "./icons";
import { DeskTerminalPane } from "./TerminalPane";
import { useArmed } from "./useArmed";
import {
  activeAt, ageWords, asksTrust, engineLine, firstMessage, introWords, isDefaultTitle, monthOf, navRows, readsWords, stateWords, titleFrom, TRUST_WORDS, whenWords,
  type ChatRow, type ProjectRead, type RowState, type WsChat, type WsChats,
} from "./wschats";
import type { WsChatsLive } from "./useWsChats";
import "./WorkspaceChat.css";

const DOT_TITLE: Record<RowState, string> = {
  waiting: "waiting on you", busy: "working", running: "open, idle", folding: "folding", "fold-failed": "its fold failed", ended: "closed",
};
/** The hue is the dot's, never the words'. */
export function ChatDot({ state }: { state: RowState }) {
  return <span className={`wsc-dot ${state}`} title={DOT_TITLE[state]} aria-hidden="true" />;
}

/** The Chats group in a workspace's card, above its projects. */
export function NavChats(props: {
  rows: readonly ChatRow[];
  total: number;
  /** The nav's filter, lower case; "" for none. */
  q: string;
  /** The chat on screen. */
  on: string | null;
  starting: boolean;
  error: string | null;
  ago(epoch: number): string;
  open(conv: string): void;
  start(): void;
  all(): void;
  retry(): void;
}) {
  const { rows, rest } = navRows(props.rows, props.q);
  const live = props.rows.filter((r) => r.live).length;
  if (props.q && rows.length === 0) return null;
  return (
    <div className="wsc-nav" data-testid="ws-chats">
      <div className="wsc-nav-h">
        <span className="wsc-nav-t">Chats</span>
        {live > 0 && <span className="wsc-nav-n">{live} live</span>}
        <button className="wsc-plus" title="New chat" aria-label="New chat" data-testid="ws-chat-new" data-track="nav.workspace.chat.new" disabled={props.starting} onClick={props.start}><Icons.Plus size={13} /></button>
      </div>
      {props.error && <p className="wsc-nav-err" role="alert" data-testid="ws-chats-error">Its chats could not be read: {props.error}. <button className="wsf-link" onClick={props.retry}>Try again</button></p>}
      {props.starting && <div className="wsc-row starting" data-testid="ws-chat-starting"><span className="wsc-dot folding" aria-hidden="true" /><span className="wsc-title">Starting…</span></div>}
      {rows.map((r) => {
        const age = ageWords(r, props.ago(activeAt(r.chat)));
        return (
          <button key={r.chat.id} className={"wsc-row" + (r.state === "ended" ? " closed" : "") + (r.chat.id === props.on ? " on" : "")} data-testid="ws-chat-row" data-track="nav.workspace.chat"
            data-chat={r.chat.id} data-state={r.state} title={`${r.chat.title} · ${stateWords(r)}`} aria-current={r.chat.id === props.on || undefined} onClick={() => props.open(r.chat.id)}>
            <span className="wsc-row-in">
              <ChatDot state={r.state} />
              <span className="wsc-title">{r.chat.title}</span>
              <span className={"wsc-age" + (age.strong ? " strong" : "")}>{age.text}</span>
            </span>
          </button>
        );
      })}
      {rest && <button className="wsc-all" data-testid="ws-chats-all" data-track="nav.workspace.chats" onClick={props.all}>All chats · {props.total} ▸</button>}
    </div>
  );
}

/** Which projects a chat reads, and why not the others. */
function Reads({ projects }: { projects: readonly ProjectRead[] }) {
  return (
    <ul className="wsc-reads" data-testid="ws-chat-reads">
      {projects.length === 0 && <li>This workspace has no projects.</li>}
      {projects.map((p) => (
        <li key={p.root} data-reads={p.reads}>
          <span className="wsc-reads-n" title={p.root}>{p.name ?? p.root.slice(p.root.lastIndexOf("/") + 1)}</span>
          <span className="wsc-reads-w">{p.reads === "yes" ? "read" : `not read: ${p.why ?? "it may not be"}`}</span>
        </li>
      ))}
    </ul>
  );
}

/** The middle column: one of the workspace's chats, or the start that was
 *  refused. */
export function WorkspaceChatColumn(props: {
  ws: { id: string; name: string };
  info: WsChats;
  rows: readonly ChatRow[];
  conv: string;
  live: WsChatsLive;
  ago(epoch: number): string;
  terminalCmd: string;
  uiRem: number;
  /** The engine connection's number: an attach belongs to its socket. */
  gen: number;
  open(conv: string): void;
  /** No chat: the documents take the whole room. */
  away(): void;
  openTerminalApp(session: string): void;
  onError(msg: string): void;
}) {
  const { ws, info, conv, live } = props;
  const row = props.rows.find((r) => r.chat.id === conv) ?? null;
  const c: WsChat | null = row?.chat ?? null;
  const [view, setView] = useState<"chat" | "terminal">("chat");
  const [menu, setMenu] = useState<{ kind: "chats" | "reads" | "close"; x: number; y: number } | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const arm = useArmed<"delete" | "close">();
  useEffect(() => { setMenu(null); setRenaming(null); setView("chat"); }, [conv]);

  // A chat nobody has named takes its name from the person's first message.
  const key = wsChatKey({ id: ws.id, conv });
  const turns = useChat(key).turns.data?.turns;
  const named = useRef(new Set<string>());
  const first = c && isDefaultTitle(c.title) && turns ? firstMessage(turns, conv) : null;
  useEffect(() => {
    const t = first && titleFrom(first);
    if (!t || named.current.has(conv)) return;
    named.current.add(conv);
    live.rename(ws.id, conv, t).catch((e) => props.onError(`Could not name the chat: ${String(e)}`));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [first, conv]);

  const act = async (what: string, run: () => Promise<void>) => {
    setBusy(true);
    try { await run(); } catch (e) { props.onError(`Could not ${what}: ${String(e)}`); }
    finally { setBusy(false); }
  };
  if (!c || !row) {
    return (
      <div className="wsc-col" data-testid="ws-chat-col">
        <div className="wsc-gone"><p>This chat is not here any more.</p><button className="ctrl sm" onClick={props.away}>Back to the documents</button></div>
      </div>
    );
  }
  const at = (kind: "chats" | "reads" | "close") => (e: React.MouseEvent) => {
    const r = e.currentTarget.getBoundingClientRect();
    setMenu((m) => (m?.kind === kind ? null : { kind, x: kind === "close" ? r.right - 180 : r.left, y: r.bottom + 4 }));
  };
  const resume = () => void act("resume it", () => live.resume(ws.id, conv));
  const close = () => void act("close it", () => live.close(ws.id, conv));
  // It is working: closing ends the turn, so it asks once.
  const closeAsk = () => (row.state === "busy" ? arm.confirm("close", close) : close());
  const remove = () => arm.confirm("delete", () => { setMenu(null); void act("delete it", async () => { await live.remove(ws.id, conv); props.away(); }); });
  const rename = (t: string) => {
    setRenaming(null);
    if (t.trim() && t.trim() !== c.title) void act("rename it", () => live.rename(ws.id, conv, t.trim()));
  };
  const term = view === "terminal" && row.live;
  return (
    <div className="wsc-col" data-testid="ws-chat-col" data-chat={conv} data-state={row.state}>
      <header className="wsc-head">
        {renaming !== null ? (
          <input className="title-input wsc-rename" data-testid="ws-chat-rename-input" autoFocus value={renaming}
            onChange={(e) => setRenaming(e.currentTarget.value)}
            onKeyDown={(e) => { if (e.key === "Enter") rename(renaming); if (e.key === "Escape") { e.stopPropagation(); setRenaming(null); } }}
            onBlur={() => setRenaming(null)} />
        ) : (
          <button className="wsc-name" data-testid="ws-chat-menu" data-track="workspace.chat.menu" title={`${c.title} · ${stateWords(row)}`} aria-haspopup="menu" aria-expanded={menu?.kind === "chats"} onClick={at("chats")}>
            {/* An inner span is the row: WebKit shrink-wraps a <button> that is
                itself the flex container (AGENTS.md). */}
            <span className="wsc-name-in"><ChatDot state={row.state} /><span className="wsc-name-t">{c.title}</span><Icons.ChevronDown size={11} /></span>
          </button>
        )}
        <button className="wsc-readsb" data-testid="ws-chat-readsb" data-track="workspace.chat.reads" title="Which projects this chat reads" aria-expanded={menu?.kind === "reads"} onClick={at("reads")}>{readsWords(info.projects)} ▾</button>
        <span className="wsc-acts">
          {row.live && (
            <span className="agent-view" role="group" aria-label="View">
              <button className={"agent-view-b" + (term ? "" : " on")} aria-pressed={!term} data-testid="ws-chat-view-chat" onClick={() => setView("chat")}>Chat</button>
              <button className={"agent-view-b" + (term ? " on" : "")} aria-pressed={term} data-testid="ws-chat-view-terminal" onClick={() => setView("terminal")}>Terminal</button>
            </span>
          )}
          {row.live ? (
            <span className="wsc-close">
              <button className={"ctrl sm" + (arm.armed === "close" ? " armed" : "")} data-testid="ws-chat-close" data-track="workspace.chat.close" disabled={busy} title="End this chat. It stays here to read, and can be resumed." onClick={closeAsk}>
                {arm.armed === "close" ? "It is working. Close?" : "Close"}
              </button>
              <button className="ctrl sm" data-testid="ws-chat-close-menu" aria-label="More" aria-haspopup="menu" aria-expanded={menu?.kind === "close"} onClick={at("close")}>▾</button>
            </span>
          ) : <button className="ctrl sm" data-testid="ws-chat-close-menu" aria-label="More" aria-haspopup="menu" aria-expanded={menu?.kind === "close"} onClick={at("close")}>▾</button>}
        </span>
      </header>
      <div className="wsc-body">
        {/* The terminal is drawn instead of the chat, never under it: an attach
            that nobody sees would hold the session to a size nobody chose. */}
        {term ? (
          <div className="wsc-term" data-testid="ws-chat-term">
            <DeskTerminalPane key={`${c.session}:${props.gen}`} session={c.session} desk={{ ws: { id: ws.id, conv } }} focusToken={1} />
          </div>
        ) : (
          <ChatPane key={conv} repo="" slug="" path={key} root={info.folder} ws={{ id: ws.id, conv }} provider="claude" session={c.session}
            activity={row.state === "busy" ? "busy" : row.state === "waiting" ? "waiting" : ""}
            doneAt={activeAt(c)} paneDraft={null} uiRem={props.uiRem} to="this chat" quiet={(t) => engineLine(t, conv)}
            promptNote={(lines) => (asksTrust(lines) ? <p className="wsc-trust" data-testid="ws-chat-trust">{TRUST_WORDS}</p> : null)}
            intro={<>A chat of <b>{ws.name}</b>. {introWords(info.projects)}</>}
            onTerminal={() => setView("terminal")} onOpenPath={() => {}} reloadToken={0} onError={props.onError}
            readOnly={row.live ? undefined : (
              <div className="wsc-ended" data-testid="ws-chat-ended">
                <span>{stateWords(row).replace(/^./, (x) => x.toUpperCase())}. Started {whenWords(c.started)}.</span>
                <button className="enter-btn sm" data-testid="ws-chat-resume" data-track="workspace.chat.resume" disabled={busy} onClick={resume}>Resume</button>
              </div>
            )} />
        )}
      </div>
      {menu?.kind === "chats" && (
        <CtxMenu x={menu.x} y={menu.y} onClose={() => setMenu(null)}>
          <div className="wsc-menu" data-testid="ws-chat-list">
            <button className="pop-item" data-testid="ws-chat-rename" data-track="workspace.chat.rename" onClick={() => { setMenu(null); setRenaming(c.title); }}>Rename…</button>
            <button className="pop-item" data-testid="ws-chat-away" data-track="workspace.chat.away" onClick={() => { setMenu(null); props.away(); }}>Put the chat away</button>
            <div className="pop-hint">the workspace's chats</div>
            {props.rows.map((r) => (
              <button key={r.chat.id} className="pop-item" data-testid="ws-chat-list-row" data-track="workspace.chat.pick" data-chat={r.chat.id} title={`${r.chat.title} · ${stateWords(r)}`}
                onClick={() => { setMenu(null); props.open(r.chat.id); }}>
                <ChatDot state={r.state} /><span className="wsc-menu-t">{r.chat.title}</span><span className="wsc-age">{props.ago(activeAt(r.chat))}</span>
              </button>
            ))}
          </div>
        </CtxMenu>
      )}
      {menu?.kind === "reads" && <CtxMenu x={menu.x} y={menu.y} onClose={() => setMenu(null)}><Reads projects={info.projects} /></CtxMenu>}
      {menu?.kind === "close" && (
        <CtxMenu x={menu.x} y={menu.y} onClose={() => { setMenu(null); arm.disarm(); }}>
          {row.live && <button className="pop-item" data-testid="ws-chat-close-item" data-track="workspace.chat.close" onClick={() => { setMenu(null); closeAsk(); }}>Close</button>}
          {row.live && props.terminalCmd.trim() && <button className="pop-item" data-testid="ws-chat-terminal-app" data-track="workspace.chat.terminal-app" onClick={() => { setMenu(null); props.openTerminalApp(c.session); }}>Open in terminal app</button>}
          <button className={"pop-item danger" + (arm.armed === "delete" ? " armed" : "")} data-testid="ws-chat-delete" data-track="workspace.chat.delete"
            title="Forgets this chat and its folder here. Its note in memory stays, and so does Claude's own transcript." onClick={remove}>
            {arm.armed === "delete" ? "Delete this chat?" : "Delete"}
          </button>
        </CtxMenu>
      )}
    </div>
  );
}

/** A start the engine refused (the live-agent limit, a spent window, no Claude),
 *  in the chat's side, in its own words. */
export function RefusedStart(props: { why: string; busy: boolean; again(): void; away(): void }) {
  return (
    <div className="wsc-col" data-testid="ws-chat-refused">
      <div className="wsc-gone" role="alert">
        <p><b>The chat was not started.</b></p>
        <p>{props.why}</p>
        <div className="ws-ov-line">
          <button className="ctrl sm" data-testid="ws-chat-again" disabled={props.busy} onClick={props.again}>Try again</button>
          <button className="ctrl sm" data-testid="ws-chat-dismiss" onClick={props.away}>Dismiss</button>
        </div>
      </div>
    </div>
  );
}

/** A new chat whose session is being made. */
export function StartingChat({ name }: { name: string }) {
  return <div className="wsc-col" data-testid="ws-chat-starting-col"><div className="wsc-gone"><p role="status">Starting a chat of {name}…</p></div></div>;
}

/** One line of a list of chats (the Overview's card). */
export function ChatLines(props: { rows: readonly ChatRow[]; ago(epoch: number): string; open(conv: string): void }) {
  return (
    <div className="ws-ov-rows">
      {props.rows.map((r) => (
        <button key={r.chat.id} className="wsc-line" data-testid="ws-chat-line" data-chat={r.chat.id} data-state={r.state} onClick={() => props.open(r.chat.id)}>
          <span className="wsc-line-in">
            <ChatDot state={r.state} />
            <span className="ws-ov-name">{r.chat.title}</span>
            <span className="ws-ov-detail">{stateWords(r)}</span>
            <span className="wsc-age">{props.ago(activeAt(r.chat))}</span>
          </span>
        </button>
      ))}
    </div>
  );
}

/** The Chats tab: every chat, what needs somebody first, the closed ones by
 *  month. */
export function ChatsTab(props: { note(path: string): void; rows: readonly ChatRow[]; on: string | null; starting: boolean; ago(epoch: number): string; open(conv: string): void; start(): void }) {
  const [q, setQ] = useState("");
  const needle = q.trim().toLowerCase();
  const hit = props.rows.filter((r) => !needle || r.chat.title.toLowerCase().includes(needle));
  const groups = useMemo(() => {
    const out: { name: string; rows: ChatRow[] }[] = [];
    for (const r of hit) {
      const name = r.state === "ended" ? monthOf(r.chat) : "Open";
      const g = out.find((x) => x.name === name);
      if (g) g.rows.push(r); else out.push({ name, rows: [r] });
    }
    return out;
  }, [hit]);
  return (
    <div className="wsc-tab" data-testid="ws-chats-tab">
      <div className="wsc-tab-h">
        <input className="wsc-filter" data-testid="ws-chats-filter" placeholder={`Filter ${props.rows.length} chats`} value={q} spellCheck={false} onChange={(e) => setQ(e.currentTarget.value)} />
        <button className="enter-btn sm" data-testid="ws-chats-new" data-track="workspace.chat.new" disabled={props.starting} onClick={props.start}>New chat</button>
      </div>
      {hit.length === 0 && <p className="ws-ov-none">No chat's title matches.</p>}
      {groups.map((g) => (
        <section key={g.name} aria-label={g.name}>
          <div className="wsf-card-name">{g.name}</div>
          <ul className="wsc-list">
            {g.rows.map((r) => (
              <li key={r.chat.id} className={"wsc-item" + (r.chat.id === props.on ? " on" : "")} data-testid="ws-chats-row" data-chat={r.chat.id} data-state={r.state}>
                <button className="wsc-item-b" onClick={() => props.open(r.chat.id)}>
                  <span className="wsc-line-in">
                    <ChatDot state={r.state} />
                    <span className="ws-ov-name">{r.chat.title}</span>
                    <span className="ws-ov-detail">{stateWords(r)}</span>
                  </span>
                </button>
                {r.chat.note_exists && <button className="wsf-link" data-testid="ws-chats-note" title={r.chat.note} onClick={() => props.note(r.chat.note)}>note</button>}
                <span className="wsc-when" title={`Last active ${whenWords(r.chat.last_activity || r.chat.started)}`}>started {whenWords(r.chat.started)}{props.ago(activeAt(r.chat)) ? ` · active ${props.ago(activeAt(r.chat))}` : ""}</span>
              </li>
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}
