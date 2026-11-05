Captured with `tmux capture-pane -e -p` from Claude Code 2.1.318 on macOS,
2026-11-04, at 120×40 unless noted. Launched the way worktrees launches it, by
`worktrees new`/`open --ai claude --model haiku` on a private tmux server: the
three `--plugin-dir`s and `--name` are all there (hence the name in the rule above
the box). The directory was a scratch repo under `/private/tmp`.

Each draft went in by `tmux paste-buffer -p`, one line per paste unless noted:

- `empty`: the box after a fresh launch.
- `chunked`: `first line`, `second line`, a blank line, `fourth line`.
- `tall`: 40 rows. The box scrolls and shows the last 15.
- `rule`: a line of `─` inside the message: not the box's edge.
- `long-line`: 2000 `w`, in pastes of 512.
- `placeholder`: five lines as ONE paste: `[Pasted text #19 +4 lines]`, with
  `paste again to expand` in place of the status row.
- `typed-after`: `first line`, `second line`, then `send-keys -l` at the terminal.
- `wrapped`: one 134-character line of prose as one paste, at 80×24: it wraps at a
  word, here after 76 of its `cols - 4` columns.
- `tokens-40`: four 40-character tokens as one paste, at 80×24: a row each, 42 of
  its 76 columns, though a row could hold nearly two.
- `leading-newline`: `\nabc` then `\ndef` pasted into an empty box, at 80×24: a blank
  first row, then one row each. A paste into a box just emptied starts like this.
- `truncated`: 15 KB as it crossed 10,000 characters: the middle folded into
  `[...Truncated text #9 +90 lines...]`, caught by polling. Next draws scroll it off.
- `after-collapsed`: a turn running 1.5 s after Enter on a collapsed paste, resized
  from 120×11, with `paste again to expand` hiding `esc to interrupt`.

## What Claude does with a paste: undocumented, so re-measure on upgrade

`docs/claude-paste-manual-checks.md` says how. On 2.1.318, with these flags:

| What | Measured | Where it matters |
|---|---|---|
| One paste collapses to a placeholder | 4+ lines (3 stay whole), or between 800 and 850 characters | `prompt::CHUNK_BYTES` |
| A collapsed paste is recorded as | `\n\n<pasted_content id="807e">\n…\n</pasted_content id="807e">\n` | `turns::pasted_body` |
| Separate pastes | stay separate, with no gap between them | `prompt::claude_chunks` |
| A paste with one line break | stays literal at 11+ rows; a placeholder at 10 and under | `prompt::CHUNK_MIN_ROWS` |
| A tall draft's window | below, every height measured | `prompt::box_window` |
| A long token in a line | moves whole to the next row; a word wider than a row breaks at `/` or at the row's end | `prompt::Pace::of` |
| The box wraps at | `cols - 4`, at a word where it can, so a row can hold less | `prompt::Pace::of` |
| Fewer rows than the window | the box shows ALL it holds, so its start must be the message's | `prompt::drew` |
| Enter at the terminal mid-send | submits the fragment; the next paste lands in the empty box | `prompt::Drew::Lost` |
| A draft past ~10,000 characters | its middle folds into `[...Truncated text …]`, and it submits whole | `prompt::drew` |
| `paste again to expand` | ~8 s after a collapsed paste, over the whole status row | `prompt::turn_shown` |
| A later line starting `/ ! #` | stays text | `harness::typed_message` |
| A later line `@<real file>` | attaches the file, as `@` mid-line does | `harness::typed_message` |
| Trailing spaces inside a line | recorded as typed | `chat.ts::isRecordOf` |

## A tall draft's window, by pane height

70 short lines by chunks, at 120 columns, counting the box's rows. At 33 heights the
measure matched `3` up to 16 rows and `rows / 2 - 5` after, which is `prompt::box_window`:

| Rows | 12–17 | 18, 19 | 20, 21 | 22, 23 | 24, 25 | 26, 27 | 28, 29 | 30, 31 | 32, 33 | 34, 35 | 36, 37 | 38, 39 | 40 | 44 | 50 | 60 | 70 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| Window | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 | 14 | 15 | 17 | 20 | 25 | 30 |
