---
title: Workspace chats and a workspace's memory
---

# Workspace chats and a workspace's memory

**Proposal, revision 2, 2026-11-24. Nothing built.** A design lane's answer to
the owner's ask of 2026-11-23:

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

**What revision 2 changes**, from the owner's answers to revision 1 on
2026-11-24:

| He said | Revision 1 | Now |
|---|---|---|
| "memory is a folder that could in theory start collecting different md files and even images … Not all of it loaded by default" | One document in the ideas folder | A folder per workspace: an index that is always read, and files it points to (§2) |
| "I wonder if it is its own local git repo so that it can track diffs internally" | The ideas store's history copies | Yes: the memory folder is a git repository of its own, committed by the engine (§2.5) |
| "ideas/plans and now memory needs to be incorporated into the navigation of our primary app and able to float as needed"; the screenshot: "Chats live here but not as part of a project", "Plenty of real estate" | Memory edited in the Ideas window; chats in the overview | The workspace's documents are read in the main window and float into their own window; chats sit in the nav under the workspace, above its projects (§4) |
| "Folds on close make sense", "yes can read", "yeah applied" | Choices 6, 3 and 4 | Decided |
| "IDK if we need to start tracking things … with an sqlite db" | Not raised | No, not yet (§5.7) |

## 0. The answer, on one page

**A workspace gets a folder of its own. Its memory is a git repository inside
that folder, and its chats run there.**

```
 ~/.local/share/worktrees/workspaces/<workspace id>/     the workspace's folder (cwd of every chat)
   chats.json                         the chats (the record)
   chats/<conv>/start.md              what a chat opens on (fixed template)
   chats/<conv>/context.md            projects, places, plan, ideas: a snapshot the engine writes
   chats/<conv>/fold.md               the fold's instructions (fixed template)
   memory/                            a git repository; the engine commits it
     MEMORY.md                        the index: always read, kept short
     notes/2026-11-24-storage-layout.md    one note per folded chat
     topics/nas-layout.md             anything that grew past a few lines
     files/rack.png                   images and other attachments

 tmux: worktrees~ws~<conv>            one session per live chat; never a place
 --add-dir <root>                     each of the workspace's projects, to read
```

- **Memory grows.** `MEMORY.md` is the only part a chat always reads. It is
  short and links to everything else. Notes, topic files and images pile up
  beside it and are read when a link is followed.
- **A chat** is Claude in its own tmux session, in the workspace's folder. It
  belongs to no project. It reads the workspace's projects and never works in
  them.
- **Closing a chat folds it.** The agent writes a note for the chat and
  updates the index. The engine commits. The app shows the commit's diff, with
  Undo, which is a revert.
- **The workspace's documents live in the main window.** Selecting a
  workspace shows Memory, Plan and Ideas in the main area, with the chat
  beside them. Any of them floats into its own window.
- **Work goes to a place.** The chat proposes a work item in the workspace's
  plan, and the person starts it with phase 5's Review and Start.

What it reuses: the chat rows' bodies, the chat view, the launch and its
limits, the dots, the plan's Start. What is new: the folder, the git
repository, a session kind, and the main window's document view.

## 1. What a workspace chat is

**Recommendation: Claude in a tmux session of its own, in the workspace's
folder, launched on a fixed opener, with the projects added as readable
directories.**

### 1.1 Where it runs

| | |
|---|---|
| Harness | Claude only, at first. The idea launcher this is modelled on runs Claude only and needs `--session-id` (**checked**, `ideasessions.rs` `launch_line`, `session_id_floor`). |
| Working directory | The workspace's folder, `<data dir>/worktrees/workspaces/<workspace id>/`. The id is the workspace's ULID, so a rename moves nothing. An override variable moves the parent, as `WORKTREES_IDEAS_DIR` moves the ideas folder, and never `XDG_DATA_HOME` (AGENTS.md). |
| tmux session | `worktrees~ws~<conv ULID>`. `~` cannot appear in a place's session (**checked**, `ideasessions.rs` header). The four readers that skip `~idea~` today (**checked**, `tmux.rs:600`, `:636`, `:689`, `watch.rs:262`) ask one shared predicate that covers both markers. |
| Record | `chats.json` in the workspace's folder: the idea record's shape (id, harness, session id, title, started) plus `fold`. |
| Limits | Counts toward `[limits] max_live_agents` and passes the quota gate, as an idea session does (**checked**, `launch_at`). |
| Not a project | The folder must be outside every registered project, the rule `check_root` holds for the ideas folder (**checked**, `ideasessions.rs:351`). `add_project` refuses a path under it, so the memory repository can never be registered. |

