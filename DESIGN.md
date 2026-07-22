# DESIGN — worktrees UI (Tauri place-manager on the CLI engine)

> Status: design locked (2026-07-20), pending build. Produced by a 4-facet design +
> adversarial-critique + synthesis workflow. See `findings.md` for how we got here
> (fork evaluation → cherry-pick → build-own-UI decision) and `task_plan.md` for phases.

A Tauri desktop app that is a **place manager with a lifecycle**, where the existing
bash CLI (`bin/worktrees`) stays the **engine**. The UI adds what the terminal can't:
resurface dormant places, an explicit lifecycle, resume-on-reopen, and per-place infra
spin-up. It is NOT a fork and NOT a rewrite.

## System in one picture

```
┌─────────────────────────────────────────────────────────────────────────┐
│  Tauri v2 desktop app  (macOS + Linux only — no tmux on Windows = non-goal)│
│                                                                            │
│  ┌──────────────── Frontend: React + TypeScript ───────────────────────┐  │
│  │  Left nav  = PLACES, grouped by lifecycle (Main first, then          │  │
│  │              Pinned/Active/Idle/Closed/Archived/Abandoned) + search   │  │
│  │  Top bar   = selected place's current branch + switch combobox +      │  │
│  │              dirty / ahead-behind chips                               │  │
│  │  Main pane = xterm.js (FitAddon + WebglAddon) + infra strip           │  │
│  │              (Start/Stop, port chips, open-localhost links)           │  │
│  └───────▲───────────────▲──────────────────────────────▲──────────────┘  │
│          │ invoke()       │ Channel<InvokeResponseBody>  │ event            │
│          │ (commands)     │  ::Raw(Vec<u8>)  PTY bytes   │ "places:changed" │
│  ┌───────┴───────────────┴──────────────────────────────┴──────────────┐  │
│  │  Rust core (Tauri backend) — COMMAND RUNNER + PTY HOST + STATE MERGER │  │
│  │   • spawns `worktrees <cmd> --json`, serde_json→structs              │  │
│  │   • hosts PTY running `tmux attach-session` (never owns a shell)      │  │
│  │   • SOLE WRITER of the DECLARED store (.worktrees.places.json)        │  │
│  │   • notify-watcher($GIT_COMMON refs + places file) + 4s poll         │  │
│  │   • NO git / tmux / docker logic of its own                          │  │
│  └───────┬──────────────────────────────────┬───────────────────────────┘  │
└──────────┼──────────────────────────────────┼──────────────────────────────┘
           │ shell out                         │ portable_pty spawn
           ▼                                    ▼
┌────────────────────────────────┐   ┌──────────────────────────────────────┐
│  bin/worktrees  (THE ENGINE)   │   │  tmux server (authoritative)          │
│  bash 3.2, works in ANY repo   │   │   session <prefix>-<slug>             │
│  reads git / tmux / docker,    │   │   pane0 AI CLI (claude), pane1 shell  │
│  emits versioned JSON,         │   │   survives UI crash; `tmux attach`-able│
│  writes NOTHING declared       │   │   from a bare terminal                 │
└──────┬──────────┬───────┬──────┘   └──────────────────────────────────────┘
       ▼          ▼       ▼
     git        tmux    docker (+ lsof)     ← DERIVED state, computed LIVE
```

## Data-flow contract (the spine)

1. **DERIVED state** (branch, dirty, ahead/behind, tmux-up, stack-up, ports,
   claude-session-present) is computed LIVE by the CLI on every `worktrees ls --json`
   — never cached, never stored. Same discipline `cmd_ls` already uses.
2. **DECLARED state** (lifecycle label, pinned, notes, last_opened_epoch, custom up_cmd)
   lives in ONE plain JSON file per repo: `$MAIN_ROOT/.worktrees.places.json`, sibling
   to `.worktrees/` (NOT inside the ephemeral tree). Rust is the SOLE writer; the CLI
   READS it (grep/sed extractor, like `cfg_get`) and folds it into `ls --json`.
