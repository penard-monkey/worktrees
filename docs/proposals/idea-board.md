---
title: "Proposal — ideas: thinking before a place exists"
---

# Proposal — ideas: thinking before a place exists

**Status:** product exploration, 2026-11-06. Nothing is built and nothing here
permits building it. "Idea board" is the working name; §2.4 argues against
"board". Implementation detail is deliberately shallow. The choices that need
the owner are in §10.

**The spark.** The owner wants to capture something like this while working:

> Clone Jellyfin server and get it working as a project. Make two Jellyfin
> servers on remote systems behave as one: my server and a friend's appear
> unified; if one disconnects, inform the client and show its content as
> unavailable.

Today that sentence has nowhere to go. It is not a branch, it is not yet a
project, and it may never become either. Worktrees starts at "I know which repo
and which branch". The thinking that comes before that happens in an unrelated
notes app, in a chat that gets lost, or in a lane that was created too early.

**Evidence base.** Terminology was checked against this repo at `0e86349`
(v0.48.0): `AGENTS.md`, the cross-project, nested-lanes, owned-planning and
chat-view proposals, and the ROADMAP's record of why automations were removed.
Jellyfin facts come from one short research pass on 2026-11-06 and are labelled
where they appear. No app was run. No Jellyfin code was read.

## The recommendation in one paragraph

Add **one workspace-level surface, Ideas, built on two objects**: a *thought*
(one line, no obligations) and an *idea* (one living document). A thought
becomes an idea only when the person says so. Everything richer is something an
idea *accumulates*, not a stage it must pass through: questions, sources,
options, decisions, an approach, and links to work. **Chats are threads attached
to an idea. A thread's transcript is evidence and the document is the memory;
what a thread found reaches the document only when the person accepts it.**
Handing an idea to execution uses the machinery that exists: the person reviews
a brief, picks a project and branch, and a normal place starts. It lives in the
app as a top-level destination beside Home and Places, stored as plain files the
user owns, with a small MCP surface so agents can file thoughts and read ideas.
A separate window is a cheap later addition; a separate app is not worth its
second store today. **The first slice is capture, the scratchpad, the idea page
and hand-off. Threads are the second slice and the real bet.**

---

## 0. The answer in one screen

| Question | Short answer | Section |
|---|---|---|
| Core objects | **Thought** and **idea**. Thread, decision and work link belong to an idea and never stand alone. | §2 |
| Lifecycle | No pipeline. An idea is *open*, *parked* or *closed*. "Researched" or "planned" is read off what the document holds. | §2.2 |
| Workspace, project, place | Thinking is workspace level. A project is a lens on it. A place is where execution happens, and it knows only its brief. | §3 |
| Fan-out | An idea links to any number of projects and branches. It contains none of them. | §3.2 |
| Lightest capture | A chord, one line, Enter, focus returns to where you were. No title, project or tag. Also `worktrees` CLI and MCP. | §4.1 |
| Deciding what next | The Ideas list groups by **what each idea is waiting for**, not by stage. Not Kanban. | §4.3 |
| Chat | Purpose-named threads on an idea, one selector that also lists linked work sessions. Promotion starts a new session from a reviewed brief. | §5 |
| Suggestions | Local hints may appear unasked. Anything using a model runs on a click. Every state change is the person's. | §6 |
| Where it lives | In the app, top level. MCP for agents and serve for UIs, not MCP as a UI's backend. | §7 |
| First slice | Thoughts, idea page, hand-off, work links. With a stop rule. | §9 |

---

## 1. What this must respect

Four things already true of worktrees shape the model.

**A place is a durable location; a branch is work flowing through it.** So
thinking must not need a place, and a link from an idea to "work" should name the
*branch*, because a place outlives what it was doing.

**The brief is the handover, and `.planning/` is a place's working memory.** So
an idea should not grow a second competing plan. It ends where a brief begins.

**The owner has rejected a tracker once.** PR #296 turned ROADMAP.md into 68
issues and was closed unmerged; the roadmap stays a file. So Ideas must not
become a project's backlog, and project stragglers keep going to its ROADMAP.

**Automations were built, then removed, and the reasons are written down**
(ROADMAP, removed 2026-10-10). They read like a checklist for this proposal:

