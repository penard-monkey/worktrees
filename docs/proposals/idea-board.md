---
title: "Proposal — ideas: thinking before a place exists"
---

# Proposal — ideas: thinking before a place exists

**Status:** product exploration, **version 2, 2026-11-06**. It replaces v1
(`815fcb0`, the same day) after independent reviews by Codex and Kimi. Nothing
is built and nothing here permits building it. "Ideas" remains a working name.
Implementation detail is deliberately shallow. The choices that need the owner
are in §13.

**The spark.** The owner wants to capture something like this while working:

> Clone Jellyfin server and get it working as a project. Make two Jellyfin
> servers on remote systems behave as one: my server and a friend's appear
> unified; if one disconnects, inform the client and show its content as
> unavailable.

Today that sentence has nowhere to go inside worktrees. It is not a branch, it
is not yet a project, and it may never become either. Worktrees starts at "I
know which repo and which branch". The thinking before that happens in a notes
app, in a chat that gets lost, or in a lane created too early.

**Evidence base.** Terminology and the mechanisms named here were checked
against this repo at `0e86349` (v0.48.0). Jellyfin facts come from one short
research pass on 2026-11-06, were not re-researched for v2, and are labelled
where they appear. No app was run and no Jellyfin code was read. **No experiment
in this document has been run, so its claims about what people will do are
hypotheses.**

## What changed from v1

v1 recommended a new top-level Ideas destination, with threads as the second
slice and "the real bet". The reviews agreed on the failure: **v1 removed stage
upkeep and left reconciliation undefined, and its first slice did not test its
bet.**

| Topic | v1 | v2 |
|---|---|---|
| Recommendation | Build a destination, thoughts and idea pages first | Rehearse by hand first. If code follows, the first code is a **bridge** beside the owner's notes (§11) |
| The bet | Threads beat a notes app plus a chat | **Continuity:** you can resume from what you kept, and results deliberately change it (§1) |
| `Needs you` | A strict, derived group | Removed. Live facts, plus one explicit review state (§4) |
| Work → memory | Status flowed back; conclusions did not | Harvest is the one door, for chats and lanes alike. Merged is not answered (§6) |
| Work link | Project + branch, place looked up live | A durable **commission**, separate from a live link that can fail (§5) |
| Harvest | Add-only, with thread + date | A freshness contract: as-of, reviewed-through, supersede, dispute, a basis per claim (§6) |
| Threads | A second slice with an open runtime | Not an object yet. A no-code commission stands in, and the runtime is an explicit gate (§7) |
| Plain files, MCP, four capture doors, second window, shared selector | Promised or implied | Candidates, each with what would earn it. Agent-filed thoughts and ambient idea reads are dropped (§2) |
| Stop rule | "Ever revisited, ever handed off" | Named tasks, thresholds and a table saying what each failure means (§11) |

Kept: thought and idea as separate objects, open/parked/closed, no stages, the
original words, reviewed memory, person-only decisions, fresh sessions from
reviewed briefs, and non-parent links. §12 records where this revision disagrees
with the reviews.

## The recommendation in one paragraph

**Do not build an Ideas destination yet.** Run a two-week, no-code rehearsal on
3–5 real ideas, in the owner's notes app, using lanes and chats that exist
today, and end it with tasks that can fail in distinguishable ways (§11). The
product bet is continuity: after days away you can tell where an idea stands
from what you kept, and a result that contradicts you changes that record on
purpose. If the rehearsal shows the notes app is good enough as a home, the
first code is a **bridge**: start a lane from a reviewed brief, keep a durable
record of what was commissioned, and bring its result back through a deliberate
review, with the notes staying canonical. An in-app home for thoughts and ideas
is built only if the rehearsal shows the notes app is what failed. A new kind of
research session is built only after that, and only if a measured stand-in
proves insufficient.

---

## 0. The answer in one screen

| Question | Short answer | Section |
|---|---|---|
| What is the bet? | Continuity between thinking and doing, at a maintenance cost the owner willingly pays | §1 |
| What is fixed? | Eleven product invariants. Everything else is a candidate | §2 |
| Core objects | Thought, idea, commission. Claims, questions, options and decisions are items in an idea | §3 |
| Lifecycle | Open, parked, closed. Items have small person-set states, and v2 admits that | §3.3 |
| What deserves attention? | Live facts stay facts. The only owed act is reviewing a result, and reading does not clear it | §4 |
| What is a link to work? | A commission record that outlives its branch and place, beside a live link that may be gone | §5 |
| How do results return? | One Harvest door: reviewed against current memory, with a basis per claim. Merged is not answered | §6 |
| Where does research run? | Unresolved on purpose. A no-code commission stands in; a new session kind must earn itself | §7 |
| Who can read an idea? | An agent is given what was reviewed into its brief, and nothing ambient. That is a route rule, not a sandbox | §8 |
| How do you get back? | Search by remembered words, a short "Now" head, and links in both directions | §8 |
| Where does it live? | First the owner's notes, then a bridge in the app. A destination only on evidence | §9 |
| What runs first? | The rehearsal, with its failure table | §11 |

---

## 1. The bet, and what it must respect

**The bet.** A maintained idea lets you resume and choose correctly without
rereading transcripts, and it survives work that proves you wrong, at a cost you
willingly pay. If most of the value still comes from rereading chat, or the
upkeep is not worth it, the feature should not exist.

Capture is a real wish of the owner's and a separate question. It is cheap to
test and proves nothing about the bet, because a fast way to write something you
never read is still write-only (§11.4).

**The strongest competitor is not nothing.** It is the owner's notes app, plus
today's chats, plus a brief written by hand. That already supports a manual
hand-off. Each thing proposed here has to beat that, and the bridge has to beat
it before a destination does.

Four things already true of worktrees shape the model.

**A place is a durable location; a branch is work flowing through it.** So
thinking must not need a place. v1 drew the right conclusion and then stopped
too soon: a link cannot name "the branch" and be durable either (§5).

**The brief is the handover, and `.planning/` is a place's working memory.** An
idea must not grow a competing plan. But a brief is a *snapshot of intent*. It
implies no continuing synchronisation, and it is not evidence of what happened.
v1's "the lane did what the brief said" was a hope.

