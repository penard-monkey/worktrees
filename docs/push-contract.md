# Push notifications: the host's contract

What `worktrees serve` promises the iOS app about notifications. The design is
the iOS repository's `docs/design/notifications.md`; this file is the wire as
built, and where it differs from that design it says so. The tests read the
files in `docs/push-fixtures/`, so a change to the host that breaks them fails
here first. The phone can read them at the commit it pins.

| File | What it holds |
|---|---|
| `seal-vector.json` | A fixed key, key id, nonce, AAD, plaintext and ciphertext, made with node:crypto rather than either side's code. For `plugins/platform/tests/push-crypto.sh` (BACKLOG B-14). |
| `rows.json` | Every row's answer, in the order a phone meets them, and each refused argument. |
| `payloads.json` | Every kind of push: the plaintext, what Apple sees, the headers. |

## Knowing the host has them

The remote `ok` frame carries `push: true`. An older host does not have the key,
and answers the rows `unknown command: <row>`. `PROTO` did not change: a new row
never moves it. `PUSH_SINCE` is the first release with these rows.

## The rows

`push_register`, `push_unregister`, `push_prefs` and `push_test` are `Own`: any
paired device, whatever its grant, Observe included, and no step-up. A device
acts on its own record only. Its identity is its connection, and no argument
names a device.

| Row | Arguments | Answer |
|---|---|---|
| `push_register` | `token` (lowercase hex, 32 to 100 bytes), `env` (`development` or `production`), `topic`, `key` (32 bytes), `key_id` (8 bytes), the last two standard padded base64 | `{registered: true}` |
| `push_unregister` | none | `{registered: false}`, however often |
| `push_prefs` | none reads; each of `input`, `reminder`, `security`, `muted` given is set | `{registered, input, reminder, security, muted: [{repo, slug?}]}` |
| `push_test` | none | `{sent: true}` once Apple took it |

A registration replaces the one before it and keeps the switches. So do a dead
token and `push_unregister`, because a phone that registers again must not lose
its mutes.

`security` is `null` until the console has allowed it for this device, and
setting it then is refused with `denied`. When the console allows it, it reads
`true`, and the phone may turn it off. A muted repo or slug the device's grant
does not cover is dropped, not an error, and is not named back later if the
grant narrows.

Errors, all exact strings or prefixes:

| Error | When |
|---|---|
| `bad-args: <field>` | A malformed argument. It never quotes the value. |
| `denied` | `security` set before the console allowed it, or a revoke that raced a `push_register`. |
| `not-registered` | `push_test` with no registration, or Apple just said the token is dead. |
| `not-configured: this host has no APNs key yet.` | `push_test` before a key is imported. The phone shows it as written. |
| `unavailable: …` | The host cannot read its push file, or could not reach Apple. The phone shows it as written. |
| `rate` | Past the row's budget: changes 30 at once then one per 10 s, `push_test` 5 at once then one per 5 min, and `push_test` past the 20-an-hour push cap. |

## The payload

As in the design: `{aps, wt: {v: 1, k, n, c}}`, all standard padded base64.
ChaCha20-Poly1305 (RFC 8439), with `c` the ciphertext and its 16-byte tag after
it. The associated data is the ASCII `worktrees-push-v1` followed by the key
id's 8 raw bytes. The nonce is random per push.

`kind` is `input`, `reminder`, `several`, `security` or `test`. `what` is
`enrolled`, `disabled`, `refused` or `address`, and may be `rotated` or
`unreadable`, though on those two no phone is usually left to hear it. `reason`
is a sentence, shown as the notification's body. `count` appears only when more
places came due than `places` names, which is at most 5.

A push whose names cannot fit Apple's 4 KB sheds places first, and past that goes
with the fallback line alone, no `wt`. A badge-only push is `{aps: {badge}}`,
priority 5.

## Delivery

As the design's "Delivery rules (host)", and these details:

- **The badge** counts places past their 30 s, in the grant, not muted, and 0
  with `input` off. A prompt answered at the Mac inside the 30 s does not reach
  the phone even as a badge. Every push carries it.
- **A wait** is a place's unbroken stay in the waiting set behind the amber dot.
- **A first push at or past 15 minutes is also the reminder,** so a phone that
  registers late does not hear "needs input" and then "still waiting" 30 s on.
- **Past 20 an hour** only the badge moves, and what came due then does not fire
  later. Security pushes count, at most one a minute per device, and at most 8
  wait.
- **`apns-collapse-id`** is HMAC-SHA256(key, repo NUL slug), first 16 bytes in
  hex. `several`, `security`, `test` and a badge have none.
- **A failed send** (429, 5xx, no connection, a timeout) is not told: what is
  still due folds into the next try, after 30 s doubling to 15 minutes. A bad
  provider token stops every device for 5 minutes doubling to 15.

## What stops pushes

- **Remote access off.** `remote_disable` stops pushes and aborts what is in the
  air. A listener paused because the address moved does not.
- **Serve not running.** An app-owned serve stops after the app quits, so no
  pushes then. A serve started by hand stays until it is stopped.
- **The audit log unhealthy,** as it stops a device's non-read acts.
- **The push file unreadable.** Nothing is sent and `serve.log` says so. Device
  access is untouched.

A serve restart forgets every wait, so a prompt still up notifies again 30 s
later.

## Where it differs from the design

- **Storage.** Token, key, switches and the console's allowance live in
  `<config_dir>/push-devices.json` (`0600`), not the device's record:
  `DeviceRecord` refuses unknown fields, so a field there would make a downgraded
  build refuse every device. A revoke or disable drops the entry at once, and a
  boot sweep removes what a crash left.
- **The allowance is not a `SecurityFields` entry,** for the same reason. So a
  same-user edit of that `0600` file can set it without disabling the device. It
  controls only alerts about the owner's own events. It is console-only and
  owner-only, a device's `security` cannot set it, and the console's act is an
  audit record.
- **Cipher crate.** `ring`, already in the host, not RustCrypto's. Same bytes.
- **The badge,** as above: the design said "waiting places".

## The console

`worktrees serve push key-import <file.p8> --key-id ID --team-id ID`, `key-remove`,
`status`, `dry-run on|off`, `security <device-id> on|off`, each with `--config-dir`.
They are `HostConsole` rows (`push_key_import`, `push_key_remove`, `push_status`,
`push_dry_run`, `push_allow_security`). `push_status` says whether a key is there,
never which. The dry run seals and logs instead of calling Apple, until serve
restarts. The key is stored in `<config_dir>/apns-key.json`, `0600`, and no row
reads it back.

## Outbound

The two APNs hosts and nothing else, chosen by the token's `env`; TLS 1.3 against
two embedded roots (USERTrust RSA and AAA Certificate Services, not the OS store);
HTTP/2 with no client above it, so no redirect and no proxy; 10 s to connect, 15 s
to send. If Apple moves to another CA, `serve.log` says the certificate does not
chain to the embedded roots, and nothing is sent until a release updates them.

**Not yet proved against Apple.** No test here reaches it, and no real `.p8` has
been imported. The first real check is the first delivery with Bret's key.
