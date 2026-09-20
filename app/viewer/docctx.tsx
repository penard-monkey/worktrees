// The per-document policy: how a reference in a document becomes something on
// screen. ONE implementation, shared by the markdown path (`blocks.tsx`) and
// the raw-HTML path (`rawhtml.tsx`), because two answers to "what may this
// image point at?" is exactly the drift this repo keeps paying for.
//
// It lives in its own module rather than in `blocks.tsx` so the sanitiser can
// import it without a cycle.
import type { ReactNode } from "react";
import { assetUrl, resolveRel } from "./contract";

export type DocCtx = {
  /** API base — `http://127.0.0.1:<port>/<token>/p/<place>/`. */
  base: URL;
  /** Directory of the document being rendered; relative refs resolve here. */
  dir: string;
  /** A relative `.md` link was followed. */
  onDoc: (path: string) => void;
  /** An in-document `#anchor` was followed. */
  onAnchor: (slug: string) => void;
};

/** `width="160"` from a raw-HTML `<img>`. Digits only — see `rawhtml.tsx`. */
export type ImgSize = { width?: number; height?: number };

/**
 * Images. `markdown.tsx` deliberately builds no `<img>` of its own — it
 * delegates here (`MarkdownProps.renderImage`, "the viewer loads them off
 * disk"). The dock's implementation reads bytes over a Tauri command; this one
 * points at the server's `asset/` route.
 *
 * Refusals are SHOWN, not swallowed. A reader who cannot see a diagram needs to
 * know whether it is missing or refused.
 */
export function renderImage(
  ctx: DocCtx, src: string, alt: string, title: string | null, size?: ImgSize,
): ReactNode {
  const s = src.trim();
  if (s.startsWith("data:image/")) {
    // Allowed by `img-src data:` and inert: an SVG inside <img> is in the
    // spec's secure static mode — no script, no external references.
    return <span className="md-img"><img src={s} alt={alt} title={title ?? undefined} {...size} /></span>;
  }
  if (/^[a-z][a-z0-9+.-]*:/i.test(s) || s.startsWith("//")) {
    // Remote. The CSP (`img-src 'self' data:`) would refuse the load anyway;
    // not emitting it means the page never even tries, which is the difference
    // between "no outbound network by policy" and "no outbound network".
    return <span className="md-img-note">[remote image not loaded: {s}]</span>;
  }
  const rel = resolveRel(ctx.dir, s);
  if (!rel) return <span className="md-img-note">[image refused — path escapes the place root: {s}]</span>;
  // NOT `loading="lazy"`. Chrome defers a lazy image past a viewport-distance
  // threshold (~2500px), and a deferred image has made no request at all —
  // `complete: false`, `naturalWidth: 0`, `currentSrc` empty, and nothing in
  // the network log to explain it. That reads exactly like a hung server, and
  // it cost a round of debugging pointed at the wrong component: a control
  // using `new Image()` "proved" the URL was fine, when all it proved was that
  // a constructed image is never lazy.
  //
  // Laziness buys nothing here and the arithmetic is not close. These are
  // local files already copied into the derived tree, extension-allow-listed,
  // capped at 10 MB each and 64 MB per place, served over loopback. What it
  // costs is every reader that does not scroll: a print, a screenshot, a
  // find-in-page landing below the fold, and any engine whose threshold
  // differs from the one this was measured against.
  return (
    <span className="md-img">
      <img src={assetUrl(ctx.base, rel)} alt={alt} title={title ?? undefined} {...size} />
    </span>
  );
}

export const DOC_EXT = /\.(md|markdown)(#|$)/i;

export function onLink(ctx: DocCtx, href: string): void {
  if (href.startsWith("#")) { ctx.onAnchor(href.slice(1)); return; }
  if (/^https?:/i.test(href)) {
    // The token lives in this page's URL, so an external navigation that
    // carried a `Referer` would hand a stranger a capability to every document
    // in this place. The shell's `<meta name="referrer" content="no-referrer">`
    // is what prevents that; `noreferrer` here is the second lock, and
    // `noopener` keeps the opened tab from reaching back through `window.opener`.
    window.open(href, "_blank", "noopener,noreferrer");
    return;
  }
  if (!DOC_EXT.test(href)) {
    // A relative link to something that is not a document. Following it would
    // be a TOP-LEVEL navigation to `asset/<rel>`, and a top-level SVG document
    // executes script — which is a property of how the server labels the
    // response, not something this page can settle. Until assets are known to
    // carry `X-Content-Type-Options: nosniff` and a `default-src 'none'` CSP of
    // their own, the link is inert and says so (see `.doc a.md-link` in
    // viewer.css, which marks it visually).
    return;
  }
  const [p, anchor] = href.split("#");
  const rel = resolveRel(ctx.dir, p);
  if (!rel) return;
  ctx.onDoc(anchor ? `${rel}#${anchor}` : rel);
}
