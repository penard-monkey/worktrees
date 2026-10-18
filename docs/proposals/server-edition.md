---
title: "Proposal — Server Edition"
---

# Proposal — Server Edition: one machine runs the engine, any device drives it

**Status:** design plus decisions, 2026-10-17. Phase 0 (the `worktrees-api`
extraction) is built and in review as PR #508. Phase 1 is in progress on
branches stacked on it: `worktrees serve` (the loopback server), the frontend
transport, and the packaging that ships the CLI inside the app. Nothing binds
off loopback, and nothing will until Phase 2's gates hold.

**Provenance.** The design is Part III of the plan kept in the
`feat-pretty-ui-server` place, written 2026-10-15 against `b47cbe0`. It is
reproduced below with its section numbers intact, so citations elsewhere in the
repo (`§III.4`, `R2`, `§III.9 Phase 0`) resolve here. Part II of that plan, the
conversation view, is a separate workstream and is not reproduced. Decisions
made after the plan come first. Where they override the plan, the plan's text
carries a dated note at that spot. Where this document and the code disagree,
the code wins.

## Decided since the plan

1. **One package; the UI is always a client.** An installed Worktrees is
   configured, not chosen: which servers it talks to, and whether one of them
   runs on this machine. A desktop can host its own places, drive a remote
   server's places, or both at once. iOS and iPadOS cannot host places at all —
   no tmux, no local git worktrees — so they are always clients of a remote
   server. That is a platform fact, not a product tier.
2. **One engine process per host.** The plan drew today's app and `worktrees
   serve` as alternatives on one machine and never both, which is the normal
   case. Two processes each holding a `Ctx` means two nav-dot polls, two sets
   of dock-shell PTYs neither can see, two docs servers, and two writers to
   `shell-cwds.json`, the scrollback rings and `ui-events.jsonl`. So the
   desktop app becomes a client of a local `worktrees serve`, the same way a
   second desktop is one. The shim keeps the window, the client-local commands
   (§III.4.B) and the job of finding or starting the server. This makes the
   version handshake mandatory: after an upgrade, "the app is new and the
   server is old" is the default state. Recorded in the crate doc of
   `crates/worktrees-api/src/lib.rs`.
3. **The phone is a Tauri iOS and iPadOS build, not a PWA.** This supersedes
   §III.8's recommendation and answers §V.3. It ships through TestFlight on the
   project's Apple developer account. The iOS CI job runs only when the signing
   secrets are present, so forks and contributors still build everything else.
   §III.8's Web Push path no longer applies; the notification design moves to
   Phase 3.
4. **Apple signing is an iOS concern only.** macOS releases stay ad-hoc signed,
   as today. `install.sh` strips quarantine recursively, so a binary nested in
   the bundle runs. Nobody signs every macOS release.
5. **The CLI ships inside the app as a private sidecar (Option B).** The app
   bundle carries the `worktrees` binary and uses it only to start `serve`.
   `install.sh` keeps putting the CLI on `PATH`, and MCP registrations keep
   pointing at that copy. Offering to link the bundled copy onto `PATH`
   (Option A) is a later opt-in; `ROADMAP.md` records why B came first.
6. **The loopback listener has its own rule set.** §III.3's table describes the
   REMOTE, TLS listener. On plaintext loopback the threats are cross-site
   WebSocket hijacking (browsers apply no CORS to a WebSocket handshake) and DNS
   rebinding, and only a `Host` check stops the second. So there are two
   listeners with two rule sets. The loopback one:
   - `Host` must be loopback: `127.0.0.1`, `localhost` or `[::1]`, with or
     without a port.
   - `Origin` must be on an allowlist: `tauri://localhost` and
     `http://tauri.localhost`, plus entries from a dev-only `--allow-origin`
     flag, which is never a config key, so a dev entry cannot ship. A MISSING
     `Origin` is refused. That deliberately diverges from the docs server,
     which allows a missing `Origin` for a same-origin document GET. No CORS
     header, ever.
   - The first frame must be `{"auth": "<token>"}` within a few seconds. The
     token is 256 bits, rotated every launch, and never in a URL.
   - `serve` writes `serve.json` (`{port, token, pid, version}`) into the app's
     config dir, mode `0600`, under the registry's lock-and-rename discipline.
   - The auth reply carries the server's version, and a client refuses a
     mismatch.
   - Every refusal closes the socket with nothing said: no body, no reason.
   - The CI witness is a CONNECT with the served case first, per the
     platform-measurement rules in `AGENTS.md`.

   Two consequences are open rather than settled. Refusing a missing `Origin`
   closes the door on a native, non-browser client until Phase 2 gives it a
   credential. And whether the loopback listener may ever enroll a device is
   not decided.

## Queued, not yet in a phase

- **Process-global statics in `worktrees-api`.** Under one engine per host, a
  `static` set from one client's preferences is silently shared by every
  client. Scrollback persistence, per-tab history and the fetch interval are
  set from the frontend's settings today. Each static needs a ruling: a
  process fact stays global; a client preference moves into the client's
  session. The caches (`PR_CACHE`, `PR_MERGED`, `STATUS_CACHE`, `CLONE_JOBS`)
  look like process facts. `viewer::ISSUED` is already §III.6's fourth item.
