# Workspace view engine review

Reviewed `162512b` on `review/workspace-view-codex` against revision 3 of the workspace chat proposal, the combined workspace view design, the Claude measurements and the Unreleased changelog. Sizes are relative to this branch. Small is a local change with a regression check. Medium crosses a few modules. Large changes ownership or isolation.

## Breaks or loses data

### Leaving during a save can drop the next edit

**Breaks or loses data. Proved at the session layer.** `app/src/doc/ideasave.ts:148`, `app/src/doc/ideasave.ts:173`, `app/src/doc/useDoc.ts:177`, `app/src/doc/useDoc.ts:189`.

Type again while the write started by Leave is running. Leave returns true after that write even when the session is still dirty. A subsequent opening can close that session and clear the only timer that would save the latest text. I held the write promise in the real TypeScript session, typed again, released the write, evaluated Leave's predicate and closed the session. Only the first text reached the scripted disk. The existing check issues Flush after a write is already in flight, so it sets `again` and exercises a different ordering (`app/scripts/ideas-save-check.mjs:213`). Clicking a place ordinarily keeps the pane mounted and the timer alive. Changing documents and closing the old session is the loss case.

**Fix (medium).** Make Leave drain saves until the current opening is clean or returns a refusal. Guard the handoff with the opening's identity and edit generation. Add a deferred-write test that types during the write started by Leave.

### Two editors can overwrite each other with equal millisecond mtimes

**Breaks or loses data. Proved for event handling, read for the disk write.** `crates/worktrees-core/src/memory.rs:305`, `crates/worktrees-core/src/memory.rs:843`, `app/src/doc/ideasave.ts:193`, `app/src/doc/memory.ts:125`.

A change with the same millisecond mtime as the open copy is silently ignored. I delivered a change event with that timestamp and different scripted disk text. The real session made zero reads. On the Rust side, the save accepts the same timestamp even when the bytes on disk differ from the editor's base. Two app windows, or the pane and an external writer, can lose the first writer's text if writes share a timestamp or an editor preserves it. The watcher already uses size when sampling, but drops it from the event, and equal-size edits defeat that detection too.

**Fix (medium).** Use an opaque revision over the loaded bytes for optimistic writes. Carry it through the file event, or reread on an ambiguous event. Test equal timestamps with both equal and unequal sizes.

### Keep mine can overwrite text that never made it into History

**Breaks or loses data. Read.** `crates/worktrees-core/src/memory.rs:604`, `crates/worktrees-core/src/memory.rs:865`, `crates/worktrees-core/src/memory.rs:937`.

An untracked Markdown file can be visible and editable in the pane while Git ignores it. If someone edits it and the person chooses Keep mine, `git status` returns no entry for the path, the history step returns success with no commit, and the app replaces the other text. The same path also commits the whole existing index if it was already staged, despite asking Git to add only this file.

**Fix (medium).** Preserve the exact replaced bytes independently of ignore rules and the ambient index. Confirm that preservation succeeded before replacing them. Test ignored drafts and an unrelated staged file.

### A fold can commit successfully and lose its receipt on restart

**Breaks or loses data. Read.** `crates/worktrees-api/src/workspacefolds.rs:261`, `crates/worktrees-api/src/workspacefolds.rs:535`, `crates/worktrees-api/src/workspacefolds.rs:563`, `crates/worktrees-api/src/workspacefolds.rs:1071`.

The Git commit happens before the receipt is stored. If serve dies in that gap, restart sees `folding` and the newer note. A clean tree produces no new commit. The receipt can then say there was nothing to commit and lose its direct Undo, even though History has a real fold. If other edits land first, the retry can record those as the fold instead. A receipt write error also allows the pane to close and the hold to release. The current restart test stops before the Git commit and lets the second engine finish before the first engine polls again.

