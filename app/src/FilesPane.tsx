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
import { Component, useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import * as Icons from "./icons";
import { CodeBlock } from "./CodeView";
import { Markdown } from "./markdown";
import { basename, fileInfo, humanSize, type FileKind } from "./filekind";
import type { Settings } from "./settings";

type FsEntry = { name: string; path: string; is_dir: boolean; ignored?: boolean };
type FileRead = { content: string; truncated: boolean; binary: boolean; mtime: number; size: number };
type FileBlob = { b64: string; size: number; truncated: boolean; mtime: number };

// ── tree ─────────────────────────────────────────────────────────────────

// One lazy directory node. Files bubble a click up via onOpen; dirs toggle.
function TreeNode({ entry, depth, openPath, showIgnored, reloadToken, onOpen, onError }: {
  entry: FsEntry; depth: number; openPath: string | null; showIgnored: boolean; reloadToken: number;
  onOpen: (path: string) => void; onError: (e: unknown) => void;
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
  useEffect(() => {
    if (!entry.is_dir || !open) return;
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
  }, [entry.is_dir, entry.path, open, showIgnored, reloadToken, onError]);

  const toggle = () => {
    if (!entry.is_dir) { onOpen(entry.path); return; }
    setOpen((o) => !o);
  };

  const isSel = !entry.is_dir && openPath === entry.path;
  const kind = entry.is_dir ? null : fileInfo(entry.name).kind;
  return (
    <div className="tree-node">
      <button
        className={"tree-row" + (isSel ? " sel" : "") + (entry.is_dir ? " dir" : "") + (entry.ignored ? " ign" : "")}
        style={{ paddingLeft: `calc(var(--s2) + ${depth} * var(--s3))` }}
        onClick={toggle}
        title={entry.ignored ? `${entry.name} — gitignored` : entry.name}
      >
        <span className="tree-caret">{entry.is_dir && (open ? <Icons.ChevronDown size={11} /> : <Icons.ChevronRight size={11} />)}</span>
        <span className={`tree-glyph k-${entry.is_dir ? "dir" : kind}`} aria-hidden="true">{entry.is_dir ? "" : KIND_GLYPH[kind!]}</span>
        <span className="tree-name">{entry.name}</span>
      </button>
      {entry.is_dir && open && (
        <div className="tree-kids">
          {/* only while there is nothing to show — a reload must not add a
              spinner line under every open directory every few seconds */}
          {loading && kids === null && <div className="tree-note">…</div>}
          {kids && kids.length === 0 && !loading && <div className="tree-note">empty</div>}
          {kids?.map((k) => (
            <TreeNode key={k.path} entry={k} depth={depth + 1} openPath={openPath}
              showIgnored={showIgnored} reloadToken={reloadToken} onOpen={onOpen} onError={onError} />
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
  // The error REPLACES the tree only when there is no tree yet. Once a listing
  // has landed, a failed reload shows the reason above the last good one
  // instead of swapping it out: this effect re-runs on every bump now, so
  // returning the bare note would unmount every TreeNode — and with them every
  // expansion the user had opened — on one transient failure.
  if (err && !entries) return <div className="tree-note err-note">{err}</div>;
  if (!entries) return <div className="tree-note">loading…</div>;
  if (!entries.length) return <div className="tree-note">empty worktree</div>;
  return (
    <div className="filetree">
      {err && <div className="tree-note err-note">{err}</div>}
      {entries.map((e) => (
        <TreeNode key={e.path} entry={e} depth={0} openPath={openPath}
          showIgnored={showIgnored} reloadToken={reloadToken} onOpen={onOpen} onError={onError} />
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
  expanded: boolean;
  onExpand: (v: boolean) => void;
};

export function FileView(props: FileViewProps) {
  const { path, reloadToken, onOpenEditor, onOpen, onError, wrap, onWrap, mdSource, onMdSource, expanded, onExpand } = props;
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
      return <div className="scroll"><Markdown src={read.content} onLink={onLink} renderImage={renderImage} /></div>;
    return <div className="scroll"><CodeBlock src={read.content} lang={info.lang} wrap={wrap} /></div>;
  }

  const showSourceToggle = info.kind === "markdown" || info.mime === "image/svg+xml";
  const showWrap = !isImage && info.kind !== "binary" && !(info.kind === "markdown" && !mdSource);

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
        {showWrap && (
          <button className={"ctrl sm" + (wrap ? " on" : "")} title="Wrap long lines" onClick={() => onWrap(!wrap)}>Wrap</button>
        )}
        <button className="ctrl sm" title={expanded ? "Collapse (⌘⇧E)" : "Expand over the main pane (⌘⇧E)"} onClick={() => onExpand(!expanded)}>
          {expanded ? "Collapse" : "Expand"}
        </button>
        <button className="ctrl sm" onClick={() => onOpenEditor(path)}>Editor</button>
      </div>
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
