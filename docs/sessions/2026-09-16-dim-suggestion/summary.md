# Session: the ✎ no longer lights for Claude's own dimmed suggestion

- **Date:** 2026-09-16
- **Worktree:** `bug-fixes` (idle base `bug-fixes-next`)
- **Branch:** `bug-fixes-dim-suggestion` off a freshly fetched `origin/main`, deleted after merge
- **PR:** [#210](https://github.com/penard-monkey/worktrees/pull/210) → `0cbdce7` (two commits, squashed)
- **Planning files:** `planning.tar.gz` beside this file
- **Scratch:** `~/.cache/worktrees/worktrees/bug-fixes/dim-suggestion/` — the 34 `capture-pane -e` screens the rule was measured on, `expected.json`, and the gate logs

## Context

David: "When I type something in the claude session we show the pen/pencil on
the left nav. However, when claude queues up its own suggestion it shows like
a modification. Is there a way to not show when the text there is from claude?
Is this because I haven't updated [claude] in a while?"

Not a version issue. App, CLI and installed bundle were all 0.23.1; the panes
on the machine ran Claude Code 2.1.269–2.1.273 and every one painted its
suggestion the same way. The ✎ (#191, v0.22.0) read each pane with
`capture-pane -p`, which drops styling, so Claude's suggested follow-up and
the user's typing were byte-identical to the parser.

## What shipped

- **`scan_drafts` captures with `-e`** (`app/src-tauri/src/lib.rs`), so the
  attributes survive.
- **`agent::draft_from_screen` reads the attribute and strips the rest**
  (`crates/worktrees-core/src/agent.rs`): `strip_escapes` removes CSI and OSC
  sequences for the text; `body_starts_dim` walks from the prompt char past
  whitespace and escapes to the first visible character and answers whether
  SGR 2 is in effect there; `apply_sgr` folds one parameter string into the
  flag, consuming `38/48/58` colour arguments in the `;` form and ignoring
  `:` sub-parameters so a channel value of 2 is not read as dim. A dim body
  yields no draft — that covers the suggestion and the empty box's `Try "…"`
  placeholder. Plain captures have no escapes and behave exactly as before.
- CHANGELOG `### Fixed` entry under Unreleased.

## How it was found

Captured all 34 idle Claude panes with `capture-pane -e -p` and dumped the
prompt line's bytes. Every suggestion was `ESC[39m❯` + U+00A0 + `ESC[2m` +
text; every typed draft was `ESC[39m❯` + U+00A0 + text with no attribute, and
one selected draft carried a background colour (`48;5;66`) *before* the space
— which is why the walk skips whitespace and escapes rather than testing the
byte right after the prompt. A wrapped suggestion drops its `ESC[0m` onto the
next line, which is why only where the body STARTS is examined.

## Verification

- Two new parser tests, each shown red first. `claudes_own_suggestion_is_not_a_draft`
  PASSED against the old parser — for the wrong reason (the leading `ESC[39m`
  meant no prompt line was found at all) — so it was proved against stripping
  alone (red) before the dim gate went in (green).
- A throwaway integration test ran the parser over the 34 real captures:
  30 suggestion/empty → `None`, 4 typed → `Some`, matching the measurement.
- Gates: fresh release build, bats 335 ok / 0 not ok, lint, core 300, cli 7,
  app --lib 47, tsc, `cargo check -p app`. CI: all nine checks green.

## The review pass

A `/code-review` on the diff found three real gaps, all fixed in the second
commit:

- **`capture-pane -e` emits bare SO/SI bytes** (0x0e / 0x0f) around DEC
  line-drawing runs — not CSI, so the first stripper left them in the text.
  Confirmed on tmux 3.7b with a throwaway session, not from documentation.
- **Dim set BEFORE the `❯`** and still in effect at the body read as not-dim,
  because the scan started at the prompt char. It now tracks SGR from column 0.
- **An OSC hyperlink between the prompt and the body** derailed the walk.

`strip_escapes` and `body_starts_dim` were also folded onto one `skip_escape`
scanner; two independent escape parsers in one file is a drift bug waiting to
happen.

The review itself was a mistake in shape: it fanned out to eight finder agents
and fourteen verifiers, ~30 minutes and ~153k tokens on a 200-line diff, and
David stopped it mid-verify. The findings above were recovered from the
verifiers' own transcripts. A diff this size gets an inline read, not a
fan-out.

## Left open

- **Is queued text in a BUSY pane drawn dim?** If Claude renders text typed
  during a turn the way it renders a suggestion, this change would hide real
  queued drafts and make `Draft::queued` dead. No busy pane on the machine had
  typed-but-unsent text at capture time, so it was never observed either way.
  The existing `a_busy_pane_still_yields_its_queued_text` covers only the
  unstyled shape. On the roadmap and in the manual checks.
- **`CHANGELOG.md` on main has two `## [Unreleased]` headers**, from a merge
  that landed while this branch was open (not from this work). Release cuts the
  section, so the second block would be left behind. On the roadmap.

## Lessons

- **A test that passes on the old code is not yet evidence.** The suggestion
  fixture returned `None` before the fix because the escapes broke the parse,
  not because the rule existed. Proving it red needed a version of the code
  that parsed styled input but lacked the gate — the intermediate step the
  repo's "show it FAIL first" rule implies but does not spell out.
- **A PR that sits open behind a slow review goes stale under you.** Two
  sessions merged while this one reviewed; `CHANGELOG.md` conflicted, and a
  conflicting PR gets NO CI run at all — GitHub cannot build the merge commit,
  so `gh pr checks` says "no checks reported" and polling for a run that will
  never be created reads exactly like a queue backlog. Check
  `mergeStateStatus` before waiting on CI.
- **Text-only captures cannot carry provenance; styling can.** The one bit
  that separates "you typed this" from "claude drew this" is an SGR attribute.
  The rule is now coupled to how Claude Code renders its suggestion (dim), so
  it belongs on the manual claude-upgrade checklist — no fake claude in bats.
- **Two of three hardening assertions were proven red; the third was not.**
  The OSC case sits behind an earlier failing assert in the same test, so the
  run aborted before reaching it. Recorded rather than claimed.
