// ⌘F find — one bar, two very different haystacks.
//
//   terminal  — the xterm buffer, searched by @xterm/addon-search (wired in
//               TerminalPane.tsx). Note it can only see what the app RECEIVED:
//               a tmux attach replays the visible screen, not tmux's history,
//               so find covers the session's output since attach.
//   file      — the RENDERED viewer body, searched here by `useFileFind`.
//
// The file side deliberately searches the DOM rather than the file's source
// text. Markdown preview is built from `marked` tokens, so source offsets do
// not map to what is on screen; the code view is tokenised into thousands of
// spans, so injecting <mark> during render would re-reconcile the whole file on
// every keystroke. Walking the rendered text gives one implementation for code,
// markdown preview and SVG source, and paints through the CSS Custom Highlight
// API — no DOM mutation at all, so React never sees it.
import { useCallback, useEffect, useRef, useState } from "react";

// ── the bar ────────────────────────────────────────────────────────────────

export type FindBarProps = {
  query: string;
  onQuery: (v: string) => void;
  /** 1-based position of the active hit; 0 when there are none */
  index: number;
  count: number;
  /** more matches exist than were collected (see MAX_HITS) */
  capped?: boolean;
  caseSensitive: boolean;
  onCaseSensitive: (v: boolean) => void;
  onNext: () => void;
  onPrev: () => void;
  onClose: () => void;
  /** bumps every time ⌘F is pressed — refocus the field and select what is in
   *  it, the way a browser's find bar does */
  focusToken: number;
  /** hover note on the field, e.g. the terminal's scrollback caveat */
  hint?: string;
};

export function FindBar(props: FindBarProps) {
  const { query, onQuery, index, count, capped, caseSensitive, onCaseSensitive,
    onNext, onPrev, onClose, focusToken, hint } = props;
  const ref = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.focus();
    el.select();
  }, [focusToken]);

  const empty = query === "";
  const none = !empty && count === 0;

  return (
    <div
      className="findbar"
      role="search"
      // On the BAR, not the field: Escape has to close from wherever focus
      // landed — clicking `Aa` or `›` moves it to that button, and a bar you
      // cannot dismiss without clicking back into the field is worse than no
      // shortcut. Every handled key stops propagating, because the bar lives
      // inside the terminal and inside the reader (both of which own Escape)
      // and the app's one window-level keydown is an ancestor of all of it.
      onKeyDown={(e) => {
        const inField = e.target === ref.current;
        const k = e.key.toLowerCase();
        // The Enter that COMMITS an IME composition is not a "next match" —
        // finishing a CJK word would otherwise scroll the view away.
        if (e.nativeEvent.isComposing) return;
        // Enter only repeats the search from the FIELD — on a button it is that
        // button's own activation, and handling it here would fire twice.
        if ((inField && e.key === "Enter") || ((e.metaKey || e.ctrlKey) && k === "g")) {
          e.preventDefault();
          e.stopPropagation();
          if (e.shiftKey) onPrev(); else onNext();
        } else if (e.key === "Escape") {
          e.preventDefault();
          e.stopPropagation();
          onClose();
        }
      }}
    >
      <input
        ref={ref}
        className="find-q"
        value={query}
        placeholder="Find"
        aria-label="Find"
        title={hint}
        spellCheck={false}
        autoComplete="off"
        onChange={(e) => onQuery(e.currentTarget.value)}
      />
      <span className={"find-n" + (none ? " none" : "")}>
        {/* Past the cap there is no meaningful position, so the count stands
            alone rather than pairing with a made-up index. */}
        {empty ? "" : none ? "no results" : index === 0 ? `${count}+` : `${index}/${count}${capped ? "+" : ""}`}
      </span>
      <button
        className={"ctrl sm icon-only" + (caseSensitive ? " on" : "")}
        aria-pressed={caseSensitive}
        title={caseSensitive ? "Match case: on" : "Match case: off"}
        onClick={() => onCaseSensitive(!caseSensitive)}
      >
        Aa
      </button>
      <button className="ctrl sm icon-only" title="Previous match (⇧⏎)" disabled={count === 0} onClick={onPrev}>‹</button>
      <button className="ctrl sm icon-only" title="Next match (⏎)" disabled={count === 0} onClick={onNext}>›</button>
      <button className="ctrl sm icon-only" title="Close (Esc)" onClick={onClose}>×</button>
    </div>
  );
}

// ── theme colours ──────────────────────────────────────────────────────────

/** xterm's search decorations only accept `#RRGGBB`, so the find colours are
 *  literal hex per theme in tokens.css rather than a color-mix(). Read live so
 *  a theme switch repaints with everything else. */
