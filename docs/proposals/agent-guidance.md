---
title: "Proposal — agent guidance"
---

# Proposal — teach every agent the worktrees paradigm, not just the tools

**Status:** research and proposal, 2026-09-30. Nothing built. A review comes
before any of it is.

**The problem.** An orchestrator Claude session in `(main)` of this repo had the
worktrees MCP tools connected for its whole life. It still did its branch work
(two release bumps and a docs PR) with raw `git worktree add` into a scratch
directory, not in places. The tools reached the agent, but the paradigm did
not. What we ship today tells an agent *what worktrees can do*. Nothing tells
it *that its own work belongs in a place*. That holds for every repo worktrees
manages, not just this one, and for every harness.

**Evidence base.**
- **Claude Code 2.1.285**, `claude-opus-5-5`: run live in 16 headless sessions
  in throwaway repos, on an isolated tmux server. The server lived under a
  scratch `TMUX_TMPDIR` and was killed afterwards (§7). This session's own
  context was also inspected.
- **Codex 0.159.0**: prompts rendered locally with `codex debug prompt-input`
  and a throwaway `CODEX_HOME`. That command makes no model call. Source was
  read at tags `rust-v0.157.0` and `rust-v0.159.0`. **No Codex session was
  run**, because the account was out of tokens.
- **pi 0.99.1**: its shipped docs and compiled source only. It was not run.
  Nothing in its §2 findings needed a model call.
- Nothing under `~/.claude.json`, `~/.codex`, `~/.pi` or any global skill
  directory was edited.

Every claim is marked with its source:

- **[observed]**: I ran it and saw the result, on this machine, against the
  versions above.
- **[source]**: read in the harness's shipped docs, `--help`, or source at the
  named tag. For worktrees, read in this repo at `d0cda58`.
- **[inferred]**: my reasoning, not measured.

---

## 0. The answer in one screen

| Question | Short answer |
|---|---|
| Does a nudge work? | **Yes, on Claude, at n=5.** The same task was given to an orchestrator in `(main)`. With today's MCP instructions, **0 of 5** runs created a place: all five ran `git checkout -b` in `(main)` and left it off the default branch. With an earlier draft of the §3.1 text in the same channel, **5 of 5** did. A skill alone got **3 of 3**. A PreToolUse guard alone got **3 of 3**, but only after a denied command (§7) **[observed]**. |
| The one channel that reaches every session | MCP `initialize` → `instructions`. It needs no setup and reaches hand-started sessions too. It is **not equally honoured**, though. Claude puts it in the system prompt verbatim. Codex turns it into the tool *namespace description*, cut to **250 chars** when the tools are deferred. pi shows it **only** inside `codemode`/`tool_search`, and worktrees installs pi with `--exposure direct`, so a pi lane **never sees it** (§2). |
| What goes in it | About 650 chars, **rule first**. Place-aware: the server already knows which place it serves, so `(main)`, a lane, an automation run and a stray worktree can each be told their role (§3.1). |
| Where the how-to goes | A `worktrees` skill whose text ships inside the binary. Worktrees loads it **per launch, through flags it already controls**: Claude `--plugin-dir`, pi `--skill` + `--append-system-prompt`, Codex `-c developer_instructions` (with a caveat). Nothing is written into a repo, and no harness config file is touched (§4). |
| Guards | The harness-agnostic guard is a **derived warning**: `(main)` off its default branch, or a stray worktree, surfaced in `list_places`/`place_status`/`doctor`/the app. Today `doctor` is silent on the first, and `list_places` reports the second only as a bare `strays` array **[observed]**. There is also a Claude **PreToolUse guard**, shipped in the same per-launch plugin. It is cheap and it works, but it covers Claude only (§5). |
| Freshness | Claude records the system prompt **once per conversation and replays it on every resume until compaction** **[source]**. Updated guidance therefore reaches new conversations, not resumed ones. Version-stamp the text, and let the existing stale-binary warning say so (§6). |
| Surfacing | One more after-update offer in `offers.ts`, `agent-guidance`. It opens Settings → Agent guidance, looks only at machine-level state, and is dismissed by fingerprint like `codex-skills`. It ships with per-launch delivery (§4.5). |
| Proof | A 16-run eval, $3.61 in total, with a script to keep (§7). |

---

## 1. What exists today