| What went wrong with automations | What it means here |
|---|---|
| The headline promise (a schedule) never ran | Show nothing that does not work yet. No "research runs nightly". |
| A project-wide thing in a per-place surface | Ideas must not be a dock tab. Dock contents follow the selected place. |
| Too much vocabulary for one verb, before it had proved useful | Five nouns, four of them visible only once used (§2.1). |
| Its output overlapped surfaces that already answer the question | Do not restate health or lane status. Link to them. |
| "Start from one concrete sweep someone keeps asking for" | Jellyfin is the concrete case. §8 must work or the model is wrong. |

---

## 2. The model

### 2.1 Vocabulary

| Word | What it is | When you meet it |
|---|---|---|
| **Thought** | One timestamped entry in the scratchpad. No title, no fields. | Always |
| **Idea** | One living document with a title. Survives everything else being lost. | When you press Develop |
| **Thread** | A conversation with an agent, for one purpose, attached to an idea. | When you ask for help |
| **Decision** | A dated entry inside an idea: what, why, what was rejected, what would reopen it. | When you decide |
| **Work link** | From an idea to a project and branch (and its PR when known). | When you hand off |

Words deliberately avoided, because the app already uses them for something
else:

- **note**: a place already has one (`set_note`, the declared store). Hence
  "thought".
- **hub**: means both a sync hub and a nested-lanes parent.
- **plan**: a place's plan lives in `.planning/`. An idea has an *approach*,
  which feeds a brief. The real plan stays in the place.
- **workspace** is kept with its existing meaning: the registered projects on
  this host, which is what `list_workspace` already answers.

### 2.2 Lifecycle

The brief's chain (fleeting note → idea → researched direction → plan → decision
→ execution) is a true story about *some* ideas. As a state machine it would be
wrong, because research can follow a decision, a plan can reveal that research
was thin, and most thoughts should stop at step one. Anything with six stages
also makes someone keep the field current.

```
  thought ──Develop──▶ idea
     │                  │  open ◀──▶ parked
     │                  │    └──────▶ closed (done, or dropped with a reason)
     ▼                  │
  (usually nothing)     │  accumulates, in any order, none required:
                        │    questions · sources · options · decisions · approach
                        ▼
                     work links ──▶ project / branch / PR  (status read live)
```

**Recommendation: three declared states, the rest derived.** That is the split
worktrees already uses: declared lifecycle plus derived live state.

- *Declared:* open, parked, closed.
- *Derived:* "has sources", "has a decision", "thread running", "work merged".

**Tradeoff.** You cannot filter on "all ideas in research" as a hand-kept status.
You can ask "ideas with a running thread" or "ideas with no decision", which is
what that filter was for.

**Alternative: explicit stages (Inbox, Research, Plan, Decide, Build).** Easier
to explain and it would suit a board. Rejected because stage upkeep is unpaid
work, and a stale stage lies. If the owner wants it, §10 C2 is where to say so.

### 2.3 Rules that keep it light

1. **Capture asks nothing.** A thought has text and a time, nothing else.
2. **A thought can stay a thought forever.** There is no overdue, no count badge
   and no "process your inbox". `projectTodos.ts` already holds this principle:
   a count no button can lower is a nag.
3. **The original words are kept.** An idea keeps the thought that started it,
   verbatim, even once the direction has moved far from it.
4. **An idea with no sections is a valid idea.** Sections appear when first used.
   A two-line idea looks like two lines, not a form with six empty headings.
5. **A project is not required, ever.** Some ideas end as a decision not to do
   anything, and that is a good outcome worth keeping.

### 2.4 Why not "board"

A board is a promise of columns, and columns are stages (§2.2). It also suits
many small similar cards; ideas here vary from two lines to a long research
document. Call the destination **Ideas**. Spatial or card views stay possible
later as another way to look at the same files, if real use asks for one (§9).

---

## 3. Levels

### 3.1 What belongs where

