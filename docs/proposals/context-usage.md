---
title: "Proposal — context usage per agent session"
---

# Proposal — context usage per agent session

**Status:** research and plan, 2026-10-03. Nothing built.

**What it is for:** each place's agent shows how full its context window is,
for example "62% of 1M". The orchestrator in `(main)` can read the same number
through MCP `place_status`, so it can tell a lane that is near its limit to
`/compact` or hand off.

**Evidence base:**

- Versions measured: Claude Code **2.1.288**, codex-cli **0.160.0** and pi
  **0.99.1**, on this Mac, on 2026-10-03.
- Real files read:
  - **251** Claude transcripts from the last 14 days, plus every compaction
    in the last 30 days;
  - **180** Codex rollouts from the last 30 days;
  - all **27** pi sessions.
- One live probe was run for each CLI, in a throwaway `worktrees new` place on
  private tmux servers (`tmux -L ctxprobe-cu` and `tmux -L ctxprobe-cx`, both
  killed afterwards). It made three tiny model calls: Claude haiku "reply ok",
  then one `/context` on opus with no reply, and Codex "reply ok". The place
  was removed afterwards.
- Read-only on `~/.claude`, `~/.codex` and `~/.pi`, with **one exception that
  the CLI caused, which was undone** (§6.1). `auth.json` was never opened.
- The samples below show field shapes and numbers only. Prompt text and ids are
  redacted.

## 1. Decision (summary)

**1. Read the number from the same 256 KiB tail read that each harness's
model reader already does.** No new file reads, no new spawns and no new
config.

| Harness | Numerator | Window |
|---|---|---|
| Claude | `input_tokens + cache_creation_input_tokens + cache_read_input_tokens` of the last real main-chain assistant `message.usage` | Not written anywhere we can read. Use a model table plus an "observed > 200K ⇒ 1M" ratchet. |
| Codex | `last_token_usage.total_tokens` from the last `token_count` event | `model_context_window` in the same event |
| pi | `usage.totalTokens` of the last assistant reply after the latest `compaction` | The model's `contextWindow` from the pi catalog we already parse |

**2. Show each CLI's own number.**

- For Claude and pi, our % is the same arithmetic as `/context` and pi's
  footer, give or take pi's estimate of the trailing messages.
- For Codex it matches `/status`'s "% left". That figure takes a 12K baseline
  off both sides, which we mirror.
- A user who looks at the lane and at our chip must never see two different
  numbers for the same session.

**3. The primary surface is the nav row, shown only when a lane nears its
compaction point.** The place header shows it at all times as detail. MCP
`place_status` and `ls --json` carry the full record. §8 gives the reasoning.

**4. Warn relative to each harness's auto-compact point, not to the window.**

- Claude 200K compacts at 83.5% of its window, and Claude 1M at 96.7%.
- So a fixed "≥ 80% of window" warns too late on one and far too early on the
  other.

**5. Ship in three phases** (§10):

1. the core reader plus MCP and `ls --json`;
2. the app surfaces, on a new context event so a turn never forces a full
   re-list;
3. optionally, an exact Claude window, which needs a decision from David
   (§11, question 1).

## 2. Per-harness summary

