# Phase 2 network spike: probes

Throwaway probes for the claims marked *unmeasured* in
`docs/proposals/server-edition-phase2.md` §4.1, §4.6, §4.7 and §10. They are
not product code, nothing builds or tests them, and none of them changes the
network configuration of the machine they run on. The Linux scripts change
only the namespaces of throwaway containers. What is still to measure on real
hardware, with the exact commands, is [`CHECKLIST.md`](CHECKLIST.md), beside
this file. The findings and raw evidence stay outside the repo, in the spike
lane's `~/.cache/worktrees/worktrees/spike-p2-network/` on the machine that
ran it.

| Probe | Runs on | What it answers |
|---|---|---|
| `netfp.c` | macOS, Linux | The §4.1 fingerprint inputs as a non-root process: point-to-point flag, prefix, IPv6 temporary/deprecated, every default route, the gateway's neighbour entry. `cc -O1 -Wall -o netfp netfp.c` |
| `linux-fingerprint.sh` | Linux container | `netfp` as uid 65534 against TUN, TAP, WireGuard and dummy interfaces, policy routing and a flushed neighbour table |
| `run-addr-removal.sh 4\|6` + `echo_srv.py`, `fifo_cli.py`, `bindtry.py` | Mac → two containers | What the kernel does to a listener, and to its connections, when its address is removed, re-added, or moved to another interface |
| `run-weakhost.sh` + `tunread.py`, `dial.py` | Mac → two containers | Weak host on Linux under each `rp_filter`; a forged overlay source; `SO_BINDTODEVICE` on a listener, and who may set it |
| `boundif_mac.py [--own <overlay-if>]` | macOS | Whether `IP_BOUND_IF` / `IPV6_BOUND_IF` scope a listening socket (same-host dials) |
| `llprobe.rs` | macOS, Linux | Rust std's parsing of zones and odd IPv4 spellings against the resolver's; dialling link-local addresses with and without a scope id. `rustc -O --edition 2021 llprobe.rs` |
| `run-linklocal.sh` | Mac → two containers on two links | Link-local IPv6 and IPv4 dials when two links both carry link-local addresses |
| `avahi-up.sh`, `avahi_reg.py`, `run-avahi.sh` | Mac → three containers | `_worktrees._tcp` through Avahi over D-Bus on one interface; the goodbye on `Reset()`, on a killed registrant, and when the address leaves |
| `mdns_reg_mac.c`, `mdns_watch.py`, `run-mdns-mac.sh` | macOS | The same through `DNSServiceRegister` with an interface index; withdrawn cleanly and by SIGKILL |
| `keychain_probe.swift` | macOS | Login keychain across two ad-hoc builds, the data-protection keychain, Secure Enclave keys, with user interaction disabled so nothing prompts |

The container scripts expect containers named `wtspike-*` on networks
`wtspike-lan` / `wtspike-lan2`, started as the findings describe, and an image
with iproute2, tcpdump, avahi-daemon, avahi-utils, dbus, python3-dbus and gcc.
Remove all of them afterwards.
