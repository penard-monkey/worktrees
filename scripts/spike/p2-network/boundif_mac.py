#!/usr/bin/env python3
"""macOS: does IP_BOUND_IF / IPV6_BOUND_IF scope a LISTENING socket?

Each case binds a listener, optionally ties it to an interface, and dials it
from this same Mac with a 1 s timeout. Same-host traffic arrives on lo0, so a
listener tied to ANOTHER interface that still accepts it is not scoped.
Loopback cases change nothing and expose nothing. With --own, also binds the
Mac's own en0 and overlay addresses for well under a second each (the
listener closes after the one dial). Run as an ordinary user: that is also
the privilege test. Spike probe for §4.1 peer rule part 2; not product code.
"""
import socket, subprocess, sys

IP_BOUND_IF, IPV6_BOUND_IF = 25, 125  # <netinet/in.h>, measured with cc on macOS 27


def case(label, addr, bound_if, src=None):
    fam = socket.AF_INET6 if ":" in addr else socket.AF_INET
    s = socket.socket(fam, socket.SOCK_STREAM)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    note = ""
    if bound_if:
        idx = socket.if_nametoindex(bound_if)
        try:
            if fam == socket.AF_INET:
                s.setsockopt(socket.IPPROTO_IP, IP_BOUND_IF, idx)
            else:
                s.setsockopt(socket.IPPROTO_IPV6, IPV6_BOUND_IF, idx)
        except OSError as e:
            print(f"{label}: setsockopt BOUND_IF {bound_if} -> {e!r}")
            return
        note = f" BOUND_IF={bound_if}({idx})"
    s.bind((addr, 0))
    s.listen(4)
    port = s.getsockname()[1]
    c = socket.socket(fam, socket.SOCK_STREAM)
    if src:
        c.bind((src, 0))
    c.settimeout(1)
    try:
        c.connect((addr, port))
        s.settimeout(1)
        a, peer = s.accept()
        res = f"ACCEPTED from {peer[0]}"
        a.close()
    except OSError as e:
        res = f"{e!r}"
    finally:
        c.close()
        s.close()
    print(f"{label}: listen {addr}{note}; dial from this Mac -> {res}")


def ifaddr(name, fam):
    out = subprocess.run(["ifconfig", name], capture_output=True, text=True).stdout
    for line in out.splitlines():
        w = line.split()
        if w and w[0] == fam and not w[1].startswith("fe80"):
            return w[1]
    return None


case("L1", "127.0.0.1", None)
case("L2", "127.0.0.1", "lo0")
case("L3", "127.0.0.1", "en0")
case("L4", "::1", None)
case("L5", "::1", "lo0")
case("L6", "::1", "en0")
if "--own" in sys.argv:
    en0 = ifaddr("en0", "inet")
    ov = sys.argv[sys.argv.index("--own") + 1]
    ov4 = ifaddr(ov, "inet")
    ov6 = ifaddr(ov, "inet6")
    case("O1", en0, None)
    case("O2", en0, "en0")
    case("O3", en0, ov)
    case("O4", ov4, None)
    case("O5", ov4, ov)
    case("O6", ov4, "en0")
    case("O7", ov6, ov)
    case("O8", ov6, "en0")
