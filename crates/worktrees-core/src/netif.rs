//! The host's network interfaces, as plain data: each one's name, index and
//! flags, and every address it holds with its prefix length and, for IPv6,
//! the two flags a bind refuses (temporary, deprecated).
//!
//! One reader for both ends of the remote listener (Server Edition Phase 2,
//! `docs/proposals/server-edition-phase2.md` §4.1): serve checks every poll
//! that its address is still on the interface it bound, and fingerprints the
//! network it is on; a device dialling a link-local address picks its OWN
//! interface from the same list (`netclass::linklocal_candidates`). All of it
//! is readable without privilege on both platforms (measured, network spike
//! 2026-10-21, `scripts/spike/p2-network/netfp.c`): `getifaddrs` for names,
//! flags and prefixes, then `SIOCGIFAFLAG_IN6` on macOS and
//! `/proc/net/if_inet6` on Linux for the IPv6 flags.
//!
//! The index is read with `if_nametoindex` after the walk, so an interface
//! re-created between the two reads reports its NEW index — which is what a
//! poll wants: the next one sees the change and treats it as the address
//! leaving (a listener bound to a device is stranded on the old index).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// One interface as the OS reports it at the moment of the read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Iface {
    pub name: String,
    pub index: u32,
    /// `IFF_UP`: administratively up.
    pub up: bool,
    /// `IFF_RUNNING`: the link is live. Linux keeps the addresses of an
    /// interface that is down, so "holds the address" alone is not "usable".
    pub running: bool,
    pub loopback: bool,
    /// `IFF_POINTOPOINT`: a TUN (Tailscale's `utun` on macOS, `tailscale0` on
    /// Linux) or WireGuard. A TAP overlay (ZeroTier) is NOT, and gets LAN rules.
    pub point_to_point: bool,
    pub addrs: Vec<IfAddr>,
}

/// One address on an interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IfAddr {
    pub ip: IpAddr,
    pub prefix: u8,
    /// IPv6 only: a SLAAC privacy address, which rotates within a day.
    pub temporary: bool,
    /// IPv6 only: its preferred lifetime has run out.
    pub deprecated: bool,
}

impl Iface {
    /// Up and running: the only state in which an address on it counts as
    /// held.
    pub fn usable(&self) -> bool {
        self.up && self.running
    }

    /// This interface's entry for `ip`, compared on the canonical bytes.
    pub fn holds(&self, ip: IpAddr) -> Option<&IfAddr> {
        self.addrs.iter().find(|a| a.ip == ip)
    }
}

/// The network `ip/prefix` belongs to: the address with its host bits
/// cleared. A prefix past the family's width is clamped to it.
pub fn network(ip: IpAddr, prefix: u8) -> IpAddr {
    match ip {
        IpAddr::V4(v) => {
            let p = u32::from(prefix.min(32));
            let mask = if p == 0 { 0 } else { u32::MAX << (32 - p) };
            IpAddr::V4(Ipv4Addr::from(u32::from(v) & mask))
        }
        IpAddr::V6(v) => {
            let p = u32::from(prefix.min(128));
            let mask = if p == 0 { 0 } else { u128::MAX << (128 - p) };
            IpAddr::V6(Ipv6Addr::from(u128::from(v) & mask))
        }
    }
}

/// The number of leading one bits in a netmask's bytes. A non-contiguous mask
/// (which no interface we read has) counts only its leading run.
fn prefix_of(mask: &[u8]) -> u8 {
    let mut n = 0u8;
    for b in mask {
        n += b.leading_ones() as u8;
        if *b != 0xff {
            break;
        }
    }
    n
}

