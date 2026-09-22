---
title: "Proposal — project automations"
---

# Proposal — project automations

**Status:** design, 2026-09-22. Nothing built. Decisions taken so far: variant
B (an automation is a **brief on a schedule**, no recipe vocabulary — §2),
project scope (§3), a fourth dock tab as the surface (§7), and the brief
editor is a **modal** (§7.4). Mockups: the design canvas
<https://claude.ai/artifact/FpGDmdggYcNByMFjjayNDz> (five frames of the chosen
design, two alternatives drawn for comparison).
**Supersedes:** three ROADMAP seeds from the 2026-08-29 status-check session —
"fold the health verdict into MCP `place_status`", "`worktrees status --ai`",
and "a workspace-wide triage sweep (all my worktrees, one `claude -p`)". The
third IS this feature's first automation; the first two become parts of §5.
**Owner:** this repo. First consumer: the close-out sweep this repo runs by
hand every session (`.claude/close-out.md`), across nine worktrees.
**Backing research:** `findings.md` in the `worktrees-automations` worktree
(gitignored; archived at close-out) — the surface survey, two hazards found
in the code (§4.3, §4.4), and the A/B/C fork.

---

## 1. The gap

Every morning the question is the same: *which of these worktrees still
matters?* The app answers it one place at a time — the status sheet's "Ask
Claude" runs a headless `claude -p` over one place's health report and caches
a paragraph (`ai_status_report`, app-only, button-only by design). Nothing
answers it for a project, nothing answers it unattended, and nothing keeps
what was answered yesterday.

The same shape recurs: "what exists only on this machine", "what happened
this week", "which branches drifted from main's docs". Each is a paragraph of
intent that a Claude session could act on, over every place of a project,
with the tools the MCP server already exposes. What is missing is the frame:
a place to write the intent, something that runs it, and somewhere the answer
lands and stays.

## 2. Design principles

1. **An automation is a brief.** Since #183 the unit of "work handed to an
   agent" is `.planning/brief.md`, plain markdown a person wrote. An
   automation is the same thing at project scope, with a *when*. There is no
   vocabulary of built-in actions to learn (that was variant A, rejected:
   "I don't want to start remembering vocabulary"). The empty state offers
   three **starter briefs** — prose you read and edit, not commands.
2. **Project scope, and no other.** Definitions and the set of places a run
   may touch belong to one project. The clock and the cross-project view are
   the app's (§3).
3. **Report first.** A run's job is to come back with findings and
   *proposals*; applying a proposal is one click by a person, through a
   command that already exists. Whether a run may change declared state on
   its own is a per-automation setting that defaults off (§4.2).
4. **Never poison the signals.** A run is *about* the places, not work *in*
   them. It must not move `last_worked_epoch`, light an agent dot, or count
   as activity in the health verdict (§4.3).
5. **ADR 0001 stands.** Nothing a cloned repository contains becomes argv or a
   prompt. Briefs are user-owned data, never `.worktrees.toml` (§4.1).
6. **Same truth everywhere.** Core owns the model; the CLI, the MCP server and
   the app read and write it through the same functions, so a run started
   from a discussion looks identical to one started from the tab.

## 3. Scope: why project, not application

- Core has no workspace. The workspace is `projects.json` in the app's config
  dir; `Project::discover` finds ONE project from cwd, which is exactly what
  "start it from any worktree, apply it to the project" needs and nothing
  more.
- Blast radius follows the repo, the same way `remove_worktree` and the AI
  profile do.
- A run needs an identity to run as. The AI profile is per project
  (`profile.rs`); an application-scope run would have none.
