---
title: "cdv migration runbook"
---

# cdv migration runbook

Derived from the read-only dry-run of 2026-07-28. Nothing in the consumer repo
was modified to produce this.

- Consumer: `/Users/davidpena/workspace/casadelvalle/casa-del-valle-monorepo`
- Config to install: `cdv.worktrees.toml` (this directory)
- Tool: branch `next-stream` in `~/workspace/worktrees/.worktrees/everything-settings`

## What the dry-run found

Better than expected: all 17 worktrees under `.worktrees/` are already correctly
linked for every entry in main's 5-file list, the copied `.env.local` is
byte-identical to main in all 17, and slots 1–15 are unique with no conflicts.
So `relink --all` is a **no-op** and `provision --all` mostly rewrites files in
place with the same slots.

Three real problems it surfaced:

1. **11 of 15 provisioned worktrees have no `WEBSITE_PORT`** — their
   `.worktree.env` predates that port entering the map. `apps/website` in those
   binds **3002, which is main's port**. Fixed by `provision --all`.
2. **`claude-work-integration` and `prod-reviews` have no `.worktree.env` at
   all**, and both have live tmux sessions. One `./scripts/deploy-local.sh` in
   either takes the `WORKTREE_MODE=false` branch, which `kill -9`s every
   `next-server` / `tsx watch` / `next dev` / `wo-mock-server` on the machine and
   force-frees :3000. This is the hazard the whole feature exists to close.
3. **Do NOT run `worktrees init` here.** Its suggestion is wrong twice, silently:
   it marks `apps/backoffice/.env.local` as a link (must be `copy` —
   `deploy-local.sh` rewrites it, so a link scribbles on main and through it on
   every other worktree), and it names ports after compose *services*
   (`POSTGRES`/`LOCALSTACK`) rather than the `PG`/`LS`/`WEBSITE`/`META_MOCK` the
   repo consumes. Use `cdv.worktrees.toml`, transcribed from
   `scripts/worktrees.sh` on `main` @ `03a0a787`.

## Step 0 — before anything (blocking)

Verify `apps/mobile/google-services.json` against the Firebase console. On
2026-07-27 at 21:02 that file appeared in main; at 21:33 every worktree's copy
became a symlink to it — the fingerprint of the old `relink --all`, whose
`ln -sfn` replaces a real file with no backup. `general-fixes` previously held
the only real copy. No `.bak` exists anywhere under `.worktrees/`.

Compare:

| field | current value |
|---|---|
| `project_id` | `casa-del-valle-firebase-202607` |
| `project_number` (FCM sender id) | `86759926600` |
| `mobilesdk_app_id` | `1:86759926600:android:9de4eda9a341d5c273a17d` |
| package | `com.casadelvalle.mobile` |

If the sender id differs from the console's, push has been quietly broken since
the 27th — which is exactly the failure mode described in §1.2 of the proposal.

## Step 1 — land the tool

The installed stable at `~/.local/bin/worktrees` has no `relink`, `doctor`,
`provision`, or `init`, and **both binaries report `0.3.1`** — the version string
cannot tell them apart.

Do **not** `make install` from the branch: that symlinks the clone's build, so
every later rebuild silently becomes cdv's "stable", and cdv's `deploy-local.sh`
behavior now depends on it. Merge `next-stream`, cut the release, run
`install.sh`.

If the migration must happen before the release, run every step below with the
explicit path `~/workspace/worktrees/.worktrees/everything-settings/target/release/worktrees`
and do **not** commit `.worktrees.toml` yet — a committed config plus an
installed binary that ignores it is the half-provisioned state §1.1 forbids.

Verify: `worktrees --help | grep doctor` in a fresh shell.
Rollback: re-run `install.sh` pinned to the previous version.

## Step 2 — install the config (do not commit yet)

```sh
cp ~/.cache/worktrees/worktrees/everything-settings/dryrun/cdv.worktrees.toml \
   ~/workspace/casadelvalle/casa-del-valle-monorepo/.worktrees.toml
worktrees doctor --config-only      # expect exit 0
```

Rollback: `rm .worktrees.toml` — the repo is byte-identical to today.

## Step 3 — inspect, change nothing

```sh
worktrees doctor
worktrees doctor --json
```

