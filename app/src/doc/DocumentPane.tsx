// The document pane: one open document (useDoc.ts) drawn the same way in both
// windows. Its header, the banners for a save that was refused, its history,
// the editor and Read. What only one window has (the Ideas window's acts and
// strips, the main window's Float) comes in through the slots.
import { useEffect, useMemo, useState, type ReactNode } from "react";
import * as Icons from "../icons";
import { Markdown } from "../markdown";
import "./doc.css";
import { DocMenu, LinkPicker, VersionsPanel, type DocAct } from "./DocParts";
import { EditorHost } from "./EditorHost";
import type { SaveState } from "./ideasave";
import { commentOnly } from "./ideatext";
import type { DocImage, DocSource, DocTarget } from "./source";
import { errText, nounOf, type DocHandle, type Open } from "./useDoc";
import { useTier } from "./useTier";

function saveWords(s: SaveState): string {
  switch (s.kind) {
    case "clean":
      return s.savedAt ? `Saved ${new Date(s.savedAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}` : "Saved";
    case "dirty":
      return "Edited";
    case "saving":
      return "Saving…";
    case "conflict":
      return "Not saved";
    case "removed":
      return "Removed";
    case "error":
      return "Not saved";
  }
}

/** An image in Read: as its source answers it, or the words of why not. */
function DocPicture({ source, from, src, alt }: { source: DocSource; from: string; src: string; alt: string }) {
  const [img, setImg] = useState<DocImage | null>(null);
  useEffect(() => {
    let gone = false;
    source.image?.(from, src).then((i) => { if (!gone) setImg(i); }, (e) => { if (!gone) setImg({ note: errText(e) }); });
    return () => { gone = true; };
  }, [source, from, src]);
  if (!img) return <span className="ix-pic-note">{alt || src}</span>;
  return "src" in img
    ? <img className="ix-pic" data-testid="doc-image" src={img.src} alt={alt} title={src} />
    : <span className="ix-pic-note" data-testid="doc-image-note">{alt ? `${alt}: ` : ""}{img.note}</span>;
}

/** Read: the dock's markdown renderer on the source's body. HTML comments draw
 *  as nothing — later phases' anchors are comments (§4.4); other raw HTML stays
 *  literal, inert text. A mermaid fence is code here: mermaid lives in the docs
 *  viewer's bundle alone. What a link or an image is, the source says. */
function ReadView({ open, text, follow, linkNote }: { open: Open; text: string; follow(to: DocTarget): void; linkNote?: (from: string, href: string) => string | null }) {
  const { source, id } = open;
  const body = useMemo(() => source.readBody(text), [source, text]);
  const draw = useMemo(() => source.image && ((src: string, alt: string) => <DocPicture source={source} from={id} src={src} alt={alt} />), [source, id]);
  const note = useMemo(() => linkNote && ((href: string) => linkNote(id, href)), [linkNote, id]);
  return (
    <div className="ix-read" data-testid="ideas-read">
      <Markdown src={body} renderHtml={(raw) => (commentOnly(raw) ? "" : null)}
        renderImage={draw} linkNote={note} onLink={(href) => { const to = source.target(id, href); if (to) follow(to); }} />
    </div>
  );
}

/** The words a window's engine or an act has to say, over the document. */
export function DocNotice(props: { text: string; act?: ReactNode; dismiss(): void }) {
  return (
    <div className="ix-notice" data-testid="ideas-notice" role="status">
      <span>{props.text}</span>
      {props.act}
      <button className="ix-btn ix-btn-quiet" onClick={props.dismiss} aria-label="Dismiss">Dismiss</button>
    </div>
  );
}

/** The header by the pane's own width (workspace-view-design.md §4.1), each one
 *  row: from `FULL` everything; under it History is in the menu; under
 *  `NARROW` the state is on hover too, unless it says the text is not saved. */
export const NARROW = 420;
export const FULL = 640;
const BREAKS = [NARROW, FULL] as const;
const bad = (s: SaveState) => s.kind === "conflict" || s.kind === "removed" || s.kind === "error";

