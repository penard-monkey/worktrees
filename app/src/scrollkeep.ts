// Keep a document's viewport where the reader left it.
//
// Two jobs, one hook:
//
//   A CHANGED document keeps what you were reading on screen. The anchor is
//   the first block whose bottom is below the scroller's top edge, remembered
//   with its offset from that edge; after the new content commits, the same
//   block is found again and the box scrolled so it sits at the same offset.
//   A plain scrollTop would hold the pixel and lose the text — an edit above
//   the fold (Claude appending to the plan's top section while you read the
//   bottom) would slide everything you were reading down by its height.
//
//   A REMOUNTED scroller (dock tab flip, place switch and back, Preview →
//   Source → Preview) comes back where it was. The memory is module scope and
//   keyed by the caller — an absolute path plus the view — so it is per place
//   for free, survives every unmount and dies with the webview. It is NOT
//   persisted: ui-state.json is a whole-blob the frontend rewrites, and a
//   reading position is not worth a write on every scroll.
//
// Blocks are matched by text, NOT by DOM identity: Markdown keys its children
// by INDEX, so a paragraph inserted at the top makes React rewrite every node
// below it in place — the element you were reading still exists, holding the
// text of the one above it. The anchor therefore carries the block's text and
// its index, and a restore searches outward from the old index for that text.
//
// `scrollkeep-check.mjs` drives the real file under DOM stubs.
import { useLayoutEffect, useRef, type RefObject } from "react";

export type Anchor = {
  /** scrollTop when captured — the fallback when no block matches */
  top: number;
  /** index of the anchor block among `blocks`, -1 when there were none */
  idx: number;
  /** its text, clipped — what a restore looks for */
  sig: string;
  /** its top edge minus the scroller's, in px (≤ 0 when partly scrolled past) */
  off: number;
};

/** How much of a block's text identifies it. Enough to tell two list items
 *  apart, short enough that a 2000-line fence costs nothing to compare. */
const SIG_LEN = 160;
/** How far either side of the old index a restore looks for the text. */
const SEARCH = 400;
/** Positions remembered. Reading positions are cheap, but unbounded is unbounded. */
const KEEP = 200;

const saved = new Map<string, Anchor>();

export function rememberAnchor(key: string, a: Anchor) {
  saved.delete(key);
  saved.set(key, a);
  if (saved.size > KEEP) saved.delete(saved.keys().next().value as string);
}

export function recallAnchor(key: string): Anchor | null {
  return saved.get(key) ?? null;
}

const sigOf = (el: Element) => (el.textContent ?? "").slice(0, SIG_LEN);

function blocksOf(box: HTMLElement, sel: string | null): Element[] {
  return sel ? Array.from(box.querySelectorAll(sel)) : [];
}

/** Where `box` is scrolled to, as a block and an offset. */
export function captureAnchor(box: HTMLElement, sel: string | null): Anchor {
  const top = box.scrollTop;
  const blocks = blocksOf(box, sel);
  if (!blocks.length) return { top, idx: -1, sig: "", off: 0 };
  const edge = box.getBoundingClientRect().top;
  // Top-level blocks are laid out in order, so their bottoms are monotonic:
  // binary-search the first one still (partly) below the top edge.
  let lo = 0, hi = blocks.length - 1;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (blocks[mid].getBoundingClientRect().bottom > edge) hi = mid;
    else lo = mid + 1;
  }
  const b = blocks[lo];
  return { top, idx: lo, sig: sigOf(b), off: b.getBoundingClientRect().top - edge };
}

/** Scroll `box` so the anchor's block is back at its offset. Falls back to the
 *  old scrollTop when the block's text is gone. Answers whether it got there
 *  EXACTLY — the block found and at its offset, not clamped short. */