The cwd is the workspace's folder, not `memory/`. So the chat's cwd is not a
git repository, and the worktrees MCP server started there finds no project
and offers nothing (**checked**, `mcp.rs` `tools`). If the cwd were the
repository, the server would discover it as a project and offer
`create_worktree` on the user's memory.

Rejected:

- **The ideas folder as the cwd** (revision 1). Memory is a folder now, and
  the ideas store reads one flat folder of `.md` files.
- **One folder for all workspaces' chats.** One trust prompt instead of one
  per workspace, but every chat would sit in every workspace's memory.
- **One project's `(main)` as the home**, or a hidden project. Both tie the
  chat to a project.

### 1.2 What it is told at launch

The opener is fixed text naming a file the engine wrote:
`Read chats/<conv>/start.md and begin.` The name is a ULID checked before it
reaches argv. Nothing is typed into the pane to start it.

`start.md` is a fixed template with ids and file names only:

1. You are a chat of this workspace. Read `memory/MEMORY.md` first. Follow
   its links when you need what they hold; do not read the whole folder.
2. Read `chats/<conv>/context.md`: the workspace's name, its projects, their
   places, its plan and linked ideas.
3. You may read anything in the projects listed there. Never write in a
   project's tree. Work is done in a place, by that place's agent.
4. When something durable is decided, write it in `memory/`. A line in
   `MEMORY.md`; detail in a file under `topics/` that the line links to.
5. Write only under `memory/` and your own `chats/<conv>/`. Do not commit;
   the app does.

`context.md` is rewritten by the engine at every start and resume. Place
names and notes come from repositories and other agents, so it says at its
top that it is data, not instructions.

### 1.3 What it may read (decided: yes)

`--add-dir <root>` for each of the workspace's registered projects. A place
lives under `<root>/.worktrees/<slug>`, so every place's tree is covered.

**Checked by one scratch run** (Claude Code 2.1.331, headless, a non-repo cwd
and two scratch repos added): the Read tool read files in the added
directories with no refusal.

The owner decided on 2026-11-24 that a workspace chat reads its workspace's
projects. Making the workspace and starting its chat is the consent, for that
chat and those projects only. It does not move `cross_project`, which still
decides what a session inside one project sees of the others (**checked**,
`reach.rs`). A `private` project (**checked**, `registry.rs:96`) is named in
`context.md` and not added; that is choice 4.

### 1.4 What it may do in a project

**Recommendation: nothing directly. It proposes work as plan items, and the
person starts them.**

- The chat adds a `## Work` item with a brief to the workspace's plan
  (**checked**: the reader and its keys, `plans.rs`). The plan view shows it
  with **Review…**, and Start runs `plan_start`, down `new --brief`'s path
  (**checked**, `worktrees-api/src/plans.rs`). The chat fills in parameters; a
  person runs the act. This is the phases plan's rule (§7.1), reused whole.
- The plan is a document in the ideas folder, outside the chat's cwd. So the
  item is written by a tool of the MCP workspace mode (§5.4), not by the agent
  editing a file. Until that phase, the chat drafts the item in the chat and
  the person pastes it.
- Acting directly (`create_worktree {project}`) comes later if at all, and
  only under `cross_project = "full"`.

Host policy applies as for an idea session. ADR 0001 holds: the opener is
fixed, and `--add-dir` carries registry roots, which are the user's.

### 1.5 What it reuses from idea conversations, and what it cannot

