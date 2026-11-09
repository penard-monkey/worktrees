---
title: "Ideas: the phase plan"
---

# Ideas: the phase plan

**Status:** plan, **revision 2, 2026-11-08**. Nothing is built yet. The owner
read v1 and v2 of [Ideas](idea-board.html) and decided to build the idea
board in phases, in the shape v1 first described: a place for ideas inside
worktrees, desktop only. This document is that build plan. It overrules v2's
"rehearse first, build no destination yet" (v2's own choices C2(c) and C3(b)).

**What changed in this revision (2026-11-08, after the owner's answers):**

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
- The phases are now seven (§8). The choices still open are C6, C8, C9 (in
  plain words), and two new ones, C10 and C11 (§9).

Claims about the app are marked **checked** (read on `origin/main` at
`1f2064d` on 2026-11-08) or **assumed** (not measured). Claims about libraries
and about Bear are marked **read** (npm registry, Bear's own pages, 2026-11-08)
or **assumed**.

## 1. The answer

Build the chain the owner named, end to end, before anything richer:

> "turn an idea into a plan which can then be a workspace which can then be a
> set of projects, places to work on it."

| # | Phase | What David can do at the end | Size |
|---|---|---|---|
| 0 | Companion window | Open an Ideas window, full screen it, close it without quitting; try a drag between windows | S–M |
| 1 | Documents | Write ideas in a Bear-like editor: create, list, open, edit, rename, remove | M |
| 2 | Workspaces view | Make named workspaces and drag projects into them, in the main window | M |
| 3 | Ideas relate | Relate, merge, split and combine ideas, by drag or menu, with nothing lost | M |
| 4 | Ideas → plan → workspace | Make a plan from one idea or several, and a workspace from the plan | M |
| 5 | Projects and places | Clone or create missing projects, and start places from the plan with a reviewed brief | M–L |
| 6 | Agent help | Ask an agent to propose a merge, a split, links or a plan draft, and accept, edit or skip it | L |

Later, and not in this plan: conversations on part of a document, the rest of
the Bear-like editor (§4.3), diagrams, quick capture, threads, MCP tools.

```
  lane A  [0 window]──[1 documents: screen]──[3 relate]──[4 chain]──[5 places]
  lane B  [1 documents: store + rows]─┘                     │
  lane C  [2 workspaces view]───────────────────────────────┘
  lane D                                       [6 agent help]  (after 3)
```

Phase 0 and phase 2 run from day one, and so do phase 1's store and rows.
Phase 3 needs 1. Phase 4 needs 2 and 3. Phase 5 needs 4. Phase 6 needs 3, and
can run beside 4 and 5; each act it helps with must exist first.

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

### 2.3 A workspace

A **named list of references to registered projects**, plus the plan it came
from and any ideas linked to it. It answers the owner's first ask
(2026-11-06): "a workspace with multiple projects? Where the workspace also has
context?" The context is the plan and its ideas.

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
      ],
      "started": [
        { "item": "Ansible role for Jellyfin", "root": "/Users/…/homelab-ansible",
          "branch": "jellyfin-role", "slug": "jellyfin-role", "at": "2026-11-12T10:04:00Z" }
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
- `started` is the durable record of what this workspace started (v2's
  "commission"), kept apart from the live place. If the place goes, the record
  stays and the view says the place is gone.

**What the existing word becomes** (choice C6). Today "workspace" means every
registered project on this host (**checked**: `list_workspace`,
`crates/worktrees-api/src/workspace.rs`). Named workspaces take the word. In
the UI the current meaning becomes **All projects**, and it stays the default.
In code `list_workspace` keeps its name; renaming a row the app, serve and the
phone all call is churn with no benefit to David.

### 2.4 From a workspace to projects and places

| Kind of project in the plan | What phase 5 does |
|---|---|
| Registered | references it |
| `clone: <url>` | runs the existing clone flow (`clone_project`, **checked** in dispatch.rs), then references it |
| `new: <path>` | `git init` at the path through a new console row, registers it with `add_project` (**checked**), then references it |

Then each `## Work` item becomes a place: David opens the item and reads its
brief, shown as the exact text that will be written and editable there; Start
creates the place with that brief; the workspace records it in `started`.
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
Agent proposals in its `.desk/` (§7).

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
| a place (nav row) | a workspace | record an existing place as this workspace's work (a `started` record, no new place) | place menu → Add to workspace ▸ | 5 |
| a `.md` file from Finder | the idea list | import it as a new idea (a copy) | ⋯ → Import… | later |

Considered and left out: dragging a work item onto a project (the item's
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

### 7.2 Where the agent runs (v1's Q2)

There is no worktree for an idea. Three shapes:

| | One-shot headless call per proposal | **A session against the ideas folder, in the chat view** | A place in a hidden `ideas` project |
|---|---|---|---|
| What it is | `claude -p` (or another harness's headless mode) started per request, its output parsed | a Claude in its own tmux session with the ideas folder as its working directory, shown in the companion through the existing chat view | the ideas folder made a git repo, registered as a project, one place per conversation |
| You can see and stop it | no; it runs and returns | yes; it is a session like a lane's | yes |
| Follow-up ("shorter", "keep the second half") | a new call with everything re-sent | type in the chat | type in the chat |
| New engine work | a parser for its output | a session kind that is not a place: launch, dots, chat view keyed by it | almost none |
| Cost to the rest of the app | none | small | a project in every list, a branch per conversation (the "not a branch" problem v1 began with), and the ideas folder becomes a repo |

**Pick: the session against the ideas folder** (choice C10). It keeps "you can
see it and stop it", it reuses the chat view that places already have
(`chat-session-view.md`), and it adds one kind of session instead of a fake
project.

How it works, shallowly:

- One request is one launch, the way a lane starts on its brief: the app
  writes the request to `ideas/.desk/<request>.md` and launches the session on
  a fixed opener that names that file. Nothing reaches Claude through argv and
  nothing is typed into it to start it, as with briefs (**checked**,
  AGENTS.md: `BRIEF_OPENER`).
- The agent writes its proposal to `ideas/.desk/<request>.proposal.md`, with
  frontmatter naming the act and its parameters. The companion shows it with
  Accept, Edit, Skip. Accepting runs the operation.
- A follow-up line goes through the chat view's existing send, which is
  `agent_send` (**checked**, AGENTS.md: the owner-approved exception for
  typing into Claude). This is the same row reaching a new kind of session, not
  a fourth path; it still needs David's yes, because AGENTS.md says not to
  grow paths that type into Claude (part of C10).
- The session ends when the proposal is accepted or skipped.

**Its reach is a route rule, not a sandbox** (v2's lesson). The agent can
technically write any file in the ideas folder. It is told to write only into
`.desk/`; the app never applies a proposal without your accept; a file changed
outside the app is noticed by the store's change event; and history makes any
stray write recoverable.

**AGENTS.md's warning about starting a Claude to ask it something.** The rule
exists because a Claude started in a repository runs that repository's hooks:
a scratch `claude -p` in an untrusted clone ran its `SessionStart` hook
(**checked**, AGENTS.md, measured 2026-11-04). The ideas folder changes the
main reason: it is worktrees' own folder, not a cloned repo, and has no
`.claude/` of anyone else's. Three things still apply:

- The **user's own** global hooks and plugins run (they are the user's, which is
  acceptable, but each start costs seconds). That is one more reason for one
  visible session per request over a burst of hidden calls.
- worktrees' own planning hooks must be **off** for this session, or it grows a
  `.planning/` inside the ideas folder (**assumed**: launched like a lane, it
  would get them).
- The first launch meets Claude's folder-trust prompt for the ideas folder;
  the amber dot already shows a pane waiting on it (**checked**, AGENTS.md).

**First used in phase 6.** Its first request is the cheapest: "Suggest
related ideas". Then merge, split, and plan drafts. This also answers C5: in
phase 4 David drafts a plan from the template; from phase 6 an agent can
propose the draft.

Ideas and workspaces stay desktop only (decided): no MCP tools for other
agents, nothing for paired devices or the phone. The desk agent reads the
ideas folder as files, which needs no MCP tool.

## 8. The phases

Every phase ends with a check in the real app, by hand, through
`app/scripts/sandbox.sh --app`, because nothing in CI runs the app
(**checked**, AGENTS.md). Each check runs with the stores' override (§5)
pointing at a scratch folder and its own tmux server (`env -u TMUX
TMUX_TMPDIR=<short dir>`). Never David's running app.

### Phase 0: the companion window

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

**David can:** switch the main window's nav to Workspaces, make a workspace,
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

**Left out on purpose:** plans, ideas, `started`, cloning or creating.

**Thrown away if we stop here:** the switch, the grouped nav, the store.
All projects is today's nav.

**Checked in the real app:** two workspaces sharing a project; select a place
through a workspace and through All projects (the same terminal); drag a
project onto a workspace and out again; remove a workspace and diff
`projects.json` before and after (no change); unregister a project a workspace
references and see "not registered".

**Size:** M.

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

**Thrown away if we stop here:** the reader and the links. Plans stay as
documents.

**Checked in the real app:** the Jellyfin example from two ideas to a
workspace; a plan with a typo in `## Work` shows the line and the document;
renaming an idea keeps the plan's link; merging one of a plan's ideas into
another keeps the plan's `plan_of` resolving.

**Size:** M.

### Phase 5: projects and places from a workspace

**David can:** clone a "to clone" project or create a "to create" one, after
which it is registered and referenced; open a work item, read and edit its
brief, and press Start, which creates the place with that brief and launches
its agent; drag an existing place onto a workspace to record it; see which
items were started and whether each place is there, busy, or gone.

**Built:**

- Clone through `clone_project`; create through a new console row that runs
  `git init` and then `add_project` (both **checked**). Both warn on a path
  under `~/Documents`, `~/Desktop` or `~/Downloads`.
- A brief on the create path (§2.4: today's `new_place` has none).
  Drive-class, console-only.
- The review step: the brief as the exact text that will be written, editable,
  then Start.
- `started` records, written after the create succeeds, with live state from
  the snapshot the nav already has.

**Left out on purpose:** a return path from finished work into the idea (v2's
Harvest), starting several items at once, MCP.

**Thrown away if we stop here:** nothing extra; this ends the chain.

**Checked in the real app:** in a scratch project, start a place from a plan
item; confirm `.planning/brief.md` holds the reviewed text and the agent opened
on it (the opener goes only on a first launch, AGENTS.md; check the pane);
remove the place and see "place removed" with the record kept; create a project
at a scratch path and see it registered. Any scratch Claude runs with
`DISABLE_AUTOUPDATER=1` and the real `XDG_DATA_HOME` (AGENTS.md), which is why
§5 gives the stores their own override.

**Size:** M–L.

### Phase 6: agent help (after 3, beside 4 and 5)

**David can:** press "Suggest…" beside relate, merge, split and Make a plan;
watch the agent work in a chat panel in the companion; accept, edit or skip
its proposal; and type a follow-up to it.

**Built:**

- A session kind that is not a place: launch against the ideas folder on a
  fixed opener, its dot, its chat view, its close (§7.2).
- `.desk/` requests and proposals, and the Accept / Edit / Skip strip, where
  Accept runs the phase 3 or 4 operation.
- Planning hooks off for that session; the trust prompt handled by the
  existing amber dot.

**Left out on purpose:** conversations anchored to part of a document (later,
on §4.4's anchors), other harnesses than Claude at first, any agent act
without a click.

**Thrown away if we stop here:** the session kind and `.desk/`. Every manual
act still works.

**Checked in the real app:** ask for related ideas on a folder of five; skip
it and see it stay dismissed; ask for a merge draft, edit it, accept, and find
the history copy; ask for a plan draft and accept it; close the companion while
the agent works and reopen it (the session is still there, or ended cleanly).
`DISABLE_AUTOUPDATER=1` on that Claude, as in phase 5.

**Size:** L.

## 9. The screens

### 9.1 The companion window

One window, labelled `companion`, separate from `main`. It can be maximised or
full screen on its own (the owner's ask). Documents in a list, one open at a
time; the agent panel (phase 6) opens on the right when asked.

```
┌ Ideas ─────────────────────────────────────────────────────────────── ◻ ✕ ┐
│ ┌────────────────────┐ ┌───────────────────────────────────┐ ┌──────────┐ │
│ │ + New idea    ⌕    │ │ Jellyfin on the home server       │ │ Agent    │ │
│ │────────────────────│ │ saved 10:42 · [Edit|Read] · ⋯     │ │ (phase 6)│ │
│ │ ● Jellyfin on the  │ │ Related: Home server GPU ·        │ │          │ │
│ │   home server  2h  │ │   Media library tidy  Suggest…    │ │ Proposal:│ │
│ │   └ Plan: Jelly 1d │ │───────────────────────────────────│ │ merge    │ │
│ │ Home server GPU 3d │ │ Jellyfin on the home server       │ │ these two│ │
│ │ Media library  1w  │ │ (heading, large; # dimmed)        │ │          │ │
│ │ Docs search    2w  │ │                                   │ │ [Accept] │ │
│ │                    │ │ - transcode on the GPU?           │ │ [Edit]   │ │
│ │                    │ │ - which box                       │ │ [Skip]   │ │
│ │ Show in Finder     │ │                                   │ │          │ │
│ └────────────────────┘ └───────────────────────────────────┘ └──────────┘ │
└──────────────────────────────────────────────────────────────────────────┘
```

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
│       jellyfin-role     ●  │   homelab-ansible   registered               │
│   ▾ worktrees              │   worktrees         registered               │
│       (main)               │   jellyfin-config   to create  [Create…]     │
│ ▸ Phone                    │                                               │
│                            │  Work                                         │
│ + New workspace            │   Ansible role      started 12 Nov ● busy    │
│                            │   Config repo       not started   [Review…]  │
│                            │   Docs page         started · place removed  │
└────────────────────────────┴───────────────────────────────────────────────┘
```

- Drag a project from All projects (or another workspace) onto a group.
- Under a workspace, each project shows the places this workspace started plus
  `(main)`, with "show all" (C8).
- The right side shows the workspace overview when the workspace row is
  selected, and the place as today when a place is selected.
- "plan ▸" and "ideas ▸" open the documents in the companion.
- In time this may become the default view (the owner's words); the switch
  makes that a setting, not a redesign.

## 10. Choices still open

Recommended answer first.

- **C6. The word "workspace".** (a) "All projects" in the UI for today's
  meaning (every registered project), code names unchanged. (b) Keep
  "Workspace" for all projects and call the new thing something else.
- **C8. Which places a workspace shows.** (a) The ones it started, plus
  `(main)`, with "show all". (b) Every place of every referenced project.
- **C9. One rule-file sentence, a yes or no before phase 1 merges.**
  AGENTS.md is the rule file every agent in this repo reads. It says the app
  uses no UI libraries and no editor, and names two exceptions: `marked` (a
  markdown parser) and `mermaid` (diagrams, only inside the docs viewer).
  Using CodeMirror contradicts that sentence, so it needs a third named
  exception, and David approves edits to that file himself. The proposed
  sentence: *"CodeMirror 6 is the third exception, only in the Ideas companion
  bundle (`app/companion/`, with its own boundary check), because ideas need a
  real editor and CodeMirror edits markdown as text, so the file stays exactly
  what was typed. A rich-text editor that rewrites markdown from its own model
  is not admitted."* (a) Yes, add it. (b) Change the wording.
- **C10. Where the helping agent runs** (§7.2). (a) A session of its own
  against the ideas folder, one per request, shown in the chat view, with
  follow-ups through the chat view's existing send. (b) A one-shot headless
  call per proposal, with no session to see or stop. (c) A place in a hidden
  ideas project. Saying yes to (a) is also saying yes to `agent_send` reaching
  this one new kind of session.
- **C11. Combine and merge as one act** (§3). (a) Yes: combine is a merge into
  a new idea; extract is a split that keeps the original. (b) Keep them as
  separate acts with their own menus.

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
- **Launching a Claude session that is not a place.** The launch, dot and chat
  view are all keyed by place today (assumed from AGENTS.md's descriptions,
  not read in code). Phase 6's first task is to find every place-shaped key.
- **Whether worktrees' planning hooks reach a session launched outside a
  place**, and how to keep them out of the ideas folder.
- **Moving a file to the Trash from serve**, including on Linux.
- **Bear's exact rules** beyond what its pages say: which markers hide, when
  tables switch to source. The table in §4.2 is ours, not a copy of Bear's.
- **The `## Work` format.** The item shape in §2.2 is a sketch; the phase 4
  lane should try it on two real plans before fixing it.
- **Bundle sizes.** Not built yet.