3. **Effective lifecycle** (`active`/`idle` overlays vs sticky `closed`/`saved`/
   `archived`/`abandoned`) is computed EXACTLY ONCE, in the CLI, emitted as
   `lifecycle_effective`. The UI only renders it.
4. **Terminals ATTACH to tmux.** Rust spawns a PTY whose child is `tmux attach-session`.
   Close = detach, never kill. Sessions outlive the UI and stay bare-shell attachable.
5. **Infra** is config-driven: the CLI reads `.worktrees.toml`, allocates a port slot,
   and delegates actual bring-up to the repo's declared command. Rust shells to
   `worktrees up/down`, never to docker directly.

### Conflict resolutions baked in
- **Lifecycle transitions live in Rust editing the JSON store — the CLI is READ-ONLY
  for lifecycle.** No `worktrees state` write-subcommand (bash can't field-merge JSON
  without `jq`, which is not a dep; Rust already owns a clean serde writer). Keeps the
  `new`/`rm` bats paths byte-for-byte untouched.
- **Infra status** = a nullable `stack` sub-object per place. Cheap `stack.up` boolean on
  the `ls --json` hot path; full per-service `ports[].listening` only via a lazy
  `worktrees status <slug> --json` for the selected place.
- **Two formats, deliberately:** committed convention = `.worktrees.toml` (TOML, Rust
  `toml` crate + a bash awk subset-reader); per-place declared state = `.worktrees.places.json` (JSON, Rust-written).

## Extended CLI surface (all additive to `bin/worktrees`)

`--json` is a **flag on read commands**, never a new verb, parsed + emitted BEFORE the
human formatting passes so the 12 `ls.bats` assertions stay byte-for-byte identical.
Also honored via `WORKTREES_JSON=1`.

```
# READ (--json flag; human output unchanged when absent)
worktrees ls   [--json]                      # array of places, MAIN first
worktrees status [<slug|branch>] [--json]    # NEW: one place (cwd's if no arg); full derived + per-service infra
worktrees paths [--json]                     # NEW: repo/main/wt roots + places-file path (UI bootstrap)

# INFRA (new verbs; shell out to repo-declared commands, never assemble docker args)
worktrees provision <slug|branch>            # NEW: (re)link/copy env + (re)allocate port slot; no bring-up; idempotent
worktrees up   <slug|branch> [--provision]   # NEW: export slot env, run declared infra.up from the worktree dir
worktrees down <slug|branch> [--keep-volumes]# NEW: --keep-volumes → infra.stop; else infra.down (destroys volumes)

# Exit codes: 0 ok · 1 guard/usage · 3 target-not-found (UI distinguishes "gone" from "broke")
```

Dispatch additions (append to the `case` ~line 670): `status|st`, `paths`, `up`, `down`,
`provision`. Verified none collide with test branch names. Must also be added to `usage()`.

### Place JSON schema (schema_version 1)

```json
{
  "schema_version": 1,
  "slug": "messaging",
  "path": "/Users/x/repo/.worktrees/messaging",
  "is_main": false,
  "registered": true,
  "branch": "feat/next",
  "detached": false,
  "dirty": true,
  "dirty_files": 3,
  "ahead": 2,
  "behind": 0,
  "upstream": "origin/feat/next",
  "created": "2026-07-01",
  "created_epoch": 1751328000,
  "last_commit_epoch": 1752710400,
  "last_commit_subject": "wire up SSE",
  "tmux_session": { "name": "repo-messaging", "up": true },
  "claude_session_present": true,
  "claude_session_dir": "/Users/x/.claude/projects/-Users-x-repo--worktrees-messaging",
  "install_cmd": "pnpm install",
  "stack": {
    "enabled": true, "slot": 3, "compose_project": "repo-wt-messaging", "up": true,
    "ports": [ { "name": "backoffice", "port": 3300, "listening": true },
               { "name": "api", "port": 3301, "listening": true },
               { "name": "pg", "port": 5732, "listening": true } ]
  },
  "declared": { "lifecycle_label": "saved", "pinned": true, "notes": "auth refactor place",
                "last_opened_epoch": 1752700000, "up_cmd": null },
  "lifecycle_effective": "saved"
}
```

