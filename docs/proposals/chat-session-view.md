---
title: "Proposal — a chat view of a place's agent"
---

# Proposal — a chat view of a place's agent, beside its terminal

**Status:** exploration begun 2026-10-30 on `feat/chat-session-view`. The
Settings switch is on by default since `feat/chat-default-on`. The three
original open questions were decided by the user on 2026-10-30 (§9). On
2026-10-31 it took in `feat/actionable-prompts`
(two new rows, richer prompts, and the prompt hook; on main as #581), and the
prompt card now answers every prompt those rows can read (§5). On 2026-11-01
it took in two of §8's asks. Its server changes:
- `agent_turns` answers each turn's `steps`, so an answered prompt stays in
  the conversation (§5, "Answered prompts stay in the conversation");
- each tool step says what it ran and whether it failed (§8, tool-call
  summaries);
- one new row, `agent_interrupt`, which the chat's Stop and Esc press (§8).

The first two are additive: an older client ignores them.

**The ask.** "Some users find the terminal-style view too challenging to use.
Early feedback on the worktrees-ios UI has been good, so its chat style is a
good starting point." In the iOS app, View session shows a place's session as a
chat (`worktrees-ios/docs/design/view-session.md`). The user's review of it
asked for "a real chat interface", answer buttons in the chat, and tappable
links.

**The recommendation in one paragraph.** Put a **Chat | Terminal switch on
each agent cell**. Claude and Codex start on Chat; pi and kimi start on
Terminal. The terminal stays mounted under Chat, hidden, so going back takes
one click and keeps its scrollback. The chat is the iOS design on the same
serve rows (`agent_turns`, `agent_prompt`,
`agent_answer`, `agent_answer_text`, `agent_answer_many`, `agent_send`,
`agent_interrupt`), adjusted for a console:
- no step-up or grant;
- the "always allow" choices are answerable here;
- Enter sends;
- a path in a reply opens in the dock.

It stays live from `sessions:busy` and `sessions:done`. While the agent works,
the one visible chat also re-reads its turns every 10 s, at about 10 ms a
read. A transcript-changed event would remove that poll. The rest of the
server asks are listed in §8. None of them blocks a first release.

## 1. Decisions

| Question | Decision | iOS does |
|---|---|---|
| Where | A per-place **Chat \| Terminal** switch in the agent cell's label row | A separate screen |
| Default | **Chat** for Claude/Codex; **Terminal** for pi/kimi. Settings → Behavior → "Chat view" starts on for new and upgraded installs; a later off choice sticks | — |
| The terminal under the chat | **Mounted, hidden at its own size, and `inert`**. No resize reaches the pty, and the way back keeps the scrollback (the same rule as reading mode) | No terminal |
| Remembered | **Per place** (2026-11-01): an optional `place_panels[*].agent_view`; absent follows the provider default without writing a record or adding a global. An explicit choice wins across place switches and relaunches, even while Chat view is off | — |
| Live | Refetch on this place's `sessions:busy` / `sessions:done` change, on a send or an answer, and on Refresh. **While busy, re-read the turns every 10 s** if the chat is visible and the window is not hidden | Events only, no polling |
| Enter | **Sends.** No key adds a line break | Return does nothing; only the button sends |
| A prompt's choice | **Two clicks:** select, then "Send answer: “Yes”" | Two taps |
| "Always allow" (`host_only: widens`) | **Answerable** on the console (`agents.rs` lets `is_console()` through), with the note "Changes what the agent may do from now on". Refused when the window drives a paired host | Disabled, "Answer this one on the host" |
| A choice that opens a text field (`host_only: text`) | Disabled, "answer this one in the terminal" | Disabled |
| Paths in a reply | **Links that open in the dock** (the Files viewer's `resolve_doc_paths`, as the Plan tab uses) | Plain text |
| pi, kimi | A fallback card ("can't be shown here yet"). **The composer and the prompt card still work**: `agent_send` and `agent_prompt` cover every harness | Same fallback |

Three of these were the user's to make, and the user decided them on
2026-10-30 (§9): Chat beside the terminal, Enter sending, and two clicks for
an answer. The Terminal default changed on 2026-11-01.

## 2. Layout at the app's real widths

The chat covers the agent cell below its label row. It is one reading column,
centred, at most 46 rem (690 px at the default size). On a wide cell, the
person's messages are right-aligned bubbles of at most 80 % of the column, and
the agent's replies are left-aligned cards the full width of the column. A
container query on the chat, not the window, tightens the margins and widens
the bubbles under 560 px. So the dock opening beside it is handled with no
code of its own.

Measured in the mock harness:

| Window | Nav | Dock | Chat cell | Column |
|---|---|---|---|---|
| 1440 × 900 | pinned | closed | 1052 px | 690 px, centred |
| 900 × 640 (the minimum width) | unpinned | open, 340 px | 472 px | the cell, tight margins |
| 900 × 640 | unpinned | closed | 812 px | 690 px |

The composer is pinned under the conversation, in the same column. It grows
to six lines as a long line wraps.

## 3. How it stays live, and what that costs

| Trigger | Rows called |
|---|---|
| The chat opens, or this place's `sessions:busy` state or `sessions:done` stamp changes | `agent_turns` (n = 20) + `agent_prompt` |
| A send confirmed, or an answer made | `agent_turns` + `agent_prompt` |
| Every 10 s while this place is **busy**, the chat is mounted and the window is visible | `agent_turns` |
| Refresh | both |

**Cost.** One `agent_turns` reads at most the last 2 MiB of the transcript
(`turns::TAIL_BYTES`) and answers at most 64 KiB. On this session's own 9 MiB
Claude transcript, a 2 MiB tail is 613 lines. Reading it took 1.6 ms and
parsing every line took 7.5 ms, using Python's `json` as a stand-in (serde is
no slower). So one read costs about 10 ms of serve's time. `agent_prompt` is
one `capture-pane`. **Only the selected place's chat fetches.** A place whose
chat is not on screen costs nothing, and neither does an idle or waiting
agent.

**Why poll at all.** `sessions:busy` moves when a turn starts and when it
ends, and a Claude turn can run for twenty minutes. Without a poll, the chat
would show "Claude is working" and nothing else for that long, while the
terminal beside it scrolls. iOS lives with that, because a phone is glanced
at. A desktop chat that replaces the terminal can't. The server ask in §8 (a
transcript-changed event) would remove the poll.

