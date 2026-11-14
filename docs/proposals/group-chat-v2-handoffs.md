---
title: A shared work room with explicit handoffs (group chat, v2)
---

# Proposal v2 — a shared work room with explicit handoffs

> **Status: parked as a later step.** David chose a different first experiment
> on 2026-11-06: see [v3, shared sessions](group-chat.md). This document is kept
> whole because its two-agent handoff, Stop and judge analysis is what v3 points
> to if that experiment earns a next step. Where the two differ, v3 wins, notably
> on transcripts (D5, §3) and the no-code rehearsal (D1). The text below is
> otherwise unchanged from `1960e52`.

*Version 2, 2026-11-06. Status: proposal for David and Bret, not approved,
nothing built. Replaces v1 (2026-11-05, commit `37c76bb`), after independent
reviews by Codex and Kimi. Code claims are against `4b99c46`, and each is
marked **read** (in the code, with file and line), **measured** (run, with the
version and the sample) or **assumed**. **Contract** marks something this
design requires and nothing yet provides. **Hypothesis** marks something only
use can settle. Where this document and the code disagree, the code wins.*

**The ask, in David's words:** "a group chat with worktrees, so that we can
build a group channel where we can both talk and drive the build. That has
impact on how we run turns and how the agent responds, because it needs
interrupt (only when appropriate) … if the user's start talking and say
'claude stop' that interrupts, but when the conversation is not exactly
relevant we judge the message and then determine when we should/shouldn't
engage. The person who starts the chat hosts the conversation … if he has the
worktrees project open and I do too, they don't match on paths or
environments, but the group chat should still show up."

---

## 0. The recommendation, on one page

**The product contract:** a shared work conversation in which each person can
hand a bounded request to an agent, that agent's owner lets it through, and
the result is deliberately published back. Conversation, local execution and
publication are three separate things, and nothing crosses from one to another
without an explicit act.

**Do not build yet.** Run the no-code rehearsal in §10 first: two people, two
Macs, two existing agents, one real task across different checkouts, handoffs
written as cards and relayed by each agent's owner. It tests the actual bet,
which is that independent local agents coordinate better through a shared room
than through ordinary chat and two windows. v1's first step could not test
that: it had one host, one person's agents, and a reader who could not hand
anything to an agent.

If the rehearsal earns it, the first native slice is a **human log plus
explicit handoff** on two hosts, with a single sequencer and no new way into
any agent.

**What v2 keeps from v1:**

- Execution stays local to the owner: tools, credentials, quota, prompts and
  audit all stay on the agent's host.
- There is no third path that types into Claude.
- A peer does not approve prompts by default.
- Project binding is explicit, and a remote URL is only a suggestion.
- Code moves through git.
- Text delivery, steering and interruption are treated as different mechanisms.

**What v2 changes:**

| v1 said | v2 says | Why |
|---|---|---|
| "Zero new trust boundary", "no new authority" | Withdrawn. A room crosses people, a sequencing host, relayed attribution and publication. §2 names each act separately. | A member's text can influence an agent without any grant, and the host sees everything. |
| A member with no grant has lines that "never reach" the agent | Any shared text an owner includes can influence the agent. The control is the owner approving an exact bundle, not a label. | `channel_read` returned those lines anyway. |
| Replies mirrored from `agent_turns`, prompt cards shown to all | No agent content leaves a host unless the owner publishes it. Only a handoff's content-free receipts are automatic. | A place is reusable and a session has private context. String filtering is not a confidentiality boundary. |
| Mention dispatches, judge may add engagement | A mention opens a handoff card. Only a human act creates a task, and the judge is out until evidence earns it. | Mention-only removes classifier errors, not unintended action. |
| Host matches stop phrases in text | A structured per-agent, per-turn Stop is primary. One exact text shortcut resolves to it in the composer. | Exact phrases miss "stop!" and "para", and nothing defined a bare "stop"'s target. |
| A five-method `ChannelTransport` could be swapped later | A coordinator and a transport are separate. No swap is promised before a second implementation passes §8.3. | Order, membership and history are the service, not the wire. |
| One-host, read-only first step | A rehearsal, then two hosts, two agents | The first step has to test the bet. |

**Decisions**, each with my pick:

| # | Decision | Pick | Where |
|---|---|---|---|
| D1 | What comes next | The no-code rehearsal. No application code until its gate is read. | §10 |
| D2 | First native shape, if earned | Human log + explicit handoff on two hosts, unless the rehearsal favours a facilitator or an external bridge. | §1, §10 |
| D3 | Who can make an agent act | Only its owner, by accepting a handoff or typing locally. No peer holds `instruct` in the first slice. | §2, §5 |
| D4 | What a member's credential grants | A new channel-only capability: read and post in one channel. No project `read` or `write`. | §2 |
| D5 | What leaves a host | Agent content only when the owner publishes it, from a draft the agent or the owner wrote. Enrolment labels and a handoff's receipts are the only automatic events, and they carry no content. No transcript or prompt mirroring. | §3 |
| D6 | Permission prompts | Private to the owner. A peer sees at most "waiting for David", on a handoff's own turn. A peer cannot answer. | §2, §3 |
| D7 | History on join; removal | A joiner sees from the join onward, unless a range is explicitly shared. Removal stops future access and cannot recall copies. | §3 |
| D8 | What the agent receives | The request in full, or it is refused, plus an immutable context bundle the owner approved. No tool for browsing the room. | §5.3 |
| D9 | Agent-authored text | Never dispatches an agent or a stop, whatever it says. | §7 |
| D10 | Stop | A structured control bound to session and turn. Requested, confirmed, refused or unconfirmed. Non-owners can only ask. | §6 |
| D11 | Relevance judge | Not in the first experiment. Shadow mode on real logs first, if failures of explicit addressing are measured. | §7.3 |
| D12 | Who sequences; outage | The opener's serve, with a persistent sequence. No failover. Outage is read-only and shows the last contiguous sequence. | §8 |
| D13 | Reachability, if native | Tailscale under a channel-only pairing, one way, member to host. Not a claim about trust. | §8.2 |
| D14 | Identity | Immutable ids for person, device, channel, project binding, agent enrolment and session incarnation. Names only render. | §4 |
| D15 | `@bret` | A public mention in the room. No DMs, no inbox, no push. | §10.3 |
| D16 | Out until earned | Phone, host migration, general chat, Slack mirror, alternate transport. | §10.3 |

