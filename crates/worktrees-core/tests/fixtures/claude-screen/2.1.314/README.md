Captured with `tmux capture-pane -p` (and `-e -p` for the `.e.txt` files)
from Claude Code **2.1.314** (native install, macOS 27.0, Haiku 5.5 unless
noted, `--permission-mode default`) on 2026-10-30/31, for
`worktrees_core::prompt` and serve's agent rows, the two added on
`feat/actionable-prompts` (`agent_answer_text`, `agent_answer_many`)
included.

Every pane ran in a PRIVATE tmux server in a scratch git repository under
`/private/tmp/claude-501/wtap/`; no user session was touched. Claude was
started with every `CLAUDE_*` variable of the capturing session unset (one
inherited `CLAUDE_CODE_CHILD_SESSION` turns transcript saving off). Only
trailing blank rows were removed; every visible character is verbatim.

**Named, as worktrees launches it.** Unless a name below says otherwise,
Claude ran with `--name wtap-probe` (`--name 'wtap-(main)'` for
`question-named`), the flag every lane worktrees launches gets. A named
Claude draws its name into a full rule — `──── wtap-probe ─` — as the
screen's LAST row while a modal is up, below the footer with blank rows
between, and into the composer box's top rule otherwise. The 2.1.309
fixtures were captured without `--name`, so `prompt::claude`, which wants the
footer as the last row, read no prompt of a worktrees-launched Claude; the
joke-topic question of 2026-10-30 was that. Claude 2.1.301 draws the tag the
same way: it is not new in 2.1.314. The folder-trust prompt comes before it
(`trust*.txt` have none). 120×40 unless noted.

Unnamed (`question.txt` at 120×40; `question-type-something*`,
`question-typed*` at 110×43, the pane the joke-topic question ran in):

- `question.txt` (+ `.e`): AskUserQuestion, one question with
  descriptions — the 2.1.309 layout: `☐` header, a rule inside the list
  above `Chat about this`, footer `Enter to select · ↑/↓ to navigate · Esc
  to cancel`.
- `question-type-something` (+ `.e`): the highlight on `Type something.`,
  whose placeholder is then dim; the footer gains `ctrl+g to edit in Vim`.
- `question-typed` (+ `.e`): `penguins` typed there: the text REPLACES the
  label (`❯ 4. penguins`), so the free-text row is known by where it sits
  (the last option, above the rule in the list), never by its words.
- `question-typed-away.e.txt`: the highlight moved off; the text stays.

Named:

- `question-named` (+ `.e`): the user's case, reproduced through a real
  `worktrees serve` (Opus and Haiku alike, auto mode, 110×43): readable
  unnamed, `readable: false` named.
- `trust`, `trust-yes` (+ `.e`): the folder-trust prompt (no tag yet).
- `permission-bash` (+ `.e`), `-moved`: a Bash approval, four choices; the
  footer is `Esc to cancel · Tab to amend` on "Yes" and "No" and `Esc to
  cancel` elsewhere.
- `permission-bash-no`, `-no-amend`, `-no-amend-typed`: "No", then Tab —
  the row becomes `No, and tell Claude what to do differently` — then
  `use a different file name` typed: `No, use a different file name`.
- `permission-bash-yes-amend`, `-yes-amend-pasted`: the same on "Yes"
  (`Yes, and tell Claude what to do next`), then `then list the directory`
  PASTED (`tmux paste-buffer -p`, as `tmux::paste_into` does): Enter ran the
  command and sent the text as the next message.
- `permission-write`, `permission-edit` (+ `.e`): Write and Edit approvals,
  three choices, the middle one switching to accept-edits.
- `permission-edit-tall` (+ `.e`): an Edit whose diff is taller than the
  pane. The modal's top is gone and tmux holds no history of it (Claude
  repaints in place); only the diff's tail, the question, the choices and
  the footer are on screen.
- `permission-mcp` (+ `.e`): an MCP tool (`worktrees — List Places Tool:
  (MCP)`); choice 2 is cut with `…` at the pane's edge.
- `plan-approval` (+ `.e`): ExitPlanMode. A sheet under a column-0 row of
  `▔`, its rules indented two columns, the plan in a dashed box, choices
  `Yes, and use auto mode` / `Yes, manually approve edits` / `Tell Claude
  what to change` (+ the hint `shift+tab to approve with this feedback`),
  footer `ctrl+g to edit in Vim · ~/.claude/plans/<name>.md`.
- `plan-approval-feedback`, `-feedback-typed`: the highlight on the feedback
  row, then `call it greet.sh instead` pasted into it (Enter sent it and
  Claude revised the plan).
- `plan-approval-tall` (+ `.e`): a 45-step plan. The sheet's top stays, but
  the plan is CLIPPED inside its box: its dashed bottom edge is missing.
- `question-multiselect` (+ `.e`), `-toggled`, `-on-submit`, `-review`:
  multiSelect. A `←  ☐ Toppings  ✔ Submit  →` tab bar even for one
  question, `[ ]` / `[✔]` boxes, an unnumbered `Submit` row between the last
  box and the rule (the highlight walks over it), Enter TICKS a box, and
  Enter on Submit opens the review screen (`1. Submit answers`).
- `question-multiselect-text-row`, `-text-typed`: the box list's own text
  row is `[ ] Type something` (no period); typing `pineapple` there ticks it.
- `question-two` (+ `.e`), `-two-second`, `-two-review`: two questions: one
  screen per question (Enter answers and moves on), then the review.
- `composer-empty`, `composer-typed`, `composer-after-turn`,
  `composer-busy-empty`, `composer-busy-typed` (+ `.e`): the composers, the
  name in the box's top rule; `half typed draft, not sent` and `queued
  thought` typed with `send-keys -l` and no Enter.

What was measured, and the code relies on:

- Up/Down move the highlight and Enter confirms, in every modal above, as on
  2.1.309. Tab opens the amend on "Yes"/"No" only.
- A bracketed paste lands in every field: the amend, `Type something.`, the
  plan's feedback row, a box list's text row.
- **The transcript does not hold a pending tool call.** While the tall Edit
  (80+ s) and a Write (25 s) waited, the session's transcript had no
  assistant entry for them; Claude writes it once the prompt is answered.
- A `PermissionRequest` hook (a throwaway `--plugin-dir`) fires while the
  prompt is up with `tool_name` and the whole `tool_input` (Bash,
  AskUserQuestion). Not used yet: a decision for the user (tall prompts).
- WebFetch is not a tool in this build (`ToolSearch found no such tool`), so
  its domain prompt could not be captured.
