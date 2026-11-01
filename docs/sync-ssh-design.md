# Sync over SSH: a design for a follow-up lane

Status: design only. Nothing here is implemented. Hubs today are mounted
volumes: removable drives, internal volumes and network shares under
`/Volumes`, classified by `worktrees_core::synchub`. This note covers a hub
that is ANOTHER MAC reached over SSH (Remote Login), with no shared drive in
between.

## 1. Naming and storing an SSH hub

An SSH hub is a `user@host:path` triple. Its id in `sync-hubs.json` is
`ssh://user@host/abs/path`: the scheme makes it disjoint from `uuid:`,
`smb://`… and `path:`, and the user stays IN the id. This is unlike a share,
whose id drops the user. Two accounts on one Mac are two hubs, because they
have two home directories and two trust decisions.

- **Declared, never discovered.** An SSH hub is added by an explicit act:
  `worktrees sync hubs --add ssh://david@studio.local/Users/david/hub`, or the
  dashboard's "Add a Mac…". There is no Bonjour scan for `_ssh._tcp`. A
  network that answers is not a network the user chose to sync to.
- `Remembered` grows `kind: "ssh"` plus `host`, `user`, `path` fields, so the
  dashboard shows it without parsing the id. `HubKind::Ssh` is never
  auto-picked. The rule is the one `auto_pick` already applies to shares.
- `host` is what the user typed: a name or an address. It is NOT resolved and
  stored. A Mac on DHCP moves, and `ssh` resolves the name each time, the way
  it does for the user's own terminal.
- `path` must be absolute, and it may not contain `..`, a NUL or a newline.
  Both the CLI and the api refuse anything else with the reason.

## 2. Reading manifests without a remote install

The dashboard needs every `<hub>/<name>/.worktrees-sync.toml`. The remote Mac
is not required to have worktrees installed, so the listing is plain POSIX
over one connection:

```
ssh -o BatchMode=yes -o ConnectTimeout=5 -- user@host \
    'cd -- "$1" 2>/dev/null || exit 3; for f in */.worktrees-sync.toml; do [ -f "$f" ] || continue; printf "%s\0" "$f"; cat -- "$f"; printf "\0"; done' sh '/abs/path'
```

- The path reaches the remote shell as `$1`, never spliced into the command
  text. The remote side runs `sh -c '<fixed script>' sh <path>`. The only
  client value is an argument, and the script quotes it.
- The output is NUL-framed (name, body). Each body goes through
  `sync::parse_manifest`, the same strict, `deny_unknown_fields` and
  data-only parser (ADR 0001). A body over 64 KiB is dropped with a reason.
- Exit 3 means "the hub path does not exist". It is shown as such and is not
  an error.
- One round trip per dashboard read, under a deadline (`proc::run_deadline`,
  10 s). A Mac that is asleep reads as "not connected", with the remembered
  list dimmed, exactly like an unplugged drive.

## 3. The rsync argv, and keeping client values out of option position

The transfer is today's `Job`, with the destination (push) or source (pull)
spelled `user@host:/abs/path/<name>/<name>/`.

```
rsync -a --delete --exclude-from=<tmp> --backup --backup-dir=<abs> \
      -e 'ssh -o BatchMode=yes -o ConnectTimeout=10' \
      [--info=progress2,name1] -- <src>/ <user@host:/abs/…>/
```

