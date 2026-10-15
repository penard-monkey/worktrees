# Reference left, agent right — the synthesis

**Status:** design + mocks, 2026-10-14, lane `dock-side-mix`. Third pass over two independent designs:
Fable (`../dock-side-fable/`) and Astra (`../dock-side-astra/`). No app code changed. Source HEAD `7f780f1`.
**Mocks:** `mocks/index.html` (offline, real `tokens.css` inlined, six themes, live drag + keyboard Move menu in frame 01;
`?theme=<id>&only=live,compare,settings,drag,term,narrow` to isolate). PNGs beside it, rendered by headless Chrome.
The static states were checked by screenshot; the live drag/menu were written but not exercised by an automated driver.

## 0. The answer in eight lines

1. **One new global field ships first: `dock_side: "left" | "right"`**, beside the existing `places_side`. Code name
   from Fable (every dock key is `dock_*`); user-facing label from Astra ("Reference panels").
2. **Same edge ⇒ the rails merge** into one tool strip (Fable). That *is* Ian's JetBrains strip, it frees 44px, and two
   adjacent 44px rails under the default auto-hide nav read as a bug.
3. **Phase 1 = `dock_side` + merged rail + a four-tile Layout picker in Settings.** ~250 lines, no xterm re-parents,
   one SETTINGS_REV step. Concrete file list in §5.
4. **Phase 2 = Terminal below the reference panel** (`dock_lower`), the half of Ian's screenshot phase 1 does not give
   him. Same shells as the Terminal tab — not a second instance (§4).
5. **Phase 3 = drag to arrange** over exactly those fields: header grips, snap zones painted on an overlay, one commit
   on drop, Undo banner, and a "Move…" menu on every grip as the keyboard path (Astra).
6. **The agent's box is sacred.** Nothing in any phase changes its height; widths change only on a deliberate act, and
   then by one coalesced resize. Fable's `under-agent` terminal slot is cut for that reason.
7. **Global, not per place, not per project** — both designers agreed on sides; I extend it to the lower slot, where
   Astra wanted per-place twins (§3.1).
8. Four things neither caught (§6): the migration must not test for a missing key, the merged-rail flip is NOT free,
   the float-stack lands on the lower shell's prompt, and new places still open with no reference panel.

## 1. Where each idea came from

