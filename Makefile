# worktrees — build/install/lint/test/release
# The shipped CLI is the Rust binary (crates/worktrees-cli). `bin/worktrees` is a
# shim that runs the built binary from a clone; `make install` symlinks the binary
# itself onto your PATH. (The legacy bash engine was retired once the Rust binary
# reached full parity — see MIGRATION.md.)

BINDIR ?= $(HOME)/.local/bin
BATS   := ./test/lib/bats-core/bin/bats
RELEASE_BIN := $(CURDIR)/target/release/worktrees

# Local code-signing identity (macOS). Empty = the ad-hoc signature cargo/tauri
# leave behind, which is fine for Gatekeeper (local builds aren't quarantined)
# but not for PRIVACY prompts. TCC records an approval — "worktrees would like
# to access data from other apps" — against the binary's designated requirement,
# and for an ad-hoc signature that requirement IS the cdhash:
#   designated => cdhash H"09f58b26..."
# so every rebuild is a brand-new app to TCC and the prompt comes back (once per
# other-app data dir the build touches). A cert-backed identity — Apple
# Development, or a self-signed code-signing cert from Keychain Access — makes
# the requirement `identifier "net.casadelvalle.worktrees" and certificate …`,
# which survives rebuilds, so the approval sticks.
#   make install-app SIGN_ID="Apple Development: Ada L (TEAMID)"
#   export WORKTREES_SIGN_ID="…"   # same thing, once, from your shell profile
SIGN_ID ?= $(WORKTREES_SIGN_ID)
APP_ID  := net.casadelvalle.worktrees

# $(call sign,<path>) — no-op off macOS; a one-line note when SIGN_ID is unset.
define sign
	@if [ "$$(uname -s)" != Darwin ]; then :; \
	elif [ -n "$(SIGN_ID)" ]; then \
	  codesign --force --sign "$(SIGN_ID)" --identifier "$(APP_ID)" "$(1)" && echo "signed: $(1)"; \
	else \
	  echo "NOTE: $(1) is ad-hoc signed — macOS privacy approvals reset on every rebuild (set SIGN_ID to keep them)"; \
	fi
endef

.PHONY: build build-debug install install-copy install-app dev-app uninstall lint \
        test test-real-tmux check release

build:
	cargo build --release -p worktrees-cli

build-debug:
	cargo build -p worktrees-cli

install: build
	mkdir -p $(BINDIR)
	ln -sfn $(RELEASE_BIN) $(BINDIR)/worktrees
	@# the symlink target is what runs, so that is what gets signed — a later bare
	@# `make build` relinks it ad-hoc, so re-run `make install` after one.
	$(call sign,$(RELEASE_BIN))
	@echo "installed: $(BINDIR)/worktrees -> $(RELEASE_BIN)"
	@case ":$$PATH:" in *:"$(BINDIR)":*) ;; *) echo "WARNING: $(BINDIR) is not on your PATH";; esac

install-copy: build
	mkdir -p $(BINDIR)
	install -m 0755 $(RELEASE_BIN) $(BINDIR)/worktrees
	$(call sign,$(BINDIR)/worktrees)
	@echo "installed (copy): $(BINDIR)/worktrees"

# Development loop for the desktop app: builds the app crate and serves the
# frontend on 1420 with hot reload. NOT an install — see install-app for that.
dev-app:
	@command -v node >/dev/null || { echo "node not found on PATH — run: nvm use"; exit 1; }
	@want=$$(cat $(CURDIR)/.nvmrc); have=$$(node -v | tr -d v); \
	  [ "$$(printf '%s\n%s\n' "$$want" "$$have" | sort -V | head -1)" = "$$want" ] || \
	  { echo "node $$have is older than .nvmrc ($$want) — run: nvm use"; exit 1; }
	pnpm --dir app tauri dev

# Build the Tauri desktop app + install to /Applications (macOS; local builds
# aren't quarantined, so Gatekeeper needs no signing — SIGN_ID above is about
# privacy prompts, not Gatekeeper). App updates = git pull + this.
install-app:
	@[ "$$(uname -s)" = Darwin ] || { echo "install-app is macOS-only"; exit 1; }
	@# pnpm dies with its own version error AFTER you've waited for cargo; check
	@# the active node against .nvmrc first and say what to run instead. Same for
	@# a bogus SIGN_ID — find out now, not after a five-minute build.
	@command -v node >/dev/null || { echo "node not found on PATH — run: nvm use"; exit 1; }
	@want=$$(cat $(CURDIR)/.nvmrc); have=$$(node -v | tr -d v); \
	  [ "$$(printf '%s\n%s\n' "$$want" "$$have" | sort -V | head -1)" = "$$want" ] || \
	  { echo "node $$have is older than .nvmrc ($$want) — run: nvm use"; exit 1; }
	@if [ -n "$(SIGN_ID)" ] && ! security find-identity -v -p codesigning | grep -qF "$(SIGN_ID)"; then \
	  echo "SIGN_ID is not a code-signing identity in your keychain: $(SIGN_ID)"; \
	  security find-identity -v -p codesigning; exit 1; fi
	pnpm --dir app tauri build
	rm -rf /Applications/worktrees.app
	ditto "$(CURDIR)/target/release/bundle/macos/worktrees.app" /Applications/worktrees.app
	$(call sign,/Applications/worktrees.app)
	@echo "installed: /Applications/worktrees.app ($$(plutil -extract CFBundleShortVersionString raw /Applications/worktrees.app/Contents/Info.plist))"
	@# A running instance keeps the code identity it exec'd with, and so does any
	@# tmux server it started — privacy approvals granted to the new build won't
	@# reach either until both restart.
	@if pgrep -f "worktrees.app/Contents/MacOS" >/dev/null 2>&1; then \
	  echo "NOTE: worktrees.app is running — quit + reopen it to run this build."; fi

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

check: lint test

# make release VERSION=x.y.z — bump the workspace version in Cargo.toml first.
release:
	@test -n "$(VERSION)" || { echo "usage: make release VERSION=x.y.z"; exit 1; }
	@grep -q '^version = "$(VERSION)"$$' Cargo.toml || { \
	  echo "workspace version in Cargo.toml != $(VERSION) — bump it first"; exit 1; }
	@git diff --quiet || { echo "working tree dirty"; exit 1; }
	git tag -a "v$(VERSION)" -m "worktrees v$(VERSION)"
	@echo "tagged v$(VERSION) — push with: git push origin main v$(VERSION)"
