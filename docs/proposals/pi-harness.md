---
title: "Proposal — pi as a third harness"
---

# Proposal — pi as a third harness, and models as a first-class choice

**Status:** research and plan, 2026-09-29. Nothing built. This proposal
extends [Codex support](codex-support.html), which added a harness, and
[Codex plan usage](codex-usage.html), which covered a harness's account
limits. It does not change the one-live-provider-per-place rule.
**§2.3 is shared:** it is the one harness × model shape for every new
harness, agreed with the parallel opencode research lane, whose proposal
references it.

**Update, 2026-09-29 (after the probes):**
- Every observation here was made against pi **0.87.1** on 2026-09-29.
- Since then the managed install has self-updated to **0.99.1**, and the
  nvm-global copies listed in §9.5 have been removed.
- `~/.local/share/pi-node/current` now exists (→ `/opt/homebrew/opt/node`,
  Homebrew node 26.10), so pi's launcher pins that node. §9.1's "does not
  exist here" and its nvm-22.19.0 result therefore record the earlier state.
- Before phase 2: re-measure §9.1, and re-pin every fixture (list-models
  table, screens, JSONL shapes) against the pi version phase 2 is built on.

**Evidence base:** pi **0.87.1** (managed install), run live in throwaway tmux
servers (`tmux -L piprobe…`, killed afterwards) in a scratch git repo. All
model calls went to the `lm-studio` provider (`qwen3.6-27b` on the Tailscale
host), which was **reachable** throughout. **No Kimi request was made**; the
`kimi-coding` provider is not signed in on this machine (§2.2). Captures and
scratch files are in `~/.cache/worktrees/worktrees/pi-harness-research/`.
Nothing under `~/.pi/` was edited. `auth.json` was never opened. The only
files the probes created there are two session files for the scratch repo,
listed in §13 in case you want them gone.

Each claim is marked with its source:

- **[observed]**: I ran it and saw the result, on this machine, against 0.87.1.
- **[source]**: read in pi's own shipped docs, `--help`, installer script or
  compiled code.
- **[web]**: third-party pages; weakest, and cited.
- **[inferred]**: my reasoning, not measured.

---

## Decisions

Taken on 2026-09-29, after the first draft:

- **Q3, trust default: yes.** pi launches with `--no-approve` by default. A
  user setting can switch it to `ask`. Worktrees never passes `--approve`
  (§8).
- **Q5, unreachable host at launch: refuse, with "launch anyway".**
  - App: a "launch anyway" button.
  - CLI: `--force`.
  - MCP `create_worktree`: returns the reason and does not launch.
  - In every case the worktree and its brief are still created, so the lane
    can be launched later (§7).

---

## 0. The answer in one screen

| Question | Short answer |
|---|---|
| Launch | `pi --model <provider>/<id> "<BRIEF_OPENER>"` **[observed]**. The positional opener works, including behind a trust modal. Pin the model in argv, never rely on pi's default: on this machine the default is Kimi, and Kimi is **not signed in**. |
| Resume | `--session-id <id>` creates or reopens an **exact** session **[observed]**. That is better than Codex's `resume --last`. A resume must **omit** `--model`, because the session keeps its own model **[observed]**. |
| Pane identity | tmux reports **`node`** **[observed]**. Today `provider::for_pane` would **attribute a pi pane to Claude**. This bug must be fixed before anything else. |
| Model list | `pi --list-models` = the models pi can use **now** (only providers with credentials) **[observed]**. It is a text table with no JSON. LM Studio serves 16 models, but pi lists only the 1 declared in `models.json`; pi has no automatic discovery. |
| Activity | Derivable from session JSONL: the last `message` entry's `role` + `stopReason` **[observed]**. Two traps: the file doesn't exist until the first reply lands, and a failed request is retried as `error` + `context_edit`. |
| Waiting on user | pi has no approval prompts by design **[source]**. The one modal seen was the project-trust prompt, and its default choice is **Trust**. |
| MCP | **None in core, by design** **[source + pi.dev]**. The bus is reached through `worktrees` CLI verbs (new) and a worktrees-owned pi extension or skill passed with `-e`/`--skill`, never written into `~/.pi`. |
| `send` | Typed text (`send-keys -l`) is shown inline, not folded. Enter while busy **queues as "Steering"** and is delivered after the current message **[observed]**. Bracketed paste folds to `[paste #1 N chars]`. |
| Usage meter | No meter for pi in phase 2. Local models have no plan limit. Kimi has an endpoint, but reading it means worktrees holds the bearer token, which codex-usage already rejected. |
| Availability | A **dead** host is reported `ready` by `pi auth check` and `--list-models` **[observed]**. A blackholed host hangs `pi -p` for **>5 min with no output** **[observed]**. Worktrees must probe the endpoint itself, with a short timeout. |
| Node | Today a worktrees pane gets managed pi 0.87.1 on **nvm node 22.19.0**, exactly the floor, only because `.zshrc` loads nvm **[observed]**. Below the floor, 22.13 runs **silently** and 20.18 crashes loudly. Recommendation: one pi, installed by pi's own installer, plus a preflight that measures the node the **pane** will get (§9). No bundling. |

---

## 1. Launch and identity

### 1.1 What was run **[observed]**

The pane command matched what `ops::launch` builds for Claude and Codex:

```
exec "${SHELL:-/bin/sh}" -ic 'pi --model lm-studio/qwen3.6-27b; exec $SHELL'
```

- The TUI comes up in about 2s. Its footer is the cwd, then
  `<tokens> <ctx>%/128k (auto)` with the model id right-aligned
  (`cap-01-start.txt`).
- `pane_current_command` = **`node`**. `ps` shows the process as **`pi`**
  (pi sets `process.title`). The pane's `pane_start_command` still holds the
  whole `-ic` string, which contains `pi --model …`.
- The positional message (`pi "<text>"`) is submitted as the first user turn.
  With a project-trust modal up (§8), the message **waits** behind the modal
  and runs once it is answered (`cap-12/13`).
- Ctrl-D exits pi back to the `exec $SHELL` shell, and the pane then reads
  `zsh`. This is the same "outlives the program" shape as Codex.

### 1.2 Flags that matter **[source: `pi --help`, 0.87.1]**