- **R3 has no phase.** §III.9 assigns every other OWASP item. The
  argument-injection invariant matters as soon as any client that is not the
  frontend's own dialogs reaches the git call sites, which a loopback client
  already does, so it belongs no later than Phase 2's remote bind.

## Part III — the server edition (the plan as written, 2026-10-15)

Goal: **one machine runs the engine; any of the user's devices drives it.**
The machine with the repos, the worktrees and the tmux server keeps being the
only machine that has them. An iPhone, an iPad, or a second Mac running
Worktrees gets the same product, not a cut-down status page.

### III.1 The seam already exists, and the mock has been using it for a year

This is the finding that decides the architecture.

`app/src/mock/install.ts:3304` assigns `window.__TAURI_INTERNALS__` — four
members, nothing more: `[v]`

```js
{ invoke, transformCallback, convertFileSrc, metadata }
```

With that object in place, **the real `App.tsx` runs unmodified in plain
Chrome**, against a stateful fake backend, driven headlessly by Playwright.
That is the daily development workflow, not a thought experiment.

So there is no refactor needed on the 20+ files that `import { invoke } from
"@tauri-apps/api/core"`. There is no chokepoint to introduce, because the
chokepoint is below them and is already load-bearing.

> **A server edition is that same object pointed at a network transport
> instead of at fixtures.**

The terminal has its own, equally convenient seam: `Transport` at
`app/src/TerminalPane.tsx:76` is a five-method interface
(`open`/`write`/`resize`/`close`/`wheel?`) with two existing implementations,
`tmuxTransport` (`:97`) and `shellTransport` (`:111`). `[v]` A third,
`remoteTransport`, slots in beside them.

Both seams were cut for other reasons — the mock for headless development, the
`Transport` for dock shells vs place terminals — and both happen to be exactly
the right shape. That is worth saying out loud, because it is the difference
between a quarter and a year.

### III.2 What the server actually is

```
  today    App.tsx --Tauri IPC--> app/src-tauri/lib.rs --> worktrees-core (in-process)
  server   App.tsx --WebSocket--> worktrees serve       --> worktrees-core (in-process)
                    (or iOS/iPad/browser client)
```

The engine does not move and does not change. What has to move is the **command
layer**, which today lives in the Tauri crate: `app/src-tauri/src/lib.rs` is
11,288 lines holding **143** `#[tauri::command]` handlers. `[v]` Nothing else in
the repo can call them.

#### The one real refactor: extract `worktrees-api`

Measured coupling of those 11,288 lines to Tauri types: `[v]`

| Type | Mentions |
|---|---|
| `AppHandle` | 60 |
| `State<…>` | 17 |
| `Channel<…>` | 5 |
| `tauri::Window` / `WebviewWindow` | 1 |

That is shallower than the line count suggests. `list_workspace(app:
AppHandle)` uses the handle for exactly one thing — `read_projects(&app)`, a
config-dir lookup. The pattern repeats.

So the extraction is mechanical, and bounded:

- `AppHandle` → a `Ctx { config_dir, events: Arc<dyn EventSink> }`.
- `State<T>` → fields on `Ctx`.
- `Channel<ArrayBuffer>` → `Arc<dyn ByteSink>`; Tauri's `Channel` and the
  server's WebSocket frame writer each implement it.
- The 1 `Window` is a genuinely client-local call (§III.4) and does not move.

`app/src-tauri/src/lib.rs` keeps 143 thin `#[tauri::command]` wrappers that
build a `Ctx` and call `worktrees_api::*`. The server calls the same functions.
**This refactor is the critical path and should be a PR of its own**, with the
app shipping unchanged on top of it before any socket is opened.

It also pays for itself independently of the server: 11,288 lines in a crate
whose tests do not run under `cargo check` (AGENTS.md: `check`/`build` do NOT
compile `mod tests`) becomes a library crate that does.

#### Why `worktrees serve`, not a new daemon binary

One engine is the repo's first architectural rule. `worktrees serve` is a
subcommand of the existing CLI, linking `worktrees-api` the way the app links
core. No third build artefact, no third place for the version to drift, and
`install.sh` already ships the binary to every machine that would host one.

Two launch hazards, both already documented for other reasons:

- **`fixup_gui_path()` exists because launchd gives a bare PATH** (no homebrew
  → no tmux). A launchd/systemd-launched server has precisely the same problem
  and must resolve the login-shell PATH at startup the same way.
- **A long-lived server outlives an upgrade.** AGENTS.md already records two
  days of debugging a "live" MCP bug that was a stale server answering after
  the fix shipped. `worktrees serve` must log its version on start and expose
  it in a handshake, or the same two days get spent again with more clients.

### III.3 The security inversion — read this before anything else

**The repo currently holds the opposite rule, written down and gated in CI.**

`app/src-tauri/src/docserver.rs` is 1,630 lines whose module header is six
numbered refusals, and whose premise is a loopback bind. The release workflow
runs its boundary tests by exact name and greps for `test result: ok. 4
passed`. AGENTS.md states the rule the witness guards as: *"reachable from the
LAN is a data leak"*.