| | **Claude** | **Codex** | **pi** |
|---|---|---|---|
| **Source** | `~/.claude/projects/<mangled>/<sid>.jsonl`, with `sid` taken from the session probe `~/.claude/sessions/<pid>.json` | `$CODEX_HOME/sessions/Y/M/D/rollout-*.jsonl`, found by `codex::latest_rollout(cwd)` | `~/.pi/agent/sessions/--<cwd>--/<ts>_<id>.jsonl`, found by `pi::current_session` |
| **Numerator** | last assistant `message.usage`: `input_tokens + cache_creation_input_tokens + cache_read_input_tokens`. Skip `<synthetic>` and all-zero usage. | `event_msg/token_count` → `info.last_token_usage.total_tokens` | last assistant `message.usage.totalTokens` (falls back to `input + output + cacheRead + cacheWrite`) |
| **Window** | **Not in any file we can read.** Use a model table, the `[1m]` suffix when we launched with it, and the ratchet "seen > 200K ⇒ 1M". Fallback is 200K. | `info.model_context_window`: 258,400 for every model seen | catalog `contextWindow` (`pi --list-models` "context" column, `models.json`); pi itself defaults to 128K for a custom model with none |
| **Matches the CLI's own display?** | **Exactly.** Live: 63,859 → `/context` "63.9k/200k tokens (32%)" | **Exactly**, with Codex's baseline. Live: 30,888 / 258,400 → `/status` "92% left (30.9K used / 258K)" | **Slightly low.** pi's footer adds an estimate (characters ÷ 4) of the messages after the last reply. |
| **Freshness** | One line per assistant message, written while the turn runs (every tool step) | One `token_count` per model response | One line per assistant message |
| **Compaction signal** | `system/compact_boundary` with `compactMetadata{trigger, preTokens, postTokens}`; `/clear` gives a new `sessionId` in the probe | a `compacted` record, then a `token_count` with `input_tokens: 0` and `total_tokens` set to the post-compaction estimate | `compaction{tokensBefore, firstKeptEntryId}`; pi shows "?" until the next reply |
| **Auto-compact point** | window − 33K buffer (`/context` "Autocompact buffer: 33k"). On 1M that is 967K, and 13 of the 14 auto-compactions since v2.1.269 fired at 967–971K (the other at 933K). | Observed at 231–245K of 258.4K (90–95%). The setting is not logged. | `contextTokens > contextWindow − reserveTokens`, where the default `reserveTokens` is 16,384 |
| **Tail fit** | The last usage line is ≤ 154 KB from the end in the 40 largest transcripts (median 6.7 KB), so it fits in the existing 256 KiB tail | ≤ 2 KB from the end (CLI rollouts) | ≤ 2.7 KB from the end |
| **Confidence** | Numerator **high**. Window **medium**: right for every session observed, but by inference. | **High** | Numerator **high**, slightly under pi. Window **high** when the catalog lists the model. |

## 3. Claude

### 3.1 Where the number lives

**The transcript.** Every assistant line carries `message.usage`: all 70,513
assistant lines across the 251 transcripts did. A redacted sample:

```json
{"type":"assistant","isSidechain":false,"timestamp":"…",
 "message":{"model":"claude-opus-5-5","stop_reason":"tool_use",
  "usage":{"input_tokens":2,"cache_creation_input_tokens":50402,
   "cache_read_input_tokens":25050,"output_tokens":126,
   "cache_creation":{"ephemeral_1h_input_tokens":50402,"ephemeral_5m_input_tokens":0},
   "iterations":[{"input_tokens":2,"cache_read_input_tokens":25050,
                  "cache_creation_input_tokens":50402,"output_tokens":126,"type":"message"}],
   "service_tier":"standard","speed":"standard"}}}
```

This sample is 75,454 tokens in context.

Points to handle when reading it:

- **One message can be logged twice.** A streamed reply is split, and the live
  probe wrote the same usage on two lines. Take the last one; both are the
  same.
- **`iterations[]`.** In 4 cases it had more than one entry. The top-level
  numbers always equalled the last iteration's, so read the top level.
- **Subagents are not in the main transcript.** There were zero
  `isSidechain: true` assistant lines in main transcripts; subagents live in
  `<sid>/subagents/*.jsonl` (424 files). The last assistant line in the main
  file is therefore the main loop's.
- **Skip `model: "<synthetic>"` and usage that is all zeros.** The first
  assistant entry after a compaction is often one of those: 6 of 33 here.
  Reading it gives a false "0%".

**Other places checked and rejected:**

- **The session probe** `~/.claude/sessions/<pid>.json` has `status` and
  `sessionId`, but no model and no tokens.
- **`~/.claude.json`** has `projects.<dir>.lastModelUsage` and
  `lastTotal*Tokens`. These are cumulative totals per project, from the last
  session that exited, so they are not current context. That file is also the
  one AGENTS.md says to read and never write.
- **`cost-state` lines** in the transcript hold cost and duration, with an
  empty `modelUsage`.
- **`/context` output.** It is written into the transcript as a
  `system/local_command` stdout, but only when someone runs it.

### 3.2 Numerator, checked against Claude's own display

In the 2.1.288 binary, `/context` and the statusline share one function:

```js
// minified, names as shipped
function RIe(e, n) {           // e = last usage, n = window
  return { total_input_tokens: e.input_tokens + e.cache_creation_input_tokens + e.cache_read_input_tokens,
           context_window_size: n, current_usage: e, used_percentage: …, remaining_percentage: … } }
// used = clamp(0, 100, round(total / window * 100))
```

