---
title: iOS MVP — implementation plan and handoff
---

# Proposal — the iOS MVP: a phone that drives sessions on your own host

*Written 2026-10-22 against main at `8338853`, with the open Phase 2 PRs
#544, #549, #550 and #551 read from their branches. Where this document and
the code disagree, the code wins. Every claim about this codebase cites a
file and line or a PR; every claim about Apple or Tauri cites a page in
§12, Sources. Anything not measured on a device is marked **unmeasured**.*

**The MVP, in the user's words:** "An app that connects to my remote host
and does basic operations like interact with sessions. No direct terminal
access or file browsing on the host machine."

"Remote host" is the user's own machine running `worktrees serve` with Phase
2's remote listener (slice 6), reached over Tailscale or the LAN. The
AWS-hosted server is deferred and nothing here plans around it.

---

## 0. The recommendation, on one page

| Question | Recommendation | Where |
|---|---|---|
| Stack | **Tauri 2's iOS target**: the Rust shim carries the pinned mutual-TLS socket (rustls plus a project-local Swift plugin for the two Secure Enclave keys), a new, phone-sized React UI, and the desktop's protocol code (`ipc.ts`, `ipcws.ts`, `reconnect.ts`) reused unchanged. Not SwiftUI over UniFFI, not pure Swift. | §1 |
| First connectivity | **Tailscale first, LAN second.** Apple's own definition of a "local network" excludes VPN interfaces, so a `100.x` dial raises no Local Network prompt; a LAN dial does, and Bonjour needs `NSBonjourServices`. Both ship, in that order. | §4 |
| Device grant | Pair as **Observe** (`read`), as §11 Q4 decided. For the interactive phase the console widens the phone to the custom set **`read` + `write` + `answer` + `instruct`**, never `stream`. That set carries §1.1's label, "can run commands on <host> as you", and step-up (Face ID) is on by default for it. | §3 |
| Server work the MVP cannot ship without | Slice 6 (the remote listener, netclass, mDNS); #544 merged; a way to SHOW the pairing QR on the host; and **four new dispatch rows**: `agent_prompt` (read), `agent_answer` (answer), `agent_send` (instruct), `agent_turns` (read). | §2.4 |
| Phases | 0 hardware spike (A1–A6, throwaway TestFlight) · 1 pairing and connection · 2 read-only views · 3 interaction · 4 polish and TestFlight | §6 |
| Repo | A second Tauri project, `app-ios/`, whose crate joins the workspace and links `worktrees-core` only; the mobile UI is a second vite entry under `app/src/mobile/`; Linux CI is untouched because nothing iOS is in the Linux jobs' package lists. | §7 |

---

## 1. The stack

### 1.1 What §4.4 of Phase 2 already fixed

The Phase 2 proposal (`docs/proposals/server-edition-phase2.md` §4.4) decided
that on iOS "the remote socket lives in the Tauri shim, in Rust", for two
reasons that hold whatever the UI is: a page's `WebSocket` cannot pin a
certificate or present a client certificate, and a key the page can use is a
key an XSS in the page can use. It also fixed the shape: "the shim is a pipe,
not a second client" — the request, channel and ordering logic stays in
`app/src/ipcws.ts`, and the door table in `app/src/ipc.ts` gains a third
door, `remoteDoor`. The earlier Server Edition document decided the phone is
"a Tauri iOS and iPadOS build, not a PWA", shipped through TestFlight on the
project's Apple developer account (`docs/proposals/server-edition.md`,
decision 3).

So the question this section answers is narrower than "which stack": is
there a reason to overturn those decisions now that the client side is about
to be built?

### 1.2 The three options

