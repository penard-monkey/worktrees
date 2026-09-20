#!/usr/bin/env python3
"""Drive the real `worktrees mcp` over stdio and check the resource surface.

The unit tests in `mcp.rs` cover the pure rules (uri sanitising, collision
resolution, the membership signal, the advertised capability). They cannot
cover the part that is actually risky: a SECOND thread writing to the same
newline-delimited stdout as the request loop, and a notification that must not
be sent before the client says it is initialized. A half-written line there is
a parse error the client never recovers from, and nothing else in the suite
would see it.

Usage:  python3 scripts/mcp-resources-check.py [path/to/worktrees]
Exits non-zero on the first failed expectation.
"""
import json, os, shutil, subprocess, sys, tempfile, threading, time

BIN = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else "target/release/worktrees")
TMP = tempfile.mkdtemp(prefix="wt-mcp-check-")
REPO = os.path.join(TMP, "repo")
WAIT = 12.0  # generous: the watcher's period is ~2s plus a pid-derived jitter

fails = []
def check(ok, what):
    print(f"  {'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        fails.append(what)

def guard(d):
    """A throwaway git script needs a guard, because `git -C ''` means HERE."""
    if not d or not TMP or not d.startswith(TMP):
        sys.exit(f"refusing to operate on {d!r}")
    return d

def git(*a):
    subprocess.run(["git", "-C", guard(REPO), *a], check=True, stdout=subprocess.DEVNULL,
                   env={**os.environ, "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t",
                        "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@t"})

try:
    os.makedirs(guard(REPO))
    git("init", "-q", "-b", "main")
    git("commit", "-q", "--allow-empty", "-m", "x")
    os.makedirs(os.path.join(guard(REPO), ".worktrees", "alpha"))

    proc = subprocess.Popen([BIN, "mcp"], cwd=guard(REPO), stdin=subprocess.PIPE,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                            text=True, bufsize=1)
    lines, bad = [], []
    def reader():
        for raw in proc.stdout:
            raw = raw.strip()
            if not raw:
                continue
            try:
                lines.append(json.loads(raw))
            except json.JSONDecodeError:
                bad.append(raw)  # a torn line: the failure this script exists for
    threading.Thread(target=reader, daemon=True).start()

    def send(o):
        proc.stdin.write(json.dumps(o) + "\n")
        proc.stdin.flush()

    def reply(n, timeout=5.0):
        end = time.time() + timeout
        while time.time() < end:
            for m in lines:
                if m.get("id") == n:
                    return m
            time.sleep(0.05)
        return None

    def notifications(since):
        return [m for m in lines[since:]
                if m.get("method") == "notifications/resources/list_changed"]

    print("handshake")
    send({"jsonrpc": "2.0", "id": 1, "method": "initialize",
          "params": {"protocolVersion": "2025-11-25"}})
    init = reply(1)
    caps = (init or {}).get("result", {}).get("capabilities", {})
    check(caps.get("resources", {}).get("listChanged") is True,
          "initialize advertises resources.listChanged")

    # Before `notifications/initialized`, the server must stay silent even
    # though the place set is changing under it.
    print("silence before the client is initialized")
    mark = len(lines)
    os.makedirs(os.path.join(guard(REPO), ".worktrees", "early"))
    time.sleep(WAIT / 2)
    check(not notifications(mark), "no list_changed before notifications/initialized")

    send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    # …and it must not LOSE it either: the watcher does not advance its baseline
    # while it is muted, so the change made during the handshake is still owed.
    # Draining it here is also what keeps the `beta` assertion below honest —
    # otherwise this deferred notification satisfies it and a watcher that fires
    # exactly once, ever, would pass.
    end, deferred = time.time() + WAIT, False
    while time.time() < end and not deferred:
        deferred = bool(notifications(mark))
        time.sleep(0.1)
    check(deferred, "a change made mid-handshake is deferred, not dropped")

    print("resources/list")
    send({"jsonrpc": "2.0", "id": 2, "method": "resources/list"})
    res = (reply(2) or {}).get("result", {}).get("resources", [])
    uris = [r["uri"] for r in res]
    check("place://main" in uris, f"the main checkout is a resource ({uris})")
    check(all(r["uri"].split("://")[-1][-1].isalnum()
              or r["uri"].split("://")[-1][-1] == "_" for r in res),
          "every uri ends on a word character (the submit regex's trailing \\b)")
    check(all(len(r["description"]) <= 60 for r in res),
          "every description fits the client's 60-char clip")

    print("resources/read")
    send({"jsonrpc": "2.0", "id": 3, "method": "resources/read",
          "params": {"uri": "place://alpha"}})
    got = reply(3)
    body = json.loads(got["result"]["contents"][0]["text"]) if got and "result" in got else {}
    check(body.get("slug") == "alpha", "read resolves the uri back to its slug")
    check(bool(body.get("place", {}).get("path")), "the body carries an absolute path")
    check(bool(body.get("place", {}).get("tmux_session", {}).get("name")),
          "…and the tmux session name, which is the messaging address")
    check("snapshot_at_epoch" in body, "…and says when it was taken")

    send({"jsonrpc": "2.0", "id": 4, "method": "resources/read",
          "params": {"uri": "place://ghost"}})
    check((reply(4) or {}).get("error", {}).get("code") == -32002,
          "an unknown uri is -32002 (resource not found), not -32602")

    print("the push")
    # Taken AFTER the deferred notification above has been seen, so nothing but
    # `beta` can satisfy this.
    mark = len(lines)
    os.makedirs(os.path.join(guard(REPO), ".worktrees", "beta"))
    end, fired = time.time() + WAIT, False
    while time.time() < end and not fired:
        fired = bool(notifications(mark))
        time.sleep(0.1)
    check(fired, "a new place pushes notifications/resources/list_changed")

    print("and the silence that makes it usable")
    mark = len(lines)
    with open(os.path.join(guard(REPO), ".worktrees", "alpha", "f.txt"), "w") as fh:
        fh.write("work happening inside a worktree")
    time.sleep(WAIT / 2)
    check(not notifications(mark), "work INSIDE a worktree pushes nothing")

    proc.stdin.close()
    proc.terminate()
    check(not bad, f"every stdout line parsed as JSON (torn lines: {bad[:2]})")
    # stderr is where human-facing output BELONGS ("stdout carries protocol
    # only"), so the check is not that it is empty — it is that nothing
    # UNEXPECTED is on it. With the temporary debug-log banner gone the server
    # prints nothing at all on a clean run, so any line here is a finding.
    stray = [l for l in (proc.stderr.read() or "").strip().splitlines() if l.strip()]
    check(not stray, f"nothing unexpected on stderr: {stray[:2]}")
finally:
    shutil.rmtree(TMP, ignore_errors=True)

print()
if fails:
    print(f"FAILED ({len(fails)}): " + "; ".join(fails))
    sys.exit(1)
print("mcp-resources-check: all good")
