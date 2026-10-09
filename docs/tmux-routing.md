# Per-project tmux routing — implementation increments

Phase 1a of [nested lanes](proposals/nested-lanes.md#8-tmux-independent-hotfix-routing-prerequisite-later-recovery)
is split into a descriptor foundation (#461) and a runtime integration.
The integration routes CLI, MCP and app operations through the descriptor;
legacy sessions move only after a successful close and reopen.

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

## Integration checklist

- [x] Rebased onto the merged non-launch `-N` hotfix (#459). Retain its
  `tmux`/`tmux_launch` wrappers, `no_start_args`, and fake-tmux global-option
  parser while adding descriptor routing.
- [x] Discover the legacy default socket from the caller's inherited
  `TMUX_TMPDIR` (or `/tmp`) plus `tmux-<UID>/default`, before normalizing the
  project-server environment. A shell using a custom root and a launchd GUI
  otherwise inventory different legacy servers; retain discovered legacy
  endpoints so both surfaces can see them during the drain.
- [x] Pass the descriptor into `wait_out_refusal`, whose `start-server` probe
  otherwise reaches the default server. On macOS tmux 3.7c, a clean final-session
  exit leaves a socket file; `-N start-server` reports `no server running on …`,
  which the hotfix treats as busy and delays the next launch by ~1s. Phase 1b's
  lease is the durable solution; evaluate stat plus a no-listener check or a
  shorter wait without treating a failed known-endpoint probe as launch consent.
  Canonicalize socket paths before comparing `socket_path()` (`/tmp/…`) with
  tmux's `#{socket_path}` (`/private/tmp/…` on macOS).
- [x] Supply an actionable repository-move/rebind remedy before enabling routing.
  A copied/restored Git common directory retains the old assignment. Either
  provide a doctor rebind operation or name the exact assignment file to remove
  after checking the old endpoint; do not refer users to a nonexistent command
  or silently discard a possibly live endpoint.
- [x] Show the effective `WORKTREES_TMUX_NAMESPACE` in `ls`/doctor diagnostics.
  A namespace exported from a shell profile can silently separate CLI sessions
  from a launchd-started app; make that mismatch discoverable.
- [x] Verify that no lane-owned legacy sessions or sidecars remain before
  reopening it on the project server. A best-effort `kill_shell_sidecars` may
  leave `~term` behind and create permanent ambiguity after a new launch; close
  must report surviving sessions instead of declaring migration complete.
- [x] Require a descriptor at every `tmux.rs` wrapper; qualify `PaneId`,
  `PaneList`, fingerprints, captures, and terminal/session handles.
- [x] Enumerate each project/known legacy endpoint once per snapshot. Preserve
  failed probes and retired-legacy state across launches; never kill a shared
  legacy server. Resolve adopted sessions by name/cwd on each endpoint.
- [x] Route core project/ops/harness/pi/activity calls and all close/remove paths.
  Group captures by endpoint; preserve sidecars through close/reopen draining.
- [x] Route CLI attach and hints; refuse cross-server switch-client with an
  explicit attach route. Inspect the original inherited client before clearing
  its environment for target operations.
- [x] Route MCP resolved cross-project targets, status/send/wait and activity
  bindings through the target project's descriptor.
- [x] Route app snapshots, pane fingerprints, Claude draft chains, Codex/pi
  ticks, PTY attach, wheel/copy-mode, shell cwd/sidecars and cleanup. Qualify
  every cache/handle and frontend activity reference that carries a session.
- [x] Update mock command shapes and sandbox namespace/cleanup. Guard raw tmux
  subprocess sites outside the wrapper, PTY adapter and version probe.
- [x] Assert Bats endpoints independently of normalized argv logs; compare
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

## Runtime inventory and failure handling

`tmux_route::Routes` persists discovered legacy socket paths in the user config
`tmux` directory. It inspects the inherited default root and non-project `TMUX`
socket, then shares that inventory with later GUI/CLI processes. A sandbox
namespace skips legacy discovery entirely. Each snapshot batches by endpoint.
An endpoint confirmed empty is retired from legacy polling; unrelated sessions
keep it in the inventory. Sessions started later by older software on a retired
legacy endpoint are outside this completed drain.

A successful nonempty probe records `known-<socket-hash>`. A failed known probe
is unknown and blocks mutation even if its socket has vanished. A never-seen,
missing endpoint may start normally. Successful last-session close records the
expected empty state, permitting reopen despite tmux's leftover socket file.
Neither a failed probe nor a best-effort cleanup grants that permission.
No shared legacy server is killed. Manual recovery diagnostics name the socket
and marker; remove them only after verifying the old server is stopped.

`ls --json` and MCP place snapshots add `tmux_session.server`, optional
`namespace`, and optional `error`. Existing fields retain their meaning on a
healthy endpoint. On routing failure lifecycle is `unknown`; the app keeps the
place visible under Unavailable and displays the error. Human `ls` uses `?`.
The app sends a place root with terminal/plan operations and resolves its endpoint
in Rust. PTYs and pane handles retain that endpoint for their lifetime.

Sandbox cleanup enumerates only locally claimed keys matching its namespace.
If an endpoint cannot be inspected it retains sandbox state and exits with the
endpoint error; verify a stopped endpoint before manually removing that state.

## Integration evidence and remaining manual gate

- Six fake-tmux routing regressions cover independent same-basename clones,
  legacy reuse/drain, duplicate refusal, failed-known refusal, sidecar-only
  legacy lanes, and cross-project MCP send with repeated `%0` identifiers.
- Mutation runs were red for legacy routing, ambiguity refusal, failed-probe
  handling, pi endpoint identity, and the raw-subprocess audit.
- Private real tmux 3.7c on macOS: repeated session/pane names route independently;
  closing one clone preserves the other; legacy close/reopen preserves an
  unmanaged session; deleting a known socket refuses relaunch while its original
  server remains alive. Existing private real-tmux Bats checks pass (5/5).
- Shipped v0.40.0 JSON comparison in isolated fixtures changes only additive
  endpoint metadata, for both `ls --json` and MCP `list_places`.
- Browser mock at `?tmuxrouting`: identical session names attach using distinct
  project roots; an unavailable place stays in navigation, displays its endpoint
  error and does not attach or relaunch when selected.
- Real sandbox app routing remains a manual gate: select `same` under the
  isolated `alpha` and `beta` projects and verify their ALPHA/BETA terminal
  markers, then close one and verify the other remains usable. AGENTS.md forbids
  scripted input to the real app. Linux real-tmux behavior remains unmeasured.

Phase 1b leases, startup serialization and socket recovery are not implemented.

### Baseline renderer error

The sandbox's xterm renderer-dimensions error also reproduces without routing
integration on main `c360e08`. In the browser mock, Home → catalog-import →
Enter throws `Cannot read properties of undefined (reading 'dimensions')` at
`@xterm/xterm`'s `RenderService.dimensions` getter (bundled line 1843). The getter
reads an absent renderer. This establishes baseline reproduction; the precise
lifecycle cause remains uninvestigated. It does not replace the manual routing
witness above.

Foreign read-only MCP status/wait responses withhold absolute socket paths and
paths embedded in diagnostics. Without tmux, activity remains unknown and an idle
wait can time out; an unreachable endpoint is never reported as idle.
