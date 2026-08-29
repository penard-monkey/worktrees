# Session: the app aborted when you selected text — macOS Writing Tools, not our code

- **Date:** 2026-08-29 (one session: crash forensics through merge)
- **Worktree:** `bug-fixes` (idle base `bug-fixes-next`)
- **Branches:** `bug-fixes-rename-tab-crash` — rebased onto a main that moved
  seven commits mid-session, deleted after merge
- **PRs:** [#175](https://github.com/penard-monkey/worktrees/pull/175)
- **Release:** none — rides the next one (`[Unreleased]` in CHANGELOG)
- **Planning files:** none (the session was short and evidence-led; the
  artifacts live in the scratch dir, listed under Verification)

## What shipped

**One crash fix, in `app/src-tauri/src/lib.rs`** (`run()`'s `.setup()`, ~60
lines including the explanation) plus `objc2 = "0.6"` as a macOS-only
dependency in `app/src-tauri/Cargo.toml`.

At startup the app walks the webview's class chain to wry's own `WryWebView`
class and adds one method to it — `allowsWritingToolsAffordance`, returning
`NO` — via `objc2::ffi::class_addMethod`. AppKit asks the view that question
before floating its Writing Tools affordance over a selection; answering NO
means the affordance never appears, and the assertion inside it can never
fire.

## The bug

Three SIGABRTs in `app.log` (08-27 ×2 on v0.17.0, 08-29 on v0.18.0), the last
one while renaming a terminal tab. Every one leaves the same trace and nothing
else:

```
[panic] panicked at library/core/src/panicking.rs:225:5:
panic in a function that cannot unwind
```

The crash report's faulting thread ends
`tao::…::app::send_event` → `panic_cannot_unwind` → `abort`, and the last
system-log line at the abort's own microsecond is:

```
[com.apple.AppKit:CampoLightweightUI] Mouse entered.
[com.apple.Foundation:general] *** Assertion failure in <private>, NSCampoLightweightUIController.m:1429
```

macOS 26 floats a Writing Tools affordance ("Campo" internally) over any
selection in the webview. Hovering it trips an assertion **inside AppKit**;
the NSException that assertion raises unwinds out through tao's `sendEvent:`
override, which is `extern "C"` and therefore nounwind, so Rust converts the
foreign unwind into an abort. Nothing of ours ran and nothing of ours was
wrong.

Not rename-specific, either: the rename box calls `select()` on focus, and a
drag across terminal output arms exactly the same thing. Work survived only
because the shells live in tmux.

## Decisions

- **Override the getter on wry's subclass, not the configuration.** The
  documented knob is `WKWebViewConfiguration.writingToolsBehavior`, which is
  settable only *before* the webview exists. This window is declared in
  `tauri.conf.json`, so the app never holds that configuration; using it would
  have meant moving window creation into Rust and re-declaring every attribute.
  `WKWebView` itself exposes `writingToolsBehavior` read-only — probing the
  live class confirmed the setter does not exist (`respondsToSelector:` → NO),
  which killed the first attempt outright.
- **Add the method to `WryWebView`, not to the KVO subclass and not to
  `WKWebView`.** The instance's class at setup time is
  `NSKVONotifying_…WryWebView…` (wry KVO-observes the document title); an
  override added there stops being reachable when the instance's isa reverts.
  wry's own class is stable, a later-created KVO subclass inherits the
  override, and no Apple class is touched.
- **Install once per process.** Every webview shares that one registered
  class, so a second window's `class_addMethod` would fail — the class
  implements the selector by then — and warn about a fix that is in place and
  working. A `std::sync::Once` keeps the log honest for whoever adds a second
  window.
- **Take the BOOL type encoding from objc2, don't hardcode it.** `BOOL`
  encodes as `B` on arm64 and `c` on x86_64. Nothing dispatches through the
  string, so the hardcoded `c` worked, but the registered signature was wrong
  on the machine it runs on.

## Dead ends / gotchas

- **A Rust panic that never happened.** `panic in a function that cannot
  unwind` with **no** panic line before it is the signature of a *foreign*
  (ObjC) exception crossing a nounwind boundary — not a Rust panic. The app's
  hook logs every real panic, so the missing line is the evidence, not a gap
  in logging. Chasing it as a Rust bug would have found nothing: the whole
  cause is in AppKit's frames.
- **The crash report has no exception info.** `asi` is just
  `abort() called`, and there is no `lastExceptionBacktrace`. The diagnosis
  came entirely from `log show --predicate 'process == "app"'` over the crash
  minute — the AppKit assertion is in the unified log, not in the `.ips`.
  Two categories carry it: `CampoLightweightUI` and `WritingTools`.
- **`log` is not a zsh builtin, but it argues like one.** `log show … 2>&1 |
  head` inside the tool's zsh died with `(eval):log:1: too many arguments`;
  `/usr/bin/log` works. Cost one confusing "no output" result that read as
  "nothing was logged".
- **The app's own log file is identifier-blind.** `APP_IDENT` is a `const`,
  so the sandbox build (`net.casadelvalle.worktrees.sbx`) writes into the
  *installed* app's `app.log`. Useful here — one file held both sides of the
  A/B — but it means a sandbox app is not as isolated as its identifier
  suggests.
- **The dev app dies at the turn boundary.** `tauri dev` launched as a
  background task was killed the moment the turn ended, mid-verification. A
  `python3 -c "os.setsid(); …"` wrapper detaches it properly and it survives.
  Same family as CLAUDE.md's note about `nohup … &` inside a tool call.
- **A stale `target/release/worktrees` is not the only stale-artifact trap.**
  Two `tauri dev` instances raced for port 1420 (`Port 1420 is already in
  use`) after a relaunch that did not kill the first — the same
  content-vs-port check CLAUDE.md documents for vite applies to the app.
- **Main moved seven commits mid-session.** #173, #174 and #176 landed while
  this was in review, and the PR went `CONFLICTING` on the one file three
  parallel sessions all touch: `CHANGELOG.md`'s `[Unreleased]`. Rebase,
  keep both sides' sections, re-run the gates on the new base.

## Verification

The mock harness cannot express any of this — the bug lives in AppKit's
event dispatch — so everything was measured on a real build via
`app/scripts/sandbox.sh --app`:

- **Before:** the installed 0.18.0 logged **286** `CampoLightweightUI` /
  `WritingTools` events while text was selected, reaching
  `Dwelling completed, show affordance` — the exact line 61ms before the
  fatal assertion in the crash.
- **After:** the sandbox build logged **0** in the same window, with
  `allowsWritingToolsAffordance=false` read back from the live view, and no
  crash when the affordance was hovered.
- After the review follow-ups, a fresh sandbox start logs **no**
  `writing-tools affordance` warning — i.e. `class_addMethod` succeeded with
  the computed encoding.
- Gates on the rebased base: bats **325/325** (`rc=0`, zero `not ok` in the
  full log), `make lint` clean, core **267**, cli **7**, `app --lib` **45**,
  `tsc --noEmit` + `cargo check -p app` clean. CI green on both OSes.

Artifacts in `~/.cache/worktrees/worktrees/bug-fixes/`:
`app-2026-08-29-185407.ips` (the crash report),
`2026-08-29-crash-window-syslog.log` (the AppKit assertion in context),
`2026-08-29-campo-baseline-installed-app.log` (the before side of the A/B),
`2026-08-29-bats-after-rebase.log`.

## Follow-ups

- The override is a workaround for an OS bug. When Apple fixes the assertion,
  or when wry/tauri exposes `WKWebViewConfiguration.writingToolsBehavior`,
  this can become one config line — or nothing. Tracked in ROADMAP.
- Worth knowing but not acted on: `WritingToolsController::willBeginWritingToolsSession
  () => attributed string is empty` appears just before the assertion, so the
  assertion may be specifically about an empty selection payload — i.e. other
  WKWebView apps that select non-text regions may hit it too.
