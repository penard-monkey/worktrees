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

@test "new: repo with no commits → refused with the first-commit remedy, nothing created" {
  local empty="$BATS_TEST_TMPDIR/fresh"
  git init -q "$empty"
  empty="$(cd "$empty" && pwd -P)"

  run_wt -C "$empty" new feat-x --no-tmux
  [ "$status" -eq 1 ]
  [[ "$output" == *"no commits yet"* ]]
  [[ "$output" == *"commit --allow-empty"* ]]
  # git's own riddle never reaches the user, and no half-made worktree is left
  [[ "$output" != *"not a valid object name"* ]]
  [ ! -e "$empty/.worktrees/feat-x" ]

  # and once a commit exists the same command works
  git -C "$empty" commit -q --allow-empty -m init
  run_wt -C "$empty" new feat-x --no-tmux
  [ "$status" -eq 0 ]
  [ -d "$empty/.worktrees/feat-x" ]
}

@test "new: remote-only branch → fetched, checked out with tracking upstream" {
  make_remote_branch rb2

  run_wt new rb2 --no-tmux
  [ "$status" -eq 0 ]
  # One fetch of the whole remote, not a per-ref one: the old pair of targeted
  # fetches spent a doomed round trip asking for a branch that usually does not
  # exist yet. What the user needs to see is unchanged — the branch was found.
  [[ "$output" == *"Fetching origin..."* ]]
  [[ "$output" == *"Checking out remote branch origin/rb2"* ]]
  [ "$(git -C "$REPO/.worktrees/rb2" rev-parse --abbrev-ref '@{u}')" = "origin/rb2" ]
}

# The fetch is GUARDED by the tracking ref, not unconditional. When
# origin/<branch> is already on disk there is nothing left to ask the remote, and
# the old code took this path fully offline — a floor of ZERO round trips that
# collapsing the two fetches into one must not quietly raise to one. Enforced by
# pointing origin at a path that cannot answer: any network attempt fails loudly.
@test "new: tracking ref already present → no fetch at all (works with origin unreachable)" {
  make_remote_branch rb3
  git -C "$REPO" fetch -q origin "refs/heads/rb3:refs/remotes/origin/rb3"
  git -C "$REPO" remote set-url origin /nonexistent/origin.git

  run_wt new rb3 --no-tmux
  [ "$status" -eq 0 ]
  [[ "$output" != *"Fetching"* ]]
  [[ "$output" == *"Checking out remote branch origin/rb3"* ]]
  [ "$(git -C "$REPO/.worktrees/rb3" rev-parse --abbrev-ref '@{u}')" = "origin/rb3" ]
}

