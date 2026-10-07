// Deterministic check for file-path links in rendered markdown (the Files
// tab's preview and the Plan tab), against the REAL source:
//
//   1. Candidate rules (mdpaths.ts): a code span is a candidate only when it is
//      a path end to end (a bare name allowed); prose only with a `/`.
//   2. Extraction BY THE RENDERER: markdown.tsx is bundled and rendered with a
//      recording `answers` map — every key it looks up is a candidate it drew.
//      Fences, raw HTML and an authored link's text must not be mined; an
//      answered candidate becomes a `.md-path` link; and with nothing answered
//      the markup is byte-identical to a render with no path links at all
//      (pending answers cause no layout shift).
//   3. Batching (`resolveDocPaths`): candidates go to the backend in batches
//      of `lib.rs`'s `TERM_PATHS_MAX` — never dropped past it — and a failed
//      batch costs only its own answers.
//   4. The resolver is the SHARED one: `resolve_doc_path` in lib.rs decides
//      every answer through `resolve_term_path` and has no file test of its
//      own, and the mock harness tracks the command.
//
// Run from the repo root, as CI does:
//   node app/scripts/mdpaths-check.mjs     exits non-zero on failure
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const esbuild = createRequire(require.resolve("vite"))("esbuild");
const SRC = new URL("../src/", import.meta.url).pathname;

let failed = 0;
const fail = (msg) => { failed++; console.log(`not ok — ${msg}`); };
const ok = (msg) => console.log(`ok — ${msg}`);
const check = (cond, msg) => (cond ? ok(msg) : fail(msg));
const eq = (a, b, msg) => check(JSON.stringify(a) === JSON.stringify(b), `${msg}${JSON.stringify(a) === JSON.stringify(b) ? "" : ` — got ${JSON.stringify(a)}, want ${JSON.stringify(b)}`}`);

// One CJS bundle of the real modules (and React's server renderer), so the
// relative imports and the bare ones all resolve without a DOM or a loader.
const out = await esbuild.build({
  stdin: {
    contents: `
      export { Markdown } from "./markdown";
      export * as P from "./mdpaths";
      export { renderToStaticMarkup } from "react-dom/server";
      export { createElement } from "react";`,
    resolveDir: SRC, loader: "ts",
  },
  bundle: true, write: false, format: "cjs", platform: "node", jsx: "automatic",
  define: { "process.env.NODE_ENV": '"production"' }, logLevel: "silent",
});
const tmp = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "mdpaths-check-")), "bundle.cjs");
fs.writeFileSync(tmp, out.outputFiles[0].text);
const { Markdown, P, renderToStaticMarkup, createElement } = require(tmp);
fs.rmSync(path.dirname(tmp), { recursive: true, force: true });

// ── 1. candidate rules ───────────────────────────────────────────────────────
const code = (s) => P.codeSpanHit(s);
const C = [
  ["app/src/App.tsx", "app/src/App.tsx"],
  ["App.tsx", "App.tsx"],
  ["quota.rs", "quota.rs"],
  [".gitignore", ".gitignore"],
  ["~/notes/todo.md", "~/notes/todo.md"],
  ["./bin/worktrees", "./bin/worktrees"],
  ["src/a.ts:42:7", "src/a.ts", 42, 7],
  ["src/a.ts:42-50", "src/a.ts", 42],
  [" src/a.ts ", "src/a.ts"],
  // not paths, or not ONLY a path
  ["npm run build", null],
  ["foo.bar()", null],
  ["x.ts, y.ts", null],
  ["v0.33.0", null],
  ["e.g.", null],
  ["https://x.dev/a/b.ts", null],
  ["Makefile", null],
  ["--flag", null],
  ["", null],
];
for (const [s, p, line, col] of C) {
  const h = code(s);
  if (p == null) check(h == null, `code span \`${s}\` is not a candidate`);
  else eq(h && [h.path, h.line, h.col], [p, line, col], `code span \`${s}\` → ${p}${line ? `:${line}` : ""}`);
}
const prose = (s) => P.textHits(s).map((h) => h.path);
eq(prose("see app/src/App.tsx and README.md for it"), ["app/src/App.tsx"], "prose: a path with a `/` only — a bare name is a word");
eq(prose("edit src/a.ts:42, then ~/x/y.md."), ["src/a.ts", "~/x/y.md"], "prose: position suffixes and trailing punctuation");
eq(prose("client/server and/or 1.2.3"), [], "prose: no false positives from slashes in words");

// ── 2. extraction by the renderer ────────────────────────────────────────────
const DOC = [
  "# Plan for app/src/heading.ts",
  "",
  "Edit app/src/App.tsx:12 and `crates/core/src/lib.rs`; `lib.rs` is ambiguous, `ghost.ts` is nothing.",
  "Bare README.md in prose is not asked. **bold/in/strong.rs** is.",
  "",
  "- item names docs/list.md",
  "",
  "| a | b |",
  "| - | - |",
  "| `table/cell.ts` | x |",
  "",
  "[link/text/inside.md](https://example.com) and [`code/in/link.ts`](x.md)",
  "",
  "```sh",
  "cat fence/never/mined.rs",
  "```",
  "",
  "<div>html/never/mined.rs</div>",
  "",
].join("\n");
const WANT = ["app/src/heading.ts", "app/src/App.tsx", "crates/core/src/lib.rs", "lib.rs", "ghost.ts", "bold/in/strong.rs", "docs/list.md", "table/cell.ts"];

const recorder = (answer) => {
  const seen = [];
  return { seen, map: { get: (k) => { if (!seen.includes(k)) seen.push(k); return answer(k); } } };
};
const render = (pathLinks) => renderToStaticMarkup(createElement(Markdown, { src: DOC, pathLinks }));
const noop = () => {};