| Level | Holds | Does not hold |
|---|---|---|
| **Workspace** (this host's registered projects) | Scratchpad, ideas, threads, decision log | Anything tied to a checkout's files |
| **Project** | A *lens*: ideas that link to it, seen from the project. Its own `ROADMAP`, proposals and ADRs stay its own. | Idea content. Nothing is written into a repo from the Ideas surface. |
| **Place** | Brief, plan, agent, code | Any knowledge of the idea beyond what the brief says |

**A place needs no awareness of ideas to work.** A lane launched from an idea is
an ordinary lane, and its brief is still the whole handover. Ideas read place
status; places do not depend on ideas.

**Crossing into a repo is an explicit act done in a place.** If a decision
deserves to be an ADR, or an idea becomes a repo's proposal, that is work a lane
does and commits. Idea text is the user's and may be private (a friend's server,
for one), so it should not leak into a shared repository by being near it.

### 3.2 Fan-out without a container

One idea can link to several projects and branches, and one project to several
ideas. The idea is **not** a parent: closing it closes nothing, removing a place
breaks nothing, and a project never "belongs to" an idea. No epics, no hierarchy,
no rollup percentage.

```
  Idea: Two Jellyfins as one
    ├─ Jellyswarrm      spike/upstream-offline     PR merged
    └─ jellyfin-web     feat/unavailable-badge     lane idle, waiting on you
```

