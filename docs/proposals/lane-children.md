---
title: "Proposal: lanes started by a lane show under it"
---

# Proposal: lanes started by a lane show under it

**Status:** design, revision 2, 2026-11-25. Nothing built. The owner answered
the twelve choices of revision 1 on 2026-11-25 (§12). Items marked **owner to
confirm** are proposals he has not seen yet. They are listed in §12.

**The answer.** Record which place started each place, once, at creation
(`started_by`). Draw the nav as a real tree from it, to any depth: a lane sits
under the lane that started it, behind a disclosure. A collapsed row shows how
many lanes it hides and one dot, and a lane that waits for the owner always
wins that dot. A child's `report` with no `to` goes to its nearest live
ancestor. `(main)` and the owner can move any lane to the top level or under
another lane; a lane can move nothing. Removing a starter sends its lanes up
one level, for drawing and for reports. It never closes or removes them.

What changed since revision 1:

| # | Revision 1 | Revision 2 |
|---|---|---|
| 4 | One indent level; grandchildren flattened | Real depth (§5) |
| 5 | Stand-in row for 2+, caption for 1 | One rule: ghost rows (§5.4) |
| 7 | Busy, waiting, done | Waiting, busy, done (§5.5) |
| 9 | Say how many lanes it started | Removal sends them up one level (§7) |
| 10 | `report` default unchanged | Nearest live ancestor, in phase 1 (§6) |
| new | Override in phase 2 | Adoption by `(main)` and the owner, in phase 1 (§8) |

This is still the small half of [nested lanes](nested-lanes.html). That
proposal's `parent` means ownership: a hub branch, a merge target, scoped
authority. Nothing here grants anything or changes a git flow. §10 says how
the two fit.

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
| `launched_by` (`store.rs`, `ops::stamp_launch`) | Principal and device of the request that created the SESSION. Rewritten or cleared on every launch. | Names a device, never a place. All six children have none. |
| Message log (`worktrees-messages` in the git common dir) | `from`, `to`, `text`, `created`, `id` | 26 messages in total. One names the pair. Claude lanes talk over Claude's own bus. |
| The brief (`.planning/brief.md`) | Free text the starter wrote | Five of six mention the starter, in prose. Not parseable. |
| Git upstream | The base the branch was cut from | Four of six point at the starter's branch. Two have none. |
| Slug prefix | `design-workspace-view-*` | Five of six share it. The starter does not. |
| `.worktrees.places.json` | 10 keys per place today | No starter field. |

**The starting place is recorded nowhere.** The MCP server knows it at the
moment of the call (`Server::caller_place`, from the launch directory; a
caller cannot pass it). It is used for a message's `from` and then dropped.

Two more facts that shape the design:

- The store never forgets. It holds 176 entries for 13 places on disk. A slug
  that is reused today inherits whatever its dead namesake left.
- A place's `created_epoch` is the directory's birth time (`project.rs`,
  `stat_birth`). It is derived, so it is always the live directory's.

## 2. The record

Three optional fields per place in `.worktrees.places.json`. Each has one
writer and one meaning.

```json
"critique-view-combined": {
  "started_by": { "place": "design-workspace-view-combined", "at": 1795557526 },
  "handed_to":  { "place": "design-workspace-chats", "at": 1795601200,
                  "from": "design-workspace-view-combined" },
  "under":      { "place": "(main)", "at": 1795603000 }
}
```

| Field | Means | Written by | Rewritten |
|---|---|---|---|
| `started_by` | Origin: the place whose agent or pane created this one | Core, when `cmd_new` CREATES the worktree | Never |
| `handed_to` | The starter was removed; this place is now the next one up's | Core, when `rm` removes the starter (§7). **Owner to confirm.** | Only by a later removal |
| `under` | Where the owner or `(main)` wants it drawn. Display only. | The adoption row (§8) | By the next adoption, or cleared |

Rules for all three:

- **Declared, not derived.** Git and tmux do not know any of this (§1).
- **A creation clears all three.** The store never forgets and a slug may be
  reused. A `new` that reuses an existing place writes nothing.
- **Never touched by a launch.** `open`, `open --brief`, a relaunch and a
  harness switch leave them alone. That is the difference from `launched_by`.
- **Labels, never grants.** No code may read them to decide what a caller can
  do. A committed store can then carry them without harm.

