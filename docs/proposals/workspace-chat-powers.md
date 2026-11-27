---
title: A workspace chat can do what a lane can do
---

# A workspace chat can do what a lane can do

**Design note, 2026-11-27. Being built on `feat/workspace-chat-powers`.**
The owner's decision of 2026-11-27 reverses the limits of
`workspace-chats.md` §1.4 and §5.4 ("it drives no lane", "the plan is the
person's document"):

> "There's no reason for that chat not to be able to spin up different plans
> or even other ideas. … I think a chat should have the ability to do anything
> a lane can do. The Chat can drive lanes but the only real difference is that
> it handles memories and doesn't have code cloned. But it should be able to
> communicate back and forth with lanes."

So a workspace chat is a first-class agent. It differs from a lane in two
ways only: it keeps the workspace's memory, and it has no checkout.

## 1. What a chat may do

Every tool below is in the chat's MCP server (workspace mode,
`crates/worktrees-cli/src/mcp/wsmode.rs`). The server pins the chat from its
launch (`WORKTREES_WORKSPACE_CHAT`) and asks again on every call, as today.

### 1.1 Plans and ideas

| Tool | What | Engine path |
|---|---|---|
| `plan_read` | The workspace's plan: items with their lines, projects, mtime | `ideas::plan_read_at` |
| `plan_make` | Make the workspace's plan when it has none, from its linked ideas (none is allowed), and set it as the workspace's plan | `ideas::make_plan_at`, `workspaces::set_plan_at` |
| `plan_add_work` | Add a `## Work` item. `made: <chat> · <id> · <when>` marks it; `proposed: true` writes today's `proposed:` line instead | `ideas::plan_propose_at` (history first, re-read before write, `ERR_PLAN_MOVED`) |
| `plan_write` | Replace the plan's text (edit items, sections, anything) with the mtime it read | `ideas::write_kept_at` (history first, `expected_mtime`) |
| `plan_start_work` | Start an item: its place made with the item's brief, the `started:` line written | worktrees-api `plans::start_in`, the app's Start |
| `idea_create` | A new idea, marked `made_by` the chat, recorded in the chat's record | `ideas::create_at` |
| `idea_write`, `idea_append` | Edit an idea this chat made, or one linked to the workspace | the lane tools' bodies |
| `workspace_link_idea` | Link an idea to the workspace | worktrees-api `plans::link_idea_in` |

"Made by" is recorded where the app reads it: a work item's `made:` line
(parsed beside `proposed:`), and an idea's `made_by:` frontmatter. Which ideas
a chat may edit is decided from the engine's own record (`chats.json`, the
chat's `made` list) and `workspaces.json`, never from a file the chat can
write.

### 1.2 Lanes

In each of its workspace's projects, what `(main)`'s agent can do there:

`list_places {project}`, `place_status {place}`, `create_worktree {project,
branch, base?, brief?, provider?, model?, spare?, force?}`, `open_place`,
`close_session`, `send`, `set_note`, `set_lifecycle`, `set_pin`. A place is
`<project>:<slug>`. **Never `remove_worktree`** (the cross-project rule).

- `project` must be one of the workspace's projects, answered for this chat
  (`workspacechats::answered`). A private project, or one outside the
  workspace, is refused in words.
- Each act runs the CLI's own op (`ops::cmd_new`, `open_place`,
  `cmd_close`) in the target project, so the live-agent limit, the quota and
  plan-window gate and host policy apply unchanged. A hub copy is refused.
- A brief written by a chat starts with a provenance line naming the chat and
  its workspace, as a cross-project brief does.
- **`launched_by`.** A chat is started only by console rows
  (`workspace_chat_start` is `HostConsole`), so no device ever launches one,
  and its server's `launched_by` is the console's (none). A lane a chat starts
  is launched for the console too. If a device is ever allowed to start a
  chat, the chat's record must carry the device's stamp and this server must
  pass it on, as `create_worktree` does for a lane's.

### 1.3 Lineage: what a lane started by a chat records

`started_by: {place: <chat id>, at, chat: "workspace:<ws>/<chat>"}`. A link
with `chat` is shown (the app names the chat) and never nests or routes
through the place tree: the chat is no place.

**Where its reports go by default**: a lane's `report` with no `to` goes to
the chat that started it while that chat's Claude is live and the lane's
project is still in the chat's workspace; otherwise the lane-children rules
as today (the nearest live lane above it, else `(main)`). The note says why.

## 2. Messages both ways

### 2.1 The address

`workspace:<workspace id>/<chat id>`, both ULIDs. A place's slug never holds
a `/`, so it cannot be read as `<project>:<slug>` even in a project named
`workspace`. The chat's own tools give it (`workspace`, `place_status`), and
a dragged chat row carries it (§3).

### 2.2 Where the log lives

The chat's inbox is the engine's half of the workspace:
`<workspaces>/.engine/<workspace id>/worktrees-messages/`, in
`messages`'s format, keyed by the chat id. A lane's own log stays its
repository's.

