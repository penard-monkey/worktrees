---
title: "Session: undeclared-file drift + ADR 0001"
---

# Session: undeclared-file drift + ADR 0001

- **Date:** 2026-08-02 → 2026-08-05
- **Worktree:** worktreesinclude
- **Branches:** worktreesinclude (feature), close-out-undeclared-drift (this archive)
- **PRs:** [#77](https://github.com/penard-monkey/worktrees/pull/77) (squash-merged → `0a361c3`)
- **Release:** none — lands in `[Unreleased]`, next tag picks it up
- **Planning files:** planning.tar.gz alongside this summary (task_plan / findings / progress)

## Where this started

The question was not "build something". It was: *we sync env files and
uncommitted documents — any reason we didn't go with `.worktreeinclude` files?
Should we redesign file sharing across worktrees?*

Answer: **no redesign.** A `.gitignore`-style include list would be a strict
downgrade. The evaluation is the first half of this session; the build is the
second half, and it came out of a gap the evaluation surfaced.

### Why not `.worktreeinclude`

Not recorded as a rejected alternative in the proposal's §14 prior-art list, but
every design principle rules it out:

1. **No place for `mode`.** link-vs-copy is load-bearing. cdv's
   `apps/backoffice/.env.local` is rewritten at runtime by `deploy-local.sh`; a
   link would scribble on the main checkout AND every sibling worktree. A
   path-only line cannot express that.
2. **Globs cannot fail loud** (principle #5 — the bug being fixed is silence).
   An explicit `path = "…/google-services.json"` missing from main is a warning
   with an exit code. A glob matching nothing is indistinguishable from a glob
   that matched everything it should.
3. **Security surface.** Layer A (`projcfg::RelPath`, string rules at parse) +
   Layer B (resolved-path containment at apply, catches a committed symlink to
   `~/.ssh/id_rsa`). A glob expands against a *hostile clone's* tree — `**`
   re-opens §4 B3 through a set nobody wrote down.
4. **Two-thirds of the file isn't files.** `[ports]` + `[compose]` need
   structure regardless — which is exactly the "ours does docker spin-up too"
   intuition that prompted the question. `.worktreeinclude` + `.worktrees.toml`
   = two parsers, two discovery paths, one more way to disagree, zero new
   capability.
5. **Per-entry comments are the file's teaching job** (§3). The cdv config is
   mostly `# why this is a copy` prose citing `worktrees.sh` line numbers.

And the glob use case is **already served at authoring time**: `init.rs` walks,
classifies, filters to gitignored-and-untracked, and emits explicit `[[file]]`
stanzas — a human reviews the expansion once instead of it being re-resolved on
every `new` against whatever happens to be on disk.

## The gap that evaluation found

`.worktrees.toml` was effectively **write-once**. Three independent paths:

1. `ops.rs` — `hint_init` sits inside `match project_cfg { None => … }`. The
   passive nudge on `new` runs **only for repos with no config**. Write one and
   that arm is unreachable forever.
2. `ops.rs` — `cmd_init` hard-refuses (exit 1) when the config exists. `--force`
   re-renders from scratch, destroying every hand-written `# why this is a copy`
   comment, every hand-set `mode = "copy"`, every tuned `[ports] base`. For the
   cdv config that is ~80% of the file's value, so `--force` is not usable as a
   re-scan.
3. `materialize::probe` iterates **config entries only**. No code path anywhere
   asked "what gitignored file is on disk and NOT declared".

Concretely: add `apps/newsvc/.env` to main. `doctor` → clean, exit 0. `new` → no
hint. `init` → exit 1. Every existing and future worktree silently lacks it.

That is proposal §1.2 again, displaced from "no config" to "stale config" —
which is the steady state of any repo older than a month.

## What shipped

### `Code::Undeclared` (`crates/worktrees-core/src/diag.rs`)

Gitignored, untracked, named by no `[[file]]` entry.

- **Repo-scoped** — `place` stays `None`, so a project with fifteen worktrees
  says it once. `Finding.place` was already `Option`, so no new report slot.
- **Severity split** — `Kind::Credential` → Warn, `Kind::Env` → Info. Same split
  `hint_init` uses (§12): a missing `.env` breaks loudly on the next command; a
  missing `google-services.json` builds fine and dies on a device days later.
- **Exit 0 by default.** `Report::exit_code` returns 2 only on `Error`.
  `doctor --strict` promotes the **Warn half only**, alongside `CopyStale`.

### Detection (`crates/worktrees-core/src/init.rs`)

- `probe_files_bounded` — `probe_files` plus the walk's truncation flag.
- `undeclared_in` — PURE set difference, config rels vs candidates, folded
  through `RelPath::fold_key` on both sides.
- `undeclared` — the one impure wrapper.
- `render_undeclared` — appendable `[[file]]` fragment, no file header and no
  `[ports]`/`[compose]`/`[project]` (any of those would be a duplicate key in
  the config it is pasted into).
- `declarable` — `split_declarable` exposed for `cmd_init --diff`.

### Wiring (`crates/worktrees-core/src/ops.rs`)

- `undeclared_findings` — builds the findings; pushes its own doctor-phrased
  truncation Info when the walk hit a bound.
- Called in `cmd_doctor` only on whole-project runs (`names.is_empty()`) and
  only outside `--config-only`.
- `init_diff` — `worktrees init --diff`: fragment on stdout, prose on stderr,
  writes nothing, round-trip guarded through `projcfg::parse`. No config at all
  → falls back to `--print` with a note.

### Docs

- `docs/adr/0001-no-repo-supplied-argv.md` — proposal §12 asked for it by name.
- `DESIGN.md` — header banner + 9 superseded markers.
- `CLAUDE.md` — new `## Decisions` section pointing at `docs/adr/`.
- `docs/proposals/project-settings.md` — §12 "v4 — undeclared drift ✅ BUILT",
  and the "Never" line now links the ADR.
- `CHANGELOG.md` `[Unreleased]` ×2.

## Decisions

- **Keep `.worktrees.toml`, reject `.worktreeinclude`.** Five reasons above. The
  docker/ports half that prompted the question is precisely why the format is
  TOML and not a line-oriented include file.
- **No glob `path` inside `[[file]]`** (the monorepo-ergonomics ask). It costs
  per-entry `mode` and "declared but missing = warning". cdv's real config is
  6 stanzas — not painful. `init --diff` gives the ergonomics with neither cost.
- **Undeclared exits 0.** Promoting to `Error` by default would break every CI
  pinned on `doctor` exiting 0. `--strict` is the opt-in, matching `CopyStale`.
- **Not in `--config-only`.** That mode is config-vs-git on a bare clone, where
  every gitignored file is absent by definition — the check could only ever
  report nothing there, and running it would make the mode machine-dependent.
- **Not on `doctor <name>`.** A named place is a question about that place. Same
  rule the session scan already follows.
- **Case-fold both sides.** `Apps/API/.env` declared vs `apps/api/.env` on disk
  is one file on macOS. Reporting it would push the user into adding a case-only
  duplicate, which `check_file_list` then refuses — a finding whose only remedy
  is a config error. On case-sensitive Linux this suppresses a genuinely
  distinct path; accepted deliberately, and documented on both sides.
- **`init --diff` writes nothing.** Each entry may need `mode = "copy"` and
  nothing on disk knows which. `--force` is the only thing that rewrites the
  config, and it rewrites it wholesale.
- **ADR + DESIGN.md markers together.** An ADR alone would have been a third
  buried file (see gotchas).

## Dead ends / gotchas

- **The re-add vector for the never-list was live, not hypothetical.**
  `DESIGN.md` still described the *reversed* design as current — `[infra]
  up/stop/down` via `sh -c`, per-place `up_cmd`, `worktrees up|down` verbs,
  `infra_up/stop/down` Tauri commands, and a **first-run trust prompt** for
  "repo-authored strings" — and `CLAUDE.md` points readers at `DESIGN.md` as the
  app's design doc. Writing only the ADR would have left the contradiction in
  the more-read file. Nine sites now carry markers.
- **Shell `grep` silently returned nothing on some files** this whole session
  (`grep -c "" projcfg.rs` → exit 1 on a 1228-line file). `/usr/bin/grep` works.
  Cost ~4 tool calls of confusion before it was spotted. If a grep comes back
  empty against a file you know contains the string, that's the cause.
- **`test/lib/bats-*` submodules were uninitialized** in this worktree — `make
  test` died with `No such file or directory` before running anything.
  `git submodule update --init --recursive`.
- **`corepack pnpm` fails** with `Cannot find matching keyid` (signature
  verification against a stale key). Homebrew `pnpm` 11.1.1 under the `.nvmrc`
  node (22.13.0) works. Node 22.12 is below pnpm 11's floor, so the default
  `node` in this shell is too old — `nvm use` first.
- **`cargo fmt --check` is dirty repo-wide** on files this branch never touched
  (default rustfmt disagrees with the house 100-col style; there is no
  `rustfmt.toml`). Not a gate in CLAUDE.md; left alone.
- **PR sat long enough to conflict.** main shipped v0.8.0 (#78) meanwhile;
  `mergeable: CONFLICTING` on `CHANGELOG.md` (`[Unreleased]` vs the new
  `## [0.8.0]` section) and no CI had ever run. Rebased, kept both sections,
  force-pushed with `--with-lease` — CI only then fired. **A PR showing zero
  checks is worth suspecting as conflicted, not as "CI is slow".**

## Review (fable) — 6 findings, none severe

Reviewer independently re-ran the gates and read the proposal. Verdict: nothing
mis-reports a declared file as undeclared or vice versa.

1. Docs said `--strict` promotes Undeclared *unqualified*; it promotes only the
   Warn (credential) half. A team gating CI on `--strict` would never hear about
   a new `.env.local`. → both sites now say which half. Behavior unchanged.
2. Truncation finding reused `init`'s `TRUNCATED` string — *"add anything **it**
   missed by hand"*, whose "it" is the config `init` just printed, with no
   referent in a doctor report. It also read as a file finding, so a `--json`
   consumer counting `"code":"undeclared"` was off by one — including this
   session's own bats test, which passes only because tiny test repos never
   truncate. → own doctor-phrased message that says it is not about a file.
3. `init --diff` silently swallowed `--force`/`-y`/`--print`. `--diff --force`
   reads as "rewrite the config with the diff applied"; swallowing a destructive
   flag is the silent skip this codebase forbids. → usage error, exit 1, the
   same guard shape as `doctor --config-only <name>`, plus a bats test.
4. All-rejected `--diff` printed *"0 undeclared entries. Append to…"* —
   undercounting (rejected paths ARE undeclared) and pointing at an
   all-comments fragment. → aside emitted only when something is appendable.
5. DESIGN.md's banner claimed every affected section was marked; **5 sites
   inside otherwise-current sections were not** (the `up_cmd` field in DECLARED
   state, the "Rust shells to `worktrees up/down`" spine item, `up_cmd` in the
   status JSON example, the `infra.stop`/`infra.down` stop-vs-down item, and the
   lifecycle table's `down`/`down --keep-volumes` resource actions). → all
   marked.
6. **Accepted, not fixed.** With `WORKTREES_NO_PROJECT_CONFIG=1` and a config
   present, `init --diff` says "No .worktrees.toml in this repo" while plain
   `init` still refuses (it stats the file directly). That wording matches how
   `doctor` already describes the switch: ignored == not there.

Verified fine by the reviewer: the case-fold coupling (`to_lowercase` ≡
`fold_key`, with a test pinning it), zero-`[[file]]` configs, the audit switch
on doctor, the depth-1 `--print` recursion, fragment validity when all-rejected,
repo-scoped emission (structurally enforced), walk-order determinism, every ADR
claim against the actual code, and that no new test is vacuous.

## Verification

```
cargo build --release -p worktrees-cli   ok
make test                                248 ok, 0 not ok   (was 241)
make lint                                shellcheck + bash-3.2 gate clean
cargo test -p worktrees-core             143 passed
cd app && tsc --noEmit                   clean
cargo check -p app                       clean
cargo clippy -p worktrees-core -p worktrees-cli --all-targets
                                         5 warnings, ALL pre-existing
                                         (config.rs, project.rs, provision.rs,
                                          ops.rs:84, render.rs — untouched)
CI on #77                                9/9 SUCCESS
```

New coverage: 6 unit tests in `init.rs` (set difference, empty config, case
fold, fragment parses standalone and is appendable, rejected path commented
out), 3 bats in `doctor.bats` (reported after the config was written / said once
not per worktree / absent from `--config-only`), 4 in `init.bats` (fragment +
write-nothing + appended fragment survives `doctor --config-only`, flag guard,
no-config fallback, rejected path).

## Follow-ups

- **Gap B — glob `path` in `[[file]]`.** Evaluated and declined this session;
  recorded so it is not re-litigated from scratch. Reopen only for a repo with
  `packages/*/.env` at a scale where 1 stanza per package is genuinely painful,
  and even then prefer extending `init --diff`.
- **App surface for undeclared.** The app already renders the finding
  (`ProjectSheet.tsx` maps all findings, and `place: null` correctly marks no
  row), but there is no button for `init --diff` the way there is for
  Relink/Provision.
- **`doctor --strict` in CI is credential-only.** If a project wants undeclared
  `.env*` to fail too, that needs a knob that does not exist yet.
- **cdv migration** — still the open item from the original proposal (§12), not
  touched here.
