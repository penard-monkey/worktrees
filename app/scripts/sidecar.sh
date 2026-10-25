#!/usr/bin/env bash
# The CLI the desktop app carries inside its own bundle: a PRIVATE sidecar, for
# the app's use only (it starts `worktrees serve` with it; see
# app/src-tauri/src/engine.rs, and crates/worktrees-api/src/lib.rs, "One engine
# process per host"). install.sh
# still puts the CLI on PATH, exactly as before; nothing outside the bundle
# points at this copy. Putting it on PATH is a later opt-in (ROADMAP, "Option A
# for the bundled CLI"). Running AS this copy, current_exe() is the bundle, so
# core's profile::is_bundled_cli knows the paths below and worktrees_bin() never
# hands them to an MCP config, a hook or a pane; a core test reads the `files`
# mappings, so moving one without the rule fails it.
#
# Where each format carries it, which is where the app is to look:
#   macOS     worktrees.app/Contents/MacOS/worktrees   beside current_exe()
#   .deb      /usr/lib/worktrees/worktrees             resource_dir()/worktrees
#   AppImage  $APPDIR/usr/lib/worktrees/worktrees      resource_dir()/worktrees
# tauri resolves a Linux resource_dir as <exe dir>/../lib/<productName>, so both
# Linux rows are one lookup. The GUI binary is Contents/MacOS/app on macOS and
# usr/bin/worktrees-app on Linux (tauri.linux.conf.json renames it there: a
# .deb would otherwise install /usr/bin/app).
#
#   app/scripts/sidecar.sh stage [<target-triple>]
#       Build the CLI (release, --locked) and copy it to the one path all three
#       `files` mappings in tauri.conf.json read: app/src-tauri/binaries/worktrees.
#       With no triple it builds for this host.
#
#   app/scripts/sidecar.sh check <bundle> <target-triple> <version>
#       <bundle> is a worktrees.app directory (macOS), or an .AppImage or .deb
#       (Linux), which are UNPACKED, never run. Asserts the sidecar is a regular
#       executable file, of the target's architecture and only that (`lipo`, or
#       the ELF header), the same as the GUI binary beside it, at <version>.
#       On macOS it also verifies the complete bundle's signature and resource
#       seal; without it Finder can report a browser download as damaged. On
#       Linux it asserts that nothing called worktrees is in usr/bin. `--version` is
#       RUN only when the sidecar is native to this machine; a cross-arch one is
#       never executed and its version is read as data instead, from the marker
#       stale.rs embeds.
#
#   app/scripts/sidecar.sh unpack <.AppImage|.deb> <empty-or-new-dir>
#       Unpack a Linux bundle without running anything in it (check uses this;
#       so does release.yml's viewer assertion). Needs readelf + unsquashfs, or
#       dpkg-deb.
#
# WHY `files` MAPPINGS AND NOT externalBin, tauri's own sidecar key
#
#   externalBin is wrong on every platform this ships on, for two reasons.
#   tauri-build copies it from the app crate's BUILD SCRIPT, in every profile:
#   in tauri.conf.json, a clean `cargo check -p app` fails ("resource path
#   `binaries/worktrees-aarch64-apple-darwin` doesn't exist"), so check, test,
#   `tauri dev` and CI would all need a CLI build first — and the copy lands on
#   target/<profile>/worktrees, the CLI's own cargo output. And on Linux it
#   goes in usr/bin: /usr/bin/worktrees from a .deb, on the system PATH beside
#   install.sh's copy, and $APPDIR/usr/bin in an AppImage, which AppRun puts
#   FIRST on the app's PATH. Measured both ways: the booted app's MCP setup
#   resolved `worktrees` to the sidecar.
#
#   The `files` mappings (bundle.macOS / linux.deb / linux.appimage) are read
#   by the bundler alone. They sit in the base config with nothing for cargo to
#   trip on, and a `tauri build` with nothing staged fails at the bundler, so a
#   bundle cannot be made without the sidecar. `check` still runs after every
#   build: a mapping lost in a merge would otherwise only be noticed by a user.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
STAGED="$ROOT/app/src-tauri/binaries/worktrees"   # tauri.conf.json's `binaries/worktrees`
NAME=worktrees

die() {
  if [ "${GITHUB_ACTIONS:-}" = true ]; then echo "::error::sidecar: $*"
  else echo "sidecar: $*" >&2; fi
  exit 1
}

