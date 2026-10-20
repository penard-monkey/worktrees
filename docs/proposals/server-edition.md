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
   - The client and the server each prove they hold the token, and the token
     itself never crosses the wire: `hello` (client nonce), `challenge`
     (server nonce), `auth` (the client's HMAC-SHA256 over both), then
     `{ok, version, proof}` (the server's HMAC, under a different label). The
     client sends nothing else until the server's proof verifies, and the whole
     exchange must finish within a few seconds. The token is 256 bits, rotated
     every launch, and never in a URL. *(2026-10-19: this replaced a first
     frame of `{"auth": "<token>"}`. The OWASP review of `serve` as built
     showed that a process holding the port after `serve` died received the
     live token that way. That was finding H1.)*
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
7. **Codex permissions and pi trust are host policy, and only the engine reads
   them** (2026-10-19). They live on the host in `host-policy.json` and change
   only when someone clicks them in Settings. A client loading never sets them
   (OWASP review finding M2; Appendix B). The CLI and lanes created over MCP
   keep following the environment variables and `config.toml`, so an agent's
   lanes do not inherit a widening someone clicked in the app. When the host
   holds no app setting, the engine follows the same environment and config
   order the CLI always has. For someone whose config says `full`, that
   widens app-launched lanes compared with the old default push. Accepted:
   it is the documented order.

## Queued, not yet in a phase

- **Process-global statics** under one engine per host: ruled in Appendix B.
- **R3, closed in Phase 0** (2026-10-19, `fix/argv-positionals`, OWASP
  review finding M3). The invariant: a value a client sent is never read as a
  flag, by our parsers or by anything they spawn.
  1. `worktrees-api` reaches a core `cmd_*` only through `argv::Argv::call`.
     Its own flags are `&'static str`, a flag's value is glued on as one
     element (`--x=<v>`), and every client positional goes after `--`. A unit
     test fails on any other reference to a `cmd_*` in the crate.
  2. Every core parser a client can reach honours `--`
     (`worktrees_core::argv::split_options`).
  3. Every subprocess argument derived from a client value is one of:
     - after a `--`;
     - `=`-anchored or prefixed, so it cannot begin with `-`;
     - the value of an option that consumes it whole.

     Each is also validated at the boundary:
     - a branch with `branch_arg`: git's `check-ref-format --branch` rule,
       pinned to git by an oracle test, and applied after core strips
       `origin/`;
     - a base, slug or name with `safe_arg`: no leading `-`, no control
       characters, no `..`.

     The MCP server uses the same validators.

  Re-check when a command gains a string parameter, when a core function gains
  a subprocess call carrying a client value, or when a parser learns `--`. Core
  was safe only by accident while its parsers refused every `-`-led positional.
  Teaching them `--` alone would have let a base of
  `--upload-pack=<cmd>` reach a bare `git fetch` (measured), which is why
  core's git calls now carry their own `--` too.
- **The last client to load set Codex permissions and pi trust:** fixed in
  Phase 0, recorded as decision 7 and in Appendix B (`fix/host-policy`).
- **Outside R3, for Phase 2's per-object scopes.** Each item below is reachable
  only by an authenticated client, and an authenticated client can already
  type into a shell, so none of them escalates today. Each one stops being
  equivalent to shell access once R1 scopes exist:
  - `write_file` can overwrite `<project>/.git/config`, and `core.fsmonitor`
    runs on the next git spawn;
  - `save_profile` stores `mcp_servers`/`hooks` that run at the next launch;
  - `term_open.session` reaches `attach-session -t` unanchored;
  - the rsync destination in `sync.rs` is checked only for being non-empty;
  - a few containment gaps (`create_initial_commit("")`,
    `resolve_doc_paths.root`, a `..` slug in `shell_open`/`list_branches`).

  Found by the R3 lane; the list is in its report.

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

> **2026-10-18 (Appendix B):** Two corrections to that paragraph. `ISSUED` is
> never pruned, so a long life only grows it, and the `_in` seam already
> exists. And under one engine per host it stays in the desktop shim, because
> the docs viewer is client-local. Appendix B rules on every process-global
> static. It also adds a fifth break: a dock shell has one sink, so a second
> client's attach silences the first client's pane.

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

---

## Appendix B — process-global state under one engine per host

Decision 2 puts every client on one `worktrees serve`: the desktop shim, the
dev stack's browser, and later a phone and a second Mac. Anything global to
that process is shared by all of them. That covers every `static`, including
`OnceLock`, `LazyLock`, a `Mutex` static and an atomic. One level out, it also
covers the registries in `Ctx`, which are per process by construction. Each
static gets one ruling. A **host fact** describes the machine and stays
global. A **host policy** stays global, but only an explicit act may change
it, and it must be persisted on the host. A **client preference** moves into
the client's session. A **per-launch identity** needs a lifecycle. The rest
are **const-like** (written once, or a counter that only has to be unique) or
**test-only**.

**Method.** The audit is at `f308062`. The inventory command was
`git grep -n -P '(^|\s)static\s+[A-Z_][A-Z0-9_]*\s*:' -- crates/worktrees-api crates/worktrees-core/src app/src-tauri/src`,
which returns 61 lines with one static each. Every one was read, along with
its writers and readers. The three crates contain no `thread_local!`,
`lazy_static!` or `static mut`. Their only production writes to the process
environment are `hostenv`'s boot-time `PATH` and `LANG` repairs.
`worktrees-cli` adds one more static, `mcp.rs`'s test-only `ENV_LOCK`. Serve's
own state was read from `feat/worktrees-serve` at `495df95`, and it has no
statics. On the frontend, all 169 `useEffect`/`useLayoutEffect` bodies in
`app/src` (mock excluded) were scanned for `invoke`, and every call site of
every writing command was checked.

**Result:** 31 host facts, 5 host policies, 0 client preferences, 2 per-launch
identities, 23 const-like or test-only (17 + 6).

Three things follow.

1. **No static holds a client preference.** The queue had suspected three:
   the fetch interval, scrollback persistence and per-tab history. Each one
   configures a single host-owned object: one fetch loop, and one shell per
   dock tab that every client opening that tab shares. Two clients cannot
   each hold their own value, so all three are host policy. The client
   preferences live in `ui-state.json`, which is §III.6.1's problem.
2. **All five host policies are reached by the same defect.** An effect in
   `App.tsx` copies a value from the page's settings blob into an in-memory
   static, on load and on every change. That was harmless while the page and
   the engine started and died together. Under one engine per host, a page
   load is an act on the host, and the last page to load decides. The fix for
   the two security settings is in flight on `fix/host-policy` (the OWASP
   review of `serve`, finding M2), and the other three take the same shape
   (see "One fix for all five").
3. **Phase 1 needs one line from this appendix and one piece of wiring;
   everything else is Phase 2.** Phase 1 ships exactly one client. The app has
   one window, and the loopback listener refuses any `Origin` but the app's
   own. So the multi-client failures below are reachable only on the dev stack
   (two tabs) until Phase 2 adds a device. The exception is the
   terminal-history push. It needs no second client, only an engine that
   outlives the page, and decision 2's find-or-start makes that normal. The
   wiring is not a static: the docs viewer's refresh has to stay where its
   registrations are (see `Ctx` below).

### The table

| Static | Where | Ruling | Blocks |
|---|---|---|---|
| `log::SINK` | `crates/worktrees-api/src/log.rs:39` | const-like | — |
| `ops::FETCH_INTERVAL_SECS` | `crates/worktrees-api/src/ops.rs:817` | **host policy** | Phase 2 |
| `prefs::UI_EVENTS_LOCK` | `crates/worktrees-api/src/prefs.rs:204` | host fact | — |
| `project::PR_MERGED` | `crates/worktrees-api/src/project.rs:379` | host fact | — |
| `project::PR_CACHE` | `crates/worktrees-api/src/project.rs:780` | host fact | — |
| `projects::import_app_projects` `ONCE` | `crates/worktrees-api/src/projects.rs:16` | const-like | — |
| `status::STATUS_CACHE` | `crates/worktrees-api/src/status.rs:95` | host fact | — |
| `term::NEXT_ID` | `crates/worktrees-api/src/term.rs:44` | **per-launch identity** | Phase 2 |
| `term::CWD_FILE_LOCK` | `crates/worktrees-api/src/term.rs:697` | host fact | — |
| `term::TERM_HIST_LOCK` | `crates/worktrees-api/src/term.rs:962` | host fact | — |
| `term::PERSIST_SCROLLBACK` | `crates/worktrees-api/src/term.rs:968` | **host policy** | **Phase 1** (interim), Phase 2 |
| `term::PER_TAB_HISTORY` | `crates/worktrees-api/src/term.rs:969` | **host policy** | **Phase 1** (interim), Phase 2 |
| `term::UNSUPPORTED_SHELL_LOGGED` | `crates/worktrees-api/src/term.rs:1406` | const-like | — |
| `viewer::ISSUED` | `crates/worktrees-api/src/viewer.rs:415` | **per-launch identity** | — |
| `viewer::temp_beside` `N` | `crates/worktrees-api/src/viewer.rs:919` | const-like | — |
| `viewer::tests::rt` `RT` | `crates/worktrees-api/src/viewer.rs:2175` | test-only | — |
| `workspace::CLONE_JOBS` | `crates/worktrees-api/src/workspace.rs:429` | host fact | — |
| shim `IDENT` | `app/src-tauri/src/lib.rs:78` | const-like | — |
| shim `AFFORDANCE` | `app/src-tauri/src/lib.rs:1409` | const-like | — |
| `activity::CODEX_TAIL` | `crates/worktrees-core/src/activity.rs:189` | host fact | — |
| `agentfiles::CLASSIFIED` | `crates/worktrees-core/src/agentfiles.rs:314` | host fact | — |
| `agentfiles::publish` `SEQ` | `crates/worktrees-core/src/agentfiles.rs:515` | const-like | — |
| `claude_usage::USAGE_CACHE` | `crates/worktrees-core/src/claude_usage.rs:94` | host fact | — |
| `claude_usage::USAGE_FAIL_AT` | `crates/worktrees-core/src/claude_usage.rs:96` | host fact | — |
| `clone::CLI_CANCEL` | `crates/worktrees-core/src/clone.rs:659` | const-like | — |
| `codex::THREAD_CWD` | `crates/worktrees-core/src/codex.rs:136` | host fact | — |
| `codex::PERMISSIONS_OVERRIDE` | `crates/worktrees-core/src/codex.rs:214` | **host policy** | Phase 2; fix in flight: `fix/host-policy` |
| `codex_usage::CACHE` | `crates/worktrees-core/src/codex_usage.rs:502` | host fact | — |
| `codex_usage::tests` `SEQ` | `crates/worktrees-core/src/codex_usage.rs:814` | test-only | — |
| `config::USER_CONFIG_LOCK` | `crates/worktrees-core/src/config.rs:97` | host fact | — |
| `git::have_git` `HAVE` | `crates/worktrees-core/src/git.rs:62` | host fact | — |
| `git::STDOUT_IS_PROTOCOL` | `crates/worktrees-core/src/git.rs:78` | const-like | — |
| `github::ssh_host` `CACHE` | `crates/worktrees-core/src/github.rs:258` | host fact | — |
| `inbox::tests` `HOME_LOCK` | `crates/worktrees-core/src/inbox.rs:296` | test-only | — |
| `logsink::LOG` | `crates/worktrees-core/src/logsink.rs:20` | const-like | — |
| `mcpmigrate::apply` `APPLY` | `crates/worktrees-core/src/mcpmigrate.rs:366` | host fact | — |
| `messages::SEQ` | `crates/worktrees-core/src/messages.rs:104` | const-like | — |
| `pi::header` `HEADERS` | `crates/worktrees-core/src/pi.rs:140` | host fact | — |
| `pi::PI_TAIL` | `crates/worktrees-core/src/pi.rs:309` | host fact | — |
| `pimodels::REACH` | `crates/worktrees-core/src/pimodels.rs:203` | host fact | — |
| `pimodels::CATALOG` | `crates/worktrees-core/src/pimodels.rs:319` | host fact | — |
| `pimodels::PREFLIGHT` | `crates/worktrees-core/src/pimodels.rs:526` | host fact | — |
| `profile::WRITE_LOCK` | `crates/worktrees-core/src/profile.rs:68` | host fact | — |
| `profile::write_atomic_bytes` `SEQ` | `crates/worktrees-core/src/profile.rs:824` | const-like | — |
| `profile::materialize_with` `MAT_LOCK` | `crates/worktrees-core/src/profile.rs:889` | host fact | — |
| `provision::tests` `NEXT_BASE` | `crates/worktrees-core/src/provision.rs:904` | test-only | — |
| `quota::tests` `LOCK` | `crates/worktrees-core/src/quota.rs:1015` | test-only | — |
| `registry::WRITE_LOCK` | `crates/worktrees-core/src/registry.rs:43` | host fact | — |
| `rules::INSPECTED` | `crates/worktrees-core/src/rules.rs:374` | host fact | — |
| `slots::CHILD` | `crates/worktrees-core/src/slots.rs:614` | const-like | — |
| `slots::PENDING` | `crates/worktrees-core/src/slots.rs:616` | const-like | — |
| `slots::INTS` | `crates/worktrees-core/src/slots.rs:617` | const-like | — |
| `store::WRITE_LOCK` | `crates/worktrees-core/src/store.rs:27` | host fact | — |
| `sync::write_exclude_file` `SEQ` | `crates/worktrees-core/src/sync.rs:697` | const-like | — |
| `sysclock::birth_cache` `C` | `crates/worktrees-core/src/sysclock.rs:32` | host fact | — |
| `sysclock::date_cache` `C` | `crates/worktrees-core/src/sysclock.rs:42` | host fact | — |
| `sysclock::SysClock::detect` `PROBE` | `crates/worktrees-core/src/sysclock.rs:51` | host fact | — |
| `tmux::NO_START` | `crates/worktrees-core/src/tmux.rs:84` | host fact | — |
| `tmux_server::publish` `NEXT` | `crates/worktrees-core/src/tmux_server.rs:172` | const-like | — |
| `tmux_server::tests` `NEXT` | `crates/worktrees-core/src/tmux_server.rs:272` | test-only | — |
| `trust::PI_TRUST_OVERRIDE` | `crates/worktrees-core/src/trust.rs:59` | **host policy** | Phase 2; fix in flight: `fix/host-policy` |

### The host policies

#### One fix for all five

The fix moves the value onto the host and stops the page from pushing it.

**Codex permissions and pi trust: fix in flight on `fix/host-policy`.** That
lane is the OWASP review's finding M2, and this appendix does not propose a
second design for it. As briefed:

- The value is persisted in a backend-owned file in the engine's config dir
  (`Ctx::config_dir`), written under a lock and a rename, and never inside
  `ui-state.json`, whose whole-blob writer is the frontend.
- Only the Settings click calls the setter. The load effects at
  `App.tsx:4727` and `4732` are deleted, and Settings renders the host's
  value, not its own copy.
- The value already in `ui-state.json` is migrated once, at boot, as a host
  act. It never moves because a client loaded.
- Precedence and scope stay as they are: the host's setting, then the env var,
  then the user config, then the default; and only the engine process reads
  the host's setting.

Serve already tags both setters `HostAdmin`
(`feat/worktrees-serve:crates/worktrees-cli/src/serve/dispatch.rs:274`,
`:282`). Once the push is gone, that tag means what it says: a device without
the capability cannot change them, instead of failing on every load.

**The other three take the same shape.**

- **Where it is stored.** The fetch cadence and the two terminal switches
  matter only to the engine, so they belong in the same kind of backend-owned
  file, next to `shell-cwds.json` and for the same reason. Their statics can
  stay as the in-memory copy, but boot loads them and the setter is their only
  writer.
- **What the setter does.** It persists, and the control renders what the
  host stored rather than what was clicked last. Serve tags both setters
  `Engine` (`dispatch.rs:215`, `:355`). Once they persist host policy,
  `HostAdmin` is the honest tag.
- **What the page stops doing.** The effects at `App.tsx:4663` and `4673` are
  deleted. The three `Settings` keys go the way `fix/host-policy` takes its
  two, so all five stay alike, and `fix/host-policy`'s boot migration can
  carry three more keys.
- **Phase 1 interim.** Gate `set_fetch_interval` and `set_term_history_opts` on
  `hydratedTick`, as the Codex and pi effects already are. That is one line
  each, and it removes the pre-hydration default push that a long-lived engine
  makes meaningful.

#### `codex::PERMISSIONS_OVERRIDE` — `crates/worktrees-core/src/codex.rs:214`

`Mutex<Option<Permissions>>`. It holds Settings → Codex → Permissions, which
is one of three modes: `ask`, `auto-review` (`--approve-for-me` inside the
workspace-write sandbox, with the git common dir writable and network on), or
`full` (`--dangerously-bypass-approvals-and-sandbox`). Found by the transport
lane driving `serve` on 2026-10-18.

- **Writer.** Only `set_permissions_override` (`codex.rs:216`) writes it, and
  only `project::set_codex_permissions`
  (`crates/worktrees-api/src/project.rs:566`) calls that. The page invokes it
  from the effect at `App.tsx:4727`, after hydration and on every change of
  `settings.codex_permissions`.
- **Reader.** `codex::permissions()` (`codex.rs:223`), where it outranks
  `$WORKTREES_CODEX_PERMISSIONS` and the user config. That feeds
  `codex::launch_flags` (`codex.rs:291`) through `harness.rs:405` and
  `ops::ai_launch_inner` (`ops.rs:166`), so it decides every Codex launch the
  process makes.

**Ruling:** host policy. It widens what an agent may do on the host, and
opening a page is enough to set it.

**The failure.** It needs two clients, so today only two tabs on the dev
stack reach it; in Phase 2 any second device does. It is the §III.6.1
lost-update bug turned into a widening:

1. Clients A and B have both loaded `full`.
2. On A, the user narrows to `ask`. The host override becomes `Ask`, and A
   saves the whole blob with `ask`.
3. On B, the user does anything that saves settings: switches a dock tab
   (`App.tsx:5013`), resizes the dock, or picks a theme. `updateSettings`
   saves the whole blob (`App.tsx:4747`), and B's blob still says `full`, so
   the file says `full` again. The override is still `Ask`, so nothing visible
   changes.
4. The next client to load reads `full` and pushes it. That could be B
   reloading, the app restarting or the phone opening. From then on, every
   Codex lane the host launches runs with
   `--dangerously-bypass-approvals-and-sandbox`. Nobody chose that after step
   2, and A's Settings still says `ask`.

Phase 2 adds two more routes. First, a §III.6.1 split that files this key with
per-device state makes the phone's value the host's every time the phone
opens. Second, once `HostAdmin` needs a capability, a device without it fails
this push on every load and shows "Codex permissions: …" each time. Serve
also runs each request on its own blocking-pool task
(`serve/conn.rs:276`), so two quick clicks are not even guaranteed to land in
order. The control has to show what the host stored, not what was clicked
last.

**Fix in flight: `fix/host-policy`** (see "One fix for all five"). It keeps
scope as it is, so a split that exists today with a single client stays open:
the setting is read only by the engine process, so MCP `create_worktree` and
`worktrees new --ai codex`, which run in other processes, launch from the env
or config tier and never see Settings → Codex at all. Whether they should is
the open question that lane reports, because honouring the setting there
widens what an MCP-created Codex lane may do.

**Blocks:** Phase 2.

#### `trust::PI_TRUST_OVERRIDE` — `crates/worktrees-core/src/trust.rs:59`

`Mutex<Option<PiTrust>>`. It holds Settings → pi → project trust. `never`
launches pi with `--no-approve`, so a repo's `.pi/extensions` (TypeScript
that pi runs in its own process), its `.pi/settings.json` and its skills do
not load. `ask` passes neither flag, which leaves the decision to pi's own
modal, whose highlighted choice is Trust.

- **Writer.** Only `set_pi_trust_override` (`trust.rs:63`) writes it, and
  only `project::set_pi_trust` (`project.rs:619`) calls that. The page invokes
  it from the effect at `App.tsx:4732`, after hydration and on every change.
- **Reader.** `trust::pi_trust()` (`trust.rs:70`), above
  `$WORKTREES_PI_PROJECT_TRUST` and the user config. `pi_launch_trust`
  (`trust.rs:168`) uses it for every pi launch (`ops.rs:176`), and
  `pi_status` and `doctor --pi` report it.

**Ruling:** host policy.

**The failure:** the Codex sequence with `never` in place of `ask` and `ask`
in place of `full`. A stale client's whole-blob save, followed by any load,
reaches every pi lane whose repo is neither in the allowance nor named in
pi's own `trust.json` (`pi_launch_trust_for`, `trust.rs:148`). Each of those
lanes then opens on a prompt whose highlighted choice runs the repo's own
code. The same split with MCP and the CLI exists today.

**Fix in flight: `fix/host-policy`**, the same change as Codex's, with the
same scope question for MCP and the CLI. The per-repo allowance (`[trust] pi`
in config.toml) needs nothing: it is already on the host, and only an
explicit act changes it (`trust::set_allowed`, from Settings → pi or
`worktrees trust pi`).

**Blocks:** Phase 2.

#### `term::PERSIST_SCROLLBACK` — `crates/worktrees-api/src/term.rs:968`

`AtomicBool`, which starts `true`. It holds whether each dock shell's 256K
ring is written to `<config>/term-history/<tab>/scrollback.bin` and replayed
on the next run. The setting's own hint calls this "the only thing that
writes a terminal's output to disk — whatever a command printed, secrets
included".

- **Writer.** Only `prefs::set_term_history_opts` (`prefs.rs:29`) writes it.
  The page calls it from the effect at `App.tsx:4673` on mount, with the
  default `true` and before hydration, then again after hydration and on
  every change.
- **Reader.** Three places: `save_shell_scrollback` (`term.rs:1229`), the
  poll's 15 s flush of every dirty shell on the host; `flush_one_scrollback`
  (`term.rs:1255`); and the spawn path in `shell_open` (`term.rs:493`).

**Ruling:** host policy. The shells belong to the host, and any client that
opens a tab attaches to the same shell. So this setting decides what the host
keeps on disk about every client's terminals.

**The failure (Phase 1).** The effect's comment calls the pre-hydration push a
no-op because the backend also starts at `true`. That holds only while the
page and the engine start together. Under one engine per host, a page can
load into an engine that is already running: a reload, or an app launch that
finds a running `serve`, which is what decision 2's "find or start" is for.
If that engine holds live shells and the user has turned this off, the flag
is `true` from the default push until the hydrated push lands. A 15 s flush
that falls inside that window writes every dirty shell's ring to disk. That
is content the user asked not to keep, and it stays until the tab is closed,
the 30-day sweep runs (`TERM_HIST_MAX_AGE_SECS`) or the user clears saved
terminal history. If `get_settings` fails, `loadSettings` answers with
DEFAULTS, and the flag stays on until the next load. In Phase 2 the window
lasts a WAN round trip. The same hint's "A dock shell dies with the app"
stops being true once `serve` outlives the app.

**Fix:** the Phase 1 interim (gate the effect on `hydratedTick`), then the
rest of "One fix for all five".

**Blocks:** Phase 1 (the interim line), Phase 2 (the rest).

#### `term::PER_TAB_HISTORY` — `crates/worktrees-api/src/term.rs:969`

`AtomicBool`, which starts `true`. It holds whether a newly spawned dock shell
gets its own command history under `term-history`. For zsh that means a
generated `ZDOTDIR` with four shims; for bash, `HISTFILE`.

- **Writer:** the same effect and setter as `PERSIST_SCROLLBACK`.
- **Reader:** `shell_open`'s spawn path reads it once per shell
  (`term.rs:492`).

**Ruling:** host policy, for the same reason. It decides how a shell that
every client shares is started, including which rc chain zsh runs.

**The failure:** the same window. A tab spawned inside it, by this client or
another, gets a `ZDOTDIR` and a history file that the user turned off, and
keeps them for the shell's whole life. The setting's own hint gives the cost:
"Turn it off if a shell setup of yours depends on $ZDOTDIR".

**Fix and blocks:** as `PERSIST_SCROLLBACK`. It is one setter and one effect.

#### `ops::FETCH_INTERVAL_SECS` — `crates/worktrees-api/src/ops.rs:817`

`AtomicU64`, in seconds, where 0 means off. It holds the auto-fetch cadence
from Settings → Behavior.

- **Writer.** Only `set_fetch_interval` (`ops.rs:821`) writes it. The page
  calls it from the effect at `App.tsx:4663` on mount, with the default `0`
  and before hydration, then again after hydration and on every change.
- **Reader.** The engine's 3 s poll (`lib.rs:1538` today, `host::poll` on
  `feat/worktrees-serve`). When the interval has elapsed, it runs
  `git fetch origin` for every registered project.

