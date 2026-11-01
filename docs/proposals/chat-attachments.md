---
title: "Proposal — attach files and paste images in the chat"
---

# Proposal — attach files and paste images in the chat

**Status:** built on `feat/chat-attachments`, 2026-11-01. Not merged, not
reviewed. (main) will ask the user about an OWASP review before it merges.
The host side (one new serve row) serves both clients: the desktop chat on
this branch, and the iOS app, which is being built against §2 in
`worktrees-ios`.

**The ask** (the user, 2026-11-01): "I'd like to be able to add file
attachments/paste images into the chat view and the ios app."

**How it works, in one paragraph.** An agent reads an attachment the way it
reads any file: by path. So the client UPLOADS the bytes with a new row,
`agent_attach`. The host decides from the bytes what they are, keeps them in
the place's own `.worktrees-attachments/` folder, and answers with a path
relative to the place root. The client then sends its message with the
existing `agent_send`, naming the path in the line. This works the same for
Claude (its Read tool reads images and PDFs), Codex, pi and kimi. A file
inside the place needs no read permission from Claude. Nothing new types into
Claude: `agent_send` is still the one path that does (AGENTS.md).

## 1. The flow

1. The person picks, pastes or drops files. The client refuses a file over
   8 MiB before reading it, and measures the line the message will become
   (§2.6) before uploading anything.
2. On Send, the client uploads each file with `agent_attach`, one after
   another, in order.
3. Only once every upload has succeeded does it send ONE line with
   `agent_send`: `<text> [attached: <path>, <path>]`. If any upload fails,
   nothing is sent.

## 2. The contract

### 2.1 The row

`agent_attach: Engine [instruct, place(repo, slug)]`, in
`crates/worktrees-cli/src/serve/dispatch.rs`, with its body in
`worktrees_api::agents::agent_attach` and its rules in
`worktrees_core::attach`.

Why `instruct`: an attachment leads to the agent reading the file, so it is
drive-class, like a line.
- A paired device is asked for a step-up first. The proof is the existing
  per-connection, five-minute one (`stepup::VALID_FOR`).
- The act spends the audit's `DRIVE` budget.
- A device's attach is stamped on the place: `last_remote_attach
  {principal, device, path, at}`. This is a new stamp, listed in
  `scope::STAMPS`, so another device sees it cut down as every stamp is.

`agent-rows-check.mjs` pins the capability: a one-word edit to `write` would
drop the step-up and the budget with every body test still green.

### 2.2 Arguments and answer

**Args:** `{repo, slug, name, data}`.
- `data` is standard padded base64 (what `btoa`, iOS's
  `base64EncodedString()` and `files::b64_encode` write), read strictly.
- `name` is the user's file name, for DISPLAY ONLY. It never reaches the
  filesystem, the audit log or the stamp. The host strips control and
  invisible characters from it (`harness::invisible`, the Trojan Source set)
  and clips it to 120 characters.

**Answer:** `{path, bytes, sha256, kind, name}`.
- `path` is relative to the place root:
  `.worktrees-attachments/<yyyymmdd>-<16 lowercase hex>.<ext>`, e.g.
  `.worktrees-attachments/20261101-1f3a9c2e5b7d0a46.png`. The date is UTC, the
  hex is 64 random bits, and `ext` comes from the sniff. The longest path is
  53 bytes.
- `bytes` is the decoded size. `sha256` is the hash of the bytes, in hex.
- `kind` is `"image"`, `"pdf"` or `"text"`.
- `name` is the display name, after cleaning. It was added to the brief's
  contract; a client may ignore it.

### 2.3 What is accepted

The BYTES decide (`attach::sniff`), never `name` and never a client's MIME
type:
- **PNG, JPEG, GIF, WebP** (stored as `.png`, `.jpg`, `.gif`, `.webp`). These
  are what Claude reads as images. HEIC is refused by name, so the phone
  converts it to JPEG first.
- **PDF** (`.pdf`).
- **UTF-8 text**: valid UTF-8 with no NUL byte. It is stored as `.txt`, or as
  `.md`, `.json`, `.csv` or `.diff` when `name`'s extension says so (`.markdown`
  counts as `.md` and `.patch` as `.diff`). The name chooses among text
  extensions only; it never makes bytes into another kind.

Anything else is refused with what it was: `empty`, `HEIC`, `HEIF`, `AVIF`,
`MP4 or QuickTime media`, `TIFF`, `BMP`, `ICO`, `ZIP (or a Word, Excel or
other Office file)`, `gzip`, `a program`, `UTF-16 text`, or `binary data`.

### 2.4 Refusals

Each refusal is a stable token, then detail after a colon, like
`start-budget: <s>`. `chat.ts` mirrors them (`ATTACH_ERR`), and
`chatmirror-check.mjs` holds the mirror against `attach.rs` and `budget.rs`.

| Token | When |
|---|---|
| `attach-type: <what> (accepted: PNG, JPEG, GIF, WebP, PDF, UTF-8 text)` | the bytes are not an accepted kind |
| `attach-size: 8388608` | over 8 MiB decoded, judged from the base64's length before anything is decoded |
| `attach-files: 50` | the place already holds 50 files |
| `attach-bytes: 104857600` | the file would take the place past 100 MiB |
| `attach-budget: <seconds>` | the device's upload budget is spent; the seconds are until THIS upload fits |
| `attach-unsafe` | no detail; the reason goes to serve.log. The folder is a symlink (wherever it points), is not a directory, belongs to another user, or git would show the file (§4.2) |
| `unknown command: agent_attach` | an older host; clients turn it into words with `unsupportedFeature` |

Other refusals are the ones every row has: `denied`, `step-up required`,
`audit-budget …`. Data that is not base64 is `invalid args \`data\`: not
base64`.

