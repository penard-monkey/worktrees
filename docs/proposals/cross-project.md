---
title: "Proposal — cross-project reach"
---

# Proposal — let an agent reach places in other projects

**Status:** research and proposal, 2026-10-01, reviewed. The decisions below
are folded into the text. Nothing here is built.

**The problem.** A session can only see and address the places of its OWN
project. The orchestrator in one repo's `(main)` cannot ask how a lane in
another repo is doing, cannot `report` to it, and cannot `wait` on it. The
app's drag and drop says the same thing out loud: drop a place from project B
onto a Claude session in project A and the app refuses, with *"a session can
only reference worktrees from its own project"*. The user works across several
repos at once. The app already holds all of them in one nav. The agents cannot
follow.

**Evidence base.**
- **worktrees 0.34.1**, the installed release binary. Its `worktrees mcp` was
  driven over stdio in two throwaway repos (`alpha`, `beta`, one lane each)
  under a scratch cache directory (§10).
- **This repo at `17c240a`** (origin/main, 2026-10-01), read for every
  `[source]` claim.
- **Claude Code's own `ListAgents`**, called from this session (§4.3).
- **Timings** of `worktrees ls --json` and `git worktree list` over the nine
  projects in the author's app workspace. Only counts and times are reported,
  never names (§7).
- Nothing under `~/.claude.json`, `~/.codex` or `~/.pi` was written. The
  app's `projects.json` was read, never written. The running app was not
  driven. No Codex session was run.

Every claim is marked with its source:

- **[observed]**: I ran it and saw the result, on this machine, against the
  versions above.
- **[source]**: read in this repo at `17c240a`, or in a client's documented
  behaviour as already recorded in this repo.
- **[inferred]**: my reasoning, not measured.

## Decisions

Taken on 2026-10-01, after the review:

- **Q2, default reach: OFF.** Read-only cross-project reach is opt-in. It is
  offered after the update by an `offers.ts` card that deep-links to the
  setting. Mutations stay a separate opt-in on top of it.
- **Q3, private projects: a bare row.** Agents elsewhere see the project's
  name only, with no places and no details. Addressing into one is refused
  with a reason that names it.
- **Q1, registry owner: core, at user level** (`~/.config/worktrees`). The
  CLI, the MCP server and the app all read it. The app's `projects.json`
  migrates into it by **union-merge**. Registering a project is the consent
  act.
- **Q5, drag and drop in P1: all three harnesses.** Claude, Codex and pi
  panes all accept the plain-text cross-project address in P1.

The review also changed three parts of the design:

- **The project handle is the registry entry's `name`, not the session
  prefix** (§3.2). The prefix can be set by the repo itself, so a handle
  derived from it could be spoofed.
- **A cross-project `reply_to` is validated against the sender's log**
  (§4.2). Validating it against the log being written makes a round trip
  impossible.
- **Registry writes are read-modify-write under a lock** (§2.2). One writer
  per file stops being true once the CLI writes the registry too.

---

## 0. The answer in one screen

| Question | Short answer |
|---|---|
| Where does the restriction live? | In **one line of `cmd_mcp`**, and everything downstream of it. `Project::discover(&root)` pins one project at startup, and every tool resolves slugs, the message log, tmux names and the caller's own place against that one value (§1). It is **by design**, not by accident. The module note says *"No tool takes a repo path, so a model cannot walk the server into another checkout."* That sentence is the safety property this proposal has to keep in a new form, not delete. |
| Where does the list of projects come from? | Today, only the app knows it: `projects.json` in the app's config dir, a JSON array of main roots. Core and the CLI never read it [source]. **Decided:** core owns it, as `~/.config/worktrees/projects.json` beside `profiles.json` and `skills.json`. The CLI, the MCP server and the app read it. The app writes through core and union-merges its own file in once. Every write is read-modify-write under a lock. Registering a project is the consent act (§2). |
| How is a foreign place named? | **`<project>:<slug>`.** `:` cannot appear in a branch name, so no slug worktrees creates contains one, and the resolver tries an exact local slug first anyway. A bare slug keeps meaning "my project", byte for byte. `<project>` is the **registry entry's `name`**, which is user tier and unique by construction. It is seeded from the session prefix at registration. It is never the prefix itself, which a cloned repo can set (§3). |
| How do new abilities reach clients that cache tool schemas? | Through a **new tool** (`list_projects`) and **new values in existing string fields** (a qualified `slug`/`to`). No existing tool's schema changes in phases 1–2. Results teach the form, the same way `create_worktree` results already carry a capability line for clients with stale schemas (§3.3). |
| Cross-project messages? | **Route to the recipient's own log.** `report alpha:x` writes into alpha's git common dir. `to` is stored bare there. `from` stays server-derived, qualified as `<registry name>:<my slug>` whenever it crosses a project. A cross-project `reply_to` is checked against the sender's own log. `messages` and `wait` keep reading only the caller's own log, which is where its mail lands (§4). |
| Safety default? | **Reach is off until the user turns it on** (decided). An after-update offer links to the setting. **Read-only reach is the first opt-in, and mutations are a second one on top**, and `remove_worktree` never crosses a project. A **per-project `private` flag** takes a project out of reach in both directions, and it shows to agents elsewhere as a bare name. All of it is user-only config that a repo cannot set (§5). |
| Drag and drop? | **The foreign-project drop stops being refused.** It types a plain-text **address**, `place alpha:lane-x`, instead of an `@worktrees:place://…` token, into Claude, Codex and pi panes alike (decided). The address works for any server version that understands qualified slugs, and it fails loudly on one that does not. A token would fail silently: the client only resolves URIs that are in its cached resource list. The `@`-mention form follows later, once the server lists foreign places as resources (§6). |
| Cost? | A full `ls` over 9 projects and 101 places took **2.2 s** serially. `git worktree list` took **12–21 ms** per project [observed]. So the cross-project list is the **cheap index** (slug, branch, lifecycle) per project. The full state is fetched for **one** place, on demand (§7). |
| Plan | P0 registry → P1a resolver + registry names → P1b Settings, private and the drag-drop address → P2 cross-project messaging → P3 opt-in mutations → P4 foreign `@`-mentions (§8). |