A server edition deliberately reverses that. This plan is not allowed to do so
quietly.

What makes the reversal defensible is that **the docserver's rule was never
"never listen off-loopback"** — it was "this surface, which hands out the
user's documents behind a URL token, must not be reachable by anything the user
did not intend." The docserver could not express intent, so it used the
interface as a proxy for it. A server edition has to express intent directly,
and then the interface stops being the control.

Concretely, every one of docserver's six refusals has to be re-derived rather
than inherited:

| docserver refusal | Still right? | What replaces it |
|---|---|---|
| 1. `Host` must be loopback | **No** — reversed | Origin pinning + TLS cert identity. DNS rebinding is answered by TLS, not by `Host`. |
| 2. Non-matching `Origin` → 403, no CORS header ever | **Yes, harder** | Browser clients are now real. Keep: no CORS header, ever. |
| 3. Per-launch path token | **Insufficient** | A per-DEVICE credential that survives restart (§III.5). A per-launch token re-pairs every device on every restart. |
| 4. Nothing outside the generated tree | **Yes** | `safepath::safe_under`, unchanged — it is already shared between docserver and the CLI (`safepath.rs`). |
| 5. GET only, no body, no keep-alive | **No** — impossible | The whole surface is mutating RPC over a long-lived socket. Request smuggling is answered by using one well-tested HTTP/WS stack, not by refusing bodies. |
| 6. `Referrer-Policy: no-referrer` | **Yes** | Keep; cheap. |

> **2026-10-17 (decision 6):** this table is the REMOTE, TLS listener. The
> loopback listener keeps refusal 1: on plaintext loopback a `Host` check is
> the only defence against DNS rebinding, so it is not reversed there.

