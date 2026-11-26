// What the document editor hands the pane and is handed by it: types alone, so
// the pane and everything above it can name them without reaching the editor
// (editor.ts is imported by loadeditor.ts and by nothing else).
export type EditorHooks = {
  /** A person's edit changed the text. */
  edited(): void;
  /** The editor lost focus: a save point. */
  blurred(): void;
  /** Phase 3: `[[` typed — the picker's query, where `[[` starts and the
   *  cursor (document positions) and where to draw it; `null` closes it. */
  picker?(p: { from: number; to: number; query: string; x: number; y: number } | null): void;
  /** A key while the picker is open; `true` = it took the key. */
  pickerKey?(key: "ArrowUp" | "ArrowDown" | "Enter" | "Tab" | "Escape"): boolean;
  /** ⌘-click (Ctrl elsewhere) on a link, by its href. `true`: it was followed. */
  follow?(href: string): boolean;
  /** Whether text is selected, when that changes. */
  selected?(has: boolean): void;
  /** The selection, or the text under one, moved. */
  selectionMoved?(): void;
};

/** The editor, as everything but editor.ts sees it. */
export type DocEditor = {
  /** Show a document: a fresh state, the cursor at its end. */
  open(text: string): void;
  /** The text exactly as a save sends it. */
  text(): string;
  /** Replace the whole text under the cursor (a reload). */
  replace(content: string): void;
  focus(): void;
  /** The selection as offsets into `text()` (UTF-16, what a JS string
   *  counts), and its text; `null` when nothing is selected. */
  selection(): { start: number; end: number; text: string } | null;
  /** Replace document positions `from..to` with `text`, as a person's input
   *  (so it saves), and put the cursor after it. */
  insert(from: number, to: number, text: string): void;
  /** Put `text` at the cursor, or where a point of the window is, as a person's
   *  input. */
  put(text: string, at?: { x: number; y: number }): void;
  /** Put the cursor at the start of line `n` (1-based, the file's) and show it. */
  gotoLine(n: number): void;
  /** Select the first heading's words (not its `# `), so what is typed names
   *  the document: a new idea's "Untitled". At the end when it has none. */
  selectTitle(): void;
  destroy(): void;
};

/** A document open in the editor: `key` says WHICH opening, so a reader never
 *  takes the text of the document before it (EditorHost.tsx). */
export type EditorHandle = Omit<DocEditor, "open" | "destroy"> & { key: string };
