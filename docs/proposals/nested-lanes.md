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
per-project tmux routing and socket recovery **before** nesting. Keep GitHub
writes in the agent's existing `gh` workflow, never in the app.

## Decisions already taken

David's brief fixes close-out at both levels, one user-facing word for place/lane,
one tmux server per project, and a read-only GitHub app.

**Depth decision, relayed by main on 2026-10-08:** main → hub → children is the
cap, and main → children remains a first-class option. A hub is opt-in, never
required. Both shapes coexist in navigation and creation. MCP `create_worktree`
defaults to a child of main unless a parent/hub is explicitly given. No migration
turns existing flat lanes into hubs or adds a hub step to their workflow.

Vocabulary choice, merge strategy, migration details and scope below remain
recommendations.

## 1. Evidence and the gaps that actually exist

Source baseline: `34ad3eec1864596e3b11478f6d5b1f40c5d83806` in this repository.
Symbol names below are the durable references; line numbers in older proposals
no longer describe the current code. The investigation read `AGENTS.md`,
`DESIGN.md`, [ADR 0001](../adr/0001-no-repo-supplied-argv.html), and the
[project-settings](project-settings.html), [cross-project](cross-project.html)
and [pull-request](pull-requests.html) proposals. Historical proposals describe
some retired implementations: the current engine is Rust core, linked by the
app, not an app subprocess invoking the old bash engine.

| Current evidence | Consequence for this design |
|---|---|
| `core/store.rs`: `Declared`, `Store`, `edit`, `reconcile`; unknown keys round-trip, writes lock, display reads are lenient | Parentage belongs here, but authorization must not turn corrupt state into an empty, unrestricted tree. |
| `core/ops.rs`: `cmd_new` already accepts a positional base; `do_switch` and new-branch creation prefer `origin/<base>` when available | “Every lane always starts at origin/main” is only the default, not a limitation. A raw base is already possible; it records no parent or target contract. |
| `core/project.rs`: `default_base` selects main/master or the checkout branch; `base_ref`, `snapshot` and `place_json` use a project-wide base | Per-lane divergence and health need a parent-aware base resolver. Do not hardcode the string `main`. |
| `core/github.rs`: `pr_for` matches head branch + push owner, rejects head == base, prefers open then recent; `Row` retains `base` | PR mapping does **not** require main today. It lacks an expected parent target, so a PR to the wrong branch can look like this lane's completed work. |
| `core/ops.rs`: health construction calls `p.base_ref()` and `git cherry`; `health.rs::maybe_merged` is explicitly a hint | A child can look unmerged against main after landing in its hub. Patch equivalence is not deletion proof. |
| `cli/mcp.rs`: `caller_place` derives identity from launch directory; local `mutable_target` accepts the resolved local slug | Identity exists; subtree authorization does not. `create_worktree` already has `base`, but no `parent`. Removal has its own path and must not miss the new guard. |
| `core/tmux.rs::tmux`, `attach_or_switch`; `app/src-tauri/src/lib.rs::term_open` | Commands have no explicit project socket today, including the direct PTY attach and CLI attach path. They can use the default or inherited server. |
| `core/provision.rs`: `select_slot`, `render_env`, `compose_project_name` | A hub already qualifies for its own slot and Compose name. Nesting needs no second slot allocator. |
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
working. A title is still a label, never a directory rename.

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
operation in the common dir: prepare validated intent → create worktree → commit
relationship → provision/launch. Recover interrupted creation explicitly; an
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
3. Each child is reviewed and runs the project's gates against the hub target.
   The hub agent may squash-merge that child PR after review, then updates its
   **own** worktree from the remote before integration testing or creating more
   children. The merge destination must match the recorded target immediately
   before the `gh` operation.
4. As main advances, the hub owner fetches and merges the updated main ref into
   the hub in the hub's worktree, resolves conflicts, runs integration gates,
   and pushes. It reports the new tip to its children. Each child's agent, once
   idle and clean, fetches and rebases in its own place; a rewritten published
   child uses `--force-with-lease`, and review/CI runs again. No orchestrator
   edits a busy child's index. Conflicts stop that child, not the whole tree.
