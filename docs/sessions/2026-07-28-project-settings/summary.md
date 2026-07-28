# Session — per-project settings (`.worktrees.toml`)

- **Date:** 2026-07-27 → 2026-07-28
- **Worktree:** `everything-settings`
- **Branches:** `next-stream` (feature), `release/0.3.2`, `chore/close-out-project-settings`
- **PRs:** [#57](https://github.com/penard-monkey/worktrees/pull/57) (v0.3.2 release), [#58](https://github.com/penard-monkey/worktrees/pull/58) (the feature, squash-merged as `eb32f7f`)
- **Release tag:** `v0.3.2` — published, signed bundles + `latest.json`
- **Planning files:** `planning.tar.gz` next to this summary
- **Spec:** `docs/proposals/project-settings.md` (rewritten this session)
- **Migration artifacts:** `~/.cache/worktrees/worktrees/everything-settings/dryrun/`

---

## How it started

A proposal doc turned up at `docs/proposals/project-settings.md`, written in a
different session from the **consumer** side — the Casa del Valle monorepo had
hit the gap and someone wrote down what was missing. The ask was a discussion,
not a build: "how could we make that work?"

The underlying problem: cdv ran **two** worktree tools that both create
worktrees — this app for places/tmux/nav, and its own `scripts/worktrees.sh` for
stack setup. Not a missing feature. A fork.

Four parallel audits (cdv tooling, worktrees-core seams, app surface,
format/security) corrected **six factual claims** in that proposal and found
something it had understated — see Dead ends.

---

## What shipped

`eb32f7f` on `main`. 17 commits squashed. **235 bats, 135 unit tests.**

### Core — `crates/worktrees-core/src/`

| File | What |
|---|---|
| `projcfg.rs` (new) | `.worktrees.toml` parsing. `RelPath` with a private field and `parse` as its only constructor; the file list deserializes *through* it, so a `ProjectConfig` holding an unvalidated path is unconstructible. Layer A string rules, the port-collision lint, the user-only-key guard. |
| `diag.rs` (new) | `Finding`/`Severity`/`Report` on a channel separate from `WtError`. Exit codes 0 clean / 1 usage-guard / 2 findings / 3 not-found / 4 needs-confirm. |
| `materialize.rs` (new) | `probe` (only FS read) → `plan` (pure) → `apply` (only FS write). Layer B containment, the destination decision table, the copy drift policy. |
| `provision.rs` (new) | Slot allocation under a PID-liveness `mkdir` lock, bind-probing instead of `lsof`, `.worktree.env` emission. |
| `init.rs` (new) | Detection over a bounded walk, and a hand-templated config emitter. |
| `ops.rs` | `cmd_relink`, `cmd_doctor`, `cmd_provision`, `cmd_init`, compose teardown, `live_session` adoption. |
| `config.rs` | `~/.config/worktrees/config.toml`, kv kept as permanent fallback. Full prefix precedence chain. |
| `ui.rs` | `CaptureUi` records severity; `Ui::can_confirm()`. |
| `project.rs` | `ensure_excluded` gains `.worktree.env`; prefix reads the project config. |

### App — `app/`

`src/ProjectSheet.tsx` (new), plus six commands in `src-tauri/src/lib.rs`
(`project_config_read`, `doctor`, `relink`, `provision`, `init_suggest`,
`init_write`), the drift glyph in `App.tsx`'s `glyphs()`, the init banner, and
matching mock cases in `src/mock/install.ts`.

### Tests

`test/relink.bats`, `test/doctor.bats`, `test/provision.bats`, `test/init.bats`
(new), plus additions to `test/close.bats` and `test/misc.bats`.

---

## Decisions (and why)

1. **Generic first, cdv second.** Each section stands alone — `[[file]]` with no
   `[ports]`, `[ports]` with no docker. cdv is the first consumer, not the
   specification. *Why: a schema shaped around one monorepo would have been
   unusable anywhere else, and the tool is not cdv's.*
2. **No repo-supplied argv, ever.** `[hooks]` does not ship, and `DESIGN.md`'s
   `[infra] up/stop/down` is **reversed** — the project declares `[compose]` data
   and the tool assembles the docker argv itself. *Why: today nothing a cloned
   repo contains can cause execution. `ai_cmd` comes from the user; `install_cmd`
   is one of four literal constants. `[hooks]` would be the first channel where
   the repo writes the argv — a category change, not a delta. The escape hatch is
   a user-scoped `post_create` in the user's own config: identical power, user
   provenance, no trust prompt to click through.*
3. **Ports in v1, not v2.** *Why: see Dead ends — a half-provisioned worktree is
   actively destructive, not merely incomplete.*
4. **The port slot is derived from `.worktree.env`, never stored in
   `.worktrees.places.json`.** *Why: that store is sticky user intent which
   outlives the worktree, and `remove_one` never writes to it — a stored slot
   would leak on every `rm` and make `max_slots` a silent exhaustion bug months
   later. Derivation has no release step at all.*
5. **`doctor` ships in v1, not v4.** *Why: creation is deliberately
   non-transactional, so materialization is a fourth thing that can half-succeed
   with nothing recording it. A feature whose own failure mode is silent does not
   fix a silent-failure bug.*
6. **A file where a link belongs is reported, never overwritten.** A deliberate
   divergence from the script being replaced. *Why: `ln -sfn` destroys it — exit
   0, no output, no backup.*
7. **TOML for both human-authored files**, kv kept as a permanent silent
   fallback. *Why: extending the kv parser meant inventing sections, lists and
   nested tables in a last-match-wins line scanner whose failure mode is silent
   omission — reproducing the motivating bug inside the fix. `toml` was already
   in `Cargo.lock` via tauri.*
8. **The app gets a sheet, not a pane; a glyph, not a dot.** *Why: `sel` is
   `{repo, slug}` and the main pane is binary, so a real pane meant reworking the
   selection model across ~30 sites for a read-only view. And the amber status
   dot already means "Claude needs input" — a second amber dot beside it is
   indistinguishable from the app's highest-value signal.*
9. **`doctor` never runs on the 3s poll.** *Why: `places:changed` already
   triggers a full `list_workspace` — up to 16 concurrent git calls per project.
   `DESIGN.md:280` had already made this ruling for the analogous case.*
10. **Two dismissal stores, accepted deliberately** — CLI under
    `$XDG_STATE_HOME`, app in `ui-state.json`. *Why: they gate different
    surfaces and share the same content-hash re-suggest rule; unifying them means
    the CLI reading an app-owned file.*

---

## Dead ends / gotchas

**The proposal was wrong about six things**, all verified against source:
`relink` was not on cdv `main` (only a branch); the port map had **7** entries,
not 5 (missing `WEBSITE` and `META_MOCK` — ship the doc's map and those two
collide with main on first run); slot occupancy was the union of a declared scan
**and** an `lsof` probe; the file lists were a hardcoded bash array, not config.
**Transcribe from the consumer's source, never from a design doc.**

**Ports were not deferrable, and the reason was worse than "incomplete."**
cdv's `deploy-local.sh` branches on `WORKTREE_MODE`, which is just "does
`.worktree.env` exist". The false branch runs a global `pkill -9` over
`next-server`/`tsx watch`/`next dev`/`wo-mock-server` and force-frees :3000. Two
worktrees created by *this* tool were in exactly that state. A half-provisioned
worktree kills the main checkout's stack.

**`ln -sfn` silently destroys a real file at the destination** — verified
empirically, and it had already fired: on 2026-07-27 at 21:33 the old
`relink --all` replaced `general-fixes`' real `google-services.json` with a
symlink, 31 minutes after main's copy appeared. No `.bak` anywhere. A faithful
port would have kept doing this.

**Every review round found something real.** The workflow was: opus implements
from a detailed brief and runs the gates → fable reviews the commit → fixes land
as a follow-up. Not one slice came back clean.

| Found | Root cause |
|---|---|
| `--force` destroyed its own backup | Fixed `.bak` name + `rename(2)` clobbers. Survived exactly one force — and the backup exists precisely because the displaced content may be the only copy. |
| Two worktrees could get the same port slot | `create_dir` then a *separate* pid write whose error was discarded. A holder that stalled between the two syscalls was evicted, then returned `Ok` on a lock it no longer held. |
| `close` killed adopted sessions unasked | Then the fix broke the GUI (`CaptureUi::confirm` always declines, so Close silently did nothing). Then the confirmation turned out **not bound** to the session it named — the ctx-menu arm had no auto-disarm, so a different session could take the place and get killed. Three rounds. |
| `init` hard-failed on legal repos | Candidates never went through `RelPath::parse`, so a gitignored `apps$1/.env` produced a config the tool's own parser rejected — "please report this", exit 1. |
| Credentials under accented dirs were silently missed | git C-quotes those paths under default `core.quotePath`; the filter compared raw strings. **Exactly the failure class the feature exists to catch.** |
| A failed `doctor` read as "zero problems" | It cleared every drift glyph and the sheet said "clean". The most-broken state decorated nothing. |
| The mock lied | Its `relink` cleared `copy-stale` findings the real command cannot clear without `--force`. |
| Failed commands showed nothing | `runCmd` set the error banner, then awaited `refresh()`, which began by clearing it. Pre-existing, and ironic for a feature about making silence loud. |

**A test was executing real `docker compose down -v`.** The harness *prepends*
shims to `$PATH`, so `rm -f "$SHIMS/docker"` resolved the real binary. It passed
either way while testing nothing. The harness already had the fix for tmux
(`install_no_tmux_path`); it just hadn't been applied to docker.

**The local `main` checkout was 23 commits stale** at release time. Cutting
v0.3.2 from it would have released the wrong tree. Always `git fetch` and cut
from `origin/main`.

**A rebase silently folded this branch's CHANGELOG entry into the shipped
`[0.3.2]` section.** Caught by reading the file; `git` reported no conflict.

---

## Verification

- Gates at merge: **235 bats** (134 pre-existing unchanged — a repo with no
  `.worktrees.toml` behaves identically), **135 core unit tests**, lint clean,
  `tsc --noEmit` clean, `cargo check -p app` clean. All 9 CI checks green on
  both PRs.
- App surface driven in a browser against the mock harness, not just
  typechecked — sheet, health badge, drift glyphs, relink clearing the badge,
  force re-seed, banner dismissal surviving a reload, the armed close naming its
  session, and the error banner surviving a background `places:changed`.
  ~40 screenshots in `~/.cache/worktrees/worktrees/everything-settings/`.
- `--force` backup rotation, the compose two-file argv, and the adopted-close
  prompt were verified against the **compiled binary**, not by reading.
- v0.3.2 release build green; all 10 assets published.
- cdv dry-run was READ-ONLY and the repo was verified byte-unmodified before and
  after (`git status`, md5 of `.git/info/exclude` and `.worktrees.places.json`,
  `.worktree.env` count).

---

## Follow-ups

**The cdv migration itself — not started, deliberately.** Runbook and
transcribed config in `~/.cache/worktrees/worktrees/everything-settings/dryrun/`.
Blocked on verifying `apps/mobile/google-services.json` (sender id
`86759926600`) against the Firebase console. See ROADMAP.

Smaller, all in ROADMAP:

- `app/src/mock/install.ts`'s `SUGGESTED_TOML` is a hand-written fixture, not a
  mirror of `render()`, so the harness preview lacks init's new warnings.
- A refresh-raised error is retracted by identical-string match; two sources
  producing byte-identical text would cross-clear.
- The 4s arm auto-disarm is tight for "Kill &lt;session&gt; — whole session?".
- `doctor`'s session-drift scan skips `(main)` on named-place runs.
- Spec §4 still says "the same rules apply to `[compose] file`" — true per
  entry, but the key is `files` now.
