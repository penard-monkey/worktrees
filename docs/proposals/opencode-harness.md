---
title: "Proposal — opencode as a harness"
---

# Proposal — opencode as a harness

**Status:** research and plan, 2026-09-29. Nothing built. This is the
sibling of [pi as a third harness](pi-harness.html), written in parallel.
It extends [Codex support](codex-support.html) and
[Codex plan usage](codex-usage.html). It keeps the rule of one live provider
per place.

**The shared shape lives in the pi doc.** The two lanes agreed on one
harness × model shape: the registry, `model_arg`, `ModelRef` /
`ModelOption` / `AgentChoice`, the per-harness adapter trait, validation,
persistence and the phase-1 "two → N" refactor. That shape is defined **once**,
in [pi-harness §2.3](pi-harness.html#shared-shape). This doc does not
restate it. It records only the rows that are specific to opencode (§2), and
it asks the adapter for **one capability the pi doc does not need** (§3.4):
talking to the agent through its own loopback API instead of through the pane.

**Evidence base:** opencode **1.18.30** (Homebrew, `/opt/homebrew/bin/opencode`),
run live in a throwaway tmux server (`tmux -L ocprobe`, killed afterwards)
against scratch git repos. All model calls went to the LM Studio host
(`http://<lm-studio-host>:1234/v1`, `qwen3.6-27b`), which was **reachable**
throughout. **No paid or hosted provider was used.** Every probe ran under
`XDG_CONFIG_HOME` / `XDG_DATA_HOME` / `XDG_STATE_HOME` / `XDG_CACHE_HOME`
pointed into `~/.cache/worktrees/worktrees/opencode-harness-research/`, and
`opencode debug paths` confirmed that opencode honoured all four. Nothing was
written under `~/.config/opencode`, `~/.local/share/opencode` or
`~/.local/state/opencode`, and no credentials file was read. Source claims
come from the `v1.18.30` tag of `github.com/sst/opencode`, cloned into the
same scratch dir.

Each claim carries its source:

- **[observed]**: run on this machine against 1.18.30.
- **[source]**: read in the 1.18.30 source tree or `--help`.
- **[docs]**: opencode.ai/docs, which is the weakest of the three and in places
  wrong for 1.18.30 (§3.1).
- **[inferred]**: my reasoning, not measured.

---

## 0. The answer in one screen

| Question | Short answer |
|---|---|
| Install | One native binary: Mach-O arm64, 216 MB, Bun-compiled **[observed]**. There is **no Node/nvm dependency at launch**. Plugins, however, trigger an npm install at runtime (§8). |
| Launch | `opencode -m <backend>/<model> --prompt "<BRIEF_OPENER>"` in the place dir. The opener **auto-submits** and the brief ran end to end **[observed]**. The brief never reaches argv; only the fixed opener does. |
| Pane identity | Under `sh -ic`, tmux reports **`opencode`** **[observed]**, so `provider::by_word` matches it exactly and the `node` bug from pi's phase 1 does not apply. |
| Resume | **Never `-c`.** `--continue` is scoped to the **repo**, not the directory: in worktree B it resumed worktree A's session, and the agent's `pwd` then returned **A** **[observed]**. Use `-s <id>`, where the id is the newest root session whose `directory` equals the place path (§1.3). |
| Second integration path | Real. The TUI runs an HTTP server **in process**. With `--port` it listens on loopback and exposes `/session/status`, `/permission`, `/tui/append-prompt`, `/tui/submit-prompt` and SSE `/event` **[observed]**. Without `--port`, 1.18.30 listens on **nothing** **[observed]**, contrary to the docs. |
| Activity | Busy/idle and pending permissions are **in memory only**, not in the DB **[source + observed]**. The honest sources are the API (§3) and, as a fallback, the pane footer. The SQLite DB can say "turn finished", but it cannot say "waiting on you". |
| Model list | `opencode models` (≈0.95 s) or the API's `/config/providers`. Both are opencode's **catalog**, taken from models.dev plus config, not what the endpoint serves. A dead host still lists its models **[observed]**. |
| MCP | Works. `opencode mcp add worktrees -- worktrees mcp` writes the **global** config non-interactively. `worktrees mcp` connected, and the model called `worktrees_list_places` via cwd discovery **[observed]**. |
| `send` | Typed text shows inline; a bracketed multi-line paste folds to `[Pasted ~41 lines]`. Enter submits, and the composer clearing plus `esc interrupt` confirms it **[observed]**. The API path `append-prompt` + `submit-prompt` also worked **[observed]**. |
| Waiting | `△ Permission required` modal, with **Allow once** highlighted. **Enter approves** **[observed]**. By default the build agent allows **every** tool except `external_directory` and `doom_loop`, so the modal is rare (§3.3). |
| Usage meter | No plan meter. `opencode stats` and `session.cost` give **local spend and tokens** only (§6). |
| Security | **A cloned repo executes code when opencode starts.** A repo `opencode.json` `mcp` command and `.opencode/plugin/*.js` both ran on a bare TUI launch with no prompt. `--pure` stops neither. `OPENCODE_DISABLE_PROJECT_CONFIG=1` stops only the MCP **[observed]**, and it also drops the repo's `AGENTS.md` **[source]**. This is the headline finding (§8). |
| Footprint | **≈1 GB RSS per TUI** (four sampled: 0.77–1.05 GB) **[observed]**. The TUI runs in the alternate screen with a tmux `history_size` of **0** **[observed]**. |

---

## 1. Launch and identity

### 1.1 What was run **[observed]**

```sh
sh -ic '. env.sh; opencode --prompt "Read .planning/brief.md and begin."; exec sh'
```

The global config's `model` pointed at `lmstudio/qwen3.6-27b`. The TUI opened
on a new session with the opener already submitted. It ran `Read
.planning/brief.md`, then `Read .`, and answered `PROBE-OK` in 41.5 s.

`opencode run -m lmstudio/qwen3.6-27b "…" </dev/null` also worked (`M-OK`).
The `-m backend/model` form therefore works from argv, as the shared shape
assumes.

Two launch traps:

- **`opencode run` reads stdin.** Started without `</dev/null` from a
  non-TTY, it hung for minutes with no output and no network connection
  **[observed]**. The TUI does not have this problem. Any scripted use of
  `run` (for example a future model probe) must close stdin.
- **Wrapping it in `bash -c` hides it.** A non-interactive `bash -c` does no
  job control, so opencode shared bash's process group and tmux reported
  `bash` **[observed]**. Worktrees' real launch shape is `sh -ic`, which
  reported `opencode`. Don't "simplify" that `-i` away.

### 1.2 Flags that matter **[source: `opencode --help`, 1.18.30]**

| Flag | Use |
|---|---|
| `-m, --model <backend>/<model>` | `model_arg`. Pass it on a fresh launch or an explicit switch, never on resume (shared rule). |
| `--prompt <text>` | The **fixed** `BRIEF_OPENER` only. It auto-submits. |
| `-s, --session <id>` | Resume. Worktrees derives the id (§1.3). |
| `-c, --continue` | **Never.** It is repo-scoped (§1.3). |
| `--port`, `--hostname` | Turn on the in-process API (§3). `--hostname` defaults to `127.0.0.1`. Worktrees never passes `--mdns`, which would default the hostname to `0.0.0.0`. |
| `--auto` | Auto-approve permissions. **Never passed.** |
| `--pure` | "Run without external plugins". It does **not** stop repo plugins (§8), so it must not be treated as a safety flag. |
| `[project]` positional | Not used. The pane's cwd is the place. |

Env, not argv: `OPENCODE_DISABLE_AUTOUPDATE=1` (worktrees does not want the
binary changing under a live lane) and `OPENCODE_SERVER_PASSWORD` (§3.2).

### 1.3 Registry row and resume

The shared registry row (see [pi §2.3](pi-harness.html#shared-shape)):

| Field | opencode |
|---|---|
| `id` / `match_word` | `opencode` / `opencode` |
| `sidecar_suffix` | `~agent~opencode` |
| `canonical_default` | `false` |
| `name_arg` | none. opencode has no session-name flag. |
| `model_arg` | `-m` (or `--model`), taking `backend/model` |
| resume (adapter method) | `-s <id>` from `session_for_dir(place)` |

Existing names stay byte-identical: `<canonical>` and `<canonical>~agent~codex`.

**Why not `-c`.** All worktrees of one repo share an opencode **project**
(the `project_id` in `opencode.db` is the same for every worktree of the repo
**[observed]**). `--continue` takes the newest root session **in the
project**:

1. `opencode --port 47112 -c` was started in worktree `proj-wt`.
2. It reopened the session last used in `proj` (main).
3. A prompt asking for `pwd` ran the bash tool, which returned
   `…/proj`, **the other worktree** **[observed; the tool part in the DB
   records `pwd` → `/…/proj`]**.

So an agent would edit a sibling place's files. The MCP server it spawned
would also resolve cwd discovery against that sibling, and the `from` of
every `report` would be wrong **[inferred from the directory-scoped
instance]**. That is the entire reason the place model exists, and `-c`
breaks it silently.

**The resume id.** `opencode session list --format json` lists the
project's sessions with `id`, `directory`, `title`, `time` and `parentID`
**[observed]**. Pick the newest entry where `directory == place path` and
`parentID` is absent. When there is none, launch fresh. This is the opencode
side of the shared `session_present(cwd)` gate. Its cost is one
opencode process start (≈1 s), paid only at launch, never in a poll.

---

## 2. Models as a first-class choice (opencode rows)

### 2.1 Where the list comes from **[observed]**

- `opencode models` prints `backend/model` lines. `--verbose` adds one JSON
  object per model (context/output limits, `cost`, capabilities,
  `status`), interleaved with the id lines, which makes it awkward to parse.
  `/config/providers` on the API returns clean JSON:
  `{providers:[{id, models:{…}}], default:{backend: model}}`.
- **It is a catalog, not a probe.** The LM Studio server serves ~16 models,
  and opencode listed the one declared in config **plus three it got from
  models.dev's `lmstudio` entry** (`openai/gpt-oss-20b`,
  `qwen/qwen3-30b-a3b-2507`, `qwen/qwen3-coder-30b`), none of which this host
  serves. With the host dead, the same four were still listed.
- **opencode ships a hosted backend that needs no sign-in**: `opencode/*`
  ("Zen"), for example `big-pickle` and several `*-free` models. Two calls a
  few minutes apart returned **different** free-model lists
  (`mimo-v2.5-free` in the first, `mimo-v2.6-flash-free` in the second). The
  catalog churns underneath you. It was never used here.
- A model id opencode does not know (`lmstudio/does-not-exist`) fails with
  an opaque `UnknownError` that carries only an `err_…` ref, not a named
  "model not found". A model the host serves but config does not declare was
  **not** tested; the attempt was killed to avoid loading a second model
  onto the shared LM Studio host.

### 2.2 How it maps to `ModelOption`

| `ModelOption` field | opencode source |
|---|---|
| `model` | `backend/model` from the catalog |
| `source` | `"opencode-models"` |
| `ready = false, reason = no_credentials` | Catalog backends that need a key opencode lacks. `opencode providers list` shows the credential count **[observed: 0]**. It reads opencode's auth file, so worktrees asks opencode and never opens that file. |
| `reason = endpoint_unreachable` | Worktrees' **own** `GET <baseURL>/models` with a short timeout, for `openai-compatible` backends. opencode does not check (a dead host took **63 s** of retries before `Cannot connect to API` **[observed]**). |
| `reason = not_served` | The catalog lists it and the endpoint does not serve it. That was three of four `lmstudio/*` rows here. |
| `meta` | `limit.context`, `limit.output`, `capabilities.reasoning` / `attachment`, and `cost` (**0** for local and free models) |
| `opencode/*` hosted models | Listed with a **"hosted by opencode"** caption, `ready` per its catalog `status`. Open question 1 (§12) asks whether to show them at all. |

Where the catalog is read: the API when a lane is running, otherwise a
cached `opencode models` run from a **neutral** directory. A place directory
must not be used, because the repo's `opencode.json` can add providers
(§8). The cache refreshes on the dialog's open, never in a poll.

### 2.3 The running model

- `session.model` in the DB and `GET /session/:id` both carry `{id,
  providerID}` **[observed]**.
- A `/models` switch in the TUI emits `ModelSwitched`, which rewrites
  `session.model` at once **[source: `core/session/projector.ts:337`]**.
  That is the same "the switch is written immediately" rule AGENTS.md
  records for Claude and Codex.
- Each assistant message also carries its own `providerID/modelID`
  **[observed]**.
- Label rule: `session.model`, which is already the newest. Read it from the
  API (§3). The DB is the fallback when no port is known.

---

## 3. Activity, send and model: the API path

### 3.1 What is really there **[observed]**

- The docs say the TUI "starts its own server on a randomized port". In
  1.18.30 a TUI started **without** `--port` has **no listening socket**;
  `lsof` showed only its outbound connections to LM Studio.
- With `--port 47111` it listens on `127.0.0.1:47111` inside the same
  process. There is no second process to supervise, and the server dies with
  the TUI, which dies with the tmux pane.
- Measured endpoints:

| Need | Endpoint | Seen |
|---|---|---|
| alive + version | `GET /global/health` | `{"healthy":true,"version":"1.18.30"}` |
| busy / idle | `GET /session/status` | `{"ses_…":{"type":"busy"}}`, then `{}` when idle (idle entries are deleted, `status.ts`) |
| waiting on you | `GET /permission` | the pending ask: permission, patterns, `tool.callID` |
| model / sessions | `GET /session` | `directory`, `model`, tokens, `cost`, `time` |
| send | `POST /tui/append-prompt` `{text}` then `POST /tui/submit-prompt` | text appeared in the composer, submitted, and was answered (`SEND-OK`) |
| edges | `GET /event` (SSE) | `session.status busy`, then `session.status idle`, then `session.idle`, plus `permission.*`, `message.*` |

- **The trap, in the API:** status lives in a per-**directory** instance. In
  the `-c` cross-directory case above, `/session/status` answered `{}`
  while the TUI showed `esc interrupt` and was mid-turn. A launch that uses
  `-s <id for this directory>` keeps the instance and the session on the same
  directory, which is one more reason §1.3 is not optional. After a submit
  there is also a short window where status is still `{}`, so `wait` must
  not take a single idle sample as final (the same rule as pi §3.1).

### 3.2 Exposure: what the port gives away **[observed]**

- **No `Host` check.** `curl -H 'Host: evil.example' …/session` returned
  **200**. That is the DNS-rebinding shape of GHSA-6pff-wf7m-6f5h, the
  advisory that retired `mo` here.
- **No CORS allowance** for a foreign `Origin`, so a web page cannot *read*
  responses. But `POST /tui/submit-prompt` takes no body, and a blind simple
  POST needs no preflight.
- **The surface includes** `/session/:id/shell` (run a command
  **[docs]**), permission replies (`POST
  /session/:id/permissions/:pid {"response":"reject"}` **[observed]**, and
  the same with `"once"` approves), and `PATCH /config`.
- **`OPENCODE_SERVER_PASSWORD` closes it.** With it set on the TUI:
  - unauthenticated requests got **401**;
  - authenticated ones got 200, including with a foreign `Host`;
  - the TUI kept working **[observed]**.

  A rebinding page cannot supply the credential.

**Rule:** worktrees never passes `--port` without also setting
`OPENCODE_SERVER_PASSWORD`:

- a random secret per launch, written 0600 into a worktrees-owned runtime
  file next to the message log in the git common dir;
- that file also holds `{port, pid, started}`, so the file is the whole
  runtime handle `{port, secret, pid, started}` (§3.4);
- it is passed in the pane's **environment**, never in argv;
- `--hostname` is always `127.0.0.1`.

A port with no password is the kind of loopback leak this repo already
tests against for its own docs server.

### 3.3 Which write sets which field

AGENTS.md asks this of every agent file. Here are the answers for opencode:

| Signal | Written by | Trust |
|---|---|---|
| busy / idle / retry | in-memory `SessionStatus` map; **never persisted** **[source: `session/status.ts`]** | API only. The DB cannot give it. |
| pending permission | in-memory; the `permission` **table** holds only "Allow always" decisions **[observed schema]** | API or pane only |
| turn finished | assistant `message.data.time.completed` + `finish` (`tool-calls` / `stop`) **[observed]** | DB-readable. `completed` is null for the whole turn, so it means "not finished", not "busy": a crashed TUI leaves it null forever **[inferred]**. |
| model | `session.model`, on switch **[source]** | DB or API |
| `session.time_updated` | also moved by an agent/model switch, not only by a turn **[source: projector]** | Do not date work by it. Date by the newest `message.time_created` (the transcript-mtime lesson). |

**Why not make the DB primary.**

- 38 migrations had been applied by 1.18.30, including
  `reset_v2_session_state` in June 2026 **[observed]**.
- A second harness's private schema is the most volatile contract on
  offer.
- It needs either `rusqlite` in `worktrees-core` (a new dependency class for
  a crate that pointedly has none) or shelling out to `sqlite3`.
- WAL reads work read-only **[observed: `?mode=ro` while a TUI wrote]**, but
  one probe, with two `opencode run` processes starting at once, failed with
  **`database is locked`** **[observed, once]**. A reader that adds
  contention to the agents' own writes is the wrong direction.

The API, by contrast, is versioned (OpenAPI spec at `/doc`) and is what
opencode's own SDK and web client use.

**Pane fallback** (no port known, or an adopted session):

| State | Screen |
|---|---|
| Busy | the footer's left segment reads `esc interrupt` |
| Idle | `ctrl+p commands` without `esc interrupt` |
| Waiting | `△ Permission required` above a row reading `Allow once   Allow always   Reject … enter confirm` |

Key on the **footer**, never on body text (the Codex rule).

**How often "waiting" happens.** The default build agent's permission list
starts with `{"permission":"*","action":"allow"}`. Only
`external_directory` and `doom_loop` ask **[observed: `opencode debug
agent build`]**. So an opencode lane edits files and runs bash with **no
approval at all**, like pi, unlike Claude and Codex defaults. The amber dot
will be rare, and when it shows up it matters.

### 3.4 The shared trait: one extra capability

The shared trait already lets `send` and `activity` be API calls. For
opencode the adapter also needs a **per-launch runtime handle**, `{port,
secret, pid, started}`, created by `launch_args` and read by `activity` / `send` /
`running_model`.

The proposed form is a `launch_env(choice) -> Vec<(String, String)>` beside
`launch_args`. It keeps the secret out of argv and lets pi and Codex return
an empty list. It is now part of [pi §2.3](pi-harness.html#shared-shape),
along with the runtime handle and the rule that resume is keyed by the place
directory, never the repo.

### 3.5 Recommendation

- **Phase 2: pane + resume.** Pane footer for the dot, `-s` resume, the DB
  read only for the model label. No port: this ships the harness with zero
  network surface.
- **Phase 3: the API.** `--port` + password, as above. The dot, the model,
  `send` confirmation and `wait` move to `/session/status`, `/permission`,
  `/session` and `append-prompt`/`submit-prompt`. This is where opencode is
  better than Codex: a submit is **acknowledged by the program**, not
  inferred from a screen.

The rule "terminals attach to tmux and never own shells" holds throughout:

- the TUI still lives in tmux;
- the server is inside that TUI, so its lifetime is the pane's;
- worktrees is a client of a process tmux owns.

**Rejected alternatives:**

- `opencode serve` per **project**: one server, many directories, and a
  crash takes every lane. It also puts lifecycle in worktrees' hands.
- A server per **user**: that is opencode's own web-app model, and it
  collides with the lanes' directory isolation.
- `serve` per place + `opencode attach` in tmux: two processes per lane at
  ~1 GB each, for no signal the in-process port lacks.

---

## 4. MCP and communication

### 4.1 Setup **[observed]**

- `opencode mcp add worktrees -- worktrees mcp` is **non-interactive** when a
  name and command are given. It writes the **global** config
  (`Global.Path.config`, a jsonc-preserving edit) and leaves the repo alone
  **[source: `cli/cmd/mcp.ts`]**. This is the opencode twin of `codex mcp
  add`: worktrees shells out to it and never edits opencode's config itself.
  Add `--mutations` the same way it is added for Claude and Codex.
- **Detection**: `opencode mcp list` **launches** each server to report
  `✓ connected`, which is the same trap AGENTS.md records for `claude mcp
  list`. Detection should read the resolved global config via `opencode
  debug config` (it prints the resolved JSON), not the health check.
  **[inferred; `debug config` was not diffed across cwd]**
- In a place, tools show up prefixed `worktrees_` (`worktrees_list_places`).
  cwd discovery found the scratch repo, and the call returned its places
  **[observed]**. With `-s` resume the MCP server's cwd is the place, so
  `report`'s `from` is correct. With `-c` it would not be (§1.3).
- **opencode also reads `~/.claude`**: CLAUDE.md, and the skills under
  `~/.claude/skills`, which appear as `external_directory` allow rules in the
  build agent **[observed]**. `OPENCODE_DISABLE_CLAUDE_CODE*` turns that off
  **[docs]**. Worktrees should **not** set it: a lane seeing the user's
  global instructions is what they would expect.

### 4.2 `send` **[observed]**

Pane path (phase 2), keeping `send_text_ok`'s one-line rule:

- `send-keys -l` of a 1,500-character line appeared **inline**, not folded,
  but only after a **>1 s render delay**: a capture at 1 s showed an empty
  composer. `submit_codex`'s settle-then-confirm loop suits this. A
  fixed-delay confirm would false-negative.
- Enter submits. Confirmation is the composer going empty plus the
  `esc interrupt` footer.
- A bracketed paste (`paste-buffer -p`) of 41 lines folds to `[Pasted ~41
  lines]`. `send` never pastes, so this only matters for Plan-tab-style
  pastes, and folding there is harmless.
- `/`, `@` and `!` at column 0 are commands, file refs and shell in opencode
  as well **[docs]**. The existing refusal and attribution prefix carry over
  unchanged.
- **Waiting = refuse.** Enter on the permission modal is **Allow once**
  (§3.3). `may_type` must treat it as Codex's approval modal is treated.

API path (phase 3): `append-prompt` + `submit-prompt` return `true`, and
the SSE `session.status busy` edge is the receipt. The one-line and prefix
rules stay: they protect the lane, not the transport.

**Claude↔opencode** goes over the MCP message log (`report` / `messages` /
`wait`), exactly like Codex. Claude's own `SendMessage` does not reach an
opencode lane.

---

## 5. UI and settings (opencode rows)

The surfaces themselves (harness segment + model select, "Switch agent…",
`default_agent`) are defined in [pi §2.4](pi-harness.html#surfaces).
opencode adds:

- **Mark:** opencode's square "O" glyph as a small monochrome mark, sourced
  as data like the other agent marks. The trademark question is the same
  one as for Codex.
- **Model select groups** by backend. `opencode/*` hosted models appear only
  if §12 open question 1 says so, captioned "hosted by opencode".
- **Settings → opencode:**
  - MCP setup (install/uninstall through `opencode mcp add/remove`);
  - "binary found / version";
  - the **project-code warning** (§8), which is a sentence, not a toggle;
  - the default model;
  - whether to use the API (phase 3, default on once it ships).
- **Terminal:** the TUI is alternate-screen with mouse capture, and tmux
  keeps **no history** for it **[observed: `alternate_on=1`,
  `history_size=0`]**. The dock's scrollback and ⌘F find nothing in an
  opencode pane, which is a visible difference from Claude. The conversation
  is in opencode's own UI (and `opencode export`).
- **Memory:** each TUI held **0.77–1.05 GB RSS** **[observed]**. Nine idle
  opencode lanes is roughly 9 GB. The dialog should not hide that. See
  §12 open question 4.

---

## 6. Account limits

| Backend | What is readable | Meter |
|---|---|---|
| LM Studio / any local endpoint | nothing to limit | **none** (legitimate) |
| `opencode/*` hosted (Zen) | the DB's `account` / `control_account` tables suggest an opencode account model **[observed schema]**; no local credential exists here, and nothing was queried | none in this plan |
| Paid API-key backends (Anthropic, OpenAI, …) | the provider's own billing, reached only with the key opencode holds | **rejected**, same as codex-usage: worktrees does not hold another tool's tokens |
| Any backend | `opencode stats` (sessions, tokens, **cost**, per-tool counts, `--days`, `--project`) and `session.cost` / `tokens_*` columns **[observed]** | an optional **spend** line (“$0.00 · 128K in”), per place from `GET /session`. This is **not** a limit and must not borrow the limit meter's bar. |

The usage adapter returns `None` for opencode in phases 2–3. A spend line is
phase 4 at most.

---

## 7. Availability

- **At creation:** worktrees' own `GET <baseURL>/models` probe (§2.2) marks
  the option `endpoint_unreachable`. Refuse, with a "launch anyway" button.
  **Decided 2026-09-29** for both harnesses (pi Question 5).
- **Mid-session:** with the host dead, opencode retried for **63 s**, then
  printed `Error: Cannot connect to API: Unable to connect. Is the computer
  able to access the url?` **[observed via `run`]**. In the TUI that minute
  looks like `busy`. The API reports `{"type":"retry", …}` status during
  retries **[source: `SessionStatusEvent`; not captured live]**. That gives
  the "stalled" dot pi §11 phase 4 wants for free, but only on the API
  path.
- **Global catalog at launch:** `opencode models` and the TUI fetch
  models.dev unless `OPENCODE_DISABLE_MODELS_FETCH` is set **[docs]**. On an
  offline machine the catalog is the cached one **[inferred]**.

---

## 8. Security and ADR 0001

**Worktrees' own provenance is fine.** The argv is `opencode -m <validated
model> --prompt "<fixed opener>" [-s <id opencode listed>] [--port <n>]`.
Every token is worktrees' constant, the user's choice, or opencode's own
data. `model` / `harness` join `USER_ONLY_KEYS` per the shared rule. A repo
cannot set them.

**opencode's provenance is not.** A cloned repo reaches opencode through its
committed files, and **opencode executes some of them at startup with no
prompt** **[observed, in a scratch repo `proj2`]**:

| Repo file | Launch | `--pure` | `OPENCODE_DISABLE_PROJECT_CONFIG=1` | + `OPENCODE_CONFIG_DIR=<empty>` |
|---|---|---|---|---|
| `opencode.json` → `mcp.evil.command = ["sh","-c","date > marker"]` | **ran** | **ran** | stopped | stopped |
| `.opencode/plugin/p.js` (top-level `writeFileSync`) | **ran** | **ran** | **ran** | **ran** |

- **The plugin row has no off switch in 1.18.30.** The v2 config path
  (`core/src/config.ts`) walks up from the directory for `.opencode` and
  `opencode.json` without consulting the flag. `config/plugin/external.ts`
  then globs `{plugin,plugins}/*.{ts,js}` and `import()`s each file
  **before** validating its shape, so the module body runs even if the
  plugin is malformed **[source]**.
- **The same path handles a `plugins` list in the repo's `opencode.json`**
  by `npm add`-ing and importing named packages **[source; not run]**.
- **opencode also `npm install`s `@opencode-ai/plugin` into every
  `.opencode` directory it finds.** In `proj2/.opencode` it wrote
  `node_modules/`, `package.json`, `package-lock.json` and a `.gitignore`
  that ignores them **[observed]**. That means network access and files in
  the place. The self-ignoring `.gitignore` keeps `dirty` clean, but the
  files are there.
- **Beyond executables,** a repo `opencode.json` can set `permission` (for
  example, allow everything), `provider` (point a backend at another URL),
  `instructions`, `agent`, `command` and `shell` **[docs]**. It can also use
  `{file:…}` / `{env:…}` substitution, so a repo config can read local files
  and env into its values **[docs]**.

**What this means for worktrees.** ADR 0001 is about **worktrees** never
turning repo content into argv, and this design keeps that. But the ADR's
reason is that "the hot path is `worktrees new` on a fresh clone, the moment
of least knowledge". Launching opencode there **is** executing the repo's
code, one step removed.

- Claude Code gates a project's `.mcp.json` behind a trust prompt.
- pi gates `.pi/extensions` behind one (pi §8).
- opencode 1.18.30 gates nothing.

**Proposal:**

1. **`OPENCODE_DISABLE_PROJECT_CONFIG=1` is not the fix.** It stops the
   repo's MCP commands, but it does not stop plugins (the table above). It
   also turns off the repo's **`AGENTS.md` / `CLAUDE.md`** loading:
   `session/instruction.ts:81,123` and `core/instruction-context.ts:48`
   both skip project instructions under the flag **[source]**. A lane
   started with it would not know the repo's conventions, which is most of
   what an agent in a place needs. Worktrees should not set it.
2. **Gate on the repo's opencode surface, not on a flag.** Before launching
   opencode in a place, worktrees reads (data, not argv) whether any
   directory from the place up to the repo root has an `opencode.json` /
   `opencode.jsonc` or a `.opencode/` directory.
   - **None:** launch normally. That is the common case, and `AGENTS.md`
     loads.
   - **Any:** refuse, unless the user has allowed that repo on this machine.
     The allowance is user-scoped, keyed by repo root, and stored in
     `~/.config/worktrees/config.toml`, the same provenance the ADR
     prescribes for `post_create`.

   The refusal names the files, and says plainly: "opencode will load this
   repo's configuration and may run its code as you, with no prompt of its
   own."

   **Scope.** Narrowing the check to "only executables" (`mcp`, `plugin(s)`,
   `.opencode/plugin*`) is tempting. But `provider` redirection,
   `permission` loosening and `{file:…}` substitution make the rest
   non-inert too, and the whole-surface rule is one `stat` walk.

   **Why this is not the ADR's rejected trust prompt.** The ADR rejects
   worktrees deciding whether to run the repo's argv. Here worktrees
   declines to be the one that starts a program which will. The user can
   still run `opencode` by hand.
3. **Never pass `--auto`.** Never write `trust`/permission config for
   opencode, and never set `permission` via `OPENCODE_CONFIG_CONTENT` to
   loosen it.
4. **Upstream:** ask opencode for a project-trust gate, separate from
   instructions: "load `AGENTS.md`, but not this repo's executable
   config". With that in place, item 2 becomes "pass the flag", and the
   check becomes a version check.

Worth stating plainly, as the pi doc does: opencode's default agent runs
bash with no approval (§3.3). Worktrees cannot make the repo's content safe.
Its job is to never be the channel through which a repo gets to execute.

---

## 9. Installation

- **One binary, zero toolchain knowledge: met, with one asterisk.** The
  Homebrew formula installs a 216 MB native executable **[observed]**.
  opencode also ships a curl installer and npm package **[docs]**. The
  preflight is only "is `opencode` on the pane's PATH, and which version".
  There is no Node floor to measure, unlike pi §9.
- **The asterisk:** plugins, including the `@opencode-ai/plugin` bootstrap
  it installs into the **global** config dir on first run **[observed:
  `cfg/opencode/node_modules`]**, are fetched with an npm client that
  opencode bundles. The user needs no Node, but first launch needs the
  network.
- **Version:** `opencode --version` → `1.18.30`. Pin fixtures (pane
  footers, API shapes) to the version and put it in `doctor`, as for Codex.
  The release cadence is fast (the probe binary is `1.18.30_2`, and the
  schema carries 38 migrations), so the pane fallback's strings are the
  part most likely to break.
- **The existing Homebrew install** is untouched and essentially unused (config
  = `$schema` only, DB last written 2026-05-15). The first real launch
  will run opencode's pending migrations on that DB. That is opencode's own
  data and expected, but worth knowing if the DB was being kept as-is.

---

## 10. Code map (opencode-specific additions)

This sits on top of the shared phase-1 refactor (pi §10–11):

- `provider.rs`: the `opencode` row (§1.3).
- A new `opencode.rs` in core, mirroring `codex.rs`:
  - `session_for_dir`: parses `session list --format json`;
  - footer classifiers with capture fixtures (`tests/fixtures/opencode-pane/`);
  - `launch_env`;
  - phase 3: a ~100-line loopback HTTP/1.1 client over `std::net`
    (basic auth, JSON, no SSE needed for polling), keeping core
    dependency-free.
- `mcpsetup.rs` / `codexmcp.rs` sibling: `opencode mcp add|remove`, and
  detection via `debug config`.
- `projcfg.rs` / config: the per-repo allowance (§8, item 2), which is
  user-only.
- `activity.rs`: an opencode reading in the N-ary `most_active`.
- Frontend: the mark, the Settings category, and the scrollback note.

---

## 11. Phased plan

**Phase 0: land this proposal** (docs only, beside pi's).

**Phase 1: shared "two → N".** Owned by [pi §11](pi-harness.html#phases).
Whichever harness lands first builds it. opencode's only ask of it is
`launch_env` (§3.4).

**Phase 2: opencode launches, resumes, shows a dot.**

- Registry row. Launch: `opencode -m <backend/model> --prompt
  "<BRIEF_OPENER>"`, env `OPENCODE_DISABLE_AUTOUPDATE=1`. Resume: `-s <id>`
  from `session_for_dir`, with no `-m`. Never `-c`.
- The repo-surface gate (§8, item 2), with bats tests for "refuses on
  `.opencode/plugin/x.js`", "refuses on a root `opencode.json`", "launches
  in a repo with neither" and "launches after the user allowance".
- Model catalog adapter plus worktrees' own reachability probe. Model label
  from the DB, read-only (`?mode=ro`, via the `sqlite3` CLI if present, else
  no label). MCP `create_worktree { provider: "opencode", model }`.
- Pane dot from footer fixtures. `send` = type, settle, Enter, and confirm
  from the pane; the permission modal means refuse.
- MCP setup in Settings and the CLI (`worktrees mcp --install --ai opencode`).
- `docs/opencode-manual-checks.md`, re-run on every opencode upgrade:
  1. launch;
  2. `-s` resume in two worktrees of one repo (the `-c` regression);
  3. the plugin gate;
  4. a dead host;
  5. the permission modal;
  6. MCP `report` `from`.

**Phase 3: the API channel.**

- `--port <free loopback port>` + a per-launch `OPENCODE_SERVER_PASSWORD`,
  with a runtime file in the git common dir.
- The dot, `wait` and the model move to the API, with the pane as fallback.
  `send` confirms by the program's own answer.
- Verify the handle before use: `/global/health` version and
  `GET /path` equals the place directory, so a port reused by another
  process can never be driven.
- Tests: a loopback fake server in the unit suite (the docs-server tests are
  the precedent). They include a **401 without the secret** assertion that
  must be shown to fail first.

**Phase 4: maybe.** The spend line (§6), the "stalled" dot from `retry`
status, and a hosted-model toggle.

---

## 12. Decisions and open questions

**Decided (2026-09-29, answered on the pi proposal):**

- **Trust default.** pi launches with `--no-approve` by default. The §8
  item 2 repo-surface gate is opencode's equivalent (refuse in a repo that
  carries opencode config until the user allows that repo). It follows from
  the pi answer and is to be confirmed when opencode is built.
- **Availability.** An unreachable model host at launch refuses, with a
  "launch anyway" button (§7).

**Open questions:**

1. **Hosted `opencode/*` models.** They need no sign-in and cost nothing
   today, but they send the place's code to opencode's service, and the
   list changes from call to call. Should the picker show them, hide them,
   or show them behind a setting?
2. **API in phase 2 or 3?** The API gives an exact send receipt, a real
   "waiting" state and retry visibility, at the cost of a password-protected
   loopback port per lane. My recommendation is phase 3, after the pane path
   works. Do you want it sooner?
3. **DB reads at all?** The model label in phase 2 needs either the
   `sqlite3` CLI (present on macOS; not guaranteed on Linux) or waiting for
   the API in phase 3. Is "no model label for opencode until phase 3"
   acceptable?
4. **Memory.** ~1 GB per opencode TUI. Should the dialog or `doctor` warn
   past N live opencode lanes, or is that the user's business?
5. **Scrollback.** opencode panes have no tmux history. Is that acceptable
   as a documented difference, or does it argue for the API-driven phase 3
   (where the app could render the transcript itself) sooner?
6. **Upstream asks.** Are you willing to file these with opencode:
   - a project-trust gate that is separate from instruction loading;
   - `--continue` scoped to the directory, or at least documented as
     project-wide;
   - `Host` validation on the server;
   - a named "unknown model" error;
   - `opencode models --json`?

   The first two are behaviour worktrees works around. The third is a
   security fix worth reporting on its own.

---

## 13. Probe record

- Scratch: `~/.cache/worktrees/worktrees/opencode-harness-research/`:
  - `env.sh`, the XDG overrides;
  - `cfg/` `data/` `state/` `cache/`, a throwaway opencode home containing
    the LM Studio provider and a `worktrees mcp` entry, read-only;
  - `proj/` (brief probe), `proj2/` (plugin + MCP execution probe);
  - `src-opencode/`, the `v1.18.30` source;
  - `cap1.txt`, `sse.txt`, `run1.json`, `dead.txt`.
- tmux server `ocprobe`: killed. All opencode processes had exited.
  `proj-wt` was removed.
- One stray `opencode run -m lmstudio/qwen3-coder-next` was started and
  killed within about a second, before any tokens came back; it may have
  asked LM Studio to load that model. No other model was requested.
- Nothing was written to `~/.config/opencode`, `~/.local/share/opencode`,
  `~/.local/state/opencode`, `~/.pi`, `~/.codex` or `~/.claude.json`.