# The deliberate narrowing that came with the single fetch: it honours the repo's
# CONFIGURED refspec, so a clone restricted to one branch no longer materializes
# an unfetched remote branch the way an explicit `refs/heads/<x>:…` forced it to.
# It falls through to a new local branch off the base instead of failing.
@test "new: restricted refspec → remote-only branch is not materialized, falls through to base" {
  make_remote_branch rb4
  # what `git clone --single-branch` leaves behind: origin only ever offers main
  git -C "$REPO" config remote.origin.fetch "+refs/heads/main:refs/remotes/origin/main"

  run_wt new rb4 --no-tmux
  [ "$status" -eq 0 ]
  [[ "$output" == *"Fetching origin..."* ]]
  # the fetch ran but this clone's refspec does not carry rb4
  ! git -C "$REPO" show-ref --verify --quiet refs/remotes/origin/rb4
  [[ "$output" == *"Creating new branch 'rb4' off 'origin/main'"* ]]
  [ -d "$REPO/.worktrees/rb4" ]
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

@test "new (default) splits a spare shell into pane 1" {
  run_wt new feat-sp
  [ "$status" -eq 0 ]
  [ -n "$(tmux_pane1_cmd repo-feat-sp)" ]   # split-window ran → pane 1 exists
  grep -q 'split-window' "$TMUX_LOG"
}

@test "new --no-spare: single pane, no split-window (the app's path)" {
  run_wt new feat-ns --no-spare
  [ "$status" -eq 0 ]
  tmux_session_exists repo-feat-ns
  [[ "$(tmux_pane0_cmd repo-feat-ns)" == *fake-ai* ]]
  [ -z "$(tmux_pane1_cmd repo-feat-ns)" ]   # no pane 1
  ! grep -q 'split-window' "$TMUX_LOG"
}

@test "new --no-spare: a detected install is hinted, never silently dropped" {
  add_lockfile pnpm-lock.yaml
  run_wt new feat-nsi --no-spare
  [ "$status" -eq 0 ]
  [[ "$output" == *"then: pnpm install"* ]]
  [ -z "$(tmux_pane1_cmd repo-feat-nsi)" ]
}

@test "new: --no-attach → session ready detached, no attach/switch-client" {
  run_wt new feat-na --no-attach
  [ "$status" -eq 0 ]
  [[ "$output" == *"detached"* ]]
  tmux_session_exists repo-feat-na
  ! grep -qE 'attach|switch-client' "$TMUX_LOG"
}

# ── agents: --name and --brief ───────────────────────────────────────────────

@test "new: claude is launched with --name <tmux session>; another AI tool is not" {
  # The FULL session name, not the slug: claude's cross-session messaging finds
  # a session by name, and this is the one string the orchestrator already has.
  WORKTREES_AI_CMD=claude run_wt new feat-nm
  [ "$status" -eq 0 ]
  [[ "$(tmux_pane0_cmd repo-feat-nm)" == *"claude --name"*"repo-feat-nm"* ]]
  run_wt new feat-nn                 # fake-ai, the suite default
  [ "$status" -eq 0 ]
  [[ "$(tmux_pane0_cmd repo-feat-nn)" != *"--name"* ]]
}

@test "new --brief: writes .planning/brief.md and opens claude on it" {
  WORKTREES_AI_CMD=claude run_wt new feat-br --brief $'# Task\n- do x\n- then y'
  [ "$status" -eq 0 ]
  [ "$(cat "$REPO/.worktrees/feat-br/.planning/brief.md")" = $'# Task\n- do x\n- then y' ]
  [[ "$output" == *"brief: .planning/brief.md"* ]]
  # the brief itself never travels through argv — only the pointer to it does
  [[ "$(tmux_pane0_cmd repo-feat-br)" == *"--name"*"repo-feat-br"*"Read .planning/brief.md and begin."* ]]
  [[ "$(tmux_pane0_cmd repo-feat-br)" != *"do x"* ]]
  # this throwaway repo does not ignore .planning/ — say so, once, in the output
  [[ "$output" == *"not ignored by git"* ]]
}

@test "new --brief: -r stays adjacent to the AI word and the opener comes last" {
  # `-r` takes an OPTIONAL session id; the opener must never follow it directly.
  WORKTREES_AI_CMD=claude run_wt new feat-rb -r --brief 'go'
  [ "$status" -eq 0 ]
  [[ "$(tmux_pane0_cmd repo-feat-rb)" == *"claude -r --name"*"repo-feat-rb"*"Read .planning/brief.md and begin."* ]]
}

@test "new --brief: a markdown list is a brief, a flag-shaped brief is a FILE" {
  # A list starts with `- `, the most natural brief there is. And a value that
  # looks like a flag is consumed whole — it lands in brief.md, never in argv.
  run_wt new feat-bl --brief $'- first\n- second'
  [ "$status" -eq 0 ]
  [ "$(cat "$REPO/.worktrees/feat-bl/.planning/brief.md")" = $'- first\n- second' ]
  run_wt new feat-bf --brief "--ai=touch $BATS_TEST_TMPDIR/PWNED"
  [ "$status" -eq 0 ]
  [ ! -e "$BATS_TEST_TMPDIR/PWNED" ]
  [ "$(cat "$REPO/.worktrees/feat-bf/.planning/brief.md")" = "--ai=touch $BATS_TEST_TMPDIR/PWNED" ]
  [[ "$(tmux_pane0_cmd repo-feat-bf)" == *fake-ai* ]]
}

@test "new --brief: no gitignore nag when .planning/ is ignored; other tools get no opener" {
  make_secret_repo '.planning/'
  run_wt new feat-bi --brief 'hi'
  [ "$status" -eq 0 ]
  [[ "$output" != *"not ignored"* ]]
  [ -f "$REPO/.worktrees/feat-bi/.planning/brief.md" ]
  # fake-ai is not claude: the file is written, the pane command is untouched
  [[ "$(tmux_pane0_cmd repo-feat-bi)" != *"brief.md"* ]]
  run_wt new feat-bx --brief
  [ "$status" -eq 1 ]
  [[ "$output" == *"--brief needs a value"* ]]
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
