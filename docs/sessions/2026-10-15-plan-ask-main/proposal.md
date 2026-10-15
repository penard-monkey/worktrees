# Plan tab: "Ask (main)" — proposal

## 1. Prompt text
New `ops::PLAN_UPDATE_PROMPT`, one line, no newline, with a single `{slug}` hole.
`ops::plan_update_prompt(slug)` fills it and REFUSES a slug outside `[A-Za-z0-9._-]`.
The slug never comes from the frontend. The command takes the place's `root`
(guarded by `guard_under_projects`), and the slug is the place's own (`.worktrees/<slug>`
dir name, checked against `Project::discover`'s list).

Draft:
> Update your plan for the place `{slug}`. Read where it actually is: its place_status,
> its task_plan.md / progress.md / findings.md and its branch's commits. Do not edit
> its tree. Then bring what you steer for it up to date: your own orchestrator plan,
> and its brief if the brief no longer matches the work. Tell its agent what changed,
> or say here that nothing needed to.

What main does with it: the lane's `task_plan.md`/`progress.md` are its SESSION's,
read-only to main. Main owns two things: its `.planning/orchestrator/` entry for the
lane, and the lane's `brief.md`, which only main writes.
**Question for you:** may main rewrite `.worktrees/<slug>/.planning/brief.md`
directly? The rule is "don't edit another place's tree". The alternatives are
re-issuing it through `create_worktree` (which replaces the brief) or sending the
change as a message. I've worded the prompt as "its brief", so main picks the
mechanism its own rules allow.

## 2. Paste vs send — recommend PASTE + refuse on draft/waiting + jump to (main)
- **MCP `report`**: rejected. Main reads its log only when it calls
  `messages`/`wait`, so the message sits there unread until main happens to check.
- **send-keys + Enter after an empty check**: rejected. `agent::draft_from_screen`
  returning `None` means "no draft OR no box recognised", so it is not proof of empty.
  An auto-sent prompt also breaks the Generate rule (the user reads it, then sends it).
- **Paste** (`tmux::paste_to_ai`, no newline), guarded server-side first:
  - refuse when main's composer holds a draft. Claude: `draft_from_screen` on a
    `capture-pane -e`. Codex: `codex::composer_on_screen`. Message: "(main) has
    unsent text at its prompt — send or clear it first". This is the glue hazard.
  - refuse when main's state is `waiting`. A paste into a permission or selector
    modal is input to the modal.
  - pi: the existing trust-modal check.
  - on success, the frontend SELECTS (main), so the user lands on its composer
    with the prompt in it, and the note says "press Enter there".

## 3. Finding (main)'s session
The frontend passes (main)'s `tmux_session.name` from `ls`, same as `plan_prompt`
(it handles adopted names). It also passes (main)'s provider. Each failure gets an
honest message:
- (main) session not up: button disabled, tooltip "(main) has no session — open it first".
- up, but no agent in it: `paste_to_ai`'s existing error, "no claude running in
  session worktrees-(main) (panes: …)".
- a different agent (codex/pi): works through that provider's match word, and the
  label names it ("Ask (main)'s Codex"). Plain shell: same as "no agent".

## 4. Placement / label
- In the dock HEADER next to the "open" button, shown on the Plan tab when
  `source` is `plan` or `brief`.
- Label "Ask (main)". Tooltip: "Put a fixed prompt at (main)'s Claude prompt
  asking it to review this place's progress and update its brief and plan.
  Switches to (main); nothing is sent until you press Enter there."
- NOT in the empty state. No brief means main never steered this place, and
  Generate plan is the right action there.
- A "pasted into (main)" note appears beside the button, as `plan-gen-note` does.

## 5. Gaps
- Hidden on a `(main)` place itself, and on a project with no (main) session record.
- Always the place's OWN project's (main). Every project has one, so it applies to
  all of them. No cross-project targeting.
- Only main's session is resolved server-side as "up". The frontend gate is a
  convenience, not the check.
- Mock: a `plan_ask_main` case that records calls, like `plan_prompt`.
- Tests: the constant has no newline and one `{slug}`, and the filler refuses a
  bad slug.
