// The document editor: CodeMirror 6 on the markdown SOURCE, with Bear's stage 1
// (docs/proposals/idea-board-phases.md §4.2): headings sized, emphasis
// styled, links styled, and every marker (`#`, `**`, `` ` ``, `>`, a list's
// bullet, a link's brackets) DIMMED — never hidden. All of it is highlighting:
// marks over the text, no replacement, no widget, so what is on screen is
// every character of the file (stage 2's hidden markers and stage 3's widgets
// will be view-only changes on top).
//
// **This is the ONE module that imports CodeMirror** (AGENTS.md, the third
// admitted library; editor-boundary-check.mjs holds it). Both windows use it,
// and both reach it only through `loadeditor.ts`'s dynamic import, so it is a
// chunk fetched on first use and never in the main window's entry chunk.
// Nothing here knows React or a row: `mountEditor` hands back a `DocEditor`
// (editortypes.ts), and the document pane (DocumentPane.tsx) does the rest.
//
// Its first half is the editor's STATE: what makes the text it holds
// byte-exact with the file (ideatext.ts says why each piece exists).
// ideas-text-check.mjs builds the very same state under node and proves the
// bytes, with and without CRLF, before any DOM is involved.
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { EditorSelection, EditorState, Prec, Transaction, type Extension, type Text, type TransactionSpec } from "@codemirror/state";
import { drawSelection, EditorView, highlightSpecialChars, keymap } from "@codemirror/view";
import { tags as t } from "@lezer/highlight";
import type { MarkdownConfig } from "@lezer/markdown";
import type { DocEditor, EditorHooks } from "./editortypes";
import { linkAt, pickerQuery } from "./ideasrelate";
import { frontOf, guardedRanges, lineBreakOf, normalizeInserted } from "./ideatext";

/** How far into the document the frontmatter is looked for. A frontmatter
 *  longer than this is not guarded (and is not a frontmatter anyone wrote). */
const FRONT_SCAN = 64 * 1024;

/** Make inserted text take the document's line ending. Only a person's
 *  input (typing, paste, drop) is rewritten; a reload or a rename replaces
 *  the state with the file's own text and never passes through here. */
function normalize(sep: string) {
  return EditorState.transactionFilter.of((tr: Transaction): Transaction | TransactionSpec => {
    if (!tr.docChanged || !tr.annotation(Transaction.userEvent)) return tr;
    let odd = false;
    tr.changes.iterChanges((_fa, _ta, _fb, _tb, ins) => {
      if (!odd && /[\r\n]/.test(ins.sliceString(0, ins.length, "\u0000"))) odd = true;
    });
    if (!odd) return tr;
    const specs: { from: number; to: number; insert: string }[] = [];
    tr.changes.iterChanges((from, to, _fb, _tb, ins) => {
      specs.push({ from, to, insert: normalizeInserted(ins.sliceString(0, ins.length, sep), sep) });
    });
    const changes = tr.startState.changes(specs);
    return {
      changes,
      selection: EditorSelection.create(tr.startState.selection.ranges.map((r) => r.map(changes, 1)), tr.startState.selection.mainIndex),
      scrollIntoView: tr.scrollIntoView,
      annotations: Transaction.userEvent.of(tr.annotation(Transaction.userEvent)!),
    };
  });
}

/** No change a person makes may touch the frontmatter's `---`, `id:` and
 *  closing lines. Not CodeMirror's `changeFilter`: it lets an insertion
 *  through at the START of a guarded range (a keystroke at the head of the
 *  `id:` line made `zid:`, measured) and drops the text of a replacement
 *  that covered one (select-all and a keystroke typed nothing). Here, per
 *  change: the deletion is clipped to what lies outside the guarded lines; a
 *  plain insertion inside one (its first character up to, not including, the
 *  start of the next line) is dropped; a replacement's text goes just after the
 *  last guarded line the change covers (when that is still inside it), so select-all and typing
 *  leaves the three lines and the typed text after them. */
function guard(sep: "\n" | "\r\n") {
  return EditorState.transactionFilter.of((tr: Transaction): Transaction | TransactionSpec => {
    if (!tr.docChanged || !tr.annotation(Transaction.userEvent)) return tr;
    const doc = tr.startState.doc;
    const flat = guardedRanges(frontOf(doc.sliceString(0, Math.min(doc.length, FRONT_SCAN)), sep), doc.length);
    if (!flat.length) return tr;
    const spans: [number, number][] = [];
    for (let i = 0; i < flat.length; i += 2) spans.push([flat[i], flat[i + 1]]);
    const span = (p: number) => spans.find(([a, b]) => p >= a && p < b);
    let touched = false;
    const specs: { from: number; to?: number; insert?: Text }[] = [];
    tr.changes.iterChanges((from, to, _fb, _tb, ins) => {
      // The deletion, minus the guarded spans.
      let cuts: [number, number][] = [[from, to]];
      if (from < to) {
        for (const [a, b] of spans) {
          const next: [number, number][] = [];
          for (const [x, y] of cuts) {
            if (b <= x || a >= y) { next.push([x, y]); continue; }
            touched = true;
            if (x < a) next.push([x, a]);
            if (b < y) next.push([b, y]);
          }
          cuts = next;
        }
      }
      let at = from;
      if (ins.length) {
        if (from === to) {
          if (span(from)) { touched = true; at = -1; }
        } else {
          // After the last guarded line the change covers, so its text
          // never lands between two of them, inside the frontmatter.
          for (const [x, y] of spans) if (x < to && y > from) at = Math.max(at, y);
          if (at !== from) touched = true;
          if (at > to || span(at)) at = -1;
        }
      }
      for (const [x, y] of cuts) if (x < y) specs.push({ from: x, to: y });
      if (ins.length && at >= 0) specs.push({ from: at, insert: ins });
    });
    if (!touched) return tr;
    const changes = tr.startState.changes(specs);
    return {
      changes,
      selection: EditorSelection.create(tr.startState.selection.ranges.map((r) => r.map(changes, 1)), tr.startState.selection.mainIndex),
      scrollIntoView: tr.scrollIntoView,
      annotations: Transaction.userEvent.of(tr.annotation(Transaction.userEvent)!),
    };
  });
}

