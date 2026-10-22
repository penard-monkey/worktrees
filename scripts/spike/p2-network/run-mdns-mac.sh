#!/bin/bash
# §4.7 on macOS: a TEST _worktrees._tcp advert registered through
# mDNSResponder on en0's index, seen by dns-sd -B/-L and by a passive group
# listener on en0/awdl0/utun; withdrawn cleanly, then by a SIGKILLed
# registrant. Changes no network configuration; each advert lives < 10 s.
set -u
B=~/.cache/worktrees/worktrees/spike-p2-network/bin
L=~/.cache/worktrees/worktrees/spike-p2-network/logs
OV=${1:-utun6}
ID1=$(python3 -c 'import base64,secrets;print(base64.b32encode(secrets.token_bytes(16)).decode().rstrip("=").lower())')
ID2=$(python3 -c 'import base64,secrets;print(base64.b32encode(secrets.token_bytes(16)).decode().rstrip("=").lower())')
HN=$(scutil --get LocalHostName)
python3 ./mdns_watch.py 30 "_worktrees,$ID1,$ID2" en0 awdl0 "$OV" > $L/mdns-mac-watch.log 2>&1 &
W=$!
script -q /dev/null dns-sd -B _worktrees._tcp > $L/mdns-mac-browse.log 2>&1 &
BR=$!
sleep 2
echo "### E1 register $ID1 on en0, release after 6 s"
$B/mdns_reg_mac en0 7443 6 release "$ID1" &
R=$!
sleep 2
script -q /dev/null dns-sd -L "$ID1" _worktrees._tcp local > $L/mdns-mac-resolve.log 2>&1 &
RS=$!; sleep 2; kill $RS 2>/dev/null
script -q /dev/null dns-sd -Gv4v6 "$HN.local" > $L/mdns-mac-hostaddr.log 2>&1 &
G=$!; sleep 2; kill $G 2>/dev/null
wait $R
sleep 3
echo "### E2 register $ID2 on en0, SIGKILL after 5 s"
$B/mdns_reg_mac en0 7443 5 kill "$ID2"
sleep 4
kill $BR 2>/dev/null; wait $W 2>/dev/null
echo "#### dns-sd -B"; cat $L/mdns-mac-browse.log
echo "#### dns-sd -L"; cat $L/mdns-mac-resolve.log
echo "#### dns-sd -G (the SRV target's addresses)"; sed -E 's/(fe80::|fd[0-9a-f]{2}:)[0-9a-f:]+/\1…/' $L/mdns-mac-hostaddr.log
echo "#### passive watcher"; cat $L/mdns-mac-watch.log
echo "#### leftovers: dns-sd -B for 3 s after everything ended"
script -q /dev/null dns-sd -B _worktrees._tcp & X=$!; sleep 3; kill $X