| Idea | From | Kept? | Why |
|---|---|---|---|
| Independent side for the dock, beside `places_side` | both | **yes** | Both found the same root cause: `places_side` mirrors the whole shell (`settings.ts:175-190`), so `nav \| dock \| agent` is unreachable. |
| Field name `dock_side` | Fable | **yes** | `dock_open`/`dock_tab`/`dock_width`, `.dock`, `DOCK_RAIL`, `onDockResize` — `reference_side` would be the one key in the family spelled differently. |
| Label "Reference panels" | Astra | **yes** | Users never see "dock". Settings and menus say "Reference panels"; code says `dock`. |
| Migration: missing ⇒ opposite of `places_side`, rev 3→4 | both | **yes, with a fix** | Same idea twice; both phrased it as "if the field is missing", which is wrong here (§6.1). |
| Nav at an outer edge only, never between reference and agent | both | **yes** | `dock \| nav \| agent` pulls the dock out of `.space` and loses the header that crowns reference + agent (`App.tsx:7501`). |
| Merged rail when nav + reference share an edge | Fable | **yes** | §3.2. |
| Separate reference rail kept beside nav | Astra | no | Two 44px rails side by side with the nav hidden (the default since rev 2) — reads as a duplicated rail. |
| Four presets as clickable schematics | Fable | **yes** | They are just the 2×2 of two booleans, drawn. One control shows the RESULT; two segmented controls make you compose it in your head. |
| "Applies to all places" stated inline | Astra | **yes** | Scope must be visible where the choice is made. |
| `term_slot: tab \| under-dock \| under-agent` | Fable | **shape no, scope partly** | `under-agent` makes the agent shorter — the one pane Ian wants full height — and every toggle of it SIGWINCHes the agent. Cut. |
| "Any section may take the lower slot" (`reference_lower: DockTab \| null`) | Astra | **shape yes, scope later** | Typed `DockTab \| null` so it can widen; phase 2 validates `"terminal"` only. Terminal is the only section with a real always-open habit and the only one whose two-at-once value is obvious (read the README while running the command it describes). Widening is a lifecycle job (Files/Docs/Plan each own state and polls), not a type change — Astra flagged this herself (her open Q3). |
| One DOM position for shells, three grid cells | Fable | no | Overbuilt: the dock body is ALREADY a tab conditional (`App.tsx:8253`), so every tab flip today unmounts the shell and re-attach replays its ring. Moving the terminal between tab and lower slot costs exactly what a tab click costs now. |
| Drag handles in section headers, never on content | both | **yes** | Nav header, dock title, agent label, lower strip. Never a place row, never the xterm surface. |
| Overlay zones, no live reflow, one commit on drop | both | **yes** | Zero `term_resize` during a drag; ≤ 1 per affected pane after. |
| Undo banner after a drop | Fable | **yes** | Reuses the nav-drag `undo` banner in `.float-stack`; no confirmation modal (Astra agrees). |
| Keyboard "Move…" menu on every grip, aria-live result | Astra | **yes** | Presets alone are a keyboard path to *a* layout, not to "move this thing". Both are kept. |
| Esc via `useEscape`, restore focus to the grip | both / Astra | **yes** | xterm swallows bubbling keys (AGENTS.md); arrange is a stack entry. |
| Narrow window: tell the user the reference yielded, offer "Auto-hide navigation" | Astra | **yes, phase 2** | Today the only signal is a rail tooltip. With reference on the left, pressing ⌘J and seeing nothing happen is the failure mode. |
| Splitter drag commits on release, guide line while dragging | Astra | **yes, phase 2** | The lower splitter's resize must not tick the shell per pause; the 80ms coalesce alone does not guarantee one. |
| Overlay-nav drag to the agent must be real-app tested | both | **yes** | The mock answers instantly; hover-hide timing is real-app only. |
| `fitLayout` takes the rail count | Fable | **yes** | Merged = `RAIL_W`, else `RAILS_W`. |
| Rail overflow on a short window (Fable Q5) | Fable | **resolved** | §3.2: fits at the 560px `minHeight`, measured in the mock; overflow rule stated anyway. |
| Ian's shell spans under the tree too | Astra (noticed) | **resolved** | §4.1: with the nav unpinned (the default) the difference vanishes. |
| `layout-check.mjs` mirror check | Fable | **yes, narrower** | Phase 1 has no preset list to drift from the shell builder if the tiles are generated from the same 2×2. Check what can actually drift (§5.3). |
| Per-project override | Fable (open) | no | §3.1. |

## 2. Today, in the code (only what the decisions rest on)

- `.app` is a one-row grid built from ONE `shell` array that is both the column list and each child's CSS `order`
  (`App.tsx:7524-7542`). `places_side === "right"` swaps the array. No DOM node moves.
- The dock is not a grid column: it is a flex sibling of `main` in `.space-body` under the place header; its side is
  `order: mirrored ? -1 : 0` (`App.tsx:8136`). Border and resizer side come from `html[data-places-side]` (`App.css:75,94`);
  the resize SIGN from `mirrored` (`App.tsx:7156`).
- The dock rail is `<nav className="rail rail-right">` (`App.tsx:8426`) — five `DOCK_RAIL` buttons and, at its foot, the
  offers button. `usage-check.mjs:318` and `offers-check.mjs:340` find it by that exact string.
- The dock body renders ONE tab through a conditional chain (`App.tsx:8253+`). The Terminal tab's `ShellPane`s unmount
  on every tab flip and re-attach by replaying the backend's 256K ring.
- `fitLayout` subtracts `RAILS_W = 88` always (`settings.ts:565,612,619`).
- A new place starts with the dock CLOSED; `dock_open` is deliberately not a seed (`settings.ts:116-130`).
- `.float-stack` (undo, errors, notices) is fixed at `left: calc(var(--rail-w) + var(--s3)); bottom` (`App.css:1231`).
- Window minimum 900×560 (`tauri.conf.json`).
- `loadSettings` merges the stored blob over `DEFAULTS` BEFORE `migrate` runs.

## 3. Decisions

### 3.1 Fields and scope

```ts
// phase 1
dock_side: "left" | "right";          // DEFAULTS "right"; rev 4 sets it to the opposite of places_side
// phase 2
dock_lower: DockTab | null;           // DEFAULTS null; validated to "terminal" | null until widened
dock_lower_frac: number;              // DEFAULTS 0.4, clamped 0.25..0.6 (share of the dock column's height)
```

