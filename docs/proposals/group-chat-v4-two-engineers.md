---
title: Two engineers, two agents, one feature (group chat, v4)
---

# Proposal v4 — two engineers, two agents, one feature

*Version 4, 2026-11-14. Status: draft for David, nothing built. Follows
[v3.1, shared sessions](group-chat.md), which shipped to staging on 2026-11-14
as PRs #650, #656, #657, #661, #663 and #664 and is going to main behind the
Group chat beta switch, and takes the two-agent, hand-off and Stop analysis from
[v2, a shared work room with explicit hand-offs](group-chat-v2-handoffs.md),
which v3 parked. Claims are marked as in v2: **read** (in the code), **measured**
(run, with the date), **assumed**, **contract** (required, not provided),
**hypothesis** (only use settles it). Where this document and the code disagree,
the code wins.*

**The ask, in David's words (2026-11-14):** "Imagine two engineers discussing a
proper feature build (like we have here but now it is two engineers). This
current flow we have is a good first step but we're going to start thinking of
the better approach." And: "we will likely need to have the messages support
files and images."

---

## 0. The recommendation, on one page

**Build a room per feature, not a share per session, in four steps that each
ship on their own.** v3.1 proved the hard part: one person can read another's
live agent session over a pinned connection and talk beside it, and the owner's
*Use these* is a safe, explicit way for a colleague's words to reach an agent.
What it cannot do is let the second engineer *work*: Bret has no agent in the
picture, cannot see a diff, cannot send a file, and his request for another
share is invisible unless David happens to be looking at that lane. The
rehearsal of 2026-11-14 (§1) measured each of those.

v4 keeps every rule v3 and v2 established, and adds the missing half:

| Step | Adds | Ships as |
|---|---|---|
| **4.1 Fix and finish** | The rehearsal's defects; the note as the first field of *Use these*; a request inbox; push for new turns; files and images in the thread | One PR on main, in the beta |
| **4.2 What changed** | A read-only diff and file view of the shared place for the guest, under the same warning as the transcript | One PR, in the beta |
| **4.3 Symmetry** | Bret's own lane, on Bret's Mac, shared back the same way; the thread becomes the room's log and outlives any one share | Two PRs |
| **4.4 Hand-offs** | A card from one engineer to the other's agent, accepted by its owner, delivered through the *Use these* path | v2 §5, scoped to two hosts |

**What stays fixed, from v2 and v3:**

- Execution stays local to the owner: tools, credentials, quota, prompts and
  audit never leave the agent's host.
- No new way into an agent. Everything reaches an agent through the owner's own
  `agent_attach` + `agent_send` on the owner's own connection, framed as a
  colleague's material, never as instructions.
- A guest never holds `read` on a project; it holds `guest` and per-share
  objects. Every byte a guest sees is one the owner chose to share, by a row
  the owner's app called.
- Agent-authored text never triggers anything. No serve-side parser acts on
  text. Software never retries an unknown send. Stop never presses twice.
- One coordinator per room: the opener's serve. No failover, no migration.

**What David decides** (my pick in the Answer column; each is a yes/no he can
flip):

| # | Decision | Answer | Where |
|---|---|---|---|
| D1 | The unit of sharing | A **room per feature** that holds shares, the thread and files, instead of a thread per share. A share still binds one lane's session; the room outlives it. | §3 |
| D2 | Files and images in the thread | **Yes**, in 4.1: images shown inline, files downloadable, both kept on the coordinator under the room, with the chat-image rules applied to anything drawn. | §4 |
| D3 | What the agent gets from an attachment | Only through *Use these*: the file is **copied** into the place's attachments folder beside the bundle, named in the frame, and the agent reads it as material. Never a path on the guest's Mac, never a URL. | §4.3 |
| D4 | Diffs and files for the guest | **Yes**, in 4.2, as a per-share switch that is **off** by default and carries v3's warning: "Bret can read every file and every change in this place." Read-only, the place's own tree only, no other place, no host paths. | §5 |
| D5 | The note in *Use these* | **Required** unless the owner unticks "send without a note". The rehearsal showed that without a note the agent reads the comments and does nothing. | §1, §6 |
| D6 | Requests and asks | A **request inbox** in the owner's corner, like join asks today, for every share request wherever it lands. | §6 |
| D7 | Liveness | The guest's transcript updates on a **push** when the host's poll sees a new turn, not on a 10 s timer. The host already indexes transcripts per tick. | §6 |
| D8 | Symmetry | Bret's host runs the same serve with remote access on; David pairs to it as a guest; Bret shares his lane. Two pairings, no third kind of principal. | §7 |
| D9 | Where the room lives | On the **opener's** serve, which sequences the log and keeps the files. The other host keeps a copy of what it was sent. When the opener is off, the room is read-only. | §7 |
| D10 | Hand-offs | v2 §5's card and states, scoped to two hosts, in 4.4 and only after 4.3 is in use. Accept is the target owner's act and goes through *Use these*. | §8 |
| D11 | Stop and the judge | Stop stays the owner's button; a guest gets "Ask David to stop". No judge. Unchanged from v2 §6 and §7.3. | §8 |
| D12 | Out of v4 | More than two people in the UI, phone, OS notifications, DMs, a Slack mirror, Kimi and pi readers, peer instruct, any transcript filter. | §10 |

