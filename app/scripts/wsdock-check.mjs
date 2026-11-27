// Workspace shell scope and compatibility with saved Terminal-only dock settings.
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

eq("a new workspace starts with its document panel selected", W.workspaceTerminalSelected(), false);
eq("a workspace from the Terminal-only build keeps Terminal", W.workspaceTerminalSelected({ dock_open: true, dock_tab: "terminal" }), true);
eq("the workspace choice wins over the compatibility tab", W.workspaceTerminalSelected({ dock_open: true, dock_tab: "terminal", workspace_terminal: false }), false);
eq("the workspace can still choose Terminal", W.workspaceTerminalSelected({ dock_open: true, workspace_terminal: true }), true);
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
check(/panelsFor\(settings, wsDock \? wsDockKey\(wsDock\) : selKey\)/.test(app), "workspace and place docks share panelsFor");
check(/<WorkspaceDockTerminal key=\{wsDock\} shown=\{dockShown && wsTerminal\}>/.test(app), "Terminal attaches on its first visible visit");
check(/dropPanels\(\(k\) => k === wsDockKey\(id\)\)/.test(app), "removing a workspace drops its dock records");

if (failed) {
  console.log(`\n${failed} failure(s)`);
  process.exit(1);
}
console.log("ok — workspace shells use ~|<id>, one scope on both sides");
