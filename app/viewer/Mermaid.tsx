// The mermaid fence — the one place the second admitted exception to the
// no-UI-libraries rule lives (see CLAUDE.md, "Design tokens").
//
// Three rules, all security, none of them negotiable:
//
// 1. `securityLevel: "strict"`, always. `loose` is the level at which a `click
//    NODE call fn()` directive WRITTEN INSIDE THE DOCUMENT runs a callback.
//    This page's hot path is a repo cloned seconds ago.
// 2. Every AUTHOR-supplied `click` directive is stripped before mermaid sees
//    the source. Strict blocks `call fn()`, but `click NODE "<href>"` is still
//    an author-controlled href, and the proposal's §5.3 rule is that only
//    TOOL-emitted targets may survive. The tool emits exactly one shape, and
//    only that shape survives — see `TOOL_CLICK` below.
// 3. The returned SVG is never assigned to `innerHTML` of a live node. It is
//    parsed into an INERT document, stripped of `<script>` and of every
//    event-handler attribute, and then `importNode`d. `markdown.tsx` exists
//    because a markdown renderer that reaches for an HTML string is one review
//    away from injecting markup; a diagram renderer gets the same treatment.
//
// Mermaid's SVG does NOT survive `DOMParser`'s `image/svg+xml` mode — its
// `foreignObject` labels carry HTML and XML parsing fails with `parsererror`.
// `text/html` is the mode that works, and it is inert either way: a document
// from `DOMParser` has no browsing context, so nothing in it runs or loads.
import { memo, useEffect, useRef, useState } from "react";
import mermaid from "mermaid";

let booted = false;

/** Read a design token off `:root`, so a diagram is not a foreign object. */
function token(name: string, fallback: string): string {
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return v || fallback;
}

function boot(): void {
  if (booted) return;
  booted = true;
  mermaid.initialize({
    startOnLoad: false,
    // NOT configurable, not overridable by a `%%{init}%%` directive in the
    // document: mermaid keeps `securityLevel` on its own secure-keys list.
    securityLevel: "strict",
    // Mermaid's error renderer draws a bomb graphic into the container. We
    // report the failure ourselves and show the source, which is more useful
    // and keeps mermaid from writing anything we did not ask it to write.
    suppressErrorRendering: true,
    theme: "base",
    fontFamily: token("--font-ui", "system-ui, sans-serif"),
    themeVariables: {
      background: token("--bg-panel", "#1a1b26"),
      primaryColor: token("--bg-elev", "#1e2030"),
      primaryTextColor: token("--txt-hi", "#e6eaff"),
      primaryBorderColor: token("--line-strong", "#33384f"),
      secondaryColor: token("--bg-input", "#12131c"),
      tertiaryColor: token("--bg-tree", "#16161e"),
      lineColor: token("--txt-dim", "#8a93bf"),
      textColor: token("--txt", "#c0caf5"),
      mainBkg: token("--bg-elev", "#1e2030"),
      nodeBorder: token("--line-strong", "#33384f"),
      clusterBkg: token("--bg-tree", "#16161e"),
      clusterBorder: token("--line", "#252838"),
      titleColor: token("--txt-hi", "#e6eaff"),
      edgeLabelBackground: token("--bg-panel", "#1a1b26"),
    },
  });
}

/**
 * THE ONE `click` SHAPE THE TOOL EMITS, and the only one that survives.
 *
 * `click <id> href "#/<rel>"`, anchored at both ends, with the id confined to
 * mermaid's own identifier characters and the target confined to a SAME-PAGE
 * FRAGMENT. A fragment is all the drill-down needs: the viewer routes on
 * `location.hash`, so `#/docs/adr/0001.md` is a navigation inside this page and
 * carries no scheme, no host, no port and no token — nothing that could point
 * a reader off the page, and nothing that goes stale when the server is
 * relaunched on a different port.
 *
 * It is also the reason this regex can be this strict. An author cannot forge
 * it into anything dangerous: the anchors leave no room for a second directive
 * or a trailing comment, `[^"]*` cannot close the quote early, and even a
 * hostile `#/…` is only ever a route into this same place's documents — which
 * the reader could already reach from the nav. `svgFromString` below keeps its
 * independent rule that an `href` must start with `#`, so a change here cannot
 * on its own let a remote target through.
 */
