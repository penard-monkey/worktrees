# 2026-11-05 — chat composer: place references and several lines

- **Date:** 2026-11-04 → 2026-11-05
- **Place:** `chat-composer-refs-lines` (a lane briefed by `(main)`)
- **Branches:** `chat-composer-refs-lines` (#611), `chat-composer-refs-lines-multiline` (#613), both squash-merged, then `chat-composer-refs-lines-close-out`
- **PRs:** [#611](https://github.com/penard-monkey/worktrees/pull/611), squash `5f19314`; [#613](https://github.com/penard-monkey/worktrees/pull/613), squash `3354fc7`
- **Release:** none (under `[Unreleased]`)
- **Planning files:** none

David's request: in Chat view, dragging a place into the chat added no reference.
A second request: the chat should take several lines, with Shift+Enter and
Cmd+Enter adding a line. **Neither has been driven in the real app yet.** Each PR
has a manual check for David at the end of its body.

## What shipped

**#611: drop a place on the chat**

- `app/src/App.tsx`, `ChatPane.tsx`, `App.css`: the whole chat, transcript included, is a drop target (`data-drop="mention"`, `data-drop-into="chat"`).
- `app/src/dnd.ts`: `referenceInDraft`, a pure insert rule, and `REFERENCE_LEAD`.
- `app/src/chat.ts`: `insertReference` and `tookReference`. The reference waits in the store until the composer takes it, because only the composer knows the cursor.
- `crates/worktrees-api/src/ops.rs`, `crates/worktrees-cli/src/serve/dispatch.rs`: a new read row, `place_reference`, sharing `reference_for` with `drop_reference`.
- `app/src/navdrag.ts`: Escape during a drag is taken in the capture phase, so it cancels the drag and interrupts nothing.
- `app/scripts/chat-drop-browser.mjs` (new), `drop-check.mjs`.

**#613: several lines, and a long message that stays the person's own words**

- `crates/worktrees-core/src/harness.rs`: `typed_message`, `MESSAGE_MAX`, `line_breaks_proved`, `send_caps`, the stop outcomes on `SendOutcome`, and Codex's multi-line paste.
- `crates/worktrees-core/src/prompt.rs`: `claude_chunks`, `Pace`, `claude_batches`, `feed_claude`, `drew`, `read_box`, `box_window`, and a verdict on every look in `submit_claude`.
- `crates/worktrees-core/src/agent.rs`: `composer_text`, `composer_box`. `tmux.rs`: `literal_arg` (a trailing `;` survives), `paste_batch`, `pane_size`. `turns.rs`: `pasted_body`, and queued messages keep their line breaks.
- `crates/worktrees-api/src/agents.rs`: `agent_send` reports a stop with `ok: false`, `stopped`, `pasted` and `total`, and a stop is not stamped. `send_note` holds the words.
- `crates/worktrees-cli/src/serve/wire.rs`: `send_caps` on the remote `ok`, with no `PROTO` bump. `audit.rs`: a scripted clock for the budget tests.
- `app/src/chat.ts` (`SendRules`, `draftIsOwnSend`, stop words), `ChatPane.tsx` (keys, field growth), `ipc.ts`, `ipcremote.ts`.
- `docs/claude-paste-manual-checks.md` and the fixtures under `crates/worktrees-core/tests/fixtures/claude-screen/2.1.318/multiline/`, whose README holds the measured window table.

## Decisions

- **`place_reference` is a row, not an argument on `drop_reference`.** An older host ignores an unknown argument and would paste. It answers an unknown row with `unknown command`, and the app can say so.
- **A reference that would start a message gets a visible, editable `Look at `.** `typed_line_ok` refuses a leading `@`, and a plain drop into an empty draft would have been a dead end. The host's rule is unchanged, and a draft is never changed to pass it.
- **Small separate pastes into Claude, not one big one.** One paste collapses at 4+ lines or 800–850 characters and is recorded as `<pasted_content>`, so the agent treats the person's words as pasted material. Measured: Haiku called a queued 5-line message "pasted text with no request in them". Separate bracketed pastes stay literal.
- **The proof looks during the feed as well as before Enter.** The box scrolls, so the final proof sees only the last rows of a tall draft. Before every call, the box must end in what has gone in so far.
- **A call adds at most `window - 2` rows, counted at half the width.** Any two rows a line wraps to hold at least the width between them. Short panes pay up to about 3x for this, and stay far from the 20 s bound.
- **A false stop costs one trip to the terminal; a mixed send costs a wrong message.** So every doubt stops, keeps the draft, and tells the person to look at the terminal.
- **`typed_line_ok` is unchanged.** MCP `send`, `agent_answer_text` and slash arguments stay one line. Only `agent_send` takes `typed_message`, and only for Claude and Codex.
- **A later `@<real file>` attaches the file.** Measured and accepted, the same as `@` mid-line today.

## Dead ends / gotchas

**Claude 2.1.318's paste behaviour is undocumented** (fixtures README):

- The box's window is 3 rows up to a pane of 17, then `rows / 2 - 5` (33 heights measured).
- It wraps at `cols - 4`, at a word and at `/`. Four 40-character tokens take 4 rows at 80 columns.
- A leading newline stays as a blank first row.
- Past about 10,000 characters the middle folds to `[...Truncated text #N +M lines...]`, and the message still submits whole.
- `paste again to expand` hides the status row for about 8 s after a collapsed paste, so Stop cannot act during those seconds.
- tmux's argv parser eats a trailing `;`.

**Holes found on the way.** Nothing shipped has them.

- Words typed mid-send were really submitted (`delivered: true`, typed words at offset 891 of 6046), because the final proof saw only the last rows.
- Enter at the terminal submitted the rest as a second message, also `delivered: true`. The next paste landed in the empty box, whose end still read as the message. Found live.
- The first fix for that passed only because my fake pane dropped a leading newline. Claude keeps it as a blank row.
- Call sizing was wrong in three rounds: bytes rather than rows, 3/4 width under word wrap, and a call that could fill the window exactly. Each was found by reading past a fake that could not express it.
- False stops: the fold, torn frames (inferred from timing, not seen), and a slow last draw. 5 of 15 sends stopped falsely before the fixes.
- A budget test decided by the wall clock failed once in CI, because a runner's delay paid a refill. A 20 s delay fails it every time. It now runs on a scripted clock.

**My mistakes during this session**

- **A scripted paste reached a Codex trust prompt.** The script pasted into the modal, and its Ctrl-C quit Codex. The later pastes then ran as shell lines in a scratch directory. No trust was saved (`wtm` has 0 hits). Codex later ran in this place's tree, then in a scratch place with a guard. Its rollouts remain in `~/.codex/sessions`, where `(main)` asked that nothing be deleted.
- **A diagnostic double Ctrl-C quit a scratch Claude,** and source text was pasted at a zsh prompt. It was bracketed, so nothing executed. The harness now clears with Esc only.
- **The first Escape browser test passed on unfixed code.** Playwright moves focus and WKWebView does not, so the test now re-focuses mid-drag.
- **Mutation runs that proved nothing at first.** An audit-cost mutant did not apply, and an audit assertion could pass vacuously. Both were fixed. One weak settling mutant stayed green, and a stronger one went red.
- **A frozen clock hung a test.** A pattern-based kill was refused by the auto-mode classifier, rightly, as it could have hit other lanes. I stopped it with TaskStop.
- **A typo ran `gates2.sh --help` outside the gate slot.** I stopped it and removed the stray `--help*` logs.
- Smaller ones: `worktrees rm` stuck on its prompt (MCP `remove_worktree` worked), and zsh's `=` trap.

**Traps for the next lane**

- Rebases conflicted in dispatch's count pins, `turns.rs`, `wire.rs` (`push` beside `send_caps`) and CHANGELOG.
- A browser Escape test needs focus put back, or it does not test WKWebView's case.

## Verification

- **Claude's real path, on the proof of `722953a`:** real `agent_send` and tmux against Claude 2.1.318 launched by `worktrees new`, through `tests/agents_e2e.rs`.
  - False stops: 0 of 104 (54 under 10,000 characters, 18 at 12–16 KB, 32 across heights 12–55). That bounds the true rate below about 3% at 95% confidence, so it is evidence, not proof.
  - Typing mid-send: 60 of 60 stopped, 0 mixed. 4 more runs were refused before pasting and are not counted. Controls: 30 of 30 exact.
  - Enter mid-send: 60 runs, each with exactly one new record. 58 stopped; 2 delivered whole because Enter hit an empty box at 0.04 s.
  - Timing: medians 0.64–1.18 s up to 8 KB, 1.63 s at 16 KB. 16 KB takes 2.30 s at 80x24, 3.25 s at 40x24, 4.72 s at 120x12 and 5.78 s at 80x16. The worst case, 16 KB of one-character lines, took 18.81 s and was delivered.
- **Codex 0.162.1, real path:** one line, 3 lines, later commands, a 1500-character chip and 16 KB all exact. This run predates the last two rounds.
- **Mutation:** 26 mutants across three rounds, all red once strong enough.
- **Browser:** `chat-drop-browser.mjs` (34 checks) and `chat-lines-browser.mjs` (94 checks), real input, Chromium and WebKit, mock harness. Neither is in CI.
- **CI:** 9/9 on `722953a`, the tree that merged (`git diff 722953a 3354fc7` is empty).
- **Local gates, before #613's merge:** all passed, except that `worktrees-cli` ran with `--skip serve::remote`, because `serve::remote` and `remote_bin` fail or stall on this Mac. No container runs, so Linux rests on CI.
- **This close-out:** docs only, so the local gates were not re-run; no code changed since the merge.
- **Reviews:** three fable rounds (`~/.cache/worktrees/worktrees/main/review-611.md`, `review-613.md`, `review-613-2.md`, `review-613-3.md`). The third found no must-fixes: 3 × 374,698 modelled checks with 0 bad.

## Follow-ups

The unfinished items are in ROADMAP, under this session. David's manual checks from #611 and #613 are the first real-app run of both.

**Proposed for AGENTS.md: not committed, awaiting David's OK**

1. **Claude's paste thresholds and fold are undocumented, so a draft's proof must look during the feed, not just before Enter.** Tall drafts scroll out of the window and a long one folds, so a check before Enter cannot see the start of what it is about to submit.
2. **A scripted scratch agent is cleared with Esc only.** A second Ctrl-C quits it, and later pastes land in a shell.

One correction goes with them: the agents paragraph says `agent_send`'s text "passes `harness::typed_line_ok`". Since #613 it passes `harness::typed_message`, which shares that rule's body and allows line breaks only where measured safe.
