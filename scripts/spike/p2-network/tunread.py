#!/usr/bin/env python3
"""Attach to an existing TUN device (so its carrier is up, as Tailscale's is)
and log a one-line summary of every IPv4/IPv6 TCP packet the kernel routes
INTO it, i.e. what would leave through the overlay. Spike probe, §4.1."""
import fcntl, os, socket, struct, sys, time
name = sys.argv[1]
fd = os.open("/dev/net/tun", os.O_RDWR)
fcntl.ioctl(fd, 0x400454ca, struct.pack("16sH", name.encode(), 0x0001 | 0x1000))  # TUNSETIFF, IFF_TUN|IFF_NO_PI
T0 = time.time()
FL = "FSRPAU"
while True:
    p = os.read(fd, 65535)
    v = p[0] >> 4
    if v == 4:
        proto, src, dst, off = p[9], socket.inet_ntop(socket.AF_INET, p[12:16]), socket.inet_ntop(socket.AF_INET, p[16:20]), (p[0] & 15) * 4
    elif v == 6:
        proto, src, dst, off = p[6], socket.inet_ntop(socket.AF_INET6, p[8:24]), socket.inet_ntop(socket.AF_INET6, p[24:40]), 40
    else:
        continue
    if proto != 6: continue
    sp, dp, fl = struct.unpack("!HH", p[off:off+4]) + (p[off+13],)
    print(f"[tun {time.time()-T0:6.2f}] out via {name}: {src}:{sp} -> {dst}:{dp} [{''.join(c for i,c in enumerate(FL) if fl>>i&1)}]", flush=True)
