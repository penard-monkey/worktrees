# close-out overrides — {project}

Project config for the `close-out` skill (`.claude/skills/close-out`, written
by worktrees). Only the deltas from that skill's defaults live here. This file
belongs to the repository: worktrees writes it once and never again, so edit
it freely.

| Setting | This repo |
| --- | --- |
| archive dir | `docs/sessions/<YYYY-MM-DD>-<slug>/` (default) |
| scratch dir | `~/.cache/worktrees/{project}/<place>/` (default) |
| roadmap | `ROADMAP.md` (default) |
| merge | squash (default) |
| idle branch | `<place>-next` (default) |

## Gates (step 1 — run before the archive PR)

TODO: list the commands that must pass before a PR, for example:

```sh
make test
make lint
```

## Notes

Add this repository's close-out gotchas here as you find them.