**Live check** (haiku, after one "ok" reply):

- The transcript's sum is `10 + 38,973 + 24,876` = **63,859**.
- `/context` printed **"63.9k/200k tokens (32%)"**. That is the same number.

`output_tokens` is **not** counted. It becomes part of the next prompt, and it
shows up there.

### 3.3 Window: the hard part

**The transcript never says how big the window is.**

`message.model` is the bare id: `claude-opus-5-5`, never
`claude-opus-5-5[1m]`. All 70,513 lines were like that. Yet:

| Model (as logged) | Sessions | With a prompt > 200K | Largest prompt |
|---|---|---|---|
| claude-opus-5-5 | 129 | 59 | 966,829 |
| claude-opus-5 | 99 | 66 | 999,461 |
| claude-fable-5-1 | 58 | 37 | 966,929 |
| claude-sonnet-5-5 | 24 | 2 | 209,283 |
| claude-fable-5 | 9 | 6 | 896,803 |
| claude-sonnet-5 | 9 | 6 | 644,030 |
| claude-opus-4-8 | 1 | 1 | 965,401 |
| claude-haiku-4-5-20251001 | 17 | 0 | 74,564 |

The live `/context` on opus printed **"82.1k/1m tokens"**, **"Auto-compact
window: 1m tokens"** and **"Autocompact buffer: 33k tokens"**. The settings
say only `"model": "opus"`, so Opus 5.5 gets a 1M window here with no suffix.

How Claude decides the window (2.1.288, function `vv`):

1. **1M** if the model string contains `[1m]`;
2. else **1M** if the 1M beta header is on and the model supports it;
3. else the model capability table's declared context, when that is over
   200K;
4. else `CLAUDE_CODE_MAX_CONTEXT_TOKENS`;
5. else **200,000**.

An account "1M credits blocked" latch can override all of this. None of these
inputs reach a file we read.

**What `/model` writes is not enough either.** `/model sonnet[1m]` logged
``Set model to `Sonnet 5.5` `` with no "(1M context)" text. (Older versions
added it, which is why `agent::model_switch` strips it.)

**Proposed resolution, in order:**

1. **`[1m]` in the model we launched with.** Worktrees knows the
   `model_arg` it passed. This is exact when it applies.
2. **A core table of known window defaults**, `claude_window(model)`, as data.
   Each family is set from evidence like the table above: opus-5*, fable-5*,
   sonnet-5 (non-5.5) and opus-4-8 are 1M here; haiku-4-5 is 200K.
3. **The ratchet.** If any prompt in this session's tail is over 200K, the
   window is 1M. This can never be wrong in the unsafe direction: a prompt
   cannot exceed its window. It upgrades a mis-tabled model on the first big
   turn.
4. **Otherwise the window is unknown.** Show tokens only ("182K") and no %.
   A wrong denominator is worse than a missing one.

**Known gap: sonnet-5-5.** 22 of its 24 sessions never passed 200K, so a 1M
sonnet-5-5 session reads as 200K-unknown until it does. Its two big sessions
show that 1M happens for it. The `[1m]` launch argument covers the case where
worktrees chose the model. A `/model sonnet[1m]` typed inside the lane is
covered only by the ratchet.

### 3.4 Compaction, clear and resume

**`/compact` and auto-compaction** write a boundary record:

```json
{"type":"system","subtype":"compact_boundary","content":"Conversation compacted",
 "compactMetadata":{"trigger":"auto","preTokens":969281,"postTokens":9987,
  "cumulativeDroppedTokens":959294,"durationMs":67024,…},"version":"2.1.283"}
```

- In the last 30 days there were 37 boundaries.
- 13 of the 14 `auto` ones since v2.1.269 fired at **967–971K**, which is
  1M − 33K; the other fired at 933K. Older versions (2.1.220–251) fired at about 1.00M.
- `manual` ones fired anywhere from 127K to 912K.
- **`postTokens` (8–50K) counts messages only.** The first real reply after a
  compaction showed **54–127K**, because the system prompt, tools and memory
  are added back. So `postTokens` is not the new numerator.
- **Rule:** after a boundary with no later real usage, the state is
  **"compacted, size unknown"**. pi shows the same state, as "?". The next
  reply gives the true number.

