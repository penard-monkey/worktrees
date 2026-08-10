import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import * as Icons from "./icons";
import { ShellPane, TerminalPane } from "./TerminalPane";
import { FilesPane, FileView } from "./FilesPane";
import { SettingsSheet } from "./SettingsSheet";
import {
  driftedSlugs, InitBanner, issueCount, ProjectSheet, reportFailed,
  type DoctorReport, type InitSuggestion,
} from "./ProjectSheet";
import { applySettings, clampDock, clampNav, DEFAULTS, fitLayout, loadSettings, saveSettings, viewportWidth, type Settings, type UpdateInfo } from "./settings";
import logoUrl from "./assets/logo.png";
import "./tokens.css";
import "./App.css";

type Declared = {
  lifecycle?: string;
  pinned?: boolean;
  note?: string;
  last_opened_epoch?: number;
  /// When Claude last FINISHED a task here (see store.rs). Opening a session
  /// never sets it — only work does.
  last_worked_epoch?: number;
} | null;

type Place = {
  slug: string;
  path: string;
  is_main: boolean;
  registered: boolean;
  branch: string | null;
  detached: boolean | null;
  dirty: boolean | null;
  dirty_files?: number | null;
  ahead: number | null;
  behind: number | null;
  tmux_session: { name: string; up: boolean };
  last_commit_epoch?: number | null;
  claude_session_present: boolean;
  /// The AI profile the LIVE session was started with, and whether that profile
  /// has been edited since. Both come from the launch stamp, so a place with no
  /// stamp simply has no badge.
  profile_name?: string | null;
  profile_stale?: boolean;
  declared: Declared;
  lifecycle_effective: string;
};
/** `unborn`: the repo was `git init`ed but has no commits — it lists fine, yet
 *  no worktree can be created off an unborn branch. The nav offers the first
 *  commit rather than letting `new_place` fail on an invalid object name. */
type Snapshot = { repo: string; prefix: string; places: Place[]; unborn?: boolean };
type ProjectView = { root: string; ok: boolean; error: string | null; snapshot: Snapshot | null };
type Workspace = { projects: ProjectView[] };
/** `needs_confirm` (close only): core stopped because killing this session needs
 *  the user's word, and the string is the session that would die. Not a failure
 *  — a question, so it must never reach the error banner. */
type DirProbe = { exists: boolean; is_git: boolean; has_commits: boolean };
type CmdResult = { ok: boolean; code: number; output: string; slug?: string | null; needs_confirm?: string | null; warnings?: string[] };
type Lens = "places" | "recent" | "attention";
/** Last known `doctor` state for one project root. `slugs` decorates rows,
 * `issues` is the project-level count (placeless findings included, so it equals
 * the sheet's Health badge), `error` means the last run did not produce a report
 * at all — in which case the other two are the last measurement, not this one. */
type ProjectHealth = { slugs: Set<string>; issues: number; error: string | null };

// Arm keys for the topbar Close (bare control) and the ctx-menu "Close session"
// (popover). Namespaced so they never collide with the remove arms sharing
// `confirmRm`. The menu's own dismissal clears `closectx|` too, but it is not
// the only thing that may — see isBareArm.
const closeKey = (repo: string, slug: string) => `close|${repo}|${slug}`;
const closeCtxKey = (repo: string, slug: string) => `closectx|${repo}|${slug}`;
/** Arms that TIME OUT rather than waiting to be dismissed. The header ✕ has no
 *  popover to dismiss it at all. Both close arms are here for a second reason:
 *  an armed kill of a WHOLE tmux session must not outlive the moment it was
 *  offered, and "the menu is still open" is not a moment — it has no upper
 *  bound, and the session the label names can be replaced under it (core
 *  refuses the mismatch, but the stale offer is still a lie). */
const isBareArm = (k: string | null) =>
  !!k && (k.startsWith("hdr|") || k.startsWith("close|") || k.startsWith("closectx|"));
/** core's `diag::EXIT_NEEDS_CONFIRM` — "I stopped to ask", never a failure. */
const EXIT_NEEDS_CONFIRM = 4;

// ⌘1..N nav targets, in the nav's displayed top-to-bottom order: Home (clear
// selection → briefing), then the rail's lens entries. Module scope = stable.
const NAV_CHORDS: { home?: boolean; lens?: Lens }[] = [
  { home: true }, { lens: "places" }, { lens: "recent" }, { lens: "attention" },
];

const LIVE_TIERS = ["pinned", "active", "idle"] as const;
const DORMANT_TIERS = ["saved", "closed", "archived", "abandoned"] as const;
const GROUP_LABEL: Record<string, string> = {
  pinned: "Pinned", active: "Active", idle: "Idle",
  saved: "Saved", closed: "Closed", archived: "Archived", abandoned: "Abandoned",
};
const SETTABLE = [
  { label: "Close", value: "closed" },
  { label: "Save", value: "saved" },
  { label: "Archive", value: "archived" },
  { label: "Abandon", value: "abandoned" },
];
const basename = (p: string) => p.replace(/\/+$/, "").split("/").pop() || p;
const bucketOf = (p: Place) => (p.declared?.pinned ? "pinned" : p.lifecycle_effective);
const hasAttention = (p: Place) => !!p.dirty || !!p.ahead || !!p.behind;

// nav icons live in ./icons now — inline SVG, inheriting currentColor per theme
const FolderIcon = Icons.Folder16;
const HomeIcon = Icons.Home;

function ago(epoch?: number): string {
  if (!epoch) return "";
  const s = Math.floor(Date.now() / 1000) - epoch;
  if (s < 60) return "now";
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}

// ── afterglow: "Claude finished a task here", decaying ───────────────────────
// The busy dot vanishing used to be the end of all visibility. These tiers keep
// a place lit after its work lands and dim it out over the working day, so the
// nav answers "what moved recently" at a glance.
//
// DISCRETE, not a continuous fade, for three reasons: absolute opacity is
// unreadable on its own (only the contrast BETWEEN rows carries), a CSS
// animation long enough to cover 12h is frozen at frame 0 by
// `prefers-reduced-motion` (tokens.css) — full brightness forever, the exact
// inverse of the signal — and steps are assertable with getComputedStyle
// instead of racing an in-flight animation.
const DONE_T1_SECS = 15 * 60; // "just finished — go look"
const DONE_T2_SECS = 2 * 3600; // this working block
const DONE_T3_SECS = 12 * 3600; // overnight; past this the Recent lens takes over
type DoneTier = "" | "t1" | "t2" | "t3";
function doneTier(epoch: number, nowSec: number): DoneTier {
  if (!epoch) return "";
  const age = nowSec - epoch;
  if (age < DONE_T1_SECS) return "t1"; // negative (clock skew) lands here too
  if (age < DONE_T2_SECS) return "t2";
  if (age < DONE_T3_SECS) return "t3";
  return "";
}
/// "When did I last USE this place" — opened or worked, whichever is newer. Work
/// outranks nothing; it is the other half of the same fact, since a place you
/// prompted in an hour ago is more current than one you opened yesterday.
///
/// Deliberately has NO commit-epoch fallback: the lists that answer "where was
/// I" (Resume, the Recent lens, auto-restore) must not be led by a branch tip.
/// A worktree created from the CLI and never touched has a commit from
/// yesterday and no user history at all — ranking it above a place actually
/// opened last week would put it at the top of Resume and make it the
/// restore-on-launch target.
const usedEpoch = (p: Place) =>
  Math.max(p.declared?.last_opened_epoch ?? 0, p.declared?.last_worked_epoch ?? 0);
/// Sort key for ⌘K, which has always fallen back to the last commit so a
/// never-opened place still lands somewhere sensible in the order rather than
/// sinking to the bottom of every list. The switcher keeps opens on purpose —
/// "jump back to where I just was" is its job. The NAV tree no longer sorts by
/// this: opens reshuffling the tree carried no signal (see activityAt).
const recencyEpoch = (p: Place) => usedEpoch(p) || p.last_commit_epoch || 0;

// fixed-order signal glyphs; geometry (3-col row grid) guarantees no collision.
//
// Drift (proposal §10) is a GLYPH, not a dot: `.status-dot.waiting` is already
// amber = "Claude needs input", the app's highest-value signal, and a second
// amber dot would be indistinguishable from it. It goes FIRST because the
// overflow below truncates at MAX — and drift is the one signal here that names
// a broken worktree rather than ordinary work in progress.
function glyphs(p: Place, drift?: boolean) {
  const g: { cls: string; text: string; title: string }[] = [];
  if (drift) g.push({ cls: "g-drift", text: "⚑", title: "config drift — right-click the project → Project settings…" });
  if (p.dirty) g.push({ cls: "g-dirty", text: `●${p.dirty_files ?? ""}`, title: `${p.dirty_files ?? "dirty"} uncommitted` });
  if (p.ahead) g.push({ cls: "g-ahead", text: `↑${p.ahead}`, title: `${p.ahead} ahead of the base branch` });
  if (p.behind) g.push({ cls: "g-behind", text: `↓${p.behind}`, title: `${p.behind} behind the base branch` });
  if (p.detached) g.push({ cls: "g-det", text: "⑂", title: "detached HEAD" });
  const MAX = 4;
  if (g.length > MAX) return [...g.slice(0, MAX), { cls: "g-more", text: `+${g.length - MAX}`, title: "more" }];
  return g;
}

// Right-click context menu shell: fixed at the cursor, clamped to the viewport,
// Esc / click-away / right-click-away all close. Top-level (NOT nested in App)
// so its position state survives App re-renders.
function CtxMenu({ x, y, onClose, children }: { x: number; y: number; onClose: () => void; children: ReactNode }) {
  const ref = useRef<HTMLDivElement | null>(null);
  const [pos, setPos] = useState({ left: x, top: y });
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setPos({
      left: Math.max(4, Math.min(x, window.innerWidth - r.width - 8)),
      top: Math.max(4, Math.min(y, window.innerHeight - r.height - 8)),
    });
  }, [x, y]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <>
      <div className="menu-catch" onClick={onClose} onContextMenu={(e) => { e.preventDefault(); onClose(); }} />
      <div ref={ref} className="ctxmenu" style={pos} onContextMenu={(e) => e.preventDefault()}>
        {children}
      </div>
    </>
  );
}

type Ctx =
  | { kind: "place"; x: number; y: number; repo: string; slug: string }
  | { kind: "project"; x: number; y: number; root: string };

// Single-quote for pasting into a shell (the main session name carries parens).
const shq = (s: string) => `'${s.replace(/'/g, "'\\''")}'`;

// "v0.2.1" vs "0.2.0" — numeric per-component version compare (a > b).
const vparts = (s: string) => s.replace(/^v/, "").split(".").map((n) => parseInt(n, 10) || 0);
const vnewer = (a: string, b: string) => {
  const x = vparts(a), y = vparts(b);
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    if ((x[i] ?? 0) !== (y[i] ?? 0)) return (x[i] ?? 0) > (y[i] ?? 0);
  }
  return false;
};

// Sections of the embedded CHANGELOG strictly newer than `seen`, up to `current`.
function changelogBetween(changelog: string, seen: string, current: string): string {
  const sections = changelog.split(/^## /m).slice(1);
  const out: string[] = [];
  for (const sec of sections) {
    const m = sec.match(/^\[([0-9][^\]]*)\]/);
    if (!m) continue;
    const v = m[1];
    if (vnewer(v, seen) && !vnewer(v, current)) out.push("## " + sec.trimEnd());
  }
  return out.join("\n\n");
}

// keepachangelog markdown → typed sections, so "What's new" renders formatted
// notes instead of raw markdown. Hard-wrapped bullet continuations are unwrapped.
type NotesGroup = { name: string; items: string[] };
type NotesSection = { version: string; date: string; groups: NotesGroup[] };

function parseNotes(md: string): NotesSection[] {
  const sections: NotesSection[] = [];
  let sec: NotesSection | null = null;
  let group: NotesGroup | null = null;
  for (const line of md.split("\n")) {
    const h2 = line.match(/^## \[([^\]]+)\](?:\s*-\s*(.*))?/);
    if (h2) {
      sec = { version: h2[1], date: (h2[2] ?? "").trim(), groups: [] };
      sections.push(sec);
      group = null;
      continue;
    }
    const h3 = line.match(/^### (.+)/);
    if (h3 && sec) {
      group = { name: h3[1].trim(), items: [] };
      sec.groups.push(group);
      continue;
    }
    const li = line.match(/^[-*] (.+)/);
    if (li && group) { group.items.push(li[1].trim()); continue; }
    if (line.trim() && group && group.items.length) {
      group.items[group.items.length - 1] += " " + line.trim();
    }
  }
  return sections.filter((s) => s.groups.some((g) => g.items.length));
}

// `code` / **strong** / *em* — the inline markup the changelog uses. One
// alternation, code first, so a `**` inside a code span stays literal. The
// inner text of a match can never re-match its own delimiter (each arm forbids
// it), so the recursion below bottoms out after at most two levels.
const INLINE = /(`[^`]+`|\*\*[^*]+\*\*|\*[^*\s][^*]*\*)/;
function renderInline(s: string): React.ReactNode[] {
  return s.split(INLINE).map((part, i) => {
    if (i % 2 === 0) return part;
    if (part.startsWith("`")) return <code key={i}>{part.slice(1, -1)}</code>;
    if (part.startsWith("**")) return <strong key={i}>{renderInline(part.slice(2, -2))}</strong>;
    return <em key={i}>{renderInline(part.slice(1, -1))}</em>;
  });
}

function ReleaseNotes({ notes }: { notes: string }) {
  const sections = parseNotes(notes);
  if (!sections.length) return <pre className="notes">{notes}</pre>;
  return (
    <div className="relnotes">
      {sections.map((sec) => (
        <section key={sec.version}>
          <div className="rel-head">
            {sections.length > 1 && <b className="rel-v">v{sec.version}</b>}
            {sec.date && <span className="rel-date">{sec.date}</span>}
          </div>
          {sec.groups.map((g) => (
            <div key={g.name} className="rel-group">
              <span className={`rel-tag rel-${g.name.toLowerCase()}`}>{g.name}</span>
              <ul>
                {g.items.map((it, i) => <li key={i}>{renderInline(it)}</li>)}
              </ul>
            </div>
          ))}
        </section>
      ))}
    </div>
  );
}

// New-worktree form. Module scope + OWN draft state: components defined inside
// App get a fresh identity every render, which remounts their DOM and drops
// input focus per keystroke — the form must live outside that churn.
// `initialBranch`/`initialName` exist so a FAILED create can hand the user back
// what they typed. The form is dismissed the moment it is submitted (the nav's
// pending row takes over from there), so without this the three fields would die
// with the unmount and a rejected branch name — a typo'd base, a branch already
// checked out elsewhere — would cost a full retype to correct.
function NewPlaceForm({ project, initialBranch, initialName, initialBase, onCreate, onCancel }: {
  project: string;
  initialBranch: string;
  initialName: string;
  initialBase: string;
  onCreate: (branch: string, name: string, base: string) => void;
  onCancel: () => void;
}) {
  const [branch, setBranch] = useState(initialBranch);
  const [name, setName] = useState(initialName);
  const [base, setBase] = useState(initialBase);
  const submit = () => { if (branch.trim()) onCreate(branch.trim(), name.trim(), base.trim()); };
  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") submit();
    if (e.key === "Escape") onCancel();
  };
  return (
    <div className="newform nav-newform">
      <div className="newform-h">
        New worktree · <b>{basename(project)}</b>
        <button className="mini" title="cancel (Esc)" onClick={onCancel}><Icons.X size={13} /></button>
      </div>
      <input placeholder="branch (e.g. feat/x)" value={branch} autoFocus
        onChange={(e) => setBranch(e.currentTarget.value)} onKeyDown={onKey} />
      <input placeholder="name (optional)" value={name}
        onChange={(e) => setName(e.currentTarget.value)} onKeyDown={onKey} />
      <input placeholder="base (default: main)" value={base}
        onChange={(e) => setBase(e.currentTarget.value)} onKeyDown={onKey} />
      <button onClick={submit} disabled={!branch.trim()}>Create</button>
    </div>
  );
}

// A place that does not exist yet — `new` is still running. It stands in the
// nav where the real row will land, so the seconds of git fetch / worktree add /
// tmux read as work in progress instead of a dead app. Deliberately inert: no
// click, no context menu, no drag — there is nothing behind it to act on yet.
// Module scope, like every other stateless row.
function PendingRow({ label }: { label: string }) {
  return (
    <li className="row pending" title={`creating ${label}…`} aria-busy="true">
      <span className="status-dot pending" />
      <span className="row-id"><span className="row-name">{label}</span></span>
      <span className="glyphs"><span className="row-age">creating…</span></span>
    </li>
  );
}

