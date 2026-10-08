# worktrees — build/install/lint/test/release
# The shipped CLI is the Rust binary (crates/worktrees-cli). `bin/worktrees` is a
# shim that runs the built binary from a clone; `make install` symlinks the binary
# itself onto your PATH. (The legacy bash engine was retired once the Rust binary
# reached full parity — see MIGRATION.md.)

BINDIR ?= $(HOME)/.local/bin
BATS   := ./test/lib/bats-core/bin/bats
RELEASE_BIN := $(CURDIR)/target/release/worktrees

.PHONY: build build-debug install install-copy install-app dev-app uninstall lint \
        test test-real-tmux test-mcp test-frontend check release

build:
	cargo build --release -p worktrees-cli

build-debug:
	cargo build -p worktrees-cli

install: build
	mkdir -p $(BINDIR)
	ln -sfn $(RELEASE_BIN) $(BINDIR)/worktrees
	@echo "installed: $(BINDIR)/worktrees -> $(RELEASE_BIN)"
	@case ":$$PATH:" in *:"$(BINDIR)":*) ;; *) echo "WARNING: $(BINDIR) is not on your PATH";; esac

install-copy: build
	mkdir -p $(BINDIR)
	install -m 0755 $(RELEASE_BIN) $(BINDIR)/worktrees
	@echo "installed (copy): $(BINDIR)/worktrees"

# Development loop for the desktop app: builds the app crate and serves the
# frontend on 1420 with hot reload. NOT an install — see install-app for that.
# `cd app && corepack pnpm`, never `pnpm --dir app`. Two separate reasons, and
# fixing only one leaves the target broken for somebody.
#
# CD, not --dir: a bare `pnpm` is a COREPACK SHIM, and corepack picks the
# version from the cwd's package.json BEFORE pnpm ever reads the flag. There is
# no package.json at the repo root, so it takes its own default, then pnpm sees
# app/package.json's `packageManager: pnpm@11.5.2` and refuses with
# ERR_PNPM_BAD_PM_VERSION. The flag names the project; only the CWD reaches
# corepack.
#
# COREPACK, not pnpm: `pnpm` is not guaranteed to exist at all. nvm installs it
# per NODE VERSION, and the version .nvmrc pins ships `corepack node npm npx`
# and nothing else — so `nvm use` (which this target tells you to run) can
# REMOVE pnpm from PATH and leave `/bin/sh: pnpm: command not found`. corepack
# is bundled with node, so it is always there, and it is what AGENTS.md already
# tells you to use. Verify this one in `env -i` with only the pinned node's bin
# on PATH: a login shell that has ever used another node version keeps that
# version's bin further down $PATH and finds pnpm anyway, which hides it.
#
# And tauri runs its OWN `beforeDevCommand`/`beforeBuildCommand` from
# tauri.conf.json, both of which say a bare `pnpm` — so fixing only this
# recipe gets you one line further and then the same "command not found" from
# inside tauri. They are overridden per target below rather than in
# tauri.conf.json, because release.yml builds through that file and CI, where
# pnpm IS on PATH, is correct as written.
#
# CI is immune to all of it: it installs the pinned pnpm directly
# (`package_json_file: app/package.json`), which is why these stayed broken
# locally while every CI run was green.
dev-app:
	@command -v node >/dev/null || { echo "node not found on PATH — run: nvm use"; exit 1; }
	@want=$$(cat $(CURDIR)/.nvmrc); have=$$(node -v | tr -d v); \
	  [ "$$(printf '%s\n%s\n' "$$want" "$$have" | sort -V | head -1)" = "$$want" ] || \
	  { echo "node $$have is older than .nvmrc ($$want) — run: nvm use"; exit 1; }
	cd app && corepack pnpm tauri dev \
	  --config '{"build":{"beforeDevCommand":"corepack pnpm dev"}}'

