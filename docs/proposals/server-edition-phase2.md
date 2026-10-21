---
title: "Proposal — Server Edition, Phase 2"
---

# Proposal — Server Edition, Phase 2: identity, capability, exposure

**Status:** design only, 2026-10-20. Nothing here is built. It is the gate
before `worktrees serve` answers anything but this machine: a second desktop,
and the Tauri iOS and iPadOS build (decision 3 of
[server-edition.md](server-edition.md)). Nothing binds off loopback until
every exit criterion in §10 holds.

**Base.** Written against `feat/app-serve-client` at `fadb2ce`, the tip of the
unmerged Server Edition stack (#508 → #516 → #515 → that branch). Every
`file:line` below is at that commit unless it names another branch. Where this
document and the code disagree, the code wins.

**Inputs.** `server-edition.md` (decisions 1–7, §III.3–III.9, the Queued
list, Appendices A and B), the OWASP review of `serve` as built (findings
H1, M1–M3, L1, I1–I10) and handshake v1, all of which this document cites by
their own names. Section numbers here are this document's own; `§III.x`,
`R1`–`R9` and "decision N" refer to `server-edition.md`.

**Method.** Every claim that something exists or is missing was read in the
code at the base commit. Nothing was built or run. A claim that rests on
platform behaviour nobody has measured here says so, and §10 lists those
measurements as exit criteria. The draft was then reviewed with the
`owasp-security` skill. §12 lists what that review changed.

---

## 0. The decisions, on one page

| # | Question | Recommendation | Phase |
|---|---|---|---|
| 1 | How does a device enroll? | A QR shown on the host carries the server's SPKI pin and a 128-bit one-use secret. The pairing listener exists only while a window is open, and the window closes after one attempt or 120 s. The device proves the secret over pinned TLS, bound to the session by a TLS exporter. Both screens show a short code, and the user confirms on the host. | 2 |
| 2 | What is a device's credential? | Its own P-256 key: in the Secure Enclave on iOS, and in a `0600` file on macOS and Linux. It authenticates with mutual TLS, so an unenrolled peer never reaches the HTTP parser. There are no passwords and no bearer tokens. On iOS, the riskiest acts also need a second, presence-bound key (a step-up). | 2 |
| 3 | Revocation, and a lost phone | A revoke takes effect at once: the device's live sockets close, its next request on any socket is refused, and its next TLS handshake fails. The host console or another device with `admin` can revoke it, but only the console can WIDEN a scope. A stolen locked phone cannot connect. An unlocked one keeps its scope until it is revoked, and the audit log shows what it did. Clients cache nothing they can lose. | 2 |
| 4 | May the loopback listener enroll? | It is where enrollment is AUTHORIZED, and never where a device enrolls. Opening a pairing window is a console act, done over loopback with serve.json's token. The loopback listener never accepts a device credential. | 2 |
| 5 | Per-object authorization (R1) | Each device has capabilities (`read`, `write`, `answer`, `instruct`, `stream`, `admin`) and a project scope. Every row of the dispatch table declares the capability it needs and the object it acts on. One resolver checks that before the body runs. The scope also rides into the api, so the api's own path guard answers with the device's roots: two checks, as R1 asked. Results and events are cut down to the scope. | 2 |
| 6 | Rows a remote device never reaches | A new `HostConsole` class, served on loopback only. It covers every row that can widen what agents on this host may do, pairing and device administration, `update_cli`, and adopting a project from a hub. | 2 |
| 7 | The remote listener | Off by default, with a one-click kill switch on the host. It binds one specific overlay or LAN address, never a wildcard, never loopback and never a public address. TLS 1.3 is mutual and pinned both ways (rustls), with no CA, no resumption and no 0-RTT in Phase 2. On iOS the socket lives in the Rust shim and not in the WebView. | 2 |
| 8 | The CI witness (R8) | A connect witness for loopback serve, which is missing today. Then identity: no certificate is refused, an unenrolled one is refused, an enrolled one is served, a revoke kills the live socket, and the pairing port is closed outside a window. Measured on both platforms, with a release gate that greps the exact count. | 2 |
| 9 | The audit log (R5) | `<config_dir>/audit.jsonl`, mode `0600`, append-only and hash-chained. Only serve's connection layer writes it, and its records are structured, never free text. | 2 |
| 10 | Resource limits (R6) | Per-device connection, in-flight and rate caps; bounded outboxes and lanes; single-flight for the expensive reads; separate budgets for loopback and remote, so a device cannot starve the desk. | 2 |
| 11 | The agent-instruction surface (R7) | One path classifier in core. Instruction files need `instruct`. Files that EXECUTE without anyone choosing to run them need `stream`. `.git/**` is never written. Provenance is kept out of band, with no markers in the file. | 2 |
| 12 | The known gaps | Eight from the brief, seven found while writing this, and one from Appendix B. Each has a fix shape and a phase (§8). One of the new ones (gap 9) can be reached from a cloned repository's content TODAY. | 2, some now |
| 13 | Multi-client | View state is per device and never reaches serve. Shared preferences become per-key patches with a revision check. Host policy goes to `host-policy.json`. A prompt answer is a compare-and-swap, and a lease is UX on top of it. | 2, lease 3 |
| 14 | Multi-user (R9) | A non-goal, recorded so the schema may rely on it. It needs the user's confirmation (§11). | 2 |

---

## 1. What Phase 2 protects, and from whom

**Assets.** The host user's account (a shell on the host is everything), the
repositories and the credentials that live in their trees, host policy (what
agents on this host may do on their own), and the user's other devices.

**Attackers this design answers.**

- **A1, an unenrolled network peer.** Anything on the overlay or LAN that can
  reach the remote port. It holds no key.
- **A2, the holder of a lost or stolen enrolled device.** It holds a key and
  has whatever that device's scope allows, until the device is revoked.
- **A3, a malicious repository.** Content the user cloned: tracked symlinks,
  instruction files, configs. This attacker exists today. Phase 2 must not
  widen what it can reach.
- **A4, a prompt-injected agent,** in what it can reach through worktrees'
  own surfaces: the MCP tools, and the files of the tree it works in. Phase 2
  adds nothing to the MCP server (§7.5), and the path classes (§7.1) apply to
  what a device writes for an agent to read.

**Out of scope, and said so.**

- **A process running as the same user on the host.** It can read
  `serve.json`, the macOS device-key file and the audit log, and it can
  ptrace serve. Handshake v1 already states this, and no design at this layer
  changes it. **An agent allowed to run shell commands IS such a process.** It
  could write `devices.json` or `serve-remote.json` directly. Phase 2 cannot
  prevent that, so it makes it LOUD: §5.2 disables any device record that has
  no pairing record behind it, and the host always shows when remote access
  is on. A sandboxed agent (Codex's workspace-write sandbox, for one) cannot
  reach the config dir at all.
- **Several humans.** R9 is answered as a non-goal (§11, Q1). Scopes keep one
  person's devices to least privilege. They are not isolation between people.
- **Browser clients over the remote listener.** Decision 3 makes the phone a
  Tauri build. A browser cannot hold the device key (§4.4), so a browser
  origin is refused remotely.
- **Exposure on the public internet.** Refused at bind time (§4.1, §11 Q6).

### 1.1 The honest ladder: which capabilities are containment

The design has to say this before it offers scopes, or the scopes will be read
as stronger than they are.

| What the device holds | What its holder can do on the host | Is the scope a boundary? |
|---|---|---|
| `read` | Read state, diffs and non-secret files in its projects | **Yes.** No path to code execution. BOLA matters most here. |
| `read` + `write` | Also: edit ordinary files, change declared state, create and close places, launch agents under host policy | **Partly.** An edited file is content an agent may later read, so it is an indirect-injection channel (LLM01). Under host policy `ask`, an injected agent still has to be approved, and this device cannot approve it. |
| `answer` added to `write` | Also: approve or deny the prompts agents raise | **No.** Edit a file, launch an agent, approve what it asks: that is a shell, run through an agent. |
| `instruct` (R7's second capability) | Write the text agents are built to obey: briefs, `AGENTS.md`, `.claude/**` and the like | **No**, as soon as an agent can act without approval or the device can approve. |
| `stream` | A login shell and tmux attaches on the host | **No.** `shell_open` spawns `$SHELL -l` (`crates/worktrees-api/src/term.rs:480`–`482`). That is full control of the account. |
| `admin` | Host configuration that cannot widen agent autonomy (§3.4) | No. It is about the host as a whole, not a project. |

So the enrollment and device-list UI computes one label from the device's
capabilities and the host's current policy: **"can run commands on <host> as
you"**. It applies when the device holds `stream`, or `answer` together with
`write` or `instruct`, or holds `instruct` while host policy lets agents act
unprompted (Codex `full` or `auto-review`, for example). The label is
recomputed when host policy changes, and it is never hidden behind a
preset's name. Above the first rung, a project scope decides
what the device SEES and what its holder can do by MISTAKE. It does not stop
a holder who is determined to do harm. That is what revocation and the audit
log are for.

---

## 2. Identity and enrollment (R4)

### 2.1 Credentials

| Credential | What it is | Where it lives | Lifetime |
|---|---|---|---|
| Server identity | A P-256 key and a self-signed certificate. Its SPKI SHA-256 is the **pin**. | `<config_dir>/server-identity.key` (PKCS#8, `0600`) and `.crt` | Created the first time remote access is enabled. Reused across launches, because a pin must survive a restart, unlike the loopback token. Rotating it means pairing every device again (§2.5). |
| Device key (iOS, iPadOS) | P-256 in the Secure Enclave, non-exportable, `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`, never synced to iCloud | The device's keychain | Until it is revoked or the app is deleted |
| Device key (macOS, Linux) | P-256, in a `0600` PKCS#8 file | The client app's own config dir | As above |
| Step-up key (iOS, iPadOS) | A second Secure Enclave P-256 key, created with `.userPresence`. It signs a server challenge before a drive-class act (§2.5). | The device's keychain | Enrolled with the device key, revoked with it |
| Device record | `{id, name, platform, spki_sha256, stepup_spki_sha256, caps, projects, enrolled_at, last_seen, last_addr, revoked_at}`; `id` is random, not derived from the key | `<config_dir>/devices.json`, `0600`. Serve is its only writer, under `servefile`'s lock-temp-rename discipline (`crates/worktrees-cli/src/serve/servefile.rs:79`–`122`). | Kept after revocation as a tombstone, for the audit trail |
| Loopback token | Unchanged: per launch, in serve.json, used for handshake v1 | `<config_dir>/serve.json` | Per launch |

Three choices in that table need their reasons.

- **P-256, not Ed25519.** The Secure Enclave signs with P-256 and nothing else.
  rustls accepts `ecdsa_secp256r1_sha256` for client authentication, and it
  can take a signer that calls `SecKeyCreateSignature` instead of holding key
  bytes. So the key on the phone never leaves the hardware. Ed25519 would
  force a software key on iOS.
- **A file on macOS, not the keychain.** Releases are ad-hoc signed
  (decision 4), so the designated requirement is a bare `cdhash` that changes
  with every build. AGENTS.md records TCC asking again after every build for
  exactly this reason. A login-keychain item's access control is keyed the
  same way, so every update would ask "allow access?". The data-protection
  keychain and the Secure Enclave need entitlements that an ad-hoc build does
  not have. *Expected, not measured: §10 lists it.* A `0600` file is the same
  protection serve.json has today, and it matches the threat model (§1).
  Revisit when macOS builds are signed with a team identity.
- **`<config_dir>`, not `~/.config/worktrees/`.** §III.5 put `devices.json`
  beside the registry. It belongs beside `serve.json` and `host-policy.json`
  instead, in the config dir the engine was started with (`Ctx::config_dir`,
  `crates/worktrees-api/src/lib.rs:115`). A sandbox serve
  (`app/scripts/sandbox.sh --app`) or a dev-stack serve runs on its own config
  dir, and it must not accept the user's real phone. The registry is shared
  between engines and the CLI on purpose. Device enrollment must not be.

### 2.2 Enrollment, step by step

**On the host.** Settings → Devices → **Pair a device…**, or
`worktrees serve pair`, which is a thin client of the loopback socket.

1. The user chooses the device's **scope first**: a preset (§3.1) and its
   projects. The QR appears only after that, so the decision is not made
   with a timer running. Which preset is pre-selected is §11 Q4. The project
   list starts empty, so a scope is always chosen, never inherited.
2. Serve opens the **pairing window**, which binds a separate pairing
   listener on the remote address. The host shows:
   - a QR encoding `worktrees-pair:v1?addr=<overlay ip or name>:<port>&pin=<spki sha256>&secret=<128-bit, base32>&host=<host name>&exp=<epoch>`;
   - the same string as a link, for a device with no camera (§2.3);
   - a 120 s countdown.
3. The device scans the QR and parses it strictly before it dials
   anything. It refuses an unknown parameter, an expired `exp`, and an
   `addr` outside §4.1's address classes: a QR is untrusted input, and a
   public address in one is a phishing attempt. Then it makes its keys
   (§2.1) and connects to the pairing port with TLS 1.3, **pinning the QR's
   SPKI** and presenting a self-signed certificate over its new device key.
4. The device sends one frame:
   `{pair: {name, platform, stepup_spki}, proof: HMAC-SHA256(secret, "worktrees-pair/v1\n" || exporter || device_spki_sha256 || stepup_spki_sha256 || name)}`.
   Here `exporter` is the TLS 1.3 exporter (RFC 8446 §7.5, label
   `EXPORTER-worktrees-pair-v1`, 32 bytes), so the proof is bound to this TLS
   session and to these keys. It cannot be relayed into another session, and
   it cannot be replayed for other keys. `name` is at most 64 characters,
   with no control characters and no bidirectional overrides, so it cannot
   pass itself off as another device's name in the host dialog. Serve
   refuses anything else.
5. Serve verifies the proof in constant time. Any failure closes the window,
   with no retry: a bad proof, a malformed frame, a second connection, a
   frame after the first, or the deadline. Serve records the failure in the
   audit log (§5).
6. Both screens show a six-digit **comparison code** derived from
   `HMAC(secret, "worktrees-pair/v1/sas\n" || exporter)`. The host asks:
   *"Bret's iPhone (iOS) wants to pair · code 482 913 · Observe on 3 projects
   · Allow only if this code is on your device right now · [Deny] [Allow]"*.
   **Allow is the consent act**, in the same sense as the registry's
   "registering is the consent act" (§III.5). Deny closes the window. A
   device that finds the window already used says so in words that point at
   the host: *"Someone else used this pairing code. Tap Deny on <host>."*
   Pairing events go to loopback clients only (§3.3).
7. On Allow, serve writes the record into `devices.json` and answers
   `{paired: {device_id, host_name, remote_addr}}`. Then it closes the
   pairing listener. From now on the device connects to the remote listener
   with its key (§4).

**Why the code is compared and not just shown.** Someone who photographs the
QR within the window can race the user's device. A QR on a shared or
recorded screen counts as photographed. Only one attempt is allowed, so
either the racer gets in first and the user's own device then fails
visibly, or the user's device gets in first and the racer fails visibly. The
comparison code is what stops the user tapping Allow on the racer's request
in the first case. A device name is chosen by whoever sends it, so the name
proves nothing.

**A residual on the device side.** A QR made by an attacker can pair the
user's phone with the ATTACKER's server. The device shows the host name from
the QR and the comparison code, and the code matches only on the screen of
the server it actually paired with. Comparing it against the user's own Mac
catches this, if the user looks there. Nothing here stops a user who scans a
stranger's QR and never looks.

### 2.3 The fallback: a pairing link, not a short code

R4 asked for a high-entropy QR plus a typed fallback rate-limited on its own.
**Recommendation: Phase 2 ships no low-entropy code at all.** The fallback is
the same `worktrees-pair:` string as text, copied over a channel the user
already trusts: Universal Clipboard, AirDrop, or a password manager. It
carries the full pin and the full secret, so it gives exactly the QR's
guarantees.

A short code typed by hand cannot carry a 256-bit pin. A device that has to
trust the first server certificate it sees, plus an HMAC over a short code,
lets an active man in the middle capture one proof and search the code space
offline. If a typed code is ever wanted, it must be a PAKE (SPAKE2 or CPace),
where one online guess per window is the rate limit. That is Phase 3 or
later, if anyone asks for it.

### 2.4 The pairing window's rules

- **One window at a time.** Opening a second one closes the first.
- **The pairing listener exists only while a window is open** (R4). Outside
  a window a connect to its port is refused, and the CI witness checks that
  (§4.6).
- **Window lengths.** 120 s without an attempt; 60 s from the frame to the
  user's Allow or Deny. Each runs out as a Deny.
- **One attempt per window**, and a failure closes it.
- **Only a console can open a window** (§2.6). Not an enrolled device, not
  even one with `admin` (§11 Q5 asks the user to confirm). Pairing started
  remotely would turn a stolen device into a lasting foothold: enroll a
  second device, then outlive the stolen one's revocation.
- **A device cap.** At most 16 enrolled devices (§6). Pairing beyond the cap
  is refused before the QR appears.

### 2.5 Revocation, rotation, and what a lost phone costs

**Revoking.** It is available from the host console, from any enrolled
device that holds `admin`, and from a device on itself. Serve marks the
record (`revoked_at`) and keeps it as a tombstone. Then it closes every live
connection of that device: it keeps a map from device id to connection ids
for exactly this. That also ends the device's tmux attaches, through the
cleanup that already exists (`crates/worktrees-cli/src/serve/conn.rs:566`–`569`).
It cancels the clone jobs that device started. A body already running
(a sync, say) finishes, its reply is dropped, and the audit log records
both. Serve writes an audit record for the revoke. Authorization reads the
device's CURRENT record on every request (§3.3), so a socket that outlived
the close for any reason is refused at its next request. The TLS verifier
reads the same in-memory set, so the next handshake fails before HTTP.

A device with `admin` revoking a DIFFERENT device is allowed on purpose. If
the phone is lost while the user is travelling with the laptop, the laptop
must be able to cut it off. The cost: a stolen `admin` device can revoke the
user's other devices. That is a denial of service which the audit log
records, and pairing again at the desk undoes it. "Revoke every device but
this one" is console-only.

**What a lost phone costs.**

- **Locked:** nothing. The key is usable only while the device is unlocked
  (`WhenUnlockedThisDeviceOnly`), so the TLS handshake cannot be signed.
- **Unlocked, or unlocked later:** its scope, until it is revoked. Section
  1.1 says what that scope is worth.
- **After the revoke:** nothing further. The key never left the Secure
  Enclave, so there is no copy to revoke separately. The other devices are
  unaffected, since every credential is per device. The server identity does
  not need to rotate: the phone held only the pin, which is public.
- **The record:** the audit log names every command the device ran,
  attributed to its id (§5).
- **What was on the phone:** nothing the app kept. Remote clients keep no
  persistent copy of file contents, diffs, terminal bytes, drafts or
  transcripts; those live in memory only. What a client does store (its view
  state, §9.1, and the host's pin) uses iOS Data Protection class
  `Complete`. Without this rule a revoke would cut off the host and leave the
  user's code readable on the stolen phone.

**Step-up for drive-class acts (iOS).** The connection key has no presence
requirement, so status, notifications and reads stay seamless. Opening a
stream, an `instruct` write, an `answer` and any `admin` row also need a
fresh signature from the step-up key (§2.1), which asks for Face ID or the
passcode. Serve sends a challenge, the device signs `challenge || exporter`,
and the proof stays valid on that connection for 5 minutes. That is the
second factor ASVS L2 asks of sensitive operations: possession of the
Secure Enclave key, plus presence. A desktop device has no presence-bound
key while macOS builds are ad-hoc signed (§2.1). Its drive-class acts
therefore rest on one factor, and the device list says so. §11 Q8 asks
whether step-up should be on by default.

**Rotating the server identity.** `worktrees serve identity rotate`, from a
console only. Every device has to pair again. Rotation is meant for a
suspected key compromise, and in that case a rotation signed by the old key
would be signed by the attacker too. So there is no signed handover, and the
cost is honest: one pairing per device.

**Idle devices.** A device not seen for 90 days is disabled. It is not
deleted, and re-enabling it is a console act. §11 Q7 asks whether the user
wants this.

### 2.6 The loopback listener and enrollment (decision 6's open half)

**Ruling: the loopback listener authorizes enrollment and never performs it.**

- The rows that open a pairing window, accept or deny a request, list
  devices, revoke all others and rotate the identity are a new `HostConsole`
  class (§3.4). They are served only on loopback, to a client that passed
  handshake v1 with serve.json's token: the desktop app, the CLI, or a dev
  stack. That client is the local user. It already holds a capability for
  everything (servefile.rs:9–10), so letting it pair adds nothing an
  attacker could take.
- The loopback listener never accepts a device certificate, and it never
  runs the pairing protocol. A device on the same host has no reason to
  enroll: it can read serve.json. A remote device that reaches loopback
  through an SSH tunnel already holds a shell, and gets nothing new.
- Decision 6's other consequence was that refusing a missing `Origin` shuts
  out a native client until Phase 2 gives it a credential. It stays as
  decided. A native LOCAL client sends the app's origin, as engine.rs's own
  probe does (`app/src-tauri/src/engine.rs:70`, `APP_ORIGIN`). A native
  REMOTE client has a device key.

### 2.7 Options considered for identity

| Option | For | Against | Verdict |
|---|---|---|---|
| The overlay's identity alone (Tailscale ACLs stand in for enrollment) | Least code; Tailscale handles rotation and NAT | The security boundary belongs to a third party, and a shared tailnet node or a mistaken ACL reaches the engine. There is no per-device capability, so R1 has nothing to key on. It also violates R4's "the overlay should not carry the whole burden". | Rejected. The overlay **sits under** enrollment as transport and network ACL (§11 Q2). |
| Per-device bearer tokens | Simple | Copyable. A token lands in logs and backups, and from there in any process that reads them. No channel binding. | Rejected |
| Passwords or a PIN | Familiar | Guessable. Needs rate limiting. Shared across devices. | Rejected |
| An application-layer signature after TLS (handshake v2 over plain TLS) | No custom TLS verifier | The HTTP and WebSocket parsers run for unauthenticated peers. The channel binding has to be built by hand. | Rejected in favour of mTLS |
| **Mutual TLS with pinned self-signed certificates** | Unenrolled peers stop inside rustls, before HTTP. The binding comes from TLS itself. The key works with the Secure Enclave. | Two small custom verifiers (§4.3) | **Chosen** |
| Noise IK (`snow`) | Pinning is native; no X.509 | X25519 only, so it cannot use a Secure Enclave key. A second protocol stack. | Rejected |

---

## 3. Per-object authorization (R1)

### 3.1 Capabilities and presets

A closed enum, so a new capability is a compile error everywhere it matters:

| Capability | Grants |
|---|---|
| `read` | Engine reads inside the scope |
| `write` | Mutations of declared state and places inside the scope; ordinary files; agent launch under host policy |
| `answer` | Answering a prompt an agent raised, with one of the prompt's own choices (§9.2). The verb lands in Phase 3; the capability is defined now, so `devices.json` does not need a migration later. |
| `instruct` | R7's second capability: text that becomes an agent's instructions (§7) |
| `stream` | Terminal attaches and dock shells. Implies everything a shell can do. |
| `admin` | Host-wide configuration that cannot widen agent autonomy (§3.4) |

| Preset | Capabilities | Label (§1.1) |
|---|---|---|
| **Observe** | `read` | — |
| **Operate** | `read`, `write` | — |
| **Drive** | `read`, `write`, `answer`, `instruct`, `stream` | "can run commands on <host> as you" |
| + Host admin (a toggle) | adds `admin` | unchanged |

Custom sets are allowed, and the label is computed, never chosen. §11 Q4 asks
which preset the dialog should pre-select.

### 3.2 Scope: projects, not places

`projects` is either `"*"` (every registered project, including ones added
later) or a list of canonical project roots, in the registry's own keys.
**Phase 2 scopes by project only.** Places are short-lived and their slugs
get reused. A per-place scope would need a durable place identity, which
does not exist, and it only pays off for multi-user, which is a non-goal.

A project removed and added again at the same path is the same project. A
project added later is outside an explicit list until the user adds it to
the device. **Only the console can WIDEN a scope**: add a capability, add a
project, or turn a list into `"*"`. An `admin` device may NARROW another
device or revoke it, and never changes itself. Every change is audited. Two
stolen `admin` devices therefore cannot widen each other, and no device can
raise its own privileges.

### 3.3 Enforcement: declared in the dispatch table, never per handler

Today a row is `name: Exposure => body`, and the macro makes `TABLE` and the
dispatcher from the same list
(`crates/worktrees-cli/src/serve/dispatch.rs:160`–`178`). Phase 2 adds two
columns to the same row:

```rust
set_note:   Engine [write, place(repo, slug)] => ops::set_note(a.get("repo")?, a.get("slug")?, a.get("note")?),
read_file:  Engine [read,  file(path)]        => files::read_file(&cx.ctx, a.get("path")?, a.get("max_bytes")?),
write_file: Engine [write, file(path)]        => files::write_file(&cx.ctx, a.get("path")?, a.get("content")?, a.get("expected_mtime")?),
term_write: Stream [stream, attach(id)]       => term::term_write(a.get("id")?, a.get("data")?, &cx.ctx.terminals),
update_cli: HostConsole                       => status::update_cli(a.get("tag")?),
```

The generated dispatcher runs, in order, before any body:

1. **The exposure.** `ClientLocal` is refused as today (`dispatch.rs:171`).
   `HostConsole` is refused with `err: "console-only"` unless the caller is
   loopback.
2. **Resolve the object.** The resolver reads the named arguments from
   `Args` and turns them into a `Resolved { project_root, place, rel, class }`.
   Then it **writes the canonical value back into `Args`**, so the body acts
   on exactly what was checked. Object kinds:
   - `project(arg)`: the argument canonicalizes to EXACTLY a registered root.
     This is stricter than `guard_under_projects`
     (`crates/worktrees-api/src/projects.rs:45`), which accepts anything
     under a root.
   - `place(repo, slug)`: `project(repo)`, and the slug is `(main)` or a
     single component that passes core's `safe_arg` (no `/`, no leading `-`,
     no `..`, no control characters).
   - `root(arg)`: a place root, meaning a registered root or a direct child
     of its `.worktrees/`. `plan_ask_main` already checks this shape by hand
     (`crates/worktrees-api/src/files.rs:704`–`710`).
   - `file(arg)`: `guard_under_projects`, then the owning project, then the
     path class (§7). The class can RAISE the capability the row needs: a
     `write_file` to `AGENTS.md` needs `instruct`.
   - `session(arg)`: the session must belong to a place in scope, looked up
     in the snapshot's place list (adopted sessions included). It is passed
     on with `=` to anchor it, as `tmux::ai_pane` already does
     (`crates/worktrees-core/src/tmux.rs:587`).
   - `attach(id)`: the id is in this connection's own set (`Frames::note_term`,
     `crates/worktrees-cli/src/serve/wire.rs:226`). That is Appendix B's
     `term::NEXT_ID` fix.
   - `shell(repo, slug, index)`: `place(repo, slug)` and a `u32` index.
   - `job(id)`: a clone job this connection started.
   - `new_path(arg)`: a path where a project WILL be. It needs `admin` and
     scope `"*"`, because it changes the registry.
   - `global`: no project object. Allowed for any device that has the
     capability, but only for rows on the `GLOBAL_OK` list (data about the
     host, not a project: usage meters, versions, the changelog).
   - `filtered(f)`: a multi-project read whose RESULT is passed through `f`,
     which drops whatever is outside the scope (`list_workspace`,
     `list_drafts`, `planning_status`).
   - `all`: requires scope `"*"` (`reorder_projects`, `log_tail`).
3. **The capability check.** The row's capability (raised by the class, where
   that applies) must be in the device's set, and `Resolved.project_root`
   must be in its scope. Loopback holds every capability and scope `"*"`.
4. **The audit record** (§5), for every refusal and every mutation.
5. **The body**, unchanged except for one thing it is now handed: the
   caller's scope (below).

**Who the caller is comes from the listener, never from the request.**
`Caller::Loopback` or `Caller::Device(id)` is set by the listener object that
accepted the socket. It is a type, fixed for the life of the connection. It
is never derived from a header, an `Origin` or a peer address, so a
tunnelled connection, a forged header or a dev `--allow-origin` page cannot
become a different kind of caller. The remote listener does not read
`--allow-origin` at all.

**The record is read per request, not per connection.** The device record
is looked up on every request through an in-memory generation counter (one
atomic load when nothing has changed). A narrowed scope or a revoke
therefore applies to the very next request on a socket that is already
open.

**The scope rides into the api, so the api's guard is the second check.**
Checking the arguments in the dispatcher is not enough, because some bodies
ENUMERATE roots to build their answers:

- `resolve_term_paths` and `resolve_doc_paths` resolve candidates against
  `project_roots(ctx)`, which is every registered project
  (`crates/worktrees-api/src/files.rs:1117`, `:1226`);
- `list_workspace` walks the whole registry;
- `drop_reference`'s reach plan reads the whole registry
  (`crates/worktrees-api/src/ops.rs:122`–`129`).

So `Call` carries the scope, and `read_projects`, `project_roots` and
`guard_under_projects` (`crates/worktrees-api/src/projects.rs:30`–`71`)
answer with the CALLER's roots. A body that forgets a filter then fails
closed instead of leaking, and every path still passes a resolver check and
an api check: §III.3a R1's "two checks, not one".

**Nested arguments are objects too.** `project_prs.places`,
`open_docs_viewer.place`, `resolve_term_paths.shell`, `reorder_projects.roots`
and every `paths` array are declared in the row's object spec and resolved
element by element. One element outside the scope refuses the whole
request.

**A refusal says the same thing whether the object is missing or outside
the scope.** The error is `err: "denied"` for a capability, and
`err: "not found or not in this device's scope"` for an object. A scoped
device must not be able to probe which paths, projects or sessions exist
(BOLA enumeration). The audit record keeps the real reason, for the host's
eyes.

**Events go through the same scope.** `Broadcast::emit` sends every event to
every authenticated socket (`crates/worktrees-cli/src/serve/conn.rs:202`–`207`).
Each event type gets a projection beside the row table:

- `places:changed` carries no payload (`crates/worktrees-api/src/host.rs:189`).
  It only says "re-read", and the re-read is itself scoped, so it goes to
  every client.
- `sessions:busy` carries the host-wide busy, waiting and waiting-by sets
  (`crates/worktrees-api/src/host.rs:316`). Each client receives only the
  members inside its scope.
- `sessions:done` and `shell:exit` are filtered the same way.
- `app:open-doc` goes to loopback clients only (§9).
- The new pairing, device and alert events (§2, §5.3) go to loopback clients
  only.

An event type with no projection is dropped for every scoped device. That
fails closed, and a test fails when an `emit("…")` name in the api crate has
no entry.

**What keeps the table honest.** These are tests in the `*-check` family.
Each one must be shown failing first (AGENTS.md).

- Every row declares a capability and an object, or is `HostConsole` or
  `ClientLocal`. The macro will not compile otherwise.
- Every argument a body reads that names an object (`repo`, `root`, `path`,
  `slug`, `session`, `id`, `dir`, `location`, `parent`, `into_*`) is covered
  by the row's object spec. A test reads `dispatch.rs` with `include_str!`,
  slices off `mod tests` (the AGENTS.md rule about a drift check that takes
  the first match) and fails on an uncovered `a.get("…")`.
- `GLOBAL_OK` is an explicit list, and a `global` row outside it fails.
- Every event name has a projection.
- A resolver test per kind: canonicalization, symlink escape, `..`, the
  `(main)` slug, a session prefix match (`api` against `api-fix`), and that
  missing and out-of-scope objects get identical errors.

### 3.4 Every command, mapped

The 150 rows at `fadb2ce`: Engine 93, HostAdmin 42, Stream 9, ClientLocal 6
(`dispatch.rs:223`–`452`; `the_table_has_every_command_once` pins 150).
Each row appears once below, under its base group; two appear a second time,
where an argument changes what they need (see the count at the end). **Bold**
marks a row whose class or capability changes from today's tag. "Console"
means the new `HostConsole`: loopback only.

**ClientLocal (unchanged, refused over any socket):** `copy_text`,
`install_app_version`, `open_editor`, `detect_editors`, `open_terminal`,
`set_zoom`.

**Console only.** Each of these widens what agents on this host may do, or
administers the host itself. "A remote device can drive work; it cannot
change what this machine's agents are allowed to do."

| Row | Today | Why it is console-only |
|---|---|---|
| `update_cli` | HostAdmin | It replaces the binary that is serving the request. §V.2 recommended refusing it, and I9 flagged it. |
| `set_codex_permissions`, `set_pi_trust` | HostAdmin | Host policy (decision 7, M2) |
| **`set_pi_allowed`, `set_kimi_allowed`** | HostAdmin | They allow a repo's own code to load in pi or kimi |
| **`set_cross_project`, `set_project_private`** | HostAdmin | They widen how far agents reach across projects |
| **`mcp_install`, `codex_mcp_install`, `pi_mcp_install`, `kimi_mcp_set`, `codex_mcp_migrate`** | HostAdmin | They give agents tools, mutating ones with `mutations` |
| **`save_profile`, `set_project_profile`, `set_default_profile`** | HostAdmin | `mcp_servers` and `settings` (claude's `hooks` live in that raw JSON) are commands run at the next launch. Binding a profile to a project chooses which of them run there. |
| **`set_agent_guidance`, `set_agent_guidance_skill`, `agent_guidance_merge`, `agent_link_skills`, `skill_install_local`, `skill_install_git`** | HostAdmin | Text and scripts that every agent on the host loads |
| **`sync_apply`, in its adopt form (no `repo`)** | HostAdmin | It writes to a path the hub's manifest chooses (§8, gap 4) |
| *new:* `pair_begin`, `pair_decide`, `devices_revoke_all_others`, `identity_rotate` | — | §2.6 |

**admin** (served remotely to a device with `admin`; Phase 2 keeps the
loopback behaviour the same):

| Object | Rows |
|---|---|
| `new_path` (needs scope `"*"`) | `add_project`, `create_project`, `clone_project` (from a device, `file://` URLs and hosts that resolve to loopback, link-local or private addresses are refused: `clone::parse_source` accepts `file://`, `crates/worktrees-core/src/clone.rs:68`, and a remote device must neither read an unregistered repo on the host's disk nor borrow the host's network position), `init_repo`, **`probe_dir`** (an Engine read today; it tells the caller whether any host path exists and is a repo), **`skill_inspect`** (Engine today; it reads `SKILL.md` and lists files at any host path) |
| `job(id)` | `clone_cancel` |
| `project` | `create_initial_commit`, `remove_project`, `set_project_planning`, `mcp_status`, `mcp_uninstall`, `sync_apply` (with `repo`) |
| `all` | `set_planning_default`, `set_quota_settings`, **`set_fetch_interval`**, **`set_term_history_opts`** (both host policy, Appendix B), **`term_history_clear`**, **`ui_events_clear`** |
| `global` | `codex_mcp_status`, `codex_mcp_uninstall`, `codex_mcp_migration_plan`, `pi_mcp_status`, `pi_mcp_uninstall`, `kimi_mcp_status`, `delete_profile`, `skill_remove`, **`skill_preview_git`** (fetches any URL from the host), **`sync_hub_list`** (lists every project's local path on the hub), **`diagnostics`** (the host's PATH, versions and log paths), **`log_tail`** (serve.log names every project's places, so it also needs scope `"*"`), **`term_history_info`**, **`profiles_info`** (profiles can carry credentials in an MCP server's `env`) |

**stream:**

| Object | Rows |
|---|---|
| `shell(repo, slug, index)` | `shell_open`, `shell_write`, `shell_resize`, `shell_detach`, **`close_shell_session`** (Engine today) |
| `session(session)` | `term_open` (exact-anchored; §8, gap 3) |
| `attach(id)` | `term_write`, `term_wheel`, `term_resize`, `term_close` |

**instruct:**

| Object | Rows |
|---|---|
| `project` | **`init_write`** (writes `.worktrees.toml`), **`agent_setup_fix`** (commits the instruction-file layout, and pushes it when asked), **`rules_apply`** (writes agent rules into the repo) |
| `file`, raised by class | `write_file` to an instruction path (§7) |

**write:**

| Object | Rows |
|---|---|
| `place(repo, slug)` | `set_lifecycle`, `set_pin`, `set_note`, `set_title`, `touch_place`, `mark_seen` (with the host's clock; §9), `switch_place`, `open_place`, `close_place` (plus `session(session)`), `remove_place`, `relink`, `provision` |
| `place` × `place` + `session` | `drop_reference`: both places in scope, and `into_session` belongs to the receiving place. The text is built by the server and pasted with no Enter (`crates/worktrees-api/src/ops.rs:165`). |
| `project` | `new_place`, `fetch_origin`, `rules_review`, `rules_lane_remove` |
| `root` + `session` | `plan_prompt` and `plan_ask_main`: fixed text, no Enter (`files.rs:649`, `:683`, `:703`) |
| `file(path)` | `write_file` to a plain path (§7) |
| `global` | `set_settings` (it becomes a per-key patch; §9), `log_event` (L1 fix, rate-limited) |
| `all` | **`reorder_projects`**: the nav order is host-wide |

**read:**

| Object | Rows |
|---|---|
| `project` | `list_places`, `project_config_read`, `sync_status`, `sync_preview` (adopt form: `admin` with `"*"`), `init_suggest`, `agent_setup_status`, `pi_status`, `kimi_status`, `remote_url`, `project_prs`, `planning_suggested` |
| `place` | `doctor`, `place_health`, `ai_status_report`, `list_branches`, `list_shell_sessions` |
| `root` | `changed_files`, `place_plan`, `list_docs` (plus `project(repo)`), `resolve_term_paths` (plus `session`), `resolve_doc_paths` (plus `file(doc)`) |
| `file` | `list_dir`, `file_diff`, `read_file`, `read_file_base64`, `file_readable` (a secret-class path needs `stream`; §7) |
| `place`, refused remotely in Phase 2 | `open_docs_viewer` (§8, gap 7) |
| `filtered` | `list_workspace`, **`list_drafts`** (the TEXT people typed into composers, host-wide), `planning_status` |
| `global` (on `GLOBAL_OK`) | `settings_info`, `get_settings`, `cross_project_status`, `quota_settings`, `claude_usage`, `codex_usage`, `kimi_usage`, `claude_status`, `check_update`, `list_releases`, `get_ai_config`, `get_codex_permissions`, `agent_guidance_status`, `agent_guidance_diff`, `agent_models`, `agent_user_skills`, `kimi_version`, `tmux_check`, `log_info`, `get_changelog`, `new_profile_id`, `skills_list`, `ui_usage` |

**Loopback-only, not console:** `ui_events_append`. Remote clients do not
emit usage events, so `ui-events.jsonl` keeps its "never leaves the machine"
premise (§III.6.2).

Count, by each row's base group: read 53, write 23, admin 33, console 21,
stream 10, instruct 3, ClientLocal 6, loopback-only 1, which is 150. Two rows
appear twice on purpose, because an argument changes what they need:
`write_file` (its path's class, §7) and `sync_apply` (the adopt form). This
paragraph is not what holds the count. A test is, and it asserts that every
name in `TABLE` has exactly one base group.

### 3.5 What changes for loopback

Loopback has scope `"*"` and every capability, so for the desktop app almost
nothing changes. Two things change on purpose, for every caller:

- **`repo` must be a registered project.** Today `set_note` writes
  `<repo>/.worktrees.places.json` into ANY directory it is given:
  `ops::set_note` (`crates/worktrees-api/src/ops.rs:76`) → `store::edit` →
  `store_base` canonicalizes and nothing else (`crates/worktrees-core/src/store.rs:134`).
  A missing file reads as an empty store (`store.rs:263`), so the write
  succeeds. `Project::discover` accepts any git repository, registered or
  not (`term.rs:634`–`637`).
- **A slug is one path component.** Today `place_dir` is
  `format!("{wt_root}/{slug}")` (`crates/worktrees-core/src/project.rs:500`–`505`),
  so `../x` leaves `.worktrees/`.

---

## 4. Exposure

### 4.1 The remote listener

- **Off by default, and one click from off again.** "Turn off other
  devices" on the host is the kill switch (ASI10). It closes every remote
  connection, unbinds the listener and the pairing port, and writes an
  audit record, all at once. While remote access is on, the host's window
  shows that it is on, persistently.
- **Turning it on** is only a console act ("Allow other devices", or
  `worktrees serve remote enable --addr <ip>`), which writes
  `<config_dir>/serve-remote.json` (`{addr, port}`). That is a host file the
  frontend never writes whole, and it is never a project-config key: the
  same reasoning as `--allow-origin` being a flag (`policy.rs:17`–`18`).
- **It binds one specific address.** These are refused at parse time and
  again at bind:
  - a wildcard (`0.0.0.0`, `::`);
  - loopback, which would merge the two rule sets decision 6 kept apart;
  - link-local addresses;
  - any public address (§11 Q6).

  The allowed classes are the Tailscale CGNAT range `100.64.0.0/10`,
  ULA `fd00::/8` (Tailscale's is `fd7a:115c:a1e0::/48`), and RFC 1918 for
  LAN mode.
- **A peer filter, as on loopback.** The accept loop drops a peer outside
  the bound address's class, mirroring the `is_loopback()` assertion at
  `conn.rs:342`.
- **An overlay that is not up yet.** If the address is not configured when
  serve starts, serve retries the bind on the host poll. The loopback
  listener never waits for it, and serve logs the state once per episode.
- **Its own `Shared` and its own rule set,** under the same dispatch table
  and the same `Ctx`. Decision 6 says two listeners, two rule sets. A flag
  on the loopback listener would let one of them leak into the other.

### 4.2 TLS: options

| Option | iOS | Pinning | Ops cost | Verdict |
|---|---|---|---|---|
| No TLS on the overlay (WireGuard encrypts) | Easy | None | None | **Rejected.** Every node on the tailnet reaches the port, and a mistaken ACL is a breach. §III.3 and R4 both refuse to put the whole burden on the overlay. |
| A public CA certificate (`tailscale cert`, `*.ts.net`) | Stock trust; works from a WebView | Hostname-based, not identity | Renewal every 90 days; depends on Tailscale and ACME | Rejected for Phase 2: it authenticates a NAME, and anyone with a valid certificate for that name is trusted. |
| A private CA, with a profile installed on iOS | A profile install for every device | CA-based | §III.3 already calls it a support burden | Rejected |
| **Self-signed, pinned both ways (SPKI), TLS 1.3 only, mutual** | Works from Rust; not from a WebView (§4.4) | Exact key | None after pairing | **Chosen** |

**The configuration.** rustls with TLS 1.3 only and its default cipher
suites. rustls 0.23.42, ring 0.17.14 and tokio-rustls 0.26.4 are already in
`Cargo.lock`, through the app's updater (`reqwest`), so the CLI gains a
dependency edge rather than new versions. Certificate generation adds
`rcgen`. Every new crate is pinned and checked on OSV at its locked version
before it is committed, as handshake v1 requires. CI gains `cargo deny check
advisories`, the durable check the serve review asked for (its §4), because
the network-facing binary now carries a TLS stack. TLS 1.3 encrypts the
certificates, so a passive observer does not learn the server's key or a
device's.

**No resumption and no 0-RTT in Phase 2.** rustls does not run the client
certificate verifier again when a session RESUMES: it restores the peer
certificates from the ticket. A device revoked after its ticket was issued
could therefore resume past a verifier-only check. Two rules follow:

- Session tickets and the session cache are off, and early data is off. A
  full handshake costs one Secure Enclave signature per reconnect; §10's
  spike measures that on a phone.
- Whatever is turned on later, the device check runs AFTER every handshake,
  from `peer_certificates()` against the live set, as well as inside the
  verifier. The verifier is the early exit, not the only gate.

**An absolute connection lifetime of 24 hours.** It forces a fresh handshake
and a fresh step-up, even though authorization is already re-read on every
request (§3.3).

### 4.3 The two custom verifiers, kept small on purpose

A custom verifier is where rustls users write their bugs, so each one does
exactly two things:

- **Server side (`ClientCertVerifier`).** The SHA-256 of the client
  certificate's SPKI must be in the live device set and not revoked. The
  handshake signature is checked with rustls's own
  `verify_tls13_signature`, against that certificate's key. Validity dates
  and the issuer are ignored (the certificates are self-signed, and the pin
  is the identity). Client authentication is mandatory. On the pairing
  listener alone, any certificate is accepted, because the pairing proof
  (§2.2) is what authenticates there.
- **Client side (`ServerCertVerifier`).** The SPKI must equal the pin. The
  signature is checked by rustls's helper. The hostname is not checked: the
  pin is stronger than a name.

Each branch gets a negative test with a fixture certificate: wrong key,
right key with a bad signature, revoked, and pairing-only.

### 4.4 iOS, and the shim as the remote door

A page's JavaScript `WebSocket` has no API to pin a certificate or to present
a client certificate. Whether WKWebView's navigation delegate is consulted for
a WebSocket's TLS challenge is not something this design should depend on
(unmeasured). And a key the page can use is a key that an XSS in the page can
use. So:

- **On iOS, iPadOS and a second desktop, the remote socket lives in the Tauri
  shim, in Rust:** rustls with a Secure Enclave signer (iOS) or the key file
  (desktop). The page talks to the shim over Tauri IPC, exactly as it does
  for `ClientLocal` rows today.
- **The shim is a pipe, not a second client.** After TLS, the shim forwards
  text frames both ways: outbound through one shim command, inbound through
  one Tauri `Channel`. The request, channel and ordering logic stays in
  `app/src/ipcws.ts`, so the protocol has one implementation. Two would need
  a drift check, by AGENTS.md's mirror rule. The `ipc.ts` door table gains a
  third door (`remoteDoor`), and `engine-route-check.mjs` holds it like the
  other two.
- **App Transport Security** governs the URL Loading System and WebView
  loads. A socket opened from Rust is outside it. *Verify on a device in
  §10's spike.*
- **Local Network privacy** (iOS 14 and later) prompts for LAN addresses and
  needs `NSLocalNetworkUsageDescription`. Whether a Tailscale `100.x`
  address through the VPN interface prompts too is unmeasured. Overlay mode
  may need no prompt; LAN mode will.
- **Backgrounding** suspends the socket. The client reconnects in the
  foreground with a full handshake, because resumption is off (§4.2). R6 has
  to expect reconnect storms (§6).

### 4.5 The two rule sets, side by side

| | Loopback (decision 6, as built) | Remote (Phase 2) |
|---|---|---|
| Bind | `127.0.0.1` only (`conn.rs:306`) | One configured overlay or LAN address (§4.1) |
| Peer filter | `is_loopback()` (`conn.rs:342`) | Inside the bound address's class |
| Transport | Plaintext | TLS 1.3, mutual, pinned both ways |
| Before HTTP | Connection caps, then hyper | Connection caps, then the rustls handshake. An unknown device ends here. |
| `Host` | Loopback on our port (`policy.rs:33`) | The configured address or name, on our port |
| `Origin` | On the allowlist; missing is refused (`policy.rs:52`–`55`) | **Absent, or the native client's own**: no browser origin is accepted. No CORS header, ever. |
| Who | Whoever holds serve.json's token: the local user | One enrolled, unrevoked device |
| Credential | Per launch, handshake v1 | Per device, persistent, revocable |
| Refusal | Zero bytes written (D6) | During TLS: a TLS alert (rustls), which is unavoidable. A refused upgrade (`Host`, `Origin`, path): the socket closes with nothing written. After the upgrade: `err` frames. |
| Exposure classes served | Engine, HostAdmin, Stream, HostConsole | Engine, HostAdmin and Stream, as the device's capabilities allow; never HostConsole or ClientLocal |
| Scope | `"*"` | The device's |
| Limits | M1 (`conn.rs:61`–`84`) | M1, plus per device (§6) |
| Audit | Mutations, refusals | Everything in §5, attributed to the device |

**The remote handshake.** After mTLS and the WebSocket upgrade, the server's
first frame is `{ok: true, version, device: {id, name, caps, projects}}`.
The client has already authenticated the server through the pin, so it may
send right away. It checks `version` as D5 does, and shows its own
capabilities and scope.

### 4.6 The CI witness: identity, not just reachability (R8)

**First, a debt from Phase 1.** Decision 6 says "the CI witness is a
CONNECT with the served case first". Serve has no such test today. The only
off-loopback connect witness in the repo is the docs server's
(`crates/worktrees-api/src/docserver.rs:1554`–`1617`; `git grep own_lan_addr`
finds only it). Serve binds correctly (`conn.rs:306`), but nothing in CI
would notice if it stopped. Close this first, by copying that test's shape:
a loopback connect that must succeed, then a connect to `own_lan_addr()` on
the same port that must fail.

**The identity witness**, run on both CI platforms, against a real serve
whose remote listener is bound to the runner's own LAN address
(`own_lan_addr`, which the docs server's test records as measured on both CI
platforms). The bind rules (§4.1) must accept that address. A runner whose
address falls outside them FAILS the test rather than skipping it, because a
witness that quietly skips is no witness. Each step names what it proves:

1. The loopback client completes handshake v1 and lists the workspace. This
   is the served case FIRST, so a dead server fails here instead of passing
   every refusal below.
2. A remote TLS client with the right pin and an enrolled certificate is
   served `list_workspace`. This is the remote served case, and it must come
   before any remote refusal.
3. No client certificate: the handshake fails, and no application byte
   arrives.
4. A valid self-signed certificate that is not enrolled: refused at TLS.
5. The wrong server pin on the client side: the client refuses. This catches
   a verifier that accepts anything.
6. Scope: a device scoped to project A asks `read_file` in B, then
   `read_file` of a missing path in A. The two errors are identical, and the
   audit log has both denials with their real reasons.
7. Revocation: revoke the device in 2 while its socket is open. The socket
   closes within one second, and reconnecting with the same certificate is
   refused at TLS.
8. Pairing: with no window open, a connect to the pairing port is refused.
   Open a window and send one bad proof: the window closes, and a connect is
   refused again.
9. The audit file is `0600`, and its hash chain verifies.
10. The kill switch: with the device in 2 connected, turn remote access off.
    Its socket closes, and connects to the remote port and the pairing port
    are both refused.

**Two platforms.** AGENTS.md's bind lesson applies: a refusal measured on
one OS is a claim about one OS. Each assertion is measured on macOS and
Linux before it is written down, with `docker run --rm -v "$PWD":/w
node:22-alpine` or the Linux runner for the Linux half.

**The release gate.** The app-bundle job already runs the docs server's
boundary tests by exact name and greps `test result: ok. 4 passed`
(`.github/workflows/release.yml:134`–`146`). Add a second step of the same
shape for the serve witness, with its own exact count, because a renamed
test turns `--exact` into a filter that matches nothing and exits 0.

---

## 5. The audit log (R5)

### 5.1 The log

**The file.** `<config_dir>/audit.jsonl`. Serve opens it with `O_APPEND`,
`O_NOFOLLOW` and mode `0600` at creation. It refuses to start the remote
listener if the file exists with another owner or a group or world bit,
rather than chmod it silently. It rotates at 5 MiB and keeps five files. The
rotation is itself a record.

**Who can write it.** Only an `Audit` value that serve's connection layer
owns (`crates/worktrees-cli/src/serve/`). It is not in `Ctx`, and
`worktrees-api` does not depend on `worktrees-cli`, so no command body can
write a line. Records are a serde struct serialized whole. There is no
`format!` of client strings into a line, so a client-supplied name, path or
command cannot add a line. That is L1's class, closed by construction. The
`cmd` field is the `&'static str` from `TABLE`. An unknown command is logged
as `"unknown"` with the length of what was sent.

**Each record:**

```json
{"seq": 812, "ts": "2026-10-20T14:03:11Z", "pid": 4242, "kind": "call",
 "device": "d_k3v9q2", "conn": 17, "peer": "100.101.7.3",
 "cmd": "write_file", "cap": "instruct", "project": "/Users/bret/Projects/x",
 "object": "AGENTS.md", "outcome": "ok", "ms": 4,
 "detail": {"bytes": 2310, "sha256": "…"}, "prev": "<sha256 of the previous line>"}
```

`device` is `"loopback"` for console clients. The `prev` hash chain makes a
partial edit detectable. It cannot stop a same-user process from rewriting
the whole file, and nothing here claims that it can (§1).

**What it records:**

- lifecycle: the listener started or stopped, with version, address and
  identity pin; remote access enabled or disabled;
- pairing: a window opened, an attempt with its result and reason, Allow or
  Deny, the window closed;
- devices: enrolled, scope changed, revoked (by whom), disabled while idle,
  identity rotated;
- authentication failures: an unknown certificate, a revoked certificate, a
  failed handshake v1. These are aggregated per episode with counts, like
  `conn.rs`'s `capped` log, so a flood does not turn into a disk-writing
  flood;
- every refusal: an exposure, a capability or scope, a limit tripped;
- every mutation: every row whose capability is not `read`, with its
  outcome. A destructive one records what it destroyed in a form that can
  be recovered: `remove_place` with `del_branch` records the branch's tip
  SHA, so `git branch <name> <sha>` restores it while the object exists
  (AGENTS.md: `force` plus `--branch` is the one remove that can destroy
  commits);
- streams: attach opened and closed, with the total bytes written. **Not one
  record per keystroke.**

**What it never records:** argument contents (file bodies, keystrokes, note
text, pastes), tokens, secrets, proofs. For `write_file` it keeps the path,
the size and a SHA-256 of the content, so a write can be matched against a
file later without the log holding the file.

**Reading it.** `audit_tail` is a read under `admin` with scope `"*"`, and
console. Settings → Devices shows each device's recent records. The
host-admin line serve writes to serve.log today (`conn.rs:575`–`579`) moves
here.

### 5.2 Records that should not exist

A same-user process, an agent with a shell among them (§1), can write
`devices.json` itself. So at boot, and whenever serve reloads the file, it
cross-checks every device against the audit chain. A device with no
`enrolled` record behind it is DISABLED until the console confirms it, and
the host raises an alert (§5.3). A corrupt `devices.json` refuses every
remote device and is never overwritten, the registry's "refused rather than
clobbered" rule. A same-user writer can forge the chain too. This makes the
forgery a two-file, hash-chained job instead of one JSON edit, and it puts a
banner in front of the user either way.

### 5.3 Alerts

ASVS L2 asks for alerting, not only logging. A file nobody reads alerts
nobody, so these raise a host notification and a badge on Settings →
Devices:

- a device paired, or remote access turned on or off;
- authentication failures past a threshold in one episode;
- a revoked or disabled device trying to connect;
- a device seen from a new address class;
- a device record with no pairing record (§5.2);
- a limit tripped by one device repeatedly (§6).

### 5.4 L1's two remainders, fixed in the same slice

- `log_event` keeps newlines, and it truncates at byte 4000, which panics in
  the middle of a UTF-8 character (`crates/worktrees-api/src/meta.rs:27`–`28`).
  Strip control characters and truncate on a character boundary.
- serve.log is opened with no mode, so it gets the umask's `0644`
  (`crates/worktrees-cli/src/serve/mod.rs:131`). Create it `0600`.

---

## 6. Resource limits (R6)

M1 bounded the pre-auth surface (`conn.rs:61`–`89`): 64 connections, 16 of
them unauthenticated, 512 bytes before auth, 16 MiB per message after. What
it does not bound is an AUTHENTICATED client. Three unbounded structures sit
behind a client that has proven itself, and each is one loop away from the
host's memory or threads:

- **Every request gets its own blocking task** (`conn.rs:545`). Tokio's
  default ceiling is 512 blocking threads. One device looping
  `list_workspace`, a git fan-out, can hold all of them, and the desktop's
  own requests queue behind it.
- **The outbox is an unbounded channel** (`conn.rs:498`). A slow WAN reader
  with a busy terminal grows the server's memory with no limit.
- **A lane per distinct stream key** (`conn.rs:217`–`237`) is a parked task
  and a map entry for the life of the connection, keyed by whatever `id` the
  client names. `term_write` for ids 1…10⁶ makes a million of them.

Proposed limits. The numbers are starting points, to be measured on both
platforms before the gate (§10):

| Limit | Value | When exceeded |
|---|---|---|
| Enrolled devices | 16 | Pairing refused before the QR appears |
| Remote connections, total / per device | 32 / 4 | New connection closed at accept, or after TLS for the per-device cap |
| TLS handshakes in flight / per source address | 8 / 2 | Closed at accept |
| TLS handshake deadline | 5 s | Closed |
| Keepalive | A ping every 20 s; closed after 60 s with no pong | Closed; the client reconnects |
| In-flight requests per connection / per device | 16 / 32 | `err: "busy"` at once, never queued |
| **Separate blocking budgets** for loopback and remote | Remote capped at 64 concurrent bodies | Remote gets `busy`. Loopback, the person at the desk, is never starved by a device. |
| Request rate per device (non-stream) | A token bucket, 50/s, burst 200 | `err: "rate"` |
| Stream input per attach | 4 MiB/s | Excess dropped, and the client is told |
| Outbox per connection | 8 MiB | A byte stream over budget: its channel ends (`{end: true}`), and the client attaches again (tmux repaints). A reply over budget: the connection closes. |
| Lanes per connection | 64. The key comes from the RESOLVED, owned stream (§3.3), never from a raw argument. | `err: "busy"` |
| Live attaches and shells per device | 8 + 8 | `err: "busy"` |
| Agent launches per device (LLM10) | 10 an hour; at most 4 live lanes launched by devices at once. The existing quota gate (`set_quota_settings`) still applies on top. | `err: "rate"`. A stolen or buggy device must not burn the user's Claude or Codex plan. |
| Pairing | One window, one attempt | §2.4 |
| Audit file | 5 × 5 MiB | Rotation |
| Remote `MAX_MESSAGE` | 16 MiB, as on loopback. `read_file` stops at 1 MB, and the measurements at `conn.rs:76`–`83` hold. | The connection ends |

**Coalescing beats rate-limiting for the poll-shaped reads.**
`list_workspace`, `project_prs`, `claude_usage`, `codex_usage`, `kimi_usage`,
`doctor`, `diagnostics`, `list_releases` and `check_update` become
single-flight host-wide: concurrent identical calls share one computation,
and results are filtered per caller afterwards (§3.3). Three phones polling
cost one git fan-out, not three.

All of these fail closed, as `err` frames to an authenticated caller and
closed sockets before authentication. Each trip is logged once per episode,
with counts.

**A panicking body must still answer.** The job a request runs pushes its
reply only when the body RETURNS (`conn.rs:539`–`541`). A body that panics
is contained by `spawn_blocking`, which is what the serve review measured,
but its request is never answered, so the client waits forever. Under
per-connection caps it would also hold an in-flight slot forever. So the job
runs the body under `catch_unwind` and answers `err: "internal"`, and the
in-flight count is a drop guard that comes back however the job ends.

---

## 7. The agent-instruction surface (R7)

### 7.1 One classifier, in core

`worktrees_core::pathclass::classify(project_root, rel) -> Class`. Every
`file(…)` resolution uses it (§3.3): `write_file`, `read_file*`, `list_dir`
(which shows restricted names but marks them), and the docs index. Matching
is on the CANONICAL relative path (after `guard_under_projects`), so a
`notes.md -> AGENTS.md` symlink is classed as what it writes. Matching is
ASCII case-insensitive, because APFS is (AGENTS.md: "macOS FS is
case-insensitive").

| Class | Paths (fixture-tested) | Read needs | Write needs |
|---|---|---|---|
| `forbidden` | `.git/**`; a place's `.git` gitfile; `.worktrees.places.json` (setters own it); `.worktrees/` itself | never, for devices | **never, for any caller** |
| `executes` (runs without anyone choosing to run it) | `.claude/settings.json`, `.claude/settings.local.json`, `.claude/hooks/**`, `.mcp.json`, `.pi/extensions/**`, `.codex/config.toml`, `.kimi-code/**` | `read` | `stream` (a device that already has a shell) |
| `instruction` | `AGENTS.md`, `CLAUDE.md` and `GEMINI.md` at any depth; `.planning/**`; `.claude/**` (other than the above); `.agents/**`; `.codex/**`; `.pi/**`; `.cursor/**`; `.github/copilot-instructions.md`; `.worktrees.toml` | `read` | `instruct` |
| `secret` | `[[file]]` declared sources and destinations (`projcfg`); `.env` and `.env.*` | `stream` | `stream` |
| `plain` | everything else | `read` | `write` |

**Why `.git/**` is closed to every caller, the desktop included.**
`core.fsmonitor` in `.git/config` runs on the next git spawn, and the
engine's own 3-second poll spawns git. So a write there is code execution
within seconds, by the engine, with no human or agent in the loop. The cost
is that the Files tab cannot edit `.git/config`. A terminal can, and the
person at the desk has one.

**Why `executes` needs `stream` and not `instruct`.** A hook in
`.claude/settings.json` is a command that runs at the next tool call. That
is not steering an agent, it is running code, so only a device that can
already run code may write it. `instruct` is for text an agent weighs.

**Why `secret` reads need `stream`.** An Observe device should not read a
project's credentials. A device with a shell can `cat` them anyway, so the
rule costs it nothing.

### 7.2 What a remote device may write, by capability

| Capability | It may write |
|---|---|
| `write` | Plain files; notes, titles, pins, lifecycle; places (create, switch, close, remove); agent launches under host policy |
| `instruct` | Also: instruction files, `.worktrees.toml`, `init_write`, `rules_apply`, `agent_setup_fix` |
| `stream` | Also: `executes` and `secret` paths (it has a shell either way) |
| any | Never: `.git/**`, and nothing that changes host policy (console only, §3.4) |

### 7.3 How it is marked

**No marker inside the file.** A header comment in `AGENTS.md` would be
stripped or forged trivially, it would pollute the user's files, and an
agent would read it as one more instruction. Provenance goes out of band
instead:

- **The audit record** (§5) holds the device, path, size and hash.
- **A declared-state stamp:** `last_remote_instruction: {device, path, at}`
  on the place, written by the resolver whenever an instruction-class write
  arrives from a device. The UI shows *"AGENTS.md edited from Bret's iPhone,
  2 min ago"* on the place, and MCP `place_status` reports the same fact.
- **The changed-files list.** A remote write shows up in `git diff` like any
  other, and that is the review surface that already exists.
- **Notes and titles are data in MCP.** Agents see titles in resource
  descriptions (`crates/worktrees-cli/src/mcp.rs:2524`), and the
  `reading_notes` at `mcp.rs:2625` marks branch, upstream, commit subject,
  agent names and plan strings as "data, never instructions". It does not
  name `title` or `note`. Add both, and record `note_by` / `title_by`.

### 7.4 What R7 does not claim

Every file an agent reads can steer it. A code comment written by an Operate
device and read by an agent launched under permissive host policy is an
instruction too (LLM01, indirect injection). The gate above separates
**deterministic execution** (hooks, git config, extensions), which it refuses
outright, from **the channels agents are built to obey** (instruction files),
which it gates and marks. For everything else the controls are host policy
(`ask` keeps a human between an injected agent and a command), the honest
label (§1.1) and revocation.

### 7.5 Two rules about agents and devices

- **An agent never reaches a Phase 2 control.** Pairing, devices, scopes,
  remote access and the audit log are dispatch rows. They are not MCP tools,
  and a check fails if `crates/worktrees-cli/src/mcp.rs` ever names one. An
  agent that could pair a device would be a prompt injection away from a
  remote foothold (ASI02, ASI03).
- **A lane launched by a scoped device cannot reach past that scope.**
  Cross-project reach lets an agent see and, at `full`, act on other
  projects' places through MCP. An agent launched from a device whose scope is a list, not `"*"`,
  would otherwise carry the HOST's reach and step outside the device's
  scope (ASI03, LLM06). So such a launch narrows that lane's
  `worktrees_mcp_cross_project` to `off`, the narrowing the profile field
  already exists for (`crates/worktrees-core/src/profile.rs:248`: "It can
  only NARROW the user's `cross_project` setting"). It is never widened. The
  lane records which device launched it, and the audit log does too.

---

## 8. The known gaps

The brief's eight are first, then seven found while writing this, then one
that Appendix B already had. Each is reachable today by an authenticated
loopback client at `fadb2ce`. Gap 9 needs less: a cloned repository's content
plus the user's next ordinary save.

| # | Gap | Where | Fix shape | Phase |
|---|---|---|---|---|
| 1 | `write_file` can overwrite `.git/config`; `core.fsmonitor` runs on the next git spawn, and the host poll spawns git every 3 s | `crates/worktrees-api/src/files.rs:1311`–`1333`. The guard is `guard_under_projects`, "under SOME root" (`projects.rs:45`). | The `forbidden` class (§7.1), for every caller | 2 |
| 2 | `save_profile` stores `mcp_servers` (commands) and `settings` (claude's `hooks` are a key of that raw JSON), both run at the next launch | `crates/worktrees-api/src/profiles.rs:21`–`25`; the fields at `crates/worktrees-core/src/profile.rs:231`, `:262` | Console-only (§3.4), together with `set_project_profile` and `set_default_profile`, which choose where a profile runs | 2 |
| 3 | `term_open` passes `session` to `attach-session -t` unanchored, so a name prefix-matches another session. `tune_session` is unanchored too. Any tmux session on the host can be attached, worktrees' or not. | `crates/worktrees-api/src/term.rs:65`, `:75`; `crates/worktrees-core/src/tmux.rs:530`–`533` | `session(…)` resolves the name to a place in scope, then passes `=<name>`. `ai_pane` is the precedent (`tmux.rs:583`–`587`). | 2 |
| 4 | `sync_apply` with no `repo` adopts: the rsync `--delete` destination is the hub manifest's `local_root`, checked only for being non-empty and not a hub copy. Deletions go to a backup dir, but the tree is mirrored over whatever path the manifest names. | `crates/worktrees-core/src/sync.rs:229`, `:1253`, `:1263`; the api passes the client's `confirmed` (`crates/worktrees-api/src/project.rs:138`) | Phase 2: the adopt form is console-only. Later, a destination rule: must not exist or must be empty, must sit under a configured parent, never `$HOME` or an ancestor of a registered root. | 2 (rule: 3) |
| 5 | Containment under `[docs]`: `resolve_rel` and `root.join(rel)` walk through an intermediate symlink. `create_initial_commit("")` runs git in serve's own cwd (AGENTS.md: `git -C ""` means HERE). `resolve_doc_paths.root` runs `git ls-files` in an unguarded directory, and an untrusted repo's `core.fsmonitor` runs there. A `..` slug in `shell_open`, `list_branches` and the rest. | `crates/worktrees-core/src/docs.rs:451`; `ROADMAP.md:1074`; `crates/worktrees-api/src/workspace.rs:558`–`563`; `files.rs:1212`–`1240`; `project.rs:500`–`505` | R1's resolver (`project`, `root`, the slug rule) for the last three. For `[docs]`, ROADMAP's own shape: lstat each component, and accept a link only if its target canonicalizes back under the place root, which is `docserver::safe_under`'s rule. **These must close before any remote bind** (§III.3a R1). | 2 |
| 6 | `privacy_denial` (PR #519) is deliberately unguarded. For any path it tells the caller whether that path is refused under a macOS privacy area. | `origin/feat/tcc-recovery:app/src-tauri/src/lib.rs:5512`; `macprivacy.rs:197`–`222` | When #519 meets the stack it becomes an api body and a dispatch row, which `engine-route-check.mjs` will demand. Make it `read · project` with the path checked LEXICALLY against registered roots: canonicalizing could fail on the very denial it exists to name, and the roots are already visible to the caller. It runs in serve, because the denial is the engine's. Its remedy names the RESPONSIBLE process's bundle id, which serve has no `app.config()` to read, so the shim passes it at spawn as it passes `WORKTREES_VIEWER_JS`. A remote client shows the card with "fix this at the host" and greys out the System Settings button. *Unmeasured: TCC attribution for a serve the app started.* | 2 |
| 7 | `open_docs_viewer` answers with `http://127.0.0.1:<port>/<token>/…`. That is meaningless off the host, and it carries the docs server's token. | `crates/worktrees-cli/src/serve/dispatch.rs:387`–`392`; `crates/worktrees-api/src/files.rs:870` | Phase 2: a remote caller gets `err: "unavailable-remote"` and no URL, and its UI greys the button with that reason. Phase 3: serve the pages through the authenticated socket (a `docs_get` read row); the docs server never listens off loopback. | 2, 3 |
| 8 | `sanitize_prefix` keeps a leading `-`, and the prefix can come from `[project] prefix` in a cloned repo's `.worktrees.toml`. This lane found no call site where a session name is a positional argument rather than a `-t`/`-s` value; it is defence in depth for R3. | `crates/worktrees-core/src/config.rs:16`–`27`, `:308` | Map a leading `-` to `_`. A session already named with a leading `-` is found by the old name once and renamed. | 2 |
| 9 | **New. Fixed-name temp files follow a planted symlink.** `write_file` writes `.<name>.wt-tmp` beside the target with `std::fs::write`, which follows a symlink, then chmods it and renames it over the target. A repository that tracks `.README.md.wt-tmp -> ../../.zshrc` turns the user's next save of `README.md` into a write of that content into `~/.zshrc`. `README.md` then becomes the link. The same shape is in every declared-state write (`.worktrees.places.json.tmp` in the repo root) and in `init_write` (`.worktrees.toml.tmp`). | `files.rs:1323`–`1328`; `crates/worktrees-core/src/store.rs:303`–`304`; `crates/worktrees-core/src/init.rs:1014`–`1015` | Create the temp file with `create_new(true)` (`O_CREAT \| O_EXCL`, which refuses an existing path, a dangling link included), with a random suffix, as `servefile::write` already does (`servefile.rs:108`–`113`). Sweep core for the same pattern. *Found by reading, not measured. The regression test must be shown failing first.* | **Now**: a local bug, independent of Phase 2 |
| 10 | **New. `repo` is never checked against the registry.** `set_note` and the other `store::edit` callers write `.worktrees.places.json` into any directory. `Project::discover` accepts any git repository. | §3.5 | The `project(…)` resolver, for every caller | 2 |
| 11 | **New. Serve has no off-loopback connect witness**, although decision 6 says it does | §4.6 | Copy the docs server's test | 2 (first) |
| 12 | **New. Events go to every socket**, including the host-wide `sessions:busy` set and `app:open-doc` | `conn.rs:202`–`207`; `host.rs:316` | Projection per event type (§3.3) | 2 |
| 13 | **New. An authenticated client is unbounded:** a blocking task per request, an unbounded outbox, a lane per key | `conn.rs:545`, `:498`, `:217`–`237` | §6 | 2 |
| 14 | **New. Arbitrary-path reads exposed as Engine rows:** `probe_dir`, `skill_inspect`, `sync_hub_list` | `dispatch.rs:228`, `:447`, `:317` | `admin` with `new_path` or `global` (§3.4) | 2 |
| 15 | **New, a residue of L1:** `log_event` keeps newlines and panics on a mid-character truncation; serve.log is `0644` | `meta.rs:27`–`28`; `mod.rs:131` | §5 | 2 |
| 16 | Known (Appendix B): `term_*` acts on any id, from any connection, and ids restart at 1 every launch | `term.rs:44`, `:138`; `dispatch.rs:422`–`429` | `attach(id)` (§3.3), plus a launch nonce in the id | 2 |

---

## 9. Multi-client effects (§III.6), now that one engine serves several clients

### 9.1 Per device, or per host

| State | Today | Ruling | Phase |
|---|---|---|---|
| View state in `ui-state.json`: selection, sidebar mode, sort, terminal visibility, dock open, tab and width, `place_panels`, `files_md_zoom`, theme | One blob, written whole by `set_settings` (`dispatch.rs:243`) | **Per device**, kept by the client: the shim's config dir on a desktop, the app container on iOS. It never reaches serve. A phone's selection is not the desktop's (§III.6.1). Theme follows each device's own appearance. | 2 |
| `last_seen_version` ("What's new") | The same blob, so the first client to load after an upgrade uses it up for everyone (Appendix B) | Per device | 2 |
| Shared preferences (default provider and the like) | The same blob | On the host, written by `patch_settings {key, value, rev}`: merged under a lock, refused if `rev` is stale. It is the `expected_mtime` compare-and-swap shape (`files.rs:1316`–`1320`) applied to a key. `key` comes from a CLOSED list of shared keys. A host-policy key or a per-device key is refused, so a client cannot reintroduce M2 through the settings door (API3, mass assignment). No shared key holds data keyed by project, so `get_settings` leaks nothing to a scoped device. | 2 |
| The five host policies | Two in `host-policy.json`; fetch interval and the two terminal switches still pushed by the page (Appendix B) | All five in `host-policy.json`, set only by explicit acts. Codex and pi are console-only; the other three are `admin` (§3.4). | 2 |
| `ui-events.jsonl` | Appended by any client | Loopback only (§3.4) | 2 |
| `mark_seen` | Stamps the client's clock, forward-only (Appendix B) | The host stamps its own `now`, and the client's epoch is ignored | 2 |
| `open_place` closing the second agent | Decided by whichever client selects the place first, using its own default (Appendix B) | Not decided by a client on load. The server decides, using the host's default, or the user is asked. | 2 |
| A dock shell's single sink | A second attach silences the first pane (`term.rs:438`–`462`) | A sink per attach. The most recently active attach owns the size, which is tmux's `window-size latest`. | 2 |
| Terminal ids | A global counter from 1, no owner (gap 16) | Owner check plus a launch nonce | 2 |
| `app:open-doc` | Sent to every socket, so one `worktrees show` opens the document on every device | Loopback clients only: `worktrees show` is run by someone at the host | 2 |
| Events in general | Every socket | Projected to the scope (§3.3) | 2 |
| Dates | Host-local (`sysclock::date_cache`) | Displays take the device's time zone, as `ui_usage` already does with `tz_offset_min` | 3 |
| The docs viewer URL | Loopback | Refused remotely, then proxied (gap 7) | 2, 3 |

### 9.2 The answer lease: a compare-and-swap first, a lease second

§III.6.3 asks for a server-side lease so two clients cannot both answer one
prompt. The failure it names is "the loser's tap lands on whatever the
prompt became". A lease prevents that only while it is held correctly. A
compare-and-swap prevents it outright:

- **`agent_answer {place, choice, fingerprint}`** (capability `answer`,
  Phase 3). `fingerprint` is the hash of the prompt the user SAW: the modal's
  lines, recognised by its footer, as `activity::codex_panes` already
  recognises a modal (AGENTS.md: "keys on the modal's FOOTER being the bottom
  line, never on its question text"). The server captures the pane again
  and refuses if the fingerprint changed. `choice` must be one of that
  prompt's own options. Free text is `instruct`, not `answer`.
- **The lease is UX on top.** `agent_lease_take` gives one client the
  prompt, and the others see "being answered on <device name>". The lease
  expires after 30 s idle or when the prompt changes. Without a lease, two
  answers race, one wins, and the other fails its compare-and-swap with "the
  prompt changed". That is correct, just less polite.
- **The phone shows the WHOLE prompt.** An answer approves a command the
  agent wrote. The verb's UI renders the full modal, scrolling if it has to,
  before the choice can be tapped, and there is no "approve all" (ASI09). A
  truncated command approved on a small screen is the trust exploitation
  this list warns about.
- **Raw keystrokes stay raw.** Two `stream` devices typing into one pane are
  two people at one keyboard, which is what two attached tmux clients
  already are. The compare-and-swap is for the verb, not for `term_write`.

Phase 2 reserves the `answer` capability and the audit kind. Phase 3 ships
the verb and the lease, with the phone, as §III.9 planned.

---

## 10. Order of work and exit criteria

Each slice merges on its own, and nothing binds off loopback until slice 6.

0. **Local fixes, no remote:** gap 9 (temp files), gap 11 (the loopback
   connect witness), gap 15 (L1), gap 8, `catch_unwind` on every body (§6),
   `cargo deny` in CI (§4.2), and the three host policies still pushed by
   the page (Appendix B, "One fix for all five").
1. **The dispatch table:** `HostConsole`, the capability and object columns,
   the resolver, result and event projection, and the tests in §3.3.
   Loopback behaviour is unchanged apart from §3.5. Gaps 2, 3, 4 (the
   console tag), 5 (all but the `[docs]` walk), 7 (the remote refusal), 10,
   12, 14 and 16.
2. **The path classifier** (§7.1): gap 1, and the `[docs]` walk (gap 5).
3. **The audit log** (§5).
4. **Resource limits** (§6), including the loopback budget split: gap 13.
5. **The device registry, server identity and pairing** (§2): console rows,
   the pairing listener, the QR, the comparison code, revocation, step-up,
   and the cross-check and alerts (§5.2, §5.3).
6. **The remote listener** (§4), behind the opt-in, with the shim's remote
   door and the identity witness (§4.6). Every gap in §8 is closed or
   tagged by now. Gap 6 lands whenever #519 meets the stack, as its row says.

**A spike, before slice 5's design is frozen:** on a real iPhone (TestFlight)
and a second Mac on the tailnet, measure the claims marked unmeasured above:

- rustls from the Rust shim with a Secure Enclave signer;
- whether ATS or Local Network privacy touches a socket opened from Rust, on
  `100.x` and on a LAN address;
- what a full handshake costs per reconnect with a Secure Enclave signer,
  since resumption is off (§4.2);
- a `.userPresence` step-up signature requested from Rust (§2.5);
- keychain behaviour for an ad-hoc-signed macOS build.

**Exit, which is §III.9's, made concrete:**

- [ ] A second machine on the tailnet, paired by QR, drives serve: list,
      mutate, one terminal attach.
- [ ] An unenrolled device cannot connect, measured at TLS.
- [ ] A revoked device's live socket dies within 1 s, and it cannot
      reconnect.
- [ ] Every step of §4.6 passes on macOS and Linux, and the release gate
      greps its exact count.
- [ ] R1: every row declares its object and capability, the drift tests are
      green, and each one was shown failing first.
- [ ] R4: pairing as §2. R5: §5. R6: §6, with the numbers measured. R7: §7.
      R8: §4.6. R9: answered (§11, Q1).
- [ ] Gaps 1–16: each closed, or tagged with its phase in this table and in
      ROADMAP.

---

## 11. Questions only the user can answer

1. **Multi-user, ever? (R9)** The recommendation is a recorded NON-GOAL: one
   human, several devices. The schema relies on it (project scopes, no
   per-user git identity, one tmux server). If the answer is "maybe", the
   device record needs an owner field now.
2. **Does the overlay's identity stand in for enrollment, or sit under it?
   (§V.1)** The recommendation is under: our own device keys always, with
   the overlay as transport and network ACL.
3. **Is `update_cli` refused from a remote device outright? (§V.2)** The
   recommendation is yes (console-only). Updating from a phone would mean
   going to the desk anyway.
4. **Which preset does the pairing dialog pre-select?** The recommendation
   is **Observe**, with Drive one deliberate click away and carrying its
   label. The cost: every phone paired to answer Claude needs that click.
5. **May an enrolled device open a pairing window?** The recommendation is
   no: console only (§2.4). The cost: someone away from the desk cannot add
   a device.
6. **LAN mode, and public addresses.** The recommendation: Phase 2 supports
   the overlay and RFC 1918 LAN addresses, and refuses public addresses
   outright. Should LAN mode ship in Phase 2 at all, or wait until the
   overlay path is proven?
7. **Idle expiry.** Disable a device unseen for 90 days? Recommended yes.
8. **Step-up (Face ID or passcode) for drive-class acts: on by default?**
   (§2.5) The recommendation is ON for iOS devices that hold `stream`,
   `instruct`, `answer` or `admin`, with a proof that lasts 5 minutes per
   connection. That is the ASVS L2 second factor for sensitive operations.
   The cost: the first terminal or answer after each foreground costs a
   glance at the phone.

---

## 12. The OWASP review of this design

The draft was reviewed with the `owasp-security` skill against OWASP Top
10:2025, the API Security Top 10:2023, ASVS 5.0 at L2 (V1–V4, V6–V9, V12–V14
and the new V17 on WebSockets), the Agentic Top 10 (ASI01–ASI10) and the LLM
Top 10. It was a review of a design, so every finding below is a change to
this document, already folded in, not a bug in shipped code. The exceptions
are two facts the review checked in the code and found true today: O6 and
O15.

**What held up.** Deny by default, through an exhaustive table with a
fail-closed resolver. Authentication before parsing: unknown devices stop
inside rustls. No bearer secrets. TLS 1.3 with mutual pinning, and channel
binding by exporter. A one-attempt pairing window. Structured audit records
that client strings cannot add lines to (V7/V16, closing L1's class). Path
classes for code that executes. The honest ladder (§1.1), which keeps scopes
from being sold as containment. WebSocket rules (V17): Origin checked,
authenticated before any message, size and rate limits, TLS underneath, no
token in a URL.

**What it changed.** Eighteen changes:

| # | OWASP | Finding in the draft | Change, and where |
|---|---|---|---|
| O1 | A01, API1 | The dispatcher checked ARGUMENTS, but `resolve_term_paths`, `resolve_doc_paths`, `list_workspace` and `drop_reference` enumerate every registered root inside their bodies, and would answer about projects outside the scope. Nested arguments (`places`, `place`, `shell`, `roots`, `paths`) had no declared object. | The scope rides into the api; `project_roots`, `read_projects` and `guard_under_projects` answer the caller's roots, so the api guard is R1's second check. Nested arguments are resolved element by element. (§3.3) |
| O2 | A01, API5 | "Loopback" was not pinned to anything a request cannot influence. | The caller kind is fixed by the listener object, never by a header, an Origin or an address. The remote listener ignores `--allow-origin`. (§3.3) |
| O3 | A01, A06 | Revocation closed sockets, but nothing re-checked a socket that survived, and an `admin` device could widen scopes, its peers' included. | The device record is read per request. Revocation cancels the device's jobs. Only the console widens a scope; `admin` devices narrow or revoke others. (§2.5, §3.2) |
| O4 | A07, API2 | §4.4 relied on TLS resumption. rustls does not re-run the client verifier on resumption, so a revoked device could resume past it. | Resumption and 0-RTT off in Phase 2. A device check after every handshake from `peer_certificates()`. A 24-hour absolute connection lifetime. (§4.2, §4.4) |
| O5 | API3 | `set_settings` took a whole blob, so a client could put host policy back in it: M2 through another door. `get_settings` could carry other projects' keys. | `patch_settings` takes a closed list of shared keys, and refuses host-policy and per-device keys. Shared keys carry no project-keyed data. (§9.1) |
| O6 | API7 (SSRF), A01 | `clone::parse_source` accepts `file://` (`clone.rs:68`). An `admin` device could clone an unregistered repo from the host's disk, or make the host fetch from its own private network. | From a device, `file://` and loopback, link-local and private hosts are refused. (§3.4) |
| O7 | ASVS V2 (L2), A07 | No second factor for sensitive operations. | A step-up key in the Secure Enclave with user presence, for `stream`, `instruct`, `answer` and `admin`. A desktop's single factor is labelled. (§2.1, §2.5, §11 Q8) |
| O8 | ASVS V8 | "What a lost phone costs" ignored what the app had cached on it. | No persistent cache of contents, diffs, terminal bytes, drafts or transcripts. What is stored uses Data Protection `Complete`. (§2.5) |
| O9 | A09, ASI10 | Logging without alerting, and no single off switch. | Host alerts for six event kinds, and a one-click kill switch for remote access. (§4.1, §5.3) |
| O10 | A08, ASI02 | The threat model put prompt-injected agents in scope and same-user processes out of it. An agent with a shell is both, and could write `devices.json` directly. | The overlap is stated. Device records with no pairing record are disabled and raise an alert. No Phase 2 control is ever an MCP tool, and a check holds that. (§1, §5.2, §7.5) |
| O11 | ASI03, LLM06 | An agent launched by a scoped device inherited the HOST's cross-project reach, which is an escape from the device's scope. | Lanes launched by a device with a listed scope get cross-project reach narrowed to `off`. (§7.5) |
| O12 | LLM10, API4 | Nothing bounded how many agents a device could launch, so a stolen or buggy device could burn the user's plan. | 10 launches an hour and 4 live device-launched lanes, on top of the quota gate. (§6) |
| O13 | A05, ASVS V5 | The new inputs were not specified: the device name, and the pairing URI as the DEVICE parses it. | The name is limited in length and stripped of control and bidi characters. The URI parser is strict, with an address-class allowlist and `exp`. (§2.2) |
| O14 | A04, A06 | The comparison code protected only a user who looks, and the race-lost case gave no guidance. | Host copy: "allow only if this code is on your device right now". The device that lost the race says to Deny. A QR on a shared screen, and an attacker's QR, are named as residuals. (§2.2) |
| O15 | A10, API4 | A body that panics is contained, but its request is never answered (`conn.rs:539`–`541` pushes only on return), so the client hangs, and in-flight caps would leak a slot. A corrupt `devices.json` had no rule. | `catch_unwind` answers `err: "internal"`, and the in-flight count is a drop guard. A corrupt file refuses every device and is not overwritten. (§5.2, §6, slice 0) |
| O16 | A06, A09 | A destructive act from a device left no way back. | `remove_place` with `del_branch` records the tip SHA in the audit log. (§5.1) |
| O17 | A03 | A TLS stack joins the network-facing binary with no advisory check in CI. | `cargo deny check advisories` in CI. (§4.2, slice 0) |
| O18 | ASI09 | Answering on a small screen invites approving a truncated command. | The answer verb renders the whole prompt, and there is no approve-all. (§9.2) |

**Considered and not changed.**

- *Client-certificate expiry dates.* The pin is the identity and revocation
  is immediate, so validity dates would add rotation work without closing
  anything. Idle expiry (§2.5) covers forgotten devices.
- *A hardware-backed server key* (ASVS V6.4.2). macOS builds are ad-hoc
  signed (§2.1), so the key stays a `0600` file. That is recorded as a
  residual, to revisit with a team-signed build.
- *Making `executes` paths console-only* instead of `stream`. A `stream`
  device already has a shell, so console-only would cost the user something
  and protect nothing.
