---
title: Where you left off: execution plan
---

# Where you left off: execution plan

**Plan, 2026-11-27. Stage 1 is being built; nothing has merged.** This turns
[the investigation](where-you-left-off.html) and the owner's decisions (its
§10 to §12) into work that can be handed out. The investigation holds the
reasons and the file references; this page holds the order, the contracts and
the checks.

## How the work runs

- Each slice is one lane on its own branch, started from a brief and built by
  a Claude or a Kimi agent (the owner, 2026-11-27: use both a lot). The lane
  commits, runs the gates and reports; it does not push. First hand-out:
  1a and 1b to Claude, 1c to Kimi, side by side.
- The diff is reviewed before `(main)` pushes it, opens the pull request and
  squash-merges.
- Everything on screen sits behind one beta switch, `beta_left_off`, default
  off (`app/src/beta.ts`). The engine row is not gated, as the beta rule says.
- Every slice shows its new tests red first, and anything touching the return
  rule or the panel is run in the real app (`app/scripts/sandbox.sh --app`)
  before it is called done, because the mock answers instantly.

## Stage 1: the digest by rule

Useful alone: on returning after six hours, a panel lists at most five places
that want the owner, with no model and no character.

| Slice | Branch | Contents | Depends on | Size |
|---|---|---|---|---|
| 1a. Cards and ranking | `feat/left-off-digest` | `worktrees_core::digest`: the card per place from the existing readers, the privacy rule, the ranking and one template line per card; `worktrees digest [--json]` | Nothing | 2 to 3 days |
| 1b. The row | same branch | serve's `digest` row, console only; its mock answer; the dispatch and boundary checks | 1a | 1 day |
| 1c. The return rule | `feat/left-off-return` | `app/src/returning.ts`, a pure module: last active, the six-hour gap, once a day; persisted; a check on a virtual clock | Nothing | 1 to 2 days |
| 1d. The panel | same branch | The panel (pattern: `UpdateBubble.tsx`), five rows, "next", "what else", shrink to a rail icon; the beta switch; Home's list reading the same row when the switch is on | 1b, 1c | 2 to 3 days |

Three pull requests: the engine (1a, 1b), the return rule (1c), then the panel (1d).

### Contract: the row

`digest { since?: <epoch secs>, limit?: <n, default 5>, offset?: <n> }`,
capability `read`, console only in the body for now (`Ctx::is_console`).

```json
{
  "generated": 1795811733,
  "changed": true,
  "total": 12,
  "rows": [
    {
      "project": "worktrees",
      "repo": "/abs/root",
      "slug": "fix-thing",
      "title": "Fix the thing",
      "reason": "waiting",
      "line": "Waiting on your answer since 14:02",
      "at": 1795800000
    }
  ],
  "left_out": { "private": 2, "unreadable": 0 }
}
```

- `reason` is one of `waiting`, `finished`, `pr`, `draft`, `at-risk`,
  `recent`, and is also the ranking's first key, in that order; then newest
  first.
- `changed` answers the panel's gate: has any place finished work, started
  waiting or changed pull-request state after `since`? With no `since` it is
  `true` when `rows` is not empty.
- `line` is built by the engine from the card with a fixed template per
  reason. No model text, no transcript text beyond the last prompt cut to 80
  characters, and that only through `turns`.
- A private project, and a project nested in one, gives no card and is
  counted in `left_out` (the rule of `workspacechats::project_reads`).
- The row never starts a harness and never writes.

### Contract: the return rule

One pure function in `returning.ts`, in the style of `reconnect.ts`:

- State: `last_active` (epoch ms, persisted), `shown_day` (local date of the
  last panel, persisted).
- Events: input, hidden, visible, focus, unload.
- A return is the first visible, focus or input event whose gap from
  `last_active` is more than six hours, when `shown_day` is not today.
- On a return the app asks `digest { since: last_active }` and opens the
  panel only when `changed` is true. Either way `shown_day` becomes today
  only when the panel opened.
- The panel never changes the selection and never takes focus.

### Done means

- `worktrees digest` on this machine answers in under a second for every
  registered project, with the time written in the lane's report.
- A private project's places appear in no row, proved by a test that fails
  with the rule removed.
- With the switch off, nothing on screen differs from today.
- In the sandbox app, with the clock gap forced, the panel opens once, a row
  enters its place, "next" and "what else" work, Escape shrinks it to the
  rail, and it does not reopen that day.

## Later stages, in order

| Stage | First slice | Open before it starts |
|---|---|---|
| 2. The model's lines | The provider setting (none, a harness, a local endpoint) and one measured run of a local model on real cards | Which local model the owner runs, and its endpoint |
| 2b. Goals | The user-level goals document and the helper's proposals into it (investigation §11.5, A filled by B) | Its own idea file, as it is the first knowledge kept about the user |
| 3. Tour and tips | The step list shared with the panel, the seen record, the Settings list, the selector check | The seven steps' wording |
| 4. Rama | Sprite playback and pointing in the app, against placeholder frames | The Blender model (below) |
| 5. Talk to it | The helper's chat as a workspace chat | The workspace chats beta |

Stages 2 and 3 can run side by side after stage 1 merges. Stage 4's code can
start on placeholder frames while the model is made.

## Rama: the owner's track, which can start now

This needs no code and is the longest lead, so it runs beside stage 1.

1. **Model.** The logo's figure in Blender: ring head, two arms ending in
   round hands, a trunk, and legs to try (they can be hidden). Flat or
   near-flat shading in the logo's green so frames sit beside the interface.
2. **Rig.** A standard human armature with the usual bone names, so Mixamo,
   CMU and Rokoko clips retarget without hand mapping.
3. **Clips**, each short and looping where marked: idle (loop), wave, point
   (eight directions, or one pose turned), walk or hop (loop), think (loop),
   celebrate, shrink to the rail, grow from the rail.
4. **Render.** Transparent frames, square, at two sizes (for 1x and 2x),
   packed as WebP sprite sheets by a script kept in the repository.
5. **Files.** The repository takes the sheets and a `.blend` holding only our
   own or CC BY motion. A file with Mixamo or bought clips stays private
   (investigation §11.3).

What the app needs from this, to be fixed before clips are rendered in bulk:
frame size, frame rate and the anchor point (where the feet are in the
frame). Proposed: 256 px and 128 px, 24 frames a second, anchor at bottom
centre.
