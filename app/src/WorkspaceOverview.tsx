/** What a workspace's view says about its plan, its ideas and its projects (idea
 *  board phase 4, §9.2; workspace-view-design.md §3.1): the Overview tab, and
 *  the parts the Plan and Ideas tabs share with it (WorkspaceFrame.tsx draws
 *  the tabs and the document).
 *
 *  The plan's Projects and Work are as core's reader answers them. A project to
 *  clone or create opens the Add project dialogs filled in; a work item is
 *  reviewed and started, or takes a place dropped from the nav (phase 5). The
 *  plan's text itself is edited in the document.
 *
 *  The plan and idea chips here are the same-window drag sources of §6.3: a plan
 *  with no workspace, dragged onto a workspace in the nav, becomes its plan; an
 *  idea dragged there is linked. Every drag has its menu path here too (Link a
 *  plan ▸, Link an idea ▸). The rules and calls are `plans.ts`'s. */
import { useState, type ReactNode } from "react";
import { STATE_WORDS, itemFree, itemPlace, problemLine, workStatus, type Activity, type Overview, type Snap, type WorkItem } from "./plans";
import type { Attention } from "./wsview";

export type DocLite = { id: string; title: string; kind: "idea" | "plan" };

/** The workspace's plan and ideas, and everything that can be done with them:
 *  what the Overview, Plan and Ideas tabs all take. */
export type WsParts = {
  ov: Overview | null;
  error: string | null;
  /** Plans no workspace has yet: what "Link a plan ▸" offers. */
  freePlans: DocLite[];
  /** Ideas this workspace does not link yet. */
  linkable: DocLite[];
  openDoc(id: string): void;
  setPlan(plan: string | null): void;
  /** A plan from the linked ideas, made this workspace's. `null`: none linked. */
  makePlan: (() => void) | null;
  linkIdea(id: string): void;
  /** A new idea, linked here and opened in Edit. */
  newIdea(): void;
  unlinkIdea(id: string): void;
  arm(e: React.PointerEvent, d: DocLite): void;
  snap: Snap;
  activity: Activity;
  gone: ReadonlySet<string>;
  /** The work item (its line) a dragged place is over. */
  workHot: number | null;
  bringProject(name: string): void;
  review(w: WorkItem): void;
  linkPlace(w: WorkItem): void;
  showPlace(root: string, slug: string): void;
};

/** One of the workspace's projects, as the Overview lists it. */
export type WsProjectRow = { root: string; name: string; registered: boolean; places: number; working: number; waiting: number };

/** "Link a plan ▸": the plans no workspace has. */
export function PlanPicker(props: Pick<WsParts, "freePlans" | "arm" | "setPlan"> & { picked(): void }) {
  return (
    <ul className="ws-ov-pick" data-testid="ws-ov-plan-pick">
      {props.freePlans.length === 0 && <li className="empty small">No plan without a workspace. Make one in the Ideas window.</li>}
      {props.freePlans.map((d) => (
        <li key={d.id}>
          <button className="ws-ov-chip" data-drag-handle="" data-testid="ws-ov-plan-pick-row" data-doc={d.id} title="Click to link it here, or drag it onto a workspace with no plan"
            onPointerDown={(e) => props.arm(e, d)} onClick={() => { props.picked(); props.setPlan(d.id); }}>{d.title}</button>
        </li>
      ))}
    </ul>
  );
}

/** No plan: link one, or make one from the linked ideas. */
export function NoPlan(props: Pick<WsParts, "freePlans" | "arm" | "setPlan" | "makePlan">) {
  const [pick, setPick] = useState(false);
  return (
    <>
      <div className="ws-ov-line">
        <span className="ws-ov-none">No plan.</span>
        <button className="ctrl sm" data-testid="ws-ov-link-plan" onClick={() => setPick((p) => !p)}>Link a plan ▸</button>
        <button className="ctrl sm" data-testid="ws-ov-make-plan" disabled={!props.makePlan} onClick={() => props.makePlan?.()}
          title={props.makePlan ? "Make a plan from the linked ideas; it becomes this workspace's" : undefined}>Make a plan</button>
        {!props.makePlan && <span className="ws-ov-none" data-testid="ws-ov-make-why">A plan is made from ideas: link one first.</span>}
      </div>
      {pick && <PlanPicker {...props} picked={() => setPick(false)} />}
    </>
  );
}