### 2.5 Storage

- The folder is `<place root>/.worktrees-attachments/`, created 0700 and set
  to 0700 on its own handle at every attach. Files are 0600.
- Files are written with `fsx::write_atomic`: a random temp name,
  `O_CREAT|O_EXCL|O_NOFOLLOW`, then a rename over the final name. A temp left
  by a killed write is swept after an hour.
- The folder is excluded in the repo's common `info/exclude` with an anchored
  line (`/.worktrees-attachments/`) under a comment that says whose it is. The
  line is appended, never rewritten. Before writing, the host checks the
  exclude works with `git check-ignore`. A repo's own `.gitignore` can
  un-ignore the folder, and that outranks `info/exclude`; then the attach is
  refused (`attach-unsafe`).
- A plain `git worktree remove` takes the folder with the place: ignored
  files do not make a tree dirty. This is tested in `attach.rs`.

### 2.6 The line

Both clients send exactly this, as (main) fixed it on 2026-11-01:
- **With text:** `<the user's text> [attached: <path>]`, and for several files
  `<text> [attached: <path1>, <path2>, <path3>]`. One space goes before `[`.
  The paths are in upload order, separated by `, `. They are the host's own,
  so they hold no space or comma.
- **With only attachments:** the lead-in is `Look at the attached file.` for
  one and `Look at the attached files.` for several. So a line never starts
  with a character `typed_line_ok` refuses.

The whole line passes `typed_line_ok`, as any `agent_send` line does: one
line, at most 4,096 bytes (`harness::LINE_MAX`), no control or invisible
characters, no leading `/ @ ! #`.

The user's text is checked by today's rule first. Then the line is measured
with every path at its longest (53 bytes), BEFORE anything is uploaded. The
suffix costs `11 + 55 × N` bytes for N files, so the text may be up to
`4096 − 11 − 55 × N` bytes. The desktop's words are "Too long with 3
attachments: 4,120 of 4,096 bytes." It also caps a message at 10 files.
`chatmirror-check.mjs` runs the composer's check (`attachProblem`) against the
model of `typed_line_ok`, for 1 to 10 files at the boundary, both ways.

The desktop chat draws a sent line's ` [attached: …]` as file chips under
the text (`splitAttached`), and only the host's path shape counts as one.

### 2.7 The audit

Each attach is one record: `bytes` (decoded), `sha256` of the bytes, and
`kind` once the body has answered. It never holds the bytes, their base64, or
`name`. A name can say what a file holds (`payslip.pdf`). An upload that does
not decode is recorded by the size of what was sent (`encoded_bytes`). A
refused attach is recorded without a `kind`. A refusal by the resolver is
recorded once.

## 3. The numbers, and why

