// The Files tab's diff renderer: two columns of the same file, aligned row for
// row, with the changed WORDS inside a changed line picked out.
//
// Layout is one CSS grid, not two scrolling panes. A pane per side has to have
// its scroll positions synchronised in JS and still desynchronises the moment a
// line wraps, because the two sides wrap at different lines. One grid row per
// aligned pair makes the row take the height of its tallest cell, so alignment
// is a property of the layout rather than something to maintain.
//
// Falls back to a unified (one-column) render when there is not enough width
// for two — the same shape as `orientationFor` in FilesPane, but measured on
// the VIEWER, since this also mounts full-width in reading mode where the
// dock's width says nothing.
import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import { tokenizeLines, type Tok } from "./highlight";
import { allAdded, parseUnifiedDiff, wordDiff, type DiffRow, type Span } from "./diff";

/** Body width (px) at or above which the diff goes side by side. Below it two
 *  columns of code are each too narrow to read a line of source without
 *  wrapping every one of them. */
export const DIFF_SIDE_AT = 560;

/** Rows past which the grid stops rendering. Full context means a 40k-line file
 *  is 40k rows × 4 cells, and the DOM cost lands on every keystroke elsewhere in
 *  the app. The cap is ANNOUNCED — a diff that silently stopped halfway reads as
 *  a file that has no more changes. */
const MAX_ROWS = 8000;

export type FileDiffDto = {
  /** Echo of what was asked for, so a response for the other base can be dropped. */
  against: "base" | "head";
  /** The ref the base resolved to — shown in the header, so "vs base" is never
   *  a claim the user has to take on faith. */
  base_label: string;
  /** git's unified diff at full context. Empty means nothing differs. */
  patch: string;
  /** git has never seen this path: there is no before, every line is an add. */
  untracked: boolean;
  binary: boolean;
  /** The patch hit the backend's byte cap and is cut short. */
  truncated: boolean;
};

type CellTok = Tok & { w?: true };

/**
 * Re-cut a line's syntax tokens at the word-diff boundaries, so one span can
 * carry both a colour and the "this word changed" mark.
 *
 * The two segmentations are independent — a changed range can start in the
 * middle of a string token — so neither can simply wrap the other; they have to
 * be intersected.
 */
function markWords(toks: Tok[], spans: Span[]): CellTok[] {
  if (!spans.length) return toks;
  const out: CellTok[] = [];
  let at = 0; // character offset of `t` within the line
  let si = 0;
  for (const t of toks) {
    let pos = 0;
    while (pos < t.s.length) {
      const abs = at + pos;
      // Drop spans that ended before this token — `si` only ever moves forward,
      // which is what keeps this linear rather than a scan per token.
      while (si < spans.length && spans[si].end <= abs) si++;
      const sp = spans[si];
      if (!sp || sp.start >= at + t.s.length) {
        out.push({ s: t.s.slice(pos), c: t.c });
        break;
      }
      if (sp.start > abs) {
        const upto = sp.start - at;
        out.push({ s: t.s.slice(pos, upto), c: t.c });
        pos = upto;
        continue;
      }
      const upto = Math.min(sp.end - at, t.s.length);
      out.push({ s: t.s.slice(pos, upto), c: t.c, w: true });
      pos = upto;
    }
    at += t.s.length;
  }
  return out;
}

/** What a screen reader hears before a changed line. The gutters and the sign
 *  column are `aria-hidden` — read aloud they are loose numbers and punctuation
 *  — which left nothing announcing whether a line was added or removed; the two
 *  sides simply interleaved. Rendered, never seen (`.vh`).
 *
 *  A context line says nothing: it is the majority of the rows, and prefixing
 *  every one of them with "unchanged" would bury the few that matter. */
function Say({ kind }: { kind: string }) {
  const word = kind.includes("del") ? "removed" : kind.includes("add") ? "added" : kind.includes("none") ? "no line" : "";
  return word ? <span className="vh">{word}: </span> : null;
}

