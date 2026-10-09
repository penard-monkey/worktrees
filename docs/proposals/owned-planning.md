---
title: "Proposal — worktrees owns planning (opt-in)"
---

# Proposal — worktrees owns the planning mechanism, opt-in

**Status:** investigation and design, 2026-10-09. Not implemented. Pilot
project: casa-del-valle-monorepo (§8.1, phase 1 in §9). Decisions
requested in §11. Nothing here permits merging or implementing anything.

**The request (David, relayed from cdv `(main)`):** "worktrees should actually
own the planning functionality. Something that the users of the application
could adopt. So if the user updates the app to a new version they should be
offered to turn it on. It should explain how the planning will work and if they
want to adopt. They can choose to adopt per project or as a whole. Something
they check when they make/load a project in the app."

**Why now.** casa-del-valle-monorepo (cdv) is the third project to hand-roll the
same layout. Each one re-implements `worktrees_core::plan`'s resolution in bash,
and each one needs its own hook. The planning-with-files skill's hooks (2.37.0)
test only a ROOT `task_plan.md`, so they never fire in the layout all three
projects converged on.

**Evidence base.** Read on this machine on 2026-10-09: worktrees at `c360e08`
(v0.40.1); valleos at `f85839bc`; cdv `main` plus its open PR #684; the
planning-with-files skill 2.37.0 in `~/.agents/skills/`; Claude Code 2.1.296,
codex-cli 0.161.0 and pi 0.99.1 `--help` output; pi's shipped
`docs/extensions.md`. Nothing in another project's tree or in any harness
config was written. No agent session was run. Claims are marked as follows:

- **[observed]**: seen on this machine.
- **[source]**: read in code, docs or `--help`.
- **[inferred]**: my reasoning. Not measured.

---

## 0. The answer in one screen

| Question | Short answer |
|---|---|
| What does "on" mean? | Worktrees owns the **layout** (`.planning/<topic>/` + `.active_plan`), the **resolution** (one function, `plan::resolve`, used by the Plan tab, MCP, the CLI and every hook), the **pointer** (`worktrees new` writes `.active_plan`), and the **agent hooks** (shipped per launch, the way agent guidance already ships). Plan *content* is still written only by the session. |
| What does "off" mean? | Exactly today's behaviour, byte for byte. The Plan tab reads whatever exists and nothing is written. The default is off. |
| Where is the choice? | **User tier only.** A global default with three values (unset / on / off), and a per-project override (inherit / on / off) on the project's registry entry. A cloned repo cannot switch it on for you. |
| How is it offered? | One machine-level offer in `offers.ts`, `planning`, which opens Settings → Planning (the explanation lives there). A checkbox in Add existing / New project, and a post-clone line for Clone. Settings' per-project list for changing it later; a ProjectSheet section in phase 2. |
| Relationship to planning-with-files | **Wrap, don't replace.** The skill stays the *method* for writing the three files. Worktrees takes over *where they live*, *which one is active* and *what gets injected*. The skill's root-only hooks then have nothing to fire on. Hooks are SessionStart and UserPromptSubmit only, because a Stop hook's stdout never reaches the model. |
| `[plan] project` | `[plan] project = "docs/plan/goals.md"` in `.worktrees.toml`. A **path**, which is data, so ADR 0001 allows it. Validated like `[docs]`, read through `safe_under`. It retires the tracked-copy hack in valleos and in cdv #684. |
| Stale-dir trap | When planning is **on**, the "newest directory" guess is **gone**. The order is `[plan] project` (main only), then `.active_plan`, then "no plan yet". A valid pointer whose plan is not written yet means "not written yet" (or the root plan, if one exists). An invalid one (not a plain name, or a symlink) is `invalid_pointer` and never tells anyone where to write. It never falls through to an old directory. When planning is off, nothing changes. |
| Symlink bug | Hooks call `worktrees plan hook <event>`, which calls the same `plan::resolve` as the tab. There is no second resolver to drift **at the same version**. The app (in-process) and the CLI (`<bin>`) can still be on different releases, so the hook stamps its version. Parity is pinned by a test running valleos/cdv's scenarios through both entry points of one binary. The hook re-checks the setting on every call, so turning planning off silences running sessions at once. |
| Codex / pi | **pi:** `--extension <path>` per launch, with `session_start` / `before_agent_start` / `agent_end` **[source]**. Buildable, but needs a probe. **Codex:** `hooks` is a stable feature in 0.161 **[observed]**, but `-c` hooks stop on a trust prompt (AGENTS.md). Phase 1 gives Codex the CLI verb and MCP only. |
| Phase 1 = the cdv pilot | cdv adopts from the offer, as it stands today. Phase 1 contains: the setting, the offer, the Settings panel and the Add/New-project checkbox (post-clone line for Clone); owned resolution; `worktrees plan resolve|hook`; `new` writing the pointer; and the Claude plugin. Nothing in cdv is moved: main's `.planning/orchestrator/` stays its plan, and live root-layout lanes keep working (§8.1). `[plan] project` and `migrate` (valleos, this repo) are phase 2. |

---

## 1. What exists today

### 1.1 In worktrees

