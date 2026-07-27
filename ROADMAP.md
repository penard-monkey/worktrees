# Roadmap

Parking lot for work we've decided to keep but not do now. Each item links
the session summary that spawned it (see docs/sessions/). Groomed during the
close-out ritual (.claude/skills/close-out).

- **App signing + notarization** — bundles ship unsigned; install.sh strips
  quarantine on a checksum-verified install. The proper distribution tier is
  Developer ID signing + notarization so Gatekeeper passes without the
  workaround. (release.yml calls this "the later distribution tier".)
  _From: [2026-07-26 themes session](docs/sessions/2026-07-26-themes-v0.2.4/summary.md)_

- **Stale worktree/branch cleanup** — `New-icon` and `spike-demo` worktrees
  (and their branches) predate the current stream; decide merge/abandon and
  remove. `release-v0.2.3` worktree is superseded by v0.2.4.
  _From: [2026-07-26 themes session](docs/sessions/2026-07-26-themes-v0.2.4/summary.md)_
