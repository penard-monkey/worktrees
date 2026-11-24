---
title: "The workspace view: four designs, critiqued"
---

# The workspace view: four designs, critiqued

**2026-11-24.** Four agents answered one brief: fable, codex, kimi, and memdoor
(a local 27B model). This reads all four against the app's code, against
[the engine proposal](workspace-chats.html), and against the document pane the
documents lane has already built. The design to build is
[the combined one](workspace-view-design.html).

Marks: **checked** is read in the code. **measured** is run: the documents
lane's commit `76b981a`, copied to a scratch folder and driven in the mock, in
headless Chromium and Playwright WebKit. Neither is the app's own WKWebView.

## 0. The short version

- **fable** is the base. It is the only one drawn from the app's real grammar.
  It over-engineers the float and makes one false statement to the person.
- **kimi** has the best honesty about failure and the best use of attention.
  It is weakest on build order, and it contradicts itself about Undo.
- **codex** has the sharpest lifecycle rules and the weakest screens. Its
  overview is today's complaint drawn larger, and it leaves documents out of
  the nav.
- **memdoor** got the shape of the brief and not the facts. Colours, sizes,
  a privacy rule and an owner decision are wrong. Nothing in it is needed.
- **The proposal's §4** has no documents in the nav and no landing page. Its
  §2.5, §3.1 and §5.5 each have a hole a screen falls into. Nobody found two of
  them (§7).

What all four missed:

1. **The nav's dot cannot do what they say today.** They and the proposal
   want amber when a chat waits. A workspace's rollup is `busy` first, then
   `waiting` (**checked**, `App.tsx:9440`, and the same in `:9207`, `:9517`).
   One working lane hides every waiting chat. The order has to change.
2. **Home lights up in a workspace.** Its class is `sel || guestOpen ? "" : " on"`
   (**checked**, `App.tsx:9740`; **measured**: `home-item on` with the overview
   open). Two rows look selected.
3. **The filter and ⌘K know places only.** `matchPlace` takes a place
   (**checked**, `App.tsx:8549`) and Enter picks `li.row[data-slug]`
   (`:8567`). A chat row is invisible to both unless somebody adds it.
4. **The document header does not fit beside a chat.** **Measured** in both
   engines: the tools need 378 px, so the header is 44 px tall at 1052 px,
   79 px at 700 down to 484, and 112 px at 400 and 340. Every mockup draws one
   row at widths where the built pane has two or three.
5. **Two sessions, one person.** Three of them build a rule against two live
   editors. The built pane already saves when the editor loses focus
   (`useDoc.ts:266`; **measured**: the write landed 3 ms after blur in
   Chromium, 4 ms in WebKit). Moving between windows is a blur, so the text is
   saved before the person can type in the other one. The rule needed is much
   smaller (§2, split 6).

## 1. fable

**Gets right**

- The same workbench as a place: the agent in the middle, what you read on the
  dock side. It is the one argument from how the app already behaves.
- Documents in the nav for 30 px, as a strip. The measured row is 29 px, so
  three rows cost 87.
- Widths that start from the app's floors (420 and 240, `settings.ts:768`,
  `:771`) and its rails. Its 1052 px at 1440 is right (**measured**).
- A commit per editing session. The pane autosaves 800 ms after a pause, so the
  proposal's commit per save would bury the folds.
- The fold's states, drawn one by one. The clock that stops while a prompt waits.

**Wrong or unbuildable**

- **"Memory was not changed" on a failed fold.** It cannot know. The agent
  writes the index and the note in any order and can stop between them. kimi
  says this in words. fable's mockup says the opposite to the person.
- **"Usually under a minute"** is in its UI, and its own §9 says there is no
  number behind it.
- **"Read never loads the editor."** The built `EditorHost` is mounted always
  and only hidden in Read (**checked**, `DocumentPane.tsx`, `EditorHost.tsx`),
  and `useDoc` starts in `edit`. So Read loads CodeMirror today. Opening in
  Read is a change to the built pane.
- **One editor per document, across two windows.** It names no mechanism, no
  way for one window to know the other has gone, and no answer when a window
  dies holding the claim. §0.5 says why it is not needed.
- **The pane in three hosts.** The built main window holds one open document
  (`mainDoc`, one state). A dock that edits is a second session on the same
  file inside one window.
- **Its fold screen is unreadable at 1440.** A 250 px list of changes plus a
  diff in what is left of 578 px: the diff wraps about every five words in its
  own PNG.