| Surface | What it gives an agent | Reaches | Says "work in a place"? |
|---|---|---|---|
| MCP `instructions` (`mcp.rs` `initialize`) | "Worktree management for the repository at … One git worktree per branch, one tmux session per worktree. Use list_places … report / messages / wait. Mutating tools are enabled …" | Every session with the server connected | **No.** It describes the tool, not the agent's own conduct **[source]**. |
| Tool descriptions (`create_worktree` …) | What each tool does | Only once the model loads or reads them | No. Under Claude's tool search they are **deferred**: the model sees names, not descriptions, until it calls `ToolSearch` **[observed]** (this session; and every eval run that called `create_worktree` loaded it with `ToolSearch` first). |
| `ops::BRIEF_OPENER` + `.planning/brief.md` | The lane's task | Lanes launched with a brief | Only if the brief says so. The orchestrator has no brief at all. |
| `worktrees agent-setup` (`agentfiles.rs`) | The CLAUDE.md→AGENTS.md stub layout, and `.claude/skills` → `.agents/skills` links | Repos that accept its PR | No. It moves a repo's own guidance around and has none of its own **[source]**. |
| Skill store + AI profiles (`skillstore.rs`, `profile.rs`) | Skills and `rules.md` (`--append-system-prompt-file`) per profile | Claude launches that use a profile | No, and it is the wrong carrier. It is opt-in per profile, Claude-only, and empty by default **[source]**. |
| This repo's AGENTS.md | Hard-won repo rules | Sessions in this repo | It covers places for releases and close-out, but it did not prevent the miss, and it exists in only one repo. |
| `doctor` `stray-worktree`; `ls --json` `strays` | A post-hoc warning for a worktree outside `.worktrees/` | Humans, and the app | Detects one of the two failure shapes, after the fact. MCP `list_places` carries the same `strays` array, with no words saying what it means **[observed]**. |

---

## 2. Which channel each harness honours

### 2.1 Claude Code 2.1.285

- **MCP `instructions` → system prompt, verbatim** **[observed]**. The text
  appears under a `# MCP Server Instructions` / `## worktrees` heading beside
  the other servers' instructions. It is the only worktrees text the model
  sees before choosing its first tool, because the tools themselves are
  deferred. The eval's A/B arms (§7) confirm that the model acts on it.
- **Frozen per conversation** **[source: `claude --help`,
  `--system-prompt-snapshot`]**. The prompt is "rendered on the
  conversation's first request … every later request and resume sends the
  record as-is, even when a later launch passes different text, until the
  conversation is compacted." That the MCP section is part of that record is
  **[inferred]**; it sits in the system prompt. The sibling behaviour, tool definitions
  that survive a `/mcp` reconnect, is what `stale.rs`'s own warning text
  already tells agents.