All three are **flat globals and none of them enters `place_panels`.** Sides: both designers agreed — a per-place side
moves the agent on every nav click. Lower slot: Astra proposed per-place twins. I'm rejecting that for three reasons.
(1) The habit is the person's ("it's actually always open because i use it constantly"), not the repo's. (2) A
`place_panels` field whose global twin is a seed has to be optional, and `updatePanels` writes the whole record — the
trap AGENTS.md documents for `files_md_zoom`. (3) A per-place lower slot changes which shell is mounted where on every
place switch, which is a re-attach + ring replay per nav click. **What stays per place is what already is:**
`dock_open`, `dock_tab`, `dock_width`. A fraction rather than px for the split, because the column's height follows the
window and a px split would need re-clamping on every resize (Astra's percentage; Fable's px).

Invariant (phase 2): `dock_tab !== dock_lower`. If the active tab is assigned below, the upper group falls back to the
last other tab, then Files. The upper group is never empty. A `dock_lower` naming PRs on a non-GitHub project
normalizes to `null` at render, not on disk.

### 3.2 Merged rail (Fable), with Astra's grouping

When `places_side === dock_side`, `.rail-right` is not rendered and its contents render inside `.rail`:

```
 Places        ← the Places toggle (⌘B), always first
 ─────         ← 1px --line hairline, 8px inset
 Files  Terminal  Docs  Plan  PRs   ← the dock group, same buttons, same data-track keys
 (spacer)
 ✦ offers      ← moves with the dock group: "foot of the dock rail"
 usage tile · add project · feedback · gear
```

- Grid: three tracks (`rail | nav | space`, or mirrored) instead of four. `fitLayout` gets the rail width as input.
- Height at the 560px window minimum, measured in the mock (app's 32px icons, `--s1` gaps, `--s2` padding, an
  APPROXIMATED usage tile): Places + hairline + 5 dock icons + offers + tile + 3 actions = 428px of content in a 511px
  rail (the mock spends 28px on a fake titlebar the real rail does not have). ~80px of slack — fits, but re-measure
  the real tile in the sandbox before calling it settled. Rule if a future icon breaks it: the dock group scrolls
  (`overflow-y: auto` on `div.rail-group`), the Places toggle and the footer never do.
- The active bar stays on the window-edge side of the strip (today's `::before` rules, keyed on the rail's edge).
- Merged with Places on the RIGHT (the "Agent left" tile) is the mirror image; same component, same rules.
- Home: the dock group shows disabled with today's "select a place first" title. Nothing new.

### 3.3 Settings surface

Settings → Navigation's "Sides" becomes **Layout**:

- **Four tiles**, radio semantics, arrow keys move between them: *Classic* (`nav · agent · reference`), *Reference
  left* (`nav · reference · agent` — Ian), *Mirrored* (`reference · agent · nav`), *Agent left*
  (`agent · reference · nav`). Each tile writes `places_side` + `dock_side`. Generated from the 2×2, so there is no
  separate preset list to drift.
- Hint: "Applies to all places. Widths, ⌘B and ⌘J are unchanged. Tab still walks the panels in their classic order."
- Phase 2 adds **Terminal: In the panel's tabs / Below the reference panel**. Phase 3 adds **Arrange by dragging…**
  and **Reset layout** (writes the defaults of every layout field; Undo banner, no modal).

### 3.4 What phase 2 and 3 actually ship; the keyboard path

Order: **sides → terminal-below → drag.** Drag over two booleans is a toy; drag over sides + lower slot is the real
feature David asked for, and phase 2 is what makes Ian's screen match his screenshot. If David wants DnD sooner,
phase 3a (drag for sides only) is separable and touches no phase-2 code — open question 1.

Phase 3 zones (all overlay rectangles; the zone equal to the current position is drawn dimmed "already here"):

| Grip on | Zones | Writes |
|---|---|---|
| Places header | window left edge · window right edge | `places_side` |
| Reference title | left of the agent · right of the agent | `dock_side` |
| Agent label | left half · right half of the work area | `dock_side` (inverse) |
| Lower terminal strip, or Terminal tab in arrange mode | "in the panel's tabs" · "below the reference panel" | `dock_lower` |