Who writes `started_by`:

| Entry point | Starter recorded |
|---|---|
| MCP `create_worktree` from a lane | That lane (`caller_place`) |
| MCP `create_worktree` from `(main)` | None. Decided: `(main)` is not a starter. |
| MCP `create_worktree` into another project | `project` + `place` of the caller, unless the caller is `(main)` |
| CLI `worktrees new` run with its cwd inside a lane | That lane. Decided. |
| CLI `worktrees new` from `(main)` or outside the repo | None |
| The app's New dialog, a phone, a plan's Start | None. A person started it. |
| MCP `open_place` with a brief (a re-brief) | Unchanged. Origin is fixed. |

A link (any of the three) holds only when:

1. The named place exists in the snapshot, or is `(main)`.
2. The named place's `created_epoch` is not later than the link's `at`. A
   slug that was removed and made again is a different place.
3. Following links upward never returns to the place itself.

A link that fails is read as absent. Nothing rewrites the store to fix it.

**Core resolves, clients draw.** Core computes three values per place and
puts them in the snapshot (`Place`, `ls --json`, MCP `list_places`, the app's
`Place` type):

| Derived value | Rule |
|---|---|
| `lineage.starter` | `started_by`, when it holds. For the header. |
| `lineage.parent` | The drawn parent: the first that holds of `under`, `handed_to`, `started_by`. Absent means top level. |
| `lineage.reports_to` | The default recipient today (§6). |

The nav builds its tree from `lineage.parent` alone. It re-implements no
rule, so it needs no drift check like `dnd-check.mjs`.

## 3. The model

**A tree by origin, drawn to its real depth. No named groups.** (Decided.)

| Question | Answer |
|---|---|
| Depth | Any. A grandchild sits under its parent. |
| Starter closed, children live | The children stay under it. In a tier it is not in, it is drawn as a ghost row (§5.4). Their reports already go further up (§6). |
| Starter removed | Its children move up one level: drawn there and reporting there (§7). |
| Child re-briefed by another place | No change. Origin is fixed. |
| A place moved by hand, or a moved project | Links are by slug, and slugs survive a project move. A hand-renamed directory is a new slug and loses its links, like every declared key. |
| Started in another project | The child shows in ITS project, at top level, and its header says "from worktrees: design-workspace-chats". It never nests across projects. |
| A place with no agent | Same rule. The link is about creation, not about a session. |
| Fixed or editable | Origin is fixed. Where it is drawn can be changed by `(main)` and the owner (§8). |

## 4. What was drawn and measured

Revision 1 drew nothing. This revision drew a static page that loads the
app's real `app/src/tokens.css` and `app/src/App.css`, uses the nav's real
markup (`.project`, `.group`, `.places`, `.row`), and adds only the rules
proposed here. Rendered in Playwright WebKit at 2x. Every number below is
`getBoundingClientRect` on that page. Chromium gave the same name widths at
240 px. The page and its script are in the lane's scratch dir, not in the
repository. It is not the app: §13 says what that leaves open.

## 5. The nav

### 5.1 Three levels deep

Each tier stays one flat list of rows, as today. A row carries its depth as a
number. Depth is drawn by indent, a rail and a caret.

```text
v worktrees                                    10
    ◆ (main)                               ↓1  1h
  v ACTIVE                                      9
      experiment-with-disk-inventory           2d
    v ● design-workspace-chats         ↑4 *   29m
      │ ● feat-documents-in-main       ↑5 *   19m
      │ v design-workspace-view-combined  ↑5  30m
      │ │ v ● critique-view-combined   ↑2 *    4m
      │ │ │ ● probe-critique-fonts     ↑1 *    2m
      │ ◉ design-workspace-view-kimi   ↑6     39m
      │   design-workspace-view-codex  ↑2      1h
      research-memdoor-harness         ↑1     20h
```

| Piece | Rule | Measured |
|---|---|---|
| Indent | One step per level. The step is the tree's own `--ind`, 14 px. | Name starts at x = 62, 76, 90, 104 for levels 0 to 3 |
| Rail | One 1 px line per level, in `--guide`, through the children's gutter. It runs under the parent's dot. | x = 44, 58, 72 for levels 1 to 3; the parent's dot centre is 44, 58, 72 |
| Caret | A row that has lanes drawn under it gets a caret left of its dot. It sits on the rail of its own level, as tier carets sit on the project's rail today. | Button 14 x 29 px (the full row height). Hit-tested with `elementFromPoint` at every level. |
| Click | The caret toggles. The row selects the place, as today. | |
| Row's own dot, glyphs, age | Unchanged. | |
| Tier count | Counts places in that tier, as today. Ghost rows are not counted. | |