Refusal 5 is the uncomfortable one and should be named in the PR: the
docserver's safety came substantially from being a read-only, one-shot,
GET-only surface. The server edition is none of those things. The compensating
control is not a cleverer parser — it is that the surface is **authenticated
before it is parsed**, and the engine behind it already refuses what it should
(`reach.rs`, `USER_ONLY_KEYS`, `safe_under`, `remove_place`'s confirm).

**Recommended default: the server binds loopback and nothing else, out of the
box.** Remote reach is opt-in, per the registry's own precedent that
*registering is the consent act*. The two supported exposures:

1. **Overlay network (recommended).** Tailscale / WireGuard. The server binds
   the overlay interface; the overlay owns device identity, key rotation and
   NAT traversal; iOS has a first-class client. This is the configuration to
   document, test and recommend, because it removes TLS-certificate pain on
   iOS entirely and because the alternative is asking a developer to run a CA.
2. **Explicit LAN bind with TLS.** For users who will not run an overlay.
   Requires a real cert; a self-signed cert on iOS needs a profile install and
   is a support burden. Document it, do not recommend it.

Not supported: binding `0.0.0.0` with a bearer token and no TLS. There is no
configuration flag that should make that reachable.

**The CI witness has to change shape, carefully.** AGENTS.md records exactly
how this test went wrong before: an assertion about *binds* was measured on
macOS only and went red on Linux, and *"no bind means the same thing twice"* —
the witness has to be a **CONNECT**, and it must be preceded by a loopback
connect that succeeds, because without it a dead server passes every check. The
new witness is the same shape with the polarity flipped per mode: loopback
connect succeeds always; LAN connect is refused in default mode and accepted
only when the user turned it on. Measure on both platforms before writing it
down.

### III.3a OWASP review of this plan — required changes

Reviewed against OWASP Top 10:2025, the API Security Top 10, and the Agentic /
LLM lists (this is an agent-orchestration surface, so those apply). Done with
the `owasp-security` skill, on the plan as written at Part III.

**What the baseline already gets right, measured rather than assumed.** Three
findings were drafted and then RETRACTED after reading the code, which is worth
recording so a later reviewer does not re-raise them:

- Shell injection into the git/tmux shell-outs: git is invoked as an argv array
  (`Command::new("git").arg(…).args(…)`), never through a shell.
- `clone_project` as a remote RCE via `ext::`: `clone::parse_source` refuses
  every `<helper>::<address>` form with a comment that says why ("`ext::` is a
  command line"), allowlists schemes, refuses a leading `-`, and passes `--`;
  there is a fixture `("ext::sh -c touch% /tmp/pwned", None)`. `GIT_ALLOW_PROTOCOL`
  is set as a second belt, and the validation lives in the ENGINE, not the dialog.
- "Internal-only, so the surface is low risk": not claimed anywhere in the plan;
  §III.3 already names the reversal honestly.

Default-loopback, opt-in exposure, overlay-first, per-device credentials over a
per-launch token, no CORS header ever, refusing `update_cli` remotely, an
exhaustive `Exposure` enum, and revocation that kills live sockets are a
stronger starting posture than most. The changes below are what is MISSING.

#### R1 — Per-OBJECT authorization is undefined (A01, API1 BOLA) · blocking

`Exposure` answers *may this command be called remotely*. Nothing answers *on
which object*. Commands carry paths and `<project>:<slug>` addresses as
ARGUMENTS, and over a socket the arguments are the attack surface.
`safepath::safe_under` constrains a path to a root; it says nothing about which
roots a device may reach, and `reach.rs` reasons about cross-project addresses
for a LOCAL user who already has the filesystem.

**Change:** a device's capability names its SCOPE (which projects), and every
command taking a path or a `<project>:<slug>` resolves it server-side against
that scope before `safe_under` ever runs. Two checks, not one. ROADMAP's open
`resolve_rel` / `root.join(rel)` symlink items become remotely reachable the day
this ships and must close first.

#### R2 — Validation that is not in `worktrees-api` is validation a remote client skips (A01/A02) · blocking, and it lands in Phase 0

`parse_source` is engine-side, which is why clone is safe. That is not a
guarantee about the other 142 commands. The frontend demonstrably carries
decision logic — `NewPlaceDialog` reimplements `cmd_new`'s ordering, and
AGENTS.md records four cases wrong in its first version — and `cmd_*` argv
parsing in `ops.rs` is a second entry path with its own checks. **A remote
client is not the dialog and not argv.**

**Change:** Phase 0's exit criteria gain an inventory: every input check
currently performed in a Tauri wrapper, a frontend dialog, or a `cmd_*` argv
parser, with each item either moved into `worktrees-api` or consciously
re-asserted there. This is the cheapest moment to do it — the extraction is
already touching all 143 — and the most expensive to retrofit.

#### R3 — Write the argument-injection invariant down and test it (A05)

clone.rs is the model; nothing states the rule for everything else. A branch
name, slug, remote name or ref that reaches git beginning with `-` is a FLAG.

**Change:** state the invariant — every subprocess argument reachable from a
remote client is `--`-separated or validated — and add one test that walks the
engine's git/tmux call sites for it, in the `*-check.mjs` mirror-drift family.

#### R4 — Enrollment needs channel binding and a closed window (A04/A07)

§III.5's short code is exchanged over a channel the device cannot yet
authenticate. Without binding, an attacker reachable during the window races
the legitimate device; without limits, a short code is guessable.

**Change:** the QR carries the server's SPKI pin, so the device knows WHICH
server it enrolled with; the pairing listener exists only while pairing is
active; one failed attempt closes the window; the code is high-entropy in the
QR with the human-typed fallback rate-limited separately. Otherwise the overlay
network is silently carrying the entire channel-security burden, which §III.3
says it should not be asked to do alone.

#### R5 — No security audit log (A09)

The plan logs the server version on start and nothing else. This surface can
remove worktrees and delete branches from another device.

**Change:** a server-side append-only audit record — device id, command,
object, outcome — for enrollment, auth failure, revocation, capability denial
and every group-A mutation. Explicitly NOT `ui-events.jsonl`: §III.6.2 keeps
that file's "never leaves the machine" premise by not emitting usage events
remotely, and an audit log is a different file with a different premise.

#### R6 — No resource limits (A10, API4) 

Unbounded: sockets per device, frame size, live terminal channels, enrolled
devices, and request rate. `list_workspace` is a git fan-out (0.28s for one
project with nine worktrees, per AGENTS.md) that a client can call in a loop.

**Change:** per-device rate limit and concurrency cap, a maximum frame size, and
a bounded number of live terminal channels. Fail closed.

#### R7 — The server makes an agent-instruction surface remotely writable (ASI01/ASI02, LLM01) · the product-specific one

This plan's real novelty, and §III.4 does not name it. worktrees launches agents
against briefs; `new_place --brief`, `write_file`, and anything that writes
`.planning/` or `AGENTS.md` steer an agent that then runs with the user's full
local privileges. ADR 0001 ("a cloned repo never supplies argv") exists because
the repo already takes this seriously for repository content — the server
edition widens *who can supply* the text that becomes an agent's instructions,
which is the same axis.

**Change:** brief-setting, and writes under `.planning/` and to agent-instruction
files, require the SECOND capability (the group-C gate), never plain `full`.
This is also the strongest argument for refusing §V.2 outright.

#### R8 — The CI witness should cover identity, not only reachability

§III.3's connect-witness proves where the server listens. It does not prove who
it answers.

**Change:** add to the witness — an unenrolled device is refused, and a revoked
device's LIVE socket dies — and keep the §III.3 rule that the loopback connect
which must succeed runs first, or a dead server passes everything.

#### R9 — §V.5 is now blocking, not open

Every control above is written for one human with several devices. Multi-user
changes `devices.json`, per-place authorization, and per-user git identity.

**Change:** answer it before the auth schema is written. If the answer is no,
record it as a NON-GOAL the design may rely on.

### III.4 Triaging 143 commands

Not one surface. Four, and the split is the bulk of the design work:

**A. Engine commands — remote, the whole point.**
`list_workspace`, `list_places`, `new_place`, `remove_place`, `switch_place`,
`open_place`, `close_place`, `set_lifecycle`, `set_pin`, `set_note`,
`place_health`, `place_plan`, `changed_files`, `file_diff`, `read_file`,
`write_file`, `list_docs`, `project_prs`, `doctor`, `provision`, `sync_*`,
`rules_*`, `agent_*`, `claude_usage`, `codex_usage`, … — the large majority.

**B. Client-local commands — must execute on the DEVICE, never the server.**
`open_editor`, `open_terminal`, `detect_editors`, `copy_text`, `set_zoom`,
`set_title`, `open_docs_viewer`. A phone has no VS Code and the server's
clipboard is not yours. These stay in the Tauri shim and the remote client
either implements its own (clipboard, zoom) or **greys them out with a reason**
— silently doing nothing on the server is the worst available outcome.

**C. Host-management commands — server-side, and privileged.**
`install_app_version`, `update_cli`, `check_update`, `list_releases`,
`add_project`, `remove_project`, `clone_project`, `init_repo`,
`create_initial_commit`, `mcp_install`/`uninstall`, `codex_mcp_*`, `pi_mcp_*`,
`save_profile`, `delete_profile`. These change the *host machine's*
configuration. A remote client should need a second, explicit capability to
reach them, and `update_cli` from a phone — replacing the binary that is
serving the request — is a footgun that should simply be refused.

**D. Terminal/stream commands — remote, but the hard ones.**
`term_open`/`write`/`resize`/`close`/`wheel`, `shell_*`, and the new
`transcript_*`. §III.7.

The deliverable for this section is a **table in code, not prose**: one
`Exposure` enum per command, checked exhaustively so a new command cannot be
added without choosing. The precedent is `projcfg::USER_ONLY_KEYS` — a hard
parse error rather than a default — and `reach.rs`'s three-level model
(`off` / `read` / `full`, user-tier only, *a profile can NARROW it, never
widen it*). A remote device's capability should be the same shape and the same
file family, and `full` should not imply C.

### III.5 Enrollment and identity

The registry's own header states the principle to copy: **"Registering is the
consent act."** Enrolling a device is the same act, and belongs in the same
user-tier config directory (`~/.config/worktrees/`), under the same
lock-and-rename discipline the registry already uses (`registry.rs` — every
write is read-modify-write under a lock, because the app and the CLI can write
at the same moment).

Shape:

- `worktrees serve --pair` prints a short code and a QR, valid ~60s, one use.
- The device posts its public key; the server records
  `{ device_id, pubkey, name, capability, enrolled_at, last_seen }` in
  `devices.json`.
- From then on the device authenticates with its key. No passwords, no bearer
  token in a URL (the docserver's own §6 note about `Referer` leaking a
  URL-borne capability applies with more force to a credential that persists).
- The server lists enrolled devices and can revoke one. Revocation must kill
  live sockets, not just refuse the next handshake.
- Pairing is only possible from the console or an already-trusted device.

**Open question for the user (§V.1):** whether an overlay network's own
identity (a Tailscale ACL) is allowed to *stand in for* this, rather than sit
under it. It is less code and one fewer thing to get wrong; it is also a
dependency on a third party for the security boundary.

### III.6 What multi-client breaks — found in the code, not guessed

The declared-state spine is **already safe**. DESIGN.md's concurrency contract
is "Rust sole writer": `.worktrees.places.json` is written under an atomic
`mkdir` lock with a re-read and a per-field merge preserving unknown keys, and
*"UI never second-writes; it calls Rust setters"*. Two clients mutating two
different places is already a solved problem. This is a large piece of luck and
it is luck the repo earned.

Three things are not safe:

1. **`ui-state.json` is written WHOLE-BLOB by the frontend.** It holds the
   selected place, sidebar mode, sort, terminal visibility. Two clients = lost
   updates, and worse, two clients *fighting over the selected place*, which is
   not even shared state conceptually — my phone's selection is not my desktop's.
   The repo has already hit this one rung down and solved it the right way:
   `shell-cwds.json` and `ui-events.jsonl` are separate, backend-owned files
   *precisely because* "a backend write into [ui-state.json] is erased by the
   next settings save." The fix is the same move again: split **per-device view
   state** (selection, zoom, sidebar) from **shared preferences** (theme,
   profiles, quota settings). Per-device state should not reach the server at
   all.

2. **`ui-events.jsonl`'s premise is that "the file never leaves the machine."**
   That is written down in DESIGN.md as a privacy guarantee. With a remote
   client, usage events are generated on a device and would have to travel to
   be recorded. Either the guarantee is restated honestly, or remote clients do
   not emit usage events. **Recommendation: remote clients do not emit them.**
   The metric is not worth amending a privacy promise for.

3. **A permission prompt now has two audiences.** §II.3a.4 is a gap in the
   single-user case and a hazard in the multi-client one: if the phone and the
   desktop both render "Claude is asking something", both can answer, and the
   loser's tap lands on whatever the prompt became. The gate from §II.4 must
   extend to a **server-side lease**: one client holds the answer-token for a
   given pending question, the others see "being answered on <device>".

A fourth, smaller: `viewer::ISSUED` is a process-global `static`, per-launch by
design, and AGENTS.md already records it making unit tests order-dependent.
Per-launch is wrong for a long-lived server with reconnecting clients; it needs
the same `_in` seam the tests got.

### III.7 Transport

**One authenticated WebSocket per client, carrying both RPC and streams.**

- Request/response frames carry `{ id, cmd, args }` → `{ id, ok | err }`. That
  is `invoke`, exactly, and it is what `__TAURI_INTERNALS__.invoke` becomes.
- Channel frames carry `{ channel_id, seq, payload }`. Tauri's `Channel` is
  already an ordered, index-buffered stream (AGENTS.md: *"only the channel's
  own order (`index`-buffered in `@tauri-apps/api`) is reliable"*), so the
  client-side semantics are unchanged — `transformCallback` keeps doing what it
  does today.

> **As built (`feat/worktrees-serve`):** channel frames are
> `{channel, seq, data}`, where `data` is base64 for a byte stream and a JSON
> value for a typed progress callback. Events are `{event, payload}`, sent to
> every authenticated socket. The code is the reference for frame shapes.

Why one socket rather than HTTP + a separate stream: the ordering guarantee
above is load-bearing and already bit this codebase once (the replay-answered-
as-input bug), and reconstructing it across two connections is how you get it
wrong.

**Terminal bytes are the thing to measure before committing.** A tmux attach
over a WAN link is latency-visible in a way that a `list_workspace` is not, and
`term_resize` is already coalesced behind `RESIZE_SETTLE_MS` because *every
distinct grid handed to the pty is a SIGWINCH*. Over a network the settle
window probably has to grow, and the existing
`app/scripts/termresize-check.mjs` runs the real `useTerm` on a **virtual
clock** — so it can test a WAN-ish profile without becoming flaky. Use it.

**Reconnection** is the new state machine with no existing analogue: a dropped
socket must not look like a closed place. The dock shell's 256K ring and its
`── restored · … ──` seam are the closest prior art and the right model —
replay on reattach, with a visible seam, and `trim_trailing_seam` so three
reconnects do not stack three seams.

### III.8 Clients

> **Superseded 2026-10-17 (decision 3):** the phone is a Tauri iOS and iPadOS
> build distributed through TestFlight, not a PWA. The reasoning below about
> one frontend still holds: the iOS build runs the same frontend. The Web Push
> path does not, and notifications move to Phase 3's design.

**Recommendation: the mobile client is the existing frontend in a browser /
PWA, not a native app.**

Reasons, in order of weight:

1. It already runs in a plain browser. The mock proves it daily, in Chrome and
   in headless WebKit.
2. The app has **no UI library** — plain CSS over `tokens.css`, scaling off
   `--ui-rem`. There is no framework lock-in to port and no component library
   that would have to be rebuilt in SwiftUI.
3. A native client is a second implementation of 143 commands' worth of
   semantics, and the repo already has a written rule about what happens when a
   frontend mirrors a core decision (`dnd.ts::predictTier` needs
   `dnd-check.mjs` to stop it lying quietly). A whole second client is that
   hazard at scale.
4. Per the Part I thesis, the mobile surface that matters is the **conversation
   view**, which is markdown and cards — the browser's home turf.

What that costs: no push notifications without extra work, iOS Safari quirks,
and the terminal is worse than a native terminal app would be. The first is the
one that hurts — "Claude is waiting for you" is the single best reason to have
a phone client at all. Web Push on iOS requires an installed PWA; that is the
path, and it should be proved in Phase 2 rather than assumed.

The **second-desktop** client is easier: it is the same Tauri app with a
different `__TAURI_INTERNALS__`. The interesting design question there is
whether a remote server appears as a **separate workspace** or as more projects
in the existing nav. Recommendation: **more projects in the existing nav.**
`registry::Entry` is `{ root, name, private, planning, … }` keyed on a local
path; a remote entry adds an origin. The nav already renders N projects, the
cross-project model (`reach.rs`) already reasons about `<project>:<slug>`
addresses, and a user with a laptop and a desktop wants one tree, not two
windows.

### III.9 Phases

#### Phase 0 — `worktrees-api` extraction (no server, no socket)

Move the 143 handler bodies out of `app/src-tauri/src/lib.rs` into a new
`crates/worktrees-api`, leaving thin Tauri wrappers. `Ctx` replaces `AppHandle`
and `State`; `ByteSink` replaces `Channel`.

Exit: the app is byte-for-byte equivalent in behaviour; all gates green; the
mock harness untouched (it talks to `__TAURI_INTERNALS__`, which has not
moved). **This PR opens no port and adds no dependency.** It should merge on
its own merits, which it has.

Plus **R2**: an inventory of every input check that today lives in a Tauri
wrapper, a frontend dialog or a `cmd_*` argv parser rather than in the engine,
each item moved into `worktrees-api` or consciously re-asserted there. The
extraction is already touching all 143 — this is the cheapest moment, and the
most expensive to retrofit. `clone::parse_source` is the shape to copy: it is
engine-side, so it holds for a caller that is not the dialog.

#### Phase 1 — loopback server + browser client

`worktrees serve` binds 127.0.0.1 only. A dev-mode frontend build points
`__TAURI_INTERNALS__` at it. No auth beyond a per-launch token, no TLS, no
remote — this phase is about proving the RPC shape and the Channel mapping,
and it is testable with the same Playwright setup used for the mock.

Exit: drive the real app in Chrome against a real engine over a socket;
`list_workspace`, a mutation, and one terminal attach all work.

> **2026-10-17 (decisions 2, 5, 6):** Phase 1 also makes the desktop app a
> client of a local `worktrees serve`, which the shim finds or starts from the
> bundled sidecar. The per-launch token is as written; the loopback rule set
> is what guards it.

#### Phase 2 — identity, capability, exposure

Device enrollment (§III.5), the `Exposure` table (§III.4), the per-device /
shared settings split (§III.6.1), the new CI connect-witness (§III.3). Only
after this does anything bind off-loopback, and only behind the opt-in.

Exit: a second machine on an overlay network drives the server; an unenrolled
device cannot; a revoked device's live socket dies; both platforms measured.

Plus the OWASP gates that belong here, and nothing binds off-loopback until all
of them hold: **R1** per-object scope (a device's capability names its
projects, resolved server-side before `safe_under`), **R4** enrollment channel
binding (SPKI pin in the QR, window closed on first failure, pairing listener
only up while pairing), **R5** the audit log, **R6** rate and concurrency
limits, **R7** brief/`.planning/`/agent-instruction writes behind the second
capability, **R8** the witness covering identity as well as reachability, and
**R9** answered.

#### Phase 3 — the phone

PWA install, Web Push for "waiting on you", the conversation view (Part II
Phase 1) as the default surface on a narrow viewport, terminal available but
not primary, the answer-lease from §III.6.3.

#### Phase 4 — second desktop in one nav

Remote projects in `registry`, rendered in the existing tree.

---

### IV — combined order of work

```
  III.0  worktrees-api extraction      <- unblocks everything, merges alone
  II.0   transcript.rs tailer          <- independent, merges alone
  II.1   conversation view (local)     <- ships value with no server
  III.1  loopback server               <- proves the transport
  III.2  identity + exposure           <- the gate before any remote bind
  II.2   composer                      }  either order
  III.3  phone                         }
  II.3   interactive handoff + III.6.3 answer lease   <- do together, same problem
  II.4 / III.4  polish, second desktop
```

The two items worth starting in parallel are `III.0` and `II.0`: different
files, no shared surface, and each is independently shippable.

### V — open questions for the user

1. **Does an overlay network's identity stand in for device enrollment, or sit
   under it?** (§III.5) Tailscale ACLs alone = much less code and a third-party
   security boundary. Own device keys = more work, no dependency.
2. **Is `update_cli` / `install_app_version` from a remote client refused
   outright, or allowed behind a second capability?** (§III.4.C) Recommendation:
   refused.
3. **Is the phone a PWA or a native app?** (§III.8) Recommendation: PWA, and
   the push story proved in Phase 3 rather than assumed.
   **Answered 2026-10-17:** a Tauri iOS and iPadOS build (decision 3).
4. **Does a remote server appear as projects in the existing nav, or as a
   separate workspace?** (§III.8) Recommendation: projects in the nav.
5. **Multi-user, ever?** Everything above assumes one human with several
   devices. Multi-user is a different product (per-user git identity,
   per-user tmux, authorisation on every place) and should be explicitly out
   of scope or explicitly in it, now, because the `devices.json` schema differs.

### VI — risks

| Risk | Mitigation |
|---|---|
| **The transcript format is private and undocumented.** Fields seen here (`atis-latch`, `wireIngestContext`, `advisorModel`) are version churn. | Tolerant parse, `Unknown` arm, never hard-fail. Pin observed `version` values in the fixture corpus so a format break is a red test, not a support ticket. The terminal is always one click away — a broken chat view is cosmetic. |
| **The security inversion is the largest single risk in this plan.** | §III.3. Default loopback; opt-in exposure; overlay recommended; the CI witness is a CONNECT measured on both platforms; the `Exposure` table is exhaustive so a new command cannot default to reachable. |
| A stale/wrong `sessionId` shows the wrong conversation. | Assert the transcript's `cwd` matches the place path before rendering a row. Mismatch → refuse and say so. |
| `/clear` mid-view silently switches files. | Watch the probe's `sessionId`; on change, re-locate and show a "new session started" divider. |
| Two claude sessions in one place. | `agents_at` already ranks them. Session picker in the header when > 1. |
| Submit lands on a permission prompt. | §II.4's bracket-the-paste gate, plus §III.6.3's lease once there are two clients. |
| **The mock answers instantly and cannot express any of this.** | AGENTS.md: three v0.12.x bugs got through exactly here. Every phase needs a real run (`app/scripts/sandbox.sh --app`), and the server phases need a real *slow* link — the mock's microtask resolution is the opposite of a WAN. |
| A long-lived server outlives its upgrade and answers with old code. | Version in the handshake, logged on start. AGENTS.md's two-day MCP debugging session is the precedent. |
| The `worktrees-api` extraction silently changes behaviour across 143 commands. | Diff `ls --json` against the shipped binary (the repo's existing technique for exactly this); the mock harness is an unchanged consumer and is a second witness. |

### VII — alternatives considered

- **App owns `claude --print --output-format stream-json`.** Clean documented
  stream, real permission control via the SDK control protocol. Rejected: it
  breaks the tmux spine — the session would not survive app quit, would not be
  `tmux attach`-able, and would not be the object the CLI manages. Revisit only
  if the product decides app-owned sessions are a second, explicitly different
  kind of place.
- **Screen-scrape the pane into structure.** Rejected on the evidence of
  `draft_from_screen`: ~250 lines and a war-story test suite to recover one line.
- **A dock tab instead of a main-pane mode for the chat view.** Rejected: the
  conversation is the primary thing in a place, and the dock is 240–400px.
- **SSH + the CLI as the "server edition".** It genuinely covers some of this
  today and costs nothing. Rejected as the answer because it does not give a
  phone anything, and because the product being asked for is the app, not a
  terminal into the machine that runs it. Worth documenting as the thing to do
  until Phase 2 lands.
- **Rendering the desktop app remotely (VNC-alike).** Rejected: it is the
  terminal problem one level up — a 1440px layout on a 390px screen, and every
  interaction pays a round trip.

---

## Appendix A — the R2 validation inventory (Phase 0's exit criterion)

Every input check that lived in a Tauri wrapper, a frontend dialog, or a
`cmd_*` argv parser rather than in the engine. A check on the far side of the
IPC is a check a remote client skips, so each one is either MOVED into
`worktrees-api` or consciously re-asserted there.

`clone::parse_source` is the shape to copy: engine-side, so it holds for a
caller that is not the dialog.

### Moved first

| Check | Was | Now | Note |
|---|---|---|---|
| `guard_under_projects` | `app/src-tauri/src/lib.rs`, reachable only via a `#[tauri::command]` | `worktrees_api::projects` | THE containment boundary. Canonicalises and requires the result under a registered root. |
| `under_roots` | same | `worktrees_api::projects` | The same test against an already-canonical path, for symlink targets. |
| `project_roots` | same | `worktrees_api::projects` | Canonical roots, built once per listing. |
| `valid_project_name` | same | `worktrees_api::workspace` | Its own comment already said the frontend's copy "is never evidence". One path component; no `/`, no leading dot, no whitespace or control chars. |
| `expand_home` | same | `worktrees_api::workspace` | Leading `~` only — `~user` stays a literal directory name rather than becoming someone else's home. |
| `probe_dir` | same | `worktrees_api::workspace` | Not a refusal, but the dialog's evidence for which offer to make. |
| `log_event` level | same | `worktrees_api::meta` | Unknown level → `info`; message truncated to 4000. |
| `log_tail` bound | same | `worktrees_api::meta` | `min(2000)` lines. |

### Already engine-side (verified, no action)

- `clone::parse_source` — scheme allowlist, refuses every `helper::address`
  form, passes `--`, guards a leading `-`, sets `GIT_ALLOW_PROTOCOL`. The
  fixture `("ext::sh -c touch% /tmp/pwned", None)` is in core.
- `clone::plan_target` — refuses a target that already exists.
- `sync::valid_name` — the hub-directory name rule `valid_project_name` mirrors.
- `init.rs` — refuses absolute paths, `~` and `$` in `[[file]].path`.

### Moved in the rest of Phase 0

| Check | Was | Now | Note |
|---|---|---|---|
| `valid_token` / `valid_event` | `lib.rs`, behind `ui_events_append` | `worktrees_api::prefs` | Allowlists ASCII identifier characters for a metrics key, and validates a whole batch on the way in. `usage-check.mjs` guards both and now reads them where they live. |
| `UI_EVENTS_MAX` | same | `worktrees_api::prefs` | The cap on the events file. |
| `LIFECYCLE_LABELS` | same, behind `set_lifecycle` | `worktrees_api::ops` | The four labels a declared lifecycle may be. `dnd-check.mjs` mirrors it. |
| `known_harness` | same | `worktrees_api::ops` | Refuses a provider id that is not in `provider::PROVIDERS`. |
| `valid_tag` | same, behind `install_app_version` | `worktrees_api::status` | Tag shape. The INSTALL stays Tauri-side; the check that decides what is installable does not. |
| `TERM_PATHS_MAX` | same | `worktrees_api::files` | Caps a path-resolution batch. `mdpaths-check.mjs` mirrors it. |
| `DIFF_MAX` / `cap_patch` | same | `worktrees_api::files` | Caps a diff at a line boundary. |
| `DOC_MAX_BYTES`, `PLACE_KEY_MAX`, `place_key_ok`, `ASSET_EXTS`, `ASSET_CSP`, `safe_under` | `app/src-tauri/src/{viewer,docserver}.rs` | `worktrees_api::{viewer,docserver}` | The docs server's whole refusal surface, moved intact with its tests. |
| `log_event` level, `log_tail` bound | `lib.rs` | `worktrees_api::meta` | |

### Left in the Tauri shim, deliberately

Two handlers and one startup path. None is an input check.

- `install_app_version` — drives `tauri_plugin_updater`. §III.4.B client-local.
- `set_zoom` — WKWebView `setPageZoom`. A window has one; a socket does not.
- `winstate` — window geometry, for the same reason.

### Still open for Phase 2

**R1 is NOT addressed by this phase and nothing here should be read as
addressing it.** `guard_under_projects` answers "is this path under SOME
registered project", which is the right question for a single trusted desktop
user and the wrong one for a device whose capability names a subset of
projects. The per-object check belongs in front of it, resolved server-side,
and it is a Phase 2 exit criterion.
