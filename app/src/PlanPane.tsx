// The Plan tab — what this place is FOR, how far along it is, and whether
// anyone is working on it right now.
//
// Fed by files the place's claude session already writes under the
// planning-with-files skill (`task_plan.md`, `.planning/brief.md`). The app
// never writes any of them — same owner rule as `ui-state.json` and
// `~/.claude.json` (CLAUDE.md). Every rule about WHICH file and WHAT it says
// lives in `worktrees_core::plan` and arrives as a `PlanSummary`; this file only
// renders it, so there is no mirror here for a `*-check.mjs` to guard.
//
// The header is a SUMMARY, not a second markdown renderer: a handful of fields
// the core extractor pulled out of a file whose format varies wildly between
// sessions (the survey in the contract: 2 of 15 used the template's phases). The
// body below it is the file itself, rendered by the same `Markdown` component
// the Files tab uses, so the summary can be wrong without hiding anything.
//
// Every string in the payload is session-written free text. It is rendered as
// text nodes only; `Markdown` already renders raw HTML as inert text.
import { useCallback, useEffect, useId, useMemo, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Markdown } from "./markdown";

export type PlanPhaseStatus = "pending" | "in_progress" | "complete" | "blocked" | "unknown";
export type PlanPhase = { name: string; status: PlanPhaseStatus; done: number; total: number };

/** `worktrees_core::plan::PlanSummary`, field for field. */
export type PlanSummary = {
  source: "plan" | "brief" | "none";
  how_resolved: "active_plan" | "newest" | "root" | null;
  plan_path: string | null;
  plan_rel: string | null;
  mtime_ms: number;
  title: string | null;
  goal: string | null;
  current: string | null;
  checks_done: number;
  checks_total: number;
  phases: PlanPhase[];
  errors: number;
  files: { task_plan: boolean; findings: boolean; progress: boolean };
  brief_path: string | null;
  brief_title: string | null;
  brief_lead: string | null;
  markdown: string | null;
  truncated: boolean;
};

export type PlanPaneProps = {
  /** The place directory — what `place_plan` resolves the plan under. */
  root: string;
  repo: string;
  /** The place's slug — the title of last resort. */
  slug: string;
  /** Bumped by `places:changed` and by the header's ↻ (DocsPane's `placesToken`). */
  reloadToken: number;
  /** `useWindowAwake`. A `focus` re-read only happens while the window is visible. */
  pageVisible: boolean;
  /** The place's live claude state (`activityOf`). */
  activity: "busy" | "waiting" | "";
  /** The unsent prompt at claude's prompt, if any (`draftPaths`). */
  draft?: string;
  /** The Files tab's markdown reading size (`files_md_zoom`, a PERCENTAGE,
   *  100 = normal), so the plan reads at the size this place's docs do. */
  mdZoom?: number;
  /** Open a file in the Files tab's renderer. */
  onOpen: (path: string) => void;
  /** Tells the dock header which file its "open" button opens (null: none). */
  onPlanPath?: (path: string | null) => void;
  onError: (e: unknown) => void;
};

/** Compact age from epoch MILLISECONDS, short enough for the status band
 *  ("7m", "3h", "2d"); the band's `title` carries the full timestamp. */
function shortAge(ms: number): string {
  if (!ms) return "";
  const s = Math.floor((Date.now() - ms) / 1000);
  if (s < 60) return "now";
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}

const GLYPH: Record<PlanPhaseStatus, string> = {
  pending: "○", in_progress: "◐", complete: "●", blocked: "✕", unknown: "·",
};
const STATUS_WORD: Record<PlanPhaseStatus, string> = {
  pending: "pending", in_progress: "in progress", complete: "complete", blocked: "blocked", unknown: "status unknown",
};

