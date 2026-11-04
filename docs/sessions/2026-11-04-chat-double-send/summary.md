# 2026-11-04 — chat double send

- **Date:** 2026-11-04
- **Place:** `fix-chat-double-send` (a lane briefed by `(main)`)
- **Branch:** `fix-chat-double-send` (squash-merged), then `fix-chat-double-send-close-out`
- **PR:** [#606](https://github.com/penard-monkey/worktrees/pull/606), squash `6f14ca7`
- **Release:** none (under `[Unreleased]`)
- **Planning files:** none

David's report: "sometimes the tool shows messages twice when I send a
message." Claude received each message once, so this was a display bug. A dashed
"Delivered" bubble stayed beside the real one, and it did not clear.

## What shipped

- **`app/src/chat.ts`**
  - `isRecordOf`: a sent line matches its transcript record when the trimmed
    text is equal, or when the record is the host's exact cut (`HOST_TEXT_MAX`
    code points plus `…`).
  - `sendDraft`: settles the new Pending against the turns already held when
    `agent_send` returns.
  - `Pending.held`: the copies of the same text already held when the send
    began. Those can never settle it.
  - `sentLine` and `draftIsOwnSend`: the "unsent text" notice ignores a composer
    sample that is only this window's own line.
- **`app/src/ChatPane.tsx`**: the notice goes through `draftIsOwnSend`.
- **`app/src/mock/install.ts`**: the mock records a sent line trimmed, as Claude
  does.
- **`app/scripts/chatsend-check.mjs` (new)**: runs the real `sendDraft`,
  `loadTurns`, `settlePending` and `itemsOf` on promises and timers it resolves
  by hand.
- **`app/scripts/chat-queued-browser.mjs`**: adds the trailing-space send and
  the notice, in Chromium and WebKit.

## Decisions

- **Compare, don't trim what is sent.** The line still goes out as typed. This
  covers any harness that changes a line's ends, and changes nothing agents
  receive.
- **Settling on landing needs `held`.** Without it, an earlier "yes" settles a
  second "yes" at once. Main already had a milder form of this: a read holding
  only the first echo hid the second bubble until its own echo arrived.
- **Only the host's exact cut counts as a clip.** The first version accepted any
  record ending in `…`, so a typed `ok…` settled `ok, go ahead`. Found in review.
  `chatsend-check` holds the constant to `turns.rs`.
- **The own-send window ends in the store, not at render.** `sessions:drafts` is
  change-gated, so if a person really types the same words, nothing else redraws
  a hidden notice. Review suggested a timer in `Composer`; `(main)` accepted one
  in `sendDraft`, because it can be tested on the real send path and leaves no
  age check in the rule. A later send clears `sentLine` in its first step.

## Dead ends / gotchas

- **The timing theory explained neither screenshot.** The brief's hypothesis was
  an echo read landing during `agent_send`. That order is real, but sends took
  533–922 ms and the next read clears it. The cause came from the audit log:
  `"make it a PR "` was 13 bytes, its hash matched only the trailing-space
  candidate, and the transcript held 12 bytes.
- **Audit data decided it.** Of 50 sends compared with transcripts by hash and
  length, 36 were exact and the other 14 were each the record plus one trailing
  space. No message text needed copying. `audit.jsonl` has `ms`, `bytes` and
  `sha256` for every `agent_send`.
- **A report said this was already fixed on main. It was not.** Main still had
  `m.text === p.text`, the new check was red on that commit, and the screenshots
  came from v0.47.0, which already includes #596.
- **The notice was a separate bug.** serve samples the composer every 15 s, and
  one sample landed in the ~600 ms between paste and Enter. `serve.log` showed
  `drafts: 1` at 22:30:03 and `drafts: 0` at 22:30:18.
- **The scratch Claude probe never ran.** The first try failed on zsh's lack of
  word splitting (`$T` holding `tmux -L x`), a trap AGENTS.md lists. In the bash
  rewrite the private tmux server was gone by the first capture. The cause was
  not found, and the probe was dropped because only 4001–4096 character lines
  depend on it. `~/.local/bin/claude` still pointed at the real install
  afterwards.
- **`await` on an async helper that returns a promise waits for it.** The check's
  first draft would have deadlocked, so `startSend` returns `{ done }`.
- **A `data:` module cannot resolve `import("./ipcws")`.** The refused-upload
  path threw inside the check until that import was stubbed.

## Verification

- **Real app, by David, on v0.47.0, relayed by `(main)`:** two sends in one place
  differing only by a trailing space. The one without showed once; the one with
  stayed doubled under Claude's reply. A third send, ending in a space by its
  audit byte count and hash, doubled too. This shows the bug, not the fix.
- **`chatsend-check`:** 12 failures on main's source, and 12 more for the review's
  cases on the first commit's source, then green. 18 of 18 single mutations of
  the fix went red.
- **`chat-queued-browser`:** passes in Chromium and WebKit against the mock, and
  fails with exact matching restored.
- **Local:** 65 of 65 `*-check.mjs` from the repo root, and `tsc`.
- **CI:** all 9 jobs green on the PR's head, `0a57fd4`.
- **Not run:** the fix in the real app. The Rust and bats gates were not run
  locally, since no Rust changed; CI ran them.

## Follow-ups

- **David's manual check** once the fix ships: the four steps in #606.
- **On ROADMAP:** the stale nav ✎ and ⌘K, a 4001–4096 character line against a
  live Claude, and Codex/pi/kimi unsurveyed.
- **iOS:** `worktrees-ios/src/session.ts` reads as having the same bugs (code read
  only, not run). `(main)` is passing this to that session.
- **Proposed for AGENTS.md, for David's OK (not applied):** "The audit log holds
  `ms`, `bytes` and `sha256` for every `agent_send`. To test a theory about a sent
  line, hash candidates against it before reading code. That is how the trailing
  space was found, without copying any message text."