export function findColors() {
  const cs = getComputedStyle(document.documentElement);
  const v = (name: string, fallback: string) => cs.getPropertyValue(name).trim() || fallback;
  return { hit: v("--find-hit", "#4a3b23"), on: v("--find-on", "#e0af68") };
}

// ── file find ──────────────────────────────────────────────────────────────

/** Collecting every match in a huge file is wasted work — nobody pages through
 *  ten thousand hits, and each one is a live Range the browser has to keep
 *  positioned. Past this the count is reported as `N+`. */
const MAX_HITS = 2000;

/** Elements that end a run of text. Inline ones (`code`, `strong`, `a`, the
 *  syntax `span`s) deliberately are NOT here: a match must be free to run
 *  across them, or "the config" stops matching the moment `config` is styled. */
const BLOCK = new Set([
  "ADDRESS", "ARTICLE", "ASIDE", "BLOCKQUOTE", "BR", "DD", "DIV", "DL", "DT",
  "FIGCAPTION", "FIGURE", "FOOTER", "H1", "H2", "H3", "H4", "H5", "H6",
  "HEADER", "HR", "LI", "MAIN", "NAV", "OL", "P", "PRE", "SECTION", "TABLE",
  "TBODY", "TD", "TFOOT", "TH", "THEAD", "TR", "UL",
]);

type Flat = { text: string; nodes: Text[]; starts: number[] };

/** Flatten the rendered body into one searchable string plus the offset of each
 *  text node inside it. Block boundaries become a newline so a match cannot run
 *  from the end of one paragraph into the start of the next. */
function flatten(root: Element): Flat {
  const nodes: Text[] = [];
  const starts: number[] = [];
  let text = "";
  let gap = false;

  const walk = (el: Node) => {
    for (let c = el.firstChild; c; c = c.nextSibling) {
      if (c.nodeType === Node.TEXT_NODE) {
        const d = (c as Text).data;
        if (!d) continue;
        if (gap && text) text += "\n";
        gap = false;
        starts.push(text.length);
        nodes.push(c as Text);
        text += d;
      } else if (c.nodeType === Node.ELEMENT_NODE) {
        const e = c as Element;
        // The code view's gutter is a separate column of line NUMBERS. Its
        // digits are real text nodes, so without this every search for a digit
        // matches the gutter first and the hits look like nonsense.
        if (e.classList.contains("code-gutter")) continue;
        if (e.getAttribute("aria-hidden") === "true") continue;
        const block = BLOCK.has(e.tagName);
        if (block) gap = true;
        walk(e);
        if (block) gap = true;
      }
    }
  };

  walk(root);
  return { text, nodes, starts };
}

function findRanges(root: Element, query: string, caseSensitive: boolean): { ranges: Range[]; capped: boolean } {
  const { text, nodes, starts } = flatten(root);
  if (!text || !query) return { ranges: [], capped: false };

  // Matched with a regex over the ORIGINAL string rather than by lower-casing
  // both sides: case folding is not length-preserving (ẞ → ss, İ → i̇), and one
  // such character anywhere in the file would shift every offset after it and
  // highlight the wrong span. The query is escaped — this is a literal search,
  // the `i` flag is the only thing the regex is for.
  const re = new RegExp(query.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), caseSensitive ? "g" : "gi");

  const locate = (o: number) => {
    let lo = 0, hi = starts.length - 1, k = 0;
    while (lo <= hi) {
      const m = (lo + hi) >> 1;
      if (starts[m] <= o) { k = m; lo = m + 1; } else hi = m - 1;
    }
    return k;
  };

  const ranges: Range[] = [];
  let capped = false;
  for (let m = re.exec(text); m; m = re.exec(text)) {
    if (ranges.length >= MAX_HITS) { capped = true; break; }
    const i = m.index;
    const len = m[0].length;
    const a = locate(i);
    const b = locate(i + len - 1);
    const r = document.createRange();
    r.setStart(nodes[a], i - starts[a]);
    r.setEnd(nodes[b], i + len - starts[b]);
    ranges.push(r);
  }
  return { ranges, capped };
}

/** Paint the hits. The Highlight API touches no DOM, so React's tree is
 *  untouched and a re-render cannot fight the highlight. Where it is missing
 *  (WKWebView older than Safari 17.2) the active hit falls back to the native
 *  selection, which at least shows WHERE the match is. */
