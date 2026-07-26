#!/usr/bin/env bash
# worktrees installer — usage:
#   curl -fsSL https://raw.githubusercontent.com/penard-monkey/worktrees/main/install.sh | bash
# Recommended (reproducible) team form — pin the tag:
#   curl -fsSL https://raw.githubusercontent.com/penard-monkey/worktrees/v0.1.0/install.sh | bash
#
# Installs the LATEST RELEASE (not main) to ~/.local/bin/worktrees. worktrees is
# a compiled binary: this fetches the prebuilt binary for your platform, or (if
# none matches, or you set WORKTREES_INSTALL_FROM_SOURCE=1) builds it from source
# with cargo.
#   WORKTREES_INSTALL_VERSION=v0.1.0     pin a specific release
#   WORKTREES_INSTALL_DIR=~/bin          alternate target dir
#   WORKTREES_INSTALL_FROM_SOURCE=1      force a cargo build from source
#   WORKTREES_INSTALL_APP=1|0            also install the macOS desktop app / never ask
#   install.sh --with-app                same as WORKTREES_INSTALL_APP=1
#   install.sh --uninstall               remove the installed binary (only that)
#
# Desktop app (macOS): with a terminal attached the installer OFFERS to install
# worktrees.app to /Applications (checksum-verified, then the quarantine attr is
# stripped — the bundle is unsigned; you explicitly chose to install it).
#
# Checksums: a downloaded binary is verified against the release's checksums.txt.
# This protects against truncation/corruption — it does NOT prove authorship (the
# checksums come from the same repo). curl|bash of this installer is
# trust-on-first-use; pin the tag form above once you trust it.
set -euo pipefail

REPO="penard-monkey/worktrees"
BIN_NAME="worktrees"

sha256_check() {   # reads "<hash>  <name>" on stdin, verifies <name> in cwd
  if command -v sha256sum >/dev/null 2>&1; then sha256sum -c -
  else shasum -a 256 -c -; fi
}

detect_triple() {  # Rust target triple for this host, or "" if unknown
  case "$(uname -s)/$(uname -m)" in
    Darwin/arm64)               echo aarch64-apple-darwin ;;
    Darwin/x86_64)              echo x86_64-apple-darwin ;;
    Linux/x86_64)               echo x86_64-unknown-linux-gnu ;;
    Linux/aarch64|Linux/arm64)  echo aarch64-unknown-linux-gnu ;;
    *)                          echo "" ;;
  esac
}

install_app() {  # $1 version  $2 release download url  $3 tmp dir
  local version="$1" dl="$2" tmp="$3"
  local triple; triple="$(detect_triple)"
  local asset="worktrees-app-$triple.app.tar.gz"
  echo "fetching $asset ..."
  if ! curl -fsSL -o "$tmp/$asset" "$dl/$asset" 2>/dev/null; then
    echo "NOTE: release $version has no desktop-app bundle ($asset) — app install skipped." >&2
    echo "      From a clone: make install-app" >&2
    return 0
  fi
  if [ -f "$tmp/checksums.txt" ] || curl -fsSL -o "$tmp/checksums.txt" "$dl/checksums.txt" 2>/dev/null; then
    ( cd "$tmp" && grep "  $asset\$" checksums.txt | sha256_check ) \
      || { echo "ERROR: app checksum verification FAILED" >&2; exit 1; }
    echo "app checksum ok (integrity only — not authorship; see header note)"
  else
    echo "WARNING: no checksums.txt on release $version — skipping app verification" >&2
  fi
  local appdir="/Applications"
  [ -w "$appdir" ] || appdir="$HOME/Applications"
  mkdir -p "$appdir"
  tar -xzf "$tmp/$asset" -C "$tmp"
  # Downloaded bundles carry the quarantine attr and the app is unsigned, so
  # Gatekeeper would refuse it. The user explicitly opted into installing our
  # unsigned build; integrity was just checksum-verified — strip the attr.
  xattr -cr "$tmp/worktrees.app" 2>/dev/null || true
  rm -rf "$appdir/worktrees.app"
  ditto "$tmp/worktrees.app" "$appdir/worktrees.app"
  echo "installed: $appdir/worktrees.app  (open -a worktrees)"
  if pgrep -f "worktrees.app/Contents/MacOS" >/dev/null 2>&1; then
    echo "NOTE: worktrees.app is currently running — quit + reopen to pick up the new version."
  fi
}