/// Read every interface. `Err` only when `getifaddrs` itself fails; an IPv6
/// flag that cannot be read is reported as "neither", which a bind treats as
/// a stable address — the same answer the spike's Mac gave for its ULA.
pub fn interfaces() -> std::io::Result<Vec<Iface>> {
    use std::ffi::CStr;
    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs writes a list head we free below, once.
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut out: Vec<Iface> = Vec::new();
    let mut cur = head;
    while !cur.is_null() {
        // SAFETY: a node of the list getifaddrs returned, alive until free.
        let ifa = unsafe { &*cur };
        cur = ifa.ifa_next;
        if ifa.ifa_name.is_null() {
            continue;
        }
        let name = unsafe { CStr::from_ptr(ifa.ifa_name) }.to_string_lossy().into_owned();
        let flags = ifa.ifa_flags as libc::c_int;
        let i = match out.iter().position(|x| x.name == name) {
            Some(i) => i,
            None => {
                out.push(Iface {
                    name,
                    index: 0,
                    up: flags & libc::IFF_UP != 0,
                    running: flags & libc::IFF_RUNNING != 0,
                    loopback: flags & libc::IFF_LOOPBACK != 0,
                    point_to_point: flags & libc::IFF_POINTOPOINT != 0,
                    addrs: Vec::new(),
                });
                out.len() - 1
            }
        };
        if ifa.ifa_addr.is_null() {
            continue;
        }
        // SAFETY: ifa_addr is a sockaddr whose family says which one.
        let fam = i32::from(unsafe { (*ifa.ifa_addr).sa_family });
        let mask = ifa.ifa_netmask;
        let entry = if fam == libc::AF_INET {
            let a = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_in) };
            let ip = Ipv4Addr::from(u32::from_be(a.sin_addr.s_addr));
            let prefix = if mask.is_null() {
                32
            } else {
                let m = unsafe { &*(mask as *const libc::sockaddr_in) };
                prefix_of(&m.sin_addr.s_addr.to_ne_bytes())
            };
            IfAddr { ip: IpAddr::V4(ip), prefix, temporary: false, deprecated: false }
        } else if fam == libc::AF_INET6 {
            let a = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_in6) };
            let ip = unembed_scope(Ipv6Addr::from(a.sin6_addr.s6_addr));
            let prefix = if mask.is_null() {
                128
            } else {
                let m = unsafe { &*(mask as *const libc::sockaddr_in6) };
                prefix_of(&m.sin6_addr.s6_addr)
            };
            IfAddr { ip: IpAddr::V6(ip), prefix, temporary: false, deprecated: false }
        } else {
            continue;
        };
        if !out[i].addrs.contains(&entry) {
            out[i].addrs.push(entry);
        }
    }
    // SAFETY: the head getifaddrs returned, freed once; no reference into the
    // list outlives this (every field above was copied out).
    unsafe { libc::freeifaddrs(head) };
    for iface in &mut out {
        iface.index = index_of(&iface.name);
    }
    v6flags::fill(&mut out);
    Ok(out)
}

/// `if_nametoindex`, 0 when the interface is gone.
pub fn index_of(name: &str) -> u32 {
    let Ok(c) = std::ffi::CString::new(name) else { return 0 };
    // SAFETY: a NUL-terminated name; the call only reads it.
    unsafe { libc::if_nametoindex(c.as_ptr()) }
}

/// The KAME convention some BSD interfaces still follow puts a link-local
/// address's scope id in its second 16-bit word (`fe80:e::1` for `fe80::1` on
/// index 14). The address's canonical bytes have zero there, by RFC 4291, so
/// clear it; the zone is the interface's index, which `Iface` carries anyway.
fn unembed_scope(ip: Ipv6Addr) -> Ipv6Addr {
    let mut s = ip.segments();
    if s[0] & 0xffc0 == 0xfe80 {
        s[1] = 0;
    }
    Ipv6Addr::from(s)
}

#[cfg(target_os = "macos")]
mod v6flags {
    //! `SIOCGIFAFLAG_IN6` on a datagram socket, per address: what `ifconfig`
    //! prints as `temporary` / `deprecated`. Unprivileged (measured, spike).

    use super::Iface;
    use std::net::IpAddr;

    /// `_IOWR('i', 73, struct in6_ifreq)`: read off the macOS 27 SDK for both
    /// arm64 and x86_64 (the struct is the same on both; 288 bytes), and
    /// pinned by `the_in6_ifreq_layout_matches_the_ioctl_number` below.
    pub(super) const SIOCGIFAFLAG_IN6: libc::c_ulong = 0xc120_6949;
    pub(super) const IN6_IFF_DEPRECATED: libc::c_int = 0x10;
    pub(super) const IN6_IFF_TEMPORARY: libc::c_int = 0x80;

    /// `struct in6_ifreq`: the name, then a union whose largest members are
    /// the per-interface statistics. Only the address (in) and `ifru_flags6`
    /// (out) are used; the rest is there so the size matches the ioctl's.
    #[repr(C)]
    pub(super) struct In6Ifreq {
        pub name: [libc::c_char; libc::IFNAMSIZ],
        pub ifru: Ifru,
    }

    #[repr(C)]
    pub(super) union Ifru {
        pub addr: libc::sockaddr_in6,
        pub flags6: libc::c_int,
        _size: [u64; 34],
    }

