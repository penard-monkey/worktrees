# Session: a Homebrew tap — designed, then declined

- **Date:** 2026-09-26 (research + design) → 2026-10-09 (declined, closed out)
- **Worktree:** `.worktrees/brew-tap`
- **Branch:** `brew-tap` (no commits); this archive lands from `brew-tap-close-out`, branched off `origin/main`
- **PRs:** this close-out only
- **Release:** none
- **Planning files:** `planning.tar.gz` (task_plan / findings / progress — Phase 1 of 4 complete)
- **Outcome:** nothing shipped. David decided against a tap ("not doing that") after the design was presented.

## What was asked

"Allow users to install the app with brew — we can create our own tap, right?"

## What was designed (Phase 1)

- A tap is a public repo named `homebrew-<x>`; the proposal was
  `penard-monkey/homebrew-worktrees` with `Formula/worktrees.rb` (CLI) and
  `Casks/worktrees-app.rb` (app). No Homebrew review is needed for a
  third-party tap, and a personal tap may ship prebuilt binaries.
- **Every artifact already exists** in each GitHub release: the four bare CLI
  binaries (`worktrees-<target>`), the two `worktrees-app-<arch>.app.tar.gz`
  bundles, and `checksums.txt`. A tap needs no new build output.
- Formula: per-OS/arch `url` + `sha256`, `depends_on "tmux"` (replacing
  install.sh's interactive "install tmux?" prompt), `--version` as the test.
- Cask: `auto_updates true` (the Tauri updater owns app upgrades; without it
  brew and the in-app updater fight over the `.app`), `depends_on formula:` the
  CLI (MCP setup needs it), and a quarantine-stripping `postflight` mirroring
  install.sh's `xattr -cr`, needed until Developer ID signing + notarization.
- Automation: a `homebrew` job after `release` in release.yml rendering both
  files from `checksums.txt` and pushing with a tap-scoped token, skipping
  prereleases.

## Decisions

- **Declined (2026-10-09, David).** Recorded in ROADMAP so it is not
  re-proposed.
- Had it gone ahead: tap separate from the main repo (brew's naming rule),
  formula `worktrees` + cask `worktrees-app` (distinct tokens avoid
  `--formula`/`--cask` ambiguity).

## Dead ends / gotchas worth keeping

- **The app's "Update CLI" would corrupt a brew-managed CLI.** `cli_binary()`
  (`app/src-tauri/src/lib.rs`) already probes `/opt/homebrew/bin/worktrees`
  and `/usr/local/bin/worktrees`, and `update_cli` runs install.sh with
  `WORKTREES_INSTALL_DIR` set to the resolved binary's directory. Under brew
  that path is a SYMLINK into the Cellar, so install.sh would replace it with
  a copy; brew would then think the old version is installed and
  `brew upgrade`/`uninstall` would act on a Cellar the PATH no longer uses.
  Any future packaging (brew or otherwise) must teach `update_cli` and
  install.sh to detect a package-managed binary (path canonicalises into
  `/Cellar/`) and defer to the package manager. This is latent today for
  anyone who hand-installs into `/opt/homebrew/bin`.
- Homebrew has been tightening unsigned casks (`--no-quarantine` deprecated);
  the postflight workaround was flagged as unverified against current brew.
  Signing + notarization (ROADMAP) is the real fix either way.

## Verification

- Nothing was created outside this worktree: `gh repo view
  penard-monkey/homebrew-worktrees` does not resolve, no `homebrew*` repo
  exists under the account, the only repo secret is
  `TAURI_SIGNING_PRIVATE_KEY`, and the `brew-tap` branch had zero commits and
  a clean diff (no release.yml edits).
- An untracked `AGENTS.md` in the tree (the branch predated it being tracked)
  was diffed against `origin/main`: an older Claude→Codex-substituted copy of
  the instructions with nothing main lacks. Not committed; moved to
  `~/.cache/worktrees/worktrees/brew-tap/AGENTS.md.stale-untracked`.

## Follow-ups

- None for the tap. The `update_cli` hazard above is recorded with the
  ROADMAP line in case packaging is ever revisited.
