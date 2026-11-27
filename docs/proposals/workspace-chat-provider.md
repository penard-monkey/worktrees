---
title: Choosing the provider of a workspace chat
---

# Choosing the provider of a workspace chat

**Design note, 2026-11-27. §3 is built on `feat/workspace-chat-provider`.**
The owner, 2026-11-27: "need to be able to choose which provider we want to
chat with".

Marks: **read** means I read the code (file:line given); **measured** means I
ran it here (codex-cli 0.172.0, kimi 2.8.1, pi 1.6.1, Claude Code 2.1.332),
without starting a session; **unmeasured** means neither.

## 0. The answer

| Harness | A workspace chat now? | Why |
|---|---|---|
| Claude | **Yes**, and it gains a **model** choice | Every rule is enforced at launch (`--settings`), and every surface reads it |
| Codex | **No, not yet.** The nearest one | It cannot keep a private project unreadable; several chats in one folder resume each other's thread; every chat surface on a workspace session is Claude's |
| kimi | **No** | No per-launch way to stop writes or reads; its opener, MCP files and trust allowance all assume a git repository |
| pi | **No** | No file rules at all; its launch needs a project's store; no transcript reader |

So the picker this branch builds offers the four harnesses the way the
new-lane dialog does, and **Codex, kimi and pi are shown and cannot be
chosen**, each with its one-line reason. Claude gets a model. The chat's record
holds both, the header, the nav row and `workspace_chats` show them, and a
resume or a fold relaunches with the same ones.

What a user would lose with each, and what it would take, is §3. Nothing here
needs writing a harness's config or answering its trust prompt; that is why
Codex stops where it does (§3.1).

## 1. What a workspace chat relies on

| Need | How Claude gives it today |
|---|---|
| Its file tools may read the workspace's projects and never write in them | `--settings` permissions: `allow Read(/<root>/**)`, `deny Edit(/<root>/**)` per read project (`workspacechats::settings`, core `workspacechats.rs:902`) |
| A private project is unreadable | `deny Read(/<private>/**)`, also for a private project nested in a read one (`:906`, `:925`). Outside its folder and the allowed roots, Claude asks before reading |
| Its own auto memory is off | `autoMemoryEnabled: false` in the same settings (`:931`) |
| The worktrees MCP server in workspace mode | `chat_env` (`:1362`) puts `WORKTREES_WORKSPACE_CHAT` in the session's environment; the user-scope server inherits it and pins the chat (`pin_at`, `:1378`) |
| The opener | Fixed argv: `Read chats/<conv>/start.md and begin.` (`opener`, `:171`) |
| Resume by id | The engine picks the id and passes `--session-id` (`desksessions.rs:409`); resume is `--resume <id>`. Several chats share one folder and never collide |
| The fold on Close | A relaunch on the fold opener (`workspacefolds.rs:479`), the turn's end read from the Claude transcript (`own_turn`, `:597`), then the commit |
| The Chat view's turns | The Claude transcript by session id (`chat_turns_at`, core `:1705`) |
| Dot, waiting, model label | Claude's probe files and transcript (`watch.rs:250-320`, `claude_model`) |
| Prompts and answers | `desk_lane` matches Claude's pane only (`agents.rs:2118`) |
| A typed line | `send_at` with Claude's rule (`agents.rs:1110`); a non-Claude desk is refused (`:1956`) |
| Folder trust | Claude asks once for the workspace's folder; the chat says so (`TRUST_WORDS`) and the person answers |

## 2. Per harness

### 2.1 Codex

- **Rules.** Read: Codex's sandboxes limit *writes* and the network, never
  reads; no per-folder read deny exists (`codex.rs:355`, `codex --help`,
  measured). A Codex chat could read every private project on the disk, in
  the workspace or not. That is a safety line the brief names: a start would
  have to be refused whenever any registered project is private.
  Write: `-s workspace-write` with the workspace folder as its cwd would keep
  the projects unwritable, and for every program it runs, which is stronger
  than Claude's rule. But host policy may say Full
  (`--dangerously-bypass-approvals-and-sandbox`, `codex.rs:359`), so a chat
  would have to force the sandbox over the host's choice. Memory: `memories`
  is a stable feature, off by default (`codex features list`, measured);
  `--disable memories` per launch would hold it (unmeasured in a session).
- **MCP.** The server is the user's, added with `codex mcp add`
  (`codexmcp.rs:72`); we never write it. Whether Codex hands its own
  environment to an MCP child is unmeasured, and I believe it passes only a
  fixed set of variables: then `WORKTREES_WORKSPACE_CHAT` never reaches the
  server and it refuses, in `pin_at`'s words. A per-launch
  `-c mcp_servers.worktrees.env_vars=[…]` would be the fix, and would need
  measuring.
