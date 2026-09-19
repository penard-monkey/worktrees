// The dock's Files tab: a lazy tree beside (or above) a viewer that renders per
// file KIND — markdown, source, image, or a named placeholder for everything
// else.
//
// Read-only with ONE exception: a markdown file's Source view is a textarea you
// can type in, saved with ⌘S through `write_file`. The exception is narrow on
// purpose. Markdown source is the only kind this viewer renders WITHOUT
// highlighting (`filekind.ts` gives it `lang: ""` — prose has no grammar), so
// swapping a `<pre>` for a `<textarea>` costs it nothing but the line-number
// gutter. Do the same to a .rs file and it loses highlighting, the gutter and
// ⌘F's match painting in one go — the CSS Custom Highlight API cannot paint
// inside a textarea. Everything else still goes through "Open in editor", and
// no editor library came in (CLAUDE.md's "no UI libraries" names editors).
//
// What keeps this from clobbering what Claude is writing in the pane next door
// is `write_file`'s compare-and-swap: the save carries the mtime the edit
// STARTED from, and the backend refuses it if the file moved. Overwriting
// anyway is a second, separately-labelled click — never the default.
//
// Layout: `split` (tree left / content right) past a dock-width threshold,
// `stack` (tree above) below it, with a manual override. The divider drags and
// its ratio persists per orientation.
//
// Everything here is at MODULE scope per CLAUDE.md — components defined inside
// App() get a new identity every render and would drop tree expansion state.
import { Component, Fragment, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore, type CSSProperties, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
// `openPath` is aliased: this module already has an `openPath` — the prop
// naming the file the viewer has open — and the two would silently shadow each
// other inside FilesPane (tsc catches it as "String has no call signatures",
// which reads like a typing problem rather than a collision).
import { openPath as openInDefaultApp, openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import * as Icons from "./icons";
import { CodeBlock } from "./CodeView";
import { CtxMenu } from "./CtxMenu";
import { DiffView, type FileDiffDto } from "./DiffView";
import { FindBar, useFileFind } from "./Find";
import { Markdown } from "./markdown";
import { basename, fileInfo, humanSize, relPath, type FileKind } from "./filekind";
import { MD_ZOOM_MAX, MD_ZOOM_MIN, clampMdZoom, stepMdZoom, type Settings } from "./settings";
import { applyMd, spliceRange, type MdAction } from "./mdedit";

// `link` is the symlink target as written (relative stays relative);
// `link_block` is why the backend will not follow it — absent when it will.
type LinkBlock = "outside" | "git" | "missing";
type FsEntry = {
  name: string; path: string; is_dir: boolean; ignored?: boolean; link?: string | null; link_block?: LinkBlock | null;
  /** Client-side only: this row has no directory entry behind it. A deleted path
   *  is not on disk, so the tree INVENTS its row (and any directory the deletion
   *  took with it) rather than marking a folder whose changed child can't be
   *  shown. Never set by `list_dir`. */
  ghost?: true;
};

const BLOCK_WHY: Record<LinkBlock, string> = {
  outside: "outside the workspace, not followed",
  git: "inside .git, not followed",
  missing: "target is missing",
};

// ── change set ───────────────────────────────────────────────────────────
// What differs from the branch's BASE — committed on this branch and
// uncommitted alike. One `changed_files` call per refresh for the whole tree
// (see lib.rs); everything below turns that flat list into per-row answers.

/** Mirrors lib.rs `ChangeKind`. */
type ChangeKind = "modified" | "added" | "untracked" | "deleted";
type FileChange = { path: string; status: ChangeKind };
type ChangeSetDto = { root: string; files: FileChange[] };

type Changes = {
  /** the worktree top the backend resolved these against, canonical */
  root: string;
  files: Map<string, ChangeKind>;
  /** absolute dir → changed files anywhere beneath it: the upward cascade AND
   *  the count badge, which is the only way to tell a one-file directory from a
   *  rewritten subsystem without expanding it */
  dirs: Map<string, number>;
  /** absolute dir → ghost rows to splice into its listing (see `FsEntry.ghost`) */
  ghosts: Map<string, FsEntry[]>;
};
const NO_CHANGES: Changes = { root: "", files: new Map(), dirs: new Map(), ghosts: new Map() };

const parentOf = (p: string) => p.slice(0, p.lastIndexOf("/"));
const nameOf = (p: string) => p.slice(p.lastIndexOf("/") + 1);

/** `list_dir`'s order — directories first, then case-insensitive by name.
 *  Compared with `<`/`>` and not localeCompare, because the backend sorts by
 *  `to_lowercase().cmp()`, a plain codepoint compare: localeCompare would file a
 *  spliced ghost `éclair.md` next to "e" where the real listing puts it after
 *  "z", and the ghost would sit visibly out of order. */
const cmpEntries = (a: FsEntry, b: FsEntry) => {
  const an = a.name.toLowerCase(), bn = b.name.toLowerCase();
  return Number(b.is_dir) - Number(a.is_dir) || (an < bn ? -1 : an > bn ? 1 : 0);
};

/** Flat list → per-row lookups, once per refresh. Every changed path walks up to
 *  the root: an ancestor collects the count, and a DELETED path also leaves a
 *  ghost at each level, because `git rm -r tools/` takes the directory with it
 *  and there would otherwise be no row anywhere to hang the mark on. Ghosts a
 *  listing does have on disk are dropped at render (`withGhosts`), which is what
 *  keeps this from having to know what still exists. */
function buildChanges(dto: ChangeSetDto): Changes {
  const root = dto.root.replace(/\/+$/, "");
  const files = new Map<string, ChangeKind>();
  const dirs = new Map<string, number>();
  // dir → name → row, so two deleted files under one vanished directory yield a
  // single ghost dir instead of one per file.
  const byDir = new Map<string, Map<string, FsEntry>>();
  for (const c of dto.files) {
    if (!root || !c.path.startsWith(`${root}/`)) continue; // not in this tree
    files.set(c.path, c.status);
    let child = c.path;
    let dir = parentOf(child);
    while (dir.length >= root.length) {
      dirs.set(dir, (dirs.get(dir) ?? 0) + 1);
      if (c.status === "deleted") {
        let m = byDir.get(dir);
        if (!m) { m = new Map(); byDir.set(dir, m); }
        // Only the leaf is a file; every level above it is a directory that may
        // or may not still exist.
        if (!m.has(nameOf(child))) m.set(nameOf(child), { name: nameOf(child), path: child, is_dir: child !== c.path, ghost: true });
      }
      if (dir === root) break;
      child = dir;
      dir = parentOf(dir);
    }
  }
  return { root, files, dirs, ghosts: new Map([...byDir].map(([d, m]) => [d, [...m.values()]])) };
}

/** A listing plus the ghost rows for `dir` that are genuinely gone — anything
 *  the real listing still has wins, so a file deleted from the index but left on
 *  disk keeps its one real row. */
function withGhosts(dir: string, kids: FsEntry[], changes: Changes): FsEntry[] {
  const g = changes.ghosts.get(dir);
  if (!g?.length) return kids;
  const have = new Set(kids.map((k) => k.name));
  const add = g.filter((e) => !have.has(e.name));
  return add.length ? [...kids, ...add].sort(cmpEntries) : kids;
}

/** Does this row survive the "changes only" filter? A FILE has to have a status
 *  of its own; a DIRECTORY has to have something changed beneath it, which is
 *  exactly the count the badge already renders.
 *
 *  Ghost rows pass without a special case: a deleted file IS in `files`, and the
 *  vanished directory above it carries the count that put it there. */
const changedRow = (e: FsEntry, changes: Changes) =>
  e.is_dir ? (changes.dirs.get(e.path) ?? 0) > 0 : changes.files.has(e.path);

/** The word the row's tooltip uses. `untracked` rather than "added" for a file
 *  git has never seen: both are new, but only one of them is staged. */
const CHANGE_WHY: Record<ChangeKind, string> = {
  modified: "modified on this branch",
  added: "added on this branch",
  untracked: "untracked",
  deleted: "deleted on this branch",
};
type FileRead = { content: string; truncated: boolean; binary: boolean; mtime: number; size: number };
type FileBlob = { b64: string; size: number; truncated: boolean; mtime: number };

// ── tree ─────────────────────────────────────────────────────────────────

// One lazy directory node. Files bubble a click up via onOpen; dirs toggle.
// A right-click bubbles up too (onContext) — the menu itself belongs to the
// pane, so only one can ever be open and the row keeps no state for it.
function TreeNode({ entry, depth, openPath, showIgnored, reloadToken, changes, changedOnly, onOpen, onContext, onError }: {
  entry: FsEntry; depth: number; openPath: string | null; showIgnored: boolean; reloadToken: number;
  changes: Changes; changedOnly: boolean;
  onOpen: (path: string) => void; onContext: (e: React.MouseEvent, entry: FsEntry) => void;
  onError: (e: unknown) => void;
}) {
  const [open, setOpen] = useState(false);
  // Turning the filter on OPENS every surviving directory, and because a child
  // only mounts once its parent is open, each newly mounted level runs this on
  // ITS first render and the expansion cascades down to the changed files. A
  // filter that left everything collapsed would hide the very rows it exists to
  // show — you would see `crates/` and still have to click your way in.
  //
  // Turning it back OFF deliberately leaves them open: collapsing what the user
  // can see is a bigger surprise than leaving it, and the tree has no record of
  // what was open before the filter to restore anyway.
  useEffect(() => { if (changedOnly) setOpen(true); }, [changedOnly]);
  const [kids, setKids] = useState<FsEntry[] | null>(null);
  const [loading, setLoading] = useState(false);

  // Children are fetched by EFFECT, not by the click handler. The handler
  // version listed a directory ONCE — `kids === null` guarded the fetch, so a
  // file created after the first expand stayed invisible for the life of the
  // node, and collapsing/re-expanding did not help either. Keying off
  // reloadToken re-lists every OPEN directory instead; closed ones still cost
  // nothing. The previous listing stays on screen while a reload runs, so the
  // periodic bump never blanks the tree.
  //
  // Both halves of the failure path exist because this now runs REPEATEDLY,
  // where the click-handler version ran once per expand:
  //   · a failed reload keeps the last good listing. Wiping to [] would render
  //     "empty" and unmount every grandchild, so one blip (a directory swapped
  //     out under a `git checkout`) would collapse a deep expansion until the
  //     next tick.
  //   · the error is reported only when it CHANGES. A directory that stays
  //     unreadable — a root-owned build dir, which the tree now lists by default —
  //     would otherwise re-raise the banner and append to app.log on every
  //     bump, forever, for as long as it stayed expanded.
  const lastErr = useRef<string | null>(null);
  // A blocked link is inert on purpose. For one pointing out of the workspace
  // or at nothing, the guard behind every command here would refuse the call
  // anyway and the click could only produce an error banner. For one into
  // `.git` it would NOT: the listing hides `.git` by name, the guard never asks,
  // so this gate is the whole defense there rather than a courtesy. The title
  // says where the link points and why nothing happens.
  // A ghost FILE is inert for the same reason: it is not on disk, so every
  // command behind the row would fail — a click could only raise a banner
  // saying so. A ghost DIRECTORY still toggles; its children come from the
  // change set, not from a listing.
  const ghost = !!entry.ghost;
  const inert = !!entry.link_block || (ghost && !entry.is_dir);
  useEffect(() => {
    // `inert` too, not just `open`: a link that becomes unfollowable while
    // expanded (retargeted, or its project unregistered) would otherwise keep
    // firing a doomed list_dir on every reload bump for as long as it stays
    // mounted — the children are already hidden by then.
    // `ghost` likewise: a deleted directory has nothing to list, and
    // canonicalize() in the guard would reject the path anyway.
    if (!entry.is_dir || !open || inert || ghost) return;
    let alive = true;
    setLoading(true);
    invoke<FsEntry[]>("list_dir", { path: entry.path, showIgnored })
      .then((e) => { if (alive) { setKids(e); lastErr.current = null; } })
      .catch((e) => {
        if (!alive) return;
        // Nothing listed yet: an empty body plus the banner. A swallowed error
        // on the FIRST listing reads as "it just didn't respond".
        setKids((prev) => prev ?? []);
        const msg = String(e);
        if (lastErr.current !== msg) { lastErr.current = msg; onError(e); }
      })
      .finally(() => { if (alive) setLoading(false); });
    return () => { alive = false; };
  }, [entry.is_dir, entry.path, open, inert, ghost, showIgnored, reloadToken, onError]);

  const toggle = () => {
    if (inert) return;
    if (!entry.is_dir) { onOpen(entry.path); return; }
    setOpen((o) => !o);
  };

  const isSel = !entry.is_dir && openPath === entry.path;
  const kind = entry.is_dir ? null : fileInfo(entry.name).kind;
  // A file says WHAT changed; a directory says HOW MUCH, since its own name
  // never changed — something under it did.
  const status = entry.is_dir ? undefined : changes.files.get(entry.path);
  const count = entry.is_dir ? changes.dirs.get(entry.path) ?? 0 : 0;
  const title = [
    entry.link ? `${entry.name} → ${entry.link}` : entry.name,
    entry.link_block ? BLOCK_WHY[entry.link_block] : null,
    status ? CHANGE_WHY[status] : null,
    // A ghost DIRECTORY has no status of its own — git tracks files, so what is
    // deleted is everything that was under it. Without this the strikethrough is
    // the only thing saying so, and a tooltip that only counts changes reads as
    // if the directory were still there.
    ghost && entry.is_dir ? "deleted with its contents" : null,
    count ? `${count} changed file${count === 1 ? "" : "s"}` : null,
    entry.ignored ? "gitignored" : null,
  ].filter(Boolean).join(" — ");
  // A ghost dir's children are the change set's, not a listing's.
  const listed = ghost
    ? changes.ghosts.get(entry.path) ?? []
    : kids && withGhosts(entry.path, kids, changes);
  const shown = listed && (changedOnly ? listed.filter((k) => changedRow(k, changes)) : listed);
  return (
    <div className="tree-node">
      <button
        className={"tree-row" + (isSel ? " sel" : "") + (entry.is_dir ? " dir" : "") + (entry.ignored ? " ign" : "")
          + (entry.link ? " link" : "") + (inert ? " inert" : "")
          + (status ? ` chg chg-${status}` : "") + (count ? " chg chg-dir" : "") + (ghost ? " ghost" : "")}
        style={{ paddingLeft: `calc(var(--s2) + ${depth} * var(--s3))` }}
        onClick={toggle}
        onContextMenu={(e) => onContext(e, entry)}
        data-tree-path={entry.path}
        aria-disabled={inert || undefined}
        // usage: the row's title IS the file path and its git status — one
        // fixed key for the control, nothing about which file it points at.
        data-track="files.row"
        title={title}
      >
        <span className="tree-caret">{entry.is_dir && !inert && (open ? <Icons.ChevronDown size={11} /> : <Icons.ChevronRight size={11} />)}</span>
        <span className={`tree-glyph k-${entry.is_dir ? "dir" : kind}`} aria-hidden="true">{entry.is_dir ? "" : KIND_GLYPH[kind!]}</span>
        <span className="tree-name">{entry.name}</span>
        {/* The one thing the row cannot say with an icon slot it already spends
            on file kind: that this name is a pointer somewhere else. */}
        {entry.link && <span className="tree-link" aria-hidden="true">↗</span>}
        {/* Shown expanded too, not just collapsed: it is the same number either
            way, and a badge that vanished on expand would read as "resolved". */}
        {count > 0 && <span className="tree-count" aria-hidden="true">{count}</span>}
      </button>
      {entry.is_dir && !inert && open && (
        <div className="tree-kids">
          {/* only while there is nothing to show — a reload must not add a
              spinner line under every open directory every few seconds */}
          {loading && kids === null && <div className="tree-note">…</div>}
          {shown && shown.length === 0 && !loading && (
            // Under the filter "empty" would be a lie about the directory — it
            // has entries, none of them changed. The count badge said there was
            // something here, so the row has to explain why nothing is under it.
            <div className="tree-note">{changedOnly && listed?.length ? "no changes here" : "empty"}</div>
          )}
          {shown?.map((k) => (
            <TreeNode key={k.path} entry={k} depth={depth + 1} openPath={openPath}
              showIgnored={showIgnored} reloadToken={reloadToken} changes={changes} changedOnly={changedOnly}
              onOpen={onOpen} onContext={onContext} onError={onError} />
          ))}
        </div>
      )}
    </div>
  );
}

// One character per kind — an icon set would be six more SVGs for a 13px slot.
const KIND_GLYPH: Record<FileKind, string> = {
  markdown: "M", image: "◧", code: "‹›", text: "¶", binary: "▦",
};

// Files tab tree. `root` = the place's worktree path; remount per place via key.
function FileTree({ root, openPath, showIgnored, reloadToken, changedOnly, onOpen, onContext, onError }: {
  root: string; openPath: string | null; showIgnored: boolean; reloadToken: number; changedOnly: boolean;
  onOpen: (path: string) => void; onContext: (e: React.MouseEvent, entry: FsEntry) => void;
  onError: (e: unknown) => void;
}) {
  const [entries, setEntries] = useState<FsEntry[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [changes, setChanges] = useState<Changes>(NO_CHANGES);
  // No setEntries(null) here: this effect re-runs on every reload, and blanking
  // first would flash "loading…" over the whole tree each time. `root` cannot
  // change under a mounted FileTree anyway — the caller keys on it — so the
  // one genuine empty state is the initial mount.
  useEffect(() => {
    let alive = true;
    invoke<FsEntry[]>("list_dir", { path: root, showIgnored })
      .then((e) => { if (alive) { setEntries(e); setErr(null); } })
      .catch((e) => { if (alive) setErr(String(e)); });
    return () => { alive = false; };
  }, [root, showIgnored, reloadToken]);
  // ONE call for the whole tree, here rather than per node: `TreeNode` would
  // make it a git process per open directory per bump, and the tree re-lists
  // every open directory on every bump. Unmounted while the reader is expanded,
  // so a full-pane read costs nothing.
  //
  // The markers are ADDITIVE, so a failure keeps the last good set and leaves
  // the tree fully usable — and is reported only when the message CHANGES, for
  // the same reason TreeNode's listing errors are: this re-runs on every bump,
  // and a repo that stays unreadable would otherwise re-raise the banner and
  // append to app.log forever.
  const chErr = useRef<string | null>(null);
  useEffect(() => {
    let alive = true;
    invoke<ChangeSetDto>("changed_files", { root })
      .then((dto) => { if (alive) { setChanges(buildChanges(dto)); chErr.current = null; } })
      .catch((e) => {
        if (!alive) return;
        const msg = String(e);
        if (chErr.current !== msg) { chErr.current = msg; onError(e); }
      });
    return () => { alive = false; };
  }, [root, reloadToken, onError]);
  // The error REPLACES the tree only when there is no tree yet. Once a listing
  // has landed, a failed reload shows the reason above the last good one
  // instead of swapping it out: this effect re-runs on every bump now, so
  // returning the bare note would unmount every TreeNode — and with them every
  // expansion the user had opened — on one transient failure.
  if (err && !entries) return <div className="tree-note err-note">{err}</div>;
  if (!entries) return <div className="tree-note">loading…</div>;
  if (!entries.length) return <div className="tree-note">empty worktree</div>;
  // Keyed on the CANONICAL root the backend resolved, not the `root` prop: a
  // place path can run through a symlink, and the ghosts are filed under
  // resolved paths (as is every path `list_dir` returns).
  const all = withGhosts(changes.root || root, entries, changes);
  const rows = changedOnly ? all.filter((e) => changedRow(e, changes)) : all;
  // Has the change set ever ARRIVED? `changed_files` always returns a root on
  // success — the canonical repo top, or the cwd for a directory that is not a
  // repo — so an empty one means the fan-out has not answered yet, or failed.
  const changesLoaded = changes.root !== "";
  return (
    <div className="filetree">
      {err && <div className="tree-note err-note">{err}</div>}
      {/* Not "empty worktree": the tree above already ruled that out, so under
          the filter an empty result means the branch matches its base.
          GATED on the set having landed, and that gate is the whole point. This
          starts at NO_CHANGES and the real `changed_files` is a git fan-out over
          the worktree — so for its first few hundred milliseconds, on every
          mount (the tree remounts per place, and unmounts whenever the reader is
          expanded), an unfiltered "nothing changed on this branch" would be
          asserting the opposite of what is about to appear. Worse on failure:
          the markers are additive, so a repo that cannot be read keeps
          NO_CHANGES and the claim would stand forever. The mock cannot show
          either — it answers in a microtask. */}
      {changedOnly && !rows.length && (
        <div className="tree-note">{changesLoaded ? "nothing changed on this branch" : "…"}</div>
      )}
      {rows.map((e) => (
        <TreeNode key={e.path} entry={e} depth={0} openPath={openPath}
          showIgnored={showIgnored} reloadToken={reloadToken} changes={changes} changedOnly={changedOnly}
          onOpen={onOpen} onContext={onContext} onError={onError} />
      ))}
    </div>
  );
}

// ── image ────────────────────────────────────────────────────────────────

// Loads bytes as base64 and shows them as a data: URI. Used both for an image
// FILE and for a relative ![]() inside a rendered markdown doc, hence the
// standalone component and the `inline` variant.
function ImageView({ path, mime, inline = false, alt = "", onError }: {
  path: string; mime: string; inline?: boolean; alt?: string; onError?: (e: unknown) => void;
}) {
  const [blob, setBlob] = useState<FileBlob | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [actual, setActual] = useState(false);
  const [dim, setDim] = useState<{ w: number; h: number } | null>(null);

  useEffect(() => {
    let alive = true;
    setBlob(null); setErr(null); setDim(null);
    invoke<FileBlob>("read_file_base64", { path })
      .then((b) => { if (alive) setBlob(b); })
      .catch((e) => { if (!alive) return; setErr(String(e)); onError?.(e); });
    return () => { alive = false; };
  }, [path, onError]);

  // Inline (inside markdown prose) these must be SPANs: a <div> inside the <p>
  // React puts prose in is invalid nesting the browser reparents.
  const note = (cls: string, body: ReactNode) =>
    inline ? <span className={`tree-note inline-note ${cls}`}>{body}</span> : <div className={`tree-note ${cls}`}>{body}</div>;
  if (err) return note("err-note", err);
  if (!blob) return note("", "loading image…");
  // A partial image decodes to garbage or nothing — say so instead of showing it.
  if (blob.truncated) return note("", `image too large to preview (${humanSize(blob.size)}+)`);

  const img = (
    <img
      className={"img-el" + (actual ? " actual" : "")}
      src={`data:${mime};base64,${blob.b64}`}
      alt={alt || basename(path)}
      onLoad={(e) => setDim({ w: e.currentTarget.naturalWidth, h: e.currentTarget.naturalHeight })}
    />
  );
  if (inline) return <span className="md-img">{img}</span>;

  return (
    <div className="imgview">
      <div className="img-stage">{img}</div>
      <div className="img-meta">
        <span>{dim ? `${dim.w} × ${dim.h}` : "—"}</span>
        <span className="dot-sep">·</span>
        <span>{humanSize(blob.size)}</span>
        <span className="dock-spacer" />
        <button className="ctrl sm" onClick={() => setActual((a) => !a)}>{actual ? "Fit" : "1:1"}</button>
      </div>
    </div>
  );
}

// A renderer failure must cost the PANE, not the window. There is no error
// boundary above App, so an uncaught throw here (a pathological document, a
// highlighter edge case) would unmount the whole root and leave a blank
// window. Class component because that is the only way to catch in React.
class ViewErrorBoundary extends Component<{ resetKey: string; children: ReactNode }, { err: Error | null }> {
  state: { err: Error | null } = { err: null };
  static getDerivedStateFromError(err: Error) { return { err }; }
  componentDidUpdate(prev: { resetKey: string }) {
    if (prev.resetKey !== this.props.resetKey && this.state.err) this.setState({ err: null });
  }
  render() {
    if (this.state.err) {
      return (
        <div className="tree-note err-note">
          could not render this file — {String(this.state.err.message || this.state.err)}
        </div>
      );
    }
    return this.props.children;
  }
}

// ── viewer ───────────────────────────────────────────────────────────────

/** ⌘F over the open file. Mounted only while the bar is up, so a viewer with no
 *  find open walks nothing and holds no Ranges; unmounting is also what clears
 *  the highlight registry. */
function FindInFile({ bodyRef, token, onClose, contentKey, editable = false }: {
  bodyRef: React.RefObject<HTMLElement | null>;
  token: number; onClose?: () => void; contentKey: string;
  /** the view under this bar is a textarea while Find is closed — say so,
   *  because the bar is the reason the caret went away */
  editable?: boolean;
}) {
  const f = useFileFind(bodyRef, true, contentKey);
  return (
    <FindBar
      query={f.query} onQuery={f.setQuery}
      index={f.index} count={f.count} capped={f.capped}
      caseSensitive={f.caseSensitive} onCaseSensitive={f.setCaseSensitive}
      onNext={f.next} onPrev={f.prev} onClose={() => onClose?.()}
      focusToken={token}
      hint={editable
        ? "Searches the file open in this viewer — editing resumes when you close Find"
        : "Searches the file open in this viewer"}
    />
  );
}

// ── unsaved edits ────────────────────────────────────────────────────────

/** An edit in progress, keyed by ABSOLUTE path.
 *
 *  Module scope, not component state, for two reasons. The same file can be
 *  open in the dock's viewer AND in the reading overlay at once — App mounts
 *  both, and two component-held buffers would fork into two answers to "what
 *  have I typed", with the last save winning silently. And a draft has to
 *  outlive an unmount: flipping to Preview, switching place or closing the dock
 *  are all things you do WHILE writing, and none of them should throw the
 *  writing away.
 *
 *  It does not outlive a reload of the webview, and nothing here pretends
 *  otherwise — an unsaved buffer is unsaved. The header says so in every view
 *  that can show it.
 *
 *  `base` is the mtime the edit STARTED from, never the latest read. It is what
 *  `write_file` compares against, so a save is refused exactly when the file
 *  moved under the edit — and keeps being refused after a re-read, which is the
 *  point. */
type Draft = { text: string; base: number };

const drafts = new Map<string, Draft>();
const draftSubs = new Set<() => void>();

function putDraft(path: string, d: Draft | null) {
  if (d) drafts.set(path, d);
  else drafts.delete(path);
  for (const fn of draftSubs) fn();
}

function subDrafts(fn: () => void) {
  draftSubs.add(fn);
  return () => { draftSubs.delete(fn); };
}

/** The draft for `path`, or null. `getSnapshot` returns the STORED object, so
 *  its identity only changes when `putDraft` replaces it — a fresh object per
 *  call would re-render forever. */
function useDraft(path: string): Draft | null {
  return useSyncExternalStore(subDrafts, () => drafts.get(path) ?? null);
}

/** The editable Source view.
 *
 *  Module scope per CLAUDE.md, and here the rule has teeth: defined inside
 *  FileView this would be a new component type on every keystroke, React would
 *  unmount the textarea and mount a fresh one, and the caret would jump to the
 *  end of the document with the focus gone. */
/** The formatting bar's buttons, in order. The label IS the affordance — there
 *  are no list glyphs in `icons.tsx`, and a word beats a drawn one at 11px.
 *  `#` is the numbered list, NOT a heading: the headings are the three that say
 *  so, and the separator before `•` is what keeps the two groups apart. */
//  `track` is spelled out per row rather than built from `action`. The title is
//  an expression, so the usage key would otherwise be derived from it — and
//  `usage-check.mjs` refuses an INTERPOLATED data-track for the same reason it
//  refuses a title: a key has to be source, not something assembled at runtime.
const MD_TOOLS: { action: MdAction; label: string; title: string; track: string; cls?: string }[] = [
  { action: "bold", label: "B", title: "Bold (⌘B)", track: "files.fmt.bold", cls: "mdbar-bold" },
  { action: "italic", label: "I", title: "Italic (⌘I)", track: "files.fmt.italic", cls: "mdbar-ital" },
  { action: "h1", label: "H1", title: "Heading 1", track: "files.fmt.h1" },
  { action: "h2", label: "H2", title: "Heading 2", track: "files.fmt.h2" },
  { action: "h3", label: "H3", title: "Heading 3", track: "files.fmt.h3" },
  { action: "ul", label: "•", title: "Bulleted list", track: "files.fmt.ul" },
  { action: "ol", label: "#", title: "Numbered list", track: "files.fmt.ol" },
];

function SourceEditor({ text, wrap, onChange, onSave }: {
  text: string; wrap: boolean;
  onChange: (v: string) => void;
  onSave: () => void;
}) {
  const ref = useRef<HTMLTextAreaElement>(null);
  /** where the caret has to land once React has re-rendered with the new value.
   *  Setting it before that would be setting it on the OLD text. */
  const nextSel = useRef<[number, number] | null>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    const want = nextSel.current;
    if (!el || !want) return;
    nextSel.current = null;
    el.setSelectionRange(want[0], want[1]);
  });

  const fmt = (action: MdAction) => {
    const el = ref.current;
    if (!el) return;
    const out = applyMd(action, { text, start: el.selectionStart, end: el.selectionEnd });
    if (out.text === text) { el.setSelectionRange(out.start, out.end); return; }
    nextSel.current = [out.start, out.end];
    // Hand the edit to the browser's OWN editing pipeline rather than assigning
    // a new value through React. Every engine drops a textarea's undo history
    // when its value is replaced wholesale, so the React route would make
    // formatting the one edit ⌘Z cannot take back — and undo is not a feature
    // you can add to a control later, it is a property of how you wrote to it.
    // `execCommand` is deprecated and has no replacement that reaches the undo
    // stack; `onChange` below is the correctness fallback, minus the undo entry.
    const { from, to, insert } = spliceRange(text, out.text);
    el.focus();
    el.setSelectionRange(from, to);
    const done = insert === ""
      // Inserting an empty string is not specified to delete a selection —
      // toggling a marker OFF is a pure deletion, and it gets its own command.
      ? document.execCommand("delete")
      : document.execCommand("insertText", false, insert);
    if (!done) onChange(out.text);
  };

  // No autofocus. The view becomes editable on a place switch, a dock open and
  // every Preview→Source flip, and a textarea that grabs focus on each of those
  // takes it off the terminal mid-command. A caret on click is what an editor
  // pane is expected to do anyway.
  return (
    <>
      <div className="mdbar" role="toolbar" aria-label="Formatting">
        {MD_TOOLS.map((t, i) => (
          // Separators before the headings and before the lists — three groups,
          // because "make this bold" and "make this a list" are different
          // questions and a run of seven identical chips reads as one.
          <Fragment key={t.action}>
            {i === 2 || i === 5 ? <span className="mdbar-sep" aria-hidden="true" /> : null}
            <button
              className={"ctrl sm mdbar-btn" + (t.cls ? ` ${t.cls}` : "")}
              data-track={t.track}
              title={t.title}
              // The caret is the input to every one of these, so the press must
              // not move focus out of the textarea first. preventDefault on
              // POINTERDOWN is what keeps the selection alive — by the time a
              // click handler runs, a focused textarea has already lost it.
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => fmt(t.action)}
            >{t.label}</button>
          </Fragment>
        ))}
      </div>
      <textarea
        ref={ref}
        className={"srcedit" + (wrap ? " wrap" : "")}
        // `wrap="off"` is what makes a textarea scroll horizontally instead of
        // soft-wrapping; the CSS white-space alone does not reach the control.
        wrap={wrap ? "soft" : "off"}
        value={text}
        spellCheck={false}
        autoCapitalize="off"
        autoCorrect="off"
        onChange={(e) => onChange(e.currentTarget.value)}
        onKeyDown={(e) => {
          // ⌘S here rather than in App's window listener: this is the only
          // surface that can save, and a global chord would have to re-derive
          // which file is open and whether it is dirty. `e.key` is safe for a
          // plain ⌘ chord (the ⌥ composition trap in CLAUDE.md is about ⌥).
          if (!(e.metaKey || e.ctrlKey) || e.altKey) return;
          const k = e.key.toLowerCase();
          if (k === "s") {
            e.preventDefault();
            e.stopPropagation();
            onSave();
            return;
          }
          // ⌘B is the app's sidebar chord everywhere else in the window. Inside
          // a text editor it is bold, and `stopPropagation` is what keeps the
          // sidebar out of it — App's listener is on `window`, so this element
          // handler runs first. It costs the chord only while the caret is in
          // the editor, which is an explicit act.
          if (k === "b" || k === "i") {
            e.preventDefault();
            e.stopPropagation();
            fmt(k === "b" ? "bold" : "italic");
          }
        }}
      />
    </>
  );
}

