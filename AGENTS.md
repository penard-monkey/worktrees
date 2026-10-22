# worktrees — working notes for Claude

One git worktree per branch, one tmux session per worktree. A worktree is a
durable PLACE; a branch is work that flows through it. Full design docs:
DESIGN.md (app), MIGRATION.md (bash→Rust history).

## Architecture

- **One engine**: `crates/worktrees-core`. The CLI (`crates/worktrees-cli`,
  binary `worktrees`) uses it in-process, and so does `worktrees serve`, which
  hosts the command layer (`crates/worktrees-api`). The Tauri app
  (`app/src-tauri`) is serve's CLIENT — one engine per host: its shim finds or
  starts serve (`engine.rs`) and keeps only the window, the client-local
  commands and `host::boot`. The legacy bash engine is retired.
- git/tmux are **shelled out** on purpose (faithful port; keeps the bats
  fake-shim harness intercepting the compiled binary). Don't switch to libs.
- **Every tmux call goes through `tmux::tmux()`, which adds `-N`** (tmux ≥ 3.2,
  feature-detected); only `new-session` uses `tmux_launch()`. A client that may
  start a server, meeting a refused or missing socket, unlinks it and starts a
  SECOND server, orphaning every session in the first (2026-10-07). A spawn
  that cannot use the wrapper (the app's pty attach) adds `no_start_args()`.
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
  Across PROVIDERS the bus is the MCP message log (`worktrees_core::messages`,
  in the git common dir; `report`/`messages`/`wait`, `from` derived from the
  server's own place) — Claude↔Claude still uses Claude's own messaging, and
  Codex activity for `place_status`/`wait` and the nav dots comes from ONE
  derivation, `worktrees_core::activity`.

- **`worktrees serve` (Server Edition, Phase 1) hosts the api behind an
  authenticated WebSocket on `127.0.0.1`**: `{id, cmd, args}` in, `{id, ok}` /
  `{id, err}` out, channel frames `{channel, seq, data}` for `ByteSink`s and
  progress — ending in `{channel, seq, end: true}` when the engine lets the
  sink go — and `{event, payload}` for `Host::emit`. Requests to ONE stream
  run on a serial lane per stream (`dispatch::lane`): a dispatcher that gave
  every frame its own blocking task delivered 26 in-order keystrokes to zsh as
  `0124356789…`; everything else stays concurrent. The rule set is in
  `crates/worktrees-cli/src/serve/policy.rs` and every refusal closes the
  socket with NOTHING written. Auth is handshake v1 ((main)'s spec, for OWASP
  H1): `hello` → `challenge` → `auth` → `ok` + `proof`, each side's proof an
  HMAC keyed with serve.json's token over the other side's nonce, so the token
  is NEVER in a frame — under v0 it was the first frame, and whatever held a
  dead server's port received it. `AUTH_DEADLINE` covers the whole exchange,
  not each frame. Limits (OWASP M1, `conn.rs`): 64 connections / 16
  unauthenticated, 512 pre-auth bytes (a `Budget` UNDER tungstenite, which
  cannot change its own limits after construction), 16 MiB per message after
  auth, a 5 s HTTP phase, RLIMIT_NOFILE raised at boot. A reconnecting local
  flood still locks NEW clients out (measured 0 of 49) — it can no longer take
  memory, descriptors or a client already in. A missing `Origin` is refused
  there and allowed by `docserver::origin_ok` ON PURPOSE (different pages,
  different questions —
  do not align them). hyper parses the HTTP and tungstenite sees only frames,
  also ON PURPOSE: tungstenite's `accept_hdr_async` WRITES an HTTP error
  response when it refuses, while a hyper service that returns `Err` closes
  with 0 bytes. The 152 commands sit in ONE macro table
  (`serve/dispatch.rs`) with an `Exposure` each; the six `ClientLocal` rows
  (clipboard, editor, terminal, zoom, the app bundle) are refused with
  `err: "client-local"`, never run on the host — the docs viewer is an
  `Engine` row since the app became a client (its registrations are the host
  poll's). `serve-dispatch-check.mjs` fails if the shim's `generate_handler!`
  registers any other row: that would be a second engine for it. **Every row
  also declares a capability and its objects** (Phase 2 R1:
  `name: Engine [write, place(repo, slug)] =>`; `HostConsole` rows are
  console-only, `HostAdmin` is gone), and `serve/resolve.rs` checks them
  against the caller's (principal, device) grant before the body runs,
  writing canonical values back; the body gets `Ctx::scoped`, so
  `read_projects` and everything on it answer with the caller's roots.
  `Row::new` refuses a row without them at COMPILE time, and the drift tests
  in `dispatch.rs` fail on an object argument a body reads that its row does
  not declare. The types are `worktrees_core::authz`. On loopback the caller
  is `Caller::console()` (scope `"*"`), and its refusals carry the error the
  body always gave. **A git fetch a device can trigger** (`clone_project`,
  `skill_preview_git`) goes through `worktrees_core::fetchpolicy`, keyed on
  the caller's class (`Ctx::fetch_policy`). The `remote_url` object resolves
  the host ONCE, and the body's git dials that pin over https only, with no
  redirect, proxy, protocol v2 or host-config rewrite left to move it. The
  console's git is unchanged. A new network git call a device can reach is
  built with `Policy::git`, never a bare `git`. Serve's in-process tests resolve against a fixture
  `World`, not the user's registry and tmux: a scratch repo is `register`ed
  there (`serve/tests.rs`), or every place row says `not a registered
  project`. The engine's
  wiring — log sink, panic hook, PATH/locale, the viewer sweep, the project
  merge, the 3 s poll, the exit sequence — is `worktrees_api::host::{boot,
  start, stop}`: serve calls all three, the shim only `boot` (it logs, can
  panic and launches editors), and `api-boundary-check.mjs` fails on any
  engine piece back in lib.rs. Host policy is NOT in that list:
  `hostpolicy::boot` is the engine's act, so `cmd_serve` calls it once,
  after the one-engine refusal and before the listener binds, and the shim
  never does (hostpolicy-check.mjs) — a serve that skipped it launched lanes
  under the default the file overrides. `<config_dir>/serve.json` (0600,
  rewritten per launch) is how a client finds the port and the token; the log
  is `~/.cache/worktrees/serve.log`, 0600 as well (re-tightened on every
  write, and after its rotation, which is a `rename` and keeps the mode it
  found), version on the first line. serve installs its signal handlers
  BEFORE it writes serve.json: a supervisor's SIGTERM in that gap killed it
  with no exit status. Anything consumed by being emitted (the inbox's
  `worktrees show`) waits for `Host::has_audience()` — and the app's own
  verification probe authenticates, so that reads true for a few ms at every
  launch (ROADMAP). There is no CLI client: a per-command log line
  (`mcp_status`) appears only once a client calls the command, so probing
  serve by hand needs a handshake-v1 client — copy `serve_bin.rs::connect`.
  `app/scripts/server-stack.sh` runs an isolated one for the frontend lane.

- **The frontend has ONE door to the backend: `app/src/ipc.ts`** (`invoke`,
  `Channel`, `listen`). Nothing else under `app/src` imports
  `@tauri-apps/api/core` or `/event` — `transport-check.mjs` fails on it, the
  mock excepted. In the app the door is `engineDoor`: the commands in ONE
  table (`CLIENT_LOCAL` = serve's `ClientLocal` rows, plus `SHIM_COMMANDS`:
  `engine_connect`, `raise_window`) go to Tauri IPC, everything else to
  `ipcws.ts`, a WebSocket to serve; `engine-route-check.mjs` holds the table
  against dispatch.rs and `generate_handler!` both ways. The dev stack is the
  same door with no local half; the mock keeps plain Tauri. The token reaches
  the page only as `engine_connect`'s answer, and the socket never sends it: handshake
  v1 (OWASP H1) has each side prove it holds it with an HMAC over the other's
  nonce, and the client sends NOTHING else — no request, and no belief in the
  version — until serve's proof passes `crypto.subtle.verify`, because whoever
  holds serve.json's port after a crash need not be serve. Frames that arrive
  during verification are held and handed on SYNCHRONOUSLY as the proof
  passes, so a listener added after `await ready` misses them — a held
  `app:open-doc` rides right behind `ok`: wire listeners before the proof. A
  parked event reaches React on a macrotask (`setTimeout(0)`), because
  StrictMode's throwaway listener has a microtask cleanup that a microtask
  hand-off reaches first. `crypto.subtle` is
  there on `tauri://localhost` (a secure context) and a `ws://127.0.0.1` socket
  from it is not blocked as mixed content — measured on macOS 27.0 in a
  standalone WKWebView registered the way wry 0.55 does, NOT in the shipped
  app. A channel ends on serve's `{channel, seq, end: true}` — ordered by
  `seq` like data — or on an `err` to the request that named it, and the
  client drops the handler then, as Tauri's `Channel` does when its Rust side
  drops; and a request over serve's `MAX_MESSAGE` is refused in the client,
  because serve answers one by ending the whole connection, every terminal on
  it. A lost socket is retried ON ITS OWN, and the policy lives in ONE pure
  module, `app/src/reconnect.ts` (backoff, ≤3 starts in 60 s, never restart
  a serve the app did not start or adopt, hidden window defers); the shim
  only obeys `engine_connect`'s `start: false` (`engine::permit`) and reports
  `owned`/`started`, the two facts the policy cannot see. Do not grow a
  second copy of the rule in Rust. **Three starts inside a minute is a crash
  loop by that rule, whatever killed them** — a drive that `kill -9`s serve
  three times in 53 s gets a stopped banner, not a fourth serve; space
  manual checks a minute apart. And keep the window VISIBLE while you drive
  it: fully covered, WKWebView reports `hidden` and the retrier defers by
  design, which looks exactly like a missed loss (check CGWindowList before
  calling it a bug). Plugins (opener, dialog, updater,
  process, `api/window`) stay direct imports on purpose: they act on the
  window's machine, and routing them through a socket is the reason the
  plan's WS-backed `__TAURI_INTERNALS__` was not used (`.planning` of
  feat-serve-transport, findings §1). Drive it:
  `J=$(app/scripts/server-stack.sh --allow-origin http://localhost:1430)`, then
  in `app/`: `WT_SERVE_JSON=$J corepack pnpm dev:serve --force`. Four traps:
  **vite's dev CORS default answers every `localhost:<any port>` page**, so the
  token endpoint (`vite.serve-door.ts`) checks `Sec-Fetch-Site`/`Origin` and
  strips the CORS headers itself — same-origin is not free there;
  **`vite build --mode development` does NOT make `import.meta.env.DEV` true**
  (NODE_ENV stays production — `NODE_ENV=development` does), so a "dev build"
  can look like proof the gate works; **`ipcws.ts` may have no runtime import**
  (the check loads it as a `data:` module; `import type` is fine); and **a
  check that awaits a promise a mutation leaves pending exits 13 with NO
  failure printed** — race every await against a deadline, as
  `transport-check.mjs`'s `settle()` does, and give fire-and-forget calls the
  same handler: against a broken client a bare `void invoke(…)` rejects
  unhandled and kills the run before the failures it found are printed.

**Adding a harness? Walk `docs/adding-a-harness.md` first, and tick it only
by SEEING each surface work.** pi shipped in v0.33.0 with a correct core —
MCP `place_status` said `busy` — and no nav dot at all: the app builds its dots
in its own poll (`lib.rs` then, `worktrees_api::watch` now: `claude_activity` + `codex_tick` → `merge_activity`
→ `sessions:busy`/`sessions:done`) and pi was never added to it. Three reviews
passed it because every check exercised the core and none looked at what the
running app draws. "Core reports it" is not "the user sees it"; the checklist
is by surface, and the last item is one lane driven end to end in the real app.

**A long-lived MCP server outlives a CLI upgrade, so "the new binary cannot be
doing this" is not evidence.** The temporary `mcp-debug.log` kept growing for
two days AFTER the release that deleted the apparatus merged, which reads
exactly like a failed removal — 981 KB to 2.8 MB, with the installed CLI
already two minors past it. It was not: `worktrees mcp` processes started
before the upgrade were still running their old image, and the proof is in the
file (no `start` line above the removing version, and last writers whose pids
were long dead). Check the WRITERS a log names and the versions it records
before doubting the deletion; a `start`-style line that stamps the running
version is what makes that possible at all. Incidentally the same two zombie
servers wrote ~1.85 MB in two days of pure sidecar churn.

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

**A client's string is never a flag (R3) — and `--` alone would have made it
worse.** Every core `cmd_*` parser sorts flags from positionals by the leading
`-`, and `worktrees-api` pushed the app's strings in bare: `relink`'s `slug:
"--all"` relinked every place, `--force` included, and came back OK. Three
parts, all required. The api builds every argv with `worktrees_api::argv::Argv`
(our flags are `&'static str`, a value is glued on as `--x=<v>`, positionals go
after `--`; a test fails on any other call to a `cmd_*`). The parsers honour
`--` (`worktrees_core::argv::split_options`). The boundary refuses a `-`-led
branch, base or slug with core's `safe_arg`/`branch_arg`, which the MCP server
shares. The trap is the middle part: core was safe by ACCIDENT, because the
parser refused every `-`-led positional as an unknown flag. Honour `--` and the
value reaches git, which permutes options the same way, and `do_switch`'s
`git fetch origin <base>` with `--upload-pack=<cmd>` RUNS `<cmd>` (measured, on
a local-path origin). So a git call carrying a client value puts its own `--`
in front of it, and teaching any other parser `--` starts with walking where its
positionals land.

## Gates (run before any PR)

```sh
cargo build --release -p worktrees-cli   # FIRST — bin/worktrees shim prefers
                                         # target/release; a stale release binary
                                         # makes bats fail mysteriously
make test           # bats suite vs the Rust binary (fake git/tmux shims)
make lint           # shellcheck + bash-3.2 gate (shim + install.sh)
cargo test -p worktrees-core
cargo test -p worktrees-cli   # MCP protocol unit tests live here
cargo test -p worktrees-api   # the command bodies + tests/headless.rs
cargo test -p app --lib      # `check`/`build` do NOT compile `mod tests`
cd app && ./node_modules/.bin/tsc --noEmit && cargo check -p app
```

CI mirrors these + builds the app crate on both OSes. Squash-merge PRs.

**Docs-only PRs skip CI by design.** `ci.yml` `paths-ignore` covers `docs/**`,
`ROADMAP.md`, `AGENTS.md`, `CLAUDE.md`, `DESIGN.md`, `MIGRATION.md`,
`README.md` and `.claude/**` — a close-out archive PR shows ZERO checks, which
is correct, not a hung run, and nothing blocks the merge (main has no required
status checks). The skip applies only when EVERY changed file matches the list;
mix in one code file and the full suite runs. `CHANGELOG.md` is deliberately
NOT on the list — it ships inside the app binary via `include_str!`. THIS file
IS on it, for the same reason as the `CLAUDE.md` that imports it: no
`include_str!` reads it and no test reads the repo's own copy — every
`AGENTS.md` under `test/` and `app/scripts/` is a fixture written into a
scratch repo.

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
The same trap in code: **"keep both" on two blocks that each APPEND at the end
of a container drops a closing brace.** Both sides added tests at the end of
`mod tests`, git hoisted their shared `    }` out of both, and the union
failed with "unclosed delimiter" — in two lanes, on one merge pair (#508 ×
#523). Put the hoisted line back between the sides.

**A clean merge is not a compiling one.** Merging #522 into the stack that
held #513 reported no conflict and failed with E0433: #513 had dropped a
`Command` import that a test #522 added still used. Run `cargo check
--workspace --all-targets` straight after any cross-branch merge; plain `cargo
check` skips test code. When the branch MOVED code (#508), main's fix to the
old file conflicts in a file that no longer holds it: `checkout --ours` only
once `git diff <base> origin/main -- <file>` shows the fix is main's whole
delta there, then port it and its tests by hand. A path-keyed allowlist
(`OWN_TEMP_NAMES`) going red on exactly the moved rows is the proof the port
took. And **`git merge … -F -`
does not read stdin**: it fails (rc 129), no merge happens, and every check
after it examines the old HEAD. Use `git merge --no-ff --no-commit <rev>`,
then `git commit -S -F -`.

**A stale release binary does not only make bats FAIL — it can make it PASS.**
The note above says "fail mysteriously", which is the friendlier half. Edit a
crate *after* the release build and `make test` happily green-lights the OLD
binary; the same goes for the `ls --json` diff below, which then compares the
shipped binary against another copy of itself. Both read `target/release`, and
neither notices it predates the change. Rebuild, then confirm before believing a
result: `[ target/release/worktrees -nt crates/worktrees-core/src/store.rs ]`.
(The app bundle's CLI sidecar no longer touches that file: tauri's
`externalBin`, which it briefly used, made tauri-build copy the staged CLI ONTO
`target/release/worktrees` with a fresh mtime — see Release, step 4.) Its
cousin: a before/after pair built from COPIED sources whose mtimes predate the
existing outputs rebuilds nothing, and a Docker "after" binary came out
byte-identical to "before". `cmp` the two binaries before believing the
comparison.

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

**Inside Claude Code, `grep` is not grep, and it skips binary files without a
word.** The shell snapshot defines `grep` as a function that runs Claude's
bundled ugrep with `-I`, so any file holding even one NUL byte is skipped
rather than searched. `projcfg.rs` held one, a literal NUL in a comment, so
`grep -r USER_ONLY_KEYS crates/worktrees-core/src` listed every file that CITES
the const and not the one that DEFINES it, and a single-file grep printed
nothing and exited 1. A lane concluded from exactly that, on 2026-10-17, that
the const "no longer exists"; it sits at line 51, and the claim was relayed
before anyone checked. A "not found" that a conclusion rests on needs a second
witness: `git grep` or `/usr/bin/grep` (`command grep` skips the function),
which both REPORT a match in a binary file instead of skipping it.

**A backslash escape written through a script can land as the raw byte.**
`"\x00"` or `"\u0000"` inside an ordinary Python string literal IS a NUL by
the time the file is written. One archived session summary says in its own
words that a Python heredoc did this — in a sentence whose `\x00-\x1f\x7f` had
itself become raw bytes — and `projcfg.rs`'s NUL fits the same shape. When a
script writes source or docs, use raw strings (`r"…"`) or double the
backslash, then check the file with `file`, which must say text. Do not count
on git to notice: it sniffs only the first 8000 bytes, so it showed that
summary's diffs as `Binary files differ` and rendered `projcfg.rs`, whose NUL
sat deeper, as ordinary text.

**A pipeline's exit status is the LAST stage's.** `make test | tail -15` exits 0
because `tail` did, and a mid-stream `not ok` scrolls off a 15-line window — so
the run reads green whatever bats said. Redirect, then check both halves:
`make test > log 2>&1; echo $?` plus `grep -cE '^not ok' log`. Same family as the
grep traps above, and the reason to run gates with `make -C <repo-root>`: the
Bash tool's cwd persists between calls, so an earlier `cd app` turns a later
`make test` into "No rule to make target `test`" — a failure that looks like the
change broke the build. And count against the tool's REAL output: a gate
summary that counted `^FAIL` read `fail=0` on a red transport-check, which
prints `not ok`; only the exit code caught it. `make test-frontend` stops at
the first failing check, so run the `*-check.mjs` in a loop that does not,
or the first failure hides the rest.

**Two `gh` commands lie about CI, in opposite directions.**
`gh pr checks <n> --watch` run in the gap between a push and the run
registering prints "no checks reported on the 'x' branch" and exits **0** —
identical to a pass, and it is what you get every time you push and immediately
watch. And `gh` reports `conclusion: ""` for an in-progress job, never `null`,
so a hand-rolled `--jq 'select(.conclusion != null)'` rollup counts every
RUNNING job as a failure. Both fired within a minute of each other on #304, one
claiming green and one claiming nine failures, neither true. Watch the run, not
the PR view: `gh run watch <run-id> --exit-status`, then read raw conclusions.

**An MCP bug report describes the server that answered it, not the installed
binary.** Every agent session launches `worktrees mcp` once and keeps it for
its life, so an install upgrades nobody already running. Two "live" Codex bugs
reported by a peer were a v0.32.0-era server answering after v0.32.1 was
installed. Before debugging one, compare `ps -Ao pid,lstart,command | grep
"worktrees mcp"` with the binary's mtime, and `git log -S` the reply text the
peer quotes: if the string only exists in history, so does the bug.

**A CONFLICTED PR gets no CI at all, and it does not look like a conflict.**
`on: pull_request` builds `refs/pull/N/merge`, which GitHub cannot create while
the branch conflicts with main — so `gh run list --branch <b>` comes back EMPTY
while every other branch runs normally, which reads as a broken workflow or a
queue that never started. #310 sat like that until a rebase, then its run
appeared within seconds. Check `gh pr view <n> --json mergeStateStatus` (DIRTY)
before debugging the workflow. Note `mergeStateStatus` also reads `UNKNOWN` for
a few seconds after any push while GitHub recomputes it.

**A GitHub-native PR stack loses its linear history the moment main moves.**
After #522/#523 merged, the Server Edition stack's bottom (#508) went DIRTY and
every PR above it sat at `unknown` with no CI at all; close/reopen did
nothing, and changing a base is refused ("part of a stack"). Merge main into
the BOTTOM branch, then each branch into the one above it, all the way up, so
every branch contains the tip below it again. Land the finished stack in one
go, by squash: one commit per PR and no intermediate main that fails.

**Run `app/scripts/*-check.mjs` from the REPO ROOT, as CI does.** From `app/`
at least one (`plan-usage-check.mjs`) fails even on `main`, for a reason that
has nothing to do with your change — and a failure that is "also on main" reads
as pre-existing. One was called pre-existing that way while it was a real
regression on the branch (#367). Compare against `main` from the same directory
CI uses before calling anything pre-existing.

**bats helpers run under macOS `/bin/bash` 3.2 on the macos runner.** A
pattern-substitution REPLACEMENT keeps its quote characters there —
`${c//"'\''"/"'"}` turns `'` into `"'"` — so a helper passes on a
Homebrew bash 5 and fails in CI on one OS only. Use `sed` for quote surgery, and
reproduce with a dir holding `bash -> /bin/bash` first on PATH for the bats run.

**A standalone `[[ ]]` does NOT fail a bats test — `[ ]` does.** Measured on
bats 1.12.0 here: a false `[ 1 -eq 2 ]` in the middle of a test aborts it, and
a false `[[ "$output" == *"NOPE"* ]]` in the same position passes. Only a
`[[ ]]` that happens to be the test's LAST line decides anything, because then
its status is the body's. **572 of this suite's 755 `[[ ]]` assertions are not
the last line**, so they currently assert nothing — every one of them is a
string check on `$output`, which is exactly what the suite leans on. The tell
is brutal: such a test passes identically with the feature deleted, so "the
bats suite is green" says less than it looks. Write string assertions as
`grep -qF "needle" <<<"$output"` (a simple command, so errexit sees it; a
herestring, so there is no pipeline and no SIGPIPE games), or load the
`bats-assert` submodule that is already vendored at `test/lib/bats-assert` and
never wired up. Found while adding a test that passed before the fix existed;
see ROADMAP.

**Killing a bats run writes a `not ok` into its log.** A `pkill` (or a branch
switch under a running suite) fails the in-flight test inside `common_setup`
and prints `bats warning: Executed N instead of expected 340 tests`. A later
grep of that log reads as a real regression. Check the plan line and the count
before believing a single failure in a log you did not watch finish.

**A throwaway tmux server needs `-L` (or `-S`) on EVERY command — `TMUX_TMPDIR`
is not isolation inside a lane.** Every lane runs in a tmux pane, so `$TMUX` is
set, and a bare `tmux …` talks to the server `$TMUX` names: the user's REAL one.
`TMUX_TMPDIR` only picks the socket directory when `$TMUX` is unset. On
2026-10-01 a lane probed with `export TMUX_TMPDIR=$T; tmux new-session …; …;
tmux kill-server` and killed the user's whole tmux server — every session, every
agent. Use `tmux -L <unique-name> …` on every call (including `kill-server`), or
`env -u TMUX tmux -S "$T/sock" …`; never a bare `tmux kill-server` anywhere.

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
deleted, so valid YAML would only have moved the failure later. A third way to
a green run that ran nothing: **`echo "k=$(cmd)" >> "$GITHUB_OUTPUT"` throws
away `cmd`'s exit status.** A failing `jq` in the plan job emitted empty
matrices, which is zero jobs and a pass; it was found only because a Docker
image had no jq. Assign first (`v=$(cmd)`), then echo.

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
below: a guard that cannot be wrong is a guard that cannot fail. And **cutting
a Rust file at its FIRST `#[cfg(test)]` is not slicing off `mod tests`**: a
test-only `#[cfg(test)] static TEST_PANIC` above dispatch.rs's table moved the
cut, and three checks read an empty table (transport-check went red on the
merge, the lucky outcome). Cut at the `#[cfg(test)]` that heads `mod tests`
(`\n#[cfg(test)]\s*\nmod tests\b`), or drop gated items one at a time as fsx's
`live_lines` does. Neither sees a test module declared from ANOTHER file
(`#[cfg(test)] mod tests;`), which is why `serve/tests.rs` carries its own
allowlist row (ROADMAP).

**`symlink_metadata` refuses to follow only the LAST component.** Every
intermediate directory in the path is resolved exactly as `metadata` would
resolve it, so `symlink_metadata("a/b/c.md")` reads through an `a -> elsewhere`
and reports a perfectly ordinary file outside the tree. A walk that lstats each
entry as it descends is safe; a check that lstats a path built from a CONSTANT
(`docs::walk`'s `.planning/brief.md`) is not, and the two sat four lines apart
in `docs.rs` — the walker refused the link, the by-name read went through it,
and the half that leaked was the half that always listed. The tell is that a
partial guard looks like a working one: the rows the walker refused are missing,
exactly as they would be if it all worked. `[docs]` paths had the same hole
through `resolve_rel` and `root.join(rel)`. `docs::passes_through` closed it:
it lstats every intermediate component and lets a link through only when its
target canonicalises back under the place root (`safe_under`'s rule), so a
monorepo's inside link keeps its tree.

**A test can pass because its FIXTURE omits the thing.** The regression test for
that symlink wrote a directory behind the link and asserted the plan rows were
absent — and passed, because nothing had put a `brief.md` in it. It was shown to
fail first, against the right code, for the wrong reason. Sibling of the
"recovery test that hand-clears the blocker" note below: when a test reaches its
assertion, ask what is NOT in the fixture as well as what was poked into it.
Its twin: **a fixture written THROUGH the code under test can do the very step
the test must prove.** D7's fixture went through `write_scrollback`, which
records the file as seen, so the read-on-spawn path the test was about never
ran. Write fixtures raw.

**A temp file beside its target has a name a cloned repo can commit.** Every
"atomic write" here was `fs::write(<fixed or pid-named tmp>)` + `rename`, and
`fs::write` follows a symlink. git checks a committed `120000` blob out with no
check, so the store, `.worktrees.toml`, `.worktree.env` and the dock editor's
save all wrote through a planted link into a file outside the repo (measured on
0.42.0). In the app the store write needs no click: a finished Claude turn
rewrites it. Replace a file with `fsx::write_atomic`, which uses a random name,
`O_CREAT|O_EXCL|O_NOFOLLOW` and fchmod on the handle, and removes only its own
temp. Do not hand-roll the dance: `fsx::tests::no_source_builds_its_own_temp_name`
scans every workspace `src` for a temp-like literal or a pid in a name and fails
unless the line is in its `OWN_TEMP_NAMES` list with a reason. A pid suffix is not randomness: a repo can
commit one link per pid. Removing whatever sits at a predictable name first
only trades following the repo's file for deleting it. The same goes for a
plain write to a fixed path in a checkout: the brief went through a committed
`.planning/brief.md`. The viewer's derived tree keeps its own writer on
purpose. It lives in the app's config dir, and a sync per document would cost
every derive.

**An assertion that describes the MECHANISM instead of the RULE encodes the bug
as correct.** `the_watch_signal_moves_on_membership_and_not_on_work` ended by
writing a content-free change to the declared sidecar (`{}` ->
`{"places":{}}`) and asserting the watch signal moved — which passed only
because the file's byte LENGTH moved, so it passed identically for a
`last_worked_epoch` stamp, the very write that should be silent. The rule was
"what the resource list SHOWS changed"; the assertion said "the file changed",
and the suite stayed green for the life of the bug. Two tells, both cheap: the
assertion names a file or a timestamp where the rule names a FIELD, and
mutating the code the other way leaves it green. A signal like this needs a
PAIR of directions (title/lifecycle must notify, clock fields must not), each
confirmed red by breaking the fix the opposite way — too narrow is as wrong as
too broad and does not look like it. Third sibling of the two notes above.

**A test that races two sleeps asserts the scheduler, not the rule.**
`a_forced_ask_queued_behind_a_fetch_shares_it` slept 40ms and expected to land
inside a 150ms fetch on another thread; a macOS CI runner spawned that thread
late enough that it did not, and the failure read as the PR cache fetching
twice. Locally it passed 40 of 40, which is why it reached CI. The rule was
about three STAMPS (asked, started, done), so the fix is the `_in` seam one
level down: `answer_with` takes the clock, the test scripts the stamps, and a
`Barrier` keeps the shape without deciding the outcome. The Rust twin of
`termresize-check.mjs`'s virtual clock — any test with a `sleep` on both sides
of a race wants the same treatment.

**To reproduce a macOS CI timing flake, starve the TEST, not the CPU.** `yes`
hogs did nothing in 400 runs (macOS demotes them); the test binary under
`taskpolicy -b` reproduced #522's kimi SIGTERM failure 17 times in 400, and a
`touch` shim on PATH sleeping 2.5 s made it deterministic. Read the CI log
first: libtest runs tests in sorted order, so the previous test's finish to
`FAILED` is how long the failing one ran — about the bound (2.07 s against a
2 s grace) means a timeout expired, about zero means a signal was lost. Run a
stress loop OUTSIDE the gate slot, one line per run, under a time cap: one
inside it held the slot ~82 min through a machine sleep, printing nothing.

**A test that writes a fake binary and then runs it can lose to ANOTHER test's
fork, and only on Linux.** `execve` of a file that any process holds open for
writing fails with ETXTBSY (errno 26). libtest runs tests on parallel threads,
and a fork made by one of them inherits every fd the process holds at that
instant, keeping each until the child's own exec. `O_CLOEXEC` acts AT the exec,
not before. So `fs::write` + `set_permissions(0o755)` in one test, plus any
spawn in another, opens a window in which the fake cannot run. It presents as
the probe's own spawn failure: `codex_usage` answered `"spawn"` where its test
expected `"timeout"` in the ubuntu job of #526, while macOS, which runs a file
that is open for writing, stayed green. A rename or a hard link does not help,
because the check is on the inode. Create every executable a test RUNS with
`fsx::exec_fixture` (the app crate's copy is in `engine.rs`): a short-lived `sh`
writes it, so the test process never holds a writable fd for a fork to inherit.
Measured in `rust:1-bookworm`: 7 of 150 runs of the core suite failed this way
before the fix, every one a fake's spawn, and 0 of 150 after. Reproduce it in
docker, never on the Mac.

**A new test must be shown to FAIL first.** ROADMAP's zombie-children item
records a regression test that passed identically with and without its fix.
Break the thing under test (drop the `skip_serializing_if`, restore the old
line), watch it go red, then restore. Two tests this repo now relies on were
confirmed this way. **Restore by editing back, never `mv file.bak file`**: `mv` keeps the
backup's OLDER mtime, cargo sees an artifact newer than the source and keeps
the mutated build, and the "restored" run stays red — or, the other way
round, a mutation copied in with `cp -p` reads green. `touch` the file after
any restore before believing the next run. A mutation can also HANG a test
instead of failing it: serve's M11 (a second serve never refused) left a test
waiting on `.output()` forever. Bound every wait on a child: spawn, poll,
kill.

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
and `pnpm install` in `app/` on the pinned toolchain: `nvm use` (reads
`.nvmrc`, Node 22.23) then `corepack pnpm install` in `app/` (corepack reads
`app/package.json`'s `packageManager`, pnpm 11.5.2). **Do not let corepack pick
pnpm on its own**: unpinned, it took 12.8.1, whose native-binary layout corepack
cannot run ("no `pnpm.cjs`"), and every fresh lane fell back to a cached 11.x by
hand while CI ran 11 — local and CI on different majors. CI and release.yml read
the SAME two pins (`node-version-file`, `package_json_file`); change them there,
never in a workflow.

**`nvm use` can REMOVE pnpm from your PATH, and three things break in a row.**
nvm installs pnpm per NODE VERSION, and the version `.nvmrc` pins ships only
`corepack node npm npx` — so the very command these targets tell you to run
leaves `/bin/sh: pnpm: command not found`. Fixing that exposes the next one:
`pnpm --dir app` from the repo root cannot work either, because a bare `pnpm`
is a corepack shim and corepack fixes its version from the CWD's package.json
BEFORE pnpm reads the flag; the root has none, so it takes corepack's default
and then refuses `app/package.json`'s pin with ERR_PNPM_BAD_PM_VERSION. The
flag names the project, only the cwd reaches corepack. And fixing THAT exposes
the third: tauri runs its own `beforeDevCommand`/`beforeBuildCommand` from
tauri.conf.json, both a bare `pnpm`, so you get the same error one line later
from inside tauri — overridden per target in the Makefile, never in
tauri.conf.json, because release.yml builds through that file. CI is immune to
all three (it installs the pinned pnpm directly), which is why `make dev-app`
and `make install-app` stayed broken while every run was green. **Verify this
family with `env -i` carrying only the pinned node's bin**: a login shell that
has ever used another node version keeps that version's bin further down
`$PATH`, finds pnpm anyway, and a broken target looks fixed.

**AI profiles have a manual gate too.** Everything claude-side (does the config
swap apply, does session adoption still see `claude`, does auto-resume resume)
is invisible to the bats suite — there is no fake claude. Re-run
`docs/ai-profiles-manual-checks.md` whenever the `claude` binary is upgraded.

**pi has one as well, and its fixtures are version-pinned.** There is no fake
pi either: `docs/pi-manual-checks.md` covers launch, resume, the trust modal,
the allowance, a dead model host and Esc, and says which pi version each
fixture under `tests/fixtures/pi-*` was captured on. Re-run it when pi is
upgraded — pi's `--list-models` table has no JSON form, and a changed header
parses to an EMPTY catalog on purpose, which reads as "pi offers nothing".

**kimi has one too, and four of its habits are traps.** `docs/kimi-manual-checks.md`
pins the 2.2.0 wire log and screens. (1) Interactive kimi takes NO prompt word —
a trailing one is "unknown command" — so its opener is typed by a helper the
pane starts (`worktrees kimi-opener`), which waits for an EMPTY composer and
gives up, logged, after ten minutes. (2) In its `/model` picker, Enter writes
`default_model` into `~/.kimi-code/config.toml`; Alt+S is session-only — the
`/model` trap Claude has, one key away. (3) `kimi provider list --json` prints
the provider API key; the catalog is read from `[models.*]` alone
(`kimimodels`). (4) `--skills-dir` REPLACES skill discovery, so guidance and
the MCP server reach kimi as files in the place's own `.kimi-code/`
(`kimiplace`), each marked as ours and excluded in the common `info/exclude`.

**`grep` here can skip a source file without a word.** It treats a file with
one stray non-UTF-8 byte as binary, and `crates/worktrees-core/src/projcfg.rs`
is one: a sweep for every place a harness is named found nothing in it,
`USER_ONLY_KEYS` included. Sweep with `grep -a`. Its zsh twin: `path` is tied to
`PATH`, so `for path in …` in a probe loop empties `PATH`, and every command
after it is "command not found" — which reads as a sandbox refusing curl.

**More zsh: a probe written for bash fails quietly in it,** and the Server
Edition lanes hit these in nearly every place. `$VAR:x` is a history
modifier: `git show $M:crates/a.rs` asked for `origin/mainrates/a.rs` (`:c`),
and this close-out's own `$r:app/…` turned into an absolute path (`:a`) —
write `${M}:path`. `for x in $list` and `kill $kids` do NOT word-split, which
orphaned a stress run's `yes` hogs: use `${=list}` or `(${(f)"$(…)"})`. A word
that starts with `=` is a command lookup, so `echo =====` dies "= not found",
and so does an unquoted tmux target `=name:`. `wait` returns 127 for a job
started inside `$(…)`. macOS has no `timeout`, and its `ps` has no `etimes`: a
wait loop on it never ended and a sandbox app ran for two hours.

## Tauri app — hard-won rules

- **Commands must be `async fn`** — sync handlers run on the main thread and
  freeze the UI for every git/tmux shell-out.
- **`Builder::setup` REPLACES, and `state::<T>()` compiles for any `T`.**
  tauri 2.11.5 stores `self.setup = Box::new(setup)`, so a second `.setup(…)`
  in the chain silently discards the first. The first held core's log sink from
  #358 (v0.35.0) on, and then, in #508, `IDENT` and the `Ctx` itself. Every
  `ctx()` panicked "state() called before manage()", and #508 sat 9/9 green
  with an app that could not answer one command: nothing in CI runs the app,
  and the check that "exactly one Ctx is managed" counted the line textually,
  in the dead closure. Fixing that surfaced a second of the same kind: the poll
  still asked `state::<viewer::Viewer>()` for a registry that had moved into
  the `Ctx`. `api-boundary-check.mjs` now counts `.setup(` and refuses any
  state lookup but the `Ctx` and the window tracker. Both failures exist only
  as `[panic] … before manage()` lines in `app.log`, so after any change to
  `run()`, launch it (`sandbox.sh --app`) and grep the log; no suite will.
  A panic is ONE line, `[panic] panicked at <file:line:col>: "<message>"`,
  the message `{:?}`-escaped and cut at 500 chars (`host::panic_line`). Until
  then the message sat on the NEXT line, raw, so `grep '[panic]'` showed where
  and never what — `grep -A1` on an older log.
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
- **A reused `<div>` keeps its scroll offset through a class flip — the reset
  came from a CLAMP.** The "loading…" flip on every refresh looked like the
  cause of "the document jumps to the top"; React reuses the div and scrollTop
  survived it (Playwright Chrome + WebKit against the mock; not measured in the
  app's WKWebView). A read that caught the file mid-write did it: the stub
  clamps scrollTop, the clamp fires its own `scroll` event, and a hook that
  trusts every scroll event records 0. Reproduce with `?slowread[=ms]` and
  `__mock.editFile(path, content?)`; `scrollkeep-check.mjs` pins it.
- **xterm's search addon fails only once you SEARCH.** Its decorations need
  `allowProposedApi: true` on the `Terminal` — without it `registerDecoration`
  throws on the first ⌘F, from inside an effect, taking the pane down with it;
  the terminal looks perfect until then. It also CACHES the last search and
  re-highlights only when the term or case/regex/wholeWord changed —
  `_didOptionsChange` never looks at `decorations` — so a theme switch needs a
  `clearDecorations()` first or the matches keep the old theme's hex. Load the
  addon after `term.open(host)`, and route its calls through a guard: losing a
  search is survivable, losing the terminal is not.
- **xterm's mouse REPORTING and its SELECTION both listen on `.xterm`; the
  linkifier listens on `.xterm-screen`, its child.** So a gesture that should
  belong to a link alone (the ⌘-click on a file path) is stopped at
  `.xterm-screen` by a listener registered AFTER the linkifier's own. Both
  fire on that element (bubble, in registration order, and `stopPropagation`
  does not skip a sibling listener on the same element), so the linkifier
  still records the press it needs to activate on release, while the press
  never reaches the parent: claude gets no click report and no selection
  starts. It is the ORDER that matters, not the phase.
  The linkifier itself checks no modifier: `activate` runs on any click that
  presses and releases on the same link, so the provider must test the
  modifier — and the BUTTON: it fires on a right-button release too. The
  right-press is stopped the same way (it is reported to claude like a left
  one), and the menu answers `contextmenu` there. One more linkifier quirk:
  it re-asks for links only when the pointer reaches a DIFFERENT cell than
  the last one it saw, and keeps that cell across a `mouseleave`, so after a
  menu covers the pane, the pointer can be back on a link with no `hover`
  ever fired. The provider therefore hit-tests the event itself (`linkAt`)
  rather than trusting hover state alone. `termlinks-check.mjs` pins all of
  it.
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
  registry lock, which may never wait on a file lock. Those UNASKED saves
  write only over what this engine last wrote or read (a content hash), since
  an orphaned serve's tick overwrote a downgraded app's ring (rehearsal D7) —
  and the guard is in the save FUNCTIONS, because the exit flush went around a
  tick-only one. A write guard belongs in the writer, not its busiest caller.
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
- **`kill(pid, 0)` is a weak liveness check, twice over.** It SUCCEEDS on a
  zombie, so a test waiting for an orphaned grandchild to die waits on init —
  late, or never in a container with none (Linux CI); read `ps -o stat=` for
  `Z`. And a pid outside `pid_t`, cast, goes NEGATIVE and probes a process
  GROUP: refuse it rather than cast (containers here had `pid_max` 99999, not
  the 4194304 a comment had guessed).
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
- **Logs: two files, one per process.** The app SHIM writes `app.log`: macOS
  `~/Library/Logs/<identifier>/app.log`, Linux
  `~/.local/share/<identifier>/logs/app.log` (HOME-relative; `XDG_DATA_HOME`
  is not read, ROADMAP). It holds the shim's lines only: its `startup` line,
  `engine:` (find, start, adopt, replace, quit), the window, and the shim's own
  `[panic]`s. The ENGINE is serve, and every serve on the machine (the app's,
  one started from the CLI, a sandbox's) writes `~/.cache/worktrees/serve.log`,
  a version line at each start; `grep '\[<pid>\]'` for one of them. That file
  has every command's work, the poll, the page's `log_event` lines (tagged
  `ui:`) and the engine's panics, and it is the one Settings → Logs shows
  (`log_info` reports whichever sink its process installed, and it runs in
  serve). Persisted UI settings: `ui-state.json` in the app config dir — written
  WHOLE-BLOB by the frontend, so the backend must never write into it (its own
  update would be erased by the next settings save; that is why each dock shell
  tab's last directory lives in a separate backend-owned `shell-cwds.json`).
- **Retiring a key from `ui-state.json` breaks a DOWNGRADE.** The blob is
  saved whole, so a key the new build stops writing is erased on disk, and an
  older build that still reads it falls back to its default. 0.43's first cut
  did that to `codex_permissions` when host policy took it over: a downgrade
  showed Auto-review, WIDER than the user's Ask, and wrote it back (rehearsal
  D6). Leave a retired key in the blob; `loadSettings` carries unknown keys
  already. Its twin: **a getter that returns the EFFECTIVE value is not a
  record of what the host holds.** `get_codex_permissions` answers host > env
  > config > default, so copying its answer anywhere a migration reads turns
  env or config into host policy; copy only after a set the host ACCEPTED.
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
- **An invalid `calc()` does not fall back to the old value — it falls back to
  the INITIAL one.** A `calc()` whose custom property fails to resolve is
  invalid at computed-value time, so the declaration is dropped: `gap` becomes
  `normal` (0), not the number you meant. The feedback dialog's spacing was
  written as `calc(n * var(--base))`, the base never resolved, and padding and
  borders survived while every gap vanished — which reads as a layout nobody
  styled rather than a variable nobody defined, i.e. exactly like the bug it
  was meant to fix. Two rounds were lost to it. Prefer literal values in a
  generated sheet; if you must use `var()` in `calc()`, assert the computed
  result, never the declaration.
- **A vendored widget in a shadow root sizes itself against OUR root.** The
  Orfis SDK lays out in `rem`, and `rem` is root-relative — it reads `html`,
  which this app pins to `--ui-rem` (15px), not the 16px the widget was drawn
  for. Everything rendered ~6% tight and nothing looked broken. `rem` cannot be
  re-based for a subtree, so the sheet is restated (`app/src/feedbackStyle.ts`).
  Two more from the same dialog, both invisible to every check that stopped at
  the module boundary: `inert` leaves an element fully VISIBLE while dropping
  its events, so inerting the dialog along with the background gives a perfect
  form that refuses every click; and `.modal-scrim` is a full-screen fixed
  scrim with its own `z-index`, not a marker — a shadow-hosted dialog cannot
  paint a light-DOM class, so it marks itself with a style-free attribute
  (`[data-dialog-open]`, in `DIALOG_OPEN`).
- **When a vendored library misbehaves, diff the REFERENCE integration before
  re-reading your own copy.** The feedback form's rows had no spacing because
  `gap` lives on `.panel` and the whole `<form>` is one of its children — the
  SDK gives form rows nothing. jobmepls carries four `margin-block-start` rules
  for exactly this, with a comment calling them "worth upstreaming". Three
  rounds were spent re-deriving it from our vendored bundle; one `diff` against
  the working integration would have ended it.
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
  Second gap, unrelated, and now CLOSED: `--app` moves the CONFIG dir to
  `net.casadelvalle.worktrees.sbx`, and `app.log` used to go to the **plain**
  identifier regardless, so a sandbox wrote into your real app's log unless
  HOME was faked. `log_dir()` read `APP_IDENT`, a `const`; it reads a runtime
  `IDENT` now, set in `.setup()` from `app.config().identifier` — the same
  field `app_config_dir()` is `config_dir().join(…)` of, which is why the
  config dir already followed the override and the log did not. One line is
  still written under the default: the `startup v… PATH=…` line, emitted
  before any app handle exists to ask. serve.log is NOT isolated at all: it is
  `$HOME/.cache/worktrees/serve.log` for every serve, so a sandbox's serve
  writes into yours, told apart only by its pid. And a faked HOME does not
  isolate WebKit either: it keys storage on NSHomeDirectory, so a
  HOME-isolated run still writes the REAL `~/Library/{Caches,WebKit}/
  <identifier>`. Use a throwaway identifier and remove exactly those two dirs
  afterwards.
- **Give a sandbox drive its own tmux server, and mind the socket path.** The
  sandbox app and the serve it starts inherit `$TMUX`, so they talk to the
  user's real server unless it is unset. `env -u TMUX TMUX_TMPDIR=<dir>` on
  the `sandbox.sh --app` launch moves the app, serve and the CLI onto a
  private one, and every probe then names it with `tmux -S <dir>/tmux-<uid>/
  default`. The dir must be SHORT: a Unix socket path over 104 bytes on macOS
  fails with "File name too long", which a scratchpad path under
  `/private/tmp/claude-…/<session>/scratchpad` is. `/tmp/claude-<uid>/<x>`
  works. And `kill <app pid>` is a CRASH as far as the app is concerned: tao
  installs no SIGTERM handler, so the Exit arm never runs and the serve the
  app owns keeps running — which is the crash-leftover path, and the next
  launch adopts it (`owner: "app"` in serve.json). For two minutes: an
  app-owned serve with no client for `OWNED_IDLE_GRACE` (serve/mod.rs) stops
  itself, so a relaunch after that starts a fresh one, and a drive of the
  adopt path has to relaunch inside it. A clean quit is ⌘Q.
- **A vanished sandbox app means the human closed it.** `tauri dev` exits 0 when
  the window closes, which is indistinguishable from a clean shutdown because it
  is one; a harness kill is 143. Twice in one session that was reported as a
  crash and a relaunch started, once with a `setsid` "fix" for a problem that
  did not exist. Its state is readable afterwards — the config dir's
  `ui-state.json` shows which path was taken.
- **Never run the bundle's binary to probe it.**
  `target/release/bundle/macos/worktrees.app/Contents/MacOS/app --version` is the
  GUI entry point — it LAUNCHES a second instance instead of printing a version.
  The file beside it, `Contents/MacOS/worktrees`, is the bundled CLI and DOES
  answer `--version`; that is what `app/scripts/sidecar.sh check` probes. On
  Linux the GUI is `usr/bin/worktrees-app` and the CLI
  `usr/lib/worktrees/worktrees`, inside an AppImage or .deb that `sidecar.sh
  unpack` opens without running anything.
- **An AppImage hands its AppDir to everything the app starts — unless the
  spawn goes through `worktrees_core::childenv`.** AppRun (AppImageKit's
  classic one, which linuxdeploy bundles) and linuxdeploy-plugin-gtk's hook
  start the app with the mount (`$APPDIR`) at the FRONT of `PATH`,
  `LD_LIBRARY_PATH`, `XDG_DATA_DIRS`, `GI_TYPELIB_PATH`, `PYTHONPATH`, plus
  `PYTHONHOME`, `GTK_*`, `GDK_BACKEND=x11` and a cwd of `$APPDIR/usr`. The app
  and WebKit's children need all of it, so it stays in the app process; but a
  tmux server the app started handed it to every pane (measured: 20 such
  variables in `tmux show-environment -g` and in a pane's `env`, 23 in the
  login shell `fixup_gui_path` runs), so the user's git, ssh and python
  loaded the AppImage's libraries. **Every process the engine starts is made
  by `childenv::command(..)`**, never `Command::new` (a pty `CommandBuilder`
  gets `scrub_pty`, `term.rs`), and it is a no-op anywhere but an AppImage.
  ALL spawns, not the "user-facing" ones: the first cut listed those, and the
  Docker measurement found two login shells (`cli_binary`, pi's preflight)
  still leaking. `app/scripts/childenv-check.mjs` fails on a bare
  `Command::new` outside childenv.rs and tests. tauri's opener plugin
  (`xdg-open`) is NOT covered: it is not our spawn (ROADMAP).
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
- **An agent's MODEL is in its transcript, never its probe — and a switch is
  written at once, for both CLIs.** Claude: the newest of an assistant reply's
  `message.model` or the ``<local-command-stdout>Set model to `X` `` entry
  `/model` writes immediately. Codex: the newest `turn_context.model` or
  `thread_settings_applied` in the rollout. The CLI writes that event AT the
  switch; one VS Code rollout, where it happened to land in the same second as
  a turn, once passed for proof it waited for the next message. Sample the
  client you are building for. Two more Codex rollout traps. Its own
  sub-agents (the auto-review "guardian": `source.subagent`,
  `parent_thread_id`) write rollouts with the SAME cwd that start LATER, so
  "newest rollout for this cwd" is routinely not the user's session. And the
  file exists seconds before its 20KB `session_meta` first line does, so never
  cache a miss on a line with no trailing newline.
  Turn STATE is in the rollout too, since 0.15x: `task_started`, then
  `task_complete` (`completed_at`, epoch secs) or `turn_aborted` on Esc —
  `codex::rollout_turn`. A count over live sessions reads as "no completion
  record exists" while their turns are simply still running, which is how a
  brief once concluded the rollout was not enough. Legacy `notify` sends only
  `agent-turn-complete`, nothing on Esc; `-c` hooks stop on a trust prompt.
  "Waiting on the user" is on disk NOWHERE — an approval's rollout tail is the
  same pending tool call as a running command — so the amber dot captures the
  pane of a mid-turn session (`activity::codex_panes`) and keys on the modal's
  FOOTER being the bottom line, never on its question text, which history can
  quote. A codex session OUTLIVES codex (`codex …; exec "$SHELL"`), and a
  kill writes no `turn_aborted`, so a rollout ending on `task_started` means
  nothing until the pane is checked: `PaneList::session_runs_program` keys on
  "not a shell", never "is codex" — npm's codex runs as `node`. Anything the snapshot
  derives from agent state also needs its own re-list trigger. The poll
  re-lists on a tmux fingerprint change or every 30s, and neither a finished
  turn nor a `/model` moves tmux; `claude_activity` + `codex_models_moved`
  exist for that.
  **Codex 0.15x runs on the ALTERNATE screen and never asks for the mouse**, so
  tmux holds no history and the wheel had nothing to scroll: `term_wheel` could
  only send ↑/↓, which Codex reads as composer history. We launch it inline
  (`--no-alt-screen`, `harness::Codex::launch_args`, resume too; needs
  codex-cli ≥ 0.81.0, older ones reject it and the lane falls to the shell).
  Measured on 0.159.0: the transcript lands in tmux history, a resize clears
  and re-emits it instead of stacking copies, and the modals still end on their
  footer in a visible-screen capture (`tests/fixtures/codex-inline/`).
  Alt-screen Codex does scroll itself on PageUp/PageDown, but only a page at a
  time, behind a "Back to bottom" mode that Esc leaves.
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
   binary via include_str!). Read the section against the FINAL state, not
   bullet by bullet: one assembled from a PR stack carries bullets written for
   an intermediate PR, and v0.43.0's ipc.ts bullet said "nothing changes" for
   the app that #524, in the same release, made serve's client.
2. Bump workspace `Cargo.toml` AND `install.sh`'s `SCRIPT_VERSION`, then
   `cargo update --workspace` for Cargo.lock → PR → merge. Never a bare
   `cargo update`: it drags third-party upgrades into the release commit.
   (A piped installer cannot see its URL, so a tagged copy knows its
   tag only by carrying it; `test/misc.bats`, `make release` and release.yml
   all refuse a mismatch.)
3. `make release VERSION=x.y.z` → `git push origin main vx.y.z` (the tag runs
   publish.yml, below).
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
4. release.yml: CLI ×4 targets + SIGNED app bundles ×4 + latest.json, built,
   checked and assembled into ONE `release-dist` artifact — and nothing that
   publishes; it runs only when called. **publish.yml** (the `v*` tag) calls it
   with `dry_run: false` and the signing key, then creates the release from
   exactly that artifact; it is the one workflow holding `contents: write`.
   **release-dry-run.yml** calls the same file with `dry_run: true` on every
   PR touching the packaging (the three workflows, `tauri*.conf.json`,
   `sidecar.sh`, `app/package.json`, the Makefile, install.sh): both Linux
   arches + the native Mac app, every assertion, a throwaway updater key made
   in the job, latest.json assembled, no secrets passed — and the bundle step
   refuses the real key in a dry run. So a release runs the steps the last
   packaging PR already ran (~30 runner-minutes, ~12 wall-clock, $0 on a
   public repo). It is not `if: inputs.publish` in one file for two reasons: a
   tag push carries no inputs, so that guard reads empty, and Actions' loose
   equality makes empty == false — the real release would silently skip its
   own publish; and a called workflow can never hold more than its caller
   grants, so keeping `contents: write` out of release.yml keeps it off the
   PR path entirely. `app/scripts/release-dryrun-check.mjs` pins the split.
   A dry run that fails to START is a zero-job run that only
   `gh run list --commit <sha>` shows. The app bundles are macOS (arm64; x86_64 cross-built on arm64 and never executed)
   and Linux (x86_64 on `ubuntu-latest`, arm64 on `ubuntu-24.04-arm` — free
   for a public repo; WebKitGTK is never cross-built), each Linux one an
   AppImage + a .deb. Updater signing key: repo secret
   `TAURI_SIGNING_PRIVATE_KEY`; local backup `~/.tauri/worktrees-updater.key`
   — irreplaceable, never commit it. createUpdaterArtifacts signs the macOS
   tarball, the AppImage AND the .deb (each `.sig` over the whole file), and
   latest.json carries `darwin-*`, `linux-*` (the AppImage) and `linux-*-deb`:
   the updater asks for `<os>-<arch>-<installer>` first, so a .deb install
   gets a .deb (`pkexec dpkg -i`) and an AppImage rewrites itself.
   **Each app bundle carries the CLI for its own arch** — macOS
   `Contents/MacOS/worktrees`, Linux `usr/lib/worktrees/worktrees` (`/usr/lib/…`
   from a .deb, `$APPDIR/usr/lib/…` in an AppImage: both are
   `resource_dir()/worktrees`) — a PRIVATE sidecar for the app (it starts
   `worktrees serve` with it), never on PATH; install.sh is still how the CLI reaches
   PATH (ROADMAP, "Option A"). **Nor is it ever HANDED OUT**: running as
   serve, its `current_exe()` IS the bundle, and `profile::worktrees_bin()` —
   which every MCP entry (Claude, Codex, pi, kimi, a profile's), hook and pane
   helper is written from — took any `current_exe()` named `worktrees` as the
   user's CLI, so `mcpsetup` would have put `<bundle>/Contents/MacOS/worktrees`
   (on Linux a per-launch `/tmp/.mount_*`) into the user's GLOBAL Claude config
   (rehearsal D1). `is_bundled_cli` refuses it by path (`Contents/MacOS/`,
   `usr/lib/<x>/`) and the lookup falls through to PATH, as the in-process app
   always did; a test reads tauri.conf.json's `files`, so a moved layout fails
   it. It is the path, not `APPDIR`, because serve starts with that scrubbed.
   The sidecar lane had written this down as safe ("in the app,
   `current_exe()` is `Contents/MacOS/app`"), which was true of the GUI
   process one phase before serve ran the same core AS the sidecar: a note
   about `current_exe()` describes a PROCESS, so re-check it whenever code
   moves into another one.
   The copy gets there through tauri.conf.json's per-format `files` mappings
   (`bundle.macOS.files`, `linux.deb.files`, `linux.appimage.files`), NOT `externalBin`, which is wrong twice over:
   tauri-build copies it from the BUILD SCRIPT in every profile (a clean `cargo
   check -p app` fails: "resource path `binaries/worktrees-<triple>` doesn't
   exist"), and on Linux it lands in `usr/bin` — `/usr/bin/worktrees` from a
   .deb, and first on the AppImage app's own PATH (measured: the app's MCP
   setup then resolved `worktrees` to it). Only the bundler reads `files`, and
   it FAILS with nothing staged ("Failed to copy custom files"), so a bundle
   cannot ship without it. `app/scripts/sidecar.sh stage` builds and stages it;
   `check` runs on the .app AND the unpacked updater tarball, and on the
   AppImage and .deb (unpacked with unsquashfs at the ELF-computed offset /
   `dpkg-deb`, never run): regular executable file, arch = the target's and
   the GUI's (`lipo` / the ELF header), version = the tag, and on Linux nothing
   in usr/bin. `--version` runs only on a native one; otherwise the version is
   read as data from stale.rs's marker. The AppImage's copy is not
   byte-identical to the build: linuxdeploy's patchelf adds `RUNPATH=$ORIGIN`.
   The GUI is `worktrees-app` on Linux (`tauri.linux.conf.json`'s
   `mainBinaryName`; a .deb would otherwise install `/usr/bin/app`). tmux and
   git are never bundled: the .deb `Depends: tmux, git` and the AppImage, like
   the .app, uses the system's — release.yml asserts both.
5. Users update from inside the app (Settings → Version: CLI + app buttons)
   or by re-running install.sh.

**Rehearsing an in-app update takes one patched line.** The recipe that drove
v0.42.0 → 0.43.0 → 0.43.1 through the real updater is in
`docs/sessions/2026-10-21-server-edition/rehearsal-report.md`. Updater
`endpoints` in `--config` do NOT make "Update app" appear: its "latest" is
`latest_release_tag()`'s hardcoded github.com curl, so point that line at a
local server and set every proxy variable to a dead `127.0.0.1:9` as proof
nothing else left the machine. An in-app relaunch (`relaunch()`, an updater
install) spawns `Contents/MacOS/<exe>` DIRECTLY, not via LaunchServices, so an
isolated environment survives it. Use a throwaway identifier and HOME (WebKit
still escapes HOME, see `sandbox.sh` above), and stop everything by pid.

## Local installs

- CLI stable: `install.sh` (copies). `make install` SYMLINKS the clone's
  build — every rebuild silently becomes "stable"; don't use it for that.
- App: `make install-app` → /Applications (local builds skip Gatekeeper); on
  Linux it builds the AppImage and installs `~/Applications/worktrees.AppImage`
  (no desktop entry — `install.sh --with-app` writes that). It stages and
  checks the CLI sidecar exactly as release.yml does, so every local install
  exercises the release's path. `APPDIR=<dir>` installs elsewhere — use a
  scratch dir to inspect a build without replacing the app the user is
  running. A Mac cannot build the Linux app; a Docker `ubuntu:24.04` with CI's
  apt set plus `xdg-utils` (the AppImage bundler copies `xdg-open` in) and the
  pinned pnpm installed directly — not through corepack, which refuses the
  pin from the repo root — runs the Linux branch as is.
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
the `/close-out` skill. It lives IN the repo at
`.claude/skills/close-out/SKILL.md` (linked for Codex at
`.agents/skills/close-out`), written by `worktrees rules apply` since #496;
`worktrees rules status` says whether it is current. Do not hand-edit it — an
edit makes it stop updating. This repo's paths, gates, index and branch naming
live in `.claude/close-out.md`, which the skill reads; edit THAT file. Short
version: scratch → `~/.cache/worktrees/…`, session summary + planning tarball
→ `docs/sessions/<date>-<slug>/` + a row in `docs/sessions/index.md`
(committed), stragglers → `ROADMAP.md`, one squash-merged PR, then park the
place on its own `<place>-next` idle base (place = worktree here; the notes
below say `<tree>-next` for the same thing).

**A personal `~/.claude/skills/close-out` outranks the repo's copy.** Claude
Code resolves a name clash personal-over-project, so while the 2026-08-10
global copy (linked from `~/workspace/claude-skills`) is still installed, THAT
is what `/close-out` runs on this machine, and the repo copy is what every
other machine and session runs. Drop this paragraph once it is retired.

**A matching commit SUBJECT is not a merge check.** Squash-merge rewrites the
SHA, so `git rev-list --count origin/main..<branch>` reports every merged side
branch as unmerged and the tempting shortcut is to match subjects against
main's log. That mis-cleared `bug-fixes-codesign-local-installs` as a merged
leftover during one close-out: its subject fuzzily matched a DIFFERENT
session's close-out commit, while the branch is in fact PR #124, closed not
merged, deliberately parked and documented in ROADMAP. Prove content presence
instead — `git cat-file -e origin/main:<path>` for the paths the branch
touched, or diff the branch against main for them (main being strictly ahead
is the signal that the work landed and was built on). A parked branch and a
merged leftover look identical from the ref alone, and only one of them is
safe to `-D`.

**`gh pr merge` reports a failure it did not cause.** From a side worktree it
dies with *fatal: 'main' is already used by worktree at …* — that is `gh`'s
local checkout step, AFTER the merge landed on GitHub. Check
`gh pr view <n> --json state,mergeCommit` before retrying, or you will re-merge
a merged PR. Same shared-branch rule as below, from a new direction.
**And it takes `--delete-branch` down with it**: that runs after the step that
died, so a successful merge leaves the remote branch behind. `git push origin
--delete <branch>` it by hand, and do not read the surviving branch as evidence
that the merge did not happen — it is the same one failure, twice.

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

<!-- worktrees:rules v1 sha=5b0e980407f1d471 — managed by `worktrees rules apply`; edit outside this block, or it stops updating -->
## Working with worktrees

This repository is managed by [worktrees](https://github.com/penard-monkey/worktrees):
every branch lives in its own PLACE, a git worktree under `.worktrees/<slug>`
plus a tmux session. `(main)` is the repository root and stays on the default
branch. If worktrees launched you, you also have the `worktrees` skill: it is
newer than this section and wins where they differ (`worktrees guide` prints
it). Without worktrees installed, ignore this section.

- **Branch work:** MCP `create_worktree {branch}` or `worktrees new <branch>
  --no-attach`, then work in `.worktrees/<slug>`. Never `git worktree add`;
  never switch branches in `(main)` or in a place that is not yours.
- **Spin up a lane** (hand work to another agent): `create_worktree {branch,
  provider, model, brief}` or `worktrees new <branch> --ai claude --brief
  "<text>" --no-attach`. The brief lands in `.planning/brief.md` and is the
  whole handover: goal, deliverable, rules, and when to stop and report. Never
  type an agent command into a pane.
- **Which tool:** `list_places` (every place) · `place_status` (one place: is
  its agent busy, what is its plan) · `report` / `messages` / `wait` (talk
  between places, wait for an agent to go idle) · `send` (type into a Codex or
  pi pane; check it is not mid-input first) · `show_doc` (open a file for the
  user) · `set_note` / `set_lifecycle` · `remove_worktree` (needs `confirm`).
  Claude sessions can also `SendMessage` the place's tmux session name.
- **A place with a running agent belongs to that agent.** Do not edit its
  tree; message it.
- **Planning:** plans are working memory in `.planning/` inside each place
  (`task_plan.md`, `findings.md`, `progress.md`). Not committed.
- **Finishing:** commit, push and open the pull request from the place. After
  it merges, remove the place or park it on a fresh `<place>-next` branch. At
  the end of a work stream, run the `close-out` skill.
- **Never touch:** another place's working tree, a branch checked out in
  another worktree, a harness's own state files (`~/.claude.json`), or a
  running app you did not start.
<!-- worktrees:rules end -->