Nullability: stale unregistered dir → `registered:false`, most fields `null` (UI shows a
`rm` affordance). Non-stack/no-docker → `stack:null`. No declared entry → `declared:null`.
`is_main:true` object emitted FIRST with slug `"(main)"`, path=MAIN_ROOT. `ls --json`
wrapper: `{schema_version, repo, prefix, places_file, places:[…]}`.

### Must-fix corrections folded into the CLI contract
1. **Valid-JSON serializer** (highest-severity bug): naive escaper missed C0 control
   bytes → invalid JSON → blank UI. Ship `json_str` escaping `\ " \t \n \r \f` AND all
   remaining 0x00–0x1F, `json_bool`, `json_nn` (number-or-null). Bats: a branch/commit
   subject with a control byte, piped through `python3 -m json.tool`, must parse.
2. **ahead/behind is NET-NEW** (no existing helper): add `wt_upstream` +
   `wt_ahead_behind` (`git rev-list --left-right --count '@{u}...HEAD'`), whitespace-split
   with `read -r behind ahead`; no upstream → `null` (NOT 0).
3. No-commit branch → `last_commit_subject: null`.
4. Read the declared file ONCE per `ls --json` (snapshot), never re-grep per place.
5. **Stop-vs-down** is structured, not a flag on an opaque string: `.worktrees.toml`
   declares TWO commands, `infra.stop` (keep volumes) + `infra.down` (destroy). UI "Stop"
   = `--keep-volumes`; volume destruction only on place removal.
6. `up`/`down`/`status` inherit stdout/stderr so Rust can tail progress; exit code is the contract.

## Declared-state store + lifecycle

**Location:** `$MAIN_ROOT/.worktrees.places.json` — sibling to `.worktrees/`, so a
`rm -rf .worktrees` cleanup can't wipe lifecycle/pins/notes. Gitignored via one new line
in `ensure_excluded`.

```json
{ "version": 1, "repo_prefix": "cdv", "updated_epoch": 1752700000,
  "places": {
    "messaging": {
      "lifecycle": "saved",            // closed | saved | archived | abandoned
      "pinned": true,                  // sort-to-top; orthogonal to lifecycle
      "note": "auth refactor place",
      "last_opened_epoch": 1752700000,
      "up_cmd": null,                  // per-place override of infra.up
      "created_by": "ui", "created_epoch": 1751328000 } } }
```
Unknown keys PRESERVED on rewrite (forward-compat). Absent/unrecognized `lifecycle` →
`closed`. Any derived field found here (hand-edit) is ignored. `active`/`idle` NEVER written.

### Lifecycle state machine

| State | Kind | Meaning | Resources on entry |
|---|---|---|---|
| Active | derived | tmux and/or stack live now | — (no write) |
| Idle | derived | opened < IDLE_WINDOW (7d) ago, nothing live | — (no write) |
| Closed | declared default | resting; no live resources | — |
| Saved | declared sticky | explicit keep; `rm` refused without `--force` | keep everything |
| Archived | declared sticky | done for now; infra stopped, **volumes kept**, dir/branch/node_modules kept | `down --keep-volumes` + `tmux kill-session` |
| Abandoned | declared sticky | dead; infra + volumes destroyed, tombstone kept | `down` (destroys volumes) + `tmux kill-session` |

