"""One function per act (§12.2's intents), each PROVED on host state.

A UI act clicks and types through the variant's coordinates
(`ui/<variant>.toml`); an act that is not a UI act in this variant uses the
CLI (`worktrees serve call` as the owner's console, `worktrees remote call`
as a guest). No act is proved by reading the screen: there is no OCR here.
The proof is what the hosts say afterwards: `shares_all`, `thread_read`,
`shared_turns`, `share_requests_list`, the lane's files.

Every act returns `Outcome(proved, proof, extra)`. `proved` is True, False,
or None when the act has nothing to prove (a wait). `wrong_place` in `extra`
marks an act that landed on something other than what it meant to touch.
"""
import json
import os
import subprocess
import time


class Outcome(dict):
    def __init__(self, proved, proof, **extra):
        super().__init__(proved=proved, proof=proof, **extra)


def _until(pred, timeout_s, every_s=1.0):
    """Poll `pred` until it returns something truthy; that value, or None."""
    t0 = time.time()
    while True:
        v = pred()
        if v:
            return v
        if time.time() - t0 >= timeout_s:
            return None
        time.sleep(every_s)


class Acts:
    """The acts, bound to one run's VMs, coordinates and share."""

    def __init__(self, run):
        self.run = run
        self.cfg = run.cfg
        self.ui = run.ui
        self.owner = run.vms["owner"]
        self.log = run.log
        self.thread_open = False
        # After the first send in an app session a "Sent to the agent." line
        # sits under the thread and moves Use these… and the note up.
        self.sent_once = False

    # ── helpers ─────────────────────────────────────────────────────────
    def person(self, who):
        return self.run.people[who]

    def at(self, role, name):
        x, y = self.ui[role][name]
        return x, y

    def click(self, vm, role, name):
        x, y = self.at(role, name)
        return vm.click(x, y, why=name)

    def share(self):
        return self.run.share_id

    def owner_share(self):
        j = self.owner.serve_call("shares_all")
        for s in (j or {}).get("shares", []):
            if s["id"] == self.share():
                return s
        return None

    def thread(self, after=0):
        """Every entry of the share's thread after `after`, from the console."""
        out, cur = [], after
        while True:
            j = self.owner.serve_call("thread_read", {"share": self.share(), "after": cur})
            if not j:
                return out if out else None
            out.extend(j.get("entries", []))
            if not j.get("more") or not j.get("entries"):
                return out
            cur = j["entries"][-1]["seq"]

    def last_seq(self):
        t = self.thread() or []
        return t[-1]["seq"] if t else 0

    def lane_session(self):
        o = self.cfg["owner"]
        return f"worktrees-{o['lane']}~agent~{o['harness']}"

    def pane(self):
        rc, out = self.owner.sh(f"tmux capture-pane -p -t {json.dumps(self.lane_session())}", timeout=20)
        return out if rc == 0 else ""

    def agent_busy(self):
        p = self.pane()
        # Codex's busy row and Claude's status both carry this hint.
        return "esc to interrupt" in p

    def agent_send(self, text):
        o = self.cfg["owner"]
        return self.owner.serve_call("agent_send", {"repo": o["repo"], "slug": o["lane"],
                                                     "session": self.lane_session(), "text": text})

    # ── the acts ────────────────────────────────────────────────────────
    def open_share(self, who, args):
        """A guest opens the shared lane: in V1 that is the join ask."""
        p = self.person(who)
        if p["drive"] == "cli":
            j = p["vm"].remote_call(p["host"], "share_join", {"share": self.share()}, config_dir=p["config_dir"])
            if j is None:
                return Outcome(False, "share_join refused or failed")
        else:
            self.click(p["vm"], "guest", "open_share")
        st = _until(lambda: self.guest_state(p["principal"]) in ("asked", "allowed")
                    and self.guest_state(p["principal"]), 15)
        return Outcome(bool(st), f"guest {p['principal']} is {st or self.guest_state(p['principal'])}")

    def guest_state(self, principal):
        s = self.owner_share()
        for g in (s or {}).get("guests", []):
            if g["principal"] == principal:
                return g["state"]
        return None

    def allow(self, who, args):
        """The owner lets a guest in, from the share strip."""
        g = self.person(args["guest"])
        self.click(self.owner, "owner", "allow")
        st = _until(lambda: self.guest_state(g["principal"]) == "allowed", 15)
        if not st:
            return Outcome(False, f"guest {g['principal']} is {self.guest_state(g['principal'])}")
        return Outcome(True, "allowed", guest=g["principal"])

    def open_thread(self, who, args):
        """The owner opens the thread panel. Nothing on the host changes, so
        this is a UI step with no proof of its own: the next use_these is."""
        key = "thread" if (self.owner_share() or {}).get("binding", {}).get("state") == "active" else "thread_ended"
        self.click(self.owner, "owner", key)
        self.thread_open = True
        return Outcome(None, "UI only; proved by the next use_these")

    def send_task(self, who, args):
        """The owner types the task into the lane. Typing into one's own
        agent is not what the experiments measure, so it is the CLI."""
        before = self.turn_count()
        j = self.agent_send(args["text"])
        if not (j and j.get("delivered")):
            return Outcome(False, f"agent_send: {j}")
        n = _until(lambda: (self.turn_count() or 0) > (before or 0) and self.turn_count(), 60, 3)
        return Outcome(bool(n), f"turns {before} -> {n}")

    def turn_count(self):
        j = self.owner.serve_call("shared_turns", {"share": self.share(), "limit": 1})
        return j.get("total") if j else None

    def wait(self, who, args):
        if "s" in args:
            time.sleep(args["s"])
            return Outcome(None, f"waited {args['s']}s")
        if args.get("until") == "agent_idle":
            t0 = time.time()
            time.sleep(5)
            # Idle twice in a row, three seconds apart, so a pause between
            # two tool calls is not read as the end of the turn.
            ok = _until(lambda: (not self.agent_busy()) and (time.sleep(3) or not self.agent_busy()),
                        args.get("max_s", 180), 3)
            return Outcome(bool(ok), f"agent idle after {time.time() - t0:.0f}s" if ok else "agent still busy")
        return Outcome(None, "nothing to wait for")

    def post(self, who, args):
        """A guest writes in the thread: the composer and Post, or the CLI for
        a person with no screen."""
        p = self.person(who)
        text = self.run.wording(who, args["text"])
        before = self.last_seq()
        if p["drive"] == "cli":
            pid = f"vmlab-{self.run.run_id}-{int(time.time() * 1000)}"
            j = p["vm"].remote_call(p["host"], "thread_post", {"share": self.share(), "id": pid, "text": text},
                                    config_dir=p["config_dir"])
            if j is None:
                return Outcome(False, "thread_post failed", text=text)
        else:
            vm = p["vm"]
            self.click(vm, "guest", "composer")
            vm.type(text, why="post")
            time.sleep(0.5)
            # Return posts (Shift+Return is a new line). The Post button sits
            # under the "<owner> let you in" toast for its first seconds.
            vm.key(36, why="post")

        def landed():
            for e in (self.thread(before) or []):
                if e["kind"] == "post" and e.get("principal") == p["principal"]:
                    return e
            return None
        e = _until(landed, 20)
        if not e:
            return Outcome(False, "no post by this guest reached the thread", text=text)
        same = e.get("text") == text
        return Outcome(True, f"thread seq {e['seq']}" + ("" if same else " (text changed on the way)"),
                       seq=e["seq"], at=e["at"], text=text, landed_text=e.get("text"), text_intact=same,
                       # macOS smart dashes: `--json` typed, `\u2014json` stored (the rehearsal's defect).
                       dash_mangled=("--" in text and "\u2014" in (e.get("text") or "")))

    def use_these(self, who, args):
        """The owner sends the selected posts to the agent (V1: the panel's
        default selection, every post since the last send), with or without a
        note. Proved by the thread's `shared_with_agent` event and a new turn."""
        if not self.thread_open:
            self.open_thread(who, {})
            time.sleep(1.5)
        before_seq, before_turns = self.last_seq(), self.turn_count()
        self.click(self.owner, "owner", "use_these_after" if self.sent_once else "use_these")
        time.sleep(1.5)
        note = args.get("note") or ""
        # The preview opens with its note field focused, and its layout moves
        # with its content, so it is driven by keys: clear the field (its
        # "Fold these in…" is a placeholder, but a draft may be there), type
        # the note, Return sends. With no note, the app asks first ("Send
        # without telling the agent what to do with them?"): confirm it.
        self.owner.key(0, why="select the note", command=True)
        self.owner.key(51, why="clear the note")
        if note:
            self.owner.type(note, why="note")
        time.sleep(0.5)
        self.owner.key(36, why="send")
        if not note:
            time.sleep(1.5)
            self.click(self.owner, "owner", "use_no_note")

        def shared():
            for e in (self.thread(before_seq) or []):
                if e["kind"] == "shared_with_agent":
                    return e
            return None
        e = _until(shared, 30)
        if not e:
            return Outcome(False, "no shared_with_agent event", note=bool(note))
        self.sent_once = True
        n = _until(lambda: (self.turn_count() or 0) > (before_turns or 0) and self.turn_count(), 60, 3)
        # The send must reach the SHARED session. After a host reboot Codex
        # comes back as a new session with no rollout yet, the binding still
        # reads active, and the send is what ends it: the guest never sees the
        # turn their comments caused (found 2026-11-14).
        ended = None
        if not n:
            ended = _until(lambda: [x for x in (self.thread(e["seq"]) or []) if x["kind"] == "session_ended"], 15, 3)
        proof = f"shared posts {e['first']}..{e['last']} at seq {e['seq']}; turns {before_turns} -> {n}"
        if ended:
            proof += "; the send ENDED the binding: it went to an unshared session"
        return Outcome(bool(n), proof, first=e["first"], last=e["last"], at=e["at"], note=bool(note),
                       note_text=note, sent_to_unshared=bool(ended))

    def read_diff(self, who, args):
        return Outcome(False, "impossible in this variant")

    def ask_share(self, who, args):
        """A guest asks the owner to share another place, with a note."""
        p = self.person(who)
        before = {r.get("id") for r in self.requests()}
        if p["drive"] == "cli":
            p["vm"].remote_call(p["host"], "share_request",
                                {"project": self.cfg["owner"]["repo"], "slug": args["slug"], "note": args.get("note")},
                                config_dir=p["config_dir"])
        else:
            vm = p["vm"]
            # A click while the thread's composer has focus only blurs it.
            self.click(vm, "guest", "blur")
            time.sleep(0.5)
            self.click(vm, "guest", "ask_main")
            time.sleep(1.5)
            if args.get("note"):
                self.click(vm, "guest", "ask_note")
                vm.type(args["note"], why="ask note")
            self.click(vm, "guest", "ask_send")

        def landed():
            return [r for r in self.requests() if r.get("id") not in before]
        new = _until(landed, 15)
        return Outcome(bool(new), f"share_requests_list: {len(new or [])} new", requests=new or [])

    def answer_request(self, who, args):
        """The owner answers a guest's request for another place. V1 (#669)
        offers it as a corner toast; Dismiss is the answer the driver gives."""
        before = self.requests()
        if not before:
            return Outcome(False, "no open request to answer")
        self.click(self.owner, "owner", "request_dismiss")
        gone = _until(lambda: not self.requests(), 15)
        return Outcome(bool(gone), "request dismissed" if gone else "request still open",
                       request=before[0].get("id"))

    def requests(self):
        j = self.owner.serve_call("share_requests_list")
        if isinstance(j, dict):
            j = j.get("requests", [])
        return j or []

    def end_session(self, who, args):
        """The lane's session ends: closed and reopened, then a first turn,
        which is when the share sees another session (the rehearsal's way)."""
        o = self.cfg["owner"]
        self.owner.sh(f"cd {json.dumps(o['repo'])} && worktrees close {o['lane']} >/dev/null 2>&1; sleep 2; "
                      f"worktrees open {o['lane']} --ai {o['harness']} --no-attach >/dev/null 2>&1", timeout=90)
        time.sleep(20)
        self.run.dismiss_harness_modal()
        self.agent_send("Reply with one line: SECOND SESSION.")
        st = _until(lambda: (self.owner_share() or {}).get("binding", {}).get("state") == "ended", 60, 3)
        return Outcome(bool(st), "binding ended" if st else f"binding {(self.owner_share() or {}).get('binding')}")

    def share_again(self, who, args):
        before = len((self.owner_share() or {}).get("ended", []))
        self.click(self.owner, "owner", "share_again")
        time.sleep(1.5)
        self.click(self.owner, "owner", "share_again_confirm")

        def again():
            s = self.owner_share() or {}
            return s.get("binding", {}).get("state") == "active" and len(s.get("ended", [])) > before
        ok = _until(again, 20)
        return Outcome(bool(ok), "binding active again; previous one kept as ended" if ok else "binding not re-bound")

    def stop_sharing(self, who, args):
        s = self.owner_share() or {}
        key = "sharing" if s.get("binding", {}).get("state") == "active" else "sharing_ended"
        self.run.snapshot_host("before-stop")
        self.click(self.owner, "owner", key)
        time.sleep(1.5)
        # The dialog says "N earlier sessions stay readable too" once a Share
        # again has kept one: a line more, and its buttons move down.
        dy = self.ui["owner"].get("stop_earlier_dy", 0) if s.get("ended") else 0
        for name in ("stop", "stop_confirm"):
            x, y = self.at("owner", name)
            self.owner.click(x, y + dy, why=name)
            time.sleep(1.0)
        gone = _until(lambda: self.owner_share() is None, 20)
        return Outcome(bool(gone), "share gone from shares_all" if gone else "share still listed")

    def reboot_owner(self, who, args):
        """A host reboot mid-run: seconds until the owner's serve answers with
        the share intact, and until each guest reads the thread again."""
        r = {}
        self.run.snapshot_host("before-reboot")
        t0 = time.time()
        self.owner.restart()
        time.sleep(15)
        r["ssh_s"] = round(self.owner.wait_ssh(400) + 15, 1)
        app_up = _until(lambda: self.owner.serve_call("shares_all", timeout=20), 60, 3)
        if not app_up:
            # Nothing reopened the app at login: Dana opens it herself.
            self.owner.sh("open -a /Applications/worktrees.app", timeout=20)
            self.owner.counts["click"] += 1
            r["app_reopened_by_hand"] = True
            app_up = _until(lambda: self.owner.serve_call("shares_all", timeout=20), 90, 3)
        r["owner_serve_s"] = round(time.time() - t0, 1) if app_up else None
        kept = self.owner_share()
        r["share_kept"] = bool(kept)
        r["binding_after"] = (kept or {}).get("binding", {}).get("state")
        self.run.normalize_window(self.owner)
        self.run.dismiss_banners(self.owner)
        # Dana starts her agent again: the lane has no session after a boot.
        o = self.cfg["owner"]
        self.owner.sh(f"cd {json.dumps(o['repo'])} && worktrees open {o['lane']} --ai {o['harness']} --no-attach "
                      ">/dev/null 2>&1", timeout=90)
        time.sleep(20)
        self.run.dismiss_harness_modal()
        guests = {}
        for name, p in self.run.people.items():
            if p["role"] != "guest" or not p.get("active"):
                continue
            ok = _until(lambda p=p: p["vm"].remote_call(p["host"], "thread_read", {"share": self.share(), "after": 0},
                                                         config_dir=p["config_dir"], timeout=30), 180, 5)
            guests[name] = round(time.time() - t0, 1) if ok else None
        r["guests_back_s"] = guests
        # The owner's view comes back on the lane and its thread.
        self.click(self.owner, "owner", "open_lane")
        time.sleep(2)
        self.thread_open = False
        self.sent_once = False
        self.open_thread(who, {})
        r["binding_after_agent"] = (self.owner_share() or {}).get("binding", {}).get("state")
        r["total_s"] = round(time.time() - t0, 1)
        proved = bool(app_up) and bool(kept) and all(v is not None for v in guests.values())
        return Outcome(proved, f"owner serve back in {r['owner_serve_s']}s, share kept {bool(kept)}, guests {guests}",
                       recovery=r)


def generate(cmd, prompt, timeout_s, workdir):
    """One sentence from Claude in -p mode, run in an empty directory so no
    repository's hooks or files are near it. None on any failure."""
    env = dict(os.environ, DISABLE_AUTOUPDATER="1")
    env.pop("XDG_DATA_HOME", None)
    os.makedirs(workdir, exist_ok=True)
    try:
        p = subprocess.run(cmd, input=prompt, capture_output=True, text=True, timeout=timeout_s,
                           cwd=workdir, env=env)
    except (OSError, subprocess.TimeoutExpired):
        return None
    out = " ".join(p.stdout.split()).strip().strip('"').strip()
    if p.returncode != 0 or not out or len(out) > 600:
        return None
    return out

