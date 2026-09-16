# Session: the repo on its remote, from the project menu and the topbar

- **Date:** 2026-09-16
- **Worktree:** `ui-tweaks` (idle base `ui-next`)
- **Branch:** `ui-remote-links` off a freshly fetched `origin/main` (84b89fa), deleted after merge
- **PR:** [#208](https://github.com/penard-monkey/worktrees/pull/208) → `21bcff0` (squash)
- **Release:** none — ships with the next one (CHANGELOG `[Unreleased]`)
- **Planning files:** `planning.tar.gz` beside this file
- **Scratch:** `~/.cache/worktrees/worktrees/ui-tweaks/{topbar-remote-link,project-menu-remote}.png`

## Context

David: "if we have a remote repo we should be able to go to it by right
clicking the project in the left nav. Also next to the right of the list of
branches at the top nav we should have a link to the repo so we can go to it
quickly and I don't have to go hunt on github." The place menu already had
"Open on GitHub"; the project menu and the topbar had nothing.

## What shipped

- **`app/src/remote.ts`** (new, pure): `remoteWebUrl(base, place)` — the ONE
  place that decides which page opens. github.com + `Place.upstream` of the
  form `origin/x` → `<base>/tree/x` (path segments URI-encoded); anything else
  → the repo home. Plus `remoteHostLabel` ("GitHub", or the bare host) and
  `remoteTitle` (URL minus scheme, for tooltips).
- **`app/scripts/remote-check.mjs`** (new): the guard for that rule, in the
  `dnd-check.mjs` family (esbuild-transforms the real source, imports it as a
  `data:` module). Run by hand: `node app/scripts/remote-check.mjs`.
- **`app/src-tauri/src/lib.rs`**: `github_url(repo, slug)` → `remote_url(repo)`,
  which only normalises `git remote get-url origin` to an https base. New unit
  test `a_remote_spec_becomes_one_https_base_or_none` for `normalize_remote`
  (it had none).
- **`app/src/App.tsx`**: `remoteBase` state (per project root, `null` = read
  and absent, missing = unread), `loadRemote` (re-read on selection change of
  PROJECT and on the project menu opening), `openRemote(root, place | null)`
  replacing `openOnRemote`. Project menu gains "Open on GitHub" (`proj-remote`)
  in the Copy path / Reveal group, gated on `pv.ok`. Topbar gains the link
  (`topbar-remote`, `Icons.ExternalLink`) after the branch chip, rendered only
  when the base is known.
- **`app/src/App.css`**: `.topbar .remote-link` — `flex: none` so it never
  yields in the identity's shrink chain, `line-height: 0` so the SVG sits on
  the chip's midline.
- **`app/src/icons.tsx`**: `ExternalLink`.
- **Mock**: `remote_url` case in `install.ts`; `upstream` on the worktrees
  fixture's `(main)` and `feat-redesign` so both link shapes (branch page /
  home) are visible on one project.
- **CHANGELOG** `[Unreleased]` → Added.

## Decisions

- **The page rule lives in TS, the spec parser in Rust.** The old Rust command
  built the whole URL, and adding a second consumer (the project menu) would
  have meant either a second command or a slug-optional one — and the topbar
  needs the URL for a place whose `upstream` and `branch` the frontend
  already holds. One pure module with a fail-first check script is the
  repo's established shape for a mirrored decision (`dnd.ts`, `afterglow.ts`).
- **`upstream`, not the local branch name.** The snapshot sets `upstream` only
  when the remote-tracking ref RESOLVES (project.rs `status_v2`, gated on
  `branch.ab`), so `origin/x` is a promise that the page exists. The old
  `/tree/<local branch>` 404'd on every unpushed branch; the repo home is the
  better answer there. `ops.rs:539` creates with `--track origin/<branch>`
  only when that ref exists, so upstream and local names agree.