- **R3 (AGENTS.md: a client's string is never a flag).** Every flag is a
  `&'static str`. The src and dst come after `--`, which rsync honours. The
  `-e` value is a constant: user, host and path never appear inside it, so no
  client value is ever parsed by ssh as an option. In particular a host that
  starts with `-` (`-oProxyCommand=…`) cannot become one. It is still refused
  at the boundary with the same rule as `safe_arg`: no leading `-`, and no
  whitespace or control characters in user or host.
- rsync splits `user@host:path` at the FIRST `:`, so a path containing `:`
  is refused rather than quoted.
- `--protect-args` (`-s`) on rsync 3.x keeps the remote shell from
  word-splitting the path. openrsync has no `-s`, so on openrsync a path
  with spaces is refused with "install rsync 3".
- The backup dir for a push lives on the REMOTE side, at the same relative
  place it has on a drive. rsync resolves a relative `--backup-dir` against
  the destination, so pass it relative (`../.worktrees-sync/backups/<stamp>/<name>`)
  instead of building a remote absolute path by hand.
- The manifest write after a push is a second `ssh … 'cat > "$1.tmp" && mv -- "$1.tmp" "$1"' sh <path>`
  with the rendered manifest on stdin. It is atomic on the far side, and
  again no client value is in the script.
- `WORKTREES_RSYNC` keeps pinning rsync. A new `WORKTREES_SSH` pins ssh for
  tests, and core passes it as the `-e` program.

## 4. Host-key trust, and a Mac with no Remote Login

- **Never `StrictHostKeyChecking=no`, never `accept-new` on the user's
  behalf.** With `BatchMode=yes`, an unknown or changed host key is an error,
  and ssh says so on stderr (`Host key verification failed`). The dashboard
  maps it to: "studio.local is not a known host — connect once from Terminal
  (`ssh david@studio.local`) and check its fingerprint". The user's own
  `known_hosts` is the trust store, as it is for git over ssh.
- Authentication is the user's ssh agent or keys. `BatchMode` forbids a
  password prompt, which would hang a GUI with no tty. "Permission denied
  (publickey,…)" maps to "set up a key for david@studio.local".
- No Remote Login: port 22 refuses (`Connection refused`) or times out. The
  message names the switch: System Settings → General → Sharing → Remote
  Login, on the OTHER Mac. A timeout reads as "not connected (asleep or off
  the network)", never as a hard error.
- No ssh process ever inherits a tty from serve. The `ssh` is spawned through
  `childenv::command`, with stdin `/dev/null` except for the manifest write.

## 5. How the dashboard shows it

- A third hub kind in the list. It has a Mac icon instead of the drive, and
  the `user@host` is in the row. Free space comes from `df -Pk -- "$1"` in
  the same listing round trip, or is shown as blank.
- Connected means the last listing answered within its deadline. The listing
  runs when the dashboard opens and when the user presses refresh, never on
  the 3 s poll. Waking a sleeping Mac every 3 s is not acceptable.
- A transfer's progress uses the same `--info=progress2` stream. A
  half-finished push over a dropped link leaves the previous manifest in
  place, because the manifest is written last, so the hub still describes
  the last COMPLETE push.
- The batch runs SSH jobs in the same one-at-a-time loop as drive jobs. A
  slow link holds the queue, and that is the price of "never two rsyncs onto
  one hub".

## 6. Tests a follow-up needs

1. **argv shape** (unit, no network): for push and pull, the assembled argv
   has every flag before `--`, the `-e` value equals the constant, and
   `user@host:/p/` sits after `--`. A host `-oProxyCommand=x` and a user with
   a space are refused before any argv exists.
2. **listing parser** (unit): NUL-framed output with an unknown key
   (refused), an oversized body (dropped) and a non-UTF-8 name (dropped), each
   with a reason; exit 3 → "no such path".
3. **fake ssh shim** (bats): `WORKTREES_SSH` points at a script that logs its
   argv and runs the remote command locally under `sh -c` in a temp dir.
   `sync push`/`pull`/`status` against an `ssh://` hub then run end to end
   with no network. Assert the logged argv, never a real host.
4. **error mapping** (unit): the stderr lines for an unknown host key,
   publickey denial, connection refused and timeout each map to their
   message, and none is retried.
5. **no auto-pick** (unit): `auto_pick` ignores `HubKind::Ssh`, as it does
   shares.
6. **id round trip** (unit): `ssh://user@host/abs/path` parses back to its
   triple, and a relative path, `..`, or a `:` in the path is refused.
7. **Manual check** (David, real Macs): Remote Login on the iMac, a key in the
   agent, push from the MacBook, pull on the iMac. Then turn Remote Login off
   and confirm the message; change the iMac's host key and confirm the
   refusal.
