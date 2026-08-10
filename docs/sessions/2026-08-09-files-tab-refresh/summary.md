---
title: "Session: the Files tab shows files as they are created"
---

# Session: the Files tab shows files as they are created

- **Date:** 2026-08-09
- **Worktree:** ui-changes
- **Branches:** ui-next-file-tree-refresh (fix), ui-next-file-tree-error-path
  (review follow-up), ui-closeout-files-tab-refresh (this archive)
- **PRs:** [#97](https://github.com/penard-monkey/worktrees/pull/97)
  (squash-merged → `0f0f508`),
  [#98](https://github.com/penard-monkey/worktrees/pull/98)
  (squash-merged → `5536d26`)
- **Release:** shipped in **v0.10.0**
  ([#100](https://github.com/penard-monkey/worktrees/pull/100), `c1b60dd`) —
  cut by another session the same day, which swept the `[Unreleased]` section in
- **Planning files:** none (single-thread session, two PRs)

## Where this started

> "I don't think current files viewer we have is getting all the files
> currently in the disk as they are created. Where does the files list come
> from?"

One symptom, two unrelated causes. Worth separating early, because the fix for
each is in a different layer and only one of them is a bug.

## The diagnosis

**Cause 1 — the tree listed each directory exactly once.** `TreeNode` fetched
its children inside the click handler, behind a `kids === null` guard:

```tsx
if (next && kids === null && !loading) {
  try { setKids(await invoke<FsEntry[]>("list_dir", { path: entry.path })); }
```

So the root was listed when a place was selected, a subfolder the first time it
was expanded, and never again. A file written after that point stayed invisible
for the life of the node. Collapsing and re-expanding did not help — `kids` was
still populated, so the guard still refused. The only way to see a new file was
to switch places and back, which remounts the whole tree (`<FileTree key={root}>`).

**Cause 2 — gitignored entries were filtered unconditionally.** `list_dir` in
`lib.rs` ran `git check-ignore` over each directory and dropped every match.
That is precisely the set a working session produces: build output, and this
repo's own gitignored planning docs (`task_plan.md`, `findings.md`,
`progress.md`). Not a bug — a deliberate choice that had simply stopped
matching what the pane is used for.

## What shipped

**#97 — the re-list, and the toggle**

- `app/src/FilesPane.tsx` — children fetched by `useEffect` keyed on
  `reloadToken` instead of by the click handler. Every OPEN directory re-lists;
  closed ones still cost nothing.
- `app/src-tauri/src/lib.rs` — `list_dir` takes `show_ignored: Option<bool>`;
  `FsEntry` gains `ignored`. Ignored entries come back **flagged rather than
  filtered** when asked for.
- `app/src/App.tsx` — `reloadFiles` callback; `↻` and `◌` buttons in the dock
  header.
- `app/src/App.css` — `.tree-row.ign` (dimmed to `--txt-mute`).
- `app/src/settings.ts` — `files_show_ignored`, persisted.
- `app/src/mock/install.ts` — gitignored fixtures, backend-parity filtering and
  sorting, and `__mock.createFile()`.

**#98 — the error path, from review of the merged diff**

- A failed reload keeps the last good listing instead of wiping the node.
- `FileTree` renders the error *above* a retained tree, replacing it only when
  there is no listing yet.
- The error is reported only when it **changes**.

## Decisions

- **Reuse `placesToken`; do not add an FS watcher or a new event.** The backend
  already emits `places:changed` on its poll (≤30s, faster on tmux activity),
  and `FileView` already re-read on it. Threading the tree onto the same token
  is a smaller change than a watcher and inherits #92's visibility gating for
  free — hidden window, no re-listing.
- **Flag ignored entries rather than filter them.** Returning them
  undifferentiated would dump `target/` and `node_modules/` into the tree as
  noise. Dimming makes the toggle useful rather than merely complete. `.git`
  stays hidden either way.
- **Fetch by effect, not by handler.** A handler runs once per click; the state
  we need to track (what is on disk) changes without a click. This is the whole
  fix — the guard was a symptom of the fetch being in the wrong place.
- **Report an error only when it changes** (#98). The failure path went from
  once-per-expand to once-per-bump, and a permanently unreadable directory
  would otherwise re-raise the banner and append to `app.log` every 30s forever.
- **Left the render churn alone.** Every open node does `setLoading` →
  `setKids(freshArray)` → `setLoading` per bump, with new array identity even
  when the listing is byte-identical. Child effects key on `entry.path`, so this
  is render waste, not a correctness bug. Noted as a follow-up rather than fixed
  under a review that was about the error path.

## Dead ends / gotchas

**Two vite instances on the same port, and the survivor served stale code.**
After being asked to kill the harness, `pkill` + a `lsof -ti:5199` check
reported the port free — and it was not. There were *two* vite processes; one
died, the other kept listening. Two traps compounded it: `lsof -ti:PORT` returns
Chrome's network-service helpers as well as the server (use `-sTCP:LISTEN`), and
CLAUDE.md's HMR warning is load-bearing — inside `.worktrees/` chokidar ignores
dot-directories, so the survivor happily served the **pre-edit** `FilesPane.tsx`.
Testing against it would have "verified" the old code. Caught by the served-vs-disk
diff CLAUDE.md prescribes:

```sh
curl -s localhost:5199/src/FilesPane.tsx | grep -c lastErr   # 4 (fix present)
```

Do the content check, not the port check.

**The mock returns a plausible fixture listing for any unknown directory.**
`seedDir()` falls through to the generic repo-root listing for any base name it
does not recognise, so `__mock.createFile()` against the *wrong* place's `src`
produced a directory that looked exactly right and proved nothing — the first
run of the re-list test showed "no change" and read as the fix not working. The
selected place was `.worktrees/feat-redesign`, not the monorepo root the path
was guessed from. Fix: instrument `invoke` and read the paths the tree actually
asks for, rather than reconstructing them.

**`origin/main` moved twice mid-session.** It was 5 commits ahead at PR time
(v0.9.1 had been released), and 3 more by close-out (v0.10.0). Two consequences:
`git stash` + branch off fresh main hit a real conflict in `App.tsx` (#92's
visibility gating landed in the same effect), and the CHANGELOG's `[Unreleased]`
section no longer existed, so the new entries had silently landed *inside the
shipped 0.9.1 section*. Check `git rev-list --left-right --count
origin/main...HEAD` when starting work, not when opening the PR.

**The review was right in aggregate and wrong in a detail.** It credited
`FileTree` with "deliberately keeping entries and showing the error without
blanking". It did not — `if (err) return <err-note>` replaced the entire tree,
unmounting every node and losing all expansion. That is the same bug it had just
flagged one level down, and it was missed because the code *reads* as if it
keeps `entries`. Reviewer findings are leads; confirm each against the source.

## Verification

Assertions in the mock harness (`getComputedStyle` / DOM probes, per CLAUDE.md —
not screenshots):

- a file created in an already-expanded directory appears after
  `places:changed`, and a `data-probe` stamp on the subtree's DOM node survives,
  proving it was re-listed **in place**, not remounted
- toggling `◌` re-lists root and expanded children; ignored rows carry `.ign`
  and resolve to `rgb(86,95,137)` (`--txt-mute`) against `rgb(192,202,245)` for
  tracked ones
- a newly created gitignored file stays hidden while the toggle is off
- with `list_dir` stubbed to reject one path: **5 consecutive failures produce 1
  reported error** (was 5), the subtree keeps its 2 children and 8
  grandchildren (was wiped to "empty"), a failed **root** reload keeps the whole
  tree with the error above it (was replaced entirely), and every case recovers
  on the next success

Gates on both PRs: bats 288/0, lint clean, `worktrees-core` 205,
`worktrees-cli` 6, `tsc --noEmit` and `cargo check -p app` clean. CI green on
all 9 checks before each merge.

## Follow-ups

- **Per-bump render churn** — see Decisions. The `lastSnap` byte-compare from
  #92 (`App.tsx`) is the house pattern if it is worth doing. Carried to
  ROADMAP.md.
- **No separate changelog entry for #98, deliberately.** Both PRs landed before
  the v0.10.0 cut, so no release ever shipped the error-path bug; the v0.10.0
  entry's claim that "the previous listing stays on screen" is true precisely
  because #98 landed. Recorded here so the gap does not read as an oversight.
