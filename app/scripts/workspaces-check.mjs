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
eq("from All projects onto a workspace that already holds it: nothing", W.projectDrop(P("/x"), { kind: "group", id: "A" }, list), null);
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
// Re-ordering the projects INSIDE one workspace (per workspace: a project in
// several keeps a position in each, and All projects' order is not involved).
const wsOrder = { workspaces: [
  { id: "A", name: "A", projects: ["/p", "/q", "/gone", "/r"].map((root) => ({ root, name: root, registered: root !== "/gone" })) },
  { id: "B", name: "B", projects: [{ root: "/r", name: "r", registered: true }, { root: "/p", name: "p", registered: true }] },
] };
eq("a project in A over A's own group moves, before the row the pointer picked",
  W.projectDrop(P("/r", "A"), { kind: "group", id: "A" }, wsOrder, "/q"), { kind: "ws-move", id: "A", root: "/r", before: "/q" });
eq("…or to the end when no row follows the pointer",
  W.projectDrop(P("/p", "A"), { kind: "group", id: "A" }, wsOrder, null), { kind: "ws-move", id: "A", root: "/p", before: null });
eq("the same project over B, which holds it, is still nothing",
  W.projectDrop(P("/p", "A"), { kind: "group", id: "B" }, wsOrder, "/r"), null);
eq("a project from All projects over a workspace that holds it does not move it",
  W.projectDrop(P("/q"), { kind: "group", id: "A" }, wsOrder, "/p"), null);
eq("over another workspace that lacks it, a drag still adds (the before is ignored)",
  W.projectDrop(P("/q", "A"), { kind: "group", id: "B" }, wsOrder, "/r"), { kind: "ws-add", id: "B", root: "/q" });
eq("the switch still takes it out of A", W.projectDrop(P("/q", "A"), { kind: "switch", view: "projects" }, wsOrder, "/p"), { kind: "ws-remove", id: "A", root: "/q" });
eq("a drag from a workspace the list no longer has: nothing",
  W.projectDrop(P("/q", "Z"), { kind: "group", id: "Z" }, wsOrder, "/p"), null);
eq("moved before: one workspace's list only", W.rootsMoved(wsOrder.workspaces[0], "/r", "/q"), ["/p", "/r", "/q", "/gone"]);
eq("moved to the end", W.rootsMoved(wsOrder.workspaces[0], "/p", null), ["/q", "/gone", "/r", "/p"]);
eq("an unregistered row moves like any other", W.rootsMoved(wsOrder.workspaces[0], "/gone", "/p"), ["/gone", "/p", "/q", "/r"]);
eq("dropped where it already is: the same list", W.rootsMoved(wsOrder.workspaces[0], "/q", "/gone"), ["/p", "/q", "/gone", "/r"]);
eq("moving in A leaves B's list as it was", W.rootsMoved(wsOrder.workspaces[1], "/zzz", null), ["/r", "/p", "/zzz"]);
eq("Move up steps one place", W.rootsStepped(wsOrder.workspaces[0], "/q", -1), ["/q", "/p", "/gone", "/r"]);
eq("Move down steps an unregistered row", W.rootsStepped(wsOrder.workspaces[0], "/gone", 1), ["/p", "/q", "/r", "/gone"]);
eq("Move up on the first is disabled", W.rootsStepped(wsOrder.workspaces[0], "/p", -1), null);
eq("Move down on the last is disabled", W.rootsStepped(wsOrder.workspaces[0], "/r", 1), null);
eq("a project the workspace does not hold cannot step", W.rootsStepped(wsOrder.workspaces[1], "/q", 1), null);
eq("Add to workspace offers the ones that do not hold it",W.addableTo(list, "/x").map((w) => w.id), ["B"]);
eq("the word", W.VIEW_LABEL, { projects: "All projects", workspaces: "Workspaces" });

// A workspace menu's "Add project ▸" and its dialogs' continuation.
const A = list.workspaces[0];
eq("Add project ▸ lists the registered projects the workspace lacks, in order", W.projectsAddable(A, ["/y", "/x", "/z"]), ["/y", "/z"]);
eq("…and nothing when it holds them all", W.projectsAddable(A, ["/x"]), []);
eq("the project a dialog brought is the root the registry gained", W.arrivedRoot(["/x"], ["/x", "/new"], "/elsewhere"), "/new");
eq("an Add existing of a registered project arrives as that project", W.arrivedRoot(["/x", "/y"], ["/x", "/y"], "/y/"), "/y");
eq("…and of a folder inside one, as the deepest root holding it", W.arrivedRoot(["/r", "/r/sub"], ["/r", "/r/sub"], "/r/sub/deep"), "/r/sub");
eq("a prefix is not a parent", W.arrivedRoot(["/r"], ["/r"], "/rx"), null);
eq("nothing new and no folder: nothing arrives", W.arrivedRoot(["/x"], ["/x"], null), null);