- **Skills**: `~/.claude/skills`, `.claude/skills`, and plugin-bundled
  `skills/`. Every skill's description is listed in context from the start,
  and the body loads when it is invoked **[observed]** (this session's own
  skill list; arm C's `Skill` calls).
- **`--plugin-dir <path>`**: "Load a plugin from a directory … for this session
  only" **[source: `--help`]**. A plugin can carry skills **and** hooks. The
  eval loaded a skill-only plugin and a hook-only plugin this way, with no
  settings write, and both took effect **[observed]**.
- **Hooks**: a PreToolUse hook that returns `permissionDecision: "deny"` with a
  reason blocks the call, and the model reads the reason and adapts
  **[observed]** (arm D).
- **`--append-system-prompt[-file]`**: already used by AI profiles. It
  replaces nothing. It is subject to the snapshot above **[source]**.

### 2.2 Codex 0.159.0 (source and local prompt rendering only)

- **MCP `instructions` → the tool namespace's description, not a prompt
  block** **[source: `codex-mcp/src/rmcp_client.rs`,
  `regular_mcp_tool_info_from_listed_tool`: `namespace_description:
  server_instructions`, at both tags]**. When the namespace is *deferred*
  (tool search), the model sees a summary in a "Deferred tool namespaces"
  block. That summary is capped at **`MAX_NAMESPACE_DESCRIPTION_CHARS = 250`**
  **[source: `core/src/context/world_state/tools.rs`]**. Loaded directly, the
  whole text rides on the namespace (cap 512 KiB) **[source: `core/src/tools/handlers/mcp.rs`]**.
  **Consequence: the first 250 characters must carry the rule on their own.**
- `codex debug prompt-input` does not start MCP servers, so it could not show
  this. It is source-only, and not observed live **[observed: absent from the
  rendered prompt]**.
- **`-c developer_instructions="…"` → the first `developer` message**, ahead of
  the skills block **[observed]**. Worktrees already passes per-launch `-c`
  (`harness.rs` `Codex::launch_args`). Caveat: `-c` *overrides* a key
  **[source: `--help`]**, so a user who set their own `developer_instructions`
  would lose it for that launch **[inferred]**.
- **Skills**: discovered from `~/.agents/skills` (user), `$CODEX_HOME/skills`,
  and the repo's `.agents/skills`. Each lands as a description line with a
  path **[observed]**. There is **no per-launch way to add one**: the path
  entries in `skills.config` only select among skills already discovered, and
  one pointed outside the roots did not appear **[observed]**.
- **AGENTS.md** → a `user` message **[observed]**.

### 2.3 pi 0.99.1 (docs and compiled source only)

- **MCP `instructions` → the namespace description, which pi renders in only
  two places**: the `codemode` tool's description (`extensions/codemode/tool.js`)
  and the text `tool_search` matches against (`extensions/tool-search/tool.js`)
  **[source]**. pi's own docs say the same: "The server's `instructions`
  describe its tools in the `codemode` description" (`docs/mcp.md:158`). `pi-ai`'s providers send no namespace description with
  directly exposed tools **[source]**. Worktrees installs pi's server with
  `--exposure direct` (`pimcp.rs`, pi-harness Q9). **So a pi lane never sees
  the instructions at all** **[inferred from source; not run]**.
- **`--append-system-prompt <text|file>`**, repeatable, and **`--skill
  <path>`**, where "explicit `--skill` paths still load" even with discovery
  off **[source: `docs/cli.md`]**. Worktrees already builds pi's argv
  (`Pi::place_flags`).
- A **repo** `.agents/skills` is a trust-protected resource
  **[source: `docs/security.md`]**. Worktrees launches pi with `--no-approve`
  unless it is trusted, so a skill committed to a repo does not reach an
  untrusted pi lane.

### 2.4 Summary

| Channel | Claude | Codex | pi | Reaches hand-started sessions? |
|---|---|---|---|---|
| MCP `instructions` | ✔ full, in the system prompt | ◐ namespace description, 250 chars when deferred | ✘ with `direct` exposure | ✔ |
| Per-launch prompt append | `--append-system-prompt-file` | `-c developer_instructions` (overrides the user's) | `--append-system-prompt` | ✘ |
| Per-launch skill | `--plugin-dir` | ✘ (discovered roots only) | `--skill` | ✘ |
| User-scope skill dir | `~/.claude/skills` | `~/.agents/skills` | `~/.agents/skills` | ✔ |
| Hook guard | ✔ PreToolUse (plugin) | — | extension (declined for pi, pi-harness §4.6) | via plugin only |

---

## 3. Content

### 3.1 Tier (a): the `instructions` text, in every session

Constraints:
- **Rule first.** Codex may show only 250 characters.
- **About 650 characters in total.** It sits in every session's prompt,
  including sessions that never touch a branch.
- **Place-aware.** The server resolves where it runs at startup
  (`Server::here`; `from` already depends on it) and knows whether an
  automation run holds it (`Server::in_run`, from `WORKTREES_RUN_ID`), so it
  can state the role instead of leaving the agent to guess.

**Draft (shared head, 244 chars by `wc -m`):**

> Managed by worktrees: every branch lives in its own PLACE (a git worktree
> under .worktrees/ plus a tmux session). Do branch work in a place:
> create_worktree or `worktrees new <branch>`. Never `git worktree add`; never
> switch branches in (main).

It forbids switching branches in `(main)` only. A lane moving its own place
between branches (parking on a `-next` base) is the paradigm, and §5.2's guard
allows it for the same reason.

**Role line (one of):**

- **`(main)`**: "This repository is `<root>`. You are in (main), the base checkout: keep it on
  the default branch. A place with an agent running belongs to that agent;
  hand work over with a brief instead of editing its tree."
- **A lane** (any other place): "This repository is `<root>`. You are in the
  place `<slug>` (branch `<branch>`); this tree is yours. Do not edit other places' trees; talk to
  their agents with report / messages / wait."
- **An automation run** (`Server::in_run`). The run's cwd is
  `<main root>/.worktrees/`, which is the container and not a place
  (`automation.rs`), so there is no `<slug>` to name: "You are an automation
  run for `<root>` (not in a place); do only what the brief asks and propose
  changes through the run's proposal output, never new places."
- **A stray**: a worktree of this repo that is not a place, which is where
  the original miss worked. `caller_place()` errs there, so today such a
  session gets the generic text: "This directory is a worktree of `<root>`
  but not a place. Move it under .worktrees/ with `git worktree move`, or
  create a place and continue there."

**Tail (unchanged in meaning):** "list_places shows every place; agents talk
through report / messages / wait. Mutating tools are enabled; destructive ones
need confirm: true." (or the read-only variant). Outside a repo the text stays
exactly as it is today.

The eval ran an **earlier** `(main)` draft (662 chars; its head was 328
chars and said "a checkout you did not create") and got 5 of 5 (§7). The
head above is shorter and has not itself been evaluated. Phase 1 re-runs A/B
against it.

### 3.2 Tier (b): the `worktrees` skill, loaded on demand

The description must make it trigger before branch work. The eval's
description was "Use BEFORE any branch work in a repository managed by
worktrees (it has a .worktrees/ directory or a worktrees MCP server) —
creating a branch, fixing something for a PR, starting a lane, releasing, or
closing out", and it was invoked in all 3 runs **[observed]**. Outline:

1. **Model.** A place is a git worktree under `.worktrees/<slug>` plus a tmux
   session. It is durable. A branch flows through it. `(main)` is the base.
   Declared state (lifecycle, pin, note) lives in `.worktrees.places.json`.
2. **Doing branch work yourself.** Use `create_worktree`/`worktrees new`, then
   work in that directory. Give every place its own idle base (`<tree>-next`)
   when parking it. Branch off a freshly fetched `origin/<default>`.
3. **Handing work to a lane.** Write a brief (what makes one good: goal,
   deliverable, rules, stop condition). The brief never goes in argv.
   Provider/model choice. Subscribe to the lane (Claude: `notify_when_idle`;
   other providers: `wait`).
4. **Messaging.** Claude↔Claude uses its own `SendMessage` to the full tmux
   session name. Across providers, use `report`/`messages`/`wait`. `send`
   types into a Codex or pi pane. Check a pane's composer before typing into
   it.
5. **Finishing.** Commit, push and open the PR from the place. After a merge:
   `remove_worktree`, or park the place on its `-next` base. Close out if the
   repo has a ritual (`.claude/close-out.md` and similar).
6. **Releasing from a place.** Tag from the worktree that owns the default
   branch. Never check out one branch in two worktrees.
7. **Never touch.** Another place's tree while its agent runs. A branch
   checked out in another worktree. `~/.claude.json` and every harness's own
   config (read, never write). The user's running app.
8. **When it has gone wrong.** `(main)` is off its default branch: switch it
   back after moving the commits to a place. A stray worktree:
   `git worktree move` it under `.worktrees/`.

Everything in it is repo-agnostic. A repo's AGENTS.md still carries repo
specifics (gates, branch naming), and the skill says so: "the repo's own
AGENTS.md wins on repo rules."

### 3.3 Tier (c): roles

| Role | Knows at start | Must be told |
|---|---|---|
| Orchestrator in `(main)` | Nothing: no brief, no opener | Keep `(main)` on the default branch; branch work goes in a place, its own or a lane's. This is the gap that bit. |
| Lane | Its brief | Its tree is its own; don't reach into siblings; how to report back. |
| Automation run | Its brief, plus the run contract | It is not in a place; stay inside the brief; output goes through the run's proposal, not new places. |
| Session in a stray worktree | Nothing: `caller_place()` errs | It is outside the paradigm; move the tree under `.worktrees/` or continue in a place. This is the original miss's shape. |
| Hand-started session in any place | Only what MCP `instructions` say | Which place it is in, and the rule. Tier (a) covers it on Claude; on Codex and pi it gets only a fragment, or nothing (§2.4). |

---

## 4. Delivery

**The principle:** the guidance is worktrees' own text. It is compiled into
the binary (`include_str!`) and reaches agents by one of two routes:

1. through a channel worktrees already speaks (MCP `initialize`);
2. through argv worktrees already builds.

Neither route writes a file another program owns, or anything inside a repo.
That keeps the `~/.claude.json` rule and the "`pi mcp add`, never write
`mcp.json`" rule intact.

**ADR 0001 check.** The ADR bars *a cloned repo* from supplying argv. Here the
text and the paths come from the binary and from worktrees' own data dir,
never from the repo, so nothing a repo contains becomes argv or a prompt it
did not already reach through its own AGENTS.md. A user override, if one is
wanted (Q3), is user scope (`~/.config/worktrees/`) for the same reason.

### 4.1 Floor: MCP instructions (phase 1)

Ship §3.1. It reaches every harness that honours the channel, including
sessions the user starts by hand, and needs no install step. It is weakest on
pi, where it does nothing; §4.2 covers that.

### 4.2 Per launch, for sessions worktrees starts (phase 2)

On launch, worktrees materialises the text from the binary into
`$XDG_DATA_HOME/worktrees/agent/<version>/`, the same data root as the skill
store. It is rewritten when the version changes. Then:

| Harness | Added to the launch line | Carries |
|---|---|---|
| Claude | `--plugin-dir <data>/agent/<v>/claude` | Plugin: `skills/worktrees/SKILL.md`, plus the §5.2 guard hook (optional) |
| pi | `--skill <data>/agent/<v>/skills/worktrees` and `--append-system-prompt <data>/agent/<v>/rules.md` | The skill, plus tier (a), since pi's MCP channel is dead with `direct` exposure |
| Codex | `-c developer_instructions=<tier (a) + "read the worktrees skill"…>` **only when the user's resolved config has no `developer_instructions`**; otherwise nothing, and `doctor` says so | Tier (a). The skill cannot be added per launch (§2.2), so tier (a) points at `worktrees guide` (below) for the how-to |

Plus one command, `worktrees guide [--role main|lane|automation]`, that
prints the skill body. It gives every harness, including a hand-started
Codex, a way to read the how-to with a tool it certainly has (a shell).

It composes with AI profiles: a profile's `rules.md` and skills still add on
top. Profiles are the user's; this is the tool's.

### 4.3 Opt-in, for hand-started sessions (phase 3)

A Settings / `worktrees agent-setup --guide` step links the materialised skill
into `~/.claude/skills/worktrees` and `~/.agents/skills/worktrees`, which
covers Codex and pi. It adds a directory entry, never edits a file. It is a
symlink into worktrees' data dir, so it stays current with the binary. It is
offered, never done silently: those directories are shared with the user's own
skills (the close-out skill lives there). Status reads the link, the same
shape as `mcpsetup::status`.

### 4.5 Surfacing it: an after-update offer

A user has to learn that this exists at the moment they install or update the
app. Otherwise per-launch delivery changes how their agents behave without
telling them, and the opt-in links (§4.3) are never found. The app already has
a channel for exactly this: `app/src/offers.ts`. Guidance joins it as one more
row, beside `mcp-server` / `codex-mcp` / `pi-mcp` / `codex-skills`. A separate
lane is making offers reopenable (Settings → What's new lists them, and a
bottom-right indicator shows pending ones), so a row is all this needs.

```ts
{
  id: "agent-guidance",
  title: "Teach your agents to work in places",
  body: "Agents worktrees launches now get the places rule and a worktrees skill. Choose the guard, and whether hand-started sessions get it too.",
  cta: "Review…",
  to: { cat: "guidance", focus: "agent-guidance" },
  fingerprint: "v1:" + unlinked.join(","),  // guidance major version + the harnesses still unlinked
}
```

Rules, taken from the ones `offers.ts` already states:

- **The machine, never a repo.** Its input is a new machine-level Tauri
  command, `agent_guidance_status`. It returns the guidance version, the
  per-launch state per harness, the guard setting, and the §4.3 link state
  for each installed harness. It must not come from a project's
  `agent_setup_status`: an offer that needs a repo in hand is the v0.25.0
  precondition bug.
- **An offer, not a problem.** It fires only for `absent`, meaning an
  installed harness whose link does not exist. A link that points somewhere
  else (`foreign`), or at an old data dir (`stale`), is a problem. Problems
  belong in the destination panel, where they cannot be silenced.
- **Fingerprint and dismissal work like `codex-skills`.** Dismissal records
  the set the user was shown. The offer asks again only when something **new**
  appears: a harness installed later, or a **major** guidance version, which
  is a real change in what agents are told. A text tweak inside a major
  version, or a link added from Settings, stays quiet.
- **The destination is where the choice lives.** `to` opens a Settings
  panel, "Agent guidance", which is its own category because it covers three
  harnesses (same reasoning as the `claude` category's comment). The panel
  shows:
  - what agents get (`worktrees guide`'s text);
  - per-launch delivery per harness, including "Codex: skipped — you have
    your own `developer_instructions`";
  - the guard toggle (Q2);
  - the §4.3 links, with link and unlink buttons.
- **Guards in the repo.** The new id and its fingerprint are covered by
  `offers-check.mjs`, feed and render both. The mock harness gains
  `agent_guidance_status`: every command in `lib.rs` must be tracked there.

### 4.4 Not recommended

- **Writing guidance into repos.** That means `agent-setup` committing a
  worktrees section to AGENTS.md, or a repo-local `.agents/skills/worktrees`.
  It is per repo, it shows up as churn in every repo's history, it is stale
  the day the tool changes, and it does not reach untrusted pi. The paradigm
  belongs to the tool, not the repo.
- **MCP `prompts`.** They are user-invoked slash commands, so they do not
  help an agent that does not know it should ask.
- **MCP resources** (a `worktrees://guide`). The model must decide to read
  one, which is the exact decision that failed. At most, keep one as a pointer
  target.
- **Routing it through AI profiles.** Profiles are opt-in, Claude-only and
  per profile, so the default user gets nothing.

---

## 5. Guards

A nudge fixed the eval outright (§7), so guards are a belt, not the fix. Two
kinds are worth having, at different costs.

### 5.1 Derived warnings (every harness; recommended, phase 1)

Both failure shapes are visible in git state.

| Shape | Seen by | Today |
|---|---|---|
| `(main)` on a branch other than its default | the eval's baseline, 5 of 5 | `list_places` shows `(main)` with `"branch": "fix-typos"` and no warning. `doctor` says "No worktrees", nothing else **[observed]**. |
| A worktree outside `.worktrees/` | the original miss | `doctor` `stray-worktree` warns, and the app flags `strays`. MCP `list_places` includes `strays` as bare data (`path`, `branch`, `slug`) **[observed]**. |

Proposal:

- A `main-off-default` doctor code: Warn, never promoted by `--strict`, same
  reasoning as `stray-worktree`.
- A matching field on `(main)` in `ls --json`, plus a line in `place_status`.
- One `warnings` line per finding in MCP `list_places`/`place_status` (`"(main) is on fix-typos, not main — move the commits to a place"`). Beside the data, not instead of it: a model reads a sentence more reliably than an array it was never told to check **[inferred]**.
- The app's existing strays flag, extended to the first shape.

This is cheap, harness-agnostic and read-only. It tells the next agent, and
the human, that something is off.

**What "default" means.** `Project::default_base()` is `main` or `master`
by existence, else **`(main)`'s current HEAD**. In a `develop` or `trunk`
repo, a wandering `(main)` therefore redefines "default", and the check can
never fire. The check must read `refs/remotes/origin/HEAD` when present and
fall back to `default_base()` only without it.

**Why it matters beyond style.** `base_ref()` is what every place's ↑↓ and
every new branch is measured from. A `(main)` left on a feature branch
corrupts both: in a no-remote repo directly, and in any repo whose default
is not `main`/`master`.

**Where it goes.** One `warnings: Vec<String>` on `LsJson`
(`model.rs`) reaches `ls --json`, MCP `list_places` and the app at once,
with no per-surface code.

**Caveat.** Some users deliberately work in `(main)` on feature branches. The
warning should name the fix and be dismissible per repo; a declared flag in
`.worktrees.places.json` would do. It must not refuse anything.

### 5.2 Claude PreToolUse guard (Claude only; phase 2, default on, can be switched off)

It ships in the per-launch plugin (§4.2), so it installs nothing globally.
The rule is cwd-aware:

- deny `git worktree add` anywhere in a managed repo;
- deny `git checkout -b` / `git switch -c` **only in `(main)`**. A lane moving
  its own place between branches (`-next` bases) is the paradigm, not a
  violation.

The deny reason names `create_worktree`. In the eval every guarded run
recovered into a place, and it cost about +3 turns and +$0.07 per run (§7).

- **Cost:** a regex over Bash commands has false negatives: aliases, scripts,
  `git -C`. It also cannot see a hand-started session.
- **Benefit:** it catches the one irreversible-feeling moment for exactly the
  agent that already ignored §3.1.

Codex has no comparable hook we would use, and a pi extension was already
declined (pi-harness §4.6). For those two, §5.1 is the guard.

### 5.3 Not recommended

Refusing in MCP tools (nothing in the failure path calls them), or a
filesystem watcher (heavy, and §5.1 already derives the same fact on every
`ls`).

---

## 6. Freshness and visibility

- **Version the text** with its own token, `(worktrees guidance v1)`,
  embedded in the binary beside the text and bumped only when the text
  changes. `stale.rs` today compares only binary versions. It gains a second
  marker read the same way, so a binary change that leaves the guidance alone
  says nothing new. When the guidance marker differs, the warning already
  appended to tool results adds "agent guidance changed; start a new
  conversation to pick it up". It says *new conversation*, not *reconnect*,
  because of the snapshot (§2.1).
- **Why not `--system-prompt-snapshot off`** for worktrees-launched Claude
  sessions: it re-renders the prompt on every request, which gives up the
  prompt cache that the recorded prompt keeps warm. That cost lands on every
  turn of every lane, to pick up a text change that happens a few times a
  year.
- **Resumed sessions keep the old text until compaction** **[source]**. That
  is acceptable: resumed lanes are mid-task, and §5.1 still fires for them.
- **Per-launch material is versioned by directory**
  (`agent/<version>/`), so a running session's plugin path never changes
  under it. Old versions are swept at app start.
- **Visibility.**
  - `worktrees guide` prints what agents get.
  - `worktrees agent-setup status` gains a "guidance" block: the version,
    the per-launch delivery per harness (on, off, or skipped because Codex
    has user `developer_instructions`), and the opt-in links.
  - Settings shows the same block beside MCP setup.
- **Override** (Q3): none in phase 1. The text is the tool's contract, and a
  user who disagrees has AI-profile rules and their own AGENTS.md.

---

## 7. Measurement

### 7.1 What was run

A throwaway repo (`README.md` with "recieve", `greet.py` with "Helo"). The
prompt: *"Fix both and get the change ready for review as a PR: commit it on a
new branch. There is no git remote here, so do not push or open the PR; just
leave the branch committed and tell me its name."*

Setup:
- `claude -p` from `(main)`: Opus 5.5, Claude Code 2.1.285, default settings.
  This machine has **no** global `~/.claude/CLAUDE.md` and no
  `~/.claude/rules`. The user-scope skills and plugins were loaded as usual.
  None of their descriptions mention places; the close-out skill's *body*
  mentions `git worktree list`, but it was never invoked.
- `--strict-mcp-config` with only a `worktrees` server behind a stdio proxy
  that rewrites `initialize.instructions`. Every arm used the proxy, so the
  text was the only variable.
- `WORKTREES_AI_CMD=none`, so `create_worktree` never started a nested agent.
- An isolated tmux server, killed afterwards.
- A tool allowlist: Read/Edit/Write/Grep/Glob, `git`, `worktrees`, and the
  MCP tools.

The outcome was classified from the transcript and the repo state.

| Arm | Instructions | Extra | Runs | Created a place | What happened otherwise | Mean cost |
|---|---|---|---|---|---|---|
| A | today's text | — | 5 | **0** | 5× `git checkout -b` in `(main)`; `(main)` left on the feature branch | $0.18 |
| B | earlier §3.1 `(main)` draft (§3.1 note) | — | 5 | **5** | 3× `create_worktree`, 2× `worktrees new`; `(main)` untouched | $0.23 |
| C | today's text | skill via `--plugin-dir` | 3 | **3** | skill invoked every run, then `create_worktree` | $0.23 |
| D | today's text | PreToolUse guard via `--plugin-dir` | 3 | **3** | each tried `checkout -b`, was denied once, recovered | $0.28 |

**[observed]**, 16 runs, $3.61 in total. Artifacts (transcripts, repo states,
proxy, plugins, runner, classifier) are in the lane's scratch cache.

### 7.2 What it does and does not show

- It **does** show that the gap is real and default: A is 0 of 5 on a trivial
  task. It also shows that the channel we already own closes it on Claude.
- It **does not** show:
  - long sessions, where early instructions compete with a long context;
  - tasks where the agent has repo-specific reasons of its own (this repo's
    miss was a *release*, where AGENTS.md's "tag from the worktree that owns
    `main`" may have pushed it to a scratch worktree **[inferred]**);
  - Codex or pi at all;
  - n large enough to separate B from C.
- **The prompt primes the failure.** "Commit it on a new branch" invites
  `git checkout -b` in place. A prompt that said only "open a PR" might fail
  differently, or less often.
- **Arm C is also a prompt-level nudge.** Skill descriptions are in context
  from the start (§2.1), so C shows that a well-written description works. It
  does not show that an on-demand body works where tier (a) would not.
- The baseline failure shape (`checkout -b` in `(main)`) differs from the
  original miss (raw `git worktree add`). Both are "branch work outside a
  place", and §5.1 needs to detect both.

### 7.3 Keep it

Commit the runner as `scripts/agent-guidance-eval.sh`: the proxy, the
template repo, arms selected by flag, and the classifier. Run it whenever
§3.1 or the skill changes. Extend it:

- a "cut a release" task and a "hand this to a lane" task (checks that a
  brief is written);
- Codex once tokens allow (`codex exec`);
- pi with a local model through `lm-studio`, with its `--append-system-prompt`
  arm.

It stays **manual**, like `docs/ai-profiles-manual-checks.md`. It spends real
tokens and must never run in CI.

---

## 8. Phased plan

**Phase 1: the floor (small; one PR).**
- `mcp.rs` `initialize`: the §3.1 text, rule first, with role by own place.
  All four roles ship here, including automation (`Server::in_run`) and
  stray (`caller_place()` erring).
- Unit tests that pin the head at ≤ 250 chars (`chars().count()`), and that
  the head contains `create_worktree` and "Never `git worktree add`".
- `main-off-default` diag code (default read from `origin/HEAD` first, §5.1);
  `warnings: Vec<String>` on `LsJson`, so `ls --json`, MCP `list_places` and
  the app get it at once; a line in `place_status`.
- The eval script committed. Re-run it with arm A on the shipped binary
  (`~/.local/bin/worktrees`) and arm B on the release build, with no proxy.

**Phase 2: per launch.**
- Materialise `agent/<version>/` from binary constants.
- Claude `--plugin-dir` (skill, plus the guard behind a setting, default on).
- pi `--skill` + `--append-system-prompt`.
- Codex `-c developer_instructions`, guarded by a read of the user's resolved
  config.
- `worktrees guide`.
- The `agent-guidance` after-update offer (§4.5), and the Settings → Agent
  guidance panel it opens: what agents get, per-launch state per harness, and
  the guard toggle. It ships in the same release as per-launch delivery,
  because that is the release that changes agent behaviour.
- `agent_guidance_status` (machine-level) in `lib.rs` and the mock harness;
  `offers-check.mjs` covers the new id.
- Walk `docs/adding-a-harness.md`: every harness adapter gains a
  "guidance" surface, which a future opencode adapter picks up.
- Manual check in the real app: launch one lane per harness and confirm the
  agent can quote the rule.

**Phase 3: hand-started sessions and visibility.**
- Opt-in user-scope skill links, in the same panel. The offer's fingerprint
  now includes the unlinked harnesses, so a harness installed later asks again.
- The `agent-setup status` guidance block and the Settings panel.
- The guidance version marker wired into the stale warning (§6).
- The app flag for `main-off-default`.

---

## 9. Open questions

1. **The `(main)` rule's strength.** "Keep `(main)` on the default branch" is
   this repo's practice. Is it the tool's? Some users run single-place repos
   and work in `(main)`. Options:
   - state it as the default and have §5.1 accept a declared opt-out; or
   - soften it to "prefer a place" for repos with no other places.

   The recommendation is the first option: Warn, never promoted by
   `--strict`, with a declared opt-out. A wandering `(main)` is not only a
   style problem, because `base_ref()` measures every place's ↑↓ and every
   new branch from it (§5.1).
2. **Guard default.** Should the Claude PreToolUse guard (§5.2) ship on by
   default in phase 2, or opt-in? The eval says it works. The cost is false
   positives from users who meant it.
3. **Override.** Should a user be able to replace or extend tier (a) (a
   `~/.config/worktrees/agent-guidance.md`)? The recommendation is no for
   phase 1.
4. **Codex `developer_instructions`.** Skip it when the user has their own
   (the recommendation), or prepend ours to theirs? Either way, worktrees has
   to know whether the user has one. Re-implementing Codex's config layering
   (profiles, `-c`, project config) would drift. Running `codex debug
   prompt-input` at `doctor`/status time and caching whether a leading
   developer message exists is lighter. It asks Codex itself and makes no
   model call (§2.2).
5. **pi exposure.** Instead of `--append-system-prompt`, should the pi install
   switch from `direct` to a mode where pi renders the instructions? That
   reverses pi-harness Q9. Per-launch append is the lighter fix.
6. **Automation runs.** Should they be forbidden from `create_worktree`
   entirely, with the guidance just stating the contract, or allowed with a
   proposal?
7. **Opt-in skill links (§4.3).** Worth the Settings surface, or is phase 2
   (worktrees-launched sessions) plus tier (a) enough in practice?

8. **The offer's destination.** Should Agent guidance be a new Settings
   category (proposed, §4.5), or a section inside AI profiles? Profiles are
   per-profile and Claude-only, which is why the proposal keeps them apart.

---

## 10. Probe record

- **Claude eval (§7).** 16 × `claude -p` in `…/eval/runs/<arm><n>/repo`,
  with `TMUX_TMPDIR=…/eval/tmux`, `TMUX` unset and `WORKTREES_PREFIX=ev<arm><n>`.
  The isolated server was killed afterwards. The real tmux server was checked
  and carries no `ev*` sessions. The Claude transcripts for these runs live
  under the scratch paths' entries in `~/.claude/projects/`, if they should
  be removed.
- **Codex (§2.2).** `codex debug prompt-input` with
  `CODEX_HOME=…/probe/codexhome` in a throwaway repo, plus `-c
  mcp_servers.worktrees…`, `-c developer_instructions=…` and `-c
  skills.config=[{path=…}]`. No model call. `~/.codex` was untouched. The
  throwaway `CODEX_HOME` holds only what Codex created: `installation_id`,
  `skills/.system`, `tmp`. Source was read via the GitHub API at
  `rust-v0.157.0` and `rust-v0.159.0`.
- **pi (§2.3).** Read `docs/{mcp,skills,cli,security}.md` and
  `dist/extensions/{mcp,codemode,tool-search}` in the managed 0.99.1 install.
  Nothing was run.
- **worktrees.** `list_places` over stdio and `doctor` against an arm-A repo
  (§5.1).
