//! The remote listener's socket (Phase 2 §4.1): one specific address, never
//! a wildcard, tied to its interface.
//!
//! `IP_BOUND_IF` / `IPV6_BOUND_IF` on macOS and `SO_BINDTOIFINDEX` on Linux,
//! through socket2's `bind_device_by_index_v4/_v6` (socket2 0.6 is already in
//! the tree under tokio, with the `all` feature these need). Measured
//! (network spike 2026-10-21): on Linux 6.12 it scopes a listening socket for
//! both families, a peer on another interface gets a RST, and an
//! unprivileged process may set it; on macOS 27 it scopes an IPv4 listener,
//! and `IPV6_BOUND_IF` did not filter same-host traffic (its cross-host
//! effect is unmeasured, §10 B1). Either way it pins the interface INDEX, so
//! the poll that sees the index change treats it as the address leaving.
//! The peer rule (`peer.rs`) is what holds where the kernel does not.
//!
//! An older Linux kernel refuses the option to an unprivileged process
//! (`EPERM`): the socket is bound without it, the caller is told, and the
//! peer rule still runs on every connection.

use std::net::{IpAddr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::num::NonZeroU32;

use socket2::{Domain, Protocol, Socket, Type};

/// Room for connections the accept loop has not taken yet. The caps are the
/// accept loop's (`listener.rs`); this only has to absorb a burst.
const BACKLOG: i32 = 128;

/// A bound, listening, non-blocking socket on `ip:port` (0: the OS picks),
/// tied to interface `index`. The `Option` is a warning to show when the
/// interface binding itself was refused.
pub fn listen(ip: IpAddr, port: u16, index: u32) -> std::io::Result<(std::net::TcpListener, Option<String>)> {
    let domain = if ip.is_ipv4() { Domain::IPV4 } else { Domain::IPV6 };
    let s = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
    if ip.is_ipv6() {
        s.set_only_v6(true)?;
    }
    // As std's `TcpListener::bind` does on unix: a resume or a move binds the
    // persisted port again while the connections it just closed sit in
    // TIME_WAIT, which would otherwise refuse it for a minute.
    s.set_reuse_address(true)?;
    let warning = match tie(&s, ip, index) {
        Ok(()) => None,
        Err(e) if e.raw_os_error() == Some(libc::EPERM) => Some(format!(
            "this kernel would not tie the listener to its interface ({e}); the peer rule still applies to every connection"
        )),
        Err(e) => return Err(e),
    };
    s.bind(&sockaddr(ip, port, index).into())?;
    s.listen(BACKLOG)?;
    s.set_nonblocking(true)?;
    Ok((s.into(), warning))
}

/// `ip:port`, with the interface as the scope of a link-local IPv6 address —
/// which means nothing without one, and is all a zone ever is here.
pub fn sockaddr(ip: IpAddr, port: u16, index: u32) -> SocketAddr {
    match ip {
        IpAddr::V4(a) => SocketAddr::V4(SocketAddrV4::new(a, port)),
        IpAddr::V6(a) => {
            let scope = if a.segments()[0] & 0xffc0 == 0xfe80 { index } else { 0 };
            SocketAddr::V6(SocketAddrV6::new(a, port, 0, scope))
        }
    }
}

/// The address a device dials and a link carries: the same, and NEVER a zone
/// (`SocketAddrV6`'s `Display` prints `%<scope>`, and a device's parser
/// refuses it: the host's interface number means nothing on the device).
pub fn public(ip: IpAddr, port: u16) -> SocketAddr {
    SocketAddr::new(ip, port)
}

fn tie(s: &Socket, ip: IpAddr, index: u32) -> std::io::Result<()> {
    let index = NonZeroU32::new(index).ok_or_else(|| std::io::Error::other("the interface has no index"))?;
    if ip.is_ipv4() {
        s.bind_device_by_index_v4(Some(index))
    } else {
        s.bind_device_by_index_v6(Some(index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A zone reaches the bind of a link-local IPv6 address and nothing else,
    /// and the address a device is given never carries one.
    #[test]
    fn only_a_link_local_v6_bind_is_scoped_and_no_link_ever_is() {
        let ll: IpAddr = "fe80::1".parse().unwrap();
        let ula: IpAddr = "fd00::1".parse().unwrap();
        let v4: IpAddr = "192.168.1.5".parse().unwrap();
        assert!(matches!(sockaddr(ll, 7000, 14), SocketAddr::V6(a) if a.scope_id() == 14));
        assert!(matches!(sockaddr(ula, 7000, 14), SocketAddr::V6(a) if a.scope_id() == 0));
        assert_eq!(sockaddr(v4, 7000, 14).to_string(), "192.168.1.5:7000");
        assert_eq!(public(ll, 7000).to_string(), "[fe80::1]:7000");
        assert!(!public(ll, 7000).to_string().contains('%'));
    }

    /// Loopback's own interface ties a listener as any other does: it binds,
    /// it is not a wildcard, and it answers on its address.
    #[test]
    fn a_tied_socket_listens_on_its_one_address() {
        let lo = worktrees_core::netif::interfaces().unwrap().into_iter().find(|i| i.loopback).expect("a loopback interface");
        let (l, warn) = listen("127.0.0.1".parse().unwrap(), 0, lo.index).unwrap();
        assert!(warn.is_none(), "{warn:?}");
        let at = l.local_addr().unwrap();
        assert_eq!(at.ip(), "127.0.0.1".parse::<IpAddr>().unwrap());
        assert!(std::net::TcpStream::connect_timeout(&at, std::time::Duration::from_secs(2)).is_ok());
    }
}
