// Unified diff → aligned rows, plus the word-level diff for a changed pair.
//
// The LINE diff is git's: the backend runs `git diff --unified=<huge>` so one
// payload carries the whole file's context AND the alignment, and nothing here
// has to implement Myers. What IS here is the parse, the pairing of a `-` run
// with the `+` run that follows it, and the intra-line word diff — none of
// which git gives you in a form a two-column grid can consume.
//
// Two arrays come out beside the rows: the reconstructed OLD text and NEW text.
// They exist so `tokenizeLines` can highlight each side as a whole file. A hunk
// tokenized on its own mis-colours anything whose context opened above it — an
// unterminated string, the inside of a block comment, a Python docstring — and
// full context is exactly what makes reconstructing them possible.

/** `mod` is a `-` line paired with the `+` line that replaced it: both sides
 *  present, and the only kind that carries a word-level diff. `gap` is a break
 *  in the line numbering, which full context should never produce — it is here
 *  because a patch that arrived with ordinary `-U3` hunks must still render
 *  something honest rather than silently splicing unrelated lines together. */
export type DiffRowKind = "ctx" | "del" | "add" | "mod" | "gap";

export type DiffRow = {
  kind: DiffRowKind;
  /** 1-based line number in the OLD file, null when this row adds a line. */
  oldNo: number | null;
  /** 1-based line number in the NEW file, null when this row deletes a line. */
  newNo: number | null;
  /** Index into `oldLines` — NOT `oldNo - 1`. They agree only while the patch
   *  is gapless; a `-U3` patch skips lines the reconstruction never saw. */
  oldIdx: number | null;
  newIdx: number | null;
};

export type ParsedDiff = {
  rows: DiffRow[];
  /** The old side as text, in row order. Feed to `tokenizeLines`. */
  oldLines: string[];
  newLines: string[];
  /** Any line of the patch was a "Binary files … differ" / "GIT binary patch"
   *  marker — there is nothing to show line by line. */
  binary: boolean;
};

const EMPTY: ParsedDiff = { rows: [], oldLines: [], newLines: [], binary: false };

/** `@@ -a,b +c,d @@` — the counts are optional (`@@ -1 +1 @@` for a one-line
 *  hunk), which is why the group is `(?:,(\d+))?` and not `,(\d+)`. */
const HUNK = /^@@+ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/;

/**
 * Parse one file's unified diff. Everything before the first `@@` is skipped,
 * so a payload with or without the `diff --git`/`---`/`+++` preamble both work.
 *
 * An empty patch (the file is identical to the base) yields zero rows, which is
 * what the viewer renders as "no changes" — deliberately distinct from a parse
 * that produced only context rows.
 */
