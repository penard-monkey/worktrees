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
// - a fix branch already made (on origin: a PR to merge; local only: a branch
//   to review in its lane, which is a different sentence). The default branch does not move until it merges, so
//   the report keeps saying `fixable` — counting it would badge a project whose
//   only remaining step is someone else's review, forever.
// - diverged dirs. Only a person can merge two instruction files; a count that
//   no button can lower is a nag, not a to-do.
// - a doctor that could not RUN. Its count is not a fact (no findings were
//   measured), so it is a row and a reason to offer Repair, never a number.
//
// The worktrees RULES (ADR 0002) are a row with n = 0 even though Apply can
// clear them: they are opt-in, every project starts without them, and counting
// them would badge every project the moment this shipped. The row is the
// per-project suggestion, and it puts "Repair / upgrade…" in the project menu
// (with no number) so Apply is reachable; the number stays what is BROKEN.
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
  | "doctor-stray"
  | "doctor-registry"
  | "doctor-manual"
  | "agent-dirs"
  | "agent-skills"
  | "agent-pending"
  | "agent-diverged"
  | "rules-apply"
  | "rules-outdated"
  | "rules-damaged";

/** What the row's own button does. `null`: the row only points at its section. */
export type TodoAction = "relink" | "force" | "provision" | "agent-fix" | "review" | "open-lane" | "remove-lane" | null;

export type Todo = {
  id: TodoId;
  label: string;
  action: TodoAction;
  /** The ProjectSheet section that explains it in full. */
  section: "health" | "agent" | "strays" | "rules";
  /** What this row adds to the project's count. 0 = informational. */
  n: number;
  sev: "error" | "warn" | "info";
  /** Puts "Repair / upgrade…" in the menu without adding to the count (see
   *  `needsRepair`) — for a row whose state, not whose id, makes it a next
   *  step. */
  repair?: boolean;
};

/** Doctor's actionable findings by the command that clears them. Built by
 *  `remedies()` in ProjectSheet.tsx; sums to `issueCount`. */
