---
title: "Proposal: lanes started by a lane show under it"
---

# Proposal: lanes started by a lane show under it

**Status:** design, 2026-11-24. Nothing built. Owner choices are in §9.

**The answer.** Record which place started each place, once, at creation, in
the declared store (`started_by`). Draw the nav as a shallow tree from that
record: inside each tier, a starter's lanes sit indented under it, behind a
disclosure with a count and a rolled-up dot. Tiers, sorting, drag and the
lifecycle rules stay exactly as they are. An owner override ("Move under…",
"Move to top level") comes in phase 2. Owner-made named groups are not built.

This is the small half of [nested lanes](nested-lanes.html). That proposal's
`parent` means ownership: a hub branch, a merge target, scoped authority. This
one's `started_by` means origin only. It grants nothing and changes no git
flow. §8 says how the two fit.

## 1. What is recorded today

Measured on this machine on 2026-11-24, in the project `worktrees`.
`design-workspace-chats` started six lanes through MCP `create_worktree`.

| Place | `launched_by` in the store | Upstream | Names its starter in its brief |
|---|---|---|---|
| design-workspace-chats (the starter) | owner, console | origin/design/workspace-chats | n/a |
| design-workspace-view-codex | none | origin/design/workspace-chats | yes, in prose |
| design-workspace-view-kimi | none | origin/design/workspace-chats | yes, in prose |
| design-workspace-view-fable | none | origin/design/workspace-chats | yes, in prose |
| design-workspace-view-memdoor | none | none | no |
| design-workspace-view-combined | none | none | yes, in prose |
| feat-documents-in-main | none | origin/design/workspace-chats | yes, in prose |

Where a starter could be read from today, and why none of it is enough:

| Source | What it holds | Verdict |
|---|---|---|
| `launched_by` (`store.rs`, `ops::stamp_launch`) | Principal and device of the request that created the SESSION. Rewritten or cleared on every launch. | Names a device, never a place. A CLI relaunch clears it. All six children have none. |
| Message log (`worktrees-messages` in the git common dir) | `from`, `to`, `text`, `created`, `id` | 26 messages in total. One names the pair (feat-documents-in-main to design-workspace-chats). Claude lanes talk over Claude's own bus, so most pairs never appear. |
| The brief (`.planning/brief.md`) | Free text the starter wrote | Five of six mention the starter, in prose. Not parseable. Only the cross-project brief carries a fixed provenance line (`mcp.rs`, `brief_provenance`). |
| Git upstream | The base the branch was cut from | Four of six point at the starter's branch. Two have none. A lane cut from `origin/main` by a starter would have nothing. |
| Slug prefix | `design-workspace-view-*` | Five of six share a prefix. The starter does not share it. The sixth child does not either. |
| `.worktrees.places.json` | 10 keys per place today | No starter field. |

**The starting place is recorded nowhere.** The MCP server knows it at the
moment of the call: `Server::caller_place` derives the caller from the launch
directory, and a caller cannot pass it as an argument. It is used for a
message's `from` and then dropped.

Two more facts that shape the design:

- The store never forgets. It holds 176 entries for 13 places on disk. A slug
  that is reused today inherits whatever its dead namesake left.
- A place's `created_epoch` is the directory's birth time
  (`project.rs`, `stat_birth`). It is derived, so it is always the live
  directory's.

## 2. The record

One new optional field per place in `.worktrees.places.json`:

```json
"design-workspace-view-kimi": {
  "started_by": { "place": "design-workspace-chats", "at": 1795557526 }
}
```

A starter in another project adds its registry name:
`{ "project": "worktrees", "place": "design-workspace-chats", "at": … }`.

Rules:

- **Declared, not derived.** Git and tmux do not know who started a place
  (§1). This is the state split's "declared" side, next to lifecycle and pin.
- **Written once, by core, when `cmd_new` CREATES the worktree.** A `new` that
  reuses an existing place writes nothing. A creation with no starter CLEARS
  the field, because the store never forgets and the slug may be reused.
- **Never touched by a launch.** `open`, `open --brief`, a relaunch and a
  harness switch leave it alone. That is the difference from `launched_by`.
- **It rides the path `launched_by` rides.** The entry point hands core the
  starter on the `Ui` (as `CaptureUi.launched_by` does today). Core writes it
  in the same `store::edit` family, before the session exists.
