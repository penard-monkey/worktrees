#!/bin/bash
# §4.1 "Link-local needs its interface": dial link-local addresses from a
# string between two containers that share TWO links (eth0 on wtspike-lan,
# eth1 on wtspike-lan2). Changes only the containers' namespaces.
set -u
H() { docker exec wtspike-host "$@"; }
C() { docker exec wtspike-cli "$@"; }
P=/b/llprobe-linux
for c in wtspike-host wtspike-cli; do docker exec $c bash -c 'mount -o remount,rw /proc/sys 2>/dev/null; sysctl -qw net.ipv6.conf.eth1.disable_ipv6=0 net.ipv6.conf.eth0.disable_ipv6=0'; done
sleep 2
HLL1=$(H ip -6 -o addr show dev eth1 scope link | awk '{print $4}' | cut -d/ -f1)
echo "host eth1 link-local: $HLL1"
H pkill -f llprobe-linux
H bash -c "nohup $P listen '$HLL1%eth1' 7401 > /tmp/ll6.log 2>&1 &"
echo "### IPv6 link-local, host's address lives on the eth1 link"
C $P dial "$HLL1" 7401
C $P dial "$HLL1" 7401 eth1
C $P dial "$HLL1" 7401 eth0
echo "### IPv4 link-local: both links carry 169.254.0.0/16"
H ip addr add 169.254.20.1/16 dev eth0; C ip addr add 169.254.20.2/16 dev eth0
H ip addr add 169.254.30.1/16 dev eth1; C ip addr add 169.254.30.2/16 dev eth1
C ip -4 route show | grep 169.254
echo -n "route the client takes for 169.254.30.1: "; C ip route get 169.254.30.1 | head -1
H bash -c "nohup $P listen 169.254.30.1 7402 > /tmp/ll4.log 2>&1 &"; sleep 0.5
echo "host arp_ignore=0 (Linux default: answer ARP for any local address on any link):"
C ip neigh flush all; C $P dial 169.254.30.1 7402
C ip neigh show 169.254.30.1
echo "host arp_ignore=1 (answer only for addresses on the receiving link, as a strict host would):"
H sysctl -qw net.ipv4.conf.all.arp_ignore=1; C ip neigh flush all; C $P dial 169.254.30.1 7402
echo "same, client socket SO_BINDTODEVICE eth1:"; C ip neigh flush all; C python3 /w/dial.py 169.254.30.1 7402 dev:eth1
H sysctl -qw net.ipv4.conf.all.arp_ignore=0
echo "### listener logs"; H cat /tmp/ll6.log /tmp/ll4.log
H pkill -f llprobe-linux; true
