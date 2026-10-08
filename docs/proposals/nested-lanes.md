---
title: "Proposal — lanes that own lanes"
---

# Proposal — lanes that own lanes

**Status:** investigation and design, 2026-10-08. Not implemented. Decisions
requested in §11; no permission to merge or implement is implied.

**Recommendation.** Call every place a **lane** in user-facing prose. A hub is
an ordinary lane with permission to own children, on a stable integration
branch. Keep the worktrees physically flat, declare their parentage, and cap
the tree at `(main) → hub → child`. Children squash into the hub; the hub
merges main periodically and ultimately squash-merges into main. Ship
per-project tmux routing and restart-based draining **before** nesting; socket
recovery is a separate follow-up. Ship the non-launch `-N` hotfix independently.
Keep GitHub writes in the agent's existing `gh` workflow, never in the app.

## Decisions already taken

David's brief fixes close-out at both levels, one user-facing word for place/lane,
one tmux server per project, and a read-only GitHub app.

**Depth decision, relayed by main on 2026-10-08:** main → hub → children is the
cap, and main → children remains a first-class option. A hub is opt-in, never
required. Both shapes coexist in navigation and creation. MCP `create_worktree`
defaults to a child of main unless a parent/hub is explicitly given. No migration
turns existing flat lanes into hubs or adds a hub step to their workflow.

**Review policy decision, relayed by main on 2026-10-08:** the project owner
chooses who reviews child PRs. worktrees supplies tooling, never a restriction:
a hub may launch a reviewer per child, the owner may review every child, or any
mix. No reviewer identity, model or approval policy is hard-coded by the tool.
A recommended default below is advice the owner may replace.

**Hotfix decision, relayed by main on 2026-10-08:** the non-launch `-N` hotfix
ships now in its own separate lane. It is decided and in progress, not part of
this docs-only PR and not an open nesting decision.

Vocabulary choice, merge strategy, migration details and scope below remain
recommendations.

## 1. Evidence and the gaps that actually exist

Source baseline: `34ad3eec1864596e3b11478f6d5b1f40c5d83806` in this repository.
Symbol/file:line references below refer to that pinned SHA; line numbers in
older proposals no longer describe the current code. The investigation read
`AGENTS.md`,
`DESIGN.md`, [ADR 0001](../adr/0001-no-repo-supplied-argv.html), and the
[project-settings](project-settings.html), [cross-project](cross-project.html)
and [pull-request](pull-requests.html) proposals. Historical proposals describe
some retired implementations: the current engine is Rust core, linked by the
app, not an app subprocess invoking the old bash engine.

| Current evidence | Consequence for this design |
|---|---|
| `Declared` (`core/store.rs:33`), `Store` (:119), `edit` (:309), `reconcile` (:230); unknown keys round-trip, writes lock, display reads are lenient | Parentage belongs here, but authorization must not turn corrupt state into an empty, unrestricted tree. |
| `cmd_new` (`core/ops.rs:611`) already accepts a positional base; `do_switch` and new-branch creation prefer `origin/<base>` when available | “Every lane always starts at origin/main” is only the default, not a limitation. A raw base is already possible; it records no parent or target contract. |
| `default_base` (`core/project.rs:589`) selects main/master or the checkout branch; `base_ref` (:604), `ls` (:281) and `place_json` (:327) use a project-wide base; `ls_json` (:321) serializes `ls` | Per-lane divergence and health need a parent-aware base resolver. Do not hardcode the string `main`. |
| `pr_for` (`core/github.rs:615`) matches head branch + push owner, rejects head == base, prefers open then recent; `Row` retains `base` | PR mapping does **not** require main today. It lacks an expected parent target, so a PR to the wrong branch can look like this lane's completed work. |
| `core/ops.rs`: health construction calls `p.base_ref()` and `git cherry`; `health.rs::maybe_merged` is explicitly a hint | A child can look unmerged against main after landing in its hub. Patch equivalence is not deletion proof. |
| `caller_place` (`cli/mcp.rs:2193`) derives identity from launch directory; local `mutable_target` (:2460) accepts the resolved local slug | Identity exists; subtree authorization does not. `create_worktree` already has `base`, but no `parent`. Removal has its own path and must not miss the new guard. |
| `tmux` (`core/tmux.rs:75`), `attach_or_switch` (:919); app `term_open` attach (`app/src-tauri/src/lib.rs:7728`) | Commands have no explicit project socket today, including the direct PTY attach and CLI attach path. They can use the default or inherited server. |
| `select_slot` (`core/provision.rs:251`), `render_env` (:163), `compose_project_name` (:511) | A hub already qualifies for its own slot and Compose name. Nesting needs no second slot allocator. |
| `core/quota.rs` module contract | The launch gate checks near-spent windows, fails open without data, and is explicitly not a burst/concurrency limiter. |

Here `core/` means `crates/worktrees-core/src/`, and `cli/` means
`crates/worktrees-cli/src/`. This is a source audit and external-documentation
survey, not an end-to-end implementation test. The October 7 socket incident
and its suspected ECONNREFUSED/unlink sequence come from the brief; this work
does not establish its root cause. No existing tmux session was changed.

## 2. Model, identity and creation

### Declared tree, flat directories

Git knows commit ancestry and branch ownership, not which durable lane owns
another. A merge or rebase changes ancestry without changing responsibility.
Keep `.worktrees/<slug>` for every lane; never nest a worktree directory under
a hub's working tree. Existing slugs stay project-unique, so `<project>:<slug>`,
Claude history paths, Compose names, pane identities and saved navigation keep
working. Existing `place_panels`, nav-history and `ui-state` lane keys are
unaffected because slugs stay project-unique. A title is still a label, never a
directory rename.

Proposed additive fields in `.worktrees.places.json`:

```json
{
  "version": 1,
  "places": {
    "experimental": {
      "hub": true,
      "integration_branch": "feature/experimental",
      "parent": "(main)",
      "target": { "remote": "origin", "branch": "main" }
    },
    "experimental-api": {
      "parent": "experimental",
      "target": { "remote": "origin", "branch": "feature/experimental" },
      "fork_oid": "<commit used to create this branch>"
    }
  }
}
```

These names are a proposed schema, not existing fields. `parent` identifies the
owner lane; `target` records the reviewed integration destination for the current
piece of work. Git derives the current branch, tip and divergence. `fork_oid`
records a useful recovery boundary; it is not “the branch's current merge base.”
A hub's `integration_branch` is an intentional binding that survives a temporary
checkout change and lets the tool detect it. No command strings go here.

Absent parent means a legacy top-level lane under `(main)`; `(main)` itself
has no parent. Only explicitly promoted hubs can accept children. Maximum depth
is two edges below main; regular top-level lanes remain leaves. The explicit
hub bit lets an empty hub exist before its first child and after its last.

Core must validate the whole relationship graph under a project relationship
lock: registered same-project parent, existing hub, no self-parent, cycle,
third level, main-as-child or duplicate ownership. All CLI, MCP and app mutations
use that validator. Invalid/unknown parent state stays visible as “parent needs
repair”; it never silently reparents to main. Strict reads are required for
mutation authorization. A tracked sidecar cannot grant agent hub authority;
require local user adoption, following `store::is_tracked`'s provenance rule.

The JSON remains additive (unknown fields already survive older writers), but
that is **data compatibility, not safe mixed-version operation**. Older binaries
can still remove hubs without reading the fields. Enable nesting only after the
app, CLI and long-lived MCP servers are upgraded/restarted; publish a capability
version and diagnose old servers. Do not claim a JSON field can restrain an old
binary or an arbitrary shell command.

### Proposed command contract

```text
# Flat lanes: existing default stays first-class.
worktrees new fix/typos --no-attach
create_worktree { branch: "fix/typos", ... }

# Optional hub and explicitly parented child.
worktrees new feature/experimental --hub --name experimental --no-attach
worktrees new feature/experimental-api --parent experimental --no-attach
create_worktree { branch: "feature/experimental-api", parent: "experimental", ... }
```

Hub creation/promotion is a user or main-orchestrator operation. An omitted
MCP `parent` always defaults to `(main)`, regardless of caller. A hub agent must
pass `parent: "<its slug>"` to create a child; omission is refused by scope with
that remedy, never silently reinterpreted. A leaf caller may not create peers
by omitting it. Main's existing `create_worktree {branch, ...}` remains the
unchanged flat flow. Explicit `parent: "(main)"` means the same thing. The CLI
shows the resolved parent and target; ⌘N shows them before Create. A raw explicit base
remains available for ordinary top-level creation; with `--parent`, a conflicting
base is refused rather than silently overriding the tree.

These extra hub preconditions apply only to children of a hub; ordinary flat
creation keeps today's behavior. Before creating a hub child, require the hub on its bound integration branch, clean
and with no merge/rebase in progress. Fetch its target remote, require the hub
branch to be published and its local tip to equal the remote tip, then capture
that OID for creation. If it is ahead, ask its owner to push; if behind, ask its
owner to update; if diverged, stop. Never push or change another lane as a side
effect of creation. This deliberately makes “base = parent's branch” precise:
children see the parent's published committed work, never its uncommitted files.
For offline/local-only projects, a user may explicitly choose a local target;
the result states that no GitHub PR is possible until published.

Existing-branch adoption must validate the same target contract, show divergence,
and require an explicit adoption action; the current `cmd_new` reuse/open paths
must not bypass parent validation. Do not infer parent from upstream: a new branch
can track its starting base, and a pushed branch normally tracks itself.

Persist parent/target before launching the agent or writing its final brief.
Because Git creation and JSON writing are not one transaction, journal the
operation in `<git-common>/worktrees-operations/<operation-id>.json` (local-only,
never committed or transferred by sync): prepare validated intent → create
worktree → commit relationship → provision/launch. Recover interrupted creation explicitly; an
unassigned child must not acquire main-level defaults. Never launch after a
relationship write fails. Concurrent child creation and hub removal take the
same relationship lock; revalidate the branch OID before creating.

Hub branch switching/renaming is refused while any children remain, including
closed or parked children. An out-of-band switch produces a mismatch warning
and blocks new children/cleanup; it does not rewrite their recorded targets.
An empty hub can be rebound explicitly by its owner after the prior work closes.

## 3. Git flow and prior art

### Survey and choice

The following are primary documentation consulted on 2026-10-08, not tools
installed or exercised by this investigation.