---

## 1. The bet and its alternatives

The differentiator is two people's **independent local agents** in one
conversation, across different paths, forks and environments. Everything else
here (a log, mentions, a surface) already exists in ordinary chat, so the
comparison that matters is against the simpler things below.

| Option | What it is | What it gets right | What it gives up | Evidence that would pick it |
|---|---|---|---|---|
| **Human log + explicit handoff** (recommended native shape) | People talk. "Send to this agent" shows exactly what will be delivered, and the owner accepts. | Almost no routing ambiguity, no judge, no new authority over an agent. | Owner mediation. An absent owner stalls the other person. | Handoffs get cheaper and mediation is tolerable. |
| **One facilitator agent** | One owner's agent takes all requests and produces branch-based handoffs. | No cross-agent coordination at all. | One person's quota, tree and environment. Bottleneck. Does not exercise the second checkout. | The second agent adds nothing measurable. |
| **External chat + constrained bridge** | Conversation stays in a chat people already use. A small local bridge accepts only structured, expiring handoffs, and nothing mirrors transcripts outward. | Notifications, mobile and history already work. No chat to build. | Log, membership and order sit with a third party, whose terms and privacy need evaluating. | The rehearsal shows the room itself was never the valuable part. |
| **Mention-only native room** | A mention delivers directly to the agent under a peer's `instruct` grant. | Fastest to speak. Closest to the ask. | Bret's words run as David under whatever David's host policy lets run unprompted (§2). Needs relayed attribution solved. | Owner mediation is the measured pain, and §2's preconditions are met. |
| **v1's full room** | Mirrored replies, judge, every agent present. | Feels most like a colleague. | Everything §2–§7 exists to prevent. | Not on the path. It would have to be earned step by step. |

"Claude stop" and judged engagement were in David's ask. They are not refused.
§6 gives the stop a dependable form with the text as a shortcut, and §7.3 keeps
the judge as a hypothesis with a gate. **David should decide whether that
reordering is acceptable**, because it changes the first thing anyone would
touch: a Stop button and a card, not a word in a conversation.

---

## 2. Authority

### 2.1 Acts, separated

v1 asked one question, "may this person reach this agent?". That hides seven
different acts, and they have different answers:

| Act | Who, in the first native slice | Mechanism | What it really is |
|---|---|---|---|
| Read and post in the channel | Members | New channel capability, object `channel(id)`. **Not** project `write` (v1 needed generic `write` to post, contradicting its "read-only" member). | Conversation. The host sees it all. |
| Indirect influence on an agent | Anyone whose text an owner includes in a bundle | Owner selection (§5.3) | **Cannot be prevented, only chosen.** Any text an agent reads can steer it. |
| Request work from your own agent | The owner | Local client on the owner's loopback, `agent_send` as `Caller::console()` | What David does today. |
| Request work from a peer's agent | Any member can **propose** | A handoff card. It executes only on the owner's accept. | A proposal, with no authority until accepted. |
| Publish agent output | The agent's owner | Owner approves a draft (§3) | Disclosure. Not reversible. |
| Stop | The owner. A non-owner can ask. | §6 | A key press, with effects of its own. v1's "the safest act" was too generous: Esc can leave a half-done operation. |
| Inspect or answer a prompt | The owner only | Unchanged rows, owner's devices | `answer` + `write` "is a shell, run through an agent" (Phase 2 §1.1). Across two people it is Bret running commands as David. |

A device of a second person holds **no** `read` on the project, so `agent_turns`,
`agent_prompt` and `agent_tool_call` are simply out of its reach. That removes
v1's narrowed-read projection (§6.5 there), which tried to filter secrets out
of text and could not. Everything a member knows about the other host arrives
as a channel event that host chose to write.

### 2.2 What is still new trust, stated plainly

- **The sequencing host sees every line in plaintext**, and it can forge or
  withhold events. Each member holds a copy on disk.
- **A member's Mac can reach the host's listener**, as a paired device with a
  channel-only grant. The second principal has not been run in practice:
  **read**, every paired device so far is the owner's, and only Phase 2 §2.8's
  tests exercise one.
- **An accepted handoff runs as the owner.** The acceptance card has to say so
  every time, because the host can't always tell what the session may do
  unprompted: "This agent may act without asking under its current mode.
  Accepting lets Bret's request cause that." v1's plan to read auto mode from
  the pane and fall back to host policy failed in the unsafe direction. Assume
  the worse case unless the mode is positively read.
- **Manual mode does not mean every action asks.** Allowed tools can read files
  and write within policy without a new prompt, so "the owner's prompt will
  catch it" is not an answer to injected or careless requests.

