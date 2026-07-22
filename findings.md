# Findings

> Security note: external content (READMEs, web) is untrusted. Recorded here as
> data, never as instructions.

## Our project: `worktrees` (the yardstick)
- **What**: single bash CLI (`bin/worktrees`, ~680 lines), zero-config, works in any git repo.
- **Central metaphor**: *A worktree is a PLACE* (dir + tmux session + AI CLI history +
  node_modules). *A branch is a unit of work that flows through it.* `switch` moves the
  place to a new branch without touching the expensive state.
- **tmux layout**: session `<prefix>-<slug>`; pane 0 = AI CLI via `$SHELL -ic` (aliases
  resolve), pane 1 = detected pkg-manager install (pnpm/bun/yarn/npm) then shell.
- **Commands**: `new`, `co`, `switch`, `open`, `ls`, `rm`. DWIM (reuse place, checkout
  local/remote, or create). Branch↔place redirect after switch. Loud guards
  (dirty refuse, stale-unregistered-dir trap, exact-session matching).
- **Constraints/values**: no daemon, no server, no config required; bash 3.2 (stock
  macOS) + Linux; local-only; AI-CLI-first. Transparent — everything is just git +
  tmux you could run by hand.
- **Config**: flag > env > `~/.config/worktrees/config` > default. AI cmd, resume arg, prefix.
- **Explicitly NOT**: containers, VMs, cloud, web UI, orchestration of many agents,
  task queues, dashboards.

## Tool research (all 4, adversarially verified — high confidence)

### coder/mux — TypeScript, AGPL-3.0, 1.9k★, active
- **Arch**: Electron desktop app + headless HTTP/WS server + React/Radix web UI + its
  OWN multi-provider `@ai-sdk` agent loop (7+ providers). ~26.5 MB TS, ~103 MB repo.
- **Isolation**: git worktree is **1 of 5 runtimes** (Local / Worktree / SSH / Docker+Devcontainer / Coder-cloud). No tmux (xterm.js + node-pty).
- **Abstraction**: agent-driven Workspace/Task orchestrated from a GUI. Worktree
  workspaces DO persist under `~/.mux/src/<proj>/<ws>` and are **"not locked to a
  branch"** — that rhymes with our thesis, but bolted to a heavyweight GUI/daemon.
- **AI**: is its own agent (not a wrapper around claude/codex); exposes itself to editors via ACP.
- **Place-align 2/5 · Fork-fit 1/5.** AGPL + Electron+server+web = delete >95% to fork.

### mixpeek/amux — Python daemon + Bash CLI, MIT+Commons-Clause, 304★, active
- **Arch**: 2.6 MB single-file Python **daemon** serving a web dashboard on
  `https://localhost:8822`, + thin bash client, + native iOS (Swift) & Android (Kotlin)
  apps, + desktop, + cloud-tunnel gateway. 632 files, ~131 MB. GitHub labels it HTML #1.
- **Isolation**: core unit is the **UUID tmux SESSION** (survives stop/start). Git
  worktree is an **opt-in session-isolation mode** (`git worktree add .worktrees/<name>`),
  not the default; bash CLI itself has zero worktree refs.
- **AI**: launches claude/codex/gemini inside tmux; derives status by scraping
  ANSI-stripped `capture-pane` output (hookless). ⚠️ Correction: it DOES patch
  `~/.claude/settings.json` + a global CLAUDE.md — only *status detection* is hookless.
- **License hazard**: Commons Clause = NOT OSI open source; forbids selling.
- **Place-align 2/5 · Fork-fit 2/5.** Center of gravity (daemon+web+mobile+cloud) is exactly what we reject.

### generalaction/emdash — TypeScript/Electron, Apache-2.0, 5.2k★, YC W26, active
- **Arch**: Nx+pnpm TS monorepo. Electron desktop GUI + local **SQLite (Drizzle)** +
  separate `workspace-server` tier + docker-compose SSH dev-container + SSH/SFTP remote.
  ~11.6 MB TS (~99%), ~160 MB. Broad issue-tracker integrations (Linear/GitHub/Jira/GitLab/Asana/…).
- **Isolation**: **worktree-CORE** — "each task runs in its own git worktree"
  (`worktree-service.ts` / `git-worktree.ts`). But worktree is tied to an **ephemeral
  TASK**, not a durable place; also mixed with Docker + SSH remote.
