---
title: Worktrees iOS MVP — UI design
---

# Worktrees iOS MVP — UI design

Design proposal, 2026-10-22. Companion to [the implementation plan](ios-mvp.md)
at `39b350c`. This specifies the proposed phone UI; it does not claim that the
remote listener, new agent rows, native bridge or iPhone behaviour already work.
The plan remains authoritative for capabilities and protocol. Examples use
fictional session data. [Wireframes and reading order](ios-ui/README.md).

## 1. Principles

1. **Show the decision before the action.** A waiting agent gets a readable
   prompt, its host and place, exact choices, and an explicit confirmation.
   Nothing answers from a list row, notification or swipe.
2. **Make freshness visible.** “Working” means observed on this connection.
   A lost connection labels old activity as last known, disables mutations,
   and never turns a timeout into “Sent”.
3. **The place is the unit of navigation.** Projects group places; a place
   holds its agent, recent turns and actions. Closing a session leaves the
   place available to start again.
4. **Explain the phone's actual access.** Pairing starts at Observe. The
   host enables actions; the phone cannot widen its own grant. Interactive
   access says “Can run commands on Studio Mac as you.”
5. **Bring the desktop's meanings, at phone scale.** Green means working,
   amber means needs input, purple means finished. Words accompany every
   status. Use the existing tokens with readable type and generous controls.

## 2. Navigation and layout

One navigation stack per selected host; no terminal tab, sidebar or bottom tab
bar. Only the selected host connects in the MVP. With one paired host, launch
into its projects after connecting. With several, restore the last selected
host; **Hosts** is always the first back destination. A revoked or forgotten
selection returns to Hosts. Host switching clears the previous host's sensitive
in-memory content and requires discarding any draft.

```text
No paired hosts → Pair a host
                    ├─ Scan QR / Paste link (native input)
                    └─ Compare codes → Wait for host → Paired
                                                        └─ Projects & places
Hosts → Selected host: Projects & places → Place detail → Recent turns
  │                      │                    ├─ Answer prompt [sheet]
  │                      │                    │    └─ iOS Face ID/passcode
  │                      │                    ├─ Send message [sheet]
  │                      │                    │    └─ iOS Face ID/passcode
  │                      │                    ├─ Start session [sheet]
  │                      │                    ├─ Close session [confirmation]
  │                      │                    └─ Place details [sheet: note/pin/lifecycle]
  │                      └─ Connection → This device → Revoke / Forget [confirmation]
  └─ Pair another host [pairing flow]
```

Place titles can wrap; the persistent context line names the host and project.
Actions also name the target place. Back restores the prior scroll position.
Pairing has a visible Cancel at every pending step. A pairing link opens this
same flow, never bypasses comparison or approval, and never interrupts a draft
without a discard confirmation.

Use one sheet at a time. Prompt review and the composer expand to full height;
start and metadata use content-sized sheets that expand at large text sizes.
The close/revoke/forget confirmations replace the action sheet rather than
stacking another custom sheet. System authentication is owned by iOS. This
follows Apple's [sheets guidance](https://developer.apple.com/design/human-interface-guidelines/sheets).

Wireframes use a 390 × 844 logical-point canvas, a 47-point top safe area and a
34-point bottom safe area as illustrative device dimensions. Production reads
actual safe-area insets; content scrolls between the navigation bar and any
bottom action. Insets are not hardcoded from these drawings. Use 20-point
horizontal margins, the existing 4-point spacing scale and 50-point primary
buttons. Back, overflow, disclosure, radio and switch targets are at least
44 × 44 points. The keyboard reduces the visible content region; the focused
field and its action remain reachable by scrolling. Respect system safe areas
as described in Apple's [layout guidance](https://developer.apple.com/design/human-interface-guidelines/layout).

## 3. Shared state and action rules

These rules apply to **every** screen below; each screen specifies exceptions.
A server refusal always wins over a locally displayed capability.

| State | Rendering and behaviour |
|---|---|
| Initial loading | Preserve the screen title and back action. Skeletons for lists, “Loading…” for a bounded card. Never show an empty-state message until the request succeeds. No skeleton contains actionable controls. |
| Refresh | Keep currently visible content with “Updating…” and its timestamp. Pull to refresh also has a visible Refresh action for assistive technology. After reconnect, actions wait for a fresh grant and the relevant fresh data. |
| Empty | Explain the missing content and the next available action; no fake sample rows. Screen-specific words below. |
| Read failure | “Couldn’t load [content]. Try again.” Keep same-foreground memory only, marked last known. Retry and Back stay available. |
| Reconnecting | Persistent banner: “Reconnecting to Studio Mac…” and “Reconnect now”. Announce once, not on every backoff tick. Stop busy-dot animation and disable host mutations. |
| Offline | “Offline · Last checked at 9:41.” Connection details and Reconnect now remain available. Host records survive relaunch; prompts, turns and drafts do not. A cold offline launch has no cached project list. |
| Capability denied | Inline notice adjacent to the unavailable action: “This device can’t answer prompts. Change its access on Studio Mac.” Offer View device access. Read-only content remains if still permitted. |
| Missing or outside scope | “This place is unavailable. It may have been removed or access may have changed.” Back to projects. Do not identify hidden projects, reveal an ungranted name, or claim which cause occurred. |
| Unsupported host feature | “This host doesn’t support [feature] yet. Update Worktrees on Studio Mac.” No permission-change instruction for an absent command. |
| Mutation in flight | One request, disabled submit, “Sending…” / “Starting…” / “Saving…”. No duplicate request on repeated taps. Dismissing after transmission cannot cancel host work. |
| Mutation outcome unknown | “Connection lost. We couldn’t confirm whether [action] completed.” No automatic replay. Refresh state; sending again requires a deliberate new decision. |