### Reconciliation (pure function, ZERO writes) — computed once in the CLI
```
effective(declared, live):
  if declared in {archived, abandoned}: return declared      # sticky wins over liveness
  if declared == saved:                 return "saved"       # UI shows live ● if tmux_up
  # closed/absent → LIVE TRUTH WINS (manual tmux-attach ⇒ active)
  if live.tmux_up or live.stack_up:     return "active"
  if now - last_opened_epoch < IDLE_WINDOW: return "idle"
  return "closed"
```
Because tmux is authoritative and attach-able from a bare shell, never write-back "Closed"
while a session is live — DERIVE the effective label every read instead.

### Concurrency (Rust sole writer)
- **No `flock`** — the `flock(1)` binary is absent on stock macOS. Use an atomic
  `mkdir` O_EXCL lock (`.worktrees.places.json.lock/`), PID-liveness stale-break
  (`kill -0`), NOT wall-clock.
- **Write:** acquire lock → re-read under lock → per-FIELD merge preserving unknown keys →
  `updated_epoch=now` → temp file in SAME dir → fsync → `rename(2)` (atomic, same-fs) → release.
- **Reads lock-free** (rename atomicity). Parse error → treat as `{}` + banner; NEVER
  delete a file that fails to parse (hand-edit typos must be human-repairable).
- **Rust:** `serde_json` `preserve_order`; every field `#[serde(default)]` + `Option`,
  plus `#[serde(flatten)] extra` per place for unknown-key round-trip. Atomic write via
  `tempfile::NamedTempFile::new_in($MAIN_ROOT)` + `.persist()` (must be `new_in` target dir).
- UI never second-writes; it calls Rust setters (`set_lifecycle`, `set_pin`, `set_note`,
  `touch_place`) and debounces notes in the UI.

## Infra convention (CDV STACK_MODE → config-driven)

Three-tier resolution: (1) `.worktrees.toml` at repo root (committed, reviewed) → explicit;
(2) auto-detect `docker-compose.worktree.yml` → synthesize EXACT CDV behavior;
(3) neither → **inert**: `up`/`down` are loud no-ops, no port allocation, no lsof dep.
Plain repos (and every bats test repo) are byte-for-byte unchanged.

```toml
prefix = "cdv"

[infra]
up   = "./scripts/deploy-local.sh"
stop = "docker compose -p $COMPOSE_PROJECT_NAME down --remove-orphans"   # keeps volumes
down = "docker compose -p $COMPOSE_PROJECT_NAME down -v --remove-orphans" # destroys volumes
status = "auto"                       # auto | compose | volumes | ports | none
compose_files = ["docker-compose.yml", "docker-compose.worktree.yml"]

[port]
enabled = true
base = 3000
stride = 100
max_slots = 50

# One sub-table per service (NO inline tables — bash awk reader can't parse them).
[port.services.backoffice]
offset = 3000
url = "http://localhost:{port}"
[port.services.api]
offset = 3001
[port.services.website]
offset = 3002
[port.services.wo_mock]
offset = 3010
[port.services.meta_mock]
offset = 3011
[port.services.pg]
offset = 5432
probe = true
[port.services.ls]
offset = 4566

[[env.links]]
path = ".env"
mode = "symlink"
[[env.links]]
path = "apps/backoffice/.env.local"
mode = "copy"
```

**CDV fidelity (must_fix):** the auto-detect fallback MUST reproduce ALL 7 ports
(`BACKOFFICE/API/WEBSITE/WO_MOCK/META_MOCK/PG/LS` offset by `100*k`) + env-links
(`.env apps/api/.env apps/api/.env.meta apps/mobile/.env apps/mobile/.env.local` symlinked,
`apps/backoffice/.env.local` copied) + `COMPOSE_PROJECT_NAME=<prefix>-wt-<slug>`, verified
against `worktrees.sh` lines 79/327/336-357. Dropping `wo_mock`/`meta_mock` would collide
slots and break `deploy-local.sh`.

