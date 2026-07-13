#!/usr/bin/env bats
# Tests for `worktrees new` / `co` (cmd_new).

load 'helpers/common'

setup() {
  common_setup
  # ($REPO physicalized centrally in make_repo; symlinked-path handling is
  #  fixed in bin/worktrees via pwd -P — regression test in misc.bats.)
}

# ── branch creation / base resolution ────────────────────────────────────────

@test "new: no base → new branch off origin/main (not local main)" {
  # Make local main AHEAD of origin/main so the two are distinguishable.
  git -C "$REPO" commit -q --allow-empty -m ahead
  local origin_sha local_sha
  origin_sha="$(git -C "$REPO" rev-parse origin/main)"
  local_sha="$(git -C "$REPO" rev-parse main)"
  [ "$origin_sha" != "$local_sha" ]

  run_wt new feat-x --no-tmux
  [ "$status" -eq 0 ]
  [[ "$output" == *"Creating new branch 'feat-x' off 'origin/main'"* ]]
  [ -d "$REPO/.worktrees/feat-x" ]
  [ "$(git -C "$REPO/.worktrees/feat-x" rev-parse HEAD)" = "$origin_sha" ]
  [ "$(git -C "$REPO/.worktrees/feat-x" rev-parse --abbrev-ref HEAD)" = "feat-x" ]
}

@test "new: explicit local-only base → new branch starts at that base" {
  local c
  c="$(git -C "$REPO" commit-tree "HEAD^{tree}" -p HEAD -m basec)"
  git -C "$REPO" branch devbase "$c"

  run_wt new feat-y devbase --no-tmux
  [ "$status" -eq 0 ]
  [[ "$output" == *"Creating new branch 'feat-y' off 'devbase'"* ]]
  [ "$(git -C "$REPO/.worktrees/feat-y" rev-parse HEAD)" = "$c" ]
}

@test "new: explicit base → origin/<base> preferred over local <base> when both exist" {
  local c2
  c2="$(git -C "$REPO" commit-tree "HEAD^{tree}" -p HEAD -m c2)"
  git -C "$REPO" branch base2                       # local base2 @ old tip
  git -C "$REPO" push -q origin "$c2:refs/heads/base2"   # origin base2 @ c2

  run_wt new feat-z base2 --no-tmux
  [ "$status" -eq 0 ]
  [[ "$output" == *"off 'origin/base2'"* ]]
  [ "$(git -C "$REPO/.worktrees/feat-z" rev-parse HEAD)" = "$c2" ]
  # explicitly NOT the (stale) local base2
  [ "$(git -C "$REPO" rev-parse base2)" != "$c2" ]
}

@test "new: existing local branch → checked out, sha unchanged, no new branch" {
  make_local_branch feat-local
  local sha nbranches
  sha="$(git -C "$REPO" rev-parse feat-local)"
  nbranches="$(git -C "$REPO" branch --format='%(refname:short)' | wc -l)"

  run_wt new feat-local --no-tmux
  [ "$status" -eq 0 ]
  [[ "$output" == *"exists locally — checking it out"* ]]
  [ "$(git -C "$REPO/.worktrees/feat-local" rev-parse HEAD)" = "$sha" ]
  [ "$(git -C "$REPO/.worktrees/feat-local" rev-parse --abbrev-ref HEAD)" = "feat-local" ]
  [ "$(git -C "$REPO" branch --format='%(refname:short)' | wc -l)" -eq "$nbranches" ]
}

@test "new: remote-only branch → fetched, checked out with tracking upstream" {
  make_remote_branch rb2

  run_wt new rb2 --no-tmux
  [ "$status" -eq 0 ]
  [[ "$output" == *"Fetching origin/rb2"* ]]
  [[ "$output" == *"Checking out remote branch origin/rb2"* ]]
  [ "$(git -C "$REPO/.worktrees/rb2" rev-parse --abbrev-ref '@{u}')" = "origin/rb2" ]
}

@test "new: remote-only branch + --no-fetch → falls through to new branch off base" {
  make_remote_branch rb1

  run_wt new rb1 --no-fetch --no-tmux
  [ "$status" -eq 0 ]
  [[ "$output" != *"Fetching"* ]]
  [[ "$output" == *"Creating new branch 'rb1' off 'origin/main'"* ]]
  # origin was never contacted for rb1 — no remote-tracking ref appeared
  ! git -C "$REPO" show-ref --verify --quiet refs/remotes/origin/rb1
  # and the checkout does NOT track origin/rb1 (git's autoSetupMerge default
  # gives the new branch origin/main as upstream — start-point tracking)
  run git -C "$REPO/.worktrees/rb1" rev-parse --abbrev-ref '@{u}'
  [[ "$output" != "origin/rb1" ]]
}