| Number | Where | Why |
|---|---|---|
| **8 MiB** a file, decoded | `attach::MAX_BYTES` | Its base64 is ~10.7 MiB, well under serve's 16 MiB `MAX_MESSAGE`, so an upload is ONE frame and no chunking row is needed. The phone caps its own uploads at ~4 MiB. A Retina screenshot of the whole app window (2560×1720 PNG, ten of them from the harness drive, Chromium and WebKit) measured 0.30–0.44 MB. |
| **50 files, 100 MiB** a place | `attach::MAX_FILES`, `MAX_TOTAL` | Enough for a long conversation's screenshots. It bounds what one place can take from the disk, whoever uploads. |
| **7 days** kept | `attach::KEEP` | Long enough to come back to a conversation; short enough that screenshots do not pile up. Swept by the next attach in the place, and by the engine's poll at start and hourly (`agents::sweep_attachments`: a `read_dir` per project and an `lstat` per place, no git). |
| **64 MiB burst, 256 MiB an hour** per device | `budget::BUDGETS.device.uploads` | The burst is 8 files at the host's cap, or 16 at the phone's, so a message with a handful of photos never waits. Once spent, a 4 MiB photo fits again in 56 s. With 7-day expiry this bounds a device to ~6 GiB a day across every place in its scope, and never more than 100 MiB in a place. The console has no upload budget: it is the person at the desk. |
| **10 files** a message | `chat.ts` `ATTACH_PER_MESSAGE` | The client's choice, to keep a line readable. The host does not enforce it: the line's length does. |

**Measured: a large upload does not hold up what serve sends.**
`serve/tests.rs`, `a_reply_goes_out_while_a_large_upload_is_still_arriving`:
- A connection's halves are split (`conn.rs`: `ws.split()`, and ONE writer
  task drains the outbox).
- A reply queued while an 11.2 MB upload frame was still arriving went out at
  once. Request 1 slept 500 ms and was answered at ~505 ms, three runs of
  three, while the upload frame took 3.3 s to arrive.
- Events go through the same outbox, so `sessions:busy` and prompts are not
  held up either.

What a big frame does hold up is the requests BEHIND it on the same socket:
frames arrive in order. A client that must keep polling while it uploads can
open a second connection (a device may hold four).

## 4. Threat model

The new surface: a paired device holding `instruct` on a place can now put
bytes on the host's disk. Before, it could only type a line (`agent_send`),
or, with `write`, write files through `write_file`.

### 4.1 What a device can write, and where

**Closed:** the device chooses only the bytes.
- The place must be in the device's scope, and the act needs `instruct` and a
  fresh step-up on the connection.
- The folder is the place's own `.worktrees-attachments/`. The name is the
  host's: date, random hex, and the extension the bytes earned. `name` never
  reaches a path. `the_stored_name_is_never_made_from_the_users_name` feeds it
  `../../../escape.png`, `/etc/passwd` and a bidi override.
- The bytes are one of five kinds, at most 8 MiB.

So a device cannot choose where a file lands, what it is called, or that it
is a script, a config or an instruction file. Compare `write_file`, which
lets a `write` device write any content to any non-`Forbidden` path in scope.

### 4.2 Symlink and path tricks

A cloned repo can commit anything at `.worktrees-attachments`, and git checks
a committed `120000` blob out with no check. So:
- **A link at the folder is refused, wherever it points.** That includes a
  link INTO the place: `-> .claude/commands` would make every attached `.md`
  a slash command, and `-> .claude/rules` an instruction. Tested by
  `a_committed_link_at_the_folder_is_refused_wherever_it_points`.
- **A non-directory, or a directory another user owns, is refused.**
- **The folder is opened with `O_DIRECTORY|O_NOFOLLOW`** and set to 0700 on
  that handle. Its canonical parent must be the place's canonical root.
- **The place root itself is the resolver's** (canonical, a registered
  project's place). The only component a repo controls is the folder, which
  is checked directly under that root.
- **A link at the final file name is replaced, not followed** (`rename`
  never follows). Names are 64 random bits, so a repo cannot plant one at
  the name anyway.
- **A committed file there is never touched** unless it has our exact name
  shape (`<8 digits>-<16 hex>.<ext>`). Then the 7-day sweep would remove it
  from the working tree, which git restores. Accepted: committing into a
  folder our exclude hides gains nothing.
- **Accepted — a race.** Between the folder's check and the rename, a process
  running as the same user could swap the folder for a link. Only something
  already running as the user can do that, the agent included, and it can
  already write anywhere the user can. A repo cannot: its content is fixed
  at checkout.

### 4.3 Type confusion

**Closed:** the bytes decide, never the name or a MIME type.
- A PNG named `.txt` is a PNG; a text file named `.png` is text. A HEIC named
  `.jpg` is refused as HEIC.