**Port-slot algorithm** (CDV shape, parameterized): scan siblings' `.worktree.env` for used
`WORKTREE_SLOT`; for `k in 1..=max_slots`, skip if used, else `lsof`-probe every
`svc.offset + stride*k` (where `probe != false`); first all-free `k` wins. Deterministic
env var = `uppercase(service_name)` + `_PORT`. **Close the slot race** (UI enables parallel
opens): `mkdir` O_EXCL `.slots.lock` around allocate+write, PID-liveness stale-break.

**Env-link safety:** reject any `path` absolute or normalizing outside the worktree
(realpath containment; gate BSD/GNU like `STAT_STYLE`). **Status off the hot path:**
`ls --json` reports cheap `stack.up`; full per-service detail only in `status <slug> --json`.
Compose ps: tolerate BOTH array (≤2.20) and NDJSON (≥2.21). Infra excludes + `wt_dirty`
pathspecs gated behind infra-active so plain repos stay identical (bats-asserted).

## Tauri architecture

**Rust core has 3 jobs, no git/tmux/docker logic of its own:** (1) command runner —
spawn `worktrees <cmd> --json`, `serde_json::from_slice` into structs; mutating commands
return `CmdResult{ok,stdout,stderr,code}` so the UI shows the CLI's loud guards verbatim.
(2) PTY host. (3) State merger + sole declared writer.

### tmux-terminal embedding (VERIFIED feasible)
```
xterm.js ─Channel<InvokeResponseBody::Raw(Vec<u8>)>─ Rust reader thread ─ PTY master
   │ term_write(id, bytes) ─────────────────────────────────────────────┐
   │ term_resize(id, cols, rows) ─→ master.resize(PtySize) ─────────────┐│
   └────────────────── portable_pty ──→ `tmux attach-session -t <session>`
```
The PTY child is literally `tmux attach-session -t <session>`. The app is just another
tmux CLIENT. **Close = detach, never kill.** Verified: `portable_pty`
(`native_pty_system().openpty`, `spawn_command`, `try_clone_reader`, `take_writer`,
`resize`); transport `tauri::ipc::Channel<InvokeResponseBody>` with
`channel.send(InvokeResponseBody::Raw(bytes))` (true binary, no JSON eval).

Must-fixes folded in:
- **Resize = grouped session by default, NOT `-f ignore-size`** (needs tmux ≥3.2, absent on
  Ubuntu 20.04 / AL2). Attach via `tmux new-session -t <session>` (own session in the group,
  independent size, dodges the smallest-client clamp). Gate `-f ignore-size` behind `tmux -V`.
- **Session bring-up via the CLI**, not Rust: `term_open` calls `worktrees status --json`
  for the session name, and if down, `worktrees open <slug> --no-attach` (reuses existing
  `launch_tmux`). Rust never runs `tmux new-session` itself.
- **Terminal registry** `Mutex<HashMap<TermId, TermHandle{writer,master,stop}>>`;
  `term_close` sets stop, detaches, drops the pty (unblocks reader `read()`), joins thread
  (no leaked thread/fd per open/close).
- **Coalescing:** reader buffers ≤16KB/≤8ms then `channel.send`, holding no lock the
  writer/resize path needs (independent portable_pty handles) → build firehose can't stall keystrokes.

### IPC commands (Rust ← React)
```rust
list_places() -> Wrapper                 // worktrees ls --json
get_place(slug) -> Place                 // worktrees status <slug> --json
paths() -> Paths
set_lifecycle(slug, label); set_pin(slug, on); set_note(slug, note); touch_place(slug)   // Rust edits JSON
new_place(branch, base?, name?) -> CmdResult
switch_branch(slug, branch, base?) -> CmdResult
remove_place(slug, del_branch, force) -> CmdResult
infra_up(slug) -> CmdResult; infra_stop(slug); infra_down(slug)   // shell to CLI
term_open(slug, cols, rows, on_bytes: Channel) -> TermId
term_write(id, data); term_resize(id, cols, rows); term_close(id)  // detach, NEVER kill
// live push: Rust emits "places:changed" from notify + poll → frontend re-pulls list_places()
```
Serde `Place` struct: every unknowable field `Option<T>` (CLI emits explicit null);
`lifecycle_effective: String` computed once by the CLI.