| | **A. Tauri 2 iOS** (recommended) | **B. SwiftUI + Rust core over UniFFI** | **C. Pure Swift** |
|---|---|---|---|
| Protocol client | `ipcws.ts` reused as is: handshake frames, `{id, cmd, args}`, channels ordered by `seq`, `end: true`, the `MAX_MESSAGE` refusal, the `listen` frame of #550. The Rust shim forwards text frames both ways and knows nothing of their meaning. | Written again, in Rust (new: nothing in Rust implements the client side today beyond the shim's probe, `app/src-tauri/src/engine.rs`) or in Swift. A second implementation of the wire contract, which AGENTS.md's mirror rule says needs a drift check. | Written again, in Swift. Same drift problem. |
| Reconnect policy | `app/src/reconnect.ts` reused, minus "may start a serve" (§4.4). Its virtual-clock check (`reconnect-check.mjs`) keeps covering it. | Reimplemented. | Reimplemented. |
| Pairing link, address classes | `worktrees_core::pairuri` (#544) and `netclass` (slice 6a) link into the shim, as §4.1 intended: "the host and the device cannot disagree about what is private". | Same, through UniFFI. | Reimplemented in Swift, and the two can disagree. |
| TLS 1.3, SPKI pinning both ways, no resumption, the exporter | rustls, with `tls::PinnedServer` from #544 ("here for the pairing tests and the shim later", `serve/tls.rs` header). The exporter (`EXPORTER-worktrees-pair-v1`, `EXPORTER-worktrees-stepup-v1`, `tls.rs`) comes from rustls. | Same rustls code, driven from Swift. | Network.framework. I found no public API for the TLS 1.3 exporter there, and both the pairing proof (`pairing.rs` header) and the step-up signature (`stepup.rs` header) are bound to it. **Unverified in this session**; if true it rules C out on its own. |
| Secure Enclave keys, Face ID | A small project-local Swift plugin (Tauri's mobile plugin API: a Swift class, `@objc` functions, `run_mobile_plugin` from Rust) that creates the two P-256 keys and signs with `SecKeyCreateSignature`. The step-up key's `.userPresence` access control makes the system prompt for Face ID or the passcode by itself. | Native, simplest. | Native, simplest. |
| UI | WKWebView, React, plain CSS over `tokens.css`. A NEW phone layout; the desktop's `App.tsx` is not reusable on a phone. The mock harness (`app/src/mock/install.ts`) drives it headless in Chromium and WebKit, which is how the desktop UI is developed. | SwiftUI: best feel, no harness, every view new. | SwiftUI. |
| App Store / TestFlight | Documented: `tauri ios build --export-method app-store-connect`, automatic signing with a team id, upload with `altool` (Sources: Tauri distribute, sign). | Xcode's native path. | Xcode's native path. |
| Background | Identical for all three: iOS suspends the app shortly after it leaves the foreground, and the socket dies (§4.3). | Same. | Same. |
| What has to be new | The mobile UI; the shim's TLS pipe (one command out, one `Channel` in); the Swift key plugin; `remoteDoor` in `ipc.ts` and its row in `engine-route-check.mjs`. | The mobile UI; a Rust protocol client; the UniFFI surface and XCFramework build; the Swift app. | Everything. |

### 1.3 Why A

1. **The protocol has one implementation today, and A keeps it that way.**
   The transport's contract is subtle in exactly the places a second
   implementation gets wrong: frames held during proof verification are
   handed on synchronously (`ipc.ts`, `engineDoor`'s `wire`), a `listen` frame goes out
   as the FIRST frame after the proof and again on every change (#550),
   channels end on `{channel, seq, end: true}` ordered by `seq`
   (`crates/worktrees-cli/src/serve/wire.rs:19`–`31`). `ipcws.ts` already
   does all of it and `transport-check.mjs` (223 checks after #550) pins
   it. B and C re-derive it.
2. **Phase 2's security design assumed A.** `pairuri`'s parser lives in
   core "where the app's shim reaches it too" (`crates/worktrees-core/src/pairuri.rs`
   header); §4.1 has the shim call `netclass`; `tls::PinnedServer` is
   written for "the shim later". Choosing B keeps those; choosing C
   discards them.
3. **The harness.** AGENTS.md records that three v0.12.x bugs lived in
   timing the mock could not express, and that the fix was to drive the real
   app. The same mock, extended with the four new rows and a remote-mode
   fixture, is the only way to develop a phone UI headless on this project.
4. **What A gives up is UI feel**, and the MVP's screens are lists, a
   detail card and a prompt modal. That is the surface where a WebView is
   least behind.

**The fallback, if Phase 0 breaks A.** Two outcomes change the choice. If
A1 fails (rustls cannot be fed a Secure Enclave signer from the shim), B
fails identically, and the question becomes whether the connection key may
be a software key (`kSecAttrAccessibleWhenUnlockedThisDeviceOnly`, no
Secure Enclave) with the step-up key still in the Enclave; that is a
weakening of §2.1 and the user's call. If Tauri's iOS tooling itself is the
problem (the dev loop, the WKWebView bridge under load, signing), the
protocol core is the same under B, and the plan's Phase 1 is where the
switch would happen, with the mobile UI as the cost.

### 1.4 Tauri 2's iOS support, as read

- Tauri in this tree is 2.11.5 (`Cargo.lock`; `app/src-tauri/Cargo.toml:29`
  pins `"2"`), wry 0.55.1, tao 0.35.3.
- iOS development "requires Xcode and is only available on macOS", full
  Xcode and not the Command Line Tools; `rustup target add aarch64-apple-ios
  x86_64-apple-ios aarch64-apple-ios-sim`; CocoaPods through Homebrew
  (Sources: Tauri prerequisites).
- `tauri ios init` ("Initialize iOS target in the project"), `tauri ios dev`
  (`--open` for Xcode, `--host`, `--force-ip-prompt`; a physical device
  needs the dev server on the address the CLI puts in `TAURI_DEV_HOST`,
  which `app/vite.config.ts:8`–`43` already honours), `tauri ios build`
  with `--export-method app-store-connect | release-testing | debugging`,
  `--build-number`, `--debug` (Sources: Tauri CLI reference, develop).
- "On first run, iOS asks for local network permission. You must allow it
  and restart the app" — that is the DEV SERVER's prompt, not ours; it goes
  away in a release build (Sources: Tauri develop).
- Signing: automatic by default, `bundle.iOS.developmentTeam` (or
  `APPLE_DEVELOPMENT_TEAM`) is "required for iOS development because code
  signing is enforced"; CI signing through an App Store Connect API key
  (`APPLE_API_ISSUER`, `APPLE_API_KEY`, `APPLE_API_KEY_PATH`) or manual
  `IOS_CERTIFICATE` / `IOS_CERTIFICATE_PASSWORD` / `IOS_MOBILE_PROVISION`
  (Sources: Tauri sign iOS, config).
- iOS-only config lives under `bundle.iOS`: `minimumSystemVersion` (default
  15.0), `frameworks`, `infoPlist`, `template`; Tauri also merges an
  `Info.ios.plist` beside `tauri.conf.json` (Sources: Tauri config).
- Native code: a plugin "is defined as a Swift package", commands are
  `@objc func name(_ invoke: Invoke)`, Rust calls them with
  `run_mobile_plugin`, and Swift can call exported Rust through
  `@_silgen_name` "even when the application WebView is suspended"
  (Sources: Tauri develop-mobile).
- Two first-party plugins matter: `barcode-scanner` (mobile only; needs
  `NSCameraUsageDescription` in `Info.ios.plist`; `scan({windowed: true,
  formats: [Format.QRCode]})`) for the pairing QR, and `biometric`, which
  is NOT needed: it answers yes/no, while our step-up key's access control
  prompts by itself when it signs (Sources: Tauri barcode-scanner,
  biometric; Apple `userPresence`).

---

## 2. The MVP feature set, mapped to the dispatch table

### 2.1 What a row costs the phone

Every row of `crates/worktrees-cli/src/serve/dispatch.rs` declares a
capability and its objects, and `serve/resolve.rs` checks them against the
caller's grant before the body runs (AGENTS.md; `dispatch.rs:449`–`679` is
the table). The capabilities are the closed enum in
`crates/worktrees-core/src/authz.rs:33`–`49`: `Read`, `Write`, `Answer`,
`Instruct`, `Stream`, `Admin`. A device's grant is its principal's
intersected with its own (`authz::effective`, `authz.rs:473`). A refusal is
`err: "denied"` for a capability and `not found or not in this device's
scope` for an object, identical for missing and out-of-scope (Phase 2
§3.3). From #544 on, a row that needs `instruct`, `answer` or `admin`, or
that OPENS a stream, also needs a fresh step-up proof on the connection
(`serve/stepup.rs` header on `origin/feat/p2-device-registry`):
`stepup_challenge` → sign `challenge ‖ exporter` with the step-up key →
`stepup_prove {spki, signature}`; the proof holds 300 s on that connection
(`VALID_FOR`), the challenge waits 120 s for Face ID (`CHALLENGE_FOR`), and
a drive act without one gets `err: "step-up required"` (`ERR_STEPUP`).

### 2.2 The feature table

**Status** says what exists on main at `8338853`, in an open PR, or not at
all. "Needs step-up" is #544's rule applied to the row's capability.

| # | Feature | Row(s) | Capability · object | Needs step-up | Status |
|---|---|---|---|---|---|
| F1 | **Pair by QR** | Not a dispatch row: the pairing listener's one-frame protocol over TLS (`serve/pairing.rs` header, #544), after `worktrees_core::pairuri::parse` (#544) and the address rule from `netclass` (slice 6a). The answer `{paired: {device_id, principal_id, host_name, remote_addr, expiry, discovery_id}}` is the host record the phone keeps. | — | — | Server side built in #544 (open); listener binds off loopback only in slice 6; the host-side QR display is NOT built (§2.4) |
| F2 | **Host list** | Device-local: one record per paired host `{name, pin, addr, discovery_id, device_id, principal_id, expiry}` (Phase 2 §2.1, "What the device keeps"). The remote handshake's first frame `{ok, version, device: {id, principal, name, caps, projects, expiry, stepup_required}}` (§4.5) refreshes it. | — | — | First frame is slice 6 |
| F3 | **Projects and places, with dots** | `list_workspace` (`dispatch.rs:450`, `[read, filtered(workspace)]`) for the tree; events `places:changed` (payload null, re-read), `sessions:busy` `{busy, waiting, waiting_by}` (`crates/worktrees-api/src/host.rs:339`–`349`), `sessions:done` `{path, epoch}` (`watch.rs:163`). The phone declares them in its `listen` frame (#550). The dot rule is the desktop's: busy/waiting from `sessions:busy`, done from `sessions:done` against the place's `declared` state (`afterglow.ts`, `unseenWork`/`unreadOf` in AGENTS.md), idle otherwise. Acking the ring is `mark_seen` (`dispatch.rs:486`, `[write, place]`; the host stamps its own clock, §9.1). | `read`; `write` for the ack | No | Rows and projection on main (slice 1); #550 open |
| F4 | **Place detail** | From `list_workspace`'s snapshot: `branch`, `dirty`, `ahead`/`behind`, `last_commit_subject`/`_epoch`, `tmux_session` (`crates/worktrees-core/src/model.rs:20`–`50`), and per harness `agent_sessions: {name, up, model}` (`crates/worktrees-api/src/workspace.rs:137`), which is how the desktop labels the agent and its model (`app/src/mock/fixtures.ts:39`). Plan or brief summary: `place_plan` (`dispatch.rs:606`, `[read, root]`) → `PlanSummary {title, goal, …}` (`crates/worktrees-core/src/plan.rs:147`). Note: `declared.note`. Last activity: `declared.last_worked_epoch` and `sessions:done`. Optional: `project_prs` (`dispatch.rs:579`, `[read, project, places]`). | `read` | No | On main |
| F5 | **Answer a waiting prompt** | **New:** `agent_prompt {repo, slug}` → the prompt the user must SEE (the modal's lines, recognised by its footer as `activity::codex_panes` does; `crates/worktrees-core/src/activity.rs:286`–`308`, `tmux::capture` at `tmux.rs:772`) plus its `fingerprint`. **New:** `agent_answer {repo, slug, choice, fingerprint}`: the compare-and-swap of §9.2 — serve captures again, refuses if the fingerprint moved, `choice` must be one of the prompt's own options. | `read` for the prompt; **`answer`**, `place(repo, slug)` | **Yes** (answer) | Not built; §9.2 put the verb in Phase 3 and reserved the capability and audit kind |
| F6 | **Send the agent a message** | **New:** `agent_send {repo, slug, text}`: one line, pasted into the agent's pane and submitted when the composer is empty. Core has the mechanism: `harness::send` / `tmux::paste_to_ai` (`tmux.rs:840`) behind MCP `send`, which refuses mid-approval and queues mid-turn; and `plan_prompt` (`dispatch.rs:607`, `[write, session]`) already pastes FIXED text into a session from serve (`files.rs:649`). Free text is `instruct`, not `answer` (§9.2). | **`instruct`**, `place` + `session_of` | **Yes** (instruct) | Not built. Note MCP `send` refuses a CLAUDE place and answers with the session name for Claude's own `SendMessage` (`crates/worktrees-cli/src/mcp.rs:1234`); from serve, pasting into a Claude pane is the same mechanism `plan_prompt` uses, so the row can serve every harness (open question Q4) |
| F7 | **Start a session** | `open_place` (`dispatch.rs:492`, `[write, place]`; args `fresh`, `provider`, `model`, `force`, `crates/worktrees-api/src/ops.rs:672`). From a device it counts toward the 10/hour launch bucket (#538) and the 4 live device-launched lanes (#551), and writes `launched_by {principal, device}` on the place (#551). Model choices: `agent_models` (`dispatch.rs:570`, `[read, global]`). | `write` | No | On main; #551 open |
| F8 | **Close a session** | `close_place` (`dispatch.rs:500`, `[write, place, opt(session_of)]`; `ops.rs:988`). | `write` | No | On main |
| F9 | **Note, pin, lifecycle** | `set_note` (`:470`), `set_pin` (`:469`), `set_lifecycle` (`:468`), all `[write, place]`. #551 stamps `note_by` with the device. | `write` | No | On main |
| F10 | **Recent turns, not a terminal** | **New:** `agent_turns {repo, slug, n}` → the last `n` user/assistant exchanges as text, from the harness's own transcript: Claude's `~/.claude/projects/<mangled>/<sid>.jsonl` (readers exist: `agent::transcript_model`, `agent.rs:260`; `activity::tail_lines_checked`, `activity.rs:166`), Codex's rollout (`codex::rollout_turn`, `codex.rs:327`). Text only, each turn truncated, tool calls summarised to their names. | `read`, `place` | No | Not built |
| F11 | **Device self-service** | `device_revoke` (`Own`, "a device may always revoke itself"), `stepup_challenge`, `stepup_prove` (`Own`) — #544 `dispatch.rs:809`, `:842`–`:853` on its branch. | own | `stepup_prove` is the proof | #544 open |
| F12 | **Usage meters** (nice to have) | `claude_usage`, `codex_usage` (`dispatch.rs:508`–`509`, `[read, global]`, on `GLOBAL_OK`). | `read` | No | On main |

**What is explicitly OUT, and how the server enforces it.**

| Out | Rows the phone never calls | Enforced by |
|---|---|---|
| Terminal attach, dock shells | `term_open`, `term_write`, `term_wheel`, `term_resize`, `term_close`, `shell_open`, `shell_write`, `shell_resize`, `shell_detach`, `close_shell_session` (`dispatch.rs:623`–`657`, all `stream`) | The phone is never granted `stream` (§3), so each answers `denied`, not merely "the UI has no button" |
| Files, diffs | `list_dir`, `read_file`, `read_file_base64`, `file_diff`, `changed_files`, `write_file` (`:594`–`:620`) | Reads are `read`, so this is a UI scope only; writes are `write` and class-raised (#549). Phase 2 §7.1's class rules apply if a later version adds a file view |
| Docs viewer | `open_docs_viewer` is `not_remote` (`:613`): `err: "unavailable-remote"` for every device (gap 7) | Server |
| Settings that need the console | Every `HostConsole` row: host policy, MCP installs, profiles, `update_cli`, pairing and device administration (§3.4; `dispatch.rs` rows tagged `HostConsole`) | Server: `err: "console-only"` |
| `ClientLocal` rows | `copy_text`, `open_editor`, `open_terminal`, `set_zoom`, `install_app_version`, `detect_editors` (`app/src/ipc.ts`, `CLIENT_LOCAL`) | Refused over any socket; the phone's door table lists none of them |

### 2.3 The agent and session surface, as the server knows it

What "interact with a session" can mean is bounded by what core derives
today:

- **Who is in a place and what they are doing.** `activity::State` is
  `Busy | Waiting | Idle | None | Unknown` (`activity.rs:46`–`61`); a
  Claude agent's state word is `busy · waiting · idle · delegated · shell`
  (`agent.rs:157`–`161`). The MCP `place_status` tool assembles `agents`,
  `agent_state` and `activity` from the same derivation the nav dots use
  (`mcp.rs:75`–`98`, `add_agent_status`); the desktop gets the same facts
  through `list_workspace` and `sessions:busy`. There is no dispatch row
  that answers "the one-line status of this place" the way `place_status`
  does; the phone composes it client-side from F3 and F4, as the desktop
  does.
- **"Waiting on the user" is on disk nowhere** (AGENTS.md): the amber dot
  comes from the pane capture and the modal's footer. That is why F5 needs
  a read row for the prompt as well as the verb.
- **The MCP catalogue** (`mcp.rs:1049`–`1380`: `list_places`,
  `place_status`, `set_note`, `set_pin`, `set_lifecycle`, `report`,
  `messages`, `wait`, `send`, `create_worktree`, `close_session`,
  `show_doc`, `remove_worktree`) is a good list of VERBS but not of rows:
  MCP tools are the agent's bus and are deliberately kept apart from Phase 2
  controls (§7.5; #551 adds a test that `mcp.rs` names none). The phone
  uses dispatch rows only.
- **Messaging between places** (`worktrees_core::messages`, the `report`/
  `messages`/`wait` log in the git common dir) has no dispatch row. A
  `messages_post` row would let the phone drop a note an agent reads the
  next time it calls `messages` or `wait`, which is weaker than F6 but needs
  no pane and no step-up class above `write`. Listed as an option under Q4.

### 2.4 Server-side prerequisites, in order

| # | Prerequisite | Where | Blocks |
|---|---|---|---|
| S1 | **Slice 6: the remote listener** — `remote_enable`, the bind and peer rules, `netclass` (6a), `DeviceVerifier` on the listener, `admit_peer` after the handshake, the remote `Shared`, keepalive (ping 20 s, close at 60 s), the remote handshake's first frame, the address code, mDNS advert (§4.1, §4.7, §10). Gap 17 closed on main by #546. | Not started | Everything after Phase 0 |
| S2 | **#544 merged** (devices, identity, pairing, revocation, expiry, step-up; waits on #534 and #538). | Open | Phase 1 |
| S3 | **A host surface that shows the QR.** #544 adds `pair_begin` and friends as console rows but changes nothing under `app/src` (its diff stat lists no frontend file). The cheapest is `worktrees serve pair`, a console client of the loopback socket that prints the QR in the terminal and the link as text; AGENTS.md notes "there is no CLI client", so it is the first one (copy `serve_bin.rs::connect`). Settings → Devices in the desktop app is the full version. | Not built | Phase 1 |
| S4 | **#550 merged** (the `listen` frame). The phone's door sends it after the proof. A serve older than #550 has no parser for the frame and answers it with `err: "bad request: no cmd"` (`wire::parse_request`), so the client sends it only when the first frame's `version` is at or past the release that carries #550; projected events arrive either way. | Open | Phase 2 |
| S5 | **#551 merged** (provenance, the 4-lane cap). `open_place` from a device must write `launched_by` first, or the lane's MCP server carries the host's reach (§7.5). | Open | Phase 3 (F7) |
| S6 | **New row `agent_prompt`** `[read, place(repo, slug)]`: capture the agent pane of the place's session, return `{harness, lines, options, fingerprint}` or `{waiting: false}`; `fingerprint` = SHA-256 of the captured modal; options parsed per harness from the modal's footer (`activity::codex_panes` for Codex; Claude's permission modal needs its own parser, fixture-tested against real captures as `tests/fixtures/codex-inline/` does). | Not built | Phase 3 |
| S7 | **New row `agent_answer`** `[answer, place(repo, slug)]` with `{choice, fingerprint}`: re-capture, compare, refuse with `err: "the prompt changed"` on mismatch, else send the keystroke(s) for `choice` to the pane. Audit kind reserved by §9.2. The 30 s lease (`agent_lease_take`) is optional UX and can follow. | Not built | Phase 3 |
| S8 | **New row `agent_send`** `[instruct, place(repo, slug)]` with `{text}`: one line, ≤ 4 KB, no control characters (MCP `send`'s rules); refused while the agent is waiting on an approval or question (`agent_prompt` says so); pasted via `tmux::paste_to_ai` and submitted only when the composer is empty (`agent::draft_from_screen`, `agent.rs:332`). Marked in the audit log; #551's `last_remote_instruction` stamp does not apply (it is a file stamp), so the place gets a `last_remote_send {principal, device, at}` or the audit record alone — decide in review. | Not built | Phase 3 |
| S9 | **New row `agent_turns`** `[read, place(repo, slug)]` with `{n}`: see F10. Reads the transcript that `agent_sessions` names; never the pane. | Not built | Phase 2 (nice) / 3 |
| S10 | **Event projection for the new rows**: none needed if the phone pulls `agent_prompt` when a place enters `waiting` (from `sessions:busy`). An optional `sessions:prompt {path, fingerprint}` event would save one round trip. | — | — |
| S11 | **Mock and drift checks**: the four rows in `app/src/mock/install.ts`; `engine-route-check.mjs` holds the new `remoteDoor` table; `serve-dispatch-check.mjs` and `dispatch.rs`'s drift tests cover the rows' object arguments (AGENTS.md). | — | Each phase |

Everything in S6–S9 is one lane's work on the server, built red-first
(AGENTS.md), and none of it binds off loopback, so it can land before slice
6 and be exercised by the desktop over loopback (the console holds every
capability, so `agent_answer` from the desktop is a legitimate feature too).

---

## 3. The device's capabilities

- **At pairing: Observe** (`read`), pre-selected by the host dialog
  (§11 Q4; `pairing.rs` header). The device's platform is `ios` or `ipados`
  (`devices.rs`, `Platform`), which #544 interprets as "step-up required at
  pairing, whatever its capabilities" (its interpretation 4). Expiry
  defaults to disabled after 30 days unused (`DEFAULT_IDLE_DAYS`,
  `devices.rs`; §11.1 N3). At most 16 devices (`MAX_DEVICES`).
- **For Phases 1–2 that is enough.** Every row in F2–F4 and F10–F12 is
  `read`, except `mark_seen`. A read-only phone cannot ack the ring; the
  phases accept that (the ring clears when the desktop acks).
- **For Phase 3 the console widens the phone** (`device_set_scope`,
  console-only, #544; "only the console can WIDEN a scope", §3.2) to the
  custom set **`read` + `write` + `answer` + `instruct`**. The presets do
  not fit: Operate lacks `answer`, Drive adds `stream`. The pairing dialog
  and the device list compute §1.1's label from the set, and this set
  earns it: "`answer` added to `write` … that is a shell, run through an
  agent". The plan says so in the phone's own device screen, in those
  words, rather than hiding it behind "Phone".
- **Never `stream`, never `admin`.** `stream` is the whole of "no direct
  terminal access"; `admin` adds nothing the MVP uses, and a stolen `admin`
  phone can revoke the user's other devices (§2.5).
- **Step-up** gates F5 and F6 (and nothing else in the MVP) with Face ID or
  the passcode (`.userPresence` is "equivalent to specifying `biometryAny`,
  `or`, and `devicePasscode`" — Sources: Apple `userPresence`). The proof
  holds 5 minutes per CONNECTION, and iOS drops the connection when the app
  leaves the foreground (§4.3), so in practice: the first answer or send
  after each return to the app asks for Face ID, the next ones within five
  minutes do not. That is acceptable for an act that approves a command the
  agent wrote; the plan does not try to soften it.
- **Project scope:** the host chooses at pairing; the project list "starts
  empty, so a scope is always chosen" (§2.2 step 1). Phase 2 §3.2 scopes by
  project, not place.

---

## 4. Connectivity

### 4.1 Tailscale (overlay) vs LAN (mDNS)

| | Tailscale, `100.x` through the VPN interface | LAN, with Bonjour rediscovery |
|---|---|---|
| Reachability | Anywhere both devices are on the tailnet; Tailscale on iOS is an App Store app that installs a "VPN configuration", iOS 15+ (Sources: Tailscale install). It keeps itself up with an automatic VPN On Demand policy "to ensure that the VPN remains active in the event of a system restart, auto-update, crash" (Sources: Tailscale on-demand). | Same Wi-Fi only; a moved host address is found over `_worktrees._tcp` (§4.7) or an address code |
| Local Network prompt | **None expected**: Apple defines a local network as "an IP network associated with a broadcast-capable network interface. Such interfaces include Wi-Fi and Ethernet, but not cellular (WWAN) or VPN" (Sources: TN3179). **Unmeasured**: A2 confirms it on a device. | **Yes**, on the first outgoing TCP connection ("Making an outgoing TCP connection: yes"), and "All Bonjour operations require local network access". `NSLocalNetworkUsageDescription` and `NSBonjourServices` (`_worktrees._tcp`) go in `Info.ios.plist`. The multicast entitlement is NOT needed: browsing through the system's API is not raw multicast (TN3179; Phase 2 §4.4). |
| A declined prompt | — | "There's no general API that returns whether the current process has local network access" (TN3179, FB8711182); an `NWConnection` reports `localNetworkDenied`, a BSD socket from Rust sees only a failure. A2 measures the errno. The app's message points at Settings → Privacy → Local Network (§4.4), never a silent connect failure. **In the background an undetermined app is denied without the alert** (TN3179), so the first LAN dial must happen in the foreground. |
| Peer rules on the host | The peer must be in the overlay's own row (`100.64.0.0/10`), part 2 of §4.1 | On-link only (part 3) |
| Phase | **Phase 1** | Phase 1 (dial by address from the QR), Phase 4 (mDNS, address code) |

Link-local from a QR (IPv4 `169.254/16`, IPv6 `fe80::/10`) is A5's
question; if iOS cannot dial it scoped to an interface it picks itself, the
QR parser refuses link-local addresses with a message (§4.1).

### 4.2 Reconnect

`app/src/reconnect.ts` is reused with ONE rule removed: a remote device
never starts a serve (§4.4; the retrier's `mayStart` is forced false). What
stays: jittered exponential backoff from 0.5 s capped at 30 s
(`BACKOFF`), one try in flight, no automatic try while the window is
hidden, `Reconnect now` as a manual try. The `CRASH_LOOP` budget is about
STARTS and never trips. Every reconnect is a full handshake, because
resumption and tickets are off (`serve/tls.rs` header, #544); A3 measures
what that costs with a Secure Enclave signature (4.7 ms for the signature
alone on the spike's Mac, §10).

### 4.3 Background and foreground, and sleep

- UIKit moves the app to the background "briefly before UIKit suspends
  it", and apps "don't normally receive any extra execution time after they
  enter the background" (Sources: Apple background). The socket is dead the
  moment the app is suspended. Serve notices within 60 s (keepalive, §6)
  and closes its side; its step-up proof and any held state go with the
  connection.
- **On return to the foreground:** `visibilitychange` fires in WKWebView
  (AGENTS.md: "`document.visibilityState` works here" was measured on
  macOS; **unmeasured on iOS**, where it is the standard mechanism) and the
  retrier's `visibility()` runs the due try at once. The phone re-pulls
  `list_workspace` after every reconnect rather than trusting any cached
  set: AGENTS.md's rule that a late state event is a stale set.
- **Phone locked, screen off:** the connection key is
  `kSecAttrAccessibleWhenUnlockedThisDeviceOnly` (§2.1; "accessible only
  while the device is unlocked by the user", Sources: Apple), so a locked
  phone cannot even handshake. That is by design (§2.5, "Locked:
  nothing").
- **No background modes.** The app declares none: Apple grants background
  time only to VoIP, audio, location and similar (Sources: Apple
  background), and asking for one to keep a WebSocket open is both refused
  at review and against §4.4's "backgrounding suspends the socket".
- **Clients cache nothing they can lose** (§0 row 3): the phone keeps its
  host records and view state, never a transcript, prompt or file.

### 4.4 Notifications

- **MVP: local only, which means in-app.** Since the app is suspended in
  the background, a local notification can only be scheduled while the app
  is up, and the one event worth notifying ("Claude is waiting") happens
  while it is down. So the MVP's "notification" is the amber dot and a
  banner while the app is open, plus the app badge set on return. No
  notification framework code in the MVP.
- **Later phase: push.** The phrase "needs a relay" is half right. APNs
  itself is the relay: serve on the user's host can call APNs over HTTP/2
  with the team's APNs auth key (`.p8`) and the device's token, with no
  third party in between. What it takes: the `aps-environment` entitlement
  in the iOS build; a `push_register {token}` row (`own`) and a token per
  device in `devices.json`; the APNs key in `<config_dir>`, `0600`, console
  only; a sender in serve fed by the same `sessions:busy` edge that paints
  the amber dot, rate-limited per device (§6's budgets); the payload holding
  only a place id, never the prompt text (it crosses Apple's servers);
  host-side opt-in and audit. That is a design of its own and belongs after
  the MVP ships.

---

## 5. Security on the phone

| Concern | Rule | Source |
|---|---|---|
| Connection key | P-256 in the Secure Enclave, `kSecAttrTokenIDSecureEnclave`, `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`, access control `.privateKeyUsage`; permanent, tagged. The Secure Enclave "only supports 256-bit elliptic curve keys" and there is no "mechanism to transfer plain-text key data into or out of the Secure Enclave". Needs an A7 or later device; **the simulator cannot do this**, so every key test is on hardware. | Apple SE page; §2.1 |
| Step-up key | A second Secure Enclave key with `.privateKeyUsage` and `.userPresence`. Signing it is what raises Face ID; no LocalAuthentication code, no biometric plugin. Enrolled with the connection key (`stepup_spki` in the pairing frame), revoked with it. | §2.1, §2.5; `pairing.rs` header |
| What the Keychain holds | The two key references and, as a generic password item under the same protection class, the host records (pin, address, ids, discovery id, expiry). Nothing else. | §2.1 "What the device keeps" |
| What the WebView holds | Nothing secret. The page sees host NAMES and a connection state; the pin, the keys and the TLS session live in the shim. The page never receives a token (there is none in the remote design) and never sees the pairing secret, which the shim parses out of the QR before the page is told "scanning ok". | §4.4 |
| View state | `localStorage` in the WebView for per-device view state (selection, sort). It is not secret, and §9.1 says view state is per device and never reaches serve. | §9.1 |
| Files, transcripts, prompts | In memory only; never written. iOS Data Protection class `Complete` for what IS written (the host records): the app sets `NSFileProtectionComplete` as its default data protection in the entitlements. | §2.5 "What was on the phone" |
| ATS | "ATS doesn't apply to calls your app makes to lower-level networking interfaces like the Network framework or CFNetwork" (Sources: Apple ATS); a rustls socket from Rust is one of those, so no ATS exception is declared and `NSAllowsLocalNetworking` is NOT set. The WebView loads only `tauri://localhost`. **Unmeasured** on device (A2). | §4.4 |
| A lost phone | Locked: nothing, the key cannot sign. Unlocked: its grant until it is revoked from the host or another `admin` device of the same principal, or until its idle expiry (30 days unused). After the revoke: nothing, the key never left the Enclave and the pin is public. The audit log names every act. | §2.5 |
| The phone's own revoke | `device_revoke` on itself (`Own`), from the device screen, no step-up: "a device may always revoke itself". | #544 |
| Pairing a stranger's QR | The parser refuses a public address (`netclass`), an unknown parameter, an expired `exp`, a `%` zone; the screen shows the host name FROM the QR and the comparison code; the residual (a user who scans a stranger's QR and never compares) is §2.2's and is stated in the pairing screen's copy. | `pairuri.rs`; §2.2 |
| Logs | The shim logs to the app container only (no `serve.log` on a phone); nothing with a pin, an address or a name goes to the WebView console. | — |

---

## 6. Phases, with acceptance criteria

Each phase names its server-side dependency by PR or slice. A phase is done
when its criteria are SEEN on a device, in the sense of
`docs/adding-a-harness.md`: "core reports it" is not "the user sees it".

### Phase 0 — the hardware spike (A1–A6), a throwaway TestFlight build

**Deliverable:** `scripts/spike/p2-ios/` with the probe code and a
`findings.md` recording the output of every A-step from
`scripts/spike/p2-network/CHECKLIST.md`; nothing of it ships.
**Dependencies:** an Apple Developer Program membership ($99 a year,
Sources: Apple Programs); a `developmentTeam`; one iPhone; Mac A on the same
Wi-Fi and tailnet. Nothing from slice 6: the checklist's `echo_srv.py`,
`llprobe` and `mdns_reg_mac` stand in for serve.

| Step | Measures | Decides |
|---|---|---|
| A1 | rustls from the Rust shim with a Secure Enclave signer: a `rustls::sign::SigningKey` whose `sign()` calls the Swift plugin's `SecKeyCreateSignature(…, .ecdsaSignatureMessageX962SHA256, …)`; a rustls server on Mac A that requires a client certificate and pins the device's SPKI | Whether A (and B) is possible at all; the shape of the Swift plugin |
| A2 | `std::net::TcpStream` from Rust to `<LAN ip>:7800` and `<100.x>:7801`, fresh install each: does the Local Network prompt appear, for which, the errno after Deny, success after Allow | §4.1's claim that overlay mode needs no prompt; what a denied Rust socket sees |
| A3 | Twenty full mTLS handshakes over Wi-Fi: median, p95, and the signature alone | Whether per-reconnect cost is acceptable with resumption off; the UI's reconnect copy |
| A4 | A `.userPresence` step-up key signed from Rust: Face ID appears, latency, the error on cancel | The step-up UX; the error the page shows on cancel |
| A5 | Link-local from a QR: IPv6 `[fe80::…]:7401` with scope 0 and with `if_nametoindex("en0")`; IPv4 `169.254` over a USB-C adapter, unscoped and with `IP_BOUND_IF` | Whether the QR parser accepts link-local on iOS |
| A6 | Bonjour from the shim: `DNSServiceBrowse` and `NWBrowser`; with and without `_worktrees._tcp` in `NSBonjourServices`; when the prompt appears; what a declined prompt returns to each API | Which browse API Phase 4 uses; the declined-prompt message |

**Acceptance:** every step has a recorded output (not an expectation);
A1, A3 and A4 pass, which confirms the stack; A2 answers the overlay prompt
question either way. Any failure is written up as a decision for the user
before Phase 1 starts (§1.3's fallback).

### Phase 1 — the pairing and connection skeleton

**Dependencies:** #544 merged (S2); slice 6's remote listener and
`netclass` (S1); a host surface for the QR (S3); Phase 0's findings.

**Builds:**

- `app-ios/` (§7): the crate, `tauri.conf.json`, `Info.ios.plist`, the
  Swift key plugin from Phase 0, the mobile vite entry.
- The shim's remote door: `remote_connect {host_id}` opens the pinned mTLS
  WebSocket from Rust (rustls, `PinnedServer`, no SNI, no resumption) and
  returns a `Channel` of inbound text frames; `remote_send {text}` writes
  one outbound frame; `remote_close`. The page's `ipcws.ts` runs unchanged
  on top of a `Sock` built from those three (its `Sock` type is already
  structural, `ipcws.ts:105`), and `ipc.ts` gains `remoteDoor` with
  `engine-route-check.mjs` holding its table.
- The pairing flow: scan (barcode-scanner plugin) → `pairuri::parse` in the
  shim with `netclass` as the address rule → create the two keys → dial the
  pairing port with `AnyKey`-style self-signed client cert over
  `PinnedServer` → send the one frame → show the six-digit code → wait for
  `{paired}` or `{denied}` → store the host record. Error copy for every
  `denied` reason, including "Someone else used this pairing code. Tap Deny
  on <host>." (§2.2).
- Connect to the remote listener; read the first frame; verify `version` as
  the desktop does (`ipcws.ts`); send the `listen` frame (#550) if the
  server's version has it; show the device's own grant, scope and expiry on
  a device screen; `device_revoke` self.
- `reconnect.ts` wired with starts disabled; the `visibility()` hook on
  `visibilitychange`.

**Acceptance:**

- [ ] Paired over Tailscale and, separately, over LAN by address; the host's
      device list shows the phone with platform `ios`, `stepup_required:
      true`, the chosen scope.
- [ ] An unpaired install of the same app is refused at TLS (nothing past
      the handshake in serve's log).
- [ ] `device_revoke` from the host closes the phone's socket within 1 s and
      the next connect fails at TLS (§10's exit criterion).
- [ ] Kill the app, relaunch: connected without pairing again. Background
      it two minutes, foreground: reconnected with a full handshake, and
      serve's log shows the old connection closed by keepalive.
- [ ] A QR with a public address, an expired `exp` or a `%` zone is refused
      BEFORE any dial, with the parser's own message.
- [ ] The WebView's storage holds no pin, key or secret (inspect with
      Safari's Develop menu).

### Phase 2 — read-only views

**Dependencies:** nothing new on the server beyond Phase 1 (every row is on
main); #550 for the `listen` frame; S9 if `agent_turns` is wanted here.

**Builds:** the host list; the projects-and-places tree with dots from
`list_workspace` plus `sessions:busy` / `sessions:done` / `places:changed`;
the place detail (branch, dirty, ahead/behind, last commit, agent and model
per harness, plan or brief summary via `place_plan`, note, last activity);
pull to refresh; the device screen from Phase 1 with the capability label;
empty and error states for `denied` and "not in this device's scope".

**Acceptance:**

- [ ] Busy, waiting and done dots on the phone match the desktop's within
      one 3 s poll tick, measured side by side on three places.
- [ ] A device scoped to one project of two sees exactly that project; the
      other is absent (not greyed) and `list_places` on its root answers
      "not found or not in this device's scope".
- [ ] A read-only phone calling `set_note` gets `denied` and the UI says the
      device is Observe-only, naming the host as where to change it.
- [ ] `place_plan` renders a brief-only place and a plan-with-phases place;
      a place with neither shows the branch and last commit only.

### Phase 3 — interaction

**Dependencies:** S6–S8 built and merged (`agent_prompt`, `agent_answer`,
`agent_send`); #551 merged (S5); the console widens the phone to
`read + write + answer + instruct`.

**Builds:** the prompt view (the WHOLE modal, scrolled if needed, the
choices as buttons, "approve all" nowhere, §9.2); the step-up flow
(`stepup_challenge` → Swift signs → `stepup_prove`, with the Face ID cancel
error from A4 surfaced); send-a-line with the composer-busy refusal shown;
start a session (provider and model from `agent_models`; the 4-lane and
10/hour refusals shown as what they are); close a session; set the note;
`mark_seen` on opening a place with a ring.

**Acceptance:**

- [ ] A real Claude permission prompt is answered from the phone; the agent
      proceeds; the audit log has the `answer` record attributed to the
      device.
- [ ] Answer a prompt the desktop already answered: the phone gets "the
      prompt changed" and nothing is typed (shown with a stale
      fingerprint, red first against a server without the check).
- [ ] The first answer on a fresh connection asks Face ID; a second within
      five minutes does not; one after a background/foreground cycle does.
- [ ] `agent_send` lands in a Codex pane and a Claude pane and the agent
      acts on it; sent while the agent is waiting on an approval, it is
      refused and the UI shows the prompt instead.
- [ ] `open_place` from the phone: the desktop shows the "from <device>"
      badge (#551) and the lane's MCP server reports the narrowed reach; a
      fifth live device-launched lane is refused.

### Phase 4 — polish and TestFlight

**Dependencies:** Phase 0's A6 answer; slice 6's mDNS advert and address
code; the CI signing secrets.

**Builds:** mDNS rediscovery (browse only when the stored address stops
answering, ≤ 30 s, candidates checked against the discovery id and on-link
rule, adopted only after a pinned handshake, §4.7); the address-code scan;
the Local Network denied screen; iPad layout (same build; the user decides
whether it is in scope, Q5); dark mode from the device's appearance; the
app icon (`tauri icon --ios-color`); the CI job that builds the iOS target
on the macOS runner and signs only when the secrets are present (Server
Edition decision 3); TestFlight internal distribution.

**Acceptance:**

- [ ] §10's exit items that name the phone: LAN mode from an iPhone with
      the prompt answered; the host's LAN address moves and the phone finds
      it over mDNS with no address code; a spoofed advert for the same id is
      tried, refused at the pin, and changes nothing.
- [ ] A declined Local Network prompt shows the Settings message, and
      allowing it in Settings makes the next connect succeed without a
      reinstall.
- [ ] A TestFlight build is on the user's phone, installed from the
      TestFlight app, and the whole of Phases 1–3's acceptance passes on
      it, not only on a `tauri ios dev` build.
- [ ] Linux CI is green with no iOS step having run there.

---

## 7. Repo layout, build, signing, distribution, CI, mock host

### 7.1 Where the iOS project lives

```
app-ios/
  tauri.conf.json          identifier, bundle.iOS { developmentTeam, minimumSystemVersion }, frontendDist → ../app/dist-mobile
  Info.ios.plist           NSLocalNetworkUsageDescription, NSBonjourServices [_worktrees._tcp], NSCameraUsageDescription, (NSFaceIDUsageDescription not needed: no LAContext)
  src-tauri/
    Cargo.toml             crate `worktrees-ios`, workspace member; deps: tauri, worktrees-core, rustls, tokio-tungstenite (client), the barcode-scanner plugin; NOT worktrees-api
    src/lib.rs             mobile_entry_point; the three remote-door commands; the pairing command
    src/remote.rs          rustls client config (PinnedServer, no resumption, no SNI), the pipe
    src/keys.rs            the SigningKey impls that call the Swift plugin
    plugins/sekeys/        the Swift package: create key, public key DER, sign (connection key, step-up key)
  gen/apple/               generated by `tauri ios init`; commit it except build/ and xcuserdata/ (Q7)
app/src/mobile/            main.tsx and the phone UI; imports ../ipc.ts, ../ipcws.ts, ../reconnect.ts, ../afterglow.ts, ../tokens.css
app/index.mobile.html      the second vite entry; `pnpm build:mobile` → app/dist-mobile
```

Why a second project and not a target inside `app/`: the desktop shim links
`worktrees-api` for `host::boot` and the client-local bodies (editor,
terminal, clipboard, the updater), none of which exist on iOS
(`app/src-tauri/Cargo.toml:20`–`36`, `lib.rs:449`–`458`), and Tauri's CLI
takes one `tauri.conf.json` per project. Keeping the iOS crate on
`worktrees-core` alone also keeps portable-pty, hyper and the PTY host out
of the phone build. The frontend shares modules by import, not by copying,
so `transport-check.mjs`'s rule ("nothing else under `app/src` imports
`@tauri-apps/api/core`") keeps holding for the mobile tree too.

### 7.2 Bundle id, account, signing

- **Bundle id:** the desktop's is `net.casadelvalle.worktrees`
  (`app/src-tauri/tauri.conf.json:4`). Recommend `net.casadelvalle.worktrees.ios`
  for the phone and `net.casadelvalle.worktrees.spike` for Phase 0's
  throwaway, which is deleted from App Store Connect afterwards (Q1). The
  id "must match the identifier defined in `tauri.conf.json > identifier`"
  and is registered in App Store Connect (Sources: Tauri distribute).
- **Account:** the Apple Developer Program, "$99 annual membership",
  includes TestFlight and App Store distribution (Sources: Apple Programs).
  Secure Enclave keys need the `keychain-access-groups` entitlement, which a
  TestFlight build has (CHECKLIST.md A1).
- **Signing:** automatic, `bundle.iOS.developmentTeam` set (or
  `APPLE_DEVELOPMENT_TEAM` in CI). Export methods: `debugging` for a device
  build, `release-testing` for TestFlight, `app-store-connect` for the
  store (Sources: Tauri CLI). Upload with `altool` and an App Store Connect
  API key (`APPLE_API_KEY_ID`, `APPLE_API_ISSUER`, `AuthKey_<id>.p8`)
  (Sources: Tauri distribute). The updater key in `~/.tauri/` is the macOS
  updater's and is unrelated.
- **TestFlight:** up to 100 internal testers from the team's App Store
  Connect users, builds pushed to them automatically; external testers need
  the first build "approved by App Review for TestFlight" (Sources: Apple
  TestFlight). The MVP needs internal only.
- **Minimum iOS:** Tauri's default is 15.0 (Sources: Tauri config).
  Recommend 17 (Q6): nothing here needs older, and the barcode and
  Network-framework behaviour measured in Phase 0 is measured on the OS the
  user's phone runs.

### 7.3 CI, and Linux

- The Linux `rust` job builds `worktrees-core` and `worktrees-cli` by name
  (`.github/workflows/ci.yml:50`–`66`) and the `app` job builds the desktop
  crate on both OSes (`:110`–). Neither names the iOS crate, so adding it
  to `[workspace] members` (`Cargo.toml:7`) changes no Linux job. The one
  workspace-wide command in AGENTS.md's gates, `cargo check --workspace
  --all-targets`, WOULD compile it: on a Mac that is fine (tauri's desktop
  target builds there), on Linux it needs the same gtk/webkit dev packages
  the `app` job installs. Rule: the iOS crate's iOS-only code is behind
  `#[cfg(target_os = "ios")]` so the crate type-checks on every host, and
  the Linux `app` job (which already has the packages) gains
  `cargo check -p worktrees-ios`. **Linux support is the default**
  (memory): the phone build must never be a reason a Linux release path
  changes, and this layout gives it none.
- The macOS `app` job gains `rustup target add aarch64-apple-ios` and
  `cargo check -p worktrees-ios --target aarch64-apple-ios` on every PR (no
  signing needed for a check), and a `tauri ios build --export-method
  release-testing` step guarded by `if: secrets present` (Server Edition
  decision 3: "The iOS CI job runs only when the signing secrets are
  present, so forks and contributors still build everything else").
- `release.yml` is NOT touched by the MVP: TestFlight uploads are a manual
  `altool` step until the phone has shipped once.
- `cargo deny` (`deny.yml`) covers the new crate's lockfile edges by
  construction; every new crate is pinned and OSV-checked before commit, as
  handshake v1 and §4.2 require.

### 7.4 Developing against a mock host

Two loops, both needed:

1. **The mock harness, extended.** `app/src/mock/install.ts` is a stateful
   fake backend under `__TAURI_INTERNALS__`; "the command names here MUST
   track the real Tauri handlers" (its header). Add: the four new rows
   (`agent_prompt` with a fixture modal per harness, `agent_answer` with the
   fingerprint check, `agent_send`, `agent_turns`), the `Own` rows
   (`stepup_challenge`/`stepup_prove`, `device_revoke`), and a
   `?remote=<grant>` mode that answers the first frame with a device grant
   and refuses rows outside it with the server's exact words, so the
   `denied` and scope screens are designed against the real refusals. The
   pairing flow gets a `?pair=ok|denied|used|expired` fixture. Playwright
   drives it headless in Chromium and WebKit at phone viewports, exactly
   as the desktop is driven; WebKit matters because WKWebView sizes
   `<button>`s by its own rules (AGENTS.md).
2. **A real serve.** `app/scripts/server-stack.sh` starts THIS branch's
   serve on its own config dir, prefix and port. Once slice 6 lands, a
   `--remote <lan addr>` option (or `worktrees serve remote enable` against
   that stack's config dir) binds the remote listener, and the phone on the
   same Wi-Fi runs `tauri ios dev` against it. This is the loop that finds
   the timing bugs the mock cannot express (AGENTS.md's v0.12.x lesson),
   and it is the only loop in which pairing, step-up and the Local Network
   prompt are real. `sandbox.sh --app` is not needed: the phone never
   touches the desktop app.

---

## 8. Risks and open questions for the user

**Risks**

1. **A1 fails** (no Secure Enclave signer under rustls from the shim). Then
   the connection key is software-backed or the design changes; §1.3.
2. **Tauri's iOS tooling** is the least exercised part of this stack in this
   repo (nothing has been built for iOS here yet). Phase 0 is sized to find
   out before anything depends on it.
3. **Step-up fatigue:** Face ID on the first answer after every return to
   the app (§3). Measured, not guessed, in Phase 3; if it is too much, the
   lever is `VALID_FOR` or a proof that survives a reconnect on the same
   device, both server-side design changes with security cost.
4. **The prompt parser** (S6) is per harness and keyed on screen captures;
   Claude's permission modal has no fixture yet, and AGENTS.md's rule that
   "a parser written to the template" renders an empty tab applies. Survey
   real captures first.
5. **mDNS on iOS** (A6) may force `NWBrowser` through the Swift plugin
   rather than `dns_sd` from Rust; either way it is Phase 4.
6. **Apple review is not in the MVP's path** (internal TestFlight needs
   none), but anything that later goes external or to the store is subject
   to it, and an app that drives a shell on another machine may draw
   questions. Not a Phase 0–4 concern.

**Open questions (the user's call)**

| # | Question | Recommendation |
|---|---|---|
| Q1 | Bundle id and display name for the phone app, and for the Phase 0 throwaway; which Apple team | `net.casadelvalle.worktrees.ios`, "Worktrees"; `.spike` for Phase 0, deleted after |
| Q2 | The Phase 3 grant: `read + write + answer + instruct` with the "can run commands on <host> as you" label, or drop `instruct` (no free-text send) and keep the phone at `read + write + answer`? | Keep `instruct`; sending a line is the second most useful act after answering. The label is honest either way (`answer` + `write` already earns it) |
| Q3 | Tailscale first, LAN second (recommended), or LAN first? | Tailscale first: no Local Network prompt expected, and the user is on a tailnet already |
| Q4 | `agent_send` for a Claude place: paste and submit into the pane (one mechanism for every harness), or post to the message bus (`messages_post`, `write`) and let Claude read it on its next `messages`/`wait`? | Paste and submit, refused while a prompt is up; the bus as a later option |
| Q5 | iPad in the MVP? | Same build, no separate layout work until Phase 4; decide then |
| Q6 | Minimum iOS version | 17 |
| Q7 | Commit `app-ios/gen/apple/` (Tauri's generated Xcode project) or regenerate it in CI? | Commit it minus `build/` and `xcuserdata/`, so signing settings and `ExportOptions.plist` are reviewed like code |
| Q8 | Phase 0 before slice 6 lands (parallel lanes), or after? | In parallel: Phase 0 needs no serve, and its findings gate slice 5's client side, which the user already decided waits for the spike (§11.3) |
| Q9 | Push notifications, later: serve calling APNs directly with a team key on the host (no third party), or a relay? | Direct, when the time comes; it keeps "nothing leaves the host but to Apple" |

---

## 9. What is already built, and what is not

| Built | Where |
|---|---|
| Serve over loopback, handshake v1, the frame format, the limits | main; `serve/wire.rs`, `conn.rs`; AGENTS.md |
| Capabilities, objects, the resolver, result and event projection (slice 1) | main (#535 merged); `authz.rs`, `dispatch.rs`, `resolve.rs` |
| The path classifier (slice 2); class raises capability (2b) | main (#537); #549 open |
| The audit log (slice 3); resource limits (slice 4) | #534, #538 open (the Phase 2 doc's §10 says built) |
| Devices, identity, pairing, revocation, expiry, step-up; `pairuri`; `tls::PinnedServer` | #544 open, branch `feat/p2-device-registry` |
| The `listen` frame | #550 open |
| Provenance, `launched_by`, the 4-lane device cap | #551 open |
| Gap 17 (a device's git fetch pinned) | main (#546) |
| The desktop's protocol client, door table, reconnect policy | main; `app/src/ipc.ts`, `ipcws.ts`, `reconnect.ts` |
| The network spike's non-hardware half | `scripts/spike/p2-network/`, findings in the lane's cache dir |

| Not built | Needed by |
|---|---|
| Slice 6: remote listener, `netclass`, mDNS, address code, remote first frame, keepalive | Phase 1 |
| A host surface that shows the pairing QR | Phase 1 |
| The phone app itself: crate, Swift key plugin, remote door, mobile UI | Phases 1–4 |
| `agent_prompt`, `agent_answer`, `agent_send`, `agent_turns` | Phases 2–3 |
| The hardware measurements A1–A6 | Phase 0 |
| Any iOS CI step | Phase 4 |

---

## 10. Handoff for a new session

You are a fresh Claude session with no memory, asked to start building the
iOS MVP of Worktrees. Do these in order.

### 10.1 Read first, in this order

1. `AGENTS.md` at the repo root (imported by `CLAUDE.md`): the
   architecture, the gates, and the traps. Read the whole thing once.
2. This document, then `docs/proposals/server-edition-phase2.md` §0, §2,
   §3.1–§3.3, §4.1, §4.4, §4.7, §9.2, §10, §11.
3. `scripts/spike/p2-network/CHECKLIST.md`, section A.
4. `crates/worktrees-cli/src/serve/wire.rs` (the first 45 lines are the
   wire contract), `app/src/ipcws.ts` (the client of it), `app/src/ipc.ts`
   (the door table), `app/src/reconnect.ts`.
5. On the branch `origin/feat/p2-device-registry` (#544), with `git show
   origin/feat/p2-device-registry:<path>`: `crates/worktrees-core/src/pairuri.rs`,
   `crates/worktrees-cli/src/serve/pairing.rs` (header), `stepup.rs`
   (header), `tls.rs` (header), and the rows at `dispatch.rs:806`–`855`.
6. `gh pr view 544 549 550 551` — each description lists the
   interpretations its lane made.
7. `app/src-tauri/src/engine.rs` and `lib.rs` (the desktop shim, which the
   phone's shim is a cut-down sibling of), `app/src/mock/install.ts`
   (header), `app/scripts/server-stack.sh` (header).
8. `docs/adding-a-harness.md`: the "by surface" checklist discipline this
   plan's acceptance criteria copy.

### 10.2 The repo's rules that bite

- **Gates before any PR** (AGENTS.md): `cargo build --release -p
  worktrees-cli` FIRST, `make test`, `make lint`, `cargo test -p
  worktrees-core`, `-p worktrees-cli`, `-p worktrees-api`, `cargo test -p
  app --lib`, `cd app && tsc --noEmit && cargo check -p app`, and every
  `app/scripts/*-check.mjs` from the REPO ROOT. Run heavy ones as
  `worktrees run --slot gate -- <cmd>`; the machine is shared.
- **A new test must be shown to FAIL first.** Break the thing, watch red,
  restore by editing (never `mv file.bak`), `touch` the file.
- **Report cargo with a grep, not a tail**; a pipeline's exit status is the
  last stage's; `grep -c` exits non-zero on zero; `grep` inside Claude Code
  skips binary files silently (`projcfg.rs` has a NUL).
- **Commits are signed as `bret@brethash.com`** with the SSH key
  `~/.ssh/id_ed25519` (`git config` in this worktree already says so;
  `commit.gpgsign true`, `gpg.format ssh`). **No `Co-Authored-By` trailer,
  no "Generated with" footer**, on commits or PR bodies: the user's global
  instructions override any default.
- **Never push unless the user asks for that push, in that message.** An
  earlier "open a PR" is not standing permission. Commit locally, say what
  is ready.
- **Branch work happens in a place** (`.worktrees/<slug>`), never in
  `(main)`; `create_worktree` or `worktrees new <branch> --no-attach`, never
  `git worktree add`. Branch off a freshly fetched `origin/main` and check
  with `git rev-list --left-right --count origin/main...HEAD`.
- **CHANGELOG placement:** an entry goes under `## [Unreleased]`, and a
  rebase across a release can land it in the wrong section without a
  conflict; count `### Fixed` headers under `[Unreleased]` after any rebase.
  Docs-only PRs skip CI by design; `CHANGELOG.md` is NOT docs-only (it
  ships in the binary).
- **Every spawn in the engine goes through `childenv::command`;** every
  tmux call through `tmux::tmux()`; a client's string is never a flag (use
  `worktrees_api::argv::Argv`, `--` before positionals).
- **Throwaway tmux needs `-L` on every call.** A bare `tmux kill-server`
  killed the user's real server once.
- **A long-lived process outlives an upgrade:** an MCP server or a serve
  from before your change is not evidence about your change. Check pids and
  versions.
- **Measure on the device, say which platform,** and mark what you did not
  measure. "Measured on this platform" has to say which.
- **Linux support is the default** (memory): any build or CI change must
  leave the Linux path whole, and a macOS-only step needs a stated reason.
- **Scratch files** go in `~/.cache/worktrees/<project>/<worktree>/`, never
  the repo root. Planning files (`task_plan.md`, `findings.md`,
  `progress.md`) live in `.planning/` and are gitignored. At the end of a
  work stream, run `/close-out`.

### 10.3 What is built, with PR numbers

See §9. In one line: the server has capabilities and objects on main
(#535), the classifier (#537), gap 17 (#546); devices, pairing and step-up
in #544; the listen frame in #550; provenance in #551; the audit log and
limits in #534 and #538. The phone has nothing yet, and neither does slice
6 (the remote listener), which the phone cannot connect without. The
desktop's `ipcws.ts` is the client you will reuse.

### 10.4 The first three concrete tasks

**Task 1 — Phase 0's project and A1.** In a place on a branch
`spike/p2-ios`, create `scripts/spike/p2-ios/` holding a minimal Tauri 2
iOS project (`tauri ios init` in a scratch Tauri app there; bundle id
`net.casadelvalle.worktrees.spike`, `developmentTeam` from the user), a
Swift plugin with three commands (`createKey {tag, userPresence}` →
public-key DER; `sign {tag, message}` → DER signature; `deleteKey {tag}`),
and a Rust `SigningKey` that calls `sign`. On the Mac, a rustls server that
requires a client certificate and pins the device's SPKI (copy the shape of
`tls::DeviceVerifier`/`AnyKey` from #544). Run A1 on the phone and record
the output in `findings.md`. Do not touch `app/`.
*Verify:* the server's log shows a completed TLS 1.3 handshake with the
Secure Enclave key's SPKI; the same build on the simulator fails at key
creation (which proves the key is hardware-bound, not a software fallback).

**Task 2 — A2, A3, A4 on the same build.** Add a `connect {addr, port}`
probe command that opens `std::net::TcpStream` from Rust and reports the
errno; run A2 against `echo_srv.py` on both addresses with a fresh install
each time, and record whether the Local Network prompt appeared on the
`100.x` dial. Add a twenty-handshake loop for A3 (median, p95, signature
alone). Create the `.userPresence` key and run A4, recording the cancel
error.
*Verify:* each of A2–A4 has a recorded number or screenshot in
`findings.md`, with the iOS version and the phone model; the A2 answer for
`100.x` is written as measured, whichever way it went.

**Task 3 — the server rows the phone needs, over loopback.** In a second
place on `feat/agent-prompt-rows`, add `agent_prompt` and `agent_answer` to
`dispatch.rs` as `Engine [read, place(repo, slug)]` and `Engine [answer,
place(repo, slug)]`, bodies in `worktrees-api` built on
`tmux::capture` and `activity::codex_panes`' footer rule; a fixture capture
of a real Claude permission modal under `tests/fixtures/`; the
compare-and-swap refusal; the mock's entries; the drift tests. Then
`agent_send` as `[instruct, place(repo, slug), session_of(session, repo,
slug)]` on `tmux::paste_to_ai`, refused while `agent_prompt` reports a
prompt. Each test red first. Nothing binds off loopback, so the desktop can
exercise all three.
*Verify:* with the desktop app connected over loopback, a planted
`invoke("agent_answer", …)` from the WebView console answers a real prompt
in a lane; a second call with the old fingerprint answers `err: "the prompt
changed"`; `cargo test -p worktrees-api` and `-p worktrees-cli` green by
grep (`^running|^test result`); `serve-dispatch-check.mjs` and the
`dispatch.rs` drift tests green from the repo root; and a mutation that
removes the fingerprint comparison turns the compare-and-swap test red.

### 10.5 Report back

When each task ends: the SHA, what was measured (not expected), what was
left out and why, and the next decision the user has to make. The brief
that launched you says whether to push; if it does not, do not.

---

## 11. Sources

**This codebase** (main at `8338853` unless noted): `AGENTS.md`;
`docs/proposals/server-edition.md` (decisions 1–6, §III.8, §V);
`docs/proposals/server-edition-phase2.md` (§0, §1.1, §2, §3, §4.1–§4.7,
§6, §7, §9, §10, §11); `scripts/spike/p2-network/CHECKLIST.md`;
`crates/worktrees-cli/src/serve/{dispatch,wire,conn,resolve}.rs`;
`crates/worktrees-core/src/{authz,activity,agent,model,plan,tmux,codex}.rs`;
`crates/worktrees-api/src/{host,watch,workspace,ops,files,config}.rs`;
`crates/worktrees-cli/src/mcp.rs`; `app/src/{ipc,ipcws,reconnect}.ts`;
`app/src/mock/{install,fixtures}.ts`; `app/src-tauri/{Cargo.toml,
tauri.conf.json, src/engine.rs, src/lib.rs}`; `app/vite.config.ts`;
`app/scripts/server-stack.sh`; `.github/workflows/ci.yml`; `Cargo.toml`,
`Cargo.lock`. On `origin/feat/p2-device-registry` (#544):
`crates/worktrees-core/src/pairuri.rs`, `crates/worktrees-cli/src/serve/{pairing,stepup,tls,devices,dispatch}.rs`.
PR descriptions: #544, #549, #550, #551 (`gh pr view`).

**Tauri** (pages opened 2026-10-22):
- Prerequisites — https://v2.tauri.app/start/prerequisites/
- Develop (mobile, `TAURI_DEV_HOST`, first-run local network prompt) — https://v2.tauri.app/develop/
- CLI reference (`ios init`, `ios dev`, `ios build`, export methods) — https://v2.tauri.app/reference/cli/
- Configuration (`bundle.iOS`, `Info.ios.plist`) — https://v2.tauri.app/reference/config/
- App Store distribution — https://v2.tauri.app/distribute/app-store/
- iOS code signing — https://v2.tauri.app/distribute/sign/ios/
- Mobile plugin development (Swift, `run_mobile_plugin`, `@_silgen_name`) — https://v2.tauri.app/develop/plugins/develop-mobile/
- Biometric plugin — https://v2.tauri.app/plugin/biometric/
- Barcode scanner plugin — https://v2.tauri.app/plugin/barcode-scanner/

**Apple** (developer.apple.com, opened 2026-10-22 through the documentation
data endpoints):
- TN3179, Understanding local network privacy — https://developer.apple.com/documentation/technotes/tn3179-understanding-local-network-privacy
- Protecting keys with the Secure Enclave — https://developer.apple.com/documentation/security/protecting-keys-with-the-secure-enclave
- `userPresence` — https://developer.apple.com/documentation/security/secaccesscontrolcreateflags/userpresence
- `kSecAttrAccessibleWhenUnlockedThisDeviceOnly` — https://developer.apple.com/documentation/security/ksecattraccessiblewhenunlockedthisdeviceonly
- Preventing insecure network connections (ATS) — https://developer.apple.com/documentation/security/preventing-insecure-network-connections
- Preparing your UI to run in the background — https://developer.apple.com/documentation/uikit/preparing-your-ui-to-run-in-the-background
- `NSLocalNetworkUsageDescription` — https://developer.apple.com/documentation/bundleresources/information-property-list/nslocalnetworkusagedescription
- `NSBonjourServices` — https://developer.apple.com/documentation/bundleresources/information-property-list/nsbonjourservices
- TestFlight — https://developer.apple.com/testflight/
- Apple Developer Program — https://developer.apple.com/programs/

**Tailscale:**
- Install on iOS — https://tailscale.com/kb/1020/install-ios
- VPN On Demand for iOS and macOS — https://tailscale.com/kb/1291/ios-vpn-on-demand

**UniFFI:** https://mozilla.github.io/uniffi-rs/latest/

**Not verified in this session:** whether Network.framework exposes a TLS
1.3 exporter (§1.2, option C); WKWebView `visibilitychange` on iOS (§4.3);
everything the A-steps measure.
