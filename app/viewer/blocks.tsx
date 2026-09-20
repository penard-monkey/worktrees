// The block list — the half of this viewer that makes a live document readable.
//
// THE REQUIREMENT (docs-transport §3.3, measured): an update must not disturb
// the reader. Swapping the document's HTML measured `sel 70 -> 0` — it destroys
// a selection exactly as a reload does. Replacing only the blocks whose
// markdown changed measured `scroll 400 -> 400, sel 55 -> 55`, for an edit at
// the top of the document AND for an append at the end.
//
// HOW IT IS OBTAINED, and why the key is not the server's `id`.
//
// React reconciles a keyed list by key. Give each block its own key and its own
// `memo`'d component and an update touches exactly the blocks whose key or
// props changed — every other DOM node is not re-created, not re-ordered and
// not written to, so the selection anchored inside it and the scroll offset
// above it both survive. That much is standard.
//
// The key is a hash of the block's MARKDOWN, not the `id` the contract carries,
// and that choice is the whole robustness of this file. The contract's example
// ids are `b0`, `b1` — positional. If they are positional, then inserting a
// paragraph at the top of a document shifts every id below it: `b3`'s markdown
// arrives under the key `b4`, every block after the insertion point re-renders,
// and the reader's selection dies three paragraphs from anything that changed.
// Content keys are immune to that: an insertion changes one key and leaves the
// rest identical, so React inserts one node and moves nothing.
//
// What content keying costs: editing a block replaces its subtree instead of
// patching it in place, so a selection INSIDE the block being edited is lost.
// That is the block the writer is typing in, not the one the reader is reading,
// and the measurement the brief asks for is explicitly "change a block
// ELSEWHERE". The server's `id` is still carried, onto `data-block-id`, so the
// two can be compared from a probe.
import { memo, useLayoutEffect, useMemo, useRef } from "react";
import { marked } from "marked";
import { Markdown } from "../src/markdown";
import { Mermaid } from "./Mermaid";
import type { Block } from "./contract";
import { onLink, renderImage, type DocCtx } from "./docctx";
import { sanitizeHtml } from "./rawhtml";

export type { DocCtx };

/** FNV-1a. A key, not a digest — it only has to distinguish, cheaply. */
function hash(s: string): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return (h >>> 0).toString(36);
}

/**
 * Stable per-content keys. Two identical blocks (`---`, or a repeated heading)
 * must not collide, so equal content is disambiguated by its occurrence count
 * — which is itself stable as long as the earlier occurrences do not move.
 */
export function blockKeys(blocks: Block[]): string[] {
  const seen = new Map<string, number>();
  return blocks.map((b) => {
    const base = `${b.md.length}.${hash(b.md)}`;
    const n = seen.get(base) ?? 0;
    seen.set(base, n + 1);
    return n === 0 ? base : `${base}#${n}`;
  });
}

/** A block that is nothing but a ```mermaid fence, via the same lexer the
 *  renderer uses — so "is this a diagram?" has exactly one answer. */
function mermaidSource(md: string): string | null {
  let tokens;
  try {
    tokens = marked.lexer(md, { gfm: true, breaks: false });
  } catch {
    return null;
  }
  const real = tokens.filter((t) => t.type !== "space");
  if (real.length !== 1 || real[0].type !== "code") return null;
  const code = real[0] as { lang?: string; text: string };
  return (code.lang ?? "").split(/\s+/)[0].toLowerCase() === "mermaid" ? code.text : null;
}

