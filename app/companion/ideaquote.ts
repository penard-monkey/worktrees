// The editor's selection as a quote for the conversation: a reference, never
// an edit. Pure, so ideas-quote-check.mjs runs the file itself.

export type Quote = {
  /** What it quotes, for telling one selection from the next. */
  key: string;
  /** "lines 12–15", counted in the idea's file, frontmatter included. */
  lines: string;
  /** What a message carries: the source line, then the text as a quote. */
  text: string;
  /** The person made this selection. False when the text under a kept
   *  selection was replaced (Claude's edit, a reload): then it is offered,
   *  not attached. */
  auto: boolean;
};

/** Where `start..end` of `text` (UTF-16 offsets, as `EditorHandle.selection()`
 *  gives) starts and ends, in 1-based lines. A selection that ends right after
 *  a line break does not reach into the next line. */
export function lineRange(text: string, start: number, end: number): { from: number; to: number } {
  const breaks = (s: string) => (s.match(/\r\n|\r|\n/g) ?? []).length;
  const from = breaks(text.slice(0, start)) + 1;
  const picked = text.slice(start, end).replace(/(\r\n|\r|\n)$/, "");
  return { from, to: from + breaks(picked) };
}

/** The quote of `sel` in `text`, or null when it holds only white space. Tabs
 *  become four spaces, because a message cannot hold one; nothing else is
 *  changed, so a character a message refuses is refused where it is sent. */
export function quoteOf(title: string, text: string, sel: { start: number; end: number }, auto: boolean): Quote | null {
  const picked = text.slice(sel.start, sel.end);
  if (!picked.trim()) return null;
  const { from, to } = lineRange(text, sel.start, sel.end);
  const lines = from === to ? `line ${from}` : `lines ${from}–${to}`;
  const body = picked.replace(/(\r\n|\r|\n)$/, "").replace(/\t/g, "    ").split(/\r\n|\r|\n/);
  const name = title.replace(/\s+/g, " ").trim() || "Untitled";
  return {
    key: `${sel.start}:${sel.end}:${picked}`,
    lines,
    text: [`> from *${name}*, ${lines}:`, ...body.map((l) => (l.trim() ? `> ${l.trimEnd()}` : ">"))].join("\n"),
    auto,
  };
}
