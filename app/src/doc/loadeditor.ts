// The one door to the editor (editor.ts), for both windows: a dynamic import
// with DESTRUCTURED names, so CodeMirror is a chunk of its own, fetched the
// first time a document is opened and never part of the main window's entry
// chunk. A namespace import (`const ed = await import(…)`) would keep every
// export alive and is refused by editor-boundary-check.mjs, as is any static
// import of editor.ts.
import type { DocEditor, EditorHooks } from "./editortypes";

export type MountEditor = (parent: HTMLElement, hooks: EditorHooks) => DocEditor;

let mounted: MountEditor | null = null;
let loading: Promise<MountEditor> | null = null;

/** The editor, if it has been fetched: a pane mounts it in its first frame. */
export const editorNow = (): MountEditor | null => mounted;

/** Fetch the editor, once. A failed fetch is not remembered here (WebKit
 *  remembers it all the same, for the page's life). */
export function loadEditor(): Promise<MountEditor> {
  loading ??= (async () => {
    const { mountEditor } = await import("./editor");
    mounted = mountEditor;
    return mountEditor;
  })().catch((e) => { loading = null; throw e; });
  return loading;
}