---

## 1. Where the restriction lives

### 1.1 The MCP server **[source: `crates/worktrees-cli/src/mcp.rs`]**

| Site | What it assumes | By design or by accident |
|---|---|---|
| Module note, "Safety shape" | *"The repo is pinned at startup… No tool takes a repo path."* | **Design.** The property is "a model cannot aim the server at an arbitrary checkout". The registry (§2) keeps that property, because a model can only *name* a project the user registered. It can never hand over a path. |
| `cmd_mcp` | `root` = `CLAUDE_PROJECT_DIR` (Claude) or the cwd. One `Project::discover(&root)`, stored as `Server.project: Option<Project>`. | Design for the *caller's* identity, which must stay single. Accident for the *reach*: nothing needs the reach to equal the identity. |
| `caller_place()` | The deepest place of THE project containing `here` becomes `from`. | **Design, and it stays.** A session lives in exactly one place. Cross-project only qualifies the name (§4.2). |
| `known_slug()` | A slug is valid if `self.proj()?.ls()` lists it. | Accident. It is also the most expensive validation in the file: a full `ls` fan-out just to check that a name exists. `place_index()` answers the same question (§7). |
| `place_status`, `wait until: idle` | Look the slug up in the one project's `ls()` / `place_dir()`. | Accident. |
| `report` / `messages` / `wait until: message` | `messages::dir(project.git_common)`. | Storage is per repo by design (§4.1). Routing only to the caller's repo is an accident. |
| `send` | `project.session_name(slug)`. Ownership is "sessions **this project** created". | Accident in the lookup. The ownership rule is sound and generalises to "sessions the TARGET project created". |
| `show_doc` | The canonical path must lie under the project's main root. | Design. Phase 3 could widen it to "under any reachable project's main root" through the same canonicalise-then-`starts_with` check. |
| `resources()` / `read_resource()` | `uri_map(project.place_index())` → `place://<slug>`. | Accident. It is also the part that blocks the `@`-mention drop (§6). |
| `spawn_list_watcher` | Watches ONE `wt_root` plus ONE sidecar. | Accident. Has to watch every reachable project once foreign places are resources (P4). |
| `initialize` instructions | The role line names `{root}`. `MESSAGE_NOTE` says *"another place of this project"*. | Wording. Both change with the feature (§9). |
| `managed()` | Its comment says *"the app's project list is not readable from here"*. | A gap the registry closes (§2). |
| `hub_copy_refusal` | Checked against the SERVER's own main root before any non-read-only tool. | Must be checked against the **target** project as well. A foreign target that is a hub copy is exactly as dangerous as a local one. |
| `HIDDEN_IN_RUN` | Six tools vanish inside an automation run, among them `send` and `remove_worktree` (the other four are the automation tools themselves). | Design, and it stays. A run gets no cross-project reach at all (§5.1). |

### 1.2 Core **[source]**

- **`Project`** is `{main_root, git_common, wt_root, prefix}`. Nothing in core
  holds more than one project at a time, except the app's loop over
  `projects.json`.
- **`messages`** lives at `<git-common>/worktrees-messages/`. Each file is
  `<id>.<hex(to)>.json`, with read markers per recipient. `from` and `to` are
  bare slugs, checked only for length and control characters. The location
  was chosen so that *"a sandboxed agent whose home is not the user's still
  reaches it"* (module note). That reason matters for §4.1.
- **`mention`** builds `@<server>:place://<slug>` tokens. The client resolves
  a mention by an exact match against its **cached** `resources/list`, and
  takes the first match. That is why the app asks core for the token rather
  than building it.
- **Session names** are `<prefix>-<slug>`. The prefix defaults to the main
  root's **basename** (`resolve_prefix`), unless the env, a
  `.worktree-prefix` file, `.worktrees.toml` or the user config overrides it.
  Two repos with the same basename therefore already collide, in tmux and in
  Claude's `--name` (§3.2).

### 1.3 The CLI **[source: `crates/worktrees-cli/src/main.rs`]**

Every command except `mcp` and its setup verbs passes a git guard that runs
`Project::discover(cwd)` and exits if it fails. Running the CLI in a repo is
one-project **by design**: a person typing `worktrees new` means *here*.
Nothing in this proposal changes the CLI verbs. A `worktrees projects`
subcommand (P0) is the one CLI addition, and like `mcp --status` it runs
outside a repo.

### 1.4 The app **[source: `app/src-tauri/src/lib.rs`, `app/src/App.tsx`]**

- `projects.json` (`projects_file`, `read_projects`, `write_projects`) is a
  plain `Vec<String>` of canonical main roots. Its order **is** the nav order.
  `add_project`, `remove_project` and the nav reorder write it.
- `list_workspace` snapshots each root, four at a time.
- The app shows **no** message log. Nothing in `lib.rs` reads `messages::`.
- Drag and drop has two cross-project refusals. They mean different things:
  - **Tier zone:** *"a worktree belongs to its project — it can't move to
    another one."* Correct forever. Moving a worktree between repos means
    nothing.
  - **Terminal (`mention`):** *"a session can only reference worktrees from
    its own project."* The comment beside it gives the reason: *"The MCP
    server is pinned to the repo it was launched in and no tool takes a repo
    path, so a foreign project's slug cannot resolve in that session — the
    token would be dead text."* This refusal goes away with this proposal
    (§6).

