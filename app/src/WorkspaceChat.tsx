/** A workspace's chats on screen (docs/proposals/workspace-view-design.md §2, §5):
 *  the rows in its nav card, the column a chat is in (the middle of the view),
 *  and the lists of them. The chat itself is the place's ChatPane on the
 *  `workspace_*` rows; the rules are wschats.ts's. Behind workspace chats' beta.
 *  Everything here is at module scope: a component with state inside App
 *  remounts on every render. */
import { useEffect, useMemo, useRef, useState } from "react";
import { ChatPane } from "./ChatPane";
import { stopTurn, useChat, wsChatKey } from "./chat";
import { memoryDocs, type MemoryChange } from "./doc/memory";
import { CtxMenu } from "./CtxMenu";
import * as Icons from "./icons";
import { DeskTerminalPane } from "./TerminalPane";
import { useArmed } from "./useArmed";
import {
  accessWords, activeAt, ageWords, asksTrust, chatActs, endedLine, failedWords, firstMessage, foldOf, foldRuns, introWords, isDefaultTitle, monthOf, navRows, quietLine, readsWords, stateWords, titleFrom, transcriptGone, TRUST_WORDS, whenWords,
  type ChatAct, type ChatActItem, type ChatRow, type DocRead, unreadWords, type ProjectRead, type RowState, type WsChat, type WsChats, type WsSessionsEv,
  modelOf, modelWords, type ChatPick, type HarnessPick,
} from "./wschats";
import { ModelPicker, useAgentModels } from "./ModelPicker";
import { HARNESS_LABEL, isHarness, type Harness } from "./harness";
import type { WsChatsLive } from "./useWsChats";
import { RevertButton, useRevert } from "./WorkspaceMemory";
import { chatNote } from "./wsmemory";
import "./WorkspaceChat.css";

const DOT_TITLE: Record<RowState, string> = {
  waiting: "waiting on you", busy: "working", running: "open, idle", folding: "folding", checking: "checking its fold", "fold-failed": "its fold failed", ended: "closed",
};
/** The hue is the dot's, never the words'. */
export function ChatDot({ state }: { state: RowState }) {
  return <span className={`wsc-dot ${state}`} title={DOT_TITLE[state]} aria-hidden="true" />;
}

/** What a chat runs on, in words: `Claude`, `Claude · sonnet`. */
export const onWords = (c: WsChat): string => `${isHarness(c.harness) ? HARNESS_LABEL[c.harness] : c.harness}${c.model ? ` · ${c.model}` : ""}`;

/** What the acts on a chat need from the window: the same for its header and
 *  for a row's right-click menu. */
export type ChatActsHost = {
  ws: string;
  live: WsChatsLive;
  /** What is open of the workspace's documents is saved: asked before an act. */
  ready(): Promise<boolean>;
  onError(msg: string): void;
  /** A close left changes in the memory: its commit. */
  landed(commit: string): void;
  /** Chat `conv` was deleted. */
  gone(conv: string): void;
};

/** Another chat of the workspace whose fold runs: the engine folds one at a time. */
function folding(rows: readonly ChatRow[], conv: string, ev: WsSessionsEv | null): string | null {
  return rows.find((r) => r.chat.id !== conv && foldRuns(foldOf(r.chat, ev)))?.chat.title ?? null;
}

/** The acts on one chat and their state: ONE copy, which the header and a row's
 *  menu both run. `said`: where the engine's sentence goes when it will not
 *  fold (the header shows it under itself; elsewhere it is an error line).
 *  `reattach`: an attached terminal must attach again. */