const asked = recorder(() => undefined);
const pending = render({ answers: asked.map, want: noop, open: noop });
eq([...asked.seen].sort(), [...WANT].sort(), "the renderer asks about exactly the candidates it draws");
check(!asked.seen.some((k) => /mined|inside|in\/link/.test(k)), "fences, raw HTML and authored link text are not mined");
check(pending === render(undefined), "nothing answered (pending or absent): markup identical to no path links");
const none = recorder(() => []);
check(render({ answers: none.map, want: noop, open: noop }) === render(undefined), "answered EMPTY: still plain text");

const hit = (rel) => [{ path: `/p/${rel}`, rel }];
const answered = recorder((k) => (k === "lib.rs" ? [...hit("a/lib.rs"), ...hit("b/lib.rs")] : k === "ghost.ts" ? [] : hit(k)));
const html = render({ answers: answered.map, want: noop, open: noop });
const links = [...html.matchAll(/<span role="link" tabindex="0" class="md-link md-path" title="([^"]*)">(.*?)<\/span>/g)].map((m) => [m[1], m[2]]);
eq(links.map(([, label]) => label.replace(/<[^>]+>/g, "")),
  ["app/src/heading.ts", "app/src/App.tsx:12", "crates/core/src/lib.rs", "lib.rs", "bold/in/strong.rs", "docs/list.md", "table/cell.ts"],
  "every answered candidate is a link, with its :line in the clickable span");
eq(links.find(([, l]) => l.includes("App.tsx"))?.[0], "app/src/App.tsx:12", "hover title = the resolved repo-relative path (and line)");
check(/^2 files are named lib\.rs/.test(links.find(([, l]) => l.includes(">lib.rs<"))?.[0] ?? ""), "an ambiguous name says it will ask");
check(html.includes('<span role="link" tabindex="0" class="md-link md-path" title="crates/core/src/lib.rs"><code class="md-code-inline">'), "a code span keeps its <code> inside the link");
check(!/href=/.test(html.replace(/<a [^>]*>/g, "")), "a detected path carries no href");

// ── 3. batching ──────────────────────────────────────────────────────────────
const MAX = Number(/const TERM_PATHS_MAX: usize = (\d+);/.exec(fs.readFileSync(new URL("../src-tauri/src/lib.rs", import.meta.url), "utf8"))?.[1]);
eq(P.DOC_PATHS_BATCH, MAX, "DOC_PATHS_BATCH matches lib.rs's TERM_PATHS_MAX");
const many = Array.from({ length: 150 }, (_, i) => `d/f${i}.md`);
{
  const calls = [];
  const inv = async (cmd, args) => { calls.push([cmd, args]); return args.paths.map((p) => [{ path: `/r/${p}`, rel: p }]); };
  const got = await P.resolveDocPaths(inv, { root: "/r", doc: "/r/x.md", generation: 3 }, [...many, ...many.slice(0, 10)]);
  eq(calls.map(([c, a]) => [c, a.paths.length]), [["resolve_doc_paths", 64], ["resolve_doc_paths", 64], ["resolve_doc_paths", 22]], "150 candidates (+10 dupes) → 64/64/22, one call per batch");
  check(got.size === 150 && got.get("d/f149.md")?.[0].rel === "d/f149.md", "nothing past the first batch is dropped");
  check(calls.every(([, a]) => a.root === "/r" && a.doc === "/r/x.md" && a.generation === 3), "every batch carries root, doc and generation");
}
{
  const calls = [];
  let n = 0;
  const inv = async (cmd, args) => {
    calls.push(cmd);
    if (cmd === "resolve_doc_paths" && n++ === 1) throw new Error("boom");
    return cmd === "resolve_doc_paths" ? args.paths.map(() => []) : null;
  };
  const got = await P.resolveDocPaths(inv, { root: "/r", doc: null, generation: 0 }, many);
  check(got.size === 86 && !got.has("d/f64.md") && got.has("d/f128.md"), "a failed batch leaves only ITS candidates unanswered");
  check(calls.includes("log_event"), "…and is logged, not swallowed");
}

// ── 4. the shared resolver, and the mock ─────────────────────────────────────
const lib = fs.readFileSync(new URL("../src-tauri/src/lib.rs", import.meta.url), "utf8").split("\n#[cfg(test)]")[0];
const body = (name) => {
  const at = lib.indexOf(`fn ${name}(`);
  if (at < 0) return "";
  const end = lib.indexOf("\n}\n", at);
  return lib.slice(at, end);
};
const rdp = body("resolve_doc_path");
check(rdp.includes("resolve_term_path(raw,") && rdp.includes("resolve_term_path(rel,"), "resolve_doc_path answers a path AND every index entry through resolve_term_path");
check(!/canonicalize\(&p|is_file\(\)|under_roots/.test(rdp), "resolve_doc_path has no file/containment test of its own (no second resolver)");
const cmd = body("resolve_doc_paths");
check(/\.take\(TERM_PATHS_MAX\)/.test(cmd) && cmd.includes("resolve_doc_path(p,"), "resolve_doc_paths caps at TERM_PATHS_MAX and calls resolve_doc_path");
check(/^\s*resolve_doc_paths,$/m.test(lib), "resolve_doc_paths is registered in the invoke handler");
const mock = fs.readFileSync(new URL("../src/mock/install.ts", import.meta.url), "utf8");
check(mock.includes('case "resolve_doc_paths":'), "the mock harness tracks resolve_doc_paths");

console.log(failed ? `\n${failed} failed` : "\nall ok");
process.exit(failed ? 1 : 0);
