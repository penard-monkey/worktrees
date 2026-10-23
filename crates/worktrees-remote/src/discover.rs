//! Looking for a moved host on the LAN (Phase 2 §4.7, the device's half).
//!
//! Serve advertises `_worktrees._tcp` in LAN mode: the instance and the TXT
//! key `id` are its discovery id, the SRV names `wt-<id>.local`, whose one
//! address record is the bound address. A device browses only when the
//! address it kept stops answering ([`crate::reach`]), and every advert is
//! UNTRUSTED input — anyone on the link can send one, with any id it copied.
//! So this module only SUGGESTS: a [`Sighting`] becomes a [`Candidate`] here,
//! and a candidate is adopted only after a pinned open to it succeeds, which
//! is `reach`'s rule, not this one's.
//!
//! A sighting is a candidate only if ([`candidate`]):
//! - its TXT record is exactly `v=1` and `id=<26 base32>`, and that id is the
//!   host's discovery id. The instance name is not read at all: mDNS renames
//!   an instance whose name another host claimed first, so a squatter could
//!   push the real advert out of sight under its own name;
//! - its address passes the classifier (`netclass::classify`), and
//! - it is on-link for the interface the advert arrived on: inside one of
//!   that interface's own subnets, or link-local. Otherwise a spoofed advert
//!   could aim the device at anything it can reach, a VPN's networks
//!   included, and use it as a probe.
//!
//! The browse itself is behind [`Browser`], so the rule is tested with a
//! scripted one. On macOS it is `dns_sd` (libSystem, no crate), exactly as
//! serve's advert registers through it; elsewhere [`platform`] says in words
//! that there is nothing to browse with, and the address code is the way.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use worktrees_core::netclass::{self, Class, NotPrivate};
use worktrees_core::netif::{network, Iface};

/// The service type serve advertises.
pub const SERVICE: &str = "_worktrees._tcp";

/// The most resolved addresses one round hands over: a link flooded with
/// adverts costs a bounded amount, and the address code is the way back.
pub const MAX_SIGHTINGS: usize = 64;

/// One address a browse turned up: an advert of `_worktrees._tcp`, resolved,
/// and one address of the host its SRV names — as seen on the interface the
/// advert arrived on. UNTRUSTED.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sighting {
    /// The TXT record's rdata, as it came.
    pub txt: Vec<u8>,
    pub ip: IpAddr,
    pub port: u16,
    /// The index of the interface the advert arrived on.
    pub iface: u32,
}

/// What looks for adverts.
pub trait Browser: Send + Sync {
    /// Look for [`SERVICE`] for at most `round`, sending each resolved address
    /// as it arrives (at most [`MAX_SIGHTINGS`]). The browse ends when the
    /// round does or the receiver is dropped. `Err`, in words, when there is
    /// nothing to browse with.
    fn browse(&self, round: Duration) -> Result<mpsc::Receiver<Sighting>, String>;
}

/// This platform's browser: the OS's own responder, or one that explains why
/// there is none.
pub fn platform() -> Arc<dyn Browser> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(DnsSd::default())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Arc::new(Unavailable(
            "this client cannot look for a host on the LAN on this platform yet (mDNS browsing is built for macOS only)",
        ))
    }
}

/// A browser that has nothing to browse with, and says why.
pub struct Unavailable(pub &'static str);

impl Browser for Unavailable {
    fn browse(&self, _: Duration) -> Result<mpsc::Receiver<Sighting>, String> {
        Err(self.0.to_string())
    }
}

/// The discovery id in a TXT record of exactly the shape serve writes
/// (§4.7): two strings, `v=1` and `id=<26 base32>`, once each, in either
/// order, and nothing else — not a third key, not an empty string, not a key
/// in another case.
pub fn txt_id(txt: &[u8]) -> Option<&str> {
    let mut parts = Vec::new();
    let mut rest = txt;
    while let Some((&n, tail)) = rest.split_first() {
        let n = n as usize;
        if n == 0 || tail.len() < n {
            return None;
        }
        parts.push(&tail[..n]);
        rest = &tail[n..];
    }
    let id = match parts.as_slice() {
        [b"v=1", id] | [id, b"v=1"] => id.strip_prefix(b"id=")?,
        _ => return None,
    };
    let id = std::str::from_utf8(id).ok()?;
    crate::hosts::is_b32_id(id).then_some(id)
}

/// A sighting that may be dialled with the pin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// The address, unscoped: what the record keeps if the pin holds.
    pub addr: SocketAddr,
    /// The interface the advert arrived on. A link-local address is dialled
    /// through it and no other.
    pub iface: Iface,
    pub class: Class,
}