- **It is a label, never a grant.** No code may read it to decide what a
  caller can do. A committed store can then carry one without harm. Authority
  stays with `launched_by`'s cap and with nested lanes' scope, if that is
  ever built.

Who writes it:

| Entry point | Starter recorded |
|---|---|
| MCP `create_worktree` from a lane | That lane (`caller_place`) |
| MCP `create_worktree` from `(main)` | None |
| MCP `create_worktree` into another project | `project` + `place` of the caller, unless the caller is `(main)` |
| CLI `worktrees new` run with its cwd inside a lane | That lane, by the same cwd rule |
| CLI `worktrees new` from `(main)` or outside the repo | None |
| The app's New dialog, a phone, a plan's Start | None. A person started it. |
| MCP `open_place` with a brief (a re-brief) | Unchanged. Origin is fixed. |

**`(main)` as the starter means no parent.** `(main)` is already the root of
every project in the nav, and the orchestrator normally sits there. A lane it
starts is a top-level lane, which is what the nav draws today. Nested lanes
decided the same: an absent parent means "under `(main)`".

Reading it. A link holds only when all of these are true:

1. The named place exists in the snapshot (same project, or a registered
   project by that name).
2. The starter's `created_epoch` is not later than `at`. A slug that was
   removed and made again is a different place, and adopts no orphans.
3. Following links upward never returns to the place itself.

A link that fails is drawn as no link. Nothing rewrites the store to "fix" it.

The snapshot carries the field through `Place` (`model.rs`), `ls --json`,
MCP `list_places` rows and the app's `Place` type, like any declared key.

## 3. The model

**A tree by origin, drawn shallow. No named groups.**

| Question | Answer |
|---|---|
| Depth | Recorded at any depth: the field names the true starter. The nav draws ONE indent level. A grandchild sits in its top ancestor's group, and its header names its real starter. |
| Starter removed | The child's link fails rule 1. It becomes a top-level lane. Nothing is deleted or re-parented. |
| Starter closed, children live | Covered by the stand-in row (§4.2). |
| Child re-briefed by another place | No change. The header can say who briefed it last only if that is recorded; not proposed. |
| A place moved by hand, or a moved project | The link is by slug, and slugs survive a project move. A hand-renamed directory is a new slug and loses its links, like every other declared key. |
| Started in another project | The child shows in ITS project with a "from worktrees: design-workspace-chats" line. It never nests across projects. |
| A place with no agent | Same rule. The link is about creation, not about a session. |
| Fixed or editable | The record is fixed. An owner override sits beside it (phase 2, §6). |

Why one level in the nav: the Places column is about 400 px wide and already
cuts `design-workspace-view-com…`. Each level costs about 16 px of name. The
live tree here has depth 1. Depth 2 is plausible (a design lane starts a
critique lane). Depth 3 was not seen.

## 4. The nav

### 4.1 The example, after

Today (the screenshot): nine flat rows under ACTIVE.

```text
v worktrees                                12  +
    ◆ (main)                           ↓1  1h
  v ACTIVE                                  9
      experiment-with-disk-inventory
      feat-documents-in-main           ↑5  19m
      design-workspace-chats           ↑4  29m
      design-workspace-view-combined   ↑5  30m
      design-workspace-view-kimi       ↑6  39m
      design-workspace-view-fable      ↑5  40m
      design-workspace-view-memdoor    ↑5  47m
      design-workspace-view-codex      ↑2  1h
      research-memdoor-harness         ↑1  20h
```

Proposed, expanded:

```text
v worktrees                                12  +
    ◆ (main)                           ↓1  1h
  v ACTIVE                                  9
      experiment-with-disk-inventory
    v design-workspace-chats        6  ↑4  29m
        feat-documents-in-main         ↑5  19m
        design-workspace-view-combined ↑5  30m
        design-workspace-view-kimi     ↑6  39m
        design-workspace-view-fable    ↑5  40m
        design-workspace-view-memdoor  ↑5  47m
        design-workspace-view-codex    ↑2  1h
      research-memdoor-harness         ↑1  20h
```

Proposed, collapsed. Nine rows become four:

```text
  v ACTIVE                                  9
      experiment-with-disk-inventory
    > design-workspace-chats      6 ●  ↑4  29m
      research-memdoor-harness         ↑1  20h
```

- The tier count stays 9. It counts places, as today.
- `6` is the number of lanes this place started that still exist, in any tier.
- `●` after the count is the children's rolled-up dot (§4.4). The row's own
  dot stays in its own slot on the left, unchanged.
