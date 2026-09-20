// Guards the boundary that makes the mermaid exception defensible.
//
//   node app/scripts/viewer-boundary-check.mjs      # exits non-zero on failure
//
// CLAUDE.md admits mermaid as the SECOND exception to the no-UI-libraries rule,
// scoped to the docs viewer bundle and never to `app/src`. That scope is the
// entire argument: mermaid genuinely fails the rule as written — it does not
// parse and hand back data, it renders; it owns layout and theming; it injects
// ~4.4 KB of its own <style> into every SVG; it ships a sanitiser; it is 5.3 MB
// against an app whose whole frontend is plain CSS and a hand-rolled
// highlighter. What makes it admissible is that the app's own bundle never
// gains a byte of it.
//
// "It is isolated" is otherwise a claim about a vite config, and a config can
// be edited, a chunk can be hoisted, and a single `import` in a shared file
// falsifies it silently. Same family as `test/misc.bats`'s version-vs-binary
// assertion: assert the artefact, not the intention.
//
// Four assertions, in the order they would actually break:
//
//   1. Nothing reachable from the APP's entry imports mermaid. This is a real
//      import-graph walk from `src/main.tsx`, not a grep — a grep for the word
//      "mermaid" in `src/` also matches the fence-language table in
//      markdown.tsx and a comment, and a check that cries wolf gets deleted.
//   2. The VIEWER's entry does reach mermaid. Without this the check is
//      vacuous: delete the diagram feature and assertion 1 passes forever while
//      guarding nothing.
//   3. The two builds have separate configs and separate outDirs, so the app's
//      build has no second input that rollup could hoist a shared chunk out of.
//   4. If a built app bundle exists, it contains no mermaid. Conditional on
//      `dist/` being there — and it SAYS which of the two it did, because a
//      gate that quietly skipped looks exactly like a gate that passed.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const APP = fileURLToPath(new URL("..", import.meta.url));
let failed = 0;
const fail = (m) => { failed++; console.log(`not ok — ${m}`); };
const ok = (m) => console.log(`ok — ${m}`);

/** Resolve a relative import to a file on disk, trying the usual extensions. */
function resolveRel(fromFile, spec) {
  const base = path.resolve(path.dirname(fromFile), spec);
  const tries = [base, `${base}.ts`, `${base}.tsx`, `${base}.js`, `${base}.jsx`,
                 path.join(base, "index.ts"), path.join(base, "index.tsx")];
  return tries.find((p) => fs.existsSync(p) && fs.statSync(p).isFile()) ?? null;
}

/**
 * Walk the import graph from `entry`, following RELATIVE imports only, and
 * return the set of bare package specifiers reached. Static `import`,
 * `export … from` and dynamic `import()` all count.
 */
function packagesReachableFrom(entry) {
  const seen = new Set();
  const pkgs = new Set();
  const queue = [entry];
  const RE = /(?:^|\n)\s*(?:import|export)[\s\S]{0,400}?from\s*["']([^"']+)["']|(?:^|[^.\w])import\s*\(\s*["']([^"']+)["']\s*\)|(?:^|\n)\s*import\s+["']([^"']+)["']/g;
  while (queue.length) {
    const file = queue.pop();
    if (!file || seen.has(file)) continue;
    seen.add(file);
    if (!/\.(t|j)sx?$/.test(file)) continue;
    const src = fs.readFileSync(file, "utf8");
    for (const m of src.matchAll(RE)) {
      const spec = (m[1] ?? m[2] ?? m[3] ?? "").split("?")[0];
      if (!spec) continue;
      if (spec.startsWith(".") || spec.startsWith("/")) {
        const next = resolveRel(file, spec);
        if (next) queue.push(next);
      } else {
        pkgs.add(spec.startsWith("@") ? spec.split("/").slice(0, 2).join("/") : spec.split("/")[0]);
      }
    }
  }
  return { pkgs, files: seen };
}

