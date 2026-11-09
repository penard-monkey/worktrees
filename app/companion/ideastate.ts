// The editor's STATE half: what makes the text it holds byte-exact with the
// file (ideatext.ts says why each piece exists). Only @codemirror/state, no
// view, so ideas-text-check.mjs builds the very same state under node and
// proves the bytes, with and without CRLF, before any DOM is involved.
import { EditorSelection, EditorState, Transaction, type Extension, type Text, type TransactionSpec } from "@codemirror/state";
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
