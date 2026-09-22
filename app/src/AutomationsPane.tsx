// The Automations tab — this PROJECT's briefs, the runs they left behind, and
// the one report a run is.
//
// Three rules from `docs/proposals/automations.md` shape every line here:
//
//  1. **The content is the PROJECT's, not the place's.** The tab is reached
//     from any place of a project (like Files or Docs), but what it shows is
//     the same for all of them — so the pane is keyed on `repo`, and its header
//     names the project. That is the Docs tab's lesson (§7.1): a surface shown
//     from many places must say whose it is.
//  2. **A run REPORTS; a person acts.** Phase 1 has one tier, `report`. A
//     finding's proposals are BUTTONS, and pressing one is a separate call
//     (`apply_proposal`) that core validates all over again. Nothing here
//     applies anything on its own, and `remove_worktree` is not in the set.
//  3. **A refusal is never invisible** (`diag.rs`'s rule). A run's `dropped`
//     entries — proposals the validator threw away, including "claude proposed
//     removing a worktree" — render as their own block with the reason.
//
// Every string that comes out of a run is model-written free text: findings,
// the report, a dropped entry's `why`. They are rendered as text nodes, and the
// report goes through the same `Markdown` component the Docs tab uses, which
// renders raw HTML as inert text.
import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { CtxMenu } from "./CtxMenu";
import * as Icons from "./icons";
import { Markdown } from "./markdown";
import { predictTier } from "./dnd";
import { useEscape } from "./useEscape";
import { STARTERS } from "./automations";
import type { ProfilesInfo } from "./ProfilesPanel";

// ── the backend's views, field for field (lib.rs) ────────────────────────────

/** `automation::When`, verbatim. Posted back unchanged, so the vocabulary has
 *  exactly one home — core's enum. */
export type When =
  | { kind: "manual" }
  | { kind: "daily"; at: string }
  | { kind: "weekly"; day: string; at: string };

export type RunSummary = {
  id: string;
  automation: string;
  trigger: string;
  started_epoch: number;
  finished_epoch: number | null;
  status: string;
  /** A COUNT — the list says "3 findings"; the findings themselves come with
   *  `get_run`, so the 2s poll does not carry them. */
  findings: number;
  dropped: number;
  actions: number;
  seconds: number | null;
  error: string | null;
};

export type AutomationRow = {
  slug: string;
  name: string;
  brief: string;
  when: When;
  scope: string;
  tier: string;
  enabled: boolean;
  created_epoch: number;
  last_run: RunSummary | null;
};

export type Proposal = { tool: string; args: Record<string, unknown> };
export type Finding = { slug: string; text: string; proposals: Proposal[] };
export type Dropped = { what: string; why: string };
export type Skipped = { slug: string; why: string };
export type RunAction = { epoch: number; tool: string; args: Record<string, unknown>; ok: boolean; output: string };

/** The core `Run` minus `facts` (see `RunView` in lib.rs). */
export type Run = {
  id: string;
  automation: string;
  trigger: string;
  started_epoch: number;
  finished_epoch: number | null;
  status: string;
  profile: string | null;
  places: string[];
  skipped: Skipped[];
  findings: Finding[];
  dropped: Dropped[];
  actions: RunAction[];
  report_md: string | null;
  turns: number | null;
  seconds: number | null;
  error: string | null;
  seen_epoch: number | null;
};

/** The slice of App's `Place` this pane reads — structural, like `PlanPlace`,
 *  so App hands over its own `Place` untouched. */
export type AutomationPlace = {
  slug: string;
  is_main: boolean;
  lifecycle_effective: string;
  tmux_session: { up: boolean };
  declared: { title?: string; pinned?: boolean; lifecycle?: string; last_opened_epoch?: number } | null;
};

export type AutomationsPaneProps = {
  /** The project root. The pane is keyed on this, not on a place. */
  repo: string;
  /** The nav's own label for the project (App's `basename(root)`). */
  projectName: string;
  /** This project's places — for slug → place lookups in the run view. */
  places: AutomationPlace[];
  /** `useWindowAwake`. Every poll here is gated on it, like `useUsage`. */
  pageVisible: boolean;
  /** Bumped by `places:changed` (App's `placesToken`). */
  reloadToken: number;
  /** The Files tab's markdown reading size, so a report reads at the size this
   *  project's documents do. */
  mdZoom?: number;
  onSelectPlace: (slug: string) => void;
  /** App's `patchDeclared`, bound to this repo. `effective` is the PREDICTED
   *  lifecycle badge — see the call site below for why it is not the label. */
  onPatchDeclared: (slug: string, patch: Record<string, unknown>, effective?: string) => void;
  /** App's `refresh` — the workspace re-read that confirms an optimistic edit. */
  onRefresh: () => void | Promise<void>;
  onError: (e: unknown) => void;
};

// ── words ────────────────────────────────────────────────────────────────────

const DAYS = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
const DAY_PLURAL: Record<string, string> = {
  mon: "Mondays", tue: "Tuesdays", wed: "Wednesdays", thu: "Thursdays",
  fri: "Fridays", sat: "Saturdays", sun: "Sundays",
};

/** `when` as the words the tab and the modal both use. Core has its own
 *  `When::label()` for the CLI ("daily at 08:00"); this one matches the modal's
 *  segment names, so what a row says is what the control you set it with said. */
export function whenLabel(w: When | null | undefined): string {
  if (!w) return "when I ask";
  if (w.kind === "daily") return `every morning · ${w.at}`;
  if (w.kind === "weekly") return `${DAY_PLURAL[w.day] ?? w.day} · ${w.at}`;
  return "when I ask";
}