### Frontend (React + TypeScript — xterm.js is React-first)
- **Left nav:** places grouped by lifecycle (Main top; then Pinned/Active/Idle/Closed/
  Archived/Abandoned). Per row: slug, live badges (● tmux, ● stack, dirty dot, ahead/behind,
  ● claude), age. Search filters slug+branch+note — this is how dormant places resurface.
- **Top bar:** branch + switch combobox + dirty/ahead-behind chips; branch≠slug shown cyan.
- **Main pane:** xterm.js (FitAddon+WebglAddon) + infra strip (Start/Stop, port chips by
  `listening`, localhost links). One-click Open = `touch_place` + `worktrees open --no-attach`
  + `term_open` + `infra_up` if configured (active is DERIVED, never written).
- **Trust prompt:** `infra.up/down` are repo-authored strings run via `sh -c` (same trust as
  `make`/`npm run`) — show the command, confirm on first run for a not-previously-trusted repo.

### Persistence
- `tauri-plugin-window-state` → geometry.
- `<app-config-dir>/worktrees-ui/ui-state.json` → selected place, terminal-visible,
  nav-collapse, sort (UI-global; NEVER mixed with per-repo `.worktrees.places.json`).
- Live cadence: `notify` watches `$GIT_COMMON` + places file (300ms debounce) + coarse 4s
  `ls --json` poll for tmux liveness + force-refresh after mutations.

## Phased build plan

| Phase | Goal | Exit criteria |
|---|---|---|
| **P0** Vertical slice | `worktrees ls --json` (valid, reuses helpers) → Tauri shells to it → read-only left nav, main first. No terminal/infra/store. | App shows every place with correct branch/dirty/tmux badges in real repo + CDV; bats 104/104 green; `ls --json \| python3 -m json.tool` valid incl. control-byte branch. |
| **P1** Embedded tmux terminal (**de-risk FIRST**) | Attach to a live session in-app, stream I/O, resize correctly, detach-not-kill on close. | Open → interact → resize (no garble) → close app → `tmux attach` from bare terminal shows SAME session; reopen reattaches; no leaked threads/fds over 20 cycles; works on tmux <3.2. |
| **P2** Declared store + lifecycle | `.worktrees.places.json`; CLI folds it + emits `lifecycle_effective`; Rust sole writer. | Set Saved/Archived/Abandoned → persists across restart, visible in `ls --json`; manual `tmux attach` a Closed place → shows Active (no write); rapid writes don't lose edits; unparseable file → banner, no delete; bats green. |
| **P3** Infra convention | Port slots, env link/copy, repo-declared up/stop/down; schema+UI; inert for plain repos, full CDV fidelity via auto-detect. | CDV: Start → parallel stack on correct offset ports (all 7); volumes survive Stop, destroyed on remove; two simultaneous opens get distinct slots; plain repo `up` is a loud no-op, `.git/info/exclude`+`wt_dirty` unchanged; bats green. |
| **P4** Polish | Live cadence, resurfacing, cross-repo discovery, hardening, packaging. | Bare-terminal branch edit reflects within one cycle; rm'd dir surfaces as orphaned; window+selection restored; installable artifact (dmg/AppImage). Windows = documented non-goal. |

## First PR — `feat: worktrees ls --json (schema v1)`

The smallest end-to-end slice that proves the architecture is the **CLI JSON contract**
(everything downstream consumes it). Ship CLI-only first; the Tauri shell is a fast-follow.

