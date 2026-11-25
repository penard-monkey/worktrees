/** The workspace view (docs/proposals/workspace-view-design.md §2, §3): a top bar
 *  like a place's, then the documents as tabs. With a chat (a later slice) the
 *  chat takes the middle and the documents the dock's side; with none they take
 *  the whole room.
 *
 *  **The document's section keeps ONE parent for its life.** `doc` (the pane,
 *  doc/MainDocument.tsx) is always the last child of `.wsf-slot`, whatever tab
 *  or workspace is shown, and the frame itself is hidden, never unmounted, while
 *  a document is open: so a text that is not saved survives a nav click. Every
 *  conditional above it is one position in its parent for the same reason.
 *
 *  Drawn with Ideas OR workspace chats on (beta.ts); what each part needs is
 *  said by `tabs` and the two flags, which come from the parts' own helpers. */
import { useLayoutEffect, useState, type ReactNode } from "react";
import * as Icons from "./icons";
import { WS_TAB_LABEL, type WsTab } from "./navHistory";
import { LinkIdea, NoChats, NoPlan, PlanProblems, PlanProjects, WorkList, type WsParts } from "./WorkspaceOverview";
import { splitBounds, splitClamp } from "./wsview";
import { useTier } from "./doc/useTier";

/** From this much room the Ideas tab has its list beside the document, and from
 *  this much the Plan tab has its lists there and the Overview is two columns.
 *  Under it a list folds to a line above the document. */
const ROOM = [720, 900] as const;
/** Room for a chat (420) and the documents (340) both; under it, one of them. */
const BOTH = [760] as const;
const SPLIT_KEY = "wt.workspace.split";
const splitNow = (): number => {
  try { const v = Number(window.localStorage.getItem(SPLIT_KEY)); return v >= 0.2 && v <= 0.8 ? v : 0.46; } catch { return 0.46; }
};

/** A document that was floated: where it went, in its place. */
export type Floated = { title: string; show(): void; here(): void };

const TAB_ICON: Record<WsTab, ReactNode> = {
  overview: null, memory: <Icons.BookText size={13} />, plan: <Icons.ListChecks size={13} />, ideas: <Icons.Lightbulb size={13} />, chats: <Icons.MessageSquare size={13} />,
};

/** The documents row in a workspace's nav card. A dashed chip has nothing
 *  behind it and still opens its tab. At the narrowest nav the icons go
 *  (App.css, by the row's own width). */
export function DocChips(props: {
  chips: readonly { tab: "memory" | "plan" | "ideas"; has: boolean; count?: number; on: boolean; floated: boolean; waits: boolean; fresh?: boolean }[];
  open(tab: "memory" | "plan" | "ideas"): void;
}) {
  return (
    <div className="wsf-chips" data-testid="ws-docs">
      {props.chips.map((c) => {
        const title = c.floated ? `${WS_TAB_LABEL[c.tab]}: in the Ideas window`
          : c.waits ? `${WS_TAB_LABEL[c.tab]}: text that is not saved`
          : c.fresh ? "Memory changed since you looked"
          : c.has ? `Open ${WS_TAB_LABEL[c.tab]}` : c.tab === "memory" ? "No memory yet" : c.tab === "plan" ? "No plan" : "No linked ideas";
        const inner = (
          <>
            <span className="wsf-chip-i" aria-hidden="true">{TAB_ICON[c.tab]}</span>
            {WS_TAB_LABEL[c.tab]}
            {c.count ? <span className="wsf-chip-n">{c.count}</span> : null}
            {c.waits ? <span className="wsf-chip-dot" aria-hidden="true" /> : c.fresh ? <span className="wsf-chip-dot new" data-testid="memory-fresh" aria-hidden="true" /> : null}
            {c.floated && <span className="wsf-chip-f" aria-hidden="true"><Icons.ExternalLink size={10} /></span>}
          </>
        );
        const cls = "wsf-chip" + (c.on ? " on" : "") + (c.has ? "" : " none");
        const act = { className: cls, title, "aria-current": c.on || undefined, onClick: () => props.open(c.tab) };
        return c.tab === "memory" ? <button key={c.tab} data-testid="ws-doc-memory" data-track="nav.workspace.memory" {...act}>{inner}</button>
          : c.tab === "plan" ? <button key={c.tab} data-testid="ws-doc-plan" data-track="nav.workspace.plan" {...act}>{inner}</button>
          : <button key={c.tab} data-testid="ws-doc-ideas" data-track="nav.workspace.ideas" {...act}>{inner}</button>;
      })}
    </div>
  );
}

