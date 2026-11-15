---
title: How a conversation starts, and how a guest is invited (group chat start design)
---

# How a conversation starts, and how a guest is invited

*Version 1, 2026-11-14. Status: decided (A) and built; §7 is the build. The
design's mocks were throwaway harness routes, removed by the build. Code claims
are against `2b4624f` (main after #669). Each claim is marked **read** (in the
code), **measured** (run, with where), **assumed**, **contract** (this design
requires it and nothing provides it yet) or **hypothesis** (only use settles it).
Where this document and the code disagree, the code wins. It sits beside
[v3.1, shared sessions](group-chat.md) (what shipped), [v4.1](group-chat-v4-two-engineers.md)
(a conversation per topic; §12 decides where a conversation *lives*) and
[v2](group-chat-v2-handoffs.md) (the authority rules). This one is only about
how a conversation *starts*: the first run of each person, the invite, the
share.*

**The ask, in David's words (2026-11-14):** "Where in the settings is the share
options? Can we review how we initiate the chat and invites? Maybe a design
decision lane?"

---

## 0. The recommendation, on one page

**Start from the session (alternative A).** *Share…* on a lane opens one dialog
that does, in order, the three things a first share needs today on three
different screens: **who** (a person already paired, or *Someone new*), **pair**
(the invite window, with remote access turned on inline if it is off, the link,
the comparison code and Allow), and **share** (v3's warning, then Share). The
guest gets *Join with a link…* on Home, out of "This window drives". Group chat's
beta switch moves to the Beginner tier, with Ideas beside it. Settings → Remote
→ Guests stays, as the place to manage people afterwards.

**Measured** in the design harness and on the two test Macs, from "app
installed, nothing set" to "Bret reads David's session" (acts = clicks plus
fields filled; §2 says how each was counted):

| | Owner acts | Guest acts | Settings pages (owner / guest) | Needs Advanced mode | Words to know before starting (owner / guest) |
|---|---|---|---|---|---|
| **Today** | **18** | **9** | 2 / 2 | yes / yes | 9 / 6 |
| **A. Share from the session** | **11** | **7** | 1 / 1 | no / no | 2 / 1 |
| B. A People page in Settings | 12 | 8 | 2 / 2 | no / no | 3 / 2 |
| C. Conversation first | 15 | 9 | 1 / 1 | no / no | 3 / 2 |

**Why A and not the others:**

- **It removes the order nobody is told about.** Today the owner must invite
  before sharing, in Settings, with remote access already on, and the share
  dialog's only answer to "no guests" is a sentence pointing at a section the
  Beginner tier hides (§1.2, F3). A puts the invite where the share is.
- **It costs the least.** Frontend only: no new host row, no new data. The
  dialog reuses `PairSheet`'s window and pending bodies, `remote_candidates`
  and `remote_enable`, `pair_begin` with `invite`, and `share_create` (§3.1).
- **It does not prejudge v4.1 §12.** C is the shape of a conversation per topic,
  and v4.1 deliberately leaves where a conversation lives to the experiments (D13).
  A's *Someone new* step is the same component C's *Add someone new* would use,
  so nothing in A is thrown away if §12 picks C.
- **Every security word stays.** v3's warning, the comparison code, one pairing
  window at a time, Allow as the pairing consent and Share as the reading
  consent, two separate presses on two separate screens (§4).

**What David decides** (my pick in the Answer column):

