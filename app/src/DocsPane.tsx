// The Docs tab — a per-place documentation index, and the staleness header
// that is the reason it exists.
//
// This is NOT a reader. `FilesPane`/`markdown.tsx` have rendered markdown since
// v0.8.0 — relative links, anchors, reading mode, per-place zoom, ⌘F — and a
// row here hands its path straight to that viewer. What was missing is upstream
// of reading: an index shaped like documentation rather than like a directory,
// and a signal saying WHICH place's documentation you are looking at.
//
// The hazard, concretely. valleos has eleven active places; seven carry the
// pre-restructure `docs/` tree and four carry the new one, and both states are
// correct for their branch. A reader that does not say "this place, 53 commits
// behind origin/main" lets an ADR that was moved and renumbered a fortnight ago
// read as current — and the failure is silent exactly when it costs most. So
// the header is not a badge and not collapsible.
//
// Every number in it is already in the `Place` the app holds; only `base` (the
// ref `behind` is measured against) comes from the backend, and that is because
// it is NOT `upstream` — see `list_docs` in lib.rs for why printing `upstream`
// there would name the wrong ref on every pushed branch.
//
// Ordering, grouping and titles all come from `worktrees_core::docs` and are
// rendered verbatim. Nothing here re-derives a core rule, so there is nothing
// for a `docs-check.mjs` to guard — which is the point of putting the walk in
// core rather than splitting it.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import * as Icons from "./icons";
import { track } from "./usage";

/** `worktrees_core::docs::DocEntry`. */
export type DocEntry = { path: string; rel: string; title: string; group: string };
/** `DocsIndex` (lib.rs). `base` is the ref `behind` counts against — the
 *  project's base ref, never `Place::upstream`. `""` when the project could not
 *  be discovered, in which case the header says "N behind" and names nothing,
 *  which is honest rather than wrong. */
export type DocsIndex = {
  base: string;
  /** This place's `.worktrees.toml` did not parse. The index below it is the
   *  CONVENTION's, not the config's — said out loud, because an index quietly
   *  showing something other than what the repo declared is the same class of
   *  silent wrongness as an unlabelled stale tree. */
  config_error?: string | null;
  entries: DocEntry[];
  truncated: boolean;
};

/** Just enough of `Place` for the header. Declared structurally so this file
 *  does not import App's type and App does not have to export it. */
export type DocsPlace = {
  branch: string | null;
  behind: number | null;
  dirty_files?: number | null;
  dirty: boolean | null;
  last_commit_subject?: string | null;
  last_commit_epoch?: number | null;
};

export type DocsPaneProps = {
  /** The place directory — the walk's root. */
  root: string;
  /** The project root, for the base ref. */
  repo: string;
  /** The place's slug. Names the viewer's group and is the first fact in the
   *  header it injects into every page — a browser tab has no nav beside it. */
  slug: string;
  place: DocsPlace;
  /** Bumped by `places:changed` (tmux change, or the 30 s poll). */
  reloadToken: number;
  /** Open a document in the Files tab's renderer. */
  onOpen: (path: string) => void;
  onError: (e: unknown) => void;
  /** ⌘F — the Docs tab's find surface is its name filter, so a press focuses
   *  and selects the box rather than mounting a second bar. The token bumps on
   *  every press so a second one re-selects, which is what every find bar does. */
  findOpen: boolean;
  findToken: number;
  onFindClose: () => void;
};

/** Compact age, matching the nav's. */
function ago(epoch?: number | null): string {
  if (!epoch) return "";
  const s = Math.floor(Date.now() / 1000) - epoch;
  if (s < 60) return "now";
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}

/** The staleness header, as words.
 *
 *  THREE LINES, not one, and the split is the design rather than a layout
 *  accident. The dock's floor is 240px; branch + "25 behind origin/main" +
 *  a dirty count on one row ellipsises at any realistic width, and the run it
 *  cut first was `origin/main` — turning the one fact that exists nowhere else
 *  in the app into "25 behind origin/m…". The branch and the dirty count are
 *  both already on screen (topbar, nav row); `behind` is not, so it gets a line
 *  of its own and truncates last.
 *
 *  `behind: 0` is said out loud too. "Up to date" is the state a reader most
 *  wants confirmed, and a line that simply is not there reads identically to
 *  one that could not be computed. */