export type Remedies = { relink: number; force: number; provision: number; stray: number; registry: number; manual: number };

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
    const { relink, force, provision, stray, registry, manual } = health.remedies;
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
    // Strays get their OWN row, because the remedy is not the manual bucket's.
    // `stray-worktree` used to fall through `remedies()`'s `default:` into
    // `manual`, whose label reads "fix by editing .worktrees.toml or by hand" —
    // and editing that file does nothing whatever for a worktree registered
    // outside `.worktrees/`. It is a `git worktree move`, which the sheet's own
    // strays section already spells out per path. One project had eight of
    // these and was told, eight times over, to edit a config that could not
    // help. `n: 0` and `sev: "info"` deliberately: nothing is broken, the trees
    // are simply unmanaged, and that is the severity doctor itself assigns.
    if (stray > 0) {
      out.push({
        id: "doctor-stray",
        label: `${stray} worktree${s(stray)} registered outside .worktrees/ — adopting ${stray === 1 ? "it" : "them"} is a move, not a config edit`,
        action: null, section: "strays", n: 0, sev: "info",
      });
    }
    // Registry notes (`nested-project`, `prefix-collision`) get their own
    // row for the same reason strays do: the manual row says "edit
    // .worktrees.toml", and neither is fixed there. A nested vendor checkout
    // is legitimate; a shared prefix is fixed by giving one repo its own
    // `.worktree-prefix`. Informational, like doctor's own severity for the
    // first — `worktrees doctor` names the other project.
    if (registry > 0) {
      out.push({
        id: "doctor-registry",
        label: `${registry} note${s(registry)} about your registered projects (nested, or sharing a session prefix) — see worktrees doctor`,
        action: null, section: "health", n: 0, sev: "info",
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
      // `pending` only says the branch EXISTS. On origin it is a PR to merge;
      // local only is the DEFAULT result of Fix / Apply (they push only when
      // asked), so it reads as the next step, at info — not as a failed push.
      // No "Fix" in the noun: Apply rules writes the same branch.
      //
      // The branch's review lane (a place + an agent on a brief) is where it is
      // applied or abandoned. A local branch with NO place is a branch nothing
      // points at, so it puts Repair in the menu (uncounted) and its row opens
      // the lane; once a place holds it, the row only leads there. A merged
      // PR's place is done with — GitHub said so (`pending_merged`), never a
      // subject match — and that is a next step again.
      const b = repo.pending;
      const place = repo.pending_place ?? null;
      const row = (label: string, action: TodoAction, repair = false): Todo =>
        ({ id: "agent-pending", label, action, section: "agent", n: 0, sev: "info", repair });
      if (repo.pending_merged && place) {
        out.push(row(`PR merged — remove its review place '${place}'`, "remove-lane", true));
      } else if (place) {
        out.push(row(repo.pending_on_origin
          ? `PR waiting to merge (branch ${b}) — under review in place '${place}'`
          : `Branch '${b}' is under review in place '${place}' — its agent applies or abandons it on your word`, "open-lane"));
      } else if (repo.pending_on_origin) {
        out.push(row(`PR waiting to merge (branch ${b})`, "review"));
      } else {
        // No origin: "push it and open a PR" is impossible advice there.
        out.push(row(repo.has_origin
          ? `Branch '${b}' is committed locally — review it, then push it and open a PR, or abandon it`
          : `Branch '${b}' is committed locally — review it, then apply it to the default branch or abandon it`, "review", true));
      }
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
        out.push({ id: "agent-dirs", label: `${label} (on its own branch)`, action: "agent-fix", section: "agent", n: dirs, sev: "warn" });
      }
      if (skills > 0) {
        // Rolled into the SAME Fix PR — the label says so, and the sheet shows
        // the button once, so two rows never read as two pushes.
        out.push({
          id: "agent-skills",
          label: `Link ${skills} repo skill${s(skills)} for Codex (${dirs > 0 ? "same commit" : "on its own branch"})`,
          action: "agent-fix", section: "agent", n: skills, sev: "warn",
        });
      }
    }
    // The rules ride the same branch: while it is pending, the row above
    // already says what to do next, and Apply would refuse.
    const rules = agent?.rules;
    if (rules && !repo.pending && !rules.pending) {
      if (rules.block.state === "malformed" || rules.skill.state === "malformed") {
        out.push({
          id: "rules-damaged", label: "The worktrees rules' markers are damaged — fix them by hand",
          action: null, section: "rules", n: 0, sev: "warn",
        });
      } else if ((!rules.blocked && rules.block.state === "outdated") || rules.skill.state === "outdated") {
        // A blocked root's block is judged but never written (core
        // `applicable`, the sheet's `rulesTodo`), so it is not an update Apply
        // can make — and Repair would land on a disabled button.
        out.push({
          id: "rules-outdated", label: `The worktrees rules are from an older worktrees — Apply updates them to v${rules.version}`,
          action: null, section: "rules", n: 0, sev: "info",
        });
      } else if (rules.block.state === "absent" && !rules.blocked) {
        out.push({
          id: "rules-apply", label: "Teach every agent here how this repo is worked on — apply the worktrees rules",
          action: null, section: "rules", n: 0, sev: "info",
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

/** Rows that put "Repair / upgrade…" in the menu without adding to its
 *  number: a doctor that could not run (it needs you), and the worktrees rules
 *  Apply can write or update — the menu is how that button is reached, but
 *  they are opt-in, so they never badge. `rules-damaged` is not here: its fix
 *  is by hand, and nothing behind Repair can do it. */
const UNCOUNTED_REPAIR: ReadonlySet<TodoId> = new Set(["doctor-error", "rules-apply", "rules-outdated"]);

/** Whether "Repair / upgrade…" belongs in the menu: something a button can
 *  change, or one of the uncounted rows above, or a row flagged `repair`
 *  (an agent-instructions branch with no review place, or a merged one whose
 *  place can go). */
export function needsRepair(todos: Todo[]): boolean {
  return todoCount(todos) > 0 || todos.some((t) => UNCOUNTED_REPAIR.has(t.id) || t.repair === true);
}
