import { useLayoutEffect, useState, type ReactNode } from "react";
import * as Icons from "./icons";
import type { WsTab } from "./navHistory";
import { LinkIdea, NoChats, NoPlan, PlanProblems, PlanProjects, WorkList, type WsParts } from "./WorkspaceOverview";
import { createPortal } from "react-dom";
import { useTier } from "./doc/useTier";

const ROOM = [900] as const;

/** A document that was floated: where it went, in its place. */
export type Floated = { title: string; show(): void; here(): void };

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

type WorkspaceLabel = { id: string; name: string; color: string | null; counts: string };

export function WorkspaceTopBar({ ws, navButtons, newChat, onMenu }: {
  ws: WorkspaceLabel;
  navButtons: ReactNode;
  newChat: { busy: boolean; run(): void; more?: ReactNode } | null;
  onMenu(e: React.MouseEvent): void;
}) {
  return (
    <header className="topbar wsf wsf-top" data-ws-color={ws.color ?? undefined}>
      {navButtons}
      <span className="wsf-name" data-testid="ws-frame-name" title={ws.name}><Icons.Layers size={13} /><span className="wsf-name-t">{ws.name}</span></span>
      <span className="wsf-counts">{ws.counts}</span>
      <span className="controls">
        {newChat && <button className="enter-btn sm" data-testid="ws-frame-new-chat" data-track="workspace.chat.new" disabled={newChat.busy} onClick={newChat.run}>New chat</button>}
        {newChat?.more}
        <button className="icon-btn" data-testid="ws-frame-menu" data-track="nav.workspace.menu" title="Workspace menu" onClick={onMenu}><Icons.Ellipsis size={15} /></button>
      </span>
    </header>
  );
}

/** A document stays in the same slot when the slot moves between the main
 *  pane and the dock. React keeps the editor, its draft and its scroll state. */
export function WorkspaceFrame(props: {
  shown: boolean;
  ws: WorkspaceLabel | null;
  tab: WsTab;
  dock: HTMLElement | null;
  parts: WsParts;
  overview: ReactNode;
  has: boolean;
  floated: Floated | null;
  openId: string | null;
  openTitle: string | null;
  docShown: boolean;
  openTab(): void;
  doc: ReactNode;
  memory: { files: ReactNode; instead: ReactNode };
  middle?: ReactNode;
  firstChat(): void;
  chatsTab: ReactNode;
}) {
  const { ws, tab, parts, dock, middle } = props;
  const [center, setCenter] = useState<HTMLDivElement | null>(null);
  const [slot] = useState(() => {
    const el = document.createElement("div");
    el.className = "wsf-slot";
    return el;
  });
  const pane = tab === "plan" || tab === "ideas" || tab === "memory";
  const side = !ws ? null
    : tab === "plan" && props.has ? <PlanSide {...parts} />
    : tab === "memory" ? props.memory.files
    : tab === "ideas" && (props.has || props.docShown) ? <IdeasSide {...parts} openId={props.openId} openTitle={props.openTitle} /> : null;
  const dockHasBody = !!dock && (!!middle || (!side && tab !== "overview"));
  const host = dockHasBody ? dock : center;
  const room = useTier(host, ROOM);
  const hideContent = !props.shown || (!!middle && !dock);
  useLayoutEffect(() => () => slot.remove(), [slot]);
  useLayoutEffect(() => {
    if (host && slot.parentNode !== host) host.appendChild(slot);
    slot.hidden = hideContent;
  }, [host, slot, hideContent]);
  const sidePanel = side && <div className={"wsf-side " + tab} data-testid="ws-side">{side}</div>;
  return (
    <section className="wsf" data-testid="ws-frame" data-panel={tab} data-ws={ws?.id} data-ws-color={ws?.color ?? undefined} hidden={!props.shown}>
      {middle && <div className="wsf-mid" data-testid="ws-mid">{middle}</div>}
      {!middle && dockHasBody && tab === "chats" && <div className="wsf-scroll" data-room="narrow">{props.overview}</div>}
      <div className="wsf-center" ref={setCenter} hidden={!!middle || dockHasBody}>
      </div>
      {dock && !dockHasBody && sidePanel && createPortal(sidePanel, dock)}
      {createPortal(
        <>
          {(dockHasBody || (!dock && !middle)) && sidePanel}
          {ws && tab === "overview" && <div className="wsf-scroll" data-room={room ? "wide" : "narrow"}>{props.overview}</div>}
          {ws && tab === "chats" && <div className="wsf-scroll">{props.chatsTab || <NoChats newChat={props.firstChat} />}</div>}
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
        </>,
        slot,
      )}
    </section>
  );
}
