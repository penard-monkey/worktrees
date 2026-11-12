---
title: Shared sessions, a first group-chat experiment (group chat, v3)
---

# Proposal v3 — share a session, talk beside it

*Version 3.1, 2026-11-12. Status: every decision in §0 is David's (D1–D6 on 2026-11-06,
D7–D14 and D8b on 2026-11-12); nothing built. Replaces v2 as the next step. v2 is parked
as a later step in [group-chat-v2-handoffs.md](group-chat-v2-handoffs.md). Code claims
are against `4b99c46`, except those added in 3.1, which are against `8cd0168`. Each is
marked **read** (in the code), **measured** (run, with the
sample) or **assumed**. **Contract** marks something this design requires and
nothing yet provides. **Hypothesis** marks something only use can settle. Where this
document and the code disagree, the code wins.*

**The ask, in David's words (2026-11-06):** "I feel like the remote host should fully
own the branch and the remote user/users can inform/chat in the same thread … I want
to be able to 'share' some of my sessions with bret. He sees them maybe as another
section under the worktrees project in a 'David's Places'. He can then open, I would
let him in, and he can then talk. His messages aren't treated as real messages to the
agent, he can message and when I'm ready I can tell the agent to use the messages or
not."

---

## 0. The recommendation, on one page

**Build this as the first experiment.** David shares one running agent session with
Bret. Bret reads that session from its first turn and posts in a thread beside it.
Bret's messages do nothing until David picks some and tells the agent to use them.

- **The host owns everything.** Branch, tree, tools, credentials, quota and agent all
  stay on David's Mac.
- **Bret needs no checkout.** Bret's copy of the repository only decides where the
  share appears in the sidebar. No git runs on either side because of a share.
- **Only David drives.** Bret holds no way to reach the agent, a file, a terminal or a
  prompt.

**What David has decided:**

| # | Decision | Answer | Decided | Where |
|---|---|---|---|---|
| D1 | First experiment | A shared session on one host, instead of v2's no-code rehearsal and two-host handoff. | 2026-11-06 | §1 |
| D2 | Who owns the branch | The host, entirely. No git dependency between the two sides. | 2026-11-06 | §0 |
| D3 | Who triggers the agent | Only the owner. Bret's messages are inert until the owner chooses to use them. | 2026-11-06 | §5 |
| D4 | Joining | The owner shares, Bret sees it and opens it, the owner lets Bret in. | 2026-11-06 | §1 |
| D5 | What Bret can read | The session's transcript, **unredacted**. | 2026-11-06 | §3.1 |
| D6 | How much history | The **full** history of the shared session. | 2026-11-06 | §3.2 |
| D7 | Bret's credential | A new `guest` capability, a new principal and a per-share object. Never `read`. | 2026-11-12 | §2 |
| D8 | Live permission prompts | Private to the owner. Bret sees "waiting for David". | 2026-11-12 | §3.1 |
| D8b | What a guest may see before a share | The **names** of the owner's places (slug and title) in every project David hasn't hidden, and nothing inside them. A guest can ask for a share of one with `share_request`; David shares or ignores. | 2026-11-12 | §2.2 |
| D9 | When a share ends | A share is attached to the **lane** and bound to one session id. A new live id is never shared automatically. An ended session's share **stays**: Bret keeps reading that session as frozen text, marked ended, and the next session needs David's *Share again*. Only Stop sharing, removing Bret, or removing the lane removes the share. | 2026-11-12 | §3.3 |
| D10 | How "use these" reaches the agent | A text file through `agent_attach` plus one line through `agent_send`. The selection defaults to everything since the last use; David's note is the guidance. No new way of typing into an agent. | 2026-11-12 | §5 |
| D11 | Harnesses | Claude and Codex first. Kimi and pi have no transcript reader yet. | 2026-11-12 | §3.2 |
| D12 | Thread location and retention | One log per share, on the host only, kept until David deletes it. | 2026-11-12 | §4 |
| D13 | How Bret learns of a share | A **push**, not a broadcast: sharing, a `share_request` and an allow emit `shares:changed` to the guest's connected sockets. Bret's app shows a toast and a sidebar badge; a closed app sees it on the next connect through `shares_list`. | 2026-11-12 | §6 |
| D14 | Out of this experiment | Bret's own agent, handoffs, mention routing, the judge, a text "claude stop", phone (including phone push), a second guest in the UI. | 2026-11-12 | §9 |

**What changed in 3.1:**

| 3 said | 3.1 says | Why |
|---|---|---|
| D7–D14 were my picks | Decided | David, 2026-11-12 |
| A guest learns nothing that isn't shared with him | A guest sees the owner's place names, unless a project is hidden, and can ask for a share | D8b. §2.2 says what the names give away. |
| A different live id ends the share for good, and Bret's view goes with it | The **binding** ends; the share stays on the lane, with the ended session readable as frozen text, until David's *Share again* | D9 |
| A guest polls `shares_list` | A `shares:changed` push, with `shares_list` on connect | D13 |