**Ruling:** host policy. One loop fetches for the whole host, so there is
nothing per-client to give it. It does not widen what an agent may do, but it
is network and credential use on the host.

**The failure.** Each page load turns auto-fetch off for one round trip, which
is harmless: at worst one tick is skipped. With two clients whose blobs
disagree, the last one to load sets the host's cadence. That happens through
the stale whole-blob write described under the Codex override, or through
per-device settings in Phase 2.

**Fix:** "One fix for all five".

**Blocks:** Phase 2.

### The per-launch identities

#### `term::NEXT_ID` — `crates/worktrees-api/src/term.rs:44`

`AtomicU32`, counting from 1. It mints the id of every tmux attach
(`term_open`, `term.rs:87`). The attach itself lives in `Ctx.terminals`.

- **Writer:** every client's place pane, through `term_open`.
- **Reader:** `term_write`, `term_resize`, `term_wheel` and `term_close`,
  which address the attach by that id alone (serve's rows are at
  `dispatch.rs:345`–`351`).

**Ruling:** per-launch identity. The id is a handle that is valid for one
engine launch, and the engine trusts it from any caller.

**The failure.** When a connection ends, serve closes the attaches it opened
(`Frames::take_terms`, `serve/conn.rs:297`). Nothing, however, checks that
the caller of `term_write` opened the id it names. The ids are also small
integers that every launch hands out again from 1. Once clients reconnect
across an engine restart, as the phone will in Phase 2, a pane still holding
an id from the old launch types into whichever attach the new launch gave
that number. That can be another device's attach, on another place's agent.
It is latent today because the transport's client never reconnects: after a
close, every later call is rejected.