main() {
  local dir="${WORKTREES_INSTALL_DIR:-$HOME/.local/bin}"
  local target="$dir/$BIN_NAME"

  # "" = ask when a terminal is attached · 1 = install · 0 = never
  local with_app="${WORKTREES_INSTALL_APP:-}"
  if [ "${1:-}" = "--with-app" ]; then with_app=1; fi

  if [ "${1:-}" = "--uninstall" ]; then
    if [ -e "$target" ] || [ -L "$target" ]; then
      rm -f "$target"; echo "removed $target (worktrees/.worktrees data in your repos is untouched)"
    else
      echo "nothing installed at $target"
    fi
    if [ -d "/Applications/worktrees.app" ] || [ -d "$HOME/Applications/worktrees.app" ]; then
      echo "desktop app left in place — remove with: rm -rf /Applications/worktrees.app"
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
    echo "ERROR: $target is a symlink (managed by a git clone — upgrade with 'git pull' + 'make install' there)." >&2
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

  # TMP_DIR is a global on purpose: the EXIT trap fires after main() returns,
  # when a `local` would already be out of scope (set -u would abort cleanup).
  TMP_DIR="$(mktemp -d)"
  trap 'rm -rf "${TMP_DIR:-}"' EXIT
  local tmp="$TMP_DIR"
  local old=""
  [ -x "$target" ] && old="$("$target" --version 2>/dev/null || true)"
  mkdir -p "$dir"

  local triple; triple="$(detect_triple)"
  local dl="https://github.com/$REPO/releases/download/$version"
  local installed_via=""

  # ── prefer a prebuilt binary; fall back to building from source ─────────────
  if [ "${WORKTREES_INSTALL_FROM_SOURCE:-}" != "1" ] && [ -n "$triple" ] \
     && curl -fsSL -o "$tmp/worktrees-$triple" "$dl/worktrees-$triple" 2>/dev/null; then
    local asset="worktrees-$triple"
    if curl -fsSL -o "$tmp/checksums.txt" "$dl/checksums.txt" 2>/dev/null; then
      ( cd "$tmp" && grep "  $asset\$" checksums.txt | sha256_check ) \
        || { echo "ERROR: checksum verification FAILED" >&2; exit 1; }
      echo "checksum ok (integrity only — not authorship; see header note)"
    else
      echo "WARNING: no checksums.txt on release $version — skipping verification" >&2
    fi
    chmod +x "$tmp/$asset"
    mv -f "$tmp/$asset" "$target"
    installed_via="prebuilt $triple"
  else
    # from source
    command -v cargo >/dev/null 2>&1 || {
      echo "ERROR: no prebuilt binary for $(uname -s)/$(uname -m) at $version, and cargo not found to build from source." >&2
      echo "       Install Rust (https://rustup.rs) and re-run, or: git clone https://github.com/$REPO && cd worktrees && make install" >&2
      exit 1
    }
    echo "building worktrees $version from source with cargo ..."
    git clone --depth 1 --branch "$version" "https://github.com/$REPO" "$tmp/src" >/dev/null 2>&1 \
      || { echo "ERROR: git clone of $version failed" >&2; exit 1; }
    ( cd "$tmp/src" && cargo build --release -p worktrees-cli ) \
      || { echo "ERROR: cargo build failed" >&2; exit 1; }
    chmod +x "$tmp/src/target/release/worktrees"
    mv -f "$tmp/src/target/release/worktrees" "$target"
    installed_via="source build"
  fi

  echo "installed: $target ($("$target" --version)) [$installed_via]"
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

  # ── desktop app (macOS) ────────────────────────────────────────────────────
  if [ "$(uname -s)" = Darwin ]; then
    if [ -z "$with_app" ]; then
      # curl|bash: stdin is the script itself — ask on the controlling terminal.
      # ([ -r /dev/tty ] passes even with NO controlling tty; actually open it.)
      if { : < /dev/tty && : > /dev/tty; } 2>/dev/null; then
        printf '\nAlso install the desktop app (worktrees.app -> /Applications)? [y/N] ' > /dev/tty
        ans=""; IFS= read -r ans < /dev/tty || ans=""
        case "$ans" in y|Y|yes|YES) with_app=1 ;; *) with_app=0 ;; esac
      else
        echo ""
        echo "NOTE: desktop app not installed — rerun with WORKTREES_INSTALL_APP=1 (or --with-app) to add worktrees.app."
      fi
    fi
    if [ "$with_app" = 1 ]; then
      install_app "$version" "$dl" "$tmp"
    fi
  fi
}

main "$@"
