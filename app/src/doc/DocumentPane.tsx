// The document pane: one open document (useDoc.ts) drawn the same way in both
// windows. Its header, the banners for a save that was refused, its versions,
// the editor and Read. What only one window has (the Ideas window's acts and
// strips, the main window's Back and Float) comes in through the slots.
import { useMemo, type ReactNode } from "react";
import { Markdown } from "../markdown";
import "./doc.css";
import { LinkPicker, VersionsPanel } from "./DocParts";
import { EditorHost } from "./EditorHost";
import type { SaveState } from "./ideasave";
import { ideaHref, readableLinks } from "./ideasrelate";
import { bodyOf, commentOnly } from "./ideatext";
import type { DocHandle } from "./useDoc";

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

/** Read: the dock's markdown renderer on the body (the frontmatter is the
 *  file's, not the prose). HTML comments draw as nothing — later phases'
 *  anchors are comments (§4.4); other raw HTML stays literal, inert text. A
 *  mermaid fence is code here: mermaid lives in the docs viewer's bundle
 *  alone. Links do nothing in phase 1. */
function ReadView({ text, follow }: { text: string; follow(id: string): void }) {
  // `idea:` links become `#idea:` ones, which the shared renderer links.
  const body = useMemo(() => readableLinks(bodyOf(text)), [text]);
  return (
    <div className="ix-read" data-testid="ideas-read">
      <Markdown src={body} renderHtml={(raw) => (commentOnly(raw) ? "" : null)} onLink={(href) => { const id = ideaHref(href); if (id) follow(id); }} />
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

export function DocumentPane(props: {
  doc: DocHandle;
  /** Before the title. */
  lead?: ReactNode;
  /** Between Edit | Read and Versions. */
  tools?: ReactNode;
  /** After Rename. */
  after?: ReactNode;
  /** Under the header, over the document. */
  strips?: ReactNode;
  textDrag?(on: boolean): void;
  follow(id: string): void;
}) {
  const { doc } = props;
  const { open, save, mode, source, arm, session } = doc;
  if (!open) return null;
  const armed = arm.armed as string | null;
  return (
    <>
      <header className="ix-head">
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
          <h1 className="ix-title" title={open.file}>{open.title || "Untitled"}</h1>
        )}
        <div className="ix-tools">
          <span className="ix-state" data-testid="ideas-save" data-state={save.kind}>{saveWords(save)}</span>
          <span className="ix-seg" role="group" aria-label="View">
            <button className={mode === "edit" ? "on" : ""} data-testid="ideas-mode-edit" aria-pressed={mode === "edit"} onClick={() => doc.switchMode("edit")}>Edit</button>
            <button className={mode === "read" ? "on" : ""} data-testid="ideas-mode-read" aria-pressed={mode === "read"} onClick={() => doc.switchMode("read")}>Read</button>
          </span>
          {props.tools}
          <button className={`ix-btn${doc.versions !== false ? " on" : ""}`} data-testid="ideas-versions-toggle" aria-pressed={doc.versions !== false}
            onClick={() => doc.setVersions((v) => (v === false ? null : false))}>Versions</button>
          {source.rename && (
            <button className="ix-btn" data-testid="ideas-rename" disabled={doc.busy} onMouseDown={(e) => e.preventDefault()} onClick={() => doc.setRenaming(open.title)}>Rename</button>
          )}
          {props.after}
        </div>
      </header>
      {props.strips}
      {doc.versions !== false && (
        <VersionsPanel versions={doc.versions} armed={armed?.startsWith("restore:") ? armed.slice(8) : null} restore={doc.restore} close={() => doc.setVersions(false)} />
      )}
      {save.kind === "conflict" && (
        <div className="ix-banner" data-testid="ideas-conflict" data-why={save.why} role="alert">
          <span>{save.why === "refused"
            ? `This ${source.noun} changed on disk, so your edit was not saved. Your text is still here.`
            : `This ${source.noun} was changed in another editor while you had unsaved text. Your text is still here.`}</span>
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
          <span>This {source.noun} was removed from the folder. Your text is still here until you close it.</span>
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
      <div className="ix-body">
        <EditorHost open={open} handle={doc.handle} held={doc.held} hooks={doc.hooks} replaced={doc.replaced} hidden={mode !== "edit"} textDrag={props.textDrag} />
        {mode === "read" && <ReadView text={doc.readBody} follow={props.follow} />}
        {doc.picker && mode === "edit" && <LinkPicker x={doc.picker.x} y={doc.picker.y} rows={doc.pickRows} index={doc.pickIdx} pick={doc.insertLink} />}
      </div>
    </>
  );
}