    pub(super) fn fill(ifaces: &mut [Iface]) {
        // SAFETY: a plain datagram socket, closed below.
        let s = unsafe { libc::socket(libc::AF_INET6, libc::SOCK_DGRAM, 0) };
        if s < 0 {
            return;
        }
        for iface in ifaces.iter_mut() {
            if iface.name.len() >= libc::IFNAMSIZ {
                continue;
            }
            for a in iface.addrs.iter_mut() {
                let IpAddr::V6(v6) = a.ip else { continue };
                // SAFETY: an all-zero in6_ifreq is a valid value of it.
                let mut r: In6Ifreq = unsafe { std::mem::zeroed() };
                for (d, b) in r.name.iter_mut().zip(iface.name.bytes()) {
                    *d = b as libc::c_char;
                }
                let mut sa: libc::sockaddr_in6 = unsafe { std::mem::zeroed() };
                sa.sin6_len = std::mem::size_of::<libc::sockaddr_in6>() as u8;
                sa.sin6_family = libc::AF_INET6 as libc::sa_family_t;
                sa.sin6_addr.s6_addr = v6.octets();
                if v6.segments()[0] & 0xffc0 == 0xfe80 {
                    sa.sin6_scope_id = iface.index;
                }
                r.ifru.addr = sa;
                // SAFETY: the request the ioctl number names, sized to match.
                if unsafe { libc::ioctl(s, SIOCGIFAFLAG_IN6, &mut r) } < 0 {
                    continue;
                }
                let f = unsafe { r.ifru.flags6 };
                a.temporary = f & IN6_IFF_TEMPORARY != 0;
                a.deprecated = f & IN6_IFF_DEPRECATED != 0;
            }
        }
        // SAFETY: the socket opened above.
        unsafe { libc::close(s) };
    }
}

#[cfg(not(target_os = "macos"))]
mod v6flags {
    //! `/proc/net/if_inet6`, which the kernel writes world-readable: one line
    //! per IPv6 address with `(u8) ifa_flags` in its fifth field.

    use super::Iface;

    pub(super) fn fill(ifaces: &mut [Iface]) {
        let Ok(text) = std::fs::read_to_string("/proc/net/if_inet6") else { return };
        super::apply_if_inet6(ifaces, &text);
    }
}

