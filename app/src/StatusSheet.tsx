import { useCallback, useEffect, useState } from "react";
import * as Icons from "./icons";
import { invoke } from "@tauri-apps/api/core";

// Right-side slide-over for ONE place's health check — ProjectSheet's twin, and
// built the same way for the same reasons (see its header comment): a sheet is
// one new App state plus two menu items, and it reuses .scrim / .settings-sheet
// / .setting / .ver-rows / .ver-actions / .update-log wholesale.
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

function Row({ k, v, title }: { k: string; v: React.ReactNode; title?: string }) {
  return (
    <div className="ver-row hs-row" title={title}>
      <span className="hs-k">{k}</span>
      <span className="hs-v">{v}</span>
    </div>
  );
}

// Module scope with props (CLAUDE.md): a component defined inside App() gets a
// fresh identity every render, remounting its DOM and dropping input focus.
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
}: {
  open: boolean;
  repo: string;
  slug: string;
  /** What the place is CALLED (title, else slug) — the sheet's header. */
  placeName: string;
  onClose: () => void;
  onEnter: () => void;
  /** App runs this as `set_lifecycle` — deliberately NOT patchDeclared, because
   *  `lifecycle_effective` is reconciled server-side (App.tsx). */
  onLifecycle: (label: string) => void;
  /** App opens its RemoveDialog. This sheet holds no destructive arm of its own:
   *  the dialog is where a remove is confirmed, and it says what it destroys. */
  onRemove: () => void;
  onCopy: (text: string) => void;
}) {
  const [rep, setRep] = useState<HealthBody | null>(null);
  const [checking, setChecking] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  // Errors are NEVER swallowed: they surface in this sheet's own error area AND
  // land in app.log (same shape as ProjectSheet — no fail() out here).
  const note = useCallback((m: string) => {
    setErr(m);
    invoke("log_event", { level: "error", msg: m }).catch(() => {});
  }, []);

  const refresh = useCallback(async () => {
    setChecking(true);
    setErr(null);
    try {
      // The mock harness returns null for an unmocked command, so this has to
      // tolerate null rather than assume a shape.
      const r = await invoke<HealthReport | null>("place_health", { repo, slug });
      setRep(r?.report ?? null);
      if (r?.error) setErr(r.error);
    } catch (e) {
      note(`status ${slug} failed: ${String(e)}`);
    } finally {
      setChecking(false);
    }
  }, [repo, slug, note]);

  useEffect(() => {
    if (!open) return;
    refresh();
  }, [open, refresh]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  const f = rep?.facts ?? null;
  const verdict = rep?.verdict ?? "";
  const v = VERDICT[verdict];
  const isMain = f ? f.slug === "(main)" : false;
  // Whichever claude signal is newer — "when did anything think in here", which
  // is not the same question as "when was this opened".
  const claudeAt = Math.max(f?.claude_last_epoch ?? 0, f?.last_worked_epoch ?? 0);
  const shown = f?.not_on_base.length ?? 0;
  const more = (f?.not_on_base_total ?? 0) - shown;

  return (
    <div className="scrim" onClick={onClose}>
      <aside className="settings-sheet status-sheet" onClick={(e) => e.stopPropagation()}>
        <header className="settings-h">
          <b>Status · {placeName}</b>
          <button className="icon-btn" title="close (Esc)" onClick={onClose}><Icons.X /></button>
        </header>

        <div className="settings-body">
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

          <section className="setting">
            <label>Act on it</label>
            <div className="ver-actions hs-actions">
              <button className="ctrl sm" onClick={onEnter}>Enter</button>
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
        </div>
      </aside>
    </div>
  );
}