function useChatActs(h: ChatActsHost, row: ChatRow | null, rows: readonly ChatRow[], gone: boolean, opts: { said?(s: string | null): void; reattach?(): void } = {}) {
  const [busy, setBusy] = useState(false);
  const arm = useArmed<"delete" | "close">();
  const conv = row?.chat.id ?? "";
  const running = row ? foldRuns(foldOf(row.chat, h.live.sessions)) : false;
  const items: ChatActItem[] = row ? chatActs(row, running, gone, folding(rows, conv, h.live.sessions)) : [];
  const said = (s: string | null) => { if (opts.said) opts.said(s); else if (s) h.onError(s.replace(/^./, (x) => x.toUpperCase()).replace(/([^.!?])$/, "$1.")); };
  const act = async (what: string, run: () => Promise<void>, gated = true) => {
    setBusy(true);
    try { if (!gated || (await h.ready())) await run(); } catch (e) { h.onError(`Could not ${what}: ${String(e)}`); }
    finally { setBusy(false); }
  };
  const close = () => void act("close it", async () => { said(null); const r = await h.live.close(h.ws, conv); if (r.commit) h.landed(r.commit); });
  /** Close: the fold. The engine says, in its words, when it will not. */
  const fold = async () => {
    said(null);
    setBusy(true);
    try { if ((await h.ready()) && (await h.live.fold(h.ws, conv)).reattach) opts.reattach?.(); } catch (e) { said(String(e)); }
    finally { setBusy(false); }
  };
  return {
    busy, arm, items, running, act,
    item: (a: ChatAct) => items.find((i) => i.act === a) ?? null,
    fold, close,
    // It is working: closing ends the turn, so it asks once.
    closeAsk: () => (row?.state === "busy" ? arm.confirm("close", close) : close()),
    resume: () => void act("resume it", () => h.live.resume(h.ws, conv)),
    remove: (after?: () => void) => arm.confirm("delete", () => { after?.(); void act("delete it", async () => { await h.live.remove(h.ws, conv); h.gone(conv); }, !!row?.live); }),
  };
}
type ChatActs = ReturnType<typeof useChatActs>;

const ACT_TESTID: Record<ChatAct, string> = { close: "ws-chat-act-close", "close-none": "ws-chat-act-close-none", "fold-now": "ws-chat-act-fold-now", resume: "ws-chat-act-resume", delete: "ws-chat-act-delete" };
const ACT_TRACK: Record<ChatAct, string> = { close: "workspace.chat.row.fold", "close-none": "workspace.chat.row.close", "fold-now": "workspace.chat.row.fold-now", resume: "workspace.chat.row.resume", delete: "workspace.chat.row.delete" };
/** One act as a menu item, by [`chatActs`]' rule: refused, it is shown with the
 *  reason and cannot be pressed. */
function ActItem({ a, acts, busy, row, close }: { a: ChatActItem; acts: ChatActs; busy: boolean; row: ChatRow; close(): void }) {
  const armed = acts.arm.armed;
  const run = () => {
    switch (a.act) {
      case "close": close(); void acts.fold(); return;
      case "fold-now": close(); void acts.fold(); return;
      case "resume": close(); acts.resume(); return;
      case "close-none": if (row.state !== "busy" || armed === "close") close(); acts.closeAsk(); return;
      case "delete": acts.remove(close); return;
    }
  };
  const danger = a.act === "delete" || (a.act === "close-none" && armed === "close");
  const label = a.act === "close-none" && armed === "close" ? "It is working. Close?" : a.act === "delete" && armed === "delete" ? "Delete this chat?" : a.label;
  return (
    <button className={"pop-item" + (danger ? " danger" : "") + ((a.act === "delete" && armed === "delete") || (a.act === "close-none" && armed === "close") ? " armed" : "")}
      data-testid={ACT_TESTID[a.act]} data-act={a.act} data-track={ACT_TRACK[a.act]}
      disabled={busy || !!a.why} title={a.why ?? a.title} onClick={run}>
      <span className="wsc-act-in"><span>{label}</span>{a.why && <span className="wsc-act-why">{a.why}</span>}</span>
    </button>
  );
}

/** A chat row's right-click menu: the acts its header has, by its state. */
function ChatRowMenu({ x, y, host, row, rows, onClose }: { x: number; y: number; host: ChatActsHost; row: ChatRow; rows: readonly ChatRow[]; onClose(): void }) {
  const reason = useChat(wsChatKey({ id: host.ws, conv: row.chat.id })).turns.data?.reason;
  const acts = useChatActs(host, row, rows, transcriptGone(reason));
  return (
    <CtxMenu x={x} y={y} onClose={() => { acts.arm.disarm(); onClose(); }}>
      <div className="pop-head wsc-menu-h" title={row.chat.title}>{row.chat.title}</div>
      {acts.items.map((a) => <ActItem key={a.act} a={a} acts={acts} busy={acts.busy} row={row} close={onClose} />)}
    </CtxMenu>
  );
}