### 2.3 Relayed writers and the confused deputy

v1 said the host dispatches a line "under the writer's grant" and the resolver
is unchanged. **That does not hold.** Step-up is bound to one TLS connection's
exporter (`serve/stepup.rs` header, **read**), and a proof cannot be carried
to another connection. David's host cannot truthfully assert
`Caller::paired(bret, device)` to Bret's host, or to its own dispatch, for
something Bret did not do on that socket. A serve acting on a remote person's
text with the serve's own power is a confused deputy.

The first slice avoids the problem instead of solving it. **The only caller
that ever drives an agent is the owner, on the owner's own connection.** A
peer's text arrives as data, shown as "Bret, as sequenced by David's host" (a
statement about the path, not proof of authorship). The owner's accept is the
authorising act.

**Before any peer may direct-instruct** (the mention-only option), all of these
must exist:

- a request signed end to end by the writer's device key, which the target host
  verifies against its own enrolment, never the sequencer's say-so
  (`sign::DeviceSigner` is a place to start, **read**);
- a freshness proof that means something off a single connection;
- a recheck of membership and grant immediately before the key is pressed;
- a host-applied label on `agent_send`, which pastes bare today (**read**,
  `agents.rs:735`);
- a budget per principal and per turn. `budget.rs` is per device, so one person
  with two devices has twice as much, and agent starts do not bound what an
  existing session spends.

That is new security work, and §10's gate decides whether it is worth doing.

### 2.4 A compromised colleague

Under this model a stolen member key can read the channel, post in it and
propose handoffs. It cannot start a turn, read a transcript or see a prompt.
What it can do is write persuasive cards and poison shared text, so the owner's
accept is exactly what it targets. Revocation closes sockets at once
(`devices.rs`, **read**) and cancels unaccepted handoffs (§5.4). It does not
undo what an accepted one did.

---

## 3. Publication

**The default is deliberate, bounded and owner-approved.** Being in a channel
publishes nothing.

- **No projection.** The room never reads `agent_turns`, `agent_prompt` or the
  pane on behalf of another person.
- **A result is a draft first.** At the end of a handoff the agent proposes a
  report (or the owner writes one). The owner sees exactly what will go, edits
  it, and publishes. The event carries the source handoff and, where it applies,
  repository and commit. Limit: 8 KiB, the precedent from
  `messages::MAX_TEXT`. Longer work belongs in a commit.
- **Filtering is not offered as protection.** Removing home paths and hiding
  tool inputs did not remove secrets from prose, summaries, first lines of a
  command or relative file names.
- **Two kinds of event are automatic, and neither carries agent content.**
  Enrolment, an owner's act, shows the label peers address (harness, place,
  host, branch at that moment). A handoff's receipts carry its state and a
  reason code, by the consent of accepting it. They still reveal that the agent
  was busy or waiting on that handoff, and nothing else about the session.

**I go further than either review here, and differ from one.** Kimi offered
"publish only turns causally initiated by the channel" as an alternative. I
reject that as a default for three reasons:

1. A channel-started turn can quote anything already in that session's context.
2. A line typed locally mid-turn joins the same turn (`turns.rs:314`, **read**),
   so a turn's cause is not one thing.
3. "Started by the room" says nothing about what the reply contains.

Codex allowed either an agent's direct `channel_post` or owner review. I pick
review for the first slice, because a post tool the agent can call by itself
is an exfiltration path that raises no prompt, and pulled room text is exactly
what could instruct it to use it. The cost is friction, and the rehearsal
measures it. A later per-handoff "publish the result without asking" is
reasonable if the numbers ask for it.

**A reusable place is not a public place.** Enrolment (§4) makes an agent
*addressable*, and only a handoff's own events reach the room. Later work in
that place produces nothing in the channel. What enrolment cannot do is empty
the session's memory: a request from the room can elicit what the session
already knows. So the card recommends a **fresh place and session for room
work**, and warns when the target session has a history that predates the
enrolment.

**Prompts.** Private to the owner. On a handoff's own turn, a peer may see a
coarse "waiting for David". Prompt text, commands and diffs are not shared,
since a short command can already contain a token. An owner can paste what they
choose. The first slice has no "peer may read prompts" switch.

**Lifecycle, as a product rule:**

| Question | Rule |
|---|---|
| New member's history | From the join onward. Earlier history only as an explicit shared range, chosen by the opener, recorded as an event. |
| Removal | Future access ends at once, and in-flight proposals cancel. **Copies already delivered cannot be recalled**, and the UI says so at publish time and at removal. |
| Retention | Kept on the host until the channel is deleted. Not `messages`' 500 lines and seven days, which suit agent mail and would erase a human mention before someone returned. Export is a file. Deletion removes the host's copy and asks clients to delete theirs, which is a request, not a guarantee. |
| At rest | Plaintext in the host's config dir at 0600, as serve's other state is. Disk encryption is the machine's. Say that rather than imply more. |
| Audit | `audit.jsonl` records handoff id → attempt → outcome → publication. Length and hash alone cannot reconstruct why an agent acted, so the correlation id is the point. The channel log is not the audit log. |

---

## 4. Identity

Names render. Ids decide. **Read:** there is no place id today (the slug is the
handle), a session name is `<prefix>-<slug>` and comes back after a recreate,
and `Turn` has no id at all, only `at` in epoch seconds
(`turns.rs:70`). So most of this table is new.

