---
title: Workspace chats and a workspace's memory
---

# Workspace chats and a workspace's memory

**Proposal, 2026-11-24. Nothing built.** A design lane's answer to the owner's
ask of 2026-11-23:

> "Workspaces need chats that aren't tied to a project. These would be
> 'workspace level' chats that know about the projects and can develop their
> own memory/history. Ideally as the workspace chats happen/created/closed the
> information in those chats becomes part of the working memory for the
> workspace. I know we can tie an idea or set of ideas to a workspace but I
> feel like there should be a similar thing for 'its working memory'. IDK what
> that would look like but let's see if we can come up with something. It
> isn't an idea per se, but it's kind of like that."

Marks used below: **checked** means I read the code or ran it. **guess** means
I did not. §8 lists what is still unknown.

## 0. The answer, on one page

**A workspace gets one memory document, and a workspace chat is a conversation
on that document.**

- **The memory** is a markdown document of a third kind in the ideas folder,
  beside ideas and plans. Its frontmatter says `memory_of: <workspace id>`, the
  way a plan's says `plan_of:`. One per workspace. It is short, current, and
  written mostly by agents. A person can read and edit it in the Ideas editor
  at any time.
- **A workspace chat** is an idea conversation whose document is that memory.
  So it runs where idea conversations run today: Claude, in its own tmux
  session `worktrees~idea~<conv>`, with the ideas folder as its working
  directory. It is never a place and belongs to no project. What is new is its
  start file, `--add-dir` for each of the workspace's projects, and later a
  workspace mode of the worktrees MCP server.
- **Closing a chat folds it.** The fold writes a note for that chat (what was
  discussed, decided, left open) into the chat's own folder, and updates the
  memory document. The memory's old text is copied to history first. The
  overview then shows what changed, with Undo.
- **A new chat starts from memory.** Its start file tells it to read the
  memory document first and gives it the list of earlier chats' notes.
- **It reads the projects. It does not work in them.** Work goes to a place.
  The chat writes a work item into the workspace's plan, and the person starts
  it with the Review and Start that phase 5 already shipped.

Why this shape: almost all of it exists. The ideas store gives the file, its
id, its history, its editor, its change event and its MCP tools. Idea sessions
give the launch, the record, resume, the chat view, the dots and the
live-agent limit. The owner's own words fit it: "it isn't an idea per se, but
it's kind of like that." A plan is already exactly that: a document in the
same folder that is not an idea.

```
 workspaces.json                         ideas folder
 ┌──────────────────────┐                ┌───────────────────────────────────────────┐
 │ Jellyfin             │   memory ────▶ │ memory-jellyfin.md   (memory_of: <ws id>) │
 │   plan   ───────────────────────────▶ │ plan-jellyfin.md     (plan_of: [...])     │
 │   ideas  ───────────────────────────▶ │ jellyfin-sharing.md                       │
 │   projects: [roots]  │                │ .history/<memory id>/<time>.md            │
 └──────────────────────┘                │ .desk/<memory id>/sessions.json   chats   │
            │ --add-dir each root        │ .desk/<memory id>/<conv>/start.md         │
            ▼                            │ .desk/<memory id>/<conv>/context.md       │
   tmux worktrees~idea~<conv>  ◀── cwd ──│ .desk/<memory id>/<conv>/note.md   fold   │
                                         └───────────────────────────────────────────┘
```

## 1. What a workspace chat is

**Recommendation: an idea conversation on the workspace's memory document,
launched with the workspace's projects added as readable directories.**

### 1.1 Where it runs

