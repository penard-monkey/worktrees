# Changelog

All notable changes to this project are documented here.
Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning: [SemVer](https://semver.org/).

## [Unreleased]

### Added
- **A pull that mirrors a source Mac which was behind origin — or dirty — now
  says so.** A faithful mirror of a machine that was seven commits behind is
  seven commits behind, and nothing about the transfer explained that: *imported
  state mirrors the source Mac: main is 7 commit(s) behind its upstream (git pull
  to advance), 2 uncommitted file(s) — mirrored state, not transfer damage.* The
  line appears in the terminal and in the app's sync modal, and only when there
  is something to explain; a tree that came back level and clean says nothing.

### Fixed
- **`sync pull` no longer lands a project full of `deleted:` files.** The exclude
  list is rebuildable bulk — except repos *commit* files that match it
  (`old-plans/*.tar.gz`, this repo's own session tarballs). The transfer skipped
  those working files while the `.git` it ferries kept saying they exist, so a
  fresh-machine import opened on a wall of deletions. Nothing was ever lost, and
  a pull now restores exactly those files from the `.git` that just arrived — in
  the project root and in every worktree under `.worktrees/` — and reports what
  it did: *restored 3 tracked file(s) the transfer skips (excluded as
  rebuildable): …*. A deletion **you** staged, and any deletion the exclude list
  does not explain, is the other machine's intent and is left exactly as it
  arrived. Restored files come back at their last committed version; an
  uncommitted change to an excluded file never rode along in the first place.

## [0.15.0] - 2026-08-16

### Added
- **`worktrees sync` carries a project between Macs on an SSD.** `sync push`
  mirrors the project root — worktrees, `.git`, and all the gitignored state a
  `git push` cannot carry — onto a mounted hub; `sync pull` brings it back on the
  other machine; `sync status` says what the hub holds. Rebuildable bulk
  (`node_modules/`, `target/`, `Pods/`, …) is skipped and reported as it goes, so
  the transfer is a fraction of the tree. Every direction previews first — *N to
  send/update, M to delete*, with the deletions listed — and asks before it
  mirrors; `--dry-run` stops at the preview, `--yes` skips the question, and
  everything overwritten or deleted lands in `.worktrees-sync/backups/<stamp>/`
  rather than nowhere.
  It works **outside a repo** too: `sync pull <name>` on a machine that has never
  seen the project reads the manifest the push left on the hub, shows you the
  destination, and adopts it after you confirm. `sync status` with no repo around
  lists every project on the hub.
  `--with-sessions` additionally ferries this project's Claude Code transcripts
  in **both** directions without deleting either machine's history, and
  `pull --install` runs your own rebuild command from
  `~/.config/worktrees/config.toml` (`[sync.projects.<name>] install`).
  The hub manifest is **data**: it is parsed strictly, an unknown key is a hard
  error, and the rebuild `hint` it carries is printed and never executed — ADR
  0001's rule that a foreign file may not supply argv, applied to removable media.
  Optional config: `[sync] hub`, `[sync] with_sessions`, and per-project
  `extra_excludes`.
- **Two guards keep a sync from eating what it is syncing.** A tree that arrived
  on a hub is another machine's mirror — its `.git` registers worktrees at paths
  that exist only there, so a prune inside it can unregister the wrong repo's —
  and every mutating command (`new`, `switch`, `open`, `close`, `rm`, `relink`,
  `provision`, `init`, and both sync directions) now refuses there and names the
  native path to run at instead. `ls`, `doctor` and `sync status` still work,
  because they are how you find out what the tree is; `doctor` reports the copy
  (`hub-copy`) and where the project really lives.
  And `sync pull` looks at what is *living* in the destination before it mirrors
  over it: with tmux sessions open in that tree it lists them before asking, and
  `--yes` on its own is refused — blanket consent given by a script before it
  could know is not an answer to "something is working in there right now".
- **Sync from the app.** Right-click a project → *Push to \<hub\>…* / *Pull from
  \<hub\>…*. Both open a preview first: what would be sent, what would be
  deleted (listed, in red), what is being skipped as rebuildable, and a checkbox
  for ferrying this project's Claude sessions along. Confirming runs it and the
  modal reports what landed. The items name the drive they found, and grey
  themselves out with the reason when there is no hub mounted — or when the
  checkout you right-clicked is itself a hub copy. A pull onto a tree with live
  tmux sessions lists them before you confirm, and if one appears while you are
  reading the preview, the transfer stops and asks again with the fresher list.
- **A sync you can watch.** The modal now shows rsync's own progress while it
  runs — a bar, the percentage, the transfer rate and the file it is on — instead
  of a button frozen on *Pushing…*. The first push of a big project is minutes of
  work, and minutes of no feedback is indistinguishable from a hang. On the
  system `rsync` (openrsync), which cannot report progress at all, the bar says
  so by staying indeterminate rather than inventing a number.
- **Sync has its own button.** Hovering a project row reveals a ⇄ next to `+`
  and `×`; it opens the same *Push to \<hub\>…* / *Pull from \<hub\>…* pair the
  right-click menu has, headed by the drive it found (or the reason there is
  none) and footed by when this project last went to the hub, from which machine.
- The *Include Claude sessions* checkbox is **remembered** — it is the same
  answer every time for a given person, and re-ticking it on every push is how a
  checkbox teaches you to ignore it.
- A pull offers to **run your rebuild** afterwards, as a checkbox that names the
  command (`[sync.projects.<name>] install`), and only when you have registered
  one. Unticked by default: running a build is an act, not a preference.
- **One way in for projects, three ways to arrive.** *Add project* (nav footer,
  and the rail's folder button) now opens a menu: **New project…**, **Add
  existing…** (today's pick-a-repo behaviour, one item deep) and **Import from
  hub…**. The Home screen keeps the two that start a workspace — new project and
  import — since a new *user* creates and a new *machine* imports.
- **New project… makes the folder for you.** A name, a location you can type,
  paste or *Browse…* to (prefilled with the folder your projects already share),
  and the full path spelled out under the fields as you type it — then one act
  creates the directory, `git init`s it with an empty first commit and adds it to
  the workspace. A folder picker could only ever choose something that already
  existed, so "new project" used to mean making the directory in Finder first.
  Names that would steer a path (`/`, `..`, a leading dot, whitespace) are
  refused as you type *and* again in the backend, and a target that already
  exists is never written into — if it is a git repo the error says to use *Add
  existing…* instead.
- **Import a project from the hub — in the app.** *Import from hub…* lists every
  project the drive holds: where each
  one will land, when it was last pushed and from which Mac. Ones already in your
  workspace are there but dead, labelled *already in workspace*. Picking one
  opens the same preview/confirm modal every sync uses, headed by **the folder it
  is about to create** — a path this Mac has never had, which is exactly why it
  leads the modal — and confirming transfers it, with the same live progress bar,
  then adds it to the workspace so a row appears in the nav. This is the flow a
  brand-new Mac needs: until now the app's sync surface hung off a project row,
  and a project that is not in the workspace has no row. If the transfer lands
  but the workspace add fails, it says so — which half worked, and what to do —
  instead of reporting a clean success or a clean failure. With no drive
  mounted, the picker says why and offers nothing to click.

### Fixed
- The hub-copy guard now also covers the **app** and **`worktrees mcp`**. Both
  drive the same core ops in-process and so never passed the CLI's dispatch
  check — every op the app runs, and every non-read-only MCP tool, is refused
  inside a tree that arrived on a sync hub, with the native path named.
- **A parked job no longer leaves the green "Claude working" dot on forever.**
  Parking a turn that is mid-flight rewrites that session's probe file with the
  job id but leaves its status at `busy`, and nothing ever writes it again —
  two places here sat green for 22h and 32h next to sessions idling at a prompt.
  A parked session whose last write did not set the status it carries no longer
  lights the dot; while the parked job really is running, its own session lights
  the same place, and a new turn in a session that once parked a job lights it
  as before.

## [0.14.0] - 2026-08-14

### Added
- **Markdown docs in the Files tab have their own text size.** A `A− 100% A+`
  stepper sits in the viewer header whenever a document is being *read* (it is
  absent in Source view, which is code and follows the terminal font), and
  ⌘+ / ⌘− / ⌘0 do the same from the keyboard, in the dock and in the reader
  (⌘⇧E) alike. The whole document scales together — headings, tables, code
  fences and the spacing between them — so a README at 175% still looks like a
  README. Deliberately separate from Settings → UI font size: enlarging one long
  document should not move every column in the app.
  **Each place remembers its own size**, so a worktree full of wide tables can
  read small while the one you're writing docs in reads large. A place you have
  never set a size in inherits the last one you chose, then keeps its own.
- **⌘F finds things.** In a terminal it searches the buffer — the place's tmux
  pane or a dock shell tab — with every match tinted, the one you are on
  brighter, and ⏎ / ⇧⏎ stepping through them. In the Files viewer it searches
  the file you have open, in the rendered markdown as readily as in source, and
  follows you when you switch files. Which one ⌘F opens depends on where you
  were last working; Esc closes it and hands the keyboard back. The terminal
  can only search what it has received since it attached — tmux keeps its own
  scrollback and does not replay it — which the field says on hover.
- Settings → Shortcuts lists ⌘J, ⌘⇧E and ⌘⇧T, which it had been leaving out.
- **Drag a place into another group to put it there.** Dragging used to be a
  sorting gesture — only in Manual mode, only inside the group a row was already
  in. Now dropping a row on Pinned pins it, on Saved/Closed/Archived/Abandoned
  sets that lifecycle, in any sort mode, and a gap opens where the row will
  land: at the pointer under Manual, at the slot the list will actually put it
  in under A–Z and Activity. Empty groups appear while you drag, so you can pin
  the first place in a project or reach a tier you have emptied.
- **The gap tells the truth about where a row is going.** Active and Idle are
  not labels — they are readings of whether a tmux session is up and how
  recently a place was opened — so a row dropped on them lands wherever those
  readings put it. The drop preview names that group before you let go, the row
  is lit for a moment once it gets there, and if the landing tier is one you
  have hidden the app says so rather than letting the row vanish. Dropping on
  Active explains itself instead of doing nothing.
- **Undo for a drag.** A drag now rewrites declared state, so a slip of the
  wrist gets one click back — pin, lifecycle and manual position together.
- **Projects reorder by dragging their headers.** The workspace order was the
  order projects happened to be added in.
- **Escape cancels a drag**, and the nav scrolls when you drag a row to its top
  or bottom edge — a place could not previously be dragged past the visible
  list.

## [0.13.0] - 2026-08-14

### Added
- **The terminal tabs you opened are still there next time.** Shells do not
  outlive the app, and the only thing that used to bring a tab back was having
  NAMED it — so three tabs you had been working in reopened as one, and which
  ones survived depended on whether you had bothered to label them. The tab
  strip is now remembered per place, named or not. Each tab still spawns its
  shell fresh, in the directory it was last in.
- **A place remembers which terminal tab you were on.** Coming back to a space
  always put you on the lowest-numbered tab, whatever you had been looking at —
  and because the tab strip is rebuilt every time you change place, that
  happened when clicking between places, not only after a restart. Each place
  now remembers its front tab. A remembered tab that is no longer there (you
  closed it, or its shell did not survive a restart) falls back to the first
  one, without forgetting what it was.
- **Terminal tabs reopen where you left them.** A dock shell lives and dies with
  the app, so quitting used to drop every tab back at the worktree root however
  deep you had worked. Each tab now remembers its directory and starts there
  again — the directory only: no history, no scrollback, nothing else about the
  session. A remembered directory that no longer exists falls back to the
  worktree root, and closing a tab forgets it — while restarting a shell that
  exited brings the tab back where it was. The place's own terminal is
  unaffected: tmux was already keeping its directory for as long as its server
  lives.

### Changed
- **Every list of places is now in the same order, by the same clock.** ⌘K, the
  Recent lens and "Resume where you left off" each used to rank by a slightly
  different notion of recent — one counted opening a place, one fell back to its
  last commit, the tree counted neither — so the same three places could appear
  in three orders, and a row's age often had nothing to do with where it sat.
  All of them now order by the date the nav has always shown: when something
  *happened* there, your work or a commit. Opening a place still doesn't move
  it, so clicking around never reshuffles anything.
- **⌘K rows show that date.** The switcher listed places in a recency order it
  never displayed; now each row carries the same age as its row in the tree.

## [0.12.1] - 2026-08-11

### Fixed
- **A background refresh can no longer put a stale name back.** Every screen
  refresh is a sweep of all your projects, several can be running at once, and
  whichever finished last used to win — so a sweep that started *before* a rename
  could land *after* it and quietly restore the old name, where it stuck until
  something else changed. Reads are now applied in the order they were started
  and stale ones are dropped, which also fixes a removed project reappearing.
- **Renaming a place (or pinning one, or editing its note) shows up at once.**
  The change was written immediately, but the nav only caught up after the app
  had re-scanned every registered project — each one a fan-out of git calls, so
  a workspace of any size left the old name on screen long enough to look
  broken. On a single-project setup it was instant, which is why it survived
  testing. Declared edits now apply to the tree the moment they are made; the
  full re-scan still follows and still has the last word.

## [0.12.0] - 2026-08-11

### Added
- **The Files tab says what the branch changed.** The tree tints and bolds every
  file that differs from the branch's base — committed on this branch *and*
  uncommitted, staged, unstaged or untracked alike — so the dock answers "what is
  this worktree's work" without a terminal. Amber is modified, green is new
  (added or untracked), red is deleted. The mark cascades: a directory anywhere
  above a changed file lifts out of the folder grey and carries a count, which is
  what tells a one-file fix from a rewritten subsystem while it is still
  collapsed.

  The base is the merge base with the repo's base branch (`origin/main`, else
  `origin/master`, else the local ones) — the same ref the nav's ↑↓ divergence
  already measures against, and the reason a base branch that has moved on does
  not light up every file those other commits touched. Note this is a wider
  question than the nav's dirty count, which stays uncommitted-only: a branch with
  clean commits shows marks in the tree and no dirty badge in the nav.

  A deleted path has no directory entry to tint, so the tree invents its row —
  dimmed, struck through and inert — including the directories a `git rm -r` took
  with it. Without those a directory would carry a mark and show nothing marked
  inside it. Cost is three git invocations per tree refresh for the WHOLE tree
  (the tree already spends one `check-ignore` per open directory), and none at all
  while the reader is expanded.
- **Places can be given a name.** A worktree was always called whatever its
  directory is called — useful for `cd`, less useful for "which of these three
  is the auth work". The `⋯` menu (or a double-click on the name in the header)
  now sets a display name, and ⌘K, the nav filter and alphabetical sort all
  follow it, so a renamed place is findable by the name you can actually see.
  Clearing the name goes back to the slug.

  The name is a LABEL: the directory, the branch and the tmux session keep the
  slug, which stays on screen next to the name rather than being replaced. That
  is deliberate rather than a shortcut — the slug is not a name the tool stores,
  it is the directory's basename read fresh from disk, so renaming the *place*
  would mean renaming the directory and with it the git registration, the tmux
  session and its shell sidecars, the recorded compose project, and the Claude
  history directory, which is keyed on the absolute path. That last one would
  silently orphan the conversation and break auto-resume — too high a price for
  something that looks like editing a label. AI profiles already split an
  immutable id from a renameable name; places now do too.

  The main checkout can be named as well, which is where it helps most: its slug
  is the literal `(main)`.
- **Every space remembers its own panels.** Whether the dock is open, which tab
  it is on and how wide it is are now remembered per worktree, so coming back to
  a place finds it the way you left it instead of however the last place you
  visited happened to be set up. A place where you have never opened the dock
  starts closed, and stays closed until you open it there — opening the dock in
  one worktree does not open it everywhere else you then click. What you set up
  survives quitting the app.

  Only those three things are per-place. How you like to *read* a file — the
  Files tab's split, wrap and gitignored toggles — stays global, as does the
  nav, which is how you leave a space rather than part of one. The open file is
  not remembered on purpose: a path can be deleted or renamed between visits,
  which would turn "restore what I left" into an error on arrival.

### Changed
- **The top bar now crowns the whole space, not just the terminal.** Files and
  Terminal read as app furniture that happened to point at a place, because
  structurally that is what they were: the header was a child of `main`, and the
  dock was a separate grid column sitting *beside* it, outside the header's
  scope. The header now spans the terminal and the dock together — they share
  one `space` cell, and the dock is a flex sibling of the terminal rather than a
  column of its own. The layout picture in DESIGN.md predates the dock, which is
  how it came to be bolted on next to the model instead of into it.

  The dock could not simply span two grid columns: the columns are *removed*
  when hidden rather than zeroed, so every line index shifts when the nav
  collapses. Reading mode gets simpler as a result — it used to need an inline
  negative offset to escape `main` and reach across the dock, and now just fills
  the space body, which is already exactly that wide.

## [0.11.0] - 2026-08-10

### Fixed
- **Symlinks in the Files tab are symlinks.** `read_dir` answers about the link,
  not its target, so a symlink to a directory came back as a plain file: a file
  glyph, no caret, and a click that tried to open a directory as text. The tree
  now stats through the link for its shape and marks it `↗` with the target in
  the tooltip. A link the app will not follow — one leaving the workspace
  (`_tmp` pointing at iCloud), one into `.git`, or one that dangles — renders
  inert with the reason in its tooltip, rather than offering a caret. Not
  following is the point rather than a limitation: a repo's own contents do not
  get to choose what the app reads. For a link out or a dangling one the guard
  would refuse the call anyway, so the row was only ever going to produce an
  error banner. For `ln -s .git` it would not — the listing hides `.git` by
  *name*, and a link is a way around that — so there the classifier is the only
  thing that stops it.

### Changed
- **The Files tab shows gitignored entries by default.** 0.10.0 made them
  reachable behind the `◌` toggle but left them hidden until you found it, and a
  tree that withholds entries without saying so does not read as a filter — it
  reads as a listing that lost files. The working notes a session had just
  written (`task_plan.md`, `findings.md`, `progress.md`) and `node_modules/`
  were simply absent next to a shell listing them. They now show dimmed from
  the start, and the toggle's glyph fills (`◉`) when nothing is being withheld,
  so the hiding state is legible without hovering. Installs that already
  persisted the old default are migrated once; toggling it back off still
  sticks. `.git` stays hidden either way.
- **The nav tree's clock and order track activity, not attention.** Each row's
  age and the tree's default sort (labelled "Activity" in the sort menu,
  previously "Last used") both counted merely opening a place as recency, so
  clicking a row reset its age to "now" and reshuffled the order — the glance
  destroyed the very signal it was after. Both now move only when something
  happens in the worktree: Claude finishes work there, or the branch tip
  changes. Opening a closed place still brings it back into the active group,
  but its age and its order within the group no longer flinch. The Recent
  lens, the Resume list, and ⌘K still count opens on purpose — "where was I"
  is their job, and the Recent lens now labels its rows with that same clock.

## [0.10.0] - 2026-08-09

### Added
- **The Files tab can show gitignored files.** A `◌` toggle in the dock header
  lists them alongside the tracked ones, dimmed and labelled "gitignored" on
  hover, and the choice persists. Everything a session actually produces —
  build output, and this repo's own gitignored planning docs — was invisible in
  a tree that filtered them unconditionally. `.git` stays hidden either way.
- **A place stays lit after Claude finishes there.** The green blinking dot only
  ever answered "is Claude working *right now*" — the moment a task landed, the
  nav forgot it, and "what moved recently" had no answer at all. Places now carry
  a third dot state: a purple ember that appears when a session finishes work and
  decays out over the working day (full with a halo under 15 minutes, dimmer to
  two hours, fainter to twelve, then nothing). It is deliberately not a
  brightness ramp on the green dot — motion stays reserved for "running now", so
  the two can never be confused, including for anyone running with reduced
  motion, where the blink is disabled entirely.
- Merely *opening* a session does not light it. The stamp comes from a session
  that actually went busy and came back out (an idle-but-open session never
  qualifies), so the ember means work happened, not that you looked in. A place
  worked on while the app was closed still lights up on next launch: startup
  backfills the last twelve hours from Claude's own prompt history, ignoring
  housekeeping commands like `/clear`. Sessions run in a repo's main worktree
  count the same as any other place.

### Changed
- "Recent" now means recently *used*, not recently *opened* — the Recent lens,
  the home screen's Resume list, ⌘K's ordering and the nav's recency sort all
  rank on whichever is newer, so a place you prompted in an hour ago outranks
  one you merely clicked into yesterday.

### Fixed
- **The file tree picks up files created while it is open.** A directory was
  listed exactly once: the root when you selected the place, a subfolder the
  first time you expanded it, and never again — the fetch was guarded on
  "children not loaded yet", so a file written after that point stayed
  invisible for the life of the node, and collapsing and re-expanding did not
  help either. Open directories now re-list on the same signal the file viewer
  already re-read on (the backend's poll, so within seconds), and the dock
  header gained a `↻` for when that is not fast enough. Expansion state and
  scroll survive a re-list, and the previous listing stays on screen while one
  runs, so the refresh never blanks the tree.

## [0.9.1] - 2026-08-07

### Changed
- **The app stopped burning power in the background.** It showed up under
  macOS's "Apps Using Significant Energy" while doing nothing, and the cause was
  that it did the same work whether or not anyone was looking at it: a forced
  refresh every 30 seconds fanned out to a git subprocess per place per project,
  measured at ~1.3 spawns per second on an idle machine with the window not even
  frontmost. Every periodic cost now gates on window visibility — the
  `places:changed` refresh, the usage countdown's tick and poll, and the
  five-minute doctor/init sweep — and each one catches up the moment the window
  comes back, which is exactly when a stale view is most obvious. Measured on
  the same workspace, backgrounded and idle: **CPU 3.19% → 0.10%, energy impact
  3.22 → 0.10**, and observed child processes over two minutes 159 → 1. Visible
  and idle, where the app must keep working: **CPU 3.19% → 1.80%, energy impact
  3.22 → 1.91.** The process counts come from a 1 Hz sampler that undercounts
  very short-lived spawns, so treat them as the shape rather than the exact
  figure; the backend's own 3-second tmux poll is not gated by this change and
  is now the largest remaining background cost.
- **A snapshot costs a third of the subprocesses it used to.** None of this
  changes the shelling-out-to-git-and-tmux design; it stops paying twice for
  answers already in hand. One `git status --porcelain=v2 --branch` replaces
  three calls (`rev-parse --abbrev-ref HEAD`, `status --porcelain`,
  `rev-parse @{u}`); one `git log` with a tab-joined format replaces the separate
  `%ct` and `%s` calls; the canonical tmux session check reads the pane list the
  snapshot already prefetched instead of running its own `list-sessions` per
  place; and the BSD-vs-GNU `stat`/`date` probe is now genuinely done once per
  process rather than once per snapshot, with the birth-date lookups memoized
  behind it. **91 → 55 subprocesses for a cold snapshot of a nine-place repo, and
  91 → 32 in the repeat-poll regime the app actually runs in.** `ls --json`
  output is byte-identical to 0.9.0 on a real workspace, verified by diff, with
  one documented exception: a repo with no commits yet whose upstream already
  resolves now reports no upstream, a field nothing consumes.
- A refresh that returns an unchanged workspace no longer replaces state and
  re-renders the tree. The forced 30-second poll produces a byte-identical
  snapshot on an idle machine, and every one of them was rebuilding the nav.
- The terminal cursor stops blinking when the window is not focused. xterm gates
  blinking on its own focus class, which never dropped here: the pane is
  force-focused on mount and on every re-entry, and element focus survives the
  OS window being deactivated — so the cursor repainted twice a second for as
  long as the app was open. The busy-session pulse is stepped rather than
  smoothly interpolated for the same reason: it was driving a compositor frame
  at display rate for as long as any session was working.
- **Creating a worktree says so while it works.** The nav now shows the place
  being created the moment you submit the form, with the form dismissed and a
  spinner on the row, instead of leaving the whole app silent for the seconds
  the work actually takes. It read as hung because nothing anywhere
  acknowledged the click — the form fired its op un-awaited and no part of the
  UI held a pending state. The row hands over to the real one with no gap, and
  a failed create retires it rather than leaving it spinning.
- **`new` on a branch you are inventing contacts the remote once, not twice.**
  It used to fetch `refs/heads/<branch>` — a request that cannot succeed for a
  branch that does not exist yet, which is the usual case — and then fetch the
  base separately, so the common path paid two network waits to learn one
  thing. A single `git fetch origin` answers both questions, taking ~0.8s off
  every such `new`. When the tracking ref is already on disk it still fetches
  nothing at all, as before. Narrowing worth knowing: the single fetch honours
  the repo's configured refspec, so a `--single-branch` clone no longer
  force-materializes a remote branch it was set up not to see — `new` creates a
  local branch off the base there instead. `switch` keeps its own pair of
  targeted fetches for now, so `new` onto an existing worktree that sits on
  another branch still goes through that older path.

### Fixed
- The upstream shown for a place whose remote-tracking branch no longer exists
  (deleted on origin, or never fetched) stays empty, as it was. Worth recording
  because the porcelain-v2 consolidation above nearly changed it silently: git's
  `# branch.upstream` reports the *configured* upstream even when it does not
  resolve, where the `rev-parse @{u}` it replaced reported only one that does.
  `# branch.ab` is the discriminator, and it is free.
- **Release notes render their bold and italics instead of printing the
  asterisks.** The "What's new" sheet parses the changelog itself — sections,
  group chips, hard-wrapped bullets — but its inline pass only knew about
  `code spans`, so every `**lead-in**` in a release since the notes gained them
  came out with the markup showing. Strong and emphasis now render, including a
  code span nested inside a bold lead-in, and the mock changelog carries all
  three so the harness exercises them.
- **Reopening a place is no longer logged as a warning.** Finding the session
  already up is what a durable place IS — the normal outcome, not something to
  act on — but it was emitted at warn severity, and the app logs warnings even
  for commands that succeeded, so every single `open` wrote a `[warn]` line and
  buried the log's real content. It also claimed to be "attaching" on the app
  path, which passes `--no-attach` and embeds the session in its own PTY; the
  message now says what actually happened.

## [0.9.0] - 2026-08-06

### Added
- **AI profiles — per-project Claude rules, skills, MCP servers and model.** A
  profile is a named bundle of rules text, skills, MCP servers, settings and
  model, applied to the `claude` session worktrees launches in a worktree's tmux
  pane. Your normal terminal `claude` is untouched. A profile is either the
  global default or bound to one project, and a project profile REPLACES the
  default rather than merging with it — merging two rule sets produces a third
  one nobody wrote. `WORKTREES_PROFILE=none` opts a single launch out, and
  `worktrees open` from a terminal resolves the same profile the app would:
  one resolver in core, so the CLI and the app cannot drift. The mechanism is a
  `CLAUDE_CONFIG_DIR` swap, verified against claude 2.1.220 — rules ship via
  `--append-system-prompt-file`, MCP via `--mcp-config` plus
  `--strict-mcp-config` when not inheriting globals, which is what lets a
  profile REMOVE a global server. Each profile holds its own credential through
  a one-time `/login` in its pane, because claude derives its keychain service
  name from the config-dir path; worktrees never copies, reads or stores a
  token, which is also why deleting a profile reports the keychain item it
  cannot remove instead of pretending to have cleaned it. It **fails closed**:
  a profile that cannot be materialized opens the pane on a plain shell with
  the reason and does not launch claude, because profiles are usually
  restrictive and "could not apply your profile" must never quietly mean "ran
  without your restrictions". Stated in the UI and in DESIGN.md: a profile
  controls user scope, not the project's — a repo's committed
  `.claude/settings.json`, `.mcp.json` and `CLAUDE.md` still load, and
  `~/.claude/CLAUDE.md` with its @-imports loads regardless, so profiles ADD
  rules and cannot suppress yours. Binding a profile to a repo that already has
  conversations starts a fresh one, since history lives with the profile;
  nothing is deleted, and unbinding brings it back.
- **A skill store (`worktrees skills`)** that treats an installed skill as
  instructions the model will read: capability-shaped frontmatter is surfaced
  for review before install, git installs are pinned to the reviewed commit and
  refuse if the branch moved underneath them, and installing never executes
  anything from the source.
- **`worktrees mcp`, a hand-rolled stdio MCP server** exposing the worktree
  model to a session. Read-only by default; worktree mutations are opt-in per
  profile and the destructive ones are additionally confirm-gated. Hand-rolled
  because rmcp would have taken the CLI from 32 to 124 crates and dragged tokio
  into a sync binary.
- **The usage bars say when the window resets.** Every row in the nav footer
  gains a countdown column: `5h  ▬▬▬  35%  3h 02m`, and the weekly rows read
  `2d 5h` — days and hours, minutes dropped at that scale. The model-scoped
  bucket (Fable) gets the same treatment; a percentage alone never said whether
  it was worth waiting out. Two units, biggest first (`<1m` / `47m` / `3h 02m` /
  `2d 5h`), tabular figures so the column can't twitch as it ticks. The
  countdown runs off the local clock at 15s — `resets_at` is absolute, so the
  rate-limited endpoint keeps its 180s poll untouched. A window whose reset has
  already passed (the statusline snapshot is often that stale) shows a blank
  cell rather than a negative one, and the absolute reset time stays in the
  tooltip.
- **`doctor` now reports files the config never learned about.** Every other
  check is judged *by* `.worktrees.toml`, so a config that stopped being true was
  invisible to all of them: detection ran exactly once, at `init`, and the
  passive hint on `new` only fires for repos that have no config at all. A
  credential added the day after the config was written was therefore
  undetectable, and every worktree silently lacked it. The new `undeclared`
  finding asks the reverse question — what is gitignored, untracked, and named by
  no `[[file]]` entry. Warn for a credential, Info for `.env*`; repo-scoped, so
  it is said once and not once per worktree. It exits 0 by default (drift is the
  steady state); `doctor --strict` promotes the credential warning to a failure,
  alongside `copy-stale`. An undeclared `.env*` stays Info and never fails a run
  — the same asymmetry `--strict` already has with `copy-stale`.
- **`worktrees init --diff`** prints just the `[[file]]` stanzas an existing
  config is missing, as an appendable fragment on stdout — the second look the
  flow never had. `init` refuses to run over an existing config and `--force`
  re-renders from scratch, which destroys every hand-set `mode = "copy"` and the
  comment explaining why. This writes nothing: paths the parser refuses come out
  commented with the reason, exactly as `init` emits them.
- Adding a folder that isn't a git repository now offers `git init` plus an empty
  first commit, instead of refusing with "Not inside a git repository." A repo
  that has no commits yet is spotted in the nav too: the new-worktree form is
  replaced by a "Create initial commit" action, because git cannot branch off an
  unborn HEAD.

### Changed
- **The nav tree now ranks the project above its places.** With one project open
  the header's position carried it; with several, the eye found `★ bug-fixes`
  before it found the repo that owns it — the project name was literally smaller
  type than its own children (13px against a 15px slug, same colour, no rule
  between projects). The name now matches a slug's size at bold weight, projects
  are separated by a hairline, and the header **sticks to the top of the nav**
  while you scroll its places, so a long PINNED list can't orphan its rows.
- **Dormant recedes by fading, not by banding.** The dark full-bleed rectangle
  was darker as designed, but a hard-edged rect reads as a divider rather than
  as depth: it cut across the tree and, sitting last in each project, doubled as
  a false separator competing with the next project header. The group now sits
  at 62% opacity with no fill, and comes back to full on hover, on keyboard
  focus, and whenever the place you're standing in lives inside it. Its caret is
  the same SVG chevron every other group header uses.

### Fixed
- `worktrees new` in a repo with no commits used to fail with git's own riddle
  (`fatal: not a valid object name: 'main'`). It now refuses up front, naming the
  cause and the one command that unblocks it.

## [0.8.0] - 2026-08-03

### Added
- **The dock's Files tab reads documents properly.** Markdown renders (headings,
  nested and task lists, GFM tables, blockquotes, fenced code, links, relative
  images) with a Preview/Source toggle; source files get syntax highlighting and
  a line-number gutter; images show inline over a checkerboard with their
  dimensions, byte size and a fit/1:1 toggle; PDFs, archives, fonts and media
  get a named placeholder with "Open in editor" and "Reveal" instead of a bare
  "binary file".
- **The Files tab lays out to fit.** Past ~620px of dock width the tree moves
  beside the content instead of above it; the divider drags and the ratio
  persists. A header button cycles auto → stacked → side-by-side.
- **Reading mode (⌘⇧E).** The open file expands over the main pane at a proper
  reading measure; Esc or the Collapse button returns. The dock falls back to
  showing just the tree while it is up.

### Changed
- **The dock's file viewer is read-only.** Editing was a plain `<textarea>` with
  a save path; it is now a renderer, and edits go through "Open in editor". This
  removes the save-conflict UI and any chance of the dock clobbering a file the
  agent in the next pane is writing. (`write_file` remains in the backend.)

## [0.7.0] - 2026-08-01

### Added
- **Claude plan usage in the nav footer.** Three hairline bars mirror Claude
  Code's `/usage` panel: the 5-hour session window, the weekly all-models
  window, and any model-scoped weekly bucket (e.g. "Fable"), colored by the
  severity Anthropic reports (normal / warning / exceeded), with reset times in
  the tooltip. Data comes from the same endpoint the `/usage` panel uses,
  authenticated with the Claude Code login already in the macOS Keychain — the
  first fetch may show one Keychain prompt. If that's unavailable the app falls
  back to the statusline snapshot in `~/.claude/widgets/rate_limits.json`
  (rendered dimmed), and with no source at all the widget simply stays hidden.
- `.nvmrc` (22.13.0 — the floor pnpm 11 requires). `make install-app` checks the
  active Node against it up front rather than letting pnpm fail with its own
  version error minutes into the cargo build.

### Fixed
- Removing a place from the app always failed with `invalid args 'delBranch' for
  command 'remove_place'` and never reached the CLI. The frontend passed the flag
  as `del_branch`, but Tauri renames Rust snake_case parameters to camelCase
  across the IPC boundary. The mock harness now rejects the wrong spelling the
  same way the real backend does — it previously ignored the flag entirely, which
  is why the bug survived headless testing.
- The "Not configured" banner no longer sticks for the rest of the session once a
  `.worktrees.toml` appears from outside the app (a merge, a pull, or the CLI's
  `init`). The suggestion probe rides the same five-minute sweep as doctor instead
  of running once per project at startup.
- **AI profiles.** Settings → AI profiles lets you define what a worktrees-launched
  `claude` runs with — rules text, skills, MCP servers, model and settings — instead
  of your global `~/.claude` setup. Your normal terminal `claude` is untouched. A
  profile can be the global default or bound to one project (a project profile
  replaces the default; the two do not merge), and `WORKTREES_PROFILE=none` opts a
  launch out entirely. `worktrees open` from a terminal applies the same profile the
  app would — the CLI and the app share one resolver, so they cannot drift.
- **Each profile signs in once, on its own.** claude keys its saved sign-in to the
  config directory it is given, so a profile's first launch shows
  `Not logged in · Run /login` in the pane and stays signed in afterwards. worktrees
  never copies, reads or stores a credential to make this work. The profile list
  labels a never-launched profile "needs sign-in" so that first pane does not read
  as broken.
- **A skill store.** `worktrees skills add <dir|--git URL>` (and the same in the UI)
  installs Claude skills that profiles can enable. Because an enabled skill's
  description is loaded into every session before anything invokes it, installing one
  is closer to running someone else's prompt than to copying a file — so anything it
  asks for beyond reading files (`allowed-tools`, `hooks`, executables) is shown
  before it lands, git installs are pinned to the reviewed commit and refuse to
  install if the branch moved, and installing never runs anything from the source.
- **`worktrees mcp`** — an MCP server over stdio, so a Claude session can drive the
  worktree layer as tools instead of raw git. Read-only by default; worktree
  mutations are opt-in per profile and removing a worktree additionally requires
  explicit confirmation.
- **A "restart to apply" badge** on a live session whose profile has been edited
  since it started. It covers rules, model, MCP and settings — skill edits already
  reach a running session, so it does not claim them.

### Notes
- A profile controls the USER scope, not the project scope: a repo's own committed
  `.claude/settings.json`, `.mcp.json` and `CLAUDE.md` still load. Your global
  `~/.claude/CLAUDE.md` also still loads — a profile ADDS rules, it cannot suppress
  your own.
- Binding a profile to a repo that already has Claude conversations starts a fresh
  one, because history lives with the profile. Nothing is deleted; unbind and the
  old conversation comes back. The picker says so before you choose.
- If a profile cannot be prepared, the pane opens on a plain shell with the reason
  rather than launching claude without it — profiles are often restrictive, and
  "could not apply your profile" must never quietly mean "ran without it".

## [0.6.0] - 2026-08-01

### Added
- **Terminal tabs can be named.** Double-click a dock terminal tab to rename it
  (Enter saves, Esc cancels). Names belong to the place and survive quitting the
  app — the shell itself doesn't, so a named tab comes back as a fresh shell
  under the same name. Closing a tab forgets its name.
- **The nav's tree guide lines are optional.** Settings → Navigation → "Tree
  guide lines" turns the 1px rails connecting a project to its places off;
  indentation still carries the depth.
- **The app says when tmux is missing.** A banner above the top bar names the
  problem (`brew install tmux` on macOS) instead of leaving every place looking
  dead for no stated reason. Its `Re-check` button re-resolves the app's PATH,
  so a tmux installed while the app was open is picked up without a restart —
  the places refresh and sessions light up on the spot.

### Changed
- **Settings has categories now.** The sheet's single 12-section scroll became
  eight categories behind a category rail (Appearance, Terminal, Navigation,
  Commands, Behavior, Updates, Data & Logs, Shortcuts), and the sheet is wider.
  Every setting kept its behaviour — it just has an address now. The Updates
  category shows the update badge on its rail entry.
- The Logs tail pane in Settings is substantially taller — 200 lines of tail in
  a pane that showed eight of them was a reading slit, not a log view.
- **The installer now requires tmux.** A place *is* a tmux session, so
  installing without it produced a half-working tool. `install.sh` stops when
  tmux is absent: on macOS it offers to run `brew install tmux` (with a terminal
  attached to ask on), and on Linux it prints your distribution's exact install
  command. The CLI's own runtime behaviour is unchanged — `new` still degrades
  to `--no-tmux` if tmux disappears later.

### Fixed
- **Creating a place from the app now opens a single pane, like reopening one.**
  New places came up with the AI pane squeezed next to a spare shell, while
  reopening the same place gave Claude the full width — the same place looked
  different depending on how you got there. `new` learned `--no-spare` (which
  the app passes) so both paths agree. Dependencies are no longer auto-installed
  in that second pane; install them in the dock's Terminal tab — the command
  that would have run is printed for you. The CLI is unchanged: a bare
  `worktrees new` still splits the spare shell and installs deps there.
- The app's PATH fixup now always appends the standard install dirs
  (`~/.local/bin`, `~/bin`, `/opt/homebrew/bin`, `/usr/local/bin`) after the
  login shell's PATH, not only when the shell probe fails. A profile that never
  runs `brew shellenv` used to hide a brew-installed tmux from the GUI app.

## [0.5.0] - 2026-07-29

### Added
- **Right activity rail.** The dock's Files/Terminal picker now lives in its own
  rail on the right edge, mirroring the lens rail on the left. Clicking the
  active icon collapses the dock, so the toolbar's panel button is gone. The
  rail is always there — with no place selected its icons explain why they're
  unavailable rather than disappearing.
- **Branch switcher offers the repo's branches.** The status-bar field became a
  combobox: filter as you type, ↑/↓ and Enter to pick, and a `create <name> off
  <base>` row when what you typed doesn't exist yet. Remote-only branches are
  listed too (picking one tracks it). Typing a name and pressing Enter still
  works exactly as before.

### Changed
- **Dock shells no longer run under tmux.** Each Terminal tab is now a login
  shell this app owns directly, which means C-b reaches your shell instead of
  tmux, scrollback works with the mouse wheel instead of copy-mode, and the tab
  works even without tmux installed. Shells survive closing the dock, flipping
  tabs and switching places — their output is replayed when you come back — but
  they no longer survive quitting the app, and they are no longer
  `tmux attach`-able from a bare terminal. A shell that exits keeps its tab and
  offers a restart. The place's own session is unchanged: still tmux, still
  durable, still attachable. Leftover `~term` sessions from previous versions
  are cleaned up automatically.
- **Icons are a real set.** The chrome's Unicode glyphs (which picked a
  different font each, so weights and sizes disagreed) are now inline SVG. The
  two panel toggles say which panel they act on, and the dock toggle is no
  longer visually identical to the Places lens.

### Fixed
- The dock was capped at 680px, stranding most of a fullscreen window; its width
  now scales with the window. Side panels also re-fit when the window resizes, so
  a window restored smaller than the one your widths were saved from no longer
  overlaps its own toolbar — the dock steps aside and returns when there's room.
- The nav's ↑/↓ arrows now measure against the repo's base branch (origin/main)
  instead of the branch's own upstream. Updating a branch from main now reads as
  in sync; previously a pushed branch that merged main in showed hundreds
  "ahead" (the merged commits counted as unpushed), and branches with no
  upstream showed no arrows at all. The (main) row still works as a pull
  counter.
- Manual sort ("drag rows") actually drags now — Tauri's native drag-drop
  handler was intercepting HTML5 drag-and-drop in the app window.

## [0.4.0] - 2026-07-28

### Added
- Right dock (⌘J, or the panel button in a place's toolbar): a collapsible,
  resizable side panel with two tabs. **Files** browses the worktree as a lazy
  tree (honouring .gitignore) — click any file to view it, and edit it inline
  with ⌘S to save (binary and very large files stay read-only; "Editor" still
  opens your external editor). **Terminal** runs one or more live shells
  alongside Claude — add tabs with ＋ or ⌘⇧T, close them individually, or close
  them all. Each shell is its own tmux session, so they survive app restarts,
  are `tmux attach`-able from a bare terminal, and the open tabs are restored
  from the live sessions next time. The dock's width and last-used tab persist.
- **Per-project setup (`.worktrees.toml`).** A repo can now declare, in a
  committed file, the untracked things every worktree of it needs: which
  gitignored files to link (or copy) from the main checkout, a port map so two
  stacks can run side by side, and a docker-compose project name per place.
  Creating a worktree materializes all of it. This replaces the per-repo shell
  script most people were maintaining next to this tool, and closes the failure
  it kept causing: a credential added after a worktree existed was missing from
  it, silently — an Android build with no `google-services.json` gets no push
  token and reports no error.
  - `worktrees init` inspects a repo that has never heard of the tool and prints
    the config it would write, asking before writing anything. It flags
    credential files louder than `.env`s, because those are the ones that fail
    without saying so.
  - `worktrees relink` re-applies the file plan to worktrees that already exist,
    so adding an entry doesn't strand every place you already had.
  - `worktrees provision` allocates or repairs a port slot and writes
    `.worktree.env`.
  - `worktrees doctor` reports drift — a missing link, a dangling one, a real
    file shadowing a declared link, a port slot claimed twice, a place with no
    slot at all — and exits non-zero so CI can gate on it.
  - A file that already exists where a link belongs is **reported, never
    overwritten**. The tool it replaces silently destroyed it.
  - Nothing in the config is ever executed. A cloned repo can describe its
    structure; it cannot supply a command for the tool to run.
- **Project settings sheet in the app.** Open it from a project's context menu
  to see what the project declares, a health badge with the current findings,
  and Relink / Provision buttons. Places that have drifted get a ⚑ in the nav.
  A project that qualifies for a config but doesn't have one gets a dismissible
  suggestion.

### Changed
- The app now opens a place's tmux session as a single pane (Claude only) so it
  gets the full width — the scratch shell that used to share the split moved to
  the new dock's Terminal tab. The `worktrees` CLI is unchanged: `new` still
  splits a second pane for the dependency install.

## [0.3.2] - 2026-07-28

### Added
- Quick switcher: press ⌘K anywhere — even with the terminal focused — to
  fuzzy-jump to any place across every project. Type to filter (matches slug,
  branch, project, or note), arrow keys to move, Enter to jump, Esc to close;
  open it with no query and it lists your most recent places. Works with the
  nav collapsed, and each row shows the same working/needs-input dot as the nav.

## [0.3.1] - 2026-07-27

### Fixed
- The green "working" dot now tracks whether Claude is actually working, not
  whether you recently touched the session. It read a tmux activity timestamp
  that only moves when you attach or type — so it would fade a few seconds after
  you looked away even while Claude kept going, and it lit up for plain shell
  typing. It now reads Claude's own per-session state: a green blinking dot while
  Claude is working, a steady amber dot when it's waiting on you (a permission or
  dialog prompt), and nothing when it's idle — on the nav row, the project
  folder, and the Home screen alike.

## [0.3.0] - 2026-07-27

The settings release: a Settings pane that finally does what its controls say,
keyboard shortcuts that actually fire, and a batch of silent failures made loud.

### Added
- Nav: a Home entry at the top (app logo + one-click "Open a project" on the
  Home screen), folder icons on project rows, deeper nesting with more
  generous indent, and a right-aligned age column on place rows.
- System theme: pick WHICH light/dark pair "System (match macOS)" flips
  between (Settings → Theme → Light ↔ dark pair).
- Keyboard shortcuts: ⌘, opens Settings, ⌘1 jumps Home, ⌘2 / ⌘3 / ⌘4 jump to
  Places / Recent / Attention (keyboard selection always reveals the nav,
  never collapses it), and ⌘E opens the current selection in your editor. A
  read-only Shortcuts section in Settings lists every one of them.
- Settings → Commands: a "Resume Claude conversation on open" toggle. Turn it
  off and a single click opens a fresh session; right-click then offers "Open
  with resume" for the times you want to pick up where you left off. The
  effective AI command and resume argument are shown read-only, with a
  "Reveal config file" button — the config is shared with the CLI.
- Settings → Git: "Auto-fetch origin" (Off / 5 / 15 / 60 min) keeps every
  project's ahead/behind counts and the Attention lens fresh in the
  background, hardened so a credential prompt can never hang the app. A
  "Fetch origin" right-click verb on projects does the same on demand.
- Settings → Commands: an external terminal command with a `{session}` token
  (e.g. `ghostty -e tmux attach -t {session}`) adds "Open in terminal app" to
  a place's right-click menu — hidden until you configure it.
- Settings → Startup: "Restore last place on launch" reselects the place you
  left off on (it selects, nothing more — it never auto-starts a session).
- Settings → Version: a "Release notes" button reopens the notes sheet on
  demand, showing the full released history (not just the unseen slice), and a
  "Check for updates at launch" toggle.
- Settings → Logs: "Copy diagnostics" — one offline click assembles the app
  and CLI versions, the GUI's real resolved PATH, git/tmux locations, your
  effective AI config, and the last 200 log lines, ready to paste into a bug
  report.
- Settings → Data: reveal the settings file in Finder, and a two-click "Reset
  to defaults".
- Removing a worktree now offers "Confirm remove + branch" alongside the plain
  remove. Branch deletion uses git's merged-only guard (`git branch -d`), so
  it can never throw away unmerged work.

### Changed
- "What's new" renders formatted release notes — version headers, colored
  Added/Changed/Fixed tags, unwrapped bullets, `code` spans — instead of the
  raw changelog markdown.
- The green dot now means one thing: this session is working right now (tmux
  output within the last few seconds). Idle-but-open sessions show no dot; a
  busy place also badges its project's folder icon. The purple ✦ "AI session"
  glyph is gone — it was true for nearly every place, so it said nothing.
- The topbar remove action now reads "Remove worktree…" (was "Remove
  place…"), matching the right-click menu.

### Fixed
- The "Window default" size inputs did nothing — they were saved but never
  applied to a window. Removed.
- "Remove from workspace" could fire with zero confirmation from two different
  surfaces; both now arm on the first click and remove on the second. A
  subtler leak also let an armed "Confirm remove?" survive closing the ⋯ menu
  and then fire on a single click much later — that's fixed too.
- Copy actions failed silently — a stale clipboard with no signal that
  anything went wrong. Copy failures now surface.
- Enter could quietly do nothing. A tmux session that failed to start reported
  success everywhere (UI, exit code, and log alike), and a session running
  under a non-canonical name made a live place read as down so its terminal
  never mounted. Both are now loud and visible.
- Creating a worktree for a branch that already lived in another place could
  select a place that didn't exist, leaving the pane blank. The app now
  selects the place the engine actually used.
- Creation and switch failures used to show progress-looking lines instead of
  git's real complaint; git's actual reason now reaches the error banner and
  the log.
- Editor commands containing spaces or quotes (`open -a "Visual Studio Code"`)
  now work, and the new terminal command uses the same quoting.

## [0.2.4] - 2026-07-26

### Added
- Themes: the Settings theme picker grows from one option to seven — System
  (follows macOS light/dark), Tokyo Night (the existing default), Tokyo Night
  Day (the new light mode), Catppuccin Mocha, Catppuccin Latte, Nord, and
  Gruvbox Dark. Every palette uses the official upstream colors,
  contrast-verified for readability on every surface, and the embedded
  terminal recolors to match, including a full per-theme ANSI palette so
  colored terminal output stays legible on light backgrounds.

## [0.2.3] - 2026-07-26

### Fixed
- Embedded terminal drew `…`, `✻`, spinners, and other non-ASCII glyphs as
  underscores: the app attached tmux without a UTF-8 locale (GUI apps get
  launchd's bare environment), so tmux deemed the client non-UTF-8 and
  substituted `_` for every cell without an ACS line-drawing fallback. The
  embedded client now attaches with `tmux -u`, and the app sets a UTF-8
  `LANG` at startup when none is present (also covers the tmux server when
  the app is the first tmux invocation). Reopen embedded panes to pick it
  up — session content was never corrupted.

## [0.2.2] - 2026-07-26

### Added
- Nav preferences: show/hide the Active / Idle / Dormant tiers (Settings →
  Nav tiers), and a sort control in the nav header — Last used, A–Z, or
  Manual with drag-to-reorder (order remembered per project).
- Release notes on update: the first launch of a new version shows a
  "What's new" sheet with the changes since the version you were on
  (offline — the changelog ships inside the app).

### Fixed
- Embedded terminal artifacts: tmux sized windows to the SMALLEST attached
  client and only redrew that region, leaving stale "undeletable"
  characters when a bare `tmux attach` ran alongside the app. Sessions now
  use `window-size latest` + `aggressive-resize` (session-scoped; your
  global tmux config is untouched).
- CI actions bumped off deprecated Node 20.

## [0.2.1] - 2026-07-25

The self-updating release: from here on, updates are one click inside the app.

### Added
- **App self-update**: releases ship minisign-SIGNED app bundles + a
  `latest.json` updater manifest; Settings → Version gains "Update app → vX"
  (verify → download → swap → relaunch) next to the existing "Update CLI"
  button. The ⚙ badge covers both.
- The curl installer now OFFERS the desktop app on macOS (`worktrees.app` →
  /Applications, checksum-verified, quarantine-stripped on explicit opt-in;
  `WORKTREES_INSTALL_APP=1` / `--with-app` for non-interactive). `make
  install-app` builds + installs from a clone.
- Persistent app log (`~/Library/Logs/net.casadelvalle.worktrees/app.log`):
  every op result, terminal/updater failures, frontend errors, panics, and a
  startup line with version + resolved PATH. Settings → Logs opens the folder
  or tails it. "Check for updates" now acknowledges its result.

### Fixed
- GUI-launched apps inherited launchd's bare PATH (no homebrew → no tmux):
  every place looked dead in the installed .app. The real PATH is resolved
  from the login shell at startup.
- Nav tree: nesting is now DRAWN — per-level plumb-line rails with a lit
  ancestor trail on selection, the (main) row's dot in the project header's
  dot column, a recessed Dormant band, and a tighter indent (rails carry the
  structure, slugs keep their width).
- Settings → Logs "Open folder" was silently rejected by the capability
  system (opener:default has no open-path); now reveals app.log in Finder.

## [0.2.0] - 2026-07-25

The Rust release: one compiled engine behind both the CLI and a desktop app.

### Added
- `worktrees ls --json` (also `WORKTREES_JSON=1 worktrees ls`): a machine-readable
  snapshot (`schema_version` 1) of every place — the main checkout first, then each
  worktree with live derived state (branch/detached, dirty + file count, ahead/behind
  vs upstream, tmux session up/down, last commit, install command, Claude-session
  presence, and a computed `lifecycle_effective`). The human `ls` table is unchanged.
- `worktrees close <name> [name...]` — end a place's tmux session; the worktree,
  branch, and declared state all stay (the inverse of `open`). Resolves a branch to
  its holder worktree, closes adopted sessions (a pane cwd'd in the worktree under
  another name), and `close main` targets the main checkout — unless a worktree is
  literally named `main` (the directory wins).
- **Desktop app** (Tauri, links the engine in-process): multi-project nav tree with
  lifecycle groups (Pinned/Active/Idle + a Dormant fold), embedded tmux terminals
  (attach-not-own), create/switch/close/remove and lifecycle/pin/note from the UI,
  right-click context menus (Enter, open fresh, close session, copy attach command,
  new worktree off a branch, open on GitHub, reveal in Finder, open in editor…),
  a collapsible rail-only nav (⌘B), persisted Settings (UI font scale, terminal font,
  density, nav width, editor command), auto-resume of an existing Claude conversation
  on open, live refresh, and an in-app update check (Settings → Version) that can
  update the installed CLI via the pinned-tag installer.
- Declared lifecycle store (`.worktrees.places.json`, schema-versioned plain JSON):
  saved/archived/abandoned/closed + pin + note, reconciled with live tmux state.

### Changed
- The CLI is now a compiled Rust binary (`crates/worktrees-cli`), behavior-identical
  to the original bash version (gated by the same bats suite — now 137 cases — plus
  real-tmux smokes). `install.sh` fetches a prebuilt binary per platform (macOS/Linux,
  x86_64/arm64) or builds from source with `cargo`; `make install` compiles +
  symlinks the release binary; `bin/worktrees` is a shim that runs the built binary
  from a clone. The legacy bash implementation was retired at full parity.
- Snapshot reads are parallel (bounded fan-out per place and per project) — big
  monorepos with many worktrees list in ~max(latency) instead of sum.
- tmux kill targeting is exact-match only (`-t =name`); the prefix-match fallback
  that could hit a sibling session (`api` → `api-fix`) is gone.

### Fixed
- Claude project-dir detection now mangles every non-alphanumeric character
  (matching Claude Code), so resume detection works for paths with `_` etc.

## [0.1.0] - 2026-07-12

### Added
- Initial release: `new`/`co`, `switch`, `open`, `ls`, `rm` — git-worktree-per-branch
  workflow with a tmux session per worktree (pane 0 AI CLI, pane 1 dependency install + shell).
- Configurable AI pane: `--ai` flag, `$WORKTREES_AI_CMD` (deprecated alias
  `$WORKTREES_CLAUDE_CMD`), `ai_cmd` in `~/.config/worktrees/config`; default `claude`,
  `none` for a plain shell. Resume arg configurable (`$WORKTREES_AI_RESUME_ARG` /
  `ai_resume_arg`, default `-r`).
- Namespace prefix: `$WORKTREES_PREFIX` > `.worktree-prefix` file > user config > repo dir name.
- Runs on stock macOS bash 3.2 and Linux; git ≥ 2.23; tmux optional (≥ 1.9) — `new`
  degrades to `--no-tmux`, `open` requires it.
- `install.sh` curl installer (release-pinned, checksum-verified, `~/.local/bin`) and
  `make install` (symlink) for clones.

### Provenance
Extracted from the Casa del Valle monorepo's `scripts/worktrees.sh`, minus its
docker/stack-mode and AI-question features.