**Scope (bin/worktrees only, strictly additive):** `json_str`/`json_bool`/`json_nn`
helpers (full C0-control escaping) near `sq()` (line 189); new read-only probes
`wt_upstream`, `wt_ahead_behind`, `claude_present` (glob `~/.claude/projects/<slug>/*.jsonl`,
respects `$HOME`); `emit_place_json <dir> <is_main>` reusing the `cmd_ls` Pass-1 logic;
`emit_ls_json` (main first, then `.worktrees/*` in existing recency order; wrapper object);
`--json` flag in `cmd_ls` parsed at TOP → `emit_ls_json; return 0` BEFORE any human
formatting. bash-3.2 safe (no mapfile/assoc arrays; `${var//}` routed through vars).

**NOT in this PR:** no new dispatch verbs, no declared store, no infra, no Rust.

**Tests** (new `test/json.bats`, existing 104 untouched): valid JSON via `python3 -m
json.tool`; control-byte + quote/backslash branch still parses (load-bearing escaper);
no-upstream → `ahead:null,behind:null`; stale dir → `registered:false`; `ls` WITHOUT
`--json` produces the exact prior human table (flag-leak guard); main emitted first.

**Files:** `bin/worktrees` (edit), `test/json.bats` (new), `README.md` (document `ls --json`).

## Open risks (biggest first)
1. **tmux-in-Tauri terminal embedding** — the single biggest risk. APIs verified, but
   resize correctness (grouped-session vs smallest-client clamp on tmux <3.2), clean
   detach-on-close (no zombie attach client), and thread/fd reaping across many cycles are
   integration risks only a real spike settles. De-risk in P1, right after P0.
2. **Claude session slug transform** unverified against a live in-worktree session; the
   `~/.claude/projects/<mangled-path>` mapping may change in newer builds. Validate against
   a real session; degrade to false, never error.
3. **bash awk TOML subset-reader vs Rust `toml` crate** can disagree (inline tables,
   multiline arrays, quoted keys) → forbid inline tables + shared golden-fixture bats test.
4. **Auto-detect infra drift from CDV** — if the preset isn't the full 7-port map + exact
   env-links, existing CDV ports shift and `deploy-local.sh` breaks. CDV-equivalence bats test.
5. **Poll cost** O(places) subprocess spawns → fs-watch-first + coarse poll + heavy docker
   probe off the hot path.
6. **Slot race** under parallel opens — mkdir O_EXCL `.slots.lock` + PID-liveness (not flock).
7. **RCE surface** — `infra.up/stop/down` run via `sh -c`; show command + first-run trust prompt.

## Principle checklist (all upheld)
1. **CLI stays the engine** — Rust only spawns `worktrees … --json` + `tmux attach` in a PTY;
   session creation delegated to `worktrees open --no-attach`; new probes are read-only. ✓
2. **Derived-vs-declared split, no DB** — live fields recomputed every read; declared facts in
   one plain JSON; CLI read-only for declared, Rust sole writer; config is data, never sourced. ✓
3. **Attach to tmux, never own PTYs** — PTY child is `tmux attach-session`; close = detach;
   session survives UI SIGKILL, still bare-shell attachable; only `worktrees rm` kills. ✓
4. **Generalize CDV stack-mode** — `.worktrees.toml` up/stop/down/port-plan/env-links;
   auto-detect reproduces exact CDV behavior; inert for plain repos. ✓
5. **Keep CLI values** — `--json` returns before human path (ls.bats untouched); no CLI
   declared writes (new/rm unchanged); infra excludes gated behind infra-active; no flock;
   no mapfile/assoc arrays; loud guards extended. ✓

## Verdict
Overall readiness: **HIGH — build in the phased order.** All four facet designs are sound
and ground in the real code; conflicts resolve cleanly. Every must-fix is folded (the
control-char JSON escaper being the highest-severity). The single biggest risk is the
tmux-in-Tauri embedded terminal — do the P0 read-only slice to prove the spine, then
IMMEDIATELY spike P1 before investing in lifecycle/infra UI. If P1 fails, the whole
place-manager value proposition is at stake, so settle it while surrounding code is cheap.
