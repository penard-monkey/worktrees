---
title: "Proposal — an in-app editor for the Files dock"
---

# Proposal — an in-app editor for the Files dock, and new files from the tree

**Status:** design, 2026-10-06. Not approved. §9 lists the decisions that are
David's to make.

**The ask.** "I can't edit a file. I opened up a file and tried to click
Editor but it doesn't work." David wants a full editor in the app. He wants
editing to be the DEFAULT, and is not sure a read-only mode should exist at
all. He also wants to right-click in the Files tree and create a new file.

## 1. What happened when he clicked "Editor"

"Editor" in the viewer header (`FilesPane.tsx`, beside Expand) never meant
"edit here". It runs `open_editor`, which hands the file to the **external**
command in Settings → Commands → Editor command (default `code`). That command
ran through `/bin/sh -c`, and the child was reaped in a thread **that ignored
its exit status**. `code` is not installed on David's machine, so sh exited 127
and nothing was ever shown. `open_terminal` had the same shape.

That bug is fixed in its own PR (`launch_watched` in `lib.rs`). A non-zero exit
within 1.5 s is now an error the user sees: "Editor command `code` was not
found — install it, or change it in Settings → Commands", or the last line the
command printed to stderr. A command still running at the deadline
(`code --wait`, a terminal that stays in the foreground) counts as launched,
because past that point "working" and "slow" cannot be told apart, and holding
the click would be worse than missing a late failure.

The fix does not change the label. Once the editor below exists, the button
should say what it does: **"Open in…"**, with the configured command in its
tooltip (§4.6).

## 2. What already exists — more than the ask assumes

The dock is not read-only today. `FilesPane.tsx`'s header says so: it is
read-only **with one exception**. #56 and #306 made a markdown file's Source
view editable. Concretely:

| Piece | Where | What it does |
|---|---|---|
| `SourceEditor` | `FilesPane.tsx` | a `<textarea>`, ⌘S, a B / I / H1–3 / list toolbar. Formatting goes through `execCommand` so ⌘Z can undo it |
| draft store | `drafts` / `useDraft` | module-scope; a draft survives Preview↔Source, a place switch and closing the dock, but **not a webview reload** |
| `write_file` | `lib.rs` | `guard_under_projects`; file must already exist; atomic temp+rename; copies the mode; **compare-and-swap on the mtime the edit started from** |
| conflict UI | viewer header | `unsaved` tag; on a refusal: `save refused` + Discard + a separately labelled **Overwrite** |
| mock | `mock/install.ts` | models `read_file` and `write_file`, CAS included |

The header also records **why editing stopped at markdown**. A textarea cannot
show highlighting, line numbers, or ⌘F's match painting: the CSS Custom
Highlight API cannot paint inside a textarea. Markdown source had none of
those to lose (`filekind.ts` gives it `lang: ""`). A `.rs` file would lose all
three. So the real question is not "add editing". It is **what replaces the
textarea so that every text file can be edited without getting worse to
read.**

Three latent defects in what exists, found while surveying it. Each is in
scope for phase 2:

1. **CRLF is silently rewritten to LF.** A textarea's `value` normalises line
   breaks; measured in WebKit, `t.value = "a\r\nb\r\n"` reads back `"a\nb\n"`.
   The first keystroke in a CRLF markdown file therefore makes every line
   ending "changed", and ⌘S writes the file back as LF.
2. **Non-UTF-8 is corrupted on save.** `read_file` decodes with
   `from_utf8_lossy`, so a Latin-1 byte becomes U+FFFD in the buffer, and
   saving writes the replacement character back.
3. **The temp name is fixed** (`.{name}.wt-tmp`). Two saves of the same file
   (the dock and the reading overlay are both mounted) race on one temp file.
   `config::edit_user_config` uses a pid-suffixed name. The fix is a unique
   name: pid plus a counter.

## 3. The editing component — the options

