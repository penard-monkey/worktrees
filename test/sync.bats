#!/usr/bin/env bats
# `worktrees sync` — courier sync of a project through a mounted hub (findings §3).
#
# rsync is FAKED for the whole suite (helpers/common.bash), so every assertion
# here is about the argv the tool assembled: direction, --delete, --exclude-from,
# --backup-dir, and the sessions ferry's deliberate ABSENCE of --delete. The hub
# is always passed with --hub, never auto-detected: there is no /Volumes in CI.

load 'helpers/common'

setup() {
  common_setup
  HUB="$BATS_TEST_TMPDIR/hub"
  RSYNC_LOG="$BATS_TEST_TMPDIR/rsync.log"
  NAME="$(basename "$REPO")"
  MF=".worktrees-sync.toml"
}

# The APPLY invocation is the only one carrying --backup-dir; the PREVIEW is the
# only one carrying -ani. Both are one line of argv in the shim's log.
has_apply()   { grep -q -- '--backup-dir=' "$RSYNC_LOG"; }
has_preview() { grep -q -- '^-ani ' "$RSYNC_LOG"; }

write_hub_manifest() {   # $1 = name, $2 = local_root, $3.. = extra lines
  local name="$1" root="$2"; shift 2
  mkdir -p "$HUB/$name/$name"
  { printf 'schema = 1\nname = "%s"\nlocal_root = "%s"\n' "$name" "$root"
    [ $# -gt 0 ] && printf '%s\n' "$@"; } > "$HUB/$name/$MF"
}

# ── grammar ──────────────────────────────────────────────────────────────────

@test "sync: no subcommand prints the usage and exits 1" {
  run_wt sync
  [ "$status" -eq 1 ]
  [[ "$output" == *"push"* ]]
  [[ "$output" == *"pull"* ]]
  [[ "$output" == *"status"* ]]
  # sync's OWN usage, not the top-level one — which also names push/pull/status,
  # and would let this test pass against a binary with no sync command at all.
  [[ "$output" == *"--with-sessions"* ]]
}

@test "sync: an unknown subcommand and an unknown flag both exit 1" {
  run_wt sync sideways
  [ "$status" -eq 1 ]
  [[ "$output" == *"Unknown sync command"* ]]

  run_wt sync push --turbo --hub "$HUB"
  [ "$status" -eq 1 ]
  [[ "$output" == *"Unknown flag: --turbo"* ]]
  [ ! -d "$HUB" ]
}

@test "sync push: outside a repo it names the fix and exits 1" {
  mkdir -p "$BATS_TEST_TMPDIR/elsewhere"
  run_wt -C "$BATS_TEST_TMPDIR/elsewhere" sync push --hub "$HUB"
  [ "$status" -eq 1 ]
  [[ "$output" == *"cd into the repo"* ]]
  [ ! -d "$HUB" ]
}

# ── push ─────────────────────────────────────────────────────────────────────

@test "sync push --dry-run: previews the mirror and touches nothing" {
  run_wt sync push --dry-run --hub "$HUB"
  [ "$status" -eq 0 ]
  grep -q -- '^-ani --delete --exclude-from=' "$RSYNC_LOG"
  grep -q -- " $REPO/ $HUB/$NAME/$NAME/\$" "$RSYNC_LOG"
  # a dry run is a dry run: no hub tree, no manifest, no apply
  ! has_apply
  [ ! -d "$HUB" ]
  [ ! -e "$HUB/$NAME/$MF" ]
}

@test "sync push --yes: applies with a backup dir, then writes the manifest" {
  run_wt sync push --yes --hub "$HUB"
  [ "$status" -eq 0 ]
  has_preview
  grep -q -- "^-a --delete --exclude-from=.* --backup --backup-dir=$HUB/$NAME/.worktrees-sync/backups/[0-9-]*/$NAME $REPO/ $HUB/$NAME/$NAME/\$" \
    "$RSYNC_LOG"
  # progress is a TTY-only flag, and the shim is not v3 either
  ! grep -q -- '--info=progress2' "$RSYNC_LOG"
  ! grep -q -- '--progress' "$RSYNC_LOG"
  [ -d "$HUB/$NAME/$NAME" ]
  [ -f "$HUB/$NAME/$MF" ]
  grep -qF "name = \"$NAME\"" "$HUB/$NAME/$MF"
  grep -qF "local_root = \"$REPO\"" "$HUB/$NAME/$MF"
  grep -q '^schema = 1$' "$HUB/$NAME/$MF"
}

@test "sync push: a declined confirmation applies nothing and leaves no manifest" {
  wt_answer "n" sync push --hub "$HUB"
  [ "$status" -eq 0 ]
  [[ "$output" == *"Skipped $NAME"* ]]
  has_preview                      # it still previewed — that is what you answered about
  ! has_apply
  [ ! -e "$HUB/$NAME/$MF" ]
}

# ── pull ─────────────────────────────────────────────────────────────────────

@test "sync pull: the direction reverses and the backups land under the LOCAL root" {
  run_wt sync push --yes --hub "$HUB"
  [ "$status" -eq 0 ]
  : > "$RSYNC_LOG"

  run_wt sync pull --yes --hub "$HUB"
  [ "$status" -eq 0 ]
  grep -q -- " $HUB/$NAME/$NAME/ $REPO/\$" "$RSYNC_LOG"
  grep -q -- "--backup-dir=$REPO/.worktrees-sync/backups/" "$RSYNC_LOG"
}

@test "sync pull: a hub with nothing pushed points at push" {
  mkdir -p "$HUB"
  run_wt sync pull --yes --hub "$HUB"
  [ "$status" -eq 1 ]
  [[ "$output" == *"sync push"* ]]
  [[ "$output" == *"on the other Mac"* ]]
  ! has_apply
}

@test "sync pull <name>: adopts from the hub manifest; without a name it lists them" {
  local dest="$BATS_TEST_TMPDIR/adopted"
  mkdir -p "$dest"
  write_hub_manifest proj "$dest" 'pushed_at = 42' 'host = "othermac"' 'hint = "cargo build"'

  # outside a repo with no name: exit 1, and the hub's projects are the answer
  run_wt -C "$BATS_TEST_TMPDIR" sync pull --hub "$HUB"
  [ "$status" -eq 1 ]
  [[ "$output" == *"proj"* ]]
  [[ "$output" == *"$dest"* ]]
  ! has_apply

  run_wt -C "$BATS_TEST_TMPDIR" sync pull proj --hub "$HUB" --yes
  [ "$status" -eq 0 ]
  grep -q -- " $HUB/proj/proj/ $dest/\$" "$RSYNC_LOG"
  # the manifest's hint is PRINTED (never run) — see the ADR test below
  [[ "$output" == *"Next:"* ]]
  [[ "$output" == *"cargo build"* ]]
}

@test "sync pull: a manifest with a command-shaped key is a hard error (ADR 0001)" {
  write_hub_manifest "$NAME" "$REPO" "install_cmd = \"touch $BATS_TEST_TMPDIR/pwned\""
  run_wt sync pull --yes --hub "$HUB"
  [ "$status" -eq 1 ]
  [[ "$output" == *"install_cmd"* ]]
  ! has_apply
  ! has_preview                    # it stopped BEFORE rsync ran at all
  [ ! -e "$BATS_TEST_TMPDIR/pwned" ]
}

@test "sync pull: the manifest hint is printed, and --install runs the USER's command" {
  # No user config yet: the hint must reach the terminal without reaching a shell.
  write_hub_manifest "$NAME" "$REPO" "hint = \"touch $BATS_TEST_TMPDIR/hint-ran\""
  run_wt sync pull --yes --install --hub "$HUB"
  [ "$status" -eq 0 ]
  [[ "$output" == *"touch $BATS_TEST_TMPDIR/hint-ran"* ]]
  [ ! -e "$BATS_TEST_TMPDIR/hint-ran" ]
  [[ "$output" == *"no rebuild registered"* ]]

  # …and now the user's own config, which IS argv (same tier as ai_cmd).
  mkdir -p "$HOME/.config/worktrees"
  printf '[sync.projects.%s]\ninstall = "touch marker"\n' "$NAME" \
    > "$HOME/.config/worktrees/config.toml"
  run_wt sync pull --yes --install --hub "$HUB"
  [ "$status" -eq 0 ]
  [ -f "$REPO/marker" ]
}

# ── sessions ferry ───────────────────────────────────────────────────────────

@test "sync push --with-sessions: ferries the project's claude dirs ADDITIVELY" {
  export CLAUDE_PROJECTS="$BATS_TEST_TMPDIR/claude-projects"
  local key; key="$(printf '%s' "$REPO" | tr '/' '-')"
  mkdir -p "$CLAUDE_PROJECTS/$key-x"
  echo '{}' > "$CLAUDE_PROJECTS/$key-x/f.jsonl"

  run_wt sync push --yes --hub "$HUB"
  [ "$status" -eq 0 ]
  ! grep -q -- '.claude-sessions' "$RSYNC_LOG"
  [[ "$output" == *"--with-sessions"* ]]

  : > "$RSYNC_LOG"
  run_wt sync push --yes --with-sessions --hub "$HUB"
  [ "$status" -eq 0 ]
  grep -q -- "^-a $CLAUDE_PROJECTS/$key-x/ $HUB/$NAME/.claude-sessions/$key-x/\$" "$RSYNC_LOG"
  # THE property: a session ferry never deletes the other Mac's history
  grep -- '.claude-sessions' "$RSYNC_LOG" > "$BATS_TEST_TMPDIR/sess.log"
  ! grep -q -- '--delete' "$BATS_TEST_TMPDIR/sess.log"
  ! grep -q -- '--backup' "$BATS_TEST_TMPDIR/sess.log"
}

# ── status ───────────────────────────────────────────────────────────────────

@test "sync status --json: one valid JSON object for this project" {
  command -v python3 >/dev/null 2>&1 || skip "python3 required to validate JSON"
  run_wt sync push --yes --hub "$HUB"
  [ "$status" -eq 0 ]

  run_wt sync status --json --hub "$HUB"
  [ "$status" -eq 0 ]
  printf '%s' "$output" | python3 -m json.tool >/dev/null
  local f; f() { printf '%s' "$output" | python3 -c "import sys,json;print(json.load(sys.stdin)[sys.argv[1]])" "$1"; }
  [ "$(f scope)" = "project" ]
  [ "$(f name)" = "$NAME" ]
  [ "$(f local_root)" = "$REPO" ]
  [ "$(f hub)" = "$HUB" ]
  [ "$(f branch)" = "main" ]
}

@test "sync status: outside a repo it reports the HUB and what has been pushed to it" {
  write_hub_manifest proj "$BATS_TEST_TMPDIR/adopted" 'host = "othermac"'
  run_wt -C "$BATS_TEST_TMPDIR" sync status --hub "$HUB"
  [ "$status" -eq 0 ]
  [[ "$output" == *"proj"* ]]
  [[ "$output" == *"$BATS_TEST_TMPDIR/adopted"* ]]
  [[ "$output" == *"othermac"* ]]
}

@test "sync status: a hub this project was never pushed to is reported, not fatal" {
  # Deliberately NOT the auto-detect path: /Volumes is the developer's real
  # machine, and a suite whose result depends on what is plugged in is not a
  # gate. The 0/1/many candidate rule is unit-tested in sync.rs instead.
  run_wt sync status --hub "$BATS_TEST_TMPDIR/nohub"
  [ "$status" -eq 0 ]
  [[ "$output" == *"never pushed"* ]]
  [[ "$output" == *"$REPO"* ]]
  [[ "$output" == *"main"* ]]
}