**The owner has rejected a tracker once.** PR #296 turned ROADMAP.md into 68
issues and was closed unmerged. So this must not become a project's backlog, and
stragglers keep going to that project's ROADMAP.

**Automations were built, then removed** (ROADMAP, 2026-10-10). The lessons
apply:

| What went wrong with automations | What it means here |
|---|---|
| The headline promise never ran | Show nothing that does not work yet. v1's threads were close to this |
| A project-wide thing in a per-place surface | Not a dock tab. Dock contents follow the selected place |
| Too much vocabulary before it proved useful | Few nouns, and items only appear once used |
| Its output overlapped surfaces already answering the question | Do not restate lane status. Link to it |
| "Start from one concrete sweep someone keeps asking for" | Jellyfin is the concrete case, and §10 must include its failures |

---

## 2. Invariants and candidates

v1 mixed promises about the product with guesses about how to build it.
Separating them narrows what has to be believed.

### 2.1 Product invariants

Whatever gets built, including nothing but a bridge, these hold:

1. **Capture asks nothing.** Text and a time, and it may stop there forever.
2. **Development is optional.** No project is required, and "use what exists" or
   "do nothing" is a success.
3. **The original words are kept** beside whatever the idea became.
4. **Memory changes only by the person's acceptance, and acceptance is not
   verification.** A kept claim can still be uncertain.
5. **Decisions are the person's, and old ones are superseded rather than
   rewritten.**
6. **Execution starts only from a reviewed brief.** Nothing runs in the
   background, on a schedule, or because a suggestion was not dismissed.
7. **An idea contains no work.** Closing it stops nothing, and removing a place
   breaks nothing in it.
8. **What was commissioned is recorded durably,** separately from whatever is
   live now.
9. **Results return deliberately, and merged is not answered.**
10. **The product gives an agent only what was reviewed for it.** No ambient
    read of your thinking.
11. **An idea can be resumed without its transcripts.**

### 2.2 Candidate commitments

| Candidate | v1 | v2 status | What would earn it |
|---|---|---|---|
| Markdown files as canonical store | Promised | **Narrowed:** prose is portable, product context is not promised (§9.2) | A rehearsal that keeps working in plain files |
| In-app top-level destination | Recommended | **Conditional** on the notes app being what failed | §11's failure table |
| In-app capture chord | Slice 1 | **Separate small test** | §11.4 |
| `worktrees thought` CLI | Slice 1 | Deferred | The chord proves useful, and capture from outside the app is what is missing |
| MCP tool for agents to file thoughts | Slice 1 | **Dropped.** Unsolicited volume before retrieval is proven, and an inbound path from repo content | Would need a new case, not more of this one |
| MCP tool for agents to read ideas | Slice 1 | **Dropped** in favour of the brief as the only route (§8) | A real task a reviewed brief cannot serve |
| Thread as a new session kind | Open question | **Gated** (§7) | §7.3 |
| One selector over chats and lanes | Core experience | **Dropped as a promise.** A commission opens its live session when there is one | A home exists and switching is the measured pain |
| Second window | "Cheap later" | **Claim withdrawn.** It may be cheap, but nothing here establishes that | Someone asks for it in use |
| Separate decision view, sort-through, similarity hints, card views | Slice 1 or later | Deferred | Each one's absence named as a problem in use |
| Phone capture | Next after threads | Deferred, with a scoping problem (§8.1) | A home exists |

The boundary "MCP is the agents' door, serve is the UIs'" is unchanged. It is an
existing fact of this codebase and no reason to widen either surface.

---

## 3. The model

### 3.1 Vocabulary

| Word | What it is | When you meet it |
|---|---|---|
| **Thought** | One timestamped line. No title, no fields. | Always |
| **Idea** | One living document with a title and the original words. | When you choose to develop a thought |
| **Item** | A claim, question, option or decision inside an idea. | When one is first written |
| **Commission** | The durable record of one piece of work asked for: brief, as-of, identity, result, outcome. | When you hand work off |
| **Result** | What a commission's agent or a conversation reports back. Evidence, not memory. | When work reports |
| **Harvest** | The act of reviewing any result into an idea. | When you choose to |

**"Thread" is no longer in the vocabulary.** The word stood for a runtime that
does not exist (§7). The conversations are real; a conversation is a source for
a Harvest.

Words still avoided because the app uses them: **note** (`set_note`), **hub**
(sync and nested lanes), **plan** (`.planning/`; an idea has a *direction*).
**Workspace** keeps its meaning: this host's registered projects.

### 3.2 The idea document

One document reading top to bottom. A two-line idea looks like two lines. A long
one gets a short head, because the resume test depends on it:

```
 # Two Jellyfins as one                        open · next (Nov 8): stop one upstream
 > Clone Jellyfin server and get it working as a project. Make two…     (captured Nov 6)

 Now        Can a stock client be told a server is down? Spiking the proxy first,
            because it leaves both servers untouched.
 Questions / Claims / Options / Decisions    appear when first used
 Commissions   C1 C2 C3, each opening its record
 Harvest log   what was reviewed from where, through when
```

**Now** is at most a few lines, written by the person, and offered for update at
the end of each Harvest. It is upkeep, and the rehearsal measures whether it is
worth it. **Next** is one optional line and shows the date it was set, so a
stale one reads as stale instead of contradicting newer facts.

### 3.3 The state that exists

v1 said "three declared states, the rest derived". That was not true of its own
examples, because an open question, a read result and a dismissed suggestion are
all state. The honest inventory is small and person-set:

| On | States | Changes when |
|---|---|---|
| Idea | open · parked · closed (done or dropped, with a reason) | The person says so |
| Question | open · answered · dropped | The person, usually in a Harvest |
| Claim | current · disputed · superseded | The person, usually in a Harvest |
| Decision | standing · reopened · superseded | The person, usually in a Harvest |
| Commission | live facts, plus review outcome (§5, §6) | Facts by reading; outcome by the person |

**Not a pipeline:** nothing requires moving through these, and no idea has a
stage. "Researched" or "planned" is still read off what the document holds. v1's
rejection of named stages stands, because stage upkeep is unpaid work and a
stale stage lies. The alternative is not "stages or inference". A plain list
with a dated *next* is a credible baseline, and it is what the rehearsal uses.

### 3.4 Why not "board"