function Cell({ toks }: { toks: CellTok[] }) {
  return (
    <>
      {toks.map((t, i) =>
        t.w
          ? <mark key={i} className={`dw${t.c ? ` tk-${t.c}` : ""}`}>{t.s}</mark>
          : t.c ? <span key={i} className={`tk-${t.c}`}>{t.s}</span> : <Fragment key={i}>{t.s}</Fragment>,
      )}
      {/* A zero-width joiner keeps an EMPTY line's cell one line-height tall.
          Without it a blank row collapses and the two sides fall out of step —
          the one alignment failure the grid cannot fix for itself, because an
          empty cell genuinely has no content to be as tall as. */}
      {toks.length === 0 && "​"}
    </>
  );
}

/** One row as the grid renders it: the row itself plus `src`, its index in the
 *  ORIGINAL row list. Unified splits a `mod` in two and both halves keep the
 *  same `src`, which is how each still finds its word spans. */
type Cut = { r: DiffRow; src: number };

/** `mod` rows are one row with both sides; unified has one column, so each of
 *  them becomes the deletion followed by the addition. */
function unify(rows: DiffRow[]): Cut[] {
  const out: Cut[] = [];
  rows.forEach((r, src) => {
    if (r.kind !== "mod") { out.push({ r, src }); return; }
    out.push({ r: { kind: "del", oldNo: r.oldNo, newNo: null, oldIdx: r.oldIdx, newIdx: null }, src });
    out.push({ r: { kind: "add", oldNo: null, newNo: r.newNo, oldIdx: null, newIdx: r.newIdx }, src });
  });
  return out;
}