- **Topbar link hidden until the base is known; menus always show the item.**
  A link that has to say "no remote" when clicked is worse than no link. A
  menu item is a click away and the place menu already had the "No origin
  remote for this project" contract, so the project item mirrors it.
- **Re-read the remote on selection change and menu open, not once per app
  run.** A remote is added once and rarely changes, but "rarely" is not
  "never"; one `git remote get-url` at a user-driven moment is free. The
  `setRemoteBase` updater keeps the same object when the value is unchanged,
  so a re-read does not re-render the topbar.
- **The topbar link goes to the BRANCH page, not the repo home**, even though
  David's words were "a link to the repo". It sits beside the branch chip and
  the branch page is the repo at that branch, one click from home; the
  tooltip shows the exact URL so there is no guessing. Easy to flip if it
  reads wrong in use.

## Dead ends / gotchas

- **`useCallback` hoisting.** `loadRemote` was first defined next to the other
  verbs (~line 4110) and used in an effect at ~3657: `TS2448 used before its
  declaration`. Callbacks referenced by effects above the verb block must be
  defined next to their state.
- **The mock harness starts with the nav COLLAPSED (rail only).** The first
  `contextmenu` dispatch on `.project-h` found a 0×0 element and no menu
  opened — it read like the menu code was broken. Click the rail's first
  button (`.rail button`, "Places — hidden; click to pin") first, then
  dispatch. One click per evaluate, read in the next.
- **Two pre-existing console errors in the mock**: `Cannot read properties of
  undefined (reading 'dimensions')` from `TerminalPane.tsx:196`'s post-switch
  `setTimeout` into xterm, on every place switch in the harness. Not this
  change; logged to the roadmap.
- **Showing a test red by breaking the code under test** turned up one lesson:
  the first break of `remoteWebUrl` (dropping the null check) CRASHED the
  script rather than failing an assertion — a crash exits non-zero too, but
  it proves less. The second break (`"origin"` vs `"origin/"` prefix → a
  `tree//x` URL) was the clean red.

## Verification

- `cargo build --release -p worktrees-cli` (binary newer than `store.rs`
  confirmed), `make test` → exit 0, 335 `ok`, 0 `not ok`; `make lint` clean;
  `cargo test -p worktrees-core` 298 passed; `-p worktrees-cli` 7 passed;
  `cargo test -p app --lib` 48 passed (the new one shown red first by
  breaking `.git` → `.gi` in `normalize_remote`); `tsc --noEmit` and
  `cargo check -p app` clean; `node app/scripts/remote-check.mjs` green, and
  red on two deliberately broken rules.
- Mock harness on `:1477` (content-checked: served `install.ts` carries
  `case "remote_url"`), driven through the Chrome devtools MCP: topbar link at
  x=307 with the chip's right edge at 295 (one `--s3` gap), all three of slug,
  chip and link on midline y=25, `elementFromPoint` at its centre returns the
  button; title `Open github.com/demo/worktrees/tree/feat/ui-redesign` on the
  pushed place and `Open github.com/demo/worktrees` on `random-work` (no
  upstream). Project menu lists "Open on GitHub" between "Project settings…"
  and "Copy path", hit-testable; both clicks reached
  `plugin:opener|open_url` in the mock's console, no error banner.
- CI on #208: lint, rust ×2, install ×2, app ×2 green; `test (macos-latest)`
  still pending at merge (bats had passed locally).
- NOT verified: the real app's browser open. The call is the same `openUrl`
  the place menu has used since the opener plugin landed, so no new permission
  is involved — but nobody clicked it in the sandbox app this session.

## Follow-ups

- Click the topbar link once in the real app (sandbox `app/scripts/sandbox.sh
  --app`), on a pushed and an unpushed branch.
- The mock has no project WITHOUT an origin (the `remote_url` case returns
  `null` only for roots named `local*` / `deleted-thing`, which has no
  snapshot); the "hidden until known" and "No origin remote" states are
  visible only by reading the code.
- The `dimensions` teardown error in the mock's TerminalPane (above).