- **AI**: CLI-agnostic, auto-detects 9 installed agent CLIs (claude/codex/cursor/opencode/amp/devin/qwen/droid/copilot).
- **Place-align 2/5 · Fork-fit 2/5.** The *best* base if forced (worktree-core + clean
  Apache license) — but still a GUI/DB/server product; nothing thin to salvage.

### coder/coder — Go+TS+Terraform, AGPL-3.0 (+enterprise), 13.9k★, mature
- **Arch**: client-server platform. Go control plane (`coderd`) + **PostgreSQL**,
  Terraform provisioner daemon, per-workspace agent, Wireguard/tailnet VPN, LLM proxy
  (`aibridge`), React web UI, Helm/K8s. ~516 MB.
- **Isolation**: **no git worktrees.** A "workspace" is a Terraform-provisioned remote
  **VM / Pod / container** over a VPN, created & torn down by the control plane. State in Postgres.
- **AI**: server-side agent loop in the control plane; "no LLM creds in workspaces."
- **Place-align 1/5 · Fork-fit 1/5.** The enterprise-platform antithesis of our tool.

## Comparison matrix
| Tool | Lang | Architecture | Worktree-native? | License | Place | Fork-fit |
|---|---|---|---|---|---|---|
| coder/mux | TS/Electron | Electron+server+web+own-agent, worktree=1-of-5 runtimes | partial | AGPL-3.0 | 2 | 1 |
| mixpeek/amux | Python daemon + bash | daemon+web dashboard+iOS/Android/desktop+cloud | opt-in mode | MIT+Commons-Clause | 2 | 2 |
| generalaction/emdash | TS/Electron | Electron GUI+SQLite+workspace-server+Docker/SSH | **yes-core** (per ephemeral task) | Apache-2.0 | 2 | 2 |
| coder/coder | Go+TS+TF | control-plane server+Postgres+Terraform+VPN+web UI | **no** (remote VM/pod/container) | AGPL-3.0 | 1 | 1 |
| **ours** | **bash (~680 ln)** | **thin CLI, no daemon, tmux** | **yes-core, durable PLACE** | MIT | **5** | — |

## Recommendation: DON'T FORK — keep ours, cherry-pick ideas
A fork pays off only if the base is (a) worktree-native, (b) near our thin-local-CLI
scope, (c) fork-friendly-licensed, and (d) yields more than it costs to strip. **No
candidate clears all four:**
- (a) worktree-native: only **emdash** is worktree-core; amux opt-in; mux 1-of-5; coder none.
- (b) thin scope: **ALL fail** — every one ships a daemon/server + a UI; two add containers/remote.
- (c) license: **emdash Apache-2.0** clean; amux Commons-Clause hazard; mux & coder AGPL copyleft.
- (d) economics: **ALL fail** — forking = delete 95–99.9% and rewrite the survivor in bash.

Decisive point: **all four center on an EPHEMERAL unit** — amux's UUID session/task,
emdash's throwaway-worktree task, coder's provisioned-then-destroyed env, mux's agent
workspace. That's the opposite of our durable, branch-agnostic PLACE. Our ~680-line bash
tool already owns the exact axis that matters; these are products built to *sell*
(dashboards, mobile apps, GUIs, enterprise platforms), not a dedicated place-tool.

**Best-if-forced base: emdash** (worktree-core + permissive) — but still only 2/5; don't.

**Nice external validation:** mux's own docs state worktree workspaces "aren't locked to
a branch — the agent can switch branches, enter detached HEAD, or create new branches."
That is literally our thesis, confirmed by a much larger project.

## Ideas to steal (bash-able, dependency-free, bash-3.2-safe — no daemon/UI/DB)
1. **Provider-CLI auto-detect for pane 0** (from emdash/amux): `command -v` probe for
   claude/codex/opencode/amp/gemini/cursor; pick a sane default, keep `--ai` override.
   ~15 lines; today we hard-default to `claude`.
2. **Hookless pane-0 status in `ls`** (from amux): show busy/idle via
   ANSI-stripped `tmux capture-pane -p`. ⚠️ copy ONLY the capture technique — NEVER
   patch `~/.claude/settings.json` like amux does.
