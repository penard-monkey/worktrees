# Proposal — per-project settings

**Status:** v1, v2, and v3 BUILT on branch `next-stream` (2026-07-28), not yet
merged. See §12 for what each phase covered. Remaining: the cdv migration.
Design decided 2026-07-27.
**Supersedes:** the first draft of this file, written from the consumer side
during a Casa del Valle push-notifications session. Six of its factual claims were
wrong; they are corrected inline below and listed in §11.
**Owner:** this repo. First consumer: `casa-del-valle-monorepo`.
**Backing research:** four parallel audits (cdv tooling, worktrees-core seams, app
surface, format/security) — archived in this session's `findings.md`.

---

## 1. The gap

The bash CLI this tool was ported from had a **stack mode**: when a repo shipped
`docker-compose.worktree.yml`, creating a worktree also symlinked a declared list
of untracked secret files from the main checkout, copied one file that a runtime
script rewrites, allocated a free port slot, and namespaced the docker project.

**The Rust port carried none of it.** `Project` (`project.rs:15`) is
`{ main_root, git_common, wt_root, prefix, clock }` — no notion of a project
having setup of its own. `config.rs` reads three user-level keys and its docstring
is explicit that config is *"parsed as data, never executed."*

So the monorepo runs **two tools** that both create worktrees. That is not a
missing feature, it is a **fork** — and the fork is not benign.

### 1.1 The live hazard (this is why ports can't be deferred)

`scripts/deploy-local.sh:283-301` branches on `WORKTREE_MODE`, which is simply
*"does `.worktree.env` exist."* The **false** branch runs a global
`pkill -9 -f` over `next-server`, `tsx.*watch`, `next dev`, `wo-mock-server`, then
force-frees host port 3000.

Two worktrees in that repo today — `claude-work-integration`, `prod-reviews` —
were created by **this** tool and have no `.worktree.env`. Running the repo's own
dev script in either one kills the main checkout's entire running stack and then
fights it for :3000/:5432/:4566.

**A half-provisioned worktree is worse than no worktree.** Either the tool fully
provisions a project it recognizes, or it refuses to create there.

### 1.2 What made the file-sync half urgent

That repo added its first non-`.env` credential (a Firebase
`google-services.json`). Two things broke, and both generalize:

- **Links are made at creation time only.** Every existing worktree knew nothing
  about the new file.
- **The failure is silent.** A worktree missing `google-services.json` builds an
  Android app with no FCM sender id: no push token, no exception, no log line. You
  find out on a device, days later.

Silent-by-default failure is what makes this worth designing rather than scripting
per repo.

---

## 2. Design principles

These are the load-bearing constraints. Everything below follows from them.

1. **Generic first.** cdv is the first consumer, not the specification. Each
   section stands alone: a plain Node repo with one gitignored `.env` gets value
   from `[[file]]` with no `[ports]` and no `[compose]`. A Rails app wanting port
   isolation gets `[ports]` with no docker anywhere. Nothing in the schema names a
   framework, and no section requires another.
2. **A project declares *structure*; a user declares *behavior*.** Anything that
   becomes argv, or names a program to run, is user-scoped — permanently. See §5.
3. **No repo-supplied argv, ever.** Today, nothing a cloned repository contains
   can cause execution: every subprocess in core is literal argv. That property is
   kept. See §5.
4. **Absent config ⇒ byte-identical behavior.** A repo with no `.worktrees.toml`
   behaves exactly as today, down to the output.
5. **Fail loud.** The bug being fixed is silence. A declared file missing from
   main is a *warning*, not a skip. A shadowed link is an *error*, not an
   overwrite.

---

## 3. The file

`.worktrees.toml` at the repo root. Committed — this is project structure, like
`docker-compose.worktree.yml`. Coexists with (and subsumes) `.worktree-prefix`.
Unrelated to `.worktrees.places.json`, which is gitignored per-machine state.

```toml
# One list, per-entry mode. Anything gitignored that a build or script reads —
# not just .env files.
[[file]]
path = ".env"
# mode defaults to "link" — ONE source of truth; a per-worktree copy drifts.

[[file]]
path = "apps/mobile/google-services.json"   # Firebase Android (FCM)

[[file]]
path = "apps/backoffice/.env.local"
mode = "copy"    # deploy-local.sh rewrites this at runtime; a link would
                 # scribble on the main checkout — and on every other worktree.

# Port isolation. Stands alone: no docker required. Each place takes the lowest
# free slot k; every port below becomes base + stride*k.
[ports]
stride = 100
max_slots = 50
base = { BACKOFFICE = 3000, API = 3001, WEBSITE = 3002, WO_MOCK = 3010,
         META_MOCK = 3011, PG = 5432, LS = 4566 }

# Optional. Only if the project namespaces a docker-compose stack per worktree.
[compose]
file = "docker-compose.worktree.yml"
project = "{prefix}-wt-{slug}"
```

### Why one `[[file]]` list instead of `[link]`/`[copy]` tables

Separate tables make "same path in both" a reachable state you then have to
reject. `doctor` output is cleaner over one list. And each entry carries its own
`# why this is a copy` comment — which is this file's whole teaching job.
`DESIGN.md:257-263` already had this shape.

### Why the env file's name is not configurable