| Piece | Where | What it does |
|---|---|---|
| Resolution | `plan.rs::resolve` (private) | Order: `.planning/.active_plan` → newest non-hidden `.planning/<dir>/` holding a `task_plan.md` (dir mtime in ms, alphabetical tie-break) → root `task_plan.md`. Every component it builds is `symlink_metadata`'d, starting with `.planning` itself. `.active_plan`'s value must be one plain path component. The final open uses `O_NOFOLLOW` **[source]**. |
| Summary | `plan::summarize` → `PlanSummary` | Feeds the Plan dock tab and MCP `place_status.plan`. A lenient, grep-shaped extractor, because only 2 of 15 surveyed plans used the template's phase markers (AGENTS.md) **[source]**. |
| Brief | `ops::BRIEF_PATH` = `.planning/brief.md`; `BRIEF_OPENER` (fixed text) | `worktrees new --brief` writes it, and claude opens on the pointer. The brief never travels in argv **[source]**. |
| "Generate plan" | `ops::PLAN_PROMPT` | Fixed text pasted into a live session. It deliberately names **no directory**, leaving that choice to the skill **[source]**. |
| Per-launch agent delivery | `guidance.rs` | Materialises a content-hashed data dir and passes Claude `--plugin-dir …/claude` (plus a guard variant with a `hooks/hooks.json` that runs `<worktrees> guard pretooluse`). pi gets `--skill` + `--append-system-prompt`. Codex gets `-c developer_instructions` only when the user has none of their own **[source]**, **[observed]** (`~/.local/share/worktrees/agent/<hash>/`). |
| Project config | `projcfg.rs` | Sections `file`, `ports`, `compose`, `project`, `docs`. Paths go through `RelPath::parse` (Layer A). An unknown key warns and is ignored; a user-only key is a hard error **[source]**. |
| Per-project user tier | `registry.rs` `Entry { root, name, private }` in `~/.config/worktrees/projects.json` | "Registering is the consent act". `private` is already a per-project, user-only flag that a repo cannot set **[source]**. |

### 1.2 In the three repos

