# Harness updates — findings and design (2026-11-02)

## Read-only measurements

On this macOS host, `claude --version` is `2.1.317 (Claude Code)` and
`~/.local/bin/claude` links to `~/.local/share/claude/versions/2.1.317`:
Anthropic's native install. `codex --version` is `codex-cli 0.160.1`, but
the executable is inside
`~/.vscode/extensions/openai.chatgpt-26.930.61225-darwin-arm64/bin/macos-aarch64/`.
It belongs to the VS Code extension; installing an npm or Homebrew Codex would
not update that executable. `kimi` and `pi` are absent from PATH. `npm ls -g
--depth=0 --json` lists neither tool, and Homebrew lists no Codex cask or
formula here. These probes did not read credentials or modify an install.
The fixed feeds answered Claude `2.1.317`, Codex npm and Homebrew `0.166.0`,
Kimi Code `2.4.1`, and Pi `1.5.0` on this date. Thus the Codex bundled here is
behind the public release, but must be updated through its extension.

| Tool and install | Recognition from the resolved executable | Latest version source (read only) | Fixed update argv |
| --- | --- | --- | --- |
| Claude native | `~/.local/bin/claude` links into `~/.local/share/claude/versions/<version>` | `@anthropic-ai/claude-code` npm latest; cross-channel comparison is an approximation | `claude update` (the resolved binary) |
| Claude global npm | Executable links into `@anthropic-ai/claude-code`, and its `prefix/bin` agrees with `npm prefix -g` | npm latest | `npm install -g @anthropic-ai/claude-code@latest` |
| Codex global npm | Executable links into `@openai/codex`, with the same prefix check | npm latest | `npm install -g @openai/codex@latest` |
| Codex Homebrew cask | `bin/codex` resolves inside `Caskroom/codex`, and the prefix/cask registration agree | `formulae.brew.sh/api/cask/codex.json` | `brew upgrade --cask codex` |
| Codex standalone or editor bundle | Any other binary, including this host's VS Code copy | npm latest for a version hint | none: update it the way it was installed |
| Kimi Code native | Executable is `~/.kimi-code/bin/kimi`, the official install script's default | `@moonshot-ai/kimi-code` npm latest | `kimi upgrade -y` (the resolved binary) |
| Kimi Code global npm | Executable links into `@moonshot-ai/kimi-code`, with the prefix check | npm latest | `npm install -g @moonshot-ai/kimi-code@latest` |
| Legacy Python Kimi (`kimi-cli`) | Python/uv shim identifying `kimi_cli` | no maintained latest; identify version only | none: migrate through Kimi's instructions |
| Pi global npm | Executable links into `@earendil-works/pi-coding-agent`, with the prefix check | npm latest | `npm install -g --ignore-scripts @earendil-works/pi-coding-agent@latest` |
| Pi official managed install | Executable resolves to the official `~/.pi/agent/bin/pi` launcher, with `install/managed-install.json` and `install/current-version`; no custom self-update override | npm latest | `pi update` (the resolved launcher) |
| Other Pi installs | Any unrecognised executable, including a standalone binary or another package manager | npm latest for a version hint | none: update it the way it was installed |

The npm prefix check is essential: an arbitrary `node_modules` segment (or a
different Node installation's global bin) is not authority to mutate the
package used by this app. Only the executable worktrees would launch is
reported. A hidden second install is not an update target. Native Kimi under
a custom `KIMI_INSTALL_DIR`, and Codex standalone, have no safe ownership
proof from PATH alone and remain manual. On Linux, there is normally no
Homebrew cask; the other recognitions and commands are the same.
Pi's `PI_SELF_UPDATE_COMMAND` and `selfUpdateCommand` setting can replace
`pi update` with arbitrary argv. When either is present, the method is treated
as unknown and the app shows no update button.

Current vendor documentation: [Claude installation and manual update](https://docs.anthropic.com/en/docs/claude-code/getting-started),
[Codex npm and Homebrew commands](https://developers.openai.com/cookbook/examples/codex/using_goals_in_codex),
[Kimi Code installation](https://moonshotai.github.io/kimi-code/en/guides/getting-started),
[Kimi upgrade semantics](https://moonshotai.github.io/kimi-code/en/reference/kimi-command),
[legacy Python migration](https://moonshotai.github.io/kimi-code/en/guides/migration),
[Pi CLI update semantics](https://pi.dev/docs/latest/cli), and
[Pi's package rename](https://pi.dev/changelog/2026/5/7/pi-has-a-new-home).
The Kimi installer was also inspected read only: it defaults to
`$HOME/.kimi-code/bin/kimi`. Pi's installer marks managed installs with
`managed-install.json`; its PATH entry is a link to `~/.pi/agent/bin/pi`,
not directly into `install/releases`.

## Design

Extend the existing `check_update` response with four harness rows, so the
manual button and hourly poll share one throttle and one result. A row names
the installed version, latest version, install method, whether a newer
version exists, and a display-only command label. Network probes use fixed
HTTPS URLs on `registry.npmjs.org` and `formulae.brew.sh`, a short timeout,
and a short cache. The network lookups use no package manager config; local
ownership probes ask npm and Homebrew for their active prefixes.
Unknown/uninstalled states are visible; an unavailable feed is not reported
as up to date. Version comparison accepts numeric dotted releases and treats
an unparseable version as unknown.

`update_harness` accepts one of four fixed tool names. The host re-detects the
install method at execution time and selects a constant argv; it never runs
an argv supplied by the client or a repo. The row requires `admin` and is
audited. One update per tool runs at a time. `childenv::command`, the GUI
login-shell PATH fixup, and a process deadline apply to both checks and
updates. Output is capped before returning it to the UI and is never copied
to the audit log. After the process, the host re-probes the version.

An already-running process keeps the code it loaded. New sessions see the
updated CLI; npm replaces package files on disk, so the UI asks the user to
finish or restart active sessions for predictable behavior. Native Claude
keeps versioned executables, so existing sessions keep their version until
restarted. Kimi native stages a replacement for its next launch. Pi's
managed updater may adjust its managed Node and package tree, so the same
restart guidance applies. The app does not close or restart agents itself.

## Threat model

A paired device with an `admin` grant and step-up can now trigger one of the
four host-global update actions. That is powerful: it changes executables
used by future agent sessions. The owner deliberately grants `admin`, the
host uses fixed vendor commands and current-install ownership checks, and
the serve resolver, step-up gate, and hash-chained audit record each request
and result. A device cannot name a package, URL, path, shell line, or version.
The same row is available to the local console. Read-only checks need only
`read` and reveal versions and install method, but no credentials.
