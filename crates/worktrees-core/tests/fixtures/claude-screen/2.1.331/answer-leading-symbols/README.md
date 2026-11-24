# What a leading `@ ! # /` does in an answer field (Claude 2.1.331)

Measured 2026-11-24 on Claude Code 2.1.331 (`claude --version`), launched as
worktrees launches it (`--name ans-lane`, `DISABLE_AUTOUPDATER=1`), in a scratch
folder, on its own tmux server (`tmux -L wtans`, killed by name afterwards).
Text was PASTED with `paste-buffer -p`, as `tmux::paste_into` does, into each
field, the screen captured, then `C-u`. Nothing was submitted (Enter was never
pressed on `/clear`, `!ls` or any other).

| Field | `@here` | `!ls` | `#2. Also` | `/clear` |
|---|---|---|---|---|
| AskUserQuestion free-text row ("Type something.") | text | text | text | text |
| Preview question's notes field (`n`) | text | text | text | text |
| Permission prompt's Tab-to-amend field (on "No") | text | text | text | text |

"text" means: the characters landed in the field and nothing else changed — no
file menu, no mode change (the footer kept its mode), no slash menu, no
shell-mode prompt, no memory prompt. A field the prompt opened is not the
composer, so the composer's reasons for refusing these (`!` shell, `#` memory,
`@` file picker) do not apply there.

`/` also only inserted text. It stays refused (`harness::typed_answer_ok`) on
the owner's "maybe": a slash command is not an answer, and the line would
read as one if the field were ever the composer.

**Codex was NOT measured.** Its first launch in a scratch folder asks to trust
it and says "Your trust decision will be saved" — a write to the user's
`~/.codex/config.toml`, which AGENTS.md leaves to the user. So
`harness::typed_answer_for` keeps `@ ! #` refused for any harness but Claude,
in words that say the field is not measured. Measure Codex's question notes
(0.172.0 installed here) and lift that line when someone may answer its trust
prompt.

Files: `question-free-text-hash.txt`, `permission-amend-at.txt`,
`preview-notes-bang.txt` — the screens after the paste (tails only).
