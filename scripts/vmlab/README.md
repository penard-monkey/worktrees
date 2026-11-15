# vmlab — the group chat experiment driver

Runs one group chat experiment (proposal v4.1 §12, `docs/proposals/
group-chat-v4-two-engineers.md`) on the Parallels test VMs: scripted
engineers drive the REAL app on two Macs, every screen is captured, every act
is proved against host state, and the run is scored on §12.4's rubric.

Everything goes over `prlctl` and ssh. Nothing here touches the host's own
app, tmux or config; the only process it starts on the host is `claude -p`
(for a post's wording, below), in an empty directory.

## Run one

```sh
# 1. The bundle the VMs install (config.toml's `bundle`): main's build.
make install-app APPDIR=$HOME/.cache/worktrees/parallels/build-main

# 2. One run: install, start the apps, pair, share, play the persona, collect.
python3 scripts/vmlab/run.py v1 json-flag-plan

# 3. The table.
python3 scripts/vmlab/score.py ~/.cache/worktrees/parallels/vmlab/<run>
```

Flags: `--persons 3` adds Cy (below); `--skip-install` keeps the bundle the
VMs already run (the apps are still restarted, see "Traps"); `--revert
[--snapshot off-ready]` reverts the owner VM first. **A revert needs the
owner's word**: without `--revert` nothing is reverted, and `--snapshot`
alone is refused. The guest VM is never reverted unless config.toml names a
snapshot for it: it is a full clone made AFTER `off-ready`, so its own
`off-ready` is the owner's disk (hostname, serve identity), and pairing the
two would pair a host with itself.

A run takes about ten minutes with the install; the owner VM's reboot is under a minute of it.

## What a run does

1. **Setup.** Installs the bundle in both VMs (`vm-install.sh`, run inside
   each over ssh: quit the app, copy it to /Applications and its CLI to
   `~/.local/bin`, turn on config's `beta` switches in `ui-state.json` while
   the app is quit, start it). Pins each app window to config's `window`
   frame, so the coordinates in `ui/<variant>.toml` hold. Turns on remote
   access on the owner if it is off. **Pairs** each guest the rehearsal's way:
   `worktrees serve pair --invite <name>` (or `--guest <id>` for a name the
   host knows) in a `tmux -L vmlab` session on the owner, the link read from
   its output and handed to the guest on ssh's stdin (it is a secret: a 0600
   file in the VM, removed after, never a log line), the comparison codes
   read on both sides and compared, the owner answering `y`. The guest pairs
   into its APP's config dir, so its app sees the host. A pairing that still
   holds (same host pin, device not revoked) is reused. Then the lane:
   earlier shares on it stopped, open requests dismissed, the task's files
   removed, the agent restarted with one warm-up turn (a share needs a session
   that has named itself), and the share created with every guest.
2. **The persona** (`personas/<variant>.yaml`): intents in order, each
   `{who, act, args, wait_s}`. After every intent the driver captures both
   screens, reads what the owner's UI should be surfacing (guests waiting to
   be let in, requests for another place), hashes the task's watched files,
   and snapshots the thread and the shared turns.
3. **Captures** every `capture_every_s` (5 s) of every VM, beside the
   per-intent ones, as half-size JPEGs in `shots/` with `shots/index.jsonl`.
4. **Collect**: both VMs' `~/.cache/worktrees/serve.log` and `audit.jsonl`
   from the run's start, the thread and the turns as last seen (a stop
   deletes the thread, so they are snapshotted before it), the driver's log.

## The files

| File | What it is |
|---|---|
| `run.py` | The orchestrator: setup, the persona, captures, collection |
| `acts.py` | One function per act, each proved on host state |
| `vm.py` | One VM: ssh, `prlctl capture`, cliclick clicks, keys, typing (counted) |
| `vm-install.sh` | Runs inside a VM: install the bundle, set the beta switches, start the app |
| `score.py` | §12.4's table from a run directory |
| `config.toml` | The VMs, the bundle, the lane, the cadence, the window frame |
| `personas/<variant>.yaml` | The intents. YAML in its JSON subset, so the stdlib reads it |
| `ui/<variant>.toml` | Where each control is, in points, per variant |

### Acts

