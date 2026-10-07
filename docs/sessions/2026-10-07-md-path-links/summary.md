---
title: "2026-10-07 — clickable file paths in rendered markdown"
---

# Clickable file paths in rendered markdown

- **Date:** 2026-10-07
- **Worktree / branch:** `md-path-links`
- **PR:** #451, squash-merged as `a25d6ed` (after a fable review and green CI)
- **Planning files:** only the brief (`.planning/brief.md`), in `planning.tar.gz`
- **Scratch:** gate logs, the `git ls-files` timing script (`lsfiles.sh`) and the harness log are in `~/.cache/worktrees/worktrees/md-path-links/`

## What shipped

Paths in the Files tab's markdown preview (and reading mode) and in the Plan
tab are links. A plain click opens the file in the Files tab, which also
reveals it in the tree (#449). `path:line[:col]` opens it at that line, the
same as the terminal's ⌘-click.

- `app/src/mdpaths.ts` holds the pure rules:
  - `codeSpanHit`: an inline code span is a candidate only when it is a path
    from end to end. A bare name counts.
  - `textHits`: in prose, only a token with a `/` is a candidate.
  - `batches` and `resolveDocPaths`: candidates go to the backend in batches
    of `DOC_PATHS_BATCH` = 64, which equals `TERM_PATHS_MAX`. A failed batch
    is logged and only its own candidates stay unanswered.
  - All of it reuses `findPaths` from `termlinks.ts`.
- `app/src/markdown.tsx` takes an optional `pathLinks` prop.
  - The renderer collects candidates in a `found` set as it draws them, so
    there is no second walk over the tokens.
  - An authored link's text is rendered with `pathLinks: undefined`.
  - Fences and raw HTML never reach the leaf helpers.
  - A link is `<span role="link" class="md-link md-path">` with no `href`.
- `app/src/useDocPathLinks.tsx` is the hook the hosts use.
  - It caches answers per (root, doc).
  - On a generation change it re-asks every candidate on screen.
  - It keeps the same store when nothing changed.
  - `want` and `open` keep their identity across renders: `open` reads the
    host's `onOpen` through a ref, and `want` reads the generation through
    one.
  - It owns the "open which?" `CtxMenu` for a name several files share.
- `app/src/FilesPane.tsx` (FileView) and `app/src/PlanPane.tsx` are the two
  hosts. `FileView` gained a `root` prop. `onOpen` in both now takes
  `at?: {line, col}`.
- `app/src-tauri/src/lib.rs`:
  - `resolve_doc_paths` is a new command with bases `[doc dir, place root]`.
  - `resolve_doc_path` decides every answer through the existing
    `resolve_term_path`, including each match from the name index.
  - `DocNameIndex` caches one `git ls-files -z --cached --others
    --exclude-standard` per root, keyed on the frontend's generation (the
    Files reload token).
  - `basename_index`, and the `DOC_NAME_CHOICES_MAX` = 12 cap on menu entries.
- `app/src/mock/install.ts`:
  - Mirrors `resolve_doc_paths` over the fixture tree.
  - `MOCK_MD` and the freeform plan fixture now name some paths, so both tabs
    can be driven in the harness.
- `app/scripts/mdpaths-check.mjs` is a new check (41 assertions).
- `app/scripts/mdedit-check.mjs` now stubs the new hook import.
- `CHANGELOG.md` has an `### Added` entry under `[Unreleased]`.

## Decisions

- **No separate candidate walker.** Two walks over marked's token tree would
  be a mirror that drifts the first time someone adds a case to `inline()`.
  The renderer records each candidate as it calls `codeSpanHit` / `textHits`.
  A `useEffect` hands the set to the host after the render that drew it.
- **Prose needs a `/`; a bare name only counts in backticks.** Without that,
  every word like `App.tsx` or `e.g` in a sentence costs a stat and could
  become a link. Backticks are the author saying "this is a name".
- **Bare names shipped in this PR (stage 2).** `git ls-files --cached
  --others --exclude-standard` measured, including timer overhead:

  | project | files | time |
  | --- | ---: | --- |
  | worktrees | 566 | 22–24 ms |
  | valleos | 1,374 | 26–31 ms |
  | anime-song-remover | 3,464 | 23–25 ms |
  | casa-del-valle-monorepo | 3,618 | 33–36 ms |

  `--others` is included because anime-song-remover tracks one file and has
  3,463 untracked. A plan also names files a branch is still adding.