Expect: zero file findings, two `no-slot` errors
(`claude-work-integration`, `prod-reviews`), one `compose-drift` warn
(`website-app`, whose recorded project name is `casa-website-app`, not the
template's `cdv-wt-website-app`), and — after the doctor fix landed — eleven
`missing-port` warns. Exit 2.

**Anything else means the repo drifted since the dry-run. Stop and re-read.**

## Step 4 — relink

```sh
worktrees relink --all
```

Expect exit 0 and nothing beyond headers. If it reports a **shadowed** file,
stop and read it: that is a real file someone created where the config claims a
link, and the tool is refusing to destroy it. The old bash script would have
deleted it without a word.

## Step 5 — provision

⚠ This is the only step that writes a file a running stack sources. Do it while
nothing is up (as of the dry-run, nothing was listening on any slot port).

```sh
cd ~/workspace/casadelvalle/casa-del-valle-monorepo
mkdir -p /tmp/wt-env-backup && for d in .worktrees/*/; do
  [ -f "$d/.worktree.env" ] && cp "$d/.worktree.env" "/tmp/wt-env-backup/$(basename "$d").env"
done
worktrees provision --all
```

Verify:
```sh
grep -c WEBSITE_PORT .worktrees/*/.worktree.env | grep -c ':1$'   # expect 17
grep -h WORKTREE_SLOT .worktrees/*/.worktree.env | sort -u | wc -l # expect 17
```

Rollback: restore from `/tmp/wt-env-backup/`. This fully reverts the step.

## Step 6 — prove one stack

In a worktree that was missing `WEBSITE_PORT` (e.g. `todos`, slot 14), run
`./scripts/deploy-local.sh` and confirm the website comes up on **4402**, not
3002, and that main's stack is untouched.

Rollback: restore that one `.worktree.env`.

## Step 7 — retire the bash stack mode

One commit in the cdv repo: commit `.worktrees.toml`, delete `scripts/worktrees.sh`'s
stack-mode block (`:75-81`, `:319-359`, and the `lsof` guard at `:251`), and
update that repo's `CLAUDE.md`, which still documents `worktrees.sh switch` at
lines 171-173.

**Keep the rest of the script** — its `ask` subcommand (the headless Claude
query, `:676-747`) has no counterpart in this tool.

Rollback: `git revert`. The script is self-contained and reads the same
`.worktree.env` files the tool now writes.

## Step 8 — the Firebase entries

Add `apps/mobile/google-services.json` to the config (and
`GoogleService-Info.plist` once it exists in main) as a normal config change plus
`relink --all`. Per the dry-run this is already a no-op — the links exist — which
is itself the demonstration that the workflow works.

## Known hazards left in place, deliberately

- **Four registered worktrees outside `.worktrees/`** — three under
  `.dmux/worktrees/` and the sibling `casa-del-valle-monorepo-design-skills`.
  They have no `.worktree.env`, so each is the same `pkill -9` hazard as §1.1,
  and `provision --all` cannot see them: place discovery only scans
  `.worktrees/`. Decided to leave them; they belong to another tool.
- **Volume reclamation before the compose-files fix.** Any `rm` run before that
  fix leaked `<project>_postgres_data` / `<project>_localstack_data`. Check with
  `docker volume ls -q | grep '^cdv-wt-'` before assuming a removed worktree is
  fully gone.

## Cleanup candidates (recommendations only)

Strong — merged into main, ≥3 weeks idle, no live session:

| worktree | branch | last commit | behind main | note |
|---|---|---|---|---|
| worktrees-work | feat/worktrees-more | 2026-07-04 | 274 | clean |
| flujo-de-caja-review | flujo-recurring-terceros | 2026-07-02 | 346 | 2 dirty files |
| bug-fixes | fix/facturas-followups | 2026-07-03 | 318 | 1 dirty file |
| feat-reports | feat/reports | 2026-06-29 | 402 | 1 dirty file |

Moderate — unmerged but small and very stale; read the commits first:
`arch-diagrams` (2 ahead), `docs-review` (3 ahead, 1 dirty), `chat-history-for-improvements`
(6 ahead), `sentry-state-and-improvements` (1 ahead), `todos` (1 ahead).

Keep: `general-fixes` (active, 21 ahead, committed today), `complete-messaging-work`,
`feature-delivery-mu`, `phone-integration`, `website-app`, `mcp-server` (18 dirty
files — `rm` would refuse without `--force`), `claude-work-integration`,
`prod-reviews`.

Before removing any of them, run `docker volume ls -q | grep '^cdv-wt-'` and note
that `website-app`'s project name is `casa-website-app`, not `cdv-wt-website-app`.
