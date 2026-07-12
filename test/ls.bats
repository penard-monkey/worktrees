#!/usr/bin/env bats
# Tests for cmd_ls (and the no-args → ls dispatch).

load 'helpers/common'

# ($REPO physicalized centrally in make_repo; the symlinked-path bug this file
# originally worked around is fixed in bin/worktrees — see misc.bats.)
setup() { common_setup; }

strip_ansi() { sed $'s/\033\[[0-9;]*m//g'; }

# stripped-output line containing pattern $1 (first match)
row_for() { printf '%s\n' "$output" | strip_ansi | grep -F "$1" | head -n 1; }

@test "ls: no .worktrees dir → No worktrees, exit 0" {
  run_wt ls
  [ "$status" -eq 0 ]
  [[ "$output" == *"No worktrees"* ]]
}

@test "ls: empty .worktrees dir → No worktrees, exit 0" {
  mkdir -p "$REPO/.worktrees"
  run_wt ls
  [ "$status" -eq 0 ]
  [[ "$output" == *"No worktrees"* ]]
}

@test "ls: one clean worktree row shows slug, branch, clean" {
  run_wt new feat-x --no-tmux
  [ "$status" -eq 0 ]
  run_wt ls
  [ "$status" -eq 0 ]
  local row; row="$(row_for 'feat-x')"
  [[ "$row" == feat-x* ]]                       # slug column
  # slug then branch cell both say feat-x (branch == slug)
  [[ "$row" =~ ^feat-x[[:space:]]+feat-x[[:space:]] ]]
  [[ "$row" == *clean* ]]
  [[ "$row" != *dirty* ]]
}

@test "ls: dirty worktree shows dirty" {
  run_wt new feat-x --no-tmux
  make_dirty "$REPO/.worktrees/feat-x"
  run_wt ls
  [ "$status" -eq 0 ]
  local row; row="$(row_for 'feat-x')"
  [[ "$row" == *dirty* ]]
  [[ "$row" != *clean* ]]
}

@test "ls: stale unregistered dir shows (rm <slug>) + stale, not main's branch" {
  mkdir -p "$REPO/.worktrees/stale-dir"
  run_wt ls
  [ "$status" -eq 0 ]
  local row; row="$(row_for 'stale-dir')"
  [[ "$row" == *"(rm stale-dir)"* ]]
  [[ "$row" == *stale* ]]
  # main checkout's branch name must NOT leak into the stale row
  [[ "$row" != *main* ]]
}

@test "ls: branch ≠ slug (via --name) tints the branch cell cyan" {
  run_wt new feat-y --name topic --no-tmux
  [ "$status" -eq 0 ]
  run_wt ls
  [ "$status" -eq 0 ]
  # raw output: cyan ANSI immediately wraps the padded branch text
  [[ "$output" == *$'\033[0;36mfeat-y'* ]]
  local row; row="$(row_for 'topic')"
  [[ "$row" =~ ^topic[[:space:]]+feat-y[[:space:]] ]]
}

@test "ls: detached HEAD worktree shows (detached)" {
  run_wt new feat-x --no-tmux
  git -C "$REPO/.worktrees/feat-x" checkout -q --detach
  run_wt ls
  [ "$status" -eq 0 ]
  local row; row="$(row_for 'feat-x')"
  [[ "$row" == *"(detached)"* ]]
}

@test "ls: sorts by last-commit recency, newest first" {
  # glob/discovery order is alphabetical (alpha, beta); give beta the NEWER
  # commit so a beta-first result proves the sort, not the tie-break.
  run_wt new alpha --no-tmux
  run_wt new beta --no-tmux
  echo newer >> "$REPO/.worktrees/beta/README.md"
  git -C "$REPO/.worktrees/beta" add -A
  GIT_COMMITTER_DATE='2040-01-01T00:00:00 +0000' \
    git -C "$REPO/.worktrees/beta" commit -qm newer
  run_wt ls
  [ "$status" -eq 0 ]
  local stripped line_beta line_alpha
  stripped="$(printf '%s\n' "$output" | strip_ansi)"
  line_beta="$(printf '%s\n' "$stripped"  | grep -n '^beta'  | head -n1 | cut -d: -f1)"
  line_alpha="$(printf '%s\n' "$stripped" | grep -n '^alpha' | head -n1 | cut -d: -f1)"
  [ -n "$line_beta" ] && [ -n "$line_alpha" ]
  [ "$line_beta" -lt "$line_alpha" ]
}

@test "ls: live tmux session shows ● in the row" {
  run_wt new feat-z --no-tmux
  # simulate a live session named after the worktree (prefix "repo")
  printf 'cwd=%s\ncmd0=x\n' "$REPO/.worktrees/feat-z" > "$TMUX_STATE/repo-feat-z"
  run_wt ls
  [ "$status" -eq 0 ]
  local row; row="$(row_for 'feat-z')"
  [[ "$row" == *"●"* ]]
}

@test "ls: no live session shows ○ in the row" {
  run_wt new feat-z --no-tmux
  run_wt ls
  [ "$status" -eq 0 ]
  local row; row="$(row_for 'feat-z')"
  [[ "$row" == *"○"* ]]
  [[ "$row" != *"●"* ]]
}

@test "ls: long branch name (>44 chars) truncated with ~, header aligned" {
  local b; b="$(printf 'x%.0s' {1..50})"   # 50-char branch name
  run_wt new "$b" --name longy --no-tmux
  [ "$status" -eq 0 ]
  run_wt ls
  [ "$status" -eq 0 ]
  local stripped header row
  stripped="$(printf '%s\n' "$output" | strip_ansi)"
  header="$(printf '%s\n' "$stripped" | grep -F 'SLUG' | head -n1)"
  row="$(printf '%s\n' "$stripped" | grep '^longy' | head -n1)"
  [ -n "$header" ] && [ -n "$row" ]
  # branch cell = first 43 chars + ~ (capped at 44)
  [[ "$row" == *"${b:0:43}~"* ]]
  [[ "$row" != *"$b"* ]]                     # full 50-char name never printed
  # alignment: maxslug=5 ("longy"), so BRANCH header + branch cell both at col 7,
  # and CREATED + the row's date both at col 7+44+2 = 53
  [ "${header:7:6}" = "BRANCH" ]
  [ "${row:7:44}" = "${b:0:43}~" ]
  [ "${header:53:7}" = "CREATED" ]
  [[ "${row:53:10}" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]]
}

@test "no-args dispatch runs ls" {
  run_wt new feat-x --no-tmux
  run_wt
  [ "$status" -eq 0 ]
  local row; row="$(row_for 'feat-x')"
  [[ "$row" =~ ^feat-x[[:space:]]+feat-x[[:space:]] ]]
  [[ "$output" == *SLUG* && "$output" == *BRANCH* ]]
}