**`/clear`.** In the live probe the session probe's `sessionId` changed at
once (`7e14…` → `b8a8…`), and the new transcript held no assistant usage.

- The existing per-pid reader follows the new `sessionId` with no extra work.
- The reading is "no reply yet" until the first reply arrives.

**Resume.** `--resume` and `-c` append to the same transcript, so the last
usage there is still the context the session resumes with.

### 3.5 Freshness and cost

- **Freshness.** Assistant lines are appended while the turn runs, one per
  tool step, so the number moves during a turn and not only at its end.
  AGENTS.md's warning still applies: the file's mtime keeps moving long after
  the last turn, so read the content.
- **Size.** Transcripts are large: median 1.5 MB, p90 8.4 MB, largest 139 MB.
  Read only the tail.
- **The existing tail already covers it.** `cached_model` (app `lib.rs`)
  already reads the last 256 KiB when a transcript grows, and caches by file
  length. Across the 40 largest transcripts the last real usage line was at
  most **154 KB** from the end (median 6.7 KB).
  - Parsing usage in that same pass costs no extra I/O.
  - When a single giant tool result pushes it out of the window, keep the last
    answer. `cached_model` already does this for the model.
- Reading 64 KiB from the end of each of the 40 largest files took 0.9 ms in
  total, with a warm cache.

## 4. Codex

### 4.1 Where the number lives

`event_msg` / `token_count`. There were 4,244 of these across 180 rollouts. A
sample (`rate_limits` omitted):

```json
{"type":"event_msg","payload":{"type":"token_count","info":{
  "total_token_usage":{"input_tokens":75068,"cached_input_tokens":68736,"output_tokens":235,
                       "reasoning_output_tokens":0,"total_tokens":75303,…},
  "last_token_usage": {"input_tokens":15235,"cached_input_tokens":14976,"output_tokens":87,
                       "reasoning_output_tokens":0,"total_tokens":15322,…},
  "model_context_window":258400}}}
```

What to note about these records:

- **`total_token_usage` is cumulative over the session.** It reached
  20,824,864 in one rollout, which is billing, not context. **The numerator is
  `last_token_usage.total_tokens`.**
- **`token_usage_record`** is a newer record (since 0.157, 4,156 of them). It
  has per-response and per-thread usage but **no window**, so use
  `token_count`.
- **`model_context_window` was 258,400 for every CLI model**: gpt-6-astra,
  gpt-6-sol, gpt-6.1-sol, gpt-6-luna, gpt-5.6-sol and the codex-auto-review
  guardian.
- **It was null in 85 events, all in Codex Desktop rollouts.** Those are
  placeholder threads with one estimated `token_count` and no turns. They are
  not worktrees lanes. Treat a null window as unknown.

### 4.2 Numerator, checked against Codex's own display

**Live check** (gpt-6-astra, after one "ok" reply):

- The rollout had `last_token_usage.total_tokens` **30,888** and window
  **258,400**.
- `/status` printed **"Context window: 92% left (30.9K used / 258K)"**.

The tokens match. The % does **not** match a plain 1 − 30,888 / 258,400, which
is 88%. It matches Codex's baseline formula:

```
left% = (window − used) / (window − 12000) = 227,512 / 246,400 = 92.3%  → "92% left"
```

Codex treats the first 12K (its own instructions) as always present. The
proposal mirrors that for Codex, so our "8% used" and Codex's "92% left" agree.

### 4.3 Compaction

```
token_count  last.input 242529  last.total 242726      ← just before
compacted    {message, replacement_history[…]}
turn_context / thread_settings_applied
token_count  last.input 0       last.total  24954      ← post-compaction estimate
```

- This happened in all 13 compactions, with the estimate between 18K and 25K.
- Unlike Claude, Codex writes the new size itself, so the numerator stays
  valid across the boundary.
- `response_item/compaction` (encrypted content) also appears, but carries no
  numbers.
- Auto-compaction was observed at 231–245K, which is 90–95% of the window. The
  limit (`model_auto_compact_token_limit`) is not logged.

### 4.4 Freshness and cost

- **Freshness.** There is one `token_count` per model response, mid-turn
  included.
- **Size.** Rollouts are small next to Claude's: median 353 KB, largest 24 MB.
- **Tail fit.** In the 21 largest CLI rollouts, the last `token_count` was
  within 2 KB of the end.
