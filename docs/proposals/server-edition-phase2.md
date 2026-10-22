---
title: "Proposal — Server Edition, Phase 2"
---

# Proposal — Server Edition, Phase 2: identity, capability, exposure

**Status:** design only, 2026-10-20. Nothing here is built. It is the gate
before `worktrees serve` answers anything but this machine: a second desktop,
and the Tauri iOS and iPadOS build (decision 3 of
[server-edition.md](server-edition.md)). Nothing binds off loopback until
every exit criterion in §10 holds.

**Decided 2026-10-20.** The user answered the eight questions of §11 the
same day. Six follow the recommendation. Two depart from it: multi-user is
"maybe later", so the schema keeps room for it (§2.8), and expiry is set per
device from the host rather than by one host-wide rule (§2.5). LAN mode
ships in Phase 2 (§4.1). This revision folds all eight in, and §11 records
each answer with its consequence. The user then answered the three
questions those answers raised (§11.1). One of them also departs from the
recommendation: a host is rediscovered over mDNS in Phase 2 (§4.7).

**Status at v0.43.0 (2026-10-21).** Phase 1 has shipped. `worktrees serve`
hosts the engine on loopback behind handshake v1 and M1's limits (#516).
The desktop app is its client (#515, #524). The app reconnects on its own,
with backoff and a crash-loop limit (#525), and carries the CLI as a private
sidecar, Linux bundles included (#513, merged with #525). Atomic writes no longer follow a planted
symlink (#523). Phase 0's extraction (#508) is under all of it. None of this
document is built. Of its sixteen gaps (§8), #516 and #523 closed 9, 11 and
15 and closed part of 3 and 5. #516 also answers a panicking body (§6). The
rest still stands as proposed: identity and pairing (§2), per-object
authorization (§3), the remote listener and its witnesses (§4), the audit log
(§5), limits on authenticated clients (§6), the path classes (§7) and the
multi-client rulings (§9). Shipping Phase 1 also changed one thing here. The
app now reconnects, including to a serve it restarted. So Appendix B's
terminal-id fix is no longer latent, and a restarted serve runs the three
page-pushed host policies at their defaults (§9.1). Phase 1's lanes also
left three questions for this design. A held `worktrees show` goes to any
socket, and an app-owned serve's idle stop counts any socket; §9.3 answers
both. The plugins act on the client's machine; §4.4 answers that.

**Base.** Written against `feat/app-serve-client` at `fadb2ce`, the tip of the
Server Edition stack (#508 → #516 → #515 → that branch) before it was
squash-merged. Every `file:line` below is at that commit unless it names
another branch. Some PRs took more commits after `fadb2ce`, so where a claim
has changed on main, the text says so and names the PR. Where this document
and the code disagree, the code wins.

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

| # | Question | Ruling | Phase |
|---|---|---|---|
| 1 | How does a device enroll? | A QR shown on the host carries the server's SPKI pin and a 128-bit one-use secret. The pairing listener exists only while a window is open, and the window closes after one attempt or 120 s. Only the host opens one (*decided*, §11 Q5). The device proves the secret over pinned TLS, bound to the session by a TLS exporter. Both screens show a short code, and the user confirms on the host. The dialog pre-selects Observe (*decided*, §11 Q4). | 2 |
| 2 | What is a device's credential? | Its own P-256 key: in the Secure Enclave on iOS, and in a `0600` file on macOS and Linux. It authenticates with mutual TLS, so an unenrolled peer never reaches the HTTP parser. There are no passwords and no bearer tokens. The server's record binds the key to a device and the device to a principal (row 14). On iOS, the riskiest acts also need a second, presence-bound key (a step-up), on by default (*decided*, §11 Q8). | 2 |
| 3 | Revocation, expiry, and a lost phone | A revoke takes effect at once: the device's live sockets close, its next request on any socket is refused, and its next TLS handshake fails. The host console or another device of the same principal with `admin` can revoke it, but only the console can WIDEN a scope. Each device carries its own expiry, chosen on the host at pairing (default: disabled after 30 days unused, for every preset; *decided*, §11.1 N3) and edited from the host only. An expired device is disabled, not deleted, and only the host re-enables it (*decided*, §11 Q7). A stolen locked phone cannot connect. An unlocked one keeps its scope until it is revoked or expires, and the audit log shows what it did. Clients cache nothing they can lose. | 2 |
| 4 | May the loopback listener enroll? | It is where enrollment is AUTHORIZED, and never where a device enrolls. Opening a pairing window is a console act, done over loopback with serve.json's token. The loopback listener never accepts a device credential. | 2 |
| 5 | Per-object authorization (R1) | Each device has capabilities (`read`, `write`, `answer`, `instruct`, `stream`, `admin`) and a project scope. Every row of the dispatch table declares the capability it needs and the object it acts on. One resolver checks that before the body runs, against the grant of the (principal, device) pair: the principal's grant intersected with the device's. The scope also rides into the api, so the api's own path guard answers with the caller's roots: two checks, as R1 asked. Results and events are cut down to the scope. | 2 |
| 6 | Rows a remote device never reaches | A new `HostConsole` class, served on loopback only. It covers every row that can widen what agents on this host may do, pairing and device administration, `update_cli` (*decided*, §11 Q3), and adopting a project from a hub. | 2 |
| 7 | The remote listener | Off by default, with a one-click kill switch on the host. It binds one specific private address on a named interface, an overlay address or a LAN one (*decided*, §11 Q6), never a wildcard and never loopback. "Private" is a closed list (RFC 1918, CGNAT `100.64.0.0/10`, link-local, ULA `fc00::/7`), and anything else is refused. When the address leaves its interface or the network changes, the listener closes until the same network returns or the host chooses again. A new address on the same network rebinds by itself, at most three times an hour (*decided*, §11.2 N4). On a LAN, devices find a moved host over mDNS: it is advertised only in LAN mode and on the bound interface, nothing serve composes in it is more than an opaque id, and a discovered address is adopted only after a pinned handshake (*decided*, §11.1 N2). Its host name is serve's own opaque one where the platform allows it (recommended, unmeasured); elsewhere it is the OS's `.local` name, which can carry the owner's first name, and the enable dialog shows it (§4.7). An address code is the fallback. TLS 1.3 is mutual and pinned both ways (rustls), with no CA, no resumption and no 0-RTT in Phase 2. On iOS the socket lives in the Rust shim and not in the WebView. | 2 |
| 8 | The CI witness (R8) | A connect witness for loopback serve, which #516 added to CI (it still needs its release gate). Then the bind rules, as connects: a public address that IS bindable is refused, the remote port never answers on loopback, a peer outside the allowed sources is dropped, and an address change pauses and resumes the listener. Then identity: no certificate is refused, an unenrolled one is refused, an enrolled one is served, a revoke kills the live socket, an expired device is refused, and the pairing port is closed outside a window. Measured on both platforms, with a release gate that greps the exact count. | 2 |
| 9 | The audit log (R5) | `<config_dir>/audit.jsonl`, mode `0600`, append-only and hash-chained. Only serve's connection layer writes it, and its records are structured, never free text. Every record names a principal as well as a device. | 2 |
| 10 | Resource limits (R6) | Per-device connection, in-flight and rate caps; bounded outboxes and lanes; single-flight for the expensive reads; separate budgets for loopback and remote, so a device cannot starve the desk. | 2 |
| 11 | The agent-instruction surface (R7) | One path classifier in core. Instruction files need `instruct`. Files that EXECUTE without anyone choosing to run them need `stream`. `.git/**` is never written. Provenance is kept out of band, with no markers in the file. | 2 |
| 12 | The known gaps | Eight from the brief, seven found while writing this, and one from Appendix B. Each has a fix shape and a phase (§8). One of the new ones (gap 9) could be reached from a cloned repository's content. #523 fixed it on main. #516 closed gaps 11 and 15 and part of gaps 3 and 5. | 2, some now |
| 13 | Multi-client | View state is per device and never reaches serve. Shared preferences become per-key patches with a revision check. Host policy goes to `host-policy.json`. A prompt answer is a compare-and-swap, and a lease is UX on top of it. A held `worktrees show` goes only to a loopback client that listens for it, and an app-owned serve idles out on loopback clients alone, so a device never keeps it running (§9.3). | 2, lease 3 |
| 14 | Multi-user (R9) | Maybe later (*decided*, §11 Q1, against the recommendation). Not built in Phase 2, and not designed out. There is one principal, kept distinct from the device in the device record, in what the credential binds to and in every audit record, and every grant check keys on (principal, device). §2.8 says what that costs now and what multi-user would still take. | 2 |

Rows marked *decided* carry the user's answers of 2026-10-20, and §11 gives
each answer with its consequence. The rest are this design's
recommendations.

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
- **Several humans, in Phase 2.** R9 is decided as "maybe later" (§11 Q1).
  There is exactly one principal, the host's own user, and the schema keeps
  room for more (§2.8). Scopes keep one person's devices to least privilege.
  They are not isolation between people, and §2.8 says why a second person
  holding a shell would be the owner in all but name.
- **Browser clients over the remote listener.** Decision 3 makes the phone a
  Tauri build. A browser cannot hold the device key (§4.4), so a browser
  origin is refused remotely.
- **Exposure on the public internet.** Refused at bind time, and a public
  peer is dropped at accept even when a router forwards a port to the
  private bind (§4.1; decided in §11 Q6).

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
| Server identity | A P-256 key and a self-signed certificate. Its SPKI SHA-256 is the **pin**. Beside it, a **discovery id**: 128 random bits, not derived from the key or anything public, that the mDNS advert carries (§4.7). | `<config_dir>/server-identity.key` (PKCS#8, `0600`) and `.crt`; the discovery id in the same directory, `0600` | Created the first time remote access is enabled. Reused across launches, because a pin must survive a restart, unlike the loopback token. Rotating it means pairing every device again (§2.5), and the discovery id rotates with it. |
| Device key (iOS, iPadOS) | P-256 in the Secure Enclave, non-exportable, `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`, never synced to iCloud | The device's keychain | Until it is revoked or the app is deleted |
| Device key (macOS, Linux) | P-256, in a `0600` PKCS#8 file | The client app's own config dir | As above |
| Step-up key (iOS, iPadOS) | A second Secure Enclave P-256 key, created with `.userPresence`. It signs a server challenge before a drive-class act (§2.5). | The device's keychain | Enrolled with the device key, revoked with it |
| Principal record | `{id, name, caps, projects, created_at}`. Phase 2 has exactly one, the **owner**: the host account's user, with every capability and scope `"*"`. Its `id` is the reserved constant `owner`, so the console never reads the file to know who it is; a later principal would get a random `p_…` id. The loopback caller is always the owner (§2.8). | `<config_dir>/devices.json`, as a second list beside the devices | Created with the file. Never removed in Phase 2. |
| Device record | `{id, principal, name, platform, spki_sha256, stepup_spki_sha256, stepup_required, caps, projects, expiry, enrolled_at, last_seen, last_addr, disabled, revoked_at}`; `id` is random, not derived from the key. `principal` is set at pairing and never changes. `expiry` and `disabled` are §2.5's. | `<config_dir>/devices.json`, `0600`. Serve is its only writer, under `servefile`'s lock-temp-rename discipline (`crates/worktrees-cli/src/serve/servefile.rs:79`–`122`). | Kept after revocation as a tombstone, for the audit trail |
| What the device keeps | Its keys, plus `{host: {name, pin, addr, discovery_id}, principal_id, device_id}`. The pairing answer and the remote handshake's first frame name both ids (§2.2, §4.5). | Beside the device key, under the same protection | Until it is revoked or the app is deleted |
| Loopback token | Unchanged: per launch, in serve.json, used for handshake v1 | `<config_dir>/serve.json` | Per launch |

Four choices in that table need their reasons.

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
  not have. Measured (spike, 2026-10-21, macOS 27 on Apple silicon, two
  ad-hoc builds): an ad-hoc rebuild is refused the previous build's
  login-keychain item, and the data-protection keychain and a persistent
  Secure Enclave key fail with `-34018`. A Secure Enclave key that is NOT
  stored in the keychain (CryptoKit) needs no entitlement, and its blob, in
  a `0600` file, survived a rebuild; the key itself cannot leave the Mac.
  Prefer that over a PKCS#8 file where a Secure Enclave exists, once the
  Rust signer path is settled. A `0600` file is the same
  protection serve.json has today, and it matches the threat model (§1).
  Revisit when macOS builds are signed with a team identity.
- **`<config_dir>`, not `~/.config/worktrees/`.** §III.5 put `devices.json`
  beside the registry. It belongs beside `serve.json` and `host-policy.json`
  instead, in the config dir the engine was started with (`Ctx::config_dir`,
  `crates/worktrees-api/src/lib.rs:115`). A sandbox serve
  (`app/scripts/sandbox.sh --app`) or a dev-stack serve runs on its own config
  dir, and it must not accept the user's real phone. The registry is shared
  between engines and the CLI on purpose. Device enrollment must not be.
- **A principal beside every device, though there is only one.** The user's
  answer to R9 is "maybe later" (§11 Q1). The device's certificate carries
  no claim about either identity: it is self-signed, so anything in it would
  be the device's own assertion. The binding from key to device to
  principal lives only in the server's record. So a device can never claim a
  principal, and a second principal later is a new record, not a new
  credential format. §2.8 has the rest.

### 2.2 Enrollment, step by step

**On the host.** Settings → Devices → **Pair a device…**, or
`worktrees serve pair`, which is a thin client of the loopback socket.

1. The user chooses the device's **scope first**: a preset (§3.1), its
   projects, and its **expiry** (§2.5). The QR appears only after that, so
   the decision is not made with a timer running. The dialog pre-selects
   **Observe** (decided, §11 Q4), with Drive one deliberate click away and
   carrying its label. The project list starts empty, so a scope is always
   chosen, never inherited. The expiry starts at the default, *disabled
   after 30 days unused*. The device's principal is the owner, and the
   dialog does not ask (§2.8).
2. Serve opens the **pairing window**, which binds a separate pairing
   listener on the remote address. The host shows:
   - a QR encoding `worktrees-pair:v1?addr=<address>:<port>&pin=<spki sha256>&secret=<128-bit, base32>&host=<host name>&exp=<epoch>`.
     `addr` is an address literal, never a name, so the device classifies
     what it will dial before it dials it (§4.1);
   - the same string as a link, for a device with no camera (§2.3);
   - a 120 s countdown.
3. The device scans the QR and parses it strictly before it dials
   anything. It refuses an unknown parameter, an expired `exp`, and an
   `addr` outside §4.1's address classes, judged by the same classifier
   serve uses: a QR is untrusted input, and a public address in one is a
   phishing attempt. Then it makes its keys
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
   · disabled after 30 days unused · Allow only if this code is on your
   device right now · [Deny] [Allow]"*.
   **Allow is the consent act**, in the same sense as the registry's
   "registering is the consent act" (§III.5). Deny closes the window. A
   device that finds the window already used says so in words that point at
   the host: *"Someone else used this pairing code. Tap Deny on <host>."*
   Pairing events go to loopback clients only (§3.3).
7. On Allow, serve writes the record into `devices.json` and answers
   `{paired: {device_id, principal_id, host_name, remote_addr, expiry,
   discovery_id}}`. Then it closes the pairing listener. From now on the device connects to
   the remote listener with its key (§4).

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
  even one with `admin` (decided 2026-10-20, §11 Q5). Pairing started
  remotely would turn a stolen device into a lasting foothold: enroll a
  second device, then outlive the stolen one's revocation.
- **A device cap.** At most 16 enrolled devices (§6). Disabled devices
  count; revoked tombstones do not. Pairing beyond the cap is refused before
  the QR appears.

### 2.5 Revocation, expiry, rotation, and what a lost phone costs

**Revoking.** It is available from the host console, from any enrolled
device that holds `admin` and belongs to the same principal, and from a
device on itself. Serve marks the record (`revoked_at`) and keeps it as a
tombstone. Then it closes every live connection of that device: it keeps a map from device id to connection ids
for exactly this. That also ends the device's tmux attaches, through the
cleanup that already exists (`crates/worktrees-cli/src/serve/conn.rs:566`–`569`).
It cancels the clone jobs that device started. A body already running
(a sync, say) finishes, its reply is dropped, and the audit log records
both. Serve writes an audit record for the revoke. Authorization reads the
device's CURRENT record on every request (§3.3), so a socket that outlived
the close for any reason is refused at its next request. The TLS verifier
reads the same in-memory set, so the next handshake fails before HTTP.

A device with `admin` revoking a DIFFERENT device of its own principal is
allowed on purpose. If the phone is lost while the user is travelling with
the laptop, the laptop must be able to cut it off. With one principal that
is every other device. The rule is written against the principal anyway, so
that a second principal, if one ever exists, cannot revoke the owner's
devices (§2.8). The cost: a stolen `admin` device can revoke the
user's other devices. That is a denial of service which the audit log
records, and pairing again at the desk undoes it. "Revoke every device but
this one" is console-only.

**What a lost phone costs.**

- **Locked:** nothing. The key is usable only while the device is unlocked
  (`WhenUnlockedThisDeviceOnly`), so the TLS handshake cannot be signed.
- **Unlocked, or unlocked later:** its scope, until it is revoked. Section
  1.1 says what that scope is worth.
- **Lost without anyone noticing:** its scope until its expiry disables it
  (below), so a forgotten device does not keep standing access forever.
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

**Expiry, per device (decided, §11 Q7).** There is no host-wide rule. Each
device record carries its own `expiry`, chosen on the host at pairing (§2.2)
and edited later from the host console only (`device_set_expiry`):

- `{"idle_days": N}`: disabled once N days pass with no completed handshake.
  **The default is 30**, for every preset (decided, §11.1 N3).
- `{"until": "<RFC 3339 time>"}`: disabled at that moment, for a device
  that is temporary on purpose, such as a borrowed laptop.
- `"never"`: allowed, and the device list marks it.

Why the default is 30 days unused, and why it counts idle time rather than
running to a date:

- A device in use connects every day, so weekends and a two-week holiday
  stay well inside 30 days, and a device the user is actually using never
  trips it. An expiry that fires on a live device teaches the user to
  re-enable without looking, and then it protects nothing.
- A device lost without anyone noticing, retired to a drawer, or handed back
  after a loan loses its standing access within a month, not a season.
- A fixed date would also expire devices in daily use, with the same
  result, so the date form is kept for devices that are temporary by
  design.

**What "seen" means.** `last_seen` moves on a completed, authorized
handshake, never on a refused one, so a disabled device cannot keep itself
alive by knocking. The 24-hour connection lifetime (§4.2) means a device
that stays connected still handshakes every day. `last_seen` is kept in
memory and written to `devices.json` at most every ten minutes and at exit.

**Where it is checked.** In three places, so that no path waits on another:

- the client-certificate verifier (§4.3) refuses an expired device at the
  handshake, and serve marks the record;
- a timer set for the earliest `until` among connected devices disables
  that device when it fires, and closes its live connections. A check that
  waited for the next request would leave a terminal attach streaming,
  since an attach sends bytes to the device without being asked (§3.3);
- the host poll marks a device whose idle window has run out even if it
  never tries again, so the Devices list and the alert (§5.3) do not wait
  for a knock.

An expiry is decided once and recorded as `disabled`, never recomputed from
the clock at each check. So setting the host's clock back cannot re-enable
an expired device. `until` is compared with the host's wall clock, and the
idle rule compares `last_seen` on that same clock.

**Disabled is not revoked.** A disabled device keeps its record, its keys
and its scope, and the verifier refuses it exactly as it refuses a revoked
one. Disabling, for any reason, closes the device's live connections the
way a revoke does. `disabled: {at, reason}` says why: `idle`, `until`,
`unrecorded` (no pairing record behind it, §5.2) or `console` (paused by
hand, `device_disable`). Only the host console re-enables a device
(`device_enable`), and re-enabling restarts its idle clock from that moment.
An `admin` device can neither re-enable a device nor change an expiry, not
even to shorten one, because §11 Q7 puts expiry in the host's hands. Every
transition is an audit record. The device learns its own expiry in the
remote handshake's first frame (§4.5), so the phone can say when its access
ends.

**Step-up for drive-class acts (iOS).** The connection key has no presence
requirement, so status, notifications and reads stay seamless. Opening a
stream, an `instruct` write, an `answer` and any `admin` row also need a
fresh signature from the step-up key (§2.1), which asks for Face ID or the
passcode. Serve sends a challenge, the device signs `challenge || exporter`,
and the proof stays valid on that connection for 5 minutes. That is the
second factor ASVS L2 asks of sensitive operations: possession of the
Secure Enclave key, plus presence. A desktop device has no presence-bound
key while macOS builds are ad-hoc signed (§2.1). Its drive-class acts
therefore rest on one factor, and the device list says so.

Step-up is **on by default** (decided, §11 Q8): every iOS and iPadOS device
record that holds `stream`, `instruct`, `answer` or `admin` has
`stepup_required: true`. Turning it off for a device removes a factor, so it
is a widening: a console act (`device_set_stepup`), audited, and shown in
the device list for as long as it stays off.

**Rotating the server identity.** `worktrees serve identity rotate`, from a
console only. Every device has to pair again. Rotation is meant for a
suspected key compromise, and in that case a rotation signed by the old key
would be signed by the attacker too. So there is no signed handover, and the
cost is honest: one pairing per device.

### 2.6 The loopback listener and enrollment (decision 6's open half)

**Ruling: the loopback listener authorizes enrollment and never performs it.**

- The rows that open a pairing window, accept or deny a request, list
  devices, set a device's expiry or step-up, disable or re-enable a device,
  revoke all others and rotate the identity are a new `HostConsole` class
  (§3.4). They are served only on loopback, to a client that passed
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
| The overlay's identity alone (Tailscale ACLs stand in for enrollment) | Least code; Tailscale handles rotation and NAT | The security boundary belongs to a third party, and a shared tailnet node or a mistaken ACL reaches the engine. There is no per-device capability, so R1 has nothing to key on. It also violates R4's "the overlay should not carry the whole burden". | Rejected. The overlay **sits under** enrollment as transport and network ACL (decided, §11 Q2). |
| Per-device bearer tokens | Simple | Copyable. A token lands in logs and backups, and from there in any process that reads them. No channel binding. | Rejected |
| Passwords or a PIN | Familiar | Guessable. Needs rate limiting. Shared across devices. | Rejected |
| An application-layer signature after TLS (handshake v2 over plain TLS) | No custom TLS verifier | The HTTP and WebSocket parsers run for unauthenticated peers. The channel binding has to be built by hand. | Rejected in favour of mTLS |
| **Mutual TLS with pinned self-signed certificates** | Unenrolled peers stop inside rustls, before HTTP. The binding comes from TLS itself. The key works with the Secure Enclave. | Two small custom verifiers (§4.3) | **Chosen** |
| Noise IK (`snow`) | Pinning is native; no X.509 | X25519 only, so it cannot use a Secure Enclave key. A second protocol stack. | Rejected |

### 2.8 Principals: one now, room for more later (R9)

R9 is decided as "maybe later" (§11 Q1). Phase 2 builds nothing for a
second person, and it designs in nothing that would have to be undone for
one. Three things carry that.

- **A principal is its own identity, beside the device.** `devices.json`
  holds principals and devices as two lists (§2.1), and every device names
  the principal it belongs to, set at pairing and never changed. Moving a
  device to another principal is a revoke and a new pairing. Phase 2 has
  exactly one principal, the **owner**: the host account's user. The
  loopback caller is always the owner, because whoever holds serve.json's
  token holds the account. The owner's id is the reserved constant `owner`
  and is never looked up, so a corrupt or deleted `devices.json` cannot lock
  the console out or split the audit trail under a new id. The Phase 2
  loader treats any other principal in the file as corruption (§5.2: every
  remote device refused, the file kept, an alert). It is a state no code
  path creates, so it is refused rather than handled. Neither identity is in
  the device's certificate (§2.1), and the pairing answer and the remote
  handshake both name the two ids separately.
- **Every grant check keys on the pair.** The caller is
  `Caller { principal, device }`, where `device` is `Console` or
  `Device(id)`, and the listener sets it (§3.3). What the caller may do is
  the principal's grant intersected with the device's. The owner's grant is
  every capability and scope `"*"`, so today the intersection is exactly the
  device's grant and nothing behaves differently. The intersection is
  defined for `"*"` as well, and it is evaluated on every request, never
  stored: `"*"` ∩ X is X, so a device whose projects are `"*"` under a
  principal with a list has that list and nothing more, and a project added
  later does not reach it through the device's `"*"`. A row that needs scope
  `"*"` (`new_path`, `all`, `log_tail`) needs the EFFECTIVE scope to be
  `"*"`, which means on both sides. A rule that names another device names
  it within the caller's principal: an `admin` device revokes and narrows
  only devices of its own principal (§2.5, §3.2), and `device(id)` resolves
  that way (§3.3). Result and event projections are computed from the
  pair's grant.
- **Every audit record names its principal** (§5.1), and so do the
  provenance stamps on places (§7.3) and the record of who launched a lane
  (§7.5).

**What this costs now.** One field in each device record and each audit
line, one more list in `devices.json`, and one intersection per
authorization. In return, `devices.json` needs no migration if multi-user
ever comes. There is no UI for it: the pairing dialog does not ask for a
principal, and the device list does not show one. The real cost is code that
no production path reaches. A device grant wider than its principal's must
be refused, and so must an `admin` device acting on another principal's
devices, and with one principal neither can happen. So the unit tests build
a second principal in a fixture and assert both refusals, each shown failing
first. Otherwise the rule rots where nobody can see it.

**What multi-user would still take.** The schema is the cheap part. The rest
is product work, and some of it is a different product:

- **Principal administration.** Creating and removing principals, setting a
  principal's grant, and opening a pairing window FOR a principal. All of it
  console only, like every widening.
- **One Unix account underneath.** Every agent, shell and git call runs as
  the host user, with that user's git identity, SSH keys, `gh` token and
  Claude and Codex plans. A second principal holding `stream`, `answer`, or
  `instruct` under permissive host policy can run commands as the owner
  (§1.1), so in practice it IS the owner. Until there are per-user Unix
  accounts, which means one serve per account, a second principal would
  have to stop at the rungs of §1.1 that are a boundary: `read`, and `write`
  within its own projects.
- **Per-place ownership.** A scope finer than a project needs a durable place
  identity with an owner, and places have neither (§3.2).
- **Per-person state.** Git authorship per lane, shared preferences per
  principal (§9.1), an answer lease that names a person rather than a device
  (§9.2), and launch budgets per principal (§6), so that one person cannot
  spend another's plan.
- **Scopes that isolate.** §1.1 would have to become true above `read`, and
  today it is not.

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

Custom sets are allowed, and the label is computed, never chosen. The
pairing dialog pre-selects Observe (decided, §11 Q4).

### 3.2 Scope: projects, not places

`projects` is either `"*"` (every registered project, including ones added
later) or a list of canonical project roots, in the registry's own keys.
**Phase 2 scopes by project only.** Places are short-lived and their slugs
get reused. A per-place scope would need a durable place identity, which
does not exist, and it pays off mainly for multi-user, which is deferred
(§2.8).

A project removed and added again at the same path is the same project. A
project added later is outside an explicit list until the user adds it to
the device. **Only the console can WIDEN a scope**: add a capability, add a
project, or turn a list into `"*"`. An `admin` device may NARROW another
device of its own principal, or revoke it, and never changes itself. Every
change is audited. Two stolen `admin` devices therefore cannot widen each
other, and no device can raise its own privileges. A device's grant can
never be set wider than its principal's, and the check intersects the two
anyway (§2.8).

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
     single component that names an existing place. "Single component" means
     no `/` and no control characters, both checked by the resolver. Core's
     `safe_arg` refuses less: a leading `-`, any `..`, and CR, LF and NUL
     (`crates/worktrees-core/src/argv.rs:42`–`57` on main). Since #516,
     `shell_open` runs `safe_arg` and then checks that the slug is a place,
     which is the shape this generalizes.
   - `root(arg)`: a place root, meaning a registered root or a direct child
     of its `.worktrees/`. `plan_ask_main` already checks this shape by hand
     (`crates/worktrees-api/src/files.rs:704`–`710`).
   - `file(arg)`: `guard_under_projects`, then the owning project, then the
     path class (§7). The class can RAISE the capability the row needs: a
     `write_file` to `AGENTS.md` needs `instruct`.
   - `session(arg)`: the session must belong to a place in scope, looked up
     in the snapshot's place list (adopted sessions included). Anchoring is
     already done: since #516, `term_open` refuses a session that does not
     exist and attaches `=<name>`, and `tune_session` targets `=<name>:`, as
     `tmux::ai_pane` did before (`crates/worktrees-core/src/tmux.rs:587`).
     What the resolver adds is the scope check.
   - `attach(id)`: the id is in this connection's own set (`Frames::note_term`,
     `crates/worktrees-cli/src/serve/wire.rs:226`). That is Appendix B's
     `term::NEXT_ID` fix. `term_write`, `term_resize` and `term_wheel` on an
     id outside the set are refused with `no such terminal`, the error an
     unknown id gets today. `term_close` on one is a no-op that answers `ok`,
     which is what it already answers for an unknown id, and the audit log
     records it as a refusal (§9.1 says why).
   - `shell(repo, slug, index)`: `place(repo, slug)` and a `u32` index.
   - `job(id)`: a clone job this connection started.
   - `device(id)`: a device of the CALLER's principal (§2.8). A device of
     another principal, which Phase 2 cannot have, gets the same error as
     an id that does not exist, and every device listing is filtered to the
     caller's principal the same way.
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
   that applies) must be in the caller's grant, and `Resolved.project_root`
   must be in its scope. The grant is the (principal, device) pair's: the
   principal's capabilities and projects intersected with the device's
   (§2.8). A row that needs scope `"*"` needs the effective scope to be
   `"*"`. The loopback caller is the owner on the console, with every
   capability and scope `"*"`.
4. **The audit record** (§5), for every refusal and every mutation.
5. **The body**, unchanged except for one thing it is now handed: the
   caller's scope (below).

**Who the caller is comes from the listener, never from the request.**
`Caller { principal, device }` is set by the listener object that accepted
the socket: the owner on `Console` for the loopback listener, and for the
remote listener the device whose key completed the handshake, with that
device's principal. It is a type, fixed for the life of the connection. It
is never derived from a header, an `Origin` or a peer address, so a
tunnelled connection, a forged header or a dev `--allow-origin` page cannot
become a different kind of caller. The remote listener does not read
`--allow-origin` at all.

**The records are read per request, not per connection.** The device's
record and its principal's are looked up on every request through an
in-memory generation counter (one atomic load when nothing has changed). A
narrowed scope, a revoke, a device disabled by hand or a passed `until`
(§2.5) therefore applies to the very next request on a socket that is
already open.

**Requests are not the only traffic, so a reduction acts on what is already
flowing.** A terminal attach, a dock shell and the event stream reach a
device without any request from it, and a per-request check alone would
let them run on. Every change that REDUCES a grant therefore also acts on
the live connections:

- a revoke or a disable closes the device's connections (§2.5);
- a narrowed scope or a removed capability ends each open channel whose
  object it no longer covers, with `{channel, seq, end: true}`;
- event projection reads the grant at the moment of each emit, never a copy
  taken when the socket connected.

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
  missing and out-of-scope objects get identical errors. For `term_close`
  the identical answer is `ok`.

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
| `update_cli` | HostAdmin | It replaces the binary that is serving the request. Refused from every remote device (decided 2026-10-20, §11 Q3); I9 flagged it. |
| `set_codex_permissions`, `set_pi_trust` | HostAdmin | Host policy (decision 7, M2) |
| **`set_pi_allowed`, `set_kimi_allowed`** | HostAdmin | They allow a repo's own code to load in pi or kimi |
| **`set_cross_project`, `set_project_private`** | HostAdmin | They widen how far agents reach across projects |
| **`mcp_install`, `codex_mcp_install`, `pi_mcp_install`, `kimi_mcp_set`, `codex_mcp_migrate`** | HostAdmin | They give agents tools, mutating ones with `mutations` |
| **`save_profile`, `set_project_profile`, `set_default_profile`** | HostAdmin | `mcp_servers` and `settings` (claude's `hooks` live in that raw JSON) are commands run at the next launch. Binding a profile to a project chooses which of them run there. |
| **`set_agent_guidance`, `set_agent_guidance_skill`, `agent_guidance_merge`, `agent_link_skills`, `skill_install_local`, `skill_install_git`** | HostAdmin | Text and scripts that every agent on the host loads |
| **`sync_apply`, in its adopt form (no `repo`)** | HostAdmin | It writes to a path the hub's manifest chooses (§8, gap 4) |
| *new:* `pair_begin`, `pair_decide`, `device_set_expiry`, `device_set_stepup`, `device_disable`, `device_enable`, `devices_revoke_all_others`, `remote_enable` (also how the bind moves to a new address, §4.1), `remote_advertise` (the mDNS switch, §4.7), `identity_rotate` | — | §2.5, §2.6, §4.1, §4.7 |

**admin** (served remotely to a device with `admin`; Phase 2 keeps the
loopback behaviour the same):

| Object | Rows |
|---|---|
| `new_path` (needs scope `"*"`) | `add_project`, `create_project`, `clone_project` (from a device, `file://` URLs and hosts that resolve to loopback, link-local or private addresses are refused: `clone::parse_source` accepts `file://`, `crates/worktrees-core/src/clone.rs:68`, and a remote device must neither read an unregistered repo on the host's disk nor borrow the host's network position), `init_repo`, **`probe_dir`** (an Engine read today; it tells the caller whether any host path exists and is a repo), **`skill_inspect`** (Engine today; it reads `SKILL.md` and lists files at any host path) |
| `job(id)` | `clone_cancel` |
| `device(id)`, of the caller's own principal | *new:* `device_revoke` (a device may always revoke itself, without `admin`), `device_narrow` |
| `project` | `create_initial_commit`, `remove_project`, `set_project_planning`, `mcp_status`, `mcp_uninstall`, `sync_apply` (with `repo`) |
| `all` | `set_planning_default`, `set_quota_settings`, **`set_fetch_interval`**, **`set_term_history_opts`** (both host policy, Appendix B), **`term_history_clear`**, **`ui_events_clear`** |
| `global` | `codex_mcp_status`, `codex_mcp_uninstall`, `codex_mcp_migration_plan`, `pi_mcp_status`, `pi_mcp_uninstall`, `kimi_mcp_status`, `delete_profile`, `skill_remove`, **`skill_preview_git`** (fetches any URL from the host), **`sync_hub_list`** (lists every project's local path on the hub), **`diagnostics`** (the host's PATH, versions and log paths), **`log_tail`** (serve.log names every project's places, so it also needs scope `"*"`), **`term_history_info`**, **`profiles_info`** (profiles can carry credentials in an MCP server's `env`) |

**stream:**

| Object | Rows |
|---|---|
| `shell(repo, slug, index)` | `shell_open`, `shell_write`, `shell_resize`, `shell_detach`, **`close_shell_session`** (Engine today) |
| `session(session)` | `term_open` (anchored since #516; scope-checked here, §8 gap 3) |
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
| `global` | `set_settings` (it becomes a per-key patch; §9), `log_event` (L1 closed by #516; rate-limited) |
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
`write_file` (its path's class, §7) and `sync_apply` (the adopt form). Rows
marked *new* do not exist yet and are not in the count. This paragraph is
not what holds the count. A test is, and it asserts that every
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
  `worktrees serve remote enable --addr <ip> --iface <name>`, the
  `remote_enable` row), which writes `<config_dir>/serve-remote.json`
  (`{addr, port, iface, network}`; `network` is the fingerprint below). That
  is a host file the frontend never writes whole, and it is never a
  project-config key: the same reasoning as `--allow-origin` being a flag
  (`policy.rs:17`–`18`).
- **It binds one specific private address, on a named interface.** Overlay
  mode and LAN mode both ship in Phase 2 (decided, §11 Q6). Never a
  wildcard, and only an address the interface holds at the moment of the
  bind. For a point-to-point interface, the name follows the address (see
  the fingerprint, below).
- **A peer filter, as on loopback,** mirroring the `is_loopback()`
  assertion at `conn.rs:342`. It has three parts, and they do not have the
  same standing (below):
  1. **Every peer's address passes the classifier.** A router that forwards
     a public port to the private bind then does not make serve reachable
     from the internet: the forwarded connection arrives from a public
     source and is dropped before TLS. A router that also rewrites the
     source to its own LAN address defeats this, and then mTLS is the gate
     that remains.
  2. **On an overlay bind** (a point-to-point interface), the peer is in the
     bound address's own row of the table below: `100.64.0.0/10` for a
     Tailscale IPv4 bind, `fc00::/7` for its IPv6 one. The listener is also
     tied to its interface: `SO_BINDTODEVICE` on Linux, `IP_BOUND_IF` /
     `IPV6_BOUND_IF` on macOS (`socket2`'s `bind_device` and
     `bind_device_by_index_v4`/`_v6`; socket2 0.6 is already in the CLI's
     tree, through tokio, with the `all` feature these calls need).
     Measured (spike, 2026-10-21): on Linux 6.12 it scopes a listening
     socket for both families, a peer on another interface gets a RST, and
     an unprivileged process may set it. On macOS 27 it scopes an IPv4
     listener (silently, no root); `IPV6_BOUND_IF` did not filter same-host
     traffic and its cross-host effect is unmeasured (§10). Either way it
     pins the interface INDEX, so a poll that sees the index change treats
     it as the address leaving. Same for LAN binds.
  3. **On a LAN bind,** the peer is on-link: inside the bound address's own
     subnet. A link-local bind is on-link by definition. A peer routed in
     from another subnet, such as an office network joining `10.x` and
     `192.168.x`, is refused in Phase 2.

  **Part 2 exists because a host may accept a packet for any of its
  addresses on any interface.** Linux uses the weak host model, and
  `rp_filter` checks the route back to the SOURCE, not where the
  destination lives (measured, spike, 2026-10-21, Linux 6.12: a LAN
  neighbour reached a listener on a TUN's `100.x` and `fd7a:` address
  under `rp_filter` 0, 1 and 2). So a neighbour on the host's Wi-Fi can
  send to the host's Tailscale address straight to the host's link-layer
  address, never touching the tailnet, and the kernel hands the connection
  to a listener bound there. The tailnet's ACLs, which §11 Q2 made the
  network layer under pairing, would then cover nothing. macOS does not,
  by default: its `net.inet.ip.check_interface` is 1 (read on macOS 27 by
  the same spike), a hybrid model that is strong for `utun` and `ipsec`
  interfaces, for IPv4 and IPv6 (XNU's `IP_CHECKINTERFACE_HYBRID_ES`, read
  in its source), but only while IP forwarding is off, and not for an
  overlay on any other kind of interface. The drop itself, seen from a
  second machine, is unmeasured (§10). Requiring a source in the overlay's
  own row closes the gap wherever the kernel does not, which is why it
  stays a requirement on both. A forged source in the row is dropped by
  Tailscale's own Linux firewall (CGNAT sources off `tailscale0`; read in
  its source, not measured) or by `rp_filter=1` (measured); failing both,
  the reply follows the host's route to that source, which for a real peer
  is the tunnel, so the handshake does not complete (measured on Linux: the
  SYN-ACK left through the TUN).

  **Which parts are requirements, and which is a default.** Parts 1 and 2
  are security requirements, and neither will ever be a setting. Part 1 is
  what keeps a port forward from becoming an internet exposure. Part 2 is
  what keeps the overlay's ACL meaningful. Part 3 is a default. A routed
  private peer reaching a LAN bind still has to get through mTLS, so
  refusing it shrinks who can reach the TLS handshake but protects no asset
  by itself. It could become a setting: a console-only list of extra
  private CIDRs, each passing the classifier, and audited when it changes.
  Phase 2 ships without that setting, because no network in scope needs
  it, and a setting is one more thing to get wrong (decided by `(main)` on
  the user's behalf, §11.1).
- **Its own `Shared` and its own rule set,** under the same dispatch table
  and the same `Ctx`. Decision 6 says two listeners, two rule sets. A flag
  on the loopback listener would let one of them leak into the other.

**What counts as private.** A closed allowlist, the user's (§11 Q6):

| Class | IPv4 | IPv6 | Typical use |
|---|---|---|---|
| Private-use (RFC 1918) | `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16` | — | A home or office LAN |
| Shared address space, CGNAT (RFC 6598) | `100.64.0.0/10` | — | Tailscale's node addresses |
| Unique local (RFC 4193) | — | `fc00::/7` | Tailscale's `fd7a:115c:a1e0::/48`; a LAN with ULA |
| Link-local (RFC 3927, RFC 4291) | `169.254.0.0/16` | `fe80::/10` | A cable between two machines, such as a Thunderbolt Bridge |

Anything else is refused, at parse time and again at bind, with its own
error (`not-private`), never a bind error, so a refusal cannot be mistaken
for an address nothing could bind. "Anything else" is every global unicast
address, and also these, which are not public but are not ours to bind:

- loopback (`127.0.0.0/8`, `::1`), which would merge the two rule sets
  decision 6 kept apart;
- the wildcard (`0.0.0.0`, `::`), multicast and broadcast;
- IPv4-mapped IPv6 (`::ffff:a.b.c.d`). Bind the IPv4 address instead. A
  mapped address is refused rather than unwrapped, so each address has one
  spelling;
- the documentation, benchmarking and reserved ranges (`192.0.2.0/24`,
  `198.51.100.0/24`, `203.0.113.0/24`, `198.18.0.0/15`, `0.0.0.0/8`,
  `240.0.0.0/4`, `2001:db8::/32`), NAT64 (`64:ff9b::/96`), 6to4
  (`2002::/16`) and the deprecated site-local `fec0::/10`.

One rule sits on top of the classes, at bind only: an IPv6 address that its
interface marks temporary or deprecated is refused even inside an allowed
class. SLAAC privacy addresses rotate within a day, so the dialog offers the
interface's stable address instead.

The classifier is one pure function in core, `worktrees_core::netclass`,
written as the allowlist above and nothing else, so a range nobody thought
of is refused rather than admitted. It decides on the address's canonical
bytes, never its spelling. The bytes come from Rust's `std::net` parsers,
which are strict: no `127.1` shorthand, no hex or octal octets, and no
leading zeros (std accepted leading zeros until CVE-2021-29922 was fixed in
Rust 1.53). They never come from `inet_aton` or a resolver, both of which
accept all of those spellings. In serve's own `--addr`, a zone (`%en0`) is
split off before parsing, must name an existing interface and must equal
`--iface`; a QR or an address code carries no zone at all (below). A string the
parser does not take is refused, never repaired. Serve calls it at parse
and at bind. The shim,
which links core (`app/src-tauri/Cargo.toml:24`), calls the same function
when it parses a pairing QR (§2.2) or an address code (below), so the host
and the device cannot disagree about what is private.

**Link-local needs its interface.** An `fe80::` address means nothing
without a zone, which is why `serve-remote.json` names the interface for
every bind and refuses a link-local IPv6 bind without one. A QR cannot carry
the device's zone, only the host's, and the host's means nothing on the
device. The QR and address-code parsers refuse any `%` in the address:
Rust's `SocketAddr` parser accepts a numeric zone (`[fe80::1%1]:7000`;
measured, spike, 2026-10-21, rustc 1.98.1), which would otherwise slip the
host's interface number into the device's connect. They split the port off
and parse the host with `IpAddr`, never `SocketAddr` or a resolver. A
device dialling a link-local address, IPv6 or IPv4, chooses its own
interface and scopes the socket to it, trying at most four, and adopts the
address only after a pinned handshake. It skips interfaces that hold a
link-local address but carry no service, such as macOS's `awdl0`, `llw0`
and the system `utun`s. IPv4 link-local needs this as much as IPv6 does:
with two links that both carry `169.254/16`, an unscoped dial follows the
routing table, which on a Mac is the primary interface (measured: on the
spike's Mac, `169.254/16` routes to Wi-Fi; on Linux 6.12, two links between
two containers took the wrong one). Whether iOS can dial either is the
iPhone spike (§10). If it cannot, the QR parser refuses link-local
addresses with a message saying so.

**CGNAT is shared space, and the dialog says so.** `100.64.0.0/10` counts
as private (§11 Q6), and Tailscale uses it for exactly that reason. It is
also what some carriers, phone hotspots and satellite ISPs hand out, and
there the "LAN" includes the carrier's other customers. None of them gets
past TLS without an enrolled key, but the TLS port answers them. So when a `100.64.0.0/10`
address sits on an interface that is not point-to-point (Tailscale's `utun`
on macOS and `tailscale0` on Linux are), the enable dialog warns and names
the interface. It is allowed with that warning (decided, §11.1 N1), and
such a bind is not advertised over mDNS (§4.7). The flag was measured
(spike, 2026-10-21) for macOS's `utun` and for a Linux TUN, which is what
`tailscale0` is. A TAP-based overlay, such as ZeroTier's, is not
point-to-point and is treated as a LAN, advert included.

**When the address changes.** At enable time `serve-remote.json` records the
address, the interface and a **network fingerprint**. For a point-to-point
interface the fingerprint is the address, its prefix length and the
point-to-point flag. The overlay assigns the address, and it is this node's
identity there. The interface's NAME is left out on purpose: macOS numbers
`utun` interfaces in the order they are created, so Tailscale's can be
`utun3` before a reboot and `utun5` after (*expected, not measured; §10*).
On the spike's Mac, Tailscale is `utun6`, behind six `utun`s the system
created first. A fingerprint that held the name would pause remote access after ordinary
reboots, and a pause dialog that fires for no reason teaches the user to
accept the one that matters, the same habituation §2.5 avoids for expiry.
For any other interface the fingerprint is the interface, the subnet (the
network prefix and its length, not the host's own address within it) and
the link-layer address of that interface's OWN router: the router of its
interface-scoped default route, whether or not a VPN or exit node holds the
primary one. On Linux both come from netlink (`RTM_GETROUTE` in every
table, `RTM_GETNEIGH`), readable without privilege (measured, spike,
2026-10-21, Linux 6.12, as uid 65534). On macOS 27 the kernel hides the
ARP table from non-root processes (measured: no entry for the gateway, even
right after a ping), so serve reads SystemConfiguration's
`State:/Network/Service/<service>/IPv4` (`Router`,
`ARPResolvedHardwareAddress`, and the OS's own `NetworkSignature`; measured
readable without root). That is one new crate for serve,
`system-configuration`, which is not in `Cargo.lock` today
(`core-foundation` is), checked on OSV as §4.2 requires, or a shell-out to
`scutil`, as with git and tmux. A missing neighbour entry is "not known
yet" and never counts as a change (on Linux it is missing before the first
packet to the gateway and after a flush); only a present, different
address raises the gateway dialog. IPv6 LAN binds use the interface and
the /64 (plus the router's address where the OS exposes it). The host poll (3 s) compares
what the interface holds now with what was recorded:

- **The address leaves its interface** (a DHCP lease moves, Wi-Fi drops,
  the overlay goes down, the machine sleeps). Serve closes the remote
  listener and any pairing window, closes every remote connection, writes
  `remote_paused` with its reason to the audit log, and shows *"Remote
  access paused: 192.168.1.5 is no longer on en0"*. It does not wait for the
  OS to fail the sockets. Nothing in the kernel does this for serve: on
  Linux 6.12 (measured, spike, 2026-10-21) the listener and its connections
  survive the removal, data waits, a re-added address is served again by
  the old listener, and an address moved to another interface keeps being
  served there. So the poll checks the address is on the bound interface
  (and its index), and resume binds a new socket after the old one is
  closed, because the old one still holds the port. macOS is unmeasured
  (§10).
- **The same address returns with the same fingerprint** (on a LAN, that
  includes the same interface). Serve binds again and writes
  `remote_resumed`, with no dialog. This is the overlay reconnecting, or
  the laptop rejoining its own Wi-Fi.
- **A new address in the same subnet, with the same fingerprint** (decided,
  §11.2 N4). This is what a moved DHCP lease looks like on a LAN: same
  interface, same subnet, same gateway, and only the host's own address
  changed. Serve binds the new address by itself, re-running the
  classifier on it, and writes `bind moved` with both addresses. Devices
  follow over mDNS (§4.7) and the pinned handshake, or with the address
  code. It applies to LAN binds only, since a point-to-point fingerprint
  includes the address. Three limits go with it (§12.3):
  - at most three automatic moves in an hour. A fourth pauses and asks,
    with an alert (§5.3), so a rogue DHCP server cannot make the listener
    flap, dropping every remote connection and writing an audit record
    each time;
  - a move closes any open pairing window, whose QR names the old address,
    and moves the mDNS advert (withdraw, then register);
  - the subnet recorded at enable must lie wholly inside one row of the
    allowlist above. A wide netmask, such as a `10.0.0.5/7` that also
    covers `11.0.0.0/8`, is refused at enable, so "the same subnet" can
    never include a public address.
- **Anything else** (an address in another subnet, another interface, or
  any address on a network whose fingerprint differs, such as a café that
  also hands out `192.168.1.x`). Serve stays paused and asks: *"Remote access is
  paused. This Mac is now 192.168.4.20 on en0, on a different network. [Use
  this address] [Keep it off]"*. Using the new address is `remote_enable`
  again, which re-runs the classifier and records a new fingerprint. When
  the ONLY thing that changed is the gateway's link-layer address (same
  interface, same subnet), the dialog says so in its own words and the
  host raises an alert (§5.3): *"The router's hardware address changed.
  This can mean a different network, or someone on this one pretending to
  be the router."* That is what ARP spoofing of the gateway looks like, and
  a generic "different network" would invite the click that records the
  impersonator as the new fingerprint.
- **An address missing at startup** is the paused state. The loopback
  listener never waits for it, and serve logs the state once per episode.

The fingerprint is there to stop an accidental exposure, not an attacker.
Someone on the café network can copy a gateway's link-layer address, and
against them mTLS is the gate.

**A device follows a moved address without pairing again.** The pin does
not change when the bind moves. So the host shows an **address code**, a QR
of `worktrees-addr:v1?addr=<address>:<port>&pin=<spki sha256>` with no
secret in it. A device accepts it only for a host whose pin it already
holds, and only for an address the classifier allows. The code itself is
NOT authenticated: the pin is public, so anyone can print one. So the device
keeps its old address and adopts the new one only after a pinned handshake
to it succeeds. A forged code then costs one failed attempt, which the
device reports as the wrong host, not a lost one. For LAN mode, the enable
dialog suggests a DHCP reservation on the router, which avoids the step.
On a LAN, a device also finds a moved host by itself over mDNS (§4.7,
decided in §11.1 N2), and the address code is the fallback when that
fails.

### 4.2 TLS: options

| Option | iOS | Pinning | Ops cost | Verdict |
|---|---|---|---|---|
| No TLS on the overlay (WireGuard encrypts) | Easy | None | None | **Rejected.** Every node on the tailnet reaches the port, and a mistaken ACL is a breach. §III.3 and R4 both refuse to put the whole burden on the overlay. |
| A public CA certificate (`tailscale cert`, `*.ts.net`) | Stock trust; works from a WebView | Hostname-based, not identity | Short-lived certificates, renewed through ACME; depends on Tailscale | Rejected for Phase 2: it authenticates a NAME, and anyone with a valid certificate for that name is trusted. |
| A private CA, with a profile installed on iOS | A profile install for every device | CA-based | §III.3 already calls it a support burden | Rejected |
| **Self-signed, pinned both ways (SPKI), TLS 1.3 only, mutual** | Works from Rust; not from a WebView (§4.4) | Exact key | None after pairing | **Chosen** |

**The configuration.** rustls with TLS 1.3 only and its default cipher
suites. rustls 0.23 (≥0.23.46), ring 0.17 and tokio-rustls 0.26 are already
in the tree, through the app's updater (`tauri-plugin-updater` → `reqwest`),
so the CLI gains a dependency edge rather than new crates. The floor is
required. cargo-deny reported four advisories against the rustls that
`Cargo.lock` held on 2026-10-21 (0.23.42: RUSTSEC-2026-0285, -0346, -0348
and -0349, all fixed by 0.23.46) and one against its rustls-webpki (0.103.13:
RUSTSEC-2026-0382, fixed in 0.103.16). The tree is at the floor since #531,
which moved `Cargo.lock` to rustls 0.23.46 and rustls-webpki 0.103.16. This document
names floors, not locked versions: the slice that adds TLS takes whatever the
tree holds then, as long as it meets them. Certificate generation adds
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
  pin is stronger than a name. So the client sends no SNI either (§4.7).

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
- **Plugins act on the client's machine, and a host path means nothing
  there.** The page imports the opener, dialog, process and updater plugins
  and the window API directly, not through `ipc.ts`, because they act on the
  machine the window runs on (AGENTS.md). On the host, that machine is also
  the engine's. On a remote client it is not:
  - `revealItemInDir` and `openPath` take a HOST path. `App.tsx`,
    `DocsPane`, `FilesPane`, `ProfilesPanel`, `SettingsSheet` and
    `TerminalPane` all call them. On a client the path does not exist, or
    names a different file.
  - The dialog's `open` (`App.tsx`, `ProfilesPanel`) picks a CLIENT path and
    hands it to a host command.

  So a remote client greys every path-taking plugin call, with a reason, as
  it does for `ClientLocal` rows (§III.4.B: never silently do nothing).
  `openUrl` keeps working for URLs that mean the same everywhere, such as a
  pull request or the release notes. The docs viewer's loopback URL is gap
  7's. The updater updates the client app, which is right, and `update_cli`
  stays console-only (§3.4). A browser client, such as the dev stack's page,
  has none of these plugins, and has to grey the same set. So one predicate
  beside the door table (`ipc.ts`) answers "does this window run on the
  host", and both kinds of client use it.
- **App Transport Security** governs the URL Loading System and WebView
  loads. A socket opened from Rust is outside it. *Verify on a device in
  §10's spike.*
- **Local Network privacy** (iOS 14 and later) prompts for LAN addresses and
  needs `NSLocalNetworkUsageDescription`. Whether a Tailscale `100.x`
  address through the VPN interface prompts too is unmeasured. Overlay mode
  may need no prompt. LAN mode will, and it ships in Phase 2 (§11 Q6), so
  the usage string ships with it, and so does the path for a user who
  declined the prompt: a message pointing at Settings → Privacy → Local
  Network, not a silent connect failure.
- **Bonjour** (§4.7, in Phase 2 by §11.1 N2). Browsing needs
  `NSBonjourServices` in Info.plist listing `_worktrees._tcp`; a type left
  off the list fails to browse. Browsing also raises the same Local Network
  prompt, so a user who declines it loses discovery and LAN mode together,
  and the message above covers both. Bonjour through the system's own APIs
  does NOT need the restricted multicast entitlement
  (`com.apple.developer.networking.multicast`), which only raw multicast
  sockets need. That is one more reason the client never implements mDNS
  itself. Whether to browse from Rust through the `dns_sd` C API, which iOS
  ships, or through a small Swift plugin around `NWBrowser` is for §10's
  spike.
- **Backgrounding** suspends the socket. The client reconnects in the
  foreground with a full handshake, because resumption is off (§4.2). R6 has
  to expect reconnect storms (§6). The policy for WHEN to try exists since
  #525 (`app/src/reconnect.ts`: jittered exponential backoff capped at 30 s,
  no try while the window is hidden, one try in flight). The remote door
  reuses it with one rule taken out: a remote device never starts a serve.

### 4.5 The two rule sets, side by side

| | Loopback (decision 6, as built) | Remote (Phase 2) |
|---|---|---|
| Bind | `127.0.0.1` only (`conn.rs:306`) | One private address on one named interface, checked again on every poll (§4.1) |
| Peer filter | `is_loopback()` (`conn.rs:342`) | Private always; on an overlay bind, in the overlay's own row; on a LAN bind, on-link (§4.1) |
| Transport | Plaintext | TLS 1.3, mutual, pinned both ways |
| Before HTTP | Connection caps, then hyper | Connection caps, then the rustls handshake. An unknown device ends here. |
| `Host` | Loopback on our port (`policy.rs:33`) | The configured address or name, on our port |
| `Origin` | On the allowlist; missing is refused (`policy.rs:52`–`55`) | **Absent, or the native client's own**: no browser origin is accepted. No CORS header, ever. |
| Who | Whoever holds serve.json's token: the owner, on the console | One enrolled device, neither revoked nor disabled, and its principal |
| Credential | Per launch, handshake v1 | Per device, persistent, revocable |
| Refusal | Zero bytes written (D6) | During TLS: a TLS alert (rustls), which is unavoidable. A refused upgrade (`Host`, `Origin`, path): the socket closes with nothing written. After the upgrade: `err` frames. |
| Exposure classes served | Engine, HostAdmin, Stream, HostConsole | Engine, HostAdmin and Stream, as the device's capabilities allow; never HostConsole or ClientLocal |
| Scope | `"*"` | The (principal, device) pair's (§2.8) |
| Limits | M1 (`conn.rs:61`–`84`) | M1, plus per device (§6) |
| Audit | Mutations, refusals | Everything in §5, attributed to the device |

**The remote handshake.** After mTLS and the WebSocket upgrade, the server's
first frame is
`{ok: true, version, device: {id, principal, name, caps, projects, expiry, stepup_required}}`.
The client has already authenticated the server through the pin, so it may
send right away. It checks `version` as D5 does, and shows its own
capabilities, scope and expiry.

### 4.6 The CI witness: identity, not just reachability (R8)

**First, a debt from Phase 1, now paid in CI.** Decision 6 says "the CI
witness is a CONNECT with the served case first". At `fadb2ce` serve had no
such test, and the only off-loopback connect witness in the repo was the docs
server's (`crates/worktrees-api/src/docserver.rs:1554`–`1617`). #516 as
merged added one in that shape:
`crates/worktrees-cli/tests/serve_bin.rs::serve_can_be_reached_on_loopback_and_not_off_it`.
It makes a loopback connect that must succeed, then a connect to
`own_lan_addr()` on the same port that must fail. On a runner with no LAN
address it fails instead of skipping. It runs in CI on both OSes (gap 11). It
is not yet in the release gate, which slice 0 adds (below).

**The bind-rule witness, as connects** (§11 Q6). The rule the bind rules
guard is "never reachable from a public address, and never on loopback".
AGENTS.md's lesson is that no bind result means the same thing on both
platforms, and that a refusal proves nothing unless the served case before
it succeeded. So the classifier gets a table test, and every claim about
the listener is a CONNECT that follows one that must succeed:

1. **The classifier, as a table** (pure, no sockets). The first and last
   address of each allowed range, and the neighbour just outside it:
   `9.255.255.255`, `10.0.0.0`, `10.255.255.255`, `11.0.0.0`,
   `172.15.255.255`, `172.16.0.0`, `172.31.255.255`, `172.32.0.0`, and the
   same for `192.168.0.0/16`, `100.64.0.0/10`, `169.254.0.0/16`,
   `fc00::/7` and `fe80::/10`. Then every refused kind in §4.1, mapped
   addresses included, and the spellings the parser must refuse: `127.1`,
   `0x7f.0.0.1`, `010.0.0.1`, and a zone naming no interface. The shim's QR
   parser runs the same table, so serve and the device cannot drift.
2. **A public address that IS bindable is refused.** Give the runner a
   documentation address, `198.51.100.7`, on a scratch interface: a dummy
   interface in a Linux container with `NET_ADMIN`, and an alias under
   `sudo` on macOS. First a plain test listener binds it and a connect to it
   succeeds, which proves the address is live. The test listener closes,
   and serve is asked to bind the same address and port. It must refuse
   with `not-private`, not a bind error, and a connect to that port must
   fail. Without the first half, an address that nothing could bind would
   pass.
3. **The remote listener is not on loopback.** With the remote listener up
   on `own_lan_addr()` and a connect there succeeding, a connect to
   `127.0.0.1` on the same port must fail. A wildcard bind would answer
   both, on both platforms.
4. **The address moves.** With a private address on the scratch interface
   and the identity witness's enrolled device (step 2 below) connected to
   it: remove the address. Within two polls the device's socket closes, a
   connect to the old address fails, and the audit log has
   `remote_paused`. Put the same address back: a connect succeeds again,
   and the log has `remote_resumed`. Give the interface a NEW address in
   the same subnet instead: the listener moves by itself, a connect to the
   new address succeeds, one to the old address fails, and the log has
   `bind moved` with both. Three more moves inside the hour: the fourth
   pauses instead, with an alert. Give it an address in a different
   subnet: a connect to it fails until `remote_enable` names it.
5. **The peer rule holds** (§4.1, parts 2 and 3). The client binds its
   socket to a chosen source address on the scratch interface before it
   connects. With the remote listener on a `100.64.0.0/10` address on a
   point-to-point scratch interface (a TUN device on Linux, as Tailscale's
   is; a plain dummy interface is not point-to-point and would test the LAN
   rule instead), a connect from a second address in that row succeeds,
   then one from an RFC 1918 address is dropped with nothing written. Then
   the same pair for a LAN bind: an on-link source is served, then a
   private source outside the subnet is dropped. When the listener is
   device-bound (§4.1), the out-of-row client address must sit on the SAME
   point-to-point scratch interface; one on another interface is refused by
   the kernel before serve sees it, and the assertion would pass with the
   peer filter deleted (measured, spike, 2026-10-21, Linux 6.12). So this
   step is shown failing with the peer filter removed.

*Unmeasured: whether the macOS runner allows a scratch interface or alias
under `sudo`. If it does not, steps 2, 4 and 5 run on Linux only, the test's
name says so, and the macOS half moves to §10's spike.* The Linux half works
as written in a `NET_ADMIN` container on 6.12 (spike, 2026-10-21: a dummy
interface with `198.51.100.7`, and a TUN with a `100.64.x` peer). It has
not yet run on a GitHub runner (§10).

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
11. Expiry: a second enrolled device whose `until` has passed is refused at
    TLS, and its record says `disabled: until`. Re-enabled from the console,
    it is served. Its audit records name its principal.
12. A reduction reaches live traffic (§3.3): with that device holding an
    open terminal attach and sending nothing, disable it from the console.
    Its socket closes within one second. Re-enable it, attach again, and
    narrow its scope to exclude that place: the attach's channel ends with
    `end: true`, and the socket stays open.

**Two platforms.** AGENTS.md's bind lesson applies: a refusal measured on
one OS is a claim about one OS. Each assertion is measured on macOS and
Linux before it is written down, with `docker run --rm -v "$PWD":/w
node:22-alpine` or the Linux runner for the Linux half.

**The release gate.** The app-bundle job already runs the docs server's
boundary tests by exact name and greps `test result: ok. 4 passed`
(`.github/workflows/release.yml:134`–`146`). Add a second step of the same
shape for the serve witness, with its own exact count, because a renamed
test turns `--exact` into a filter that matches nothing and exits 0. Slice 0
adds it for the loopback witness that is already in the repo. Each later
witness joins it, with the count raised to match.

### 4.7 Rediscovery on the LAN: mDNS (decided, §11.1 N2)

The address code (§4.1) needs someone to scan it. In LAN mode, a device can
also find a host whose address moved by itself, through DNS-SD over
multicast DNS (RFC 6762, RFC 6763), so a phone on the same Wi-Fi reconnects
without help. The address code stays, as the fallback.

**When, and where.**

- Only while LAN mode is on: the remote listener is bound and not paused,
  on an interface that is not point-to-point. An overlay bind is never
  advertised, and mDNS would not cross the tunnel anyway. A TAP-based
  overlay (ZeroTier's shape) is not point-to-point, so it is a LAN here
  and is advertised to that whole virtual network (§4.1).
- Never for a `100.64.0.0/10` bind on an interface that is not
  point-to-point (§11.1 N1). That link may include strangers, and the
  enable dialog already warns about it.
- Only on the bound interface, registered by its interface index.
- Withdrawn the moment the listener closes: a pause (§4.1), the kill
  switch, or remote access turned off. Withdrawing means freeing the
  registration, not only closing the socket: Avahi re-announces a live
  registration by itself when its address returns (measured, spike,
  2026-10-21, Avahi on Linux 6.12). The goodbye (TTL 0) goes out when the
  interface still has an address; when the pause is BECAUSE the address
  left, there is nothing to send it from, and peers may list the host until
  the records expire (Avahi's TTLs: SRV 120 s, PTR and TXT 75 min).
  Adoption needs the pinned handshake, so a stale listing costs one failed
  try. A crashed serve is withdrawn by the responder (measured on both: on
  Linux on the wire, 2 ms after a SIGKILL; on macOS as the local browser
  sees it, 1.2 s after one).
- Visible on the host. While the advert is up, the indicator that remote
  access is on (§4.1) also says that this Mac can be found on this
  network. A console switch turns the advert off and leaves LAN mode on. It
  starts on, as §11.1 N2 decided, and turning it off only narrows.

**What it says.**

- The service type `_worktrees._tcp`.
- The host's **discovery id** (§2.1): 128 random bits in base32, as both
  the instance name and the TXT key `id`. It is meaningless to anyone
  without a record that holds it, and nothing public derives it: not the
  pin, not the host name, not the key.
- One other TXT key, `v=1`.
- An SRV record pointing at the remote listener's port and at a host name
  of serve's own, `wt-<discovery id>.local`, whose one address record is
  the bound address (*recommended, unmeasured*; "The host name", below).
  Where that cannot be done, the SRV target is the host name the OS
  already advertises (`<LocalHostName>.local`), and serve adds no host
  record.
- The remote listener only. The pairing listener (§2.4) is never
  advertised, not even while a window is open. One bad attempt closes a
  window, so a window that announced itself could be burned by anyone on
  the link the moment it opened.

Serve composes every record from constants and the discovery id, with no
client string in any of them, so nothing can be injected into the advert.

Nothing else that serve composes: no project or place names, and no pin.

**The host name: serve's own, not the OS's** (*recommended, unmeasured*;
§10). The spike found that when serve names no host, the SRV target is the
OS's own `.local` name, which on a default macOS install is
`<First name>s-MacBook-Pro` (on the spike's Mac, the owner's first name),
and that a resolve of it returns every address on the interface: IPv4, ULA
and link-local (spike, 2026-10-21, macOS 27 and Avahi on Linux 6.12;
§12.4 T1). That contradicts what decision 7 and §11.1 N2 promised, an
advert that reveals nothing but an opaque id. So serve registers a host
record of its own and makes it the SRV target:

- The name is `wt-<discovery id>.local`, using the id the advert already
  carries, so it is exactly as linkable as the TXT record and reveals
  nothing more. The label is 29 characters, inside DNS's 63.
- It has one address record: the bound address (an A or an AAAA record),
  on the bound interface only. A resolve then returns that address and no
  other, which is also the only one a device should dial.
- **mDNSResponder:** one `DNSServiceCreateConnection`; on it, a
  `DNSServiceRegisterRecord` for the address record
  (`kDNSServiceFlagsUnique`, the bound interface's index), then
  `DNSServiceRegister` on the same interface with `host` set to that name.
  `dns_sd.h` says a host named there gets no address record by itself,
  which is why the record is registered first. Deallocating the connection
  removes both.
- **Avahi:** in the one entry group, `AddAddress` for the name and the
  bound address (with `AVAHI_PUBLISH_NO_REVERSE`, so it adds no
  reverse-lookup record beside the one Avahi publishes for its own host
  name), then `AddService` with its `host` argument set to that name.
  Resetting or freeing the group removes both.
- A conflict on the name, meaning another host on the link already claims
  `wt-<id>.local`, is not resolved by renaming. Serve does not advertise
  for that episode, writes the conflict to the audit log and says so
  beside the indicator, and the address code still works. A squatter that
  answers for the name with its own address gains what a spoofed advert
  gains: one failed pinned handshake.
- Every record serve registered is withdrawn together, as "When, and where"
  says, and a bind move (§4.1, §11.2 N4) withdraws the host record and
  registers it again with the new address.

What it does not do: the OS's responder still answers for the host's own
name, with all its addresses, whether or not serve runs. An observer who
asks for that name can still match one of its addresses to the advert's.
The opaque host removes the advert's pointer to the name, not the name.

*Fallback, per platform.* If the opaque host cannot be done on a platform,
that platform advertises with the OS's own host name, and its enable
dialog shows the name that will be advertised (`scutil --get
LocalHostName` on macOS, Avahi's host name on Linux), so the user can
rename the machine before turning discovery on. There, nothing else that
serve composes appears, but the SRV target is the OS's own `.local` name,
which on a default macOS install is `<First name>s-MacBook-Pro`, and a
resolve returns every address on the interface. So on that platform the
advert does point at a user name, the OS's.

**How serve advertises.** Through the OS's own responder, never one of its
own: mDNSResponder (`DNSServiceRegister`) on macOS and Avahi over D-Bus on
Linux, each given the bound interface's index. So serve parses no multicast
traffic, and no new UDP listener runs in the engine's process: the one that
answers is the responder the OS already runs. Where there is none, such as
a Linux host without Avahi, discovery is off, the enable dialog says so,
and the address code is the only path. How serve calls each responder, and
whether either needs a new crate, is decided when it is built and checked
on OSV, as §4.2 requires. One is known already: zbus 5.18 is in
`Cargo.lock` only under the app (through `tauri-plugin-opener`), so
calling Avahi over D-Bus from serve adds it to the CLI.

**How a device uses it.** A device gets the discovery id in the pairing
answer and keeps it with the host's record (§2.1). It browses
`_worktrees._tcp` only when it needs to: when its stored address stops
answering, for at most 30 seconds, with a back-off between rounds. It never
browses in the background, because a phone that keeps asking for
`_worktrees._tcp` announces, on every network it joins, that it is a
worktrees client. Each advert is untrusted input, and a candidate is tried
only if all of these hold:

- Its TXT record is exactly `v=1` plus an `id` of 26 base32 characters,
  and that `id` equals the host's discovery id. The device matches on the
  TXT key and ignores the instance name. mDNS renames an instance whose
  name another host claimed first, so a squatter could otherwise push the
  real advert out of sight and put its own in that place.
- Its address passes the classifier (§4.1) and is on-link for the
  interface the advert arrived on: inside that interface's own subnet, or
  link-local. Without this, a spoofed advert could point the phone at any
  address the phone can reach, a VPN's networks included, and use it as a
  probe.
- It is one of the first four distinct candidates this round. A candidate
  that fails is not tried again in the same round.

A discovered address is adopted only after a pinned handshake to it
succeeds, the same rule S8 set for the address code, and the old address is
kept until then. Discovery chooses where to try, never whom to believe.

**A spoofer learns nothing about the device that tries it.** In TLS 1.3
the server proves its key before the client sends any certificate, and
rustls checks the pin as soon as the server's certificate arrives (§4.3).
So a wrong pin ends the handshake before the device's certificate is sent,
and that certificate would be encrypted in any case. The client sends no
SNI (rustls's `enable_sni` is off, since the pin needs no name), so the
ClientHello does not carry the host's name either. A test checks both
against a server with the wrong key.

**What it costs.**

- In serve: registering and withdrawing an advert and its host record on
  two platforms, tied to the listener's state, plus two audit kinds (§5.1).
- In the clients: a browse in the shim for iOS and iPadOS (§4.4) and for
  the desktop client, the `NSBonjourServices` entry, and the
  `discovery_id` in the host's record.
- A second Local Network moment on iOS, and on macOS 15 and later (§10's
  spike).
- One more thing the user can see on the network, which the next item is
  about.

**What it exposes.**

- **LAN reconnaissance.** Anyone on the link learns that this host runs
  worktrees with remote access on, and on which port. A port scan finds the
  port anyway; the advert removes the effort and labels it. It adds no
  capability, because the port is mTLS and an unenrolled peer stops inside
  rustls (§4.3). That is the cost the user accepted in N2, and it is why
  the advert exists only in LAN mode and on one interface.
- **Spoofed adverts.** Anyone on the link can advertise any instance,
  including one carrying a copied discovery id and their own address. The
  device tries it, the pin does not match, and nothing is adopted.
- **Replayed adverts.** An old advert, or a cached record of the host's
  previous address, sends the device to an address that is either still
  the host (fine) or no longer is (refused at the pin).
- **Enumeration.** The type is listed under `_services._dns-sd._udp`, so a
  peer learns that worktrees is here without asking for it by name
  (measured, spike, 2026-10-21, on Avahi's wire).
- **Linkability.** The discovery id is stable, so an observer on two
  networks where LAN mode was on can tell it is the same machine. With the
  opaque host name the SRV target carries the same id, and nothing more.
  The host's own `.local` name, which its responder answers for on every
  network whether or not serve runs, is just as linkable, so the id adds
  nothing the OS does not already show.
- **Sleep.** A Mac with "Wake for network access" hands its adverts to a
  Bonjour Sleep Proxy when it sleeps, so the advert can outlive the awake
  listener, and a connect may wake the Mac (unmeasured, §10).
- **Denial of service.** Anyone on the link can flood candidates, or
  spoof ARP, and keep a device from finding its host. Nothing at this
  layer prevents that, and the address code is the way back.

**On the devices.**

- **iOS and iPadOS:** `NSBonjourServices` and the Local Network
  permission, as §4.4 says. No multicast entitlement.
- **A macOS client** browses through `dns_sd`. macOS 15 and later has a
  Local Network permission of its own, and how it treats an ad-hoc-signed
  client after an update is part of §10's spike.
- **A Linux client** browses through Avahi when it is present, and uses the
  address code when it is not.
- **Android** is not a Phase 2 client (decision 3). If one is ever built,
  its DNS-SD is `NsdManager`, the same rules apply, and its permission and
  multicast requirements, which have changed across Android releases, are
  checked at that time.

**The witness.** In CI, against a real serve in LAN mode on the scratch
interface (§4.6), with the OS's browser (`dns-sd -B` on macOS,
`avahi-browse` on Linux):

1. With LAN mode on, the advert is present on the bound interface and on
   no other, and its TXT record is `v` and `id` exactly. Its SRV target is
   `wt-<id>.local`, and a resolve of it returns the bound address and no
   other (where the platform takes the opaque host). With a pairing
   window open, no advert names the pairing port.
2. Pause the listener (§4.6, step 4): the advert is withdrawn, the host
   record with it, and when the address returns the advert does not come
   back until serve binds again. Turn remote access off: still withdrawn.
   An overlay bind, a `100.64.0.0/10` bind on
   an interface that is not point-to-point, and the console switch turned
   off: never advertised.
3. In the client, as unit tests of the candidate logic:
   - a discovered address whose pinned handshake fails is not adopted, and
     the stored address is unchanged;
   - a candidate that is off-link, or that has any other TXT shape, is
     never dialled;
   - at most four candidates are tried in a round;
   - against a server with the wrong key, the device sends no certificate
     and no SNI.

---

## 5. The audit log (R5)

### 5.1 The log

**The file.** `<config_dir>/audit.jsonl`. Serve opens it with `O_APPEND`,
`O_NOFOLLOW` and mode `0600` at creation. It refuses to start the remote
listener if the file exists with another owner or a group or world bit,
rather than chmod it silently. It rotates at 5 MiB and keeps five files. The
rotation is itself a record. Rotation must not delete the evidence §5.2
checks devices against: a device paired long ago has its `enrolled` record
in a file that rotation removed. So each new file opens with a
`checkpoint` record, which holds the previous file's final hash and, for
every live device, the security fields of its record (principal, key pin,
capabilities, projects, expiry, step-up, disabled). The chain continues
across files, and §5.2 reads from the newest checkpoint onward, never a
file that rotation may already have removed.

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
 "principal": "owner", "device": "d_k3v9q2", "conn": 17, "peer": "100.101.7.3",
 "cmd": "write_file", "cap": "instruct", "project": "/Users/bret/Projects/x",
 "object": "AGENTS.md", "outcome": "ok", "ms": 4,
 "detail": {"bytes": 2310, "sha256": "…"}, "prev": "<sha256 of the previous line>"}
```

`principal` and `device` name the ACTOR (§2.8). Both fields are in every
record, so a reader can filter by principal without guessing which records
have one:

- a console client is the owner principal with `device: "loopback"`;
- before authentication there is no actor, so an authentication failure or
  a pairing attempt has `principal: null` and `device: null`, and `peer`
  identifies it. A pairing attempt also carries the principal its window
  was opened for, in `detail.for_principal`;
- a record about another device (a revoke, a narrowed scope, a changed
  expiry, a re-enable) names that device in
  `detail.subject: {principal, device}`.

The `prev` hash chain makes a
partial edit detectable. It cannot stop a same-user process from rewriting
the whole file, and nothing here claims that it can (§1).

**What it records:**

- lifecycle: the listener started or stopped, with version, address,
  interface and identity pin; remote access enabled or disabled; paused or
  resumed by an address change, with the reason; `bind moved` to a new
  address, with both addresses and whether the console chose it or serve
  moved it within the same subnet (§4.1); the mDNS advert registered or
  withdrawn, with its interface, or not registered because its host name
  was claimed (§4.7);
- pairing: a window opened, an attempt with its result and reason, Allow or
  Deny, the window closed;
- devices: enrolled (with principal, scope and expiry), scope changed,
  expiry changed, step-up turned off or on, disabled (with its reason:
  `idle`, `until`, `unrecorded` or `console`), re-enabled, revoked (by
  whom), identity rotated;
- authentication failures: an unknown certificate, a revoked certificate, a
  failed handshake v1. These are aggregated per episode with counts, like
  `conn.rs`'s `capped` log, so a flood does not turn into a disk-writing
  flood;
- every refusal: an exposure, a capability or scope, a limit tripped. A
  `term_close` of an id the connection did not open is recorded here too,
  although the caller is answered `ok` (§9.1);
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
cross-checks every device against the audit chain, from the newest
checkpoint onward (§5.1). It matches the records that name the device as
`detail.subject`, never the actor fields: the console enrolled the device,
so the console is the actor. A device with no
`enrolled` record behind it is DISABLED (`reason: unrecorded`, §2.5) until
the console re-enables it, and the host raises an alert (§5.3). So is a
device whose principal is missing, or differs from the one its `enrolled`
record names, so that moving a device between principals by editing the
file is caught the same way. The same check compares the record's scope,
expiry and step-up with the last audit record that set them, and a record
that is wider than its chain says, or whose expiry is later, is disabled
too. A corrupt `devices.json` refuses every remote device and is never
overwritten, the registry's "refused rather than clobbered" rule; in Phase
2, a principal other than the owner counts as corruption (§2.8). The
console keeps its identity through all of it, because the loopback caller's
principal is never read from the file (§2.8), so the user at the host can
still repair it. A same-user writer can forge the chain too. This makes the
forgery a two-file, hash-chained job instead of one JSON edit, and it puts a
banner in front of the user either way.

### 5.3 Alerts

ASVS L2 asks for alerting, not only logging. A file nobody reads alerts
nobody, so these raise a host notification and a badge on Settings →
Devices:

- a device paired, or remote access turned on or off;
- remote access paused by an address change, and waiting for the host to
  choose (§4.1);
- automatic rebinds past their limit, and a gateway whose link-layer
  address changed on the same subnet (§4.1);
- authentication failures past a threshold in one episode;
- a device disabled by its expiry (§2.5);
- a revoked or disabled device trying to connect;
- a device seen from a new address class;
- a device record with no pairing record (§5.2);
- a limit tripped by one device repeatedly (§6).

### 5.4 L1's two remainders, closed on main by #516

Both were open at `fadb2ce` (gap 15), and the merged #516 closed both:

- `log_event` kept newlines and truncated at byte 4000, which panics in the
  middle of a UTF-8 character (`crates/worktrees-api/src/meta.rs:27`–`28` at
  `fadb2ce`). `meta::ui_line` now escapes the C0/C1 controls, the Unicode line
  and paragraph separators and the bidi controls, then truncates on a
  character boundary. Escaping rather than stripping keeps an attempt
  visible.
- serve.log was opened with no mode, so it got the umask's `0644`
  (`crates/worktrees-cli/src/serve/mod.rs:131` at `fadb2ce`). It is now
  created `0600` and re-tightened on every write, since an older binary may
  have created it. The rotated `serve.log.1` is re-tightened too, because a
  `rename` keeps the old mode.

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
| Enrolled devices | 16, disabled ones included; revoked tombstones are not | Pairing refused before the QR appears |
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

**What Phase 2 leaves alone on loopback.** A reconnecting local flood still
locks NEW loopback clients out: measured on macOS, 0 of 49 got in (AGENTS.md,
M1). Since M1 it cannot take memory, descriptors or a client already
connected. A same-user process is outside §1 anyway. Another local user is
not, and can still keep the user's own app from connecting: a denial of
service at the desk, which gets no frame past the handshake. Phase 2 does not
change this, and must not let it spread. The remote listener has its own
`Shared` and its own caps (§4.1). A remote flood therefore cannot fill
loopback's 16 unauthenticated slots, and a local flood cannot fill the remote
listener's. A per-source cap would not help on loopback, where every client
has the same source address.

**A panicking body must still answer.** At `fadb2ce` the job a request runs
pushed its reply only when the body RETURNED (`conn.rs:539`–`541`). A body
that panicked was contained by `spawn_blocking`, which is what the serve
review measured, but its request was never answered, so the client waited
forever. #516 fixed that half: `conn::answer` runs the body under
`catch_unwind` and answers `err: "internal error"` (`wire::INTERNAL_ERROR`),
and the log gets one line naming the command. The other half arrives with
the caps. Under per-connection caps a panicking body would hold an in-flight
slot forever, so the in-flight count must be a drop guard that comes back
however the job ends.

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
- **A declared-state stamp:**
  `last_remote_instruction: {principal, device, path, at}` on the place,
  written by the resolver whenever an instruction-class write arrives from
  a device. The UI shows *"AGENTS.md edited from Bret's iPhone, 2 min ago"*
  on the place, and MCP `place_status` reports the same fact.
- **The changed-files list.** A remote write shows up in `git diff` like any
  other, and that is the review surface that already exists.
- **Notes and titles are data in MCP.** Agents see titles in resource
  descriptions (`crates/worktrees-cli/src/mcp.rs:2524`), and the
  `reading_notes` at `mcp.rs:2625` marks branch, upstream, commit subject,
  agent names and plan strings as "data, never instructions". It does not
  name `title` or `note`. Add both, and record `note_by` / `title_by` as
  `{principal, device}`.

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
  lane records the principal and the device that launched it, and the audit
  log does too.

---

## 8. The known gaps

The brief's eight are first, then seven found while writing this, then one
that Appendix B already had. At `fadb2ce`, an authenticated loopback client
could reach each one. Gap 9 needed less: a cloned repository's content plus
the user's next ordinary save. Since then, #516 as merged closed gaps 11 and
15 and part of 3 and 5, and #523 closed gap 9. Those rows keep their
`fadb2ce` text, and the last column says what main has now.

| # | Gap | Where | Fix shape | Phase |
|---|---|---|---|---|
| 1 | `write_file` can overwrite `.git/config`; `core.fsmonitor` runs on the next git spawn, and the host poll spawns git every 3 s | `crates/worktrees-api/src/files.rs:1311`–`1333`. The guard is `guard_under_projects`, "under SOME root" (`projects.rs:45`). | The `forbidden` class (§7.1), for every caller | 2 |
| 2 | `save_profile` stores `mcp_servers` (commands) and `settings` (claude's `hooks` are a key of that raw JSON), both run at the next launch | `crates/worktrees-api/src/profiles.rs:21`–`25`; the fields at `crates/worktrees-core/src/profile.rs:231`, `:262` | Console-only (§3.4), together with `set_project_profile` and `set_default_profile`, which choose where a profile runs | 2 |
| 3 | `term_open` passes `session` to `attach-session -t` unanchored, so a name prefix-matches another session. `tune_session` is unanchored too. Any tmux session on the host can be attached, worktrees' or not. | `crates/worktrees-api/src/term.rs:65`, `:75`; `crates/worktrees-core/src/tmux.rs:530`–`533` | `session(…)` resolves the name to a place in scope, then passes `=<name>`. `ai_pane` is the precedent (`tmux.rs:583`–`587`). | 2. **The prefix match is closed on main by #516:** `term_open` refuses a session that does not exist and attaches `=<name>`, and `tune_session` targets `=<name>:`. Still open: any tmux session on the host can be attached by its exact name, worktrees' or not, until `session(…)` checks it against a place in scope. |
| 4 | `sync_apply` with no `repo` adopts: the rsync `--delete` destination is the hub manifest's `local_root`, checked only for being non-empty and not a hub copy. Deletions go to a backup dir, but the tree is mirrored over whatever path the manifest names. | `crates/worktrees-core/src/sync.rs:229`, `:1253`, `:1263`; the api passes the client's `confirmed` (`crates/worktrees-api/src/project.rs:138`) | Phase 2: the adopt form is console-only. Later, a destination rule: must not exist or must be empty, must sit under a configured parent, never `$HOME` or an ancestor of a registered root. | 2 (rule: 3) |
| 5 | Containment under `[docs]`: `resolve_rel` and `root.join(rel)` walk through an intermediate symlink. `create_initial_commit("")` runs git in serve's own cwd (AGENTS.md: `git -C ""` means HERE). `resolve_doc_paths.root` runs `git ls-files` in an unguarded directory, and an untrusted repo's `core.fsmonitor` runs there. A `..` slug in `shell_open`, `list_branches` and the rest. | `crates/worktrees-core/src/docs.rs:451`; `ROADMAP.md:1074`; `crates/worktrees-api/src/workspace.rs:558`–`563`; `files.rs:1212`–`1240`; `project.rs:500`–`505` | R1's resolver (`project`, `root`, the slug rule) for the last three. For `[docs]`, ROADMAP's own shape: lstat each component, and accept a link only if its target canonicalizes back under the place root, which is `docserver::safe_under`'s rule. **These must close before any remote bind** (§III.3a R1). | 2. **On main since #516,** `shell_open` refuses a `..` slug, and any slug that is not a place, before anything spawns. `list_branches` and the rest still hand the raw slug to `place_dir`. |
| 6 | `privacy_denial` (PR #519) is deliberately unguarded. For any path it tells the caller whether that path is refused under a macOS privacy area. | `origin/feat/tcc-recovery:app/src-tauri/src/lib.rs:5512`; `macprivacy.rs:197`–`222` | #519 is still open. Rebased onto main, where the app has been serve's client since #524, it becomes an api body and a dispatch row, which `engine-route-check.mjs` will demand. Make it `read · project` with the path checked LEXICALLY against registered roots: canonicalizing could fail on the very denial it exists to name, and the roots are already visible to the caller. It runs in serve, because the denial is the engine's. Its remedy names the RESPONSIBLE process's bundle id, which serve has no `app.config()` to read, so the shim passes it at spawn as it passes `WORKTREES_VIEWER_JS`. A remote client shows the card with "fix this at the host" and greys out the System Settings button. *Unmeasured: TCC attribution for a serve the app started.* | 2 |
| 7 | `open_docs_viewer` answers with `http://127.0.0.1:<port>/<token>/…`. That is meaningless off the host, and it carries the docs server's token. | `crates/worktrees-cli/src/serve/dispatch.rs:387`–`392`; `crates/worktrees-api/src/files.rs:870` | Phase 2: a remote caller gets `err: "unavailable-remote"` and no URL, and its UI greys the button with that reason. Phase 3: serve the pages through the authenticated socket (a `docs_get` read row); the docs server never listens off loopback. | 2, 3 |
| 8 | `sanitize_prefix` keeps a leading `-`, and the prefix can come from `[project] prefix` in a cloned repo's `.worktrees.toml`. This lane found no call site where a session name is a positional argument rather than a `-t`/`-s` value; it is defence in depth for R3. | `crates/worktrees-core/src/config.rs:16`–`27`, `:308` | Map a leading `-` to `_`. A session already named with a leading `-` is found by the old name once and renamed. | 2 |
| 9 | **New. Fixed-name temp files follow a planted symlink.** `write_file` writes `.<name>.wt-tmp` beside the target with `std::fs::write`, which follows a symlink, then chmods it and renames it over the target. A repository that tracks `.README.md.wt-tmp -> ../../.zshrc` turns the user's next save of `README.md` into a write of that content into `~/.zshrc`. `README.md` then becomes the link. The same shape is in every declared-state write (`.worktrees.places.json.tmp` in the repo root) and in `init_write` (`.worktrees.toml.tmp`). | `files.rs:1323`–`1328`; `crates/worktrees-core/src/store.rs:303`–`304`; `crates/worktrees-core/src/init.rs:1014`–`1015` | Create the temp file with `create_new(true)` (`O_CREAT \| O_EXCL`, which refuses an existing path, a dangling link included), with a random suffix, as `servefile::write` already does (`servefile.rs:108`–`113`). Sweep core for the same pattern. *Found by reading, not measured. The regression test must be shown failing first.* | **Closed on main by #523.** One `worktrees_core::fsx::write_atomic` for every site (store, init, provision, materialize, the dock save, the brief, the user-dir writes), with a guard test. A local bug, independent of Phase 2. One more site of the same class is in ROADMAP: `kimiplace::sync_file` writes through a tracked `.kimi-code` link. |
| 10 | **New. `repo` is never checked against the registry.** `set_note` and the other `store::edit` callers write `.worktrees.places.json` into any directory. `Project::discover` accepts any git repository. | §3.5 | The `project(…)` resolver, for every caller | 2 |
| 11 | **New. Serve has no off-loopback connect witness**, although decision 6 says it does | §4.6 | Copy the docs server's test | **Closed on main by #516.** `crates/worktrees-cli/tests/serve_bin.rs::serve_can_be_reached_on_loopback_and_not_off_it` runs in CI on both OSes, loopback connect first. Still missing: the release gate, which slice 0 adds (§4.6). |
| 12 | **New. Events go to every socket**, including the host-wide `sessions:busy` set and `app:open-doc` | `conn.rs:202`–`207`; `host.rs:316` | Projection per event type (§3.3) | 2 |
| 13 | **New. An authenticated client is unbounded:** a blocking task per request, an unbounded outbox, a lane per key | `conn.rs:545`, `:498`, `:217`–`237` | §6 | 2 |
| 14 | **New. Arbitrary-path reads exposed as Engine rows:** `probe_dir`, `skill_inspect`, `sync_hub_list` | `dispatch.rs:228`, `:447`, `:317` | `admin` with `new_path` or `global` (§3.4) | 2 |
| 15 | **New, a residue of L1:** `log_event` keeps newlines and panics on a mid-character truncation; serve.log is `0644` | `meta.rs:27`–`28`; `mod.rs:131` | §5 | **Closed on main by #516.** `meta::ui_line` escapes the controls, the line separators and the bidi controls, then truncates on a character boundary. serve.log is created `0600` and re-tightened on every write, the rotated copy included. |
| 16 | Known (Appendix B): `term_*` acts on any id, from any connection, and ids restart at 1 every launch | `term.rs:44`, `:138`; `dispatch.rs:422`–`429` | `attach(id)` (§3.3), plus a launch nonce in the id. A stale `term_close` becomes a no-op that answers `ok`; `term_write`, `term_resize` and `term_wheel` on an id the caller did not open are refused (§9.1). | 2. **No longer latent since #525** (§9.1): a pane being replaced after a reconnect sends its old id to the new socket. |

---

## 9. Multi-client effects (§III.6), now that one engine serves several clients

### 9.1 Per device, or per host

| State | Today | Ruling | Phase |
|---|---|---|---|
| View state in `ui-state.json`: selection, sidebar mode, sort, terminal visibility, dock open, tab and width, `place_panels`, `files_md_zoom`, theme | One blob, written whole by `set_settings` (`dispatch.rs:243`) | **Per device**, kept by the client: the shim's config dir on a desktop, the app container on iOS. It never reaches serve. A phone's selection is not the desktop's (§III.6.1). Theme follows each device's own appearance. | 2 |
| `last_seen_version` ("What's new") | The same blob, so the first client to load after an upgrade uses it up for everyone (Appendix B) | Per device | 2 |
| Shared preferences (default provider and the like) | The same blob | On the host, written by `patch_settings {key, value, rev}`: merged under a lock, refused if `rev` is stale. It is the `expected_mtime` compare-and-swap shape (`files.rs:1316`–`1320`) applied to a key. `key` comes from a CLOSED list of shared keys. A host-policy key or a per-device key is refused, so a client cannot reintroduce M2 through the settings door (API3, mass assignment). No shared key holds data keyed by project, so `get_settings` leaks nothing to a scoped device. | 2 |
| The five host policies | Two in `host-policy.json` (#508). The fetch interval and the two terminal switches are still pushed by the page (`App.tsx:4731`, `:4748` on main), at hydration and on each change. Since #525 that has a new failure: the effects do not depend on the connection, and App stays mounted across a reconnect, so a serve the app restarted after a crash keeps its boot defaults (auto-fetch off, scrollback persistence and per-tab history on) until the setting changes or the window reloads. For a user who turned persistence off, the next 15 s flush writes every dirty dock shell's ring to disk. *Found by reading, not measured.* | All five in `host-policy.json`, set only by explicit acts. Codex and pi are console-only; the other three are `admin` (§3.4). | 2 (slice 0, the first of §10) |
| `ui-events.jsonl` | Appended by any client | Loopback only (§3.4) | 2 |
| `mark_seen` | Stamps the client's clock, forward-only (Appendix B) | The host stamps its own `now`, and the client's epoch is ignored | 2 |
| `open_place` closing the second agent | Decided by whichever client selects the place first, using its own default (Appendix B) | Not decided by a client on load. The server decides, using the host's default, or the user is asked. | 2 |
| A dock shell's single sink | A second attach silences the first pane (`term.rs:438`–`462`) | A sink per attach. The most recently active attach owns the size, which is tmux's `window-size latest`. | 2 |
| Terminal ids | A global counter from 1, no owner (gap 16). Appendix B called it latent "because the transport's client never reconnects". Since #525 it does, including to a serve it restarted. Panes are keyed by the connection's `gen`, so each one remounts and opens a fresh id. But the pane being replaced closes its OLD id on unmount (`TerminalPane.tsx:106` on main), and that call goes out on the NEW socket. A new launch counts from 1 again, so the id may already name an attach it just handed out: a fresh pane, or another client's. *Found by reading, not measured.* | Owner check plus a launch nonce. With the owner check, the stale close is a no-op that answers `ok`, because the new connection never opened that id. A non-owned id and an unknown id look identical to the caller, and `term_close` already answers `ok` for an unknown one (`crates/worktrees-api/src/term.rs:315`–`320` on main). It is not a refusal because TerminalPane sends the close with no `.catch`, and App logs every unhandled rejection as an error (`App.tsx:4689`), so a refusal would log an error on every reconnect. The audit log records the no-op as a refused act (§5.1). `term_write`, `term_resize` and `term_wheel` on a non-owned id are refused with `no such terminal`, as an unknown id is today. | 2 |
| `app:open-doc` | Sent to every socket, so one `worktrees show` opens the document on every device | Loopback clients only: `worktrees show` is run by someone at the host. Slice 1 sends it, and releases a held one, to console connections only. Slice 1b narrows that to a loopback connection that declared it listens for `app:open-doc` (§9.3, §10). | 2 |
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

### 9.3 Held requests and the engine's lifetime, with devices connected

Phase 1 gave serve two rules that count its clients. Both assume every
client is a page on this host, and each needs a different count once a
device can connect.

**A held `worktrees show` goes to a listener, not to a socket.** Since #516,
serve keeps a `worktrees show` while nobody can see it, and releases it on
the first tick that has an audience (`watch::drain_inbox`). The audience is
`Host::has_audience`, which is true for any authenticated socket
(`crates/worktrees-cli/src/serve/conn.rs:215` on main). The app's door then
parks the event for up to 10 s until App listens (`ONE_SHOT_TTL_MS` in
`ipc.ts`, #524). Serve cannot tell who listens, because a page's `listen`
never reaches it: every event goes to every socket, and the client filters.
So two kinds of client take a held show and drop it:

- the shim's own verification probe, which completes handshake v1 and
  closes at once. ROADMAP records it, estimated at 0.1–0.2% of launches,
  not measured;
- in Phase 2, every device. §3.3 sends `app:open-doc` to loopback clients
  only, so a phone counted as an audience would drain the inbox into an
  event that no client receives.

*Ruling:* a connection declares the events it listens for, once after
authentication and again when the set changes. The door already holds that
set (`engineDoor`'s handlers, re-wired on every reconnect), so the client's
part is one frame. Serve releases a held request only while a LOOPBACK
connection declares `app:open-doc`, and sends it only to those connections.
A connection that declares nothing is sent no `app:open-doc`, so the rule
fails closed. Event projection (§3.3) can use the same set to skip a socket
that would drop a frame anyway.

*It ships in two steps* (§10). Slice 1 ships the narrower half:
`has_audience` counts console connections only, meaning connections on the
loopback listener that passed handshake v1 (§2.6), and `app:open-doc` goes
only to them. That closes the second case above before any device can
connect, which is slice 6. It does not close the first: the shim's probe is
a console connection, so it still counts as an audience. Slice 1b, a
follow-up after slice 1 merges, adds the `listen` frame and makes
`has_audience` "a loopback connection that declared `app:open-doc`".

A request still lives 30 s (`inbox::MAX_AGE_SECS`). A show is a host act, so
the update rehearsal's residue of a held show expiring after 30 s, when no
app came back, is correct behaviour, not a gap.

**An app-owned serve counts loopback clients for its idle stop.** Since
#524, a serve the app started (`owner: "app"`) stops itself after 120 s
with no client (`OWNED_IDLE_GRACE`,
`crates/worktrees-cli/src/serve/mod.rs:239` on main), so a crash leftover
does not run unseen. It counts every authenticated socket
(`conn::Clients`). If devices counted, a phone left connected would keep a
crashed app's serve running for as long as the phone stayed. The update
rehearsal's other two residues (ROADMAP, "Re-run the update rehearsal")
would then have no bound:

- **A downgrade inside the grace runs two engines:** the older app's own
  engine at the desk, and this serve. Today that lasts at most the 120 s
  grace. With a device holding the serve up, the device and the desk would
  drive two engines for as long as it stayed, each with its own terminals,
  dock shells and in-memory host policy.
- **With AppImage's extract-and-run, a crash leaves serve running from a
  mount that no longer exists,** for up to 2 minutes. With a device holding
  it up, the serve would go on answering that device from a deleted tree.

*Ruling (a recommendation, not yet put to the user):* the idle count is
loopback clients only, and nothing a device does keeps an app-owned serve
alive. So a clean quit of the app ends remote access too, and while remote
access is on, the quit says that it will. A host meant to serve devices
with no app running runs `worktrees serve` itself. A serve with no owner is
never signalled and never idles out (Phase 1's rule, §III.9).

---

## 10. Order of work and exit criteria

Each slice merges on its own, and nothing binds off loopback until slice 6.

0. **Local fixes, no remote:** the release gate for the loopback connect
   witness (§4.6), gap 8, `cargo deny` in CI (§4.2), and the three host
   policies still pushed by the page (Appendix B, "One fix for all five").
   Since #525 that last item can turn a user's "off" back on (§9.1), which
   makes it the most urgent of the four. Already on main: gap 9 (#523), the
   witness itself (gap 11), gap 15's L1 fix, and `catch_unwind` on every
   body (all #516). The in-flight drop guard that §6 pairs with
   `catch_unwind` comes with the caps in slice 4.
1. **The dispatch table:** `HostConsole`, the capability and object columns,
   the resolver, result and event projection, and the tests in §3.3.
   Loopback behaviour is unchanged apart from §3.5. Gaps 2, 3, 4 (the
   console tag), 5 (all but the `[docs]` walk), 7 (the remote refusal), 10,
   12, 14 and 16. `has_audience` counts console connections only, and
   `app:open-doc` goes only to them (§9.3).

   **1b. The declared listener set** (§9.3), its own item, after slice 1
   merges: the `listen` frame, and `has_audience` becomes a loopback
   connection that declared `app:open-doc`. Until it lands, the shim's
   verification probe still takes a held `worktrees show`.
2. **The path classifier** (§7.1): gap 1, and the `[docs]` walk (gap 5).
3. **The audit log** (§5).
4. **Resource limits** (§6), including the loopback budget split: gap 13.
5. **The device registry, server identity and pairing** (§2): the principal
   and device records (§2.8), console rows, the pairing listener, the QR,
   the comparison code, revocation, per-device expiry, step-up, and the
   cross-check and alerts (§5.2, §5.3).
6. **The remote listener** (§4), behind the opt-in: the address classifier,
   the address-change rules and the address code (§4.1), mDNS rediscovery
   (§4.7), the shim's remote door and the plugin calls a remote client
   greys (§4.4), the idle stop that counts loopback clients only (§9.3),
   and both witnesses (§4.6, §4.7). Every
   gap in §8 is closed or tagged by now. Gap 6 lands with #519, as its row
   says.

**A spike, before slice 5's design is frozen:** on a real iPhone (TestFlight)
and a second Mac on the tailnet, measure the claims marked unmeasured above.

The half that needs neither ran on 2026-10-21: macOS 27 on Apple silicon,
read-only with no sudo, and Docker containers with their own network
namespaces on Docker Desktop's Linux 6.12 (`linuxkit`), a kernel that is
neither a stock distribution's nor a GitHub runner's. Its probes are in `scripts/spike/p2-network/`,
whose README lists them. Its findings, and the hardware checklist with the
exact command for each step named in brackets below, are the lane's
`~/.cache/worktrees/worktrees/spike-p2-network/findings.md` on the machine
that ran it. **Measured** (the result is in the section cited):

- the network fingerprint on both platforms: the point-to-point flag, the
  subnet and the IPv6 temporary and deprecated flags, all readable without
  root; and the gateway's link-layer address, which netlink gives on Linux
  and macOS hides from a non-root process, while SystemConfiguration
  publishes it (§4.1);
- what Linux does to a listener whose address is removed: nothing (§4.1);
- the weak host model on Linux, under every `rp_filter` mode; the macOS
  default, a hybrid that is strong for `utun` (from sysctl and XNU's
  source, not from the wire); and whether `SO_BINDTODEVICE` (Linux, both
  families, unprivileged) and `IP_BOUND_IF` (macOS, IPv4) scope a
  listening socket (§4.1, peer rule part 2);
- Avahi registering on one interface and its goodbye, seen from a peer
  container on each link (§4.7);
- registering on one interface through mDNSResponder, as far as the host's
  own browser sees it (§4.7);
- keychain behaviour for an ad-hoc-signed macOS build (§2.1);
- Rust's parsers against link-local and odd IPv4 strings, and how an
  unscoped link-local dial fails on each OS (§4.1).

**Still to measure**, on the user's hardware:

- *iPhone.* rustls from the Rust shim with a Secure Enclave signer [A1];
  whether ATS or Local Network privacy touches a socket opened from Rust,
  on `100.x` and on a LAN address [A2]; what a full handshake costs per
  reconnect with a Secure Enclave signer, since resumption is off (§4.2;
  the signature alone took 4.7 ms on the spike's Mac) [A3]; a
  `.userPresence` step-up signature requested from Rust (§2.5) [A4];
  whether an iOS device can dial a link-local address from a QR, IPv4 and
  IPv6, scoped to an interface it picks itself (§4.1) [A5]; Bonjour on iOS
  (§4.4, §4.7): browsing `_worktrees._tcp` from the shim, through the
  `dns_sd` C API or an `NWBrowser` plugin, that a type missing from
  `NSBonjourServices` fails, when the Local Network prompt appears, and
  what a declined prompt looks like to the app [A6].
- *A second Mac.* The hybrid host model across hosts: a packet for the
  Tailscale address that arrives on Wi-Fi is dropped, for IPv4 and IPv6,
  and whether `IPV6_BOUND_IF` filters it where the kernel does not (§4.1)
  [B1]; the mDNS wire capture from a second machine: mDNSResponder's
  records, TTLs and goodbye after a release and after a kill, and nothing
  on `awdl0` (§4.7) [B2]; the Bonjour Sleep Proxy (§4.7) [B3]; what macOS
  does to a listener whose address is removed, on a scratch interface
  under sudo (§4.1) [B4]; link-local between two Macs on a Thunderbolt
  Bridge (§4.1) [B5]; macOS's own Local Network permission (macOS 15 and
  later), on both ends: whether it touches serve's listener or its advert,
  which process the prompt is attributed to when the app started serve,
  and whether an ad-hoc-signed client is asked again after every update,
  as TCC is (AGENTS.md); and the Application Firewall's "accept incoming
  connections" prompt for an ad-hoc serve, again after a rebuild [B6].
- *The opaque host name* (§4.7), new, with no probe yet (`mdns_reg_mac.c`
  and `avahi_reg.py` are where to start): `DNSServiceRegisterRecord` plus
  `DNSServiceRegister` with `host`, and Avahi's `AddAddress` (with
  `AVAHI_PUBLISH_NO_REVERSE`) plus `AddService` with `host`, each from an
  unentitled, unprivileged process. Seen from a second machine: the SRV
  target is `wt-<id>.local`, a resolve returns the bound address and no
  other, both records go on a release and on a kill, and a name conflict
  is reported rather than renamed. From the iPhone, that the shim resolves
  it [with A6].
- *A reboot.* `utun` numbering across reboots, a Tailscale restart and a
  sleep, which decides whether the point-to-point fingerprint may leave out
  the name, and whether the network service's UUID stays stable where the
  name does not (§4.1) [C1].
- *CI runners.* Whether the macOS runner allows the scratch interface or
  alias the bind-rule witness needs (§4.6) [D1]; on the Linux runner's own
  kernel, an unprivileged `SO_BINDTODEVICE`, the scratch interfaces, and
  whether Avahi is there at all [D2].

**Exit, which is §III.9's, made concrete:**

- [ ] A second machine on the tailnet, paired by QR, drives serve: list,
      mutate, one terminal attach.
- [ ] The same in LAN mode, from an iPhone on the host's Wi-Fi, with the
      Local Network prompt answered.
- [ ] The host's LAN address moves, the host chooses the new one, and the
      iPhone finds it over mDNS and reconnects with no address code. A
      spoofed advert for the same id is tried, refused at the pin, and
      changes nothing on the phone. The advert names `wt-<id>.local`, which
      resolves to the bound address alone, or, on a platform that took the
      fallback, the enable dialog showed the OS's name first (§4.7).
- [ ] An unenrolled device cannot connect, measured at TLS.
- [ ] A revoked device's live socket dies within 1 s, and it cannot
      reconnect. An expired device is refused, and a re-enabled one is
      served.
- [ ] A public address is refused at bind, and an address change pauses
      and resumes remote access as §4.1 says, on macOS and Linux.
- [ ] With a device connected and the app killed, the app-owned serve
      still stops after its grace. A `worktrees show` run while the device
      is connected waits for the desktop and is never sent to the device
      (§9.3).
- [ ] Every step of §4.6 passes on macOS and Linux, and the release gate
      greps its exact count.
- [ ] R1: every row declares its object and capability, the drift tests are
      green, and each one was shown failing first.
- [ ] R4: pairing as §2. R5: §5. R6: §6, with the numbers measured. R7: §7.
      R8: §4.6. R9: decided "maybe later" (§11 Q1), and built as §2.8
      says: a principal in every device record and audit record, every
      grant check keyed on (principal, device), and the second-principal
      fixtures shown failing first.
- [ ] Gaps 1–16: each closed, or tagged with its phase in this table and in
      ROADMAP.

---

## 11. Decided 2026-10-20

The draft put eight questions here that only the user could answer, and the
user answered all eight the same day. Two answers depart from the
recommendation: Q1 and Q7. Each answer is below with what it changed. The
numbering is the draft's, and the rest of this document cites it as
§11 Q1 to Q8.

1. **Multi-user (R9): maybe later.** *Departs from the recommendation*,
   which was to rule multi-user out for good so the schema could rely on
   that. It is not built in Phase 2, and it is not designed out.
   *Consequence:* a principal identity beside every device, kept distinct
   in the device record and in what the device's credential binds to
   (§2.1); a principal on every audit record (§5.1); every grant check keyed
   on (principal, device) (§3.3), with one principal, the owner, for now.
   §2.8 has the design, what it costs now (a field, a list, an intersection,
   and tests for paths nothing reaches yet) and what multi-user would still
   take: above all, per-user Unix accounts before a second person may hold
   a shell.
2. **The overlay sits under pairing.** As recommended: our own device keys
   always, with Tailscale or another overlay as transport and network ACL.
   *Consequence:* none beyond the draft (§2.7).
3. **`update_cli` is refused from remote devices outright.** As
   recommended. *Consequence:* it stays `HostConsole` (§3.4).
4. **Pairing pre-selects Observe.** As recommended. *Consequence:* §2.2 step
   1 and §3.1. Every phone paired to answer Claude costs one deliberate
   click to Drive, which carries its label.
5. **Only the host may open a pairing window.** As recommended. A paired
   device never can, `admin` or not. *Consequence:* §2.4. Nobody can add a
   device while away from the desk.
6. **LAN mode is in Phase 2, and public addresses are refused outright.**
   As recommended, with the user's classes: RFC 1918, ULA `fc00::/7`,
   link-local and CGNAT `100.64.0.0/10` count as private, and anything else
   is refused. That widens the draft in two places: link-local was refused
   there, and ULA was only `fd00::/8`. *Consequence:* §4.1 states the
   allowlist as a table and one classifier shared by serve and the shim, the
   peer filter's answer to port forwarding, and what happens when an
   interface's address changes (pause, resume on the same network, ask on
   any other) along with the address code that lets a device follow a moved
   bind. §4.6 adds a bind-rule witness made of connects, in which a public
   address that is bindable is refused. §4.4 ships the Local Network usage
   string.
7. **Expiry is set per device from the host.** *Departs from the
   recommendation*, which was one host-wide idle rule for every device.
   *Consequence:* each device record carries its own `expiry`, chosen on the
   host at pairing and edited from the host console only: disabled after N
   days unused (the default is 30), at a date, or never. An expired device
   is disabled, not deleted, and only the host re-enables it. §2.5 gives the
   reasons for the default and defines "seen". §5.1 records every
   transition, §5.2 catches an expiry edited into the file by hand, and
   §5.3 alerts.
8. **Step-up is on by default** for iOS and iPadOS devices holding `stream`,
   `instruct`, `answer` or `admin`, with Face ID or the passcode and a proof
   that lasts 5 minutes per connection. As recommended. *Consequence:*
   §2.5. Turning it off for one device removes a factor, so it is a console
   act, audited, and shown in the device list.

### 11.1 Decided 2026-10-20: the questions those answers raised

The answers above raised three more questions, and the user answered them
the same day. One departs from the recommendation: N2. A fourth point was
decided by `(main)` on the user's behalf, and is recorded here so that it
is visible.

- **N1. CGNAT on an interface that is not a tunnel: allowed, with a
  warning.** As recommended. `100.64.0.0/10` is private for Tailscale, but
  on a phone hotspot, and with some carriers and satellite ISPs, it is
  shared with strangers. *Consequence:* the enable dialog warns and names
  the interface (§4.1), and mTLS holds either way. Such a bind is not
  advertised over mDNS (§4.7).
- **N2. Rediscovery by mDNS (Bonjour) is in Phase 2.** *Departs from the
  recommendation*, which was to leave it for Phase 3 because it is more
  surface on the LAN and announces the host to every device there.
  *Consequence:* a new §4.7. The host is advertised only while LAN mode is
  on, and only on the bound interface. Serve puts no project or place name
  and no pin in the advert, only an opaque id that paired devices hold.
  *Amended after the spike of 2026-10-21:* the host name the advert points
  at was to be the OS's own, and on a default Mac that name carries the
  owner's first name. §4.7 now prefers a host name of serve's own, made
  from the same opaque id (recommended, unmeasured), and where a platform
  cannot do that, the enable dialog shows the OS's name before discovery
  goes on (§12.4 T1). A
  discovered address is adopted only after a pinned handshake to it
  succeeds, the rule S8 set for the address code, and the address code stays
  as the fallback. §4.7 says what it costs and what it exposes; §4.4 and
  §10 cover the iOS side; §12.2 is its own review.
- **N3. The expiry default is 30 days unused, for every preset.** As
  recommended. *Consequence:* §2.5 as written. A default that depended on
  the preset would be one more thing to explain at the moment of pairing.
- **Routed private peers on a LAN bind stay refused, as a default.**
  Decided by `(main)` on the user's behalf. This is part 3 of §4.1's peer
  rule. Unlike parts 1 and 2, it is not a security requirement: a routed
  private peer still has to get through mTLS, so refusing it shrinks who
  can reach the TLS handshake but protects no asset by itself. It is
  refused in Phase 2 because no network in scope needs it. If one does
  later, it can become a console-only list of extra private CIDRs, each
  passing the classifier, and audited when it changes.

### 11.2 Decided 2026-10-20: raised by N2

- **N4. A new address on the SAME network rebinds by itself.** As
  recommended. Before this, any new address paused remote access until the
  host chose it, even when the interface, the subnet and the gateway were
  all unchanged, which is exactly what a moved DHCP lease looks like. With
  mDNS in Phase 2 the devices can follow such a move without help, and now
  the host does too. The fingerprint already decides what "the same
  network" means, and a new address inside it changes nothing about who
  can reach the port. *Consequence:* §4.1 rebinds by itself when the
  fingerprint matches and only the address within the subnet changed,
  audited as `bind moved`, and any other change still pauses and asks at
  the host. §4.6 step 4 witnesses both. §12.3 is the check on whether a
  LAN attacker can steer such a rebind.

---

## 12. The OWASP review of this design

The draft was reviewed with the `owasp-security` skill against OWASP Top
10:2025, the API Security Top 10:2023, ASVS 5.0 at L2 (V1–V4, V6–V9, V12–V14
and the new V17 on WebSockets), the Agentic Top 10 (ASI01–ASI10) and the LLM
Top 10. It was a review of a design, so every finding below is a change to
this document, already folded in, not a bug in shipped code. The exceptions
are two facts the review checked in the code at `fadb2ce` and found true: O6
and O15. #516 has since fixed O15's unanswered panic. Its in-flight half
waits for the caps (§6).

**This first pass predates the decisions of 2026-10-20 (§11).** A second
pass covered what those decisions changed, and §12.1 lists what it changed
in turn. §12.2 is a third pass, over the mDNS design (§4.7) that §11.1 N2
added, §12.3 a check of the automatic rebind that §11.2 N4 added, and
§12.4 a fourth pass over what the network spike measured.

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
| O15 | A10, API4 | A body that panics is contained, but its request is never answered (`conn.rs:539`–`541` pushes only on return), so the client hangs, and in-flight caps would leak a slot. A corrupt `devices.json` had no rule. | `catch_unwind` answers `err: "internal error"` (on main since #516), and the in-flight count is a drop guard. A corrupt file refuses every device and is not overwritten. (§5.2, §6, slice 4) |
| O16 | A06, A09 | A destructive act from a device left no way back. | `remove_place` with `del_branch` records the tip SHA in the audit log. (§5.1) |
| O17 | A03 | A TLS stack joins the network-facing binary with no advisory check in CI. | `cargo deny check advisories` in CI. (§4.2, slice 0) |
| O18 | ASI09 | Answering on a small screen invites approving a truncated command. | The answer verb renders the whole prompt, and there is no approve-all. (§9.2) |

**Considered and not changed.**

- *Client-certificate expiry dates.* The pin is the identity and revocation
  is immediate, so validity dates would add rotation work without closing
  anything. Per-device expiry (§2.5) covers forgotten devices.
- *A hardware-backed server key* (ASVS V6.4.2). macOS builds are ad-hoc
  signed (§2.1), so the key stays a `0600` file. That is recorded as a
  residual, to revisit with a team-signed build. §12.4 narrows it.
- *Making `executes` paths console-only* instead of `stream`. A `stream`
  device already has a shell, so console-only would cost the user something
  and protect nothing.

### 12.1 Second pass, 2026-10-20: what the decisions changed

The user's answers (§11) added principals (§2.8), per-device expiry (§2.5),
LAN binds with one address classifier, and a bind-rule witness (§4.1,
§4.6). This pass reviewed exactly those parts, again with the
`owasp-security` skill: OWASP Top 10:2025 (A01, A05, A06, A07, A08, A09,
A10), the API Security Top 10 (API1), ASVS 5.0 at L2 (access control,
session management, input validation, logging, communication) and ASI03.
As before, every finding is a change to this document, already folded in.
It also answered one question put to it directly: whether refusing routed
peers is a security requirement or a default (S2).

**What held up.** Credentials that carry no identity claim, so a device
cannot assert a principal. Expiry and step-up widened only from the
console, and checked against the audit chain. An allowlist classifier
rather than a denylist, shared by host and device. Refusals with their own
error kind, so a refusal is never confused with a failed bind. Connect
witnesses with the served case first.

**What it changed.** Ten changes:

| # | OWASP | Finding in the revision | Change, and where |
|---|---|---|---|
| S1 | A01, ASVS session management | A disable, a passed `until` and a narrowed scope were enforced per REQUEST. A terminal attach, a dock shell and the event stream reach a device without one, so a disabled device would have kept receiving terminal bytes until it next asked for something. | A reduction acts on what is already flowing: a disable or a passed `until` closes the device's connections as a revoke does, a narrowed scope ends the channels whose object it no longer covers, and event projection reads the grant at each emit. (§2.5, §3.3) |
| S2 | A01, A06 | The peer filter said "same class", while the table made `10.x` and `192.168.x` one class and the example called them different. Worse, both platforms accept a packet for any local address on any interface, so a Wi-Fi neighbour could reach a Tailscale-bound listener through the LAN, around the tailnet ACL that §11 Q2 puts under pairing. | Three parts, each with its standing stated. Every peer private: a requirement. An overlay bind's peer in the overlay's own row, with the listener tied to its interface where the OS allows: a requirement. A LAN bind's peer on-link: a default, which could later become a console-only list of private CIDRs. A connect witness for each. (§4.1, §4.6) |
| S3 | API1 (BOLA) | `device(id)` had no resolver entry, so nothing said what an `admin` device is told about another principal's device, or what a device listing contains. | `device(id)` resolves within the caller's principal and gives the identical not-found error, and device listings are filtered the same way. (§3.3) |
| S4 | A01 | "Principal ∩ device" was not defined for `"*"`. Read naively, a device with `"*"` under a principal with a list could reach projects outside the list, and a row needing scope `"*"` could pass on the device's side alone. | `"*"` ∩ X is X, evaluated per request, and a row that needs `"*"` needs it on both sides. (§2.8, §3.3) |
| S5 | A01, A10 | The owner's id was random and stored in `devices.json`. A corrupt or deleted file would have left the console with no principal to act as, and a recreated one would have split the audit trail under a new id. A second principal in the file was a state no code creates, and nothing refused it. | The owner is the reserved id `owner`, never read from the file. The Phase 2 loader treats any other principal as corruption. (§2.1, §2.8, §5.2) |
| S6 | A09, A08 | §5.2 checks each device against its `enrolled` record and the records that set its scope, expiry and step-up. Rotation keeps 25 MiB, so a device paired long ago would have lost that evidence and been disabled as `unrecorded`. | Each new file opens with a `checkpoint`: the previous file's final hash and every live device's security fields. §5.2 reads from the newest checkpoint, matching on `detail.subject`. (§5.1, §5.2) |
| S7 | A05, ASVS input validation | "Canonical bytes, never its spelling" named no parser, and the classic ways around an address check are spellings: `127.1`, `0x7f.0.0.1`, leading zeros. | Rust's strict `std::net` parsers only, never `inet_aton` or a resolver. A zone is split off and must match `--iface`, and an unparsable string is refused, never repaired. The table test includes the spellings. (§4.1, §4.6) |
| S8 | A06, A07 | The address code carries only public data, so anyone can print one. A device that switched to its address at once could be pointed at a stranger's machine: a denial of service by poster. | The code is marked unauthenticated, and the device adopts its address only after a pinned handshake to it succeeds. (§4.1) |
| S9 | A06 | A point-to-point fingerprint that included the interface NAME would break across reboots on macOS, which numbers `utun` interfaces in creation order. Each break is a pause dialog, and a dialog that fires for no reason trains the user to accept the one that matters. | The point-to-point fingerprint is the address, its prefix and the point-to-point flag, not the name. Measured in the spike. (§4.1, §10) |
| S10 | A07, A10 | Nothing said whether an expiry was recomputed from the clock at each check, so setting the host clock back could have re-enabled an expired device. | An expiry is decided once and recorded as `disabled`. (§2.5) |

**Considered and not changed.**

- *Refusing `"never"` for a device that can run commands.* §11 Q7 puts
  expiry in the host's hands, the device list marks `"never"`, and the
  user kept one 30-day default for every preset (§11.1 N3).
- *Signing the address code with the server's key.* The pinned handshake
  that follows already authenticates the address, so a signature would
  check the same fact twice.
- *Address-change events (`PF_ROUTE`, netlink) instead of the 3 s poll.*
  The gap is at most one poll, and it sits behind mTLS. Worth doing when the
  listener is written, but not a design requirement.
- *A setting for routed LAN peers in Phase 2* (S2, part 3). Refusing them
  protects no asset by itself, so such a setting would be safe, but nothing
  in scope needs one yet.

### 12.2 Third pass, 2026-10-20: mDNS rediscovery

§11.1 N2 put rediscovery over mDNS into Phase 2, against the
recommendation. §4.7 was drafted to the user's constraints, then reviewed
with the `owasp-security` skill, together with its hooks in §2.1 (the
discovery id), §4.3, §4.4 (iOS), §5.1 and §10. The review covered OWASP
Top 10:2025 (A01, A02, A05, A06, A07, A09), ASVS 5.0 at L2 (input
validation, communication, logging) and the zero-trust and
secure-by-default principles, with the brief's focus: spoofed and replayed
adverts, LAN reconnaissance, the device as a confused deputy, untrusted TXT
parsing, exposure, and the platform permissions.

**What held up.** Discovery is never trusted on its own: an address is
adopted only after a pinned handshake. The OS's own responder answers, so
serve parses no multicast traffic. The advert carries only constants and a
random id that nothing public derives. It exists only in LAN mode, on one
interface, never for N1's shared CGNAT links, and it is withdrawn with the
listener. The address code stays as the fallback.

**What it changed.** Six changes to the draft:

| # | OWASP | Finding in the draft | Change, and where |
|---|---|---|---|
| D1 | A01, zero trust | The device "tries their addresses" with no limit on which addresses. A spoofed advert could point a phone at any address it can reach, a VPN's networks included, and use it as a probe. | A candidate must pass the classifier and be on-link for the interface its advert arrived on. At most four distinct candidates per round, none tried twice. (§4.7) |
| D2 | A05, ASVS input validation | "Keeps the instances whose id matches" did not say which field matched, or how an advert is parsed. mDNS renames an instance whose name was claimed first, so a squatter matching on the instance name could hide the real host and stand in its place. | The device matches on the TXT `id` and ignores the instance name. An advert with any TXT shape but exactly `v=1` and a 26-character base32 `id` is dropped. (§4.7) |
| D3 | A02, A06 | The draft did not say the pairing listener stays unadvertised. One bad attempt closes a pairing window (§2.4), so a window that announced itself could be burned by anyone on the link. | The pairing port is never advertised, even with a window open, and the witness checks it. The advert is built from constants, so no client string reaches it. (§4.7) |
| D4 | A07, A04 | What a spoofer learns when a device dials it was unstated. A ClientHello carrying SNI would hand it the host's name. | The pin is checked before the client sends a certificate (the TLS 1.3 order), so a spoofer gets nothing about the device's key, and the client sends no SNI. A test checks both. (§4.3, §4.7) |
| D5 | A06 | Browsing was unbounded in time. A phone that keeps querying `_worktrees._tcp` announces on every network it joins that it runs a worktrees client: reconnaissance pointed the other way. | The device browses only when its stored address fails, for at most 30 seconds, with back-off, and never in the background. (§4.7) |
| D6 | A09, secure by default | An advert announcing the host had no visible counterpart on the host, and there was no way to stay in LAN mode without it. | The remote-access indicator says when the host can be found, the advert is audited, and a console switch (`remote_advertise`) turns it off while LAN mode stays on. (§3.4, §4.7, §5.1) |

**Considered and not changed.**

- *A discovery id that rotates* (an HMAC of a per-host key over the time).
  The host's own `.local` name, which its responder answers for whether or
  not serve runs, is just as linkable, so rotating the id would hide
  nothing the OS does not already show. That still holds since §12.4 T1
  moved the SRV target off that name.
- *Changing the discovery id when a device is revoked.* A revoked device
  that still recognises the advert learns only what a scan of the LAN
  shows. A new id would also strand any device whose stored address had
  gone stale before it could learn the id. Rotating the server identity
  (§2.5) changes it.
- *Signed adverts* (DNSSEC or SIG(0) on the records). The pinned handshake
  already authenticates the host behind any advert, so signing would check
  the same fact twice.
- *A responder of serve's own.* It would put a multicast parser in the
  engine's process, and on iOS a client-side one would need the restricted
  multicast entitlement. The OS's responder is already running.
- *Denial of service by a LAN attacker.* Flooding adverts, or spoofing ARP,
  can keep a device from its host, and nothing at this layer prevents it.
  The address code is the way back.

### 12.3 Check, 2026-10-20: the automatic rebind

§11.2 N4 lets serve move its LAN bind by itself when the fingerprint
(interface, subnet, gateway link-layer address) matches and only the host's
own address changed. This check, again with the `owasp-security` skill,
asked one question of that rule: can an attacker steer such a rebind, and
what would it gain? It covered A01, A05, A06, A09 and A10, and the
habituation concern ASI09 names.

**The answer: a LAN attacker can steer it, only within the same subnet,
and gains nothing but denial of service.**

- **A rogue DHCP server, or a rogue IPv6 router advertisement, on the
  host's own LAN** can give the host a new address. If it keeps the subnet
  and the real gateway, the fingerprint matches and serve moves. The
  listener is still on the same network, in front of the same peers, and
  the peer rule (§4.1, part 3) is relative to that same subnet. mTLS and
  the pins are untouched. If the rogue server names itself as the gateway,
  the gateway's link-layer address changes, the fingerprint does not
  match, and serve pauses. A new IPv6 prefix is a different subnet, which
  pauses too.
- **ARP spoofing of the gateway** changes the link-layer address the host
  sees, so the fingerprint stops matching and serve pauses. That is a
  denial of service, plus a dialog the attacker hopes the user will click
  through.
- **An evil twin** (a network the laptop joins that copies the home
  subnet and the gateway's link-layer address) needs that address and a
  network the laptop will join. Such an attacker could already have served
  the host its old address, since a DHCP client asks for the address it
  had, and that address auto-resumed before N4. So N4 adds no capability.
  What the attacker gets is the TLS port in front of its own network, with
  mTLS still the gate, as §4.1 already says: the fingerprint stops accidents,
  not attackers.

**What it changed.** Four limits, all in §4.1:

| # | OWASP | Finding | Change, and where |
|---|---|---|---|
| R1 | A06, A10 | A rogue DHCP server could make the listener move without end, dropping every remote connection and writing an audit record each time. | At most three automatic moves an hour. The fourth pauses and asks, with an alert. (§4.1, §4.6, §5.3) |
| R2 | A09, ASI09 | A gateway change on the same subnet would have shown the generic "different network" dialog, inviting the click that records an impersonator as the new fingerprint. | That case has its own words, naming ARP spoofing as one cause, and an alert. (§4.1, §5.3) |
| R3 | A05, A06 | "The same subnet" was only as safe as the netmask recorded at enable. A wide one such as `/7` on a `10.x` address also covers `11.0.0.0/8`, which is public. | The recorded subnet must lie wholly inside one allowlist row, and the classifier runs again on every new address. (§4.1) |
| R4 | A06 | A move left an open pairing window whose QR names the old address, and it did not say what happens to the mDNS advert, which is tied to a listener that closes and reopens. | A move closes any pairing window, and withdraws the advert and registers it again with the new listener. (§4.1) |

**Considered and not changed.**

- *Adding the Wi-Fi network's name (SSID) or the access point's address
  (BSSID) to the fingerprint.* Reading either needs Location permission on
  recent macOS, and an evil twin copies the SSID anyway. The gateway's
  link-layer address already tells home from elsewhere for accidents, which
  is all the fingerprint is for.
- *Authenticating the gateway.* Nothing on a home LAN offers it, and mTLS
  already makes the network's honesty irrelevant to who gets in.

### 12.4 Fourth pass, 2026-10-22: the spike's measurements

The network spike of 2026-10-21 (§10) measured most of what §4.1, §4.6 and
§4.7 had left unmeasured, on macOS 27 and on Linux 6.12 in containers, and
§2.1's keychain claim. This pass, again with the `owasp-security` skill,
asked which of its results move an OWASP-relevant claim. Most confirm one.
Six did not, and each is folded in above:

| # | OWASP | What the spike found | Change, and where |
|---|---|---|---|
| T1 | A02, ASVS V8 | The advert was said to reveal nothing but an opaque id. Its SRV target is the OS's `.local` name, which on a default Mac is `<First name>s-MacBook-Pro`, and a resolve lists every address on the interface (IPv4, ULA, link-local). The type is enumerable under `_services._dns-sd._udp`, and the PTR and TXT records live 75 minutes in a peer's cache. | Serve registers its own host name, `wt-<id>.local`, with the bound address only (recommended, unmeasured). Where a platform cannot, the enable dialog shows the OS's name before discovery goes on. Enumeration and the Sleep Proxy are listed as exposures. Decision 7 and §11.1 N2 amended. (§4.7, §10) |
| T2 | A10, A06 | "After one poll it no longer matters" assumed something would close a listener whose address left. On Linux nothing does: the listener and its connections survive, a re-added address is served by the old listener, and a moved address is served on its new interface. | Serve's close at the poll is mandatory. The poll checks the bound interface and its index, resume binds a new socket, and a pause frees the mDNS registration, which Avahi would otherwise announce again by itself. (§4.1, §4.7) |
| T3 | A01, ASVS V1 | The rationale for peer rule part 2 said both platforms are weak hosts. macOS is a hybrid that is strong for `utun` while IP forwarding is off. | Rationale corrected; the rule is not. Part 2 stays a requirement: Linux is weak, and macOS is weak with forwarding on or for an overlay that is not a `utun`. The listener is bound to its interface on both, which pins the interface index. (§4.1) |
| T4 | A06, R8 | With a device-bound listener, §4.6 step 5's out-of-row source on ANOTHER scratch interface is refused by the kernel, so the assertion would pass with the peer filter deleted. | The out-of-row source sits on the same point-to-point interface, and the step is shown failing with the filter removed. (§4.6) |
| T5 | A05, ASVS V5 | Rust's `SocketAddr` parser accepts a numeric zone, so `[fe80::1%1]:7000` in a QR would put the host's interface number into the device's connect, past the zone rule. | The QR and address-code parsers refuse any `%` and parse the host with `IpAddr`. A device scopes a link-local dial, IPv4 or IPv6, to an interface it picks itself. (§4.1) |
| T6 | A09, ASI09 | The gateway's link-layer address was to come from the neighbour table. macOS hides that table from a non-root process, and on both platforms an entry is often simply absent. Read naively, a quiet network would raise the ARP-spoofing alert, and an alert that fires for nothing teaches the click that matters. | macOS reads SystemConfiguration. An absent entry is "not known yet", and only a present, different address raises the dialog and the alert. (§4.1) |

**Considered and not changed.**

- *The hardware-backed server key* that the first pass left as a residual
  (ASVS V6.4.2). The spike found that an unentitled CryptoKit Secure
  Enclave key works in an ad-hoc build, and that its blob survives a
  rebuild, so the residual no longer waits for a team-signed build. It
  stays a `0600` PKCS#8 file until the Rust signer path is settled (§2.1).
- *Advertising on a TAP-based overlay* (ZeroTier's shape), which is not
  point-to-point and so gets LAN rules and the advert. Its virtual network
  may span sites, but every peer on it is one the overlay admitted, the
  pinned handshake is the gate as on any LAN, and the console switch turns
  the advert off (§4.7).
