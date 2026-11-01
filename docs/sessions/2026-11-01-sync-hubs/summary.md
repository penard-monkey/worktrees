# 2026-11-01 — sync hubs

- **Date:** 2026-11-01
- **Place:** `sync-hubs` (a lane briefed by `(main)`)
- **Branch:** `sync-hubs` (squash-merged), then `sync-hubs-close-out`
- **PR:** [#587](https://github.com/penard-monkey/worktrees/pull/587), squash `36400bb`
- **Release:** none (under `[Unreleased]`)
- **Planning files:** `planning.tar.gz` beside this summary (`task_plan.md`, `progress.md`)

David's ask: a hub selector when several drives are plugged in, a dashboard
showing what each hub holds and how long ago it was synced, syncing many
projects at once, pushing to several hubs ("multiple backups" means one copy
per hub, not dated versions), and networked computers. His decision: mounted
shares now, SSH as a design only.

## What shipped

- **`crates/worktrees-core/src/synchub.rs` (new).** Hubs as a classified list.
  - **Kind**, from one `statfs` per `/Volumes` entry:
    - removable = `MNT_REMOVABLE`;
    - network = `smbfs`/`afpfs`/`nfs`/`webdav`;
    - internal = any other local mount.
  - **Never a hub:** an entry that is not a real mount point, the root
    filesystem, a `MNT_DONTBROWSE` mount (Time Machine), a symlink.
  - **Identity:** `getattrlist(ATTR_VOL_UUID)` → `uuid:<UUID>` for a disk.
    For a share, `f_mntfromname` with the user stripped → `smb://host/share`.
  - **The chosen/remembered file:** `$XDG_CONFIG_HOME/worktrees/sync-hubs.json`,
    written only through `fsx::write_atomic` and `edit_at`'s lock. It holds the
    ordered `chosen` list (the first entry is the primary) and the remembered
    hubs with their last-known projects.
  - **Rules:** `auto_pick` takes exactly one removable volume and never a
    share. `primary` names a chosen hub that is unplugged and never swaps it.
    `path_for_id` resolves only connected or configured hubs.
  - **The dashboard:** `dashboard_in` lists each hub with what it holds. Each
    hub's listing has a 3 s deadline (`READ_DEADLINE`); past it the hub shows
    its last-known list, flagged `slow`.
- **`sync.rs`.** `HUB_DENYLIST`, with David's username in it, is gone.
  - `resolve_hub`: flag > env > `[sync] hub` > chosen > one removable.
  - `local_state`: the one helper for branch, dirty count and HEAD time.
  - `hub_projects`.
  - `SyncRequest.cfg`: a seam so tests never read `config.toml`.
  - `worktrees sync hubs [--choose]… [--clear] [--json]`.
- **`crates/worktrees-api/src/project.rs`.**
  - `sync_hubs {lite}`, `sync_hub_choose`, `sync_hub_forget`.
  - An optional `hub` id on `sync_preview`/`sync_apply`. A paired device may
    name only a chosen hub (`device_may_use`).
  - `sync_batch_preview`/`sync_batch_apply` (HostConsole): sequential under a
    process-wide `SYNC_RUN` lock. Each job runs `apply_one`, the single sync's
    body. The serve table is 191 rows (`dispatch.rs`, `AGENTS.md`).
- **App.**
  - `app/src/hubs.ts`: the pure rules, held by `app/scripts/hubs-check.mjs`.
  - `HubsDashboard`, which replaces `ImportPicker`. It opens from "Hubs…" on
    Home and in the add-project menu, and from "All hubs…" in the ⇄ popover.
  - `BatchSyncModal`: one combined preview, one confirm, a bar per row.
  - `HubPick` in `SyncModal`.
  - Mock: 2 hubs plus a remembered unplugged one; `?nohub`, `?onehub`.
- **Docs.**
  - `docs/sync-ssh-design.md`: naming, manifest reads with no remote install,
    the `-e ssh` argv under R3, host-key trust, and the tests a follow-up
    needs.
  - `docs/sync-hubs-manual-checks.md`: 13 checks David does on his real drives
    and SMB share.

## Decisions

- **`MNT_REMOVABLE`, not `diskutil`'s `Removable`.** Measured on both of
  David's USB SSDs: `diskutil` says `Removable: false` and `Internal: false`,
  while the mount flag is set. `diskutil` also costs 0.1–1.3 s per volume as a
  spawn. `getattrlist` gives the same UUID as `diskutil`'s VolumeUUID, with no
  spawn and no privacy prompt.
- **The CLI reads `sync-hubs.json` below `[sync] hub`.** Otherwise the app and
  the terminal would disagree about which drive "the hub" is.
- **Hubs are ids on the wire, never paths.** A client value never becomes an
  rsync `--delete` target. A device gets chosen hubs only (review item 5): a
  share with a planted manifest is otherwise a pull over the device's project.
- **Batch rows are HostConsole.** A job list names projects that a row cannot
  declare one by one, and a drive plugged into the host is the host user's act.
- **A pull defaults to the newest connected copy; the primary is the CLI's
  hub.** The review caught the docs saying both.
- **A batch pull never imports.** A hub-only project's destination is a folder
  named by a manifest written on another machine. It is shown only in the
  single import modal (review item 3).
- **The entry point** was approved at the checkpoint: "Import from hub…"
  renamed to "Hubs…", import as a row action, "All hubs…" in the popover, no
  rail icon.

## Dead ends / gotchas

- **The first cut swapped drives in the APP.** `defaultPushHubs` fell back to
  the auto drive when every chosen hub was unplugged. The CLI and the backend
  were right; the frontend picked a hub id before asking, so the backend's
  named error never fired. Every core test passed. Only the review's read of
  `hubs.ts` found it, and `hubs-check.mjs` was then shown red on it. A rule
  the backend enforces must be re-checked wherever a client CHOOSES before
  asking.
- **A dashboard read was a read-modify-write outside the lock,** and
  `last_seen = now` rewrote the file on every open. It could clobber a
  concurrent `choose`. Now it goes through `edit_at`, touches only snapshots,
  and stamps `last_seen` at hour grain.
- **A blanket `sed` weakened a pre-existing bats assertion.** Replacing
  `[ ! -s "$RSYNC_LOG" ]` in MY new tests (where `find_rsync`'s `--version`
  probe legitimately runs) also hit the hub-copy test, which exists to prove
  no probe ran. The review caught it, and it is restored.
- **`origin/main` moved during a merge,** so a "lines of main's missing" scan
  compared against a newer main. It reported #590's CHANGELOG entry as
  dropped, and I re-added it by hand before noticing MERGE_HEAD ≠ origin/main.
  The fix: compare against `MERGE_HEAD`, never a ref that can move.
- **zsh does not split `$G` holding a command line.** `env … $G cargo …`
  failed with "No such file or directory". Spell the command out.
- **The vite harness restart races the browser.** A navigate before vite
  printed "ready" leaves Chrome on an error page that every later
  `javascript_tool` call fails on. Navigate again after the restart.
- **The env carries `WORKTREES_VIEWER_JS`,** which fails
  `open_docs_viewer_without_a_bundle_is_a_clear_refusal`. Unset it for the
  cli suite (already in the 2026-10-22 trap list).

## Verification

- New bats in `test/sync.bats`: six `sync hubs` tests on a fake volume list
  (`WORKTREES_SYNC_VOLUMES`). All six were red against the shipped 0.45.0 and
  are green now.
- Core `synchub` tests, each shown red by a mutation:
  - auto-pick taking shares;
  - `last_seen` stamped on every read.
  - Also covered: the read deadline, the configured-hub error, and the
    round trip.
- Api tests:
  - `a_batch_push_reports_every_job_and_one_failure_hides_nothing`: real
    rsync on temp dirs, under `forbid_real_writes` and an empty `SyncCfg`.
    Shown red with the loop stopping at the first failure.
  - `a_client_cannot_name_a_hub_by_path`.
  - `a_device_may_name_only_a_chosen_hub`.
- `hubs-check.mjs`: red on the review's two bugs, then green.
- Mock harness (port 1437): every dashboard and batch control passes an
  `elementFromPoint` hit test. Checked in tokyo-night, tokyo-day and
  catppuccin-latte. Screenshots are in `~/.cache/worktrees/worktrees/sync-hubs/`.
  Driven flows:
  - push to two hubs with one injected failure (the other job finished);
  - pull with a source pick, including an import;
  - a chosen hub unplugged: nothing ticked, the hub named, Push disabled.
- Full local gates on the merged tree, all green: release build, bats 592/592,
  lint, core, cli, api, `cargo check -p app`, tsc, every `*-check.mjs`.
- CI was green 9/9 at the final head `e91c049` after one re-run of an
  unrelated `engine.rs` flake (see ROADMAP).
- NOT verified: real drives and the real SMB share in the real app. That is
  David's, per `docs/sync-hubs-manual-checks.md`.

## Follow-ups

In ROADMAP:
- the `engine.rs` flake's second sighting;
- a batch that re-asks on new live sessions and can run install;
- AFP id measurement;
- a `statfs` deadline when listing volumes;
- removing `sync_hub_list`;
- a local-only api agents flake;
- SSH hubs (designed, not scheduled).
