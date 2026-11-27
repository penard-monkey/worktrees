---
title: Where you left off, and the helper
---

# Where you left off, and the helper

**Investigation, 2026-11-27. Nothing is built.** The owner's idea
("Worktrees where you left off", 2026-11-10): on coming back after a while,
a VERY short digest across every project's recent sessions, from a helper
drawn from the app's icon that can point at things and be told "next" and
"what else". Added in the workspace chat on 2026-11-27: a walkthrough of the
app at first install.

Marks: **read** means I read the code (file:line given, at `ff78c38e`,
v0.55.0); **measured** means I ran it here (macOS 27.2, Claude Code on this
machine); **sourced** means a licence or vendor page, with its URL;
**unmeasured** means none of those.

## 0. The answer

- **The digest needs no new recording, only a new reader.** Everything it
  needs is already on disk per place. What is missing is one engine function
  that turns each place into a short *card*, across projects, with private
  projects left out.
- **The first version should use no model and no character.** A ranked list
  built by rule is instant, never fails, and is the fallback every later
  stage needs anyway.
- **A model then rewrites the cards into five lines.** Measured here on 17
  real cards: 4.8 s (Sonnet 5.5) to 11.4 s (Haiku 5.5), run in an empty
  folder so no repository's hooks run.
- **The character is the logo's own SVG, moved with CSS.** The icon is
  already eight vector shapes. No 3D render, no motion capture, no new
  library.
- **The walkthrough shares the pointing machinery, not the content.** Same
  bubble, same character, a fixed script in place of the digest.
- **"Talk to it" is a workspace chat.** The app already has a Claude that
  sits outside every repository and reads the non-private projects.

The two open questions, directly:

| Question | Answer |
|---|---|
| Should the animation use open motion-capture data? | **No.** The figure has one trunk, two arms and no legs, elbows or knees; a capture skeleton has twenty-odd joints and almost none map. Six to eight hand-made poses cover it. Of the sets checked, only two could legally ship anyway (§5.3). |
| Should the logo be 3D rendered first? | **No.** The logo is a flat mark that already exists as vectors (`app/public/icon.svg`). 3D adds a model, a renderer and lighting to reach a look the rest of the interface does not have. |

## 1. Where the knowledge comes from

### 1.1 What exists (read)

| Source | Type and file | What it gives a digest |
|---|---|---|
| A place's declared state | `Declared`, `crates/worktrees-core/src/store.rs:34`, in each repo's `.worktrees.places.json` | `title`, `note`, `lifecycle`, `pinned`, `last_opened_epoch` (:60), `last_worked_epoch` (:71, when an agent last finished), `last_seen_epoch` (:76, when the owner last looked), lineage (`started_by`, `under`) |
| Is an agent busy or waiting | `Activity`, `crates/worktrees-core/src/activity.rs:47`; Claude's probe `ClaudeProbe`, `agent.rs:30` | busy, waiting on the owner, idle; one reader for every harness |
| An unsent prompt | `agent::draft_from_screen`, `agent.rs:600` | "you were typing something here" |
| What was said | `Turn`, `crates/worktrees-core/src/turns.rs:80`; serve's `agent_turns`, `crates/worktrees-api/src/agents.rs:1530` | the last prompt and the last reply, already cut to size and already free of system prompts (Claude, Codex, kimi) |
| The plan | `PlanSummary`, `crates/worktrees-core/src/plan.rs:147` | `goal`, `current`, phases done and total, the brief's title and lead |
| Health | `HealthFacts`, `crates/worktrees-core/src/health.rs:29` | dirty files, unpushed commits, verdict `active`, `parked`, `at-risk`, `cold` |
| Pull requests | `Pr`, `crates/worktrees-core/src/github.rs:414`; cache in `crates/worktrees-api/src/project.rs:1139` | failing CI, changes requested, conflicts, merged |
| Ideas and plans made from them | `Idea`, `crates/worktrees-core/src/ideas.rs:432`; `plans.rs` | recently edited ideas; a plan's `## Goal` and `## Work` items |
| Workspaces | `Workspace`, `crates/worktrees-core/src/workspaces.rs:94`; `Chat`, `workspacechats.rs:198`; memory index, `memory.rs` | which projects belong together, a workspace's `MEMORY.md`, each closed chat's note |
| Messages between places | `Message`, `crates/worktrees-core/src/messages.rs:65` | a lane's last report to the place that briefed it (kept 7 days, per project) |