| Need | Flag | Notes |
|---|---|---|
| pin provider+model | `--model <provider>/<id>[:<thinking>]` | `--provider` also exists. The `:<thinking>` suffix and `--thinking <level>` set the reasoning level. |
| exact session | `--session-id <id>` | **[observed]** Creates `<ts>_<id>.jsonl` or reopens it. Ids must match `[A-Za-z0-9._-]` and start and end alphanumeric **[observed error text]**, so `~agent~pi` cannot be used as an id. |
| most recent in cwd | `--continue` / `-c` | The equivalent of `codex resume --last`. |
| session storage | `--session-dir <dir>` | Env twin: `PI_CODING_AGENT_SESSION_DIR`. The CLI flag has the highest precedence, above a project's `sessionDir` **[source: sessions.md]**. §3.2, §8 and phase 2 rely on it. |
| display name | `--name <n>` | Session display name only. It is **not** a messaging address (pi has no cross-session bus). |
| trust | `--approve` / `--no-approve` | A per-run project-trust override (§8). |
| offline startup | `--offline` / `PI_OFFLINE=1` | Skips startup network work, such as catalog refresh. |
| extension / skill | `-e <path>`, `--skill <path>` | Load from an explicit path **without** writing pi's config (§4). |
| system prompt | `--append-system-prompt <text\|file>` | Could carry worktrees' bus instructions (§4). |
| config dir | `PI_CODING_AGENT_DIR` | Used by the probes to fake a dead host without touching `~/.pi`. |

### 1.3 Registry entry and session name

`Provider` gains `pi`:

```rust
Provider {
    id: "pi", label: "pi", match_word: "pi",
    resume_arg: /* see below */, sidecar_suffix: "~agent~pi",
    canonical_default: false, name_arg: None,
}
```

- **Session name:** `<canonical>~agent~pi`. Claude's and Codex's names stay
  byte-identical. A pi pane is never the canonical owner, following the Codex
  precedent.
- **Resume:** `resume_arg` is a static string today. For pi the right resume
  is **`--session-id <deterministic id>`**, not `--continue`:
  - The id is derived from the canonical session name, **sanitised to pi's
    id charset**, plus a generation counter.
    - `Project::session_name` only replaces `.` with `-`. The slug is
      `basename(worktree dir)`, which can hold any character except `/`, and
      main's slug is literally `(main)`. So `worktrees-(main)` is a real
      canonical name, and pi rejects it as an id **[observed error: ids
      must be `[A-Za-z0-9._-]` with alphanumeric ends]**.
    - Sanitising: map every character outside `[A-Za-z0-9._-]` to `-`,
      collapse runs, trim non-alphanumeric ends.
    - That can collapse two slugs into one id (`a b` and `a-b` both become
      `a-b`). To keep it unique per place, append a short hash of the
      **unsanitised** canonical name. Example: `worktrees-main-3f9a1c-g3`
      for `worktrees-(main)`, generation 3.
  - A fresh launch bumps the generation. Resume reuses the current one.
  - The generation lives in declared state (`.worktrees.places.json`) or in
    a worktrees-owned file. It is never inferred from pi's directory.

  This gives worktrees the one thing Codex never had: resume, activity and
  model detection all read **the same named file**, rather than "the newest
  rollout for this cwd" (the Codex sub-agent trap in AGENTS.md). This needs
  `resume_arg` to become a function (`fn resume_args(&self, ctx) -> Vec<String>`)
  or a pi branch in `ops::resume_command`. §10 has the code map.
- **Pane detection is broken for pi today** **[source: explorer read of
  `provider.rs::for_pane` + `tmux::is_ai_command`]**:
  - `for_pane("node")` finds no exact match.
  - It falls back to the `canonical_default` provider (Claude), whose
    heuristic accepts `node`.
  - So a pi pane is **read as Claude** by `agents_in` and `canonical_provider`.

  Fix: `for_pane` must consult `pane_start_command` (it contains `pi --model`)
  or the session-name suffix **before** the node heuristic. Keying on the
  sidecar suffix is the cheapest and matches how the app already names
  sessions. This is also a latent bug for any node-based harness launched
  through generic `--ai`.

---

## 2. Models as a first-class choice

### 2.1 Where pi's "available" list comes from

| Source | What it holds | Verdict |
|---|---|---|
| `~/.pi/agent/models.json` | User-declared custom providers (`lm-studio` → `qwen3.6-27b`). | Part of the answer, but raw. It says nothing about credentials, and it can hold `!command` keys **[source: models.md]**, so worktrees must never *evaluate* it. |
| `~/.pi/agent/models-store.json` | pi's cached **built-in catalog** (`kimi-coding`: `k3`, `k3-256k`, `kimi-for-coding`, `kimi-for-coding-highspeed`, plus `etag`/`checkedAt`) **[observed]**. | It is a cache, not a declaration of what the user can use. Don't read it. |
| `pi --list-models [search]` | Merged catalog **filtered to providers with credentials** **[observed: Kimi absent because not signed in]**. Takes 0.25s with `--offline`. | **Primary source.** It applies pi's own merge and readiness rules, so worktrees does not reimplement them. The cost is parsing a **text table** (`provider model context max-out thinking images`). No JSON form exists: `--mode json` does not change it **[observed]**. |
| `pi auth check --provider X --json --no-refresh` | `{"status":"ready"\|"not_ready","reason":…}` **[observed]**. | Useful to *explain* a missing provider ("Kimi is not signed in"). `--no-refresh` keeps it read-only. It never touches credentials unless asked with `--credentials`, which we never pass. |
| LM Studio `GET /v1/models` | **16** models on the host, including embeddings **[observed]**. | Not pi's list. pi uses only models declared in `models.json`; dynamic discovery requires a provider extension with `refreshModels` **[source: custom-provider.md]**. |

That last row is the direct answer to "that list will grow". **Adding a model
to LM Studio does not make it available to pi.** Someone has to add it to
`models.json`, or install a discovery extension. Worktrees should show the
gap instead of papering over it. In the model picker, the lm-studio group
would say: "16 served, 1 declared in pi's models.json". Worktrees must not
write `models.json` (the `~/.claude.json` rule: read someone else's live
config, never write it).

### 2.2 The default trap **[observed]**

`settings.json` sets `defaultProvider: kimi-coding`, `defaultModel:
kimi-for-coding`, while `pi auth check --provider kimi-coding` answers
`not_ready / credentials_not_configured`. **A plain `pi` in a lane starts on
a model that cannot answer.** Worktrees therefore always passes an explicit
`--model` on a *fresh* launch, and makes the user pick one when none has been
chosen. It never inherits pi's default silently.

A **resume** does the opposite. It omits `--model`, and pi reopened the
session on its recorded model (`lm-studio/qwen3.6-27b`), not the Kimi
default **[observed]**. Passing `--model` on resume would write a
`model_change` entry. That is correct only when the user actually asked to
switch.

<a id="shared-shape"></a>