A board is a promise of columns, and columns are stages. It also suits many
small similar cards, whereas ideas here run from two lines to a long research
document. No spatial view should be added before the retrieval tasks in §11 have
been tried.

---

## 4. Facts, acknowledgement, resolution

v1's `Needs you` promised "something only you can act on, and acting clears it",
all derived. It could not keep that promise. A finished thread stays finished
after you read it. A merged PR does not say whether the question was answered.
An idea can hold a blocked lane, a running lane and an open question at once.
**v2 removes the group** and separates three things that v1 ran together:

| Kind | Examples | Source | Can it be wrong? |
|---|---|---|---|
| **Live facts** | Session busy or idle, prompt waiting, PR merged, branch gone | Read on view, never stored as truth | Only stale by seconds. May be unavailable, and says so |
| **Acknowledgement** | "Reviewed through here" for one source | One marker per source, set by Harvest | Only by your own act |
| **Resolution** | Question answered, decision reopened, commission outcome | Person-set item state | It is a judgment, kept as one |

**The one owed act: a result to review.**

- **Enters** when a commission or conversation has a result newer than its
  reviewed-through marker.
- **Clears** only when the person records a Harvest outcome, including one click
  on *Nothing to keep*. Reading does not clear it, and neither does a merge, the
  session closing, or a later decision.
- **Comes back** if the same source later reports something new.

**An idle agent is not a result.** Idle can mean finished, stuck or waiting on a
permission that was skipped. A merge is not one either. They appear as facts:
"PR merged · no result brought back". That line owes nothing and clears nothing,
but a row saying it is honest about what is unknown.

**Rows show every fact, and no group wins.** A row can read "1 result to review
· C2 lane waiting on a prompt · 2 open questions". Waiting prompts stay with the
nav dots that already own them, and the row links there.

**A parked idea keeps its facts.** Parking is your attention, not the work's
state. Running commissions keep running (invariant 7), and their results still
count as results to review in that one filter. The row stays in Parked.

**Ordering is by when the person last touched the idea,** so a chatty agent
cannot promote an idea past a dormant question that matters more. There is no
priority field and no score. There is no count badge either, until use shows one
helps. `projectTodos.ts` already holds the principle: a count no button can
lower is a nag. This one could be lowered, and that alone does not make it
wanted.

---

## 5. The commission: durable identity, separate from the live link

v1 named a link by project + branch and looked the place up live. The reviews
showed why that fails, and **this lane is a witness.** Place `docs-idea-board`
on branch `docs/idea-board` received v1's brief, then this revision's.
`ops::write_brief` replaces `.planning/brief.md` atomically, and `.planning/` is
gitignored, so the first brief is gone from the place. Same project, same
branch, same place, two different commissions, and a link could not tell them
apart. It would also have offered today's session as if it were the session that
wrote v1.

### 5.1 Two records

```
 C3  Spike: stop one upstream                                Jellyfin idea
 ─ COMMISSIONED (never changes) ──────────────────────────────────────────
 Nov 8 · jellyswarrm @ <base commit> · spike/upstream-offline · Claude/Sonnet
 Should tell you: what a stock client shows when one upstream stops
 Brief (copied in full) ▸      As of: idea memory, Nov 8
 ─ WHAT CAME OF IT (added to, never rewritten) ───────────────────────────
 Session  <harness session id, when known>      PR  #41      Head  <commit>
 Result   brought back Nov 10 ▸                 Outcome  Answered: no
 ─ LIVE NOW (read on view, may be gone) ──────────────────────────────────
 Place has moved on: now on feat/other, re-briefed since.  [ Open place ]
```

- **Commissioned** is written once, at hand-off. The brief is *copied*, because
  the place's copy can be overwritten and is not tracked.
- **What came of it** is appended. The result is *copied* too, because the
  harness owns the transcript and may prune or move it. A session id is
  best-effort and is how you reach a transcript while one exists, never the
  thing relied upon.
- **Live now** is recomputed and never stored.

### 5.2 What the live link may say

| Live state | Shown as | Never |
|---|---|---|
| Branch in a place, brief matches the copy | The session, one click away | |
| Same branch, place's brief differs | "Re-briefed since this commission" | Presenting that session as this work's |
| Place moved to another branch | "Place has moved on" | Opening today's conversation as if it were this one |
| Branch deleted, or merged and gone | "Branch gone," with PR and commit if known | A fresh branch of the same name counting as this one |
| Project renamed or unregistered | Name as recorded, plus "not on this host" | Silently relinking by name |

**One commission belongs to one idea.** Two ideas relying on the same work is
common, and a merge cannot settle both of their questions. The second idea
references the work in prose or commissions its own check. A plain reference to
a PR needs no record; a commission is only what you asked for.

**Fan-out stays a set, not a tree.** One idea can hold commissions across many
projects. No epics, hierarchy or rollup. If two lanes must agree on something,
that goes into both reviewed briefs. Cross-project reach *permits* coordination
and does not supply it.

---

## 6. The return path and the freshness contract

### 6.1 Harvest is one door

v1 had Harvest for threads and a silent jump from "lane finishes" to a decision.
In v2 every source goes through the same review: a conversation, a lane's
result, or text the owner pastes from anywhere.

A result is written to a fixed shape that a reviewed brief asks for at its end:
*what was asked · what was done · what was observed · what this changes · what
was not tested.* That needs no code, and the rehearsal tests it.

### 6.2 Merged is not answered

Harvesting a commission records an outcome against what it was meant to tell
you. Git does not decide it.

| Git says | Outcome can be | Example |
|---|---|---|
| Merged | **Partly** | Proxy and web branches both merged; together they were never tried with a stock client |
| Merged | **Not answered** | The change landed; the question it was for needed real clients |
| Never merged | **Answered** | A spike was never meant to merge, and its finding is the point |
| PR closed | **Answered: no** | The approach failed, and that was worth learning |
| Anything | **Invalidated a decision** | §10.4 |
| Anything | **Abandoned** | The direction changed, and nothing is kept |

The idea closes only when the person closes it.

### 6.3 The review

