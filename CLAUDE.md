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

**A throwaway git script needs a guard, because `git -C ""` means HERE.** A
probe let a repo path come back empty and aimed `branch -M main` and
`push origin main` at the live worktree; git refused both (the worktree guard on
the rename, non-fast-forward on the push) and nothing was lost, but neither
refusal was the script's doing. Route every call through one helper that
hard-exits unless the directory is non-empty and under `$TMP`. The cause is
worth knowing too: **bash expands every assignment on a `local a=1 b="$a"` line
before binding any of them**, so `b` is empty — and under `set -u` the function
dies mid-way inside a command substitution, leaving the caller with "".

**A new test must be shown to FAIL first.** ROADMAP's zombie-children item
records a regression test that passed identically with and without its fix.
Break the thing under test (drop the `skip_serializing_if`, restore the old
line), watch it go red, then restore. Two tests this repo now relies on were
confirmed this way.

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