Reconnect follows the existing retrier with remote starts disabled: one attempt
at a time, backoff capped at 30 seconds, paused while hidden. UI copy says what
is observed, not a diagnosis. No “Restart host” button. On foreground return,
show Connecting, authenticate, refresh grant and workspace, then obtain fresh
prompt/turn data for the open screen. No live label survives just because a
previous socket was healthy. Foreground timing still requires Phase 0/1 device
measurement.

On background, obscure the app-switcher snapshot and clear prompts, transcripts,
answer selections and message drafts. Keep only host records and non-sensitive
view preferences as the plan allows. The system authentication overlay is not app background: the native lifecycle
bridge must distinguish its temporary inactivity from leaving the app so Face
ID does not erase the choice it is confirming. Within one foreground session, a
transient network loss may retain a draft in memory, clearly unsent; no offline
queue.
Copy near the composer: “Drafts are cleared when you leave the app.”

## 4. Screens

### S1 — Pair a host: scan or paste (F1)

**Priority:** short purpose → host setup instruction → scanner → paste
alternative → trust explanation. Copy: “Your projects, wherever you are.” /
“On your host, open Worktrees pairing. Then scan the QR code shown there.”
Do not invent a shipped desktop menu path: the host QR surface is prerequisite
S3 in the plan.

Scan QR code requests camera access only when tapped. The native scanner hands
the URI directly to the shim. Paste pairing link uses an explicit native paste
control/input; it never reads the clipboard on launch. Raw links and their
secrets stay out of React state, storage and logs. A link opened from outside
the app enters the same native parser. “Pair only with a host you trust. You’ll
compare a code on both screens.” No QR or secret is rendered in these drawings.

**States:** scanning / “Checking pairing link…” / connecting / parsed. No hosts
is the normal first-run state; no `read` capability is required to begin pairing.
Camera refusal: “Camera access is off. Allow it in Settings, or paste the
pairing link.” Actions Open Settings and Paste pairing link. Empty paste: “Paste
a pairing link from your host.” Invalid: “This pairing link isn’t valid. Copy a
new link from your host.” Preserve the parser's specific safe explanation for
unsupported address, unknown field, expired link or unsupported link-local
address; never echo the secret. Offline: “Couldn’t reach this host. Check your
connection and try again.” Cancel deletes the pending attempt; Retry revalidates
the link before dialing. Pairing failures are not capability failures.

### S2 — Compare codes (F1)

**Priority:** host name from the QR, explicitly not verified by its name →
six-digit code from the connection → comparison instruction → Codes match /
Cancel pairing. Say: “Check all six digits on Studio Mac. The host name alone
is not proof.” Group digits for sighted reading; VoiceOver reads each digit.

Codes match records the user's comparison and moves to S3; it does **not**
approve the device or send a new invented protocol command. If host approval
arrives early, retain the comparison screen until the user confirms. Persist
pairing only after host approval and local comparison; cancel after early
approval attempts self-revocation and otherwise instructs removal on the host.
A host denial or timeout invalidates this code. No locally generated comparison
code, no copy button, no automatic confirmation on a matching host name.

**States:** “Connecting to host…” until the authenticated exchange supplies the
code; a code is never replaced with an empty card. Interrupted/expired attempt:
“Pairing ended. Get a new code on Studio Mac.” Mismatch: Cancel pairing, plus
“Deny this request on Studio Mac.” Lost connection does not auto-resume approval
against a new attempt. No grant is available yet.

### S3 — Waiting for host approval (F1)

**Priority:** “Finish on your host” → same host and code → “On Studio Mac, choose
the projects this phone can see, then approve.” → Observe explanation → Cancel.
Do not offer permission toggles on the phone or a timer based on guessed expiry.
Use a real expiry countdown only if supplied by the pairing protocol.