**Fix:** record the owning connection with each attach, and refuse a
`term_*` call for an id the caller did not open. The per-connection set
already exists in `Frames`. Also make ids unguessable, or tag them with a
launch nonce, so a stale id fails loudly instead of landing somewhere. No
test asserts an id, so this one needs no `_in` seam.

**Blocks:** Phase 2.

#### `viewer::ISSUED` — `crates/worktrees-api/src/viewer.rs:415`

`Mutex<BTreeMap<String, PathBuf>>`. It holds every docs-viewer route segment
issued in the process, mapped to the place root it was issued to. By design
it is never pruned: freeing a segment once let a removed place's open tab
poll another project's documents.

- **Writer and reader.** Only `place_key` (`viewer.rs:354`) touches it. Its
  only caller is `viewer::open` (`viewer.rs:1168`), reached from
  `files::open_docs_viewer` (`files.rs:893`) and the shim's
  `open_docs_viewer` command. Serve's table marks that command `ClientLocal`
  and refuses it (`dispatch.rs:315`).

**Ruling:** per-launch identity. This corrects §III.6's fourth item twice:

- Under one engine per host, `ISSUED` never lives in serve. The docs viewer
  opens a browser on the client's machine, so the map stays in the desktop
  shim's process, as it does today.
- "Per-launch" is not wrong for a long-lived process. Because claims are
  never pruned, a long life only means a map that grows by one entry per
  place ever opened, and the old URLs die with the docs server's token anyway.