**A link names project + branch, not the place.** The place is looked up live.
If the branch's PR merged and the place moved on to other work, the link still
tells the truth about what this idea produced. Projects are named the way
cross-project reach already names them (the registry entry's name).

If those lanes need to talk to each other, that is the existing cross-project
reach, with its existing opt-ins. The idea grants nothing. A coordinating agent
would be a lane like any other.

### 3.3 Is one workspace enough?

**Recommendation: yes, for now.** A named-workspaces feature would be a large
concept to add for a problem nobody has hit yet. If ideas for separate parts of
life need separating, free-form tags on an idea will show whether that is real.
See open question Q1, because this one is the owner's experience to report.

---

## 4. Flows

These are for reacting to. Layouts are sketches, not specs.

### 4.1 Capture

From anywhere in the app, including with focus in a terminal: a chord opens one
line. Enter saves and returns focus exactly where it was. Escape cancels.

```
 ┌──────────────────────────────────────────────────────────────┐
 │ two jellyfins as one? mine + a friend's, offline = greyed out│
 └──────────────────────────────────────────────────────────────┘
```

The same box sits at the top of Home. Two more doors, with the same result:

- `worktrees thought "…"` from any shell;
- an MCP tool, so an agent can file what it noticed mid-task. These are marked
  as coming from that place, and they are only ever thoughts (§6).

A phone is the most natural capture device of all, and it comes later (§9).

### 4.2 The scratchpad

Reverse chronological. Nothing to clear, and nothing counts against you.

```
 Scratchpad                                         [ Sort through ]
 ─────────────────────────────────────────────────────────────────
 today 14:02   two jellyfins as one? mine + a friend's, offline…
                                       related: "Home media setup" ▸
 today 11:40   nav dots could say how long a lane has waited
 Tue           from worktrees:feat-push   retry policy differs on iOS?
 Oct 28        look at what Zed does with agent threads
```

Hovering or selecting a row offers **Develop · Add to idea… · Drop**. *Sort
through* is optional: one thought at a time, those three actions plus *Keep*, and
it ends when you stop. It is a mode you enter, never a task you owe.

### 4.3 Ideas: deciding what to do next

This list is the answer to "what should I do next". It groups by **what the idea
is waiting for**, all derived, with no column to maintain:

```
 Ideas
 ─────────────────────────────────────────────────────────────────
 NEEDS YOU
   Two Jellyfins as one     research thread finished · 1 open decision
   Sync over Tailscale      spike PR merged · was the answer yes?

 AGENTS WORKING
   Phone capture            thread: what does iOS share sheet allow…

 RESTING                                             newest touched first
   Home media setup         next: ask which clients the family uses
   Better diff navigation   3 weeks ago

 Parked (4)  ·  Closed (11)  ·  Decisions
```

- **Needs you** has one strict entry rule: there is something only you can act
  on, and acting clears it. It is the only count this surface could ever show.
- **Next** is one optional free-text line on an idea. It is what the row shows
  while resting. It costs nothing to skip and is the cheapest way to leave
  yourself a way back in.
- **No priority field and no score.** Choosing between ideas is the person's
  judgment, and a computed rank would be a confident lie.
- **Decisions** is a derived view across all ideas, newest first. There is no
  separate store. It answers "why did I decide against that?".

Each thinking mode from the brief gets a view without gaining a noun:

| Mode | Where |
|---|---|
| Dump something | Capture box |
| What was I thinking about? | Scratchpad |
| What should I pick up? | Ideas list |
| Think hard about one thing | Idea page |
| What did I find out? | Sources section of the idea page |
| What did I decide, and why? | Decisions, per idea or across all |
| What came of it? | Work section of the idea page |

### 4.4 The idea page

One document that reads top to bottom, with threads and work beside it. **One
page rather than tabs**, because most ideas are short and tabs would give a
two-line idea a five-tab frame.

```
 ┌ Two Jellyfins as one ──────────── open ▾ ─┬ THREADS ───────────────┐
 │ next: stop one upstream, see what apps do │ ● What exists already  │
 │                                           │   finished · read it   │
 │ > Clone Jellyfin server and get it        │ ○ Clarify the ask      │
 │ > working as a project. Make two…         │   you, 2 days ago      │
 │   captured Nov 6                          │ + New thread           │
 │                                           ├ WORK ──────────────────┤
 │ Questions                                 │ Jellyswarrm            │
 │  ? What must a stock client show offline? │  spike/upstream-offline│
 │  ? Whose credentials sit on the proxy?    │  lane busy             │
 │                                           │ + Hand to a session    │
 │ Sources (3)                               │                        │
 │ Options                                   │                        │
 │ Decisions                                 │                        │
 │  Nov 8 Start from a proxy, not a fork     │                        │
 │ Approach                                  │                        │
 └───────────────────────────────────────────┴────────────────────────┘
```

Selecting a thread opens its chat in place of the document, or beside it on a
wide window. It is the same chat the agent cell already shows.

---

## 5. Chats, threads and execution

### 5.1 Several threads, each for one purpose

**Recommendation: many short threads per idea, named by purpose** ("What exists
already", "Clarify the ask", "Compare the topologies"). A single long chat per
idea mixes concerns and fills its context. Starting a thread asks for one line,
its purpose, which also opens the conversation. Templates for common purposes
can come later if the same few keep being typed.

### 5.2 One selector, two kinds of chat

The owner's framing was "a selectable chat that can also become a working
session". The selector on an idea lists **thinking threads and the sessions of
its linked work together**, so from the idea you can move between research and
the lane doing the work with one click. What differs is shown plainly:

| | Thread | Work session |
|---|---|---|
| Runs where | Against the idea, with no checkout | A place, on a branch |
| May change | Nothing directly; it proposes (§5.3) | Its tree, as any lane does |
| Ends | When it has answered its purpose | When the work is merged or closed |

### 5.3 Transcript is evidence, document is memory

A transcript is long, full of wrong turns, and owned by the harness (which may
prune or move it). It must not be what you rely on six months later.

**Harvest** is how a thread's findings reach the idea. On a click, the thread's
agent proposes specific additions, and each one is accepted, edited or skipped:

```
 Harvest from "What exists already"
   Source   Jellyswarrm: reverse proxy, looks like one Jellyfin   [Keep] [Skip]
   Source   Federation plugin: server-to-server, needs a plugin   [Keep] [Skip]
   Question README says nothing about an upstream going offline   [Keep] [Skip]
   Option   Proxy in front; no fork of the server                 [Edit] [Skip]
```

- Anything kept records which thread and date it came from, so you can go back
  to the conversation. The document still reads fine if that transcript is gone.
- **A decision is never harvested as a decision.** A thread can suggest one, and
  it lands as an option. Only the person writes "decided".

**Tradeoff.** Harvest is friction, on purpose. The alternative, an agent editing
the document directly, is smoother and leaves you owning a document you have not
read. If it turns out to be too much, loosen it for sources and keep it for
everything else.

### 5.4 Promotion: "Hand to a session"

```
 Hand to a session
   Project  [ Jellyswarrm            ▾ ]   or  Clone… / New project…
   Branch   [ spike/upstream-offline   ]
   Agent    [ Claude · Sonnet          ▾ ]
   Brief    ┌────────────────────────────────────────────────────┐
            │ Goal: with two local Jellyfins behind the proxy,   │
            │ stop one and record what each client shows.        │
            │ Context: …  Deliverable: …  Stop and report when…  │
            └────────────────────────────────────────────────────┘
                                         [ Cancel ]  [ Start place ]
```

The brief is drafted from the idea (on request) and is **always shown, editable,
before anything starts**. Starting is an ordinary `create_worktree` with a brief,
so harness, guidance, opener and limits work as they do for any lane. The idea
gains a work link. Nothing else changes.

**Recommendation: promotion starts a new session from a brief, and does not
continue the thread's session.** The transcript carries every wrong turn into
working context, while the brief is what the repo's conventions already treat as
the handover. It also keeps provenance honest, since the lane did what the brief
said. The brief can point back to the thread. The owner's wish is still met in
how it feels: the selector keeps both chats together, one click apart.

**Alternative: truly continue the same chat in the new place.** It feels magical
and keeps every nuance. It depends on each harness tolerating a change of working
directory and would carry a full context into a coding task. Worth a later
experiment, and it should not be the model's foundation.

---

## 6. What the app suggests, and what stays yours

| Action | Local hint, unasked | Agent, on a click | Person only |
|---|---|---|---|
| Related or duplicate ideas | Text match when capturing, one quiet link | Deeper "is this the same idea?" | Merging, attaching |
| Clarifying questions | | Yes; they land as questions you keep or skip | Answering |
| Research tasks | | Proposes a list | Starting each thread |
| Which projects this touches | Names a registered project if the text does | Proposes a mapping and repos to look at | Linking, cloning, creating |
| "This reads like two ideas" | | Proposes a split | Splitting |
| A draft of a decision | | As an option, with tradeoffs | Writing "decided" |
| A draft of a brief | | Yes | Editing and starting |
| Staleness | Facts only ("3 weeks ago") | | Parking, closing, dropping |

**The lines this draws:**

- **No model runs without a click.** That covers cost and also trust: nothing
  happens in the background, so nothing can be happening that you did not ask
  for. Local hints use no model at all.
- **No autonomous execution, full stop.** No scheduled research, no thread that
  starts itself, no place created because a suggestion was not dismissed. A
  running thread is one you started, and you can see it and stop it.
- **A dismissed suggestion stays dismissed.**
- **An agent's reach is narrow.** Over MCP an agent can add a thought and read an
  idea. It cannot promote, edit a decision, link work on its own say-so or start
  anything. Whether a *thread's* agent may append directly to its own idea's
  sources is the loosening mentioned in §5.3.

**Idea text is data, never instructions or argv.** This is ADR 0001's stance one
level up, and it matters more here than it first appears, because research
threads read the open web and agents can file thoughts:

- A web page can try to steer a research thread, and a repo can steer a lane into
  filing a thought. So harvested content and agent-filed thoughts stay marked with
  where they came from, and they reach any later agent as quoted material.
- A clone URL, a branch name or a project a suggestion names is shown in full in
  the form and takes effect only when the person submits it, through the same
  checks those paths already have.

---

## 7. Where it lives

| | **A. In the app, top level** | **B. A second window, same app** | **C. A separate app, over MCP** | **D. Existing notes app + MCP** |
|---|---|---|---|---|
| Capture while working | Best: same keyboard, same focus | Needs the same chord; window is an extra step | Global hotkey, separate process | Whatever it already does, outside the app |
| Seeing live lane status beside an idea | Native | Native | Only what a tool call returns | Only what a tool call returns |
| Reusing the chat, composer and prompt handling | Yes | Yes | Rebuilt, or settled for less | No |
| Thinking beside execution | Competes for the same panes | Best, especially on two screens | Good | Good |
| Useful with no repo in sight | Yes, but it is a dev tool's section | Same | Best: can stand alone as a product | Best |
| Concepts added | One destination | One destination, one window | A second product, store, update path and permission model | None in the app |
| Conceptual fit | "Before a place" belongs beside places | Same | Weakens the hand-off, which is the point | Loses it |

**The conceptual point about MCP.** MCP here is the *agents'* door: each session
launches its own server, scoped by where it runs. The door for a *UI* is serve:
authenticated, event-carrying, capability-checked, and already what the app, the
remote window and the phone all use. A separate UI that spoke MCP as its backend
would be using the wrong door, so **C really means "another serve client" or "a
second product with its own store"**. Those are different proposals, and only the
second is independent.

**Is D the honest competitor?** Yes. If capture and a markdown document were all
this was, a notes app would win, so the value has to come from what a notes app
cannot do: threads that hand to places and links that read live status. That is
why slice 2 is the real test (§9), and why the files stay plain markdown, so a
person can keep them in a notes vault too (§10 C3).

**Recommendation: A, built so B is cheap.** A top-level destination, not a dock
tab (automations' mistake). The app is already a client of serve, so a second
window is a later client of the same data, and the choice of window does not have
to be made now. Keep C open by keeping the format plain and the MCP tools small.

**Tradeoff.** The app's layout is place-centred, and a new top-level destination
is real attention for a section that might not earn it. The app once had nav
lenses (Recent, Attention) that no longer exist, so destinations here do get
removed. Hence the stop rule in §9.

---

## 8. Scenarios

### 8.1 Jellyfin, end to end

The sources below are real, from a short pass on 2026-11-06. **The dates after
capture, both decisions and the spike's result are invented** to show the flow.
All of it is here to test the model, and it is not a recommendation about
Jellyfin.

**Capture.** The owner is mid-task in a lane. Chord, types the sentence, Enter,
focus returns to the terminal. Elapsed time is what it takes to type. Nothing
else happens, except a quiet "related: Home media setup" if such an idea exists.

**Develop.** That evening, from the scratchpad: Develop. An idea page opens with
the sentence quoted, and a suggested title they can overwrite. The sentence
already contains two things and an assumption: *clone Jellyfin*, *unify two
servers*, and the assumption that the first is needed for the second.

**Clarify thread.** Purpose: "Clarify the ask". It produces questions, and the
owner keeps these:

- Which clients must work: stock apps, or is a custom client acceptable?
- What does "inform the client" mean on an app we do not control?
- Whose credentials end up where, and what has the friend agreed to?
- How do the two machines reach each other?

**Research thread.** Purpose: "What exists already". Harvested as sources:

- **Upstream:** a Jellyfin developer said on the forum in October 2025 that they
  did not believe the project planned federation. *(A year old, so it stays a
  question to re-check, which is why sources carry dates.)*
- **Jellyswarrm:** a reverse proxy that appears to clients as a normal Jellyfin
  server, merges libraries and plays directly from the owning server. Its README
  says early development; about 960 stars, pushed this week, GPL-2.0.
- **JellyfinFederationPlugin:** a server-side plugin taking the server-to-server
  route, and smaller.
- **A security note:** on a public thread the Jellyswarrm developer said that,
  because Jellyfin is unmodified, the password is sent to each server.

And, most valuably, a new **question**: *the README does not say what clients see
when one upstream goes offline.* That is the owner's actual requirement. Issue #76
is the nearest evidence: a top-priority server going offline once made the whole
proxy unavailable, and the maintainer's stated fix was a background reachability
check that would "filter out offline servers". Filtering is the opposite of showing
content as unavailable, though that comment concerned public endpoints, and library
behaviour was not checked. Whether a stock client can show "a peer is down" at all
may turn out to be the whole project.

**Options, with their repo topology.** This is where the multi-repo question
stops being abstract:

| Option | Repos | Unavailable state | Cost |
|---|---|---|---|
| Proxy in front, as is or contributing upstream | 1 (Jellyswarrm) | Limited to what a proxy can express to stock clients | Lowest |
| Server plugin on both machines | 1 (a plugin), both servers changed | Depends on what a plugin can surface | Medium; friend must install it |
| Fork server and clients | 2+ (`jellyfin`, `jellyfin-web`, perhaps apps) | First-class | Permanent merge burden |
| Proxy + patched web client | 2 (Jellyswarrm, `jellyfin-web`) | First-class on web, degraded on stock apps | Medium |

**A decision, recorded:**

> **Nov 8. Spike the proxy before considering any fork.** Why: it is the only
> option that leaves both servers untouched, and the friend need not do anything.
> Rejected for now: forking the server (merge burden, and possibly unnecessary).
> **Reopen if** stock clients cannot be told anything useful about an offline
> upstream.

The original words "Clone Jellyfin server" are still at the top of the page, and
the page shows why that is not what happened. That is the provenance rule earning
its keep.

**Hand off.** "Hand to a session". The owner chooses *Clone…* for Jellyswarrm, an
explicit, visible act, and reviews a brief: run two local Jellyfins in Docker
behind the proxy, stop one, and record what each client shows. Stop and report. A
normal place starts. The idea shows a work link, and the lane's chat is one click
away in the selector.

**Back again.** The idea moves to *Needs you* when the lane finishes. The result:
suppose the web client shows items that then fail with a generic error, and the
library rows go empty. The owner records:

> **Nov 10. Add an availability layer to the proxy, and patch `jellyfin-web` for
> the badge. Stock apps get degraded behaviour, accepted for now.**

**Fan-out.** Hand to a session twice more: one branch on Jellyswarrm, one on a new
`jellyfin-web` project. The idea links to both. Neither lane knows the other
exists unless the owner opts them into cross-project reach. When the proxy branch
merges, the idea says so, live. The owner closes the idea as done months later,
or parks it when the friend loses interest. Both are fine, and either way the
reasoning survives.

**What this tested.** Five nouns held up. One idea covered two repos without being
a parent of either. No step was forced: had research found a finished solution,
the idea would have closed with one decision, *"use it"*, and no place at all.
That may be the single most valuable path in this model, and a board with a Build
column would hide it.

### 8.2 A random thought

> nav dots could say how long a lane has waited

Chord, typed, Enter. No fields. Then one of these, all fine:

- **Nothing, ever.** It scrolls down. It cost four seconds and owes nobody anything.
- **Drop.** One action during a sort-through, gone.
- **Add to idea.** It becomes one line on an existing idea, keeping its date.
- **Develop.** Only if it turns out to be worth a page.
- **It belongs in a repo.** A plain worktrees-roadmap item, added the way those are
  added today, by a lane. Ideas does not become that repo's tracker.

**What this tested.** Zero objects were created beyond one line, and there were no
decisions at capture. If this path ever needs a second keystroke, the model has
failed its lightness test.

---

## 9. Slices

**Slice 0: try it by hand first. No code, a couple of weeks.** Make an ordinary
`ideas` git repository, register it as a project, and keep one markdown file per
idea. Develop ideas in lanes there, and hand off by writing briefs. This costs
nothing and tests the riskiest belief: *that the owner will go back to what they
wrote*. It will also show how the idea document actually wants to be shaped. Its
known flaw is that capture stays clumsy, so it says little about thoughts.

**Slice 1: capture and keep.**

- The chord, the Home box, `worktrees thought`, the MCP tool.
- Scratchpad with Develop, Add to idea, Drop.
- Idea page: an editable document with quoted origin, *next*, open/parked/closed.
- Ideas list with Resting and Parked; *Needs you* arrives as links do.
- Hand to a session: brief review, ordinary place creation, a live work link.
- Decisions, as a plain dated form on the idea page.

No model involved except what the lane does. **This slice alone does not deliver
"the app helps me think".** It delivers "my thinking is kept, and connects to my
work".

**Slice 2: threads. The real bet.** Threads on an idea, the shared selector,
Harvest, and on-click help: clarify, split, draft a brief. If this does not make
ideas better than a notes app plus a chat, stop here.

**Stop rule.** Automations stayed for eleven releases before being removed, so
state the test up front. After a month of slice 1, look at whether thoughts were
ever revisited and ideas were ever handed off. If the scratchpad is write-only,
remove the surface and keep the capture command, or remove both.

**Deferred, each until real use asks:**

- Phone capture, through serve with its capabilities. Very likely the next thing
  after slice 2, since ideas arrive away from the desk.
- A second window.
- Card, spatial or map views.
- Project-side lens (ideas linking to this project).
- Thread templates.
- Named workspaces.
- Sharing an idea with another person (the friend, in fact).

**Not planned:** scheduled or background research, priority scoring, due dates,
hierarchy between ideas, and writing idea content into repos automatically.

---

## 10. Choices for the owner

Each of these changes what gets built. Recommended answers first.

**C1. Where does it live?**
(a) In the app, top level, window-ready. *Recommended (§7).* (b) A second window
from day one, if thinking beside the terminal is the point rather than a bonus.
(c) A standalone product, if you can imagine using this for ideas that have
nothing to do with code, which would change its whole audience.

**C2. How much lifecycle?**
(a) Open, parked, closed, with the rest derived. *Recommended (§2.2).* (b) Named
stages, if seeing "everything in research" at a glance matters more than the
upkeep. That reopens the case for a real board.

**C3. Where do the files live?**
(a) A folder the user owns and can move, default under the user's worktrees data,
plain markdown, optionally a git repo of their own. *Recommended:* keeps a notes
vault and backups possible, and survives the feature being removed. (b) Hidden in
the app's config directory: simpler, less exposed, and harder to use with anything
else.

**C4. A thread's agent and the document.**
(a) Proposes through Harvest, and only the person accepts. *Recommended (§5.3).*
(b) May append sources directly, and proposes the rest. (c) Edits freely, relying
on history to undo. Fast and likely to feel good for a week.

**C5. What runs first?**
(a) Slice 0 by hand, then slice 1. *Recommended:* the cheapest way to find out
this model is wrong. (b) Straight to slice 1, if you already know you would use it
and that by-hand tells you nothing about capture, which is the part you asked for.

## 11. Open questions

**Q1. Is one workspace enough for you?** All registered projects share one nav
today. If that mixes business and personal work, would you want their ideas apart,
or would a tag do? Only your use can answer this, and it affects naming more than
anything else in this document.

**Q2. Where does a thread physically run?** Worktrees' unit is one session per
worktree, and a thread has no worktree. Two shapes:

- A thread is a new kind of session, against the idea's folder. It is cleanest for
  the user, and it adds a kind of thing the engine must track.
- A thread is a place in a hidden `ideas` project. It reuses everything, but needs
  a branch per conversation, which is exactly the "not a branch" problem this
  proposal began with.

The first looks right, and this needs real design before slice 2. It also touches
agent limits and which harnesses qualify at first (likely those with a chat view).

**Q3. Do ideas follow you between hosts?** Ideas on the engine's host fit "one engine
per host", and the phone and remote windows then see that host's ideas. But if you
work on two Macs, a thought belongs to *you*, and not to a machine. A git-backed
folder (C3) may be enough. Sync hubs may help too.

**Q4. Can a thread read a linked repo?** "How would this fit our code?" is a natural
research question, and it is useful for a thread to read code without a lane. Is
read-only access to a registered project fine, or must it go through a place? Private
projects and cross-project opt-ins already draw lines here that it must respect.

**Q5. What about research that needs a checkout that does not exist yet?** Reading
Jellyfin's code means cloning it. Cloning for reference feels different from starting
a project, yet today it adds a project to your nav. Is a "reference clone" a useful
distinct thing, or is it just a project you remove afterwards?

**Q6. Does capture need to work with the app closed?** A system-wide hotkey is a
different feature from an in-app chord. The CLI covers terminals. Is that enough?

**Q7. Is "thought" the right word?** It avoids colliding with a place's note, and it
matches how you described them. It may also feel twee on a button. "Scratch" is the
fallback.

---

## Sources for §8.1

Checked 2026-11-06, and read for this example only.

- [Jellyswarrm](https://github.com/LLukas22/Jellyswarrm): README, repository
  metadata and `docs/health-monitoring.md` (its endpoints do not depend on the
  upstream servers), read directly.
- [Jellyswarrm issue #76](https://github.com/LLukas22/Jellyswarrm/issues/76): an
  offline server making the proxy unavailable, closed December 2025, read directly.
  The fix itself was not read or tested.
- [JellyfinFederationPlugin](https://github.com/Angablade/JellyfinFederationPlugin):
  repository description and metadata only.
- [`jellyfin/jellyfin`](https://github.com/jellyfin/jellyfin) and
  [`jellyfin/jellyfin-web`](https://github.com/jellyfin/jellyfin-web): separate
  repositories for the server (C#) and the web client (JavaScript).
- [Jellyfin forum thread](https://forum.jellyfin.org/printthread.php?tid=14081): the
  October 2025 developer comment, seen through a search summary, not read in full.
- [Lemmy discussion of Jellyswarrm](https://lemmy.thefloatinglab.world/post/94437):
  the password comment, also seen through a search summary. Verify both before
  relying on them.
