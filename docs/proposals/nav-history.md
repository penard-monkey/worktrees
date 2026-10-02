---
title: "Proposal — back / forward navigation history"
---

# Proposal — back / forward through what the two panes showed

**Status:** design, 2026-10-02, approved the same day (see Decisions).

**The ask.** Browser-style history over what the app shows: the place (or
Home) on the left, and whatever the right side is on — dock tab, the file in
the viewer and its line, the shell tab, the agent. ← / → buttons and chords
walk it.

## 1. What a location is

Every piece of state that decides what the panes show, mapped in `App.tsx`:

| State | Where | In the location? |
|---|---|---|
| `sel` `{repo, slug}` / `null` = Home | `App.tsx:3251` | **yes** — the spine |
| `eff.dock_tab` (files/terminal/docs/plan/automations) | `place_panels` via `panelsFor`, `:3430` | **yes** |
| `dockFile` + `dockAt.line/col` | `:3453`, `:3457`; all opens go through `openDockFile` `:4389` | **yes** (only when tab = files) |
| active dock shell | `TerminalTabs`' local `active` (`:1163`), reported up by `onFront` (restore and fallbacks included; `term_tab_active` is only written by picks) | **yes** (only when tab = terminal) |
| main-pane agent | `activeProvider` → `planProvider` (`:4610`); one cell is shown | **no** — dropped while building: a place runs ONE live agent (a second is closed by the reconcile at `:4644`), so a switch replaces the session and there is nothing to go back to |
| `eff.dock_open` (⌘J) | `place_panels` | **no** — layout, like ⌘B. Applying an entry opens the dock only if the entry's dock content differs from what is there |
| reading mode (⌘⇧E), find bar, md zoom, diff mode, layout, scroll | various | **no** — view modes, not places |
| Docs / Plan / Automations inner selection | inside their panes | **no** (v1) — anything they open lands in Files, which IS recorded |
| Settings, ⌘K palette, Project / Status sheets, dialogs, offers | modals (`.modal-scrim`/`.scrim`) | **no** — see §5 |

```ts
type Loc = {
  place: { repo: string; slug: string } | null;           // null = Home
  dock?: { tab: DockTab; file?: { path: string; line?: number; col?: number }; shell?: number };
};
```

**How entries are captured — observe, don't instrument.** One effect derives
the current `Loc` from the state above and hands it to the history whenever
its key changes. Default is **push**, so a navigation added next year is
recorded without anyone remembering to. The known *system* paths instead
**amend** the current entry (a ref they set): the launch restore (`:6400`),
the per-place `files_open` restore (`:3487`), `TerminalTabs`' restore and
dead-tab fallback, the removed-place `setSel(null)` (`:4990`, `:5344`), and
the session-down provider fallback (`:4605`). A missed system path costs an
extra entry; a missed user path under instrumentation would be a silent hole —
so default-push is the safer way to be wrong. History applying an entry
suppresses recording for that change.

**Noise (never an entry):** polls, refreshes re-selecting the same place,
hover, scroll, hydration. History starts at the first settled location after
launch (the restore becomes entry 0, not a push).

**Coalescing.** (a) an identical consecutive `Loc` is dropped; (b) a change
within **600 ms** of the last push *replaces* the top entry — this folds
"click place → its remembered file arrives 50 ms later" into one entry, and
arrowing quickly through nav rows into the row you stopped on. Push after
Back truncates forward, as in a browser.

**Agent `show_doc` and ⌘-click to a line are entries.** `show_doc` is the
strongest case for the feature: an agent replaced your view, Back takes you to
what you had.

## 2. Chords — recommendation

**⌘[ / ⌘] always; ⌘← / ⌘→ everywhere except a text field.** Plus mouse
buttons 4/5 (back/forward).

