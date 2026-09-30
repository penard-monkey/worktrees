---
title: "Session — NotAllowedError toast"
---

# Session — the NotAllowedError toast: every "Copy …" goes through pbcopy

- **Date:** 2026-09-29 → 2026-09-30
- **Worktree:** `.worktrees/notallowed-error`
- **Branch:** `notallowed-error`
- **PR:** [#372](https://github.com/penard-monkey/worktrees/pull/372) (squash `1900e93`)
- **Release tag:** none (the fix is under `## [Unreleased]`)
- **Planning files:** `planning.tar.gz` (the lane's brief only; no task_plan/findings/progress)

## What David saw

The error banner showed WebKit's raw `NotAllowedError: The request is not allowed
by the user agent or the platform in the current context, possibly because the user
denied permission.` It is in `app.log` three times: 2026-09-27 01:09:41Z and
01:09:49Z (8 s apart, which looks like one copy clicked twice), and 2026-09-30
00:39:25Z.

## Cause

- The only gated web API in `app/src` was `navigator.clipboard.writeText`, called
  from six "Copy …" click handlers. The app has no Notification, media playback,
  fullscreen, `window.open` or `showPicker` calls, and xterm has no clipboard addon.
- `writeText` needs the click's transient user activation. **On macOS 27.0, WebKit
  clears that activation on every `evaluateJavaScript:` call.** The regression dates
  from WebKit 265168@main and was fixed in 321448@main (2026-09-19), which is not in
  27.0 (26A428). The same thing was reported against Wails as wailsapp/wails#6170.
- Tauri 2.11.5 delivers every `emit`, and every `Channel` message under 1 KB raw or
  8 KB JSON, through `webview.eval` (`tauri/src/ipc/channel.rs:156-165`). That
  includes each chunk of terminal output. While a session is printing, one of these
  lands between mousedown and click almost every time, so the click handler runs
  with `navigator.userActivation.isActive === false` and the write is refused.
- The `drafts:` lines next to the errors in the log, which the brief treated as the
  lead, were a coincidence. `scan_drafts` only reads unsent prompts for display, and
  those lines change all the time.

## What shipped (#372)

- `app/src/clipboard.ts::copyToClipboard` is now the only way the app writes to the
  clipboard. It calls the backend command `copy_text` first. The web API is used only
  as a fallback, and if both fail the error carries both reasons.
- `copy_text` (`app/src-tauri/src/lib.rs`) runs `/usr/bin/pbcopy` on
  `spawn_blocking`. It removes `LC_ALL` from pbcopy's environment and sets
  `LC_CTYPE=UTF-8`, and it logs a failure as `warn` before returning it. On
  non-macOS it returns an error, so the frontend falls back to the web API.
- All six call sites were switched: App, FilesPane, AutomationsPane, McpPanel, and
  SettingsSheet ×2. Copy diagnostics was broken for a second reason as well: it
  awaited `invoke("diagnostics")` before writing.
- The mock records copies: `case "copy_text"`, `__mock.copies()`.
- `app/scripts/clipboard-check.mjs` fails on any clipboard API use outside
  `clipboard.ts` (a `clipboard` property or string, `writeText`, `readText`), and
  checks that the helper calls the native command first and that both the command
  and the mock exist.

## Decisions

- **Native first, not a web-first retry.** A retry loses the same race as long as
  output keeps arriving. The native path needs no user gesture, so it behaves the
  same way every time.
- **`pbcopy`, not NSPasteboard through objc2.** It follows the repo's practice of
  shelling out, and it avoids the main-thread requirement behind the crash reported
  in tauri-apps/plugins-workspace#3205.
- **`LC_CTYPE=UTF-8` and removing `LC_ALL`.** With no locale set, `año/✎` came out as
  `a√±o/‚úé` (measured). `fixup_gui_locale` already sets `LANG`, but it leaves a
  non-UTF-8 `LC_ALL` (for example `C` from `launchctl setenv`) alone when `LANG` is
  UTF-8, and `LC_ALL` overrides both.

## Dead ends / gotchas

- **The brief's lead was the drafts timing, and it did not hold.** Read what the
  correlated log line does before treating it as a trigger.
- **A click handler can lack activation on macOS 27 even though the click is real.**
  Any API gated on user activation is affected: `window.open`, `showPicker`,
  fullscreen, WebAuthn, and the clipboard. If the app adds one of these, it needs a
  native path as well.
- **In a `sed` restore, check that the pattern is unique.** Restoring a line that had
  been removed to show a test fail also rewrote the function's trailing `c`, which
  had the same shape. `diff` against a saved copy is safer.
- **The global `pnpm` shim on this machine is broken.** The corepack 12.8.1 directory
  has no `bin/pnpm.cjs`. Running `node ~/.cache/node/corepack/v1/pnpm/11.5.2/bin/pnpm.cjs
  install --frozen-lockfile` under Node 22.19 worked. Node 22.23.2, which AGENTS.md
  names, is not installed; `.nvmrc` says 22.13.0.

## Verification

- A bare WKWebView repro, built with swiftc and driven by synthesized NSEvents, is in
  `~/.cache/worktrees/worktrees/notallowed-error/repro.swift`. Results: a plain click
  copied (`active=true`). The same click with one `evaluateJavaScript("void 0")`
  between mousedown and mouseup gave the exact banner text (`active=false`). A third
  plain click copied again. The clipboard was restored after each probe.
- Tests shown failing first:
  - `clipboard-check.mjs` failed 6 of 6 checks on the tree before the fix.
  - `navigator["clipboard"]` and an aliased `navigator` both passed the first version
    of the check and fail the second.
  - `pbcopy_is_told_its_input_is_utf8` failed with each environment line removed.
- Gates: bats 1..388 with 0 `not ok`; lint ok; core 545; cli 28; app 163 plus 1
  ignored; tsc ok; `cargo check -p app` ok (one dead-code warning in `viewer.rs`
  that was already there); frontend checks 24 of 24. CI passed 9 of 9 on both
  pushes.
- **Not tried in the real app.** David's running app was not driven, and the sandbox
  app was not used.

## Follow-ups

These are in ROADMAP.

- Hand-test Copy path in the real app while a session is streaming.
- Once a macOS with WebKit 321448@main is the minimum, decide whether the native path
  should stay.
