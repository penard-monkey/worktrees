# worktrees — working notes for Claude

One git worktree per branch, one tmux session per worktree. A worktree is a
durable PLACE; a branch is work that flows through it. Full design docs:
DESIGN.md (app), MIGRATION.md (bash→Rust history).

## Architecture

- **One engine**: `crates/worktrees-core`. The CLI (`crates/worktrees-cli`,
  binary `worktrees`) and the Tauri app (`app/src-tauri`) BOTH use it — the app
  links it in-process (no subprocess). The legacy bash engine is retired.
- git/tmux are **shelled out** on purpose (faithful port; keeps the bats
  fake-shim harness intercepting the compiled binary). Don't switch to libs.
- State split: **derived** (live git/tmux, recomputed) vs **declared**
  (`.worktrees.places.json` — lifecycle/pin/note, plain JSON, no DB).
  Terminals ATTACH to tmux, never own shells. Session name =
  `<prefix>-<slug>` with `.` → `-`.
- One version source: workspace `Cargo.toml`. The app crate + tauri.conf
  inherit it; `test/misc.bats` asserts the binary against it.
- **Agents (since #183): project → place = a WORKSTREAM → agent = a claude
  session launched against a brief.** The brief is `.planning/brief.md` in
  the worktree (`ops::BRIEF_PATH`); claude opens on the fixed
  `ops::BRIEF_OPENER` and NEVER receives the brief via argv. Pane-0 claude
  gets `--name <full tmux session>` so claude's own cross-session messaging
  (`ListAgents`/`SendMessage`/`notify_when_idle`) addresses it — that is the
  bus, we do not build one. `worktrees_core::agent` reads
  `~/.claude/sessions/<pid>.json` for BOTH the nav dots and MCP `place_status`;
  keep them on that one reader. The orchestrator is `(main)`'s claude; one
  user-scope `claude mcp add -s user worktrees -- worktrees mcp --mutations`
  serves every repo (cwd discovery), and a PROFILE needs
  `worktrees_mcp_mutations` or its injected server is read-only.

**`~/.claude.json` is claude's live state — READ it, never write it.** It is the
user-scope `mcpServers` home (`mcpsetup.rs`), a few hundred KB of onboarding
flags, caches and per-project history that every running session rewrites whole.
A read-modify-write from here loses whatever a live session wrote in between,
silently, and it is someone else's data. The app's setup wizard shells out to
`claude mcp add -s user` for that reason — same rule as `ui-state.json`'s
whole-blob owner, one level out. Its twin: **`claude mcp list`/`get` cannot tell
you whether a server is installed.** It health-checks by LAUNCHING each server in
the current directory, so `worktrees mcp` — which legitimately serves nothing
outside a repo — reports `✘ CONNECTION_CLOSED` from `/tmp` and `✔ Connected` from
a checkout, for the same correct install. It also costs ~1.1s a call. Detection
parses the file; only the WRITE goes through claude.

**A user-scope MCP server is launched by every session, including the ones with
no repo.** `worktrees mcp` used to exit at `Project::discover`, which was fine
while it was added per-repo and became a red ✘ in `/mcp` for every session
started in a home or scratch directory the moment the app began installing it
globally. It now handshakes, advertises zero tools and says why; `test/mcp.bats`
pins that, and the bats suite caught the change by asserting the old contract.

## Gates (run before any PR)

```sh
cargo build --release -p worktrees-cli   # FIRST — bin/worktrees shim prefers
                                         # target/release; a stale release binary
                                         # makes bats fail mysteriously
make test           # bats suite vs the Rust binary (fake git/tmux shims)
make lint           # shellcheck + bash-3.2 gate (shim + install.sh)
cargo test -p worktrees-core
cargo test -p worktrees-cli   # MCP protocol unit tests live here
cargo test -p app --lib      # `check`/`build` do NOT compile `mod tests`
cd app && ./node_modules/.bin/tsc --noEmit && cargo check -p app
```

CI mirrors these + builds the app crate on both OSes. Squash-merge PRs.

**Docs-only PRs skip CI by design.** `ci.yml` `paths-ignore` covers `docs/**`,
`ROADMAP.md`, `CLAUDE.md`, `DESIGN.md`, `MIGRATION.md`, `README.md` and
`.claude/**` — a close-out archive PR shows ZERO checks, which is correct, not
a hung run, and nothing blocks the merge (main has no required status checks).
The skip applies only when EVERY changed file matches the list; mix in one code
file and the full suite runs. `CHANGELOG.md` is deliberately NOT on the list —
it ships inside the app binary via `include_str!`.

**A CHANGELOG entry can rebase CLEANLY into the wrong release.** An entry
written under `## [Unreleased]` while a release is cut underneath you applies
with no conflict into the now-published `## [x.y.z]` section — the surrounding
context still matches, so git has nothing to complain about — and the change is
then documented as part of a release it is not in, which release.yml has already
published and the app already ships. #309 did exactly this across v0.26.0. A
rebase exit code of 0 says the patch applied, not that it applied where you
meant; after any rebase that crosses a release boundary, look at which section
the entry actually landed in.

Its twin, one level down: **resolving a CHANGELOG conflict by keeping both
sides gives you two `### Fixed` blocks under one `## [Unreleased]`.** Both sides
legitimately opened the same subsection, so "keep both" is right for the ITEMS
and wrong for the HEADER. Nothing complains — the file renders, the rebase exits
0, and the second header is below where the eye stops. `grep -c '^### Fixed'`
between the `[Unreleased]` and the release under it, every time, the same way
the duplicate `## [Unreleased]` hazard in ROADMAP has to be checked by counting.

**A stale release binary does not only make bats FAIL — it can make it PASS.**
The note above says "fail mysteriously", which is the friendlier half. Edit a
crate *after* the release build and `make test` happily green-lights the OLD
binary; the same goes for the `ls --json` diff below, which then compares the
shipped binary against another copy of itself. Both read `target/release`, and
neither notices it predates the change. Rebuild, then confirm before believing a
result: `[ target/release/worktrees -nt crates/worktrees-core/src/store.rs ]`.

**Report cargo with a grep, not a tail.** `cargo test -p worktrees-core | tail -4`
prints `running 0 tests / ok. 0 passed` — that is the **Doc-tests** block, and
the real `running 207 tests` is above the cut. A tail-truncated run reads exactly
like a crate with no tests. Use `grep -E "^running|^test result"`. Related shell
trap when scripting gates: `grep -c` **exits non-zero on a count of 0**, so
`... | grep -c "^not ok" && next-gate` silently skips everything after it — a
gate that never ran looks identical to a gate with no output.
Its twin: **`… | grep -q X` under `set -o pipefail` reports FAILURE on a
MATCH.** `-q` exits at the first hit, the upstream stage takes SIGPIPE, and
pipefail promotes that to the pipeline's status — so "found" reads as "not
found". Two probe results were misread as product bugs before this was spotted.
Write to a file, then grep the file.

**A pipeline's exit status is the LAST stage's.** `make test | tail -15` exits 0
because `tail` did, and a mid-stream `not ok` scrolls off a 15-line window — so
the run reads green whatever bats said. Redirect, then check both halves:
`make test > log 2>&1; echo $?` plus `grep -cE '^not ok' log`. Same family as the
grep traps above, and the reason to run gates with `make -C <repo-root>`: the
Bash tool's cwd persists between calls, so an earlier `cd app` turns a later
`make test` into "No rule to make target `test`" — a failure that looks like the
change broke the build.

**Two `gh` commands lie about CI, in opposite directions.**
`gh pr checks <n> --watch` run in the gap between a push and the run
registering prints "no checks reported on the 'x' branch" and exits **0** —
identical to a pass, and it is what you get every time you push and immediately
watch. And `gh` reports `conclusion: ""` for an in-progress job, never `null`,
so a hand-rolled `--jq 'select(.conclusion != null)'` rollup counts every
RUNNING job as a failure. Both fired within a minute of each other on #304, one
claiming green and one claiming nine failures, neither true. Watch the run, not
the PR view: `gh run watch <run-id> --exit-status`, then read raw conclusions.

**A CONFLICTED PR gets no CI at all, and it does not look like a conflict.**
`on: pull_request` builds `refs/pull/N/merge`, which GitHub cannot create while
the branch conflicts with main — so `gh run list --branch <b>` comes back EMPTY
while every other branch runs normally, which reads as a broken workflow or a
queue that never started. #310 sat like that until a rebase, then its run
appeared within seconds. Check `gh pr view <n> --json mergeStateStatus` (DIRTY)
before debugging the workflow. Note `mergeStateStatus` also reads `UNKNOWN` for
a few seconds after any push while GitHub recomputes it.

**Killing a bats run writes a `not ok` into its log.** A `pkill` (or a branch
switch under a running suite) fails the in-flight test inside `common_setup`
and prints `bats warning: Executed N instead of expected 340 tests`. A later
grep of that log reads as a real regression. Check the plan line and the count
before believing a single failure in a log you did not watch finish.

**A throwaway git script needs a guard, because `git -C ""` means HERE.** A
probe let a repo path come back empty and aimed `branch -M main` and
`push origin main` at the live worktree; git refused both (the worktree guard on
the rename, non-fast-forward on the push) and nothing was lost, but neither
refusal was the script's doing. Route every call through one helper that
hard-exits unless the directory is non-empty and under `$TMP`. The cause is
worth knowing too: **bash expands every assignment on a `local a=1 b="$a"` line
before binding any of them**, so `b` is empty — and under `set -u` the function
dies mid-way inside a command substitution, leaving the caller with "".

**"Measured on this platform" has to say WHICH platform.** A test asserted that
binding `0.0.0.0:<our port>` succeeds while a loopback-only listener holds it,
and its comment said it had been measured — on macOS. Linux refuses a wildcard
bind over ANY holder of the port, so it went red in CI for a server that was
bound exactly right. The mirror witness fails the other way round. Measured, all
four, on both: `0.0.0.0:P` tells them apart on macOS and is `EADDRINUSE` either
way on Linux; `<lan ip>:P` succeeds either way on macOS and tells them apart on
Linux. **No bind means the same thing twice** — and the rule being guarded never
mentioned binds, it said "reachable from the LAN is a data leak", so the witness
is a CONNECT, which answers identically on both. Two corollaries: an assertion
about OS behaviour needs the second platform measured before it is written down
(`docker run --rm -v "$PWD":/w node:22-alpine` is enough), and the loopback
connect that precedes the LAN one is load-bearing, because without it a DEAD
server passes — every connect fails, including the one that must.

**A workflow that fails to PARSE is invisible.** GitHub answers an unparsable
workflow with a startup-failure run carrying **zero jobs and no annotations**;
it never appears in `gh pr checks`, so a PR shows all-green beside it. A pasted
block that landed at line 1 of `release.yml` — above `name:`/`on:` — left it
broken for a whole branch, and since it runs only on `v*` tags the first
exercise would have been a release. `gh run list --commit <sha>` shows the runs
`gh pr checks` does not. The twin trap: that same paste was meant to REPLACE a
step and instead duplicated it, leaving an assertion for a binary the branch
deleted, so valid YAML would only have moved the failure later.

**A process-global `static` makes unit tests order-dependent, and the suite
hides it.** `viewer::ISSUED` is per-launch by design, so a test asserting an
exact route segment passed only while no other test had claimed it — and the
full suite passed on an accident of ALPHABETICAL order, microseconds apart. A
filtered run, a new test sorting earlier, or a slower runner turns it red with a
message that reads like a `place_key` regression. Check with
`cargo test -p app --lib -- --test-threads=1 <a> <b>`; the fix is a `_in` seam
so pure tests own the map they assert against, leaving the global to the tests
that are actually about it.

**A recovery test that hand-clears the blocker asserts the bug away.** The
failed-derive test set `fingerprint = 0` before checking that a broken place
recovers — which was precisely the comparison preventing recovery, so a place
broken by a re-open stayed `503` until the button was pressed again. If a test
pokes state to reach the thing it is asserting, ask what the poke is standing in
for and whether production ever does it.

**A drift check that takes the FIRST match can be silently repointed.**
`docsviewer-check.mjs` found the emitter by the first `format!("…click…")` in
`derive.rs`; a later one added above it would become the thing asserted while
the real emitter drifted from the strip rule, green throughout. Slice off `mod
tests` (its `format!`s are fixtures) and fail when more than one line claims to
be the emitter, rather than guessing. Same family as the mirror-drift rule
below: a guard that cannot be wrong is a guard that cannot fail.

**`symlink_metadata` refuses to follow only the LAST component.** Every
intermediate directory in the path is resolved exactly as `metadata` would
resolve it, so `symlink_metadata("a/b/c.md")` reads through an `a -> elsewhere`
and reports a perfectly ordinary file outside the tree. A walk that lstats each
entry as it descends is safe; a check that lstats a path built from a CONSTANT
(`docs::walk`'s `.planning/brief.md`) is not, and the two sat four lines apart
in `docs.rs` — the walker refused the link, the by-name read went through it,
and the half that leaked was the half that always listed. The tell is that a
partial guard looks like a working one: the rows the walker refused are missing,
exactly as they would be if it all worked. `resolve_rel` and every
`root.join(rel)` under `[docs]` still have this (ROADMAP); the fix shape is
`docserver::safe_under`'s — canonicalise and require the result under the
canonical root.

**A test can pass because its FIXTURE omits the thing.** The regression test for
that symlink wrote a directory behind the link and asserted the plan rows were
absent — and passed, because nothing had put a `brief.md` in it. It was shown to
fail first, against the right code, for the wrong reason. Sibling of the
"recovery test that hand-clears the blocker" note below: when a test reaches its
assertion, ask what is NOT in the fixture as well as what was poked into it.

**A new test must be shown to FAIL first.** ROADMAP's zombie-children item
records a regression test that passed identically with and without its fix.
Break the thing under test (drop the `skip_serializing_if`, restore the old
line), watch it go red, then restore. Two tests this repo now relies on were
confirmed this way.

**A message's REMEDY is a claim, and it needs following, not proofreading.**
B3 refused a declared symlink source with "link the real file, or point the
config at it" — and BOTH are impossible in the only case that reaches the
check, because `[[file]].path` is repo-relative (`init.rs` refuses absolute
paths, `~` and `$`) and the target is outside the repo by definition or B3
would not have fired. A test asserting the string would have passed forever.
Check advice by DOING it end-to-end through the release binary in a scratch
repo; here that turned up an escape hatch that is not in the config at all —
`git worktree add` checks out a tracked `120000` blob with no Layer B check, so
committing a symlink is free exactly where declaring it is refused (this repo's
own `_tmp` has always worked that way). Two corollaries. **A feature hand-
patched wherever it fails looks like a feature that works**: four of eight
worktrees in the affected project had `_tmp` linked by hand, which disguised a
declaration that had NEVER once succeeded in six days — mtimes scattered across
three days is what gave it away. And **a tracked symlink is branch-dependent
where a hand-made one is not**: checking out a branch that predates the
tracking DELETES the link from that tree (git removes a tracked path absent
from the target commit; ignore rules do not protect it), idle `-next` bases
included.

**Before parsing a format a tool's TEMPLATE promises, count the real files.**
The planning-with-files template ships `### Phase N` + `**Status:**` lines;
of fifteen real `task_plan.md` files on this machine, two had them, nine had
checkboxes, most had a `## Goal`, several were free prose. A parser written
to the template would have rendered an empty Plan tab on thirteen places
while every fixture-driven test passed. `worktrees_core::plan` mirrors the
skill's own Stop-hook greps (`check-complete.sh`) instead and always shows
the rendered file under the summary, so the summary being wrong hides
nothing. The survey took ten minutes; do it before the parser, not after.

**Consolidating a git invocation? Diff `ls --json` against the SHIPPED binary.**
Folding three git calls into one `status --porcelain=v2` silently changed
`upstream` for one worktree — v2 reports the CONFIGURED upstream, `rev-parse
@{u}` reported only one that RESOLVES. Neither bats nor the 205 unit tests
covered it; only the output diff did. `~/.local/bin/worktrees` is the last
release, so it is the reference.

**Counting subprocesses: shim the CLI, never the app.** Wrap `git`/`tmux`/
`stat`/`date` in counting wrappers on PATH and run `worktrees ls --json` — the
identical `snapshot()` path (`spawn-count.sh` in the worktree's cache dir).
`ps`-sampling undercounts by ~3× and misses sub-millisecond spawns entirely. The
shim can't measure the APP: `fixup_gui_path()` prepends the login-shell PATH at
startup and keeps the inherited one only as a trailing fallback, so the real git
wins.

A FRESH worktree needs two bootstraps first, and both fail confusingly:
`git submodule update --init --recursive` (without it `make test` dies with a
bare "No such file or directory" naming the bats binary, not the submodule),
and `pnpm install` in `app/` under Node >= 22.13 (`nvm use 22.23.2`).

**AI profiles have a manual gate too.** Everything claude-side (does the config
swap apply, does session adoption still see `claude`, does auto-resume resume)
is invisible to the bats suite — there is no fake claude. Re-run
`docs/ai-profiles-manual-checks.md` whenever the `claude` binary is upgraded.

## Tauri app — hard-won rules

- **Commands must be `async fn`** — sync handlers run on the main thread and
  freeze the UI for every git/tmux shell-out.
- **GUI launches get launchd's bare PATH** (no homebrew → no tmux).
  `fixup_gui_path()` in lib.rs resolves the login-shell PATH at startup —
  don't add subprocess calls that assume PATH before it runs.
- **xterm STOPS PROPAGATION of every keydown it handles, and a WKWebView click
  does not move focus.** So a `window` bubble-phase key listener never hears a
  key pressed while the terminal has focus — Escape on a dialog opened from a
  menu went to tmux and the dialog stayed, for every dialog that did not focus
  an input of its own. Modal Escape lives in `useEscape.ts`: one CAPTURE
  listener plus a stack ordered by activation, so the top surface takes the
  key and the pty never sees it. Any new dialog, sheet, menu or popover calls
  `useEscape`; do not add another `addEventListener("keydown")`. The Chrome
  extension's synthetic key press did NOT reach the page here — verify with a
  `KeyboardEvent` dispatched at the focused element, and read `escapeDepth()`.
- **Components defined inside App() remount every render** (new identity) —
  anything with local state or input focus goes at module scope with props.
- The mock harness (`pnpm dev:mock`, `app/src/mock/install.ts`) must track
  every command in lib.rs — it's how the UI is developed/driven headlessly
  (Playwright). Port 1420 = `tauri dev`; run the harness on another port.
- **The mock answers INSTANTLY, and that hides a whole class of bug.** Its
  invokes resolve in a microtask, so two `list_workspace` sweeps never overlap
  and there is no gap between "write done" and "refresh returned" — the real one
  is a git fan-out over every project (0.28s for one project with nine
  worktrees, seconds across a workspace). Three v0.12.x bugs passed gates,
  review and harness checks and were then found by running the real app; all
  three lived in timing the harness cannot express. Before releasing anything
  touching refresh, optimistic UI or per-place state, run it for real:
  `app/scripts/sandbox.sh --app` (isolated identifier + tmux prefix, so it
  cannot collide with your installed app — and NOTE bare `sandbox.sh` is the
  CLI sandbox meant to be `eval`'d, it does not launch the app). Two tools now
  cover the shapes already hit: `?slowlist=<ms>` makes the mock's
  `list_workspace` slow, and `app/scripts/race-check.mjs` drives the real
  `refresh`/`commitWs`/`patchDeclared`/`mutate` source under controlled
  promise-resolution orders (`node app/scripts/race-check.mjs [App.tsx]`, exits
  non-zero on failure — it fails on v0.12.0, which is how it earns trust).
- **A harness tab that is not FRONTMOST never polls, and that reads as a broken
  render.** `useUsage` (and every poll gated the same way) skips both the
  immediate pull and the interval when `document.visibilityState` is `hidden` —
  correct behaviour, and it means a tab driven while your terminal has focus
  shows no usage meter at all, no matter how right the code is. Force it with
  `window.dispatchEvent(new Event("focus"))`: the hook's `focus` listener is
  registered unconditionally, so the pull lands without faking visibility.
  Two more from driving that widget in an unfocused tab: `element.focus()` fires
  no `focus` event there, and a synthetic `pointerenter` does not reach React's
  `onPointerEnter` — but `element.click()` does reach `onClick`, which is why
  pinning the panel works when hovering it does not.
- **HMR is dead inside `.worktrees/`** — chokidar ignores dot-directories, so
  vite never sees the edit and keeps serving the PRE-edit file. A reload and a
  `touch` both "work" and change nothing; a real fix looks like it failed.
  Restart with `--force` after every source edit, and when a change seems not to
  apply, diff what the server serves (`curl -s localhost:PORT/src/App.css`)
  against disk before debugging the change itself.
  **Killing the harness needs a CONTENT check, not a port check.** Two vite
  instances can hold the same port — kill one and `lsof -ti:PORT` still answers,
  so "port free" reads as true while a survivor serves the PRE-edit file and a
  test "verifies" the old code. Use `lsof -ti:PORT -sTCP:LISTEN` (plain `-ti`
  also returns Chrome's network-service helpers), then grep the served file for
  something the edit added — for CODE, never a comment: esbuild strips comments,
  so `curl … | grep "the note I just wrote"` reports 0 on a server that is
  serving the new file perfectly.
  **And keep the harness OUT of a foreground shell's process group.** A vite
  started with `nohup … &` inside a tool call is SIGTERM'd (exit 143) when a
  later call's group is cleaned up. Its death mid-session is not quiet: HMR
  drops, Fast Refresh resets App's state, and the app jumps to the Home screen —
  which reads exactly like whatever chord you just pressed having cleared the
  selection. A whole debugging detour came from that. Launch it as a real
  background task and check `lsof` before believing any harness result.
- **xterm's search addon fails only once you SEARCH.** Its decorations need
  `allowProposedApi: true` on the `Terminal` — without it `registerDecoration`
  throws on the first ⌘F, from inside an effect, taking the pane down with it;
  the terminal looks perfect until then. It also CACHES the last search and
  re-highlights only when the term or case/regex/wholeWord changed —
  `_didOptionsChange` never looks at `decorations` — so a theme switch needs a
  `clearDecorations()` first or the matches keep the old theme's hex. Load the
  addon after `term.open(host)`, and route its calls through a guard: losing a
  search is survivable, losing the terminal is not.
- **An xterm host is a RATCHET without `min-width: 0`.** `.term-host` is a row
  flex item, so its automatic minimum size is its min-content width — and xterm
  writes an explicit `width: <cols × cell>px` onto `.xterm-screen`, which makes
  that floor the grid it is painting *right now*. The host then only ever grows:
  open the dock and `.main` narrows while the host keeps its old width,
  overhanging the dock by 354px, tmux still painting columns that are now behind
  it. Nothing self-corrects, because `TerminalPane`'s ResizeObserver watches that
  same box — no shrink, no `fit()`, no `term_resize`. Invisible to every suite:
  the size is correct when written and only wrong once something else takes
  width away. `app/scripts/termfit-check.mjs` guards the declaration; measure a
  seam like this with `getBoundingClientRect` in the harness (`.term-host`'s
  right edge vs `.dock`'s left), never by eye — the clipping looks like a font
  or repaint bug.
- **The fit addon reads the host's BORDER box and subtracts `.xterm`'s padding —
  two different elements.** `proposeDimensions()` takes its available size from
  `getComputedStyle(term.element.parentElement)` and its padding from
  `getComputedStyle(term.element)`, and the app's global
  `* { box-sizing: border-box }` makes the first of those *include* the host's
  own `padding: var(--s2)`. Nothing ever takes it off: the grid is sized for
  16px it does not have, and the last ~2 columns land under
  `.xterm-viewport`'s 15px scrollbar gutter, which paints over them and slices
  the final glyph down the middle. `box-sizing: content-box` on `.term-host` is
  what makes the addon's arithmetic true, on both axes; the layout does not
  move, but ONLY because the rule sets no width/height/basis length for
  box-sizing to reinterpret (flex-basis 0% floors at the padding either way — a
  `max-height` would not). This survived the `min-width: 0` fix above and reads
  exactly like it (a cut glyph at the right edge), so check WHICH box is wrong
  before assuming a regression: derive
  `floor((borderBox − scrollbar) / cell)` and `floor((content − scrollbar) /
  cell)` and see which one the live `cols` matches. Do not "fix" it by hiding
  the scrollbar — xterm caches `scrollBarWidth` in the Viewport constructor as
  `offsetWidth − scrollArea.offsetWidth || 15`, so a 0-width gutter still costs
  15px.
- **Every DISTINCT grid handed to the pty is a SIGWINCH, and the shell reprints
  its prompt for each.** `TerminalPane`'s ResizeObserver used to `tx.resize()`
  per callback: a 240px drag sent 120 `term_resize` invokes carrying 17 distinct
  sizes, and both panes filled with stacked truncated prompts + full-width rules
  — the "lines" a resize left behind. `RESIZE_SETTLE_MS` coalesces a gesture into
  one resize; `fit()` waits WITH it, because refitting per frame while the pty
  holds the old grid has tmux painting a screen that no longer matches the canvas
  (garbled for the whole drag, versus a strip of host background that closes when
  you let go). Two traps in the coalescing itself, both locked down by
  `app/scripts/termresize-check.mjs` (which evaluates the real `useTerm` under
  stubs on a VIRTUAL clock — on real timers a scheduler stall mid-drag
  fails it while blaming the component): the baseline it
  dedups against must be seeded from **the grid passed to `open`**, captured
  before the await — read it back afterwards and you record a size the pty never
  got, masking the resize the transport dropped while the attach was in flight
  (both transports gate `resize` on it; the DROP pre-dates the coalescing, which
  only removed the accident that hid it — an unconditional re-send on the next
  observer callback) — and the `termVersion` effect resizes on
  its own, so it must INVALIDATE that baseline rather than write to it. An
  over-claiming baseline suppresses a resize the pty needs; a cleared one costs
  at most one redundant send.
- **`.term-host` paints `--term-bg`, not `--bg-abyss`.** The grid is whole cells,
  so the host is always bigger than the terminal by `content % cell` — 0..cell−1
  px per axis, changing with every resize. That strip is the host's background,
  and the two tokens differ in tokyo-day and catppuccin-latte, where it reads as
  a stray line along the bottom edge that thickens and thins as the window moves.
  The dark themes only hid it by having the tokens agree.
- **The terminal's glyph widths must MATCH TMUX, and the Node probe lies.**
  tmux (utf8proc) lays out emoji as 2 cells; xterm 5.5's default Unicode 6
  tables said 1, and every tmux partial repaint interleaved one column off —
  Claude's spinner turned that into permanently shredded lines. The graphemes
  addon (`activeVersion = "15-graphemes"`) aligns them, VS16 (⚠️) included,
  which Unicode11Addon would NOT. Two traps: probing the addon's widths under
  Node reports astral emoji as narrow (pooled-Buffer bug in its `_dec()`;
  `delete globalThis.Buffer` first), and `tmux send-keys` mangles pasted
  VS16/ZWJ — measure with UTF-8 byte escapes and `#{cursor_x}`. Clean
  `capture-pane -p` + garbled pane = width mismatch, nothing else.
  `dnd.ts::predictTier` reimplements `store::reconcile` so a drag can predict
  which tier a row will land in; `app/scripts/dnd-check.mjs` parses `store.rs`
  and `lib.rs` for `IDLE_WINDOW_SECS`, the sticky-label set and
  `LIFECYCLE_LABELS` and fails if the mirror drifts. Without that, a change on
  the Rust side leaves a frontend that is confidently wrong and passes every
  test — the mirror's own unit tests keep testing the OLD rule. Same shape as
  the version-vs-binary assertion in `test/misc.bats`.
- **`remove_place`'s `force` is TWO permissions wearing one flag.** `ops.rs:1014`
  reads it as "remove a dirty tree"; `ops.rs:1055` reads the same bool to pick
  `git branch -D` over `-d`. So force+`--branch` force-deletes an UNMERGED
  branch — the only combination in the remove path that can destroy commits. The
  inline arm this repo shipped for a year was immune only because it hardcoded
  `force: false`, and the docstring's "del_branch is safe by construction" was
  true *of that call site*, not of the command. Any UI that exposes force must
  re-word what it says about the branch; `RemoveDialog` does it with
  `forceDeletesBranch`. Nothing in bats or the mock catches this — the mock
  models no branch objects (`install.ts`: "delBranch is state-invisible here").
- **A menu's clamp must re-run when the menu RESIZES, not when the cursor
  moves.** `CtxMenu` clamped in a `useLayoutEffect` keyed `[x, y]` — coords that
  are frozen for the menu's whole life — so a menu that grew after opening (an
  item arming into two) kept the `top` computed for its old height and pushed
  its new last row off the bottom edge, unreachable and with no scrollbar to
  admit it. A `ResizeObserver` covers callers that do not exist yet; `.ctxmenu`
  carries `max-height`/`overflow-y` as the belt for a menu taller than the
  window. Measure with `offsetWidth/Height`, NOT `getBoundingClientRect()`,
  which measures through the `pop` keyframe's `scale(0.98)` and reports a box 2%
  small on the first frame. `app/scripts/ctxmenu-check.mjs` evaluates the real
  CtxMenu source under DOM stubs and fails on the pre-fix version (same
  slice-the-real-source shape as `race-check.mjs`).
- **The mock harness is CHROME; the app is WKWebView, and WebKit sizes a
  `<button>` on its own rules.** The usage meter shipped with bars and
  percentages and NO labels in the real app after every gate and the harness
  passed at 1280px. Two WebKit-only gaps, both inside a `<button>`: a button
  that is itself a flex container is shrink-wrapped WITHOUT its
  `overflow: hidden` children, so those (the only shrinkable items) go to 0px;
  and an empty span sized only by `flex-basis` contributes nothing to the
  button's intrinsic width — three 40px bars came out as 120px missing. Make
  an inner span the flex container, pin short labels with `flex: none`, and
  give a basis-sized bar a real `width` too. Do not reason about it — measure
  it: headless Playwright WebKit against the mock (`npm i playwright &&
  playwright install webkit` in a scratch dir), `addStyleTag` a candidate
  rule, re-measure, and edit only when the number moves (201px → 321px, which
  is Chrome's number). Anything sized by flex inside a button needs that probe.
- **grep the sheet before naming a class, and never autofocus inside an
  `overflow: hidden` ancestor.** `.seg` already meant the Settings segmented
  control (border, hidden overflow, `fit-content`), and reusing it for the
  compact usage segments made bordered pills that could shrink to nothing.
  Separately, a popover positioned inside `.identity` was clipped to the
  header's 17px and its `autoFocus` SCROLLED that hidden-overflow box 34px,
  pushing the place name off the header: fixed positioning from the anchor's
  rect plus `focus({ preventScroll: true })`. Both read fine as rects; only
  `elementFromPoint` and `scrollTop` told the truth.
- **A synthetic pointer drag bypasses hit-testing on the way IN.** Dispatching
  `pointerdown` on a row starts a drag even when a full-screen overlay
  (`.menu-catch`) is up — which a real press could never do, because it would
  hit the overlay — and then `elementFromPoint` answers with the overlay for
  the whole drag, so every drop silently resolves nothing. Harness-only, but it
  reads exactly like a broken drop target. `body.dragging .menu-catch {
  pointer-events: none }` neutralises it.
- **Check what a drag test ASKS for before believing it found a bug.** Two
  "project reorder is broken" reports in one session were a drop onto a
  position the row already occupied (a legitimate no-op) and a drop onto the
  scroller's padding. The second was real — `closest('[data-project-root]')` is
  null over `.nav-scroll`'s own padding, and the strip above the first project
  is exactly where you aim to make one first — but it was found by reading the
  test, not by debugging the code it accused.
- Assert layout in the harness (`getComputedStyle`), don't eyeball it — a CSS
  rule killed by a stray `*/` still renders a plausible-looking widget.
  **But a rect is not reachability: hit-test with `elementFromPoint`.** A
  dialog's Create button, pushed past `.sync-modal`'s hidden overflow on a short
  viewport, rendered with a perfectly plausible bounding box and was not
  clickable — `elementFromPoint` at its centre returned the scrim.
  `getComputedStyle`, visibility and `getBoundingClientRect` all called it fine,
  and so would a screenshot. Anything inside a clipping ancestor (every modal
  here) needs the hit test, not the box. Corollary for the `.sync-*` family: a
  body with `overflow-y: auto` CLIPS an absolutely-positioned popover into its
  own scrollbar, so a dialog hosting a combobox has to move the scrolling to the
  modal (`.nw-modal`) rather than delete it.
- **A frontend that mirrors a core DECISION needs a drift check, or it lies
  quietly.** `dnd.ts::predictTier` has `dnd-check.mjs`; the new-worktree verdict
  line has nothing, and it reimplements `cmd_new`'s *ordering* — which is not the
  obvious one (the holder logic is reached only when the derived directory does
  not exist, `ops.rs:417`). Four cases were wrong in the first version and every
  test passed. When you touch `ops.rs`'s create path, walk `NewPlaceDialog`'s
  chain against it by hand.
- **An accent token is a FILL, not text, and the theme you develop in will not
  tell you.** `--warn` as 11px bold text on its own 16% tint measures 3.1:1 in
  tokyo-day and 2.2:1 in catppuccin-latte; `--danger` is 2.2:1 in nord and 3.2:1
  in gruvbox-dark — and 6.2:1 in tokyo-night, which is where it gets looked at.
  The status chip shipped its first cut that way and read perfectly. The rule
  the app already follows elsewhere is the fix: hue lives in a DOT or a tint,
  words take `--txt-hi` / `--txt-dim` (`.status-row`, `.status-chip`). Measuring
  it has two traps of its own: a translucent background must be COMPOSITED over
  the first opaque ancestor before any ratio means anything — read
  `backgroundColor` straight and every ratio comes back 1.0, which reads as "no
  contrast anywhere" rather than "the probe is wrong" — and the parser needs
  both shapes (see the `color-mix()` note below). Then check the number against
  the app's OWN tokens before calling it a defect: `--txt-mute` is 2.5–2.8 and
  `--accent` is 3.1 in tokyo-day everywhere in this app, so a new panel matching
  them is consistent, not broken, and "fixing" only that panel is the actual
  regression.
- **Giving a `pointer-events: none` panel something to click turns its own
  dismiss handler against it.** `.usage-pop` is inert because it is measured
  before it is placed; the status band's link needed
  `.usage-pop.pinned { pointer-events: auto }`, and `UsageMeter`'s outside-click
  handler excluded only `trigRef`. Pointerdown on the link unpinned the panel,
  React unmounted it, and the `click` never landed on anything — a link that
  does nothing, and only in one host. Any close-on-outside handler must exclude
  the PANEL as well as the trigger. The harness cannot find this by clicking:
  `.click()` dispatches no `pointerdown`, so reproducing it needs a real
  `PointerEvent` at the element.
- **A `·` separator as `::before` on an ellipsising span dangles, and a
  `max-height` on a summary header nests a scroller.** Both shipped in the
  Plan tab's first cut. A span squeezed to nothing keeps its pseudo-element,
  so the line read "· " with nothing after it; the fix that holds is a real
  text-node separator beside each item with the row starting one
  separator-width LEFT of an `overflow: hidden` box (`.plan-seprow`), so the
  item that begins any line has its separator clipped. And a header capped
  at 55% of the pane with its own `overflow-y` put the phase list in one
  scroller and the document in another on a short window; bound each row
  on its own (line clamps, a collapsed list) and let the header not scroll.
  Where an item sits in a wrapping band decides what orphans at 240px —
  the age last left it alone on line 2 while `current` shrank to 51px;
  measure, then order.
- **`sel` is a selection; `selected` is a LOOKUP that can be null while `sel`
  is set.** `selected` resolves `sel` against `ws`, so it is null for the
  seconds between a restored selection and the first `list_workspace`, and for
  good once a place is removed elsewhere. Anything gated on `sel` that renders
  inside `selected && sel` disappears in that window — the status chip was
  assigned to a footer that did not exist and appeared nowhere at all. Gate on
  whatever the host actually renders under, which means declaring it below
  `selected` rather than up with the other derived state.
- **Read what `getComputedStyle` hands back before doing arithmetic on it.** A resolved
  `color-mix()` comes back as `color(srgb 0-1 / a)` while plain colours come
  back as `rgb(0-255)`; parsing both on the 0-255 scale made an added row and a
  deleted row measure IDENTICAL, which read as a real bug for a while. Once
  fixed, the same measurement found the actual defect (a "no line here" cell
  sitting 7–12 RGB units from context — invisible).
- **A `position: sticky` cell whose tint REPLACES an opaque background is
  see-through.** `.dg` set an opaque `--bg-tree` and the higher-specificity
  `.dg.del` swapped in a 14%-over-`transparent` mix, on the one column pinned
  inside a horizontally scrolling `max-content` grid — so code slid under the
  pinned line numbers, and only on CHANGED rows, i.e. exactly the rows being
  read. A sticky cell's background must be composited over a surface colour
  (`color-mix(… , var(--bg-tree))`), never over `transparent`.
- **A recessed box is only recessed against a surface it does not EQUAL, and
  moving a component to a new host re-asks that question.** `.update-log`
  paints `--bg-abyss`; so does `.main`. Mounting the status check there gave
  its three boxes (error pre, commits list, Claude's read) a background
  distance of exactly **0 in all six themes**, leaving a 1px `--line` border to
  carry them — which tokyo-day cannot, at ~5 RGB units (the band this file
  already calls invisible two rules up). The sheet never showed it because
  `.settings-sheet` is a different surface, so the bug is not in the component
  or in the class, but in the PAIRING. `--bg-input` was the plausible wrong
  fix: identical to `--bg-abyss` in catppuccin-mocha and under 7 units off in
  five of six. Check a borrowed surface token against the host in EVERY theme,
  and prefer `--bg-panel`, which differs everywhere.
  Three more from the same move, none visible to any suite:
  **a shared class restyled for one host restyles the others** — `.term-empty`
  also dresses the dock's "process exited" and "No shells" cards, so a scroller
  belongs in a `.term-empty-scroll` modifier the one host carries, leaving the
  base rule byte-identical; **`place-items: center` cannot host a scroller**,
  because an item taller than its track centres by overflowing BOTH edges and
  puts its own top above the scroll origin, unreachable; and **`align-items`
  default `stretch` eats a scrolled column's trailing padding** — the column
  clamps to the container, so its bottom padding sits at the scroll origin and
  the last section finishes flush against whatever is below (exactly `--s6`,
  32px, measured), which `flex-start` fixes while moving nothing when the
  content is short. That last one reads as decoration in review; it is not.
- **A container that rescales prose with `font-size` does NOT reach a child
  sized in an absolute length.** `.hs-read-md .md { font-size: var(--fs-meta) }`
  rescales every `em` in the block, but `.md-fence .code` sizes off
  `--term-size` (px), so a fence in Claude's read painted at 13px inside
  11.25px prose — the bigger the smaller. The inverse of the `--md-zoom`
  lesson: that conversion made sizes relative to the ZOOM knob, not to a
  container, so a new host has to re-state `font-size: inherit` itself (and
  scope it, or the dock's document view loses the 13px it means).
- **A replayed recording must not be ANSWERED, and "arrived before the invoke
  resolved" is not how you find it.** `shell_open` replays a dock shell's 256K
  ring into a fresh xterm on every re-attach; any terminal query in the ring
  (vim's DA2 / CPR / colour / cursor-blink burst — every `git commit` without
  `-m` leaves one) was re-asked, xterm replied down the pty as INPUT, and zsh
  echoed `2RR0;276;0c11;rgb:…` onto the prompt after every place switch.
  `TerminalPane` mutes `onData` while the replay parses, lifted by
  `term.write(bytes, cb)`'s callback (xterm runs it after THAT chunk and before
  the next, synchronously). The replay is the channel's FIRST message when
  `open` reports `replay > 0` — not whatever landed before `open` resolved,
  because Tauri sends a `Channel` payload above a size threshold through a
  separate `fetch` that can arrive AFTER the invoke's own response; only the
  channel's own order (`index`-buffered in `@tauri-apps/api`) is reliable.
  `app/scripts/termreplay-check.mjs` guards it and fails on the pre-fix file.
- **A dock tab's scrollback now OUTLIVES the app, so a spawn replays too.**
  `shell_open`'s spawn branch used to answer `replay: 0` — "a recording means a
  re-attach" was load-bearing documentation and is now false. The saved ring is
  sent before `sink` exists and before the reader thread is spawned, which makes
  it the channel's first message by CONSTRUCTION rather than by a lock (early
  shell output waits in the pty buffer with nobody reading it). The mute needed
  no change, which is the evidence the original contract was cut on the right
  axis. Two invariants that are easy to "tidy" away. The `── restored · … ──`
  seam IS seeded into the ring, which is the opposite of the tidy-looking rule
  ("the ring holds only what the shell wrote") and was got wrong first: a
  re-attach replays the ring and NOTHING else, so a seam kept out of it lives
  only until the first tab flip — and StrictMode re-attaches every pane before
  anyone has seen anything, so it was dead on arrival and the mock reproduced it
  exactly. Being in the ring makes it persistable, hence an ABSOLUTE local time
  rather than an age that would freeze at "2h ago", and hence
  `trim_trailing_seam`, so three launches with nothing typed between them do not
  stack three seams — and that trim must test the buffer's END (a seam is fixed
  width) rather than search a window, or it eats a boundary that had a session's
  work under it. And the ring is flushed on a 15s tick + at exit but NOT on
  `shell_detach`: a tab flip is not worth a 256K write, and detach runs under the
  registry lock, which may never wait on a file lock.
- **`HISTFILE` in the environment does nothing to zsh on macOS.** `/etc/zshrc:16`
  sets `HISTFILE=${ZDOTDIR:-$HOME}/.zsh_history` UNCONDITIONALLY for every
  interactive shell — export it, ask, and you get `~/.zsh_history` back. Same
  family as the OSC 7 note above: Apple's `/etc` is in the way, and editing the
  user's rc files is not the answer. `ZDOTDIR` is the one lever that file
  honours, so per-tab history rides on it — which means zsh also looks THERE for
  `.zshenv`/`.zprofile`/`.zshrc`/`.zlogin`, and the generated shims have to hand
  the user's own environment back **twice**: once in `.zshenv` (their `~/.zshenv`
  may `export ZDOTDIR=$HOME/.config/zsh`, the standard dotfiles layout — sourced
  blind, that hijacks the rest of the chain and the tab silently shares the
  global history again) and once in `.zshrc` before sourcing theirs (a config
  that does `source $ZDOTDIR/aliases.zsh` has to find its own files). Restoring
  it there is also what keeps a nested `zsh` on the user's normal history rather
  than the tab's. `INC_APPEND_HISTORY` is not a nicety: the app SIGHUPs its
  shells and zsh's default flushes only on a clean exit. String assertions on
  the generated shims prove nothing here — they pass just as happily if zsh
  ignores them — so the test that counts spawns a REAL login zsh on a pty with a
  temp `$HOME`, and it is the one that catches all three mistakes.
- **xterm's `scrollback` defaults to 1000 lines, and the backend replays 256K.**
  That is ~3200 lines at 80 columns, so two thirds of every replay was being
  dropped before anyone could scroll to it — invisible, because what is left
  looks like a perfectly good terminal. `TERM_SCROLLBACK` (5000) is checked
  against `SHELL_RING` by `termresize-check.mjs`, the same mirror shape as
  `dnd-check.mjs`. The reflow that fixes a replay's WIDTH lives in the same
  place and is safe only because the xterm is brand new when a replay arrives:
  the temporary grid must never reach `tx.resize` or `sentRef`, or the pty takes
  a SIGWINCH for a size nobody is looking at and the shell redraws into the ring
  being restored.
- **portable-pty's `Child::kill()` sends SIGHUP, not SIGKILL** (crate
  `lib.rs:347`), and an interactive `/bin/sh` on a pty whose master is still
  open SURVIVES it. The app only gets away with this because dropping the
  `Shell` closes the master and the EOF finishes the job. Two more traps in the
  same family, each of which cost a >10-minute hang: **a pty test must DRAIN the
  master** (without a reader the shell fills the pty buffer and the child wedges
  mid-exit — `ps` state `E`, never reaped, so even SIGKILL + `wait()` blocks
  forever), and **`process_id()` keeps returning a REAPED pid** (portable-pty's
  impl is an unconditional `Some(self.id())`, and `list_shell_sessions` reaps on
  every dock mount — so anything sampling by pid needs a `try_wait` liveness
  check first, or it eventually reads a stranger's process).
- **A Rust panic that never happened is an ObjC exception.** `panic in a
  function that cannot unwind` with **no** panic line before it is not our bug:
  the app's hook logs every real panic, so the missing line IS the evidence. An
  NSException raised inside AppKit unwinds through tao's `sendEvent:` override
  — `extern "C"`, therefore nounwind — and Rust converts the foreign unwind into
  an abort. The crash report will not name it either (`asi` is just `abort()
  called`, no `lastExceptionBacktrace`); the assertion text is in the unified
  log: `/usr/bin/log show --predicate 'process == "app"'` over the crash minute,
  and note `log` is not a zsh builtin, so a bare `log show …` dies with "too
  many arguments" and reads as "nothing was logged". This is how the
  select-text-and-die crash was found (`NSCampoLightweightUIController.m:1429`,
  macOS 26's Writing Tools affordance), and why the app now answers
  `allowsWritingToolsAffordance` with NO — added to wry's OWN WKWebView
  subclass, never the `NSKVONotifying_` one, whose methods go out of reach when
  the instance's isa reverts.
- **⌘+/⌘−/⌘0 is WKWebView page zoom, not a CSS multiplier.** `set_zoom` in
  lib.rs → `setPageZoom`; the frontend owns the step table (`ZOOM_STEPS`) and
  persists it. Page zoom is the only mechanism that reaches everything — the
  px-sized `Icons` props and the terminal, whose ResizeObserver refit re-cols
  the tmux pane; `--ui-rem` deliberately cannot (tokens.css keeps `--term-size`
  independent). Do NOT switch to tauri's `zoomHotkeysEnabled`: it injects a
  `window` listener that ignores `defaultPrevented`, so it fires ALONGSIDE the
  app's own handler, keeps a script-local level that desyncs from any
  programmatic call, and forgets it on restart.
- **An ⌥ chord cannot be matched on `e.key`.** macOS composes Option with the
  layout: ⌥- arrives as "–" (en dash), ⌥= as "≠", ⌥0 as "º". A handler keyed on
  the character is silently dead on every US Mac — the chord fires and matches
  nothing. `e.code` (`Minus`/`Equal`/`Digit0`) is the physical key and is
  immune; `zoomDir` in App.tsx tries `e.key` first and falls back to it. Merge
  the two tables with `??`, never `||` — 0 is a legal direction (reset).
  `app/scripts/zoom-check.mjs` guards both, and also that `ZOOM_STEPS` stays
  inside the Rust clamp; it slices the real handler out of App.tsx the way
  `race-check.mjs` does, so it tests the edit and not a paraphrase.
  **`settings.ts` cannot gain a relative import without breaking that
  script**: it loads `settings.ts` as a `data:` URL module, which has no base
  to resolve `./x` against (`ERR_UNSUPPORTED_RESOLVE_REQUEST`). `afterglow.ts`
  is inlined ahead of it (exports stripped, import line removed) — inline the
  real source, never stub the imported functions, or the check carries a
  second answer to the rule it exists to guard.
- **Playwright: a two-click arm needs BOTH clicks in one `browser_evaluate`.**
  The arm expires in 4s — longer than one MCP round-trip — and the button's
  `title` CHANGES when armed, so selecting on the unarmed title silently hits a
  DIFFERENT row. A run that did nothing at all reads exactly like a run that
  passed. This is the one exception to one-click-per-evaluate above, which
  exists for reading state, not for timing; read state in the NEXT call.
- **One click per `browser_evaluate`.** React batches, so several `.click()`s in
  a single eval return before any of them render — the DOM you read back is the
  one from before the clicks, which reads as "the tree ignored them". Drive
  state changes one call at a time and query in the next.
- **A suggestion's surface may not add preconditions of its own.** The v0.25.0
  MCP card was right about the machine and unreachable: `sel === null` AND
  `projects.length > 0`, each defensible, multiplying to almost never — while
  the release notes, which have NO preconditions, already carried the same
  sentence as prose. `offers.ts` now holds every after-update suggestion; an
  `Offer` is data with a destination (`to: {cat, focus}`), never a function,
  because "Set up" was never one click (the panel's checkbox decides whether
  Claude may REMOVE worktrees) and because N offers must stay N rows rather than
  N embedded panels. Three separate attempts here put the thing to DO after the
  thing to READ — changelog entry four of six, then a band below the notes, then
  a 6px dot — so the band is pinned OUTSIDE `.settings-body`, the modal's only
  scrolling child. And `offers-check.mjs` guards the modal's `offers=` FEED as
  well as its render: pinning `{offers.length > 0 && (` says nothing about
  `offers={sel ? offers : []}` one line up, which is the same bug relocated and
  which passed the check until review found it.
- **When a change deletes a thing, grep for comments that reason FROM it.**
  `SettingsSheet.tsx` justified an un-badged category with "the Home card is
  already making that offer" — of a card the same branch removed. The comment
  was not merely stale; its reasoning had inverted, and it was pointing straight
  at a real hole (the new dot had no off switch on a fresh install, because the
  only dismissal lived in a modal that never opens there).
- Plugin permissions live in `app/src-tauri/capabilities/default.json`;
  `opener:default` has open-url + reveal-item-in-dir but NOT open-path —
  a missing permission rejects the invoke silently. Never swallow errors:
  route failures through `fail()` (frontend) / `applog` (backend).
  **A path permission is only half of one.** `opener:allow-open-path` allows
  NOTHING on its own: the plugin's `is_path_allowed` ANDs the fs scope with
  "some allowed entry names a path", and a permission with no scope has no such
  entry — so the invoke rejects exactly as if it were missing. It needs the
  object form, `{"identifier": "opener:allow-open-path", "allow": [{"path":
  "**"}]}`. And `**` does not mean everything: glob runs with
  `require_literal_leading_dot`, which is TRUE by default on unix, so a dot
  COMPONENT never matches a wildcard — `**` covers `/Users/x/repo/f.ts` and
  rejects `/Users/x/repo/.worktrees/tree/f.ts`, which is every path this app
  exists to open. `plugins.opener.requireLiteralLeadingDot: false` in
  tauri.conf.json is what makes the scope mean what it reads as.
- App log: `~/Library/Logs/net.casadelvalle.worktrees/app.log` (Settings →
  Logs). Persisted UI settings: `ui-state.json` in the app config dir — written
  WHOLE-BLOB by the frontend, so the backend must never write into it (its own
  update would be erased by the next settings save; that is why each dock shell
  tab's last directory lives in a separate backend-owned `shell-cwds.json`).
- **A `place_panels` field whose global twin is a SEED must be optional.** Every
  key in that record also exists as a flat `Settings` key, and `panelsFor` uses
  the flat one as the seed for a place with no entry. `updatePanels` writes the
  WHOLE record, so filling a field in from `cur` (which is already seeded) freezes
  the seed into a place the user never set it in — and then spreads that stale
  value back over the global for the next place to inherit. `dock_open`/`dock_tab`/
  `dock_width` get away with it because each is written by an act you can SEE;
  `files_md_zoom` is optional so that ABSENT keeps meaning "still inheriting".
  Every test passed with the bug in: the value written was always correct at the
  moment it was written.
- **A `.md`-wide zoom means every size inside it must be relative.** `--md-zoom`
  (inline on the scroll box) works because the whole `.md` block was converted —
  headings from `rem`, tables/badges from `--fs-*`, fences from `--term-size`,
  spacing from `--s*` — to `em` / `calc(… * var(--md-z))`. Two traps: a shared
  class the block merely BORROWS keeps its own px padding (`.code-text` inside
  `.md-fence`), and **form controls do not inherit font**, so `1em` on an
  `<input>` resolves against WebKit's ~13.33px control font, not the prose —
  `.md-check` needs `font-size: inherit` before `width: 1em` means anything.
- Design tokens: `app/src/tokens.css` — everything scales off `--ui-rem`;
  terminal font is independent (`--term-*`). No UI libraries, plain CSS. "No UI
  libraries" means no COMPONENT/design-system libraries and no editor — a pure
  PARSER that emits data we render ourselves is allowed, and `marked` (lexer
  only, for the dock's markdown) is the one instance. Syntax highlighting is
  hand-rolled in `app/src/highlight.ts` for the same reason.
  **`mermaid` is the SECOND admitted exception, and only inside the docs viewer
  bundle (`app/viewer/`) — never `app/src`.** It does not fit through the
  `marked` door and should not be let in through it: it is not a parser that
  hands back data, it renders; it owns layout and theming; it injects ~4.4 KB of
  its own `<style>` into every SVG; it ships a sanitiser, which is a library
  telling you it puts untrusted input in a DOM; and it is 5.3 MB, about 95% of
  that bundle. It is admitted because a diagram is CONTENT, in a surface this
  app deliberately does not own, and because the alternative is not a weekend —
  it is Sugiyama layering and orthogonal edge routing. What makes it defensible
  is the boundary, not the argument: the viewer is a separate vite build
  (`vite.viewer.config.ts`, classic IIFE) and **never a second input to the
  app's**, so no shared chunk can hoist mermaid into `app/src`.
  `app/scripts/viewer-boundary-check.mjs` asserts that by walking the app's real
  import graph and, when a build exists, grepping `app/dist` — because
  "it is isolated" is otherwise a claim about a config file, and a config can be
  edited. Two traps that build found: **vite's LIB MODE does not define
  `process.env.NODE_ENV`**, so the bundle silently shipped React's development
  build (+1.9 MB, and the only symptom was a large file), and **a hash added to
  `style-src` beside `'unsafe-inline'` voids the keyword** and makes every
  diagram vanish while the prose still renders perfectly.
- **Driving the sandbox app by NAME drives the INSTALLED app — and it has no
  bundle id to address instead.** `sandbox.sh --app` runs `tauri dev`, which
  execs the UNBUNDLED binary (`<worktree>/target/debug/app` — the workspace
  shares one target dir); LaunchServices records it with a NULL bundle id and
  the name `app`, which is also the executable name inside
  `/Applications/worktrees.app`. So `tell application "worktrees"` resolves to
  the app the user is sitting in, `tell application id
  "net.casadelvalle.worktrees.sbx"` resolves to NOTHING (that id only exists on
  a `tauri build` bundle), `tell process "app"` is a coin toss between the two,
  and a bare System Events `keystroke` goes to whatever window is frontmost —
  the one stray "x" this produced landed in the user's live session. The only
  safe handle is the PID: find it with `pgrep -fl 'ui-tweaks/target/'` (your
  worktree's path), prove it with `ps -o pid,command`, and quit it with `kill
  <pid>`. Do not script clicks or keys into it at all; the sandbox is for
  READING its own files (`~/Library/Application Support/
  net.casadelvalle.worktrees.sbx/…`) and its app.log after using it by hand,
  and the mock harness is for everything that does not need real timing. If
  the sandbox is not running, stop — never fall back to whatever answers to a
  name.
- **`sandbox.sh` does not isolate `$HOME`, and that is right until it isn't.**
  The header says so deliberately ("testing an AI profile means checking that
  your global CLAUDE.md still loads"), and it is fatal for anything that READS
  `$HOME`: `mcpsetup::status()` parses `~/.claude.json`, so on a machine that
  already has the server the sandbox always reports `installed` and an offer can
  never appear. Reproducing `absent` needs a throwaway HOME — and the real
  `PATH` KEPT, because without it `worktrees` stops resolving, the state becomes
  `cli-missing`, and that state deliberately suppresses the offer, so you would
  measure the wrong thing and conclude the feature is broken. Point `CARGO_HOME`
  and `RUSTUP_HOME` at the real ones or the build re-downloads the registry.
  Second gap, unrelated: `--app` moves the CONFIG dir to
  `net.casadelvalle.worktrees.sbx` but `app.log` still goes to the **plain**
  identifier — a sandbox writes into your real app's log unless HOME is faked.
- **A vanished sandbox app means the human closed it.** `tauri dev` exits 0 when
  the window closes, which is indistinguishable from a clean shutdown because it
  is one; a harness kill is 143. Twice in one session that was reported as a
  crash and a relaunch started, once with a `setsid` "fix" for a problem that
  did not exist. Its state is readable afterwards — the config dir's
  `ui-state.json` shows which path was taken.
- **Never run the bundle's binary to probe it.**
  `target/release/bundle/macos/worktrees.app/Contents/MacOS/app --version` is the
  GUI entry point — it LAUNCHES a second instance instead of printing a version.
- **A Claude session probe's `status` can be STALE BY DESIGN.**
  `~/.claude/sessions/<pid>.json` is rewritten on status transitions — but ALSO
  on a park, which moves `updatedAt` alone and carries a mid-flight `busy`
  forward forever (upstream anthropics/claude-code#87131). `updatedAt >
  statusUpdatedAt` means the last write did not set the status the file carries;
  that, plus `parkedJobId`, is what `busy_is_delegated` keys on. Age is NOT a
  substitute — a genuinely busy session can go minutes without a write, which is
  why the dot has no expiry. Anything new read out of that file needs the same
  question asked: which write set this field?
- **A live Claude transcript's MTIME is not its last turn.** Claude Code keeps
  rewriting `~/.claude/projects/<mangled>/<sid>.jsonl` long after the session's
  last entry: measured across every transcript touched in a day, 24 of 28 had an
  mtime running from 12 minutes to **34 hours** ahead of the newest `timestamp`
  INSIDE the file. `backfill_worked` read that mtime to date a completion, so
  every launch re-dated every place whose session was still up in tmux to "just
  finished" — and since the stamp is forward-only and the backfill runs on every
  launch, the unread ring came back on each restart, on exactly the places the
  user had just acked. `last_worked_epoch` is also the nav's row age and sort
  key, so the same read reshuffled the tree. Date a session by `transcript_epoch`
  (max `timestamp` over a tail of the file), never by `stat`. The general rule:
  for any file claude owns, the metadata describes claude's bookkeeping and only
  the CONTENT describes the work.
- **A predicate that gates a DISPLAY must not also gate a WRITE.** The dot slot
  shows one glyph, so `unreadOf` subtracts live activity — correct for painting,
  and fatal as the ack's guard: a visit to a place whose session sits at
  `waiting` (the state it is in *because* it wants you) spent nothing, and the
  ring returned the moment that session went quiet. `unseenWork` is the FACT and
  guards the write; `unreadOf` is the fact minus live state and only paints.
  `afterglow-check.mjs` pins which one reaches `ack`.
- **`document.visibilityState` works here** — WKWebView fires `visibilitychange`
  on minimize, ⌘H, Space switch and full occlusion (confirmed on a real build via
  logged transitions). The Tauri issues claiming otherwise are Windows/WebView2.
  No `objc2`/NSWindow occlusion observer needed. It does NOT fire on plain focus
  loss, which is correct: a visible-but-unfocused window is still being read.
- Measuring anything in the app: `app.log` timestamps are **UTC**, most harness
  output is local. Cross-reference before trusting a window-state measurement —
  a "hidden" run that showed MORE work turned out to have flapped visible five
  times mid-window.
- macOS FS is case-insensitive: `Settings.tsx` collided with `settings.ts`
  once (component is `SettingsSheet.tsx`). Watch new filenames.

## Release

1. CHANGELOG: move `[Unreleased]` into `## [x.y.z] - date` (release.yml uses
   the section as notes; the app shows it as "What's new" — it ships in the
   binary via include_str!).
2. Bump workspace `Cargo.toml` → PR → merge.
3. `make release VERSION=x.y.z` → `git push origin main vx.y.z`.
   ⚠ **The app-bundle job builds the docs viewer's browser bundle and gates
   the server that serves it**: the gate step runs the docs-server boundary
   tests by exact name, which start the real server and, over a raw loopback
   socket, require `403` for a foreign `Host` and *not* 403 for a loopback one
   — both directions, so a server that refuses everything cannot ship either.
   It greps for `test result: ok. 4 passed`, because a renamed test turns
   `--exact` into a filter that matches nothing and exits **0**. `mo`, the
   third-party viewer this used to build from source and the reason the gate
   existed, is gone (it never validated `Host` — GHSA-6pff-wf7m-6f5h, reported
   2026-09-16); `place-docs.md` §17 is what replaced it, and there is no
   `vars.VIEWER_MO_*` any more.
4. release.yml: CLI ×4 targets + SIGNED app bundles ×2 + latest.json.
   Updater signing key: repo secret `TAURI_SIGNING_PRIVATE_KEY`; local backup
   `~/.tauri/worktrees-updater.key` — irreplaceable, never commit it.
5. Users update from inside the app (Settings → Version: CLI + app buttons)
   or by re-running install.sh.

## Local installs

- CLI stable: `install.sh` (copies). `make install` SYMLINKS the clone's
  build — every rebuild silently becomes "stable"; don't use it for that.
- App: `make install-app` → /Applications (local builds skip Gatekeeper).
- **macOS re-asks its privacy prompts after every build, and that is signing,
  not a bug.** TCC keys "worktrees would like to access data from other apps"
  to the designated requirement; ad-hoc/linker-signed code (everything cargo
  and tauri produce here) has `designated => cdhash H"…"`, a new identity per
  build. `codesign -d -r- <path>` shows which you have. Grants are recorded per
  TARGET app's data dir, so one build asks several times. Signing with any
  cert-backed identity makes them stick — but re-signing does NOT affect a
  RUNNING process (identity is fixed at exec) nor a tmux server it already
  started (responsible-process attribution is inherited at spawn and outlives
  reparenting to launchd), so a correct fix looks like a failed one until the
  app is quit+reopened and the old server is gone: check `ps -o lstart` on the
  tmux server before concluding otherwise. Don't try to diagnose from TCC.db —
  reading it needs Full Disk Access on the TERMINAL, and the tccd log is
  redacted. Releases stay ad-hoc; see ROADMAP for the distribution tier (and
  why the Mac App Store is not it).

## Decisions

`docs/adr/` holds decisions that must survive being forgotten. Read them before
adding config surface. **ADR 0001: a cloned repo never supplies argv** — no
`[hooks]`, no `[infra] up/stop/down`, no per-place `up_cmd`. `projcfg.rs`'s
`USER_ONLY_KEYS` makes them hard parse errors, and `DESIGN.md` still *describes*
them (marked superseded) because it was written before the reversal.

## Planning docs

`task_plan.md` / `findings.md` / `progress.md` are gitignored working memory —
read them at session start, keep them current. At close-out they get
tarballed into the session archive (see below). `_tmp/` is a user symlink
(iCloud) where screenshots for review land.

## Scratch files

Screenshots, harness output, and other throwaway artifacts go in
`~/.cache/worktrees/<project>/<worktree-name>/` (e.g.
`~/.cache/worktrees/worktrees/ui-changes/`) — never the repo root.

The Playwright MCP tools can't honour that directly: they refuse any path
outside the repo ("outside allowed roots") and drop their own output in
`.playwright-mcp/`. Let them write into the repo, then MOVE the artifacts to
the cache dir before close-out.

## Close-out ritual

When a work stream is done and the session is about to be `/clear`ed, run
the `/close-out` skill — GLOBAL since 2026-08-10, source in
`~/workspace/claude-skills` (symlinked into `~/.claude/skills`). This repo's
paths, gates, index and branch naming live in `.claude/close-out.md`, which
the skill reads; edit that file, not the skill. Short version: scratch →
`~/.cache/worktrees/…`, session summary + planning tarball →
`docs/sessions/<date>-<slug>/` + a row in `docs/sessions/index.md`
(committed), stragglers → `ROADMAP.md`, one squash-merged PR, then a fresh
branch off origin/main.

**`gh pr merge` reports a failure it did not cause.** From a side worktree it
dies with *fatal: 'main' is already used by worktree at …* — that is `gh`'s
local checkout step, AFTER the merge landed on GitHub. Check
`gh pr view <n> --json state,mergeCommit` before retrying, or you will re-merge
a merged PR. Same shared-branch rule as below, from a new direction.

**Tag the release from the worktree that already owns `main`** (the repo root).
`git checkout -B main` inside a side worktree moves the SHARED branch ref out
from under it, leaving the root on the new commit with a stale working tree and
phantom "modifications" — the inverse of the release, staged. Recoverable with
`reset --hard`, but check for untracked files first.

**No `checkout -B` is required to hit this.** Any branch checked out in TWO
worktrees does it: whoever moves the ref wins, the other tree keeps a stale
working copy, and its index reads as the inverse of everything that landed in
between — 600 lines of deletions that are not real. The reflog will not show it
(it records only that tree's own checkouts), so prove it before resetting:
`diff <(git diff --cached) <(git diff <branch-tip> <the-commit-you-were-on>)`
empty ⇒ the tree is exactly the old commit and there is nothing local to lose.
Give every worktree its own idle base (`<tree>-next`); `.claude/close-out.md`
lists them.

Branch off a FRESHLY FETCHED `origin/main`, and check with
`git rev-list --left-right --count origin/main...HEAD` — an idle worktree's
last commit can look like the tip and not be. PR numbers are not merge order:
a long-lived PR merges after higher-numbered ones, so a worktree parked on
"close out #83" was a commit behind because #72 landed later.