export type FileViewProps = {
  path: string;
  /** bumps on places:changed and on the dock's Refresh → re-read from disk
   *  (nothing to lose: read-only). FilesPane re-lists the tree off the same
   *  token, so a new file appears without the user reselecting the place. */
  reloadToken: number;
  onOpenEditor: (path: string) => void;
  onOpen: (path: string) => void;
  onError: (e: unknown) => void;
  /** wrap long lines; owned by settings so it survives a place switch */
  wrap: boolean;
  onWrap: (v: boolean) => void;
  /** markdown starts rendered; the toggle flips to source */
  mdSource: boolean;
  onMdSource: (v: boolean) => void;
  /** show the two-column diff instead of the file. Per PLACE (see PlacePanels) */
  diff: boolean;
  onDiff: (v: boolean) => void;
  /** what the diff's "before" is — the branch's base, or HEAD */
  diffBase: "base" | "head";
  onDiffBase: (v: "base" | "head") => void;
  /** reading size of the RENDERED markdown, % of normal (see MD_ZOOM_STEPS) */
  mdZoom: number;
  onMdZoom: (v: number) => void;
  expanded: boolean;
  onExpand: (v: boolean) => void;
  /** ⌘F — App keeps exactly one find bar open across the whole window */
  findOpen?: boolean;
  /** bumps on every ⌘F, so a second press re-selects the field */
  findToken?: number;
  onFindClose?: () => void;
};

