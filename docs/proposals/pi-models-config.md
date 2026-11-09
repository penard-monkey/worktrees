---
title: pi's model hosts, configured in worktrees
---

# pi's model hosts, configured in worktrees

**Decided 2026-11-09; built in `feat/pi-models-settings`.** Measured on
pi 1.5.0 (1.3.0 for the extension too), on a scratch `PI_CODING_AGENT_DIR`.

## The ask

Settings → pi → Models only read `pi --list-models`. To add a model host
(LM Studio, Ollama, any OpenAI-compatible server) or declare a model it
serves, the user hand-edited `~/.pi/agent/models.json`. David asked to do it
from Settings — and then decided *where* it lives: "can we make it so that we
override the models.json with one we own? Then always start pi with that
models.json? … I want to be able to change the config within worktrees and not
affect global pi."

So: worktrees keeps its own hosts, every pi worktrees starts uses them, and
nothing under `~/.pi` is ever written.

## What was measured

pi has no command that edits providers (`pi --help`, `update`, `config`,
`auth`, `mcp`), and no flag or variable that names models.json alone: the
path is always `join(agentDir, "models.json")` (`model-runtime.js:81`,
`agent-session-services.js:59`, `sdk.js:74`). Moving the whole agent dir
would fork `auth.json` and `settings.json`, which pi replaces by rename. An
extension (`-e`, since 0.35.0) calling `pi.registerProvider` (since 0.50.0) is
the one door, and every pi worktrees can launch is past both: launches already
pass `--session-id` (0.76.0).

What pi 1.5.0 does with an extension's providers:

| Question | Answer |
|---|---|
| Loads under `--approve` / `--no-approve` / `--no-extensions` | yes, all three (an explicit `-e` always loads) |
| Defaults | **none**: a model without `maxTokens` crashes `pi --list-models` (TypeError in `formatTokenCount`). The extension fills every field (`contextWindow` 128000, `maxTokens` 16384, as models.json's defaults) |
| Same id in pi's models.json | the extension composes ABOVE it: its model list REPLACES pi's, its `baseUrl` wins, its key falls back to pi's file when it gives none |
| Needs a key | yes: a provider with none is not listed. A host worktrees adds gets its id as a placeholder key; an override gets none and keeps pi's |
| `pi auth check` | ignores `-e` (either position; before `auth` it is a parse error), so it calls our hosts `provider_not_found` |
| Resume (`--session-id`, no `--model`) | with `-e`, back on its model; **without, silently on another model** |
| `/model` in a running pi | lists our hosts; Enter is session-only (Ctrl+S sets the default) |
| A change to the data file | reaches a running pi only at restart (`/model` reloads models.json, not extensions, and no hook fires when it opens) |
| Missing data file / malformed / a host pi rejects | pi starts; missing is silent, the others print one line each |
| `apiKey: "!command"` | not run by `--list-models` or `auth check --no-refresh` (a marker file stayed absent) |

Unmeasured: what a host's `/models` reply can prefill. LM Studio's OpenAI
server was not running; the declare flow uses ids and pi's defaults, and a
prefill is an addition for later.

## The design

- **The data file** is `$XDG_CONFIG_HOME/worktrees/pi-models.json`, the
  registry's directory, so the CLI, serve and the MCP server build one view.
  Its schema is worktrees' own (`pihosts::Owned`): hosts with `baseUrl`,
  `api`, `override` and models — and no key, ever. Edits go through
  `pihosts::edit_at`: one lock, `fsx::write_atomic`, and a refusal when the
  file changed on disk between the read and the swap or does not parse.
- **The extension** (`pihosts_ext.js`) is static code with the data file's
  path set in, written under worktrees' data dir and rewritten whenever its
  content differs. It reads the file once, normalizes, registers, and catches
  everything; it does nothing on a pi without `registerProvider`.
- **One builder.** `pimodels::pi_command` (a process) and
  `pimodels::pane_ext_words` (a pane's launch line) add `-e` and nothing else.
  The launch, the resume and `--list-models` all carry it; `auth check` and
  `pi mcp` go through the same builder without it. `pi-spawn-check.mjs` fails
  on a `pi_bin()` used anywhere else without a stated reason, and a core test
  holds the three argvs to one path.
- **The catalog** sees what a launched pi sees: pi's declared hosts with ours
  laid over them (`effective_declared`), and never asks `auth check` about
  ours.
- **The page** lists worktrees' hosts (editable) and pi's (read-only), each
  with its state, its models and what it serves undeclared. "Override in
  worktrees" copies one of pi's hosts — its models, never its key — because
  declaring one extra model on a bare override would hide the rest; "Remove
  override" hands it back. Writes are one `HostConsole` row, `pi_host_edit`,
  whose argument has no key field and refuses one by name.

## What the user gives up

A pi started by hand reads only pi's own models.json. A session on one of
worktrees' models, resumed by hand outside worktrees, starts on another model
without a warning. The page says so in one sentence.