- Clicking the caret toggles. Clicking the row selects the place, as today.
- Open by default. Collapse state is the frontend's (`ui-state.json`, the
  same open-map the tier groups use), keyed by project and starter slug.

### 4.2 A child in a different tier than its starter

Tiers stay truthful. A row sits in the tier `store::reconcile` gives IT, and
never moves because of a relative. So a family can be split across tiers.
The rule, per tier:

1. Children whose starter is in the same tier nest under the starter's row.
2. Two or more children whose starter is in ANOTHER tier nest under a
   **stand-in row**: the starter's name, dim, with the tier it is really in.
   It is not counted, not draggable, and a click selects the starter.
3. A single such child gets no stand-in. It carries a caption line instead.

Starter closed, children still working (the orchestrator finished first):

```text
  v ACTIVE                                  4
    v design-workspace-chats · closed   3 ●
        design-workspace-view-kimi     ↑6  39m
        design-workspace-view-fable    ↑5  40m
        design-workspace-view-codex    ↑2  1h
      research-memdoor-harness         ↑1  20h
  > DORMANT                                 1
```

Starter active, some children closed:

```text
  v ACTIVE                                  3
    v design-workspace-chats        6  ↑4  29m
        feat-documents-in-main         ↑5  19m
        design-workspace-view-combined ↑5  30m
  v DORMANT                                 4
      CLOSED
      v design-workspace-chats · active   4
          design-workspace-view-kimi
          design-workspace-view-fable
          design-workspace-view-memdoor
          design-workspace-view-codex
```

One child alone in another tier, and a child from another project:

```text
  v PINNED                                  1
      design-workspace-view-fable      ↑5  40m
        ↳ design-workspace-chats
  v ACTIVE
      fix-ios-push                     ↑1  5m
        ↳ worktrees-ios: feat-push-notifications
```

The caption takes the line the branch name uses today when both do not fit.
Phase 1 may show the caption and move the branch to the tooltip for these
rows.

Rejected: placing the whole family in the starter's tier (nested lanes §6
drew it that way for hubs). A closed starter would hide three working agents
inside Dormant, which is collapsed by default. A row under ACTIVE that has no
live session would also break what ACTIVE means, and `predictTier` with it.

### 4.3 Sorting, pins, filter, drag

| Existing piece | What changes |
|---|---|
| Tiers and `store::reconcile` | Nothing. |
| `bucketOf`, pins | Nothing. A pinned child goes to Pinned with its caption or stand-in, by the rule above. |
| Sort: last used | Top-level rows sort as today, except a group sorts by the newest activity of its header and its shown children. A busy child lifts its group. Children sort among themselves by the same mode. |
| Sort: A to Z | Groups sort by the starter's name. Children sort by name. |
| Sort: manual | `manual_order[repo]` stays one flat slug array. A group's position is its starter's. Children order among themselves by the same array. `normalizeOrder` and `spliceOrder` are unchanged. |
| Filter box | A matching child shows under its starter. A starter that does not match itself is drawn dim, as context. Collapse is ignored while a filter is typed and is not overwritten. |
| Drag between tiers | Unchanged: a lifecycle edit on that ONE row. Dragging a starter does not move its children. Each has its own session. |
| Drag to reorder (manual) | A child reorders among its siblings. A top-level row reorders among top-level rows, and its group travels with it. A drop that would cross that line is refused with a hint. |
| `dnd.ts::predictTier`, `dropIntent` | Unchanged, and `dnd-check.mjs` keeps passing as is. Only the landing-slot code (`tierRows`, `pointerIndex`) learns that a tier's rows have two levels. |
| Workspaces / All projects | Both draw projects with `ProjectNode`. One change serves both. |
| Activity dots, unread rings on rows | Unchanged per row. |

Drag does NOT re-parent in any phase proposed here. A drop on a row and a
drop between rows are 10 px apart in this nav, and a miss would silently
change a relation. The override is a menu item (§6).

### 4.4 Roll-up on a collapsed group

Use the project header's existing rule (`ProjectNode`, the `rollup` value),
over the group's children only:

1. Busy if any child is busy.
2. Else waiting if any child is waiting.
3. Else done if any child just finished (the freshest tier only).
4. Else nothing.

Rules that keep it honest:

- The roll-up dot sits by the count. It never replaces the starter's own dot.
- It is drawn only while the group is collapsed. Expanded, the rows speak.
- It reads `unseenWork` and writes nothing. Expanding a group, or selecting
  the starter, does not mark a child read. Entering the child does, as today.
- The tooltip separates the two: "3 lanes: 1 working, 1 needs input".

Waiting-over-busy is the other defensible order (nested lanes §6 chose it).
The project header uses busy-first today, and two orders in one column would
read as a bug. If the owner wants waiting first, change both together.

## 5. Beyond the nav

| Surface | What it shows | Phase |
|---|---|---|
| Place header | A child: "started by design-workspace-chats", a link. A starter: "started 6 lanes". | 1 |
| `worktrees ls --json`, MCP `list_places` | `started_by` on each row. An orchestrating lane filters for its own slug to list ITS lanes. | 1 |
| MCP `place_status` | `started_by`, and `started: [slugs]` for the place asked about. | 2 |
| `worktrees ls` (text) | A `↳ starter` suffix, and `--tree` to indent. | 2 |
| Close and remove | Closing or removing a starter says "it started 6 lanes; 4 are still open" and offers to open the list. No cascade. Each child is closed or removed on its own. | 2 |
| iOS list | The same indent and count, from the same field in `list_workspace`. No new row; an older app ignores the key. | 2 |
| Chat | Nothing new. The header line covers it. | none |
| Default recipient of `report` | Stays `(main)`. A child reporting to its starter by default is a behaviour change for every brief written so far. See §9. | not proposed |

## 6. The owner override (phase 2)

A second optional field, set only by a person:

```json
"feat-documents-in-main": { "under": "(top)" }
"fix-shared-chat-images":  { "under": "design-workspace-chats" }
```

- `under` wins over `started_by` for DRAWING. `started_by` is never edited,
  so the header still says who really started the lane.
- Menu items on a place row: "Move under…" (a picker of this project's
  places), "Move to top level", and "Back to its starter" when both differ.
- It goes through a serve row and a CLI verb (`worktrees place under <slug>
  <starter|--top>`), so the phone and the terminal agree with the nav.
- The same validity rules as §2 apply. A cycle is refused at write time.
- It also fixes history. The six live children have no record and never
  will. The owner can file them with six menu picks, or one CLI line each.

## 7. Designs compared

| | A. Recorded starter, shallow tree | B. Owner-made groups | C. Derived from names or git | D. Full hubs (nested lanes) |
|---|---|---|---|---|
| The live example after shipping | New lanes nest with no action | Nothing happens until the owner makes a group and files 6 lanes | 5 of 6 by prefix, under no real row. 4 of 6 by upstream. | Nests, after the hub flow exists |
| Who does the work | The tool, at creation | The owner, every time | Nobody | The tool |
| Wrong answers | None for new lanes. Old lanes stay flat. | None, but stale groups pile up | Frequent, silent | None |
| Says who started what | Yes | No | No | Yes |
| New declared state | 1 field (2 with the override) | Group list + membership + order | 0 | 5+ fields, a journal, a lock |
| Touches tiers, `reconcile`, `predictTier` | No | No | No | Yes |
| Handles "group unrelated lanes" | Only with the override | Yes | No | No |
| Size | S (phase 1), S (phase 2) | M | S, then rework | XL, behind tmux routing |

**B, end to end.** The owner right-clicks, makes "workspace view", drags six
lanes in. Groups need a name, an order, a place in each tier, a rule for a
group that spans tiers, an empty state and a delete. Lanes started tomorrow
land outside it again. It answers "a way to group them" and misses the
complaint, which was that the tool knew the relation and showed nothing.

**C, end to end.** No record. The nav groups rows that share a slug prefix,
or whose upstream is another place's branch. On today's data it would put
five lanes under a header that is no place, leave the real starter outside
its own group, and miss the sixth. It cannot be corrected, because there is
nothing to edit.

**D** is the right design for a different problem: a feature branch that
collects child PRs. Its `parent` changes merge targets and close-out. Using
it to answer "show me who started what" would make every orchestrating design
lane a hub.

**Recommendation: A, then the override from §6.** A with the override covers
B's one real use (filing a lane by hand) without a second concept.

### The smallest first phase

Phase 1 fixes the complaint for every lane started after it ships:

1. `started_by` in `Declared`, written by `cmd_new` on creation, from MCP
   `create_worktree` and from a CLI `new` run inside a lane.