```
 Harvest "Spike: stop one upstream"       drawn as of Nov 8
 Since then: Decision "Web client only" (Nov 9). This run did not know it.
 ─────────────────────────────────────────────────────────────────────────
 Should tell you   what a stock client shows when one upstream stops
 Outcome           [ Answered: no ▾ ]
 ┌ proposed ──────────────────────────┬ your memory now ──────────────────┐
 │ Answer  Library rows empty, items  │ ? What must a stock client show   │
 │  fail with a generic error         │   offline?                        │
 │  ran · 2 local servers · spike log │                                   │
 │ Trips   "Reopen if stock clients   │ Nov 8 Spike the proxy first       │
 │  cannot be told anything useful"   │                                   │
 │ New ?   Can a patched web client   │                                   │
 │  alone carry the badge?            │                                   │
 └───────── [Keep] [Edit] [Skip] ─────┴───────────────────────────────────┘
                                 Update Now?   [ Nothing to keep ]  [ Done ]
```

**A review never hides the memory it is changing.** v1's chat replaced the
document exactly when the two needed comparing.

### 6.4 The contract

**Two markers carry the freshness rule:**

- **As-of**, on a commission: which revision of the idea its brief was drawn
  from.
- **Reviewed-through**, on a source: how far that source has been harvested.

| Case | Rule |
|---|---|
| What a new run receives | Only what its reviewed brief contains. A snapshot, labelled with its as-of. No shared live document |
| Proposal made from an older snapshot | Reviewed against **current** memory. The review names what changed since as-of |
| Two sources in parallel | Each is reviewed against memory as it stands, so the second sees what you kept from the first |
| Repeated Harvest of one source | Proposes only what is past reviewed-through. The Harvest log, with what you skipped, is given as input |
| Duplicates | Reduced, not eliminated: proposals are model output. A duplicate is one click to skip, and no promise is made beyond that |
| A skipped item | Stays skipped, and may return only with new evidence that says so |
| You edited an accepted item | **Your text wins, always.** No later Harvest overwrites it. The log keeps the original proposal, and the item says "edited by you" beside its basis |
| A proposal changes an edited item | Shown as a replacement against your current text. You choose |
| A claim is overtaken | A proposal may *supersede* a named item. The old one leaves current reading, stays in history, and points to its replacement |
| A claim is contradicted, and you cannot say which is right | Both stay, marked **disputed**, usually with a new question. An unresolved dispute is a legitimate state |
| Decision's "Reopen if" comes true | A proposal may say so. You mark it **reopened**. A later decision supersedes it, and both stay readable |

**Model help is on a click, and flags nothing unasked.** A proposal can name a
conflict, and the marker can name what changed since as-of. No background
process decides that two things disagree.

### 6.5 Evidence that stands without the transcript

Every accepted claim that came from a source carries one line:

```
 Library rows go empty when one upstream stops.
   ran · two local servers, stock web client · spike log in PR #41 · Nov 10
```

- **Basis, one word:** *ran · read · summary · said · inferred · mine*. It
  records what was actually done, and acceptance does not upgrade it. A claim
  seen only through a search summary stays *summary* however confidently it is
  kept. This document's own Sources section draws the same distinction by hand.
- **What, a few words:** enough that the claim is intelligible and could be
  checked again.
- **Pointer and date, when there is one:** URL, PR, commit or file.

The test: someone who never saw the conversation can tell what was done and how
to check it again.

**What keeps this from being citation management:** no source records, no
bibliography, no deduplicating by source, no link checking, no confidence score,
and no evidence requirement for your own view (*mine* is a full answer).

---

## 7. The research runtime

v1 called threads the real bet while leaving unresolved where one runs. That hid
the product's central risk behind storage and UI work. Everything in the engine
is keyed place → session: the dots, chat rows, resume, limits, device
capabilities and audit. A reused chat view does not show that a checkout-free
session can browse, stop, resume or be limited under that model. "May change
nothing directly" was also a promise with no enforcement behind it, and it would
vary by harness.

### 7.1 What simplifies it

**A research question is a commission whose deliverable is a result rather than
a merge.** It uses the same reviewed brief, same record and same Harvest. Most
of what "thread" meant needs no second concept. So:

- **A question needing code needs a checkout, so it is an ordinary commission.**
  That closes v1's Q4 for now: a research task gets no special read of a
  registered project.
- **Reading a project you do not have means cloning it,** an explicit, visible
  act that today adds a project. A "reference clone" remains an open convenience
  (§14), not a boundary in the model.
- **"Changes nothing" is replaced by "is not given memory".** The idea's
  document is not in the run's tree, nothing names its path to the run, and the
  brief is a copy. The run can write what it likes in its own place, and no
  product route leads from there into what you have kept. **This is a route
  rule, not a sandbox.** An agent with full access on the same host can read any
  file you can, so the hard boundary is still the harness's permission setting.
  What this removes is the accident: an agent drifting into your thinking
  because a tool or a shared folder handed it over.

### 7.2 The first supported task, and three runtimes

**First supported task:** one bounded question, answerable from the public web
and the brief, ending in a result of the §6.1 shape. Nothing else is claimed.

| | **R0 Bring your own chat** | **R1 A place in a small research project** | **R2 A new session kind** |
|---|---|---|---|
| What it is | Any conversation outside worktrees, pasted into Harvest | A registered repo holding only briefs and results, one branch per question | Engine tracks sessions with no place |
| Stop, resume, dots, limits, chat view | Whatever that chat does | All work today | All must be built or generalised |
| Memory given to it? | No | No, if the repo holds no ideas | No, by design |
| Cost to the person | Copying, and no live status | Branch and nav noise. Unmeasured | Least, if it worked |
| Cost to build | None | None | Large: touches every place-keyed surface and the serve capability model |

v1 dismissed R1 as "exactly the 'not a branch' problem". **That was rhetoric.**
The original complaint was being forced to name a repo and branch before knowing
what the thing was. A branch per research question, in a repo that exists for
that and is created at the moment you commission one, is a different cost, and
it has not been measured. v2 uses R0 and R1 in the rehearsal, without claiming
that they test R2.

**What this gives up.** The owner's framing was a selectable chat that can
become a working session, and clarifying back-and-forth beside the document is
the part neither R0 nor R1 provides well. A commission is not a conversation.
The bet is being tested without it, and if that loss turns out to be the main
thing the owner misses, that is the most useful result the rehearsal could give.

