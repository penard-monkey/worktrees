import { Fragment, useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import * as Icons from "./icons";
import { CtxMenu } from "./CtxMenu";
import { ShellPane, TerminalPane } from "./TerminalPane";
import { FilesPane, FileView } from "./FilesPane";
import { SettingsSheet } from "./SettingsSheet";
import {
  driftedSlugs, InitBanner, issueCount, ProjectSheet, reportFailed,
  type DoctorReport, type InitSuggestion,
} from "./ProjectSheet";
import { fileInfo } from "./filekind";
import { applySettings, clampDock, clampMdZoom, clampNav, DEFAULTS, fitLayout, loadSettings, panelsFor, placeKey, saveSettings, stepMdZoom, viewportWidth, type PlacePanels, type Settings, type UpdateInfo } from "./settings";
import {
  alphaIndex, dropIntent, landingNote, moveBefore, naturalTop, pointerIndex,
  predictTier, recentIndex, spliceOrder, TIER_LABEL, type DeclPatch, type Tier,
} from "./dnd";
import { useNavDrag, type DragItem } from "./navdrag";
import logoUrl from "./assets/logo.png";
import "./tokens.css";
import "./App.css";

type Declared = {
  lifecycle?: string;
  pinned?: boolean;
  /// Display name. The slug stays the identity (see store.rs) — this only
  /// changes what a place is CALLED. Use `nameOf(place)` to render either.
  title?: string;
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

// Markdown reading-size chords. Both faces of each key: ⌘+ is ⌘⇧= on a US
// layout ("+"), ⌘− is plain "-" but "_" when shifted, and the numeric keypad
// sends "+"/"-" unshifted. ⌘0 resets.
const ZOOM_KEYS = new Set(["+", "=", "-", "_", "0"]);

/** Where ⌘F lands. Follows the surface the user last TOUCHED, not the one that
 *  holds DOM focus: the file tree's rows are plain divs, so clicking a file to
 *  read it leaves `document.activeElement` on `<body>` and a focus-based rule
 *  would send ⌘F to the terminal while the user is looking at the file.
 *
 *  The reader wins outright when it is up (it covers everything else), and the
 *  place's own terminal is the fallback. A dock showing Files with nothing open
 *  is NOT a target — that viewer renders a hint, not a file. */
function findTarget(s: {
  reading: boolean; dockFile: string | null; dockShown: boolean;
  dockTab: Settings["dock_tab"]; mainTermUp: boolean; last: Surface;
}): null | "main" | "dock" | "read" {
  if (s.reading && s.dockFile) return "read";
  // The Terminal tab renders NO shell when the place's session is down (dock
  // shells are swept with it), and a target that mounts no bar is worse than
  // no target: every further ⌘F picks it again and the main pane is
  // unreachable from the keyboard.
  const dockOk = s.dockShown && (s.dockTab === "terminal" ? s.mainTermUp : !!s.dockFile);
  if (s.last === "dock" && dockOk) return "dock";
  if (s.mainTermUp) return "main";
  return dockOk ? "dock" : null;
}

type Surface = "main" | "dock" | "read";

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
/** What a place is CALLED. The slug remains its identity — the directory, the
 *  tmux session and "Copy path" are all still slug-derived — so anywhere this
 *  is rendered the slug must stay reachable rather than be replaced outright. */
const nameOf = (p: { slug: string; declared?: Declared }) => p.declared?.title?.trim() || p.slug;
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
/// "When did I last USE this place" — opened or worked, whichever is newer.
///
/// ONE consumer left: the restore-on-launch target. Every list a user reads —
/// the nav tree, ⌘K, the Recent lens, the home Resume list — orders and labels
/// by `activityAt` instead, so the age printed on a row is the reason it sits
/// where it does. Four lists that agreed on nothing were four different answers
/// to "what did I touch last".
///
/// Restore keeps THIS key because it deliberately has no commit-epoch fallback.
/// A worktree created from the CLI and never touched has a commit from
/// yesterday and no user history at all; `activityAt` would rank it above a
/// place actually opened last week, and the app would launch itself into a
/// place the user has never seen. A list can survive being wrong about that —
/// it shows the next row too. The restore target can't.
const usedEpoch = (p: Place) =>
  Math.max(p.declared?.last_opened_epoch ?? 0, p.declared?.last_worked_epoch ?? 0);

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

type Ctx =
  | { kind: "place"; x: number; y: number; repo: string; slug: string }
  | { kind: "project"; x: number; y: number; root: string }
  // The ⇄ popover. Same state machine as the right-click menus on purpose: only
  // one of them can be open, and every dismissal path already clears it.
  | { kind: "sync"; x: number; y: number; root: string }
  // The nav footer's add menu. A project can arrive three ways now — created,
  // added from disk, imported off the hub — and three footer buttons would be
  // three permanent rows of chrome for an act performed a handful of times.
  | { kind: "add"; x: number; y: number };

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

// Rename-in-place for the space header's name.
//
// ⚠ MODULE SCOPE, and it has to be: a component defined inside App() gets a new
// identity on every render, so React would unmount and recreate this input —
// and the 3s poll renders App constantly. It would lose focus, the caret, and
// any partial edit, roughly once per keystroke's worth of polling.
//
// Uncontrolled + committed on blur, matching the note strip. Enter commits,
// Escape reverts; both blur, so `done` guards against the blur handler firing a
// second commit after the key already handled it.
function TitleEditor({ initial, slug, onCommit, onCancel }: {
  initial: string;
  slug: string;
  onCommit: (v: string) => void;
  onCancel: () => void;
}) {
  const done = useRef(false);
  return (
    <input
      className="title-input"
      defaultValue={initial}
      autoFocus
      spellCheck={false}
      aria-label={`Name for ${slug}`}
      placeholder={slug}
      onFocus={(e) => e.currentTarget.select()}
      onBlur={(e) => { if (!done.current) { done.current = true; onCommit(e.currentTarget.value); } }}
      onKeyDown={(e) => {
        if (e.key === "Enter") { done.current = true; onCommit(e.currentTarget.value); }
        else if (e.key === "Escape") { done.current = true; onCancel(); }
        else return;
        e.preventDefault();
        e.stopPropagation(); // never let Enter/Escape reach the global keydown
      }}
    />
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

// ── ⌘K quick-switcher ──
// Fuzzy SUBSEQUENCE match over a composite key (slug + branch + project basename
// + note). A place matches if the query chars appear IN ORDER (case-insensitive);
// score prefers a contiguous substring hit over a scattered subsequence, an
// earlier hit over a later one, and a slug hit over a branch/project hit.
//
// `rank` (desc) breaks ties, and IS the order when the query is empty — the
// instant "recent places" list, which is the common case. It arrives as a prop
// because it is the NAV's clock (`activityAt`, which closes over App state) and
// the two must not drift: the empty-query list and the tree behind the palette
// show the same places, so any disagreement reads as one of them being broken.
// Each row prints that clock, for the same reason nav rows do.
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

function QuickSwitch({ open, items, rank, busyPaths, waitingPaths, onPick, onClose }: {
  open: boolean;
  items: SwitchItem[];
  rank: (p: Place) => number;
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
    if (!q) {
      return [...items]
        .sort((a, b) => rank(b.p) - rank(a.p))
        .slice(0, SWITCH_CAP);
    }
    const scored: { it: SwitchItem; score: number }[] = [];
    for (const it of items) {
      const { pv, p } = it;
      const slug = p.slug.toLowerCase();
      // A renamed place must be findable by the name actually on screen —
      // otherwise ⌘K is the one place a title makes you WORSE off.
      const title = (p.declared?.title ?? "").toLowerCase();
      const branch = (p.branch ?? "").toLowerCase();
      const proj = basename(pv.root).toLowerCase();
      const note = (p.declared?.note ?? "").toLowerCase();
      const composite = `${slug} ${title} ${branch} ${proj} ${note}`;
      // reject early: must match the whole composite as a subsequence
      if (fuzzyScore(q, composite) < 0) continue;
      // rank on the best field, biased toward whatever NAMES the place — the
      // title carries the slug's bias because it is what the user sees
      const s = Math.max(
        fuzzyScore(q, slug) + 200, // slug hits win
        title ? fuzzyScore(q, title) + 200 : -1,
        fuzzyScore(q, branch),
        fuzzyScore(q, proj),
        fuzzyScore(q, note),
      );
      scored.push({ it, score: s });
    }
    scored.sort((a, b) => (b.score - a.score) || (rank(b.it.p) - rank(a.it.p)));
    return scored.slice(0, SWITCH_CAP).map((x) => x.it);
  }, [items, query, rank]);

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
                    {p.declared?.pinned ? "★ " : p.is_main ? "◆ " : ""}{nameOf(p)}
                  </span>
                  {nameOf(p) !== p.slug && <span className="qs-alias">{p.slug}</span>}
                  <span className="qs-proj">{basename(pv.root)}</span>
                  {p.branch && p.branch !== p.slug && <span className="qs-branch">{p.branch}</span>}
                  <span className={"life " + p.lifecycle_effective}>{p.lifecycle_effective}</span>
                  <span className="qs-age">{ago(rank(p))}</span>
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

function TerminalTabs({ repo, slug, sessionUp, termVersion, focusToken, addToken, names, onRename, tabs, onTabs, activeTab, onActiveTab, onError, findOpen, findToken, onFindClose }: {
  repo: string; slug: string; sessionUp: boolean; termVersion: number; focusToken: number; addToken: number;
  names: Record<number, string>; onRename: (index: number, name: string | null) => void;
  tabs: number[]; onTabs: (ids: number[]) => void;
  activeTab: number | null; onActiveTab: (index: number | null) => void;
  onError: (e: unknown) => void;
  /** ⌘F goes to whichever shell tab is showing — only that one is mounted */
  findOpen?: boolean; findToken?: number; onFindClose?: () => void;
}) {
  const [ids, setIds] = useState<number[] | null>(null); // null = restoring
  const [active, setActive] = useState<number | null>(null);
  // exited-but-kept tabs (see the shell:exit listener below) — declared here so
  // the restore can seed it: the exit EVENT is transient and a shell that died
  // while the dock was closed had no listener, so liveness rides on the restore
  const [dead, setDead] = useState<number[]>([]);
  // Bumped when the place's session comes back UP, to re-run the restore below
  // (see the effect that watches `sessionUp`).
  const [upToken, setUpToken] = useState(0);
  const idsRef = useRef<number[]>([]);
  idsRef.current = ids ?? [];
  // Read by the restore and by the stable callbacks below, which must NOT be
  // rebuilt when the remembered tab changes — same reason `names` has a ref.
  const activeTabRef = useRef(activeTab);
  activeTabRef.current = activeTab;
  const tabsRef = useRef(tabs);
  tabsRef.current = tabs;
  const onTabsRef = useRef(onTabs);
  onTabsRef.current = onTabs;
  /** Change the tab strip AND remember it, so the tabs you opened are still
   *  there after a restart. Only deliberate edits go through here: the restore
   *  must not write, or a place visited while its session is down would record
   *  the empty strip it is correctly showing.
   *
   *  `shown` and `remembered` are separate because they legitimately differ. A
   *  place whose session is down shows NO tabs — remembered ones are gated out,
   *  since resurrecting tabs for a dead session would be a lie — so if a
   *  deliberate edit wrote the visible strip, the first "+ new terminal" there
   *  would record `[1]` over the `[1,2,3]` you actually have. Callers pass what
   *  to display and, separately, what the record should become. */
  const commitIds = useCallback((shown: number[], remembered: number[]) => {
    setIds(shown);
    onTabsRef.current(remembered);
  }, []);
  /** Sorted union — the record must never lose a tab it already knew about. */
  const withRemembered = (list: number[]) =>
    [...new Set([...tabsRef.current, ...list])].sort((a, b) => a - b);
  const onActiveTabRef = useRef(onActiveTab);
  onActiveTabRef.current = onActiveTab;
  /** Change tabs AND remember it. Every path that moves the front tab goes
   *  through here, so "what I was looking at" is whatever the user last did. */
  const pick = useCallback((id: number | null) => {
    setActive(id);
    onActiveTabRef.current(id);
  }, []);
  const restoringRef = useRef(true);
  // names is read by the restore, which must NOT re-run on every rename
  const namesRef = useRef(names);
  namesRef.current = names;
  const [editing, setEditing] = useState<number | null>(null);
  const labelOf = (id: number) => names[id] || `sh ${id}`;

  // Restore tabs from the live shell registry on mount / place change, UNIONed
  // with the strip this place had last time. Shells do not outlive the app, so
  // that remembered strip is what brings a tab back at all: activating it
  // mounts ShellPane, which spawns a fresh shell — in the tab's LAST directory,
  // which the backend remembers separately (shell-cwds.json). The union is
  // gated on a live session, so a closed place still shows nothing.
  //
  // `names` is unioned in as well, purely for installs that predate the strip
  // being remembered: their only record of a tab is the fact it was named.
  useEffect(() => {
    let alive = true;
    restoringRef.current = true;
    setIds(null); setActive(null); setDead([]); setEditing(null);
    invoke<{ index: number; dead: boolean }[]>("list_shell_sessions", { repo, slug })
      .then((existing) => {
        if (!alive) return;
        const isTab = (n: number) => Number.isInteger(n) && n > 0;
        const remembered = sessionUp
          ? [...tabsRef.current.filter(isTab), ...Object.keys(namesRef.current).map(Number).filter(isTab)]
          : [];
        const union = [...new Set([...existing.map((t) => t.index), ...remembered])].sort((a, b) => a - b);
        const list = union.length ? union : (sessionUp ? [1] : []);
        setDead(existing.filter((t) => t.dead).map((t) => t.index));
        // The remembered tab, if it is still one of the tabs — it can have been
        // closed, or belong to a session that is now down. Restoring does NOT
        // write the fallback back: a place whose remembered tab is temporarily
        // absent should find it again next time, not have it overwritten.
        const want = activeTabRef.current;
        setIds(list); setActive(want != null && list.includes(want) ? want : list[0] ?? null);
        restoringRef.current = false;
      })
      .catch((e) => {
        if (!alive) return;
        setIds(sessionUp ? [1] : []); setActive(sessionUp ? 1 : null); restoringRef.current = false; onError(e);
      });
    return () => { alive = false; };
    // sessionUp intentionally excluded — its transitions are handled below so a
    // flip doesn't clobber the user's tabs mid-session. `upToken` is that
    // handling for down→up: a fresh restore, rather than the raw flip.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [repo, slug, upToken]);

  // When the place's session goes DOWN, Close swept its dock shells too (same
  // rule as the tmux era: scratch shells die with the place) — clear the tabs so
  // the dock reflects reality instead of resurrecting dead ones.
  const prevUp = useRef(sessionUp);
  useEffect(() => {
    if (prevUp.current && !sessionUp) { setIds([]); setActive(null); }
    // …and the other direction, which nothing used to handle: this component is
    // keyed on the PLACE, so entering a place whose session was down does not
    // remount it and the mount-only restore never runs again. The strip stayed
    // empty until you switched places — with the remembered tabs sitting right
    // there in ui-state.json, which reads as the feature being broken.
    if (!prevUp.current && sessionUp) setUpToken((t) => t + 1);
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
    // keepCwd — this closes the DEAD shell to make room for a new one in the
    // same tab, so the tab keeps the directory it was last in.
    invoke("close_shell_session", { repo, slug, index: id, keepCwd: true }).catch(onError);
    setDead((d) => d.filter((x) => x !== id));
    setRestartToken((t) => t + 1); // remount the pane → shell_open spawns afresh
  };
  const [restartToken, setRestartToken] = useState(0);

  const addTab = useCallback(() => {
    if (restoringRef.current) return; // don't add a tab the restore is about to overwrite
    // Numbered above every index this place KNOWS about, not just the ones on
    // screen. With the session down the strip is empty by design while the
    // record still holds [1,2,3], and picking `max(shown)+1` handed the new tab
    // index 1 — so it appeared already wearing tab 1's name and its shell
    // opened in tab 1's remembered directory. A brand-new tab impersonating a
    // tab you cannot see reads as the app being haunted.
    const known = withRemembered(idsRef.current);
    const cur = idsRef.current;
    const next = (known.length ? Math.max(...known) : 0) + 1;
    const shown = [...cur, next];
    commitIds(shown, withRemembered(shown));
    pick(next);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [commitIds, pick]);

  // ⌘T / ⌘⇧T → add a tab. Skip the initial token value so mounting doesn't add
  // one — which is also what lets ⌘T bump the token in the same render that
  // MOUNTS this component (opening the dock on the Terminal tab) without the
  // restore and the bump both producing a tab.
  const firstTok = useRef(true);
  useEffect(() => {
    if (firstTok.current) { firstTok.current = false; return; }
    addTab();
  }, [addToken, addTab]);

  const closeTab = (id: number) => {
    // the tab is going away, so the backend drops its remembered directory too
    // (the counterpart of dropping its name, just below)
    invoke("close_shell_session", { repo, slug, index: id, keepCwd: false }).catch(onError);
    onRename(id, null); // an explicitly closed tab drops its name — otherwise it
                        // would be seeded straight back on the next restore
    const remaining = idsRef.current.filter((x) => x !== id);
    commitIds(remaining, withRemembered(remaining).filter((x) => x !== id));
    if (active === id) pick(remaining.length ? remaining[remaining.length - 1] : null);
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
                onClick={() => pick(id)} onDoubleClick={() => setEditing(id)}>{labelOf(id)}</button>
            )}
            <button className="termtab-x" title="close shell" onClick={() => closeTab(id)}><Icons.X size={11} /></button>
          </span>
        ))}
        <button className="termtab-add" title="new terminal (⌘T)" onClick={addTab}><Icons.Plus size={13} /></button>
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
            index={active} termVersion={termVersion} focusToken={focusToken}
            findOpen={findOpen} findToken={findToken} onFindClose={onFindClose} />
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

// ── branch combobox ──
//
// A combobox, NOT a <select>: `switch` is DWIM — local branch → switch,
// remote-only → track it, anything else → create it off the default base — so a
// picker that only offered existing branches would delete the create path.
// Typing stays first-class; the list just means you no longer have to remember
// the name.
//
// The ▾ is not decoration. Without it this is an input with a placeholder and
// nothing anywhere says a list exists — the popover opened on FOCUS, which you
// can only discover by clicking into a box you believe you have to type into.
//
// Module scope with props, per CLAUDE.md: it holds local state and input focus,
// both of which a component defined inside App() would lose on every render.
// The TEXT is deliberately the parent's: the switcher clears it only on a
// SUCCESSFUL switch, which is knowledge this component does not have.
type BranchList = { branches: string[]; current: string; default_base: string };

function BranchCombo({
  value, data, placeholder, exclude, inputClass, ariaLabel, testid,
  drop = "up", allowCreate = true, disabled = false, autoFocus = false, openOnFocus = true,
  onChange, onOpen, onCommit,
}: {
  value: string;
  /** null = not loaded yet (the pop says so). The PARENT owns the fetch: one
   *  list can back two fields, and only the parent knows when to invalidate. */
  data: BranchList | null;
  placeholder: string;
  /** A branch to drop from the list — the switcher hides the one you are on. */
  exclude?: string;
  inputClass: string;
  ariaLabel: string;
  testid?: string;
  /** Which way the popover grows. "up" for the status bar (last row of the
   *  window); "down" inside a dialog, where up would leave the box off-screen. */
  drop?: "up" | "down";
  /** The DWIM "create X off base" row. Off for a field that must name something
   *  that ALREADY resolves — a base is a start point, not a thing to invent. */
  allowCreate?: boolean;
  disabled?: boolean;
  autoFocus?: boolean;
  /** Whether merely focusing the field opens the list. True in the status bar,
   *  where the field exists to be typed in and the pop has empty space above it.
   *  False in a dialog: the branch field autofocuses on open, and a list dropped
   *  over the fields below it before you have typed anything hides the very
   *  things the dialog was built to show. Typing and the ▾ still open it. */
  openOnFocus?: boolean;
  onChange: (v: string) => void;
  /** First open / first keystroke — the parent's cue to fetch lazily. */
  onOpen: () => void;
  /** Enter, or a row picked with the mouse. */
  onCommit: (v: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [hi, setHi] = useState(0);
  const boxRef = useRef<HTMLDivElement | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);
  // Per-INSTANCE ids: two combos can be on screen at once (the dialog's Branch
  // and Base), and a duplicated id would point every input's activedescendant
  // at the same list.
  const uid = useId();
  const listId = `${uid}-list`;
  const optId = (i: number) => `${uid}-opt-${i}`;

  const q = value.trim();
  const matches = (data?.branches ?? [])
    .filter((b) => b !== exclude && b.toLowerCase().includes(q.toLowerCase()))
    .slice(0, 50);
  const exact = matches.some((b) => b === q);
  const creating = allowCreate && q.length > 0 && !exact;
  // the create row is last, so ↓ from the top walks real branches first
  const options: { branch: string; create: boolean }[] = [
    ...matches.map((b) => ({ branch: b, create: false })),
    ...(creating ? [{ branch: q, create: true }] : []),
  ];

  // One row, and it is the exact thing already typed: there is nothing to pick.
  // Left visible it would cover whatever sits under the field — in the dialog
  // that is the verdict line, i.e. the answer you typed the branch to get.
  const useless = options.length === 1 && !options[0].create && options[0].branch === q;
  const showPop = open && !useless;

  const commit = (branch: string) => {
    const b = branch.trim();
    if (!b) return;
    setOpen(false);
    onCommit(b);
  };

  const show = () => { setOpen(true); onOpen(); };

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
      if (!open) { show(); return; }
      const d = e.key === "ArrowDown" ? 1 : -1;
      setHi((i) => (options.length ? (i + d + options.length) % options.length : 0));
      return;
    }
    if (e.key === "Escape" && open) {
      // Only the OPEN pop is closed here, and the event stops: a dialog hosting
      // this field must not also take the same Escape as "cancel".
      e.preventDefault();
      e.stopPropagation();
      setOpen(false);
      return;
    }
    if (e.key === "Enter") {
      // A highlighted row wins; otherwise the raw text does, so Enter still
      // works exactly as it did before the list existed.
      commit(open && options[hi] ? options[hi].branch : value);
      // Picking from an OPEN list is the whole keypress. Without this the
      // dialog hosting the field would also read it as "submit", so choosing a
      // branch would create the worktree before you had touched base or name.
      // A CLOSED pop lets it through, which is what makes Enter still submit.
      if (open) e.stopPropagation();
    }
  };

  return (
    <div className={"combo" + (drop === "down" ? " down" : "") + (disabled ? " off" : "")} ref={boxRef}>
      <input
        ref={inputRef}
        className={inputClass}
        placeholder={placeholder}
        value={value}
        role="combobox"
        aria-expanded={showPop}
        aria-label={ariaLabel}
        aria-autocomplete="list"
        aria-controls={showPop ? listId : undefined}
        // Focus never leaves the input, so WITHOUT this the arrow keys are
        // silent to a screen reader and Enter commits a row it never announced.
        aria-activedescendant={showPop && options[hi] ? optId(hi) : undefined}
        data-testid={testid}
        spellCheck={false}
        autoCapitalize="off"
        autoCorrect="off"
        disabled={disabled}
        autoFocus={autoFocus}
        onFocus={() => { if (openOnFocus) show(); else onOpen(); }}
        onChange={(e) => { onChange(e.currentTarget.value); setHi(0); show(); }}
        onKeyDown={onKey}
      />
      <button
        className="combo-caret"
        type="button"
        tabIndex={-1}
        disabled={disabled}
        title="show branches"
        aria-label="show branches"
        // Never let the caret take focus off the field — a click that blurs the
        // input while opening its own popover reads as the list flickering.
        onMouseDown={(e) => e.preventDefault()}
        onClick={() => { if (open) { setOpen(false); return; } show(); inputRef.current?.focus(); }}
      >
        <Icons.ChevronDown size={12} />
      </button>
      {showPop && (
        // The notes sit OUTSIDE the listbox: a `role="listbox"` may only contain
        // options, and "loading branches…" is not one.
        <div className="combo-pop">
          {data === null && <div className="combo-note">loading branches…</div>}
          {data !== null && options.length === 0 && (
            <div className="combo-note">{data.branches.length ? "no branch matches" : "no other branches"}</div>
          )}
          <div role="listbox" id={listId} aria-label={ariaLabel}>
            {options.map((o, i) => (
              <button
                key={(o.create ? "new:" : "b:") + o.branch}
                id={optId(i)}
                className={"combo-item" + (i === hi ? " hi" : "") + (o.create ? " create" : "")}
                role="option"
                aria-selected={i === hi}
                // mousedown, not click: the input's blur would tear the row out
                // from under the click
                onMouseDown={(e) => { e.preventDefault(); commit(o.branch); }}
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
        </div>
      )}
    </div>
  );
}

// Status-bar branch switcher — the combo above plus the one thing it cannot
// know: whether the switch it just asked for actually landed (the text is
// cleared only if it did).
function BranchSwitcher({ repo, slug, onSwitch, onError }: {
  repo: string; slug: string;
  onSwitch: (branch: string) => Promise<boolean>;
  onError: (e: unknown) => void;
}) {
  const [text, setText] = useState("");
  const [data, setData] = useState<BranchList | null>(null);
  // In flight, so `data` is still null — without this the caret's first click
  // fires TWO `git for-each-ref`: opening the pop calls load(), and the focus()
  // that follows it synchronously re-enters onFocus before the first invoke has
  // resolved. Cleared in `finally` so a FAILED read is retried on the next open.
  const loading = useRef(false);

  // Reset when the place changes — a half-typed branch belongs to the place it
  // was typed in.
  useEffect(() => { setText(""); setData(null); }, [repo, slug]);

  // Lazy: one `git for-each-ref` when the switcher is first used, not on every
  // place selection.
  const load = () => {
    if (data || loading.current) return;
    loading.current = true;
    invoke<BranchList>("list_branches", { repo, slug })
      .then(setData)
      .catch(onError)
      .finally(() => { loading.current = false; });
  };

  return (
    <BranchCombo
      value={text}
      data={data}
      placeholder="switch branch…"
      exclude={data?.current}
      inputClass="switchto"
      ariaLabel="switch branch"
      testid="switch-branch"
      onChange={setText}
      onOpen={load}
      onCommit={async (b) => {
        if (!(await onSwitch(b))) return;
        setText("");
        // The list is now stale in the one way that shows: `current` still names
        // the branch we just LEFT, so reopening would hide it and offer the
        // branch we are on. Drop it and let the next open re-read.
        setData(null);
      }}
    />
  );
}

/** What is wrong with the folder a new worktree would land in.
 *
 *  NOT `nameProblem` — that one guards a PROJECT directory and rejects `/`,
 *  which would be wrong here: core slugifies the name exactly as it slugifies
 *  the branch, so `feat/x` is a fine name that becomes `feat-x`. What is left is
 *  what the filesystem and the app's own selection model refuse. */
function slugProblem(slug: string): string {
  if (!slug) return "";
  if (slug === "." || slug === "..") return "'.' and '..' are not names";
  if (slug.startsWith(".")) return "a name starting with '.' would make a hidden folder";
  if (slug === "(main)") return "'(main)' is the name of the main checkout";
  // eslint-disable-next-line no-control-regex -- control chars are exactly what this rejects
  if (/[\s\u0000-\u001f\u007f]/.test(slug)) return "a name cannot contain spaces or control characters";
  return "";
}

/** What `new` is about to DO, said before it does it. `blocking` marks the one
 *  case core refuses outright — Create is disabled on it rather than letting the
 *  op fail into the error banner. */
type Verdict = { tone: "info" | "warn" | "error"; text: string; blocking?: boolean; openSlug?: string };

/** "New worktree" — the `+` on a project header.
 *
 *  This was a card pinned to the TOP of the nav, which is not where the project
 *  you clicked lives: with several projects on screen the only thing tying the
 *  form to one of them was a line of 11px text, and it shoved the whole tree
 *  down for as long as it was up. A dialog carries the project in its title,
 *  dismisses by clicking away, and — the actual point — has room for the thing
 *  the card could never fit: what this is going to do. `new` is DWIM (branch
 *  exists → check it out, remote-only → track it, unknown → create it off a
 *  base, already in another worktree → reuse that worktree), and none of that
 *  was visible in three bare inputs whose base placeholder said "default: main"
 *  on a repo whose default base is `master`.
 *
 *  Module scope (CLAUDE.md): it owns three fields, and App re-renders on the 3s
 *  poll — defined inside App() it would remount and drop focus per keystroke. */
function NewPlaceDialog({
  project, prefix, places, unborn, initial, initialBase,
  onCreate, onClose, onOpenPlace, onInitialCommit, onError,
}: {
  project: string;
  prefix: string;
  /** This project's places. The live snapshot is the only thing that knows which
   *  worktree already holds a branch and which slugs are taken — no Rust rule is
   *  reimplemented here, it is read. */
  places: Place[];
  unborn: boolean;
  /** A REJECTED create, handed back so a typo costs one edit and not a retype. */
  initial: { branch: string; name: string; base: string } | null;
  /** ctx-menu "New worktree from this branch…" — a base, nothing else. */
  initialBase: string;
  onCreate: (branch: string, name: string, base: string) => void;
  onClose: () => void;
  onOpenPlace: (slug: string) => void;
  onInitialCommit: (repo: string) => Promise<void>;
  onError: (e: unknown) => void;
}) {
  const [branch, setBranch] = useState(initial?.branch ?? "");
  const [base, setBase] = useState(initial?.base || initialBase);
  const [name, setName] = useState(initial?.name ?? "");
  // Until this flips, the folder name MIRRORS the branch. Emptying the field
  // hands it back — which is what the hint under it says.
  const [touched, setTouched] = useState(!!initial?.name);
  const [data, setData] = useState<BranchList | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let alive = true;
    // "(main)" resolves to the main checkout (project.rs `place_dir`), which is
    // how a PROJECT-level branch list is asked for — there is no place yet.
    invoke<BranchList>("list_branches", { repo: project, slug: "(main)" })
      .then((d) => {
        if (!alive) return;
        setData(d);
        // Seed the base ONCE, and only into an empty field: the real default is
        // main → master → HEAD, which the old placeholder merely guessed at.
        setBase((b) => b || d.default_base);
      })
      .catch((e) => { if (alive) onError(e); });
    return () => { alive = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- one read per open
  }, [project]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape" && !busy) onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [busy, onClose]);

  // core strips a leading origin/ before anything else (ops.rs `strip_origin`)
  const b = branch.trim().replace(/^origin\//, "");
  const mirrored = b.replace(/\//g, "-");
  const typed = touched ? name.trim() : "";
  // ⚠ Send a name ONLY when it differs from what core would derive anyway. WITH
  // a name core refuses a branch that already lives in another worktree; without
  // one it reuses that worktree. A mirrored name sent unconditionally would
  // silently turn the second case into the first (ops.rs:417-428).
  const sendName = typed && typed !== mirrored ? typed : "";
  const shownName = touched ? name : mirrored;
  const slug = (sendName || b).replace(/\//g, "-");
  const baseShown = base.trim() || data?.default_base || "";

  // `wt_for_branch` only looks under `.worktrees/`, so the main checkout being
  // on the branch is NOT a holder — and must not be reported as one.
  const holder = places.find((p) => !p.is_main && p.branch === b);
  const taken = places.find((p) => p.slug === slug);
  const known = !!data?.branches.includes(b);
  const problem = slugProblem(slug);

  const verdict: Verdict | null =
    !b ? null
    : problem ? { tone: "error", text: problem, blocking: true }
    : holder && sendName
      ? {
          tone: "error", blocking: true, openSlug: holder.slug,
          text: `'${b}' is already checked out in ${holder.slug}, so it cannot also go in ${slug}. Clear the folder name to reuse ${holder.slug}.`,
        }
    : holder
      ? { tone: "warn", openSlug: holder.slug, text: `${holder.slug} is already on '${b}' — creating here reuses that worktree.` }
    : taken && taken.branch === b
      ? { tone: "warn", text: `${slug} already exists and is already on '${b}' — it will be reused as-is.` }
    : taken
      ? { tone: "warn", text: `${slug} already exists on '${taken.branch ?? "?"}' — it will be switched to '${b}'.` }
    // ⚠ Only the last two lines depend on the branch LIST, and until it lands
    // `known` is false — which reads as "this branch is new" for a branch that
    // exists. The mock answers in a microtask so that window is invisible there;
    // the real read is a `git for-each-ref` fan-out. Say "still reading" instead
    // of asserting the wrong one of the two (CLAUDE.md's mock-timing rule).
    : data === null
      ? { tone: "info", text: `reading this project's branches…` }
    : known
      ? { tone: "info", text: `'${b}' already exists — it will be checked out here.` }
      : { tone: "info", text: `'${b}' will be created off ${baseShown || "the default base"}.` };

  // Base is consulted ONLY when the branch has to be created (ops.rs:511).
  // Leaving it live in the other cases invites a value that is then ignored.
  // An EMPTY branch counts as used: nothing has been decided yet, and a field
  // greyed out before you have typed anything reads as broken, not as inert.
  const baseUsed = !b || (!known && !holder && !taken);
  const ready = !!b && !verdict?.blocking && !busy;
  const submit = () => { if (ready) onCreate(b, sendName, base.trim()); };

  return (
    <div className="scrim scrim-center" onClick={() => !busy && onClose()}>
      <div className="sync-modal" role="dialog" aria-label="New worktree" data-testid="new-place-dialog"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => { if (e.key === "Enter") submit(); }}>
        <header className="sync-h">
          <b>New worktree</b>
          <span className="sync-hub">{basename(project)}</span>
        </header>

        {unborn ? (
          // `new` cannot work here at all — git will not branch off an unborn
          // HEAD — so the fields give way to the one act that unblocks them.
          <>
            <div className="sync-body nw-body">
              <div className="np-path" data-testid="nw-unborn">
                <code>{project}</code> has no commits yet. git can't create a worktree off an
                unborn HEAD — make the first commit, then create worktrees.
              </div>
            </div>
            <footer className="sync-foot">
              <button className="ctrl" onClick={onClose} disabled={busy}>Cancel</button>
              <button className="enter-btn" data-testid="nw-commit" disabled={busy}
                onClick={async () => { setBusy(true); await onInitialCommit(project); setBusy(false); }}>
                {busy ? "Committing…" : "Create initial commit"}
              </button>
            </footer>
          </>
        ) : (
          <>
            <div className="sync-body nw-body">
              <label className="np-field">
                <span className="np-label">Branch</span>
                <BranchCombo
                  value={branch}
                  data={data}
                  placeholder="feat/checkout"
                  inputClass="np-input"
                  ariaLabel="branch for the new worktree"
                  testid="nw-branch"
                  drop="down"
                  openOnFocus={false}
                  autoFocus
                  onChange={setBranch}
                  onOpen={() => {}}
                  onCommit={setBranch}
                />
              </label>
              {verdict && (
                <div className={"nw-verdict " + verdict.tone} data-testid="nw-verdict">
                  <span>{verdict.text}</span>
                  {verdict.openSlug && (
                    <button className="ctrl sm" data-testid="nw-open-holder"
                      onClick={() => onOpenPlace(verdict.openSlug!)}>Open {verdict.openSlug}</button>
                  )}
                </div>
              )}

              <label className="np-field">
                <span className="np-label">Base</span>
                <BranchCombo
                  value={base}
                  data={data}
                  placeholder={data?.default_base ?? "main"}
                  inputClass="np-input"
                  ariaLabel="base branch"
                  testid="nw-base"
                  drop="down"
                  openOnFocus={false}
                  allowCreate={false}
                  disabled={!baseUsed}
                  onChange={setBase}
                  onOpen={() => {}}
                  onCommit={setBase}
                />
                <span className="np-hint">
                  {baseUsed
                    ? "start point for the new branch — a branch, tag or commit"
                    : "only used when the branch has to be created"}
                </span>
              </label>

              <label className="np-field">
                <span className="np-label">Folder name</span>
                <input className="np-input" data-testid="nw-name" value={shownName} disabled={busy}
                  spellCheck={false} autoCapitalize="off" autoCorrect="off"
                  onChange={(e) => {
                    const v = e.currentTarget.value;
                    setName(v);
                    // Emptying it re-links the mirror; that is the way back.
                    setTouched(v !== "");
                  }} />
                <span className="np-hint">
                  {touched ? "clear this field to follow the branch again" : "follows the branch — edit to pin it"}
                </span>
              </label>

              {/* Suppressed while blocked: a path promised next to the reason
                  it cannot be written is the one line here that would be a lie. */}
              <div className="np-path" data-testid="nw-preview">
                {verdict?.blocking ? (
                  <i>nothing will be created until the above is resolved</i>
                ) : (
                  <>
                    will create <code>{project}/.worktrees/{slug || "…"}</code>
                    <br />
                    tmux session <code>{`${prefix}-${slug || "…"}`.replace(/\./g, "-")}</code>
                  </>
                )}
              </div>
            </div>
            <footer className="sync-foot">
              <button className="ctrl" onClick={onClose} disabled={busy}>Cancel</button>
              <button className="enter-btn" data-testid="nw-create" disabled={!ready} onClick={submit}>Create</button>
            </footer>
          </>
        )}
      </div>
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

// ── sync (courier sync of a project through a mounted hub) ──────────────────
// Backend: lib.rs `sync_status` / `sync_preview` / `sync_apply` over
// worktrees_core::sync. A preview you answer, a live bar while it runs, and a
// result you can read. The bar is rsync's own `--info=progress2,name1` relayed
// over a Channel (the `term_open` mechanism): an 8.5G first push is minutes
// long, and a button frozen on "Pushing…" for four of them reads as a hang.

type SyncStatusView = {
  scope: string;
  name: string | null;
  local_root: string | null;
  hub: string | null;
  hub_error: string | null;
  branch: string | null;
  dirty: number | null;
  pushed_at: number | null;
  pushed_host: string | null;
  /** Non-null ⇒ this checkout arrived on a hub. Every sync item is disabled and
   *  this message is the reason. */
  hub_copy: string | null;
};
type SyncDir = "push" | "pull";
/** One project the hub has been pushed to, from any machine (core's
 *  `ManifestBrief`). `local_root` is where it lives — on the Mac that pushed it,
 *  and, because both Macs keep the same paths, where it will land here. */
type HubProject = { name: string; local_root: string; pushed_at: number; host: string };
/** `sync_hub_list` — the hub's own status, with no project involved. The one
 *  sync call a machine with an EMPTY workspace can make. */
type HubList = { hub: string | null; hub_error: string | null; projects: HubProject[] };
/** An import in flight: the project being adopted and where it will land. Held
 *  by App (not derived from the preview) so the modal can name the destination
 *  from the first frame — before the preview returns, and even if it fails. */
type Adopt = { name: string; dest: string };
type SyncPreview = {
  name: string;
  direction: SyncDir;
  hub: string;
  src: string;
  dst: string;
  plan: { sends: number; deletes: number; delete_paths: string[] };
  skipped: [string, number][];
  /** rsync 3.x. Doubles as "this transfer can report progress": the openrsync
   *  fallback has neither the hiding report nor `--info=progress2`, so the bar
   *  stays indeterminate there rather than pretending to know. */
  skipped_available: boolean;
  live_sessions: string[];
  with_sessions: boolean;
  /** The user's registered rebuild for this project, or null. Only a pull can
   *  run it, and only if asked. */
  install_cmd: string | null;
  /** This machine has no tree for the project yet — `dst` is about to become
   *  one. Set only for an import (adoption). */
  adopting: boolean;
  warnings: string[];
};
/** One sample from the Channel while an apply runs (core's `SyncProgress`). */
type SyncProgressEvt = { percent: number | null; rate: string | null; current: string | null };

/** Middle-truncate a path: the head says which project, the tail says which
 *  file, and the modal's width must not move when either grows. */
function midTrunc(s: string, max = 52): string {
  if (s.length <= max) return s;
  const head = Math.ceil((max - 1) / 2);
  return `${s.slice(0, head)}…${s.slice(-(max - 1 - head))}`;
}

/** The apply's live state. Determinate as soon as rsync has reported a
 *  percentage AND this rsync can report one at all; indeterminate before the
 *  first event and for the whole run on openrsync — an empty bar that never
 *  moves is a worse lie than an honest "working". */
function SyncBar({ progress, determinate, verb }: {
  progress: SyncProgressEvt | null;
  determinate: boolean;
  verb: string;
}) {
  const pct = determinate && progress?.percent != null
    ? Math.max(0, Math.min(100, Math.round(progress.percent)))
    : null;
  return (
    <div className="sync-prog" data-testid="sync-prog">
      {pct === null ? (
        <>
          <div className="sync-bar indet" data-testid="sync-bar-indet"><div className="sync-bar-fill" /></div>
          <div className="sync-prog-line"><span>{verb}ing…</span></div>
        </>
      ) : (
        <>
          <div className="sync-bar" data-testid="sync-bar">
            <div className="sync-bar-fill" data-testid="sync-bar-fill" data-pct={pct} style={{ width: `${pct}%` }} />
          </div>
          <div className="sync-prog-line">
            <span data-testid="sync-pct" data-pct={pct}>{pct}%</span>
            {progress?.rate && <span className="sync-rate" data-testid="sync-rate">{progress.rate}</span>}
          </div>
        </>
      )}
      {progress?.current && (
        <div className="sync-file" data-testid="sync-file" title={progress.current}>{midTrunc(progress.current)}</div>
      )}
    </div>
  );
}

/** The two sync actions, shared VERBATIM by the ⇄ popover and the project
 *  context menu — one definition, so the two surfaces cannot come to disagree
 *  about what the hub is or when the actions are unusable. */
function SyncMenuItems({ status, root, onOpen }: {
  status: SyncStatusView | undefined;
  root: string;
  onOpen: (root: string, dir: SyncDir) => void;
}) {
  // The hub's basename is the name the user knows the drive by. While the status
  // is still in flight the items say "hub" and stay LIVE: the preview is where a
  // missing hub would be reported anyway, and a menu that greys itself out for a
  // moment reads as broken.
  const hub = status?.hub ? basename(status.hub) : "hub";
  const why = status?.hub_copy ?? status?.hub_error ?? "";
  const off = !!status && (!!status.hub_copy || !status.hub);
  return (
    <>
      <button className="pop-item" disabled={off} title={why} data-testid="sync-push"
        onClick={() => onOpen(root, "push")}>Push to {hub}…</button>
      <button className="pop-item" disabled={off} title={why} data-testid="sync-pull"
        onClick={() => onOpen(root, "pull")}>Pull from {hub}…</button>
    </>
  );
}

/** The ⇄ popover: where this project syncs, the two actions, and when it last
 *  went anywhere. The last-push line is the question the drive itself cannot
 *  answer — "did I already push this from the other Mac?" */
function SyncPopover({ status, root, onOpen }: {
  status: SyncStatusView | undefined;
  root: string;
  onOpen: (root: string, dir: SyncDir) => void;
}) {
  const head = status?.hub_copy ? "hub copy" : status?.hub ? basename(status.hub) : status ? "no hub" : "sync";
  const why = status?.hub_copy ?? status?.hub_error ?? "";
  const when = status?.pushed_at ? ago(status.pushed_at) : "";
  return (
    <>
      <div className="pop-hint">{head}</div>
      {why && <div className="pop-note warn" data-testid="sync-pop-why">{why}</div>}
      <SyncMenuItems status={status} root={root} onOpen={onOpen} />
      <div className="pop-note" data-testid="sync-pop-last">
        {status?.pushed_at
          ? `last push: ${when === "now" ? "just now" : `${when} ago`} from ${status.pushed_host || "?"}`
          : "never pushed"}
      </div>
    </>
  );
}

/** The name rule, mirrored from `lib.rs::valid_project_name` for the inline
 *  hint. The BACKEND decides — it re-validates every field it is handed — so a
 *  drift here can only make this message worse, never let a bad name through. */
function nameProblem(n: string): string {
  if (!n) return "";
  if (n === "." || n === "..") return "'.' and '..' are not names";
  if (n.startsWith(".")) return "a name starting with '.' would make a hidden folder";
  if (n.includes("/")) return "a name cannot contain '/' — the folder above it goes in Location";
  // eslint-disable-next-line no-control-regex -- control chars are exactly what this rejects
  if (/[\s\u0000-\u001f\u007f]/.test(n)) return "a name cannot contain spaces or control characters";
  return "";
}

/** "New project…" — a name, a place to put it, and the path that makes.
 *
 *  A native folder picker cannot express this: it can only choose something
 *  that ALREADY exists, so creating meant making the folder in Finder first and
 *  then pointing the app at it. Here the path is assembled in front of you and
 *  the backend creates it, `git init`s it and adds it in one act.
 *
 *  Module scope (CLAUDE.md): this component owns two text inputs — inside App()
 *  it would be a new identity on every 3s refresh tick, and typing would lose
 *  both its state and the focus ring. */
function NewProjectDialog({ defaultLocation, busy, error, onBrowse, onCreate, onClose }: {
  defaultLocation: string;
  busy: boolean;
  /** The backend's refusal, verbatim (exists / not a name / mkdir failed). */
  error: string;
  /** Browse… fills the FIELD rather than submitting: the pick is one way to
   *  answer the question, not a second way to ask it. */
  onBrowse: () => Promise<string | null>;
  onCreate: (location: string, name: string) => void;
  onClose: () => void;
}) {
  const [name, setName] = useState("");
  const [loc, setLoc] = useState(defaultLocation);
  const nameRef = useRef<HTMLInputElement | null>(null);
  useEffect(() => { nameRef.current?.focus(); }, []);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape" && !busy) onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [busy, onClose]);
  const trimmedName = name.trim();
  const trimmedLoc = loc.trim().replace(/\/+$/, "");
  const problem = nameProblem(trimmedName);
  const ready = !!trimmedName && !!trimmedLoc && !problem && !busy;
  const full = `${trimmedLoc || "…"}/${trimmedName || "…"}`;
  const submit = () => { if (ready) onCreate(trimmedLoc, trimmedName); };
  return (
    <div className="scrim scrim-center" onClick={() => !busy && onClose()}>
      <div className="sync-modal" role="dialog" aria-label="New project" data-testid="new-project-dialog"
        onClick={(e) => e.stopPropagation()}>
        <header className="sync-h"><b>New project</b></header>
        <div className="sync-body">
          <label className="np-field">
            <span className="np-label">Project name</span>
            <input ref={nameRef} className="np-input" data-testid="np-name" value={name} disabled={busy}
              placeholder="my-project" spellCheck={false} autoCapitalize="off" autoCorrect="off"
              onChange={(e) => setName(e.currentTarget.value)}
              onKeyDown={(e) => { if (e.key === "Enter") submit(); }} />
          </label>
          <label className="np-field">
            <span className="np-label">Location</span>
            <div className="np-row">
              <input className="np-input" data-testid="np-location" value={loc} disabled={busy}
                spellCheck={false} autoCapitalize="off" autoCorrect="off"
                onChange={(e) => setLoc(e.currentTarget.value)}
                onKeyDown={(e) => { if (e.key === "Enter") submit(); }} />
              <button className="ctrl" data-testid="np-browse" disabled={busy}
                onClick={async () => { const d = await onBrowse(); if (d) setLoc(d); }}>Browse…</button>
            </div>
          </label>
          {/* The path is the point: two fields, one folder, said out loud
              before anything is written. */}
          <div className="np-path" data-testid="np-path">
            will create <code>{full}</code>
          </div>
          {problem && <div className="sync-err" data-testid="np-error">{problem}</div>}
          {!problem && error && <div className="sync-err" data-testid="np-error">{error}</div>}
        </div>
        <footer className="sync-foot">
          <button className="ctrl" onClick={onClose} disabled={busy}>Cancel</button>
          <button className="enter-btn" data-testid="np-create" disabled={!ready} onClick={submit}>
            {busy ? "Creating…" : "Create"}
          </button>
        </footer>
      </div>
    </div>
  );
}

/** "Import from hub…" — the workspace-level picker.
 *
 *  Every other sync surface hangs off a project ROW, which a project that is
 *  not in the workspace does not have. That is exactly the state a brand-new
 *  Mac is in: the drive holds everything and the app can see none of it. This
 *  lists what the hub has, says where each one would land, and hands the pick
 *  to the same preview/confirm modal every other sync goes through.
 *
 *  Module scope (CLAUDE.md) — App re-renders on every 3s refresh tick. */
function ImportPicker({ list, loading, error, known, onPick, onClose }: {
  list: HubList | null;
  loading: boolean;
  error: string;
  /** Roots already in the workspace — those rows are dead, with the reason on
   *  them: importing one would mirror --delete over a tree the user is using. */
  known: Set<string>;
  onPick: (p: HubProject) => void;
  onClose: () => void;
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  const projects = list?.projects ?? [];
  return (
    <div className="scrim scrim-center" onClick={onClose}>
      <div className="sync-modal" role="dialog" aria-label="Import from hub" data-testid="import-picker"
        onClick={(e) => e.stopPropagation()}>
        <header className="sync-h">
          <b>Import from hub</b>
          {list?.hub && <span className="sync-hub">{list.hub}</span>}
        </header>
        <div className="sync-body">
          {loading && <div className="sync-pending" data-testid="import-loading">Reading the hub…</div>}
          {error && <div className="sync-err" data-testid="import-error">{error}</div>}
          {/* No hub is not an error — it is a drive that is not plugged in. It
              reads as the reason, dimmed, with nothing to click. */}
          {list && !list.hub && (
            <div className="sync-skipped" data-testid="import-nohub">
              {list.hub_error || "no hub found"}
            </div>
          )}
          {list?.hub && projects.length === 0 && (
            <div className="sync-skipped" data-testid="import-empty">
              nothing has been pushed to this hub yet — run a push on the other Mac first
            </div>
          )}
          {projects.map((p) => {
            const here = known.has(p.local_root);
            return (
              <button
                key={p.name}
                className="import-row"
                data-testid="import-row"
                data-name={p.name}
                data-known={here ? "1" : "0"}
                disabled={here}
                title={here ? "already in workspace" : `import into ${p.local_root}`}
                onClick={() => onPick(p)}
              >
                <span className="import-name">{p.name}</span>
                {here && <span className="import-tag" data-testid="import-known">already in workspace</span>}
                <span className="import-dest">{p.local_root}</span>
                <span className="import-when">
                  {p.pushed_at
                    ? `pushed ${ago(p.pushed_at) === "now" ? "just now" : `${ago(p.pushed_at)} ago`} from ${p.host || "?"}`
                    : "never pushed"}
                </span>
              </button>
            );
          })}
        </div>
        <footer className="sync-foot">
          <button className="ctrl" onClick={onClose}>Close</button>
        </footer>
      </div>
    </div>
  );
}

// Module scope with props (CLAUDE.md): a component defined inside App() is a new
// identity every render, so the checkbox below would lose its state — and the
// modal re-renders on every refresh tick while an apply runs.
function SyncModal({ dir, project, adopt, preview, loading, busy, error, live, done, sessions, onSessions, install, onInstall, progress, onConfirm, onClose }: {
  dir: SyncDir;
  project: string;
  /** Non-null ⇒ this is an IMPORT: the project has no tree here, and `dest` is
   *  the path the transfer is about to create. Known from the picker, so it is
   *  on screen before the preview returns. */
  adopt: Adopt | null;
  preview: SyncPreview | null;
  loading: boolean;
  busy: boolean;
  error: string;
  /** Live tmux sessions in the destination — from the preview, then REPLACED by
   *  the fresher list an apply answers with when one appeared in between. */
  live: string[];
  done: string;
  sessions: boolean;
  onSessions: (on: boolean) => void;
  /** Pull only: run the user's registered rebuild afterwards. */
  install: boolean;
  onInstall: (on: boolean) => void;
  /** Latest Channel sample, or null before the first one. Owned by App so a
   *  refresh tick (every 3s, mid-transfer) cannot reset the bar — this
   *  component is at module scope for the same reason. */
  progress: SyncProgressEvt | null;
  onConfirm: () => void;
  onClose: () => void;
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // Esc cannot cancel a transfer that is already running — the rsync is not
      // ours to stop, and a modal that vanishes mid-apply hides what happened.
      if (e.key === "Escape" && !busy) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [busy, onClose]);

  const plan = preview?.plan;
  const deletes = plan?.deletes ?? 0;
  const verb = adopt ? "Import" : dir === "push" ? "Push" : "Pull";
  return (
    <div className="scrim scrim-center" onClick={() => !busy && onClose()}>
      <div className="sync-modal" role="dialog" aria-label={adopt ? `Import ${adopt.name}` : `Sync ${dir}`}
        data-testid={adopt ? "import-modal" : "sync-modal"} onClick={(e) => e.stopPropagation()}>
        <header className="sync-h">
          <b>{adopt ? `Import ${adopt.name}` : `Sync ${dir} — ${project}`}</b>
          {preview && <span className="sync-hub">{preview.hub}</span>}
        </header>
        <div className="sync-body">
          {/* The destination is the WHOLE of what an import asks consent for:
              a folder this Mac has never had, about to be created and filled
              from a drive. It leads the modal, it is on screen from the first
              frame (the picker knew it), and the preview only confirms it. */}
          {adopt && (
            <div className="sync-dest" data-testid="sync-dest">
              <span className="sync-dest-l">will be created at</span>
              <code className="sync-dest-p" data-testid="sync-dest-path">{preview?.dst || adopt.dest}</code>
            </div>
          )}
          {loading && <div className="sync-pending" data-testid="sync-pending">Previewing…</div>}
          {error && <div className="sync-err" data-testid="sync-error">{error}</div>}
          {done && <div className="sync-done" data-testid="sync-done">{done}</div>}
          {busy && (
            <SyncBar progress={progress} determinate={!!preview?.skipped_available} verb={verb} />
          )}
          {preview && !done && (
            <>
              <div className="sync-route" title={`${preview.src} → ${preview.dst}`}>
                <span>{preview.src}</span>
                <span className="sync-arrow">→</span>
                <span>{preview.dst}</span>
              </div>
              <div className="sync-counts" data-testid="sync-counts">
                <b>{plan!.sends}</b> to send/update
                {deletes > 0 && <> · <b className="sync-del">{deletes}</b> to delete</>}
              </div>
              {plan!.delete_paths.length > 0 && (
                <ul className="sync-dels">
                  {plan!.delete_paths.map((p) => <li key={p}>{p}</li>)}
                  {deletes > plan!.delete_paths.length && (
                    <li className="sync-more">… and {deletes - plan!.delete_paths.length} more</li>
                  )}
                </ul>
              )}
              <div className="sync-skipped">
                {!preview.skipped_available
                  ? "skipped report unavailable (openrsync)"
                  : preview.skipped.length === 0
                    ? "excluded: nothing matched"
                    : `excluded (rebuild locally): ${preview.skipped.map(([p, n]) => `${p} ×${n}`).join("  ")}`}
              </div>
              {preview.warnings.map((w) => <div className="sync-warn" key={w}>{w}</div>)}
              {live.length > 0 && (
                <div className="sync-live" data-testid="sync-live">
                  <b>{live.length} live tmux session{live.length > 1 ? "s" : ""} in the destination</b>
                  <ul>{live.map((s) => <li key={s}>● {s}</li>)}</ul>
                  A pull mirrors with --delete over the tree they are using.
                </div>
              )}
              <label className="sync-opt">
                <input type="checkbox" checked={sessions} disabled={busy}
                  onChange={(e) => onSessions(e.currentTarget.checked)} />
                Include Claude sessions (additive — never deletes the other Mac's history)
              </label>
              {/* Only when a rebuild is REGISTERED, and it names what it would
                  run: an offer to execute something the user cannot see is the
                  shape ADR 0001 exists to prevent. The command is the user's own
                  config (never the hub manifest's hint, which rode in on the
                  drive and is printed only). */}
              {dir === "pull" && preview.install_cmd && (
                <label className="sync-opt" data-testid="sync-install-opt">
                  <input type="checkbox" data-testid="sync-install" checked={install} disabled={busy}
                    onChange={(e) => onInstall(e.currentTarget.checked)} />
                  Run rebuild after pull: <code className="sync-cmd">{preview.install_cmd}</code>
                </label>
              )}
            </>
          )}
        </div>
        <footer className="sync-foot">
          <button className="ctrl" onClick={onClose} disabled={busy}>{done ? "Close" : "Cancel"}</button>
          {!done && (
            <button
              className={"enter-btn" + (deletes > 0 ? " danger" : "")}
              data-testid="sync-go"
              disabled={busy || loading || !preview}
              onClick={onConfirm}
            >
              {busy ? `${verb}ing…` : verb}
            </button>
          )}
        </footer>
      </div>
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
  // ── nav drag & drop (see navdrag.ts for the gesture, dnd.ts for the rules) ──
  const navScrollRef = useRef<HTMLDivElement | null>(null);
  /** A row that has just been moved by a drag, lit for a moment. It is the only
   *  honest answer when a drop lands somewhere other than the group it was
   *  dropped on — which the derived tiers make routine. */
  const [flash, setFlash] = useState<{ repo: string; slug: string } | null>(null);
  /** A drag can change lifecycle with one slip of the wrist, and the row it
   *  moves may leave the visible tree entirely (a hidden tier, a collapsed
   *  group). The way back has to be one click, not a hunt. */
  const [undo, setUndo] = useState<
    { text: string; run: () => void } | null
  >(null);
  const [whatsNew, setWhatsNew] = useState<{ version: string; notes: string; manual?: boolean } | null>(null);
  const [projSheet, setProjSheet] = useState<string | null>(null);
  // ── sync ──
  // Cached per project, refreshed when the context menu OPENS (one cheap
  // backend call) and again after an apply, so the menu can name the hub it
  // would use and grey itself out when there is none.
  const [syncStatus, setSyncStatus] = useState<Record<string, SyncStatusView>>({});
  /** `adopt` non-null ⇒ an IMPORT: there is no local root yet, so `root` is ""
   *  and the backend is addressed by hub NAME instead. */
  const [sync, setSync] = useState<{ root: string; dir: SyncDir; adopt: Adopt | null } | null>(null);
  const [syncPrev, setSyncPrev] = useState<SyncPreview | null>(null);
  const [syncLoading, setSyncLoading] = useState(false);
  const [syncBusy, setSyncBusy] = useState(false);
  const [syncErr, setSyncErr] = useState("");
  const [syncDone, setSyncDone] = useState("");
  /** Live sessions the user has been SHOWN. Seeded from the preview, replaced by
   *  the fresher list an apply answers with — confirming is only consent to what
   *  is on screen. */
  const [syncLive, setSyncLive] = useState<string[]>([]);
  /** Rebuild after a pull. NOT persisted, unlike the sessions checkbox: running
   *  a build command is an act with a cost, and consent to it belongs to the one
   *  transfer you ticked it for. Reset on every open. */
  const [syncInstall, setSyncInstall] = useState(false);
  /** The last Channel sample of a running apply. Lives HERE, not in the modal:
   *  the modal is re-rendered by every 3s refresh tick while a transfer runs, and
   *  state that far down would be at the mercy of any remount. */
  const [syncProg, setSyncProg] = useState<SyncProgressEvt | null>(null);
  /** The New-project dialog (name + location). Separate from `initAsk`, which
   *  is the OFFER made when an added folder turns out not to be a repo — this
   *  one is the deliberate act, and it creates the folder too. */
  const [npOpen, setNpOpen] = useState(false);
  const [npBusy, setNpBusy] = useState(false);
  const [npErr, setNpErr] = useState("");
  /** The import picker (workspace-level, no project). Its own small state: the
   *  hub listing is a different question from any project's sync status, and a
   *  machine with an empty workspace has no project to hang it off. */
  const [importOpen, setImportOpen] = useState(false);
  const [importList, setImportList] = useState<HubList | null>(null);
  const [importLoading, setImportLoading] = useState(false);
  const [importErr, setImportErr] = useState("");
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

  // ── per-place panel state ────────────────────────────────────────────────
  // A space owns its dock: what is open, which tab, how wide. `settings` holds
  // the LAST-USED values (the seed a never-visited place inherits), and
  // `place_panels[key]` overrides them for a place you have set up before.
  // `eff` is what the UI must READ everywhere the dock is concerned — reading
  // `settings.dock_*` directly is the bug this indirection exists to prevent.
  const selKey = sel ? placeKey(sel.repo, sel.slug) : null;
  const eff = panelsFor(settings, selKey);
  // The selected place, for callbacks that must stay dependency-free (see
  // `toggleDock`, registered once in the keydown effect).
  const selRef = useRef(sel);
  selRef.current = sel;

  // rename-in-place for the header name. Transient, and reset on a place switch
  // like dockFile below — a half-typed name must not follow you somewhere else.
  const [renaming, setRenaming] = useState(false);

  // right dock: which file the Files tab is viewing (null = none). Reset per place.
  const [dockFile, setDockFile] = useState<string | null>(null);
  // Reading mode (⌘⇧E): the open file takes over the main pane. Closed by a
  // place switch or by the file going away — an overlay with nothing under it
  // would hide the terminal for no reason.
  const [reading, setReading] = useState(false);
  useEffect(() => { setDockFile(null); setReading(false); setRenaming(false); }, [sel?.repo, sel?.slug]);
  useEffect(() => { if (!dockFile) setReading(false); }, [dockFile]);
  // ⌘J / the rail says "hide files" — leaving a full-pane reader behind would
  // make that a lie. Same for flipping the dock to the Terminal tab.
  const filesDockShown = eff.dock_open && eff.dock_tab === "files";
  useEffect(() => { if (!filesDockShown) setReading(false); }, [filesDockShown]);
  // ⌘⇧T bumps this → the dock's Terminal tab adds a shell (if mounted/visible).
  const [newTermToken, setNewTermToken] = useState(0);
  // ⌘F — which surface owns the find bar. Exactly ONE at a time app-wide: the
  // file side paints through the global CSS highlight registry, so two live
  // bars would overwrite each other's hits, and a bar left open behind the
  // reader would be highlighting a file nobody can see.
  const [findOn, setFindOn] = useState<null | "main" | "dock" | "read">(null);
  // bumps on every ⌘F so a second press re-focuses and selects the field
  const [findToken, setFindToken] = useState(0);
  const closeFind = useCallback(() => setFindOn(null), []);
  // Which surface the user last interacted with — see findTarget.
  //
  // Deliberately NOT `focusin`: every terminal calls `term.focus()` when it
  // mounts, so a focus-based rule is decided by mount order rather than by the
  // user — open the dock's Terminal tab and it takes ⌘F away from the pane you
  // are actually working in. Pointer and key events are things the user did.
  // Capture phase so the mark is already right when the app's own keydown
  // handler (a bubble-phase listener on the same window) reads it.
  const lastSurface = useRef<Surface>("main");
  useEffect(() => {
    const mark = (e: Event) => {
      const t = e.target;
      if (!(t instanceof Element)) return;
      if (t.closest(".reading")) lastSurface.current = "read";
      else if (t.closest(".dock")) lastSurface.current = "dock";
      else if (t.closest(".main")) lastSurface.current = "main";
    };
    window.addEventListener("pointerdown", mark, true);
    window.addEventListener("keydown", mark, true);
    return () => {
      window.removeEventListener("pointerdown", mark, true);
      window.removeEventListener("keydown", mark, true);
    };
  }, []);
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
  // Start-order ticket for the read below. Eight call sites fire `refresh()` —
  // the mount effect, mutate, runCmd, the "places:changed" listener, the
  // visible-edge catch-up, reloadFiles, the tmux re-check and onConfigWritten —
  // none of them awaiting each other, so several `list_workspace` sweeps can be
  // in flight at once and they do NOT resolve in the order they started (each
  // is seconds of git fan-out whose duration
  // depends on which project it hit). Without a ticket the LAST to RESOLVE wins:
  // a sweep that snapshotted the store BEFORE a declared write lands after the
  // write's own refresh and puts the pre-write value back. That is the renamed
  // place reverting to its slug in the nav.
  const refreshSeq = useRef(0);
  // The newest ticket that has already SPOKEN. The test is `older than what has
  // landed`, not `not the newest issued`: a read only has to outrank what is on
  // screen, and gating on the newest ISSUED ticket instead would discard every
  // read that completes while another is in flight — during a burst of refreshes
  // (the Refresh button, a run of edits, a workspace whose sweep outlasts the
  // next trigger) that is every one of them, and the tree stops updating until
  // the burst ends. Each read carries a view at least as new as the one before
  // it, so letting them land in ticket order is both safe and live.
  const refreshDone = useRef(0);
  const refresh = useCallback(async () => {
    const seq = ++refreshSeq.current;
    try {
      const w = await invoke<Workspace>("list_workspace");
      // A read that a NEWER one already spoke for is history, not news — its
      // snapshot predates that one's by construction. Dropping it here is what
      // stops a pre-write sweep from putting the old value back.
      if (seq < refreshDone.current) return;
      refreshDone.current = seq;
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
      // Same rule for failures: a superseded read must not banner an error that
      // a newer, successful sweep has already disproved. A FAILED read does not
      // advance `refreshDone` — it spoke for nothing.
      if (seq < refreshDone.current) return;
      refreshErr.current = String(e); // same text fail() shows
      fail(e);
    }
  }, []);
  /** The ONLY other way `ws` may be replaced. `lastSnap` is the dedupe's record
   * of what `ws` currently holds; a bare `setWs` leaves the two describing
   * different things, and a later refresh that happens to match `lastSnap` would
   * then bail while `ws` still held the stale value — permanently, because the
   * snapshot is stable at that point. Commands that mutate the workspace and get
   * a fresh one back go through here.
   *
   * It also OUTRANKS every read still in flight. The workspace here came back
   * from the command that changed things, so it is newer than any sweep that
   * started before it — without the bump, a read issued before `remove_project`
   * resolves after it and puts the removed project back. */
  const commitWs = useCallback((w: Workspace) => {
    refreshDone.current = ++refreshSeq.current;
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
  // THE clock. Row age and sort key for every list a user reads — the nav tree,
  // the Recent lens, the home Resume list, ⌘K — so a row's printed age is the
  // reason it sits where it does. When something HAPPENED here: Claude work or a
  // commit, never an open. `last_opened_epoch` is deliberately excluded so
  // clicking a row neither resets its clock to "now" nor reshuffles the tree
  // (⌘K used to rank by opens, which is why the palette and the tree behind it
  // disagreed about what you touched last). The one place opens still decide is
  // the restore-on-launch target — see `usedEpoch`.
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

  /** The functional form exists for patches that must be built from OTHER
   *  keys' current value — undo restoring one repo's `manual_order` entry.
   *  Building such a patch from a captured `settings` bakes in whatever that
   *  closure saw, and a reorder made in between is silently reverted (the
   *  saved blob is whole, so the loss reaches disk). Function patches never
   *  touch the terminal keys, which is why the term-version bump below only
   *  inspects object patches. */
  const updateSettings = (patch: Partial<Settings> | ((prev: Settings) => Partial<Settings>)) => {
    setSettings((prev) => {
      const p = typeof patch === "function" ? patch(prev) : patch;
      // resizing a hidden nav is dead UI — bring it back for live preview
      const auto = p.nav_width !== undefined && prev.nav_collapsed ? { nav_collapsed: false } : null;
      const next = { ...prev, ...p, ...auto };
      applySettings(next);
      if (hydrated.current) saveSettings(next);
      else Object.assign(preHydration.current, p, auto);
      return next;
    });
    // theme changes the terminal colors too (xterm reads CSS vars once per version)
    if (typeof patch !== "function" &&
        (patch.term_family !== undefined || patch.term_size !== undefined || patch.theme !== undefined ||
        patch.theme_light !== undefined || patch.theme_dark !== undefined))
      setTermVersion((v) => v + 1);
  };

  /** Write dock panel state for the SELECTED place (and to the globals, which
   *  keep tracking "last used" so the next unvisited place inherits it).
   *
   *  Always stores the FULL record, never the patch it was handed. A partial
   *  entry falls through to the globals for whatever it omits, which is exactly
   *  how another place's choice bleeds in: open the dock in A (storing only
   *  `dock_open`), flip to Terminal in B (moving the global `dock_tab`), come
   *  back to A and it is open on B's tab. That is the jarring switch this
   *  feature exists to remove.
   *
   *  Stable (refs only, functional setState) so `toggleDock` can stay
   *  dependency-free for the keydown effect. */
  const updatePanels = useCallback((patch: Partial<PlacePanels> | ((cur: PlacePanels) => Partial<PlacePanels>)) => {
    setSettings((prev) => {
      const s = selRef.current;
      const key = s ? placeKey(s.repo, s.slug) : null;
      const cur = panelsFor(prev, key);
      const p = typeof patch === "function" ? patch(cur) : patch;
      // The zoom is the one field that must NOT be filled in from `cur`, which
      // falls back to the global: every dock toggle would freeze whatever the
      // seed happened to be into a place you never chose a size in, and then —
      // because the record is spread back over the globals — hand that stale
      // number to the next place you visited. Carry the STORED value if there
      // is one, otherwise leave the key out and let `panelsFor` keep inheriting.
      const stored = key ? prev.place_panels?.[key] : undefined;
      const zoom = p.files_md_zoom ?? stored?.files_md_zoom;
      const panels: PlacePanels = {
        dock_open: p.dock_open ?? cur.dock_open,
        dock_tab: p.dock_tab ?? cur.dock_tab,
        dock_width: p.dock_width ?? cur.dock_width,
        ...(zoom !== undefined ? { files_md_zoom: zoom } : null),
      };
      const next: Settings = {
        ...prev,
        ...panels,
        // …and the global only tracks a size you actually CHOSE, so it stays a
        // "last used" seed rather than an echo of wherever you last clicked.
        ...(p.files_md_zoom === undefined ? { files_md_zoom: prev.files_md_zoom } : null),
        ...(key ? { place_panels: { ...(prev.place_panels ?? {}), [key]: panels } } : null),
      };
      applySettings(next);
      if (hydrated.current) saveSettings(next);
      // Pre-hydration, record ONLY the fields the caller actually asked for —
      // `p`, not `panels`. `preHydration` is SHALLOW-merged over the settings
      // read from disk and then saved, so every key present here overwrites the
      // stored value. `panels` fills its gaps from `cur`, which pre-hydration is
      // DEFAULTS: stashing it would write default `dock_tab`/`dock_width` over
      // the user's real ones for a ⌘J that only meant to toggle `dock_open`.
      // (`place_panels` must stay out for the same reason, one level up: a
      // shallow merge would replace the whole map with a one-entry object.)
      else Object.assign(preHydration.current, p);
      return next;
    });
  }, []);

  /** Forget remembered panels for keys matching `shouldDrop`.
   *
   *  `ui-state.json` is written as one blob and unknown keys survive a reload
   *  forever, so without pruning this map only ever grows — a place removed
   *  years ago would still be carrying a dock width. */
  const dropPanels = useCallback((
    shouldDrop: (key: string) => boolean,
    fields: readonly ("place_panels" | "term_tab_names" | "term_tab_active" | "term_tabs")[] =
      ["place_panels", "term_tab_names", "term_tab_active", "term_tabs"],
  ) => {
    setSettings((prev) => {
      // The default sweeps EVERY per-place map, not just the panels: they are
      // all keyed `repo|slug` and, when the place is truly GONE, they all
      // outlive it otherwise. `term_tab_names` had been leaking entries for
      // removed places since it was added — a tab name is only dropped when
      // the tab is CLOSED, which a removed place never gets the chance to do.
      // A caller whose place still exists narrows `fields` (see removeProject).
      const swept = fields.flatMap((field) => {
        const cur = prev[field] ?? {};
        const kept = Object.fromEntries(Object.entries(cur).filter(([k]) => !shouldDrop(k)));
        return Object.keys(kept).length === Object.keys(cur).length ? [] : [[field, kept] as const];
      });
      if (!swept.length) return prev;
      const next = { ...prev, ...Object.fromEntries(swept) } as Settings;
      if (hydrated.current) saveSettings(next);
      return next;
    });
  }, []);

  // Sweep entries whose place no longer exists. ONCE per session, on the first
  // workspace that actually reported something: at hydration there is no
  // snapshot to compare against, and re-running it on every poll would risk
  // discarding real state over a momentary blip.
  //
  // Guarded PER PROJECT. A repo whose snapshot failed (`ok: false`) is skipped
  // entirely — its places are *unknown*, not absent, and treating an unreadable
  // repo as an empty one would wipe every place in it. A repo that is no longer
  // tracked at all reports nothing either way, which is why untracking has its
  // own drop (see removeProject).
  const prunedOnce = useRef(false);
  useEffect(() => {
    if (!ws || prunedOnce.current || !hydrated.current) return;
    const ok = ws.projects.filter((pv) => pv.ok && pv.snapshot);
    if (!ok.length) return;
    prunedOnce.current = true;
    const live = new Set(ok.flatMap((pv) => pv.snapshot!.places.map((p) => placeKey(pv.root, p.slug))));
    dropPanels((k) => ok.some((pv) => k.startsWith(pv.root + "|")) && !live.has(k));
  }, [ws, dropPanels]);

  /** Name a place, or clear the name with "". Declared state, so it lives in
   *  `.worktrees.places.json` beside the note and the pin — NOT in ui-state,
   *  because what a place is called belongs to the project, not to this app
   *  install. */
  const setTitle = (repo: string, slug: string, title: string) => {
    setRenaming(false);
    patchDeclared(repo, slug, { title: title.trim() || undefined });
    mutate(invoke("set_title", { repo, slug, title }));
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

  /** Remember this place's shell tab strip (empty = drop the entry). */
  const setTermTabs = (repo: string, slug: string, ids: number[]) => {
    const key = placeKey(repo, slug);
    const all = { ...(settings.term_tabs ?? {}) };
    if (ids.length) all[key] = ids;
    else delete all[key];
    updateSettings({ term_tabs: all });
  };

  /** Remember which shell tab is in front for this place (null = none left).
   *  Sibling of `term_tab_names` rather than a `place_panels` field — see the
   *  note on `term_tab_active` in settings.ts for why it cannot be one. */
  const setTermTab = (repo: string, slug: string, index: number | null) => {
    const key = placeKey(repo, slug);
    const all = { ...(settings.term_tab_active ?? {}) };
    if (index == null) delete all[key];
    else all[key] = index;
    updateSettings({ term_tab_active: all });
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
  const fit = fitLayout(eff, dockEligible, vw);
  const dockShown = fit.dockShown;
  // Would the dock fit if it were open? Drives the toggle's disabled state, so a
  // ⌘J that can't visibly do anything is at least honest about why.
  const dockFits = fitLayout({ ...eff, dock_open: true }, dockEligible, vw).dockShown;

  // A find bar cannot outlive the thing it is searching. Closing the dock,
  // flipping its tab, leaving reading mode or switching place all unmount the
  // surface — without this the state would say "open" against nothing, and the
  // next ⌘F on that surface would look like a no-op (same value, no re-render).
  useEffect(() => { if (!dockShown) setFindOn((f) => (f === "dock" ? null : f)); }, [dockShown]);
  useEffect(() => { setFindOn((f) => (f === "dock" ? null : f)); }, [eff.dock_tab]);
  useEffect(() => { if (!reading) setFindOn((f) => (f === "read" ? null : f)); }, [reading]);
  // The reader covers the dock, so a dock bar opened behind it is invisible —
  // and its highlights would sit painted on a file nobody can see.
  useEffect(() => { if (reading) setFindOn((f) => (f === "dock" ? null : f)); }, [reading]);
  // The main pane unmounts when the session goes down. Without this `findOn`
  // stays "main", and the next TerminalPane — a brand new session — mounts
  // with a find bar already open on it.
  useEffect(() => {
    if (!selected?.tmux_session.up) setFindOn((f) => (f === "main" ? null : f));
  }, [selected?.tmux_session.up]);
  useEffect(() => { setFindOn(null); }, [sel?.repo, sel?.slug]);

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

  /** Apply a DECLARED change to the workspace already in hand, so the UI shows
   *  it now instead of after a full git sweep.
   *
   *  `refresh()` re-snapshots EVERY registered project, and each snapshot fans
   *  out up to 16 concurrent git calls — ~0.3s for one project with nine
   *  worktrees, and seconds across a real workspace. Declared edits do not need
   *  to wait for any of that: the store write has already happened and the
   *  result is known in advance, so a name you just typed should appear at once.
   *  The refresh still follows and remains the source of truth; this only closes
   *  the gap until it lands.
   *
   *  ⚠ Never write the optimistic value into `lastSnap`. That is refresh's record
   *  of what the BACKEND last said, and a guess does not belong in it. CLEARING
   *  it is the opposite move and is required: `ws` now holds something the
   *  backend never produced, so the confirming refresh must be allowed to write
   *  even when the backend's answer matches what refresh last saw. Otherwise a
   *  write that changes nothing on disk — one that FAILED, a name retyped
   *  identically, a note blurred unedited — leaves the dedupe bailing against a
   *  snapshot that still describes the pre-patch world, and the optimistic value
   *  stands with nothing left to correct it. `""` never equals a real
   *  JSON.stringify, so the next refresh always lands; the dedupe resumes on the
   *  one after it, which is the idle case it was written for.
   *
   *  ⚠ Declared fields the backend copies through verbatim need no more than
   *  `patch`. `lifecycle` is the exception: `lifecycle_effective` is reconciled
   *  server-side from the declared label AND live tmux state, so patching the
   *  label ALONE would show a row disagreeing with its own badge until the
   *  refresh landed. A lifecycle patch must therefore arrive together with
   *  `effective` — the PREDICTED badge from dnd.ts `predictTier`, which mirrors
   *  `store::reconcile` exactly (dnd-check.mjs re-reads store.rs and goes red
   *  if the mirror drifts). The prediction, never the label: writing the label
   *  into the badge is precisely the disagreement this warning prevents. */
  const patchDeclared = (
    repo: string, slug: string, patch: Partial<NonNullable<Declared>>, effective?: string,
  ) => {
    // Outranks every read already in flight, the same way commitWs does. Those
    // reads were issued before the user typed this, so none of them can contain
    // it — letting one land would wipe the value back off the screen, and the
    // confirming refresh (issued after this, so higher-ticketed) still corrects
    // whatever the guess got wrong.
    refreshDone.current = ++refreshSeq.current;
    lastSnap.current = ""; // force the confirming refresh to land — see above
    setWs((cur) =>
      !cur ? cur : {
        projects: cur.projects.map((pv) =>
          pv.root !== repo || !pv.snapshot
            ? pv
            : {
                ...pv,
                snapshot: {
                  ...pv.snapshot,
                  places: pv.snapshot.places.map((p) =>
                    p.slug !== slug ? p : {
                      ...p,
                      ...(effective === undefined ? null : { lifecycle_effective: effective }),
                      declared: { ...(p.declared ?? {}), ...patch },
                    },
                  ),
                },
              },
        ),
      },
    );
  };

  const mutate = async (p: Promise<unknown>) => {
    try { await p; } catch (e) { fail(e); }
    // Re-read either way. A failed declared edit has usually left an optimistic
    // value on screen, and a failed op can be a PARTIAL success in general (the
    // same reasoning runCmd states) — so the tree gets re-pulled regardless, and
    // the refresh corrects anything the optimism got wrong.
    await refresh();
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
  const openNewProject = () => {
    closeCtx();
    setNpErr("");
    setNpOpen(true);
  };
  /** Browse… inside the dialog: the native picker fills the LOCATION FIELD and
   *  nothing else. It answers "which folder does this go in", never "create
   *  this" — the field stays the thing the Create button reads. */
  const browseLocation = async (): Promise<string | null> => {
    try {
      const dir = await open({ directory: true, title: "Where should the project live?" });
      return typeof dir === "string" ? dir : null;
    } catch (e) { fail(e); return null; }
  };
  /** `<location>/<name>` — created, `git init`-ed with a first commit and added
   *  to the workspace by one backend command (which re-validates both fields,
   *  refuses an existing target, and reuses the same `init_repo` path the
   *  add-a-plain-folder offer runs). Errors stay IN the dialog: the fields that
   *  caused them are still on screen and still editable. */
  const createProject = async (location: string, name: string) => {
    setNpBusy(true);
    setNpErr("");
    try {
      setErr("");
      commitWs(await invoke<Workspace>("create_project", { location, name }));
      setNpOpen(false);
    } catch (e) {
      setNpErr(String((e as { message?: string })?.message ?? e));
      fail(e);
    } finally {
      setNpBusy(false);
    }
  };
  /** Where a new project should go, in order: the folder this workspace's
   *  projects already share (one answer for most people, and the right one), or
   *  `~/workspace` — a tilde path on purpose, since the backend expands it and
   *  it reads as a path rather than as somebody's username. */
  const defaultLocation = useMemo(() => {
    const parents = (ws?.projects ?? [])
      .map((p) => p.root.slice(0, p.root.lastIndexOf("/")))
      .filter(Boolean);
    const counts = new Map<string, number>();
    for (const p of parents) counts.set(p, (counts.get(p) ?? 0) + 1);
    let best = "";
    let n = 0;
    for (const [p, c] of counts) if (c > n) { best = p; n = c; }
    return best || "~/workspace";
  }, [ws]);
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
      // untracking reports nothing either way, so the snapshot sweep can never
      // judge these keys — drop the panels here or they are stranded forever.
      // ONLY the panels, though: untracking is reversible and the backend keeps
      // its half of a tab's identity (shell-cwds.json survives for places still
      // on disk), so wiping the term_tab_* maps loses the names AND lets a
      // fresh "+" tab after re-add land on a surviving index, inheriting a cwd
      // it never visited.
      dropPanels((k) => k.startsWith(root + "|"), ["place_panels"]);
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
  // A dialog, so there is no nav to bring back — but the draft MUST be cleared:
  // it belongs to the create that was rejected, and reopening for a different
  // project would otherwise seed that project's fields with it.
  const openNewForm = (root: string, base: string) => {
    setNewFor(root);
    setNewBase(base);
    setNewDraft(null);
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
      dropPanels((k) => k === placeKey(repo, slug));
      if (sel?.repo === repo && sel?.slug === slug) setSel(null);
    }
  };
  // ── sync ──
  // Cheap enough to fire on menu-open (one rsync --version, one manifest read,
  // two git calls). Failures go through `fail` like every other invoke — a
  // greyed-out sync group with no reason is exactly the silence CLAUDE.md bans.
  const loadSyncStatus = (root: string) => {
    invoke<SyncStatusView>("sync_status", { repo: root })
      .then((s) => setSyncStatus((m) => ({ ...m, [root]: s })))
      .catch((e) => fail(e));
  };
  /** The hover popover on the project header's ⇄. Anchored to the button and
   *  rendered through `CtxMenu` rather than an in-flow `.popover`: the nav is a
   *  scroller (`.nav-scroll { overflow-y: auto }`), which clips an absolutely
   *  positioned child — a menu opened next to the last project would be half
   *  invisible. Same `.menu-catch` backdrop, same Esc/click-away dismissal, same
   *  `.pop-item` family. */
  const openSyncPop = (e: React.MouseEvent, root: string) => {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    setMenu(null);
    setConfirmRm(null);
    setCtx({ kind: "sync", x: r.left, y: r.bottom + 4, root });
    // Re-read on every open: a hub is a drive someone plugs in and pulls out.
    loadSyncStatus(root);
  };
  /** Open the modal and fetch its preview. `adopt` non-null addresses the
   *  backend by hub NAME (there is no local root yet); everything downstream —
   *  the modal, the confirm, the progress bar — is the same code path, which is
   *  the point: an import is a pull that happens to have no row yet. */
  const openSyncFor = (root: string, dir: SyncDir, adopt: Adopt | null) => {
    // Also what closes the popover: an open menu must never survive under a
    // modal (closeCtx's contract, which also disarms any two-click confirm).
    closeCtx();
    setSync({ root, dir, adopt });
    setSyncPrev(null);
    setSyncErr("");
    setSyncDone("");
    setSyncLive([]);
    setSyncProg(null);
    setSyncInstall(false);
    setSyncLoading(true);
    invoke<SyncPreview>("sync_preview", {
      repo: adopt ? null : root,
      name: adopt ? adopt.name : null,
      direction: dir,
      withSessions: settings.sync_with_sessions,
    })
      .then((p) => { setSyncPrev(p); setSyncLive(p.live_sessions); })
      // In the modal, not the banner: it is the answer to the question the modal
      // just asked. Logged too — `fail`'s job — so it is never only on screen.
      .catch((e) => { setSyncErr(String((e as { message?: string })?.message ?? e)); fail(e); })
      .finally(() => setSyncLoading(false));
  };
  const openSync = (root: string, dir: SyncDir) => openSyncFor(root, dir, null);
  const closeSync = () => {
    // Cancelling an IMPORT goes back to the picker it came from — the reason to
    // cancel is usually "wrong project", and the list is the answer to that.
    // Never after a completed one: the project is in the nav now.
    const back = !!sync?.adopt && !syncDone && !syncBusy;
    setSync(null);
    setSyncPrev(null);
    setSyncErr("");
    setSyncDone("");
    setSyncLive([]);
    setSyncProg(null);
    setSyncInstall(false);
    if (back) setImportOpen(true);
  };
  // ── import from hub (adoption: a project the workspace has never had) ──
  const openImport = () => {
    closeCtx();
    setImportOpen(true);
    setImportList(null);
    setImportErr("");
    setImportLoading(true);
    invoke<HubList>("sync_hub_list")
      .then((l) => setImportList(l))
      .catch((e) => { setImportErr(String((e as { message?: string })?.message ?? e)); fail(e); })
      .finally(() => setImportLoading(false));
  };
  /** Pick one → the same modal every sync goes through, in adoption mode. The
   *  picker closes: two stacked scrims would hide which question is being
   *  answered. */
  const pickImport = (p: HubProject) => {
    setImportOpen(false);
    openSyncFor("", "pull", { name: p.name, dest: p.local_root });
  };
  /** The nav footer's add menu — the one place a project arrives from, whichever
   *  of the three ways it arrives by. Anchored to the button and rendered
   *  through `CtxMenu`, which clamps it INTO the viewport: opened from a button
   *  on the bottom edge that means it opens upward, over the footer, rather
   *  than off the bottom of the window. */
  const openAddMenu = (e: React.MouseEvent) => {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    setMenu(null);
    setConfirmRm(null);
    setCtx({ kind: "add", x: r.left, y: r.top });
  };
  /** Roots the workspace already tracks — a hub project pointing at one of them
   *  is not importable (it is already here, and importing would mirror
   *  --delete over it). The row says so instead of failing later. */
  const knownRoots = useMemo(
    () => new Set((ws?.projects ?? []).map((p) => p.root)),
    [ws],
  );
  // `confirmed` says a HUMAN was shown the live sessions — the modal's warning
  // block IS that showing. Core re-lists them at apply time, so a session that
  // started since comes back as a fresh question rather than being mirrored over.
  const confirmSync = async () => {
    if (!sync || syncBusy) return;
    setSyncBusy(true);
    setSyncErr("");
    setSyncProg(null);
    // rsync's own progress, relayed by core through the same Channel mechanism
    // the terminal uses for pty bytes. Throttled backend-side (~15/s), so every
    // message that arrives here is worth a render.
    const onProgress = new Channel<SyncProgressEvt>();
    onProgress.onmessage = (p) => setSyncProg(p);
    const r = await runCmd("sync_apply", {
      repo: sync.adopt ? null : sync.root,
      name: sync.adopt ? sync.adopt.name : null,
      direction: sync.dir, withSessions: settings.sync_with_sessions,
      confirmed: syncLive.length > 0, install: sync.dir === "pull" && syncInstall,
      onProgress,
    });
    setSyncBusy(false);
    if (!r) { setSyncErr("sync could not start — see Settings → Logs."); return; }
    if (r.code === EXIT_NEEDS_CONFIRM) {
      // Sessions appeared between preview and apply: re-render the warning with
      // the list core actually saw, and let the user answer THAT one.
      setSyncLive((r.needs_confirm ?? "").split("\n").filter(Boolean));
      return;
    }
    // A half-succeeded import lands here too (transfer ok, workspace add failed):
    // the backend reports it as a failed CmdResult whose text says which half
    // worked, and that text is what the modal shows.
    if (!r.ok) { setSyncErr(r.output || `sync ${sync.dir} failed (exit ${r.code})`); return; }
    setSyncDone(r.output);
    // An import's new project is now in the workspace (runCmd already refreshed,
    // and `places:changed` re-pulls again): the picker's listing is stale, and
    // the row it was offering is a row in the nav now.
    if (sync.adopt) setImportOpen(false);
    else loadSyncStatus(sync.root); // the push stamp the next menu shows
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
    // Re-read on every open rather than once: a hub is a drive someone plugs in
    // and pulls out, so a cached "no hub" would outlive the truth.
    if (ws?.projects.find((v) => v.root === root)?.ok) loadSyncStatus(root);
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
    // by the DISPLAYED name: sorting renamed places by a hidden slug reads as
    // an alphabetical list that is not alphabetical
    if (settings.sort_mode === "alpha") out.sort((a, b) => nameOf(a).localeCompare(nameOf(b)));
    else out.sort((a, b) => activityAt(b) - activityAt(a));
    const flip = settings.sort_mode === "alpha" ? settings.sort_dir === "desc" : settings.sort_dir === "asc";
    if (flip) out.reverse();
    return out;
  };

  // ── nav drag & drop ────────────────────────────────────────────────────────
  // Dragging a row between tier groups is a LIFECYCLE edit, not a sort: it works
  // in every sort mode. Only the row's position inside the group it lands in is
  // sort-mode-specific — the pointer chooses it under Manual, and under A–Z /
  // last-used the list itself does, so the gap opens at the slot that mode will
  // actually produce rather than wherever the cursor happens to be.
  type DropTarget =
    | { kind: "tier"; repo: string; tier: Tier; before: string | null; lands: Tier; patch: DeclPatch }
    | { kind: "reject"; hint: string }
    | { kind: "project"; before: string | null };

  const placeAt = (repo: string, slug: string) =>
    ws?.projects.find((v) => v.root === repo)?.snapshot?.places.find((p) => p.slug === slug) ?? null;
  /** The rows a tier group is currently RENDERING, in render order — the same
   *  derivation ProjectNode uses, filter included, so a landing slot computed
   *  here names a row that is actually on screen. */
  const tierRows = (repo: string, tier: Tier): Place[] => {
    const places = (ws?.projects.find((v) => v.root === repo)?.snapshot?.places ?? []).filter(matchPlace);
    return sortPlaces(repo, places.filter((p) => !p.is_main && bucketOf(p) === tier));
  };
  /** Live geometry of the drop rows inside `zone`, with the open gap
   *  arithmetically removed (dnd.ts `naturalTop` explains why measuring around
   *  it is not optional). Returns rows in DOM order, minus the one being
   *  dragged; place rows and project headers differ only in selector, identity
   *  attribute, and which gap displaces them. */
  const rowGeometry = (
    zone: HTMLElement, y: number, dragging: string,
    sel: string, keyOf: (el: HTMLElement) => string, gapSel = ".drop-gap",
  ) => {
    const gr = zone.querySelector<HTMLElement>(gapSel)?.getBoundingClientRect();
    const gapTop = gr?.top ?? Infinity;
    const gapH = gr?.height ?? 0;
    const rows = [...zone.querySelectorAll<HTMLElement>(sel)]
      .filter((el) => keyOf(el) !== dragging)
      .map((el) => {
        const r = el.getBoundingClientRect();
        return { slug: keyOf(el), top: naturalTop(r.top, gapTop, gapH), height: r.height };
      });
    return { rows, y: naturalTop(y, gapTop, gapH) };
  };

  const resolveDrop = (item: DragItem, x: number, y: number): DropTarget | null => {
    const el = document.elementFromPoint(x, y) as HTMLElement | null;
    if (item.kind === "project") {
      const list = navScrollRef.current;
      // Anywhere in the list column counts, not just over a project node. The
      // scroller's own padding is a real place to aim: the strip ABOVE the
      // first project is how you make one first, and the empty space below the
      // last is how you send one to the end. Requiring a project under the
      // pointer made both of those silently do nothing — no gap, no drop.
      if (!list || !el || !list.contains(el)) return null;
      const { rows, y: ny } = rowGeometry(
        list, y, item.root, "[data-project-root]", (n) => n.dataset.projectRoot!, ".drop-gap.proj",
      );
      const i = pointerIndex(rows, ny);
      return { kind: "project", before: rows[i]?.slug ?? null };
    }

    const zone = el?.closest<HTMLElement>("[data-tier]");
    if (!zone) return null;
    if (zone.dataset.repo !== item.repo)
      return { kind: "reject", hint: "a worktree belongs to its project — it can't move to another one" };
    const p = placeAt(item.repo, item.slug);
    if (!p) return null;
    const tier = zone.dataset.tier as Tier;
    const plan = dropIntent(p, tier, Math.floor(Date.now() / 1000), bucketOf(p) as Tier);
    if (!plan.ok) return { kind: "reject", hint: plan.hint };
    // The gap opens in the group the row will REALLY land in. For the derived
    // tiers those differ often enough that showing it under the pointer would
    // be a promise the list cannot keep.
    const lands = plan.lands;
    const rows = tierRows(item.repo, lands).filter((r) => r.slug !== item.slug);
    let before: string | null;
    if (settings.sort_mode === "manual" && lands === tier) {
      const geo = rowGeometry(zone, y, item.slug, "[data-slug]", (n) => n.dataset.slug!);
      before = geo.rows[pointerIndex(geo.rows, geo.y)]?.slug ?? null;
    } else if (settings.sort_mode === "alpha") {
      before = rows[alphaIndex(rows.map(nameOf), nameOf(p), settings.sort_dir === "desc")]?.slug ?? null;
    } else if (settings.sort_mode === "recent") {
      before = rows[recentIndex(rows.map(activityAt), activityAt(p), settings.sort_dir === "asc")]?.slug ?? null;
    } else {
      // Manual, but landing in a group the pointer is not over: the pointer has
      // nothing to say about a position in a list it is not in. Land at the top,
      // which is where the eye goes looking for what just moved.
      before = rows[0]?.slug ?? null;
    }
    return { kind: "tier", repo: item.repo, tier, before, lands, patch: plan.patch };
  };

  /** Spring-open: a group that is collapsed can still be a drop target, but it
   *  cannot show WHERE. Hovering it for a beat opens it. */
  const springRef = useRef<{ key: string; timer: number } | null>(null);
  const onDragZone = (zone: HTMLElement | null, item: DragItem | null) => {
    if (springRef.current) { clearTimeout(springRef.current.timer); springRef.current = null; }
    const key = zone?.dataset.gkey;
    if (item?.kind !== "place" || !zone || !key || zone.dataset.open === "1") return;
    springRef.current = {
      key,
      timer: window.setTimeout(() => { setGroupOpen((m) => ({ ...m, [key]: true })); }, 600),
    };
  };

  const commitDrop = (item: DragItem, target: DropTarget | null) => {
    if (!target || target.kind === "reject") {
      if (target?.kind === "reject") setNotice(target.hint);
      return;
    }
    if (item.kind === "project") {
      if (target.before === item.root) return;
      const roots = moveBefore((ws?.projects ?? []).map((pv) => pv.root), item.root, target.before);
      reorderProjects(roots);
      return;
    }
    if (target.kind !== "tier") return;
    const p = placeAt(item.repo, item.slug);
    if (!p) return;
    const before = { pinned: !!p.declared?.pinned, lifecycle: p.declared?.lifecycle ?? null };
    const after = {
      pinned: target.patch.pinned ?? before.pinned,
      lifecycle: target.patch.lifecycle !== undefined ? target.patch.lifecycle : before.lifecycle,
    };
    const moved = after.pinned !== before.pinned || after.lifecycle !== before.lifecycle;

    // Position first: under Manual the drop is also a reorder, and a reorder is
    // pure settings — no round trip, so it lands in the same frame as the gap
    // closing. `manual_order` is one flat array per repo whose only job is
    // within-group order, which is why a cross-group move splices into it too.
    // Functional patch: the splice must read `manual_order` as it is NOW, not
    // as the render that produced this callback saw it.
    const prevOrder = settings.manual_order[item.repo];
    if (settings.sort_mode === "manual") {
      const slugs = (ws?.projects.find((v) => v.root === item.repo)?.snapshot?.places ?? [])
        .filter((pl) => !pl.is_main).map((pl) => pl.slug);
      updateSettings((prev) => ({
        manual_order: {
          ...prev.manual_order,
          [item.repo]: spliceOrder(prev.manual_order[item.repo] ?? [], slugs, item.slug, target.before),
        },
      }));
    }
    if (!moved) return;

    applyTier(item.repo, item.slug, before, after);
    setFlash({ repo: item.repo, slug: item.slug });
    // Two ways a drop can leave the row where the gesture cannot see it: it
    // landed in a tier other than the one under the pointer (routine — half the
    // groups are derived), or that tier is switched off in Settings, in which
    // case the row does not just move, it DISAPPEARS. Both get said out loud.
    const hidden = settings.hidden_tiers.includes(target.lands)
      || (DORMANT_TIERS.includes(target.lands as (typeof DORMANT_TIERS)[number])
        && settings.hidden_tiers.includes("dormant"));
    const note = landingNote(target.tier, target.lands);
    if (hidden) setNotice(`${nameOf(p)} → ${TIER_LABEL[target.lands]}, a tier you have hidden — it is out of the list until you show it again.`);
    else if (note) setNotice(`${nameOf(p)} — ${note}`);
    setUndo({
      text: `Moved ${nameOf(p)} to ${TIER_LABEL[target.lands]}`,
      run: () => {
        applyTier(item.repo, item.slug, after, before);
        // Restore ONLY this repo's order, merged into `manual_order` as it is
        // NOW: this closure can be up to 8s old, a pure reorder in another repo
        // does not replace the banner, and a captured-`settings` spread would
        // write that reorder back out of existence. `prevOrder === undefined`
        // means the repo had never been ordered — restored by DELETING the key,
        // not by skipping (skipping leaves the drop's splice behind).
        // `sort_mode` is deliberately the drop-time value: it decides whether
        // the DROP spliced, not what mode the app is in when Undo is clicked.
        if (settings.sort_mode === "manual")
          updateSettings((prev) => {
            const manual_order = { ...prev.manual_order };
            if (prevOrder) manual_order[item.repo] = prevOrder;
            else delete manual_order[item.repo];
            return { manual_order };
          });
        setUndo(null);
        setFlash({ repo: item.repo, slug: item.slug });
      },
    });
  };

  /** Write a tier change, optimistically on BOTH axes. Pin the backend copies
   *  through verbatim. The lifecycle badge (`lifecycle_effective`) is
   *  reconciled server-side from the label AND live tmux, and for that reason
   *  it used to be off-limits to optimism — but `predictTier` now mirrors that
   *  reconciliation exactly, and dnd-check.mjs re-reads store.rs so the mirror
   *  cannot drift silently. So the badge is patched to the PREDICTION — never
   *  to the label, which is the disagreement the old rule guarded against.
   *  Do not "simplify" this back to label-only or non-optimistic: without the
   *  badge patch the row sits in its old group for the whole git fan-out
   *  (seconds in a real workspace) with the gap closed and the flash firing in
   *  the group the row is LEAVING. The confirming refresh remains the source
   *  of truth and corrects any misprediction.
   *  Both writes go out as ONE promise: two `mutate` calls would race two
   *  refreshes for one gesture. */
  const applyTier = (
    repo: string, slug: string,
    from: { pinned: boolean; lifecycle: string | null },
    to: { pinned: boolean; lifecycle: string | null },
  ) => {
    const p = placeAt(repo, slug);
    const decl: Partial<NonNullable<Declared>> = {};
    if (to.pinned !== from.pinned) decl.pinned = to.pinned;
    if (to.lifecycle !== from.lifecycle) decl.lifecycle = to.lifecycle ?? undefined;
    if (p && (to.pinned !== from.pinned || to.lifecycle !== from.lifecycle))
      patchDeclared(repo, slug, decl,
        to.lifecycle !== from.lifecycle
          // `pinned: false` steps predictTier past its pin short-circuit: the
          // badge under a pin is still the reconciled label, never "pinned" —
          // pin lives in `declared`, and bucketOf ranks it separately.
          ? predictTier(p, { pinned: false, lifecycle: to.lifecycle }, Math.floor(Date.now() / 1000))
          : undefined);
    mutate((async () => {
      if (to.pinned !== from.pinned) await invoke("set_pin", { repo, slug, on: to.pinned });
      if (to.lifecycle !== from.lifecycle)
        await invoke("set_lifecycle", { repo, slug, label: to.lifecycle ?? "" });
    })());
  };

  /** Optimistic re-order, then the backend's answer. The optimistic step
   *  follows `patchDeclared`'s discipline (outrank in-flight reads, force the
   *  confirming write to land) because the real `list_workspace` is a git
   *  fan-out — seconds, during which the row would otherwise snap back. */
  const reorderProjects = (roots: string[]) => {
    refreshDone.current = ++refreshSeq.current;
    lastSnap.current = "";
    setWs((cur) =>
      !cur ? cur : { projects: roots.map((r) => cur.projects.find((pv) => pv.root === r)!).filter(Boolean) },
    );
    invoke<Workspace>("reorder_projects", { roots })
      .then(commitWs)
      // mutate's rule — re-read either way: a failed write leaves the
      // optimistic order on screen with `lastSnap` cleared, and without this
      // nothing corrects it until the next poll happens to fire.
      .catch((e) => { fail(e); refresh(); });
  };

  // No per-handler click guards anywhere below: the click that trails a drag
  // is swallowed window-wide, capture-phase, inside navdrag.ts — a drop can
  // release over ANY control, not just the handlers that remembered to check.
  const { drag, arm } = useNavDrag<DropTarget>({
    scrollRef: navScrollRef,
    resolve: resolveDrop,
    commit: commitDrop,
    onZone: onDragZone,
  });
  /** The gap element, rendered at the landing slot of the group being hovered.
   *  `before === null` means "at the end of that group". */
  const gapAt = (repo: string, tier: Tier, before: string | null) =>
    drag?.item.kind === "place" && drag.target?.kind === "tier"
    && drag.target.repo === repo && drag.target.lands === tier && drag.target.before === before;
  const projGapAt = (root: string | null) =>
    drag?.item.kind === "project" && drag.target?.kind === "project" && drag.target.before === root;
  /** Which tiers must exist as drop targets for the drag in flight, even empty:
   *  an empty group renders nothing, and a tier you cannot see is a tier you
   *  cannot drop on — the reason "pin the first place" was undraggable. */
  const dragTiersFor = (repo: string): Tier[] =>
    drag?.item.kind === "place" && drag.item.repo === repo
      ? [...LIVE_TIERS, ...DORMANT_TIERS].filter((t) => t !== "active")
      : [];

  useEffect(() => {
    if (!flash) return;
    const t = setTimeout(() => setFlash(null), 1600);
    return () => clearTimeout(t);
  }, [flash]);
  useEffect(() => {
    if (!undo) return;
    const t = setTimeout(() => setUndo(null), 8000);
    return () => clearTimeout(t);
  }, [undo]);

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
    if ((await runCmd("remove_place", { repo: sel.repo, slug: sel.slug, delBranch, force: false }))?.ok) {
      dropPanels((k) => k === placeKey(sel.repo, sel.slug));
      setSel(null);
    }
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
    updatePanels(
      eff.dock_open && eff.dock_tab === tab
        ? { dock_open: false }
        : { dock_tab: tab, dock_open: true },
    );
  // right dock (Files / Terminal) — remembered per place, with the global value
  // as the seed. Stable useCallback (updatePanels reads the place from a ref),
  // so the keydown effect still registers once.
  const toggleDock = useCallback(() => {
    updatePanels((cur) => ({ dock_open: !cur.dock_open }));
  }, [updatePanels]);
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
  const keyRef = useRef({ selectLens, selectedPath: null as string | null, editorCmd: settings.editor_cmd, switchOpen, dockFile: null as string | null, reading: false, settingsOpen: false, filesTabOpen: false, mdPreview: false, mdZoom: DEFAULTS.files_md_zoom, dockShown: false, dockTab: "files" as Settings["dock_tab"], mainTermUp: false, findOn: null as null | Surface });
  const filesTabOpen = eff.dock_open && eff.dock_tab === "files";
  keyRef.current = {
    selectLens, selectedPath: selected?.path ?? null, editorCmd: settings.editor_cmd, switchOpen, dockFile, reading, settingsOpen,
    filesTabOpen,
    // ⌘+/⌘−/⌘0 only mean something while a RENDERED markdown doc is on screen —
    // in the dock's Files tab or in the reader. Anywhere else the chord is left
    // alone rather than silently adjusting a size nobody can see.
    mdPreview: (reading || filesTabOpen) && !!dockFile && fileInfo(dockFile).kind === "markdown" && !settings.files_md_source,
    // The SELECTED place's size, not the global one — the chord must move the
    // same number the header shows, which is `eff`.
    mdZoom: eff.files_md_zoom,
    // ⌘F: which surfaces can take the bar, and which one already has it.
    dockShown, dockTab: eff.dock_tab, mainTermUp: !!selected?.tmux_session.up, findOn,
  };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // ⌘⇧T — new dock terminal. The chord this started as; ⌘T (in the
      // meta-only section below) is the advertised one now and ⌘⇧T is kept as
      // a silent alias, since it costs nothing and fingers remember.
      // Handled BEFORE the meta-only guard (it needs shift). ⌘ only (not ctrl)
      // so Ctrl+Shift+T still reaches the embedded shell; swallowed while the
      // ⌘K palette owns the keyboard. No-op unless the dock's Terminal tab is
      // mounted (it ignores the token otherwise) — which is exactly the gap
      // ⌘T closes.
      if (e.metaKey && e.shiftKey && !e.altKey && !e.repeat && e.key.toLowerCase() === "t") {
        e.preventDefault();
        if (!keyRef.current.switchOpen) setNewTermToken((v) => v + 1);
        return;
      }
      // Esc closes the find bar before anything else — but NOT when the
      // terminal has the keyboard, where Escape is the user's (vim, a menu, a
      // prompt) and swallowing it would be a worse bug than a bar left open.
      // The bar's own handler covers Escape while the field or its buttons are
      // focused; this is the case where focus went back to the content.
      if (e.key === "Escape" && keyRef.current.findOn && !keyRef.current.settingsOpen && !keyRef.current.switchOpen) {
        const inTerm = e.target instanceof Element && e.target.closest(".term-host");
        if (!inTerm) {
          e.preventDefault();
          setFindOn(null);
          return;
        }
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
      // ⌘+ / ⌘− / ⌘0 — reading size of the rendered markdown. ABOVE the
      // meta-only gate on purpose: on a US layout ⌘+ arrives as ⌘⇧= (key "+"),
      // and the gate drops every shifted chord. Repeat is allowed here, unlike
      // the toggles — holding the key to walk up the steps is the point.
      if (e.metaKey && !e.ctrlKey && !e.altKey && ZOOM_KEYS.has(e.key)) {
        const kr = keyRef.current;
        if (!kr.mdPreview || kr.switchOpen || kr.settingsOpen) return;
        e.preventDefault();
        const cur = clampMdZoom(kr.mdZoom);
        const next = e.key === "0" ? 100 : stepMdZoom(cur, e.key === "-" || e.key === "_" ? -1 : 1);
        // keyRef is rebuilt on render; without this the second press of a fast
        // double-tap would step from the value the FIRST one started at.
        kr.mdZoom = next;
        // updatePanels, not updateSettings: the size belongs to the place being
        // read. (It is `useCallback([])`-stable, so this stale-closure-free.)
        if (next !== cur) updatePanels({ files_md_zoom: next });
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
      } else if (e.metaKey && k === "t") {
        // ⌘T — new terminal, macOS Terminal's chord and the ADVERTISED one now
        // (⌘⇧T, handled above, stays as a silent alias). ⌘-only like ⌘J, so
        // plain Ctrl+T keeps reaching the embedded shell: no branch in this
        // chain but ⌘B answers a ctrl-only chord, and nothing here calls
        // preventDefault for one.
        //
        // Two jobs, never both at once. With the Terminal tab already on
        // screen the chord adds a shell; anywhere else (dock closed, or open
        // on Files) it brings the tab up and stops — MOUNTING TerminalTabs
        // already restores the remembered tabs (or spawns the first one), so
        // adding on top of that would double up. The token consumer skips its
        // initial value precisely so a bump in the same render as the mount
        // does nothing; this branch doesn't fight that, it relies on it.
        e.preventDefault();
        const kr = keyRef.current;
        if (kr.dockShown && kr.dockTab === "terminal") setNewTermToken((v) => v + 1);
        else {
          // The same write the right rail makes (pickDockTab) — one
          // updatePanels carrying only the two fields this act chose, so no
          // seeded value gets frozen into a place that never set it.
          updatePanels({ dock_tab: "terminal", dock_open: true });
        }
      } else if (e.metaKey && e.key === ",") {
        // ⌘, opens Settings (macOS convention) — a meta chord is safe past the
        // term-host (its passthrough concerns are ctrl-only). Esc already closes.
        e.preventDefault();
        setSettingsOpen((v) => !v);
      } else if (e.metaKey && k === "f") {
        // ⌘F — find. Which surface depends on where the user is (see findTarget);
        // a second press re-selects the field rather than toggling the bar shut,
        // which is what every other find bar does.
        e.preventDefault();
        if (keyRef.current.settingsOpen) return;
        // `lastSurface` is read HERE, live. A copy taken during render goes
        // stale the moment the user does something that changes no React state
        // — clicking into a terminal to focus it, clicking the open file's
        // text — and ⌘F would then open on whichever surface last happened to
        // re-render.
        const target = findTarget({ ...keyRef.current, last: lastSurface.current });
        if (!target) return;
        setFindOn(target);
        setFindToken((v) => v + 1);
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
  }, [toggleNav, toggleDock, updatePanels, fail, settings.nav_collapsed]);

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
    // a renamed place has to be findable by the name on screen — and still by
    // its slug, which is what the directory and the tmux session are called
    (p.declared?.title ?? "").toLowerCase().includes(q) ||
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
  // Home "resume where you left off" — same clock and same order as the nav
  // tree, because it is the same question asked from the empty state.
  // `donePaths` is a dep, not a bystander: `activityAt` reads it, so a task
  // finishing while the home view is up has to be able to re-rank the list.
  const resume = useMemo(
    () => allPlaces
      .filter(({ p }) => !p.is_main)
      .sort((a, b) => activityAt(b.p) - activityAt(a.p))
      .slice(0, 6),
    [allPlaces, donePaths],
  );
  // Restore-on-launch target, and the ONE list-shaped thing that still ranks by
  // opens. Kept separate from `resume` above on purpose: `activityAt` counts
  // commits, and a CLI-made worktree that was never opened would then be what
  // the app launches into. See `usedEpoch`. Nothing renders this — it is one
  // place, read once.
  const restoreTarget = useMemo(
    () => allPlaces
      .filter(({ p }) => !p.is_main)
      .sort((a, b) => usedEpoch(b.p) - usedEpoch(a.p))[0],
    [allPlaces],
  );

  // Restore last place on launch (opt-in). SELECTION-ONLY: this calls setSel and
  // nothing else — NO enterPlace/touch_place/open_place (those would auto-resume a
  // Claude session on every reboot). TerminalPane attaches on its own if the tmux
  // session is up; otherwise the place view shows its normal "Enter ▸ to start".
  // Target = the most recently USED place across all projects (max of opened /
  // worked, main excluded) — NOT the top of the Resume list, which ranks by
  // activity and would hand this a place that only has a commit.
  // Fires exactly once, and only after BOTH settings hydration and the
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
    if (restoreTarget) setSel({ repo: restoreTarget.pv.root, slug: restoreTarget.p.slug });
  }, [ws, settings.restore_last, restoreTarget, sel]);

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
        nav_width: clampNav(startW + (ev.clientX - startX), dockShown ? eff.dock_width : 0, window.innerWidth),
      });
    const up = () => { window.removeEventListener("mousemove", move); window.removeEventListener("mouseup", up); };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  // ── dock resizer (drag the dock's LEFT edge — moving left GROWS it) ──
  const onDockResize = (e: React.MouseEvent) => {
    e.preventDefault();
    const startX = e.clientX;
    const startW = eff.dock_width;
    const move = (ev: MouseEvent) =>
      updatePanels({
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
  // No age override any more: every list this row appears in sorts by
  // `activityAt`, so the row can just print it. The old `ageEpoch` prop existed
  // because the Recent lens ordered by a different clock, and a list that labels
  // rows with a clock it does not order by reads as a broken sort.
  const PlaceRow = ({ repo, p, showProject }: { repo: string; p: Place; showProject?: boolean }) => {
    const divergent = !p.is_main && !p.detached && p.branch && p.branch !== p.slug;
    // The main worktree is not in any tier group and cannot leave one, so it is
    // the one row that never arms a drag. `showProject` marks the flat lenses
    // (Recent / Attention), which mix projects and order by activity — there is
    // no position there to drag a row into.
    const draggable = !p.is_main && !showProject;
    const isSource = drag?.item.kind === "place" && drag.item.slug === p.slug && drag.item.repo === repo;
    return (
      <li
        className={
          "row" +
          (sel?.repo === repo && sel?.slug === p.slug ? " sel" : "") +
          (isSource ? " drag-src" : "") +
          (flash?.repo === repo && flash.slug === p.slug ? " flash" : "") +
          (draggable ? " draggable" : "")
        }
        data-slug={p.slug}
        onClick={() => enterPlace(repo, p)}
        onContextMenu={(e) => placeCtx(e, repo, p)}
        onPointerDown={draggable ? (e) => arm(e, { kind: "place", repo, slug: p.slug }) : undefined}
        title={p.declared?.title ? `${p.declared.title} — ${p.slug}` : p.slug}
      >
        <span className={"status-dot" + dotClass(p)} title={dotTitle(p)} />
        <span className="row-id">
          <span className="row-name">
            {p.is_main ? "◆ " : p.declared?.pinned ? "★ " : ""}
            {nameOf(p)}
            {showProject ? <span className="row-proj">{basename(repo)}</span> : null}
          </span>
          {divergent ? <span className="row-branch">↗ {p.branch}</span> : null}
        </span>
        <span className="glyphs">
          {glyphs(p, health[repo]?.slugs.has(p.slug)).map((g, i) => (
            <span key={i} className={"g " + g.cls} title={g.title}>{g.text}</span>
          ))}
          <span className="row-age">{ago(activityAt(p))}</span>
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
    // Tiers this project must SHOW while a drag is in flight, so an empty
    // group is still somewhere a row can be dropped.
    const dragTiers = new Set(dragTiersFor(pv.root));
    const liveShown = LIVE_TIERS.filter(
      (g) => (buckets[g]?.length || dragTiers.has(g)) && !hiddenTiers.has(g),
    );
    const dormantShown = DORMANT_TIERS.filter((t) => buckets[t]?.length || dragTiers.has(t));
    /** One tier group's rows plus the gap, in render order. */
    const rowsWithGap = (g: Tier) => (
      <ul className="places">
        {(buckets[g] ?? []).map((p) => (
          <Fragment key={p.slug}>
            {gapAt(pv.root, g, p.slug) && <li className="drop-gap" />}
            <PlaceRow repo={pv.root} p={p} />
          </Fragment>
        ))}
        {gapAt(pv.root, g, null) && <li className="drop-gap" />}
      </ul>
    );

    return (
      <div className="project" data-project-root={pv.root} data-drop="project">
        <div
          className="project-h"
          onContextMenu={(e) => projectCtx(e, pv.root)}
          onPointerDown={(e) => arm(e, { kind: "project", root: pv.root })}
        >
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
          {/* The sync entry point David asked for on day one: hover-revealed,
              next to + and ×, opening the popover the ctx menu duplicates.
              `.mini` is a <button>, which navdrag's `arm` already declines to
              start a drag from — no stopPropagation needed, same as +. */}
          {pv.ok && (
            <button className="mini" title="Sync" data-testid={`sync-mini|${pv.root}`}
              onClick={(e) => openSyncPop(e, pv.root)}>⇄</button>
          )}
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
            {liveShown.map((g) => {
              const key = `${pv.root}|${g}`;
              const opened = isOpen(key, g !== "idle"); // idle collapsed by default
              return (
                <div
                  className={"group" + (buckets[g]?.length ? "" : " group-empty")}
                  key={key}
                  data-drop="tier" data-tier={g} data-repo={pv.root}
                  data-gkey={key} data-open={opened ? "1" : "0"}
                >
                  <GroupHeader gkey={key} label={GROUP_LABEL[g]} count={buckets[g]?.length ?? 0} open={opened} onToggle={() => toggleGroup(key, g !== "idle")} />
                  {opened && rowsWithGap(g)}
                </div>
              );
            })}
            {(dormant.length > 0 || dormantShown.length > 0) && !hiddenTiers.has("dormant") && (() => {
              const key = `${pv.root}|dormant`;
              const opened = isOpen(key, false);
              return (
                <div className="group dormant" key={key} data-gkey={key} data-open={opened ? "1" : "0"} data-drop="dormant">
                  <div className="group-h dormant-h" onClick={() => toggleGroup(key, false)}>
                    {/* same SVG caret as every other group header — the ASCII
                        ▾/▸ was one more thing making the quietest row louder */}
                    <span className="caret">{opened ? <Icons.ChevronDown size={11} /> : <Icons.ChevronRight size={11} />}</span>
                    Dormant<span className="count">{dormant.length}</span>
                  </div>
                  {opened && (
                    <div className="kids-d">
                      {dormantShown.map((t) => (
                        <div
                          className={"subgroup" + (buckets[t]?.length ? "" : " group-empty")}
                          key={t}
                          data-drop="tier" data-tier={t} data-repo={pv.root}
                        >
                          <div className="subdiv">{GROUP_LABEL[t]}</div>
                          {rowsWithGap(t)}
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
  const FlatLens = ({ items }: { items: { pv: ProjectView; p: Place }[] }) => (
    <ul className="places flat">
      {items.length === 0 && <li className="flat-empty">Nothing here.</li>}
      {items.map(({ pv, p }) => <PlaceRow key={pv.root + p.slug} repo={pv.root} p={p} showProject />)}
    </ul>
  );

  // Recent = the tree flattened and ranked, not a second opinion about time.
  // It used to rank (and label) by `usedEpoch`, so the same place could sit
  // third here and first in the tree with two different ages next to it.
  // `donePaths` is a dep because `activityAt` reads it.
  const recentItems = useMemo(
    () => allPlaces.filter(({ p }) => matchPlace(p) && !p.is_main)
      .sort((a, b) => activityAt(b.p) - activityAt(a.p)),
    [allPlaces, q, donePaths],
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
  //
  // The DOCK IS NOT A COLUMN. It and the terminal share one `.space` cell so the
  // space header can crown both (see `.space` in App.css) — a header spanning
  // two grid columns can't be expressed here, because these columns are
  // *removed* when hidden, not zeroed, so every line index shifts with ⌘B.
  const gridCols = [
    "var(--rail-w)",
    fit.navShown ? `${fit.navW}px` : null,
    "minmax(0, 1fr)", // .space — terminal + dock, under one header
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
        {/* The same three-way menu as the footer's: with the nav collapsed this
            is the ONLY way in, and a shortcut that silently answers one of the
            three questions is the inconsistency the menu exists to remove. */}
        <button className="rail-icon" title="add project" data-testid="add-menu-rail" onClick={openAddMenu}><Icons.FolderPlus size={17} /></button>
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
        <div className="nav-scroll" ref={navScrollRef}>
          {ws && ws.projects.length === 0 && <div className="empty small">No projects yet.<br />Add one from the rail.</div>}
          {lens === "places" && ws?.projects.map((pv) => (
            <Fragment key={pv.root}>
              {projGapAt(pv.root) && <div className="drop-gap proj" />}
              <ProjectNode pv={pv} />
            </Fragment>
          ))}
          {lens === "places" && projGapAt(null) && <div className="drop-gap proj" />}
          {lens === "recent" && <FlatLens items={recentItems} />}
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
        {/* One footer row, three ways in (menu below). It no longer picks a
            folder on click: "add a project" is a question with three answers
            now, and the folder pick is only one of them. */}
        <button className="add-footer with-icon" data-testid="add-menu-open" onClick={openAddMenu}>
          <Icons.Plus size={13} /> Add project<span className="add-caret">▾</span>
        </button>
        <div className="nav-resizer" onMouseDown={onResize} />
      </aside>

      {/* ── space: the selected place's whole workbench ──
          The header crowns BOTH the terminal and the dock, because both belong
          to the place. Before this the header lived inside `main` and the dock
          sat outside it as a sibling, which is precisely what made Files and
          Terminal read as app furniture pointed at a place rather than as parts
          of it. */}
      <div className="space">
        {/* tmux missing is an APP-level condition, not a per-place one, so it
            stays above the space header rather than under it. */}
        {!tmuxOk && <TmuxBanner onRecheck={recheckTmux} />}

        {selected && sel && (
          <>
            <header className="topbar">
              <div className="identity">
                {renaming ? (
                  <TitleEditor
                    key={sel.repo + "|" + sel.slug}
                    initial={selected.declared?.title ?? ""}
                    slug={selected.slug}
                    onCommit={(v) => setTitle(sel.repo, sel.slug, v)}
                    onCancel={() => setRenaming(false)}
                  />
                ) : (
                  <>
                    <b
                      className="slug"
                      title={selected.declared?.title ? "Double-click to rename" : "Double-click to name this place"}
                      onDoubleClick={() => setRenaming(true)}
                    >
                      {selected.is_main ? "◆ " : ""}{nameOf(selected)}
                    </b>
                    {/* renamed: the slug still names the directory and the tmux
                        session, so it stays on screen rather than being replaced */}
                    {selected.declared?.title?.trim() ? (
                      <span className="slug-alias" title="worktree directory and tmux session name">{selected.slug}</span>
                    ) : null}
                  </>
                )}
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
                  onClick={() => {
                    const on = !selected.declared?.pinned;
                    patchDeclared(sel.repo, sel.slug, { pinned: on });
                    mutate(invoke("set_pin", { repo: sel.repo, slug: sel.slug, on }));
                  }}>
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

                {/* The menu is no longer gated on `!is_main`: Rename and Copy
                    path apply to the main checkout too — its slug is the
                    literal "(main)", which is exactly where a real name helps
                    most. Only Remove stays main-only. */}
                <div className="menu-wrap">
                  <button className="ctrl icon-only" title="more actions" onClick={() => (menu === "more" ? closeMenu() : (setConfirmRm(null), setMenu("more")))}><Icons.Ellipsis /></button>
                  {menu === "more" && (
                    <div className="popover right">
                      <button className="pop-item" onClick={() => { closeMenu(); setRenaming(true); }}>
                        {selected.declared?.title ? "Rename…" : "Name this place…"}
                      </button>
                      {selected.declared?.title && (
                        <button className="pop-item" onClick={() => { closeMenu(); setTitle(sel.repo, sel.slug, ""); }}>
                          Clear name (show <code>{selected.slug}</code>)
                        </button>
                      )}
                      <button className="pop-item" onClick={() => copyText(selected.path)}>Copy path</button>
                      {!selected.is_main && (
                        confirmRm === `${sel.repo}|${sel.slug}` ? (
                          <>
                            <button className="pop-item danger armed" onClick={() => confirmRemove(false)}>Confirm remove</button>
                            <button className="pop-item danger armed" onClick={() => confirmRemove(true)}>Confirm remove + branch</button>
                          </>
                        ) : (
                          <button className="pop-item danger" onClick={armRemove}>Remove worktree…</button>
                        )
                      )}
                    </div>
                  )}
                </div>
              </div>
            </header>

            <input className="note-strip" placeholder="note…" defaultValue={selected.declared?.note ?? ""}
              key={sel.repo + sel.slug + (selected.declared?.note ?? "")}
              onBlur={(e) => {
                const note = e.currentTarget.value;
                patchDeclared(sel.repo, sel.slug, { note: note.trim() || undefined });
                mutate(invoke("set_note", { repo: sel.repo, slug: sel.slug, note }));
              }} />
          </>
        )}

        {/* ── space body: terminal and dock, side by side under the header ──
            `position: relative` lives here now (it used to be on `.main`) so it
            anchors the reading overlay — which therefore reaches across the
            dock for free, instead of needing a negative inline offset. */}
        <div className="space-body">
          <main className="main">
            {selected && sel ? (
              <>
                {selected.tmux_session.up ? (
                  <TerminalPane key={selected.tmux_session.name} session={selected.tmux_session.name} termVersion={termVersion} focusToken={termFocus}
                    findOpen={findOn === "main"} findToken={findToken} onFindClose={closeFind} />
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
                {/* The two moments a workspace starts from: a new user makes a
                    project, a new MACHINE imports one it already owns. Adding a
                    repo that is already on disk is the third way in and lives in
                    the nav footer's menu, one click away, beside these. */}
                <div className="home-actions">
                  <button className="enter-btn big home-open with-icon" data-testid="new-project-home" onClick={openNewProject}>
                    <Icons.Plus size={15} /> New project…
                  </button>
                  <button className="ctrl home-import with-icon" data-testid="import-open-home" onClick={openImport}>
                    <Icons.Import size={13} /> Import from hub…
                  </button>
                </div>
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
                      <span className="rr-name">{p.declared?.pinned ? "★ " : ""}{nameOf(p)}</span>
                      <span className="rr-proj">{basename(pv.root)}</span>
                      <span className="rr-life">{p.lifecycle_effective}</span>
                      <span className="rr-age">{ago(activityAt(p))}</span>
                      <button className="enter-btn sm with-icon">Enter <Icons.ChevronRight size={12} /></button>
                    </div>
                  ))}
                </div>
              </div>
            )}
          </main>

          {/* ── right dock: Files (browse + edit) / Terminal (embedded shell) ──
              A flex sibling of `main` inside the space body — no longer a grid
              column of its own. That is what puts it under the space header
              instead of beside it. */}
          {dockShown && selected && sel && (
            <aside className="dock" style={{ flex: `0 0 ${fit.dockW}px` }}>
              <div className="dock-resizer" onMouseDown={onDockResize} />
              {/* the rail owns tab selection AND collapse, so this is a title, not
                  a control strip */}
              <div className="dock-tabs">
                <span className="dock-title">{eff.dock_tab === "files" ? "Files" : "Terminal"}</span>
                <span className="dock-spacer" />
                {eff.dock_tab === "files" && (
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
                {eff.dock_tab === "files" ? (
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
                    mdZoom={eff.files_md_zoom}
                    onMdZoom={(v) => updatePanels({ files_md_zoom: v })}
                    expanded={false}
                    onExpand={(v) => setReading(v)}
                    findOpen={findOn === "dock"}
                    findToken={findToken}
                    onFindClose={closeFind}
                  />
                ) : (
                  <TerminalTabs key={sel.repo + "|" + sel.slug}
                    repo={sel.repo} slug={sel.slug} sessionUp={selected.tmux_session.up}
                    termVersion={termVersion} focusToken={termFocus} addToken={newTermToken}
                    names={(settings.term_tab_names ?? {})[sel.repo + "|" + sel.slug] ?? {}}
                    onRename={(index, name) => renameTermTab(sel.repo, sel.slug, index, name)}
                    tabs={(settings.term_tabs ?? {})[sel.repo + "|" + sel.slug] ?? []}
                    onTabs={(ids) => setTermTabs(sel.repo, sel.slug, ids)}
                    activeTab={(settings.term_tab_active ?? {})[sel.repo + "|" + sel.slug] ?? null}
                    onActiveTab={(index) => setTermTab(sel.repo, sel.slug, index)}
                    onError={fail}
                    findOpen={findOn === "dock"} findToken={findToken} onFindClose={closeFind} />
                )}
              </div>
            </aside>
          )}

          {/* ── reading mode (⌘⇧E): the dock's open file over the whole space
              body. The terminal stays MOUNTED underneath (hidden, not
              unmounted) — unmounting would drop the xterm and its scrollback;
              TerminalPane refits when it is revealed again.
              It reaches across the dock because it is anchored to
              `.space-body`, which is exactly as wide as terminal + dock. The
              old negative-right offset compensated for an overlay trapped
              inside `main`; there is nothing left to compensate for. */}
          {reading && dockFile && selected && (
            <div className="reading" role="dialog" aria-label="File reader">
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
                mdZoom={eff.files_md_zoom}
                onMdZoom={(v) => updatePanels({ files_md_zoom: v })}
                expanded
                onExpand={(v) => setReading(v)}
                findOpen={findOn === "read"}
                findToken={findToken}
                onFindClose={closeFind}
              />
            </div>
          )}
        </div>
      </div>

      {/* ── right rail: mirrors the left one. Permanent, so the dock always has
          a visible affordance; the active icon collapses the dock. Disabled
          with no place selected — Files/Terminal both need a worktree. */}
      <nav className="rail rail-right">
        {DOCK_RAIL.map((d) => {
          const on = dockShown && eff.dock_tab === d.key;
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

      {/* error surface lives OUTSIDE the nav — must stay visible in rail-only
          mode. ONE bottom-anchored stack: the undo banner and the landing note
          routinely arrive from the same drop, and as two fixed elements at the
          same coordinates the undo banner sat exactly on top of the note it
          was supposed to accompany. Undo first in DOM = above on screen. */}
      {(err || notice || undo) && (
        <div className="float-stack">
          {/* One click back from a slip of the wrist — a drag rewrites declared
              state, and the row it moves can land in a collapsed group or a
              hidden tier, i.e. out of sight of the gesture that moved it. */}
          {undo && (
            <div className="err err-float undo">
              {undo.text}
              <button className="undo-btn" onClick={undo.run}>Undo</button>
            </div>
          )}
          {err && <div className="err err-float" title="dismiss" onClick={() => setErr("")}>{err}</div>}
          {!err && notice && <div className="err err-float notice" title="dismiss" onClick={() => setNotice("")}>{notice}</div>}
        </div>
      )}
      {/* The dragged row's label, riding the pointer. Rendered at the app root
          so it is never clipped by the nav's own overflow. */}
      {drag && (
        <div
          className={"drag-ghost" + (drag.target?.kind === "reject" ? " no" : "")}
          style={{ left: drag.x, top: drag.y }}
        >
          {drag.item.kind === "project" ? basename(drag.item.root) : nameOf({ slug: drag.item.slug, declared: placeAt(drag.item.repo, drag.item.slug)?.declared })}
          {drag.target?.kind === "reject" && <span className="drag-why">{drag.target.hint}</span>}
          {drag.target?.kind === "tier" && <span className="drag-why">→ {TIER_LABEL[drag.target.lands]}</span>}
        </div>
      )}

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
        <QuickSwitch open items={allPlaces} rank={activityAt} busyPaths={busyPaths} waitingPaths={waitingPaths}
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
      {newFor && (
        <NewPlaceDialog
          // The draft is part of the identity: a REJECTED create reopens the
          // dialog for the same project, and without it in the key React would
          // reuse the old instance and its now-stale field state.
          key={newFor + "|" + newBase + "|" + (newDraft?.branch ?? "")}
          project={newFor}
          prefix={ws?.projects.find((p) => p.root === newFor)?.snapshot?.prefix ?? ""}
          places={ws?.projects.find((p) => p.root === newFor)?.snapshot?.places ?? []}
          unborn={unbornProjects.has(newFor)}
          initial={newDraft}
          initialBase={newBase}
          onCreate={(b, n, ba) => createPlace(newFor, b, n, ba)}
          onClose={() => { setNewFor(null); setNewBase(""); setNewDraft(null); }}
          onOpenPlace={(slug) => {
            setNewFor(null); setNewBase(""); setNewDraft(null);
            setSel({ repo: newFor, slug });
          }}
          onInitialCommit={createInitialCommit}
          onError={fail}
        />
      )}
      {npOpen && (
        <NewProjectDialog
          defaultLocation={defaultLocation}
          busy={npBusy}
          error={npErr}
          onBrowse={browseLocation}
          onCreate={createProject}
          onClose={() => { setNpOpen(false); setNpErr(""); }}
        />
      )}
      {importOpen && (
        <ImportPicker
          list={importList}
          loading={importLoading}
          error={importErr}
          known={knownRoots}
          onPick={pickImport}
          onClose={() => setImportOpen(false)}
        />
      )}
      {sync && (
        <SyncModal
          dir={sync.dir}
          project={sync.adopt ? sync.adopt.name : basename(sync.root)}
          adopt={sync.adopt}
          preview={syncPrev}
          loading={syncLoading}
          busy={syncBusy}
          error={syncErr}
          live={syncLive}
          done={syncDone}
          sessions={settings.sync_with_sessions}
          onSessions={(on) => updateSettings({ sync_with_sessions: on })}
          install={syncInstall}
          onInstall={setSyncInstall}
          progress={syncProg}
          onConfirm={confirmSync}
          onClose={closeSync}
        />
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
              <button className="pop-item" onClick={() => {
                closeCtx();
                const on = !ctxPlace.declared?.pinned;
                patchDeclared(ctx.repo, ctxPlace.slug, { pinned: on });
                mutate(invoke("set_pin", { repo: ctx.repo, slug: ctxPlace.slug, on }));
              }}>
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

      {/* ── the nav footer's add menu (opens upward: CtxMenu clamps it in) ── */}
      {ctx?.kind === "add" && (
        <CtxMenu x={ctx.x} y={ctx.y} onClose={closeCtx}>
          <div className="pop-hint">add a project</div>
          {/* Each item closes the menu FIRST: the folder dialog is native and
              modal, and a popover left open under it is still open when it
              returns (closeMenu discipline). */}
          <button className="pop-item" data-testid="add-new"
            onClick={openNewProject}>New project…</button>
          <button className="pop-item" data-testid="add-existing"
            onClick={() => { closeCtx(); addProject(); }}>Add existing…</button>
          <button className="pop-item" data-testid="add-import"
            onClick={() => { closeCtx(); openImport(); }}>Import from hub…</button>
        </CtxMenu>
      )}

      {/* ── the ⇄ popover (anchored to the project header's mini button) ── */}
      {ctx?.kind === "sync" && (
        <CtxMenu x={ctx.x} y={ctx.y} onClose={closeCtx}>
          <SyncPopover status={syncStatus[ctx.root]} root={ctx.root} onOpen={openSync} />
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
            {pv?.ok && <SyncMenuItems status={syncStatus[ctx.root]} root={ctx.root} onOpen={openSync} />}
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