**Fix (medium).** Persist a fold attempt ID and baseline before relaunch, include the attempt ID in the memory commit, and use it to repair the receipt idempotently before closing. Inject failures immediately before and after the commit, the receipt write and the tmux close.

### Project privacy and membership changes leave old chat permissions in force

**Breaks or loses data. Read.** `crates/worktrees-core/src/workspacechats.rs:588`, `crates/worktrees-core/src/workspacechats.rs:862`, `crates/worktrees-api/src/workspacechats.rs:108`, `crates/worktrees-api/src/workspaces.rs:164`, `crates/worktrees-api/src/ops.rs:560`.

Making a project private or removing it from a workspace changes what a fresh chat would be allowed to read. The running chat keeps its launch rules, while the list reports from the current workspace and registry. The fold relaunch rebuilds settings but keeps the old context file. Loss of registry data also makes the launch use an empty default, dropping explicit privacy denies (`crates/worktrees-core/src/registry.rs:163`). The measurements show that Claude auto mode can read paths without an explicit rule and that its file-tool rules do not confine subprocesses.

**Fix (large).** Treat permissions as a versioned launch snapshot. Report the active version and stop or require relaunch when access narrows. Fail closed on unreadable policy. Provide an OS-enforced filesystem boundary if the product promises private reads or project write protection with shell tools enabled. Until then, state the actual limits in the chat UI.

## Wrong but recoverable

### The memory hold does not serialize folds with Undo, the poll, or a second engine

**Wrong but recoverable. Read.** `crates/worktrees-api/src/workspacechats.rs:37`, `crates/worktrees-api/src/workspacefolds.rs:319`, `crates/worktrees-api/src/workspacefolds.rs:340`, `crates/worktrees-api/src/memory.rs:225`, `crates/worktrees-api/src/memory.rs:369`, `crates/worktrees-core/src/memory.rs:620`.

Undo can pass its hold check before a fold takes the hold and run after the fold has begun. The Undo row can also run before the first poll has recovered holds after a restart. The quiet poll similarly selects a commit before taking the Git lock. The process-local lifecycle lock and running-fold map also allow two engines using the same folder to start separate folds. Git serializes individual repository writes. It can still commit one chat's changes under another's receipt or put a pending fold's changes in an Undo baseline.

**Fix (large).** Put the fold reservation, readiness recheck and Git operations under one cross-process workspace lock with a durable active attempt. Keep lock ordering explicit. Test competing folds and interleave Undo and quiet commits with acquisition of that reservation.

### An unreadable record can end fold recovery for the engine's lifetime

**Wrong but recoverable. Read.** `crates/worktrees-api/src/workspacefolds.rs:261`, `crates/worktrees-api/src/workspacefolds.rs:504`, `crates/worktrees-core/src/desksessions.rs:130`.

A failed startup read completes the one-shot recovery anyway. A later successful read does not restore the fold or its hold. A transient chat-record failure during a fold is also treated as though someone already finished it. Repairing the file leaves a persisted `folding` state without a run, and the quiet poll can commit the partial work. The read path has clear errors for malformed, oversized or unreadable records. Recovery currently drops those distinctions.

**Fix (medium).** Distinguish a definite absence or completed attempt from a failed read. Retain the reservation while the state is unknown, surface the error, and retry per workspace.

### Failed workspace removal can leave a hold with no running fold

**Wrong but recoverable. Read.** `crates/worktrees-api/src/workspacechats.rs:274`, `crates/worktrees-api/src/workspacechats.rs:286`, `crates/worktrees-api/src/workspaces.rs:146`, `crates/worktrees-api/src/workspaces.rs:153`.

With an active fold, the removal path can lose access to the chat record and continue with an empty list. It then forgets the run without calling the stop path that releases its hold. If retiring the workspace folder fails as well, the workspace stays registered, but the memory is still held. Even if both filesystem errors are fixed, the quiet poll and Undo stay blocked until a restart.