2. The field in `ls --json`, `list_places` and the app's `Place`.
3. The nav: same-tier nesting, the stand-in row, the caption, the count, the
   collapsed roll-up, collapse state.
4. The header line.
5. The mock harness: fixtures with a starter, one split family, one orphan.

| Phase | Adds | Size |
|---|---|---|
| 1 | The record and the nav | S to M |
| 2 | The override, `place_status`, close/remove wording, `ls --tree`, iOS | S to M |
| 3 | Cross-project counts on the starter's row, the plan overview (§8) | S |

One upgrade trap. A lane's `worktrees mcp` server is as old as its session
(AGENTS.md). A lane that was running before the upgrade starts children with
the old server, and they get no record. The fix is to restart that lane, or
to file its children by hand in phase 2.

## 8. How this meets plans, workspace chats and hubs

**A plan's work items** ([workspace chats](workspace-chats.html), and phase 5
of the idea board). A work item names its place in the plan's own `started:`
line. That is the same kind of fact, held on the other side: the plan knows
which place it started. A place started by Start has no `started_by`, because
a person pressed the button and a plan is not a place.

The two compose and do not merge:

- A place started from a work item is a top-level lane.
- If its agent starts lanes, they nest under it, by `started_by`.
- The workspace overview can then show an item's place with "+3 lanes" from
  the same field. That is phase 3.
- A workspace chat proposes work items and never starts a place itself
  (workspace chats §1). So a chat is never a starter.

**Hubs.** If nested lanes is built, a hub's children are created by the hub's
agent with an explicit `parent`. That call also records `started_by`, with
the same value. The nav then needs no second tree: a hub is a starter whose
row also says `hub`. `parent` keeps its git and authority meaning, and
`started_by` keeps meaning origin. This proposal does not need hubs, and
does not block them. It uses the name `started_by` so that `parent` stays
free for them.

## 9. Owner choices

| # | Choice | My pick |
|---|---|---|
| 1 | Tree by recorded starter, owner-made groups, or both | The tree. No named groups. |
| 2 | Does `(main)` count as a starter | No. Its lanes are top level, as today. |
| 3 | Is a CLI `worktrees new` typed in a lane's pane a child of that lane | Yes. Same cwd rule as MCP. A person typing there gets the same result, and the override can undo it. |
| 4 | Nav depth | One indent level. Deeper lanes join their top ancestor's group. |
| 5 | A child in another tier than its starter | Each row stays in its own tier. Stand-in row for 2 or more, caption for 1. |
| 6 | Groups open or collapsed by default | Open. |
| 7 | Roll-up order on a collapsed group | Busy, then waiting, then done: the project header's order. |
| 8 | Does dragging re-parent | No. Menu only, in phase 2. |
| 9 | Closing or removing a starter | Say how many lanes it started and how many are open. Never cascade. |
| 10 | Should a child's `report` default to its starter | Not now. Decide it with nested lanes, where it was first proposed. |
| 11 | Backfill the six live children | By hand with the phase 2 override. No guessing from briefs or upstreams. |
| 12 | Field name | `started_by`, leaving `parent` for hubs. |

## 10. Not established

- **Why the six children carry no `launched_by`.** The starter has a console
  stamp, and `create_worktree` passes the server's stamp on. The likely
  cause is that the starter's stamp was written by a later relaunch, after
  its children existed. Not traced. It does not change the design, because
  the stamp names a device either way.
- **Nothing was drawn or measured in the app.** The mockups are ASCII. Row
  widths, the caption beside the branch line, hit targets for the caret next
  to the dot, and the stand-in row's contrast in the light themes need the
  mock harness and WebKit, per the AGENTS.md layout rules.
- **Drag with two levels.** The gap arithmetic (`pointerIndex`, `naturalTop`)
  was read, not run, against nested rows.
- **Birth time on Linux.** Rule 2 in §2 relies on `stat_birth`. What it
  returns on a filesystem with no birth time was not checked. If it can be
  absent, rule 2 must pass rather than fail.
- **The iOS list.** Its repository was not read. "An older app ignores the
  key" is the wire's general rule, not a check of that client.
- **Depth beyond 1 in real use.** One live tree was looked at. It has depth 1.
- **Cross-project children.** No live example exists on this machine. The
  caption design is untested against a real one.
- **Whether CLI `new` can always tell its place from its cwd.** The MCP
  server does it with `place_containing`. The CLI path was not traced.