### 1.5 What the probe showed **[observed, 0.34.1]**

Two scratch repos, `alpha` and `beta`, each with one lane. The server ran in
`alpha/.worktrees/alpha-lane`:

```
list_places                        → ["(main)", "alpha-lane"]
place_status {slug: "beta-lane"}   → no such place: beta-lane
report {to: "beta-lane"}           → no such place: beta-lane
place_status {slug: <beta's path>} → no such place: <path>
report {} (to (main))              → filed in alpha/.git/worktrees-messages
messages, run in beta              → []
```

The restriction is complete and consistent. There is no partial path that
half-works today.

---

## 2. The project registry

### 2.1 Options

| Option | For | Against |
|---|---|---|
| **(a) Read the app's `projects.json`** | Zero migration. It is already the user's list. | Its path depends on the app's bundle identifier: the sandbox uses `….sbx`, and Linux uses a different base. A CLI-only user has no file. The app owns it as a whole-blob writer, the same shape as `ui-state.json`, so core could only ever read it. |
| **(b) A core-owned registry, `~/.config/worktrees/projects.json`** | One owner, one path on every OS, the same root as `profiles.json`, `skills.json` and `config.toml`. Works without the app. `worktrees projects add/rm/ls` can write it too. | A one-time migration of the app's file. The app has to go through core for every write, including the reorder. |
| **(c) Discover from live tmux sessions or git common dirs** | No list to maintain. | A project with no running session is invisible. It also turns "which repos exist" into a property of what is running right now, which is the wrong shape for a permission. |
| **(d) Explicit project paths in tool arguments only** | No registry at all. | Breaks the "no tool takes a repo path" property outright. A model could aim the server at any directory on disk. **Rejected.** |

### 2.2 Decided: (b), with the app as a client

- A new **`worktrees_core::registry`**: `read_lenient()`, `add(root)`,
  `remove(root)`, `reorder(roots)`, `rename(root, name)` and
  `set_private(root, bool)`. Each entry is `{root, name, private?}`. The
  array order stays the nav order.
- **Every write is read-modify-write under a lock, then temp + rename.**
  "One writer per file" stops being true the moment `worktrees projects add`
  exists beside the app. The app's `reorder_projects` already treats its list
  as a preference: `merge_project_order` drops roots the file no longer has
  and keeps, appended, roots the frontend never saw **[source: `lib.rs`]**.
  But it reads, merges and writes with no lock, so an `add` that lands in
  between is lost. In core the lock covers the read **and** the write, for
  add, remove, rename and set-private as well as reorder. The reorder rule
  moves into core as is: order applies to present entries, unknown ones are
  kept and appended. The app **re-reads after every write** and never holds
  the list as truth.
- **Migration is a union-merge** (decided). On every start until it is
  marked done, the app merges its own `projects.json` into the core
  registry. Core entries keep their place, app-only roots are appended in the
  app's order, and duplicates are keyed by canonical root. The old file is
  left in place, so a downgrade still has a list.
- **Registering is the consent act** (decided). A project an agent can reach
  is one the user added, through the app or `worktrees projects add`. Nothing
  else adds to the registry.
- **The registry is user-only.** No `.worktrees.toml` key may add a project
  to it, rename one or mark one non-private. The same `USER_ONLY_KEYS`
  mechanism that holds `trust` and `model` covers `cross_project`.
- **`managed()`** gains the signal its comment says it lacks: a repo in the
  registry is managed.
- **Nesting.** A registered root can lie inside another registered project's
  tree, for example a repo checked out under another repo's ignored
  directory. The caller's identity stays what it is today, the project
  `Project::discover` finds at launch. The registry entry it is matched to is
  the one whose root equals that project's canonical `main_root`, never the
  nearest enclosing entry. `doctor` flags nested entries, because a person
  reading the nav will not expect it.

---

## 3. Addressing

### 3.1 The form: `<project>:<slug>`

- **Unambiguous by construction.** A slug is a directory name derived from a
  branch (`ops::slugify`, `/` → `-`). Git forbids `:` in branch names
  (`git check-ref-format --branch a:b` is fatal; `(main)` passes)
  **[observed]**, so no slug that worktrees created contains `:`. A `:` in a
  `slug`, `to` or `wait.slug` therefore means "qualified". The one exception
  is a directory someone made by hand under `.worktrees/` with a `:` in its
  name. The resolver tries the exact local slug **first**, so such a place
  stays reachable locally. It just cannot be addressed from elsewhere.
- **Backwards compatible.** A bare slug resolves exactly as today. Every
  existing call and every existing test keeps its meaning.
- **`(main)` qualifies like any place:** `alpha:(main)`.
- **Self-qualification is allowed.** `<my project>:<slug>` resolves to the
  local place, so an agent can copy an address from `list_projects` without
  knowing which project it is in.

### 3.2 What `<project>` is: the registry entry's `name`

**Not the session prefix.** The first draft used `Project.prefix`. The review
found that the prefix is **repo-supplied**. `resolve_prefix` reads
`WORKTREES_PREFIX`, then a `.worktree-prefix` file (which can be committed),
then `.worktrees.toml`'s `[project] prefix`, which is a REPO-tier key and not
in `USER_ONLY_KEYS`. Only after those does it read the user config and the
basename **[source: `project.rs` `resolve_prefix`, `projcfg.rs`]**. A cloned
repo could set `prefix = "alpha"` and:

- make the real `alpha` ambiguous, so it cannot be addressed at all; and
- if the handle were derived per caller from its own prefix, sign messages
  `from: "alpha:(main)"`.

So the handle is **the registry entry's `name`**:

- **User tier.** Only the user writes the registry (§2.2).
- **Seeded from the prefix at registration,** so the default still reads the
  same as tmux and `ListAgents`. The prefix is read-only input here, never
  the handle.
- **Unique by construction.** `add` refuses a name already taken, or
  suffixes it `name-2`. The app's add flow shows the result and lets the user
  rename it. Two clones of the same repo get two names.
- **The qualified `from` comes from the registry, not from the caller.** The
  server takes its project's canonical `main_root`, finds the entry with
  that root, and uses its `name`. A caller in an unregistered repo has no
  cross-project identity and no reach.
- **Names follow `safe_arg` plus `[A-Za-z0-9._-]`.** No `:` and no `/`, so a
  name never splits an address and fits the P4 URI form.

**The session-name gap stays visible.** `session_name` replaces `.` with `-`
(`project.rs`), and `ListAgents` shows the mangled form. An entry named
`my.app` is therefore addressed `my.app:lane` while its sessions read
`my-app-lane`. `list_projects` returns each place's agent session names, so
an agent never has to derive one from the other.

**Prefix collisions remain a tmux bug.** Two repos with the same prefix
already share a tmux namespace (§1.2). They no longer break addressing, but
`doctor` gains a code for colliding prefixes among registered projects, and
the app shows a badge on such a project.

Rejected handles:
- **A path.** It breaks the module note's property (§2.1 d).
- **The prefix** (above).
- **An opaque id.** Nobody can read it.

### 3.3 Frozen schemas

Tool definitions are cached by some clients at connect time. The code
already plans for that: `answer()` appends a capability line to
`create_worktree` and `place_status` results because *"a current server
cannot observe the client's cached schema. Results do refresh, even when
definitions don't"* [source]. So:

- **Phase 1 adds one tool, `list_projects`,** and changes no existing schema.
  A client with cached definitions does not see `list_projects` until it
  re-lists. It can still use qualified slugs in the fields it already has,
  because those are free strings.
- **The same capability-line mechanism advertises the form**, appended to
  `list_places` and `place_status` results **only when reach is not `off`**,
  and **once per session**: the first such result carries it, later ones do
  not. Text: *"Places in other projects are addressed `<project>:<slug>`;
  list_projects lists them."* A line on every result would be noise in every
  session that never leaves its repo.
- **`list_places` stays local.** Its contract is "this repository", and a
  caller that iterates it to act on every place must not suddenly act on
  other repos. Each entry gains a `project` field (additive): the registry
  name, or null in an unregistered repo.