The `_in` seam that §III.6 asks for also already exists (`place_key_in`,
`claim_in`). It was added for the order-dependent tests AGENTS.md records.

**Fix:** what remains is tidying. Move the map into `Viewer`, next to `srv`,
so its lifetime is the docs server's rather than the process's, and the
`open()` tests stop sharing a global.

**Blocks:** nothing.

### Const-like and test-only (23)

Each of these is written once, is a counter whose only job is uniqueness, or
belongs to a process that is not the engine:

- `log::SINK`, `logsink::LOG`: the host's log destination, installed once by
  `host::boot`. The first install wins.
- `projects::import_app_projects`'s `ONCE`: the one-way merge of the app's
  pre-registry `projects.json`, done once per process.
- `term::UNSUPPORTED_SHELL_LOGGED`: logs once that `$SHELL` has no per-tab
  history scheme.
- Temp-name counters: `viewer::temp_beside`'s `N`, `agentfiles::publish`'s
  `SEQ`, `profile::write_atomic_bytes`'s `SEQ`, `sync::write_exclude_file`'s
  `SEQ` and `tmux_server::publish`'s `NEXT`. Each pairs with the pid, so it is
  unique per host. `messages::SEQ` does the same job for message ids.
- The shim's `IDENT` and `AFFORDANCE`: the identifier the window process logs
  under, and wry's Writing Tools override. Both are client-side and never
  reach serve.
