---
title: "Workspace view review: the person at the screen"
---

# Workspace view review: Kimi

Review of `162512b`, the branch as built. I read the proposals, the frontend, the engine rows, and the fold and memory stores. I drove the mock in Chromium 159 and Playwright WebKit 27.2. I did not change product code.

I found no case that destroyed text. Two bugs make extra recovery work necessary. The rest are wrong states, keyboard friction, or unfinished parts of the design.

## Findings

### 1. Close can fold before the open editor has saved

**Severity:** wrong but recoverable.

**Where:** `app/src/WorkspaceChat.tsx:184`, `app/src/WorkspaceChat.tsx:188`, `app/src/App.tsx:6909`, `app/src/WorkspaceMemory.tsx:227`, `app/src/doc/MainDocument.tsx:113`.

**What happens:** Close, New chat, and Undo call the engine without awaiting the document's `leave()`. A click does blur the editor and start a save, but the fold starts while that save is in flight. The fold can commit and close first. When the save then meets the fold's newer index, the person gets a conflict caused by their own Close.

**How to see it:** Edit `MEMORY.md`, then Close immediately. With a 700 ms write and a 50 ms mock fold, the write began first and the fold began 1 to 2 ms later. The receipt appeared while the header still said `Saving…`. The write then became `This file was changed in another editor`. Both engines did this. Screenshots: `.worktrees-attachments/agent/close-save-race-chromium.png` and `close-save-race-webkit.png`.

**Proof:** PROVED with controlled IPC latency in both engines.

**Fix:** Give lifecycle and revert actions one shared `await mainDocApi.leave()` gate. Do not start the engine act if it refuses. Cover Close, Fold now, New chat, Resume, Undo, and Redo with delayed-write browser tests. **Size:** M.

### 2. Turning off a beta can strand unsaved text behind a dead link

**Severity:** wrong but recoverable.

**Where:** `app/src/App.tsx:6856`, `app/src/App.tsx:6951`, `app/src/App.tsx:10780`, `app/src/App.tsx:10785`, `app/src/wsview.ts:20`.

**What happens:** The beta-off effect tries to save and close the document. If the save fails, the pane correctly stays alive but hidden. `tabOr` maps its now-unavailable tab to Overview. The Overview says the text is not saved, but `Go to it` maps to Overview again. Nothing visible opens the text. Re-enabling the beta is the only way back that I found.

**How to see it:** With both betas on, make an idea's write fail, type, and turn Ideas off. With the reverse combination, make a memory write fail, type, and turn Workspace chats off. In both directions and both engines, `Go to it` left Overview selected and the document hidden. Screenshot: `.worktrees-attachments/agent/beta-off-strands-unsaved-chromium.png` and its `-webkit.png` twin.

**Proof:** PROVED for an idea and for memory, in both engines.

**Fix:** Keep a rescue surface for a stuck document regardless of either beta. `Go to it` must open that surface. Another option is to refuse the switch with a direct path to Keep mine or Reload theirs. **Size:** M.

### 3. An image preview can show the previous file under the new file's name

**Severity:** wrong but recoverable.

**Where:** `app/src/WorkspaceMemory.tsx:169`, `app/src/WorkspaceMemory.tsx:171`, `app/src/WorkspaceMemory.tsx:179`.

**What happens:** `MemoryPreview` keeps `img` while its `file.path` changes. The effect fetches the new image but never clears the old one. During the read, the caption and selected row name the new file while the pixels are the old file's.

**How to see it:** Open `files/rack.svg`, then open another image with its read delayed. The preview said `files/second.svg` and the image's alt was `second.svg`, but its `src` was still rack's. Both engines did this. Screenshots: `.worktrees-attachments/agent/image-switch-stale-chromium.png` and `image-switch-stale-webkit.png`.

**Proof:** PROVED by comparing the old and new image sources in both engines.

**Fix:** Clear the picture when path or mtime changes, or key the preview by both. `DocPicture` in `app/src/doc/DocumentPane.tsx:36` has the same state shape and deserves the same test. **Size:** S.

### 4. The intro promises a write boundary that shell programs do not have

**Severity:** wrong but recoverable.

**Where:** `app/src/wschats.ts:118`, `crates/worktrees-core/src/workspacechats.rs:21`, `docs/proposals/workspace-chats-measurements.md:33`.

**What happens:** Every chat says `It never writes in a project.` The measured rule blocks Claude's file tools and recognized shell writes. Another program run by the shell can still write. The copy reads as a guarantee, not as an instruction plus a partial guard. A person may trust it more than the implementation earns.

**Proof:** READ. The engine's own comment and the measurement record the shell hole. I did not rerun that Claude probe.

**Fix:** Say what is true, for example: `It is instructed not to write in a project. Claude's file tools are blocked; programs it runs are not covered.` Keep the shorter claim only if a real sandbox enforces it. **Size:** S.