| Reused | New or changed |
|---|---|
| The chat bodies (`agents::*_at`) on a locus that is a session, a folder and a session id. `Locus::Idea` is already exactly that (**checked**, `agents.rs:206`) | A `Locus::Workspace`, or the same variant renamed; the rows that build it |
| `idea_send`'s body for a typed line: the door the owner approved on 2026-11-11. No fourth path | A second set of thin rows (§5.3), because the idea rows resolve an idea id (**checked**, `resolve.rs:680`) |
| The launch: `--name`, `--session-id`, resume, limits, the prompt plugin | `--add-dir` words; `AiLaunch` has no field for them (**checked**: none in `crates/`) |
| `ChatPane`'s second kind of target, the ended-chat reading, the terminal fallback | A third kind of target |
| The poll's by-session-name bucket that makes `ideas:sessions` | Its twin, `workspaces:sessions` |
| The record's shape and its rules (ids checked, written atomically) | It moves from `.desk/<idea>/` to the workspace's folder, so `ideasessions` is generalised over where the record lives |

## 2. What the working memory is

**Recommendation: a folder per workspace that is its own git repository. One
index file is always read. Everything else is reached by a link.**

### 2.1 What is in it

| Path | What | Loaded |
|---|---|---|
| `MEMORY.md` | The index: what the workspace is for, its projects in a line each, decisions, how we work here, what is open. Each line may link to a file | Always, by every chat, at start |
| `notes/<date>-<slug>.md` | One note per folded chat: Asked, Decided, Done, Open | When a chat follows the link |
| `topics/<name>.md` | Anything that outgrew a few lines in the index | When followed |
| `files/…` | Images, PDFs, exported tables, anything a chat or the person drops | When followed |

The names are a convention the templates teach. The app parses none of it. It
knows two things: `MEMORY.md` is the index, and the folder is a repository.

This is the shape of Claude Code's own memory (a short index that is always
loaded, files read on demand) and of `/compact` (a chat reduced to a
summary). Borrowed, not copied.

### 2.2 Who writes it, and when

| Who | When | How |
|---|---|---|
| The app | When the workspace's first chat starts, or on Make memory | `git init`, the `MEMORY.md` template, a first commit. After that the app never rewrites a file's text |
| The agent | During a chat, when something durable is decided; at the fold | It writes files under `memory/`. They are in its cwd |
| The person | Any time | The editor (§4.3), or any other editor: they are files |
| The engine | After each of those | It commits (§2.5). Nobody else needs to |

### 2.3 How it stays bounded

The folder is not bounded; that is the point of it. The **index** is: a
budget of about 16 KB (a guess). The engine never cuts it. Over budget, the
Memory tab says "The index is long (23 KB)" with **Tidy…**, which starts a
chat on a fixed tidy file: move detail into `topics/`, keep one line and a
link. The fold's instructions say the same.

An attachment is capped per file (4 MB, the chat image rule's number,
AGENTS.md). Larger things are linked by path, not copied in.

### 2.4 How it differs

| | Whose | About | Lifetime | Given to |
|---|---|---|---|---|
| An idea | The person's thinking; an agent changes it only when asked | Anything | Until removed | Nobody, unless opened |
| A plan | The person's intent; the app reads Projects and Work | Work to start | A record after the work | The workspace view; a work item's brief is copied to a place |
| A place's `.planning/` | One place's agent | One branch | Dies with the place | That place's agent |
| **The memory** | The workspace's; agents write it, the person corrects it | What is true across the workspace's projects, and how it got that way | As long as the workspace | Every workspace chat: the index at start, the rest by link |

It does not reuse the ideas store. That store is one flat folder of id'd
markdown with a copy-per-change history (**checked**, `ideas.rs` header). A
memory is a tree with binary files and changes that touch several files at
once. Ideas and plans stay where they are; a workspace keeps linking them by
id.

### 2.5 The repository

**Recommendation: a real git repository, made and committed by the engine.**
git is already required, and worktrees shells out to it on purpose
(AGENTS.md). Nothing is embedded and nothing new is installed.

| Question | Answer |
|---|---|
| When is a commit made | When a chat starts, if the tree is dirty ("edited by hand"). When a fold ends ("fold: <chat title>"). When the person saves in the app's editor. When a chat closes without folding and left changes |
| Who is the author | A fixed identity (`worktrees <memory@localhost>`), given per command, so the person's git config is not needed and not written |
| What Undo is | `git revert` of the fold's commit: a new commit. Nothing is ever reset or dropped |
| What "what changed" is | `git show --stat` and the diff of one commit, rendered in the Memory tab |
| What history is | `git log` for a file or the folder, as a list of dated, titled changes |

