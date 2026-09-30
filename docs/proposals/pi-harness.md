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

**Update, 2026-09-29 (phase 3 research):**
- Phases 1 and 2 shipped in v0.33.0.
- pi 0.99.1 has MCP built in, so §4 has been rewritten from probes on 0.99.1.
  The bus now goes through the same user-scope `worktrees mcp` as Claude and
  Codex.
- The `worktrees msg` CLI verbs and the bus skill are dropped.
- §11's phase 3 and §12 (questions 9–12) follow from that. The probe record
  is §13.1.

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
  user setting can switch it to `ask`.
- **Q3 follow-up, per-repo allowance: yes.** A repo the user has allowed
  launches with `--approve`, so its `.pi/` and `.agents/skills` load without
  pi's prompt, and so do its `.pi/mcp.json` servers (§4.4). The allowance
  is user-scoped and keyed by repo root. That is the only case in which
  worktrees passes `--approve` (§8, item 5).
- **Q5, unreachable host at launch: refuse, with "launch anyway".**
  - App: a "launch anyway" button.
  - CLI: `--force`.
  - MCP `create_worktree`: returns the reason and does not launch.
  - In every case the worktree and its brief are still created, so the lane
    can be launched later (§7).

Taken on 2026-09-29, after the phase 3 research (§4, §12 questions 9–12):

- **Q9, exposure: `direct`.** The install passes `--exposure direct`.
- **Q10, install route: shell out to `pi mcp add`.** It installs at user
  scope and never with `-l`. Worktrees never writes `mcp.json`, and detects
  the install by reading it. There is no per-launch extension.
- **Q11, allowance: warn, and PROTECT ours.**
  - At launch, where `trust::pi_flag` picks `--approve`: if
    `<place>/.pi/mcp.json` defines a `worktrees` server, that launch does
    not get `--approve`, and worktrees says why.
  - `doctor --pi` warns about the same thing, as a belt.
  - The allowance's wording says two things:
    - an allowed repo's `.pi/mcp.json` servers run with no prompt;
    - `.pi/mcp.json` is per BRANCH while the allowance is per repo root,
      so a fork's PR checked out as a worktree is covered by it.
- **Q12, upstream `--timeout` on `pi mcp add`:** left open.

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
| MCP | **Built in since 0.99.1** (0.87.1 had none). `pi mcp add worktrees --env WORKTREES_MCP_PROVIDER=pi --exposure direct -- worktrees mcp --mutations` gives a pi lane the same user-scope server Claude and Codex use: `from` is right and no new CLI verbs are needed **[observed]**. Two fixes are ours: `wait` must send progress, because pi times requests out at 60 s; and `CLAUDE_PROJECT_DIR` must be ignored for non-Claude providers (§4). |
| `send` | Typed text (`send-keys -l`) is shown inline, not folded. Enter while busy **queues as "Steering"** and is delivered after the current message **[observed]**. Bracketed paste folds to `[paste #1 N chars]`. On 0.99.1 the JSONL user entry is written at submit when idle, but only at DELIVERY for a steering message, so a busy send confirms from the screen (§4.5). |
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
| extension / skill | `-e <path>`, `--skill <path>` | Load from an explicit path **without** writing pi's config. The bus plan that used them is superseded by MCP (§4.6). |
| system prompt | `--append-system-prompt <text\|file>` | Could carry worktrees' bus instructions. Superseded by MCP (§4.6). |
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
    - pi confirms from the JSONL user entry when idle and from the screen
      (`Steering:` line) when busy (§4.5).
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
  (phase 2's two-quiet-samples rule in `mcp.rs`'s `wait`).
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

  Recommendation: JSONL + screen in phase 2. The bus no longer needs the
  extension, because pi 0.99.1 has MCP (§4.6), so it is kept only for the
  first-turn gap and as a file-free way to register the server.

---

## 4. MCP and communication

