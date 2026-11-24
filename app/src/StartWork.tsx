/** A plan's work item in both windows (idea board phase 5, §2.4): the review
 *  before Start, and "Link a place…". The rules, calls and words are
 *  `plans.ts`'s; each act is one row call the window runs.
 *
 *  Nothing starts without the review: the brief is shown as the exact text
 *  that will be written to `.planning/brief.md`, and Start starts this ONE
 *  item. In the Ideas window it is editable; in the main window's overview it
 *  is read-only, because the plan's document is not there to be read beside
 *  it: "Edit the plan" opens it. Editing the text here edits the
 *  brief, not the plan. */
import { useMemo, useRef, useState } from "react";
import { HARNESS_LABEL, NEEDS_MODEL, isHarness } from "./harness";
import { LAUNCH_REFUSED, slugOf, startProblem, type ItemPlace, type Snap, type StartDone, type WorkItem, type WorkState } from "./plans";
import { useEscape } from "./useEscape";
import "./StartWork.css";

const errText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));
/** Core's last line: the reason, after the steps that came before it. */
const lastLine = (out: string) => out.split("\n").map((l) => l.trim()).filter(Boolean).pop() ?? "It failed, and said nothing.";

export function StartReview(props: {
  item: WorkItem;
  state: WorkState | undefined;
  at: ItemPlace;
  snap: Snap;
  agent: { provider: string; model: string };
  readOnly?: boolean;
  /** Start the item with this brief. */
  start(brief: string): Promise<StartDone>;
  /** The place is ready and its agent was not started: start it anyway. */
  launch(root: string, slug: string): Promise<{ ok: boolean; output: string }>;
  show(root: string, slug: string): void;
  edit?(): void;
  close(): void;
}) {
  // Fixed when the review opens: a re-read of the plan under it (or the
  // `started:` line a Start writes) must not change what was reviewed.
  const [item] = useState(props.item);
  const [brief, setBrief] = useState(item.brief ?? "");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const [done, setDone] = useState<StartDone | null>(null);
  const [launched, setLaunched] = useState(false);
  const running = useRef(false);
  useEscape(() => { if (!busy) props.close(); });
  const root = props.state?.root ?? "";
  const started = done?.started ?? null;
  const refused = !!started && done?.result.code === LAUNCH_REFUSED && !launched;
  const label = isHarness(props.agent.provider) ? HARNESS_LABEL[props.agent.provider] : props.agent.provider;
  const needsModel = isHarness(props.agent.provider) && NEEDS_MODEL[props.agent.provider] && !props.agent.model;
  const problem = started ? null
    : startProblem(item, props.state, props.at, props.snap)
      ?? (!isHarness(props.agent.provider) ? `“${props.agent.provider}” is not an agent this app knows: fix the item's \`ai:\` line.` : null)
      ?? (needsModel ? `${label} needs a model: add a \`model:\` line to the item, or set its default in Settings.` : null)
      ?? (brief.trim() ? null : "The brief is empty: say what the agent is to do.");
  const run = async (f: () => Promise<void>) => {
    if (running.current) return;
    running.current = true;
    setBusy(true);
    setErr("");
    try { await f(); } catch (e) { setErr(errText(e)); } finally { running.current = false; setBusy(false); }
  };
  const go = () => void run(async () => {
    const d = await props.start(brief);
    if (d.started) setDone(d);
    if (!d.result.ok && !(d.started && d.result.code === LAUNCH_REFUSED)) setErr(d.started ? d.result.output : lastLine(d.result.output));
  });
  const anyway = () => void run(async () => {
    const r = await props.launch(started!.project, started!.slug);
    if (r.ok) setLaunched(true); else setErr(lastLine(r.output));
  });
  return (
    <div className="sw-scrim" data-dialog-open="" onPointerDown={(e) => { if (e.target === e.currentTarget && !busy) props.close(); }}>
      <div className="sw-dialog" role="dialog" aria-label={`Start ${item.title}`} data-testid="start-review">
        <p className="sw-head">{item.title}</p>
        <dl className="sw-facts">
          <dt>Project</dt><dd data-testid="start-project">{item.project ?? "none"}{root && <span className="sw-quiet"> {root}</span>}</dd>
          <dt>Branch</dt><dd data-testid="start-branch">{item.branch ?? "none"}{root && item.branch && <span className="sw-quiet"> in {root}/.worktrees/{slugOf(item.branch)}</span>}</dd>
          <dt>Agent</dt><dd data-testid="start-agent">{label}{props.agent.model ? ` · ${props.agent.model}` : ""}<span className="sw-quiet">{item.ai || item.model ? " from the plan" : " your default"}</span></dd>
        </dl>
        <label className="sw-label" htmlFor="sw-brief">
          Brief, as it will be written to <code>.planning/brief.md</code>
          {props.readOnly ? "" : ". Edits here change the brief, not the plan."}
        </label>
        <textarea id="sw-brief" className="sw-brief" data-testid="start-brief" value={brief} readOnly={props.readOnly || busy || !!started} spellCheck={false}
          rows={12} onChange={(e) => setBrief(e.target.value)} />
        {problem && <p className="sw-note sw-bad" role="alert" data-testid="start-problem">{problem}</p>}
        {err && <p className="sw-note sw-bad" role="alert" data-testid="start-error">{err}</p>}
        {started && !refused && !err && <p className="sw-note" data-testid="start-done">Started in {started.slug}. Its agent opens on the brief.</p>}
        {started && !!done?.result.warnings?.length && <p className="sw-note sw-bad" data-testid="start-warnings">{done.result.warnings.join("\n")}</p>}
        {refused && <p className="sw-note sw-bad" role="alert" data-testid="start-refused">The place {started!.slug} is ready with its brief. Its agent was not started: {lastLine(done!.result.output)}</p>}
        {done?.plan_error && <p className="sw-note sw-bad" role="alert" data-testid="start-plan-error">{done.plan_error}</p>}
        <div className="sw-foot">
          {props.readOnly && props.edit && !started && <button className="sw-btn" data-testid="start-edit" disabled={busy} onClick={props.edit}>Edit the plan ▸</button>}
          <span className="sw-gap" />
          <button className="sw-btn" data-testid="start-close" disabled={busy} onClick={props.close}>{started ? "Close" : "Cancel"}</button>
          {started && <button className="sw-btn" data-testid="start-show" disabled={busy} onClick={() => props.show(started.project, started.slug)}>Show the place ▸</button>}
          {refused && <button className="sw-btn sw-go" data-testid="start-anyway" disabled={busy} onClick={anyway}>{busy ? "Starting…" : "Launch anyway"}</button>}
          {!started && <button className="sw-btn sw-go" data-testid="start-go" disabled={busy || !!problem} onClick={go}>{busy ? "Starting…" : "Start"}</button>}
        </div>
      </div>
    </div>
  );
}

