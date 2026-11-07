---
title: SimpleX as a transport for worktrees communications
---

# Proposal — SimpleX as a transport for worktrees communications

*Written 2026-11-04 against main (`aa7e874`) by an investigation lane, for
David and Bret. It answers the eight questions of the lane's brief against
the group-chat proposal's transport boundary (`group-chat.md` §5.4, T1–T8
and the six places where the transport changes the design). Every claim
about SimpleX is marked **read** (in its docs or source, linked, with the
revision's date), **measured** (done in this lane without running any
SimpleX software: release metadata, App Store metadata, crates.io) or
**assumed**. Nothing SimpleX was installed, run, or contacted. This is an
analysis, not code, and nothing in it is legal advice.*

**The ask, in David's words:** "investigate simplex.chat integration with
worktrees. Could we make it so that communications work that way? Maybe
once that group chat ideation comes back? Also maybe even direct messages.
Would be good to be able to message bret directly in the chat like `@bret
what do you think of this` and it comes up on his UI somewhere. That has a
ton of impact on the iOS app."

---

## 0. The verdict, on one page

**No as the channel's transport. Yes, later and small, as a one-way bridge
from a host's serve to a person's own SimpleX app, run through the user's
own `simplex-chat` binary, never linked into ours. Nothing of SimpleX goes
into the iOS app.**

Four facts carry that, each from a primary source:

1. **Everything SimpleX is AGPL-3.0** — the core library, the CLI, the
   prebuilt `libsimplex`, the Node and Python packages (**read**: the
   `LICENSE` files, `simplex-chat.cabal`, `package.json`, the
   `simplex-chat-libs` repository). This project is MIT, ships binaries,
   and `deny.toml` admits no copyleft beyond file-scoped MPL (**read**).
   Linking the Haskell core into the desktop app or the iOS app makes the
   distributed binary an AGPL work; a separate process spoken to over its
   published WebSocket API does not (**assumed**: the common reading of the
   GPL's process boundary, not legal advice). So the only licence-clean
   shape is the one this project already uses for git and tmux: a binary
   the user installs, never bundled, spoken to over a socket (§3).
2. **SimpleX's identity is a connection, by design, and its content is
   deniable by design.** No user ids, per-queue keys, a double ratchet per
   contact, and content messages are **not signed** — "neither routers nor
   recipients can cryptographically prove message origin" (**read**,
   `security.md` rev 1, 2026-03-09; `rfcs/2025-04-14-signing-messages.md`:
   "Signing content messages would create non-repudiable proof of
   authorship … antithetical to SimpleX's privacy model"). v7.1 (published
   2026-11-04 as a pre-release) adds opt-in signing with Ed25519 member keys
   (**read**: `APISendMessages … sign=on`). For T1 this means: the host's
   own SimpleX client can attribute a line to the contact it came in on,
   and that is as far as attribution goes. Good enough for a grant check on
   the host's own machine; not a verifiable identity (§2, §6).