**What changed from v2:**

| v2 said | v3 says | Why |
|---|---|---|
| Rehearse with no code, first | Build the small version | David's first question is whether sharing and talking in a live session is useful. Screen-sharing plus a chat app already answers part of that, so §8 includes it as a cheap baseline, not a gate. |
| Two hosts, two agents, code moving between them | One host, one agent, no code moves | Removes cross-fork handoffs, SHA checks, peer branches and the coordinator's hardest cases. |
| No agent content leaves a host unless published | The transcript of a shared session does, in full | David's decision, D5 and D6. §3.4 states the cost. |
| A joiner sees from the join onward | Full history | Same. |
| Cards, states and a journal per handoff | One "use these" act by the owner | One person drives one agent, so there is nothing to arbitrate. |

**What v3 keeps from v2 and the reviews:** agent-authored text never triggers anything;
a stale target refuses with nothing typed; text is refused, never silently altered;
an uncertain send is never retried by software; prompts stay with the owner; the
relevance judge waits for evidence; and filtering is not offered as protection.

---

## 1. The experiment, step by step

**How it looks to David.** In Settings → Remote access, David chooses *Invite a guest*
and types a name, "Bret". That opens a pairing window and gives David a link to send.
When Bret pastes it, David's app asks to Allow "Bret's MacBook" as Bret. From then on
Bret appears in two places: in the share dialog's list of people, and in the devices
list, as a guest, under his own name and apart from David's own devices. Removing him
there removes him from every share. (**Contract**: today's pairing has no invite step and
no guest, §2.1.)

1. **Pair once.** As above. Both need Tailscale or the same LAN.
2. **Share.** On a running agent session, David chooses *Share with Bret*. The dialog
   says what is about to become readable: "Bret can read this whole session from its
   first turn, including anything it printed. This can't be recalled."
   - **Or Bret asks first.** Bret sees David's place names (§2.2) and chooses *Ask
     David to share* on one. David gets a nudge; sharing answers it, ignoring it does
     nothing.
3. **Bret sees it.** A toast and a badge in Bret's sidebar (§6). The session sits under
   *David's Places* in Bret's own `worktrees` project, or under *Shared with me* if
   nothing matches.
4. **Bret opens it.** That asks to join. David gets a request and allows it.
5. **Bret reads.** A read-only chat view of the session, from its first turn, updating
   as the agent works.
6. **They talk.** Both write in a thread beside the session. The agent sees none of it.
7. **David uses it, or doesn't.** David selects messages, optionally adds a note, sees
   exactly what will go, and sends. The thread records that David did.
8. **The session ends, the share doesn't.** When the agent moves to a new session
   (`/clear`, or a new session in the lane), Bret's view freezes: the old session stays
   readable, marked *ended*, with no new turns, and the new session isn't shared. The
   thread stays open. David's *Share again* on the lane binds the new session, with Bret
   already selected. Exit and resume of the same session are only a pause.
9. **It's removed.** David's Stop sharing or Remove Bret, or removing the lane. Bret's
   access ends at once, and the thread stays on David's host.

---

## 2. Authority

| Act | Who | Mechanism |
|---|---|---|
| See what is shared with me | A guest | New row `shares_list`, answering only that principal's shares, ended bindings included |
| See the owner's place names | A guest | New row `guest_places`: project name, place slug and title, for projects the owner hasn't hidden. Nothing else about a place (§2.2). |
| Ask for a share | A guest | New row `share_request(place, note?)`. Grants nothing; lands as a nudge in David's app. Audited with the note's length and hash, never its text. |
| Ask to join | A guest | New row `share_join`. Grants nothing until the owner allows. |
| Hear that something changed | A guest's connected sockets, and the console | New event `shares:changed`, emitted on share, Share again, allow, remove, end, and a request (§6) |
| Read the shared transcript and tool-call inputs | An allowed guest | New rows `shared_turns`, `shared_tool_call`. After the binding ends, the ended session only, frozen. |
| Read and post in the thread | An allowed guest, and the owner | New rows `thread_read`, `thread_post` |
| Share, Share again, allow, remove, stop sharing, hide a project's names | The owner | New **console-only** rows |
| Use messages with the agent | The owner | Existing `agent_attach` and `agent_send`, as the console (§5) |
| Stop a turn, answer a prompt | The owner | Unchanged. Bret can say "stop" in the thread, and that is only text. |

