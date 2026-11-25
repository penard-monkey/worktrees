/** Plans and their workspaces (idea board phase 4, docs/proposals/
 *  idea-board-phases.md §2.2, §2.3, §6.1): the RULES both windows follow,
 *  pure. No runtime import: `scripts/plans-check.mjs` loads this file as a
 *  `data:` module and runs every non-drag path through it, since a check
 *  cannot drive a real drag (§6.1).
 *
 *  Every act ends in ONE call built here (`makePlanCall`, `addIdeasCall`,
 *  …), the drag and its menu path alike; that call is the row a later
 *  agent's proposal makes too (§7.1). The reader is core's
 *  (`worktrees_core::plans`); this file only words what it answers. */

// ── what the rows answer ────────────────────────────────────────────────────

export type ProjectState = {
  line: number; name: string;
  state: "registered" | "not-registered" | "to-clone" | "to-create";
  root?: string; url?: string; path?: string;
};
/** Core's `Started`: a work item's `started:` line, read (phase 5). */
export type Started = { project: string; slug: string; at: string };
export type WorkItem = {
  line: number; title: string; project: string | null; branch: string | null; brief: string | null;
  /** The agent the item names, when it names one. */
  ai: string | null; model: string | null;
  /** The `started:` line as written, and as read (null when it is neither). */
  started: string | null; place: Started | null;
};
/** A work item judged against the registry (core's answer, one per item):
 *  where its place would go, and the root of the place it has. */
export type WorkState = { line: number; project_state: ProjectState["state"] | null; root?: string; place_root?: string };
export type PlanProblem = { line: number; section: "projects" | "work"; text: string; words: string };
export type PlanIdea = { id: string; live: string | null; title: string | null; merged: boolean };
/** `plan_read`'s answer. `projects`/`work` are null when the plan has no such
 *  section, an empty list when it has an empty one. */
export type PlanView = {
  doc: { id: string; title: string; file: string; mtime_ms: number; content: string; kind: "plan" };
  ideas: PlanIdea[];
  projects: unknown[] | null;
  work: WorkItem[] | null;
  problems: PlanProblem[];
  project_states: ProjectState[];
  work_states: WorkState[];
  workspace: { id: string; name: string } | null;
};
export type OvProject = ProjectState & { in_workspace: boolean };
/** `workspace_overview`'s answer (§9.2). */
export type Overview = {
  id: string; name: string;
  plan: { id: string; live: string | null; title: string | null; file: string | null; mtime_ms: number | null; error?: string } | null;
  ideas: PlanIdea[];
  projects: OvProject[] | null;
  work: WorkItem[] | null;
  work_states: WorkState[];
  problems: PlanProblem[];
};

/** One row call: its command and its arguments, as `invoke` takes them. */
export type Call = { cmd: string; args: Record<string, unknown> };

// ── the calls (§7.1: one named operation each) ──────────────────────────────

/** Make a plan from one idea or several; `title` defaults to the first
 *  idea's in core. The draft is a parameter the row takes (an agent's later),
 *  never sent from here: phase 4 makes the template. */