- Executables (ELF, Mach-O), archives and Office files are named and refused.
  Any other binary is refused.
- Text is stored only as `.txt`, `.md`, `.json`, `.csv` or `.diff`. An HTML,
  SVG or shell script is accepted as TEXT and stored as `.txt`, never with an
  extension that makes something on the host run or render it.
- **The host never opens, renders or runs an attachment.** The Docs tab and
  the docs viewer skip dot-folders (`docs::walk`), so an attached `.md` is
  not indexed or rendered there. The agent reads the file with its own
  tools.

**Accepted:** a polyglot (valid PNG bytes that are also something else) is
stored as `.png` and read as an image. Nothing on the host treats it as
anything else.

### 4.4 Disk filling

**Bounded:**
- 8 MiB a file, 50 files and 100 MiB a place.
- The device's 64 MiB burst and 256 MiB an hour.
- 7-day expiry, by the next attach and by the hourly sweep.
- An upload past serve's 16 MiB message closes the connection, as before.

The worst a device can do is ~100 MiB in each place in its scope until the
sweep, at most ~6 GiB a day. Note that `write_file`, which a `write` device
already has, is bounded only by the request rate; attachments are not the
weakest link here.

### 4.5 Prompt injection through a file's contents

**Accepted, as it is for any file a person asks an agent to read.** An
attachment is data the user chose to hand the agent, like a paste.
- A file can carry text that tries to instruct the agent: a screenshot of
  instructions, a PDF with hidden text.
- The host does not filter contents and could not: the agent sees what the
  person sent.

What bounds it:
- The agent's own permissions, unchanged. In Claude's default mode, writes
  and commands still ask.
- A device that can attach can already send any line (`instruct`), so an
  attachment does not widen what a device can tell the agent.
- The line naming the file passes `typed_line_ok`.
- The audit records which bytes went to which place, by hash.

The guidance to users is the paste rule: attach only what you would paste.

### 4.6 The name, the stamp and the log

**Closed:**
- `name` is display-only. It is cleaned of control and invisible characters
  and clipped, and it is never written to disk, the audit log, the place's
  stamp or serve.log.
- The audit holds size, hash and kind (`an_attach_is_recorded_by_size_hash_
  and_kind_never_its_bytes_or_name`). The stamp holds the host's path.

### 4.7 Git

**Closed:** a file git can see is a file an agent's `git add -A` commits and
`git push` publishes, a user's screenshot included.
- The folder is excluded.
- The exclusion is CHECKED (`git check-ignore`) before every write, and a
  repo that un-ignores it is refused (`a_folder_git_would_show_is_refused`).
- The place reads clean in `git status`.

### 4.8 Other local readers

**Accepted:** the folder is 0700 and files are 0600, so other users on the
host cannot read them. Anything running as the user can: other places'
agents, the user's tools. The same holds for any file in a checkout.

## 5. What changed, file by file

- `crates/worktrees-core/src/attach.rs`: new. Decode, sniff, display name,
  store, sweep, exclude, and 12 tests.
- `crates/worktrees-core/src/fsx.rs`: `random_u64` and `atomic_temp_target`
  (so the sweep recognises a killed write's temp without spelling its
  pattern twice).
- `crates/worktrees-core/src/store.rs`: `Declared::last_remote_attach`.
- `crates/worktrees-api/src/agents.rs`: `agent_attach`, and
  `sweep_attachments` for the poll.
- `crates/worktrees-api/src/host.rs`: the sweep at start and hourly.
- `crates/worktrees-cli/src/serve/dispatch.rs`: the row (186 rows; `instruct`
  7).
- `crates/worktrees-cli/src/serve/audit.rs`: the record.
- `crates/worktrees-cli/src/serve/budget.rs`: the `uploads` rate and
  `attach-budget: <s>`.
- `crates/worktrees-cli/src/serve/scope.rs`: `last_remote_attach` in
  `STAMPS`.
- `crates/worktrees-cli/src/serve/tests.rs`: the chain, the live budget, the
  big-frame measurement.
- `app/src/chat.ts`: `ATTACH_*`, `attachLine`, `splitAttached`,
  `attachProblem`, `attachWords`, the attachments in the store, and the
  upload-then-send flow.
- `app/src/ChatPane.tsx`, `ChatPane.css`, `icons.tsx`: the paperclip, paste,
  drop, chips, and files in sent bubbles.