| | valleos (`main`) | cdv (PR #684, open) | worktrees (this repo) |
|---|---|---|---|
| Lane plan | `.planning/<topic>/` + `.active_plan`, written by each lane at start | the same, after #684 | **root** `task_plan.md` (6 of 12 places; none use topic dirs) **[observed]** |
| Main's plan | tracked copy `.planning/orchestrator/task_plan.md` of `docs/plan/goals.md`, kept identical by `scripts/memory/sync-goals.sh`; `.active_plan=orchestrator` | the same, plus a pre-push `--check` on blob ids | none |
| Hooks | `.claude/settings.json` → `scripts/memory/plan-context.sh session|prompt|stop` | a port of it that resolves "exactly like plan.rs" | the skill's own (root only) |
| `.gitignore` | `.planning/*`, then two negations and one re-ignore (`!.planning/orchestrator/`, `.planning/orchestrator/*`, `!.planning/orchestrator/task_plan.md`) re-including one file | the same, plus `/*/**/.planning/` anchoring | root files |
| Close-out | archives `.planning/` | `old-plans/<topic>-YYYYMMDD.tar.gz` (203 archives so far) **[observed]** | the global skill tarballs only the three ROOT files **[source]** |

Survey (`~/.cache/worktrees/worktrees/owned-planning/survey.txt`) **[observed]**:

- **valleos:** 12 of 13 lanes have `.active_plan`. Every lane has **two** topic
  dirs, its own plus the tracked `orchestrator/`.
- **cdv:** 8 of 15 places still use the root layout.
- **worktrees:** no place has `.planning/<topic>/`.

### 1.3 The hijack is live, not hypothetical

The valleos lane `investigate-ci-rentention` has no `.active_plan`. Its Plan
tab and its MCP `place_status` show **main's project goals** as the lane's own
plan, today: `how_resolved: "newest"`, `plan_rel:
".planning/orchestrator/task_plan.md"`, title "ValleOS: project goals",
26/35 checks **[observed]**, via a cross-project `place_status`. The tracked
copy is checked out in every lane, its directory mtime is the checkout time, so
"newest" picks it. That is cdv #684's Finding 2, already happening in valleos.
It is the same mechanism as the June plan that took over cdv main's tab before
`.active_plan` existed. A guess over directory mtimes picks whatever was
touched last, not whatever is current.

---

## 2. Opt-in UX

### 2.1 The setting

There are two keys, both in the user tier. ADR 0001's provenance rule applies
even though no argv is involved: adopting changes what agents are *told* and
which *hooks* run, so a cloned repo must not be able to switch it on.

```
global   ~/.config/worktrees/planning.json   { "default": "unset" | "on" | "off", "version": N }
project  projects.json Entry                 "planning": "on" | "off"     (absent = inherit)
```

- **Effective value:** project ?? global ?? off. `unset` behaves as off; it
  differs only in that the offer is still pending.
- **Every launcher reads the same file.** The app, the CLI and an MCP
  `create_worktree` all read it, and so do the per-launch delivery in
  `guidance.rs`, `cmd_new` and `plan::resolve`. This is the same reasoning as
  `guidance::settings()`: an app-memory override would never reach a
  `worktrees new` run in a terminal.
- **The repo may *suggest* adoption, never decide it.** A `.worktrees.toml`
  with a `[plan]` section pre-ticks the checkbox in Add existing / New project (§2.3; Clone decides after the clone), and the
  checkbox label says why ("this repo uses worktrees planning"). It does not
  change the effective value (Q2).

### 2.2 The offer: machine-level, data with a destination

`offers.ts` rules (AGENTS.md "A suggestion's surface may not add
preconditions"; the file's own header comment):

```ts
{
  id: "planning",
  title: "Let Worktrees keep your agents' plans",
  body: "One plan folder per place, the active plan always the one you see in the Plan tab, and agents reminded of it as they work.",
  cta: "Review…",
  to: { cat: "planning", focus: "planning" },
  fingerprint: `v${ctx.planning.version}`,  // ctx.planning = planning_status, a machine-level command
}
```

- **Shown when** `planning.default === "unset"`, and nothing else. That is a
  fact about the machine. Neither "a project is selected" nor "a project is
  registered" may become a precondition (the v0.25.0 lesson). On a machine
  with no projects the global choice is still meaningful: it is the default
  every later Add inherits.
- **Retired** by making any global choice (on, off, or "choose per project",
  which stores `off` and so leaves every project's own checkbox in charge).
  Dismissal stores the fingerprint, so a later planning `version` worth
  re-asking about asks again.
- **The explanation lives at the destination, not in the row.** Settings →
  Planning is a new category, because planning covers three harnesses, the
  same reasoning as the `guidance` category. Its panel shows:
  1. **How it works**, in five lines:
     - each place keeps its plan in `.planning/<topic>/` (the three files);
     - `.active_plan` says which plan is current, and Worktrees writes it when
       it creates the place;
     - the Plan tab and your agents always see the same plan;
     - agents Worktrees launches get the current phase at start and when the
       plan changes;
     - nothing is committed, and `.planning/` stays out of git.
  2. **What changes in a repo:** `.git/info/exclude` gains `/.planning/`
     (Q4). It is local and not committed. No other file in your repo is
     written.
  3. **What is NOT changed:** your plans' content, your `.gitignore`, your
     `.claude/settings.json`, and any skill.
  4. **The global choice:** On for all projects / Choose per project / Off.
  5. **The per-project list:** each registered project with its
     inherit/on/off value and a "migrate this project's plans" button
     (§8).
  6. **Turning it off later** (§2.4).
- **Guards.** `offers-check.mjs` covers the new id for both the feed and the
  render, as the rule for `agent-guidance` requires. The mock harness tracks
  the new `planning_status` / `set_planning` commands, because every command
  in `lib.rs` must be tracked there.

### 2.3 Choosing per project: when it is made, loaded or cloned

`add_project` and `clone_project` (`lib.rs:525`, `lib.rs:730`) go through one
door: `add_project`. The dialogs in front of them (Add existing…, Clone…, New
project…) gain one row:

```
[x] Use Worktrees planning in this project   (your default: on · this repo uses it)
```

- It is pre-set from the effective global value. **Add existing** and **New
  project** can also pre-tick it when the repo has a `[plan]` section, because
  the repo is on disk when the dialog shows. **Clone** cannot:
  `clone_project` (`lib.rs:730`) has no repo to read until the clone
  finishes. So for Clone, the checkbox shows the global default only, and a
  `[plan]` hint becomes a post-clone line on the result ("this repo uses
  worktrees planning — turn it on for this project?").
- It writes the registry entry's `planning` **only when it differs from the
  default**. "Inherit" stays the common case, so changing the global value
  later still means something.
- **ProjectSheet → Planning** is where the choice is changed later. It sits
  beside the existing sections and holds the same toggle plus the migrate
  button.
- **Not a project to-do.** `projectTodos.ts` counts what a button can *fix*.
  An unadopted feature is not broken, so it gets no badge. The only nag is the
  machine-level offer, once.

### 2.4 Off, and turning it off later

- **Off means today's behaviour exactly.** `plan::resolve` keeps the
  newest-directory fallback (it mirrors the skill), `cmd_new` writes no
  `.active_plan`, launches carry no planning plugin, and nothing is written to
  `info/exclude`.
- **Turning it off is a toggle.** Nothing is deleted:
  - existing `.planning/<topic>/` dirs and `.active_plan` files stay, and are
    read as today;
  - the `info/exclude` line stays (removing it could start showing hundreds of
    untracked files), and the panel says so;
  - sessions already running keep their plugin's *flags* until restart, but
    the plugin goes **silent at once**: `worktrees plan hook` re-checks
    `planning::effective(project)` on every call and prints nothing when it is
    off. So turning planning off takes effect on the next prompt, not the next
    launch.

---

## 3. What worktrees owns

### 3.1 Layout

```
.planning/
  brief.md            worktrees writes (unchanged, ops::BRIEF_PATH)
  .active_plan        worktrees writes at `new` (on); the session may change it
  <topic>/
    task_plan.md      the session writes
    findings.md       the session writes
    progress.md       the session writes
```

- `<topic>` defaults to the place's slug (Q10). `worktrees new --topic <t>`
  overrides it.
- Main has no `.active_plan` by default. Its plan is `[plan] project` when one
  is declared (§4).

### 3.2 Resolution: one function, made public

`plan::resolve` becomes `pub fn resolve(root, mode) -> Resolution`, where
`Resolution { how, rel, abs, topic }` and `mode` is `Owned | Legacy`.

| Step | Legacy (off; today) | Owned (on) |
|---|---|---|
| 0 | — | main only: `[plan] project` (§4; phase 2) |
| 1 | `.active_plan` → `.planning/<id>/task_plan.md` | see the case table below |
| 2 | newest `.planning/<dir>/` | **removed** (§6) |
| 3 | root `task_plan.md` | root, still reported as `root`; `owned: true` says it is the legacy spot (and `migrate` is offered from phase 2, §8.2) |

**Owned mode, every case.** A pointer that is *missing its plan* is not the same
as a pointer that is *invalid*. The first means "not written yet" and tells
the agent where to write. The second must never tell anyone to write anywhere:
for `.active_plan = evil` with `.planning/evil -> /elsewhere`, "write your plan
in `.planning/evil/`" would instruct the agent to write *through the link*.

| `.active_plan` | What is on disk | Result (`how_resolved`) |
|---|---|---|
| absent | main with `[plan] project` | the project file (`project_key`, phase 2) |
| absent | root `task_plan.md` | root (`root`) |
| absent | nothing | none (`null`); the tab shows the brief |
| present, valid | `.planning/<topic>/task_plan.md` is a regular file under a real (lstat'd) dir | that plan (`active_plan`). **It wins over a root `task_plan.md`** if both exist. |
| present, valid | `.planning/<topic>/` absent, or a real dir without a plan | root `task_plan.md` if one exists (`root`), otherwise `pending` |
| present, **invalid** | value fails `plain_component` (`plan.rs:162`: empty, `.`, `..`, contains `/` or NUL), or `.planning/<topic>` is a symlink or not a directory (`is_dir_nofollow`, `plan.rs:141`), or `.active_plan` itself is a symlink | `invalid_pointer`. The tab says "`.active_plan` is not usable" and falls back to the root plan if there is one. The hook prints "`.active_plan` is not usable — fix or remove it" and **never echoes the pointer's value** (it is session-written text) or any path built from it. |

"Valid" means the value passes `plain_component` and the topic path, if it
exists, is an lstat'd real directory. Only then may anything say "write
here".

**A reused place can show a stale root plan.** A place is durable, and branches
flow through it. A root `task_plan.md` left by the previous branch shows over
a fresh pointer whose plan is not written yet (row 5). The cure is the
close-out removing root files, or `migrate`. It is not a resolution rule,
because the root is the one legacy location a session may legitimately be
writing.

There are four callers, and only one implementation:

1. `summarize`, for the Plan tab and MCP;
2. `worktrees plan resolve [--json]`, for humans and scripts;
3. `worktrees plan show`, which prints the plan, for Codex and any shell;
4. `worktrees plan hook <event>`, for agent hooks (§3.3), which re-checks
   `planning::effective(project)` on every call.

**The contract change, stated honestly.** `PlanSummary` is a contract with the
frontend and MCP ("do not rename fields"). New *fields* are additive:
`owned: bool`, `topic: Option<String>`, and `project: Option<{ rel, title,
current }>` (phase 2). New `how_resolved` *values* are **not** additive: the
field is a closed union in `app/src/PlanPane.tsx:32` (`"active_plan" | "newest"
| "root" | null`) and is matched by MCP clients. So:

- existing values keep their meaning. Owned mode reports a root plan as
  `root`, not as a renamed `legacy_root`; `owned: true` carries the
  difference;
- the new values are `pending` and `invalid_pointer` (phase 1) and
  `project_key` (phase 2). Widening the TS union and the frontend's handling
  of each is listed in phase 1's work (§9), and MCP clients see them only on
  projects that opted in.

### 3.3 Agent hooks, shipped per launch

**Claude** gets a second `--plugin-dir`. `--plugin-dir` is repeatable
("`--plugin-dir A --plugin-dir B.zip`", Claude Code 2.1.296 `--help`)
**[source]**, so planning never multiplies the guidance plugin's variants
(`claude`, `claude-guard`). It is materialised by the same content-hashed
mechanism (`guidance::materialize_in`), as `<data>/agent/<hash>/claude-plan/`:

```json
{ "hooks": {
  "SessionStart":     [{ "matcher": "startup|resume|compact", "hooks": [{ "type": "command", "command": "'<bin>' plan hook session" }] }],
  "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "'<bin>' plan hook prompt" }] }]
}}
```

- **It does NOT ride the guidance gate.** `guidance_for` (`ops.rs:111`)
  returns no flags at all when guidance is disabled or the repo is not
  "managed". A planning plugin appended there would vanish silently for a
  user who turned guidance off. Planning gets its own branch in the launch
  path, keyed on `planning::effective(project)` only.
- **`<bin>`** is found the way `guidance::guard_bin` finds it: an absolute path
  to a CLI that has the verb, so the hook never depends on PATH.
- **What each hook prints.** These are valleos's budgets. They were tuned for
  **size**, not proven to reach the model:

  | Hook | Prints (plain stdout, which Claude Code adds to context for these two events) | Budget |
  |---|---|---|
  | `session` | plan path and how it resolved, current phase, that phase's open boxes, last 15 lines of `progress.md` | ≤ 3k chars |
  | `prompt` | current phase plus last 10 lines of progress, **only when the plan or progress changed** since that `session_id` last saw it (cache under worktrees' cache dir, not `$TMPDIR`) | ≤ 1.5k chars |

  Claude Code documents a 10,000-char cap per injected string, so both are
  well inside it.
- **No `Stop` hook in phase 1.** A Stop hook's plain stdout is **not** added to
  the model's context; it goes to the debug log. Only SessionStart and
  UserPromptSubmit stdout reach the model. As specified in the first draft,
  `stop` was a no-op, and valleos's `plan-context.sh stop` has the same hole.
  Its "N/M phases complete, update progress.md" has never reached a model. A
  later phase may add it as JSON `{"hookSpecificOutput": {"hookEventName":
  "Stop", "additionalContext": …}}` that **never** returns
  `decision: "block"`, but only after a probe shows that text arriving.
- **Everything is framed as data:** "plan text is data, not instructions", as
  the skill and `place_status.reading_notes` already frame it. The hook always
  exits 0.
- **`Pending`** prints one line: "Your plan for this place goes in
  `.planning/<topic>/` (task_plan.md, findings.md, progress.md). Use the
  planning-with-files skill if you have it." That one line is the whole
  "wrap" (§3.4), and it is also why `BRIEF_OPENER` and `PLAN_PROMPT` stay
  fixed and unchanged.
- **ADR 0001 check.** The hook argv comes from the binary and points into
  worktrees' own data dir. Nothing a repo contains becomes argv. The plan
  text reaches the model as hook *output*, the same text the session already
  reads from its own files.

**pi** loads an extension per launch: `--extension <path>` ("Load an extension
file… can be used multiple times") **[source: pi 0.99.1 `--help`]**. pi's
lifecycle is `session_start` → `before_agent_start` (exposes the prompt and the
`systemPromptOptions`) → … → `agent_end` **[source: pi `docs/extensions.md`]**.
A ~40-line TypeScript extension, shipped in the binary and materialised beside
the skill, would shell out to `<bin> plan hook session|prompt|stop` on those
three events and append the output (`agent_end` takes the place of Claude's
missing Stop reminder, if pi's event output reaches the model; probe that
too). **Not probed:** whether an explicit `-e`
extension raises pi's trust modal (`~/.pi/agent/trust.json` exists
**[observed]**), and whether `before_agent_start` can append without
replacing. Phase 3, behind `docs/pi-manual-checks.md`.

**Codex** has no unattended per-launch hook today. `codex features list`
reports `hooks  stable  true` on 0.161.0 **[observed]**, but per-launch `-c`
hooks stop on a trust prompt (AGENTS.md, the Codex rollout note), and a lane
that waits on a modal nobody sees is worse than no hook. So Codex gets:

- `worktrees plan show` named in its `developer_instructions` line, delivered
  only when the user has none of their own (the existing rule);
- MCP `place_status.plan`, which is already there;
- a probe of Codex's hook trust model in phase 3.

### 3.4 Relationship to planning-with-files: wrap

| Option | Verdict |
|---|---|
| **Replace** (ship our own planning skill) | No. The skill is third-party and versioned on its own cadence (2.37.0 here), and users who already have it would get two competing instructions. Vendoring it means owning its method, its templates and its attestation feature. |
| **Coexist untouched** | This is what all three repos do today, and it is why each repo needs a hook: the skill injects only a root `task_plan.md` **[source: SKILL.md frontmatter]**. |
| **Wrap** (recommended) | The skill remains *how* to write a plan. Worktrees decides *where* it goes (the `Pending` line, and `.active_plan` already pointing there), *which* one is current, and *what* is injected. In the owned layout there is no root `task_plan.md`, so the skill's hooks find nothing and no context is injected twice. A lane on a root plan is the exception: worktrees' hook injects it too, so that lane loses nothing. If the skill has been invoked in that session, its frontmatter hooks stay active for the rest of it, and its `PreToolUse` hook `cat`s 30 lines of `task_plan.md` on **every** Read/Write/Edit/Bash/Glob/Grep call. That is far more than "twice", and it is the skill's cost, already paid today in every root-layout lane; worktrees adds one bounded copy at start and on change. It is acceptable for lanes that are on their way out, and `migrate` ends it. Without the skill, the `Pending` line names the three files, which is enough to work by. |

**What the owned layout loses: the skill's tamper check.** The skill's hooks
compare `task_plan.md` against `.planning/<active>/.attestation` (or
`./.plan-attestation`) and refuse to inject a plan whose hash changed (`/plan-attest`).
Its hooks only ever read the root file, so in the owned layout that check
never runs. Worktrees' hook does not carry it in phase 1. It frames the text
as data, which is the same defence the tab and MCP use. If attestation
matters to someone, the hook can honour an existing `.attestation` beside the
resolved plan (refuse and say why on a mismatch) in a later phase; it is a
hash compare, not a new mechanism.

One rough edge, worth reporting upstream as valleos ROADMAP (c) already
intends: the skill's `$PLAN_ID` step cannot be seen by worktrees, so a session
that exports `PLAN_ID` will diverge from the tab. Worktrees ignores it, as
`plan.rs` already documents.

---

## 4. `[plan] project`: the project's tracked plan

```toml
# .worktrees.toml
[plan]
project = "docs/plan/goals.md"
```

- **It is data.** The value is a repo-relative path to a file, never a
  command, so it is on ADR 0001's allowed side, exactly like `[docs] index`.
  It goes into `survey_keys` as section `plan` with the closed key set
  `["project"]`.
  - **Layer A:** `RelPath::parse` refuses absolute paths, `~`, `$` and `..`.
  - **Layer B:** at read time, `docserver::safe_under` (app crate today; it moves to core, because the CLI hook needs it too) canonicalises the path
    and requires the result under the canonical root, then reads with
    `O_NOFOLLOW` and `MAX_READ`. It has several components, so per-component
    lstat alone would not be enough (AGENTS.md on `symlink_metadata`).
- **Main's plan.** For main, step 0 resolves to this file, read from main's
  own working tree. Main's `.active_plan` (valleos/cdv: `orchestrator`), if
  present, still supplies `findings.md` and `progress.md` beside it. That is
  the "exactly the one to show" pairing valleos's hook comment describes (Q9).
- **Lanes never take it as their plan.** Every place's summary instead
  carries `project: { rel, title, current }`, read from **main's** working
  copy, because the orchestrator edits it there and a lane's checkout is
  whatever its branch had. The Plan tab shows it as one line above the lane's
  own plan ("Project: Phase 3, Fase II PoC"). The session hook prints it as
  one line at `session` only.
- **It does not need the opt-in.** A repo that declares the key gets it in
  either mode, because it adds only reads. It ships in phase 2: the pilot
  (cdv) tracks no goals file, so it does not need it.
- **What it retires:**
  - valleos: `sync-goals.sh`, the three `.gitignore` negations, and the
    tracked `.planning/orchestrator/task_plan.md`;
  - cdv #684: the same, plus the pre-push `--check`.

  The tracked copy is the source of the §1.3 hijack, so removing it fixes the
  hijack too.

---

## 5. Lane lifecycle

| Stage | Off (today) | On |
|---|---|---|
| `worktrees new` / MCP `create_worktree` | writes `.planning/brief.md` when given a brief | the same, **plus** `.planning/.active_plan` = `<topic>` and an empty `.planning/<topic>/`; ensures `/.planning/` in `$GIT_COMMON_DIR/info/exclude` (shared by every worktree of the clone). The hook point exists: `cmd_new` already runs `git check-ignore` on the brief and warns when `.planning/` is not ignored (`ops.rs:862`); owned mode turns that warning into the exclude write. |
| Launch | guidance plugin | guidance plugin **+** `claude-plan` plugin (Claude); later the pi extension |
| Working | session writes wherever the skill says | session writes `.planning/<topic>/`, guided by the `Pending` line, then reminded by `session`/`prompt` |
| Plan dock tab | the summary; "Generate plan" | adds: how it resolved in words ("active plan: `<topic>`", "no plan yet", "project goals"); the project line; for a root plan in owned mode, "Move into `.planning/<topic>/`" (phase 2); a topic picker that rewrites `.active_plan` (Q5) |
| MCP `place_status.plan` | as today | adds `owned`, `topic`, `project`, and the new `how_resolved` values (§3.2) |
| Close-out / remove | user's own ritual; removing a place deletes its gitignored plans for good | `worktrees plan archive [<place>] [--to <dir>]` tars `.planning/` (brief + every topic) into `<dir>` (default `~/.cache/worktrees/<project>/plans/<slug>-<date>.tar.gz`); `remove_worktree` runs it first when planning is on (Q6) |

**The `/close-out` skill should not ship with the app.** It is a personal
ritual that encodes `docs/sessions/`, `.claude/close-out.md`, ROADMAP grooming
and branch naming, none of which a worktrees user has. What every user needs is
the one primitive it uses, archiving the planning files, and that is
`worktrees plan archive`. It is also the one step the global skill gets wrong
today: it tars only the three ROOT files (`SKILL.md` §4), so on the owned
layout it would archive nothing. Fix the skill (David's repo) to call the verb.
The `worktrees` guidance skill gains one paragraph naming it.

---

## 6. The stale-dir rule

**Rule: when planning is on, resolution never guesses.** The plan is the one
the pointer names. With no pointer, it is the project key (main) or nothing.

- **Why not a smarter guess?** Every heuristic fails on a case already seen:
  - "Newest dir" picked a June plan in cdv main, and picks the tracked goals
    copy in valleos lanes (§1.3).
  - "Ignore dirs older than the place" fails on main, which is older than
    everything, and on the tracked copy, whose mtime *is* the checkout.
  - "Skip a dir named `orchestrator` outside main" (cdv's suggestion) is one
    project's convention hard-coded into the tool.
- **Why it is safe to remove the guess.** In owned mode, `new` always writes
  the pointer, so the case the guess existed for (a session that made a plan
  dir and no pointer) is fixed at the source.
- **The tab does not hide candidates.** With no pointer and other
  `.planning/<dir>/` plans on disk, it says "No active plan · 2 other plans
  here" and lists them with their ages. Choosing one writes `.active_plan`
  (Q5).
- **Off mode keeps the guess.** It mirrors the skill's `resolve-plan-dir.sh`,
  and changing it would break "off is today".

---

## 7. One code path, and the symlink rule

valleos's `plan-context.sh` checks `! -L` on the **final** `task_plan.md`
only. As a result it:

- follows a symlinked `.planning`;
- follows a symlinked `.planning/<name>`;
- follows a `*/` glob entry that is a link to a directory;
- accepts `.active_plan` values such as `../x` or `a/b`;
- compares mtimes in seconds.

cdv #684 reproduced it injecting `.planning/zz-link/task_plan.md` while the
tab refused it (#684, Finding 1) **[source]**. cdv's port fixes it by
re-implementing `plan.rs` in bash, which is the third copy of the rule and the
next one to drift.

The fix is structural, not another port:

1. **One resolver.** Hooks run `worktrees plan hook`, which calls
   `plan::resolve`. That is the function `summarize` calls, so the hook and
   the tab cannot disagree **at the same version**. They are two binaries:
   the tab runs core in-process inside the app (`place_plan`,
   `lib.rs:5836`), and the hook runs the CLI at `<bin>`. An app and a CLI on
   different releases can resolve differently. `guard_bin`-style detection
   proves the verb exists, not the version. So the hook stamps its version
   in `plan resolve --json`, and the app's existing stale-CLI warning covers
   a CLI behind the app. "By construction" holds only within one version.
2. **The root is found the way the tab finds it.** `Project::discover(cwd)`
   finds the place whose root contains the hook's `cwd`, then canonicalises
   that root. A hook in a subdirectory resolves the same place, not
   `git rev-parse --show-toplevel` of some nested repo.
3. **Two symlink rules, chosen by how the path is built.**
   - `.planning/<id>/task_plan.md` is built from constants plus ONE plain
     component, and every component is lstat'd as `plan.rs` already does.
     That is safe *because* it walks every component.
   - `[plan] project` is a multi-component, repo-supplied path, so it uses
     `safe_under` (canonicalise, then require the result under the canonical
     root), which is the fix shape AGENTS.md names.
4. **A parity test, written to fail first.** It runs cdv's four scenarios,
   plus a symlinked `.planning`, a `../` pointer and `.active_plan = evil`
   with `.planning/evil -> elsewhere` (must be `invalid_pointer` and print no
   path), through both `summarize` and the `plan hook session` output **of one
   binary**, and asserts the same `plan_rel`. It
   must be shown red against a deliberately divergent resolver before it is
   trusted (AGENTS.md, "A new test must be shown to FAIL first").

---

## 8. Migration

### 8.1 The pilot: casa-del-valle-monorepo, in the state it is in today

cdv is the pilot (decided 2026-10-09, relayed from cdv `(main)`). cdv will not
merge its interim port: #684 stays a draft. It will install the release that
ships phase 1, accept the offer, and adopt for cdv. **Its state on 2026-10-09
[observed]:**

- **`main` (`ec7e73cd`).** `.planning/.active_plan` = `orchestrator`, and
  `.planning/orchestrator/{task_plan,findings,progress}.md`, all local and
  gitignored by an unanchored `.planning/`. **Nothing from #684 is on `main`.**
  There is no `scripts/memory/`, nothing under `.planning/` or `docs/plan/` is
  tracked, and `.claude/settings.json` has only the next-server SessionStart
  hook. `old-plans/` holds 203 committed tarballs.
- **Lanes.** Eight use the legacy ROOT layout:
  - `bosanet-and-export-discussions`
  - `chat-history-for-improvements`
  - `feat-training-web-batch3`
  - `feedback-20260804`
  - `fix-factura-forma-pago-default`
  - `general-fixes`
  - `investigate-jorge-agente-roles`
  - `white-label`

  Two have a brief and no plan (`feat-training-mobile-batch2`,
  `feat-training-web-batch2`). Three have neither. Only
  `chore-planning-convention` (#684's own lane) has `.planning/<topic>/` and a
  pointer.

**What adoption does with each, and what it does not do:**

| What is there | After adopting (phase 1) | Moved or written? |
|---|---|---|
| main: `.active_plan` = `orchestrator` + `.planning/orchestrator/task_plan.md` | Resolves at step 1 (`active_plan`) to the same file the tab shows today. The difference is that the Claude hooks now inject it at start and on change, once main's session is relaunched. | **Nothing.** This IS main's plan now. Do not delete it or move it. |
| A live lane with a root `task_plan.md` | Resolves `root` (`owned: true`), still shown as today. The hooks inject it too (§3.3), so a lane on the old layout loses nothing. It finishes and closes out the old way. | **Nothing.** Phase 1 has no automatic move and no `migrate` (it is phase 2). A move under a live session would race it. |
| A live lane with a brief and no plan | No pointer, so owned resolution gives "no plan yet" and the brief, the same as today. Its session may create a root plan, which then shows as `root`. | **Nothing.** Adoption never writes into existing places. |
| `chore-planning-convention` (`.active_plan` = `planning-convention`) | Step 1, unchanged. The `orchestrator/` copy beside it is no longer reachable by a guess (§6). | Nothing. |
| A lane created AFTER adoption | `cmd_new` writes `.active_plan` = `<slug>` and an empty `.planning/<slug>/`. The session hook's `Pending` line says where the three files go. | Written by worktrees: the pointer and the empty dir only. |
| `.gitignore` | `.planning/` is already ignored, so adoption's ignore check passes and `info/exclude` is not touched. | Nothing. |

**One rule this table needs** (it is folded into §3.2 step 1b): a pointer that
names a plan not yet written yields to a root `task_plan.md` if one exists, and
only otherwise means `Pending`. That is not a guess, because the root is one
fixed location the session chose. Without the rule, a new lane whose agent
followed the skill's no-argument default (root) would show "no plan yet" over
a plan that exists.

**Two pilot traps worth stating:**

- **Restart `(main)`'s session after installing.** Lanes are created through
  MCP `create_worktree`. A `worktrees mcp` server started before the upgrade
  keeps running the old image (AGENTS.md, "an MCP bug report describes the
  server that answered it"), so it creates lanes **without a pointer**. That
  reads exactly like adoption failing.
- **The plugin reaches only sessions launched after adoption.** A running lane
  keeps its launch flags until it is relaunched. That is expected, not a bug.

**What cdv must NOT remove:**

- **The `.gitignore` rules:** `.planning/`, `/task_plan.md` …, `**/task_plan.md`,
  `**/findings.md`. Owned planning relies on `.planning/` being ignored, and
  the `**` rules still catch a legacy file written from a subdirectory.
- **`.easignore`'s planning lines.**
- **Main's local `.planning/orchestrator/` and `.active_plan`.**
- **`old-plans/`.**
- **The next-server SessionStart hook in `.claude/settings.json`.**
- **`lane-create`'s `.planning/brief.md` wording,** which is still exactly
  where `new` puts the brief.

**What cdv removes or changes:**

- **Before adopting:** nothing is required. Nothing from #684 landed, so
  there is nothing to undo.
- **Separately, and worth doing anyway:** #684's Finding 5. `.dockerignore`
  on `main` still does not exclude `.planning/` or root `task_plan.md`, so
  planning files, which may hold business data, ride into every API build
  context **[observed: no `.planning` line in `.dockerignore`]**. That is a
  one-line cdv PR, independent of this proposal.
- **After adopting** (cdv's own docs, written by cdv):
  - `AGENTS.md` line ~26 and § Plan lifecycle (line ~406): change "live in
    the worktree root" to `.planning/<topic>/`;
  - its step 2 archive command (`tar … task_plan.md findings.md progress.md`)
    to `tar -czf old-plans/<topic>-YYYYMMDD.tar.gz -C .planning <topic>`,
    keeping the root form for lanes still on the old layout;
  - the planning row in `.claude/close-out.md`, to match.
- **#684 itself:** close it once the pilot runs. Each piece is superseded by
  phase 1:
  - `plan-context.sh` by `worktrees plan hook`;
  - `migrate-lane-plan.sh` by the phase 2 `migrate`;
  - the `.active_plan` brief line, because `new` writes the pointer.

  The goals sync and the tracked copy wait for `[plan] project` (phase 2) and
  should not be revived.

**#684's Findings 3 and 4 do not carry over, by design.** Finding 3 is that
negation order decides which file under `.planning/` is tracked. Finding 4 is
that pushed content must be checked by blob, not by the working tree. Both
exist only because a goals file was tracked *inside* an ignored directory.
Under this design nothing under `.planning/` is ever tracked: `[plan] project`
points at a normal tracked file such as `docs/plan/goals.md`, so there is no
negation to order and no copy to verify.

### 8.2 The other two repos (phase 2, with `migrate` and `[plan] project`)

**`worktrees plan migrate [<place>] [--topic <t>]`** (phase 2) behaves like
cdv's `migrate-lane-plan.sh`, whose eight cases #684 specified and tested:

- it moves root (or `.planning/` top-level) files into `.planning/<topic>/`
  and writes `.active_plan`;
- it is idempotent;
- it refuses:
  - an existing destination;
  - a file present in both places;
  - a second topic while a pointer is set;
  - a non-plain topic;
- it never overwrites;
- it refuses a place whose agent is `busy`.

| Repo | Adopt | Can delete afterwards |
|---|---|---|
| **worktrees** | Turn planning on. Lanes on root plans (6 of 12) keep them, or `migrate` between sessions. | The root-file line in `.claude/close-out.md` (point it at `plan archive`). Rewrite AGENTS.md "Planning docs" to the owned layout. |
| **valleos** | Add `[plan] project = "docs/plan/goals.md"`, then turn planning on. Lanes already have pointers. | `sync-goals.sh` and its gate; `plan-context.sh` and its three hooks in `.claude/settings.json`; the `.gitignore` negations; `git rm --cached .planning/orchestrator/task_plan.md` (main's untracked findings/progress there stay); the "write `.active_plan`" line in its lane-create brief. Delete the tracked copy **in the same PR that adds the key**: a repo with both keeps hijacking lanes that lack a pointer (§1.3). |

---

## 9. Phasing

**Phase 1: the cdv pilot.** It is the smallest slice that lets a project
adopt from the offer and have its agents and its Plan tab agree.

- **Core**
  - `planning.json` (global default) and the registry entry's `planning`
    field.
  - `planning::effective(project)`, read by the app, the CLI and the MCP
    server alike.
- **Resolution**
  - `plan::resolve` made public with `Resolution` and an `Owned | Legacy`
    mode.
  - Owned mode has no newest-dir guess, step 1b's root-then-`Pending` rule,
    root still reported as `root`, and `invalid_pointer` (§3.2 case table).
  - `PlanSummary`: new fields `owned`, `topic`; new `how_resolved` values
    `pending`, `invalid_pointer`. Widen the closed union in
    `app/src/PlanPane.tsx:32` and handle each value.
- **CLI**
  - `worktrees plan resolve [--json]` and `worktrees plan hook
    <session|prompt>` (no Stop, §3.3), with valleos's budgets, re-checking
    `planning::effective` on every call.
  - The parity test (§7, item 4), through one binary, shown red first.
  - bats: `new` writes `.active_plan` + the topic dir only when planning is
    on (and nothing when off); `plan resolve --json` for every row of the
    §3.2 case table.
- **`cmd_new`**
  - When planning is on, it writes `.active_plan` = slug and an empty topic
    dir.
  - An ignore check: if `.planning/` is not ignored, it appends to
    `info/exclude` (Q4). It is a no-op for cdv.
- **Claude**
  - The `claude-plan` per-launch plugin (second `--plugin-dir`), through
    `guidance::materialize_in`, on its own launch branch, NOT inside
    `guidance_for` (`ops.rs:111`).
- **App**
  - The `planning` offer.
  - Settings → Planning: the explanation, the global choice, and the
    per-project list, which is how an already-registered project like cdv
    is adopted.
  - The checkbox in Add existing / New project, and the post-clone line for
    Clone (§2.3).
  - In owned mode only, one line in the Plan tab saying how the plan
    resolved (off mode renders exactly as today).
  - `offers-check.mjs` and mock harness entries.
- **Proof**
  - The `adding-a-harness.md` surfaces, ticked by seeing them, for Claude.
  - One new cdv lane driven end to end in the real app: create it, see the
    `Pending` line, see the plan appear in the tab and in the hook, at the
    same path.

**Not in phase 1, deliberately:**

- `[plan] project` (cdv tracks no goals file today);
- `migrate` (live root lanes finish the old way);
- `plan archive` and archive-on-remove;
- the topic picker;
- a ProjectSheet section (the Settings list covers changing it later);
- pi and Codex.

**Phase 2: valleos and this repo.**

- `[plan] project` (§4), with `safe_under` moved to core and the `project`
  line on every place.
- `worktrees plan migrate` and `plan show`.
- `plan archive` + archive-on-remove (Q6).
- The Plan tab's topic picker (Q5) and migrate button.
- The ProjectSheet → Planning section.

valleos then deletes `sync-goals.sh`, `plan-context.sh` and the tracked copy,
which also ends the §1.3 hijack.

**Phase 3: the other harnesses and hand-started sessions.**

- The pi extension, after a probe of its trust modal (`pi-manual-checks.md`).
- A probe of Codex's hook trust model; per-launch Codex hooks only if they can
  run unattended.
- Optionally, linking the planning plugin for hand-started Claude sessions
  (the agent-guidance §4.3 shape: a link, never an edit to
  `~/.claude/settings.json`).
- The upstream issue on the skill's root-only hooks.

---

## 10. Not recommended

- **A repo-tier switch** (`[plan] enabled = true`). Adoption changes agent
  behaviour on a machine the repo does not own. A repo may suggest it
  (§2.1), never decide it.
- **Editing the repo's `.gitignore`.** That is churn in every adopting repo's
  history, and it needs a PR. `info/exclude` does the same job locally.
  Doctor can report a `.planning/` that is not ignored.
- **Bundling the planning-with-files skill**, or rewriting it (§3.4).
- **Keeping the newest-dir guess in owned mode** "for compatibility" (§6).
- **Hooks through `~/.claude/settings.json`.** That is someone else's live
  file, and the plugin dir already carries hooks per launch.

---

## 11. Open questions for David

1. **Default when the global value is unset: off until chosen?**
   Recommended: yes. The offer asks once.
2. **Whether a repo's `[plan]` section only pre-ticks the checkbox.**
   Recommended: yes, pre-tick only, with the effective value staying user
   tier. The alternative is that it turns planning on for that repo.
3. **Whether owned mode drops the newest-directory fallback entirely.**
   Recommended: yes (§6).
4. **Whether worktrees may append `/.planning/` to the clone's
   `.git/info/exclude` on adoption.** Recommended: yes, local and shown in
   the panel. The alternative is doctor reporting it only.
5. **Whether the Plan tab may write `.active_plan`** (the topic picker,
   "make active"). In owned mode it is worktrees' pointer, not the session's
   content. Recommended: yes.
6. **Whether removing a place in owned mode archives its `.planning/`
   first.** Recommended: yes, to the cache dir, never into the repo.
7. **Whether to ship the `/close-out` skill.** Recommended: no. Ship
   `worktrees plan archive`, and update your skill to call it.
8. **cdv #684 (decided: stays a draft; cdv is the pilot):** whether to
   close it once the pilot runs. Recommended: yes, every piece is superseded
   (§8.1). Separately, land its `.dockerignore` fix (Finding 5) as a one-line
   cdv PR now, because planning files still reach the API build context.
9. **Main with `[plan] project` and an `.active_plan`:** whether the
   orchestrator's `findings.md`/`progress.md` show beside the goals.
   Recommended: yes (§4).
10. **The default topic name.** Recommended: the place's slug. The
    alternatives are `YYYY-MM-DD-<slug>` (the skill's own `init-session.sh`
    shape) or asking.