/** "Link an idea ▸" and its list, and New idea beside it. */
export function LinkIdea(props: Pick<WsParts, "linkable" | "arm" | "linkIdea" | "newIdea">) {
  const [pick, setPick] = useState(false);
  return (
    <>
      <button className="ctrl sm" data-testid="ws-ov-link-idea" onClick={() => setPick((p) => !p)}>Link an idea ▸</button>
      <button className="ctrl sm" data-testid="ws-ov-new-idea" title="Make a new idea and link it to this workspace" onClick={() => { setPick(false); props.newIdea(); }}>New idea</button>
      {pick && (
        <ul className="ws-ov-pick" data-testid="ws-ov-idea-pick">
          {props.linkable.length === 0 && <li className="empty small">No other idea to link.</li>}
          {props.linkable.map((d) => (
            <li key={d.id}>
              <button className="ws-ov-chip" data-drag-handle="" data-testid="ws-ov-idea-pick-row" data-doc={d.id} title="Click to link it here, or drag it onto a workspace"
                onPointerDown={(e) => props.arm(e, d)} onClick={() => { setPick(false); props.linkIdea(d.id); }}>{d.title}</button>
            </li>
          ))}
        </ul>
      )}
    </>
  );
}

/** What the plan's reader could not take, all of it. */
export function PlanProblems({ ov }: { ov: Overview }) {
  if (!ov.problems.length) return null;
  return (
    <ul className="ws-ov-problems" data-testid="ws-ov-problems" role="alert">
      {ov.problems.map((p, n) => <li key={n} title={p.text}>{problemLine(p)}</li>)}
    </ul>
  );
}

/** The plan's Projects: each with where it is, and Clone… or Create…. */
export function PlanProjects(props: Pick<WsParts, "bringProject"> & { ov: Overview }) {
  const { ov } = props;
  if (ov.projects === null) return <div className="ws-ov-none">The plan has no Projects section.</div>;
  if (ov.projects.length === 0) return <div className="ws-ov-none">The plan lists no projects yet.</div>;
  return (
    <div className="ws-ov-rows">
      {ov.projects.map((p) => (
        <div key={p.line} className="ws-ov-row" data-testid="ws-ov-project" data-state={p.state}>
          <span className="ws-ov-item">
            <span className="ws-ov-name">{p.name}</span>
            <span className="ws-ov-detail" title={p.root ?? p.url ?? p.path}>{p.root ?? p.url ?? p.path ?? ""}</span>
          </span>
          <span className={"ws-ov-state " + p.state}>{STATE_WORDS[p.state]}{p.state === "registered" && !p.in_workspace ? " · not in this workspace" : ""}</span>
          {(p.state === "to-clone" || p.state === "to-create") && (
            <button className="ctrl sm" data-testid="ws-ov-bring" data-name={p.name} onClick={() => props.bringProject(p.name)}>{p.state === "to-clone" ? "Clone…" : "Create…"}</button>
          )}
        </div>
      ))}
    </div>
  );
}

/** The plan's Work: each item's state, and Review…, Link a place… or Show ▸. */
export function WorkList(props: Pick<WsParts, "snap" | "activity" | "gone" | "workHot" | "review" | "linkPlace" | "showPlace"> & { ov: Overview }) {
  const { ov } = props;
  const [brief, setBrief] = useState<number | null>(null);
  if (ov.work === null) return <div className="ws-ov-none">The plan has no Work section.</div>;
  if (ov.work.length === 0) return <div className="ws-ov-none">The plan lists no work yet.</div>;
  return (
    <div className="ws-ov-rows">
      {ov.work.map((w, n) => {
        const at = itemPlace(w, ov.work_states[n], props.snap, props.activity, props.gone);
        return (
          <div key={w.line} className={props.workHot === w.line ? "ws-ov-work on" : "ws-ov-work"} data-testid="ws-ov-work" data-drop="work" data-work-line={w.line} data-place={at.kind}>
            <div className="ws-ov-row">
              <button className="ws-ov-item" data-testid="ws-ov-item" title={[w.title, w.project, w.branch].filter(Boolean).join(" · ")} onClick={() => setBrief((b) => (b === w.line ? null : w.line))} aria-expanded={brief === w.line}>
                <span className="ws-ov-name">{w.title || "(no title)"}</span>
                <span className="ws-ov-detail">{[w.project, w.branch].filter(Boolean).join(" · ")}</span>
              </button>
              <span className={`ws-ov-state at-${at.kind}`} data-proposed={w.proposal && at.kind === "none" ? "" : undefined} title={w.started ?? (w.proposal ? `Proposed by the workspace chat “${w.proposal.by}” on ${w.proposal.at}. Nothing has started: Review… reads its brief, then Start.` : undefined)}>{workStatus(w, at)}</span>
              {at.place && at.root && (
                <button className="ctrl sm" data-testid="ws-ov-show-place" onClick={() => props.showPlace(at.root!, at.place!.slug)}>Show ▸</button>
              )}
              {itemFree(at) && (
                <>
                  <button className="ctrl sm" data-testid="ws-ov-review" title="Read its brief, then Start" onClick={() => props.review(w)}>Review…</button>
                  <button className="ctrl sm" data-testid="ws-ov-link-place" title="Record a place that exists already as this item's (or drag one here from the nav)" onClick={() => props.linkPlace(w)}>Link a place…</button>
                </>
              )}
            </div>
            {brief === w.line && <pre className="ws-ov-brief">{w.brief ?? "(no brief)"}</pre>}
          </div>
        );
      })}
    </div>
  );
}