export function FileView(props: FileViewProps) {
  const { path, reloadToken, onOpenEditor, onOpen, onError, wrap, onWrap, mdSource, onMdSource, mdZoom, onMdZoom, expanded, onExpand,
    diff, onDiff, diffBase, onDiffBase, findOpen = false, findToken = 0, onFindClose } = props;
  const info = useMemo(() => fileInfo(path), [path]);
  const bodyRef = useRef<HTMLDivElement>(null);
  const [read, setRead] = useState<FileRead | null>(null);
  const [loading, setLoading] = useState(true);
  /** what the user has typed and not saved (module store — see `useDraft`) */
  const draft = useDraft(path);
  const [saving, setSaving] = useState(false);
  /** the last save's refusal, kept until the next edit or save. The conflict
   *  case is the one worth showing INLINE rather than only as a toast: the
   *  recovery is two buttons that are only meaningful while it stands. */
  const [saveErr, setSaveErr] = useState<string | null>(null);
  /** re-read on demand (Discard), without touching the dock-wide token */
  const [selfToken, setSelfToken] = useState(0);

  const isImage = info.kind === "image";
  // Only text has lines to line up. An image or a binary has no diff view, so
  // the toggle is not offered and a place that arrives with `diff` already on
  // falls back to the file — silently, because the alternative is an error
  // banner every time you click a .png in a branch you are reviewing.
  const diffable = !isImage && info.kind !== "binary" && !read?.binary;
  const showDiff = diff && diffable;

  useEffect(() => {
    if (isImage) { setRead(null); setLoading(false); return; }
    let alive = true;
    setLoading(true);
    invoke<FileRead>("read_file", { path })
      .then((r) => { if (alive) setRead(r); })
      .catch((e) => { if (alive) { setRead(null); onError(e); } })
      .finally(() => { if (alive) setLoading(false); });
    return () => { alive = false; };
    // reloadToken re-reads from disk unconditionally, unsaved buffer or not.
    // That is safe because `read` is only ever the BASELINE — what is on disk —
    // and the editor renders the draft on top of it. So a refresh while you are
    // typing updates what a Discard would return you to, and what the diff and
    // the byte count describe, without touching a character you wrote.
  }, [path, reloadToken, selfToken, isImage, onError]);

  // The diff is fetched ONLY while it is being shown. It is a git spawn per
  // file, and a viewer that pre-fetched one for every file you clicked would
  // pay for it on a tab nobody has opened.
  //
  // `dstate` is cleared on the way in rather than left stale: the previous
  // file's diff under the new file's name is worse than a blank frame, and the
  // same goes for switching base — "vs HEAD" showing the base's patch is a
  // wrong answer that looks right.
  const [dstate, setDstate] = useState<FileDiffDto | null>(null);
  const [dloading, setDloading] = useState(false);
  useEffect(() => {
    if (!showDiff) { setDstate(null); return; }
    let alive = true;
    setDstate(null);
    setDloading(true);
    invoke<FileDiffDto>("file_diff", { path, against: diffBase })
      .then((d) => { if (alive) setDstate(d); })
      .catch((e) => { if (alive) { setDstate(null); onError(e); } })
      .finally(() => { if (alive) setDloading(false); });
    return () => { alive = false; };
  }, [path, reloadToken, showDiff, diffBase, onError]);

  // A relative link in a markdown doc resolves against the DOC's directory, so
  // [DESIGN.md](DESIGN.md) inside docs/ opens docs/DESIGN.md, not the root one.
  const dir = path.slice(0, path.lastIndexOf("/"));
  const resolve = useCallback((href: string) => {
    if (href.startsWith("/")) return href;
    const parts = `${dir}/${href}`.split("/");
    const out: string[] = [];
    for (const p of parts) {
      if (p === "." || p === "") continue;
      if (p === "..") out.pop();
      else out.push(p);
    }
    return `/${out.join("/")}`;
  }, [dir]);

  const onLink = useCallback((href: string) => {
    if (/^[a-z][a-z0-9+.-]*:/i.test(href)) {
      // Absolute URL. Only http(s) leaves the app — a doc is untrusted input,
      // and file:/javascript:/custom schemes are not ours to hand to the OS.
      if (/^https?:/i.test(href)) openUrl(href).catch(onError);
      else onError(new Error(`refused to open ${href.split(":")[0]}: link`));
      return;
    }
    if (href.startsWith("#")) {
      // In-document anchor. Headings carry slug ids (markdown.tsx), so scroll
      // the rendered doc rather than doing nothing — every `](#…)` link in the
      // repo's own docs depends on this.
      const id = decodeURIComponent(href.slice(1));
      const el = bodyRef.current?.querySelector(`[id="${CSS.escape(id)}"]`);
      el?.scrollIntoView({ behavior: "smooth", block: "start" });
      return;
    }
    onOpen(resolve(href.split("#")[0]));
  }, [onOpen, onError, resolve]);

  const renderImage = useCallback((src: string, alt: string) => {
    if (/^https?:/i.test(src)) return <em className="md-img-note">[remote image: {alt || src}]</em>;
    const abs = resolve(src);
    const inf = fileInfo(abs);
    if (inf.kind !== "image") return <em className="md-img-note">[{alt || src}]</em>;
    return <ImageView key={abs} path={abs} mime={inf.mime} inline alt={alt} />;
  }, [resolve]);

  // ── editing ──────────────────────────────────────────────────────────
  // Markdown Source only — the module header says why this does not widen to
  // every text file. `truncated` is excluded by force rather than by taste: the
  // buffer is the first 1 MiB of the file, and saving it would silently DELETE
  // everything past the cap.
  const editable =
    info.kind === "markdown" && mdSource && !showDiff &&
    !!read && !read.binary && !read.truncated;
  // ⌘F paints through the CSS Custom Highlight API, which cannot reach inside a
  // textarea. Rather than let the bar report "0" over a file full of matches,
  // an open Find puts the source back in the read-only renderer — rendering the
  // DRAFT, so you search what you have written and not what is on disk.
  const editing = editable && !findOpen;
  const text = draft ? draft.text : read?.content ?? "";
  const dirty = !!draft;

  const onEdit = useCallback((v: string) => {
    setSaveErr(null);
    if (!read) return;
    // A buffer typed back to exactly what is on disk is not an edit. Clearing
    // the draft there keeps "unsaved" honest, and lets the next edit re-base on
    // the current read rather than carrying an mtime from before a refresh.
    // The base, once set, is NOT re-read: it is the point the edit forked from.
    putDraft(path, v === read.content ? null : { text: v, base: drafts.get(path)?.base ?? read.mtime });
  }, [path, read]);

  const save = useCallback(async (force: boolean) => {
    const d = drafts.get(path);
    if (!d || saving) return;
    setSaving(true);
    try {
      const mtime = await invoke<number>("write_file", {
        path, content: d.text, expectedMtime: force ? null : d.base,
      });
      // Only the draft we actually sent is cleared. Keystrokes that landed
      // while the write was in flight are a NEWER draft, and dropping them
      // would lose typing to a save the user did not know they were racing.
      if (drafts.get(path) === d) putDraft(path, null);
      setSaveErr(null);
      // Adopt what we just wrote as the baseline instead of re-reading it: the
      // bytes are ours and the ack carries the new mtime, so a re-read would
      // buy nothing but a round trip and a window in which the next save holds
      // the wrong `expected_mtime`.
      setRead((r) => (r ? { ...r, content: d.text, mtime, size: new TextEncoder().encode(d.text).length } : r));
    } catch (e) {
      setSaveErr(String(e));
      onError(e); // toast + app.log — a refusal is never only a local badge
    } finally {
      setSaving(false);
    }
  }, [path, saving, onError]);

  /** Throw the edit away and go back to what is on disk. The re-read is the
   *  point after a conflict: the draft was based on bytes that have moved. */
  const discard = useCallback(() => {
    putDraft(path, null);
    setSaveErr(null);
    setSelfToken((t) => t + 1);
  }, [path]);

  const size = isImage ? null : read?.size;
  const pct = clampMdZoom(mdZoom);
  const zoom = String(pct / 100);
  const body = renderBody();

  function renderBody(): ReactNode {
    if (isImage && info.mime !== "image/svg+xml") return <ImageView path={path} mime={info.mime} onError={onError} />;
    if (loading) return <div className="tree-note">loading…</div>;
    // Before the markdown/SVG branches on purpose: a diff of a README is a diff
    // of its SOURCE. Rendering two columns of formatted prose would hide exactly
    // the whitespace and link changes you opened the diff to see.
    if (showDiff) {
      if (dloading || !dstate) return <div className="tree-note">{dloading ? "diffing…" : "no diff"}</div>;
      return <DiffView diff={dstate} content={read?.content ?? ""} lang={info.lang} wrap={wrap} />;
    }
    // SVG: rendered unless the source toggle is on (it reuses the md toggle —
    // one "show me the markup" affordance, not two).
    if (isImage) {
      return mdSource
        ? <div className="scroll"><CodeBlock src={read?.content ?? ""} lang="xml" wrap={wrap} /></div>
        : <ImageView path={path} mime={info.mime} onError={onError} />;
    }
    if (!read) return <div className="tree-note">could not read this file</div>;
    if (read.binary || info.kind === "binary")
      return (
        <div className="binview">
          <div className="bin-glyph" aria-hidden="true">▦</div>
          <div className="bin-label">{info.label} file{read.size ? ` · ${humanSize(read.size)}` : ""}</div>
          <div className="bin-actions">
            <button className="ctrl sm" onClick={() => onOpenEditor(path)}>Open in editor</button>
            <button className="ctrl sm" onClick={() => revealItemInDir(path).catch(onError)}>Reveal</button>
          </div>
        </div>
      );
    if (info.kind === "markdown" && !mdSource)
      // --md-zoom rides on the scroll box, not on `.md` itself: App.css reads it
      // through `var(--md-zoom, 1)`, so an unset value is simply "normal" and
      // the document keeps rendering if this ever mounts without a zoom.
      //
      // `text`, not `read.content`: Preview renders the DRAFT, so flipping to
      // it mid-edit shows what you have written rather than a ghost of the file
      // as it was before you started.
      return (
        <div className="scroll" style={{ "--md-zoom": zoom } as CSSProperties}>
          <Markdown src={text} onLink={onLink} renderImage={renderImage} />
        </div>
      );
    // The editable branch. It is NOT wrapped in `.scroll`: the textarea is its
    // own scroll box, and a scroller inside a scroller gives the file two
    // scrollbars and a caret that can be scrolled out of sight.
    if (editing) return <SourceEditor text={text} wrap={wrap} onChange={onEdit} onSave={() => save(false)} />;
    return <div className="scroll"><CodeBlock src={text} lang={info.lang} wrap={wrap} /></div>;
  }

  const hasPreview = info.kind === "markdown" || info.mime === "image/svg+xml";
  // One segmented control for "what am I looking at", not two: Preview and
  // Source and Diff are three answers to the same question, and a separate Diff
  // button beside a Preview/Source pair would let both read as "on".
  const showViewSeg = hasPreview || diffable;
  // Wrap stays available in the diff. It is the one place the grid earns its
  // keep: a wrapped row still lines up, because the row takes the height of its
  // tallest cell — where two independently scrolling panes would drift apart at
  // the first long line.
  const showWrap = !isImage && info.kind !== "binary" && !(info.kind === "markdown" && !mdSource && !showDiff);
  // Zoom belongs to the RENDERED document only: the Source view is code, sized
  // by the terminal font like every other source file in this viewer, and the
  // diff is the same code twice.
  const showZoom = info.kind === "markdown" && !mdSource && !showDiff;

  return (
    <div className="viewer">
      <div className="viewer-h">
        <span className="viewer-path" title={path}>
          <span className="vp-dir">{dir ? `${basename(dir)}/` : ""}</span>{basename(path)}
        </span>
        <span className="viewer-tag kind">{info.label}</span>
        {read?.truncated && <span className="viewer-tag">truncated</span>}
        {/* Not gated on `editing`: a draft you left behind by flipping to
            Preview or opening Find is still unsaved, and the one place that can
            say so is the header of the file it belongs to. */}
        {dirty && <span className="viewer-tag" title="Edited here and not written to disk yet">unsaved</span>}
        {saveErr && (
          <span className="viewer-tag err" title={saveErr}>save refused</span>
        )}
        {size ? <span className="viewer-size">{humanSize(size)}</span> : null}
        <span className="dock-spacer" />
        {showDiff && dstate && (dstate.untracked || dstate.base_label) && (
          // The base is NAMED, not implied. Which ref "base" resolved to differs
          // per repo (origin/main, master, or nothing at all), and a diff that
          // does not say what it is against is a claim you cannot check.
          // An untracked file gets a phrase of its own rather than a ref: it has
          // no before, and "vs nothing" reads like a bug in the label.
          <span
            className="viewer-tag"
            title={dstate.untracked
              ? "git has never seen this file — every line is new"
              : `Comparing the working file against ${dstate.base_label}`}
          >
            {dstate.untracked ? "new file" : `vs ${dstate.base_label}`}
          </span>
        )}
        {showViewSeg && (
          <div className="seg" role="group" aria-label="View">
            {hasPreview && (
              <button className={"seg-b" + (!mdSource && !showDiff ? " on" : "")}
                onClick={() => { onMdSource(false); onDiff(false); }}>Preview</button>
            )}
            {/* `onMdSource` only where there IS a preview to turn off. On a .rs
                file it would flip the GLOBAL markdown preference as a side
                effect of leaving a diff, and the next README you opened would
                come up as source for no reason you could trace. */}
            <button className={"seg-b" + ((mdSource || !hasPreview) && !showDiff ? " on" : "")}
              onClick={() => { if (hasPreview) onMdSource(true); onDiff(false); }}>Source</button>
            {diffable && (
              <button className={"seg-b" + (showDiff ? " on" : "")}
                title="Show what this branch changed in this file"
                onClick={() => onDiff(true)}>Diff</button>
            )}
          </div>
        )}
        {showDiff && (
          <div className="seg" role="group" aria-label="Diff base">
            <button className={"seg-b" + (diffBase === "base" ? " on" : "")}
              title="Compare against the branch's base — what this branch changed, committed and not"
              onClick={() => onDiffBase("base")}>Base</button>
            <button className={"seg-b" + (diffBase === "head" ? " on" : "")}
              title="Compare against HEAD — only what is not committed yet"
              onClick={() => onDiffBase("head")}>HEAD</button>
          </div>
        )}
        {showZoom && (
          <div className="seg zoomseg" role="group" aria-label="Text size">
            <button
              className="seg-b"
              disabled={pct <= MD_ZOOM_MIN}
              title="Smaller text (⌘⌥−)"
              aria-label="Smaller text"
              onClick={() => onMdZoom(stepMdZoom(pct, -1))}
            >A−</button>
            {/* The readout is the RESET: a percentage you cannot click back to
                100 leaves the only way home as counting steps. */}
            <button
              className="seg-b zoom-val"
              disabled={pct === 100}
              title="Reset text size (⌘⌥0)"
              aria-label={`Text size ${pct}%. Reset to 100%.`}
              onClick={() => onMdZoom(100)}
            >{pct}%</button>
            <button
              className="seg-b"
              disabled={pct >= MD_ZOOM_MAX}
              title="Larger text (⌘⌥+)"
              aria-label="Larger text"
              onClick={() => onMdZoom(stepMdZoom(pct, 1))}
            >A+</button>
          </div>
        )}
        {showWrap && (
          <button className={"ctrl sm" + (wrap ? " on" : "")} title="Wrap long lines" onClick={() => onWrap(!wrap)}>Wrap</button>
        )}
        {editable && (
          // Always present once the view can be edited, disabled while clean.
          // A save affordance that only appears once you are dirty is a save
          // affordance you have to discover by accident — and this viewer spent
          // several versions being read-only, so nothing about the textarea
          // announces itself.
          <button
            className="ctrl sm"
            data-track="files.save"
            disabled={!dirty || saving}
            title={saveErr ? "Save again" : "Save (⌘S)"}
            onClick={() => save(false)}
          >{saving ? "Saving…" : "Save"}</button>
        )}
        {editable && dirty && (
          <button className="ctrl sm" data-track="files.discard" title="Throw the edit away and re-read the file" onClick={discard}>Discard</button>
        )}
        {editable && dirty && saveErr && (
          // Only after a refusal, and never the default. `expected_mtime: null`
          // skips the backend's compare-and-swap, which is the one way for the
          // dock to overwrite something Claude wrote in the pane next door — so
          // it is a deliberate second click with its own word on it.
          <button
            className="ctrl sm danger"
            data-track="files.overwrite"
            disabled={saving}
            title={`${saveErr}\n\nSave anyway, discarding what is on disk.`}
            onClick={() => save(true)}
          >Overwrite</button>
        )}
        <button className="ctrl sm" data-track="files.expand" title={expanded ? "Collapse (⌘⇧E)" : "Expand over the main pane (⌘⇧E)"} onClick={() => onExpand(!expanded)}>
          {expanded ? "Collapse" : "Expand"}
        </button>
        <button className="ctrl sm" onClick={() => onOpenEditor(path)}>Editor</button>
      </div>
      {/* Between the header and the body, never INSIDE the body: the body is
          the search root, so a bar within it would offer its own count ("3/12")
          and button glyphs up as matches. */}
      {findOpen && (
        <FindInFile bodyRef={bodyRef} token={findToken} onClose={onFindClose}
          editable={editable}
          contentKey={`${path}|${reloadToken}|${wrap}|${mdSource}|${loading}|${showDiff}|${diffBase}|${dloading}|${text.length}`} />
      )}
      <div className="viewer-body" ref={bodyRef}>
        <ViewErrorBoundary resetKey={path}>{body}</ViewErrorBoundary>
      </div>
    </div>
  );
}