The app also already asks the question once: the Home screen's "RESUME WHERE
YOU LEFT OFF" list (`app/src/App.tsx:9160-9169`, drawn at `:11281`) shows the
six most recently active places across projects. It has names and ages and no
word about what was done.

Two records that look useful and are not: `ui-events.jsonl` holds clicks with
no place and no text, on purpose (`crates/worktrees-api/src/prefs.rs:181`),
and the audit log holds lengths and hashes, never text.

### 1.2 What is missing

- **No summary of a session.** Nothing stores what a session did. The one
  stored piece of model prose is the per-place "Claude's read"
  (`ai_status_report`, `crates/worktrees-api/src/config.rs:492`), made when a
  button is pressed.
- **No statement of overarching goals.** Goals exist per place (the plan),
  per plan document (`## Goal`) and per workspace (memory). The one real plan
  document read here has an empty `## Goal`. The digest can only point back
  at goals somebody wrote down.
- **No history of completions.** `last_worked_epoch` is one number per place.
  "Three sessions finished while you were away" can be counted only from
  transcripts.
- **No "last time the owner was here".** See §2.
- **Transcripts are too big to read raw.** Measured: 91 Claude transcripts
  touched in seven days hold 1.49 GB. The digest must be built from cards,
  never from transcripts.

### 1.3 The card

One card per place, built by the engine from the readers above: project,
place, title or note, state, plan goal and current step, the last prompt and
the tail of the last reply, health, pull request state, unread, draft, ages.
Measured on this machine: 17 projects hold 90 places; 17 places were active
in the last 14 days; their cards, without transcript text, came to 3,610
characters.

## 2. Detecting "coming back"

### 2.1 What the app tracks (read)

- `useWindowAwake` (`app/src/App.tsx:2797`) follows `visibilitychange`, which
  in this WebView fires on minimise, ⌘H, a Space switch and full cover, and
  not on plain loss of focus.
- `notify.ts:47` keeps `hiddenSince`, the moment the window was hidden. It is
  in memory only.
- `app/src/usage.ts:283` sees every `pointerdown` in the window, and keeps no
  "last input" time.
- The shim handles only `RunEvent::Ready` and `Exit`
  (`app/src-tauri/src/lib.rs:633-636`). Nothing observes sleep, wake or app
  activation.
- Per place, `last_seen_epoch` against `last_worked_epoch` already says
  "finished and not looked at" (`app/src/afterglow.ts:130`).
- Serve's idle rule (`OWNED_IDLE_GRACE`, `crates/worktrees-cli/src/serve/mod.rs:394`)
  measures whether the app is gone, not whether the person is.

### 2.2 The rule proposed

Keep one number, **last active**: the later of the last input in any app
window and the moment the window was last hidden. Persist it when the window
hides and when the page unloads.

A **return** is the first of these after last active is older than a
threshold: the window becomes visible, the window takes focus, or an input
arrives. The third case covers a laptop that slept with the window showing:
timers stop during sleep, so the first key or click after waking finds the
gap. No native wake observer is needed. Whether `visibilitychange` fires on
sleep or screen lock is **unmeasured**, and the rule does not depend on it.

Two gates keep it quiet:

- **Something changed.** No digest when no place has finished work, started
  waiting, or changed pull request state since last active.
- **Once per return.** Dismissed means dismissed until the next gap.