/** `href` from a rendered plan, against the plan's own directory. */
function resolveFrom(dir: string, href: string): string {
  if (href.startsWith("/")) return href;
  const out: string[] = [];
  for (const p of `${dir}/${href}`.split("/")) {
    if (p === "." || p === "") continue;
    if (p === "..") out.pop();
    else out.push(p);
  }
  return "/" + out.join("/");
}

/** The plan text without its HTML comments, for DISPLAY only.
 *
 *  The skill's template is mostly `<!-- WHAT: … WHY: … -->` guidance, and the
 *  dock's `Markdown` shows raw HTML as literal inert text (by design — see
 *  markdown.tsx), so a fresh template would render as a column of code blocks
 *  around the few lines the session actually wrote. Core's extractor makes the
 *  same pre-pass. Fenced blocks are left byte-for-byte alone, since a comment
 *  inside a fence is code, not guidance. The Files tab (the header's "open")
 *  still shows the file exactly as written. */
export function withoutComments(md: string): string {
  const parts = md.split(/(^(?:```|~~~)[^\n]*\n[\s\S]*?^(?:```|~~~)[ \t]*$)/m);
  // A comment that owns its lines takes its newline with it; one that shares a
  // line with text must not, or it would glue that line to the next.
  const strip = (p: string) => p.replace(/^[ \t]*<!--[\s\S]*?-->[ \t]*\n/gm, "").replace(/<!--[\s\S]*?-->/g, "");
  return parts.map((p, i) => (i % 2 ? p : strip(p))).join("");
}

/** A row of short facts separated by `·`, rendered as TEXT between siblings.
 *
 *  Not `::before` on the items: an item that ellipsises takes its own
 *  pseudo-element with it, and a squeezed span left a bare "· " with nothing
 *  after it (the first cut's meta line). Here each item owns its LEADING
 *  separator as a real sibling of its text, and the row sits one separator
 *  width to the left inside an `overflow: hidden` box, so whichever item
 *  starts a line — the first, or one the row wrapped — has its separator
 *  clipped. A wrap can therefore never leave a separator dangling at either
 *  end of a line. `grow` marks the one item allowed to take (and give back)
 *  the slack; its text ellipsises, its separator does not. */
function SepRow({ className, items }: { className: string; items: { key: string; node: ReactNode; grow?: boolean }[] }) {
  if (items.length === 0) return null;
  return (
    <div className={"plan-seprow " + className}>
      <div className="plan-seprow-in">
        {items.map((it) => (
          <span key={it.key} className={"plan-bi" + (it.grow ? " grow" : "")}>
            <span className="plan-sep" aria-hidden>·</span>
            {it.node}
          </span>
        ))}
      </div>
    </div>
  );
}

/** The phase list. Module scope: a component declared inside `PlanPane` would
 *  be a new identity per render and remount (CLAUDE.md). */
function PhaseRows({ phases, id }: { phases: PlanPhase[]; id?: string }) {
  return (
    <ol className="plan-phases" id={id}>
      {phases.map((ph, i) => (
        <li key={i} className={"plan-phase " + ph.status} title={`${ph.name} — ${STATUS_WORD[ph.status]}`}>
          {/* Hue lives in the glyph only; the words take a text token
              (CLAUDE.md: an accent token is a fill, not text). */}
          <span className="plan-glyph" aria-hidden>{GLYPH[ph.status]}</span>
          <span className="plan-phase-name">{ph.name}</span>
          {/* Blocked is the one state a reader must not miss, and its glyph's
              hue is the weakest channel there is (2.5:1 in nord), so it also
              says so in words. The other states stay a glyph + a screen-reader
              word: a column of "complete" would be noise. */}
          {ph.status === "blocked"
            ? <span className="plan-phase-word">blocked</span>
            : <span className="plan-sr">{STATUS_WORD[ph.status]}</span>}
          {ph.total > 0 && <span className="plan-phase-n">{ph.done}/{ph.total}</span>}
        </li>
      ))}
    </ol>
  );
}

/** The phase the collapsed line names: the one in progress, else the first
 *  that is not done (blocked or next up), else the last. */
