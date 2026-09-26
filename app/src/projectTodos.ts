// A project's "things to do" — ONE model behind the nav header badge, the
// project menu's "Repair / upgrade…" entry and the To do list at the top of the
// ProjectSheet. Three surfaces, one answer: none of them computes its own count,
// which is how the doctor badge and the row glyphs once came to disagree
// (see `issueCount` in ProjectSheet.tsx).
//
// Pure, and importing only TYPES, so `scripts/todos-check.mjs` evaluates this
// file itself rather than a paraphrase of it.
//
// # What counts
//
// The COUNT is the number of things one button here can change: the doctor
// findings a relink, a re-seed or a provision clears (split by remedy — see
// `remedies` in ProjectSheet.tsx) plus the agent-setup items the Fix PR would
// carry (instruction dirs and repo skills). Everything else is a ROW with n = 0:
//
// - doctor findings only an edit clears (`.worktrees.toml`, or by hand). A
//   count under no button is a nag, and a count under the WRONG button is a
//   lie: `shadowed` under Relink promised a repair relink refuses by design.
// - a fix branch already made (on origin: a PR to merge; local only: a push
//   that never happened, which is a different sentence). The default branch does not move until it merges, so
//   the report keeps saying `fixable` — counting it would badge a project whose
//   only remaining step is someone else's review, forever.
// - diverged dirs. Only a person can merge two instruction files; a count that
//   no button can lower is a nag, not a to-do.
// - a doctor that could not RUN. Its count is not a fact (no findings were
//   measured), so it is a row and a reason to offer Repair, never a number.
//
// The USER'S own skills (~/.claude/skills) are machine-wide and deliberately NOT
// here: a per-project to-do about them would stand under every project at once.
// They are an after-update offer instead (`offers.ts`, `codex-skills`).
import type { AgentSetupStatus } from "./AgentSetup";

export type TodoId =
  | "doctor-error"
  | "doctor-relink"
  | "doctor-force"
  | "doctor-provision"
  | "doctor-manual"
  | "agent-dirs"
  | "agent-skills"
  | "agent-pending"
  | "agent-diverged";

/** What the row's own button does. `null`: the row only points at its section. */
export type TodoAction = "relink" | "force" | "provision" | "agent-fix" | null;

export type Todo = {
  id: TodoId;
  label: string;
  action: TodoAction;
  /** The ProjectSheet section that explains it in full. */
  section: "health" | "agent";
  /** What this row adds to the project's count. 0 = informational. */
  n: number;
  sev: "error" | "warn" | "info";
};

/** Doctor's actionable findings by the command that clears them. Built by
 *  `remedies()` in ProjectSheet.tsx; sums to `issueCount`. */
export type Remedies = { relink: number; force: number; provision: number; manual: number };

/** The slice of App's `ProjectHealth` this needs — the sheet builds the same
 *  shape from its own fresher report. */
export type TodoHealth = { issues: number; error: string | null; remedies: Remedies };

const s = (n: number) => (n === 1 ? "" : "s");

export function projectTodos(
  health: TodoHealth | null | undefined,
  agent: AgentSetupStatus | null | undefined,
): Todo[] {
  const out: Todo[] = [];

  if (health?.error) {
    out.push({
      id: "doctor-error", label: "Config unreadable — doctor could not check this project",
      action: null, section: "health", n: 0, sev: "error",
    });
  } else if (health) {
    const { relink, force, provision, manual } = health.remedies;
    if (relink > 0) {
      out.push({
        id: "doctor-relink", label: `${relink} declared file${s(relink)} not linked into ${relink === 1 ? "a worktree" : "worktrees"}`,
        action: "relink", section: "health", n: relink, sev: "warn",
      });
    }
    if (force > 0) {
      // Its own row and its own ARMED button: it moves the local file aside as
      // .bak and rewrites it from main, and that content may be the only copy.
      out.push({
        id: "doctor-force", label: `Re-seed ${force} file${s(force)} from main (the local copy is kept as .bak)`,
        action: "force", section: "health", n: force, sev: "warn",
      });
    }
    if (provision > 0) {
      out.push({
        id: "doctor-provision", label: `${provision} port setup${s(provision)} missing — provision allocates a slot`,
        action: "provision", section: "health", n: provision, sev: "warn",
      });
    }
    if (manual > 0) {
      out.push({
        id: "doctor-manual", label: `${manual} finding${s(manual)} to fix by editing .worktrees.toml or by hand`,
        action: null, section: "health", n: 0, sev: "info",
      });
    }
  }

  const repo = agent?.repo;
  if (repo) {
    if (repo.pending) {
      // `pending` only says the branch EXISTS. Whether there is anything to
      // wait for depends on where: a push that failed (or no origin, no gh)
      // leaves a local branch and no PR at all.
      out.push({
        id: "agent-pending",
        label: repo.pending_on_origin
          ? `Fix PR waiting to merge (branch ${repo.pending})`
          : `Fix branch '${repo.pending}' was committed but never pushed — push it, or delete it to redo`,
        action: null, section: "agent", n: 0, sev: repo.pending_on_origin ? "info" : "warn",
      });
    } else if (repo.fixable) {
      const claudeOnly = repo.dirs.filter((d) => d.kind === "claude-only").length;
      const agentsOnly = repo.dirs.filter((d) => d.kind === "agents-only").length;
      const skills = repo.skills.filter((k) => k.kind === "missing").length;
      const dirs = claudeOnly + agentsOnly;
      if (dirs > 0) {
        const label = claudeOnly > 0 && agentsOnly > 0
          ? `Move ${claudeOnly} CLAUDE.md file${s(claudeOnly)} to AGENTS.md and stub ${agentsOnly} more`
          : claudeOnly > 0
            ? `Move ${claudeOnly} CLAUDE.md file${s(claudeOnly)} to AGENTS.md`
            : `Add a CLAUDE.md stub beside ${agentsOnly} AGENTS.md file${s(agentsOnly)}`;
        out.push({ id: "agent-dirs", label: `${label} (opens a PR)`, action: "agent-fix", section: "agent", n: dirs, sev: "warn" });
      }
      if (skills > 0) {
        // Rolled into the SAME Fix PR — the label says so, and the sheet shows
        // the button once, so two rows never read as two pushes.
        out.push({
          id: "agent-skills",
          label: `Link ${skills} repo skill${s(skills)} for Codex (${dirs > 0 ? "same PR" : "opens a PR"})`,
          action: "agent-fix", section: "agent", n: skills, sev: "warn",
        });
      }
    }
    const diverged = repo.dirs.filter((d) => d.kind === "diverged").length;
    if (diverged > 0) {
      out.push({
        id: "agent-diverged",
        label: `${diverged} folder${s(diverged)} where CLAUDE.md and AGENTS.md differ — merge by hand`,
        action: null, section: "agent", n: 0, sev: "error",
      });
    }
  }
  return out;
}

/** The project's number: what the buttons here can change. */
export function todoCount(todos: Todo[]): number {
  return todos.reduce((a, t) => a + t.n, 0);
}

/** Whether "Repair / upgrade…" belongs in the menu: something to fix, or a
 *  doctor that could not run (the one uncounted row that still needs you). */
export function needsRepair(todos: Todo[]): boolean {
  return todoCount(todos) > 0 || todos.some((t) => t.id === "doctor-error");
}
