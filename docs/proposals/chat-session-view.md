---
title: "Proposal — a chat view of a place's agent"
---

# Proposal — a chat view of a place's agent, beside its terminal

**Status:** exploration, 2026-10-30. A prototype is on the branch
`feat/chat-session-view`, behind a Settings switch that is off by default. It
is not merged and not reviewed. No server change. Its three open questions
were decided by the user on 2026-10-30 (§9).

**The ask.** "Some users find the terminal-style view too challenging to use.
Early feedback on the worktrees-ios UI has been good, so its chat style is a
good starting point." In the iOS app, View session shows a place's session as a
chat (`worktrees-ios/docs/design/view-session.md`). The user's review of it
asked for "a real chat interface", answer buttons in the chat, and tappable
links.

**The recommendation in one paragraph.** Put a **Chat | Terminal switch on
each agent cell**. The terminal stays the default and stays mounted under the
chat, hidden, so going back takes one click and keeps its scrollback. The chat
is the iOS design on the same four serve rows (`agent_turns`, `agent_prompt`,
`agent_answer`, `agent_send`), adjusted for a console:
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
| Default | **Terminal**, for every place. The switch shows only while Settings → General → "Chat view" is on (a preview, off) | — |
| The terminal under the chat | **Mounted, hidden at its own size, and `inert`**. No resize reaches the pty, and the way back keeps the scrollback (the same rule as reading mode) | No terminal |
| Remembered | **No** (prototype). Every place opens on its terminal. The proposal for a release is an optional `place_panels[*].agent_view` | — |
| Live | Refetch on this place's `sessions:busy` / `sessions:done` change, on a send or an answer, and on Refresh. **While busy, re-read the turns every 10 s** if the chat is visible and the window is not hidden | Events only, no polling |
| Enter | **Sends.** No key adds a line break | Return does nothing; only the button sends |
| A prompt's choice | **Two clicks:** select, then "Send answer: “Yes”" | Two taps |
| "Always allow" (`host_only: widens`) | **Answerable** on the console (`agents.rs` lets `is_console()` through), with the note "Changes what the agent may do from now on". Refused when the window drives a paired host | Disabled, "Answer this one on the host" |
| A choice that opens a text field (`host_only: text`) | Disabled, "answer this one in the terminal" | Disabled |
| Paths in a reply | **Links that open in the dock** (the Files viewer's `resolve_doc_paths`, as the Plan tab uses) | Plain text |
| pi, kimi | A fallback card ("can't be shown here yet"). **The composer and the prompt card still work**: `agent_send` and `agent_prompt` cover every harness | Same fallback |

Three of these were the user's to make, and the user decided them on
2026-10-30 (§9): Chat beside the terminal with Terminal the default, Enter
sending, and two clicks for an answer.

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
conversation:
- the heading "Claude needs your answer", and its kind (Permission or
  Question);
- the prompt's own rows, **whole**, in the terminal's font, wrapped and never
  cut;
- each choice as a button with its exact label and any continuation rows.

A click selects a choice. **Send answer: “Yes”** then calls `agent_answer`
with the fingerprint the person was shown, and Cancel clears the selection.
After the answer:
- **On success**, the card goes and the conversation gets the line "You
  answered “Yes” · 10:59 PM", plus "· not confirmed" when serve could not
  see the prompt go.
- **On `the prompt changed`**, the selection clears and the card says "Not
  sent: the prompt changed before the answer reached it. Someone may have
  answered it in the terminal." A choice never carries over to a new prompt.

**An unreadable prompt** (`readable: false`) shows a card that says so and
offers Open the terminal. **"Answer in the terminal"** is always on the card.

## 6. What does not fit a chat, so the terminal stays one click away

| Thing | Why | What the chat does |
|---|---|---|
| Tool output and diffs | `agent_turns` carries tool **names** only | Chips under the reply; the top line names the terminal. A path in a reply opens the file in the dock |
| Slash commands | `typed_line_ok` refuses a leading `/`, on purpose | The refusal says "run in the terminal" |
| Esc / interrupt | No row presses Esc (§8) | Not offered |
| Free-text modals (`host_only: text`) | `agent_answer` would open the field and leave it | The choice is disabled and names the terminal |
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
| `app/src/settings.ts`, `SettingsSheet.tsx` | `chat_view` (default off), General → Chat view |
| `app/src/mock/install.ts` | Per-harness turns, the pi/kimi refusal, `?agentsend=mid\|nocomposer\|unconfirmed`, `?agentprompt=waiting`, `?activity=<n>` |

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

**Not done:**
- persisting the choice;
- a drift check holding `chat.ts`'s copy of `agents.rs`'s strings and
  `typed_line_ok`'s rule (`agent-rows-check.mjs` is the place for it);
- the iOS "New messages" announcement for VoiceOver;
- keyboard answers (1/2/3);
- a link's domain shown after its text.

## 8. Server asks (listed, not built)

| Ask | Why | iOS lives with |
|---|---|---|
| **A `turns:changed` event** per place (or `agent_turns` with `since`) | Removes the busy poll; a running turn shows as it happens | No updates mid-turn |
| **Paging:** `agent_turns` with `before` | A conversation longer than 20 turns, or 64 KiB | "Older turns are on the host" |
| **Tool-call summaries:** each tool's one-line input (the Bash command, the file path) and outcome | Chips that say *what* ran, not only that something did; the most-wanted piece of the terminal | Names only |
| **A streaming partial turn**: the reply so far, marked incomplete | Only with the event above; until then the poll shows each finished message | Nothing mid-turn |
| **`agent_interrupt`** (Esc into the lane's pane, `instruct`-class) | Stopping a runaway turn without the terminal | The terminal on the host |
| **Multi-line sends**: `\n` allowed for harnesses verified to take one bracketed paste, confirmed by "[Pasted text +N lines]" | Pasting a stack trace or a list | One line (`view-session.md` "Why a message is one line" has the design) |
| **Sent-from-chat marked in the transcript** | Telling a chat message from one another place sent | Matched by text and time |
| **pi and kimi transcript readers** | The fallback card goes | Same fallback |
| **A slash-command allowlist** (`/compact`, `/clear`) as a row of its own, not typed text | The commonest reason to go back to the terminal | Not offered |

## 9. Decided

The user decided all three on 2026-10-30. The prototype at 5e2da63 already
does each, so none of them changed code.

1. **DECIDED: beside the terminal.** Chat is an opt-in view next to Terminal,
   and Terminal stays the default for every place. Making Chat the default,
   for new places or for everyone, would be a NEW decision, taken only once
   the turns event (§8) exists.
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
1. Settings → General → tick **Chat view**.
2. Open a place with a running Claude. Press **Chat** on the agent's label row.
   The composer should take the keyboard.
3. Send "say hello in one word". Expect a dashed bubble, then the real turn
   and Claude's reply within about 10 s of the turn ending.
4. Ask for something that needs a permission (`touch probe.txt`). Expect the
   card, the two-click answer, and "You answered …".
5. Type into the TERMINAL without sending, switch to Chat and send. Expect
   "Claude's input already has text in it…".
6. Switch back with **Terminal**. Expect the scrollback intact, the keyboard in
   the terminal, and no reflow of the pane.
7. Repeat steps 2–4 in a Codex place. Then open a pi or kimi place, and expect
   the fallback card with a working composer.
