// The markdown editor's formatting actions, as pure text transforms.
//
// Every one of them is (text, selection) → (text, selection) and nothing else:
// no DOM, no React, no textarea. That is what lets `mdedit-check.mjs` exercise
// them exhaustively — the awkward cases here are all about WHERE the caret ends
// up, and a rule that only ever runs against a live control is a rule nobody
// can check.
//
// Two decisions worth stating, because both look like bugs from the outside:
//
//   * Everything TOGGLES. Applying `## ` to a line that already has it strips
//     it. A toolbar whose buttons only ever add markers makes `#### ## Title`
//     out of two honest clicks, and the second click is the one you meant as
//     "undo".
//   * Italic will not eat a bold marker. `**x**` with italic applied is
//     `***x***`, not `*x*`: the `*` either side of the selection is there to
//     make it BOLD, and treating it as the italic to remove is the difference
//     between "add emphasis" and "silently remove some".

export type MdAction = "bold" | "italic" | "h1" | "h2" | "h3" | "ul" | "ol";

/** A textarea's value and selection — `start === end` is a bare caret. */
export type Sel = { text: string; start: number; end: number };

const INLINE: Record<string, string> = { bold: "**", italic: "*" };
const HEADING: Record<string, number> = { h1: 1, h2: 2, h3: 3 };

/** Any list or heading marker at the head of a line, plus the indent it sits
 *  under. One expression so that switching between the three list-ish actions
 *  replaces the marker instead of stacking a second one on top. */
const MARKER = /^(\s*)(?:(#{1,6})\s+|[-*+]\s+|\d+[.)]\s+)?/;

/** Word characters for the caret-with-no-selection case. Deliberately narrow:
 *  it exists so ⌘B on a word bolds the word, not so it can guess at phrases. */
const WORD = /[\p{L}\p{N}_'’-]/u;

/** The smallest [from, to) of `before` that has to be replaced, and what with,
 *  to turn it into `after`. Returned as a triple so the caller can hand the
 *  edit to the browser's own editing pipeline (`execCommand("insertText")`)
 *  instead of assigning a whole new value — which is what keeps it on the
 *  textarea's UNDO STACK. Replacing the value wholesale drops that history,
 *  and a formatting button you cannot ⌘Z is worse than one that is slow. */
export function spliceRange(before: string, after: string): { from: number; to: number; insert: string } {
  const max = Math.min(before.length, after.length);
  let from = 0;
  while (from < max && before[from] === after[from]) from++;
  // The tails are compared only over what the heads left, so a string that is
  // a prefix of the other cannot have the same character counted twice.
  let tail = 0;
  while (tail < max - from && before[before.length - 1 - tail] === after[after.length - 1 - tail]) tail++;
  return { from, to: before.length - tail, insert: after.slice(from, after.length - tail) };
}

export function applyMd(action: MdAction, sel: Sel): Sel {
  if (action in INLINE) return inline(INLINE[action], sel);
  return block(action, sel);
}

// ── inline: bold / italic ───────────────────────────────────────────────────

function inline(mark: string, { text, start, end }: Sel): Sel {
  // A bare caret takes the word it sits in. With no word under it (a blank
  // line, a run of punctuation) the markers go in empty and the caret lands
  // between them, which is how you start typing something bold.
  if (start === end) {
    let a = start;
    let b = start;
    while (a > 0 && WORD.test(text[a - 1])) a--;
    while (b < text.length && WORD.test(text[b])) b++;
    if (a === b) {
      return { text: text.slice(0, start) + mark + mark + text.slice(start), start: start + mark.length, end: start + mark.length };
    }
    start = a;
    end = b;
  }

  const n = mark.length;
  // The markers may be inside the selection (you dragged over them) or outside
  // it (you selected the words and are toggling them off). Both are "this is
  // already emphasised".
  const inside = end - start >= 2 * n && text.slice(start, start + n) === mark && text.slice(end - n, end) === mark;
  const outside = start >= n && text.slice(start - n, start) === mark && text.slice(end, end + n) === mark;

  if (inside && !(mark === "*" && guardedByBold(text, start + 1, end - 1))) {
    const body = text.slice(start + n, end - n);
    return { text: text.slice(0, start) + body + text.slice(end), start, end: start + body.length };
  }
  if (outside && !(mark === "*" && guardedByBold(text, start, end))) {
    const body = text.slice(start, end);
    return { text: text.slice(0, start - n) + body + text.slice(end + n), start: start - n, end: end - n };
  }

  const body = text.slice(start, end);
  return { text: text.slice(0, start) + mark + body + mark + text.slice(end), start: start + n, end: end + n };
}

/** Is the single `*` either side of [a, b) actually half of a `**` pair? If so
 *  it is the bold marker, and italic must not take it apart. */
function guardedByBold(text: string, a: number, b: number): boolean {
  return text.slice(a - 2, a) === "**" || text.slice(b, b + 2) === "**";
}

// ── block: headings and lists ───────────────────────────────────────────────

function block(action: MdAction, { text, start, end }: Sel): Sel {
  const from = text.lastIndexOf("\n", Math.max(0, start - 1)) + 1;
  // A selection that ends exactly on a line start took the newline and nothing
  // else — the line below it is not part of what you highlighted.
  const lastCh = end > start && text[end - 1] === "\n" ? end - 1 : end;
  const nlAfter = text.indexOf("\n", lastCh);
  const to = nlAfter === -1 ? text.length : nlAfter;

  const lines = text.slice(from, to).split("\n");
  // A blank line inside a multi-line selection is a paragraph break, not an
  // item — prefixing it produces a stray bullet and an empty heading. A single
  // blank line IS the target, though: that is an empty document.
  const targets = lines.filter((l) => l.trim() !== "");
  const act = targets.length ? targets : lines;

  const level = HEADING[action];
  const already = level
    ? act.every((l) => (l.match(MARKER)?.[2] ?? "").length === level)
    : action === "ul"
      ? act.every((l) => /^\s*[-*+]\s+/.test(l))
      : act.every((l) => /^\s*\d+[.)]\s+/.test(l));

  let n = 0;
  const out = lines.map((line) => {
    if (targets.length && line.trim() === "") return line;
    const m = line.match(MARKER);
    const indent = m?.[1] ?? "";
    const rest = line.slice((m?.[0] ?? "").length);
    if (already) return indent + rest;            // toggle the marker off
    n++;
    if (level) return `${indent}${"#".repeat(level)} ${rest}`;
    if (action === "ul") return `${indent}- ${rest}`;
    return `${indent}${n}. ${rest}`;
  });

  const body = out.join("\n");
  return { text: text.slice(0, from) + body + text.slice(to), start: from, end: from + body.length };
}
