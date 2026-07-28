#!/usr/bin/env bats
# Plumbing: version/help/dispatch, git guards, ensure_excluded, default_base,
# prefix resolution + sanitization, AI command precedence chain, quote-safety.

load 'helpers/common'

setup() { common_setup; }

# key = value lines into the XDG user config the CLI reads.
write_config() {
  export XDG_CONFIG_HOME="$BATS_TEST_TMPDIR/xdg"
  mkdir -p "$XDG_CONFIG_HOME/worktrees"
  printf '%s\n' "$@" > "$XDG_CONFIG_HOME/worktrees/config"
}

# ── version / help / dispatch ────────────────────────────────────────────────

@test "--version prints the workspace version and exits 0 outside any git repo" {
  # read the expected version from Cargo.toml so releases don't break this test —
  # the binary-matches-manifest property is exactly what release.yml gates on
  ver="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$BATS_TEST_DIRNAME/../Cargo.toml" | head -n1)"
  [ -n "$ver" ]
  run_wt -C "$BATS_TEST_TMPDIR" --version
  [ "$status" -eq 0 ]
  [ "$output" = "worktrees $ver" ]
}

@test "help / -h / --help print usage (contains 'worktrees new') and exit 0 outside any git repo" {
  local v
  for v in help -h --help; do
    run_wt -C "$BATS_TEST_TMPDIR" "$v"
    [ "$status" -eq 0 ]
    [[ "$output" == *"worktrees new"* ]]
  done
}

@test "unknown subcommand prints usage and exits 1" {
  run_wt bogus
  [ "$status" -eq 1 ]
  [[ "$output" == *"Unknown command: bogus"* ]]
  [[ "$output" == *"worktrees new"* ]]
}

@test "repo-requiring command outside a git repo fails with a clear error" {
  run_wt -C "$BATS_TEST_TMPDIR" ls
  [ "$status" -eq 1 ]
  [[ "$output" == *"Not inside a git repository"* ]]
}

# ── ensure_excluded ──────────────────────────────────────────────────────────

@test "new adds .worktrees/ to .git/info/exclude exactly once" {
  run_wt new feat-a --no-tmux
  [ "$status" -eq 0 ]
  grep -qFx '.worktrees/' "$REPO/.git/info/exclude"
  run_wt new feat-b --no-tmux
  [ "$status" -eq 0 ]
  [ "$(grep -cFx '.worktrees/' "$REPO/.git/info/exclude")" -eq 1 ]
}

@test "new also excludes the per-worktree port file, exactly once" {
  # Not polish: .worktree.env is untracked, so without this wt_dirty is true
  # forever and switch/rm refuse without --force (proposal §8).
  run_wt new feat-a --no-tmux
  [ "$status" -eq 0 ]
  grep -qFx '.worktree.env' "$REPO/.git/info/exclude"
  run_wt new feat-b --no-tmux
  [ "$status" -eq 0 ]
  [ "$(grep -cFx '.worktree.env' "$REPO/.git/info/exclude")" -eq 1 ]
}

@test "new also excludes the UI declared-state sidecar, exactly once" {
  run_wt new feat-a --no-tmux
  [ "$status" -eq 0 ]
  grep -qFx '.worktrees.places.json' "$REPO/.git/info/exclude"
  run_wt new feat-b --no-tmux
  [ "$status" -eq 0 ]
  [ "$(grep -cFx '.worktrees.places.json' "$REPO/.git/info/exclude")" -eq 1 ]
}

# ── default_base ─────────────────────────────────────────────────────────────

@test "default base falls back to master when main does not exist" {
  local r="$BATS_TEST_TMPDIR/mrepo"
  git init -q -b master "$r"
  ( cd "$r" && echo x > f.txt && git add -A && git commit -qm init )
  run_wt -C "$r" new feat-m --no-tmux --no-fetch
  [ "$status" -eq 0 ]
  [[ "$output" == *"off 'master'"* ]]
  [ "$(git -C "$r/.worktrees/feat-m" rev-parse --abbrev-ref HEAD)" = "feat-m" ]
}