/** A box of the Overview: its name, what is beside it, and its act. */
function Card(props: { name: string; testId: string; side?: ReactNode; act?: ReactNode; className?: string; children: ReactNode }) {
  return (
    <section className={"wsf-card" + (props.className ? " " + props.className : "")} data-testid={props.testId} aria-label={props.name}>
      <header className="wsf-card-h">
        <span className="wsf-card-name">{props.name}</span>
        {props.side && <span className="wsf-card-side">{props.side}</span>}
        {props.act}
      </header>
      {props.children}
    </section>
  );
}

function WorkCard(props: WsParts & { showTab(tab: "plan"): void }) {
  const { ov } = props;
  const plan = ov?.plan ?? null;
  return (
    <Card name="Work from the plan" testId="ws-ov-card-work" act={plan?.live && (
      <span className="wsf-card-acts">
        <button className="wsf-link" data-testid="ws-ov-plan" onClick={() => props.openDoc(plan.live!)} title={`${plan.title}: open it here`}>{plan.title}</button>
        <button className="ws-ov-x" title="Unlink the plan (the plan stays)" data-testid="ws-ov-plan-unlink" onClick={() => props.setPlan(null)}>×</button>
      </span>
    )}>
      {!ov ? <p className="ws-ov-none">{props.error ? "It could not be read." : "Reading…"}</p>
        : !plan ? <div className="wsf-card-b"><NoPlan {...props} /></div>
        : !plan.live ? (
          <div className="wsf-card-b"><span className="ws-ov-gone" data-testid="ws-ov-plan-gone">
            The plan cannot be read: {plan.error ?? "it is gone"}.
            <button className="ws-ov-x" title="Unlink it" onClick={() => props.setPlan(null)}>×</button>
          </span></div>
        ) : (
          <>
            <WorkList {...props} ov={ov} />
            {(ov.projects ?? []).some((p) => p.state === "to-clone" || p.state === "to-create") && (
              <button className="wsf-more" data-testid="ws-ov-plan-projects" onClick={() => props.showTab("plan")}>
                {(() => { const n = ov.projects!.filter((p) => p.state === "to-clone" || p.state === "to-create").length; return `${n} of its projects ${n === 1 ? "is" : "are"} not here yet`; })()} ▸
              </button>
            )}
          </>
        )}
    </Card>
  );
}

function IdeasCard(props: WsParts) {
  const { ov } = props;
  return (
    <Card name="Ideas" testId="ws-ov-card-ideas">
      <div className="wsf-card-b">
        {ov && ov.ideas.length === 0 && <span className="ws-ov-none">None linked.</span>}
        {ov && ov.ideas.length > 0 && (
          <div className="ws-ov-line">
            {ov.ideas.map((i) => (
              <span key={i.id} className="ws-ov-chip ws-ov-idea" data-testid="ws-ov-idea" data-idea={i.id}>
                {i.live
                  ? <button className="ws-ov-chip-open" data-drag-handle="" title="Open it here; drag it onto another workspace to link it there too"
                      onPointerDown={(e) => props.arm(e, { id: i.live!, title: i.title ?? "", kind: "idea" })}
                      onClick={() => props.openDoc(i.live!)}>{i.title}{i.merged && <span className="ws-ov-merged" title="This id was merged into it">merged</span>}</button>
                  : <span className="ws-ov-gone">an idea that is gone</span>}
                <button className="ws-ov-x" title="Unlink it (the idea stays)" data-testid="ws-ov-idea-unlink" onClick={() => props.unlinkIdea(i.id)}>×</button>
              </span>
            ))}
          </div>
        )}
        <div className="ws-ov-line"><LinkIdea {...props} /></div>
      </div>
    </Card>
  );
}

const placeWords = (p: WsProjectRow): string =>
  [`${p.places} place${p.places === 1 ? "" : "s"}`, p.waiting ? `${p.waiting} waiting on you` : "", p.working ? `${p.working} working` : ""].filter(Boolean).join(" · ");

