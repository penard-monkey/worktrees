"""One Parallels VM, driven from the host: ssh, screen captures, clicks and keys.

The Python twin of the rehearsal's `vm.sh` (ssh | shot | type | key | click |
open | revert). Everything goes over `prlctl` and ssh; nothing here touches the
host's own app, tmux or config. Every click, key and typed string is counted
on the VM and logged by the driver, which is how §12.4's "acts per outcome" is
measured.
"""
import shlex
import subprocess
import threading
import time

# The guest's PATH has neither ~/.local/bin nor Homebrew until we add them, and
# Codex runs on nvm's node.
PRELUDE = ('export PATH=$HOME/.local/bin:/opt/homebrew/bin:$PATH; '
           'export NVM_DIR=$HOME/.nvm; . $NVM_DIR/nvm.sh >/dev/null 2>&1 || true; ')

APP_DIR = '$HOME/Library/Application Support/net.casadelvalle.worktrees'
# The app's config dir on Linux (tauri's app_config_dir).
APP_DIR_LINUX = '$HOME/.config/net.casadelvalle.worktrees'

# macOS key codes the acts use, as xdotool key names for a Linux VM.
XDO_KEYS = {36: "Return", 53: "Escape", 48: "Tab", 51: "BackSpace", 0: "a", 9: "v"}


class VMError(RuntimeError):
    pass


