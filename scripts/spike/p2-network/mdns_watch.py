#!/usr/bin/env python3
"""Passive mDNS watcher, no root: binds the mDNS GROUP address (never the
wildcard, so it cannot take unicast meant for the system responder) with
SO_REUSEPORT, joins 224.0.0.251 / ff02::fb on the named interfaces, and
prints every record in every multicast mDNS message whose name or data
mentions one of the FILTER words, with its TTL. Records of other hosts are
not printed unless they match the filter.

Usage: mdns_watch.py SECONDS FILTER[,FILTER...] IFACE [IFACE...]
Spike probe for §4.7; not product code."""
import select, socket, struct, sys, time

secs, filt, ifaces = float(sys.argv[1]), sys.argv[2].lower().split(","), sys.argv[3:]
TYPES = {1: "A", 12: "PTR", 16: "TXT", 28: "AAAA", 33: "SRV", 47: "NSEC", 255: "ANY"}


def name(b, o, depth=0):
    labels = []
    while True:
        n = b[o]
        if n & 0xC0 == 0xC0:
            ptr = struct.unpack("!H", b[o:o + 2])[0] & 0x3FFF
            labels.append(name(b, ptr, depth + 1)[0])
            return ".".join(labels), o + 2
        o += 1
        if n == 0:
            return ".".join(labels), o
        labels.append(b[o:o + n].decode("utf-8", "replace"))
        o += n


def rdata(b, t, o, ln):
    d = b[o:o + ln]
    if t == 1: return socket.inet_ntop(socket.AF_INET, d)
    if t == 28: return socket.inet_ntop(socket.AF_INET6, d)
    if t == 12: return name(b, o)[0]
    if t == 33:
        pr, w, port = struct.unpack("!HHH", d[:6]); return f"{name(b, o + 6)[0]}:{port}"
    if t == 16:
        out, i = [], 0
        while i < len(d):
            out.append(d[i + 1:i + 1 + d[i]].decode("utf-8", "replace")); i += 1 + d[i]
        return " ".join(repr(x) for x in out)
    return d.hex()


def parse(b):
    _, flags, qd, an, ns, ar = struct.unpack("!HHHHHH", b[:12])
    o, recs = 12, []
    for _ in range(qd):
        n, o = name(b, o); t, c = struct.unpack("!HH", b[o:o + 4]); o += 4
        recs.append(("Q", n, TYPES.get(t, t), None, ""))
    for sect, cnt in (("AN", an), ("NS", ns), ("AR", ar)):
        for _ in range(cnt):
            n, o = name(b, o); t, c, ttl, ln = struct.unpack("!HHIH", b[o:o + 10]); o += 10
            recs.append((sect, n, TYPES.get(t, t), ttl, rdata(b, t, o, ln))); o += ln
    return flags & 0x8000, recs


def ipv4_of(ifn):
    import subprocess
    for line in subprocess.run(["ifconfig", ifn], capture_output=True, text=True).stdout.splitlines():
        w = line.split()
        if w and w[0] == "inet":
            return w[1]
    return None


socks = []
for ifn in ifaces:
    idx = socket.if_nametoindex(ifn)
    a4 = ipv4_of(ifn)
    if a4:  # ip_mreq names the interface by its IPv4 address
        s4 = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        s4.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); s4.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEPORT, 1)
        s4.bind(("224.0.0.251", 5353))
        s4.setsockopt(socket.IPPROTO_IP, socket.IP_ADD_MEMBERSHIP, socket.inet_aton("224.0.0.251") + socket.inet_aton(a4))
        socks.append((s4, ifn, "v4"))
    try:
        s6 = socket.socket(socket.AF_INET6, socket.SOCK_DGRAM)
        s6.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); s6.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEPORT, 1)
        s6.bind(("ff02::fb", 5353, 0, idx))
        s6.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_JOIN_GROUP, socket.inet_pton(socket.AF_INET6, "ff02::fb") + struct.pack("@I", idx))
        socks.append((s6, ifn, "v6"))
    except OSError as e:
        print(f"[watch] v6 on {ifn}: {e!r}", flush=True)
print(f"[watch {time.time():.3f}] listening on {[(i, f) for _, i, f in socks]} filter={filt}", flush=True)
end = time.time() + secs
while time.time() < end:
    r, _, _ = select.select([s for s, _, _ in socks], [], [], 0.5)
    for s in r:
        b, src = s.recvfrom(9000)
        ifn, fam = next((i, f) for x, i, f in socks if x is s)
        try:
            resp, recs = parse(b)
        except Exception as e:
            continue
        hit = [x for x in recs if any(w in (x[1] + " " + str(x[4])).lower() for w in filt)]
        if not hit:
            continue
        print(f"[watch {time.time():.3f}] {fam} {ifn} from {src[0]} {'response' if resp else 'query'}:", flush=True)
        for sect, n, t, ttl, d in recs:
            print(f"    {sect} {n} {t} ttl={ttl} {d}", flush=True)