- **Opener.** Argv, as for a lane (`harness.rs:216`). Fine.
- **Resume.** Codex picks its thread id itself; the engine finds it afterwards
  by the cwd in each rollout's `session_meta` (`codex.rs:46-87`), newest
  first (`:172`). Every chat of a workspace shares the workspace's folder, so
  resuming one chat could open another's thread. A chat would need its own
  cwd (`chats/<conv>/`), which changes what `start.md` says and how the MCP
  server finds its workspace (`workspace_of_folder` keys on the folder).
- **Fold, turns, dot, waiting, prompts, send, model.** Codex has readers for
  each, keyed by a place's canonical session or its directory (`turns.rs:390`,
  `activity.rs:398`, `prompt.rs:356`). None is reached for a workspace session:
  `desk_lane`, `desk_activity`, `desk_transcripts`, `own_turn` and the
  workspace poll are Claude's. Each would be a port, walked surface by surface
  as `adding-a-harness.md` asks.
- **Trust.** The workspace folder is in no `[projects]` entry and is no repo,
  so Codex opens without asking (`codextrust.rs:20-29`, Codex's own rule
  mirrored). The `[trust] codex` allowance is keyed on a repo root and does
  not apply; nothing would need answering. Fine.
- **ADR 0001.** The folder is the engine's, not a cloned repo. But Codex reads
  a `.codex/` in a trusted cwd, and a chat can write in its folder. It is not
  trusted, so that is not read today; keep it so.

**Loses:** private projects (refused while any exists), reads limited to the
workspace (none). **Would take:** its own cwd per chat, the env measurement,
a forced sandbox, and the port of six surfaces.

### 2.2 kimi

- **Rules.** No sandbox, no read or write rule, no memory switch per launch
  (`kimi.rs`, `kimiplace.rs`, `docs/kimi-manual-checks.md:126`). Its approvals
  are kimi's own steps. The rule would be advice only, and a private project
  readable.
- **MCP.** Files in the place's own `.kimi-code/mcp.json`, excluded through the
  repo's common `info/exclude` (`kimiplace.rs:104-122`, `:262`): assumes a git
  repository, and loaded only in a trusted folder (`:28`).
- **Opener.** Not argv: `worktrees kimi-opener` types the fixed lane opener
  (`ops.rs:137-164`, `kimi.rs:894`), and logs under a git common dir (`:916`).
- **Resume.** Found after the run by cwd (`kimi.rs:144-213`): the same
  collision as Codex.
- **Trust.** Asks for every new folder (`docs/kimi-manual-checks.md:75`); the
  allowance needs a repo root (`trust.rs:241`), so the person would answer it,
  and Esc there exits kimi (`trust.rs:215`).

**Cannot**, for the rules first and the repo assumptions after.

### 2.3 pi

- **Rules.** Only `--approve`/`--no-approve`, which is about a project's own
  resources, not file access (`trust.rs:5-10`). No write rule, no read rule.
- **MCP.** User scope; pi passes its environment to the server
  (`mcp.rs:906-911`, read), so workspace mode would work.
- **Launch.** Needs a `Project` and its store for the session generation
  (`harness.rs:630`, `:750`) and refuses with no model (`:728`).
- **Turns.** No pi reader in `turns` (`turns.rs:390-397`).

**Cannot**, for the rules.

## 3. What this branch builds

1. **The record.** `Chat` keeps `harness` (already there, always `claude`)
   and a new optional `model`. A start takes `{harness?, model?}`; a harness
   other than Claude is refused with the words of §0 (one table,
   `workspacechats::harness_refusal`), so no client can start what the picker
   shows disabled. A model passes `choice::validate_model`.
2. **The launch.** `Launch` carries the model; `--model <m>` on a fresh start,
   a resume and a fold's relaunch. No model is Claude's own default, as today.
3. **The surfaces.** `workspace_chats` lists both, and `harnesses`: every
   harness with why it cannot start a chat (`harness_picks`: the refusal
   first, then `Adapter::installed`). The header shows "Claude <model it last
   replied with>" as before, and the model it was started on until it has
   replied. The nav row's title ends "Claude · <model>", and a Chats tab row
   with a chosen model says it. The dot, waiting, turns and notifications are
   Claude's, as before: no surface changes for a model.
4. **The picker.** New chat's plain press starts the workspace's last choice
   (Claude and its default model on a first chat), with no step. A ▾ beside it
   opens the harness segment and the model picker of the new-lane dialog;
   Codex, kimi and pi are shown disabled with their reason, and a harness that
   is not installed says that instead. The choice is remembered per workspace
   (`ui-state.json`, `ws_chat_pick`).

## 4. Parked, 2026-11-27

The owner, on this note: "we'll leave the other providers' chat off for now
… we may revisit later." Both questions are parked:

- **Codex as a chat** (§2.1): its own cwd per chat, the env measurement, a
  sandbox forced over host policy, the six surfaces, and a rule that refuses
  it while any registered project is private. Not started.
- **Changing a chat's model mid-chat** (`/model` from the chat is
  session-only, `AGENTS.md`): not answered. The model is chosen at start only,
  and a resume and a fold relaunch on it.
