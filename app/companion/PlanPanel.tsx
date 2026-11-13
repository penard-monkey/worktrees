// Plans (§2.2, §6.1, §9.1, phase 4): the companion's plan surfaces. For an
// idea, the strip of plans it is in and "Make a plan"; for a plan document,
// its ideas, its Projects and Work as core's reader answers them (problems
// named by line, the document shown under it whatever they are), "Make
// workspace" / "Open workspace", and the pickers of the menu paths. The
// rules and calls are app/src/plans.ts's; every act ends in one row call the
// window runs (Companion.tsx's `act`).
import { useMemo, useState } from "react";
import { useEscape } from "../src/useEscape";
import { STATE_WORDS, problemLine, workStatus, type PlanView } from "../src/plans";

export type RegProject = { root: string; name: string };
export type WsOption = { id: string; name: string; plan: string | null; ideas: string[] };

/** An idea's plans (`planned-in`), and "Make a plan" from it (or from the
 *  selected ideas, when it is one of them). */
export function PlanStrip(props: { plans: { id: string; title: string }[]; sources: number; busy: boolean; open(id: string): void; make(): void; link(): void }) {
  return (
    <div className="ix-related ix-planstrip" data-testid="plans-strip">
      <span className="ix-related-head">Plans</span>
      {props.plans.length === 0 && <span className="ix-quiet ix-plan-none">In no plan yet.</span>}
      {props.plans.map((p) => (
        <button key={p.id} className="ix-chip ix-chip-plan" data-testid="plans-chip" data-plan={p.id} onClick={() => props.open(p.id)}
          title="Open this plan">{p.title}</button>
      ))}
      <button className="ix-btn ix-btn-small" data-testid="plans-make" disabled={props.busy || props.sources === 0} onClick={props.make}
        title="A new plan document from this idea, or from the selected ideas">{props.sources > 1 ? `Make a plan from ${props.sources}` : "Make a plan"}</button>
      <button className="ix-btn ix-btn-small ix-btn-quiet" data-testid="plans-link-ws" disabled={props.busy} onClick={props.link}
        title="Link this idea to a workspace's context">Link to workspace…</button>
    </div>
  );
}

/** A plan document's strip: what the reader read, read-only here (phase 5
 *  acts on projects and work). `data-drop-plan` makes it a drop target for an
 *  idea; its Projects box takes a project from the picker. */
export function PlanPanel(props: {
  view: PlanView;
  busy: boolean;
  /** The registered project being dragged is over the Projects box. */
  projectsHot: boolean;
  /** An idea being dragged is over the strip. */
  ideasHot: boolean;
  openIdea(id: string): void;
  removeIdea(id: string): void;
  addIdea(): void;
  addProject(): void;
  makeWorkspace(): void;
  openWorkspace(id: string): void;
  linkWorkspace(): void;
  gotoLine(line: number): void;
}) {
  const v = props.view;
  const [open, setOpen] = useState(true);
  const [brief, setBrief] = useState<number | null>(null);
  return (
    <section className={`ix-plan${props.ideasHot ? " ix-drop" : ""}`} data-testid="plan-panel" data-drop-plan={v.doc.id} aria-label="Plan">
      <div className="ix-plan-head">
        <button className="ix-btn ix-btn-quiet ix-btn-small" aria-expanded={open} onClick={() => setOpen((o) => !o)}>{open ? "▾" : "▸"} Plan</button>
        <span className="ix-related-head">From</span>
        {v.ideas.map((i) => (
          <span key={i.id} className="ix-chip ix-chip-idea" data-testid="plan-idea" data-idea={i.id} data-live={i.live ?? ""}>
            {i.live
              ? <button className="ix-chip-open" onClick={() => props.openIdea(i.live!)} title={i.merged ? "This id was merged into it" : "Open this idea"}>
                  {i.title}{i.merged && <span className="ix-chip-how">merged</span>}
                </button>
              : <span className="ix-chip-gone" title={i.id}>an idea that is gone</span>}
            <button className="ix-chip-x" data-testid="plan-idea-remove" aria-label="Take it out of this plan" title="Take it out of this plan (the idea stays)"
              disabled={props.busy} onClick={() => props.removeIdea(i.id)}>×</button>
          </span>
        ))}
        <button className="ix-btn ix-btn-small" data-testid="plan-add-idea" disabled={props.busy} onClick={props.addIdea}
          title="Add an idea to this plan (or drag one from the list onto this strip)">Add idea…</button>
        <span className="ix-plan-ws">
          {v.workspace ? (
            <button className="ix-btn ix-btn-small" data-testid="plan-open-ws" onClick={() => props.openWorkspace(v.workspace!.id)}
              title="Show it in the main window">Open workspace “{v.workspace.name}” ▸</button>
          ) : (
            <>
              <button className="ix-btn ix-btn-small ix-btn-primary" data-testid="plan-make-ws" disabled={props.busy} onClick={props.makeWorkspace}
                title="A workspace named after this plan, holding its registered projects">Make workspace</button>
              <button className="ix-btn ix-btn-small ix-btn-quiet" data-testid="plan-link-ws" disabled={props.busy} onClick={props.linkWorkspace}
                title="Make an existing workspace with no plan this plan's workspace">Link ▸</button>
            </>
          )}
        </span>
      </div>
      {open && (
        <div className="ix-plan-body">
          {v.problems.length > 0 && (
            <ul className="ix-plan-problems" data-testid="plan-problems" role="alert">
              {v.problems.map((p, n) => (
                <li key={n}>
                  <button className="ix-plan-problem" data-testid="plan-problem" data-line={p.line} onClick={() => props.gotoLine(p.line)} title={p.text}>
                    {problemLine(p)}
                  </button>
                </li>
              ))}
            </ul>
          )}
          <div className={`ix-plan-col${props.projectsHot ? " ix-drop" : ""}`} data-drop-plan-projects={v.doc.id}>
            <div className="ix-plan-col-head">
              <span>Projects</span>
              <button className="ix-btn ix-btn-small ix-btn-quiet" data-testid="plan-add-project" disabled={props.busy} onClick={props.addProject}>Add project…</button>
            </div>
            {v.projects === null ? <p className="ix-quiet">No <code>## Projects</code> section.</p> : v.project_states.length === 0 ? (
              <p className="ix-quiet">None yet. Add one, or write <code>- name (registered)</code>, <code>(clone: url)</code> or <code>(new: path)</code>.</p>
            ) : (
              <ul className="ix-plan-rows">
                {v.project_states.map((p) => (
                  <li key={`${p.line}`} className="ix-plan-row" data-testid="plan-project" data-state={p.state}>
                    <span className="ix-plan-name">{p.name}</span>
                    <span className={`ix-plan-state ix-plan-${p.state}`}>{STATE_WORDS[p.state]}</span>
                    <span className="ix-plan-detail" title={p.root ?? p.url ?? p.path}>{p.root ?? p.url ?? p.path ?? ""}</span>
                  </li>
                ))}
              </ul>
            )}
          </div>
          <div className="ix-plan-col">
            <div className="ix-plan-col-head"><span>Work</span></div>
            {v.work === null ? <p className="ix-quiet">No <code>## Work</code> section.</p> : v.work.length === 0 ? (
              <p className="ix-quiet">None yet. Each item is <code>### title</code> with <code>project:</code>, <code>branch:</code> and <code>brief: |</code>.</p>
            ) : (
              <ul className="ix-plan-rows">
                {v.work.map((w) => (
                  <li key={w.line} className="ix-plan-work" data-testid="plan-work">
                    <button className="ix-plan-row ix-plan-row-btn" onClick={() => setBrief((b) => (b === w.line ? null : w.line))} aria-expanded={brief === w.line}>
                      <span className="ix-plan-name">{w.title || "(no title)"}</span>
                      <span className="ix-plan-detail">{[w.project, w.branch].filter(Boolean).join(" · ")}</span>
                      <span className="ix-plan-state">{workStatus(w)}</span>
                    </button>
                    {brief === w.line && <pre className="ix-split-preview">{w.brief ?? "(no brief)"}</pre>}
                  </li>
                ))}
              </ul>
            )}
          </div>
        </div>
      )}
    </section>
  );
}

