/** A named workspace's overview (idea board phase 4, §9.2): its plan and
 *  linked ideas, and the plan's Projects and Work as core's reader answers
 *  them, READ-ONLY here (phase 5 clones, creates and starts). Shown in the
 *  main area when the workspace's name is clicked and no place is selected.
 *
 *  "plan ▸" and an idea open the document in the Ideas window. The plan and
 *  idea chips here are the same-window drag sources of §6.3: a plan with no
 *  workspace, dragged onto a workspace in the nav, becomes its plan; an idea
 *  dragged there is linked. Every drag has its menu path here too (Link a
 *  plan ▸, Link an idea ▸). The rules and calls are `plans.ts`'s. */
import { useState } from "react";
import { STATE_WORDS, problemLine, workStatus, type Overview } from "./plans";

export type DocLite = { id: string; title: string; kind: "idea" | "plan" };

export function WorkspaceOverview(props: {
  name: string;
  ov: Overview | null;
  error: string | null;
  /** Plans no workspace has yet: what "Link a plan ▸" offers. */
  freePlans: DocLite[];
  /** Ideas this workspace does not link yet. */
  linkable: DocLite[];
  openDoc(id: string): void;
  setPlan(plan: string | null): void;
  linkIdea(id: string): void;
  unlinkIdea(id: string): void;
  arm(e: React.PointerEvent, d: DocLite): void;
}) {
  const ov = props.ov;
  const [pick, setPick] = useState<"plan" | "idea" | null>(null);
  const [brief, setBrief] = useState<number | null>(null);
  const plan = ov?.plan ?? null;
  return (
    <div className="home-scroll">
      <div className="ws-ov" data-testid="ws-overview">
        <h1 className="ws-ov-title">{props.name}</h1>
        {props.error && <p className="ws-ov-err" role="alert">{props.error}</p>}
        {!ov && !props.error && <p className="empty small">Reading…</p>}
        {ov && (
          <>
            <div className="ws-ov-line">
              <span className="ws-ov-key">plan</span>
              {plan ? (
                plan.live ? (
                  <>
                    <button className="ctrl sm" data-testid="ws-ov-plan" onClick={() => props.openDoc(plan.live!)} title="Open it in the Ideas window">
                      {plan.title} ▸
                    </button>
                    <button className="ws-ov-x" title="Unlink the plan (the plan stays)" data-testid="ws-ov-plan-unlink" onClick={() => props.setPlan(null)}>×</button>
                  </>
                ) : (
                  <span className="ws-ov-gone" data-testid="ws-ov-plan-gone">
                    The plan cannot be read: {plan.error ?? "it is gone"}.
                    <button className="ws-ov-x" title="Unlink it" onClick={() => props.setPlan(null)}>×</button>
                  </span>
                )
              ) : (
                <>
                  <span className="empty small">No plan.</span>
                  <button className="ctrl sm" data-testid="ws-ov-link-plan" onClick={() => setPick(pick === "plan" ? null : "plan")}>Link a plan ▸</button>
                </>
              )}
            </div>
            {pick === "plan" && !plan && (
              <ul className="ws-ov-pick" data-testid="ws-ov-plan-pick">
                {props.freePlans.length === 0 && <li className="empty small">No plan without a workspace. Make one in the Ideas window.</li>}
                {props.freePlans.map((d) => (
                  <li key={d.id}>
                    <button className="ws-ov-chip" data-testid="ws-ov-plan-pick-row" data-doc={d.id} title="Click to link it here, or drag it onto a workspace with no plan"
                      onPointerDown={(e) => props.arm(e, d)} onClick={() => { setPick(null); props.setPlan(d.id); }}>{d.title}</button>
                  </li>
                ))}
              </ul>
            )}
            <div className="ws-ov-line">
              <span className="ws-ov-key">ideas</span>
              {ov.ideas.length === 0 && <span className="empty small">None linked.</span>}
              {ov.ideas.map((i) => (
                <span key={i.id} className="ws-ov-chip ws-ov-idea" data-testid="ws-ov-idea" data-idea={i.id}>
                  {i.live
                    ? <button className="ws-ov-chip-open" title="Open it in the Ideas window; drag it onto another workspace to link it there too"
                        onPointerDown={(e) => props.arm(e, { id: i.live!, title: i.title ?? "", kind: "idea" })}
                        onClick={() => props.openDoc(i.live!)}>{i.title}{i.merged && <span className="ws-ov-merged" title="This id was merged into it">merged</span>} ▸</button>
                    : <span className="ws-ov-gone">an idea that is gone</span>}
                  <button className="ws-ov-x" title="Unlink it (the idea stays)" data-testid="ws-ov-idea-unlink" onClick={() => props.unlinkIdea(i.id)}>×</button>
                </span>
              ))}
              <button className="ctrl sm" data-testid="ws-ov-link-idea" onClick={() => setPick(pick === "idea" ? null : "idea")}>Link an idea ▸</button>
            </div>
            {pick === "idea" && (
              <ul className="ws-ov-pick" data-testid="ws-ov-idea-pick">
                {props.linkable.length === 0 && <li className="empty small">No other idea to link.</li>}
                {props.linkable.map((d) => (
                  <li key={d.id}>
                    <button className="ws-ov-chip" data-testid="ws-ov-idea-pick-row" data-doc={d.id} title="Click to link it here, or drag it onto a workspace"
                      onPointerDown={(e) => props.arm(e, d)} onClick={() => { setPick(null); props.linkIdea(d.id); }}>{d.title}</button>
                  </li>
                ))}
              </ul>
            )}
            {plan?.live && (
              <>
                {ov.problems.length > 0 && (
                  <ul className="ws-ov-problems" data-testid="ws-ov-problems" role="alert">
                    {ov.problems.map((p, n) => <li key={n} title={p.text}>{problemLine(p)}</li>)}
                  </ul>
                )}
                <div className="resume-h">PROJECTS (FROM THE PLAN)</div>
                {ov.projects === null ? <div className="empty small">The plan has no Projects section.</div>
                  : ov.projects.length === 0 ? <div className="empty small">The plan lists no projects yet.</div> : (
                  <div className="ws-ov-rows">
                    {ov.projects.map((p) => (
                      <div key={p.line} className="ws-ov-row" data-testid="ws-ov-project" data-state={p.state}>
                        <span className="ws-ov-name">{p.name}</span>
                        <span className={"ws-ov-state " + p.state}>{STATE_WORDS[p.state]}{p.state === "registered" && !p.in_workspace ? " · not in this workspace" : ""}</span>
                        <span className="ws-ov-detail" title={p.root ?? p.url ?? p.path}>{p.root ?? p.url ?? p.path ?? ""}</span>
                      </div>
                    ))}
                  </div>
                )}
                <div className="resume-h">WORK (FROM THE PLAN)</div>
                {ov.work === null ? <div className="empty small">The plan has no Work section.</div>
                  : ov.work.length === 0 ? <div className="empty small">The plan lists no work yet.</div> : (
                  <div className="ws-ov-rows">
                    {ov.work.map((w) => (
                      <div key={w.line} data-testid="ws-ov-work">
                        <button className="ws-ov-row ws-ov-row-btn" onClick={() => setBrief((b) => (b === w.line ? null : w.line))} aria-expanded={brief === w.line}>
                          <span className="ws-ov-name">{w.title || "(no title)"}</span>
                          <span className="ws-ov-detail">{[w.project, w.branch].filter(Boolean).join(" · ")}</span>
                          <span className="ws-ov-state">{workStatus(w)}</span>
                        </button>
                        {brief === w.line && <pre className="ws-ov-brief">{w.brief ?? "(no brief)"}</pre>}
                      </div>
                    ))}
                  </div>
                )}
              </>
            )}
          </>
        )}
      </div>
    </div>
  );
}
