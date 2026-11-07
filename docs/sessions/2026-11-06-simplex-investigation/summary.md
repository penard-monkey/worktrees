# 2026-11-06 — SimpleX as a transport for worktrees communications (investigation)

- **Date:** 2026-11-04 to 2026-11-06
- **Place:** `simplex-investigation` (a lane briefed by `(main)`)
- **Branch:** `simplex-investigation` (never a PR, by the brief), then `simplex-investigation-closeout`
- **PR:** the close-out PR carries the proposal and this archive
- **Release:** none (docs only)
- **Planning files:** `planning.tar.gz`
- **Artifact:** https://claude.ai/artifact/AUi8u3pqYoZZDPc9ZkmLVG (private)

David asked whether SimpleX (simplex.chat) could carry worktrees
communications: the group chat channel, `@bret` direct messages, and what
that would mean for the iOS app. The brief said the deliverable is an
analysis, not code, and it is. Nothing SimpleX was installed, run or
contacted. David's answer on 2026-11-06: the worktrees mobile app will be
the communication path for now; parked, may come back later.

## What shipped

- **`docs/proposals/simplex-transport.md`:** the eight questions of the brief
  answered against `group-chat.md` §5.4 (T1–T8 and the six places). Every
  claim marked read (with a link and the revision's date), measured, or
  assumed. Leads with the verdict and seven decisions S1–S7, each with a
  pick; ends with what could not be established and a source list.
- **The artifact** (private): the same verdict and decisions with four inline
  SVG figures: two Macs with relays and trust boundaries (existing solid,
  new dashed, third-party dotted); the three integration shapes with the
  AGPL process boundary; the `@bret` sequence while Bret's app is closed and
  his phone locked, the D6 and APNs path against the bridge path; and what
  stays on which path.
- **This archive.**

## Decisions

- **Not the channel's transport; keep D6** (serve-to-serve over Tailscale).
  SimpleX gives neither one order (T2) nor named membership (T4), and for two
  people already on a tailnet its relays add a third party and an 80–170 MB
  sidecar for a leg Tailscale already carries.
- **Never link the Haskell core.** Everything SimpleX is AGPL-3.0 (the core,
  the CLI, the prebuilt `libsimplex`, the Node and Python packages). This
  project is MIT and ships binaries; `deny.toml` admits no copyleft beyond
  file-scoped MPL. The one licence-clean shape is the git/tmux one: a binary
  the user installs, never bundled, spoken to over its WebSocket API. Not
  legal advice; the process-boundary reading is the common one.
- **Nothing of SimpleX in the iOS app.** Instant push needs a SimpleX
  notification server that calls APNs with its own key, and the reference
  server hardcodes SimpleX's app id and team id, so their servers push only
  to their app. Ours would need the AGPL core linked, a Haskell runtime in
  the 24 MB Notification Service Extension, and a public notification
  server of ours, which T7 refuses.
- **`@bret` goes through Bret's own host**, as `notifications.md` already
  designs it. SimpleX's app could be a second, optional notifier via a
  bridge, and only later.
- **The one experiment, if ever:** a one-way "mentions to my SimpleX" bridge
  through the user's own `simplex-chat -p`, two Macs, preset relays, two
  days, after David's OK. Kill criteria and five measurements are in §8.
- **Preset operators only, never an SMP server of ours.** A server of ours is
  a public bind, refused twice already.
- **Identity rule if any SimpleX line ever reaches a host:** a contact maps
  to a principal only by the owner, at the console, by accepting a one-time
  invitation the owner issued. A display name resolves nothing; unknown
  members, group links and contact addresses are refused. SimpleX names are
  local labels and content is deniable by design.
- **Close-out carries the proposal.** The brief forbade a PR for the lane's
  delivery; David's close-out instruction is what lands it on `main`.

## Dead ends / gotchas

- **The App Store page is an empty shell to `curl`**, and the iTunes lookup
  by app id returned nothing. `lookup?bundleId=chat.simplex.app` answered
  (track id 1605771084, 217 131 008 bytes).
- **`_defaultNtfServers` is not in `Chat.hs`.** Found with `gh api
  search/code` in `src/Simplex/Chat/Library/Commands.hs` (ntf3 and ntf4).
- **`simplex-chat-libs` has no tag matching the chat version**; list its
  releases (core-7.1.0.18 on 2026-11-03) instead of guessing a tag.
- **`apps/ios/Libraries` and `nodejs/scripts` are not in git** (404). The
  iOS libraries come from their nix build; the Node package downloads
  `libsimplex` at install time.
- **BSD `sed`'s `0,/re/` form is GNU-only**; a status line in `task_plan.md`
  did not update until a Python replace. A Python one-liner with escaped
  quotes inside an f-string also failed; use a heredoc script.
- **The June 2026 Trail of Bits assessment could not be confirmed.**
  `SECURITY.md` (2026-05-25) says "scheduled"; the blog index as of
  2026-11-04 has no post about it.
- **Commits here are unsigned**: `commit.gpgsign` is not configured in this
  checkout, and `-c commit.gpgsign=false` was used for the proposal commit.

## Verification

- **Read:** SMP v20, overview rev 4, security rev 1, agent protocol v7, push
  notifications v3, channels overview rev 1, CHAT-RELAY, SECURITY.md,
  TRANSPARENCY, CLI.md, the bot API commands, `Push/APNS.hs`,
  `Presets.hs`, `Mobile.hs`, `SimpleX.h`, the iOS README, every `LICENSE`
  named; `deny.toml` here and in worktrees-ios; `notifications.md`,
  `conn.ts`, `messages.rs`, `group-chat.md` §5.4.
- **Measured:** release asset sizes for v7.0.3 (CLI 80–173 MB by platform,
  .deb 13 MB), the Android APK, the App Store binary, crates.io for a Rust
  client (none official; third-party `simploxide-*`, `libsimplex-rs`).
- **Not done, by design:** no SimpleX software run, no account created, no
  relay contacted. Latency, offline delivery and the sidecar's footprint are
  therefore the docs' claims, not numbers.
- **Report to `(main)`:** sent 2026-11-05 with the link, the path and the
  decisions; David answered on 2026-11-06 with the close-out.

## Follow-ups

In `ROADMAP.md`: the parked bridge experiment, with what would make it worth
running.
