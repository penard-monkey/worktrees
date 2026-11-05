# 2026-11-04 — push notifications, the host's half

- **Date:** 2026-11-04
- **Place:** `push-notifications-host` (a lane briefed by `(main)`)
- **Branch:** `push-notifications-host` (squash-merged), then `push-notifications-host-closeout`
- **PR:** [#612](https://github.com/penard-monkey/worktrees/pull/612), squash `b58ede3`
- **Release:** none (under `[Unreleased]`)
- **Planning files:** `planning.tar.gz`

The worktrees-ios session asked for the host half of notifications, with
Bret's design and the phone's fixed wire details as the contract. Real delivery
needs Bret's `.p8`, so the lane built up to that and stopped. **Nothing has
reached Apple from this code, and no real `.p8` has been parsed.**

## What shipped

- **`crates/worktrees-cli/src/serve/push/`:**
  - `rows.rs`: `push_register`, `push_unregister`, `push_prefs`, `push_test` (`Own`), and the bodies behind the console rows.
  - `store.rs`: `<config_dir>/push-devices.json`, `0600`.
  - `seal.rs`, `payload.rs`: ChaCha20-Poly1305 and what each kind of push says.
  - `plan.rs`: the delivery rules, pure, on a scripted clock.
  - `apns.rs`: HTTP/2 to the two APNs hosts, the JWT, and a local fixture server.
  - `key.rs`: the `.p8` import, stored in `apns-key.json`, `0600`.
  - `mod.rs`: the sender thread and its seams.
- **Console rows:** `push_key_import`, `push_key_remove`, `push_status`, `push_dry_run`, `push_allow_security`, and `worktrees serve push …` (`client.rs`).
- **`ok` flag:** the remote `ok` carries `push: true`. `PROTO` did not move.
- **Limits and records:** `budget.rs` gains `push_changes` and `push_tests`. `audit.rs` records register, unregister and prefs changes, and the sends as counts.
- **The contract:** `docs/push-contract.md`, and `docs/push-fixtures/` (`seal-vector.json`, `rows.json`, `payloads.json`), which the tests read.

## Decisions

- **Secrets in their own file, not the device's record** ((main), overriding the brief and the lane's recommendation). `DeviceRecord` has `deny_unknown_fields`, so a field there makes a downgraded build refuse every device. It also means no listing can return a secret, by construction.
- **The security allowance is in that file, not `SecurityFields`.** Same hazard. So a same-user edit of the `0600` file can set it without disabling the device. That was accepted: it only controls alerts about the owner's own events, and the console's act is an audit record.
- **`Own` gate, Observe included, no step-up.** Identity comes from the connection, each push is cut by `scope::event` when it is sent, and Turn On must not open Face ID.
- **The kill switch stops pushes** and aborts what is in the air, `push_test` included. A listener paused because the address moved does not stop them. An unhealthy audit log does.
- **`ring`, not RustCrypto.** Same bytes, no crate. `h2` 0.4.20 is the one new crate.
- **Two embedded roots, TLS 1.3 only.** `(main)` measured both APNs hosts with a bare handshake. The OS store would let `SSL_CERT_FILE` change trust. The cost: if Apple changes CA, pushes fail until a release, and `serve.log` says why.
- **The badge counts places past their 30 s,** where the design said "waiting places": a badge at t=0 would reach the phone during the window meant to keep it quiet.
- **`topic` is one of two named ids** (review item 1). The team's key signs for every app on the team.
- **The sender only reads what the poll left** (`Clients`' last `sessions:busy`), so the poll never waits on it. A wait is a place's unbroken stay in the waiting set, kept in memory, so a serve restart can repeat a notification 30 s later.

## Dead ends / gotchas

- **The disk filled and broke the shared Podman engine.** Free disk fell to 1.4 GiB during a container run, this lane's own `target` being 23 GiB and two named volumes on top. A shell then failed with `no space left on device`, and every `docker` command after that answered `database disk image is malformed`. Likely the cause, not proved. David repairs it. Check `df` before a container run, and `cargo clean` after a heavy gate.
- **This Mac's connections to its own LAN address never reach `accept`.** A bare Python socket shows it: `connect` succeeds and `accept` times out, on plain and interface-bound listeners alike, while loopback works. So `serve::remote::tests` and `remote_bin` hang locally with no output, more than a dozen tests "running for over 60 seconds". It looks exactly like a deadlock from the change in hand. Measure it outside Rust first. They passed in a container and in CI.
- **A test can deadlock on its own assertion.** `assert_eq!(x.lock().unwrap()[0], x.lock().unwrap()[1])` holds both guards for the statement. It hung the suite for ten minutes and held a gate slot. `sample <pid>` named the test at once. Bound a local test run with a watchdog while new tests are unproved.
- **The first mutation pass found tests that passed for the wrong reason.** A mute outside the grant looked refused only because the read path filters too, so storage was never checked. Three more mutations stayed green because the same guard lived in two places, or a grant check hid the registry check. A guard that exists twice cannot be shown red until one goes.
- **A failed send must also settle its badge.** A refused push left `told_badge` unset, so the phone would have got a badge push every minute forever.
- **A no-op write created an entry.** `Store::change` compared against `Some(rec)`, so a `drop_token` on an absent device wrote a default entry after its revoke. Found while fixing review item 7.
- **`rust (macos-latest)` has no tmux,** so the end-to-end test's `tmux` call failed, then its cleanup panicked while unwinding and aborted the whole test binary. It now skips there, with a tripwire so a Linux runner without tmux cannot pass silently. On that job `ok` means skipped.
- **A green PR can still be `DIRTY`.** `main` moved twice during CI. Check `mergeStateStatus` as well as the runs.

## Verification

- **Local, on the final head `8c2e975`:** release build, bats (0 `not ok`), lint, core, the CLI's binary tests without `serve::remote::tests` (495), `serve_bin`, `plan_parity`, api, remote, app lib, tsc, every `*-check.mjs`, `cargo deny`.
- **CI on `8c2e975`:** all jobs and `deny` green. `a_real_waiting_edge_becomes_a_sealed_push_in_the_dry_run` really ran on Ubuntu, and skipped on macOS.
- **Container, at the earlier `76e01e5`:** `serve::remote` (33), `serve::push` (70), `remote_bin` (4). No container run after that.
- **End to end, real binary:** a pane shows Claude's trust prompt, serve's poll sees it, and a sealed dry-run push is logged about 30 s later.
- **Red first:** 46 mutations on the first build, then 17 more and the tmux tripwire on the review's fixes. All red once the tests above were fixed. Not red: the `allow_security` panic itself, which needs a repair landing between two reads.
- **The phone's code:** its real `PushShared.swift`, compiled in a scratch dir, opened `seal-vector.json`, sealed to the same bytes, and gave the right words for every fixture in `payloads.json`. Its own `push-crypto.sh` was not changed or run.
- **Review:** Fable found no must-fix and eight should-fix items, all fixed.
- **Not verified:** anything against Apple, a real `.p8`, and any real phone.

## Follow-ups

In `ROADMAP.md`: the leftover volumes, first real delivery, sound, the LAN failure, and `push_test`'s words in the remote-off race.

The phone lane still has to set `PUSH_SINCE`, add the vector to `push-crypto.sh` (B-14), and hear about the topic rule.

**Proposed for `AGENTS.md`, not committed (needs David's OK):**

1. The command count is now 207; the file says 194.
2. `push-devices.json` and `apns-key.json` hold secrets that no row returns, and a push field must never go in `DeviceRecord`, because a downgraded build would refuse every device.
3. A lane's Docker volumes and `target` can fill the disk and break the shared engine: check `df` before a container run, and clean up after a heavy gate.
4. On this Mac, a `serve::remote` or `remote_bin` hang may be the own-LAN-address failure: check with a bare socket before suspecting the change.
5. `.claude/close-out.md`'s gate list hangs on the same failure while it lasts, since `cargo test -p worktrees-cli` includes those tests.
