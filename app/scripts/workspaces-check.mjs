// Checks the named workspaces' RULES — app/src/workspaces.ts — and holds the
// wiring that a drag cannot be checked through:
//
//   1. the pure drop rules (what a project drag means over a group or the
//      switch; reorder and Move up/down);
//   2. serve's six rows are console-only and the page lists them as such
//      (ipc.ts CONSOLE_ROWS), and the mock answers every one;
//   3. App.tsx: every drag has its menu twin, the header ✕ under a workspace
//      takes the project OUT of the workspace (never `confirmRemoveProject`),
//      and the view is the per-viewer `nav_view` setting.
//
//   node app/scripts/workspaces-check.mjs     # exits non-zero on failure
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const at = (p) => fileURLToPath(new URL(p, import.meta.url));
const read = (p) => fs.readFileSync(at(p), "utf8");
const js = (await transformWithEsbuild(read("../src/workspaces.ts"), "workspaces.ts", { loader: "ts", format: "esm" })).code;
const W = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

let failed = 0;
const eq = (name, got, want) => {
  const g = JSON.stringify(got), w = JSON.stringify(want);
  if (g === w) { console.log(`ok — ${name}`); return; }
  failed++;
  console.log(`not ok — ${name}\n     got: ${g}\n    want: ${w}`);
};
const ok = (name, cond) => eq(name, !!cond, true);

// ── 1. rules ──
const list = {
  workspaces: [
    { id: "A", name: "A", projects: [{ root: "/x", name: "x", registered: true }, { root: "/gone", name: null, registered: false }] },
    { id: "B", name: "B", projects: [{ root: "/y", name: "y", registered: true }] },
  ],
};
const P = (root, ws) => ({ kind: "project", root, ...(ws ? { ws } : {}) });
eq("a project from All projects onto a workspace adds it", W.projectDrop(P("/y"), { kind: "group", id: "A" }, list), { kind: "ws-add", id: "A", root: "/y" });
eq("a project in A onto B adds it there too", W.projectDrop(P("/x", "A"), { kind: "group", id: "B" }, list), { kind: "ws-add", id: "B", root: "/x" });
eq("onto the workspace that already holds it: nothing", W.projectDrop(P("/x", "A"), { kind: "group", id: "A" }, list), null);
eq("onto a workspace that is gone: nothing", W.projectDrop(P("/x"), { kind: "group", id: "Z" }, list), null);
eq("a project in A onto the All projects switch takes it out of A", W.projectDrop(P("/x", "A"), { kind: "switch", view: "projects" }, list), { kind: "ws-remove", id: "A", root: "/x" });
eq("a project from All projects onto that switch: nothing", W.projectDrop(P("/x"), { kind: "switch", view: "projects" }, list), null);
eq("onto the Workspaces switch: no drop (it springs)", W.projectDrop(P("/x"), { kind: "switch", view: "workspaces" }, list), null);
eq("over nothing: nothing", W.projectDrop(P("/x"), null, list), null);
eq("a workspace is not a project drop", W.projectDrop({ kind: "workspace", id: "A" }, { kind: "group", id: "B" }, list), null);
eq("springs toward Workspaces from All projects", W.springsTo(P("/x"), { kind: "switch", view: "workspaces" }, "projects"), "workspaces");
eq("never springs toward All projects", W.springsTo(P("/x", "A"), { kind: "switch", view: "projects" }, "workspaces"), null);
eq("a workspace drag never springs", W.springsTo({ kind: "workspace", id: "A" }, { kind: "switch", view: "workspaces" }, "projects"), null);
eq("add appends once", W.rootsWith(list.workspaces[0], "/y"), ["/x", "/gone", "/y"]);
eq("add of a held root is the same list", W.rootsWith(list.workspaces[0], "/x"), ["/x", "/gone"]);
eq("remove keeps the unregistered one", W.rootsWithout(list.workspaces[0], "/x"), ["/gone"]);
eq("reorder before", W.idsMoved(["A", "B", "C"], "C", "A"), ["C", "A", "B"]);
eq("reorder to the end", W.idsMoved(["A", "B", "C"], "A", null), ["B", "C", "A"]);
eq("move up", W.idsStepped(["A", "B", "C"], "B", -1), ["B", "A", "C"]);
eq("move down at the end is disabled", W.idsStepped(["A", "B", "C"], "C", 1), null);
eq("Add to workspace offers the ones that do not hold it", W.addableTo(list, "/x").map((w) => w.id), ["B"]);
eq("the word", W.VIEW_LABEL, { projects: "All projects", workspaces: "Workspaces" });