@test "new: origin/feat/x argument → origin/ prefix stripped, remote branch tracked" {
  make_remote_branch feat/x

  run_wt new origin/feat/x --no-tmux
  [ "$status" -eq 0 ]
  [ -d "$REPO/.worktrees/feat-x" ]
  [ "$(git -C "$REPO/.worktrees/feat-x" rev-parse --abbrev-ref HEAD)" = "feat/x" ]
  [ "$(git -C "$REPO/.worktrees/feat-x" rev-parse --abbrev-ref '@{u}')" = "origin/feat/x" ]
}

@test "new: slashes in branch slug to dashes → feat/foo lives in .worktrees/feat-foo" {
  run_wt new feat/foo --no-tmux
  [ "$status" -eq 0 ]
  [ -d "$REPO/.worktrees/feat-foo" ]
  [ "$(git -C "$REPO/.worktrees/feat-foo" rev-parse --abbrev-ref HEAD)" = "feat/foo" ]
}

# ── --name ───────────────────────────────────────────────────────────────────

@test "new: --name topic → dir .worktrees/topic with the given branch + session repo-topic" {
  run_wt new feat/bar --name topic
  [ "$status" -eq 0 ]
  [ -d "$REPO/.worktrees/topic" ]
  [ ! -e "$REPO/.worktrees/feat-bar" ]
  [ "$(git -C "$REPO/.worktrees/topic" rev-parse --abbrev-ref HEAD)" = "feat/bar" ]
  tmux_session_exists repo-topic
}

@test "new: --name with missing value → error exit 1" {
  run_wt new feat-a --name
  [ "$status" -eq 1 ]
  [[ "$output" == *"--name needs a value"* ]]
  [ ! -e "$REPO/.worktrees/feat-a" ]
}

@test "new: --name followed by a flag → error exit 1 (value not swallowed)" {
  run_wt new feat-a --name --no-tmux
  [ "$status" -eq 1 ]
  [[ "$output" == *"--name needs a value (got '--no-tmux')"* ]]
}

@test "new: --ai with missing value → error exit 1" {
  run_wt new feat-a --ai
  [ "$status" -eq 1 ]
  [[ "$output" == *"--ai needs a value"* ]]
}

# ── branch→place redirect ────────────────────────────────────────────────────

@test "new: branch living in a differently-named worktree → reuses the holder" {
  run_wt new b1 --no-tmux
  [ "$status" -eq 0 ]
  run_wt switch b1 b2 --no-fetch          # place 'b1' now holds branch 'b2'
  [ "$status" -eq 0 ]

  run_wt new b2 --no-tmux
  [ "$status" -eq 0 ]
  [[ "$output" == *"already lives in worktree 'b1'"* ]]
  [ ! -e "$REPO/.worktrees/b2" ]
  [ "$(git -C "$REPO/.worktrees/b1" rev-parse --abbrev-ref HEAD)" = "b2" ]
}

@test "new: redirect + --name conflict → error \"can't also put it\" exit 1" {
  run_wt new b1 --no-tmux
  [ "$status" -eq 0 ]
  run_wt switch b1 b2 --no-fetch
  [ "$status" -eq 0 ]

  run_wt new b2 --name other --no-tmux
  [ "$status" -eq 1 ]
  [[ "$output" == *"can't also put it"* ]]
  [ ! -e "$REPO/.worktrees/other" ]
}

# ── existing dirs / reuse / auto-switch ──────────────────────────────────────

@test "new: unregistered dir already at target → error exit 1" {
  mkdir -p "$REPO/.worktrees/stale"

  run_wt new stale --no-tmux
  [ "$status" -eq 1 ]
  [[ "$output" == *"not a registered worktree"* ]]
}

@test "new: same branch again → reusing, no reinstall" {
  add_lockfile pnpm-lock.yaml
  run_wt new feat-r --no-tmux
  [ "$status" -eq 0 ]

  run_wt new feat-r --no-tmux
  [ "$status" -eq 0 ]
  [[ "$output" == *"reusing"* ]]
  [[ "$output" != *"pnpm install"* ]]     # no install hint on reuse
  [ ! -s "$BATS_TEST_TMPDIR/pnpm.log" ]   # pnpm shim never invoked
}

@test "new: worktree exists on another branch → auto-switches to requested branch" {
  run_wt new feat-s --no-tmux
  [ "$status" -eq 0 ]
  run_wt switch feat-s other-b --no-fetch
  [ "$status" -eq 0 ]
  [ "$(git -C "$REPO/.worktrees/feat-s" rev-parse --abbrev-ref HEAD)" = "other-b" ]

  run_wt new feat-s --no-tmux
  [ "$status" -eq 0 ]
  [[ "$output" == *"switching to 'feat-s'"* ]]
  [ "$(git -C "$REPO/.worktrees/feat-s" rev-parse --abbrev-ref HEAD)" = "feat-s" ]
}