# The architecture a bundle built for <triple> must carry, spelled as `uname -m`
# spells it on that OS (arm64 on macOS, aarch64 on Linux), so it can be compared
# with the host directly. Fails rather than dies: callers read it through
# `$(…)`, where a GitHub `::error::` line would be captured, not printed.
arch_of() {
  case "$1" in
    aarch64-apple-darwin)      echo arm64 ;;
    x86_64-apple-darwin)       echo x86_64 ;;
    aarch64-unknown-linux-gnu) echo aarch64 ;;
    x86_64-unknown-linux-gnu)  echo x86_64 ;;
    *) return 1 ;;
  esac
}

# A binary's architecture, the same spelling as arch_of: `lipo -archs` for a
# Mach-O (several words for a fat one, which then matches nothing), the ELF
# header's Machine for Linux. The `|| true`s are not decoration: whether `-e`
# reaches a command substitution differs between bash 3.2 and 5.
bin_arch() {   # $1 file  $2 macos|linux
  local m
  if [ "$2" = macos ]; then
    lipo -archs "$1" 2>/dev/null || echo "unreadable"
    return 0
  fi
  m="$(LC_ALL=C readelf -h "$1" 2>/dev/null | sed -n 's/^ *Machine: *//p' || true)"
  case "$m" in
    AArch64)                          echo aarch64 ;;
    "Advanced Micro Devices X86-64")  echo x86_64 ;;
    "")                               echo "not-an-ELF-file" ;;
    *)                                echo "$m" ;;
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

perms_of() { stat -c '%A' "$1" 2>/dev/null || stat -f '%Sp' "$1"; }

# Where an AppImage's squashfs begins: the end of its runtime, an ELF file
# whose section header table is its last part (e_shoff + e_shentsize*e_shnum).
# Computed rather than asked for with `--appimage-offset`, which RUNS the
# runtime — impossible for a cross-arch AppImage, and pointless for the rest.
appimage_offset() {
  { LC_ALL=C readelf -h "$1" 2>/dev/null || true; } | awk -F: '
    /Start of section headers/  { s = $2 + 0 }
    /Size of section headers/   { z = $2 + 0 }
    /Number of section headers/ { n = $2 + 0 }
    END { if (s > 0 && z > 0 && n > 0) print s + z * n }'
}

stage() {
  local triple="${1:-}" built
  if [ -n "$triple" ]; then
    arch_of "$triple" >/dev/null || die "no desktop app bundle is built for '$triple'"
    cargo build --manifest-path "$ROOT/Cargo.toml" --release -p worktrees-cli --locked --target "$triple"
    built="$TARGET_DIR/$triple/release/$NAME"
  else
    triple="$(rustc -vV | sed -n 's/^host: //p')"
    [ -n "$triple" ] || die "rustc -vV reported no host triple"
    arch_of "$triple" >/dev/null || die "no desktop app bundle is built for this host ($triple)"
    cargo build --manifest-path "$ROOT/Cargo.toml" --release -p worktrees-cli --locked
    built="$TARGET_DIR/release/$NAME"
  fi
  [ -f "$built" ] || die "cargo finished but there is no CLI at $built"
  mkdir -p "$(dirname "$STAGED")"
  # rm first: cp onto a symlink would write through it.
  rm -f "$STAGED"
  cp "$built" "$STAGED"
  chmod 0755 "$STAGED"
  # tauri seals a Mac bundle, and codesign refuses to seal one holding unsigned
  # code ("code object is not signed at all / In subcomponent: …/worktrees").
  # The linker signs an arm64 build by itself; a cross-built x86_64 one has no
  # signature at all, and v0.44.1's first release run died on exactly that.
  # Ad-hoc sign it here for both, so the bundle step never meets bare code.
  case "$triple" in
    *-apple-darwin) codesign --force --sign - "$STAGED" || die "could not ad-hoc sign $STAGED" ;;
  esac
  echo "staged: ${STAGED#"$ROOT/"} (worktrees $(marker_version "$STAGED"), $triple)"
}