function paint(ranges: Range[], active: number) {
  const reg = CSS.highlights;
  if (reg && typeof Highlight === "function") {
    reg.delete("find-hit");
    reg.delete("find-hit-on");
    if (ranges.length) {
      const all = new Highlight(...ranges);
      all.priority = 1;
      reg.set("find-hit", all);
      const one = ranges[active];
      if (one) {
        const hot = new Highlight(one);
        hot.priority = 2;
        reg.set("find-hit-on", hot);
      }
    }
    return;
  }
  const sel = window.getSelection();
  if (!sel) return;
  sel.removeAllRanges();
  if (ranges[active]) sel.addRange(ranges[active]);
}

function unpaint() {
  const reg = CSS.highlights;
  if (reg) { reg.delete("find-hit"); reg.delete("find-hit-on"); }
  // The fallback above paints by SELECTING the active hit, so closing the bar
  // has to drop it too — otherwise the last match stays selected in a viewer
  // that is no longer searching anything.
  else window.getSelection()?.removeAllRanges();
}

/** Bring a hit into view inside whatever actually scrolls. `.scroll` is the
 *  usual answer but the reader and the markdown body are not the same box, so
 *  the container is found by walking up rather than hard-coded. */
function reveal(r: Range | undefined) {
  if (!r) return;
  const rect = r.getBoundingClientRect();
  if (rect.width === 0 && rect.height === 0) return; // hit inside a collapsed box
  let sc = r.startContainer.parentElement;
  while (sc && sc !== document.body) {
    const st = getComputedStyle(sc);
    const scrolls = /(auto|scroll)/.test(st.overflowY) || /(auto|scroll)/.test(st.overflowX);
    if (scrolls && (sc.scrollHeight > sc.clientHeight || sc.scrollWidth > sc.clientWidth)) break;
    sc = sc.parentElement;
  }
  if (!sc || sc === document.body) {
    r.startContainer.parentElement?.scrollIntoView({ block: "center" });
    return;
  }
  const box = sc.getBoundingClientRect();
  const pad = 56;
  if (rect.top < box.top + pad) sc.scrollTop += rect.top - box.top - pad;
  else if (rect.bottom > box.bottom - pad) sc.scrollTop += rect.bottom - box.bottom + pad;
  if (rect.left < box.left + pad) sc.scrollLeft += rect.left - box.left - pad;
  else if (rect.right > box.right - pad) sc.scrollLeft += rect.right - box.right + pad;
}

export type FileFind = {
  query: string;
  setQuery: (v: string) => void;
  caseSensitive: boolean;
  setCaseSensitive: (v: boolean) => void;
  /** 1-based, 0 when there are no hits */
  index: number;
  count: number;
  capped: boolean;
  next: () => void;
  prev: () => void;
};

/**
 * Find inside one rendered file.
 *
 * @param rootRef  the viewer body to search
 * @param open     the bar is showing — closed means no work and no paint
 * @param contentKey  changes whenever what is RENDERED changes (path, wrap,
 *   preview/source, a reload). The collected Ranges point at DOM nodes that a
 *   re-render replaces, so they must be rebuilt, not merely repainted.
 */
export function useFileFind(rootRef: React.RefObject<HTMLElement | null>, open: boolean, contentKey: string): FileFind {
  const [query, setQuery] = useState("");
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [count, setCount] = useState(0);
  const [capped, setCapped] = useState(false);
  const [active, setActive] = useState(0);
  const ranges = useRef<Range[]>([]);
  // Bumped on every recollection, so the paint effect below re-runs even when
  // the hit COUNT happens to be unchanged (edit a query, land on 3 hits again).
  const [gen, setGen] = useState(0);

  useEffect(() => {
    const root = rootRef.current;
    if (!open || !query || !root) {
      ranges.current = [];
      setCount(0);
      setCapped(false);
      setActive(0);
      setGen((v) => v + 1);
      return;
    }
    const found = findRanges(root, query, caseSensitive);
    ranges.current = found.ranges;
    setCount(found.ranges.length);
    setCapped(found.capped);
    setActive(0);
    setGen((v) => v + 1);
  }, [rootRef, open, query, caseSensitive, contentKey]);

  useEffect(() => {
    paint(ranges.current, active);
    reveal(ranges.current[active]);
  }, [gen, active]);

  // Leaving the file, or closing the bar, must clear the paint — a highlight
  // registry entry outlives the component that set it.
  useEffect(() => () => unpaint(), []);
  useEffect(() => { if (!open) unpaint(); }, [open]);

  const step = useCallback((d: number) => {
    const n = ranges.current.length;
    if (n === 0) return;
    setActive((i) => (i + d + n) % n);
  }, []);

  return {
    query, setQuery, caseSensitive, setCaseSensitive,
    index: count === 0 ? 0 : active + 1,
    count, capped,
    next: () => step(1),
    prev: () => step(-1),
  };
}