3. **The phone path does not exist for a third-party app.** Instant
   notifications on iOS need a SimpleX *notification server* that
   subscribes to the user's queues and calls APNs with its own key; the
   reference server hardcodes `appName = "chat.simplex.app"` and SimpleX's
   team id (**read**, `simplexmq`'s `Push/APNS.hs`), so SimpleX's servers
   push only to SimpleX's app. Our app would have to link the AGPL Haskell
   core (217 MB App Store binary for theirs, **measured**), hold a SimpleX
   database in the app group, run a Haskell runtime inside the 24 MB
   Notification Service Extension as they do (512 KB heap, **read**,
   `apps/ios/README.md`), and run our own notification server on a public
   address, which T7 refuses. §5 says this in full. It is not large; it is
   not possible under this project's rules.
4. **SimpleX carries lines; it does not carry the rest.** Fixed 16 KiB
   blocks, at-least-once delivery through routers, queue quotas, 100 ms–1 s
   added latency by its own objective (**read**, SMP v20; overview rev 4).
   Terminal attach, the 197 request/response rows and file reads stay on
   the remote door over Tailscale. SimpleX could only ever replace the
   channel half's Mac-to-Mac leg, and D6 already has a cheaper answer for
   that leg with zero new trust boundary (§7).

**The decisions David must make**, each with my pick:

| # | Decision | My pick | Why, in one line |
|---|---|---|---|
| S1 | Transport for the channel half (D12): serve-to-serve over Tailscale (D6), SimpleX, or both? | **Keep D6.** SimpleX does not replace it and does not improve it for two people who are already on a tailnet. | T2 (one order) and T4 (named membership) are not SimpleX's to give; SimpleX relays add a third party and a 80–170 MB sidecar for a leg Tailscale already carries (§2, §7). |
| S2 | Licence stance | **Never link the Haskell core. Sidecar only, user-installed like tmux, never bundled; never in the iOS app.** | AGPL-3.0 everywhere; `deny.toml` and the App Store both say no (§3.4). |
| S3 | `@bret` on Bret's phone | **Through Bret's own host**, as `notifications.md` already designs it: Bret's serve sees the mention and pushes with the team key. SimpleX's own app as a second, optional notifier via the bridge below. | The phone stays a device of one host; nothing new is held on it (§4, §5). |
| S4 | The one experiment | **A one-way "mentions to my SimpleX" bridge**: a host's serve drives the user's `simplex-chat -p` as a bot contact and sends a line to the owner's SimpleX app when a person is mentioned. Two days, on two Macs, through preset relays, after David's OK. | It is the smallest thing that puts an `@bret` on a real phone with nothing linked and nothing hosted, and it measures what the docs only claim: latency, offline delivery, footprint (§8). |
| S5 | Relays | **Preset operators (SimpleX Chat Ltd, Flux) with private routing on, for the experiment. Never an SMP server of ours.** | A server of ours is option B of the group-chat proposal (a public bind), refused twice already; a preset relay sees queue ids and timing, never content or names (§6.5). |
| S6 | Identity rule, if any SimpleX line ever reaches a host | **A SimpleX contact maps to a principal only by the owner, at the console, by accepting a one-time invitation the owner issued; a display name never resolves anything; an unknown member, a group link, or a contact address is refused.** | SimpleX names are local labels and groups are decentralised rosters an admin can grow (§4.2, §6.4). |
| S7 | What to tell the group-chat proposal's six places | Recorded in §2.3: hosting **different** (nobody mints ids), identity **different** (a contact is enrolled at pairing), reachability **different** (agent replies must be posted), phone **not possible**, E2E-vs-log **same**, bounds **same**. | So D5, D7, D9 and §8.1 stand as written. |

The rest is the reasoning, in the brief's order.

---

## 1. What SimpleX is, today, from primary sources

*Versions read:* `simplex-chat` v7.0.3 stable (2026-09-26); v7.1.0 published
2026-11-04 as a pre-release; `simplexmq` v6.5.0 stable (2026-04-29), 7.1.0
betas through 2026-10-17. Protocol documents: SMP v20 (2026-05-25), network
overview rev 4 (2026-03-09), security rev 1 (2026-03-09), agent protocol
v7 (2025-01-24), push notifications v3 (2025-01-24), channels overview rev
1 (2026-04-28), channels protocol, chat relays guide (2026-07-28). All
**read** on the `stable` branch, 2026-11-04. Both repositories are Haskell.

### 1.1 The stack

Three layers, each on the one below (**read**, `channels-overview.md`):

| Layer | What it gives | Where identity lives |
|---|---|---|
| **SMP** (SimpleX Messaging Protocol) | Unidirectional queues on routers. The recipient creates a queue, gets a recipient id and a sender id, and hands the sender id out of band. Fixed 16 384-byte blocks both ways; client message up to 16 048 bytes. Routers hold a message until it is fetched or expires (the reference server's default `expire_messages_days: 21`, **read**, `SERVER.md`) and refuse past a per-queue capacity (`ERR QUOTA`). Routers "MUST NOT" log commands or keep history (**read**, SMP §Router security). Two-hop onion routing ("private routing") hides the sender's IP from the recipient's router. | Nowhere. Per-queue ephemeral keys; "no user identity". |
| **Agent** | Duplex connections from two queues; NaCl box per queue plus a double ratchet (X3DH with Curve448, SNTRUP761 post-quantum KEM each step, AES-GCM); a sequential id and the previous message's hash per queue; delivery receipts; at-least-once; queue rotation. A connection starts from a **connection link** passed out of band: a one-time invitation, or a reusable contact address. | In the ratchet state on each device. |
| **Chat** | JSON sub-protocols (profiles, content, files via XFTP, groups, calls). A JSON message must fit 15 610 bytes. Groups are peer-to-peer: no group id on any server, every member holds a duplex connection to every other member, the inviting admin introduces and forwards (**read**, `simplex-chat.md` §groups). | A local SQLite database (`~/.simplex` for the CLI). |

**Channels** (v6.5 beta, 2026-04-30; v7 "multiple owners") sit on the chat
layer: an owner holds the authoritative state and an Ed25519 root key;
*relays* — the same CLI binary run with `--relay` — cache and forward to
subscribers; the channel's identity is `sha256(genesis root key)`;
roster and admin messages are signed; content is not, by default, and
public channels are not end-to-end encrypted to relays on purpose ("any
channel joinable via a public link … must be considered completely
public", **read**). Three preset relays are SimpleX Chat Ltd's
(**read**, `Operators/Presets.hs`). Cross-relay difference detection is
"designed, not implemented" (**read**, channels overview §Current gaps).

**XFTP** is the file layer: fixed-size chunks on file servers, expired
after a configured interval, up to gigabytes (**read**, `xftp.md`).

### 1.2 Relays and who runs them

- Preset SMP operators in the apps: **SimpleX Chat Ltd** (UK; `simplex.im`,
  eleven enabled servers) and **Flux / InFlux Technologies Limited**
  (`simplexonflux.com`, six SMP and six XFTP) (**read**, `Presets.hs`).
  Anyone can run `smp-server`/`xftp-server` (**read**, `SERVER.md`).
- The notification servers are SimpleX Chat Ltd's only (`ntf3`/`ntf4.simplex.im`,
  **read**, `Library/Commands.hs`), and the push doc says self-hosting them
  is "in the future" (**read**, push-notifications v3).
- Transparency: 12 law-enforcement requests in 2025, "no responsive
  information was identified/provided" (**read**, 2026-02-09).
- Governance is moving: a "SimpleX Network Consortium" and foundation were
  announced 2026-04-30, plus equity crowdfunding and paid "supporter
  badges" and "public names" (sale from December) (**read**, blog, 7.1
  notes). The protocol's resolver role for names makes outbound Ethereum
  RPC calls (**read**, SMP §Router security). Not relevant to us except
  as a sign the surface is still moving.

### 1.3 Identity with no user ids

There is no account. A person is, to each contact, a pair of queues and a
ratchet; to each group, a member id the inviter issued. A display name is
a local label, de-duplicated with a numeric suffix when two contacts
choose the same one (**read**, `CLI.md`). Connecting means one side makes
a link and the other side uses it out of band; the link is the trust
anchor, and an attacker who sees it "can impersonate Bob to Alice", not
the reverse (**read**, agent protocol §threat model). Contacts "cannot
prove that two contacts they have is the same user" (**read**,
`security.md`).

