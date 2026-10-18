#!/usr/bin/env bash
# The CLI the desktop app carries inside its own bundle: a PRIVATE sidecar at
# worktrees.app/Contents/MacOS/worktrees, for the app's use only (it will start
# `worktrees serve`; see crates/worktrees-api/src/lib.rs, "One engine process
# per host"). install.sh still puts the CLI on PATH, exactly as before; nothing
# outside the bundle points at this copy. Putting it on PATH is a later opt-in
# (ROADMAP, "Option A for the bundled CLI").
#
#   app/scripts/sidecar.sh stage [<target-triple>]
#       Build the CLI (release, --locked) and stage it where the bundle-time
#       overlay app/src-tauri/tauri.bundle.conf.json tells tauri to look:
#       app/src-tauri/binaries/worktrees-<triple>. With no triple it builds for
#       this host WITHOUT --target, to match a `tauri build` run without one.
#
#   app/scripts/sidecar.sh check <worktrees.app> <target-triple> <version>
#       Assert a bundle carries it: a regular executable file, of the target's
#       architecture and only that, at <version>. `--version` is RUN only when
#       the sidecar is native to this machine; a cross-arch one is never
#       executed (Rosetta on a runner is not something to lean on) and its
#       version is read as data instead, from the marker stale.rs embeds.
#
# WHY externalBin IS NOT IN tauri.conf.json
#
#   tauri-build (2.6.3) copies every externalBin from the app crate's BUILD
#   SCRIPT, in every profile. With it in the base config, `cargo check -p app`
#   on a clean checkout fails — measured: "resource path
#   `binaries/worktrees-aarch64-apple-darwin` doesn't exist" — and so would
#   `cargo test -p app --lib`, `tauri dev` and CI's app build on both OSes, none
#   of which should need a CLI build first. So it lives in an overlay that only
#   the two things that make a bundle pass with `--config`: release.yml and
#   `make install-app`. Both also run `check` afterwards, so a bundle made
#   without the overlay cannot pass for one made with it.
#
# WHY stage MUST USE THE SAME --target CHOICE AS THE tauri build
#
#   That build-script copy lands on target/[<triple>/]<profile>/worktrees — the
#   CLI's OWN cargo output, the binary bin/worktrees and bats prefer. It deletes
#   that file and copies the staged one into its place. Staging the build made
#   seconds earlier, from the same target dir, turns that into a copy of a file
#   onto itself. Staging anything else (an older build, a downloaded release)
#   would silently replace your CLI, with a fresh mtime that makes every `-nt`
#   freshness check pass. That is also why the staged file is a real copy and
#   not a symlink into target/: the destination is deleted before the copy.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
STAGE_DIR="$ROOT/app/src-tauri/binaries"
NAME=worktrees   # = the overlay's `binaries/worktrees`, minus the directory

die() {
  if [ "${GITHUB_ACTIONS:-}" = true ]; then echo "::error::sidecar: $*"
  else echo "sidecar: $*" >&2; fi
  exit 1
}

# The architecture `lipo -archs` reports for a bundle built for <triple>.
arch_of() {
  case "$1" in
    aarch64-apple-darwin) echo arm64 ;;
    x86_64-apple-darwin)  echo x86_64 ;;
    *) die "no macOS app bundle is built for '$1'" ;;
  esac
}

# The version a CLI binary carries, read as DATA (never by running it): the
# `\0WORKTREES_CLI_VERSION=<v>\0` record in crates/worktrees-cli/src/stale.rs.
# Read the way stale.rs's own `read_version` reads it — NUL on BOTH sides and a
# numeric x.y.z core — because the scanner's needle, the same prefix with no
# value and no closing NUL, is in the binary too: a bare grep for the prefix
# finds two and the empty one fails the check. So: newlines out of the way,
# NULs become line ends, and only a WHOLE segment matches. Prints every
# distinct value found, so a binary carrying two cannot pass for either.
marker_version() {
  LC_ALL=C tr '\n\0' '\001\n' < "$1" \
    | { LC_ALL=C grep -a -x -E 'WORKTREES_CLI_VERSION=[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.+-]*)?' || true; } \
    | sed 's/^WORKTREES_CLI_VERSION=//' | sort -u
}

stage() {
  local triple="${1:-}" built
  if [ -n "$triple" ]; then
    cargo build --manifest-path "$ROOT/Cargo.toml" --release -p worktrees-cli --locked --target "$triple"
    built="$TARGET_DIR/$triple/release/$NAME"
  else
    triple="$(rustc -vV | sed -n 's/^host: //p')"
    [ -n "$triple" ] || die "rustc -vV reported no host triple"
    cargo build --manifest-path "$ROOT/Cargo.toml" --release -p worktrees-cli --locked
    built="$TARGET_DIR/release/$NAME"
  fi
  [ -f "$built" ] || die "cargo finished but there is no CLI at $built"
  mkdir -p "$STAGE_DIR"
  local staged="$STAGE_DIR/$NAME-$triple"
  rm -f "$staged"
  # -p keeps the build's mtime, so tauri-build's rerun-if-changed on this file
  # (and the app recompile behind it) fires only when the CLI actually changed.
  cp -p "$built" "$staged"
  chmod 0755 "$staged"
  echo "staged: ${staged#"$ROOT/"} (worktrees $(marker_version "$staged"), $triple)"
}

check() {
  [ $# -eq 3 ] || die "usage: sidecar.sh check <worktrees.app> <target-triple> <version>"
  local app="$1" triple="$2" want="$3"
  local sc="$app/Contents/MacOS/$NAME" arch got main marks host out
  arch="$(arch_of "$triple")"
  [ -d "$app/Contents/MacOS" ] || die "no app bundle at $app"
  [ ! -L "$sc" ] || die "Contents/MacOS/$NAME is a symlink, not the binary ($(readlink "$sc"))"
  [ -f "$sc" ] || die "no sidecar at Contents/MacOS/$NAME — the bundle has: $(cd "$app/Contents/MacOS" && echo *)"
  [ -x "$sc" ] || die "the sidecar is not executable ($(stat -f '%Sp' "$sc"))"

  got="$(lipo -archs "$sc")" || die "lipo cannot read the sidecar"
  [ "$got" = "$arch" ] || die "the sidecar is '$got'; a $triple bundle needs '$arch'"
  main="$(lipo -archs "$app/Contents/MacOS/app")" || die "lipo cannot read Contents/MacOS/app"
  [ "$main" = "$got" ] || die "the sidecar is '$got' but the app beside it is '$main'"

  marks="$(marker_version "$sc")"
  [ "$marks" = "$want" ] \
    || die "the sidecar's version marker says '$(printf '%s' "$marks" | tr '\n' ' ')', expected $want"

  # Run it only on its own architecture. Never Contents/MacOS/app: that is the
  # GUI entry point, and it launches a second instance instead of answering.
  host="$(uname -m)"
  if [ "$host" = "$arch" ]; then
    out="$("$sc" --version </dev/null)" || die "the sidecar's --version exited non-zero"
    [ "$out" = "worktrees $want" ] || die "the sidecar's --version says '$out', expected 'worktrees $want'"
    echo "sidecar ok: Contents/MacOS/$NAME, $got, ran --version: $out"
  else
    echo "sidecar ok: Contents/MacOS/$NAME, $got, version marker $marks (not run: $arch binary, $host host)"
  fi
}

case "${1:-}" in
  stage) shift; stage "$@" ;;
  check) shift; check "$@" ;;
  *) die "usage: sidecar.sh stage [<target-triple>] | check <worktrees.app> <target-triple> <version>" ;;
esac
