---
title: Read a place's docs in the browser
---

# Read a place's docs in the browser

- **Date:** 2026-09-16 → 2026-09-20
- **Worktree:** `.worktrees/live-docs`
- **Branches:** `live-docs-phase1`, `live-docs-integrate`, `live-docs-review-fixes`, `live-docs-close-out`
- **PRs:** [#301](https://github.com/penard-monkey/worktrees/pull/301) (phase 1) ·
  [#308](https://github.com/penard-monkey/worktrees/pull/308) (the viewer) ·
  [#311](https://github.com/penard-monkey/worktrees/pull/311) (post-merge review fixes)
- **Release tag:** none — ships in the next release off `main`
- **Planning files:** `planning.tar.gz` beside this summary (`.planning/brief.md` + research)

A place's documents, rendered in the real browser, live-updating as you edit
them, with a persistent left navigation and mermaid diagrams. Built on our own
loopback server after the third-party viewer it was designed around was dropped.

---

## What shipped

**The Docs tab is a tree** (`app/src/DocsPane.tsx`, `app/src/doctree.ts`).
Nested the way the directories are, collapse state remembered per place. It
remembers what you CLOSED, not what you opened, so a directory added tomorrow
appears rather than hiding inside a set recorded before it existed. Order is the
walk's — a `[docs] paths = ["b", "a"]` lists `b` first, because sorting would
quietly overrule the repo.

**The documentation server** (`app/src-tauri/src/docserver.rs`, 1,497 lines).
hyper on loopback, started on first use and never at launch. Routes
`/<token>/p/<place>/`, `/doc?path=`, `/index`, `/asset/<rel>`, `/viewer.js`.

**The derive** (`crates/worktrees-core/src/derive.rs`, `docs.rs`). The served
copy is not the repo's file: frontmatter stops being a "Metadata" block,
staleness is a live header rather than text baked into the document, and a
mermaid node naming a page in the index becomes a link to it. Nothing is written
into the user's tree — derived copies live in the app config dir.

**The browser bundle** (`app/viewer/`). A separate vite lib-mode build, which is
what makes mermaid admissible as the second dependency exception: it can never
be hoisted into `app/src` by a shared chunk, and
`app/scripts/viewer-boundary-check.mjs` proves that by walking the real import
graph rather than trusting the config.

**Live updates without a reload.** Conditional GET; a 304 when nothing moved;
block-level DOM patching keyed on a hash of each block's markdown, so an edit
lands without disturbing scroll position or an active text selection.

---

## Decisions, and why

**`mo` was dropped after being the whole design.** It never validates the `Host`
header (GHSA-6pff-wf7m-6f5h, reported 2026-09-16), so any web page could reach it
by DNS rebinding. A proxy in front could not fix it, because `mo` holds its own
loopback port — the vulnerable listener stays reachable. Writing the server
ourselves was less work than the alternatives and removed a dependency users
would have had to install, configure and update.

**Conditional GET, not SSE.** Browsers cap ~6 connections per origin; the sixth
open document wedges the page. Polling once a second with an ETag costs a
410-byte response when nothing changed.

**Content-derived block ids, not positional.** With positional ids, inserting a
paragraph at the top shifts every id below it and every block re-renders — the
reader's selection dies three paragraphs from anything that changed.

**A drill-down link is a fragment, not a URL.** Decided during the post-merge
review (see below). It is also why no derived file contains the token.

**`Staleness` has no `upstream` field.** Core measures against `base_ref()`, not
upstream. The struct is the guard: the wrong answer cannot be spelled.

---

## Dead ends and gotchas

**`file://` cannot detect change at all.** Measured in Chrome and Safari before
any server existed. This is the finding the entire architecture rests on.

**Seven defects in the first integration were found by running the app, and
none by a gate.** Including a blank page (the server served `<div id="app">`,
the bundle mounts `#root` — the contract never said who supplies the shell) and
a deep link that opened the right place at the wrong document (`?path=` when the
page routes on the hash).

**A methodological error worth remembering:** an image was "proved" fine by
constructing `new Image()` and watching it load. A constructed image is never
lazy — that tested a different *kind of element*. The real cause was
`loading="lazy"`.

**"Measured on this platform" did not say which platform.** A test asserted that
binding `0.0.0.0:<our port>` succeeds while a loopback-only listener holds it —
true on macOS, where it was measured, and false on Linux, which refuses a
wildcard bind over any holder. It passed locally and went red in CI for a server
that was bound correctly. All four combinations, measured on both:

| witness bind | macOS | Linux |
| --- | --- | --- |
| `0.0.0.0:P` | tells them apart | `EADDRINUSE` either way |
| `<lan ip>:P` | succeeds either way | tells them apart |

No bind means the same thing twice. The rule being guarded never mentioned
binds — it says a docs server reachable from the LAN is a data leak — so the
witness is now a **connect**, which answers identically on both.

**`release.yml` was invalid YAML for the length of the branch, and nothing
said so.** A replacement step was pasted at line 1 instead of into its job,
leaving `name:`/`on:` under an orphaned sequence item. GitHub answers that with
a startup-failure run carrying **zero jobs and no annotations** — it never
appears in `gh pr checks`. The workflow runs only on `v*` tags, so the first
exercise of the file would have been a release. Found because David opened the
run and asked about it.

**Three defects the gates could not see, found by a post-merge review:**

- *Going back to a document you had already read hung on "loading …" forever.*
  The ETag was cached per path and never invalidated while the payload was a
  single slot written only on a 200. A → B → back to A got a correct 304, wrote
  nothing, and rendered the loading line at 1 Hz. The nav exists to move between
  documents, so this is the second thing anyone does.
- *One project's URL served another's documents.* `forget_place` pruned the set
  `place_key` consults, so a removed place's route segment was reissued to the
  next project wanting that slug — same token, same port, same URL.
- *The mermaid drill-down was dead on both sides,* and wrote the per-launch
  token to disk for nothing: Rust baked an absolute loopback URL into every
  diagram and the browser stripped every `click` line, reporting the tool's own
  directives to the reader as the author's.

**A root file was listed twice on every Mac.** `is_regular_file(root/"README.md")`
is true on APFS for a file named `Readme.md`, while `seen` compared
case-sensitively. Two index rows for one file, the first naming a path that does
not exist on disk.

**A test that passed on the order it ran in.** `ISSUED` is process-global, so a
test asserting an exact route segment held only while no other test had claimed
it. The full suite passed on an accident of alphabetical ordering; a filtered
run failed with a message that reads like a `place_key` regression and is not
one.

**A recovery test that asserted the bug away.** It hand-set `fingerprint = 0`
before checking that a broken place recovers — which is exactly the skip that
made it *not* recover. A place broken by a re-open stayed 503 until the button
was pressed again.

---

## Verification

- Gates on the rebased tree: core **388** · cli **14** · app **124** (also clean
  at `--test-threads=1`) · bats **340, 0 not ok** · lint · `tsc --noEmit` ·
  **18/18** frontend check scripts.
- Bind and reachability semantics measured on macOS *and* Linux (docker),
  four combinations each — `bindprobe.mjs`, `reachprobe.mjs`, `udpprobe.mjs` in
  the scratch dir.
- `docsviewer-check.mjs` shown red on the pre-fix source (exit 1, 19 ok / 26 not
  ok) and green after (52 ok) — the same script judging both trees; re-verified
  independently during integration.
- Every drift check mutation-tested: each fails on the code it guards.
- The cross-language mermaid seam guard proven in both directions, including
  with a planted decoy `format!`.

---

## Follow-ups

In `ROADMAP.md`:

- Derive outside the `srv`/`places` locks — every server request stalls for a
  derive today. Much closer to possible now: `write_tree` became a pure function
  of `(tree, root, entries)`.
- A workflow-parse gate in CI, since a startup failure is invisible to every PR
  check by construction.
- `image_refs` reports images inside 4-space indented code blocks.
- Refuse a duplicate `Origin` the way `Host` already does.
- The accept-loop backoff has no test (needs process-wide fd exhaustion).
- GHSA-6pff-wf7m-6f5h — whether upstream ever ships a `Host` check.

**Stragglers, left in place deliberately** (removal is the user's call, not a
close-out's): five worktrees spawned for this stream still exist —
`docs-server`, `docs-page`, `docs-transport`, `docs-render` (the no-`mo`
investigations, whose work is merged in #308) and `live-docs-mo` (the abandoned
`mo` branch, kept as the record of what was dropped and why). The remote
branches `origin/live-docs-integrate` and `origin/live-docs-review-fixes` are
squash-merged and so are not ancestors of `main`; `git branch -r --contains`
reports them unmerged, which is expected and not a reason to keep them.
