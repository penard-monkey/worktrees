# Sync hubs: manual checks

Run these by hand, in the real app, with real drives and the SMB share. No
suite reaches a real `/Volumes`: the tests read a fake volume list
(`WORKTREES_SYNC_VOLUMES`), so these checks are the only proof on real
hardware. Before you start, push one throwaway project you don't mind
mirroring.

## Identity and kind

1. With Extreme SSD, SanDisk and the SMB share all mounted, run
   `worktrees sync hubs`. Each drive must say `removable`, and the share must
   say `network`. `/Volumes/Macintosh HD` and `.timemachine` must not be
   listed.
2. Run `worktrees sync hubs --json`. The drives' ids are `uuid:…`, matching
   `diskutil info <vol> | grep "Volume UUID"`. The share's id is
   `smb://<host>/<share>`, with no user name in it.
3. Rename SanDisk in Finder, then run `sync hubs` again. Same id, new name.
   Then eject it and plug it back in: still the same id.

## Choosing, and never swapping

4. With nothing chosen and two drives plugged in, `worktrees sync push`
   fails with "multiple removable drives … sync hubs --choose". With one
   drive and the share, it auto-picks the drive and never the share.
5. In the app, open Hubs… and tick Extreme SSD. Then eject it and push from
   the ⇄ popover. The popover and the dialog say "'Extreme SSD' is not
   connected". Nothing goes to SanDisk.
6. Tick the SMB share and press "make primary". `worktrees sync status`
   (CLI) names the share as the hub.

## Dashboard

7. Hubs… opens from Home (even with no projects registered), from the
   add-project menu, and from "All hubs…" in the ⇄ popover.
8. Each project row shows an age and a host for every hub it is on. A project
   with uncommitted changes shows "· N dirty · ● changed". An ejected hub that
   you had chosen stays listed, dimmed, with its last-known ages. "forget"
   removes it from the list and does not touch the drive.
9. A project that exists only on a hub shows "import" in its row. Clicking it
   opens the import dialog with the destination shown.

## Many at once

10. Select two projects and press Push…. Both chosen hubs are ticked. The
    preview shows one row per project × hub, with sends and deletes. Untick
    a hub and the preview re-asks. Confirm: the rows run ONE AT A TIME, each
    with its own bar. Check Activity Monitor or `ps` for a single `rsync` at
    a time.
11. Pull one project that sits on both a drive and the share. The source
    menu lists both, newest first, labelled "(newest)", and the newest is
    picked by default.
12. Unmount the share in the middle of a batch push. That row fails with
    rsync's error, the next row still runs, and the summary says "1 of N
    transfers failed".
13. In the single push dialog, ticking a second hub switches to the combined
    preview with both hubs ticked.
