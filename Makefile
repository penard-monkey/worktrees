# worktrees — install/lint/test/release
# `make install` symlinks (the clone is the dev loop: `git pull` upgrades in place).
# `make install-copy` copies instead (bin dir on another volume, etc.).

BINDIR ?= $(HOME)/.local/bin
BATS   := ./test/lib/bats-core/bin/bats
SCRIPT := bin/worktrees
RUST_SHIM := $(CURDIR)/bin/worktrees-rs

.PHONY: install install-copy uninstall lint test test-real-tmux test-rust check release

install:
	mkdir -p $(BINDIR)
	ln -sfn $(CURDIR)/$(SCRIPT) $(BINDIR)/worktrees
	@echo "installed: $(BINDIR)/worktrees -> $(CURDIR)/$(SCRIPT)"
	@case ":$$PATH:" in *:"$(BINDIR)":*) ;; *) echo "WARNING: $(BINDIR) is not on your PATH";; esac

install-copy:
	mkdir -p $(BINDIR)
	install -m 0755 $(SCRIPT) $(BINDIR)/worktrees
	@echo "installed (copy): $(BINDIR)/worktrees"

uninstall:
	rm -f $(BINDIR)/worktrees
	@echo "removed: $(BINDIR)/worktrees"

lint:
	shellcheck -x $(SCRIPT) bin/worktrees-rs install.sh test/helpers/*.bash
	bash -n $(SCRIPT) && bash -n bin/worktrees-rs && bash -n install.sh
	@# bash-4-ism gate: strip comments first, then hunt builtins/syntax bash 3.2 lacks
	@if sed 's/[[:space:]]*#.*//' $(SCRIPT) | grep -nE 'mapfile|readarray|declare -A|\$$\{[A-Za-z_]+(,,|\^\^)'; then \
	  echo "bash-4-ism found (see above)"; exit 1; else echo "bash-3.2 gate: clean"; fi

test:
	$(BATS) --filter-tags '!real-tmux' test/

test-real-tmux:
	$(BATS) --filter-tags real-tmux test/

# Conformance gate: run the bats suite against the compiled Rust binary via the
# shim. The full unit suite (Increment 2 ports the write ops); real-tmux smokes
# via test-rust-real-tmux.
test-rust:
	cargo build -p worktrees-cli
	WT_BIN=$(RUST_SHIM) $(BATS) --filter-tags '!real-tmux' test/

test-rust-real-tmux:
	cargo build -p worktrees-cli
	WT_BIN=$(RUST_SHIM) $(BATS) --filter-tags real-tmux test/

check: lint test

# make release VERSION=x.y.z — bump must already be committed; gate tag == embedded version.
release:
	@test -n "$(VERSION)" || { echo "usage: make release VERSION=x.y.z"; exit 1; }
	@grep -q '^WORKTREES_VERSION="$(VERSION)"$$' $(SCRIPT) || { \
	  echo "WORKTREES_VERSION in $(SCRIPT) != $(VERSION) — bump it first"; exit 1; }
	@git diff --quiet || { echo "working tree dirty"; exit 1; }
	git tag -a "v$(VERSION)" -m "worktrees v$(VERSION)"
	@echo "tagged v$(VERSION) — push with: git push origin main v$(VERSION)"
