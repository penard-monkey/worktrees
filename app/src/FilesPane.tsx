// The dock's Files tab: a lazy tree beside (or above) a read-only viewer that
// renders per file KIND — markdown, source, image, or a named placeholder for
// everything else.
//
// Deliberately NOT an editor. The viewer used to be a textarea with a save
// path; editing now goes through "Open in editor", which means no editor
// library, no save-conflict UI, and no way for the dock to clobber what Claude
// is writing in the pane next door. `write_file` still exists in the backend.
//
// Layout: `split` (tree left / content right) past a dock-width threshold,
// `stack` (tree above) below it, with a manual override. The divider drags and
// its ratio persists per orientation.
//
// Everything here is at MODULE scope per CLAUDE.md — components defined inside
// App() get a new identity every render and would drop tree expansion state.
import { Component, useCallback, useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import * as Icons from "./icons";
import { CodeBlock } from "./CodeView";
import { FindBar, useFileFind } from "./Find";
import { Markdown } from "./markdown";
import { basename, fileInfo, humanSize, type FileKind } from "./filekind";
import { MD_ZOOM_MAX, MD_ZOOM_MIN, clampMdZoom, stepMdZoom, type Settings } from "./settings";

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
function TreeNode({ entry, depth, openPath, showIgnored, reloadToken, changes, onOpen, onError }: {
  entry: FsEntry; depth: number; openPath: string | null; showIgnored: boolean; reloadToken: number;
  changes: Changes; onOpen: (path: string) => void; onError: (e: unknown) => void;
}) {
  const [open, setOpen] = useState(false);
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
  const shown = ghost
    ? changes.ghosts.get(entry.path) ?? []
    : kids && withGhosts(entry.path, kids, changes);
  return (
    <div className="tree-node">
      <button
        className={"tree-row" + (isSel ? " sel" : "") + (entry.is_dir ? " dir" : "") + (entry.ignored ? " ign" : "")
          + (entry.link ? " link" : "") + (inert ? " inert" : "")
          + (status ? ` chg chg-${status}` : "") + (count ? " chg chg-dir" : "") + (ghost ? " ghost" : "")}
        style={{ paddingLeft: `calc(var(--s2) + ${depth} * var(--s3))` }}
        onClick={toggle}
        aria-disabled={inert || undefined}
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
          {shown && shown.length === 0 && !loading && <div className="tree-note">empty</div>}
          {shown?.map((k) => (
            <TreeNode key={k.path} entry={k} depth={depth + 1} openPath={openPath}
              showIgnored={showIgnored} reloadToken={reloadToken} changes={changes} onOpen={onOpen} onError={onError} />
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
function FileTree({ root, openPath, showIgnored, reloadToken, onOpen, onError }: {
  root: string; openPath: string | null; showIgnored: boolean; reloadToken: number;
  onOpen: (path: string) => void; onError: (e: unknown) => void;
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
  const rows = withGhosts(changes.root || root, entries, changes);
  return (
    <div className="filetree">
      {err && <div className="tree-note err-note">{err}</div>}
      {rows.map((e) => (
        <TreeNode key={e.path} entry={e} depth={0} openPath={openPath}
          showIgnored={showIgnored} reloadToken={reloadToken} changes={changes} onOpen={onOpen} onError={onError} />
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
function FindInFile({ bodyRef, token, onClose, contentKey }: {
  bodyRef: React.RefObject<HTMLElement | null>;
  token: number; onClose?: () => void; contentKey: string;
}) {
  const f = useFileFind(bodyRef, true, contentKey);
  return (
    <FindBar
      query={f.query} onQuery={f.setQuery}
      index={f.index} count={f.count} capped={f.capped}
      caseSensitive={f.caseSensitive} onCaseSensitive={f.setCaseSensitive}
      onNext={f.next} onPrev={f.prev} onClose={() => onClose?.()}
      focusToken={token} hint="Searches the file open in this viewer"
    />
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
    findOpen = false, findToken = 0, onFindClose } = props;
  const info = useMemo(() => fileInfo(path), [path]);
  const bodyRef = useRef<HTMLDivElement>(null);
  const [read, setRead] = useState<FileRead | null>(null);
  const [loading, setLoading] = useState(true);

  const isImage = info.kind === "image";

  useEffect(() => {
    if (isImage) { setRead(null); setLoading(false); return; }
    let alive = true;
    setLoading(true);
    invoke<FileRead>("read_file", { path })
      .then((r) => { if (alive) setRead(r); })
      .catch((e) => { if (alive) { setRead(null); onError(e); } })
      .finally(() => { if (alive) setLoading(false); });
    return () => { alive = false; };
    // reloadToken re-reads from disk — safe unconditionally now the viewer owns
    // no unsaved buffer.
  }, [path, reloadToken, isImage, onError]);

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

  const size = isImage ? null : read?.size;
  const pct = clampMdZoom(mdZoom);
  const zoom = String(pct / 100);
  const body = renderBody();

  function renderBody(): ReactNode {
    if (isImage && info.mime !== "image/svg+xml") return <ImageView path={path} mime={info.mime} onError={onError} />;
    if (loading) return <div className="tree-note">loading…</div>;
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
      return (
        <div className="scroll" style={{ "--md-zoom": zoom } as CSSProperties}>
          <Markdown src={read.content} onLink={onLink} renderImage={renderImage} />
        </div>
      );
    return <div className="scroll"><CodeBlock src={read.content} lang={info.lang} wrap={wrap} /></div>;
  }

  const showSourceToggle = info.kind === "markdown" || info.mime === "image/svg+xml";
  const showWrap = !isImage && info.kind !== "binary" && !(info.kind === "markdown" && !mdSource);
  // Zoom belongs to the RENDERED document only: the Source view is code, sized
  // by the terminal font like every other source file in this viewer.
  const showZoom = info.kind === "markdown" && !mdSource;

  return (
    <div className="viewer">
      <div className="viewer-h">
        <span className="viewer-path" title={path}>
          <span className="vp-dir">{dir ? `${basename(dir)}/` : ""}</span>{basename(path)}
        </span>
        <span className="viewer-tag kind">{info.label}</span>
        {read?.truncated && <span className="viewer-tag">truncated</span>}
        {size ? <span className="viewer-size">{humanSize(size)}</span> : null}
        <span className="dock-spacer" />
        {showSourceToggle && (
          <div className="seg">
            <button className={"seg-b" + (!mdSource ? " on" : "")} onClick={() => onMdSource(false)}>Preview</button>
            <button className={"seg-b" + (mdSource ? " on" : "")} onClick={() => onMdSource(true)}>Source</button>
          </div>
        )}
        {showZoom && (
          <div className="seg zoomseg" role="group" aria-label="Text size">
            <button
              className="seg-b"
              disabled={pct <= MD_ZOOM_MIN}
              title="Smaller text (⌘−)"
              aria-label="Smaller text"
              onClick={() => onMdZoom(stepMdZoom(pct, -1))}
            >A−</button>
            {/* The readout is the RESET: a percentage you cannot click back to
                100 leaves the only way home as counting steps. */}
            <button
              className="seg-b zoom-val"
              disabled={pct === 100}
              title="Reset text size (⌘0)"
              aria-label={`Text size ${pct}%. Reset to 100%.`}
              onClick={() => onMdZoom(100)}
            >{pct}%</button>
            <button
              className="seg-b"
              disabled={pct >= MD_ZOOM_MAX}
              title="Larger text (⌘+)"
              aria-label="Larger text"
              onClick={() => onMdZoom(stepMdZoom(pct, 1))}
            >A+</button>
          </div>
        )}
        {showWrap && (
          <button className={"ctrl sm" + (wrap ? " on" : "")} title="Wrap long lines" onClick={() => onWrap(!wrap)}>Wrap</button>
        )}
        <button className="ctrl sm" title={expanded ? "Collapse (⌘⇧E)" : "Expand over the main pane (⌘⇧E)"} onClick={() => onExpand(!expanded)}>
          {expanded ? "Collapse" : "Expand"}
        </button>
        <button className="ctrl sm" onClick={() => onOpenEditor(path)}>Editor</button>
      </div>
      {/* Between the header and the body, never INSIDE the body: the body is
          the search root, so a bar within it would offer its own count ("3/12")
          and button glyphs up as matches. */}
      {findOpen && (
        <FindInFile bodyRef={bodyRef} token={findToken} onClose={onFindClose}
          contentKey={`${path}|${reloadToken}|${wrap}|${mdSource}|${loading}`} />
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
    showIgnored, reloadToken } = props;
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
              reloadToken={reloadToken} onOpen={onOpen} onError={onError} />
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
    </div>
  );
}