### 1.4 Stable vs beta

Stable: direct messaging, p2p groups, files, the CLI's WebSocket API, iOS
instant notifications through SimpleX's own servers, the desktop apps.
Beta or moving: channels and relays (6.5 "first beta", 7.0 multi-owner,
7.1 moderators and content signing), public names, badges, the Node and
Python libraries (the TypeScript WebSocket client is deprecated in favour
of a Node library that links `libsimplex`, **read**). `CLI.md` itself is
dated 2023-01-31.

### 1.5 Audits

Trail of Bits: implementation assessment of the cryptography and
networking, October/November 2022; cryptographic design review of the
protocols, July 2024 (**read**, `SECURITY.md`, both reports in `docs/`).
`SECURITY.md` (2026-05-25) says an implementation assessment was
"scheduled for June 2026"; the blog's 2026 index lists five posts and none
about it, so I could not establish that it happened.

---

## 2. Scoring T1–T8, and the six places

### 2.1 The requirements

| Ask | Verdict | Evidence |
|---|---|---|
| **T1 Identity**: each line bound to a writer the receiving HOST can verify and map to a principal | **Partly.** The host's own SimpleX client knows which *contact connection* a line arrived on (the ratchet authenticates the peer to that client), and the host can map that contact to a principal once. Nothing beyond that client can verify it; content is unsigned by default; a stolen decrypted database sends as the user until the next message exposes it (**read**, `security.md` §database). v7.1's opt-in `sign=on` signs with a member key the host would still have to pin. | The grant check in `serve/resolve.rs` needs a (principal, device) it trusts; a SimpleX contact can stand in for a device *only* if the owner enrolled it deliberately (S6). The audit log would record a contact id, never a device key. |
| **T2 One order per channel** | **Not met.** Order and integrity are per queue, so per sender pair (`agentMsgId` + `prevMsgHash`). A p2p group is n(n−1) queues with no sequencer; a channel's relay sequences what it forwards, but "stop before the judge" needs the order at the *host*, and relays are a third party. | §5.4 place 1: with SimpleX "nobody mints ids". The channel would have to keep its own sequence on top (the opener stamps what it sees; others carry writer-sequence pairs; a merge rule). That is the group-chat proposal's log, again, with SimpleX underneath it. |
| **T3 Offline delivery**, bounded in age and count | **Met.** Queues hold messages until fetched, up to the router's expiry (21 days default) and per-queue capacity (`QUOTA`, then a "capacity reached" marker). (**read**, SMP §Send message; `SERVER.md`.) | Better than our serve-hosted channel's "host away = read-only": a line to an absent person waits on a relay. For a line to an AGENT the same property is the `inbox.rs` hazard, and the agent half must still expire it (T8). |
| **T4 Groups with named membership control** | **Partly.** A p2p group's owner/admins add members; roles exist; a group link lets anyone with it join; the inviting admin can MITM introductions and "disrupt decentralized group state" (**read**, chat protocol threat model). Channels add signed rosters, and relays "cannot alter the roster", but subscribers are anonymous by design. | The host can name who is in a room only for members it enrolled itself; everyone else is an "unknown member id" and must be refused (S6). Membership is a fact on each member's device, not one place. |
| **T5 A log the host can read whole** | **Met, per member.** Each client keeps every message it received in its SQLite database; the CLI's API returns chats and items. There is no shared log: the host reads *its own* copy. | §5.4 place 5 already resolves this: the member's own client does the digest and the judge. With the CLI as sidecar, "the member's client" is a process beside serve. |
| **T6 Bounds** | **Met for size; ours for the rest.** A chat JSON message ≤ 15 610 bytes (> our 8 KiB `MAX_TEXT`); relays enforce queue quotas and expiry; there is no per-writer rate limit beyond that. | `channel_read` is bounded by us whatever the transport allows (§5.4 place 6). |
| **T7 No public bind by us** | **Met with preset relays; broken by self-hosting.** Clients only dial out (TLS to port 5223, or Tor). Running `smp-server` or `ntf-server` ourselves is a public listener of ours: option B. | S5. |
| **T8 The agent half stays local** | **Met.** SimpleX would carry text between people's machines; `agent_send`, `agent_interrupt`, `agent_answer`, the place lock, the audit record all stay on the owner's serve. Nothing in SimpleX reaches a pane. | Unchanged §6 of the group-chat proposal. |

Two things the table does not show and the design must know:

- **Delivery is at-least-once and the client deduplicates** (**read**,
  agent protocol; channels protocol §deduplication). A line that reaches
  the host twice must be idempotent before it is a line in the channel
  log, keyed on the message's shared id.
- **Latency** is "100 ms–1 s in addition to the underlying TCP network
  latency" as the network's own objective (**read**, overview rev 4
  §objectives), and more with private routing's second hop. Not measured
  here. For chat it is fine; for anything request/response it is not (§7).

### 2.2 Where the choice changes the design (the six places of §5.4)