**Also used:** `sessions:drafts` (already pushed every 15 s). When the
terminal's input holds unsent text, the chat says so above the composer
before a send is refused for it.

## 4. The composer

**What it can send:** exactly what `agent_send` accepts, which is MCP `send`'s
rule (`harness::typed_line_ok`): one line, at most 4096 UTF-8 bytes, no
control or invisible format characters, and no leading `/ @ ! #`. It is
checked as the person types (`chat.ts::draftProblem`), so a refusal is
explained under the field before anything leaves. A pasted draft is never
changed silently. When the only problem is line breaks, the line offers
**Join into one line**, and the person decides.

**What it can't send, and what it says:**

| Case | Words under the field |
|---|---|
| A line break or tab | "Use one line: a message can't hold line breaks, tabs or other control characters." (+ Join into one line) |
| Bidi or zero-width characters | "Remove the invisible formatting characters: the agent would read something other than what you see." |
| Over 4096 bytes | "Too long: 4,312 of 4,096 bytes." A byte count appears from 3,500 |
| Starts with `/` | "Slash commands run in the terminal: a message can't start with /." |
| Starts with `@ ! #` | "A message can't start with !: at the agent's prompt that runs something, it is not a message." |
| A prompt is up | The field is paused: "Answer the prompt to continue" |
| Someone's draft in the pane (`ERR_MID_INPUT`) | "Claude's input already has text in it, typed in the terminal. Send it or clear it there, then try again." + Open the terminal |
| Not at its prompt (`ERR_NO_COMPOSER`) | "Claude isn't at its prompt yet (starting up, or a screen this app doesn't know). Try again in a moment." |
| Typed, not confirmed (`delivered: false`) | "Typed, but not confirmed. Look at the terminal before sending it again." |
| Its session has gone | "No Claude is running in this place any more." |