function ProjectsCard(props: { projects: WsProjectRow[]; takeOut(root: string): void; addProject(e: React.MouseEvent): void }) {
  return (
    <Card name="Projects" testId="ws-ov-card-projects" act={<button className="ctrl sm" data-testid="ws-ov-add-project" title="The workspace's menu, with Add project" onClick={props.addProject}>Add ▸</button>}>
      {props.projects.length === 0 && <p className="ws-ov-none">No projects yet.</p>}
      <div className="ws-ov-rows">
        {props.projects.map((p) => (
          <div key={p.root} className="ws-ov-row" data-testid="ws-ov-here" data-root={p.root} data-registered={p.registered || undefined}>
            <span className="ws-ov-item">
              <span className="ws-ov-name" title={p.root}>{p.name}</span>
              <span className="ws-ov-detail">{p.registered ? placeWords(p) : "not registered"}</span>
            </span>
            {!p.registered && <button className="ctrl sm" data-testid="ws-ov-take-out" title="Take it out of this workspace" onClick={() => props.takeOut(p.root)}>Take it out</button>}
          </div>
        ))}
      </div>
    </Card>
  );
}

/** No memory, and no chats: the designed empty states. */
export function NoMemory({ newChat, make, busy }: { newChat?: () => void; make: () => void; busy: boolean }) {
  return (
    <div className="wsf-empty" data-testid="ws-no-memory">
      <p>No memory yet. It starts with the first chat.</p>
      <div className="ws-ov-line">
        {newChat && <button className="ctrl sm" data-testid="ws-new-chat-memory" onClick={newChat}>New chat</button>}
        <button className="ctrl sm" data-testid="ws-make-memory" disabled={busy} onClick={make} title="A folder of its own with an index, kept as a history. A chat makes one by itself.">Make memory</button>
      </div>
    </div>
  );
}
export function NoChats({ newChat }: { newChat: () => void }) {
  return (
    <div className="wsf-empty" data-testid="ws-no-chats">
      <p>No chats yet.</p>
      <div className="ws-ov-line"><button className="ctrl sm" data-testid="ws-new-chat" onClick={newChat}>New chat</button></div>
    </div>
  );
}

/** The Overview: what needs a person, then the widest thing the workspace has
 *  (its memory; its work while there are no chats), then the rest. Under
 *  `WIDE` px of room it is one column (App.css), memory last. */
export function WorkspaceOverview(props: WsParts & {
  ideasOn: boolean;
  chatsOn: boolean;
  attention: readonly Attention[];
  /** What an attention line is about. */
  goTo(a: Attention): void;
  showTab(tab: "memory" | "plan" | "ideas" | "chats"): void;
  projects: WsProjectRow[];
  takeOut(root: string): void;
  addProject(e: React.MouseEvent): void;
  /** The chats' own lines, and how many there are. */
  /** The memory's column, and what is beside its name. */
  memory: ReactNode;
  memorySide: string | null;
  chats: ReactNode;
  chatCount: number;
  newChat(): void;
}) {
  const work = props.ideasOn && <WorkCard {...props} />;
  return (
    <div className="wsf-ov" data-testid="ws-overview">
      {props.error && <p className="ws-ov-err" role="alert">{props.error}</p>}
      {props.attention.map((a) => (
        <div key={a.kind === "chat" ? a.conv : a.kind} className="wsf-attn" data-testid="ws-attention" data-kind={a.kind} role="status">
          <span className="wsf-attn-dot" aria-hidden="true" />
          <span className="wsf-attn-t">{a.text}</span>
          <button className="ctrl sm" data-testid="ws-attention-go" onClick={() => props.goTo(a)}>{a.kind === "chat" ? "Open the chat ▸" : a.kind === "unsaved" ? "Go to it ▸" : "Open the plan ▸"}</button>
        </div>
      ))}
      <div className="wsf-cols">
        <div className="wsf-col wsf-col-main">
          {props.chatsOn
            ? <Card name="Memory" testId="ws-ov-card-memory" className="wsf-card-tall" side={props.memorySide} act={<button className="ctrl sm" data-testid="ws-ov-memory-open" onClick={() => props.showTab("memory")}>Open ▸</button>}>{props.memory}</Card>
            : work}
        </div>
        <div className="wsf-col wsf-col-side">
          {props.chatsOn && <Card name="Chats" testId="ws-ov-card-chats" act={props.chatCount > 0 && <button className="ctrl sm" data-testid="ws-ov-chats-all" onClick={() => props.showTab("chats")}>All {props.chatCount}</button>}>{props.chatCount > 0 ? props.chats : <NoChats newChat={props.newChat} />}</Card>}
          {props.chatsOn && work}
          {props.ideasOn && <IdeasCard {...props} />}
          <ProjectsCard projects={props.projects} takeOut={props.takeOut} addProject={props.addProject} />
        </div>
      </div>
    </div>
  );
}