| Place | With SimpleX | Verdict |
|---|---|---|
| 1 Hosting (D5) | Nobody holds the log and nobody mints the order. Each member's client holds its copy; "host away" means nothing, "nobody sequences" means everything. The channel needs its own sequence rule on top. | **Different, and it costs the design a merge rule it does not have today.** D5 stands for the Tailscale channel. |
| 2 Identity (D1) | A member's SimpleX contact is enrolled at the same deliberate step pairing is today: the owner issues a one-time invitation link and accepts the connection it produces. No step-up key comes with it; a desktop pairing has none today either. | **Different, mappable.** Rule S6. |
| 3 Reachability (D6) | A member off the tailnet can still write and read lines. Reading an agent's replies needs the owner's serve to POST them into the chat as lines (`channel_post` from the host), as §5.4 item 3 already names. | **Different.** This is the one real gain and it is the bridge of §8, not a transport swap. |
| 4 The phone (§8.1) | The phone would hold a SimpleX client, its database and keys, a Haskell runtime in the extension, and depend on a notification server of ours. | **Not possible** within this project's rules. §5. |
| 5 Encryption vs T5 | Each member's own client reads its copy; the serve beside it builds the digest. | **Same** as §5.4 already resolves it. |
| 6 Bounds | 15.6 KB per message is above our 8 KiB. | **Same.** |

---

## 3. How would worktrees integrate, concretely?

### 3.1 Three shapes

| Option | What runs | Runtime and size | Licence | Verdict |
|---|---|---|---|---|
| **A. The CLI as a local WebSocket bot API** (`simplex-chat -p <port> -d <db>`) | A second long-lived process per host, user-installed (their install script or a release binary), with its own SQLite databases in `~/.simplex` and its own TLS connections to relays. serve speaks its JSON API over loopback: `/_connect`, `/_send <chatRef> json […]`, events like `newChatItems`. The commands are generated from the core's types (**read**, `bots/api/COMMANDS.md`). | **Measured** from the v7.0.3 release assets: macOS arm64 binary 173 MB, macOS x86_64 83 MB, Ubuntu x86_64 80 MB (`.deb` 13 MB compressed), Ubuntu arm64 102 MB. A GHC runtime; its memory not measured here. | AGPL-3.0 program, used unmodified, across a socket. No obligation on our code under the common reading (**assumed**); if we ever *shipped* the binary we would owe its source offer and notices, so we would not ship it, as we do not ship tmux or git. | **The only shape that fits.** It is exactly how this project treats git and tmux: shelled out, never bundled, PATH-discovered, the fake-shim harness able to intercept it. |
| **B. The Haskell core as a library** (`libsimplex`: `chat_migrate_init_key`, `chat_send_cmd_retry`, `chat_recv_msg_wait`, **read**, `Mobile.hs`, `SimpleX.h`) | Our binary loads a 48–55 MB prebuilt shared library per platform (**measured**, `simplex-chat-libs` core-7.1.0.18) or links the static one the iOS build makes (nix `-staticlib -threaded`, **read**, `flake.nix`). The Node and Python packages do this; third-party Rust crates exist (`simploxide-ffi-core` AGPL-3.0, `libsimplex-rs` 0.1.0, both 0.x from mid-2026, **measured**, crates.io). | A Haskell runtime inside serve; a database serve owns; the app bundle grows by the library. | **The combined binary is AGPL.** `deny.toml` refuses it at the graph (no copyleft beyond MPL; no git sources), and the MIT project would have to ship under AGPL terms, on the App Store, where the FSF's reading of the store's terms conflicts with the GPL family (**assumed**). | **Blocker.** Say it plainly: not unless David relicenses the project, and not for iOS even then. |
| **C. Implement SMP ourselves in Rust** | serve speaks SMP to relays directly. | SMP v20 is specified in ABNF and is small. But a *conversation* needs the agent layer: X3DH with Curve448, the double ratchet with SNTRUP761 and its wire format, queue rotation, the chat JSON sub-protocols to interoperate with a SimpleX app, and the link formats. The Haskell is the reference and the spec trails it (v20 in May; `CLI.md` in 2023). | Clean-room implementation of a public protocol; no licence issue. | **Months of cryptography nobody audits, to reach a transport that does not meet T2 anyway.** Not an option. |

**What the desktop would run under A:** serve finds `simplex-chat` on
PATH (through `childenv`, like every spawn), starts it with a database
under the config dir, and keeps one WebSocket to it; `Policy::git`'s
sibling would decide what it may dial (it dials relays, not hosts we
choose). **What the phone would run:** nothing — there is no CLI on iOS
and B is refused (§5). The phone keeps talking to its host.

### 3.2 What a sidecar costs, honestly