The threshold is the owner's call (§9). Four hours would show it once after
lunch and once the next morning; "first return of the day" is the calmer
alternative.

A launch after the app was quit is a return like any other, which is why last
active must be on disk. Sending a line from the phone is activity on another
device; `last_remote_send` (`store.rs:126`) records it per place, and the
first version can ignore it.

## 3. Producing the summary

### 3.1 Two layers

**Layer 0, by rule.** Rank the cards and print one line each from a template.
Order: waiting on the owner; finished and not seen; pull request failing or
changes requested; an unsent draft; uncommitted or unpushed work at risk;
then by recency. It is instant and cannot fail, and it is what the panel
shows first, every time.

**Layer 1, a model.** One headless run rewrites the top cards into at most
five lines and adds one line per workspace on its goal. It replaces layer 0's
lines in place when it arrives, and layer 0 stays if it does not.

### 3.2 How the model runs

- **Where.** In a folder the engine owns and that is not a repository, as
  workspace chats do (`crates/worktrees-core/src/workspacechats.rs:24-41`).
  The rule "never start a Claude to ask it something, it runs the
  repository's hooks" is about starting Claude *in a repository*.
  `ai_status_report` does exactly that today, behind a button
  (`config.rs:548`); a digest that starts by itself must not.
- **What it gets.** The cards on stdin, one turn, no tools, no project
  folders. It never reads a file.
- **What it returns.** Card numbers and a line each. The engine builds every
  link from the card, never from the model's text, so a transcript that says
  "open this other place" cannot forge a destination.
- **Which harness.** Claude first, because its headless mode is the one
  measured. The choice of provider can follow the workspace chat's later.
- **Deadline.** `proc::run_deadline_stdin` (`crates/worktrees-core/src/proc.rs:44`),
  20 s. Past it, layer 0 stays.

### 3.3 Cost and time (measured)

`claude -p --max-turns 1` in an empty folder, 17 real cards on stdin, one run
each:

| Model | Wall time | Cost the CLI reported | Output |
|---|---|---|---|
| Haiku 5.5 | 11.4 s | $0.005 | five usable lines |
| Sonnet 5.5 | 4.8 s | $0.088 | five usable lines |

About 34,000 input tokens per run were Claude Code's own prompt and this
machine's user configuration; the cards were about 1,200. On a subscription
the cost is plan usage, not money. Two runs are a sample, not a benchmark.

Both models invented a little: a card saying only "1 dirty file" became
"commit or discard the uncommitted change". Cards with a plan goal and a last
prompt would leave less to guess, and the template lines of layer 0 have no
such problem.

### 3.4 When

**On return, not kept warm.** A run prepared in the background spends the
owner's plan for a digest nobody may read, and every run is a Claude start.
On return the wait is 5 to 11 s and layer 0 covers it. The result is cached
in an engine-owned file with a fingerprint of the cards, so an unchanged set
of cards never runs twice.

### 3.5 Privacy

- A project marked private (`registry::Entry.private`,
  `crates/worktrees-core/src/registry.rs:96`) produces no card. The rule to
  copy is `project_reads` (`workspacechats.rs:675`), which also leaves out a
  project nested inside a private one.
- A project the system refuses to read produces no card, and the panel says
  how many were left out.
- The row answers through the caller's scope, so a paired device gets only
  the projects its grant covers. First version: console only.
- Transcript text in a card comes through `turns`, which already drops system
  prompts and reminders.
- Sending cards to a model sends them to the owner's own account, as every
  lane already does. A switch turns layer 1 off and leaves layer 0.

## 4. The dialog and the conversation

**A panel in the main window, not a new window.** The pattern is
`app/src/UpdateBubble.tsx` (90 lines): placed from an anchor's rectangle,
never takes focus, not modal. A modal dialog on return would stand between
the owner and the terminal they came back to type in.