export function makePlanCall(ideas: readonly string[], title?: string | null): Call {
  const args: Record<string, unknown> = { ideas: [...new Set(ideas)] };
  if (title && title.trim()) args.title = title.trim();
  return { cmd: "plan_make", args };
}
export function addIdeasCall(plan: string, ideas: readonly string[], mtime?: number | null): Call {
  const args: Record<string, unknown> = { id: plan, ideas: [...new Set(ideas)] };
  if (mtime != null) args.expectedMtime = mtime;
  return { cmd: "plan_add_ideas", args };
}
export function removeIdeaCall(plan: string, idea: string, mtime?: number | null): Call {
  const args: Record<string, unknown> = { id: plan, idea };
  if (mtime != null) args.expectedMtime = mtime;
  return { cmd: "plan_remove_idea", args };
}
export function addProjectCall(plan: string, root: string, mtime?: number | null): Call {
  const args: Record<string, unknown> = { id: plan, root };
  if (mtime != null) args.expectedMtime = mtime;
  return { cmd: "plan_add_project", args };
}
/** "Make workspace": the plan's workspace if it has one (opened), else a new one. */
export const fromPlanCall = (plan: string): Call => ({ cmd: "workspace_from_plan", args: { plan } });
export const setPlanCall = (ws: string, plan: string | null): Call => ({ cmd: "workspace_set_plan", args: { id: ws, plan } });
export const linkIdeaCall = (ws: string, idea: string): Call => ({ cmd: "workspace_link_idea", args: { id: ws, idea } });
export const unlinkIdeaCall = (ws: string, idea: string): Call => ({ cmd: "workspace_unlink_idea", args: { id: ws, idea } });

// Phase 5. A work item is named by its line AND title, and every act carries
// the plan's mtime, so an act on a plan that has moved is refused.
export type ItemRef = { line: number; title: string };
/** A cloned or created project: its line becomes `(registered)`, and the
 *  plan's workspace references it. */
export function setProjectCall(plan: string, name: string, root: string, mtime?: number | null): Call {
  const args: Record<string, unknown> = { id: plan, name, root };
  if (mtime != null) args.expectedMtime = mtime;
  return { cmd: "plan_set_project", args };
}
/** Record an existing place as the item's: the drop and "Link a place…". */
export function linkPlaceCall(plan: string, item: ItemRef, root: string, slug: string, mtime?: number | null): Call {
  const args: Record<string, unknown> = { id: plan, line: item.line, title: item.title, repo: root, slug };
  if (mtime != null) args.expectedMtime = mtime;
  return { cmd: "plan_link_place", args };
}
/** Start ONE item, with the brief the person reviewed (§7.1: `start(item,
 *  brief)`, the same call an agent's proposal would fill). */
export function startCall(
  plan: string, item: ItemRef & { branch: string | null }, root: string, brief: string, agent: { provider: string; model: string }, mtime?: number | null,
): Call {
  const args: Record<string, unknown> = { id: plan, line: item.line, title: item.title, repo: root, branch: item.branch, brief, provider: agent.provider, model: agent.model || null };
  if (mtime != null) args.expectedMtime = mtime;
  return { cmd: "plan_start", args };
}
/** The agent of a place that was made and not launched: `open_place`, which
 *  sends the opener on a harness's first launch in a place with a brief. */
export const launchCall = (root: string, slug: string, agent: { provider: string; model: string }): Call =>
  ({ cmd: "open_place", args: { repo: root, slug, provider: agent.provider, model: agent.model || null, force: true } });

/** `plan_start`'s answer. */
export type StartDone = {
  result: { ok: boolean; code: number; output: string; slug?: string; warnings?: string[] };
  started: Started | null;
  doc: { id: string; mtime_ms: number } | null;
  plan_error?: string;
};
/** `plan_set_project`'s answer, beside the plan's document. */
export type ProjectSet = { workspace: { id: string; name: string } | null; workspace_error?: string };
/** `diag::EXIT_LAUNCH_REFUSED`: the place is ready, its agent was not started. */
export const LAUNCH_REFUSED = 5;

// ── a work item's place, from the snapshot (phase 5) ────────────────────────
// Never stored: `started:` says WHICH place, the snapshot the nav already has
// says what it is doing now.

export type SnapPlace = { slug: string; path: string; branch: string | null; is_main: boolean; tmux_session: { up: boolean }; declared?: { title?: string | null } | null };
export type SnapProject = { root: string; name?: string | null; ok: boolean; snapshot?: { places: SnapPlace[] } | null };
export type Snap = { projects: SnapProject[] } | null;
/** Places (by path) whose agent works, or waits on the person. */
export type Activity = { busy: ReadonlySet<string>; waiting: ReadonlySet<string> };