# Everything after "where is it": the same assertions for every format.
verify() {   # $1 sidecar  $2 its path as shown  $3 GUI binary  $4 macos|linux  $5 arch  $6 triple  $7 version
  local sc="$1" shown="$2" main="$3" os="$4" arch="$5" triple="$6" want="$7"
  local got gui marks host out dir
  dir="$(dirname "$sc")"
  [ ! -L "$sc" ] || die "$shown is a symlink, not the binary ($(readlink "$sc"))"
  if [ ! -f "$sc" ]; then
    if [ -d "$dir" ]; then die "no sidecar at $shown — beside it: $(cd "$dir" && echo *)"
    else die "no sidecar at $shown — there is no $(dirname "$shown") at all"; fi
  fi
  [ -x "$sc" ] || die "$shown is not executable ($(perms_of "$sc"))"

  got="$(bin_arch "$sc" "$os")"
  [ "$got" = "$arch" ] || die "$shown is '$got'; a bundle for $triple needs '$arch'"
  [ -f "$main" ] || die "no app binary beside it, at ${main##*/}"
  gui="$(bin_arch "$main" "$os")"
  [ "$gui" = "$got" ] || die "$shown is '$got' but the app binary is '$gui'"

  marks="$(marker_version "$sc")"
  [ "$marks" = "$want" ] \
    || die "$shown's version marker says '$(printf '%s' "$marks" | tr '\n' ' ')', expected $want"

  # Run it only on its own architecture. Never the GUI binary: on macOS that
  # launches a second instance instead of answering.
  host="$(uname -m)"
  if [ "$host" = "$arch" ]; then
    out="$("$sc" --version </dev/null)" || die "$shown --version exited non-zero"
    [ "$out" = "worktrees $want" ] || die "$shown --version says '$out', expected 'worktrees $want'"
    echo "sidecar ok: $shown, $got, ran --version: $out"
  else
    echo "sidecar ok: $shown, $got, version marker $marks (not run: $arch binary, $host host)"
  fi
}

unpack() {
  [ $# -eq 2 ] || die "usage: sidecar.sh unpack <.AppImage|.deb> <dir>"
  local pkg="$1" dest="$2" off
  [ -f "$pkg" ] || die "no package at $pkg"
  case "$pkg" in
    *.AppImage)
      command -v readelf >/dev/null 2>&1 || die "readelf not found (binutils)"
      command -v unsquashfs >/dev/null 2>&1 || die "unsquashfs not found (squashfs-tools)"
      off="$(appimage_offset "$pkg")"
      [ -n "$off" ] || die "${pkg##*/} has no ELF runtime header — not an AppImage"
      unsquashfs -n -d "$dest" -o "$off" "$pkg" >/dev/null \
        || die "cannot unpack the squashfs in ${pkg##*/} (offset $off)"
      ;;
    *.deb)
      command -v dpkg-deb >/dev/null 2>&1 || die "dpkg-deb not found"
      dpkg-deb -x "$pkg" "$dest" || die "cannot unpack ${pkg##*/}"
      ;;
    *) die "a Linux bundle is an .AppImage or a .deb, not ${pkg##*/}" ;;
  esac
}

check() {
  [ $# -eq 3 ] || die "usage: sidecar.sh check <worktrees.app|.AppImage|.deb> <target-triple> <version>"
  local bundle="$1" triple="$2" want="$3" arch root work
  arch="$(arch_of "$triple")" || die "no desktop app bundle is built for '$triple'"
  case "$triple" in
    *-apple-darwin)
      [ -d "$bundle/Contents/MacOS" ] || die "no app bundle at $bundle"
      verify "$bundle/Contents/MacOS/$NAME" "Contents/MacOS/$NAME" \
        "$bundle/Contents/MacOS/app" macos "$arch" "$triple" "$want"
      # A linker-signed GUI executable is not a signed .app: without the
      # bundle's CodeResources seal, Gatekeeper can call a browser download
      # damaged. Check both the build output and the shipped updater tarball.
      codesign --verify --deep --strict --verbose=4 "$bundle" \
        || die "$bundle has an invalid macOS bundle signature"
      ;;
    *-linux-gnu)
      command -v readelf >/dev/null 2>&1 || die "readelf not found (binutils)"
      work="$(mktemp -d "${TMPDIR:-/tmp}/sidecar-check.XXXXXX")"
      # shellcheck disable=SC2064  # expand now: $work is local, the trap runs at exit
      trap "rm -rf '$work'" EXIT
      root="$work/root"
      unpack "$bundle" "$root"
      # What `files` is FOR: a .deb's usr/bin is the system PATH, and an
      # AppImage's is first on the app's own. externalBin would put it there.
      if [ -e "$root/usr/bin/$NAME" ] || [ -L "$root/usr/bin/$NAME" ]; then
        die "${bundle##*/} has usr/bin/$NAME — that puts the CLI on PATH; it belongs in usr/lib/worktrees/"
      fi
      verify "$root/usr/lib/worktrees/$NAME" "usr/lib/worktrees/$NAME in ${bundle##*/}" \
        "$root/usr/bin/worktrees-app" linux "$arch" "$triple" "$want"
      ;;
  esac
}

case "${1:-}" in
  stage)  shift; stage "$@" ;;
  check)  shift; check "$@" ;;
  unpack) shift; unpack "$@" ;;
  *) die "usage: sidecar.sh stage [<target-triple>] | check <worktrees.app|.AppImage|.deb> <target-triple> <version> | unpack <.AppImage|.deb> <dir>" ;;
esac
