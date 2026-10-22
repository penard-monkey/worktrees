//! The poll's readers: the interfaces (`worktrees_core::netif`) plus each
//! interface's OWN default router and that router's link-layer address,
//! without privilege on either platform (§4.1, §12.4 T6).
//!
//! - **Linux: netlink.** An `RTM_GETROUTE` dump covers every table (an exit
//!   node or a VPN puts its default in another one; the interface's own is
//!   still there), and `RTM_GETNEIGH` gives the router's link-layer address.
//!   Both readable as uid 65534 with no capabilities (measured, network spike
//!   2026-10-21, Linux 6.12). Parsed here from the raw bytes — no netlink
//!   crate — so the parser runs in every platform's tests.
//! - **macOS: `scutil`.** The kernel hides the ARP table from a non-root
//!   process (measured: no entry for the gateway even right after a ping), but
//!   SystemConfiguration publishes each service's `Router`,
//!   `ARPResolvedHardwareAddress` and `NetworkSignature` under
//!   `State:/Network/Service/<id>/IPv4`, readable by anyone. Read through
//!   `/usr/sbin/scutil` rather than the `system-configuration` crate: the
//!   network-facing binary gains no crate, FFI or `unsafe` for one dictionary
//!   (cargo-deny's graph, §4.2, stays as it is), the absolute path is immune
//!   to `PATH`, and the parser is plain text, pinned to fixtures captured on
//!   macOS 27. The cost is two short-lived processes per poll while a LAN
//!   bind is up (~3 ms of work each, measured; up to ~120 ms of
//!   `run_deadline`'s wait granularity, on the poll thread), the same trade
//!   as git and tmux. `scutil` has shipped with that output format since Mac
//!   OS X 10.1; a format it no longer matches reads as "no router known",
//!   which never counts as a change.

use std::net::IpAddr;

use super::{IfRouter, LinkAddr, Router, Snapshot};
use worktrees_core::netif::Iface;

/// Everything one poll needs.
pub fn snapshot() -> std::io::Result<Snapshot> {
    let ifaces = worktrees_core::netif::interfaces()?;
    let routers = routers(&ifaces);
    Ok(Snapshot { ifaces, routers })
}

#[cfg(target_os = "linux")]
fn routers(ifaces: &[Iface]) -> Vec<IfRouter> {
    netlink::read(ifaces).unwrap_or_default()
}