/** Phase 1 ships one tier, and the row still says which — the second and third
 *  arrive in phase 3, and a row that said nothing today would change meaning
 *  under the reader then. */
export function tierLabel(t: string): string {
  return t === "report" ? "report only" : t;
}

/** `72` → `1m12s`, `41` → `41s`. Null when the run has not finished. */
function duration(secs: number | null | undefined): string | null {
  if (secs == null) return null;
  if (secs < 60) return `${secs}s`;
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return s ? `${m}m${s}s` : `${m}m`;
}

const pad2 = (n: number) => String(n).padStart(2, "0");

/** Local wall-clock time of day, `08:02`. */
function timeOf(epoch: number): string {
  const d = new Date(epoch * 1000);
  return `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/** The heading a run's day gets: "Today", "Yesterday", then `Mon 15 Sep`.
 *  Compared as CALENDAR days in local time — a run 20 hours ago can be
 *  yesterday or today, and the reader means the date, not the elapsed hours. */
export function dayLabel(epoch: number, now = Date.now()): string {
  const d = new Date(epoch * 1000);
  const t = new Date(now);
  const midnight = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diff = Math.round((midnight(t) - midnight(d)) / 86_400_000);
  if (diff === 0) return "Today";
  if (diff === 1) return "Yesterday";
  return `${WEEKDAYS[d.getDay()]} ${d.getDate()} ${MONTHS[d.getMonth()]}`;
}

/** The dot beside a run's result. Hue lives HERE and nowhere else on the row —
 *  the words take `--txt-hi`/`--txt-dim`, because an accent token is a fill and
 *  not text (CLAUDE.md). */
function statusTone(status: string): "warn" | "ok" | "danger" | "mute" | "run" {
  if (status === "findings") return "warn";
  if (status === "clean") return "ok";
  if (status === "failed") return "danger";
  if (status === "running") return "run";
  return "mute";
}

/** What a finished run says in words, beside its dot. */
function resultWords(r: RunSummary): string {
  if (r.status === "running") return "running…";
  if (r.status === "failed") return `failed · ${r.error ?? "no reason recorded"}`;
  const d = duration(r.seconds);
  const head = r.status === "findings"
    ? `${r.findings} ${r.findings === 1 ? "finding" : "findings"}`
    : "clean";
  return d ? `${head} · ${d}` : head;
}

/** A proposal's button label, by tool. The set is closed
 *  (`runs::PROPOSAL_TOOLS`) and mirrored in `automations.ts`; an unknown tool
 *  cannot reach here (core drops it into `dropped`), and if one did it would
 *  name itself rather than render a blank button. */
export function proposalLabel(p: Proposal): string {
  switch (p.tool) {
    case "set_lifecycle": return `Mark ${String(p.args.lifecycle ?? "")}`.trim();
    case "set_note": return "Add note";
    case "set_pin": return p.args.pinned === false ? "Unpin" : "Pin";
    case "close_session": return "Close session";
    default: return p.tool;
  }
}

/** Has this exact call already been made on this run?
 *
 *  ⚠ NOT by `(finding, proposal)` index: `runs::Action` records `tool` + `args`
 *  and no indices at all, so the pair the proposal document describes is not
 *  recoverable from the ledger. The call itself is the identity — two proposals
 *  that would make the identical call have the identical effect, so treating
 *  them as one is right rather than merely convenient. Both sides are
 *  serialised by the same Rust `Map`, so the key order matches by construction. */
function alreadyApplied(run: Run, p: Proposal): RunAction | null {
  const key = JSON.stringify(p.args);
  return run.actions.find((a) => a.tool === p.tool && JSON.stringify(a.args) === key) ?? null;
}

// ── rows ─────────────────────────────────────────────────────────────────────

function Dot({ tone }: { tone: ReturnType<typeof statusTone> }) {
  return <span className={"auto-dot " + tone} aria-hidden />;
}

/** One automation. Module scope — a component declared inside the pane would be
 *  a new identity every render and remount its children (CLAUDE.md). */
function AutomationItem({ a, running, onOpen, onRun, onMenu }: {
  a: AutomationRow;
  running: boolean;
  onOpen: () => void;
  onRun: () => void;
  onMenu: (e: React.MouseEvent) => void;
}) {
  const last = a.last_run;
  return (
    <div
      className="auto-row"
      role="button"
      tabIndex={0}
      data-testid={`auto-row-${a.slug}`}
      data-track="dock.automations.edit"
      title={a.brief}
      onClick={onOpen}
      onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); onOpen(); } }}
      onContextMenu={onMenu}
    >
      <div className="auto-row-id">
        <div className="auto-name">{a.name}</div>
        <div className="auto-when">{whenLabel(a.when)} · {tierLabel(a.tier)}</div>
      </div>
      <div className="auto-last">
        {last ? (
          <>
            <Dot tone={statusTone(last.status)} />
            <span className="auto-last-words">{resultWords(last)}</span>
          </>
        ) : (
          <span className="auto-never">never ran</span>
        )}
      </div>
      <button
        type="button"
        className="ctrl sm icon-only auto-play"
        data-track="dock.automations.run"
        aria-label={`Run ${a.name} now`}
        title={running ? "already running" : "Run now"}
        disabled={running}
        onClick={(e) => { e.stopPropagation(); onRun(); }}
      >
        {running ? <span className="auto-spin" aria-hidden>◐</span> : <Icons.Play size={11} />}
      </button>
    </div>
  );
}

/** One run in the RUNS list. */
function RunItem({ r, name, onOpen }: { r: RunSummary; name: string; onOpen: () => void }) {
  return (
    <div
      className="auto-runrow"
      role="button"
      tabIndex={0}
      data-testid={`auto-run-${r.id}`}
      data-track="dock.automations.open-run"
      title={new Date(r.started_epoch * 1000).toLocaleString()}
      onClick={onOpen}
      onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); onOpen(); } }}
    >
      <span className="auto-time">{timeOf(r.started_epoch)}</span>
      <Dot tone={statusTone(r.status)} />
      <span className="auto-runname">{name}</span>
      <span className="auto-runres">{resultWords(r)}</span>
    </div>
  );
}

// ── the run view ─────────────────────────────────────────────────────────────

/** One finding's card: which place, what about it, and the buttons. */
function FindingCard({ run, finding, index, place, busy, onSelectPlace, onApply }: {
  run: Run;
  finding: Finding;
  index: number;
  place: AutomationPlace | undefined;
  busy: string | null;
  onSelectPlace: (slug: string) => void;
  onApply: (findingIx: number, proposalIx: number, p: Proposal) => void;
}) {
  return (
    <div className="auto-find" data-testid={`auto-finding-${index}`}>
      <div className="auto-find-h">
        <button
          type="button"
          className="auto-placelink"
          data-track="dock.automations.place"
          title={`Select ${finding.slug} in the nav`}
          onClick={() => onSelectPlace(finding.slug)}
        >
          {finding.slug}
        </button>
        {place && <span className={"life " + place.lifecycle_effective}>{place.lifecycle_effective}</span>}
      </div>
      <div className="auto-find-text">{finding.text}</div>
      {finding.proposals.length > 0 && (
        <div className="auto-props">
          {finding.proposals.map((p, pi) => {
            const done = alreadyApplied(run, p);
            const key = `${index}:${pi}`;
            if (done) {
              return (
                <span key={key} className="auto-prop-done">
                  {/* Disabled, so it records nothing — but `usage-check.mjs`
                      keys a button on its `title` when nothing else names it,
                      and this title is what the CALL said, i.e. model-adjacent
                      free text. The key has to be a constant that lives in
                      this repo's source, so it is a literal testid and not the
                      interpolated one that would have been more convenient. */}
                  <button type="button" className="ctrl sm" disabled
                    data-testid="auto-applied" title={done.output}>
                    Applied ✓
                  </button>
                  {!done.ok && <span className="auto-prop-err">{done.output}</span>}
                </span>
              );
            }
            return (
              <button
                key={key}
                type="button"
                className={pi === 0 ? "enter-btn sm" : "ctrl sm"}
                data-testid={`auto-prop-${index}-${pi}`}
                data-track="dock.automations.apply"
                disabled={busy === key}
                title={JSON.stringify(p.args)}
                onClick={() => onApply(index, pi, p)}
              >
                {busy === key ? "…" : proposalLabel(p)}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

/** A run's report: the meta line, the findings, what was refused, and Claude's
 *  read. Replaces the two lists while it is open (proposal §7.2). */
function RunReport({ run, name, places, mdZoom, busy, onBack, onAgain, onSelectPlace, onApply, onCopy }: {
  run: Run;
  name: string;
  places: AutomationPlace[];
  mdZoom?: number;
  busy: string | null;
  onBack: () => void;
  onAgain: () => void;
  onSelectPlace: (slug: string) => void;
  onApply: (findingIx: number, proposalIx: number, p: Proposal) => void;
  onCopy: () => void;
}) {
  const byslug = useMemo(() => new Map(places.map((p) => [p.slug, p])), [places]);
  // Each item is dropped when it is null, rather than rendered as an empty
  // segment: "today 08:02 · · 3 findings" reads as a missing value, and the
  // value that is missing most often (`seconds`, on a running run) is the one
  // the reader is least likely to notice.
  const meta = [
    `${dayLabel(run.started_epoch).toLowerCase()} ${timeOf(run.started_epoch)}`,
    duration(run.seconds),
    run.profile ? `profile ${run.profile}` : null,
    run.status === "findings"
      ? `${run.findings.length} ${run.findings.length === 1 ? "finding" : "findings"}`
      : run.status === "clean" ? "clean" : null,
  ].filter(Boolean) as string[];

  return (
    <div className="auto-run" data-testid="auto-run-view">
      <div className="auto-run-h">
        <button
          type="button"
          className="ctrl sm icon-only"
          data-testid="auto-back"
          data-track="dock.automations.back"
          aria-label="Back to the automations list"
          title="Back"
          onClick={onBack}
        >
          <Icons.ChevronLeft size={12} />
        </button>
        <span className="auto-run-name">{name}</span>
        <span className="dock-spacer" />
        <button
          type="button"
          className="ctrl sm"
          data-track="dock.automations.again"
          title="Run this automation again"
          onClick={onAgain}
        >
          Run again
        </button>
      </div>
      <div className="scroll auto-run-body">
        <div className="auto-meta">
          <Dot tone={statusTone(run.status)} />
          <span>{meta.join(" · ")}</span>
        </div>

        {run.status === "failed" && (
          <>
            <div className="auto-sec">FAILED</div>
            {/* The reason, verbatim and unwrapped: it is a stderr tail, and
                re-flowing it hides where the command broke. */}
            <pre className="auto-err" data-testid="auto-run-error">{run.error ?? "no reason recorded"}</pre>
          </>
        )}

        {run.findings.length > 0 && (
          <>
            <div className="auto-sec">FINDINGS</div>
            {run.findings.map((f, i) => (
              <FindingCard
                key={i}
                run={run}
                finding={f}
                index={i}
                place={byslug.get(f.slug)}
                busy={busy}
                onSelectPlace={onSelectPlace}
                onApply={onApply}
              />
            ))}
          </>
        )}
        {run.status === "clean" && run.findings.length === 0 && (
          <div className="auto-note">Nothing to report — every worktree looked fine.</div>
        )}

        {run.skipped.length > 0 && (
          <>
            <div className="auto-sec">SKIPPED</div>
            <ul className="auto-dropped">
              {run.skipped.map((s, i) => <li key={i}><b>{s.slug}</b> — {s.why}</li>)}
            </ul>
          </>
        )}

        {/* NEVER collapsed away, however dull it looks: "claude proposed
            removing a worktree and we said no" is the single most important
            thing a run can say about itself (runs.rs on `Dropped`). */}
        {run.dropped.length > 0 && (
          <>
            <div className="auto-sec">NOT SHOWN</div>
            <ul className="auto-dropped" data-testid="auto-dropped">
              {run.dropped.map((d, i) => <li key={i}><b>{d.what}</b> — {d.why}</li>)}
            </ul>
          </>
        )}

        {run.report_md && (
          <>
            <div className="auto-sec">CLAUDE'S READ</div>
            {/* The same `.md` block the Docs tab renders, at the same reading
                size. `--md-zoom` is the block's one knob; the fences inside it
                size off it too (CLAUDE.md on `--md-zoom`). */}
            <div className="auto-md" style={{ "--md-zoom": String((mdZoom ?? 100) / 100) } as CSSProperties}>
              <Markdown src={run.report_md} />
            </div>
          </>
        )}
        {run.status === "running" && (
          <div className="auto-note"><span className="auto-spin" aria-hidden>◐</span> running…</div>
        )}
      </div>
      <div className="auto-run-foot">
        <button type="button" className="ctrl sm" data-track="dock.automations.copy" onClick={onCopy}>
          Copy as markdown
        </button>
      </div>
    </div>
  );
}

/** The run, as markdown, for the clipboard: the findings the app rendered as
 *  cards plus the report itself. A screenshot of the tab is not pasteable into
 *  the session that would act on it; this is. */
export function runAsMarkdown(run: Run, name: string): string {
  const out: string[] = [`# ${name} — ${new Date(run.started_epoch * 1000).toLocaleString()}`, ""];
  if (run.status === "failed") out.push(`**failed:** ${run.error ?? "no reason recorded"}`, "");
  for (const f of run.findings) {
    out.push(`- **${f.slug}** — ${f.text}`);
    for (const p of f.proposals) out.push(`  - proposed: ${proposalLabel(p)} (\`${p.tool}\`)`);
  }
  if (run.findings.length) out.push("");
  for (const d of run.dropped) out.push(`- not shown: ${d.what} — ${d.why}`);
  if (run.dropped.length) out.push("");
  if (run.report_md) out.push(run.report_md);
  return out.join("\n");
}

