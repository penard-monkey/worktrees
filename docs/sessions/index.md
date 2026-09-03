---
title: Development log
---

# Development log

One archive per working session. Each summary records what shipped, the
decisions and their reasoning, and — the load-bearing part — the dead ends:
what was tried, what failed, and why. They are written for whoever picks the
work up next, which is usually a fresh agent with no memory of the session.

Each session's planning files (`task_plan.md`, `findings.md`, `progress.md`)
are archived beside its summary as `planning.tar.gz` in the repository.

| Date | Session |
|---|---|
| [2026-09-02](2026-09-02-agent-brief/summary.html) | agents — brief, `--name`, agent state in MCP; strays in `ls`/doctor |
| [2026-08-30](2026-08-30-status-inline/summary.html) | a worktree with no session now opens onto its own status check instead of one line and a button, and Claude's read is rendered rather than printed — one `StatusBody` shared by both hosts so they cannot drift; the menu item steps aside only when the check is already on screen for *that* place, because right-click does not select; and four defects that every unit gate passed — a recessed box with zero contrast against the surface it sits on (invisible outright in tokyo-day), a shared class quietly restyling the dock's empty states, 32px of padding lost to `align-items: stretch`, and a code fence sized in absolute units painting bigger than the prose around it |
| [2026-08-28](2026-08-28-sync-doctor-heal/summary.html) | eight of ten worktrees were silently carrying walls of `deleted:` tarballs — the #146 damage class, invisible between syncs because the heal was pull-only; 172 files healed by hand, then `doctor` learned to name the damage (suppressed in hub copies, where absence is the pushed state and the remedy is refused), the heal moved to every sync edge including a pull whose transfer failed, and `.worktrees-sync/` joined the `info/exclude` entries |
| [2026-08-29](2026-08-29-writing-tools-crash/summary.html) | the app aborted whenever you selected text — macOS 26's Writing Tools affordance trips an assertion inside AppKit, and that NSException unwinds through tao's `extern "C"` `sendEvent:`, so Rust turns a foreign unwind into SIGABRT; the tell is `panic in a function that cannot unwind` with **no** panic line before it, and the diagnosis lives in the unified log, not the crash report |
| [2026-08-29](2026-08-29-status-check/summary.html) | clicking a worktree stopped spawning sessions (the root cause of "everything reads active"), the header stopped saying things twice, ↓ became main-only, stale rows dim, and right-click grew a status check — a verdict computed once in core, with an Ask Claude button that is the repo's first headless spawn; four stacked PRs, an adversarial spec pass that caught a guard built on an invented fact, and a merge train whose only conflicts were three parallel `[Unreleased]` sections |
| [2026-08-29](2026-08-29-app-zoom/summary.html) | ⌘+/⌘− became an overall-size knob that finally reaches the terminal and its tmux pane — WKWebView page zoom, because `--ui-rem` deliberately never touched the xterm grid; and ⌥ chords cannot be matched on `e.key`, since macOS composes ⌥- into an en dash |
| [2026-08-18](2026-08-18-remove-dialog-menu-clamp/summary.html) | removing a worktree became a dialog that says what it will destroy, and a menu near the bottom edge stopped hiding its last item — a clamp keyed on frozen cursor coords, and `--force` turning out to mean `git branch -D` as well |
| [2026-08-18](2026-08-18-side-by-side-diff/summary.html) | the Files tab shows what changed, not just what is there — git's full-context diff in two aligned columns, word-level marks, and a changed-files-only tree; `git -C ""` means *here*, and a sticky cell tinted over `transparent` is see-through |
| [2026-08-18](2026-08-18-new-worktree-dialog/summary.html) | new worktree became a dialog that says what it will do — the branch picker gained a ▾ and got shared, and the verdict line had to learn `cmd_new`'s ordering, which is not the obvious one |
| [2026-08-18](2026-08-18-terminal-dock-overlap/summary.html) | the terminal ran on under the Files dock — a row flex item with no `min-width: 0` is floored at xterm's painted grid, so the pane could grow and never shrink, and the resize observer watched the box that could not move |
| [2026-08-17](2026-08-17-gitignore-cmdt-replay/summary.html) | new projects start with a clean `git status`, ⌘T + a files right-click menu, and the shell tab that came back four prompts deep — the ring replays fine at its own width; one column off is what stacks it |
| [2026-08-17](2026-08-17-parked-busy-dot/summary.html) | a parked job pinned the green dot on — the probe file keeps a mid-flight `busy` when a turn is parked, and nothing ever writes it again |
| [2026-08-17](2026-08-17-emoji-width/summary.html) | emoji stopped shredding tmux output — tmux said 2 cells, xterm said 1; the graphemes addon aligns them, and the Node width probe lies |
| [2026-08-16](2026-08-16-sync-macs/summary.html) | `worktrees sync` — courier sync between Macs on an SSD, plan to six merged PRs: CLI parity, guards on every surface, the app's modal + progress bar + import, and the heal for tracked files the transfer skips |
| [2026-08-14](2026-08-14-drag-drop-nav/summary.html) | drag a place into another group to put it there — a drop sets the tier, the gap opens where it will really land, projects reorder too |
| [2026-08-14](2026-08-14-cmd-f-find/summary.html) | ⌘F finds — the xterm buffer in either terminal, the open file in the viewer, one bar routed by what you last touched |
| [2026-08-14](2026-08-14-markdown-zoom/summary.html) | markdown docs read at whatever size you need — a per-place reading zoom for the Files viewer, one multiplier and a rem→em sweep |
| [2026-08-14](2026-08-14-terminal-tab-memory/summary.html) | terminals come back as you left them — the tab strip, the front tab and each tab's directory all survive a restart → v0.13.0 |
| [2026-08-13](2026-08-13-codesign-privacy-prompts/summary.html) | macOS privacy prompts and what signing them would cost — ad-hoc signing keys TCC to a cdhash; MAS ruled out, Developer ID scoped |
| [2026-08-11](2026-08-11-cmdk-activity-order/summary.html) | one clock for every list of places — ⌘K, Recent and Resume ranked three different ways, now all on the nav's activity date |
| [2026-08-11](2026-08-11-v0-12-releases/summary.html) | the sandbox found what the mock harness could not — dock default, nav refresh race → v0.12.0, v0.12.1 |
| [2026-08-11](2026-08-11-files-changed-markers/summary.html) | the Files tab says what the branch changed — tinted names, cascading counts, ghost rows for deletions |
| [2026-08-11](2026-08-11-space-workbench/summary.html) | a space owns its whole workbench — header over terminal+dock, per-place panel memory, nameable places |
| [2026-08-10](2026-08-10-files-tab-visibility/summary.html) | the Files tab stops hiding things silently — gitignored by default, symlinks marked → v0.11.0 |
| [2026-08-10](2026-08-10-global-skills-repo/summary.html) | close-out leaves the repo — a personal skills repo at `~/workspace/claude-skills` |
| [2026-08-10](2026-08-10-nav-activity-age/summary.html) | nav clock + order track activity, not attention → v0.11.0 |
| [2026-08-09](2026-08-09-afterglow-dot/summary.html) | the afterglow dot — a place stays lit after Claude finishes → v0.10.0 |
| [2026-08-09](2026-08-09-files-tab-refresh/summary.html) | the Files tab shows files as they are created → v0.10.0 |
| [2026-08-07](2026-08-07-power-consumption/summary.html) | the app stopped burning power in the background → v0.9.1 |
| [2026-08-07](2026-08-07-relnotes-inline-markdown/summary.html) | release notes render their own markdown |
| [2026-08-07](2026-08-07-new-place-feedback/summary.html) | creating a place says so, reopening one stops shouting |
| [2026-08-06](2026-08-06-nav-hierarchy/summary.html) | nav hierarchy — projects up, dormant down |
| [2026-08-06](2026-08-06-empty-project-onboarding/summary.html) | empty-project onboarding — git init + first commit |
| [2026-08-06](2026-08-06-ai-rules-layer/summary.html) | AI profiles — per-project Claude rules, skills, MCP and model |
| [2026-08-06](2026-08-06-usage-countdown/summary.html) | time-until-reset on the usage bars |
| [2026-08-05](2026-08-05-undeclared-drift/summary.html) | undeclared-file drift + ADR 0001 |
| [2026-08-03](2026-08-03-files-viewer/summary.html) | dock Files tab — document rendering + layout |
| [2026-08-02](2026-08-02-usage-widget/summary.html) | Claude plan-usage widget → v0.7.0 |
| [2026-08-02](2026-08-02-remove-place-delbranch/summary.html) | remove was broken for every place (`delBranch`) + stale "Not configured" banner |
| [2026-08-01](2026-08-01-ui-tweaks/summary.html) | UI batch — named terminal tabs, settings categories, guide-line toggle, log tail (+ v0.6.0 release) |
| [2026-08-01](2026-08-01-single-pane-new/summary.html) | app-created places open single-pane |
| [2026-07-30](2026-07-30-tmux-gate/summary.html) | tmux install gate + missing-tmux banner |
| [2026-07-29](2026-07-29-right-panel/summary.html) | right panel, icon set, owned dock shells, nav fixes → v0.5.0 |
| [2026-07-28](2026-07-28-right-dock/summary.html) | right dock — file browser, editable viewer, embedded terminals |
| [2026-07-28](2026-07-28-project-settings/summary.html) | Session — per-project settings (`.worktrees.toml`) |
| [2026-07-27](2026-07-27-settings-and-audits/summary.html) | settings evaluation, nav audits, ⌘K, v0.3.0 + v0.3.1 |
| [2026-07-27](2026-07-27-release-notes-ui/summary.html) | release-notes UI — 2026-07-27 |
| [2026-07-27](2026-07-27-readme-media/summary.html) | README desktop-app media — session summary |
| [2026-07-27](2026-07-27-close-out-ritual/summary.html) | close-out ritual + session archive system |
| [2026-07-27](2026-07-27-busy-dots-nav-home/summary.html) | busy-only dots, nav with Home, system theme pairs |
| [2026-07-26](2026-07-26-themes-v0.2.4/summary.html) | light mode + theme gallery → v0.2.4 |