| | |
|---|---|
| Harness | Claude only, at first. The idea launcher runs Claude only and needs `--session-id` (**checked**, `ideasessions.rs` `launch_line`, `session_id_floor`). |
| Working directory | The ideas folder. One folder-trust answer, one Claude project directory for every transcript, and `check_root` already refuses an ideas folder inside a registered project (**checked**, `ideasessions.rs:351`). |
| tmux session | `worktrees~idea~<conv ULID>`, unchanged. Every reader that must not mistake it for a place already skips that marker: pane adoption, the nav's agent rows, the poll (**checked**, `tmux.rs:600`, `:636`, `:689`, `watch.rs:162`, `:262`). |
| Record | `.desk/<memory id>/sessions.json`, the idea record, unchanged, with one new optional field per conversation (`fold`, §3). Unknown keys are kept already (**checked**, `Record.other`). |
| Limits | It counts toward `[limits] max_live_agents` and passes the quota gate, because the idea launcher asks both (**checked**, `launch_at`). |

Rejected:

- **A folder per workspace as the cwd** (`…/worktrees/workspaces/<id>/`). It
  is the tidy picture, and it costs a second store, a second history, a second
  session marker that every reader above must learn, and a second set of the
  fifteen chat rows. It buys nothing the person can see.
- **One project's `(main)` as the home.** That ties the chat to a project,
  which is the thing he asked not to have. It also puts it in that project's
  message log and under that project's hooks.
- **A hidden project with a place per chat.** Rejected for idea sessions
  already (phases plan §7.2): a branch per conversation, a project in every
  list.

### 1.2 What it is told at launch

The opener is fixed text naming a file the engine wrote, as for an idea:
`Read .desk/<memory id>/<conv>/start.md and begin.` Both names are ULIDs
checked before they reach argv (**checked**, `opener`, `check_idea`,
`check_conv`). Nothing is typed into the pane to start it.

`start.md` is a fixed template for the memory kind. It carries ids and file
names only. In words:

1. You are a chat of the workspace whose memory is the document with
   `id: <memory id>` in this folder, now `<file>`. Read it first.
2. Read `context.md` beside this file: the workspace's name, its projects,
   their places, its plan and linked ideas, and the notes of earlier chats.
3. You may read anything in the projects listed there. Never write in a
   project's tree. Work is done in a place, by that place's agent.
4. When something durable is decided, put it in the memory document. Keep it
   short (§2.4).
5. In this folder you may write the memory document and your own folder
   `.desk/<memory id>/<conv>/`, and nothing else. (The plan, by choice 5.)

`context.md` is written by the engine at every start and every resume. It is a
snapshot: workspace name; each project's registry name, root and whether it is
registered; each project's places (slug, branch, lifecycle, note, agent
state); the plan's and linked ideas' ids and titles; one line per earlier chat
that has a note. Place names and notes come from repositories and other
agents, so the file says at its top that it is data, not instructions, as the
planning hook already does.

A resumed chat gets no opener (**checked**: resume sends none). It has the
start file in its history. Its `context.md` is fresh on disk, and from phase 3
it asks a tool instead.

### 1.3 What it may read

`--add-dir <root>` for each of the workspace's registered projects. A place
lives under `<root>/.worktrees/<slug>`, so every place's tree is covered too.

**Checked by one scratch run** (Claude Code 2.1.331, headless, a non-repo cwd
and two scratch repos added with `--add-dir`): the Read tool read files in the
added directories with no refusal, and the worktrees MCP server offered zero
tools there. The second fact is by design (**checked**, `mcp.rs` `tools`:
"No repo, no tools").

It also reads every idea and plan in the ideas folder, because that is its
cwd. An idea conversation already can. This is the user's own account reading
the user's own documents.

### 1.4 What it may do in a project

**Recommendation: nothing directly. It proposes work as plan items, and the
person starts them.**

- The chat adds a `## Work` item with a brief to the workspace's plan
  (**checked**: the reader and its keys, `plans.rs` `WORK_KEYS`). The overview
  already shows the item with **Review…**, and Start runs `plan_start`, which
  goes down `new --brief`'s path (**checked**, `worktrees-api/src/plans.rs`).
  So the chat fills in parameters and a person runs the act. That is the
  phases plan's own rule (§7.1), reused whole.