/// Why a sighting is not a candidate. Never dialled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Skip {
    /// Not serve's TXT shape, or another host's id: someone else's advert.
    NotOurs,
    /// The classifier refused the address.
    NotPrivate(NotPrivate),
    /// The interface it arrived on is not one this device can use: gone,
    /// down, loopback, point-to-point (an overlay is never advertised), or
    /// the responder's local-only pseudo-interface.
    Interface(u32),
    /// Outside every subnet of the interface it arrived on.
    OffLink { addr: SocketAddr, iface: String },
}

impl std::fmt::Display for Skip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Skip::NotOurs => write!(f, "not this host's advert"),
            Skip::NotPrivate(n) => write!(f, "an advert named {}: {n}; not dialled", n.ip),
            Skip::Interface(i) => write!(f, "an advert arrived on interface {i}, which this device does not dial through; not dialled"),
            Skip::OffLink { addr, iface } => write!(f, "an advert named {addr}, which is not on {iface}'s own network; not dialled"),
        }
    }
}

/// Is `s` a candidate for the host whose discovery id is `id`, given this
/// device's interfaces as they are now? The checks run cheapest first, and
/// nothing here dials.
pub fn candidate(s: &Sighting, id: &str, ifaces: &[Iface]) -> Result<Candidate, Skip> {
    if txt_id(&s.txt) != Some(id) {
        return Err(Skip::NotOurs);
    }
    let class = netclass::classify(s.ip).map_err(Skip::NotPrivate)?;
    let iface = ifaces
        .iter()
        .find(|i| i.index == s.iface && i.usable() && !i.loopback && !i.point_to_point)
        .ok_or(Skip::Interface(s.iface))?;
    let addr = SocketAddr::new(s.ip, s.port);
    let on_link = class == Class::LinkLocal
        || iface.addrs.iter().any(|a| {
            a.ip.is_ipv4() == s.ip.is_ipv4()
                && netclass::row_of(a.ip).is_none_or(|r| r.class != Class::LinkLocal)
                && network(a.ip, a.prefix) == network(s.ip, a.prefix)
        });
    if !on_link {
        return Err(Skip::OffLink { addr, iface: iface.name.clone() });
    }
    Ok(Candidate { addr, iface: iface.clone(), class })
}

#[cfg(target_os = "macos")]
pub use macos::DnsSd;

#[cfg(target_os = "macos")]
mod macos {
    //! `DNSServiceBrowse` → `DNSServiceResolve` → `DNSServiceGetAddrInfo`, all
    //! on ONE connection (`DNSServiceCreateConnection` and
    //! `kDNSServiceFlagsShareConnection`), each on the interface the browse
    //! answer arrived on, so an address is the one that interface saw. One
    //! thread owns the connection for the round and is the only one that
    //! touches it; callbacks run only inside its `DNSServiceProcessResult`.
    //! Deallocating the connection ends every operation on it (dns_sd.h,
    //! "Don't Double-Deallocate"), so the subordinate refs are never freed
    //! one by one.

    use std::collections::HashSet;
    use std::ffi::{c_char, c_void, CStr};
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use std::time::{Duration, Instant};

    use tokio::sync::mpsc;

    use super::{Browser, Sighting, MAX_SIGHTINGS, SERVICE};

    #[repr(C)]
    struct Opaque {
        _p: [u8; 0],
    }
    type DNSServiceRef = *mut Opaque;
    type BrowseReply = extern "C" fn(DNSServiceRef, u32, u32, i32, *const c_char, *const c_char, *const c_char, *mut c_void);
    type ResolveReply = extern "C" fn(DNSServiceRef, u32, u32, i32, *const c_char, *const c_char, u16, u16, *const u8, *mut c_void);
    type AddrReply = extern "C" fn(DNSServiceRef, u32, u32, i32, *const c_char, *const libc::sockaddr, u32, *mut c_void);