#[cfg(target_os = "macos")]
fn routers(_ifaces: &[Iface]) -> Vec<IfRouter> {
    scutil::read().unwrap_or_default()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn routers(_ifaces: &[Iface]) -> Vec<IfRouter> {
    Vec::new()
}

/// rtnetlink, from `linux/netlink.h`, `linux/rtnetlink.h` and
/// `linux/neighbour.h`. The parser is pure and compiled everywhere; only
/// [`netlink::read`] touches a socket.
pub mod netlink {
    use super::*;

    pub const RTM_NEWROUTE: u16 = 24;
    pub const RTM_GETROUTE: u16 = 26;
    pub const RTM_NEWNEIGH: u16 = 28;
    pub const RTM_GETNEIGH: u16 = 30;
    pub const NLMSG_ERROR: u16 = 2;
    pub const NLMSG_DONE: u16 = 3;
    pub const NLM_F_REQUEST: u16 = 0x1;
    pub const NLM_F_DUMP: u16 = 0x300;
    pub const RTA_OIF: u16 = 4;
    pub const RTA_GATEWAY: u16 = 5;
    pub const RTA_TABLE: u16 = 15;
    pub const RTN_UNICAST: u8 = 1;
    pub const RT_TABLE_MAIN: u32 = 254;
    pub const NDA_DST: u16 = 1;
    pub const NDA_LLADDR: u16 = 2;
    /// Neighbour states with no usable link-layer address.
    pub const NUD_INCOMPLETE: u16 = 0x01;
    pub const NUD_FAILED: u16 = 0x20;
    pub const AF_INET: u8 = 2;
    pub const AF_INET6: u8 = 10;
    /// `struct nlmsghdr`, `struct rtmsg`, `struct ndmsg`, `struct rtattr`.
    pub const NLMSG_HDRLEN: usize = 16;
    pub const RTMSG_LEN: usize = 12;
    pub const NDMSG_LEN: usize = 12;

    /// A default unicast route: no destination prefix, an output interface,
    /// a gateway when it has one.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Route {
        pub table: u32,
        pub oif: u32,
        pub gateway: Option<IpAddr>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Neigh {
        pub ifindex: u32,
        pub dst: IpAddr,
        pub lladdr: Option<LinkAddr>,
        pub state: u16,
    }

    /// One datagram's worth of messages, as `(type, payload)`, and whether it
    /// held `NLMSG_DONE`. An `NLMSG_ERROR` is an error; a truncated message
    /// ends the walk.
    pub fn messages(buf: &[u8]) -> std::io::Result<(Vec<(u16, &[u8])>, bool)> {
        let mut out = Vec::new();
        let mut at = 0;
        while at + NLMSG_HDRLEN <= buf.len() {
            let len = u32::from_ne_bytes(buf[at..at + 4].try_into().unwrap()) as usize;
            let ty = u16::from_ne_bytes(buf[at + 4..at + 6].try_into().unwrap());
            if len < NLMSG_HDRLEN || at + len > buf.len() {
                break;
            }
            match ty {
                NLMSG_DONE => return Ok((out, true)),
                NLMSG_ERROR => {
                    let code = buf.get(at + 16..at + 20).map(|b| i32::from_ne_bytes(b.try_into().unwrap())).unwrap_or(0);
                    if code != 0 {
                        return Err(std::io::Error::from_raw_os_error(-code));
                    }
                }
                _ => out.push((ty, &buf[at + NLMSG_HDRLEN..at + len])),
            }
            at += (len + 3) & !3;
        }
        Ok((out, false))
    }

    /// `rtattr`s after a fixed header: `(type, payload)`.
    fn attrs(body: &[u8]) -> Vec<(u16, &[u8])> {
        let mut out = Vec::new();
        let mut at = 0;
        while at + 4 <= body.len() {
            let len = u16::from_ne_bytes(body[at..at + 2].try_into().unwrap()) as usize;
            let ty = u16::from_ne_bytes(body[at + 2..at + 4].try_into().unwrap());
            if len < 4 || at + len > body.len() {
                break;
            }
            out.push((ty & 0x3fff, &body[at + 4..at + len]));
            at += (len + 3) & !3;
        }
        out
    }

    fn ip(family: u8, b: &[u8]) -> Option<IpAddr> {
        match (family, b.len()) {
            (AF_INET, 4) => Some(IpAddr::from(<[u8; 4]>::try_from(b).ok()?)),
            (AF_INET6, 16) => Some(IpAddr::from(<[u8; 16]>::try_from(b).ok()?)),
            _ => None,
        }
    }

    /// The default unicast routes in an `RTM_NEWROUTE` payload.
    pub fn route(payload: &[u8]) -> Option<Route> {
        if payload.len() < RTMSG_LEN {
            return None;
        }
        let (family, dst_len, table, ty) = (payload[0], payload[1], payload[4], payload[7]);
        if dst_len != 0 || ty != RTN_UNICAST {
            return None;
        }
        let mut r = Route { table: u32::from(table), oif: 0, gateway: None };
        for (t, v) in attrs(&payload[RTMSG_LEN..]) {
            match t {
                RTA_OIF if v.len() == 4 => r.oif = u32::from_ne_bytes(v.try_into().unwrap()),
                RTA_TABLE if v.len() == 4 => r.table = u32::from_ne_bytes(v.try_into().unwrap()),
                RTA_GATEWAY => r.gateway = ip(family, v),
                _ => {}
            }
        }
        (r.oif != 0).then_some(r)
    }

    /// An `RTM_NEWNEIGH` payload.
    pub fn neigh(payload: &[u8]) -> Option<Neigh> {
        if payload.len() < NDMSG_LEN {
            return None;
        }
        let family = payload[0];
        let ifindex = i32::from_ne_bytes(payload[4..8].try_into().unwrap());
        let state = u16::from_ne_bytes(payload[8..10].try_into().unwrap());
        let (mut dst, mut lladdr) = (None, None);
        for (t, v) in attrs(&payload[NDMSG_LEN..]) {
            match t {
                NDA_DST => dst = ip(family, v),
                NDA_LLADDR if !v.is_empty() => lladdr = Some(LinkAddr(v.to_vec())),
                _ => {}
            }
        }
        Some(Neigh { ifindex: u32::try_from(ifindex).ok()?, dst: dst?, lladdr, state })
    }

    /// Each interface's own router per family: a default route out of it,
    /// the main table first, then any other in the order the kernel listed
    /// them; with its neighbour entry's link-layer address when the entry is
    /// present and resolved.
    pub fn select(ifaces: &[Iface], routes: &[Route], neighs: &[Neigh]) -> Vec<IfRouter> {
        let mut out = Vec::new();
        for iface in ifaces {
            for v4 in [true, false] {
                let mine = routes.iter().filter(|r| r.oif == iface.index && r.gateway.is_some_and(|g| g.is_ipv4() == v4));
                let Some(r) = mine.clone().find(|r| r.table == RT_TABLE_MAIN).or_else(|| mine.clone().next()) else {
                    continue;
                };
                let gw = r.gateway.unwrap();
                let lladdr = neighs
                    .iter()
                    .find(|n| n.ifindex == iface.index && n.dst == gw && n.state & (NUD_INCOMPLETE | NUD_FAILED) == 0 && n.state != 0)
                    .and_then(|n| n.lladdr.clone());
                out.push(IfRouter { iface: iface.name.clone(), router: Router { ip: gw, lladdr } });
            }
        }
        out
    }

    /// The request a dump starts with: `nlmsghdr` then a zeroed family
    /// header with only its family set (rtmsg and ndmsg both lead with it).
    pub fn request(ty: u16, family: u8, seq: u32) -> Vec<u8> {
        let len = NLMSG_HDRLEN + RTMSG_LEN;
        let mut b = Vec::with_capacity(len);
        b.extend_from_slice(&(len as u32).to_ne_bytes());
        b.extend_from_slice(&ty.to_ne_bytes());
        b.extend_from_slice(&(NLM_F_REQUEST | NLM_F_DUMP).to_ne_bytes());
        b.extend_from_slice(&seq.to_ne_bytes());
        b.extend_from_slice(&0u32.to_ne_bytes());
        b.push(family);
        b.resize(len, 0);
        b
    }

    #[cfg(target_os = "linux")]
    pub fn read(ifaces: &[Iface]) -> std::io::Result<Vec<IfRouter>> {
        let mut routes = Vec::new();
        let mut neighs = Vec::new();
        for (n, family) in [AF_INET, AF_INET6].into_iter().enumerate() {
            for buf in dump(request(RTM_GETROUTE, family, 2 * n as u32 + 1))? {
                for (ty, p) in messages(&buf)?.0 {
                    if ty == RTM_NEWROUTE {
                        routes.extend(route(p));
                    }
                }
            }
            for buf in dump(request(RTM_GETNEIGH, family, 2 * n as u32 + 2))? {
                for (ty, p) in messages(&buf)?.0 {
                    if ty == RTM_NEWNEIGH {
                        neighs.extend(neigh(p));
                    }
                }
            }
        }
        Ok(select(ifaces, &routes, &neighs))
    }

    /// Send one dump request and collect its datagrams up to `NLMSG_DONE`,
    /// with a one-second receive timeout so a kernel that never answers
    /// cannot hold the poll.
    #[cfg(target_os = "linux")]
    fn dump(req: Vec<u8>) -> std::io::Result<Vec<Vec<u8>>> {
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
        let err = std::io::Error::last_os_error;
        // SAFETY: a fresh socket, owned (and closed) by `fd`.
        let raw = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_RAW | libc::SOCK_CLOEXEC, libc::NETLINK_ROUTE) };
        if raw < 0 {
            return Err(err());
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let tv = libc::timeval { tv_sec: 1, tv_usec: 0 };
        // SAFETY: a timeval of the size given.
        let set = unsafe {
            libc::setsockopt(
                fd.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_RCVTIMEO,
                &tv as *const libc::timeval as *const libc::c_void,
                std::mem::size_of::<libc::timeval>() as libc::socklen_t,
            )
        };
        if set < 0 {
            return Err(err());
        }
        // SAFETY: a zeroed sockaddr_nl is the kernel's address (pid 0).
        let mut to: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        to.nl_family = libc::AF_NETLINK as libc::sa_family_t;
        let sent = unsafe {
            libc::sendto(
                fd.as_raw_fd(),
                req.as_ptr() as *const libc::c_void,
                req.len(),
                0,
                &to as *const libc::sockaddr_nl as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
            )
        };
        if sent < 0 {
            return Err(err());
        }
        let mut out = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        for _ in 0..4096 {
            // SAFETY: recv into a buffer of the length given.
            let n = unsafe { libc::recv(fd.as_raw_fd(), buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0) };
            if n < 0 {
                return Err(err());
            }
            let chunk = buf[..n as usize].to_vec();
            let done = messages(&chunk)?.1;
            out.push(chunk);
            if done || n == 0 {
                return Ok(out);
            }
        }
        Err(std::io::Error::other("netlink dump did not end"))
    }
}