| Id | Is | Changes when | Exists today? |
|---|---|---|---|
| **Channel** | Opaque, minted by the opener's serve | Never. A lost host means a new channel (§8.1). | No |
| **Host** | The serve's identity key | Machine re-identity | Yes, `server-identity` |
| **Person** | Channel-local member id, with enrolled devices. Display handle is an alias, and history stores the id. | Never. Renaming a device must not rewrite authorship. | No. `principal` is the base, but `device_name` is per device |
| **Device** | The paired device key | Re-pairing | Yes |
| **Project binding** | (channel, host, local project id), made by hand, once | Replacing it requires confirmation and is an event. | No. Registry key is a path, and the entry grows an id. |
| **Agent enrolment** | (host, binding, place id, generation) | **Every** re-enrolment. A new agent participant, never an inherited one. | No |
| **Session incarnation** | Enrolment + tmux session created-at + harness session id | A new launch, or a resume that changes the id | Partly. Claude's probe `session_id`, Codex's held thread and pi's generation are all there. **Hypothesis:** how `/clear` and `open`'s resume move these per harness has to be measured. |
| **Turn** | Incarnation + the harness's own id for the turn | Each turn | **Contract.** The ids exist and `Turn` drops them: Claude entries carry `uuid` (**measured** on this session's transcript), Codex's rollout carries `turn_id` (`codex.rs:370`, **read**). pi and kimi: unread. |

**Rules:**

- A mention resolves in the composer to explicit ids, shown as you type
  (`Claude · fix-api · David's Mac · branch x`), and history stores the ids, so
  nothing is reparsed later. Server-side, an event naming no id is plain text.
  The composer's parse is never the authority.
- Two `fix-api` places on two hosts are two agents, and the picker shows both.
  A slug addresses nothing.
- Deleting a place ends its enrolment. Recreating the slug is a new participant
  with no context, no cursor and no pending handoffs.
- A binding states that **someone chose it**, not that two checkouts share code,
  APIs or history. A fork binds fine. Every card shows the real host, tree and
  branch that would receive it.
- `@worktrees:place://…`, Claude Code's own mention token
  (`mention.rs`, **read**), stays literal text.

---

## 5. The handoff contract

A handoff is an expiring action request. It is **not** a chat line with
side effects. History replay never executes anything.

### 5.1 Envelope

```
handoff {
  id            client-generated; dedup key per writer
  channel, seq, epoch, membership_epoch     assigned by the sequencer
  writer        person + device, as sequenced (§2.3)
  writer_kind   human   (an agent cannot create one, §7)
  target        agent enrolment + session incarnation
  expects       idle | may_queue_behind(turn) | any
  request       one line of text, delivered whole
  context       immutable [seq_a..seq_b] minus excluded ids, + hash
  base_seq      the last seq the writer had seen
  expires_at    default 30 min (starting value, to measure)
  supersedes    optional handoff id
}
```

Receipts are separate events keyed by `(handoff id, attempt)`.

### 5.2 States

