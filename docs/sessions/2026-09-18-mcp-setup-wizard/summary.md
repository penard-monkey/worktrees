---
title: "Wiring Claude's MCP server up from the app"
---

# Wiring Claude's MCP server up from the app

- **Date:** 2026-09-18
- **Working tree:** `.worktrees/mcp-setup-wizard`
- **Branches:** `mcp-setup-wizard-install` (feature), `mcp-setup-wizard-close-out` (this archive)
- **PR:** [#218](https://github.com/penard-monkey/worktrees/pull/218) — squash-merged as `c005220`
- **Release tag:** none — sits in `[Unreleased]`
- **Planning files:** none. The work was scoped in one design pass in-thread and
  built straight through; no `task_plan.md` / `findings.md` / `progress.md` were
  created, so there is no `planning.tar.gz` beside this summary.

The ask: when someone installs the app and the worktrees MCP server is not
registered with their claude, offer to wire it up — installing it for them where
possible, telling them how where not, and putting the control in Settings for
anyone who dismisses the offer.

## What shipped

**The engine** — `crates/worktrees-core/src/mcpsetup.rs` (new). Holds the entire
rule set: what counts as installed, which scopes are read, which single scope is
written, and how the write happens. Both the CLI and the app call it, so the two
surfaces cannot disagree about a verdict.

- `status(repo: Option<&str>) -> Status` — parses `~/.claude.json`, consults the
  local/project scopes when a repo is in hand, and asks whether the *effective*
  AI profile already exposes the server.
- `install(repo, mutations)` / `uninstall(repo)` — shell out to `claude mcp add`
  / `claude mcp remove`, deadline-guarded, and answer with the **re-read** status
  rather than an exit code.
- `command_line()` — one renderer for the `claude mcp add …` line, used both for
  what is shown on screen and for the argv `install` actually runs, so the two
  cannot drift.

**The CLI** — `worktrees mcp --status [--json]`, `--install [--read-only]`,
`--uninstall` (`crates/worktrees-cli/src/mcp.rs`, dispatched in `main.rs`). All
three are hoisted *above* the git guard.

**The app** — `mcp_status` / `mcp_install` / `mcp_uninstall` in
`app/src-tauri/src/lib.rs`; `app/src/McpPanel.tsx` (new) holds both surfaces; a
**Claude** category in `app/src/SettingsSheet.tsx`; a dismissible card on Home in
`app/src/App.tsx`; `mcp_nudge_dismissed` in `app/src/settings.ts`; styles in
`app/src/App.css`; both commands mirrored statefully in `app/src/mock/install.ts`
with a `?mcp=<state>` query knob.

**A fix that the feature forces** — `worktrees mcp` now serves with no project
instead of exiting. See *Dead ends* below.

## Decisions

**We read `~/.claude.json`; we never write it.** That file is claude's own live
state — onboarding flags, caches, per-project conversation history — and every
running session rewrites it whole. A read-modify-write from here would silently
drop whatever a live session wrote in between, and it would be *someone else's*
data that went missing. All writes go through `claude mcp add -s user`, which is
claude's own supported entry point into its own file. This is the same discipline
the app already keeps for `ui-state.json` (the side that does not own a
whole-blob file never writes into it), pointed one level outward.

**Detection parses the file rather than asking `claude mcp list`.** That command
health-checks each server by *launching* it in the current working directory, and
`worktrees mcp` legitimately serves nothing outside a repo. Measured:

```text
$ cd /tmp      && claude mcp get worktrees   ✘ Failed to connect: CONNECTION_CLOSED
$ cd <a repo>  && claude mcp get worktrees   ✔ Connected
```

Both are the *same, correct* install. A status panel built on that verdict would
show a red ✘ depending on where the app happened to be standing. It also costs
~1.1s per invocation, which rules it out of anything repeated.

**Only user scope is ever written.** Local (`~/.claude.json` →
`projects[<path>].mcpServers`) is detected so we do not nag someone already
covered, and never written — it is invisible from every other checkout of the
same repo. Project (`<repo>/.mcp.json`) is detected and **never** written: a
committed `.mcp.json` is a cloned repository naming a program for claude to
spawn, which is exactly the boundary [ADR 0001](../../adr/0001-no-repo-supplied-argv.html)
draws. `ROADMAP.md` already noted that cwd discovery makes per-repo copies
redundant, so nothing is lost by refusing.

**Four states, not two.** As well as *installed* and *absent*, the panel reports:

| State | Why it needs its own answer |
|---|---|
| `stale` | the stanza names a binary that is gone. Offers **Repair** — re-installing the CLI cannot fix a stale path inside claude's config, so sending the user to Updates would send them somewhere that cannot help |
| `read-only` | registered without `--mutations`; an orchestrator cannot create or close a place with it |
| `foreign` | a server under our key that is not ours. Reported, never overwritten — including a non-stdio one |
| `elsewhere` / `cli-missing` / `not-applicable` | stays quiet. `cli-missing` defers to Updates rather than raising a second competing banner; `not-applicable` is a machine whose `ai_cmd` is not claude |

**Only `absent` may nudge** (`State::nudgeable`). A dismissal of the install offer
must not silence the broken-server warning — the `init_dismissed` lesson (a
dismissal keyed to one suggestion must not swallow a different one) applied here.

**`mcp_nudge_dismissed` is a plain boolean, and that is deliberate** — the
opposite call from `init_dismissed`, which is a content hash precisely because a
boolean was wrong there. It is right here because the card has nothing to
re-suggest: its content never changes, the states that *do* change something are
not this card, and the card retires itself the moment the server exists, so the
flag is unreachable in every other state. No `SETTINGS_REV` bump — an absent key
correctly reads as not-dismissed.

**Home, not a banner over the terminal.** The offer is a fact about the *machine*,
like the version rows and the logo it sits under; a card there is never in the
way of work and does not race the What's-new modal at startup. It appears only
once there is a project to use it on — the server discovers its repo by cwd, so
the pitch is meaningless on an empty workspace. `TmuxBanner`'s slot was rejected
because that idiom means "nothing works until you fix this", and this is an
enhancement.

**Its own Settings category rather than a block inside Commands.** It starts as
one section but already carries status, scope, two repair paths, the hand-run
command and an uninstall — and it is where a dismissed Home card sends people, so
it has to be findable by name.

**The install defaults to `--mutations`, with the consequence stated plainly.**
That is what the orchestrator pattern needs. The honesty has to be in the copy
rather than in a guard, because `confirm: true` on the destructive tools is a
speed bump *the model sets itself*; what actually holds is that it happens in a
pane you are watching.

**Profile coverage is judged on the EFFECTIVE profile.** The first version asked
"does any profile have `worktrees_mcp` ticked", which is wrong: a profile made
once and never assigned says nothing about the sessions you actually launch, and
counting it would silence the offer for someone not covered at all.

## Dead ends / gotchas

**A user-scope server is launched by every session — including the ones with no
repo.** `worktrees mcp` exited at `Project::discover`, which was fine while the
server was added per-repo and becomes a red `✘ CONNECTION_CLOSED` in `/mcp` for
every claude session started in a home or scratch directory, for a setup that is
entirely correct. Installing it globally is precisely what makes this universal —
so the feature could not ship without the fix. `Server.project` became
`Option<Project>`; the server now handshakes, advertises zero tools, and says why.

**The bats suite caught that by asserting the old contract.** `test/mcp.bats` had
*"outside a git repository the server refuses to start"*. The right move was to
rewrite the test to the new contract, not to silence it — it is now
*"…the server serves, with no tools"*, plus two new cases for the setup verbs.

**The git guard in `main.rs` is upstream of `cmd_mcp`.** First attempt hoisted
only the setup verbs above it; the server itself still died on the guard with
`✗ Not inside a git repository.` — an error from a completely different layer
than the one being debugged. All of `mcp` now runs ahead of the guard.

**A non-stdio entry under our key read as *not installed*.** `Entry::parse`
returned `None` for anything without a `command`, which collapses "somebody else
holds our key" into "nothing is there" — so the install button would have run
`claude mcp add` over an existing name and failed with claude's own error *after*
the click. Found by hand-writing an http entry into a throwaway `HOME`; no test
would have caught it. `parse` now returns a non-ours `Entry`, and `None` is
reserved for a value that is not an object at all.

**The bats harness sets `WORKTREES_AI_CMD=fake-ai`,** so a new test asserting
`"state":"absent"` failed with `"state":"not-applicable"` — the *feature working
correctly*, and the test's premise wrong. It became two tests: not-applicable from
the harness default (free coverage of the suppression rule), and absent with
`WORKTREES_AI_CMD=claude` forced.

**`getComputedStyle().backgroundColor` on a transparent host reads as black.**
Measuring the Settings verdict box against `.settings-body` gave 231 and 245 RGB
units of "separation" in the light themes — an artifact, not a result. Re-measured
by walking up to the first ancestor that actually *paints*: 8–17 units against
`.settings-modal`, plus a clear border in all six themes. The Home card's own
measurement (`.mcp-card` vs `.main`) was sound: 16–43 units, `--bg-elev` over
`--bg-abyss`.

**`VITE_MOCK=1` is what installs the mock backend**, not the port or a query
param. A harness started with a bare `vite` throws
`Cannot read properties of undefined (reading 'transformCallback')` — the real
Tauri API failing — and every selector then times out, which reads exactly like a
component that did not render.

**Two sessions cannot share the chrome-devtools-mcp profile.** `new_page` failed
with *"The browser is already running for …/chrome-profile"*. Driving the harness
with a scratch-dir Playwright + headless chromium was both the fix and the better
tool: deterministic, scriptable, and it touches nobody's browser.

**`gh pr merge` from a side worktree reports a failure it did not cause** — the
documented one, hit exactly as written: `fatal: 'main' is already used by worktree
at …`, from `gh`'s *local checkout* step, after the merge had already landed on
GitHub. `gh pr view 218 --json state,mergeCommit` confirmed `MERGED` / `c005220`.
Do not retry.

**Main moved three times mid-session, and one merge was not mechanical.** #216 +
#217 (Claude status indicator) auto-merged with one additive `CHANGELOG` conflict.
**#215** (places as MCP resources) landed in the file this session restructured
most: it adds a resource surface and a watcher thread built on `Server.project`
being a plain `Project`. Git auto-merged the *field* and left the consequences —
so the same treatment tools already had was extended to resources (no project ⇒
no resources; any uri is the `-32002` the client already understands for an
unknown one), the watcher is not spawned at all without a project rather than
started against a path that does not exist, and `dlog` falls back to the launch
directory. **#215 also added a CI gate, `make test-mcp`,** which drives the server
over a real pipe and is directly in the path of these changes — worth knowing that
a rebase can hand you a new gate as well as new code.

## Verification

Gates, all re-run on the final rebased tree (not carried over from before the
rebases): `make test` 338/338 · `make test-mcp` · `make lint` ·
`cargo test -p worktrees-core` 321 · `-p worktrees-cli` 15 · `-p app --lib` 58 ·
`tsc --noEmit` · `cargo check -p app` · all 12 `app/scripts/*-check.mjs`.
CI green on both OSes across all 9 jobs.

Beyond the gates:

- **The real `claude`, against a throwaway `HOME`** so no real config was touched
  (verified intact afterwards): install read-only → detect read-only → upgrade via
  remove-then-add → detect mutating → uninstall → absent. Plus `stale`, `foreign`
  and http-under-our-key by hand; `install` *and* `uninstall` both refuse a
  foreign entry.
- **The no-project server over a real pipe:** clean handshake, 0 tools,
  0 resources, `-32002` on read, exit 0, no watcher spawned.
- **UI driven headlessly** through 21 assertions across all eight states, using
  `elementFromPoint` hit-tests rather than bounding boxes, plus a 900×620 pass for
  the clipping ancestor. Driver preserved at
  `~/.cache/worktrees/worktrees/mcp-setup-wizard/drive.mjs` (needs
  `VITE_MOCK=1 vite --port 5199`); `themes.mjs` / `verdict.mjs` are the contrast
  measurements. Screenshots of every state are in the same directory.
- **All three new tests shown red before green** (`ours` judged on what it runs;
  the no-project tool list; the no-project resource list).

**Not done: a real-app run** (`app/scripts/sandbox.sh --app`). `mcp_install` is a
~1s subprocess behind a busy state, and CLAUDE.md is explicit that the mock's
instant answers hide exactly that class of bug. Flagged on the PR.

## Follow-ups

Carried into `ROADMAP.md`:

- Real-app pass over the setup flow before the next release.
- #216's Claude *status* indicator and this session's Claude *settings* category
  are unrelated code but the same word; the status detail may want to live in the
  category.
- This working tree's idle base is the bare tree name `mcp-setup-wizard`, not
  `mcp-setup-wizard-next` as `.claude/close-out.md` requires.