| # | Decision | Answer | Where |
|---|---|---|---|
| S1 | How a first share starts | **A**: *Share…* on the lane (the bar under the agent's name, the header's ⋯ menu, the place's context menu) opens Who → Pair → Share. | §3.1 |
| S2 | The beta switch's tier | **Beginner** for the Beta features section, Ideas included. A person invited into a beta is not an advanced user by being invited. | §3.1, §5 |
| S3 | Guests' section tier, once the beta is on | **Beginner**, same as the switch: it is where people are removed. | §5 |
| S4 | Where the guest pastes the link | **Join with a link…** on Home (beta on) and as its own section on Remote, out of "This window drives". | §3.1 |
| S5 | A guest pairing while the beta is off | **Refuse in words** ("Turn on Group chat in Settings → General → Beta features first"), rather than pair a host whose shares nothing will show. The other choice, turning the beta on by pairing, puts a second writer on a switch the person owns. | F6 |
| S6 | Remote access turned on from the invite | **Yes, inline**, the one address preselected (an overlay first, else the LAN), in words, with "change it in Settings → Remote" beside it. Never silently. | §4 |
| S7 | A `worktrees-pair:` URL scheme, so the link opens the app | **Not now.** Pasting costs one act; a clickable link in a chat app is a new surface to judge on its own (§6). | §6 |
| S8 | The People page (B) | **Not now.** Rename Remote → Guests to *People* later if a second guest makes the list worth a page. | §3.2 |
| S9 | Conversation first (C) | **Left to v4.1 §12.** It is V4/V6 there; this lane's mock is its first draft. | §3.3 |

---

## 1. Today's first runs

### 1.1 How it was measured

- **Harness, measured:** `pnpm dev:mock` on `2b4624f`, Playwright 1.63 in
  Chromium and WebKit, a fresh tab (Beginner, beta off, remote access off, no
  guests: `?guests=0`). The script pressed each control and counted; it stopped
  at the copied link, because the mock has no second machine.
- **Test Macs, measured:** the owner VM (`macOS`) and the guest VM
  (`macOS-guest`), main's build, already paired as David and Bret by the
  vmlab lane, read screen by screen without pairing again: an invite window
  opened and cancelled, *Share with…* opened and cancelled, *Ask* opened and
  cancelled. Screenshots `today-*` below.
- **Read:** the steps between the two (Allow, Done, the lane's share bar, the
  join ask) from `PairSheet.tsx`, `LaneShare.tsx`, `GuestNav.tsx`.

### 1.2 The owner, David: 18 acts, 2 pages, Advanced mode

| # | Act | Screen | Source |
|---|---|---|---|
| 1 | Settings (gear) | General, Beginner: **no Beta features** | measured |
| 2 | Mode → Advanced | General shows Beta features | measured |
| 3 | Tick Group chat | | measured |
| 4 | Remote page | "This window drives", then "Pair with a host" (the guest's control) first | measured |
| 5–7 | Remote access: *Turn on…*, pick an address, *Turn on* | address picker: "overlay (Tailscale or a VPN tunnel)", "home or office network" | measured |
| 8 | *Invite a guest* (Guests, Advanced) | name sheet | measured |
| 9 | Type "Bret" | | measured |
| 10 | *Show the pairing code* | QR, link, 2:00, "Scan this with the device" | measured |
| 11 | *Copy* | send the link outside the app; **Settings must stay open** | measured |
| 12 | *Allow*, after comparing the code | pending | read |
| 13 | *Done* | | read |
| 14 | Close Settings | | read |
| 15 | Open the lane | | read |
| 16 | *Share with…* | the guest list | measured (VM) |
| 17 | *Share* (one guest is preselected) | warning | read |
| 18 | *Allow* Bret's join | the lane bar or the corner | read |

![Settings → General on the owner's Mac in Beginner: Beta features shows only because it is already on](img/group-chat-start/today-owner-1-general-beginner.png)
![Settings → Remote in Beginner: the page leads with the guest's "Pair with a host"](img/group-chat-start/today-owner-2-remote-beginner.png)
![Searching "invite": Guests is an Advanced section](img/group-chat-start/today-owner-3-guests-is-advanced.png)
![Guests: "bret" and "Bret", four "bret-mac" devices](img/group-chat-start/today-owner-4-guests.png)
![The invite window](img/group-chat-start/today-owner-5-invite-window.png)
![Share with… offers bret, ana and Bret](img/group-chat-start/today-owner-6-share-dialog.png)
![The lane's ⋯ menu has no Share](img/group-chat-start/today-owner-7-header-menu.png)

### 1.3 The guest, Bret: 9 acts, 2 pages, Advanced mode

| # | Act | Screen | Source |
|---|---|---|---|
| 1 | Settings | | measured |
| 2 | Remote page | "This window drives" → "Pair with a host" | measured |
| 3 | Paste the link | the hint says the host opens "Settings → Remote → Pair a device" | measured |
| 4 | *Pair* (the name is the Mac's, prefilled) | the six digits; read them to David | measured |
| 5 | General | Beginner: no Beta features | measured |
| 6 | Mode → Advanced | | measured |
| 7 | Tick Group chat | | measured |
| 8 | Close Settings | the sidebar shows *David's places* | measured (VM) |
| 9 | Open the shared row | "waiting for David", then the session | read |

Bret can do 5–7 before 1–4; the count is the same.

![Settings → Remote on the guest's Mac: David's Mac is listed as macos.shared, a guest host](img/group-chat-start/today-guest-1-remote.png)
![The guest's sidebar: David's places under Bret's own worktrees project, with Ask](img/group-chat-start/today-guest-2-sidebar-ask.png)

### 1.4 What the walk found

| # | Finding | Source |
|---|---|---|
| F1 | **Every owner control is Advanced.** `beta`, `remote-access`, `devices` and `guests` are tier `advanced` (`settingsMap.ts:211`, `:284`, `:285`, `:288`). A fresh install is Beginner (`settings.ts:640`), and the guest's switch is the same `beta` section. Search finds them (it ignores the mode, `settingsSearch.ts:21`), tagged "Advanced". | read, measured |
| F2 | **Turning the beta on keeps the switch, not the section it unlocks.** Rule 2 draws a changed section in any mode ("shown because it is not at its default", measured on the owner VM), so Beta features stays; Guests holds no changed key and stays hidden in Beginner. | read, measured (VM) |
| F3 | **The share dialog's empty state is a dead end.** "No guests yet. Invite one in Settings → Remote → Guests." (`LaneShare.tsx:155`): no button, and in Beginner that section is not drawn. Also shown when every guest's devices are revoked (`guestLive`). | read, measured (harness) |
| F4 | **The invite has to come first, and Settings has to stay open.** Closing the sheet cancels the pairing window (`PairSheet.tsx:88`, unmount → `pair_cancel`), which is right (an open window nobody watches is one a stranger can use) and means the owner waits in Settings, not in the lane. | read |
| F5 | **The guest's field is in the wrong place and points at the wrong button.** "Pair with a host" sits under "This window drives", whose words are about driving another engine; its hint says the host opens "Settings → Remote → Pair a device" (`HostsPanel.tsx`), while a guest's link comes from *Invite a guest*. The doc Bret was given says "Join as a guest", which exists nowhere. | read, measured (VM) |
| F6 | **A guest can pair with the beta off and see nothing.** The pair succeeds and says "What it shares with you shows in the sidebar" (`HostsPanel.tsx`), but the guest connection starts only with the beta on (`main.tsx:201`, **read**). | read |
| F7 | **Re-inviting makes a new person.** The owner VM lists guests "bret" (Nov 12) and "Bret" (Nov 14), the second with four devices all named "bret-mac", one never seen. The share dialog then offers "bret, ana, Bret" with nothing to tell them apart. | measured (VM) |
| F8 | **A pairing toast names a guest's device by its id.** "d_6udj54awhjorcv6pfa6emcuoqe was paired with this computer." `alertText` names a device through `labelOf` (`remoteStore.ts:76`), which looks only in `devices_list`; the mock's answers the owner's devices only. | measured (VM); cause **assumed** for serve |
| F9 | **The invite window speaks to a phone.** "Scan this with the device, or paste the link on a device with no camera: `worktrees remote pair '<link>'`" (`PairSheet.tsx`): a guest is a person on a Mac, and a CLI line is not their path. | read, measured |
| F10 | **There is no Share in the lane's ⋯ menu** or the place's context menu; only the bar under the agent's name, which appears only with the beta on and a Claude or Codex lane. | measured (VM), read |
| F11 | **The guest sees the host by its machine name** ("macos.shared") in Remote, and by the owner's name ("David's places") in the sidebar. | measured (VM) |
| F12 | **Words the owner must already know:** beta feature, Advanced mode, remote access, an address and its interface (LAN or overlay), guest, pairing window, comparison code, share, lane. The guest: Remote, "This window drives", host, pair, beta, Advanced. | measured (walk) |

F7 and F8 are defects of the shipped beta, not of the start design; they are
listed for the rehearsal's defects lane.

---

## 2. How the alternatives were counted

Each alternative is a click-through mock in the design harness
(`app/src/mock/startDesign.tsx`): `?design=start-a`, `start-b` or `start-c`,
with `&as=guest` for the guest's side. A Playwright script presses every control
by its `data-act` name and counts (Chromium and WebKit agree, every flow). The
mocks start where the change starts, so the table adds what they leave out the
same way for each: the beta switch (gear, tick, close: 3 acts) where the mock
starts outside Settings, and the gear (1) where it starts inside. Sending the
link and reading the code aloud happen outside the app and are counted for
nobody. All the counts in §0 are therefore **measured on a mock**: what a
person would press if the build matched it, not what anyone has pressed.

---

## 3. The alternatives

### 3.1 A. Share from the session (recommended)

**Owner, 11 acts** (8 in the dialog plus the beta's 3): *Share…* → **Who**:
nobody is paired yet, so *Someone new* with a name field → *Invite Bret* →
**Pair**: remote access is off, so one block says Bret's Mac must reach this
one, with the address preselected → *Turn on and show the link* → the window,
the link and *Copy* → the comparison code → *Allow* → **Share**: "Bret's MacBook
is paired as Bret, a guest", Bret ticked, v3's warning → *Share with Bret* →
Bret's join ask in the corner → *Allow*.

With Bret already paired the dialog is today's: Who lists the people, Share
shows the warning, 2 acts.

![A: the lane's ⋯ menu gains "Share this session…"](img/group-chat-start/a-owner-1-menu.png)
![A, step 1: who, with Someone new](img/group-chat-start/a-owner-2-who.png)
![A, step 2: remote access turned on inline, one address preselected](img/group-chat-start/a-owner-3-reach.png)
![A, step 2: the invite window](img/group-chat-start/a-owner-4-window.png)
![A, step 2: the comparison code and Allow](img/group-chat-start/a-owner-5-code.png)
![A, step 3: the warning and Share, a separate press](img/group-chat-start/a-owner-6-share.png)
![A: Bret asks to join](img/group-chat-start/a-owner-7-join-ask.png)

**Guest, 7 acts** (4 plus the beta's 3): Home → *Join with a link…* → paste →
*Join* → read the code to David → when David allows, the share he made in the
same dialog is already there: a toast, "David shared gc-lane with you" →
*Open* → waiting for David → reading.

![A, guest: Join with a link](img/group-chat-start/a-guest-1-join.png)
![A, guest: the code to read to David](img/group-chat-start/a-guest-2-code.png)
![A, guest: the share arrives as the pairing completes](img/group-chat-start/a-guest-3-toast.png)

**What it costs, read against the code:**

- `LaneShare.tsx`'s `ShareDialog` becomes three steps. The pair step is
  `PairSheet`'s window, pending and outcome bodies lifted into a component both
  use, so the 120 s window, one attempt, *Cancel pairing* and "closing cancels"
  stay one implementation. `pair_begin { invite: { name } }` and `pair_decide`
  exist (**read**); `pair_decide` answers the `DeviceRecord`, whose `principal`
  is the new guest (`devices.ts`, **read**), and that preselects the Share step.
- The reach block calls `remote_candidates` and `remote_enable`, the rows
  `RemoteAccessPanel` uses (**read**); the choice of default is the panel's
  first overlay candidate, else its first LAN one (**contract**: today the
  panel asks; nothing picks).
- *Share this session…* in the header's ⋯ menu and the place's context menu,
  both gated by `groupChatBeta` and the harness rule `LaneShareBar` already
  applies (Claude or Codex, live).
- Home's *Join with a link…* and a Remote section of its own for the guest:
  `HostsPanel`'s pair half, moved and reworded, the same `remote_pair` call.
- No serve change: every row above exists and keeps its audit record.

**What it assumes about the guest's first run:** that the guest has the app and
turns the beta on (S2 makes that a Beginner tick); that the link reaches them
within the window, which the owner now watches from the lane, not from Settings.

### 3.2 B. A People page in Settings

A Settings page of its own, *People*, listed when the beta is on and drawn in
every mode. It holds your name, the people who can read what you share (each
with *Share a session…* and *Remove*), what you share (with *Stop sharing…*),
the place-name switches, and, for a guest, the computers you are a guest of with
*Join with a link…*. Remote keeps the listener and the devices.

**Owner, 12 acts:** gear, tick Group chat, *People*, *Invite someone…*, name,
*Turn on and show the link*, *Copy*, *Allow*, *Share a session…*, pick the
lane, *Share*, *Allow* the join. **Guest, 8 acts:** gear, tick, *People*,
*Join with a link…*, paste, *Join*, close, open the row.

![B: the beta switch in Beginner, People appears](img/group-chat-start/b-owner-1-beta-beginner.png)
![B: People, Bret paired, nothing shared](img/group-chat-start/b-owner-2-people.png)
![B: Share a session with Bret: pick the lane](img/group-chat-start/b-owner-3-pick.png)
![B, guest: People with Join with a link](img/group-chat-start/b-guest-1-people.png)

**Costs:** a new `PageId` and group in `settingsMap.ts`, the `guests` section
moved there, a new `people-share` section reading `shares_all`, and a lane
picker over the workspace snapshot, which `settingsmap-check.mjs` and
`beta-check.mjs` both hold (**read**). **What it does well:** one place to see
who can read what, which today needs every lane's bar. **Why not first:** the
share still starts in Settings, away from the session it shares, and a person
who does not know the page exists starts from the lane and meets F3 anyway.

### 3.3 C. Conversation first

A Conversations button in the rail; *New conversation* takes a topic, people
(*Someone new* is A's pair step) and, optionally, running sessions, each under
v3's warning; Start shares them. The guest finds the conversation in the same
list, with a badge.

**Owner, 15 acts. Guest, 9.**

![C: Conversations, empty](img/group-chat-start/c-owner-1-empty.png)
![C: New conversation, with Bret paired and gc-lane attached under the warning](img/group-chat-start/c-owner-2-new.png)
![C: the conversation, people and sessions beside the log](img/group-chat-start/c-owner-3-conversation.png)
![C, guest: the conversation, with Open on the shared session](img/group-chat-start/c-guest-1-conversation.png)

**Costs:** v4.1 §3's conversation model (`conversation` on `Share`, refs,
members, a log keyed by conversation) is a **contract** nothing provides; the
rail list, the dialog and the guest's list are new surfaces. **What it does
well:** a conversation can start before any session exists, which is v4.1's
point. **Why not first:** it is V4/V6 of v4.1 §12, whose experiments decide
where a conversation lives, and it costs the most acts here because a topic and
a session are two more fields than "share this". This mock is offered to §12 as
its first draft.

---

## 4. The security words, in each alternative

The rules are v3 §1–§2 and v2 §2. Each alternative keeps every one; this is
where each lives.

| Rule | Today | A | B | C |
|---|---|---|---|---|
| Pairing is consented by **Allow**, after comparing the code | Invite sheet | Pair step, screen 2 of 3 | Invite sheet | *Add someone new* |
| **One pairing window at a time**; closing it cancels | Invite sheet; Settings must stay open | the dialog; closing it cancels | Invite sheet | the person dialog |
| Reading is consented by **Share**, after **v3's warning** | Share dialog | Share step, screen 3 of 3, a **separate press** | its own dialog | in the New dialog, under the attached session |
| A guest holds `guest` and nothing else | `pair_begin { invite }` | unchanged | unchanged | unchanged |
| The join is let in by the owner | lane bar, corner | unchanged | unchanged | unchanged |
| Turning on a listener is the owner's act | Remote access | the reach block, one address shown, its words | the reach block | the reach block |

**A's two risks, and the answer to each:**

- *Allow and Share could blur into one consent.* They stay two screens and two
  presses, Allow's screen says "It shares nothing yet", and Share's says who
  was just paired. A test holds that `share_create` is never called from the
  pair step (**contract**).
- *Remote access turned on as a side effect.* The block names the address and
  what it means, and the press says "Turn on and show the link", not "Next".
  It turns on nothing in a session where remote access is already listening.

C carries a third: the warning sits inside a dialog with three other fields, so
it is easier to read past. A keeps it alone on its screen.

---

## 5. What A changes, file by file

| Where | Change |
|---|---|
| `app/src/settingsMap.ts` | `beta` tier `advanced` → `beginner` (S2; Ideas moves with it, one section). `guests` tier → `beginner`; its `applies` already gates it on the beta. Keywords: `guests` gains "people", "join", "invite someone". A new beginner section `join-host` on the Remote page for the guest's paste, out of `this-window-drives`, which keeps Drive and the moved-address form. |
| `app/src/beta.ts` | No new flag (one feature, one flag). Group chat's `what` gains where to start: "Share… on a running session; Join with a link… on Home." |
| `app/src/pages/remote.tsx`, `HostsPanel.tsx` | The pair form moves to the `join-host` section, titled *Join with a link*; its hint names *Invite a guest* / *Share…* on the host (F5); a pair with the beta off is refused in words (S5, F6). |
| `app/src/LaneShare.tsx` | `ShareDialog` gains Who → Pair → Share and *Someone new* (F3). |
| `app/src/PairSheet.tsx` | Its window, pending and outcome bodies become a component the share dialog also renders; the invite's "Scan this with the device" words say "Send Bret this link" (F9). |
| `app/src/App.tsx` | *Share this session…* in the header ⋯ menu and the place's context menu (F10); *Join with a link…* on Home, both behind `groupChatBeta`. |
| `app/src/GuestsPanel.tsx` | Unchanged in substance; it stays the place to remove people and hide place names. |
| Checks | `beta-check.mjs` (the Settings row moves tier, not table); `settingsmap-check.mjs` (a new section with keywords); a new check that the pair step never calls `share_create`. |
| Serve | None. |

---

## 6. What is unknown

| Question | Settled by |
|---|---|
| Does a person on a call finish Who → Pair → Share inside the 120 s window, now that the owner waits in the lane? | The first build, on the two test Macs (hypothesis) |
| Is "Turn on and show the link" read as turning on a listener? | Watching David and Bret use it (hypothesis) |
| Would a `worktrees-pair:` URL scheme be worth a clickable link in a chat app? It saves one paste; it also makes any link in any chat open the app's pairing. | S7, later |
| Does B's one list of who reads what earn a page once there are two guests? | Use; S8 |
| Where does a conversation live? | v4.1 §12, not here |

---

## 7. As built (2026-11-14, `feat/group-chat-start`)

A was built as §5 lists it, and driven end to end twice on the two test Macs
with the real apps (owner `macOS`, guest `macOS-guest`): remote access off and
nobody paired on the owner, no host on the guest, to Bret reading David's
session. **Measured** acts (clicks plus fields filled), beside §0's mock counts:

| | Mock (§0) | Built, measured |
|---|---|---|
| Owner, from the lane's *Share with…* | 8 + the beta's 3 = **11** | 8 + 3 = **11** |
| Owner, from the ⋯ menu or the place menu | — | 9 + 3 = **12** (opening the menu is one more) |
| Owner, Bret paired already | 2 | **2** from the bar, 3 from a menu |
| Guest | 4 + 3 = **7** | 5 + 3 = **8**: the join dialog ends on *Done*, and the share's toast has no *Open* (the sidebar row opens it) |

The beta's 3 (gear, tick, close) were not re-pressed on the VMs, whose switch
was on already; that Beta features is now drawn in Beginner was checked there.

What the drive found, and the build fixed:

- **The guest never saw the comparison code** (pass 1): the app made
  `remote_pair`'s event channel on the serve socket, and `remote_pair` is a
  shim command, so `{code}` went to a channel the shim does not own. This is
  older than A (*Pair with a host* had it too; §1's walk never paired from the
  app). Pass 2, with `ShimChannel`, showed the same code on both screens.
- **F7 met real data**: *Remove* keeps the person (their devices are revoked,
  their thread posts keep an author), so "Bret" stays taken after removing
  Bret, and the dialog offers *Pair another device for Bret*: the same number
  of acts as *Invite Bret*. With both "bret" and "Bret" on file it now names
  the exact one.
- A guest link is not told from a drive link before pairing, so *Pair with a
  host to drive it* stays in "This window drives" for Advanced, and *Join
  with a link* (every mode, refused with the beta off) is the guest's.

![Built: the lane's ⋯ menu](img/group-chat-start/build-owner-1-menu.jpg)
![Built: Who, with a removed Bret's name taken](img/group-chat-start/build-owner-2-who-name-taken.jpg)
![Built: the reach block, remote access off](img/group-chat-start/build-owner-3-reach.jpg)
![Built: the invite window, "Send Bret this link"](img/group-chat-start/build-owner-4-window.jpg)
![Built: the owner's code and Allow, "It shares nothing yet"](img/group-chat-start/build-owner-5-code.jpg)
![Built: the guest's Join with a link, the same code](img/group-chat-start/build-guest-1-code.jpg)
![Built: Share, a separate step after Allow; the pairing notice names Bret and the device](img/group-chat-start/build-owner-6-share.jpg)
![Built: Bret asks to join](img/group-chat-start/build-owner-7-join-ask.jpg)
![Built: Bret reads the session](img/group-chat-start/build-guest-2-reading.jpg)
![Built: Bret paired already, the two-act share](img/group-chat-start/build-owner-8-paired-already.jpg)
![Built: Settings → Remote in Beginner, Join with a link first](img/group-chat-start/build-owner-0-remote-beginner.jpg)