# ── prefix resolution ────────────────────────────────────────────────────────

@test ".worktree-prefix file names the tmux session" {
  echo "myproj" > "$REPO/.worktree-prefix"
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]
  tmux_session_exists myproj-feat-x
}

@test "WORKTREES_PREFIX env beats the .worktree-prefix file" {
  echo "myproj" > "$REPO/.worktree-prefix"
  export WORKTREES_PREFIX=zzz
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]
  tmux_session_exists zzz-feat-x
  [ ! -f "$TMUX_STATE/myproj-feat-x" ]
}

@test "prefix is sanitized: lowercased, '.' and '!' become '-'" {
  echo 'My.Repo!' > "$REPO/.worktree-prefix"
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]
  tmux_session_exists my-repo--feat-x
}

@test "user-config prefix is used when no env and no file" {
  write_config 'prefix = confpfx'
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]
  tmux_session_exists confpfx-feat-x
}

# ── [project] prefix (proposal §5) ───────────────────────────────────────────
# Full order: $WORKTREES_PREFIX > .worktree-prefix > [project] prefix >
# user config > basename(main_root). The legacy file is ahead of the config key
# so that adding one to a repo that has the other renames nothing.

@test "[project] prefix names new tmux sessions" {
  write_project_config '[project]' 'prefix = "teamx"'
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]
  tmux_session_exists teamx-feat-x
}

@test ".worktree-prefix still wins over [project] prefix" {
  echo "legacy" > "$REPO/.worktree-prefix"
  write_project_config '[project]' 'prefix = "teamx"'
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]
  tmux_session_exists legacy-feat-x
  [ ! -f "$TMUX_STATE/teamx-feat-x" ]
}

@test "WORKTREES_PREFIX still wins over both project-scoped sources" {
  echo "legacy" > "$REPO/.worktree-prefix"
  write_project_config '[project]' 'prefix = "teamx"'
  export WORKTREES_PREFIX=zzz
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]
  tmux_session_exists zzz-feat-x
  [ ! -f "$TMUX_STATE/legacy-feat-x" ]
  [ ! -f "$TMUX_STATE/teamx-feat-x" ]
}

@test "[project] prefix beats the user config, and config.toml is read for it" {
  export XDG_CONFIG_HOME="$BATS_TEST_TMPDIR/xdg"
  mkdir -p "$XDG_CONFIG_HOME/worktrees"
  printf 'prefix = "tomlpfx"\n' > "$XDG_CONFIG_HOME/worktrees/config.toml"
  # config.toml alone: it used to be read through the kv-only path, so `prefix`
  # there silently did nothing.
  run_wt new feat-a --no-install --no-attach
  [ "$status" -eq 0 ]
  tmux_session_exists tomlpfx-feat-a

  write_project_config '[project]' 'prefix = "teamx"'
  run_wt new feat-b --no-install --no-attach
  [ "$status" -eq 0 ]
  tmux_session_exists teamx-feat-b
}

@test "a hostile [project] prefix is sanitized before it names anything" {
  # The config arrives with a git clone (§4). sanitize_prefix is the whole
  # reason a project may set this key at all.
  write_project_config '[project]' 'prefix = "Ev;il $x/../rm -rf"'
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]
  # Assert the PROPERTY, not a hand-computed name: nothing a cloned repo writes
  # can carry a shell metacharacter, a path separator or whitespace into the
  # session name (or, later, into `docker -p`).
  local sess; sess="$(ls -1 "$TMUX_STATE")"
  [[ "$sess" =~ ^[a-z0-9_-]+$ ]]
  [[ "$sess" == *"-feat-x" ]]
}

# ── a prefix change must not orphan live sessions (§5's ⚠) ───────────────────
# The hazard: merely ADDING a config with a prefix to a repo that has running
# sessions renames every one of them at once. The sessions are found by pane
# cwd, so close/rm/open/ls keep working on the old-named session; new sessions
# get the new name.