/** "Link a place…": the places of every registered project, the item's own
 *  project first. In the Ideas window a row is also the drag source (the
 *  same-window form of §6.1's "a place onto a work item"); in the main window
 *  the nav's own rows are. */
export function PickPlace(props: {
  title: string;
  snap: Snap;
  /** The item's project, listed first. */
  root?: string;
  act(root: string, slug: string): void;
  arm?(e: React.PointerEvent, root: string, slug: string): void;
  close(): void;
}) {
  const [q, setQ] = useState("");
  useEscape(props.close);
  const rows = useMemo(() => {
    const s = q.trim().toLowerCase();
    const name = (p: { root: string; name?: string | null }) => p.name ?? p.root.split("/").pop() ?? p.root;
    return [...(props.snap?.projects ?? [])]
      .sort((a, b) => Number(b.root === props.root) - Number(a.root === props.root))
      .flatMap((p) => (p.snapshot?.places ?? []).filter((x) => !x.is_main).map((x) => ({ root: p.root, project: name(p), slug: x.slug, title: x.declared?.title ?? "" })))
      .filter((r) => !s || `${r.project} ${r.slug} ${r.title}`.toLowerCase().includes(s));
  }, [props.snap, props.root, q]);
  return (
    <div className="sw-pick" role="dialog" aria-label={props.title} data-testid="pick-place">
      <p className="sw-head">{props.title}</p>
      <input className="sw-filter" autoFocus placeholder="Filter by project or place" value={q} onChange={(e) => setQ(e.target.value)}
        onKeyDown={(e) => { if (e.key === "Enter" && rows[0]) props.act(rows[0].root, rows[0].slug); }} />
      <div className="sw-pick-rows">
        {rows.map((r) => (
          <button key={`${r.root}|${r.slug}`} className="sw-pick-row" data-drag-handle="" data-testid="pick-place-row" data-root={r.root} data-slug={r.slug} title={r.root}
            onPointerDown={(e) => props.arm?.(e, r.root, r.slug)} onClick={() => props.act(r.root, r.slug)}>
            <span className="sw-quiet">{r.project}</span> {r.title || r.slug}
          </button>
        ))}
        {rows.length === 0 && <p className="sw-quiet">{props.snap ? "No place to link." : "Reading the places…"}</p>}
      </div>
      <div className="sw-foot"><button className="sw-btn" onClick={props.close}>Close</button></div>
    </div>
  );
}