AGENTS.md: "No UI libraries" means no component or design-system libraries
**and no editor**. Only two exceptions have been admitted, each with a written
justification: `marked` (a lexer that returns data) and `mermaid` (renders,
but is confined to the viewer bundle and fenced off by
`viewer-boundary-check.mjs`). An editor library would be a third exception,
and it would live in `app/src`, which mermaid is explicitly barred from. That
makes it **David's decision** (§9 Q1). The options, with numbers measured for
this proposal (esbuild `--minify`, ESM, in
`~/.cache/worktrees/worktrees/in-app-editor/bundle`):

**For scale, the app today:** `vite build` produces 968 KB raw / 287 KB gzip
of JS (two chunks).

### (a) Hand-rolled

**A `<textarea>` for every file** (what markdown has now). It costs nothing
and is native, so undo, IME, selection, spellcheck-off and accessibility all
come free. It loses everything §2 said: no highlighting, no line numbers, no
⌘F painting. Undo is also fragile: any programmatic write except
`execCommand` erases the undo stack. It also normalises CRLF (§2). It is the
right tool for prose and the wrong one for code. It also re-renders a 1 MB
string through React on every keystroke.

**A transparent textarea over the `highlight.ts` rendering** (the
"react-simple-code-editor" trick). It keeps our highlighter, but each of these
has to be got exactly right, in WebKit, forever:

- **Pixel-identical layout** of the overlay and the `<pre>`: font, tab size,
  wrap points, scrollbar gutter. One subpixel of drift and the caret sits
  between glyphs. AGENTS.md already records WKWebView sizing a `<button>` by
  its own rules.
- **Re-tokenising per keystroke.** `tokenizeLines` on 1 MB measured 38–76 ms,
  over a frame budget, so this needs debouncing. While the debounce waits,
  the colours sit on the wrong characters.
- **No virtualisation.** A textarea must hold the whole document, and so must
  the `<pre>` under it. `CodeBlock` already renders every line of a file up to
  the 1 MiB cap.
- **Line numbers under soft wrap.** These need per-line height measurement.
- **Tab / Shift-Tab indent, auto-indent on Enter, bracket handling.** Each is
  an `execCommand` dance, or undo breaks.
- **Find/replace.** The Custom Highlight API still cannot paint inside the
  textarea, so find paints on the `<pre>` underneath. That works only while
  the overlay is pixel-exact.
- **IME composition** (dead keys, CJK) has to stay in step across both
  layers.

**contenteditable over `highlight.ts`.** Strictly harder. Every engine's
contenteditable produces different DOM, and you own selection mapping, IME,
undo and paste sanitising. This is the problem CodeMirror 6 *is*.

The hand-rolled cost is not one weekend. It is a permanent tax on every
WebKit release.

### (b) CodeMirror 6 — recommended

Modular, so we import only what we use. Measured:

| Tier | Contents | Raw | Gzip |
|---|---|---|---|
| core | `state` + `view` + `commands` (line numbers, history, keymaps, draw-selection) | 289 KB | 94 KB |
| **+ search, language** | + `@codemirror/search` (find/replace state), `@codemirror/language` (`StreamLanguage`, `HighlightStyle`, folding, bracket matching) | **324 KB** | **105 KB** |
| + six Lezer grammars | + markdown, JS/TS, Rust, Python, CSS, JSON | 684 KB | 238 KB |
| `basicSetup` + seven grammars | what the getting-started guide gives you | 741 KB | 256 KB |

- **Size.** The recommended tier adds about a third to the app's JS. It
  should be a **dynamic `import()`** so vite splits it into its own chunk,
  loaded the first time a file opens editable. That does not shrink the
  binary, but it keeps startup parse time where it is.