# Build the Tauri desktop app + install to /Applications (macOS; local builds
# aren't quarantined, so no signing needed). App updates = git pull + this.
install-app:
	@[ "$$(uname -s)" = Darwin ] || { echo "install-app is macOS-only"; exit 1; }
	@# Two different version errors can kill this AFTER you have waited for
	@# cargo, so both are checked up front. The node one is below; the pnpm one
	@# is structural and is why the recipe cd's into app/ (see dev-app above).
	@command -v node >/dev/null || { echo "node not found on PATH — run: nvm use"; exit 1; }
	@want=$$(cat $(CURDIR)/.nvmrc); have=$$(node -v | tr -d v); \
	  [ "$$(printf '%s\n%s\n' "$$want" "$$have" | sort -V | head -1)" = "$$want" ] || \
	  { echo "node $$have is older than .nvmrc ($$want) — run: nvm use"; exit 1; }
	@# Two overrides, because this target only ever ditto's the .app below while
	@# tauri.conf.json is written for RELEASES. Both failures land AFTER the whole
	@# cargo release build has succeeded, and make stops on them, so the install
	@# simply never happens — the build log reads like a success with an error
	@# stapled to the end.
	@#   --bundles app   : conf asks for `targets: "all"`; the dmg is a published
	@#                     release artifact and pure waste here.
	@#   createUpdaterArtifacts false : conf has an updater pubkey, so tauri signs
	@#                     the .tar.gz and ABORTS without TAURI_SIGNING_PRIVATE_KEY
	@#                     (the real blocker — it is a repo secret, see Release in
	@#                     AGENTS.md). A local install is never served by the
	@#                     updater, so there is nothing to sign.
	@# Do NOT "fix" either by narrowing tauri.conf.json: release.yml builds with
	@# neither flag and needs both behaviours.
	cd app && corepack pnpm tauri build --bundles app \
	  --config '{"bundle":{"createUpdaterArtifacts":false},"build":{"beforeBuildCommand":"corepack pnpm build"}}'
	rm -rf /Applications/worktrees.app
	ditto "$(CURDIR)/target/release/bundle/macos/worktrees.app" /Applications/worktrees.app
	@echo "installed: /Applications/worktrees.app ($$(plutil -extract CFBundleShortVersionString raw /Applications/worktrees.app/Contents/Info.plist))"

uninstall:
	rm -f $(BINDIR)/worktrees
	@echo "removed: $(BINDIR)/worktrees"

lint:
	shellcheck -x bin/worktrees install.sh test/helpers/*.bash
	bash -n bin/worktrees && bash -n install.sh
	@# bash-4-ism gate on the shim + installer (must run on stock bash 3.2)
	@if sed 's/[[:space:]]*#.*//' bin/worktrees install.sh | grep -nE 'mapfile|readarray|declare -A|\$$\{[A-Za-z_]+(,,|\^\^)'; then \
	  echo "bash-4-ism found (see above)"; exit 1; else echo "bash-3.2 gate: clean"; fi

# The gate = the Rust binary (bin/worktrees shim is common.bash's WT_BIN).
test: build-debug
	$(BATS) --filter-tags '!real-tmux' test/

test-real-tmux: build-debug
	$(BATS) --filter-tags real-tmux test/

# The MCP resource surface end to end: a SECOND thread writing to the same
# newline-delimited stdout as the request loop, and the silence owed before
# `notifications/initialized`. Neither is reachable from bats or from the unit
# tests — this drives the real binary over a pipe. Needs the release build,
# because that is the binary it drives.
test-mcp: build
	python3 scripts/mcp-resources-check.py

# The `app/scripts/*-check.mjs` family: each one slices REAL frontend source and
# evaluates it under stubs, guarding a rule that `tsc` and the unit tests cannot
# see (a CSS declaration, a branch order, a mirror of a core decision). There
# are a dozen of them and until now not one ran anywhere but by hand, so a
# regression they were written to catch would have been found by a person.
# Pure and fast — no browser, no harness, no network.
test-frontend:
	@for f in app/scripts/*-check.mjs; do \
	  printf '%-34s ' "$$(basename $$f)"; \
	  node "$$f" >/dev/null 2>&1 && echo ok || { echo FAIL; node "$$f"; exit 1; }; \
	done

check: lint test test-mcp test-frontend

# make release VERSION=x.y.z — bump the workspace version in Cargo.toml AND
# install.sh's SCRIPT_VERSION first.
release:
	@test -n "$(VERSION)" || { echo "usage: make release VERSION=x.y.z"; exit 1; }
	@grep -q '^version = "$(VERSION)"$$' Cargo.toml || { \
	  echo "workspace version in Cargo.toml != $(VERSION) — bump it first"; exit 1; }
	@grep -q '^SCRIPT_VERSION="v$(VERSION)"$$' install.sh || { \
	  echo "install.sh SCRIPT_VERSION != v$(VERSION) — bump it with Cargo.toml"; exit 1; }
	@git diff --quiet || { echo "working tree dirty"; exit 1; }
	git tag -a "v$(VERSION)" -m "worktrees v$(VERSION)"
	@echo "tagged v$(VERSION) — push with: git push origin main v$(VERSION)"
