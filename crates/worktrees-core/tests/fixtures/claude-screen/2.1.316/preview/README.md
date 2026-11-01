Captured with `tmux capture-pane -p` (and `-e -p` for the `.e.txt` files)
from Claude Code **2.1.316** (native install, macOS 27.0, Haiku 5.5,
`DISABLE_AUTOUPDATER=1`) on 2026-11-01, for `prompt::claude`'s reading of
AskUserQuestion when its options carry a `preview`, and for serve's
`agent_prompt` / `agent_answer` / `agent_answer_text` on it
(`fix/chat-question-preview`).

**Launched the way worktrees launches a lane.** A scratch repository under
`/private/tmp/claude-501/wtqp/`, this branch's CLI with its own
`XDG_CONFIG_HOME`, a PRIVATE tmux server (`TMUX_TMPDIR`, `$TMUX` unset),
`WORKTREES_PREFIX=wtqp`, every `CLAUDE_*` variable of the capturing session
unset. `worktrees new lane --ai claude --model haiku --no-attach` ran
exactly:

    claude --model haiku --plugin-dir <guidance> --name wtqp-lane --plugin-dir <claude-prompt>

The folder-trust prompt was answered, the permission mode set to manual
(shift+tab), and Claude asked in plain words for an AskUserQuestion with the
options and previews below. Trailing blank rows removed; every visible
character verbatim. The pane was resized with `tmux resize-window`.

## The question

`Banner copy`: "On a place's screen the banner says 1 place needs input,
which reads as if it means this place. What should it say instead?", three
single-select options, each with a description and a two-row (option 3:
one-row) preview: `Another place needs input (Recommended)`, `Name first`,
`Hide it on place screens` (`hook-banner.json` has the call).

What Claude draws, at every width measured:

- `☐ Banner copy` (a form: the tab bar), then the question, behind a dim
  `│` bar when it is long (`Which layout for the status file?` has none);
- the options in a LEFT column, numbered, the label WRAPPED onto rows of its
  own (`    (Recommended)`), and **no description**;
- a box in a RIGHT column, starting on option 1's row, with ONE option's
  preview: the option the `❯` marks. Its rows wrap at the box's width, which
  fits the widest preview line up to what the pane leaves; inner spacing is
  kept (`[    L    ]`, `    println!("hi");`), and a wrap drops the spaces it
  breaks at;
- `Notes: press n to add notes` under the box, in the same column;
- the list's rule, then `Chat about this`, UNNUMBERED;
- the key hints: `Enter to select · ↑/↓ to navigate · n to add notes · Esc to
  cancel` (a form adds `Tab to switch questions`; an open notes field adds
  `ctrl+g to edit in Vim`), wrapped onto a second row when the pane is
  narrower than they are;
- a named Claude's name tag as the screen's last row.

Screens (`wide-*` at 160×45, `80-*` at 80×24):

- `wide-1`, `wide-2`, `wide-3` (+ `.e`): each option highlighted; the box
  follows the highlight.
- `wide-chat` (+ `.e`): Down from option 3: `❯ Chat about this`, AND option
  3 keeps its `❯`, styled exactly as before, and the box keeps its preview.
  Down from there does nothing; Up goes back to option 3.
- `wide-2-notes-open` (+ `.e`): `n` on option 2: `Notes: Add notes on this
  design…` (dim), and the key hints gain `ctrl+g to edit in Vim`.
- `wide-2-notes-typed` (+ `.e`): `keep the place name bold` pasted
  (`tmux paste-buffer -p`, as `tmux::paste_into` does): the text shows.
  While the field is open, Up and Down do NOT move the highlight.
- `wide-notes-closed` (+ `.e`): on another copy of the question, `n`, a
  pasted `first option note`, then Esc: the field closes (italic dim, no
  `ctrl+g` hint) and KEEPS its text.
- `wide-notes-closed-2` (+ `.e`): then Down: the note stays with the
  QUESTION — one notes field per question, not per option.
- `wide-answered` (+ `.e`): what the pane shows after Enter in the notes
  field: `⏺ User answered Claude's questions: ⎿ · … → Name first`.