**Why not `read`.** Grants cover projects, not places (`authz.rs:172`, **read**: "Projects,
not places"). `dispatch.rs` has 62 rows at `read` (**read**), including `read_file`,
`list_dir`, `file_diff` and `agent_turns` for every place in the project. A device with
`read` on `worktrees` could read every file and every session in it. So Bret gets none
of them.

**The new capability, contract:**

- `guest` joins `authz::Capability`, with a new object `share(id)` that `serve/resolve.rs`
  checks against `shares.json` on every request. As with `devices.json` (**read**), an
  unreadable file refuses everything.
- Bret is a **new principal** holding `guest` and no projects (§2.1). Phase 2 warned that "a
  second principal above `read`/`write` IS the owner", because one Unix account sits
  underneath. `guest` stays below that line because every row it reaches is a read of a
  chosen session, a read of place names (§2.2), or a write to an inert log or a nudge
  that only David acts on.
- `guest` needs no step-up, since `needs_stepup` covers `instruct`, `answer` and `admin`
  (`dispatch.rs:694`, **read**) and a Mac has no Face ID key. It must never be added to a
  row that reaches an agent. A drift test holds that.
- **Trap, read:** the audit log records every admitted row whose capability column is not
  `read`. A guest's 10-second transcript poll would then write a record each time and
  spend the device's audit budget. The polled read rows (`shared_turns`, `shared_tool_call`,
  `thread_read`, `shares_list`, `guest_places`) need an explicit exemption. Join, allow,
  remove, each request and each post (length and hash, as `agent_send` does) are recorded.

**What is still new trust, stated plainly:**

- Another person's Mac can reach David's remote listener. Every paired device so far has
  been the owner's (**read**), so this is the first real use of a second principal.
- A guest's text, once David uses it, can steer the agent, which runs as David under
  David's permission mode. Manual mode does not mean every action asks. The control is
  David reading exactly what will go, not the label on it.
- A stolen guest key can read what is shared with Bret, read David's unhidden place names,
  post in those threads and send David share requests, and nothing else. Revocation closes the socket at once (`devices.rs`, **read**).

### 2.1 Who the user is

**A user is a principal on the host, and the host alone decides who that is.** No accounts,
no login and no central service. The model already exists in `devices.json` (**read**):

| Thing | What it is | Who sets it | Used for |
|---|---|---|---|
| **Person** | `PrincipalRecord`: an opaque id, a name, a grant | **David**, at invite. The client can't claim one. | Authorship, sharing, permissions |
| **Device** | `DeviceRecord`: a key pin tied to one principal | Key made on Bret's Mac, name reported by Bret's client and renamable on the host (`dispatch.rs:1172`) | Which machine, revoking one machine |
| **Caller** | (principal, device), worked out from the mutual TLS key on each connection | The server. The client sends no identity. | Every permission check and audit record |

- **A thread post shows the principal's name.** History stores the id, so a rename can't rewrite
  who said what. The device name is secondary and can't impersonate a person.
- **A share names principals, not devices.** Bret on a second Mac is a second invite for the same
  person, and shares follow. Losing a Mac means revoking that device, not the person.
- **The audit log already records principal and device** (`audit.rs:308`, **read**).
- **David's own side** is the console and any device under `owner`. `PrincipalRecord::owner` is
  named literally "owner", so the host needs a display name (**contract**).
- **Tailscale gets Bret onto the network and identifies nothing here.** Serve doesn't read
  Tailscale's identity, and the design doesn't depend on it, so a LAN works the same way.
- **Git identity isn't used.** It is self-declared and Bret needs no checkout.
- **Identity is per host.** If Bret hosts later, David is a separate guest id on Bret's host, and
  nothing links the two. That is fine with one host per share. A global identity waits until
  something needs one.

**Two things in the code must change first, both read:**

- `devices.rs:405` refuses any `devices.json` naming a second principal as corruption, and an
  unreadable file refuses **every** device. It must accept guests, holding exactly the guest
  capability and no projects, and still insist on one all-powerful owner. **Downgrade trap:** an older
  serve then refuses the whole file, David's phone included, until the guest is removed. The
  release note has to say so.
- Pairing is hard-wired to the owner (`pairing.rs:311`, `:359`). `pair_begin` gains an invite: a
  new or existing guest, named by David before the window opens.

### 2.2 Place names, and asking for a share (D8b)

**What a guest sees, contract:** `guest_places` answers, for every registered project David hasn't
hidden from guests, the project's name and each place's slug and title. Nothing inside a place: no
note, branch beyond what the slug says, lifecycle, activity dot, agent, harness, plan, file or
session. The names change as David works, so Bret's client reads them on connect and on
`shares:changed`, not on a fast poll.

**What the names give away, plainly.** A slug is usually the branch name, and a title is whatever
David or an agent wrote. Together they tell Bret what David is working on across every visible
project: client or product names, an unannounced feature, a bug's subject, a security fix before it
ships, a personal project. Project names say which repositories exist on this Mac. None of it is a
transcript, and all of it is more than a guest learns from v3.