    extern "C" {
        fn DNSServiceCreateConnection(sd: *mut DNSServiceRef) -> i32;
        fn DNSServiceBrowse(sd: *mut DNSServiceRef, flags: u32, iface: u32, regtype: *const c_char, domain: *const c_char, cb: BrowseReply, ctx: *mut c_void) -> i32;
        #[allow(clippy::too_many_arguments)]
        fn DNSServiceResolve(
            sd: *mut DNSServiceRef,
            flags: u32,
            iface: u32,
            name: *const c_char,
            regtype: *const c_char,
            domain: *const c_char,
            cb: ResolveReply,
            ctx: *mut c_void,
        ) -> i32;
        fn DNSServiceGetAddrInfo(sd: *mut DNSServiceRef, flags: u32, iface: u32, protocol: u32, host: *const c_char, cb: AddrReply, ctx: *mut c_void) -> i32;
        fn DNSServiceRefSockFD(sd: DNSServiceRef) -> i32;
        fn DNSServiceProcessResult(sd: DNSServiceRef) -> i32;
        fn DNSServiceRefDeallocate(sd: DNSServiceRef);
    }

    const FLAG_ADD: u32 = 0x2;
    const SHARE_CONNECTION: u32 = 0x4000;
    const PROTOCOL_V4: u32 = 0x1;
    const PROTOCOL_V6: u32 = 0x2;
    /// The most instances one round resolves.
    const MAX_RESOLVES: usize = 32;
    /// How often the thread looks at the clock and the receiver.
    const TICK: Duration = Duration::from_millis(100);

    /// The browse, every interface (`0`) by default. A test passes the
    /// responder's local-only index, so no packet leaves the machine.
    #[derive(Default)]
    pub struct DnsSd {
        pub iface: u32,
    }

    /// What the callbacks share, on the connection's thread.
    struct Ctx {
        conn: DNSServiceRef,
        tx: mpsc::Sender<Sighting>,
        /// The resolve and address operations' contexts (`Box::into_raw`);
        /// freed only after the connection is deallocated.
        ops: Vec<*mut Op>,
        resolved: HashSet<(u32, Vec<u8>)>,
        sent: usize,
    }

    /// One resolve's or one address lookup's context.
    struct Op {
        ctx: *mut Ctx,
        txt: Vec<u8>,
        port: u16,
    }

    /// The connection and its context (`Box::into_raw`), moved to the thread
    /// that alone uses them.
    struct Owned(DNSServiceRef, *mut Ctx);
    // SAFETY: after `browse` hands it over, only the spawned thread touches
    // the connection or the context.
    unsafe impl Send for Owned {}

    extern "C" fn on_browse(_: DNSServiceRef, flags: u32, iface: u32, code: i32, name: *const c_char, regtype: *const c_char, domain: *const c_char, ctx: *mut c_void) {
        if code != 0 || flags & FLAG_ADD == 0 || name.is_null() || regtype.is_null() || domain.is_null() {
            return;
        }
        let ctxp = ctx as *mut Ctx;
        // SAFETY: `ctx` is the round's `Ctx`, which outlives the connection;
        // callbacks run one at a time on its thread.
        let c = unsafe { &mut *ctxp };
        // SAFETY: dns_sd hands over NUL-terminated strings, valid for the call.
        let key = (iface, unsafe { CStr::from_ptr(name) }.to_bytes().to_vec());
        if c.resolved.len() >= MAX_RESOLVES || !c.resolved.insert(key) {
            return;
        }
        let op = Box::into_raw(Box::new(Op { ctx: ctxp, txt: Vec::new(), port: 0 }));
        c.ops.push(op);
        let mut sd = c.conn;
        // SAFETY: as dns_sd.h documents it, on the shared connection; `op`
        // is freed only after the connection is deallocated.
        unsafe { DNSServiceResolve(&mut sd, SHARE_CONNECTION, iface, name, regtype, domain, on_resolve, op as *mut c_void) };
    }