- **Phase 3 adds `project` to `create_worktree`.** That is a schema change.
  An old cached schema simply lacks the parameter, and a call without it
  keeps today's meaning. **[inferred]** No client is known to reject an
  unknown argument before sending it; the server's own `additionalProperties:
  false` is what decides, and the server is the new one.

### 3.4 `list_projects`

```json
{
  "projects": [
    { "project": "alpha", "this": true,
      "places": [ { "slug": "(main)", "address": "alpha:(main)", "branch": "main", "lifecycle": null,
                    "sessions": ["alpha-(main)"] },
                  { "slug": "lane-x", "address": "alpha:lane-x", "branch": "lane-x", "lifecycle": "saved",
                    "sessions": ["alpha-lane-x"] } ] },
    { "project": "beta", "places": [ … ] },
    { "project": "client", "private": true }
  ],
  "note": "Summary only. place_status <address> for one place's live state."
}
```

- The **cheap index** only: `place_index()` plus the declared sidecar, which
  is the same budget `resources/list` already keeps (§7). `sessions` is what the
  name that `session_name` gives, a pure function with no tmux call.
- **No `root`, unless reach is `full`.** A root is a home-directory path.
  Through a profile it can land in another model account's conversation, and
  read-only reach has no use for it. With `full`, `show_doc` and
  `create_worktree` need it, so it is returned.
- A `private` project is **a bare row**: its name, `"private": true`, and
  nothing else (decided). Addressing into it is refused with a reason that
  names it: *"client is private"*.
- A registered root that no longer exists is one row with `error`, as
  `list_workspace` already does for one dead repo.

---

## 4. Messaging across projects

### 4.1 Route, do not share

The log stays per repo. A cross-project `report` **writes into the
recipient's repo log**:

- Each place's agent keeps reading exactly one log, its own. `messages` and
  `wait until: message` need no change beyond accepting a qualified `slug`
  filter.
- **Rejected: a user-scope shared log.** It would give up the property the
  module note names, *"needs no `$HOME` — a sandboxed agent whose home is not
  the user's still reaches it."* It would also make every repo's mail one
  file set, so one project's 500-message cap evicts another's.
- **Risk: whose sandbox.** The write is done by the **`worktrees mcp`
  process**, not by the agent's shell. What matters is whether that process
  runs inside the harness's sandbox.
  - **Codex.** Its auto-review launch adds exactly one writable root, the
    lane's own git common dir: `sandbox_workspace_write.writable_roots =
    [<own git_common>]` **[source: `codex.rs` `permission_flags`]**. That
    binds Codex's **shell tool**. Whether it also binds the MCP servers Codex
    spawns is **not measured**. **[inferred]** They are started by Codex
    itself, outside the per-command sandbox, so the write probably succeeds.
    P2 measures it before shipping. An auto-review Codex lane in `beta`
    reports to `alpha:(main)`; check that the server's write lands, and that
    a shell `touch` into `alpha/.git` from the same lane is refused, so the
    check is known to be able to fail. Codex had no tokens for this round.
  - **Claude.** Claude's sandbox setting covers its Bash tool. worktrees
    does not turn it on in any launch. A Claude whose user enabled it is in
    scope for the same measurement, but not a blocker.
  - **If a write is refused,** the error says so plainly (*"this session
    cannot write to beta's message log (sandbox?); report to your own
    (main) instead"*) rather than surfacing a raw `os error 1`.

### 4.2 `from` across a project boundary

- `from` stays **derived, never an argument**. Today it is `caller_place().slug`.
- **When the recipient is in another project**, the server writes
  `from: "<registry name>:<my slug>"`. The name comes from the registry entry
  whose root is the caller's canonical `main_root` (§3.2), never from the
  prefix and never from an argument. Local messages keep the bare slug, so
  nothing in an existing log changes meaning.
- **`to` is stored bare in the recipient's log.** The file name's
  `hex(to)` and the `.read/<hex(to)>/` markers keep working unchanged,
  because the recipient reads its log by its own bare slug.
- **`reply_to` is validated against the SENDER's log for a cross-project
  post.** `messages::post` checks that `reply_to` names a message in the log
  it is **writing** **[source: `messages.rs` `post`]**. For a cross-project
  reply that is the recipient's log. But the message being answered sits in
  the replier's own log, where it arrived. Checked as today, A→B→A fails
  with *"reply_to names no message in the log"*. So `post` gains a
  `reply_log: &Path` parameter, the caller's own log for a cross-project
  post and the target log otherwise, and checks there. Checking somewhere is
  better than recording it unchecked: the id is still a real message the
  replier received, and a typo is still refused. The bats round trip
  (P2) is the witness, and it must fail on the unchanged `post` first.
- A reply then needs no other rule. `report {to: <the from it received>}` is
  a valid address, qualified or not.
- `messages::post`'s `check_slug` accepts `:` already (only length and
  control characters are checked) [source]. Nothing about storage changes.
- **Own-place guards compare resolved pairs.** `report` refuses `to ==
  me.slug` and `wait until: idle` refuses the caller's own place, both by
  comparing **bare strings** today **[source: `mcp.rs`]**. A
  self-qualified `alpha:(main)` from `alpha`'s `(main)` would pass both. Both
  guards compare the resolved `(project root, slug)` pairs instead.
- `MESSAGE_NOTE` gains: *"A `from` with a `project:` prefix came from another
  repository."* That is the honest framing for an agent that has to weigh it.
- **Trust stays at the user account,** as the module note already says:
  anything running as the user can write any log directly. Qualification is
  attribution, not authentication.

### 4.3 How this interacts with Claude's own messaging **[observed]**

`ListAgents`, called from this session, listed **seven peer sessions across
four different projects**, each by its full tmux session name
(`<prefix>-<slug>`). So **Claude↔Claude already crosses projects today**. The
missing piece is discovery: a Claude session has no worktrees tool that tells
it *which* name belongs to *which* foreign place, or whether that agent is
busy.

So after P1:
- `place_status beta:lane-x` returns the foreign place's `agents[].name`. A
  Claude orchestrator uses that name with `SendMessage`, as it already does
  locally.
- `send` to a foreign Claude place keeps answering `Elsewhere` with that
  session name. It never types into a Claude pane, and that does not change.
- The worktrees log (P2) remains **the cross-provider bus**: Claude↔Codex,
  Codex↔pi, and any pair where one side cannot `SendMessage`.

### 4.4 `wait`

- `until: idle` on `beta:lane-x` resolves the target project and polls
  `activity::place_activity(beta, …)`. It belongs in P1 because it only reads.
- `until: message` with `slug: "beta:lane-x"` filters on the qualified
  `from`. It belongs in P2, with the qualified `from`.

---

## 5. Safety and permissions

### 5.1 Tiers

| Capability | Default | Gate |
|---|---|---|
| `list_projects`, `place_status`/`wait idle` on a foreign place | **Off** until the user turns it on (decided) | User setting `cross_project` (`off` / `read` / `full`; **default `off`**). An after-update offer in `offers.ts` deep-links to it. |
| `report` to a foreign place (P2) | With `read` | It only appends to a log, which is the same tier it has locally (it is in the read-only tier today) |
| `send`, `create_worktree`, `close_session`, `set_note`/`set_pin`/`set_lifecycle` on a foreign place (P3) | **Off** | `cross_project = "full"` **and** the server's own `--mutations` |
| `remove_worktree` on a foreign place | **Never** | Not offered at any setting. It is the one path that can destroy commits (`force` + `--branch`, AGENTS.md). Cross-project is exactly where an agent knows least about another repo's branches. |
| Anything cross-project inside an automation run | **Never** | `in_run` narrows to the local project, the same way `HIDDEN_IN_RUN` narrows tools |

- **One user-only setting, not a per-call grant.** A model cannot raise its
  own reach. The precedent is `--mutations`: *"enabling it is a profile
  decision the user makes once, not something a session can grant itself."*
  A profile may narrow the setting (`cross_project = "off"` for a
  client-work profile), never widen it. **Profile narrowing ships in P1**,
  with the setting itself.
- **The setting is read at server start,** like `--mutations` and `in_run`.
  A change reaches new sessions, not running ones. Settings shows
  *"restart sessions to apply"* beside the control, as the MCP setup wizard
  does for `--mutations`. AGENTS.md records why: the server a session holds
  is the one it started with.

### 5.2 `private` projects

A user who keeps one client's repo away from another's agents marks it
`private` in the registry. A private project:

- is **not reachable from** other projects (its places are not listed or
  resolved, its log is not written); and
- **cannot reach out** (its own server serves only itself, whatever
  `cross_project` says).

Both directions matter **[inferred]**. Listing branch names, notes and plan
goals of client B to an agent working in client A sends B's metadata into a
conversation about A, and possibly to a different model account through a
profile. The reverse direction lets A's content land in B's agent as a
message or a brief.

### 5.3 ADR 0001

ADR 0001 says a cloned repo never supplies argv. Cross-project reach adds no
argv channel:

- A foreign `create_worktree` (P3) launches the **target** project's agent
  with the AI command resolved for the **target** repo, from the user's flags,
  env or config, exactly as a local create does. The brief is data written to
  `.planning/brief.md`, which is what it is today.
- **The new thing is content crossing a boundary, not argv.** Repo A's files
  can prompt-inject A's agent, and that agent can now write a brief or a
  message into repo B. That is the reason mutations are opt-in, and the reason
  `MESSAGE_NOTE`'s "not the user's instruction" framing applies to every
  qualified `from`.
- **No repo-supplied key may influence reach.** `cross_project`, `private` and
  the registry are user tier only. A `.worktrees.toml` that names them is a
  hard parse error, through `USER_ONLY_KEYS`.

### 5.4 Per-target guards

Each of these is evaluated against the **target** project, never the caller's:
the hub-copy refusal, `send`'s "sessions this project created" ownership rule,
pi trust (for a P3 `create_worktree` with `provider: "pi"`), and the
`.pi/mcp.json` shadow check. **The hub-copy refusal reaches `report` in
P2:** `report` and `messages` carry `readOnlyHint: false` **[source:
`mcp.rs`]**, so the guard in `call` already applies to them. It must test
the target's root for a cross-project `report`, because writing mail into a
hub copy is lost on the next pull just as any other write is. Each is a one-line change of which `Project` is
passed. Each needs its own test, because getting it wrong looks like working:
the caller's project passes the check for a target that should fail it.

---

## 6. Drag and drop

### 6.1 Today **[source: `App.tsx` `resolveDrop`/`commitDrop`, `lib.rs` `drop_reference`, `mention.rs`]**

- Dropping a place onto a terminal resolves to a `mention` target **only** in
  a Claude pane (`TerminalPane` sets `drop="mention"` only when `provider ===
  "claude"`).
- `drop_reference(repo, slug, into_slug, into_session)` discovers `repo` (the
  **dragged** place's project), builds `place://<slug>` with `uri_for`, and
  asks `server_name_for(repo, into_slug, …)` which server name the
  **target** session registered. Then it pastes ` @<server>:place://<slug> `
  into the pane's AI.
