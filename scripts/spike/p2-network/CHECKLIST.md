# Phase 2 network spike: the hardware checklist

What the spike of 2026-10-21 could not measure without an iPhone, a second
Mac, a reboot or a CI runner. Each step answers one item of §10's "Still to
measure" list in `docs/proposals/server-edition-phase2.md`, which names it in
brackets (A1–D2). Record the output of every command; the expected result is
there to compare against, not to assume.

"Mac A" runs serve. "Mac B" is a second Mac on the same Wi-Fi and the same
tailnet. Steps marked **sudo** change Mac A's network configuration for the
length of the step and say how to undo it.

## Fill these in first

Every command below assumes Mac A's Wi-Fi is `en0`
(`networksetup -listallhardwareports` says which it is); substitute yours if
not. Then look these up and replace them in the commands:

| Placeholder | What it is | How to find it (on Mac A unless noted) |
|---|---|---|
| `<MAC_A_LAN_IP>` | Mac A's Wi-Fi IPv4 address | `ipconfig getifaddr en0` |
| `<EN0_LL>` | Mac A's Wi-Fi IPv6 link-local address, without the `%en0` | `ifconfig en0 inet6 \| awk '/fe80/{sub(/%.*/,"",$2); print $2}'` |
| `<MAC_A_TAILSCALE_IP>` | Mac A's Tailscale IPv4 address (`100.x`) | `tailscale ip -4` |
| `<MAC_A_TAILSCALE_IP6>` | Mac A's Tailscale IPv6 address (`fd7a:…`) | `tailscale ip -6` |
| `<TS_UTUN>` | the `utun` interface Tailscale is on | `ifconfig \| awk '/^[a-z]/{i=$1} /inet 100\./{sub(":","",i); print i}'` |
| `<INSTANCE_ID>` | the advert's instance name | the `name=` field on `mdns_reg_mac`'s first output line |
| `<MAC_A_BRIDGE_LL4>`, `<MAC_A_BRIDGE_LL6>` | Mac A's `169.254` and `fe80::` addresses on `bridge0` (B5 only) | `ifconfig bridge0`, with the cable in |
| `<MAC_B_BRIDGE_LL4>` | the same on Mac B (B5 only) | on Mac B: `ifconfig bridge0` |
| `<MAC_A_USB_LL4>` | Mac A's `169.254` address on a USB-C Ethernet adapter (A5 only) | `ifconfig` for the adapter's `en<N>` |

## Build the probes

On each machine that runs them, in a scratch copy so nothing is built inside
the checkout:

```sh
D=$(mktemp -d) && cp scripts/spike/p2-network/* "$D" && cd "$D"
cc -O1 -Wall -o netfp netfp.c
cc -O1 -o mdns_reg_mac mdns_reg_mac.c        # macOS only
rustc -O --edition 2021 llprobe.rs
```

Every later command runs from that directory. Two limits of the probes:
`llprobe dial` applies a zone to IPv6 only, because an IPv4 socket address has
no scope, and `echo_srv.py`'s third argument is `SO_BINDTODEVICE`, which
exists on Linux only. Never pass it on a Mac. Where a step needs a scoped
IPv4 dial on macOS, it uses this one-liner (`25` is `IP_BOUND_IF`):

```sh
dial4() { python3 -c 'import socket,sys; s=socket.socket(); s.setsockopt(socket.IPPROTO_IP,25,socket.if_nametoindex(sys.argv[3])); s.settimeout(3); s.connect((sys.argv[1],int(sys.argv[2]))); print("OK", s.getsockname())' "$@"; }
# dial4 <addr> <port> <ifname>
```

## A. iPhone (a TestFlight build of a throwaway Tauri iOS target)

