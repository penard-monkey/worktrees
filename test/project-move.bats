#!/usr/bin/env bats
# A project whose folder was renamed outside worktrees, through the compiled
# binary: `projects rm` names the sessions it would leave running, `projects
# add` adopts a moved project's sessions under the adoption rule, and doctor's
# session-drift remedy is that rename, never a close.
#
# The fake tmux has one pane per session, cwd'd where its state file says.
# String checks are `grep -qF` on a herestring, never a mid-test `[[ ]]`
# (which asserts nothing in bats; AGENTS.md).

load 'helpers/common'

setup() { common_setup; }

# A planted session: name, cwd, and optionally the program its pane runs.
plant() {
  printf 'cwd=%s\ncmd0=bash\n' "$2" > "$TMUX_STATE/$1"
  if [ -n "${3:-}" ]; then printf '%s' "$3" > "$TMUX_STATE/$1.cmd"; fi
}

@test "projects rm with no live session says nothing new" {
  run_wt -C "$BATS_TEST_TMPDIR" projects add "$REPO"
  run_wt -C "$BATS_TEST_TMPDIR" projects rm repo
  [ "$status" -eq 0 ]
  grep -qF "unregistered 'repo'" <<<"$output"
  ! grep -qF "live session" <<<"$output" || false
  ! grep -qF "left running" <<<"$output" || false
}

@test "projects rm names what it would leave running, and keeps it unless told" {
  run_wt new lane --no-install --no-attach
  [ "$status" -eq 0 ]
  plant "repo-(main)" "$REPO" claude
  run_wt -C "$BATS_TEST_TMPDIR" projects add "$REPO"

  # Asked; an EOF is a "no": named, and kept.
  run_wt -C "$BATS_TEST_TMPDIR" projects rm repo
  [ "$status" -eq 0 ]
  grep -qF "'repo' has 2 live session(s):" <<<"$output"
  grep -qF "(main)  repo-(main)  (Claude running)" <<<"$output"
  grep -qF "lane  repo-lane" <<<"$output"
  grep -qF "Close them before unregistering?" <<<"$output"
  grep -qF "2 session(s) left running" <<<"$output"
  ! grep -qF "pass --close" <<<"$output" || false
  [ -f "$TMUX_STATE/repo-(main)" ]
  [ -f "$TMUX_STATE/repo-lane" ]
  ! grep -qF "kill-session" "$TMUX_LOG" || false

  # A caller that cannot answer: not asked, kept, and the flags are offered.
  run_wt -C "$BATS_TEST_TMPDIR" projects add "$REPO"
  WORKTREES_NO_PROMPT=1 run_wt -C "$BATS_TEST_TMPDIR" projects rm repo
  [ "$status" -eq 0 ]
  ! grep -qF "Close them before unregistering?" <<<"$output" || false
  grep -qF "pass --close" <<<"$output"
  [ -f "$TMUX_STATE/repo-(main)" ]

  # --keep: the same, without the note.
  run_wt -C "$BATS_TEST_TMPDIR" projects add "$REPO"
  run_wt -C "$BATS_TEST_TMPDIR" projects rm repo --keep
  [ "$status" -eq 0 ]
  grep -qF "left running" <<<"$output"
  ! grep -qF "pass --close" <<<"$output" || false
  [ -f "$TMUX_STATE/repo-(main)" ]

  # --close: through close's own path, then unregistered.
  run_wt -C "$BATS_TEST_TMPDIR" projects add "$REPO"
  run_wt -C "$BATS_TEST_TMPDIR" projects rm repo --close
  [ "$status" -eq 0 ]
  grep -qF "closed tmux repo-(main)" <<<"$output"
  grep -qF "closed tmux repo-lane" <<<"$output"
  grep -qF "unregistered 'repo'" <<<"$output"
  ! grep -qF "left running" <<<"$output" || false
  [ ! -f "$TMUX_STATE/repo-(main)" ]
  [ ! -f "$TMUX_STATE/repo-lane" ]

  run_wt -C "$BATS_TEST_TMPDIR" projects add "$REPO"
  run_wt -C "$BATS_TEST_TMPDIR" projects rm repo --close --keep
  [ "$status" -eq 1 ]
  grep -qF "contradict each other" <<<"$output"

  # A "y" at the prompt closes them too.
  plant "repo-(main)" "$REPO" claude
  run_wt -C "$BATS_TEST_TMPDIR" projects add "$REPO"
  wt_answer y -C "$BATS_TEST_TMPDIR" projects rm repo
  [ "$status" -eq 0 ]
  grep -qF "closed tmux repo-(main)" <<<"$output"
  [ ! -f "$TMUX_STATE/repo-(main)" ]
}