// ── 2. rows ──
const ROWS = ["workspaces_list", "workspace_create", "workspace_rename", "workspace_remove", "workspace_set_projects", "workspace_reorder"];
const dispatch = read("../../crates/worktrees-cli/src/serve/dispatch.rs");
const ipc = read("../src/ipc.ts");
const consoleRows = ipc.match(/export const CONSOLE_ROWS[^=]*=\s*\[([\s\S]*?)\];/)?.[1] ?? "";
const mock = read("../src/mock/install.ts");
for (const r of ROWS) {
  ok(`${r} is a bare HostConsole row`, new RegExp(`^\\s+${r}: HostConsole => workspaces::${r}\\(`, "m").test(dispatch));
  ok(`${r} is in CONSOLE_ROWS`, consoleRows.includes(`"${r}"`));
  ok(`the mock answers ${r}`, mock.includes(`case "${r}":`));
}
const mcp = read("../../crates/worktrees-cli/src/mcp.rs");
ok("no MCP tool reaches a workspace", !/workspace_(create|rename|remove|set_projects|reorder)|workspaces_list/.test(mcp));

// ── 3. App wiring ──
const app = read("../src/App.tsx");
for (const [what, re] of [
  ["project menu → Add to workspace", /data-testid="ctx-ws-add"/],
  ["project menu → Remove from workspace", /data-testid="ctx-ws-remove"/],
  ["workspace menu → Move up / Move down", /data-testid="ws-move-up"[\s\S]*data-testid="ws-move-down"/],
  ["workspace menu → Rename", /data-testid="ws-rename"/],
  ["workspace menu → Remove workspace", /data-testid="ws-remove"/],
  ["a New workspace button", /data-testid="ws-new"/],
]) ok(`menu twin: ${what}`, re.test(app));
// The header ✕ of a project under a workspace. Sliced from the ProjectNode
// header so a match elsewhere cannot stand in for it.
const hdr = app.slice(app.indexOf("const ProjectNode = "), app.indexOf("{open && pv.ok && ("));
ok("ProjectNode was found", hdr.length > 200);
ok("under a workspace the header ✕ takes it out of the workspace", /inWs\s*\?[\s\S]{0,500}wsRemoveProject\(/.test(hdr));
ok("…and never unregisters it", !/inWs[^\n]*confirmRemoveProject/.test(hdr));
ok("the view is the per-viewer nav_view setting", /settings\.nav_view/.test(app) && /updateSettings\(\{\s*nav_view:/.test(app));
const settings = read("../src/settings.ts");
ok("nav_view is in Settings, defaulting to All projects", /nav_view: "projects" \| "workspaces";/.test(settings) && /nav_view: "projects",/.test(settings));
ok("a junk nav_view loads as All projects", /if \(s\.nav_view !== "workspaces"\) s\.nav_view = "projects";/.test(settings));
const navdrag = read("../src/navdrag.ts");
ok("navdrag clears the workspace drag's body class", /"dragging-workspace"/.test(navdrag));
ok("still no HTML5 drag-and-drop in the nav", !/draggable=|onDragStart|dataTransfer/.test(app.slice(app.indexOf("const ProjectNode = "), app.indexOf("const DOCK_RAIL"))));

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("\nworkspaces-check: all ok");