**Rewritten 2026-09-29 for pi 0.99.1** (the phase 3 research lane). The first
draft of this section was researched on 0.87.1, which had no MCP. It planned the
bus around new `worktrees msg` CLI verbs, a `worktrees-bus` skill and an
optional `-e` extension. 0.99.1 ships MCP as a built-in extension, so the
user-scope `worktrees mcp` serves a pi lane the same way it serves Claude and
Codex, and most of that plan is dropped (§4.6). Unless marked otherwise, every
**[observed]** below was run on 0.99.1 with the release build of this branch's
`worktrees`. Runs used a throwaway `PI_CODING_AGENT_DIR`, a throwaway tmux
server (`-L piphase3`) and a scratch repo with one place (`lane`). The model was
`lm-studio/qwen/qwen3-coder-480b`, the one model the user's pi declares today
(§13.1).

### 4.1 How pi's MCP works on 0.99.1

**Config files** **[source: `docs/mcp.md`, `extensions/mcp/config.js`]**

- pi reads the user file `<agent dir>/mcp.json`. The agent dir is
  `$PI_CODING_AGENT_DIR`, else `~/.pi/agent`, which is what `pi::agent_dir()`
  already resolves.
- It also reads the project file `<session cwd>/.pi/mcp.json`, but only when
  the project is trusted. It looks at the cwd itself, not its ancestors.
- The shape is the usual `mcpServers` map. It is parsed with plain
  `JSON.parse`, so there are no comments.
- **A project entry replaces a user entry with the same name.**
- `~/.pi/agent/mcp.json` does not exist on this machine yet.

**Transports** **[source]**

- stdio (`command`/`args`/`env`/`cwd`) and streamable HTTP (`url`). SSE is
  rejected.
- A stdio server inherits pi's whole environment plus its own `env`
  (`pi-mcp` `transports/stdio.js`: `{ ...process.env, ...options.env }`).
- Its cwd is `resolve(<session cwd>, config.cwd ?? ".")`.
- On shutdown pi closes stdin, then SIGTERMs and SIGKILLs the process group.

**Exposure** **[source + observed]**

Tools are named `mcp__<server>__<tool>`. The TUI renders them as
`worktrees/report`. The exposure mode decides how the model reaches them:

| Mode | What the model sees | Observed with worktrees' 22 tools |
|---|---|---|
| `codemode` (default) | One `codemode` tool whose description lists the MCP tools as TypeScript declarations, within a 3000-token budget. The model writes a JS script that calls `tools.mcp__worktrees__report(...)`. | **Works.** The model wrote `await tools.mcp__worktrees__report({ text: … })` and the call returned in 92 ms. The first turn was ~19k input tokens (7.6% of 128k). |
| `codemode-deferred` | The same, but only the server name and tool count are listed. Scripts must `searchTools()` first. | not run |
| `deferred` | Tools are hidden until the model calls `tool_search`, and are declared from then on. | not run |
| `direct` | Every tool is declared like a built-in, and is also callable from codemode. | **Works.** The model called `mcp__worktrees__report` natively. The first turn was ~17k input tokens (6.7%). |
| `hidden` | The tools are registered but cannot be called. | n/a |

- **Direct costs no more context than codemode here.** The codemode
  declarations for our 22 tools fill most of the same budget.
- **Which mode a weaker model handles better is inferred, not measured.**
  The planned lane model (qwen3.6-27b) is no longer declared, so nothing
  weaker than the 480b coder was tried. Writing a correct script is one more
  thing a small model can get wrong, and a native tool call is the shape
  every tool-calling model is trained on. **[inferred]**
- `pi mcp add` has `--exposure` for the whole server and no per-tool flag.
  Per-tool `toolExposure` exists only in the file, and worktrees does not
  write the file (§4.3).

**Recommendation:** install with **`--exposure direct`**. It is the only mode
in which `report`/`messages`/`wait`/`place_status` sit next to `bash` with no
indirection. It measured no dearer than the default. And it is the one choice
the CLI lets us make without editing pi's file. **Decided: `direct` (Q9).**

**`pi mcp list` LAUNCHES every server** **[observed, and docs/cli.md says so]**

This is the `claude mcp list` trap from AGENTS.md again. With the same
throwaway config:

- from a non-repo directory, the result was `state: connected, tools: []`;
- from a checkout, it was `connected, 22 tools … resources: 13`.