// ── 1b. Everything Else: the derived last group ──
// Registered projects in no named workspace. Never stored, never a drop target.
{
  const EE = W.EVERYTHING_ELSE_ID;
  ok("its id is reserved: not a ULID, so core can never mint it", typeof EE === "string" && !/^[0-9A-HJKMNP-TV-Z]{26}$/i.test(EE));
  eq("its name is display text", W.EVERYTHING_ELSE_NAME, "Everything Else");
  const g = W.everythingElse(list, ["/x", "/y", "/z", "/w"]);
  eq("holds the registered projects that are in no named workspace, in All projects' order", g?.projects.map((p) => p.root), ["/z", "/w"]);
  eq("…each as a registered row", g?.projects.every((p) => p.registered), true);
  eq("…under the reserved id, name and gray", [g?.id, g?.name, g?.color], [EE, "Everything Else", "gray"]);
  eq("an unregistered reference in a named workspace never lands in it", W.everythingElse(list, ["/gone"]), null);
  eq("with no named workspace it holds every project", W.everythingElse({ workspaces: [] }, ["/a", "/b"])?.projects.map((p) => p.root), ["/a", "/b"]);
  eq("before the list loads it holds nothing (no flash)", W.everythingElse(null, ["/a"]), null);
  eq("with zero members it is hidden", W.everythingElse(list, ["/x", "/y"]), null);
  eq("no projects at all: hidden", W.everythingElse({ workspaces: [] }, []), null);
  const moved = { workspaces: [{ ...list.workspaces[1], projects: [...list.workspaces[1].projects, { root: "/z", name: "z", registered: true }] }] };
  eq("adding a project to a named workspace takes it out", W.everythingElse(moved, ["/y", "/z"]), null);
  eq("removing it from its last workspace brings it back", W.everythingElse({ workspaces: [{ id: "B", name: "B", projects: [] }] }, ["/y"])?.projects.map((p) => p.root), ["/y"]);
  const mine = { workspaces: [{ id: "01HZZZZZZZZZZZZZZZZZZZZZZZ", name: "Everything Else", projects: [{ root: "/y", name: "y", registered: true }] }] };
  eq("a user's own workspace of that name does not collide", [W.everythingElse(mine, ["/y", "/q"])?.id, W.everythingElse(mine, ["/y", "/q"])?.projects.map((p) => p.root)], [EE, ["/q"]]);
  eq("it is not a drop target: an add onto it is nothing", W.projectDrop(P("/x"), { kind: "group", id: EE }, list), null);
  eq("…nor a move within it", W.projectDrop(P("/z", EE), { kind: "group", id: EE }, list), null);
  eq("a project dragged out of it onto a named workspace adds it there", W.projectDrop(P("/z", EE), { kind: "group", id: "A" }, list), { kind: "ws-add", id: "A", root: "/z" });
  eq("…and onto the All projects switch it is not 'removed' from anything", W.projectDrop(P("/z", EE), { kind: "switch", view: "projects" }, list), null);
  eq("its project fold keys are its own", W.wsProjectKey(EE, "/z"), `${EE}|/z`);
}

