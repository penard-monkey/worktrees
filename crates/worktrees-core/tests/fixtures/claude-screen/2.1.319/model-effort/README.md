# Claude Code 2.1.319 — model list and "Switch model?"

Captured 2026-11-06 from a scratch Claude launched `claude --name e2e-lane`
(`DISABLE_AUTOUPDATER=1`) on a private tmux server, 120×40, after one
exchange (prompt cache warm). `*.e.txt` is `capture-pane -e -p` (what the host
reads), the other `capture-pane -p`.

- `list-on-sonnet`: `/model` with Sonnet 5.5 in effect (`4. Sonnet 5.5 ✔`).
  Differences from 2.1.318: a sixth row, `6. More models  Show 8 more`
  (replacing `… +N models`), and a tmux hint on the sheet's top edge
  (`▔▔ tmux focus-events off · … ▔`). The reader still reads the list, and
  **reads `More models` as a model choice** — not handled here.
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
