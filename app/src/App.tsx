import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import * as Icons from "./icons";
import { TerminalPane } from "./TerminalPane";
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
  declared: Declared;
  lifecycle_effective: string;
};
type Snapshot = { repo: string; prefix: string; places: Place[] };
type ProjectView = { root: string; ok: boolean; error: string | null; snapshot: Snapshot | null };
type Workspace = { projects: ProjectView[] };
/** `needs_confirm` (close only): core stopped because killing this session needs
 *  the user's word, and the string is the session that would die. Not a failure
 *  — a question, so it must never reach the error banner. */
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
  if (p.ahead) g.push({ cls: "g-ahead", text: `↑${p.ahead}`, title: `${p.ahead} ahead` });
  if (p.behind) g.push({ cls: "g-behind", text: `↓${p.behind}`, title: `${p.behind} behind` });
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

// `code spans` → <code>; the only inline markup the changelog uses.
function renderInline(s: string): React.ReactNode[] {
  return s.split(/`([^`]+)`/g).map((part, i) => (i % 2 ? <code key={i}>{part}</code> : part));
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
function NewPlaceForm({ project, initialBase, onCreate, onCancel }: {
  project: string;
  initialBase: string;
  onCreate: (branch: string, name: string, base: string) => void;
  onCancel: () => void;
}) {
  const [branch, setBranch] = useState("");
  const [name, setName] = useState("");
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

// ── ⌘K quick-switcher ──
// Fuzzy SUBSEQUENCE match over a composite key (slug + branch + project basename
// + note). A place matches if the query chars appear IN ORDER (case-insensitive);
// score prefers a contiguous substring hit over a scattered subsequence, an
// earlier hit over a later one, and a slug hit over a branch/project hit. Recency
// (declared.last_opened_epoch desc) breaks ties. Empty query → all places sorted
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
    const rec = (p: Place) => p.declared?.last_opened_epoch ?? p.last_commit_epoch ?? 0;
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

// ── right dock: file browser + editable viewer + embedded terminal ───────────
// All at MODULE scope (stable identity — components defined inside App() remount
// every render and would lose tree-expansion / textarea focus).

type FsEntry = { name: string; path: string; is_dir: boolean };

// One lazy directory node: fetches its children the first time it's expanded and
// caches them. Files bubble a click up via onOpen; dirs toggle.
function TreeNode({ entry, depth, openPath, onOpen, onError }: {
  entry: FsEntry; depth: number; openPath: string | null; onOpen: (path: string) => void; onError: (e: unknown) => void;
}) {
  const [open, setOpen] = useState(false);
  const [kids, setKids] = useState<FsEntry[] | null>(null);
  const [loading, setLoading] = useState(false);

  const toggle = async () => {
    if (!entry.is_dir) { onOpen(entry.path); return; }
    const next = !open;
    setOpen(next);
    if (next && kids === null && !loading) {
      setLoading(true);
      // Surface a failure (permissions, backend error) rather than showing it as
      // an empty directory — a swallowed error reads as "it just didn't respond".
      try { setKids(await invoke<FsEntry[]>("list_dir", { path: entry.path })); }
      catch (e) { setKids([]); onError(e); }
      finally { setLoading(false); }
    }
  };

  const isSel = !entry.is_dir && openPath === entry.path;
  return (
    <div className="tree-node">
      <button
        className={"tree-row" + (isSel ? " sel" : "") + (entry.is_dir ? " dir" : "")}
        style={{ paddingLeft: `calc(var(--s2) + ${depth} * var(--s3))` }}
        onClick={toggle}
        title={entry.name}
      >
        <span className="tree-caret">{entry.is_dir && (open ? <Icons.ChevronDown size={11} /> : <Icons.ChevronRight size={11} />)}</span>
        <span className="tree-name">{entry.name}</span>
      </button>
      {entry.is_dir && open && (
        <div className="tree-kids">
          {loading && <div className="tree-note">…</div>}
          {kids && kids.length === 0 && !loading && <div className="tree-note">empty</div>}
          {kids?.map((k) => (
            <TreeNode key={k.path} entry={k} depth={depth + 1} openPath={openPath} onOpen={onOpen} onError={onError} />
          ))}
        </div>
      )}
    </div>
  );
}

// Files tab tree. `root` = the place's worktree path; remount per place via key.
function FileTree({ root, openPath, onOpen, onError }: { root: string; openPath: string | null; onOpen: (path: string) => void; onError: (e: unknown) => void }) {
  const [entries, setEntries] = useState<FsEntry[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    setEntries(null); setErr(null);
    invoke<FsEntry[]>("list_dir", { path: root })
      .then((e) => { if (alive) setEntries(e); })
      .catch((e) => { if (alive) setErr(String(e)); });
    return () => { alive = false; };
  }, [root]);
  if (err) return <div className="tree-note err-note">{err}</div>;
  if (!entries) return <div className="tree-note">loading…</div>;
  if (!entries.length) return <div className="tree-note">empty worktree</div>;
  return (
    <div className="filetree">
      {entries.map((e) => <TreeNode key={e.path} entry={e} depth={0} openPath={openPath} onOpen={onOpen} onError={onError} />)}
    </div>
  );
}

// Editable viewer. Reads on path change; plain textarea (no editor lib). ⌘S /
// Save writes back. Binary + truncated files are read-only (partial saves would
// corrupt), with an "Open in editor" escape hatch. Because Claude edits the same
// tree in another pane, this guards against clobbering: it auto-reloads from disk
// while the buffer is clean (`reloadToken` bumps on places:changed), and on save
// passes the file's mtime as a compare-and-swap token — a diverged file is
// refused, not overwritten.
type FileRead = { content: string; truncated: boolean; binary: boolean; mtime: number };
function FileViewer({ path, reloadToken, onOpenEditor, onError }: {
  path: string; reloadToken: number; onOpenEditor: (path: string) => void; onError: (e: unknown) => void;
}) {
  const [orig, setOrig] = useState("");
  const [text, setText] = useState("");
  const [meta, setMeta] = useState<{ binary: boolean; truncated: boolean; mtime: number } | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [conflict, setConflict] = useState(false);

  const load = useCallback(() => {
    let alive = true;
    setLoading(true); setMeta(null); setConflict(false);
    invoke<FileRead>("read_file", { path })
      .then((r) => { if (!alive) return; setOrig(r.content); setText(r.content); setMeta({ binary: r.binary, truncated: r.truncated, mtime: r.mtime }); })
      .catch((e) => { if (alive) onError(e); })
      .finally(() => { if (alive) setLoading(false); });
    return () => { alive = false; };
  }, [path, onError]);

  useEffect(() => load(), [load]); // (re)load on path change

  const dirty = text !== orig;
  const canEdit = !!meta && !meta.binary && !meta.truncated;

  // Auto-refresh from disk when the tree changed elsewhere AND the buffer is
  // clean — never nuke unsaved edits (the save-time CAS covers the dirty case).
  const dirtyRef = useRef(false); dirtyRef.current = dirty;
  const firstReload = useRef(true);
  useEffect(() => {
    if (firstReload.current) { firstReload.current = false; return; }
    if (!dirtyRef.current) load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [reloadToken]);

  const save = async () => {
    if (!dirty || saving || !canEdit) return;
    setSaving(true);
    try {
      await invoke("write_file", { path, content: text, expectedMtime: meta?.mtime ?? null });
      setOrig(text); setConflict(false);
      const r = await invoke<FileRead>("read_file", { path }); // pick up the new mtime for the next CAS
      setMeta((m) => (m ? { ...m, mtime: r.mtime } : m));
    } catch (e) {
      if (String(e).includes("changed on disk")) setConflict(true);
      onError(e);
    } finally { setSaving(false); }
  };

  return (
    <div className="viewer">
      <div className="viewer-h">
        <span className="viewer-path" title={path}>{basename(path)}{dirty ? " ●" : ""}</span>
        {meta?.truncated && <span className="viewer-tag">truncated</span>}
        {conflict && <span className="viewer-tag conflict">changed on disk</span>}
        <span className="dock-spacer" />
        {conflict && <button className="ctrl sm" onClick={load}>Reload</button>}
        {canEdit && <button className="ctrl sm" disabled={!dirty || saving} onClick={save}>{saving ? "Saving…" : "Save"}</button>}
        <button className="ctrl sm" onClick={() => onOpenEditor(path)}>Editor</button>
      </div>
      {loading ? (
        <div className="tree-note">loading…</div>
      ) : meta?.binary ? (
        <div className="tree-note">binary file — <button className="linklike" onClick={() => onOpenEditor(path)}>open in editor</button></div>
      ) : (
        <textarea
          className="viewer-text"
          value={text}
          spellCheck={false}
          readOnly={!canEdit}
          onChange={(e) => setText(e.currentTarget.value)}
          onKeyDown={(e) => {
            if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "s") { e.preventDefault(); save(); }
          }}
        />
      )}
    </div>
  );
}

// One embedded shell: ensures the place's sidecar for tab `index` exists, then
// attaches. The backend derives the session name + cwd from repo+slug (the
// webview never names a session), so it stays stable across session up/down.
function DockTerminal({ repo, slug, index, termVersion, focusToken, onError }: {
  repo: string; slug: string; index: number; termVersion: number; focusToken: number; onError: (e: unknown) => void;
}) {
  const [shell, setShell] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    setShell(null);
    invoke<string>("open_shell_session", { repo, slug, index })
      .then((s) => { if (alive) setShell(s); })
      .catch((e) => { if (alive) onError(e); });
    return () => { alive = false; };
  }, [repo, slug, index]);
  if (!shell) return <div className="tree-note">starting shell…</div>;
  return <TerminalPane session={shell} termVersion={termVersion} focusToken={focusToken} />;
}

// Terminal tab: several shells per place. Tabs are restored from the live tmux
// sidecars (self-healing across restarts); a live place defaults to one shell, a
// closed place to none. Only the active shell is mounted (tmux keeps the rest
// warm). `addToken` bumps → add a tab (⌘⇧T from the global handler).
function TerminalTabs({ repo, slug, sessionUp, termVersion, focusToken, addToken, onError }: {
  repo: string; slug: string; sessionUp: boolean; termVersion: number; focusToken: number; addToken: number; onError: (e: unknown) => void;
}) {
  const [ids, setIds] = useState<number[] | null>(null); // null = restoring
  const [active, setActive] = useState<number | null>(null);
  const idsRef = useRef<number[]>([]);
  idsRef.current = ids ?? [];
  const restoringRef = useRef(true);

  // Restore tabs from live tmux on mount / place change.
  useEffect(() => {
    let alive = true;
    restoringRef.current = true;
    setIds(null); setActive(null);
    invoke<number[]>("list_shell_sessions", { repo, slug })
      .then((existing) => {
        if (!alive) return;
        const list = existing.length ? existing : (sessionUp ? [1] : []);
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

  // When the place's session goes DOWN (topbar Close swept its sidecars), clear
  // the tabs so the dock reflects reality instead of resurrecting dead shells.
  const prevUp = useRef(sessionUp);
  useEffect(() => {
    if (prevUp.current && !sessionUp) { setIds([]); setActive(null); }
    prevUp.current = sessionUp;
  }, [sessionUp]);

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
    const remaining = idsRef.current.filter((x) => x !== id);
    setIds(remaining);
    if (active === id) setActive(remaining.length ? remaining[remaining.length - 1] : null);
  };

  if (ids === null) return <div className="tree-note">…</div>;

  return (
    <div className="termtabs">
      <div className="termtab-strip">
        {ids.map((id) => (
          <span key={id} className={"termtab" + (active === id ? " on" : "")}>
            <button className="termtab-label" onClick={() => setActive(id)}>sh {id}</button>
            <button className="termtab-x" title="close shell" onClick={() => closeTab(id)}><Icons.X size={11} /></button>
          </span>
        ))}
        <button className="termtab-add" title="new terminal (⌘⇧T)" onClick={addTab}><Icons.Plus size={13} /></button>
      </div>
      {active != null ? (
        <DockTerminal key={repo + "|" + slug + ":" + active} repo={repo} slug={slug}
          index={active} termVersion={termVersion} focusToken={focusToken} onError={onError} />
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

function App() {
  const [ws, setWs] = useState<Workspace | null>(null);
  const [err, setErr] = useState("");
  const [sel, setSel] = useState<{ repo: string; slug: string } | null>(null);
  const [filter, setFilter] = useState("");
  const [lens, setLens] = useState<Lens>("places");
  const [newFor, setNewFor] = useState<string | null>(null);
  const [newBase, setNewBase] = useState("");
  const [ctx, setCtx] = useState<Ctx | null>(null);
  const [switchTo, setSwitchTo] = useState("");
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
  useEffect(() => { setDockFile(null); }, [sel?.repo, sel?.slug]);
  // ⌘⇧T bumps this → the dock's Terminal tab adds a shell (if mounted/visible).
  const [newTermToken, setNewTermToken] = useState(0);
  // bumps on places:changed → the dock's file viewer re-reads from disk when clean.
  const [placesToken, setPlacesToken] = useState(0);

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
  const refresh = useCallback(async () => {
    try {
      const w = await invoke<Workspace>("list_workspace");
      // Read the claim into a local FIRST: the updater below runs when React gets
      // to it, by which time the ref would already be cleared and every message
      // would look like someone else's.
      const stale = refreshErr.current;
      refreshErr.current = "";
      if (stale) setErr((e) => (e === stale ? "" : e));
      setWs(w);
    } catch (e) {
      refreshErr.current = String(e); // same text fail() shows
      fail(e);
    }
  }, []);
  useEffect(() => { refresh(); }, [refresh]);

  // live refresh: backend emits "places:changed" (poll/fs-watch) → re-pull
  useEffect(() => {
    const un = listen("places:changed", () => { refresh(); setPlacesToken((v) => v + 1); });
    return () => { un.then((f) => f()).catch(() => {}); };
  }, [refresh]);

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
  // Which roots have already had their once-per-project init probe (below).
  // Declared here because the sweep effect prunes it alongside the two maps.
  const suggestedFor = useRef<Set<string>>(new Set());
  useEffect(() => {
    const roots = rootsKey ? rootsKey.split("\n") : [];
    // A removed project must not keep its drift decoration, its suggestion, or
    // its once-only probe marker alive for the rest of the session — re-adding it
    // would then show stale findings and never re-probe.
    const live = new Set(roots);
    for (const r of suggestedFor.current) if (!live.has(r)) suggestedFor.current.delete(r);
    const prune = <T,>(m: Record<string, T>) =>
      Object.keys(m).every((k) => live.has(k))
        ? m
        : Object.fromEntries(Object.entries(m).filter(([k]) => live.has(k)));
    setHealth((m) => prune(m));
    setSuggest((m) => prune(m));
    if (roots.length === 0) return;
    const sweep = () => roots.filter((r) => okRoots.current.has(r)).forEach((r) => sweepDoctor(r));
    sweep();
    const t = setInterval(sweep, 5 * 60_000); // slow timer, not the poll
    return () => clearInterval(t);
  }, [rootsKey, sweepDoctor]);
  // The suggestion is stable for a given checkout, so probe each project once;
  // `probeSuggest` re-runs it after a config is written (the banner retires).
  const probeSuggest = useCallback(async (root: string) => {
    try {
      const s = await invoke<InitSuggestion | null>("init_suggest", { repo: root });
      if (s) setSuggest((m) => ({ ...m, [root]: s }));
    } catch (e) {
      invoke("log_event", { level: "warn", msg: `init_suggest ${root}: ${String(e)}` }).catch(() => {});
    }
  }, []);
  useEffect(() => {
    for (const root of rootsKey ? rootsKey.split("\n") : []) {
      if (suggestedFor.current.has(root)) continue;
      suggestedFor.current.add(root);
      probeSuggest(root);
    }
  }, [rootsKey, probeSuggest]);

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
  const fit = fitLayout(settings, !!selected && !!sel, vw);
  const dockShown = fit.dockShown;
  // Would the dock fit if it were open? Drives the toggle's disabled state, so a
  // ⌘J that can't visibly do anything is at least honest about why.
  const dockFits = fitLayout({ ...settings, dock_open: true }, !!selected && !!sel, vw).dockShown;

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
      return r;
    } catch (e) {
      fail(e);
      return null;
    }
  };

  const addProject = async () => {
    try {
      const dir = await open({ directory: true, title: "Add a git project" });
      if (typeof dir === "string") { setErr(""); setWs(await invoke<Workspace>("add_project", { dir })); }
    } catch (e) { fail(e); }
  };
  const removeProject = async (root: string) => {
    try {
      setWs(await invoke<Workspace>("remove_project", { root }));
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
    if ((await runCmd("remove_place", { repo, slug, del_branch: delBranch, force: false }))?.ok) {
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
  const recencyOf = (p: Place) => p.declared?.last_opened_epoch ?? p.last_commit_epoch ?? 0;
  const sortPlaces = (repo: string, arr: Place[]): Place[] => {
    const out = [...arr];
    if (settings.sort_mode === "manual") {
      const order = settings.manual_order[repo] ?? [];
      const idx = (p: Place) => { const i = order.indexOf(p.slug); return i < 0 ? order.length : i; };
      out.sort((a, b) => idx(a) - idx(b));
      return out;
    }
    if (settings.sort_mode === "alpha") out.sort((a, b) => a.slug.localeCompare(b.slug));
    else out.sort((a, b) => recencyOf(b) - recencyOf(a));
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
    const r = await runCmd("new_place", { repo, branch, base: base || null, name: name || null });
    if (r?.ok) {
      // Select core's ACTUAL final slug (origin/ stripped, holder-reuse applied);
      // fall back to the old derivation only if the backend didn't report one.
      setSel({ repo, slug: r.slug ?? (name || branch).replace(/\//g, "-") });
      setNewFor(null);
      setNewBase("");
    }
  };
  const doSwitch = async () => {
    if (!sel) return;
    const b = switchTo.trim();
    if (!b) return;
    if ((await runCmd("switch_place", { repo: sel.repo, slug: sel.slug, branch: b, base: null }))?.ok) setSwitchTo("");
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
    if ((await runCmd("remove_place", { repo: sel.repo, slug: sel.slug, del_branch: delBranch, force: false }))?.ok) setSel(null);
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
  const keyRef = useRef({ selectLens, selectedPath: null as string | null, editorCmd: settings.editor_cmd, switchOpen });
  keyRef.current = { selectLens, selectedPath: selected?.path ?? null, editorCmd: settings.editor_cmd, switchOpen };
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
      .sort((a, b) => (b.p.declared?.last_opened_epoch ?? 0) - (a.p.declared?.last_opened_epoch ?? 0))
      .slice(0, 6),
    [allPlaces],
  );

  // Restore last place on launch (opt-in). SELECTION-ONLY: this calls setSel and
  // nothing else — NO enterPlace/touch_place/open_place (those would auto-resume a
  // Claude session on every reboot). TerminalPane attaches on its own if the tmux
  // session is up; otherwise the place view shows its normal "Enter ▸ to start".
  // Target = the most recently opened place across all projects (max
  // last_opened_epoch, main excluded) — same derivation as the Resume list, so
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
  const PlaceRow = ({ repo, p, showProject }: { repo: string; p: Place; showProject?: boolean }) => {
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
        <span
          className={"status-dot" + (activityOf(p) === "busy" ? " busy" : activityOf(p) === "waiting" ? " waiting" : "")}
          title={activityOf(p) === "busy" ? "Claude working" : activityOf(p) === "waiting" ? "Claude needs input" : undefined}
        />
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
          <span className="row-age">{ago(recencyOf(p))}</span>
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
    // Rollup: a working child dominates a waiting one — busy wins, else waiting.
    const rollup = places.some((p) => activityOf(p) === "busy")
      ? "busy"
      : places.some((p) => activityOf(p) === "waiting")
        ? "waiting"
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
            ? <span className={"picon" + (rollup ? " " + rollup : "")} title={rollup === "busy" ? "a session is working" : rollup === "waiting" ? "a session needs input" : undefined}><FolderIcon /></span>
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
          <button className="mini" title="new worktree" onClick={() => { setNewFor(newFor === pv.root ? null : pv.root); setNewBase(""); }}><Icons.Plus size={13} /></button>
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
                    <span className="caret">{opened ? "▾" : "▸"}</span>
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
  const FlatLens = ({ items }: { items: { pv: ProjectView; p: Place }[] }) => (
    <ul className="places flat">
      {items.length === 0 && <li className="flat-empty">Nothing here.</li>}
      {items.map(({ pv, p }) => <PlaceRow key={pv.root + p.slug} repo={pv.root} p={p} showProject />)}
    </ul>
  );

  const recentItems = useMemo(
    () => allPlaces.filter(({ p }) => matchPlace(p) && !p.is_main)
      .sort((a, b) => (b.p.declared?.last_opened_epoch ?? 0) - (a.p.declared?.last_opened_epoch ?? 0)),
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

  // `minmax(0, 1fr)` — a bare `1fr` is `minmax(auto, 1fr)`, which refuses to
  // shrink below the center pane's content and pushes the fixed columns off the
  // window instead of letting anything ellipsise.
  const gridCols = [
    "var(--rail-w)",
    fit.navShown ? `${fit.navW}px` : null,
    "minmax(0, 1fr)",
    dockShown ? `${fit.dockW}px` : null,
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
                {([["recent", "Last used"], ["alpha", "A–Z"], ["manual", "Manual (drag rows)"]] as const).map(([m, label]) => (
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
        {newFor && (
          <NewPlaceForm
            key={newFor + "|" + newBase}
            project={newFor}
            initialBase={newBase}
            onCreate={(b, n, ba) => createPlace(newFor, b, n, ba)}
            onCancel={() => { setNewFor(null); setNewBase(""); }}
          />
        )}
        <div className="nav-scroll">
          {ws && ws.projects.length === 0 && <div className="empty small">No projects yet.<br />Add one from the rail.</div>}
          {lens === "places" && ws?.projects.map((pv) => <ProjectNode key={pv.root} pv={pv} />)}
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
        <button className="add-footer with-icon" onClick={addProject}><Icons.Plus size={13} /> Add project</button>
        <div className="nav-resizer" onMouseDown={onResize} />
      </aside>

      {/* ── main ── */}
      <main className="main">
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
                    {(selected.ahead || selected.behind) && <span className="s ab">↑{selected.ahead ?? 0} ↓{selected.behind ?? 0}</span>}
                    <span className={"life " + selected.lifecycle_effective}>{selected.lifecycle_effective}</span>
                  </span>
                )}
              </div>

              <div className="controls">
                <button
                  className={"icon-btn" + (settings.dock_open ? " on" : "")}
                  disabled={!dockFits}
                  title={!dockFits ? "window too narrow for files & terminal" : settings.dock_open ? "hide files & terminal (⌘J)" : "files & terminal (⌘J)"}
                  onClick={toggleDock}
                >{settings.dock_open ? <Icons.PanelRightClose /> : <Icons.PanelRightOpen />}</button>
                {selected.tmux_session.up ? (
                  <>
                    <span className="live-badge" title="session live"><span className="status-dot on" /> live</span>
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
                    <span className="sb-label"><Icons.GitBranch size={13} /></span>
                    <input className="switchto" placeholder="switch branch…" value={switchTo}
                      onChange={(e) => setSwitchTo(e.currentTarget.value)}
                      onKeyDown={(e) => e.key === "Enter" && doSwitch()} />
                    <button className="ctrl sm" onClick={doSwitch} disabled={!switchTo.trim()}>Switch</button>
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
                  <span
                    className={"status-dot" + (activityOf(p) === "busy" ? " busy" : activityOf(p) === "waiting" ? " waiting" : "")}
                    title={activityOf(p) === "busy" ? "Claude working" : activityOf(p) === "waiting" ? "Claude needs input" : undefined}
                  />
                  <span className="rr-name">{p.declared?.pinned ? "★ " : ""}{p.slug}</span>
                  <span className="rr-proj">{basename(pv.root)}</span>
                  <span className="rr-life">{p.lifecycle_effective}</span>
                  <span className="rr-age">{ago(p.declared?.last_opened_epoch)}</span>
                  <button className="enter-btn sm with-icon">Enter <Icons.ChevronRight size={12} /></button>
                </div>
              ))}
            </div>
          </div>
        )}
      </main>

      {/* ── right dock: Files (browse + edit) / Terminal (embedded shell) ── */}
      {dockShown && selected && sel && (
        <aside className="dock">
          <div className="dock-resizer" onMouseDown={onDockResize} />
          <div className="dock-tabs">
            <button className={"dock-tab" + (settings.dock_tab === "files" ? " on" : "")}
              onClick={() => updateSettings({ dock_tab: "files" })}>Files</button>
            <button className={"dock-tab" + (settings.dock_tab === "terminal" ? " on" : "")}
              onClick={() => updateSettings({ dock_tab: "terminal" })}>Terminal</button>
            <span className="dock-spacer" />
            <button className="icon-btn" title="hide dock (⌘J)" onClick={toggleDock}><Icons.X /></button>
          </div>
          <div className="dock-body">
            {settings.dock_tab === "files" ? (
              <div className="dock-files">
                <div className="dock-tree">
                  <FileTree key={selected.path} root={selected.path} openPath={dockFile} onOpen={setDockFile} onError={fail} />
                </div>
                {dockFile
                  ? <FileViewer key={dockFile} path={dockFile} reloadToken={placesToken} onOpenEditor={editIn} onError={fail} />
                  : <div className="tree-note viewer-hint">select a file to view</div>}
              </div>
            ) : (
              <TerminalTabs key={sel.repo + "|" + sel.slug}
                repo={sel.repo} slug={sel.slug} sessionUp={selected.tmux_session.up}
                termVersion={termVersion} focusToken={termFocus} addToken={newTermToken} onError={fail} />
            )}
          </div>
        </aside>
      )}

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
        onShowNotes={showReleaseNotes} onReset={onReset} />

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
