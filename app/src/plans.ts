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
export type WorkItem = {
  line: number; title: string; project: string | null; branch: string | null; brief: string | null; started: string | null;
};
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
  workspace: { id: string; name: string } | null;
};
export type OvProject = ProjectState & { in_workspace: boolean };
/** `workspace_overview`'s answer (§9.2), read-only. */
export type Overview = {
  id: string; name: string;
  plan: { id: string; live: string | null; title: string | null; file: string | null; error?: string } | null;
  ideas: PlanIdea[];
  projects: OvProject[] | null;
  work: WorkItem[] | null;
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

/** A plan dropped on a workspace sets its plan, only when it has none (one
 *  plan, at most one workspace: a plan that has a workspace is no target
 *  anywhere else either). */
export function planOntoWorkspace(plan: string, onto: WsLite | null, all: readonly WsLite[]): string | null {
  if (!onto || onto.plan) return null;
  if (all.some((w) => w.plan === plan)) return null;
  return onto.id;
}

/** An idea dropped on a workspace links it, unless it is linked already. */
export function ideaOntoWorkspace(idea: string, onto: WsLite | null): string | null {
  if (!onto || onto.ideas.includes(idea)) return null;
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

/** A work item's status line in phase 4: its `started:` line as written, or
 *  "not started". Phase 5 judges the place; this only shows the text. */
export const workStatus = (w: WorkItem): string => (w.started ? `started: ${w.started}` : "not started");

/** The main window's hand-off to the Ideas window: the document to open,
 *  written to localStorage under this key just before `open_companion`
 *  brings the window forward. The companion reads it on load, on focus and
 *  on the `storage` event, and removes it. Older than `SHOW_TTL_MS` is
 *  ignored (a request the companion never saw). */
export const SHOW_KEY = "wt.ideas.show";
/** …and the other way: the workspace the main window shows, written before
 *  the companion raises it (`raise_window`). The same shape and age rule. */
export const WS_SHOW_KEY = "wt.workspace.show";
export const SHOW_TTL_MS = 60_000;
export function showRequest(raw: string | null, now: number): string | null {
  if (!raw) return null;
  try {
    const v = JSON.parse(raw) as { id?: unknown; at?: unknown };
    if (typeof v.id !== "string" || !/^[0-9A-HJKMNP-TV-Z]{26}$/.test(v.id) || typeof v.at !== "number") return null;
    return now - v.at <= SHOW_TTL_MS && v.at <= now + 5_000 ? v.id : null;
  } catch { return null; }
}