@test "new: worktree exists on another branch + dirty → refuses, exit 1" {
  run_wt new feat-s --no-tmux
  [ "$status" -eq 0 ]
  run_wt switch feat-s other-b --no-fetch
  [ "$status" -eq 0 ]
  make_dirty "$REPO/.worktrees/feat-s"

  run_wt new feat-s --no-tmux
  [ "$status" -eq 1 ]
  [[ "$output" == *"Refusing to switch"* ]]
  [ "$(git -C "$REPO/.worktrees/feat-s" rev-parse --abbrev-ref HEAD)" = "other-b" ]
}

# ── arg validation ───────────────────────────────────────────────────────────

@test "new: no branch arg → error exit 1" {
  run_wt new
  [ "$status" -eq 1 ]
  [[ "$output" == *"Branch name required"* ]]
}

@test "new: unknown flag → error exit 1" {
  run_wt new feat-q --bogus
  [ "$status" -eq 1 ]
  [[ "$output" == *"Unknown flag: --bogus"* ]]
}

@test "new: too many args → error exit 1" {
  run_wt new a b c
  [ "$status" -eq 1 ]
  [[ "$output" == *"Too many args: c"* ]]
}

# ── tmux / AI pane behavior ──────────────────────────────────────────────────

@test "new: --no-tmux → no tmux calls, prints cd hint" {
  run_wt new feat-nt --no-tmux
  [ "$status" -eq 0 ]
  [[ "$output" == *"cd $REPO/.worktrees/feat-nt"* ]]
  [ ! -s "$TMUX_LOG" ]
}

@test "new: tmux absent → warns and still creates the worktree" {
  # Symlink-built PATH without tmux — "no tmux in /usr/bin" only holds on
  # macOS; ubuntu's apt tmux IS /usr/bin/tmux, and hitting the real binary
  # daemonizes a server that hangs bats (holds its FDs).
  install_no_tmux_path

  run_wt new feat-notmux
  [ "$status" -eq 0 ]
  [[ "$output" == *"tmux not found"* ]]
  [ -d "$REPO/.worktrees/feat-notmux" ]
  [[ "$output" == *"cd $REPO/.worktrees/feat-notmux"* ]]
}

@test "new: WORKTREES_AI_CMD=codex → pane 0 command runs codex" {
  install_fake_cmd codex
  export WORKTREES_AI_CMD=codex

  run_wt new feat-c
  [ "$status" -eq 0 ]
  [[ "$(tmux_pane0_cmd repo-feat-c)" == *codex* ]]
}

@test "new: -r/--resume → pane 0 command contains 'fake-ai -r'" {
  run_wt new feat-rr -r
  [ "$status" -eq 0 ]
  [[ "$(tmux_pane0_cmd repo-feat-rr)" == *"fake-ai -r"* ]]
}

# ── install-pane detection ───────────────────────────────────────────────────

@test "new: pnpm-lock.yaml → pane 1 runs pnpm install" {
  add_lockfile pnpm-lock.yaml
  run_wt new feat-pl
  [ "$status" -eq 0 ]
  [[ "$(tmux_pane1_cmd repo-feat-pl)" == *"pnpm install"* ]]
}

@test "new: yarn.lock → pane 1 runs yarn" {
  add_lockfile yarn.lock
  run_wt new feat-yl
  [ "$status" -eq 0 ]
  [[ "$(tmux_pane1_cmd repo-feat-yl)" == *yarn* ]]
}

@test "new: no lockfile → pane 1 has no install command" {
  run_wt new feat-nl
  [ "$status" -eq 0 ]
  local p1; p1="$(tmux_pane1_cmd repo-feat-nl)"
  [[ "$p1" != *install* ]]
  [[ "$p1" != *yarn* ]]
}

@test "new: --no-install → pane 1 has no install despite lockfile" {
  add_lockfile pnpm-lock.yaml
  run_wt new feat-ni --no-install
  [ "$status" -eq 0 ]
  [[ "$(tmux_pane1_cmd repo-feat-ni)" != *install* ]]
}

@test "new: --no-attach → session ready detached, no attach/switch-client" {
  run_wt new feat-na --no-attach
  [ "$status" -eq 0 ]
  [[ "$output" == *"detached"* ]]
  tmux_session_exists repo-feat-na
  ! grep -qE 'attach|switch-client' "$TMUX_LOG"
}

# ── co alias ─────────────────────────────────────────────────────────────────

@test "co: alias behaves like new (remote branch checkout with tracking)" {
  make_remote_branch rb3
  run_wt co rb3 --no-tmux
  [ "$status" -eq 0 ]
  [[ "$output" == *"Checking out remote branch origin/rb3"* ]]
  [ -d "$REPO/.worktrees/rb3" ]
  [ "$(git -C "$REPO/.worktrees/rb3" rev-parse --abbrev-ref '@{u}')" = "origin/rb3" ]
}
