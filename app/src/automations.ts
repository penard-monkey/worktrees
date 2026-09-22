// Automations — the DATA the tab renders that is not state: the starter briefs
// the empty state offers, and the closed set of tools a proposal may name.
//
// No React and no imports on purpose, for the same reason `dnd.ts` has none:
// `dockrail-check.mjs` loads this file as a module and compares `PROPOSAL_TOOLS`
// against `runs.rs`, and a relative import would give that loader a `data:` URL
// with no base to resolve it against (the `settings.ts` trap in CLAUDE.md).

/** `worktrees_core::runs::PROPOSAL_TOOLS`, mirrored. A run's proposals are
 *  validated in core; this copy exists so the run view can LABEL a button
 *  ("Mark abandoned", "Add note") without a second parse, and
 *  `dockrail-check.mjs` re-reads `runs.rs` so the mirror cannot drift.
 *
 *  `remove_worktree` is not here and never will be — see `automation.rs`'s
 *  module note and CLAUDE.md on `remove_place`'s `force`. */
export const PROPOSAL_TOOLS = ["set_lifecycle", "set_note", "set_pin", "close_session"] as const;

export type ProposalTool = (typeof PROPOSAL_TOOLS)[number];

/** The three starters the empty state offers (proposal §7.5).
 *
 *  Each brief is PROSE, 3–5 sentences, and deliberately names no tool: the
 *  whole design decision is that the thing you edit is a paragraph, and a
 *  starter that read like a command would teach the opposite (§2). The runner's
 *  fixed opener already tells claude where the facts are and what to write —
 *  these say what to look FOR.
 *
 *  They are examples, not commands: the empty state opens the modal prefilled
 *  so the first thing a new user does is read and edit one. */
export const STARTERS: { name: string; brief: string }[] = [
  {
    name: "Close-out candidates",
    brief:
      "Look at every worktree in this project and tell me which ones look finished. " +
      "A worktree is a candidate when its branch is already merged into the base, or it has no commits of its own, " +
      "and nobody has worked in it for a couple of weeks. " +
      "Say what each candidate was for, going by its branch name and its last commit, so I can recognise it. " +
      "Leave anything alone that still has unpushed commits or uncommitted changes — those are not finished, they are stalled, and they belong in a different report.",
  },
  {
    name: "Unpushed work",
    brief:
      "Find the work in this project that exists only on this machine. " +
      "For each worktree, tell me whether it has commits that are not on its upstream branch, or changes that are not committed at all. " +
      "Say how old that work is and what its last commit was about. " +
      "Order the list by how much would be lost if this laptop died tonight. " +
      "Ignore worktrees that are fully pushed, however busy they look.",
  },
  {
    name: "What happened this week",
    brief:
      "Write me a short summary of what moved in this project over the last seven days. " +
      "Go worktree by worktree: what was committed, what got merged, what was started and abandoned. " +
      "Group it into a few sentences of prose rather than a list of commits — I want to remember the week, not audit it. " +
      "End with the one or two worktrees that look like they need me next, and say why.",
  },
];