| Tool | Relevant behavior | What to borrow |
|---|---|---|
| [Graphite restack](https://graphite.com/docs/restack-branches) | Tracks dependent branches and rebases them when ancestors change; handles the boundary left when a lower PR is squash-merged | Explicit parent metadata and deliberate restacking. Do not infer the replay boundary from commit subjects. |
| [Git Town branch types](https://www.git-town.com/branch-types) and [sync](https://www.git-town.com/commands/sync.html) | Distinguishes long-lived perennial branches from feature branches; sync follows parent relationships and has separate feature/perennial strategies | A stable integration branch with disposable feature children is the closest model. We choose our own merge policy below rather than importing its defaults. |
| [GitHub's gh-stack extension](https://github.com/github/gh-stack) | Models an ordered PR chain with each PR based on the previous branch; stores shared metadata in the git common dir. Its worktree-aware rebase checks owners and stops on busy/dirty owners | Respect worktree ownership and keep operation recovery explicit. Our siblings feed one hub; they are not one ordered stack. Do not make gh-stack a dependency or run its multi-owner mutations behind agents' backs. |

**Choose a Git Town-like integration branch model, using ordinary git/gh.**
The hub is shared published history: merge main into it; do not routinely rebase
or force-push it. Children are independently owned topics: their owner rebases
onto the updated hub. This bounds a main update to one integration resolution
plus each affected child's resolution, without rewriting every sibling's base.

### One feature from start to finish

1. Main's orchestrator authorizes the hub and brief. Its agent pushes its
   integration branch and opens a draft PR targeting the project's main branch.
2. The hub creates children from its published tip. Each child opens a PR with
   an **explicit** `--base feature/experimental`, naming its parent in the body.
   A child never targets main merely because `gh` defaulted there.
3. Each child follows the project owner's review policy and runs the project's
   gates against the hub target. A suggested default is a Fable reviewer per
   child, launched by the hub; the owner may instead review every child or use
   any mix. Tooling never requires that default, a particular reviewer or model.
   Under the owner's chosen merge authority and approval policy, the hub agent
   may squash-merge, then updates its **own** worktree from the remote before
   integration testing or creating more children. The merge destination must
   match the recorded target immediately before the `gh` operation.
4. As main advances, the hub owner fetches and merges the updated main ref into
   the hub in the hub's worktree, resolves conflicts, runs integration gates,
   and pushes. It reports the new tip to its children. Each child's agent, once
   idle and clean, fetches and rebases in its own place; a rewritten published
   child uses `--force-with-lease`, and review/CI runs again. No orchestrator
   edits a busy child's index. Conflicts stop that child, not the whole tree.
5. After children close out, the hub finishes integration tests and its own
   close-out, updates the draft PR with the complete feature behavior, migration
   notes and links to child PRs. The suggested default is owner review of the
   aggregate diff with main before main squash-merges it; the owner chooses the
   actual reviewers and approval policy here too. Child approvals are evidence,
   not automatic approval of the final composition. Only main releases.

Squash at both boundaries preserves this repository's convention. Merge commits
for child PRs would preserve child SHAs and ease ancestry checks, but would
introduce a second PR policy and noisy integration history. Squash means cleanup
must use target-specific evidence, not raw ancestry. Merging main into the hub
is synchronization, not a change to the squash-PR convention. The final squash
loses the individual child commit lineage on main; PR links and close-out
archives retain the review narrative.

Do not continue new work on a squash-merged child branch: park on a fresh
`<slug>-next` from the fetched hub tip, unset its upstream, then start a new topic.
Otherwise rebasing can replay already-squashed commits. Exceptional recovery
needs the recorded old boundary and an explicit `rebase --onto` plan reviewed by
the owning agent; no automatic “guess which commits landed.”

Never delete the hub branch while `gh pr list --base <hub-branch> --state open`
is non-empty, even if no local child worktree remains. Check in the correct
repository immediately before hub close-out/merge and again before deletion;
a failed query is not an empty result. GitHub documents that deleting a merged
head branch retargets open PRs based on it to the merged PR's base: for a hub,
that is main. `gh pr merge --delete-branch` performs deletion after merging,
so omit it for hubs until this guard passes; account for repository automatic
head deletion too, blocking the hub merge while child PRs remain. This rule
protects remote children as well as local lanes. [GitHub merge documentation](https://docs.github.com/en/pull-requests/how-tos/merge-and-close-pull-requests/merging-a-pull-request).

Concurrent child merges are serialized by the hub's workflow and rechecked against current base
and CI. Git locks alone do not serialize remote PR decisions.

### The existing assumptions to change

| Surface | Required change |
|---|---|
| `github::pr_for` and `view` | Accept expected target as well as head/owner; match its base, and expose a mismatched PR separately. A merged PR into some other branch is not “landed here.” Branch reuse also requires matching the current work's head/OID, not merely a historical PR with the same name. |
| `Project::ls_json` → `ls`/`place_json` (`core/project.rs:321`, :281, :327), `ops` health construction | Resolve parent target once per lane, report its name beside ahead/behind. Do not silently fall back to main when the recorded hub is missing. Keep project totals separate. |
| `Place` / `ls --json` | Carry new `parent`/`hub`/`target` fields through `core/model.rs:20`, `place_json` (`project.rs:327`, currently passed ONE project-wide `base_ref`), `test/json.bats`, MCP `list_places` rows (`cli/mcp.rs:1490`), app `Place` (`app/src/App.tsx:240`) and mock `fixtures.ts:23` / `install.ts`. The app's `snapshot` (`lib.rs:326`) and MCP `place_snapshot` (`mcp.rs:2987`) are separate consumers; there is no `Project::snapshot`. Rebuild and diff `ls --json` against the shipped binary, allowing only intentional additive fields and parent-relative divergence. |
| `health::maybe_merged` | Remains a hint against the lane's integration target. A squash of several commits may match no individual `git cherry` patch. |
| close-out skill and `.claude/close-out.md` | Replace hardcoded `origin/main` in fetch, did-it-land, archive PR and fresh-base steps with the recorded target. Child archive PR → hub; hub archive PR → main. Resolve once, print it and revalidate before writing/merging. |
| `ops::remove_one` / app `remove_place` | `git branch -d` checks merger into the branch's upstream (or HEAD without one), not our parent contract. A pushed child equal to `origin/<child>` passes even after a squash into the hub: `-d` is no safety net for integration. Require PR evidence independently. CLI/app force selects `-D` as well as allowing a dirty removal; never infer deletion permission from either command's success. |
| `<slug>-next` | Still unique per physical worktree, created from that lane's integration target with upstream unset. A parked child still belongs to its hub and blocks final hub retirement until removed or explicitly detached. |

The close-out skill (`~/.claude/skills/close-out/SKILL.md`) currently has seven
`origin/main` occurrences, including its preconditions and fresh-base commands;
`.claude/close-out.md:42–45` also hardcodes it. Update all these sites, not just
the final parking command. Every child work/archive PR must explicitly use
`gh pr create --base <hub-branch>`; a default `gh` base is unsafe here.

For cleanup, fetch the target and check the exact PR's head, repository, base,
merged state and merge commit. Check for commits or uncommitted work made after
that PR head, and verify the merge commit is reachable from the current target.
For uncertain/squashed history use a reviewed content comparison; a matching
subject, a missing remote branch or a green `maybe_merged` count never suffices.
Target advancement/reverts do not grant permission to discard subsequent local
work. Separate “discard dirty files” and “force-delete branch” confirmations in
any future removal API/UI; keep the legacy flag's two effects explicit.

## 4. Hub authority and messaging

### Enforce scope in the server, not just the brief

Compute caller identity from `Server::caller_place`, then load validated current
relationships for **each mutation**. Return the role, parent, expected target and
capability version in initialization/status results. Never accept `caller`,
`scope` or an arbitrary base as a grant supplied by the model.

| Caller | Worktrees mutations allowed with existing mutation opt-in |
|---|---|
| Main | Existing project authority; can create/promote hubs and repair relationships. Existing cross-project consent still applies. |
| Hub | Own ordinary lane state, plus create/open/close/remove and metadata operations for its children. No siblings, main, other hubs or foreign-project mutations, even with cross-project `full`. Cannot promote itself or children, change its parent, or grant wider authority. |
| Child / ordinary leaf | Own non-destructive lane operations and metadata. No peer creation or peer mutation. Ask its owner to close/remove it. |
| Automation or unresolved caller | Existing automation restrictions still subtract powers; unresolved identity gets no mutation powers. |

Today `mutable_target` (`cli/mcp.rs:2460`) accepts any local slug: a child can
`send`, `set_lifecycle` or `close_session` on a sibling. The proposed scope
removes that authority server-side. In contrast, `remove_worktree`
(`mcp.rs:1924`) currently invokes `cmd_rm <slug> -y` with neither `--branch` nor
`--force`: **MCP removal cannot execute `git branch -D` today**, or delete a
branch at all. The two-permissions force hazard in §3 is a CLI/app concern,
not an existing MCP branch-deletion capability.

Scope binds worktrees **mutations only**. Claude `SendMessage` and agent `gh`
commands remain unscoped by this mechanism, by design; briefs and review policy
must teach those boundaries without claiming MCP enforces them.

Use one core authorization decision at all MCP dispatch sites, including
`create_worktree`, `remove_worktree`, `send`, `close_session`, setters,
project/automation-wide writes and indirect “apply” operations. Repository-wide
operations require main; a child must not mutate a sibling through an automation
proposal. Resolve aliases and `<project>:<slug>` before comparing identity.
Existing `foreign_gate`, private-project restrictions, confirmation requirements
and quota gates compose by intersection; `confirm` or launch `force` never
bypasses scope. A read-only server remains read-only. Human app/CLI actions can
choose a target explicitly, with the same structural guards.

Retain readable peer status and messages under existing project/privacy consent.
Reading or reporting across a boundary does not confer mutation permission.
For main's opted-in cross-project creation, resolve the requested parent inside
the **target** project and apply the same graph validation; never create a
cross-project parent edge.

**Limit:** this is an MCP boundary, not OS or GitHub isolation. Today agents
share a Git common directory and may have shell tools and the user's `gh`
credentials. There is no merge/release MCP tool here to constrain. “Hub must not
merge to main, release or touch siblings” therefore also belongs in its launch
guidance/brief and the review procedure. A hard guarantee requires separately
scoped credentials, protected-branch controls and shell/filesystem restrictions;
adding a parent field cannot supply it. Do not advertise that stronger guarantee.

The hub agent can perform authorized child merges with `gh`; the **app remains
read-only toward GitHub**, including no PR creation, approval, comment or merge
button, as [pull-requests §10](pull-requests.html#10-decisions-david-2026-10-07)
requires. No new GitHub write MCP is proposed.

### Two buses, neither transported by tmux

`core/messages.rs` implements `report`/`messages` as files in
`<git-common>/worktrees-messages`, with per-recipient read markers. It has no
tmux transport dependency; `wait until: message` uses that log. Default `report`
should address the immediate parent for a child and main for a hub, with the
resolved recipient stated in results and tool descriptions. Existing explicit
addresses keep their meaning. Do not silently broadcast child reports to main.

Claude↔Claude keeps Claude's own `ListAgents`/`SendMessage`, using the actual
reported agent name. The repository delegates to that bus rather than
implementing it. The cross-project proposal §4.3 recorded successful discovery
across four projects. A read-only filesystem check in this investigation found
about 20 Unix socket files (a changing count) under `/tmp/cc-socks`; that corroborates the separate
socket transport described in the brief. It is not a fresh delivery/recovery
test of Claude internals. Splitting tmux servers should require no bus rewrite;
verify delivery across two servers before release. Keep names globally
unambiguous or return the actual discovered agent identity when prefixes collide.

What **does** need tmux routing: `activity::capture_chain`, Codex mid-turn modal
capture, pi pane/trust-state checks, pane/program liveness, app Claude draft
capture, `place_status`/`wait until: idle` activity derivation, MCP `send`
(send-keys for supported non-Claude harnesses), launch/resume/adoption/close,
all AI and shell sidecars, app terminal attach/copy/scroll/resize handling, and
CLI attach/switch-client. Claude's probe reader stays shared; no second activity
implementation just for hubs.

## 5. A hub's own development environment

A hub is already a normal worktree for `provision.rs`: it gets a unique slot,
`.worktree.env` with `WORKTREE_SLOT` and service ports, and a sanitized
`COMPOSE_PROJECT_NAME`. Main uses the base ports; hub and children allocate from
the same project-wide pool. Do not let a child reuse its parent's slot. Preserve
assigned values until explicit reallocation; a topology change must not point
teardown at a different Compose stack. Removal only tears down that lane's
recorded project, never the subtree implicitly.

That isolates configured ports and ordinary Compose project resources. It does
not isolate hardcoded container names, external/shared volumes, database schemas,
cloud resources, callback URLs, app bundle IDs or user configuration. A hub's
integration stack should have its own data by default, with explicit user
choices for any shared services. Existing file materialization resolves sources
from main; parentage does **not** silently change secret/file inheritance to the
hub. Config differences in a child branch need the usual provisioning checks.

For a desktop app, follow `app/scripts/sandbox.sh --app`'s separation of app
identity, config/data directories and tmux prefix. Its current bundle identifier
is the fixed `net.casadelvalle.worktrees.sbx`, so it demonstrates stable-vs-sandbox
separation, **not arbitrary simultaneous hub app identities**. Multiple hub builds
need distinct IDs/product names, config/data roots, dev-server ports and any
single-instance/updater identity. A worktrees development app must also isolate
its test project registry and tmux server key; a new prefix alone will no longer
be sufficient when socket routing is project-based. Do not register the real
workspace by default in a sandbox app.

Keep every executable “run this” setting in the user's own config/profile or an
explicit user command. `.worktrees.toml` remains declarative `[ports]`, `[compose]`
and `[[file]]`; no `[infra] up`, hooks, hub command strings or repo-selected
executables. A user may opt into a repo-provided development script through
user-only configuration. This proposal does not invent a supported runner key or
claim the ADR's example hook is a shipped start-stack API.

## 6. Navigation and the word “lane”

Use **lane** everywhere a person chooses, opens or removes a place. “Hub” is a
role for a lane, not a competing noun or a required name for its agent. Keep
`place` in Rust types, JSON, persisted keys, resource URIs and existing MCP tool
names. One coordinated copy pass changes UI strings (including empty/error
states and accessibility labels), MCP descriptions, CLI help, current README/
agent guidance, AGENTS.md and the worktrees skill. Existing API names and CLI
verbs remain compatible; historical proposals/archives stay historical with a
short glossary link. New tools should explain once: “lane (API: place).”

```text
Flat project (unchanged)          Mixed project (hubs opt-in)
v worktrees                      v worktrees
    (main)                           (main)
    Active                           Active
      API                              API
      Navigation                       v Experimental  hub
                                         API experiment
```

The existing flat ordering, lifecycle groups and creation shortcuts remain
unchanged in a project with no hubs. The mixed tree adds one expandable row
where a hub was explicitly requested:

```text
v worktrees
    (main)                         o
    Active
    v Experimental        hub      ●  3 children
        Active
          API                      ●
          Navigation               ○
        Saved
          Migration                ◉
      Fix typos                    o

> Experimental            hub      ●  3 children · 1 unread

header: worktrees / Experimental / API
        feature/experimental-api → feature/experimental     [PR #…]
```

The ASCII glyphs stand for existing activity/unread shapes, not new colored
text. A hub appears once in its own root-level tier; its children appear only
beneath it, with lifecycle grouping and ordering local to that sibling set.
Saved, Closed, Archived and Abandoned tiers nest under each hub too; they do
not collect children into global tiers. Root-level flat lanes keep global tiers.
Pinning a child pins it **within the hub**, never extracts a duplicate top-level
row. The hub's own lifecycle stays its own reading: a closed hub with busy
children says so through roll-up, not by falsifying `store::reconcile`.

Collapsed roll-up is computed from the same per-lane readings: waiting takes
precedence over busy, followed by unread completion and existing quiet state.
Use the existing dot/ring vocabulary with a neutral-text count/tooltip that
separates “hub agent” from “2 children busy.” Count unread descendants without
acknowledging them: opening the hub, expanding it or seeing its row must not mark
children read. Entering a child keeps today's `unseenWork` acknowledgment rule.
Unknown activity remains visible and is never reported as idle.

PR attention rolls up in the **existing Pull requests rail badge** when a hub
is selected, with hub/subtree counts explained in its tooltip and dock summary.
The collapsed hub's tooltip also summarizes its subtree's PR attention (for
example, “2 child PRs need attention”), even when another lane is selected; use
the existing cached project data and show stale/unknown rather than polling a
hidden project. The dock still shows the whole project, the selected lane's PR
pinned first; add a subtree filter if needed, not a different default list. Do not add another
colored nav dot. A collapsed hub's optional attention marker can only use the
previously deferred distinct-shape, opt-in policy; it is not a prerequisite.

- Collapse state and sibling order belong to frontend-owned `ui-state.json`,
  keyed by project + parent slug. A hub deletion drops obsolete UI keys; backend
  writes must not touch that whole-blob file.
- ⌘N from a hub means “New lane in Experimental.” From a child, default to its
  hub (a sibling); from main or a top-level leaf, default to main. The dialog
  displays parent and target branch. Offer “New hub” only at project level.
- Search includes ancestors as context and temporarily reveals matches without
  overwriting saved collapse state. Keyboard navigation uses tree semantics;
  chevron toggles expansion, row activation selects the lane.
- History keeps stable project/slug locations, not tree positions. Back/forward
  expands ancestors to reveal a child. Collapse is not a navigation event and
  does not detach its selected terminal. Gone lanes follow existing gone-target
  behavior; no surprise fallback that operates on main.
- Drag reorders **siblings** or changes a lifecycle tier within their parent.
  Dragging a hub moves its subtree as a unit. Cross-parent/project reparenting
  is refused in the first version; it changes a Git/PR contract, not just layout.
  Cross-project text-reference drops into agent panes keep existing semantics.
  A later explicit reparent flow must review target branch and open PR changes.
- `dnd.ts::predictTier` still mirrors `store::reconcile` for the individual row.
  Add parent-aware drop validation in core and its preview, and extend the
  existing `dnd-check.mjs` drift check (which parses `store.rs` constants) to catch both overly broad and overly narrow acceptance.

Use existing React/CSS primitives and tokens, no UI library. Words use
`--txt-hi`/`--txt-dim`; hue belongs to shapes/tints. Module-scope row components,
`useEscape` for overlays, async Tauri commands, and mocks for every added command
remain requirements. Check indentation, short windows and narrow nav widths by
measurement/hit-testing in both the mock and the real isolated app; restart the
harness after edits under `.worktrees/`. Roll-up must also work with slow and
out-of-order workspace refreshes.

## 7. Lifecycle and close-out order

**Refuse hub removal while any child record/worktree remains.** This includes
closed, archived, abandoned, idle-base and orphaned children, not just live tmux
sessions. The refusal lists blockers and their state. No `--force` cascade in
phase one. Closing the hub's own session is allowed and leaves children running;
label it “Close hub session,” not “Close tree.” Archiving/abandoning a hub with
unfinished children is refused rather than hiding them.

A child commits its session archive/remaining work into a PR targeting the hub;
review and merge it, prove it landed there, then close and remove or explicitly
park it. Parking is useful during the feature, but before final hub retirement
remove that parked child or explicitly release it to main via a reviewed empty
lane transfer. Do not automatically rebase unfinished children onto main when
the hub lands. They block final close-out or require an explicit changed plan.

Hub close-out also queries `gh pr list --base <hub-branch> --state open`;
any open PR blocks hub branch deletion and final merge when automatic head
deletion is enabled. No local children does not prove no remote child PRs.

The hub owns the checklist: every child and open child PR accounted for → full feature tests →
hub archive/roadmap reconciliation → aggregate review by main → hub PR merged
into main → target-specific verification → clean up the hub. Archive paths keep
project-unique slugs, so child histories do not collide. The hub summarizes and
links children rather than duplicating every transcript. Hub work is never a
release authority; tags, publishing and the release ritual stay in `(main)`.

## 8. Tmux: independent hotfix, routing prerequisite, later recovery

### Independent hotfix — decided, in progress in a separate lane

David decided to ship independently now: add global `-N` to **every non-launch
tmux invocation**
(list/list-panes, capture, send-keys, kill-session, tune, attach and the app's
PTY client), including chained commands. Do not add it to intentional
`new-session`/server creation. This can ship on the existing socket topology.

In tmux 3.2, `client_connect` reaches socket unlink/startup only through the
`CLIENT_STARTSERVER` path; `-N` sets `CLIENT_NOSTARTSERVER`, making it return
before that path. Thus protected non-launch clients cannot unlink/recreate a
live server's socket after ECONNREFUSED. This does not protect intentional
launches or establish which client caused the October 7 incident.
[tmux 3.2 client.c](https://github.com/tmux/tmux/blob/3.2/client.c#L94-L153),
[tmux.c](https://github.com/tmux/tmux/blob/3.2/tmux.c#L372-L374).

`-N` arrived in [tmux 3.2](https://github.com/tmux/tmux/blob/3.2/CHANGES), while
`README.md:132` recommends ≥1.9. Detect support once per executable/version,
without starting a server; never blindly pass an unsupported option. On older
versions retain explicitly non-starting commands, report the protection gap
and recommend upgrading; do not claim the same guarantee. Shipping this guarded
hotfix is decided and proceeding separately from nesting (§11, decision 8).

### Phase 1a — routing and identity

Introduce a core-owned `TmuxServer` descriptor, required by every session-scoped
operation. Its normal endpoint is `tmux -L wt-<name>-<hash> …`: a readable
frozen local name plus a project-identity hash. Derive the key from the canonical git common-directory identity (fixed stable hash algorithm,
not Rust's randomized hasher), with a locally recorded identity check to detect
collisions. Do not use the repo-controlled prefix, mutable registry display name
or hub slug as identity; the readable name is frozen when the endpoint is
assigned. Linked worktrees share the key; independent clones do not. Moving a repo
requires explicit adoption/rebinding, not silently starting another server.
Normalize the socket root across GUI/CLI environments; inherited `TMUX_TMPDIR`
or `TMUX` must not redirect an otherwise identical key.

A user-only sandbox namespace may intentionally isolate a development app's test
projects. Record it in the descriptor; it must never leak from repo config or
silently partition the real project. One server per project is the steady-state
rule; the legacy migration below is the bounded exception, not one server per hub.

Thread the descriptor through all of `tmux.rs`, `activity.rs`, project snapshots,
harness launch, CLI attach/switch-client and MCP resolved targets. Update direct
app calls too: global pane lists/fingerprints, Claude draft chains, `codex_tick`,
`pi_tick`, `term_open`'s `CommandBuilder`, wheel/copy-mode helpers, shell sidecars
and cleanup. Group capture chains by server. A pane ID such as `%3` or a session
name alone is no longer globally unique: caches, terminal handles, fingerprints,
confirmation bindings and activity maps must key `(server, session/pane)`.
Global session existence must not select the first same-named session it finds.

Cross-project `place_status`, `send`, `wait` and app references use the **target's**
descriptor; reports continue to use its common-dir log. CLI `switch-client` only
works within one server. When already attached to a different server, return the
explicit attach command/new-terminal route; do not switch the wrong client or
nest attaches accidentally. Version detection (`tmux -V`) is the only ordinary
call with no session endpoint. Add an audit guard that rejects new raw tmux
subprocess sites outside the descriptor/PTY adapter and version probe.

### Phase 1b — lease and recovery guard (not a nesting dependency)

Record a local, non-synced lease under the git common dir: endpoint, server PID,
UID, process start identity/boot identity and executable identity. Take a
cross-process project startup lock around inspect/recover/start. Record the real
server PID and socket path reported by tmux, never the transient client's PID.

1. Probe the recorded endpoint without permitting server startup (`-N` on
   supported tmux versions). Successful connection must identify the expected
   server. A different live owner is a conflict, never something to kill.
2. If connection fails and the lease still identifies a live owned tmux process,
   **do not run new-session/start-server**. On a missing socket, restore only
   the owned socket directory with correct permissions if needed, send that
   verified process `SIGUSR1`, then retry for a bounded period. Never signal a
   PID based only on its number or unlink an endpoint held by an unknown process.
3. ECONNREFUSED or timeout is not proof of death. A surviving socket with unclear
   ownership fails closed with a diagnostic; do not blindly unlink or signal
   both old and new servers. If recovery fails, show “server unreachable,” keep
   lane activity unknown and suppress relaunch. A probe error must not become
   the current `false`/empty-list “session absent” answer.
4. Only confirmed process death/no known server permits startup under the lock.
   After startup record identity before releasing the lock. If the creator dies
   before recording it, the next caller discovers a reachable endpoint first;
   ambiguous leftovers are reported rather than replaced.

The [tmux manual](https://man.openbsd.org/tmux) documents named independent
servers, `-N`, and `SIGUSR1` socket recreation; missing parent directories prevent
recreation. This is a recovery mechanism, not proof of the incident's suspected
cause. Feature-detect/support the minimum shipped tmux versions explicitly; if
`-N` is unavailable, use only non-starting probes and fail closed on ambiguity.
macOS and Linux process-identity verification need platform-specific witnesses.

### Phase 1a — restart-based drain, without an adoption UI

Tmux cannot transparently move running panes between independent servers.
Existing live lanes stay on their legacy server; new launches use the project
server. A lane moves when its owner closes and reopens it. No forced mass
restart, capture-pane “restore,” special adoption screen or migration wizard.

During the drain, the app lists **both** the legacy default server and all
project servers, including provider and `~term` sidecars. Enumerate each endpoint
once and associate sessions by existing name/cwd rules; retain endpoint identity
on every resolved session handle and control call. Existing legacy sessions win
for their lane until closed; do not launch a duplicate on the project server.
If both endpoints contain a candidate, report ambiguity and refuse mutation
rather than adding an adoption UI or choosing arbitrarily. Stop querying the
legacy endpoint after it is confirmed empty; never kill that shared server or
other projects' or unmanaged sessions to hasten the drain.

Phase 1a does not promise automatic recovery of a missing legacy socket. An
unreachable known endpoint blocks automatic relaunch and requests manual
recovery; an empty successful list is different from a failed probe. Phase 1b
adds the verified lease/SIGUSR1 recovery described above. Restart old MCP
processes during the upgrade: they otherwise keep the old socket implementation
and lack the hotfix despite a new CLI on disk. Rollback must retain endpoints
and block unsupported mutation, not silently fall back to default.

**Only phase 1a is a nesting prerequisite.** It reduces cross-project blast
radius; phase 1b is independently useful hardening and must not delay the tree.
One server still fails a project's whole tree. Future socket-loss tests use
unique explicit `-L` on every call, on macOS and Linux; never experiment on the
user's servers.

## 9. Load, quota and acceptance evidence

A hub plus N children means N+1 agents and their MCP processes; provider sidecars
can add more. Main remains another agent. Do not start a second MCP per hub for
routing: each existing server already knows its caller and can resolve the tree.
The quota gate remains at harness preparation, preserving the worktree and brief
when launch is refused. A hub does not receive a new allowance; its children
consume the same provider/account budget.

Before broad fan-out, add a user-only concurrency cap with a cross-process launch
reservation, expiry and reconciliation against actual live activity. Busy count
alone races simultaneous launches; a process-local mutex or cached usage value
cannot reserve across MCP servers. Count across hubs/projects sharing an account;
where account identity is unavailable, use a conservative provider-wide bucket
and explain it. Expose waiting/refused launches and costs as estimates, not a
promise about tokens. Recommend two concurrent children per hub as initial
workflow guidance, not a hidden hardcoded spending policy. Quota overrides
remain explicit and never override scope.

Do not poll a separate full workspace for each hub. Build adjacency/roll-ups
once from the existing snapshot, O(lanes). AGENTS.md records a 0.28s git sweep
for one project with nine worktrees; that workload grows with lanes, not tree
depth. Per-project servers also replace one global session/pane enumeration with
P `list-sessions`/`list-panes` batches per tick for P project servers, plus the
legacy endpoint while draining. Read each once, batch captures per server, and
cache provider logs as today; isolation is not a claim of lower polling cost.
MCP resource lists stay cheap; full activity is on demand. Many `wait` clients can still multiply
polling: measure one project with 1, 5 and 20 agents, watcher count, process
count, git/tmux spawns and visible/hidden app CPU before adding a shared daemon.
Use CLI counting wrappers, not app PATH injection (AGENTS.md explains why that
undercounts). Keep GitHub polling selected-project-only; hubs add no per-hub
requests and no new background GitHub writer.

The Bats shim must change in the same PR as the first global tmux flag.
`test/helpers/common.bash:212–306` sets `sub` from `$1` at :224 before its
option loop, and checks the `copy-mode -q -t X ;` prefix at :216 before either.
Leading `-L`/`-N` therefore misparse the current shim. There are 47 `TMUX_LOG`
references in `test/*.bats`, including literal argv assertions. Parse global
options first, log endpoint/options separately from normalized subcommand argv,
and preserve the copy-mode chain behavior. Keep explicit endpoint assertions so
normalizing the log cannot hide a missing `-L`/`-N`. `real-tmux.bats` already uses
an explicit `-S` socket; preserve that isolation.

Required implementation witnesses, with new regression tests shown red first:

| Area | Must demonstrate |
|---|---|
| Flat compatibility | No hubs: unchanged nav grouping, ⌘N and omitted-parent MCP creation off main; mixed project: flat creation still available without promoting a hub. |
| Relationship storage | Legacy empty parent, unknown-key preservation, invalid/cyclic/deep graph, tracked store, missing parent, branch mismatch, crash between Git/store writes, concurrent create/remove. |
| Scope | Hub can act on child; same request to sibling/main/foreign project is denied, with aliases too. Leaf cannot create peers. Corrupt/missing identity cannot expand authority; generic apply/removal paths cannot bypass checks. |
| Git | Two sibling PRs to a published hub, squash both, merge new main into hub, owner-only child rebase, child archive to hub, hub archive/aggregate PR to main. Wrong-base merged PR and post-merge child commits must block cleanup; an open remote child PR blocks hub branch deletion even with no child worktree. |
| Removal | Closed/orphaned/parked child blocks hub removal, force does not cascade, branch-deletion consent distinct from dirty-tree consent. |
| Bats | Shim parses leading `-L`/`-N` before subcommand and copy-mode detection, records flags separately, preserves literal argv assertions and chain execution; a missing required flag fails its own assertion. Real `-S` tests stay isolated. |
| Hotfix | On supported tmux, every non-launch call carries `-N`, including command chains and PTY attach; intentional launch omits it. Unsupported versions produce a clear compatibility result rather than broken commands. |
| tmux | Every CLI/app/MCP route selects the target server; repeated `%1`/session names on two servers stay distinct. Missing-socket recovery preserves running pane PIDs; dead/reused/unknown PID cannot be signalled; concurrent launches cannot start duplicates. Phase 1a restart-based drain preserves sidecars and other projects without an adoption UI; phase 1b separately proves recovery. |
| Real app | Collapsed hub shows busy/waiting/unread children without clearing unread; hub/child terminals attach to intended endpoint; nav history, short-window hit tests and slow refresh races work. Claude, Codex and pi each observed on screen. |
| Environment | Stable app and two hub dev builds coexist with separate data, ports and identities. Compose teardown of one lane leaves the others intact. |
| Quota/load | Parallel launches across distinct MCP processes honor reservations; a near-spent window refuses launch but preserves brief; hidden app polling and collapsed-tree roll-up do not add full snapshots. |

## 10. Phasing

- **Independent hotfix — decided, in progress in its own lane:** feature-detected
  `-N` on non-launch calls, with the Bats shim updated. No dependency on project routing or nesting.
- **Phase 1a — per-project routing, useful alone:** descriptor and sandbox
  namespace, `-L` on every core/app/MCP call, endpoint-qualified identities,
  restart-based drain listing legacy and project servers, and real-app witnesses.
  No adoption UI. This is the **only tmux prerequisite for phase 2**.
- **Phase 1b — recovery, separate:** local lease, startup lock, no-start probes,
  verified PID/socket identity and SIGUSR1 recovery. Can ship after phase 2;
  phase 1a's unreachable state must not imply it already recovers sockets.
- **Phase 2 — a complete shallow-tree vertical slice, depends on 1a only.**
   Declared relationships, bound targets, core invariants, scoped MCP, parent-aware new/health/PR interpretation,
   guarded removal, lane terminology and the minimal tree UI ship together.
   Manually operated git/gh flow is sufficient; demonstrate one hub and two
   children through both close-outs before enabling general use.
- **Phase 3 — comfort and larger fan-out.** Refined roll-ups/search/DnD, concurrency
   reservations and measured polling budgets, user-only dev-environment presets,
   parent-aware skill guidance and explicit repair/rebind tooling. The essential
   target-aware close-out instructions ship in phase 2, not after users need them.

Do not build arbitrary depth, automatic cascading deletion, a new message bus,
a stacked-PR dependency manager, or GitHub write controls in the app. Those are
separate decisions, not latent powers of `parent`.

## 11. Open decisions and recorded decisions from David

1. **Vocabulary:** adopt “lane,” with optional “hub” role? Recommended yes;
   API `place` stays compatible. Depth and first-class flat lanes are already
   decided above and do not need reconfirmation.
2. **Git policy:** squash child and hub PRs, merge main into the published hub,
   and let each child owner rebase? Recommended yes; no routine hub force-push.
3. **Scope strength:** accept server-enforced MCP subtree scope plus explicit
   agent/`gh` policy, or require separate credentials/sandboxing before calling
   hubs autonomous? The former is the proposed first release; it is not a hard
   shell/GitHub security boundary.
4. **Lifecycle:** refuse hub removal/archive with any children and omit cascade?
   Recommended yes; close only the hub session independently.
5. **Rollout:** ship phase 1a routing/sandbox namespace/restart-based drain
   before nesting, with phase 1b lease/SIGUSR1 recovery independent? Recommended
   yes; upgrade/restart long-lived MCPs, no adoption UI or forced mass restart.
6. **Fan-out:** start with guidance of two concurrently working children per hub,
   then ship configurable account/provider-wide launch reservations before larger
   fan-out? Choose a preferred concurrency budget; quota percentage alone cannot
   prevent a burst.
7. **Review policy — decided, 2026-10-08:** the project owner decides who
   reviews child PRs. worktrees provides tooling, never a restriction: reviewer
   agents per child, owner review of every child, or any mix. A Fable reviewer
   per child and owner review of the aggregate is only a suggested default;
   nothing in the design requires it or hard-codes a reviewer/model.
8. **Immediate tmux hotfix — decided, in progress, 2026-10-08:** the
   feature-detected non-launch `-N` hotfix ships now in its own separate lane,
   independently of 1a/1b/nesting, with older-tmux compatibility and Bats shim
   changes. No implementation is included here. Intentional launches still
   need the later recovery guard for a missing live-server socket.