### 7.3 The gate for R2

No R2 design is scoped until all three hold:

1. The rehearsal passes the memory bet (§11).
2. R1's costs, measured in use, are named by the owner as the blocker, or
   chatting beside the memory is.
3. A spike shows one harness running the first supported task, stopped, resumed
   and counted against limits, with its access stated.

**A click authorises a start, and not what the run then encounters.** A web page
can steer a research run. Quoting gives provenance and is not a boundary. §8.1's
rule limits what such a run is handed, and what it can actually do with shell,
files and network remains the harness's permission setting. This proposal does
not claim to improve that, and a research run reading the open web is a reason
to keep that setting narrow.

---

## 8. Visibility and retrieval

### 8.1 Who can see an idea

A workspace-level store has no project root, while everything else here is
scoped by one. One idea may touch personal and business work, and name a
friend's server. v1 called this a naming question. It is a privacy question.

- **An agent is given what was reviewed into its brief, and nothing ambient**
  (invariant 10). The brief is where thinking crosses into a repo, so it is the
  place the crossing gets checked. There is no MCP read of ideas: a place-scoped
  server has no sound rule for which of your ideas belong to it. As §7.1 says,
  this governs the product's routes and does not sandbox an agent that already
  has full access to the host.
- **A place's agent gets no pointer into memory.** The commission points at the
  place, and a brief may name its source idea only in words.
- **Crossing into a repo stays an explicit act in a place.** If a decision
  deserves to be an ADR, a lane writes and commits it.
- **Paired devices are the open edge.** Serve resolves each row against a
  device's project scope. A workspace store would be a new kind of object, and a
  device with a scope list should see none of it by default. **Phone capture
  therefore needs a design it does not have**, which is one more reason it
  waits. A commission is better placed, because it has a project to scope by.
- **Idea text is data, never instructions or argv** (ADR 0001 one level up). A
  clone URL or branch a suggestion names takes effect only when the person
  submits it, through the checks those paths already have.

### 8.2 Getting back

v1 shipped capture and hoped return followed. Each direction needs an answer:

| Getting back to | Answer | Where it may fail |
|---|---|---|
| A thought from months ago | **Search by remembered words**, over every thought, including developed ones, which point to their idea. Reverse chronology is for this week only | The words are not remembered; rehearsal task T1 |
| Where an idea stands | **Now**, next, open questions, at the top | Now goes stale; T2 |
| Why you decided something | The decision's text and its claims' bases | T3 |
| An idea's finished work | Its commission, whether or not anything is live | |
| **From a place, why it exists** | A person-facing "from idea…" on the place, when a commission exists. For the person only, and not for its agent | |
| Which idea had a result waiting | The one filter of §4 | |

**Dropped thoughts:** a drop removes one from the list and keeps it searchable
until the owner says otherwise. Losing a thought by accident costs more than
keeping a line.

**What these do not justify:** a graph, map, tags system or a second view. The
scale problem in Jellyfin (many items, several commissions) is tested first
against one document with a head and a search. It may fail. Tabs are the fix to
try then, and not before.

---

## 9. Where it lives

### 9.1 The options

| | **Baseline: notes + chats + hand-written briefs** | **Bridge: notes canonical, commissions in the app** | **Ideas destination in the app** | **Separate app** |
|---|---|---|---|---|
| Capture while working | Leaves the app | Leaves the app, or the §11.4 chord | Best | Global hotkey, separate process |
| Search, sync, phone | Already there | Already there | Must be built or given up | Must be built |
| Reviewed brief → place | By hand | **Built in** | Built in | Through serve |
| Durable record of the work | Whatever you keep up | **Built in** | Built in | Through serve |
| Live status beside the record | No | **Yes** | Yes | As a serve client |
| Result return | Copy by hand | **A review, then a block for your notes** | In place | In place |
| Harvest compared with memory | By eye | By eye | **In the product** | In the product |
| New concepts | None | One (commission) | Destination, two stores, item states | A second product and update path |
| If abandoned | Nothing to remove | One record type | A section to remove, and data to migrate | A product to retire |

**A separate app** speaks serve or has its own store; those are different
proposals, and only the second is independent. Neither has a case today. **A
second window** is a layout question for later, once there is something worth a
second window.

**The bridge's honest weakness.** It leaves as-of comparison and item state to
the owner's discipline in a notes app that knows nothing about them. Disputes,
supersession and reopened decisions become typed conventions. If the rehearsal
shows reconciliation is real and the owner keeps failing it by hand, that is the
case for the destination, and a better one than v1 made.

**The bridge's strength.** It builds only what a notes app cannot do, and it
stays useful whichever home wins: a later destination reads the same
commissions. It has a project to scope by, and it never asks the owner to move
their notes.

### 9.2 What "plain files" now promises

v1 promised plain Markdown that "survives the feature being removed", while
lifecycle, next, links, provenance and dismissals all needed somewhere to live.
Narrowed:

- **Your prose is portable. Product context is not promised to be.** A moved
  file keeps its words, and may lose its link to commissions and marks.
- **Under the bridge, the question mostly disappears.** Your notes are not ours
  and are never written by the app. Commissions are ours; each can be exported
  as text.
- **If a destination is built,** the supported behaviour is one host, one
  engine, and external edits to prose allowed: re-read, never merged. An item
  whose anchor an edit destroyed becomes plain prose and says so. Two editors at
  once is last-write-wins with a warning. Git in the folder is the owner's
  history and resolves no live conflict. Multiple hosts are not supported.

---

## 10. Workflows

The sources in §10.1 are real, from the short pass on 2026-11-06. **All dates
after capture, all decisions, every spike result, and every claim in §§10.2–10.4
are invented** to exercise the model. Nothing here is a recommendation about
Jellyfin.

### 10.1 Jellyfin: from sentence to first commission

**Capture.** Mid-task, the owner writes the sentence down wherever capture
currently lives. Focus returns to the terminal. Nothing else happens.

**Develop.** That evening it becomes an idea with the sentence quoted. It
already contains two things and an assumption: *clone Jellyfin*, *unify two
servers*, and the assumption that the first is needed for the second.

**Clarify.** In a chat, the owner keeps four questions:

- Which clients must work: stock apps, or is a custom client acceptable?
- What does "inform the client" mean on an app we do not control?
- Whose credentials end up where, and what has the friend agreed to?
- How do the two machines reach each other?

**Research.** A question, "what exists already", is commissioned or asked in a
chat. Harvested, each with its basis:

- *summary:* a Jellyfin developer said on the forum in October 2025 that they
  did not believe the project planned federation. A year old, so it stays a
  question to re-check.
- *read:* **Jellyswarrm** is a reverse proxy that appears to clients as a normal
  Jellyfin server, merges libraries and plays directly from the owning server.
  README says early development; about 960 stars, pushed this week, GPL-2.0.
- *read (description only):* **JellyfinFederationPlugin** takes the
  server-to-server route, and is smaller.
- *summary:* on a public thread the Jellyswarrm developer said that, because
  Jellyfin is unmodified, the password is sent to each server.

And the most valuable item, a new **question**: *the README does not say what
clients see when one upstream goes offline.* That is the owner's actual
requirement. Issue #76 is the nearest evidence (*read*): a top-priority server
going offline once made the whole proxy unavailable, and the maintainer's stated
fix was a background check that would "filter out offline servers". Filtering is
the opposite of showing content as unavailable, though that comment concerned
public endpoints and library behaviour was not checked.

**Options.** This is where the multi-repo question stops being abstract:

| Option | Repos | Unavailable state | Cost |
|---|---|---|---|
| Proxy in front, as is or contributing upstream | 1 (Jellyswarrm) | Limited to what a proxy can express to stock clients | Lowest |
| Server plugin on both machines | 1 (a plugin), both servers changed | Depends on what a plugin can surface | Medium; friend must install it |
| Fork server and clients | 2+ (`jellyfin`, `jellyfin-web`, perhaps apps) | First-class | Permanent merge burden |
| Proxy + patched web client | 2 (Jellyswarrm, `jellyfin-web`) | First-class on web, degraded on stock apps | Medium |

**Decision, by the owner:**

> **Nov 8. Spike the proxy before considering any fork.** It is the only option
> that leaves both servers untouched. Rejected for now: forking the server.
> **Reopen if** stock clients cannot be told anything useful about an offline
> upstream.

**Commission C1.** The owner clones Jellyswarrm (an explicit act) and reviews a
brief: run two local Jellyfins behind the proxy, stop one, record what each
client shows, end with the result shape. It starts as an ordinary
`create_worktree` with a brief, so harness, guidance, opener and limits work as
for any lane. **Starting fresh, not continuing the research chat,** survives
from v1: a transcript carries every wrong turn into working context, and the
brief is the repo's convention for handover. The record copies the brief and
notes *as of Nov 8*.

The next three are the cases v1 avoided.

### 10.2 Two sources disagree

Two pieces of work are open. A web-only research run read Jellyswarrm's docs and
proposes: *"The proxy can mark an offline server's items unavailable."* It is
accepted Nov 7, basis *read*. Run B, an ordinary commission with a checkout
because trying it needs one, drew on the same snapshot, tried it on Nov 9 and
proposes: *"Items from a stopped server vanish from the library."* Basis *ran*.

- **Harvesting B reviews against current memory,** so the kept claim is beside
  it, with its basis visible. B's proposal contradicts a named item.
- **The owner has three moves,** and none is automatic: supersede the first (B
  ran it; the first only read). Keep both as **disputed** with a new question
  (perhaps A meant a different client). Or skip B.
- **They choose disputed**, because B tested one client. The question "does this
  differ by client?" is new.
- **What enters memory:** both claims marked disputed, each with its basis, and
  the question. **What stays historical:** both runs' conversations, and the
  Harvest log's line for each.
- **Nothing was asked to agree.** The model flagged no conflict in the
  background. The review surfaced it because the proposal named the item it
  opposed.

Basis is what makes the choice intelligible six months later, with neither
transcript available.

### 10.3 Direction changes while work runs

C1 is running, drawn as of Nov 8. On Nov 9 the friend says they will not change
anything on their side and the family only uses the web client. The owner
records:

> **Nov 9. Web client only, for now.** Stock apps are out of scope. Supersedes
> nothing yet.

- **C1 does not hear about this.** The brief was a snapshot. There is no
  synchronisation to break.
- **The record shows the gap:** C1 now reads "drawn before: Web client only". It
  is a fact, not a nag.
- **The owner has three options.** Let it finish, since the spike's finding
  still matters. Tell the agent, in its own chat or terminal, as they would any
  lane. Or stop it.
- **A changed brief is a new commission.** A re-brief already refuses a live
  session, because the opener reaches only a session the launch creates. If the
  place is re-briefed outside this flow, C1's live line says "re-briefed since"
  and C1's own brief stays what it was.
- **When the result arrives,** Harvest shows "this run did not know: Web client
  only". Its findings about stock apps are kept as claims with their true basis
  and marked out of current scope; they are not wrong.

### 10.4 A finished result invalidates a decision

C1 reports, invented for this example: stock clients show library rows empty,
and items fail with a generic error. The branch is never merged, and was never
meant to be.

- **Outcome:** *Answered: no*, with git saying "unmerged, branch still there".
  Neither contradicts the other.
- **The result names what it trips:** the Nov 8 decision's "Reopen if stock
  clients cannot be told anything useful". The owner marks the decision
  **reopened**. Its text stays exactly as written.
- **A second commission drawn from that decision** is now flagged, the same way
  as in §10.3.
- **The owner writes a new decision, superseding the first:**

> **Nov 10. Add an availability layer to the proxy and patch `jellyfin-web` for
> the badge.** Stock apps get degraded behaviour, accepted for now.

- **What enters memory:** the answered question, the claim with basis *ran*, the
  reopened decision, the new one, an updated Now. **What stays historical:**
  C1's record, brief and result, in full, whatever happens to the branch. The
  original words at the top still say "Clone Jellyfin server", and the page
  shows why that is not what happened.

**And then merged is not answered.** Two commissions follow, on Jellyswarrm and
`jellyfin-web`, sharing the contract in both reviewed briefs. Both merge. Each
Harvest records **Partly**: each did its part, and no one has stood one server
down with the web client in front of it. The idea stays open, with the question.
A third small commission tries it end to end. Only then does the owner close the
idea, as done, months later, or park it when the friend loses interest. Either
way, the reasoning survives.