So the list answers "what does this server serve from HERE", not "is it
installed". It exits 1 whenever any server fails to connect, including
someone else's. **Detection must read `mcp.json`.**

**No per-launch way to add a server without a file** **[source]**

- There is no `--mcp-config` flag and no environment variable. `pi --help`,
  `docs/cli.md` and `docs/environment-variables.md` were all checked.
- `PI_CODING_AGENT_DIR` moves ALL of pi's config (auth, models, settings,
  sessions default), so it is not a per-launch knob for a real lane (brief
  rule).
- The one per-session route is an extension that calls
  `pi.registerMcpServer(name, config)`, loaded with `-e`. Registrations are
  not saved. A `mcp.json` entry with the same name overrides one, and `pi mcp`
  shell commands never see it. That route is kept as a fallback (§4.6), not
  the plan.
- Two other facts, for completeness:
  - `--no-extensions` (`-ne`) also disables the built-in MCP. worktrees does
    not pass it.
  - A user who installed `pi-mcp-adapter` has replaced built-in MCP
    wholesale, and `mcp.json` is then not read in sessions at all
    **[source]**. Status can only say "configured". It cannot promise the
    lane sees it.

**Running sessions do not pick up a new server.** pi connects at session
start. `/reload` or a new session is needed **[source]**. The first prompt
waits up to 10 s for startup connections.

### 4.2 `worktrees mcp` inside pi **[observed]**

A pi session was started in `lane` with `--no-approve`, and the model was asked
to `report`.

- **Handshake:** fine. `connected, 22 tools`, and 13 resources, so pi also
  adds its three resource tools (`list_mcp_resources` …).
- **cwd discovery:** the server process's cwd was the place
  (`lsof -d cwd` → `…/repo/.worktrees/lane`). The session cwd is the place
  because worktrees launches pi there, and pi resolves the server's cwd
  against it.
- **`from`:** `report` returned `"from": "lane", "to": "(main)"`.
  `caller_place()` needs nothing new.
- **Environment:** the server saw `WORKTREES_MCP_PROVIDER=pi` (ours, from
  `--env`) and **`PI_CODING_AGENT=true`, which pi sets on itself**. The
  server can tell it is serving pi even without our variable.

**`wait` breaks under pi's request timeout** **[observed]**

- pi times every MCP request out at **60 s** by default. The per-server
  `timeout` field can change that, but `pi mcp add` has no flag for it.
- `wait` defaults to **60 s** and allows up to 120.
- Asked for `wait until: message, timeout_s: 90`, pi reported `MCP request
  timed out after 60000ms` at 60 s, and the model saw an error, not
  `{"event":"timeout"}`.
- On timeout pi sends `notifications/cancelled`. `mcp.rs` ignores it, so the
  server keeps blocking its stdio loop until its own 90 s were up. Every
  other call to that server queued behind it for those 30 s.
- The 60 s default is a coin-flip race with pi's 60 s timeout.
  **[inferred]**

**The fix is on our side and helps every client:**

- pi sends a `progressToken` on every `tools/call`, and a
  `notifications/progress` for that token **re-arms the timeout**
  **[source: `pi-mcp/dist/client.js` `handleProgress` → `armTimeout`]**.
- `wait` should emit a progress notification every ~15 s while it blocks,
  whenever the request carried `_meta.progressToken`.
- The server should also honour `notifications/cancelled` for the in-flight
  `wait` by returning early, so a cancelled call stops holding the loop.
- The writer exists: every stdout line already goes through `emit`, which
  the watcher thread shares with the loop.
- **The reader does not.** The stdio loop is single-threaded: it reads a
  line, runs `handle_line` → the tool call → `wait` → `poll_until`, which
  sleeps. A `notifications/cancelled` cannot be READ until `wait` returns.
  Stage 2 needs a stdin reader thread feeding the loop through a channel,
  or `wait` moved off the loop, before cancellation can work.
- Claude and Codex gain the same protection. The 120 s cap stays, because
  it is also a cap on how long the loop is held.

**Does the server need to know it serves pi?** Only for one thing.

