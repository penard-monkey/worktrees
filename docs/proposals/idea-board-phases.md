---
title: "Ideas: the phase plan"
---

# Ideas: the phase plan

**Status:** plan, **2026-11-08**. Nothing is built yet. The owner read v1 and
v2 of [Ideas](idea-board.html) and decided to build the idea board in phases,
in the shape v1 first described: a place for ideas inside worktrees. This
document is that build plan. It overrules v2's "rehearse first, build no
destination yet" (v2's own choices C2(c) and C3(b)). From v2 it keeps one
thing: a stop rule, with phases small enough to remove.

Claims about the app are marked **checked** (read on `origin/main` at
`1f2064d` on 2026-11-08) or **assumed** (not measured). Claims about libraries
are marked **read** (npm registry, 2026-11-08) or **assumed** (from what I
know, not checked today).

## 1. The answer

Build the chain the owner named, end to end, before anything richer:

> "turn an idea into a plan which can then be a workspace which can then be a
> set of projects, places to work on it."

Five phases, each one usable on its own and each one removable:

| # | Phase | What David can do at the end | Size |
|---|---|---|---|
| 0 | Companion window | Open an empty Ideas window, full screen it, close it without quitting the app | S |
| 1 | Documents | Write ideas: create, list, open, edit, rename, remove | M |
| 2 | Workspaces view | Make named workspaces of existing projects and switch to them in the main window | M |
| 3 | Idea → plan → workspace | Turn an idea into a plan, and the plan into a workspace | M |
| 4 | Projects and places | Clone or create the missing projects, and start places from the plan with a reviewed brief | M–L |

Later, and not in this plan: conversations on part of a document, diagrams,
quick capture, threads, MCP tools, the phone.

Phase 0 and phase 2 can run as parallel lanes from day one. Phase 1's store
and rows can start beside phase 0, and its screen lands once the window exists.
Phase 3 needs 1 and 2. Phase 4 needs 3.

```
  lane A   [0 window]──[1 documents: screen]──┐
  lane B   [1 documents: store + rows]────────┤
  lane C   [2 workspaces view]────────────────┴──[3 chain]──[4 places]
```

Sizes: **S** is about one lane-day, **M** two to four, **L** about a week.
These are guesses.

My picks for the open choices (all in §9):

- "Canvas" means a blank document, not a spatial board.
- The editor is CodeMirror 6 on the markdown source, with a rendered view
  beside it, in the companion's own bundle.
- Documents live in `~/.local/share/worktrees/ideas/`. Workspaces live in
  `~/.config/worktrees/workspaces.json`, beside `projects.json`.
- A plan is its own document, linked to its idea.
- Ideas and workspaces are console-only at first: no MCP tools, nothing for
  paired devices or the phone.

## 2. The chain, made concrete

Four things, each in plain files, and none of them contains or owns a project.
That is v1's rule and it stays.

```
 idea (document) ──make plan──▶ plan (document) ──make workspace──▶ workspace
                                     │                                  │
                                     │ work items                       │ references
                                     ▼                                  ▼
                              reviewed brief ──start──▶ place  ◀── project (registered)
```

### 2.1 An idea

One markdown file. It is the blank document the owner asked for. Nothing in it
is required except a first heading, which becomes its title.

```markdown
---
id: 01JC8Z4K3M9QX2T7VBN5R6WYHD
created: 2026-11-09
---

# Jellyfin on the home server

Free writing. Lists, links, code, whatever.
```

- `id` is a ULID written once, at creation. It never changes. The file name
  can change on rename; links between documents use the id, not the name.
- The title is the first `# heading`, read from the text. There is no title in
  the frontmatter, so the two cannot disagree.
- The file name is a slug of the title at creation (`jellyfin-on-the-home-
  server.md`). Rename changes both the heading and the file name.

### 2.2 A plan

A plan is **its own document**, made from an idea with "Make a plan". It is a
markdown file in the same folder, with a link back:

```markdown
---
id: 01JC90A1...
plan_of: 01JC8Z4K3M9QX2T7VBN5R6WYHD
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

Why a separate document and not a `## Plan` section in the idea:

- The idea stays loose. The plan is the thing that gets reviewed and that
  briefs are cut from. Mixing them means every edit to the thinking looks like
  an edit to the plan.