// ── pane (tree + viewer + divider) ───────────────────────────────────────

/** Dock width past which the tab lays out side-by-side under `auto`. */
export const SPLIT_AT = 620;
/** Hard floor for side-by-side, even when the user PINS it: below this the
 *  content column cannot hold the viewer's own header controls. */
export const SPLIT_FLOOR = 420;

export type FilesPaneProps = Omit<FileViewProps, "path" | "expanded" | "onExpand"> & {
  root: string;
  openPath: string | null;
  /** list gitignored entries too, dimmed (the reader overlay has no tree) */
  showIgnored: boolean;
  /** hide anything the branch did not touch — the tree becomes its diff list */
  changedOnly: boolean;
  /** live dock width — decides the `auto` orientation */
  dockW: number;
  layout: Settings["files_layout"];
  /** tree share when side-by-side (%) */
  splitPct: number;
  /** tree share when stacked (%) — a vertical ratio is NOT a horizontal one */
  stackPct: number;
  onSplitPct: (v: number, orient: "split" | "stack") => void;
  expanded: boolean;
  onExpand: (v: boolean) => void;
};

export function orientationFor(layout: Settings["files_layout"], dockW: number): "split" | "stack" {
  if (layout === "stack") return "stack";
  if (dockW < SPLIT_FLOOR) return "stack"; // floor beats the pin
  if (layout === "split") return "split";
  return dockW >= SPLIT_AT ? "split" : "stack";
}

