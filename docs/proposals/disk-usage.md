---
title: Disk usage of places
---

# Disk usage of places

**Built 2026-11-25** (`experiment-with-disk-inventory`). The owner's ask of
2026-11-12: "worktrees uses up a ton of disk space because we're always making
copies of the code and each lane could potentially have its own node_modules or
other files … On a project/workspace/place and home, it would be good to know
how much space we're consuming", then (2026-11-25) "we need to know how the disk
sizes work with workspaces" and "allow for deletions directly from the disk
inventory view".

## What was measured first

On the owner's Mac, 2026-11-24 and 25 (scanner prototype, then `worktrees du`):

- 88 places, 2.5 M files, scanned in about 50 s. A `du`-style sum said 131 GB;
  counting each APFS clone once, 100 GB.
- Build output by folder name was most of it: 68 GB on 11-24. On 11-25 three
  workspace-view lanes alone held 61 GB of `target/debug`.
- After those lanes were removed, free space did not move: four to twelve Time
  Machine local snapshots held the blocks.
- In one project ten places shared cloned `node_modules` for almost nothing;
  one did not and held its own 1.6 GB.

## Decisions

- **Own versus shared, by clone id and reference count.** A file's bytes are
  the place's own when every hard link and every APFS clone of it is inside the
  place. `ATTR_CMNEXT_PRIVATESIZE` was tried first and rejected: snapshots pin
  blocks, so it reads about zero for older files. On Linux only hard links
  count as shared.
- **Categories by folder name** (`target`, `.next`, `node_modules`, …): an
  estimate, labelled as one. Deleting does not trust it.
- **Deleting is git's question, not the category's.** Only a path git ignores,
  holding nothing git tracks, never `.git`, never `(main)`'s `.worktrees/`,
  never through a symlink, and only the object the scan saw (`dev:ino`).
  Permanent, because the Trash frees nothing. Untracked files that git does not
  ignore are refused: they may be someone's work.
- **Bulk cleanup offers build output only**, never dependencies (a reinstall is
  slower and can change versions), ticked by default only for places idle two
  weeks with no agent working.
- **A place with a working agent** gets "Delete when idle", held by the app
  until the place's busy dot clears.
- **Workspaces** reference projects, and one project may be in several: a
  workspace's card counts it, the treemap draws it once.
- **No scan in the 3 s poll.** A scan runs when the view asks and is cached in
  the engine until Rescan; the nav's badges show only what is cached.
- Disk Inventory X (GPL-3) was the inspiration; nothing is copied. The layout
  is the squarified treemap (Bruls, Huizing and van Wijk, 2000) and the shading
  van Wijk and van de Wetering's cushions (1999), on a canvas.

## Rows

| Row | Capability | What |
|---|---|---|
| `disk_usage` | `read`, `project(repo)` | every place's own, shared and `du` bytes and its cleanable folders; `refresh`, `cached` |
| `disk_place_tree` | `read`, `place(repo, slug)` | one place's tree, pruned to 0.15 % and 2500 nodes |
| `disk_delete` | `write`, `place(repo, slug)` | one path, by the rules above; `scanned` is the scan's `dev:ino` |

## Not done

- A diverged clone (a partial write after `cp -c`) is counted whole.
- Workspace folders (memory, chats) from the workspace view get their own row
  once that ships.
- Shared `CARGO_TARGET_DIR` or sccache per project, and cleaning a place's build
  output when it is parked, are the next ways to spend less.
