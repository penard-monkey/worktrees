/** Named workspaces (idea board phase 2) — the frontend's RULES, pure, checked
 *  by `scripts/workspaces-check.mjs`. The store is core's
 *  (`worktrees_core::workspaces`, `workspaces.json`); the rows are serve's six
 *  console rows (`workspaces_list`, `workspace_*`), every one answering with
 *  the whole list.
 *
 *  A workspace is a named list of references to registered projects. It never
 *  touches a project: taking one out of a workspace is never unregistering it,
 *  and a project that leaves the registry stays listed as "not registered"
 *  until the user drops it. A workspace shows each of its projects and then
 *  every place of that project, exactly as All projects does (the owner,
 *  2026-11-08: a place belongs to its project; a workspace references
 *  projects). No imports: the check loads this file as a `data:` module. */

export type NavView = "projects" | "workspaces";

/** The word (choice C6). Today's nav — every registered project on this host,
 *  `list_workspace` in code — is "All projects" in the UI; "workspace" means a
 *  named one. One table, so the word flips in one place. */
export const VIEW_LABEL: Record<NavView, string> = {
  projects: "All projects",
  workspaces: "Workspaces",
};

export const isNavView = (v: unknown): v is NavView => v === "projects" || v === "workspaces";

export type WsProject = { root: string; name: string | null; registered: boolean };
/** `plan` and `ideas` (phase 4) are ids as stored; `workspace_overview`
 *  resolves them. */
export type WsDef = { id: string; name: string; projects: WsProject[]; plan?: string | null; ideas?: string[] };
export type WsList = { workspaces: WsDef[] };

/** What a nav drag in the workspace view can carry. A project row under a
 *  workspace carries the workspace it is in (`ws`); one from All projects
 *  carries none. */
export type WsItem =
  | { kind: "project"; root: string; ws?: string }
  | { kind: "workspace"; id: string };

/** What the pointer is over, as data: a workspace group (its header or any
 *  row inside it), or one segment of the view switch. */
export type WsHit = { kind: "group"; id: string } | { kind: "switch"; view: NavView } | null;

export type WsDrop =
  | { kind: "ws-add"; id: string; root: string }
  | { kind: "ws-remove"; id: string; root: string };

/** The drop a PROJECT drag means over `hit`, or null when the thing under the
 *  pointer cannot take it — and null shows no highlight and does nothing.
 *
 *  - onto a workspace that does not hold it: add it (from All projects, or
 *    from another workspace — a project can be in many);
 *  - onto the workspace it is already in: nothing;
 *  - a project IN a workspace onto the "All projects" switch: take it out of
 *    that workspace (never out of the registry);
 *  - anything else: nothing. */
export function projectDrop(item: WsItem, hit: WsHit, list: WsList | null): WsDrop | null {
  if (item.kind !== "project" || !hit || !list) return null;
  if (hit.kind === "group") {
    const w = list.workspaces.find((x) => x.id === hit.id);
    if (!w || w.projects.some((p) => p.root === item.root)) return null;
    return { kind: "ws-add", id: w.id, root: item.root };
  }
  if (hit.view === "projects" && item.ws) {
    const w = list.workspaces.find((x) => x.id === item.ws);
    if (!w || !w.projects.some((p) => p.root === item.root)) return null;
    return { kind: "ws-remove", id: w.id, root: item.root };
  }
  return null;
}

/** Whether hovering a switch segment during `item`'s drag should flip the nav
 *  to it (spring-load), so a project dragged in All projects can reach a
 *  workspace. Only toward Workspaces, only for a project, only from the other
 *  view: the "All projects" segment is a drop target, not a door. */
export const springsTo = (item: WsItem, hit: WsHit, view: NavView): NavView | null =>
  item.kind === "project" && hit?.kind === "switch" && hit.view === "workspaces" && view !== "workspaces"
    ? "workspaces" : null;

/** The roots `workspace_set_projects` gets for an add: appended, once. */
export const rootsWith = (w: WsDef, root: string): string[] =>
  w.projects.some((p) => p.root === root) ? w.projects.map((p) => p.root) : [...w.projects.map((p) => p.root), root];

/** …and for a removal. */
export const rootsWithout = (w: WsDef, root: string): string[] =>
  w.projects.filter((p) => p.root !== root).map((p) => p.root);

/** `ids` with `moved` placed before `before` (null = at the end). */
export function idsMoved(ids: string[], moved: string, before: string | null): string[] {
  const out = ids.filter((x) => x !== moved);
  const to = before === null ? -1 : out.indexOf(before);
  out.splice(to < 0 ? out.length : to, 0, moved);
  return out;
}

/** The menu's Move up / Move down: the same act as a drag between groups.
 *  `null` at an edge, where the item is disabled. */
export function idsStepped(ids: string[], id: string, dir: -1 | 1): string[] | null {
  const i = ids.indexOf(id);
  const j = i + dir;
  if (i < 0 || j < 0 || j >= ids.length) return null;
  const out = ids.slice();
  [out[i], out[j]] = [out[j], out[i]];
  return out;
}

/** The workspaces a project's "Add to workspace ▸" offers: the ones that do
 *  not hold it yet. */
export const addableTo = (list: WsList | null, root: string): WsDef[] =>
  (list?.workspaces ?? []).filter((w) => !w.projects.some((p) => p.root === root));

/** The collapse key of a project UNDER a workspace: its own per workspace, so
 *  folding it in one leaves All projects and every other workspace alone. */
export const wsProjectKey = (id: string, root: string) => `${id}|${root}`;

/** A workspace menu's "Add project ▸": the registered projects (`registered`,
 *  in All projects' order) it does not hold yet. */
export const projectsAddable = (w: WsDef, registered: string[]): string[] =>
  registered.filter((r) => !w.projects.some((p) => p.root === r));

/** The project a New project / Add existing / Clone just brought, so a
 *  workspace menu's continuation can put it in that workspace: the root the
 *  registry gained, or — when it already held it (Add existing of a project
 *  that is registered, or of a folder inside one) — the registered root that
 *  is `dir` or holds it, the deepest one. Null when neither: nothing is added. */
export function arrivedRoot(before: string[], after: string[], dir: string | null): string | null {
  const had = new Set(before);
  const fresh = after.find((r) => !had.has(r));
  if (fresh) return fresh;
  if (!dir) return null;
  const d = dir.replace(/\/+$/, "");
  const holds = after.filter((r) => d === r || d.startsWith(r + "/"));
  return holds.sort((a, b) => b.length - a.length)[0] ?? null;
}
