---
title: "Session: empty-project onboarding (git init + first commit)"
---

# Session: empty-project onboarding (git init + first commit)

- **Date:** 2026-08-02 → 2026-08-06
- **Worktree:** bug-fixes
- **Branches:** bug-fixes-empty-project-onboarding (feature),
  close-out-empty-project-onboarding (this archive)
- **PRs:** [#80](https://github.com/penard-monkey/worktrees/pull/80)
  (squash-merged → `624ccc4`)
- **Release:** none — lands in `[Unreleased]`, next tag picks it up
- **Planning files:** planning.tar.gz alongside this summary (task_plan /
  findings / progress)

## Where this started

A screenshot, and a question with a wrong guess baked in. A brand-new project
(`~/workspace/headshots`: an empty directory) was added to the app; the app
refused it for not being a git repo; a hand-run `git init` got it added; then
**New worktree** failed:

```
Creating new branch 'project-config' off 'main'.
fatal: not a valid object name: 'main'
Failed to create branch 'project-config' off 'main' at …/.worktrees/project-config.
```

The question was *"is that because we don't have a remote, or because we don't
have an initial commit?"*

**Neither guess needed guessing** — it reproduces in four lines:

```
$ git symbolic-ref HEAD          # refs/heads/main   ← the ref exists
$ git rev-parse --verify HEAD    # fatal: Needed a single revision
$ git worktree add -b project-config /tmp/wt-test main
fatal: not a valid object name: 'main'
```

**Unborn HEAD.** `git init` writes `HEAD → refs/heads/main` before any commit
exists, so `main` is a name pointing at nothing. `git worktree add` needs a
commit-ish start point, and every candidate (`main`, `HEAD`, `origin/main`) is
unresolvable. The missing remote is *not* the cause: `ops.rs` only prefers
`origin/<base>` when that ref exists and falls back to the local base branch
otherwise — a repo with commits and no remote works fine.

So the app had **two** dead ends stacked, and fixing only the first one just
moves the wall a click later.

## What shipped

### 1. Core refuses before the side effects — `crates/worktrees-core/`

- `git.rs` — `has_commits()`: `rev-parse --verify --quiet HEAD`.
- `ops.rs` (`cmd_new`, right after the branch name is validated) — if HEAD is
  unborn, error out with the cause and the exact remedy:

  ```
  ✗ This repo has no commits yet — git cannot create a worktree off an unborn branch.
  ✗ Make the first commit, then retry:  git -C <root> commit --allow-empty -m "Initial commit"
  ```

  Placed **before** `ensure_excluded` / config load / any mutation, so a refusal
  leaves nothing half-made. The CLI gets the same guard for free.

### 2. Backend commands — `app/src-tauri/src/lib.rs`

- `probe_dir(dir) -> {exists, is_git, has_commits}` — the Add-project path has to
  tell "not a repo" (offer `git init`) from "repo with no commits" (offer a first
  commit) apart, and `add_project`'s flat `Err(String)` cannot carry that.
- `init_repo(dir)` — `git init` + `git commit --allow-empty -m "Initial commit"`,
  then the normal add.
- `create_initial_commit(repo)` — the bootstrap commit for an already-tracked
  repo.
- `snapshot()` now stamps `unborn: bool` on the per-repo JSON.

### 3. App — `app/src/App.tsx`, `App.css`

- `addProject` probes **before** adding. A bare folder renders `InitRepoPrompt`
  in the nav: *"`<dir>` isn't a git repository. Initialize it with an empty first
  commit?"* → `git init + first commit`.
- A tracked repo with `unborn: true` replaces `NewPlaceForm` with `UnbornPrompt`
  → **Create initial commit**. After it lands the real form takes over.
- Both are module-scope components with their own busy flag (App-scoped
  components remount every render — the standing rule in CLAUDE.md).

### 4. Harness + test

- `app/src/mock/install.ts` — `?empty` / `?unborn` query knobs (same idiom as
  `?notmux`); the picked path carries its kind (`empty-N` / `unborn-N`) so a dir
  stays what it was picked as, and `mockInited` promotes it once bootstrapped.
  `probe_dir` / `init_repo` / `create_initial_commit` mocked.
- `test/new.bats` — *"new: repo with no commits → refused with the first-commit
  remedy, nothing created"*: asserts the remedy text, asserts git's raw
  `not a valid object name` never reaches the user, asserts no worktree dir is
  left, then commits `--allow-empty` and asserts the same command now succeeds.

## Decisions

- **`git init` implies the empty first commit.** Init-only was offered and
  rejected: it leaves HEAD unborn, so the very next click (New worktree) hits the
  second wall. `--allow-empty` touches no user file, which is what makes bundling
  them safe.
- **The guard lives in core, not in the app.** Same failure reaches the CLI, and
  a bats test can hold it. The app's affordance sits *on top* of the guard, not
  instead of it.
- **`probe_dir` as a separate command** rather than pattern-matching
  `add_project`'s error string. Error strings are prose; this is a state machine
  with three states, two of which have a fix button.
- **`unborn` rides on the snapshot** (one flag on data already fetched) rather
  than a per-project probe command the nav would have to call on render.

## Dead ends / gotchas

- **The user's own hypothesis ("no remote?") was the wrong fork**, and the
  four-line repro settled it faster than reading the create path would have.
  Reproduce first; `git rev-parse --verify HEAD` is the whole diagnosis.
- **CHANGELOG rebase, resolved wrong the first time.** `main` shipped v0.8.0
  mid-session, so `[Unreleased]` conflicted. The conflict region spanned *more
  than the Unreleased block* — a naive marker-splice dropped the new entries into
  the **0.7.0** section, silently, and re-introduced two Fixed bullets that 0.7.0
  already carried. Fix: throw the merge away, take `origin/main:CHANGELOG.md`
  verbatim, re-insert only the session's own entries under `[Unreleased]`. Read
  the resolved section afterwards — the markers being gone proves nothing.
- **Stale `node_modules` after a rebase onto a moved main.** `tsc --noEmit`
  failed on `Cannot find module 'marked'` — a dependency main added in 0.8.0.
  `pnpm install` then failed because the shell's node was v22.12.0 and `.nvmrc`
  pins 22.13.0; `nvm use` printed "Now using v22.13.0" while `node -v` still said
  22.12.0 (an earlier node on PATH wins in a non-interactive shell). Prepending
  `~/.nvm/versions/node/v22.13.0/bin` to PATH was the working form.
- **`gh pr merge --delete-branch` reports a scary failure that isn't one.**
  `failed to run git: fatal: 'main' is already checked out at …` — that is gh's
  *local* post-merge checkout step, in a worktree layout where main lives
  elsewhere. The merge itself had already landed; check `gh pr view --json state`
  before reacting.
- **Merged with CI never reporting** (GitHub incident, checks stuck pending).
  Deliberate call by the repo owner, on the strength of the local gates. See
  Follow-ups.

## Verification

- `make test` — **249 ok**, 0 failures (post-rebase, against the new main),
  including the new `new.bats` case.
- `make lint` (shellcheck + bash-3.2 gate), `cargo test -p worktrees-core`,
  `cargo check -p app`, `tsc --noEmit` — all clean.
- Both UI flows driven headlessly (Playwright against `VITE_MOCK=1` vite on
  :5199, port 1420 left for `tauri dev`): `?empty` → prompt → project added;
  `?unborn` → **Create initial commit** → form swaps to New worktree. Zero
  console errors. Screenshot: `git-init-prompt.png` beside this summary.
- The real repro repo (`~/workspace/headshots`) now refuses cleanly with the
  remedy. It was deliberately left uncommitted — the guard is read-only, and
  making a commit in the user's repo was not this session's business.

## Follow-ups

- **Re-run CI on `main` once GitHub recovers.** #80 and everything else merged
  since the last release landed with checks pending; the Linux leg in particular
  has never run on this change (all local verification is macOS).
- The app never offers to *add* an already-tracked-but-unborn repo's first commit
  from anywhere except the new-worktree form. If that state turns out to be
  common, the project row is the more discoverable home for it.