| From → to | Filed in | `from` |
|---|---|---|
| chat → `<project>:<slug>` | that project's log | the chat's address |
| place → `workspace:…/…` | the chat's log | `<project>:<slug>` |
| chat → another chat of its workspace | that chat's log | the chat's address |

A place may post to a chat only when its project (by registry name) is one
of that chat's workspace's projects and is not private. `from` is always
derived by the posting server, never an argument. A reply with no `to`
returns to the sender, as today, because `sender_of` reads the address back.

### 2.3 Claude ↔ Claude

A chat's Claude is started with `--name worktrees~ws~<chat id>`, its tmux
session. `place_status` on a chat's address returns that name as `session`,
so a Claude lane can `SendMessage` it; a chat sends to a Claude lane the same
way, by the session `place_status` names.

### 2.4 Status and reading

- `place_status` takes a chat's address in every server (lanes, `(main)`,
  chats): its title, workspace, state (`busy`, `waiting`, `idle`, `none`) and
  `session`.
- `read_chat {chat, n?}`: the chat's last turns, each its prompt and its reply
  text only, through `turns::claude_opened` (the transcript's user and
  assistant messages, engine openers marked). Never raw transcript text, no
  tool input or output.
- Both are answered only to a place whose project is one of the chat's
  workspace's projects and not private, or to another chat of that
  workspace.

## 3. Dragging a chat into another chat

A workspace chat's row in the nav drags like a place. The composer receives
a reference the receiving agent can act on:

```
[workspace chat “<title>” · workspace:<ws id>/<chat id>]
```

with one line saying to use `place_status` for its state and `read_chat` for
what it said. Dropped into `(main)`'s chat, `(main)`'s agent resolves it
through its own server (its project must be in that workspace).

## 4. What the chat is told

`start.md` lists the tools by group (plans and ideas, lanes, messages) and
drops "the plan and the linked ideas are the person's own documents; never
change one". Its own file tools still write only in `memory/` and its chat
folder: code is changed in a place, by a lane.

## 5. What stays

- The chat's file tools never write in a project's tree; it has no checkout.
- Private projects are neither read nor driven.
- Nothing here reaches a paired device: workspace chats are console rows, and
  no MCP tool gains a pairing, device, scope, remote-access, audit or trust
  argument (`provenance.rs` reads `wsmode.rs` too).
- `remove_worktree` stays refused outside a server's own project.

## 6. Decided by the owner, 2026-11-27

1. **Who may read a chat's turns** (`read_chat`): any place of a project in
   the chat's workspace that is not private, and the workspace's other chats.
2. **Default reports to the chat**: a lane a chat started reports to it with
   no `to` while the chat runs; otherwise up the place tree as before.
3. **Plan rewrites**: allowed. **Every change a chat makes to a plan or an
   idea lands in its history as its own version**, so nothing a chat replaces
   is lost and each change can be undone on its own. Held by a test per write
   tool: a rewrite, an added item, an edited item, an idea write and an idea
   append each leave the text before it as the newest history copy
   (`a_chats_plan_writes_each_keep_the_version_they_replace`,
   `a_chat_edits_the_ideas_it_made_or_the_workspace_links_and_keeps_each_version`).
4. **A chat may start an item it made without Review…**: yes
   (`plan_start_work`). The app shows such an item as "made by <chat>".
5. **The words**: "made by <chat>", the designer's choice.

## 7. What is not built

- A dragged chat goes into a PLACE's chat (the draft). Into a workspace
  chat's own composer, or a terminal, it is refused with a hint.
- The app does not yet draw a lane's chat starter in its header (it draws no
  starter today); the snapshot carries `lineage.starter.chat`.
- An idea's "made by" is its `made_by:` frontmatter line, which the editor
  shows dimmed with the rest of the frontmatter; no list draws it yet.

## 8. Remember and forget (the owner, 2026-11-27)

> "We should be able to tell it to remember things / forget things in memory."

**What it was.** A chat wrote memory files with its own file tools, and the
engine committed a dirty tree only after a quiet minute
(`memory::commit_quiet_at`, from the poll), or at a fold, a chat start or a
revert (`commit_if_dirty_at`). A "remember" therefore reached History late,
lumped with whatever else was dirty, and under a subject naming files.

**What it is now.**

- `start.md` tells the chat, under its memory rule, to write a "remember"
  into the folder at once (an index line, the detail under `topics/`), and to
  remove or correct a "forget" in the index and every file that holds it.
- Then it calls `remember` or `forget` with a few words and the files it
  changed. They commit **those files only**, at once, as
  `Remembered: <what>` or `Forgot: <what>` (`memory::commit_paths_at`, which
  stages the named paths and leaves every other change in the folder for the
  quiet minute, under the workspace's one lock). So each is one change in
  History, and Undo reverts it alone.
- A forget takes the text out of what chats read. History still holds it, and
  the tool's answer tells the chat to say so in one line.

Held by `memory::tests::a_change_to_named_files_is_its_own_commit_and_leaves_the_rest`
and `mcp::tests::remember_and_forget_are_each_their_own_change`.

