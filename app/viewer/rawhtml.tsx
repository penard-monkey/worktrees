// Raw HTML in a document, through an allow-list.
//
// WHY THIS EXISTS AT ALL, since `markdown.tsx` decided the opposite.
//
// The dock shows raw HTML as literal, visible, inert text by decision
// (`2026-08-03-files-viewer` D2: "a doc with an <img> tag shows the tag, which
// is honest"). That rule is right THERE: the dock renders inside the app's own
// `tauri://` webview, where a document's HTML is a code-execution path into the
// app itself. This page is a different place — a separate loopback origin
// behind a per-launch token, under `default-src 'none'; script-src 'self'` with
// no `'unsafe-inline'` — where a `<script>` or an `onerror=` in a document is
// already inert before this file does anything.
//
// Inheriting the dock's rule by reusing its renderer was an accident, and it
// made the viewer unable to read its own documents: `<p align="center"><img
// src="logo.svg"></p>` is how half the READMEs in the world centre a logo, and
// it was showing as a code block. Worse, the system contradicted itself — the
// derived tree's `html_images` already scans raw-HTML `<img src>` and COPIES
// those files in. We were copying the asset and refusing to render the tag that
// referenced it. `docs/proposals/place-docs.md` §4.3 named both branches
// ("must either do the same or sanitise through an allow-list"); this is the
// second one, taken deliberately this time.
//
// ── INLINE <svg> IS NOT RENDERED, AND THAT IS NOT AN OVERSIGHT ──────────────
//
// It was considered and refused. Do not "finish the job" by adding it.
//   - The element surface is large and adversarial — `<use href>` reaches
//     outside the document, `<foreignObject>` reintroduces arbitrary HTML,
//     `<animate>` and friends can set attributes after the fact. An allow-list
//     over SVG is a thing we would own and re-prove forever.
//   - The construct is rare, and the one real document that uses it hand-draws
//     in SVG the same flowchart mermaid renders natively one screen above it.
//     Supporting both means supporting the worse one.
//   - Mermaid already covers the case, under `securityLevel: "strict"`, with a
//     boundary we have already measured.
// An `<svg>` therefore falls off the allow-list like anything else and comes
// out as the literal text it always was. Note that marked splits an inline
// `<svg viewBox=…><rect/></svg>` into THREE separate `html` tokens, so each
// fragment is refused on its own; the namespace check below is what stops the
// first of them from ever being treated as an element.
//
// ── HOW ──────────────────────────────────────────────────────────────────────
//
// ── ONE LIMIT, MEASURED, WORTH KNOWING BEFORE YOU DEBUG IT ─────────────────
//
// marked splits an INLINE `<a href="x">text</a>` into THREE `html` tokens —
// the opening tag, the text, the closing tag — so this sanitiser sees an empty
// `<a>` on its own and falls back to literal text for all three. An anchor
// therefore renders as an anchor only inside a BLOCK html token, where it is a
// child of a kept element (`<p align="center"><a href="./x.md">…</a></p>`,
// which is the form that actually appears in documents). Joining adjacent
// fragments would mean reimplementing marked's inline HTML handling here — a
// second answer to "where does this tag end", which is the mirror this repo
// keeps paying for. The safe behaviour is asserted instead: a `javascript:`
// anchor is shown as text and no `<a>` in the document ever carries one.
//
// `DOMParser` into an INERT document: no browsing context, so nothing in it
// runs, loads or fetches, ever. Then a walk that CONSTRUCTS React elements from
// the allow-list. Nothing is "stripped" from a live tree and nothing is handed
// to `innerHTML` — an element not on the list simply never becomes an element,
// which is the same invariant `markdown.tsx` is built on.
import { createElement, type ReactNode } from "react";
import { safeHref } from "../src/markdown";
import { onLink, renderImage, type DocCtx, type ImgSize } from "./docctx";

/**
 * Deliberately minimal: enough for a centred logo, a badge row and a hand-laid
 * table, and nothing whose meaning is layout or identity. Adding to it is one
 * line, and should be done because a real document needed it — headings and
 * `b`/`i`/`sub`/`sup` are the likeliest next asks.
 */
const TAGS = new Set([
  "p", "div", "span", "a", "img", "br", "strong", "em", "code", "pre",
  "table", "thead", "tbody", "tfoot", "tr", "th", "td", "caption", "colgroup", "col",
]);

/** Elements that carry their meaning with no children. Everything else that
 *  comes out empty at the TOP level of a fragment is an unbalanced opening tag
 *  (`<div align="center">` on its own line) and must fall back to text, or the
 *  matching `</div>` two tokens later renders as text while this one does not. */
const VOID = new Set(["img", "br", "col"]);

/**
 * Attributes, per tag. Three exclusions are load-bearing and are the reason
 * this is a list of what is allowed rather than a list of what is stripped:
 *
 *   `style`  — `style-src 'unsafe-inline'` is REQUIRED by this page (mermaid
 *              injects 4.4 KB of its own <style> plus 73 inline style=
 *              attributes, and a hash beside the keyword voids it). So an
 *              inline style attribute WOULD apply, and a document could park a
 *              `position: fixed` box over the staleness header — the one piece
 *              of chrome whose whole job is to be unforgeable. Measured, see
 *              the session report.
 *   `class`  — the page identifies its own chrome by class. A document that can
 *              set `class` can dress itself as chrome, which is the same
 *              forgery `Chrome.tsx` exists to prevent.
 *   `id`     — collides with the heading-anchor namespace that `#section` links
 *              resolve against.
 *
 * `on*` is not excluded by name anywhere: it is simply not on the list.
 */