- One idea can have more than one plan over time, when the direction changes.
  An old plan stays as a record of what was tried.
- The plan's `## Projects` and `## Work` sections have a shape the app reads.
  The idea has no shape at all. Keeping the shape in one kind of document keeps
  the parser small.

The plan is still plain markdown that David edits in the same editor. The app
reads two sections from it (`## Projects`, `## Work`) and leaves everything
else alone. If those sections do not parse, the plan view says which line and
shows the document anyway, the way the Plan tab does for `task_plan.md`.

**Who writes the first draft:** David, from a template the app fills with the
idea's title and a link. A "draft it with Claude" button is a conversation, and
conversations are later (§7). This is choice C5.

**How a plan relates to a place's `.planning/`:** it sits above it. The idea
plan says which places should exist and why. A place's own `task_plan.md` is
that place's agent's working memory, as today. They do not sync. The link
between them is the brief: at start (phase 4) the work item's brief is
**copied** into the new place's `.planning/brief.md` through the existing
brief path. After that, the brief in the place is the place's.

### 2.3 A workspace

A workspace is a **named list of references to registered projects**, plus a
link to the plan it came from (if any). It answers the owner's first ask on
2026-11-06: "a workspace with multiple projects? Where the workspace also has
context?" The context is the plan and its idea.

```json
{
  "workspaces": [
    {
      "id": "01JC91...",
      "name": "Jellyfin",
      "plan": "01JC90A1...",
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

- A project is referenced by its registry `root`, the same key `projects.json`
  uses (**checked**: a registry entry has `root` and `name`). A workspace never
  copies a project's name or settings.
- Removing a workspace removes the list. No project, place or branch is
  touched. Removing a project from the registry leaves the reference, and the
  workspace shows it as "not registered" with a way to drop it.
- A project can be in any number of workspaces.
- `started` is the durable record of what this workspace started (v2's
  "commission"). It is separate from the live place: if the place is removed
  or its branch changes, the record stays and the view says the place is gone.
  This is kept shallow on purpose; it is the one piece of v2's memory work
  that phase 4 needs.

**What the existing word becomes.** Today "workspace" means every registered
project on this host (**checked**: `list_workspace` in
`crates/worktrees-api/src/workspace.rs`). Named workspaces take the word. In
the UI the current meaning becomes **All projects**, and it stays the default.
In code, `list_workspace` keeps its name for now; renaming a row that the app,
serve and the phone all call is churn with no user benefit. This is choice C6.

### 2.4 From a workspace to projects and places

A plan lists three kinds of project:

| Kind | In the plan | What phase 4 does |
|---|---|---|
| Registered | the project's name | references it |
| To clone | `clone: <url>` | runs the existing clone flow (`clone_project`, **checked** in dispatch.rs), then references it |
| To create | `new: <path>` | `git init` at the path through a new console row, registers it with `add_project` (**checked**), then references it |

Then each `## Work` item becomes a place:

1. David opens the item and reads its brief. It is shown as the text that will
   be written, editable there.
2. "Start" creates the place with that brief, in that project, on that branch.
3. The workspace records it in `started`.

Nothing starts without step 1. A plan with five items does not start five
agents on one click.

**A conflict with the brief I was given.** (main)'s brief says to start places
"with the machinery that exists (create_worktree with a reviewed brief)". The
brief path exists in the CLI (`worktrees new --brief`) and in MCP
`create_worktree`, but the app's serve row `new_place` takes **no brief**
(**checked**: `ops::new_place(repo, branch, base, name, provider, model,
force)`). So phase 4 must add a brief argument to `new_place` (or a sibling
row) that calls the same core path as `new --brief`. That is small, but it is
new code in a drive-class row, not reuse.

## 3. Where the files live

**Pick:**

- Documents: `$XDG_DATA_HOME/worktrees/ideas/`, which is
  `~/.local/share/worktrees/ideas/` by default.
- Workspaces: `$XDG_CONFIG_HOME/worktrees/workspaces.json`, which is
  `~/.config/worktrees/workspaces.json`, beside `projects.json`.
- A later setting may move the documents folder. Workspaces stay put.

Why:

- Documents are the user's data, not settings and not engine state. worktrees
  already keeps its user data in `$XDG_DATA_HOME/worktrees`: profiles, the
  skill store and `agent/` live there (**checked**: `profile.rs`,
  `guidance.rs`; the folder exists on this Mac with `agent` in it, per
  (main)). The same path on macOS and Linux, no setup.
- Workspaces reference `projects.json` by root, and both are read by the CLI
  and every engine. Keeping them in one folder keeps one place to back up and
  one place a test must isolate.
- Not the app's config dir (`~/Library/Application Support/<identifier>`): that
  holds engine state, and a sandbox identifier moves it, so a sandbox run would
  "lose" every idea.
- Not `~/.cache/worktrees`: cache can be deleted.
- Never under `~/Documents`, `~/Desktop` or `~/Downloads`: macOS asks a privacy
  prompt on first touch, and a denial takes file contents while leaving
  metadata (AGENTS.md has the measurement). A later "move the folder" setting
  must warn there for the same reason.

**Against `~/.worktrees`:**

- It does not exist today (**checked** by (main)), so it is a new top-level
  dot-folder in `$HOME` for one feature.
- It collides with places. A repository rooted at `$HOME` (a dotfiles repo)
  registered as a project keeps its places in `~/.worktrees/<slug>`, so an
  `ideas` folder there is a place slug waiting to happen. The rule "a place
  lives at `<root>/.worktrees/<slug>`" is **checked** (AGENTS.md, "Working with
  worktrees").
- Its only advantage is that it is easy to find. A "Show in Finder" item in
  the companion gives that without the collision.

**A trap with the data dir, and how to avoid it.** AGENTS.md says never give a
scratch Claude its own `XDG_DATA_HOME`: Claude keeps its versions there and
relinks `~/.local/bin/claude` to a copy. A real-app check of phase 4 starts a
Claude. So the ideas folder must be movable WITHOUT moving `XDG_DATA_HOME`:
one override, read by the store, used by tests and by sandbox runs.
Workspaces need the same: serve's `--config-dir` does not move the registry's
folder (**checked**, AGENTS.md: "a test's config dir does not isolate it"), so
it would not move `workspaces.json` either. Simplest is one variable, for
example `WORKTREES_USER_DIR`, that moves both stores. The name is the phase 1
lane's; the rule is that neither store can be isolated only through `XDG_*`.

**What a test must do so it never writes the user's real files:**

- Every store function has an `_at(path)` form. Unit tests call only those,
  with a temp dir.
- An in-process engine under test runs a guard like
  `registry::forbid_real_writes()` (**checked**: it exists for
  `projects.json`), extended to the ideas folder and `workspaces.json`. After
  it, a write to the real path fails with an error saying why.
- A test that spawns the binary gives the child its own `HOME`,
  `XDG_CONFIG_HOME` and the override above, per child, never with `set_var` in
  the test process.
- Show the guard red once under a temp `HOME`, as AGENTS.md says for the
  registry guard.

## 4. The editor

**Pick: CodeMirror 6 editing the markdown source, with a rendered view beside
it (or toggled), in the companion window's own bundle.** Not a rich or block
editor, not now.

### 4.1 Why

The deciding rule is the first one: markdown on disk is the canonical format
and must round-trip without churn. CodeMirror edits the text itself. What
David types is the file, byte for byte; a save of an untouched paragraph
changes nothing. Every rich editor in the list parses markdown into its own
document tree and prints it back out. The print is a fresh rendering, so the
file changes in places nobody edited (bullet markers, emphasis markers,
escapes, blank lines, table padding). That breaks `git diff`, any agent that
also edits the file, and the "stable anchor" goal below. The rich editors can
be tamed for this, but taming a serializer is permanent work.