export function parseUnifiedDiff(patch: string): ParsedDiff {
  if (!patch) return EMPTY;
  if (/^(Binary files .* differ|GIT binary patch)$/m.test(patch)) return { ...EMPTY, binary: true };

  const rows: DiffRow[] = [];
  const oldLines: string[] = [];
  const newLines: string[] = [];
  // Buffers for the current `-` run and the `+` run that follows it. They are
  // flushed on the first line that is neither, which is what turns
  // "three deletions then three additions" into three side-by-side pairs
  // instead of three empty-left rows above three empty-right ones.
  let dels: { no: number; text: string }[] = [];
  let adds: { no: number; text: string }[] = [];

  const flush = () => {
    if (!dels.length && !adds.length) return;
    const n = Math.max(dels.length, adds.length);
    for (let k = 0; k < n; k++) {
      const d = dels[k];
      const a = adds[k];
      // Leftovers on one side become plain del/add rows: a 5-for-2 replacement
      // is two `mod` rows and three `del`s, which reads as what it is.
      let oldIdx: number | null = null;
      let newIdx: number | null = null;
      if (d) { oldIdx = oldLines.length; oldLines.push(d.text); }
      if (a) { newIdx = newLines.length; newLines.push(a.text); }
      rows.push({
        kind: d && a ? "mod" : d ? "del" : "add",
        oldNo: d ? d.no : null,
        newNo: a ? a.no : null,
        oldIdx,
        newIdx,
      });
    }
    dels = [];
    adds = [];
  };

  let oldNo = 0;
  let newNo = 0;
  let inHunk = false;
  // A hunk header whose start does not continue where the last one stopped
  // means lines were skipped. Recorded once, at the boundary, rather than per
  // missing line: the row is a marker, not a stand-in for content we do not have.
  let expectOld = 0;

  // The trailing newline is dropped BEFORE splitting: `split("\n")` would
  // otherwise hand back a final "" that the empty-context-line rule below reads
  // as a real blank line, appending a phantom row to every diff.
  let files = 0;
  for (const raw of stripTrailingNewline(patch).split("\n")) {
    // ONE file's diff, as the doc says — the backend always passes `-- <path>`.
    // Stopping at the second header keeps that a guarantee rather than a habit:
    // a multi-file patch would otherwise splice file 2's rows onto file 1's
    // under file 1's line numbers, with no gap marker, because the `diff --git`
    // line resets `inHunk` before the next `@@` arrives.
    if (raw.startsWith("diff --git ") && ++files > 1) break;
    const h = HUNK.exec(raw);
    if (h) {
      flush();
      oldNo = Number(h[1]);
      newNo = Number(h[3]);
      if (inHunk && oldNo > expectOld) rows.push({ kind: "gap", oldNo: null, newNo: null, oldIdx: null, newIdx: null });
      inHunk = true;
      continue;
    }
    if (!inHunk) continue; // preamble: `diff --git`, `index`, `---`, `+++`
    // `\ No newline at end of file` annotates the line ABOVE it. The viewer has
    // no way to render the distinction and inventing a row for it would shift
    // every number below by one.
    if (raw.startsWith("\\")) continue;
    const marker = raw[0];
    const text = raw.slice(1);
    if (marker === "-") {
      dels.push({ no: oldNo++, text });
    } else if (marker === "+") {
      adds.push({ no: newNo++, text });
    } else if (marker === " " || raw === "") {
      // `raw === ""` is DEFENSIVE, not observed: git writes a blank context line
      // as a single space and `app/scripts/diff-check.mjs` passes with this arm
      // removed. It stays because the failure it prevents is silent and total —
      // anything that strips trailing whitespace between git and here (a copied
      // patch, a future transport) would end the hunk at the file's first blank
      // line and the viewer would show a truncated file as if it were whole.
      flush();
      rows.push({ kind: "ctx", oldNo: oldNo++, newNo: newNo++, oldIdx: oldLines.length, newIdx: newLines.length });
      oldLines.push(text);
      newLines.push(text);
    } else {
      // Anything else ends the hunk — the next `diff --git` in a multi-file
      // patch, or trailing junk.
      flush();
      inHunk = false;
      continue;
    }
    expectOld = oldNo;
  }
  flush();
  return { rows, oldLines, newLines, binary: false };
}

/** A patch for a file git has never seen. The backend does not spawn git for an
 *  untracked path — there is no "before" to name — so the all-added rows are
 *  built here from the content the viewer already read. */
export function allAdded(content: string): ParsedDiff {
  const lines = stripTrailingNewline(content).split("\n");
  return {
    rows: lines.map((_, i) => ({ kind: "add" as const, oldNo: null, newNo: i + 1, oldIdx: null, newIdx: i })),
    oldLines: [],
    newLines: lines,
    binary: false,
  };
}

/** A file with no trailing newline has the same line count as one with; keeping
 *  the empty tail would render a phantom last line, the same reason
 *  `CodeBlock` trims it. */
export function stripTrailingNewline(s: string) {
  return s.endsWith("\n") ? s.slice(0, -1) : s;
}

// ── word-level diff ──────────────────────────────────────────────────────────

/** A half-open `[start, end)` range of CHARACTER offsets within one line. */
export type Span = { start: number; end: number };

/** Words, runs of whitespace, and single punctuation characters. Splitting on
 *  whitespace alone would make `foo(bar)` one atom, so renaming `bar` would
 *  light the whole call up; splitting per character makes the LCS quadratic in
 *  line length and highlights individual letters inside an unrelated word. */
const WORDS = /[A-Za-z0-9_$]+|\s+|[^\sA-Za-z0-9_$]/g;

/** Above this many word-pairs the LCS table is not worth building — a minified
 *  bundle line is tens of thousands of atoms and would freeze the render. Past
 *  it, the cheap prefix/suffix trim still gives a usable highlight. */
const LCS_BUDGET = 160_000;

/**
 * Which parts of `a` were removed and which parts of `b` were added, as
 * character ranges. Returns empty spans for both when the lines are identical,
 * and — deliberately — when they share nothing at all: highlighting an entire
 * line on both sides says no more than the row's own del/add colour already
 * does, and reads as noise.
 */
