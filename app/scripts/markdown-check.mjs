// Deterministic check of markdown.tsx's text: character references and heading
// ids, against the REAL renderer (bundled, rendered with react-dom/server).
//
//   1. References: the named ones it knows, decimal and hex; one that names no
//      character is U+FFFD and never a throw (`String.fromCodePoint` throws a
//      RangeError past U+10FFFF, inside a render); an unknown name, and one
//      inside a code span, stays as it was written.
//   2. Heading ids: a repeated heading gets `-1`, `-2`, never an id another
//      heading already has, and a document's ids do not depend on how often it
//      was rendered or on what was rendered before it.
//
// Run from the repo root, as CI does:
//   node app/scripts/markdown-check.mjs     exits non-zero on failure
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const esbuild = createRequire(require.resolve("vite"))("esbuild");
const SRC = new URL("../src/", import.meta.url).pathname;

let failed = 0;
const eq = (a, b, msg) => {
  const same = JSON.stringify(a) === JSON.stringify(b);
  if (!same) failed++;
  console.log(`${same ? "ok" : "not ok"} — ${msg}${same ? "" : ` — got ${JSON.stringify(a)}, want ${JSON.stringify(b)}`}`);
};

const out = await esbuild.build({
  stdin: {
    contents: `
      export { Markdown } from "./markdown";
      export { renderToStaticMarkup } from "react-dom/server";
      export { createElement } from "react";
      export * as React from "react";`,
    resolveDir: SRC, loader: "ts",
  },
  bundle: true, write: false, format: "cjs", platform: "node", jsx: "automatic",
  define: { "process.env.NODE_ENV": '"production"' }, logLevel: "silent",
});
const dir = fs.mkdtempSync(path.join(os.tmpdir(), "markdown-check-"));
fs.writeFileSync(path.join(dir, "bundle.cjs"), out.outputFiles[0].text);
const { Markdown, renderToStaticMarkup, createElement, React } = require(path.join(dir, "bundle.cjs"));
fs.rmSync(dir, { recursive: true, force: true });

const html = (src) => renderToStaticMarkup(createElement(Markdown, { src }));
/** The text a one-paragraph document draws (React's own escapes undone). */
const text = (src) => {
  try {
    return html(src).replace(/<[^>]+>/g, "")
      .replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&quot;/g, '"').replace(/&#x27;/g, "'").replace(/&amp;/g, "&");
  } catch (e) { return `threw ${e.name}`; }
};
const ids = (src) => [...html(src).matchAll(/<h\d id="([^"]*)"/g)].map((m) => m[1]);

// ── 1. references ────────────────────────────────────────────────────────────
for (const [src, want, why] of [
  ["a &lt;b&gt; &amp; &quot;c&quot; &#39;d&apos;", `a <b> & "c" 'd'`, "the five XML entities, as before"],
  ["a&nbsp;b", "a b", "&nbsp; is U+00A0, not a plain space"],
  ["&copy;&reg;&trade;&hellip;&mdash;&ndash;", "©®™…—–", "named references"],
  ["&laquo;x&raquo; &middot; 5&deg; &plusmn;1 2&times;3", "«x» · 5° ±1 2×3", "more named references"],
  ["&#169; &#8212;", "© —", "decimal references"],
  ["&#xA9; &#XA9; &#x1F600;", "© © 😀", "hex references, either case of x, astral"],
  ["&amp;copy; &amp;#169;", "&copy; &#169;", "decoded once, never twice"],
  ["&unknown; &constructor; &Copy;", "&unknown; &constructor; &Copy;", "an unknown name is left as written"],
  ["&#1114112; &#x110000; &#xFFFFFF;", "� � �", "past the last code point: U+FFFD, not a RangeError"],
  ["&#0; &#xD800; &#57343;", "� � �", "NUL and a lone surrogate: U+FFFD"],
  ["&#99999999; &#x1234567;", "&#99999999; &#x1234567;", "too many digits is not a reference"],
  ["\\&copy; x", "&copy; x", "an escaped & is literal"],
  ["see `&copy; &#169; &lt;` here", "see &copy; &#169; &lt; here", "a code span is verbatim"],
]) eq(text(src), want, why);

// ── 2. heading ids ───────────────────────────────────────────────────────────
eq(ids("# Overview\n\n## Setup\n\n## Overview\n\n### Overview\n"), ["overview", "setup", "overview-1", "overview-2"], "a repeated heading gets -1, -2");
eq(ids("# A\n\n# A\n\n# A 1\n\n# A\n"), ["a", "a-1", "a-1-1", "a-2"], "a suffix never lands on an id another heading has");
eq(ids("# A 1\n\n# A\n\n# A\n"), ["a-1", "a", "a-2"], "…whichever of the two came first");
eq(ids("# Same\n\n> # Same\n\n- # Same\n"), ["same", "same-1", "same-2"], "headings inside quotes and lists count");
const DOC = "# Overview\n\n# Overview\n";
eq(ids(DOC), ids(DOC), "two renders of one document agree");
// The dock re-renders the SAME document with new props when path links are
// answered: a counter that outlived a render would number it overview-2, -3.
// Driven for real: `Markdown`'s body is called twice on one set of hook slots
// (a dispatcher whose useMemo keeps its value while its deps hold), which is
// what a re-render is and what react-dom/server never does.
{
  const internals = React.__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE;
  const slots = [];
  let at = 0;
  const dispatcher = {
    useMemo: (make, deps) => {
      const i = at++;
      if (!slots[i] || deps.some((d, j) => !Object.is(d, slots[i].deps[j]))) slots[i] = { deps, v: make() };
      return slots[i].v;
    },
    useEffect: () => { at++; },
  };
  const pass = (props) => {
    at = 0;
    internals.H = dispatcher;
    let el;
    try { el = Markdown.type(props); } finally { internals.H = null; }
    return [...renderToStaticMarkup(el).matchAll(/<h\d id="([^"]*)"/g)].map((m) => m[1]);
  };
  eq(pass({ src: DOC }), ["overview", "overview-1"], "one mounted document, first render");
  eq(pass({ src: DOC, onLink: () => {} }), ["overview", "overview-1"], "…re-rendered with new props: the same ids, not -2 and -3");
  eq(slots.length >= 2, true, "…and the re-render really went through the memos");
}

console.log(failed ? `\n${failed} failed` : "\nall ok");
process.exit(failed ? 1 : 0);