@test "projects add adopts a moved project's sessions, and doctor names that rename" {
  run_wt new lane --no-install --no-attach
  [ "$status" -eq 0 ]
  # The folder was `old`: its sessions still carry that prefix, cwd'd here.
  mv "$TMUX_STATE/repo-lane" "$TMUX_STATE/old-lane"
  plant "old-(main)" "$REPO" claude
  plant "old-lane~agent~codex" "$REPO/.worktrees/lane" codex

  run_wt doctor
  grep -qF 'live tmux session is `old-(main)`' <<<"$output"
  grep -qF "Rename it, closing nothing, with: worktrees projects add '$REPO'" <<<"$output"
  ! grep -qF "worktrees close" <<<"$output" || false

  run_wt -C "$BATS_TEST_TMPDIR" projects add "$REPO"
  [ "$status" -eq 0 ]
  grep -qF "registered 'repo'" <<<"$output"
  grep -qF "renamed tmux old-(main) → repo-(main) (nothing was closed)" <<<"$output"
  grep -qF "renamed tmux old-lane → repo-lane" <<<"$output"
  grep -qF "renamed tmux old-lane~agent~codex → repo-lane~agent~codex" <<<"$output"
  [ -f "$TMUX_STATE/repo-(main)" ] && [ -f "$TMUX_STATE/repo-(main).cmd" ]
  [ -f "$TMUX_STATE/repo-lane" ] && [ -f "$TMUX_STATE/repo-lane~agent~codex" ]
  [ ! -f "$TMUX_STATE/old-(main)" ]
  ! grep -qF "kill-session" "$TMUX_LOG" || false

  # Again: idempotent, and it says there was nothing to do.
  run_wt -C "$BATS_TEST_TMPDIR" projects add "$REPO"
  [ "$status" -eq 0 ]
  grep -qF "already registered as 'repo'" <<<"$output"
  grep -qF "nothing to adopt" <<<"$output"

  run_wt doctor
  ! grep -qF "live tmux session is" <<<"$output" || false
}

@test "adoption leaves a hand-named session, another project's, and a taken name alone" {
  run_wt new lane --no-install --no-attach
  [ "$status" -eq 0 ]
  local beta="$BATS_TEST_TMPDIR/beta"
  git init -q "$beta"
  run_wt -C "$BATS_TEST_TMPDIR" projects add "$beta"
  plant "scratch" "$REPO" claude                          # named by hand
  plant "beta-(main)" "$REPO"                             # another registered project's prefix
  plant "old-lane" "$REPO/.worktrees/lane"                # repo-lane already holds its name

  run_wt -C "$BATS_TEST_TMPDIR" projects add "$REPO"
  [ "$status" -eq 0 ]
  ! grep -qF "renamed tmux" <<<"$output" || false
  [ -f "$TMUX_STATE/scratch" ] && [ -f "$TMUX_STATE/beta-(main)" ] && [ -f "$TMUX_STATE/old-lane" ]
  [ ! -f "$TMUX_STATE/repo-(main)" ]

  # doctor still reports the drift, and offers no rename for it.
  run_wt doctor
  grep -qF "live tmux session is" <<<"$output"
  grep -qF "adopted by pane cwd and left as it is" <<<"$output"
  ! grep -qF "worktrees projects add" <<<"$output" || false
}