- The slices were written before the documents lane landed. Slice 5, "the long
  pole, start it first", is built.

**Only fable**: the dock tab for a place, the crumb, Redo, "reads 2 of 3
projects", the amber dot on the Workspaces switch, the purple "changed since you
looked" dot, fold state in the event from the first day, the selection carrying
its workspace, file rows for drop, rename and remove, what a floated list shows.

## 2. codex

**Gets right**

- **The path you came in by wins** when a project is in two workspaces.
- Outcomes of a fold named apart: a note only ("Chat note added"), nothing
  ("No memory changes"), unknown after a restart ("Checking fold status").
- **One fold at a time.** It is the only one to say the app must serialise
  folds so that a receipt is one chat's work.
- An in-flight save stays tied to the document it opened. Relative links in an
  old version resolve in that version. A read failure is not "no memory".
- Its slice checks are the best of the four: "Fold two chats and edit a shared
  file between the first fold and Undo. Check an unrelated later edit survives."

**Wrong or unbuildable**

- **No documents in the nav.** The owner: "incorporated into the navigation of
  our primary app". A tab bar in the main area is one click too late for that.
- **The overview shows counts.** "Memory, updated 1 h ago. Plan, 2 work items.
  Ideas, 2 linked." That is today's "plan / ideas" panel with borders. Its own
  PNG leaves more than half the room empty. "Plenty of real estate" is unanswered.
- **A chat and a document never meet on a laptop.** The split needs 1,100 px of
  main area. At 1440 with the nav pinned there are 1052 (**measured**). Its own
  picture of it is 2,038 px wide.
- **The editor.** It wants the companion's bundle embedded and a spike. The owner
  decided a lazily loaded module the same day, and it is built. Its branch
  predates the words, so this is age, not judgment. Its slice 3 is void.
- **Undo of the note is put to the owner as a decision.** Revision 2 decided it:
  the note is in the commit and reverts with it (§3.2).
- **Three headers over a place**: the workspace bar, the place's top bar, the
  agent's label. It also moves project-title clicks in Workspaces only, so one
  row means two things in two views of the nav.
- A permanent line under the chat header explaining Close. People read it once.
- Zero closed chats in the nav. The chat closed this morning is the one wanted.
- The mockups are not this app: hand-drawn glyphs for icons, no search, no Home.

**Only codex**: the items under "gets right", scroll and selection carried
through Float, a supported way to enter a workspace from All projects.

## 3. kimi

**Gets right**

- **An attention line on the overview**, only when something needs a person.
- **Honest failure**: "memory may contain edits made before it stopped".
- **One file of the diff at a time**, with a file list. It is the readable form
  at real widths, and codex reached it too.
- A dashboard that fills the room with the memory itself.
- A conflict blocks Float. A revert that meets later edits never restores an old tree.
- It re-read revision 2 and cites it by line.

**Wrong or unbuildable**

- **It contradicts itself on Undo.** §3.2 and §4.3: the note reverts with the
  fold. §8.4: "reverting the memory commit while preserving the separate chat
  note". The second is left over from revision 1.
- **Its picture contradicts its text.** "Chat note saved … changed 3 files", and
  the three files are `MEMORY.md`, `topics/storage.md` and an SVG. The note is
  not among them, though §4.3 says it must be.
- **The lightbulb becomes a Documents library with "Workspace memory groups"**,
  in slice 1. That is a second way round to memory, which §1 argues belongs to
  its workspace, and it puts the largest move first.
- **"No additional engine row is required."** Its own §3.3 pastes an image into
  `files/`. No row writes one. Its conflict handling needs a change event with
  paths, and the proposal's carries only a workspace id (§7.5).
- **Three document rows** per workspace. 87 px each, before a chat is drawn.
- **Close & fold is the loudest button on the screen**, filled, beside Terminal.
  It ends the session. It should not look like Send.
- The chat takes over the top bar and the workspace's name goes with it.
- **Widths.** It draws one 44 px rail and a 1256 px main area at 1600. The default
  layout has two rails: 1212 with the nav pinned. Its 556 px at 900 is 512.
- The mockup shows a chat that made a plan item ("ready for Review and Start").
  That is the proposal's phase 4. The text says so. The picture is what is seen.
- Accent blue as text (`make deploy`, the commit id). AGENTS.md: a fill, not text.

**Only kimi**: the attention line, an image's before and after in a diff, the
removed workspace's folder being findable, words in the nav under a row that
needs you, an explicit rule that the tree has no 200-row expansion.