- A second always-on process per host with its own network identity, its
  own 21-day queues on third-party servers, and its own state to back up
  (lose the database, lose every contact: the threat model's "loss of all
  owner devices" is the catastrophic event, **read**).
- A fourth kind of installed dependency to doctor (`worktrees doctor`),
  version-detect and keep working across its betas; the API is
  "automatically generated from core library types", which means it moves
  with them.
- Every `@bret` line would exist in two places: our channel log and a
  SimpleX database, each with its own retention.

### 3.3 Size, for scale

| Thing | Size | Source |
|---|---|---|
| `simplex-chat` CLI, macOS arm64 / Ubuntu x86_64 | 173 MB / 80 MB | **measured**, GitHub release assets v7.0.3 |
| `libsimplex` prebuilt, macOS arm64 | 49 MB zipped | **measured**, `simplex-chat-libs` core-7.1.0.18 |
| SimpleX Chat iOS app | 217 MB | **measured**, App Store metadata, v7.1, 2026-11-05 |
| SimpleX Chat Android APK | 88 MB | **measured**, release assets |
| worktrees app's largest admitted dependency (mermaid, viewer bundle only) | 5.3 MB | **read**, AGENTS.md |

### 3.4 The licence, checked

- `LICENSE` in `simplex-chat` and `simplexmq`: GNU AGPL v3 (**read**).
  `simplex-chat.cabal`: `license: AGPL-3`. Node `package.json`:
  `"license": "AGPL-3.0"`. The prebuilt-library repository: AGPL-3.0.
  The README adds that the name, logo and graphic assets are *not* under
  it (trademark and assets licences).
- `deny.toml` (**read**): allow list is MIT, Apache-2.0 (with LLVM
  exception), BSD-2/3, ISC, Zlib, Unicode-3.0 and MPL-2.0 as file-scoped
  copyleft "we ship unmodified from crates.io"; `unknown-git = "deny"`,
  crates.io only. The iOS repository's `deny.toml` is the same list minus
  BSD-2. Neither admits AGPL, and the comment on MPL says why: obligations
  must attach to files we do not modify, never to our binary.
- How we distribute: the CLI on PATH, signed app bundles for macOS,
  AppImage and `.deb` for Linux, and the iOS app through TestFlight and
  the App Store (**read**, AGENTS.md §Release; `notifications.md`).
- **Is it a blocker?** For linking (B): **yes**, and for the iOS app
  doubly so. For a user-installed sidecar (A): **no**, with one rule —
  never bundle it, never patch it, never put it in `externalBin` or the
  `.deb`'s files. The same sentence AGENTS.md already has for tmux and git.

---

## 4. `@bret` and channels on SimpleX

### 4.1 How the lines map

| Group-chat concept | SimpleX object | Fit |
|---|---|---|
| A person-to-person line (`@bret …`, D11) | A **direct contact**: one duplex connection David's client ↔ Bret's client. | Natural. Two queues, a ratchet, offline delivery, and the host's client knows which contact spoke. |
| A channel (several people, several agents) | A **p2p group** (every member to every member) or a **channel** (owner → relays → subscribers). | Awkward. A p2p group has no order and an admin who can rewrite introductions; a channel is a broadcast with a third-party relay that reads the content and where subscribers are anonymous. Neither is "a shared log two people and their agents append to". |
| An agent's reply appearing in the room | A line the owner's serve **posts** as the owner's contact (the `channel_post` of §5.4 item 3). | Fine, and it is the one thing SimpleX makes nicer: Bret off the tailnet still sees David's Claude answer. The agent itself never touches SimpleX; the serve posts on its behalf, labelled. |
| "stop" | A line like any other. | Only the agent's own host turns it into an Escape, after T1's check (unchanged). |

### 4.2 Resolving a name with no global ids

`@bret` resolves against the host's member list exactly as the group-chat
proposal already says (D11, §1.5): the host knows "bret" because David
enrolled a principal of that name. With SimpleX the enrolment is: David's
serve issues a **one-time invitation** (`APIAddContact`), David sends the
link to Bret over any channel they already trust, Bret's serve accepts it
(`APIConnect`), and when the connection event lands, David confirms at the
console "this contact is Bret" (S6). From then on the host's SimpleX
*contact id* is the key for that principal, the way a device id is today.
Nothing else resolves: not the display name a contact sends in its
profile, not a contact that arrives through a **contact address** (anyone
holding the address can request), not a member an admin introduced into a
group. SimpleX's own threat model is why: "a group admin can carry out
MITM attack between user and other group member(s) when forwarding
invitations" (**read**).

One-time invitation versus this project's pairing, side by side:

| | worktrees pairing (Phase 2) | SimpleX one-time invitation |
|---|---|---|
| Secret in the link | A one-use secret, the host's address and pin, 120 s | A queue address and the inviter's keys; one use; the link expires once the intended party connects |
| Who can abuse a seen link | Nobody: the proof needs the device key and the six-digit code | "Impersonate Bob to Alice" (the inviter is fooled, not the joiner) |
| Proof of the other side | Mutual TLS pinned both ways; step-up key for drive-class acts | The ratchet from then on; no second key |
| Where the identity lives | A device key (Secure Enclave on the phone, a `0600` file on a Mac) | The client's database |

So a SimpleX contact is weaker than a paired device in two ways: a seen
link can be raced, and the identity is a database rather than a key. Both
are acceptable for `read`-class lines to a person; neither is acceptable
for `instruct` or `answer` toward an agent without the step-up the
group-chat proposal already wants for peers.

### 4.3 How `@bret` shows up when Bret is not looking

Through Bret's host, every time; SimpleX changes only the leg between the
two Macs:

1. David types `@bret what do you think of this` in the channel.
2. David's serve parses the mention, appends the line to the channel log,
   and — under D6 — sends it to Bret's serve over the paired Tailscale
   connection; under SimpleX, it hands the line to the sidecar, which
   puts it on Bret's queue at a relay.
3. Bret's serve (or his sidecar, which his serve reads) receives it,
   stamps the per-member unread marker, and emits the `channel:mention`
   fact.
4. Bret's app, if open, shows the strip notice and the nav ring. If closed,
   Bret's serve pushes to Bret's phone with the team key, "David mentioned
   you in #worktrees", names encrypted end to end, no text — exactly
   `notifications.md`'s model with one new `kind`.
5. If Bret's serve is not running either: under D6 the line waits on
   David's host until Bret's serve is back; under SimpleX it waits on the
   relay for up to 21 days. Either way nothing reaches the phone until a
   serve of Bret's runs, because only a serve holds the APNs key.

The **bridge** of §8 adds a sixth row that needs no new iOS work at all:
David's serve sends the line to *Bret's SimpleX app* as a bot contact, so
Bret's own phone buzzes through SimpleX's notification path. It is a
mirror, like the Slack mirror the group-chat proposal keeps for later,
with a better privacy story and no account.

---

## 5. iOS

David expected this to be large. It is, and most of it is "not possible
under the rules we have", which is the better answer to get early.

### 5.1 Is there a usable iOS library inside a Tauri app?

What SimpleX's own app does (**read**, `apps/ios/README.md`,
`spec/services/notifications.md`, `flake.nix`): a SwiftUI app links a
static Haskell library cross-built with haskell.nix (`-staticlib
-threaded -DIOS`, archives patched with `mac2ios`, built on their Hydra;
the archives are not in git), initialises the GHC runtime with a 64 MB
heap, and talks to it over a C API of about twenty functions carrying JSON
strings. The App Store binary is 217 MB (**measured**).