// ── the modal ────────────────────────────────────────────────────────────────

type WhenKind = "manual" | "daily" | "weekly";

export type DialogSeed = {
  /** The slug being edited; null for a create. */
  slug: string | null;
  name: string;
  brief: string;
  when: When;
  scope: string;
};

/** New / edit. `NewPlaceDialog`'s shape, deliberately: same scrim, same modal
 *  classes, `useEscape`, first field autofocused, and Enter does NOT submit —
 *  the brief is multi-line, and a dialog whose primary field is a textarea
 *  cannot take Enter away from it. */
export function AutomationDialog({ seed, projectName, profileName, busy, error, onSave, onClose }: {
  seed: DialogSeed;
  projectName: string;
  profileName: string | null;
  busy: boolean;
  /** Core's refusal, verbatim — its wording names the cause and the fix. */
  error: string | null;
  onSave: (patch: { name: string; brief: string; when: When; scope: string }, andRun: boolean) => void;
  onClose: () => void;
}) {
  const [name, setName] = useState(seed.name);
  const [brief, setBrief] = useState(seed.brief);
  const [kind, setKind] = useState<WhenKind>(seed.when.kind);
  const [at, setAt] = useState(seed.when.kind === "manual" ? "08:00" : seed.when.at);
  const [day, setDay] = useState(seed.when.kind === "weekly" ? seed.when.day : "mon");
  const [scope, setScope] = useState(seed.scope);
  const first = useRef<HTMLInputElement | null>(null);

  // A dialog mid-save keeps its entry and makes `fn` a no-op, rather than
  // dropping it — dropping it would hand Escape to whatever is beneath
  // (useEscape's header).
  useEscape(() => { if (!busy) onClose(); });

  useEffect(() => {
    // `preventScroll`: a focus() inside a clipping ancestor scrolls that box,
    // which is how the header lost its place name once (CLAUDE.md).
    first.current?.focus({ preventScroll: true });
  }, []);

  const when: When =
    kind === "daily" ? { kind: "daily", at }
    : kind === "weekly" ? { kind: "weekly", day, at }
    : { kind: "manual" };

  // Mirrors core's own validation (`upsert` + `When::validate`) so the button
  // is honest; core still checks, and its message is what gets shown on a
  // refusal — this only decides whether the press is worth making.
  const timeOk = kind === "manual" || /^\d{2}:\d{2}$/.test(at);
  const ready = !busy && name.trim() !== "" && brief.trim() !== "" && timeOk;
  const editing = seed.slug !== null;

  return (
    <div className="scrim scrim-center" onClick={() => !busy && onClose()}>
      <div
        className="sync-modal nw-modal auto-modal"
        role="dialog"
        aria-label={editing ? "Edit automation" : "New automation"}
        data-testid="automation-dialog"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="sync-h">
          <b>{editing ? "Edit automation" : "New automation"}</b>
          <span className="sync-hub">{editing ? seed.slug : projectName}</span>
        </header>
        <div className="sync-body auto-body">
          <label className="np-field">
            <span className="np-label">Name</span>
            <input
              ref={first}
              className="np-input"
              data-testid="auto-name"
              value={name}
              disabled={busy}
              spellCheck={false}
              onChange={(e) => setName(e.currentTarget.value)}
            />
            {editing && <span className="np-hint">the slug stays <code>{seed.slug}</code> — it keys every run already recorded</span>}
          </label>

          <label className="np-field">
            <span className="np-label">What should Claude do?</span>
            <textarea
              className="np-input auto-brief"
              data-testid="auto-brief"
              rows={8}
              value={brief}
              disabled={busy}
              onChange={(e) => setBrief(e.currentTarget.value)}
            />
            <span className="np-hint">a paragraph, in your own words — it is read as prose, not as commands</span>
          </label>

          <div className="np-field">
            <span className="np-label">When</span>
            <div className="auto-seg" role="group" aria-label="when this runs">
              {([["manual", "When I ask"], ["daily", "Every morning"], ["weekly", "Weekly"]] as [WhenKind, string][]).map(([k, label]) => (
                <button
                  key={k}
                  type="button"
                  className={kind === k ? "on" : ""}
                  data-testid={`auto-when-${k}`}
                  disabled={busy}
                  onClick={() => setKind(k)}
                >
                  {label}
                </button>
              ))}
            </div>
            {kind !== "manual" && (
              <div className="auto-whenrow">
                {kind === "weekly" && (
                  <select className="np-input" data-testid="auto-day" value={day} disabled={busy}
                    onChange={(e) => setDay(e.currentTarget.value)}>
                    {DAYS.map((d) => <option key={d} value={d}>{DAY_PLURAL[d]}</option>)}
                  </select>
                )}
                <input
                  className="np-input auto-at"
                  data-testid="auto-at"
                  value={at}
                  disabled={busy}
                  placeholder="08:00"
                  spellCheck={false}
                  onChange={(e) => setAt(e.currentTarget.value)}
                />
                {!timeOk && <span className="auto-inline-err">HH:MM, 24-hour</span>}
              </div>
            )}
            {/* The field is stored and validated today and evaluated in phase 2
                (§6.2). Saying so is the difference between a feature that is
                coming and one that is silently broken. */}
            <span className="np-hint">Scheduled runs arrive in the next version; saved schedules start then.</span>
          </div>

          <div className="np-field">
            <span className="np-label">Which worktrees</span>
            <div className="auto-seg" role="group" aria-label="which worktrees">
              {([["all", "All of them"], ["brief", "Let the brief say"]] as [string, string][]).map(([k, label]) => (
                <button key={k} type="button" className={scope === k ? "on" : ""}
                  data-testid={`auto-scope-${k}`} disabled={busy} onClick={() => setScope(k)}>
                  {label}
                </button>
              ))}
            </div>
          </div>

          <div className="np-field">
            <span className="np-label">What it may change</span>
            {/* ⚠ One tier ships (§10.1), and the other two are RENDERED, disabled.
                Hiding them would make phase 1 look like the whole design; showing
                them enabled would run a job under permissions core will not
                grant. Core's `Tier` has exactly one variant, so nothing here can
                post a second one even by accident. */}
            <label className="auto-radio">
              <input type="radio" name="tier" checked readOnly disabled={busy} data-testid="auto-tier-report" />
              <span>
                <b>Report only</b>
                <i>It reads, and writes a report with proposals you press yourself.</i>
              </span>
            </label>
            <label className="auto-radio off" title="next version">
              <input type="radio" name="tier" disabled />
              <span>
                <b>Notes, pins, lifecycle</b>
                <i>It could also set notes, pins and lifecycle — reversible, every call logged.</i>
              </span>
            </label>
            <label className="auto-radio off" title="next version">
              <input type="radio" name="tier" disabled />
              <span>
                <b>Also close sessions</b>
                <i>It could also close a session in a worktree with no live agent.</i>
              </span>
            </label>
          </div>

        </div>
        {/* ⚠ BOTH of these are OUTSIDE `.sync-body`, the modal's only scrolling
            child, and for the same reason: inside it they sat below the fold at
            700px. A refusal you have to scroll to find reads as "Save did
            nothing" — the silent failure the never-swallow-an-error rule exists
            to prevent, arrived at through layout instead of through code — and
            the line naming the profile a run launches as, and swearing it can
            never remove a worktree, is a PERMISSIONS statement that has to be
            readable at the moment you press the button, not after it. Same fix,
            and the same reason, as the offers band pinned outside
            `.settings-body` (CLAUDE.md). */}
        <div className="auto-foot-note auto-dialog-note">
          Runs headless as this project's AI profile {profileName ? <b>{profileName}</b> : <i>(the default)</i>}. It can never remove a worktree.
        </div>
        {error && <div className="sync-err auto-dialog-err" data-testid="auto-dialog-error">{error}</div>}
        <footer className="sync-foot">
          <button className="ctrl" onClick={onClose} disabled={busy}>Cancel</button>
          <button className="ctrl" data-testid="auto-save" disabled={!ready}
            onClick={() => onSave({ name: name.trim(), brief, when, scope }, false)}>
            {busy ? "Saving…" : "Save"}
          </button>
          {/* Absent in edit mode: "try it" is what the empty state's starters
              lead to, and an edit already has a Run now on its row. */}
          {!editing && (
            <button className="enter-btn" data-testid="auto-save-run" disabled={!ready}
              onClick={() => onSave({ name: name.trim(), brief, when, scope }, true)}>
              Save &amp; run now
            </button>
          )}
        </footer>
      </div>
    </div>
  );
}