5. After children close out, the hub finishes integration tests and its own
   close-out, updates the draft PR with the complete feature behavior, migration
   notes and links to child PRs. Main reviews the entire aggregate diff and
   squash-merges it. Child approvals are evidence, not approval of the final
   composition. Only main releases.

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
the owning agent; no automatic “guess which commits landed.” Concurrent child
merges are serialized by the hub's workflow and rechecked against current base
and CI. Git locks alone do not serialize remote PR decisions.

### The existing assumptions to change

| Surface | Required change |
|---|---|
| `github::pr_for` and `view` | Accept expected target as well as head/owner; match its base, and expose a mismatched PR separately. A merged PR into some other branch is not “landed here.” Branch reuse also requires matching the current work's head/OID, not merely a historical PR with the same name. |
| `Project::snapshot`, `place_json`, `ops` health construction | Resolve parent target once per lane, report its name beside ahead/behind. Do not silently fall back to main when the recorded hub is missing. Keep project totals separate. |
| `health::maybe_merged` | Remains a hint against the lane's integration target. A squash of several commits may match no individual `git cherry` patch. |
| close-out skill and `.claude/close-out.md` | Replace hardcoded `origin/main` in fetch, did-it-land, archive PR and fresh-base steps with the recorded target. Child archive PR → hub; hub archive PR → main. Resolve once, print it and revalidate before writing/merging. |
| `ops::remove_one` / app `remove_place` | `git branch -d` checks Git's upstream/HEAD rules, not our parent contract. Force selects `-D` as well as allowing a dirty removal. Never auto-escalate because a squash makes `-d` fail. |
| `<slug>-next` | Still unique per physical worktree, created from that lane's integration target with upstream unset. A parked child still belongs to its hub and blocks final hub retirement until removed or explicitly detached. |

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
23 Unix socket files under `/tmp/cc-socks`; that corroborates the separate
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
The dock still shows the whole project, the selected lane's PR pinned first;
add a subtree filter if needed, not a different default list. Do not add another
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
  existing drift check to catch both overly broad and overly narrow acceptance.

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

The hub owns the checklist: every child accounted for → full feature tests →
hub archive/roadmap reconciliation → aggregate review by main → hub PR merged
into main → target-specific verification → clean up the hub. Archive paths keep
project-unique slugs, so child histories do not collide. The hub summarizes and
links children rather than duplicating every transcript. Hub work is never a
release authority; tags, publishing and the release ritual stay in `(main)`.

## 8. One tmux server per project — prerequisite

### Routing and identity