export function FilesPane(props: FilesPaneProps) {
  const { root, openPath, dockW, layout, splitPct, stackPct, onSplitPct, onOpen, onError, expanded,
    showIgnored, changedOnly, reloadToken } = props;
  const orient = expanded ? "split" : orientationFor(layout, dockW);
  const hostRef = useRef<HTMLDivElement>(null);
  // The drag reads BOTH of these live: the window can resize mid-drag (flipping
  // `auto` from split to stack), and a captured axis would then move the
  // divider along the wrong one.
  const orientRef = useRef(orient); orientRef.current = orient;
  const dragCleanup = useRef<(() => void) | null>(null);
  // Unmounting mid-drag (⌘J closes the dock) must tear the listeners down.
  // Otherwise `move` keeps firing against a DETACHED host whose rect is all
  // zeroes → (clientX - 0) / 0 = Infinity → the split slams to the clamp.
  useEffect(() => () => { dragCleanup.current?.(); }, []);

  // Drag the divider. Clamped to 15–85% so neither side can be dragged shut,
  // and measured against the LIVE host box.
  const onDividerDown = (e: React.MouseEvent) => {
    e.preventDefault();
    const host = hostRef.current;
    if (!host) return;
    const move = (ev: MouseEvent) => {
      const b = host.getBoundingClientRect();
      if (b.width <= 0 || b.height <= 0) return; // detached / collapsed: ignore
      const pct = orientRef.current === "split"
        ? ((ev.clientX - b.left) / b.width) * 100
        : ((ev.clientY - b.top) / b.height) * 100;
      if (!Number.isFinite(pct)) return;
      onSplitPct(Math.max(15, Math.min(85, Math.round(pct))), orientRef.current);
    };
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      window.removeEventListener("blur", up);
      dragCleanup.current = null;
    };
    dragCleanup.current = up;
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
    window.addEventListener("blur", up); // release outside the OS window
  };

  // ── right-click menu ──────────────────────────────────────────────────────
  // One menu for the whole pane, keyed off the row that opened it. CtxMenu is
  // the same shell the nav's place/project menus use (App.tsx), so this is the
  // frame, the clamping and all three dismissal paths for free.
  const [ctx, setCtx] = useState<{ x: number; y: number; entry: FsEntry } | null>(null);
  const onContext = useCallback((e: React.MouseEvent, entry: FsEntry) => {
    e.preventDefault();
    setCtx({ x: e.clientX, y: e.clientY, entry });
  }, []);
  const closeCtx = useCallback(() => setCtx(null), []);
  // Every verb closes the menu FIRST and reports through onError — a rejected
  // opener invoke (a missing permission is one, and it rejects SILENTLY) must
  // reach the banner, not the void.
  const copyText = (text: string) => {
    setCtx(null);
    if (!navigator.clipboard) { onError("clipboard unavailable"); return; }
    navigator.clipboard.writeText(text).catch(onError);
  };
  const reveal = (path: string) => { setCtx(null); revealItemInDir(path).catch(onError); };
  const openIn = (path: string) => { setCtx(null); openInDefaultApp(path).catch(onError); };

  const pct = orient === "split" ? splitPct : stackPct;
  const treeStyle = orient === "split"
    ? { flex: `0 0 ${pct}%`, minWidth: 0 }
    : { flex: `0 0 ${pct}%`, minHeight: 0 };

  return (
    <div className={`dock-files o-${orient}`} ref={hostRef}>
      {!expanded && (
        <>
          <div className="dock-tree" style={treeStyle}>
            <FileTree key={root} root={root} openPath={openPath} showIgnored={showIgnored}
              changedOnly={changedOnly} reloadToken={reloadToken}
              onOpen={onOpen} onContext={onContext} onError={onError} />
          </div>
          <div
            className="files-divider"
            role="separator"
            tabIndex={0}
            aria-label={orient === "split" ? "Resize tree width" : "Resize tree height"}
            aria-orientation={orient === "split" ? "vertical" : "horizontal"}
            aria-valuenow={pct}
            aria-valuemin={15}
            aria-valuemax={85}
            onMouseDown={onDividerDown}
            onDoubleClick={() => onSplitPct(orient === "split" ? 32 : 40, orient)}
            // Arrow keys move it in 2% steps, Home/End jump to the clamps —
            // otherwise the ratio is mouse-only and unreachable by keyboard.
            onKeyDown={(e) => {
              const back = orient === "split" ? "ArrowLeft" : "ArrowUp";
              const fwd = orient === "split" ? "ArrowRight" : "ArrowDown";
              let next: number | null = null;
              if (e.key === back) next = pct - 2;
              else if (e.key === fwd) next = pct + 2;
              else if (e.key === "Home") next = 15;
              else if (e.key === "End") next = 85;
              if (next === null) return;
              e.preventDefault();
              onSplitPct(Math.max(15, Math.min(85, next)), orient);
            }}
          />
        </>
      )}
      <div className="dock-content">
        {openPath
          ? <FileView {...props} path={openPath} />
          : <div className="tree-note viewer-hint">select a file to view</div>}
      </div>

      {/* Same `.pop-item` / `.ctx-sep` markup as the nav's menus — one look for
          every right-click in the window. A row the tree has already made
          INERT (a symlink it will not follow, a ghost left by a deletion) is
          not on disk or not ours to open, so it gets the two facts that are
          still true — its path — and nothing that could only raise a banner. */}
      {ctx && (() => {
        const { entry } = ctx;
        const inert = !!entry.link_block || (!!entry.ghost && !entry.is_dir);
        return (
          <CtxMenu x={ctx.x} y={ctx.y} onClose={closeCtx}>
            <div className="pop-hint">{entry.name}</div>
            {!inert && (entry.is_dir ? (
              <>
                <button className="pop-item" onClick={() => openIn(entry.path)}>Open in Finder</button>
                <button className="pop-item" onClick={() => reveal(entry.path)}>Reveal in Finder</button>
              </>
            ) : (
              <>
                <button className="pop-item" onClick={() => reveal(entry.path)}>Reveal in Finder</button>
                <button className="pop-item" onClick={() => openIn(entry.path)}>Open</button>
              </>
            ))}
            {!inert && <div className="ctx-sep" />}
            <button className="pop-item" onClick={() => copyText(entry.path)}>Copy path</button>
            <button className="pop-item" onClick={() => copyText(relPath(root, entry.path))}>Copy relative path</button>
          </CtxMenu>
        );
      })()}
    </div>
  );
}