| State | Means | Where it comes from |
|---|---|---|
| `requested` | Sequenced, waiting for the owner. Executes nothing. | Coordinator |
| `declined`, `expired`, `cancelled`, `superseded` | Terminal, nothing was sent. | Owner, clock, writer or owner |
| `accepted` | The owner approved this exact request and bundle. | Owner's client |
| `refused` | One attempt, **no key sent**: wrong incarnation, or `ERR_PROMPT_UP`, `ERR_MID_INPUT`, `ERR_NO_COMPOSER` (`agents.rs`, **read**). | Target host |
| `queued` | Taken, behind a running turn. Not yet read. | pi and kimi: `SendOutcome::Queued` (`pi.rs`, `kimi.rs`, **read**). **Claude and Codex never return it**: behind a turn they answer `Submitted`, whose note says "if it was working" (`agents.rs`, **read**; v1 got this wrong). For them this state is **contract**: derived from a busy capture taken at submission, until `observed`. |
| `submitted` | Taken by the agent. A retry would deliver it twice (`SendOutcome`'s doc, **read**). | `SendOutcome::Submitted` |
| `unknown` | It may or may not have gone in: a modal opened, submission unconfirmed, Enter failed, or a crash between the journal and the result. | `Modal`, `Unconfirmed`, `EnterFailed`, a dangling journal entry |
| `observed` | The line, carrying its handoff id, is in the transcript. **Says nothing about whether the model obeyed it, or still remembers it.** | Claude, Codex. pi and kimi transcripts are not read, so they stop at `submitted`. |

There is no hidden queue. Acceptance makes **one** attempt. After `refused`, the
owner re-attempts by hand, and the card explains why it was refused.

### 5.3 What the agent receives

The rule is `harness::typed_line_ok`, which **refuses** what it does not like.
v1 said it strips, which is wrong, and silently changing a pasted command would
change what it means. One line, at most 4096 bytes, **including** our envelope
(`harness.rs`, **read**), so:

```
[worktrees: request from Bret in "<channel>", accepted by David, handoff <id>] <request> [context: .worktrees-attachments/<file>]
```

- The composer counts the remaining bytes. Too long or multi-line text is not
  cut: it moves to the bundle, and the card says so. Kimi asks that an addressed
  instruction be delivered in full, and refusing rather than truncating is how
  that holds under the line limit.
- **The context is an immutable file, not a moving read.** v1 pointed at "lines
  since you last read", which changes as people type and which an agent may skip.
  Instead the owner approves a range, excludes what they want, and the client
  writes it through `agent_attach`'s folder. That folder already has random
  names, 0600, a seven-day sweep, refusal of planted links and a git exclude
  that must be seen to take effect, and every harness can read a text file
  (`attach.rs` header, **read**). No new MCP tool for reading the room. The
  folder's sniff refuses control and invisible characters, so a pasted log with
  ANSI colour is refused rather than cleaned, and the owner sees why.
- **The bundle keeps authorship on every entry** (human or agent, who, which seq),
  says it is material and not instructions, and says if history is missing.
  **That is framing, not enforcement.**
- **`read` ≠ used.** A transcript can show the file was read, which is all this
  system can claim. An agent that skips it has only the request line, so a
  request must stand on its own.

v1's contradiction (its D4 promised a digest and its body a pointer) is resolved
with neither: full request plus a bounded bundle. A digest would put an editor
between people and their agent.

### 5.4 Rules

- **Idempotent.** A client that times out retries with the same id. The
  coordinator dedups per writer and returns the original seq. Chat lines use the
  same mechanism.
- **Journal before paste; never blindly retry.** The target host writes
  `(id, attempt, incarnation)` locally before `agent_send`. On restart, a dangling
  entry is reconciled by searching the transcript for the id: found becomes
  `observed`, not found stays `unknown`. `unknown` is never retried by software.
  The owner looks, then chooses "attempt again", which is a new attempt with a
  warning. The row's own note already says "look before sending it again"
  (`send_note`, **read**).
- **Recheck immediately before execution**, under the place lock: not expired,
  writer's membership current, incarnation matches, `expects` holds, not superseded.
- **Conflicting human instructions.** Agents never arbitrate. On the owner's own
  agent, **the owner's word wins**, and every other request is a proposal. A
  later request to the same target from anyone shows on the earlier card as a
  conflict before accept (compare `base_seq`), and the owner chooses replace,
  keep both or decline. Arrival order at the host decides nothing here.
- **Supersession and cancellation.** The writer or the owner may cancel anything
  unaccepted. A peer's "wait" on someone else's request raises a visible hold and
  does not cancel it.
- **After submission nothing can be taken back.** `queued` has no cancel in this
  design (**hypothesis:** whether a queued line can be removed from a harness's
  queue is unmeasured). What remains is a correction, as a new handoff, or Stop.
  Revoking a member does not stop what an agent already holds. The UI says this
  in these words.

---

## 6. Stop

**Primary:** a Stop control on the agent's row in the room, showing the turn it
is about (who asked, how long ago).

| Outcome | Means |
|---|---|
| `requested` | The control fired. Not yet stopped, and displayed that way. |
| `confirmed` | One Esc pressed and the turn seen to end. **The turn ended. Its effects stand**: an external operation already launched is not rolled back. **Hypothesis:** what happens to a running child per harness needs measuring before any UI implies more. |
| `refused` | Nothing pressed, with the row's reason: prompt up (show the owner's own prompt), draft, idle, session gone, or the turn already over. Also pi and kimi: `agent_interrupt` refuses them today (**read**), so the room offers no Stop on those agents and says so. |
| `unconfirmed` | Pressed once, not seen to stop. **Never pressed twice**, as today's row promises. "Look at the agent." |

**What the row needs, and lacks.** `agent_interrupt` takes `repo` and `slug` only
and presses on the **first** lane showing a running turn (`agents.rs:858`,
**read**). That is not safe for a target-bound stop. It gains two optional
arguments, session incarnation and turn, and refuses on mismatch.

**A late stop.** David sees turn A running and presses Stop. By the time it lands,
turn B is running. Today's row would stop B, because the screen is busy. With
binding, the result is `refused: that turn ended`, and the UI shows what is
running now with a fresh button.

**Residual race, said plainly:** the place lock serialises our clients, and it
does not stop Claude advancing. A turn can end and a queued line can start one
between the read and the Esc. Binding narrows this window and cannot close it.
Stop requests expire in about 30 s, a starting value.

**Text.** One shortcut exists: exactly `@<agent> stop`, resolved in the composer
to the structured action with the target shown before send. What travels is the
action. **No serve parses text to act.** So:

| Typed | Result |
|---|---|
| `stop`, bare | A chat line. The composer offers "Stop an agent?" and lists running ones. It never acts by itself, since it may be aimed at a human. |
| `claude, stop please`, `stop!`, `para`, `arrête`, `no, wait` | Chat lines. Natural language **is not recognised**, in any language. The same offer appears for a close match, and it is a hint, never a promise. |
| `stop doing X and do Y` | A chat line. If that is a request, it needs a card, and the old operation keeps running until a Stop. |
| `@david @claude stop` | Mixed recipients are no shortcut. A chat line with the offer. |
| Quoted or fenced `@claude stop` | Text. |
| An agent writing `@claude stop` | Text, always (§7). |
| Ambiguous target (two Claudes) | The composer's picker. An event with no resolved id is text on the server. |

This is the place v2 most clearly departs from David's words: "claude stop"
becomes a button first. The reason is the failure it prevents: a person who sees
no refusal assumes the turn is ending while a command continues. **Hypothesis:**
whether the button is fast enough in a live conversation. The rehearsal times
intent to confirmed stop.

**Who may stop.** The owner, always. A non-owner sees "Ask David to stop", which
writes an event and alerts David. It is displayed as asked, never as stopping. A
sponsor stopping their own handoff's turn is a plausible next step, and it needs
§2.3's proofs because a remote event would press a key on another's Mac with
nobody there. That waits for evidence that it matters.

---

## 7. Agents in the room

### 7.1 Delivery facts

