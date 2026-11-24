#!/usr/bin/env python3
"""Run one group chat experiment on the Parallels test VMs (group chat v4.1 §12).

    run.py <variant> <task> [--persons 2|3] [--snapshot off-ready] [--revert]
           [--skip-install] [--config PATH]

Installs the bundle config.toml names, starts the apps, pairs the guests,
shares the lane, then plays personas/<variant>.yaml, capturing every screen
every few seconds and at every intent, and writes <runs>/<run>/record.json
with the logs beside it. score.py reads that directory.

A snapshot is reverted only with --revert: the owner's word is required.
Nothing here touches the host's own app, tmux or config.
"""
import argparse
import json
import os
import re
import shlex
import subprocess
import sys
import threading
import time
import tomllib
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from acts import Acts, Outcome, generate  # noqa: E402
from vm import APP_DIR, PRELUDE, VM, VMError  # noqa: E402


def expand(p):
    return Path(os.path.expanduser(p))


class Run:
    def __init__(self, a):
        self.args = a
        self.cfg = tomllib.loads(Path(a.config).read_text())
        self.ui = tomllib.loads((HERE / "ui" / f"{a.variant}.toml").read_text())
        # A UI file written before three persons names its sections by role
        # ([owner], [guest]); the acts look them up per person and share.
        for role, key in (("owner", f"{self.cfg['owner']['person']}_host"), ("guest", f"{self.cfg['guest']['person']}_guest")):
            if role in self.ui and key not in self.ui:
                self.ui[key] = self.ui[role]
        self.persona = json.loads((HERE / "personas" / f"{a.variant}.yaml").read_text())
        if self.persona.get("task") != a.task:
            raise SystemExit(f"personas/{a.variant}.yaml is task {self.persona.get('task')!r}, not {a.task!r}")
        if a.continue_run:
            # Another segment of a run (`--intents a:b`): same directory, same
            # record, the intents appended.
            self.dir = expand(a.continue_run)
            self.run_id = self.dir.name
        else:
            self.run_id = time.strftime("%Y%m%d-%H%M%S") + f"-{a.variant}-{a.task}"
            self.dir = expand(self.cfg["runs"]) / self.run_id
        (self.dir / "shots").mkdir(parents=True, exist_ok=True)
        (self.dir / "host").mkdir(exist_ok=True)
        self._logf = open(self.dir / "driver.log", "a", buffering=1)
        self.share_id = None
        self.vms = {}
        self.people = {}
        self.host_snap = {}
        self.record = {"run": self.run_id, "variant": a.variant, "task": a.task, "persons": a.persons,
                       "snapshot": a.snapshot if a.revert else None, "started_at": time.time(),
                       "intents": [], "setup": {}, "errors": []}
        if a.continue_run:
            self.record = json.loads((self.dir / "record.json").read_text())
            self.record.setdefault("segments", []).append({"intents": a.intents, "at": time.time()})

    # ── log ─────────────────────────────────────────────────────────────
    def log(self, msg):
        line = f"{time.strftime('%H:%M:%S')} {msg}"
        self._logf.write(line + "\n")
        print(line, flush=True)

    # ── VMs and people ──────────────────────────────────────────────────
    def build(self):
        c = self.cfg
        mk = lambda role, k: VM(role, c[k]["vm"], c[k]["ssh"], c[k]["ip"], self.log, os=c[k].get("os", "macos"))
        own, gst = mk("owner", "owner"), mk("guest", "guest")
        self.vms = {"owner": own, "guest": gst}
        o, g = c["owner"], c["guest"]
        # Every guest as seen from Dana's host. `ui` names the guest's section
        # in ui/<variant>.toml; `host` and `principal` are filled at setup.
        dana_guests = {
            g["person"]: {"vm": gst, "drive": "ui", "active": True, "name": g["name"], "device": g["device"],
                          "config_dir": None, "ui": f"{g['person']}_guest"},
        }
        self.ctx = {"dana": {"key": "dana", "owner": own, "oui": f"{o['person']}_host", "lane": o, "share": None,
                             "people": dana_guests, "host_name": o["name"], "host_person": o["person"]}}
        if self.args.persons >= 3:
            t = c["third"]
            thr = mk("third", "third")
            self.vms["third"] = thr
            drive = t.get("drive", "ui")
            dana_guests[t["person"]] = {"vm": thr if drive == "ui" else gst, "drive": drive, "active": True,
                                        "name": t["name"], "device": t["device"],
                                        "config_dir": t.get("config_dir"), "ui": f"{t['person']}_guest"}
            if "host" in t:
                # Cy hosts too (§12.2): his lane, Dana his guest.
                self.ctx[t["person"]] = {
                    "key": t["person"], "owner": thr, "oui": f"{t['person']}_host", "lane": t["host"], "share": None,
                    "host_name": t["name"], "host_person": t["person"],
                    "people": {o["person"]: {"vm": own, "drive": "ui", "active": True, "name": t["host"]["guest_name"],
                                             "device": t["host"]["guest_device"], "config_dir": None,
                                             "ui": f"{o['person']}_guest"}}}
        # The flat view wording() and the record use: person -> their vm.
        self.people = {o["person"]: {"role": "owner", "vm": own, "name": o["name"]}}
        for k, p in dana_guests.items():
            p["role"] = "guest"
            self.people[k] = p  # the same dict: pairing fills it for both views

    # ── setup ───────────────────────────────────────────────────────────
    def setup(self):
        s = self.record["setup"] if not self.args.continue_run else {}
        if self.args.revert:
            self.vms["owner"].revert(self.args.snapshot)
            gsnap = self.cfg["guest"].get("snapshot")
            if gsnap:
                self.vms["guest"].revert(gsnap)
            s["reverted"] = {"owner": self.args.snapshot, "guest": gsnap or None}
        for vm in self.vms.values():
            if vm.status() != "running":
                self.log(f"{vm.role}: {vm.status()}; starting")
                vm.prl("start", vm.name)
            vm.wait_ssh()
        for vm in self.vms.values():
            if vm.os == "linux":
                self.linux_session(vm)
        if self.args.continue_run:
            pass  # a later segment: the apps keep the state the last one left
        elif self.args.release:
            s["install"] = self.install_release(self.args.release)
        elif not self.args.skip_install:
            s["install"] = self.install()
        else:
            # The thread panel's open state lives in the app's memory, per
            # place: only a fresh app is known to have it closed.
            for vm in self.vms.values():
                self.restart_app(vm)
        for vm in self.vms.values():
            self.normalize_window(vm, dismiss=True)
            self.dismiss_banners(vm)
        s["beta"] = {vm.role: self.beta_state(vm) for vm in self.vms.values()}
        if self.args.from_ui:
            s.update(self.adopt_ui_state())
            if not self.args.continue_run:
                # A later segment leaves every screen where the last one did.
                self.click_ui(self.ctx["dana"]["oui"], "open_lane")
                time.sleep(2)
            return
        s["remote"] = self.remote_on()
        self.vms["owner"].serve_call("owner_set_name", {"name": self.cfg["owner"]["name"]})
        s["pairing"] = {n: self.pair(n, p) for n, p in self.people.items() if p["role"] == "guest"}
        s["lane"] = self.lane()
        self.share_id = self.share_create()
        self.ctx["dana"]["share"] = self.share_id
        s["share"] = self.share_id
        self.record["share"] = self.share_id
        self.click_ui("owner", "open_lane")
        time.sleep(2)

    def click_ui(self, section, name):
        x, y = self.ui[section][name]
        vm = next(c["owner"] for c in self.ctx.values() if c["oui"] == section)
        vm.click(x, y, why=name)

    def install_release(self, tag):
        """The published release, the way a user installs it: quit the app,
        `curl …/<tag>/install.sh | WORKTREES_INSTALL_APP=1 bash`, start it."""
        out = {}
        script = (HERE / "vm-install-release.sh").read_text()
        for vm in self.vms.values():
            rc, o = vm.sh(f"bash -s -- {shlex.quote(tag)}", stdin=script, timeout=600)
            last = (o.strip().splitlines() or ["{}"])[-1]
            out[vm.role] = json.loads(last) if last.startswith("{") else {"error": o[-300:]}
            self.log(f"{vm.role}: installed {out[vm.role]}")
        return out

    def restart_app(self, vm):
        if vm.os == "linux":
            vm.sh("pkill -x worktrees-app; sleep 5; ~/launch-app.sh >/dev/null; sleep 12", timeout=60)
        else:
            vm.sh('osascript -e \'tell application "worktrees" to quit\'; sleep 5; '
                  'open -a /Applications/worktrees.app; sleep 10', timeout=60)

    def linux_session(self, vm):
        """The Linux VM's GNOME session: the helper that runs a command in it
        (~/sess.sh), the display awake (a blanked display makes `prlctl
        capture` return its LAST frame, a frozen clock), no lock, no idle."""
        vm.sh("cat > ~/sess.sh && chmod +x ~/sess.sh", stdin=(HERE / "lin-session.sh").read_text(), timeout=20)
        vm.sh("cat > ~/launch-app.sh && chmod +x ~/launch-app.sh", stdin=(HERE / "lin-launch-app.sh").read_text(),
              timeout=20)
        vm.sh("~/sess.sh gsettings set org.gnome.desktop.session idle-delay 0; "
              "~/sess.sh gsettings set org.gnome.desktop.screensaver lock-enabled false; "
              "~/sess.sh gsettings set org.gnome.settings-daemon.plugins.power idle-dim false; "
              "~/sess.sh gdbus call --session --dest org.gnome.ScreenSaver --object-path /org/gnome/ScreenSaver "
              "--method org.gnome.ScreenSaver.SetActive false; "
              "~/sess.sh gdbus call --session --dest org.gnome.Mutter.DisplayConfig --object-path "
              "/org/gnome/Mutter/DisplayConfig --method org.freedesktop.DBus.Properties.Set "
              "org.gnome.Mutter.DisplayConfig PowerSaveMode \"<0>\"", timeout=30)

    def beta_state(self, vm):
        """Each beta key the config names, as the app's ui-state.json holds it."""
        keys = self.cfg.get("beta", [])
        rc, out = vm.sh(f"cat \"{vm.app_dir}/ui-state.json\"", timeout=20, quiet=True)
        try:
            st = json.loads(out)
        except ValueError:
            return {k: None for k in keys}
        return {k: st.get(k) for k in keys}

    def adopt_ui_state(self):
        """`--from-ui`: the pairings and shares were made in the apps (the
        first run is what the release test measures, and it is driven by
        hand, see the record's `first_run`). Read them back: each share by
        its lane, each guest's principal by name on that host, and the host
        id each guest's app keeps for it (same pin, a live device)."""
        out = {"from_ui": True, "shares": {}, "pairing": {}}
        self.record["first_run"] = self.persona.get("first_run")
        for key, c in self.ctx.items():
            host = c["owner"]
            shares = (host.serve_call("shares_all") or {}).get("shares", [])
            mine = [x for x in shares if x["slug"] == c["lane"]["lane"]]
            if not mine:
                raise VMError(f"{key}: no share on {c['lane']['lane']}: share it in the app first")
            c["share"] = mine[0]["id"]
            out["shares"][key] = c["share"]
            pin = (host.serve_call("remote_status") or {}).get("pin")
            guests = host.serve_call("guests_list") or []
            for who, p in c["people"].items():
                g = next((x for x in guests if x["name"].lower() == p["name"].lower()
                          and any(not d.get("revoked_at") and not d.get("disabled") for d in x["devices"])), None)
                if not g:
                    raise VMError(f"{key}: {p['name']} is not a live guest there")
                live = {d["id"] for d in g["devices"] if not d.get("revoked_at") and not d.get("disabled")}
                cdir = shlex.quote(p["config_dir"]) if p["config_dir"] else f'"{p["vm"].app_dir}"'
                rc, js = p["vm"].sh(f"worktrees remote hosts --json --config-dir {cdir}", timeout=30)
                hosts = (json.loads(js) if rc == 0 and js.strip().startswith("{") else {}).get("hosts", [])
                h = next((x for x in hosts if x.get("pin") == pin and x.get("device_id") in live), None)
                if not h:
                    raise VMError(f"{key}: {who}'s app keeps no live pairing with this host")
                p.update(principal=g["id"], host=h["id"])
                out["pairing"][f"{key}:{who}"] = {"principal": g["id"], "host": h["id"],
                                                   "stale_records_same_pin": sum(1 for x in hosts if x.get("pin") == pin) - 1}
            self.log(f"{key}: share {c['share']} with {sorted(c['people'])}")
        self.share_id = self.ctx["dana"]["share"]
        self.record["share"] = self.share_id
        if self.args.continue_run:
            return out
        # Only the watched files go: the lane keeps its session, or the
        # binding the people made would end before the run starts.
        o = self.cfg["owner"]
        tree = f"{o['repo']}/.worktrees/{o['lane']}"
        for f in self.persona.get("watch_files", []):
            self.vms["owner"].sh(f"rm -f {shlex.quote(tree + '/' + f)}", timeout=20)
        return out

    def install(self):
        bundle = expand(self.cfg["bundle"])
        if not (bundle / "Contents" / "MacOS" / "worktrees").exists():
            raise SystemExit(f"no bundle at {bundle}: run `make install-app APPDIR=…` on the host first")
        out = {}
        for vm in self.vms.values():
            self.log(f"{vm.role}: copying {bundle.name}")
            tar = subprocess.Popen(["tar", "-C", str(bundle.parent), "-czf", "-", bundle.name], stdout=subprocess.PIPE)
            p = subprocess.run(["ssh", "-o", "BatchMode=yes", vm.ssh_alias,
                                "rm -rf ~/vmlab-build && mkdir -p ~/vmlab-build && tar -C ~/vmlab-build -xzf -"],
                               stdin=tar.stdout, capture_output=True, text=True, timeout=600)
            tar.wait()
            if p.returncode != 0:
                raise VMError(f"{vm.role}: copy failed: {p.stderr[-300:]}")
            script = (HERE / "vm-install.sh").read_text()
            keys = " ".join(shlex.quote(k) for k in self.cfg.get("beta", []))
            rc, o = vm.sh(f"bash -s -- {keys}", stdin=script, timeout=180)
            last = (o.strip().splitlines() or ["{}"])[-1]
            out[vm.role] = json.loads(last) if last.startswith("{") else {"error": o[-300:]}
            self.log(f"{vm.role}: installed {out[vm.role]}")
        return out

    def normalize_window(self, vm, dismiss=False):
        """Bring the app forward and pin its window to config's frame, so the
        coordinates hold. `dismiss`: Esc a What's new sheet, close the
        Updates card."""
        if vm.os == "linux":
            # The AppImage's window is maximized on the VM's one screen; the
            # Linux coordinates are measured for that.
            vm.sh("~/sess.sh xdotool search --class worktrees-app windowactivate 2>/dev/null", timeout=20, quiet=True)
            return
        x, y, w, h = self.cfg["window"]
        proc = '(first process whose bundle identifier is "net.casadelvalle.worktrees")'
        lines = ['tell application "worktrees" to activate', "delay 1"]
        if dismiss:
            lines.append('tell application "System Events" to key code 53')
        lines += [f'tell application "System Events" to tell {proc} to set position of window 1 to {{{x}, {y}}}',
                  f'tell application "System Events" to tell {proc} to set size of window 1 to {{{w}, {h}}}']
        vm.sh("osascript " + " ".join(f"-e {shlex.quote(ln)}" for ln in lines), timeout=30)
        if dismiss:
            sec = self.ui.get(f"{self.cfg[vm.role]['person']}_{'host' if vm.role == 'owner' else 'guest'}", {})
            if "updates_close" in sec:
                ux, uy = sec["updates_close"]
                vm.sh(f"/opt/homebrew/bin/cliclick c:{ux},{uy}", timeout=20)

    def dismiss_banners(self, vm):
        """macOS notifications stack at the top right, over the app. Each one
        shows its close button at its top-left corner only under the pointer:
        hover, click, repeat. `killall NotificationCenter` does not do it, they
        come back."""
        if vm.os == "linux":
            return
        b = self.ui.get("macos", {})
        hx, hy = b.get("banner_hover", [1180, 90])
        cx, cy = b.get("banner_close", [1010, 50])
        vm.sh(" ".join(f"/opt/homebrew/bin/cliclick m:{hx},{hy} w:600 c:{cx},{cy} w:500;" for _ in range(4)), timeout=60)

    def remote_on(self):
        o = self.vms["owner"]
        st = o.serve_call("remote_status") or {}
        if st.get("state") != "listening":
            self.log(f"owner: remote access {st.get('state')}; turning it on at {self.cfg['owner']['ip']}")
            o.sh(f"worktrees serve remote on {self.cfg['owner']['ip']}", timeout=60)
            st = o.serve_call("remote_status") or {}
        return {"state": st.get("state"), "at": st.get("at"), "pin": st.get("pin")}

    def pair(self, who, p):
        """The guest pairs into its app's config dir (or its own, for a person
        with no screen) the rehearsal's way: an invite window on the owner in
        a `tmux -L vmlab` session, the link read from a file and handed over
        on stdin, the codes compared, the owner answering `y`. A pairing that
        still holds (same host pin, device not revoked) is reused."""
        o, g = self.vms["owner"], p["vm"]
        pin = (o.serve_call("remote_status") or {}).get("pin")
        cdir = shlex.quote(p["config_dir"]) if p["config_dir"] else f'"{APP_DIR}"'
        guests = o.serve_call("guests_list") or []
        mine = next((x for x in guests if x["name"] == p["name"]), None)
        live = {d["id"] for d in (mine or {}).get("devices", []) if not d.get("revoked_at") and not d.get("disabled")}
        rc, out = g.sh(f"mkdir -p {cdir}; worktrees remote hosts --json --config-dir {cdir}", timeout=30)
        hosts = (json.loads(out) if rc == 0 and out.strip().startswith("{") else {}).get("hosts", [])
        for h in hosts:
            if h.get("pin") == pin and h.get("device_id") in live:
                p.update(host=h["id"], principal=mine["id"])
                self.log(f"{who}: pairing reused ({h['id']}, {h['device_id']})")
                return {"reused": True, "host": h["id"], "principal": mine["id"]}
        for h in hosts:
            if h.get("pin") == pin:
                g.sh(f"worktrees remote forget {h['id']} --config-dir {cdir}", timeout=30)
        how = f"--guest {mine['id']}" if mine else f"--invite {shlex.quote(p['name'])}"
        o.sh("tmux -L vmlab kill-session -t pair 2>/dev/null; rm -f ~/vmlab-pair.out; umask 077; "
             f"tmux -L vmlab new-session -d -s pair -x 200 -y 50 "
             f"{shlex.quote(f'worktrees serve pair {how} > ~/vmlab-pair.out 2>&1; sleep 300')}", timeout=30)
        link = None
        for _ in range(20):
            time.sleep(1)
            rc, out = o.sh("grep -o 'worktrees-pair:v1?[^ ]*' ~/vmlab-pair.out | head -1", timeout=20, quiet=True)
            if out.strip():
                link = out.strip()
                break
        if not link:
            raise VMError(f"{who}: the owner's invite printed no link")
        # The link is a secret: stdin to the guest, a 0600 file there, never a log line.
        # The guest's dial runs in an ssh session the host keeps open until it
        # ends. Backgrounded in the VM with its session closed, the same
        # command got "No route to host" (os error 65) from the window's port
        # (four of four), while a dial inside a live session paired (three of
        # three, 2026-11-14). Most likely macOS's Local Network privacy judging
        # a process whose session is gone; not proved beyond those seven.
        dial = subprocess.Popen(
            ["ssh", "-o", "BatchMode=yes", g.ssh_alias,
             PRELUDE + "umask 077; cat > ~/vmlab-link; "
             f"worktrees remote pair \"$(cat ~/vmlab-link)\" --name {shlex.quote(p['device'])} --config-dir {cdir} "
             "> ~/vmlab-pair-guest.out 2>&1; echo \"rc=$?\" >> ~/vmlab-pair-guest.out; rm -f ~/vmlab-link"],
            stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, text=True)
        dial.stdin.write(link + "\n")
        dial.stdin.close()
        del link
        code = None
        for _ in range(30):
            time.sleep(1)
            _, go = g.sh("cat ~/vmlab-pair-guest.out 2>/dev/null", timeout=20, quiet=True)
            _, oo = o.sh("cat ~/vmlab-pair.out", timeout=20, quiet=True)
            gc = re.search(r"Comparison code: (\d{3} \d{3})", go)
            oc = re.search(r"code (\d{3} \d{3})", oo)
            if gc and oc:
                if gc.group(1) != oc.group(1):
                    raise VMError(f"{who}: comparison codes differ")
                code = gc.group(1)
                break
        if not code:
            raise VMError(f"{who}: no comparison code on both sides")
        o.sh("tmux -L vmlab send-keys -t pair y Enter", timeout=20)
        res = None
        for _ in range(30):
            time.sleep(1)
            _, go = g.sh("cat ~/vmlab-pair-guest.out", timeout=20, quiet=True)
            m = re.search(r"remote call (h_[a-z0-9]+) ", go)
            if "rc=" in go:
                res = (m.group(1) if m else None, go)
                break
        try:
            dial.wait(timeout=30)
        except subprocess.TimeoutExpired:
            dial.terminate()
        o.sh("tmux -L vmlab kill-session -t pair 2>/dev/null; rm -f ~/vmlab-pair.out", timeout=20)
        if not res or not res[0]:
            raise VMError(f"{who}: pairing did not finish: {(res or ('', ''))[1][-200:]}")
        guests = o.serve_call("guests_list") or []
        mine = next(x for x in guests if x["name"] == p["name"])
        p.update(host=res[0], principal=mine["id"])
        self.log(f"{who}: paired as {mine['id']} with host {res[0]}")
        if p["drive"] == "ui":
            # The app reads its hosts at launch.
            g.sh('osascript -e \'tell application "worktrees" to quit\'; sleep 4; open -a /Applications/worktrees.app; sleep 8',
                 timeout=60)
            self.normalize_window(g, dismiss=True)
        return {"reused": False, "host": res[0], "principal": mine["id"]}

    def lane(self):
        """The lane, as every run starts it: no share on it, the task's files
        gone, the agent up with one warm-up turn (a share needs a session
        that has named itself)."""
        o, c = self.vms["owner"], self.cfg["owner"]
        for s in (o.serve_call("shares_all") or {}).get("shares", []):
            if s["slug"] == c["lane"]:
                o.serve_call("share_stop", {"share": s["id"]})
                self.log(f"owner: stopped an earlier share {s['id']}")
        for r in (o.serve_call("share_requests_list") or []):
            o.serve_call("share_request_dismiss", {"request": r["id"]})
        tree = f"{c['repo']}/.worktrees/{c['lane']}"
        for f in self.persona.get("watch_files", []):
            o.sh(f"rm -f {shlex.quote(tree + '/' + f)}", timeout=20)
        o.sh(f"cd {shlex.quote(c['repo'])} && worktrees close {c['lane']} >/dev/null 2>&1; sleep 2; "
             f"worktrees open {c['lane']} --ai {c['harness']} --no-attach >/dev/null 2>&1", timeout=90)
        time.sleep(20)
        self.dismiss_harness_modal()
        acts = Acts(self)
        acts.agent_send("Reply with one line: READY.")
        time.sleep(15)
        acts.wait("dana", {"until": "agent_idle", "max_s": 90})
        return {"session": acts.lane_session()}

    def dismiss_harness_modal(self):
        """Codex opens on its "Update available" list when npm has a newer one:
        `3` is "Skip until next version". Pressed only when that list shows."""
        o = self.vms["owner"]
        sess = json.dumps(Acts(self).lane_session())
        _, pane = o.sh(f"tmux capture-pane -p -t {sess}", timeout=20)
        if "Update available" in pane and "Skip until next version" in pane:
            o.sh(f"tmux send-keys -t {sess} 3", timeout=20)
            time.sleep(2)
            self.log("owner: skipped Codex's update list")

    def share_create(self):
        o, c = self.vms["owner"], self.cfg["owner"]
        guests = [p["principal"] for p in self.people.values() if p["role"] == "guest"]
        j = o.serve_call("share_create", {"repo": c["repo"], "slug": c["lane"], "guests": guests})
        if not j or "id" not in j:
            raise VMError(f"share_create failed: {j}")
        self.log(f"owner: shared {c['lane']} as {j['id']} with {guests}")
        return j["id"]

    # ── wording ─────────────────────────────────────────────────────────
    def wording(self, who, spec, ctx=None):
        """A post's words: literal, or one sentence from Claude written from
        what THIS guest can read (shared_turns and the thread, through their
        own guest rows), so the scripted person never sees more."""
        if isinstance(spec, str):
            return spec
        ctx = ctx or self.ctx["dana"]
        p = ctx["people"][who]
        turns = p["vm"].remote_call(p["host"], "shared_turns", {"share": ctx["share"], "limit": 6},
                                    config_dir=p["config_dir"]) or {}
        thread = p["vm"].remote_call(p["host"], "thread_read", {"share": ctx["share"], "after": 0},
                                     config_dir=p["config_dir"]) or {}
        seen = []
        for t in reversed(turns.get("turns", [])):
            if t.get("user"):
                seen.append(f"[owner to agent] {t['user'][:600]}")
            if t.get("reply"):
                seen.append(f"[agent] {t['reply'][:1200]}")
            if t.get("tools"):
                seen.append(f"[agent ran] {', '.join(t['tools'][:8])}")
        for e in thread.get("entries", []):
            if e.get("kind") == "post":
                seen.append(f"[thread, {e.get('name')}] {e.get('text')}")
        me = self.persona["people"].get(who, {}).get("about", "")
        prompt = (f"You are {p['name']}, {me}\n"
                  f"You are watching a colleague's coding agent session, read-only, and writing in the thread beside it.\n"
                  f"What you can see, oldest first:\n" + "\n".join(seen[-30:]) +
                  f"\n\nYour intent: {spec['generate']}\n"
                  "Reply with the message only: one sentence, plain text, no quotes, no preamble.")
        out = generate(self.cfg["generate_cmd"], prompt, self.cfg["generate_timeout_s"], str(self.dir / "gen"))
        self.log(f"{who}: wording {'generated' if out else 'fell back'} ({len(out or spec['fallback'])} chars)")
        return out or spec["fallback"]

    # ── captures ────────────────────────────────────────────────────────
    def shot(self, vm, name):
        png = self.dir / "shots" / f"{name}.png"
        if not vm.capture(png):
            return None
        jpg = png.with_suffix(".jpg")
        # Half size (points) and JPEG: a run's hundreds of captures stay small.
        r = subprocess.run(["sips", "-s", "format", "jpeg", "-s", "formatOptions", "70", "-Z", "1365",
                            str(png), "--out", str(jpg)], capture_output=True)
        if r.returncode == 0:
            png.unlink()
            return jpg.name
        return png.name

    def cadence(self, stop):
        every = self.cfg["capture_every_s"]
        while not stop.wait(every):
            t = time.time()
            for vm in self.vms.values():
                n = self.shot(vm, f"t{t:.0f}-{vm.role}")
                if n:
                    with open(self.dir / "shots" / "index.jsonl", "a") as f:
                        f.write(json.dumps({"at": round(t, 1), "vm": vm.role, "file": n}) + "\n")

    # ── host state ──────────────────────────────────────────────────────
    def snapshot_host(self, why):
        """Each share's thread and shared turns while it exists (a stop
        deletes the thread), and the watched files' hashes. Dana's share is
        kept at the top level (host/thread.json), any other host's under its
        key (host/thread-<key>.json)."""
        for key, c in self.ctx.items():
            if not c.get("share"):
                continue
            snap = self.host_snap if key == "dana" else self.host_snap.setdefault(key, {})
            a = Acts(self, c)
            th = a.thread()
            if th is not None:
                snap["thread"] = th
            s = a.owner_share()
            if s:
                snap["share"] = s
                turns = {}
                for k, b in [("current", None)] + [(f"ended{i}", i) for i in range(len(s.get("ended", [])))]:
                    args = {"share": c["share"], "limit": 50}
                    if b is not None:
                        args["binding"] = b
                    j = c["owner"].serve_call("shared_turns", args)
                    if j:
                        turns[k] = j
                if turns:
                    snap["turns"] = turns
        self.host_snap["why"] = why

    def files_state(self):
        c = self.cfg["owner"]
        tree = f"{c['repo']}/.worktrees/{c['lane']}"
        out = {}
        for f in self.persona.get("watch_files", []):
            _, h = self.vms["owner"].sh(f"shasum -a 256 {shlex.quote(tree + '/' + f)} 2>/dev/null | cut -c1-16",
                                        timeout=20, quiet=True)
            out[f] = h.strip() or None
        return out

    def pending(self):
        """What the owner's UI should be surfacing now: guests waiting to be
        let in, and requests for another place. score.py's lost attention."""
        items = []
        for key, c in self.ctx.items():
            o = c["owner"]
            for s in (o.serve_call("shares_all") or {}).get("shares", []):
                for g in s.get("guests", []):
                    if g["state"] == "asked":
                        items.append(f"{key}:join:{s['id']}:{g['principal']}")
            for r in (o.serve_call("share_requests_list") or []):
                items.append(f"{key}:request:{r['id']}")
        return items

    # ── the script ──────────────────────────────────────────────────────
    def play(self):
        # One Acts per share: each keeps its own UI state (thread open, sent once).
        by_ctx = {k: Acts(self, c) for k, c in self.ctx.items()}
        # What the apps show is carried from segment to segment (a panel left
        # open by the last one must not be toggled shut by this one).
        for k, st in self.record.get("ui_state", {}).items():
            if k in by_ctx:
                by_ctx[k].thread_open, by_ctx[k].sent_once = st.get("thread_open", False), st.get("sent_once", False)
        lo, _, hi = (self.args.intents or ":").partition(":")
        lo, hi = int(lo or 0), int(hi) if hi else len(self.persona["intents"])
        for i, it in enumerate(self.persona["intents"]):
            if not lo <= i < hi:
                continue
            who = it["who"]
            on = it.get("on", "dana")
            if on not in by_ctx:
                continue
            acts = by_ctx[on]
            ctx = self.ctx[on]
            people_needed = [who] + ([it["args"]["guest"]] if it.get("args", {}).get("guest") else [])
            known = set(ctx["people"]) | {ctx["host_person"]}
            if any(w not in known for w in people_needed):
                continue
            rec = {"i": i, "who": who, "on": on, "act": it["act"], "args": it.get("args", {}), "t_start": time.time()}
            before = {r: vm.snapshot_counts() for r, vm in self.vms.items()}
            self.log(f"intent {i}: {who} {it['act']}")
            if it.get("impossible"):
                out = Outcome(False, "blind spot", impossible=it["impossible"])
            else:
                fn = getattr(acts, it["act"], None)
                if fn is None:
                    out = Outcome(False, f"unknown act {it['act']}")
                else:
                    try:
                        out = fn(who, it.get("args", {}))
                    except Exception as e:  # noqa: BLE001 — an act's failure is the record's, not the run's
                        out = Outcome(False, f"{type(e).__name__}: {e}")
                        self.record["errors"].append({"i": i, "error": str(e)})
            rec["t_end"] = time.time()
            after = {r: vm.snapshot_counts() for r, vm in self.vms.items()}
            rec["inputs"] = {r: {k: after[r][k] - before[r][k] for k in after[r]} for r in after}
            rec.update(out)
            rec["proved"] = {True: "proved", False: "unproved", None: "n/a"}[out["proved"]]
            rec["shots"] = [n for n in (self.shot(vm, f"i{i:02d}-{it['act']}-{vm.role}") for vm in self.vms.values()) if n]
            rec["pending"] = self.pending()
            rec["files"] = self.files_state()
            self.snapshot_host(f"intent {i}")
            self.log(f"intent {i}: {rec['proved']} — {out['proof']}")
            self.record["intents"].append(rec)
            self.record["ui_state"] = {k: {"thread_open": a.thread_open, "sent_once": a.sent_once} for k, a in by_ctx.items()}
            self.save()
            time.sleep(it.get("wait_s", 0))

    # ── the end ─────────────────────────────────────────────────────────
    def collect(self):
        """Both VMs' serve.log and audit log (the run's window), the thread
        and turns last seen, and the record."""
        since = time.strftime("%Y-%m-%d %H:%M:%S", time.gmtime(self.record["started_at"] - 5))
        since_iso = since.replace(" ", "T")
        for vm in self.vms.values():
            _, log = vm.sh(f"awk -v s={shlex.quote(since)} 'substr($0,1,19) >= s' ~/.cache/worktrees/serve.log",
                           timeout=120)
            (self.dir / "host" / f"{vm.role}-serve.log").write_text(log)
            _, audit = vm.sh(f"python3 -c {shlex.quote(AUDIT_TAIL)} {shlex.quote(since_iso)} \"{vm.app_dir}/audit.jsonl\"",
                             timeout=120)
            (self.dir / "host" / f"{vm.role}-audit.jsonl").write_text(audit)
        # A segment that saw no share (it was stopped earlier) leaves the
        # last segment's snapshot as it was.
        for part in ("thread", "turns", "share"):
            if part in self.host_snap:
                (self.dir / "host" / f"{part}.json").write_text(json.dumps(self.host_snap[part], indent=1))
        for key in self.ctx:
            if key != "dana" and key in self.host_snap:
                for part in ("thread", "turns", "share"):
                    if part in self.host_snap[key]:
                        (self.dir / "host" / f"{part}-{key}.json").write_text(json.dumps(self.host_snap[key][part], indent=1))

    def save(self):
        self.record["inputs_total"] = {r: vm.snapshot_counts() for r, vm in self.vms.items()}
        (self.dir / "record.json").write_text(json.dumps(self.record, indent=1, default=str))

    def main(self):
        self.build()
        self.log(f"run {self.run_id} in {self.dir}")
        stop = threading.Event()
        try:
            self.setup()
            self.save()
            cap = threading.Thread(target=self.cadence, args=(stop,), daemon=True)
            cap.start()
            self.play()
        except (VMError, SystemExit) as e:
            self.record["errors"].append({"setup_or_play": str(e)})
            self.log(f"stopped: {e}")
        finally:
            stop.set()
            self.record["ended_at"] = time.time()
            try:
                self.collect()
            except Exception as e:  # noqa: BLE001
                self.record["errors"].append({"collect": str(e)})
            self.save()
            self.log(f"record: {self.dir / 'record.json'}")