**States:** waiting is the loading state; successful `{paired}` moves to S4;
“Pairing was declined on Studio Mac.” → Scan new code. Used-code refusal:
“Someone else used this pairing code. Tap Deny on Studio Mac.” Expired:
“This pairing code expired. Create a new one on Studio Mac.” Host capacity:
“This host has reached its device limit. Remove an unused device on the host.”
Other refusal: safe reason plus Try again with a new code. Background or network
loss cancels the pending UI flow and explains how to retry; never claim paired
without the host result. Empty and capability-denied states do not apply.

### S4 — Paired (F1, F2, F11)

**Priority:** “This iPhone is paired” → host → actual access and allowed projects
→ View projects → View device access. Default example: “Access: Observe”.
“To answer or send messages, change this device’s access on Studio Mac.”
The server's actual grant wins if different from the default.

**States:** pairing success and connection readiness are separate. “Paired.
Connecting to Studio Mac…” while opening the remote connection; a failure keeps
the pairing and offers Reconnect now. “No projects available to this device” if
the refreshed scope is empty. A capability refusal on workspace read points to
This device; it does not undo pairing. Confirmed revoked/expired enrollment
routes to the connection recovery state, never silently re-pairs.

### S5 — Hosts (F2)

**Priority:** saved host names → selected host connection status and transport →
Pair another host. Rows not currently connected say “Not connected”, not Offline
or a stale count of live agents. The sample has Studio Mac and Home server.

**Actions:** tap a host to select/connect and push S7; Pair another host begins
S1. Connection settings are available inside the selected host. No reorder,
rename, delete swipe or automatic concurrent host fan-out in the MVP.

**States:** loading local records; empty “No hosts paired yet. Pair a host to see
your sessions.”; unreadable record “Couldn’t load this pairing. Pair this host
again.” with explicit Forget confirmation; offline leaves records usable;
no capability needed for local host selection. An unselected host has no claim
about current reachability. Changing host with a draft asks Discard draft / Keep
editing before closing the old connection.

### S6 — Connection (F2, F11; supports all host features)

**Priority:** host name → Connected / Connecting / Reconnecting / Offline →
Tailscale or Local network → last successful refresh → This device → recovery.
The shim supplies non-secret display state; no token, fingerprint, raw pairing
link or editable address field. Connected means handshake complete; “Updating
activity…” can still follow it. Reconnect now is available while disconnected.

**States and copy:**

| Evidence | Copy and action |
|---|---|
| Initial attempt | “Connecting to Studio Mac…”; Back remains usable. |
| Tailscale route fails | “Make sure Studio Mac is awake and Tailscale is connected on both devices.” Retry; no assertion that a VPN is off without evidence. |
| LAN route fails, permission unknown | “Check that both devices are on the same network. Also check Worktrees in Settings → Privacy & Security → Local Network.” Retry / Open Settings. |
| Native layer positively identifies LAN denial | “Local Network access is off for Worktrees.” Open Settings; recheck on return. Do not infer denial from a generic Rust socket failure. |
| Address changed | Phase 4: “Looking for Studio Mac on this network…” for at most 30 seconds; then Scan address code. Only adopt the address after the pinned handshake; this is recovery of an existing host, not a new pairing. |
| Host identity does not match | “Couldn’t verify this host. Check it on Studio Mac.” Stop automatic attempts for that candidate; no Trust anyway. |
| Explicit revoke/expiry | “This device no longer has access to Studio Mac. Pair again on the host.” Pair again / Forget host. |
| Generic authentication failure | “Couldn’t authenticate this device. Check its access on Studio Mac.” Do not label a TLS failure “revoked” without evidence. |
| Version incompatibility | “Update Worktrees on Studio Mac to connect.” Show safe version information, no mobile update-host action. |

No host selected returns to Hosts. This device remains readable with last
confirmed grant marked as such while offline. Pair-again and address-code scan
have distinct labels and distinct native parsing paths. Never ask the user to
turn off certificate verification or expose a public listener.

### S7 — Projects and places (F3; F12 optional)

**Priority:** host/connection → Connection & device → project sections → place
rows → Refresh. No project browser beyond the granted workspace. Keep the host's
project and place order, including pin priority where provided.
Do not reorder rows under a finger when status changes. Each project includes a
text count such as “1 needs input · 4 places”; a separate waiting count still
shows attention when a working place dominates the rollup dot.

Row: place name → agent + status → last activity where available. Show busy >
waiting > done precedence and the desktop afterglow/unread derivation. Idle has
an empty dot slot, with “Idle” or “No active session” as appropriate. Unknown
provider activity is “Status unavailable”, not Idle. Activity and lifecycle are
separate facts. Tap a place → S8. No swipe close or inline answer.

**States:** skeleton rows; empty “No projects available to this device. Choose
projects for it on Studio Mac.” Project with no places: “No places yet. Create a
place on the host.” Read denied: “This device can’t read projects. Check its
access on Studio Mac.” Reconnecting shows last-known timestamps, not animated
live dots. Pull refresh failures keep a readable inline retry. Cold offline:
“Connect to load projects.” Out-of-scope projects are absent, never greyed out.

