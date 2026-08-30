import { useCallback, useEffect, useRef, useState } from "react";
import * as Icons from "./icons";
import { invoke } from "@tauri-apps/api/core";
import { Markdown } from "./markdown";

// One place's health check, in two hosts. `StatusBody` is everything the check
// SAYS; `StatusSheet` is the right-side slide-over that frames it for a place
// whose terminal owns the main window. App.tsx mounts the same body inline in
// the empty area of a place with no live session, where there is nothing for a
// sheet to slide over and the check is the only thing worth showing.
//
// One rendering, two hosts, on purpose: the verdict, the facts table and the
// act-on-it buttons are a single answer to "what is this worktree", and two
// copies of it would drift the moment one host grew a row the other did not.
// The only difference either host is allowed is `hideEnter` — the inline host
// already has Enter as its hero, so the body drops its own.
//
// The sheet itself is ProjectSheet's twin, and built the same way for the same
// reasons (see its header comment): a sheet is one new App state plus two menu
// items, and it reuses .scrim / .settings-sheet / .setting / .ver-rows /
// .ver-actions / .update-log wholesale.
//
// The verdict is NOT computed here. `place_health` runs the real `cmd_status`
// in-process and hands back `worktrees_core::health::Report` verbatim, so this
// file renders a judgement it never makes — the CLI and the app cannot drift.
//
// The lifecycle and remove ACTIONS are props, not invokes. App already owns
// those flows (`set_lifecycle` and its refresh semantics; RemoveDialog and the
// two checkboxes that say what a remove destroys), and a second implementation
// of either would be a second answer to "what did that button just do".

/** `health::HealthFacts` (crates/worktrees-core/src/health.rs) over the wire. */
export type HealthFacts = {
  slug: string;
  branch: string | null;
  base: string;
  created_epoch: number | null;
  last_commit_epoch: number | null;
  last_commit_subject: string | null;
  last_opened_epoch: number | null;
  last_worked_epoch: number | null;
  claude_last_epoch: number | null;
  dirty_files: number;
  ahead: number;
  behind: number;
  upstream: string | null;
  tmux_up: boolean;
  lifecycle_effective: string;
  note: string | null;
  title: string | null;
  not_on_base: string[];
  not_on_base_total: number;
  true_unpushed: number | null;
  maybe_merged: number;
};
/** `health::Report`. */
export type HealthBody = {
  schema_version: number;
  verdict: string;
  reasons: string[];
  facts: HealthFacts;
};
/** The Tauri wrapper (`HealthReport`, lib.rs) — `report: null` + `error` when
 *  `cmd_status` exited on a guard before it emitted any JSON. */
export type HealthReport = { code: number; report: HealthBody | null; error: string | null };

/** What `ai_status_report` returns, and what it caches under the place's
 *  `status_report` declared key (store.rs `extra` round-trips it verbatim).
 *
 *  Exported so App's `Declared` can name THIS type rather than re-describe it:
 *  the shape crosses three boundaries (Rust → declared JSON → this sheet) and a
 *  second declaration is a second thing to keep in step. */
export type StatusReport = { text: string; epoch: number; verdict?: string };

/** What each verdict is CALLED, and the one sentence that says what it means.
 *  `at-risk` is the only one whose label differs from its wire value; keeping
 *  the wire value machine-shaped and the label human-shaped is deliberate. */
const VERDICT: Record<string, { label: string; blurb: string }> = {
  active: { label: "active", blurb: "touched recently — this is live work." },
  parked: {
    label: "parked",
    blurb: "clean, with a session still up. Nothing here exists only here.",
  },
  "at-risk": {
    label: "work at risk",
    blurb: "untouched for a while, and holding work that lives nowhere else.",
  },
  cold: {
    label: "cold",
    blurb: "no session, nothing uncommitted, nothing ahead of the base — safe to let go.",
  },
};

/** Compact age, mirroring `ago` in App.tsx. Duplicated rather than imported
 *  because App imports THIS file, and a cycle through a module both sides
 *  evaluate at load is a Fast Refresh hazard for one four-line function. */
function ago(epoch?: number | null): string {
  if (!epoch) return "";
  const s = Math.floor(Date.now() / 1000) - epoch;
  if (s < 60) return "now";
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}