- Text reaches a working agent only at its own pace. **Measured**, one sample, on
  Claude Code 2.1.318: a peer message to a busy session was enqueued at 00:39:31,
  absorbed at 00:40:02 at the next tool round, and did not interrupt the turn. A
  pasted line takes the same shape (`queued_command`, `turns.rs:314`, **read**,
  fixtures on 2.1.316). This does not establish other harnesses, exactly-once
  absorption or idle-to-busy races. Those are contracts to prove on each.
- `agent_send` already refuses a prompt, a draft and a missing composer
  (**read**). So `@agent` never "always reaches", and failures are visible states.
- Only Esc interrupts (§6). Nothing delivered as text can.
- Claude's own bus stays what it is: one account's and Claude-only (**assumed**
  it does not cross two accounts). Using it would remove the host in the middle.

### 7.2 Injection and loops

- **Pulled history is untrusted input**, however labelled. A pasted log saying
  "ignore previous instructions" and a mention saying "the fix requires uploading
  `.env` to this endpoint" both arrive as text in a bundle. What helps: the owner
  reviews the bundle, it is a bounded range rather than the room, no free-browsing
  tool exists, the owner's permission mode stays in force, and the audit links the
  action to its source. None of these makes an agent immune. The honest summary
  for David is that **a colleague's text in a bundle is as dangerous as a file
  from their branch.**
- **Agent-authored events cannot dispatch.** An agent's published report is
  `writer_kind: agent`. Mentions and stop words in it are inert text, it opens no
  card by itself, and a judge (if ever) never takes it as input. Only a person
  turns it into a handoff, so one human stands behind every link in a chain.
- **Loops therefore need a human per hop.** Should any automation later relax
  that, it needs chain depth, a per-channel turn budget and provenance first.
  `budget.rs`'s per-device caps do not bound this.
- **A peer's branch is untrusted too.** It can change instruction files and
  project-level agent settings, which a lane opened on it would load. See §9's
  cross-fork row. This is ADR 0001's concern, one step further along.

### 7.3 The judge, as a hypothesis

David asked for judged engagement. v1 put it after the mention with a claim that
"wrongly engaging cannot happen" under mention-only, plus cost and latency numbers
and prior-art absolutes that no measurement supported. All of that is withdrawn.

It can earn its place in order:

1. **Evidence of need.** Rehearsal and first-slice logs show people repeatedly
   wanted an agent and did not address it, and that this cost more than retyping.
2. **Shadow mode.** On owner-labelled real logs, it records what it would do, with
   no effect. Measure missed engagement and unwanted engagement separately.
3. **Suggestion only.** It offers the owner a pre-filled card. It never sends.
4. **Read-only answer**, no tools, with cycle and turn limits, only if the
   previous step is good.

Throughout: it never mints authority, never reads agent-authored text as a
trigger, and treats `waiting` or `unknown` as not idle. v1's §1.2 said "idle or
waiting" in one place and "idle" in another. Waiting on permission is exactly
when not to paste.

---

## 8. Coordination and transport

### 8.1 The coordinator

**One sequencer per channel: the opener's serve.** It owns enrolment, membership
and its epoch, `seq`, idempotency, the log, retention, cursors and routing of
action events. **Read:** v1 pointed at `messages::new_id` as the precedent, and it
cannot be: it is wall-clock milliseconds + pid + a process-local counter
(`messages.rs:106`), so clock rollback and restart break its order. One file per
message gives atomic visibility, not a transaction, and these ids are not
"content-addressed" as v1 said. The coordinator needs:

- a persistent, monotonic `seq` written durably before acknowledgement;
- an `epoch`, which never changes in the first slice;
- a dedup table per writer;
- explicit gap detection: a client knows its `last_contiguous_seq`;
- retention and deletion as events, so a gap can be told from a removal.

**What `seq` does not do.** It orders arrival at the host. It does not order
execution on another host. An instruction at 10 and a stop at 11 can race on
their way out. So stops are targeted by **turn**, handoffs carry **expects**, and
in the first slice the owner's acceptance is what orders execution.

**Cursors are three different things:** read state per **person** (read on the Mac
clears it everywhere), a cache position per **device**, and delivery state per
**session incarnation**. An agent has no "last read": it has the bundles it was given.

**Outage.** An app-owned serve stops 120 s after its last console client
(`OWNED_IDLE_GRACE`, **read**), and a device never holds it up. The rule stays.
A host wanting to serve while away runs `worktrees serve` by hand.

- Host gone: the room is read-only and shows "history through seq N". Clients may
  have stopped at different N, and that is displayed, not hidden. No client is
  called the authority.
- Drafts stay local and are sent only by a person, under the same id on return.
  Nothing queued is drained onto an agent.
- An agent host offline, with the room up: `requested` waits, and expires.
- An accepted handoff's attempt is local, and its receipts are appended later,
  idempotently.
- **No failover and no migration.** If the host is lost for good, export what you
  hold and open a **new channel**. Merging would need fencing and an authority
  decision. v1's "append the old log and announce" had no answer for a partition
  in which both sides accepted writes.

### 8.2 Reachability, if the slice is native

The pick stays Tailscale, but with a smaller claim. A member's Mac pairs to the
host's remote listener with a channel-only grant (private address class, mTLS, peer
filter, audit, kill switch: **read**, `serve/remote/`). **One way**: a member dials
the host. v1's bilateral pairing is gone, because no host needs to reach the other's
engine. Bret's own agents are driven by Bret's own loopback, and that host writes
receipts as a member.

This is not "nothing new":