export function DocumentPane(props: {
  doc: DocHandle;
  /** Before the title. */
  lead?: ReactNode;
  /** The title, when it is not the document's own (a memory file's name). */
  title?: string;
  /** Why a link in Read goes nowhere, when it does. */
  linkNote?: (from: string, href: string) => string | null;
  /** Between Edit | Read and History, at every width: one control. `small`:
   *  there is room for its icon alone. */
  tool?: (small: boolean) => ReactNode;
  /** After History: the document's move to another window. */
  float?: { title: string; run(): void };
  /** In the menu, after History (when it is folded there) and Rename. */
  acts?: readonly DocAct[];
  /** The acts are the header's own buttons, on as many rows as they take, and
   *  fold into the menu only under `NARROW`: the Ideas window, which stays as it
   *  is (the owner, 2026-11-24). `acts[].first`: before History. */
  buttons?: boolean;
  /** What History shows, when the source's own is not it: a source whose
   *  history is its folder's. It takes the document's place. Absent: the
   *  document's versions, over it. */
  history?: (close: () => void) => ReactNode;
  /** Under the header, over the document. */
  strips?: ReactNode;
  textDrag?(on: boolean): void;
  follow(to: DocTarget): void;
}) {
  const { doc } = props;
  const { open, save, mode, arm, session } = doc;
  const [head, setHead] = useState<HTMLElement | null>(null);
  const tier = useTier(head, BREAKS);
  const [menu, setMenu] = useState<{ x: number; y: number; byKey: boolean } | null>(null);
  // The editor is mounted on the first Edit and stays for the pane's life: a
  // document that is only read fetches none.
  const [edited, setEdited] = useState(false);
  useEffect(() => { if (open && mode === "edit") setEdited(true); }, [open, mode]);
  const openId = open?.id;
  useEffect(() => setMenu(null), [openId, tier]);
  if (!open) return null;
  const { source } = open;
  const noun = nounOf(open);
  const armed = arm.armed as string | null;
  const hasHistory = !!props.history || !!source.versions;
  const buttons = !!props.buttons && tier > 0;
  const renameAct: DocAct[] = source.rename ? [{ testId: "ideas-rename", keeps: true, label: buttons ? "Rename" : "Rename…", disabled: doc.busy, run: () => doc.setRenaming(open.title) }] : [];
  const inlineActs = (first: boolean) => (buttons ? [...(first ? [] : renameAct), ...(props.acts ?? []).filter((a) => !!a.first === first)] : []).map((a) => (
    <button key={a.testId} className={`ix-btn${a.armed ? " ix-btn-armed" : ""}`} data-testid={a.testId} title={a.title} disabled={a.disabled}
      onMouseDown={a.keeps ? (e) => e.preventDefault() : undefined} onClick={a.run}>{a.label}</button>
  ));
  const toggleHistory = () => doc.setHistory((h) => !h);
  const acts: DocAct[] = buttons ? [] : [
    ...(hasHistory && tier < 2 ? [{ testId: "doc-menu-history", label: doc.history ? "Hide History" : "History", run: toggleHistory }] : []),
    ...renameAct,
    ...(props.acts ?? []),
  ];
  const openMenu = (e: React.MouseEvent<HTMLButtonElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    setMenu((m) => (m ? null : { x: r.right, y: r.bottom + 4, byKey: e.detail === 0 }));
  };
  const state = saveWords(save);
  return (
    <>
      <header className={"ix-head" + (buttons ? " ix-head-buttons" : "")} ref={setHead} data-tier={tier}>
        {props.lead}
        {doc.renaming !== null ? (
          <input
            className="ix-rename"
            data-testid="ideas-rename-input"
            autoFocus
            value={doc.renaming}
            onChange={(e) => doc.setRenaming(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void doc.rename(doc.renaming ?? "");
              if (e.key === "Escape") doc.setRenaming(null);
            }}
            onBlur={() => doc.setRenaming(null)}
          />
        ) : (
          <h1 className="ix-title" title={`${open.file} · ${state}`}>{props.title ?? (open.title || "Untitled")}</h1>
        )}
        <div className="ix-tools">
          <span className="ix-state" data-testid="ideas-save" data-state={save.kind} hidden={tier === 0 && !bad(save)}>{state}</span>
          <span className="ix-seg" role="group" aria-label="View" title={tier === 0 ? state : undefined}>
            <button className={mode === "edit" ? "on" : ""} data-testid="ideas-mode-edit" aria-pressed={mode === "edit"} onClick={() => doc.switchMode("edit")}>Edit</button>
            <button className={mode === "read" ? "on" : ""} data-testid="ideas-mode-read" aria-pressed={mode === "read"} onClick={() => doc.switchMode("read")}>Read</button>
          </span>
          {props.tool?.(tier < 2 && !buttons)}
          {inlineActs(true)}
          {hasHistory && (tier === 2 || buttons) && (
            <button className={`ix-btn${doc.history ? " on" : ""}`} data-testid="ideas-versions-toggle" aria-pressed={doc.history} onClick={toggleHistory}>History</button>
          )}
          {inlineActs(false)}
          {props.float && (
            <button className="ix-btn ix-btn-icon" data-testid="main-doc-float" title={props.float.title} aria-label={props.float.title} onClick={props.float.run}>
              <Icons.ExternalLink size={13} />
            </button>
          )}
          {acts.length > 0 && (
            <button className={`ix-btn ix-btn-icon${menu ? " on" : ""}`} data-testid="doc-menu-open" title="More" aria-label="More" aria-haspopup="menu" aria-expanded={!!menu}
              onMouseDown={(e) => e.preventDefault()} onClick={openMenu}><Icons.Ellipsis size={13} /></button>
          )}
        </div>
      </header>
      {menu && <DocMenu {...menu} acts={acts} close={() => setMenu(null)} />}
      {props.strips}
      {doc.history && (props.history
        ? props.history(() => doc.setHistory(false))
        : <VersionsPanel versions={doc.versions} armed={armed?.startsWith("restore:") ? armed.slice(8) : null} restore={doc.restore} close={() => doc.setHistory(false)} />)}
      {save.kind === "conflict" && (
        <div className="ix-banner" data-testid="ideas-conflict" data-why={save.why} role="alert">
          <span>{save.why === "refused"
            ? `This ${noun} changed on disk, so your edit was not saved. Your text is still here.`
            : `This ${noun} was changed in another editor while you had unsaved text. Your text is still here.`}</span>
          <button className="ix-btn" data-testid="ideas-keep-mine" onClick={() => void session.current?.keepMine()}
            title="Saves your text. The other version is copied to the folder's .history first.">Keep mine (theirs goes to .history)</button>
          <button className={`ix-btn${armed === "reload" ? " ix-btn-armed" : ""}`} data-testid="ideas-reload"
            onClick={() => arm.confirm("reload", () => void session.current?.reload())}>
            {armed === "reload" ? "Drop mine and reload?" : "Reload theirs"}
          </button>
        </div>
      )}
      {save.kind === "removed" && (
        <div className="ix-banner" data-testid="ideas-removed" role="alert">
          <span>This {noun} was removed from the folder. Your text is still here until you close it.</span>
          {source.keepAsNew && <button className="ix-btn" data-testid="ideas-save-as-new" onClick={() => void doc.saveAsNew()}>Save as a new {source.noun}</button>}
          <button className="ix-btn" data-testid="ideas-close" onClick={doc.closeOpen}>Close</button>
        </div>
      )}
      {save.kind === "error" && (
        <div className="ix-banner" data-testid="ideas-error" role="alert">
          <span>Not saved: {save.message}</span>
          <button className="ix-btn" onClick={() => void session.current?.flush()}>Try again</button>
        </div>
      )}
      <div className="ix-body" hidden={doc.history && !!props.history}>
        {(edited || mode === "edit") && (
          <EditorHost open={open} handle={doc.handle} held={doc.held} hooks={doc.hooks} cursorNext={doc.cursorNext} replaced={doc.replaced} hidden={mode !== "edit"} textDrag={props.textDrag} />
        )}
        {mode === "read" && <ReadView open={open} text={doc.readBody} follow={props.follow} linkNote={props.linkNote} />}
        {doc.picker && mode === "edit" && <LinkPicker x={doc.picker.x} y={doc.picker.y} rows={doc.pickRows} index={doc.pickIdx} pick={doc.insertLink} />}
      </div>
    </>
  );
}