/** A row's right-click menu, opened by the pointer or by the menu key. */
function useRowMenu() {
  const [m, setM] = useState<{ conv: string; x: number; y: number } | null>(null);
  const on = (conv: string) => ({
    onContextMenu: (e: React.MouseEvent) => { e.preventDefault(); e.stopPropagation(); setM({ conv, x: e.clientX, y: e.clientY }); },
    onKeyDown: (e: React.KeyboardEvent) => {
      if (e.key !== "ContextMenu" && !(e.key === "F10" && e.shiftKey)) return;
      e.preventDefault();
      e.stopPropagation();
      const r = e.currentTarget.getBoundingClientRect();
      setM({ conv, x: r.left + 12, y: r.bottom });
    },
  });
  return { m, on, close: () => setM(null) };
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
  /** Arm a drag of a row: into a place's chat, it is a reference. */
  arm?(e: React.PointerEvent, conv: string, title: string): void;
  /** A row's right-click menu. */
  acts?: ChatActsHost;
}) {
  const { rows, rest } = navRows(props.rows, props.q);
  const menu = useRowMenu();
  const mrow = menu.m && props.rows.find((r) => r.chat.id === menu.m!.conv);
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
            data-chat={r.chat.id} data-state={r.state} title={`${r.chat.title} · ${stateWords(r)} · ${onWords(r.chat)}`} aria-current={r.chat.id === props.on || undefined} onClick={() => props.open(r.chat.id)}
            data-drag-handle="" onPointerDown={props.arm ? (e) => props.arm!(e, r.chat.id, r.chat.title) : undefined}
            {...(props.acts ? menu.on(r.chat.id) : {})} aria-haspopup={props.acts ? "menu" : undefined}>
            <span className="wsc-row-in">
              <ChatDot state={r.state} />
              <span className="wsc-title">{r.chat.title}</span>
              {r.unread > 0 && <span className="wsc-unread" data-testid="ws-chat-unread" title={unreadWords(r.unread)} aria-label={unreadWords(r.unread)}>{r.unread}</span>}
              <span className={"wsc-age" + (age.strong ? " strong" : "")}>{age.text}</span>
            </span>
          </button>
        );
      })}
      {mrow && props.acts && menu.m && <ChatRowMenu x={menu.m.x} y={menu.m.y} host={props.acts} row={mrow} rows={props.rows} onClose={menu.close} />}
      {rest && <button className="wsc-all" data-testid="ws-chats-all" data-track="nav.workspace.chats" onClick={props.all}>All chats · {props.total} ▸</button>}
    </div>
  );
}