- CLI-only state: `clone::CLI_CANCEL` (SIGINT during `worktrees clone`; serve
  cancels through `CLONE_JOBS`), `slots::CHILD`, `PENDING` and `INTS` (signal
  forwarding for `worktrees run`), and `git::STDOUT_IS_PROTOCOL`, which only
  `worktrees mcp` sets because its stdout is JSON-RPC. Serve's protocol is a
  socket, so it rightly leaves the flag unset. But core's `git_status`
  inherits stdio, so whatever starts serve must not hand it a stdout pipe that
  nobody drains.
- Test-only, inside a `mod tests`: `viewer.rs:2175` `RT`, `codex_usage.rs:814`
  `SEQ`, `inbox.rs:296` `HOME_LOCK`, `provision.rs:904` `NEXT_BASE`,
  `quota.rs:1015` `LOCK` and `tmux_server.rs:272` `NEXT`. Outside the count,
  `crates/worktrees-cli/src/mcp.rs:3517` `ENV_LOCK` is test-only too.

### Host facts worth a sentence

The other 31 are caches of git, tmux, GitHub, the status page, the harnesses'
own files and the host's tools, plus the registries of host processes and the
write locks. All of them stay global. Three caveats:

- **One engine per host makes the write locks stronger.** `store`,
  `registry`, `profile`, `MAT_LOCK`, `mcpmigrate::APPLY`, `CWD_FILE_LOCK`,
  `TERM_HIST_LOCK` and `UI_EVENTS_LOCK` each serialise writers within one
  process, and now every client's writes are in that one process. Against
  the CLI and the MCP servers, which are separate processes, the first five
  rely on a file lock that sits beside the mutex. The three engine-only files
  have no other writer. `USER_CONFIG_LOCK` is the exception: no file lock
  sits beside it, so an engine write to `config.toml` (`[quota]`, the reach
  level, `[trust] pi`) and a CLI write such as `worktrees trust pi` can still
  lose one of the two edits. That gap predates `serve` and lies between
  processes, so one engine per host neither opens nor closes it.
