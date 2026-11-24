// "Review with init" beside an `undeclared` doctor finding, and the suggestion
// section it opens, against the REAL ProjectSheet.tsx (bundled; the row is
// rendered with react-dom/server, the sheet's wiring is read from the source).
//
// Run from the repo root, as CI does:
//   node app/scripts/initdrift-check.mjs     exits non-zero on failure
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const esbuild = createRequire(require.resolve("vite"))("esbuild");
const SRC = new URL("../src/", import.meta.url).pathname;

let failed = 0;
const check = (cond, msg) => { if (!cond) failed++; console.log(`${cond ? "ok" : "not ok"} — ${msg}`); };

const out = await esbuild.build({
  stdin: {
    contents: `
      export * as S from "./ProjectSheet";
      export { renderToStaticMarkup } from "react-dom/server";
      export { createElement } from "react";`,
    resolveDir: SRC, loader: "ts",
  },
  bundle: true, write: false, format: "cjs", platform: "node", jsx: "automatic",
  loader: { ".css": "empty" },
  define: { "process.env.NODE_ENV": '"production"', "import.meta.env": "{}", __WT_VERSION__: '"0"' }, logLevel: "silent",
});
const dir = fs.mkdtempSync(path.join(os.tmpdir(), "initdrift-check-"));
fs.writeFileSync(path.join(dir, "bundle.cjs"), out.outputFiles[0].text);
const { S, renderToStaticMarkup, createElement } = require(path.join(dir, "bundle.cjs"));
fs.rmSync(dir, { recursive: true, force: true });

// ── the row ──────────────────────────────────────────────────────────────────
if (typeof S.FindingRow !== "function" || typeof S.initSuggestionShown !== "function") {
  check(false, "ProjectSheet.tsx exports FindingRow and initSuggestionShown");
} else {
  const f = (code) => ({ severity: "info", code, place: null, path: ".env", message: `about ${code}` });
  const row = (code, onInit, busy = false) => renderToStaticMarkup(createElement(S.FindingRow, { f: f(code), onInit, busy }));
  check(/<button[^>]*>Review with init<\/button>/.test(row("undeclared", () => {})), "an undeclared finding has Review with init");
  for (const c of ["not-linked", "copy-stale", "shadowed", "no-slot", "unknown-key"])
    check(!/<button/.test(row(c, () => {})) && row(c, () => {}).includes(`about ${c}`), `a ${c} finding has no such button`);
  check(!/<button/.test(row("undeclared", null)), "no button when init has nothing to suggest");
  check(/<button[^>]* disabled=""/.test(row("undeclared", () => {}, true)), "the button is disabled while the sheet runs something");

  // ── the section ────────────────────────────────────────────────────────────
  const s = (qualifies) => ({ qualifies });
  const cfg = (exists) => ({ exists });
  const shown = S.initSuggestionShown;
  check(shown(s(true), cfg(false), false) === true, "no config yet: the suggestion shows, as before");
  check(shown(s(true), null, false) === true, "config not read yet: the suggestion shows, as before");
  check(shown(s(true), cfg(true), false) === false, "a configured project: not until it is asked for");
  check(shown(s(true), cfg(true), true) === true, "a configured project, opened from a finding: it shows");
  check(shown(s(false), cfg(true), true) === false && shown(null, cfg(true), true) === false, "nothing to suggest: never, opened or not");
}

// ── the wiring ───────────────────────────────────────────────────────────────
const src = fs.readFileSync(path.join(SRC, "ProjectSheet.tsx"), "utf8");
check(/<FindingRow key=\{i\} f=\{f\} busy=\{busy\} onInit=\{suggestion\?\.qualifies \? reviewInit : null\} \/>/.test(src), "every finding is drawn by FindingRow, with the button only when init suggests something");
check(/const canSuggest = initSuggestionShown\(suggestion, cfg, initForDrift\);/.test(src), "the section's gate is initSuggestionShown");
const opened = /useEffect\(\(\) => \{\s*if \(!open\) return;[\s\S]*?\}, \[open, refresh, root\]\);/.exec(src)?.[0] ?? "";
check(/setInitForDrift\(false\);/.test(opened), "opening the sheet, or another project in it, closes the section again");
check(/const reviewInit = [\s\S]{0,300}setInitForDrift\(true\);[\s\S]{0,300}data-section="init"[\s\S]{0,120}scrollIntoView/.test(src) && /data-section="init"/.test(src.slice(src.indexOf("{canSuggest && suggestion && ("))), "the button brings the section into view: it is at the bottom of the sheet");
// init never writes over a config, so the section must not promise a Write
// button there, and must name what does help: the fragment to append.
const section = src.slice(src.indexOf("{canSuggest && suggestion && ("));
check(/\{cfg\?\.exists \? \([\s\S]{0,400}worktrees init --diff[\s\S]{0,400}\) : \([\s\S]{0,400}until you press Write/.test(section), "on a configured project the section points at `worktrees init --diff`, not at a Write button it does not have");

console.log(failed ? `\n${failed} failed` : "\nall ok");
process.exit(failed ? 1 : 0);