Keyboard: every grip is a `<button>` ("Move Reference panels…"). Enter opens a menu of the same destinations (current
one disabled, "already here"), arrows + Enter commit, Esc cancels via `useEscape`, an `aria-live` region announces
"Reference panels moved to the left of the agent", focus returns to the grip. Settings tiles remain the second path.

### 3.5 Phase 1 — see §5.

### 3.6 Terminal under reference — see §4.

## 4. The terminal under the reference panel (phase 2)

### 4.1 How close to Ian's shape

His shell spans under the project tree AND the README. Ours spans under the reference column only. With the nav
unpinned — the default, and what an "always look at the agent" user will run — the reference column starts at the
rail, so the screen is `strip | README / shell | agent`: his screenshot minus a permanently visible tree. If he pins
the nav, the shell does not extend under it. Extending it would make the nav a participant in a vertical split, and
the nav is global, overlays, and hides on ⌘B — a shell under a panel that slides away has nowhere to live. Not worth
it; if he wants a tree beside the README, Files is a tab in the same column.

### 4.2 Same shells, not a new instance

The lower slot renders the existing `TermTabs` (strip + `ShellPane`) with the same `repo`/`slug` — the same backend
shell sessions, the same ring, the same tab names. A second instance is wrong for a hard reason: a pty has ONE window
size, and two xterms of different sizes attached to one shell is a SIGWINCH tug-of-war between them. While
`dock_lower === "terminal"`:

- Terminal leaves the upper tab set; its rail icon is `active` whenever the lower slot is on screen and a click
  FOCUSES the front shell (opening the column first if ⌘J had closed it). ⌘T adds a shell in the lower strip.