function staleness(p: DocsPlace, base: string): { branch: string; dirty: string; behind: string | null } {
  const n = p.dirty_files ?? 0;
  const behind =
    typeof p.behind !== "number"
      ? null
      : p.behind === 0
        ? base ? `up to date with ${base}` : "up to date"
        : base ? `${p.behind} behind ${base}` : `${p.behind} behind`;
  return {
    // A detached HEAD has no branch, and a blank line where the identity goes
    // is worse than naming the state.
    branch: p.branch || "detached",
    dirty: p.dirty ? (n === 1 ? "1 dirty" : `${n} dirty`) : "clean",
    behind,
  };
}

/** Does `q` match this entry? Name filter, not full text — a substring over the
 *  title and the path, which is what "find the page called X" needs and all the
 *  index can honestly offer. Case-insensitive, and every space-separated term
 *  must hit, so `arch over` finds `docs/architecture/overview.md`. */
function matches(e: DocEntry, terms: string[]): boolean {
  if (!terms.length) return true;
  const hay = `${e.title}\n${e.rel}`.toLowerCase();
  return terms.every((t) => hay.includes(t));
}

/** Entries in backend order, cut into the groups the backend assigned. Order of
 *  FIRST APPEARANCE, never re-sorted: the walk already decided, and a second
 *  opinion here is the mirror this component exists without. */
function group(entries: DocEntry[]): Array<{ name: string; rows: DocEntry[] }> {
  const out: Array<{ name: string; rows: DocEntry[] }> = [];
  for (const e of entries) {
    const last = out[out.length - 1];
    if (last && last.name === e.group) last.rows.push(e);
    else out.push({ name: e.group, rows: [e] });
  }
  return out;
}