| | Markdown round-trip | Anchors for a part of a doc | Diagrams later | WKWebView | Licence, size (unpacked) |
|---|---|---|---|---|---|
| Plain source + rendered view (`<textarea>` + the viewer's renderer) | exact by construction | positions only; none survive an edit | rendered view can draw a mermaid fence | fine (**assumed**: a textarea) | none |
| **CodeMirror 6** (`@codemirror/view` 6.43.15, `@codemirror/lang-markdown` 6.5.2) | exact by construction | positions that map through every edit (its change sets); persisted anchors are text in the file (§4.2) | a fence can render as a widget, or in the rendered view | supports Safari (**assumed** from its docs; not measured in this app) | MIT (**read**); view 1.26 MB |
| ProseMirror + `prosemirror-markdown` (1.42.9 / 1.13.9) | re-serialized; markdown-it in, its own printer out | node positions map through edits; node attributes can hold ids | node views | contenteditable, works, has WebKit quirks (**assumed**) | MIT (**read**); 0.91 MB + model |
| Tiptap 3 + `@tiptap/markdown` (3.33.0) | re-serialized; parses with `marked` (**read**: its one dependency) | as ProseMirror | as ProseMirror | as ProseMirror | MIT core (**read**); core 3.3 MB; some extensions are paid (**assumed**) |
| Lexical + `@lexical/markdown` (0.53.0) | re-serialized; markdown is an import/export, not its model | node keys are per session, not persisted | decorator nodes | contenteditable (**assumed**) | MIT (**read**); 3.7 MB; still 0.x |
| Milkdown (`@milkdown/kit` 7.23.0) | re-serialized via remark (`remark-parse`/`remark-stringify`, **read**) | ProseMirror underneath | plugins | as ProseMirror | MIT (**read**) |
| BlockNote (`@blocknote/core` 0.57.0) | markdown export is lossy by its own docs (**assumed**) | block ids, but in its JSON, not in markdown | custom blocks | as ProseMirror | **MPL-2.0** core, **GPL-3.0 or proprietary** for its AI package (**read**); 4.9 MB |

Sizes are npm's unpacked size, which is not bundle size. A measured bundle is a
phase 1 task.

BlockNote is out on licence alone: MPL-2.0 is file-level copyleft inside an
MIT app, and its AI package is GPL. Lexical stores its markdown as an export,
not a model. Tiptap and Milkdown are the credible rich options, and both
re-print the file.

What CodeMirror does not give: a document that looks finished while you type
(headings big, links clickable). Two answers, both later and both possible on
the same file: CodeMirror "live preview" decorations, which style the source
in place without changing it; or a rich editor added as a second mode once
anchors exist and a round-trip test can prove a no-op save writes nothing. The
format does not change either way, which is the door kept open.

### 4.2 What the format must allow now, for later

- **Conversations on part of a document** need a stable anchor. Rule: an
  anchor is written into the file only when something attaches to that part,
  as an HTML comment at the end of the block: `<!-- ¶ 7k2q -->`. Renderers hide
  HTML comments, so a document with no conversations has no anchors and no
  noise. If the anchor is lost (someone deletes the line), the attachment falls
  back to the quoted text it was made on. Phase 1 writes no anchors; it only
  promises to keep unknown HTML comments and frontmatter exactly as they are,
  which CodeMirror does by construction.
- **Diagrams** are a fenced ```` ```mermaid ```` block in the markdown, which
  the docs viewer already renders. Nothing new in the format.
- **Links between documents** use the `id`, as `[text](idea:01JC8Z…)`. A
  phase 3 detail.

### 4.3 Its own bundle, like the viewer

The companion is its own Vite build, the way the docs viewer is
(`app/vite.viewer.config.ts`, **checked**). The editor library and, later,
mermaid enter that bundle and never `app/src`. A check in the shape of
`viewer-boundary-check.mjs` walks the app's import graph and fails if
`@codemirror/*` is reachable from it. It reuses the viewer's rendering source
(`app/viewer/blocks.tsx`, `Mermaid.tsx`, **checked** they exist) as source
files, never as a shared chunk.

One difference from the viewer, **assumed** and to be proved in phase 0: the
viewer is a page served by `docserver` to a browser; the companion is a Tauri
window loading a page from the app's own `frontendDist`. So the companion's
build must land in `app/dist/` as a second HTML page with its own script, built
by its own config, not as a second input to `vite.config.ts` (that would let
rollup share chunks, which is exactly what the viewer's config forbids).

### 4.4 Proposed AGENTS.md wording

For David to approve; this document does not edit AGENTS.md. Replacing the
"No UI libraries" sentence in "Design tokens":

> No UI libraries, plain CSS. That means no component or design-system
> libraries. The admitted exceptions each live behind a boundary a check
> enforces: `marked` (a parser, in `app/src`); `mermaid`, only in the docs
> viewer bundle (`app/viewer/`, `viewer-boundary-check.mjs`); and CodeMirror 6,
> only in the Ideas companion bundle (`app/companion/`, its own boundary
> check), because ideas need a real editor and CodeMirror edits markdown as
> text, so the file on disk stays exactly what was typed. A new exception
> names its bundle and its check in this paragraph. A rich-text editor that
> re-serializes markdown is not admitted.

## 5. The screens

### 5.1 The companion window (phases 0, 1, 3)

One window, labelled `companion`, separate from `main`. It can be maximised or
full screen on its own (the owner's ask). One window, with documents in a list
and one open at a time; tabs are a later nicety.

```
┌ Ideas ───────────────────────────────────────────────────────────────── ◻ ✕ ┐
│ ┌──────────────────────┐ ┌───────────────────────────────────────────────┐ │
│ │ + New idea      ⌕    │ │ Jellyfin on the home server      [Source|Read]│ │
│ │──────────────────────│ │ saved 10:42 · Make a plan ▸ · ⋯               │ │
│ │ ● Jellyfin on the    │ │───────────────────────────────────────────────│ │
│ │   home server   2h   │ │ # Jellyfin on the home server                 │ │
│ │   Plan: Jellyfin  1d │ │                                               │ │
│ │ Phone pairing UX  3d │ │ Free writing. Lists, links, code.             │ │
│ │ Docs search      2w  │ │                                               │ │
│ │                      │ │ - transcode on the GPU?                       │ │
│ │                      │ │ - which box                                   │ │
│ │                      │ │                                               │ │
│ │ Show in Finder       │ │                                               │ │
│ └──────────────────────┘ └───────────────────────────────────────────────┘ │
└────────────────────────────────────────────────────────────────────────────┘
```

- The list is newest-edited first. A plan sits indented under its idea.
- `Source | Read` flips between CodeMirror and the rendered view. A wide window
  could show both side by side; that is a phase 1 detail.
- `⋯` holds Rename, Remove (asks), Show in Finder.
- A plan document adds a strip under its title: its projects, each with a
  state (registered, to clone, to create), and "Make workspace ▸" (phase 3) or
  "Open workspace ▸" once it has one.

### 5.2 The workspaces view in the main window (phases 2, 3, 4)

A switch at the top of the nav: **All projects | Workspaces**. All projects is
today's nav, unchanged. Workspaces swaps the nav for workspace groups. The
right side (terminal, dock) is unchanged: selecting a place works exactly as
today.

```
┌ worktrees ─────────────────────────────────────────────────────────────────┐
│ [All projects|Workspaces]  │  Jellyfin                                     │
│────────────────────────────│  plan: Plan: Jellyfin ▸   idea ▸              │
│ ▾ Jellyfin              ●  │──────────────────────────────────────────────│
│   ▾ homelab-ansible        │  Projects                                     │
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

- Under a workspace, each project shows only the places this workspace started
  plus `(main)`, with a "show all places" toggle. This is choice C8; the other
  answer is "every place of every referenced project".
- The right side shows the workspace overview when the workspace row itself is
  selected, and the place as today when a place is selected.
- "plan ▸" and "idea ▸" open the document in the companion window.
- In time this may become the default view (the owner's words). The switch
  makes that one setting later, not a redesign.

## 6. The phases

Every phase ends with a check in the real app, by hand, through
`app/scripts/sandbox.sh --app`, because nothing in CI runs the app
(**checked**, AGENTS.md). Each check runs with the ideas and workspaces
overrides of §3 pointing at a scratch folder, and its own tmux server
(`env -u TMUX TMUX_TMPDIR=<short dir>`). Never David's running app.

### Phase 0: the companion window

**David can:** open an Ideas window from the main window (a button and a
shortcut), maximise it or make it full screen on its own Space, close it, and
reopen it where it was. Closing it does not quit the app. It shows "No ideas
yet" and is connected to the engine.

**Built:**

- A second window, label `companion`, created on demand by the shim (not
  declared at launch). Today there is one window, `main`, declared in
  `tauri.conf.json` with no label (Tauri's default is `main`), and no code
  creates another (**checked**: no `WebviewWindowBuilder` under
  `app/src-tauri`).
- A capability for it. `capabilities/default.json` names only `main`
  (**checked**). The companion gets its own capability file with only what it
  needs (core, and the opener for Show in Finder), not a copy of main's.
- Frame memory. `winstate.rs` restores only `main` (**checked**: line 282).
  It learns a second label.
- The page: `app/companion/` with its own entry and its own Vite config, built
  into `app/dist/` as a second HTML page (§4.3), and a boundary check.
- The engine door. The companion is one more loopback client of serve: it calls
  `engine_connect` and opens its own socket (**checked** that this is how
  `main` connects). It **never starts serve**: its reconnect policy runs
  reconnect-only (`start: false` always). `main` owns starting and restarting,
  so two windows never both restart a lost engine. `reconnect.ts` is pure and
  injected (**checked**), so this is a parameter, not a fork of the rule.
- Window code that says "main" today, and must say which window it means:
  `raise_window` (**checked**: `lib.rs:258`), the reopen at `lib.rs:451`,
  `feedback.ts:123` (**checked**: it already returns early outside `main`).
- `ipc.ts` stays the one door; the companion imports it like `main` does.

**Left out on purpose:** documents, any editor, any store.

**Thrown away if we stop here:** the window, its capability, its bundle and
the label in winstate. Small.

**Checked in the real app:** open, full screen, exit full screen, close,
reopen at the same frame; quit with ⌘Q while both are open and relaunch (both
frames back? the companion only if it was open); kill serve by pid once and
see the companion wait while `main` restarts it (one restart, not two, per
serve.log); hide `main` and confirm the companion still reconnects. Space
checks a minute apart (three starts in a minute is a crash loop by
`reconnect.ts`'s rule).

**Size:** S–M. Multi-window Tauri is new to this app, and every "main" in the
shim is a question.

### Phase 1: documents

**David can:** create an idea, see the list, open one, write in it, rename it,
remove it. It saves on its own. Opening the folder in Finder shows plain `.md`
files he can also edit elsewhere.

**Built:**

- A core store, `worktrees_core::ideas`: the folder of §3, one file per
  document, the frontmatter of §2.1, `_at` forms for tests, the guard of §3,
  atomic writes through `fsx::write_atomic` (AGENTS.md: never hand-roll a temp
  name).
- Console-only serve rows (`HostConsole`, choice C7): `ideas_list`,
  `idea_read`, `idea_create`, `idea_write` (with `expected_mtime`, as
  `write_file` already has, **checked**: `files.rs:1390`), `idea_rename`,
  `idea_remove`. Named here for size; the lane names them for real.
- A change event (`ideas:changed`) so a file edited outside the app shows up,
  with the same "the file changed under you" handling the dock editor has.
- The companion's list and editor (§5.1): CodeMirror 6 with the markdown
  language, a rendered view from the viewer's source, the boundary check.
- Remove moves the file to the system Trash rather than deleting it
  (**assumed** possible from serve; if not, a `.trash/` folder inside ideas).

**Left out on purpose:** plans, links between documents, anchors,
conversations, diagrams (a mermaid fence shows as code in Source and is drawn
in Read only if reusing `Mermaid.tsx` costs nothing), search beyond a title
filter, tabs.

**Thrown away if we stop here:** the editor bundle and the rows. The `.md`
files stay and are useful without the app; that is the point of plain files.

**Checked in the real app:** create three ideas, edit, rename (file name and
heading both change, `id` does not), remove one; edit a file in another editor
while it is open and see the app notice; save with no change and confirm the
file's bytes and mtime did not move (`shasum` before and after); a document
with frontmatter, an HTML comment, a table and a mermaid fence survives an
edit elsewhere in the file byte for byte (diff).

**Size:** M.

### Phase 2: the workspaces view (parallel with 1)

**David can:** switch the main window's nav to Workspaces, make a workspace,
name it, add and remove registered projects, see each project's places under
it, and switch back to All projects with nothing changed.

**Built:**

- A core store, `worktrees_core::workspaces`: `workspaces.json` of §2.3,
  `_at` forms, the guard of §3.
- Console-only rows: `workspaces_list`, `workspace_create`,
  `workspace_rename`, `workspace_remove`, `workspace_set_projects`.
- The switch and the grouped nav (§5.2), built from the same place rows as
  today's nav; the right side unchanged.
- The rename of the UI word "workspace" to "All projects" where it means every
  project (choice C6). A sweep of user-facing strings, not of code names.

**Left out on purpose:** plans, `started`, cloning or creating projects,
workspace context documents.

**Thrown away if we stop here:** the switch, the grouped nav, the store. The
All projects view is today's nav, so nothing else moves.

**Checked in the real app:** make two workspaces sharing a project; select a
place through a workspace and through All projects (the same terminal); remove
a workspace and confirm `projects.json` and every place are untouched (diff
`projects.json` before and after); unregister a project that a workspace
references and see "not registered".

**Size:** M.

### Phase 3: idea → plan → workspace

**David can:** in an idea, press "Make a plan" and get a plan document from a
template, linked both ways; fill in its Projects and Work; press "Make
workspace" and get a workspace named after the plan, referencing the projects
that are registered and listing the rest as "to clone" or "to create"; open
the plan from the workspace and the workspace from the plan.

**Built:**

- `plan_of` links and the `idea:` link form (§2.1, §2.2).
- A small reader for a plan's `## Projects` and `## Work` sections, which says
  which line it could not read and never blocks showing the document.
- `plan` on a workspace (§2.3). One plan, at most one workspace; making a
  second workspace from the same plan opens the first.
- The plan strip in the companion (§5.1) and the overview's Projects and Work
  lists in the main window (§5.2), read-only here.

**Left out on purpose:** cloning, creating, starting places. Claude drafting.

**Thrown away if we stop here:** the reader and the links. The plans are
still documents.

**Checked in the real app:** the Jellyfin example end to end up to the
workspace; a plan with a typo in `## Work` shows the line and the document;
renaming the idea keeps the plan's link.

**Size:** M.

### Phase 4: projects and places from a workspace

**David can:** from the workspace overview, clone a "to clone" project or
create a "to create" one, after which it is registered and referenced; open a
work item, read and edit its brief, and press Start, which creates the place
with that brief and launches its agent; see which items were started and
whether each place is still there, busy, or gone.

**Built:**

- Clone through the existing `clone_project` flow (**checked** row). Create
  through a new console row that runs `git init` at a path and then
  `add_project` (**checked** row). Both refuse a path under `~/Documents`,
  `~/Desktop` or `~/Downloads` with a warning, as §3 says.
- A brief on the create path: a brief argument on `new_place` or a sibling row
  calling the same core path as `worktrees new --brief` (§2.4: today's row has
  none). It is drive-class; console-only for now.
- The review step: the brief shown as the exact text that will be written,
  editable, then Start.
- `started` records (§2.3), written after the create succeeds, and the live
  state beside each record from the snapshot the nav already has.

**Left out on purpose:** any return path from finished work into the idea
(v2's Harvest), conversations, starting several items at once, MCP.

**Thrown away if we stop here:** nothing extra; this is the end of the first
pass.

**Checked in the real app:** in a scratch project, start one place from a
plan item; confirm `.planning/brief.md` holds the reviewed text and that the
agent opened on it (the opener goes out only on a first launch, AGENTS.md;
check the pane); remove the place and see "place removed" with the record
kept; create a project at a scratch path and see it registered. Any scratch
Claude runs with `DISABLE_AUTOUPDATER=1` and the real `XDG_DATA_HOME`
(AGENTS.md), which is why §3 gives the stores their own override.

**Size:** M–L.

## 7. Later, and the doors kept open

None of this is in the first pass. Each one is still possible because of a
choice above.

- **Conversations on part of a document.** Anchors as comments in the file
  (§4.2), written only when something attaches. Where the conversation runs is
  open: a Claude in a scratch place, or the companion's own chat view. The
  chat view that exists for places (`chat-session-view.md`) is the obvious
  starting point.
- **Diagrams.** Mermaid fences, drawn in the Read view from the viewer's
  `Mermaid.tsx`, inside the companion bundle. Needs the AGENTS.md exception
  line for mermaid to name a second bundle.
- **Claude drafting a plan from an idea.** A conversation, so after the first.
- **Quick capture** (a global shortcut, the phone, a share sheet). v2's
  capture doors, still deferred.
- **Threads and Harvest** (v2 §§6–8): the return path from finished work into
  the idea. The `started` record is the hook it would hang on.
- **MCP tools and paired devices.** An idea has no project, and every serve
  row's capability is scoped to projects (**checked**, AGENTS.md). Until there
  is a rule for an object with no project, ideas and workspaces are
  console-only. I agree with (main)'s lean.
- **Moving the documents folder** (a setting), and **tabs** in the companion.

## 8. The stop rule

Each phase ships behind nothing; it is just the app. A phase that is not used
is removed, not extended. Concretely, after each phase David uses it for two
weeks of normal work, then:

- **After phase 1:** if fewer than three ideas were written in the companion
  (rather than in his notes app), stop. Keep the `.md` files, remove the
  window and the rows.
- **After phase 2:** if he did not switch to Workspaces at least once a week,
  remove the view. Phases 3 and 4 then do not start, because a plan has
  nowhere to go.
- **After phase 3:** if no plan became a workspace, stop; plans stay as
  documents.
- **After phase 4:** if the places for a workspace were started by hand
  anyway, remove the Start path and keep the rest.

"Removed" means a PR that deletes the phase's code, rows and checks, the same
week. The files the user wrote are never deleted by a removal.

## 9. Choices for David

Recommended answer first.

- **C1. What "canvas" means.** (a) A blank document per idea, which is what
  this plan builds. (b) A spatial board of cards. I read "a blank document
  canvas and creation" as (a). Please confirm; (b) is a different product.
- **C2. The editor.** (a) CodeMirror 6 on the source with a rendered view, in
  the companion's own bundle (§4). (b) Tiptap 3 with its markdown extension,
  accepting that saves re-print the file. (c) A plain textarea first, an
  editor later.
- **C3. Where documents live.** (a) `~/.local/share/worktrees/ideas/`. (b)
  `~/.worktrees/ideas/` (easy to find; collides with a `$HOME` repo's places).
  Workspaces in `~/.config/worktrees/workspaces.json` either way.
- **C4. What a plan is.** (a) Its own document linked to the idea. (b) A
  `## Plan` section inside the idea.
- **C5. Who writes the first plan.** (a) David, from a template, in the first
  pass. (b) A Claude drafts it, which pulls conversations into the first pass.
- **C6. The word.** (a) "All projects" in the UI for today's meaning, code
  names unchanged. (b) Keep "Workspace" for all projects and call the new thing
  something else (a "set", a "stream").
- **C7. Reach.** (a) Console-only: no MCP tools, nothing for paired devices or
  the phone until there is a rule for an object with no project. (b) Read-only
  MCP for ideas from phase 1, so agents can read them.
- **C8. Which places a workspace shows.** (a) The ones it started, plus
  `(main)`, with "show all". (b) Every place of every referenced project.
- **C9. The AGENTS.md paragraph** in §4.4: approve, change, or hold until
  phase 1 merges.

## 10. What I could not establish

- **CodeMirror 6 in this app's WKWebView.** Assumed to work from its Safari
  support; not measured. Phase 1's first task.
- **A second Tauri window loading a second page from `frontendDist`, built by
  a separate config.** Assumed; the viewer is served by `docserver` to a
  browser, not loaded in a Tauri window, so it is not proof. Phase 0's first
  task.
- **How a second window interacts with** ⌘Q, the updater's relaunch, the
  app-owned serve's idle stop (it counts console connections, so a companion
  holds serve up; assumed harmless), macOS full screen on its own Space, and
  the "hidden window defers retries" rule when only one window is hidden.
- **Bundle sizes.** The table has npm unpacked sizes, not built bundles.
- **Moving a file to the Trash from serve** (a process the app started),
  including on Linux.
- **Whether `~/.local/share/worktrees` exists on every machine.** (main)
  checked this Mac; the store creates it either way.
- **Whether BlockNote's markdown export is lossy** in its current version; I
  rejected it on licence, so this does not change the pick.
- **The `## Work` format.** The YAML-ish item shape in §2.2 is a sketch; the
  phase 3 lane should try it on two real plans before fixing it.