/** "Add project…": the registered projects the plan does not list yet. A
 *  row can be clicked, or dragged onto the plan's Projects box (§6.3's
 *  same-window form of "a project onto a plan"). */
export function PickProject(props: { rows: RegProject[]; skip: string[]; act(root: string): void; arm(e: React.PointerEvent, root: string): void; close(): void }) {
  const [q, setQ] = useState("");
  useEscape(props.close, true);
  const shown = useMemo(() => {
    const s = q.trim().toLowerCase();
    return props.rows.filter((r) => !props.skip.includes(r.root) && (!s || r.name.toLowerCase().includes(s)));
  }, [props.rows, props.skip, q]);
  return (
    <div className="ix-pop ix-pickproj" role="dialog" aria-label="Add a project to the plan" data-testid="plan-pick-project">
      <p className="ix-pop-head">Add a registered project (click, or drag onto Projects)</p>
      <input className="ix-filter" autoFocus placeholder="Filter by name" value={q} onChange={(e) => setQ(e.target.value)}
        onKeyDown={(e) => { if (e.key === "Enter" && shown[0]) props.act(shown[0].root); }} />
      <div className="ix-pick-rows">
        {shown.map((r) => (
          <button key={r.root} className="ix-pop-item" data-testid="plan-pick-project-row" data-root={r.root} title={r.root}
            onPointerDown={(e) => props.arm(e, r.root)} onClick={() => props.act(r.root)}>{r.name}</button>
        ))}
        {shown.length === 0 && <p className="ix-quiet ix-pop-head">No other registered project.</p>}
      </div>
      <button className="ix-btn ix-btn-small ix-btn-quiet" onClick={props.close}>Close</button>
    </div>
  );
}

/** "Link to workspace…" (an idea) or "Link ▸" (a plan, onto a workspace with
 *  none): the workspaces it can go to. */
export function PickWorkspace(props: { title: string; rows: WsOption[]; act(id: string): void; close(): void; empty: string }) {
  useEscape(props.close, true);
  return (
    <div className="ix-scrim" onPointerDown={(e) => { if (e.target === e.currentTarget) props.close(); }}>
      <div className="ix-dialog" role="dialog" aria-label={props.title} data-testid="plan-pick-ws">
        <p className="ix-dialog-head">{props.title}</p>
        <ul className="ix-pick-rows">
          {props.rows.map((w) => (
            <li key={w.id}><button className="ix-pop-item" data-testid="plan-pick-ws-row" data-ws={w.id} onClick={() => props.act(w.id)}>{w.name}</button></li>
          ))}
          {props.rows.length === 0 && <li className="ix-quiet">{props.empty}</li>}
        </ul>
        <div className="ix-dialog-foot"><button className="ix-btn" onClick={props.close}>Cancel</button></div>
      </div>
    </div>
  );
}