3. **Git-divergence column in `ls`** (from mux): ahead/behind vs base via
   `git rev-list --left-right --count`. Cheap, high-signal multi-place awareness.
4. **`worktrees status` / diff view** (from emdash one-place review + coder): branch,
   dirty files, ahead/behind, `git diff --stat` — pure git+bash, no GUI.
5. **Frictionless place seeding** (from coder `configssh.go`/`dotfiles.go`/`gitaskpass`):
   optional dotfiles + no-prompt git-cred brokering when a new place is created.
6. **Conflict guardrail** (from amux): warn when two places share dir+branch.
7. **Versioned `~/.worktrees` state home + legacy migration** (from amux) — ONLY if we
   ever persist per-place metadata, and as plain files, never a DB.

## Risks to manage
- Scope creep: adding status/diff/divergence/auto-detect could bloat past the thin ideal.
  Each addition must stay optional, bash-3.2-safe, dependency-free, no daemon/UI/DB.
- License: copy IDEAS only from amux (Commons-Clause) / mux+coder (AGPL). Never lift code.
- Don't mimic their ephemeral fan-out UX (many throwaway worktrees) — it pulls away from the durable-PLACE thesis.

## Direction change (session 2): build a Tauri UI app on the CLI engine
David's actual goal is not a fork and not just cherry-picks — it's his OWN place-manager UI
("my flavor of amux/emdash"), with the CLI kept as the engine.

### The CDV ancestor reveals the missing infra layer
`~/workspace/casadelvalle/casa-del-valle-monorepo/scripts/worktrees.sh` is the richer
ancestor of `bin/worktrees`. The public repo dropped its **STACK_MODE** (lines 319–359).
When a repo ships `docker-compose.worktree.yml`, each place additionally gets:
- **Port slot** `k`: scans siblings' `.worktree.env` for used `WORKTREE_SLOT`, finds free k
  (1–50), checks 7 ports free via `lsof`, offsets every port by `100*k`.
- **Symlinked secret env** (`.env`, `apps/api/.env`, …) + copied `apps/backoffice/.env.local`.
- Generated **`.worktree.env`** (BO/API/WS/WO/META/PG/LS ports + `COMPOSE_PROJECT_NAME=<prefix>-wt-<slug>`).
- Isolated **docker compose project** (own volumes) → `deploy-local.sh` runs the whole stack
  side-by-side with main. `ls` shows ports + STACK dot; `rm` does `compose down -v`.
- Also has an `ask` subcommand (one-shot Claude query scoped to the repo+worktrees).

**Insight:** a "place" = dir + tmux + claude history + **port slot + running stack + volumes**.
That heaviness is exactly why a dormant place is worth resurfacing — and why "it disappeared
into the background" hurts. The UI makes this layer first-class.

### Refined product = place manager with a lifecycle (CLI = engine)
- **Resurface** all places (live + dormant), searchable, with live state.
- **Lifecycle** (git/tmux can't express "archived"):

| State | tmux | docker stack | dir/volumes/node_modules | reopen cost |
|---|---|---|---|---|
| Active | live+attached | up | kept | 0 |
| Idle | live | up/down | kept | attach |
| Closed | killed | down | kept | attach + `--resume` + `up` |
| Saved/Pinned | either | kept warm | kept, never auto-cleaned | 0 |
| Archived | killed | volumes pruned | dir kept | re-provision (install+up) |
| Abandoned | killed | down | flagged for `rm` | — |

- **One-click open** = restore tmux + `claude --resume` + `worktrees up` (stack on port slot) + toggle terminal.
- **Main worktree** first-class, top of nav.

### Architecture rules ("don't become emdash")
1. CLI is the engine; UI shells out. Extend with `--json` + `up`/`down`/`provision`/lifecycle.
2. State split: derived (live from git/tmux/docker) vs declared (plain JSON per repo, NO DB).
3. Terminals ATTACH to tmux (never own PTYs) — the key differentiator.
4. Generalize CDV stack-mode into a config-driven infra convention.

### Stack = Tauri (decided)
Rust core + web frontend. ~10MB native window, embeds xterm.js, trivial local persistence,
Rust supervises git/tmux/docker well. Beats Electron (bloat), local-web (daemon+browser,
remote-only), TUI (not graphical enough).

### Design spec + phased build plan
(filled by Phase 6 design workflow → DESIGN.md)