// ── the empty state ──────────────────────────────────────────────────────────

function EmptyState({ onStarter, onBlank }: {
  onStarter: (i: number) => void;
  onBlank: () => void;
}) {
  return (
    <div className="auto-empty" data-testid="auto-empty">
      <div className="auto-empty-bolt" aria-hidden><Icons.Zap size={28} /></div>
      <p className="auto-empty-lead">
        An automation is a brief that Claude runs across this project's worktrees, on a
        schedule or whenever you ask. Every run leaves a report here.
      </p>
      <div className="auto-sec">START FROM ONE</div>
      <div className="auto-starters">
        {STARTERS.map((s, i) => (
          <button
            key={s.name}
            type="button"
            className="auto-starter"
            data-testid={`auto-starter-${i}`}
            data-track="dock.automations.starter"
            onClick={() => onStarter(i)}
          >
            <b>{s.name}</b>
            {/* The first sentence only: the card is an invitation to read the
                rest in the modal, where it is editable. */}
            <i>{s.brief.split(". ")[0]}.</i>
          </button>
        ))}
      </div>
      <button type="button" className="auto-own" data-testid="auto-write-own" onClick={onBlank}>
        Write your own…
      </button>
    </div>
  );
}

// ── the pane ─────────────────────────────────────────────────────────────────