- **Content.** At most five rows: project, place, one line. Above them, at
  most one line per workspace on its goal, when one is written.
- **A row leads somewhere.** A place: `enterPlace` (`App.tsx:7420`), which
  also marks it seen. A workspace chat or document: `showWs` (`:6958`). An
  idea: `floatDoc` (`:7168`).
- **"What else"** shows the next five rows of the same ranking. **"Next"**
  goes to the top row and moves it out of the list. Both are buttons and
  need no model.
- **Home.** The Home list asks the same question and should show the same
  rows, so there is one answer in the app.
- **Its own beta switch** (`app/src/beta.ts`), default off.

**Typed conversation comes later, and it is a workspace chat.** A workspace
chat is already a Claude in its own folder that reads the projects through
permission rules, with private ones denied, and is handed a `context.md` the
engine writes at every start (`workspacechats.rs:8-18`). The helper's chat is
that, over every non-private project, with the cards as its context. Building
a second chat system for it would repeat that work.

## 5. The character

### 5.1 The figure

`app/public/icon.svg` (1,436 bytes) is the whole logo: a trunk, two curved
branches, two small circles (hands), a ring (head) and a root circle (foot),
over a green squircle. It is a figure with raised arms on one leg. Every part
is a separate vector shape, so it is already a rig.

### 5.2 Animation routes