- **Evidence that the terminal loses nothing.** xterm 5.5 `Keyboard.ts`:
  `case 37: // left-arrow  if (ev.metaKey) { break; }` (same for 39) — ⌘← / ⌘→
  produce **no bytes**; `_keyDown` returns without cancelling, so the event
  reaches the window listener (the existing "meta chords fire past the
  term-host" rule). Today ⌘← in zsh or Claude's composer does nothing at all;
  line start there is ⌃A. So the terminal can have Back.
- **Text fields keep line start.** `input`, `textarea` (the markdown editor in
  `FilesPane`, guidance/profile/automation editors), `select`,
  `[contenteditable]` — but NOT `.xterm-helper-textarea`, xterm's focus sink,
  which is a textarea and must be special-cased or the terminal loses the chord.
- **⌘[ / ⌘]** is Safari / Finder / Xcode's Back/Forward, works in fields too
  (WebKit binds nothing to it), and matches on `e.key` with an `e.code`
  (`BracketLeft/Right`) fallback, per the ⌥/layout rule.
- No collision with `zoomDir` (⌘= ⌘- ⌘0), ⌘1–9 nav chords, ⌘K/⌘J/⌘T/⌘B.
  Handled in the existing window `onKey`, BELOW the modal guard: unbound (not
  swallowed) while a dialog is up. `e.repeat` allowed (hold to walk).
- Rejected: ⌃- / ⌃⇧- (VS Code) — ⌃ belongs to the shells; trackpad swipe —
  WKWebView's gesture is page history and would reload the app.

## 3. Things that are gone

- **Place removed** → its entries are **pruned** at removal (same hook as
  `dropPanels`), and `back`/`forward` also skip any entry whose place is not in
  `ws` (removed elsewhere). Place = skip.
- **Below the place, degrade, don't skip:** file deleted → land on the place +
  Files tab, empty viewer, one-line notice "`x.rs` no longer exists" (checked
  with `file_readable`, as the restore does); shell closed → Terminal tab on
  its nearest shell; agent not live → whatever the place shows now. You still
  get where you were going.
- **Cap 100 entries**, in memory, one per window, **not persisted**. A relaunch
  already returns you via `restore_last` + `files_open`; persisting would mean
  another key in the whole-blob `ui-state.json` for little gain.
- **Applying never seeds.** Restoring a dock tab goes through `updatePanels`
  carrying only fields that differ from `eff` (the `place_panels` SEED rule).
  Like `pendingDoc` (`:4433`), the place is selected first and the dock part is
  parked until `sel` lands, since `updatePanels` writes for `selRef.current`.
  The `files_open` restore must stand down while a parked apply is pending, or
  it reopens the remembered file over an entry that had none.

## 4. UI

- **‹ › at the leading edge of the topbar**, before the place name; on Home
  (which has no topbar) the same pair at the top-left of the Home view. Icon
  buttons; disabled when there is nothing that way.
- **Tooltip names the destination**: "Back to nav-history · Files · App.tsx:6087
  (⌘[)".
- **Right-click either button** → a `CtxMenu` of up to 12 entries that way,
  current one marked; click jumps. (Long-press: no — WebKit/trackpad have no
  good long-press, right-click is the Mac idiom in Safari too.)
- WebKit button rule: no flex container on the `<button>` itself; hit-test with
  `elementFromPoint`; tokens for colour, dim via `--txt-mute`.

## 5. Interplay

- **⌘K palette, nav click/double-click, Resume rows, quick-switch, drop onto a
  place** → whatever they select is an entry (observed, nothing special).
- **Settings / Project / Status sheets, Switch agent, offers, dialogs** are not
  entries and the chords are unbound while they are up (existing modal guard).
  Their *result* — Switch agent landing a different agent — is an entry.
- **Drag and drop** (reorder, cross-project drop onto a session) is not
  navigation and changes nothing recorded.
- **Reading mode** survives Back only if the entry is the same file; otherwise
  the existing rule (`setReading(false)` on file/place change) closes it.

## Decisions (David, 2026-10-02)

1. The Project sheet is **not** an entry in v1 — a project row click only folds it
   (`toggleProject`), and the sheet is a modal.
2. `dock_open` is **not** part of a location — ⌘J is layout.
3. Coalesce window **600 ms**.
4. Chords as proposed: ⌘[ / ⌘] always; ⌘← / ⌘→ except in text fields, with
   `.xterm-helper-textarea` special-cased as NOT a text field; mouse buttons
   4/5; all unbound while a modal is up.

## Build slices (after go)

1. `app/src/navHistory.ts` (pure: push / amend / back / forward / prune /
   label) + `app/scripts/navhistory-check.mjs` evaluating the real module,
   red-first.
2. Observer + Home/place capture + buttons + chords + mouse buttons.
3. Dock tab + file/line capture and apply (parked apply, restore stand-down).
4. Shells (`TerminalTabs` gets a `goto` token and reports its front tab).
5. Gone targets (prune, notice) + right-click list.
6. Harness in Chromium and headless WebKit, CHANGELOG, gates, PR.
