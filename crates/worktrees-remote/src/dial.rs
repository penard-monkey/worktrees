//! The TCP connect under every dial: the pairing port and the remote listener.
//!
//! The address has already passed `netclass::parse_dial` (a link, an address
//! code, or a kept record, re-read). A link-local one means nothing without an
//! interface, and §4.1 has the device pick its OWN, scoped, at most four
//! (`netclass::linklocal_candidates`). This client does not do that yet, so it
//! refuses such an address with words that say so rather than dialling it
//! unscoped, which follows the routing table to whichever link it likes
//! (measured, network spike: the wrong one, on Linux, with two links).

use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::TcpStream;
use worktrees_core::netclass::{self, Class};

/// Connect to `addr` within `wait`. Nagle is off: frames are small and a
/// keystroke should not wait for the next one.
pub async fn tcp(addr: SocketAddr, wait: Duration) -> std::io::Result<TcpStream> {
    if netclass::classify(addr.ip()).map(|c| c == Class::LinkLocal).unwrap_or(false) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            format!(
                "{} is link-local, and this client cannot yet choose the interface such an address needs; \
                 use the host's LAN or overlay address",
                addr.ip()
            ),
        ));
    }
    let s = tokio::time::timeout(wait, TcpStream::connect(addr))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, format!("no answer from {addr} within {}s", wait.as_secs())))??;
    let _ = s.set_nodelay(true);
    Ok(s)
}