- a person-to-person pairing link and a longer window than today's 120 s;
- a channel capability and object;
- pairing for someone else's device, which has not been done;
- Tailscale as an operational dependency;
- step-up: a Mac has no Face ID key. A channel-only grant holds nothing drive-class,
  so nothing requires it yet.

**v1 overstated its case against a relay.** Private-bind policy forbids a public
*listener* on the owner's machine. It says nothing about a client making outbound
connections to a relay. Privacy, operations, and who holds the log are real reasons
to prefer the overlay, and they are reasons to weigh, not a prior refusal. The same
goes for a third-party room as the record: it is an alternative to test (§1), not a
disqualified one.

### 8.3 The boundary, corrected

v1's `ChannelTransport` (`append`, `read(since)`, `members()`, …) was the coordinator
with a new name. Two interfaces instead:

- **Message transport:** authenticated send and receive of opaque frames between
  known keys, with reconnection. Provides peer identity, confidentiality and a
  delivery attempt.
- **Coordinator:** everything in §8.1. Above it sits the canonical envelope and action
  model (§5), which does not depend on the wire.

The agent half (`agent_send`, `agent_interrupt`, the place lock, the grant check, the
audit) stays on each owner's serve whatever is chosen.

**No swap is promised.** A candidate is tested against the contract, and "it could
replace the first implementation" is a claim only once a second implementation passes
all of these:

| Test | Question for a candidate (SimpleX or another) |
|---|---|
| Order | Who mints one order? A queue-based messenger typically has no such thing, so a designated member sequences, which is the coordinator again. |
| Replay and gaps | Can a returning member detect a gap and get it filled, and from whom? |
| Membership | Who may add or remove, is it enforced, and what does a removed member still receive? |
| Offline | Bounded in age and count? Person text may wait; **an action must expire** whatever the transport holds. |
| Identity | How is a transport identity bound to the person and device ids, by a deliberate act? |
| Encryption | If nothing in the middle can read, each member's own serve holds the log, and sequencing has to be re-decided. |
| Bounds | Its own size and rate limits versus 8 KiB events. |

The SimpleX lane should answer these, not fit five methods. It can also say what it
would buy (no tailnet for the human half) and what that costs. Phone-as-member is out
of scope until the phone itself is earned.

---

## 9. Failure scenarios

Each traces the model. The last column says what stays exposed.

| Scenario | What happens | What is still true |
|---|---|---|
| **Conflicting instructions.** David's card: "update the migration". Bret's card: "do not touch it yet". | Each to the same agent. The later card shows the conflict; if the agent is David's, David decides. If it was submitted first, Bret's becomes a hold plus an alert, and David can Stop and correct. | Work done before the correction stands. Agents never arbitrate. |
| **Duplicate retry.** Bret's client times out after its post landed. | Same id, the original seq returned, one card. On David's side an `Unconfirmed` send is `unknown`, and not retried. Restart reconciles by transcript search. | pi and kimi cannot be reconciled, so they stay `unknown` until a person looks. |
| **Same slug, two hosts; place reuse.** Both have `fix-api`. David deletes that place and recreates the slug for something else. | Two enrolments, and the picker shows both. A delayed card names the old incarnation and is `refused`. The new `fix-api` is a new participant with nothing inherited. | Requires the place id and incarnation (§4), which do not exist yet. |
| **Host sleeps, mirrors diverge.** Bret has through 40; David's client missed 39. | Read-only for both, each showing its own last contiguous seq. No new writes. On return, gaps fill from the host and no client is the authority. | If the host never returns, a new channel. Divergent copies stay unreconciled. |
| **Late stop.** Stop aimed at turn A; B is running on arrival. | `refused: that turn ended`, with a fresh button on B. | A narrow race remains between read and Esc (§6). |
| **Private work in a shared place.** A week after a handoff, David uses the place for a credential issue, and Claude prints a `.env` line. | Nothing reaches the room, because nothing reads transcripts for it. Only a publish could send it, and it is previewed. | If a session with that context later takes a handoff, it may repeat it, so the draft preview is the last defence. The card warns; a fresh session is recommended. |
| **Cross-fork handoff.** Bret's fork has branch `fix`; David's has a different `fix`. | The card carries the source repository as Bret's host names it, an immutable **SHA**, and the branch only as a label. David's client fetches under `fetchpolicy`'s **device** rules even though David is the console, because the URL came from a peer (console git is otherwise unrestricted, **read**). It verifies the SHA, reports merge-base and divergence, then David confirms a new place. | **v1's "click and it fetches" is withdrawn.** The peer's branch is untrusted: David reviews before launching an agent on it, because instruction files and agent settings arrive with it. A SHA proves which commit, not that it is good. |
| **Owner away.** David accepted nothing and left. | `requested` until expiry, and Bret sees that. Bret's own agent is still Bret's. | This is the cost of D3, and the main thing the rehearsal measures. |

---

## 10. Experiment and gates

### 10.1 The rehearsal (no code)

Two people, two Macs, a real shared task, different checkouts and ideally one fork.
Sessions of 60–90 minutes. Use an existing private chat the two already trust, on the
understanding that branch names and task details reach it. Three arms:

- **Control:** ordinary chat and today's separate windows.
- **Cards:** every handoff is a written card: target (host, owner, place, branch),
  outcome wanted, source messages, expiry. The owner relays it through their own
  composer or `agent_send`. The other person cannot touch that agent. Each owner
  publishes a short result, with repository and commit, and keeps transcripts and
  prompts local.
- **Facilitator:** one shared agent for a shorter run. Do two personal agents add
  anything?

