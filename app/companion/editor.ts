// The Ideas editor: CodeMirror 6 on the markdown SOURCE, with Bear's stage 1
// (docs/proposals/idea-board-phases.md §4.2): headings sized, emphasis
// styled, links styled, and every marker (`#`, `**`, `` ` ``, `>`, a list's
// bullet, a link's brackets) DIMMED — never hidden. All of it is highlighting:
// marks over the text, no replacement, no widget, so what is on screen is
// every character of the file (stage 2's hidden markers and stage 3's widgets
// will be view-only changes on top). The byte-exact half — line endings, the
// guarded frontmatter, the text a save sends — is ideastate.ts.
//
// CodeMirror is the companion bundle's alone (AGENTS.md, the third admitted
// library; companion-boundary-check.mjs holds it out of app/src).
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { EditorState, Prec, Transaction, type Extension } from "@codemirror/state";
import { drawSelection, EditorView, highlightSpecialChars, keymap } from "@codemirror/view";
import { tags as t } from "@lezer/highlight";
import type { MarkdownConfig } from "@lezer/markdown";
import { linkAt, pickerQuery } from "./ideasrelate";
import { textExtensions } from "./ideastate";

/** The frontmatter as its own block, so its keys are not read as a setext
 *  heading (`created: …` over the closing `---` is one, to markdown). Shown
 *  dimmed and in the code face: it is the file's, not the prose. A block
 *  parser sees one line at a time and cannot look for the close first, so an
 *  unclosed `---` at the top dims to the end; core reads no frontmatter then
 *  (and ideastate guards none), and only the colour differs. */
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
  /** ⌘-click (Ctrl elsewhere) on an `idea:` link. */
  follow?(id: string): void;
  /** Whether text is selected, when that changes. */
  selected?(has: boolean): void;
};

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
    EditorView.contentAttributes.of({ spellcheck: "true", autocorrect: "off", autocapitalize: "off", "aria-label": "Idea text" }),
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