**Hiding, contract:** a per-project switch, *Show place names to guests*, on David's side, default
**on**, as D8b states it. It lives in host state beside `shares.json`, never in
`.worktrees.toml`, since a cloned repository must not decide what David's guests see (ADR 0001's
spirit). A hidden project disappears from `guest_places` entirely, project name included, and a
place in it can still be shared deliberately. No such switch exists today (**read**: the registry and
workspace rows carry no per-project visibility).

**Asking, contract:** `share_request(place, note?)` from a guest, for a place `guest_places` would list.

- It grants nothing and reaches no agent. It lands as a nudge in David's app on the place: "Bret asks
  to see this session", with the note. David's *Share* answers it, preselecting Bret; ignoring it
  leaves it until it expires or David dismisses it.
- The note passes the thread's character rule and a short limit, and is audited by length and hash.
- One open request per guest per place, and a per-guest rate, so it can't be used as a doorbell.
- A request for a hidden project's place, or one that doesn't exist, gets the same answer as a
  successful one, so asking doesn't reveal what is hidden.

---

## 3. What Bret can see

### 3.1 Contents

Bret sees what David's chat view shows, because it comes from the same parser
(`turns.rs`, **read**).

| Included | Not included |
|---|---|
| David's prompts and the agent's replies, unredacted | Tool **output** and the contents of files the agent read: `Step::Tool` carries a flag for failure and "no output" (**read**) |
| Each tool call's one-line summary, and on opening it, its input (`agent_tool_call`) | The system prompt and every CLAUDE.md, held in `api-request-blob` entries that `turns` never returns (**read**) |
| Questions the agent asked and the answers given | Live permission prompts (D8) |
| Peer messages, background-task notices and compact summaries | Files, diffs, terminals and other sessions |
| `!` shell commands **with their output**, and slash-command output (`Notice::Shell`, `Notice::Command`, **read**) | |

So D5's exposure is smaller than the raw file and still real: a secret reaches Bret if the
agent repeats it, if it is in a command or a tool input, or if David runs it through `!`.

### 3.2 Full history needs a new reader

`agent_turns` cannot do this (`turns.rs:53-64`, `agents.rs:960`, **read**):

| Limit today | Value | Consequence for D6 |
|---|---|---|
| Turns per answer | 20 | Older turns unreachable, no paging |
| Bytes of turn text per answer | 64 KiB, oldest dropped | Same |
| Transcript read | Last 2 MiB | A turn older than that isn't seen at all |
| Text per prompt or reply | 4,000 characters | "Unredacted" would still be cut |

Transcript sizes, **measured** on this Mac's 220 Claude transcripts touched in the last 30
days: median 3.4 MB, 90th percentile 20.5 MB, largest 393.3 MB. This session's is 3.8 MB,
with 35 `api-request-blob` entries, each about 100 KB (`turns.rs`, **read**), so most of it
is blobs nobody returns.

**Contract:** `shared_turns(share, before?, limit?)` answers one page, newest first, with
a cursor.

- It builds an index of turn offsets once per share and extends it as the file grows, so
  a page never re-parses the whole file. A 393 MB read per poll isn't acceptable.
- Each page keeps a byte bound under the 8 MiB outbox (`budget.rs:187`, **read**). A single
  turn is returned whole, up to a stated maximum. Anything cut says so.
- It reuses `turns::claude` and `turns::codex` parsing rather than a second reading of
  a format we don't own, and the same control and invisible character removal.
- Live updates poll at the chat view's 10 s while busy (`ChatPane.tsx:39`, **read**), well
  under the device budget of 50 requests a second (`budget.rs:187`, **read**).

"Full history" means the transcript of **this** agent session. Compaction doesn't lose it, on
Claude or Codex (§3.3, **measured**). Earlier separate sessions in the same place aren't included,
and neither is anything before a `/clear`, unless David binds them with *Share again*, each as its own
history.

**Harnesses.** `agent_turns` reads Claude and Codex and says Kimi's and pi's aren't read yet
(`agents.rs:960`, **read**). David's list includes Kimi. So Share refuses Kimi and pi for now
and says why. Adding a harness means adding its reader, walked through
`docs/adding-a-harness.md`.

### 3.3 What a share is bound to

A share is **attached to a lane** (a project and a place) and holds a **binding** to one harness
session id at a time. The binding, not the slug or the tmux session, says what Bret reads: those
come back, and `--name` survives `/clear` (**measured**). The lane says where the share lives and
when it is removed (D9).

**Measured**, one run each on Claude Code 2.1.319 and Codex 0.167.0, in a private tmux server:

| Act | Claude | Codex |
|---|---|---|
| Compact | Same id, same file, earlier turns still in it | Same |
| Exit or kill, then resume by id | Same id, appends to the same file | Same |
| `/clear` | New id and file at once. The session file's `sessionId` moves immediately. | New id and file **only at the next turn**. Until then nothing shows the change. |
| After `/clear` and a turn | The old file stops growing | Same, but **the process holds both rollouts open** |
| Resume the id from before `/clear` | Works, appends to the old file, and leaves the newer file behind | Same |
| Claude's `-r` picker, which `worktrees open` uses | Lists both sessions. The person picks, and the chosen id continues. | Not used. `open` resumes the newest started thread by id (**read**). |

**The rules that follow:**

- **A share reads exactly the file its binding names**, never "the latest in this place". So `/clear`
  can't leak new work: the new turns aren't in the bound file. Compaction loses nothing, and
  the history Bret reads doesn't join across a `/clear`.
- **No live agent means paused.** Exit, a crash or a closed app ends nothing. Resuming the same
  id continues the binding.
- **A live agent with a different id ends the binding, for good, and that's recorded.** Rechecking
  alone isn't enough, because an old id can be resumed later and would match again. David could
  then be working privately in a session the UI had told him was over. So an ended binding stays
  ended even if its id comes back; only *Share again* binds anything new.
- **An ended binding is frozen, not removed.** Bret keeps reading that session's history up to the
  moment it ended, marked *ended*, with no further turns, even if the old id is later resumed and
  grows. `shared_turns` stops at the byte offset recorded when the binding ended (**contract**).
  The share, its guests and its thread stay on the lane.
- **Share again** is one explicit act by David on the lane: it binds the lane's current live session,
  with the share's guests preselected, and shows the same "can't be recalled" warning, because it
  shares a new session's whole history. The ended binding stays readable beside it. A new id is
  never bound automatically.
- **Removal is separate.** The share goes only by David's Stop sharing, by removing Bret from it, or by
  the lane being removed. A removed lane removes its shares even when the CLI removes it with serve
  down, so serve checks on start and on its poll that each share's place still exists
  (**contract**). A later place with the same slug never inherits a share.
- **Serve checks active bindings on its own poll**, not only when Bret reads, so ending doesn't depend
  on Bret being connected. Residual: a change and a resume of the old id while serve is down look
  continuous when it returns.
- **Which id is live.** Claude: the session file's `sessionId`, found by pane (**read**). Codex: the
  **newest started** rollout the process holds. Codex's id doesn't move between `/clear` and the next
  turn, which is harmless because nothing is written in that gap.
- **The thread across bindings** is my reading of D9 and D12, for David to overrule: one thread per
  share, continuing across *Share again*, with "session ended" and "shared again" as events in it.
  Bret can post while a binding is ended; *Use these* needs a live bound session, so it waits for
  *Share again*.

**Two existing Codex problems this found, read and not yet reproduced in the product:**

- `codex::held_thread` returns the first held rollout it finds (`codex.rs:75`), and after `/clear`
  there are two. So an agent restart after `/clear` can resume the older thread. The share check
  must not reuse it as it stands.
- `agent_turns` reads `codex::latest_rollout`, the newest **started** in the place
  (`agents.rs:1202`). After someone resumes an older thread, today's chat view shows the wrong
  conversation. Claude's path goes by the live id and doesn't have this problem.

Fixing both is a small change on its own, and PR A needs the first fix.

### 3.4 What D5 and D6 cost

The Codex and Kimi reviews argued a session holds private context and that string filters
can't protect it. Both points stand. This design doesn't filter, and instead makes sharing a
deliberate act on one session for one named person.

- **It can't be recalled.** Anything Bret has seen, Bret has seen. The share dialog and the
  remove action both say so.
- **History is shared all at once.** Sharing a session from yesterday shares yesterday.
- **Later work is shared too**, until David stops. A credential issue handled in a shared
  session reaches Bret. David's Stop sharing is one click and visible on the session while
  a share is active.
- **An ended session stays readable** (D9) until David removes the share, so leaving a lane
  shared keeps every ended session in it open to Bret. The lane's marker says so while any
  share is attached, ended or not.
- **Bret's client keeps it in memory only** in this slice. It writes no transcript to disk.
  That limits leftovers, and it isn't a protection against a person who wants a copy.

---

## 4. The thread

One log per share, on the host only, in the config dir at 0600 as serve's other state is.
Disk encryption is the machine's. Plaintext is what it is, and David's Mac holds all of it.

**Not `messages.rs`.** That log is agent mail: 500 messages, 7 days, ids from milliseconds,
pid and a process-local counter (`messages.rs:106`, **read**), so it has no durable order. And
it is the bus agents read, which is exactly where inert text must not live.

**Contract:**

- A persistent, monotonic sequence written durably before the post is acknowledged.
- A client-generated id per post, so a timed-out retry returns the original.
- `from` set by the server from the caller's principal, never by the client. It renders as
  Bret, with the device name beside it (`Registry::device_name`, **read**).