const TOOL_CLICK = /^\s*click\s+[A-Za-z0-9_-]+\s+href\s+"#\/[^"]*"\s*$/;

/**
 * Rule 2. A `click` directive is a whole line in mermaid's grammar, so the line
 * is the unit to remove.
 *
 * THIS USED TO STRIP EVERY LINE, INCLUDING THE TOOL'S OWN — which killed the
 * drill-down twice over (`svgFromString` also dropped the resulting `href`,
 * since it was an absolute loopback URL rather than a fragment) and then
 * reported the tool's own directives to the reader as "N author click
 * directives removed". `stripped` is the count of what was ACTUALLY removed, so
 * a diagram carrying nothing but tool links says nothing at all, which is what
 * a note claiming an author was overruled has to mean.
 */
export function stripClickDirectives(src: string): { src: string; stripped: number } {
  let stripped = 0;
  const out = src
    .split("\n")
    .filter((line) => {
      if (TOOL_CLICK.test(line)) return true;
      if (/^\s*click\s+\S/.test(line)) { stripped++; return false; }
      return true;
    })
    .join("\n");
  return { src: out, stripped };
}

/** Rule 3. Inert parse → strip → import. No `innerHTML` on a live node. */
export function svgFromString(svg: string): SVGElement | null {
  const doc = new DOMParser().parseFromString(svg, "text/html");
  const el = doc.body.querySelector("svg");
  if (!el) return null;
  el.querySelectorAll("script").forEach((s) => s.remove());
  // Belt to the `<script>` braces: mermaid should never emit an inline handler,
  // and a future version that did would be a script-execution sink the CSP does
  // not cover (`script-src` does not govern attributes already in the DOM —
  // only the absence of `'unsafe-inline'` does, and we do not want to depend on
  // one directive for this).
  el.querySelectorAll("*").forEach((n) => {
    for (const a of Array.from(n.attributes)) {
      const name = a.name.toLowerCase();
      if (name.startsWith("on")) n.removeAttribute(a.name);
      if ((name === "href" || name === "xlink:href") && !a.value.startsWith("#")) n.removeAttribute(a.name);
    }
  });
  return document.importNode(el, true) as SVGElement;
}

let seq = 0;

export const Mermaid = memo(function Mermaid({ code }: { code: string }) {
  const host = useRef<HTMLDivElement | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [stripped, setStripped] = useState(0);

  useEffect(() => {
    let live = true;
    const el = host.current;
    if (!el) return;
    boot();
    const cleaned = stripClickDirectives(code);
    setStripped(cleaned.stripped);
    const id = `mmd-${seq++}`;
    mermaid
      .render(id, cleaned.src)
      .then(({ svg }) => {
        if (!live || !host.current) return;
        const node = svgFromString(svg);
        if (!node) { setError("mermaid returned no <svg>"); return; }
        host.current.replaceChildren(node);
        setError(null);
      })
      .catch((e: unknown) => {
        if (!live) return;
        setError(String((e as Error)?.message ?? e));
      });
    return () => {
      live = false;
      // mermaid renders into a detached probe element keyed by id; a failed
      // render can leave it behind in <body>.
      document.getElementById(id)?.remove();
      document.getElementById(`d${id}`)?.remove();
    };
  }, [code]);

  return (
    <div className="mermaid-block">
      <div className="mermaid-host" ref={host} data-rendered={error ? "0" : "1"} />
      {stripped > 0 && (
        <div className="mermaid-note">
          {stripped} author <code>click</code> {stripped === 1 ? "directive" : "directives"} removed — a diagram in a
          document may not choose its own link targets
        </div>
      )}
      {error && (
        <>
          <div className="mermaid-note mermaid-err">diagram did not render — {error}</div>
          <pre className="md-rawhtml-block">{code}</pre>
        </>
      )}
    </div>
  );
});