    extern "C" fn on_resolve(_: DNSServiceRef, _: u32, iface: u32, code: i32, _: *const c_char, target: *const c_char, port: u16, txt_len: u16, txt: *const u8, ctx: *mut c_void) {
        if code != 0 || target.is_null() {
            return;
        }
        // SAFETY: `ctx` is one of `ops`, alive until the connection goes; read
        // its round pointer and let the borrow end before touching `Ctx`.
        let ctxp = unsafe { (*(ctx as *const Op)).ctx };
        let txt = if txt.is_null() || txt_len == 0 {
            Vec::new()
        } else {
            // SAFETY: dns_sd hands over `txt_len` bytes, valid for the call.
            unsafe { std::slice::from_raw_parts(txt, txt_len as usize) }.to_vec()
        };
        // SAFETY: as `on_browse`.
        let c = unsafe { &mut *ctxp };
        if c.ops.len() >= 2 * MAX_RESOLVES {
            return;
        }
        let op = Box::into_raw(Box::new(Op { ctx: ctxp, txt, port: u16::from_be(port) }));
        c.ops.push(op);
        let mut sd = c.conn;
        // SAFETY: as in `on_browse`; `target` is valid for this call, and
        // dns_sd copies it.
        unsafe { DNSServiceGetAddrInfo(&mut sd, SHARE_CONNECTION, iface, PROTOCOL_V4 | PROTOCOL_V6, target, on_addr, op as *mut c_void) };
    }

    extern "C" fn on_addr(_: DNSServiceRef, flags: u32, iface: u32, code: i32, _: *const c_char, sa: *const libc::sockaddr, _: u32, ctx: *mut c_void) {
        if code != 0 || flags & FLAG_ADD == 0 || sa.is_null() {
            return;
        }
        // SAFETY: dns_sd hands over a sockaddr of the family it names.
        let ip = unsafe {
            match (*sa).sa_family as i32 {
                libc::AF_INET => {
                    let v = &*(sa as *const libc::sockaddr_in);
                    IpAddr::V4(Ipv4Addr::from(u32::from_be(v.sin_addr.s_addr)))
                }
                libc::AF_INET6 => IpAddr::V6(Ipv6Addr::from((*(sa as *const libc::sockaddr_in6)).sin6_addr.s6_addr)),
                _ => return,
            }
        };
        // SAFETY: as `on_resolve`.
        let (ctxp, txt, port) = unsafe {
            let op = &*(ctx as *const Op);
            (op.ctx, op.txt.clone(), op.port)
        };
        // SAFETY: as `on_browse`.
        let c = unsafe { &mut *ctxp };
        if c.sent < MAX_SIGHTINGS && c.tx.try_send(Sighting { txt, ip, port, iface }).is_ok() {
            c.sent += 1;
        }
    }

    fn run(owned: Owned, round: Duration) {
        let Owned(conn, ctxp) = owned;
        let end = Instant::now() + round;
        // SAFETY: the connection is live until the deallocate below, on this
        // thread alone; `ctxp` is freed only after it.
        unsafe {
            let sock = DNSServiceRefSockFD(conn);
            loop {
                let left = end.saturating_duration_since(Instant::now());
                if left.is_zero() || (*ctxp).tx.is_closed() || sock < 0 {
                    break;
                }
                let ms = left.min(TICK).as_millis().max(1) as i32;
                let mut fd = libc::pollfd { fd: sock, events: libc::POLLIN, revents: 0 };
                let n = libc::poll(&mut fd, 1, ms);
                if n < 0 {
                    if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    break;
                }
                if n > 0 && DNSServiceProcessResult(conn) != 0 {
                    break;
                }
            }
            // Ends the browse and every resolve and lookup on it; then
            // nothing can call back into the contexts.
            DNSServiceRefDeallocate(conn);
            let ctx = Box::from_raw(ctxp);
            for op in &ctx.ops {
                drop(Box::from_raw(*op));
            }
        }
    }