---

## 1. What the rehearsal taught (measured, 2026-11-14)

Two test Macs, one as David (owner) and one as Bret (guest), the staging build
at `ed54d01`, Codex 0.167.2 in the shared lane. The whole v3.1 flow passed:
invite and pair, share, join and Allow, live read-only transcript, thread by
push, *Use these* with the exact file and line, session end and *Share again*,
a host reboot with the share surviving, Stop sharing. Beside that it found:

| Finding | What it means for v4 |
|---|---|
| **The note is the instruction.** Two comments sent through *Use these* with no note: Codex read the file and answered "The plan is unchanged." The same with a note ("add a sixth line naming one test"): it edited the file at once. | The frame works exactly as §5 of v3 intended, so the UI must treat the note as the primary field (D5). "Fold these in" is a one-line note, and the default should offer it. |
| **A request for another place is invisible.** Bret asked for `(main)`; nothing told David unless he opened that lane. | Requests need the corner, like join asks (D6). |
| **Flags get mangled.** Bret typed `--json`; the thread and the bundle carried `—json`. macOS smart dashes apply to the guest's composer. | Composer fields that carry code need smart substitutions off. A one-line fix, and a reminder that the thread is engineers' text. |
| **The guest sees that a command ran, not what changed.** `exec` rows and the call's input are there; the file the agent wrote is not. Bret reviewed a plan he could not read. | A diff and file view (D4). Without it, "two engineers discussing a feature build" has one engineer blindfolded. |
| **Ten seconds is a long pause in a conversation.** The guest's transcript polls; Bret's comments arrived on David's screen at once, David's agent's replies reached Bret late. | Push for turns (D7). |
| **The Ask form broke under its own width** and its send never reached the host; the sidebar listed the host twice while it was offline; the thread panel collapsed in the system WKWebView until its height stopped depending on a percentage. | Layout work needs the real app on the real engine (`AGENTS.md`'s rule), and 4.1 fixes these first. |
| **A reboot took every share with it** until #664: the place's identity compared a device number macOS renumbers at boot. | Fixed. Noted because it is the kind of thing only a real host finds. |
| **Attachments and the thread are already most of the way there.** The attach folder has random names, 0600, a sweep, a planted-link refusal and a git exclude; the thread has durable seqs, idempotent posts and a byte limit. | Files and images (D2, D3) extend both rather than add a store. |

**Hypothesis from the same hour:** a colleague who reads the live session and
writes beside it *is* useful when the owner uses the notes. Whether that beats a
screen share plus chat was not measured; v3 §9's comparison still stands to be
run with Bret.

---

## 2. The bet, restated

v2 bet that two people's independent local agents coordinate better through a
shared room than through chat and two windows. v3 bet, smaller, that one
person watching and commenting on the other's agent is useful. v4 bets that
**the two bets are one path**: the room v3 built is the right room, and what it
lacks is the second engineer's own agent, the artefacts, and a way to hand work
across. Each step of §0's table can stop the path if its proof fails.

What v4 does **not** bet on: agents talking to agents. Every hand-off is a
person's request, accepted by a person, delivered to an agent by that person's
own app. v2 §7's analysis of agents in the room, the judge and injection loops
holds unchanged, and nothing here reopens it.

---

## 3. A room per feature (D1)

Today a share is attached to a lane and carries its own thread
(`shares.rs`, `sharethread.rs`, **read**). Bret's view is "David's lane, this
session"; when the lane is removed the thread goes with it. For one session that
is right. For a feature that runs over days, two lanes and both people's agents,
it is the wrong unit: the conversation about the feature is the thing that
lasts.

**Contract:** a room is `{ id, name, opener, members, shares[], files[], log }`.

- The **log** is today's thread, lifted one level: one durable sequence per
  room, idempotent posts, events for every share added, joined, ended, shared
  again, stopped. Today's `thread_read`/`thread_post` keep their shape with
  `room` in place of `share`; a share's events name the share they are about.
- **Shares** are v3's shares unchanged (lane-bound, session-bound, ended
  bindings kept) and now belong to a room. A room can hold David's lane and
  Bret's lane at once (§7). Stopping a share leaves the log.
