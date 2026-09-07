# Session: Escape dismisses every dialog, even with the terminal focused

- **Date:** 2026-09-07
- **Worktree:** `ui-changes` (idle base `ui-changes-next`)
- **Branch:** `ui-changes-esc-modals` off a freshly fetched `origin/main`, deleted after merge
- **PR:** [#196](https://github.com/penard-monkey/worktrees/pull/196) → `e5b0867`
- **Release:** none — the entry sits in `[Unreleased]` on top of v0.21.0, under the usage-meter entries from #194
- **Planning files:** none — a single-fix session, no task plan was opened

## Context

David: "when a modal is open, I want the escape key to dismiss it. not
currently doing that." Then, while the diagnosis was underway: "Settings is
one example where I pressed escape and it interacted with the tmux session."
That second sentence is the whole bug — the key was not being ignored, it was
being delivered to the wrong place.

## What shipped

- **`app/src/useEscape.ts`** — one capture-phase `keydown` listener on
  `window`, attached only while at least one surface is up, and a stack of
  entries ordered by activation. The top entry takes Escape; the event is
  `preventDefault`ed and `stopPropagation`ed so nothing underneath (the
  terminal, a lower dialog, the app's chord handler) sees it. `fn` is read
  through a ref so a fresh closure per render never re-registers (re-registering
  would move a LOWER surface to the top). `escapeDepth()` is a test seam.
- **Every modal surface calls it** (`app/src/App.tsx`, `SettingsSheet.tsx`,
  `ProjectSheet.tsx`, `StatusSheet.tsx`, `CtxMenu.tsx`): Settings, Project
  sheet, Status sheet, What's new, ⌘K, the context menu, New worktree, Remove,
  New project, Import picker, Sync, the branch combobox's popover (while its
  list is VISIBLE) and the header's branch-chip popover from #194.
- **Retired:** each component's own `window.addEventListener("keydown")`,
  Settings' `.modal-scrim.stacked` DOM test (the stack orders by activation, so
  What's new over Settings needs no lookup), the combobox's Escape branch with
  its `stopPropagation`, and ⌘K's input-level Escape branch.
- **`app/scripts/ctxmenu-check.mjs`** strips the new import and stubs
  `useEscape` in its env, the same way it already stubs React's hooks.
- `CHANGELOG.md` `[Unreleased]` → Fixed; a CLAUDE.md rule under the Tauri
  section.

## Decisions

- **Capture phase at `window`, not focus management.** Moving focus into each
  dialog on open would also have fixed it, but it fixes it per dialog, and the
  next dialog written without a focus target regresses silently. One listener
  that runs before the target sees the event fixes the class and keeps the ESC
  byte out of the pty as a side effect.
- **A stack, ordered by activation, not a DOM query.** The `.modal-scrim.stacked`
  test only knew about one pairing (What's new over Settings) and the
  combobox handled its own pairing with `stopPropagation` in the input
  handler. Both were correct and neither generalised; the third pairing (the
  branch list inside the chip popover) arrived in #194 with a comment relying
  on the combobox's `stopPropagation`, which this PR was removing.
- **A busy dialog keeps its entry and makes `fn` a no-op.** Passing
  `active=false` while `busy` would hand the key to the surface beneath — the
  Sync modal mid-rsync over the Project sheet would close the SHEET. Swallowing
  is the right wrong.
- **The usage-meter pinned panel keeps its bubble listener.** Its comment
  documents that the panel is inert and Escape must stay with the terminal
  (vim, a prompt) while it is pinned. That is the opposite contract from a
  modal's, and it is not on the stack on purpose.
- **Not converted:** the inline rename editors (`TitleEditor`,
  `TermTabRename`), the Find bar, reading mode and the revealed sidebar. None
  is modal; each already owns Escape where focus actually is.

## Dead ends / gotchas

- **The Chrome extension's synthetic key press never reached the page.**
  `computer key Escape` reported success and the dialog stayed — before AND
  after the fix. A `KeyboardEvent` dispatched at `document.activeElement`
  (the xterm textarea) goes through capture → target → bubble exactly like a
  real key, xterm's `stopPropagation` included, and is what the verification
  ran on. Read `escapeDepth()` alongside the DOM, or a "closed" that was never
  opened reads as a pass.
- **The first PR was silently unmergeable.** Main moved (#194/#195) between
  the fetch and the PR; GitHub computes no merge commit for a conflicting PR
  and therefore runs NO checks — `gh pr checks` said "no checks reported",
  which reads like a paths-ignore skip. `gh pr view --json mergeable` is the
  tell (`CONFLICTING`). Rebased; the only conflict was two `[Unreleased]`
  sections.
- **Child effects run before parent effects.** A parent and child that both
  activate in ONE commit push child-first, i.e. the visually-upper surface
  lands LOWER on the stack. Every pairing here activates across separate
  commits (the list opens after the popover's input focuses), so it is not
  hit — but it is the one shape that would make the stack lie. Checked in the
  harness rather than assumed.
- **`gh pr merge` from a side worktree fails AFTER merging** (known; CLAUDE.md).
  `gh pr view 196 --json state,mergeCommit` → MERGED.
- **This worktree was 23 commits behind at session start** with a clean tree
  and no unpushed commits. The rev-list check caught it before any edit.

## Verification

- Mock harness (`VITE_MOCK=1 vite --port 1431 --force`; HMR is dead inside
  `.worktrees/`, so a restart per edit), Escape dispatched at the focused
  xterm textarea:
  - Settings open, terminal focused → closes; `escapeDepth` 1→0; a spy
    listener on the textarea never fired.
  - What's new over Settings → first press closes only the notes, second
    closes Settings.
  - New worktree with the branch list showing → list, then dialog.
  - Branch chip popover: list showing → list, then popover; no list → one
    press closes the popover.
- Reproduced the bug the same way on the pre-fix build first.
- `tsc --noEmit`, `cargo check -p app`, all nine `app/scripts/*-check.mjs`;
  the full CI matrix on the rebased PR (nine checks green).

## Follow-ups

- A drift guard for the stack: every `.scrim` / `.modal-scrim` /
  `.menu-catch` surface should call `useEscape` — see ROADMAP.
- Three unmerged remote branches under this tree's prefix predate this
  session and were left alone: `origin/ui-changes-branch-combo` (1 commit,
  2026-08-17), `origin/ui-changes-close-out` (1 commit, 2026-08-18),
  `origin/ui-changes-new-worktree-modal` (3 commits, 2026-08-18). Decide
  whether any of that work is still wanted, then delete them.