- **Highlighting without the grammars.** We do not need Lezer: the 360 KB
  between the second and third rows is exactly what can be skipped.
  `highlight.ts` (3.6 KB gzip) already defines the app's look, and the viewer,
  the diff and the editor should agree on it. It plugs in as a decoration
  source. In phase 2, decorate from a debounced whole-document tokenise
  (<5 ms below 100 KB; 38–76 ms at the 1 MiB cap, off the keystroke path).
  Later, refactor the scanner so it can resume at a line
  (its only cross-line states are "inside block comment X" and "inside string
  Q"), and wrap it in `StreamLanguage`, which caches per line for free.
- **WKWebView.** CM6 supports Safari as a first-class target, and its input
  layer exists to normalise exactly the contenteditable/IME differences that
  (a) would have to handle by hand. Measured in Playwright **WebKit** (build
  2359) and Chromium: a 3.66 MB, 60,000-line document mounted in **22 ms**
  with **67** `.cm-line` elements in the DOM (viewport virtualisation).
  Typing, Tab-indent and undo behaved identically in both. IME and the
  macOS 26 Writing Tools path (`allowsWritingToolsAffordance`, AGENTS.md) need
  a hand check in the real app. That is the phase-2 manual gate.
- **Theming.** CM6 styles through `EditorView.theme({...})`, which emits
  ordinary CSS. Every value can be a `var(--…)` from `tokens.css`: `--term-*`
  for the font, the `--bg-*` surfaces, `--accent` for the selection and caret,
  and the existing `.tk-*` classes (`CodeView.tsx`) for syntax. One theme follows all six app
  themes, with no per-theme JS and no rebuild on a theme switch.
- **License.** MIT (state 6.7.6, view 6.43.13). No runtime network, no
  workers, no `eval`, so the CSP does not change.

### (c) Monaco

- **Size.** Measured: **2.72 MB** of JS plus 91 KB of CSS (701 KB gzip) for
  the core API with **no languages**, plus a 307 KB editor worker. The
  `node_modules` footprint is 102 MB. That is three times the whole app,
  before a single language.
- **Workers.** Monaco needs `MonacoEnvironment.getWorker` wiring. Under
  Tauri's `tauri://` scheme that means `worker-src` in the CSP and bundling
  worker entry points through vite.
- **WKWebView.** Monaco is built for Chromium (VS Code). Safari works, but its
  IME and selection behaviour there are its weakest, least-tested path.
- **Look.** Monaco's look is VS Code's and fights the app's tokens. Its
  theming is a JS theme object, not CSS variables, so every app theme needs
  its own Monaco theme, regenerated on every switch.
- **License.** MIT. Monaco only pays off for LSP-grade features (IntelliSense,
  multi-file models). This dock is not trying to be an IDE: agents write the
  code, and the dock is where you read it and fix a line.

### (d) Others considered

- **Ace.** Mature, about 400 KB, but older architecture, no virtualised
  layout of the same quality, and themes in JS. CM6 dominates it on every
  axis that matters here.
- **ProseMirror / Lexical / TipTap.** Rich-text frameworks. A markdown WYSIWYG
  would be one, but they do not edit code, and ProseMirror would be a second
  editor beside whatever edits code. Not now.
- **`<textarea>` plus a CM6 fallback for big files.** Two editors and two sets
  of bugs. No.

### Recommendation

**CodeMirror 6, tier two (`state`, `view`, `commands`, `search`,
`language`), no Lezer grammars, highlighting from `highlight.ts`, loaded by
dynamic import.** The AGENTS.md exception note, in the mermaid paragraph's
shape:

> **CodeMirror 6 is the THIRD admitted exception, and only the five core
> packages (`@codemirror/state`, `view`, `commands`, `search`, `language`) —
> no `basicSetup`, no Lezer grammar packages, no third-party extensions.** It
> does not fit through the `marked` door either: it renders, owns a DOM
> subtree, and has its own input model. It is admitted because an editor is
> the one component whose correctness is a property of the platform's
> text-input stack. IME, undo grouping, selection, virtualised layout and
> WebKit's contenteditable quirks were each named in turn as the cost of a
> hand-rolled one, and #306's textarea reached exactly the point where the
> next feature needed all of them at once. What keeps it bounded is the
> boundary, not the argument. It loads only through one dynamic `import()` in
> `app/src/editor/`. Its look comes entirely from `tokens.css` via
> `EditorView.theme`. Its highlighting comes from `highlight.ts`, so the
> viewer, the diff and the editor cannot disagree about a token. Nothing
> outside `editor/` imports `@codemirror/*`, and `editor-boundary-check.mjs`
> asserts that by walking the real import graph, the same way
> `viewer-boundary-check.mjs` guards mermaid. A Lezer grammar would be a
> second highlighter, so adding one reopens this paragraph.

## 4. Integration with the Files dock

### 4.1 What "editor by default" means

| File | Opens as | Notes |
|---|---|---|
| source / text, not truncated, valid UTF-8 | **the editor** | no Preview/Source split for code; the editor IS the source view |
| markdown | **the editor** (source), with Preview as a toggle | the segmented control, as now. Phase 4 adds a **split** (editor left, preview right, scroll-synced by heading) when the content column is ≥ `SPLIT_AT`-wide. `--md-zoom` keeps applying to Preview only |
| SVG | Preview (image), Source = editor | as today |
| image, binary | viewer (unchanged) | "Open in…" stays |
| truncated (> 1 MiB read cap) | read-only viewer + a "too large to edit here — Open in…" note | saving a truncated buffer would delete the tail; the cap stays a hard gate |
| non-UTF-8 (lossy decode) | read-only viewer + "not UTF-8 — Open in…" | §4.4 |
| Diff | read-only `DiffView` | the diff is a comparison, not a buffer. Typing in it means nothing |
| opened at a line (`at`, ⌘-click from a terminal) | **the editor**, cursor and scroll on that line, the line marked by a decoration | today this forces the read-only `CodeBlock` with a mark. In the editor, the mark is a line decoration that clears on the first edit or view change |

**Should read-only mode exist at all?** For text that can be edited: **no
separate mode**. An editor with nothing typed *is* a viewer, provided it does
not grab focus (§4.5). Clicking into it gives a caret, as `SourceEditor`
already decided ("No autofocus … a caret on click is what an editor pane is
expected to do anyway"). The read-only surfaces that remain are the ones
where editing is impossible or meaningless: binary, truncated, non-UTF-8,
and the diff. One cost to know about: the `CodeBlock` read path is what ⌘F
paints on today, so find moves onto the editor's own search state (§4.5).
`CodeBlock` stays for the diff, for SVG source, and for the reading overlay
until phase 2 is proven.

### 4.2 Saving: explicit ⌘S, and drafts that cannot be lost

**Recommendation: explicit ⌘S, not autosave** (§9 Q3). The reason is
specific to this app. The files open in the dock are the files agents are
editing in the pane next door. Autosave turns every pause in typing into a
write that either races an agent's write (and the CAS refuses it, mid-typing,
as an error) or clobbers it (if the CAS were relaxed to make autosave quiet).
With ⌘S, the decision to write is always the human's, and it always happens
at a moment when they are looking.

What makes ⌘S safe rather than lossy:

- **Dirty state is visible everywhere it can matter.** The `unsaved` header
  tag (as now). A dot on the file's **tree row**, and on every ancestor
  directory, mirroring the change-set cascade. A dot on the dock's **Files tab
  pill** when any draft exists in the place. A dot on the **place's nav row**,
  since drafts outlive a place switch.
- **Leaving is not losing.** Drafts already outlive Preview, a file switch, a
  place switch and closing the dock. Two gaps close:
  - **App quit or window close with drafts open.** A Tauri `CloseRequested`
    handler (none exists yet) asks the frontend, and the frontend shows "N unsaved files — Save
    all · Discard · Cancel" (via `useEscape`; Cancel is Escape).
  - **Webview reload or crash.** Persist drafts, debounced at 1 s, to a
    backend-owned `drafts/` directory under the app config dir (keyed by
    canonical path, holding text plus base mtime/hash). On launch, restore
    them as drafts, never as writes. This must be backend-owned:
    `ui-state.json` is whole-blob and frontend-written, and AGENTS.md says the
    backend never writes into it. The reverse holds too: drafts must not ride
    in it.

### 4.3 Writing: atomic, mode-preserving, through symlinks

`write_file` keeps its shape and gets the `edit_user_config` (#437) treatment:

- write a **unique** temp (`.{name}.wt-{pid}-{n}`) in the target's directory;
  `set_permissions` from the target's metadata **before** the rename; then
  rename. On any error, remove the temp.
- `fsync` the temp before the rename, and the directory after. A crash must
  leave either the old file or the new one.
- **Symlinks: write the target, keep the link.** This already happens:
  `guard_under_projects` canonicalises and the rename lands on the canonical
  path. That should be named in the docstring and pinned by a test, because
  a "fix" that writes to the literal path would replace the link with a
  regular file.
- **New files** are not `write_file`'s job; it keeps refusing a missing path.
  Creation is its own command with its own checks (§5).

### 4.4 External changes: agents edit these files constantly

Today the only re-read trigger is `places:changed`, which fires on a tmux
fingerprint change or every 30 s. A clean buffer can sit up to 30 s stale.
A dirty buffer only learns of the conflict when ⌘S is refused.

**Detect.** Add a cheap `file_stat(path) -> {mtime, size}` command. Poll it
for the **open file only**, every 1.5 s, while the document is visible
(gated the same way `useUsage` is; AGENTS.md records that
`visibilitychange` works in WKWebView). One `stat` per 1.5 s is nothing. An
FSEvents watcher (`notify`) would be push-based but is a new dependency and a
new thread per place, for one file. Not worth it.

**Decide by content, not by clock.** `git checkout`, formatters and some
agents rewrite identical bytes. So when the stat moves, re-read, and compare
a **hash** of the new bytes with the hash of the baseline. `read_file` should
return `hash` (FNV-1a 64 is enough; this is change detection, not security).

- **Clean buffer, content changed.** Reload in place, as a CM6 transaction
  that replaces only the **changed range** (a common prefix/suffix diff, the
  same one `spliceRange` already does). Cursor, selection and scroll then map
  through the change instead of jumping to the top. A brief, non-modal
  "reloaded — changed on disk" header tag.
- **Dirty buffer, content changed.** A banner across the top of the editor:
  *"This file changed on disk while you were editing."* It has three actions:
  - **Reload** discards my edits.
  - **Keep mine** re-bases the draft on the new disk version, so the next ⌘S
    overwrites it knowingly.
  - **Compare** opens `DiffView` of disk vs. buffer, reusing the existing
    two-column grid.

  The banner stays until one is chosen, and ⌘S while it stands does what
  "Keep mine" says only after a confirm. This replaces today's "save refused,
  then Overwrite" with the same safety, offered *before* the user has typed
  more on a stale base.
- **The file is deleted under the buffer.** The banner says so, and ⌘S
  offers "Save as new file" (it goes through the create command).
- **The CAS stays as the last line.** `write_file` compares
  `(mtime, size, hash)` rather than mtime alone, so a same-bytes touch no
  longer reads as a conflict.

**Line endings and encoding.**

- `read_file` reports `eol: "lf" | "crlf" | "mixed"` and `utf8: bool`.
- CRLF: set CM6's `EditorState.lineSeparator` facet to `"\r\n"`, so the
  document round-trips byte-for-byte. Mixed: open read-only-by-default, with
  "Normalize to LF and edit" as an explicit choice. Saying nothing would
  silently rewrite every minority line.
- Not UTF-8: read-only with "Open in…". Supporting other encodings is scope
  creep for a dock whose files are written by agents in UTF-8.
- A **BOM** round-trips: strip it on read, flag it, and restore it on write.
- A **trailing newline** is preserved as typed (no editor-side "ensure final
  newline").

### 4.5 Keyboard

- **xterm and the app chords.** The editor is a contenteditable, so
  `isTextField` already exempts it from ⌘←/⌘→ history and the other
  text-field-aware chords. Every app chord that is NOT text-field-aware still
  reaches `window`, after CM6's own keymap: ⌘B (sidebar), ⌘J, ⌘K, ⌘E, ⌘1–9,
  ⌘[ / ⌘], page zoom. CM6 returns `true` from a handled binding and calls
  `preventDefault`, but **not** `stopPropagation`. So the rule from
  `SourceEditor` carries over, made explicit: **the editor stops propagation
  for exactly the chords it binds**, and binds as few app-colliding chords as
  possible.
  - **⌘[ / ⌘]** are CM6's `indentLess` / `indentMore` in `defaultKeymap`.
    **Drop them** from the keymap. Navigation history promises ⌘[ ⌘] work
    everywhere, and Tab / ⇧Tab already indent.
  - **⌘B / ⌘I.** Bold and italic in markdown only (as now). In code they fall
    through to the app.
  - **⌘S** saves. **⌘⇧S** saves all drafts in the place.
  - **⌘Z / ⌘⇧Z** are CM6 history. The app binds no global undo, so there is
    no collision.
  - **⌘/** toggles a comment (CM6's `toggleComment`, which needs a language's
    `commentTokens`; supply them from `highlight.ts`'s grammar table).
- **⌘F: one find bar, not two.** CM6's own search panel would be a second
  find UI with a second look. Instead, keep the app's `FindBar` (one bar
  across the window, as App enforces now) and, when the target is an editor,
  drive CM6's search **state** with `setSearchQuery`, `findNext` /
  `findPrevious`, and `replaceNext` / `replaceAll`. Matches are then painted
  by CM6's decorations, which work in an editor where the Custom Highlight
  API cannot. The bar gains a Replace row only when the target is an editor.
  This also removes today's oddity, where opening Find kicks the markdown
  editor back to the read-only renderer.
- **Escape** stays with `useEscape`, the one capture-phase stack. CM6 binds
  Escape to `simplifySelection`, and that is the only Escape it should see.
  Any app surface stacked above it wins first, which is what the stack is
  for. The editor must not register its own `keydown` for Escape.
- **Focus.** Never autofocus on open, place switch or dock open (the reasons
  are recorded in `SourceEditor`). The one exception: a file just **created**
  from the tree (§5) opens with the caret in it, because the user has just
  asked to type into it.

### 4.6 The header, after

`[path] [kind] [unsaved] [size] … [Preview|Source|Diff] [Wrap] [Save] [Discard] [Expand] [Open in…]`

"Editor" becomes **"Open in…"**, with tooltip "Open in `<editor_cmd>`".
Calling the external app "Editor" while the pane itself is an editor is the
label that caused this proposal. ⌘E keeps meaning "open in the external
editor".

### 4.7 Performance

- **1 MiB read cap.** It stays the edit cap, and maybe moves to 2–4 MiB
  later. CM6 handled 3.66 MB in 22 ms with virtualised lines, so the limit is
  now the IPC string and the tokenise, not the DOM.
- **Tokenising.** Debounce at 150 ms behind the last keystroke. Visible-range
  decoration only. Phase 2's whole-document tokenise is fine up to the cap;
  the resumable scanner (§3b) is the follow-up if 1 MB files feel laggy.
- **Long, log-like files** (the "5000-line scrollback" shape). Virtualised
  lines make 60k lines a non-event. Soft wrap on very long lines is CM6's
  known cost: measure with a 100 KB single-line minified file in the phase-2
  probe, and fall back to wrap-off above a line length threshold.
- **React.** The editor owns its document. React must never pass `value`
  per keystroke, which is what `SourceEditor` does now. The draft store is
  updated from a CM6 `updateListener`, debounced, and the editor is never
  re-created for a re-render. This is the "components inside App() remount"
  rule, applied to an imperative view: the `EditorView` lives in a ref inside
  a module-scope component.

### 4.8 Security and scope

- **Today's guard is "under ANY registered project"**, not "under this
  place". For reading that was deliberate (⌘-click links can name another
  worktree of the same project). For **writes**, tighten to the **place
  root**: the editor is opened from a place, and a write that lands in a
  different worktree, or in `(main)`, from a dock showing this one is a
  surprise. `write_file` and the new create/rename/trash commands take
  `root` as well as `path`, and require
  `canonicalize(path).starts_with(canonicalize(root))`. That is
  `docserver::safe_under`'s shape: canonical on both sides, never
  `symlink_metadata` on a joined path, which checks only the last component
  (AGENTS.md).
- **`.git`** is refused for writes by component, however it is reached,
  matching `classify_symlink`.
- **Files outside the place** (opened through a ⌘-click on an absolute path,
  another worktree, or `(main)`). **Recommendation: open them read-only**,
  with a header note "outside this place — Open in…" (§9 Q4). The other
  reasonable answer is to allow writes anywhere under a registered project,
  which is what `write_file` permits today. The case against it is the same
  one ADR 0001 makes about trust: a link printed by an agent's terminal
  output should not get to pick which tree the user's ⌘S writes into.
- **The existing `[docs]` / `resolve_rel` symlink hole** (ROADMAP) is not
  touched by this work, and must not be copied. New commands use the
  canonical-under-root check only.

## 5. New file (and folder, rename, delete) from the tree

**Phase 3 scope: New file… and New folder… only.** Rename and Trash go to
phase 5 (§9 Q5).

- **Where it is offered.**
  - Right-click on a **directory** row: "New file…", "New folder…", creating
    inside that directory.
  - Right-click on a **file** row: the same two, creating in the file's
    directory (VS Code's behaviour, and the least surprising one).
  - Right-click on **empty tree space**: creates at the place root. This
    needs a new `onContextMenu` on `.dock-tree` itself; today only rows have
    one.
  - Not offered on inert rows (blocked symlinks, ghosts).
- **The name input is inline, in the tree.** A temporary row appears under the
  target directory, which is expanded if collapsed, holding an `<input>`
  pre-focused with `preventScroll`. Enter creates; Escape cancels (via
  `useEscape`); blur with an empty name cancels. A dialog would hide the
  thing you are naming the file *next to*. The CtxMenu closes first, so its
  clamp and ResizeObserver rules are untouched.
- **Validation**, live, under the input, with the same rules again in the
  backend (the backend is the authority):
  - non-empty;
  - no `/` at the start;
  - no `..` or `.` component;
  - no NUL;
  - ≤ 255 bytes per component;
  - no existing entry with that name. Check case-insensitively: macOS APFS is
    case-insensitive, and AGENTS.md records a `Settings.tsx`/`settings.ts`
    collision;
  - not `.git`.

  `a/b/c.ts` is **allowed** and creates the intermediate directories. It is
  the fastest way to make a nested file, and the per-component rules cover
  it.
- **Backend: `create_entry(root, dir, name, kind: "file" | "dir")`.**
  - Canonicalise `root` and `dir`, and require `dir` under `root` (§4.8).
  - Validate `name` component-wise.
  - Create with `OpenOptions::create_new(true)`, which is atomic against a
    race with an agent creating the same name; `create_dir` for a folder.
  - Return the new canonical path.

  The mock models it with a `fsFiles.set` plus an existence check.
- **Showing up.** `TreeNode` lists by effect off `reloadToken`. After a
  create, bump a pane-local token folded into it, so the parent re-lists
  immediately rather than on the next `places:changed`. Expand the parent and
  open the new file in the editor with the caret in it (§4.5).
- **Ignored names.** The listing drops gitignored entries unless "show
  ignored" is on. A new `foo.log` in a repo that ignores `*.log` would be
  created and then vanish from the tree. The editor still opens it, and a
  one-line note says: "created — hidden by .gitignore (show ignored files to
  see it)". "Changed only" mode shows it once `changed_files` reports it as
  untracked.
- **Phase 5: Rename… and Move to Trash.**
  - **Rename** is the same inline input on the row. It uses `rename(2)`
    within the place root, refuses to overwrite, and carries any open draft
    across to the new path.
  - **Trash, never a hard delete.** Use `NSFileManager
    trashItemAtURL:resultingItemURL:error:` through the `objc2` the app
    already links for the WKWebView override. No new crate is needed, and
    Finder's "Put Back" works.
  - It is a single click, without a confirm, because Trash is the undo. A
    directory with uncommitted changes underneath gets one confirm naming the
    count. An open draft for a trashed file is kept, and the editor shows the
    deleted-under-buffer banner (§4.4).

## 6. Testing strategy

- **Mock harness.** `install.ts` must model every new command: `file_stat`,
  `create_entry`, `rename_entry`, `trash_entry`, and the extended `read_file`
  (`eol`, `utf8`, `hash`) and `write_file` (hash CAS, root guard). Add an
  `?extwrite=<ms>` switch that rewrites the open file under the editor on a
  timer. That makes the external-change banner drivable headlessly, and is
  the shape the `?slowlist` switch already set.
- **Drift / guard checks.**
  - `editor-boundary-check.mjs`: nothing outside `app/src/editor/` imports
    `@codemirror/*`, and no `@lezer/*` grammar package in the import graph or
    in `app/dist`.
  - The keymap check: evaluate the real keymap module and fail if it binds
    `Mod-[`, `Mod-]` or any chord in App's chord table without
    `stopPropagation`. Same slice-the-real-source shape as `zoom-check.mjs`.
  - The validator mirror: if the frontend's name validation duplicates
    `create_entry`'s, pin the two against each other (the `dnd-check.mjs`
    shape), or have the frontend call a `validate_name` command and not
    mirror at all. The latter is preferred.
- **Rust unit tests**, each shown failing first:
  - CRLF, BOM and mixed EOL round-trip byte-for-byte;
  - a write through a symlink keeps the link;
  - `create_new` refuses an existing name, including on a case-insensitive
    match;
  - `..`, an absolute path and `.git` are refused;
  - a write outside the place root is refused while inside the project;
  - the unique temp name: two concurrent saves both land, last one wins
    whole;
  - a hash-equal touch is not a conflict.
- **WebKit probes** (headless Playwright WebKit against the mock, per the
  AGENTS.md button-sizing note). These cover:
  - mount and scroll at the 1 MiB cap;
  - the editor host's `min-width: 0` (it is a flex child exactly like
    `.term-host`, and the ratchet note applies);
  - `elementFromPoint` on the banner's buttons inside `.viewer-body`'s hidden
    overflow;
  - CM6's own scroller nested in no other scroller (the `.scroll` note in
    `renderBody`);
  - contrast of the dirty dot and banner in all six themes, composited
    (AGENTS.md's accent-token rule).
- **Manual gate in the real app** (`sandbox.sh --app`, by hand, never
  scripted):
  - IME (Japanese input, dead keys);
  - macOS Writing Tools on a selection;
  - ⌘S while an agent writes the same file in the next pane;
  - quit with drafts open;
  - reload restoring drafts.

## 7. Phases

Each phase can ship on its own.

1. **Editor-command errors.** Done in this lane's code PR. Nothing else
   changes.
2. **Editor core.**
   - CM6 under the exception note: every UTF-8 text file editable by default,
     highlighting from `highlight.ts`, line numbers.
   - ⌘S, dirty dots, external-change detection and banner, hash CAS.
   - EOL, BOM and UTF-8 handling (fixing the three §2 defects).
   - Writes limited to the place root; outside-place read-only.
   - ⌘F driving CM6 search; "Editor" renamed "Open in…".
   - Markdown keeps a Preview toggle and its toolbar.

   This is the big one, but it cannot be split further without shipping an
   editor that loses data in one of the §4.4 cases.
3. **New file / New folder** from the tree (§5), opening in the editor.
4. **Markdown split preview**, plus replace in the find bar if it was not in
   phase 2.
5. **Rename and Move to Trash.**
6. *(Later, optional.)*
   - Quit/close guard and persisted drafts, if phase 2's in-memory drafts
     plus the quit prompt prove not to be enough.
   - The resumable tokeniser.
   - Raising the 1 MiB cap.

## 8. Not doing

- LSP, IntelliSense, go-to-definition, multi-cursor beyond what CM6's
  defaults give, a minimap.
- Tabs of open files. The dock is one file, and nav history (⌘[ ⌘]) is how
  you go back.
- A WYSIWYG markdown editor.
- Editing in the diff view.
- Other encodings.

## 9. Open questions for David

1. **The library exception.** Admit CodeMirror 6 (five core packages, no
   grammars, behind a boundary check), as recommended? Or stay hand-rolled,
   accepting the costs in §3a?
2. **Read-only mode.** Recommendation: none for editable text, since an
   untouched editor is the viewer. Read-only remains only for binary,
   truncated, non-UTF-8, the diff, and files outside the place. Agree?
3. **⌘S or autosave.** Recommendation: ⌘S, with drafts that survive
   everything except a crash (§4.2), because agents write the same files.
   Want autosave at all, even opt-in?
4. **Files outside the place.** Recommendation: read-only, with "Open in…".
   Or allow writes anywhere under a registered project, which is what
   `write_file` permits today?
5. **Rename and delete.** Recommendation: phase 5, with delete going to the
   Trash, one click, no confirm. Want them in phase 3 instead?