/// SystemConfiguration through `/usr/sbin/scutil`. The parsers are pure and
/// compiled everywhere; only [`scutil::read`] runs anything.
pub mod scutil {
    use super::*;

    pub const SCUTIL: &str = "/usr/sbin/scutil";
    pub const LIST: &str = "list State:/Network/Service/[^/]+/IPv[46]\n";

    /// The service keys a `list` printed (`  subKey [0] = State:/…/IPv4`),
    /// each checked to be exactly `State:/Network/Service/<id>/IPv4|6` with
    /// an id of letters, digits and `-` — they go back into `scutil`'s stdin,
    /// one per line.
    pub fn keys(list: &str) -> Vec<String> {
        list.lines()
            .filter_map(|l| l.split_once(" = ").map(|(_, k)| k.trim()))
            .filter(|k| {
                let Some(rest) = k.strip_prefix("State:/Network/Service/") else { return false };
                let Some((id, fam)) = rest.split_once('/') else { return false };
                !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') && (fam == "IPv4" || fam == "IPv6")
            })
            .map(str::to_string)
            .collect()
    }

    /// The `show` commands for those keys, one process's stdin.
    pub fn shows(keys: &[String]) -> String {
        keys.iter().map(|k| format!("show {k}\n")).collect()
    }

    /// The top-level scalars of each `<dictionary> { … }` a run of `show`s
    /// printed, nested arrays and dictionaries skipped; a "No such key" is no
    /// dictionary at all.
    pub fn dicts(text: &str) -> Vec<Vec<(String, String)>> {
        let mut out = Vec::new();
        let mut cur: Option<Vec<(String, String)>> = None;
        let mut depth = 0usize;
        for line in text.lines() {
            let t = line.trim();
            if depth == 0 {
                if t == "<dictionary> {" {
                    cur = Some(Vec::new());
                    depth = 1;
                }
                continue;
            }
            if t == "}" {
                depth -= 1;
                if depth == 0 {
                    out.extend(cur.take());
                }
                continue;
            }
            if t.ends_with('{') {
                depth += 1;
                continue;
            }
            if depth == 1 {
                if let (Some(d), Some((k, v))) = (cur.as_mut(), t.split_once(" : ")) {
                    d.push((k.trim().to_string(), v.trim().to_string()));
                }
            }
        }
        out
    }