- No new way to start a place is built for the chat. A workspace with no plan
  gets one from the existing `plan_make` when the chat first proposes work, on
  the person's click.
- Later, and only as the owner's choice (7.4): with `cross_project = "full"`
  the chat's MCP server could offer `create_worktree {project}` for the
  workspace's projects, as a `(main)` server does today (**checked**, `mcp.rs`
  tool text: "Needs cross_project = full").

### 1.5 How that sits with the cross-project setting and host policy

- **Reading.** `cross_project` is off unless the user turns it on, and decides
  what a session in one project may see of the others (**checked**,
  `reach.rs`). A workspace chat is in no project, so the key does not describe
  it. My pick: **making a workspace, putting projects in it and starting its
  chat is the consent to read those projects, for that chat only.** It reaches
  the workspace's projects, never the whole registry. This is choice 3.
- **A `private` project** (**checked**, `registry.rs:96`) stays private: it is
  named in `context.md` and is not added with `--add-dir`. The overview says
  so on its row.
- **Acting** follows the existing key. Nothing acts in another project below
  `full`, and in the first phases nothing acts at all.
- **Host policy.** The live-agent limit and the quota gate apply as for an
  idea session. No device launches one, so there is no `launched_by` to stamp.
- **ADR 0001** holds. Nothing a repository supplies reaches argv: the opener
  is fixed, and `--add-dir` carries registry roots, which are the user's.

### 1.6 What it reuses from idea conversations, and what it cannot

| Reused as is | Changed |
|---|---|
| The record, `prepare_at` (with its history copy of the document as the chat starts), rename, remove | `start_text`: a memory template, chosen by the document's kind |
| `launch_at`: session name, `--session-id`, resume, limits, prompt plugin | The launch gains `--add-dir` words; `AiLaunch` has no field for them today (**checked**: no `add-dir` anywhere in `crates/`) |
| Every chat row: `idea_turns`, `idea_prompt`, `idea_answer*`, `idea_send`, `idea_interrupt`, `idea_term_open` (**checked**, dispatch.rs 1345–1364) | Close: a workspace chat's close folds first (§3) |
| `Locus::Idea`, the per-conversation lock, `ChatPane`'s idea target | The MCP server: zero tools in this folder today; phase 3 gives it a workspace mode |
| `ideas:sessions` (live, busy, waiting, `idea_of`) | The main window must map a memory id to its workspace; `workspaces.json` has the link |
| `IdeaChat` in the Ideas window, which will list a memory's chats with no change | `context.md` is new |

No fourth path that types into Claude is added. Sending a line is `idea_send`,
the console-only door the owner approved on 2026-11-11.

## 2. What the working memory is

**Recommendation: one markdown document per workspace in the ideas folder, of
kind `memory`, plus one note per folded chat in that chat's own folder.**

### 2.1 Two layers

| | The memory | A chat note |
|---|---|---|
| What | What is true and current for this workspace | What one chat discussed, decided and left open |
| File | `<ideas>/memory-<slug>.md`, `memory_of: <workspace id>` | `<ideas>/.desk/<memory id>/<conv>/note.md` |
| How many | One | One per folded chat |
| Size | Bounded (§2.4) | Not bounded; never loaded wholesale |
| Changes | Rewritten in place, a history copy before each change | Written at the fold; a second fold of the same chat replaces it |
| A new chat gets | All of it | Its title and date in `context.md`; it reads one when it needs it |

This is the shape Claude Code's own memory has (a short index that is always
loaded, detail in files read on demand) and the shape `/compact` has (a chat
reduced to a summary). Borrowed, not copied.

### 2.2 The document

```markdown
---
id: 01JD4...
memory_of: 01JC91...
created: 2026-11-24
---

# Memory: Jellyfin

## What this workspace is for
One paragraph.

## Projects
- homelab-ansible: the roles. Deploys go through `make deploy`.
- jellyfin-config: generated, never edited by hand.

## Decisions
- 2026-11-24 Media lives on the NAS, not the server. (chat: Storage layout)

## How we work here
- Test a role in the scratch VM before the server.

## Open
- Who can see the kids' library?
```

