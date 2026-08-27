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

# What a push leaves on the hub: a REAL repo at <hub>/<name>/<name>, and the
# manifest one level up saying the project natively lives on the other machine.
# Echoes the copy's physical path (the CLI canonicalizes, so assertions must).
make_hub_copy() {        # $1 = name, $2 = the native root on the other Mac
  local name="$1" native="$2"
  write_hub_manifest "$name" "$native" 'host = "othermac"'
  git init -q "$HUB/$name/$name"
  ( cd "$HUB/$name/$name" && echo hi > README.md && git add -A && git commit -qm init )
  ( cd "$HUB/$name/$name" && pwd -P )
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

# ── post-pull heal + status note ─────────────────────────────────────────────
# The field bug: the exclude list is "rebuildable bulk", but repos COMMIT files
# matching it (`old-plans/*.tar.gz`, session tarballs). The transfer skips them,
# so the copy's `.git` says they exist while the working files never arrive — a
# fresh import opens on a wall of `deleted:`. The blobs ride along INSIDE `.git`,
# so the pull restores the working copies from there.
#
# The fake rsync transfers nothing, which is exactly right for these: the heal
# reads REAL git state, so the post-import tree is built by hand.

commit_bulk() {   # a tracked *.tar.gz (excluded) beside a tracked plain file
  ( cd "$REPO" && mkdir -p old-plans docs && echo tar > old-plans/data.tar.gz \
      && echo keep > docs/keep.txt && git add -A && git commit -qm bulk )
}

@test "sync pull: an excluded TRACKED file is healed, a plain deletion is left alone" {
  commit_bulk
  write_hub_manifest "$NAME" "$REPO" 'host = "othermac"'
  rm "$REPO/old-plans/data.tar.gz" "$REPO/docs/keep.txt"

  run_wt sync pull --yes --hub "$HUB"
  [ "$status" -eq 0 ]
  [ -f "$REPO/old-plans/data.tar.gz" ]      # the transfer skips it; git carries it
  [ ! -e "$REPO/docs/keep.txt" ]            # NOT excluded ⇒ mirrored intent, left alone
  [[ "$output" == *"restored 1"* ]]
  [[ "$output" == *"old-plans/data.tar.gz"* ]]
  [[ "$output" != *"keep.txt"* ]]
}

@test "sync pull: a STAGED deletion is the source Mac's intent — the heal leaves it" {
  commit_bulk
  write_hub_manifest "$NAME" "$REPO" 'host = "othermac"'
  ( cd "$REPO" && git rm -q old-plans/data.tar.gz )

  run_wt sync pull --yes --hub "$HUB"
  [ "$status" -eq 0 ]
  [ ! -e "$REPO/old-plans/data.tar.gz" ]
  [[ "$output" != *"restored"* ]]
}

@test "sync pull: a worktree under .worktrees/ is healed too" {
  ( cd "$REPO" && echo log > x.log && git add -A && git commit -qm log )
  git -C "$REPO" worktree add -q "$REPO/.worktrees/feat" -b feat
  write_hub_manifest "$NAME" "$REPO" 'host = "othermac"'
  rm "$REPO/.worktrees/feat/x.log"

  run_wt sync pull --yes --hub "$HUB"
  [ "$status" -eq 0 ]
  [ -f "$REPO/.worktrees/feat/x.log" ]
  [[ "$output" == *"restored 1"* ]]
  [[ "$output" == *".worktrees/feat/x.log"* ]]
}

@test "sync pull: a clean, level tree gets neither a restored line nor a status note" {
  write_hub_manifest "$NAME" "$REPO" 'host = "othermac"'
  run_wt sync pull --yes --hub "$HUB"
  [ "$status" -eq 0 ]
  [[ "$output" != *"restored"* ]]
  [[ "$output" != *"mirrors the source"* ]]
}

@test "sync pull: a tree behind its upstream reads as fidelity, not breakage" {
  # The source Mac was behind origin; a faithful mirror is behind too, and
  # nothing in the transfer explains that. $ORIGIN is a real bare repo here.
  ( cd "$REPO" && echo more >> README.md && git add -A && git commit -qm second \
      && git push -q origin main && git reset -q --hard HEAD~1 )
  write_hub_manifest "$NAME" "$REPO" 'host = "othermac"'

  run_wt sync pull --yes --hub "$HUB"
  [ "$status" -eq 0 ]
  [[ "$output" == *"mirrors the source Mac"* ]]
  [[ "$output" == *"main is 1 commit(s) behind"* ]]
}

# ── the heal runs on every sync EDGE, not just a successful pull ─────────────
# The repair used to be pull-only, so damage in a tree nobody pulled into sat
# there indefinitely (the field case: 8 of 10 worktrees, months old). A push
# heals before it transfers, and a pull heals even when the transfer failed.

# rsync that fails the APPLY — the only invocation carrying --backup-dir — and
# answers the preview normally: an interrupted transfer, not a broken setup.
install_failing_apply_rsync() {
  cat > "$SHIMS/rsync" <<EOF
#!/usr/bin/env bash
echo "\$@" >> "$BATS_TEST_TMPDIR/rsync.log"
for a in "\$@"; do
  case "\$a" in --backup-dir=*) echo "rsync: connection unexpectedly closed" >&2; exit 12;; esac
done
exit 0
EOF
  chmod +x "$SHIMS/rsync"
}