Introduce a core-owned `TmuxServer` descriptor, required by every session-scoped
operation. Its normal endpoint is `tmux -L wt-<project-key> …`. Derive the key
from the canonical git common-directory identity (fixed stable hash algorithm,
not Rust's randomized hasher), with a locally recorded identity check to detect
collisions. Do not use the repo-controlled prefix, registry display name or hub
slug. Linked worktrees share the key; independent clones do not. Moving a repo
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

### Guard against duplicate servers after socket loss

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

### Live migration: drain, do not “move” processes

Tmux does not provide a transparent transfer of running panes between independent
servers. Do not kill/relaunch 25 agents as an update side effect, and do not
pretend capture-pane restores process state.

On upgrade, perform a read-only inventory of legacy endpoints, matching canonical
session identities and pane cwd to registered lanes, including provider and
`~term` sidecars. Save the concrete server/session assignment for adopted live
lanes. Ambiguous sessions require user resolution; no default-server catch-all
adoption on every poll. New launches use the project server, while existing
lanes keep their legacy endpoint until their owner closes/restarts them.
Snapshots query each distinct legacy endpoint once and partition its readings
by project; control operations use that saved assignment. The UI shows the
remaining legacy count and a planned drain/restart action. Never kill the shared
legacy server while another project's or unmanaged sessions remain.

If the legacy socket is already missing, require verified process ownership
before recovery; if it cannot be established, report the blocker and suppress
duplicate launch. Remove the transition assignment only after the old session
is confirmed gone, then create its replacement on the project server. Keep the
session name where unambiguous so Claude naming does not change unnecessarily.
Restart old MCP processes during the coordinated upgrade; they otherwise retain
the default-socket implementation despite a new CLI on disk. Rollback must retain
new endpoints and block unsupported mutation, not silently fall back to default.

**Ship this first.** It reduces the blast radius of today's failure without any
new lane UI. One server still fails an entire project's tree, and does not solve
quota or filesystem contention. No socket-loss experiments against the user's
servers: future tests use unique explicit `-L` on **every** call, macOS and Linux,
and prove one project's recovery leaves another project's panes alive.

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
once from the existing snapshot, O(lanes); read each server once per app tick,
batch captures per server, and cache provider logs as today. MCP resource lists
stay cheap; full activity is on demand. Many `wait` clients can still multiply
polling: measure one project with 1, 5 and 20 agents, watcher count, process
count, git/tmux spawns and visible/hidden app CPU before adding a shared daemon.
Use CLI counting wrappers, not app PATH injection (AGENTS.md explains why that
undercounts). Keep GitHub polling selected-project-only; hubs add no per-hub
requests and no new background GitHub writer.

Required implementation witnesses, with new regression tests shown red first:

| Area | Must demonstrate |
|---|---|
| Flat compatibility | No hubs: unchanged nav grouping, ⌘N and omitted-parent MCP creation off main; mixed project: flat creation still available without promoting a hub. |
| Relationship storage | Legacy empty parent, unknown-key preservation, invalid/cyclic/deep graph, tracked store, missing parent, branch mismatch, crash between Git/store writes, concurrent create/remove. |
| Scope | Hub can act on child; same request to sibling/main/foreign project is denied, with aliases too. Leaf cannot create peers. Corrupt/missing identity cannot expand authority; generic apply/removal paths cannot bypass checks. |
| Git | Two sibling PRs to a published hub, squash both, merge new main into hub, owner-only child rebase, child archive to hub, hub archive/aggregate PR to main. Wrong-base merged PR and post-merge child commits must block cleanup. |
| Removal | Closed/orphaned/parked child blocks hub removal, force does not cascade, branch-deletion consent distinct from dirty-tree consent. |
| tmux | Every CLI/app/MCP route selects the target server; repeated `%1`/session names on two servers stay distinct. Missing-socket recovery preserves running pane PIDs; dead/reused/unknown PID cannot be signalled; concurrent launches cannot start duplicates. Legacy drain preserves sidecars and other projects. |
| Real app | Collapsed hub shows busy/waiting/unread children without clearing unread; hub/child terminals attach to intended endpoint; nav history, short-window hit tests and slow refresh races work. Claude, Codex and pi each observed on screen. |
| Environment | Stable app and two hub dev builds coexist with separate data, ports and identities. Compose teardown of one lane leaves the others intact. |
| Quota/load | Parallel launches across distinct MCP processes honor reservations; a near-spent window refuses launch but preserves brief; hidden app polling and collapsed-tree roll-up do not add full snapshots. |

## 10. Phasing

1. **Per-project tmux servers and recovery, useful alone.** Descriptor, complete
   core/app routing, validated lease, unknown/unreachable state, legacy drain,
   cross-project target routing and real-app witnesses. No nesting yet.
2. **A complete shallow-tree vertical slice.** Declared relationships, bound
   targets, core invariants, scoped MCP, parent-aware new/health/PR interpretation,
   guarded removal, lane terminology and the minimal tree UI ship together.
   Manually operated git/gh flow is sufficient; demonstrate one hub and two
   children through both close-outs before enabling general use.
3. **Comfort and larger fan-out.** Refined roll-ups/search/DnD, concurrency
   reservations and measured polling budgets, user-only dev-environment presets,
   parent-aware skill guidance and explicit repair/rebind tooling. The essential
   target-aware close-out instructions ship in phase 2, not after users need them.

Do not build arbitrary depth, automatic cascading deletion, a new message bus,
a stacked-PR dependency manager, or GitHub write controls in the app. Those are
separate decisions, not latent powers of `parent`.

## 11. Decisions requested from David

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
5. **Rollout:** ship tmux isolation/recovery first, drain legacy sessions on
   owner-approved restarts, and upgrade/restart long-lived MCPs before nesting?
   Recommended yes; no forced mass restart.
6. **Fan-out:** start with guidance of two concurrently working children per hub,
   then ship configurable account/provider-wide launch reservations before larger
   fan-out? Choose a preferred concurrency budget; quota percentage alone cannot
   prevent a burst.
