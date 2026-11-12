# Claude Code 2.1.319 — model list and "Switch model?"

Captured 2026-11-06 from a scratch Claude launched `claude --name e2e-lane`
(`DISABLE_AUTOUPDATER=1`) on a private tmux server, 120×40, after one
exchange (prompt cache warm). `*.e.txt` is `capture-pane -e -p` (what the host
reads), the other `capture-pane -p`.

- `list-on-sonnet`: `/model` with Sonnet 5.5 in effect (`4. Sonnet 5.5 ✔`).
  Differences from 2.1.318: a sixth row, `6. More models  Show 8 more`
  (replacing `… +N models`), and a tmux hint on the sheet's top edge
  (`▔▔ tmux focus-events off · … ▔`). `More models` is NOT a model (see
  below); the reader leaves it out of the choices.
- `list-on-opus`: the same list with the highlight moved to `2. Opus 5.5`.
- `switch-confirm`: `s` on Opus 5.5, pressed by hand at the pane's keyboard
  (`tmux send-keys`), brought up "Switch model? / Your next response will be
  slower and use more tokens / … Switching to Opus 5.5 means the full history
  gets re-read. / `❯ 1. Yes, switch to Opus 5.5` / `2. No, go back`" — the
  same rows as 2.1.318's (`../../2.1.318/model-effort/`). The host reads this
  dialog only beside its own record of the `s` it pressed
  (`prompt::read_switch`; AGENTS.md).

Real runs through `agents::tests::e2e_switch_model_through_a_real_claude`
(`#[ignore]`) answered the dialog's Yes through the real rows twice
(`Opus 5.5`, `Sonnet 5.5`): `Set model to X for this session only`.

## `More models` (measured 2026-11-07, 120×40, `claude --name e2e-more`)

A scratch Claude on the real `~/.claude/settings.json` (a `CLAUDE_CONFIG_DIR`
copy asks to log in), which was hashed (SHA-256) after every key and never
changed. Keys were pressed on this row only on a fresh capture showing the
highlight on it.

- `list-on-more`: highlight on `6. More models  Show 8 more`. No effort row,
  and the footer is `Enter to expand · Esc to cancel` — not the model list's
  footer, so the reader does not read this screen as a model list at all.
- `list-on-more-after-s`: after `s` — unchanged (`s` is inert here). Space is
  inert too (only the tmux hint row differs).
- `list-more-expanded`: after Enter — expanded IN PLACE, nothing saved: row 6
  reads `Hide`, rows 7-14 follow (`7. Haiku 4.5` … `14. Sonnet 4.6`), the
  window scrolls (`↑ 5. Haiku 5.5` on top), footer `Enter to collapse`.
- `list-more-expanded-on-7`: highlight moved to row 7: the usual footer and
  effort row (`○ Effort not supported for Haiku 4.5`). Row 6 sits between 5
  and 7, so keys to a model past it count it (`prompt::claude_models` keeps
  it in `order`, out of `choices`). Up then Enter on `Hide` collapses back
  to `list-on-more`'s screen.
