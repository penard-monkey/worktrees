# kimi 2.8.1 screens, by permission mode

Captured 2026-11-27 with `tmux capture-pane -p` (no `-e`), 160x50, in a linked
worktree of a scratch repo, `KIMI_CODE_TRUST_WORKSPACE=1`, on a private tmux
server. One history row quoting an ssh config line is scrubbed.

| File | Launch | Screen |
|---|---|---|
| `yolo-idle.txt` | `kimi --yolo` | composer, status row starts `Ask When Needed` |
| `yolo-dangerous.txt` | `kimi --yolo` | `Run this command?` with `Dangerous: recursive delete` (an `rm -rf` inside the tree) |
| `yolo-resumed.txt` | `kimi -S <id>` with NO flag, after a `--yolo` resume | composer, still `Ask When Needed`: the session keeps its mode |
| `manual-approval.txt` | `/permission` → Always Ask, mid-session | `Run this command?` for `touch c.txt`; the status row names no mode |
| `auto-idle.txt` | `kimi --auto` | composer, status row starts `Never Ask` |

Findings: `docs/proposals/harness-permission-modes.md`.
