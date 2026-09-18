# Session: reference another worktree while you type

- **Date:** 2026-09-18
- **Worktree:** `drag-drop-worktree-reference` (idle base
  `drag-drop-worktree-reference-next`)
- **Branch:** `drag-drop-worktree-reference-mcp-resources` off a freshly
  fetched `origin/main`
- **PR:** [#215](https://github.com/penard-monkey/worktrees/pull/215) →
  `470b1dc` (three commits + a merge of `origin/main`, squashed)
- **Planning files:** `planning.tar.gz` beside this summary
- **Scratch:** `~/.cache/worktrees/worktrees/drag-drop-worktree-reference/` —
  the final bats log and the two throwaway probes (`probe.py` drove the server
  over a pipe before the check script existed; `shadow.py` is the one that
  caught the uri-shadowing bug)

## Context

David: "I want to be able to reference another worktree from any tmux/claude
session by dragging and dropping it… Another useful thing may be a way to auto
complete the worktrees so maybe if I type `wt:` or `@wt` I get a list of
worktrees that I can auto-complete to reference. Would be useful when
orchestrating worktrees and using the mcp."

Two asks: a **drag** from the app's nav into a session, and an **autocomplete**
while typing. This session shipped the autocomplete half and deliberately left
the drag unbuilt — see Decisions.

## What shipped

`worktrees mcp` now publishes every place as an MCP **resource**, so
`@worktrees:place://bug-fixes` completes from claude's own `@` menu and expands
into that place's status.

- `crates/worktrees-core/src/model.rs` — `PlaceRef`: a place named and located,
  with nothing that costs a git call per place.
- `crates/worktrees-core/src/project.rs` — `place_index()` (one
  `git worktree list --porcelain` + one `read_dir`) and `place_one(&PlaceRef)`
  (the full `Place` for ONE place, so a prompt with three mentions does not pay
  three `ls` fan-outs).
- `crates/worktrees-cli/src/mcp.rs` — `resources/list`, `resources/read`,
  `resources/templates/list`, per-error JSON-RPC codes (`-32002` for
  resource-not-found), `capabilities.resources.listChanged: true`, a `ready`
  flag set by `notifications/initialized`, a single-writer `emit()`, and
  `spawn_list_watcher()` pushing `notifications/resources/list_changed`.
- `scripts/mcp-resources-check.py` + `make test-mcp`, wired into `make check`
  and into CI's install job (both OSes).
- Temporary debug logging to `~/.cache/worktrees/mcp-debug.log`, with its
  removal enforced by a test — see Follow-ups.

## Decisions

**The reference is an MCP resource, not a new syntax.** Claude Code already
accepts MCP resources as an `@`-completion source and fuzzy-ranks a resource's
`name` above its uri (Fuse, `name` weight 3), so making `name` the slug means
`@bug-fix` finds a place without typing the server prefix. This server
advertised tools only, so `resources/list` + `resources/read` was the whole
gap — no app changes, and it works in any terminal, not just the app.

**The first recommendation was wrong and was reversed.** It proposed "one
vocabulary, two producers" — the drag and the autocomplete both inserting the
resource uri. That makes both inherit the resource path's worst properties
(escaping, staleness, the note-injection surface, cost) for a benefit only one
needs. The revised plan: the drag should insert a **quoted absolute path**
(`@"/…/.worktrees/bug-fixes"`), which needs no server-name discovery, no
staleness handling and no sanitising, works cross-project, and degrades to
something the model can still act on. That half is NOT built; it is in ROADMAP.

**Membership, not state, drives the push.** The client caches the resource list
and invalidates only on `list_changed`, a reconnect, or a fetch error — and
Claude Code does not periodically ping a stdio MCP server, so there is nothing
to piggyback on and a thread is required. The watcher compares a
**non-recursive** `read_dir` of `.worktrees/` plus the sidecar's stamp. If it
noticed state, every `git add` in every worktree would notify every session in
the repo; content is re-read per mention anyway, so state does not need pushing.

**A version gate, not a date, for removing the debug logging.** A date bomb
fires on whatever unrelated PR is open that morning and — the disqualifying
part — passes SILENTLY on a runner with no `date`.

## Dead ends / gotchas

**`place://(main)` completes in the picker and then resolves to nothing.** The
client applies TWO different rules and they disagree: the menu's token charset
`/^@[\p{L}\p{N}\p{M}_\-./\\()[\]~:]*/u` admits `(` and `)`, but the submit-time
extractor `/@([^\s]+:[^\s]+)\b/g` ends on a word boundary and backtracks
`place://(main)` to `place://(main`, which is in no list. `%` is outside the
menu charset, so percent-encoding is not an escape hatch. `slugify` in core is
`s.replace('/', "-")` and nothing else, so every git-legal branch character
reaches a slug — the sanitiser has to be narrower than either rule.

**The first collision fix re-created the bug it existed to close.** Suffixing
naively gave dirs `wip`, `wip-`, `wip-2` the uris `wip`, `wip-2`, `wip-2-2` —
so `place://wip-2` named the dir `wip-`, and a model asking for the obvious uri
got the wrong place silently. Also unstable: creating `wip` renumbered `wip-`.
Fix is two-pass — every slug needing no sanitising reserves its own name first.
Caught by a throwaway probe against the real binary, not by a test.

**`is_ascii_alphanumeric` was stricter than either client rule.** Non-ASCII
slugs collapsed: `функция` → `place://place`, `日本語` → `place://place-2`,
renumbering whenever a sibling appeared. Both rules admit Unicode mid-uri; only
the LAST character must be ASCII, because JavaScript's `\b` is ASCII even under
`/u`.

**The debug log was 100% test pollution, and the feature's only deliverable was
already drowned.** Measured: 11 lines, 7 `repo=repo` from the check script's
fixture and 4 `repo=wt-mcp-caps-<pid>` from a unit test, zero from a session —
and `test/mcp.bats` drives the real binary with the real `HOME`, so every
`make test` added more, CI included. ROADMAP tells whoever removes this to read
the log first; they would have found nothing but fixtures. Three fixes:
`cfg!(test)` early-return in `dlog`, `WORKTREES_MCP_DEBUG=0` in bats, and the
check script redirecting the log into its own tempdir — where it now ASSERTS
the contents, turning pollution into the only coverage that path had.

**`eprintln!` PANICS on EPIPE**, because Rust ignores SIGPIPE. Every other
`eprintln!` in `mcp.rs` is an error path taken once; the new startup banner ran
on every launch, so a client that pipes stderr and closes the read end would
have killed the server at startup, every time.

**`@`-mention expansion cannot be tested headlessly, and the differential is
what tells you so.** `claude -p '… @worktrees:place://x …'` does not expand —
but neither does `-p` with a plain `@CHANGELOG.md`. Without that control the
run reads exactly like a broken resource. This cost one false alarm before the
control was run.

**A test that could not fail.** The empty-override case asserted `is_some()`,
true whether or not the guard existed. Only the deliberate mutation exposed it;
it now compares against the default path.

**Env mutation in tests is a race, not a style question.** `temp_env` set
process-global vars while cargo's thread pool ran, so a concurrent `dlog` could
observe another test's `WORKTREES_MCP_DEBUG=0`. Removed entirely by making the
path rule a pure function of its three inputs.

**`"0.9.0" < "0.26.0"` is FALSE.** The obvious string compare in the removal
gate would have stopped firing once a minor went past 9 — silently, which is
the one failure a reminder cannot have. Parsed numerically, with a test pinning
exactly that.

**Two wait-loop bugs read as CI failures.** `gh pr view --jq` on a bad field
errored, `2>/dev/null` swallowed it, empty output read as "zero pending" and
the loop exited on the first iteration; later, `gh pr checks` exiting non-zero
for "no checks reported yet" read as "finished". Both times an error looked
exactly like success. (A third suspect was ruled out: `pipefail` is off in this
shell, so the `grep -q` SIGPIPE trap CLAUDE.md warns about was dormant here —
confirmed by demonstrating it returns 8 under `set -o pipefail`.)

**The CHANGELOG merge hazard is real and nearly recurred.** The merge-base
(`9ce5a77`, the v0.24.0 release) had ZERO `## [Unreleased]` sections because the
release consumed it, and both sides independently re-created one — the exact
shape that produced the earlier two-header incident, where an entry ships in no
release notes at all. Resolved to one header with both bullets; verified by
header count across base/ours/theirs/merged (0/1/1/1) rather than by eye.

## Verification

- Gates on the merge commit: `make test` exit 0, `1..335`, 335 ok / 0 not ok;
  `make lint`; core 315; cli 14; app 58; `cargo check -p app`; `make test-mcp`;
  `ls --json` byte-identical to the shipped binary. CI 9/9 on both OSes.
- Every new test was shown to fail under a deliberate mutation: the trailing-char
  trim, the collision dedupe, the watch signal noticing in-worktree file mtimes,
  `-32002` → `-32602`, `listChanged` true → false, the naive single-pass
  `uri_map`, the ASCII-only charset, the malformed-uri code, the version gate,
  the numeric version compare, the log rotation, and the pollution mute (4 lines
  into the real log without it, 0 with).
- `scripts/mcp-resources-check.py` fails with 8 failed expectations against the
  shipped v0.24.0 binary — that is how it earns trust.
- **Against a real claude 2.1.276:** it lists all 13 places, and reading
  `place://bug-fixes` returned "branch `bug-fixes-next`, clean, 1 behind
  `origin/main`, tmux `worktrees-bug-fixes` (up)" in 158ms for one place.
- Three fable reviews (design, build, pre-merge) plus one verifying the conflict
  resolution. Each finding was re-verified against the binary or the source
  before being acted on; one was pushed back on (the check script HAD been shown
  to fail, against the shipped binary) and the fair half of it fixed.

## Follow-ups

In ROADMAP: the drag half (path mentions), removing the debug logging at
v0.26.0, the `-N` suffix's remaining dirty-vs-dirty instability, the two
`safe_uri_part` Unicode edges, and a guard against more than one
`## [Unreleased]`.

The manual check that no suite here can do — typing a real `@` in an
interactive session — is in `docs/ai-profiles-manual-checks.md` and has NOT
been run yet.