- `app/src/mock/install.ts`: `agent_attach` (§7).
- `app/scripts/chatmirror-check.mjs`, `agent-rows-check.mjs`,
  `docs/proposals/ios-mvp.md`: the mirrors and the "As built" row.

## 6. The desktop chat

- **The paperclip** sits beside the field. It is a plain `<input type=file
  multiple>`, so it reads bytes, not paths, and works the same through the
  remote door.
- **Paste:** an image or file on the clipboard (`clipboardData.files`)
  becomes a chip. Text pastes as before.
- **Drop:** a file dropped on the composer becomes a chip, and the field
  shows a dashed accent while a file is held over it. The app sets
  `dragDropEnabled: false`, so HTML5 drops reach the page.
- **Chips** sit above the field: the name (clipped), the size, the upload's
  state (`uploading`, `uploaded`, `not attached`), and a ✕. The ✕ is a button
  named `Remove <name>`, and Tab and Enter reach it. The list is labelled "N
  files to send".
- **Before any read**, a file over 8 MB or an empty one is refused with
  words, and so is an 11th file.
- **On Send** the line under the field says `Uploading 2 of 3…`, then
  `Sending…`. While the agent works or a prompt waits, attaching is allowed;
  Send keeps its existing rules.
- **A refusal sends nothing** and says why in the words of its token. Example:
  "Not attached: “IMG_0001.heic” is HEIC. Attach PNG, JPEG, GIF, WebP, PDF or
  text. Export it as JPEG or PNG first." The text and the chips stay.
  Uploaded chips keep their path, so a retry does not upload them again.
- **An older host** answers `unknown command: agent_attach`. Then the
  paperclip turns off, titled "This host doesn't support attachments yet:
  update Worktrees on it to attach files", and paste and drop stop taking
  files.

## 7. The mock harness

`app/src/mock/install.ts` answers `agent_attach` as the host does: it decides
by the bytes, gives paths in the host's shape and a real SHA-256, and refuses
with the host's tokens. Its scripted reply to a line with ` [attached: `
says it read the file. Flags:

| Flag | What |
|---|---|
| `?attach=slow` | each upload takes 1.5 s, so the chips' progress shows |
| `?attach=refuse` | `attach-type: HEIC (accepted: …)` |
| `?attach=full` | `attach-files: 50` |
| `?attach=bytes` | `attach-bytes: 104857600` |
| `?attach=budget` | `attach-budget: 42` |
| `?attach=unsafe` | `attach-unsafe` |
| `?attach=old` | `unknown command: agent_attach`, an older host |

`chatmirror-check.mjs` fails if the mock's `agent_attach` refuses with
anything that is not a host token.

**Checked in the harness** (port 1448, Playwright 1.64.1, Chromium and WebKit
at 1280×860, 34 assertions each, all passing):
- the picker, paste and drop each add chips;
- an oversize file is refused before upload;
- a text paste stays text;
- the ✕ is named, reachable by `elementFromPoint`, and removes by keyboard;
- `Uploading 1 of 3…` shows, and the ✕ is disabled while sending;
- the sent bubble shows the text with the files apart;
- attachments alone are sent with the lead-in;
- a HEIC refusal sends nothing and keeps the text;
- an older host turns the paperclip off with words;
- a line one byte too long is refused before upload, and one byte shorter is
  not.

The page logs one xterm `Viewport` error ("dimensions") whenever the chat
opens with its terminal hidden. It happens with nothing attached, so it is
not from this branch.

## 8. Not done

- **A live end-to-end run with a real Claude.** It waits on the user's call,
  because the quota gate refused to start Claude at 75% of its weekly window
  and asks the user before overriding (see the lane's report).
- **A drop from Finder into the real app.** WKWebView should deliver `File`s
  to the page, since `dragDropEnabled` is off, but no one has dropped a file
  onto the running app yet. Neither has a paste from Preview been tried there.
  Both need a hand test (the report lists the steps).
- **The chat shows no thumbnails.** A sent image is a chip with its host name.
  The transcript has only the path, and fetching it back means a read of the
  file through `read_file_base64`, which is a separate decision.
- **No per-message progress within one file.** An upload is one frame, so
  there is no byte-level progress, only "uploading N of M".
- **No HEIC conversion on the host.** The phone converts; on the desktop a
  HEIC is refused with words saying to export it as JPEG or PNG.
- **`agent_attach` from MCP.** Agents can already read and write files; the
  row is for clients.
- **A client-side sniff.** A file the host will refuse (a video, say) is
  uploaded before it is refused. Only the size is checked first.