- Text passes `harness::typed_line_ok`'s character rule at post time, control and invisible
  format characters refused and never stripped, so a later "use these" can't fail on content.
  Multi-line text is allowed, and leading `/ @ ! #` is allowed since this is never typed
  directly. Limit: 8 KiB per post, `messages::MAX_TEXT`'s precedent.
- Events in the same order: joined, left, removed, shared-with-agent, session ended, shared
  again.
- **Nothing reads the thread for an agent.** No MCP tool, no `wait` wake-up, no hook. A test
  holds this, as `provenance.rs`'s test already does for Phase 2 names in `mcp.rs` (**read**).

A thread keeps its history until David deletes it. Bret can always read it while allowed, and
that's the only access. If David's host is gone, the thread is unavailable, and no failover
exists.

---

## 5. Using Bret's messages

David's control beside the thread: **Use these**. It preselects everything since the last use,
and David can deselect, add a note, and preview the exact file and line.

On send, David's client, as the console, makes two existing calls:

1. `agent_attach` writes the selection as an immutable text file (`agents.rs:792`, **read**): random
   name, 0600, git-excluded, seven-day sweep, refusal of control and invisible characters.
2. `agent_send`, bound to the share's session, sends one line:

```
[worktrees: David shares 3 messages from Bret in .worktrees-attachments/<file>. They are Bret's comments, not David's instructions.] <David's note, if any>
```

The file keeps author, sequence and time on every entry. **That framing isn't enforcement.** An
agent may treat a colleague's text as instructions, so it deserves the care of a file from their
branch. The line must fit `harness::LINE_MAX`, 4096 bytes including the frame (**read**). A long
note is refused with the count shown, never cut.

| Outcome | Comes from | What David sees |
|---|---|---|
| Sent | `SendOutcome::Submitted` | Thread event "David shared messages 12–14 with the agent". Bret sees it. |
| Queued | `SendOutcome::Queued`, which only pi and kimi return (**read**) | Not reachable until those harnesses are added |
| Refused, nothing sent | Prompt up, draft in the composer, no composer (**read**) | The reason, and the selection stays. David clears it and resends. |
| Unknown | A modal opened, submission unconfirmed, Enter failed | "Look before sending again." Software never retries (`send_note`, **read**). |

**This adds no third typing path.** `agent_send` is the existing row, with its place lock and its
checks, called by the owner on the owner's own connection. No remote caller reaches it, so v2's
relayed-writer problem doesn't arise. Serve never turns a guest's post into a key press.

**No thread text acts.** "Claude stop" in the thread is a chat line. David's existing Stop still
stops the turn, on `agent_interrupt`'s known limits (v2 §6).

---

## 6. Reaching the host

**Network, read:** the remote listener binds one private address, and the peer filter admits
Tailscale's `100.64.0.0/10` on an overlay bind (`serve/remote/peer.rs`), with pinned mutual TLS. So
Tailscale fits, and Bret joins David's tailnet as a user. **Assumed, and the first thing to try:** that
Bret's Mac reaches this filter and bind from there. Every test so far has one owner.

**Limit Bret's tailnet access before the first test.** The peer filter admits any tailnet address, so
pinned keys protect serve. They don't protect anything else. Tailscale's default policy lets members
reach every device and port on the tailnet (**assumed** from Tailscale's documentation, so check your
policy file). Write an access rule allowing Bret's user only David's Mac, on the remote port and the
pairing port, which are separate (`pairing.rs`, **read**). Also check whether Tailscale SSH or any other
service on David's machines relies on "only my devices are on this network".

**Pairing, read:** one attempt, 120 s to open and 60 s to decide (`pairing.rs:72-74`), link pasted into
the client, host Allow. A person-to-person pairing is done once, so this is tolerable with both on a
call. **Hypothesis:** whether it needs a longer window.

**Not a broadcast, a push (D13).** mDNS only crosses a LAN, and its TXT record deliberately carries
only a version and an id (`advert.rs:23`, **read**), so it can't list sessions and shouldn't. Instead
the host pushes to the people a change is about:

- **The event.** Sharing, *Share again*, an allow, a removal, an ended binding and a `share_request`
  emit `shares:changed`. Its payload carries ids only (share, place, what happened), and the client
  re-reads `shares_list`, as `places:changed` already means "re-read".
- **Who gets it.** Every emit already goes through `scope::event` for each connected socket, remote
  devices included, and today every device-related event is `Event::Console` (`scope.rs:179`,
  `conn.rs:645`, **read** at `8cd0168`). `shares:changed` needs a new kind (**contract**): the console,
  plus the sockets of the principal the share or request names, and nobody else. Another guest never
  hears about a share that isn't theirs.
- **What Bret sees.** A toast ("David shared *slug* with you") and a badge on *David's Places* until he
  opens it. On David's side, a request shows as a nudge on the place.
