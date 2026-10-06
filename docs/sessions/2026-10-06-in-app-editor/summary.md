# in-app-editor — the Editor button failed silently; an editor picker; an in-app editor proposed and parked

- **Date:** 2026-10-06
- **Place:** `in-app-editor` (branches `in-app-editor-cmd-errors`, `in-app-editor-proposal`, `in-app-editor-close-out`)
- **PRs:**
  - [#443](https://github.com/penard-monkey/worktrees/pull/443), squash-merged as `3d48e06` after one review round
  - [#444](https://github.com/penard-monkey/worktrees/pull/444), the proposal, **closed unmerged** (parked by David). Its branch `in-app-editor-proposal` is deliberately kept on GitHub
- **Release:** none (under `## [Unreleased]`)
- **Planning files:** `planning.tar.gz` beside this file (`task_plan.md`, `findings.md` with every measurement, `progress.md`)

## The ask

David: "I can't edit a file. I opened up a file and tried to click Editor but it doesn't work." He wanted a full in-app editor, editing as the default, and New file from a right-click in the Files tree.

## What shipped (#443)

- **`launch_watched`** (`app/src-tauri/src/lib.rs`), shared by `open_editor` and `open_terminal`. Both used to spawn `/bin/sh -c <cmd>` and reap it blind. David's `editor_cmd` is the default `code`, and VS Code is not installed, so sh exited 127 and nothing was shown.
  - It waits up to `LAUNCH_GRACE` (1.5 s). A non-zero exit inside that window becomes an `Err`, shown through the existing `fail()` path:
    - 127 → "Editor command `code` was not found — choose an installed editor in Settings → Commands". The terminal command says "install it, or change it".
    - 126 → "not executable".
    - otherwise the last stderr line.
  - A command still running at the deadline counts as launched (`code --wait`, foreground terminals).
  - stderr is drained for the child's whole life, so a long-lived launcher cannot block on a full pipe; the comment says the thread can outlive the click by an editor's whole session. stdin and stdout go to `/dev/null`. The wait runs on `spawn_blocking`.
- **Installed-editor picker** (`app/src-tauri/src/editors.rs`, `detect_editors`; Settings → Commands in `SettingsSheet.tsx`). Added at (main)'s request.
  - **Detection**: one `stat` per known bundle name in `/Applications`, `~/Applications` and `~/Applications/JetBrains Toolbox`, plus known launchers on `PATH`, which `fixup_gui_path` has already made the login shell's. The `KNOWN` table covers VS Code, Cursor, Zed, Windsurf, Sublime, Nova, BBEdit, TextMate, Xcode, the JetBrains IDEs, DataGrip and Android Studio.
  - **Commands**: an app entry runs `open -a "<bundle>"`. CLI entries are listed after the apps, one per product. The first bundle name in `KNOWN` wins, and launchers are judged through symlinks.
  - **UI**: a `<select>` plus **Custom…**. `editor_cmd` stays the single stored value, and a stored command that matches nothing reads as Custom.
  - **Mock**: `?editors=none|one|many`.
- **CHANGELOG**: one `### Added` and one `### Fixed`. **ROADMAP**: a flaky-test entry (below).

## Decisions

- **Grace rule.** Only a non-zero exit inside 1.5 s is an error. Past that point "working" and "slow" can't be told apart, and holding the click open would be worse than missing a late failure. GUI launchers either hand off or fail within a few hundred ms.
- **`open -a` for apps, a CLI only where it is on PATH.** `open -a` works for any bundle with no shim. A CLI is offered beside it because `code <dir>` opens a folder as a project.
- **Terminal picker skipped.** `terminal_cmd` must run `tmux attach -t {session}` inside the new window. Terminal and iTerm need AppleScript quoting, and Ghostty, Kitty, WezTerm and Alacritty each need a verified `-e` form plus tmux on that window's PATH. None of that can be verified without launching apps on David's machine.
- **No `*-check.mjs` for the picker.** No frontend list mirrors a backend one; the frontend only renders what `detect_editors` returns.
- **The in-app editor is parked** (David's call). The proposal recommended CodeMirror 6: five core packages, no Lezer grammars, highlighting from `highlight.ts`, a dynamic import behind a boundary check. It would be a third "no UI libraries" exception. §9's five questions are unanswered:
  1. the library exception;
  2. whether a read-only mode stays;
  3. ⌘S or autosave;
  4. writes to files outside the place;
  5. when rename and delete land.

## Dead ends / gotchas

- **The ask assumed a read-only dock. It is not.** #56 and #306 already made a markdown file's Source view an editable `<textarea>`, with ⌘S, a draft store and a compare-and-swap on mtime. The FilesPane header records why editing stopped at markdown: a textarea can show no highlighting, no line numbers and no Custom Highlight API find painting. "Editor" in the header was always the EXTERNAL command.
- **Three latent bugs in that existing markdown editor**, recorded in ROADMAP:
  - **CRLF → LF on save.** A textarea normalises line breaks; measured in WebKit, `"a\r\nb\r\n"` reads back as `"a\nb\n"`.
  - **Non-UTF-8 corrupted on save.** `read_file` decodes with `from_utf8_lossy`.
  - **A fixed temp name** (`.{name}.wt-tmp`), so two saves of one file race. The dock and the reading overlay are both mounted.
- **Review must-fix: a typing field that unmounted itself.** The Custom field showed either because Custom was chosen or because the stored value matched nothing. Typing text equal to a detected entry's command made it "known" again and unmounted the focused input mid-word. Fixed by having typing call `setEditorCustom(true)`.
  - The guard is a Playwright drive that types one key at a time past a match: red in WebKit and Chromium, then green.
  - It lives in scratch, not in the repo, because the repo has no Playwright dependency.
- **Review nice-to-have: two behaviours no test pinned.** Replacing `.find` with `.rfind` (last edition wins) and `metadata` with `symlink_metadata` (links not followed) both passed the first fixtures. A new fixture now turns each red.
- **CI flake, not ours.** The first CI run for #443 failed rust (ubuntu) on core `codex_usage::tests::fake_cli_auth_classification_and_rpc_failure_do_not_invent_logout` ("unavailable" vs "unsupported_auth"). The branch touched no core code, and the next run and local runs passed. It is in ROADMAP.
- **HMR is dead inside `.worktrees/` (AGENTS.md), and it bit once.** After the must-fix edit, the harness served the old file. Restarting with `--force` and grepping the served source fixed it. Grep for CODE, not comments.
- A backup written to `$TMPDIR` rather than the scratchpad caught one restore out. Check where a backup landed before restoring from it.

## Verification

- **Unit tests**, all shown failing first:
  - `launch_watched`: 4 tests, 3 of them red on the old spawn-and-reap shape. The long-runner case passes either way, by design.
  - `editors::tests`: 3 tests, red on a stub and on the two mutations above.
- **Picker in headless Playwright WebKit and Chromium** against the mock, `?editors=many|one|none`:
  - options listed;
  - a pick writes its command;
  - Custom… reveals the field;
  - an undetected `code` reads as Custom;
  - `elementFromPoint` hits the select;
  - the typing guard stays green.
- **Local gates on the final tip**:
  - `make test` 1..476, 0 not ok; `make lint` ok;
  - core 707 passed (1 ignored), cli 70, `app --lib` 161;
  - tsc, `cargo check -p app` (one pre-existing `viewer.rs` dead-code warning) and `make test-frontend` ok.
- **CI**: run 37467030248 green on all 9 jobs.
- **Proposal measurements** (esbuild `--minify`; scratch in `~/.cache/worktrees/worktrees/in-app-editor/bundle`). The app itself is 968 KB raw / 287 KB gz.

  | Option | Raw | Gzip | Notes |
  |---|---|---|---|
  | CodeMirror 6 core | 289 KB | 94 KB | |
  | + search and language | 324 KB | 105 KB | the recommended tier |
  | + six Lezer grammars | 684 KB | 238 KB | |
  | Monaco core, no languages | 2.72 MB | 701 KB | plus a 307 KB worker |

  CM6 in WebKit: a 3.66 MB, 60k-line document mounts in 22 ms with 67 DOM lines. `highlight.ts` tokenises 1 MB in 38–76 ms.

## Follow-ups

- **ROADMAP**: the parked in-app editor (#444 / `in-app-editor-proposal`, §9 unanswered).
- **ROADMAP**: the three markdown-editor bugs, which are real today whatever happens to the editor.
- **ROADMAP**: the `codex_usage` CI flake (added in #443).
- **Not tracked**: a terminal-command picker, as its own small PR, hand-tested. Mentioned only, since nobody asked for it.
