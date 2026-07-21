# worktrees — build/install/lint/test/release
# The shipped CLI is the Rust binary (crates/worktrees-cli). `bin/worktrees` is a
# shim that runs the built binary from a clone; `make install` symlinks the binary
# itself onto your PATH. The legacy bash engine (bin/worktrees.bash) is kept as a
# parallel test gate until the migration completes (MIGRATION.md).

BINDIR ?= $(HOME)/.local/bin
BATS   := ./test/lib/bats-core/bin/bats
BASH_ENGINE := $(CURDIR)/bin/worktrees.bash
RELEASE_BIN := $(CURDIR)/target/release/worktrees

.PHONY: build build-debug install install-copy uninstall lint \
        test test-real-tmux test-bash test-real-tmux-bash check release

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

uninstall:
	rm -f $(BINDIR)/worktrees
	@echo "removed: $(BINDIR)/worktrees"

lint:
	shellcheck -x bin/worktrees bin/worktrees.bash install.sh test/helpers/*.bash
	bash -n bin/worktrees && bash -n bin/worktrees.bash && bash -n install.sh
	@# bash-4-ism gate on the shim + the legacy engine (both must run on 3.2)
	@if sed 's/[[:space:]]*#.*//' bin/worktrees bin/worktrees.bash | grep -nE 'mapfile|readarray|declare -A|\$$\{[A-Za-z_]+(,,|\^\^)'; then \
	  echo "bash-4-ism found (see above)"; exit 1; else echo "bash-3.2 gate: clean"; fi

# Default gate = the Rust binary (bin/worktrees shim is common.bash's WT_BIN).
test: build-debug
	$(BATS) --filter-tags '!real-tmux' test/

test-real-tmux: build-debug
	$(BATS) --filter-tags real-tmux test/

# Parallel legacy gate = the bash engine.
test-bash:
	WT_BIN=$(BASH_ENGINE) $(BATS) --filter-tags '!real-tmux' test/

test-real-tmux-bash:
	WT_BIN=$(BASH_ENGINE) $(BATS) --filter-tags real-tmux test/

check: lint test

# make release VERSION=x.y.z — bump the workspace version in Cargo.toml first.
release:
	@test -n "$(VERSION)" || { echo "usage: make release VERSION=x.y.z"; exit 1; }
	@grep -q '^version = "$(VERSION)"$$' Cargo.toml || { \
	  echo "workspace version in Cargo.toml != $(VERSION) — bump it first"; exit 1; }
	@git diff --quiet || { echo "working tree dirty"; exit 1; }
	git tag -a "v$(VERSION)" -m "worktrees v$(VERSION)"
	@echo "tagged v$(VERSION) — push with: git push origin main v$(VERSION)"