- **Cost.** `activity::codex_tail` already reads 256 KiB on growth, caches it
  in `CODEX_TAIL` and returns `(model, Turn)`. Context becomes a third field in
  that cache entry.
- **Subagent rollouts.** Those of the auto-review guardian (36 of them) share
  the cwd. `latest_rollout` already picks the user's session (AGENTS.md).

## 5. pi

### 5.1 Where the number lives

A session is a tree of entries, linked by `parentId`. Redacted samples:

```json
{"type":"model_change","provider":"lm-studio","modelId":"qwen/qwen3-coder-480b",…}
{"type":"message","message":{"role":"assistant","api":"openai-completions",
  "provider":"lm-studio","model":"qwen3-coder-next","stopReason":"stop",
  "usage":{"input":100987,"output":381,"cacheRead":0,"cacheWrite":0,"reasoning":0,
           "totalTokens":101368,"cost":{…}}}}
{"type":"compaction","summary":"<2047 chars>","firstKeptEntryId":"…","tokensBefore":112575,
 "usage":{"input":14745,"output":345,…,"totalTokens":15090}}
```

- **Every assistant message has `usage`**: 958 of 958.
- **The `usage` on a `compaction` entry belongs to the summarising call**, not
  to the new context. Do not read it as the numerator.

### 5.2 Numerator, checked against pi's own display

From pi 0.99.1's source:

- **`getContextUsage()`** (`core/agent-session.js`):
  - If there is a compaction and no successful assistant reply after it, the
    result is `{tokens: null, percent: null}`, which the footer shows as
    **"?"**.
  - Otherwise it takes the last assistant's `calculateContextTokens(usage)`,
    which is `totalTokens || input + output + cacheRead + cacheWrite`, and
    adds an estimate (characters ÷ 4) of the messages after it.
- **The footer** (`components/footer.js`) shows `"<pct>%/<window>"`. It turns
  warning-coloured above **70%** and error-coloured above **90%**.

**Our tail read takes the last assistant's `totalTokens` only.** That is lower
than pi's footer by the size of the trailing tool results. The gap is large
only in the middle of a turn, after a big tool result. A pi lane that is not
idle is mid-turn, so this is acceptable for a nav chip. It should be stated in
the tooltip ("as of the last reply").

**Tree caveat.** After `/tree` navigation, the newest assistant line in the
file may belong to an abandoned branch. `pi::session_model` takes the same
approach and so has the same caveat. Rare, and it heals on the next reply.

### 5.3 Window

- **What pi uses.** pi's window is `model.contextWindow` from its registry.
  Built-in models carry it: the 0.99.1 dist has hundreds of
  `contextWindow: 1e6 / 262144 / 128e3 …` entries. For a custom model with
  none, pi uses 128K.
- **This machine's `models.json`** declares `lm-studio` models with **no**
  `contextWindow`, and `pi --list-models` prints `context 128K` for them.
- **Where we get it.** `pimodels.rs` already parses `pi --list-models`, but
  only provider and model. Adding the `context` column ("128K", "1M") gives
  the same number pi uses, with no new spawn: the catalog is already fetched
  for the model picker.
- **Catalog miss.** If the model is not in the catalog, the window is unknown
  and we show tokens only.

### 5.4 Compaction, freshness and cost

- **Compaction.** The `compaction` entry is the signal. As with Claude, the
  state is "compacted, size unknown" until a later assistant reply.
- **Auto-compaction** fires when `contextTokens > contextWindow −
  reserveTokens` (default 16,384). On 128K that is 87.5%.
- **Freshness.** One line per assistant message.
- **Cost.** `pi::tail_info` already reads 256 KiB on growth and caches it in
  `PI_TAIL`. The last assistant usage was within 2.7 KB of the end in every
  session.

## 6. What is impossible or unreliable

1. **Testing a Claude window with `/model` changes the user's default.** In
   2.1.288, `/model X` prints "…and saved as your default for new sessions",
   and it **writes `model` into `~/.claude/settings.json`**.
   - The probe's `/model sonnet[1m]` did this. I restored it at once with
     `/model opus`, using the CLI rather than editing the file, and the file
     is back to `"model": "opus"`.
   - No other Claude session started in that window of about one minute: I
     checked `startedAt` in the session probes.
   - Anyone re-running these probes should know this; it belongs in
     `docs/ai-profiles-manual-checks.md`.
   - It also means a lane's `/model` changes **every later launch's** default
     model when the launch does not pin one. That is out of scope here, but
     worth knowing.
