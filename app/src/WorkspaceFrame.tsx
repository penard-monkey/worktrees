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
import { useState, type ReactNode } from "react";
import * as Icons from "./icons";
import { WS_TAB_LABEL, type WsTab } from "./navHistory";
import { LinkIdea, NoChats, NoMemory, NoPlan, PlanProblems, PlanProjects, WorkList, type WsParts } from "./WorkspaceOverview";
import { useTier } from "./doc/useTier";

/** From this much room the Ideas tab has its list beside the document, and from
 *  this much the Plan tab has its lists there and the Overview is two columns.
 *  Under it a list folds to a line above the document. */
const ROOM = [720, 900] as const;

/** A document that was floated: where it went, in its place. */
export type Floated = { title: string; show(): void; here(): void };

const TAB_ICON: Record<WsTab, ReactNode> = {
  overview: null, memory: <Icons.BookText size={13} />, plan: <Icons.ListChecks size={13} />, ideas: <Icons.Lightbulb size={13} />, chats: <Icons.MessageSquare size={13} />,
};

/** The documents row in a workspace's nav card. A dashed chip has nothing
 *  behind it and still opens its tab. At the narrowest nav the icons go
 *  (App.css, by the row's own width). */
export function DocChips(props: {
  chips: readonly { tab: "memory" | "plan" | "ideas"; has: boolean; count?: number; on: boolean; floated: boolean; waits: boolean }[];
  open(tab: "memory" | "plan" | "ideas"): void;
}) {
  return (
    <div className="wsf-chips" data-testid="ws-docs">
      {props.chips.map((c) => {
        const title = c.floated ? `${WS_TAB_LABEL[c.tab]}: in the Ideas window`
          : c.waits ? `${WS_TAB_LABEL[c.tab]}: text that is not saved`
          : c.has ? `Open ${WS_TAB_LABEL[c.tab]}` : c.tab === "memory" ? "No memory yet" : c.tab === "plan" ? "No plan" : "No linked ideas";
        const inner = (
          <>
            <span className="wsf-chip-i" aria-hidden="true">{TAB_ICON[c.tab]}</span>
            {WS_TAB_LABEL[c.tab]}
            {c.count ? <span className="wsf-chip-n">{c.count}</span> : null}
            {c.waits && <span className="wsf-chip-dot" aria-hidden="true" />}
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
  /** The chat, when one is open (a later slice): the middle column. */
  middle?: ReactNode;
  dockLeft: boolean;
}) {
  const { ws, tab, parts } = props;
  const [docs, setDocs] = useState<HTMLElement | null>(null);
  const room = useTier(docs, ROOM);
  const beside = room === 2 || (room === 1 && tab === "ideas");
  const [sideOpen, setSideOpen] = useState(false);
  const pane = tab === "plan" || tab === "ideas";
  const side = !ws ? null
    : tab === "plan" && props.has ? <PlanSide {...parts} />
    : tab === "ideas" && (props.has || props.docShown) ? <IdeasSide {...parts} openId={props.openId} openTitle={props.openTitle} /> : null;
  const sideLine = tab === "plan"
    ? `${parts.ov?.projects?.length ?? 0} projects · ${parts.ov?.work?.length ?? 0} work items`
    : `${parts.ov?.ideas.length ?? 0} ideas`;
  return (
    <section className="wsf" data-testid="ws-frame" data-ws={ws?.id} data-ws-color={ws?.color ?? undefined} hidden={!props.shown}>
      {ws && (
        <header className="topbar wsf-top">
          {props.navButtons}
          <span className="wsf-name" data-testid="ws-frame-name" title={ws.name}><Icons.Layers size={13} /><span className="wsf-name-t">{ws.name}</span></span>
          <span className="wsf-counts">{ws.counts}</span>
          <span className="controls">
            <button className="icon-btn" data-testid="ws-frame-menu" data-track="nav.workspace.menu" title="workspace menu" onClick={props.onMenu}><Icons.Ellipsis size={15} /></button>
          </span>
        </header>
      )}
      <div className="wsf-body" data-mid={props.middle ? "" : undefined} data-dock={props.dockLeft ? "left" : "right"}>
        {props.middle && <div className="wsf-mid">{props.middle}</div>}
        <div className="wsf-docs" ref={setDocs} data-room={room === 2 ? "wide" : "narrow"}>
          {ws && (
            <div className="wsf-tabs" role="tablist" aria-label="The workspace's documents">
              {props.tabs.map((t) => (
                <button key={t} role="tab" className={"wsf-tab" + (t === tab ? " on" : "")} aria-selected={t === tab} data-testid="ws-tab" data-tab={t} onClick={() => props.showTab(t)}>
                  {WS_TAB_LABEL[t]}{props.counts[t] ? <span className="wsf-tab-n">{props.counts[t]}</span> : null}
                </button>
              ))}
            </div>
          )}
          {ws && tab === "overview" && <div className="wsf-scroll">{props.overview}</div>}
          {ws && tab === "memory" && <div className="wsf-scroll"><NoMemory /></div>}
          {ws && tab === "chats" && <div className="wsf-scroll"><NoChats /></div>}
          <div className="wsf-split" hidden={!pane} data-beside={beside || undefined}>
            {side && !beside && (
              <button className="wsf-side-line" data-testid="ws-side-toggle" aria-expanded={sideOpen} onClick={() => setSideOpen((o) => !o)}>
                {sideOpen ? <Icons.ChevronDown size={11} /> : <Icons.ChevronRight size={11} />} {sideLine}
              </button>
            )}
            {side && (beside || sideOpen) && <aside className={"wsf-side " + tab} data-testid="ws-side">{side}</aside>}
            <div className="wsf-slot">
              {ws && pane && !props.docShown && (
                <div className="wsf-nodoc" data-testid="ws-nodoc">
                  {props.floated ? (
                    <div className="wsf-floated" data-testid="ws-floated">
                      <p>{props.floated.title} is in the Ideas window.</p>
                      <div className="ws-ov-line">
                        <button className="ctrl sm" data-testid="ws-floated-show" onClick={props.floated.show}>Show the window</button>
                        <button className="ctrl sm" data-testid="ws-floated-here" onClick={props.floated.here}>Open it here</button>
                      </div>
                    </div>
                  ) : tab === "plan" ? (
                    props.has
                      ? (parts.ov?.plan && !parts.ov.plan.live
                        ? <div className="wsf-empty"><p>The plan cannot be read: {parts.ov.plan.error ?? "it is gone"}.</p><button className="ctrl sm" onClick={() => parts.setPlan(null)}>Unlink it</button></div>
                        : <div className="wsf-empty"><p>The plan is not open.</p><button className="ctrl sm" data-testid="ws-open-tab" onClick={props.openTab}>Open it</button></div>)
                      : <div className="wsf-empty" data-testid="ws-no-plan"><NoPlan {...parts} /></div>
                  ) : props.has
                    ? <div className="wsf-empty"><p>Pick an idea to open it here.</p></div>
                    : <div className="wsf-empty" data-testid="ws-no-ideas"><div className="ws-ov-line"><span className="ws-ov-none">None linked.</span><LinkIdea {...parts} /></div></div>}
                </div>
              )}
              {props.doc}
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