class VM:
    def __init__(self, role, name, ssh, ip, log, os="macos"):
        self.role, self.name, self.ssh_alias, self.ip = role, name, ssh, ip
        # "macos": osascript and cliclick, coordinates in points.
        # "linux": xdotool inside the logged-in GNOME session (~/sess.sh
        # copies its display env from gsd-xsettings), coordinates in pixels
        # (a capture is 1:1 there).
        self.os = os
        self.app_dir = APP_DIR_LINUX if os == "linux" else APP_DIR
        self.log = log
        self.counts = {"click": 0, "key": 0, "type": 0}
        self._lock = threading.Lock()

    # ── shell ──────────────────────────────────────────────────────────────
    def sh(self, script, stdin=None, timeout=120, check=False, quiet=False):
        """Run `script` in the VM's login user's shell. Returns (rc, stdout)."""
        cmd = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", self.ssh_alias, PRELUDE + script]
        try:
            p = subprocess.run(cmd, input=stdin, capture_output=True, text=True, timeout=timeout)
        except subprocess.TimeoutExpired:
            if not quiet:
                self.log(f"{self.role}: ssh timed out after {timeout}s")
            if check:
                raise VMError(f"{self.role}: timeout")
            return 124, ""
        if check and p.returncode != 0:
            raise VMError(f"{self.role}: rc={p.returncode}: {(p.stderr or p.stdout)[-400:]}")
        return p.returncode, p.stdout

    def reachable(self):
        rc, out = self.sh("echo up", timeout=10, quiet=True)
        return rc == 0 and out.strip() == "up"

    def wait_ssh(self, deadline_s=240):
        t0 = time.time()
        while time.time() - t0 < deadline_s:
            if self.reachable():
                return time.time() - t0
            time.sleep(3)
        raise VMError(f"{self.role}: ssh never answered in {deadline_s}s")

    # ── screen ────────────────────────────────────────────────────────────
    def wake(self):
        """Linux: the display awake. GNOME on this VM powers the display off
        (Mutter PowerSaveMode 3) even with idle-delay 0, and `prlctl capture`
        then returns the LAST frame it had: a frozen clock, a click that seems
        to do nothing. Set it on before every capture and input."""
        if self.os == "linux":
            self.sh("~/sess.sh gdbus call --session --dest org.gnome.Mutter.DisplayConfig --object-path "
                    "/org/gnome/Mutter/DisplayConfig --method org.freedesktop.DBus.Properties.Set "
                    "org.gnome.Mutter.DisplayConfig PowerSaveMode '<0>' >/dev/null", timeout=20, quiet=True)
            time.sleep(0.5)

    def capture(self, path):
        """A screenshot of the VM's screen (prlctl, so nothing runs inside)."""
        self.wake()
        try:
            p = subprocess.run(["prlctl", "capture", self.name, "--file", str(path)],
                               capture_output=True, text=True, timeout=30)
            return p.returncode == 0
        except subprocess.TimeoutExpired:
            return False

    def click(self, x, y, why=""):
        """A click at (x, y) in POINTS (a capture's pixels / 2)."""
        with self._lock:
            self.counts["click"] += 1
        self.log(f"{self.role}: click {x},{y} {why}".rstrip())
        if self.os == "linux":
            self.wake()
            return self.sh(f"~/sess.sh xdotool mousemove {int(x)} {int(y)} sleep 0.2 click 1", timeout=20)[0] == 0
        return self.sh(f"/opt/homebrew/bin/cliclick c:{int(x)},{int(y)}", timeout=20)[0] == 0

    def key(self, code, why="", command=False):
        """A key code (36 Return, 53 Esc, 48 Tab, 51 Delete, 0 "a") to the
        frontmost app, with ⌘ held when `command`."""
        with self._lock:
            self.counts["key"] += 1
        self.log(f"{self.role}: key {'cmd-' if command else ''}{code} {why}".rstrip())
        if self.os == "linux":
            k = XDO_KEYS[int(code)]
            return self.sh(f"~/sess.sh xdotool key {'ctrl+' if command else ''}{k}", timeout=20)[0] == 0
        using = " using command down" if command else ""
        return self.sh(f"osascript -e 'tell application \"System Events\" to key code {int(code)}{using}'",
                       timeout=20)[0] == 0

    def type(self, text, why=""):
        """Type `text` into the frontmost app. The text goes on stdin, never argv
        or a log line beyond its length."""
        with self._lock:
            self.counts["type"] += 1
        self.log(f"{self.role}: type {len(text)} chars {why}".rstrip())
        # The text reaches the VM on ssh's stdin and AppleScript (or xdotool)
        # reads it from a 0600 temp file, so no shell quoting ever touches it.
        if self.os == "linux":
            rc, _ = self.sh('f=$(mktemp); chmod 600 "$f"; cat > "$f"; ~/sess.sh xdotool type --delay 15 --file "$f"; rm -f "$f"',
                            stdin=text, timeout=60)
            return rc == 0
        rc, _ = self.sh('f=$(mktemp); chmod 600 "$f"; cat > "$f"; '
                        'osascript -e "set t to (read POSIX file \\"$f\\" as «class utf8»)" '
                        '-e \'tell application "System Events" to keystroke t\'; rm -f "$f"',
                        stdin=text, timeout=60)
        return rc == 0

    def snapshot_counts(self):
        with self._lock:
            return dict(self.counts)

    # ── the app ───────────────────────────────────────────────────────────
    def serve_call(self, cmd, args=None, timeout=60):
        """`worktrees serve call` against the app's serve (the owner's console)."""
        import json
        a = shlex.quote(json.dumps(args or {}))
        rc, out = self.sh(f"worktrees serve call {cmd} {a}", timeout=timeout)
        return _json_or_none(out) if rc == 0 else None

    def remote_call(self, host, cmd, args=None, timeout=60, config_dir=None):
        """`worktrees remote call` as a guest: from the guest app's own config
        dir, or a scripted person's own one."""
        import json
        a = shlex.quote(json.dumps(args or {}))
        d = shlex.quote(config_dir) if config_dir else f'"{self.app_dir}"'
        rc, out = self.sh(f'worktrees remote call {shlex.quote(host)} {cmd} {a} '
                          f'--config-dir {d} --timeout 20', timeout=timeout)
        return _json_or_none(out) if rc == 0 else None

    # ── Parallels ─────────────────────────────────────────────────────────
    def prl(self, *args, timeout=300):
        p = subprocess.run(["prlctl", *args], capture_output=True, text=True, timeout=timeout)
        return p.returncode, (p.stdout + p.stderr).strip()

    def snapshot_id(self, name):
        import json
        rc, out = self.prl("snapshot-list", self.name, "-j")
        if rc != 0:
            raise VMError(f"{self.role}: snapshot-list: {out[-200:]}")
        for sid, s in json.loads(out).items():
            if s.get("name") == name:
                return sid
        raise VMError(f"{self.role}: no snapshot named {name}")

    def revert(self, name):
        """Switch to snapshot `name`. The caller has the owner's word (--revert)."""
        sid = self.snapshot_id(name)
        self.log(f"{self.role}: revert to {name} {sid}")
        rc, out = self.prl("snapshot-switch", self.name, "-i", sid, timeout=600)
        if rc != 0:
            raise VMError(f"{self.role}: snapshot-switch: {out[-300:]}")
        state = self.status()
        if state != "running":
            self.log(f"{self.role}: {state} after revert; starting")
            rc, out = self.prl("start", self.name, timeout=300)
            if rc != 0:
                raise VMError(f"{self.role}: start: {out[-300:]}")

    def status(self):
        rc, out = self.prl("status", self.name)
        return out.rsplit(" ", 1)[-1] if rc == 0 else "unknown"

    def restart(self):
        self.log(f"{self.role}: prlctl restart {self.name}")
        return self.prl("restart", self.name, timeout=300)[0] == 0


def _json_or_none(out):
    import json
    out = out.strip()
    if not out:
        return None
    try:
        return json.loads(out)
    except ValueError:
        # A CLI that printed a line before its JSON: take from the first brace.
        for i, ch in enumerate(out):
            if ch in "{[":
                try:
                    return json.loads(out[i:])
                except ValueError:
                    break
        return None
