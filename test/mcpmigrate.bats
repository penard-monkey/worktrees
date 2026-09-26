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
python3 - "$@" <<'PYSHIM'
import json, os, pathlib, sys
args = sys.argv[1:]
assert args[:2] == ['mcp', 'add'], args
name, fields, env, oauth = args[2], {}, {}, {}
i = 3
while i < len(args):
    flag = args[i]
    if flag == '--':
        fields['command'], fields['args'] = args[i + 1], args[i + 2:]
        break
    value = args[i + 1]
    if flag == '--env':
        key, value = value.split('=', 1)
        env[key] = value
    elif flag == '--url':
        fields['url'] = value
    elif flag == '--bearer-token-env-var':
        fields['bearer_token_env_var'] = value
    elif flag == '--oauth-client-id':
        oauth['client_id'] = value
    elif flag == '--oauth-resource':
        fields['oauth_resource'] = value
    else:
        assert flag == '--oauth-client-registration', flag
    i += 2
quote = lambda value: json.dumps(value, ensure_ascii=False)
with pathlib.Path(os.environ['CODEX_HOME'], 'config.toml').open('a') as output:
    output.write('\n[mcp_servers.' + quote(name) + ']\n')
    for key, value in fields.items():
        output.write(quote(key) + ' = ' + quote(value) + '\n')
    for section, values in [('env', env), ('oauth', oauth)]:
        if values:
            output.write('[mcp_servers.' + quote(name) + '.' + section + ']\n')
            for key, value in values.items():
                output.write(quote(key) + ' = ' + quote(value) + '\n')
PYSHIM
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

@test "migration verifies arbitrary stdio and HTTP argv instead of a canned server" {
  cat > "$HOME/.claude.json" <<'JSON'
{"mcpServers":{"custom":{"command":"node","args":["server.js","two words","quote\"here"],"env":{"API_KEY":"sk-example","REF":"${KEEP}"}},"http":{"type":"http","url":"https://example.com/mcp","headers":{"Authorization":"Bearer ${TOKEN}"}}}}
JSON
  run "$WT_BIN" mcp --migrate --ai codex --apply custom http --json
  [ "$status" -eq 0 ]
  [[ "$output" == *'"name":"custom","ok":true'* ]]
  [[ "$output" == *'"name":"http","ok":true'* ]]
  run "$WT_BIN" mcp --migrate --ai codex --json
  [ "$status" -eq 0 ]
  [ "$(printf '%s' "$output" | python3 -c 'import json,sys; print(sum(r["status"] == "exists" for r in json.load(sys.stdin)))')" -eq 2 ]
}

@test "migration locks before rechecking and adding across processes" {
  python3 - "$WT_BIN" <<'PY'
import fcntl, os, pathlib, subprocess, sys
config = pathlib.Path(os.environ['CODEX_HOME'], 'config.toml')
lock = config.with_name('config.toml.worktrees-migrate.lock')
with lock.open('a') as handle:
    fcntl.flock(handle, fcntl.LOCK_EX)
    child = subprocess.Popen([sys.argv[1], 'mcp', '--migrate', '--ai', 'codex', '--apply', 'demo', '--json'], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        try:
            child.communicate(timeout=0.5)
            raise AssertionError('migration bypassed the held file lock')
        except subprocess.TimeoutExpired:
            pass
        assert not pathlib.Path(os.environ['MIGRATION_LOG']).exists(), 'codex add ran before the lock was released'
        config.write_text('[mcp_servers.demo]\ncommand="other"\n')
        fcntl.flock(handle, fcntl.LOCK_UN)
        stdout, stderr = child.communicate(timeout=10)
        assert child.returncode == 1, (stdout, stderr)
        assert 'will not be overwritten' in stdout, stdout
        assert not pathlib.Path(os.environ['MIGRATION_LOG']).exists(), 'did not recheck after acquiring lock'
        assert config.read_text() == '[mcp_servers.demo]\ncommand="other"\n'
    finally:
        if child.poll() is None:
            child.kill()
            child.wait()
PY
}
