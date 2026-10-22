---
title: server-edition — the desktop app becomes a client of worktrees serve (v0.43.0)
---

# server-edition: one engine per host, shipped as v0.43.0

- **Dates:** 2026-10-16 → 2026-10-21. `(main)` launched every lane against a
  brief, relayed the user's decisions and clicks, and merged the stack.
- **Places** (each branch is the slug with its first `-` as `/`, except
  `release-v0.43.0`): `feat-pretty-ui-server` (the original plan, no code);
  `feat-server-edition` (#508), with `fix-argv-positionals`,
  `fix-host-policy`, `docs-statics-audit` and `chore-server-edition-sync-main`
  folded into it; `feat-worktrees-serve` (#516); `feat-serve-transport` (#515);
  `feat-app-serve-client` (#524); `feat-engine-auto-reconnect` (#525);
  `feat-sidecar-bundle` (#513); `fix-atomic-write-no-follow` (#523);
  `fix-kimi-usage-flaky-test` (#522); `rehearse-server-edition-update` (no PR;
  two local commits, never pushed); `release-v0.43.0` (#526).
- **PRs**, squash commits on main in merge order:
  [#522](https://github.com/penard-monkey/worktrees/pull/522) `e40e5cd` and
  [#523](https://github.com/penard-monkey/worktrees/pull/523) `498306f`
  (merged first, by the user); then the stack at once:
  [#508](https://github.com/penard-monkey/worktrees/pull/508) `78b6009`,
  [#516](https://github.com/penard-monkey/worktrees/pull/516) `80a9a77`,
  [#515](https://github.com/penard-monkey/worktrees/pull/515) `3d4259f`,
  [#524](https://github.com/penard-monkey/worktrees/pull/524) `cac41a4`,
  [#525](https://github.com/penard-monkey/worktrees/pull/525) `8f65acd`;
  then [#526](https://github.com/penard-monkey/worktrees/pull/526) `5a823aa`.
  [#513](https://github.com/penard-monkey/worktrees/pull/513) was
  squash-merged INTO `feat/engine-auto-reconnect` (`73d6cb3`), so its content
  is inside `8f65acd`. Follow-on:
  [#527](https://github.com/penard-monkey/worktrees/pull/527) (the git-init
  card's ✕, and Escape) and `fix/test-exec-etxtbsy` (not yet pushed).
- **Release:** `v0.43.0`, tagged by the user; publish run 39717798280 green.
- **Planning files:** `planning.tar.gz`, one directory per place holding its
  `.planning/` (briefs included); `feat-pretty-ui-server/` holds that place's
  root `task_plan.md`, the original Server Edition plan.
- **Rehearsal:** [`rehearsal-report.md`](rehearsal-report.md). Its evidence,
  bundles and HOME stay in
  `~/.cache/worktrees/worktrees/rehearse-server-edition-update/`.
- **Scratch:** each lane's in `~/.cache/worktrees/worktrees/<place>/`; the
  lane drafts this summary condenses in `…/close-out-server-edition/`.

## Why

A terminal does not cross a network to a phone, so the engine has to live in
a server the app talks to (`feat-pretty-ui-server`'s plan,
`docs/proposals/server-edition.md`). Phase 0 pulls the command layer out of
Tauri; Phase 1 hosts it behind an authenticated loopback socket and makes the
desktop app its first client. Every phase shipped in one release.

## What shipped

**#508, Phase 0: the command layer leaves the app.**
- `crates/worktrees-api` (no tauri): `Ctx` replaces AppHandle + State, `Host`
  carries events, `ByteSink` replaces `Channel<ArrayBuffer>`, `log` is a
  process sink. All 149 of the app's commands moved behind it (main's five
  kimi ones included; only `install_app_version` and `set_zoom` kept bodies in
  the shim), plus the new `get_codex_permissions`: 150.
  `app/src-tauri/src/lib.rs` went from 11,288 lines to ~2,090 (606 by the
  release). The registries became fields of one
  `Ctx`; the viewer, docs server, editors and `fixup_gui_path` moved whole.
- 155 unit tests moved with their code; new `crates/worktrees-api/tests/
  headless.rs` and `app/scripts/api-boundary-check.mjs`. CI tests the api
  crate on both OSes, and release.yml's docs-server gate moved from `-p app`
  (0 tests) to `-p worktrees-api`.
- Folded in: **R3** (OWASP M3: `worktrees_api::argv::Argv`, `--` honoured by
  core's parsers, `safe_arg`/`branch_arg` at the boundary); **host policy**
  (OWASP M2: Codex permissions and pi trust in `host-policy.json`, changed only
  by a click; the terminal-history switches wait for settings to load);
  **Appendix B** (every process-global static, ruled); **main's kimi
  harness**, ported into the extraction.
- Found on the way: a second `.setup()`; a runtime `IDENT` for the sandbox's
  app.log; a clock-scripted forced-ask test; two raw NULs; `doctor --kimi`
  stopping at `--`. D6: `settings.ts` keeps the two host-policy keys in
  ui-state.json, copied after a set the host ACCEPTED (`hostpolicy-check.mjs`
  rule 6).

**#516, `worktrees serve`** (`crates/worktrees-cli/src/serve/`).
- `mod.rs`, `policy.rs` (loopback `Host`, `Origin` allowlist, a missing one
  refused), `conn.rs` (hyper upgrade → tungstenite frames, handshake v1,
  connection caps and byte budgets: OWASP H1, M1), `dispatch.rs` (150
  commands, one macro table, an `Exposure` each, a serial lane per stream,
  `catch_unwind` per request), `wire.rs`, `servefile.rs` (serve.json 0600,
  refused while a live pid holds it).
- `worktrees_api::host::{boot,start,stop}`, the wiring both hosts call.
  D2: `Host::has_audience`, so a `worktrees show` waits for a client.
- OWASP L1: both logs 0600 (rotation too), one write per line, one escaped
  `[panic]` line, controls and bidi escaped.
- Engine fixes: `shell_open` refuses a non-place slug; `term_open` and
  `tune_session` target exact names; a dock shell outlives its client.
- `serve-dispatch-check.mjs`, `server-stack.sh`, `serve_bin.rs` (12
  subprocess tests).

**#515, one door.** `app/src/ipc.ts` is the only import of
`invoke`/`Channel`/`listen` (25 files swapped). `app/src/ipcws.ts` is the
socket client: nothing is sent until serve's proof passes
`crypto.subtle.verify`, versions must be equal, channels are ordered by `seq`
and released on `end`, and an oversize request is refused client-side.
`pnpm dev:serve` serves the token same-origin, for development only.
`transport-check.mjs` holds the client against serve's own Rust.

**#524, the app is serve's client.**
- `app/src-tauri/src/engine.rs`: observe serve.json → pure `decide()` → act.
  Start when nothing usable is there; refuse an unproven port holder and kill
  nothing; SIGTERM and replace another version, never SIGKILL; use ours, or
  adopt an `owner: "app"` leftover. `lib.rs` keeps the six `ClientLocal`
  commands, `engine_connect` and `raise_window`.
- `engineDoor` (one routing table) and `EngineGate.tsx`; an `app:open-doc`
  with nobody listening is parked for 10 s.
- serve: `--owner app`; `open_docs_viewer` is an Engine row; an app-owned
  serve with no client for 120 s stops itself (D7); D7's dock saves write only
  over what this engine last wrote or read (`term.rs`); D3 (`winstate.rs`).
- `engine-route-check.mjs`, `docs/engine-client-manual-checks.md`.

**#525, reconnect on its own.** `app/src/reconnect.ts`, a pure policy on an
injected clock: 0.5 s·2ⁿ capped at 30 s with equal jitter, ≤3 starts per
sliding minute, owned vs unowned, a hidden window defers. `engine.rs` adds
`start: false`, the `owned`/`started` report and `settle()` (up to 10 s for a
pid still alive on a closed port); `reconnect-check.mjs`.

**#513 (inside #525): the CLI as the app's private sidecar.** macOS
`Contents/MacOS/worktrees`, Linux `usr/lib/worktrees/worktrees`, via
tauri.conf.json's per-format `files`; `app/scripts/sidecar.sh`, shared by
release.yml and `make install-app`; Linux AppImage + .deb for x86_64 and arm64;
`worktrees_core::childenv` for every spawn; three release workflows
(`release.yml`, `publish.yml`, `release-dry-run.yml`); D1's
`profile::is_bundled_cli`.

**#522, #523.** `kimi_usage.rs` takes its kill `GRACE` as a seam, tested
with a patient grace and a SIGTERM-deaf fake. `worktrees_core::fsx::
write_atomic` (random temp, `O_CREAT|O_EXCL|O_NOFOLLOW`, fchmod, rename) for
every write a cloned repo can reach; `write_brief` refuses a linked
`.planning`; `fsx::tests::no_source_builds_its_own_temp_name` guards it.

**#526, v0.43.0**, with one note fixed (the ipc.ts bullet said "nothing
changes"). **The rehearsal** drove the real in-app updater 0.42.0 → 0.43.0 →
0.43.1 on real bundles under a throwaway identifier and HOME and found D1–D7;
all but D4's wording were fixed before the tag.

## Decisions

- **One release for the whole stack** (the user): fix D1–D3, D6, D7 first;
  skip D4/D5, which bite only someone running an intermediate PR.
- **Merge the stack at once, by squash.** GitHub's native stack evaluates each
  PR against main; an atomic squash leaves no intermediate main that fails.
- **After a branch was pushed, cascades were signed merges, not rebases,** so
  no force-push. A standing grant covered the fix cascade's pushes only; the
  release branch, PR and tag were each asked for.
- **Kimi shipped without its real-app walk-through.** Raised three times; the
  user tagged without it.
- **One engine process per host, with a mandatory version handshake.** Two
  `Ctx`s would be two polls, two shell sets, two writers.
- **The log is a process sink, not part of `Host`:** 150 call sites, and none
  varies by it. **Bodies are sync, wrappers async.**
- **hyper does HTTP; tungstenite only frames.** `accept_hdr_async` writes an
  HTTP error when it refuses, and a refusal must say nothing; a hyper `Err`
  closes with 0 bytes.
- **serve refuses a missing `Origin`; `docserver::origin_ok` allows it,** on
  purpose: every serve client is a webview or a browser, and both send one.
- **Handshake v1 replaced v0,** whose first frame was the token: whatever held
  a dead server's port received it.
- **`ClientLocal` rows are refused, never run on the host:** `copy_text`
  there writes the SERVER's clipboard.
- **A serial lane per stream:** a task per frame turned 26 keystrokes into
  `0124356789…`. No client-side workaround, which would have hidden it from
  the phone client.
- **A module door, not a WS-backed `__TAURI_INTERNALS__`,** which would put a
  man-in-the-middle on every plugin's traffic in production. Plugins stay
  direct imports: they act on the window's machine.
- **No in-process fallback** (a silent embedded engine is a second one).
  **Refuse an unproven port holder, never kill it; adopt an `owner: app`
  leftover; never signal an unowned serve.**
- **No App before the first connect:** `loadSettings` would fall back to
  DEFAULTS and a later save would write them over the real settings.
- **D2 holds a `show` only while nobody is connected** and keeps v0.42.0's
  30 s expiry; the client parks `app:open-doc`, never a state event (stale on
  arrival).
- **D6 copies only after a set the host accepted.** The getter returns the
  EFFECTIVE value (host > env > config), so a load-time copy would turn env
  or config into host policy.
- **D7's guard sits in the save functions, not the tick:** the exit flush
  went around a tick-only guard.
- **The reconnect policy lives once, in TS,** testable on a virtual clock;
  Rust obeys `start: false` and reports the two facts the page cannot see.
  The first retry is 0.5 s, not 0, because a closing serve is still exiting.
  A manual press may start one serve, and trips the limit ((main), Q1).
- **`files` mappings, not `externalBin`:** tauri-build copies an externalBin
  in every profile (a clean `cargo check -p app` exits 101), over
  `target/release/worktrees` with a fresh mtime, and on Linux into `usr/bin`,
  first on the AppImage's PATH.
- **childenv covers every spawn,** not a list: Docker found two login shells
  the first list missed. **D1 is a path rule,** not an env marker (it would
  reach every pane) or `APPDIR` (scrubbed before serve starts).
- **Three workflows, not `if: inputs.publish`:** a tag push has no inputs,
  and empty == false would skip the real publish.
- **fsx: one helper plus a guard,** not 18 patches; `O_EXCL` on a random
  name, not lstat-then-write; skip a taken name, never remove it.
- **#522: a seam, not a bigger constant;** any fixed grace still times the
  scheduler.
- **The rehearsal used the real updater,** at the cost of one patched line,
  because only that path drives Exit → restart and the bundle rename.
- **`cargo update --workspace`** for the release's lock bump, never bare.

## Dead ends / gotchas

**Stacks and merges**
- **A native stacked PR loses its linear history when main moves.** After
  #522/#523, #508 went DIRTY and every PR above read `unknown`, with no CI;
  close/reopen did nothing and a base change is refused. Fix: merge main into
  the BOTTOM branch, then each branch into the one above.
- **A clean merge did not compile.** #513 dropped a `Command` import a #522
  test used (E0433). `cargo check --workspace --all-targets` finds it in
  seconds; plain `cargo check` skips test code.
- **"Keep both" on two blocks appended at the end of `mod tests` drops a
  brace:** git hoists the shared `    }` out of both ("unclosed delimiter").
  Two lanes hit it in the same merge pair.
- **Merging main into a branch that MOVED code** conflicted in three files
  that no longer held it. `checkout --ours` was safe only after `git diff
  <base> origin/main -- <file>` showed main's whole delta was the fix being
  ported by hand; the stale `OWN_TEMP_NAMES` rows failing proved the port.
- **The #513 merge had two conflicts git never flagged:** a bare
  `Command::new` for serve (childenv-check caught it) and childenv-check
  reading `serve/tests.rs` as product code.
- **`git merge … -F -` does not read stdin** (rc 129); the checks after it
  examined the unmerged HEAD.

**The app**
- **Tauri keeps only the LAST `.setup()`.** #508 sat 9/9 green while every
  `ctx()` panicked; the fix surfaced a `state::<Viewer>()` panic of the same
  kind. Only running the app showed either.
- **D1 sat in the sidecar lane's findings marked "leave it":** "in the app,
  `current_exe()` is `Contents/MacOS/app`" was true of the GUI process and
  false once serve ran the same core AS the sidecar.
- **D6:** 0.43 dropped two keys from the whole-blob ui-state.json, so 0.42,
  which reads only that, reset Codex from Ask to the WIDER Auto-review.
- **A client that holds frames until the proof passes hands them on
  synchronously as it passes:** subscribing after `await ready` lost a `show`
  riding right behind `ok`.
- **A microtask hand-off reached StrictMode's throwaway listener** (its
  cleanup is a microtask too); `setTimeout(0)` works.
- **The shim's probe authenticates, so serve counts it as an audience for a
  few ms;** a `show` released then is lost. Open (ROADMAP).
- **"No retry after kill -9" was a fully covered window:** WKWebView reports
  `hidden` under occlusion and the retrier deferred by design.
- **Three starts in 53 s tripped the crash-loop limit** mid-drive.

**serve**
- **A fail-first mutation HUNG its test:** `.output()` on a serve that never
  exits waits forever. Spawn, poll and kill within a bound.
- **A SIGTERM between writing serve.json and installing the handler** killed
  serve with no exit status; handlers go first (race test red 6/6 before).
- **`kill(pid, 0)` with a pid outside `pid_t`, cast negative, probes a
  process GROUP.** Containers had `pid_max` 99999, not the guessed 4194304.
- **macOS:** `set_read_timeout` returns EINVAL on a reset socket; `ps` has no
  `etimes`, so a wait loop never ended and a sandbox app ran 2 h.
- **CI's macOS rust job has no tmux;** a `Drop` that `expect`ed it panicked.
- **Two comments claimed "measured on macOS 15 and ubuntu-latest"** and
  neither run had happened; re-measured before commit.

**Checks that read source**
- **Cutting a Rust file at its FIRST `#[cfg(test)]`** is not cutting off
  `mod tests`: a test-only `TEST_PANIC` above dispatch.rs's table emptied
  three checks' slice. Cut at the `#[cfg(test)]` that heads `mod tests`.
- **fsx's line scan cannot see `#[cfg(test)] mod tests;`,** so
  `serve/tests.rs` needed an allowlist row (and childenv-check a rule).
- **A gate summary counted `^FAIL`; transport-check prints `not ok`:**
  `fail=0` on a red run. Only the rc caught it.
- **transport-check exited 13 silently** under a mutation that left an await
  pending; every door await now races `settle()`.
- **The test-mover split items on a naive brace count** and swallowed 669
  lines (braces in strings count); reset, not patched.
- **Five drift checks broke on the move;** dnd threw a TypeError on an empty
  regex match instead of failing.
- **Vacuous tests:** a "maps are shared" test asserted on `pi_tick` output,
  nothing either way; D7's fixture went through `write_scrollback`, which
  records "seen"; dropping `#[serde(default)]` stays green for an `Option`;
  a stray-temp check looked for `.tmp-`, which new names never contain; a test
  forced failure with a dir at the FIXED temp name, red for the wrong reason.
- **Allowlisting a SHARED helper line** (`materialize::tmp_beside`) let any
  future write borrow the name; it is inlined now.

**Packaging and CI**
- **`echo "x=$(jq …)" >> "$GITHUB_OUTPUT"` swallowed jq's failure:** empty
  matrices, zero jobs, a green run.
- **A Docker "after" binary was byte-identical to "before":** copied sources
  were older than the outputs, so cargo rebuilt nothing. `cmp` first.
- **A stale gitignored `app/dist`** turned two checks red after a merge.
- **#526's Linux job failed once with `"spawn"`:** ETXTBSY. libtest's
  threads fork while another test holds its fake binary open for writing; 7 of
  150 stress runs red before, 0 after (`fix/test-exec-etxtbsy`).

**Timing (#522)**
- **The CI timestamps named the cause:** ≥2.07 s from the previous sorted
  test's finish to FAILED is the 2 s grace plus ~70 ms, not a lost trap (ms).
- **CPU hogs do not reproduce it on macOS** (0 of 400; they are demoted). The
  test binary under `taskpolicy -b` did (17/400); a slowed `touch` shim made
  it deterministic.
- **A stress run inside the gate slot held it ~82 min** while the machine
  slept, printing nothing.
- **`kill(pid, 0)` succeeds on a zombie;** an orphaned grandchild waits for
  init, which a container may not have.

**Rehearsal and release**
- **A `--config` endpoint cannot make v0.42.0's "Update app" appear:** the
  button's "latest" is `latest_release_tag()`'s hardcoded github.com curl.
- **WebKit ignores `$HOME`,** writing the real `~/Library/{Caches,WebKit}/
  <identifier>`. **tauri's restart spawns `Contents/MacOS/<exe>`** directly,
  not via LaunchServices, so an isolated env survives an update.
- **"REHEARSAL-3 missing" was ↑/↓ recalls with no Enter.** Read the ring
  before calling scrollback lost.
- **serve logs `mcp_status` only when a client calls it,** so checking D1
  needed a handshake-v1 client, plus a contrast run to show it not vacuous.
- **Disk:** six merged lanes' target dirs left 16 GiB free; `cargo clean`
  brought it to 62. A lane reading `df` across that minute blamed APFS
  purgeable space: re-read it before acting on one reading.
- **zsh, in nearly every lane:** `$list` and `kill $kids` do not word-split;
  `echo =====` dies; `$M:crates/…` is a history modifier; an unquoted tmux
  target `=name:` dies; `wait` gives 127 for a job started in `$(…)`; no
  `timeout`; a scratchpad tmux socket path passes the 104-byte limit.

## Verification

- **Release gates** on a fresh release build: bats 538/538, core 898 (+2
  ignored), cli 133, api 199, app `--lib` 36, tsc, `cargo check -p app`,
  lint, test-mcp, test-real-tmux 5/5, all 48 `*-check.mjs` from the root.
- **CI:** #526 green after one re-run (the ETXTBSY flake); release-dry-run
  green on #513 (×3), #525 and the release branch; publish run green.
- **Fail-first:** every new test and guard seen red, with mutation matrices
  per lane (serve M1–M12, transport 41, reconnect 12, term.rs 10, ipc.ts 7,
  D1 5), each restored by editing back and `touch`.
- **Live:** `sandbox.sh --app` driven by PID on a private tmux server (start,
  adopt, replace a 0.42.0 serve, use an unowned one, kill -9 → retry, the
  crash-loop stop, D7's self-stop at +122 s; 0 `[panic]`); Chrome and a Node
  driver against `server-stack.sh` (an impostor on a dead serve's port got
  only `hello`); headless Chromium for the reconnect banner; #523's victim
  files on 0.42.0 vs the fix; Docker ubuntu:24.04 for the Linux bundles (9/9
  steps, the AppImage env leak from 20–23 variables to 0, signatures
  verified); D1 on the real binary, inside and outside a bundle path.
- **The rehearsal:** the real updater 0.42.0 → 0.43.0 → 0.43.1, clean quit
  ×3, crash then adopt, both downgrades, mixed v0.42.0 CLI/MCP both ways.

**NOT done**
- **The kimi real-app walk-through** (`docs/kimi-manual-checks.md` §10).
  v0.43.0 shipped kimi without it.
- **The manual client checklist** (`docs/engine-client-manual-checks.md`) by
  a person, on a bundled build and on Linux. Nothing was clicked in WKWebView,
  "Reconnect now" was never pressed there, and the bundled app on
  `tauri://localhost` never ran.
- **A re-run of the rehearsal against the published v0.43.0.** D1–D7's fixes
  have only main's tests as witnesses, never a real update.
- Also not run: a Linux real-app drive or in-app Linux update; the x86_64 Mac
  bundle (cross-built, never executed); D6's real 0.43 → 0.42 → 0.43 trip
  (mock only); the moved dock-save test shown red in its new crate; Windows.

## Follow-ups

Added to ROADMAP (each links here): re-run the rehearsal on the published
release; the client checklist on a bundled build and Linux;
`kimiplace::sync_file` through a tracked `.kimi-code` link; the CLI updater's
pid-named installer in a shared `/tmp`; the shim's probe counted as an
audience; glibc floor and app.log's `XDG_DATA_HOME`; `term_resize` with no
`.catch`; transport-check's one-line-duplicate blind spot; the fsx guard and
a file-level `#[cfg(test)] mod x;`; N2 (no provider ignores `ai_cmd =
none`); and N3 (`claude_status` twice per "window visible"). The kimi item is
reworded; Option A says `files`. The published notes' "149 commands" vs
"all 150" was flagged as a contradiction; counted, it is not one: main had 149
before #508, and serve's table is those plus `get_codex_permissions`, as the
`### Changed` bullet says.

For `(main)`: merge #527; open a PR for `fix/test-exec-etxtbsy`; ship
`docs/server-edition-phase2` refreshed against what shipped, noting that the
proposal's `term_*` id-ownership fix is no longer latent (the app reconnects
since #525) and that a non-Tauri client cannot use the plugins (dialog,
opener, `getCurrentWindow`, updater). Already tracked in
`docs/proposals/server-edition.md`: R1 per-object authorization, `HostAdmin`
rows, a per-device rate limit (a local flood still locks NEW clients out, 0
of 49 on macOS), host-side scrollback/history/fetch toggles, the viewer's
loopback URL, and `Broadcast` sending `app:open-doc` everywhere.
