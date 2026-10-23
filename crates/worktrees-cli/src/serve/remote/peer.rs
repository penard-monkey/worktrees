//! Who may reach the remote listener's TLS handshake at all (Phase 2 §4.1,
//! "a peer filter, as on loopback"): checked on the accepted socket's source
//! address, before a byte is read or written, and the same for the pairing
//! listener, whose window one stray connection would burn (§2.4).
//!
//! Three parts, and they do not have the same standing:
//!
//! 1. **Every peer passes the classifier** (`netclass::classify`). A router
//!    that forwards a public port to the private bind does not make serve
//!    reachable from the internet: the forwarded connection arrives from a
//!    public source and is dropped here. A requirement.
//! 2. **On an overlay bind** (a point-to-point interface), the peer is in the
//!    bound address's own allowlist row: `100.64.0.0/10` for a Tailscale IPv4
//!    bind, `fc00::/7` for its IPv6 one. Linux is a weak host (and macOS is,
//!    with forwarding on or for an overlay that is not a `utun`), so a LAN
//!    neighbour can send to the overlay address straight to the host's
//!    link-layer address, around the overlay's ACL (measured, network spike
//!    2026-10-21, Linux 6.12). A requirement.
//! 3. **On a LAN bind**, the peer is on-link: inside the bound address's own
//!    subnet (a link-local bind is on-link by definition). A routed private
//!    peer still has to get through mTLS, so this shrinks who can reach the
//!    handshake and protects no asset by itself: a DEFAULT, decided by (main)
//!    on the user's behalf (§11.1).
//!
//! The socket is also tied to its interface (`sock::listen`), which pins the
//! interface index; this rule is what holds where the kernel's does not.

use std::net::IpAddr;

use worktrees_core::netclass;
use worktrees_core::netif::network;

/// What the rule needs to know about the bind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule {
    pub ip: IpAddr,
    /// The bound address's prefix on its interface: its subnet, for part 3.
    pub prefix: u8,
    /// Overlay mode (part 2) rather than LAN mode (part 3).
    pub point_to_point: bool,
}

impl Rule {
    /// May a connection from `peer` reach the handshake?
    pub fn admits(&self, peer: IpAddr) -> bool {
        // Part 1. A mapped or otherwise embedded IPv4 address is refused
        // here, never unwrapped: each address has one spelling.
        let Some(theirs) = netclass::row_of(peer) else { return false };
        if peer.is_ipv4() != self.ip.is_ipv4() {
            return false;
        }
        if self.point_to_point {
            // Part 2: the overlay's own row, and no other private one.
            netclass::row_of(self.ip) == Some(theirs)
        } else {
            // Part 3: on-link.
            network(peer, self.prefix) == network(self.ip, self.prefix)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn lan(addr: &str, prefix: u8) -> Rule {
        Rule { ip: ip(addr), prefix, point_to_point: false }
    }

    fn overlay(addr: &str, prefix: u8) -> Rule {
        Rule { ip: ip(addr), prefix, point_to_point: true }
    }

    /// Part 3: a LAN bind admits its own subnet and nothing routed in, even
    /// from another private range.
    #[test]
    fn a_lan_bind_admits_on_link_peers_only() {
        let r = lan("192.168.1.5", 24);
        for ok in ["192.168.1.1", "192.168.1.254", "192.168.1.5"] {
            assert!(r.admits(ip(ok)), "{ok}");
        }
        for no in ["192.168.2.7", "10.0.0.7", "172.16.0.1", "100.64.0.9", "169.254.1.1"] {
            assert!(!r.admits(ip(no)), "{no} is private but not on-link");
        }
        let r = lan("10.20.0.5", 16);
        assert!(r.admits(ip("10.20.255.1")));
        assert!(!r.admits(ip("10.21.0.1")));
    }

    /// Part 1, on every kind of bind: a public, loopback, mapped or reserved
    /// source never reaches TLS — a port forward's connection included.
    #[test]
    fn a_source_outside_the_allowlist_is_refused_on_every_bind() {
        let binds = [lan("192.168.1.5", 24), lan("10.0.0.5", 8), overlay("100.101.7.3", 32), lan("fd00::5", 64), overlay("fd7a:115c:a1e0::5", 128)];
        for r in binds {
            for no in ["8.8.8.8", "127.0.0.1", "0.0.0.0", "255.255.255.255", "198.51.100.7", "::1", "::", "2001:db8::1", "2606:4700::1111", "::ffff:192.168.1.7", "::ffff:10.0.0.7"] {
                assert!(!r.admits(ip(no)), "{no} reached a bind on {}", r.ip);
            }
        }
    }

    /// Part 2: an overlay bind admits its own row and NOTHING else private —
    /// the LAN neighbour on the weak host is refused, however on-link it is.
    #[test]
    fn an_overlay_bind_admits_its_own_row_only() {
        let r = overlay("100.101.7.3", 32);
        assert!(r.admits(ip("100.64.0.1")));
        assert!(r.admits(ip("100.127.255.254")));
        for no in ["192.168.1.7", "10.0.0.7", "172.16.0.1", "169.254.1.1", "100.128.0.1", "100.63.255.255"] {
            assert!(!r.admits(ip(no)), "{no}");
        }
        let r6 = overlay("fd7a:115c:a1e0::5", 128);
        assert!(r6.admits(ip("fd7a:115c:a1e0::9")));
        assert!(r6.admits(ip("fc00::1")), "the whole ULA row, as §4.1 says");
        assert!(!r6.admits(ip("fe80::1")), "link-local is another row");
        assert!(!r6.admits(ip("100.64.0.1")), "the other family");
    }

    /// A link-local LAN bind: on-link by its prefix, and the IPv4 and IPv6
    /// halves stay apart.
    #[test]
    fn a_link_local_bind_is_on_link_by_its_prefix() {
        let r = lan("169.254.10.20", 16);
        assert!(r.admits(ip("169.254.200.1")));
        assert!(!r.admits(ip("192.168.1.7")));
        let r6 = lan("fe80::1", 64);
        assert!(r6.admits(ip("fe80::abcd")));
        assert!(!r6.admits(ip("fd00::1")));
        assert!(!r6.admits(ip("169.254.1.1")));
    }
}