Could worktrees-ios link that? Technically the shim is already a Rust
`staticlib` under a Tauri Xcode project and a C static library can be
linked beside it; the Haskell runtime's threads and signal handling would
have to coexist with tokio's. Nobody has done it in Tauri that I could
find, and it would not matter if they had: the library is AGPL-3.0 and
the product goes through the App Store. **Blocker (S2).** The two Rust
FFI crates on crates.io (`simploxide-ffi-core`, `libsimplex-rs`) are
thin wrappers over the same library and inherit the same answer.

### 5.2 Background delivery: SimpleX's modes against `notifications.md`

| | SimpleX Chat | `notifications.md` (decided 2026-11-04) | Fit |
|---|---|---|---|
| Who calls APNs | A SimpleX *notification server* (`ntf-server`) that holds the APNs key and subscribes to the user's queues on the SMP routers. Defaults: `ntf3`/`ntf4.simplex.im`. | The user's own serve, with the team's `.p8`, no third party. | **Collide.** SimpleX's push is a third party by construction. |
| Whose app can be pushed | `defaultAPNSPushClientConfig` has `appName = "chat.simplex.app"`, `appTeamId = "5NN7GUYB6T"` (**read**, `Push/APNS.hs`). A different app needs a different notification server, built with different constants, holding that app's key, on a public address. | Our bundle id, our key, on the host. | **Not possible without running a server of ours** — option B, refused. |
| What the push carries | An e2e-encrypted envelope to the device; the NSE starts a Haskell runtime (512 KB heap), opens the shared database, fetches the queued message, decrypts, and shows a preview; ~30 s window; three modes: instant, periodic (`BGAppRefreshTask` ~20 min), off (**read**). | ChaCha20-Poly1305 names only; the NSE decrypts names with CryptoKit; "nothing answers from a notification"; prompt text never leaves the host. | The *shape* is the same idea (an NSE that decrypts on device). The *content* differs: SimpleX's NSE fetches the message; ours decrypts a name. |
| Where state lives on the phone | A SimpleX database and keys in the app group, readable by the NSE. | Nothing derived from more than one host; records under `NSFileProtectionComplete`; one push key at `AfterFirstUnlock`, "the only item at that class". | A SimpleX database on the phone is the first thing the phone would hold that is not derived from one host (§8.1 item 4 of the group-chat proposal calls this the largest change). |

### 5.3 Memory, size, App Store

- The NSE limit is 24 MB (**read**, `notifications.md`); SimpleX runs a
  Haskell runtime inside it with a 512 KB heap and closes the store after
  each burst (**read**). It is possible; it is theirs, tuned for years.
- Binary size: the SimpleX app is 217 MB; worktrees-ios ships a WebView, a
  Rust shim and three small plugins. Adding a Haskell static library plus
  OpenSSL, GMP and libffi (the archives the nix post-install copies,
  **read**) is on the order of tens of megabytes before the runtime.
- App Store: distributing an AGPL-linked binary under the store's terms is
  the FSF-contested case (**assumed**); SimpleX can do it because it holds
  the copyright. We would not.

### 5.4 What the Notification Service Extension could do

Nothing new is needed. The decided design already has the extension
decrypt names and set the thread; a person mention is one more `kind`
(`mention`, `Active`, its own switch, off by default) sent by Bret's own
host. The extension never needs SimpleX to put `@bret` on the lock screen,
because the host that saw the mention is the host that pushes.

### 5.5 Summary for the phone

Nothing SimpleX goes into worktrees-ios. The phone's `@bret` arrives from
its paired host over our APNs path. If David and Bret want SimpleX's app to
buzz as well, the bridge of §8 does that from the Mac, and the phone's
*SimpleX* app, not ours, receives it.

---

## 6. Trust and authority

### 6.1 Does SimpleX change "a person's line reaching an agent with someone else's credentials"?

No. SimpleX protects the wire and the metadata; it has no notion of a
principal, a grant or a host. The group-chat proposal's three rules stand
unchanged: an agent is driven only by its own host; "stop" is a word the
host recognises; a person's line is text to an agent, never the owner's
voice. A line that came in over SimpleX is, to the owner's serve, a line
from *the contact it came in on*, and that contact is a principal only
because the owner said so at the console (S6). Then `resolve.rs` checks
the grant, `agent_send` pastes it with the label, the audit log records the
principal and the text's length and hash. Nothing in that path knows what
carried the line.

### 6.2 Device identity against `devices.json`, mutual TLS, step-up, audit

| Control | Today | With a SimpleX contact as a principal's "device" |
|---|---|---|
| `devices.json` | A device key's SPKI hash is the pin; grants key on (principal, device). | The key would be a SimpleX contact id in the host's sidecar database: not a key we hold, not pinnable, rotatable by the sidecar (queue rotation), lost with the database. It can be a *member* record; it should never be a *device* record. |
| Mutual TLS | The device signs the certificate and the handshake with its key, every dial. | None. The sidecar's TLS is to a relay with the relay's certificate; the contact proves nothing to serve beyond "the ratchet still decrypts". |
| Step-up | A second key with presence (Face ID), per connection, for drive-class acts. | Not available: a desktop contact has no presence proof, as today's desktop pairing has none. So any SimpleX-borne line is `read`/`write`-class at most until a peer step-up exists (group-chat §6.4). |
| Audit log | Every mutation names principal and device; `agent_send` records length and hash. | Records the principal and a contact id; the line's text still never reaches the log. A refusal for an unknown contact is a record like any refused caller. |
| Revocation | Closes the device's sockets at once. | Delete the contact in the sidecar: the ratchet ends, the queues are deleted on the relay. Equivalent in effect, slower, and it lives in another program's state. |