The headings are a template, not a schema. The app parses none of them. The
kind is decided by one fact, the `memory_of:` key, exactly as `Kind::of`
decides a plan by `plan_of:` (**checked**, `ideas.rs`). The workspace holds
the other end: `"memory": "<id>"` in `workspaces.json`, beside `plan` and
`ideas`. An older build keeps a key it does not know (**checked**,
`Workspace.extra`). One memory has at most one workspace, the rule
`set_plan_at` already enforces for plans.

### 2.3 Who writes it, and when

| Who | When | How |
|---|---|---|
| The app | Once, when the workspace's first chat starts or the person clicks Make memory | The template above. After that the app never rewrites the text |
| The agent | During a chat, when something durable is decided | Phase 1: it edits the file. Phase 3: `memory_append` and `memory_write`, which take `expected_mtime` and copy the old text to history first (`write_kept_at`, **checked**) |
| The agent | At the fold (§3) | The same, preceded by a history copy the engine takes itself |
| The person | Any time | The Ideas editor. It is a document |

History is the store's own: `.history/<memory id>/<time>.md`. A copy is taken
when each chat starts (**checked**: `prepare_at` does this for any document),
before each fold, and before each tool write. The Versions list and Restore
exist already (`idea_versions`, `idea_restore`, **checked**).

### 2.4 How it stays bounded

- A budget of 16 KB, about 250 lines. A guess at a good number; it should hold
  what a new chat can read in one go.
- The engine never cuts the text. Over budget, the overview says "Memory is
  long (23 KB)" with **Tidy…**, which starts a chat on a fixed tidy file:
  move old decisions into one archive note, keep one line each.
- The fold's instructions say the same thing: detail goes in the note, one
  line goes in memory.

### 2.5 How it differs

| | Whose | About | Lifetime | Given to |
|---|---|---|---|---|
| An idea | The person's thinking; an agent changes it only when asked | Anything | Until removed | Nobody, unless opened |
| A plan | The person's intent; the app reads Projects and Work | Work to start | A record after the work | The overview; a work item's brief is copied to a place |
| A place's `.planning/` | One place's agent | One branch | Dies with the place; archived at close-out | That place's agent |
| **The memory** | The workspace's; agents write it, the person corrects it | What is true across the workspace's projects | As long as the workspace | Every new workspace chat, at start |

### 2.6 Why reuse the ideas store

Reuse it. The memory needs an id that survives a rename, bytes that are never
re-printed, a history copy before every change, an editor, a change event and
a way for an agent to write it with a stale-write check. The ideas store has
all seven (**checked**, `ideas.rs` header). A separate store would rebuild
them and add a second folder to back up.

What it must not inherit: a memory is not an idea in the lists. It gets its
own group in the Ideas window. Merge, combine, split, "make a plan from" and
"link as an idea" refuse a memory, as `link_idea_in` refuses a plan today
(**checked**). It is removed only from its workspace.

Rejected: memory as JSON in `workspaces.json` (not prose, not editable, no
history); memory as a section of the plan (a plan is intent and becomes a
record; memory must stay current); memory as an ordinary linked idea (the app
could not tell it from the person's own writing, so it could not hand it to
chats or guard it).

## 3. The lifecycle

**Recommendation: close folds by default, applies the result, and shows what
changed with Undo.**

