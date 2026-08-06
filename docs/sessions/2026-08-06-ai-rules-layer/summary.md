---
title: "Session: AI profiles — the AI rules layer"
---

# AI profiles — the AI rules layer

- **Dates:** 2026-08-01 → 2026-08-06
- **Worktree:** `.worktrees/ai-rules-layer`
- **Branch:** `ai-rules-layer` → squashed to `6585c50` on main
- **PR:** [#72](https://github.com/penard-monkey/worktrees/pull/72) (22 commits, merged)
- **Release tag:** none — v0.8.0 shipped from another stream while this was open
- **Planning files:** `planning.tar.gz` beside this file

---

## What shipped

A user-defined **AI profile** — rules text, skills, MCP servers, settings and
model — applied to the `claude` session worktrees launches, instead of the
user's global `~/.claude`. Normal terminal `claude` is untouched.

| Area | Files |
|---|---|
| Profile model, resolution, materializer, launch shape | `crates/worktrees-core/src/profile.rs` (~2k lines) |
| Skill store (local + git installs, review gate) | `crates/worktrees-core/src/skillstore.rs` |
| Launch seam + stamping | `crates/worktrees-core/src/ops.rs` (`ai_launch_for`, `launch`) |
| Effective-config probes | `crates/worktrees-core/src/project.rs`, `store.rs` (`profile_id`/`profile_epoch`) |
| MCP server | `crates/worktrees-cli/src/mcp.rs`, wired in `main.rs` |
| Tauri commands + snapshot overlay | `app/src-tauri/src/lib.rs` (13 new commands) |
| UI | `app/src/ProfilesPanel.tsx` (new), `SettingsSheet.tsx`, `ProjectSheet.tsx`, `App.tsx`, `App.css` |
| Mocks | `app/src/mock/install.ts`, `fixtures.ts` |
| Tests | `test/profile.bats`, `test/skills.bats`, `test/mcp.bats` (+36 bats, +40 unit) |
| Docs | `DESIGN.md` §AI profiles, `docs/ai-profiles-manual-checks.md`, `docs/ai-profiles.html` (linked from the site index), README §Scripts |
| Tooling | `app/scripts/sandbox.sh`, `shoot-profiles.{sh,py}`, `record-profiles.{sh,py}` |

Resolution chain: `$WORKTREES_PROFILE > assignments[repo] > default_id > none`.
A project profile **replaces** the global default; they do not merge.

---

## Decisions

**Storage split: declarations in `~/.config/worktrees/`, materialized dirs in
`$XDG_DATA_HOME`.** The recon said "app config dir", which contradicts core
owning resolution — `app_config_dir()` is a Tauri API and core has no
`AppHandle`, so the CLI could not read it. Materialized dirs are *data*: claude
writes transcripts and caches into them without bound, and burying gigabytes in
`~/.config` would ambush anyone backing up dotfiles.

**The directory path is an identity, not a location.** claude derives its macOS
keychain service name from the config-dir path
(`Claude Code-credentials-<8hex>`), so a profile's sign-in is bound to
`profile_dir(id)`. Hence: `id` immutable after creation, directories stable (not
per-launch snapshots — a fresh dir means a forced `/login`), and `name` is what
the user renames.

**worktrees never handles a credential.** Each profile signs in once, in its own
pane. No copying, no `security(1)`, no credential code path in core. This began
as a risk to mitigate and became the thing that made the design safe — it is
also why deleting a profile *reports* the keychain item it cannot remove instead
of pretending to clean it.

**Fail closed.** A profile that cannot be materialized does not launch claude;
the pane opens on a shell with the reason. Profiles are frequently *restrictive*
(`--strict-mcp-config` removes a global server; settings carry permission
denies), so "could not apply your profile" must never quietly mean "ran without
your restrictions". Changed from fail-open after review — see gotchas.

**Skill capabilities: allow-list, not deny-list.** `name` and `description` are
the only frontmatter keys treated as harmless; everything else is surfaced for
review. A deny-list of known-dangerous keys can always be spelled around; an
allow-list can only be noisy, and noise is the safe direction for a gate.

**Hand-rolled MCP server.** rmcp with the needed features takes the CLI from
**32 to 124 crates** and drags `tokio` into a deliberately sync binary, with a
breaking major roughly annually across four release targets. What it buys is a
newline-delimited JSON-RPC loop, and `serde_json` was already a dependency.
Measured cost of hand-rolling: **+197KB, zero new crates.** Protocol shapes were
read out of rmcp's vendored source rather than guessed.

**Walkthrough as MP4, not GIF.** Same 18s clip: 240K vs 4.8MB, and sharper,
because a gif must quantise this UI's flat dark fills to a 128-colour palette.

---

## Dead ends / gotchas

**The auth spike printed "ROUTE A IS DEAD" twice, and both were my harness.**
Run 1: `timeout` does not exist on macOS, so both probes exited 127 without
running and the verdict logic read rc≠0 as auth failure. Run 2: the profiled
probe's rc=1 was a *terms-acceptance gate*, and the "original" probe's rc=137
was my own 90s watchdog SIGKILLing a `claude -p` that was blocked reading an
inherited TTY. **A nonzero exit code is not evidence of the thing you are
testing.** The eventual fix classified TIMEOUT / HARNESS_BUG / TERMS_GATE /
AUTH_FAIL separately and refused to conclude from a probe that never ran.

**Run 2 also silently tested claude 1.0.120.** A stale npm global under
homebrew's node shadowed the real 2.1.220 in login-shell PATH. Product-relevant,
because `fixup_gui_path()` resolves that same PATH — the app would have launched
the year-old binary. Pin the binary in any spike; never trust bare `claude`.

**Cherry-picking claude's onboarding flags is unwinnable.** Seeding
`hasCompletedOnboarding` + `theme` + trust still parked a pane on an
"Allow external CLAUDE.md file imports?" dialog nobody knew to seed. The real
file has ~80 top-level keys. The materializer copies it wholesale and strips
`mcpServers` + `projects` instead.

**Four of seven serious review findings PASSED every gate.** This is the
session's central lesson:

- A test that grepped a whole `tools/list` blob for `"destructiveHint":true`, so
  swapping every annotation onto the wrong tool stayed green.
- `--settings` asserted *absent* but never *present* — deleting the flag entirely
  passed 34 unit + 8 bats tests, and that flag carries permission denies.
- A capability spelled `"allowed-tools"` (quoted) that no test used, which made
  the entire review gate theatre.
- A checkbox persistence bug that only exists on WKWebView — the engine the mock
  harness specifically does **not** run.

**Ask what a test would fail on, not whether it passes.** Reverting each fix to
confirm the new test breaks became standard practice mid-session and caught
several tests that could not fail.

**Fixing review findings introduces new ones.** `ac0080b` fixed the WKWebView
checkbox bug on **one checkbox out of four**; the other three were worse —
bound to state that never updated, so "also load my global MCP servers" could be
turned on and never off. Every checkbox now routes through one `patch()` helper,
because the inconsistency *was* the defect. The final review existed only
because that fix commit had never itself been reviewed.

**Trust must be mirrored, never invented.** The materializer pre-accepted the
trust dialog for every profiled worktree, including
`hasClaudeMdExternalIncludesApproved` — which waves through `@`-imports reaching
outside the repo, so a hostile clone's `CLAUDE.md` could pull `@~/.ssh/config`
into context. Enabling a profile was strictly *weaker* than plain claude. The
asymmetry was self-evident in hindsight: `projects` is stripped precisely so
other repos' `allowedTools` cannot leak in, and the next block granted an
unreviewed repo the strongest per-project flags available.

**`create_worktree`'s `base` reached `resolve_ai_cmd`.** Core's arg parsers
consume anything flag-shaped *as a flag*, so `base: "--ai=touch /tmp/x"` set the
AI command, which `ops::launch` interpolates into `sh -ic`. A tool advertised as
"create a worktree" was arbitrary command execution. This is the class ADR 0001
(landed on main mid-session) declares permanently out of bounds.

**`ext::` git URLs execute during clone,** and whether that is permitted came
from the user's own `~/.gitconfig` — so the "installing never executes anything"
invariant rested on config the module does not own. Now the transport-helper
shape is refused and the clone pins its own protocol allow-list.
`--recurse-submodules=no` was also the wrong spelling; git reads it as a
pathspec, so it never disabled anything.

**Mock parity is not just commands.** The command audit said 62/62, but the new
*snapshot fields* had no fixture, so the topbar badge was undevelopable
headlessly. Worse, `place()` in `fixtures.ts` builds its object field by field
rather than spreading — fields added at call sites were silently dropped,
type-valid on both sides and a lie. **The screenshot caught what `tsc` could
not.**

**`bin/worktrees` prefers `target/release`.** Already in CLAUDE.md; still cost
an hour when a const flip was not rebuilt in release and bats tested the old
binary. Rebuild release *first*, always.

**A worktree is not a clone.** `make test` died on a missing bats binary
(submodules uninitialised) and `tsc` on a missing `marked` (`pnpm install`
predated a merge). Both now documented in CLAUDE.md by a parallel session that
hit the same walls.

---

## Verification

- Gates at merge: release build · **bats 286** · lint · **core 205** · cli 6 ·
  `tsc --noEmit` · `cargo check -p app`. CI now also runs
  `cargo test -p worktrees-cli`, which it never did — a test module with a
  lifetime error shipped because of that gap.
- **Empirically established against claude 2.1.220**, not assumed: the
  per-config-dir keychain derivation; that a profiled `/login` leaves the main
  credential untouched; that MCP servers and skills isolate under the swap but
  **user memory does not**; that `--append-system-prompt-file` works
  interactively and survives `/clear`.
- Seven adversarial review rounds, each finding handed to a separate skeptic
  prompted to refute it. Every round found something real.
- The screenshots and walkthrough are generated from the mock harness
  (`app/scripts/shoot-profiles.sh`, `record-profiles.sh`), so they regenerate
  deterministically and cannot drift from the UI.

**Not verified:** `docs/ai-profiles-manual-checks.md` has never been run. Every
claude-side behaviour is invisible to CI — the bats suite drives fake git/tmux
and there is no fake claude. That checklist is the only gate for it.

---

## Follow-ups

- [ ] **Run `docs/ai-profiles-manual-checks.md`.** §3 (adoption + auto-resume)
      is the one that fails *silently*. `app/scripts/sandbox.sh` makes it safe to
      do alongside a running app.
- [ ] **Three orphaned keychain items** from the auth spike hold live OAuth
      tokens for config dirs that no longer exist:
      `Claude Code-credentials-{1fbcb801,88cf516b,ec8e0248}`. Remove via Keychain
      Access, or `security delete-generic-password -s "<service>"`.
- [ ] **`sandbox.sh --app`'s identifier override is unverified** — `tauri info`
      does not report the identifier, and confirming needs a GUI launch. The
      script tells you to check Settings → Data & Logs on first run; if it shows
      `net.casadelvalle.worktrees.sbx`, downgrade that caveat to a note.
- [ ] **`--` end-of-options in core's arg parsers**, as defence in depth behind
      the MCP boundary's `safe_arg`. ADR 0001 makes this more relevant, not less.
- [ ] **No automated UI tests exist** in this repo at all. The mocks make the
      profiles editor drivable headlessly; nothing drives it. Both HIGH findings
      in the last two reviews were UI bugs.
- [ ] **Keychain GC on profile delete** is deliberately unimplemented — core has
      no credential code path, and the service-name suffix is an undocumented
      hash of the directory path. Delete *names* the leftovers instead. Revisit
      only if claude documents the derivation.
- [ ] **A profile is executable content** — `mcp_servers` become subprocess
      command lines and `settings.json` is where `hooks` live. Fine while the
      user authors them; **not** fine the day a profile can be imported. Before
      any import/share feature: confirmation UI for `command`/`args`/`hooks`,
      default-drop `hooks`/`permissions` from imported settings, and a
      `source: "imported"` provenance badge.
