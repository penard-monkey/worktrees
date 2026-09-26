#!/usr/bin/env bats
load 'helpers/common'
setup() {
  common_setup
  export CODEX_HOME="$BATS_TEST_TMPDIR/codex"
  export MIGRATION_LOG="$BATS_TEST_TMPDIR/migration.log"
  mkdir -p "$CODEX_HOME"
  cat > "$HOME/.claude.json" <<'JSON'
{"mcpServers":{"demo":{"command":"echo","args":["${TOKEN}","two words"],"env":{"KEY":"${SECRET}"}},"fail":{"command":"false"},"legacy":{"type":"sse","url":"https://example.com"},"worktrees":{"command":"worktrees"}}}
JSON
  cp "$HOME/.claude.json" "$BATS_TEST_TMPDIR/claude-before.json"
  cat > "$SHIMS/codex" <<'SH'
#!/bin/sh
printf '%s\n' "$@" >> "$MIGRATION_LOG"
[ "$3" = fail ] && exit 1
[ "${MIGRATION_NOOP:-}" = yes ] && exit 0
cat >> "$CODEX_HOME/config.toml" <<'TOML'
[mcp_servers.demo]
command = "echo"
args = ["${TOKEN}", "two words"]
[mcp_servers.demo.env]
KEY = "${SECRET}"
TOML
SH
  chmod +x "$SHIMS/codex"
}

@test "migration plan is available outside a repo and never writes" {
  cd "$BATS_TEST_TMPDIR"
  run "$WT_BIN" mcp --migrate --ai codex --json
  [ "$status" -eq 0 ]
  [[ "$output" == *'"name":"demo"'*'"status":"copy"'* ]]
  [[ "$output" == *'"status":"unsupported"'* ]]
  [[ "$output" != *'"name":"worktrees"'* ]]
  [ ! -e "$MIGRATION_LOG" ]
  [ ! -e "$CODEX_HOME/config.toml" ]
  cmp "$HOME/.claude.json" "$BATS_TEST_TMPDIR/claude-before.json"
}

@test "migration applies only selected names with literal argv and verifies persisted config" {
  run "$WT_BIN" mcp --migrate --ai codex --apply demo --json
  [ "$status" -eq 0 ]
  [[ "$output" == *'"name":"demo","ok":true'* ]]
  grep -Fx 'KEY=${SECRET}' "$MIGRATION_LOG"
  grep -Fx '${TOKEN}' "$MIGRATION_LOG"
  grep -Fx 'two words' "$MIGRATION_LOG"
  cmp "$HOME/.claude.json" "$BATS_TEST_TMPDIR/claude-before.json"
  run "$WT_BIN" mcp --migrate --ai codex --json
  [[ "$output" == *'"name":"demo"'*'"status":"exists"'* ]]
}

@test "migration rechecks existing names and refuses to overwrite" {
  printf '[mcp_servers.demo]\ncommand="other"\n' > "$CODEX_HOME/config.toml"
  cp "$CODEX_HOME/config.toml" "$BATS_TEST_TMPDIR/codex-before.toml"
  run "$WT_BIN" mcp --migrate --ai codex --apply demo --json
  [ "$status" -eq 1 ]
  [[ "$output" == *'will not be overwritten'* ]]
  [ ! -e "$MIGRATION_LOG" ]
  cmp "$CODEX_HOME/config.toml" "$BATS_TEST_TMPDIR/codex-before.toml"
}

@test "migration reports partial failures and continues selected rows" {
  run "$WT_BIN" mcp --migrate --ai codex --apply fail demo legacy unknown worktrees --json
  [ "$status" -eq 1 ]
  [[ "$output" == *'"name":"fail","ok":false'* ]]
  [[ "$output" == *'"name":"demo","ok":true'* ]]
  [[ "$output" == *'"name":"legacy","ok":false'* ]]
  [[ "$output" == *'"name":"unknown","ok":false'* ]]
  [[ "$output" == *'"name":"worktrees","ok":false'* ]]
  [ "$(grep -c '^add$' "$MIGRATION_LOG")" -eq 2 ]
}

@test "migration rejects a successful CLI exit without a persisted server" {
  export MIGRATION_NOOP=yes
  run "$WT_BIN" mcp --migrate --ai codex --apply demo --json
  [ "$status" -eq 1 ]
  [[ "$output" == *'"ok":false'*'did not save'* ]]
}

@test "migration refuses unreadable config rather than treating it as empty" {
  printf 'not valid TOML [' > "$CODEX_HOME/config.toml"
  run "$WT_BIN" mcp --migrate --ai codex --json
  [ "$status" -eq 1 ]
  [[ "$output" == *'Cannot parse Codex'* ]]
  printf '' > "$CODEX_HOME/config.toml"
  printf '{bad json' > "$HOME/.claude.json"
  run "$WT_BIN" mcp --migrate --ai codex --apply demo --json
  [ "$status" -eq 1 ]
  [[ "$output" == *'Cannot parse Claude'* ]]
  [ ! -e "$MIGRATION_LOG" ]
}

@test "migration requires an explicit provider and nonempty selection" {
  run "$WT_BIN" mcp --migrate --json
  [ "$status" -eq 1 ]
  [[ "$output" == *'--ai codex'* ]]
  run "$WT_BIN" mcp --migrate --ai codex --apply --json
  [ "$status" -eq 1 ]
  [[ "$output" == *'at least one server name'* ]]
  [ ! -e "$MIGRATION_LOG" ]
}

@test "migration refuses structurally invalid Claude configuration" {
  printf '[]' > "$HOME/.claude.json"
  run "$WT_BIN" mcp --migrate --ai codex --json
  [ "$status" -eq 1 ]
  [[ "$output" == *'Claude configuration must be an object'* ]]
  [ ! -e "$MIGRATION_LOG" ]
}
