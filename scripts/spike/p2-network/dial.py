#!/usr/bin/env python3
"""One connect attempt: dial.py ADDR PORT [SRC] [TIMEOUT]. Prints ok/refused/timeout.
SRC may be 'dev:IFACE' to SO_BINDTODEVICE instead of binding an address.
ADDR may carry a %zone. Spike probe, §4.1."""
import socket, sys, time
a, p = sys.argv[1], int(sys.argv[2])
src = sys.argv[3] if len(sys.argv) > 3 and sys.argv[3] != "-" else None
to = float(sys.argv[4]) if len(sys.argv) > 4 else 3
fam = socket.AF_INET6 if ":" in a else socket.AF_INET
s = socket.socket(fam, socket.SOCK_STREAM)
if src and src.startswith("dev:"): s.setsockopt(socket.SOL_SOCKET, socket.SO_BINDTODEVICE, src[4:].encode())
elif src: s.bind((src, 0))
s.settimeout(to)
t = time.time()
target = (a, p)
if "%" in a:
    h, z = a.split("%"); target = (h, p, 0, socket.if_nametoindex(z))
try:
    s.connect(target); print(f"connect {a}:{p} from {s.getsockname()[0]} -> OK ({time.time()-t:.2f}s)")
except OSError as e:
    print(f"connect {a}:{p} src={src} -> {e!r} ({time.time()-t:.2f}s)")