**Fix (small).** Couple removal of a run and release of its hold. After a failed retirement, reconcile the surviving folder and record before returning. Test an unreadable record plus a failed move.

### A partial note can hold memory indefinitely or land after an interrupted fold

**Wrong but recoverable. Read.** `crates/worktrees-api/src/workspacefolds.rs:156`, `crates/worktrees-api/src/workspacefolds.rs:525`, `crates/worktrees-core/src/workspacechats.rs:782`, `crates/worktrees-api/src/workspacefolds.rs:829`.

The template asks Claude to write the note first and update the index next. After the note exists, a still-busy or still-starting session has no timeout. A session that exits after the note but before the index can land after two absent samples. Polling uses an empty pane list on a tmux read error, so persistent read failures can be mistaken for absence (`crates/worktrees-api/src/host.rs:289`). The current clock test explicitly accepts a busy fold with a note at 15 minutes.

**Fix (medium).** Keep a deadline for active work after the note and pause it for actual prompts. Require a completion witness after the note and index writes. Treat a failed pane sample as unknown. Offer to keep or retry partial work when the session really is gone.

### A second fold can accept an old no-op response or a future-dated note

**Wrong but recoverable. Read.** `crates/worktrees-core/src/workspacechats.rs:760`, `crates/worktrees-core/src/workspacechats.rs:824`, `crates/worktrees-api/src/workspacefolds.rs:174`, `crates/worktrees-api/src/workspacefolds.rs:475`.

Refolding reuses the same opener. The transcript test compares seconds, even though the attempt starts in milliseconds, and accepts its sentinel on any reply line. An old response in the same second can qualify while the new opener has yet to appear. A note already on disk with an mtime in the future also qualifies without any change to the file. The old-note test uses an equal timestamp and misses this case.

**Fix (medium).** Match the exact attempt in the transcript, require the sentinel at the end of its reply, and compare against the pre-fold note revision as well as observing new work. Cover a preexisting future timestamp and two attempts in one second.

### Mutable workspace files are treated as engine state

**Wrong but recoverable. Read.** `crates/worktrees-core/src/workspacechats.rs:87`, `crates/worktrees-core/src/workspacechats.rs:242`, `crates/worktrees-core/src/workspacechats.rs:345`, `crates/worktrees-core/src/workspacechats.rs:817`, `crates/worktrees-core/src/desksessions.rs:151`, `crates/worktrees-core/src/desksessions.rs:284`.

Every chat starts in the folder holding the shared record and the instruction files. The launch asks Claude to keep its writes elsewhere. The reader validates a chat ULID and the shape of Claude's UUID, then trusts the relationship, the start date and fold state. Ordinary record corruption can move a chat's derived note path, hide the right transcript or strand a fold. Atomic writes protect against partial engine rewrites. They cannot establish provenance for files another process is allowed to change.

**Fix (large).** Put records, locks and engine instructions under an independently protected root. Verify chat-to-transcript ownership against the launch record. Keep the writable workspace content separate.

### Git inputs and memory paths lack one consistent containment boundary

**Wrong but recoverable. Read.** `crates/worktrees-core/src/memory.rs:254`, `crates/worktrees-core/src/memory.rs:277`, `crates/worktrees-core/src/memory.rs:411`, `crates/worktrees-core/src/memory.rs:533`, `crates/worktrees-core/src/memory.rs:620`.

The memory APIs derive a lexical path from the workspace ID. The list and watcher walk that path, text reads canonicalize the supplied memory root, and Git does its own final-directory checks. The API therefore does not have one pinned workspace and memory root across reads, writes and Git. The memory lock also lacks the no-follow check used for the chat record lock. Git reads config from the writable metadata directory and disables recognized directives before invoking later commands. That is a configuration snapshot taken separately from the commands that consume it. This review did not test execution or repository escape.