- `cmd_mcp` trusts `CLAUDE_PROJECT_DIR` unless the provider is `codex`
  (`mcp.rs:380`).
- A pi server inherits pi's entire environment. If a `CLAUDE_PROJECT_DIR`
  leaks into a pi pane (a pi started from inside a Claude session, or a
  tmux server that inherited one), the server would pin a DIFFERENT
  project, and `from` would be wrong or refused. **[inferred; not present
  in any pane measured here]**
- **Invert the test:** honour `CLAUDE_PROJECT_DIR` only when
  `WORKTREES_MCP_PROVIDER` is unset or `claude`. That also covers any
  future harness without a code change.
- The install passes `--env WORKTREES_MCP_PROVIDER=pi`, as Codex's does.
- Nothing else in `mcp.rs` reads the caller's provider. `send` keys on the
  TARGET's harness.

### 4.3 Setup UX, mirroring Codex

This is `codexmcp.rs` again, as `pimcp.rs`.

- **Install:** `pi mcp add worktrees --env WORKTREES_MCP_PROVIDER=pi
  --exposure direct -- <worktrees bin> mcp --mutations`.
  - User scope, which is the default.
  - **Never `-l`**: that writes the place's `.pi/mcp.json`, i.e. into the
    repo.
  - worktrees never writes `mcp.json` itself. It is pi's file, the same
    rule as `~/.claude.json` (AGENTS.md).
  - `add` replaces an entry with the same name and does not connect
    **[source]**, so no `remove` is needed first. The replacement drops any
    `toolExposure`/`timeout` a user hand-added, so the panel says so before
    a re-install.
  - **[observed]:** with a throwaway agent dir, the command wrote exactly
    that entry and created no other file.
- **Uninstall:** `pi mcp remove worktrees` (never `-l`). OAuth credentials
  are not involved.
- **Detection:** READ `pi::agent_dir()/mcp.json` → `mcpServers.worktrees`.
  - `ours`: command basename `worktrees` and `mcp` in args.
  - `command_ok`: the command still exists.
  - `mutations`: `--mutations` in args.
  - `exposure`: shown in the panel; `direct` recommended, others allowed.
  - `enabled: false` becomes a state of its own ("disabled in pi"), since
    `/mcp` can toggle it.
  - The states are Codex's: `installed` / `read-only` / `stale` / `foreign`
    / `absent` / `cli-missing`. Add `pi-missing` when `pimodels::pi_bin()`
    is none.
  - Never `pi mcp list` (§4.1). It costs a launch of every server the user
    has, and answers a different question.
- **CLI:** `worktrees mcp --status|--install|--uninstall --ai pi [--json]
  [--read-only]`, beside `--ai codex` in `cmd_mcp_setup`.
- **App:**
  - a "Worktrees tools" block in Settings → pi (`PiPanel.tsx`), the
    `CodexMcpPanel` shape;
  - backend commands `pi_mcp_status` / `pi_mcp_install` /
    `pi_mcp_uninstall`, tracked by the mock;
  - after an install the panel says running pi lanes need `/reload`.
- **Offer:** a `pi-mcp` entry in `offers.ts`, with the Codex rule exactly:
  `absent` AND pi installed (`pi_bin`), fingerprint `absent`, and
  destination `{cat: "pi", focus: "pi-mcp"}`. It extends `offers-check.mjs`.
- **Doctor:** `doctor --pi` gains a line for the MCP state.
- **Allowance interaction (§8 item 5), new:** an allowed repo's
  `.pi/mcp.json` can shadow `worktrees`. Status cannot see the repo from
  the startup probe (the `ctx.mcp` caveat in `offers.ts`). Decided (Q11):
  the launch protects ours (§4.4), and `doctor --pi` in a repo checks for
  that name and warns.

### 4.4 What `--no-approve` and the allowance mean for repo MCP servers **[observed]**

The place was given a `.pi/mcp.json` with two stdio servers, each `sh -c
'touch <marker>; exec cat'`. One was named `marker`. The other was named
**`worktrees`**, to test shadowing. pi was then started with no prompt sent:

- **`--no-approve`:** neither marker appeared. pi printed "This project is
  not trusted. Project .pi resources and packages are ignored." The global
  `worktrees` server started as normal.
- **`--approve`:** **both markers appeared at session start, before any
  prompt, with no dialog.** No server of ours was started for that session.
  **The repo's `worktrees` entry replaced ours entirely.**

What this means:

- **The default is safe.** `--no-approve` (phase 2's default) keeps a
  cloned repo from naming a single argv. ADR 0001 holds.
- **The allowance now covers more than it said.** §8 item 5 said the
  allowance lets repo `.pi/` resources load, and pi extensions already
  execute, so this is not a new CLASS of risk. But two consequences are new
  and belong in the allowance's words (Settings → pi, and the launch
  confirmation):
  1. Repo MCP servers START the moment the lane launches, not when a tool
     is used.
  2. The repo can REPLACE the worktrees server that lane talks to. A
     replacement can file messages with any `from`, because it is repo code
     running as the user, not our server.
- **Decided (Q11): warn, and protect ours.**
  - At launch, where `trust::pi_flag` would choose `--approve`: if
    `<place>/.pi/mcp.json` defines a `worktrees` server, that launch gets no
    `--approve`, and worktrees says why.
  - `doctor --pi` flags the same thing.
  - **Wording:** "Allowed: this repo's pi extensions, skills and
    `.pi/mcp.json` servers run with no prompt when a lane starts."
  - It adds: `.pi/mcp.json` is per BRANCH while the allowance is per repo
    root, so any branch checked out as a worktree is covered by it,
    including a fork's PR.

### 4.5 `send` to a pi pane (re-verified on 0.99.1) **[observed]**

The first draft's `send` section still holds, with one correction to the confirmation
plan:

| Probe | 0.99.1 result |
|---|---|
| 1,909-char attributed message, `send-keys -l`, idle | Shown **inline**, wrapped in the composer, not folded (`cap-send-typed`). One Enter submitted it. The JSONL user entry was 1,908 chars: **pi trims trailing whitespace**, so match the attributed header as a prefix, never the full text. |
| Enter while busy | `Steering: [from (main)] …` above the working rule, plus `↳ Option+Up to edit all queued messages` (`cap-send-steer`). It was delivered after the current answer, and the model answered it. |
| **When the JSONL entry is written** | **Idle send: at submit** (the entry's timestamp matched the Enter). **Steering send: at DELIVERY**, i.e. when the running turn ends: 00:26:19, versus an Enter at about 00:26:11. |

Consequences for the build:

- **Two confirmations, by state at the time of typing:**
  - idle: `Submitted` = a new user entry whose text starts with our header;
  - busy: `Queued` = the composer cleared and a `Steering:` line starting
    with our header sits above the rule.
  - Waiting for the JSONL on a busy send would hold the MCP call for a
    whole turn.
  - `SendOutcome` needs a `Queued` (or the Codex "busy input queues"
    outcome reused, see `send_review_busy_input_queues…` in
    `harness.rs`).
- **The "first turn has no file" fallback is gone on 0.99.1.** pi writes
  the session file at the first user message (phase 2 finding), and a
  lane's first user message is the brief opener, which is typed before any
  `send` can target it.
- **Unchanged:**
  - no bracketed paste (it folds to `[paste #1 N chars]`);
  - no control keys (Ctrl-C clears the composer, and a second Ctrl-D
    exits);
  - the trust modal is `waiting`, so refuse, because Enter selects
    **Trust** (phase 2 verified `may_type` refuses there);
  - the one-line and attribution rules carry over from `send_text_ok`.
- Fixtures go in `tests/fixtures/pi-send/`, pinned to 0.99.1: idle composer
  with typed text, the steering line, and the composer after submit. The
  raw captures are in §13.1.

### 4.6 What survives of the first plan

- **`worktrees msg report|list|wait` CLI verbs: DROP.** Every harness in
  scope now speaks MCP: Claude, Codex, pi, and opencode (per its proposal
  §4.1). The verbs would be a second transport to the same log, with a
  second trust story (cwd-derived `from` in a shell the model controls),
  for no harness that needs it. It is parked in ROADMAP, for the day a
  harness without MCP arrives.
- **The `worktrees-bus` skill (`--skill`): DROP.** In direct mode the tool
  descriptions are the documentation. The server's `instructions` already
  reach codemode's description **[source]**.
- **The `-e` extension: DROP for the bus, KEEP as the escape hatch.**
  Native tools now come from MCP, which removes the extension's main
  reason. Its other reason, exact `agent_start`/`agent_settled` edges for
  the first-turn activity gap, is unchanged from §3.3 and is not a phase 3
  need: phase 2 shipped without it, and ROADMAP tracks the startup-gap dot.
  One new use would justify it: an extension's `registerMcpServer` is the
  only way to give a lane the server WITHOUT touching `mcp.json`. That is
  worth building only if users refuse the global install. **Decided: no
  extension (Q10).**
- **`send` to pi: BUILD**, per §4.5.

Claude↔Claude keeps Claude's own messaging. pi↔anyone goes over the MCP message
log (`report` / `messages` / `wait`) and `send`, exactly as Codex does.

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
  node the **pane** will use (§9), and per-backend readiness. A
  "Worktrees tools" block, the `CodexMcpPanel` shape (§4.3).
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
   for people who want repo skills. Worktrees passes `--approve` only for a
   repo the user has allowed (item 5), and never writes `trust.json`.
2. `send` and `may_type` treat the trust modal as **waiting**. Its footer is
   `↑↓ navigate  enter select  escape/ctrl+c cancel` under a `Trust project
   folder?` heading. Enter is never pressed into it.
3. `--session-dir <pi default dir>` is passed explicitly, so a repo's
   `sessionDir` cannot redirect where worktrees reads activity (§3.2).
4. The worktrees MCP server reaches pi through pi's USER `mcp.json`, and
   only via `pi mcp add` (§4.3), never `-l` and never a write of ours. If
   an extension is ever shipped (§4.6), it is loaded by **path from
   worktrees' own data dir**, the same provenance as the brief opener. It is
   never installed into `~/.pi`, and never read from the repo.
5. **Per-repo allowance (decided 2026-09-29).** The user can allow a repo
   once, and every pi lane in that repo then launches with `--approve`
   instead of `--no-approve`.
   - **Where it lives:** `~/.config/worktrees/config.toml`, as a list of
     canonicalised repo roots per harness (for example
     `[trust] pi = ["/path/to/repo"]`). This is the provenance ADR 0001
     prescribes for `post_create`. The key joins `USER_ONLY_KEYS`, so a
     `.worktrees.toml` that sets it is a hard parse error.
   - **Scope:** the repo root (the parent of the git common dir), so it
     covers every worktree of that repo, including ones created later. It
     is never inferred from a place directory a repo could fake.
   - **Who can grant it:** only the user, from Settings → pi or a CLI verb
     (`worktrees trust pi [<repo>]` / `--revoke`). MCP `create_worktree`
     cannot grant it, and an orchestrating agent cannot either. MCP may
     *report* whether a repo is allowed.
   - **What it lets run (measured on 0.99.1, §4.4):**
     - The repo's `.pi/mcp.json` stdio servers START at session start,
       with no dialog.
     - A project entry named `worktrees` would REPLACE the user's, so the
       launch refuses `--approve` when one exists (decided, Q11).
     - The file is per branch and the allowance per repo root, and the
       wording says so.
   - **What it changes:** one flag. Worktrees still never writes pi's
     `trust.json`, so the allowance does not leak into `pi` runs outside
     worktrees, and revoking it takes effect at the next launch.
   - **The app shows it:** the pi row of a place in an allowed repo, and
     the launch confirmation, say that repo code will load.
   - **Shared with opencode:** the same `[trust]` table is the allowance
     the opencode proposal's §8 gate needs, keyed `opencode = [...]`. One
     mechanism, one place for the user to look.

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
  --no-approve "<BRIEF_OPENER>"` (`--approve` instead for a repo in the
  user's `[trust] pi` allowance, §8 item 5). Resume is the same minus
  `--model`.
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

**Phase 3 — pi on the bus, through pi's own MCP (rewritten for 0.99.1, §4).**
Two PRs: core/CLI/MCP, then app.

- **Server fixes (every client benefits):**
  - `wait` emits `notifications/progress` about every 15 s when the call
    carried a `progressToken`, so pi's 60 s request timeout never fires
    mid-wait.
  - It honours `notifications/cancelled` for the in-flight call.
  - `cmd_mcp` honours `CLAUDE_PROJECT_DIR` only when
    `WORKTREES_MCP_PROVIDER` is unset or `claude`.
- **`pimcp.rs`** (the `codexmcp.rs` shape):
  - status by READING `pi::agent_dir()/mcp.json`, never `pi mcp list`;
  - install via `pi mcp add worktrees --env WORKTREES_MCP_PROVIDER=pi
    --exposure direct -- <bin> mcp [--mutations]` (Q9, Q10);
  - uninstall via `pi mcp remove worktrees`;
  - never `-l`, and never a write of ours.
  - CLI: `worktrees mcp --status|--install|--uninstall --ai pi`.
  - `doctor --pi` gains the MCP state, plus a warning when an allowed repo's
    `.pi/mcp.json` defines `worktrees`.
- **`send` to pi:**
  - literal type, with the one-line and attribution rules;
  - idle → `Submitted` on a new JSONL user entry that starts with the header;
  - busy → `Queued` on the composer clearing plus a `Steering:` line;
  - trust modal → refuse.
  - Fixtures in `tests/fixtures/pi-send/`, pinned to 0.99.1.
  - The Plan tab's paste into pi follows the same gate.
- **App:**
  - a "Worktrees tools" block in Settings → pi;
  - `pi_mcp_*` commands (in the mock too);
  - a `pi-mcp` offer (`absent` and pi installed);
  - the allowance's wording names repo MCP servers and the per-branch
    caveat (§4.4).
- **Build notes from the #371 review:**
  - **a. Cancellation needs a reader thread.**
    - A stdin reader thread feeds the loop through a channel, so
      `notifications/cancelled` is seen mid-`wait`.
    - `poll_until` checks a cancel flag, and its sleep becomes a
      `recv_timeout`.
    - Per the spec, no response is sent after a cancel.
  - **b. Progress needs the token and the request id.**
    - Plumb `params._meta.progressToken` and the request id into the call
      and into `wait`. The call dispatch strips `params` down to
      `arguments` today, so `wait` never sees `_meta`.
    - Emit via `emit` with a NUMERIC `progress`, because pi's client
      ignores a non-numeric one.
    - Keep it injectable, so `poll_until`'s virtual-time test stays.
  - **c. Invert the `CLAUDE_PROJECT_DIR` test in `cmd_mcp`.**
    - It is trusted only when `WORKTREES_MCP_PROVIDER` is unset or
      `claude`.
    - Update the doc comments that describe it, and test it under
      `ENV_LOCK`.
    - Grep every reader of `WORKTREES_MCP_PROVIDER` first.
  - **d. `pimcp.rs`.**
    - JSON-parse `agent_dir()/mcp.json`.
    - States: Codex's six, plus `disabled` and `pi-missing`.
    - Expose the exposure mode.
    - Dispatch `--ai pi` beside `--ai codex`.
    - `pi mcp add` resolves `PI_CODING_AGENT_DIR` from ITS environment, so
      pass it the same value `pi::agent_dir()` used, so that status and
      install agree.
  - **e. `send`.**
    - Add `SendOutcome::Queued`.
    - Remove pi's by-name `send` refusal and the Plan-tab paste refusal.
    - Add the pi send fixtures.
  - **f. The launch-time `.pi/mcp.json` `worktrees` check** (Q11).
- **Docs:** `docs/pi-manual-checks.md` gains an MCP section: install, a
  `report` from a lane, `wait` past 60 s, a `send` idle and busy, `/reload`.
- **Dropped from the first plan** (§4.6): the `worktrees msg` CLI verbs, the
  `worktrees-bus` skill, and the `-e` extension for the bus.

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
3. **Answered (see Decisions): `--no-approve` by default, plus a
   user-scoped per-repo allowance (§8 item 5).** Original
   question kept for the record. **Trust default (one decision for pi AND
   opencode).** This is the same
   question as the opencode proposal's Q1. opencode runs a repo's
   `.opencode/plugin/*.js` and `opencode.json` MCP commands at launch
   without prompting, and its only off-switch also drops `AGENTS.md`, so
   that lane proposes a user-scoped per-repo allowance gate. Answer once for
   both. For pi: is `--no-approve` right by default? It means repo
   `.agents/skills` and `.pi/` never load in pi lanes unless you opt in, and
   this repo's own layout will raise the prompt otherwise.
4. **Superseded by pi 0.99.1's MCP (§4.6).** The first draft asked whether
   to ship a `-e` extension or a skill for the bus. The bus now goes through
   MCP; what remains of the extension question is question 10.
   Original question: **Extension vs. skill.** Are you comfortable with worktrees shipping a
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

9. **Answered (see Decisions): `direct`.** Original question: **Exposure mode (phase 3).** Install with `--exposure direct`, which
   gives native tool calls and measured no dearer than the default, or
   leave pi's default `codemode`, which respects the user's pi setup and
   needs the model to write a script? Both worked with the 480b coder. No
   weaker model was tried (§4.1).
10. **Answered (see Decisions): `pi mcp add`, no extension.** Original question: **A file-free route.** The global install writes one entry into
    `~/.pi/agent/mcp.json` through `pi mcp add`, the same trade as Claude and
    Codex. The only alternative is a worktrees-owned extension loaded with
    `-e` that calls `registerMcpServer` per lane: nothing is written to
    `~/.pi`, but our TypeScript runs in pi's process and pi's extension API
    becomes a compatibility surface. Recommendation: global install only,
    and revisit if someone objects to the entry.
11. **Answered (see Decisions): warn, and refuse `--approve` for that launch.** Original question: **Allowance wording (§4.4).** An allowed repo's `.pi/mcp.json` starts
    its servers at launch and can replace the `worktrees` server. Is it
    enough to SAY so in Settings → pi and the launch confirmation, or should
    worktrees refuse `--approve` for a repo whose `.pi/mcp.json` defines
    `worktrees`?
12. **Still open.** **Upstream ask, added.** `pi mcp add --timeout` (and `--tool-exposure`)
    would let the install carry the right values without our server
    working around them. Not needed if `wait` sends progress.

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

### 13.1 Phase 3 research probes (pi 0.99.1, 2026-09-29)

These are in `~/.cache/worktrees/worktrees/pi-phase3/`:

- `agentdir/` is the throwaway `PI_CODING_AGENT_DIR`.
  - `mcp.json` was written by `pi mcp add`.
  - `models.json` is a copy of the user's (read for the provider only).
  - I wrote a two-key `settings.json`.
  - pi itself created `auth.json` (never opened), `models-store.json` and
    `bin/fd` there.
- `repo/` is the scratch repo, with one place, `lane`.
- `sessions*/` are the probe sessions, via `--session-dir`.
- `list-nonrepo.json` and `list-repo.txt` show `pi mcp list` from outside a
  repo (connected, 0 tools) and from a checkout (22 tools).
- `cap-send-typed.txt`, `cap-send-steer.txt` and `cap-idle-after.txt` are
  `send` screens.
- `pi-home-before.txt` / `pi-home-after.txt` are `ls -la ~/.pi/agent`
  around the probes.

Model: `lm-studio/qwen/qwen3-coder-480b`. The brief named `qwen3.6-27b`, but
the user's `models.json` now declares only the 480b coder.

**Nothing was written under `~/.pi`, and `~/.pi/agent/mcp.json` still does
not exist.** No file was added, removed or changed there. The directory mtimes
of `~/.pi/agent` and `~/.pi/agent/install` did move during the probes, so
something created and removed a transient entry. That is presumably the
managed launcher's version and update check, but it was not identified. The
throwaway tmux server (`-L piphase3`) was killed. The two stdio marker
servers of §4.4 lived only in the place's `.pi/`, which was deleted
afterwards, together with the markers.