/// Set the IPv6 flags from `/proc/net/if_inet6`'s text: `<32 hex> <ifindex>
/// <prefix> <scope> <flags> <name>`, all hex but the name. Linux's reader,
/// pure so it is tested on every platform; a line it cannot read is skipped.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn apply_if_inet6(ifaces: &mut [Iface], text: &str) {
    // `IFA_F_TEMPORARY` and `IFA_F_DEPRECATED` (`linux/if_addr.h`).
    const IFA_F_TEMPORARY: u8 = 0x01;
    const IFA_F_DEPRECATED: u8 = 0x20;
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() != 6 || f[0].len() != 32 {
            continue;
        }
        let Ok(bits) = u128::from_str_radix(f[0], 16) else { continue };
        let Ok(flags) = u8::from_str_radix(f[4], 16) else { continue };
        let ip = IpAddr::V6(Ipv6Addr::from(bits));
        let Some(iface) = ifaces.iter_mut().find(|i| i.name == f[5]) else { continue };
        for a in iface.addrs.iter_mut().filter(|a| a.ip == ip) {
            a.temporary = flags & IFA_F_TEMPORARY != 0;
            a.deprecated = flags & IFA_F_DEPRECATED != 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn a_netmask_becomes_its_prefix_length() {
        for (mask, want) in [
            (vec![255, 255, 255, 0], 24),
            (vec![255, 255, 255, 255], 32),
            (vec![0, 0, 0, 0], 0),
            (vec![255, 192, 0, 0], 10),
            (vec![255, 254, 0, 0], 15),
        ] {
            assert_eq!(prefix_of(&mask), want, "{mask:?}");
        }
        let mut m6 = [0u8; 16];
        m6[..8].fill(0xff);
        assert_eq!(prefix_of(&m6), 64);
    }

    #[test]
    fn the_network_clears_the_host_bits() {
        assert_eq!(network(ip("192.168.1.77"), 24), ip("192.168.1.0"));
        assert_eq!(network(ip("10.0.0.5"), 7), ip("10.0.0.0"));
        assert_eq!(network(ip("11.0.0.5"), 7), ip("10.0.0.0"));
        assert_eq!(network(ip("100.96.164.69"), 32), ip("100.96.164.69"));
        assert_eq!(network(ip("fd95:7329:9f61:1e61:1c04:d030:2ebd:e7d5"), 64), ip("fd95:7329:9f61:1e61::"));
        assert_eq!(network(ip("10.1.2.3"), 0), ip("0.0.0.0"));
        assert_eq!(network(ip("10.1.2.3"), 99), ip("10.1.2.3"));
    }

    #[test]
    fn a_kame_embedded_scope_is_cleared_from_a_link_local_address_only() {
        let e: Ipv6Addr = "fe80:e::1c04:1".parse().unwrap();
        assert_eq!(unembed_scope(e), "fe80::1c04:1".parse::<Ipv6Addr>().unwrap());
        let ula: Ipv6Addr = "fd00:e::1".parse().unwrap();
        assert_eq!(unembed_scope(ula), ula);
    }

    /// The fifth field is `(u8) ifa_flags`; the fixture is the spike's Linux
    /// read (`linux-fingerprint.sh`, 6.12): one deprecated address, one
    /// temporary, one permanent.
    #[test]
    fn proc_if_inet6_sets_the_temporary_and_deprecated_flags() {
        let addr = |s: &str| IfAddr { ip: ip(s), prefix: 64, temporary: false, deprecated: false };
        let mut ifs = vec![Iface {
            name: "eth0".into(),
            index: 11,
            up: true,
            running: true,
            loopback: false,
            point_to_point: false,
            addrs: vec![addr("fd00:77::20"), addr("fd00:77::744d:1:2:b068"), addr("fd00:77::10")],
        }];
        let text = "\
fd000077000000000000000000000020 0b 40 00 a0     eth0
fd00007700000000744d00010002b068 0b 40 00 01     eth0
fd000077000000000000000000000010 0b 40 00 80     eth0
garbage line
00000000000000000000000000000001 01 80 10 80       lo
";
        apply_if_inet6(&mut ifs, text);
        let a = &ifs[0].addrs;
        assert!(a[0].deprecated && !a[0].temporary, "{:?}", a[0]);
        assert!(a[1].temporary && !a[1].deprecated, "{:?}", a[1]);
        assert!(!a[2].temporary && !a[2].deprecated, "{:?}", a[2]);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_in6_ifreq_layout_matches_the_ioctl_number() {
        // _IOWR encodes the parameter's size in bits 16..29.
        let size = ((v6flags::SIOCGIFAFLAG_IN6 >> 16) & 0x1fff) as usize;
        assert_eq!(size, 288);
        assert_eq!(std::mem::size_of::<v6flags::In6Ifreq>(), size);
        assert_eq!(std::mem::offset_of!(v6flags::In6Ifreq, ifru), 16);
    }

    /// The live read, checked against an INDEPENDENT source on each OS: the
    /// names, indexes and flags `ifconfig -a` prints (macOS) or sysfs holds
    /// (Linux), and the loopback address every host has.
    #[test]
    fn the_live_read_agrees_with_the_os_s_own_listing() {
        let ifs = interfaces().expect("getifaddrs");
        let lo = ifs.iter().find(|i| i.loopback).expect("a loopback interface");
        assert!(lo.holds(ip("127.0.0.1")).is_some_and(|a| a.prefix == 8), "{lo:?}");
        for i in &ifs {
            assert_ne!(i.index, 0, "{} has no index", i.name);
        }
        let mut names: Vec<&str> = ifs.iter().map(|i| i.name.as_str()).collect();
        names.sort();
        let theirs = os_listing();
        let mut their_names: Vec<&str> = theirs.iter().map(|(n, ..)| n.as_str()).collect();
        their_names.sort();
        assert_eq!(names, their_names);
        for (name, index, ptp, loopback) in &theirs {
            let i = ifs.iter().find(|i| &i.name == name).unwrap();
            assert_eq!((i.point_to_point, i.loopback), (*ptp, *loopback), "{name}");
            if let Some(index) = index {
                assert_eq!(i.index, *index, "{name}");
            }
        }
    }

    /// `(name, index, point-to-point, loopback)` from `ifconfig -a`'s header
    /// lines (which carry no index).
    #[cfg(target_os = "macos")]
    fn os_listing() -> Vec<(String, Option<u32>, bool, bool)> {
        let o = crate::childenv::command("/sbin/ifconfig").arg("-a").output().expect("ifconfig");
        String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|l| !l.starts_with(char::is_whitespace) && l.contains(": flags="))
            .map(|l| {
                let (name, rest) = l.split_once(": flags=").unwrap();
                (name.to_string(), None, rest.contains("POINTOPOINT"), rest.contains("LOOPBACK"))
            })
            .collect()
    }

    /// The same from sysfs: `ifindex` and `flags` (IFF_LOOPBACK 0x8,
    /// IFF_POINTOPOINT 0x10) per interface under `/sys/class/net` — which
    /// also holds `bonding_masters`, a plain file, on a kernel with bonding
    /// (Docker Desktop's, measured), so an entry with no `ifindex` is skipped.
    #[cfg(not(target_os = "macos"))]
    fn os_listing() -> Vec<(String, Option<u32>, bool, bool)> {
        let mut v = Vec::new();
        for e in std::fs::read_dir("/sys/class/net").expect("sysfs") {
            let p = e.unwrap().path();
            if !p.join("ifindex").exists() {
                continue;
            }
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            let read = |f: &str| std::fs::read_to_string(p.join(f)).unwrap().trim().to_string();
            let index: u32 = read("ifindex").parse().unwrap();
            let flags = u32::from_str_radix(read("flags").trim_start_matches("0x"), 16).unwrap();
            v.push((name, Some(index), flags & 0x10 != 0, flags & 0x8 != 0));
        }
        v
    }
}