const ATTRS: Record<string, Set<string>> = {
  img: new Set(["src", "alt", "title", "width", "height"]),
  a: new Set(["href", "title"]),
  "*": new Set(["align", "title"]),
};

const HTML_NS = "http://www.w3.org/1999/xhtml";
const MAX_DEPTH = 20;
const MAX_NODES = 500;

export type SanitizeResult = { node: ReactNode; dropped: number } | null;

/**
 * `null` means "I will not render this" — the caller shows the literal text,
 * which is `markdown.tsx`'s own behaviour and the honest fallback. A sanitiser
 * that silently produced nothing would delete a document's content.
 */
export function sanitizeHtml(raw: string, ctx: DocCtx): SanitizeResult {
  let doc: Document;
  try {
    doc = new DOMParser().parseFromString(raw, "text/html");
  } catch {
    return null;
  }
  let dropped = 0;
  let budget = MAX_NODES;

  const attrsFor = (tag: string, el: Element): Record<string, unknown> => {
    const allowed = new Set([...(ATTRS[tag] ?? []), ...ATTRS["*"]]);
    const out: Record<string, unknown> = {};
    for (const a of Array.from(el.attributes)) {
      const name = a.name.toLowerCase();
      if (!allowed.has(name)) { dropped++; continue; }
      if (name === "width" || name === "height") {
        // Digits only. `width="100%"` is not a valid value for this attribute,
        // and a unit-bearing string here is someone probing what we accept.
        if (/^\d{1,5}$/.test(a.value)) out[name] = Number(a.value);
        else dropped++;
        continue;
      }
      out[name] = a.value;
    }
    return out;
  };

  const walk = (node: Node, key: string, depth: number): ReactNode => {
    if (budget-- <= 0) return null;
    if (node.nodeType === Node.TEXT_NODE) return node.nodeValue;
    if (node.nodeType !== Node.ELEMENT_NODE) return null; // comments, PIs, doctype
    const el = node as Element;
    // The namespace check, not the tag name, is what refuses `<svg>` and
    // `<math>` and everything inside them — an `<a>` inside an `<svg>` is not
    // the `<a>` on the allow-list, and `<use>` would otherwise be an unknown
    // HTML element rather than the SVG one it really is.
    if (el.namespaceURI !== HTML_NS) { dropped++; return null; }
    const tag = el.localName.toLowerCase();
    if (!TAGS.has(tag) || depth > MAX_DEPTH) { dropped++; return null; }

    if (tag === "img") {
      const src = el.getAttribute("src") ?? "";
      const a = attrsFor("img", el);
      const size: ImgSize = {};
      if (typeof a.width === "number") size.width = a.width;
      if (typeof a.height === "number") size.height = a.height;
      // ONE image policy, shared with the markdown path: same resolution
      // against the document's directory, same refusal of a path that climbs
      // out, same refusal of a remote scheme, same visible reason.
      return <span key={key}>{renderImage(ctx, src, el.getAttribute("alt") ?? "", el.getAttribute("title"), size)}</span>;
    }

    const kids: ReactNode[] = [];
    Array.from(el.childNodes).forEach((c, i) => {
      const r = walk(c, `${key}-${i}`, depth + 1);
      if (r !== null && r !== undefined) kids.push(r);
    });

    if (tag === "a") {
      const rawHref = el.getAttribute("href") ?? "";
      // The SAME `safeHref` the markdown path uses — a scheme refused there is
      // refused here, and refused visibly rather than silently.
      const href = safeHref(rawHref);
      if (!href) {
        dropped++;
        return <span key={key} className="md-link-blocked" title={`blocked link: ${rawHref}`}>{kids}</span>;
      }
      return (
        <a key={key} className="md-link" href={href} title={el.getAttribute("title") ?? rawHref}
           onClick={(e) => { e.preventDefault(); onLink(ctx, href); }}
           onAuxClick={(e) => e.preventDefault()}>
          {kids}
        </a>
      );
    }

    // `createElement` rather than `<Tag>`: the tag is data from a document, and
    // building it as data keeps it that way. `TAGS` is the only thing that
    // decides what may appear here.
    return createElement(tag, { key, ...attrsFor(tag, el) }, kids.length ? kids : null);
  };

  const top: ReactNode[] = [];
  Array.from(doc.body.childNodes).forEach((n, i) => {
    // An unbalanced OPENING tag parses to an empty element, and its matching
    // closing tag two tokens later parses to nothing at all. Dropping the empty
    // one keeps the pair symmetric: both come out as literal text, which is
    // what they were before this file existed. Applied at the TOP level only —
    // an empty `<td>` inside a kept `<tr>` is structure, not an artefact.
    if (n.nodeType === Node.ELEMENT_NODE) {
      const el = n as Element;
      const t = el.localName.toLowerCase();
      if (!VOID.has(t) && el.childNodes.length === 0) return;
    }
    const r = walk(n, `h${i}`, 0);
    if (r !== null && r !== undefined && r !== "") top.push(r);
  });

  const meaningful = top.some((n) => typeof n !== "string" || n.trim() !== "");
  if (!meaningful) return null;
  return { node: top, dropped };
}