// A picked folder that isn't a git repo. Offering `git init` here is the whole
// point — the old path just said "Not inside a git repository." and stopped.
// The empty first commit rides along because a repo without one cannot host a
// worktree at all (git: "not a valid object name"), so init-only would just
// move the dead end one click later. Module scope: keeps its busy flag across
// App's re-renders.
function InitRepoPrompt({ dir, onInit, onCancel }: {
  dir: string;
  onInit: (dir: string) => Promise<void>;
  onCancel: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const run = async () => {
    setBusy(true);
    await onInit(dir);
    setBusy(false);
  };
  return (
    <div className="newform nav-newform">
      <div className="newform-h">
        Not a git repo · <b>{basename(dir)}</b>
        <button className="mini" title="cancel" onClick={onCancel}><Icons.X size={13} /></button>
      </div>
      <p className="newform-note">
        <code>{dir}</code> isn't a git repository. Initialize it with an empty first commit?
      </p>
      <button disabled={busy} onClick={run}>{busy ? "Initializing…" : "git init + first commit"}</button>
    </div>
  );
}

// Tracked repo, unborn HEAD: `new` cannot work until a commit exists, so the
// new-worktree form is replaced by the one action that unblocks it.
function UnbornPrompt({ project, onCommit, onCancel }: {
  project: string;
  onCommit: (repo: string) => Promise<void>;
  onCancel: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const run = async () => {
    setBusy(true);
    await onCommit(project);
    setBusy(false);
  };
  return (
    <div className="newform nav-newform">
      <div className="newform-h">
        No commits yet · <b>{basename(project)}</b>
        <button className="mini" title="cancel (Esc)" onClick={onCancel}><Icons.X size={13} /></button>
      </div>
      <p className="newform-note">
        git can't branch off an unborn HEAD. Make the first commit, then create worktrees.
      </p>
      <button disabled={busy} onClick={run}>{busy ? "Committing…" : "Create initial commit"}</button>
    </div>
  );
}

// ── ⌘K quick-switcher ──
// Fuzzy SUBSEQUENCE match over a composite key (slug + branch + project basename
// + note). A place matches if the query chars appear IN ORDER (case-insensitive);
// score prefers a contiguous substring hit over a scattered subsequence, an
// earlier hit over a later one, and a slug hit over a branch/project hit. Recency
// (`recencyEpoch` desc — opened OR worked) breaks ties. Empty query → all places sorted
// by recency (the instant "recent places" list — the common case).
type SwitchItem = { pv: ProjectView; p: Place };
const SWITCH_CAP = 50; // rendered-list cap for big workspaces (perf; query narrows it)

// subsequence test + a small score; higher is better, -1 = no match.
function fuzzyScore(query: string, hay: string): number {
  if (!query) return 0;
  const q = query, h = hay;
  // strong signal: contiguous substring (and where it lands)
  const sub = h.indexOf(q);
  if (sub >= 0) return 1000 - sub; // earlier = better
  // fall back to in-order subsequence walk
  let hi = 0, gaps = 0, first = -1;
  for (let qi = 0; qi < q.length; qi++) {
    const c = q[qi];
    const found = h.indexOf(c, hi);
    if (found < 0) return -1;
    if (first < 0) first = found;
    if (qi > 0 && found > hi) gaps++;
    hi = found + 1;
  }
  return 400 - first - gaps * 5; // scattered: below any substring hit
}

function QuickSwitch({ open, items, busyPaths, waitingPaths, onPick, onClose }: {
  open: boolean;
  items: SwitchItem[];
  busyPaths: Set<string>;
  waitingPaths: Set<string>;
  onPick: (root: string, p: Place) => void;
  onClose: () => void;
}) {
  const [query, setQuery] = useState("");
  const [hi, setHi] = useState(0);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const listRef = useRef<HTMLDivElement | null>(null);

  const results = useMemo(() => {
    const q = query.trim().toLowerCase();
    const rec = recencyEpoch;
    if (!q) {
      return [...items]
        .sort((a, b) => rec(b.p) - rec(a.p))
        .slice(0, SWITCH_CAP);
    }
    const scored: { it: SwitchItem; score: number }[] = [];
    for (const it of items) {
      const { pv, p } = it;
      const slug = p.slug.toLowerCase();
      const branch = (p.branch ?? "").toLowerCase();
      const proj = basename(pv.root).toLowerCase();
      const note = (p.declared?.note ?? "").toLowerCase();
      const composite = `${slug} ${branch} ${proj} ${note}`;
      // reject early: must match the whole composite as a subsequence
      if (fuzzyScore(q, composite) < 0) continue;
      // rank on the best field, biased toward the slug
      const s = Math.max(
        fuzzyScore(q, slug) + 200, // slug hits win
        fuzzyScore(q, branch),
        fuzzyScore(q, proj),
        fuzzyScore(q, note),
      );
      scored.push({ it, score: s });
    }
    scored.sort((a, b) => (b.score - a.score) || (rec(b.it.p) - rec(a.it.p)));
    return scored.slice(0, SWITCH_CAP).map((x) => x.it);
  }, [items, query]);

  // fresh mount per open (App renders <QuickSwitch> only when open) → autofocus
  // fires and query/highlight reset. Focus the input on mount as a belt-and-braces
  // (autoFocus can miss when the node mounts inside a portal-less overlay).
  useEffect(() => {
    if (open) inputRef.current?.focus();
  }, [open]);

  // keep the highlighted row in view as Arrow keys move it
  useEffect(() => {
    listRef.current?.querySelector<HTMLElement>(".qs-row.hi")?.scrollIntoView({ block: "nearest" });
  }, [hi]);

  if (!open) return null;

  const pick = (it: SwitchItem) => onPick(it.pv.root, it.p);
  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setHi((i) => (results.length ? Math.min(i + 1, results.length - 1) : 0));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setHi((i) => Math.max(i - 1, 0));
    } else if (e.key === "Home") {
      e.preventDefault();
      setHi(0);
    } else if (e.key === "End") {
      e.preventDefault();
      setHi(Math.max(results.length - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const it = results[hi];
      if (it) pick(it);
    } else if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    }
  };

  return (
    <div className="scrim scrim-center" onClick={onClose}>
      <div className="qs-panel" onClick={(e) => e.stopPropagation()} role="dialog" aria-label="Quick switch">
        <input
          ref={inputRef}
          className="qs-input"
          placeholder="Jump to a place…"
          value={query}
          autoFocus
          spellCheck={false}
          onChange={(e) => { setQuery(e.currentTarget.value); setHi(0); }}
          onKeyDown={onKey}
        />
        <div className="qs-list" ref={listRef}>
          {results.length === 0 ? (
            <div className="qs-empty">No places match</div>
          ) : (
            results.map((it, i) => {
              const { pv, p } = it;
              const act = busyPaths.has(p.path) ? "busy" : waitingPaths.has(p.path) ? "waiting" : "";
              return (
                <div
                  key={`${pv.root}::${p.slug}`}
                  className={"qs-row" + (i === hi ? " hi" : "")}
                  onMouseEnter={() => setHi(i)}
                  onMouseDown={(e) => { e.preventDefault(); pick(it); }} // mousedown: fire before the input blurs
                >
                  <span
                    className={"status-dot" + (act ? " " + act : "")}
                    title={act === "busy" ? "Claude working" : act === "waiting" ? "Claude needs input" : undefined}
                  />
                  <span className="qs-slug">
                    {p.declared?.pinned ? "★ " : p.is_main ? "◆ " : ""}{p.slug}
                  </span>
                  <span className="qs-proj">{basename(pv.root)}</span>
                  {p.branch && p.branch !== p.slug && <span className="qs-branch">{p.branch}</span>}
                  <span className={"life " + p.lifecycle_effective}>{p.lifecycle_effective}</span>
                </div>
              );
            })
          )}
        </div>
      </div>
    </div>
  );
}

// ── right dock: files + embedded terminal ───────────────────────────────────
// The Files tab (tree, viewers, split layout) lives in FilesPane.tsx. Both are
// at MODULE scope — components defined inside App() remount every render.

// Files-tab layout cycle for the dock header button.
const NEXT_FILES_LAYOUT: Record<Settings["files_layout"], Settings["files_layout"]> = {
  auto: "stack", stack: "split", split: "auto",
};

// A dock shell is just a ShellPane now — the backend spawns-or-reattaches on
// open, keyed by repo+slug+index (the webview never names one), so there's no
// separate "create the session first" round trip to wait on.

// Terminal tab: several shells per place. Tabs are restored from the shells the
// backend still has running — within a session that survives dock closes and
// place switches, but NOT an app restart (the PTYs are ours, nothing outlives
// the process). A live place defaults to one shell, a closed place to none.
// Only the active shell is mounted; the rest keep running detached, and their
// output is replayed from the backend's ring buffer when you flip back.
// `addToken` bumps → add a tab (⌘⇧T from the global handler).
//
// Inline tab-rename box. Module scope with props, per CLAUDE.md: defined inside
// TerminalTabs it would get a new identity on every render and lose focus after
// the first keystroke. Enter / blur commit, Esc cancels; keys are stopped from
// reaching the global shortcut handler (Esc there closes sheets).
function TermTabRename({ initial, onCommit, onCancel }: {
  initial: string; onCommit: (name: string) => void; onCancel: () => void;
}) {
  const [text, setText] = useState(initial);
  const ref = useRef<HTMLInputElement | null>(null);
  const settled = useRef(false); // Esc unmounts → don't let a trailing blur commit
  useEffect(() => { ref.current?.focus(); ref.current?.select(); }, []);
  const commit = () => { if (settled.current) return; settled.current = true; onCommit(text.trim()); };
  return (
    <input
      ref={ref} className="termtab-rename" value={text} spellCheck={false}
      onChange={(e) => setText(e.currentTarget.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") { e.preventDefault(); commit(); }
        else if (e.key === "Escape") { e.preventDefault(); settled.current = true; onCancel(); }
      }}
    />
  );
}

function TerminalTabs({ repo, slug, sessionUp, termVersion, focusToken, addToken, names, onRename, onError }: {
  repo: string; slug: string; sessionUp: boolean; termVersion: number; focusToken: number; addToken: number;
  names: Record<number, string>; onRename: (index: number, name: string | null) => void;
  onError: (e: unknown) => void;
}) {
  const [ids, setIds] = useState<number[] | null>(null); // null = restoring
  const [active, setActive] = useState<number | null>(null);
  // exited-but-kept tabs (see the shell:exit listener below) — declared here so
  // the restore can seed it: the exit EVENT is transient and a shell that died
  // while the dock was closed had no listener, so liveness rides on the restore
  const [dead, setDead] = useState<number[]>([]);
  const idsRef = useRef<number[]>([]);
  idsRef.current = ids ?? [];
  const restoringRef = useRef(true);
  // names is read by the restore, which must NOT re-run on every rename
  const namesRef = useRef(names);
  namesRef.current = names;
  const [editing, setEditing] = useState<number | null>(null);
  const labelOf = (id: number) => names[id] || `sh ${id}`;

  // Restore tabs from the live shell registry on mount / place change, UNIONed
  // with the tabs the user named for this place. Names outlive the process (they
  // live in ui-state.json) while shells don't, so a named tab comes back as a
  // tab: activating it mounts ShellPane, which spawns a fresh shell. The union
  // is gated on a live session — a closed place still shows nothing.
  useEffect(() => {
    let alive = true;
    restoringRef.current = true;
    setIds(null); setActive(null); setDead([]); setEditing(null);
    invoke<{ index: number; dead: boolean }[]>("list_shell_sessions", { repo, slug })
      .then((existing) => {
        if (!alive) return;
        const named = sessionUp
          ? Object.keys(namesRef.current).map(Number).filter((n) => Number.isInteger(n) && n > 0)
          : [];
        const union = [...new Set([...existing.map((t) => t.index), ...named])].sort((a, b) => a - b);
        const list = union.length ? union : (sessionUp ? [1] : []);
        setDead(existing.filter((t) => t.dead).map((t) => t.index));
        setIds(list); setActive(list[0] ?? null); restoringRef.current = false;
      })
      .catch((e) => {
        if (!alive) return;
        setIds(sessionUp ? [1] : []); setActive(sessionUp ? 1 : null); restoringRef.current = false; onError(e);
      });
    return () => { alive = false; };
    // sessionUp intentionally excluded — its transitions are handled below so a
    // flip doesn't clobber the user's tabs mid-session.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [repo, slug]);

  // When the place's session goes DOWN, Close swept its dock shells too (same
  // rule as the tmux era: scratch shells die with the place) — clear the tabs so
  // the dock reflects reality instead of resurrecting dead ones.
  const prevUp = useRef(sessionUp);
  useEffect(() => {
    if (prevUp.current && !sessionUp) { setIds([]); setActive(null); }
    prevUp.current = sessionUp;
  }, [sessionUp]);

  // A shell that exits on its own (you typed `exit`, or it died) keeps its tab
  // and says so — a tab that silently vanished would look like a bug, and the
  // scrollback is often the thing you wanted to read. (`dead` state lives above,
  // next to `ids`, because the restore seeds it too.)
  useEffect(() => {
    const un = listen<{ repo: string; slug: string; index: number }>("shell:exit", (e) => {
      if (e.payload.repo !== repo || e.payload.slug !== slug) return;
      setDead((d) => (d.includes(e.payload.index) ? d : [...d, e.payload.index]));
    });
    return () => { un.then((f) => f()); };
  }, [repo, slug]);

  const restartTab = (id: number) => {
    invoke("close_shell_session", { repo, slug, index: id }).catch(onError);
    setDead((d) => d.filter((x) => x !== id));
    setRestartToken((t) => t + 1); // remount the pane → shell_open spawns afresh
  };
  const [restartToken, setRestartToken] = useState(0);

  const addTab = useCallback(() => {
    if (restoringRef.current) return; // don't add a tab the restore is about to overwrite
    const cur = idsRef.current;
    const next = (cur.length ? Math.max(...cur) : 0) + 1;
    setIds([...cur, next]);
    setActive(next);
  }, []);

  // ⌘⇧T → add a tab. Skip the initial token value so mounting doesn't add one.
  const firstTok = useRef(true);
  useEffect(() => {
    if (firstTok.current) { firstTok.current = false; return; }
    addTab();
  }, [addToken, addTab]);

  const closeTab = (id: number) => {
    invoke("close_shell_session", { repo, slug, index: id }).catch(onError);
    onRename(id, null); // an explicitly closed tab drops its name — otherwise it
                        // would be seeded straight back on the next restore
    const remaining = idsRef.current.filter((x) => x !== id);
    setIds(remaining);
    if (active === id) setActive(remaining.length ? remaining[remaining.length - 1] : null);
  };

  if (ids === null) return <div className="tree-note">…</div>;

  return (
    <div className="termtabs">
      <div className="termtab-strip">
        {ids.map((id) => (
          <span key={id} className={"termtab" + (active === id ? " on" : "") + (dead.includes(id) ? " dead" : "")}>
            {editing === id ? (
              <TermTabRename
                initial={labelOf(id)}
                onCommit={(name) => { onRename(id, name && name !== `sh ${id}` ? name : null); setEditing(null); }}
                onCancel={() => setEditing(null)}
              />
            ) : (
              <button className="termtab-label" title={dead.includes(id) ? "process exited" : "double-click to rename"}
                onClick={() => setActive(id)} onDoubleClick={() => setEditing(id)}>{labelOf(id)}</button>
            )}
            <button className="termtab-x" title="close shell" onClick={() => closeTab(id)}><Icons.X size={11} /></button>
          </span>
        ))}
        <button className="termtab-add" title="new terminal (⌘⇧T)" onClick={addTab}><Icons.Plus size={13} /></button>
      </div>
      {active != null ? (
        dead.includes(active) ? (
          <div className="term-empty">
            <div className="term-empty-card">
              <div className="te-title">{labelOf(active)} — process exited</div>
              <button className="enter-btn" onClick={() => restartTab(active)}>Restart shell</button>
            </div>
          </div>
        ) : (
          <ShellPane key={repo + "|" + slug + ":" + active + ":" + restartToken} repo={repo} slug={slug}
            index={active} termVersion={termVersion} focusToken={focusToken} />
        )
      ) : (
        <div className="term-empty">
          <div className="term-empty-card">
            <div className="te-title">No shells</div>
            <button className="enter-btn with-icon" onClick={addTab}><Icons.Plus size={13} /> new terminal</button>
          </div>
        </div>
      )}
    </div>
  );
}

// Status-bar branch switcher. A combobox, NOT a <select>: `switch` is DWIM —
// local branch → switch, remote-only → track it, anything else → create it off
// the default base — so a picker that only offered existing branches would
// delete the create path. Typing stays first-class; the list just means you no
// longer have to remember the name.
//
// Module scope with props, per CLAUDE.md: it holds local state and input focus,
// both of which a component defined inside App() would lose on every render.
type BranchList = { branches: string[]; current: string; default_base: string };

function BranchSwitcher({ repo, slug, onSwitch, onError }: {
  repo: string; slug: string;
  onSwitch: (branch: string) => Promise<boolean>;
  onError: (e: unknown) => void;
}) {
  const [text, setText] = useState("");
  const [open, setOpen] = useState(false);
  const [data, setData] = useState<BranchList | null>(null);
  const [hi, setHi] = useState(0);
  const boxRef = useRef<HTMLDivElement | null>(null);

  // Reset when the place changes — a half-typed branch belongs to the place it
  // was typed in.
  useEffect(() => { setText(""); setOpen(false); setData(null); }, [repo, slug]);

  // Lazy: one `git for-each-ref` when the switcher is first used, not on every
  // place selection.
  const load = () => {
    if (data) return;
    invoke<BranchList>("list_branches", { repo, slug })
      .then(setData)
      .catch(onError);
  };

  const q = text.trim();
  const matches = (data?.branches ?? [])
    .filter((b) => b !== data?.current && b.toLowerCase().includes(q.toLowerCase()))
    .slice(0, 50);
  const exact = matches.some((b) => b === q);
  const creating = q.length > 0 && !exact;
  // the create row is last, so ↓ from the top walks real branches first
  const options: { branch: string; create: boolean }[] = [
    ...matches.map((b) => ({ branch: b, create: false })),
    ...(creating ? [{ branch: q, create: true }] : []),
  ];

  const submit = async (branch: string) => {
    const b = branch.trim();
    if (!b) return;
    setOpen(false);
    if (await onSwitch(b)) setText("");
  };

  // click-away — the popover floats over the terminal, so it must not linger
  useEffect(() => {
    if (!open) return;
    const away = (e: MouseEvent) => {
      if (!boxRef.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", away);
    return () => document.removeEventListener("mousedown", away);
  }, [open]);

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (!open) { setOpen(true); load(); return; }
      const d = e.key === "ArrowDown" ? 1 : -1;
      setHi((i) => (options.length ? (i + d + options.length) % options.length : 0));
      return;
    }
    if (e.key === "Escape" && open) { e.preventDefault(); setOpen(false); return; }
    if (e.key === "Enter") {
      // A highlighted row wins; otherwise the raw text does, so Enter still
      // works exactly as it did before the list existed.
      submit(open && options[hi] ? options[hi].branch : text);
    }
  };

  return (
    <div className="combo" ref={boxRef}>
      <input
        className="switchto"
        placeholder="switch branch…"
        value={text}
        onFocus={() => { setOpen(true); load(); }}
        onChange={(e) => { setText(e.currentTarget.value); setHi(0); setOpen(true); load(); }}
        onKeyDown={onKey}
      />
      {open && (
        <div className="combo-pop">
          {data === null && <div className="combo-note">loading branches…</div>}
          {data !== null && options.length === 0 && (
            <div className="combo-note">{data.branches.length ? "no branch matches" : "no other branches"}</div>
          )}
          {options.map((o, i) => (
            <button
              key={(o.create ? "new:" : "b:") + o.branch}
              className={"combo-item" + (i === hi ? " hi" : "") + (o.create ? " create" : "")}
              // mousedown, not click: the input's blur would tear the row out
              // from under the click
              onMouseDown={(e) => { e.preventDefault(); submit(o.branch); }}
              onMouseEnter={() => setHi(i)}
            >
              {o.create ? (
                <>create <b>{o.branch}</b> off {data?.default_base}</>
              ) : (
                o.branch
              )}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

// tmux is missing: every place's session is unreachable, and nothing in the UI
// said so — it just looked like everything was dead. Module scope (not inside
// App) so the "checking…" state survives App's re-renders.
function TmuxBanner({ onRecheck }: { onRecheck: () => Promise<boolean> }) {
  const [busy, setBusy] = useState(false);
  const [stillMissing, setStillMissing] = useState(false);
  const recheck = async () => {
    setBusy(true);
    setStillMissing(false);
    const ok = await onRecheck();
    setBusy(false);
    // `ok` retires the whole banner from App — only the miss needs saying here.
    if (!ok) setStillMissing(true);
  };
  return (
    <div className="tmux-banner">
      <Icons.TriangleAlert size={14} />
      <span className="tmux-banner-txt">
        tmux is not installed — sessions need it. macOS: <code>brew install tmux</code>
      </span>
      {stillMissing && <span className="tmux-banner-miss">still not found</span>}
      <button className="tmux-banner-btn" disabled={busy} onClick={recheck}>
        {busy ? "Checking…" : "Re-check"}
      </button>
    </div>
  );
}

// ── Claude plan usage (nav footer) ──────────────────────────────────────────
// The same bars as Claude Code's /usage panel: 5h session window, weekly
// all-models, plus any model-scoped weekly bucket ("Fable"). Backend
// (`claude_usage`) never errors on missing data — it answers
// source: "unavailable" and we render nothing, so a machine without Claude Code
// credentials just has a plain nav.
//
// Module scope with props, per CLAUDE.md: it owns poll state, which a component
// declared inside App() would throw away (and re-fetch) on every render.
type UsageLimit = {
  kind: string;
  label: string;
  percent: number;
  severity: string;
  resets_at: number | null;
};
type UsageInfo = { source: string; fetched_at: number; limits: UsageLimit[] };

// 180s — the endpoint is undocumented and has rate-limited hard before; the
// backend also caps real fetches at one per 120s.
const USAGE_POLL_MS = 180_000;
/// Afterglow re-tier cadence. Boundaries then land within ±60s of true, which is
/// invisible at 15m/2h/12h — and it is a pure recompute, no I/O.
const DECAY_TICK_MS = 60_000;
// The countdown ticks off the LOCAL clock, not off a poll: `resets_at` is
// absolute, so a 15s tick keeps the minute display honest without touching the
// rate-limited endpoint (polling harder to animate a clock would be absurd).
const USAGE_TICK_MS = 15_000;

/** "5h" / "7d" for the two standard windows; model buckets keep their name. */
function usageTick(l: UsageLimit): string {
  if (l.kind === "session") return "5h";
  if (l.kind === "weekly_all") return "7d";
  return l.label;
}

/** Seconds-until-reset → two units, biggest first: "2d 5h", "3h 02m", "42m",
 *  "<1m". Empty once the window has rolled over — a window whose reset is in the
 *  past (the statusline snapshot is often that stale) shows no countdown rather
 *  than a negative one. Minutes are zero-padded so the column can't jitter. */
function fmtEta(secs: number): string {
  if (secs <= 0) return "";
  if (secs < 60) return "<1m";
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ${String(mins % 60).padStart(2, "0")}m`;
  const days = Math.floor(hours / 24);
  // "3d 0h" reads as noise — at day scale the zero hour carries nothing
  return hours % 24 === 0 ? `${days}d` : `${days}d ${hours % 24}h`;
}

/** Is anyone actually looking at this window?
 *
 * `document.visibilityState` — WebKit drives it off
 * `NSWindowDidChangeOcclusionState`, so it catches minimize, ⌘H, a Space switch
 * and FULL occlusion. It deliberately does NOT catch plain focus loss: a visible
 * but unfocused window is a dashboard someone is still reading, and freezing it
 * would be a bug, not a saving.
 *
 * This hook tracks visibility ONLY. Cursor blink is the one thing that follows
 * window FOCUS, and it lives in TerminalPane with its own listeners — keeping a
 * focus boolean here too would re-render the whole App tree on every ⌘-Tab to
 * store something nobody reads, which is the exact cost this file is trying to
 * remove.
 *
 * Every transition is logged, because whether WKWebView really fires
 * `visibilitychange` for occlusion is the one thing in this whole effort that
 * the test suite cannot answer — read app.log (Settings → Logs) to confirm on a
 * real build before trusting the gate.
 */
function useWindowAwake() {
  const [pageVisible, setPageVisible] = useState(() => document.visibilityState !== "hidden");
  useEffect(() => {
    const onVis = () => {
      const v = document.visibilityState !== "hidden";
      setPageVisible(v);
      invoke("log_event", { level: "info", msg: `window ${v ? "visible" : "hidden"}` }).catch(() => {});
    };
    document.addEventListener("visibilitychange", onVis);
    return () => document.removeEventListener("visibilitychange", onVis);
  }, []);
  return { pageVisible };
}

function UsageWidget({ onError }: { onError: (e: unknown) => void }) {
  const [info, setInfo] = useState<UsageInfo | null>(null);
  const [nowSec, setNowSec] = useState(() => Math.floor(Date.now() / 1000));
  const { pageVisible } = useWindowAwake();

  // Own timer, deliberately separate from the poll effect: cheap (a number in a
  // module-scope component, so the re-render stays inside the widget) and it
  // must keep running between the 180s pulls — but only while the window is
  // actually on screen. Nobody needs a countdown animated at a minimized window.
  useEffect(() => {
    if (!pageVisible) return;
    setNowSec(Math.floor(Date.now() / 1000)); // re-zero: we may have been away for hours
    const id = setInterval(() => setNowSec(Math.floor(Date.now() / 1000)), USAGE_TICK_MS);
    return () => clearInterval(id);
  }, [pageVisible]);

  useEffect(() => {
    let alive = true;
    const pull = () => {
      // a pull is also the moment to re-zero the countdown clock: coming back
      // from sleep, the 15s tick may be up to a tick behind
      setNowSec(Math.floor(Date.now() / 1000));
      invoke<UsageInfo>("claude_usage")
        .then((u) => { if (alive) setInfo(u); })
        .catch((e) => { if (alive) onError(e); });
    };
    // Hidden → no pull and no interval. The guard covers the immediate pull too:
    // this effect re-runs on the hidden edge, and invoking there would spend a
    // fetch on the transition into going quiet. Coming back re-runs it visible,
    // so the catch-up pull is free. A doubled pull with the focus listener below
    // is harmless: the backend caps real fetches at one per 120s.
    if (pageVisible) pull();
    const id = pageVisible ? setInterval(pull, USAGE_POLL_MS) : null;
    // coming back to the window is exactly when a stale bar is most visible
    window.addEventListener("focus", pull);
    return () => {
      alive = false;
      if (id) clearInterval(id);
      window.removeEventListener("focus", pull);
    };
  }, [onError, pageVisible]);

  if (!info || info.source === "unavailable" || info.limits.length === 0) return null;
  // statusline = a local snapshot only as fresh as the last Claude Code session
  const stale = info.source === "statusline";
  // no row has a live reset (endpoint dropped the field, or every window has
  // already rolled over) → no column at all, rather than a strip of blanks
  const anyEta = info.limits.some((l) => l.resets_at && l.resets_at > nowSec);
  return (
    <div
      className={"usage" + (stale ? " stale" : "")}
      title={stale ? `Claude usage — statusline snapshot from ${new Date(info.fetched_at * 1000).toLocaleString()}` : undefined}
    >
      {info.limits.map((l) => {
        const pct = Math.max(0, Math.min(100, Math.round(l.percent)));
        const tone = l.severity === "normal" ? "" : l.severity === "warning" ? " warn" : " over";
        const eta = l.resets_at ? fmtEta(l.resets_at - nowSec) : "";
        const resets = l.resets_at ? `, resets ${new Date(l.resets_at * 1000).toLocaleString()}` : "";
        return (
          <div
            className={"usage-row" + tone}
            key={l.kind + "|" + l.label}
            title={`${l.label} — ${pct}% used${eta ? `, ${eta} left` : ""}${resets}`}
          >
            <span className="usage-label">{usageTick(l)}</span>
            <span className="usage-bar"><i style={{ width: `${pct}%` }} /></span>
            <span className="usage-pct">{pct}%</span>
            {/* column is reserved even when a row's reset has passed, so the
                three ETAs stay right-aligned with each other */}
            {anyEta && <span className="usage-eta">{eta}</span>}
          </div>
        );
      })}
    </div>
  );
}

function App() {
  const [ws, setWs] = useState<Workspace | null>(null);
  const [err, setErr] = useState("");
  const [sel, setSel] = useState<{ repo: string; slug: string } | null>(null);
  const [filter, setFilter] = useState("");
  const [lens, setLens] = useState<Lens>("places");
  const [newFor, setNewFor] = useState<string | null>(null);
  const [newBase, setNewBase] = useState("");
  // What a REJECTED create had typed in it, so reopening the form restores the
  // fields instead of handing back three empty boxes. Null for a fresh form.
  const [newDraft, setNewDraft] = useState<{ branch: string; name: string; base: string } | null>(null);
  // Places whose `new` is still running. Creating one takes seconds of network
  // and disk (git fetch, worktree add, materialize, tmux) and the nav had NO
  // representation of it, so the app read as hung — the whole complaint. Each
  // entry is one in-flight op; the id keeps two concurrent creates from
  // retiring each other's row, since the slug is only a guess until core
  // answers with the real one.
  const [pendingNew, setPendingNew] = useState<{ id: number; repo: string; label: string }[]>([]);
  const pendingSeq = useRef(0);
  // A picked folder that is not a repo yet, awaiting the `git init` offer.
  const [initAsk, setInitAsk] = useState<string | null>(null);
  const [ctx, setCtx] = useState<Ctx | null>(null);
  const [confirmRm, setConfirmRm] = useState<string | null>(null);
  // The armed Close needs one fact more than `confirmRm` can carry: WHICH tmux
  // session would die, as the BACKEND named it. It rides alongside the arm
  // rather than replacing it, so `confirmRm` stays the single register every
  // dismissal path already clears and this can never re-arm anything by itself
  // (it is only ever read while confirmRm holds the matching close key).
  const [closeSess, setCloseSess] = useState("");
  // Same float as `err`, other colour: something the user must be TOLD but that
  // nothing failed at (an armed kill that hit a session which had already been
  // replaced). A red banner for a question is what this whole path exists to
  // stop, so it does not share the error register.
  const [notice, setNotice] = useState("");
  const [menu, setMenu] = useState<"life" | "more" | null>(null);
  const [groupOpen, setGroupOpen] = useState<Record<string, boolean>>({});
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const [settings, setSettings] = useState<Settings>(DEFAULTS);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [switchOpen, setSwitchOpen] = useState(false);
  const [termVersion, setTermVersion] = useState(0);
  const [termFocus, setTermFocus] = useState(0);
  const [upd, setUpd] = useState<UpdateInfo | null>(null);
  const [sortOpen, setSortOpen] = useState(false);
  const [drag, setDrag] = useState<{ repo: string; slug: string } | null>(null);
  const [dragOver, setDragOver] = useState<string | null>(null);
  const [whatsNew, setWhatsNew] = useState<{ version: string; notes: string; manual?: boolean } | null>(null);
  const [projSheet, setProjSheet] = useState<string | null>(null);
  // Per-project doctor state. Structurally identical to busyPaths/waitingPaths
  // (the house pattern for row decoration that is NOT in the snapshot) —
  // deliberately outside `Place`, because `Place.stack` is reserved for the infra
  // phase and putting drift on the hot path would mean a filesystem probe inside
  // every 3s poll.
  //
  // `slugs` decorates ROWS (place-attached findings only). `issues` is the
  // project-level count and includes placeless findings, so it matches the
  // sheet's Health badge exactly. `error` is set when doctor could not run at
  // all — and when that happens `slugs`/`issues` are LEFT AS THEY WERE (see
  // takeReport): an unreadable config is the most broken a project gets, and the
  // one thing it must not do is quietly undecorate everything.
  const [health, setHealth] = useState<Record<string, ProjectHealth>>({});
  // What `worktrees init` would suggest per project (§9's nudge). Probed ONCE per
  // project — it walks the checkout, so it is not poll-path work either.
  const [suggest, setSuggest] = useState<Record<string, InitSuggestion>>({});
  // Badge = actionable updates. CLI via the pinned-tag installer; the app via
  // tauri-plugin-updater (signed bundles) — both one click in Settings now.
  const cliStale = !!(upd?.latest && upd.cli_version && vnewer(upd.latest, upd.cli_version));
  const cliMissing = !!(upd?.latest && !upd.cli_version);
  const appStale = !!(upd?.latest && vnewer(upd.latest, upd.app_version));
  const updateAvail = cliStale || cliMissing || appStale;
  const searchRef = useRef<HTMLInputElement | null>(null);
  // right dock: which file the Files tab is viewing (null = none). Reset per place.
  const [dockFile, setDockFile] = useState<string | null>(null);
  // Reading mode (⌘⇧E): the open file takes over the main pane. Closed by a
  // place switch or by the file going away — an overlay with nothing under it
  // would hide the terminal for no reason.
  const [reading, setReading] = useState(false);
  useEffect(() => { setDockFile(null); setReading(false); }, [sel?.repo, sel?.slug]);
  useEffect(() => { if (!dockFile) setReading(false); }, [dockFile]);
  // ⌘J / the rail says "hide files" — leaving a full-pane reader behind would
  // make that a lie. Same for flipping the dock to the Terminal tab.
  const filesDockShown = settings.dock_open && settings.dock_tab === "files";
  useEffect(() => { if (!filesDockShown) setReading(false); }, [filesDockShown]);
  // ⌘⇧T bumps this → the dock's Terminal tab adds a shell (if mounted/visible).
  const [newTermToken, setNewTermToken] = useState(0);
  // bumps on places:changed → the dock's file viewer re-reads from disk when clean.
  const [placesToken, setPlacesToken] = useState(0);
  // Gates every periodic cost in the app — see useWindowAwake. Only the
  // visibility half is wanted here: cursor blink is the one thing that follows
  // FOCUS, and it is wired inside TerminalPane where the terminals live.
  const { pageVisible } = useWindowAwake();

  // every surfaced error also lands in the app log (Settings → Logs)
  const fail = useCallback((e: unknown) => {
    const m = String(e);
    setErr(m);
    invoke("log_event", { level: "error", msg: m }).catch(() => {});
  }, []);

  // The message a FAILED refresh put in the banner, so a later successful one can
  // retract it — and ONLY it. A blanket clear here is not an option: refresh also
  // runs on every "places:changed" event, so it would wipe whatever the command
  // that caused the change had just reported (runCmd, below). That is every
  // "the op ran and said no" message the app has — a dirty-remove refusal, a
  // branch collision, a shadowed file on relink — and silence is the bug this
  // whole surface exists to fix.
  const refreshErr = useRef("");
  // Serialized last snapshot, for the no-change bail below.
  const lastSnap = useRef("");
  const refresh = useCallback(async () => {
    try {
      const w = await invoke<Workspace>("list_workspace");
      // Read the claim into a local FIRST: the updater below runs when React gets
      // to it, by which time the ref would already be cleared and every message
      // would look like someone else's.
      const stale = refreshErr.current;
      refreshErr.current = "";
      if (stale) setErr((e) => (e === stale ? "" : e));
      // An idle poll returns a BYTE-IDENTICAL workspace, and the backend forces
      // one every 30s whether or not anything moved. Replacing `ws` anyway gave
      // it a fresh identity and re-rendered the whole tree for no change at all
      // — forever, in the background, on battery. Compare and bail.
      const snap = JSON.stringify(w);
      if (snap === lastSnap.current) return;
      lastSnap.current = snap;
      setWs(w);
    } catch (e) {
      refreshErr.current = String(e); // same text fail() shows
      fail(e);
    }
  }, []);
  /** The ONLY other way `ws` may be replaced. `lastSnap` is the dedupe's record
   * of what `ws` currently holds; a bare `setWs` leaves the two describing
   * different things, and a later refresh that happens to match `lastSnap` would
   * then bail while `ws` still held the stale value — permanently, because the
   * snapshot is stable at that point. Commands that mutate the workspace and get
   * a fresh one back go through here. */
  const commitWs = useCallback((w: Workspace) => {
    lastSnap.current = JSON.stringify(w);
    setWs(w);
  }, []);
  useEffect(() => { refresh(); }, [refresh]);

  // ── tmux presence ──
  // Optimistic default: assume present until the probe says otherwise, so the
  // banner never flashes on a machine that has tmux. Re-check re-resolves the
  // GUI PATH backend-side (it is resolved once at startup), which is the only
  // way a tmux installed while the app was open becomes visible without a
  // restart — so a successful one also re-pulls the workspace, otherwise every
  // place would keep showing the dead session it was last polled with.
  const [tmuxOk, setTmuxOk] = useState(true);
  useEffect(() => {
    invoke<boolean>("tmux_check", { refresh: false }).then(setTmuxOk).catch(fail);
  }, [fail]);
  const recheckTmux = useCallback(async () => {
    try {
      const ok = await invoke<boolean>("tmux_check", { refresh: true });
      setTmuxOk(ok);
      if (ok) { refresh(); setPlacesToken((v) => v + 1); }
      return ok;
    } catch (e) {
      fail(e);
      return false;
    }
  }, [fail, refresh]);

  // live refresh: backend emits "places:changed" (poll/fs-watch) → re-pull.
  //
  // This listener is the THROTTLE POINT for the app's biggest power cost. The
  // event itself is nearly free; `refresh()` is what invokes `list_workspace`,
  // and that fans out to a git subprocess per place per project — measured at
  // ~1.3 git spawns/second on an idle machine, running whether or not the
  // window was on screen. Hidden → remember that we owe a refresh and do
  // nothing else.
  //
  // `document.visibilityState` is read live rather than closing over
  // `pageVisible`, so the listener never has to be torn down and re-registered
  // (and can't act on a stale value).
  const pendingRefresh = useRef(false);
  useEffect(() => {
    const un = listen("places:changed", () => {
      if (document.visibilityState === "hidden") { pendingRefresh.current = true; return; }
      refresh();
      setPlacesToken((v) => v + 1);
    });
    return () => { un.then((f) => f()).catch(() => {}); };
  }, [refresh]);

  // The deferred catch-up. Coming back to the window is precisely when a stale
  // nav is most obvious, so this runs on the visible edge rather than waiting
  // for the next backend tick. It carries the Files tab with it: placesToken is
  // what the tree re-lists on, so a file written while the app was hidden is
  // there when you look, without its own visibility plumbing.
  useEffect(() => {
    if (!pageVisible || !pendingRefresh.current) return;
    pendingRefresh.current = false;
    refresh();
    setPlacesToken((v) => v + 1);
  }, [pageVisible, refresh]);

  // What the dock's Refresh button does. placesToken is what the Files tab
  // re-lists and re-reads off, so this is the same work "places:changed" does
  // — the button only exists to skip the wait for the next poll tick.
  const reloadFiles = useCallback(() => { refresh(); setPlacesToken((v) => v + 1); }, [refresh]);

  // Claude working state — pushed by the backend poll thread from the
  // ~/.claude/sessions/<pid>.json probes (see lib.rs claude_activity). Keyed by
  // WORKTREE PATH (probe cwd == place.path), not the tmux session name. Two sets:
  // busy → green blinking dot, waiting → static amber "needs input". Pure event
  // state: no extra invokes.
  const [busyPaths, setBusyPaths] = useState<Set<string>>(new Set());
  const [waitingPaths, setWaitingPaths] = useState<Set<string>>(new Set());
  useEffect(() => {
    const un = listen<{ busy: string[]; waiting: string[] }>("sessions:busy", (e) => {
      setBusyPaths(new Set(e.payload.busy));
      setWaitingPaths(new Set(e.payload.waiting));
    });
    return () => { un.then((f) => f()).catch(() => {}); };
  }, []);
  const activityOf = (p: Place): "busy" | "waiting" | "" =>
    busyPaths.has(p.path) ? "busy" : waitingPaths.has(p.path) ? "waiting" : "";

  // Task-completed stamps. Two sources, and the overlay is why both exist: the
  // snapshot carries `declared.last_worked_epoch` (durable, survives restarts,
  // backfilled at launch), but a snapshot only re-pulls on `places:changed` —
  // which a completion does NOT trigger, since finishing a task leaves no tmux
  // trace. The `sessions:done` event closes that gap instantly, at the cost of
  // one Map. Taking the max of the two means neither can regress the other.
  const [donePaths, setDonePaths] = useState<Map<string, number>>(new Map());
  useEffect(() => {
    const un = listen<{ path: string; epoch: number }>("sessions:done", (e) => {
      setDonePaths((m) => {
        const next = new Map(m);
        next.set(e.payload.path, Math.max(next.get(e.payload.path) ?? 0, e.payload.epoch));
        return next;
      });
    });
    return () => { un.then((f) => f()).catch(() => {}); };
  }, []);
  const workedAt = (p: Place) =>
    Math.max(donePaths.get(p.path) ?? 0, p.declared?.last_worked_epoch ?? 0);
  // Nav row age AND the "Activity" sort key: when something HAPPENED here —
  // Claude work or a commit, never an open. `last_opened_epoch` is deliberately
  // excluded so clicking a row neither resets its clock to "now" nor reshuffles
  // the tree. Opens still count where "where was I" is the question: the Recent
  // lens, Resume (usedEpoch), and ⌘K (recencyEpoch).
  const activityAt = (p: Place) => Math.max(workedAt(p), p.last_commit_epoch ?? 0);

  // The decay clock. One minute is far finer than the coarsest boundary it has
  // to land on (15m), and it is gated on visibility exactly like the usage poll
  // — a hidden window has nothing to re-render for. Returning to the window
  // re-runs this effect, so a woken nav corrects on the next frame instead of
  // waiting out the rest of an interval (the effect runs after paint, so one
  // stale-tier frame can show — invisible at these horizons).
  const [nowSec, setNowSec] = useState(() => Math.floor(Date.now() / 1000));
  useEffect(() => {
    if (!pageVisible) return;
    setNowSec(Math.floor(Date.now() / 1000));
    const id = setInterval(() => setNowSec(Math.floor(Date.now() / 1000)), DECAY_TICK_MS);
    return () => clearInterval(id);
  }, [pageVisible]);
  // Precedence in the dot slot: busy > waiting > done. Live state always wins —
  // the ember is what the slot shows when there is nothing happening NOW.
  const doneOf = (p: Place): DoneTier =>
    activityOf(p) ? "" : doneTier(workedAt(p), nowSec);
  // One derivation for both the nav row and the Resume list — they are the same
  // signal in two places, and drifting them apart is how a dot starts lying.
  const dotClass = (p: Place) => {
    const act = activityOf(p);
    if (act) return " " + act;
    const tier = doneOf(p);
    return tier ? ` done ${tier}` : "";
  };
  const dotTitle = (p: Place) => {
    const act = activityOf(p);
    if (act === "busy") return "Claude working";
    if (act === "waiting") return "Claude needs input";
    if (!doneOf(p)) return undefined;
    const a = ago(workedAt(p));
    return a === "now" ? "Claude finished just now" : `Claude finished ${a} ago`;
  };

  // ── project config: drift + the init suggestion ──
  // NEITHER runs on the 3s poll (proposal §10). `places:changed` already triggers
  // a full list_workspace (up to 16 concurrent git calls per project); doctor on
  // top of that would put the config on the hot path §8 keeps it off. So: once on
  // load, on sheet open, after a repair, and on a slow timer.
  //
  // ⚠ The failure branch is the whole point. `cmd_doctor` exits on a guard (an
  // unreadable .worktrees.toml) BEFORE it emits any JSON, so the report arrives
  // as `code: 1, findings: [], error: "…"`. Taking that at face value would clear
  // every glyph in the project — the most-broken state decorating nothing, which
  // is the exact inversion of the signal. So a failed run keeps the last measured
  // decoration and adds an `error`; only a run that actually produced a report
  // may retire a glyph.
  const takeReport = useCallback((root: string, r: DoctorReport | null) => {
    setHealth((h) => {
      const prev = h[root];
      if (reportFailed(r)) {
        return {
          ...h,
          [root]: {
            slugs: prev?.slugs ?? new Set<string>(),
            issues: prev?.issues ?? 0,
            error: r?.error ?? "doctor produced no report",
          },
        };
      }
      return { ...h, [root]: { slugs: driftedSlugs(r), issues: issueCount(r), error: null } };
    });
  }, []);
  // A BACKGROUND doctor failure is logged, never banner'd — the user didn't ask
  // for it, and a repo that can't be probed is already visible as a broken node.
  // The sheet surfaces its own failures in its own error area.
  const sweepDoctor = useCallback(async (root: string) => {
    try {
      takeReport(root, (await invoke<DoctorReport | null>("doctor", { repo: root, slug: null })) ?? null);
    } catch (e) {
      invoke("log_event", { level: "warn", msg: `doctor ${root}: ${String(e)}` }).catch(() => {});
    }
  }, [takeReport]);
  // The sweep's identity key: EVERY project root, sorted. It deliberately does
  // NOT carry `pv.ok` or the nav's ordering. A project that flips ok under git
  // contention, or a manual re-order, would otherwise re-run the immediate sweep
  // for all roots and restart the 5-minute timer on every flip. Only adding or
  // removing a project may do that.
  const rootsKey = useMemo(
    () => (ws?.projects ?? []).map((pv) => pv.root).sort().join("\n"),
    [ws],
  );
  // …so ok-ness is read at sweep TIME instead, through a ref. A root that is not
  // a readable repo right now is skipped rather than logged every 5 minutes.
  const okRoots = useRef<Set<string>>(new Set());
  useEffect(() => {
    okRoots.current = new Set((ws?.projects ?? []).filter((pv) => pv.ok).map((pv) => pv.root));
  }, [ws]);
  // Probes for the `.worktrees.toml` suggestion behind the "Not configured"
  // banner. Also called directly after the sheet writes a config, so the banner
  // retires immediately instead of at the next sweep.
  const probeSuggest = useCallback(async (root: string) => {
    try {
      const s = await invoke<InitSuggestion | null>("init_suggest", { repo: root });
      // Store unconditionally. Do NOT skip on an unchanged `hash` to save the
      // re-render: `suggestion_key` covers files/ports/compose/truncated and
      // deliberately NOT `exists` (it keys the DISMISSAL, so writing the config
      // must not re-suggest). A config appearing therefore keeps the same hash
      // while flipping `exists` — the one update the banner depends on.
      if (s) setSuggest((m) => ({ ...m, [root]: s }));
    } catch (e) {
      invoke("log_event", { level: "warn", msg: `init_suggest ${root}: ${String(e)}` }).catch(() => {});
    }
  }, []);
  useEffect(() => {
    const roots = rootsKey ? rootsKey.split("\n") : [];
    // A removed project must not keep its drift decoration or its suggestion
    // alive for the rest of the session — re-adding it would show stale findings.
    const live = new Set(roots);
    const prune = <T,>(m: Record<string, T>) =>
      Object.keys(m).every((k) => live.has(k))
        ? m
        : Object.fromEntries(Object.entries(m).filter(([k]) => live.has(k)));
    setHealth((m) => prune(m));
    setSuggest((m) => prune(m));
    if (roots.length === 0) return;
    // The init probe rides the doctor sweep rather than running once per root:
    // `.worktrees.toml` can appear from OUTSIDE the app (a merge, a pull, the
    // CLI's `init`), and a once-only probe leaves "Not configured" on screen for
    // the rest of the session after it does. Same cost class as doctor, same
    // 5-minute cadence, and it skips not-ok roots for free.
    const sweep = () =>
      roots.filter((r) => okRoots.current.has(r)).forEach((r) => {
        sweepDoctor(r);
        probeSuggest(r);
      });
    // Slow timer, not the poll — but it still shells out per root, so it stops
    // while the window is off screen. The guard sits BEFORE the immediate sweep
    // deliberately: this effect re-runs on the hidden edge too, and sweeping
    // there would spend git on the exact transition we're trying to go quiet on
    // — occlusion flaps (a window dragged across, a Space switch) would then
    // cost MORE than the ungated timer did. Coming back re-runs it with
    // pageVisible true, so the catch-up is still free.
    if (!pageVisible) return;
    sweep();
    const t = setInterval(sweep, 5 * 60_000);
    return () => clearInterval(t);
  }, [rootsKey, sweepDoctor, probeSuggest, pageVisible]);

  // uncaught frontend errors → app log (the "it just didn't respond" killers)
  useEffect(() => {
    const onErr = (e: ErrorEvent) =>
      invoke("log_event", { level: "error", msg: `window: ${e.message} @ ${e.filename}:${e.lineno}` }).catch(() => {});
    const onRej = (e: PromiseRejectionEvent) =>
      invoke("log_event", { level: "error", msg: `unhandledrejection: ${String(e.reason)}` }).catch(() => {});
    window.addEventListener("error", onErr);
    window.addEventListener("unhandledrejection", onRej);
    return () => {
      window.removeEventListener("error", onErr);
      window.removeEventListener("unhandledrejection", onRej);
    };
  }, []);

  // update check: once shortly after launch (delayed off the startup path);
  // manual re-check from Settings. Offline → latest stays null, no nagging.
  const checkUpdate = useCallback(async () => {
    try { setUpd(await invoke<UpdateInfo>("check_update")); } catch { /* offline */ }
  }, []);
  // Gate on the setting AND keep it in the deps: a timer scheduled pre-hydration
  // (under DEFAULTS, update_auto_check=true) is CANCELLED by this effect's cleanup
  // when hydration lands with the toggle off — re-running the flag INSIDE the
  // callback would already have fired. Manual "Check for updates" is unaffected.
  useEffect(() => {
    if (!settings.update_auto_check) return;
    const t = setTimeout(checkUpdate, 3000);
    return () => clearTimeout(t);
  }, [checkUpdate, settings.update_auto_check]);

  // Push the auto-fetch cadence to the backend watcher (which owns the pass loop
  // but can't read the opaque settings blob). Fires post-hydration and on every
  // change of the setting; before hydration `settings` still holds DEFAULTS
  // (fetch_interval_min=0 → off), so an early push is a harmless no-op. Idempotent
  // (a plain store), so re-syncing the same value costs nothing.
  useEffect(() => {
    invoke("set_fetch_interval", { mins: settings.fetch_interval_min }).catch(() => {});
  }, [settings.fetch_interval_min]);

  // hydrate persisted settings BEFORE first meaningful paint. A pre-hydration
  // interaction (⌘B at launch) must neither be visually reverted nor let its
  // debounced save write a DEFAULTS-seeded object over the on-disk settings —
  // so: record early patches, merge them over the loaded state, and gate every
  // save on hydration having landed.
  const hydrated = useRef(false);
  const preHydration = useRef<Partial<Settings>>({});
  useLayoutEffect(() => {
    (async () => {
      const s = await loadSettings();
      const merged = { ...s, ...preHydration.current };
      hydrated.current = true;
      applySettings(merged);
      setSettings(merged);
      if (Object.keys(preHydration.current).length > 0) saveSettings(merged);
      setLens(merged.lens);
      setCollapsed(merged.collapsed ?? {});
      // release notes: embedded CHANGELOG vs last-seen version. Fresh install
      // (no last_seen) records silently; a version CHANGE shows What's-new.
      try {
        const ci = await invoke<{ version: string; changelog: string }>("get_changelog");
        if (!merged.last_seen_version) {
          const next = { ...merged, last_seen_version: ci.version };
          setSettings(next);
          saveSettings(next);
        } else if (merged.last_seen_version !== ci.version) {
          const notes = changelogBetween(ci.changelog, merged.last_seen_version, ci.version);
          if (notes) setWhatsNew({ version: ci.version, notes });
          else {
            const next = { ...merged, last_seen_version: ci.version };
            setSettings(next);
            saveSettings(next);
          }
        }
      } catch { /* harness / older backend */ }
    })();
  }, []);

  const updateSettings = (patch: Partial<Settings>) => {
    setSettings((prev) => {
      // resizing a hidden nav is dead UI — bring it back for live preview
      const auto = patch.nav_width !== undefined && prev.nav_collapsed ? { nav_collapsed: false } : null;
      const next = { ...prev, ...patch, ...auto };
      applySettings(next);
      if (hydrated.current) saveSettings(next);
      else Object.assign(preHydration.current, patch, auto);
      return next;
    });
    // theme changes the terminal colors too (xterm reads CSS vars once per version)
    if (patch.term_family !== undefined || patch.term_size !== undefined || patch.theme !== undefined ||
        patch.theme_light !== undefined || patch.theme_dark !== undefined)
      setTermVersion((v) => v + 1);
  };

  // Dock terminal tab names live in ui-state.json under `repo|slug` → index →
  // name (name === null deletes). Empty buckets are pruned so a place you never
  // named leaves no trace in the settings file.
  const renameTermTab = (repo: string, slug: string, index: number, name: string | null) => {
    const key = repo + "|" + slug;
    const all = { ...(settings.term_tab_names ?? {}) };
    const bucket = { ...(all[key] ?? {}) };
    if (name) bucket[index] = name;
    else delete bucket[index];
    if (Object.keys(bucket).length) all[key] = bucket;
    else delete all[key];
    updateSettings({ term_tab_names: all });
  };

  // theme "system": re-apply when macOS appearance flips
  useEffect(() => {
    if (settings.theme !== "system") return;
    const mq = window.matchMedia("(prefers-color-scheme: light)");
    const onFlip = () => {
      applySettings(settings);
      setTermVersion((v) => v + 1);
    };
    mq.addEventListener("change", onFlip);
    return () => mq.removeEventListener("change", onFlip);
  }, [settings]);

  const selected: Place | null =
    (sel && ws?.projects.find((p) => p.root === sel.repo)?.snapshot?.places.find((pl) => pl.slug === sel.slug)) || null;

  // ── column fitting ──
  // Track the viewport so the side panels re-fit on every resize (and on a
  // restore into a window smaller than the one the widths were saved from —
  // that mismatch is what produced the overlapping topbar after a restart).
  const [vw, setVw] = useState(viewportWidth);
  useEffect(() => {
    let raf = 0;
    const onWinResize = () => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => setVw(viewportWidth()));
    };
    window.addEventListener("resize", onWinResize);
    return () => { window.removeEventListener("resize", onWinResize); cancelAnimationFrame(raf); };
  }, []);

  // Dock only makes sense with a place selected (Files/Terminal need a worktree).
  const dockEligible = !!selected && !!sel;
  const fit = fitLayout(settings, dockEligible, vw);
  const dockShown = fit.dockShown;
  // Would the dock fit if it were open? Drives the toggle's disabled state, so a
  // ⌘J that can't visibly do anything is at least honest about why.
  const dockFits = fitLayout({ ...settings, dock_open: true }, dockEligible, vw).dockShown;

  useLayoutEffect(() => {
    const root = document.documentElement.style;
    root.setProperty("--nav-w", `${fit.navW}px`);
    root.setProperty("--dock-w", `${fit.dockW}px`);
  }, [fit.navW, fit.dockW]);

  // ctx target derived live from ws (a refresh while the menu is open must not go stale)
  const ctxPlace: Place | null =
    ctx?.kind === "place"
      ? ws?.projects.find((v) => v.root === ctx.repo)?.snapshot?.places.find((pl) => pl.slug === ctx.slug) ?? null
      : null;

  // if a refresh removes the ctx target, finalize the close (else a stale ctx
  // resurrects the menu — with its armed remove — when the target reappears)
  useEffect(() => {
    if (!ctx || !ws) return;
    // BOTH branches must clear the arm (closeCtx's contract, inlined here to keep
    // this effect's deps stable): a vanished target must never leave confirmRm
    // armed — for the project branch that would resurrect a one-click remove.
    if (ctx.kind === "place" && !ctxPlace) { setCtx(null); setConfirmRm(null); }
    if (ctx.kind === "project" && !ws.projects.some((v) => v.root === ctx.root)) { setCtx(null); setConfirmRm(null); }
  }, [ctx, ctxPlace, ws]);

  const mutate = async (p: Promise<unknown>) => {
    try { await p; await refresh(); } catch (e) { fail(e); }
  };
  // Returns the op's CmdResult (so callers can read result.slug), or null when
  // the invoke itself threw. On a non-ok result the error banner is set here —
  // and it OUTLIVES this call: nothing clears it until the user dismisses it or
  // the next command starts.
  const runCmd = async (name: string, args: Record<string, unknown>): Promise<CmdResult | null> => {
    try {
      // A command takes ownership of both banners for its whole run — including
      // refresh's claim on the error one, so a stale message cannot be retracted
      // out from under this op's own report.
      refreshErr.current = "";
      setErr("");
      setNotice("");
      const r = await invoke<CmdResult>(name, args);
      // Re-read state FIRST, report SECOND. A failed op is normally a PARTIAL
      // success (`cmd_new` deliberately leaves the worktree in place when tmux
      // fails), so the list has to be re-pulled either way — but the report must
      // be the last write to the banner, or it is set and then wiped a tick later
      // by the very refresh that shows what half-happened.
      await refresh();
      // EXIT_NEEDS_CONFIRM is not-ok but not a failure: the op stopped to ASK.
      // The caller arms a second click; a red banner would be a lie. That holds
      // even without a `needs_confirm` name — the session died while core was
      // asking about it, so the question is moot, not broken (the refresh above
      // shows the place dormant, and doClose says so, through `notice`, which
      // nothing here touches once the run has started).
      if (!r.ok && r.code !== EXIT_NEEDS_CONFIRM) setErr(r.output || `${name} failed (exit ${r.code})`);
      // Warnings on a SUCCESSFUL op used to go nowhere but app.log. That is fine
      // for cosmetic notes and wrong for the ones that change what a session can
      // do — an AI profile skipping a skill, or a launch that could not apply the
      // profile at all. Silence there reads as "it worked", so a degraded session
      // looked identical to a good one.
      else if (r.warnings?.length) setNotice(r.warnings.join(" · "));
      return r;
    } catch (e) {
      fail(e);
      return null;
    }
  };

  // Roots whose repo has no commits yet — the new-worktree form is useless there.
  const unbornProjects = useMemo(
    () => new Set((ws?.projects ?? []).filter((p) => p.snapshot?.unborn).map((p) => p.root)),
    [ws],
  );
  const addProject = async () => {
    try {
      const dir = await open({ directory: true, title: "Add a git project" });
      if (typeof dir !== "string") return;
      setErr("");
      // Probe BEFORE add_project: a plain folder is a normal thing to pick for a
      // new project, and the answer is an offer (`git init`), not an error.
      const probe = await invoke<DirProbe>("probe_dir", { dir });
      if (!probe.is_git) {
        setInitAsk(dir);
        if (settings.nav_collapsed) toggleNav();
        return;
      }
      commitWs(await invoke<Workspace>("add_project", { dir }));
    } catch (e) { fail(e); }
  };
  const initRepo = async (dir: string) => {
    try {
      setErr("");
      commitWs(await invoke<Workspace>("init_repo", { dir }));
      setInitAsk(null);
    } catch (e) { fail(e); }
  };
  const createInitialCommit = async (repo: string) => {
    try {
      setErr("");
      commitWs(await invoke<Workspace>("create_initial_commit", { repo }));
    } catch (e) { fail(e); }
  };
  const removeProject = async (root: string) => {
    try {
      commitWs(await invoke<Workspace>("remove_project", { root }));
      if (sel?.repo === root) setSel(null);
    } catch (e) { fail(e); }
  };
  // Two-click arm for the project ctx-menu item (shares confirmRm with the
  // place-remove; key namespaced "proj|" so it never collides).
  const removeProjectCtx = (root: string) => {
    const key = `proj|${root}`;
    if (confirmRm !== key) { setConfirmRm(key); return; } // arm; menu stays open
    closeCtx();
    removeProject(root);
  };
  // Two-click arm for the project-header ✕ (key namespaced "hdr|"). Unlike the
  // popover surfaces there's no dismissal path to clear the arm, so it also
  // auto-disarms after a few seconds (effect below) and on selection change.
  const removeProjectHdr = (root: string) => {
    const key = `hdr|${root}`;
    if (confirmRm !== key) { setConfirmRm(key); return; }
    setConfirmRm(null);
    removeProject(root);
  };
  // Auto-disarm the TIMED arms: the project-header ✕ ("hdr|") and both Close
  // arms ("close|" bare, "closectx|" in the menu). The first two have no popover
  // to dismiss them at all; the ctx one does, but a menu can sit open for hours
  // and an armed whole-session kill must expire like any other offer. The
  // remaining popover arms ("proj|", `repo|slug`) are cleared by their own close
  // paths.
  // `closeSess` is a dep so a re-arm that names a DIFFERENT session (the one
  // that took the place while the first arm sat) gets a full window of its own
  // rather than the remainder of the window raised for the session that is gone.
  useEffect(() => {
    if (!isBareArm(confirmRm)) return;
    const t = setTimeout(() => setConfirmRm(null), 4000);
    return () => clearTimeout(t);
  }, [confirmRm, closeSess]);
  // Clear any bare arm when the selection changes (a switch of focus means the
  // destructive intent is stale).
  useEffect(() => {
    setConfirmRm((c) => (isBareArm(c) ? null : c));
  }, [sel]);

  // Every ctx-menu dismissal goes through here: an armed confirmRm must NEVER
  // survive the menu (it would leak into the topbar ⋯ popover as one-click remove).
  const closeCtx = () => {
    setCtx(null);
    setConfirmRm(null);
  };

  // Same guarantee for the topbar Lifecycle/⋯ popover: dismissing it must also
  // disarm the ⋯ "Remove worktree…" confirm — otherwise a stray arm survives and
  // reopening ⋯ shows a pre-armed one-click destructive remove.
  const closeMenu = () => {
    setMenu(null);
    setConfirmRm(null);
  };

  // THE primary verb: inhabit a place — stamp recency, ensure its session, select it.
  // `fresh` skips the AI auto-resume. Explicit opts.fresh (right-click override)
  // wins; otherwise the ai_auto_resume setting decides — OFF → default opens are
  // fresh. (Backend no-ops resume unless the AI command is claude anyway.)
  const enterPlace = (repo: string, p: Place, opts?: { fresh?: boolean }) => {
    setSel({ repo, slug: p.slug });
    setMenu(null);
    closeCtx();
    setTermFocus((v) => v + 1); // hand the keyboard back to the terminal
    const fresh = opts?.fresh ?? !settings.ai_auto_resume;
    (async () => {
      invoke("touch_place", { repo, slug: p.slug }).catch(() => {}); // fire-and-forget recency stamp
      await runCmd("open_place", { repo, slug: p.slug, fresh });
    })();
  };

  // ── close ──
  // One click when the session is this tool's own (canonical name — no doubt
  // whose it is). When core reports the session was ADOPTED (found by pane cwd:
  // started by hand, or left behind by a prefix change) it answers
  // needs_confirm instead of killing, and we arm a second click — the same
  // shape as the remove confirms, sharing `confirmRm` so every dismissal path
  // disarms it. `armed` is the user's word, forwarded as `yes`.
  //
  // The armed click sends BACK the session the arm displayed: consent is bound
  // to that name, and core kills nothing else. Between arming and clicking the
  // named session can exit and another can adopt the place — then core answers
  // with a fresh needs_confirm naming the newcomer, which we re-arm and SAY, so
  // the click reads as "the session changed" rather than as a dud.
  const doClose = async (repo: string, slug: string, key: string, armed: boolean) => {
    const expect = armed ? closeSess : "";
    const r = await runCmd("close_place", { repo, slug, yes: armed, session: expect || null });
    if (r?.needs_confirm) {
      if (expect && r.needs_confirm !== expect)
        setNotice(`${expect} is gone — ${r.needs_confirm} is in this place now. Nothing was killed.`);
      setCloseSess(r.needs_confirm);
      setConfirmRm(key); // arm; any open menu stays open
      return false;
    }
    // Code 4 with no session to name: it died while core was asking about it.
    // Nothing to close any more — the refresh runCmd already did shows that.
    if (r?.code === EXIT_NEEDS_CONFIRM) setNotice(`${expect || "That session"} is already gone — nothing to close.`);
    setConfirmRm((c) => (c === key ? null : c));
    return true;
  };
  // Topbar Close (bare control, "close|" key — auto-disarms, see isBareArm).
  const closeTopbar = (repo: string, slug: string, armed: boolean) =>
    doClose(repo, slug, closeKey(repo, slug), armed);
  // ── context-menu verbs ──
  // ctx-menu "Close session": same two-click arm, popover key. On the arming
  // click the menu STAYS open (removeProjectCtx's shape); it closes once the
  // session is actually gone.
  const closeSession = async (repo: string, slug: string) => {
    const key = closeCtxKey(repo, slug);
    if (await doClose(repo, slug, key, confirmRm === key)) closeCtx();
  };
  const copyText = (text: string) => {
    closeCtx();
    closeMenu(); // also dismiss the topbar ⋯ popover (its "Copy path" routes here)
    if (!navigator.clipboard) { fail("clipboard unavailable"); return; }
    navigator.clipboard.writeText(text).catch(fail);
  };
  const openOnRemote = async (repo: string, slug: string) => {
    closeCtx();
    try {
      const url = await invoke<string | null>("github_url", { repo, slug });
      if (url) await openUrl(url);
      else setErr("No origin remote for this project.");
    } catch (e) { fail(e); }
  };
  const revealPlace = (path: string) => {
    closeCtx();
    revealItemInDir(path).catch((e) => fail(e));
  };
  // Settings → "Release notes": the What's-new sheet again, with the FULL
  // released history (every section ≤ the running version), on top of Settings.
  const showReleaseNotes = async () => {
    try {
      const ci = await invoke<{ version: string; changelog: string }>("get_changelog");
      const notes = changelogBetween(ci.changelog, "0", ci.version);
      setWhatsNew({ version: ci.version, notes: notes || ci.changelog, manual: true });
    } catch (e) { fail(e); }
  };
  const editIn = (path: string) => {
    closeCtx();
    invoke("open_editor", { path, cmd: settings.editor_cmd }).catch((e) => fail(e));
  };
  // §9's banner, dismissed for this project until its suggestion CHANGES (the
  // value is the suggestion's content hash, never a boolean — see settings.ts).
  const dismissInit = (root: string, hash: string) => {
    updateSettings({ init_dismissed: { ...(settings.init_dismissed ?? {}), [root]: hash } });
  };
  // Settings → Data → "Reset to defaults". Preserve last_seen_version so the
  // What's-new sheet doesn't re-fire, push through the same apply/persist/refit
  // path as every other change, and reset the React state that MIRRORS settings
  // (lens + per-project collapsed) so the UI doesn't show stale nav state.
  //
  // ⚠ This DOES clear `init_dismissed`, so every dismissed init banner comes
  // back. That is the right behavior and is deliberate: "reset to defaults" is
  // asked for when the UI is in an unknown state, the banner is a suggestion
  // rather than a destructive action, and each one is one click to dismiss
  // again. `last_seen_version` is the only exception because re-showing What's
  // new for a version you already read is noise with no corresponding signal.
  const onReset = () => {
    const next = { ...DEFAULTS, last_seen_version: settings.last_seen_version };
    updateSettings(next);
    setLens(next.lens);
    setCollapsed(next.collapsed);
  };
  const editNote = (repo: string, p: Place) => {
    closeCtx();
    setSel({ repo, slug: p.slug });
    setTimeout(() => document.querySelector<HTMLInputElement>(".note-strip")?.focus(), 60);
  };
  // the form lives in the nav — a ctx-menu path must first bring the nav back
  const openNewForm = (root: string, base: string) => {
    setNewFor(root);
    setNewBase(base);
    if (settings.nav_collapsed) toggleNav();
  };
  const newFromBranch = (root: string, base: string) => {
    closeCtx();
    openNewForm(root, base);
  };
  // Unarmed click arms (menu stays open, showing the two danger buttons below).
  const armRemovePlaceCtx = (repo: string, slug: string) => {
    setConfirmRm(`ctx|${repo}|${slug}`); // namespaced: never matches the topbar's key
  };
  // Armed confirm. `delBranch` picks the button: false = remove only, true =
  // remove + branch. With force:false the core uses `git branch -d`, so only a
  // MERGED branch is deleted; an unmerged one degrades to a warning while the
  // remove still succeeds — del_branch is safe by construction.
  const confirmRemovePlaceCtx = async (repo: string, slug: string, delBranch: boolean) => {
    closeCtx();
    // Arg keys are camelCase: Tauri renames Rust snake_case params over IPC.
    if ((await runCmd("remove_place", { repo, slug, delBranch, force: false }))?.ok) {
      if (sel?.repo === repo && sel?.slug === slug) setSel(null);
    }
  };
  const placeCtx = (e: React.MouseEvent, repo: string, p: Place) => {
    e.preventDefault();
    e.stopPropagation();
    setMenu(null);
    setConfirmRm(null);
    setCtx({ kind: "place", x: e.clientX, y: e.clientY, repo, slug: p.slug });
  };
  const projectCtx = (e: React.MouseEvent, root: string) => {
    e.preventDefault();
    e.stopPropagation();
    setMenu(null);
    setConfirmRm(null);
    setCtx({ kind: "project", x: e.clientX, y: e.clientY, root });
  };

  // ── nav sorting (Settings-persisted; Manual = drag) ──
  const sortPlaces = (repo: string, arr: Place[]): Place[] => {
    const out = [...arr];
    if (settings.sort_mode === "manual") {
      const order = settings.manual_order[repo] ?? [];
      const idx = (p: Place) => { const i = order.indexOf(p.slug); return i < 0 ? order.length : i; };
      out.sort((a, b) => idx(a) - idx(b));
      return out;
    }
    if (settings.sort_mode === "alpha") out.sort((a, b) => a.slug.localeCompare(b.slug));
    else out.sort((a, b) => activityAt(b) - activityAt(a));
    const flip = settings.sort_mode === "alpha" ? settings.sort_dir === "desc" : settings.sort_dir === "asc";
    if (flip) out.reverse();
    return out;
  };
  const dropOnRow = (repo: string, targetSlug: string) => {
    if (!drag || drag.repo !== repo || drag.slug === targetSlug) { setDrag(null); setDragOver(null); return; }
    const pv = ws?.projects.find((v) => v.root === repo);
    const slugs = (pv?.snapshot?.places ?? []).filter((p) => !p.is_main).map((p) => p.slug);
    const existing = settings.manual_order[repo] ?? [];
    const order = [...existing.filter((x) => slugs.includes(x)), ...slugs.filter((x) => !existing.includes(x))];
    const from = order.indexOf(drag.slug);
    if (from >= 0) order.splice(from, 1);
    const to = order.indexOf(targetSlug);
    order.splice(to < 0 ? order.length : to, 0, drag.slug);
    updateSettings({ manual_order: { ...settings.manual_order, [repo]: order } });
    setDrag(null);
    setDragOver(null);
  };

  const createPlace = async (repo: string, branch: string, name: string, base: string) => {
    if (!branch) return;
    // Dismiss the form and put a ghost row in the nav IMMEDIATELY — the click is
    // acknowledged before any of the work starts. The label is the same
    // derivation the old success path used as its fallback; core may land on a
    // different slug (origin/ stripped, holder reuse), which is why the ghost is
    // retired by `id` and the selection below still uses core's answer.
    const id = ++pendingSeq.current;
    const label = (name || branch).replace(/\//g, "-");
    setPendingNew((v) => [...v, { id, repo, label }]);
    setNewFor(null);
    setNewBase("");
    setNewDraft(null);
    try {
      const r = await runCmd("new_place", { repo, branch, base: base || null, name: name || null });
      // Select core's ACTUAL final slug (origin/ stripped, holder-reuse applied);
      // fall back to the old derivation only if the backend didn't report one.
      if (r?.ok) setSel({ repo, slug: r.slug ?? label });
      // A rejected create is usually a fixable typo (a base that does not exist,
      // a branch checked out in another worktree). Put the form back with what
      // was typed still in it — dismissing on submit must not also mean losing
      // the input. `runCmd` owns the banner, so this only restores the fields.
      else if (r) { setNewDraft({ branch, name, base }); setNewBase(base); setNewFor(repo); }
    } finally {
      // `runCmd` awaits its own refresh() before returning, so the real row is
      // already in `ws` by now — the ghost hands over with no empty frame
      // between them. `finally`: a failed create must not leave a row spinning
      // forever (runCmd has already put the reason in the error banner).
      setPendingNew((v) => v.filter((x) => x.id !== id));
    }
  };
  // The switcher owns its own text now; this just runs the op and reports.
  const doSwitch = async (branch: string) => {
    if (!sel) return false;
    const b = branch.trim();
    if (!b) return false;
    return !!(await runCmd("switch_place", { repo: sel.repo, slug: sel.slug, branch: b, base: null }))?.ok;
  };
  // Topbar ⋯ remove — same armed two-button pair as the ctx menu (arm key
  // `repo|slug`). Unarmed click arms; the armed state renders "Confirm remove"
  // (del_branch:false) + "Confirm remove + branch" (del_branch:true).
  const armRemove = () => {
    if (!sel) return;
    setConfirmRm(`${sel.repo}|${sel.slug}`);
  };
  const confirmRemove = async (delBranch: boolean) => {
    if (!sel) return;
    closeMenu();
    if ((await runCmd("remove_place", { repo: sel.repo, slug: sel.slug, delBranch, force: false }))?.ok) setSel(null);
  };

  const toggleProject = (root: string) => {
    setCollapsed((c) => {
      const next = { ...c, [root]: !c[root] };
      updateSettings({ collapsed: next });
      return next;
    });
  };
  const isOpen = (key: string, def: boolean) => groupOpen[key] ?? def;
  const toggleGroup = (key: string, def: boolean) =>
    setGroupOpen((g) => ({ ...g, [key]: !(g[key] ?? def) }));

  // rail-only mode: hide the nav column, keep the rail (persisted in ui-state)
  const toggleNav = useCallback(() => {
    setSettings((prev) => {
      const next = { ...prev, nav_collapsed: !prev.nav_collapsed };
      applySettings(next);
      if (hydrated.current) saveSettings(next);
      else preHydration.current.nav_collapsed = next.nav_collapsed;
      return next;
    });
  }, []);
  // Right rail → dock tab. Clicking the ACTIVE tab collapses the dock (VS Code's
  // model), which is why the topbar no longer carries its own dock toggle.
  const pickDockTab = (tab: Settings["dock_tab"]) =>
    updateSettings(
      settings.dock_open && settings.dock_tab === tab
        ? { dock_open: false }
        : { dock_tab: tab, dock_open: true },
    );
  // right dock (Files / Terminal) — only renders with a place selected, but the
  // preference persists globally (stable useCallback: safe in the keydown effect).
  const toggleDock = useCallback(() => {
    setSettings((prev) => {
      const next = { ...prev, dock_open: !prev.dock_open };
      applySettings(next);
      if (hydrated.current) saveSettings(next);
      else preHydration.current.dock_open = next.dock_open;
      return next;
    });
  }, []);
  // keyboard lens select (⌘2..N): unlike changeLens, re-selecting the active lens
  // must NOT toggle/collapse the nav — a keyboard chord always REVEALS the view.
  const selectLens = useCallback((l: Lens) => {
    setLens(l);
    setSettings((prev) => {
      if (prev.lens === l && !prev.nav_collapsed) return prev;
      const next = { ...prev, lens: l, nav_collapsed: false };
      applySettings(next);
      if (hydrated.current) saveSettings(next);
      else Object.assign(preHydration.current, { lens: l, nav_collapsed: false });
      return next;
    });
  }, []);
  // ⌘-digit / ⌘E read live state (selection, editor cmd) — hold it in a ref so
  // the keydown listener stays stable (registered once, no per-render churn).
  const keyRef = useRef({ selectLens, selectedPath: null as string | null, editorCmd: settings.editor_cmd, switchOpen, dockFile: null as string | null, reading: false, settingsOpen: false, filesTabOpen: false });
  keyRef.current = { selectLens, selectedPath: selected?.path ?? null, editorCmd: settings.editor_cmd, switchOpen, dockFile, reading, settingsOpen, filesTabOpen: settings.dock_open && settings.dock_tab === "files" };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // ⌘⇧T — new dock terminal. Handled BEFORE the meta-only guard (it needs
      // shift). ⌘ only (not ctrl) so Ctrl+Shift+T still reaches the embedded
      // shell; swallowed while the ⌘K palette owns the keyboard. No-op unless
      // the dock's Terminal tab is mounted (it ignores the token otherwise).
      if (e.metaKey && e.shiftKey && !e.altKey && !e.repeat && e.key.toLowerCase() === "t") {
        e.preventDefault();
        if (!keyRef.current.switchOpen) setNewTermToken((v) => v + 1);
        return;
      }
      // Esc leaves reading mode — but only when reading is the TOP surface.
      // The settings sheet and the ⌘K palette are above it and own Escape
      // first, or one keypress dismisses the thing you can't see.
      if (e.key === "Escape" && keyRef.current.reading && !keyRef.current.settingsOpen && !keyRef.current.switchOpen) {
        e.preventDefault();
        setReading(false);
        return;
      }
      // ⌘⇧E — reading mode: the dock's current file expands over the main pane.
      // No-op with no file open, so the chord can't blank the terminal for
      // nothing. Also shift-guarded, hence its place above the meta-only gate.
      if (e.metaKey && e.shiftKey && !e.altKey && !e.repeat && e.key.toLowerCase() === "e") {
        e.preventDefault();
        // Only meaningful with the Files tab showing a file: opening a reader
        // for a stale path behind a modal, or over the Terminal tab, is noise.
        if (keyRef.current.switchOpen || keyRef.current.settingsOpen) return;
        setReading((v) => (v ? false : keyRef.current.filesTabOpen && !!keyRef.current.dockFile));
        return;
      }
      if (!(e.metaKey || e.ctrlKey) || e.repeat || e.shiftKey || e.altKey) return;
      const k = e.key.toLowerCase();
      // ⌘K — quick switcher. A meta chord fires PAST the embedded terminal (like
      // ⌘B/⌘,) so it opens even with the terminal focused; toggles closed if open.
      if (e.metaKey && k === "k") {
        e.preventDefault();
        setSwitchOpen((v) => !v);
        return;
      }
      // While the palette is open, swallow the OTHER app chords (⌘1-4, ⌘E, ⌘,,
      // ⌘B) — the palette owns the keyboard; only ⌘K (handled above) and Escape
      // (owned by the palette itself) do anything.
      if (keyRef.current.switchOpen) return;
      if (k === "b") {
        // Ctrl+B is the tmux prefix — let the embedded terminal keep it; ⌘B still toggles
        if (e.ctrlKey && !e.metaKey && e.target instanceof Element && e.target.closest(".term-host")) return;
        e.preventDefault();
        toggleNav();
      } else if (e.metaKey && k === "j") {
        // ⌘J — toggle the right dock (Files / Terminal). Meta chord fires past
        // the embedded terminal, like ⌘B.
        e.preventDefault();
        toggleDock();
      } else if (e.metaKey && e.key === ",") {
        // ⌘, opens Settings (macOS convention) — a meta chord is safe past the
        // term-host (its passthrough concerns are ctrl-only). Esc already closes.
        e.preventDefault();
        setSettingsOpen((v) => !v);
      } else if (e.metaKey && e.key >= "1" && e.key <= "9") {
        // ⌘1..N — jump to a nav view (Home / lenses). Always REVEALS (never
        // toggles). Meta-only chord is safe past the term-host.
        const idx = e.key.charCodeAt(0) - 49; // '1' → 0
        const target = NAV_CHORDS[idx];
        if (!target) return;
        e.preventDefault();
        if (target.home) { setSel(null); setMenu(null); closeCtx(); if (settings.nav_collapsed) toggleNav(); }
        else if (target.lens) keyRef.current.selectLens(target.lens);
      } else if (e.metaKey && k === "e") {
        // ⌘E — open the selected place in the editor (no-op when nothing selected).
        e.preventDefault();
        const path = keyRef.current.selectedPath;
        if (path) invoke("open_editor", { path, cmd: keyRef.current.editorCmd }).catch((err) => fail(err));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [toggleNav, toggleDock, fail, settings.nav_collapsed]);

  // lens click: collapsed → expand into that lens; active lens again → collapse (VS Code style)
  const changeLens = (l: Lens) => {
    if (settings.nav_collapsed) { setLens(l); updateSettings({ lens: l, nav_collapsed: false }); return; }
    if (l === lens) { toggleNav(); return; }
    setLens(l);
    updateSettings({ lens: l });
  };

  const q = filter.trim().toLowerCase();
  const matchPlace = (p: Place) =>
    !q ||
    p.slug.toLowerCase().includes(q) ||
    (p.branch ?? "").toLowerCase().includes(q) ||
    (p.declared?.note ?? "").toLowerCase().includes(q);

  // workspace-wide stats for the Briefing cockpit
  const allPlaces = useMemo(
    () => (ws?.projects ?? []).flatMap((pv) => (pv.snapshot?.places ?? []).map((p) => ({ pv, p }))),
    [ws],
  );
  const stats = useMemo(() => {
    let live = 0, dirty = 0;
    for (const { p } of allPlaces) { if (p.tmux_session.up) live++; if (p.dirty) dirty++; }
    return { live, dirty };
  }, [allPlaces]);
  const resume = useMemo(
    () => allPlaces
      .filter(({ p }) => !p.is_main)
      .sort((a, b) => usedEpoch(b.p) - usedEpoch(a.p))
      .slice(0, 6),
    [allPlaces],
  );

  // Restore last place on launch (opt-in). SELECTION-ONLY: this calls setSel and
  // nothing else — NO enterPlace/touch_place/open_place (those would auto-resume a
  // Claude session on every reboot). TerminalPane attaches on its own if the tmux
  // session is up; otherwise the place view shows its normal "Enter ▸ to start".
  // Target = the most recently USED place across all projects (max of opened /
  // worked, main excluded) — same derivation as the Resume list, so
  // resume[0]. Fires exactly once, and only after BOTH settings hydration and the
  // first workspace load have landed, only if restore_last is on, and only if the
  // user hasn't already selected something.
  const restoredOnce = useRef(false);
  useEffect(() => {
    if (restoredOnce.current) return;
    if (!hydrated.current || !ws) return; // wait for both hydration + first ws load
    restoredOnce.current = true; // launch moment passed — one-shot even with the
    // toggle off, so enabling it later can't yank the selection mid-session
    if (!settings.restore_last) return;
    if (sel) return; // user already clicked — don't override their choice
    const target = resume[0];
    if (target) setSel({ repo: target.pv.root, slug: target.p.slug });
  }, [ws, settings.restore_last, resume, sel]);

  // ── nav resizer (drag the nav's right edge) ──
  // Both resizers clamp against the LIVE viewport, so a drag can never push the
  // center pane under its floor. `window.innerWidth` is read inside the move
  // handler rather than closed over — the window can be resized mid-drag.
  const onResize = (e: React.MouseEvent) => {
    e.preventDefault();
    const startX = e.clientX;
    const startW = settings.nav_width;
    const move = (ev: MouseEvent) =>
      updateSettings({
        nav_width: clampNav(startW + (ev.clientX - startX), dockShown ? settings.dock_width : 0, window.innerWidth),
      });
    const up = () => { window.removeEventListener("mousemove", move); window.removeEventListener("mouseup", up); };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  // ── dock resizer (drag the dock's LEFT edge — moving left GROWS it) ──
  const onDockResize = (e: React.MouseEvent) => {
    e.preventDefault();
    const startX = e.clientX;
    const startW = settings.dock_width;
    const move = (ev: MouseEvent) =>
      updateSettings({
        dock_width: clampDock(startW - (ev.clientX - startX), fit.navShown ? fit.navW : 0, window.innerWidth),
      });
    const up = () => { window.removeEventListener("mousemove", move); window.removeEventListener("mouseup", up); };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  // ── row ──
  // ⚠ Defined inside App() against CLAUDE.md's own rule. It is safe ONLY because
  // it holds no local state and takes no focus — the drift glyph rides in on the
  // `drift` prop-by-closure and its tooltip is a plain `title` attribute. The
  // moment this needs useState (a hover card, an inline editor), hoist it to
  // module scope with props FIRST.
  // `ageEpoch` overrides the age clock for lists that sort by a different key —
  // a list must label rows with the SAME clock it orders by, or its ages read
  // as a broken sort (the Recent lens sorts by usedEpoch, opens included).
  const PlaceRow = ({ repo, p, showProject, ageEpoch }: { repo: string; p: Place; showProject?: boolean; ageEpoch?: number }) => {
    const divergent = !p.is_main && !p.detached && p.branch && p.branch !== p.slug;
    return (
      <li
        className={
          "row" +
          (sel?.repo === repo && sel?.slug === p.slug ? " sel" : "") +
          (dragOver === p.slug && drag?.repo === repo ? " dragover" : "")
        }
        onClick={() => enterPlace(repo, p)}
        onContextMenu={(e) => placeCtx(e, repo, p)}
        draggable={settings.sort_mode === "manual" && !p.is_main}
        onDragStart={(e) => { e.dataTransfer.effectAllowed = "move"; setDrag({ repo, slug: p.slug }); }}
        onDragOver={(e) => { if (drag && drag.repo === repo && drag.slug !== p.slug && !p.is_main) { e.preventDefault(); setDragOver(p.slug); } }}
        onDragLeave={() => { if (dragOver === p.slug) setDragOver(null); }}
        onDrop={(e) => { e.preventDefault(); dropOnRow(repo, p.slug); }}
        onDragEnd={() => { setDrag(null); setDragOver(null); }}
        title={p.slug}
      >
        <span className={"status-dot" + dotClass(p)} title={dotTitle(p)} />
        <span className="row-id">
          <span className="row-name">
            {p.is_main ? "◆ " : p.declared?.pinned ? "★ " : ""}
            {p.slug}
            {showProject ? <span className="row-proj">{basename(repo)}</span> : null}
          </span>
          {divergent ? <span className="row-branch">↗ {p.branch}</span> : null}
        </span>
        <span className="glyphs">
          {glyphs(p, health[repo]?.slugs.has(p.slug)).map((g, i) => (
            <span key={i} className={"g " + g.cls} title={g.title}>{g.text}</span>
          ))}
          <span className="row-age">{ago(ageEpoch ?? activityAt(p))}</span>
        </span>
      </li>
    );
  };

  const GroupHeader = ({ gkey, label, count, open, onToggle }: { gkey: string; label: string; count: number; open: boolean; onToggle: () => void }) => (
    <div className="group-h" key={gkey} onClick={onToggle}>
      <span className="caret">{open ? <Icons.ChevronDown size={11} /> : <Icons.ChevronRight size={11} />}</span>
      {label}
      <span className="count">{count}</span>
    </div>
  );

  const ProjectNode = ({ pv }: { pv: ProjectView }) => {
    const open = !collapsed[pv.root];
    const places = (pv.snapshot?.places ?? []).filter(matchPlace);
    const main = places.find((p) => p.is_main) ?? null;
    // In-flight creates for this project. Checked against the UNFILTERED
    // snapshot: `new` on a branch that already has a worktree reuses that one,
    // and a ghost beside the row it is about to become reads as a duplicate.
    // The nav filter is deliberately not applied — this row is the answer to a
    // click the user just made, so it shows even if the text filter excludes it.
    const ghosts = pendingNew.filter(
      (g) => g.repo === pv.root && !(pv.snapshot?.places ?? []).some((p) => p.slug === g.label),
    );
    // Rollup: a working child dominates a waiting one — busy wins, else waiting,
    // else a JUST-finished one (t1 only). Restricting the ember to the freshest
    // tier is deliberate: t2/t3 would leave a permanent badge on every project
    // anyone touched today, which is decoration, not signal.
    const rollup = places.some((p) => activityOf(p) === "busy")
      ? "busy"
      : places.some((p) => activityOf(p) === "waiting")
        ? "waiting"
        : places.some((p) => doneOf(p) === "t1")
          ? "done"
          : "";
    const buckets: Record<string, Place[]> = {};
    for (const p of places) { if (p.is_main) continue; (buckets[bucketOf(p)] ??= []).push(p); }
    for (const k of Object.keys(buckets)) buckets[k] = sortPlaces(pv.root, buckets[k]);
    const hiddenTiers = new Set(settings.hidden_tiers);
    const dormant = DORMANT_TIERS.flatMap((t) => buckets[t] ?? []);

    return (
      <div className="project">
        <div className="project-h" onContextMenu={(e) => projectCtx(e, pv.root)}>
          <span className="caret" onClick={() => toggleProject(pv.root)}>{open ? <Icons.ChevronDown size={11} /> : <Icons.ChevronRight size={11} />}</span>
          {pv.ok
            ? <span className={"picon" + (rollup ? " " + rollup : "")} title={rollup === "busy" ? "a session is working" : rollup === "waiting" ? "a session needs input" : rollup === "done" ? "a session just finished" : undefined}><FolderIcon /></span>
            : <span className="rollup broken" title="repo gone">⊘</span>}
          <span className="pname" title={pv.root} onClick={() => toggleProject(pv.root)}>{basename(pv.root)}</span>
          {pv.ok ? <span className="pcount">{places.length}</span> : <span className="pgone">repo gone</span>}
          {/* An unreadable .worktrees.toml is a PROJECT-level fact — no row can
              carry it, and the row glyphs are deliberately frozen at their last
              measurement while doctor can't run. Without this the nav's only
              honest state for "the config broke" would be silence. */}
          {pv.ok && health[pv.root]?.error ? (
            <button
              className="mini pbroken"
              title={`config unreadable — doctor could not run here:\n${health[pv.root]!.error}`}
              onClick={() => setProjSheet(pv.root)}
            >⚑</button>
          ) : null}
          <button className="mini" title="new worktree" onClick={() => { setNewFor(newFor === pv.root ? null : pv.root); setNewBase(""); setNewDraft(null); }}><Icons.Plus size={13} /></button>
          <button
            className={"mini" + (confirmRm === `hdr|${pv.root}` ? " armed" : "")}
            title={confirmRm === `hdr|${pv.root}` ? "click again to remove from workspace" : "remove project"}
            onClick={() => removeProjectHdr(pv.root)}
          >
            {confirmRm === `hdr|${pv.root}` ? "remove?" : <Icons.X size={13} />}
          </button>
        </div>

        {open && pv.ok && (
          <div className="kids">
            {(() => {
              // §9's passive nudge. Shown only for a QUALIFYING project with no
              // config, and only until its exact suggestion is dismissed.
              const sug = suggest[pv.root];
              if (!sug?.qualifies || sug.exists) return null;
              if (settings.init_dismissed?.[pv.root] === sug.hash) return null;
              return (
                <InitBanner
                  suggestion={sug}
                  onOpen={() => setProjSheet(pv.root)}
                  onDismiss={() => dismissInit(pv.root, sug.hash)}
                />
              );
            })()}
            {main && <ul className="places"><PlaceRow repo={pv.root} p={main} /></ul>}
            {/* above the tier groups, not inside one: which tier it lands in is
                not known until it exists */}
            {ghosts.length > 0 && (
              <ul className="places">{ghosts.map((g) => <PendingRow key={g.id} label={g.label} />)}</ul>
            )}
            {LIVE_TIERS.filter((g) => buckets[g]?.length && !hiddenTiers.has(g)).map((g) => {
              const key = `${pv.root}|${g}`;
              const opened = isOpen(key, g !== "idle"); // idle collapsed by default
              return (
                <div className="group" key={key}>
                  <GroupHeader gkey={key} label={GROUP_LABEL[g]} count={buckets[g].length} open={opened} onToggle={() => toggleGroup(key, g !== "idle")} />
                  {opened && <ul className="places">{buckets[g].map((p) => <PlaceRow key={p.slug} repo={pv.root} p={p} />)}</ul>}
                </div>
              );
            })}
            {dormant.length > 0 && !hiddenTiers.has("dormant") && (() => {
              const key = `${pv.root}|dormant`;
              const opened = isOpen(key, false);
              return (
                <div className="group dormant" key={key}>
                  <div className="group-h dormant-h" onClick={() => toggleGroup(key, false)}>
                    {/* same SVG caret as every other group header — the ASCII
                        ▾/▸ was one more thing making the quietest row louder */}
                    <span className="caret">{opened ? <Icons.ChevronDown size={11} /> : <Icons.ChevronRight size={11} />}</span>
                    Dormant<span className="count">{dormant.length}</span>
                  </div>
                  {opened && (
                    <div className="kids-d">
                      {DORMANT_TIERS.filter((t) => buckets[t]?.length).map((t) => (
                        <div className="subgroup" key={t}>
                          <div className="subdiv">{GROUP_LABEL[t]}</div>
                          <ul className="places">{buckets[t].map((p) => <PlaceRow key={p.slug} repo={pv.root} p={p} />)}</ul>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              );
            })()}
          </div>
        )}
      </div>
    );
  };

  // flat lens (recent / attention) across all projects
  const FlatLens = ({ items, ageOf }: { items: { pv: ProjectView; p: Place }[]; ageOf?: (p: Place) => number }) => (
    <ul className="places flat">
      {items.length === 0 && <li className="flat-empty">Nothing here.</li>}
      {items.map(({ pv, p }) => <PlaceRow key={pv.root + p.slug} repo={pv.root} p={p} showProject ageEpoch={ageOf?.(p)} />)}
    </ul>
  );

  const recentItems = useMemo(
    () => allPlaces.filter(({ p }) => matchPlace(p) && !p.is_main)
      .sort((a, b) => usedEpoch(b.p) - usedEpoch(a.p)),
    [allPlaces, q],
  );
  const attentionItems = useMemo(
    () => allPlaces.filter(({ p }) => matchPlace(p) && hasAttention(p)),
    [allPlaces, q],
  );

  const RAIL = [
    { key: "places" as Lens, icon: <Icons.ListTree size={17} />, title: "Places — the full tree" },
    { key: "recent" as Lens, icon: <Icons.History size={17} />, title: "Recent — resurface dormant places" },
    { key: "attention" as Lens, icon: <Icons.TriangleAlert size={17} />, title: "Attention — dirty / ahead-behind / broken" },
  ];
  const DOCK_RAIL = [
    { key: "files" as Settings["dock_tab"], icon: <Icons.Folder size={17} />, title: "Files" },
    { key: "terminal" as Settings["dock_tab"], icon: <Icons.SquareTerminal size={17} />, title: "Terminal" },
  ];

  // `minmax(0, 1fr)` — a bare `1fr` is `minmax(auto, 1fr)`, which refuses to
  // shrink below the center pane's content and pushes the fixed columns off the
  // window instead of letting anything ellipsise.
  const gridCols = [
    "var(--rail-w)",
    fit.navShown ? `${fit.navW}px` : null,
    "minmax(0, 1fr)",
    dockShown ? `${fit.dockW}px` : null,
    "var(--rail-w)", // right rail — permanent, like the left one
  ].filter(Boolean).join(" ");

  return (
    <div className="app" style={{ gridTemplateColumns: gridCols }}>
      {/* ── activity rail ── */}
      <nav className="rail">
        {RAIL.map((r) => (
          <button
            key={r.key}
            className={"rail-icon" + (lens === r.key ? (settings.nav_collapsed ? " remembered" : " active") : "")}
            title={r.title}
            onClick={() => changeLens(r.key)}
          >
            {r.icon}
          </button>
        ))}
        <div className="rail-spacer" />
        <button className="rail-icon" title={settings.nav_collapsed ? `show nav — ${lens} (⌘B)` : "hide nav (⌘B)"} onClick={toggleNav}>
          {settings.nav_collapsed ? <Icons.PanelLeftOpen size={17} /> : <Icons.PanelLeftClose size={17} />}
        </button>
        <button className="rail-icon" title="add project" onClick={addProject}><Icons.FolderPlus size={17} /></button>
        <button className={"rail-icon" + (updateAvail ? " upd" : "")} title={updateAvail ? "settings — update available" : "settings (⌘,)"} onClick={() => setSettingsOpen(true)}><Icons.Settings size={17} /></button>
      </nav>

      {/* ── nav (kept mounted while collapsed so form drafts / scroll survive ⌘B) ── */}
      <aside className={"nav" + (fit.navShown ? "" : " hidden")}>
        <button className={"home-item" + (sel ? "" : " on")} onClick={() => { setSel(null); setMenu(null); closeCtx(); }}>
          <HomeIcon /> Home
        </button>
        <div className="nav-head">
          <span className="nav-title">{lens === "places" ? "PLACES" : lens === "recent" ? "RECENT" : "ATTENTION"}</span>
          <div className="menu-wrap">
            <button className={"icon-btn" + (settings.sort_mode !== "recent" ? " on" : "")} title="sort places" onClick={() => setSortOpen(!sortOpen)}><Icons.ArrowUpDown /></button>
            {sortOpen && (
              <div className="popover sortpop">
                <div className="pop-hint">sort places</div>
                {([["recent", "Activity"], ["alpha", "A–Z"], ["manual", "Manual (drag rows)"]] as const).map(([m, label]) => (
                  <button key={m} className="pop-item"
                    onClick={() => updateSettings({ sort_mode: m, sort_dir: m === "alpha" ? "asc" : "desc" })}>
                    <span className="check">{settings.sort_mode === m && <Icons.Check size={12} />}</span>{label}
                  </button>
                ))}
                <div className="ctx-sep" />
                <button className="pop-item" disabled={settings.sort_mode === "manual"}
                  onClick={() => updateSettings({ sort_dir: settings.sort_dir === "desc" ? "asc" : "desc" })}>
                  <span className="check" />{settings.sort_dir === "desc" ? "↓ descending" : "↑ ascending"}
                </button>
              </div>
            )}
          </div>
          <button className="icon-btn" title="focus search" onClick={() => searchRef.current?.focus()}><Icons.Search /></button>
        </div>
        <input ref={searchRef} className="search" placeholder="filter places…" value={filter} onChange={(e) => setFilter(e.currentTarget.value)} />
        {initAsk && (
          <InitRepoPrompt dir={initAsk} onInit={initRepo} onCancel={() => setInitAsk(null)} />
        )}
        {newFor && (
          unbornProjects.has(newFor) ? (
            <UnbornPrompt
              project={newFor}
              onCommit={createInitialCommit}
              onCancel={() => { setNewFor(null); setNewBase(""); setNewDraft(null); }}
            />
          ) : (
            <NewPlaceForm
              // the draft is part of the identity: a rejected create reopens the
              // form for the same project, and without it in the key React would
              // reuse the old instance and its now-stale field state
              key={newFor + "|" + newBase + "|" + (newDraft?.branch ?? "")}
              project={newFor}
              initialBranch={newDraft?.branch ?? ""}
              initialName={newDraft?.name ?? ""}
              initialBase={newDraft?.base ?? newBase}
              onCreate={(b, n, ba) => createPlace(newFor, b, n, ba)}
              onCancel={() => { setNewFor(null); setNewBase(""); setNewDraft(null); }}
            />
          )
        )}
        <div className="nav-scroll">
          {ws && ws.projects.length === 0 && <div className="empty small">No projects yet.<br />Add one from the rail.</div>}
          {lens === "places" && ws?.projects.map((pv) => <ProjectNode key={pv.root} pv={pv} />)}
          {lens === "recent" && <FlatLens items={recentItems} ageOf={usedEpoch} />}
          {lens === "attention" && (
            <>
              <FlatLens items={attentionItems} />
              {ws?.projects.filter((pv) => !pv.ok).map((pv) => (
                <div className="project broken-flat" key={pv.root}><span className="rollup broken">⊘</span> {basename(pv.root)} <span className="pgone">repo gone</span></div>
              ))}
            </>
          )}
        </div>
        {/* Claude plan usage — nav-only by design: no rail affordance, it rides
            out ⌘B inside the hidden nav (and keeps polling, one call/180s). */}
        <UsageWidget onError={fail} />
        <button className="add-footer with-icon" onClick={addProject}><Icons.Plus size={13} /> Add project</button>
        <div className="nav-resizer" onMouseDown={onResize} />
      </aside>

      {/* ── main ── */}
      <main className="main">
        {!tmuxOk && <TmuxBanner onRecheck={recheckTmux} />}
        {selected && sel ? (
          <>
            <header className="topbar">
              <div className="identity">
                <b className="slug">{selected.is_main ? "◆ " : ""}{selected.slug}</b>
                {selected.branch && (
                  <span className={"branch" + (!selected.is_main && selected.branch !== selected.slug ? " hi" : "")}>
                    {!selected.is_main && selected.branch !== selected.slug ? "↗ " : ""}{selected.branch}
                  </span>
                )}
                {/* squeezed window: the badges go, not the name — every fact
                    here is also in the nav row and the status bar */}
                {!fit.tight && (
                  <span className="status-cluster">
                    {selected.tmux_session.up && <span className="s ok" title="tmux live"><span className="status-dot on" /> live</span>}
                    {selected.dirty && <span className="s dirty">● {selected.dirty_files ?? ""}</span>}
                    {(selected.ahead || selected.behind) && <span className="s ab" title="vs the base branch">↑{selected.ahead ?? 0} ↓{selected.behind ?? 0}</span>}
                    <span className={"life " + selected.lifecycle_effective}>{selected.lifecycle_effective}</span>
                  </span>
                )}
              </div>

              <div className="controls">
                {/* the dock toggle moved to the right rail — it lives next to
                    the thing it opens, and no longer collides with the lens ▤ */}
                {selected.tmux_session.up ? (
                  <>
                    <span className="live-badge" title="session live"><span className="status-dot on" /> live</span>
                    {/* Which profile this LIVE session is actually running, and
                        whether it has drifted from the profile as edited since.
                        Deliberately says "rules/model/MCP": skills are symlinked
                        and reach a running session already, so claiming the badge
                        covers them would be a lie. */}
                    {selected.profile_name || selected.profile_stale ? (
                      <span
                        className={"live-badge profile-badge" + (selected.profile_stale ? " stale" : "")}
                        title={
                          !selected.profile_stale
                            ? `AI profile: ${selected.profile_name}`
                            : selected.profile_name
                              ? `This session started with an older version of “${selected.profile_name}”. Restart it to apply your rules/model/MCP edits (skill edits already apply).`
                              // Started before a profile was bound to this repo:
                              // the backend flags it stale with no name, and the
                              // badge used to render nothing at all for it.
                              : "This session started before a profile was bound to this repo. Restart it to apply that profile."
                        }
                      >
                        {selected.profile_name ?? "unprofiled"}
                        {selected.profile_stale ? " · restart to apply" : ""}
                      </span>
                    ) : null}
                    {confirmRm === closeKey(sel.repo, sel.slug) ? (
                      <button className="ctrl danger armed"
                        title={`${closeSess} was adopted, not opened under this repo's name — killing it takes the WHOLE session, every window and pane in it`}
                        onClick={() => closeTopbar(sel.repo, sel.slug, true)}>Kill {closeSess} — whole session?</button>
                    ) : (
                      <button className="ctrl" title="end the tmux session — the worktree stays"
                        onClick={() => closeTopbar(sel.repo, sel.slug, false)}>Close</button>
                    )}
                  </>
                ) : (
                  <button className="enter-btn with-icon" onClick={() => enterPlace(sel.repo, selected)}>Enter <Icons.ChevronRight size={13} /></button>
                )}
                <button className={"icon-btn" + (selected.declared?.pinned ? " on" : "")} title={selected.declared?.pinned ? "unpin" : "pin"}
                  onClick={() => mutate(invoke("set_pin", { repo: sel.repo, slug: sel.slug, on: !selected.declared?.pinned }))}>
                  <Icons.Pin filled={!!selected.declared?.pinned} />
                </button>

                <div className="menu-wrap">
                  <button className="ctrl with-icon" onClick={() => (menu === "life" ? closeMenu() : (setConfirmRm(null), setMenu("life")))}>
                    Lifecycle <Icons.ChevronDown size={13} />
                  </button>
                  {menu === "life" && (
                    <div className="popover right">
                      <div className="pop-hint">active / idle are derived</div>
                      {SETTABLE.map((s) => (
                        <button key={s.value} className="pop-item" onClick={() => { mutate(invoke("set_lifecycle", { repo: sel.repo, slug: sel.slug, label: s.value })); closeMenu(); }}>
                          <span className="check">{selected.declared?.lifecycle === s.value && <Icons.Check size={12} />}</span>{s.label}
                        </button>
                      ))}
                    </div>
                  )}
                </div>

                {!selected.is_main && (
                  <div className="menu-wrap">
                    <button className="ctrl icon-only" title="more actions" onClick={() => (menu === "more" ? closeMenu() : (setConfirmRm(null), setMenu("more")))}><Icons.Ellipsis /></button>
                    {menu === "more" && (
                      <div className="popover right">
                        <button className="pop-item" onClick={() => copyText(selected.path)}>Copy path</button>
                        {confirmRm === `${sel.repo}|${sel.slug}` ? (
                          <>
                            <button className="pop-item danger armed" onClick={() => confirmRemove(false)}>Confirm remove</button>
                            <button className="pop-item danger armed" onClick={() => confirmRemove(true)}>Confirm remove + branch</button>
                          </>
                        ) : (
                          <button className="pop-item danger" onClick={armRemove}>Remove worktree…</button>
                        )}
                      </div>
                    )}
                  </div>
                )}
              </div>
            </header>

            <input className="note-strip" placeholder="note…" defaultValue={selected.declared?.note ?? ""}
              key={sel.repo + sel.slug + (selected.declared?.note ?? "")}
              onBlur={(e) => mutate(invoke("set_note", { repo: sel.repo, slug: sel.slug, note: e.currentTarget.value }))} />

            {selected.tmux_session.up ? (
              <TerminalPane key={selected.tmux_session.name} session={selected.tmux_session.name} termVersion={termVersion} focusToken={termFocus} />
            ) : (
              <div className="term-empty">
                <div className="term-empty-card">
                  <div className="te-title">No live session for <b>{selected.slug}</b></div>
                  <button className="enter-btn big with-icon" onClick={() => enterPlace(sel.repo, selected)}>Enter <Icons.ChevronRight size={13} /> to start</button>
                </div>
              </div>
            )}

            <footer className="statusbar">
              <div className="switch-wrap">
                {!selected.is_main && (
                  <>
                    <span className="sb-label" title={`on ${selected.branch ?? "?"}`}><Icons.GitBranch size={13} /></span>
                    <BranchSwitcher key={sel.repo + "|" + sel.slug} repo={sel.repo} slug={sel.slug}
                      onSwitch={doSwitch} onError={fail} />
                  </>
                )}
              </div>
              <div className="sb-facts">
                {selected.tmux_session.up ? <>tmux <span className="ok">●</span> up · {selected.tmux_session.name}</> : <>tmux <span className="off">○</span> down</>}
                {selected.claude_session_present ? " · pane0 claude" : ""}
              </div>
            </footer>
          </>
        ) : (
          <div className="briefing">
            <div className="home-hero">
              <img className="home-logo" src={logoUrl} alt="worktrees logo" />
              <div className="home-id">
                <h1>worktrees</h1>
                <div className="home-tag">a place for every work stream</div>
              </div>
            </div>
            <button className="enter-btn big home-open with-icon" onClick={addProject}><Icons.Plus size={15} /> Open a project</button>
            <div className="chips">
              <span className="chip"><span className="dot" style={{ background: "var(--ok)" }} /> {stats.live} live</span>
              <span className="chip"><span className="dot" style={{ background: "var(--dirty)" }} /> {stats.dirty} dirty</span>
            </div>
            <div className="resume-h">RESUME WHERE YOU LEFT OFF</div>
            <div className="resume">
              {resume.length === 0 && <div className="empty small">No places yet — open a project to start.</div>}
              {resume.map(({ pv, p }) => (
                <div className="resume-row" key={pv.root + p.slug} onClick={() => enterPlace(pv.root, p)} onContextMenu={(e) => placeCtx(e, pv.root, p)}>
                  <span className={"status-dot" + dotClass(p)} title={dotTitle(p)} />
                  <span className="rr-name">{p.declared?.pinned ? "★ " : ""}{p.slug}</span>
                  <span className="rr-proj">{basename(pv.root)}</span>
                  <span className="rr-life">{p.lifecycle_effective}</span>
                  <span className="rr-age">{ago(usedEpoch(p))}</span>
                  <button className="enter-btn sm with-icon">Enter <Icons.ChevronRight size={12} /></button>
                </div>
              ))}
            </div>
          </div>
        )}

        {/* ── reading mode (⌘⇧E): the dock's open file over the whole main
            pane. The terminal stays MOUNTED underneath (hidden, not
            unmounted) — unmounting would drop the xterm and its scrollback;
            TerminalPane refits when it is revealed again. */}
        {reading && dockFile && selected && (
          // Spans the main pane AND the dock (stopping at the rails): an
          // overlay confined to `main` gets NARROWER as the dock grows, which
          // is the opposite of "expand".
          <div
            className="reading"
            role="dialog"
            aria-label="File reader"
            style={{ left: 0, right: dockShown ? -fit.dockW : 0 }}
          >
            <FileView
              key={dockFile}
              path={dockFile}
              reloadToken={placesToken}
              onOpen={setDockFile}
              onOpenEditor={editIn}
              onError={fail}
              wrap={settings.files_wrap}
              onWrap={(v) => updateSettings({ files_wrap: v })}
              mdSource={settings.files_md_source}
              onMdSource={(v) => updateSettings({ files_md_source: v })}
              expanded
              onExpand={(v) => setReading(v)}
            />
          </div>
        )}
      </main>

      {/* ── right dock: Files (browse + edit) / Terminal (embedded shell) ── */}
      {dockShown && selected && sel && (
        <aside className="dock">
          <div className="dock-resizer" onMouseDown={onDockResize} />
          {/* the rail owns tab selection AND collapse, so this is a title, not
              a control strip */}
          <div className="dock-tabs">
            <span className="dock-title">{settings.dock_tab === "files" ? "Files" : "Terminal"}</span>
            <span className="dock-spacer" />
            {settings.dock_tab === "files" && (
              <>
                <button
                  className="ctrl sm icon-only"
                  aria-label="Refresh the file tree"
                  title="Re-list files from disk"
                  onClick={reloadFiles}
                >
                  ↻
                </button>
                <button
                  className={"ctrl sm icon-only" + (settings.files_show_ignored ? " on" : "")}
                  aria-label={`Gitignored files: ${settings.files_show_ignored ? "shown" : "hidden"}. Click to toggle.`}
                  aria-pressed={settings.files_show_ignored}
                  title={settings.files_show_ignored
                    ? "Hide gitignored files"
                    : "Show gitignored files (build output, working notes)"}
                  onClick={() => updateSettings({ files_show_ignored: !settings.files_show_ignored })}
                >
                  {/* Filled when nothing is being withheld. The `on` class alone
                      is a tint, and a tint is not enough to notice that a tree
                      IS hiding entries — which is the state that misleads. */}
                  {settings.files_show_ignored ? "◉" : "◌"}
                </button>
                <button
                  className="ctrl sm icon-only"
                  aria-label={`Files layout: ${settings.files_layout}. Click to cycle.`}
                  title={`Layout: ${settings.files_layout} — click to cycle (auto → stacked → side by side)`}
                  onClick={() => updateSettings({ files_layout: NEXT_FILES_LAYOUT[settings.files_layout] })}
                >
                  {settings.files_layout === "auto" ? "A" : settings.files_layout === "stack" ? "▤" : "▥"}
                </button>
              </>
            )}
          </div>
          <div className="dock-body">
            {settings.dock_tab === "files" ? (
              <FilesPane
                root={selected.path}
                openPath={dockFile}
                dockW={fit.dockW}
                layout={settings.files_layout}
                splitPct={settings.files_split_pct}
                stackPct={settings.files_stack_pct}
                onSplitPct={(v, o) => updateSettings(o === "split" ? { files_split_pct: v } : { files_stack_pct: v })}
                showIgnored={settings.files_show_ignored}
                reloadToken={placesToken}
                onOpen={setDockFile}
                onOpenEditor={editIn}
                onError={fail}
                wrap={settings.files_wrap}
                onWrap={(v) => updateSettings({ files_wrap: v })}
                mdSource={settings.files_md_source}
                onMdSource={(v) => updateSettings({ files_md_source: v })}
                expanded={false}
                onExpand={(v) => setReading(v)}
              />
            ) : (
              <TerminalTabs key={sel.repo + "|" + sel.slug}
                repo={sel.repo} slug={sel.slug} sessionUp={selected.tmux_session.up}
                termVersion={termVersion} focusToken={termFocus} addToken={newTermToken}
                names={(settings.term_tab_names ?? {})[sel.repo + "|" + sel.slug] ?? {}}
                onRename={(index, name) => renameTermTab(sel.repo, sel.slug, index, name)}
                onError={fail} />
            )}
          </div>
        </aside>
      )}

      {/* ── right rail: mirrors the left one. Permanent, so the dock always has
          a visible affordance; the active icon collapses the dock. Disabled
          with no place selected — Files/Terminal both need a worktree. */}
      <nav className="rail rail-right">
        {DOCK_RAIL.map((d) => {
          const on = dockShown && settings.dock_tab === d.key;
          const why = !dockEligible ? "select a place first" : !dockFits ? "window too narrow" : null;
          return (
            <button
              key={d.key}
              className={"rail-icon" + (on ? " active" : "")}
              disabled={!!why}
              title={why ? `${d.title} — ${why}` : on ? `hide ${d.title.toLowerCase()} (⌘J)` : `${d.title} (⌘J)`}
              onClick={() => pickDockTab(d.key)}
            >
              {d.icon}
            </button>
          );
        })}
      </nav>

      {/* error surface lives OUTSIDE the nav — must stay visible in rail-only mode */}
      {err && <div className="err err-float" title="dismiss" onClick={() => setErr("")}>{err}</div>}
      {!err && notice && <div className="err err-float notice" title="dismiss" onClick={() => setNotice("")}>{notice}</div>}

      {sortOpen && <div className="menu-catch" onClick={() => setSortOpen(false)} />}

      {/* Per-project sheet (right-click a project → Project settings…). Keyed by
          root so switching projects remounts it with fresh state; `open` gates
          the render exactly like SettingsSheet's. */}
      <ProjectSheet
        key={projSheet ?? "none"}
        open={!!projSheet}
        root={projSheet ?? ""}
        editorCmd={settings.editor_cmd}
        suggestion={projSheet ? suggest[projSheet] ?? null : null}
        onClose={() => setProjSheet(null)}
        onReport={takeReport}
        onConfigWritten={(root) => { probeSuggest(root); refresh(); }}
      />

      <SettingsSheet open={settingsOpen} settings={settings} onChange={updateSettings} onClose={() => setSettingsOpen(false)}
        update={upd} cliStale={cliStale} cliMissing={cliMissing} appStale={appStale} onCheckUpdate={checkUpdate}
        onShowNotes={showReleaseNotes} onReset={onReset}
        repo={sel?.repo ?? ""} onReport={(m) => setNotice(m)} />

      {/* ⌘K quick switcher — a full overlay independent of the nav (works in
          rail-only mode). Gated on switchOpen so it MOUNTS FRESH each open (query
          + highlight reset, autofocus fires). enterPlace bumps termFocus, returning
          focus to the terminal after a pick. */}
      {switchOpen && (
        <QuickSwitch open items={allPlaces} busyPaths={busyPaths} waitingPaths={waitingPaths}
          onPick={(root, p) => { setSwitchOpen(false); enterPlace(root, p); }}
          onClose={() => setSwitchOpen(false)} />
      )}

      {/* after SettingsSheet: opened from Settings, the notes stack ON TOP of it */}
      {whatsNew && (
        <div className="scrim" onClick={() => { updateSettings({ last_seen_version: whatsNew.version }); setWhatsNew(null); }}>
          <aside className="settings-sheet whatsnew" onClick={(e) => e.stopPropagation()}>
            <header className="settings-h">
              <b>{whatsNew.manual ? "Release notes" : `What's new — v${whatsNew.version}`}</b>
              <button className="icon-btn" title="close"
                onClick={() => { updateSettings({ last_seen_version: whatsNew.version }); setWhatsNew(null); }}><Icons.X size={13} /></button>
            </header>
            <div className="settings-body">
              <ReleaseNotes notes={whatsNew.notes} />
            </div>
          </aside>
        </div>
      )}
      {menu && <div className="menu-catch" onClick={closeMenu} />}

      {/* ── right-click: place ── */}
      {ctx?.kind === "place" && ctxPlace && (
        <CtxMenu x={ctx.x} y={ctx.y} onClose={closeCtx}>
          <div className="pop-hint">{ctxPlace.is_main ? "◆ main" : ctxPlace.slug}</div>
          <button className="pop-item" onClick={() => enterPlace(ctx.repo, ctxPlace)}>Enter <Icons.ChevronRight size={12} /></button>
          {!ctxPlace.tmux_session.up && ctxPlace.claude_session_present && (
            settings.ai_auto_resume ? (
              <button className="pop-item" onClick={() => enterPlace(ctx.repo, ctxPlace, { fresh: true })}>Open fresh (skip resume)</button>
            ) : (
              <button className="pop-item" onClick={() => enterPlace(ctx.repo, ctxPlace, { fresh: false })}>Open with resume</button>
            )
          )}
          {ctxPlace.tmux_session.up && (
            <>
              {confirmRm === closeCtxKey(ctx.repo, ctxPlace.slug) ? (
                <button className="pop-item danger armed" onClick={() => closeSession(ctx.repo, ctxPlace.slug)}>
                  Kill {closeSess} — whole session?
                </button>
              ) : (
                <button className="pop-item" onClick={() => closeSession(ctx.repo, ctxPlace.slug)}>Close session</button>
              )}
              <button className="pop-item" onClick={() => copyText(`tmux attach -t ${shq(ctxPlace.tmux_session.name)}`)}>Copy attach command</button>
              {settings.terminal_cmd.trim() && (
                <button className="pop-item" onClick={() => {
                  const session = ctxPlace.tmux_session.name;
                  closeCtx();
                  invoke("open_terminal", { cmd: settings.terminal_cmd, session }).catch((e) => fail(e));
                }}>Open in terminal app</button>
              )}
            </>
          )}
          <div className="ctx-sep" />
          {!ctxPlace.is_main && (
            <>
              <button className="pop-item" onClick={() => { closeCtx(); mutate(invoke("set_pin", { repo: ctx.repo, slug: ctxPlace.slug, on: !ctxPlace.declared?.pinned })); }}>
                {ctxPlace.declared?.pinned ? "★ Unpin" : "☆ Pin"}
              </button>
              <div className="pop-hint">lifecycle</div>
              <div className="ctx-life">
                {SETTABLE.map((s) => (
                  <button key={s.value} className={ctxPlace.declared?.lifecycle === s.value ? "on" : ""}
                    onClick={() => { closeCtx(); mutate(invoke("set_lifecycle", { repo: ctx.repo, slug: ctxPlace.slug, label: s.value })); }}>
                    {s.label}
                  </button>
                ))}
              </div>
              <button className="pop-item" onClick={() => editNote(ctx.repo, ctxPlace)}>Edit note…</button>
              <div className="ctx-sep" />
              {ctxPlace.branch && (
                <button className="pop-item" onClick={() => newFromBranch(ctx.repo, ctxPlace.branch!)}>New worktree off {ctxPlace.branch}…</button>
              )}
            </>
          )}
          <button className="pop-item" onClick={() => openOnRemote(ctx.repo, ctxPlace.slug)}>Open on GitHub</button>
          <button className="pop-item" onClick={() => revealPlace(ctxPlace.path)}>Reveal in Finder</button>
          <button className="pop-item" onClick={() => editIn(ctxPlace.path)}>Open in editor</button>
          <button className="pop-item" onClick={() => copyText(ctxPlace.path)}>Copy path</button>
          {ctxPlace.branch && <button className="pop-item" onClick={() => copyText(ctxPlace.branch!)}>Copy branch</button>}
          {!ctxPlace.is_main && (
            <>
              <div className="ctx-sep" />
              {confirmRm === `ctx|${ctx.repo}|${ctxPlace.slug}` ? (
                <>
                  <button className="pop-item danger armed" onClick={() => confirmRemovePlaceCtx(ctx.repo, ctxPlace.slug, false)}>
                    Confirm remove
                  </button>
                  <button className="pop-item danger armed" onClick={() => confirmRemovePlaceCtx(ctx.repo, ctxPlace.slug, true)}>
                    Confirm remove + branch
                  </button>
                </>
              ) : (
                <button className="pop-item danger" onClick={() => armRemovePlaceCtx(ctx.repo, ctxPlace.slug)}>
                  Remove worktree…
                </button>
              )}
            </>
          )}
        </CtxMenu>
      )}

      {/* ── right-click: project ── */}
      {ctx?.kind === "project" && (() => {
        const pv = ws?.projects.find((v) => v.root === ctx.root);
        const main = pv?.snapshot?.places.find((p) => p.is_main) ?? null;
        return (
          <CtxMenu x={ctx.x} y={ctx.y} onClose={closeCtx}>
            <div className="pop-hint">{basename(ctx.root)}</div>
            <button className="pop-item" onClick={() => { closeCtx(); openNewForm(ctx.root, ""); }}>New worktree…</button>
            {main && <button className="pop-item" onClick={() => enterPlace(ctx.root, main)}>Enter main ▸</button>}
            {pv?.ok && (
              <button className="pop-item" onClick={() => { closeCtx(); mutate(invoke("fetch_origin", { root: ctx.root })); }}>Fetch origin</button>
            )}
            {pv?.ok && (() => {
              // The badge is the SHEET's count (issueCount), not the row-glyph set:
              // those two answer different questions, and a placeless finding used
              // to make them disagree with no way to tell which was lying. When the
              // last run couldn't produce a report at all, the count is not a fact
              // — say so instead of showing a number.
              const h = health[ctx.root];
              return (
                <button className="pop-item" onClick={() => { closeCtx(); setProjSheet(ctx.root); }}>
                  Project settings…
                  {h?.error
                    ? <span className="upd-tag warn" title={h.error}>unchecked</span>
                    : (h?.issues ?? 0) > 0
                      ? <span className="upd-tag warn" title="findings from the last doctor run">{h!.issues}</span>
                      : null}
                </button>
              );
            })()}
            <div className="ctx-sep" />
            <button className="pop-item" onClick={() => copyText(ctx.root)}>Copy path</button>
            <button className="pop-item" onClick={() => revealPlace(ctx.root)}>Reveal in Finder</button>
            <button className="pop-item" onClick={() => editIn(ctx.root)}>Open in editor</button>
            <div className="ctx-sep" />
            <button
              className={"pop-item danger" + (confirmRm === `proj|${ctx.root}` ? " armed" : "")}
              onClick={() => removeProjectCtx(ctx.root)}
            >
              {confirmRm === `proj|${ctx.root}` ? "Confirm remove?" : "Remove from workspace"}
            </button>
          </CtxMenu>
        );
      })()}
    </div>
  );
}

export default App;