**Fix (large).** Keep Git metadata and locks in an engine-owned location. Validate root identity once and use directory-relative file operations and a controlled Git configuration. Add benign fixtures for replaced roots, locks and config, with writes and reads confined to the fixture.

### Git stages files that the memory scan omits

**Wrong but recoverable. Read.** `crates/worktrees-core/src/memory.rs:353`, `crates/worktrees-core/src/memory.rs:369`, `crates/worktrees-core/src/memory.rs:383`, `crates/worktrees-core/src/memory.rs:595`, `crates/worktrees-core/src/memory.rs:942`.

The walk skips dot names and stops at the entry and depth limits. Git stages the whole tree with exclusions for oversized files and nested repositories the walk happened to find. For example, an oversized file beyond the scan limit can be considered by Git but absent from the listing and stamp. The initial repo excludes a couple of specific filenames, so skipping a dot name in the scan does not generally exclude it from Git. A truncated walk is also interpreted as a complete stamp and can report false removals when the sampled set moves.

**Fix (medium).** Use one eligibility set for the watcher, listing and staged tree, including deletions of eligible tracked paths. Treat a truncated scan as incomplete and surface skipped files. Cover large files, dot names, nested repositories, symlinks and odd filenames at the limit.

### Quiet commits can leave Overview and open History stale

**Wrong but recoverable. Read.** `crates/worktrees-api/src/memory.rs:335`, `crates/worktrees-api/src/memory.rs:369`, `app/src/useWsMemory.ts:35`, `app/src/useWsMemory.ts:51`, `app/src/WorkspaceMemory.tsx:282`.

An edit triggers a file event and a refresh before the quiet minute is over. The eventual commit logs its SHA but sends no invalidation. An already open Overview can still show the previous change, and its unseen dot and History can stay stale until another refresh. Permanent commit errors are also only logged while the poll retries.

**Fix (small).** Emit a console-only history change after a successful commit and surface repeated commit failures. Leave editor file-change handling tied to actual file changes.

### Float can report success without opening the document

**Wrong but recoverable. Read.** `app/src/doc/MainDocument.tsx:145`, `app/src/App.tsx:4920`, `app/src/App.tsx:7024`, `app/src/App.tsx:10850`, `app/companion/Companion.tsx:629`.

Main saves and closes the source before the destination acknowledges that it opened the document. If the companion refuses to leave its own conflicted draft, it consumes the incoming request and leaves the wrong document open. If the window fails to open or local storage is unavailable, the same card claims the document moved. The saved text is still on disk and Open it here recovers it.

**Fix (medium).** Keep the source visible until an acknowledgement names the requested document and its opening. Surface refusal at the source. Use the same handshake for Memory.

## Friction

### Memory has no Float or Dock handoff yet

**Friction. Read.** `app/src/App.tsx:10848`, `app/src/App.tsx:7024`, `app/companion/Companion.tsx:629`, `app/src-tauri/src/companion.rs:95`.

The branch intentionally omits Float for a memory file. The handoff carries only an idea ID and the destination knows only the Ideas library. The combined design calls for a workspace and path handoff with the shared editor and a return to the original workspace.

**Fix (medium).** Add a document target to the acknowledged handoff above. Preserve the Ideas window's ordinary entry point.

### The poll and small history reads can pay for the whole workspace collection

**Friction. Read.** `crates/worktrees-api/src/watch.rs:207`, `crates/worktrees-core/src/workspacechats.rs:139`, `crates/worktrees-api/src/host.rs:345`, `crates/worktrees-core/src/memory.rs:1086`, `crates/worktrees-core/src/memory.rs:1134`, `crates/worktrees-api/src/memory.rs:84`.

Each live chat searches the workspace directory again. Every memory is walked on the shared host poll, and quiet commits run Git on that same thread. Fetching a single latest change or one paged History list also scans revert messages across the full history. The checked-in microbenchmark measures only file stamps for 20 small workspaces (`crates/worktrees-core/src/memory.rs:1871`). It does not measure whole-poll latency, command contention or long Git histories.