- **If Bret's app is closed**, nothing is queued for him: the next connect calls `shares_list` and the
  badge comes from that. No OS notification and no phone push (D14).

Nobody who hasn't been paired and named learns anything. A paired guest learns the place names of
§2.2 and what is shared with him.

**Availability.** An app-owned serve stops 120 s after its last console client, and a device never
holds it up (`OWNED_IDLE_GRACE`, **read**). So quitting David's app ends Bret's view. The agent keeps
running in tmux, and the share resumes when David returns. Bret's view says "David's host is offline"
and keeps the last page in memory.

---

## 7. The clients

**Bret's sidebar is the largest UI change.** A window talks to one engine, chosen once at startup
(`main.tsx:47`, `remote-hosts.json`'s `chosen`, **read**). Bret must keep his own workspace **and** see
David's shares, so the window can't simply switch hosts.

**Contract:** a small, separate guest connection beside the engine door, allowed only the guest rows.

- It can reuse the protocol's one client: `connectSocket` takes its dependencies per call
  (`ipcws.ts:245`) and the shim hands out a `conn` per `remote_connect` (`ipcremote.ts`), and neither
  looks tied to one socket (**read**). **Assumed** until driven: two sockets at once in a window.
- Pairing doesn't choose a host (`remote_pair` and `remote_choose` are separate calls, `HostsPanel.tsx:65`,
  `:102`, **read**). The Hosts panel marks a guest host and doesn't offer "drive this host".
- It stays outside `ipc.ts`'s single door and its route table, so a stray `invoke` can't land on David's
  host, and `transport-check.mjs` gains a check for that.

**Placement in the sidebar is cosmetic.** Project identity is a local path, with no repository identity
in the registry (**read**). `shares_list` carries the origin URL as a label, and Bret's client compares
it to the origin of its own projects: a match goes under that project as *David's Places*, otherwise
*Shared with me*. A wrong match puts a row in the wrong place and has no other effect, since nothing acts
on it.

**Bret's session view** is `ChatPane` reading through `shared_turns`, with no composer, Stop or prompt
controls, plus the thread. The real risk is a component built around `agent_send`; a read-only variant
must hold no route to it. An ended binding shows as **frozen**: an *ended* mark, no polling, and the
thread still open. Under *David's Places* Bret also sees David's unshared place names (§2.2), each with
*Ask David to share*, and nothing more.

**David's view** gains a share control on the session, a join request with Allow, a request nudge with
Share, a *Shared with Bret* marker on the lane that stays visible while any share is attached, *Share
again* on a lane whose binding has ended, the thread, Use these, and the per-project *Show place names
to guests* switch. Per this repo's rule, the check is what the running
app draws on two real Macs, not what a core test reports.

---

## 8. Build order and proof

Each step can be shown working before the next starts.

| Step | Build | Reuses | Proved by |
|---|---|---|---|
| 0 | Reach: Bret pairs to David's serve over Tailscale, today's code | Remote listener, pairing, `worktrees remote` CLI | One `worktrees remote call` between the two Macs. Costs no code and tests the riskiest assumption. **Today's pairing makes Bret's device the owner's**, so narrow it from the host console (`devices.rs:1164`, **read**) to `read` on a scratch project, and revoke it afterwards. |
| 0b | **Done.** Session-id probe on Claude and Codex | Scratch sessions, no product code | §3.3's table and rules |
| 1 (PR A) | Guest principal (§2.1), `guest`, invite pairing, `shares.json` with lane-attached shares and bindings, share rows, ending and freezing a binding, lane removal, `guest_places` and the hide switch, `share_request`. Includes the `held_thread` fix (§3.3). | Resolver, `devices.rs`, audit | `worktrees remote call` as Bret: shared session answers, every other row is refused, a hidden project is absent |
| 2 (PR B) | Thread log and rows; the `shares:changed` event and its new scope kind | `typed_line_ok`, `conn::Clients` for revocation, `scope::event` | Same CLI, two sides; a second guest's socket hears nothing |
| 3 (PR C) | Full-history reader, stopping at an ended binding's offset | `turns` parsers | A 393 MB transcript pages without re-reading the file; a poll stays cheap |
| 4 (PR D) | David's UI: invite, share, allow, request nudge, *Share again*, hide switch, thread, Use these | `agent_attach`, `agent_send`, `ChatPane` | Real app, this host |
| 5 (PR E) | Bret's UI: guest connection, sidebar with names and *Ask*, toast and badge, read-only and frozen views | `connectSocket`, `remote_connect` | Real app, two Macs, one real task |

Steps 2 and 3 can run in parallel once step 1 lands. Step 4 needs 2, and step 5 needs 2 and 3. Bret
needs a build with step 5 on his Mac, so a pre-release or a local `make install-app` is part of the
last step.