Slot allocation works by scanning sibling places for their env files (§6). That
scan hard-depends on every place using the same filename. Making it configurable
means a repo that changes it after worktrees exist makes the old places invisible
to the scan and double-allocates slots. So the name is fixed at `.worktree.env`,
tool-owned, like `.worktrees.places.json`. No consumer was found that needs
otherwise.

### `.worktree.env` is a public wire format

Not an internal artifact. In cdv, `deploy-local.sh:54-57` does
`set -a; . .worktree.env; set +a` — **every line is sourced as shell**. Six
consumers read it directly, and `deploy-local.sh` derives ~15 further values from
it (`DATABASE_URL`, four SQS queue URLs, `CORS_ORIGIN`, `AUTH_URL`, …).

Two consequences:

- Emitted lines must stay valid POSIX shell. `KEY=value`, no quoting surprises.
- **`WORKTREE_SLOT` must be emitted**, not held as "internal state that belongs in
  places.json." `deploy-local.sh:758` keys `WATCHPACK_POLLING=true` off its mere
  presence; drop it and every worktree stack silently regains the
  EMFILE → all-routes-404 failure that repo already debugged once.

Emitted content:

```
WORKTREE_SLOT=<k>
COMPOSE_PROJECT_NAME=<sanitized>      # only when [compose] is present
<NAME>_PORT=<base + stride*k>         # one per [ports].base entry
```

---

## 4. Security boundary

`.worktrees.toml` arrives with a `git clone`. Treat it as hostile input. The
threat is a malicious (or careless) config linking something outside the repo into
a worktree where `git add -A` publishes it.

Two layers. The second one is the actual boundary; the first exists for early
rejection with good error messages.

### Layer A — literal string, at parse time

A failure rejects the **whole config** — partial application is how you get
half-provisioned worktrees.

- non-empty, no NUL, ≤1024 bytes, ≤256 entries
- reject leading `/` (absolute)
- reject leading `~` — no expansion is performed, so it would silently become a
  literal directory name
- reject `$` anywhere — no env expansion is performed, and this keeps the door
  shut on ever adding it
- split on `/`; reject any component that is empty, `.`, or `..`
- **reject any component equal to `.git`, case-insensitively.** Containment cannot
  catch this: `.git/hooks/pre-commit` *is* inside the repo, and linking it is code
  execution on the victim's next commit. Highest-value target on the whole
  surface; must be a name rule.
- reject `.worktrees` as first component; reject `.worktrees.places.json` and
  `.worktrees.toml` as whole paths
- **reject case-only duplicates** (`Foo/.env` vs `foo/.env`) — one file on macOS,
  two in Linux CI. That divergence must be a config error, not a race.
- reject the same path appearing twice
- **the same rules apply to `[compose] file`** — it is a repo-relative path too

macOS and Linux are the only shipped targets (`release.yml:18-21`), so `\` is an
**ordinary filename byte**, not a separator. Treating it as one would make a
legitimately-named file unlinkable. One unit test asserts `a\b.env` is a single
component.

### Layer B — resolved path, at apply time

With `main = canonicalize(main_root)` and `wt = canonicalize(worktree_dir)`:

- **B1** `canonicalize(main/<rel>.parent())` must be inside `main`. Catches a
  **symlinked ancestor** (`apps` → `/etc`). Only a resolved-path check sees this.
- **B2** `symlink_metadata(src)` — lstat, does not follow.
- **B3** if the source is itself a symlink, its `canonicalize` must be inside
  `main`. **This is the real attack**: git can commit a symlink, so a hostile repo
  ships `apps/mobile/google-services.json` as a committed symlink to
  `~/.ssh/id_rsa` and every Layer-A rule passes. Cost: a user's legitimate
  `main/.env → ~/secrets/prod.env` stops working. Accepted and documented for v1;
  the safe-by-default direction is unambiguous here.
- **B4** followed metadata must be a regular file. A FIFO source hangs `copy`
  forever; a device node is worse.
- **B5** cap copy source size at 8 MiB.
- **B6** never blind `create_dir_all`. Walk components from `wt` down; a symlinked
  ancestor you didn't create is a hard error.
- **B7** destination branch:
  | destination is | action |
  |---|---|
  | absent | create |
  | symlink → expected src | no-op — *this is what makes `relink` idempotent* |
  | symlink → elsewhere | replace, and log it |
  | **regular file** | **shadowing. Do NOT overwrite.** Report, exit non-zero, require `--force` |
  | directory | hard error |

  Zero recursive deletes anywhere in this module.
- **B8** `git -C <wt> check-ignore -q <rel>`. Not ignored ⇒ **error**: *"declared
  file `apps/mobile/google-services.json` is not gitignored — `git add -A` in this
  worktree would commit it."* Layer A/B containment stops the hostile repo; B8
  stops the far more likely **accident** — a teammate adds a credential to the
  config and forgets `.gitignore`.

### TOCTOU

The adversary is the repo's contents, not a racing local process — anyone who can
win a symlink race inside your main checkout already has your uid. So
`openat2(RESOLVE_BENEATH)` is unwarranted (and unavailable on macOS). What is
warranted and cheap:

- **Validate and act on the same resolved `PathBuf`.** Never re-derive from the
  string after validation. That is 90% of the mitigation.
- For copies: `File::open` first, then check regular-file-ness on the open fd's
  metadata, not a second lstat.
- For links: `symlink(src, tmp_in_same_dir)` then `rename(tmp, dst)` — atomic, and
  `rename` never follows a symlink at `dst`. Reuse `store.rs::write_atomic`'s
  pattern (`store.rs:126-133`).

### Make it unbypassable structurally, not procedurally

```rust
pub struct RelPath(String);                      // field PRIVATE
impl RelPath {
    pub fn parse(s: &str) -> Result<RelPath, CfgError>   // ONLY constructor — Layer A
}
// no From<String>, no Deref<Target=str>, no AsRef<Path>