/** "2026-08-02 · 27d" — the sheet has room for the date the nav row does not,
 *  and a bare age makes a reader do date arithmetic to place the work. */
function when(epoch?: number | null): string {
  if (!epoch || epoch <= 0) return "—";
  return `${new Date(epoch * 1000).toLocaleDateString()} · ${ago(epoch)}`;
}

/** "just now" / "3d ago" — `ago`'s compact form reads as a label beside a row
 *  key, but as a sentence ("as of now") it says the wrong thing. */
function asOf(epoch: number): string {
  const a = ago(epoch);
  return a === "now" ? "just now" : `${a} ago`;
}

function Row({ k, v, title }: { k: string; v: React.ReactNode; title?: string }) {
  return (
    <div className="ver-row hs-row" title={title}>
      <span className="hs-k">{k}</span>
      <span className="hs-v">{v}</span>
    </div>
  );
}

/** Last good `place_health` report per place, so a REVISIT paints instantly and
 *  refetches behind itself (stale-while-revalidate). It matters because the
 *  inline host mounts and unmounts with the selection: without a seed, every
 *  switch back to a session-less place flashed "Checking…" for the length of a
 *  git fan-out. The `report: null` error case is not cached, and DELETES any
 *  entry it finds — an answer that says "the check did not run" is not worth
 *  showing again later as if it were the last known state, and a seed it has
 *  just disproved would be shown exactly that way on every later mount.
 *  Module scope, so it outlives both hosts. */
const healthCache = new Map<string, HealthBody>();
const cacheKey = (repo: string, slug: string) => repo + "|" + slug;