- A foreign drop is refused in `resolveDrop` before any of that runs.

**A latent bug this proposal has to fix first.** `server_name_for` reads the
*target* slug's `profile_id` stamp from the *dragged* repo's sidecar. That is
correct only while the two repos are the same one, which the refusal
guarantees today. Lift the refusal without splitting `repo` into `from_repo`
and `into_repo`, and the server name is looked up for a slug in the wrong
project. The lookup would either miss and fall back to user scope, or hit a
same-named place and read a different session's profile. Either way the
token can name a server the session does not have, and the notice reports
success.

### 6.2 Why not just a foreign `@`-token

Claude resolves `@server:uri` only against its **cached** `resources/list`,
and silently [source: `mention.rs`]. For a foreign token to resolve, the
target session's server must:

1. list foreign places as resources, which needs §7's cheap index across
   projects and a watcher over every reachable project; **and**
2. be a **new enough server**, while sessions keep the server they started
   with (AGENTS.md: *"An MCP bug report describes the server that answered
   it"*).

The app cannot see which server version a session holds. A foreign token
dropped into a session with an older server is dead text under a notice that
says it worked. That is precisely the failure the current refusal exists to
prevent.

### 6.3 Recommendation

**P1: drop a foreign place as a plain-text address.**

- The drop types ` place beta:lane-x ` (a stable phrase plus the qualified
  address). The `@`-token stays for same-project drops, which still resolve
  as they do today.
- A model that sees it calls `place_status beta:lane-x`. On an old server the
  call fails with *"no such place: beta:lane-x"*. That is loud, visible to
  the agent, and fixed by restarting the session. It is never a silent
  nothing.
- **The address is built by core,** not the frontend. It goes in a
  `mention::address(project, slug)` beside `mention::mention`, for the same
  reason the token is: two producers of one string drift.
- **The text form needs no MCP resource, so P1 takes it to Codex and pi
  panes too** (decided). Today `TerminalPane` gives only Claude panes a drop
  target. Three things change for the other two:
  - `TerminalPane` sets the drop target for every harness, and `resolveDrop`
    picks **token or address** per pane. A Claude pane with a same-project
    place gets the token, as today. Everything else gets the address,
    including a same-project drop into Codex or pi, which has no `@`-resource
    either.
  - `drop_reference` passes the pane's provider to `tmux::paste_to_ai`,
    which already takes an `ai_word`. Its error text is hardcoded to "no
    Claude running" and gets the provider's name instead.
  - **The paste is bracketed** (`paste-buffer -p`) and never submits
    **[source: `tmux.rs` `paste_to_ai`]**. That is what keeps a Claude
    permission prompt from reading it as an answer. For Codex and pi the same
    property is **not verified**. P1b refuses a drop into a Codex or pi pane
    whose activity is `waiting` (an approval or question on screen, the same
    reading `send` uses to refuse) until each composer is checked by hand.
    The checks go into the existing `docs/pi-manual-checks.md` and its Codex
    counterpart.
- **When reach is off or the target is private,** the drop is still refused,
  with the reason: *"cross-project reach is off (Settings → Agents)"* or
  *"beta is private"*. Never with today's sentence, which will no longer be
  true.
- The drag chip reads `→ reference beta:lane-x in this session` for a foreign
  drop, so the user sees before letting go that it is an address and not an
  inline mention.

**P4: foreign `@`-mentions,** after resources span projects (§8, P4). By then
the URI form `place://<project>/<slug>` fits the client's menu charset (which
admits `/`, per the charset quoted on `safe_uri_part`), and the app can switch to the token once it can
tell the session's server is new enough. One way: the server writes its
version beside its probe, and the app reads it. Until then, the address stays.

