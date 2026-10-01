// File paths in terminal output → clickable links (⌘-click opens the dock's
// file viewer at the line). This module is the PURE half: finding candidates in
// a line of text, and turning a wrapped run of xterm rows back into one line.
// Whether a candidate is a link at all is decided by the backend
// (`resolve_term_paths`): only a path that EXISTS, is a regular file, and lies
// inside a registered project — exactly what the viewer can read — gets one.
// So detection here is allowed to be generous; its job is to keep obvious
// non-paths (URLs, version numbers, prose like "and/or") from costing a stat.
//
// `termlinks-check.mjs` runs the fixture table against this file and the link
// provider in TerminalPane.tsx against stubs.

/** One candidate: `[start, end)` is the whole clickable span in the text (the
 *  path plus any `:42:7` / `(42,7)` / `, line 42` suffix), `path` is the path
 *  as written, without the suffix. */
export type PathHit = { start: number; end: number; path: string; line?: number; col?: number };

// What a path is made of. Deliberately no spaces (a quoted path with spaces is
// rare in agent output and ambiguous everywhere else), no `:` (that is the line
// separator), and no brackets or quotes (those wrap paths in prose).
const TOKEN = /[\p{L}\p{N}_.+@~/-]+/gu;
// A URL is masked as a whole first, so `https://x.dev/a/b.ts` is not mined for
// an `a/b.ts`. Any scheme — `file://`, `vscode://` — not just http.
const URL_RE = /\b[a-zA-Z][a-zA-Z0-9+.-]*:\/\/[^\s"'<>`]+/g;
// `name.ext`, ext starting with a letter: `App.tsx`, `.gitignore`, `Cargo.toml`.
// Rules out `1.2.3`, `v0.33.0` and `3.5` — none of which end in a letter-led ext.
const HAS_EXT = /(?:^|\/)[^/]*\.[A-Za-z][A-Za-z0-9_-]{0,15}$/;
// Lines past this are noise (a hash, a timestamp), not positions.
const MAX_LINE = 10_000_000;

const num = (s: string | undefined) => {
  if (s == null) return undefined;
  const n = Number(s);
  return Number.isInteger(n) && n >= 1 && n <= MAX_LINE ? n : undefined;
};

/** Is this token worth asking the filesystem about? */
function plausible(tok: string): boolean {
  if (tok.endsWith("/")) return false;           // a directory — the viewer shows files
  if (tok.startsWith("//")) return false;        // `//` comment markers, protocol-relative URLs
  if (!/[\p{L}]/u.test(tok)) return false;       // `1.2.3`, `--`, `/`
  if (tok.includes("@") && !tok.includes("/")) return false; // `user@host`, `pkg@1.2`
  if (HAS_EXT.test(tok)) return true;
  if (!tok.includes("/")) return false;          // a bare word
  // A slash but no extension: `bin/worktrees` is a file, `and/or` is prose.
  // Accept it only when it is anchored (`./x`, `../x`, `/x`, `~/x`) or deep
  // enough that prose does not produce it (`test/helpers/common`).
  if (/^(?:\.{1,2}\/|\/|~\/)/.test(tok)) return true;
  return (tok.match(/\//g) ?? []).length >= 2;
}

/** Every plausible path in `text`, left to right. */
export function findPaths(text: string): PathHit[] {
  const masked: [number, number][] = [];
  for (const m of text.matchAll(URL_RE)) masked.push([m.index!, m.index! + m[0].length]);
  const inUrl = (i: number) => masked.some(([a, b]) => i >= a && i < b);

  const out: PathHit[] = [];
  for (const m of text.matchAll(TOKEN)) {
    const at = m.index!;
    if (inUrl(at)) continue;
    // Sentence punctuation is not part of the path: "see src/a.ts." and
    // "(see src/a.ts)". A `-` too, for "src/a.ts—" style dashes; no real
    // filename ends in either.
    let tok = m[0].replace(/[.-]+$/, "");
    // A leading `-` is a flag (`--file=x` never reaches here, `-x/y` might).
    const lead = tok.match(/^-+/)?.[0].length ?? 0;
    tok = tok.slice(lead);
    const start = at + lead;
    if (!tok || !plausible(tok)) continue;
    let end = start + tok.length;
    let line: number | undefined;
    let col: number | undefined;
    const rest = text.slice(end, end + 32);
    // The position forms agents and tools print, most specific first.
    const pos =
      rest.match(/^:(\d+)(?::(\d+))?/) ??          // src/a.ts:42:7 (and :42-50 → 42)
      rest.match(/^\((\d+)(?:,\s*(\d+))?\)/) ??    // src/a.ts(42,7)   tsc, msbuild
      rest.match(/^#L(\d+)(?:C(\d+))?/) ??         // src/a.ts#L42     GitHub
      (text[start - 1] === '"' ? rest.match(/^",\s*line\s+(\d+)/) : null) ?? // File "a.py", line 42
      rest.match(/^,?\s+line\s+(\d+)/i);           // src/a.ts line 42
    if (pos) {
      line = num(pos[1]);
      col = line != null ? num(pos[2]) : undefined;
      if (line != null) end += pos[0].length;
    }
    out.push({ start, end, path: tok, ...(line != null ? { line } : {}), ...(col != null ? { col } : {}) });
  }
  return out;
}

// ── wrapped rows → one line ─────────────────────────────────────────────────

/** The slice of xterm's `IBufferLine` this needs — kept structural so the check
 *  script can hand it plain objects. */
export type RowLike = {
  isWrapped: boolean;
  length: number;
  getCell(x: number): { getChars(): string; getWidth(): number } | undefined;
};

/** A cell a string index came from, 0-based, with its width (2 for a wide
 *  glyph, so a link ending on one covers both cells). */
export type CellAt = { x: number; y: number; w: number };

/** How far a wrapped line may extend either way. A path does not span pages,
 *  and a pathological 10,000-row line should not be walked per mousemove. */
const MAX_WRAP = 16;

/** The logical line row `y` (0-based) belongs to, as text plus a map from each
 *  UTF-16 index of that text back to the cell it was drawn in.
 *
 *  Cells, not characters, because they differ: a wide glyph (emoji, CJK)
 *  takes two cells and its second is an empty continuation, and a surrogate
 *  pair is two string indices in ONE cell. Mapping by string index would put
 *  every link after an emoji one column off — the same width mismatch AGENTS.md
 *  records for tmux and the graphemes addon, met again from the other side. */
export function logicalLine(row: (y: number) => RowLike | undefined, y: number): { text: string; cells: CellAt[]; first: number; last: number } {
  let first = y;
  while (first > 0 && y - first < MAX_WRAP && row(first)?.isWrapped) first--;
  let last = y;
  while (last - y < MAX_WRAP && row(last + 1)?.isWrapped) last++;
  let text = "";
  const cells: CellAt[] = [];
  for (let ry = first; ry <= last; ry++) {
    const r = row(ry);
    if (!r) continue;
    for (let x = 0; x < r.length; x++) {
      const c = r.getCell(x);
      if (!c) continue;
      const w = c.getWidth();
      if (w === 0) continue; // the right half of a wide glyph
      const ch = c.getChars() || " ";
      for (let i = 0; i < ch.length; i++) cells.push({ x, y: ry, w });
      text += ch;
    }
  }
  return { text, cells, first, last };
}

/** A hit's span as xterm's 1-based, end-INCLUSIVE buffer range. */
export function hitRange(hit: PathHit, cells: CellAt[]) {
  const a = cells[hit.start];
  const b = cells[hit.end - 1];
  return { start: { x: a.x + 1, y: a.y + 1 }, end: { x: b.x + b.w, y: b.y + 1 } };
}
