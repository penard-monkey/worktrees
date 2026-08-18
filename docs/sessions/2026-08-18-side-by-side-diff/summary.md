---
title: "2026-08-18 — side-by-side diff in the Files tab"
---

# Session: the Files tab shows what changed, not just what is there

- **Date**: 2026-08-18 (work started 2026-08-17)
- **Worktree**: `.worktrees/ui-tweaks`
- **Branches**: `ui-tweaks-side-by-side-diff` (feature),
  `ui-tweaks-close-out` (this archive). Idle base for this tree stays `ui-next`.
- **PRs**: [#159](https://github.com/penard-monkey/worktrees/pull/159)
  (feature, squash `68300a2`)
- **Release**: none — sits in `[Unreleased]` for the next one
- **Planning files**: `planning.tar.gz` beside this summary

## What shipped

The Files tab could already *mark* a changed file (tint + directory counts,
from `changed_files`, since the 2026-08-11 session). It could only ever show
that file's **current** contents. It now shows the diff, two columns aligned
row for row, with the changed words inside a changed line picked out.

[side-by-side.png](side-by-side.png) is the reading-mode view — note the
word-level marks on `import_old` → `import`, and the hatched left half of the
inserted row. It was captured mid-session and so predates one later fix (the
sticky gutters' opacity, which only shows when scrolled horizontally);
everything else in it is as shipped.

| Piece | File |
| --- | --- |
| Unified-diff parser → aligned rows + reconstructed old/new text; word-level diff | `app/src/diff.ts` (new) |
| Two-column grid renderer, side-by-side / unified, word marks | `app/src/DiffView.tsx` (new) |
| `tokenizeLines()` — whole-file tokenize, then cut at `\n` | `app/src/highlight.ts` |
| `file_diff` command, `trim_patch`, `patch_is_binary`, `file_diff_for` | `app/src-tauri/src/lib.rs` |
| Diff toggle + Base/HEAD seg; changed-only tree filter + auto-expand | `app/src/FilesPane.tsx` |
| `files_diff` (per-place, optional), `files_diff_base`, `files_changed_only` | `app/src/settings.ts`, `app/src/App.tsx` |
| Grid, tints, hatch, sticky-safe gutters, `.vh` | `app/src/App.css` |
| Mock parity for `file_diff` | `app/src/mock/install.ts` |
| Parser round-trip gate against real git | `app/scripts/diff-check.mjs` (new) |

## Decisions

**The line diff is git's, at full context.** `file_diff` runs
`git diff --unified=1000000`, so ONE payload carries both complete versions of
the file *and* their alignment. Rejected alternatives: a JS Myers/LCS over two
blobs (we would own and have to test the algorithm), and ordinary `-U3` hunks
(smallest payload, but limited context, an extra round-trip to expand, and —
the deciding factor — highlighting with no whole-file context).

**Because that highlighting is the whole reason.** `tokenize()` deliberately
emits tokens that span newlines; `CodeBlock` gets away with it because its
gutter is a separate column. A diff row cannot contain a token that crosses
rows, and tokenizing a hunk alone mis-colours every line whose block comment or
string opened above it. `tokenizeLines()` runs the existing scanner over the
whole source and splits afterwards — scanner untouched.

**One CSS grid, not two panes.** Each aligned pair is a grid row, so the row
takes the height of its tallest cell and alignment is a property of the layout.
Two synchronised scrollers need JS to stay in sync and still drift the moment a
line wraps, because the two sides wrap at different lines. `width: max-content`
is what makes the two `1fr` content tracks come out equal and as wide as the
longest line on either side.

**Merge-base by default, with a Base/HEAD toggle.** `BASE_CANDS` is now shared
by `changed_files` and `file_diff` so the two can never resolve different
bases — a diff taken against a different base than the tint that made you click
would contradict it.

**Full file, nothing collapsed.** No expander state at all; `MAX_ROWS` (8000) is
a DOM backstop and says so on screen when it bites.

**Four things in the first slice, not one.** Side-by-side + unified fallback,
word-level intra-line marks, per-place persistence, and the "changes only" tree
filter that had been parked in `ROADMAP.md` since the markers session.

## Dead ends / gotchas

**A probe of mine ran git against the real repo.** The first `base-agree.sh`
let a repo path come back empty — bash expands *every* assignment on a
`local a=1 b="$a"` line before binding any of them — so `git -C ""` aimed
`branch -M main` and `push origin main` at this working tree. Git refused both
(the worktree guard on the rename; non-fast-forward on the push) and remote
`main`, local `main`, HEAD and the tree were all verified unchanged. The lesson
is not "be careful": it is that **every git call in a throwaway script needs a
guard that hard-exits unless the directory is non-empty and under `$TMP`**. An
empty `-C` is not an error to git; it is "here".

**`… | grep -q X` under `set -o pipefail` reports FAILURE on a match.** `-q`
exits at the first hit, the upstream `sort` takes SIGPIPE, and pipefail
promotes that to the pipeline's status. Two cases then looked like product bugs
and were not. Same family as the `grep -c` trap already in CLAUDE.md. Write to a
file, then grep the file.

**A new test that passes with the code broken is not a test.** Of the three
Rust tests written for `trim_patch`/`patch_is_binary`, two went red immediately
when the code was reverted — and the UTF-8 one did **not**. The fixture's line
length happened to put `DIFF_MAX` on a character boundary, so `is_char_boundary`
was never exercised. The fixture now straddles deliberately *and asserts that it
does*, so a future `DIFF_MAX` cannot silently retire it.

**My first fix for the word diff removed working highlights.** The docstring
promised "lines sharing nothing get no marks"; the code marked the whole line.
Fixing it as "the LCS kept nothing → no marks" is wrong in the dangerous
direction: for `const foo = 1;` → `const bar = 1;` the prefix/suffix trim leaves
`foo` against `bar`, which share no atom, so the one word you actually changed
stopped being marked. `diff-check.mjs` caught it. Correct predicate:
**no common prefix AND no common suffix AND the LCS kept nothing.**

**Measuring colour: `getComputedStyle` returns two different syntaxes.** A
resolved `color-mix()` comes back as `color(srgb 0-1 / a)`, plain colours as
`rgb(0-255)`. Parsing both as 0–255 made additions and deletions measure
*identical*, which read as a real bug. With the parse fixed, the genuine finding
appeared: the "no line here" cell sat 7–12 RGB units from context — invisible —
so it became a hatch, and add/del gained a saturated inset edge instead of more
alpha (alpha is paid for in text legibility).

**A `sticky` cell whose tint replaces an opaque background is see-through.**
`.dg` sets an opaque `--bg-tree`; `.dg.del` (higher specificity) replaced it
with a 14%-over-*transparent* mix, and that column is `position: sticky` inside
a horizontally scrolling `max-content` grid. Code slid visibly under the pinned
line numbers — and *only on changed rows*, i.e. exactly the rows being read.
Gutter tints must mix over `--bg-tree`, not over `transparent`.

**`git_out` returning `None` is a non-zero exit, not "no output".** Collapsing
it with `unwrap_or_default()` turned any git failure into an empty patch, which
is documented and rendered as "identical to the base". Reachable: an unborn HEAD
with a **staged** file — it *is* tracked, so the untracked arm misses it, every
`merge-base` fails, the `HEAD` fallback exits 128, and the viewer said "no
changes vs HEAD" over a row the tree had just tinted `added`.

**"Nothing changed on this branch" was asserted before anything had loaded.**
The changed-only filter's empty-state note rendered against `NO_CHANGES` — the
initial state — so it appeared for the first few hundred ms of every mount of
the real backend's git fan-out, and *permanently* after a failure (markers are
additive, so `NO_CHANGES` sticks). The mock cannot show this: it answers in a
microtask. Exactly the bug class CLAUDE.md documents three prior instances of.

**`sandbox.sh --app` cannot be driven from here.** It `exec`s `tauri dev` into a
native WKWebView window; Playwright drives Chrome. Its scratch repo also has no
changed files, so there would be nothing to diff. What *was* done instead was to
verify the claim the mock genuinely cannot express — that `changed_files` and
`file_diff` agree on real git — with a script over eight synthetic repo shapes.

## Verification

- **`app/scripts/diff-check.mjs`** (new, committed): for every path the branch
  changed, the parser's reconstructed old side must equal `git show <base>:<p>`
  and its new side the working file, byte for byte. 12/12. Earns trust by
  failing when `flush()` is corrupted.
- **Base agreement across 8 repo shapes** (committed-only, uncommitted-only,
  base moved on, no origin, no main/master, unrelated histories, rename,
  staged-in-unborn): the two candidate loops always settle on the same ref, even
  though one breaks on the first `diff <cand>...HEAD` that succeeds and the
  other on the first `merge-base` that does. Script archived to the scratch dir.
- **Layout and colour measured, not eyeballed** (`getComputedStyle`): 18/18 grid
  rows share top and height, wrap on and one row wrapped to two lines; every
  content cell the same width (the inset edge costs no layout); zero translucent
  sticky cells in *both* layouts — the `add` gutter is sticky only in unified,
  so that fix mattered specifically there; tint separation across all six themes.
- **42 Rust unit tests** (`cargo test -p app --lib`), 4 of them new here. Each
  new one was shown to fail with its fix reverted.
- Full gates green on the final base; **all 9 CI checks green** before merge.

## Follow-ups

Both are in `ROADMAP.md`:

- **The diff has no manual side-by-side / unified pin** — it picks by measured
  body width (`DIFF_SIDE_AT`, 560px) where the tree beside it gives
  `files_layout` an explicit cycle.
- **The add/del edge is under 3:1 on two light themes** (~2.4:1 for deletions on
  Tokyo Day, additions on Catppuccin Latte). Found while measuring it:
  **`--syn-com` is already below 4.5:1 against `--bg-abyss` on all four dark
  themes** (Nord 3.13, gruvbox-dark 3.52, Catppuccin Mocha 4.02, Tokyo Night
  4.09) — pre-existing and unrelated to the diff, but `tokens.css` claims the
  ramp is ≥ 4.5:1 and that is only true of the light pair.

And one thing this session did **not** do:

- **The feature has never run in the real app.** Everything was the mock harness
  plus direct git probes. `file_diff` is a real git spawn per file, and CLAUDE.md
  is explicit that the mock cannot express real timing. A human pass on a real
  repo is still worth having before the next release.