## 4. memdoor, the local-model experiment

Judged on the same terms. It answers all eight questions in the right order. It
picks the sensible side on chats above projects, three closed chats, waiting
first, and a tabbed main area. Then:

- **Colours are wrong by name.** Amber is given as `--ok #9ece6a` (green). Green
  is given as `--ai #bb9af7` (purple). "Muted purple" is `--sticky` (grey).
  **checked**, `tokens.css:39`, `:40`, `:62`, `:64`. Built as written, a waiting
  chat is green and a working one is purple.
- **It reverses the private-project rule.** "The plan viewer reads them via
  `--add-dir`". §1.3: named, never added. It also lets a chat read an
  unregistered project "if the folder exists".
- **It reverses an owner decision**: "the owner decided on CodeMirror in the
  Ideas window". He decided the opposite, and it is in the brief.
- **The lightbulb** is called "New idea". It opens the Ideas window (`App.tsx:9710`).
- **Arithmetic.** 600 px of main area at 900 ignores both rails. It splits that
  300 and 276, under both of the app's floors. A row of 78 + 15 + 200 + 40 px
  does not fit its 300 px nav. Three closed rows are 87 px, not 200.
- **Invented progress.** "Processing… (30%)", and a result "after 5 seconds".
- **Save is ⌘S.** The pane autosaves. "Merge" does not exist in it.
- Documents stay out of the nav on purpose, and Plan and Ideas still open the
  Ideas window. It answers the old app.
- It never wrote the mockups.

**How it compares.** The other three differ on judgment. This one differs on
facts it had in front of it. It restates the brief's sketch fluently and cannot
be trusted on a single number. As a first reading of what a brief is about it
was cheap. As a design it adds nothing the proposal's §4 had not said.

**Only memdoor**: the index's length in lines rather than KB. Not adopted: a
long line is what costs.

## 5. The comparison

| Question | fable | codex | kimi | memdoor | Combined |
|---|---|---|---|---|---|
| **1. Nav: where chats sit** | Above projects | Above projects | Above projects | Above projects | Above projects |
| Documents in the nav | A strip of three chips | None | Three rows | None | The strip |
| Live and closed chats shown | All live, last 3 closed | Up to 3, no closed | All live, up to 2 closed if room | Live, 3 closed | All live, last 3 closed |
| A row's words | Age; state for fold | Age | Age; a second line for state | State and age | Age; a word when it needs you |
| All projects | A dot on the switch | Nothing | Nothing | Everything Else as today | The dot |
| Lightbulb | Stays; library in main last | Library in main now | Documents library in main now | "Stays" (misread) | Stays; library last, if at all |
| **2. Main, no chat** | Overview: index and cards | Overview: counts | Dashboard: index and cards | The Memory tab | Overview: attention, index, cards |
| Tabs | Five, first Overview | Five, first Overview | None; a dashboard | Four | Five, first Overview |
| **3. Chat and document** | Chat middle, documents dock side | Chat left, document right | Chat left, document right | Documents left, chat right | Chat middle, documents dock side |
| Both at once from | 760 px | 1,100 px | 960 px | Always | 760 px |
| Opens in | Read | Read | Read | Read, edit on click | Read; ideas in Edit |
| Float | Moves; a card; one editor | Moves; a card; one editor | Moves; a card; one editor | Copies | Moves; a card; no lock |
| History | One word for all; log beside diff | Commits, then a diff | Commits; one file at a time | A list at the foot | One word; one change, one file at a time |
| **4. Close** | A menu button; click folds | A split button | A filled button | A button | A menu button; click folds |
| A fold running | One line for the composer | Composer paused | A progress bar | An overlay with a percent | One line for the composer |
| A failed fold says | "Memory was not changed" | The reason when known | "may contain edits" | "A partial commit may exist" | What changed, by name |
| After it lands | A chip; History opens | A receipt; diff on request | A card; diff opens | A card | A receipt; the diff opens unless you are editing |
| **5. A place** | Crumb and a dock tab | Workspace bar over everything | A strip above the place | Links in the header | Crumb and a dock tab, read only |
| A project's name | Folds, as today | Opens a project page | Folds, as today | Not said | Folds, as today; asked |
| **6. 900 px** | Both, nav unpinned; one, pinned | One | One | 300 and 276 px | Both, nav unpinned; one, pinned |
| 200 files | Folders by month, a filter | A filter, folders | A filter, folders | Pages of 50 | Folders by month, a filter |
| **7. Build order** | 8 slices, view first | 6, nav scope first | 7, pane and library first | 5 phases, engine's | 7, on the built pane |
| **8. Asks of the engine** | 9 | 4 | 0 | 0 | 12 |