- **Files** are the room's attachments (§4).
- **Members** are v3's guest principals plus the opener. Two in v4.
- **Removing a lane** removes its shares and nothing else. **Deleting the
  room** deletes the log and the files, on the opener's host, after a
  confirmation that says what is lost.

The v3 data model allows this: a share already carries guests as a list, the
thread store is keyed by an id, and the events are typed (`Kind`, **read**).
The migration is a `room` field on `Share` and a thread file keyed by room,
with today's one-share rooms created on first read from each existing share.

---

## 4. Files and images in the thread (D2, D3)

Engineers paste screenshots, logs, a patch, a CSV. Today a post is text under
8 KiB (`sharethread::TEXT_MAX`, **read**), and a `--json` typed on the guest's
Mac arrives as an em dash. v4 adds attachments to posts and fixes the composer.

### 4.1 The rules

| Rule | Value | Why |
|---|---|---|
| Who may attach | Any member, on a share they are let into | Same gate as a post |
| Per file | 10 MiB | A screenshot, a log, a patch. Not a build artefact. |
| Per room | 200 MiB, oldest sweepable by the opener | `attach`'s 100 MB precedent, doubled for two people |
| Types shown inline | `IMAGE_TYPES` (png, jpg, gif, webp, bmp, svg) through `check_image`: the 40 MP cap, header sniffed, SVG with DOCTYPE or ENTITY refused | The chat-image rules, `safepath`/`imgsrc` (**read**) |
| Types kept as files | Anything else, by extension and sniff; downloaded, never opened by the app; executables and archives refused | No surface runs a guest's file |
| Names | Sanitised as `attach::store` does (**read**); the original name is display text | A name is not a path |
| Where | Under the room on the opener's host: `<config_dir>/rooms/<id>/files/<random>`, 0600, swept with the room | One store, the host's |
| Transport | A new guest row, `thread_attach {room, id, name, bytes}` under the 16 MiB frame cap; the post that references it carries `files: [id]` | Through the guest connection, which sends only guest rows (**read**, `guestdoor.ts`) |
| Reading | `thread_file {room, file}` answers bytes, to members only, with an etag so a picture is fetched once | `ImageView`'s cache by etag (**read**) |
| Audit | Attach and read are `guest` acts: name length, bytes and hash, never content | As posts |

### 4.2 The composer

- Smart dashes and smart quotes **off** on the thread composer and on the
  *Use these* note (`autocorrect`/`spellcheck` are not enough on macOS;
  the field needs the WebKit text-substitution attributes, and the fix is
  measured in the real app, not the harness). Code goes in backticks and is
  rendered as code, the way the chat already renders a reply.
- Paste or drop an image into the composer: it uploads, the post shows a
  thumbnail; drop a file: a chip. Posting with nothing typed and one file is
  allowed.
- A post over the byte limit is refused with the count, never cut, as today.

### 4.3 Attachments and the agent

The only way an attachment reaches an agent is *Use these*. When a selected post
carries files, the owner's app **copies** each one into the place's
`.worktrees-attachments/` beside the bundle, through `agent_attach` as it does
the bundle (**read**, random name, 0600, excluded, swept), and the frame names
them:

```
[worktrees: David shares 2 messages and 1 file from Bret in .worktrees-attachments/<bundle>. They are Bret's comments and material, not David's instructions.] <note>
```

