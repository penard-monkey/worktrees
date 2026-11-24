// The text rules of an idea's document, pure: no import at runtime, so
// ideas-text-check.mjs loads this file as it is and drives it under node with
// the real @codemirror/state (docs/proposals/idea-board-phases.md §4, §8
// phase 1).
//
// **The file is canonical and byte-exact.** The editor holds the file's text
// and saves exactly what it holds; nothing here re-prints a document. Two
// ways an editor breaks that without anyone typing a character, each settled
// here:
//
// 1. **Line endings.** CodeMirror splits on `\r\n`, `\r` AND `\n` by default
//    and joins with `\n`, so a CRLF file, or one stray `\r`, comes back LF on
//    the first save. `lineBreakOf` names the one separator the editor is
//    built with (`EditorState.lineSeparator`), ALWAYS explicitly: `\r\n` when
//    every `\n` in the file is part of a `\r\n`, else `\n`. Under `\n` a lone
//    `\r` (and a mixed file's CRLF lines) keep their `\r` as an ordinary
//    character, so they too come back as they went in. The save reads the
//    text with `state.sliceDoc()`, which joins with that separator —
//    `doc.toString()` would join with `\n` whatever the file had. And text
//    the person INSERTS (typing Enter, a paste, a drop) takes the document's
//    own ending (`normalizeInserted`), so a paste of LF text into a CRLF file
//    does not leave it mixed; text already in the file is never touched.
//
// 2. **The frontmatter carries the `id`**, and `idea_write` refuses a text
//    that lost it. It is SHOWN, dimmed, and three of its lines are read-only:
//    the opening `---`, the `id:` line and the closing `---` (with their line
//    breaks), plus any insertion before the opening one, which would stop it
//    being the frontmatter (`guardedRanges`). Hidden would not be enough —
//    a select-all and a keystroke delete hidden text too — and the guard is
//    needed either way. The rest of the block stays the person's: later
//    phases write keys there (`related`, `absorbed`, `plan_of`) through their
//    own rows, and a person who reads them may fix one by hand. With the
//    guard, select-all + delete leaves `---\nid: …\n---\n` and nothing else,
//    which is still an idea `idea_write` accepts.

/** The separator the editor is built with for `text`. See the header. */
export function lineBreakOf(text: string): "\n" | "\r\n" {
  const lf = (text.match(/\n/g) ?? []).length;
  const crlf = (text.match(/\r\n/g) ?? []).length;
  return lf > 0 && lf === crlf ? "\r\n" : "\n";
}

/** Inserted text with every line ending made `sep`. */
export function normalizeInserted(text: string, sep: string): string {
  return /[\r\n]/.test(text) ? text.replace(/\r\n|\r|\n/g, sep) : text;
}

/** The frontmatter's span, as core's `frontmatter()` reads it: it opens with
 *  `---` as the very first line and closes at the next line that is `---` or
 *  `...`. Offsets are into `text` as the editor holds it: one character per
 *  line break (`sep`), so a CRLF file's offsets are its editor positions. */
export type Front = {
  /** The opening line's end, the start of the first key's line. */
  open: [number, number];
  /** The `id:` line, when there is one. */
  id: [number, number] | null;
  close: [number, number];
  /** Where the body begins: just past the closing line's break. */
  body: number;
};

/** `text` split into lines on `sep` alone, with each line's start offset in
 *  the joined-by-one-character space the editor uses. */
function lines(text: string, sep: string): { text: string; from: number }[] {
  const out: { text: string; from: number }[] = [];
  let pos = 0;
  for (const l of text.split(sep)) {
    out.push({ text: l, from: pos });
    pos += l.length + 1;
  }
  return out;
}

/** `doc` is the editor's text joined with `\n` (one character per break, as
 *  `state.doc.toString()` gives it); `sep` is only needed to tell a `\r` that
 *  is part of a line from one that ends it, and under `\n` it is part of it. */
export function frontOf(doc: string, sep: "\n" | "\r\n" = "\n"): Front | null {
  const ls = lines(doc, "\n");
  const bare = (s: string) => (sep === "\n" ? s.replace(/\r$/, "") : s);
  if (ls.length < 2 || bare(ls[0].text) !== "---") return null;
  let id: [number, number] | null = null;
  for (let i = 1; i < ls.length; i++) {
    const t = bare(ls[i].text);
    if (t === "---" || t === "...") {
      const end = ls[i].from + ls[i].text.length;
      return {
        open: [0, ls[0].text.length],
        id,
        close: [ls[i].from, end],
        body: Math.min(end + 1, doc.length),
      };
    }
    if (id === null && /^id:/.test(t)) id = [ls[i].from, ls[i].from + ls[i].text.length];
  }
  return null;
}

/** The ranges no change may touch, as `[from, to]` pairs for CodeMirror's
 *  `changeFilter` (which suppresses changes in them). Each guarded line takes
 *  its line break with it. The opening line's range starts at 0, so an
 *  insertion at 0 (before the `---`) is suppressed too: a change that touches
 *  a range's edge counts as inside it. `docLen` keeps the close's break from
 *  running past a document that ends on `---`. */
export function guardedRanges(f: Front | null, docLen: number): number[] {
  if (!f) return [];
  const out: number[] = [0, Math.min(f.open[1] + 1, docLen)];
  if (f.id) out.push(f.id[0], Math.min(f.id[1] + 1, docLen));
  out.push(f.close[0], Math.min(f.close[1] + 1, docLen));
  return out;
}

/** The body the Read view renders: the text after the frontmatter. */
export function bodyOf(text: string): string {
  const f = frontOf(text.replace(/\r\n/g, "\n"));
  if (!f) return text;
  // Offsets are into the LF form; count the same number of line breaks in
  // the original to find the cut.
  const breaks = text.replace(/\r\n/g, "\n").slice(0, f.body).split("\n").length - 1;
  let at = 0;
  for (let n = 0; n < breaks; n++) {
    const i = text.indexOf("\n", at);
    if (i < 0) return "";
    at = i + 1;
  }
  return text.slice(at);
}

/** True for raw HTML that is only comments (and space): the Read view draws
 *  it as nothing, as a renderer does. Later phases' anchors (`<!-- ¶ 7k2q -->`,
 *  §4.4) are comments; any other HTML stays literal, inert text. */
export function commentOnly(raw: string): boolean {
  return /^\s*(?:<!--[\s\S]*?-->\s*)+$/.test(raw);
}

/** Replace the `id:` line's value with `id`, every other byte kept: the text
 *  a removed idea's editor saves as a NEW idea. */
export function withId(text: string, id: string): string | null {
  if (!frontOf(text.replace(/\r\n/g, "\n"))?.id) return null;
  // The frontmatter is the file's head, so the first `id:` line is its own.
  const m = /^id:[ \t]*([^\r\n]*?)[ \t]*(?=\r?$)/m.exec(text);
  if (!m) return null;
  const at = m.index + m[0].indexOf(m[1], 3);
  return text.slice(0, at) + id + text.slice(at + m[1].length);
}