## 6. Where they split, and my verdict

1. **Which side the chat is on.** fable, codex and kimi: chat first. The
   proposal and memdoor: documents first. The Ideas window is documents first
   too (`companion.css:106`), and none of the four said so.
   **Chat in the middle, documents on the dock side, following `dock_side`.** The
   deciding fact: with the dock tab of split 5, the workspace's memory is in the
   same spot whether you are in its chat or in one of its places. The cost: with
   no chat open the documents are full width, and they move over when one opens.
2. **What a workspace shows with nothing open.** **An Overview that contains
   memory itself** (fable, kimi), not counts (codex) and not the Memory tab
   (the proposal). From kimi: the attention line, and an excerpt that ends in
   "Read all of it". From fable: the index as the widest thing.
3. **Tabs or a dashboard.** Both. The dashboard is the first tab. kimi's
   objection, that tabs hide the rest, is answered by the first tab being
   everything.
4. **Documents in the nav.** **The strip.** codex and memdoor do not answer the
   owner. kimi answers for three times the height.
5. **A place inside a workspace.** **The crumb and the dock tab, read only at
   first.** It is the only answer that shows the documents *while* the place is
   on screen, which is the closest to "visible when you click on the project".
   codex's bar is a third header. kimi's strip replaces the place to show a
   document. Read only, because the built pane is one open document per window.
6. **Float.** Three say it moves. The built one stays open and editable in main
   (**measured**). **It moves, and there is no lock.** After the save, main
   closes its copy and shows where it went. If the person opens it here as well,
   blur saves, and a clean copy reloads. What is left is typing in both inside
   one poll tick, and the conflict banner that exists handles that.
7. **How wide for both.** **760 px** (fable): 420 + 340, the same arithmetic as a
   place and its dock. 1,100 fails on a laptop. 960 gives away the 900 px window
   for nothing. The document header must be rebuilt to fit (§0.4).
8. **The diff.** **One change, one file at a time** (codex, kimi). fable's two
   columns do not fit.
9. **Closed chats.** **Three** (fable, memdoor). A judgment, cheap to change.
10. **Close's weight.** **A plain button with a menu** (fable). Filled blue is
    for New chat and Resume, which start something.
11. **The lightbulb.** **Leave it.** The documents lane already opens linked
    documents in main. Unlinked ideas are the only thing the window alone
    reaches. codex and kimi move the whole library for that.
12. **Edit first or Read first.** **Read for memory and plans, Edit for ideas**,
    the last choice remembered per kind. fable and kimi argue Read for agent-written
    memory, which is right. For ideas it would undo a habit nobody complained of.

**Where all four agree**, and I do: chats above projects and in no project; one
shared pane; Close folds; the fold is shown and never only a toast; an ended chat
stays readable; several chats, one on screen; no chat inside a place's view; a
private project is named and not read.

## 7. The proposal's own §4, and the holes behind it

1. **§4.2 has no documents in the nav.** The owner's first sentence asks for them.
2. **§4.2's amber dot** needs the rollup's order changed (§0.1).
3. **§4.3 draws tabs and the chat's header in one row.** At 476 px of documents
   the tabs alone use most of it.
4. **§4.3's float** says main keeps the chat and not what stands in the
   document's place. The built Float leaves an editable copy there.
5. **§5.5's `workspaces:memory` carries a workspace id.** The pane's `watch` needs
   `{changed: [{id, mtime_ms}], removed: []}` (**checked**, `ideasave.ts`,
   `Changed`). Without a path and an mtime the open file cannot tell its own save
   from an agent's. Every conflict screen here rests on it. **No design noticed.**
6. **§2.5's commit per save** against an 800 ms autosave (fable).
7. **§3.1 step 5 ends a fold "when the note exists".** On a second fold the note
   already exists before Claude has read a word, and a session that has just
   launched reads idle. The fold can be committed empty in one tick. It must be
   the note's mtime past the fold's start. **No design noticed.** kimi's check
   ("its note does not double") would have caught it by accident.
8. **§3.1 lets the agent pick `<date>-<slug>`.** The engine waits for a file it
   cannot name. It must put the exact path in `fold.md` and in `chats.json`.
