// Read-mode source rendering: a line-number gutter beside highlighted text.
//
// Shared by the dock's code viewer and by markdown fenced code blocks, so a
// fence in a README colours exactly like the file it was copied from.
//
// The gutter is a SEPARATE column of numbers rather than a per-line wrapper —
// tokens are free to span newlines (block comments, template literals) and the
// two columns stay aligned because they share line-height. The cost is that
// wrapped lines desynchronise the gutter, so wrap mode hides it.
import { useLayoutEffect, useMemo, useRef } from "react";
import { tokenize } from "./highlight";

export function CodeSpans({ src, lang }: { src: string; lang: string }) {
  const toks = useMemo(() => tokenize(src, lang), [src, lang]);
  return (
    <>
      {toks.map((t, i) => (t.c ? <span key={i} className={`tk-${t.c}`}>{t.s}</span> : t.s))}
    </>
  );
}

/** A line to mark and bring into view, 1-based. `seq` identifies the REQUEST:
 *  the block scrolls once per seq, so a re-read of the file (reloadToken) moves
 *  the band with the text without yanking the view back to it. */
export type CodeMark = { line: number; col?: number; seq: number };

/** The client rects of `[from, to)` in `root`'s text, by character offset. */
function textRects(root: HTMLElement, from: number, to: number): DOMRect[] {
  const walk = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const range = document.createRange();
  let at = 0;
  let started = false;
  for (let n = walk.nextNode(); n; n = walk.nextNode()) {
    const len = n.textContent?.length ?? 0;
    // Strict: an offset at a node's END is the next node's START, and a range
    // begun at the end of one token measures as a zero-width rect at the seam
    // (the column caret came out 2px wide on exactly the columns that begin a
    // token, which is most of the ones a compiler points at).
    if (!started && from < at + len) { range.setStart(n, from - at); started = true; }
    if (started && to <= at + len) { range.setEnd(n, to - at); return Array.from(range.getClientRects()); }
    at += len;
  }
  return [];
}

export function CodeBlock({ src, lang, gutter = true, wrap = false, className = "", mark = null }: {
  src: string; lang: string; gutter?: boolean; wrap?: boolean; className?: string;
  /** Mark one line (a ⌘-clicked `file:42`, an agent's `show_doc {line}`). */
  mark?: CodeMark | null;
}) {
  // Trailing newline would render a phantom last line number.
  const body = src.endsWith("\n") ? src.slice(0, -1) : src;
  const lines = body === "" ? 0 : body.split("\n").length;
  const showGutter = gutter && !wrap && lines > 1;
  const boxRef = useRef<HTMLDivElement>(null);
  const textRef = useRef<HTMLElement>(null);
  const bandRef = useRef<HTMLDivElement>(null);
  const colRef = useRef<HTMLDivElement>(null);
  const scrolledSeq = useRef<number | null>(null);

  // Positioned from the RENDERED text rather than from `line × line-height`:
  // with wrap on, a line is as many rows as it wraps to and the gutter is
  // hidden, so only the text itself knows where line 42 is. A Range over the
  // line's characters answers that in both modes.
  useLayoutEffect(() => {
    const box = boxRef.current, code = textRef.current, band = bandRef.current, caret = colRef.current;
    if (!box || !code || !band || !caret || !mark) return;
    // Re-placed whenever the box RESIZES, not just when the mark changes: with
    // wrap on, narrowing the dock re-wraps every line above the mark and moves
    // it down, and a band placed once stays at the old y over the wrong text.
    // Only the placement re-runs — the scroll is once per request (`seq`), or
    // dragging the dock would keep yanking the view back to the line.
    const ro = new ResizeObserver(() => place(box, code, band, caret));
    ro.observe(box);
    place(box, code, band, caret);
    return () => ro.disconnect();
  }, [mark, body, lines, wrap]);

  function place(box: HTMLDivElement, code: HTMLElement, band: HTMLDivElement, caret: HTMLDivElement) {
    band.style.display = "none";
    caret.style.display = "none";
    if (!mark || mark.line > lines) return; // past the end: mark nothing, stay put
    let start = 0;
    for (let i = 1; i < mark.line; i++) start = body.indexOf("\n", start) + 1;
    const nl = body.indexOf("\n", start);
    const end = nl < 0 ? body.length : nl;
    const origin = box.getBoundingClientRect();
    // An empty line has no glyph to measure; measure its newline instead.
    const rects = textRects(code, start, Math.max(end, Math.min(start + 1, body.length)));
    if (!rects.length) return;
    // A Range rect is the GLYPH box (the font's content area, ~15px at 13px
    // type), not the line box the gutter numbers sit in (`line-height` 1.55,
    // ~20px) — measured in WebKit, a band 5px short of its own gutter row. So
    // each rect only says where a visual row is CENTRED, and the row is one
    // line-height tall around that.
    const lh = parseFloat(getComputedStyle(code).lineHeight) || rects[0].height;
    const mid = (r: DOMRect) => (r.top + r.bottom) / 2;
    const top = Math.min(...rects.map(mid)) - lh / 2 - origin.top;
    const bottom = Math.max(...rects.map(mid)) + lh / 2 - origin.top;
    band.style.display = "block";
    band.style.top = `${top}px`;
    band.style.height = `${Math.max(bottom - top, 1)}px`;
    // As wide as the CONTENT, not the box: an unwrapped long line scrolls the
    // box sideways, and a band that stopped at the box's edge would end there.
    band.style.width = `${Math.max(box.scrollWidth, box.clientWidth)}px`;
    let colLeft: number | null = null;
    if (mark.col && start + mark.col - 1 < end) {
      const c = textRects(code, start + mark.col - 1, start + mark.col)[0];
      if (c) {
        colLeft = c.left - origin.left;
        caret.style.display = "block";
        caret.style.left = `${colLeft}px`;
        caret.style.top = `${c.top - origin.top}px`;
        caret.style.width = `${Math.max(c.width, 2)}px`;
        caret.style.height = `${c.height}px`;
      }
    }
    if (scrolledSeq.current === mark.seq) return;
    scrolledSeq.current = mark.seq;
    const scroller = box.closest<HTMLElement>(".scroll") ?? box.parentElement;
    if (!scroller) return;
    // A third of the way down rather than centred: what follows a line is
    // usually what you came to read, and the line still sits well clear of
    // the header.
    const sr = scroller.getBoundingClientRect();
    scroller.scrollTop += origin.top + top - sr.top - scroller.clientHeight / 3;
    if (colLeft != null && !wrap) {
      const x = origin.left + colLeft - sr.left;
      if (x < 0 || x > scroller.clientWidth - 40) scroller.scrollLeft += x - scroller.clientWidth / 3;
    }
  }

  return (
    <div ref={boxRef} className={`code ${wrap ? "wrap" : ""} ${mark ? "marked" : ""} ${className}`.trim()}>
      {showGutter && (
        <div className="code-gutter" aria-hidden="true">
          {Array.from({ length: lines }, (_, i) => (
            <div key={i} className={mark && mark.line === i + 1 ? "on" : undefined}>{i + 1}</div>
          ))}
        </div>
      )}
      <pre className="code-text"><code ref={textRef}><CodeSpans src={body} lang={lang} /></code></pre>
      {mark && <div ref={bandRef} className="code-mark" aria-hidden="true" />}
      {mark && <div ref={colRef} className="code-mark-col" aria-hidden="true" />}
    </div>
  );
}