/** Which projects a chat reads, and why not the others. */
function Reads({ chat, projects, documents, limits }: { chat: WsChat; projects: readonly ProjectRead[]; documents: readonly DocRead[]; limits?: string }) {
  return (
    <>
    {chat.access === "stale" && (
      <div className="wsc-stale" data-testid="ws-chat-stale">
        <p>What it may read has changed since it started:</p>
        <ul>{(chat.access_changes ?? []).map((c, i) => <li key={i} title={c.path || undefined}>{accessWords(c)}</li>)}</ul>
        <p>It keeps the rules it started with. Close and start a new chat to apply the new ones.</p>
      </div>
    )}
    {chat.access === "unknown" && <p className="wsc-stale" data-testid="ws-chat-access-unknown">Whether its rules are still current cannot be told. Close and start a new chat to be sure.</p>}
    <ul className="wsc-reads" data-testid="ws-chat-reads">
      {projects.length === 0 && <li>This workspace has no projects.</li>}
      {projects.map((p) => (
        <li key={p.root} data-reads={p.reads}>
          <span className="wsc-reads-n" title={p.root}>{p.name ?? p.root.slice(p.root.lastIndexOf("/") + 1)}</span>
          <span className="wsc-reads-w">{p.reads === "yes" ? "read" : `not read: ${p.why ?? "it may not be"}`}</span>
        </li>
      ))}
      {documents.map((d) => (
        <li key={`${d.kind}${d.id}`} data-testid="ws-chat-reads-doc" data-reads={d.reads}>
          <span className="wsc-reads-n" title={d.path ?? undefined}>{d.title ?? (d.kind === "plan" ? "its plan" : "a linked idea")}</span>
          <span className="wsc-reads-w">{d.kind} · {d.reads === "yes" ? "read" : `not read: ${d.why ?? "it may not be"}`}</span>
        </li>
      ))}
    </ul>
    {limits && <p className="wsc-limits" data-testid="ws-chat-limits">{limits}</p>}
    </>
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
  /** A fold landed, or a close left changes: its commit. */
  landed(commit: string): void;
  /** This workspace's memory, as a fold's receipt and its failure reach it. */
  showChange(commit: string): void;
  openMemory(path: string | null): void;
  /** How a memory file is called in a sentence. */
  nameOf(path: string): string;
  /** What is open of the workspace's documents is saved: asked before any act. */
  ready(): Promise<boolean>;
  memoryTick: number;
  memoryChanged(): void;
  onError(msg: string): void;
  /** Why this chat has no worktrees tools (`mcpWords`), and the way to set
   *  the server up; null when it has them. */
  mcp?: { words: string; setUp(): void } | null;
}) {
  const { ws, info, conv, live } = props;
  const row = props.rows.find((r) => r.chat.id === conv) ?? null;
  const c: WsChat | null = row?.chat ?? null;
  const [view, setView] = useState<"chat" | "terminal">("chat");
  const [menu, setMenu] = useState<{ kind: "chats" | "reads" | "close"; x: number; y: number } | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  /** The engine's own sentence, when it would not fold. */
  const [said, setSaid] = useState<string | null>(null);
  /** The session under an attached terminal was replaced: attach again. */
  const [attach, setAttach] = useState(0);
  useEffect(() => { setMenu(null); setRenaming(null); setView("chat"); setSaid(null); }, [conv]);
  const key = wsChatKey({ id: ws.id, conv });
  const data = useChat(key).turns.data;
  const turns = data?.turns, reason = data?.reason;
  const acts = useChatActs(
    { ws: ws.id, live, ready: props.ready, onError: props.onError, landed: props.landed, gone: () => props.away() },
    row, props.rows, transcriptGone(reason), { said: setSaid, reattach: () => setAttach((n) => n + 1) });
  const { busy, arm, running, act, fold, close } = acts;

  // A fold of this chat that lands while it is on screen: its diff is shown
  // beside it (App says when not to).
  const change = useFoldChange(ws.id, c, props.memoryTick);
  const ran = useRef<{ conv: string; asked?: string } | null>(null);
  const commit = c?.fold?.state === "done" ? c.fold.commit : undefined;
  useEffect(() => {
    if (running) { ran.current = { conv, asked: c?.fold?.asked }; return; }
    if (ran.current?.conv === conv && commit) props.landed(commit);
    if (ran.current?.conv === conv && c?.fold?.state !== "folding") ran.current = null;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [running, commit, conv]);

  // A chat nobody has named takes its name from the person's first message.
  const named = useRef(new Set<string>());
  const first = c && isDefaultTitle(c.title) && turns ? firstMessage(turns) : null;
  useEffect(() => {
    const t = first && titleFrom(first);
    if (!t || named.current.has(conv)) return;
    named.current.add(conv);
    live.rename(ws.id, conv, t).catch((e) => {
      // Naming it was this window's idea: the engine's no to its own words is no news.
      if (!/what the app told the chat/.test(String(e))) props.onError(`Could not name the chat: ${String(e)}`);
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [first, conv]);

  if (!c || !row) {
    return (
      <div className="wsc-col" data-testid="ws-chat-col">
        <div className="wsc-gone"><p>This chat is not here any more.</p><button className="ctrl sm" onClick={props.away}>Back to the documents</button></div>
      </div>
    );
  }
  const at = (kind: "chats" | "reads" | "close") => (e: React.MouseEvent) => {
    const r = e.currentTarget.getBoundingClientRect();
    // What it reads, as it is now.
    if (kind === "reads") void live.refresh(ws.id);
    setMenu((m) => (m?.kind === kind ? null : { kind, x: kind === "close" ? r.right - 180 : r.left, y: r.bottom + 4 }));
  };
  const now = foldOf(c, live.sessions);
  const ended = endedLine(c, transcriptGone(reason));
  const closeItem = acts.item("close"), foldNow = acts.item("fold-now"), resumeItem = acts.item("resume");
  const rename = (t: string) => {
    setRenaming(null);
    if (t.trim() && t.trim() !== c.title) void act("rename it", () => live.rename(ws.id, conv, t.trim()), false);
  };
  const term = view === "terminal" && row.live;
  const model = modelWords(c, live.sessions, row.live);
  const who = isHarness(c.harness) ? HARNESS_LABEL[c.harness] : c.harness;
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
        {model && <span className="agent-model wsc-model" data-testid="ws-chat-model" title={modelOf(c, live.sessions, row.live) ? "the model this chat last replied with" : "the model this chat was started on"}>{who} {model}</span>}
        <button className="wsc-readsb" data-testid="ws-chat-readsb" data-track="workspace.chat.reads" title="Which projects this chat reads" aria-expanded={menu?.kind === "reads"} onClick={at("reads")}>{readsWords(info.projects)} ▾</button>
        {c.access === "stale" && <button className="wsc-stale-b" data-testid="ws-chat-stale-chip" title="What it may read has changed since it started" onClick={at("reads")}>its rules changed</button>}
        {props.mcp && <button className="wsc-stale-b" data-testid="ws-chat-no-mcp" title={props.mcp.words} onClick={props.mcp.setUp}>no worktrees tools · Set up</button>}
        <span className="wsc-acts">
          {row.live && (
            <span className="agent-view" role="group" aria-label="View">
              <button className={"agent-view-b" + (term ? "" : " on")} aria-pressed={!term} data-testid="ws-chat-view-chat" onClick={() => setView("chat")}>Chat</button>
              <button className={"agent-view-b" + (term ? " on" : "")} aria-pressed={term} data-testid="ws-chat-view-terminal" onClick={() => setView("terminal")}>Terminal</button>
            </span>
          )}
          {row.live ? (
            <span className="wsc-close">
              <button className="ctrl sm" data-testid="ws-chat-close" data-track="workspace.chat.fold" disabled={busy || !!closeItem?.why} title={closeItem?.why ?? closeItem?.title} onClick={() => void fold()}>Close</button>
              <button className="ctrl sm" data-testid="ws-chat-close-menu" aria-label="More" aria-haspopup="menu" aria-expanded={menu?.kind === "close"} onClick={at("close")}>▾</button>
            </span>
          ) : <button className="ctrl sm" data-testid="ws-chat-close-menu" aria-label="More" aria-haspopup="menu" aria-expanded={menu?.kind === "close"} onClick={at("close")}>▾</button>}
        </span>
      </header>
      {said && (
        <div className="wsc-said" role="alert" data-testid="ws-chat-said">
          <span>{said.replace(/^./, (x) => x.toUpperCase()).replace(/([^.!?])$/, "$1.")}</span>
          {row.state === "busy" && <button className="ctrl sm" data-testid="ws-chat-said-stop" onClick={() => { setSaid(null); void stopTurn({ repo: "", slug: "", path: key, session: c.session, ws: { id: ws.id, conv } }, "Claude"); }}>Stop</button>}
          <button className="ctrl sm" onClick={() => setSaid(null)}>Dismiss</button>
        </div>
      )}
      {now?.state === "failed" && (
        <div className="wsc-said failed" role="alert" data-testid="ws-chat-fold-failed">
          <span>{failedWords(now)}</span>
          {!!now.left && <button className="wsf-link" data-testid="ws-chat-fold-see" onClick={() => props.openMemory(null)}>See memory</button>}
          <button className="ctrl sm" data-testid="ws-chat-fold-again" disabled={busy} onClick={() => void fold()}>Try again</button>
          <button className="ctrl sm" data-testid="ws-chat-fold-none" disabled={busy} onClick={close}>Close without folding</button>
        </div>
      )}
      <div className="wsc-body">
        {/* The terminal is drawn instead of the chat, never under it: an attach
            that nobody sees would hold the session to a size nobody chose. */}
        {term ? (
          <div className="wsc-term" data-testid="ws-chat-term">
            <DeskTerminalPane key={`${c.session}:${props.gen}:${attach}`} session={c.session} desk={{ ws: { id: ws.id, conv } }} focusToken={1} />
          </div>
        ) : (
          <ChatPane key={conv} repo="" slug="" path={key} root={info.folder} ws={{ id: ws.id, conv }} provider="claude" session={c.session}
            activity={row.state === "busy" ? "busy" : row.state === "waiting" ? "waiting" : ""}
            doneAt={activeAt(c)} paneDraft={null} uiRem={props.uiRem} to="this chat" quiet={quietLine}
            promptNote={(lines) => (asksTrust(lines) ? <p className="wsc-trust" data-testid="ws-chat-trust">{TRUST_WORDS}</p> : null)}
            intro={<>A chat of <b>{ws.name}</b>. {introWords(info.projects, info.limits)}</>}
            onTerminal={() => setView("terminal")} onOpenPath={() => {}} reloadToken={0} onError={props.onError}
            insert={c.fold?.state === "done" && c.fold.commit && !running ? {
              at: change?.at ?? Number.MAX_SAFE_INTEGER,
              node: <Receipt ws={ws.id} chat={c} change={change} show={props.showChange} note={chatNote(c)} openNote={props.openMemory} nameOf={props.nameOf} ready={props.ready} changed={() => { props.memoryChanged(); void live.refresh(ws.id); }} />,
            } : null}
            readOnly={running && now ? (
              <div className="wsc-ended" data-testid="ws-chat-folding" data-fold={now.state} role="status">
                <span>{now.state === "checking" ? "Checking the fold…" : now.state === "waiting" ? "The fold waits on your answer. Its clock has stopped." : "Folding into memory…"}</span>
                <button className="ctrl sm" data-testid="ws-chat-fold-term" onClick={() => setView("terminal")}>Show the terminal</button>
                <button className="ctrl sm" data-testid="ws-chat-fold-stop" data-track="workspace.chat.fold-stop" disabled={busy} title="End it. What it wrote so far is kept, under its own change." onClick={close}>Stop the fold</button>
              </div>
            ) : row.live ? undefined : (
              <div className="wsc-ended" data-testid="ws-chat-ended">
                <span>{ended.text} Started {whenWords(c.started)}.</span>
                {ended.foldNow && foldNow && <button className="ctrl sm" data-testid="ws-chat-fold-now" data-track="workspace.chat.fold" disabled={busy || !!foldNow.why} title={foldNow.why ?? foldNow.title} onClick={() => void fold()}>Fold now</button>}
                {ended.resume && resumeItem && <button className="enter-btn sm" data-testid="ws-chat-resume" data-track="workspace.chat.resume" disabled={busy || !!resumeItem.why} title={resumeItem.why ?? undefined} onClick={acts.resume}>Resume</button>}
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
      {menu?.kind === "reads" && <CtxMenu x={menu.x} y={menu.y} panel="What this chat reads" onClose={() => setMenu(null)}><Reads chat={c} projects={info.projects} documents={info.documents ?? []} limits={info.limits} /></CtxMenu>}
      {menu?.kind === "close" && (
        <CtxMenu x={menu.x} y={menu.y} onClose={() => { setMenu(null); arm.disarm(); }}>
          {row.live && (
            <button className={"pop-item" + (arm.armed === "close" ? " danger armed" : "")} data-testid="ws-chat-close-item" data-track="workspace.chat.close" title={acts.item("close-none")?.title}
              onClick={() => { if (row.state !== "busy" || arm.armed === "close") setMenu(null); acts.closeAsk(); }}>
              {arm.armed === "close" ? "It is working. Close?" : "Close without folding"}
            </button>
          )}
          {row.live && props.terminalCmd.trim() && <button className="pop-item" data-testid="ws-chat-terminal-app" data-track="workspace.chat.terminal-app" onClick={() => { setMenu(null); props.openTerminalApp(c.session); }}>Open in terminal app</button>}
          <button className={"pop-item danger" + (arm.armed === "delete" ? " armed" : "")} data-testid="ws-chat-delete" data-track="workspace.chat.delete"
            title={acts.item("delete")?.title} onClick={() => acts.remove(() => setMenu(null))}>
            {arm.armed === "delete" ? "Delete this chat?" : "Delete"}
          </button>
        </CtxMenu>
      )}
    </div>
  );
}

/** What a fold left, in its chat for good: how much it changed, and the way to
 *  see it, undo it and put it back. */
function Receipt(props: { ws: string; chat: WsChat; change: MemoryChange | null; show(commit: string): void; note: string | null; openNote(path: string | null): void; nameOf(path: string): string; ready(): Promise<boolean>; changed(): void }) {
  const { chat } = props;
  const f = chat.fold!;
  const { change } = props;
  const rv = useRevert(props.ws, props.changed, props.nameOf, props.ready);
  return (
    <div className={"wsc-receipt" + (change?.reverted_by ? " undone" : "")} data-testid="ws-chat-receipt" data-commit={f.commit}>
      <p><b>{change?.reverted_by ? "Folded into memory, and undone" : "Folded into memory"}</b> <span className="wsm-change-d">{change ? whenWords(new Date(change.at * 1000).toISOString()) : ""} · {f.files ?? 0} file{f.files === 1 ? "" : "s"}{f.lines?.added ? ` +${f.lines.added}` : ""}{f.lines?.removed ? ` −${f.lines.removed}` : ""}</span></p>
      <div className="ws-ov-line">
        <button className="ctrl sm" data-testid="ws-receipt-diff" onClick={() => props.show(f.commit!)}>Show the diff</button>
        {props.note && <button className="ctrl sm" data-testid="ws-receipt-note" title={props.note} onClick={() => props.openNote(props.note)}>Open the note</button>}
        {change && <RevertButton c={change} rv={rv} what="this fold" />}
      </div>
      {rv.said && <p className="wsm-err" role="alert" data-testid="memory-conflict">{rv.said.text}</p>}
    </div>
  );
}
/** The change a chat's last fold made: when it landed, and what became of it. */
function useFoldChange(ws: string, c: WsChat | null, tick: number): MemoryChange | null {
  const [change, setChange] = useState<MemoryChange | null>(null);
  const commit = c?.fold?.state === "done" ? c.fold.commit : undefined;
  useEffect(() => {
    if (!commit) { setChange(null); return; }
    let gone = false;
    memoryDocs(ws).change(commit).then((x) => { if (!gone) setChange(x); }, () => { if (!gone) setChange(null); });
    return () => { gone = true; };
  }, [ws, commit, c?.fold?.reverted, tick]);
  return change && change.commit === commit ? change : null;
}

/** New chat's ▾: what a new chat starts on, the harness and its model, as a
 *  new lane's dialog offers them. Every harness is listed; one that cannot be a
 *  workspace chat, or is not installed, is disabled with the engine's reason
 *  (`workspacechats::harness_picks`). New chat's plain press starts the
 *  workspace's last pick with no step. */
export function NewChatMore(props: { picks: readonly HarnessPick[] | undefined; value: ChatPick; busy: boolean; start(p: ChatPick): void; onError(e: unknown): void; testid: string }) {
  const [at, setAt] = useState<{ x: number; y: number } | null>(null);
  return (
    <>
      <button className="ctrl sm wsc-more" data-testid={props.testid} data-track="workspace.chat.new-with" aria-label="New chat with…" title="New chat with…" aria-haspopup="dialog" aria-expanded={!!at} disabled={props.busy}
        onClick={(e) => { const r = e.currentTarget.getBoundingClientRect(); setAt((a) => (a ? null : { x: r.right - 260, y: r.bottom + 4 })); }}>▾</button>
      {at && <CtxMenu x={at.x} y={at.y} panel="New chat with" onClose={() => setAt(null)}>
        <PickPanel picks={props.picks} value={props.value} onError={props.onError} start={(p) => { setAt(null); props.start(p); }} />
      </CtxMenu>}
    </>
  );
}
function PickPanel(props: { picks: readonly HarnessPick[] | undefined; value: ChatPick; start(p: ChatPick): void; onError(e: unknown): void }) {
  const picks = props.picks ?? [{ id: "claude", label: "Claude" }];
  const [h, setH] = useState(props.value.harness);
  const [m, setM] = useState(props.value.model);
  const harness: Harness = isHarness(h) ? h : "claude";
  const models = useAgentModels(harness, props.onError);
  const refused = picks.filter((p) => p.why);
  return (
    <div className="wsc-pick" data-testid="ws-chat-pick">
      <div className="pop-head">New chat with</div>
      <div className="seg" role="group" aria-label="Agent provider">
        {picks.map((p) => (
          <button key={p.id} type="button" data-testid="ws-chat-pick-harness" data-harness={p.id} className={h === p.id ? "on" : ""} aria-pressed={h === p.id}
            disabled={!!p.why} title={p.why} onClick={() => { setH(p.id); setM(""); }}>{p.label}</button>
        ))}
      </div>
      <ModelPicker harness={harness} value={m} options={models} testid="ws-chat-pick-model" onChange={setM} />
      {refused.length > 0 && <ul className="wsc-pick-why" data-testid="ws-chat-pick-why">{refused.map((p) => <li key={p.id}>{p.why}</li>)}</ul>}
      <div className="ws-ov-line"><button className="enter-btn sm" data-testid="ws-chat-pick-start" data-track="workspace.chat.new-with.start" onClick={() => props.start({ harness: h, model: m })}>Start</button></div>
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
export function ChatsTab(props: { note(path: string): void; rows: readonly ChatRow[]; on: string | null; starting: boolean; ago(epoch: number): string; open(conv: string): void; start(): void; acts?: ChatActsHost;
  /** New chat's ▾. */
  more?: React.ReactNode }) {
  const [q, setQ] = useState("");
  const menu = useRowMenu();
  const mrow = menu.m && props.rows.find((r) => r.chat.id === menu.m!.conv);
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
        <button className="enter-btn sm" data-testid="ws-chats-new" data-track="workspace.chat.new" disabled={props.starting} onClick={props.start}>New chat</button>{props.more}
      </div>
      {hit.length === 0 && <p className="ws-ov-none">No chat's title matches.</p>}
      {groups.map((g) => (
        <section key={g.name} aria-label={g.name}>
          <div className="wsf-card-name">{g.name}</div>
          <ul className="wsc-list">
            {g.rows.map((r) => (
              <li key={r.chat.id} className={"wsc-item" + (r.chat.id === props.on ? " on" : "")} data-testid="ws-chats-row" data-chat={r.chat.id} data-state={r.state}>
                <button className="wsc-item-b" onClick={() => props.open(r.chat.id)} {...(props.acts ? menu.on(r.chat.id) : {})} aria-haspopup={props.acts ? "menu" : undefined}>
                  <span className="wsc-line-in">
                    <ChatDot state={r.state} />
                    <span className="ws-ov-name">{r.chat.title}</span>
                    <span className="ws-ov-detail">{stateWords(r)}</span>
                    {r.chat.model && <span className="ws-ov-detail" data-testid="ws-chats-on">{onWords(r.chat)}</span>}
                  </span>
                </button>
                {chatNote(r.chat) && <button className="wsf-link" data-testid="ws-chats-note" title={r.chat.note} onClick={() => props.note(chatNote(r.chat)!)}>note</button>}
                <span className="wsc-when" title={`Last active ${whenWords(r.chat.last_activity || r.chat.started)}`}>started {whenWords(r.chat.started)}{props.ago(activeAt(r.chat)) ? ` · active ${props.ago(activeAt(r.chat))}` : ""}</span>
              </li>
            ))}
          </ul>
        </section>
      ))}
      {mrow && props.acts && menu.m && <ChatRowMenu x={menu.m.x} y={menu.m.y} host={props.acts} row={mrow} rows={props.rows} onClose={menu.close} />}
    </div>
  );
}
