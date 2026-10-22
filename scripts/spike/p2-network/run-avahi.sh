#!/bin/bash
# §4.7 on Linux: Avahi registration on ONE interface, seen (or not) from a peer
# on each of the host's two links; the goodbye on Reset(), on a crashed
# registrant, and when the bound address leaves the interface.
# Needs wtspike-mdhost (eth0=wtspike-lan, eth1=wtspike-lan2), wtspike-peerA
# (lan), wtspike-peerB (lan2), each with avahi up (avahi-up.sh).
set -u
X() { docker exec "wtspike-$1" "${@:2}"; }
for p in peerA peerB; do
  X $p bash -c 'pkill avahi-browse; pkill tcpdump; rm -f /tmp/browse.log /tmp/dump.log
    nohup bash -c "avahi-browse -prk _worktrees._tcp | while IFS= read -r l; do echo \"\$(date +%s.%3N) \$l\"; done" > /tmp/browse.log 2>&1 &
    nohup tcpdump -tt -n -l -vvv -i eth0 udp port 5353 > /tmp/dump.log 2>&1 &'
done
X mdhost bash -c 'pkill tcpdump; nohup tcpdump -tt -n -l -vvv -i eth0 udp port 5353 > /tmp/dump0.log 2>&1 &
  nohup tcpdump -tt -n -l -vvv -i eth1 udp port 5353 > /tmp/dump1.log 2>&1 &'
sleep 2
echo "### E1: register on eth0, withdraw with EntryGroup.Reset()"
X mdhost python3 /w/avahi_reg.py eth0 7443 4 reset
sleep 3
echo "### E2: register on eth0, then the registrant is SIGKILLed (a crashed serve)"
X mdhost python3 /w/avahi_reg.py eth0 7443 4 kill
sleep 4
echo "### E3: registered on eth0; the host's IPv4 address leaves eth0, then returns"
X mdhost bash -c 'nohup python3 /w/avahi_reg.py eth0 7443 0 hold > /tmp/reg3.log 2>&1 &'; sleep 4
A=$(X mdhost ip -4 -o addr show eth0 | awk '{print $4}')
echo "$(date +%s.%3N) deleting $A from eth0"; X mdhost ip addr del "$A" dev eth0; sleep 5
echo "$(date +%s.%3N) re-adding $A to eth0"; X mdhost ip addr add "$A" dev eth0; sleep 6
X mdhost cat /tmp/reg3.log; X mdhost pkill -f avahi_reg.py; sleep 3
for p in peerA peerB; do echo "#### $p avahi-browse"; X $p cat /tmp/browse.log; done
for p in peerA peerB; do echo "#### $p tcpdump, packets naming _worktrees"; X $p grep -c . /tmp/dump.log; X $p bash -c "grep -B1 -A8 -i 'worktrees' /tmp/dump.log | head -150"; done
echo "#### mdhost eth1 tcpdump lines naming _worktrees:"; X mdhost grep -ci worktrees /tmp/dump1.log