1. **rustls with a Secure Enclave signer.** In the iOS target, create a
   permanent SE P-256 key (`kSecAttrTokenIDSecureEnclave`,
   `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`; it needs the
   `keychain-access-groups` entitlement, which a TestFlight build has). Wrap
   it as a `rustls::sign::SigningKey` whose `sign()` calls
   `SecKeyCreateSignature(…, .ecdsaSignatureMessageX962SHA256, …)`. On Mac A,
   run any rustls server that requires a client certificate and pins the
   device's SPKI. Record: the handshake completes, and the server sees the SE
   key's SPKI.
2. **ATS and Local Network from a Rust socket.** Add
   `NSLocalNetworkUsageDescription` first. On Mac A, one listener per address:

   ```sh
   python3 echo_srv.py <MAC_A_LAN_IP> 7800
   python3 echo_srv.py <MAC_A_TAILSCALE_IP> 7801
   ```

   From Rust on the iPhone (`std::net::TcpStream`), connect to
   `<MAC_A_LAN_IP>:7800` and to `<MAC_A_TAILSCALE_IP>:7801`, on a fresh
   install each time. Record: does the Local Network prompt appear, and for
   which of the two; after declining, the exact error (`errno`) on each;
   after allowing, success.
3. **Handshake cost.** Twenty full mTLS handshakes in a row to Mac A over
   Wi-Fi with the SE signer. Record the median and p95, and the SE `sign()`
   time alone (the spike's Mac: 4.7 ms per signature).
4. **`.userPresence` step-up.** A second SE key created with
   `SecAccessControlCreateWithFlags(…, .userPresence, …)`; call
   `SecKeyCreateSignature` from Rust. Record that Face ID appears, the
   latency, and the error on cancel.
5. **Link-local from a QR.** iPhone and Mac A on the same Wi-Fi. On Mac A:

   ```sh
   ./llprobe listen '<EN0_LL>%en0' 7401
   ```

   On the iPhone, dial `[<EN0_LL>]:7401` with scope id 0, then with
   `if_nametoindex("en0")`. Record both results. IPv4: connect the iPhone and
   Mac A with a USB-C Ethernet adapter each and one cable, no DHCP, so both
   self-assign `169.254`; on Mac A, `./llprobe listen <MAC_A_USB_LL4> 7402`;
   the iPhone dials it unscoped, then with `IP_BOUND_IF` (25) set to the
   adapter's index before `connect`. Record which work.
6. **Bonjour on iOS.** On Mac A, `./mdns_reg_mac en0 7443 120 release`.
   Browse `_worktrees._tcp` from the shim, through the `dns_sd` C API
   (`DNSServiceBrowse`) and separately through `NWBrowser`. Record: with
   `_worktrees._tcp` in `NSBonjourServices`, found; with it removed, the error
   code; when the Local Network prompt appears (first browse, or first
   connect); what a declined prompt returns to each API.

## B. Mac B, or Mac A with sudo where marked

1. **The host model across hosts.** On Mac A, a listener on the Tailscale
   address and the kernel's drop counter before and after:

   ```sh
   python3 echo_srv.py <MAC_A_TAILSCALE_IP> 7600
   netstat -s -p ip | grep 'no interface address match'
   ```

   On Mac B, with Tailscale OFF on B, route the address to Mac A's Wi-Fi and
   dial (undo with `sudo route -n delete -host <MAC_A_TAILSCALE_IP>`):

   ```sh
   sudo route -n add -host <MAC_A_TAILSCALE_IP> <MAC_A_LAN_IP>
   nc -vz -G 3 <MAC_A_TAILSCALE_IP> 7600
   ```

   Expected: a timeout, and Mac A's counter goes up (the hybrid model). Then
   on Mac A, **sudo**, `sudo sysctl net.inet.ip.check_interface=0`, dial
   again from B (expected: connected), and restore with
   `sudo sysctl net.inet.ip.check_interface=1`. Repeat for IPv6: on Mac A
   `python3 echo_srv.py <MAC_A_TAILSCALE_IP6> 7600`, `net.inet6.ip6` in place
   of `net.inet.ip` for the sysctl, and on Mac B:

   ```sh
   sudo route -n add -inet6 -host <MAC_A_TAILSCALE_IP6> <EN0_LL>%en0
   nc -6 -vz -G 3 <MAC_A_TAILSCALE_IP6> 7600
   ```

   Then the `IPV6_BOUND_IF` question: with `net.inet6.ip6.check_interface=0`
   on Mac A (restore it after), replace the IPv6 listener with one tied to the
   Tailscale interface (`125` is `IPV6_BOUND_IF`) and dial it from B again.
   Refused, or accepted?

   ```sh
   python3 -c 'import socket,sys; s=socket.socket(socket.AF_INET6); s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1); s.setsockopt(socket.IPPROTO_IPV6,125,socket.if_nametoindex(sys.argv[3])); s.bind((sys.argv[1],int(sys.argv[2]))); s.listen(); print("listening"); c,a=s.accept(); print("accepted",a)' <MAC_A_TAILSCALE_IP6> 7601 <TS_UTUN>
   ```

   (and dial `7601` from B). Remove B's IPv6 route with
   `sudo route -n delete -inet6 -host <MAC_A_TAILSCALE_IP6>`.
2. **mDNS on the wire.** On Mac B:

   ```sh
   sudo tcpdump -i en0 -n -vvv udp port 5353
   dns-sd -B _worktrees._tcp
   ```

   On Mac A, `./mdns_reg_mac en0 7443 10 release`, then
   `./mdns_reg_mac en0 7443 10 kill`. Record the announcement's records and
   TTLs, and the TTL-0 goodbye and its delay after the release and after the
   kill. On Mac A during the same run, `sudo tcpdump -i awdl0 -n udp port 5353`:
   nothing for `_worktrees` should appear there.
3. **Bonjour Sleep Proxy.** Needs an Apple TV or HomePod on the LAN, and
   System Settings → Battery (or Energy) → "Wake for network access" on. On
   Mac A, a listener and an advert, then sleep:

   ```sh
   python3 echo_srv.py <MAC_A_LAN_IP> 7443 &
   ./mdns_reg_mac en0 7443 600 release &
   pmset sleepnow
   ```

   On Mac B, for 5 minutes, `dns-sd -B _worktrees._tcp` and
   `dns-sd -L <INSTANCE_ID> _worktrees._tcp`, then
   `nc -vz -G 5 <MAC_A_LAN_IP> 7443`. Record: does the advert stay listed while
   A sleeps (proxied), and does the connect wake A?
4. **A listener whose address is removed, macOS.** Mac A, **sudo**, a scratch
   interface only:

   ```sh
   sudo ifconfig feth90 create
   sudo ifconfig feth90 inet 10.99.0.1/24 up
   python3 echo_srv.py 10.99.0.1 7700 &
   python3 fifo_cli.py 10.99.0.1 7700 /tmp/wtspike-cmd &
   sudo ifconfig feth90 inet 10.99.0.1 -alias     # remove the address
   netstat -an | grep 7700
   echo 'send after-del' > /tmp/wtspike-cmd       # on the held connection
   echo new > /tmp/wtspike-cmd                    # a fresh connect
   python3 bindtry.py 10.99.0.1 7701              # bind the removed address
   sudo ifconfig feth90 inet 10.99.0.1/24 alias   # put it back
   python3 bindtry.py 10.99.0.1 7700              # the old listener's port
   echo new > /tmp/wtspike-cmd
   kill %1 %2; sudo ifconfig feth90 destroy; rm -f /tmp/wtspike-cmd
   ```

   Record the same rows as the Linux run in §4.1 of the proposal: does the
   listener survive, is the queued data delivered after the re-add, does the
   second bind fail, and with which error.
5. **Link-local between two Macs, Thunderbolt Bridge.** Cable A to B, no
   other change. On both, `ifconfig bridge0` for the addresses. On Mac A,
   `route -n get <MAC_B_BRIDGE_LL4>`: which interface? Then on Mac A:

   ```sh
   ./llprobe listen <MAC_A_BRIDGE_LL4> 7402
   ./llprobe listen '<MAC_A_BRIDGE_LL6>%bridge0' 7401
   ```

   On Mac B, each unscoped and then scoped:

   ```sh
   ./llprobe dial <MAC_A_BRIDGE_LL4> 7402
   dial4 <MAC_A_BRIDGE_LL4> 7402 bridge0
   ./llprobe dial <MAC_A_BRIDGE_LL6> 7401
   ./llprobe dial <MAC_A_BRIDGE_LL6> 7401 bridge0
   ```

   Record which work.
6. **macOS Local Network permission and the Application Firewall, for
   serve.** Once slice 6 exists: launch the sandbox app
   (`app/scripts/sandbox.sh --app`, which starts serve), enable LAN mode,
   connect to the remote port from Mac B, and from serve dial a LAN address.
   Record: does a Local Network prompt appear, and naming which app (System
   Settings → Privacy & Security → Local Network lists it); does the advert
   appear without the permission; after a rebuild and relaunch, is it asked
   again. Separately, turn the Application Firewall on (System Settings →
   Network → Firewall), start serve's remote listener, and record whether
   "accept incoming connections" appears for the ad-hoc `worktrees` binary,
   and again after a rebuild. Put the firewall back as it was.

## C. A reboot

1. **`utun` numbering.** Before:

   ```sh
   ifconfig | grep -B3 'inet 100\.'
   printf 'list State:/Network/Service/.*/IPv4\n' | scutil
   ```

   Note which service UUID owns `<TS_UTUN>`
   (`printf 'show State:/Network/Service/<uuid>/IPv4\n' | scutil`). Reboot, log
   in, wait for Tailscale, and repeat. Repeat also after quitting and
   reopening Tailscale, and after a sleep and wake, without a reboot. Record
   the `utun` number each time, and whether the Tailscale service's UUID stays
   the same: a stable key that is not the name.

## D. CI runners (a `workflow_dispatch` job on a branch; no merge needed)

1. **macOS runner, the scratch interface (§4.6).** In one job, from the repo
   root:

   ```sh
   sudo ifconfig lo0 alias 198.51.100.7/32 \
     || { sudo ifconfig feth90 create && sudo ifconfig feth90 inet 198.51.100.7/32 up; }
   python3 scripts/spike/p2-network/bindtry.py 198.51.100.7 7500
   ifconfig feth90; sysctl net.inet.ip.check_interface
   ```

   Then B4's rows against `feth90`. Record whether sudo, `feth` and an alias
   are each allowed, and the outputs.
2. **Linux runner, without Docker.** From the repo root:

   ```sh
   cd scripts/spike/p2-network
   uname -r
   sudo ip link add scratch0 type dummy
   sudo ip addr add 198.51.100.7/32 dev scratch0 && sudo ip link set scratch0 up
   python3 bindtry.py 198.51.100.7 7500
   sudo ip tuntap add dev ptp0 mode tun
   sudo ip addr add 100.64.7.1/32 dev ptp0 && sudo ip link set ptp0 up
   sudo timeout 3 setpriv --reuid=65534 --regid=65534 --clear-groups \
     --inh-caps=-all --bounding-set=-all python3 echo_srv.py 100.64.7.1 7502 ptp0
   cc -O1 -Wall -o netfp netfp.c && ./netfp
   systemctl is-active avahi-daemon
   ```

   The `setpriv` line answers whether an unprivileged process may set
   `SO_BINDTODEVICE` on the runner's kernel: look for `SO_BINDTODEVICE ptp0
   ok`. `systemctl` says whether Avahi is there at all.

## Not yet probed

The opaque host name in §4.7 has no probe yet. §10's row says what to
measure; `mdns_reg_mac.c` and `avahi_reg.py` are where to start.