    /// One service dictionary's router: `Router` (a zone dropped), and its
    /// link-layer address from `ARPResolvedHardwareAddress` when
    /// `ARPResolvedIPAddress` names that router, else from the OS's own
    /// `NetworkSignature` (`IPv4.Router=…;IPv4.RouterHardwareAddress=…`) when
    /// it names it.
    pub fn router(d: &[(String, String)]) -> Option<IfRouter> {
        let get = |k: &str| d.iter().find(|(x, _)| x == k).map(|(_, v)| v.as_str());
        let iface = get("InterfaceName")?;
        let ip: IpAddr = get("Router")?.split('%').next()?.parse().ok()?;
        let arp = (get("ARPResolvedIPAddress").and_then(|a| a.parse::<IpAddr>().ok()) == Some(ip))
            .then(|| get("ARPResolvedHardwareAddress").and_then(LinkAddr::parse))
            .flatten();
        let sig = || {
            let s = get("NetworkSignature")?;
            let field = |k: &str| s.split(';').find_map(|kv| kv.strip_prefix(k)?.strip_prefix('='));
            (field("IPv4.Router")?.parse::<IpAddr>().ok()? == ip).then(|| field("IPv4.RouterHardwareAddress").and_then(LinkAddr::parse))?
        };
        let lladdr = arp.or_else(sig);
        Some(IfRouter { iface: iface.to_string(), router: Router { ip, lladdr } })
    }

    pub fn routers(shown: &str) -> Vec<IfRouter> {
        let mut out: Vec<IfRouter> = Vec::new();
        for r in dicts(shown).iter().filter_map(|d| router(d)) {
            let dup = out.iter().any(|o| o.iface == r.iface && o.router.ip.is_ipv4() == r.router.ip.is_ipv4());
            if !dup {
                out.push(r);
            }
        }
        out
    }

