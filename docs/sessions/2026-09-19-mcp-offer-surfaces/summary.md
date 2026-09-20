---
title: "The suggestion nobody saw"
---

# The suggestion nobody saw

- **Date:** 2026-09-19
- **Working tree:** `.worktrees/bug-fixes`
- **Branches:** `bug-fixes-mcp-nudge` (feature), `bug-fixes-close-out` (this archive)
- **PR:** [#304](https://github.com/penard-monkey/worktrees/pull/304) — squash-merged as `9a253a9`
- **Release tag:** none — sits in `[Unreleased]`
- **Planning files:** none. The work started as a support question and turned
  into a design pass in-thread, so there is no `planning.tar.gz` beside this
  summary.
- **Design record:** a clickable artifact of the three candidate surfaces was
  built during the session (private link, not in the repo). Its content is
  reproduced below under *Decisions*.

The ask began as a support question: a contributor updated the app, still had no
MCP server, and was never offered one. v0.25.0 (#218) had shipped exactly that
offer. Why had it not fired?

## What shipped

**The diagnosis first, because it decides everything after it.** The card's
render condition (`App.tsx`, since deleted) was:

```
canNudge(mcpStatus) && !settings.mcp_nudge_dismissed
  && sel === null                       // the Home screen only
  && (ws?.projects.length ?? 0) > 0     // and only with a project
```

Their status was `absent` with both binaries resolved — every test the detection
runs, it passed. Two *surface* preconditions, each defensible alone, multiplied
to almost never. Nobody had checked what they multiplied to.

Worse: the surface that reaches everyone had already carried the message.
v0.25.0's release notes said *"The app can wire Claude's MCP server up for you"* —
entry four of six, as prose, with nothing to press. It converted nobody.

**`app/src/offers.ts` (new)** — one registry behind every after-update
suggestion. `pendingOffers(ctx, dismissed)` answers what is pending; an `Offer`
is plain data carrying a destination (`to: { cat, focus }`), never a function.

**The release notes became the surface.** `WhatsNewModal` renders pending offers
in a band between the modal `<header>` and `<div className="settings-body">` —
the only child that scrolls — so a long changelog cannot push it under a fold.
The Home card is deleted.

**Settings gained a deep link.** `settingsOpen: boolean` became
`settingsAt: { cat, focus } | null` with `open` derived from it; `SettingsSheet`
honours the target and flashes the named section (`data-focus="mcp-server"` on
`McpSection`).

**The gear dot carries the residue.** `.rail-icon.upd` already existed and
already meant "something in Settings needs you"; it gained a second source, and
an `upd-offer` variant in `--ai` purple so an unset-up server is distinguishable
from a stale CLI without hovering.

**Settings → Claude carries "Stop suggesting this"** — the only on-demand way to
end the offer. See *Dead ends* below; this was missed on the first two passes and
found by review.

**`mcp_nudge_dismissed` → `offers_dismissed: Record<string, string>`** (id →
fingerprint), `SETTINGS_REV` 2 → 3 with a migration carrying an existing
dismissal across.

**`mcp_status` now logs its verdict** (`app/src-tauri/src/lib.rs`).

**`app/scripts/offers-check.mjs` (new)** — slices the real `offers.ts` and pins
which states offer, what a dismissal silences, the modal's `offers=` feed, and
the panel's dismissal.

## Decisions

**An offer's action is a deep link, never the act.** The inline "Set up now"
button was the obvious design and is wrong: the MCP panel's Set up carries a
*permissions* choice (`--mutations` vs read-only — whether Claude may close and
**remove** worktrees) plus the caveat about discarding uncommitted work. An
inline button either drops that decision or rebuilds the panel inside a modal.
The corollary is what makes several offers tractable: N offers are N rows with N
links, not N embedded panels needing an ordering rule.

**Dismissal stores a fingerprint, not a boolean.** Generalised from
`init_dismissed` (content hash, so a repo that gains a credential file
re-suggests) and explicitly not from `mcp_nudge_dismissed`, which was safe only
because its one suggestion could never change.

**A dismissal silences one offer, never a class.** `stale` / `read-only` /
`foreign` are problems, not offers: not dismissible, Settings-only.
`cli-missing` stays silent because Updates owns that sentence.

**The coachmark was designed, then cut, and stayed cut.** Candidate B included a
popover anchored to the gear. It was dropped as the only piece of new chrome and
the weakest link — dismissed reflexively, after which you rely on the dot anyway.
The written prediction was that if the badge proved too quiet we would know by
measurement rather than argument. It did, and we did (see *Dead ends*), but the
answer turned out to be prominence and an off switch, not a second interruption.

**The band is not a card.** The notes below it are a column of bordered entries,
so another bordered box reads as one more of them. Full-bleed, un-rounded,
tinted `--ai` over `--bg-tree` (the modal's real surface). Purple because that is
already this repo's word for claude, which also keeps it clear of `--accent`.

## Dead ends / gotchas

**Three separate attempts put the thing to DO after the thing to READ.** This is
the session's one real lesson. v0.25.0 buried it as changelog entry four of six;
the first cut of the band sat *below* the notes and was "barely noticed" by the
person who designed it; the gear dot was not noticed at all. Placement beat copy
every time.

**The residue had no off switch, and review caught it, not testing.** The band
is the only surface that could silence an offer — and it is the one most people
never see: the What's-new modal opens once per version, and on a fresh install
(`!merged.last_seen_version`) it never opens at all. So the commonest way to meet
the offer was a purple dot with no way to clear it but installing the server.
Closing the notes with ✕ left the same standing dot, since that records the
version without silencing anything. The tell was already in the tree: a comment
in `SettingsSheet.tsx` justifying an un-badged `claude` category with *"the Home
card is already making that offer"* — of a card this branch deletes. **When a
change deletes a thing, grep for comments that reason FROM it.**

**A drift check can be green and still guard nothing.** `offers-check.mjs` was
shown red on five mutations before being trusted. Review then defeated it in a
sixth way: gating the modal's `offers=` **prop** (`offers={sel ? offers : []}`)
passed all fourteen checks — the original bug relocated one line off the render
the check pinned. Pinning a render says nothing about its feed.

**A vacuous test looks exactly like a passing one.** The first "is the band
pinned?" measurement returned `movedOnScroll: 0` — because the notes body did not
overflow at that viewport, so nothing scrolled. Re-run at 520px tall:
`overflows: true`, `scrolledTo: 33`, `movedOnScroll: 0`. Only the second one
meant anything.

**Two `gh` commands lied in opposite directions within a minute.**
`gh pr checks --watch`, run in the gap between a push and the run registering,
prints *"no checks reported"* and exits **0** — indistinguishable from a pass. A
hand-rolled `jq` rollup then counted nine *failing* jobs, because `gh` reports
`conclusion: ""` for in-progress work, not `null`, so `select(.conclusion != null)`
catches everything running. Use `gh run watch <id> --exit-status` and read raw
conclusions.

**`sandbox.sh` deliberately does not isolate `$HOME`**, which is right for AI
profiles ("testing a profile means checking your global CLAUDE.md still loads")
and fatal for this: `mcpsetup::status()` reads `$HOME/.claude.json`, so on a
maintainer's machine the sandbox always reports `installed` and the offer can
never appear. Reproducing `absent` needs a throwaway `HOME` with the real `PATH`
kept — drop the PATH and `worktrees` stops resolving, the state becomes
`cli-missing`, and *that* state deliberately suppresses the offer, so you would
be measuring the wrong thing and concluding the nudge is broken. Wrapper:
`~/.cache/worktrees/worktrees/bug-fixes/mcp-sandbox.sh`.

**`sandbox.sh --app` isolates the config dir but not the log.** Config lands
under `net.casadelvalle.worktrees.sbx`; `app.log` goes to the *plain*
`net.casadelvalle.worktrees` — i.e. your real app's log file. Only the faked
`HOME` kept this session's sandbox out of it.

**A missing sandbox means the human closed it.** Twice a vanished sandbox app was
reported as a crash and a relaunch started, once with a `setsid` "fix" for a
problem that did not exist. `tauri dev` exits 0 when the window closes, which is
identical to a clean shutdown because it is one; a harness kill is 143.

**A TDZ error hid behind a plausible refactor.** Moving the `offers` memo above
the `mcpStatus` state compiled in the editor's head and failed `tsc` with
*"Block-scoped variable 'mcpStatus' used before its declaration"* — a real
runtime error, not a lint. Hook-order moves need the typechecker, not reasoning.

**Perl's `s{}{}` cannot take CSS.** A batch of style edits silently did nothing
because a literal `{` in the pattern unbalanced the delimiters; the script
aborted at compile time and a later `grep` found a pre-existing match, which read
as partial success. Plain-string replacement in Python for anything brace-heavy.

## Verification

- Reproduced the contributor's exact state before writing any fix: real
  `worktrees mcp --status --json` under a throwaway `HOME` → `state: absent`,
  `found_in: []`, both binaries resolved.
- `offers-check.mjs` shown **red first** on seven mutations in total: a nudging
  `stale`, a boolean dismissal, a `sel === null` render gate, a missing
  `data-focus`, a dropped migration, a gated `offers=` feed, and a panel with no
  dismissal.
- Flow driven headlessly against the mock with `elementFromPoint` hit-tests, not
  rects: band above the notes with the body genuinely overflowing, band moves
  **0px** scrolled to the end, CTA reachable, deep link lands on **Claude** with
  the section on screen; `installed` and `stale` render nothing; fresh install
  with no notes → dot lit, off switch reachable, dot cleared after silencing
  while the panel still says "Not set up".
- Band tint measured against its host in all six themes: **21, 21, 27, 28, 34,
  35** (floor: tokyo-day and nord), against the 16 the Home card was held to.
- Exercised by hand in the isolated sandbox app.
- Gates on the merged tree: bats **340 ok / 0 not ok**, lint, core **338**, cli
  **14**, app **58**, `tsc --noEmit`, `cargo check -p app`, **14** drift checks.
  Re-run after rebasing onto #303, which touched every file this PR did.
- CI green on both pushes (9/9 jobs).

## Follow-ups

- **`pendingOffers` is fed a `repo: null` probe**, so it cannot see a server
  installed at *local* or *project* scope — such a machine is covered and still
  gets offered the server until Settings → Claude re-probes with the repo and
  corrects it. Inherited from the Home card; it matters more now the dot is on
  every screen. Documented in `offers.ts`, not fixed.
- **`cli-missing` is not an offer.** It was kept silent so it could not compete
  with Updates; now that offers carry a `settingsCat`, that competition
  disappears and it could become one.
- **`init_dismissed` keeps its own store.** Folding the app half into
  `offers_dismissed` is cheap; folding both would mean the CLI reading an
  app-owned file, which ADR 0001 refuses.
- **The coachmark remains unbuilt**, deliberately. If the band plus the coloured
  dot still under-reach, it is strictly additive — and by then it would be a
  measurement.