const FIRST_RUNS = 10;
const POLL_MS = 2000;

export function AutomationsPane({
  repo, projectName, places, pageVisible, reloadToken, mdZoom,
  onSelectPlace, onPatchDeclared, onRefresh, onError,
}: AutomationsPaneProps) {
  const [autos, setAutos] = useState<AutomationRow[] | null>(null);
  const [runs, setRuns] = useState<RunSummary[]>([]);
  const [openId, setOpenId] = useState<string | null>(null);
  const [run, setRun] = useState<Run | null>(null);
  const [seed, setSeed] = useState<DialogSeed | null>(null);
  const [dialogBusy, setDialogBusy] = useState(false);
  const [dialogErr, setDialogErr] = useState<string | null>(null);
  const [applying, setApplying] = useState<string | null>(null);
  const [allRuns, setAllRuns] = useState(false);
  const [ctx, setCtx] = useState<{ x: number; y: number; slug: string } | null>(null);
  const [armed, setArmed] = useState<string | null>(null);
  const [profileName, setProfileName] = useState<string | null>(null);
  const [starting, setStarting] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);

  // A stale answer must never paint over a fresh one — the same ticket the
  // other panes use (DocsPane has the long note).
  const seq = useRef(0);
  const load = useCallback(async () => {
    const mine = ++seq.current;
    try {
      const [a, r] = await Promise.all([
        invoke<AutomationRow[]>("list_automations", { repo }),
        invoke<RunSummary[]>("list_runs", { repo, automation: null }),
      ]);
      if (seq.current !== mine) return;
      setAutos(a ?? []);
      setRuns(r ?? []);
    } catch (e) {
      if (seq.current !== mine) return;
      setAutos((cur) => cur ?? []);
      onError(e);
    }
  }, [repo, onError]);

  useEffect(() => { void load(); }, [load, reloadToken]);

  // The profile the runs launch as. On mount only: `profiles_info` does a
  // filesystem probe per profile, so it must never ride a poll (ProjectSheet
  // has the same note for the same call).
  useEffect(() => {
    let alive = true;
    invoke<ProfilesInfo | null>("profiles_info", { repo })
      .then((i) => {
        if (!alive || !i) return;
        const id = i.effective_id ?? null;
        setProfileName(id ? (i.profiles ?? []).find((p) => p.id === id)?.name ?? id : null);
      })
      // Swallowing this would leave the footer claiming a profile it does not
      // know — and the footer is a permissions statement.
      .catch((e) => onError(e));
    return () => { alive = false; };
  }, [repo, onError]);

  const anyRunning = useMemo(
    () => runs.some((r) => r.status === "running"),
    [runs],
  );

  // While a run is in flight the ledger is the only thing that changes, and it
  // changes on disk — so this is the one poll in the pane. Gated on
  // `pageVisible` exactly like `useUsage`: a hidden window is not being read.
  useEffect(() => {
    if (!anyRunning || !pageVisible) return;
    const t = setInterval(() => { void load(); }, POLL_MS);
    return () => clearInterval(t);
  }, [anyRunning, pageVisible, load]);

  // The open run re-reads on the same beat, so a spinner becomes a report
  // without a click.
  const reloadRun = useCallback(async (id: string) => {
    try {
      const r = await invoke<Run>("get_run", { repo, id });
      setRun(r ?? null);
    } catch (e) {
      // Back to the lists, not a permanent "reading…": `get_run` refuses an id
      // the ledger does not hold, which is what an open report of a run whose
      // automation was just deleted looks like. The banner says why.
      onError(e);
      setRun(null);
      setOpenId(null);
    }
  }, [repo, onError]);

  useEffect(() => {
    if (!openId) { setRun(null); return; }
    void reloadRun(openId);
  }, [openId, reloadRun, runs]);

  const nameOfAutomation = useCallback(
    (slug: string) => autos?.find((a) => a.slug === slug)?.name ?? slug,
    [autos],
  );

  const runNow = useCallback(async (slug: string) => {
    setStarting(slug);
    setNote(null);
    try {
      const r = await invoke<{ id: string; already_running: boolean }>("run_automation", { repo, slug });
      if (r?.already_running) setNote(`${nameOfAutomation(slug)} is already running`);
      await load();
      // Straight into the report: the run writes `running` before anything
      // slow, so there IS an entry to open, and the poll turns it into the
      // findings without a second click.
      if (r?.id) setOpenId(r.id);
    } catch (e) {
      onError(e);
    } finally {
      setStarting(null);
    }
  }, [repo, load, onError, nameOfAutomation]);

  const save = useCallback(async (
    patch: { name: string; brief: string; when: When; scope: string },
    andRun: boolean,
  ) => {
    if (!seed) return;
    setDialogBusy(true);
    setDialogErr(null);
    try {
      const saved = await invoke<AutomationRow>("upsert_automation", {
        repo,
        slug: seed.slug,
        patch: { ...patch, tier: "report" },
      });
      setSeed(null);
      await load();
      if (andRun && saved?.slug) await runNow(saved.slug);
    } catch (e) {
      // Core's own string, verbatim: it names the cause and the fix (a slug
      // that already exists, an empty brief, a bad time), and paraphrasing it
      // here would be a second answer to "why was this refused".
      setDialogErr(String(e));
    } finally {
      setDialogBusy(false);
    }
  }, [repo, seed, load, runNow]);

  const remove = useCallback(async (slug: string) => {
    setCtx(null);
    setArmed(null);
    try {
      await invoke("delete_automation", { repo, slug });
      // Its runs go with it (core deletes them), so an open report of this
      // automation has nothing behind it any more.
      setOpenId((cur) => (cur && runs.find((r) => r.id === cur)?.automation === slug ? null : cur));
      await load();
    } catch (e) {
      onError(e);
    }
  }, [repo, load, onError, runs]);

  const applyProposal = useCallback(async (findingIx: number, proposalIx: number, p: Proposal) => {
    if (!run) return;
    const key = `${findingIx}:${proposalIx}`;
    setApplying(key);
    const slug = String(p.args.slug ?? "");
    const place = places.find((pl) => pl.slug === slug);
    // Optimistic for the three DECLARED tools, exactly as the Lifecycle menu
    // and the drag do. `close_session` gets nothing: it is a tmux act, not a
    // declared field, and there is nothing on the row to guess at.
    //
    // ⚠ `set_lifecycle` must arrive with the PREDICTED badge, never the label:
    // `lifecycle_effective` is reconciled server-side from the label AND live
    // tmux, so patching the label alone shows a row disagreeing with its own
    // badge until the refresh lands (App.tsx `patchDeclared`). `pinned: false`
    // steps `predictTier` past its pin short-circuit, the same way `applyTier`
    // does — pin is ranked separately and is not what this writes.
    if (place) {
      if (p.tool === "set_lifecycle") {
        const label = String(p.args.lifecycle ?? "");
        onPatchDeclared(slug, { lifecycle: label || undefined },
          predictTier(place, { pinned: false, lifecycle: label || null }, Math.floor(Date.now() / 1000)));
      } else if (p.tool === "set_note") {
        const n = String(p.args.note ?? "");
        onPatchDeclared(slug, { note: n || undefined });
      } else if (p.tool === "set_pin") {
        onPatchDeclared(slug, { pinned: p.args.pinned === true });
      }
    }
    try {
      await invoke<RunAction>("apply_proposal", {
        repo, runId: run.id, finding: findingIx, proposal: proposalIx,
      });
    } catch (e) {
      onError(e);
    } finally {
      setApplying(null);
      // Both re-reads either way: a refused call has left an optimistic value
      // on screen, and the run's `actions` is what flips the button to
      // `Applied ✓` (or shows the reason it did not).
      await reloadRun(run.id);
      await onRefresh();
    }
  }, [run, repo, places, onPatchDeclared, onRefresh, onError, reloadRun]);

  const copyRun = useCallback(() => {
    if (!run) return;
    if (!navigator.clipboard) { onError("clipboard unavailable"); return; }
    navigator.clipboard.writeText(runAsMarkdown(run, nameOfAutomation(run.automation))).catch(onError);
  }, [run, nameOfAutomation, onError]);

  // ── render ────────────────────────────────────────────────────────────────

  const dialog = seed && (
    <AutomationDialog
      seed={seed}
      projectName={projectName}
      profileName={profileName}
      busy={dialogBusy}
      error={dialogErr}
      onSave={save}
      onClose={() => { setSeed(null); setDialogErr(null); }}
    />
  );

  // `openId` set with no `run` yet is the tick between opening a row and
  // `get_run` answering. Falling through to the lists for that tick would
  // flash them — and a run started from a row opens its report immediately,
  // so that tick happens on every Run now.
  if (openId && !run) {
    return (
      <div className="autopane" data-testid="automations-pane">
        <div className="auto-run-h">
          <button type="button" className="ctrl sm icon-only" data-testid="auto-back"
            aria-label="Back to the automations list" title="Back" onClick={() => setOpenId(null)}>
            <Icons.ChevronLeft size={12} />
          </button>
          <span className="auto-run-name">{nameOfAutomation(runs.find((r) => r.id === openId)?.automation ?? "")}</span>
        </div>
        <div className="auto-note">reading…</div>
        {dialog}
      </div>
    );
  }

  if (openId && run) {
    return (
      <div className="autopane">
        <RunReport
          run={run}
          name={nameOfAutomation(run.automation)}
          places={places}
          mdZoom={mdZoom}
          busy={applying}
          onBack={() => setOpenId(null)}
          onAgain={() => void runNow(run.automation)}
          onSelectPlace={onSelectPlace}
          onApply={applyProposal}
          onCopy={copyRun}
        />
        {dialog}
      </div>
    );
  }

  const shown = allRuns ? runs : runs.slice(0, FIRST_RUNS);
  // Grouped by CALENDAR day, in the order `list_runs` already sorted them
  // (newest first, id as the tiebreak) — the grouping must not re-sort, or two
  // runs stamped in the same second swap places between renders.
  const groups: { day: string; rows: RunSummary[] }[] = [];
  for (const r of shown) {
    const d = dayLabel(r.started_epoch);
    if (groups.length === 0 || groups[groups.length - 1].day !== d) groups.push({ day: d, rows: [] });
    groups[groups.length - 1].rows.push(r);
  }

  return (
    <div className="autopane" data-testid="automations-pane">
      <div className="auto-head">
        {/* The dock header above already says AUTOMATIONS (it reads the title
            out of `DOCK_RAIL`), so this row carries the fact that header
            cannot: WHICH project's automations these are. §7.1's rule is that
            a surface reached from many places must name its owner — not that
            the word appears twice. */}
        <span className="chip auto-chip" title={repo}>
          <Icons.Folder size={12} />
          {projectName}
        </span>
        <span className="dock-spacer" />
        <button
          type="button"
          className="ctrl sm"
          data-testid="auto-new"
          data-track="dock.automations.new"
          title="Write a new automation for this project"
          onClick={() => { setDialogErr(null); setSeed({ slug: null, name: "", brief: "", when: { kind: "manual" }, scope: "all" }); }}
        >
          + New
        </button>
      </div>

      {note && <div className="auto-note auto-said" onClick={() => setNote(null)} title="dismiss">{note}</div>}

      <div className="scroll auto-lists">
        {autos === null ? (
          <div className="auto-note">reading…</div>
        ) : autos.length === 0 ? (
          <EmptyState
            onStarter={(i) => { setDialogErr(null); setSeed({ slug: null, ...STARTERS[i], when: { kind: "manual" }, scope: "all" }); }}
            onBlank={() => { setDialogErr(null); setSeed({ slug: null, name: "", brief: "", when: { kind: "manual" }, scope: "all" }); }}
          />
        ) : (
          <>
            <div className="auto-sec">AUTOMATIONS</div>
            <div className="auto-rows">
              {autos.map((a) => (
                <AutomationItem
                  key={a.slug}
                  a={a}
                  running={starting === a.slug || runs.some((r) => r.automation === a.slug && r.status === "running")}
                  onOpen={() => { setDialogErr(null); setSeed({ slug: a.slug, name: a.name, brief: a.brief, when: a.when, scope: a.scope }); }}
                  onRun={() => void runNow(a.slug)}
                  onMenu={(e) => { e.preventDefault(); e.stopPropagation(); setArmed(null); setCtx({ x: e.clientX, y: e.clientY, slug: a.slug }); }}
                />
              ))}
            </div>

            <div className="auto-sec">RUNS</div>
            {runs.length === 0 ? (
              <div className="auto-note">No runs yet — press ▸ on one above.</div>
            ) : (
              <>
                {groups.map((g) => (
                  <div key={g.day} className="auto-daygroup">
                    <div className="auto-day">{g.day}</div>
                    {g.rows.map((r) => (
                      <RunItem key={r.id} r={r} name={nameOfAutomation(r.automation)} onOpen={() => setOpenId(r.id)} />
                    ))}
                  </div>
                ))}
                {!allRuns && runs.length > FIRST_RUNS && (
                  <button type="button" className="auto-more" data-testid="auto-all-runs"
                    data-track="dock.automations.allruns" onClick={() => setAllRuns(true)}>
                    All runs… ({runs.length})
                  </button>
                )}
              </>
            )}
          </>
        )}
      </div>

      <div className="auto-foot">
        Runs as this project's AI profile {profileName ? <b>{profileName}</b> : <i>(the default)</i>} · never removes a worktree
      </div>

      {ctx && (
        <CtxMenu x={ctx.x} y={ctx.y} onClose={() => { setCtx(null); setArmed(null); }}>
          <button className="pop-item" onClick={() => { const s = ctx.slug; setCtx(null); void runNow(s); }}>Run now</button>
          <button className="pop-item" onClick={() => {
            const a = autos?.find((x) => x.slug === ctx.slug);
            setCtx(null);
            if (a) { setDialogErr(null); setSeed({ slug: a.slug, name: a.name, brief: a.brief, when: a.when, scope: a.scope }); }
          }}>Edit…</button>
          <div className="ctx-sep" />
          {/* Two-click arm, the shape every destructive row in this app uses
              (`confirmRm` in App.tsx). Deleting an automation deletes its RUNS
              too — core does that deliberately, so the armed label says so. */}
          {armed === ctx.slug ? (
            <button className="pop-item danger armed" onClick={() => void remove(ctx.slug)}>
              Delete {nameOfAutomation(ctx.slug)} and its runs?
            </button>
          ) : (
            <button className="pop-item danger" onClick={() => setArmed(ctx.slug)}>Delete…</button>
          )}
        </CtxMenu>
      )}
      {dialog}
    </div>
  );
}