@test "prefix change: close still closes a session running under the OLD name" {
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]
  tmux_session_exists repo-feat-x

  write_project_config '[project]' 'prefix = "teamx"'

  run_wt close feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"closed adopted tmux repo-feat-x"* ]]
  [[ "$output" != *"nothing to close"* ]]
  ! tmux_session_exists repo-feat-x
  [ -d "$REPO/.worktrees/feat-x" ]
}

@test "prefix change: rm still kills a session running under the OLD name" {
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]

  write_project_config '[project]' 'prefix = "teamx"'

  run_wt rm -y feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"killed tmux repo-feat-x"* ]]
  grep -q "kill-session -t =repo-feat-x" "$TMUX_LOG"
  ! tmux_session_exists repo-feat-x
  [ ! -e "$REPO/.worktrees/feat-x" ]
}

@test "prefix change: ls still shows the old-named session as live" {
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]

  write_project_config '[project]' 'prefix = "teamx"'

  run_wt ls
  [ "$status" -eq 0 ]
  [[ "$output" == *"●"* ]]
  run_wt ls --json
  [ "$status" -eq 0 ]
  [[ "$output" == *'"name":"repo-feat-x","up":true'* ]]
}

@test "prefix change: open re-attaches the old session instead of starting a second one" {
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]

  write_project_config '[project]' 'prefix = "teamx"'

  run_wt open feat-x --no-attach
  [ "$status" -eq 0 ]
  [[ "$output" == *"tmux session 'repo-feat-x' already in this worktree"* ]]
  [ ! -f "$TMUX_STATE/teamx-feat-x" ]
}

@test "prefix change: a NEW worktree gets the new name, and doctor reports the drift" {
  run_wt new feat-old --no-install --no-attach
  [ "$status" -eq 0 ]

  write_project_config '[project]' 'prefix = "teamx"'

  run_wt new feat-new --no-install --no-attach
  [ "$status" -eq 0 ]
  tmux_session_exists teamx-feat-new

  run_wt doctor
  # a Warn, so still exit 0 — nothing is broken, it is just not obvious
  [ "$status" -eq 0 ]
  [[ "$output" == *"repo-feat-old"* ]]
  [[ "$output" == *"teamx-feat-old"* ]]
  [[ "$output" != *"feat-new"* ]]
}

@test "doctor: warns when .worktree-prefix and [project] prefix disagree, naming the winner" {
  echo "legacy" > "$REPO/.worktree-prefix"
  write_project_config '[project]' 'prefix = "teamx"'

  run_wt doctor
  [ "$status" -eq 0 ]
  [[ "$output" == *'says `legacy`'* ]]
  [[ "$output" == *'[project] prefix says `teamx`'* ]]
  [[ "$output" == *'legacy-<slug>'* ]]

  # sources that agree say nothing at all
  echo "teamx" > "$REPO/.worktree-prefix"
  run_wt doctor
  [ "$status" -eq 0 ]
  [[ "$output" != *"[project] prefix says"* ]]
}

# ── AI command precedence chain ──────────────────────────────────────────────

@test "config ai_cmd is used when env vars are unset" {
  unset WORKTREES_AI_CMD
  write_config 'ai_cmd = fake-ai'
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]
  local p0; p0="$(tmux_pane0_cmd repo-feat-x)"
  [[ "$p0" == *"-ic"* ]]
  [[ "$p0" == *"fake-ai"* ]]
}

@test "WORKTREES_CLAUDE_CMD is honored as deprecated fallback when WORKTREES_AI_CMD unset" {
  unset WORKTREES_AI_CMD
  export WORKTREES_CLAUDE_CMD=fake-ai
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]
  local p0; p0="$(tmux_pane0_cmd repo-feat-x)"
  [[ "$p0" == *"fake-ai"* ]]
  [[ "$p0" != *"claude"* ]]
}

