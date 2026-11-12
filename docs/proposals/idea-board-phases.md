---
title: "Ideas: the phase plan"
---

# Ideas: the phase plan

**Status:** plan, **revision 4, 2026-11-10**. Phases 0, 1 and 2 **shipped in
v0.50.0** (#632 to #636, behind Settings → General → Beta features, off by
default). The owner read v1 and v2 of [Ideas](idea-board.html) and decided to
build the idea board in phases, in the shape v1 first described: a place for
ideas inside worktrees, desktop only. This document is that build plan. It
overrules v2's "rehearse first, build no destination yet" (v2's own choices
C2(c) and C3(b)).

**What changed in revision 4 (2026-11-10, after the owner used v0.50.0):**

- Phases 0 to 2 are marked shipped (§8).
- Phase 1 gets a follow-up: **a first heading names the file.** The owner
  wrote an idea headed "# Jellyfin Sharing" and the file stayed
  `untitled.md`. A change to the first heading now renames the file, as Rename
  does. A lane has it (`feat/idea-heading-names-file`).
- Part of phase 5 is pulled forward: **the workspace's menu adds projects.**
  The owner: "I think we need a menu to be able to create/open/add/projects
  directly in this view. One thing I want to do is add/clone the jellyfin
  repo(s) as needed." The workspace's menu and its empty state get Add project
  ▸ New project…, Add existing…, Clone from URL…, each reusing the nav
  footer's dialogs and adding the result to that workspace. A lane has it
  (`feat/workspace-project-menu`).
- New, and next to design before anyone codes it: **idea sessions** (§7.3,
  phase S). The owner: "We need to be able to open up a corresponding chat
  session or sessions that go with that particular idea. The goal being that
  we can have that session tied to that idea and we can iterate on it.
  Eventually we can tell that idea to go ahead and make/clone things into
  workspaces as necessary. So the idea stays there and we can see where it
  started. The workspace is then the next phase." This settles C10 as (a), a
  session against the ideas folder in the chat view, but changes its shape: a
  **persistent conversation**, or several, tied to one idea, that can be
  reopened like a lane's chat. It goes before phase 3. Phase 6's agent help
  becomes a later use of the same session kind.
- New choices C12 to C15 (§10). On 2026-11-11 the owner answered C11 to C15
  "all (a)"; none is open now.

Phase numbers stay as they were, because shipped code cites them (for
example `ideas.rs` names "phase 3's `related`" and "phase 4's `plan_of`").
The new phase is called **phase S** and runs between 2 and 3.