// ── 2. rows ──
const ROWS = ["workspaces_list", "workspace_create", "workspace_rename", "workspace_remove", "workspace_set_projects", "workspace_reorder", "workspace_set_color"];
const dispatch = read("../../crates/worktrees-cli/src/serve/dispatch.rs");
const ipc = read("../src/ipc.ts");
const consoleRows = ipc.match(/export const CONSOLE_ROWS[^=]*=\s*\[([\s\S]*?)\];/)?.[1] ?? "";
const mock = read("../src/mock/install.ts");
for (const r of ROWS) {
  if (r === "workspaces_list") {
    // iOS workspaces phase 1 (the owner, 2026-11-25): a device reads it,
    // projected by serve; a remote window still does not ask (CONSOLE_SHAPED).
    ok(`${r} is a device's read, filtered by named_workspaces`, new RegExp(`^\\s+${r}: Engine \\[read, filtered\\(named_workspaces\\)\\] => workspaces::${r}\\(`, "m").test(dispatch));
    ok(`${r} is not in CONSOLE_ROWS`, !consoleRows.includes(`"${r}"`));
    const shaped = ipc.match(/export const CONSOLE_SHAPED[^=]*=\s*\[([\s\S]*?)\];/)?.[1] ?? "";
    ok(`${r} is in CONSOLE_SHAPED, so a remote window does not ask`, shaped.includes(`"${r}"`) && /if \(CONSOLE_SHAPED\.includes\(cmd\)\) return `/.test(ipc));
  } else {
    ok(`${r} is a bare HostConsole row`, new RegExp(`^\\s+${r}: HostConsole => workspaces::${r}\\(`, "m").test(dispatch));
    ok(`${r} is in CONSOLE_ROWS`, consoleRows.includes(`"${r}"`));
  }
  ok(`the mock answers ${r}`, mock.includes(`case "${r}":`));
}
const mcp = read("../../crates/worktrees-cli/src/mcp.rs");
ok("no MCP tool reaches a workspace", !/workspace_(create|rename|remove|set_projects|reorder|set_color)|workspaces_list/.test(mcp));

// ── 2b. the card colors: ONE palette, by name, in five places ──
// Core's COLORS decides what the file may hold, the page's WS_COLORS what the
// menu offers, the mock's what it accepts; tokens.css gives every theme its
// own shade of each, and App.css maps each name onto the card. A name missing
// from any of them is a swatch that draws nothing, or a card that is refused.
{
  const core = read("../../crates/worktrees-core/src/workspaces.rs");
  const app0 = read("../src/App.tsx");
  const rust = (core.match(/pub const COLORS: &\[&str\] = &\[([^\]]*)\]/)?.[1] ?? "").match(/"[a-z]+"/g)?.map((x) => x.slice(1, -1)) ?? [];
  const page = W.WS_COLORS.map((c) => c.id);
  const mockList = (mock.match(/const MOCK_WS_COLORS = \[([^\]]*)\]/)?.[1] ?? "").match(/"[a-z]+"/g)?.map((x) => x.slice(1, -1)) ?? [];
  eq("core's COLORS and the page's WS_COLORS name the same colors, in order", rust, page);
  eq("…and so does the mock", mockList, page);
  ok("…eight of them", page.length === 8);
  const tokens = read("../src/tokens.css");
  const blocks = [...tokens.matchAll(/(^:root|^:root\[data-theme="([a-z-]+)"\])\s*\{([\s\S]*?)^\}/gm)]
    .filter((m) => /--bg-tree:/.test(m[3]));
  ok(`tokens.css has the six theme blocks (${blocks.length})`, blocks.length === 6);
  for (const m of blocks) {
    const missing = page.filter((c) => !new RegExp(`--hue-${c}:\\s*#[0-9a-f]{6};`, "i").test(m[3]));
    ok(`${m[2] ?? "tokyo-night (:root)"} defines its own shade of every color${missing.length ? `: missing ${missing}` : ""}`, missing.length === 0);
  }
  const css = read("../src/App.css");
  const unmapped = page.filter((c) => !css.includes(`.ws-group[data-ws-color="${c}"] { --ws-hue: var(--hue-${c}); }`));
  ok(`App.css maps every color onto the card${unmapped.length ? `: missing ${unmapped}` : ""}`, unmapped.length === 0);
  eq("wsColorOf draws a palette name", W.wsColorOf({ color: "teal" }), "teal");
  eq("…and the accent for none or a name it does not know", [W.wsColorOf({ color: null }), W.wsColorOf({}), W.wsColorOf({ color: "pink" })], [null, null, null]);
  ok("the workspace menu offers Color ▸, with Default and every color", /<SubMenu label="Color" testId="ws-color"/.test(app0) && /WS_COLORS\.map\(\(c\) =>/.test(app0) && /wsSetColor\(id, null\)/.test(app0));
  ok("a card carries its color as data-ws-color", /data-ws-color=\{wsColorOf\(w\) \?\? undefined\}/.test(app0));
}

// ── 3. App wiring ──
const app = read("../src/App.tsx");
for (const [what, re] of [
  ["project menu → Add to workspace", /<SubMenu label="Add to workspace" testId="ctx-ws-add"/],
  ["project menu → Remove from workspace", /data-testid="ctx-ws-remove"/],
  ["project menu → Move up / Move down under a workspace", /data-testid="ctx-ws-proj-up"[\s\S]*data-testid="ctx-ws-proj-down"/],
  ["workspace menu → Move up / Move down", /data-testid="ws-move-up"[\s\S]*data-testid="ws-move-down"/],
  ["workspace menu → Rename", /data-testid="ws-rename"/],
  ["workspace menu → Remove workspace", /data-testid="ws-remove"/],
  ["a New workspace button", /data-testid="ws-new"/],
]) ok(`menu twin: ${what}`, re.test(app));
// The workspace menu's add section: the footer's three acts with this
// workspace as their continuation, never a fork of them.
const wsMenuSrc = app.slice(app.indexOf('{ctx?.kind === "ws" && ('), app.indexOf('data-testid="ws-remove"'));
ok("the workspace menu was found", wsMenuSrc.length > 200);
ok("…offers Add project ▸, beside it on hover", /<SubMenu label="Add project" testId="ws-add-project"/.test(wsMenuSrc));
ok("…New project… through the footer's openNewProject", /openNewProject\(w\.id\)/.test(wsMenuSrc));
ok("…Add existing… through addProject, host-only as in the footer", /hostOnly\("Choosing a folder"\)[\s\S]{0,120}addProject\(w\.id\)/.test(wsMenuSrc));
ok("…Clone from URL… through openClone", /openClone\(w\.id\)/.test(wsMenuSrc));
ok("…and not Hubs (a hub import is not one project)", !/openHubs/.test(wsMenuSrc));
ok("the footer menu still opens them with no workspace", /data-testid="add-new"\s+onClick=\{\(\) => openNewProject\(\)\}/.test(app) && /data-testid="add-clone"\s+onClick=\{\(\) => openClone\(\)\}/.test(app));
const subAt = app.indexOf('<SubMenu label="Add project"');
const subSrc = subAt < 0 ? "" : app.slice(subAt, app.indexOf("</SubMenu>", subAt));
ok("Add project ▸ adds through wsAddProject, from projectsAddable", /const addable = projectsAddable\(/.test(wsMenuSrc) && /addable\.map/.test(subSrc) && /wsAddProject\(id, root\)/.test(subSrc));
ok("…and says so when every project is in it", /disabled>every project is in it</.test(subSrc));
// The owner (2026-11-14): a ▸ row opens BESIDE its menu on hover. Until then
// each replaced the whole menu on click, through a Ctx kind of its own.
ok("no ▸ row replaces its menu with a second one", !/kind: "ws-add", x:|kind: "ws-projects"/.test(app));
for (const [fn, call] of [
  ["confirmAdd", /add_project[\s\S]{0,120}await wsArrive\(ask\.into, before, after, ask\.dir\);\s*commitWs\(after\)/],
  ["createProject", /create_project[\s\S]{0,120}await wsArrive\(into, before, after, null\);\s*commitWs\(after\)/],
  ["runClone", /clone_project[\s\S]{0,200}await wsArrive\(into, ws, done\.workspace, done\.root\);\s*commitWs\(done\.workspace\)/],
  ["initRepo", /init_repo[\s\S]{0,120}await wsArrive\(into, before, after, dir\);\s*commitWs\(after\)/],
]) ok(`${fn} puts the project in its workspace before committing the registry`, call.test(app));
ok("wsArrive writes only through workspace_set_projects, from arrivedRoot", /const wsArrive = [\s\S]{0,900}arrivedRoot\([\s\S]{0,600}wsRun\("workspace_set_projects"/.test(app));
ok("the empty workspace offers Add a project, opening the same menu", /data-testid="ws-empty-add"[^>]*onClick=\{\(e\) => wsMenuAt\(e, w\.id\)\}/.test(app) && /const wsMenuAt = [\s\S]{0,300}kind: "ws", x: r\.left/.test(app));
// The hint's WORDS, comments cut: the Workspaces view lists no project to drag
// (the owner, 2026-11-14: "the projects are hidden behind the toggle"), so a
// drag is only ever offered from All projects, by holding over the switch.
{
  const a = app.indexOf('className="empty small ws-drop-hint"');
  const hint = a < 0 ? "" : app.slice(a, app.indexOf('data-testid="ws-empty-add"', a)).replace(/\{\/\*[\s\S]*?\*\/\}/g, "").replace(/\s+/g, " ");
  ok("the empty workspace's hint was found", hint.length > 40);
  ok("…and never asks for a drag from a view that lists no project", !/drag (a|one|the) project here/i.test(hint) && /All projects/.test(hint) && /hold it over Workspaces/.test(hint));
}
// The header ✕ of a project under a workspace. Sliced from the ProjectNode
// header so a match elsewhere cannot stand in for it.
const hdr = app.slice(app.indexOf("const ProjectNode = "), app.indexOf("{open && pv.ok && ("));
ok("ProjectNode was found", hdr.length > 200);
ok("under a workspace the header ✕ takes it out of the workspace", /inNamed\s*\?[\s\S]{0,500}wsRemoveProject\(/.test(hdr));
ok("…and never unregisters it", !/inWs[^\n]*confirmRemoveProject/.test(hdr));
// Everything Else's wiring: a fixed card, never addressable as a workspace.
{
  const at = app.indexOf('data-testid="ee-group"');
  const card = at < 0 ? "" : app.slice(app.lastIndexOf("everythingElse(wsList", at), app.indexOf("})()}", at));
  ok("Everything Else is drawn from the derived rule, after the named ones", /everythingElse\(wsList, /.test(app) && app.indexOf("everythingElse(wsList") > app.indexOf("{wsGapAt(null)"));
  ok("…its card is no drop target and no reorder row (no data-ws-id / data-drop)", card.length > 100 && !/data-ws-id|data-drop/.test(card));
  ok("…it folds under its reserved id", /toggleWs\(ee\.id\)/.test(card) && /settings\.ws_collapsed\?\.\[ee\.id\]/.test(card));
  const head = app.slice(app.indexOf("function EverythingElseHeader"), app.indexOf("function WsGoneRow") > 0 ? app.indexOf("function WsGoneRow") : undefined);
  ok("…its header has no menu, rename or drag arm", head.length > 100 && !/onMenu|onArm|onRename|ws-menu|onContextMenu|onPointerDown/.test(head.replace(/\/\*\*[\s\S]*?\*\//, "")));
  ok("…its rows get the All projects menu (no ws on the ctx) and no leave-workspace ✕", /inWs && inWs !== EVERYTHING_ELSE_ID \? \{ ws: inWs \}/.test(app) && /const inNamed = inWs && inWs !== EVERYTHING_ELSE_ID/.test(app));
}
ok("the view is the per-viewer nav_view setting", /settings\.nav_view/.test(app) && /updateSettings\(\{\s*nav_view:/.test(app));
ok("a not-registered row is armed for the drag like any other", /<WsGoneRow[\s\S]{0,400}onArm=\{\(e\) => arm\(e, \{ kind: "project", root: p\.root, ws: w\.id \}\)\}/.test(app));
ok("a drop within a workspace writes the whole list through workspace_set_projects", /const wsSetOrder = [\s\S]{0,700}wsRun\("workspace_set_projects", \{ id, roots \}\)/.test(app));
ok("the project drag's order rule is the pure one", /rootsMoved\(w, root, before\)/.test(app) && /rootsStepped\(w, ctx\.root, -1\)/.test(app));
const settings = read("../src/settings.ts");
ok("nav_view is in Settings, defaulting to All projects", /nav_view: "projects" \| "workspaces";/.test(settings) && /nav_view: "projects",/.test(settings));
ok("a junk nav_view loads as All projects", /if \(s\.nav_view !== "workspaces"\) s\.nav_view = "projects";/.test(settings));
const navdrag = read("../src/navdrag.ts");
// The class a drag adds is `dragging-<kind>`; the end removes that same
// class (since phase 3's idea drags reuse the plumbing), so a workspace
// drag's is cleared like every other kind's.
ok("navdrag clears the workspace drag's body class",
  /classList\.add\("dragging", `dragging-\$\{s\.item\.kind\}`\)/.test(navdrag) && /classList\.remove\("dragging", `dragging-\$\{s\.item\.kind\}`\)/.test(navdrag));
ok("still no HTML5 drag-and-drop in the nav", !/draggable=|onDragStart|dataTransfer/.test(app.slice(app.indexOf("const ProjectNode = "), app.indexOf("const DOCK_RAIL"))));

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("\nworkspaces-check: all ok");