- ⌘J hides the column, both halves (Fable's Q4 answer; Astra agrees). The split intent is kept.
- Moving Terminal between tab and lower slot = unmount + remount = one ring replay, i.e. what a tab click costs today.
  `termreplay-check` already covers the mute.
- `navHistory`: `stripUp` (`App.tsx:6919`) becomes "the dock is shown AND (tab is terminal OR lower is terminal)", so
  a location records the front shell whenever a strip is visible. `navhistory-check`/`navwire-check` gain the case.
- The splitter: 4px hit area on the lower strip's top edge, a guide line while dragging, `dock_lower_frac` written on
  release only. The SHELL gets one resize per release; the agent gets none — its box is untouched by anything in
  the reference column.
- Floors: upper ≥ 160px, lower ≥ 140px (strip + five rows at 13px). At the 560px window both fit; at high app zoom
  the lower slot temporarily renders as a tab again (Astra's compaction) without touching the setting.
- `termfit-check` grows: the new lower host is a flex column item, so it needs `min-height: 0` (the vertical twin of
  the `min-width: 0` ratchet) and `.term-host`'s rule stays byte-identical.

## 5. Phase 1 — implementation plan (this is what gets built next)

**Goal:** Ian can pick *Reference left* and get `rail(merged) | [nav] | reference | agent`, with the agent at the
window's right edge. Nothing else changes for anyone who doesn't touch the setting.

### 5.1 Files

| File | Change |
|---|---|
| `app/src/settings.ts` | `dock_side` in `Settings` + `DEFAULTS` ("right"); `SETTINGS_REV = 4`; `migrate`: `if (from < 4) s.dock_side = s.places_side === "right" ? "left" : "right"` (validate `places_side` first; see §6.1); sanitise in `loadSettings` like `isDockTab`; `fitLayout(s, dockEligible, w, railsW = RAILS_W)` and the three `RAILS_W` uses in `navCeiling`/`dockCeiling` take it too; `applySettings` writes `root.dataset.dockSide`; new pure `railsMerged(s) = s.places_side === s.dock_side` exported for App and the check. |
| `app/src/App.tsx` | `dockLeft`/`merged` from settings; `shell` built from `(places_side, merged)` — 3 or 4 tracks; dock `order: dockLeft ? -1 : 0`; `onDockResize` sign from `dockLeft` (nav sign stays on `places_side`); extract module-scope `DockRailGroup` (props: tabs, `dockShown`, `eff.dock_tab`, `why`, `prsOn`, prs counts, `onPick`) and `OffersRailButton` (props: `count`, `onOpen`); render both inside `.rail` after the Places button + `.rail-sep` when `merged`, inside `<nav className="rail rail-right">` otherwise; `.rail-right` is not rendered when merged. Usage tile and gear bubble `side` keep following `places_side` (they live in the Places rail either way) — write that down where `side=` is computed. |
| `app/src/App.css` | Move the dock rules at `:75` and `:94` from `html[data-places-side="right"]` to `html[data-dock-side="left"]` (border-right, resizer on the right edge). `.rail-sep { height: 1px; margin: var(--s1) var(--s2); background: var(--line) }`. `.rail .rail-group` (flex column, `min-height: 0`, `overflow-y: auto`). No new tokens. |
| `app/src/SettingsSheet.tsx` | "Sides" → "Layout": four schematic tiles (`role="radiogroup"`, roving tabindex), generated from `[places_side] × [dock_side]`; hint text from §3.3. |
| `app/src/mock/install.ts` | Default `dock_side`. |
| `app/scripts/usage-check.mjs` | Anchor on `function DockRailGroup(` instead of `<nav className="rail rail-right"`; same `data-track` ternary assertion. |
| `app/scripts/offers-check.mjs` | Anchor 4b on `function OffersRailButton(` + assert both hosts render it under complementary `merged` conditions and that its gate is still `offers.length > 0` at the call sites. |
| `app/scripts/layout-check.mjs` (new) | (a) `migrate`'s rev-4 step derives `dock_side` from `places_side` and does not test key presence; (b) the `shell` builder has a branch per merged × side and its keys match the children's `ord.*` uses; (c) `DockRailGroup` and `OffersRailButton` are each rendered in exactly two places under `merged` / `!merged`; (d) the four tiles' (places, dock) pairs cover the 2×2 exactly once. Wired into CI next to the other `*-check.mjs`. Shown red against a deliberately broken copy first. |
| `app/scripts/plan-usage-browser.mjs`, `update-bubble-browser.mjs` | Side matrix gains `dock_side` (four shells). |
| `app/scripts/termresize-check.mjs` | One case: a side flip that keeps `.main`'s width sends **0** resizes; a merge flip (±44px) sends **exactly 1** after settle. |
| `CHANGELOG.md` | `### Added` under `[Unreleased]`: "Reference panels can sit on the left, beside Places, with the agent on the right (Settings → Layout)." Re-check the section after any rebase. |

Not touched: `TerminalPane.tsx`, `.term-host`, `useEscape.ts`, `navHistory.ts`, any Rust. Estimated 250–300 lines incl.
the new check.

### 5.2 Tests and checks to run

All gates from AGENTS.md (app part: `tsc --noEmit`, `cargo check -p app`, every `app/scripts/*-check.mjs` from the repo
root). Then by hand in `app/scripts/sandbox.sh --app`: the four tiles × pinned/auto-hide nav × dock open/closed;
`getBoundingClientRect` the `.term-host` right edge against the window (Reference left) and against `.dock`'s left
(Classic); count `term_resize` invokes per flip in app.log; window at 900×560 with each tile (merged rail fully
hit-testable — `elementFromPoint` on the gear); drag a place row from the hover-revealed nav across the reference
panel into the agent; drag a file from Files into the agent; a mirrored v0.41 `ui-state.json` loads as *Mirrored*,
a default one as *Classic*.

### 5.3 What phase 1 deliberately leaves alone

Tab order (DOM order: agent, then reference — documented in the hint, as the mirror already does); per-place layout;
the narrow-window notice (phase 2); drag (phase 3).

## 6. What both missed

1. **The migration cannot ask "is `dock_side` missing?"** `loadSettings` spreads the stored blob over `DEFAULTS` before
   `migrate` runs, so the key is always present by then and a presence test silently gives every mirrored user
   `dock_side: "right"` — i.e. moves their dock to the same side as their nav and merges their rails on upgrade. The
   rev number is the test: `from < 4` means the key never existed. Pinned by `layout-check` (a).
2. **The merge is not free in pixels.** Fable's "a side flip costs zero SIGWINCHes" holds for Classic↔Mirrored only.
   Classic↔Reference-left also removes a rail, so `.main` gains 44px → exactly one coalesced resize of the agent per
   deliberate flip. Acceptable, but it's one, not zero, and the check should say so.
3. **`.float-stack` lands on the lower shell's prompt.** It is fixed at the window's bottom-left beside the rail; with
   reference left + terminal below, that is precisely the shell's prompt line, and the undo banner of a phase-3 drop
   would cover the line you are typing on. Phase 2 anchors the stack to the agent's bottom-left, lifted above the
   composer (it already takes a `bubbleLift`; add a `left`) — which is roughly where it sits in Classic today.
4. **A new place still opens with no reference panel.** `dock_open` is deliberately not a seed (the "dock spreads"
   complaint). For Ian, that means ⌘J in every new place before his layout appears. I would NOT reverse the rule —
   open question 3.
5. **Tab order runs backwards in Reference left** (agent before reference in DOM). Both noted DOM order in general;
   in the new default-for-Ian layout it is now the *common* case, not the mirrored corner. Phase 3's grips give a
   region-jump path; real reordering stays rejected (it re-parents an xterm).
6. **Status/usage.** The footer meter is in `.main` and moves with the agent; the usage strip spans the window; the
   rail tile stays in the Places rail. Popover `side` must be derived from the host rail's edge — which coincides with
   `places_side` in every layout because the tile never leaves the Places rail, merged or not. The offers button DOES
   move (into the merged rail) and opens a modal, so it has no side to get wrong.
7. **The agent isn't always Claude.** Codex and pi lanes occupy the same pane; the grip and menus say "Agent", the
   label shows the harness.
8. **Dragging into the agent** (place mentions, Files drags) is hit-tested at the pointer (`[data-drop="mention"]`), so
   side is irrelevant — but the hover-revealed nav now overlays the reference panel, so a place drag crosses one more
   panel before reaching the agent. Real-app check in §5.2.

## 7. Phase 2 and 3 file sketch

**Phase 2** — `settings.ts` (`dock_lower`, `dock_lower_frac`, rev 5 only if a default changes, otherwise none),
`App.tsx` (lower host after `.dock-body` inside `<aside className="dock">`, `stripUp`, Terminal rail-icon semantics,
⌘T routing, float-stack anchor, narrow-window notice), `App.css` (`.dock-lower`, splitter, guide line), `SettingsSheet`
(Terminal row), checks: `termfit` (min-height), `navhistory`/`navwire`/`tabsrestore`, `dockrail` (the rail now has a
"focus lower" mode), `layout-check` (`dock_tab !== dock_lower` normalisation). Spike first: mount `TermTabs` and one
other tab body concurrently and confirm no duplicate polls (`useDocs`, plan watcher) and one transport per shell.

**Phase 3** — `arrange.ts` (pure: zones per grip, current-position detection, the write), module-scope
`ArrangeOverlay` (`.arrange-catch` above the nav overlay's z 30, below menus' 40; `useEscape` entry; real
`PointerEvent`s in its check, since synthetic drags bypass hit-testing on the way in), grips in four headers,
`MoveMenu` (reuse `CtxMenu`), undo banner reuse, Reset; `layout-check` gains the zone ids; `termresize-check` gains
"zero resizes during a drag, one after a size-changing drop".

## 8. SIGWINCH budget (agent pane)

| Act | Agent resizes |
|---|---|
| Classic ↔ Mirrored, Reference-left ↔ Agent-left | 0 |
| Any flip that merges/unmerges the rails | 1 (±44px), after settle |
| Toggle / resize / move the lower terminal | 0 (the shell gets 1 on release) |
| Hover a drag over zones, cancel, Esc | 0 |
| Drop | 0 or 1, per the rows above |

## 9. Open questions for David

1. **Order of phases 2 and 3.** I put terminal-below before drag because drag over two booleans is thin and phase 2
   is Ian's remaining pain. Want phase 3a (drag for sides only) right after phase 1 instead?
2. **Lower slot beyond Terminal.** The field is typed for any section. Ship Terminal-only and widen on a real ask?
   (Recommended.)
3. **New places open closed.** Keep the no-seed rule (recommended), or seed `dock_open` only while
   `dock_side === "left"` — i.e. "reference-left people want reference everywhere"? The latter is exactly the
   spreading complaint, chosen deliberately.
4. **A collapse control on the lower strip** (JetBrains' "—"). Without it, hiding the shell means moving it back to
   the tabs. Ian says it's always open; I'd ship without and wait.
5. **Name of the tile.** "Reference left" or "Agent right"? The second names what the user is choosing for.