Open by default (decided). Collapse state is the frontend's, a new map in
`ui-state.json` keyed by project and slug.

### 5.2 Depth 4 and deeper, and a narrow nav

The nav's floor is 220 px (`NAV_MIN`). Its default is 300 px. A name is 15 px
medium, about 8 px a character. Width left for the name on the fullest row
(ahead count, agent mark, age), in px:

| Nav width | L0 | L1 | L2 | L3 | L4 | L5 |
|---|---|---|---|---|---|---|
| 400, step 14 | 246 | 232 | 218 | 204 | 190 | 176 |
| 300, step 14 | 146 | 132 | 118 | 104 | 90 | 76 |
| 240, step 14 | 86 | 72 | 58 | 44 | 30 | 16 |
| 220, step 14 | 66 | 52 | 38 | 24 | 10 | 0 |

A plain 14 px ladder leaves two characters at level 4 in a 240 px nav, and
nothing at level 5 in a 220 px one. So the ladder has a stop:

| Nav width | Step | Indent stops after | Name width at the stop |
|---|---|---|---|
| 280 px and wider | 14 px | level 4 | 90 px at 300, 70 px at 280 |
| under 280 px | 8 px | level 3 | 62 px at 240, 42 px at 220 |

A row deeper than the stop keeps the last indent and shows its level as a
small number before its name. Order still says whose child it is: a row
follows its parent.

```text
  v ACTIVE                         8      (240 px nav)
    v ● design-wo…          ↑4 *  29m
      v design-work…        ↑5    30m
       v critique-vie…      ↑2     4m
        v ● probe-…         ↑1 *   2m
        v ● 4 probe-fo…        *   1m
          ● 5 probe-fo…        *   1m
```

With that rule the name never drops under 62 px at 240 px, at any depth. No
scene scrolled sideways at 220, 240, 300 or 400 px. **Owner to confirm** the
two numbers (14 px to level 4; 8 px to level 3 under 280 px).

### 5.3 A collapsed row

```text
  v ACTIVE                                     12
      experiment-with-disk-inventory           2d
    > ● design-workspace-chats   (● 6)  ↑4 *  29m
    >   design-lane-children     (● 2)  ↑1    12m
      research-memdoor-harness          ↑1    20h
```

- The pill appears only while the row is collapsed. Expanded, the rows speak,
  and the name keeps the 29 px the pill would take.
- The number is how many lanes the row hides in this tier, at every depth
  below it. Here: 6, although only 4 are direct children.
- The dot in the pill is the roll-up (§5.5). The first row hides a waiting
  lane three levels down, so its pill is amber. Its own dot stays green in
  its own slot: it is working.
- A collapsed child inside an open parent shows its own pill. Every level
  answers for what it hides.

### 5.4 A lane in another tier than its starter

In plain words. A tier says what state a lane is in. ACTIVE means it has a
running session. CLOSED means it has none. A starter and the lanes it started
can be in different states: the orchestrator finishes and is closed while two
of its lanes still work. Those two are active, so they stay in ACTIVE. To
show whose they are, ACTIVE draws a dim copy of the closed starter above
them, with a label that says where the real row is. That copy is a **ghost
row**. The real row is in CLOSED.

```text
  v ACTIVE                                      4
    v design-workspace-chats            [CLOSED]     <- ghost: the real row is below
      │ ● design-workspace-view-kimi   ↑6 *   39m
      │ ● design-workspace-view-fable  ↑5 *   40m
      ● fix-shared-chat-images         ↑3 *    8m
        research-memdoor-harness       ↑1     20h
  v DORMANT                                     3
      CLOSED
        design-workspace-chats                 3h    <- the real row
      v fix-shared-chat-images          [ACTIVE]     <- ghost: its real row is above
        │ probe-image-cache                    1d
        │ probe-image-etag                     1d
```

One rule, at every depth: **a row is drawn in the tier it is really in, with
its starters above it, all the way up. A starter that lives in another tier
is drawn as a ghost.** A closed lane in the middle of a chain is a ghost in
the middle:

```text
  v ACTIVE                                      3
    v ● design-workspace-chats         ↑4 *   29m
      │ v design-workspace-view-combined [CLOSED]
      │ │ ● critique-view-combined     ↑2 *    4m
      │   design-workspace-view-kimi   ↑6     39m
```

A ghost row:

- Shows the name in italics in `--txt-dim`, and the real tier in a small
  outlined label. No dot, no glyphs, no age.
- Is not counted in the tier, cannot be dragged, and has a caret and a pill
  like any row.
- Selects the real place on click.

This replaces revision 1's two cases (a stand-in for two or more children, a
caption for one). One rule is easier to learn than two, and a lone child
costs one extra line either way.

Ghost name against the nav background, by theme (`--txt-dim`; the row age
uses `--txt-mute`):

| Theme | Ghost name | Row age, for scale | A real name |
|---|---|---|---|
| tokyo-night | 6.0 | 2.9 | 15.1 |
| tokyo-day | 3.2 | 2.2 | 6.2 |
| catppuccin-mocha | 7.9 | 3.6 | 12.1 |
| catppuccin-latte | 4.1 | 2.6 | 6.6 |
| nord | 4.6 | 3.1 | 10.8 |
| gruvbox-dark | 6.8 | 4.0 | 13.0 |

The ghost reads between the app's own dim text and its real names in all six.

**The other design, rejected.** Keep each family whole, in the tier of its
liveliest member, and draw the quieter members dim in place. It needs no
ghost rows. It was rejected for three reasons. A starter with four closed
lanes would show four closed rows under ACTIVE, which is what tiers exist to
hide. The tier count would stop meaning "places in this state". And dropping
a row on a tier would no longer say where the row lands, which is the rule
`dnd.ts::predictTier` mirrors from `store::reconcile`. **Owner to confirm**
ghost rows over this.

### 5.5 The roll-up dot

On a collapsed row, over every lane it hides in that tier:

1. **Waiting** if any hidden lane is waiting.
2. Else busy if any is busy.
3. Else done if any just finished (the freshest tier only).
4. Else no dot, and the pill shows the number alone.

Decided by the owner: a collapsed group must never hide a lane that needs
him. Rules that keep it honest:

- It sits in the pill. It never replaces the row's own dot.
- It reads `unseenWork` and writes nothing. Expanding a row, or selecting it,
  marks no hidden lane read. Entering the lane does, as today.
- The tooltip separates the states: "6 lanes: 1 needs input, 2 working".
- A pinned lane lives in PINNED, under a ghost of its starter there. Its
  starter's real row does not roll it up, because it does not hide it.

The project header's folder badge uses busy first today (`ProjectNode`, the
`rollup` value). Two orders in one column would read as a bug. So phase 1
changes the project badge to waiting first as well. **Owner to confirm.**

### 5.6 Sorting, pins, filter, drag

| Existing piece | What changes |
|---|---|
| Tiers and `store::reconcile` | Nothing. |
| `bucketOf`, pins | Nothing. A pinned lane goes to PINNED, under ghosts of its starters. |
| Sort: last used | Siblings sort among themselves. A row sorts by the newest activity in the subtree drawn under it, so a busy grandchild lifts its family. |
| Sort: A to Z | Siblings sort by name, at every level. |
| Sort: manual | `manual_order[repo]` stays one flat slug array. Siblings order by it. `normalizeOrder` and `spliceOrder` are unchanged. |
| Filter box | A matching row shows with its starters above it, dim, as context. Collapse is ignored while a filter is typed and is not overwritten. |
| Drag between tiers | Unchanged: a lifecycle edit on that ONE row. Its lanes do not follow. Each has its own session. |
| Drag to reorder (manual) | A row reorders among its siblings, and its subtree travels with it. A drop among another parent's rows is refused with a hint. |
| Drag re-parents | Never. Decided. The menu does it (§8). |
| `dnd.ts::predictTier`, `dropIntent` | Unchanged, and `dnd-check.mjs` keeps passing as is. The landing-slot code (`tierRows`, `pointerIndex`) learns that rows have a depth and that ghosts are not slots. |
| Workspaces / All projects | Both draw projects with `ProjectNode`. One change serves both. |

