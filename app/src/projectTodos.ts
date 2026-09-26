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
// The COUNT is the number of things one button here can change: doctor findings
// (relink) plus the agent-setup items the Fix PR would carry (instruction dirs
// and repo skills). Everything else is a ROW with n = 0:
//
// - a Fix PR already open. The default branch does not move until it merges, so
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
  | "doctor"
  | "agent-dirs"
  | "agent-skills"
  | "agent-pending"
  | "agent-diverged";

/** What the row's own button does. `null`: the row only points at its section. */
export type TodoAction = "relink" | "agent-fix" | null;

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

/** The slice of App's `ProjectHealth` this needs — the sheet builds the same
 *  shape from its own fresher report. */
export type TodoHealth = { issues: number; error: string | null };

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
  } else if (health && health.issues > 0) {
    out.push({
      id: "doctor", label: `${health.issues} file-sync issue${s(health.issues)} found by doctor`,
      action: "relink", section: "health", n: health.issues, sev: "warn",
    });
  }

  const repo = agent?.repo;
  if (repo) {
    if (repo.pending) {
      out.push({
        id: "agent-pending", label: `Fix PR waiting to merge (branch ${repo.pending})`,
        action: null, section: "agent", n: 0, sev: "info",
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