### 6.3 Could a SimpleX group become a way around them?

It could, if the host ever trusted group membership. Three ways, each
refused by S6:

- **An admin adds a stranger.** In a p2p group any admin can introduce a
  member; the host's client then has a member id it never enrolled, with a
  profile name of the member's choosing. If names resolved, `@david` could
  be the stranger. Names resolve nothing; unknown members are refused.
- **A group link.** A link lets anyone who holds it join. The host never
  creates one.
- **A relay.** In a channel the relay forwards unsigned content and "can
  send arbitrary unsigned content messages to subscribers, effectively
  fabricating the content stream" (**read**, chat protocol §channel relay).
  A channel is therefore not a place a host should read lines from at all,
  signed or not, until cross-relay checking exists.

And one way SimpleX *removes*: it cannot be used to reach an agent from
outside the member list by any route, because the only thing that pastes
into a pane is the owner's serve, after the grant check (T8).

### 6.4 What is revealed to relay operators

From the threat model (**read**, `security.md`, `push-notifications.md`,
channels overview), for David and Bret using preset relays with private
routing on:

| Who | Learns | Does not learn |
|---|---|---|
| Bret's SMP router (SimpleX Chat Ltd or Flux) | That some queue on it receives messages, when, how many; Bret's IP when he fetches (unless Tor); when he is online. | Who sends (David's IP is hidden by the proxy hop), the content, the type, that two queues belong to one person (except by connection reuse or timing). |
| David's proxy router | David's IP; how many messages he sends and to which destination routers. | The destination queues, the content, whether they trigger notifications. |
| A channel relay (only if channels were used) | **The content**, the member ids, the owner's signing key. | Subscribers' IPs or identities. |
| A notification router (only for SimpleX's own app) | The device token; how many queues and routers; how many notifications. | Queue addresses, content, anything about messages sent without the flag. |
| Apple (for SimpleX's own app) | That a push happened. | Everything else. |

For a worktrees channel the lines are branch names, commit subjects, agent
replies and prompt cards' first lines. None of that reaches a relay. What
reaches a relay is that two IPs (one hidden) exchange a few hundred 16 KiB
blocks a day, from a UK company's and a Flux-run server. That is a far
smaller surface than the Slack of option C and a larger one than D6's
Tailscale, which reveals nothing to anyone but Tailscale's coordination
server (key exchange only).

---

## 7. Replace Tailscale or remote access, sit beside them, or not fit?

**Sit beside, for lines only, and only Mac-to-Mac; everything else stays
where it is.**

| Path | What it carries | Needs | SimpleX fits? | Stays on |
|---|---|---|---|---|
| Chat lines between people's machines (the channel half) | Short text, a few a minute, tolerant of seconds | Attribution, order, offline delivery, bounds | **Partly** (T3, T5, T6, T7, T8 yes; T1 partly; T2, T4 no) — usable with our own sequence on top, at the cost of a sidecar and a third party | **D6 (Tailscale + peer pairing)** stays the pick; SimpleX is the §8 bridge, not the channel |
| Agent replies into the room | Text the owner's serve posts | The same | Same as above | Same |
| `@bret` to a phone | A name and a kind | The APNs key, which only a host holds | **No** for our app; **yes** for Bret's *SimpleX* app through the bridge | The host's APNs path (`notifications.md`) |
| Terminal attach (`term_*`, the dock shells, `agent_*` screens) | Keystrokes and bytes both ways, ordered per stream, tens of frames a second | Low latency, a serial lane per stream, a long-lived socket | **No.** A message queue through two routers with 16 KiB fixed blocks and at-least-once delivery is the wrong shape; the dispatcher's lane rule exists because even a WebSocket needed it | The remote door (pinned mTLS over Tailscale) |
| The 197 rows (`list_workspace`, `agent_turns`, `read_file*`, `push_register`, step-up…) | Request/response JSON, up to 16 MiB | A reply to a request, now | **No.** SMP is "not semantic", asynchronous, and routers "do not communicate with each other"; a request through a queue is an RPC built on a mailbox | The remote door |
| File transfer | Bytes, up to a diff or a document | Bounded, now, scoped by the path classifier | **No** for `read_file`; XFTP exists for files but is asynchronous, expires, and lives on third-party file servers | The remote door; `fetchpolicy` for git |
| Pairing and identity | A key, a pin, a proof | Mutual TLS, a presence key | **No.** SimpleX's identity is a database; ours is a key | `worktrees-remote` |

What this means for D6: unchanged. Tailscale is the overlay under pairing;
SimpleX cannot be the overlay under anything but chat, and for chat
between two Macs that are both on the tailnet it buys only the case
"Bret away from the tailnet can still write a line", which the bridge
gives him on his phone's SimpleX app anyway.

---

## 8. Should we?

**Not as the transport. Yes to one small bridge, after a two-day probe
with David's OK.**

### 8.1 The experiment that proves or kills it

**"Mentions to my SimpleX."** A host-policy setting that names a local
`simplex-chat` the owner installed. When on, the owner's serve starts it
headless on loopback (`simplex-chat -p <port> -d <db under the config
dir>`), creates a bot profile once, and issues a one-time invitation the
owner opens in their own SimpleX app. From then on, when a person is
mentioned in a channel (or, before channels exist, when a place's agent
hits the 30 s waiting edge), serve sends one line to that contact:
"David mentioned you in #worktrees: <first line>". Nothing comes back;
replies in SimpleX are ignored in the first cut.

What it proves, each with a number:

- **Latency**, send to buzz, over preset relays with private routing, ten
  samples: is it the "100 ms–1 s" the overview promises?
- **Offline delivery**: the phone off for an hour, the Mac asleep for a
  night; does every line arrive once, in order?
- **Footprint**: the sidecar's RSS and CPU idle on the Mac, and disk under
  the config dir after a week.
- **The install story**: does `worktrees doctor` find and version it, and
  what does a missing or old binary read like.
- **Attribution**: that the line arrives from the bot contact and nothing
  else, and that a second invitation cannot be accepted.

What it needs: the sidecar on two Macs, a SimpleX app on a phone, preset
relays (metadata as in §6.4: the relays learn that two IPs exchange a few
blocks), two days. No change to the agent half, no iOS code, no licence
question, no server of ours. It must wait for David's OK because public
relays see the metadata and because it creates a SimpleX profile for the
bot (the brief forbids this lane from doing that).

What kills it: a measured latency over a few seconds; a dropped line in
the offline test; a sidecar that costs more than a tmux server at idle;
or David deciding one more thing to install is one too many.

### 8.2 What I would do instead for the channel

Exactly what the group-chat proposal recommends: Step 1, a one-host
channel over the paired Tailscale connection, with `@bret` as a log entry
plus `channel:mention`, and the person-mention `kind` added to the push
list. The transport seam (`ChannelTransport`) is worth keeping as a trait,
not for SimpleX, but because the bridge above is a second *implementation
of `notify`*, not of `append`.

### 8.3 A later "yes", if ever

If two things change, revisit: (1) SimpleX ships self-hostable
notification servers with a configurable app id, and the project is
willing to run one on a public address (it is not today); (2) a Rust SMP
client appears that this project could depend on under its licence rules.
Neither is on SimpleX's published roadmap (**read**, README §Roadmap).

---

## 9. What I could not establish

- **Whether the June 2026 Trail of Bits assessment happened.** `SECURITY.md`
  (2026-05-25) says scheduled; nothing on the blog by 2026-11-04.
- **Real latency and footprint.** The overview's "100 ms–1 s" and the
  CLI's memory are the docs' numbers and a guess; the probe measures them.
- **Whether a Haskell static library links cleanly beside a Rust
  `staticlib` in a Tauri iOS target.** Moot under S2, so not pursued.
- **The exact AGPL reading** of "a program spoken to over a socket". I
  used the common one; the project should decide whether it wants counsel
  before even the sidecar, since the same reading already covers git and
  tmux.
- **How SimpleX's queue rotation interacts with a host that keys a
  principal on a contact id.** The agent rotates queues, not contacts, so
  the contact id should be stable; not verified in the source.

---

## Appendix A. Sources, with the revision read

SimpleX (all on the `stable` branch, read 2026-11-04):

- Network overview, rev 4, 2026-03-09 — `simplexmq/protocol/overview-tjr.md`
- SMP, v20, 2026-05-25 — `simplexmq/protocol/simplex-messaging.md`
- Security and threat model, rev 1, 2026-03-09 — `simplexmq/protocol/security.md`
- Agent protocol, v7, 2025-01-24 — `simplexmq/protocol/agent-protocol.md`
- Push notifications, v3, 2025-01-24 — `simplexmq/protocol/push-notifications.md`
- XFTP — `simplexmq/protocol/xftp.md`
- APNs client config — `simplexmq/src/Simplex/Messaging/Notifications/Server/Push/APNS.hs`
- Channels overview, rev 1, 2026-04-28 — `simplex-chat/docs/protocol/channels-overview.md`
- Channels protocol — `simplex-chat/docs/protocol/channels-protocol.md`
- Chat protocol (groups, threat model) — `simplex-chat/docs/protocol/simplex-chat.md`
- Signing RFC, 2025-04-14; identity binding RFC, 2026-03-28; chat relays RFC, 2025-10-20 — `simplex-chat/docs/rfcs/`
- CLI guide (2023-01-31), chat relay guide (2026-07-28), server guide, security policy (2026-05-25), transparency (2026-02-09) — `simplex-chat/docs/`
- Preset operators — `simplex-chat/src/Simplex/Chat/Operators/Presets.hs`; default notification servers — `src/Simplex/Chat/Library/Commands.hs`
- C API — `simplex-chat/src/Simplex/Chat/Mobile.hs`, `apps/ios/SimpleXChat/SimpleX.h`; iOS build — `flake.nix`, `apps/ios/README.md`, `apps/ios/spec/services/notifications.md`
- Bot API — `simplex-chat/bots/api/COMMANDS.md`; Node and Python packages under `packages/`
- Blog 2026-04-30 (channels, consortium); release notes v7.0.3 and v7.1.0

Measured in this lane: GitHub release asset sizes (v7.0.3; `simplex-chat-libs`
core-7.1.0.18); App Store metadata for the SimpleX iOS app (v7.1,
217 131 008 bytes); crates.io metadata for `simploxide-*`,
`libsimplex-rs`, `simplex-chat`.

This repository: `docs/proposals/group-chat.md` (branch
`group-chat-ideation`, §5.4 and §8.1), `AGENTS.md`, `deny.toml`,
`crates/worktrees-remote/IOS.md`, `crates/worktrees-core/src/messages.rs`;
the phone repository's `docs/design/notifications.md`, `src/conn.ts`,
`src-tauri/Cargo.toml`, `deny.toml`.
