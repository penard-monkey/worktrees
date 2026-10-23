Captured with `tmux capture-pane -p` (and `-e -p` for the `.e.txt` files)
from Claude Code **2.1.309** (native install, macOS 27.0, Haiku 5.5,
`--permission-mode default`) on 2026-10-23, for `worktrees_core::prompt` and
serve's `agent_prompt` / `agent_answer` / `agent_send` rows.

The pane ran in a PRIVATE tmux server (`tmux -L wtfx-probe`, tmux 3.7b) in a
scratch git repository, `/private/tmp/claude-501/wtfx/repo`; no user session
was touched. The window was 120×40 unless a name says 80. Only trailing blank
rows were removed; every visible character is verbatim, the login banner
included.

The modals:

- `trust.txt`: the folder-trust prompt a new directory opens on. Its
  highlighted choice is **No, exit**; there are no numbers, and the footer is
  `Enter to confirm · Esc to cancel`. Claude writes NO probe file until it is
  answered, so nothing on disk says the session is waiting.
- `trust-yes.txt`: the same after one Down — the highlight on **Yes, I trust
  this folder**.
- `permission-bash.txt` (+ `.e.txt`): a Bash approval for `touch probe-a.txt`.
  Four choices, two of which widen what the agent may do (`always allow access
  to … from this project`, `switch to auto mode`). Footer
  `Esc to cancel · Tab to amend`. The probe file said `waiting`.
- `permission-bash-moved.txt`: the same after one Down. The FOOTER CHANGED to
  `Esc to cancel` with the highlight on choice 2 — a footer is matched on its
  stable head, never whole.
- `permission-bash-80.txt`: a Bash approval at 80 columns; choice 2 wraps onto
  a deeper-indented continuation row.
- `permission-write.txt`: a Write approval with its file preview; choice 2
  switches the session to accept-edits.
- `question.txt` (+ `.e.txt`): AskUserQuestion with one question. A full-width
  rule sits INSIDE the modal, above `4. Chat about this`, so a modal's top is
  the rule whose next row is not a choice. Footer
  `Enter to select · ↑/↓ to navigate · Esc to cancel`.
- `question-multi.txt`: the first of two questions — a `← ☐ Colour ☐ Size
  ✔ Submit →` tab bar and the footer `Enter to select · Tab/Arrow keys to
  navigate · Esc to cancel`.
- `question-review.txt`: the review screen after both answers (80×40). It has
  NO footer: the screen ends on its last choice, `2. Cancel`.

The composers (all `-e`, because only the styling tells Claude's dim
placeholder from typed text — `agent::draft_from_screen`):

- `composer-empty.e.txt`: idle, the box holding the dim `Try "…"` placeholder.
- `composer-typed.e.txt`: idle, `half typed draft, not sent` typed with
  `send-keys -l` and no Enter.
- `composer-after-turn.e.txt`: idle after a turn, the box empty (`❯ `).
- `composer-busy-empty.e.txt`: mid-turn (a `sleep 20` running), box empty.
- `composer-busy-typed.e.txt`: mid-turn, `queued thought` typed, no Enter.

What was measured while capturing, and the code relies on:

- Up/Down move the highlight and Enter confirms it, in every modal above; the
  highlight is `❯`. A digit also answers a numbered menu in one key (pressing
  `2` on the question submitted **Blue** at once), but the rows answer by
  moving and then confirming, so that a fresh capture can prove the chosen
  choice is the highlighted one before anything is confirmed.
- Answering `1. Yes` ran the command; `3. No` on the Write approval wrote
  `User rejected write to notes.txt`; the multi-question form moved to the
  next question on Enter and submitted from the review screen.
- `C-u` clears a typed draft back to the placeholder.