export function DiffView({ diff, content, lang, wrap }: {
  diff: FileDiffDto;
  /** The working file as the viewer already read it — the source for an
   *  untracked path, where the backend has no git output to give. */
  content: string;
  lang: string;
  wrap: boolean;
}) {
  const hostRef = useRef<HTMLDivElement>(null);
  const [w, setW] = useState(0);
  useEffect(() => {
    const el = hostRef.current;
    if (!el) return;
    const ro = new ResizeObserver(([e]) => setW(e.contentRect.width));
    ro.observe(el);
    setW(el.getBoundingClientRect().width);
    return () => ro.disconnect();
  }, []);
  // Until the first measurement lands, assume side-by-side: `0 < DIFF_SIDE_AT`
  // would render one frame of unified and then reflow the whole grid, which is
  // a visible flash on every file you open in a wide dock.
  const side = w === 0 || w >= DIFF_SIDE_AT;

  const parsed = useMemo(
    () => (diff.untracked ? allAdded(content) : parseUnifiedDiff(diff.patch)),
    [diff.untracked, diff.patch, content],
  );
  const oldToks = useMemo(() => tokenizeLines(parsed.oldLines.join("\n"), lang), [parsed.oldLines, lang]);
  const newToks = useMemo(() => tokenizeLines(parsed.newLines.join("\n"), lang), [parsed.newLines, lang]);
  // Word spans for every `mod` row, keyed by the row's index in `parsed.rows` so
  // the unified expansion below can look them up by the row it came from.
  const words = useMemo(() => {
    const m = new Map<number, { a: Span[]; b: Span[] }>();
    parsed.rows.forEach((r, i) => {
      // Bounded by MAX_ROWS like the render is. Without this the cap protected
      // only the DOM: a generated file rewritten line by line would still run a
      // full LCS per row for every row in it — tens of thousands of them, each
      // up to LCS_BUDGET cells — and freeze the main thread before the capped
      // grid ever painted. `src` never exceeds its row index (unify only ever
      // ADDS rows, in order), so this bound covers both layouts.
      if (i >= MAX_ROWS) return;
      if (r.kind !== "mod" || r.oldIdx === null || r.newIdx === null) return;
      m.set(i, wordDiff(parsed.oldLines[r.oldIdx], parsed.newLines[r.newIdx]));
    });
    return m;
  }, [parsed]);

  const rows: Cut[] = useMemo(
    () => (side ? parsed.rows.map((r, src) => ({ r, src })) : unify(parsed.rows)),
    [parsed.rows, side],
  );

  if (diff.binary || parsed.binary)
    return <div className="tree-note">binary file — no line-by-line diff</div>;
  if (!diff.untracked && !diff.patch)
    // The label is empty for a path in a registered folder that is not a git
    // repo at all, and "no changes vs " with nothing after it reads as a bug.
    return <div className="tree-note">{diff.base_label ? `no changes vs ${diff.base_label}` : "no changes"}</div>;
  if (!parsed.rows.length)
    return <div className="tree-note">nothing to show — the patch had no hunks</div>;

  const shown = rows.slice(0, MAX_ROWS);
  const oldSpans = (i: number) => words.get(i)?.a ?? [];
  const newSpans = (i: number) => words.get(i)?.b ?? [];
  const tk = (arr: Tok[][], idx: number | null) => (idx === null ? [] : arr[idx] ?? []);

  return (
    <div className="scroll diff-scroll" ref={hostRef}>
      {diff.truncated && <div className="tree-note diff-note">diff truncated — only the first part is shown</div>}
      <div className={`diff-grid ${side ? "side" : "unified"}${wrap ? " wrap" : ""}`} data-side={side ? "1" : "0"}>
        {shown.map(({ r, src: i }, k) => {
          if (r.kind === "gap")
            return side
              ? <Fragment key={k}>
                  <div className="dg gap" aria-hidden="true">⋯</div><div className="dc gap" />
                  <div className="dg gap" aria-hidden="true">⋯</div><div className="dc gap" />
                </Fragment>
              : <Fragment key={k}>
                  <div className="dg gap" aria-hidden="true">⋯</div><div className="dg gap" aria-hidden="true" />
                  <div className="ds gap" aria-hidden="true" /><div className="dc gap" />
                </Fragment>;
          // On the side-by-side grid a `ctx` row paints neither half; `mod`
          // paints both, one as a deletion and one as an addition.
          const lc = r.kind === "ctx" ? "" : r.oldIdx !== null ? " del" : " none";
          const rc = r.kind === "ctx" ? "" : r.newIdx !== null ? " add" : " none";
          if (side)
            return (
              <Fragment key={k}>
                <div className={`dg${lc}`} aria-hidden="true">{r.oldNo ?? ""}</div>
                <div className={`dc${lc}`} data-k={r.kind}>
                  <Say kind={lc} />
                  <Cell toks={markWords(tk(oldToks, r.oldIdx), oldSpans(i))} />
                </div>
                <div className={`dg${rc}`} aria-hidden="true">{r.newNo ?? ""}</div>
                <div className={`dc${rc}`} data-k={r.kind}>
                  <Say kind={rc} />
                  <Cell toks={markWords(tk(newToks, r.newIdx), newSpans(i))} />
                </div>
              </Fragment>
            );
          const cls = r.kind === "ctx" ? "" : r.kind === "del" ? " del" : " add";
          const from = r.kind === "del" ? tk(oldToks, r.oldIdx) : tk(newToks, r.newIdx);
          const sp = r.kind === "del" ? oldSpans(i) : r.kind === "add" ? newSpans(i) : [];
          return (
            <Fragment key={k}>
              <div className={`dg${cls}`} aria-hidden="true">{r.oldNo ?? ""}</div>
              <div className={`dg${cls}`} aria-hidden="true">{r.newNo ?? ""}</div>
              {/* The sign is its own column so it never lands in a copied
                  selection of the code — the whole reason not to fake it with
                  a ::before on the text cell. */}
              <div className={`ds${cls}`} aria-hidden="true">{r.kind === "del" ? "−" : r.kind === "add" ? "+" : ""}</div>
              <div className={`dc${cls}`} data-k={r.kind}>
                <Say kind={cls} />
                <Cell toks={markWords(from, sp)} />
              </div>
            </Fragment>
          );
        })}
      </div>
      {rows.length > MAX_ROWS && (
        // "rows", not "lines": unified splits every `mod` into two rows, so the
        // same remainder counted as lines would overstate by up to 2×.
        <div className="tree-note diff-note">
          {rows.length - MAX_ROWS} more rows not shown — open in your editor for the whole file
        </div>
      )}
    </div>
  );
}