    fn run(input: String) -> std::io::Result<String> {
        let cmd = worktrees_core::childenv::command(SCUTIL);
        let o = worktrees_core::proc::run_deadline_stdin(cmd, input.into_bytes(), 2)?;
        if !o.status.success() {
            return Err(std::io::Error::other(format!("scutil exited {}", o.status)));
        }
        Ok(String::from_utf8_lossy(&o.stdout).into_owned())
    }

    pub fn read() -> std::io::Result<Vec<IfRouter>> {
        let keys = keys(&run(LIST.to_string())?);
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        Ok(routers(&run(shows(&keys))?))
    }
}

#[cfg(test)]
mod tests {
    use super::netlink::*;
    use super::*;
    use worktrees_core::netif::IfAddr;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn mac(s: &str) -> LinkAddr {
        LinkAddr::parse(s).unwrap()
    }

    fn iface(name: &str, index: u32) -> Iface {
        Iface { name: name.into(), index, up: true, running: true, loopback: false, point_to_point: false, addrs: Vec::<IfAddr>::new() }
    }

    /// A netlink message as the kernel lays it out: header, fixed part,
    /// 4-aligned attributes.
    fn nlmsg(ty: u16, fixed: &[u8], attrs: &[(u16, Vec<u8>)]) -> Vec<u8> {
        let mut body = fixed.to_vec();
        for (t, v) in attrs {
            body.extend_from_slice(&((4 + v.len()) as u16).to_ne_bytes());
            body.extend_from_slice(&t.to_ne_bytes());
            body.extend_from_slice(v);
            while body.len() % 4 != 0 {
                body.push(0);
            }
        }
        let mut m = Vec::new();
        m.extend_from_slice(&((NLMSG_HDRLEN + body.len()) as u32).to_ne_bytes());
        m.extend_from_slice(&ty.to_ne_bytes());
        m.extend_from_slice(&0x2u16.to_ne_bytes());
        m.extend_from_slice(&[0; 8]);
        m.extend(body);
        m
    }

    fn rtmsg(family: u8, dst_len: u8, table: u8, ty: u8) -> Vec<u8> {
        vec![family, dst_len, 0, 0, table, 3, 0, ty, 0, 0, 0, 0]
    }

    fn ndmsg(family: u8, ifindex: i32, state: u16) -> Vec<u8> {
        let mut v = vec![family, 0, 0, 0];
        v.extend_from_slice(&ifindex.to_ne_bytes());
        v.extend_from_slice(&state.to_ne_bytes());
        v.extend_from_slice(&[0, 1]);
        v
    }

    fn octets(s: &str) -> Vec<u8> {
        match ip(s) {
            IpAddr::V4(v) => v.octets().to_vec(),
            IpAddr::V6(v) => v.octets().to_vec(),
        }
    }

    fn done() -> Vec<u8> {
        nlmsg(NLMSG_DONE, &[0, 0, 0, 0], &[])
    }