- Every cross-project need is a **view** (Home's "Since you left" strip, §7.5)
  or a **fan-out** (the app runs each project's due automations). A later
  "workspace template" that stamps one brief into every project changes
  nothing below.

## 4. Security boundary

### 4.1 Where things live

| Thing | Where | Travels between Macs? |
|---|---|---|
| Definitions (`automations.json`) | `<main root>/.worktrees.automations.json` — twin of `.worktrees.places.json`: git-excluded via `.git/info/exclude` (`store.rs` 113–140), machine JSON, Rust sole writer | **Yes** — the places sidecar is not in `sync.rs`'s builtin excludes, so this one travels too. Wanted: the same jobs on both machines. |
| Briefs | inside `automations.json` as a string (a brief is a paragraph, not a document) | yes, with the definitions |
| Run ledger | `$XDG_STATE_HOME/worktrees/runs/<hash of canonical main root>/<run id>.json` — the per-machine precedent is `init-hints/<hash>` (`init.rs` ~1025) | **No, and must not**: two machines would double-run a daily job and overwrite each other's runs. |

Provenance: `automations.json` is written by this tool on this machine at a
person's request. A clone never carries it. That is the same argument that
lets `.worktrees.places.json` hold a `note` that Claude reads.

### 4.2 What a run may do — three tiers, per automation

| Tier | The run may | Default |
|---|---|---|
| **Report only** | read (`list_places`, `place_status`, `show_doc`, files); write its report and *proposals* | ✔ |
| Notes, pins, lifecycle | additionally `set_note`, `set_pin`, `set_lifecycle` — declared state, reversible, every call logged in the run | |
| Also close sessions | additionally `close_session` on a place with **no** live agent | |

**`remove_worktree` is never available to a run.** Not as a tier, not with
`confirm: true`. `remove_place`'s `force` is two permissions in one flag
(CLAUDE.md) and the only path in this codebase that can destroy commits; an
unattended caller does not get near it. A run that thinks a place should go
says so as a proposal, and the button on the proposal opens the same
`RemoveDialog` a person would use.

Mechanism: the runner launches the in-run MCP server with
`WORKTREES_RUN_TIER=<tier>` and `WORKTREES_RUN_ID=<id>`. The server, seeing
`WORKTREES_RUN_ID`, offers the tier's tool set and **no automation tools at
all** (§4.5), whatever `--mutations` and the profile's
`worktrees_mcp_mutations` say. The tier can only narrow the profile, never
widen it: a profile whose server is read-only stays read-only.

### 4.3 The two hazards the code already has

**A headless claude with `cwd = place` is activity.** `health::assess` takes
`claude_last_epoch` — the newest transcript timestamp under the place's
`~/.claude/projects/<mangled>/` — into its activity max. `ai_status_report`
runs `claude -p` *in the place directory*, so a sweep that did the same over
every place would make every place read `active` the next morning and blind
itself. Rule: a run's claude executes with **cwd = the project's main root**
and receives place paths as data in the prompt. Whether `-p` also writes a
`sessions/<pid>.json` probe (the nav dot) is unverified; the same rule
covers it either way, and the manual-checks doc gets an entry (§9).

**`last_worked_epoch` is not touched by a report.** `ai_status_report` says
so in its own comment ("a report ABOUT a worktree is not work IN it"). Same
here: a run writes only its ledger entry, and — for tiers above report-only —
the declared fields it was allowed to.

### 4.4 Live sessions

`worktrees_core::agent` knows whether a Claude session is busy or waiting in a
place. A run above report-only **skips** any such place and says so in the
findings ("Skipped: Claude is working here"). Report-only runs read freely.

### 4.5 Recursion

A run holds the worktrees MCP server. If `run_automation` were visible to it,
a brief could spawn runs. The in-run server (§4.2) hides every `*_automation`
tool and every `*_run` mutation; it may `list_runs` and `get_run` so a brief
can say "compare with yesterday".

## 5. The model

```jsonc
// <main root>/.worktrees.automations.json
{
  "version": 1,
  "automations": {
    "close-out-candidates": {                  // slug, immutable, from the name at creation
      "name": "Close-out candidates",
      "brief": "Look at every worktree in this project except …",
      "when": { "kind": "daily", "at": "08:00" },   // | {"kind":"manual"} | {"kind":"weekly","day":"mon","at":"09:00"}
      "scope": "all",                              // | "brief" — "let the brief say"
      "tier": "report",                            // | "declared" | "sessions"   (§4.2)
      "created_epoch": 1758520800,
      "enabled": true
    }
  }
}
```

```jsonc
// $XDG_STATE_HOME/worktrees/runs/<hash>/2026-09-22T08-02-11Z-close-out-candidates.json
{
  "version": 1,
  "id": "2026-09-22T08-02-11Z-close-out-candidates",
  "automation": "close-out-candidates",
  "trigger": "schedule",                       // | "manual" | "mcp"
  "started_epoch": 1758528131, "finished_epoch": 1758528203,
  "status": "findings",                        // | "clean" | "failed" | "running"
  "profile": "default",
  "places": ["feat-redesign", "search-index", "fix-flaky-ci", "billing-refactor", "(main)"],
  "skipped": [{ "slug": "billing-refactor", "why": "live agent" }],
  "facts": { /* health::Report per place, tier-0, computed by the runner */ },
  "findings": [
    { "slug": "fix-flaky-ci", "text": "Merged into main 19 days ago …",
      "proposals": [{ "tool": "set_lifecycle", "args": { "slug": "fix-flaky-ci", "lifecycle": "abandoned" } }] },
    { "slug": "search-index", "text": "3 commits not on main and no upstream …",
      "proposals": [{ "tool": "set_note", "args": { "slug": "search-index", "note": "push before deciding" } }] }
  ],
  "actions": [],                               // calls the run actually made (tiers above report-only)
  "report_md": "Three of five worktrees hold no unique work …",
  "turns": 12, "seconds": 72,
  "error": null,
  "seen_epoch": null                           // the ack (§7.3)
}
```

Findings are **data** (a slug, a sentence, proposals from a closed set of
tools); the report is prose. The app renders both; a discussion asks for
either through MCP. A proposal's `tool` must be one of `set_lifecycle`,
`set_note`, `set_pin`, `close_session` — the runner drops anything else and
records that it did (never silently: `diag.rs`'s rule).

## 6. Execution

### 6.1 One runner, any clock

`worktrees automations run <slug>` is the only thing that runs an automation.
The app's tick, a future launchd job, and the MCP `run_automation` tool all
spawn it. It:

1. Takes a lock (`<ledger dir>/<slug>.lock`) — a second caller exits 0 with
   "already running", so two clocks never double-run.
2. Writes the ledger entry with `status: running` first — the tab shows a
   spinner from a file, not from an in-process promise.
3. **Tier 0**: computes `health::Report` for every place (the pure `assess`,
   same call as `worktrees status`), and the agent probe for each. Free,
   deterministic, testable. Writes `facts` into the entry.
4. Composes the prompt: the fixed opener (the brief is never argv, same rule
   as `BRIEF_OPENER`), the path of a temp file holding the brief, the path of
   `facts.json`, the tier, and the output contract: *write `findings.json`
   and `report.md` to `<run dir>`; proposals use only these tools with these
   arguments; do not run git.*
5. Launches through **the seam**: `ops::ai_launch_for(project, ui, main_root,
   ai_cmd)` — the same call every interactive launch and `ai_status_report`
   make, so the run inherits the project's profile, its `CLAUDE_CONFIG_DIR`
   and its MCP servers — then `exec <ai.cmd> -p <opener> --max-turns N` under
   a deadline (`run_deadline`, moved from `lib.rs` into core), cwd = main
   root (§4.3), env `WORKTREES_RUN_ID`/`WORKTREES_RUN_TIER` for the in-run
   server (§4.2).
6. Validates `findings.json` (schema, closed tool set, slugs that exist),
   folds it and `report.md` into the ledger entry, sets `status`, releases
   the lock, exits 0 (clean), 2 (findings — `doctor`'s convention), 1
   (failed).

Only the tier-0 facts are handed to claude by file; the MCP server is there
for anything the brief wants beyond them. A brief that says "read each
place's `.planning/brief.md`" gets it through `show_doc`.

### 6.2 The clock

**"Due since the last run", not cron.** A laptop is closed at 08:00 more
often than not. The app's existing background tick (`FETCH_INTERVAL_SECS`,
`lib.rs` ~6567) gains one more job: for every project in the workspace, for
every enabled automation whose `when` has a slot between the last run's
`started_epoch` and now, spawn the runner. Missed slots collapse to one run
(anacron semantics). The CLI has the same check as `worktrees automations
tick`, which is what a launchd job would call — out of scope for phase 1.

Because the ledger is per machine (§4.1), each Mac keeps its own "last run",
which is what makes the shared definitions safe.

### 6.3 MCP

| Tool | Mutating | Notes |
|---|---|---|
| `list_automations` | | slug, name, when, tier, last run summary |
| `get_automation` | | the brief too |
| `upsert_automation` | ✔ | name, brief, when, scope, tier; slug fixed at creation |
| `delete_automation` | ✔ | |
| `run_automation` | ✔ | **async**: spawns the runner, returns the run id. An MCP call must not block for the minutes a run takes. |
| `list_runs` | | newest first, `automation` filter, `unseen` filter |
| `get_run` | | findings + report + actions |
| `mark_run_seen` | ✔ | the ack (§7.3) — so a discussion that read a run can clear the dot |
| `apply_proposal` | ✔ | `run id` + finding index + proposal index → makes that one call, records it in `actions`. This is how a discussion applies what a run proposed without retyping arguments. |

Resource: `worktrees:run://<id>` beside `worktrees:place://<slug>`, built by
core (`mention.rs`'s rule: one producer). Hidden inside a run: everything
mutating above except `mark_run_seen` (§4.5).

All of it is served by the one user-scope server (`claude mcp add -s user
worktrees …`), discovered from cwd — a session in any worktree of the project
reaches its automations, which is the "from any worktree" requirement.

## 7. App surface

The canvas has the pixels; this section has the rules.

### 7.1 A fourth dock tab: Automations

`Settings["dock_tab"]` gains `"automations"`; `DOCK_RAIL` gains an entry
(`key` + `track` pinned together, as the comment there requires); the rail
icon is a bolt. Like Files/Terminal/Docs it is disabled with no place
selected — the content is the **project's**, reached from any of its places,
so the header names the project (the Docs tab's lesson: a surface shown from
many places must say whose it is).

Two lists. **Automations**: name, `when · tier` on the second line, last
result on the right (`● 3 findings · 6h ago` / `● clean · 2d ago` / `never
ran`), a run-now button per row. **Runs**: grouped by day, newest first,
unread dot on unseen runs with findings, `All runs…` to expand. Footer: the
profile it runs as, and "never removes a worktree".

`panelsFor` seeds `dock_tab` from the global; the new key needs no per-place
field of its own (the CLAUDE.md note about optional twins applies only if
one is added).

### 7.2 A run

Back link, name, one meta line (`today 08:02 · 1m12s · 12 turns · profile
default · 3 findings`). Then **FINDINGS** as cards: the place name is a link
that selects the place in the nav; the proposal is a button that runs the
existing command (`set_lifecycle` → the same optimistic `patchDeclared`
path the Lifecycle menu uses; a remove proposal opens `RemoveDialog`).
Skipped places are cards at 70% with no buttons. Then **CLAUDE'S READ**: the
markdown through `markdown.tsx`, in the same `.md` block the Docs tab uses.
Footer: `Mark as seen`, `Copy as markdown`.

### 7.3 Unread

A run with findings is *unseen* until acked. The rail icon carries an amber
dot when any project has an unseen run; the tab's run rows carry it per
run. The afterglow rule applies verbatim: the **fact** (`seen_epoch ==
null && status == findings`) guards the write; what is painted may subtract
live state, never the other way round. `mark_run_seen` is the one ack, from
the button, from opening the run in the tab, and from MCP.

### 7.4 New / edit — a modal

Decided: a modal over the main pane, the `NewPlaceDialog` shape, with
`useEscape`. Fields, top to bottom:

- **Name** (text; the slug is derived once, on create).
- **What should Claude do?** — the brief, a textarea, prefilled by a starter
  when one was picked.
- **When** — segmented: *When I ask · Every morning · Weekly*, with a time
  (and a day for weekly). Three choices, no cron syntax.
- **Which worktrees** — *All of them · Let the brief say*.
- **What it may change** — three radios (§4.2), report-only selected, each
  with a one-line consequence under it.
- A footer line: *Runs headless as this project's AI profile `<name>`. It
  can never remove a worktree.*
- Buttons: Cancel · Save · **Save & run now**.

The modal is the same for edit; `Save & run now` is what makes "try it"
one click, which is what the empty state's starters lead to.

### 7.5 The empty state, and Home

Empty tab: the bolt, one sentence ("An automation is a brief that Claude runs
across this project's worktrees, on a schedule or whenever you ask. Every run
leaves a report here."), then **START FROM ONE** with three cards — *Close-out
candidates*, *Unpushed work*, *What happened this week* — each a prose brief
that opens prefilled in the modal, and `Write your own…`. This is where
"obvious to a new user" is paid for: a starter is an example, not a
command.

Home gains **SINCE YOU LEFT** above *Resume where you left off*: one row per
unseen run across all projects (`● Close-out candidates · worktrees · 3
findings · 08:02 · View ▸`). `View` selects a place of that project and opens
the tab on the run. Rows disappear when acked. That strip is the entire
application-scope surface.

### 7.6 Alternatives drawn and not chosen

- **A nav node under each project with its own main-pane page.** Findings
  next to the places they are about is attractive; but the main pane is
  binary (place | Home) and `ProjectSheet.tsx`'s header records that a third
  mode touches ~30 `sel?.repo` read sites. Not for a first version.
- **A section in Project settings.** Definitions fit (files, ports, docs
  config live there); results do not, and the daily surface would sit behind
  a right-click.

## 8. Phasing

| Phase | Ships | Not in it |
|---|---|---|
| **1 — manual runs** | core `automation.rs` + `runs.rs`; `worktrees automations ls/add/rm/run/runs/show`; MCP tools (§6.3); the tab, the run view, the modal, the empty state; report-only tier; `apply_proposal` from the UI buttons | schedule, Home strip, unread dot, other tiers |
| **2 — the clock** | `when`, the app tick + `automations tick`; unread + `mark_run_seen`; Home strip; rail dot | launchd |
| **3 — tiers** | *declared* and *sessions* tiers; live-agent skip; `actions` in the ledger | anything that removes |
| later | launchd install; workspace templates; retention beyond "last 50 per automation"; `worktrees:run://` mentions | |

Phase 1 alone already replaces this repo's hand-run close-out sweep.

## 9. Test plan

- **bats** (fake `git`/`tmux` shims already intercept the binary): add a fake
  `claude` shim for `-p` runs — it copies a canned `findings.json` +
  `report.md` into `<run dir>` — and pin: the runner's cwd is the main root
  (the shim records `$PWD`); the brief never appears in the shim's argv; a
  proposal naming `remove_worktree` is dropped *and recorded*; exit codes
  0/2/1; the lock; `--max-turns` and the deadline present in argv;
  `WORKTREES_RUN_ID` set. The AI-profiles note "there is no fake claude" is
  about interactive launches; a `-p` contract is shimmable.
- **core unit tests**: schema round-trip; `when` → due-slots (anacron
  collapse, DST); ledger hashing matches `init-hints`; the in-run tool set
  per tier (`mcp.rs` tests already enumerate tools by mode).
- **mcp.bats**: `run_automation` returns before the run finishes;
  `list_runs` sees `running`; the in-run server hides the automation tools.
- **app**: mock harness gets `list_automations`/`list_runs`/… with
  `?slowrun=<ms>`; a `dockrail-check.mjs` asserting the 4th entry's `key` ==
  `track` suffix (the comment's rule, mechanised); `afterglow-check.mjs`
  extended: the unread **fact** guards `mark_run_seen`.
- **manual** (`docs/ai-profiles-manual-checks.md`, new §): one real run in
  `sandbox.sh --app`; confirm no place's dot lights and no verdict flips to
  `active` after a sweep (§4.3); confirm a run started from a `(main)`
  discussion via MCP appears in the tab.

## 10. Open questions

1. **Does phase 1 ship any tier above report-only?** Recommendation: no.
   Proposals + one-click apply cover the close-out case entirely, and every
   day the feature runs report-only is a day of evidence about what the
   briefs actually propose before anything acts unattended.
2. **`facts` in the ledger** — the full per-place `health::Report` is a few
   KB × places × runs. Keep it (it is what makes a run explainable a week
   later) with "last 50 per automation" retention, or keep only the verdicts?
3. **Cost.** A daily `-p` run per project, `--max-turns 12`, under the
   subscription. Worth a per-automation `max_turns`? Default first, measure.
4. **Is `-p` writing a session probe?** (§4.3) — measure once in the sandbox
   before phase 1 lands; the cwd rule holds regardless.
