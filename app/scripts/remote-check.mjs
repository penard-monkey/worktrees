// Checks the remote-page RULE — app/src/remote.ts — the one place that decides
// which page on the origin remote a project or a place opens at. The backend
// (`remote_url` in lib.rs) only hands over an https base; if this rule drifts,
// the topbar link, the place menu and the project menu all drift together, and
// nothing else would say so. No DOM, no React: remote.ts is pure on purpose.
//
//   node app/scripts/remote-check.mjs         # exits non-zero on failure
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const SRC = fileURLToPath(new URL("../src/remote.ts", import.meta.url));
const js = (await transformWithEsbuild(fs.readFileSync(SRC, "utf8"), "remote.ts", {
  loader: "ts", format: "esm",
})).code;
const R = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

let failed = 0;
const eq = (name, got, want) => {
  const g = JSON.stringify(got), w = JSON.stringify(want);
  if (g === w) return;
  failed++;
  console.log(`not ok — ${name}\n     got: ${g}\n    want: ${w}`);
};

const GH = "https://github.com/acme/repo";
const GL = "https://gitlab.internal/group/repo";

// ── a project (no place) is the repo home, whatever the host ──────────────
eq("project → home (github)", R.remoteWebUrl(GH, null), GH);
eq("project → home (other host)", R.remoteWebUrl(GL, null), GL);

// ── a place on github: its branch's tree ONLY when the branch is on origin ──
eq("pushed branch → /tree/", R.remoteWebUrl(GH, { upstream: "origin/feat/x" }), `${GH}/tree/feat/x`);
eq("main → /tree/main", R.remoteWebUrl(GH, { upstream: "origin/main" }), `${GH}/tree/main`);
// The rule the old `github_url` got wrong: an unpushed branch has no page on
// the remote, so it goes home rather than to a 404.
eq("no upstream → home", R.remoteWebUrl(GH, { upstream: null }), GH);
eq("undefined upstream → home", R.remoteWebUrl(GH, {}), GH);
// An upstream on some OTHER remote is not a page on origin.
eq("foreign remote → home", R.remoteWebUrl(GH, { upstream: "fork/feat/x" }), GH);
// A branch name with a character that is not URL-safe survives the trip.
eq("branch with '#' is encoded", R.remoteWebUrl(GH, { upstream: "origin/fix#12" }), `${GH}/tree/fix%2312`);
// Slashes in a branch stay slashes: GitHub resolves `tree/a/b/c` itself.
eq("slashes kept", R.remoteWebUrl(GH, { upstream: "origin/a/b/c" }), `${GH}/tree/a/b/c`);

// ── other hosts spell the tree path differently: always the home ──────────
eq("gitlab + upstream → home", R.remoteWebUrl(GL, { upstream: "origin/feat/x" }), GL);

// ── menu label and tooltip ────────────────────────────────────────────────
eq("label github", R.remoteHostLabel(GH), "GitHub");
eq("label other host", R.remoteHostLabel(GL), "gitlab.internal");
eq("label before the read", R.remoteHostLabel(undefined), "GitHub");
eq("label no remote", R.remoteHostLabel(null), "GitHub");
eq("title drops the scheme", R.remoteTitle(`${GH}/tree/main`), "github.com/acme/repo/tree/main");

if (failed) { console.log(`\n${failed} failure(s)`); process.exit(1); }
console.log("ok — remote.ts: page rule, host label, tooltip");