| Act | What happens |
|---|---|
| **New chat** | Makes the memory if the workspace has none. Then the idea start: history copy of the memory, the conversation's folder, `start.md`, `context.md`, the record with the session id, the launch. A refused launch leaves nothing behind (**checked**, `start_at`) |
| **Resume** | Rewrites `context.md`, then `--resume '<session id>'` under the same tmux name, no opener (**checked**) |
| **Close** | The fold (below), then the session ends. **Close without folding** is in the menu |
| **Reopen** | Resume. A reopened chat that is closed again is folded again, and its note is replaced, not doubled |
| **Delete** | Asks first. Ends the session, removes the record entry and the chat's folder, note included. What the chat added to memory stays; it is memory now. Claude's transcript is Claude's file and stays (**checked**, `remove_at`) |
| **Workspace removed** | Its live chats are closed without folding. The memory document and the notes stay in the ideas folder, listed as "Memory (no workspace)". Nothing is deleted |

### 3.1 What the fold is, concretely

1. Refused while the agent is working or waiting on a prompt, in words: "it is
   working: stop it or wait".
2. The engine copies the memory to history and writes
   `.desk/<memory id>/<conv>/fold.md`, a fixed template: write `note.md` here
   with these four headings (Asked, Decided, Done, Open); then update the
   memory document, one line per durable fact, nothing that is only this
   chat's detail; say in two lines what you changed.
3. The engine ends the session if it is live and launches the same
   conversation again as a resume **with a fixed opener**:
   `Read .desk/<memory id>/<conv>/fold.md and do what it says.`
4. The record gets `fold: {asked, memory_before}`. When `note.md` exists and
   the agent has been idle for one poll tick, the poll ends the session and
   stamps `fold.done`. With no note after five minutes it is `fold.failed`,
   the session is left open, and the chat row says so.

Why a relaunch and not a typed line: an opener reaches only a session the
launch creates (AGENTS.md, the re-brief rule), and it is how every brief
already reaches Claude. It works the same for a chat whose Claude already
exited. It types nothing, so the "no fourth path" rule is not touched. The
cost is a few seconds. The alternative, typing a fixed line through
`idea_send`'s body into the live session, is faster and is a new use of the
typing door; it is choice 2.

Rejected: a headless `claude -p --resume` for the fold. Print mode answers
permission questions with no person there, and nobody can see or stop it.

### 3.2 When the fold is wrong

- **Undo** on the overview's "memory changed" line restores `memory_before`
  through `idea_restore`, which itself copies the current text first. Nothing
  is lost in either direction.
- **Edit** the memory by hand.
- **Reopen** the chat, say what is wrong, close again. The note is replaced.
- The note is separate from the memory, so a bad memory edit never loses the
  chat's record, and a bad note never touches memory.

### 3.3 What a new chat is given from memory

The whole memory document (it is bounded), by instruction in `start.md`, and
one line per earlier note in `context.md`. Nothing is pasted into argv or
into the pane.

## 4. The surfaces

**Recommendation: its own beta flag, `beta_workspace_chats`. The overview and
the nav in the main window are its home. The Ideas window gets it for free.**

### 4.1 The beta flag

One entry in `BETA_FEATURES`, its own `beta_workspace_chats` key, its own
checkbox and helper (AGENTS.md; **checked**, `beta.ts`). The workspace
overview is drawn only under Ideas' flag today (**checked**, `App.tsx:6724`).
So the overview page must draw when either flag is on, and each block asks its
own helper: plan and ideas ask Ideas', memory and chats ask this one. With
Ideas off, the memory is shown rendered and read-only in the main window,
with a line saying the Ideas beta has the editor.

### 4.2 The workspace overview

```
 Jellyfin
 ──────────────────────────────────────────────────────────────────────
 memory   Memory: Jellyfin ▸      4.1 KB · changed 2 h ago by "Storage layout"
          ┌ 3 lines added, 1 changed ─────────────── View ▸   Undo ┐
 CHATS                                                  [ New chat ]
  ●  Release order            working            started 14:02     ▸
  ◐  Who can see what         waiting on you     started 11:40     ▸
  ○  Storage layout           closed · folded    yesterday         ▸
  ○  First look               closed · no note   2026-11-20        ▸
 ──────────────────────────────────────────────────────────────────────
 plan     Plan: Jellyfin ▸  ×
 ideas    Jellyfin sharing ▸ ×      [ Link an idea ▸ ]
 PROJECTS (FROM THE PLAN) …
 WORK (FROM THE PLAN)
  Ansible role for Jellyfin   homelab-ansible · jellyfin-role
                              proposed by "Release order"   [ Review… ]
```

