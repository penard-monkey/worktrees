# Session: SSH host aliases in GitHub remotes

- **Date:** 2026-10-08
- **Worktree:** `.worktrees/ssh-host-alias`
- **Branch:** `ssh-host-alias` → PR [#465](https://github.com/penard-monkey/worktrees/pull/465), squash-merged as `80d9fe3`
- **Release:** v0.40.1
- **Planning files:** none; `planning.tar.gz` holds the lane's brief (`.planning/brief.md`)

## Why

casa-del-valle-monorepo's origin is `git@github.com-penard-monkey:penard-monkey/…`. That is a
multi-account `Host` alias in `~/.ssh/config` with `HostName github.com`. `github::split_remote`
took the alias as the host. The header link opened `https://github.com-penard-monkey/…`, and the
Pull requests view asked `gh auth token -h github.com-penard-monkey`, so the project read as not
GitHub.

## What shipped

All of it is in `crates/worktrees-core/src/github.rs`:

- `ssh_host` resolves an SSH-form remote's host with `ssh -G -- <host>`, which prints the
  effective config and connects nowhere. This covers scp-like remotes with any user or none
  (previously only `git@`), `ssh://`, `git+ssh://` and `ssh+git://`. https never consults ssh.
- `web_base`/`parse_remote` resolve by default; `_with(…, HostOf)` variants are the pure seam.
  `resolve_from` takes the resolver, and tests pass `literal_host` or a table.
- `valid_ssh_host` refuses a host before it becomes argv (leading alphanumeric, `[A-Za-z0-9._-]`,
  at most 253 characters). The `hostname` reply is validated the same way before it goes into a
  URL.
- `SSH_ALT_HOSTS`: `ssh.github.com`, `altssh.gitlab.com` and `altssh.bitbucket.org` map back to
  their web host. A `Host github.com → HostName ssh.github.com` firewall config would otherwise
  break plain github.com.
- Cache per (ssh binary, host): a success is kept for 10 min and a failure for 60 s
  (`ssh_host_in`, with an injected cache and clock). There is a 3 s deadline, and any failure
  falls back to the literal host.
- `WORKTREES_SSH_BIN` seam; the default is `ssh` from PATH, as git runs it.

## Decisions

- **`ssh -G`, not a parser of `~/.ssh/config`.** Include, Match, wildcards and `%` tokens make
  ssh the only faithful reader, and it is the binary git itself uses on push.
- **insteadOf needed no change.** `git remote -v` and `git remote get-url` both print URLs after
  `url.<base>.insteadOf` rewriting (verified in a scratch repo), so `ls-remote --get-url` would
  have added nothing. `resolve_sees_urls_after_instead_of` pins it against real git.
- **No frontend change.** `remote.ts` only receives the resolved https base, so there is no
  second parser to drift.
- **ADR 0001 holds.** The program is fixed (the user's `ssh`), and the repo only selects a
  validated host operand after `--`. The one repo-authored route to that operand is a submodule
  registered as a project (its `origin` comes from `.gitmodules`), and it is charset-limited.

## Dead ends / gotchas

- **Memoising a failure re-creates the bug (review must-fix).** The first cut cached
  `unwrap_or(literal)` forever. One cold or offline timeout, or an alias added while the app
  runs, pinned `github.com-penard-monkey` for the life of the process. A failure is now cached
  briefly, and a success expires too.
- **The insteadOf pin passed on the old code by design**, because the behaviour already held. It
  was shown red by mutating `resolve` to read raw `remote.*.url` config. The pre-argv validation
  was shown red separately: the fake ssh's run log changed when the check was dropped.
- `ssh -G` prints "Pseudo-terminal will not be allocated" on stderr when stdin is not a tty.
  This is harmless, because only stdout and the exit code are read.

## Verification

- 5 new unit tests plus the insteadOf pin, each shown red first (see the PR body).
- A read-only real check on this machine: the casa-del-valle remote resolves to
  `github.com/penard-monkey/casa-del-valle-monorepo`, with 2 lookups in about 57 ms.
- Gates after a fresh release build:
  - bats 480/480 and lint
  - core 738, cli 70 and app `--lib` 162 unit tests
  - `tsc` and `cargo check -p app`
  - `remote-check.mjs`
- `ls --json` is byte-identical to v0.40.0. CI was 9/9, and the fable re-review was clean.

## Follow-ups

These are in ROADMAP:

- `PrCache.entries` is keyed by project root, so two projects on the same repo fetch twice.
- An https Enterprise remote on a non-default port: the port is dropped from the host `gh` is
  asked about. Decide whether that is right.
- `core.sshCommand` / `GIT_SSH_COMMAND` with `-F <file>` is not followed; `ssh -G` reads the
  default config.

The review's cosmetic notes are dropped deliberately: the IPv6-literal `ssh://[::1]/…` split
(an IPv6 host fails validation and falls back to the literal host, which is harmless) and
expressing the deadline const as a `Duration`.