- **The caches that never evict now live as long as serve.**
  `codex::THREAD_CWD`, `pi::header`'s `HEADERS`, `activity::CODEX_TAIL`,
  `pi::PI_TAIL` and the two `sysclock` memos are keyed by path, inode or
  epoch. They stay correct for any lifetime, and they grow with every file
  they ever see. Their lifetime used to be one app launch and is now serve's
  uptime. That is not a defect at today's sizes, but it is a property to keep
  in mind.
- **`sysclock::date_cache` formats in the host's local time.** Its own comment
  accepts a date that is "a day off until restart" after a time-zone change,
  and "until restart" now means until serve restarts. A Phase 2 device in
  another zone reads host-local dates. `ui_usage` already takes the client's
  `tz_offset_min`, which is the right shape for anything a device displays.

### Not statics, same question: `Ctx` and serve's own state

`Ctx`'s registries are per process by construction (its doc comment in
`crates/worktrees-api/src/lib.rs` says so), so under one engine per host they
are as shared as any static:

- `watch` (the nav-dot poll's watches and model cache) and `docnames` are
  host facts.
- `terminals` holds per-connection objects in a host-wide map. See
  `term::NEXT_ID`.
- `shells` holds host objects keyed by `(repo, slug, index)`, so two clients
  that open the same tab get the same shell. That part is right, as it is for
  tmux. The problem is that a shell has one sink. `shell_open` on a live shell
  swaps that sink (`term.rs:438`–`462`), so a second client's attach silences
  the first client's pane without a word, while the first client's
  keystrokes still reach the shell. The second client's resize also resizes
  the pty under the first. This is a fifth item for §III.6, and it blocks
  Phase 2. The fix shape is a sink per attach, with one attach at a time
  owning the size, which is what tmux's `window-size` does.
- `viewer` is filled by the shim's `open_docs_viewer`, which is client-local,
  while `viewer::refresh` runs in `host::poll`
  (`feat/worktrees-serve:crates/worktrees-api/src/host.rs:212`) over the
  host's `Ctx`. Today those are the same process. Once the shim stops running
  `host::start`, they are two, and nothing refreshes the documents a browser
  tab is showing. Whoever makes the shim a client has to keep a refresh where
  the registrations are.

Serve's own state on `feat/worktrees-serve` at `495df95` contains no statics:

- `Shared` (`serve/conn.rs:70`): one per listener. It holds the engine's
  `Ctx`, the launch token, the port, the Origin allowlist, the client set, and
  `next_id` for connection ids. This is per-launch identity held by an
  object, the shape the two per-launch statics above should move toward.
- `Clients` (`conn.rs:45`): each authenticated connection's queue.
  `Broadcast::emit` sends every event to every client. That includes the
  inbox's `app:open-doc`, so once a second device is connected, one
  `worktrees show` opens the document on every device.
- Per connection: `Call { ctx, frames }`. `Frames` (`serve/wire.rs:131`) holds
  the connection's channels and the attaches it opened, and closes them on
  disconnect. This is the per-connection session the question asks for, and
  it already exists. If a client preference ever has to live in the engine,
  it lives here, or in the device's enrollment record (§III.5) when it has to
  survive a reconnect. Nothing found in this audit needs either.

### The load-time setters

Every `invoke` in `app/src` that writes state from an effect, on load, on a
settings change, or on a trigger that belongs to one device. Every other
`invoke` in the 169 effect bodies is a read, and every other call site of a
writing command is a click handler.

| Call site | Command | When | Writes | Ruling |
|---|---|---|---|---|
| `App.tsx:4663` | `set_fetch_interval` | on mount with the default `0`, then after hydration and on every change | `ops::FETCH_INTERVAL_SECS` | host policy: delete; Phase 1 interim, gate on `hydratedTick` |
| `App.tsx:4673` | `set_term_history_opts` | on mount with the defaults `true, true`, then after hydration and on every change | `term::PERSIST_SCROLLBACK`, `term::PER_TAB_HISTORY` | host policy: **Phase 1, gate on `hydratedTick`**; then delete |
| `App.tsx:4727` | `set_codex_permissions` | after hydration and on every change | `codex::PERMISSIONS_OVERRIDE` | host policy: delete; fix in flight: `fix/host-policy` |
| `App.tsx:4732` | `set_pi_trust` | after hydration and on every change | `trust::PI_TRUST_OVERRIDE` | host policy: delete; fix in flight: `fix/host-policy` |
| `App.tsx:5021` → `settings.ts:558` | `set_zoom` | after hydration and when the zoom changes | WKWebView page zoom | client-local and correct: serve refuses it (`ClientLocal`), and the shim keeps it |
| `App.tsx:4686` (`useLayoutEffect`) → `settings.ts:839`, `App.tsx:4697`, `4706`, `4713` | `set_settings` | on load: a `settings_rev` migration, a pre-hydration patch, or a new `last_seen_version` | `ui-state.json`, whole | §III.6.1. `last_seen_version` belongs to the device: the first client to load after an upgrade uses up "What's new" for all of them |
| `App.tsx:5116` → `ack` → `App.tsx:4420` | `mark_seen` | a selected place stays visible for `SEEN_DWELL_MS` | `last_seen_epoch` in `.worktrees.places.json`, forward-only, at the client's clock | shared, since one person reads it, but triggered by one device. A device left open on a place acks every completion there for all of them, and a device whose clock runs ahead stamps a future epoch that forward-only keeps. The host should stamp its own `now`. Phase 2 |
| `App.tsx:5045` | `open_place` | a selected place has two live agents (state left by an older build) | closes whichever agent is not `settings.default_provider` | a destructive act decided by whichever client selects the place first, using its own default. Phase 2, rare |
| `usage.ts:284` → `usage.ts:179` | `ui_events_append` | a timer, plus on hide and unload | `ui-events.jsonl` | §III.6.2 |
| `App.tsx:2473`, `4619`, `4621` | `log_event` | visibility changes, window errors | the host's log | host fact. Every client's lines interleave unlabeled, so R5's audit log should carry the connection |