### 6.4 What does not change

The **tier-zone** refusal (*"a worktree belongs to its project"*) is correct
and stays. Dragging a project header still only reorders projects. Neither
reorder writes anything a session can see.

### 6.5 Testing it

- `dnd-check.mjs`-style: a script that slices the real `resolveDrop` and
  asserts each case. Foreign plus reach on gives `mention` with `foreign:
  true`. Foreign plus reach off gives `reject` with the Settings hint.
  Foreign plus private gives `reject` naming the project. Same-project gives
  today's token path, unchanged.
- The mock harness (`install.ts`) learns the `drop_reference` signature with
  `into_repo`, plus the setting and `private`. The AGENTS.md rule is that it
  tracks every command.
- **Core:** `drop_reference` unit tests for the repo split. Same-named slugs
  in two repos with different `profile_id` stamps must resolve the
  **target's**. The test must be shown to fail on the unsplit code first.
- **Real app, end to end** (AGENTS.md's rule that walking `docs/adding-a-harness.md` exists to
  enforce: "Core reports it" is not "the user sees it"). Use `sandbox.sh --app`, two scratch
  projects, and a Claude in A. Drag B's lane onto A's pane, see the address
  arrive in the composer, send it, and watch the agent call `place_status` and
  get B's state. Repeat with a Codex pane and a pi pane in A, and with reach
  off to see the refusal notice.

---

## 7. Cost

**[observed]** Nine registered projects, 101 places, release 0.34.1, serial:

| | per project | all 9 |
|---|---|---|
| `worktrees ls --json` | 0.13–0.93 s (the 0.93 s is the 50-place project) | 2.20 s |
| `git worktree list --porcelain` | 12–21 ms | ~130 ms |

Consequences:

- **`list_projects` is the cheap index only:** one `git worktree list` plus
  one sidecar read per project, ~130 ms here. That is fine for an on-demand
  call. Live state (dirty, ahead/behind, tmux, agents) belongs to
  `place_status` on **one** place, which costs one `place_one` exactly as
  `resources/read` does today.
- **`known_slug` should move to `place_index()` while it is being touched.**
  Today each `set_note`, `report` and `send` pays a full local `ls` just to
  check a name exists. For a foreign project it would pay that project's
  `ls`, with no use for the result.
- **No caching in P1 [inferred].** Each call is a fresh read and the cheap
  index is cheap. A cache adds the staleness question for a benefit nobody has
  measured as missing.
- **The P4 watcher is the real cost.** Each live session polls every
  reachable project's `wt_root` plus its sidecar every ~2 s. With 9 projects
  and ~10 sessions that is ~45 `read_dir` + sidecar reads per second
  machine-wide. Each read is microseconds, but that is the multiplier to
  measure in P4 before shipping. The alternative is to watch only the
  registry, and refresh foreign membership on `resources/list`.

---

## 8. Phased plan

### P0 — the registry (no agent-visible change)

- **Core:** `registry.rs` (read/add/remove/reorder/rename/set-private;
  read-modify-write under a lock, then temp + rename; lenient read;
  `{root, name, private?}` entries with unique names seeded from the
  prefix); `USER_ONLY_KEYS` for `cross_project`; a `doctor` code for nested
  entries.
- **App:** `read_projects`/`write_projects`/`reorder_projects` go through
  core and re-read after every write. Union-merge migration of the app's
  file.
- **CLI:** `worktrees projects [ls|add|rm|rename]`, which runs outside a repo
  like `mcp --status`.
- **Tests:** core unit tests (union-merge keeps both sides; idempotent add;
  a name collision suffixes or refuses; reorder keeps an entry the caller did
  not know about; a missing root); a lock test where an `add` between a
  reorder's read and write survives, shown to fail without the lock; bats
  `projects` with two scratch repos; `cargo test -p app --lib` for the
  migration.
- **Verified by:** the real app (sandbox) reorders and adds projects with the
  nav unchanged, and `worktrees projects ls` prints the same order.

### P1a — resolver and addressing (no UI)