// Module scope with props (CLAUDE.md): a component defined inside App() gets a
// fresh identity every render, remounting its DOM and dropping input focus.
// Both hosts key this by repo|slug, so a switch of place is a fresh mount.
export function StatusBody({
  repo,
  slug,
  hideEnter,
  onEnter,
  onLifecycle,
  onRemove,
  onCopy,
  declared,
  onReport,
}: {
  repo: string;
  slug: string;
  /** The inline host's hero button is already Enter; a second one in the
   *  act-on-it row would be the same gesture twice on one screen. */
  hideEnter?: boolean;
  onEnter: () => void;
  /** App runs this as `set_lifecycle` — deliberately NOT patchDeclared, because
   *  `lifecycle_effective` is reconciled server-side (App.tsx). */
  onLifecycle: (label: string) => void;
  /** App opens its RemoveDialog. This body holds no destructive arm of its own:
   *  the dialog is where a remove is confirmed, and it says what it destroys. */
  onRemove: () => void;
  onCopy: (text: string) => void;
  /** THIS tick's declared state for the place — where a previous "Ask Claude"
   *  was cached. Handed down rather than read here so the body keeps having
   *  exactly one source of truth for the place, the one App refreshes. */
  declared: { status_report?: StatusReport | null } | null;
  /** A report just came back. App patches it into the workspace in hand and
   *  THEN refreshes — both halves in one callback so the order is App's to
   *  guarantee: a `list_workspace` sweep already in flight when the backend
   *  wrote the store carries pre-write declared state, and letting it land
   *  after the resolve would blank a report the user is reading. */
  onReport: (r: StatusReport) => void;
}) {
  // Seeded synchronously from the cache: the first paint of a revisit is the
  // last verdict, not a spinner. `useState`'s initialiser runs once per mount,
  // which is exactly the moment the seed is wanted.
  const [rep, setRep] = useState<HealthBody | null>(() => healthCache.get(cacheKey(repo, slug)) ?? null);
  // TRUE, because the mount effect below refetches unconditionally: this body is
  // checking from its first committed frame. Starting at `false` made an
  // uncached mount paint "No report — the check did not run." for one frame
  // (the `rep === null && !checking` branch) before the effect flipped it —
  // measured at 43ms, replaced at 58ms. Behind a menu item that was a blink;
  // the inline host cold-mounts on every first selection of a session-less
  // place, front and centre in the main window.
  const [checking, setChecking] = useState(true);
  const [err, setErr] = useState<string | null>(null);
  const [asking, setAsking] = useState(false);
  // The read this body just fetched. Belt AND braces with the `declared` prop:
  // the optimistic patch normally puts it there within a render, but the place
  // can vanish from the snapshot under a host that outlives it, and an answer
  // the user waited 90s for must not depend on a list still listing.
  const [fresh, setFresh] = useState<StatusReport | null>(null);

  // Errors are NEVER swallowed: they surface in this body's own error area AND
  // land in app.log (same shape as ProjectSheet — no fail() out here).
  const note = useCallback((m: string) => {
    setErr(m);
    invoke("log_event", { level: "error", msg: m }).catch(() => {});
  }, []);

  // TWO `place_health` calls for one place can be in flight: the mount fetch,
  // and the unconditional re-check the Abandon/Archive buttons fire (they are
  // not disabled while one is running). The mock resolves in a microtask so
  // they cannot cross there, but the real one is a git fan-out and they can —
  // and the OLDER answer landing last would both paint and CACHE pre-lifecycle
  // facts. A ticket per call, so only the newest may write: App's `refreshSeq`
  // rule, at this scale.
  const seq = useRef(0);

  const refresh = useCallback(async () => {
    const my = ++seq.current;
    setChecking(true);
    setErr(null);
    try {
      // The mock harness returns null for an unmocked command, so this has to
      // tolerate null rather than assume a shape.
      const r = await invoke<HealthReport | null>("place_health", { repo, slug });
      if (my !== seq.current) return; // a newer check is already in flight
      setRep(r?.report ?? null);
      // The seed must be able to say "I no longer know". `report: null` is the
      // backend affirming the check did NOT run (guard exit — the worktree
      // directory was deleted under us, the repo moved), which is proof any
      // cached report is wrong: delete it, or every later mount of either host
      // re-paints the disproved verdict and blanks it again a fan-out later,
      // forever, on exactly the places whose facts are most wrong.
      //
      // The `catch` below deliberately does NOT delete, and the asymmetry is
      // the point: a thrown invoke is a transport failure that says nothing
      // about the report's truth, so the last known state stays on screen (and
      // seeded) with the error beside it. One shape disproves the cache; the
      // other only fails to confirm it.
      if (r?.report) healthCache.set(cacheKey(repo, slug), r.report);
      else healthCache.delete(cacheKey(repo, slug));
      if (r?.error) setErr(r.error);
    } catch (e) {
      if (my !== seq.current) return;
      note(`status ${slug} failed: ${String(e)}`);
    } finally {
      if (my === seq.current) setChecking(false);
    }
  }, [repo, slug, note]);

  // ⚠ NEVER automatic. `refresh` (place_health) is local git and runs on mount
  // — including the inline host's, which mounts on selection; this spawns
  // claude, costs tokens and takes ~30–90s, so it happens only when someone
  // presses the button. There is no effect that calls it, in either host.
  const ask = useCallback(async () => {
    setAsking(true);
    setErr(null);
    try {
      const r = await invoke<StatusReport | null>("ai_status_report", { repo, slug });
      if (!r || !r.text) {
        // The backend Errs on an empty answer rather than caching one, so this
        // is the mock's unmocked-command null — still not a report.
        note(`ask claude ${slug}: no report came back`);
        return;
      }
      setFresh(r);
      onReport(r);
    } catch (e) {
      note(`ask claude ${slug} failed: ${String(e)}`);
    } finally {
      setAsking(false);
    }
  }, [repo, slug, note, onReport]);

  // ⚠ The deps are the PLACE, and nothing else. `refresh` is left out on
  // purpose even though it is called here: its own deps are exactly
  // [repo, slug, note] and `note` is stable, so listing it would say the same
  // thing at one remove. What must never get in is anything derived from the
  // workspace object — a `list_workspace` sweep replaces it every tick, and a
  // dep on it would re-run this git fan-out several times a second.
  useEffect(() => {
    refresh();
  }, [repo, slug]); // eslint-disable-line react-hooks/exhaustive-deps

  const f = rep?.facts ?? null;
  const verdict = rep?.verdict ?? "";
  const v = VERDICT[verdict];
  const isMain = f ? f.slug === "(main)" : false;
  // Whichever claude signal is newer — "when did anything think in here", which
  // is not the same question as "when was this opened".
  const claudeAt = Math.max(f?.claude_last_epoch ?? 0, f?.last_worked_epoch ?? 0);
  const shown = f?.not_on_base.length ?? 0;
  const more = (f?.not_on_base_total ?? 0) - shown;
  // This run first, then the cache. They are the same value once the patch
  // lands; in the gap before it does, this run is the newer of the two.
  const read = fresh ?? declared?.status_report ?? null;

  // A fragment, not a wrapper: the gap between these sections is the HOST's
  // (the sheet's `.settings-body`, the inline panel's `.term-status`), and a
  // wrapper here would be a second, invisible layout box in both.
  return (
    <>
      <section className="setting">
        {rep === null ? (
          <div className="hint" data-testid="status-pending">
            {checking ? "Checking…" : "No report — the check did not run."}
          </div>
        ) : (
          <>
            <div className={"hs-verdict " + verdict} data-testid="status-verdict">
              <span className="hs-label">{v?.label ?? verdict}</span>
              <span className="hs-blurb">{v?.blurb ?? ""}</span>
            </div>
            {/* The receipts. Empty for active and cold by design — the
                verdict is the whole story there, and the facts below carry
                the rest — so the blurb above is what those two read as. */}
            {rep.reasons.length > 0 && (
              <div className="hs-reasons" data-testid="status-reasons">
                {rep.reasons.map((r, i) => (
                  <div className="hs-reason" key={i}>{r}</div>
                ))}
              </div>
            )}
          </>
        )}
        <div className="ver-actions">
          <button className="ctrl sm" onClick={refresh} disabled={checking}>
            {checking ? "Checking…" : "Re-check"}
          </button>
        </div>
        {err && <pre className="update-log">{err}</pre>}
      </section>

      {f && (
        <section className="setting">
          <label>Facts</label>
          <div className="ver-rows" data-testid="status-facts">
            <Row k="branch" v={f.branch ?? "(detached)"} />
            <Row k="created" v={when(f.created_epoch)} />
            <Row
              k="last commit"
              v={
                f.last_commit_epoch
                  ? `${when(f.last_commit_epoch)}${f.last_commit_subject ? ` · ${f.last_commit_subject}` : ""}`
                  : "—"
              }
            />
            <Row k="last claude work" v={claudeAt ? when(claudeAt) : "—"} />
            <Row k="last entered" v={when(f.last_opened_epoch)} />
            <Row k="uncommitted" v={f.dirty_files ? `${f.dirty_files} file${f.dirty_files === 1 ? "" : "s"}` : "none"} />
            <Row k="ahead" v={`↑${f.ahead} not on ${f.base}`} />
            {/* Labelled "base moved", never "behind": the number describes
                the base, not a failing of this worktree — and the verdict
                above never counted it. */}
            <Row
              k="base moved"
              v={<>{f.behind} commit{f.behind === 1 ? "" : "s"} <i className="hs-info">(informational)</i></>}
              title="the base branch advanced — this worktree did nothing wrong"
            />
            <Row
              k="upstream"
              v={
                f.upstream
                  ? f.true_unpushed && f.true_unpushed > 0
                    ? `${f.upstream} · ${f.true_unpushed} not pushed`
                    : f.upstream
                  : "none — nothing pushed"
              }
            />
            <Row k="session" v={f.tmux_up ? "up" : "down"} />
            <Row k="lifecycle" v={f.lifecycle_effective} />
            {f.note ? <Row k="note" v={f.note} /> : null}
          </div>
        </section>
      )}

      {f && f.not_on_base.length > 0 && (
        <section className="setting">
          <label>Commits not on {f.base}</label>
          <pre className="update-log" data-testid="status-commits">
            {f.not_on_base.join("\n")}
            {more > 0 ? `\n…and ${more} more` : ""}
          </pre>
          {f.maybe_merged > 0 && (
            <div className="hint">
              {f.maybe_merged} commit{f.maybe_merged === 1 ? "" : "s"} match patches already on {f.base}
              {" "}(patch-id — unreliable across squash merges)
            </div>
          )}
        </section>
      )}

      {/* The facts above say what IS here. This says what it was FOR — the
          one question no git fact answers, and the reason a stale worktree
          is hard to let go of. On demand only, and cached until re-run:
          it costs tokens and a minute of waiting. */}
      <section className="setting">
        <label>Claude's read</label>
        {read ? (
          <>
            {/* The answer is markdown-shaped by construction — `status_prompt`
                asks three numbered questions and a recommendation — so it is
                rendered as markdown, not printed. `Markdown` is the lexer-only
                renderer (no dangerouslySetInnerHTML), which is what makes it
                safe to point at model output; no `onLink` and no `renderImage`
                is deliberate, and leaves links inert and relative images as
                their alt text. Plain-text reads cached before this change still
                render — as paragraphs, which is what they always were. */}
            <div className="update-log hs-read hs-read-md" data-testid="status-ai-text">
              <Markdown src={read.text} />
            </div>
            <div className="hint" data-testid="status-ai-age">
              as of {asOf(read.epoch)}
              {read.verdict ? ` · on a "${read.verdict}" verdict` : ""}
            </div>
          </>
        ) : (
          <div className="hint" data-testid="status-ai-hint">
            Hand these facts to your repo's AI profile and get back what this worktree was
            for, where it ended up, and whether to resume it, push it and let it go, or
            drop it. Runs your repo's AI profile headless — ~30–90s.
          </div>
        )}
        <div className="ver-actions">
          <button
            className="ctrl sm"
            data-testid="status-ask"
            onClick={ask}
            disabled={asking}
            title={read ? "ask again — this replaces the cached read" : "runs claude once, headless"}
          >
            {asking ? "Asking Claude…" : read ? "Re-run" : "Ask Claude"}
          </button>
        </div>
      </section>

      <section className="setting">
        <label>Act on it</label>
        <div className="ver-actions hs-actions">
          {!hideEnter && <button className="ctrl sm" onClick={onEnter}>Enter</button>}
          {!isMain && (
            <>
              <button className="ctrl sm" onClick={() => { onLifecycle("abandoned"); refresh(); }}>
                Abandon
              </button>
              <button className="ctrl sm" onClick={() => { onLifecycle("archived"); refresh(); }}>
                Archive
              </button>
              <button className="ctrl sm danger" data-testid="status-remove" onClick={onRemove}>
                Remove worktree…
              </button>
            </>
          )}
          {f?.branch && (
            <button className="ctrl sm" onClick={() => onCopy(f.branch!)}>Copy branch</button>
          )}
        </div>
        <div className="hint">
          Abandon and Archive are labels — the worktree and its branch stay on disk, and the
          place drops out of the live list. Remove is the one that deletes.
        </div>
      </section>
    </>
  );
}