export type ItemPlace = { kind: "none" | "unreadable" | "unknown" | "unregistered" | "gone" | "there" | "busy" | "waiting"; place?: SnapPlace; root?: string };

/** One item's place, as `started:` names it: what a snapshot is asked about. */
export const placeKey = (root: string, p: Started): string => `${root}|${p.slug}|${p.at}`;
/** The places these items name that `snap` does not have, in a project it
 *  could read. Missing is not gone: the plan's `started:` line is read in a
 *  moment and the list of places is a git fan-out, so right after a Start the
 *  list is older than the line (found in the real app; the mock answers at
 *  once). The window asks for the list again, and only that settles it. */
export function missingPlaces(work: readonly WorkItem[] | null, states: readonly WorkState[], snap: Snap): string[] {
  return (work ?? []).flatMap((w, n) => {
    const root = states[n]?.place_root;
    const p = root && snap?.projects.find((x) => x.root === root);
    return w.place && root && p && p.ok && p.snapshot && !p.snapshot.places.some((x) => x.slug === w.place!.slug) ? [placeKey(root, w.place)] : [];
  });
}

/** Where an item stands. `unknown` is a snapshot that has not come, a project
 *  it could not read, or a place missing from a list that may be older than
 *  the item's line (`gone`: the keys a later list settled). Never called gone. */
export function itemPlace(item: WorkItem, state: WorkState | undefined, snap: Snap, act: Activity, gone: ReadonlySet<string>): ItemPlace {
  if (!item.place) return { kind: item.started ? "unreadable" : "none" };
  const root = state?.place_root;
  if (!root) return { kind: "unregistered" };
  const p = snap?.projects.find((x) => x.root === root);
  if (!p?.ok || !p.snapshot) return { kind: "unknown", root };
  const place = p.snapshot.places.find((x) => x.slug === item.place!.slug);
  if (!place) return { kind: gone.has(placeKey(root, item.place)) ? "gone" : "unknown", root };
  return { kind: act.waiting.has(place.path) ? "waiting" : act.busy.has(place.path) ? "busy" : "there", place, root };
}
/** An item a Start or a link may take: it has no place, or the one it names
 *  is gone (its line is kept until then). Core holds the same rule. */
export const itemFree = (p: ItemPlace): boolean => p.kind === "none" || p.kind === "gone" || p.kind === "unregistered";

/** A place dropped on a work item, or picked for it: the item when it can
 *  take it, else null (no highlight, nothing happens). */
export function placeOntoItem(root: string, slug: string, item: WorkItem | null, at: ItemPlace | null, snap: Snap): ItemRef | null {
  if (!item || !item.title || !at || !itemFree(at) || !slug) return null;
  if (!snap?.projects.some((p) => p.root === root && p.snapshot?.places.some((x) => x.slug === slug))) return null;
  return { line: item.line, title: item.title };
}

