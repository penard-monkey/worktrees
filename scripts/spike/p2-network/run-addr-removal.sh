#!/bin/bash
# Orchestrates the §4.1 "listener whose address is removed" probe across two
# containers (wtspike-host, wtspike-cli) on the wtspike-lan network. Run on
# the Mac with bash; every address change happens inside wtspike-host's netns.
# Usage: run-addr-removal.sh 4|6
set -u
FAM=${1:-4}
H() { docker exec wtspike-host "$@"; }
C() { docker exec wtspike-cli "$@"; }
cmd() { C bash -c "echo '$1' > /tmp/cmd"; }
if [ "$FAM" = 6 ]; then A=fd00:77::50; PL=64; ADDR6=1; else A=10.77.0.50; PL=24; ADDR6=; fi
H pkill -f echo_srv.py; C pkill -f fifo_cli.py; sleep 0.3
if [ -n "$ADDR6" ]; then
  for c in wtspike-host wtspike-cli; do docker exec $c bash -c 'mount -o remount,rw /proc/sys; sysctl -qw net.ipv6.conf.eth0.disable_ipv6=0 net.ipv6.conf.eth0.accept_dad=0'; done
  C ip -6 addr add fd00:77::99/64 dev eth0
fi
H ip link add dm0 type dummy 2>/dev/null; H ip link set dm0 up
H ip addr add $A/$PL dev eth0; sleep 1
H bash -c "nohup python3 /w/echo_srv.py $A 7000 > /tmp/srv.log 2>&1 &"; sleep 1
C bash -c "rm -f /tmp/cmd; mkfifo /tmp/cmd; nohup python3 /w/fifo_cli.py $A 7000 /tmp/cmd > /tmp/cli.log 2>&1 &"; sleep 1
cmd "send a1"; sleep 2.5
echo "### delete $A from eth0"; H ip addr del $A/$PL dev eth0; sleep 0.5
H ss -tna "( sport = :7000 )"
cmd "send a2-after-del"; sleep 1; cmd "new"; sleep 4
echo "### a second bind of the removed address:"; H python3 /w/bindtry.py $A 7001
H ss -tna "( sport = :7000 )"
echo "### re-add $A to eth0"; H ip addr add $A/$PL dev eth0; sleep 1
echo "### a second bind of the SAME port after re-add:"; H python3 /w/bindtry.py $A 7000
cmd "new"; sleep 6
echo "### move $A to dm0 (another interface)"; H ip addr del $A/$PL dev eth0; H ip addr add $A/128 dev dm0 2>/dev/null || H ip addr add $A/32 dev dm0; sleep 1
cmd "new"; cmd "send a3-addr-on-dm0"; sleep 4
echo "#### server log"; H cat /tmp/srv.log; echo "#### client log"; C cat /tmp/cli.log
H ip addr del $A/128 dev dm0 2>/dev/null; H ip addr del $A/32 dev dm0 2>/dev/null; H pkill -f echo_srv.py; C pkill -f fifo_cli.py
true
