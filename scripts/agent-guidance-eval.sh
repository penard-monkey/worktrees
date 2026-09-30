#!/usr/bin/env bash
# agent-guidance-eval.sh — does an orchestrator in (main) do its branch work in a
# place? (docs/proposals/agent-guidance.md §7)
#
#   scripts/agent-guidance-eval.sh <label> <worktrees-binary> [runs] [instructions-file]
#   scripts/agent-guidance-eval.sh --report
#
# Each run: a fresh throwaway repo, `claude -p` in its (main) with ONLY a
# worktrees MCP server (served by <worktrees-binary>) and a fixed task. The
# outcome is read from the transcript and the repo afterwards.
#
# Give <instructions-file> to try a DRAFT text without building a binary: a
# stdio proxy then rewrites `initialize.instructions` ({root} = the repo path).
#
# ⚠ MANUAL ONLY. It spends real tokens (~$0.2/run on Opus) and must never run
# in CI. Isolation, all of it load-bearing:
#   - repos, transcripts and a private tmux server live under $EVAL_ROOT
#     (default ~/.cache/worktrees/worktrees/agent-guidance-eval); TMUX is
#     unset so nothing reaches your real tmux server, which is killed at exit;
#   - WORKTREES_AI_CMD=none, so create_worktree never starts a nested agent;
#   - --strict-mcp-config, so your own MCP servers are not loaded;
#   - a tool allowlist: file tools, git, worktrees, and the worktrees MCP tools.
set -euo pipefail

EVAL_ROOT="${EVAL_ROOT:-$HOME/.cache/worktrees/worktrees/agent-guidance-eval}"
TMUX_DIR="$EVAL_ROOT/tmux"

# Every destructive path goes through this: non-empty, and under EVAL_ROOT.
safe_dir() {
  case "$1" in
    "$EVAL_ROOT"/?*) ;;
    *) echo "refusing to touch '$1' (not under $EVAL_ROOT)" >&2; exit 2 ;;
  esac
}

report() {
  python3 - "$EVAL_ROOT/runs" <<'PY'
import glob, json, os, re, sys
rows = {}
for d in sorted(glob.glob(os.path.join(sys.argv[1], "*"))):
    label, tools, cost = os.path.basename(d).rsplit("-", 1)[0], [], 0.0
    try:
        for line in open(os.path.join(d, "stream.jsonl")):
            m = json.loads(line)
            if m.get("type") == "assistant":
                for c in m["message"]["content"]:
                    if c.get("type") == "tool_use":
                        tools.append(c["name"] + ":" + c.get("input", {}).get("command", ""))
            if m.get("type") == "result":
                cost = m.get("total_cost_usd") or 0.0
        state = open(os.path.join(d, "state.txt")).read()
    except (OSError, ValueError):
        continue
    made = any(t.startswith("mcp__worktrees__create_worktree") or re.search(r"\bworktrees new\b", t) for t in tools)
    place = made and "/.worktrees/" in state
    raw = any(re.search(r"\bworktree\s+add\b", t) for t in tools)
    switched = any(re.search(r"checkout\s+-[bB]|switch\s+(-c|--create)", t) for t in tools)
    main_moved = re.search(r"^\* main$", state, re.M) is None
    r = rows.setdefault(label, dict(n=0, place=0, raw=0, switched=0, main_moved=0, cost=0.0))
    r["n"] += 1; r["place"] += place; r["raw"] += raw; r["switched"] += switched
    r["main_moved"] += main_moved; r["cost"] += cost
print(f"{'arm':24} {'runs':>4} {'place':>6} {'raw add':>8} {'checkout -b':>12} {'(main) moved':>13} {'mean $':>7}")
for label, r in rows.items():
    print(f"{label:24} {r['n']:>4} {r['place']:>6} {r['raw']:>8} {r['switched']:>12} {r['main_moved']:>13} {r['cost'] / r['n']:>7.2f}")
PY
}