/** A place's folder for a branch (`cmd_new`'s slug: `origin/` off, `/` → `-`). */
export const slugOf = (branch: string): string => branch.replace(/^origin\//, "").replace(/\//g, "-");

/** Why this item cannot be started, in words; null when it can. The host
 *  checks all of it again; this is what the review shows before it is asked. */
export function startProblem(item: WorkItem, state: WorkState | undefined, at: ItemPlace, snap: Snap): string | null {
  if (!itemFree(at)) return at.kind === "unreadable" ? "Its `started:` line cannot be read: fix or remove it in the plan." : "It has a place already.";
  if (!item.project) return "It names no project: add a `project:` line.";
  if (state?.project_state === "to-clone") return `“${item.project}” is not cloned yet: clone it from the plan's Projects.`;
  if (state?.project_state === "to-create") return `“${item.project}” is not created yet: create it from the plan's Projects.`;
  if (!state?.root) return `“${item.project}” is not a registered project.`;
  if (!item.branch) return "It names no branch: add a `branch:` line.";
  const places = snap?.projects.find((p) => p.root === state.root)?.snapshot?.places ?? [];
  const branch = item.branch.replace(/^origin\//, "");
  const holder = places.find((p) => p.branch === branch);
  if (holder) return holder.is_main ? `The branch ${branch} is checked out in the main checkout: give the item its own branch.` : `The branch ${branch} is already in the place ${holder.slug}: link that place instead.`;
  const taken = places.find((p) => p.slug === slugOf(branch));
  return taken ? `There is a place named ${taken.slug} already: link it, or give the item another branch.` : null;
}

/** The agent an item starts: the one it names, else the app's default. A
 *  model the item names goes with it; the default model belongs to its own
 *  harness only. */
export function agentFor(item: Pick<WorkItem, "ai" | "model">, defaults: { provider: string; models: Record<string, string | undefined> }): { provider: string; model: string } {
  const provider = item.ai || defaults.provider;
  return { provider, model: item.model || defaults.models[provider] || "" };
}

/** A `new: <path>` split for the New project dialog's two fields. */
export function newPath(path: string): { location: string; name: string } {
  const p = path.trim().replace(/\/+$/, "");
  const i = p.lastIndexOf("/");
  return i < 0 ? { location: "", name: p } : { location: p.slice(0, i) || "/", name: p.slice(i + 1) };
}

// ── the drop rules (§6.1) ───────────────────────────────────────────────────
// A drop onto something that cannot take it is `null`: no highlight, nothing
// happens. Adding and linking never ask.

export type DocRow = { id: string | null; kind?: "idea" | "plan"; problem?: string };

/** An idea dropped on a list row: onto a PLAN it is added to that plan's
 *  `plan_of` (the plan's id), never a relate/merge chooser. `null` when the
 *  row is not a plan or the dragged one is itself a plan. */
export function ideaOntoPlan(drag: DocRow | null, onto: DocRow | null, planned: readonly string[] = []): string | null {
  if (!drag?.id || !onto?.id || drag.kind === "plan" || onto.kind !== "plan" || onto.problem) return null;
  if (drag.id === onto.id || planned.includes(drag.id)) return null;
  return onto.id;
}

export type WsLite = { id: string; plan: string | null; ideas: string[] };

/** A workspace that can hold a plan or an idea. Everything Else (workspaces.ts,
 *  `EVERYTHING_ELSE_ID`, which plans-check.mjs holds this equal to) is derived,
 *  never stored, and takes neither. */
const namedWs = (w: WsLite | null): w is WsLite => !!w && w.id !== "everything-else";

/** A plan dropped on a workspace sets its plan, only when it has none (one
 *  plan, at most one workspace: a plan that has a workspace is no target
 *  anywhere else either). */
export function planOntoWorkspace(plan: string, onto: WsLite | null, all: readonly WsLite[]): string | null {
  if (!namedWs(onto) || onto.plan) return null;
  if (all.some((w) => w.plan === plan)) return null;
  return onto.id;
}

/** An idea dropped on a workspace links it, unless it is linked already. */
export function ideaOntoWorkspace(idea: string, onto: WsLite | null): string | null {
  if (!namedWs(onto) || onto.ideas.includes(idea)) return null;
  return onto.id;
}

/** A registered project dropped on a plan adds it, unless the plan already
 *  lists it (by the root its line matched). */
export function projectOntoPlan(root: string, plan: Pick<PlanView, "project_states"> | null): boolean {
  if (!plan || !root.startsWith("/")) return false;
  return !plan.project_states.some((s) => s.root === root);
}

// ── words ───────────────────────────────────────────────────────────────────

export const STATE_WORDS: Record<ProjectState["state"], string> = {
  registered: "registered",
  "not-registered": "not in the registry",
  "to-clone": "to clone",
  "to-create": "to create",
};

/** A reader problem as one line: where it is, and what was expected. */
export const problemLine = (p: PlanProblem): string => `Line ${p.line}: ${p.words}`;

/** The plans an idea's related list names (`planned-in`), once each. */
export function plansIn<T extends { id: string; how: string }>(rels: readonly T[]): T[] {
  const seen = new Set<string>();
  return rels.filter((r) => r.how === "planned-in" && (seen.has(r.id) ? false : (seen.add(r.id), true)));
}

/** "Make a plan": the selected ideas when the open one is among them (or
 *  nothing is open), else the open one alone. Plans are never included. */
export function planSources(open: DocRow | null, selected: readonly string[], rows: readonly DocRow[]): string[] {
  const isIdea = (id: string) => rows.some((r) => r.id === id && r.kind !== "plan" && !r.problem);
  const sel = selected.filter(isIdea);
  if (sel.length && (!open?.id || sel.includes(open.id))) return [...new Set(sel)];
  return open?.id && isIdea(open.id) ? [open.id] : [];
}

/** A work item's status: what its `started:` line says, and what is there. */
export function workStatus(w: WorkItem, at: ItemPlace): string {
  const when = w.place ? `started ${w.place.at}` : "";
  switch (at.kind) {
    case "none": return "not started";
    case "unreadable": return `started: ${w.started}`;
    case "unknown": return when;
    case "unregistered": return `${when} · its project is not registered`;
    case "gone": return `${when} · place removed`;
    case "busy": return `${when} · busy`;
    case "waiting": return `${when} · waiting on you`;
    case "there": return `${when} · ${at.place?.tmux_session.up ? "open" : "there"}`;
  }
}

/** The Ideas window's hand-off to the main one: the workspace to show, written to
 *  localStorage under this key just before it raises that window (`raise_window`),
 *  which reads it on load, on focus and on the `storage` event, and removes it.
 *  Older than `SHOW_TTL_MS` is ignored. A DOCUMENT goes by doc/handoff.ts, either
 *  way, and is answered. */
export const WS_SHOW_KEY = "wt.workspace.show";
/** …and that the Ideas window changed a workspace (linked, set a plan, made
 *  one): no event carries a change of the list, so the other window is told to
 *  read it again. The value is only a time; the `storage` event is the word. */
export const WS_CHANGED_KEY = "wt.workspaces.changed";
/** …and an act the main window's own dialogs do (phase 5): clone or create
 *  the plan's project `name`, or show a place. The same shape and age rule;
 *  nothing is done from it but opening a dialog, filled in, or selecting. */
export const MAIN_ASK_KEY = "wt.main.ask";
export type MainAsk = { act: "project"; plan: string; name: string } | { act: "place"; root: string; slug: string };
export function mainAsk(raw: string | null, now: number): MainAsk | null {
  if (!raw) return null;
  try {
    const v = JSON.parse(raw) as Record<string, unknown>;
    if (typeof v.at !== "number" || now - v.at > SHOW_TTL_MS || v.at > now + 5_000) return null;
    if (v.act === "project" && typeof v.plan === "string" && /^[0-9A-HJKMNP-TV-Z]{26}$/.test(v.plan) && typeof v.name === "string" && v.name) return { act: "project", plan: v.plan, name: v.name };
    if (v.act === "place" && typeof v.root === "string" && v.root.startsWith("/") && typeof v.slug === "string" && v.slug) return { act: "place", root: v.root, slug: v.slug };
    return null;
  } catch { return null; }
}
export const SHOW_TTL_MS = 60_000;
export function showRequest(raw: string | null, now: number): string | null {
  if (!raw) return null;
  try {
    const v = JSON.parse(raw) as { id?: unknown; at?: unknown };
    if (typeof v.id !== "string" || !/^[0-9A-HJKMNP-TV-Z]{26}$/.test(v.id) || typeof v.at !== "number") return null;
    return now - v.at <= SHOW_TTL_MS && v.at <= now + 5_000 ? v.id : null;
  } catch { return null; }
}