const BlockView = memo(function BlockView(
  { md, id, dataKey, ctx }: { md: string; id: string; dataKey: string; ctx: DocCtx },
) {
  const diagram = useMemo(() => mermaidSource(md), [md]);
  return (
    // `data-block-id` is the SERVER's id, carried so a probe can compare the two
    // notions of identity; `data-key` is the content key React actually
    // reconciles on, and what the scroll anchor above looks the block up by.
    <div className="mdb" data-block-id={id} data-key={dataKey}>
      {diagram !== null ? (
        <Mermaid code={diagram} />
      ) : (
        <Markdown
          src={md}
          className="mdb-md"
          renderImage={(s, alt, t) => renderImage(ctx, s, alt, t)}
          onLink={(h) => onLink(ctx, h)}
          renderHtml={(raw) => {
            // `null` hands the token back to `markdown.tsx`, which shows it as
            // the literal text it has always shown. Every refusal in
            // `rawhtml.tsx` — an `<svg>`, a `<script>`, an unbalanced opening
            // tag — arrives here as `null`, so nothing a document wrote is ever
            // silently deleted.
            const r = sanitizeHtml(raw, ctx);
            if (!r) return null;
            return (
              <span className="md-rawhtml-render">
                {r.node}
                {r.dropped > 0 && (
                  <span className="md-rawhtml-dropped" title="an allow-list decides what raw HTML in a document may render as">
                    {" "}[{r.dropped} not allowed]
                  </span>
                )}
              </span>
            );
          }}
        />
      )}
    </div>
  );
});

/**
 * Scroll anchoring, by hand.
 *
 * Block patching keeps a selection alive and keeps the scroll OFFSET identical,
 * which is enough when the change is below the reader or in place. It is not
 * enough when a block is INSERTED ABOVE the viewport: the offset is unchanged
 * and the document under it is 45 px longer, so the reader is looking at
 * different words. Measured before this existed: `scroll 400 -> 445`.
 *
 * MEASURED, and the measurement is not what I expected. Both engines already
 * anchor this natively today — Chromium via CSS scroll anchoring, and WebKit
 * (Playwright 2359) by some mechanism `overflow-anchor: none` does not switch
 * off. With this function disabled, the read position held in both. So it is a
 * BACKSTOP and it is currently redundant, which is worth writing down rather
 * than discovering later by deleting it on a hunch.
 *
 * What proves it is not dead code: with the browser's own anchoring turned off
 * (`html, body, .doc { overflow-anchor: none }`) and this disabled, the read
 * position moved 1557 → 1602 px, exactly the 45 px the inserted block added.
 * With it enabled, under the same conditions, 1557 → 1557 in Chromium and
 * 1554 → 1554 in WebKit. It works, and it is the only thing that does when the
 * engine will not.
 *
 * Whoever wants to delete this: rerun that experiment first. The default
 * configuration cannot tell the two cases apart.
 *
 * Reading layout in the RENDER body is deliberate and is the only place the
 * pre-update geometry still exists — React runs the component body before it
 * mutates the DOM, and `useLayoutEffect` runs after. It costs one forced layout
 * per real update, which is at most one per second.
 *
 * `scrollY === 0` is exempt, matching what the CSS scroll-anchoring spec does:
 * a reader parked at the top of a document should SEE something inserted at the
 * top, not be scrolled past it.
 */
function useBlockScrollAnchor(host: React.RefObject<HTMLElement | null>) {
  const snap = useRef<{ key: string; top: number } | null>(null);

  if (host.current && window.scrollY > 0) {
    snap.current = null;
    for (const child of Array.from(host.current.children)) {
      const r = child.getBoundingClientRect();
      if (r.bottom > 0) {
        const key = child.getAttribute("data-key");
        if (key) snap.current = { key, top: r.top };
        break;
      }
    }
  }

  useLayoutEffect(() => {
    const s = snap.current;
    snap.current = null;
    if (!s || !host.current) return;
    // A document switch replaces every key, so the anchor is simply absent and
    // nothing is adjusted — which is right: a new document starts at its top.
    const el = host.current.querySelector(`[data-key="${CSS.escape(s.key)}"]`);
    if (!el) return;
    const delta = el.getBoundingClientRect().top - s.top;
    if (delta !== 0) window.scrollBy(0, delta);
  });
}

export function DocBody({ blocks, ctx }: { blocks: Block[]; ctx: DocCtx }) {
  const keys = useMemo(() => blockKeys(blocks), [blocks]);
  const host = useRef<HTMLElement | null>(null);
  useBlockScrollAnchor(host);
  return (
    <article className="doc md" ref={host}>
      {blocks.map((b, i) => (
        <BlockView key={keys[i]} dataKey={keys[i]} md={b.md} id={b.id} ctx={ctx} />
      ))}
    </article>
  );
}
