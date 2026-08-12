# Session — the Files tab says what the branch changed

- **Date**: 2026-08-11
- **Worktree**: `.worktrees/ui-changes`
- **Branches**: `ui-next` (squash-merged), `close-out-files-changed-markers` (this archive)
- **PRs**: [#115](https://github.com/penard-monkey/worktrees/pull/115) changed-file markers
- **Release**: unreleased (one `[Unreleased]` CHANGELOG entry)
- **Planning files**: `planning.tar.gz` beside this summary

## The report

> "would it be possible to see which files have changed in the current files
> section? So if the current worktree has modified files in the current branch
> (commited or not) would it be possible to see the files that were modified in
> the list? would also need to cascade up through the directory structure."

Three requirements, and the parenthetical is the load-bearing one. "Modified in
the current branch (committed or not)" is a **wider question than dirtiness**:
it is the union of what `git status` reports and what the branch's commits
changed against their base. The nav's `dirty_files` counter answers only the
first half, so a branch with clean commits reads as untouched there — and the
tree now says otherwise on purpose.

## What shipped

`app/src-tauri/src/lib.rs`, `app/src/FilesPane.tsx`, `app/src/App.css`,
`app/src/mock/install.ts`, `CHANGELOG.md`

### Backend — one command for the whole tree

`changed_files(root) -> ChangeSet { root, files: [{ path, status }] }`, with
`status` one of `modified | added | untracked | deleted`. Three spawns per call:

```
git rev-parse --show-toplevel
git diff --name-status -z <base>...HEAD          # committed on this branch
git status --porcelain -z --untracked-files=all  # uncommitted
```

Union, working-tree second so it wins. Paths come back **absolute and
canonical** — the same shape `list_dir` hands the tree — plus the canonical
`root` they hang off.

Two pure parsers (`parse_status_z`, `parse_name_status_z`) carry the format
knowledge and all seven new tests.

### Frontend — a flat list becomes per-row answers

`buildChanges()` turns the list into three maps once per refresh: `files`
(path → status), `dirs` (path → changed files beneath it, for the cascade *and*
the count badge), `ghosts` (dir → invented rows). `withGhosts()` splices ghosts
into a listing, filtering by name against what is really there. Rows cost one
`Map.get`.

Marker is the **name** — tint + weight, no new column: amber `--warn` modified,
green `--ok` added/untracked, red `--danger` deleted, and a changed directory
just lifted out of the folder grey (`--txt`, 550) with a count badge.

The fetch lives in `FileTree`, keyed on `(root, reloadToken)`.

## Decisions

| Decision | Why |
| --- | --- |
| **Base = merge base with the repo's base branch** — `<base>...HEAD`, candidates `origin/main → origin/master → main → master` | Mirrors core's `base_ref()`, the ref the nav's ↑↓ arrows already use. Three dots so a base branch that has moved on doesn't light up every file those commits touched. `@{u}` was rejected twice over: a branch with no upstream (most local branches) would show nothing, and a just-merged main would show hundreds. |
| **The name carries the mark**, not a new glyph column | The row already spends its glyph slot on file kind, and every other state this tree has — gitignored, symlink, inert — is a colour on the same text. A second column would be a second thing to scan. |
| **A changed directory gets no hue** | It did not change; something under it did. Undimming it *is* the mark, and the count is the detail — one file is a typo fix, twelve is a subsystem. |
| **Deleted paths get ghost rows** — invented, dimmed, struck, inert — including ghost *directories* | A deleted path has no `read_dir` entry, so without this a directory carries a mark and shows nothing marked inside it. `git rm -r tools/` takes the directory too, hence ghost dirs whose children come from the change set rather than a listing. |
| **Count badge yes; "changes only" filter and auto-expand no** | Explicit scope call by the user. Both deferred to ROADMAP. |
| **One command per refresh, not a batch per listing** | `list_dir` runs one `check-ignore` per OPEN directory per bump, and the tree re-lists every open directory on every bump — per-directory would be a git process per node per tick. |
| **Badge shown expanded too**, not only when collapsed | Same number either way; a badge that vanished on expand reads as "resolved". |
| **A selected changed row keeps its tint** | Selection still reads through the accent inset bar and the wash; dropping the tint would hide the state the user is looking at. |

## Dead ends / gotchas

- **`git status --porcelain -z` needs `--untracked-files=all`.** The default
  `-unormal` collapses a whole new directory into ONE `dir/` record. The
  directory would carry a mark and every file inside it would be unmarked —
  precisely the bug the ghost rows exist to prevent, arriving from the other
  direction. Found while writing the mock fixture, not by a test.
- **Rust's `\` line continuation strips leading whitespace.** A `-z` fixture
  written as one continued string literal silently lost the ` M` / ` D` status
  column, so the parser read `rc/App.tsx` and the test failed pointing at the
  parser. The fixture was wrong, not the code. Records are now built from a
  slice (`z(&[…])`) so leading spaces survive.
- **Working-tree status must be applied AFTER the committed diff.** It describes
  the disk the tree is about to list: a file added in a commit and then deleted
  has to come back `deleted` (a ghost row), not `added`. The first sketch ranked
  statuses by severity instead, which would have tinted a row for a file that
  wasn't there.
- **`AD` = staged-added then removed from the worktree**, so the delete arm has
  to precede the plain `A` arm in the match.
- **Porcelain paths are relative to the repo TOP-LEVEL, not to `-C`.** Resolved
  with `rev-parse --show-toplevel` + `canonicalize`, and RETURNED to the
  frontend rather than assumed: the ghost/cascade walk needs the same canonical
  anchor `list_dir` paths carry, and a place path can run through a symlink.
- **A pipeline's exit code is the LAST stage's.** `make test | tail -15`
  exited 0 because `tail` did, and a mid-stream `not ok` would have scrolled
  off the 15-line window. The bats verdict was re-taken with full output
  captured (`make test > log; echo $?`) before it was reported. Never read a
  gate's verdict off a pipeline.
- **`cd app` persists across Bash tool calls.** A later `make test` ran inside
  `app/` and died with "No rule to make target `test`" — nothing to do with the
  change. Use `make -C <root>`.
- **React batches, so the harness must click once per `browser_evaluate`.**
  Four expand clicks in one eval left the DOM untouched and read as "the tree
  ignored them"; the updates land after the eval returns.
- **Playwright MCP wrote its screenshot to the repo ROOT**, not
  `.playwright-mcp/`, and a `find` run seconds later missed it — it appeared
  later. Both copies were swept to the cache dir; check `git status` for strays
  before committing.

## Verification

| Gate | Result |
| --- | --- |
| `cargo test -p app --lib` | 18 passed (7 new) |
| `cargo test -p worktrees-core` | 208 passed |
| `cargo test -p worktrees-cli` | 6 passed |
| `make test` (bats) | 288 ok / 0 not ok, real exit 0 |
| `make lint` | clean |
| `tsc --noEmit`, `cargo check -p app` | clean |
| CI on #115 | all 9 checks pass (app/install/rust/test × macOS+Ubuntu, lint) |

Everything above was re-run ON the `origin/main` merge result, not just
pre-merge — main's per-space dock rework (#110–#114) touched four of the same
five files.

Beyond the parsers' unit tests, the union is pinned by a fixture of **real
captured git output** from a scratch repo whose branch carried a committed
modify + add + delete + rename plus uncommitted modify + delete +
untracked-in-a-new-directory.

Harness readings (`getComputedStyle`, mock backend, Tokyo Night):

| Row | class | name colour | weight | extra |
| --- | --- | --- | --- | --- |
| `src/App.tsx` | `chg chg-modified` | `rgb(255,158,100)` `--warn` | 600 | title "modified on this branch" |
| `src/new.tsx` | `chg chg-untracked` | `rgb(158,206,106)` `--ok` | 600 | title "untracked" |
| `README.md` | `chg chg-added` | `rgb(158,206,106)` | 600 | title "added on this branch" |
| `src` | `dir chg chg-dir` | `rgb(192,202,245)` `--txt` | 550 | badge `3` |
| `crates` → `worktrees-core` → `src` | `chg chg-dir` each level | — | 550 | badge `1` each (cascade) |
| `docs/old-spec.md` | `inert chg chg-deleted ghost` | `--txt-mute` | 400 | `line-through`, `aria-disabled=true` |
| `tools` (dir deleted with its file) | `dir chg chg-dir ghost` | `--txt-mute` | 400 | `line-through`, badge `1` |
| `main.rs`, `logo.png` (unchanged) | — | `--txt` | 400 | no badge |
| `App.tsx` while selected | `sel chg chg-modified` | keeps `--warn` | — | selection reads via accent bar + wash |

Screenshot: `~/.cache/worktrees/worktrees/ui-changes/tree-markers.png` (not
committed).

## Follow-ups

See ROADMAP.md — the "changes only" filter and auto-expand-to-changes were
deliberately deferred, the nav's dirty count and the tree's badge count
deliberately different things, and the markers have no settings toggle.