@test "WORKTREES_AI_CMD beats WORKTREES_CLAUDE_CMD" {
  install_fake_cmd other
  export WORKTREES_AI_CMD=fake-ai WORKTREES_CLAUDE_CMD=other
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]
  local p0; p0="$(tmux_pane0_cmd repo-feat-x)"
  [[ "$p0" == *"fake-ai"* ]]
  [[ "$p0" != *"other"* ]]
}

@test "--ai flag beats env WORKTREES_AI_CMD" {
  install_fake_cmd other
  # WORKTREES_AI_CMD=fake-ai is set by common_setup
  run_wt new feat-x --no-install --no-attach --ai other
  [ "$status" -eq 0 ]
  local p0; p0="$(tmux_pane0_cmd repo-feat-x)"
  [[ "$p0" == *"other"* ]]
  [[ "$p0" != *"fake-ai"* ]]
}

@test "config ai_cmd = none gives pane 0 a plain shell (no -ic)" {
  unset WORKTREES_AI_CMD
  write_config 'ai_cmd = none'
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]
  local p0; p0="$(tmux_pane0_cmd repo-feat-x)"
  [[ "$p0" == *'exec "${SHELL'* ]]
  [[ "$p0" != *"-ic"* ]]
}

@test "--ai none gives pane 0 a plain shell even with WORKTREES_AI_CMD set" {
  # WORKTREES_AI_CMD=fake-ai is set by common_setup
  run_wt new feat-x --no-install --no-attach --ai none
  [ "$status" -eq 0 ]
  local p0; p0="$(tmux_pane0_cmd repo-feat-x)"
  [[ "$p0" == *'exec "${SHELL'* ]]
  [[ "$p0" != *"-ic"* ]]
  [[ "$p0" != *"fake-ai"* ]]
}

@test "ai_resume_arg config key changes what -r appends" {
  write_config 'ai_resume_arg = --continue'
  # WORKTREES_AI_RESUME_ARG unset by common_setup; ai cmd stays fake-ai (env)
  run_wt new feat-x --no-install --no-attach -r
  [ "$status" -eq 0 ]
  local p0; p0="$(tmux_pane0_cmd repo-feat-x)"
  [[ "$p0" == *"fake-ai --continue"* ]]
}

@test "AI command containing a single quote is escaped safely into pane 0" {
  export WORKTREES_AI_CMD="fake-ai --note it's"
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]
  local p0; p0="$(tmux_pane0_cmd repo-feat-x)"
  # sq() turns it's into it'\''s inside the single-quoted -ic string
  local esc="it'\\''s"
  [[ "$p0" == *"$esc"* ]]
}

@test "symlinked repo path: worktrees stay registered (pwd -P regression)" {
  # macOS $TMPDIR traverses /var → /private/var, so $REPO_LOGICAL reaches the
  # repo THROUGH a symlink. Before the pwd -P fix, wt_registered() compared the
  # logical WT_ROOT against git's physical paths: ls showed every worktree as
  # stale, switch refused, rm plain-deleted instead of `git worktree remove`.
  [ "$REPO_LOGICAL" != "$REPO" ] || skip "tmpdir not behind a symlink on this OS"
  run_wt -C "$REPO_LOGICAL" new feat/sym --no-tmux
  [ "$status" -eq 0 ]
  run_wt -C "$REPO_LOGICAL" ls
  [ "$status" -eq 0 ]
  [[ "$output" != *"stale"* ]]
  [[ "$output" == *"feat/sym"* ]]
  run_wt -C "$REPO_LOGICAL" switch feat-sym feat/sym2
  [ "$status" -eq 0 ]
  run_wt -C "$REPO_LOGICAL" rm -y feat-sym
  [ "$status" -eq 0 ]
  [[ "$output" == *"removed worktree feat-sym"* ]]   # git-removed, NOT plain-deleted
}