AUDIT_TAIL = """
import sys
since, path = sys.argv[1], sys.argv[2]
try:
    with open(path) as f:
        for line in f:
            i = line.find('"ts":"')
            if i >= 0 and line[i + 6:i + 25] >= since:
                sys.stdout.write(line)
except OSError:
    pass
"""


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("variant")
    ap.add_argument("task")
    ap.add_argument("--persons", type=int, choices=(2, 3), default=2)
    ap.add_argument("--snapshot", default="off-ready")
    ap.add_argument("--revert", action="store_true",
                    help="revert the owner VM to --snapshot first (needs the owner's word)")
    ap.add_argument("--skip-install", action="store_true", help="keep the bundle the VMs already run")
    ap.add_argument("--config", default=str(HERE / "config.toml"))
    ap.add_argument("--release", metavar="TAG",
                    help="install this published release in every VM the way a user does (install.sh)")
    ap.add_argument("--intents", metavar="A:B", help="play only intents A..B-1 (a segment)")
    ap.add_argument("--continue", dest="continue_run", metavar="RUN_DIR",
                    help="append this segment to an earlier run's record (implies no install)")
    ap.add_argument("--from-ui", action="store_true",
                    help="start from the pairings and shares people made in the apps, not the CLI pairing")
    a = ap.parse_args()
    if a.snapshot != "off-ready" and not a.revert:
        ap.error("--snapshot names what --revert reverts to; without --revert nothing is reverted")
    Run(a).main()


if __name__ == "__main__":
    main()