function currentPhase(phases: PlanPhase[]): number {
  const ip = phases.findIndex((p) => p.status === "in_progress");
  if (ip >= 0) return ip;
  const open = phases.findIndex((p) => p.status !== "complete");
  return open >= 0 ? open : phases.length - 1;
}

/** "Phase 3: Frontend pane" → "Frontend pane", since the line already says
 *  "Phase 3 of 5". A name that does not follow the template is kept whole. */
function bareName(name: string): string {
  return name.replace(/^phase\s+\d+\s*[:.\u2014\u2013-]\s*/i, "") || name;
}

/** Phases, collapsed by default to ONE line naming the current phase, its
 *  position and its status as a word; the disclosure expands the full list.
 *  Local state and no persistence: `PlanPane` is keyed on the place, so a
 *  switch collapses it again, which is the point — the list is a look-up, the
 *  line is the answer. */
function PhaseSummary({ phases }: { phases: PlanPhase[] }) {
  const [open, setOpen] = useState(false);
  const listId = useId();
  const i = currentPhase(phases);
  const ph = phases[i];
  return (
    <div className="plan-phase-box">
      <button
        type="button"
        className={"plan-phase-toggle " + ph.status}
        data-track="dock.plan.phases"
        aria-expanded={open}
        aria-controls={listId}
        title={open ? "Hide the phase list" : "Show every phase"}
        onClick={() => setOpen((o) => !o)}
      >
        <span className="plan-chev" aria-hidden>{open ? "▾" : "▸"}</span>
        <span className="plan-glyph" aria-hidden>{GLYPH[ph.status]}</span>
        <span className="plan-phase-pos">Phase {i + 1} of {phases.length}</span>
        <span className="plan-sep" aria-hidden>·</span>
        <span className="plan-phase-cur" title={ph.name}>{bareName(ph.name)}</span>
        <span className="plan-sep" aria-hidden>·</span>
        <span className="plan-phase-status">{STATUS_WORD[ph.status]}</span>
      </button>
      {open && <PhaseRows phases={phases} id={listId} />}
    </div>
  );
}