export function restoreAnchor(box: HTMLElement, sel: string | null, a: Anchor): boolean {
  const blocks = a.idx >= 0 ? blocksOf(box, sel) : [];
  let hit: Element | null = null;
  if (blocks.length) {
    for (let d = 0; d <= SEARCH && !hit; d++) {
      for (const i of d === 0 ? [a.idx] : [a.idx - d, a.idx + d]) {
        if (i >= 0 && i < blocks.length && sigOf(blocks[i]) === a.sig) { hit = blocks[i]; break; }
      }
    }
  }
  if (!hit) {
    box.scrollTop = a.top;
    return a.idx < 0 && Math.abs(box.scrollTop - a.top) < 1;
  }
  const at = () => hit!.getBoundingClientRect().top - box.getBoundingClientRect().top;
  box.scrollTop += at() - a.off;
  return Math.abs(at() - a.off) < 1;
}

type KeepState = {
  el: HTMLElement | null; key: string | null; content: unknown;
  anchor: Anchor | null;
  /** scrollTop right after our own restore: the scroll event that restore (or
   *  the browser's clamp of it) fires is OURS, and must not become the anchor */
  ours: number | null;
  off: (() => void) | null;
};

/**
 * Keep `ref`'s scroll position across content changes and remounts.
 *
 * `key` names the document+view for the remount memory (null: no memory, the
 * content-change anchoring still applies). `content` is whatever identifies
 * what is DRAWN — the source string — and an identical value is a no-op, so a
 * refresh that re-read the same bytes touches nothing. `blocks` selects the
 * anchorable children (`.md > *` for rendered markdown); null keeps scrollTop
 * only, which is right for a single `<pre>` of code.
 *
 * The anchor moves only when the READER scrolls. A restore that could not get
 * back exactly — the block is missing, or the document is too short to reach
 * it — leaves the anchor where it was, and so does the scroll event the
 * browser fires when it clamps a shrunk document. That is the case that
 * reached a user: a refresh that read the file while an agent was writing it
 * drew a stub, the stub clamped the viewport to the top, the clamp was taken
 * for the reader's position, and the full file came back at the top of the
 * document. Kept, the next read that has the block puts it back.
 *
 * Runs after EVERY commit with no deps on purpose: the scroller can appear,
 * vanish and be replaced by a different element (a loading note, a different
 * view branch) without any prop of ours changing, and comparing three values
 * is cheaper than missing that.
 */
export function useScrollKeep(
  ref: RefObject<HTMLElement | null>,
  key: string | null,
  content: unknown,
  blocks: string | null,
) {
  const st = useRef<KeepState>({ el: null, key: null, content: undefined, anchor: null, ours: null, off: null });
  useLayoutEffect(() => {
    const s = st.current;
    const el = ref.current;
    const restore = (a: Anchor) => {
      const exact = restoreAnchor(el!, blocks, a);
      s.ours = el!.scrollTop;
      s.anchor = exact ? captureAnchor(el!, blocks) : a;
      if (s.key) rememberAnchor(s.key, s.anchor);
    };
    if (el !== s.el || key !== s.key) {
      s.off?.();
      s.off = null;
      const sameEl = el === s.el;
      s.el = el;
      s.key = key;
      s.content = content;
      s.anchor = null;
      s.ours = null;
      if (!el) return;
      const back = key ? recallAnchor(key) : null;
      if (back) restore(back);
      // A different document in the SAME element (the Files tab opening
      // another file) starts at its top. React reuses the box across that
      // switch, and the browser would otherwise keep the last file's offset.
      else if (sameEl && el.scrollTop !== 0) { el.scrollTop = 0; s.ours = 0; }
      const onScroll = () => {
        if (s.ours !== null && Math.abs(el.scrollTop - s.ours) < 1) return;
        s.ours = null;
        s.anchor = captureAnchor(el, blocks);
        if (s.key) rememberAnchor(s.key, s.anchor);
      };
      el.addEventListener("scroll", onScroll, { passive: true });
      s.off = () => el.removeEventListener("scroll", onScroll);
      return;
    }
    if (!el || content === s.content) return;
    s.content = content;
    // Nothing scrolled yet: the top is the right place to stay.
    if (s.anchor) restore(s.anchor);
  });
  useLayoutEffect(() => () => { st.current.off?.(); }, []);
}
