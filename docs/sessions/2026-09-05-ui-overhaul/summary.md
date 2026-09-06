# Session: UI overhaul — one sidebar control, Settings modal, usage metrics, unsent-prompt marker

- **Date:** 2026-09-05
- **Worktree:** `ui-tweaks` (idle base `ui-next`)
- **Branches:** `ui-tweaks-header-chip`, `ui-tweaks-sidebar`, `ui-tweaks-settings-modal`,
  `ui-tweaks-usage`, `ui-tweaks-drafts` — each off a freshly fetched `origin/main`,
  all deleted after merge
- **PRs:** [#187](https://github.com/penard-monkey/worktrees/pull/187) → `80a46f9`,
  [#188](https://github.com/penard-monkey/worktrees/pull/188) → `be92edb`,
  [#189](https://github.com/penard-monkey/worktrees/pull/189) → `33af14d`,
  [#190](https://github.com/penard-monkey/worktrees/pull/190) → `92559eb`,
  [#191](https://github.com/penard-monkey/worktrees/pull/191) → `a13a4c6`
- **Release:** none — everything sits in `[Unreleased]` on top of v0.20.0; not
  installed locally either (David still runs the pre-session build)
- **Planning files:** `planning.tar.gz` here — task_plan, findings, progress
- **Design canvas:** the approved mockups (sidebar states, header, compact notes,
  Settings modal, usage heatmap, draft marker) — a Claude Design canvas, private
  to David's account, id `40e61a3a-d944-4788-8e6a-20d44612ffa4`

## Context

David asked for several things in one breath: metrics on which parts of the app
get used ("a heatmap"), so chrome can be hidden or collapsed with evidence; the
left nav's two toggles (the Places lens icon and the rail's hide button) folded
into one with a hover-reveal state that stays keyboard-navigable; a doubled chip
in a renamed place's header (screenshot in `_tmp/`); shorter release notes;
Settings out of the right-hand sheet; and a signal for a Claude session that has
a typed-but-unsent prompt. He asked for "our design skills", so the session
opened with research + a design canvas, then five PRs in the order the canvas
note proposed. Implementation ran on opus agents from written briefs; fable
reviewed each diff and merged (David's cost split).

## What shipped

1. **Header chip** (#187, `app/src/App.tsx` topbar identity, `app/src/mock/fixtures.ts`)
   — the branch chip renders only when `is_main || branch !== slug`, the rule the
   nav row's `divergent` already used; the alias's title says it stands for the
   branch when the chip is suppressed. First renamed-place fixture in the mock.
2. **Sidebar** (#188, `App.tsx`, `App.css`, `settings.ts` rev 2, `SettingsSheet.tsx`,
   `icons.tsx`) — `nav_pinned` + `nav_hover_reveal` replace `nav_collapsed` +
   `lens`; unpinned = an absolutely positioned `.nav.overlay` (z 30, under
   `.menu-catch`) that never resizes the terminal; ⌘B reveals+focuses the filter
   / pins / unpins; Esc hides and restores focus; Enter in the filter selects the
   first visible row; sticky while a menu, drag, rename or `initAsk` holds it.
   Removed: Recent + Attention lenses, ⌘3/⌘4, the rail hide button, the `note…`
   strip and "Edit note…". Attention is a count + filter in the Places header.
3. **Settings modal + compact What's new + chord guard** (#189, `SettingsSheet.tsx`,
   `App.tsx` `WhatsNewModal`/`splitItem`/`modalOpen`, `App.css`, `zoom-check.mjs`,
   ROADMAP) — `.modal-scrim`/`.modal` (1000×680, `min(…,100%)`); notes show each
   bullet's bold lead with the paragraph behind a chevron and a Show/Hide details
   toggle; `modalOpen()` = any `.modal-scrim`/`.scrim` unbinds every app chord
   behind a dialog except ⌘, (Settings alone), ⌘K (palette open) and page zoom.
4. **Usage metrics** (#190, `app/src-tauri/src/lib.rs` `ui_events_append`/`ui_usage`/
   `ui_events_clear`, `app/src/usage.ts`, `app/scripts/usage-check.mjs`,
   `SettingsSheet.tsx` Usage category, `mock/install.ts`, DESIGN.md, CLAUDE.md) —
   `ui-events.jsonl` in the app config dir; fixed control keys (`data-track` >
   constant `data-testid` > static `title` via `titleKey`) and ten fixed surfaces;
   dwell only while visible AND focused; `valid_token` allowlist at the door;
   heatmap + bars in Settings → Usage with Reveal/Clear.
5. **Unsent prompt** (#191, `crates/worktrees-core/src/agent.rs`
   `draft_from_screen`, `lib.rs` `scan_drafts`/`list_drafts`, `App.tsx`, `App.css`,
   `mock/install.ts`) — every 15 s ONE chained `tmux` call captures every live
   claude pane; text after `❯` under a box edge is a draft (busy → queued);
   `sessions:drafts` change-gated; `✎` in the row glyphs, first words in ⌘K and
   Home resume rows, nothing in the topbar.

## Decisions

- **Two persisted booleans, not a three-value mode.** Pinned/unpinned is the
  state; hover reveal is a behaviour of unpinned. Settings shows them as one
  Pinned · Auto-hide · Hidden control. Everyone migrated to auto-hide (David's
  pick, "so it doesn't bleed into the spaces"), not a preserved preference.
- **The overlay is not a grid column** because every width change of `.space`
  is a SIGWINCH + refit for tmux; hover must never do that. Measured: `.term-host`
  width and `term_resize` count unchanged across a reveal.
- **Lenses out now, not after data.** David decided; ⌘K already ranks by
  activity (Recent's job) and Attention is a question, so it became a filter.
- **Settings is a modal, not a page** — David preferred it; the terminal stays
  mounted underneath; Status/Project sheets stay sheets (they are about the
  thing selected behind them).
- **Page zoom stays live behind dialogs.** The uniform chord guard first blocked
  ⌘+/⌘−; reverted in review: changing the size moves nothing behind a dialog,
  and the zoom slider lives inside Settings. The ⌥ markdown zoom stays guarded.
- **No draft chip in the topbar.** The selected place's terminal IS the draft;
  a session-less place cannot have one.
- **Metrics record names, never text.** Keys and surfaces are fixed vocabularies
  at both ends; a structural check fails on any dynamic-title button without a
  key. Count per control, not per place, on purpose.
- **Release notes adapt to the file.** The parser peels the bold lead; the
  changelog is not rewritten (113 of 183 bullets had a lead; the rest render
  as before).

## Dead ends / gotchas

- **An agent drove the INSTALLED app.** Its "real app" check used
  `osascript` System Events `keystroke` and `tell process "app"` while David's
  app was frontmost; a stray `x` landed in his live session and he stopped the
  agent. Root cause chain: `sandbox.sh --app` runs `tauri dev` → an UNBUNDLED
  binary with a NULL bundle id and the executable name `app`, identical to
  `/Applications/worktrees.app`'s; `tell application "worktrees"` resolves to
  the installed one, `tell application id "net.casadelvalle.worktrees.sbx"`
  resolves to nothing (my first CLAUDE.md rule said to use it — wrong, fixed),
  `tell process "app"` is a coin toss. Rule now in CLAUDE.md: pid only, no
  scripted input into the sandbox at all; read its files and app.log.
- **A stopped agent cannot be resumed.** After David stopped it, `SendMessage`
  refused; a new agent needs the user's explicit go. Its partial work was
  intact on the branch and the replacement built on it.
- **claude writes `❯` + U+00A0.** `strip_prefix(' ')` matched every hand-written
  fixture and no real pane; two sandbox samples were silently empty until a
  hex dump. `strip_prefix(char::is_whitespace)`.
- **`❯` is also a selector-menu cursor and an agent-footer glyph.** "❯ No, exit /
  Yes, I trust this folder" has the exact shape of a wrapped draft, and a busy
  pane shows a second `❯` line BELOW the input box. Rule: the prompt line must
  sit directly under a box edge (`───`). Capture `-S -20` so the box top is in
  frame (`-S -N` is "start N rows into scrollback", not "N lines").
- **`display-message -p` eats `%`** in its format, so a chain marker like
  `@@%0@@` prints `@@@@`. Marker goes FIRST in each chained pair, so a chain
  that dies part-way still attributes its surviving segments.
- **Synthetic `pointerenter` does nothing in React** — it implements
  `onPointerEnter/Leave` on `pointerover/out`. Dispatch those in the harness.
- **`titleKey` alone leaks a spaced title** ("Quokka Fanclub HQ — messaging" →
  `quokka-fanclub-hq`); only `data-track` on the row prevented it. Hence the
  structural guard in `usage-check.mjs`, which found seven more dynamic-title
  buttons on its first run (all harmless today, all keyed).
- **xterm prevents default on most keys**, so `trackChord` was recording Ctrl+C
  and the tmux prefix as chords — a typing meter. Ctrl-only chords inside
  `.term-host` are ignored.
- **The zoom-check script encoded the old policy** and went red on the new
  guard; when a slice-the-source check fails after a deliberate change, update
  its assertions and show them red without the change.
- **The dev/sandbox build logs into the installed app's `app.log`** —
  `APP_IDENT` is a const, so only the config dir moves. That is why the draft
  sampler logs counts only unless `WORKTREES_TRACE_DRAFTS=<path substring>`.
- **The brief said "every changelog bullet has a bold lead"** — 113 of 183 did.
  The parser handles both; the older half renders as before.
- **Enter in the filter selects the first VISIBLE row** — a match inside a
  collapsed tier group selects nothing. Noted, not fixed (ROADMAP).

## Verification

- Every PR: mock harness readings with DOM values (widths, `term_resize`
  counts, `defaultPrevented`, `elementFromPoint` hit tests), red-then-green for
  each new check/test, `tsc`, `cargo check -p app`, `cargo test -p app --lib`,
  every `app/scripts/*-check.mjs`; CI green on all five; #190/#191 also
  `make test` (335 ok), `make lint`, core (274→285), cli (7) with the release
  binary rebuilt first.
- Real app (sandbox by pid, own `sbx-` tmux sessions): #190 wrote a
  `ui-events.jsonl` containing only fixed keys, with a clicked place's slug
  absent from the file and the installed app's config dir untouched; #191 saw a
  typed draft in 9 s and its clearing in 9 s, one tmux spawn per ~15 s across 9
  panes / 23 sessions.
- Not verified: the new UI in David's own installed build (not installed).

## Follow-ups

- Install (`make install-app`, quit + reopen) and live with auto-hide, the
  modal and Usage for two weeks; then read Settings → Usage and revisit chrome.
- ROADMAP items filed from this session: filter Enter vs collapsed groups;
  sandbox log path; usage heatmap blind spots and state-fragmenting keys; tree
  keyboard navigation.