| Route | Effort | Asset pipeline | Licence | Fits this figure? |
|---|---|---|---|---|
| **Inline SVG moved with CSS** (recommended) | Small: the shapes exist; six to eight poses | The SVG in the repo, edited by hand | Ours | Yes. Three strokes and five circles |
| Sprite sheet | Small to medium | Draw every frame; redraw for each size and theme | Ours | Yes, but blurry at zoom and fixed in colour |
| Rive | Medium, plus learning the editor | `.riv` from Rive's editor | Runtimes MIT; 221 to 364 KB minified. The free plan's exports play a Rive splash screen; $9 a month removes it (sourced, [rive.app/pricing](https://rive.app/pricing)) | Yes, and more than it needs |
| Lottie | Medium | Glaxnimate (free, GPL tool) or After Effects | lottie-web MIT, 306 KB minified; files downloaded from LottieFiles carry a share-alike licence, ours would not (sourced, [licence](https://lottiefiles.com/page/license)) | Yes, but playback only: pointing at a live target needs code anyway |
| three.js with a 3D model | Large | Model in Blender, export glTF | MIT, 742 KB minified whole | Poorly: a flat mark made round |
| SceneKit or RealityKit | Large | USDZ model; a native view beside the WebView | System frameworks; SceneKit was soft-deprecated at WWDC 2025 (sourced, [session 288](https://developer.apple.com/videos/play/wwdc2025/288/)) | Poorly, and it cannot draw inside the page |
| Motion capture onto any of the above | Large | Retarget a human skeleton | See §5.3 | No |

Rive and Lottie would each be a fourth admitted library, with its own bundle
and boundary check as AGENTS.md requires. Nothing here needs one.

### 5.3 Motion capture, checked

| Set | Licence | May ship in this app? |
|---|---|---|
| CMU Graphics Lab | Custom: free, may be included in products, not resold as data | Yes |
| 100STYLE | CC BY 4.0 | Yes, with credit |
| Mixamo | Adobe terms: embedded use yes, raw files may not be distributed | Doubtful in a public repository |
| AMASS, SFU, Bandai Namco, LAFAN1 | Non-commercial or research only | No |
| Truebones, ActorCore, Rokoko | Files may not be redistributed, or terms not found | No, or unverified |

The CMU wording was confirmed from mirrors only (its own page failed its
certificate here), and Mixamo's from a community copy of Adobe's FAQ.

### 5.4 Moving across windows

| Where the helper can go | How | Cost |
|---|---|---|
| **Inside one window** | An element placed from the target's `getBoundingClientRect`, as `UpdateBubble` does | None. No permission, no window |
| **Between the app's windows** (main, Ideas) | Each window draws its own; a handoff through the existing localStorage asks (`app/src/doc/handoff.ts`), so it leaves one and arrives in the other | Small |
| **Over the app, outside its window edges** | A transparent, click-through child window of the main window | Tauri 2.11.5 (this repo's) needs the `macos-private-api` feature for transparency; 2.12.1 and later do not (sourced, [release notes](https://v2.tauri.app/release/tauri/v2.12.1/)). `tauri = { features = [] }` today (`app/src-tauri/Cargo.toml:29`) |
| **Over other apps' windows** | The same overlay, placed from the system's window list | Works without permission today, and Apple documents none of it |

Two throwaway Swift probes settled the last two rows (measured; sources kept
in `~/.cache/worktrees/worktrees/research-where-you-left-off/`, not in the
repo):

- `wins.swift`: an unsigned process with no Screen Recording permission read
  the app's window bounds from `CGWindowListCopyWindowInfo`, and got no
  window titles.
- `overlay.swift`: a borderless, transparent, click-through window at
  floating level appeared over the app while it was full screen, and the app
  stayed frontmost. A child window moved by exactly its parent's (120, 40)
  with no code moving it.

Neither probe used Tauri, so the Tauri side of the third row is sourced, not
measured.

**Recommendation:** stay inside the app's own windows. A helper that walks
over other applications is a different product, with a surface Apple may
change.

The character must respect `prefers-reduced-motion` and have an off switch
that leaves the panel working.

## 6. First-install walkthrough

### 6.1 Today (read)

- A fresh install shows Home with "No places yet — open a project to start."
  (`App.tsx:11282`) and "No projects yet. Add one from the rail." (`:10677`).
- There is no tour, coach mark or first-run flag. The one signal is an empty
  `last_seen_version`, and the same effect that reads it overwrites it
  (`App.tsx:5723-5724`), silently, so a new user gets no notes and no offers.
- The agent server and the command-line tool are set up from Settings; a
  suggestion reaches the user only through the after-update offers
  (`app/src/offers.ts`).
- Nearly every region already carries a stable hook: about 239 `data-testid`
  in `App.tsx` alone, for example `places-rail`, `ideas-rail`,
  `new-project-home`, `.row[data-slug]`, `.status-chip`, `aside.dock`.

### 6.2 What it should cover

Seven steps, each finished by doing the thing, not by pressing Next:

1. Add a project.
2. A place is a branch with its own folder and session; make one.
3. Start an agent in it with a brief.
4. The dots: working, waiting on you, finished and not seen.
5. Chat and Terminal are two views of the same session.
6. The dock: files, plan, documents, pull requests.
7. Set up agents: the command-line tool and the agent server, in Settings.

It can be skipped at any step, resumes where it stopped, and can be started
again from Settings.

### 6.3 Shared with the return panel?

**Yes for the machinery, no for the content.** Both need: a bubble anchored
to a real element, the character beside it, and a list of steps that each
name a target and a destination. The return panel fills the list from the
digest; the walkthrough fills it from a fixed script.

What the walkthrough needs of its own:

- A first-run key that existing users never trip. It must be decided at the
  moment `last_seen_version` is found empty, before that effect overwrites
  it.
- A check script, in the tradition of `dnd-check.mjs`, that fails when a
  step's selector no longer exists in the source. A tour that points at
  nothing is worse than none.

## 7. iOS

- **The digest: yes, later, as one host row.** The phone already pulls the
  whole workspace on every return to the foreground (`src/store.ts:194-200`
  in `worktrees-ios`) and shows a waiting banner on its host screen
  (`src/screens/Places.tsx:97`). A `digest` row, read-class and scoped to the
  device's projects, gives it the same five lines. A host without the row
  answers `unknown command`, which the phone already turns into "update the
  host" (`src/errors.ts:35`), and no protocol number changes.
- **What must be shared:** the row and its card type on the host, and "last
  active" kept per device, since the phone clears its data when it goes to
  the background.
- **The character: not now.** The phone has no animation runtime and only
  CSS motion (`src/app.css`). An SVG and CSS figure would carry over through
  the vendored submodule if it is ever wanted; any other route would not.
- **The walkthrough: no.** It is a different application with different
  screens.

## 8. The two linked ideas

- **"Have the app evolve over time."** It changes the walkthrough's shape:
  tips that appear when they become relevant (the first project, the first
  lane, the first finished session) fit that idea better than one tour at
  install, and they use the same bubble. It also overlaps the Beginner,
  Advanced and Expert modes in `settings-reorg.md`. Worth deciding together
  (§9, question 6).
- **"voice chat."** It does not change the recommendation. Voice would be a
  way in and out of the helper's chat (stage 5), and lines short enough to
  read at a glance are short enough to speak.

## 9. Recommendation

Build the digest as an engine row over cards, show it by rule first, add the
model second, and draw the character from the logo's own SVG third.

Rejected, a line each:

- **A modal dialog on return:** it stands between the owner and what they
  came back to do.
- **A model-only digest:** nothing to show for ten seconds, and nothing at
  all when it fails.
- **Summaries kept warm in the background:** spends the plan on digests
  nobody reads.
- **Running the model inside a project:** runs that repository's hooks.
- **Letting the model read transcripts or files:** 1.49 GB a week, and tools
  it does not need.
- **Rive or Lottie:** a new library and an outside editor for eight shapes.
- **3D and motion capture:** large work toward a look and a skeleton the
  figure does not have.
- **An overlay over other apps:** undocumented system behaviour and a
  private-API feature, for decoration.
- **A separate chat system for the helper:** workspace chats already are one.
- **A separate tour system:** it would duplicate the panel's anchoring.

### Stages

| Stage | Adds | Size |
|---|---|---|
| 1. The digest by rule | The card reader and a console-only `digest` row with the privacy rule; "last active" and the return rule; the panel with rows, "next" and "what else"; Home showing the same rows; a beta switch; the mock and a check | About a week |
| 2. The model's lines | The headless run in an engine folder, the deadline, the cache, the switch and model choice in Settings, goal lines per workspace | Three to five days |
| 3. The walkthrough | The step list and anchoring shared with the panel, the script, the first-run key, the selector check | About a week |
| 4. The character | The SVG rig, six to eight poses, pointing at a target, the handoff between windows, reduced motion and the off switch | One to two weeks, mostly drawing and tuning |
| 5. Talk to it | The helper's chat as a workspace chat over every non-private project, opened from the panel | About a week, on the workspace chats beta |
| 6. Optional | The phone's digest card; the overlay outside window edges | A few days each; the overlay needs a Tauri upgrade first |

Stage 1 is useful by itself. Stages 3 and 4 can swap.

## 10. Decisions for the owner

1. **How long is "away"?** Four hours, first return of the day, or a setting.
2. **Panel or dialog?** The note recommends a panel that does not take
   focus; the idea says "dialog window".
3. **May the app start a model by itself on return?** If yes, which model:
   Haiku 5.5 (cheaper, about 11 s) or Sonnet 5.5 (about 5 s).
4. **Should the Home list become the digest**, so the app has one answer?
5. **Is the character worth one to two weeks**, and who draws the poses?
6. **One tour at install, or tips that unlock** as the "evolve over time"
   idea suggests?
7. **Where do overarching goals live?** The digest can quote a workspace
   plan's `## Goal` or its memory, and the one read here is empty. Should
   the helper ask for a goal when it finds none?
8. **Should it ever leave the app's windows?** The note says no.
9. **Does the phone get the digest**, and does a return on the phone count
   as a return on the desktop?
10. **Does the helper have a name?**