After a confirmed send, the message shows as a dashed bubble ("Delivered ·
waiting for Claude", or "Queued behind Claude's running turn"). It is
replaced by the real turn once the transcript has it, matched by text and
time as on iOS (`settlePending`). The draft clears only on a confirmed
delivery.

## 5. A waiting prompt

When `agent_prompt` answers `waiting: true, readable: true`, a card closes the
conversation, and what it asks always comes before its choices:
- the heading "Claude needs your answer", and its kind (Permission, Question,
  or Plan);
- for a prompt taller than the pane, its **`detail`, whole**, rendered per
  tool: an Edit as a line diff of `old_string` → `new_string`, a MultiEdit
  as one diff per change, a Write as its file, a Bash command as itself with
  its description, a plan (`ExitPlanMode`) as markdown with its file, and
  any other tool as its JSON. The clipped rows the terminal shows fold away
  under "The part the terminal shows";
- the prompt's own rows, **whole**, in the terminal's font, wrapped and never
  cut;
- its choices. A new prompt is scrolled into view, and one taller than the
  view opens at its top, so it is read before its choices are reached.

Answering still takes two acts, a selection and then a send (§9.3). How the
answer goes depends on the choice:

| Choice | On the card | Row |
|---|---|---|
| Plain | A button; **Send answer: “Yes”** | `agent_answer` |
| `text: "amend"` (Claude's Yes/No, every Codex question choice) | "Takes a note". Selected, an optional field: "No, and tell Claude what to do differently", "Yes, and tell Claude what to do next", or for Codex "Add a note". With a note, **Send “No” with the note** | `agent_answer_text` with a note, else `agent_answer` |
| `text: "field"` with `host_only: "text"` (an empty "Type something.", a plan's "Tell Claude what to change") | Selected, a required field, focused. **Send answer: “penguins”** | `agent_answer_text` |
| `text: "field"` without `host_only` (someone typed into it in the terminal; its label IS that text) | A plain choice, noted "Typed in the terminal: sent as it reads" | `agent_answer` |
| `host_only: "widens"` | Allowed from the console, noted "Changes what the agent may do from now on"; refused in a window driving a paired host | as its kind |
| `host_only: "text"` without `text` (kimi's fields) | Disabled: "answer this one in the terminal" | none |
| A multi-select's boxes (`multi`, `checked`) | Checkboxes from `checked`. The free-text box is a checkbox and a field; typing ticks it. **Send 3 choices** | The free-text box first with `agent_answer_text` (which ticks it and sends nothing), then the prompt read again, then `agent_answer_many` against the new fingerprint, only if it still shows that box ticked |

A multi-select's other choices ("Chat about this") are plain buttons below the
boxes. Claude's review screen after a multi-select is an ordinary question,
and gets an ordinary card. A two-question form is one card per screen, with
its tab bar in the first row. Codex's "No, and tell Codex what to do
differently" has no field: it is answered plainly, and the composer then takes
focus with "Now tell Codex what to do differently: type it here and press
Enter."

A field or a note is one line under `typed_line_ok`'s rule, checked as it is
typed, in the same words as the composer. Enter in it is the send.

After the answer:
- **On success**, the card goes and the conversation gets the line "You
  answered “No” with a note: “use a different name” · 10:59 PM", plus
  "· not confirmed" when serve could not see the prompt go. A field's answer
  reads "You answered “penguins”"; a multi-select's lists its boxes.
- **A refusal gets words** (`answerWords`), and re-reads the prompt when it
  says the prompt moved:

| Serve says | The card says |
|---|---|
| `the prompt changed`, `no such choice`, `… is not one of the boxes` | Not sent: the prompt changed before the answer reached it. Someone may have answered it in the terminal. |
| `…holds someone's text…` | Someone has typed into this choice’s field in the terminal. Answer it there, or pick the choice as it reads now. |
| `a multi-select: tick its boxes…` / `not a multi-select…` | This prompt is a multi-select: tick its boxes, then send them. / …no longer a multi-select. It has been read again. |
| `…takes no text…` | This choice takes no note. Send it without one. |
| `…did not open…` | The choice’s text field didn’t open, so nothing was typed. Answer it in the terminal. |
| `…could not be seen there…` | The text was typed into the field but couldn’t be seen there, so nothing was sent. Look at the terminal before answering again. |
| `name at least one box…` / `choice N is the free-text box…` | Tick at least one box. / Type your own answer in its box before sending it. |
| `someone has this question's notes field open…` | Someone is typing a note into this question in the terminal. Answer it there. |
| any other `… — answer it at the host` | Nothing was answered: <what serve said>. Answer it in the terminal. |

A choice never carries over to a new prompt: a fingerprint that moves clears
it, except while an answer is in flight (filling a box moves it on purpose).

**Claude's resume picker** (`picker: "resume"`, fix/chat-resume-picker): a
worktrees resume runs `claude -r`, which asks which earlier conversation to
resume before Claude has any session, so `agent_turns` fails ("…has written
no session file yet"). While a prompt is up that failure is no failure: the
conversation area says "Claude has no conversation open yet", and the card
says "Claude is asking which conversation to resume" (kind **Resume**). Its
choices are the conversations on Claude's screen, each its title (a lane's
are all its session name) over Claude's `age · branch · size` row; there are
no screen rows above them, since those are the picker's search box and key
hints. A list longer than the pane's window offers only the rows on screen
and says "4 of 7 shown. The other 3 are in the terminal's list", with **Open
the terminal**. Send reads **Resume “…”**; the conversation then gets "You
resumed “…” (2 hours ago · main · 1.2MB)", which stays, since no transcript
records the pick, and the turns are read again after 0.5, 1.5 and 3.5 s
until Claude has written its session. Digits and Enter work as on every
card, and the announcer says "Claude needs input".

**Claude's question with previews** (`previews`, fix/chat-question-preview):
an AskUserQuestion whose options carry a `preview` is drawn by Claude as
two columns, the options on the left and ONE option's preview in a box
beside them, and serve reads it as a question whose options each carry
`preview`. `previews: "all"` means every option's came from Claude's own
call (its PermissionRequest hook fires for AskUserQuestion) and agrees with
the screen; `"shown"` means only the one the terminal shows is known, its
`preview_hidden` rows cut off there. The card shows the selected option's
preview in a monospace block that keeps its spacing and scrolls rather than
wraps, beside the choices on a wide card and right under the selected
choice on a narrow one (`.chat-pv-wrap` is the container, 620 px); with
nothing selected, the highlighted option's. With `"shown"` it shows the one
it has, named, and says the terminal shows one at a time. The question has
ONE notes field (`notes`), which every option takes as an amend: "Takes a
note", sent with `agent_answer_text`, which moves onto the option, opens
the field with `n`, pastes, and sends it from there. A note someone typed in
the terminal (`notes.text`) goes with whichever answer is sent, and the card
says so; while the field is open there (`notes.open`) every choice is
disabled. "Chat about this" is the last choice, plain.

**An unreadable prompt** (`readable: false`) shows the bottom of the agent's
screen that serve sends with it (`lines`), says this app can’t answer it, and
offers **Answer it in the terminal**. A prompt is unreadable when the agent
was launched without the hook (before it existed, or by hand), when a tool's
input matches nothing (an MCP tool's long input), or when the record and the
screen disagree. **"Answer in the terminal"** is always on the card.

### Answered prompts stay in the conversation (2026-10-31)

Once a prompt is answered, the live card goes, and the conversation keeps it
as what was asked and then the answer, in the order it happened. Both come
from the TRANSCRIPT (`agent_turns`' new `steps`, `worktrees_core::turns`), so
they are the same whoever answered: this card, the terminal or a phone.
`reply` and `tools` are unchanged for older clients. Each step has its own
time, so a reply after a 12:22 answer is labelled 12:22, not with the time of
the turn's prompt.

| Asked | Drawn as | From (measured: Claude 2.1.316, codex-cli 0.160.1 scratch sessions) |
|---|---|---|
| A Claude question | The header and question; the picked option marked with its description, the others muted; typed text as a marked row | `AskUserQuestion`'s `input.questions`; `toolUseResult.answers` (a multi-select's labels joined by ", "; typed text is the value itself); "Chat about this" is a refusal whose `userFeedback` starts "The user wants to clarify" |
| A Claude permission | "Asked to use Bash" and the command's first line (a file's path, a URL) | `permissionDecision.source` `user_*` (asked) vs `config` (asked nobody: stays a chip); `toolDenialKind`; a No's reason is `userFeedback`; a Yes's note is a text block beside the result |
| A Claude plan | The plan, folded under its title | `ExitPlanMode`'s `input.plan`; approved, or sent back with `userFeedback` |
| A Codex question | As Claude's, with its note | `request_user_input`'s `arguments`; its output's `answers`, a note as `user_note: …` |
| A Codex approval | Not drawn: a tool chip | The rollout keeps the escalated call and its output, never the decision; a No's "aborted by user" is what an Esc writes too |

**The answer is a bubble on the user's side** ("Banana", "No" with its
reason under it, "Approved", "Sent back" with the feedback), and the card
above it marks the option. A bubble reads as the person's turn in the
conversation, and it is where typed text and notes fit. The card's mark says
which of the offered options it was.

**This window's "You answered …" line goes once the transcript has an answer
given after it was sent** (allowing 120 s of clock skew, one recorded answer
for each line), as a sent message's pending bubble does.

## 6. What does not fit a chat, so the terminal stays one click away

| Thing | Why | What the chat does |
|---|---|---|
| Tool output and diffs | `agent_turns` carries each call's name, one-line input and whether it failed (§8), never its output | A row per call under the reply; the top line names the terminal. A path in a reply opens the file in the dock |
| Slash commands | `typed_line_ok` refuses a leading `/`, on purpose | The refusal says "run in the terminal" |
| Esc / interrupt | `agent_interrupt` (§8) presses ONE Esc, only while the pane shows a turn running, and refuses a prompt, an idle agent, a draft typed in the terminal, and pi and kimi | Stop beside Send, and Esc in the chat, while the agent works; a refusal that leaves a turn running names the terminal, where it can still be stopped |
| Free text the rows cannot type (kimi's fields: `host_only: text`, no `text`) | No row fills them | The choice is disabled and names the terminal. Claude's and Codex's fields and notes are answered on the card (§5) |
| Images, a pasted screenshot, a file drop | One line of text only | Not offered. The terminal's drag and drop is unchanged |
| pi, kimi | No transcript reader (`agents.rs`) | A fallback card; sends and prompts still work |
| ⌘F | Finds in the terminal | Off while Chat is shown |
| Scrollback past 20 turns | `turns::MAX_TURNS` | "The latest 20 turns" at the top |

## 7. The prototype

| File | What |
|---|---|
| `app/src/chat.ts` | The rows' shapes, `agents.rs`'s refusal strings, the composer's rule, the refusal words, the conversation's items (ported from iOS `ViewSession.itemsOf` / `session.settlePending`), and a per-place store (memory only) |
| `app/src/ChatPane.tsx` | The chat, the prompt card, the composer, and the `AgentViewSwitch` |
| `app/src/ChatPane.css` | Plain CSS on `tokens.css` |
| `app/src/App.tsx` | The switch in `.agent-label`, the terminal wrapped in a hidden, `inert` `.agent-term`, and the activity, done stamp and pane draft passed in |
| `app/src/settings.ts`, `SettingsSheet.tsx` | `chat_view` (default on, migrated once on upgrade), Behavior → Chat view; each place's optional `agent_view` (`agentViewFor`, `withAgentView`) |
| `app/src/mock/install.ts` | `?turns=answered` (a transcript whose prompts were answered, as `steps`; a mock answer lands in the transcript 1.5 s later), per-harness turns, the pi/kimi refusal, `?agentsend=mid\|nocomposer\|unconfirmed`, `?agentprompt=permission\|question\|multi\|plan\|tall\|unreadable\|resume\|preview\|preview-shown\|preview-note\|off` (`,waiting` for waiting places only; a Codex place gets Codex's approval; `resume` is Claude's resume picker, four conversations shown and three more in the terminal, with `agent_turns` failing as before a session exists until a pick, and for half a second after it; `preview` is Claude's question with a preview per option, every one known, `preview-shown` the same at 80×24 with only the terminal's one, cut, and `preview-note` one with a note typed in the terminal), `?agentanswer=unseen`, `?activity=<n>` |

**Checked:**
- **Contrast**, in all six themes, by the browser's resolved colours. Body
  text is ≥ 5.2:1 everywhere. The dim lines (times, the line under the field,
  tool chips) are 2.8–3.9:1 in tokyo-day and catppuccin-latte. Those are the
  app's own `--txt-dim` on its own surfaces, the same pairs the nav and the
  dock already use.
- **WebKit**: the switch, the choices, the send button and Send answer
  measure the same as in Chromium, and each is hit-tested by
  `elementFromPoint`.
- **The hidden terminal** is `visibility: hidden` and `inert` while Chat is
  shown.
- **The 2026-10-31 card** (fields, notes, boxes, `detail`, unreadable lines),
  in the mock harness at 1280 px and in the 472 px cell, dark and light, and
  in WebKit: each send button is hit-tested. A diff's words are ≥ 4.87:1 on
  its tints in all six themes, and the tints sit 23–37 RGB units off the
  field. Its `+`/`−` marks are `--txt-dim` (2.5:1 in tokyo-day), beside the
  tint that carries the same meaning.

**Not done:**
- ~~persisting the choice~~: done. Each place keeps its explicit view in `place_panels` (`agent_view`, absent uses the provider default, no global), across switches and relaunches, and keeps it while Chat view is off (`placeview-check.mjs`);
- ~~a drift check holding `chat.ts`'s copy of `agents.rs`'s strings and
  `typed_line_ok`'s rule~~: done, in its own `chatmirror-check.mjs`;
- ~~the iOS "New messages" announcement for VoiceOver~~: done on
  `feat/chat-voiceover`. One polite, atomic `.sr-only` region says "1 new
  message" / "N new messages" and "Claude needs input" (`chat.ts` `hear`,
  held by `chat-announce-check.mjs`; `__mock.agentReply` drives it). Checked
  at the DOM in Chromium and WebKit, not yet heard with VoiceOver;
- keyboard answers: **built** (`feat/chat-answer-keys`). A digit is a click
  on that choice (it selects, ticks, or selects and focuses the choice's
  text), Enter is the card's Send, and a new prompt takes the keyboard from an
  empty composer, once per prompt; a word typed at the card goes back to the
  composer, so a message never answers. Checked by `chat-keys-check.mjs`, and
  in the mock harness in Chromium and WebKit, counting one answer per Enter;
- a link's domain shown after its text.

## 8. Server asks (listed, not built)

| Ask | Why | iOS lives with |
|---|---|---|
| **A `turns:changed` event** per place (or `agent_turns` with `since`) | Removes the busy poll; a running turn shows as it happens | No updates mid-turn |
| **Paging:** `agent_turns` with `before` | A conversation longer than 20 turns, or 64 KiB | "Older turns are on the host" |
| **Tool-call summaries — BUILT** (`feat/chat-tool-summaries`, 2026-11-01): a `tool` step carries `summary` (Claude: the command, the file, Grep's `<pattern> in <path>`, Glob, WebSearch, Agent/Task's description, the Skill, "N items"; Codex 0.160.1's code-mode `exec`: its `cmd` literal or its patch's files, else the `CommandExecution` event's argv; never an MCP tool's) and `failed: true` (Claude's `is_error`; Codex's `Script failed`, `aborted by user`, or a failed `CommandExecution`), control and invisible characters removed, the oldest summaries shed first to keep 64 KiB. The chat draws a row per call (the line in monospace, cut to one row, root stripped, whole in the title; a failure as a dot AND "failed"), three then "+N more · M failed"; an older host keeps the name chips. Mock: `?turns=tools` | Chips that say *what* ran, not only that something did; the most-wanted piece of the terminal | Names only |
| **A streaming partial turn**: the reply so far, marked incomplete | Only with the event above; until then the poll shows each finished message | Nothing mid-turn |
| **`agent_interrupt`** (Esc into the lane's pane, `instruct`-class). **Built** (`feat/chat-interrupt`): ONE Esc, only on a fresh capture that shows the turn running, proved or `unconfirmed`; the composer's Stop and Esc while the agent works; mock `?agentinterrupt=ok\|notbusy\|prompt\|draft\|unconfirmed\|unsupported\|old` | Stopping a runaway turn without the terminal | The terminal on the host |
| **Multi-line sends**: `\n` allowed for harnesses verified to take one bracketed paste, confirmed by "[Pasted text +N lines]" | Pasting a stack trace or a list | One line (`view-session.md` "Why a message is one line" has the design) |
| **Sent-from-chat marked in the transcript** | Telling a chat message from one another place sent | Matched by text and time |
| **pi and kimi transcript readers** | The fallback card goes | Same fallback |
| **A slash-command allowlist** (`/compact`, `/clear`) as a row of its own, not typed text | The commonest reason to go back to the terminal | Not offered |

## 9. Decided

The user decided all three on 2026-10-30. The prototype at 5e2da63 already
did each, so none of those decisions changed code at the time. The first
decision's default was superseded on 2026-11-01.

1. **DECIDED: beside the terminal.** The 2026-10-30 decision kept Terminal as
   every place's default. The 2026-11-01 decision supersedes that default:
   Claude/Codex places open on Chat, pi/kimi places on Terminal, and existing
   installs get Chat view enabled once. A later explicit off choice persists.
2. **DECIDED: Enter sends** on the desktop. This differs from iOS, where a
   stray Return should not instruct an agent. A desktop keyboard's Enter is
   deliberate, and the terminal sends on Enter too.
3. **DECIDED: two clicks to answer a prompt**: select a choice, then "Send
   answer". This is kept from iOS, because an answer can run a command. A
   terminal answers with one key.

## 10. What only the real app can answer

The mock answers instantly, so these need a hand test (below):
- whether `agent_send` into a live Claude lands, and the pending bubble then
  settles;
- how often the 10 s poll actually adds a message mid-turn;
- whether `visibility: hidden` + `inert` really keeps WKWebView's xterm from
  the keyboard after a switch;
- whether `sessions:drafts`' 15 s beat is too slow to warn before a refusal.

**Hand test** (`app/scripts/sandbox.sh --app`, the user drives it):
1. Settings → Behavior → Chat view is on by default. Leave it on.
2. Open a place with a running Claude. Chat should already show, with focus in
   the composer.
3. Send "say hello in one word". Expect a dashed bubble, then the real turn
   and Claude's reply within about 10 s of the turn ending.
4. Ask for something that needs a permission (`touch probe.txt`). Expect the
   card, the two-click answer, and "You answered …".
5. Type into the TERMINAL without sending, switch to Chat and send. Expect
   "Claude's input already has text in it…".
6. Switch back with **Terminal**. Expect the scrollback intact, the keyboard in
   the terminal, and no reflow of the pane.
7. Repeat steps 2–4 in a Codex place. Then open a pi or kimi place: Terminal
   should be selected. Press Chat to see the fallback card and working composer.