/** The Plan tab's side: what the reader could not take, its Projects and Work. */
function PlanSide(props: WsParts) {
  const { ov } = props;
  const plan = ov?.plan ?? null;
  return (
    <>
      <div className="wsf-side-h">
        <span>What the plan says</span>
        {plan && <button className="wsf-link" data-testid="ws-ov-plan-unlink" title="Unlink the plan (the plan stays)" onClick={() => props.setPlan(null)}>Unlink</button>}
      </div>
      {!ov ? <p className="ws-ov-none">{props.error ?? "Reading…"}</p> : !plan?.live ? null : (
        <>
          <PlanProblems ov={ov} />
          <div className="wsf-card-name">Projects</div>
          <PlanProjects {...props} ov={ov} />
          <div className="wsf-card-name">Work</div>
          <WorkList {...props} ov={ov} />
        </>
      )}
    </>
  );
}

/** The Ideas tab's side: the linked ideas, the one open marked, and one that is
 *  open and not linked (a followed link). */
function IdeasSide(props: WsParts & { openId: string | null; openTitle: string | null }) {
  const { ov } = props;
  const ids = ov?.ideas ?? [];
  const extra = props.openId && ov && !ids.some((i) => i.live === props.openId || i.id === props.openId) ? props.openId : null;
  return (
    <>
      {ov && ids.length === 0 && <p className="ws-ov-none">None linked.</p>}
      {!ov && <p className="ws-ov-none">{props.error ?? "Reading…"}</p>}
      <ul className="wsf-ideas">
        {ids.map((i) => (
          <li key={i.id} className={"wsf-idea" + (i.live && i.live === props.openId ? " on" : "")} data-testid="ws-idea-row" data-idea={i.id}>
            {i.live
              ? <button className="wsf-idea-open" data-drag-handle="" title="Open it; drag it onto another workspace to link it there too" aria-current={i.live === props.openId || undefined}
                  onPointerDown={(e) => props.arm(e, { id: i.live!, title: i.title ?? "", kind: "idea" })}
                  onClick={() => props.openDoc(i.live!)}>{i.title || "Untitled"}{i.merged && <span className="ws-ov-merged" title="This id was merged into it">merged</span>}</button>
              : <span className="ws-ov-gone">an idea that is gone</span>}
            <button className="ws-ov-x" title="Unlink it (the idea stays)" data-testid="ws-ov-idea-unlink" onClick={() => props.unlinkIdea(i.id)}>×</button>
          </li>
        ))}
        {extra && (
          <li className="wsf-idea on" data-testid="ws-idea-row" data-idea={extra} data-unlinked="">
            <span className="wsf-idea-open" aria-current>{props.openTitle || "Untitled"}</span>
            <button className="wsf-link" data-testid="ws-idea-link" title="Link it to this workspace" onClick={() => props.linkIdea(extra)}>Link</button>
          </li>
        )}
      </ul>
      <div className="ws-ov-line"><LinkIdea {...props} /></div>
    </>
  );
}

