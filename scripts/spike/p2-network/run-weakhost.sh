#!/bin/bash
# §4.1 peer rule part 2: does a packet for the overlay address that arrives on
# the LAN interface reach a listener bound to it (Linux)? Does rp_filter stop
# it? Does SO_BINDTODEVICE scope a LISTENING socket, and who may set it?
# Needs wtspike-host and wtspike-cli on wtspike-lan, started with NET_ADMIN,
# SYS_ADMIN (for /proc/sys rw inside the netns) and /dev/net/tun.
set -u
H() { docker exec wtspike-host "$@"; }
C() { docker exec wtspike-cli "$@"; }
for c in wtspike-host wtspike-cli; do docker exec $c bash -c 'mount -o remount,rw /proc/sys; sysctl -qw net.ipv6.conf.eth0.disable_ipv6=0 net.ipv6.conf.eth0.accept_dad=0'; done
HIP=$(H ip -4 -o addr show eth0 | awk '{print $4}' | cut -d/ -f1)
H ip -6 addr add fd00:77::2/64 dev eth0; C ip -6 addr add fd00:77::3/64 dev eth0
# The host's overlay: a TUN with a CGNAT /32 and a ULA /128, the tailnet routed into it.
H ip tuntap add dev ts0 mode tun; H ip addr add 100.96.1.1/32 dev ts0; H ip -6 addr add fd7a:115c:a1e0::1/128 dev ts0
H ip link set ts0 up; H ip route add 100.64.0.0/10 dev ts0; H ip -6 route add fd7a:115c:a1e0::/48 dev ts0
H bash -c 'nohup python3 /w/tunread.py ts0 > /tmp/tun.log 2>&1 &'; sleep 1
H ip -br link show ts0
# The LAN neighbour aims straight at the host's link-layer address.
C ip route add 100.96.1.1/32 via "$HIP"; C ip -6 route add fd7a:115c:a1e0::1/128 via fd00:77::2
echo "### rp_filter defaults in a fresh netns:"; H sysctl net.ipv4.conf.all.rp_filter net.ipv4.conf.default.rp_filter net.ipv4.conf.eth0.rp_filter
H bash -c 'nohup python3 /w/echo_srv.py 100.96.1.1 7100 > /tmp/srvA.log 2>&1 &'
H bash -c 'nohup python3 /w/echo_srv.py fd7a:115c:a1e0::1 7100 > /tmp/srvA6.log 2>&1 &'; sleep 1
echo "### A. LAN source -> overlay address, plain listener"
for rp in 0 1 2; do
  H sysctl -qw net.ipv4.conf.all.rp_filter=$rp net.ipv4.conf.eth0.rp_filter=$rp net.ipv4.conf.ts0.rp_filter=$rp
  echo -n "rp_filter=$rp: "; C python3 /w/dial.py 100.96.1.1 7100
done
echo -n "IPv6 (no rp_filter for v6): "; C python3 /w/dial.py fd7a:115c:a1e0::1 7100
echo "### B. forged overlay source 100.96.1.2 from the LAN neighbour"
C ip addr add 100.96.1.2/32 dev eth0
for rp in 0 1; do
  H sysctl -qw net.ipv4.conf.all.rp_filter=$rp net.ipv4.conf.eth0.rp_filter=$rp net.ipv4.conf.ts0.rp_filter=$rp
  echo -n "rp_filter=$rp: "; C python3 /w/dial.py 100.96.1.1 7100 100.96.1.2 3
  H ss -tan state syn-recv "( sport = :7100 )" | tail -n +2
done
H sysctl -qw net.ipv4.conf.all.rp_filter=0 net.ipv4.conf.eth0.rp_filter=0 net.ipv4.conf.ts0.rp_filter=0
echo "### C. SO_BINDTODEVICE=ts0 on the LISTENING socket (root)"
H bash -c 'nohup python3 /w/echo_srv.py 100.96.1.1 7101 ts0 > /tmp/srvC.log 2>&1 &'
H bash -c 'nohup python3 /w/echo_srv.py fd7a:115c:a1e0::1 7101 ts0 > /tmp/srvC6.log 2>&1 &'; sleep 1
echo -n "LAN neighbour v4: "; C python3 /w/dial.py 100.96.1.1 7101
echo -n "LAN neighbour v6: "; C python3 /w/dial.py fd7a:115c:a1e0::1 7101
echo -n "same host, source on ts0 (the CI-witness shape): "; H python3 /w/dial.py 100.96.1.1 7101 100.96.1.1
echo -n "same host, client socket also SO_BINDTODEVICE ts0: "; H python3 /w/dial.py 100.96.1.1 7101 dev:ts0
echo -n "same host, SO_BINDTODEVICE=lo client: "; H python3 /w/dial.py 100.96.1.1 7101 dev:lo
echo "### D. who may set SO_BINDTODEVICE on a listener"
NOB="setpriv --reuid=65534 --regid=65534 --clear-groups --inh-caps=-all --bounding-set=-all"
H bash -c "$NOB timeout 2 python3 /w/echo_srv.py 100.96.1.1 7102 ts0" 2>&1 | head -3
echo "root without CAP_NET_RAW:"; H bash -c "capsh --drop=cap_net_raw -- -c 'timeout 2 python3 /w/echo_srv.py 100.96.1.1 7103 ts0'" 2>&1 | head -3
echo "### logs"; for f in srvA srvA6 srvC srvC6 tun; do echo "-- $f"; H cat /tmp/$f.log; done