Clicking a chat opens it in the main area, in place of the overview:

```
 ‹ Jellyfin    Release order ▾            Chat | Terminal     [ Close ▾ ]
 ──────────────────────────────────────────────────────────────────────
   (the chat view: ChatPane on an idea target)
 ──────────────────────────────────────────────────────────────────────
  [ Message…                                                       ⏎ ]
```

`Close ▾` holds Close (folds), Close without folding, Delete…. An ended chat
reads read-only with Resume, as an ended idea conversation does (**checked**,
`idea_turns` answers `ended: true`).

### 4.3 The nav

In the Workspaces view, a workspace's card lists its live chats above its
projects. Closed chats are only in the overview.

```
 ▾ Jellyfin                    ◐
     ● Release order
     ◐ Who can see what
     homelab-ansible
       (main)
       jellyfin-role        ●
     jellyfin-config
```

The card's dot is amber when any of its chats waits on the person, from
`ideas:sessions`' `waiting` and the memory-to-workspace link. All projects is
unchanged: a workspace chat belongs to no project.

### 4.4 The Ideas window

- The list gets a third group, **Workspace memory**, under ideas and plans.
- A memory opens in the same editor, with Versions.
- The chat panel on the right lists the memory's conversations. This needs no
  work: it is `idea_conversations` on that id.
- A chat's note opens as a read-only document from the chat's menu.

## 5. The engine

**Recommendation: four new console rows, one new object, no new event in the
first phase. Everything else is the idea rows, addressed with the memory's
id.**

### 5.1 Files

| File | Written by |
|---|---|
| `workspaces.json`: `memory` on a workspace | `workspaces::set_memory_at`, the twin of `set_plan_at` |
| `<ideas>/memory-<slug>.md` | The store (`create`), then the agent and the person |
| `<ideas>/.history/<memory id>/…` | The store |
| `<ideas>/.desk/<memory id>/sessions.json` | `ideasessions`, with the optional `fold` field |
| `<ideas>/.desk/<memory id>/<conv>/start.md`, `context.md`, `fold.md` | The engine, `create_new` or `fsx::write_atomic` |
| `<ideas>/.desk/<memory id>/<conv>/note.md` | The agent |

### 5.2 Rows

All `HostConsole`, so no device grant reaches them (**checked**: the idea rows
are declared this way). A new object `workspace(id)`: an id `workspaces.json`
knows. Today's workspace rows declare no object (**checked**, dispatch.rs
1431–1459).

| Row | Objects | What |
|---|---|---|
| `workspace_memory` | `workspace(id)` | The memory's id, title, file, mtime, size, over budget, and the last fold's before and after. Read only |
| `workspace_memory_make` | `workspace(id)` | Creates it from the template and links it. Refused when the workspace has one |
| `workspace_chats` | `workspace(id)` | The memory's conversations with state, plus note present and fold state |
| `workspace_chat_start` | `workspace(id)` | Makes the memory if absent, then the start of §3 |
| `workspace_chat_fold` | `idea_conv(conv, id)` | §3.1. Phase 2 |

Unchanged rows that serve a workspace chat when given the memory's id:
`idea_conversation_resume`, `_close` (close without folding), `_remove`,
`_rename`, `idea_turns`, `idea_turn_text`, `idea_tool_call`, `idea_prompt`,
`idea_answer`, `idea_answer_text`, `idea_answer_many`, `idea_send`,
`idea_interrupt`, `idea_term_open`, `idea_versions`, `idea_restore`. The one
change under them: `launch_at` and `prepare_at` ask the document's kind, to
pick the template and add the directories.