The bundle lists each file with its original name, who attached it and when.
An image is copied as is; an agent that can read images (Claude's Read does)
sees it, one that cannot sees the name. **Nothing is executed, extracted or
run.** The preview shows the files that will go, with their sizes, and the
owner unticks any. This is D3: a copy under the owner's control, not a path.

---

## 5. What changed: diffs and files for the guest (D4)

v3's D5 shares the transcript unredacted. The transcript already shows every
command and much of every file the agent printed; what it does not show is the
state of the tree. For a reviewer that is the thing that matters.

**Contract:** two guest rows under a per-share switch, **Share files and
diffs**, off by default, set by the owner in the share's *Sharing…* dialog with
the warning "Bret can read every file and every change in this place."

| Row | Answers | Rules |
|---|---|---|
| `shared_diff {share, base?}` | The place's working-tree diff against its base branch, or against a commit the owner picks; unified, with file list and counts | Built by the host with the place's own `git`, never a guest-supplied argument reaching git (R3, **read** `argv::Argv`); size-capped with a note; binary files listed, not shown |
| `shared_file {share, path, rev?}` | One file's text, at the working tree or at a commit | `safepath::safe_under` the place root, no link out, no `.worktrees/` under `(main)`, as `read_place_image` already decides (**read**); text only, size-capped; binaries refused |

Both are `guest` rows, refused without the switch, audited by path length and
hash. The guest's view gains a **Changes** tab beside the transcript: the diff,
file by file, with the owner's commits since the share began as a list. No
editing, no terminal, no checkout.

**Why not v2's "no projection" rule?** Because v3 already chose projection for
the transcript (D5), and a reviewer who can read every command the agent ran but
not the file it wrote has the worse half of the trade. The switch keeps it the
owner's deliberate act, per share, visible in the bar as *files shared*.

---

## 6. Finish the owner's side (D5, D6, D7)

Three changes to what shipped, each measured in §1:

1. **The note first.** *Use these* opens on the note field, focused, with a
   placeholder that is a real instruction ("Fold these in and show me the
   result"). Sending with an empty note asks once: "Send without telling the
   agent what to do with them?" with a checkbox to stop asking. The frame is
   unchanged.
2. **A request inbox.** `share_requests_list` already answers every open
   request (**read**); the corner's `ShareAlerts` shows only join asks. It shows
   requests too, with *Share…* opening the share dialog on that place and
   *Dismiss*. A request for a place with no live session says so and offers to
   open the place.
3. **Push for turns.** The host's poll already notices transcript growth per
   place for the nav dots (`watch`, **read**). When a shared binding's
   transcript grows, emit `shares:changed {what: "turn", share}` to that share's
   guests; the guest view re-reads its newest page on it and keeps the 10 s poll
   as the fallback. Cost: one event per turn per share.

And the rehearsal's layout defects: the Ask form, the offline duplicate section,
all measured in the real app with the browser check PR E added
(`guest-thread-browser.mjs`, **read**).

---

## 7. Symmetry: Bret's lane too (D8, D9)

The second engineer's agent runs on the second engineer's Mac. Nothing in the
transport makes that special: Bret's Mac runs the same app, its serve has remote
access on, David pairs to it as a guest with an invite, and Bret shares his lane
into the same room.

**What a room needs that a share did not:**

- **Two hosts, one log.** The room's log lives on the opener's serve (D9). A
  post Bret makes travels over his guest connection to David's serve as today.
  A share Bret *opens* on his own host is announced to the room by Bret's app
  through the same connection: an event `share_added {host, share}` carrying
  ids and the host's pin fingerprint, so David's app knows which host to read
  that share from. Reading Bret's share goes over David's guest connection
  **to Bret's host**: shares are read from the host that holds them, the room
  is only the index.
- **Two guest connections in one window.** v3's guest door already keeps one
  socket per guest host (`remote.rs`, **read**), so a window that is the owner
  of its own engine, a guest of David's and a guest of Bret's is three sockets
  it already knows how to hold.
- **Who may see what.** Each share keeps its own owner, its own guests and its
  own switches. The room shows a share's row to every member; opening it asks to
  join, and the share's owner allows. Membership of the room grants nothing on
  any host.
- **Naming.** A room's share is shown as `<owner's name> · <slug>`; a lane slug
  alone names nothing (v2 §4).

**When the opener is off**, the room is read-only: no posts, no new shares,
and the last page of the log stays on screen ("David's host is offline" is
already the words). Bret's share on Bret's host stays readable by David, because
that read never touched the opener. **No failover** (v2 §8.1).

---

## 8. Hand-offs, scoped (D10, D11)

The last step, and only after 4.3 is in daily use. The need it answers is
measured by the rehearsal's shape: Bret wanted David's agent to do something,
and the only way was to write it in the thread and wait for David to use it. A
hand-off makes that explicit and bounded, and it is v2 §5 with two hosts:

- A member writes a **card** aimed at a share's agent: one request line, the
  posts and files it should carry, an expiry (30 min to start). It is a room
  event in state `requested`, executes nothing, and shows on the owner's corner
  like a join ask.
- The owner's **accept** is *Use these* with the card's selection preloaded and
  the request line as the note. The frame says `request from Bret, accepted by
  David, hand-off <id>`. The states and receipts are v2 §5.2's, written to the
  room log; `observed` comes from the transcript as it does for the owner's
  own sends.
- Nothing new types into an agent. The journal-before-paste rule and "unknown
  is never retried" hold because the path is the owner's existing one.
- **Stop** stays the owner's button, bound to the turn (v2 §6's two new
  arguments on `agent_interrupt`). A guest's "Ask David to stop" is an event.
- **No judge**, no mention routing, no `@agent` in v4.

---

## 9. Build order and proof

| Step | Build | Proved by |
|---|---|---|
| **4.1a** | The rehearsal's fixes: Ask form, offline duplicate, smart substitutions off, the note first, the request inbox, push for turns | The two-VM rehearsal again, scripted (`scenario.sh` in the parallels cache), every step on screen on both Macs |
| **4.1b** | Attachments: `thread_attach`/`thread_file`, the composer's paste and drop, inline images, *Use these* copying files into the bundle | A screenshot and a patch from Bret reach Claude as files; an SVG with an entity is refused; a 11 MiB file is refused with the count; a guest without the share is refused |
| **4.2** | `shared_diff`/`shared_file`, the switch, the Changes tab | Bret reads the diff of the file Codex wrote in the rehearsal; a path with `..`, a link out, and `(main)`'s `.worktrees/` are refused in words; the switch off refuses both rows |
| **4.3a** | Rooms: the `room` field, the log keyed by room, the migration of existing shares, the events | Existing shares read unchanged after the upgrade; a room with two shares keeps one log |
| **4.3b** | Symmetry: `share_added`, reading a share from the host that holds it, the room's share list across hosts | The rehearsal with the VMs swapped: Bret's VM shares a lane into David's room; David reads it over his guest connection to Bret's VM while David's own lane is shared the other way |
| **4.4** | Hand-off cards, accept through *Use these*, receipts, turn-bound Stop | v2 §10.3's list: kill-at-every-step journal test, replayed history executes nothing, no agent-authored event makes a card, a stale incarnation refuses with no key sent, one lane end to end on two real Macs |

**Tests that must exist, each shown red first:** a guest calling any
non-guest row is refused, generated from the table (exists; extended to the new
rows); an attachment outside the room's store is unreachable by any id; a file
named with a path is stored under a random name and listed by its display name;
`shared_file` on a link out of the place refuses with the same words as
`read_place_image`; `shared_diff` with a guest-supplied `base` that is not a
commit the owner listed is refused before git runs; the room's log keeps its
order across a restart; a turn push reaches only the share's guests; the note
prompt appears only with an empty note; a request appears in the corner within
one poll.

---

## 10. Out of v4, and should not be built (D12)

**Out until a result asks:** more than two members in the UI (the data model
allows them); phone and phone push; OS notifications; DMs; a Slack mirror; Kimi
and pi readers; peer instruct (v2 §2.3's preconditions are real security work);
a guest answering prompts or stopping turns; failover or host migration; any
transport but the pinned one; editing from the guest's side.

**Should not be built:** an agent-to-agent path of any kind; a serve-side
parser that acts on thread text; a transcript or file filter offered as
protection; a file store the agent reads by path on another machine; an
attachment the app opens or runs; a host-asserted caller for a person who is
not on that socket.

---

## 11. What must be true, and what is unknown

**Must be true** (contracts, testable):

- Only an owner's act on the owner's own connection submits to an agent, for
  the owner's own selection or an accepted card alike.
- A guest's bytes reach the host only through guest rows, and the guest
  connection sends nothing else.
- Every file a guest sends is stored under a random name on the opener's host,
  shown by display name, never executed, and copied into a place only by the
  owner's *Use these*.
- A guest reads a share from the host that owns it, and the room grants nothing
  on any host.
- The log's order survives restarts, posts are idempotent, and replay executes
  nothing.
- Unknown sends are never retried by software; Stop never presses twice.

**Unknown**, for use or measurement:

| Question | Settled by |
|---|---|
| Does a note-first *Use these* get used, or does David keep typing to the agent himself? | 4.1 in use for a week, counted in the log |
| Are images in the thread mostly screenshots of the app, and would a "share this screen" act beat them? | 4.1b in use |
| Is a diff enough, or does Bret want to open the file in his editor? | 4.2 in use |
| Does the opener's host being off block the feature too often? | 4.3 in use; the answer decides whether a room needs a second host |
| Do hand-offs get written, or does the thread plus *Use these* cover it? | 4.3 in use before 4.4 is started |
| What does a 10 MiB image cost on the guest's first read over Tailscale? | Measured in 4.1b |
| Which WebKit attribute turns smart dashes off in a textarea, and does it hold in the system WKWebView? | Measured in the VM before 4.1a closes |