2. **The exact Claude window is not on disk.** The table plus ratchet is right
   for every session observed. It can be wrong in two cases, and §3.3 handles
   both by showing tokens without a %:
   - a 1M-capable model that runs at 200K because of the account latch or
     `CLAUDE_CODE_MAX_CONTEXT_TOKENS`, which would make our % too low;
   - a model the table does not know.
   - Only phase 3 (the statusline) makes the window exact.
3. **Right after a compaction or `/clear`, the size is unknown.**
   - For Claude and pi there is no truthful number until the next reply.
     `postTokens` misses the system prompt and tools, by 50–70K.
   - Codex is the exception, because it logs an estimate.
   - Show "compacted" or "new", never 0%.
4. **Claude subagents and pi trailing estimates are not counted.** Their usage
   is not the main loop's context. pi's footer counts trailing messages, and we
   do not (§5.2).
5. **Codex Desktop rollouts** have no window. They are not lanes, so this
   affects nothing worktrees launches.
6. **`/context` category breakdowns** (memory files, tools, skills) are
   estimates computed inside the CLI. They are on disk only when the user runs
   `/context`. Out of scope.
7. **Numbers for foreign harness sessions**, ones worktrees did not launch,
   work only as well as the existing session-finding: same reader, same limits.

## 7. Shared shape (core)

**One module, `worktrees_core::context`, and one struct.** It is filled by
each harness's existing tail parser. This follows the AGENTS.md rule "keep the
nav dots and MCP on one reader".

```rust
pub struct ContextUse {
    pub tokens: Option<u64>,        // None ⇒ unknown (fresh / just compacted)
    pub window: Option<u64>,        // None ⇒ unknown; never guessed for a %
    pub window_source: WindowSource,// Reported (codex) | Catalog (pi) | LaunchArg
                                    // | Table | Observed (ratchet) | Unknown
    pub used_pct: Option<u8>,       // the CLI's OWN arithmetic (§4.2 for codex)
    pub compact_at: Option<u64>,    // claude: window−33K · pi: window−16384
                                    // · codex: None (not logged)
    pub state: CtxState,            // Measured | Compacted | Fresh
    pub as_of: Option<i64>,         // epoch of the usage line (content, not mtime)
}
```

- **Claude:** extend `agent::transcript_model`'s backward walk (or the app's
  `cached_model` pass) to also return the first real usage it meets.
- **Codex:** `codex_tail` returns `(model, Turn, ContextUse)`.
- **pi:** `tail_info` adds a `ContextUse`.

These are pure functions over `&[String]` lines, so each can be unit-tested
against fixtures captured from the real files above. Each fixture must be
pinned to the CLI version it came from, as the pi fixtures already are.

**`used_pct` is per harness on purpose:**

- Claude: `round(t/w·100)`;
- pi: `t/w·100`;
- Codex: `100 − round((w−t)/(w−12000)·100)`.

The front end never does the arithmetic. It only formats the result.

## 8. Surfaces

The checklist runs by surface, as in [adding-a-harness](../adding-a-harness.html).
Each item below needs to be **seen working** in the real app against a live
lane before it is ticked.

The same session is drawn in three formats in each mockup below:

- a %: `62%`
- a bar: `▰▰▰▰▰▱▱▱`
- tokens: `620K/1M`

### 8.1 Nav row: proposed PRIMARY surface, shown only near the threshold

**Why this is the primary surface:**

- The question it answers is "**which** of my lanes is about to compact?".
  That question spans every place, and the nav is the only surface that shows
  every lane at once.
- The header and the dock show one place, and only after you select it.

**Why it is shown only near the threshold:**

- The row is already dense: dot, name, glyphs, ✎, agent mark, age.
- A % on every row would be noise, because most lanes sit at 5–40% most of the
  time.
- So the row shows nothing until a lane reaches the warn level (§9). It then
  shows a small mark next to `AgentMark`, outside `glyphs()` so that truncation
  cannot drop it, the same way ✎ is placed.