F12 is an optional collapsed “Usage” section below the projects, per-provider
used percentage and reset time from `claude_usage` / `codex_usage`. It has its own
Loading / “Usage unavailable” / Retry states and a stale timestamp offline.
No data: “No usage data for this provider.” Denied: omit values and show “Usage
isn’t available to this device.” Never block places or imply host-wide coverage
from one provider's meter. No separate usage tab or synthetic combined balance.

### S8 — Place detail (F3, F4, F5–F10)

**Priority:** host/project/place → waiting-prompt card if relevant → agent/model
and activity → provenance → brief/plan summary → message/recent-turn actions →
branch, last commit, dirty and ahead/behind facts → note/pin/lifecycle → session
actions. On a short screen the last sections scroll below the fold (frame 07).

“Started by Bret’s iPhone” is shown only from `launched_by`; absent attribution
says “Launch source unavailable”, never guesses “host”. Last activity uses a
real timestamp; tapping/VoiceOver gives an absolute date. A brief-only place
renders the returned summary, a plan shows title and goal, neither shows
“No plan or brief summary.” No raw file fallback; no diff links. Model absent:
“Model unavailable.” Plain text summary wraps; long summaries have Show more.

**Actions:** Review prompt → S9; Send message → S10; Recent turns → S11; Place
details → S13; Start session → S12 if stopped; Close session → confirmation if
running. Observe users can review/read but see an actionable access explanation
instead of enabled mutation buttons. Opening a finished place calls `mark_seen`
only with `write` and a fresh connection. Observe cannot clear the shared ring;
show “Finished · not acknowledged on host” and do not fake a successful ack.

**States:** independently load workspace summary, plan and prompt. A failed plan
must not blank the place. A waiting signal with unreadable/unsupported prompt
says “This agent needs input. Review it on the host.” No guessed choices. A stale
place reference gets the shared unavailable notice. A closed session still has
branch and note; “No active session” with Start session if allowed. Offline keeps
only same-foreground non-sensitive summary, with actions unavailable; prompt and
turn data cleared at background never reappear from disk.

### S9 — Prompt review, answer and step-up (F5)

**Priority:** host/place/agent → whole prompt, including command and consequence
text → exact supported choices → selected choice → Confirm answer. Plain text,
wrapping long command lines; a long prompt scrolls vertically without truncation.
It is a prompt view, not a terminal. Never collapse the risky part behind a
summary. No option is preselected on initial load (frame 08 shows a user-selected
option). Reading with Observe is allowed; answering requires `answer`.

1. Fetch `agent_prompt` when waiting is observed and on entering the sheet.
   `{waiting: false}` says “No prompt waiting. The agent may have continued.”
2. Keep the response's fingerprint with its options. A user selects one exact
   option, then taps Confirm answer. Show the selected words beside that action.
   An unrecognised prompt or option does not get a free-text answer fallback.
3. If required, obtain step-up through the native signing flow. iOS owns Face
   ID and any passcode fallback. The rationale names the target: “Confirm this
   answer for Studio Mac.” Do not use a web passcode field or a separate yes/no
   biometric check. Frame 09 is a schematic system surface, not OS artwork.
4. After step-up, submit `agent_answer` with the original fingerprint and chosen
   option, on that connection. The host re-captures and checks it. Prompt updates
   while Face ID is open invalidate selection; never transfer a choice to a new
   prompt. A valid proof may cover later answers/messages for five minutes on
   that connection; no per-action Face ID promise, no background grace period.
5. Success: “Answer sent.” Refresh the place and prompt, close the sheet, return
   focus to the prompt card or status. This confirms delivery, not command
   success. Optional success haptic, once.

**Race:** on `the prompt changed`, clear the choice and show “Your answer was
not sent. Someone else may have answered first. The agent’s prompt has changed.”
Check current prompt fetches anew; Back to place leaves safely. Do **not** say
someone definitely answered; the fingerprint cannot establish that fact. If the
fresh prompt is gone, show No prompt waiting. No automatic resubmit, no answer
lease or “approve all” control. Never create a bulk or standing-approval shortcut;
if a harness exposes a standing-approval choice, withhold that action in the MVP
and explain “Review this option on the host,” while preserving its text in the
full prompt. Serve must enforce the supported-choice policy too.

**Other states:** loading skeleton then prompt; read denied uses shared notice;
answer denied names `answer` in This device, not a Face ID remedy. Authentication
cancel: “Confirmation cancelled. Your answer was not sent.” Try again / Cancel,
keeping the selection only if the prompt remains current in this foreground
session. System authentication failure: “Couldn’t confirm your identity. Try
again.” System may offer passcode. Expired challenge: “Confirmation expired.
Confirm again.” An expired proof restarts authentication before retrying the
still-current action, never silently answers. Connection change during
confirmation: “Connection changed. Review the prompt again.” A lost reply after
submission says “We couldn’t confirm whether your answer was sent.” Re-read the
prompt, but do not infer delivery solely from its disappearance.