pub struct PlannedOp { src: PathBuf, dst: PathBuf, mode: Mode }   // fields PRIVATE
pub fn resolve(p: &Project, wt: &Path, r: &RelPath)
    -> Result<PlannedOp, CfgError>               // ONLY constructor — Layer B

pub fn apply(op: PlannedOp) -> Result<Outcome, io::Error>
    // takes by value; performs NO string manipulation whatsoever
```

`ProjectConfig`'s list is `Vec<RelPath>`, deserialized through a custom
`Deserialize` that calls `RelPath::parse` — so **a `ProjectConfig` holding an
invalid path is unconstructible.** `new`, `relink`, `doctor`, and the app's Relink
button are all type-forced through `parse → resolve → apply`. There is no
`link_file(&str)` in the public API to find and misuse.

---

## 5. Execution: none, permanently

**Decision: `[hooks]` will not ship, and `DESIGN.md:225-228`'s
`[infra] up/stop/down` is reversed.** This is a deliberate reversal of a doc marked
"design locked (2026-07-20)"; nothing was built, so the cost is zero.

The "we already execute `ai_cmd`, so hooks are a small delta" argument proves the
opposite, because provenance is the entire thing:

| Channel | Who writes the string |
|---|---|
| `ai_cmd` → `sh -ic '<ai_cmd>; exec $SHELL'` (`ops.rs:52`) | flag, `$WORKTREES_AI_CMD`, or user config — **a cloned repo cannot set it** |
| `install_cmd` → tmux pane 1 (`ops.rs:57`) | one of **four literal constants** selected by lockfile detection (`ops.rs:303-316`) — the repo picks *which of my commands*, not *what* |
| `[hooks] post_create` | **the cloned repo's committed file, verbatim** |

That is a categorical change from *"the repo selects among my commands"* to
*"the repo writes my command."*

direnv's hash-trust model also transfers badly: direnv prompts on `cd` into a repo
you already work in; this tool's hot path is `worktrees new` **on a fresh clone**,
the moment of least knowledge. The GUI would have to fire a security modal from a
background poll, which is the definition of a dialog people click through. And the
original proposal's own *"CI defaults to untrusted"* is a tell that the shape is
wrong — the feature would be off precisely where automation wants it.

**What replaces it**, at ~20 lines and zero new surface: a **user-scoped** hook.

```toml
# ~/.config/worktrees/config.toml — user file, never .worktrees.toml
post_create = "./scripts/dev-setup.sh"
```

Identical execution power, user provenance. No hash file, no trust prompt, no GUI
modal, no CI question. The repo still ships the script; each developer opts in once
per machine.

**This is affordable because the declarative sections cover the real need.** The
audit of cdv's 761-line script found exactly one behavior not expressible as
file/ports/compose: `docker compose -p <project> down -v` at removal. That becomes
a first-class `[compose]` teardown — the tool already knows the project name and
the file list — not an arbitrary string. `pnpm install` is already covered by
lockfile detection.

Correspondingly, `[compose]` is data and **the tool assembles the argv**:
`docker compose -f <validated files> -p <sanitized name> up -d`. Never a command
string from the repo. The first-run trust prompt (`DESIGN.md:343, 399`) is deleted
rather than implemented.

### Precedence

```
flag  >  env (WORKTREES_*)  >  .worktrees.toml  >  ~/.config/worktrees/config[.toml]  >  default
```

The project rung exists **only for allow-listed keys**.

| Project MAY set | Project may NEVER set |
|---|---|
| `[[file]] path` / `mode` | `ai_cmd`, `ai_resume_arg` |
| `[ports] stride` / `max_slots` / `base` | any install-command override |
| `[compose] file` | `post_create` |
| `[compose] project` — a template with a **closed** placeholder set (`{prefix}`, `{slug}`), pushed through `sanitize_prefix` before it reaches `docker -p`. Not a free string. | `[infra] up/stop/down` |
| `[project] prefix` | |

`prefix` is project-settable because (i) `sanitize_prefix` (`config.rs:10-21`)
reduces any input to `[a-z0-9_-]`, so no shell metacharacter or leading `-` can
survive, and (ii) **the project already sets it today** via the committed
`.worktree-prefix`. Order within the project tier:
`$WORKTREES_PREFIX > .worktree-prefix > [project] prefix > user config >
basename(main_root)` — legacy file first keeps migration non-breaking, and
`doctor` warns when both exist and disagree.

⚠ Prefix changes are session-orphaning. `session_name` is the tmux identity and is
reconstructed independently at `ops.rs:31`, `ops.rs:521`, `ops.rs:607`, and
`project.rs:195`. Merely *adding* a config with a prefix to a repo with live
sessions renames them all, and `close_one` would report "nothing to close" for a
session that is very much alive. **`[project] prefix` is therefore deferred out of
v1** — lowest value, highest blast radius.

Two policy details:

- A user-only key found in `.worktrees.toml` is a **hard parse error** with a
  pointing message (`ai_cmd may not be set by a project — this is a user setting`).
  **Not** a silent ignore: silent ignore trains people to write it, and eventually
  someone "fixes" the ignore.
- Unknown keys warn once and are ignored — same forward-compat discipline as
  `store.rs`'s `#[serde(flatten)] extra`.
