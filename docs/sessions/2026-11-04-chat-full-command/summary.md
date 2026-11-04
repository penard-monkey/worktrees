# 2026-11-04 — chat tool calls open on click

- **Date:** 2026-11-04
- **Place:** `chat-full-command` (a lane briefed by `(main)`)
- **Branch:** `chat-full-command` (squash-merged), then `chat-full-command-close-out`
- **PR:** [#607](https://github.com/penard-monkey/worktrees/pull/607), squash `f3e0cd0`
- **Release:** none (under `[Unreleased]`)
- **Planning files:** none

David, from a screenshot: "It would be great to be able to click and see the
full command, they should stay collapsed like that unless needed." Mid-way he
widened it through `(main)`: MCP calls open too, and the phone must be able to
use the same answer.

A row's `title` looked like the fix and was not. `summary` is only a command's
first non-empty line, clipped to 200 characters, so the backend had to supply
the rest.

## What shipped

- **`crates/worktrees-core/src/turns.rs`**
  - `Step::Tool` gains optional `call_id` and `detail`, and an unserialized
    `ran` (`Ran`: nothing, a command's parts, or a JSON input).
  - `claude_tool_call` and `codex_tool_call` answer `CallDetail {tool, input,
    truncated, bytes}`, over the same turn walk `agent_turns` uses (`claude_turns`,
    `codex_turns`), so they read `assistant` `tool_use` blocks only.
  - `PATH_ONLY` is the named exclusion: Edit, MultiEdit, Write and NotebookEdit
    keep their path and nothing else.
  - `clean_text` and `clean_json` keep line breaks and tabs, and remove control
    and invisible characters, inside JSON strings and keys too.
- **`crates/worktrees-api/src/agents.rs`**: `agent_tool_call`, which searches the
  place's Claude and Codex agents, and the `gone` refusal (`ERR_GONE`).
- **`crates/worktrees-cli/src/serve/dispatch.rs`**: the row, `Engine [read,
  place(repo, slug)]`. The table is now 195 rows.
- **`app/src/chat.ts`**: `callKeys`, `toggleCall`, `readCall`, `CallRead`; open
  state lives in the store, by call id.
- **`app/src/ChatPane.tsx`**: `CallRow`, `CallChip`, `CallFull`, `useClipped`,
  `useRevealOnChange`.
- **`app/src/stickToBottom.ts`**: `reveal` and `settle`, so opening a row is the
  reader's act and not a change to follow.
- **`app/src/mock/install.ts`**: the row, with `?toolcall=old|gone|fail|cut|slow`.
- **`app/scripts/chat-call-browser.mjs` (new)**: 110 checks in Chromium and
  WebKit. It needs `PLAYWRIGHT_MODULE` and a mock on port 1472 (or
  `CHAT_CALL_URL`), and CI does not run it.
- **`docs/proposals/ios-mvp.md`**: the contract.

## Decisions

- **Fetch on click, by measuring.** Bash commands: n=1,324, p50 309, p90 1,753,
  max 15,015 characters, 68.7% over the clip. A 20-turn window holds a median
  19 KB of Bash text against 4.6 KB of summaries, and 3 of 40 windows exceed the
  64 KB budget on that alone. MCP inputs are small (n=224, max 6,654), but they
  have no line to fall back on.
- **`read` class, no step-up, and no redaction.** David decided against
  redaction. This is a real widening for a paired device: anything a tool was
  handed now reaches anything with `read` on the place, where before only a
  command's first 200 characters did. The PR body says so.
- **The contract follows what the iOS lane asked for:** `detail` is the only thing
  a client keys on, so no client keeps a list of tool names. A structured input
  past 64 KiB comes back as text, never as broken JSON.
- **No `session` argument.** The id addresses the call by itself, so the row
  searches the place's agents, and a rotated session is simply `gone`.
- **Opening may scroll, against the brief's "must not jump".** A call that fits
  moves nothing. One that doesn't scrolls just enough, stopping with its row at
  the top. Never scrolling would open a row near the bottom with its text out of
  sight. Stated in the PR for review.
- **No nested scroller.** The chat's scroller is the only one, at any height.
- **Agent/Task opens to its prompt**, because that is its command.
- **A file tool's path may open** (review): the path is not content, so only a
  path the line cut says `detail`.

## Dead ends / gotchas

- **A green suite of scroll checks that tested nothing.** All 100 browser checks
  passed first time, and removing the scroll logic left them green: the bounds
  were loose enough to accept a row that never moved. Only the mutation run showed
  it. Tightened, the same mutation puts the row at −96 px.
- **A vacuous regex survived to review.** `chatsteps-check` looked for the text
  inside a button, in a slice that started after the component holding the
  `<pre>`, so it passed with the text inside the button. Review found it. It now
  asserts `<CallFull` is not inside a button, shown red by moving it there.
- **`one_line` hides a long line from a "says more" test.** A 201-character
  command and its clipped summary compare equal after clipping, so the length has
  to be checked as well.
- **A pretty-printer hides what a text cleaner would need to see.** Once printed,
  an escape byte is the six characters `\u001b`, so a JSON input is cleaned before
  it is printed, not after. Cleaning can also merge keys that differ only by a
  removed character, so the later one is kept as `<key> (2)`.
- **Existing tests pinned "no tool ids on the wire"** (`toolu_`, `call_`). Exposing
  `call_id` is deliberate, so they now strip that one key and keep looking for ids
  anywhere else (`without_call_ids`).
- **`chat.ts`'s dynamic `import("./ipcws")` cannot load in a `data:` module.** The
  check loads the real `ipcws.ts` instead of stubbing its rule.
- **`nvm use` did not take in a tool call's shell**: `node -v` stayed 26.12 with
  `.nvmrc` at 22.23. The run used the pinned bin on `PATH` directly.
- **A local `cargo test -p worktrees-cli` hung** for about 30 minutes in
  `serve::remote::tests::a_proven_client_below_the_protocol_floor_is_told_to_update_and_never_served`.
  I killed it and did not find the cause. The same test passed in CI's
  `rust (macos-latest)` on all three heads I checked.
- **A flex-button mutation failed by a timeout I did not trace**, so it proves
  little. The WebKit evidence is the direct measurement: button and plain rows
  match on height, left edge and line styling.

## Verification

- CI: all 9 jobs green on the final head `c29d187` (run 41934727511), before the
  squash.
- Local, final tree after merging main: `cargo check --workspace --all-targets`,
  `tsc --noEmit`, and all 65 `*-check.mjs` from the repo root.
- Local, on the review-fix head: `cargo test -p worktrees-core` (1,118 passed, 2
  ignored), the api `agents::` tests, `chat-call-browser.mjs` (110).
- Local, before the review fixes: release build, bats 592/592, lint, full api suite,
  `cargo test -p app --lib` (41), `cargo check -p app`, `chat-scroll-browser.mjs`
  (38). **`cargo test -p worktrees-cli` never finished locally**: CI is its only
  run on this branch.
- Shown red by mutation: 12 Rust rules, 6 `chat.ts` rules, and the scroll logic.
  The review fixes went red before their code changed.
- **Not done:** nothing was driven in the real app or in WKWebView, only Playwright
  Chromium and WebKit 26.0. David's manual check is in the PR body, and I have no
  result for it. The phone (worktrees-ios PR #22) checked the contract by reading
  its code and has run only against its mock.

## Follow-ups

- In `ROADMAP.md`: the `permission` step, the local hang, and the CI flake
  `(main)` reported.
- **MCP tools handed file content show it.** The exclusion goes by Claude's four
  tool names. `(main)` is putting that to David.
- **Proposed for `AGENTS.md`, not committed (needs David's OK):**
  - "The 194 commands" should say 195.
  - A note beside the serve rows: `chat-call-browser.mjs` needs `PLAYWRIGHT_MODULE`
    and a mock on port 1472 or `CHAT_CALL_URL`, and CI does not run it.
