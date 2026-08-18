// Checks `relPath` in app/src/filekind.ts — what the Files tab's right-click
// "Copy relative path" puts on the clipboard. No DOM, no React: filekind.ts is
// pure and dependency-free on purpose, so it can be imported straight.
//
//   node app/scripts/relpath-check.mjs        # exits non-zero on failure
//
// Same idiom as dnd-check.mjs: transform the TS, import it as a data: URL,
// assert. A wrong answer here is silent — the clipboard takes whatever it is
// handed, and a relative path computed from the wrong base still LOOKS like a
// valid one right up until it is pasted into the wrong repo.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const SRC = fileURLToPath(new URL("../src/filekind.ts", import.meta.url));
const js = (await transformWithEsbuild(fs.readFileSync(SRC, "utf8"), "filekind.ts", {
  loader: "ts", format: "esm",
})).code;
const F = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

let failed = 0;
const eq = (name, got, want) => {
  const g = JSON.stringify(got), w = JSON.stringify(want);
  if (g === w) return;
  failed++;
  console.log(`not ok — ${name}\n     got: ${g}\n    want: ${w}`);
};

const ROOT = "/Users/dev/workspace/proj/.worktrees/feature";

eq("a direct child loses the root", F.relPath(ROOT, `${ROOT}/README.md`), "README.md");
eq("a nested file keeps its subpath", F.relPath(ROOT, `${ROOT}/app/src/App.tsx`), "app/src/App.tsx");
eq("a directory row is a path like any other", F.relPath(ROOT, `${ROOT}/app/src`), "app/src");
eq("a dot component survives", F.relPath(ROOT, `${ROOT}/.github/workflows/ci.yml`), ".github/workflows/ci.yml");
eq("a trailing slash on the root is ignored", F.relPath(`${ROOT}/`, `${ROOT}/README.md`), "README.md");
eq("the root itself is .", F.relPath(ROOT, ROOT), ".");
eq("the root with a trailing slash is still .", F.relPath(`${ROOT}/`, ROOT), ".");

// The separator is part of the prefix test. Without it a SIBLING whose name
// merely starts with the root's would be reported as living inside it — and
// `/…/feature` sitting next to `/…/feature-2` is the normal shape of this
// app's worktree directory, not an exotic case.
eq("a sibling with the root as a name prefix stays absolute",
  F.relPath(ROOT, `${ROOT}-2/README.md`), `${ROOT}-2/README.md`);
// Rows come back from the backend already resolved, so a place reached through
// a symlink hands us a path under a different prefix entirely.
eq("a path outside the root stays absolute",
  F.relPath(ROOT, "/private/var/other/README.md"), "/private/var/other/README.md");
eq("root / is not doubled", F.relPath("/", "/etc/hosts"), "etc/hosts");

console.log(failed ? `\n${failed} check(s) failed` : "all relPath checks passed");
process.exit(failed ? 1 : 0);