### 5.3 The MCP server's workspace mode (phase 3)

Today a server in the ideas folder has no project and offers nothing. Phase 3
teaches it one more start: the launch names the conversation, the server
checks that it is a conversation of a memory document whose workspace exists,
and pins that workspace as it pins a repo today ("no tool takes a repo path").

| Tool | Tier | What |
|---|---|---|
| `workspace` | read | Name, projects, each project's places with agent state: `context.md`, live |
| `place_status {<project>:<slug>}` | read | As today, for the workspace's projects only |
| `ideas_list`, `idea_read`, `idea_related` | read | As in every server |
| `memory_read` | read | The pinned memory, with `mtime_ms` |
| `memory_append`, `memory_write` | write | The pinned memory only. `expected_mtime`, history copy first |
| `plan_propose_work` | write | Adds one `## Work` item to the workspace's plan, marked as proposed by this chat. Starts nothing |

No tool takes a memory id or a workspace id. `idea_create`, `idea_append` and
`idea_write` stay `(main)`-only (**checked**, `ideas_writable`).

### 5.4 Events

None new in phase 1. `ideas:sessions` already carries a workspace chat's
live, busy and waiting state and its document id. `ideas:changed` already
fires when the memory moves. The chat list is re-read on either. If the fold's
end proves slow to show, phase 2 adds a `folding` list to `ideas:sessions`.

### 5.5 A paired device, a group-chat guest

Nothing, by default and by construction. Every row is `HostConsole`. Both
events are console events (**checked**, `scope.rs:204`, `:207`). A share is
bound to a lane's session in a place, and a workspace chat is in no place, so
it cannot be shared. Sharing one with a guest would be a decision of its own.

### 5.6 The audit record

The console's non-read rows are recorded as the idea rows are. `idea_send`
keeps its record of length and hash, never the text. `workspace_chat_start`
and `workspace_chat_fold` record the workspace id and the conversation id. The
fold's opener is a constant, so there is no text to hash. Memory text never
reaches the log.

## 6. Phases

Each ends with a hand check in the sandbox app, with `WORKTREES_IDEAS_DIR` at
a scratch folder and its own tmux server.

| # | Phase | At the end the owner can | Size |
|---|---|---|---|
| **1** | A chat and a memory | Start a chat from a workspace's overview. It has read the memory and the projects. It writes durable facts into the memory when told, or when it judges them durable. He edits the memory in the Ideas window. Close, resume, delete | M |
| **2** | The fold | Close a chat and see its note, the memory's change, and Undo. Close without folding. Tidy… | M |
| **3** | The MCP workspace mode | Ask a chat what is running across the workspace and get a live answer. Memory writes go through tools with a history copy each | M |
| **4** | Work from a chat | The chat proposes a work item; it appears in the overview with Review…, and Start makes the place | S–M |
| **5** | Memory for the lanes | A place started from the workspace's plan gets one line in its brief naming the memory's id. Its agent reads it with `idea_read`, which every server has already (**checked**) | S |
| later | | Codex as the chat's harness. Acting directly with `cross_project = "full"`. Messaging lanes from a chat through the MCP bus (a chat is no place, so it has no `from` and no log today) | |

Phase 1 in detail, since it is the smallest thing worth using:

- core: `Kind::Memory`; `workspaces` `memory` field and `set_memory_at`; the
  memory template; `start_text` and `context.md` for the kind; `AiLaunch`
  words for `--add-dir`; the refusals of §2.6.
- api and serve: `workspace_memory`, `workspace_memory_make`,
  `workspace_chats`, `workspace_chat_start`; the `workspace(id)` object.
- app: the flag; the overview's memory line and chats block; a chat in the
  main area on `ChatPane`'s idea target; the nav's chat rows and dot; the
  Ideas window's memory group; the mock's rows.
- Not in it: the fold, any MCP change, any act in a project.