### 2.3 Shared harness × model shape (canonical for every new harness)

**This section is shared.** It was agreed on 2026-09-29 with the
`opencode-harness-research` lane, which is researching opencode as a harness
in parallel. The opencode proposal references this section and records only
opencode-specific rows. Change the shape **here**, not in a harness doc.

Today the only model knob is `Profile.model` (Claude-only, `--model`,
`shell_quote`d; `profile.rs:248`, `:518`). Codex has none. The shape:

**1. `provider::Provider` stays the harness registry.**

- No rename. The prose says "harness" and the code keeps `Provider`, since
  renaming is churn with no payoff.
- New entries: `pi`, `opencode`. Sidecars: `~agent~pi`, `~agent~opencode`,
  with `canonical_default: false`. Existing names (`<canonical>`,
  `~agent~codex`) stay byte-identical.
- The registry does **not** grow a model dimension. A harness × model
  product in a static table would go stale the moment someone edits pi's
  `models.json`.

**2. One new registry field: `model_arg: Option<&'static str>`.**

- It is the only way a model reaches argv. Values:
  - Claude: `--model`
  - Codex: `-m`
  - pi: `--model`, taking `<backend>/<id>[:thinking]`
  - opencode: per its doc
- The adapter decides **when** to pass it:
  - on a fresh launch;
  - on an explicit model switch;
  - **never on resume**. pi reopens a session on its own model, and passing
    `--model` writes a `model_change` entry **[observed]**.
- `resume_arg: &str` becomes an adapter method. pi resumes with
  `--session-id <derived id>` (§1.3), which a static string cannot express.
- **Resume is keyed by the PLACE directory, never the repo.** opencode's
  `-c` is repo-scoped: in worktree B it resumed worktree A's session, whose
  bash then ran in A **[observed by the opencode lane]**. opencode therefore
  resumes with `-s <id>`, picked from `session list --format json` filtered
  by `directory == place`.
- pi is safe on both paths. `--continue` looks only in the cwd's session
  directory, and the `--session-id` worktrees derives comes from the place's
  canonical name: sanitised to pi's charset and made unique with a hash of
  the unsanitised name (§1.3).

**3. Value types (outside the registry):**

```text
ModelRef                          # a model, as the harness names it
  harness: string                 # provider.rs id
  backend: string | null          # pi/opencode "provider" (lm-studio, kimi-coding, anthropic…);
                                  #   null for claude/codex. Named `backend` because
                                  #   `provider` already means the harness in provider.rs.
  model:   string                 # qwen3.6-27b / opus / gpt-5-codex
  label:   string | null          # display only
  # serialises to the harness's own string: backend/model for pi and opencode

ModelOption                       # what a picker lists (derived, cached, recomputed)
  model:  ModelRef
  ready:  bool
  reason: "no_credentials" | "endpoint_unreachable" | "not_served" | null
                                  # no_credentials covers OAuth sign-in AND API keys
                                  #   (pi: auth check `credentials_not_configured`)
  source: string                  # "pi-list-models", "opencode-models", "claude-aliases", "user"
  meta:   { context?, max_out?, thinking?, images? }

AgentChoice                       # what a lane is launched with
  harness:  string
  model:    ModelRef | null       # null = the CLI's own default; NOT offered for pi (§2.2)
  thinking: string | null
```

`ready`/`reason` are not optional decoration. On this machine pi's default
provider is not signed in (§2.2), and pi reports a dead model host as ready
(§7). A picker that cannot say *why* a model is unusable offers exactly the
launches that fail.

**4. Per-harness adapter.**

