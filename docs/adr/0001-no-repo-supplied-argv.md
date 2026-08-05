# ADR 0001 — a cloned repo never supplies argv

- **Status:** accepted, permanent
- **Date:** 2026-08-02 (decided 2026-07-28 in `docs/proposals/project-settings.md` §5; recorded here because §12 asked for it)
- **Supersedes:** `DESIGN.md`'s "Infra convention" section — `[infra] up/stop/down`, `[place].up_cmd`, the `worktrees up|down` verbs, the `infra_up/infra_stop/infra_down` Tauri commands, and the first-run trust prompt. That section was marked "design locked (2026-07-20)"; nothing was ever built from it, so the cost of reversing it was zero.

## Decision

**`[hooks]` will not ship. `[infra] up/stop/down` will not ship. Nothing a cloned
repository contains may become argv, or name a program to run.**

This is not a "not yet". There is no threshold of demand that flips it. What
changes the answer is a change in the threat model, not a change in convenience —
see *Revisiting* below.

## Why

The argument for hooks is always the same: *we already execute `ai_cmd`, so this
is a small delta.* It proves the opposite, because provenance is the entire thing.

| Channel | Who writes the string |
|---|---|
| `ai_cmd` → `sh -ic '<ai_cmd>; exec $SHELL'` | flag, `$WORKTREES_AI_CMD`, or the user's own config — **a cloned repo cannot set it** |
| `install_cmd` → tmux pane 1 | one of **four literal constants**, selected by lockfile detection — the repo picks *which of my commands*, never *what* |
| `[hooks] post_create` | **the cloned repo's committed file, verbatim** |

The first two are "the repo selects among my commands." The third is "the repo
writes my command." That is a categorical change, and it is the whole boundary.

Three further reasons the usual escape hatches do not apply here:

- **direnv's trust-by-hash does not transfer.** direnv prompts when you `cd` into
  a repo you already work in. This tool's hot path is `worktrees new` **on a fresh
  clone** — the moment of least knowledge. The GUI would have to raise a security
  modal from a background poll, which is the textbook definition of a dialog
  people click through.
- **"CI defaults to untrusted" is a tell.** The original proposal's own fallback
  meant the feature would be off precisely where automation wants it. A security
  control that is disabled in the environment that most needs it is not a control.
- **The declarative sections already cover the real need.** The audit of cdv's
  761-line `worktrees.sh` found exactly *one* behavior not expressible as
  `[[file]]` / `[ports]` / `[compose]`: `docker compose -p <project> down -v` at
  removal. That became a first-class `[compose]` teardown — the tool already knows
  the project name and the file list. `pnpm install` was already covered by
  lockfile detection.

## What replaces it

A **user-scoped** hook. Identical execution power, opposite provenance:

```toml
# ~/.config/worktrees/config.toml — the USER's file, never .worktrees.toml
post_create = "./scripts/dev-setup.sh"
```

The repo still ships the script. Each developer opts in once per machine. No hash
file, no trust prompt, no GUI modal, no CI question.

Correspondingly, `[compose]` is **data**, and the tool assembles the argv:
`docker compose -f <validated files> -p <sanitized name> …`. The trust prompt
`DESIGN.md` specified is deleted rather than implemented — there is no repo-authored
string left for it to guard.

## The allow-list

The project rung of the precedence chain

```
flag > env (WORKTREES_*) > .worktrees.toml > ~/.config/worktrees/config.toml > default
```

exists **only for allow-listed keys**.

| A project MAY set | A project may NEVER set |
|---|---|
| `[[file]] path` / `mode` | `ai_cmd`, `ai_resume_arg` |
| `[ports] stride` / `max_slots` / `base` | `install_cmd*`, `no_install` |
| `[compose] files` (or the one-file `file`) | `post_create`, `[hooks]` |
| `[compose] project` — a template over a **closed** placeholder set (`{prefix}`, `{slug}`), pushed through `sanitize_prefix` before it reaches `docker -p`. Not a free string. | `[infra] up/stop/down` |
| `[project] prefix` — `sanitize_prefix` reduces any input to `[a-z0-9_-]`, and the project already sets this today via the committed `.worktree-prefix`. | |

A user-only key found in `.worktrees.toml` is a **hard parse error**, not a silent
ignore. Silent ignore trains people to write the key, and eventually someone
"fixes" the ignore.

## Enforcement — where this lives in code

The point of writing it down is that a re-add trips a **test**, not a reviewer's
memory. If you are here because one of these failed, this ADR is the reason.

| Enforcement | Where |
|---|---|
| `USER_ONLY_KEYS` — `ai_cmd`, `ai_resume_arg`, `post_create`, `hooks`, `infra`, `install_cmd`, `no_install` | `crates/worktrees-core/src/projcfg.rs` |
| `is_user_only` — prefix match on `install_cmd*`, so a future `install_cmd_pnpm` is refused on arrival rather than after someone ships it | `projcfg.rs` |
| `a_user_only_key_is_a_hard_error_that_points_at_the_user_config` — asserts `[hooks]` and `[infra]` are hard errors, at the top level and smuggled into any section | `projcfg.rs` tests |
| Every scope has a CLOSED known-key set: a key is expected, user-only (hard error), or unknown (warn + ignore) | `survey_keys` in `projcfg.rs` |
| `compose_down` — argv assembled from validated paths and a sanitized name; there is no command string from the repo to run | `crates/worktrees-core/src/provision.rs` |
| `WORKTREES_NO_PROJECT_CONFIG=1` — disables the project rung wholesale ("I am auditing an untrusted clone") | `projcfg.rs` |

## Consequences

- A project cannot express "run this after create" in a committed file. That is
  the accepted cost, and it is paid by the user-scoped `post_create`.
- Anything genuinely common enough to want a hook for is a candidate for a **new
  declarative section** where the tool assembles the argv — the route `[compose]`
  took. That is the path forward, not a string field.
- The tool stays safe to point at an untrusted clone, which is what makes
  `worktrees new` on a fresh clone an unremarkable act.

## Revisiting

Reopen this if — and only if — the threat model changes:

- the tool stops being run against clones the user has not read, or
- a sandbox exists that makes repo-authored execution equivalent in blast radius
  to reading a file.

"Several projects asked for it" is not one of these.