**Tests that must exist, each shown red first:**

- A guest calling each of the other rows in the dispatch table is refused, generated from the table so
  a new row can't slip through.
- A guest can't read a session not shared with them, one in the same place, or a later session after
  `/clear` until *Share again*.
- An ended binding serves nothing past its recorded offset, even after its old id is resumed and grows.
- Removing a share or Bret closes the view immediately, on the next request and on open channels.
  Removing the lane, with serve up or down, removes its shares, and a new place with the same slug
  inherits none.
- `guest_places` lists no hidden project and nothing but names; a request for a hidden or missing place
  answers exactly as a real one does.
- `shares:changed` reaches the console and the named guest's sockets only.
- No path takes thread text to an agent except the owner's Use these: no MCP tool or hook reads it.
- An unreadable `shares.json` refuses every guest.
- A retried post creates one entry; a restart keeps the order.
- `api-request-blob` content never appears in `shared_turns`.
- A guest's polling writes no audit lines, and its posts and requests do, by length and hash.

**Baseline, cheap and optional:** one hour with Bret on a screen share and a chat app, David relaying by
hand. It costs nothing, and gives §9's comparison a reference point. It doesn't block step 0.

---

## 9. What this tests, and what comes next

**What this tests:** whether watching a live session and talking beside it is more useful than screen
share and chat, and whether "my words wait for your OK" feels like help or like friction.

**What this doesn't test:** v2's bet, that two people's own agents coordinate well. Bret brings no agent.
Bret is also stalled whenever David is away, since only David can trigger anything.

**Record over a few real sessions:** how much of Bret's input David used, and how long it waited; how
often Use these was refused or unknown; how far back Bret actually read; whether either person fell back
to a call or another chat, and for what; anything Bret saw that David wished hadn't been shared. Two people
and a few sessions give direction, not reliability.

| Result | Next |
|---|---|
| Bret keeps opening it, David uses the messages | Keep going: Kimi and pi readers, more than one guest in the UI |
| David wishes the messages went straight to the agent | Consider peer instruct, with v2 §2.3's preconditions, which are real security work |
| Bret wants to work in parallel, not watch | v2's two-agent handoffs |
| People keep wanting the agent to notice things unasked | v2 §7.3's judge, starting in shadow mode |
| Stopping is too slow or too indirect | v2 §6's turn-bound Stop |
| The view was enough and the thread wasn't used | Keep sharing, drop the thread |
| Screen share and chat were just as good | Stop. Don't grow this into a chat product. |

**Out of this experiment, until a result above asks:** Bret's own agent or code moving between people;
handoff cards and states; mention routing and `@agent`; the judge; a text "claude stop"; Bret answering
prompts or stopping turns; DMs, OS notifications, phone and phone push (the in-app toast of §6 is
in); failover and host migration; another transport;
files, diffs or terminals for a guest; more than one guest in the UI. The data model allows several
guests per share, since a list costs nothing, but nothing else is designed for them.

**Should not be built at all:** a thread that drains onto an agent; any serve-side text parser that acts;
a filter offered as protection for a transcript; a guest path to `agent_send`.

---

## 10. What must be true, and what is unknown

**Must be true for this to hold, each testable:**

- A guest reaches only the guest rows, and only for shares naming them, plus the names of §2.2.
- A binding never moves to another session without David's *Share again*, and an ended one never
  grows. A share never outlives its lane.
- Only the owner's act puts anything in front of the agent. Thread text never does by itself.
- Text is refused rather than altered, and an uncertain send isn't retried.
- Adding a row to the table can't widen a guest without someone choosing it.

**Unknown:**

| Question | Settled by |
|---|---|
| Do two people's Macs reach each other through this filter and bind, over Tailscale? | Step 0 |
| Does §3.3's table hold on the next harness versions? It's one run on one version of each. | Re-run the probe on an upgrade |
| Is the full-history index cheap enough on the largest real transcripts? | Step 3 |
| Do two sockets in one window behave? | Step 5 |
| Do people read the history, or just the last few turns? | Use |
| Is owner-only triggering tolerable? | Use |
| Is a 10-second view live enough when the conversation is live? | Use. `shares:changed` covers sharing; pushing turns is a later option. |
| Do the place names tell Bret more than David meant? | Use, and §9's record |

**Where this differs from the reviews, and why:**

- **Codex and Kimi on transcripts:** both argued against mirroring. This mirrors one session, deliberately,
  to one named person, unfiltered, on David's decision. Their warning stands in §3.4.
- **Codex on the first step testing the bet (v2's framing):** v3 moves the bet. It tests the shared session
  first, because that's what David wants to learn, and §9 says plainly what is left untested.
- **Kimi on direct peer instruct:** still deferred, now with David's own words behind it. His messages "aren't
  treated as real messages to the agent".