- **Shape carries the meaning, not colour**, because the dot slot already owns
  colour (the `AgentMark` comment says the same).

```
 %       ● context-usage        ⚑  ◆ 86%   4m
 bar     ● context-usage        ⚑  ◆ ▰▰▰▰▰▰▰▱  4m
 tokens  ● context-usage        ⚑  ◆ 860K   4m
         ○ ui-tweaks               ◆        2h      ← below warn: nothing
         ● codex-scroll            ▲ new    1m      ← after /clear or compaction
```

**Recommended format: %.** It is 3–4 characters, has no width jitter if the
number is tabular, and needs no legend. The tooltip carries the rest:
"Opus 5.5 · 860K of 1M · auto-compacts at 967K · as of the last reply 4m ago".

### 8.2 Place header (`.agent-cell`): always-on detail

- It sits next to `.agent-model`, which already says which model the window
  belongs to.
- **It is not the `.status-chip`.** That class is the Claude *service* status
  widget, a name AGENTS.md warns about.

```
 %       claude  Opus 5.5 · 62% of 1M
 bar     claude  Opus 5.5  ▰▰▰▰▰▱▱▱ 62%
 tokens  claude  Opus 5.5 · 620K / 1M · compacts at 967K
 unknown claude  Sonnet 5.5 · 182K            ← window unknown: tokens, no %
 fresh   codex   GPT-6-Astra · compacted — size after next reply
```

Recommended: **"62% of 1M"**. It gives the % and, through the window, which
variant is running. The tokens go in the tooltip.

### 8.3 Dock: no

- The dock is the place's working area: terminal shells, files, docs and plan.
- A context meter there would repeat the header for the same place, in a spot
  that is often closed.
- Not proposed.

### 8.4 MCP `place_status`: phase 1, and the reason the core comes first

Add a `context` object to every agent row (Claude rows currently carry no
model either; add `model` at the same time):

```json
"agents":[{"provider":"claude","state":"idle","model":"claude-opus-5-5",
  "context":{"tokens":620412,"window":1000000,"window_source":"table",
             "used_pct":62,"compact_at":967000,"state":"measured",
             "as_of":"2026-10-03T18:14:24Z"}}]
```

- This lets the orchestrator act: "lane X is at 91% with 40K to auto-compact:
  tell it to `/compact` or to write its hand-off now."
- It is also the cheapest surface to verify by seeing it, since one tool call
  shows it.
- **`wait` is not changed.** A context threshold is not a turn end.

### 8.5 `worktrees ls --json`

`ls --json` carries no agent fields today. `agent_sessions` exists only in the
app snapshot.

Two options:

- add `context` under a new per-place `agent` object, at the cost of reading
  transcript tails on every `ls`;
- or leave `ls --json` as git/tmux state only.

**Recommendation: leave it out.** `place_status` is the agent-facing read, and
`ls` should stay fast and agent-agnostic. This is §11, question 4.

### 8.6 How it reaches the app without re-listing every turn

Models reach the front end today through `places:changed`. That makes the
front end re-pull `snapshot()`, which is a git fan-out. Context changes on
**every** assistant message, so using the same path would re-list the
workspace every few seconds per busy lane. **Do not use that path.**

Instead:

1. The 3 s poll already stats each probe or rollout and re-reads the tail on
   growth (`claude_activity`, `codex_tick`, `pi_tick`). It collects
   `ContextUse` per place from those reads.
2. It emits a new `sessions:context` event, `{path → ContextUse}`, only when a
   value changes **by a whole percentage point or by state**, the same way
   `sessions:busy` is emitted only on change.
3. The front end keeps it in a map beside `busyPaths`, as the busy and waiting
   sets are kept.

There are no new reads, no new spawns and no re-list. This goes in the
harness checklist's §2 list of consumers: the core reader, MCP, the app poll
and the front end.

## 9. Thresholds

**The problem with a fixed % of the window:**

| Window / harness | Auto-compact point | As % of window |
|---|---|---|
| Claude 1M | 967K | 96.7% |
| Claude 200K | 167K | 83.5% |
| pi 128K | 111.6K | 87.2% |
| pi 262K | 245.7K | 93.8% |
| Codex 258.4K | ~231–245K (observed) | ~90–95% |

A fixed "≥ 80% of window":

- warns at 800K on Claude 1M, 167K before it compacts, which is too early to
  mean anything;