Claude lanes can already be messaged from a chat with Claude's own messaging,
since the chat has a `--name` like every Claude worktrees launches (**guess**:
I did not try it from an idea session).

## 7. Open choices for the owner

1. **Memory lives in the ideas folder as a third kind of document.** My pick:
   yes. The other way is a folder per workspace with its own store.
2. **How the fold reaches the agent.** My pick: relaunch on a fixed opener
   (§3.1), no typing. The other way: type a fixed line into the live session,
   which is faster and widens the typing door.
3. **Does a workspace chat read the workspace's projects when `cross_project`
   is off?** My pick: yes, for that chat and those projects only; the
   workspace is the consent. The other way: it sees names only until the key
   is `read`.
4. **Is the fold applied or reviewed first?** My pick: applied, then shown
   with Undo. A review on every close is a chore, and a chore that is skipped
   means no memory.
5. **May a chat write the workspace's plan?** My pick: yes, from phase 4, and
   only to add a work item marked as proposed. Never before that.
6. **Does Close fold by default?** My pick: yes, with Close without folding in
   the menu.
7. **May a chat start places itself, later?** My pick: only under
   `cross_project = "full"`, and not before the plan-item road has been lived
   with.
8. **Are a private project's files readable by its workspace's chat?** My
   pick: no.
9. **Delete a chat: does its note go with it?** My pick: yes, and the confirm
   says so.
10. **The memory budget.** My pick: 16 KB, said and never enforced by cutting.
11. **The name on screen.** My pick: "Memory" for the document and "Chats"
    for the list. "Working memory" is longer and says no more.
12. **AGENTS.md.** Phase 1's PR should add a paragraph: a workspace chat is an
    idea conversation of the memory kind, and it reuses `idea_send`, so there
    is still no fourth path. I did not edit AGENTS.md.

## 8. What I could not establish

- **What an added directory brings besides files.** If Claude runs the hooks
  or loads the settings of a directory given with `--add-dir`, a cloned
  repository in a workspace would run code in a chat whose folder trust the
  person never gave for it. I believe it does not, and I did not measure it.
  **Phase 1 must measure this first.** If it does, drop `--add-dir` and let
  reads outside the cwd ask, or wait for phase 3's tools.
- **Whether folder trust covers added directories**, or asks again per
  directory, in an interactive session. My run was headless, which asks
  nothing.
- **Instruction files.** In my one headless run the model reported no
  `CLAUDE.md` text in its context, from the added directories or from its own
  cwd. That is one model's report, not a measurement, so nothing above depends
  on it. The design reaches the agent through the opener and `start.md` only.
- **Whether a write into an added directory can be denied by a rule.** A
  `--settings` deny on `Edit` under each root may make "never write in a
  project" a guard and not only a rule. Not tried, and it would not cover the
  shell.
- **`--resume <id>` with a positional opener** in an interactive Claude, which
  the fold needs. The CLI's help suggests it, and `launch_cmd` already puts an
  opener after a resume's words (**checked**, `profile.rs:729`), but the idea
  launcher never sends both and I did not run it. If Claude refuses the pair,
  choice 2 falls to the typed line.
- **How the MCP server learns its conversation** in phase 3: an environment
  variable set at launch, if Claude passes its environment to a user-scope
  server, or a per-conversation `--mcp-config`. Not measured.
- **Two chats writing one memory at once.** I expect Claude's edit tool to
  refuse a stale edit and re-read. Phase 3's `expected_mtime` makes it certain.
- **How long a fold takes** on a long chat, and what it costs. No number.
- **Transcripts that Claude has cleaned up.** A chat whose transcript is gone
  cannot be folded. The note it wrote earlier, if any, is all there is.
- **Overlap with group chat v4's conversations per topic.** I read its
  headings only.
- **The other harnesses.** Whether Codex has an equal of `--add-dir` and of
  `--session-id`. Not looked at.
- **The numbers** in §2.4 and §3.1 (16 KB, five minutes) are guesses.