export function DocsPane({ root, repo, slug, place, reloadToken, onOpen, onError, findOpen, findToken, onFindClose }: DocsPaneProps) {
  const [idx, setIdx] = useState<DocsIndex | null>(null);
  const [loading, setLoading] = useState(true);
  const [q, setQ] = useState("");
  const [sel, setSel] = useState<string | null>(null);
  const filterRef = useRef<HTMLInputElement>(null);
  // Has the filter been USED in this place, yet? The rail button, the rows,
  // refresh and reveal all carry a `data-track` and are counted by the global
  // click listener; an `<input>` is not a control `keyForTarget` reads, so
  // without this the one Docs affordance with no evidence behind it would be
  // the one most likely to be cut for lack of evidence.
  //
  // ONCE per mount, on the empty→non-empty edge, not per keystroke: a
  // per-keystroke key outranks every real control within a day and drowns the
  // comparison it exists to serve (the same reasoning that drops the pty's
  // ctrl chords in `trackChord`). The pane is keyed by place, so a remount
  // resets it and the count reads as "places where the filter was used".
  //
  // ⚠ The key is a LITERAL and must stay one. The query is a person's typing,
  // and typing may never reach `ui-events.jsonl` — `usage-check.mjs` half 6
  // refuses a non-literal `track()` argument outside `usage.ts` precisely
  // because this file is where the temptation lives.
  const filterUsed = useRef(false);

  // A stale answer must never paint over a fresh one: the walk is a few hundred
  // file reads, so two of them CAN overlap when the place changes mid-flight.
  // The mock resolves in a microtask and cannot express that (CLAUDE.md), which
  // is exactly why the guard is written rather than discovered.
  const seq = useRef(0);
  const load = useCallback(() => {
    const mine = ++seq.current;
    setLoading(true);
    invoke<DocsIndex | null>("list_docs", { repo, root })
      .then((r) => {
        if (seq.current !== mine) return;
        // A typed invoke can still answer `null` — the harness resolves unknown
        // commands that way, and so does an older backend. A blank pane, not a
        // crash.
        setIdx(r ?? { base: "", entries: [], truncated: false });
        setLoading(false);
      })
      .catch((e) => {
        if (seq.current !== mine) return;
        setIdx({ base: "", entries: [], truncated: false });
        setLoading(false);
        onError(e);
      });
  }, [repo, root, onError]);

  useEffect(() => { load(); }, [load, reloadToken]);
  // A half-typed filter must not follow you to another place — the same rule
  // the dock's open file and the header rename already follow.
  useEffect(() => { setQ(""); setSel(null); }, [root]);

  useEffect(() => {
    if (!findOpen) return;
    const el = filterRef.current;
    if (!el) return;
    // `preventScroll`: the pane is inside the dock's clipping box, and a focus
    // that scrolls it pushes the header out of view (CLAUDE.md has the header
    // popover version of this).
    el.focus({ preventScroll: true });
    el.select();
  }, [findOpen, findToken]);

  const terms = useMemo(() => q.toLowerCase().split(/\s+/).filter(Boolean), [q]);
  const shown = useMemo(() => (idx ? idx.entries.filter((e) => matches(e, terms)) : []), [idx, terms]);
  const groups = useMemo(() => group(shown), [shown]);
  const total = idx?.entries.length ?? 0;

  const open = (e: DocEntry) => { setSel(e.path); onOpen(e.path); };

  // ── the browser viewer (phase 3) ───────────────────────────────────────────
  //
  // OPTIMISTIC, like `tmuxOk`'s `useState(true)`: nothing is probed at mount,
  // no banner can flash on launch, and unavailability is discovered at the point
  // of use. The blast radius of the whole viewer is these two buttons — a place
  // with no viewer at all still lists, filters, reads and reveals every
  // document, which is what phases 1 and 2 shipped.
  //
  // `viewerErr` is per pane and per place (the pane is keyed by place in App),
  // and it is set BESIDE `onError` rather than instead of it: the toast is how
  // the user hears about it once, the line under the button is how they see it
  // is still true. Never swallowed, never only-logged.
  const [viewerErr, setViewerErr] = useState<string | null>(null);
  const [viewerBusy, setViewerBusy] = useState(false);
  const browse = useCallback(
    async (path: string | null) => {
      setViewerBusy(true);
      try {
        const url = await invoke<string | null>("open_docs_viewer", {
          repo,
          root,
          slug,
          path,
          // The header's facts, from the same `Place` the header above is
          // rendering — so the page in the browser and the pane in the dock can
          // never disagree about how stale this place is. `base` is the
          // backend's (it is the one fact `Place` does not carry, and §11.4 is
          // what happens when it is guessed).
          place: {
            branch: place.branch,
            behind: place.behind,
            dirty: place.dirty,
            dirty_files: place.dirty_files ?? null,
            last_commit_subject: place.last_commit_subject ?? null,
            last_commit_epoch: place.last_commit_epoch ?? null,
          },
        });
        // A typed invoke can still answer `null` — the harness resolves unknown
        // commands that way, and an older backend has no such command at all.
        // Opening `null` would be a blank tab with no explanation.
        if (!url) throw new Error("the documentation viewer is not available in this build");
        setViewerErr(null);
        await openUrl(url);
      } catch (e) {
        setViewerErr(String(e));
        onError(e);
      } finally {
        setViewerBusy(false);
      }
    },
    [repo, root, slug, place, onError],
  );

  const head = staleness(place, idx?.base ?? "");
  const subject = place.last_commit_subject ?? "";
  const age = ago(place.last_commit_epoch);

  return (
    <div className="docspane">
      {/* THE HEADER. Always, never collapsible — see the file note. */}
      <div className="docs-head">
        <div className="docs-idline">
          <span className="docs-branch" title={head.branch}>{head.branch}</span>
          <span className="docs-dirty">{head.dirty}</span>
        </div>
        {head.behind && <div className="docs-behind" title={head.behind}>{head.behind}</div>}
        {(subject || age) && (
          <div className="docs-last" title={subject}>
            {subject && <span className="docs-subj">{subject}</span>}
            {age && <span className="docs-age">{age}</span>}
          </div>
        )}
      </div>

      {idx?.config_error && (
        <div className="docs-warn" title={idx.config_error}>
          <span className="docs-warn-i" aria-hidden>⚠</span>
          <span className="docs-warn-t">
            <code>.worktrees.toml</code> did not parse — listing by convention. {idx.config_error}
          </span>
        </div>
      )}

      <div className="docs-filter">
        <input
          ref={filterRef}
          className="docs-q"
          type="text"
          placeholder={total ? `filter ${total} document${total === 1 ? "" : "s"}…` : "filter by name…"}
          value={q}
          onChange={(e) => {
            const v = e.target.value;
            if (v && !filterUsed.current) { filterUsed.current = true; track("docs.filter"); }
            setQ(v);
          }}
          onKeyDown={(e) => {
            if (e.key !== "Escape") return;
            // Escape clears, then releases. Two presses, because a filter you
            // cannot see the effect of clearing is a filter you have to retype.
            e.stopPropagation();
            if (q) setQ("");
            else { e.currentTarget.blur(); onFindClose(); }
          }}
          aria-label="Filter documents by name"
        />
        {q && (
          <button className="ctrl sm icon-only" data-track="docs.clear" title="Clear the filter" aria-label="Clear the filter" onClick={() => { setQ(""); filterRef.current?.focus({ preventScroll: true }); }}>
            <Icons.X size={13} />
          </button>
        )}
      </div>

      <div className="docs-list">
        {loading && !idx && <div className="docs-note">reading…</div>}
        {idx && !total && !loading && (
          <div className="docs-note">
            No markdown in this place yet.
            <span className="docs-note-sub">README, CLAUDE, DESIGN, ROADMAP, CHANGELOG, a brief, and anything under <code>docs/</code>.</span>
          </div>
        )}
        {idx && !!total && !shown.length && <div className="docs-note">Nothing matches “{q}”.</div>}
        {/* Keyed on the first row's PATH, not on the group name. Core promises
            each group is one contiguous run (`every_group_is_one_contiguous_run`),
            so the name would do — but it did not, once: the walk used to emit
            the leftover root files after the tree, giving two `""` runs and two
            siblings keyed `(root)`, which React answers by duplicating and
            omitting children. A path is unique by construction, so a future
            reordering can be wrong without also being silently wrong. */}
        {groups.map((g) => (
          <div className="docs-group" key={g.rows[0].path}>
            {g.name && (
              <div className="docs-ghead">
                <span className="docs-gname">{g.name}</span>
                <span className="docs-gcount">{g.rows.length}</span>
              </div>
            )}
            {g.rows.map((e) => (
              <div
                key={e.path}
                className={"docs-row" + (sel === e.path ? " sel" : "")}
                role="button"
                tabIndex={0}
                data-track="docs.row"
                title={e.rel}
                onClick={() => open(e)}
                onKeyDown={(ev) => { if (ev.key === "Enter" || ev.key === " ") { ev.preventDefault(); open(e); } }}
              >
                <span className="docs-title">{e.title}</span>
                <span className="docs-rel">{e.rel}</span>
                <span className="docs-verbs">
                  {/* §7.2: the browser becomes the row's other action, and the
                      in-app read (the row itself) stays the primary one. It is
                      the only way to see a diagram at all — the dock's renderer
                      shows a mermaid fence as a code block, by decision. */}
                  <button
                    className="docs-verb"
                    data-track="docs.rowbrowser"
                    title="Open in the browser"
                    aria-label={`Open ${e.rel} in the browser`}
                    disabled={viewerBusy}
                    onClick={(ev) => { ev.stopPropagation(); void browse(e.path); }}
                  >
                    <Icons.ExternalLink size={13} />
                  </button>
                  <button
                    className="docs-verb"
                    data-track="docs.reveal"
                    title="Reveal in Finder"
                    aria-label={`Reveal ${e.rel} in Finder`}
                    onClick={(ev) => { ev.stopPropagation(); revealItemInDir(e.path).catch(onError); }}
                  >
                    <Icons.Folder size={13} />
                  </button>
                </span>
              </div>
            ))}
          </div>
        ))}
        {idx?.truncated && (
          <div className="docs-note docs-trunc">
            Stopped at {total} documents — this place has more. The Files tab lists everything.
          </div>
        )}
      </div>

      {/* §7.1's last row. Present and enabled from the first frame — there is no
          probe deciding whether to show it, because a probe is a thing that can
          be wrong at launch about a subsystem nothing has needed yet. */}
      <div className="docs-foot">
        <button
          className="ctrl sm docs-browse"
          data-track="docs.browser"
          title="Open this place in the browser — diagrams render there"
          disabled={viewerBusy || !total}
          onClick={() => void browse(null)}
        >
          <Icons.ExternalLink size={13} />
          <span>Open this place in the browser</span>
        </button>
        {viewerErr && (
          <div className="docs-foot-err" title={viewerErr}>{viewerErr}</div>
        )}
      </div>
    </div>
  );
}
