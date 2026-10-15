# project-rules — the worktrees way of working, written into a repo

- **Date:** 2026-10-14 → 2026-10-15
- **Place:** `project-rules`
- **Branches:** `project-rules-pack`, `project-rules-app`, `project-rules-closeout-pointers`, `project-rules-repair-offer`, `agent-instructions` (B3)
- **PRs:** #488 (B1 core + CLI + MCP), #492 (B2 app), #496 (B3, this repo applied), #499 (stale close-out pointers), #500 (Repair for rules rows). Elsewhere: mac-utilities #22, saywhat #94, valleos #395, claude-skills #1 (global close-out retired)
- **Release:** v0.42.0 (#498) carries B1 + B2
- **Planning files:** `task_plan.md` (in `planning.tar.gz`); the brief stays in `.planning/brief.md`

## What shipped

- **`worktrees rules status | diff | apply [--push] [--force]`** (#488),
  `crates/worktrees-core/src/rules.rs`. Three pieces, all text an agent reads:
  - a managed block at the END of `AGENTS.md`, between
    `<!-- worktrees:rules v1 sha=… -->` and `<!-- worktrees:rules end -->`
    (text in `guidance/rules-block.md`);
  - the close-out skill at `.claude/skills/close-out/SKILL.md`, marker under
    the frontmatter, plus `.agents/skills/close-out` → it for Codex
    (`guidance/close-out/SKILL.md`);
  - a one-time `.claude/close-out.md` template when the repo has none
    (`guidance/close-out-config.md`). A repo's own config is never touched.
  - States per piece: absent / current / outdated / edited / newer /
    malformed / foreign. Apply writes absent and outdated; `--force` also
    replaces edited; newer, malformed and foreign are never written.
  - `worktrees guide --close-out` prints the skill without applying anything.
  - MCP: `rules_status` (read-only), `apply_rules` (`--mutations` only, never
    pushes, force needs `confirm`).
- **Apply and `agent-setup fix` commit LOCALLY by default** (#488,
  `agentfiles.rs` split into `prepare` / `publish`): a plumbing commit on
  `agent-instructions` off `origin/<default>`, through a private index, never a
  checkout. `--push` pushes and opens a PR. CHANGELOG `### Changed` records the
  `agent-setup fix` default change.
- **ADR 0002** (`docs/adr/0002-rules-pack-in-repo.md`): the pack is data an
  agent reads, never argv, so it stays inside ADR 0001.
- **App** (#492, `app/src/AgentSetup.tsx`): a Worktrees rules section beside
  Agent setup in Project settings, the `rules_apply` command, per-section push
  checkboxes, a force checkbox only when the block is edited ("Replace edited
  text + commit"), and To do rows `rules-apply` / `rules-outdated` /
  `rules-damaged` that all count 0.
- **This repo carries the pack** (#496): unedited tool output, verified byte
  for byte against the generator and with the existing AGENTS.md prefix
  unchanged. #499 rewrote AGENTS.md's "Close-out ritual" and
  `.claude/close-out.md` to point at the repo skill, and added
  `cargo test -p app --lib` to the close-out gates.
- **"Repair / upgrade…" for rules rows** (#500, `app/src/projectTodos.ts`):
  `rules-apply` and `rules-outdated` put the item in the project menu with no
  number (`UNCOUNTED_REPAIR`). `rules-damaged`, a pending branch, and an
  outdated block in a blocked root do not. Mock: `?rulesonly[=outdated]`.
- **Three other repos carry the pack** (mac-utilities, saywhat, valleos), each
  applied locally with the v0.42.0 CLI and checked the same way.
- **The global close-out skill is retired** (claude-skills #1):
  `skills/close-out` became `retired/close-out.md`, and `install.sh` now
  removes links into its `skills/` whose target is no longer a skill. Running
  it removed `~/.claude/skills/close-out`, so `/close-out` now runs each
  repo's copy.

## Decisions

- **Local commit by default, `--push` to publish.** The branch is reviewed as
  a place before anything leaves the machine. `agent-setup fix` switched to
  the same default so the two buttons that share `agent-instructions` behave
  alike.
- **The close-out skill lives in the binary and is written into each repo.**
  A per-machine skills repo already went missing once (ROADMAP), and a
  teammate or another machine never had it.
- **The block goes at the end of AGENTS.md, and only the block is ours.**
  Text after the END marker survives a re-apply, so close-out learnings
  appended later are safe. Do not "fix" that by moving the block.
- **Rules rows count 0.** They are opt-in and absent everywhere on day one,
  so counting them would badge every project the moment v0.42.0 shipped. #500
  still makes them reachable from the menu, without a number.
- **No machine-level offer for the rules** (B2 deviation): `offers.ts`
  forbids a per-project destination.
- **The retirement waited on order:** v0.42.0 shipped, then the pack landed
  in all four repos, and only then the global copy went. Retiring it first
  would have broken `/close-out` in four repos.

## Dead ends / gotchas

- **A personal skill outranks a project skill of the same name.** While
  `~/.claude/skills/close-out` existed, this repo's copy was dormant on
  David's machine and live everywhere else, so AGENTS.md had to say which one
  was running.
- **`rules apply` refuses while `agent-instructions` exists.** Both this repo
  and valleos had a stale one from an already-merged PR (valleos #228:
  `rev-list --left-right` 667/0). Deleting another repo's branch was left to
  David.
- **`rules status` reads `origin/<default>`, not the checkout.** After an
  apply it still says "not applied" until the branch merges, by design.
- **The fable review of #488 found real holes:** `rules diff` previewed writes
  apply would refuse; a hand-edited `v2` block read as `edited`, so `--force`
  could downgrade it (version is now checked before hash); a symlinked
  `.claude` ancestor surfaced a raw git index error (now `foreign`); CRLF
  read as `edited`; `trim_end` ate a markdown hard break.
- **#500's first cut gave a blocked root's outdated block a Repair that
  landed on a disabled Apply.** The menu mirrored the row, while the button
  mirrored core `applicable`. The fix gates on `!blocked`, and ROADMAP has
  the drift check that would have caught it.
- **Mock harness traps** (Playwright, port 1488):
  - the nav starts `hidden`, and ⌘B cycles hidden → revealed → pinned, with
    the pin persisting across reloads, so the script presses until the class
    is exactly `nav`;
  - the menu reads App's cached health and agent state, which refreshes on a
    5-minute sweep, so a post-load `__mock` mutation is invisible to it.
    Hence `?rulesonly` applies at module load.
- **CI `app (ubuntu-latest)` died in `pnpm/action-setup`** (exit 152) before
  any code ran: infra. The next push ran green.

## Verification

- Every gate in AGENTS.md ran green before #488 and #492. CI was green on
  every PR: 9/9 jobs (#499: run 38803496510; #500: run 38805513705 after a
  re-push).
- Each apply (this repo, mac-utilities, saywhat, valleos) was checked by
  `cmp` of the old AGENTS.md against the new prefix,
  `git diff --quiet -- .claude/close-out.md`, and the link resolving in the
  adopted place.
- #500: `todos-check.mjs` was red before each change (2 failures, then 1),
  green after. `make test-frontend`, `tsc` and `cargo check -p app` pass. A
  Playwright run in Chromium and WebKit against the mock checked:
  - the counted fixture still shows its number;
  - with only the rules absent, and again with only the rules outdated, the
    item shows with no number and hit-tests;
  - the To do row's Details button scrolls to the rules section, where
    "Apply rules…" hit-tests.
- claude-skills `install.sh` prune: a scratch `CLAUDE_SKILLS_DIR` with one
  retired link, one foreign link and one real directory. Only the retired
  link was removed, and a second run removed nothing.

## Follow-ups

- ROADMAP: rules block v2 (`tmux -L` under "Never touch", and one wording for
  `.planning/` vs the root planning files), both already there from #499; the
  `UNCOUNTED_REPAIR` drift check; dropping the "personal copy outranks"
  paragraph from AGENTS.md now that the global copy is gone.
- Leftover places on merged branches, created by this session in other
  projects, left for their owners to remove: `agent-instructions` in
  mac-utilities, saywhat and valleos, and `retire-close-out` in claude-skills.