    /// The spike's Linux read (`linux-fingerprint.sh`): eth0's default in the
    /// main table, and an exit node's default via the overlay in table 52,
    /// which does NOT take eth0's router away.
    #[test]
    fn netlink_routes_give_each_interface_its_own_router_main_table_first() {
        let mut dg = Vec::new();
        dg.extend(nlmsg(RTM_NEWROUTE, &rtmsg(AF_INET, 0, 252, RTN_UNICAST), &[(RTA_TABLE, 52u32.to_ne_bytes().to_vec()), (RTA_OIF, 12u32.to_ne_bytes().to_vec())]));
        dg.extend(nlmsg(RTM_NEWROUTE, &rtmsg(AF_INET, 0, 100, RTN_UNICAST), &[(RTA_TABLE, 100u32.to_ne_bytes().to_vec()), (RTA_GATEWAY, octets("10.77.0.254")), (RTA_OIF, 11u32.to_ne_bytes().to_vec())]));
        dg.extend(nlmsg(RTM_NEWROUTE, &rtmsg(AF_INET, 0, 254, RTN_UNICAST), &[(RTA_TABLE, 254u32.to_ne_bytes().to_vec()), (RTA_GATEWAY, octets("10.77.0.1")), (RTA_OIF, 11u32.to_ne_bytes().to_vec())]));
        dg.extend(nlmsg(RTM_NEWROUTE, &rtmsg(AF_INET, 24, 254, RTN_UNICAST), &[(RTA_GATEWAY, octets("10.77.0.9")), (RTA_OIF, 11u32.to_ne_bytes().to_vec())]));
        dg.extend(nlmsg(RTM_NEWROUTE, &rtmsg(AF_INET, 0, 255, 2), &[(RTA_OIF, 11u32.to_ne_bytes().to_vec())]));
        dg.extend(nlmsg(RTM_NEWROUTE, &rtmsg(AF_INET6, 0, 254, RTN_UNICAST), &[(RTA_GATEWAY, octets("fe80::1")), (RTA_OIF, 11u32.to_ne_bytes().to_vec())]));
        dg.extend(done());
        let (msgs, end) = messages(&dg).unwrap();
        assert!(end);
        let routes: Vec<Route> = msgs.iter().filter(|(t, _)| *t == RTM_NEWROUTE).filter_map(|(_, p)| route(p)).collect();
        assert_eq!(routes.len(), 4, "{routes:?}");
        assert_eq!(routes[0], Route { table: 52, oif: 12, gateway: None });

        let mut nd = Vec::new();
        nd.extend(nlmsg(RTM_NEWNEIGH, &ndmsg(AF_INET, 11, 0x02), &[(NDA_DST, octets("10.77.0.1")), (NDA_LLADDR, vec![0x62, 0xb7, 0x54, 0x57, 0x5c, 0x2b])]));
        nd.extend(nlmsg(RTM_NEWNEIGH, &ndmsg(AF_INET, 12, 0x02), &[(NDA_DST, octets("10.77.0.1")), (NDA_LLADDR, vec![1, 2, 3, 4, 5, 6])]));
        nd.extend(nlmsg(RTM_NEWNEIGH, &ndmsg(AF_INET6, 11, NUD_INCOMPLETE), &[(NDA_DST, octets("fe80::1"))]));
        nd.extend(done());
        let neighs: Vec<Neigh> = messages(&nd).unwrap().0.iter().filter_map(|(_, p)| neigh(p)).collect();
        assert_eq!(neighs.len(), 3);

        let got = select(&[iface("eth0", 11), iface("ts0", 12)], &routes, &neighs);
        assert_eq!(
            got,
            vec![
                IfRouter { iface: "eth0".into(), router: Router { ip: ip("10.77.0.1"), lladdr: Some(mac("62:b7:54:57:5c:2b")) } },
                IfRouter { iface: "eth0".into(), router: Router { ip: ip("fe80::1"), lladdr: None } },
            ],
            "main table first; the neighbour on ANOTHER interface is not eth0's; an INCOMPLETE entry is not known yet; ts0's gateway-less default is no router"
        );
    }

    /// Before the first packet to the gateway, and after a flush, there is
    /// no neighbour entry at all: not known yet, not "none".
    #[test]
    fn netlink_without_a_neighbour_entry_reports_the_router_with_no_lladdr() {
        let routes = vec![Route { table: 254, oif: 11, gateway: Some(ip("10.77.0.1")) }];
        let stale = Neigh { ifindex: 11, dst: ip("10.77.0.1"), lladdr: Some(mac("62:b7:54:57:5c:2b")), state: 0x04 };
        let failed = Neigh { state: NUD_FAILED, ..stale.clone() };
        assert_eq!(select(&[iface("eth0", 11)], &routes, &[])[0].router.lladdr, None);
        assert_eq!(select(&[iface("eth0", 11)], &routes, &[failed])[0].router.lladdr, None);
        assert_eq!(select(&[iface("eth0", 11)], &routes, &[stale])[0].router.lladdr, Some(mac("62:b7:54:57:5c:2b")), "STALE still names the router");
    }

    #[test]
    fn netlink_framing_errors_and_truncation() {
        let mut err = nlmsg(NLMSG_ERROR, &(-1i32).to_ne_bytes(), &[]);
        err.extend_from_slice(&[0; 16]);
        assert_eq!(messages(&err).unwrap_err().raw_os_error(), Some(1));
        let ack = nlmsg(NLMSG_ERROR, &0i32.to_ne_bytes(), &[]);
        assert!(messages(&ack).unwrap().0.is_empty());
        let mut cut = nlmsg(RTM_NEWROUTE, &rtmsg(AF_INET, 0, 254, RTN_UNICAST), &[(RTA_OIF, 11u32.to_ne_bytes().to_vec())]);
        cut.truncate(cut.len() - 2);
        assert_eq!(messages(&cut).unwrap(), (vec![], false));
        assert_eq!(route(&[2, 0, 0]), None);
        let req = request(RTM_GETROUTE, AF_INET6, 7);
        assert_eq!(req.len(), 28);
        assert_eq!(u16::from_ne_bytes([req[6], req[7]]), NLM_F_REQUEST | NLM_F_DUMP);
        assert_eq!(req[16], AF_INET6);
    }