@test "sync push: the damaged tree is healed BEFORE the transfer" {
  commit_bulk
  rm "$REPO/old-plans/data.tar.gz" "$REPO/docs/keep.txt"

  run_wt sync push --yes --hub "$HUB"
  [ "$status" -eq 0 ]
  has_apply
  [ -f "$REPO/old-plans/data.tar.gz" ]      # excluded + tracked ⇒ ours to repair
  [ ! -e "$REPO/docs/keep.txt" ]            # not excluded ⇒ the user's own deletion
  [[ "$output" == *"restored 1"* ]]
  [[ "$output" == *"old-plans/data.tar.gz"* ]]
  # push heals only — the mirrored-state note is a sentence only an import says
  [[ "$output" != *"mirrors the source Mac"* ]]
}

@test "sync push --dry-run: a damaged tree is previewed, never repaired" {
  commit_bulk
  rm "$REPO/old-plans/data.tar.gz"

  run_wt sync push --dry-run --hub "$HUB"
  [ "$status" -eq 0 ]
  ! has_apply
  [ ! -e "$REPO/old-plans/data.tar.gz" ]    # --dry-run writes NOTHING, here included
  [[ "$output" != *"restored"* ]]

  # …and a declined confirmation is the same promise
  wt_answer "n" sync push --hub "$HUB"
  [ "$status" -eq 0 ]
  [ ! -e "$REPO/old-plans/data.tar.gz" ]
  [[ "$output" != *"restored"* ]]
}

@test "sync pull: a transfer that FAILS still heals, and still reports the failure" {
  commit_bulk
  write_hub_manifest "$NAME" "$REPO" 'host = "othermac"'
  rm "$REPO/old-plans/data.tar.gz"
  install_failing_apply_rsync

  run_wt sync pull --yes --hub "$HUB"
  [ "$status" -eq 1 ]                       # the transfer's error is the outcome
  [[ "$output" == *"rsync failed (exit 12)"* ]]
  [[ "$output" != *"done."* ]]
  [ -f "$REPO/old-plans/data.tar.gz" ]      # …and the repair happened anyway
  [[ "$output" == *"restored 1"* ]]
}