### S10 — Send message (F6)

**Priority:** host/place and agent/model → “This message can direct work on
Studio Mac.” → labelled Message field → validation → Send message. A one-line
message can wrap visually into a growing field. Return does not submit; no
attachments, command toolbar or code execution buttons. Disable autocorrection
for instruction text while keeping normal keyboard accessibility.

Empty: “Write a message for Claude.” Disable Send message. Validate ≤4096 UTF-8
bytes, no newline or control characters. Show the byte count and a plain error:
“Use one line, with no control characters.” / “Message is too long. Use 4096
bytes or fewer.” Do not silently change a pasted instruction to make it valid.
The keyboard in frame 11 is only an area marker; its real height is dynamic.

Tap Send message → step-up if needed (same connection proof as S9) → `agent_send`.
After success: “Message sent.” If a future response explicitly reports queueing,
say “Message queued for the agent”; do not invent that status from the agent
being busy. Clear the draft only on confirmed acceptance. No optimistic user turn
is added to Recent turns. Pending draft disclosure: “Drafts are cleared when you
leave the app.” Back/Cancel with unsent text asks Discard draft / Keep editing.

**States:** waiting prompt refusal: “The agent needs an answer first.” Keep draft
in memory, return to detail with Review prompt; no stacked sheets. Busy composer:
“The agent’s input already contains text. Clear it on the host, then try again.”
Do not overwrite it. No live session: “Start a session before sending a message.”
Capability refused: “This device can’t send messages. Change its access on Studio
Mac.” Unsupported row/harness: “Messaging isn’t available for this session.”
Authentication cancellation retains draft and says “Message not sent.” Offline
before sending: “Reconnect before sending.” Lost reply: “We couldn’t confirm
whether your message was sent. Check recent turns before sending it again.”
Even after checking, absence from recent turns is not proof of non-delivery.

### S11 — Recent turns (F10)

**Priority:** host/place → “Read-only · latest 10 exchanges” → chronological
user/assistant turns → refresh. Open at the newest exchange without auto-scrolling
away from someone reading older text. A New turns button moves to fresh content.
Each turn has role, timestamp if supplied, text and tool-name summary only.
Truncation is explicit: “Turn shortened by host.” Do not offer Show full unless
the endpoint can return it. It is a bounded recent history, not infinite scroll.

**States:** “Loading recent turns…”; empty “No recent turns for this session.”;
provider unsupported “Recent turns aren’t available for this agent yet.”;
read refusal uses shared notice; error Retry. During a same-foreground disconnect,
label visible turns “Last loaded at 9:41”; after background/relaunch show a blank
loading/offline state until refetched. No local transcript storage. No cursor,
prompt `$`, input field, tool output expansion, file links or terminal escape
sequences. Render transcript text as text, not executable HTML or an interactive
command. Back returns to detail; sending remains a separate deliberate action.

### S12 — Start / close a session (F7, F8)

Start sheet: context → existing place/branch → Agent picker → Model picker from
`agent_models` → Start session. Use provider labels from the supported registry;
do not assume every place is Claude. Pickers are single-choice subviews within
the sheet with Back; no nested sheet. Only offer supported provider/model pairs.
The example “Sonnet” is illustrative, not a fixed model identifier.

Loading model choices disables start. Empty: “No agent models available. Configure
an agent on Studio Mac.” Failed lookup: Retry. `write` required; no Face ID under
the current plan. “Starting…” → refresh, then show the observed session. If already
running: “This place already has a session.” No force or fresh-session override.
Four-lane refusal: “This device has 4 live sessions. Close one before starting
another.” Ten/hour refusal: “This device has reached 10 starts in an hour. Try
again later.” Use a retry time only if the server provides one. These counts are
server limits, not guessed counters on the phone. Other refusals preserve their
safe reason. Offline disables start; unknown delivery requires refresh before a
new start attempt.

Close confirmation: “Close the session in ios-ui-design?” / “The agent will stop.
The place and its files stay on Studio Mac.” Actions Keep session / Close session.
For busy or waiting activity add “This agent is still working” or “This agent is
waiting for input.” Closing requires `write`, no Face ID in this MVP. Success
returns to detail with No active session. Already closed is idempotent UI success
after refresh; denied/error keeps the running state until refreshed. Unknown
result stays unknown until the workspace confirms. Close never removes a worktree.

### S13 — Place details: note, pin, lifecycle (F9)

