#!/bin/bash
# Linux half of the §4.1 fingerprint measurement. Runs INSIDE a throwaway
# container (own netns, NET_ADMIN, /dev/net/tun); every change is to the
# container's namespace. Reads are done as uid 65534 with no capabilities.
set -u
cd /w
cc -O1 -Wall -o /tmp/netfp netfp.c || exit 1
nobody() { setpriv --reuid=65534 --regid=65534 --clear-groups --inh-caps=-all --bounding-set=-all "$@"; }
sysctl -qw net.ipv6.conf.eth0.use_tempaddr=2
ip -6 addr add fd00:77::10/64 dev eth0 mngtmpaddr
ip -6 addr add fd00:77::20/64 dev eth0 preferred_lft 0          # deprecated on purpose
ip tuntap add dev ts0 mode tun                                    # Tailscale-like overlay (TUN)
ip addr add 100.96.1.1/32 dev ts0
ip -6 addr add fd7a:115c:a1e0::1/128 dev ts0
ip link set ts0 up
ip tuntap add dev zt0 mode tap                                    # ZeroTier-like overlay (TAP)
ip addr add 10.147.17.5/24 dev zt0; ip link set zt0 up
ip link add wg0 type wireguard 2>/tmp/wgerr && { ip addr add 10.9.0.1/24 dev wg0; ip link set wg0 up; } || echo "wireguard link: $(cat /tmp/wgerr)"
ip link add dm0 type dummy; ip addr add 192.168.50.5/24 dev dm0; ip link set dm0 up
sleep 2   # let DAD finish and the temporary address appear
echo "== link flags (ip -br link)"; ip -br link
echo "== /sys/class/net/*/flags (IFF_POINTOPOINT=0x10)"; for i in eth0 ts0 zt0 wg0 dm0; do [ -e /sys/class/net/$i ] && echo "$i $(cat /sys/class/net/$i/flags)"; done
GW=$(ip -4 route show default | awk '{print $3}')
echo "== gateway neighbour BEFORE any traffic to it (as root: ip neigh)"; ip neigh show "$GW"; echo "(end)"
ping -c1 -W1 "$GW" >/dev/null; echo "== after one ping"; ip neigh show "$GW"
echo "== netfp as uid 65534, no caps"; nobody /tmp/netfp
echo "== /proc/net/arp and ip neigh as uid 65534"; nobody cat /proc/net/arp; nobody ip neigh show
echo "== policy routing: a default via the overlay in table 52, as Tailscale's exit node does"
ip route add default dev ts0 table 52; ip rule add pref 5270 lookup 52
nobody /tmp/netfp | grep -E '^default|neigh'
echo "== neighbour flushed (as a quiet period or a GC would leave it)"; ip neigh flush all
nobody /tmp/netfp | grep -E '^default|neigh'