    /// `scutil` on macOS 27 (the spike's Mac, 2026-10-22; hardware addresses
    /// altered): en0's IPv4 and IPv6 services, Tailscale's on utun6, a system
    /// utun's IPv6, and a key that vanished between `list` and `show`.
    const SHOWN: &str = "\
<dictionary> {
  ARPResolvedHardwareAddress : d8:44:89:a:b:c
  ARPResolvedIPAddress : 192.168.0.1
  AdditionalRoutes : <array> {
    0 : <dictionary> {
      DestinationAddress : 192.168.0.40
      SubnetMask : 255.255.255.255
    }
    1 : <dictionary> {
      DestinationAddress : 169.254.0.0
      SubnetMask : 255.255.0.0
    }
  }
  Addresses : <array> {
    0 : 192.168.0.40
  }
  ConfirmedInterfaceName : en0
  InterfaceName : en0
  NetworkSignature : IPv4.Router=192.168.0.1;IPv4.RouterHardwareAddress=d8:44:89:a:b:c
  NetworkSignatureHash : <data> 0xef3402be0000
  Router : 192.168.0.1
  SubnetMasks : <array> {
    0 : 255.255.255.0
  }
}
<dictionary> {
  Addresses : <array> {
    0 : fd95:7329:9f61:1e61:1c04:d030:2ebd:e7d5
  }
  ConfirmedInterfaceName : en0
  InterfaceName : en0
  PrefixLength : <array> {
    0 : 64
  }
}
<dictionary> {
  AdditionalRoutes : <array> {
    0 : <dictionary> {
      DestinationAddress : 100.88.40.69
      SubnetMask : 255.255.255.255
    }
  }
  Addresses : <array> {
    0 : 100.96.164.69
  }
  InterfaceName : utun6
  Router : 100.96.164.69
  ServerAddress : 127.0.0.1
}
<dictionary> {
  Addresses : <array> {
    0 : fe80::452:4e36:d245:fec1%utun2
  }
  InterfaceName : utun2
  PrefixLength : <array> {
    0 : 64
  }
  Router : fe80::
}
  No such key