- **mcp.rs:** a `Server.reach` resolver (the registry minus private
  entries; empty when the setting or the profile says `off`, inside a run, in
  an unregistered repo, or when the caller's own project is private);
  `resolve(addr) -> (Project, slug)` keyed on registry names, exact local
  slug first; `list_projects` (no `root` below `full`; private as a bare row);
  qualified `slug` in `place_status` and `wait idle`; own-place guards on
  resolved pairs; `known_slug` on `place_index()`; `project` field on
  `list_places` rows; the once-per-session capability line; the
  instructions line (§9).
- **Core:** `mention::address`; `cross_project` setting with profile
  narrowing (decided for P1); the prefix-collision `doctor` code.
- **Tests:** mcp unit tests (bare slug unchanged; qualified resolves; a
  repo-set prefix cannot claim another project's name; private refused with
  its name; in-run, `off` and profile-`off` refuse with a reason;
  `alpha:(main)` from `alpha`'s `(main)` hits the own-place guard); bats with
  two scratch repos driving `worktrees mcp` over stdio as §10 did.
- **Verified by:** `place_status` from A's Claude on a busy lane in B showing
  the same `activity` as B's nav dot, in the sandbox app.

### P1b — Settings, private, and the drag-drop address

- **App:** Settings → Agents gets the `cross_project` select with *"restart
  sessions to apply"*; the project sheet gets the `private` toggle and the
  rename; a collision badge on a project header whose prefix collides; an
  `offers.ts` card for the new setting (decided), dismissed by fingerprint
  like the others and guarded by `offers-check.mjs`.
- **Drag and drop:** `drop_reference` split into `from_repo`/`into_repo`;
  the drop target on every harness's pane; token or address chosen per pane;
  refusals with Settings/private hints; Codex/pi `waiting` refused; mock
  harness updated.
- **Tests:** a `dnd` check script over the real `resolveDrop`; the
  `drop_reference` repo-split test, shown to fail first; Codex and pi
  composer checks by hand.
- **Verified by:** the §6.5 end-to-end run, in all three harnesses.

### P2 — cross-project messaging

- **mcp.rs:** `report` to a qualified `to` writes the target log with a
  qualified `from`; `wait until: message` accepts a qualified filter;
  `MESSAGE_NOTE` updated.
- **mcp.rs/core:** `messages::post` checks a cross-project `reply_to`
  against the sender's log (§4.2); `to` stored bare; the target's hub-copy
  refusal applies to `report`.
- **Tests:** bats round trip A→B→A with `reply_to`, shown to fail on the
  unchanged `post` first; a message from A is
  invisible in A's own `messages`; a private target refuses; a sandbox-write
  failure gives the plain error (unit test with an unwritable dir).
- **Verified by:** a Claude in A and a pi or Codex lane in B exchanging a
  report and a reply in the real app. Measure the Codex sandbox write (§4.1)
  then.

### P3 — opt-in cross-project mutations

- `cross_project = "full"` + `--mutations`: `send`, `create_worktree
  {project}`, `close_session`, metadata setters, `show_doc` under any
  reachable root. Never `remove_worktree`.
- Every per-target guard from §5.4, each with a test that runs the caller in
  a clean project and the target in a failing one.
- **Verified by:** an orchestrator in A creating a lane in B with a brief,
  the lane appearing in B's nav, its agent opening on the brief, and its
  `report` reaching A.

### P4 — foreign `@`-mentions

- `resources/list` gains foreign places as `place://<project>/<slug>`; the
  watcher spans reachable projects (measure §7 first); the server's version
  becomes readable by the app; the drop switches to the token when the
  session's server is new enough.

---

## 9. Agent guidance and UI

- **Instructions (tier a):** one sentence, only when reach is not `off`:
  *"Places in other registered projects are reachable read-only as
  `<project>:<slug>` (list_projects); they belong to other repositories, so
  hand work over by report rather than editing their trees."* With `full`,
  the sentence says mutations are allowed too.
- **The skill (#393):** a short "Across projects" section covering when to
  look (the user names another repo, a dependency is being changed
  elsewhere), the address form, that `list_places` stays local, and that a
  qualified `from` is a colleague in another repo, not the user.
- **App:** the Settings select, the `private` toggle and rename, the
  after-update offer, the drag-drop changes (§6) and the collision badge, all
  in P1b. There is no message-log UI today and this proposal does not add one.

---

## 10. Probe record

- Scratch repos at `~/.cache/worktrees/worktrees/cross-project/probe/{alpha,beta}`:
  `git init`, an empty commit, and `git worktree add .worktrees/<r>-lane`.
  Throwaway repos, not checkouts of this one.
- Driven with a three-line stdio script: `initialize`,
  `notifications/initialized`, one `tools/call`. `WORKTREES_MCP_PROVIDER=codex`
  pins the cwd as the place. The tmux server was isolated with a scratch
  `TMUX_TMPDIR`.
- Timings: `worktrees ls --json` and `git worktree list --porcelain` in each
  registered root, run from Python's `subprocess` with wall-clock timing.
  Only the counts are reported.
- `ListAgents` was called once, read-only. Project names from it are not
  reproduced here.

---

## 11. Open questions

1. **Registry owner.** Core-owned `~/.config/worktrees/projects.json`, or
   core reading the app's file? **Decided:** core, at user level, read by
   the CLI, the MCP server and the app. The app's file migrates in by
   union-merge, and registering a project is the consent act.
2. **Default reach.** `read` or `off`? **Decided:** `off`. Read-only reach is
   an opt-in offered after the update by an `offers.ts` card, and mutations
   are a second opt-in on top.
3. **Private projects in `list_projects`.** A bare row, or invisible?
   **Decided:** a bare row with the name only. Addressing into one is refused
   with a reason that names it.
4. **The project handle.** **Settled by the review:** the registry entry's
   user-tier `name`, seeded from the prefix and unique by construction
   (§3.2). The prefix is repo-supplied and could be spoofed.
5. **Drag and drop in P1.** **Decided:** all three harnesses, Claude, Codex
   and pi, get the plain-text address in P1.
6. **`remove_worktree`.** "Never across projects" is the recommendation.
   Should `full` ever allow it with `confirm: true` plus the project name
   typed out? **Open.**
7. **A profile narrowing reach.** **Settled by the review:** in P1 (P1a).
8. **The Codex sandbox write (§4.1).** Does Codex's auto-review sandbox bind
   the MCP server it spawns, or only its shell? **Open**, and measured in P2
   before cross-project `report` ships.