// ── 1. the app's graph is mermaid-free ──────────────────────────────────────
const appEntry = path.join(APP, "src/main.tsx");
if (!fs.existsSync(appEntry)) fail("app/src/main.tsx not found — did the app entry move?");
else {
  const { pkgs, files } = packagesReachableFrom(appEntry);
  if (pkgs.has("mermaid")) fail(`mermaid is reachable from app/src/main.tsx — the boundary is gone (${files.size} files walked)`);
  else ok(`no mermaid anywhere in the app's import graph (${files.size} files, ${pkgs.size} packages)`);
  // The viewer must not be pulled in either — it is what would drag mermaid
  // in one import later.
  const viewerFile = [...files].find((f) => f.includes(`${path.sep}viewer${path.sep}`));
  if (viewerFile) fail(`the app imports a viewer file (${path.relative(APP, viewerFile)}) — the viewer may import the app, never the other way round`);
  else ok("the app imports nothing from app/viewer/");
}

// ── 2. the viewer's graph DOES reach mermaid (no vacuous pass) ──────────────
const viewerEntry = path.join(APP, "viewer/main.tsx");
if (!fs.existsSync(viewerEntry)) fail("app/viewer/main.tsx not found — the check has nothing to guard");
else {
  const { pkgs, files } = packagesReachableFrom(viewerEntry);
  if (!pkgs.has("mermaid")) fail("the viewer does NOT import mermaid — this check would pass vacuously from here on");
  else ok(`the viewer's graph reaches mermaid (${files.size} files)`);
  const usesApp = [...files].some((f) => f.includes(`${path.sep}src${path.sep}markdown.tsx`));
  if (!usesApp) fail("the viewer no longer reuses app/src/markdown.tsx — a second markdown policy is a mirror, and mirrors drift");
  else ok("the viewer reuses app/src/markdown.tsx rather than a second renderer");
}

// ── 2b. the raw-HTML allow-list stays an allow-list ─────────────────────────
// The viewer sanitises raw HTML where the dock shows it as text, and that is
// only defensible while the list is short and three specific things stay off
// it. Each has a reason a future reader will not reconstruct from the diff:
// `style` because `style-src 'unsafe-inline'` is REQUIRED by this page (mermaid
// needs it), so an inline style attribute really applies and a document can
// park a `position: fixed` box over the staleness header — measured; `class`
// because the page identifies its own chrome by class; `id` because it collides
// with the heading-anchor namespace. And `svg` because rendering a document's
// inline SVG means owning an allow-list over `<use>`, `<foreignObject>` and
// `<animate>` forever, for a construct mermaid already covers better.
const rawPath = path.join(APP, "viewer/rawhtml.tsx");
if (!fs.existsSync(rawPath)) fail("app/viewer/rawhtml.tsx not found — the viewer's raw-HTML policy has no home");
else {
  const raw = fs.readFileSync(rawPath, "utf8");
  const listed = (name) => {
    const m = new RegExp(`const ${name} = new Set\\(\\[([\\s\\S]*?)\\]\\)`).exec(raw);
    return m ? [...m[1].matchAll(/"([^"]+)"/g)].map((x) => x[1]) : null;
  };
  const tags = listed("TAGS");
  if (!tags) fail("could not find the TAGS allow-list in rawhtml.tsx — did it get renamed?");
  else {
    const banned = ["script", "style", "iframe", "object", "embed", "svg", "math", "link", "meta", "base", "form", "input"];
    const leaked = banned.filter((t) => tags.includes(t));
    if (leaked.length) fail(`these tags are on the raw-HTML allow-list and must not be: ${leaked.join(", ")}`);
    else ok(`raw-HTML tag allow-list is ${tags.length} tags and admits none of ${banned.join("/")}`);
  }
  const attrBlock = /const ATTRS[\s\S]*?\n};/.exec(raw)?.[0] ?? "";
  const badAttrs = ["style", "class", "id", "srcset", "background", "formaction"].filter((a) =>
    new RegExp(`"${a}"`).test(attrBlock));
  if (badAttrs.length) fail(`these attributes are on the raw-HTML allow-list and must not be: ${badAttrs.join(", ")}`);
  else ok("raw-HTML attribute allow-list admits no style/class/id");
  if (/on[a-z]+"/.test(attrBlock)) fail("an on* handler attribute appears in the raw-HTML attribute allow-list");
  else ok("no on* handler is allow-listed");
  // The namespace check is what refuses <svg>'s CHILDREN — a tag-name list
  // alone would let a standalone `<a>` inside an SVG through.
  // Anchored on the whole statement, not the phrase: `if (false && el.namespaceURI
  // !== HTML_NS)` matched a looser pattern and the guard read as present. A
  // static check can always be outwitted; it should at least cost more than a
  // two-word edit.
  if (!/if \(el\.namespaceURI !== HTML_NS\)/.test(raw)) fail("rawhtml.tsx no longer refuses elements by namespace — an <a> or <use> inside an <svg> is not the HTML element of that name");
  else ok("rawhtml.tsx refuses every element outside the HTML namespace");
  if (!/safeHref/.test(raw)) fail("rawhtml.tsx does not route href through markdown.tsx's safeHref — that is a second link policy");
  else ok("raw-HTML links go through the same safeHref as markdown links");
}

