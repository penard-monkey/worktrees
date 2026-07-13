#!/usr/bin/env bash
# worktrees installer — usage:
#   curl -fsSL https://raw.githubusercontent.com/penard-monkey/worktrees/main/install.sh | bash
# Recommended (reproducible) team form — pin the tag:
#   curl -fsSL https://raw.githubusercontent.com/penard-monkey/worktrees/v0.1.0/install.sh | bash
#
# Installs the LATEST RELEASE (not main) to ~/.local/bin/worktrees.
#   WORKTREES_INSTALL_VERSION=v0.1.0   pin a specific release
#   WORKTREES_INSTALL_DIR=~/bin        alternate target dir
#   install.sh --uninstall             remove the installed binary (only that)
#
# Checksums: the payload is verified against the release's checksums.txt. This
# protects against truncation/corruption — it does NOT prove authorship (the
# checksums come from the same repo). curl|bash of this installer is
# trust-on-first-use; pin the tag form above once you trust it.
set -euo pipefail

REPO="penard-monkey/worktrees"
BIN_NAME="worktrees"

main() {
  local dir="${WORKTREES_INSTALL_DIR:-$HOME/.local/bin}"
  local target="$dir/$BIN_NAME"

  if [ "${1:-}" = "--uninstall" ]; then
    if [ -e "$target" ] || [ -L "$target" ]; then
      rm -f "$target"; echo "removed $target (worktrees/.worktrees data in your repos is untouched)"
    else
      echo "nothing installed at $target"
    fi
    return 0
  fi

  # ── preflight ──────────────────────────────────────────────────────────────
  command -v curl >/dev/null 2>&1 || { echo "ERROR: curl is required" >&2; exit 1; }
  if [ "$(uname -s)" = Darwin ] && ! xcode-select -p >/dev/null 2>&1; then
    echo "ERROR: Xcode Command Line Tools missing (git lives there). Run: xcode-select --install" >&2; exit 1
  fi
  command -v git >/dev/null 2>&1 || { echo "ERROR: git is required (>= 2.23). macOS: xcode-select --install · debian: apt install git" >&2; exit 1; }
  git_ver="$(git --version | sed 's/[^0-9.]*\([0-9][0-9.]*\).*/\1/')"
  git_major="${git_ver%%.*}"; rest="${git_ver#*.}"; git_minor="${rest%%.*}"
  if [ "$git_major" -lt 2 ] || { [ "$git_major" -eq 2 ] && [ "$git_minor" -lt 23 ]; }; then
    echo "ERROR: git >= 2.23 required (found $git_ver — 'git switch' is missing)" >&2; exit 1
  fi
  if command -v tmux >/dev/null 2>&1; then
    tmux_ver="$(tmux -V 2>/dev/null | sed 's/[^0-9.]*\([0-9][0-9.]*\).*/\1/')"
    case "$tmux_ver" in
      0.*|1.[0-8]|1.[0-8].*) echo "WARNING: tmux $tmux_ver is older than 1.9 — sessions may fail; upgrade tmux" >&2 ;;
    esac
  else
    echo "NOTE: tmux not found — 'new' will degrade to --no-tmux, 'open' needs tmux. macOS: brew install tmux · debian: apt install tmux" >&2
  fi

  # Existing clone-managed symlink? Don't fight `make install`.
  if [ -L "$target" ] && [ "${WORKTREES_INSTALL_FORCE:-}" != "1" ]; then
    echo "ERROR: $target is a symlink (managed by a git clone — upgrade with 'git pull' there)." >&2
    echo "       Set WORKTREES_INSTALL_FORCE=1 to overwrite with a copy." >&2
    exit 1
  fi

  # ── resolve version ────────────────────────────────────────────────────────
  local version="${WORKTREES_INSTALL_VERSION:-}"
  if [ -z "$version" ]; then
    # releases/latest redirects to .../tag/vX.Y.Z — no API, no rate limit, no jq.
    version="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest" | sed 's|.*/tag/||')"
    case "$version" in
      v[0-9]*) : ;;
      *) echo "ERROR: could not resolve latest release (got '$version'). Is the repo public with a release?" >&2; exit 1 ;;
    esac
  fi

  # ── download + verify + install atomically ────────────────────────────────
  # TMP_DIR is a global on purpose: the EXIT trap fires after main() returns,
  # when a `local` would already be out of scope (set -u would abort cleanup).
  TMP_DIR="$(mktemp -d)"
  trap 'rm -rf "${TMP_DIR:-}"' EXIT
  local tmp="$TMP_DIR"
  echo "downloading worktrees $version ..."
  curl -fsSL -o "$tmp/$BIN_NAME" "https://raw.githubusercontent.com/$REPO/$version/bin/worktrees"
  if curl -fsSL -o "$tmp/checksums.txt" "https://github.com/$REPO/releases/download/$version/checksums.txt" 2>/dev/null; then
    ( cd "$tmp" && grep " $BIN_NAME\$" checksums.txt | shasum -a 256 -c - ) \
      || { echo "ERROR: checksum verification FAILED" >&2; exit 1; }
    echo "checksum ok (integrity only — not authorship; see header note)"
  else
    echo "WARNING: no checksums.txt on release $version — skipping verification" >&2
  fi
  head -n 1 "$tmp/$BIN_NAME" | grep -q '^#!' || { echo "ERROR: download doesn't look like a script" >&2; exit 1; }

  local old=""
  [ -x "$target" ] && old="$("$target" --version 2>/dev/null || true)"
  mkdir -p "$dir"
  chmod +x "$tmp/$BIN_NAME"
  mv -f "$tmp/$BIN_NAME" "$target"      # atomic on same fs; tmp→$dir may cross devices, mv still safe-ish for a single file
  echo "installed: $target ($("$target" --version))"
  [ -n "$old" ] && echo "upgraded from: $old"

  # ── PATH + shadowing checks ────────────────────────────────────────────────
  case ":$PATH:" in
    *:"$dir":*) : ;;
    *)
      rc=".bashrc"; case "${SHELL:-}" in */zsh) rc=".zshrc" ;; esac
      echo ""
      echo "NOTE: $dir is not on your PATH. Add it (then restart your shell):"
      echo "  echo 'export PATH=\"$dir:\$PATH\"' >> ~/$rc"
      ;;
  esac
  # An earlier PATH hit (e.g. an old ~/bin/worktrees symlink to a repo script) silently wins.
  local first
  first="$(command -v "$BIN_NAME" 2>/dev/null || true)"
  if [ -n "$first" ] && [ "$first" != "$target" ]; then
    echo "WARNING: '$BIN_NAME' currently resolves to $first (earlier on PATH than $target)." >&2
    echo "         Remove/rename it or reorder PATH, or you'll keep running the old one." >&2
  fi
}

main "$@"