export function wordDiff(a: string, b: string): { a: Span[]; b: Span[] } {
  if (a === b) return { a: [], b: [] };
  const at = atoms(a);
  const bt = atoms(b);

  // Trim the common head and tail first. It is what makes the common case (one
  // identifier changed in a long line) cost nothing, and it shrinks what the
  // LCS below has to chew on.
  let lo = 0;
  while (lo < at.length && lo < bt.length && at[lo].s === bt[lo].s) lo++;
  let hiA = at.length;
  let hiB = bt.length;
  while (hiA > lo && hiB > lo && at[hiA - 1].s === bt[hiB - 1].s) { hiA--; hiB--; }

  const midA = at.slice(lo, hiA);
  const midB = bt.slice(lo, hiB);
  if (!midA.length && !midB.length) return { a: [], b: [] };
  // Nothing in common: the row colour already says "this line was replaced".
  if (!midA.length || !midB.length) {
    return {
      a: midA.length ? [{ start: midA[0].start, end: midA[midA.length - 1].end }] : [],
      b: midB.length ? [{ start: midB[0].start, end: midB[midB.length - 1].end }] : [],
    };
  }

  const keepA = new Set<number>();
  const keepB = new Set<number>();
  let ran = false;
  if (midA.length * midB.length <= LCS_BUDGET) {
    lcsKeep(midA.map((t) => t.s), midB.map((t) => t.s), keepA, keepB);
    ran = true;
  }
  // These two lines share NOTHING: no common prefix, no common suffix, and the
  // LCS found nothing between them either. Marking every word on both sides then
  // says exactly what the row's own del/add colour already says, in a form that
  // reads as noise — so mark nothing.
  //
  // All three conditions matter. Testing only "the LCS kept nothing" is wrong,
  // and wrong in the direction that silently removes working highlights: for
  // `const foo = 1;` → `const bar = 1;` the trim leaves `foo` against `bar`,
  // which have no common atom, and the one word you actually changed would stop
  // being marked. `lo`/`hiA`/`hiB` untouched is what makes this "the lines share
  // nothing", not "the leftover middles do".
  //
  // `ran` too: OVER budget the sets are also empty, but there emptiness means
  // "not computed", and the right answer is still to mark the trimmed middle.
  const sharedNothing = lo === 0 && hiA === at.length && hiB === bt.length;
  if (ran && sharedNothing && !keepA.size && !keepB.size) return { a: [], b: [] };
  // Over budget: keepA/keepB stay empty, so the whole trimmed middle is marked.
  // That is the prefix/suffix answer, which is still far better than the line.
  return {
    a: merge(midA.filter((_, i) => !keepA.has(i))),
    b: merge(midB.filter((_, i) => !keepB.has(i))),
  };
}

type Atom = { s: string; start: number; end: number };

function atoms(s: string): Atom[] {
  const out: Atom[] = [];
  WORDS.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = WORDS.exec(s))) out.push({ s: m[0], start: m.index, end: m.index + m[0].length });
  return out;
}

/** Classic LCS table. Fills `keepA`/`keepB` with the indexes that MATCH, so the
 *  caller marks everything else. Bounded by LCS_BUDGET at the call site. */
function lcsKeep(a: string[], b: string[], keepA: Set<number>, keepB: Set<number>) {
  const n = a.length;
  const m = b.length;
  // One flat Int32Array rather than nested arrays: (n+1)*(m+1) is up to
  // ~160k cells and the row-of-arrays version allocates n+1 objects for it.
  const dp = new Int32Array((n + 1) * (m + 1));
  const at = (i: number, j: number) => dp[i * (m + 1) + j];
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i * (m + 1) + j] = a[i] === b[j] ? at(i + 1, j + 1) + 1 : Math.max(at(i + 1, j), at(i, j + 1));
    }
  }
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) { keepA.add(i); keepB.add(j); i++; j++; }
    else if (at(i + 1, j) >= at(i, j + 1)) i++;
    else j++;
  }
}

/** Adjacent atoms → one span, so a changed `foo.bar` is a single highlight
 *  rather than three abutting ones with seams down the middle. */
function merge(list: Atom[]): Span[] {
  const out: Span[] = [];
  for (const t of list) {
    const last = out[out.length - 1];
    if (last && last.end === t.start) last.end = t.end;
    else out.push({ start: t.start, end: t.end });
  }
  return out;
}
