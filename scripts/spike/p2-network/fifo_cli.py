#!/usr/bin/env python3
"""Client for the address-removal probe. Holds one connection open; reads
commands from a FIFO: 'send X' sends on it, 'new' tries a fresh connect with a
3 s timeout. Logs everything the server sends. Usage: fifo_cli.py ADDR PORT FIFO
[SRC] (SRC: bind this source address first). Spike probe, §4.1."""
import os, socket, sys, threading, time
addr, port, fifo = sys.argv[1], int(sys.argv[2]), sys.argv[3]
src = sys.argv[4] if len(sys.argv) > 4 else None
T0 = time.time()
def log(m): print(f"[cli {time.time()-T0:6.2f}] {m}", flush=True)
fam = socket.AF_INET6 if ":" in addr else socket.AF_INET
def dial(timeout):
    c = socket.socket(fam, socket.SOCK_STREAM)
    if src: c.bind((src, 0))
    c.settimeout(timeout)
    c.connect((addr, port))
    c.settimeout(None)
    return c
c = dial(5)
log(f"connected {c.getsockname()[0]} -> {addr}:{port}")
def reader():
    while True:
        try: d = c.recv(200)
        except OSError as e: log(f"recv error {e!r}"); return
        if not d: log("eof"); return
        log(f"recv {d!r}")
threading.Thread(target=reader, daemon=True).start()
if not os.path.exists(fifo): os.mkfifo(fifo)
while True:
    with open(fifo) as f:
        for line in f:
            line = line.strip()
            if line.startswith("send "):
                try: c.send(line[5:].encode()); log(f"sent {line[5:]!r}")
                except OSError as e: log(f"send error {e!r}")
            elif line == "new":
                try: n = dial(3); log(f"NEW connect ok local={n.getsockname()[0]}"); n.close()
                except OSError as e: log(f"NEW connect failed {e!r}")