## 6. Where a report goes

Today `report` with no `to` goes to `(main)` (`mcp.rs`, the `"report"` arm).
The owner asked why a child should not report to its starter now. So it does,
in phase 1. The rule below is `(main)`'s proposal with two additions of this
lane. **Owner to confirm** all of it.

1. An explicit `to` is unchanged.
2. No `to`, and a `reply_to`: the message goes to the sender of the message it
   answers. (Added here. Without it, a child's answer to a question from
   `(main)` would go to its starter.)
3. No `to`, no `reply_to`: the message goes to the **nearest live ancestor**.
   Walk up from the place: its `handed_to`, else its `started_by`. The first
   place on that walk that exists in this project and is live gets it.
4. Nothing live on the way up: `(main)`, live or not, as today.

"Live" means an agent is running in the place, by the one reader
`place_status` uses. A tmux session whose pane is back at the shell is not
live: a message there waits for nobody. (`(main)`'s wording was "has a
session". This is narrower.)

Adoption by menu (§8) does not change the walk. It is display only.

The reply tells the sender where it went and why:

```json
{ "id": "…", "to": "design-workspace-chats",
  "note": "Your starter design-workspace-view-combined has no agent running. Sent to the next lane up. Pass to: \"(main)\" to reach (main)." }
```

The stored message gets one optional field, `routed`, with the same reason,
so the recipient knows why a lane it never briefed wrote to it. An older
reader ignores the field.

What breaks, and what does not:

| Case | Effect |
|---|---|
| A lane started by `(main)` | Nothing. It has no starter. Default stays `(main)`. |
| Every lane that exists today | Nothing. None has a `started_by`, and none will get one. |
| An old brief that says "report to (main)", agent passes `to: "(main)"` | Nothing. Explicit `to` wins. |
| A NEW lane-started lane whose brief says "report to (main)", agent omits `to` | **Changes.** It goes to the starter. The reply says so and names the fix. This is the only exposure. The house rule already says a lane reports to the place that briefed it. |
| A lane expecting `(main)` to hear every finish | `(main)` no longer hears a grandchild's default report. The starter does, and is responsible for passing it on. `(main)` can still list them (`list_places`, `started_by`). |
| A starter that closes mid-task | Its child's earlier reports are in the starter's inbox, unread. Later ones go further up. Nothing replays the earlier ones. A reopened starter reads them with `messages`. |
| A starter that is reopened | It is live again, so its children's default reports come back to it at once. Nothing was written, so nothing has to be undone. |
| A starter in another project | Skipped. The walk stays in this project, then `(main)`. A cross-project report keeps needing an explicit address. |
| Claude lanes that use `SendMessage` | Unaffected. That is Claude's own bus. The rule is for MCP `report`. |
| Cost | One liveness read per default report: the pane list and the session probes `place_status` already reads. Not measured. |

The upgrade trap, in both directions. A lane's `worktrees mcp` is as old as
its session.

| Who is old | What happens |
|---|---|
| The STARTER's server | Its `create_worktree` writes no `started_by`. The child has no starter, so its default stays `(main)`. Today's behaviour. |
| The CHILD's server | Cannot happen for a new child: a new session starts the installed binary. |
| An old lane, re-parented by hand with §8 | `under` is display only, so its reports do not move. |

So a mixed fleet never routes wrongly. It routes as today until the starter
is restarted.

The tool description, the `worktrees` skill and the AGENTS.md "Which tool"
line all say "`to` defaults to (main)". They change with the code.

## 7. When a starter goes away

The owner's answer to "closing or removing a starter": its lanes go up one
level. Nothing cascades. No lane is closed or removed because its starter
was. The split below is `(main)`'s proposal. **Owner to confirm.** It holds,
with the edges listed.

| The starter… | Drawn | Default report | Written |
|---|---|---|---|
| has no agent running (closed, crashed, or back at its shell) | Its lanes stay under it. Where it is in another tier, under its ghost. | Skips it, by §6. Goes to the next live lane up. | Nothing |
| is reopened | Unchanged | Comes back to it | Nothing |
| is REMOVED with `rm` (CLI, MCP `remove_worktree`, the app) | Its lanes move under ITS parent, or to top level | The same parent, then up | `handed_to` on each of its lanes |
| is made again with the same slug | The new place gets nothing back | Unchanged | Nothing. Rule 2 of §2. |
| vanishes without `rm` (a hand `git worktree remove`) | Its lanes fall to top level | `(main)` | Nothing. `doctor` names them in phase 2. |

"Ends" therefore means two different facts, and neither is a lifecycle label:

- **Not live:** no agent runs there now. Read fresh on every report. Nothing
  is stored, so it undoes itself.
- **Removed:** `cmd_rm` took the worktree away. Stored once, as `handed_to`.
  It does not undo itself.

A label alone (`closed`, `archived`, `abandoned` set by hand while the agent
still runs) changes nothing here.

What `rm` writes. For each place whose `handed_to`, else `started_by`, names
the removed place: `handed_to = { place: <the removed place's own parent by
the same rule, or "(main)">, at: now, from: <removed slug> }`. `started_by`
is not touched, so the header still says who really started it. A lane that
was only DRAWN under the removed place (`under`) gets no write: its `under`
link fails and it falls back to its own line.

Edges:

- A lane can be drawn under a closed starter while it reports two levels up.
  The header says both: "started by X (closed) · reports to Y".
- The remove dialog must say what will happen before it happens: "It started
  3 lanes. They move up to design-workspace-chats." That sentence is phase 1,
  because removal now writes.
- A reopened starter whose agent starts blank will receive reports from lanes
  it does not remember. `open` resumes the conversation by default, which is
  the normal case. Not tested.

## 8. Adoption

**`(main)` can move any lane, at any depth. The owner can. A lane can move
nothing.**

Adoption changes where a lane is DRAWN. It writes `under`. It never rewrites
`started_by`, and it does not move the lane's reports (§6). A lane's subtree
comes with it, because its lanes still point at it.

| Door | Who | What |
|---|---|---|
| Nav menu on a place row | The owner | "Move to top level", "Move under…" (a picker of this project's places), "Back to its starter" when `under` is set |
| MCP `adopt_place { place, under? }` | `(main)`'s agent only. Any other caller is refused in words that name the rule. | `under` defaults to `(main)`, which means top level |
| CLI `worktrees place under <slug> <target \| --top \| --reset>` | The owner at a terminal. Refused when the cwd is inside a lane. | The same row |
| Serve row `set_place_under: Engine [write, place(repo, slug)]` | The app and a phone | The same core call |

All four go through one core function, which applies §2's validity rules and
refuses a cycle at write time.

Two honest limits. The CLI's cwd check is a courtesy, not a boundary: an
agent with a shell can `cd` out, as it can already start an unscoped lane
(the O10 overlap). The stake is where a row is drawn. And the MCP tool may
place a lane under ANOTHER lane, not only at top level. That is what backfill
needs: the owner decided the six live children are filed by hand, and
"Move under…" is the hand. **Owner to confirm** that `(main)`'s agent gets
the same reach as the menu.

An adopted lane, selected. Its header (schematic; the header was not drawn
from the app's own classes):

```text
critique-view-combined
started by design-workspace-view-combined · moved to top level by (main), 25 Nov
reports to design-workspace-view-combined
started 1 lane
```

The header lines, in general:

| Line | Shown when |
|---|---|
| "started by X" | `started_by` holds. A link to X. "(removed)" or "(closed)" after it when true. |
| "from worktrees: X" | The starter is in another project |
| "moved to top level by (main), date" / "moved under Y by the owner, date" | `under` is set |
| "now under Y" | `handed_to` is set |
| "reports to Z" | The default recipient is not the drawn parent |
| "started N lanes" | Any lane names it as starter |

## 9. Beyond the nav

| Surface | What it shows | Phase |
|---|---|---|
| Place header | The lines of §8 | 1 |
| `worktrees ls --json`, MCP `list_places` | The three fields and `lineage` on each row | 1 |
| MCP `place_status` | `lineage`, and `started: [slugs]` for the place asked about, so an orchestrating lane can list ITS lanes | 1 |
| Remove dialog, `rm` output | "It started 3 lanes. They move up to Y." | 1 |
| `worktrees ls` (text) | `--tree` to indent, a `↳ starter` suffix | 2 |
| `doctor` | Lanes whose starter vanished without `rm` | 2 |
| iOS list | The same indent, pill and ghost, from `lineage.parent` | 2 |
| Workspace overview | A work item's place with "+3 lanes" (§10) | 3 |
| Chat | Nothing new. The header covers it. | none |

## 10. How this meets plans, workspace chats and hubs

**A plan's work items** ([workspace chats](workspace-chats.html)). A work
item names its place in the plan's own `started:` line. That is the same kind
of fact, held on the other side. A place started by Start has no
`started_by`, because a person pressed the button and a plan is not a place.
The two compose: a place started from a work item is a top-level lane, the
lanes its agent starts nest under it, and the overview can then show
"+3 lanes" from the same field. A workspace chat proposes work items and
never starts a place itself, so a chat is never a starter.

**Hubs.** If nested lanes is built, a hub's children are created by the hub's
agent with an explicit `parent`. That call also records `started_by`, with
the same value, and the nav needs no second tree. `parent` keeps its git and
authority meaning. Nested lanes' own proposal that a child reports to its hub
becomes this proposal's §6, already built. The name `started_by` leaves
`parent` free for hubs (decided).

## 11. Designs compared

| | A. Recorded starter, real tree | B. Owner-made groups | C. Derived from names or git | D. Full hubs (nested lanes) |
|---|---|---|---|---|
| The live example after shipping | New lanes nest with no action | Nothing until the owner makes a group and files 6 lanes | 5 of 6 by prefix, under no real row. 4 of 6 by upstream. | Nests, after the hub flow exists |
| Who does the work | The tool, at creation | The owner, every time | Nobody | The tool |
| Wrong answers | None for new lanes. Old lanes stay flat until filed. | None, but stale groups pile up | Frequent, silent | None |
| Says who started what | Yes | No | No | Yes |
| New declared state | 3 fields | Group list + membership + order | 0 | 5+ fields, a journal, a lock |
| Touches tiers, `reconcile`, `predictTier` | No | No | No | Yes |
| Files an unrelated lane by hand | Yes, by adoption | Yes | No | No |

**Decided: A.** B answers "a way to group them" and misses the complaint,
which was that the tool knew the relation and showed nothing. C cannot be
corrected, because there is nothing to edit. D is the right design for a
feature branch that collects child PRs, and would make every orchestrating
design lane a hub.

### Phases

| Phase | Holds | Size |
|---|---|---|
| 1 | The three fields. `ls --json`, `list_places`, `place_status`. The nested nav with ghosts, pills and the depth stop. The header lines. Adoption (menu, CLI, `(main)`'s MCP tool). The removal re-parent and the remove dialog's sentence. The report default. The project badge's order. | M to L |
| 2 | `ls --tree`. `doctor` for vanished starters. The iOS list. | S to M |
| 3 | Cross-project counts on a starter's row. The plan overview's "+3 lanes". | S |

### Build order for phase 1

Each step is one commit a lane can gate on its own. Core first, the nav last,
so the nav is built on data that already exists.

1. **Core, the record.** `started_by`, `handed_to`, `under` in `Declared`.
   A pure `lineage` module: validity, the upward walk, the cycle guard.
   Unit tests for each rule of §2, each shown red first.
2. **Core, the writers.** `cmd_new` writes or clears on creation. The starter
   arrives on the `Ui`, the way `CaptureUi.launched_by` does. MCP
   `create_worktree` passes `caller_place`. CLI `new` resolves its cwd.
   bats: a lane's `new` records, `(main)`'s does not, a reused slug is clean.
3. **The snapshot.** `lineage` on `Place`, `ls --json`, `list_places`,
   `place_status` (`started`). Diff `ls --json` against the shipped binary.
4. **Removal.** `cmd_rm` writes `handed_to`. Its output names where the lanes
   went. Tests: one level, a chain, a lane with `under`, a reused slug.
5. **Reports.** The four rules of §6 in the `"report"` arm, the reply's
   `note`, the `routed` field, the tool description, the skill text. Tests:
   live starter, dead starter, removed starter, `reply_to`, explicit `to`.
6. **Adoption.** The core call, the serve row with its capability, the CLI
   verb, MCP `adopt_place` with its `(main)`-only refusal.
7. **The mock.** Fixtures in `app/src/mock/install.ts`: three levels, a
   split family, a chain with a closed middle, an adopted lane, depth 6.
8. **The nav.** Depth, rails, carets, the pill, ghosts, the depth stop, the
   collapse map, the roll-up order (rows and the project badge), the drag
   slots. A check script that measures the caret hit box and the name width
   at 220 and 240 px in WebKit.
9. **The header lines and the remove dialog.**
10. **One family driven end to end in the sandbox app**, by hand: start a
    lane from a lane, collapse it, close the starter, reopen it, remove it,
    adopt a grandchild. The mock answers instantly and cannot show a refresh
    race.

Steps 1 to 6 need no frontend. Steps 7 to 9 need no Rust beyond step 3.

## 12. Owner choices

Decided on 2026-11-25:

| # | Choice | Decision |
|---|---|---|
| 1 | Tree, groups, or both | The tree. No named groups. |
| 2 | Is `(main)` a starter | No. |
| 3 | CLI `new` typed in a lane's pane | Makes a child of that lane. |
| 4 | Nav depth | Real depth, to any level. |
| 6 | Open or collapsed by default | Open. |
| 7 | Roll-up order | Waiting, then busy, then done. |
| 8 | Does dragging re-parent | No. |
| 9 | A starter goes away | Its lanes go up one level. Never a cascade. |
| 10 | `report` default | Do it now. |
| 11 | Backfill the six live children | By hand. |
| 12 | Field name | `started_by`. |
| new | Adoption | `(main)` and the owner can adopt at any depth. A lane cannot. Display only, except on removal. |

To confirm:

| # | Proposal | Whose | My pick |
|---|---|---|---|
| A | Default report goes to the nearest live ancestor, `(main)` at the top (§6) | `(main)` | Yes |
| B | "Live" means an agent is running, not only that a session exists (§6) | this lane | Yes. A shell reads no mail. |
| C | A `reply_to` with no `to` goes to the asker (§6) | this lane | Yes. The rule misroutes replies without it. |
| D | Removing re-parents and is stored; closing only moves reports and stores nothing (§7) | `(main)` | Yes. It holds. |
| E | A starter that vanishes without `rm` leaves its lanes at top level, reporting to `(main)` (§7) | this lane | Yes. `doctor` names them in phase 2. |
| F | Cross-tier: ghost rows, over "the family follows its liveliest member" (§5.4) | this lane | Ghost rows |
| G | The depth stop: 14 px to level 4; 8 px to level 3 under 280 px (§5.2) | this lane | Yes |
| H | The pill shows only on a collapsed row (§5.3) | this lane | Yes. It gives 29 px back to the name. |
| I | The project folder badge also becomes waiting first (§5.5) | this lane | Yes. One order per column. |
| J | `(main)`'s MCP tool can move a lane under another lane, not only to top level (§8) | this lane | Yes. Backfill needs it. |
| K | A starter in another project is skipped by the default report (§6) | this lane | Yes |

## 13. Not established

- **Nothing was run in the app or the mock harness.** The drawings are a
  static page with the app's two stylesheets and the nav's markup. The agent
  mark is a stand-in glyph of the same width rule. Hover, selection on a
  ghost, focus rings and the row's enter button were not exercised.
- **The system WKWebView.** Measured in Playwright WebKit and checked once in
  Chromium. The app's own web view was not used.
- **Light themes by eye.** Contrast was computed in all six themes. Only
  tokyo-night was looked at as a picture.
- **Drag with depth.** The gap arithmetic (`pointerIndex`, `naturalTop`) was
  read, not run, against rows with a depth and ghost rows between them.
- **Rails as children of the row.** In the page a rail is drawn inside each
  row, so it paints over the hover fill. Today's rails sit under it. Which
  the build picks was not tried.
- **The cost of the liveness read** on every default report.
- **How many briefs say "report to (main)" and rely on the default.** The
  exposure in §6 is reasoned, not counted.
- **A reopened starter that starts blank.** Not tested.
- **Why the six live children carry no `launched_by`.** Not traced. It does
  not change the design.
- **Birth time on Linux.** Rule 2 in §2 relies on `stat_birth`. If it can be
  absent, rule 2 must pass rather than fail.
- **The iOS list.** Its repository was not read.
- **Depth beyond 1 in real use.** One live tree was looked at. It has depth 1.
  Depth 3 to 5 here is fixture data.
- **Cross-project children.** No live example exists on this machine.
- **Whether CLI `new` can always tell its place from its cwd.** The MCP
  server does it with `place_containing`. The CLI path was not traced.
