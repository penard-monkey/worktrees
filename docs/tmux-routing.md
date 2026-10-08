# Per-project tmux routing — implementation increments

Phase 1a of [nested lanes](proposals/nested-lanes.md#8-tmux-independent-hotfix-routing-prerequisite-later-recovery)
is being implemented in reviewable increments. **The descriptor foundation alone
does not enable routing or migrate any running session.** Existing CLI, MCP and
app callers still use the legacy wrappers until the integration increment.

## Foundation contract

`worktrees_core::tmux_server::TmuxServer` assigns a project endpoint from the
canonical git common directory. Its normal argv is `-L wt-<name>-<hash>` and
`TMUX_TMPDIR=/tmp`; the subprocess's inherited `TMUX` is removed. The PTY adapter
must apply the same `endpoint_args`, `socket_root`, and environment removal.
The hotfix wrapper independently supplies feature-detected `-N` for non-launch
calls. Descriptor assignment never runs tmux.

The fixed hash is FNV-1a 64 over the canonical path's Unix bytes, a NUL separator,
and the namespace's bytes (empty for normal operation). The 20-character readable
label comes from the common directory's parent at first assignment. It does not
read project configuration, prefix, registry labels, or lane slugs.

An immutable assignment lives at `<git-common>/worktrees-tmux/project.json`.
An explicit user `WORKTREES_TMUX_NAMESPACE` selects a separate
`sandbox-<namespace>.json`; only ASCII letters, digits, hyphens and underscores
are accepted (1–32 characters). The namespace participates in the hash. These
are local Git metadata, not tracked or synced project settings.

A matching full-identity claim at
`$XDG_CONFIG_HOME/worktrees/tmux/<endpoint-key>.json`
(or `~/.config/worktrees/tmux/<endpoint-key>.json`) checks for key collisions.
Assignments and claims are published with same-directory atomic no-replace hard
links, so concurrent readers cannot see a partially written JSON document.
Corrupt or conflicting records fail closed. Moving a repository keeps the old
assignment and refuses a new identity; explicit rebinding is not implemented in
this increment. Do not silently delete these records on a discovery error.

`TmuxTarget` carries `(server, Session(name) | Pane(id))` for endpoint-qualified cache and handle
keys. `resolve_lane` accepts endpoint snapshots and candidates obtained from the
existing name/cwd rules. Any candidate, including a provider or shell sidecar,
keeps the lane on that server. Multiple candidate endpoints or an unreachable
endpoint refuse mutation. No candidate selects the project server only after
that endpoint was inspected. `legacy_drained` requires a successful empty list;
unrelated/unmanaged sessions keep the shared endpoint in the inventory.

The integration must distinguish a never-started endpoint from a failed probe
of a known endpoint. `EndpointState::Absent` is not permission to reinterpret
connection failures as an empty server. Persisting known endpoints and drain
completion belongs to that increment; the foundation makes no socket-recovery
claim. Lease, startup serialization and SIGUSR1 remain phase 1b.

## Remaining integration checklist

- [x] Rebased onto the merged non-launch `-N` hotfix (#459). Retain its
  `tmux`/`tmux_launch` wrappers, `no_start_args`, and fake-tmux global-option
  parser while adding descriptor routing.
- [ ] Discover the legacy default socket from the caller's inherited
  `TMUX_TMPDIR` (or `/tmp`) plus `tmux-<UID>/default`, before normalizing the
  project-server environment. A shell using a custom root and a launchd GUI
  otherwise inventory different legacy servers; retain discovered legacy
  endpoints so both surfaces can see them during the drain.
- [ ] Pass the descriptor into `wait_out_refusal`, whose `start-server` probe
  otherwise reaches the default server. On macOS tmux 3.7c, a clean final-session
  exit leaves a socket file; `-N start-server` reports `no server running on …`,
  which the hotfix treats as busy and delays the next launch by ~1s. Phase 1b's
  lease is the durable solution; evaluate stat plus a no-listener check or a
  shorter wait without treating a failed known-endpoint probe as launch consent.
  Canonicalize socket paths before comparing `socket_path()` (`/tmp/…`) with
  tmux's `#{socket_path}` (`/private/tmp/…` on macOS).
- [ ] Supply an actionable repository-move/rebind remedy before enabling routing.
  A copied/restored Git common directory retains the old assignment. Either
  provide a doctor rebind operation or name the exact assignment file to remove
  after checking the old endpoint; do not refer users to a nonexistent command
  or silently discard a possibly live endpoint.
- [ ] Show the effective `WORKTREES_TMUX_NAMESPACE` in `ls`/doctor diagnostics.
  A namespace exported from a shell profile can silently separate CLI sessions
  from a launchd-started app; make that mismatch discoverable.
- [ ] Verify that no lane-owned legacy sessions or sidecars remain before
  reopening it on the project server. A best-effort `kill_shell_sidecars` may
  leave `~term` behind and create permanent ambiguity after a new launch; close
  must report surviving sessions instead of declaring migration complete.
- [ ] Require a descriptor at every `tmux.rs` wrapper; qualify `PaneId`,
  `PaneList`, fingerprints, captures, and terminal/session handles.
- [ ] Enumerate each project/known legacy endpoint once per snapshot. Preserve
  failed probes and retired-legacy state across launches; never kill a shared
  legacy server. Resolve adopted sessions by name/cwd on each endpoint.
- [ ] Route core project/ops/harness/pi/activity calls and all close/remove paths.
  Group captures by endpoint; preserve sidecars through close/reopen draining.
- [ ] Route CLI attach and hints; refuse cross-server switch-client with an
  explicit attach route. Inspect the original inherited client before clearing
  its environment for target operations.
- [ ] Route MCP resolved cross-project targets, status/send/wait and activity
  bindings through the target project's descriptor.
- [ ] Route app snapshots, pane fingerprints, Claude draft chains, Codex/pi
  ticks, PTY attach, wheel/copy-mode, shell cwd/sidecars and cleanup. Qualify
  every cache/handle and frontend activity reference that carries a session.
- [ ] Update mock command shapes and sandbox namespace/cleanup. Guard raw tmux
  subprocess sites outside the wrapper, PTY adapter and version probe.
- [ ] Assert Bats endpoints independently of normalized argv logs; compare
  shipped `ls --json` and MCP `list_places` in isolated fixtures.
- [ ] Observe mock and real sandbox app routes, including cross-project targets,
  legacy sidecars, ambiguity and failed probes. Measure Linux separately.

## Foundation witnesses

Run unit coverage with `cargo test -p worktrees-core tmux_server`.
The opt-in real-server witness is:

```sh
cargo test -p worktrees-core \
  tmux_server::tests::real_servers_keep_same_named_sessions_and_panes_separate \
  -- --ignored --exact
```

It starts two private endpoints with the same session name and pane ID, captures
different content despite hostile inherited tmux environment, and closes one
while the other remains reachable. Every tmux call specifies its descriptor's
endpoint; cleanup closes only its own test sessions and removes their dead
socket files after checking they no longer accept connections. The witness uses tmux 3.2+
(`-N`); it is not an older-version compatibility test or an app witness.