";

    #[test]
    fn scutil_service_dictionaries_give_each_interface_its_router() {
        let got = scutil::routers(SHOWN);
        assert_eq!(
            got,
            vec![
                IfRouter { iface: "en0".into(), router: Router { ip: ip("192.168.0.1"), lladdr: Some(mac("d8:44:89:0a:0b:0c")) } },
                IfRouter { iface: "utun6".into(), router: Router { ip: ip("100.96.164.69"), lladdr: None } },
                IfRouter { iface: "utun2".into(), router: Router { ip: ip("fe80::"), lladdr: None } },
            ]
        );
        assert_eq!(scutil::dicts(SHOWN).len(), 4, "nested dictionaries are not services");
    }

    /// The link-layer address must be the ROUTER's: an ARP entry for another
    /// address is not it, and the OS's own signature is the fallback.
    #[test]
    fn scutil_takes_the_lladdr_only_when_it_names_the_router() {
        let d = |extra: &[(&str, &str)]| {
            let mut v = vec![("InterfaceName".to_string(), "en0".to_string()), ("Router".to_string(), "192.168.0.1".to_string())];
            v.extend(extra.iter().map(|(k, x)| (k.to_string(), x.to_string())));
            scutil::router(&v).unwrap().router.lladdr
        };
        assert_eq!(d(&[]), None);
        assert_eq!(d(&[("ARPResolvedIPAddress", "192.168.0.9"), ("ARPResolvedHardwareAddress", "1:2:3:4:5:6")]), None);
        assert_eq!(
            d(&[("ARPResolvedIPAddress", "192.168.0.9"), ("ARPResolvedHardwareAddress", "1:2:3:4:5:6"), ("NetworkSignature", "IPv4.Router=192.168.0.1;IPv4.RouterHardwareAddress=a:b:c:d:e:f")]),
            Some(mac("0a:0b:0c:0d:0e:0f"))
        );
        assert_eq!(d(&[("NetworkSignature", "IPv4.Router=192.168.0.2;IPv4.RouterHardwareAddress=a:b:c:d:e:f")]), None);
        assert_eq!(d(&[("ARPResolvedIPAddress", "192.168.0.1"), ("ARPResolvedHardwareAddress", "not-a-mac")]), None);
        let zoned = vec![("InterfaceName".to_string(), "en0".to_string()), ("Router".to_string(), "fe80::1%en0".to_string())];
        assert_eq!(scutil::router(&zoned).unwrap().router.ip, ip("fe80::1"));
    }

    #[test]
    fn scutil_keys_are_checked_before_they_go_back_into_its_stdin() {
        let list = "  subKey [0] = State:/Network/Service/435849CF-7F43-46BE-BD70-107B4BD08F20/IPv4
  subKey [1] = State:/Network/Service/CE2A0D16-7640-45D5-84D0-2B1AC0F655EA/IPv6
  subKey [2] = State:/Network/Service/x y/IPv4
  subKey [3] = State:/Network/Service/abc/DNS
  subKey [4] = State:/Network/Global/IPv4
  no subKeys.
";
        let k = scutil::keys(list);
        assert_eq!(k, ["State:/Network/Service/435849CF-7F43-46BE-BD70-107B4BD08F20/IPv4", "State:/Network/Service/CE2A0D16-7640-45D5-84D0-2B1AC0F655EA/IPv6"]);
        assert_eq!(scutil::shows(&k).lines().count(), 2);
    }

    /// The live read on Linux, checked against independent sources:
    /// `/proc/net/route` (the main table's IPv4 defaults) and
    /// `/proc/net/arp` (complete entries, flag 0x2), both ways — a complete
    /// ARP entry for the router must be read, and an lladdr must come from one.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_live_linux_read_agrees_with_proc() {
        let snap = snapshot().expect("snapshot");
        let routes = std::fs::read_to_string("/proc/net/route").unwrap();
        let arp = std::fs::read_to_string("/proc/net/arp").unwrap();
        let mut seen = 0;
        for l in routes.lines().skip(1) {
            let f: Vec<&str> = l.split_whitespace().collect();
            let flags = u32::from_str_radix(f[3], 16).unwrap();
            if f[1] != "00000000" || f[7] != "00000000" || flags & 0x2 == 0 {
                continue;
            }
            let gw = IpAddr::from(u32::from_str_radix(f[2], 16).unwrap().to_le_bytes());
            let mine = snap.routers.iter().find(|r| r.iface == f[0] && r.router.ip.is_ipv4()).unwrap_or_else(|| panic!("no router for {} in {:?}", f[0], snap.routers));
            assert_eq!(mine.router.ip, gw);
            let entry = arp.lines().skip(1).map(|l| l.split_whitespace().collect::<Vec<_>>()).find(|a| a[0] == gw.to_string() && a[5] == f[0] && u32::from_str_radix(a[2].trim_start_matches("0x"), 16).unwrap() & 0x2 != 0);
            assert_eq!(mine.router.lladdr, entry.map(|a| mac(a[3])), "{f:?}");
            seen += 1;
        }
        assert!(seen > 0, "no IPv4 default route in /proc/net/route: this witness needs a network");
    }

    /// The live read on macOS, checked against `route -n get default`: when
    /// the primary default names a gateway address, our read has that router
    /// on that interface. A link-layer address, when read, is six bytes.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_live_macos_read_agrees_with_the_routing_table() {
        let snap = snapshot().expect("snapshot");
        let o = worktrees_core::childenv::command("/sbin/route").args(["-n", "get", "default"]).output().expect("route");
        let text = String::from_utf8_lossy(&o.stdout);
        let field = |k: &str| text.lines().find_map(|l| l.trim().strip_prefix(k).map(|v| v.trim().to_string()));
        let iface = field("interface:").expect("no default route: this witness needs a network");
        if let Some(gw) = field("gateway:").and_then(|g| g.parse::<IpAddr>().ok()) {
            assert!(
                snap.routers.iter().any(|r| r.iface == iface && r.router.ip == gw),
                "default via {gw} on {iface}, read {:?}",
                snap.routers
            );
        }
        for r in &snap.routers {
            assert!(r.router.lladdr.as_ref().is_none_or(|l| l.0.len() == 6), "{r:?}");
            assert!(snap.ifaces.iter().any(|i| i.name == r.iface), "{r:?}");
        }
    }
}
