#!/usr/bin/env python3
"""Register _worktrees._tcp through Avahi over D-Bus on ONE interface (by
index), exactly as §4.7 describes serve doing it, then withdraw it.
Usage: avahi_reg.py IFACE PORT HOLD_SECONDS reset|kill|hold
  reset: EntryGroup.Reset() after HOLD (the clean withdrawal)
  kill:  SIGKILL ourselves after HOLD (a crashed serve)
  hold:  stay registered until killed from outside
The instance name and TXT id are the §2.1 discovery id: 128 random bits in
base32. Spike probe for §4.7; not product code."""
import base64, os, secrets, signal, sys, time
import dbus
T0 = time.time()
def log(m): print(f"[reg {time.time():.3f} +{time.time()-T0:5.2f}] {m}", flush=True)
ifname, port, hold, mode = sys.argv[1], int(sys.argv[2]), float(sys.argv[3]), sys.argv[4]
bus = dbus.SystemBus()
server = dbus.Interface(bus.get_object("org.freedesktop.Avahi", "/"), "org.freedesktop.Avahi.Server")
idx = server.GetNetworkInterfaceIndexByName(ifname)
group = dbus.Interface(bus.get_object("org.freedesktop.Avahi", server.EntryGroupNew()),
                       "org.freedesktop.Avahi.EntryGroup")
disc = base64.b32encode(secrets.token_bytes(16)).decode().rstrip("=").lower()
txt = [dbus.Array([dbus.Byte(b) for b in s], signature="y") for s in (b"v=1", b"id=" + disc.encode())]
group.AddService(dbus.Int32(idx), dbus.Int32(-1), dbus.UInt32(0), disc, "_worktrees._tcp", "", "",
                 dbus.UInt16(port), dbus.Array(txt, signature="ay"))
group.Commit()
log(f"committed on {ifname} (avahi if index {idx}), name/id={disc} len={len(disc)}")
for _ in range(100):
    st = group.GetState()
    if st == 2: log("ESTABLISHED"); break
    if st in (3, 4): log(f"state {st} (collision/failure)"); break
    time.sleep(0.05)
time.sleep(hold)
if mode == "reset":
    group.Reset(); log("Reset() — withdrawn"); time.sleep(3)
elif mode == "kill":
    log("SIGKILL self"); os.kill(os.getpid(), signal.SIGKILL)
else:
    while True: time.sleep(60)