@test "doctor: skipped-file damage is reported between syncs, as a warning" {
  commit_bulk
  rm "$REPO/old-plans/data.tar.gz" "$REPO/docs/keep.txt"

  run_wt doctor
  [ "$status" -eq 0 ]                       # Warn never fails a run (§7)
  [[ "$output" == *"old-plans/data.tar.gz"* ]]
  [[ "$output" == *"nothing is lost"* ]]
  [[ "$output" == *"sync push"* ]]          # the remedy, named
  [[ "$output" != *"docs/keep.txt"* ]]      # not excluded ⇒ not ours to report

  run_wt doctor --json
  [ "$status" -eq 0 ]
  [[ "$output" == *'"code":"sync-skipped-files"'* ]]
  [[ "$output" == *'"severity":"warn"'* ]]

  # doctor DETECTS, it never repairs — the tree is exactly as it was
  [ ! -e "$REPO/old-plans/data.tar.gz" ]

  # …and a healed tree says nothing at all
  ( cd "$REPO" && git checkout -- old-plans/data.tar.gz )
  run_wt doctor
  [ "$status" -eq 0 ]
  [[ "$output" != *"tracked file(s) are missing"* ]]
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

# ── guards ───────────────────────────────────────────────────────────────────

@test "sync guard: mutating ops refuse inside a hub copy and name the way out" {
  local copy; copy="$(make_hub_copy proj "$BATS_TEST_TMPDIR/native")"

  run_wt -C "$copy" new x
  [ "$status" -eq 1 ]
  [[ "$output" == *"hub copy"* ]]
  [[ "$output" == *"othermac"* ]]
  [[ "$output" == *"$BATS_TEST_TMPDIR/native"* ]]
  [ ! -d "$copy/.worktrees/x" ]

  # reading the tree is how you find out WHAT it is — never refused
  run_wt -C "$copy" ls
  [ "$status" -eq 0 ]

  # both directions: push from a copy is backwards, pull into one is hub→hub
  run_wt -C "$copy" sync push --hub "$HUB"
  [ "$status" -eq 1 ]
  [[ "$output" == *"hub copy"* ]]
  run_wt -C "$copy" sync pull --yes --hub "$HUB"
  [ "$status" -eq 1 ]
  [[ "$output" == *"hub copy"* ]]
  # refused before rsync was even looked for — not so much as a --version probe
  [ ! -s "$RSYNC_LOG" ]
}

@test "sync guard: a manifest about ANOTHER project leaves the tree alone" {
  local copy; copy="$(make_hub_copy proj "$BATS_TEST_TMPDIR/native")"
  printf 'schema = 1\nname = "other"\nlocal_root = "%s"\n' "$BATS_TEST_TMPDIR/native" \
    > "$HUB/proj/$MF"
  run_wt -C "$copy" new x
  [ "$status" -eq 0 ]
  [ -d "$copy/.worktrees/x" ]
}

@test "sync guard: a manifest naming THIS path is not a copy — the way out cannot be where you stand" {
  # NOT what a hub looks like: on the hub the manifest names the OTHER machine's
  # root, never the tree beside it. The equality case is a manifest that ended up
  # next to the native tree (a whole hub dir copied home, a hand-written file) —
  # and refusing there would tell the user to go to the path they are already in.
  mkdir -p "$BATS_TEST_TMPDIR/host"
  git init -q "$BATS_TEST_TMPDIR/host/proj"
  ( cd "$BATS_TEST_TMPDIR/host/proj" && echo hi > README.md && git add -A && git commit -qm init )
  local native; native="$(cd "$BATS_TEST_TMPDIR/host/proj" && pwd -P)"
  printf 'schema = 1\nname = "proj"\nlocal_root = "%s"\n' "$native" \
    > "$BATS_TEST_TMPDIR/host/$MF"
  run_wt -C "$native" new x
  [ "$status" -eq 0 ]
  [ -d "$native/.worktrees/x" ]
}

@test "sync guard: doctor reports the hub copy even in a repo with no .worktrees.toml" {
  local copy; copy="$(make_hub_copy proj "$BATS_TEST_TMPDIR/native")"
  run_wt -C "$copy" doctor
  [ "$status" -eq 2 ]                     # diag::EXIT_FINDINGS — an Error finding
  [[ "$output" == *"hub copy"* ]]
  [[ "$output" == *"$BATS_TEST_TMPDIR/native"* ]]

  run_wt -C "$copy" doctor --json
  [ "$status" -eq 2 ]
  [[ "$output" == *'"code":"hub-copy"'* ]]
  [[ "$output" == *'"severity":"error"'* ]]

  # …and never the skipped-files warning beside it: a hub copy is missing every
  # excluded-tracked file BY CONSTRUCTION (the push skipped them), so absence
  # there is the pushed state, not damage — and the remedy that finding names is
  # exactly what the hub-copy guard refuses
  ( cd "$copy" && mkdir -p old-plans && echo t > old-plans/data.tar.gz \
    && git add -A && git commit -qm bulk && rm old-plans/data.tar.gz )
  run_wt -C "$copy" doctor
  [ "$status" -eq 2 ]
  [[ "$output" == *"hub copy"* ]]
  [[ "$output" != *"tracked file(s) are missing"* ]]

  # …and an ordinary repo's doctor is untouched by the check
  run_wt doctor
  [ "$status" -eq 0 ]
  [[ "$output" != *"hub copy"* ]]
}

@test "sync pull: a live session refuses --yes and is named, but a human may answer" {
  run_wt sync push --yes --hub "$HUB"
  [ "$status" -eq 0 ]
  run_wt new feat
  [ "$status" -eq 0 ]
  tmux_session_exists "$NAME-feat"
  : > "$RSYNC_LOG"

  run_wt sync pull --yes --hub "$HUB"
  [ "$status" -eq 4 ]                     # diag::EXIT_NEEDS_CONFIRM
  [[ "$output" == *"$NAME-feat"* ]]
  [[ "$output" == *"--yes"* ]]
  ! has_apply
  has_preview                             # it got as far as the preview, then asked

  : > "$RSYNC_LOG"
  wt_answer "y" sync pull --hub "$HUB"
  [ "$status" -eq 0 ]
  [[ "$output" == *"$NAME-feat"* ]]
  has_apply
}

@test "sync pull: adoption onto a machine with no such tree passes the session guard" {
  local dest="$BATS_TEST_TMPDIR/not-here-yet"
  write_hub_manifest proj "$dest" 'host = "othermac"'
  [ ! -e "$dest" ]
  run_wt -C "$BATS_TEST_TMPDIR" sync pull proj --yes --hub "$HUB"
  [ "$status" -eq 0 ]
  has_apply
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