- `WORKTREES_NO_PROJECT_CONFIG=1` disables the project rung wholesale — the "I'm
  auditing an untrusted clone" switch.

---

## 6. Ports

**Decision: the slot is derived from `<wt>/.worktree.env`. It is never stored in
`.worktrees.places.json`.** Two independent audits converged on this.

Why not the places store:

- The store's contract (`DESIGN.md:158-176`) is *sticky user intent that survives
  the worktree being gone*. A port slot is meaningless without the worktree.
- `DESIGN.md:176` says "any derived field found here is ignored" — a slot there
  would be a derived field you *couldn't* ignore.
- The store is gitignored and safely-losable by design. Losing a lifecycle label
  is annoying; losing a port assignment orphans a running docker stack under a
  stale project name.
- **`remove_one` never calls `store::edit`** — verified: `store::` appears nowhere
  outside `store.rs` in `crates/`. A stored slot would leak on every `rm`, making
  `max_slots = 50` a silent exhaustion bug that surfaces months later.
- With derivation there is **no release step at all**: `rm` deletes the directory,
  the env file goes with it, the next scan sees the hole.

### Algorithm

```
lock = <main_root>/.worktrees.slots.lock       # mkdir(2) O_EXCL, PID written inside

1. acquire(lock)
     EEXIST → read pid; kill(pid,0)==ESRCH → rmdir, retry
              else 15ms backoff, ~3s cap, then a hard error naming the holder.
     Use PID-liveness, NOT wall-clock mtime — see the note below.

2. scan INSIDE the lock:
     for each dir in .worktrees/*:
         k = WORKTREE_SLOT from dir/.worktree.env
         if k already claimed → record CONFLICT(dir, k)  else  used.insert(k → dir)
     main_root implicitly holds slot 0.

3. if this place already holds a unique slot → return it unchanged.
     ⇒ create / relink / provision are idempotent no-ops.

4. for k in 1..=max_slots:
       if k in used → continue                      # conflicted k counts as USED
       if any port base+stride*k fails to bind → continue
       chosen = k; break
     none → error "no free slot in 1..=max_slots (N declared, M ports busy)"

5. write <wt>/.worktree.env atomically (tmp in same dir, rename)

6. ensure_excluded() gains ".worktree.env"; verify with `git check-ignore`

7. drop(lock)
```

`git worktree add` happens **outside** the lock; only the scan and write are
inside it.

**Probe by `TcpListener::bind` on `127.0.0.1` and `0.0.0.0`, not `lsof`.** Removes
the `lsof` dependency (`DESIGN.md:276` worried about it; cdv's script hard-errors
without it), is ~100× faster, identical on both targets, and has no false negative
on docker-proxy. `SO_REUSEADDR` means a `TIME_WAIT` socket may bind — acceptable,
because the declared `used` set, not the probe, provides stability. The probe is
only the tiebreaker for "some unrelated process squats 3300."

**Conflict policy.** Two places claiming the same k (hand-edit, restored backup):
`provision` **refuses; it never silently re-allocates.** Rewriting a slot under a
running stack orphans containers on the old project name and silently moves ports
the developer has bookmarked. Exit 1, name both paths, print both remedies.
`doctor` reports it as an error. A conflicted k counts as **used** so conflicts
never compound.

**Hand-edited `.worktree.env`: the user always wins.** Allocation never rewrites an
existing `WORKTREE_SLOT` without `--reallocate`. It is the file the running stack
sources; the tool disagreeing with it would be the tool lying.