**How it is kept apart from real projects:**

- It lives in the user data dir, outside every registered project, and
  `add_project` refuses it (§1.1).
- It has no remote, and the app never adds one. The person may; it is their
  folder.
- Every git call the engine makes there passes `-c core.hooksPath=/dev/null`,
  `-c commit.gpgsign=false` and the fixed author, so nothing in the folder or
  in the person's config can run code or block a commit.
- No chat's cwd is the repository (§1.1), so no agent session treats it as a
  code project and the MCP server never serves it.
- It is never a worktree, has one branch, and nothing branches it.

Rejected: the ideas store's copy-per-change (one file at a time, no images,
no multi-file change); an embedded library (`gix`, `git2`: the "don't switch
to libs" rule); another tool (jj, fossil: a new dependency for the user); one
repository for all workspaces (removing a workspace would mean rewriting
history, and a revert would not be scoped).

Two chats can write the tree at once. A fold's commit takes whatever is dirty
then, so it may carry another live chat's half-written change. The commit's
diff shows it, and a revert undoes all of it. I think that is acceptable.

## 3. The lifecycle

**Decided: Close folds, and the fold is applied.** It is then shown, with
Undo.

| Act | What happens |
|---|---|
| **New chat** | Makes the folder and the memory if the workspace has none. Commits a dirty tree. Writes the chat's folder, `start.md`, `context.md`, the record with the session id, then launches. A refused launch takes the chat back out, as `start_at` does (**checked**) |
| **Resume** | Rewrites `context.md`, then `--resume '<session id>'` under the same tmux name, no opener |
| **Close** | The fold (below), then the session ends. **Close without folding** is in the menu |
| **Reopen** | Resume. Closing it again folds again; its note is rewritten, not doubled |
| **Delete** | Asks first. Ends the session, removes the record entry and `chats/<conv>/`. Its note stays: it is in the memory. Claude's transcript is Claude's file and stays |
| **Workspace removed** | Asks, and says what it holds. Live chats are closed without folding. The folder is moved to `workspaces/.removed/<id>/`, never deleted by the app |

### 3.1 What the fold is, concretely

1. Refused while the agent is working or waiting on a prompt: "it is working:
   stop it or wait".
2. The engine commits a dirty tree, so the fold's commit holds only the fold.
3. It writes `chats/<conv>/fold.md`, a fixed template: write or rewrite
   `memory/notes/<date>-<slug>.md` with four headings (Asked, Decided, Done,
   Open); update `memory/MEMORY.md`, one line and a link per durable fact;
   move anything long to `topics/`; then say in two lines what you changed.
4. It ends the session if it is live and launches the same conversation as a
   resume **with a fixed opener**: `Read chats/<conv>/fold.md and do what it
   says.`
5. When the note exists and the agent has been idle for one poll tick, the
   poll commits ("fold: <title>"), records the commit in `chats.json`, and
   ends the session. With no note after five minutes the fold is marked
   failed, the session stays open, and the chat row says so.

Why a relaunch and not a typed line: an opener reaches only a session the
launch creates (AGENTS.md, the re-brief rule), and it is how every brief
reaches Claude. It works the same for a chat whose Claude already exited. It
types nothing. The alternative, a fixed line typed through `idea_send`'s body,
is faster and widens the typing door; it is choice 1.

Rejected: a headless `claude -p --resume`. Print mode answers permission
questions with no person there, and nobody can see or stop it.

### 3.2 When the fold is wrong

- **Undo** reverts the fold's commit. The note and the index go back together.
- **Edit** any file by hand; the next commit records it.
- **Reopen** the chat, say what is wrong, close again.
- Nothing is lost either way: a revert is a commit, and the transcript stays.

### 3.3 What a new chat is given from memory

`MEMORY.md`, by instruction in `start.md`. Everything else by the links in
it. Nothing is pasted into argv or into the pane.

## 4. The surfaces

**Recommendation: a workspace's documents and chats are in the main window.
The nav holds the chats; the main area holds Memory, Plan and Ideas beside the
open chat; each document can float into its own window. Its own beta flag.**

### 4.1 The beta flag

One entry in `BETA_FEATURES`: `beta_workspace_chats`, its own checkbox and
helper (AGENTS.md; **checked**, `beta.ts`). The workspace view is drawn only
under Ideas' flag today (**checked**, `App.tsx:6724`). It must draw when
either flag is on, and each part asks its own helper: Plan and Ideas tabs ask
Ideas', the Memory tab and chats ask this one.

### 4.2 The nav: chats under the workspace, not under a project

Where the screenshot says "Chats live here":

```
 ▾ Casa del Valle                  ◐  1
     CHATS                             +
       ● Release order
       ◐ Who can see what
       ○ Storage layout          closed
     ▾ casa-del-valle             24   +
         ◆ (main)
         PINNED …
```

- Live chats and the last few closed ones. "All chats" is in the main area.
- `+` starts a chat. Clicking a row opens it in the main area.
- The workspace's dot is amber when one of its chats waits on the person.
- All projects is unchanged: a workspace chat belongs to no project.

### 4.3 The main area: the room in the screenshot

Selecting a workspace, or one of its chats:

```
 Personal                                               [ New chat ]
 ─────────────────────────────────────────────────────────────────────────────
 Memory   Plan   Ideas 2   Chats 4            ⇱ float │  Release order ▾   Close ▾
 ─────────────────────────────────────────────────────│───────────────────────
  MEMORY.md                       changed 2 h ago     │
  ┌ fold: Storage layout  +3 −1 ──── Diff ▸   Undo ┐  │   (the chat view)
                                                      │
  # Memory: Personal                                  │
  ## What this workspace is for                       │
  …                                                   │
  ## Decisions                                        │
  - 2026-11-24 Media lives on the NAS. → notes/…      │
                                                      │
  FILES   notes 3 · topics 2 · files 1     History ▸  │  [ Message…          ⏎ ]
```

- **Left: the documents.** Memory shows the index rendered, a files list, and
  History (the commits). A link in the index opens that file in the same
  pane. Plan shows the plan with its Projects and Work, which is today's
  overview body (**checked**, `WorkspaceOverview.tsx`). Ideas lists the linked
  ideas and renders the one picked. Chats lists every chat with its state and
  its note.
- **Right: the open chat**, when there is one. With none, the documents take
  the whole width.
- **`⇱ float`** opens the current document in its own window, and the main
  area keeps the chat. This is the Ideas window, taught to open a workspace's
  memory file as well as an idea.

**Reading here, editing where the editor is.** CodeMirror is admitted only
inside the companion window's bundle, never `app/src` (AGENTS.md). So the main
window renders documents with the markdown view it already has, and **Edit**
floats the document into the companion, which has the editor. Putting the
editor in the main window would mean changing that boundary; it is choice 2.

An image in memory is shown by a new console row that reads a file under the
workspace's memory folder. `read_place_image` cannot: it is the place's rule
(AGENTS.md).

### 4.4 The Ideas window

Unchanged for ideas and plans. It gains the ability to open a memory file by
(workspace, path), with its history, when a document is floated or Edit is
pressed.

## 5. The engine

**Recommendation: one new object, about twenty thin console rows over
existing bodies, one event, and no database.**

### 5.1 Files

| File | Written by |
|---|---|
| `<ws folder>/chats.json` | The engine, atomically |
| `<ws folder>/chats/<conv>/start.md`, `context.md`, `fold.md` | The engine |
| `<ws folder>/memory/**` | The agent and the person; `MEMORY.md` first by the app |
| `<ws folder>/memory/.git` | git, called only by the engine |
| `workspaces.json` | Unchanged. The folder is found by the workspace's id, so no new reference is stored |

### 5.2 The object

`workspace(id)`: an id `workspaces.json` knows. And `workspace_chat(conv,
id)`: a chat in that workspace's record, the twin of `idea_conv` (**checked**,
`resolve.rs:685`). A memory path is checked with `safepath::safe_under` the
memory folder, and never under `.git`.

