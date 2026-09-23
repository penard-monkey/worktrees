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
import { Component, memo, useLayoutEffect, useMemo, useRef, type CSSProperties, type ReactNode } from "react";
import { marked } from "marked";
import { Markdown } from "../src/markdown";
import { Mermaid } from "./Mermaid";
import type { Block } from "./contract";
import { onLink, renderImage, type DocCtx } from "./docctx";
import { sanitizeHtml } from "./rawhtml";
import type { DocView } from "./zoom";

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

/**
 * Every link/image DEFINITION in the document, as markdown, for re-appending.
 *
 * REFERENCE LINKS SPAN BLOCKS AND THE RENDERER DOES NOT. `[ci][badge]` in one
 * block and `[badge]: https://…` in another are one document to a reader and
 * two independent lexes here — each block gets its own `marked` run with its
 * own (empty) definition table, so the reference resolves against nothing and
 * renders as the literal text `[ci][badge]`. It worked in the dock, which lexes
 * the whole file at once, and broke in the browser, on exactly the documents
 * that use it most: a README's badge row and footnote-style links.
 *
 * `marked.lexer()` hangs the table off the token list as `.links`, so this is
 * the lexer's own answer rather than a second definition-line parser — which
 * would have to know about fenced code, indented code and `\[`, and would be
 * wrong about one of them. The lines are rebuilt from the parsed values, which
 * is why no escaping is needed: `href` and `title` come back already unwrapped.
 */
type Defs = Record<string, { href: string; title?: string | null }>;