- warns 7K before compaction on Claude 200K, which is too late to act on.

**Proposal: measure "headroom", the share of the auto-compact point that has
been used.**

- **Warn** when `tokens ≥ 0.85 × compact_at`. The nav mark appears.
- **Critical** when `tokens ≥ 0.95 × compact_at`. The mark changes shape (for
  example `◆` → `◈`), and MCP adds `"near_compact": true`.
- When `compact_at` is unknown (Codex, or an unknown window), fall back to the
  window: warn at 75% and critical at 88%. For Codex that sits just under the
  observed compaction band.
- **Display stays % of window**, as each CLI shows it, so the numbers agree.
  Only the decision to *show* the mark uses headroom.

**A separate reason to care at 1M:** quality worsens well before 967K, so a
user may want to compact by choice at, say, 400K. That is a preference, not a
fact we can derive. It is §11, question 3.

## 10. Phases

**Phase 1: core reader + MCP.**

- Add `worktrees_core::context` with the three parsers, folded into the
  existing tails (`transcript_model`'s walk, `codex_tail`, `tail_info`).
- Add the Claude window table and ratchet, and parse the context column of
  `pi --list-models`.
- Add `context` and Claude `model` to `place_status` agent rows.
- Fixtures: redacted tails of a Claude transcript with a `compact_boundary` and
  a synthetic zero-usage line, a Codex rollout with `compacted` and the
  0-input estimate, and a pi session with a `compaction`. Each is pinned to its
  CLI version.
- **Each new test must be shown red first**, for example by summing
  `output_tokens` into the Claude numerator or by dropping the 12K baseline.
- **Seen when:** `place_status` on a live Claude, Codex and pi lane each shows
  the same number as that lane's `/context`, `/status` or footer.

**Phase 2: app.**

- Add the `sessions:context` event from the existing poll, the nav mark gated
  by headroom, and the header detail.
- Add a mock-harness command entry, since `install.ts` must track every
  command.
- Add a drift check, `context-check.mjs`, so that `compact_at` and the warn
  fractions in TS cannot drift from the Rust ones. This is the same shape as
  `dnd-check.mjs`.
- **Seen when:** one live lane is driven past the warn level in
  `sandbox.sh --app`, and the mark appears on its row while another place is
  selected.

**Phase 3 (optional, needs a decision): exact Claude window.**

- Claude hands its statusline command JSON that contains `context_window_size`
  and `used_percentage`.
- Worktrees could pass `--settings '{"statusLine":{…}}'` **per launch**. That
  is argv we supply, not the repo, so ADR 0001 is fine, and nothing is written
  to `~/.claude`.
- The command would be a tiny `worktrees statusline` that writes the JSON to a
  per-session cache file and prints the user's own statusline output.
- **Costs:**
  - It takes over the statusline slot, so the user's own statusline has to be
    chained.
  - Claude spawns it on every statusline refresh, which is new process churn
    on Claude's side.
  - It is another launch-time behaviour, to be re-checked with every Claude
    upgrade.
- **Only worth it if** the table-plus-ratchet window turns out wrong in
  practice.

## 11. Open questions for David

1. **Exact Claude window: is phase 3 wanted?** The alternative is the table
   plus ratchet, which is right on every session measured and blind to the
   1M-credits latch and to a `/model …[1m]` typed inside the lane. The
   statusline route is exact, but invasive.
2. **The primary surface.** Is a nav mark shown only near the threshold right,
   or do you want the % always visible on every row? An always-on % costs row
   width on every place and makes the nav busier.
3. **A personal ceiling on 1M sessions.** Should there be a setting such as
   "warn me at 400K regardless of window", for quality rather than for
   compaction?
4. **`ls --json`.** Add agent context to it, which reads transcripts on every
   `ls`, or keep it to git/tmux state and leave context in `place_status`?
5. **Codex's baseline.** Mirror Codex's "% left" arithmetic, so that 30.9K of
   258K shows as 8% used and agrees with `/status`? Or use plain
   tokens ÷ window (12%), which is consistent across harnesses but disagrees
   with the lane on screen?
6. **The `/model` side effect (§6.1).** Should this go into ROADMAP or
   AGENTS.md as a known hazard? A lane that runs `/model` silently changes
   David's default model for every new session that does not pin one.