| Act | Who | How (V1) | Proved by |
|---|---|---|---|
| `open_share` | guest | clicks the shared lane under "<owner>'s places" (CLI `share_join` for a person with no screen) | the guest is `asked` or `allowed` in `shares_all` |
| `allow` | owner | the share strip's Allow | the guest is `allowed` |
| `open_thread` | owner | the strip's Thread toggle | nothing on the host: the next `use_these` |
| `send_task` | owner | CLI `agent_send` (typing to one's own agent is not what is measured) | a new turn in `shared_turns` |
| `post` | guest | composer, typed, Return (CLI `thread_post` with no screen) | a post by that guest in `thread_read`, text compared |
| `use_these` | owner | Use these…, then keys in the preview (its note field opens focused): ⌘A, Delete, the note, Return; with no note, "Send without a note" | a `shared_with_agent` event, then a new turn; a send that instead ENDS the binding is flagged `sent_to_unshared` |
| `ask_share` | guest | a click on empty nav (a click while the composer has focus only blurs it), Ask beside a place, the note, the form's send | a new entry in `share_requests_list` |
| `end_session` | owner | CLI: close and reopen the lane, one turn | the binding is `ended` |
| `share_again` | owner | Share again, confirm | the binding is `active`, the old one in `ended` |
| `reboot_owner` | owner | `prlctl restart` | serve answers with the share kept; each guest reads the thread again |
| `stop_sharing` | owner | Sharing…, Stop sharing…, confirm | the share is gone from `shares_all` |
| `wait` | anyone | `{s}` seconds, or `{until: agent_idle}` (no `esc to interrupt` in the lane's pane, twice) | — |

An intent with `impossible: "<why>"` is not acted: it is the variant's blind
spot, recorded as such (V1's "read the diff").

A post's `text` is literal or `{generate, fallback}`: `claude -p` (with
`DISABLE_AUTOUPDATER=1`, never its own `XDG_DATA_HOME`, in an empty
directory) writes one sentence from what THAT guest can read, fetched with
their own `worktrees remote call … shared_turns` and `thread_read`, so the
scripted person never sees more than the real one would. The fallback is
used when Claude fails.

### Three persons

`--persons 3` adds Cy on the third VM in config.toml's `[third]`: the Debian 13
arm64 VM with the AppImage, his own screen (§12.2). He is Dana's second guest
AND a host: `[third.host]` is his lane, shared with Dana. An intent's `on`
names the share it acts on (`dana`, the default, or `cy`); the acts are the
same, bound to that share's host, lane and guests (`Acts(run, ctx)`).

A VM's `os` picks its backend. `macos`: osascript and cliclick, coordinates
in points. `linux`: xdotool inside the logged-in GNOME session through
`lin-session.sh` (copied to `~/sess.sh`; an ssh login has no display, so it
borrows DISPLAY, WAYLAND_DISPLAY, the D-Bus address and mutter's XAUTHORITY from
`gsd-xsettings`), coordinates in pixels (a capture is 1:1), the app maximized
and started by `lin-launch-app.sh`. Its config dir is
`~/.config/net.casadelvalle.worktrees`. Clipboard checks need `xclip`.

UI files name one section per person and role: `<person>_host` (their own
share) and `<person>_guest` (reading someone else's). `ui/v1.toml`'s
`[owner]`/`[guest]` still work as Dana's and Bret's.

### The release test (`--release`, `--from-ui`, segments)

- `--release vX.Y.Z` installs a PUBLISHED release in every VM the way a user
  does (`vm-install-release.sh`: quit the app, `curl …/<tag>/install.sh |
  WORKTREES_INSTALL_APP=1 bash`, start it) instead of a local bundle.
- `--from-ui` starts from pairings and shares people made in the apps (the
  first run is what a release test measures, and it is driven by hand: see the
  persona's `first_run`): each share is read back by its lane, each guest's
  principal by name, each guest's host id by pin and live device. Only the
  first segment touches the screens (Dana back on her lane).
- `--intents A:B` plays a range, `--continue <run dir>` appends it to that
  run's record. A run measured control by control goes in segments: measure
  the next segment's buttons on the screens as they are, then play it. The
  apps' UI state the acts rely on (the thread panel open, a send done) is
  carried in the record between segments. Time-based scores of a segmented run
  are upper bounds.

## The record

`<runs>/<run>/record.json`:

- `setup`: what was installed (versions), remote access, each pairing
  (reused or new, host and principal ids — never the link), the lane, the share.
- `intents[]`: per intent `who`, `act`, `args`, `t_start`/`t_end`, `proved`
  (`proved` / `unproved` / `n/a`) and its `proof` in words, `inputs` (clicks,
  keys and typed strings per VM, from `vm.py`'s counters), `shots`,
  `pending` (what the owner's UI should be surfacing after it), `files`
  (the watched files' hashes), and the act's own facts (a post's `seq`, its
  text and the text that landed; a send's `first`/`last` and note; a reboot's
  `recovery`).
- `errors`: an act's exception, or what stopped the run.

Beside it: `driver.log`, `shots/`, `host/{owner,guest}-serve.log`,
`host/{owner,guest}-audit.jsonl`, `host/thread.json`, `host/turns.json`,
`host/share.json`.

## The scores (§12.4)

| Measure | How it is computed |
|---|---|
| Comment to agent action | Per guest post: seconds to the `shared_with_agent` event that carried it, to the start of the agent's next turn, and to its last step. Median over posts that reached a turn |
| Notes used | Use these sends with a note, and whether a watched file's hash changed before the next send (with a note, and without) |
| Blind spots | Intents marked `impossible` |
| Lost attention | A pending item (a guest waiting to be let in, a request for another place) still pending two intents after it appeared. Toasts themselves are not read (no OCR): these are the host's items the toasts exist to surface |
| Acts per outcome | Clicks, keys and typed strings per act, from the driver's counters |
| Wrong place | Acts that landed on something other than what they meant to touch |
| Errors | `[warn]`/`[error]` lines in both serve logs, refusals in both audit logs, unproved acts; and any text the thread changed on the way (smart dashes) |
| Recovery | The reboot: seconds to the owner's serve answering with the share kept, and to each guest reading the thread again |

## Traps

- **A Linux display that powered off freezes the captures.** GNOME on the
  Debian VM sets Mutter's PowerSaveMode to 3 even with idle-delay 0, and
  `prlctl capture` then returns the LAST frame: an old clock, clicks that seem
  to do nothing. `VM.wake()` sets it back before every Linux capture and click.
- **XWayland's window geometry is not the screen's.** `xdotool
  getwindowgeometry` reports the app 37 px lower than it paints; clicks by
  SCREEN pixels (a capture) land right, window-relative ones do not.
- **`sips --cropOffset` is y then x.** A crop to read a button's position with
  the offsets swapped places the next click outside a dialog, which cancels a
  pairing window.
- **Positions move with names and state**: the share strip's buttons follow
  "Shared with Bret and Cy", a pending ask adds "Bret asks to join · Allow", a
  second earlier session adds a line to the Sharing… dialog, and a guest's
  sidebar gains a group per kept pairing (B3). Measure in the state the act
  will meet.
- **A guest let in before the click proves nothing**: `open_share` says n/a
  for a guest already allowed.

- **Coordinates are points**: a capture is 2730 px wide, the screen 1365 pt.
  They hold only for the pinned window frame, and the owner's lane row only
  while the lane is the one place under ACTIVE.
- **The thread panel's open state is in the app's memory**, per place, and
  survives a share stop. Clicking Thread on a panel already open closes it,
  so every run restarts the apps first.
- **The guest's "let you in" toast covers Post** for its first seconds: a
  post is sent with Return.
- **"Use these…" moves** once the app has sent once ("Sent to the agent."
  under it): `use_these_after`. The preview's own layout moves with its
  content, which is why it is driven by keys.
- **After a host reboot the share still reads "active"** while the agent is a
  new session with no transcript yet; the first send goes to that unshared
  session and ends the binding. The persona sends once after the reboot so
  every run measures it.
- **A guest's pairing dial must run inside a live ssh session.** Backgrounded
  in the VM after its ssh session closed, `worktrees remote pair` got "No
  route to host" (os error 65) from the window's port while `nc` reached it;
  inside a session the host keeps open it pairs. `pair()` holds that session.
- **Codex opens on "Update available"** when npm has a newer one; the driver
  presses `3` (skip until next version) only when that list shows.
- **macOS notifications** land over the top right of the screen. Each shows
  its × at its top-left only while hovered, so the driver hovers and clicks
  (`[macos]` in the UI file) at setup and after the reboot. `killall
  NotificationCenter` does not clear them: they come back.