9. **§3.2's revert** does not say what a conflict does, nor that `git revert`
   refuses over uncommitted edits to the same files, which a live chat may have.
   Commit first, revert, and on a conflict abort and say so.
10. **§3** lets two folds run at once and accepts mixed commits. On the screen
    that becomes a receipt that lies. One at a time (codex).
11. **§4.4's "by (workspace, path)"** cannot use today's hand-off: `showRequest`
    accepts a 26-character id and nothing else (**checked**, `plans.ts:335`).
12. **§4.1.** The built document is gated on `ideasOn` alone (`docShown`,
    `openDocHere`). Memory with Ideas off needs that split first.
13. **The relaunch in §3.1** kills the tmux session under an open Terminal view. It
    has to attach again or the person sees "ended" mid fold.

## 8. Every change asked of the engine proposal

| # | Change | Asked by | Verdict |
|---|---|---|---|
| E1 | Fold state per chat in `workspaces:sessions`: folding, waiting, failed and why; right after a restart | fable, codex | **Keep** |
| E2 | `chats.json` records last activity and the fold's result (commit, files, lines, reverted) | fable | **Keep**. Add the note's path (§7.8) |
| E3 | A commit per editing session, not per save | fable | **Keep**. Modify: the engine commits after a quiet minute and before any fold, start or revert. No argument on the write row |
| E4 | What a revert that does not apply does | fable, codex, kimi | **Keep**: commit the dirty tree, revert, on a conflict `--abort`, answer with the files |
| E5 | Redo, a revert of a revert | fable | **Keep**. It is the same row |
| E6 | The five minutes stop while a prompt waits | fable | **Keep** |
| E7 | Close without folding, and Stop the fold, commit what is left under their own message | fable | **Keep**. Stop the fold is `workspace_chat_close` while folding |
| E8 | Rows to put, rename and remove a memory file | fable (kimi needs put) | **Keep put**, for a dropped image. **Drop rename and remove** for now: Finder, and an agent, can |
| E9 | `context.md` says which projects are not read; the chat rows answer the same list | fable | **Keep** |
| E10 | The selection carries its workspace | fable, codex | **Keep**. Frontend |
| E11 | One fold at a time in a workspace | codex | **Keep**. The second Close is refused in words |
| E12 | Undo of the generated note | codex, kimi §8.4 | **Drop**. Decided in revision 2 |
| E13 | Read and write with an expected revision | codex | **Drop**. `expected_mtime` is that |
| E14 | Memory must work with Ideas off | codex | **Keep** |
| E15 | Removed-workspace listing | kimi (later) | **Drop** for now. The removal notice names the folder |
| E16 | Lines, not KB, in the Tidy line; a two second overlay; 20 commits | memdoor | **Drop** |
| E17 | `workspaces:memory` carries paths and mtimes | nobody | **Add** (§7.5) |
| E18 | A fold ends on a note newer than the fold's start, at a path the engine named | nobody | **Add** (§7.7, §7.8) |
| E19 | Lists: fold results and history are answered by rows that make no git call per chat | fable, in part | **Keep**, through E2 |

Surface changes (§4) are in the combined design and are not repeated here.

## 9. What the built document pane needs

It is good, and its shape is right: a source, a session, slots in the header, a
pane that outlives its place on screen. A workspace view needs:

| Change | Why |
|---|---|
| The header folds. State, Edit \| Read, Float as an icon, the rest in `⋯` | 112 px tall at 340 px (§0.4) |
| Back goes away inside the workspace view | The tabs are the way back |
| `startIn`, and the editor mounted on the first Edit | Read first. Today it loads CodeMirror to read |
| Float closes main's copy after the save | §6, split 6 |
| `mainDoc` holds a source and a name, not an idea id | Memory |
| `ReadView` takes a link and an image resolver from the source | It follows `idea:` links only and draws no image (**checked**) |
| History as a slot | `versions` and `restore` are one document's. Memory's are the folder's |
| The gate is not `ideasOn` alone | §7.12 |
| The pane's section sits in a frame that stays mounted | Keeps "unsaved text survives a nav click" with a chat beside it |

## 10. What I did not establish

- Anything in the app's own WKWebView, or against a real engine.
- The mock's blur was an element's blur, not a switch between two real windows.
- What a fold takes, or costs.
- Whether fable's, codex's and kimi's agents saw each other's work. The brief
  forbade it. fable's and kimi's agreement on the dashboard is close.
