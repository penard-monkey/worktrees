#!/usr/bin/env python3
"""Replace a running server's executable as data; never execute the replacement."""
import json
import os
from pathlib import Path
import queue
import shutil
import subprocess
import sys
import tempfile
import threading

binary = Path(sys.argv[1]).resolve()
with tempfile.TemporaryDirectory(prefix="wt-mcp-stale-") as tmp:
    target = Path(tmp) / "worktrees"
    shutil.copy2(binary, target)
    proc = subprocess.Popen([str(target), "mcp", "--mutations"], stdin=subprocess.PIPE,
                            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    replies = queue.Queue()
    def read():
        for line in proc.stdout:
            replies.put(json.loads(line))
    threading.Thread(target=read, daemon=True).start()
    def request(method, params):
        proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}) + "\n")
        proc.stdin.flush()
        return replies.get(timeout=15)["result"]
    def call():
        # Invalid input avoids mutation but still exercises the tool response.
        return request("tools/call", {"name": "create_worktree", "arguments": {"branch": "--invalid"}})
    def replace(data):
        new = target.with_suffix(".new")
        new.write_bytes(data)
        os.replace(new, target)
    try:
        version = request("initialize", {"protocolVersion": "2025-06-18"})["serverInfo"]["version"]
        original = call()
        assert original["isError"] is True
        assert len(original["content"]) == 2, original
        capability = original["content"][-1]["text"]
        assert "create_worktree.provider accepts claude or codex" in capability, capability
        assert "full session restart is unverified" in capability, capability
        same = b"\0WORKTREES_CLI_VERSION=" + version.encode() + b"\0"
        assert same in binary.read_bytes(), "release binary must retain the version marker"
        replace(same)
        assert call() == original, "same-version replacement must stay quiet"
        replace(b"#!/bin/sh\nexit 99\n\0WORKTREES_CLI_VERSION=99.8.7\0")
        changed = call()
        assert changed["isError"] == original["isError"]
        assert changed["content"][:-1] == original["content"]
        warning = changed["content"][-1]["text"]
        assert f"server is v{version}" in warning, warning
        assert "installed binary is v99.8.7" in warning, warning
        assert "full session restart is unverified" in warning, warning
        assert call() == changed, "cached warning persists"
        replace(same)
        assert call() == original, "restoring version clears warning"
        replace(b"unknown older binary")
        assert call() == original, "unknown version is not a mismatch"
        print("ok: live replacement, matching version, cached warning, restore, unknown marker")
    finally:
        proc.stdin.close()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()