**Deliberate drills in the Cards arm:** one conflicting pair, one duplicate relay, one
place replaced mid-task, one Stop during a long operation, one person offline, one
cross-fork handoff, one session with private context asked to publish.

**Record:**

- handoff time, context omissions and repeated explanations;
- wrong-tree or wrong-target requests, duplicate and late actions;
- owner mediation time, and how often the other person was left waiting;
- accidental disclosure;
- intent-to-confirmed-stop, and what the operation did after Esc;
- whether each person can explain who requested and performed each change;
- how often someone wanted an agent and did not address it (the judge's only evidence);
- fallback to terminal or other chat.

**The bar, set together beforehand.** My proposal: zero wrong-target and zero unintended
publication, plus a clear reduction in handoff and context-repair effort over Control.
Two people and a few sessions give direction, not reliability. It proves nothing about
machinery a human is standing in for.

### 10.2 The gate

| Result | Next |
|---|---|
| Cards beat Control, mediation tolerable | The first native slice (below), or a bridge if the external room served well. |
| Value appears, mediation is the pain | Consider §2.3's preconditions for peer direct instruct. |
| Facilitator matches Cards | Build the simpler product. |
| External chat was fine | Compare a constrained bridge against building chat, on privacy and cost. |
| People kept missing addressed moments | The judge's step 2 (§7.3). |
| Cards do not beat Control | Stop. Do not expand this into a chat product. |

### 10.3 The first native slice, if earned

**In:** two hosts, one enrolled agent each, the coordinator (§8.1), a channel capability,
the §5 handoff with cards and journal, owner accept through `agent_send`, a bundle via
attachments, owner-approved publication, §4's ids, §6's Stop with its two new arguments,
receipts, public `@person` mentions as room text, and a per-person read marker. Harnesses:
Claude and Codex first, since those are what `observed` and Stop can serve.

**Out until earned:**

- phone (no iOS work, push, or phone-as-member; the design's `notifications.md` rule of
  no "message arrived" event stands);
- DMs, cross-channel inbox, OS notifications;
- host migration or failover;
- Slack mirror;
- judge;
- peer instruct or peer Stop;
- shared prompts;
- a tool for browsing the room;
- alternate transport.

v1's estimate that phone work is "moderate" is withdrawn along with it.

**Must pass before anyone relies on it:** deterministic tests for each §9 row; a
kill-at-every-step test of the journal (dead after journal, after paste, before receipt);
a test that a replayed history executes nothing; a test that no agent-authored event
produces a card; a test that a stale incarnation refuses with no key sent; and **one lane
driven end to end in the real app on two real hosts**, per `docs/adding-a-harness.md`'s
lesson that a core report is not what the user sees.

### 10.4 What should not be built

- A silent queue that drains onto an agent.
- Any serve-side text parser that acts.
- A transcript mirror with a filter on it.
- A channel-level "approve".
- A host-asserted `Caller` for a person who is not on that socket.

---

## 11. What must be true, and what is unknown

**Must be true for the model to hold** (contracts, each testable in the slice):

- Only the owner's act submits to an agent. Replay, retry and agent text never do.
- No agent content leaves a host except by owner publication. Automatic events
  carry labels, states and reason codes only.
- A target is ids, and a stale id refuses with no key sent.
- `unknown` is never retried automatically, and Stop never presses twice.
- Text is refused rather than altered.
- Every action correlates from request to outcome in the audit.
- Membership and expiry are rechecked immediately before execution.

**Hypotheses**, for use or measurement to settle:

| Question | Settled by |
|---|---|
| Do two local agents beat ordinary chat, or one facilitator? | Rehearsal |
| Is owner mediation tolerable, or is it the whole product's problem? | Rehearsal |
| Is a Stop button fast enough mid-conversation? | Rehearsal timings |
| Is explicit addressing enough? | Rehearsal count, then shadow mode |
| Do people publish enough, or drop the drafts? | Rehearsal |
| What survives Esc per harness (child processes, background tasks)? | A probe per harness |
| How do session ids behave across `/clear`, resume and `open`? | A probe per harness |
| Can a queued line be retracted? | A probe per harness |
| Mid-turn absorption, exactly-once, idle-to-busy, beyond one 2.1.318 sample | A probe per harness and version |
| Can the host positively read a session's permission mode? If not, the worst-case wording stays. | A probe; the wording does not wait for it |
| Is a person-to-person pairing tolerable to set up? | The slice |
| Do budgets sized for a phone fit a Mac client? | The slice |
| Does any second transport meet §8.3? | The SimpleX lane |

**Where I differ from the reviews, in one place:**

- **Direct peer instruct in the first experiment (Kimi):** deferred, in favour of Codex's
  owner relay. It is the highest-risk grant in the system, and the bet can be tested
  without it. If mediation turns out to be the pain, the gate reopens it.
- **"Confirm tool use" for remote instructions regardless of mode (Kimi):** the goal is
  right, but this host cannot enforce a per-tool ask on a running session it did not launch
  that way. Acceptance per handoff, plus an unconditional warning and a recommended fresh
  session, is what can actually be delivered. **Hypothesis:** launching room lanes in a
  stricter mode may do better.
- **Excluding unprivileged lines from agent context (Codex's first option):** not taken. A
  colleague's analysis is why you want a shared room. The owner chooses, and the document
  admits what that means.
- **Causal turn publication (Kimi) and direct agent posting (Codex's first form):** §3.
- **Full common history as a product bar (Kimi):** kept for what the sequencer holds. Not
  offered as a claim about any client during an outage.