// ── 3. two builds, two outputs ──────────────────────────────────────────────
const appCfg = fs.readFileSync(path.join(APP, "vite.config.ts"), "utf8");
const viewerCfgPath = path.join(APP, "vite.viewer.config.ts");
if (!fs.existsSync(viewerCfgPath)) fail("vite.viewer.config.ts not found — the viewer has no build of its own");
else {
  const viewerCfg = fs.readFileSync(viewerCfgPath, "utf8");
  if (/viewer/.test(appCfg.replace(/\/\*[\s\S]*?\*\/|\/\/.*/g, ""))) {
    fail("the app's vite.config.ts mentions the viewer — a second input is a shared chunk waiting to happen");
  } else ok("the app's vite build has no viewer input");
  if (!/formats:\s*\[\s*["']iife["']\s*\]/.test(viewerCfg)) fail("the viewer build is not IIFE — a module script is blocked outright on a file:// page, and code splitting turns mermaid's await import() into fetches the CSP refuses");
  else ok("the viewer builds a classic IIFE");
  if (!/inlineDynamicImports:\s*true/.test(viewerCfg)) fail("the viewer build does not force inlineDynamicImports");
  else ok("the viewer build forces a single file");
  if (!/process\.env\.NODE_ENV/.test(viewerCfg)) {
    fail("the viewer build does not define process.env.NODE_ENV — vite's LIB MODE leaves it alone, and the bundle silently ships React's development build (measured: +1.9 MB)");
  } else ok("the viewer build pins NODE_ENV (lib mode does not)");
}

// ── 4. the built app bundle, when there is one ──────────────────────────────
const dist = path.join(APP, "dist");
if (!fs.existsSync(dist)) {
  ok("app/dist absent — assertion 1 (the import graph) is the whole check on this run");
} else {
  const js = [];
  const walk = (d) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) walk(p);
      else if (/\.(js|mjs|css)$/.test(e.name)) js.push(p);
    }
  };
  walk(dist);
  // A built bundle older than the source it was built from proves nothing —
  // this run's break-the-boundary test passed assertion 4 on a `dist/` that
  // predated the breaking import. Same family as CLAUDE.md's
  // `[ target/release/worktrees -nt …/store.rs ]`.
  const newest = (dir) => {
    let t = 0;
    const walkSrc = (d) => {
      for (const e of fs.readdirSync(d, { withFileTypes: true })) {
        const q = path.join(d, e.name);
        if (e.isDirectory()) walkSrc(q);
        else t = Math.max(t, fs.statSync(q).mtimeMs);
      }
    };
    walkSrc(dir);
    return t;
  };
  const distAge = Math.min(...js.map((f) => fs.statSync(f).mtimeMs));
  const stale = distAge < newest(path.join(APP, "src"));
  const hits = js.filter((f) => /mermaid/i.test(fs.readFileSync(f, "utf8")));
  if (hits.length) fail(`mermaid appears in the BUILT app bundle: ${hits.map((h) => path.relative(APP, h)).join(", ")}`);
  else if (stale) {
    // NOT a failure: editing app/src without rebuilding is the ordinary state
    // of a working tree, and failing the gate for it would train people to
    // ignore this script. But it is not a pass either — say which one it was.
    ok(`built app bundle contains no mermaid, BUT app/dist is older than app/src, so it cannot see a boundary broken since the build — assertion 1 (the import graph) is the one carrying this run`);
  } else ok(`built app bundle contains no mermaid (${js.length} files scanned, newer than app/src)`);
}

process.exit(failed ? 1 : 0);