Had research found a finished solution, this idea would have closed on one
decision, *"use it"*, and no place at all. That may be the most valuable path in
the model, and a board with a Build column would hide it.

### 10.5 A random thought

> nav dots could say how long a lane has waited

Written down. No fields. Then one of these, all fine: **nothing, ever**;
**dropped** (still searchable); **developed**, if it turns out to be worth a
page; or **it belongs in a repo**, added to that ROADMAP by a lane as today. Its
untested cost was never capture; it is finding it again in five months, which is
task T1.

---

## 11. What runs first

### 11.1 The rehearsal: no code, two weeks

**On 3–5 real ideas,** including Jellyfin, a handful of incidental thoughts, and
at least one idea expected to end without execution.

**In the owner's existing notes app,** with one running file for thoughts and
one document per developed idea:

```
 # Title                          open · next (date): …
 > original words
 Now         a few lines
 Items       claim / question / option, with  basis · what · pointer · date
 Decisions   date, what, why, rejected, reopen if
 Commissions  C1: date · project · branch · session id · should tell you ·
             as-of · brief (pasted in full) · result (pasted in full) · outcome
 Harvest log  source · reviewed through · kept / edited / skipped
```

**Hand-off is a normal lane,** with a reviewed brief ending in the result shape.
Research goes through R0 or R1, and **this does not test R2**.

**Harvests are recorded as they happen,** including the ones skipped because
they were not worth doing. That number is itself a finding.

### 11.2 The tasks

After several days away from each idea, using **only the document** at first:

| | Task | Pass |
|---|---|---|
| T1 | Recover an old thought from remembered words | Found, by normal notes search, in about a minute |
| T2 | State the current question, direction and next action | Correct when checked afterwards against the transcripts |
| T3 | Explain a decision and what evidence it rested on, including what was only a summary | Correct on the same check |
| T4 | Review a brief drawn from the document | Few additions needed from memory or chat |
| T5 | Incorporate a result that contradicts a kept assumption | Done without rebuilding the document. If none arises naturally, commission a check aimed at the weakest kept assumption |
| T6 | Close one idea with no execution | The reason is still legible afterwards |

**Compare with baseline:** for one idea handled the old way, how long did it
take to reconstruct where it stands, and what was lost?

**Suggested thresholds, for the owner to change before starting:** T2 and T3
pass on at least 3 ideas. At least one commission round-trips with its result
reviewed. The owner says they would keep doing this, and can name which parts
they resent.

### 11.3 What each failure means

This is what v1's stop rule could not do. Few hand-offs are not a failure if
ideas ended deliberately; low revisits could have meant anything.

| Observed | Means | Next |
|---|---|---|
| T2/T3 fail; transcripts reopened for missing state | The memory's shape is wrong, or Harvest loses what matters | Change the shape by hand and rerun. No code |
| Documents never reopened, though search works | The memory is not wanted | **Stop.** Maybe keep capture |
| Reopened, but things cannot be found | Retrieval | Search or head changes, not a board |
| Harvests skipped or bypassed | Review costs more than it returns | Loosen it (§12), or bridge without structured review |
| Cost is mainly *copying* | The judgment is worth it and the mechanics are not | **Bridge** |
| Cost is mainly *reconciling items* by hand, and it keeps slipping | The notes app is what failed | **Bridge, then home** |
| Lost links to work, briefs gone, wrong session reopened | The original gap is the handover | **Bridge** |
| The main complaint is having no chat beside the memory | The missing part is the runtime | §7.3's spike, not storage |
| Capture was the bottleneck, and all else was fine | Capture | §11.4 only |
| Bypassed for plain notes, happily | The current workflow is enough | **Stop.** That is a good outcome |

**Limits of this test.** One motivated owner, who asked for it, on a small
sample, is not a population. Manual copying overstates some costs, and
enthusiasm understates others. The rehearsal cannot test in-app capture, live
status or R2.

### 11.4 Capture is its own small test

The owner did ask for it. If wanted at any point: one in-app chord, one line,
Enter, focus returns, appending to one file the owner chooses. It can be tried
independently, costs little, and can be removed without trace. **It tests only
whether writing things down gets easier.** Success says nothing about §1's bet,
and failure says nothing against it. The hard part to test is that the file does
not become a write-only pile, and that is T1.

### 11.5 The first coded slice, if the table says bridge

- **Start from text.** Paste or write a brief, choose project, branch and agent,
  add "should tell you", review, start. It is an ordinary place.
- **The commission record** of §5, including all live states. A short list of
  commissions, by project, and "from commission…" on the place.
- **Bring back.** Take the result the lane wrote, or a paste. Record the
  outcome. Copy out a block for the owner's notes.
- **The one fact of §4:** results not yet reviewed.

Not in it: capture, thoughts, idea pages, item state, as-of comparison beyond a
label, search, MCP, model calls beyond what a lane does. A drafted brief, in
this slice, comes from a chat you already have.

Its own test, stated now: after a month, are commissions opened again after
their lanes end, and do results come back through it instead of around it? If
not, remove it, and the place's own brief and PR are no worse off than today.

**If the table says home, later:** one capture door, thoughts with Develop and
Drop, search, one editable idea with Now and item states, Harvest against
current memory, and the existing commissions. This is deliberately narrower than
v1's slice 1: no `worktrees thought`, MCP, sort-through, *Add to idea*, global
decisions view, similarity hints or attention grouping.

**Not planned at any stage:** scheduled or background research, priority
scoring, due dates, hierarchy between ideas, writing idea content into repos
automatically, agents reading ideas ambiently.

---

## 12. Where this revision disagrees with the reviews

Both reviews were mostly right, and §§4–8 adopt most of what they raised. These
are the choices made against them or beyond them.

**A cheaper first slice than Kimi's, and with a return path.** Kimi proposed one
capture door, scratchpad, editable idea, prefilled hand-off and a static work
link. That builds first exactly what a notes app does best, and it stops before
the return path, which is the part both reviews rank highest. The bridge leads
unless the rehearsal implicates the notes app.

**Capture stays in the product conversation, against Codex's order of
priority.** Codex is right that capture does not test the bet. It does not
follow that it waits behind all of it. The owner asked for it, it is cheap, and
§11.4 keeps it from contaminating anything.