    impl Browser for DnsSd {
        fn browse(&self, round: Duration) -> Result<mpsc::Receiver<Sighting>, String> {
            let (tx, rx) = mpsc::channel(MAX_SIGHTINGS);
            let mut conn: DNSServiceRef = std::ptr::null_mut();
            // SAFETY: dns_sd.h's calls; on any failure the connection is
            // deallocated here, before the context it points at is dropped.
            unsafe {
                let e = DNSServiceCreateConnection(&mut conn);
                if e != 0 {
                    return Err(format!("mDNSResponder could not be reached (dns_sd error {e})"));
                }
                let ctx = Box::into_raw(Box::new(Ctx { conn, tx, ops: Vec::new(), resolved: HashSet::new(), sent: 0 }));
                let regtype = std::ffi::CString::new(SERVICE).expect("no NUL");
                let mut sd = conn;
                let e = DNSServiceBrowse(&mut sd, SHARE_CONNECTION, self.iface, regtype.as_ptr(), c"local.".as_ptr(), on_browse, ctx as *mut c_void);
                if e != 0 {
                    DNSServiceRefDeallocate(conn);
                    drop(Box::from_raw(ctx));
                    return Err(format!("mDNSResponder refused the browse (dns_sd error {e})"));
                }
                let owned = Owned(conn, ctx);
                if let Err(e) = std::thread::Builder::new().name("mdns-browse".into()).spawn(move || run(owned, round)) {
                    // The closure was dropped unrun: the connection and its
                    // context leak (no callback can run without a thread
                    // calling `DNSServiceProcessResult`).
                    return Err(format!("mDNS: {e}"));
                }
            }
            Ok(rx)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use worktrees_core::netif::IfAddr;

    pub fn txt(parts: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for p in parts {
            out.push(p.len() as u8);
            out.extend_from_slice(p.as_bytes());
        }
        out
    }

    fn id() -> String {
        worktrees_core::pairuri::base32(&[7; 16])
    }

    fn en0() -> Iface {
        let a = |ip: &str, prefix| IfAddr { ip: ip.parse().unwrap(), prefix, temporary: false, deprecated: false };
        Iface {
            name: "en0".into(),
            index: 4,
            up: true,
            running: true,
            loopback: false,
            point_to_point: false,
            addrs: vec![a("192.168.1.20", 24), a("fe80::1c", 64), a("fd00:1::20", 64)],
        }
    }

    fn sighting(ip: &str) -> Sighting {
        Sighting { txt: txt(&["v=1", &format!("id={}", id())]), ip: ip.parse().unwrap(), port: 7461, iface: 4 }
    }

    /// §4.7: exactly `v=1` and the id, once each, either order — the shape
    /// serve's `advert::txt` writes — and nothing else.
    #[test]
    fn only_serves_txt_shape_names_an_id() {
        let id = id();
        let idkv = format!("id={id}");
        assert_eq!(txt_id(&txt(&["v=1", &idkv])), Some(id.as_str()));
        assert_eq!(txt_id(&txt(&[&idkv, "v=1"])), Some(id.as_str()), "order is not meaning");
        for (what, t) in [
            ("empty", Vec::new()),
            ("no version", txt(&[&idkv])),
            ("another version", txt(&["v=2", &idkv])),
            ("a third key", txt(&["v=1", &idkv, "pin=ab"])),
            ("twice", txt(&["v=1", &idkv, &idkv])),
            ("version twice", txt(&["v=1", "v=1"])),
            ("an upper-case key", txt(&["v=1", &format!("ID={id}")])),
            ("a short id", txt(&["v=1", &format!("id={}", &id[..25])])),
            ("a long id", txt(&["v=1", &format!("id={id}a")])),
            ("not base32", txt(&["v=1", "id=0000000000000000000000000!"])),
            ("an empty string", [txt(&["v=1", &idkv]), vec![0]].concat()),
            ("a length past the end", [txt(&["v=1"]), vec![40, b'i', b'd']].concat()),
        ] {
            assert_eq!(txt_id(&t), None, "{what}");
        }
    }

    #[test]
    fn a_sighting_on_the_interfaces_own_network_is_a_candidate() {
        let ifs = [en0()];
        for ip in ["192.168.1.9", "fd00:1::9", "fe80::99"] {
            let c = candidate(&sighting(ip), &id(), &ifs).unwrap_or_else(|e| panic!("{ip}: {e}"));
            assert_eq!(c.addr, SocketAddr::new(ip.parse().unwrap(), 7461));
            assert_eq!(c.iface.name, "en0");
        }
    }

    /// Another host's advert, or this host's id in another shape, is not
    /// ours, whatever its address.
    #[test]
    fn another_id_or_shape_is_not_a_candidate() {
        let ifs = [en0()];
        let mut s = sighting("192.168.1.9");
        s.txt = txt(&["v=1", &format!("id={}", worktrees_core::pairuri::base32(&[8; 16]))]);
        assert_eq!(candidate(&s, &id(), &ifs), Err(Skip::NotOurs));
        s.txt = txt(&["v=1", &format!("id={}", id()), "x=y"]);
        assert_eq!(candidate(&s, &id(), &ifs), Err(Skip::NotOurs));
    }

    /// The classifier decides before anything else about the address: an
    /// interface that claims to hold a public or loopback network does not
    /// make one of its addresses dialable.
    #[test]
    fn the_classifier_refuses_before_the_link_is_asked() {
        let mut odd = en0();
        let a = |ip: &str, prefix| IfAddr { ip: ip.parse().unwrap(), prefix, temporary: false, deprecated: false };
        odd.addrs = vec![a("8.8.8.1", 24), a("127.0.0.2", 8), a("2001:db8::1", 64), a("100.64.0.1", 10)];
        for ip in ["8.8.8.8", "127.0.0.1", "2001:db8::9", "0.0.0.0", "::1", "::ffff:192.168.1.9", "224.0.0.251"] {
            match candidate(&sighting(ip), &id(), std::slice::from_ref(&odd)) {
                Err(Skip::NotPrivate(_)) => {}
                other => panic!("{ip}: {other:?}"),
            }
        }
    }

    /// On-link only: an address outside every subnet of the interface it
    /// arrived on, or from an interface this device does not dial through,
    /// is never dialled.
    #[test]
    fn an_off_link_sighting_or_a_foreign_interface_is_not_a_candidate() {
        let ifs = [en0()];
        for ip in ["192.168.2.9", "10.0.0.9", "fd00:2::9", "100.64.0.9"] {
            match candidate(&sighting(ip), &id(), &ifs) {
                Err(Skip::OffLink { .. }) => {}
                other => panic!("{ip}: {other:?}"),
            }
        }
        // Link-local is on-link by definition, and dialled through the
        // interface it arrived on and no other.
        let ll = candidate(&sighting("169.254.1.1"), &id(), &ifs).unwrap();
        assert_eq!((ll.class, ll.iface.index), (Class::LinkLocal, 4));
        let mut s = sighting("192.168.1.9");
        for (what, iface) in [("gone", 9), ("local-only", u32::MAX)] {
            s.iface = iface;
            assert_eq!(candidate(&s, &id(), &ifs), Err(Skip::Interface(iface)), "{what}");
        }
        let mut lo = en0();
        lo.loopback = true;
        let mut tun = en0();
        tun.point_to_point = true;
        let mut down = en0();
        down.running = false;
        for (what, i) in [("loopback", lo), ("point-to-point", tun), ("down", down)] {
            assert_eq!(candidate(&sighting("192.168.1.9"), &id(), &[i]), Err(Skip::Interface(4)), "{what}");
        }
    }

    /// The real responder, on its LOCAL-ONLY interface (no packet leaves the
    /// machine): an advert registered the way serve's is — `dns-sd -P`, the
    /// same two records — is browsed, resolved and handed over with its TXT,
    /// port and address. Its interface is the local-only one, which
    /// [`candidate`] never dials through.
    #[cfg(target_os = "macos")]
    #[test]
    fn mdnsresponder_hands_over_a_resolved_advert() {
        const LOCAL_ONLY: u32 = u32::MAX;
        let mut b = [0u8; 16];
        getrandom::fill(&mut b).unwrap();
        let id = worktrees_core::pairuri::base32(&b);
        let host = format!("wt-{id}.local");
        let mut reg = std::process::Command::new("/usr/bin/dns-sd")
            .args(["-lo", "-P", &id, SERVICE, "local", "7443", &host, "10.99.0.7", "v=1", &format!("id={id}")])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        struct Kill<'a>(&'a mut std::process::Child);
        impl Drop for Kill<'_> {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let _reg = Kill(&mut reg);
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let found = rt.block_on(async {
            let mut rx = DnsSd { iface: LOCAL_ONLY }.browse(Duration::from_secs(8)).unwrap();
            while let Some(s) = rx.recv().await {
                if txt_id(&s.txt) == Some(id.as_str()) {
                    return Some(s);
                }
            }
            None
        });
        let s = found.expect("the advert was browsed and resolved within the round");
        assert_eq!(s.ip, "10.99.0.7".parse::<IpAddr>().unwrap());
        assert_eq!(s.port, 7443);
        assert_eq!(s.iface, LOCAL_ONLY);
        assert_eq!(candidate(&s, &id, &worktrees_core::netif::interfaces().unwrap()), Err(Skip::Interface(LOCAL_ONLY)));
    }

    /// A round ends on time, and a dropped receiver ends it early.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_browse_round_is_bounded() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let t = std::time::Instant::now();
        rt.block_on(async {
            let mut rx = DnsSd { iface: u32::MAX }.browse(Duration::from_millis(600)).unwrap();
            while rx.recv().await.is_some() {}
        });
        assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    }
}