/** The state extensions for a document whose file text is `text`. */
export function textExtensions(text: string): Extension[] {
  const sep = lineBreakOf(text);
  return [EditorState.lineSeparator.of(sep), normalize(sep), guard(sep)];
}

/** The text a save sends: joined with the document's own separator. NEVER
 *  `state.doc.toString()`, which joins with `\n` whatever the file had. */
export function savedText(state: EditorState): string {
  return state.sliceDoc();
}

/** The frontmatter as its own block, so its keys are not read as a setext
 *  heading (`created: …` over the closing `---` is one, to markdown). Shown
 *  dimmed and in the code face: it is the file's, not the prose. A block
 *  parser sees one line at a time and cannot look for the close first, so an
 *  unclosed `---` at the top dims to the end; core reads no frontmatter then
 *  (and nothing is guarded), and only the colour differs. */
const Frontmatter: MarkdownConfig = {
  defineNodes: [{ name: "Frontmatter", block: true, style: t.meta }],
  parseBlock: [{
    name: "Frontmatter",
    before: "HorizontalRule",
    parse(cx, line) {
      if (cx.lineStart !== 0 || line.text.replace(/\r$/, "") !== "---") return false;
      let end = cx.lineStart + line.text.length;
      while (cx.nextLine()) {
        end = cx.lineStart + line.text.length;
        const s = line.text.replace(/\r$/, "");
        if (s === "---" || s === "...") { cx.nextLine(); break; }
      }
      cx.addElement(cx.elt("Frontmatter", 0, end));
      return true;
    },
  }],
};

/** Stage 1, in the app's tokens: hue only where the app already uses it for
 *  the same thing (a link is `--accent`, as `.md-link` is); markers and the
 *  frontmatter in `--txt-dim`, a step under the prose's `--txt`. Not
 *  `--txt-mute`: on the editor's surface it measured 1.98:1 in tokyo-day,
 *  and a marker is dimmed, never hidden (ideas-browser.mjs holds ≥ 2:1 in
 *  all six themes). */
const stage1 = HighlightStyle.define([
  { tag: t.heading1, fontSize: "1.6em", fontWeight: "var(--fw-bold)", color: "var(--txt-hi)" },
  { tag: t.heading2, fontSize: "1.35em", fontWeight: "var(--fw-bold)", color: "var(--txt-hi)" },
  { tag: t.heading3, fontSize: "1.18em", fontWeight: "var(--fw-bold)", color: "var(--txt-hi)" },
  { tag: [t.heading4, t.heading5, t.heading6], fontWeight: "var(--fw-bold)", color: "var(--txt-hi)" },
  { tag: t.strong, fontWeight: "var(--fw-heavy)", color: "var(--txt-hi)" },
  { tag: t.emphasis, fontStyle: "italic" },
  { tag: t.strikethrough, textDecoration: "line-through" },
  { tag: t.monospace, fontFamily: "var(--font-mono)", fontSize: "0.92em" },
  { tag: t.link, color: "var(--accent)" },
  { tag: t.url, color: "var(--txt-dim)" },
  { tag: t.quote, fontStyle: "italic" },
  { tag: [t.processingInstruction, t.contentSeparator, t.labelName], color: "var(--txt-dim)" },
  { tag: t.meta, color: "var(--txt-dim)", fontFamily: "var(--font-mono)", fontSize: "0.85em" },
  { tag: t.comment, color: "var(--syn-com)" },
]);

/** The editor's chrome, from the tokens (so a theme switch needs nothing). */
const chrome = EditorView.theme({
  "&": { height: "100%", color: "var(--txt)", backgroundColor: "transparent", fontSize: "1rem" },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": { fontFamily: "var(--font-ui)", lineHeight: "var(--lh-prose)" },
  ".cm-content": { maxWidth: "46rem", margin: "0 auto", padding: "var(--s5) var(--s5) 40vh", caretColor: "var(--txt-hi)" },
  ".cm-line": { padding: "0" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--txt-hi)" },
  "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection":
    { backgroundColor: "color-mix(in srgb, var(--accent) 28%, transparent)" },
});

