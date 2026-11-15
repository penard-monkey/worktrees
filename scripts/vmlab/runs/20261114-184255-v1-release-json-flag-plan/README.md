# The v0.52.0 release on three VMs, three people (2026-11-14)

The owner's ask: run the group chat test again **on the real release**, ideally with three devices, and turn
every issue into a fix. This directory is the scored run (`record.json`, `score.md`); the screenshots stay in
`~/.cache/worktrees/parallels/vmlab/20261114-184255-v1-release-json-flag-plan/shots/` and the setup's in
`~/.cache/worktrees/parallels/vmlab/release3-setup/` (the file names below are from those two folders).

## Who and what

| Person | Machine | App | Role |
|---|---|---|---|
| Dana | `macOS` VM (10.211.55.15) | v0.52.0 `.app` | owner of `gc-lane` (Codex); Cy's guest |
| Bret | `macOS-guest` VM (10.211.55.16) | v0.52.0 `.app` | Dana's guest |
| Cy | Debian 13 arm64 VM (10.211.55.17) | v0.52.0 AppImage | Dana's second guest **and** a host: `cy-lane` (Claude 2.1.325) shared with Dana |

Installed the way a user does, from the published release: quit the app, `curl …/v0.52.0/install.sh |
WORKTREES_INSTALL_APP=1 bash` (`vm-install-release.sh`). Every machine then reported 0.52.0 for the CLI, the app
bundle and serve.log's `startup v0.52.0` line. The group chat beta was on in all three `ui-state.json` (the Macs
from earlier runs, Debian switched on in Settings → General → Beta features, `1-beta-on-Debian.png`).

## The first run, by hand (what the release ships)

Reset on Dana's machine first (Settings → Remote → Guests → Remove on every guest: B1 below). Then, all in the apps:

1. **Dana shares with Bret**: `gc-lane` → *Share with…* → *Someone new* "Bret" → *Invite them* → (B1 detour:
   "A guest named Bret exists" → *Pair another device for Bret*) → *Copy* → Bret: Home → *Join with a link…*,
   paste, device name, *Join* → codes `480 424` on both → *Allow* → *Share with Bret*. 7 owner acts, 6 guest acts.
2. **Dana adds Cy**: no way to add a second person to a shared lane (B4). Paired through Settings → Guests →
   *Invite a guest* (Cy: *Join with a link…* on Linux, codes `319 917`, *Allow*); Cy's *Ask* on `gc-lane` brought a
   corner toast with only *Dismiss*; Cy was let in with `share_allow` from the CLI, the one stand-in of the run.
3. **Cy shares with Dana**: `cy-lane` → *Share with…* → *Someone new* "Dana" → *Invite them* → remote access turned
   on **inline** ("Turn on and show the link", 10.211.55.17, this network only) → *Copy* (works on Linux) → Dana:
   *Join with a link…* → codes `982 253` → *Allow* → *Share with Dana*. Works; B5 shows up here.

## The scored run

`run.py v1-release json-flag-plan --persons 3 --from-ui` adopted those pairings and shares and played
`personas/v1-release.yaml` (41 intents) **in segments** (`--intents a:b --continue`), measuring the next
segment's controls on screen before running it. The segments are why every time-based measure is inflated (posts
wait for the next segment: median comment-to-action 140 s here, 20 s in the two-person run of 160404) — read the
times as an upper bound, not a comparison.

Everything the release is meant to do worked across the three machines:

- Bret's ask → Dana's *Allow* → Bret in; Dana's thread shows "Cy joined", "Bret joined".
- Posts from **both** guests, one from the Linux app, landed intact (`--json and --no-color` survived the composer:
  #669 holds), and Dana's *Use these* sent posts 3..5 **from Bret and Cy together** in one file.
- Without a note the agent read and changed nothing; with a note it folded Cy's edge case into the plan file
  (the watched file's hash changed after 2 of 3 noted sends, 0 of 1 without).
- Cy's leg: Dana asked to join `cy-lane`, Cy's *Allow* on Linux, Dana posted in Cy's thread, Cy's *Use these* sent
  it to his Claude, 13 s from her post to his agent's turn.
- Bret's *Ask* for `(main)` reached Dana's corner; Dismiss answered it. End session → binding ended; *Share again*
  bound the new session, the old one kept as ended.
- **Owner reboot**: Dana's serve answered in 29 s with the share kept and the binding still active, the app
  reopened by itself, and Bret and Cy read the thread again at 61 s.
- *Stop sharing* on both hosts.

## Bugs (each its own fix PR off main)

| # | Bug | Evidence | Fix |
|---|---|---|---|
| B1 | **Remove keeps a guest forever and their name taken.** Settings → Guests → Remove revokes every device but the card stays ("bret · 0 shares · removed …"); a second Remove answers ok and does nothing (audit seq 7990-7992); *Share with…* then says "Nobody is paired with this computer yet." AND "A guest named Bret exists (0 devices)". | `2-guests-after2.png`, `3-invite-bret.png` | `fix/removed-guest-lingers` (#678) |
| B2 | **A removed guest's app says "offline" forever.** Bret's sidebar keeps "DAVID'S PLACES · offline" with *Ask* buttons and redials on a timer, while its app.log has "the host closed the connection: revoked" and "the host refused this device's key (AccessDenied)". `guestdoor.ts` has no such state. | `2-guest-nav.png` | `fix/guest-removed-state` (#677) |
| B3 | **Re-pairing the same host keeps the dead pairing beside the new one.** Two groups in Bret's sidebar ("DAVID'S PLACES", later "MACOS.SHARED'S PLACES · offline"); the dead one keeps dialling, and the HOST gets "Bret's macos-guest tried to connect and was refused: it is revoked." again on every episode and after a reboot. Stacked, that alert covered *Use these…* and the click opened Settings → Remote (intent 14, wrong place). `HostStore::add` never replaces a record with the same pin. | `3-guest-joined.png`, `i00-open_share-guest.jpg`, `i14-use_these-owner.jpg`, `host/owner-audit.jsonl` (`auth`, `revoked`) | `fix/repair-replaces-host` |
| B4 | **A shared lane cannot take a second guest.** *Sharing…* lists guests, Remove and Stop only; a paired guest's *Ask* for that lane gets "It is shared already: Sharing… on its lane adds a guest." with only *Dismiss*. The host has `share_allow`. | `4-sharing-dialog.png`, `4c-owner-request.png` | `fix/share-add-guest` |
| B5 | **An unnamed host is "owner".** Cy never set "Your name, as guests see it" (the first-share flow never asks), so Dana reads "OWNER'S PLACES", "owner's cy-lane · only owner talks to the agent", and Cy's agent is told "the owner shares 1 message from Dana … not the owner's instructions". `PrincipalRecord::owner` is named `"owner"`; `guest.ts`'s fallback never fires. | `5-owner-sees-cy.png`, `i16-open_share-owner.jpg`, `i23-use_these-third.jpg` | `fix/unnamed-host` |
| B6 | **Claude's usage is "unavailable" on Linux.** Claude is logged in on Debian, but `claude_usage` only asks the macOS Keychain (`/usr/bin/security`): "keychain: no Claude Code credentials" every 3 min in serve.log. Linux keeps `~/.claude/.credentials.json`. | `host/third-serve.log` | `fix/claude-usage-linux` |

## Findings that are not bugs (recorded, no PR)

- **After a host reboot the lane's Codex is a new session.** *Use these* refuses ("the lane's agent has not named
  its session yet: send it something first. Nothing was sent", #670's guard, `i35-use_these-owner.jpg`). Doing what
  it says sends to the unshared session and ENDS the binding (`share_check`: "Share again first"). The guard's words
  could say that a new session ends the share and *Share again* binds it.
- The inline *Use these* preview does not close on Esc (a modal does); a click elsewhere left it open across a
  retry.
- Small words: Linux Settings says ⌘E and ⌘, and "found in Applications"; the share dialog says "pair their Mac"
  on Linux; the pairing error says "Tap Deny" on a desktop; macOS capitalised a typed device name ("Bret-mac").
- A click outside the comparison-code dialog cancels the pairing window without asking (a harness misclick did it).

## Driver defects found and fixed during the run

Each is a note in `record.json` at its intent; the intent was replayed.

- Debian's display powers off (Mutter `PowerSaveMode` 3, even with idle-delay 0) and `prlctl capture` then returns
  the last frame: a frozen clock, clicks that "do nothing". `VM.wake()` before every Linux capture and click.
- Segments forgot what the apps show: the thread panel's open state is now carried in the record (`ui_state`), and
  only the first segment navigates Dana to her lane (a later one took her off Cy's share and her post landed in her
  own thread, intent 22).
- `open_share` "proved" a guest who was let in before the click; it now says n/a.