Sheet: labelled Note field → Pinned switch → Lifecycle picker → Save changes.
Offer Automatic (clear the declared label), Closed, Saved, Archived and
Abandoned, matching `store::LIFECYCLE_LABELS` and `ops::set_lifecycle`. Active
and Idle are derived status, never values sent to this row. Existing note loads before editing; absent note: “Add a note for this
place.” Include “These details are shared with the host. Lifecycle does not close
the session.” A saved note's provenance appears only if returned by the host.

All three mutations require `write`. Observe sees read-only values plus “This
device can’t edit place details. Change its access on Studio Mac.” Busy state:
“Saving…”; validate using the same limits as the underlying rows. On failure
retain the in-memory edit and mark the specific field that failed. The three
rows are not a transaction: if note saved and pin failed, say exactly that and
retry only the failed changes. Do not claim all saved or roll back another
client's changes. On uncertain response reload before offering retry. Cancel
with unsaved changes asks Discard changes / Keep editing. Offline cannot save;
background clears the draft. No optimistic lifecycle move that conceals a failed
write. The plan has no edit revision/CAS for notes: last accepted write wins;
a stronger concurrent-edit warning would need a server contract.

### S14 — This device, revoke and forget (F11)

**Priority:** device/host identity → effective access label → plain-language
abilities and project scope → authentication policy → expiry → Revoke / Forget.
Show grant intersection, not just the device's configured grant. Optional
technical disclosure lists `read`, `write`, `answer`, `instruct`; `stream` and
`admin` are absent from the phone grant. Never show toggles that imply this screen
can grant access.

Observe copy: “You can read activity. Actions are enabled on Studio Mac.”
Interactive copy: “Can run commands on Studio Mac as you.” Then explain the actual
allowed abilities individually; do not call a partial grant Interactive with
abilities it lacks. “Face ID or passcode protects answers and messages” describes
the policy, not per-tap authentication. Expiry uses actual supplied policy;
default example “Disabled after 30 days unused.” Unknown policy says “Check expiry
on Studio Mac”; do not calculate a deadline from last app launch.

Revoke confirmation: “Revoke this device on Studio Mac?” / “This iPhone will lose
access to this host. You’ll need to pair again.” Actions Keep access / Revoke
access. `device_revoke` acts on this device only and needs no elevated capability
or Face ID. Await confirmed success before deleting the local host record and
returning to Hosts; do not require `read` to offer own-device revocation. A lost
reply says “Revocation couldn’t be confirmed. Check this device on Studio Mac.”
A subsequent TLS refusal alone does not prove revocation.

Forget confirmation: “Forget Studio Mac?” / “This removes this host’s pairing
from this iPhone. It does not revoke the device on Studio Mac. Remove it there
to revoke access.” Actions Keep host / Forget host. This is a local action,
available offline; delete the host-specific stored credentials and view state,
clear in-memory data and disconnect. It never claims the host was contacted.

**States:** first-frame loading; last-confirmed access badge offline; no host
returns to Hosts; unsupported revoke row gets host-side instructions. Offline
revoke is unavailable with “Connect to revoke access, or remove this device on
Studio Mac.” Forget remains available. Device access cannot be widened here even
if a stale grant claims otherwise. Explicitly revoked enrollment shows Pair
again; generic handshake errors remain the S6 uncertainty state.

## 5. Component contract

| Component | Anatomy | States and interaction |
|---|---|---|
| Status dot | 8-point fill, fixed slot, adjacent word | Green `--ok` working; static amber `--warn` needs input; purple `--done` finished with unread ring; empty slot idle. Unknown has a word. Busy > waiting > done; use existing afterglow helpers. Motion stops with Reduce Motion or loss of freshness. Dot is decorative when the row supplies the accessible name. |
| Place row | Name, agent/status, age, disclosure | Entire row ≥72 points at default type; grows with content. One button/link target, no nested tap target. Selected/focus uses text-token outline; loading is inert; last-known labels replace live claims offline. Pin is an additional label, not a new status colour. |
| Prompt card | Agent, needs-input label, readable excerpt, Review prompt | Excerpt may shorten on detail only, never inside review. Loading, gone, unreadable and denied have distinct words. Amber dot does not become an amber text heading. Review remains available to Observe. |
| Prompt choice | Exact host option label, selection indicator | Full-width ≥50-point radio row, no initial selection; one choice at a time. A separate Confirm answer commits it. Changed fingerprint clears selection. Unsupported standing approval is visible as text with a host instruction, not an enabled control. |
| Composer | Context, labelled field, limit/help/error, submit | Empty, editing, invalid, authenticating, submitting, accepted, refused, unknown delivery. Keep input readable when submit is unavailable; show the reason outside a disabled button. |
| Connection banner | State symbol, host, state, last refresh, retry | Connecting / reconnecting / offline / checking fresh data. Persistent below nav, pushes content rather than covering it. On confirmation sheets it appears above choices. Connected success collapses after a polite announcement. No forced scroll jump. |
| Waiting banner | Amber dot, “1 place needs input”, Review | Foreground only; tapping navigates to the existing place detail, never auto-opens Face ID. Multiple places lead to the project list. Coalesce repeated events, no duplicate haptic every poll. |
| Capability notice | Action-specific refusal, host instruction, View device access | Use text and optional neutral lock shape. Readable offline as last-confirmed policy. Never hide the reason in a tooltip or imply buying access. Object-scope errors use generic unavailable wording instead. |
| Confirmation | Explicit target/action, consequence, Keep/Cancel, commit | Used for close, revoke, forget, draft discard. Default focus on title or safe choice; destructive intent is in words, not red alone. Waiting/unknown completion disables duplicate commit. |