export function collectDefs(blocks: Block[]): string {
  const out: string[] = [];
  const seen = new Set<string>();
  for (const b of blocks) {
    let links: Defs | undefined;
    try {
      links = (marked.lexer(b.md, { gfm: true, breaks: false }) as { links?: Defs }).links;
    } catch {
      continue;
    }
    for (const [tag, v] of Object.entries(links ?? {})) {
      // First definition wins, which is markdown's own rule for a repeated
      // label — so appending these to a block that already carries one of them
      // changes nothing.
      if (seen.has(tag) || !v || typeof v.href !== "string") continue;
      seen.add(tag);
      // `[x]: <a b>` parses to the href `a b`; re-emitting it bare would end
      // the destination at the space and swallow the rest as a title. The
      // angle form is how markdown spells a destination containing one, and
      // it is always legal, so it is used whenever the round trip is not
      // obviously safe.
      const href = /[\s<>]/.test(v.href) ? `<${v.href.replace(/[<>]/g, "")}>` : v.href;
      const title = typeof v.title === "string" && v.title ? ` "${v.title.replace(/"/g, "&quot;")}"` : "";
      out.push(`[${tag}]: ${href}${title}`);
    }
  }
  return out.join("\n");
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
  { md, defs, id, dataKey, ctx }: { md: string; defs: string; id: string; dataKey: string; ctx: DocCtx },
) {
  // ON THE ORIGINAL `md`, NEVER ON `src`. A block that is nothing but a
  // ```mermaid fence is one token; append a definition line and it is two, so
  // the diagram test stops matching and every diagram in the place renders as
  // a code fence. The defs only ever reach the markdown path.
  const diagram = useMemo(() => mermaidSource(md), [md]);
  const src = useMemo(() => (defs && diagram === null ? `${md}\n\n${defs}` : md), [md, defs, diagram]);
  return (
    // `data-block-id` is the SERVER's id, carried so a probe can compare the two
    // notions of identity; `data-key` is the content key React actually
    // reconciles on, and what the scroll anchor above looks the block up by.
    <div className="mdb" data-block-id={id} data-key={dataKey}>
      {diagram !== null ? (
        <Mermaid code={diagram} />
      ) : (
        <Markdown
          src={src}
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
 * One broken block must not take the page.
 *
 * The same rule the terminal's search addon taught this repo (CLAUDE.md: "losing
 * a search is survivable, losing the terminal is not"). A document is arbitrary
 * text from a worktree, run through a lexer, a sanitiser and — for a fence — a
 * 5 MB diagram library; a throw anywhere in there unmounts the whole viewer and
 * leaves a blank page with the poll still running behind it. Bounded per block,
 * the reader loses one paragraph and can still read, navigate and see which
 * block failed.
 *
 * A class, because that is the only form an error boundary has in React.
 * `key`ed by the block's content key at the call site, so a block that changes
 * gets a fresh boundary and a fixed document recovers on its own.
 */
class BlockBoundary extends Component<{ children: ReactNode }, { err: string | null }> {
  state: { err: string | null } = { err: null };
  static getDerivedStateFromError(e: unknown) {
    return { err: String((e as Error)?.message ?? e) };
  }
  render() {
    if (this.state.err === null) return this.props.children;
    return <div className="mdb-err">this block could not be rendered — {this.state.err}</div>;
  }
}

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
export type Anchor = { key: string; top: number; height: number; sizing: boolean };

/**
 * How far to scroll so the anchored block lands where it was — the one piece of
 * arithmetic in this file, pulled out because it is the piece that can be
 * WRONG, and because nothing above can test it (the anchor needs real layout).
 *
 * The two cases are genuinely different, and conflating them is the bug this
 * function exists to name:
 *
 *   A DOCUMENT UPDATE changes a block's CONTENT. Pinning the block's `top` is
 *   exactly right: whatever grew, grew below the line being read, and the
 *   reader must not move.
 *
 *   A SIZE CHANGE rescales the block ITSELF. Pinning its top then pushes the
 *   line you were reading down by however far into the block you already were —
 *   measured on a real document, a block 951px tall with 702px of it above the
 *   viewport drifted 102px on a single 10% step, and would drift ~520px going
 *   from 100% to 175%, which is most of a screen. What has to be held constant
 *   is the FRACTION of the block above the fold, not the pixel count.
 *
 * So the scaling is applied only when the anchor straddles the viewport top
 * (`top < 0`) and only on a size change. A block that starts below the fold has
 * no fraction above it to preserve, and holding its `top` is already correct:
 * everything above it grew too, and the gap should stay the size it looks.
 */
export function anchorDelta(s: Anchor, nextTop: number, nextHeight: number): number {
  const want = s.sizing && s.top < 0 && s.height > 0 ? s.top * (nextHeight / s.height) : s.top;
  return nextTop - want;
}

function useBlockScrollAnchor(host: React.RefObject<HTMLElement | null>, view: DocView) {
  const snap = useRef<Anchor | null>(null);
  // The view the DOM on screen was built with — NOT the one being rendered.
  // Written in the layout effect below, i.e. once the DOM actually reflects it,
  // so a render that bails out before committing cannot make a later size
  // change look like an update.
  const shown = useRef<DocView>(view);
  const sizing = shown.current.zoom !== view.zoom || shown.current.wide !== view.wide;

  if (host.current && window.scrollY > 0) {
    snap.current = null;
    for (const child of Array.from(host.current.children)) {
      const r = child.getBoundingClientRect();
      if (r.bottom > 0) {
        const key = child.getAttribute("data-key");
        if (key) snap.current = { key, top: r.top, height: r.height, sizing };
        break;
      }
    }
  }

  useLayoutEffect(() => {
    shown.current = view;
    const s = snap.current;
    snap.current = null;
    if (!s || !host.current) return;
    // A document switch replaces every key, so the anchor is simply absent and
    // nothing is adjusted — which is right: a new document starts at its top.
    const el = host.current.querySelector(`[data-key="${CSS.escape(s.key)}"]`);
    if (!el) return;
    const r = el.getBoundingClientRect();
    const delta = anchorDelta(s, r.top, r.height);
    if (delta !== 0) window.scrollBy(0, delta);
  });
}

export function DocBody({ blocks, ctx, view }: { blocks: Block[]; ctx: DocCtx; view: DocView }) {
  // KEYED ON THE ORIGINAL MARKDOWN, deliberately, even though what is rendered
  // is `md + defs`. Keys are how React decides what to keep; folding the defs
  // into them would re-key — and therefore replace the DOM subtree of — every
  // block in the document the moment someone edits one unrelated `[x]: url`
  // line, which is precisely the selection-and-scroll churn this file's whole
  // design exists to prevent.
  const keys = useMemo(() => blockKeys(blocks), [blocks]);
  const defs = useMemo(() => collectDefs(blocks), [blocks]);
  const host = useRef<HTMLElement | null>(null);
  useBlockScrollAnchor(host, view);
  // THE READING SIZE RIDES ON `.doc`, which also carries `.md` — App.css reads
  // it through `var(--md-zoom, 1)`, so an element that never gets one simply
  // renders at 100%. The unit is a bare ratio, as `FilesPane` writes it.
  //
  // Applying it HERE rather than on a wrapper is load-bearing in one more way:
  // this is the element `useBlockScrollAnchor` watches. A zoom or measure
  // change re-renders this component, so the snapshot above is taken against
  // the OLD layout and the `useLayoutEffect` below runs against the new one —
  // which means the block you were reading keeps its place on screen across a
  // size change, for free, by the machinery that already holds it across a
  // document update.
  //
  // `data-wide` is the measure: `.md`'s 78ch is a reading measure, and turning
  // it off is the one thing zoom alone cannot do (78ch grows WITH the text, so
  // a bigger size fills more of the window but never all of it).
  return (
    <article
      className="doc md"
      ref={host}
      data-wide={view.wide ? "1" : "0"}
      style={{ "--md-zoom": String(view.zoom / 100) } as CSSProperties}
    >
      {blocks.map((b, i) => (
        <BlockBoundary key={keys[i]}>
          <BlockView dataKey={keys[i]} md={b.md} defs={defs} id={b.id} ctx={ctx} />
        </BlockBoundary>
      ))}
    </article>
  );
}