**Fix (medium).** Build the chat-to-workspace map once per poll and share it with activity. Move slow memory work off the shared poll. Cache the revert index by repository and HEAD. Measure full ticks and UI requests with thousands of files and commits.

## Five changes before shipping

1. Drain and check saves across navigation and close. Use real document revisions so two writers cannot silently overwrite each other.
2. Give each fold a durable attempt, receipt recovery and one cross-engine reservation. Test a crash at each durable step and check the partial-note deadline.
3. Make actual workspace chat permissions visible, revoke stale grants and enforce the filesystem boundary for subprocesses if private access and read-only projects are advertised.
4. Protect engine records, Git metadata and locks from the chat's writable tree. Use consistent path containment and verify that Keep mine preserves the replaced bytes.
5. Send a history invalidation when a quiet commit lands and acknowledge Float from the destination. Drive the resulting save, close, conflict, Undo and restart flows in the sandbox app.

## Checked and found sound

- The normal launch quotes structured settings and uses checked IDs and fixed openers. It does not pass workspace projects to Claude with `--add-dir`. The measured skill-loading issue therefore has no path through that flag. `crates/worktrees-core/src/desksessions.rs:384`, `crates/worktrees-core/src/workspacechats.rs:862`, `docs/proposals/workspace-chats-measurements.md:12`.
- The direct workspace chat and memory commands are console-only. The resolver checks workspace membership before chat membership. Both workspace events are console events, and the terminal open takes a chat selected by the resolver. The audited text mutations carry text length and hash, and the poll's fold event carries the IDs and commit. These are static checks of the direct paths. `crates/worktrees-cli/src/serve/resolve.rs:405`, `crates/worktrees-cli/src/serve/resolve.rs:698`, `crates/worktrees-cli/src/serve/dispatch.rs:1471`, `crates/worktrees-cli/src/serve/scope.rs:207`, `crates/worktrees-cli/src/serve/audit.rs:1648`, `crates/worktrees-cli/src/serve/audit.rs:2091`.
- On the ordinary memory path, Git has explicit work tree and git dir arguments, disables hook paths and signing, rejects a top-level gitfile and commondir, and starts with no template. Image reads and historical image blobs apply size and format checks. A normal conflicting revert tries to abort and reports the conflicted paths, with unit coverage for that sequence. `crates/worktrees-core/src/memory.rs:446`, `crates/worktrees-core/src/memory.rs:533`, `crates/worktrees-core/src/memory.rs:555`, `crates/worktrees-core/src/memory.rs:670`, `crates/worktrees-core/src/memory.rs:814`, `crates/worktrees-core/src/memory.rs:1184`, `crates/worktrees-core/src/memory.rs:1241`, `crates/worktrees-core/src/memory.rs:1805`.
- Ordinary navigation to a place keeps the main editor mounted, and a refused save with a conflict retains the editor's text. The shared save session serializes actual writes and tests the case where a second flush arrives during a first write. `app/src/doc/MainDocument.tsx:119`, `app/src/doc/MainDocument.tsx:159`, `app/src/doc/ideasave.ts:153`, `app/scripts/ideas-save-check.mjs:213`.

## What I could not check

- The two deterministic Node checks used the actual save session with scripted reads and writes. They do not cover two real windows, native close, companion focus, or an actual save collision on APFS.
- No live Claude or app session, paired device, guest, or existing workspace was used. The private-path review covers source and the recorded Claude measurements. Claude's live path matching, alternate permission modes, and OS-level behavior were outside this run.
- The designated scratch path is outside the review sandbox's writable roots. No separate Git or Rust fault-injection fixture, two-engine runtime stress test, or large-history benchmark was run.
- The targeted Rust fold tests did not start. The required gate wrapper stayed queued, and a bounded probe also timed out. The engine results here are source review.