**What changed in revision 3 (2026-11-08, more of the owner's answers):**

- Decided: C9 (AGENTS.md gets CodeMirror as a third named exception, in the
  phase 0 PR), C6 (today's nav is "All projects"; workspaces exist only once
  someone makes one), C8 (a workspace shows its projects, then every place
  of each).
- A place belongs to its project, never to a workspace. The record of which
  place a work item started moved from the workspace to the plan's work
  item (§2.2), and a place is dragged onto a work item, not onto a
  workspace (§6.1).
- Agent help stays last, and every earlier phase now says what it must leave
  in place for it ("For agent help later", §8).
- Still open: C10 and C11 (§10).

**What changed in revision 2 (2026-11-08, after the owner's answers):**

- Decided and now fixed: a blank document per idea; CodeMirror 6, with Bear's
  way of handling markdown as the north star (§4); documents in
  `~/.local/share/worktrees/ideas`; a plan is its own document made from one
  idea or several; desktop only.
- Each phase says what is thrown away if we stop there, as information.
- New: drag and drop throughout, with a drag grammar and a non-drag way for
  every drop (§6), and drags between the two windows if a phase 0 test shows
  they work (§6.3).
- New: ideas relate, merge, split and combine (§3), as their own phase
  straight after documents.
- New: agents help while you act (§7). Every act has a manual result first,
  and an optional agent proposal that you accept, edit or skip. The agent runs
  in a session of its own against the ideas folder, shown in the chat view.
- The phases are now seven (§8). The choices then open were C6, C8, C9, C10
  and C11.

Claims about the app are marked **checked** (read on `origin/main` at
`1f2064d` on 2026-11-08; for revision 4's new text, §7.3 and phase S, at
`d5e5e46` on 2026-11-10) or **assumed** (not measured). Claims about libraries,
Bear and Claude's CLI are marked **read** (npm registry, Bear's own pages,
2026-11-08; `claude --help` of Claude Code 2.1.322, 2026-11-10) or
**assumed**.

## 1. The answer

Build the chain the owner named, end to end, before anything richer:

> "turn an idea into a plan which can then be a workspace which can then be a
> set of projects, places to work on it."

| # | Phase | What David can do at the end | Size | State |
|---|---|---|---|---|
| 0 | Companion window | Open an Ideas window, full screen it, close it without quitting; try a drag between windows | S–M | shipped, v0.50.0 |
| 1 | Documents | Write ideas in a Bear-like editor: create, list, open, edit, rename, remove | M | shipped, v0.50.0; follow-up: the first heading names the file (lane) |
| 2 | Workspaces view | Make named workspaces and drag projects into them, in the main window | M | shipped, v0.50.0; follow-up: the workspace's project menu, from phase 5 (lane) |
| S | Idea sessions | Open a conversation (or several) tied to one idea, iterate in it, close it, and resume it later | M–L | next to design |
| 3 | Ideas relate | Relate, merge, split and combine ideas, by drag or menu, with nothing lost | M | |
| 4 | Ideas → plan → workspace | Make a plan from one idea or several, and a workspace from the plan | M | |
| 5 | Projects and places | Clone or create missing projects, and start places from the plan with a reviewed brief | M–L | the menu part pulled forward |
| 6 | Agent help | From an idea's conversation, the agent proposes a merge, a split, links, a plan draft or a workspace, and David accepts, edits or skips it | M | |

Later, and not in this plan: conversations on part of a document, the rest of
the Bear-like editor (§4.3), diagrams, quick capture, threads, MCP tools.

```
  shipped [0 window] [1 documents] [2 workspaces view]
  lanes   [1+ heading names the file]  [2+ workspace project menu (from 5)]
  next    [S idea sessions]──[3 relate]──[4 chain]──[5 places]
                    └────────────────────[6 agent help]  (after S and 3)
```

Phase S needs 1 (shipped). Phase 3 needs 1, and can run beside S. Phase 4
needs 2 and 3. Phase 5 needs 4. Phase 6 needs S and 3, and can run beside 4
and 5; each act it helps with must exist first.

Sizes: **S** is about one lane-day, **M** two to four, **L** about a week.
These are guesses.

## 2. The chain, made concrete

Four kinds of thing, all in plain files. None of them contains or owns a
project; that is v1's rule and it stays.

```
 ideas (documents) ──make plan──▶ plan (document) ──make workspace──▶ workspace
   ▲  relate / merge / split          │                                  │
   └──────────────┘                   │ work items                       │ references
                                      ▼                                  ▼
                               reviewed brief ──start──▶ place  ◀── project (registered)
```

### 2.1 An idea

One markdown file: the blank document the owner asked for. Nothing in it is
required except a first heading, which is its title.

```markdown
---
id: 01JC8Z4K3M9QX2T7VBN5R6WYHD
created: 2026-11-09
---

# Jellyfin on the home server

Free writing. Lists, links, code, whatever.
```

- `id` is a ULID written once, at creation, and never changed. Links between
  documents use the id, so a rename breaks nothing.
- The title is the first `# heading`, read from the text. No title in the
  frontmatter, so the two cannot disagree.
- The file name is a slug of the title. Rename changes the heading and the
  file name together.
- The frontmatter may also carry the relation fields of §3 (`related`,
  `absorbed`, `from`). The app writes those; David never has to.

### 2.2 A plan

A plan is **its own document** that comes out of an idea or a set of ideas
(the owner's words). It sits in the same folder and links back to a **list**
of ideas:

```markdown
---
id: 01JC90A1...
plan_of: [01JC8Z4K..., 01JC8Z9P...]
created: 2026-11-10
---

# Plan: Jellyfin on the home server

## Goal
One paragraph.

## Projects
- worktrees            (registered)
- homelab-ansible      (clone: git@github.com:david/homelab-ansible.git)
- jellyfin-config      (new: ~/workspace/jellyfin-config)

## Work
### Ansible role for Jellyfin
project: homelab-ansible
branch: jellyfin-role
brief: |
  Goal, deliverable, rules, when to stop.
started: homelab-ansible · jellyfin-role · 2026-11-12
```

- An idea can feed more than one plan; a plan can come from several ideas. An
  old plan stays as a record of what was tried.
- The app reads two sections (`## Projects`, `## Work`) and leaves the rest
  alone. If they do not parse, the plan view names the line and shows the
  document anyway, as the Plan tab does for `task_plan.md`.
- **Who writes it:** in phase 4, David, from a template the app fills with the
  ideas' titles and links. From phase 6, an agent can propose the draft (§7).
- **How it relates to a place's `.planning/`:** it sits above it. A place's
  `task_plan.md` stays that place's agent's working memory; they do not sync.
  The link is the brief: at start (phase 5) a work item's brief is **copied**
  into the new place's `.planning/brief.md`. After that it is the place's.
- **Which item started which place** is the item's `started:` line, which the
  app writes into the plan when a Start succeeds (phase 5): project, place,
  date. It is provenance kept in the document, as Bear keeps its metadata in
  the markdown (§4.1), so it survives a copy of the plan. If the place is
  later removed, the line stays and the item shows "place removed". The app
  writes it like any other act: history copy first, `expected_mtime` checked.

### 2.3 A workspace

A **named list of references to registered projects**, plus the plan it came
from and any ideas linked to it. It answers the owner's first ask
(2026-11-06): "a workspace with multiple projects? Where the workspace also has
context?" The context is the plan and its ideas.

Today there are no workspaces, and none is made for you. The owner: "We would
create them as needed, for example after that exists, we could create the
worktrees workspace which could have the `worktrees` and the `worktrees-ios`
projects." The Workspaces view starts empty.

```json
{
  "workspaces": [
    {
      "id": "01JC91...",
      "name": "Jellyfin",
      "plan": "01JC90A1...",
      "ideas": ["01JC8Z4K..."],
      "projects": [
        { "root": "/Users/davidpena/workspace/worktrees" },
        { "root": "/Users/davidpena/workspace/homelab-ansible" }
      ]
    }
  ]
}
```

- A project is referenced by its registry `root` (**checked**: a registry entry
  has `root` and `name`). A workspace never copies a project's name or
  settings.
- Removing a workspace removes the list. No project, place or branch is
  touched. A project removed from the registry stays referenced and shows as
  "not registered", with a way to drop it.
- A project can be in any number of workspaces.
- **A place belongs to its project, never to a workspace.** A workspace shows
  a project and then every place under it (C8, decided). The owner: "I don't
  know if there are places directly tied to a workspace though I'm not
  opposed. But it would be weird." So a workspace holds no place ids. What a
  plan started is recorded on the plan's work item (§2.2), which is v2's
  "commission" record, kept apart from the live place.

**What the existing word becomes** (C6, decided). In code, "workspace" means
every registered project on this host (**checked**: `list_workspace`,
`crates/worktrees-api/src/workspace.rs`). In the UI that is now the projects
view, labelled **All projects**, and it stays the default. "Workspace" in the
UI means only a named workspace someone made. Code names stay: renaming a row
the app, serve and the phone all call is churn with no benefit to David.

### 2.4 From a workspace to projects and places

| Kind of project in the plan | What phase 5 does |
|---|---|
| Registered | references it |
| `clone: <url>` | runs the existing clone flow (`clone_project`, **checked** in dispatch.rs), then references it |
| `new: <path>` | `git init` at the path through a new console row, registers it with `add_project` (**checked**), then references it |

Then each `## Work` item becomes a place: David opens the item and reads its
brief, shown as the exact text that will be written and editable there; Start
creates the place with that brief; the app writes the item's `started:` line.
Nothing starts without that review. A plan with five items does not start
five agents on one click.

**The app cannot start a place with a brief today.** The brief path exists in
the CLI (`worktrees new --brief`) and MCP `create_worktree`, but the app's serve
row `new_place` takes no brief (**checked**: `ops::new_place(repo, branch,
base, name, provider, model, force)`). Phase 5 adds a brief argument to it, or
a sibling row, calling the same core path as `new --brief`. Small, but new
code in a drive-class row.

## 3. Ideas relate to each other

The owner: "Ideas can be related, combined, merged, split and anything in
between." Four words, three acts. **Combine and merge are the same act** with
one difference: where the result lands.

| Act | What it does | Ids | Stored | Text |
|---|---|---|---|---|
| **Relate** | marks two ideas as connected; both stay as they are | both stay | `related: [id]` in the frontmatter of the idea it was made from; the other side's "related" list is computed, never written | untouched |
| **Merge** (B into A) | B's text is appended to A under `## From: <B's title> (date)`; B's file leaves the list | A survives; B's id goes into A's `absorbed: [B]` and keeps resolving to A | `absorbed` in A's frontmatter | all of B is in A, and B's last version is in history |
| **Combine** (A, B → new C) | a merge whose target is a new idea; A and B are merged into C | C is new; A and B go into C's `absorbed` | `absorbed` in C | all of A and B are in C; both originals are in history |
| **Split** (part of A → new C) | the selected text moves to a new idea C; in A, a link to C takes its place | A stays; C is new with `from: A` | `from` in C; the link is in A's text | the moved text is in C; A's previous version is in history |

"Anything in between":

- **Extract** is a split that leaves the original text in A (a copy, not a
  move). It is a checkbox on Split, not a fifth act.
- **A plan from several ideas** (§2.2) is not a merge. The ideas stay; the plan
  links to them in `plan_of`.
- **Unmerge** is not offered. History holds every version, so a merge that was
  a mistake is undone by restoring from history (a phase 3 menu item), not by a
  reverse operation.

**An id that goes away keeps working.** The store builds one index of every
document's `id` plus every `absorbed` id, and every lookup goes through it. A
plan, workspace or link that names B finds A, and shows "B was merged into A
on 12 Nov". Nothing that points at B breaks, and nothing has to be rewritten.

**No text is ever lost** (v1: "the original words are kept"). Before any merge,
combine or split, the app copies each file it is about to change into
`ideas/.history/<id>/<time>.md`. History is never pruned by the app in this
plan. It is plain files too, so Finder, Time Machine and `grep` all see it. A
git-backed ideas folder would do the same job and more, and is left for later.

Relations written in the text itself, as links (`[Jellyfin](idea:01JC8Z…)`),
count as relations too: the index reads them and lists them alongside
`related`. Typing `[[` offers a picker that inserts such a link by title (Bear
does the same with wiki links, **read**; Bear's links are by title, ours are by
id so that renames are free, **assumed** to be the better trade for us).

**Why this is phase 3, straight after documents:** relating and merging is
what you do with ideas before you plan from them, and a plan's `plan_of` list
and the agent's proposals (§7) both build on the same index and history.

## 4. The editor: CodeMirror 6, Bear as the north star

**Decided:** CodeMirror 6 editing the markdown text itself, in the companion's
own bundle. **North star:** the way Bear handles markdown. The owner: "I like
the way that bear.app works with markdown. I don't think we need that fully in
our first pass but that's our north star."

### 4.1 What Bear does (read)

From Bear's own pages (bear.app FAQ "How to use Bear's Editor", Bear's
community forum, and the Lettera announcement on blog.bear.app):

- Bear formats your text as you type markdown, and a **Hide Markdown** setting
  hides the markdown characters while keeping the formatting (**read**, FAQ).
- Bear 2's editor is a "hybrid WYSIWYG": when the cursor is on a heading line
  you see its markers; off that line the cue is only the styling (**read**, a
  Bear team reply on the forum). Bear's separate editor app, Lettera, built from
  the same editor, says "Markdown syntax hides when you are not editing"
  (**read**, Bear's blog).
- Tables, photos, todos and links between notes live in the same note; `[`
  links to another note (**read**, FAQ). Headings and lists fold (**read**).
- Bear keeps style metadata (image widths, link previews) **as HTML comments
  inside the markdown**, "to not defy one of the key advantages of MD:
  portability" (**read**, a Bear team reply on the forum). That is the same
  answer §4.4 gives for anchors, so the north star and our format agree.

### 4.2 How CodeMirror 6 gets there

CodeMirror keeps the text as it is and draws over it. A view plugin walks the
markdown syntax tree (`@lezer/markdown`, which `@codemirror/lang-markdown`
uses, **read**) and adds **decorations**: marks that style a range, and
replacements that hide a range or draw a widget in its place. Decorations are
view only. The file never changes because of them, so the round-trip promise
of v1 holds by construction (**assumed** from CodeMirror's design; the phase 1
check measures it).

The Bear behaviour is one rule on top: **skip the decorations that hide
markers on any line the cursor or selection touches.** Then you see `## ` when
you are editing that heading, and a large heading when you are not.

| Piece | How | Stage |
|---|---|---|
| Headings sized and weighted | mark decoration per heading level | 1 (phase 1) |
| Bold, italic, inline code, strikethrough styled | mark decorations | 1 |
| Markers (`#`, `**`, `` ` ``, `>`, list bullets) dimmed | mark decoration with a muted colour | 1 |
| Links styled, ⌘-click opens; `idea:` links open the idea | mark decoration plus a click handler | 1 |
| Markers **hidden** off the cursor's line | replace decorations, skipped on active lines | 2 |
| Checkboxes (`- [ ]`) drawn and clickable | widget; the click edits the two characters | 2 |
| Horizontal rules, quotes drawn | widgets and line decorations | 2 |
| Folding headings and lists | CodeMirror's fold service | 2 |
| Images inline | widget for `![]()` | 3 |
| Tables drawn as grids | block widget, back to source when the cursor enters | 3 |
| Mermaid fences drawn as diagrams | block widget reusing the viewer's `Mermaid.tsx` | 3 |
| A formatting bar (Bear's BIU) | buttons that insert markdown | 3 |

**Phase 1 ships stage 1 only**: everything is styled and markers are dimmed,
never hidden. It is the smallest part that already feels like writing rather
than coding, and it cannot hide anything from you. Stage 2 is the Bear feel,
and is a later lane in the companion bundle; stage 3 is the rich content.
Neither is in the phase order, because the chain does not need them; both
change only view code, never the file format.

### 4.3 Does the Source | Read switch survive?

Only until stage 3. In phase 1 the companion has two modes: **Edit** (the
CodeMirror view with stage 1 styling) and **Read** (rendered from the docs
viewer's source), because stage 1 cannot draw tables, images or diagrams. Once
stage 3 draws those in place, Read has nothing left to show and goes. What
stays is Bear's own switch: a **Show markdown** setting that turns off the
hiding from stage 2, for anyone who wants to see every marker.

### 4.4 What the format must allow now, for later

- **Conversations on part of a document** need a stable anchor. An anchor is
  written only when something attaches to that part, as an HTML comment at the
  end of the block, `<!-- ¶ 7k2q -->`: Bear's method for its own metadata.
  Renderers hide comments, and stage 2 hides them in the editor too. A document
  with no conversations has no anchors. If an anchor is deleted, the attachment
  falls back to the quoted text it was made on. Phase 1 writes no anchors; it
  keeps unknown comments and frontmatter exactly as they are, which CodeMirror
  does by construction.
- **Diagrams** are a ```` ```mermaid ```` fence, which the docs viewer already
  renders. Nothing new in the format.
- **Links between documents** are `[text](idea:<id>)`.

### 4.5 Its own bundle, like the viewer

The companion is its own Vite build, the way the docs viewer is
(`app/vite.viewer.config.ts`, **checked**). CodeMirror and, later, mermaid
enter that bundle and never `app/src`. A check in the shape of
`viewer-boundary-check.mjs` fails if `@codemirror/*` is reachable from the
app's import graph. The Read view reuses the viewer's rendering source
(`app/viewer/blocks.tsx`, `Mermaid.tsx`, **checked** they exist) as source
files, never a shared chunk.

One difference from the viewer, **assumed** and proved in phase 0: the viewer
is a page that `docserver` serves to a browser; the companion is a Tauri window
loading a page from the app's own `frontendDist`. So its build lands in
`app/dist/` as a second HTML page built by its own config, never a second input
to `vite.config.ts` (that would let rollup share chunks, which the viewer's
config exists to forbid).

Versions read on npm, 2026-11-08: `@codemirror/view` 6.43.15 and
`@codemirror/lang-markdown` 6.5.2, both MIT. The rich editors compared in the
first revision (ProseMirror, Tiptap, Lexical, Milkdown, BlockNote) all print
the markdown back out from their own model, which changes the file in places
nobody edited; BlockNote is also MPL-2.0 with a GPL AI package. That comparison
is settled by C2 and is not repeated here.

## 5. Where the files live

**Decided:** documents in `$XDG_DATA_HOME/worktrees/ideas/`, which is
`~/.local/share/worktrees/ideas/` by default. History in its `.history/`.
Idea conversations (their records and `start.md`) and, later, agent
proposals in its `.desk/` (§7.3, §7.4).

**Picked here:** workspaces in `$XDG_CONFIG_HOME/worktrees/workspaces.json`
(`~/.config/worktrees/workspaces.json`), beside `projects.json`, which they
reference. Both are read by the CLI and every engine, so one folder to back up
and one to isolate.

Notes that still apply:

- worktrees already keeps user data under `$XDG_DATA_HOME/worktrees`: profiles,
  the skill store, `agent/` (**checked**: `profile.rs`, `guidance.rs`).
- A later "move the folder" setting must warn about `~/Documents`, `~/Desktop`
  and `~/Downloads`: macOS asks a privacy prompt on first touch there, and a
  denial takes contents while leaving metadata (AGENTS.md).

**The stores need their own override, not `XDG_*`.** AGENTS.md says never give
a scratch Claude its own `XDG_DATA_HOME` (Claude keeps its versions there and
relinks `~/.local/bin/claude`), and phases 5 and 6 start Claudes in their
checks. Serve's `--config-dir` does not move the registry's folder either
(**checked**, AGENTS.md), so it would not move `workspaces.json`. So one
variable, for example `WORKTREES_USER_DIR`, moves both stores. The name is the
phase 1 lane's.

**What a test must do so it never writes the user's real files:**

- Every store function has an `_at(path)` form; unit tests use only those, with
  a temp dir.
- An in-process engine under test runs a guard like
  `registry::forbid_real_writes()` (**checked**: it exists for
  `projects.json`), extended to the ideas folder and `workspaces.json`.
- A test that spawns the binary gives the child its own `HOME`,
  `XDG_CONFIG_HOME` and the override, per child, never `set_var` in the test
  process.
- The guard is shown red once under a temp `HOME`, as AGENTS.md says for the
  registry's.

## 6. Drag and drop

The owner: "For all this we want to heavily use drag and drop. We should be
able to drag projects to workspaces and we should be able to make
relationships where it makes sense."

### 6.1 The drag grammar

Every drop has a **non-drag way** to do the same act: a menu item, a
keyboard path, or both. That is for the keyboard, and because checks cannot
drive a real drag: the mock harness's synthetic pointer drag bypasses
hit-testing on the way in, and Playwright cannot drive a WKWebView
`dataTransfer` (AGENTS.md and `navdrag.ts`'s header, **checked**). Checks run
the non-drag path; the drag itself is checked by hand in the real app.

| Drag | Onto | Means | Non-drag way | Phase |
|---|---|---|---|---|
| a project (nav row) | a workspace (nav group) | add the project to the workspace | project menu → Add to workspace ▸ | 2 |
| a project in a workspace | another workspace | add it there too (a project can be in many) | same menu | 2 |
| a workspace | between workspaces | reorder | workspace menu → Move up/down | 2 |
| a project in a workspace | the All projects switch | remove it from the workspace (never from the registry) | project menu → Remove from workspace | 2 |
| an idea (list row) | another idea | a small chooser: **Relate** (default, Enter) or **Merge into this** | idea menu → Relate to… / Merge into… | 3 |
| two or more selected ideas | the "+ New idea" button | combine into a new idea | multi-select → Combine | 3 |
| selected text in the editor | the idea list | split it into a new idea (⌥ to extract a copy) | selection menu → Split into new idea | 3 |
| an idea | a plan (list row or plan strip) | add the idea to the plan's `plan_of` | plan strip → Add idea… | 4 |
| an idea | a workspace | link the idea to the workspace's context (`ideas`) | idea menu → Link to workspace ▸ | 4 |
| a plan | a workspace with no plan | set the workspace's plan | plan strip → Make workspace / Link ▸ | 4 |
| a project | a plan | add it to the plan's `## Projects` as registered | plan strip → Add project… | 4 |
| a place (nav row) | a work item in the workspace overview's Work list (same window) or in the plan (companion) | record that existing place as the item's place (its `started:` line, no new place) | work item menu → Link a place… | 5 |
| a `.md` file from Finder | the idea list | import it as a new idea (a copy) | ⋯ → Import… | later |

Considered and left out: dragging a place onto a workspace (a place belongs
to its project, and the workspace already shows every place of its
projects), dragging a work item onto a project (the item's
`project:` line is one word to type), and dragging an idea onto a project
(the relation is idea → plan → project, not idea → project; a shortcut would
make two ways to say the same thing).

Drop rules that hold everywhere:

- A drop that removes or merges anything asks first, or offers Undo. Relate and
  add never ask.
- A drop onto something that cannot take it shows no target highlight and
  does nothing. No error toast for a drag that went nowhere.
- Escape cancels a drag, as `navdrag.ts` already does (**checked**).

### 6.2 How, inside one window

Inside a window, reuse the app's existing pointer-event drags. `navdrag.ts`
is deliberately not HTML5 drag-and-drop, for reasons its header lists
(**checked**): it works with `dragDropEnabled: false`, pointer events can be
driven by the harness, and a per-frame `elementFromPoint` hit test does not
chase the gap it opens. Its `DragItem` already has a `project` kind keyed by
`root` (**checked**), which is exactly what phase 2 needs. The companion gets
the same plumbing for ideas.

### 6.3 Across the two windows

The owner: "I don't know if we can drag and drop from one window to another
but would be interesting if possible."

**What is known.** A pointer-event drag cannot leave its window: the pointer
events belong to the page that started it. A drag between windows needs the
HTML5 drag-and-drop API, whose data the system carries between web views.
`tauri.conf.json` sets `dragDropEnabled: false` on main (**checked**), which
is what leaves HTML5 drops to the page instead of Tauri's file-drop handler;
the companion would set the same. `dataTransfer` appears in the app today only
in `ChatPane.tsx` (files dropped into the chat) and `navdrag.ts`'s comment
(**checked**). **Whether a drag started in one Tauri window drops in another,
on macOS and on Linux, is not measured.** (main) is putting that measurement
into the phase 0 lane as a hand test for David (§8, phase 0).

**If it works**, these drags cross windows, carrying a small typed payload
(for example a custom type `application/x-worktrees` with `{kind, id}` or
`{kind, root}`):

- a project from the main nav onto a plan in the companion (add to Projects);
- an idea or a plan from the companion onto a workspace in the main window
  (link it; set the plan);
- a place from the main nav onto a plan's work item in the companion (record
  it as that item's place).

Only those sources use HTML5 drags, and only for the cross-window case: the
same row starts a pointer drag inside its own window. That keeps
`navdrag.ts`'s reasons intact where they apply.

**If it does not work**, nothing in the grammar is lost; each cross-window
drag has a same-window form instead:

- The companion's plan strip has its own small project picker, so a project
  is dragged within the companion.
- The main window's workspace overview lists the workspace's plan and ideas,
  so an idea or plan is dragged onto a workspace within the main window.
- And every one has its menu ("Add to workspace ▸", "Add project…").

## 7. Agents help while you act

The owner: "Ideally we would get help from agents as we perform actions in
that view." He said ideally, so this is the last phase. The seam is designed
now, so the earlier phases do not have to change for it.

### 7.1 The seam: manual first, a proposal second

Every act in this plan is an app operation with parameters: merge(A, B),
split(A, range), relate(A, B), make_plan(ideas, draft text). The manual path
collects those parameters from you, by drag or menu, and runs the operation.
The agent path produces the **same parameters as a proposal**; you accept,
edit or skip it; and accepting runs the very same operation. So:

| Act | Manual result (phases 3–4) | Agent proposal (phase 6) |
|---|---|---|
| Relate | you pick the other idea | "these three ideas look related", with one line each on why |
| Merge / combine | you pick the ideas; text is appended under headings | a merged draft that reads as one text, shown as a diff against the plain append |
| Split | you select the text | "this reads like two ideas", with the cut marked |
| Make a plan | a template with the ideas' titles | a drafted Goal, Projects and Work, with briefs |

v1 §6's lines stand, word for word where they fit:

- **No model runs without a click.** Each proposal is asked for with a button
  ("Suggest…") next to the manual act. Nothing runs in the background.
- **An agent proposes; the person decides.** A proposal changes nothing until
  you accept it. Accepting is the same operation, with the same history copy.
- **A dismissed suggestion stays dismissed.**

### 7.2 Where the agent runs (v1's Q2): decided, a session

There is no worktree for an idea. Revision 3 weighed three shapes:

| | One-shot headless call per proposal | **A session against the ideas folder, in the chat view** | A place in a hidden `ideas` project |
|---|---|---|---|
| What it is | `claude -p` (or another harness's headless mode) started per request, its output parsed | a Claude in its own tmux session with the ideas folder as its working directory, shown in the companion through the existing chat view | the ideas folder made a git repo, registered as a project, one place per conversation |
| You can see and stop it | no; it runs and returns | yes; it is a session like a lane's | yes |
| Follow-up ("shorter", "keep the second half") | a new call with everything re-sent | type in the chat | type in the chat |
| New engine work | a parser for its output | a session kind that is not a place: launch, dots, chat view keyed by it | almost none |
| Cost to the rest of the app | none | small | a project in every list, a branch per conversation (the "not a branch" problem v1 began with), and the ideas folder becomes a repo |

**Decided (C10, 2026-11-10): the session against the ideas folder.** The
owner's words settle it: "open up a corresponding chat session or sessions
that go with that particular idea … tied to that idea and we can iterate on
it." They also change its shape. Revision 3 had one short session per request,
ending when its proposal was accepted or skipped. The owner wants a
**conversation that lasts**: tied to one idea, iterated in, closed, and
reopened later, like a lane's chat. §7.3 designs that. Revision 3's request and
proposal strip is not thrown away: it becomes phase 6, a later use of the same
conversation (§7.4).

**Its reach is a route rule, not a sandbox** (v2's lesson). The agent can
technically write any file in the ideas folder. The app never applies a
proposal without your accept; a file changed outside the app is noticed by the
store's change event (`ideas:changed`, shipped in phase 1); and history makes
any stray write recoverable.

### 7.3 Idea sessions

An **idea session** is one conversation with Claude about one idea. It runs in
its own tmux session with the ideas folder as its working directory, and the
companion shows it in the chat view. An idea can have several. The idea's
document stays the record of where the idea started; the conversations sit
beside it, never inside it.

```
 ideas/                                   (~/.local/share/worktrees/ideas, or WORKTREES_IDEAS_DIR)
   jellyfin-sharing.md                    the idea, id 01JC8Z…; its text is never rewritten by this
   .history/01JC8Z…/<time>.md             a copy taken when each conversation starts
   .desk/01JC8Z…/sessions.json            the idea's conversations (the record, below)
   .desk/01JC8Z…/<conv>/start.md          what the conversation was opened on (fixed template)

 tmux: worktrees~idea~<conv>              one session per live conversation
 ~/.claude/projects/<ideas folder, mangled>/<session id>.jsonl   its transcript (Claude's own)
```

#### Where it runs, and what it opens on

- **Working directory: the ideas folder,** as §7.2 picked. One folder for every
  conversation means one folder-trust prompt, once, and one Claude project
  directory for every transcript.
- **The opener is fixed text naming a file**, the way `BRIEF_OPENER` ("Read
  .planning/brief.md and begin.") names the brief (**checked**,
  `ops.rs:805`). Here it is `Read .desk/<idea>/<conv>/start.md and begin.`,
  where both names are ULIDs the engine made (`crate::ulid`, **checked**:
  the store's ids are ULIDs) and are checked as ULIDs before they reach argv.
  It carries no text of the idea and nothing the user typed. Nothing else
  reaches Claude through argv, and nothing is typed into it to start it.
- **`start.md` is a fixed template the engine fills with ids and a file name**:
  this conversation is about the idea whose frontmatter `id` is `<id>`, now in
  `<file>.md`; read it; talk it through with David; change that file only when
  he asks; never write elsewhere in the folder except `.desk/<idea>/<conv>/`.
  It names the id as well as the file because a heading change now renames the
  file (phase 1's follow-up), so the name in `start.md` can go stale during a
  conversation, and the id cannot: "if the file is not there, find the `.md`
  whose frontmatter says `id: <id>`."
- **No `CLAUDE.md` in the ideas folder** (C13). `start.md` carries the
  conversation's instructions, and a resumed conversation already has them.
  A `CLAUDE.md` at the folder's root would also be listed as an idea: the
  store reads every `.md` there not starting with `.` (**checked**,
  `ideas.rs:404`).
- **worktrees' planning hooks are off, by construction.** They reach a Claude
  only through `ops::ai_launch_for`, which takes a `Project` and adds the plan
  plugin only when that project's planning level is Full (**checked**,
  `ops.rs:84`, `planning_for`). An idea session has no project, so it gets a
  launcher of its own, built from the same parts minus the project ones:
  `--name`, the model, and the prompt plugin (`prompt_hook_for`, which keys on
  the harness alone, **checked**), because the chat reads a tall permission
  prompt through that plugin's record. No guidance plugin (it is for managed
  repos), no planning plugin.
- **This is the user's real Claude**, launched as a lane's is, with their own
  settings, global hooks and plugins. `DISABLE_AUTOUPDATER=1` is for the
  scratch Claudes of a sandbox check only (AGENTS.md); it is never set here.
- **The folder-trust prompt** comes on the first conversation ever. Claude
  records the answer in `~/.claude.json`, which worktrees reads and never
  writes (AGENTS.md), so the user answers it. Before any probe file exists, a
  waiting pane is seen by its screen (`watch::claude_startup_tick`,
  **checked**). Whether the chat's prompt reader answers the trust prompt is
  **assumed** not; the first slice gives the panel an "Open in terminal"
  fallback (the shim's terminal row, as the main window has) for any screen
  the chat cannot answer.
- **The user-scope worktrees MCP server** starts in this session too, and in a
  folder that is not a repository it handshakes and offers zero tools
  (**checked**, AGENTS.md, `test/mcp.bats`). So the agent has no worktrees
  tools, as decided (desktop only; §7.4 for later).

#### How a conversation is tied to its idea

One record per idea, `ideas/.desk/<idea id>/sessions.json`, written with
`fsx::write_atomic`. The store skips every name starting with `.`
(**checked**, `ideas.rs:404`), so `.desk/` is never listed as an idea. The
idea's own frontmatter gets nothing: a conversation is not part of the text.

```jsonc
{
  "conversations": [
    {
      "id": "01JCB2…",                         // the conversation's ULID: names start.md's folder and the tmux session
      "harness": "claude",
      "session_id": "5f0c…-…",                 // Claude's own id, chosen by us, written BEFORE launch
      "title": "Who can see what",             // editable; defaults to "Conversation 1", "2", …
      "started": "2026-11-10T14:02:11Z",
      "started_from": ".history/01JC8Z…/20261110T140211Z.md"
    }
  ]
}
```

- **The session id is chosen before launch** with Claude's `--session-id
  <uuid>` (**read**: `claude --help`, 2.1.322) and written into the record
  first, as a place's launch stamp is written before `new-session`
  (AGENTS.md). Then the transcript's path is known without a probe:
  `claude_dir_in(<claude root>, <ideas folder>)/<session id>.jsonl`, the same
  join the chat uses today (**checked**, `agents.rs` `transcript`,
  `project.rs:838`).
- **"We can see where it started":** starting a conversation first copies the
  idea into `.history/` (the store's own history rule, **checked**: rename and
  remove already copy there), and `started_from` names that copy. The
  conversation list shows it as "started from this version".
- **Live state** comes from the engine's existing readers: Claude's probe
  files, found by pane (`agent::locate`, `ClaudeProbe::session()`, **checked**:
  `agent.rs:118`), matched by the tmux session name, never by `cwd`, which
  every conversation shares.
- **A conversation that has ended** has no tmux session and no probe, and its
  transcript is still on disk. The chat reads it by session id and shows it
  read-only, with Resume. This is new: today a lane's chat shows "no agent is
  running in this place" once its session ends, because `agent_turns` reads
  only a live lane (**checked**, `agents.rs:1378`, `chat.ts:191`). A
  transcript Claude has since cleaned up is shown as "this conversation's
  transcript is no longer on disk" (**assumed**: Claude removes old
  transcripts after its cleanup period).

#### One idea, several conversations

- The chat panel has a header: the conversation's title, and a menu with the
  idea's conversations, newest first, each with its state (running, waiting,
  ended) and its start date, plus **New conversation**.
- **New conversation** writes the record and `start.md`, takes the history
  copy, and launches.
- **Close** ends the tmux session (`kill-session`, through `tmux::tmux()`).
  Claude's transcript stays. The record stays.
- **Resume** launches `claude --resume '<session id>'` in the ideas folder,
  under the same tmux name. The launch args already carry a UUID resume after
  `--name` (**checked**, `harness.rs:385`). The opener is NOT sent again: a
  resumed conversation has it.
- **Remove a conversation** (in the menu, asks first) deletes its record entry
  and its `<conv>/` folder, and leaves Claude's transcript, which is Claude's
  file.
- Removing the idea (phase 1's remove) closes its live conversations first and
  moves `.desk/<id>/` along with the file. Merging B into A (phase 3) lists B's
  conversations under A, marked "from B"; the id index already resolves B to A.

#### The tmux session, and why nothing mistakes it for a place

- **Name: `worktrees~idea~<conv>`** (C12). A place's session is
  `<prefix>-<slug>` (**checked**, `project.rs:755`), its prefix is only
  `[a-z0-9_-]` (**checked**, `config.rs` `sanitize_prefix`), and `~` cannot
  appear in a place's session; the dock's shell sidecars rely on that already
  (`<session>~term`, **checked**, `tmux.rs:21` and `:45`). So the name can
  never be a place's. A ULID starts with a digit, so `~term` cannot appear in
  it either.
- **`Project::live_session_in` cannot adopt it.** Adoption is by pane cwd under
  a place's directory, and the ideas folder is under no registered project
  (**assumed** for an override: `WORKTREES_IDEAS_DIR` should refuse a folder
  under a registered project, so this stays true).
- **`--name worktrees~idea~<conv>`** as for every pane-0 Claude worktrees
  launches, because the chat's prompt reader is proved on named Claudes (the
  name row under every modal, AGENTS.md). It gives the conversation an address
  for Claude's own messaging; the reach is not new, since local Claude sessions
  are listed to each other anyway (**assumed**).
- **Dots.** The poll's `claude_activity` buckets probes into busy and waiting
  `cwd`s for `sessions:busy` (**checked**, `watch.rs:116`). An idea session
  lands there as the ideas folder's path, which matches no place, so the main
  window's nav shows nothing for it, correctly. But every conversation shares
  that path, so it cannot tell them apart. Phase S adds a second bucket in the
  same loop, keyed by tmux session name for names carrying `~idea~`, pushed as
  its own change-gated event (`ideas:sessions`, declared in `scope::EVENTS`
  like the rest). The companion paints a dot per conversation and per idea in
  its list; the main window's lightbulb gets the amber dot when any
  conversation waits on David, and nothing for busy.
- **Host policy's live-agent limit** (`[limits] max_live_agents`, **checked**:
  `policy_presets.rs:45`) counts idea sessions, since each is a Claude
  spending the same quota (C15, decided), and a start past it is refused with
  the same words as a lane's. Whether today's counter already sees a session
  that is not a place is **assumed** not, so phase S teaches it.

#### The rows: new ones, keyed by the idea

The chat view is driven by `agent_turns`, `agent_prompt`, `agent_send`,
`agent_answer*`, `agent_interrupt` and friends, every one keyed by `(repo,
slug)` and declared `Engine [cap, place(repo, slug)]` (**checked**,
`dispatch.rs:1113`–`1171`). An idea session has no repo and no slug. Two
shapes:

- **Reuse the rows with a synthetic key** (say `repo: "~ideas"`). That is a
  lie the resolver would have to tell: `Obj::Place` resolves a registered root
  and stats a directory under it (**checked**, `resolve.rs:63`), so a fake
  project needs a special case there. And those rows are `Engine` rows, which
  a paired iPhone reaches with the right grant. Ideas are desktop only
  (decided), so a synthetic key would hand idea sessions to the phone.
- **New rows, `HostConsole`, keyed by the idea** (recommended). A `HostConsole`
  row names objects and no capability, and no device grant reaches it
  (**checked**, `dispatch.rs` header and `Row::new`). Two new object kinds,
  each a true statement the resolver can check: `idea(id)`, an id the engine's
  index knows, and `idea_conv(conv, id)`, a conversation in that idea's
  record. Names for size: `idea_conversations`, `idea_conversation_start`,
  `idea_conversation_resume`, `idea_conversation_close`,
  `idea_conversation_remove`, `idea_turns`, `idea_turn_text`,
  `idea_tool_call`, `idea_prompt`, `idea_answer`, `idea_answer_text`,
  `idea_answer_many`, `idea_send`, `idea_interrupt`.

The bodies are not copied. `agents.rs` puts every screen and transcript read
behind `AgentIo`, whose methods take `(project, slug, lane)` (**checked**,
`agents.rs:198`); most of them use only the `Lane` (session, pane, harness).
The refactor gives the bodies a `Lane` and a transcript locator, with two thin
entries: the place rows find the lane from `(repo, slug)` as now; the idea rows
find it from the record (the tmux name, and the transcript path from the
session id). The place rows keep the place lock (`agents::place_lock`,
**checked**: it exists); the idea rows take one lock per conversation, which
does for a conversation what the place lock does for a place: two clients
cannot answer one prompt twice.

**Typing into an idea session is `agent_send`'s path, reaching one more kind of
session** (C14). AGENTS.md has exactly three ways text reaches a Claude
(`agent_send`, `agent_answer_text`, `agent_command`) and says not to grow a
fourth. `idea_send` and `idea_answer_text` are new rows over the same bodies:
the same `typed_line_ok` rule, the same paste, the same empty-composer and
no-prompt checks, length and hash in the audit and never the text. That is the
same path with a second door, console only. David said yes to it on
2026-11-11 (C14); the AGENTS.md paragraph on the three paths names this door
in the phase S PR, with his approval. No slash commands from the idea chat in
phase S.

#### The companion's screen

The chat panel opens on the right of the editor, in the room phase 0 left for
it (§9.1). The idea's list gets a dot per idea with a live conversation. The
panel is the main window's `ChatPane`; the companion may import from
`app/src` (**checked**: `Companion.tsx` imports `../src/ipc`,
`../src/markdown`), and its boundary check guards only the other direction.
`ChatPane`'s target today is `{repo, slug, path, session}` (**checked**,
`chat.ts:1146`), so it learns a second kind of target whose calls go to the
`idea_*` rows. The main window shows nothing new except the lightbulb's dot.

When the agent edits the idea's file, the editor gets `ideas:changed`; if
David has unsaved typing, the existing "the file changed under you" handling
applies, as for an edit in another editor (phase 1, `expected_mtime`).

### 7.4 The seam for "make and clone"

"Eventually we can tell that idea to go ahead and make/clone things into
workspaces as necessary." Not in phase S. What it would need, so phase S does
not close it off:

- **The agent proposes, David accepts** (§7.1). The conversation writes a
  proposal file into its own folder, `.desk/<idea>/<conv>/proposals/<n>.md`,
  whose frontmatter names one act and its parameters: `make_plan(ideas,
  draft)`, `workspace_create(name)`, `clone_project(url, path)`,
  `create_project(path)`, `workspace_set_projects(id, roots)`. The companion
  shows it in the chat with Accept, Edit, Skip. Accept runs the existing
  console row with those parameters, so nothing new can act; the agent only
  fills in parameters.
- **No MCP tools for it, first.** The ideas folder's MCP server offers zero
  tools today, by decision. Proposal files need none: the agent writes a file
  and the engine reads it. MCP tools that only write proposals
  (`propose_workspace`, …) would give structured input and need a mode of the
  MCP server for this one folder; that is a later choice with its own consent.
- **Each act keeps its own guard.** `clone_project` is a drive-class console
  act that warns on `~/Documents` and the like (phase 5); none of that moves
  into the agent.
- **What phase S leaves for it:** the conversation's own folder, the record,
  and the chat panel. Phase 6 adds the proposal reader and the strip.

## 8. The phases

Every phase ends with a check in the real app, by hand, through
`app/scripts/sandbox.sh --app`, because nothing in CI runs the app
(**checked**, AGENTS.md). Each check runs with the stores' override (§5)
pointing at a scratch folder and its own tmux server (`env -u TMUX
TMUX_TMPDIR=<short dir>`). Never David's running app.

### Phase 0: the companion window

**Shipped in v0.50.0** (#632; the beta switch, #636).

**David can:** open an Ideas window from the main window (a button and a
shortcut), maximise it or make it full screen on its own Space, close it, and
reopen it where it was. Closing it does not quit the app. It shows "No ideas
yet" and is connected to the engine.

**Built:**

- A second window, label `companion`, created on demand by the shim. Today
  there is one window, `main`, declared in `tauri.conf.json` without a label
  (Tauri's default is `main`), and no code creates another (**checked**: no
  `WebviewWindowBuilder` under `app/src-tauri`).
- Its own capability file with only what it needs. `capabilities/default.json`
  names only `main` (**checked**).
- Frame memory: `winstate.rs` restores only `main` (**checked**, line 282) and
  learns a second label.
- The page: `app/companion/`, its own entry and Vite config, built into
  `app/dist/` as a second page (§4.5), and a boundary check.
- The engine door: one more loopback client of serve, through `engine_connect`
  and its own socket, as `main` does (**checked**). It **never starts serve**;
  its reconnect policy runs reconnect-only. `main` owns starting, so two windows
  never both restart a lost engine. `reconnect.ts` is pure and injected
  (**checked**), so this is a parameter, not a fork of the rule.
- The places that say "main" today and must say which window they mean:
  `raise_window` (`lib.rs:258`), the reopen at `lib.rs:451`, `feedback.ts:123`
  (all **checked**).
- `dragDropEnabled: false` on the companion too, and a throwaway pair of drag
  source and drop target, one in each window, for the hand test below.

**Left out on purpose:** documents, the editor, any store.

**For agent help later:** the window's layout leaves room for a side panel
(the chat panel, phase S), and the page's engine door is the one any later panel
uses; no second socket.

**Thrown away if we stop here:** the window, its capability, its bundle, the
label in winstate. Small.

**Checked in the real app:** open, full screen, exit, close, reopen at the same
frame; ⌘Q with both open and relaunch; kill serve by pid once and see the
companion wait while `main` restarts it (one restart in serve.log, not two);
hide `main` and confirm the companion still reconnects. Space the kills a
minute apart (three starts in a minute is a crash loop by `reconnect.ts`'s
rule). **Hand test for David: the cross-window drag.** Drag the test source
from the main window onto the target in the companion and back, on macOS, and
on Linux if a Linux build is to hand. Record whether the drop arrives and what
`dataTransfer.types` it carries. The result decides §6.3.

**Size:** S–M.

### Phase 1: documents

**Shipped in v0.50.0** (#634 the store and rows, #635 the list and editor).

**Follow-up, with a lane (`feat/idea-heading-names-file`): a first heading
names the file.** Phase 1 shipped the rule that typing a heading changes
neither the name nor anything else, and only Rename renames (**checked**,
`ideas.rs` header). The owner found that wrong in use: an idea headed "#
Jellyfin Sharing" stayed `untitled.md`. Now a change to the first heading
renames the file as Rename does, with its history copy. Two consequences
for later phases: links are by id (§3), so they do not care; and an idea
session's `start.md` names the id as well as the file (§7.3).

**David can:** create an idea, see the list, open one, write in it with
headings sized and markers dimmed (stage 1), switch to Read, rename it, remove
it. It saves on its own. Finder shows plain `.md` files he can also edit
elsewhere.

**Built:**

- A core store, `worktrees_core::ideas`: the folder of §5, one file per
  document, the frontmatter of §2.1, `_at` forms, the test guard, atomic writes
  through `fsx::write_atomic` (AGENTS.md: never hand-roll a temp name).
- Console-only serve rows: `ideas_list`, `idea_read`, `idea_create`,
  `idea_write` (with `expected_mtime`, as `write_file` has, **checked**:
  `files.rs:1390`), `idea_rename`, `idea_remove`. Names for size; the lane
  names them for real.
- A change event (`ideas:changed`) so an edit made elsewhere shows up, with the
  dock editor's "the file changed under you" handling.
- The companion's list and editor (§9's sketch): CodeMirror 6, markdown
  language, stage 1 decorations, the Read view from the viewer's source.
- Remove moves the file to the system Trash (**assumed** possible from serve;
  if not, to `.history/`).

**Left out on purpose:** relations, plans, links' `[[` picker (phase 3),
hidden markers (stage 2), anchors, search beyond a title filter, tabs.

**For agent help later:** every change the app makes, as opposed to your
typing, is a named operation with parameters (`idea_create(title)`,
`idea_rename(id, title)`, `idea_remove(id)`), so a proposal can later carry
the same parameters; and the history rule starts here: rename and remove copy
the file into `.history/` first.

**Thrown away if we stop here:** the bundle and the rows. The `.md` files stay
and are useful without the app.

**Checked in the real app:** create three ideas, edit, rename (file name and
heading change, `id` does not), remove one; edit a file in another editor while
it is open and see the app notice; save with no change and confirm the bytes
and mtime did not move (`shasum` before and after); a document with
frontmatter, an HTML comment, a table and a mermaid fence survives an edit
elsewhere in it byte for byte; type in CodeMirror in the app's WKWebView with
IME input (accents, an emoji) and undo.

**Size:** M.

### Phase 2: the workspaces view (parallel with 1)

**Shipped in v0.50.0** (#633).

**Follow-up, with a lane (`feat/workspace-project-menu`), pulled forward from
phase 5.** The empty state said to drag projects in or add them, and the
owner wants to do it from the view: "I think we need a menu to be able to
create/open/add/projects directly in this view. One thing I want to do is
add/clone the jellyfin repo(s) as needed." The workspace's menu and its empty
state get **Add project ▸ New project…, Add existing…, Clone from URL…**, each
reusing the nav footer's existing dialog and adding the result to that
workspace (`workspace_set_projects`). This is phase 5's "clone or create a
missing project" without the plan: phase 5 later calls the same menu from a
plan's "to clone" and "to create" rows.

**David can:** switch the main window's nav to Workspaces (empty at first),
make a workspace (say `worktrees`, holding `worktrees` and `worktrees-ios`),
drag projects into it (or use the menu), reorder workspaces, see each
project's places under it, and switch back to All projects with nothing
changed.

**Built:**

- A core store, `worktrees_core::workspaces`: `workspaces.json` of §2.3, `_at`
  forms, the guard.
- Console-only rows: `workspaces_list`, `workspace_create`,
  `workspace_rename`, `workspace_remove`, `workspace_set_projects`,
  `workspace_reorder`.
- The switch and the grouped nav (§9's sketch), from the same place rows as
  today's nav; the right side unchanged.
- The phase 2 drags of §6.1 on `navdrag.ts`'s plumbing, each with its menu.
- "All projects" in the UI wherever "workspace" means every project (C6): a
  sweep of user-facing strings, not code names.

**Left out on purpose:** plans, ideas, cloning or creating.

**For agent help later:** each workspace act is a named operation with
parameters (`workspace_create(name)`, `workspace_set_projects(id, roots)`,
and the rest), so "these three projects belong together" can later be a
proposal of the same call. `workspaces.json` keeps its previous version
before every change (one `.prev` copy is enough here; there is no prose to
lose).

**Thrown away if we stop here:** the switch, the grouped nav, the store.
All projects is today's nav.

**Checked in the real app:** two workspaces sharing a project; select a place
through a workspace and through All projects (the same terminal); drag a
project onto a workspace and out again; remove a workspace and diff
`projects.json` before and after (no change); unregister a project a workspace
references and see "not registered".

**Size:** M.

### Phase S: idea sessions (after 1, before 3)

**David can:** open an idea and press **New conversation**; the chat panel
opens beside the editor and Claude starts on the idea; he talks it through,
approves Claude's edit to the idea's file from the chat, and sees the editor
update; starts a second conversation on the same idea; closes one; closes the
Ideas window and the app, comes back the next day, sees both conversations in
the idea's list (one ended, one still running in tmux), reads the ended one,
and resumes it. The main window gains nothing but an amber dot on the
lightbulb while a conversation waits on him.

**Built, first slice (M–L):**

- The record and the folder of §7.3: `.desk/<id>/sessions.json`, `start.md`
  from its fixed template, the history copy on start, ULID checks on every id
  that reaches argv.
- A launcher for a session that is not a place: tmux `worktrees~idea~<conv>`,
  cwd the ideas folder, `--name`, `--session-id` chosen first, the prompt
  plugin, no guidance and no planning plugin, the fixed opener. Resume with
  `--resume '<session id>'` and no opener.
- The rows of §7.3, `HostConsole`, on two new object kinds, `idea(id)` and
  `idea_conv(conv, id)`, in `resolve.rs`. The agent bodies refactored to take a
  `Lane` and a transcript locator, so place rows and idea rows share them.
- A transcript read by session id that works after the session has ended.
- The `ideas:sessions` event from the poll's Claude loop, keyed by session
  name; the dots in the companion; the lightbulb's amber dot.
- The chat panel in the companion: `ChatPane` with an idea target, the
  conversation header and menu (New, Close, Resume, Rename, Remove), and "Open
  in terminal" for any screen the chat cannot answer (the trust prompt first).
- Single-select prompts and free-text answers (`idea_prompt`, `idea_answer`,
  `idea_answer_text`) in the first slice, because every edit Claude makes to
  the idea asks permission first; without them the first edit is a trip to the
  terminal. Multi-select (`idea_answer_many`) and `idea_tool_call` can follow.

**Left out on purpose:** harnesses other than Claude (Codex's rollout is found
by cwd, which every conversation would share, **checked**: `agents.rs`
`transcript` uses `codex::latest_rollout(place_dir)`); slash commands from the
idea chat; any act beyond editing the idea's own file; proposals (phase 6);
a conversation about several ideas at once (phase 4's plan is the place for
that; until then, one idea per conversation); anything on a paired device.

**For agent help later:** each conversation has its own folder
(`.desk/<idea>/<conv>/`) where phase 6's proposal files go; the chat panel is
where the Accept, Edit, Skip strip will sit; and the record is per idea, so a
"Suggest…" button can start a conversation or reuse the newest live one.

**Thrown away if we stop here:** the launcher, the rows, the event, the chat
panel in the companion, and the agents refactor (which stays useful: it is
only a cleaner seam). The ideas, their history copies, the records and
Claude's transcripts stay on disk, and each conversation can still be resumed
by hand with `claude --resume <id>` in the ideas folder.

**Checked in the real app:** through `sandbox.sh --app` with
`WORKTREES_IDEAS_DIR` on a scratch folder, its own tmux server (`env -u TMUX
TMUX_TMPDIR=<short dir>`), and the scratch Claude under `DISABLE_AUTOUPDATER=1`
with the real `XDG_DATA_HOME` (AGENTS.md; the user's own Claude never gets
that variable). Then:

- the first conversation meets the trust prompt, the lightbulb and the
  conversation go amber, and "Open in terminal" answers it;
- Claude reads the idea from `start.md` alone (look at its first turn), and
  `ps` shows no idea text in its argv;
- ask for an edit, approve it in the chat, and see the editor update; type in
  the editor while Claude edits and see the "changed under you" handling;
- rename the idea's heading mid-conversation (phase 1's follow-up), then ask
  Claude to edit again: it finds the file by id;
- two conversations on one idea, both busy, two separate dots;
- close one: its tmux session is gone, its transcript reads in the panel;
  resume it: the same session id, no second opener in the transcript;
- quit the app with a conversation running; relaunch; it is still running and
  its dot is right;
- the main window: no new place, nothing in the nav, `list_places` and MCP
  `place_status` do not list it, `doctor` does not call it an orphan;
- `audit.jsonl` records each `idea_send` with length and hash, never the text;
- no `.planning/` and no `CLAUDE.md` appear in the ideas folder.

**Size:** M–L.

### Phase 3: ideas relate

**David can:** relate two ideas (drag one onto the other, or the menu), merge
one into another, combine several into a new one, split a selection into a new
idea or extract a copy, follow `idea:` links and insert them with `[[`, see an
idea's related ideas, and restore any earlier version from history.

**Built:**

- The id index (§3): every `id` and `absorbed` id; lookups go through it.
- The four acts as core operations with parameters (the seam of §7.1), each
  writing its history copy first. Console-only rows for each.
- `.history/` and a "Versions" list per idea, with Restore.
- The phase 3 drags of §6.1 in the companion, each with its menu.

**Left out on purpose:** agent proposals, unmerge as an act.

**For agent help later:** this phase is the seam. `relate(a, b)`,
`merge(into, from)`, `combine(ids, title)` and `split(id, range, keep)` are
the operations a proposal will carry (C11, decided: combine is a merge into a
new idea, and extract is a split that keeps the original, so the four share
two bodies), so they take their whole input as
parameters (a merge takes the merged text, defaulting to the plain append,
so an agent's rewritten text is the same call). Every one writes history
first.

**Thrown away if we stop here:** the operations and the index. Every file and
every history copy stays.

**Checked in the real app:** merge B into A and open a link that pointed at B
(it lands on A, with the note); combine two into a new idea; split a paragraph
out and back by restoring; diff the history copy against the original file
(identical); drag an idea onto another and choose each answer of the chooser.

**Size:** M.

### Phase 4: ideas → plan → workspace

**David can:** select one idea or several and press "Make a plan" to get a plan
from the template, linked to all of them; drag more ideas onto it; fill in
Projects and Work; press "Make workspace" to get a workspace referencing the
registered projects and listing the rest as "to clone" or "to create"; drag a
plan or idea onto a workspace; open each from the other.

**Built:**

- `plan_of` lists (§2.2), and a small reader for `## Projects` and `## Work`
  that names a bad line and never blocks showing the document.
- `plan` and `ideas` on a workspace (§2.3). One plan, at most one workspace;
  making a second workspace from the same plan opens the first.
- The plan strip in the companion and the overview's Projects and Work lists in
  the main window, read-only here.
- The phase 4 drags, across windows if phase 0's test passed (§6.3), otherwise
  their same-window forms.

**Left out on purpose:** cloning, creating, starting places; agent drafts.

**For agent help later:** `make_plan(ideas, draft)` takes the draft text as a
parameter, defaulting to the template, so an agent's draft is the same call;
linking ideas, projects and a workspace are named operations too; history
before every change the app makes to a plan.

**Thrown away if we stop here:** the reader and the links. Plans stay as
documents.

**Checked in the real app:** the Jellyfin example from two ideas to a
workspace; a plan with a typo in `## Work` shows the line and the document;
renaming an idea keeps the plan's link; merging one of a plan's ideas into
another keeps the plan's `plan_of` resolving.

**Size:** M.

### Phase 5: projects and places from a workspace

**Part pulled forward:** the workspace's Add project menu (New, Add existing,
Clone from URL) is phase 2's follow-up. Phase 5 reuses it from a plan's "to
clone" and "to create" rows, so what is left here is the plan's side and the
places.

**David can:** clone a "to clone" project or create a "to create" one, after
which it is registered and referenced; open a work item, read and edit its
brief, and press Start, which creates the place with that brief and launches
its agent; drag an existing place onto a work item to record it as that
item's place; see which items were started and whether each place is there,
busy, or gone.

**Built:**

- Clone through `clone_project`; create through a new console row that runs
  `git init` and then `add_project` (both **checked**). Both warn on a path
  under `~/Documents`, `~/Desktop` or `~/Downloads`.
- A brief on the create path (§2.4: today's `new_place` has none).
  Drive-class, console-only.
- The review step: the brief as the exact text that will be written, editable,
  then Start.
- The work item's `started:` line (§2.2), written into the plan after the
  create succeeds, with live state shown beside it from the snapshot the nav
  already has.

**Left out on purpose:** a return path from finished work into the idea (v2's
Harvest), starting several items at once, MCP.

**For agent help later:** `start(item, brief)` takes the brief text as a
parameter, so an agent's proposed brief is the same call, and the review step
stays in front of it: an agent never starts a place, it can only fill the
text you review.

**Thrown away if we stop here:** nothing extra; this ends the chain.

**Checked in the real app:** in a scratch project, start a place from a plan
item; confirm `.planning/brief.md` holds the reviewed text and the agent opened
on it (the opener goes only on a first launch, AGENTS.md; check the pane);
remove the place and see "place removed" on the item, with its `started:`
line kept; drag another place onto an item and see the line written; create a project
at a scratch path and see it registered. Any scratch Claude runs with
`DISABLE_AUTOUPDATER=1` and the real `XDG_DATA_HOME` (AGENTS.md), which is why
§5 gives the stores their own override.

**Size:** M–L.

### Phase 6: agent help (after S and 3, beside 4 and 5)

Revision 3 had phase 6 build its own session kind, one session per request.
Phase S builds the session kind; phase 6 becomes a use of it (§7.4).

**David can:** press "Suggest…" beside relate, merge, split and Make a plan,
which asks in the idea's newest live conversation or starts one; or ask in his
own words in a conversation ("make a workspace for this and clone the jellyfin
repos"); see the agent's proposal in the chat with Accept, Edit, Skip; and
accept it, which runs the same operation the manual path runs.

**Built:**

- The proposal file of §7.4 (`.desk/<idea>/<conv>/proposals/<n>.md`, one act
  and its parameters in frontmatter), its reader, and the strip in the chat
  panel, where Accept runs the phase 3, 4 or 5 console row.
- "Suggest…" buttons, each sending one fixed request line into the
  conversation through `idea_send`.
- The acts it may propose, growing with the phases that ship them: relate,
  merge, split, combine (3); a plan draft, a workspace (4); clone or create a
  project, a reviewed brief (5).

**Left out on purpose:** conversations anchored to part of a document (later,
on §4.4's anchors), other harnesses than Claude at first, MCP tools for it
(§7.4), any agent act without a click.

**Thrown away if we stop here:** the proposal reader and the strip. Every
manual act still works, and the conversations stay.

**Checked in the real app:** ask for related ideas on a folder of five; skip
it and see it stay dismissed; ask for a merge draft, edit it, accept, and find
the history copy; ask for a workspace with one registered and one missing
project, accept, and see the clone dialog open with the agent's URL filled in
and waiting for David; close the companion while the agent works and reopen it
(the conversation is still there). `DISABLE_AUTOUPDATER=1` on the scratch
Claude, as in phase S.

**Size:** M.

## 9. The screens

### 9.1 The companion window

One window, labelled `companion`, separate from `main`. It can be maximised or
full screen on its own (the owner's ask). Documents in a list, one open at a
time; the idea's conversation panel (phase S) opens on the right.

```
┌ Ideas ───────────────────────────────────────────────────────────────────── ◻ ✕ ┐
│ ┌──────────────────┐ ┌──────────────────────────────┐ ┌──────────────────────────┐ │
│ │ + New idea   ⌕   │ │ Jellyfin Sharing             │ │ Who can see what  ▾  ⋯   │ │
│ │──────────────────│ │ saved 10:42 · [Edit|Read] · ⋯│ │──────────────────────────│ │
│ │ Jellyfin      ●  │ │──────────────────────────────│ │ You: who outside the     │ │
│ │  Sharing     2h  │ │ Jellyfin Sharing             │ │ house needs access?      │ │
│ │ Home server  3d  │ │ (heading, large; # dimmed)   │ │ Claude: three groups …   │ │
│ │  GPU             │ │                              │ │  ┌ Edit jellyfin-       │ │
│ │ Media library 1w │ │ - family outside the house   │ │  │ sharing.md   [Yes]   │ │
│ │ Docs search   2w │ │ - read-only for most         │ │  └ [No] [Open in term.] │ │
│ │                  │ │                              │ │──────────────────────────│ │
│ │ Show in Finder   │ │                              │ │ [type a message…]   Send │ │
│ └──────────────────┘ └──────────────────────────────┘ └──────────────────────────┘ │
└────────────────────────────────────────────────────────────────────────────────────┘

  ▾ (the conversation menu)
    ● Who can see what        running · started 10 Nov
      Transcoding hardware    ended · started 9 Nov       Resume
    + New conversation
```

- The dot beside an idea means a live conversation: amber when one waits on
  David, the busy colour while one works. The panel's `▾` lists the idea's
  conversations; `⋯` holds Rename, Close, Remove, Open in terminal, and
  "Started from this version" (the history copy).
- An ended conversation reads in the same panel, read-only, with Resume in
  place of the composer.
- Phase 6 adds the proposal strip (Accept, Edit, Skip) inside the chat.

- Newest-edited first. A plan sits indented under its first idea.
- Drag a row onto another row to relate or merge (§6.1). Drag selected text
  onto the list to split.
- `⋯` holds Rename, Remove (asks), Versions, Show in Finder.
- A plan adds a strip under its title: its ideas, its projects with state
  (registered, to clone, to create), and "Make workspace ▸" or "Open
  workspace ▸".

### 9.2 The workspaces view in the main window

A switch at the top of the nav: **All projects | Workspaces**. All projects is
today's nav, unchanged. Workspaces swaps the nav for workspace groups. The
right side (terminal, dock) is unchanged.

```
┌ worktrees ─────────────────────────────────────────────────────────────────┐
│ [All projects|Workspaces]  │  Jellyfin                                     │
│────────────────────────────│  plan: Plan: Jellyfin ▸   ideas: 2 ▸          │
│ ▾ Jellyfin              ●  │──────────────────────────────────────────────│
│   ▾ homelab-ansible        │  Projects                    (drop one here)  │
│       (main)               │   homelab-ansible   registered               │
│       jellyfin-role     ●  │   worktrees         registered               │
│   ▾ worktrees              │   jellyfin-config   to create  [Create…]     │
│       (main)               │                                               │
│       ui-changes           │  Work (from the plan)                         │
│       bug-fixes         ●  │   Ansible role      started 12 Nov ● busy    │
│ ▸ worktrees                │   Config repo       not started   [Review…]  │
│ + New workspace            │   Docs page         started · place removed  │
└────────────────────────────┴───────────────────────────────────────────────┘
```

- The Workspaces view starts empty, with only "+ New workspace". The second
  group in the sketch is the owner's example, a `worktrees` workspace holding
  the `worktrees` and `worktrees-ios` projects.
- Drag a project from All projects (or another workspace) onto a group.
- Under a workspace: its projects, then every place of each project (C8,
  decided). No toggle, no filter by what the workspace started.
- The Work list is the plan's work items, each with its status; a place is
  dragged onto an item there, never onto the workspace.
- The right side shows the workspace overview when the workspace row is
  selected, and the place as today when a place is selected.
- "plan ▸" and "ideas ▸" open the documents in the companion.
- In time this may become the default view (the owner's words); the switch
  makes that a setting, not a redesign.

## 10. Choices

**Decided on 2026-11-08** (now fixed in the plan above): C1, C2, C3, C4, C5
(answered by §7: you draft plans until phase 6), C7, and:

- **C6, the word:** today's nav is the projects view, "All projects"; a
  workspace exists only once someone makes one; code names stay.
- **C8, which places a workspace shows:** its projects, then every place of
  each project.
- **C9, AGENTS.md:** yes. The owner: "We can fix the AGENTS.md file to remove
  that contradiction." (main) handed the edit to the phase 0 lane, so it lands
  in the PR that creates the companion bundle and its boundary check. The
  wording to start from is the sentence below; this document does not edit
  AGENTS.md. *"CodeMirror 6 is the third exception, only in the Ideas
  companion bundle (`app/companion/`, with its own boundary check), because
  ideas need a real editor and CodeMirror edits markdown as text, so the file
  stays exactly what was typed. A rich-text editor that rewrites markdown from
  its own model is not admitted."*

**Decided on 2026-11-10:**

- **C10, where the helping agent runs:** (a), a session against the ideas
  folder in the chat view, in the shape the owner described: persistent
  conversations tied to one idea, several per idea, reopened like a lane's
  chat (§7.2, §7.3). The one-per-request shape moves into phase 6 as a use of
  them.

**Decided on 2026-11-11.** The owner, on PR #641's choices: "all (a)".

- **C11, combine and merge:** one act. Combine is a merge into a new idea;
  extract is a split that keeps the original (§3, phase 3).
- **C12, the tmux name of an idea session:**
  `worktrees~idea~<conversation ULID>`. `~` never appears in a place's
  session, so nothing can mistake it for one, and the dock's `~term` sidecars
  already rely on that (§7.3).
- **C13, what tells the agent its job:** a per-conversation `start.md` from a
  fixed template, named by the fixed opener; no `CLAUDE.md` in the ideas
  folder (§7.3).
- **C14, the idea chat's send:** yes. `idea_send` and `idea_answer_text` are
  new console-only rows over the SAME bodies as `agent_send` and
  `agent_answer_text` (the same `typed_line_ok`, paste, checks and audit), so
  text reaches one more kind of session by the existing path. This is the
  owner's yes to that second door, which AGENTS.md otherwise forbids (§7.3).
- **C15, idea sessions and the live-agent limit:** they count against host
  policy's `max_live_agents`, like any lane, and a start past it is refused
  with the same words (§7.3).

Nothing is open.

## 11. What I could not establish

- **A drag between two Tauri windows**, on macOS or Linux. Not measured; it is
  phase 0's hand test, and §6.3 has both outcomes.
- **CodeMirror 6 in this app's WKWebView**, including IME input and undo.
  Assumed from its Safari support; phase 1's check measures it.
- **That stage 1 decorations never change the file.** True by CodeMirror's
  design (assumed); the phase 1 byte-for-byte check proves it.
- **A second Tauri window loading a second page from `frontendDist`, built by
  a separate config.** Assumed; the viewer runs in a browser, so it is not
  proof. Phase 0's first task.
- **How a second window interacts with** ⌘Q, the updater's relaunch, the
  app-owned serve's idle stop (it counts console connections, so a companion
  holds serve up; assumed harmless), full screen on its own Space, and the
  "hidden window defers retries" rule when only one window is hidden.
- **Phases 0 to 2 shipped.** What their real-app checks found (the
  cross-window drag of §6.3, CodeMirror in WKWebView, the second page) is in
  their PRs, #632 to #636; revision 4 did not re-read them, so the items
  below about those phases are left as revision 3 wrote them.
- **Every place-shaped key an idea session meets.** Read for revision 4: the
  launcher (`ops::ai_launch_for` takes a `Project`), the agent rows
  (`place(repo, slug)`), `AgentIo` (`(project, slug, lane)`), the chat's
  target (`{repo, slug, path, session}`) and the dots (`sessions:busy`, by
  cwd). Not read: `doctor`'s view of tmux sessions it cannot place, the
  live-agent limit's counter, the MCP server's `list_places`, and the
  audit's record shape for a row with no place. Phase S's first task is to
  read those too.
- **Whether the chat's prompt reader reads Claude's folder-trust prompt.**
  Assumed not; "Open in terminal" covers it.
- **Claude's `--session-id` with `--name` and the plugin dirs on one launch,**
  and `--resume <id>` from the same cwd finding the transcript. Read in
  `claude --help` (2.1.322), not run.
- **How long Claude keeps a transcript.** Assumed it removes old ones after a
  cleanup period; an ended conversation then has a record and no text.
- **Whether worktrees' planning hooks reach a session launched outside a
  place**: checked for revision 4, they do not, as long as the idea launcher
  does not go through `ai_launch_for` (§7.3). The user's own global plugins
  still run.
- **Moving a file to the Trash from serve**, including on Linux.
- **Bear's exact rules** beyond what its pages say: which markers hide, when
  tables switch to source. The table in §4.2 is ours, not a copy of Bear's.
- **The `## Work` format.** The item shape in §2.2 is a sketch; the phase 4
  lane should try it on two real plans before fixing it.
- **Bundle sizes.** Not built yet.