**No owner-set "waiting for" field (Kimi's alternative).** It would be one more
thing going stale. A fact, one review state and a dated *next* cover it. v1's
*next* survives, with a date added.

**The "new/unreviewed activity" Kimi suggested is narrowed to results.** General
activity would mean every agent message owes a review. That is the nag v1 was
rightly trying to avoid.

**R1 stands in, where Codex warned against a hidden-project engine.** Agreed on
not building one. But a plain, visible small project is no engine, and its cost
is unmeasured. v1's objection was stronger than its evidence. Kimi's caution
holds: it does not test the eventual runtime.

**One document stays the default, despite the scale worry.** Kimi and Codex both
doubt it at Jellyfin scale, and they may be right. Adding tabs now means
building for a failure not yet seen. It is what T2 and T4 can show.

**A choice among "remembering, exploring, coordinating" is asked for, with a
caveat.** Codex's framing is adopted in C1. But the proposal's position is that
the bet is the connection between them, and asking for one should not suggest
they are independent products.

**Harvest stays friction.** Both reviews question its bookkeeping, and §6.4 adds
rules. Neither asks to remove the person, and v2 does not offer that. The escape
valve is loosening by kind: sources first, as C4 offers. It is not an agent
editing memory.

**Evidence stays one line.** Codex asks what supports a claim and what was
checked; the basis word and a few words answer that. Going further becomes the
citation system both reviews warn against. Some claims will be poorly supported,
and they will say so.

**No automatic contradiction detection, though it would look helpful.** Neither
review proposed this, but the workflows invite it. It is a background model
reading your memory, against invariant 6. A proposal names conflicts on a click,
and that is the limit.

---

## 13. Choices for the owner

Recommended answers first. Each changes what happens next.

**C1. What do you most want from this?** (a) Being able to pick work back up
with its reasoning intact. *The proposal's reading of the bet.* (b) Writing
thoughts down without leaving flow. Then §11.4 is most of the job, and little
else here is needed. (c) Better exploration with agents. Then the chat beside
the memory matters most, and §7 moves up. (d) Coordinating work across repos.
Then commissions matter most, and thoughts barely. *This shapes the rehearsal's
emphasis, so answer before starting.*

**C2. Run the rehearsal first?** (a) Yes, two weeks, §11 as written.
*Recommended.* (b) Skip to the bridge, if the record and return are already what
you miss. A reasonable bet, since it is small and removable. (c) Skip to a
destination. *Not recommended:* it commits to a home before there is evidence
the notes app failed.

**C3. Can your notes app stay canonical?** (a) Yes, unless the rehearsal says
otherwise. *Recommended.* (b) No, you want this inside worktrees regardless, for
reasons of your own. That is a legitimate preference over evidence. Say so, and
the home moves up, with the §9.2 limits. Which notes app you use, and how well
it searches, also affects T1.

**C4. How strict is Harvest?** (a) The person accepts everything. *Recommended.*
(b) Sources may append directly with their basis, and the rest is proposed. The
fallback if the rehearsal shows the review is skipped. (c) An agent edits
freely, with history to undo. It is likely to feel good for a week.

**C5. Is giving up chat-beside-the-document acceptable for the rehearsal?** (a)
Yes, and report how much it hurt. *Recommended.* (b) No, that was the point.
Then the rehearsal should be replaced by §7.3's spike, and the rest of this
proposal waits for it.

**C6. Lifecycle.** Open, parked, closed, with person-set item state and no
stages, is unchanged. Say so now if you want "everything in research" at a
glance, because that reopens the case for a board.

## 14. Open questions

**Q1. Is one workspace enough?** This now affects privacy as well as naming
(§8.1). All registered projects share one nav. If thinking about business and
personal work should not be near each other, a tag will not do, and separate
notes folders may. In the bridge, the owner's notes already answer this.

**Q2. Where does a commission record live?** Beside a project's message log, it
fits device scoping but dies with the repo. On the host, it survives the repo
being removed and needs its project filter applied to every read. Leaning
host-level, since outliving the repo is the point.

**Q3. Do ideas follow you between hosts?** Under the bridge, your notes app
already answers this for prose, and commissions stay with the host that ran
them. Under a home, it is unsupported.

**Q4. What should a lane be required to report?** The result shape is a
convention in a brief. Whether an agent reliably writes it, and whether a
place-local file or a message is the better carrier, is something only the
rehearsal can show. Idle is not a signal of it.

**Q5. Is a "reference clone" a distinct thing?** Reading Jellyfin's code means
cloning it. Today that adds a project to your nav, and you remove it afterwards.
It may be fine.

**Q6. Does capture need to work with the app closed?** A system-wide hotkey is a
different feature from an in-app chord. Your notes app may already have one, and
that may end the matter.

**Q7. Is "commission" the right word?** It is precise, and it collides with
nothing in this repo. It also sounds formal for a small spike; "run" and "job"
are the fallbacks, and both are vaguer. "Thought" stays a candidate word too
("scratch" is the fallback).

---

## Sources for §10.1

Checked 2026-11-06, read for this example only, and not re-checked for v2.

- [Jellyswarrm](https://github.com/LLukas22/Jellyswarrm): README, repository
  metadata and `docs/health-monitoring.md` (its endpoints do not depend on the
  upstream servers), read directly.
- [Jellyswarrm issue #76](https://github.com/LLukas22/Jellyswarrm/issues/76): an
  offline server making the proxy unavailable, closed December 2025, read
  directly. The fix itself was not read or tested.
- [JellyfinFederationPlugin](https://github.com/Angablade/JellyfinFederationPlugin):
  repository description and metadata only.
- [`jellyfin/jellyfin`](https://github.com/jellyfin/jellyfin) and
  [`jellyfin/jellyfin-web`](https://github.com/jellyfin/jellyfin-web): separate
  repositories for the server (C#) and the web client (JavaScript).
- [Jellyfin forum thread](https://forum.jellyfin.org/printthread.php?tid=14081):
  the October 2025 developer comment, seen through a search summary, not read in
  full.
- [Lemmy discussion of
  Jellyswarrm](https://lemmy.thefloatinglab.world/post/94437): the password
  comment, also seen through a search summary. Verify both before relying on
  them.
