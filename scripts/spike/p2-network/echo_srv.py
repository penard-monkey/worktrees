#!/usr/bin/env python3
"""Listener for the address-removal probe: logs every accept, receive and
server-side send, and sends a 'tick' on each connection every 2 s so the
server->client direction is visible too. Usage: echo_srv.py ADDR PORT [IFACE]
(IFACE: SO_BINDTODEVICE). Spike probe, §4.1; not product code."""
import socket, sys, threading, time
addr, port = sys.argv[1], int(sys.argv[2])
dev = sys.argv[3] if len(sys.argv) > 3 else None
T0 = time.time()
def log(m): print(f"[srv {time.time()-T0:6.2f}] {m}", flush=True)
fam = socket.AF_INET6 if ":" in addr else socket.AF_INET
s = socket.socket(fam, socket.SOCK_STREAM)
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
if dev:
    try:
        s.setsockopt(socket.SOL_SOCKET, socket.SO_BINDTODEVICE, dev.encode())
        log(f"SO_BINDTODEVICE {dev} ok")
    except OSError as e:
        log(f"SO_BINDTODEVICE {dev} FAILED {e!r}")
if "%" in addr:
    a, zone = addr.split("%")
    target = (a, port, 0, socket.if_nametoindex(zone))
else:
    target = (addr, port)
s.bind(target); s.listen(16)
log(f"listening on {s.getsockname()}")
conns = []
def handle(c, peer):
    while True:
        try: d = c.recv(200)
        except OSError as e: log(f"recv error {peer} {e!r}"); return
        if not d: log(f"eof {peer}"); return
        log(f"recv {d!r} from {peer[0]}")
def ticker():
    n = 0
    while True:
        time.sleep(2); n += 1
        for c, peer in list(conns):
            try: c.send(f"tick{n};".encode())
            except OSError as e: log(f"send tick{n} to {peer[0]} -> {e!r}"); conns.remove((c, peer))
threading.Thread(target=ticker, daemon=True).start()
while True:
    try: c, peer = s.accept()
    except OSError as e: log(f"accept error {e!r}"); time.sleep(1); continue
    log(f"accept {peer[0]}:{peer[1]} local={c.getsockname()[0]}")
    conns.append((c, peer))
    threading.Thread(target=handle, args=(c, peer), daemon=True).start()