### 5.3 Rows

All `HostConsole`: no device grant reaches them (**checked**: the idea rows
are declared this way).

| Rows | What |
|---|---|
| `workspace_chats`, `workspace_chat_start`, `_resume`, `_close`, `_fold`, `_remove`, `_rename` | The lifecycle of §3 |
| `workspace_turns`, `_turn_text`, `_tool_call`, `_prompt`, `_answer`, `_answer_text`, `_answer_many`, `_send`, `_interrupt`, `_term_open` | The chat: `agents::*_at` on the workspace locus, as the `idea_*` rows are on theirs (**checked**, `worktrees-api/src/ideasessions.rs`) |
| `workspace_memory` | The index's text and mtime, the file list, the index's size, the last fold's commit |
| `workspace_memory_file`, `workspace_memory_image` | One file, read |
| `workspace_memory_write` | The person's save from the editor: `expected_mtime`, then a commit |
| `workspace_memory_log`, `workspace_memory_diff`, `workspace_memory_revert` | History, one commit's diff, Undo |

`workspace_send` and `workspace_answer_text` are `idea_send`'s and
`idea_answer_text`'s bodies behind one more console-only door, with the same
rule, checks and audit record. AGENTS.md's paragraph on typing into Claude
names each door, so phase 1's PR must add this one, with the owner's yes.