- **Index matches are re-resolved.** A stale entry (the file was deleted) or a
  tracked symlink that leads out of the project is refused by the same rules
  as a terminal path.
- **Fences stay unlinked.** Commands, imports and strings would be noise. If
  fences ever get links, limit it to `sh`/`console` output, opt-in.
- **Plain click, dotted underline.** This is prose, not a terminal, so no ⌘ is
  needed. The dotted underline sets a detected path apart from a link the
  author wrote.

## Dead ends / gotchas

- **An SSR render runs no effects**, so `renderToStaticMarkup` never calls
  `want`. The check sees extraction through `answers.get` instead: the
  renderer looks up every candidate it draws, so a recording map shows
  exactly the set. That also proves pending answers produce byte-identical
  markup.
- **`mdedit-check.mjs` evaluates FilesPane with a hand-made `env` of its
  imports.** Any new import in FilesPane.tsx must be stubbed there. If it
  isn't, the failure is a 100 KB base64 data: URL dump, not a name.
- **`usage-check.mjs` rejects a `<button>` whose `title` is an expression and
  that has no `data-track`.** The ambiguity menu needed
  `data-track="md.path.choose"`.
- **`make test-frontend` stops at the first FAIL.** "13 ok" meant 13 of 36
  checks had run, not that 13 passed.
- **A host passing an inline arrow breaks Markdown's memo.** PlanPane's
  `onOpen` is an inline arrow in App. Feeding it into `open` would have given
  `pathLinks` a new identity every App render, and Markdown would rebuild its
  body (~165 ms on a 1 MB doc). Fixed in the hook with a ref. The same flaw
  already exists one level up; see Follow-ups.
- **The `want` identity had to survive a refresh too.** With the generation
  in `want`'s deps, every `places:changed` would rebuild the body even when
  nothing changed. The generation is read from a ref, and a refresh re-asks
  from a `wanted` ref inside an effect keyed on `generation`.
- **Mock fixture artefact:** `seedDir` falls back to the root listing for any
  directory it doesn't know. So the harness's name index held seven `lib.rs`,
  and a plan's directory "contained" `src/App.tsx`, which the doc-dir base
  found first. The real filesystem does neither, so this is not a resolver
  bug.

## Verification

- **Mock harness** (Chrome, port 1437), with real clicks:
  - `src/main.rs:3` opened `main.rs` at line 3, with `src/` expanded and the
    row selected.
  - `lib.rs` opened the menu, and picking `src/lib.rs` opened it.
  - `src/ghost.ts` stayed plain.
  - A path in the Plan tab switched the dock to Files.
  - The Files ↻ sent one `resolve_doc_paths` at the new generation, and a
    tagged link node survived (no rebuild).
- **`mdpaths-check.mjs`** went red for each of three breakages: mining link
  text, dropping batches past the first, not mining prose.
- **The Rust test** went red when index hits skipped `resolve_term_path`.
- **Gates on the PR:**

  | Gate | Result |
  | --- | --- |
  | `make test` | 478 tests, 0 `not ok` |
  | `make lint` | ok |
  | `cargo test -p worktrees-core` | 710 passed |
  | `cargo test -p worktrees-cli` | 70 passed |
  | `cargo test -p app --lib` | 162 passed |
  | `tsc --noEmit` | ok |
  | `cargo check -p app` | ok |
  | `make test-frontend` | 36/36 |

  CI was green.

## Follow-ups

These are review nits from the fable review plus one pre-existing issue, all
in ROADMAP:

1. `resolve_doc_path` stops at `DOC_NAME_CHOICES_MAX` before sorting. It
   should collect all matches, sort, truncate, and title the menu "12 of N".
2. `resolve_doc_paths` should reject an empty or relative `root`, because
   `git -C ""` means the current directory.
3. The mock hard-codes 64 and 12. It should import `DOC_PATHS_BATCH` and
   mirror the choices cap.
4. A failed batch is not retried until the next generation. The `mdpaths.ts`
   comment should say so.
5. The name index is rebuilt every time `placesToken` moves (roughly every
   30 s). If that ever matters, key it on the git index mtime.
6. Pre-existing: App passes inline `onOpen` arrows to Plan and Files, which
   breaks Markdown's memo and re-runs `block()` on big plans on every App
   render. Fix with `useCallback`.