export function WorkspaceFrame(props: {
  shown: boolean;
  /** The workspace shown. `null` while only the document waits here, hidden. */
  ws: { id: string; name: string; color: string | null; counts: string } | null;
  tabs: readonly WsTab[];
  tab: WsTab;
  counts: Partial<Record<WsTab, number>>;
  showTab(tab: WsTab): void;
  navButtons: ReactNode;
  onMenu(e: React.MouseEvent): void;
  parts: WsParts;
  overview: ReactNode;
  /** The document the tab would show: none behind it, the one floated away, or
   *  the pane's. */
  has: boolean;
  floated: Floated | null;
  openId: string | null;
  openTitle: string | null;
  /** The pane is showing a document for this tab. */
  docShown: boolean;
  /** Put the tab's document in the pane (it was closed there). */
  openTab(): void;
  doc: ReactNode;
  /** The Memory tab: its list, how many files it has, and what stands in the
   *  document's place (no memory, one that cannot be read, an image). */
  memory: { files: ReactNode; count: number; instead: ReactNode };
  /** The chat, when one is open (or being started): the middle column. */
  middle?: ReactNode;
  /** Which side is shown where there is room for one. */
  side: "chat" | "docs";
  setSide(side: "chat" | "docs"): void;
  /** In the bar, once there are chats. */
  newChat: { busy: boolean; run(): void } | null;
  firstChat(): void;
  chatsTab: ReactNode;
  dockLeft: boolean;
}) {
  const { ws, tab, parts } = props;
  const [docs, setDocs] = useState<HTMLElement | null>(null);
  const [body, setBody] = useState<HTMLElement | null>(null);
  const both = useTier(body, BOTH) === 1;
  const one = props.middle && !both ? props.side : undefined;
  // The documents' share of the room beside a chat, this machine's own.
  const [wanted, setShare] = useState(splitNow);
  const [width, setWidth] = useState(0);
  useLayoutEffect(() => {
    if (!body) return;
    const look = () => { const w = body.getBoundingClientRect().width; if (w > 0) setWidth(w); };
    look();
    const ro = new ResizeObserver(look);
    ro.observe(body);
    return () => ro.disconnect();
  }, [body]);
  // What is drawn, said and kept: never past a floor, whoever moved it.
  const share = splitClamp(wanted, width), bounds = splitBounds(width);
  const drag = (x: number) => {
    const r = body?.getBoundingClientRect();
    if (!r?.width) return;
    const w = props.dockLeft ? x - r.left : r.right - x;
    setShare(splitClamp(w / r.width, r.width));
  };
  const dropped = () => { try { window.localStorage.setItem(SPLIT_KEY, String(share)); } catch { /* it starts at its default */ } };
  const room = useTier(docs, ROOM);
  const beside = room === 2 || (room === 1 && tab !== "plan");
  const panel = (t: WsTab) => ({ role: "tabpanel", id: `wsf-panel-${t}`, "aria-labelledby": `wsf-tab-${t}` });
  const [sideOpen, setSideOpen] = useState(false);
  const pane = tab === "plan" || tab === "ideas" || tab === "memory";
  const side = !ws ? null
    : tab === "plan" && props.has ? <PlanSide {...parts} />
    : tab === "memory" ? props.memory.files
    : tab === "ideas" && (props.has || props.docShown) ? <IdeasSide {...parts} openId={props.openId} openTitle={props.openTitle} /> : null;
  const sideLine = tab === "plan"
    ? `${parts.ov?.projects?.length ?? 0} projects · ${parts.ov?.work?.length ?? 0} work items`
    : tab === "memory" ? `Files · ${props.memory.count}` : `${parts.ov?.ideas.length ?? 0} ideas`;
  return (
    <section className="wsf" data-testid="ws-frame" data-ws={ws?.id} data-ws-color={ws?.color ?? undefined} hidden={!props.shown}>
      {ws && (
        <header className="topbar wsf-top">
          {props.navButtons}
          <span className="wsf-name" data-testid="ws-frame-name" title={ws.name}><Icons.Layers size={13} /><span className="wsf-name-t">{ws.name}</span></span>
          <span className="wsf-counts">{ws.counts}</span>
          <span className="controls">
            {one && (
              <span className="agent-view" role="group" aria-label="Side">
                <button className={"agent-view-b" + (one === "chat" ? " on" : "")} aria-pressed={one === "chat"} data-testid="ws-side-chat" onClick={() => props.setSide("chat")}>Chat</button>
                <button className={"agent-view-b" + (one === "docs" ? " on" : "")} aria-pressed={one === "docs"} data-testid="ws-side-docs" onClick={() => props.setSide("docs")}>Documents</button>
              </span>
            )}
            {props.newChat && <button className="enter-btn sm" data-testid="ws-frame-new-chat" data-track="workspace.chat.new" disabled={props.newChat.busy} onClick={props.newChat.run}>New chat</button>}
            <button className="icon-btn" data-testid="ws-frame-menu" data-track="nav.workspace.menu" title="workspace menu" onClick={props.onMenu}><Icons.Ellipsis size={15} /></button>
          </span>
        </header>
      )}
      <div className="wsf-body" ref={setBody} data-mid={props.middle ? "" : undefined} data-one={one} data-dock={props.dockLeft ? "left" : "right"}>
        {props.middle && <div className="wsf-mid" data-testid="ws-mid">{props.middle}</div>}
        {props.middle && !one && (
          <div className="wsf-split-bar" role="separator" aria-orientation="vertical" aria-label="Documents width" tabIndex={0} data-testid="ws-split"
            aria-valuenow={Math.round(share * 100)} aria-valuemin={Math.floor(bounds.min * 100)} aria-valuemax={Math.ceil(bounds.max * 100)}
            onPointerDown={(e) => { e.preventDefault(); e.currentTarget.setPointerCapture(e.pointerId); }}
            onPointerMove={(e) => { if (e.currentTarget.hasPointerCapture(e.pointerId)) drag(e.clientX); }}
            onPointerUp={dropped}
            onKeyDown={(e) => {
              const by = (e.key === "ArrowLeft" ? 0.02 : e.key === "ArrowRight" ? -0.02 : 0) * (props.dockLeft ? -1 : 1);
              if (!by) return;
              e.preventDefault();
              setShare(splitClamp(share + by, width));
            }} onKeyUp={dropped} />
        )}
        <div className="wsf-docs" ref={setDocs} style={props.middle && !one ? { flex: `0 0 ${share * 100}%` } : undefined} data-room={room === 2 ? "wide" : "narrow"}>
          {ws && (
            <div className="wsf-tabs" role="tablist" aria-label="The workspace's documents" onKeyDown={(e) => {
              const all = [...e.currentTarget.querySelectorAll<HTMLElement>("[role=tab]")], at = all.indexOf(document.activeElement as HTMLElement);
              const to = e.key === "ArrowRight" ? at + 1 : e.key === "ArrowLeft" ? at - 1 : e.key === "Home" ? 0 : e.key === "End" ? all.length - 1 : null;
              if (to === null || at < 0 || e.metaKey || e.ctrlKey || e.altKey) return;
              e.preventDefault();
              // The focus alone: opening a tab leaves a document, so Enter does that.
              all[(to + all.length) % all.length].focus();
            }}>
              {props.tabs.map((t) => (
                <button key={t} role="tab" id={`wsf-tab-${t}`} aria-controls={t === tab ? `wsf-panel-${t}` : undefined} tabIndex={t === tab ? 0 : -1} className={"wsf-tab" + (t === tab ? " on" : "")} aria-selected={t === tab} data-testid="ws-tab" data-tab={t} onClick={() => props.showTab(t)}>
                  {WS_TAB_LABEL[t]}{props.counts[t] ? <span className="wsf-tab-n">{props.counts[t]}</span> : null}
                </button>
              ))}
            </div>
          )}
          {ws && tab === "overview" && <div className="wsf-scroll" {...panel("overview")}>{props.overview}</div>}
          {ws && tab === "chats" && <div className="wsf-scroll" {...panel("chats")}>{props.counts.chats ? props.chatsTab : <NoChats newChat={props.firstChat} />}</div>}
          <div className="wsf-split" {...(ws && pane ? panel(tab) : {})} hidden={!pane} data-beside={beside || undefined}>
            {side && !beside && (
              <button className="wsf-side-line" data-testid="ws-side-toggle" aria-expanded={sideOpen} onClick={() => setSideOpen((o) => !o)}>
                {sideOpen ? <Icons.ChevronDown size={11} /> : <Icons.ChevronRight size={11} />} {sideLine}
              </button>
            )}
            {side && (beside || sideOpen) && <aside className={"wsf-side " + tab} data-testid="ws-side">{side}</aside>}
            <div className="wsf-slot">
              {ws && pane && !props.docShown && (tab === "memory" && props.memory.instead ? props.memory.instead : (
                <div className="wsf-nodoc" data-testid="ws-nodoc">
                  {props.floated ? (
                    <div className="wsf-floated" data-testid="ws-floated">
                      <p>{props.floated.title} is in the Ideas window.</p>
                      <div className="ws-ov-line">
                        <button className="ctrl sm" data-testid="ws-floated-show" onClick={props.floated.show}>Show the window</button>
                        <button className="ctrl sm" data-testid="ws-floated-here" onClick={props.floated.here}>Open it here</button>
                      </div>
                    </div>
                  ) : tab === "memory" ? <div className="wsf-empty"><p>{props.has ? "Pick a file to open it here." : "Reading…"}</p>{props.has && <button className="ctrl sm" data-testid="ws-open-tab" onClick={props.openTab}>Open the index</button>}</div>
                  : tab === "plan" ? (
                    props.has
                      ? (parts.ov?.plan && !parts.ov.plan.live
                        ? <div className="wsf-empty"><p>The plan cannot be read: {parts.ov.plan.error ?? "it is gone"}.</p><button className="ctrl sm" onClick={() => parts.setPlan(null)}>Unlink it</button></div>
                        : <div className="wsf-empty"><p>The plan is not open.</p><button className="ctrl sm" data-testid="ws-open-tab" onClick={props.openTab}>Open it</button></div>)
                      : <div className="wsf-empty" data-testid="ws-no-plan"><NoPlan {...parts} /></div>
                  ) : props.has
                    ? <div className="wsf-empty"><p>Pick an idea to open it here.</p></div>
                    : <div className="wsf-empty" data-testid="ws-no-ideas"><div className="ws-ov-line"><span className="ws-ov-none">None linked.</span><LinkIdea {...parts} /></div></div>}
                </div>
              ))}
              {props.doc}
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