### 5.4 The MCP server's workspace mode (a later phase)

Today a server in the workspace's folder has no project and offers nothing.
The mode: the launch names the chat, the server checks it against
`chats.json`, and pins that workspace as it pins a repo today ("no tool takes
a repo path").

| Tool | Tier | What |
|---|---|---|
| `workspace` | read | Projects and each one's places with agent state: `context.md`, live |
| `place_status {<project>:<slug>}` | read | For the workspace's projects only |
| `ideas_list`, `idea_read`, `idea_related` | read | As in every server (**checked**) |
| `plan_propose_work` | write | Adds one `## Work` item to the workspace's plan, marked as proposed by this chat. Starts nothing |

No memory tools: memory is files in the chat's cwd.

### 5.5 Events

`workspaces:sessions`, console only, change-gated: the chats' sessions that
are live, busy and waiting, and each one's workspace. The twin of
`ideas:sessions` (**checked**, `watch.rs:123`), from the same poll. A memory
change is seen by the poll's stat of the folder's `HEAD` and index mtime,
pushed as `workspaces:memory` with the workspace id. (guess: the cheapest
signal; phase 1 picks.)

### 5.6 A paired device, a group-chat guest, the audit record

A device and a guest see nothing: every row is `HostConsole` and both events
are console events. A share is bound to a lane's session in a place, and a
workspace chat is in no place, so it cannot be shared.

The console's non-read rows are recorded as the idea rows are.
`workspace_send` records length and hash, never the text. Start, fold and
revert record the workspace id, the chat id and the commit. Memory text never
reaches the log.

### 5.7 A database for the references? Not yet

**Recommendation: no SQLite now.**

- What must be tracked is small and already has a home: a workspace's
  projects, plan and ideas in `workspaces.json`; a work item's place on the
  plan's `started:` line; a workspace's chats in its `chats.json`; memory's
  history in git. The memory folder is found by id, so it adds no reference.
- The files are the truth, and people and agents edit them by hand. A
  database beside them is a second truth that can disagree.
- The state split in AGENTS.md is "plain JSON, no DB", and every store keeps
  unknown keys so an older build can rewrite it. A schema needs migrations and
  makes a downgrade a problem.
- The counts are tens of workspaces and hundreds of chats.

When it would earn its place: search across memory, notes and transcripts, or
"which chats touched this file". Then build it as a **derived index** in the
cache dir, rebuilt from the files at any time, and never the record. That is
a later proposal.

## 6. Phases

Each ends with a hand check in the sandbox app, with the workspaces folder
and the ideas folder at scratch paths and its own tmux server.

| # | Phase | At the end the owner can | Size |
|---|---|---|---|
| **1** | A chat and a memory folder | Start a chat from the nav under a workspace. It has read the index and the projects. It writes into `memory/` when told, or when it judges something durable. The Memory tab shows the index and the files, read only. Close, resume, delete | M–L |
| **2** | The fold, on the repository | Close a chat and see its note, the commit's diff, and Undo. History. Close without folding. Tidy… | M |
| **3** | The workspace view | Plan and Ideas tabs beside Memory, the chat on the right, float any document, Edit in the floated window, images in memory | M |
| **4** | The MCP workspace mode | Ask a chat what is running across the workspace and get a live answer. The chat proposes a work item; Review… and Start make the place | M |
| **5** | Memory for the lanes | A place started from the workspace's plan gets one line in its brief naming the memory folder; its agent reads the index | S |
| later | | Codex as the harness. Acting under `cross_project = "full"`. Messaging lanes through the MCP bus. A derived search index | |

Phase 1 makes the repository (`git init` and a first commit) so that phase 2
has history from day one. It measures the first item of §8 before anything
else.

Phases 2 and 3 can run side by side.

## 7. Open choices for the owner

Decided on 2026-11-24: memory is a folder; Close folds; the fold is applied;
a workspace chat reads its workspace's projects.

1. **How the fold reaches the agent.** My pick: relaunch on a fixed opener,
   nothing typed. The other way: a fixed line typed into the live session,
   faster, and a wider typing door.
2. **Where a document is edited.** My pick: read in the main window, edit in
   the floated window, so CodeMirror stays in the companion's bundle. The
   other way: admit the editor into the main window and change that rule.
3. **A git repository per workspace's memory, committed by the engine.** My
   pick: yes, as §2.5.
4. **Are a private project's files readable by its workspace's chat?** My
   pick: no.
5. **Removing a workspace.** My pick: its folder moves to `.removed/`, and
   the app never deletes memory.
6. **No database now.** My pick: as §5.7.
7. **May a chat start places itself, later?** My pick: only under
   `cross_project = "full"`, after the plan-item road has been lived with.
8. **One more door for a typed line** (`workspace_send`,
   `workspace_answer_text`, §5.3). It needs your yes, as `idea_send` did.
9. **The index's budget.** My pick: 16 KB, said and never enforced by cutting.
10. **The words.** My pick: "Memory" and "Chats".
11. **AGENTS.md.** Phase 1's PR adds the workspace door to the paragraph on
    typing into Claude, the `~ws~` marker, and the memory repository's rules.
    I did not edit AGENTS.md.

## 8. What I could not establish

- **What an added directory brings besides files.** If Claude runs the hooks
  or loads the settings of a directory given with `--add-dir`, a cloned
  repository in a workspace would run code in a chat whose folder trust the
  person never gave for it. I believe it does not, and I did not measure it.
  **Phase 1 must measure this first.** If it does, drop `--add-dir` and let
  reads outside the cwd ask.
- **Folder trust.** Each workspace's folder is new to Claude, so I expect one
  trust prompt per workspace, once. Whether added directories ask too is not
  measured; my run was headless, which asks nothing.
- **A git repository below the cwd.** Whether Claude treats `memory/` as its
  repository in any way that matters (status in its prompt, its own commits).
  Not measured. The start file tells it not to commit, and a commit by the
  agent would be harmless.
- **Instruction files.** In my one headless run the model reported no
  `CLAUDE.md` text in its context, from the added directories or its own cwd.
  That is one model's report, so nothing above depends on it.
- **Whether a write into an added directory can be denied by a rule.** A
  `--settings` deny on `Edit` under each root may make "never write in a
  project" a guard and not only a rule. Not tried, and it would not cover the
  shell.
- **`--resume <id>` with a positional opener** in an interactive Claude, which
  the fold needs. The CLI's help suggests it, and `launch_cmd` already puts an
  opener after a resume's words (**checked**, `profile.rs:729`), but I did not
  run it. If Claude refuses the pair, choice 1 falls to the typed line.
- **How the MCP server learns its chat** in phase 4: an environment variable
  set at launch, or a per-chat `--mcp-config`. Not measured.
- **How big a memory repository gets** with images over a year, and whether
  the per-file cap is the right one.
- **How long a fold takes** on a long chat, and what it costs.
- **"Visible when you click on the project."** I read this as the workspace,
  which is what the screenshot selects. If a project's own view should show
  its workspace's documents too, that is more surface than §4 draws.
- **Overlap with group chat v4's conversations per topic.** Headings only.
- **Other harnesses.** Whether Codex has an equal of `--add-dir` and of
  `--session-id`.
- **The numbers** (16 KB, 4 MB, five minutes) are guesses.
