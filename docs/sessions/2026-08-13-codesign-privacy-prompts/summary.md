---
title: "macOS privacy prompts and what signing them would cost"
---

# macOS privacy prompts and what signing them would cost

- **Date:** 2026-08-12 → 2026-08-13
- **Worktree:** `bug-fixes` (`.worktrees/bug-fixes`)
- **Branches:** `bug-fixes-codesign-local-installs` (parked, pushed, NOT merged),
  `bug-fixes-close-out-codesign` (this archive)
- **PRs:** [#124](https://github.com/penard-monkey/worktrees/pull/124) — **closed, not merged**
- **Release tag:** none — nothing shipped
- **Planning files:** none (investigation ran inline)

## What this session was

A bug report with no bug in it. Every build of an unrelated monorepo, run in a
place's terminal, raised the macOS dialog *"worktrees would like to access data
from other apps"* — two or three times per build, forever, no matter how often
it was allowed. The diagnosis was quick; the fix turned out to be a distribution
question, so the code is parked rather than merged and this archive is mostly
the reasoning.

**Nothing shipped.** One commit exists on a pushed branch behind a closed PR.

## The diagnosis

The dialog is TCC's App Data check (`kTCCServiceSystemPolicyAppData`). TCC
records an approval against the requesting binary's **designated requirement**,
and everything this repo produces is ad-hoc (linker-signed):

```
$ codesign -d -r- /Applications/worktrees.app
# designated => cdhash H"09f58b26727cb124bc33b65f2cbbdad50bf9ca48"
```

The requirement *is* the content hash. Every rebuild is a new app as far as TCC
is concerned, so every prior approval is void. Two builds of the *same version*,
each ad-hoc signed, proved it:

| build | designated requirement |
|---|---|
| `target/release/worktrees` (0.12.1) | `cdhash H"ec7b74367b136cb43faf95031caf7513503165d3"` |
| `~/.local/bin/worktrees` (0.12.1) | `cdhash H"e9ec77150f24288915c4ac2f6ed82427273111a3"` |

Two further facts explain the *shape* of the annoyance:

- The grant is recorded **per target app's data directory**, which is why one
  build raises the prompt three times rather than once.
- Signing with any cert-backed identity — an Apple Development cert, or a
  self-signed Code Signing cert from Keychain Access — changes the requirement to
  `identifier "net.casadelvalle.worktrees" and anchor apple generic and
  certificate leaf[…]`, which the *next* build still satisfies. TCC wants a
  **stable** identity, not a trusted one.

## Decisions

- **Ad-hoc re-signing is not a fix, even though the internet says it is.**
  [radarr.video](https://radarr.video/#downloads-macos) tells macOS users to run
  `codesign --force --deep -s - /Applications/Radarr.app && xattr -rd
  com.apple.quarantine …`. That is **Gatekeeper repair** — it makes an app with
  a broken/absent signature launchable — and has nothing to do with TCC
  persistence. It appears to work for their users because they sign once per
  download and never rebuild: bytes frozen, cdhash frozen, grants last until the
  next upgrade. We already do everything that command does (`install.sh` strips
  quarantine, local builds are never quarantined, tauri's output is ad-hoc), so
  running it here is a no-op.
- **The local fix is parked, not abandoned.** `SIGN_ID` / `WORKTREES_SIGN_ID` on
  `bug-fixes-codesign-local-installs` works and is gated, but it fixes one
  machine. The general form of the same fix is a Developer ID signature on
  releases, so merging the local one first would have meant shipping half a
  decision. See ROADMAP.
- **The Mac App Store is not a route for this app.** Written up in ROADMAP with
  the specifics; the short version is that MAS requires App Sandbox, and a
  sandboxed process may only exec binaries inside its own bundle. Shelling out to
  `git` and `tmux` is this repo's stated architecture, and the shared tmux server
  — the thing that makes the CLI and the app two views of one place — cannot
  survive a container. The store would "fix" the prompt by making the access
  impossible.
- **Developer ID + notarization is the tier that fits** (already ROADMAP'd, now
  expanded). Same $99/yr as MAS, no sandbox, no review, keeps the updater, drops
  the quarantine dance — *and* gives every user a stable requirement, which is
  the general form of the parked fix.

## Dead ends / gotchas

- **Full Disk Access did not silence the prompt, and that was not a bug.** Two
  processes were still holding the pre-signing identity:
  - the **running app** (`PID 22515`, started two days earlier) — a process's
    code identity is fixed at exec, so re-signing on disk does nothing to it;
  - the **tmux server** (`PID 11677`, started three days earlier, reparented to
    launchd) — responsible-process attribution is inherited at spawn and outlives
    reparenting, so every build in every session was still being attributed to
    whatever ad-hoc binary started that server.

  A correct signing fix therefore looks exactly like a failed one until both
  restart. `ps -o lstart` on the tmux server is the check. This is now a
  CLAUDE.md rule.
- **The TCC database is not readable for diagnosis.** `sqlite3
  ~/Library/Application Support/com.apple.TCC/TCC.db` fails with `unable to open
  database file` unless the *terminal* has Full Disk Access — which the agent's
  shell does not. `log show --predicate 'subsystem == "com.apple.TCC"'` and a
  `process == "tccd"` variant both returned nothing useful (redacted). Everything
  above was established with `codesign -d -r-` instead, which needs no privileges.
- **An Apple Development cert is not a distribution cert.** It is fine for making
  TCC grants stick locally, but a bundle signed with it is still rejected by
  Gatekeeper on anyone else's Mac. Distribution needs Developer ID + notarization
  — a different cert type from the same $99/yr membership.
- **`TAURI_SIGNING_PRIVATE_KEY` is not codesigning.** It is the minisign key for
  updater artifacts. release.yml's app bundles are described as "signed" in the
  workflow's own log lines, which reads as codesigned and is not.

## Verification

Everything below ran against the parked branch (base `ebc1cbf`):

| check | result |
|---|---|
| `make install-copy` with `SIGN_ID` into a temp `BINDIR` | `signed:` + cert-backed DR on the copy |
| same without `SIGN_ID` | ad-hoc note, install still succeeds |
| `make install-app SIGN_ID="No Such Identity"` | fails **before** `tauri build`, lists valid identities |
| `sign_if_asked` (install.sh) unset / set / bogus | no-op · signs · warns and leaves a working install |
| `make test` | 288 bats, 0 failing |
| `make lint` | clean |
| `cargo test -p worktrees-core` / `-p worktrees-cli` | 208 / 6 passing |

The archive PR itself is documentation only; no gates apply to it.

## Follow-ups

All three are in ROADMAP.md:

1. Developer ID + notarization (existing entry, now carrying the MAS evaluation).
2. The parked local-signing branch — what it does and how to resurrect it.
3. `install.sh` minting a per-machine self-signed cert, so users without any
   Apple membership get grants that survive upgrades.