- There is no trait today (§10). Phase 1 extracts one from the two existing
  harnesses **before** a third lands:
  - `launch_args(choice, fresh | resume)`
  - `launch_env(choice) -> Vec<(String, String)>`: so a per-launch secret
    never reaches argv. opencode needs `OPENCODE_SERVER_PASSWORD`; pi and
    Codex return `[]`.
  - A per-launch **runtime handle** (e.g. `{port, secret, pid, started}` for
    opencode) that `activity`, `send` and `running_model` read. It may be
    adapter-private, but the trait must not assume a launch is argv-only or
    stateless.
  - `session_for(panes, canonical)`
  - `activity(panes, canonical, path) -> Option<Activity>`
  - `session_present(cwd)`
  - `running_model(cwd)`: the label reader, separate from the catalog
  - `models() -> Vec<ModelOption>`
  - `usage() -> Option<…>`: `None` is legitimate, e.g. for local models
  - `send(pane, text) -> Delivery`: delivery **and** its confirmation belong
    to the harness.
    - Codex confirms from the screen (#352).
    - pi confirms from the JSONL user entry (§4.3).
    - opencode's TUI, **started with `--port`**, serves `/session/status`,
      `/permission`, `/tui/append-prompt` + `/tui/submit-prompt` and SSE
      `/event` in-process. Without `--port` it listens on nothing, whatever
      its docs say. It has no Host check (a foreign Host gets 200);
      `OPENCODE_SERVER_PASSWORD` turns unauthenticated requests into 401.
      **[verified on 1.18.30 by the opencode lane]**. So its `send` and
      `activity` are API calls rather than pane reads.
    - The trait must not assume the pane is the only channel.
- `activity::most_active` becomes N-ary.

**5. Validation.**

- A model string reaches argv `shell_quote`d, like `Profile.model` (the
  `profile.rs:2285` test), **and** it must match `[A-Za-z0-9._/:-]+`.
- The catalog is a UI convenience, not a gate. Free text is allowed, as with
  `--model` today.
- `model` and `harness` join `USER_ONLY_KEYS` (ADR 0001, §8). Values come
  only from the user, the app or an MCP call's validated data, never from
  repo files.

**6. Persistence.**

- The last-launched choice per place goes in `.worktrees.places.json` as
  a new `agent` field (`{harness, model}`) on `store::Declared`, which has
  no per-place provider field today. That is the user's machine
  file, not the repo.
- The per-harness default goes in Settings → Agents (§5).
- `Profile.model` keeps winning for Claude.

**7. Pane identity.**

- `for_pane` must prefer the session-name suffix / `pane_start_command` over
  the `node`/version heuristic. Today it attributes any `node` pane to
  Claude, and pi's pane reports `node` (§1.3).
- This is shared phase-1 work for every node-based harness.

**Generalising back to Claude and Codex:**

- Claude: a small static alias set (`opus`, `sonnet`, `haiku`, `fable`) plus
  free text. Its model list is not locally queryable **[inferred; not probed
  this lane]**.
- Codex: models named in `~/.codex` profiles plus free text **[inferred]**.
- Both keep `model: null` as "whatever the CLI defaults to", which is safe
  for them.

<a id="surfaces"></a>

### 2.4 Where the choice is exposed

| Surface | Today | Proposed |
|---|---|---|
| MCP `create_worktree` | `provider` enum from `provider::ids()` (`mcp.rs:852`) | `provider` gains `pi` automatically. Add an optional `model: "<backend>/<id>"` string, validated as data (same rule as above) and mapped to `--ai pi --model …`. Unknown or not-ready models return a **named** error listing `ready` options, so an orchestrator can pick one. No free-form argv. |
| CLI | `new/open --ai <cmd>` | `--ai pi --model lm-studio/qwen3.6-27b`. `--model` is also accepted for claude/codex. |
| New-worktree dialog | segmented Claude/Codex when both available (`App.tsx:1898`) | A harness segment, then a **model select** populated from `ModelOption`s, grouped by backend, with not-ready rows disabled and captioned with the reason. The segment appears only for harnesses that are installed. |
| Place ⋯ menu | "Switch to Claude/Codex" (`App.tsx:6823`) | "Switch agent…" opens a small sheet (harness + model) instead of N×M menu items. The confirm text names both the outgoing and incoming choice. |
| Settings → Agents | `default_provider` | `default_agent: AgentChoice`. A pi default must name a model. |

---

## 3. Activity and model detection

### 3.1 What the session file shows **[observed]**

Path: `~/.pi/agent/sessions/--<cwd with / → ->--/<ISO ts>_<session id>.jsonl`.
The mangling matches pi's docs and the brief's guess. With `--session-id`,
the file name ends in the id we chose.

A plain turn (`ls` via bash, then "DONE"):

```text
session                  cwd, version 3            (header, no id/parent)
model_change             provider lm-studio, modelId qwen3.6-27b
thinking_level_change    off
message system           (prompt sections + tool loadout)
message user
message assistant        stopReason toolUse   provider/model on the message
message toolResult
message assistant        stopReason stop
```

- **Busy:** the last *message* entry is `user` or `toolResult`, or an
  `assistant` with `stopReason: toolUse`. While tokens stream, **nothing is
  written**: mid-stream the file ended on the user message (`cap-04`).
- **Turn finished:** last message is `assistant` with `stop` (or `length`).
- **Esc:** `assistant` with `stopReason: aborted`, `errorMessage: "Operation
  aborted"` (`cap-05`). This is the analogue of Codex's `turn_aborted`.
- **Failed request:** each attempt writes `assistant stopReason: error`,
  followed by a `context_edit` that removes it from context. pi retried 3
  times with backoff, showing `── ⠼ Retrying (3/3) in 7s…` on screen. The
  **final** failure is an `assistant error` with **no** `context_edit` after
  it (`cap-11`).

  So:
  - "error, then context_edit" = still busy (retrying).
  - "error last" = the turn is over (idle, failed).
  - A reader that looks only at the last *line* reads `context_edit` and
    knows nothing.
- **Steering:** a message sent mid-turn landed as a separate `user` entry
  **2ms** after the `assistant stop` it followed (`16:07:08.845` →
  `.847`). A sampler between the two writes sees "idle" for 2ms. That is
  harmless for a dot, but `wait` must not treat one idle sample as final
  (§4.3).
- **Model label:** on every assistant message (`provider`, `model`), and in
  `model_change` entries. Per pi's docs, a `/model` switch writes
  `model_change` immediately **[source]**; I did not switch mid-session in
  the probe. Label = the newest of (last assistant `provider/model`, last
  `model_change`). This is the same rule AGENTS.md records for Claude and
  Codex.

### 3.2 "Which write set this field?", applied to pi

1. **The file does not exist until the first reply.** In both the TUI probe
   and `-p`, the session file appeared only when the first assistant message
   landed. The dead-host `-p` run wrote **no file in 5.7 minutes**. So the
   first turn of every fresh session is invisible in JSONL: "pi process
   alive + no file" covers both "idle before the first prompt" and "first
   turn in flight". Worktrees always sends the brief opener as the first
   turn, so a fresh lane is **busy in fact and blank in the file**. It needs
   a screen read (below) or the extension (§3.3).
2. **Non-message entries appear without a turn.** `usage` (idle
   prompt-cache warming: "runs only when the model declares a cache
   lifetime" **[source: settings.md]**; not seen with LM Studio), `label`,
   `session_info`, `custom` (extensions), `compaction`. The state machine
   keys on the newest **message** entry only. mtime is never activity: the
   AGENTS.md transcript rule applies unchanged.
3. **Trees, not logs.** Entries have `parentId`. `/tree` navigation branches
   *inside the same file*, so "the last line" is not necessarily on the
   active branch **[source: session-format.md]**. For a dot this is harmless
   (any append means something happened). For the model label, take the
   last *appended* assistant message, which is what the user just saw.
4. **A killed pi writes nothing.** This is inferred from the format: there
   is no shutdown entry. A file ending on `user` with the pane back at a
   shell means dead, not busy. Use the same `session_runs_program` check
   Codex uses ("not a shell", which also holds for `node`).
5. **A repo can move the session dir.** Project `.pi/settings.json`
   `sessionDir` is read **before** trust is decided **[source:
   security.md]**. Worktrees must not assume `~/.pi/agent/sessions` for a
   given place. Passing `--session-dir` explicitly would override it (the
   CLI has highest precedence). Alternatively, read the path pi reports.
   Recommend `--session-dir` pointing at pi's default computed dir, so the
   repo cannot redirect it.

### 3.3 Screen and extension signals

- **Screen:** busy shows a rule `── ⠙ Working ──…` directly above the
  composer; a retry shows `── ⠼ Retrying (n/3) in Ns… (escape to cancel)`
  (`cap-02/04/11`). As with Codex, key on the **rule line's position** (the
  line directly above the composer's top border), never on text anywhere in
  history.
- **Waiting (amber):** pi has **no permission prompts** by design **[source:
  pi.dev, security.md]**. The only modal seen was project trust (§8). An
  extension-provided `ask_question` tool exists in the ecosystem (`--help`
  example `--exclude-tools ask_question`) **[source]**; it is not in 0.87.1's
  built-in list. So pi's amber is *rare*: phase 2 can ship with busy / idle
  / afterglow only, and add amber when a real modal is captured.
- **The stronger option: a worktrees-owned pi extension.** It is loaded with
  `-e <worktrees data dir>/pi/worktrees.ts`, so nothing is written to
  `~/.pi`. pi's extension API has `agent_start`, `turn_end`,
  `agent_settled` ("final … Pi will not continue automatically") and
  `session_shutdown` **[source: extensions.md]**. The extension would write
  a small probe file owned by worktrees (`{pid, state, updatedAt,
  sessionFile, model}`), which is Claude's `sessions/<pid>.json` shape with
  one difference: **we are the writer**, so every field's meaning is ours.
  - This closes trap 1: `agent_start` fires before the first reply.
  - It gives an exact "settled" edge for `wait`.
  - The price: our code runs inside pi's process, and pi's extension API
    becomes a compatibility surface. pi moves fast (0.74 → 0.87 between the
    installs on this machine).

  Recommendation: JSONL + screen in phase 2, and the extension as the phase
  3 upgrade once the bus needs it anyway (§4).

---

## 4. MCP and communication

### 4.1 Does pi speak MCP?

**Not in core, deliberately.** Three checks agree:

- pi.dev: "Build CLI tools with READMEs (see Skills), or build an extension
  that adds MCP support" **[web: pi.dev]**.
- 0.87.1 ships no `mcp` subcommand or flag, and no MCP in `docs/`, README
  or CHANGELOG **[source]**.
- Its compiled `dist` mentions MCP only inside vendored provider SDKs
  (Anthropic/Gemini request types) **[source]**.

A third-party adapter exists: `pi install npm:pi-mcp-adapter`, which exposes
one proxy `mcp` tool **[web: github.com/nicobailon/pi-mcp-adapter; not
installed or tested]**. It reads **project `.mcp.json`**, so a cloned repo
can name MCP server commands for it to spawn. Recommending it would walk
straight into ADR 0001's territory (§8), so this proposal does not depend
on it.

### 4.2 How a pi agent joins the bus

`report` / `messages` / `wait` are thin over `worktrees_core::messages`.
`from` is derived from the server's own place, via `caller_place()`: the
deepest place containing the launch dir (`mcp.rs:1513`). The CLI has **no
equivalent verbs** today (explorer: `main.rs`). Proposal:

1. **CLI verbs, harness-neutral:** `worktrees msg report <text>`, `worktrees
   msg list [--ack]`, `worktrees msg wait [--for <place>] [--timeout N]`.
   - `list`, not `inbox`: `worktrees_core::inbox` already names the
     show-doc drop directory, and a second "inbox" would be ambiguous.
   - `from` is derived from **cwd** with the exact `caller_place` rule. It
     is never taken from an argument. This is the same trust property MCP
     has.
   - pi's `bash` tool runs in the place, so cwd is right.
   - This is useful beyond pi: any harness with a shell can join.
2. **Tell the agent the verbs exist**, without writing to `~/.pi`:
   - `--append-system-prompt <worktrees-owned file>` (a few lines: "you are
     in place X; report with `worktrees msg report`…"), or
   - `--skill <worktrees-owned skill dir>` (a `worktrees-bus` skill, loaded
     on demand, which keeps it out of every prompt).

   Recommend the skill: pi implements Agent Skills **[source]**, and
   `skillstore.rs` already owns skills as data. The launch argv comes from
   worktrees (user scope), never from the repo.
3. **Phase 3, optional:** the extension from §3.3 registers `report` /
   `messages` / `wait` as real tools (`pi.registerTool`) that call
   `worktrees_core` via the CLI. This gives native tool calls instead of
   bash, at the same compatibility cost as above.

Claude↔Claude keeps Claude's own messaging. pi has none, so a pi lane is
reachable only through the log (and through `send`).

### 4.3 `send` to a pi pane **[observed]**

`send` is Codex-or-Claude today (`mcp.rs:1616-1712`). The Codex path types
with `send_literal`, waits for `composer_settled`, presses Enter at most 3
times, and confirms by the composer clearing (#352). #352 existed because a
screen layout was **assumed**, so here is what pi actually does:

| Probe | Result |
|---|---|
| 3150-char message via `send-keys -l`, idle | Shown **inline**, wrapped across the composer, not folded. One Enter submitted it intact: the JSONL user message was 3150 chars (`cap-06`). |
| 2401-char message via `paste-buffer -p` (bracketed) | Folded to **`[paste #1 2401 chars]`** (`cap-07`). The Codex parser's `[Pasted Content N chars]` would **not** match it. |
| Enter while busy | Accepted and shown as **`Steering: <text>`** with `↳ Option+Up to edit all queued messages`. Delivered after the current assistant message as its own user turn; the model answered it (`cap-08/09`). |
| Ctrl-C with text in composer | Clears the composer (banner: "ctrl+c/ctrl+d clear/exit"). A **second** Ctrl-D exits pi. Never send control keys as "cleanup". |

Consequences:

- pi needs its **own** `composer_on_screen` / `composer_submitted` with pi
  fixtures (`tests/fixtures/pi-send/`), pinned to a version, as the Codex
  ones are pinned to 0.157.1.
- **Confirmation is cheaper than for Codex:** pi appends the user message to
  the session file on submit (after the first turn, per §3.2). "The JSONL
  gained a user entry whose text starts with our attributed header" is a
  delivery proof that does not read the screen at all. Fall back to the
  screen only for the first turn of a fresh session.
- **Mid-turn sends are safe to allow.** pi queues them as steering and does
  not interleave keystrokes into a modal, because it has none except trust.
  `may_type` must still refuse when the trust modal is up (§8), because
  Enter there selects **Trust**.

---

## 5. UI and settings

- **Agent mark per row:** a pi glyph beside `ClaudeMark` / `OpenAIMark`
  (`App.tsx:783`). The **model** is the more useful label for pi. The row's
  agent label reads `pi · qwen3.6-27b`, from §3.1's rule.
- **Harness × model picker:** in the new-worktree dialog and the "Switch
  agent…" sheet (§2.4). One component, fed by `ModelOption[]`:
  - grouped by backend
  - not-ready rows disabled with the reason
  - a free-text "other model" row per harness
  - a "declared in pi: 1 of 16 served" hint for OpenAI-compatible backends
    (§2.1)
- **Settings → Agents:** `default_agent` (harness + model), plus a **pi**
  category mirroring the Codex one: the resolved pi path and version, the
  node the **pane** will use (§9), and per-backend readiness. No
  "install MCP for pi" panel, because there is no MCP (§4).
- **The two-provider assumptions to unwind:** the union `"claude" | "codex"`
  is repeated in `App.tsx`, `settings.ts`, `SettingsSheet.tsx` and
  `planUsage.ts`. `requestOpenAgent` treats "from" as "the other one".
  `agent_sessions: {claude, codex}` is a fixed record in the snapshot
  (`lib.rs:276`). Phase 1 turns these into one shared `Harness` type and a
  keyed map. That is mechanical, but it touches ~100 sites (explorer
  count), so it deserves its own PR before pi appears in any of them.

---

## 6. Account limits

| Backend | What is honestly showable | Proposal |
|---|---|---|
| Local OpenAI-compatible (LM Studio) | Nothing plan-shaped: no quota, no window. Context use is per-session and already in pi's own footer (`3.6%/128k`). | **No meter.** Reachability belongs to §7, not to the usage meter. |
| `kimi-coding` | Kimi exposes `GET https://api.kimi.com/coding/v1/usages` with 5h and 7d windows **[web; not called]**, and upstream reports its `used_ratio` can contradict a 403 **[web: MoonshotAI/kimi-code#3951]**. Reading it needs the bearer token, via `pi auth print-bearer-token`. | **Not in phase 2.** codex-usage rejected "direct HTTP with tokens from auth storage" (worktrees would own credentials and an undocumented contract). The same reasoning applies, and here the endpoint's data is known to be wrong. Revisit only if pi itself exposes usage (e.g. an RPC/`auth` subcommand that returns windows, not tokens). |
| Other pi backends (Anthropic, OpenAI, …) | pi can use the *same* Claude or ChatGPT subscriptions (`openai-codex` bearer tokens appear in `--help`). | Out of scope. If a pi lane runs on the ChatGPT account, the existing Codex meter already shows that account. Do not double-count. |

`codex-usage`'s rule stands: a provider with no reading gets a named
placeholder, never a fabricated zero. For pi, the placeholder is simply not
rendered.

---

## 7. Availability (the remote LM Studio host)

Observed:

- **`pi auth check` and `--list-models` report a dead host as `ready`.**
  They check configuration, not reachability. Same result for a blackholed
  Tailscale address and a refused port.
- **Refused port:** 3 retries with backoff (~18s), then `Error: Retry failed
  after 3 attempts: Connection error.`, and the pane is usable again.
- **Blackholed address** (what an offline Tailscale peer looks like): `pi -p`
  produced **no output and no session file for 5 min 41 s**, until I killed
  it. `curl -m 5` gives up at 5s. In the TUI this would be a `Working`
  spinner with nothing in the JSONL **[inferred from the two observations]**.

So worktrees must do its own check:

- **At lane creation / launch / switch**, when the chosen backend is an
  OpenAI-compatible `baseUrl`:
  - Read the URL from `models.json` (read-only, no `!command` evaluation).
  - `GET <baseUrl>/models` with a **2–3s** timeout, from the app/CLI
    process, never from the pane.
  - Result: `reachable` / `unreachable` / `model not served`, where the
    declared id is missing from the server's list (useful on its own: it
    catches a model unloaded in LM Studio).
  - Unreachable: refuse the launch with a named reason, and offer "launch
    anyway" in the app, or `--force` on the CLI. MCP `create_worktree`
    returns the reason and does not launch; creating the worktree itself
    still succeeds.
  - Only the model call is gated. The place and brief are unaffected, so the
    lane can be launched later.
- **Mid-session:** detect a turn with no JSONL progress for N minutes plus a
  failing reachability probe, and paint the row's dot as **stalled** (a new
  state: amber-ish, "model host unreachable"), not busy.
  - The existing poll already re-lists every 30s. The probe is cached per
    `baseUrl` for 60s and shared across lanes.
  - `wait` on such a place returns `stalled` with the reason, instead of
    blocking to its cap.
- **Picker:** the lm-studio group shows reachability beside readiness, so
  "we're about to spin up a session — what about pi with this model?" is
  answered before the launch, not after a 5-minute spinner.

---

## 8. Security and ADR 0001

**Where pi's choice may come from:** the flag, `$WORKTREES_*` env,
`~/.config/worktrees/config.toml`, app settings, or MCP `create_worktree`'s
`model` argument (validated data from the caller's own session). Never
`.worktrees.toml`.

- `model` / `harness` join `USER_ONLY_KEYS` in `projcfg.rs`, so a repo
  setting them is a **hard parse error**, as `ai_cmd` is. A model id looks
  inert, but for pi it selects a provider, and a provider can carry an
  `apiKey: "!command"` **[source: models.md]**. The user wrote that
  command, but the repo would be choosing *when* it runs. That is "the repo
  selects among my commands", which the ADR permits for `install_cmd` only
  as a closed set of literals. Keep it user-only; it costs nothing.

**pi's own repo-supplied surface is bigger than Claude's or Codex's**
**[source: security.md]**:

- Project `.pi/settings.json`, `.pi/extensions` (executable TypeScript),
  `.pi/skills`, `.pi/SYSTEM.md`, and **`.agents/skills` in any ancestor**
  all trigger a project-trust decision.
- `sessionDir` is read **before** trust.
- The trust modal's highlighted default is **Trust** **[observed, `cap-12`]**.
  One stray Enter trusts the repo and lets it run extensions in pi's
  process.
- `agentfiles.rs` steers repos toward `.agents/skills` (for Codex), so
  **this repo's own recommended layout will raise pi's trust prompt in
  every lane.**

Proposal:

1. Worktrees launches pi with **`--no-approve` by default**: project
   resources are skipped for this run, and pi's context files
   (`AGENTS.md`/`CLAUDE.md`) still load, which is what worktrees relies on.
   A user setting (`pi_project_trust: "never" | "ask"`) can switch to `ask`
   for people who want repo skills. Worktrees never passes `--approve`, and
   never writes `trust.json`.
2. `send` and `may_type` treat the trust modal as **waiting**. Its footer is
   `↑↓ navigate  enter select  escape/ctrl+c cancel` under a `Trust project
   folder?` heading. Enter is never pressed into it.
3. `--session-dir <pi default dir>` is passed explicitly, so a repo's
   `sessionDir` cannot redirect where worktrees reads activity (§3.2).
4. The worktrees extension/skill (§3.3, §4.2) is loaded by **path from
   worktrees' own data dir**, the same provenance as the brief opener. It is
   never installed into `~/.pi`, and never read from the repo.

Worth saying plainly: pi itself states it has no sandbox and no per-tool
approval **[source: security.md]**. Worktrees' contribution is not making
the repo's content safe. It is making sure worktrees never becomes the
channel through which a repo gets to *execute*.

---

## 9. Which pi, on which node

### 9.1 What a pane actually gets today **[observed]**

The real tmux server's global PATH (`tmux show-environment -g PATH`) has
**neither Homebrew nor nvm** on it. Panes run `zsh -ic`, so `.zshrc` runs:

- It prepends `/opt/homebrew/bin`.
- Then it sources `nvm.sh`, which activates `alias/default` = `22`, the
  highest installed 22.x.

Measured with a throwaway server started with that exact PATH:

```text
command -v pi    → ~/.local/bin/pi            (managed 0.87.1)
command -v node  → ~/.nvm/versions/node/v22.19.0/bin/node
node -v          → v22.19.0                   (exactly the engine floor)
```

So today's lanes work because nvm's default happens to resolve to 22.19.0.
The same pi in my probe server, which inherited this tool's environment,
ran on **Homebrew node 26.10.0** (`lsof` on the pi process). **The node is
whatever the environment and rc files leave first on PATH.** pi's launcher
pins nothing:

- It prepends `~/.local/share/pi-node/current/bin` only if that exists. It
  does not exist here.
- The installer checks `node >= 22.19` only **at install time**. It
  downloads its own standalone node **only when there is no node and no
  Homebrew** **[source: install.sh `run_preflight_checks`,
  `detect_node_install_method`]**.

The app's `fixup_gui_path()` only changes how the app finds binaries; the
pane still runs the user's `zsh -ic`, so the rc wins there too. The CLI path
is identical, because the same pane command is built.

### 9.2 Below the floor **[observed]**

| node | `pi --version` | real prompt with a bash tool call |
|---|---|---|
| 22.13.0 | `0.87.1`, exit 0 | **worked, no warning** (it reported `v22.13.0` from its own bash tool) |
| 20.18.0 | crash: `SyntaxError: … 'node:module' does not provide an export named 'enableCompileCache'` | — |

22.13 failing **silently** is the dangerous case. It is below a floor pi
declares, so something will break eventually, and the only symptom will be
a mysterious bug.

### 9.3 For comparison

- Claude: bare word inside `sh -ic`, so PATH decides. It is a native binary,
  so the node question does not arise.
- Codex: `codex_bin()`, which tries PATH and then falls back to the
  VS Code/Cursor extension's bundled binary.
- Neither is bundled by worktrees.

### 9.4 Recommendation: one pi, zero Node knowledge

| Option | Verdict |
|---|---|
| (a) Bundle our own pi + node | **No.** pi 0.87.1's `node_modules` + a node runtime is on the order of 100+ MB. Worktrees would own pi's upgrade cadence (0.74 → 0.87 on this machine within weeks) and diverge from the pi the user runs by hand, including its sessions, auth and extensions. A second pi is exactly the confusion this lane started from. |
| (b) Launch an absolute node + pi's entry point | **No.** It couples worktrees to pi's `releases-v1` install layout and bypasses `pi update`'s own switch. |
| (c) A user setting for the pi path | **Not needed** once there is one pi. Keep a hidden env override (`WORKTREES_PI_BIN`) for development only. No install picker: the target is one pi, not a choice of installs. |
| (d) Preflight that measures the pane's node, and refuses below the floor | **Yes.** |
| (e) Point users at pi's own installer, and make it pin node | **Yes, as the install story.** |

The end-user path:

1. **Install:** the app's pi category shows `curl -fsSL https://pi.dev/install.sh | sh`
   (shown, never run, like `CodexInstallDialog`). On a Mac without a
   suitable node, the installer offers Homebrew or standalone node itself.
   The user never touches nvm.
2. **Preflight, before every launch (cached 10 min):**
   - Run the pane's own shell the way the pane will: `$SHELL -ic 'command -v
     pi; pi --version; node -p process.versions.node'`, with a 5s deadline.
   - Record `pi path / pi version / node path / node version`, and show it in
     Settings → pi and in `worktrees doctor`.
   - **Refuse below `engines.node`**, read from pi's own `package.json` next
     to the resolved binary rather than hard-coded. The message is
     actionable: "pi 0.87.1 needs node ≥ 22.19; your shell gives 22.13 from
     nvm. Run `nvm alias default 22.19` or re-run pi's installer."
3. **Ask upstream:** the launcher should prefer `pi-node` whenever the
   installer *chose* it, and refuse below the engine floor at **launch**. A
   one-line check in their launcher would make (d) a belt rather than the
   only guard. This goes in the open questions, not a dependency.

### 9.5 Cleanup note (machine-specific; the first item is done, see Status)

- Remove the two nvm-global pis, so `~/.local/bin/pi` (managed 0.87.1) is
  the only one:
  - `nvm exec 22.13.0 npm uninstall -g @earendil-works/pi-coding-agent pi-provider-kimi-code`
  - `nvm exec 26.8.1 npm uninstall -g @earendil-works/pi-coding-agent`

  0.87.1 gets `kimi-coding` from its built-in catalog (`models-store.json`),
  so the old plugin is not needed.
- Make the pane's node explicit: `nvm alias default 22.19.0` (or newer).
  `22` works today only because 22.19.0 is the highest 22.x installed.
  Consider removing 22.13.0/22.13.1, which are below pi's floor.
- `settings.json`'s default is `kimi-coding`, which is **not signed in**.
  Either `/login` to Kimi, or set the default to `lm-studio/qwen3.6-27b`.
  Worktrees will pass `--model` either way, but a hand-run `pi` currently
  starts on a model that cannot answer.
- Probe leftovers under `~/.pi/agent/sessions/--<mangled path of the scratch repo>--/`
  (2 files) are safe to delete.

---

## 10. Code map (what a third harness touches)

From a read-only survey of the current tree (line numbers as of `91b5ed0`):

- **Registry-driven, gets pi for free:** `mcp.rs:852` (provider enum),
  `mcp.rs:1238`, `ops.rs:208,220,943`, `project.rs:313`,
  `tmux.rs:204,218,245`.
- **Hard-coded two-provider logic in core (~12 sites):**
  - `ops.rs:99,110` (codex flags, claude-only profiles)
  - `ops.rs:724,923` (resume gate `!= "codex" || session_present`)
  - `profile.rs:542,576,664`, `config.rs:187,242,269`
  - `tmux.rs:21-32,198-206`
  - `activity.rs:250-298` (`most_active` takes **exactly two** readings,
    so it becomes N-ary)
  - `agent.rs:200`
- **MCP (~7):** `mcp.rs:72-97` (agent status), `:224,338` (setup dispatch),
  `:385` (`WORKTREES_MCP_PROVIDER`), `:1616-1712` (send).
- **App backend (~13):**
  - `lib.rs:276-303` (`agent_sessions: {claude, codex}`)
  - `:918,996,1062,4993` (literal provider checks)
  - `:783`, `:7056` (codex PATH fixup)
- **Frontend (~90 codex mentions in `App.tsx`)** plus `settings.ts`,
  `SettingsSheet.tsx`, `planUsage.ts`, `PlanPane.tsx:80`, `offers.ts`, and
  `NewPlaceDialog` (`App.tsx:1634`).
- **There is no activity trait.** A provider needs:
  - `session_for`
  - `activity(panes, canonical, path) -> Option<Activity>`
  - a turn source
  - `session_present(cwd)` for the resume gate
  - `model(cwd)` for the label

  Phase 1 extracts this as a trait with the two existing implementations,
  *before* adding a third.

---

<a id="phases"></a>

## 11. Phased plan

**Phase 0 — land this proposal.** Docs only.

**Phase 1 — make "two" into "N", with no pi yet.** One PR, no behaviour
change:

- Extract the adapter trait from [§2.3](#shared-shape) (launch args,
  session, activity, resume, running model, models, usage, send) with the
  two existing implementations. `most_active` becomes N-ary. **This phase is
  shared with the opencode proposal**: whichever harness lands first builds
  it, and neither re-plans it.
- One shared `Harness` type in the frontend. `agent_sessions` becomes a
  keyed map. "Switch to the other one" becomes "Switch agent…".
- Fix `for_pane` to prefer the session-name suffix / `pane_start_command`
  over the `node` heuristic. Add a test that a `node` pane on a
  `~agent~<x>` session is not Claude. This fixes a live bug for any
  node-based `--ai` too.
- Acceptance: bats + unit suites unchanged. `ls --json` is byte-identical
  against the shipped binary (the AGENTS.md rule for consolidations).

**Phase 2 — pi launches, resumes, and shows a dot.**

- Registry entry `pi`, sidecar `~agent~pi`. Launch is `pi --model
  <backend>/<id> --session-id <derived> --session-dir <pi default>
  --no-approve "<BRIEF_OPENER>"`. Resume is the same minus `--model`.
- `AgentChoice` / `ModelRef` / `ModelOption`. The pi model adapter parses
  `pi --list-models` (fixture-pinned to the version phase 2 is built against), with `auth check` for
  reasons. MCP `create_worktree` gains `model`. CLI `--model`.
  `USER_ONLY_KEYS` gains the new keys.
- Activity from JSONL (§3.1), with the traps in §3.2, plus the
  `Working`/`Retrying` rule line on screen for the first-turn gap. The
  model label follows §3.1.
- Preflight (§9.4) and reachability probe (§7), both surfaced in `doctor`
  and in the launch refusal.
- App: harness + model picker in the new-worktree dialog and the Switch
  sheet, a pi row mark, and a Settings → pi category.
- A manual gate `docs/pi-manual-checks.md`, like the AI-profiles one: launch,
  resume, trust modal, dead host, Esc. There is no fake pi in bats.

**Phase 3 — pi on the bus.**

- `worktrees msg report|list|wait` CLI verbs with cwd-derived `from`
  (usable by any harness).
- A worktrees-owned `worktrees-bus` skill passed with `--skill`.
- `send` to pi: literal type, confirmation via the JSONL user entry, screen
  fallback, trust modal = refuse. Fixtures in `tests/fixtures/pi-send/`.
- Optional: the `-e` extension for exact `agent_start`/`agent_settled`
  edges and a worktrees-owned probe file, if phase 2's first-turn gap
  proves annoying in practice.

**Phase 4 — maybe.**

- A "stalled" dot state for unreachable backends mid-turn.
- A model-discovery hint ("16 served, 1 declared").
- Kimi usage, only if pi exposes it without handing us a token.

---

## 12. Open questions

1. **Model list source.** Is parsing `pi --list-models`'s text table
   acceptable (fixture-pinned, with an upstream ask for `--json`), or would
   you rather read `models.json` + `auth check` and skip built-in catalogs
   entirely? The table is the one that knows what pi can *actually* use.
2. **Unlisted models.** LM Studio serves 16 models and pi declares 1. Should
   worktrees only *show* the gap, or also offer "add to pi's models.json"?
   That would be a write to pi's config, which this proposal currently
   forbids, the same as `~/.claude.json`.
3. **Answered (see Decisions): `--no-approve` by default.** Original
   question kept for the record. **Trust default (one decision for pi AND
   opencode).** This is the same
   question as the opencode proposal's Q1. opencode runs a repo's
   `.opencode/plugin/*.js` and `opencode.json` MCP commands at launch
   without prompting, and its only off-switch also drops `AGENTS.md`, so
   that lane proposes a user-scoped per-repo allowance gate. Answer once for
   both. For pi: is `--no-approve` right by default? It means repo
   `.agents/skills` and `.pi/` never load in pi lanes unless you opt in, and
   this repo's own layout will raise the prompt otherwise.
4. **Extension vs. skill.** Are you comfortable with worktrees shipping a
   small TypeScript pi extension loaded with `-e` (exact activity edges,
   native bus tools, and our code in pi's process)? Or should it stay
   skill + CLI only (weaker signals, zero coupling)?
5. **Answered (see Decisions): refuse, with "launch anyway".** Original
   question: **Unreachable host at launch.** Hard refuse with "launch anyway", or warn
   and launch?
6. **Kimi.** Do you want Kimi signed in and part of the picker? Nothing here
   needs it, and its usage meter is deferred (§6).
7. **Scope of `--model` for Claude and Codex.** Should the model picker also
   apply to Claude and Codex in phase 2, or stay pi-only until the pi path
   has proved the shape? `Profile.model` already covers Claude via profiles.
8. **Upstream asks.** Are you willing to file these with pi: `--list-models
   --json`; a launcher that refuses below `engines.node`; a
   reachability-aware `auth check`; a first-prompt session flush? Each one
   removes a workaround in this plan.

---

## 13. Probe record

All in `~/.cache/worktrees/worktrees/pi-harness-research/`:

- `cap-01…13` are screen captures:
  - start (`cap-01`)
  - busy (`cap-02`)
  - streaming (`cap-04`)
  - Esc (`cap-05`)
  - long typed message (`cap-06`)
  - bracketed paste (`cap-07`)
  - steering (`cap-08/09`)
  - refused host (`cap-11`)
  - trust modal and after (`cap-12/13`)
- `agentdir-offline/`, `agentdir-refused/` are scratch `PI_CODING_AGENT_DIR`s
  for the dead-host probes (fake providers, no credentials).
- `pi-install.sh` is pi's installer as downloaded 2026-09-29, read for §9.

Files the probes created outside the cache dir are the two sessions in §9.5.
The four throwaway tmux servers (`piprobe`, `piprobe2`, `pienv`, `pitrust`)
were killed and their sockets removed.
