import { useEffect, useRef, useState } from "react";
import { editorNow, loadEditor } from "./loadeditor";
import type { DocEditor, EditorHandle, EditorHooks } from "./editortypes";
import { errText, type CursorNext, type Held, type Open } from "./useDoc";

/** One editor view from the pane's first Edit to its end (DocumentPane.tsx
 *  mounts this then, so a document that is only read fetches no editor); a new
 *  document gets a new STATE (its own history, its own line separator). The
 *  view is fetched the first time
 *  (`loadeditor.ts`); until it is here the document's text is `held`. */
export function EditorHost(props: {
  open: Open;
  handle: React.RefObject<EditorHandle | null>;
  held: React.RefObject<Held | null>;
  hooks: EditorHooks;
  /** Edit was asked for before the editor was here. */
  cursorNext: React.RefObject<CursorNext>;
  /** The text was replaced under the editor. */
  replaced(): void;
  hidden: boolean;
  /** Text dragged out of the editor (an HTML5 drag the editor starts). */
  textDrag?(on: boolean): void;
}) {
  const { open, handle, held, hooks, replaced } = props;
  const host = useRef<HTMLDivElement>(null);
  const [editor, setEditor] = useState<DocEditor | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let mine: DocEditor | null = null;
    let gone = false;
    const mount = (make: (parent: HTMLElement, hooks: EditorHooks) => DocEditor) => {
      if (gone || !host.current) return;
      mine = make(host.current, hooks);
      setEditor(mine);
    };
    setError(null);
    const now = editorNow();
    if (now) mount(now);
    else loadEditor().then(mount, (e) => { if (!gone) setError(errText(e)); });
    return () => {
      gone = true;
      // What the editor holds outlives it, for as long as its session does.
      if (handle.current && held.current?.key === handle.current.key) held.current.text = handle.current.text();
      handle.current = null;
      mine?.destroy();
      setEditor(null);
    };
  }, [handle, held, hooks]);
  // The editor's own keys stay in it. CodeMirror takes ⌘[ (indent less), ⌃B
  // and others and prevents their default, and the window's chords listen
  // further up without asking: ⌘[ also went back, ⌃B also toggled the nav.
  useEffect(() => {
    const el = host.current;
    const own = (e: KeyboardEvent) => { if (e.defaultPrevented) e.stopPropagation(); };
    el?.addEventListener("keydown", own);
    return () => el?.removeEventListener("keydown", own);
  }, []);
  useEffect(() => {
    if (!editor || held.current?.key !== open.key) return;
    editor.open(held.current.text);
    handle.current = {
      key: open.key,
      text: () => editor.text(),
      replace: (content) => { editor.replace(content); replaced(); },
      focus: () => editor.focus(),
      selection: () => editor.selection(),
      insert: (from, to, text) => editor.insert(from, to, text),
      put: (text, at) => editor.put(text, at),
      gotoLine: (n) => editor.gotoLine(n),
      selectTitle: () => editor.selectTitle(),
    };
    const next = props.cursorNext.current;
    props.cursorNext.current = null;
    if (next?.line !== undefined) editor.gotoLine(next.line);
    else if (next?.title) editor.selectTitle();
    else if (open.focus || next) editor.focus();
    // The text is read once per opening; a reload goes through `replace`,
    // never through a re-render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open.key, editor]);
  return (
    <div ref={host} className="ix-editor" data-testid="ideas-editor" hidden={props.hidden}
      onDragStart={() => props.textDrag?.(true)} onDragEnd={() => props.textDrag?.(false)}>
      {!editor && !error && <p className="ix-editor-wait">Opening the editor…</p>}
      {error && (
        <p className="ix-editor-wait" role="alert" data-testid="ideas-editor-error">
          {/* No Try again: WebKit keeps a module that failed to load as failed
              for the page's life (measured on a build, documents-browser.mjs). */}
          The editor could not be loaded: {error}. Read still works. Quit Worktrees and open it again to edit.
        </p>
      )}
    </div>
  );
}