- `80-1`, `80-2`, `80-3`, `80-chat` (+ `.e`): the same at 80×24, the size a
  detached lane gets. The box has no room: Claude CUTS its tail —
  `├─── ✂ ─── 3 lines hidden ───┤` — and the long first preview row wraps
  inside the box (`3 lines` counts the box's rows).
- `80-3-notes-open` (+ `.e`): `n` at 80 columns: the key hints wrap onto a
  second row (`… · Esc` / `to cancel`).
- `80-3-notes-long` (+ `.e`): a long note pasted: it wraps under itself,
  indented past `Notes: `.
- `80-notes-long-closed`, `80-notes-long-closed-2` (+ `.e`): Esc on that
  long note, then Up. **Claude re-lays the columns out to fit the note and
  draws option 1 askew** (`    Another place` / ` 1.needs input` /
  `   (Recommended)`). Not a prompt this reader reads.
- `80x40-1` (+ `.e`): 80×40, the box whole, its rows wrapped.
- `60-1` (+ `.e`): 60×34: the key hints wrap (`… · Esc to` / `cancel`).
- `45-1` (+ `.e`): 45×34: the box's own rows break (`├─── ✂ ─── 9 lines` /
  `hidden ┤`) and so does the notes row. Not read.

`Layout` (`hook-long.json`): option 1 a twelve-row preview, option 2 code
with inner indentation, option 3 NO preview.

- `long-1`, `long-2`, `long-3` (+ `.e`): 160×45; option 3 highlighted shows
  Claude's own `No preview available`, in plain (not dim) styling.
- `long-80-1` (+ `.e`): 80×24: one row, `✂ 11 lines hidden`.

A form (`hook-two.json`): `Colour` (Red / Blue) and `Size` (Small / Large),
both with previews.

- `two-1` (+ `.e`): `←  ☐ Colour  ☐ Size  ✔ Submit  →` heads the same
  layout.
- `two-2`, `two-2-large` (+ `.e`): after Enter on question 1 (with a note
  typed and the highlight never moved: see below), question 2, its notes
  empty, then Large highlighted.
- `two-review` (+ `.e`): the review: `Which colour? → (notes only)`.

- `multi` (+ `.e`, `hook-multi.json`): `multiSelect: true` with previews:
  Claude IGNORES the previews and draws its ordinary multi-select
  (descriptions, `[ ]` boxes, `Type something`, `Submit`).

Hook inputs (`hook-*.json`, in the PermissionRequest stdin's shape): the
`tool_input` of each question as Claude's transcript recorded the call. For
`Banner copy` the PermissionRequest hook's own record (`pending`) was read
while the question was up and is the same input.

## What was measured, and the code relies on

- **The PermissionRequest hook fires for AskUserQuestion** while the
  question is up (worktrees' `claude-prompt` plugin saved it), with the
  whole call: every option's `label`, `description` and `preview`.
- The transcript's answer: `toolUseResult.answers[<question>]` is the label,
  and `toolUseResult.annotations[<question>]` is `{preview, notes}` — the
  chosen option's preview, and the note when there is one. The tool result
  Claude reads: `"…"="Name first" selected preview:\n<preview> notes: keep
  the place name bold`.
- **Enter on an option** (list focused) answers it, with the question's
  note if one was typed (open or closed since): `Another place needs input
  (Recommended)` + `t2 note`.
- **Enter in the OPEN notes field** sends the option the highlight was last
  MOVED to, with the note; when the highlight never moved since the
  question appeared, NO option: `answers` = `(notes only)`, the tool result
  `(no option selected) notes: …` (measured four ways: no move → `(notes
  only)`, twice, single question and form; a move onto the option, or away
  and back, → that option, three times).
- **Esc** with the notes field open closes it and keeps its text; Esc with
  it closed cancels the question (`User declined to answer questions`,
  Claude stops and waits).
- **`Chat about this`** is a refusal: `The user wants to clarify these
  questions … Start by asking them what they would like to clarify`, with
  `User notes: …` when a note was typed. Claude ends its turn and asks, and
  its composer is empty — the next thing is the user's message.
- `n` on an option opens the notes field; the arrows and digits do not reach
  the list while it is open.