## 6. Type, themes and accessibility

Use plain CSS and `app/src/tokens.css`, importing rather than copying palettes.
Wireframes resolve **Catppuccin Latte** for light and **Catppuccin Mocha** for
dark. The proposed initial appearance follows iOS light/dark using this existing
pair. Other existing themes remain compatible with the role mapping below;
a separate mobile theme picker is deferred. No new brand palette or translucent
material is needed to communicate this MVP.

Use `--font-ui` for interface and `--font-mono` for codes/commands. Keep the
weight, spacing, radius and line-height tokens. Phone sizing is a scoped role
adaptation: default `--ui-rem: 17px`, body/row/label 1rem, meta 0.875rem, heading
1.25rem, title 1.647rem; these override the corresponding existing `--fs-*`
tokens in the future mobile entry only. Larger text is not the desktop zoom
slider. Bridge the preferred iOS content size into this one scale, update while
the app is running, and verify WKWebView actually responds on hardware. The
specific bridge is still implementation work, not measured by these SVGs.

At accessibility sizes, headers, paired actions and label/value rows stack;
place names, error text and options wrap without line clamps. Sheets expand and
scroll; bottom actions become part of scroll flow if they would leave too little
reading space. Never shrink type to fit 390 points. Keep zoom enabled, support
Bold Text and test the largest accessibility size, landscape and a narrow phone.
These choices follow Apple's [Dynamic Type guidance](https://developer.apple.com/videos/play/wwdc2024/10074/).

### Contrast measured from this branch's tokens

Ratios below use opaque sRGB token pairs, WCAG relative luminance, rounded to two
decimals. No composited tint is assumed. Normal readable text targets ≥4.5:1;
focus/essential graphical indicators ≥3:1. Decorative status hue always has a
text equivalent. The repository explicitly warns against accent-as-text.

| Theme | `txt-hi` / panel | `txt-dim` / panel | `txt-dim` / tree | `accent-ink` / accent |
|---|---:|---:|---:|---:|
| Tokyo Night | 14.31 | 5.70 | 6.00 | 7.58 |
| Tokyo Day | 7.03 | 3.57 | 3.15 | 4.75 |
| Catppuccin Mocha | 11.34 | 7.37 | 7.89 | 8.91 |
| Catppuccin Latte | 7.06 | 4.37 | 4.06 | 4.91 |
| Nord | 8.87 | 3.80 | 4.64 | 6.24 |
| Gruvbox Dark | 11.57 | 6.03 | 6.77 | 6.49 |

Use `--txt-hi` for names and status words. Use `--txt-dim` for metadata only on
surfaces where it passes; promote to existing `--txt` or `--txt-hi` otherwise.
The light wireframes use `--txt` for metadata (including timestamps). Nord panel
metadata also needs promotion; Tokyo Day tree metadata should use `--txt-hi`.
No global token change is part of this proposal. `--txt-mute` is for disabled or
decorative content, never necessary instructions, placeholders that replace a
label, or errors. Accent/status/danger colours are fills, dots or strips; text
stays in text tokens. Buttons pair `--accent` with `--accent-ink`. Thin `--line`
dividers are decorative; form/selection/focus outlines use a contrast-checked
text token. Recheck actual composite colours if implementation introduces tints.

### VoiceOver, input and feedback

- Row example: “ios-ui-design. Claude needs input. Worktrees on Studio Mac.
  Button.” Hint: “Opens place details.” Other status names: “Codex working”,
  “Agent finished, not seen yet”, “Idle”, “No active session”, “Status unavailable”.
  A standalone dot uses these labels; a dot inside an already labelled row is
  hidden from the accessibility tree.
- Use real headings, lists, buttons, radios and labelled inputs. Prompt review
  is an accessible dialog with focus moved to its heading. Return focus to the
  invoking card on dismissal. Background content is inert, the dialog is not.
- Connection transitions use a polite live region; submission refusals are
  announced once. Do not announce every 3-second poll or continually update
  relative ages in a live region. Preserve reading position during refresh.
- Comparison code reads “4, 8, 2, 1, 9, 3”; visible option text is the accessible
  choice label. Announce selected and disabled states with their reason.