**Free parse-time lint** (turns `DESIGN.md` risk #4 into a pure function): with a
shared stride, services *i* and *j* collide iff `(bᵢ − bⱼ) % stride == 0` and
`|bᵢ − bⱼ| ≤ stride × max_slots`. Also require all bases distinct and
`max(base) + stride × max_slots < 65536`.

⚠ `store.rs:104-113` currently uses **wall-clock (15s mtime) staleness**,
contradicting `DESIGN.md:205` which mandates PID-liveness. It false-positives
across laptop sleep. Fix it there, or at minimum do not copy it into the slot lock.

---

## 7. Commands

| Command | Does |
|---|---|
| `worktrees relink [<wt>\|--all]` | re-apply the file plan to existing worktrees. **Ships first** — without it, any config change strands every existing worktree. |
| `worktrees doctor [<wt>]` | report drift. **Also v1**, not v4 — see below. |
| `worktrees provision [<wt>\|--all]` | allocate/repair a port slot + write `.worktree.env`. Idempotent. |
| `worktrees init` | the suggestion flow (§9). |

`doctor` must ship **with** v1, not after. Creation is deliberately
non-transactional (`ops.rs:297-300`: *"a failed session is a partial success the
user MUST see"*). Materialization adds a fourth thing that can half-succeed with no
rollback, and "materialized" is not a state anything records — so without `doctor`,
a half-linked worktree is indistinguishable from a never-linked one. A feature whose
own failure mode is silent does not fix a silent-failure bug.

### Findings and exit codes

`doctor` needs structured findings with severities. `WtError` is a
single-fatal-error-with-exit-code type and must not be widened. New module:

```rust
// crates/worktrees-core/src/diag.rs
pub enum Severity { Info, Warn, Error }
pub struct Finding { severity, code, place, path, message }
pub struct Report  { schema_version: u32, findings: Vec<Finding> }
```

`code` is a stable slug: `missing-source`, `shadowed`, `dangling-link`,
`wrong-mode`, `not-gitignored`, `slot-conflict`, `no-slot`, `copy-stale`.

Exit codes: **`0`** clean · **`1`** usage/guard (the tool's existing convention,
`main.rs`) · **`2`** findings present. The original "exit non-zero" collided with 1;
CI must distinguish "doctor broke" from "doctor found problems."

`doctor --json` follows the `ls --json` template exactly: a struct in `model.rs`
with its own `schema_version`, honoring `WORKTREES_JSON=1`.

⚠ Prerequisite: `CaptureUi` (`ui.rs:58-84`) flattens info/warn/error into one
`Vec<String>` — **severity is erased at capture**, and its `errored: bool` is set
but never read. `run_op` (`lib.rs:322-335`) consumes only `ui.lines`, so a warning
on a successful op vanishes entirely (already on the roadmap as *"success/warning
output discarded by `runCmd`"*). A `doctor` whose warnings the app drops does not
solve the stated problem. Fix the channel in the same stream.

### Copy drift policy — a copy is a seed, not a mirror

Links cannot drift by construction. Copies can, and "is this drift OK?" is
unanswerable — so answer a different question: *is this drift **older than** the
source?*

| State | Condition | Severity |
|---|---|---|
| **missing** | dest absent, or dest is a symlink (wrong mode, likely an older config) | **error** |
| **pristine** | `hash(dest) == hash(src)` | ok |
| **drifted** | differs **and** `mtime(dest) >= mtime(src)` | **info** — never affects exit code; this is the expected steady state after a script rewrote it |
| **stale** | differs **and** `mtime(src) > mtime(dest)` | **warn**; non-zero only under `--strict` |

Hash is checked first, so a `git checkout` that only touches mtime cannot produce a
false `stale`. `relink` without `--force` never touches an existing regular file;
with `--force` it writes a `.bak` alongside first — the drifted content may be the
only copy of a locally-tuned value.

### `doctor` in CI

Yes, but the useful check is a **different** one: declared sources are gitignored,
so they are absent in CI and cannot be verified. What *can* be checked on a bare
clone with pure git and no filesystem state:

1. every declared path is gitignored, and
2. no declared path is tracked in git.

(1) catches a config/`.gitignore` mismatch; (2) catches someone having committed a
secret. Ship as `worktrees doctor --config-only`.

---

## 8. Implementation seams

Three new modules in `worktrees-core`. No change to `Project`, `error.rs`,
`store.rs`'s schema, or `config.rs`'s existing keys.

- **`diag.rs`** — `Finding` / `Report` / `Severity` (§7).
- **`projcfg.rs`** — `ProjectConfig`, `RelPath`, `parse` (pure), `load` (returns
  `None` when the file is absent). ⚠ macOS is case-insensitive: do not name this
  `Config.rs`. `Settings.tsx` already collided with `settings.ts` once.
- **`materialize.rs`** — the plan→apply split:
  ```rust
  pub fn probe(cfg, main_root, wt) -> FsFacts;   // the ONLY filesystem read
  pub fn plan(cfg, main_root, wt, &FsFacts) -> Plan;   // PURE — the whole test surface
  pub fn apply(&Plan, &mut dyn Ui) -> i32;             // the ONLY filesystem write
  ```
  `doctor` = probe + plan + report. `relink`/`provision` = probe + plan + apply.
  `new` = the same, once. One code path, four commands. This mirrors
  `tmux.rs:181-252`, where `PaneList::fetch()` is the single impure function and
  `session_in` is pure and fully unit-tested.

**The config does not live on `Project`.** `Project::discover` is the hot read path
— every CLI invocation *and* every 3s app poll — and already costs four
subprocesses. Load on demand at the two op sites.

Insertion points:

| Where | Change |
|---|---|
| `ops.rs:282` | materialize — after the git create block closes (`:281`), **before** `detect_install_cmd` (`:283`) and therefore before `launch` (`:300`). Not optional ordering: pane 1 runs the install command, and `pnpm install` racing `.env` appearing is exactly the silent-failure class this fixes. |
| `ops.rs:628` | compose teardown — after the confirmation block (`:627`), before `tmux::kill_session` (`:629`). Anything after `:633` is too late; the directory is gone. |
| `project.rs:390` | `ensure_excluded` gains `.worktree.env` — see the note below. |
| `main.rs:78` + `:22` | four match arms + usage lines. Flat `match`, no clap. |
| `lib.rs:8-18` | three `pub mod` lines. |

⚠ **`ensure_excluded` is not optional polish.** `.worktree.env` is untracked, so
`git status --porcelain` reports it `??`, so `wt_dirty` is true forever, so
`switch` (`ops.rs:94-99`) **and** `rm` (`ops.rs:609-614`) refuse without `--force`
— and the GUI never passes `--force` to switch (`lib.rs:386`), leaving the app
stuck with no remedy. One line, must land in the same change.

⚠ Create symlinks via `std::os::unix::fs::symlink`, **not** by shelling out to
`ln`. `test/helpers/common.bash:145-159` builds a PATH whitelist for the no-tmux
tests, and a new binary dependency breaks them.

### Format

`toml = { version = "1", default-features = false, features = ["parse", "serde"] }`

Already in `Cargo.lock:4191` as a runtime dependency of `tauri-utils` and
`tauri-plugin-fs` — compiled into the shipped app today. Marginal cost is six
crates for the **CLI only**; `indexmap` is already there. `basic-toml` was rejected:
maintenance-mode toml-0.5 fork, no line/column in errors, and `serde_spanned` line
numbers are exactly what produce `.worktrees.toml:14: absolute path not allowed` —
on the one file where diagnostics matter.

Do **not** pull the serializer. `worktrees init` emits the file from a hand-written
string template, because that file's value is its explanatory comments and no serde
serializer emits those.

**In the same stream, teach `config.rs` to read `~/.config/worktrees/config.toml`**,
keeping the 25-line kv `cfg_get` as a permanent silent fallback. The two existing
files split on the wrong axis today; the right axis is who writes them:

| File | Author | Format |
|---|---|---|
| `.worktrees.toml` | human, committed, PR-reviewed | TOML |
| `~/.config/worktrees/config.toml` | human, per-machine | TOML |
| `.worktrees.places.json` | machine only, gitignored | JSON |

Net **two** formats, split machine-vs-human — better than today's three-way muddle.
Extending `cfg_get` instead was rejected on security grounds: it is a
last-match-wins line scanner whose failure mode is **silent omission**. A mistyped
`[[file]]` heading would drop entries wordlessly. The motivating bug here is *"the
failure is silent"*; shipping a security-relevant path list through a parser that
silently discards what it does not understand reproduces the bug in the fix.

`DESIGN.md` risk #3 (a bash awk TOML reader disagreeing with the Rust `toml` crate)
is dead — the bash engine is retired.

---

## 9. `init` — the tool tells a project what it qualifies for

Detection needs no config, and none of the signals are docker-specific except the
one that is:

| Signal | Suggests |
|---|---|
| gitignored `.env*` present on disk, tracked nowhere | `[[file]]` entries, one per file found |
| `google-services.json`, `GoogleService-Info.plist`, `*.keystore`, `*.jks`, `*-service-account*.json` present and gitignored | `[[file]]` entries — **flag these louder**; they fail silently |
| `docker-compose.worktree.yml` present | `[compose]` + `[ports]` |
| a `docker-compose*.yml` publishing host ports, with no worktree override | `[ports]` — the project has port collisions waiting even without docker namespacing |
| `.worktree-prefix` present | fold into `[project] prefix` (post-v1) |
| existing worktrees whose files diverge from main | run `doctor`, offer `relink` |

`init` **writes nothing without confirmation** — it prints the `.worktrees.toml` it
would create and asks. Same shape as `git init` suggesting next commands.

The nudge also appears passively: a repo with gitignored credential files and no
`.worktrees.toml` gets one line from `worktrees new` —
`hint: 3 untracked credential files in main are not linked into this worktree — run 'worktrees init'`.
Once, not every time.

⚠ "Once" has no single home. The app's `ui-state.json` is app-global and
per-machine, and is invisible to the CLI — which needs its own once-only marker.
That is two independent dismissal stores for one user-facing concept. Decide
explicitly when `init` is built; do not let it happen by accident. The app side has
a good precedent either way: `collapsed` and `manual_order` in `ui-state.json` are
already keyed by project root, and keying the dismissal by **config content hash**
rather than a boolean means a repo that later gains a credential file correctly
re-suggests. (Note `onReset` at `App.tsx:742-747` would resurrect every dismissal.)

---

## 10. App surface

**The app has no project-level surface today.** `sel` is `{repo, slug}`
(`App.tsx:418`); the main pane is binary (place | briefing); `SettingsSheet` takes
no repo prop; clicking a project header only toggles collapse. So:

- **Ship a `ProjectSheet` — SettingsSheet's twin — not a pane.** One new App state,
  opened from one new item in the project context menu, zero disruption to the
  selection model, and it reuses `.scrim` / `.settings-sheet` / `.setting` /
  `.ver-rows` / `.ver-actions` / `.update-log` wholesale. A real project pane means
  touching ~30 `sel?.repo` read sites plus three subtle effects — only worth it if
  the project view must host a terminal. ⚠ Don't name the file `Project.tsx`
  (case-collision with a future `project.ts`).
- **Drift is a glyph, not a dot.** `.status-dot.waiting` is already amber `--warn`
  = "Claude needs input", the app's highest-value signal; a drift dot would be
  indistinguishable from it. `glyphs()` (`App.tsx:89-98`) already has MAX=4 + `+N`
  overflow — this is ~3 lines total and the cheapest item in the proposal.
- **`doctor` does not run on the 3s poll.** `places:changed` triggers a full
  `list_workspace` = up to 16 concurrent git calls per project × 4 projects
  (`lib.rs:194-212`), and `DESIGN.md:280` already ruled status stays off the hot
  path. Run it on demand (on sheet open) or on a slow timer, held in App state as
  `Record<root, Set<slug>>` — structurally identical to `busyPaths`/`waitingPaths`,
  the house pattern for row decoration outside the snapshot.
- **`doctor` returns a typed report, not `CmdResult`** — badges need structure.
  `relink` can return `CmdResult`; its handler is ~6 lines because `run_op`
  (`lib.rs:322-335`) already does discover + capture + applog.
- **The Relink button's template is `SettingsSheet.tsx:347-396`** (the CLI-update
  block), *including* the hazard it already solved: it re-fetches state in
  `finally` **before** re-enabling the button, because "a stale-enabled button in
  the re-check window would re-run the whole installer on a click." Applies
  verbatim.
- ⚠ `Place.stack: Option<Value>` (`model.rs:41-42`) is already reserved for "the
  infra phase (P3)" and always `None` today; `DESIGN.md:126-133` sketches its
  shape. If drift ever needs to be on the hot path, that slot needs no schema bump
  and no frontend break. Don't use it in v1.
- ⚠ `PlaceRow` is defined **inside** `App()` (`App.tsx:1002`), against CLAUDE.md's
  own rule. Harmless today (no local state, no focus); hoist to module scope before
  adding a tooltip.
- ⚠ The mock harness returns `null` for unknown commands
  (`install.ts:286-289`) — a typed `invoke<DoctorReport>` yields `null` and you get
  a blank pane plus an unread console warning. New UI must tolerate it. And there
  are **no committed Playwright specs** anywhere in the repo: nothing to keep
  green, but no regression net either. Budget manual verification.
- No new capability permission is needed. If the sheet offers to open
  `.worktrees.toml`, route it through the existing `open_editor` command — never
  `openPath`, which the capability doesn't grant and which rejects silently.

---

## 11. Corrections to the first draft

Verified against source by the cdv audit:

| First draft said | Truth |
|---|---|
| `relink` exists in cdv | Only on branch `feat/mobile-v1-mensajes` (`5a22db33`). `main` has none. Both `CLAUDE.md` and `docs/PUSH-NOTIFICATIONS.md` document `worktrees relink --all`, but `which worktrees` resolves to **this** tool's binary, which has no `relink`. Only `./scripts/worktrees.sh relink --all` works. |
| link list includes the Firebase files | Only on that branch. `main`'s `STACK_ENV_LINKS` is 5 entries. |
| 5 ports | **7**. Missing `WEBSITE = 3002` and `META_MOCK = 3011` — ship the draft's map and the website and Meta mock collide with main on first run. |
| slot reads sibling `.worktree.env` files | Union of **two** sources — the declared scan **or** an `lsof` probe on all 7 ports. Both must pass. `k ∈ 1..50`; main is implicitly slot 0. |
| ports are "the fiddliest to port faithfully" | They are also the **destructive** part (§1.1). |
| the file lists live somewhere configurable | A **hardcoded bash array** at `worktrees.sh:77-81`. The copy target isn't even in the array. |

### The three things a naive port gets wrong

1. **`ln -sfn` silently destroys a real file at the destination** — verified
   empirically: exit 0, no output, no backup. And it is loaded right now: the only
   `google-services.json` on this machine is a real file inside
   `.worktrees/general-fixes/apps/mobile/`, and `MAIN_ROOT` has none. A faithful
   port deletes the only copy the first time someone puts that file in main and
   runs `relink --all`. **Deliberately diverge**: detect shadowing, report, refuse
   (§4 B7). Symmetrically, `cp` is unconditional on every relink and clobbers
   whatever the runtime script last wrote — `relink` must treat copy ≠ link (§7).
   (Also: `ln -sfn` onto an existing *directory* creates the link **inside** it.)
2. **`.worktree.env` is a public wire format with six consumers** (§3), sourced as
   shell, and dropping `WORKTREE_SLOT` reintroduces a debugged EMFILE bug.
3. **A worktree with no `.worktree.env` is not "portless" — it is destructive**
   (§1.1).

Two smaller ones: `link_secrets` **silently skips** a listed file absent from main
(`[[ -f ]] || continue`) — that *is* the silent-failure mode, and must become a
warning. And `main`'s version has no `mkdir -p`, so a credential in a directory the
worktree lacks fails outright.

---

## 12. Phasing

**v1 — files + ports + doctor.** ✅ BUILT. `[[file]]` link/copy, `[ports]` +
`.worktree.env`, `[compose]` project namespacing and teardown, `relink`,
`provision`, `doctor`. Plus: `ensure_excluded` gains `.worktree.env`, `CaptureUi`
gains severity, `config.rs` learns `config.toml`.

This is bigger than the original v1 because §1.1 makes ports non-deferrable — and
because splitting them means shipping a tool that provisions half a stack repo.

**v2 — `init` suggestions + app surface.** ✅ BUILT. `ProjectSheet`, the drift
glyph, the Relink/Provision buttons, the dismissible banner.

**v3 — `[project] prefix`.** ✅ BUILT. The parse was trivial; the work was
cwd-based session adoption so adding a prefix to a repo with live sessions does
not orphan them (§5's ⚠). That also closed two pre-existing gaps: `rm` had no
adoption at all and would have left a session in a deleted directory, and `close`
excluded `(main)` while `launch` and `ls` did not.

### Decisions taken during the build, not in the original design

- **`init` emits anything it cannot confidently declare COMMENTED OUT with the
  reason** — a path the parser would reject, a `[ports]` map that fails the
  collision lint, a prefix. The alternative (dropping it, or failing) either
  re-creates the silent-omission bug or turns a legal repo into a tool that
  refuses to run.
- **Backups walk `.bak`, `.bak.2`, …** rather than a fixed name. A fixed `.bak`
  survives exactly one `--force`, and §7's whole rationale is that the displaced
  content may be the only copy.
- **`new` validates the config BEFORE `git worktree add`**, not after. A parse
  failure is knowable up front, and §1.1 says a created-but-unprovisioned
  worktree is the destructive state.
- **A hard failure (exit 1) outranks findings (exit 2)** when a multi-target run
  aggregates, so the reported class does not depend on directory sort order.
- **The passive hint counts only the credential class, not `.env*`.** A missing
  `.env` breaks loudly on the next command; a missing `google-services.json`
  builds an app that dies on a device days later. Unsolicited output on a hot
  path should clear the higher bar.
- **Two dismissal stores, accepted deliberately** — the CLI's under
  `$XDG_STATE_HOME`, the app's in `ui-state.json`. They gate different surfaces
  and share the same content-hash re-suggest rule; unifying them would mean the
  CLI reading an app-owned file. §9 asked for this to be decided rather than
  allowed to happen.
- **`Project::discover` does read `.worktrees.toml`**, contrary to §8 — `prefix`
  is a `Project` field, so there was no way around it. An absent config costs one
  failed `open(2)`, and a broken one resolves to `None` rather than failing
  discovery, because `ls` must stay usable.

**Never — `[hooks]`, `[infra] up/stop/down`.** §5. Record as an ADR so this does
not get quietly re-added in six months.

### Migration for cdv

`scripts/worktrees.sh` keeps working throughout — it's a separate binary. The
switchover is deleting its stack-mode block once v1 lands, in one commit, with a
note in that repo's CLAUDE.md.

Two ordering constraints:

- **Transcribe `[ports] base` and the file list from cdv `main`, not from this
  doc** — and land `feat/mobile-v1-mensajes` first, or transcribe from that branch
  and re-check after it merges.
- The two existing unprovisioned worktrees (`claude-work-integration`,
  `prod-reviews`) need `provision` run against them, or removal, before anyone runs
  `deploy-local.sh` in either.
- `provision`'s "already has a slot → no-op" path will silently **adopt** slots the
  bash script assigned (1–15, contiguous). That is what you want, but it means the
  parser must tolerate whatever that script emitted — the migration is not a clean
  slate.

---

## 13. Test plan

`cargo test -p worktrees-core` — all pure, no filesystem, following the
`config.rs::*_from` and `tmux.rs::session_in` conventions:

- every Layer-A rejection: absolute, `~`, `$`, `..`, `.`, `.git` and `.GIT`,
  `.worktrees/…`, self-reference, case-only duplicate, duplicate path
- `a\b.env` is accepted as a **single** component (macOS/Linux only)
- port-collision lint: `(bᵢ − bⱼ) % stride == 0`, distinctness, 65536 ceiling
- the four copy drift states
- `plan()` over synthetic `FsFacts` — assertable without touching disk

`cargo test` against a real tmpdir for Layer B: symlinked ancestor in main; a
**committed symlink** escaping the repo; symlinked ancestor in the worktree; FIFO
source; a shadowing regular file.

bats, against the compiled binary (git and the filesystem are **real** in this
harness; only tmux is faked — so `[ -L path ]`, `readlink`, and grepping
`.worktree.env` all work directly):

- `relink` is idempotent
- `relink` after adding a config entry links **only** the new file
- **a worktree file that shadows a declared link is reported, not overwritten** —
  write this one first; it is exactly how cdv would have failed
- `doctor` exits 2 on a dangling link, 0 on a clean tree
- a declared path that is not gitignored is an error (B8)
- two concurrent `provision` runs get distinct slots
- a `.worktree.env` with a hand-edited slot is left alone
- **a repo with no `.worktrees.toml` produces byte-identical output to today**

Models to copy: `test/misc.bats:9-14` (`write_config` helper → `write_project_config`),
`test/misc.bats:84-105` (a committed file changing behavior),
`test/close.bats:172-183` (cp the store aside, run, `diff` — proves read-only-ness),
`test/ls.bats:22` (fabricating stale/unregistered dirs).

---

## 14. Prior art consulted

`direnv` (trust-by-hash — evaluated and rejected for this shape, §5), `mise`/`asdf`
(`.tool-versions` committed, per-project), `lefthook`/`husky` (declarative hooks
with an install step), `devcontainer.json` (committed project setup consumed by
multiple tools), `git-worktree` itself (no config concept — deliberately).