if [ "${1:-}" = "--report" ]; then report; exit 0; fi
[ $# -ge 2 ] || { sed -n '2,8p' "$0"; exit 2; }
label=$1 bin=$2
runs=${3:-5} instr=${4:-}
[ -x "$bin" ] || { echo "not executable: $bin" >&2; exit 2; }
[ -z "$instr" ] || [ -f "$instr" ] || { echo "no such file: $instr" >&2; exit 2; }
command -v claude >/dev/null || { echo "claude not on PATH" >&2; exit 2; }
case "$label" in *[!A-Za-z0-9_.]*|'') echo "label: letters, digits, . and _ only" >&2; exit 2 ;; esac

mkdir -p "$EVAL_ROOT/runs" "$TMUX_DIR"
trap 'env -u TMUX TMUX_TMPDIR="$TMUX_DIR" tmux kill-server 2>/dev/null || true' EXIT

proxy="$EVAL_ROOT/proxy.py"
cat > "$proxy" <<'PY'
import json, os, subprocess, sys, threading
text = open(os.environ["EVAL_INSTR"]).read().strip()
p = subprocess.Popen([os.environ["EVAL_BIN"], "mcp", "--mutations"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, bufsize=0)
def up():
    for line in sys.stdin.buffer:
        p.stdin.write(line); p.stdin.flush()
    p.stdin.close()
threading.Thread(target=up, daemon=True).start()
for line in p.stdout:
    try:
        m = json.loads(line)
        r = m.get("result")
        if isinstance(r, dict) and "protocolVersion" in r and "instructions" in r:
            r["instructions"] = text.replace("{root}", os.getcwd())
            line = (json.dumps(m) + "\n").encode()
    except ValueError:
        pass
    sys.stdout.buffer.write(line); sys.stdout.buffer.flush()
PY

PROMPT='The README has a typo ("recieve") and greet.py says "Helo". Fix both and get the change ready for review as a PR: commit it on a new branch. There is no git remote here, so do not push or open the PR; just leave the branch committed and tell me its name.'

run_one() {
  local d=$1
  safe_dir "$d"
  rm -rf "$d"
  mkdir -p "$d/repo"
  git -C "$d/repo" init -q -b main
  printf '# Demo\n\nThis tool will recieve messages and print them.\n' > "$d/repo/README.md"
  printf 'def greet(name):\n    return "Helo, " + name\n' > "$d/repo/greet.py"
  git -C "$d/repo" add -A
  git -C "$d/repo" -c user.email=eval@example.invalid -c user.name=eval commit -qm init
  local env_json cmd args
  env_json="\"TMUX_TMPDIR\":\"$TMUX_DIR\",\"WORKTREES_AI_CMD\":\"none\",\"WORKTREES_PREFIX\":\"ev$2\""
  if [ -n "$instr" ]; then
    cmd=python3 args="[\"$proxy\"]"
    env_json="$env_json,\"EVAL_BIN\":\"$bin\",\"EVAL_INSTR\":\"$instr\""
  else
    cmd=$bin args='["mcp","--mutations"]'
  fi
  printf '{"mcpServers":{"worktrees":{"command":"%s","args":%s,"env":{%s}}}}\n' "$cmd" "$args" "$env_json" > "$d/mcp.json"
  (
    cd "$d/repo"
    env -u TMUX TMUX_TMPDIR="$TMUX_DIR" WORKTREES_AI_CMD=none WORKTREES_PREFIX="ev$2" PATH="$(dirname "$bin"):$PATH" \
      claude -p "$PROMPT" --mcp-config "$d/mcp.json" --strict-mcp-config --max-turns 30 \
      --allowedTools "Read,Edit,Write,Glob,Grep,Bash(git:*),Bash(worktrees:*),Bash(ls:*),Bash(cat:*),Bash(cd:*),mcp__worktrees__*" \
      --output-format stream-json --verbose < /dev/null > "$d/stream.jsonl" 2> "$d/stderr.txt" || true
    { git worktree list; echo; git branch; } > "$d/state.txt" 2>&1
  )
}

i=1
while [ "$i" -le "$runs" ]; do
  run_one "$EVAL_ROOT/runs/$label-$i" "$i" &
  i=$((i + 1))
done
wait
report