- Support Voice Control, Switch Control and external-keyboard focus. Pull to
  refresh and swipe-to-dismiss have explicit button alternatives. Never make
  swipe the only route to an action.
- Optional haptics: one light cue for a newly waiting place while foregrounded;
  one success cue on confirmed answer/message delivery; one warning on a stale
  prompt refusal. Respect system preferences; words supply the same information.
  No haptic on every retry or polling event.

Target size, non-colour status and multimodal feedback follow Apple's
[accessibility guidance](https://developer.apple.com/design/human-interface-guidelines/accessibility)
and [44-point control guidance](https://developer.apple.com/design/tips/).
The SVGs are static review artifacts; they cannot prove VoiceOver, Dynamic Type,
keyboard avoidance, haptics or native Face ID behaviour.

## 7. Feature and delivery coverage

| Feature | Screen(s) | Delivery boundary |
|---|---|---|
| F1 Pairing | S1–S4 | Phase 1; native key/QR flow and host QR prerequisite |
| F2 Hosts | S4–S7 | Phase 1 connection, Phase 2 list |
| F3 Places/status/ack | S7–S8 | Phase 2; ack requires `write`, introduced with interaction |
| F4 Detail | S8 | Phase 2; provenance depends on #551 data |
| F5 Answer | S8–S9 | Phase 3; new prompt/answer rows and per-harness parser |
| F6 Message | S10 | Phase 3; new send row and step-up |
| F7 Start | S12 | Phase 3; provenance and server launch limits |
| F8 Close | S12 | Phase 3; `write`, explicit confirmation |
| F9 Note/pin/lifecycle | S13 | Phase 3; existing write rows |
| F10 Recent turns | S11 | New turns row; Phase 2 if ready, otherwise Phase 3 |
| F11 Self-service | S6, S14 | Phase 1; own-device revoke, local forget |
| F12 Usage | S7 optional section | Nice to have; never delays core flows |

Phase 0 must settle native authentication cancellation, connection behaviour and
foreground timing before the UI's timings and OS error handling are final.
Phase 4 adds validated address recovery and device polish. These are proposed
screen contracts, not fabricated server response schemas. In particular,
exact model IDs, prompt options, lifecycle values, policy expiry and refusal
payloads must come from the actual rows; unknown command and capability denied
remain different states.

## 8. Deliberately absent

- Terminal, shell keyboard, streamed tool output, file tree, diffs and docs
  viewer: outside the requested phone scope. `stream` is never granted. File
  reads being technically permitted by `read` does not add a file UI.
- Host administration, install/update, grant widening, pairing another device,
  remote server startup and force-close/force-start: the phone acts within its
  own grant on existing places.
- New worktrees, branch switching, merging or release controls: not F7's existing
  place session launch, and not among the MVP features.
- Bulk answers, standing-approval shortcuts and free-text prompt answers:
  the MVP confirms a recognised, current choice only.
- Persistent transcripts, prompt caches, background send queues and automatic
  retries of uncertain mutations: foreground reads and explicit acts only.
- Background alerts or a notifications permission prompt: no APNs sender/relay
  in the MVP. Waiting is visible while the app is open. The plan's app-icon
  badge-on-return is conditional on platform permission/support; omit it unless
  verified without adding a notification flow. Never promise a background alert.
- Multi-host live aggregation, search across hidden projects, theme/density
  customization and a separate iPad design: defer until the single-host phone
  workflow is proven. Larger windows may use the same stack with constrained
  content width; no second navigation model is promised.

## 9. Review and implementation acceptance

Artifact review: XML-parse every SVG, confirm 390 × 844 viewBox, shapes/text only,
no embedded/external resources, check rendered text bounds and inspect the light
and dark sheets. Verify local Markdown links and token ratios. This change is
docs/SVG only; it does not justify running the application build gates.

Before calling the **implementation** done, drive on a physical iPhone:
Observe-only pairing and scope; all four status meanings beside the desktop;
first answer requiring Face ID and a second on the same proof; cancellation;
another client answering first; disconnect during send with no duplicate;
background/foreground purge and refresh; generic TLS failure versus confirmed
revocation; LAN denial recovery; busy composer refusal; start limits and close;
partial note/pin save; largest Dynamic Type, VoiceOver and light/dark appearance.
Use real parser fixtures and unsupported-provider cases. Record device and iOS
version; no static drawing counts as these measurements.

Three design decisions for user review:

1. **Keep free-text Send message in the first interactive release?** Recommended
   yes, with the explicit command-capable access label and existing step-up.
2. **Use select then Confirm answer for every prompt?** Recommended yes; Face ID
   may already be valid, and accidental taps should still require confirmation.
3. **Start with the existing Latte/Mocha theme pair following iOS appearance?**
   Recommended yes for these phone screens, with text-role promotion where
   contrast needs it. Tokyo Day/Night remains a compatible alternative.
