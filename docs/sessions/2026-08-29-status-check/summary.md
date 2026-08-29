# Session: worktree status check — click-select, calm header, verdict sheet, Ask Claude

- **Date:** 2026-08-29 (one session, design through merge)
- **Worktree:** `ui-tweaks` (idle base `ui-next`)
- **Branches:** `ui-click-select`, `ui-header-calm`, `ui-status-check`,
  `ui-ask-claude` — stacked, rebased at merge time, deleted after
- **PRs:** [#171](https://github.com/penard-monkey/worktrees/pull/171)
  click-select · [#172](https://github.com/penard-monkey/worktrees/pull/172)
  calm header · [#173](https://github.com/penard-monkey/worktrees/pull/173)
  status check · [#174](https://github.com/penard-monkey/worktrees/pull/174)
  Ask Claude
- **Release:** none this session (all four ride the next one)
- **Planning files:** `planning.tar.gz` here — task_plan, findings, progress,
  **and `spec-status-check.md`**, the anchor-verified spec the four
  implementation briefs were cut from

## What shipped

1. **Click selects, opening is explicit** (#171). `PlaceRow`'s click ran
   `enterPlace` — every idle click spawned a tmux session (with the AI
   profile in it) and stamped `last_opened_epoch`. New `selectPlace`
   (App.tsx) does selection only; opening = double-click, a hover/selected
   `▸` button in the age's grid slot (`.row-slot`, width-stable by
   construction), topbar Enter, or the menus. Home Resume rows keep
   click-to-open. This is the root-cause fix for "everything reads active".
2. **Calmer header + honest arrows + aging pins** (#172). The duplicated
   "live" badge is gone; the lifecycle chip renders only when it disagrees
   with the dot; `↓behind` left worktree rows/header/attention-lens and
   survives ONLY on `(main)`, where the base ref is origin/main and ↓ means
   "pull" (`glyphs()`, `hasAttention`, topbar in App.tsx). Topbar lost the
   never-used `Lifecycle ▾` (states live in the ctx menu + status sheet) and
   its standing Close (moved into `⋯` with the arm pattern intact). Rows
   untouched for `STALE_DAYS = 14` dim their name (`.row.stale`); pins keep
   position, `(main)` never dims.
3. **`worktrees status <name> [--json]` + the status sheet** (#173). New
   `crates/worktrees-core/src/health.rs`: pure `assess(facts, now, fmt_date)`
   → verdict `active | parked | at-risk | cold` + reason strings; gathering
   in `ops::cmd_status` (resolution shaped like `close` — bare `main` works,
   `(main)` legal, unregistered refused). The app's `place_health` runs the
   same op in-process (doctor pattern) and `StatusSheet.tsx` renders
   verdict + reasons + facts + the commits-not-on-base list + actions
   (Enter / Abandon / Archive / Remove via App's existing flows). `behind`
   is a fact line, never a reason.
4. **Ask Claude** (#174). `ai_status_report` — the repo's first headless
   claude: profile seam via `ops::ai_launch_for`, guard on
   `ai_word_of(&ai.cmd)`, `exec {cmd} -p '<prompt>' --max-turns 12` under
   `run_deadline(180)`, result cached in the place's
   `Declared.extra["status_report"]` (store round-trips unknown keys).
   Button-only, never automatic. `docs/ai-profiles-manual-checks.md` gained
   §11.

## Decisions (each with the why)

- **On-demand only, never ambient.** The whole feature exists to answer "what
  happened here?" when asked — auto-scans and nagging were explicitly ruled
  out (David).
- **`↓behind` is noise on worktrees, signal on main.** For a worktree it says
  "the base moved" — true of everything by Friday, unactionable. For `(main)`
  it is the app's only pull indicator (adversarial review caught that
  removing it there would blind the app). Exception is one `is_main &&` term
  per site.
- **A0 had to land first.** `touch_place` fired on every click, so
  `last_opened_epoch` recorded browsing; the health heuristic's activity max
  includes it and needed it honest.
- **Verdict computed once, in core.** CLI `--json` is the schema; the app
  parses the same struct in-process. CLI and sheet cannot disagree.
- **`ahead` ≠ unpushed.** Divergence is vs the base ref (json.bats pins it),
  so reasons distinguish machine-local ("no upstream") from
  pushed-but-unmerged via `@{u}` — and a CONFIGURED upstream that does not
  RESOLVE counts everything unpushed (the porcelain-v2 trap, again).
- **Fail-closed guard reads the composed command, not `match_word`.** The
  printf sentinel from `ai_launch_for` keeps `match_word: "claude"` — a
  match_word guard would run printf and cache its stderr as Claude's opinion.
  `ai_word_of(&ai.cmd)` yields "printf" there. Found by adversarial review
  before any code existed.
- **`exec` in the sh -c line.** `run_deadline` kills one pid; without exec a
  timed-out claude survives as an orphan.
- **Report cached in `.worktrees.places.json`, not ui-state.json** —
  backend-owned, locked, unknown-keys round-trip; ui-state is frontend-owned
  whole-blob.
- **Tier regrouping deferred** — auto-open poisoned Active/Idle; live with
  #171 before redesigning the buckets.

## Dead ends / gotchas

- **The spec's first draft contained three invented facts**, all caught by an
  adversarial verify pass (3 skeptics) before implementation: the
  match_word guard (above), a `sync` CLI verb that did not exist in the
  checkout being read, and a parked-verdict bats case that could not pass as
  written (parked requires tmux up; `--no-tmux` gives cold — and a fresh
  commit is `active` without `WORKTREES_STATUS_NOW`).
- **Anchors rot fast.** The spec's line numbers were collected on a checkout
  ~37 commits behind; by implementation PlaceRow had moved ~1700 lines.
  Briefs switched to "quoted code is a search key, never trust line numbers"
  — and later agents found `resolve_place` REFUSES main (spec claimed it
  aliased it) and that the ⋯ Remove had become a RemoveDialog with no arm.
- **tmux `session_activity` is useless as a human-recency signal** — the
  app's own attach/poll refreshes it (a month-idle worktree showed activity
  "5 minutes ago"). The honest trio: last commit, `last_worked_epoch`,
  claude-session-dir newest `*.jsonl` mtime.
- **`ai_word_of("")` defaults to `"claude"`** — an empty ai_cmd would pass
  the word guard, so the empty check must come first (unit test documents
  the ordering).
- **Two sheets keyed `"none"`** — StatusSheet's closed-sentinel collided with
  ProjectSheet's sibling key; React reports duplicate keys and may cross-wire
  state. Caught by the harness console check; keys are namespaced now.
- **Pre-existing, not ours:** the mock harness throws an xterm
  `Cannot read properties of undefined (reading 'dimensions')` when a mock
  terminal mounts — proven identical on base with the change stashed and the
  served file verified token-free.
- **The fixtures' frozen `NOW` was a time bomb** — a pinned epoch drifts;
  with row-aging in, the whole harness would have booted dimmed. It is the
  wall clock now.

## Verification

- Full gate suite green **seven times**: once per PR branch at implementation
  and once per rebase during the merge train (release build first +
  freshness check, bats 318→325 with 0 `not ok`, core 254→267, cli 7,
  app-lib 42→45, tsc, lint, cargo check).
- **Every new test shown red first**: health.rs unit tests 10-red against a
  stubbed `assess`; `status.bats` 7/7 red against the shipped 0.18.0 binary;
  PR C's guard tests red against a deliberately match_word-based guard.
- Headless harness passes per PR (mock on :5199, served-file code-token
  checks, one interaction per evaluate, `getComputedStyle` not eyeballs):
  A0 click-spawns-nothing + width-stable `▸`; A 33/34 (the 1 = the
  pre-existing xterm error); B 6/6 incl. verdict colors vs resolved tokens;
  C no-auto-run + in-page 250 ms spinner sampling + cache-on-reopen +
  error path.
- Real CLI on the real workspace: `worktrees status everything-settings` →
  `active` (Entered an hour earlier — correct), and with
  `WORKTREES_STATUS_NOW=+20d` → `work at risk` naming the Aug 2 commit,
  "no upstream", and the patch-id hint.
- Merge train: origin/main had moved (+2, app-zoom) → extra A0 rebase; only
  CHANGELOG conflicted (3×, parallel `[Unreleased]`), merged both sides.
- **NOT done:** `app/scripts/sandbox.sh --app` real-app pass and
  manual-checks §11 — both owed, see Follow-ups.

## Follow-ups

- **Manual verification owed** (the mock cannot express these): one
  `sandbox.sh --app` pass on main — click-select feel, row `▸` hover, ⋯
  Close, the sheet, ONE real Ask Claude run — and
  `docs/ai-profiles-manual-checks.md` §11 (fail-closed and timeout are the
  load-bearing pair).
- Watch whether Active/Idle regain meaning now that clicks don't spawn; if
  not, regroup nav by activity buckets (design sketch in the archived spec).
- MCP `place_status` could fold in the health verdict; `worktrees status
  --ai` (headless report from the CLI) — both cheap now, both deferred.
- Workspace-wide triage sweep ("all my worktrees, one claude -p") — the
  tier-1.5 candidate.
- `app/scripts/record-*.py` were fixed to double-click, but the recorded
  media still SHOWS single-click-opens — regenerate next time media is
  touched.
- Harness xterm `dimensions` error (pre-existing) — track down when it next
  gets in the way.
