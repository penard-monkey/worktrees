# Remove was broken for every place — `delBranch`, and a banner that outlived its cause

- **Date:** 2026-07-28 → 2026-08-02 (archived 2026-08-17)
- **Working tree:** `.worktrees/everything-settings`
- **Branches:** started on `scratch-next`; work landed from `fix/remove-place-delbranch`
- **PR:** [#60](https://github.com/penard-monkey/worktrees/pull/60) — squash-merged `efe4619`, 2026-08-02
- **Shipped in:** v0.7.0
- **Planning files:** none (single-thread bug fix; no `task_plan.md` was created)

Started from two screenshots the user dropped in `_tmp`, with "I am not able to
remove I get this error" and no error text pasted. Both are archived next to this
summary: `remove-error.png` (the red arg banner over the nav) and
`project-health.png` (the Project sheet showing 18 shadowed-link issues).

## What shipped

**`app/src/App.tsx`** — two call sites passed `del_branch`; the wire name is
`delBranch`. Tauri v2 renames Rust snake_case command params to camelCase across
the IPC boundary, so `remove_place(repo, slug, del_branch, force)` in
`app/src-tauri/src/lib.rs` is `delBranch` on the JS side. Every remove in the app
was rejected before any git work happened:

```
invalid args `delBranch` for command `remove_place`:
command remove_place missing required key delBranch
```

**`app/src/App.tsx`** — the `.worktrees.toml` init probe moved off a once-per-root
startup effect and onto the existing five-minute doctor sweep; the `suggestedFor`
ref went away with it.

**`app/src/mock/install.ts`** — the mock's `remove_place` now throws the same
error the real backend does when `delBranch` is absent.

**`.nvmrc`** (new) — `22.13.0`. **`Makefile`** — `install-app` checks the active
Node against it before building. (Since reused by `dev-app` on current main.)

**`CHANGELOG.md`** — `[Unreleased]` → Fixed ×2, Added ×1.

## Decisions

- **Fix the frontend, not the backend.** Renaming the Rust param to `delBranch`
  would have made the Rust read oddly and diverged from every other handler. The
  wire contract is Tauri's, and the rest of the app already honours it —
  `term_open` passes `onBytes` correctly.
- **Make the mock strict rather than just correct.** The mock read `args.force`
  but never the branch flag, so remove "succeeded" headlessly under either
  spelling. Fixing only the caller would have left the harness just as blind to
  the next drift. It now rejects a missing `delBranch` the way Tauri does.
- **Mock throws a bare string, not an `Error`.** A real `invoke` rejects with a
  string, and `fail()` renders it verbatim; an `Error` would prefix `Error: ` in
  the mock and nowhere else.
- **Probe rides the doctor sweep** instead of getting its own timer — same cost
  class, same cadence, same not-ok-root gating, one fewer moving part.
- **`.nvmrc` pinned to `22.13.0`, not `22`.** It is the lowest version clearing
  pnpm 11's floor, it was already installed, and it sits inside CI's
  `node-version: 22`. A bare `22` would resolve to whatever nvm had locally —
  which is exactly how the build broke.

## Dead ends / gotchas

- **A clean auto-merge is not a correct merge.** The rebase onto 0.6.0 reported
  no conflict and silently appended the `[Unreleased]` entries *inside* the
  already-shipped `[0.5.0]` section, because the release process had consumed
  `[Unreleased]` in the meantime. Git had no way to see it. Always read the
  CHANGELOG after rebasing across a release.
- **`suggestion_key` deliberately excludes `exists`** (`lib.rs`, `suggestion_key`
  hashes files/ports/compose/truncated only). It keys the *dismissal*, so writing
  a config must not re-suggest. Review suggested gating `setSuggest` on an
  unchanged `hash` to skip a no-op re-render; that would have reintroduced the
  banner bug exactly — a config appearing keeps the same hash while flipping
  `exists`, which is the one update the banner depends on. The code now carries a
  comment naming the trap, because the field name invites the optimization.
- **The stale-worktree downgrade.** This tree was one commit behind main and
  still on 0.3.2 while `/Applications` held 0.4.0. Building and installing from
  it would have silently downgraded the app and removed the right dock. Check
  `plutil -extract CFBundleShortVersionString raw` on both the bundle and the
  installed app before `ditto`.
- **`make install-app` cannot finish unattended.** It bundles fine, then dies
  signing the updater artifact: the key at `~/.tauri/worktrees-updater.key` is
  password-protected and there is no TTY for the prompt, so `make` aborts
  *before* the `ditto` step. Symptom is a successful build with nothing
  installed. Exporting `TAURI_SIGNING_PRIVATE_KEY` is not enough — it still needs
  the interactive password.
- **pnpm 11 requires Node >= 22.13** and says so only after cargo has finished
  building, which is what the `.nvmrc` + preflight now front-loads.
- **The Bash tool's cwd persists between calls.** An earlier `cd app` turned a
  later `make install-app` into "No rule to make target `install-app`" — a
  failure that reads like the Makefile is broken. Use `make -C <repo-root>`.
- **Piping a gate through `tail` buffers all of it.** `make test | tail -25`
  emits nothing until the whole suite finishes, so progress polling learns
  nothing; and the pipeline's exit status is `tail`'s, not the suite's.
- **CI `lint` went red on an infra flake** — `docker pull bash:3.2` hit a
  registry timeout (exit 125) on a gate that parses `bin/worktrees`, untouched by
  this PR. Re-ran the job; passed in 24s. Read the failing log before assuming
  the change did it.

## Verification

- Full gate suite green on the final diff: `cargo build --release -p
  worktrees-cli`, `make test` (238 bats), `make lint`, `cargo test -p
  worktrees-core` (137), `tsc --noEmit`, `cargo check -p app`.
- CI 9/9 across macOS and Ubuntu, `mergeStateStatus: CLEAN`.
- Both Makefile preflight messages tested against a no-`node` PATH and a
  22.12.0 PATH.
- Reviewed by a fable subagent before merge: no blockers, four nits — three
  applied, one rejected (the `hash` gate above).
- Built 0.6.0 with the fixes, installed to `/Applications`, confirmed `delBranch`
  present in the shipped binary and the app relaunches.

**Not verified:** neither fix was exercised at runtime. No place was actually
removed and no banner was watched retiring. The bats and core suites do not cover
this path — it is frontend IPC — so the evidence is "the right string is in the
binary", not "the button works". Worth 30 seconds next time the app is open.

## Follow-ups

- `make install-app` still cannot run unattended (updater signing password).
  Filed on the roadmap.
- Runtime check of the remove button and the banner retiring, per above.