export function PlanPane({ root, slug, reloadToken, pageVisible, activity, draft, mdZoom, onOpen, onPlanPath, onError }: PlanPaneProps) {
  const [plan, setPlan] = useState<PlanSummary | null>(null);
  const [failed, setFailed] = useState(false);
  const [brief, setBrief] = useState<string | null>(null);
  const bodyRef = useRef<HTMLDivElement | null>(null);

  // A stale answer must never paint over a fresh one (DocsPane has the note).
  const seq = useRef(0);
  const load = useCallback(() => {
    const mine = ++seq.current;
    invoke<PlanSummary | null>("place_plan", { root })
      .then((r) => {
        if (seq.current !== mine) return;
        // A typed invoke can still answer `null` (the harness for an unknown
        // command, an older backend) — render as "nothing here", not a crash.
        setPlan(r ?? null);
        setFailed(!r);
      })
      .catch((e) => {
        if (seq.current !== mine) return;
        setPlan(null);
        setFailed(true);
        onError(e);
      });
  }, [root, onError]);

  useEffect(() => { load(); }, [load, reloadToken]);
  // No interval: `places:changed` already bumps `reloadToken`, and coming back
  // to the window is the other moment a plan is likely to have moved. Same
  // `pageVisible` gate as every other poll in the app.
  useEffect(() => {
    if (!pageVisible) return;
    const on = () => load();
    window.addEventListener("focus", on);
    return () => window.removeEventListener("focus", on);
  }, [load, pageVisible]);

  const planPath = plan?.source === "plan" ? plan.plan_path : null;
  // A brief with no plan: the brief IS the document, so it is read in full
  // through the same `read_file` the Files tab uses.
  const briefPath = plan?.source === "brief" ? plan.brief_path : null;
  // What the dock header's "open" button opens: the plan, or — when there is
  // only a brief — the brief, since that is what the body is showing.
  const docPath = planPath ?? briefPath;
  useEffect(() => { onPlanPath?.(docPath); }, [docPath, onPlanPath]);
  useEffect(() => () => onPlanPath?.(null), [onPlanPath]);

  useEffect(() => {
    // Not cleared on a reload of the SAME brief: a blank flash on every
    // `places:changed` would read as the document disappearing.
    if (!briefPath) { setBrief(null); return; }
    let alive = true;
    invoke<{ content: string } | null>("read_file", { path: briefPath })
      .then((r) => { if (alive) setBrief(r?.content ?? null); })
      .catch((e) => { if (alive) onError(e); });
    return () => { alive = false; };
  }, [briefPath, reloadToken, onError]);

  const onLink = useCallback((href: string) => {
    if (/^[a-z][a-z0-9+.-]*:/i.test(href)) {
      // Only http(s) leaves the app — the plan is untrusted input.
      if (/^https?:/i.test(href)) openUrl(href).catch(onError);
      else onError(new Error(`refused to open ${href.split(":")[0]}: link`));
      return;
    }
    if (href.startsWith("#")) {
      const id = decodeURIComponent(href.slice(1));
      bodyRef.current?.querySelector(`[id="${CSS.escape(id)}"]`)?.scrollIntoView({ behavior: "smooth", block: "start" });
      return;
    }
    if (!docPath) return;
    onOpen(resolveFrom(docPath.slice(0, docPath.lastIndexOf("/")), href.split("#")[0]));
  }, [docPath, onOpen, onError]);

  // Above the early returns (hooks keep their order), and memoised: the text
  // can be 512 KiB and the pane re-renders on every activity flip.
  const raw = plan?.source === "plan" ? plan.markdown : plan?.source === "brief" ? brief : null;
  const body = useMemo(() => (raw == null ? null : withoutComments(raw)), [raw]);

  const draftLine = (draft ?? "").split("\n").find((l) => l.trim()) ?? "";
  const actItem = activity ? [{
    key: "act",
    node: (
      <span className="plan-act" title={activity === "busy" ? "Claude is working" : "Claude needs input"}>
        <span className={"status-dot " + activity} aria-hidden />
        <span>{activity === "busy" ? "working" : "needs input"}</span>
      </span>
    ),
  }] : [];
  const draftRow = draftLine ? <div className="plan-draft" title={draft}>✎ {draftLine}</div> : null;
  // Before a summary exists (and when there is nothing to summarise) the band
  // carries only the live state, so a waiting session is visible either way.
  const status = actItem.length > 0 || draftRow ? (
    <>
      <SepRow className="plan-band" items={actItem} />
      {draftRow}
    </>
  ) : null;

  if (!plan) {
    return (
      <div className="planpane">
        {status && <div className="plan-head">{status}</div>}
        <div className="plan-note">{failed ? "could not read this place's plan" : "reading…"}</div>
      </div>
    );
  }

  if (plan.source === "none") {
    return (
      <div className="planpane">
        {status && <div className="plan-head">{status}</div>}
        <div className="plan-empty">
          <div className="plan-empty-card">
            <div className="plan-empty-title">No plan or brief here yet</div>
            <div className="plan-empty-sub">
              This tab reads <code>.planning/brief.md</code> and a{" "}
              <code>task_plan.md</code> (under <code>.planning/</code> or at the
              place root) when this place's session writes them.
            </div>
          </div>
        </div>
      </div>
    );
  }

  // Brief first: the brief is the one document the TOOL wrote, so its title and
  // lead sentence say what this place is FOR on every briefed place, whatever
  // shape the session's plan took. Core clamps both to 280 chars.
  const title = plan.brief_title || plan.title || slug;
  const lead = plan.brief_lead || plan.goal;
  // With both files, the plan's own title names the WORKSTREAM under the
  // assignment — a second, dimmer line, never above the brief.
  const sub = plan.source === "plan" && plan.brief_title && plan.title && plan.title !== plan.brief_title
    ? plan.title : null;
  const pct = plan.checks_total > 0 ? Math.round((plan.checks_done / plan.checks_total) * 100) : 0;
  const missing = plan.source === "plan"
    ? [!plan.files.findings && "findings.md", !plan.files.progress && "progress.md"].filter(Boolean) as string[]
    : [];
  const age = shortAge(plan.mtime_ms);
  const hasPhases = plan.phases.length > 0;
  // `current` usually IS the current phase's heading; when the phase line
  // below already names it, the band does not say it twice.
  const curPh = hasPhases ? plan.phases[currentPhase(plan.phases)] : null;
  const showCurrent = !!plan.current && !(curPh && curPh.name.trim() === plan.current.trim());

  const band: { key: string; node: ReactNode; grow?: boolean }[] = [...actItem];
  if (plan.checks_total > 0) band.push({
    key: "prog",
    node: (
      <span className="plan-prog">
        <span
          className="plan-bar"
          role="progressbar"
          aria-valuemin={0}
          aria-valuemax={plan.checks_total}
          aria-valuenow={plan.checks_done}
          aria-label="Checklist progress"
        >
          <span className="plan-bar-fill" style={{ width: `${pct}%` } as CSSProperties} />
        </span>
        <span className="plan-count">{plan.checks_done}/{plan.checks_total}</span>
      </span>
    ),
  });
  if (age) band.push({
    key: "age",
    node: <span className="plan-age" title={`changed ${new Date(plan.mtime_ms).toLocaleString()}`}>{age}</span>,
  });
  // `current` LAST: it is the one long item, so when the band cannot hold it
  // (the 240px floor with a live state) it wraps to a line of its own at full
  // width, rather than the age orphaning onto line 2 while `current` shrinks
  // to three letters on line 1.
  if (showCurrent) band.push({ key: "cur", grow: true, node: <span className="plan-current" title={plan.current!}>{plan.current}</span> });

  const shorts: { key: string; node: ReactNode }[] = [];
  if (plan.source === "brief") shorts.push({ key: "nop", node: <span>no task_plan.md</span> });
  if (plan.errors > 0) shorts.push({ key: "err", node: <span className="plan-errs">{plan.errors} {plan.errors === 1 ? "error" : "errors"} logged</span> });
  if (missing.length > 0) shorts.push({ key: "miss", node: <span className="plan-missing">no {missing.join(" / ")}</span> });

  return (
    <div className="planpane">
      <div className="plan-head">
        <div className="plan-title" title={title}>{title}</div>
        {lead && (
          // A brief-only place shows the WHOLE lead: that sentence is the
          // entire document. With a plan under it, 6 lines at most (see CSS).
          <div className={"plan-lead" + (plan.source === "plan" ? " clamp" : "")} title={lead}>{lead}</div>
        )}
        {sub && <div className="plan-sub" title={sub}>{sub}</div>}
        <SepRow className="plan-band" items={band} />
        {draftRow}
        {hasPhases && <PhaseSummary phases={plan.phases} />}
        <div className="plan-meta">
          {plan.source === "plan" ? (
            <div className="plan-file" title={plan.plan_path ?? undefined}>{plan.plan_rel}</div>
          ) : (
            <div className="plan-file" title={plan.brief_path ?? undefined}>.planning/brief.md</div>
          )}
          <SepRow className="plan-meta-row" items={shorts} />
        </div>
        {plan.truncated && (
          <div className="plan-trunc">Only the first 512 KiB of this plan is shown.</div>
        )}
      </div>
      {body != null && (
        <div ref={bodyRef} className="scroll plan-body" style={{ "--md-zoom": String((mdZoom ?? 100) / 100) } as CSSProperties}>
          <Markdown src={body} onLink={onLink} />
        </div>
      )}
    </div>
  );
}
