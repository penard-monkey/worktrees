# Session: dock Files tab — document rendering + layout

- **Date:** 2026-08-02 → 2026-08-03
- **Worktree:** `ui-changes`
- **Branches:** `ui-files-viewer`, `ui-release-0.8.0` (both squash-merged + deleted)
- **PRs:** [#76](https://github.com/penard-monkey/worktrees/pull/76) (feature),
  [#78](https://github.com/penard-monkey/worktrees/pull/78) (release bump)
- **Release:** [v0.8.0](https://github.com/penard-monkey/worktrees/releases/tag/v0.8.0)
- **Planning files:** `planning.tar.gz` beside this summary

## What shipped

The dock's Files tab was a lazy tree over an editable `<textarea>` — fine for a
peek, useless for reading a doc. It now renders per file **kind** and lays out
to fit the dock's width.

**New modules** (all `app/src/`):

| File | Job |
|---|---|
| `filekind.ts` | path → `{kind, lang, mime, label}`; `humanSize`. One table per concern (ext, bare name, image, binary) |
| `highlight.ts` | 15-grammar single-pass tokenizer → `Tok[]`. Scans, never parses |
| `CodeView.tsx` | line-number gutter + highlighted `<pre>`; shared by the code viewer and markdown fences |
| `markdown.tsx` | `marked.lexer()` tokens → React nodes. No `innerHTML`, no sanitizer |
| `FilesPane.tsx` | tree + `FileView` + `ImageView` + binary placeholder + split layout + error boundary |

**Changed:** `App.tsx` (old tree/viewer deleted, −7.4 kB; `FilesPane` wired;
reading mode + ⌘⇧E + Esc), `App.css` (Files section rewritten), `tokens.css`
(new `--syn-*` ramp), `settings.ts` (4 new settings),
`app/src-tauri/src/lib.rs` (`read_file_base64`, `b64_encode`, `size` on
`FileContent`, cap clamps), `mock/install.ts` (new arm + fixtures for every
renderer), README + `make dev-app`.

Renderers: markdown (headings with slug ids, nested and task lists, GFM tables,
blockquotes, fenced code, links, relative images), source with gutter +
highlighting, images as `data:` URIs over a checkerboard with dimensions /
size / fit / 1:1, and named placeholders (PDF, archive, font, media) with Open
in editor + Reveal.

Layout: side-by-side past 620px of dock width, stacked under it, header button
to pin either, draggable divider (arrow keys too) with a ratio per axis, and
⌘⇧E reading mode spanning the main pane **and** the dock.

## Decisions

**D1 — read-only viewer.** The textarea, Save, ⌘S and the mtime-CAS conflict
banner are gone; editing goes through "Open in editor". *Why:* the dock edits
the same tree an agent is writing in the pane next door. Read-only removes the
whole class of clobbering, deletes the save-conflict UI, and means no editor
library is ever needed. `write_file` stays in the backend, unused by the
frontend — cheap to re-wire if editing is ever wanted back.

**D2 — `marked`, lexer only.** First runtime dep beyond React/Tauri/xterm.
*Why:* hand-rolled markdown gets GFM tables, nested lists and reference links
subtly wrong, and those are exactly what this repo's own docs use. Taking
`marked.lexer()` (not `marked.parse()`) means we construct every DOM node, so
there is no `innerHTML` and no sanitizer — the security property comes from the
architecture, not from escaping. +3.2 kB on the app chunk.
CLAUDE.md now records that "no UI libraries" means no *component* libraries and
that a parser we render ourselves is allowed.

**D3 — hand-rolled highlighting.** `lowlight`/`shiki` were the alternative
(~35 KB–1 MB). *Why not:* approximate colouring on a quick-peek pane is
harmless, unlike wrong markdown. Trade-off accepted and written into ROADMAP.

**D4 — auto layout + reading mode** over "always side-by-side" (unreadable at
`DOCK_MIN`) and "separate window" (window lifecycle, out of scope).

## Dead ends / gotchas

**A depth cap does not stop the stack overflow — the LEXER blows up, not the
renderer.** `"*".repeat(4000) + "a" + "*".repeat(4000)` is 8 KB, well inside
`read_file`'s 1 MB cap, and `marked.lexer` throws `RangeError` before any of our
code runs. The first fix (a `MAX_DEPTH` guard in `inline()`/`block()`) proved
nothing; the working fix is try/catch around the lexer call plus
`ViewErrorBoundary`. **There is still no error boundary above `App`** — an
uncaught throw anywhere in the tree unmounts the root and leaves a blank
window. Worth fixing globally (in ROADMAP).

**`preventDefault()` in `onClick` is not a security boundary.** A middle-click
fires `auxclick` with **no `click` event at all**, so the handler never runs and
the WebView follows the raw `href`. React 19 neutralizes `javascript:` at the
attribute level but nothing else — `file:`, `data:`, `vbscript:` and custom
schemes rendered intact. Sanitize where the attribute is *written*, never where
the click is *handled*. (`app/src-tauri/tauri.conf.json` has `"csp": null`, so
there is no second line of defence.)

**`cap + 1` on a frontend-controlled `max_bytes`.** Debug builds panic
(`overflow-checks` on); release wraps to 0 → `take(0)` → a non-empty file
reports `b64: ""`, `size: 0`, `truncated: false`. It *lies* rather than
degrading. Clamp any caller-supplied cap.

**Chrome tokens are not syntax colours.** Reusing `--txt-mute`/`--ok`/`--ai`
for code put 6/8 classes under 3:1 on tokyo-day and 5/8 on catppuccin-latte
(comments worst at **1.98:1**). Dark themes were fine, which is why it survived
visual review — the failure only exists on the two light themes. Contrast has
to be *computed*, not eyeballed. Fix: a separate `--syn-*` ramp per theme.

**Fixing one markdown bug broke another.** Unwrapping tight list items to stop
text wrapping under the checkbox flattened *nested* lists into literal text,
because marked nests a sublist inside the item's `text` token. The unwrap has to
bail when any child is a block-level token.

**`git branch --merged` is useless in a squash-merge repo.** It reported zero
merged branches out of 27, because squashing means branch commits never appear
in main's history. Classify by PR state via `gh` instead.

**Stale `origin/ui-next`** held the pre-squash commits of an already-merged PR,
so pushing this session's work to it was rejected. Worked on a fresh
descriptive branch instead of force-pushing a merged one.

**pnpm needs node ≥ `.nvmrc`.** The default node here is 22.12.0 and pnpm 11
refuses it; `nvm use` first. `make dev-app` now checks this up front (the same
guard `install-app` already had).

## Verification

- Gates: `cargo build --release -p worktrees-cli`, `make test` (bats, 241),
  `make lint`, `cargo test -p worktrees-core`, `tsc --noEmit`,
  `cargo check -p app`, `pnpm build` — all green
- CI: PR #76 9/9 checks, PR #78 10/10
- Release workflow exit 0; 10 assets incl. both signed `.app.tar.gz` + `.sig`
  and `latest.json`
- Four adversarial review agents. Confirmed-correct: the highlighter's
  round-trip invariant (10 real files × 16 grammars, 54 hostile inputs, 400k
  fuzz cases, zero violations), `b64_encode` vs python `base64` (16 vectors
  incl. 3 real PNGs + 1 MiB random, zero mismatches), `guard_under_projects`
  (symlink / `..` / FIFO / device all rejected), and marked token coverage
  (46 repo docs — every type hits a real switch arm)
- Contrast re-measured live in-browser after the fix: tokyo-day 4.54–5.05:1
- 40 screenshots in `~/.cache/worktrees/worktrees/ui-changes/`; `split-markdown.png`
  beside this summary is the pivotal one. (The two final captures — light-theme
  code and the widened reading mode — were taken after the scratch copy and lost
  when the repo's `shots/` was cleaned; their measurements above are recorded,
  the images are not.)

**Not verified:** nothing was run in the real Tauri app. The mock cannot prove
`read_file_base64` against real bytes, the `file:`-scheme refusal in a real
WKWebView, or middle-click behaviour there. v0.8.0 shipped without it.

## Follow-ups

In ROADMAP: real-app smoke of this feature, a global error boundary, the
in-document-anchor and entity-table gaps, and the syntax-highlighter's known
approximations.
