#!/usr/bin/env bash
# make-samples.sh — scaffold throwaway git repos + worktrees in varied lifecycle
# states, so the worktrees app has rich REAL data to open during design/dev.
#
#   ./scripts/make-samples.sh [TARGET_DIR]      (default: ~/worktrees-samples)
#   ./scripts/make-samples.sh --clean [TARGET]  wipe + regenerate
#
# Emits real git worktrees under <repo>/.worktrees/<slug> plus a declared-state
# sidecar (.worktrees.places.json, schema the core reads) so every lifecycle
# group shows up. tmux is NOT started (so nothing is "active" until you Open one).
# bash-3.2 safe (no assoc arrays). Nothing here touches the worktrees repo itself.
set -euo pipefail

TARGET="$HOME/worktrees-samples"
CLEAN=0
for a in "$@"; do
  case "$a" in
    --clean) CLEAN=1 ;;
    *) TARGET="$a" ;;
  esac
done

if [ "$CLEAN" = 1 ] && [ -d "$TARGET" ]; then
  echo "cleaning $TARGET"
  rm -rf "$TARGET"
fi
mkdir -p "$TARGET"

now=$(date +%s)
day=86400

# git that never depends on the caller's global identity / hooks / templates
g() { git -c user.name='Worktrees Sample' -c user.email='sample@example.com' -c commit.gpgsign=false -c init.defaultBranch=main "$@"; }

commit() { # <dir> <msg>
  g -C "$1" add -A
  g -C "$1" commit -q -m "$2"
}

init_repo() { # <dir>
  local d="$1"
  mkdir -p "$d"
  g -C "$d" init -q
  printf '# %s\n\nsample project.\n' "$(basename "$d")" > "$d/README.md"
  mkdir -p "$d/src"
  printf 'export const version = "0.1.0";\n' > "$d/src/index.ts"
  commit "$d" "initial commit"
  printf 'export const feature = () => true;\n' > "$d/src/feature.ts"
  commit "$d" "add feature module"
}

add_wt() { # <repo> <slug> <branch>
  local repo="$1" slug="$2" branch="$3"
  g -C "$repo" worktree add -q -b "$branch" ".worktrees/$slug" >/dev/null
}

make_dirty() { # <repo> <slug>
  printf '\n// wip %s\n' "$(date +%s)" >> "$1/.worktrees/$2/src/feature.ts"
}

wt_commit() { # <repo> <slug> <msg>  (advances the branch; shows recent commit)
  printf '\n// %s\n' "$3" >> "$1/.worktrees/$2/src/index.ts"
  commit "$1/.worktrees/$2" "$3"
}

# ── sample-web: the rich one — one place per lifecycle group ──────────────────
WEB="$TARGET/sample-web"
init_repo "$WEB"
add_wt "$WEB" messaging        feat/messaging-sse
add_wt "$WEB" billing-refactor feat/billing-v2
add_wt "$WEB" search-index     feat/search-opensearch
add_wt "$WEB" hotfix-login     fix/login-loop
add_wt "$WEB" legacy-migration chore/knex-to-prisma
add_wt "$WEB" spike-graphql    spike/graphql

wt_commit "$WEB" messaging "wire up SSE reconnect"
make_dirty "$WEB" messaging
wt_commit "$WEB" billing-refactor "extract invoice service"
make_dirty "$WEB" hotfix-login

cat > "$WEB/.worktrees.places.json" <<JSON
{
  "version": 1,
  "updated_epoch": $now,
  "places": {
    "messaging":        { "lifecycle": "saved",     "pinned": true, "note": "auth refactor place",     "last_opened_epoch": $((now - day)) },
    "billing-refactor": { "note": "invoicing v2",                                                       "last_opened_epoch": $((now - 3600)) },
    "search-index":     { "note": "waiting on infra ticket",                                            "last_opened_epoch": $((now - 2 * day)) },
    "hotfix-login":     { "lifecycle": "closed",                                                        "last_opened_epoch": $((now - 20 * day)) },
    "legacy-migration": { "lifecycle": "archived",  "note": "resume Q3",                                "last_opened_epoch": $((now - 40 * day)) },
    "spike-graphql":    { "lifecycle": "abandoned",                                                     "last_opened_epoch": $((now - 60 * day)) }
  }
}
JSON

# ── sample-api: a small second project (multi-project tree) ───────────────────
API="$TARGET/sample-api"
init_repo "$API"
add_wt "$API" payments feat/stripe-webhooks
add_wt "$API" ratelimit fix/rate-limit
wt_commit "$API" payments "handle charge.refunded"
make_dirty "$API" payments

cat > "$API/.worktrees.places.json" <<JSON
{
  "version": 1,
  "updated_epoch": $now,
  "places": {
    "payments":  { "pinned": true, "note": "stripe webhooks", "last_opened_epoch": $((now - 1800)) },
    "ratelimit": { "lifecycle": "closed",                     "last_opened_epoch": $((now - 12 * day)) }
  }
}
JSON

echo
echo "sample projects created under: $TARGET"
echo "  - $WEB   (6 worktrees, all lifecycle groups)"
echo "  - $API   (2 worktrees)"
echo
echo "Open them in the app via ＋ add, or point the CLI:  worktrees ls  (run inside a repo)"