### 5. Chat menus and workspace tabs do not implement their keyboard roles

**Severity:** friction.

**Where:** `app/src/CtxMenu.tsx:38`, `app/src/WorkspaceChat.tsx:280`, `app/src/WorkspaceFrame.tsx:225`, `app/src/doc/DocParts.tsx:49`.

**What happens:** Opening a chat menu with Enter leaves focus on its trigger. Tab moves to a control behind the menu, not to Rename. The menu has no `menu` role and its items have no `menuitem` role. Escape closes it without returning focus to the trigger. The workspace tabs all sit in Tab order, have no associated tab panels, and do nothing on ArrowLeft or ArrowRight. The document menu focuses its first item when opened by key, but also loses focus when it closes.

**How to see it:** In Chromium, Enter on the chat title followed by Tab focused `reads 2 of 3`. In WebKit it focused the composer. Neither entered the menu. ArrowRight on Overview changed neither focus nor selection in either engine.

**Proof:** PROVED with active-element and ARIA inspection in both engines.

**Fix:** Give the shared menu shell menu semantics, initial focus, arrow navigation, and focus return. Use the tab pattern with one tab stop, arrow movement, and linked panels. Add keyboard-only tests for open, move, Escape, and the next action. **Size:** M.

### 6. Keyboard resizing can make the splitter's value false

**Severity:** friction.

**Where:** `app/src/WorkspaceFrame.tsx:171`, `app/src/WorkspaceFrame.tsx:211`, `app/src/WorkspaceFrame.tsx:216`.

**What happens:** Pointer dragging clamps against 340 px for documents and 420 px for chat. Arrow keys instead clamp to 20 to 80 percent. CSS enforces the pixel floors afterward. The stored share and `aria-valuenow` keep moving after the visible pane stops. A screen reader hears a percentage that is not on screen.

**How to see it:** At a 1052 px body, ArrowRight down to the keyboard minimum produced `aria-valuenow=20` and stored `0.2`. The documents stayed 340 px, which is 32.3 percent. Both engines matched. Screenshots: `.worktrees-attachments/agent/split-keyboard-range-chromium.png` and `split-keyboard-range-webkit.png`.

**Proof:** PROVED with geometry, storage, and ARIA reads in both engines.

**Fix:** Run pointer and key changes through one bounds function. Publish the resulting actual share, not the requested one. Test both dock sides and a resize after the gesture. **Size:** S.

### 7. Memory files cannot float

**Severity:** friction.

**Where:** `app/src/App.tsx:10848`, `app/src/doc/MainDocument.tsx:143`, `app/companion/Companion.tsx:629`.

**What happens:** Float is passed only for an idea source. Memory has no Float button, no hand-off by workspace and path, and no Dock action to bring one back. This is a direct gap against `workspace-view-design.md` §4.4 and §4.6.

**How to see it:** Open Memory in either engine. The header has Edit, Read, and History, with no Float. The existing browser check explicitly accepts `does not float yet`.

**Proof:** PROVED by the absence of the control in both engines, and READ through the hand-off.

**Fix:** Add the workspace-and-path request, the shared companion pane, the main-window card, and Dock. Decide whether the window is now named Documents, as the design's word decision says. **Size:** M-L.

### 8. A place does not know its workspace

**Severity:** friction.

**Where:** `app/src/App.tsx:7201`, `app/src/App.tsx:10250`, `app/src/App.tsx:10894`.

**What happens:** Selecting a place stores only repo and slug. Its header shows project and place, not the workspace it came from. The dock has the existing place tabs, not the workspace's read-only Memory, Plan, and Ideas tab. There is no `Edit in <workspace>` route. This is the whole of design §6 that is absent here.

**How to see it:** Select a place from a workspace card in either engine. The header has no workspace crumb and no control naming the workspace. Back through global history still works, but that is not the designed persistent crumb.

**Proof:** PROVED for the header in both engines. READ for selection and the dock.

**Fix:** Carry the originating workspace in selection, including the multi-workspace picker and the All projects rule. Add the crumb and read-only dock tab, with `Edit in <workspace>` and return. **Size:** M.

### 9. A long index offers a Tidy button that can never work

**Severity:** friction.

**Where:** `app/src/WorkspaceMemory.tsx:121`.

**What happens:** Past 16 KB, the strip is useful and clear, but Tidy is disabled with `Not yet`. This is a visible dead control in a shipped flow. The design asks it to start a chat that moves detail into topics.

**Proof:** PROVED by the existing 16 KB mock state in both engines, and READ in the component.

**Fix:** Implement the fixed tidy chat, or remove the button until that exists and leave only the explanation. **Size:** M to implement, S to hide honestly.

### 10. A removed workspace's kept folder has no direct reveal action

**Severity:** idea.

