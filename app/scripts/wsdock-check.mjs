// The dock in a workspace's view (wsdock.ts): only its Terminal is live, its
// tab is always the Terminal, and its shells are the backend's ordinary dock
// shells under the workspace scope, `~|<id>`.
//
// Three halves:
//  1. The rule itself, run from the real wsdock.ts: in a workspace's view the
//     rail offers the Terminal alone, and every other tab keeps "select a
//     place first"; a place's view is unchanged; "window too narrow" still
//     wins for a live tab.
//  2. The scope is ONE constant on both sides: wsdock.ts's `WORKSPACE_SCOPE`
//     is worktrees-api's `term::WORKSPACE_SCOPE`. A drift would send every
//     workspace shell to the backend as a place that does not exist — or,
//     worse, as one that does.
//  3. App.tsx hands the workspace's strip that scope and the workspace's own
//     keys, and asks the rule for every rail button.
//
//   node app/scripts/wsdock-check.mjs   (from the repo root) exits non-zero on failure
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const at = (rel) => fileURLToPath(new URL(rel, import.meta.url));
let failed = 0;
const ok = (m) => console.log(`ok — ${m}`);
const fail = (m) => { failed++; console.log(`not ok — ${m}`); };
const check = (c, m) => (c ? ok(m) : fail(m));
const eq = (m, got, want) => check(JSON.stringify(got) === JSON.stringify(want), `${m}${JSON.stringify(got) === JSON.stringify(want) ? "" : ` — got ${JSON.stringify(got)}, want ${JSON.stringify(want)}`}`);

// ── 1. the rule ────────────────────────────────────────────────────────────
const src = fs.readFileSync(at("../src/wsdock.ts"), "utf8");
check(!/^import (?!type)/m.test(src), "wsdock.ts has no runtime import (it loads as a data: module)");
const W = await import("data:text/javascript;base64," + Buffer.from(
  (await transformWithEsbuild(src, "wsdock.ts", { loader: "ts", format: "esm" })).code).toString("base64"));

const TABS = ["files", "terminal", "docs", "plan", "prs", "workspace"];
const why = (where) => Object.fromEntries(TABS.map((k) => [k, W.dockWhyFor(k, where)]));
const PLACE_FIRST = "select a place first";
eq("a workspace's view: only the Terminal is live",
  why({ place: false, workspace: true, fits: true }),
  { files: PLACE_FIRST, terminal: null, docs: PLACE_FIRST, plan: PLACE_FIRST, prs: PLACE_FIRST, workspace: PLACE_FIRST });
eq("…and too narrow, the Terminal says so",
  W.dockWhyFor("terminal", { place: false, workspace: true, fits: false }), "window too narrow");
eq("…while the others still say why they are a place's",
  W.dockWhyFor("files", { place: false, workspace: true, fits: false }), PLACE_FIRST);
eq("a place's view: every tab live", why({ place: true, workspace: false, fits: true }),
  Object.fromEntries(TABS.map((k) => [k, null])));
eq("nothing selected: nothing live", why({ place: false, workspace: false, fits: true }),
  Object.fromEntries(TABS.map((k) => [k, PLACE_FIRST])));
check(W.dockEligibleAt({ place: false, workspace: true }) && W.dockEligibleAt({ place: true, workspace: false })
  && !W.dockEligibleAt({ place: false, workspace: false }), "the dock may show for a place or a workspace, not for nothing");
for (const tab of TABS)
  eq(`a workspace's dock shows its Terminal whatever dock_tab says (${tab})`, W.dockTabAt({ place: false, workspace: true }, tab), "terminal");
eq("a place's dock shows its own tab", W.dockTabAt({ place: true, workspace: false }, "docs"), "docs");
eq("a workspace's key", W.wsDockKey("01WS"), "~|01WS");
eq("…read back", W.wsOfDockKey("~|01WS"), "01WS");
eq("a place's key is no workspace's", W.wsOfDockKey("/Users/me/repo|(main)"), null);
eq("…not even one whose repo begins with ~ and more", W.wsOfDockKey("~x|y"), null);

// ── 2. one constant, both sides ────────────────────────────────────────────
const term = fs.readFileSync(at("../../crates/worktrees-api/src/term.rs"), "utf8");
const rust = [...term.matchAll(/^pub const WORKSPACE_SCOPE: &str = "([^"]*)";/gm)].map((m) => m[1]);
eq("term.rs declares WORKSPACE_SCOPE once", rust.length, 1);
eq("wsdock.ts's WORKSPACE_SCOPE is term.rs's", W.WORKSPACE_SCOPE, rust[0]);
check(!W.WORKSPACE_SCOPE.startsWith("/"), "…and no absolute path, so no project's repo can be it");

// ── 3. App.tsx wires it ────────────────────────────────────────────────────
const app = fs.readFileSync(at("../src/App.tsx"), "utf8");
const strip = app.match(/<TerminalTabs key=\{wsDockKey\(wsDock\)\}[\s\S]*?\/>/);
check(!!strip, "App.tsx renders a TerminalTabs keyed on the workspace's dock key");
if (strip) {
  const s = strip[0];
  check(s.includes("repo={WORKSPACE_SCOPE} slug={wsDock}"), "…with the workspace scope and the workspace's id");
  check(s.includes("sessionUp"), "…always up: a workspace's shells need no session");
  for (const field of ["term_tab_names", "term_tabs", "term_tab_active"])
    check(s.includes(`(settings.${field} ?? {})[wsDockKey(wsDock)]`), `…reading ${field} under the workspace's key`);
  check(!s.includes("onOpenPath"), "…and no links: there is no Files tab to open one in");
}
const rails = [...app.matchAll(/<DockRailGroup [^>]*why=\{dockWhy\}/g)].length;
eq("both rail hosts ask the rule for each button", rails, 2);
check(/const dockWhy = \(key: Settings\["dock_tab"\]\) => dockWhyFor\(key, /.test(app), "dockWhy is dockWhyFor's");
check(/const shownTab = dockTabAt\(/.test(app), "shownTab is dockTabAt's");
check(/dropPanels\(\(k\) => k === wsDockKey\(id\)\)/.test(app), "removing a workspace drops its dock records");

if (failed) {
  console.log(`\n${failed} failure(s)`);
  process.exit(1);
}
console.log("ok — a workspace's dock is its Terminal alone, under ~|<id>, one scope on both sides");