/** The phase 3 pieces: the `[[` picker's trigger and keys, following a link,
 *  and telling the window a selection exists (Split acts on it). */
function relateExtensions(hooks: EditorHooks): Extension[] {
  const keys = (["ArrowUp", "ArrowDown", "Enter", "Tab", "Escape"] as const).map((key) => ({
    key,
    run: () => hooks.pickerKey?.(key) ?? false,
  }));
  return [
    Prec.highest(keymap.of(keys)),
    EditorView.updateListener.of((u) => {
      if (u.selectionSet || u.docChanged) {
        const r = u.state.selection.main;
        if (u.startState.selection.main.empty !== r.empty) hooks.selected?.(!r.empty);
        if (!r.empty || !u.startState.selection.main.empty) hooks.selectionMoved?.();
        let open: Parameters<NonNullable<EditorHooks["picker"]>>[0] = null;
        if (r.empty && u.view.hasFocus) {
          const line = u.state.doc.lineAt(r.head);
          const q = pickerQuery(line.text.slice(0, r.head - line.from));
          const at = q && u.view.coordsAtPos(r.head);
          if (q && at) open = { from: line.from + q.from, to: r.head, query: q.query, x: at.left, y: at.bottom };
        }
        hooks.picker?.(open);
      }
    }),
    EditorView.domEventHandlers({
      mousedown: (e, view) => {
        if (!(e.metaKey || e.ctrlKey) || e.button !== 0) return false;
        const pos = view.posAtCoords({ x: e.clientX, y: e.clientY });
        if (pos == null) return false;
        const line = view.state.doc.lineAt(pos);
        const id = linkAt(line.text, pos - line.from);
        if (!id) return false;
        e.preventDefault();
        hooks.follow?.(id);
        return true;
      },
    }),
  ];
}

/** A fresh state for a document whose file text is `text`. */
export function ideaState(text: string, hooks: EditorHooks, selection?: { anchor: number; head: number }): EditorState {
  const ext: Extension[] = [
    textExtensions(text),
    history(),
    keymap.of([...defaultKeymap, ...historyKeymap]),
    drawSelection(),
    highlightSpecialChars(),
    EditorView.lineWrapping,
    // The text is markdown SOURCE: autocorrect and autocapitalize stay off,
    // as CodeMirror defaults them, so macOS's smart dashes and quotes cannot
    // turn `---` into an em dash or a straight quote in code into a curly
    // one (unmeasured in WKWebView: the conservative default). Spelling is
    // only underlined, so it stays on.
    EditorView.contentAttributes.of({ spellcheck: "true", autocorrect: "off", autocapitalize: "off", "aria-label": "Document text" }),
    markdown({ base: markdownLanguage, extensions: [Frontmatter], codeLanguages: [] }),
    syntaxHighlighting(stage1),
    chrome,
    EditorView.updateListener.of((u) => {
      if (u.docChanged && u.transactions.some((tr) => tr.annotation(Transaction.userEvent))) hooks.edited();
    }),
    EditorView.domEventHandlers({ blur: () => { hooks.blurred(); hooks.picker?.(null); } }),
    relateExtensions(hooks),
  ];
  const state = EditorState.create({ doc: text, extensions: ext });
  if (!selection) return state;
  // Positions count one per line break, so clamp to the state's own length.
  const clamp = (n: number) => Math.max(0, Math.min(n, state.doc.length));
  return state.update({ selection: { anchor: clamp(selection.anchor), head: clamp(selection.head) } }).state;
}

/** One CodeMirror view in `parent`. Each `open` gives it a new STATE: its own
 *  history and its own line separator. */
export function mountEditor(parent: HTMLElement, hooks: EditorHooks): DocEditor {
  const v = new EditorView({ parent });
  return {
    open(text) {
      // The cursor starts at the end of the text: at 0 it would sit before the
      // frontmatter, where the guard refuses every keystroke.
      const end = Number.MAX_SAFE_INTEGER;
      v.setState(ideaState(text, hooks, { anchor: end, head: end }));
    },
    text: () => savedText(v.state),
    replace(content) {
      const sel = v.state.selection.main;
      v.setState(ideaState(content, hooks, { anchor: sel.anchor, head: sel.head }));
    },
    focus: () => v.focus(),
    selection() {
      const r = v.state.selection.main;
      if (r.empty) return null;
      return { start: v.state.sliceDoc(0, r.from).length, end: v.state.sliceDoc(0, r.to).length, text: v.state.sliceDoc(r.from, r.to) };
    },
    insert(from, to, text) {
      v.dispatch({ changes: { from, to, insert: text }, selection: { anchor: from + text.length }, userEvent: "input.complete", scrollIntoView: true });
      v.focus();
    },
    gotoLine(n) {
      const l = v.state.doc.line(Math.max(1, Math.min(n, v.state.doc.lines)));
      v.dispatch({ selection: { anchor: l.from, head: l.to }, scrollIntoView: true });
      v.focus();
    },
    destroy: () => v.destroy(),
  };
}