**Where:** `app/src/App.tsx:6779`, `app/src/App.tsx:6782`.

**What happens:** Removal keeps the folder and shows its full path in a transient notice. It does not offer `Show in Finder`, unlike the memory preview and the design's §7. The person must copy a long path from a notice before it goes away.

**Proof:** READ. The notice has only text; `revealOnHost` is already used for memory files at `app/src/App.tsx:10815`.

**Fix:** Make the notice persistent enough to use and add `Show in Finder` for the moved folder. **Size:** S.

### 11. The chat header does not name its model

**Severity:** idea.

**Where:** `app/src/WorkspaceChat.tsx:203`, `app/src/WorkspaceChat.tsx:216`.

**What happens:** The mockups show the model beside the title. The build omits it. The written header rule does not require it, so this is not a regression. A workspace chat uses the configured Claude default, and the person cannot see that choice here.

**Proof:** READ, and confirmed by the header rendered in both engines.

**Fix:** If model identity matters to these chats, show the transcript's current model where the width allows and fold it into the title menu when narrow. **Size:** S-M.

## The five things I would do before this ships

1. Serialize editor saves with chat starts, folds, resumes, Undo, and Redo. Prove no lifecycle receipt can outrun a save.
2. Make unsaved text reachable after every beta switch. Test both directions with failed writes.
3. Finish keyboard and screen-reader behavior for menus, tabs, the splitter, and inline rename. Check focus return after every close.
4. Either finish memory Float, the place's workspace route, and Tidy, or remove those promises from the release surface and copy.
5. Clean up misleading states and claims: reset old image pixels, make splitter values true, and replace the absolute project-write guarantee.

## What I checked and found sound

- The existing `documents`, `workspace-view`, `workspace-chats`, `workspace-memory`, and `workspace-fold` browser suites all passed in Chromium and WebKit against this worktree. No suite reported a page error.
- The pure `wsview-check`, `wschats-check`, `wsmemory-check`, and `workspaces-check` scripts passed.
- All four beta combinations drew the intended tabs and chips. With both off, the workspace name folds its card. Ideas alone has no chats. Chats alone has Memory without Plan or Ideas.
- A first chat, a refused start, Try again, automatic naming, manual rename, per-chat drafts, prompt answers, and Chat or Terminal switching worked. The engine opener stayed a quiet line rather than a person's bubble.
- Close refusal while busy or waiting, Stop, one fold at a time, waiting with its clock stopped, checking, failure, landing, no changes, Fold now, Resume, Delete, and a missing transcript all had readable states.
- Fold receipts stayed in the transcript. A second fold did not reuse the first note. Undo and Redo made new changes. A conflicting Undo named the file and changed nothing.
- Memory Read did not fetch the editor. Edit did. Autosave, conflicts, Keep mine, Reload theirs, New file, `[[` links, relative links, missing links, remote-image refusal, image drops, and the 4 MB refusal worked.
- History filtered to one file or all files, showed one file's diff, and drew changed images before and after. Empty, unreadable, and long-memory states were distinct.
- Thirty-two chats showed all live plus three closed in the nav. The Chats tab listed and filtered all 32. At 900, 1280, and 1600 px in Tokyo Night and Tokyo Day, neither engine overflowed and New chat stayed reachable.
- Two hundred notes folded by month and filtered correctly in both engines. The six-theme contrast checks in the existing suites passed. Diff tints were composited over the panel.
- Delayed workspace and document reads with `slowlist=500` and `slowread=200` survived a workspace to place to workspace round trip in both engines. The document and selection returned without an error or sideways overflow.
- Waiting beat working on the workspace and Workspaces dots. Back and Forward included workspaces, tabs, chats, ideas, and places. Home was not lit in a workspace.

## Differences from the design not already listed

- The Overview hides the index's duplicate first-level heading. The static mockup shows it. **The build is better.** One heading is enough.
- The empty Overview puts New chat in the Chats card, not again inside No memory. The Memory tab has both designed actions. **The build is better.** The second New chat would be noise.
- The build's chat and fold copy is shorter than the static mockups. The written design wins where they disagree. **The build is better where the meaning survives.** The exceptions are the write guarantee and the missing model idea above.

## What I could not check

- I did not drive the real macOS app or a real Claude in this lane. The brief's earlier VM run covers one such run. Browser WebKit is not the app's WKWebView.
- Memory Float and its two-window conflict behavior do not exist to exercise. Tidy does not exist either.
- I did not measure OS notifications. The brief already lists the lack of a background waiting notification as open.
- I did not repeat the known fold-note naming issue. It is still present by design of the current engine path, and the brief already records it.
- I did not exercise a paired device, a guest share, Linux, a real Finder reveal, or a real full disk.
- The save race used controlled latency to make the ordering deterministic. I proved the missing ordering guarantee, not how often a real machine hits it.