// The sheet: a scrim, a slide-over and an Escape key around `StatusBody`. It
// keeps the `open` gate rather than pushing it to the caller so the body MOUNTS
// on open — which is what runs the check — and unmounts on close.
export function StatusSheet({
  open,
  repo,
  slug,
  placeName,
  onClose,
  onEnter,
  onLifecycle,
  onRemove,
  onCopy,
  declared,
  onReport,
}: {
  open: boolean;
  repo: string;
  slug: string;
  /** What the place is CALLED (title, else slug) — the sheet's header. */
  placeName: string;
  onClose: () => void;
  onEnter: () => void;
  onLifecycle: (label: string) => void;
  onRemove: () => void;
  onCopy: (text: string) => void;
  declared: { status_report?: StatusReport | null } | null;
  onReport: (r: StatusReport) => void;
}) {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div className="scrim" onClick={onClose}>
      <aside className="settings-sheet status-sheet" onClick={(e) => e.stopPropagation()}>
        <header className="settings-h">
          <b>Status · {placeName}</b>
          <button className="icon-btn" title="close (Esc)" onClick={onClose}><Icons.X /></button>
        </header>

        <div className="settings-body">
          {/* No `hideEnter`: this sheet has no hero of its own, so Enter belongs
              in the act-on-it row where every other action is. */}
          <StatusBody repo={repo} slug={slug} onEnter={onEnter} onLifecycle={onLifecycle}
            onRemove={onRemove} onCopy={onCopy} declared={declared} onReport={onReport} />
        </div>
      </aside>
    </div>
  );
}
