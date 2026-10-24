//! Which addresses the remote listener may bind, and which a device may dial:
//! one closed allowlist, shared by serve and the shim so the host and the
//! device cannot disagree (Server Edition Phase 2, decision 7;
//! `docs/proposals/server-edition-phase2.md` §4.1).
//!
//! **"Private" is four classes and nothing else**: RFC 1918, CGNAT
//! `100.64.0.0/10`, link-local `169.254.0.0/16` and `fe80::/10`, ULA
//! `fc00::/7` ([`ALLOWED`]). The classifier asks only "which row is this
//! address in?", so a range nobody thought of is refused rather than admitted.
//! Every refusal carries the code `not-private` and a reason ([`Refused`]) —
//! the reason is for the message, never for the decision.
//!
//! **It decides on canonical bytes, never on a spelling.** The bytes come from
//! `std::net`'s parsers, which refuse `127.1`, `0x7f.0.0.1` and `010.0.0.1`
//! (since CVE-2021-29922, Rust 1.53), never from `inet_aton` or a resolver,
//! which take all three (measured, network spike 2026-10-21, rustc 1.98.1). An
//! IPv4 address written as IPv6 (mapped, compatible, NAT64, 6to4) is refused
//! rather than unwrapped, so each address has ONE spelling: bind the IPv4
//! address instead. [`embedded_v4`] decodes those forms for the message here,
//! and for the INVERSE rule a device's fetch needs
//! ([`crate::fetchpolicy::host_local`], which refuses everything but global
//! unicast and judges an embedded address as the IPv4 one it carries): one
//! decoder, so the two cannot read an address differently.
//!
//! The parsers:
//! - [`parse_bind`], serve's `--addr` with `--iface`: a zone is split off
//!   before parsing, only on a link-local IPv6 address, and must equal
//!   `--iface`; [`at_bind`] then checks the interface holds the address now.
//! - [`parse_dial`], the `addr=<address>:<port>` field of a pairing QR and of
//!   an address code: ANY `%` is refused, because `SocketAddr`'s parser takes
//!   a numeric zone (`[fe80::1%1]:7000` → scope id 1, the HOST's index, on the
//!   device; measured, spike). The port is split off and the host parsed with
//!   `IpAddr`, never `SocketAddr` or a resolver.
//! - [`parse_addr_code`], `worktrees-addr:v1?addr=…&pin=…`.
//!
//! A string a parser does not take is refused, never repaired.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::netif::{network, IfAddr, Iface};

/// The rows of §4.1's table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// RFC 1918: a home or office LAN.
    Rfc1918,
    /// RFC 6598 shared address space: Tailscale's node addresses, and also
    /// what some carriers, hotspots and satellite ISPs hand out — where "the
    /// LAN" includes the carrier's other customers (see [`Bound::shared_space`]).
    Cgnat,
    /// RFC 3927 / RFC 4291: a cable between two machines.
    LinkLocal,
    /// RFC 4193: Tailscale's `fd7a:115c:a1e0::/48`, or a LAN with ULA.
    UniqueLocal,
}

/// One allowlist row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub net: IpAddr,
    pub len: u8,
    pub class: Class,
}

const fn v4(a: u8, b: u8, len: u8, class: Class) -> Row {
    Row { net: IpAddr::V4(Ipv4Addr::new(a, b, 0, 0)), len, class }
}

const fn v6(first: u16, len: u8, class: Class) -> Row {
    Row { net: IpAddr::V6(Ipv6Addr::new(first, 0, 0, 0, 0, 0, 0, 0)), len, class }
}

/// The allowlist, and nothing else is private.
pub const ALLOWED: [Row; 7] = [
    v4(10, 0, 8, Class::Rfc1918),
    v4(172, 16, 12, Class::Rfc1918),
    v4(192, 168, 16, Class::Rfc1918),
    v4(100, 64, 10, Class::Cgnat),
    v4(169, 254, 16, Class::LinkLocal),
    v6(0xfc00, 7, Class::UniqueLocal),
    v6(0xfe80, 10, Class::LinkLocal),
];

/// The error code every refusal carries — never a bind error, so a refusal
/// cannot be mistaken for an address nothing could bind (§4.1).
pub const NOT_PRIVATE: &str = "not-private";

/// The row `ip` is in, if any.
pub fn row_of(ip: IpAddr) -> Option<&'static Row> {
    ALLOWED.iter().find(|r| r.net.is_ipv4() == ip.is_ipv4() && network(ip, r.len) == r.net)
}

/// An IPv4 address written in IPv6.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Embedding {
    /// `::ffff:a.b.c.d`.
    Mapped,
    /// `::a.b.c.d`, deprecated by RFC 4291 and still parsed. `::` and `::1`
    /// are the unspecified and loopback addresses, not this.
    Compatible,
    /// The NAT64 well-known prefix `64:ff9b::/96` (RFC 6052).
    Nat64,
    /// 6to4, `2002:aabb:ccdd::/48` (RFC 3056).
    SixToFour,
}

/// The IPv4 address an IPv6 address carries, and how. Teredo (`2001::/32`)
/// is not here: it obfuscates the address it carries, so a caller refuses it
/// whole rather than decode it.
///
/// The device-fetch rule ([`crate::fetchpolicy::host_local`]) decodes with
/// this too, after its own loopback/unspecified check: `::` and `::1` return
/// `None` here and are refused there by that check.
pub fn embedded_v4(v6: &Ipv6Addr) -> Option<(Embedding, Ipv4Addr)> {
    let s = v6.segments();
    let carried = |hi: u16, lo: u16| Ipv4Addr::new((hi >> 8) as u8, hi as u8, (lo >> 8) as u8, lo as u8);
    if let Some(m) = v6.to_ipv4_mapped() {
        return Some((Embedding::Mapped, m));
    }
    if s[..6].iter().all(|x| *x == 0) && !v6.is_unspecified() && !v6.is_loopback() {
        return Some((Embedding::Compatible, carried(s[6], s[7])));
    }
    if s[0] == 0x64 && s[1] == 0xff9b && s[2..6].iter().all(|x| *x == 0) {
        return Some((Embedding::Nat64, carried(s[6], s[7])));
    }
    if s[0] == 0x2002 {
        return Some((Embedding::SixToFour, carried(s[1], s[2])));
    }
    None
}

/// Why an address is not private. Decided AFTER the allowlist said no, for the
/// message only: a reason this enum lacks still ends in a refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// `127.0.0.0/8`, `::1`: binding it would merge the remote rule set into
    /// the loopback one (decision 6).
    Loopback,
    /// `0.0.0.0`, `::`: a wildcard answers on every interface.
    Unspecified,
    Multicast,
    /// `255.255.255.255`, or a subnet's directed broadcast ([`at_bind`]).
    Broadcast,
    /// `192.0.2.0/24`, `198.51.100.0/24`, `203.0.113.0/24`, `2001:db8::/32`.
    Documentation,
    /// `198.18.0.0/15`.
    Benchmarking,
    /// `0.0.0.0/8` ("this network"), `240.0.0.0/4`, `192.0.0.0/24`, Teredo.
    Reserved,
    /// The deprecated site-local `fec0::/10`.
    SiteLocal,
    /// An IPv4 address in IPv6 clothing: bind (or dial) the IPv4 one.
    Embedded(Embedding, Ipv4Addr),
    /// Global unicast, or anything else the allowlist does not name.
    Public,
}

/// A refusal: the address, and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotPrivate {
    pub ip: IpAddr,
    pub why: Refused,
}

impl fmt::Display for NotPrivate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ip = self.ip;
        let why = match self.why {
            Refused::Loopback => "loopback, which the loopback listener already serves".to_string(),
            Refused::Unspecified => "the wildcard, which answers on every interface".to_string(),
            Refused::Multicast => "multicast".to_string(),
            Refused::Broadcast => "a broadcast address".to_string(),
            Refused::Documentation => "a documentation address".to_string(),
            Refused::Benchmarking => "a benchmarking address".to_string(),
            Refused::Reserved => "reserved".to_string(),
            Refused::SiteLocal => "site-local, deprecated since RFC 3879".to_string(),
            Refused::Embedded(e, v4) => {
                let form = match e {
                    Embedding::Mapped => "IPv4-mapped",
                    Embedding::Compatible => "IPv4-compatible",
                    Embedding::Nat64 => "NAT64",
                    Embedding::SixToFour => "6to4",
                };
                format!("{form} IPv6 carrying {v4}; use the IPv4 address itself")
            }
            Refused::Public => "public".to_string(),
        };
        write!(
            f,
            "{NOT_PRIVATE}: {ip} is {why} — only RFC 1918, 100.64.0.0/10, link-local (169.254.0.0/16, fe80::/10) and fc00::/7 may be used"
        )
    }
}

/// The allowlist's verdict on `ip`.
pub fn classify(ip: IpAddr) -> Result<Class, NotPrivate> {
    match row_of(ip) {
        Some(r) => Ok(r.class),
        None => Err(NotPrivate { ip, why: refused(ip) }),
    }
}

fn refused(ip: IpAddr) -> Refused {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            if v.is_loopback() {
                Refused::Loopback
            } else if v.is_unspecified() {
                Refused::Unspecified
            } else if v.is_broadcast() {
                Refused::Broadcast
            } else if v.is_multicast() {
                Refused::Multicast
            } else if v.is_documentation() {
                Refused::Documentation
            } else if o[0] == 198 && o[1] & 0xfe == 18 {
                Refused::Benchmarking
            } else if o[0] == 0 || o[0] >= 240 || (o[0] == 192 && o[1] == 0 && o[2] == 0) {
                Refused::Reserved
            } else {
                Refused::Public
            }
        }
        IpAddr::V6(v) => {
            let s = v.segments();
            if v.is_loopback() {
                Refused::Loopback
            } else if v.is_unspecified() {
                Refused::Unspecified
            } else if let Some((e, v4)) = embedded_v4(&v) {
                Refused::Embedded(e, v4)
            } else if v.is_multicast() {
                Refused::Multicast
            } else if s[0] == 0x2001 && s[1] == 0x0db8 {
                Refused::Documentation
            } else if s[0] & 0xffc0 == 0xfec0 {
                Refused::SiteLocal
            } else if s[0] == 0x2001 && s[1] == 0 {
                Refused::Reserved
            } else {
                Refused::Public
            }
        }
    }
}

/// Why a string, or an address at bind, was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddrError {
    /// Not an address literal std's parser takes.
    Syntax(String),
    /// A zone where none may be, or one that does not match `--iface`.
    Zone(String),
    Port(String),
    NotPrivate(NotPrivate),
    /// The recorded subnet would reach past its allowlist row (§12.3 R3).
    SubnetTooWide { ip: IpAddr, prefix: u8, row: Row },
    /// The interface does not hold the address, is down, or is not there.
    NotHeld(String),
    /// A temporary or deprecated IPv6 address: it rotates within a day.
    Unstable(IpAddr),
    /// A malformed address code.
    Code(String),
}

impl AddrError {
    /// The machine-readable code: `not-private` for every classifier refusal,
    /// so a dialog and the audit log can tell it from a bind error.
    pub fn code(&self) -> &'static str {
        match self {
            AddrError::Syntax(_) => "bad-address",
            AddrError::Zone(_) => "bad-zone",
            AddrError::Port(_) => "bad-port",
            AddrError::NotPrivate(_) => NOT_PRIVATE,
            AddrError::SubnetTooWide { .. } => "subnet-too-wide",
            AddrError::NotHeld(_) => "not-held",
            AddrError::Unstable(_) => "unstable-address",
            AddrError::Code(_) => "bad-code",
        }
    }
}

impl fmt::Display for AddrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AddrError::NotPrivate(n) => fmt::Display::fmt(n, f),
            AddrError::SubnetTooWide { ip, prefix, row } => write!(
                f,
                "subnet-too-wide: {}/{prefix} reaches past {}/{}, so \"the same subnet\" could include addresses that are not private",
                network(*ip, *prefix),
                row.net,
                row.len
            ),
            AddrError::Unstable(ip) => write!(
                f,
                "unstable-address: {ip} is a temporary or deprecated IPv6 address and will change; use the interface's stable address"
            ),
            AddrError::Syntax(m) | AddrError::Zone(m) | AddrError::Port(m) | AddrError::NotHeld(m) | AddrError::Code(m) => {
                write!(f, "{}: {m}", self.code())
            }
        }
    }
}

impl From<NotPrivate> for AddrError {
    fn from(n: NotPrivate) -> Self {
        AddrError::NotPrivate(n)
    }
}

/// An address literal through std's parser, and nothing else.
fn literal(s: &str) -> Result<IpAddr, AddrError> {
    s.parse::<IpAddr>().map_err(|_| AddrError::Syntax(format!("{s:?} is not an IP address literal")))
}

fn is_v6_link_local(ip: IpAddr) -> bool {
    matches!(ip, IpAddr::V6(v) if v.segments()[0] & 0xffc0 == 0xfe80)
}

/// An interface name as `--iface` may give it: what the kernel would take,
/// and no `%`, so it can never smuggle a zone of its own.
fn iface_name(name: &str) -> Result<&str, AddrError> {
    let bad = name.is_empty()
        || name.len() >= 16
        || name.starts_with('-')
        || name.chars().any(|c| c == '%' || c == '/' || c == ':' || c.is_whitespace() || c.is_control());
    if bad {
        return Err(AddrError::Zone(format!("{name:?} is not an interface name")));
    }
    Ok(name)
}

/// What `--addr`/`--iface` asked for, before the interface is consulted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindAddr {
    pub ip: IpAddr,
    pub iface: String,
    pub class: Class,
}

/// Serve's `--addr <ip>[%zone] --iface <name>`. A zone is allowed only on a
/// link-local IPv6 address, where it means something, and must equal `--iface`
/// (§4.1); the interface's existence is [`at_bind`]'s question.
pub fn parse_bind(addr: &str, iface: &str) -> Result<BindAddr, AddrError> {
    let iface = iface_name(iface)?;
    let ip = match addr.split_once('%') {
        Some((a, zone)) => {
            let ip = literal(a)?;
            if !is_v6_link_local(ip) {
                return Err(AddrError::Zone(format!("{addr:?}: a zone belongs only on a link-local IPv6 address")));
            }
            if zone != iface {
                return Err(AddrError::Zone(format!("{addr:?}: the zone must name the --iface interface, {iface:?}")));
            }
            ip
        }
        None => literal(addr)?,
    };
    let class = classify(ip)?;
    Ok(BindAddr { ip, iface: iface.to_string(), class })
}

/// What the interface said at bind time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bound {
    pub ip: IpAddr,
    pub prefix: u8,
    pub iface: String,
    pub index: u32,
    pub class: Class,
    /// Overlay mode: the interface is point-to-point, so its fingerprint is
    /// the address itself, its peers must sit in the address's own row, and
    /// it is never advertised over mDNS.
    pub point_to_point: bool,
    /// A `100.64.0.0/10` address on an interface that is NOT point-to-point:
    /// possibly a carrier's or a hotspot's shared space. Allowed, with a
    /// warning that names the interface, and never advertised (§4.1, §11.1 N1).
    pub shared_space: bool,
}

/// The classifier again, at bind, against the interface as it is NOW (§4.1:
/// "at parse and at bind"): the interface exists, is up and running and holds
/// the address; an IPv6 address is not temporary or deprecated; its subnet
/// lies wholly inside the address's own row (§12.3 R3, so a later automatic
/// move inside "the same subnet" can never reach a public address); and an
/// IPv4 address is neither its subnet's network address nor its broadcast.
pub fn at_bind(want: &BindAddr, ifaces: &[Iface]) -> Result<Bound, AddrError> {
    let class = classify(want.ip)?;
    let iface = ifaces
        .iter()
        .find(|i| i.name == want.iface)
        .ok_or_else(|| AddrError::NotHeld(format!("there is no interface {:?}", want.iface)))?;
    if !iface.usable() {
        return Err(AddrError::NotHeld(format!("{} is not up", iface.name)));
    }
    let held = iface
        .holds(want.ip)
        .ok_or_else(|| AddrError::NotHeld(format!("{} does not hold {}", iface.name, want.ip)))?;
    check_held(held)?;
    Ok(Bound {
        ip: want.ip,
        prefix: held.prefix,
        iface: iface.name.clone(),
        index: iface.index,
        class,
        point_to_point: iface.point_to_point,
        shared_space: class == Class::Cgnat && !iface.point_to_point,
    })
}

/// The per-address half of [`at_bind`], which an automatic move re-runs on
/// the new address (§4.1: "re-running the classifier on it").
pub fn check_held(a: &IfAddr) -> Result<Class, AddrError> {
    let class = classify(a.ip)?;
    if a.temporary || a.deprecated {
        return Err(AddrError::Unstable(a.ip));
    }
    subnet_row(a.ip, a.prefix)?;
    if let IpAddr::V4(v) = a.ip {
        if a.prefix <= 30 {
            let host_bits = u32::MAX >> a.prefix;
            let h = u32::from(v) & host_bits;
            if h == 0 || h == host_bits {
                return Err(NotPrivate { ip: a.ip, why: Refused::Broadcast }.into());
            }
        }
    }
    Ok(class)
}

/// §12.3 R3: `ip/prefix` lies wholly inside one allowlist row.
pub fn subnet_row(ip: IpAddr, prefix: u8) -> Result<&'static Row, AddrError> {
    let width = if ip.is_ipv4() { 32 } else { 128 };
    if prefix > width {
        return Err(AddrError::Syntax(format!("{ip}/{prefix}: the prefix is longer than the address")));
    }
    let row = row_of(ip).ok_or(NotPrivate { ip, why: refused(ip) })?;
    if prefix < row.len {
        return Err(AddrError::SubnetTooWide { ip, prefix, row: *row });
    }
    Ok(row)
}

/// The `addr=` field of a pairing QR or an address code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DialAddr {
    pub ip: IpAddr,
    pub port: u16,
    pub class: Class,
}

impl DialAddr {
    /// A link-local address means nothing without an interface, and the
    /// host's zone means nothing on the device: the device scopes the dial to
    /// an interface it picks itself ([`linklocal_candidates`]).
    pub fn needs_scope(&self) -> bool {
        self.class == Class::LinkLocal
    }
}

/// `<ipv4>:<port>` or `[<ipv6>]:<port>`, raw — never percent-encoded, so any
/// `%` is a zone and refused, whatever follows it.
pub fn parse_dial(s: &str) -> Result<DialAddr, AddrError> {
    if s.contains('%') {
        return Err(AddrError::Zone(format!(
            "{s:?}: a code carries no zone; the device chooses its own interface"
        )));
    }
    let (ip, port) = if let Some(rest) = s.strip_prefix('[') {
        let (host, after) = rest
            .split_once(']')
            .ok_or_else(|| AddrError::Syntax(format!("{s:?}: an unclosed '['")))?;
        let port = after.strip_prefix(':').ok_or_else(|| AddrError::Port(format!("{s:?}: no port")))?;
        let ip = host
            .parse::<Ipv6Addr>()
            .map(IpAddr::V6)
            .map_err(|_| AddrError::Syntax(format!("{host:?} is not an IPv6 address literal")))?;
        (ip, port)
    } else {
        let (host, port) = s.rsplit_once(':').ok_or_else(|| AddrError::Port(format!("{s:?}: no port")))?;
        let ip = host
            .parse::<Ipv4Addr>()
            .map(IpAddr::V4)
            .map_err(|_| AddrError::Syntax(format!("{host:?} is not an IPv4 address literal (IPv6 goes in brackets)")))?;
        (ip, port)
    };
    let port = port_number(port)?;
    let class = classify(ip)?;
    Ok(DialAddr { ip, port, class })
}

/// 1–65535 in plain decimal: no sign (`u16::from_str` takes a `+`), no
/// leading zero, nothing after it.
fn port_number(p: &str) -> Result<u16, AddrError> {
    let ok = !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()) && !p.starts_with('0');
    match p.parse::<u16>() {
        Ok(n) if ok => Ok(n),
        _ => Err(AddrError::Port(format!("{p:?} is not a port number"))),
    }
}

/// An address code: where a host this device already holds the pin of has
/// moved to (§4.1). It is NOT authenticated — the pin is public — so a device
/// adopts the address only after a pinned handshake to it succeeds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddrCode {
    pub addr: DialAddr,
    /// The SHA-256 of the host's SPKI.
    pub pin: [u8; 32],
}

pub const ADDR_CODE_PREFIX: &str = "worktrees-addr:v1?";

/// `worktrees-addr:v1?addr=<address>:<port>&pin=<spki sha256>`: exactly those
/// two keys, once each, in either order, nothing else. The pin is the
/// pairing link's spelling, `pairuri::is_pin` (64 lowercase hex digits), so
/// the link and the address code cannot spell a pin differently.
pub fn parse_addr_code(s: &str) -> Result<AddrCode, AddrError> {
    let query = s
        .strip_prefix(ADDR_CODE_PREFIX)
        .ok_or_else(|| AddrError::Code(format!("an address code starts with {ADDR_CODE_PREFIX:?}")))?;
    let (mut addr, mut pin) = (None, None);
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').ok_or_else(|| AddrError::Code(format!("{pair:?} is not key=value")))?;
        let slot = match k {
            "addr" => &mut addr,
            "pin" => &mut pin,
            _ => return Err(AddrError::Code(format!("unknown key {k:?}"))),
        };
        if slot.replace(v).is_some() {
            return Err(AddrError::Code(format!("{k:?} given twice")));
        }
    }
    let addr = parse_dial(addr.ok_or_else(|| AddrError::Code("no addr".into()))?)?;
    let pin = pin.ok_or_else(|| AddrError::Code("no pin".into()))?;
    if !crate::pairuri::is_pin(pin) {
        return Err(AddrError::Code("the pin is 64 lowercase hex digits".into()));
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&pin[2 * i..2 * i + 2], 16).map_err(|_| AddrError::Code("bad pin".into()))?;
    }
    Ok(AddrCode { addr, pin: out })
}

/// The address code the HOST shows after its bind moves (§4.1): exactly what
/// [`parse_addr_code`] reads back, built here beside it so the two cannot
/// drift. An address the classifier refuses is refused here too, and the
/// address carries no zone, ever.
pub fn addr_code(ip: IpAddr, port: u16, pin: &str) -> Result<String, AddrError> {
    classify(ip)?;
    if pin.len() != 64 || !pin.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err(AddrError::Code("the pin is 64 lowercase hex digits".into()));
    }
    let addr = match ip {
        IpAddr::V4(v) => format!("{v}:{port}"),
        IpAddr::V6(v) => format!("[{v}]:{port}"),
    };
    Ok(format!("{ADDR_CODE_PREFIX}addr={addr}&pin={pin}"))
}

/// How many interfaces a device tries for one link-local dial (§4.1).
pub const LINKLOCAL_TRIES: usize = 4;

/// Interface names that hold a link-local address and carry no service:
/// Apple Wireless Direct Link, low-latency WLAN and Wi-Fi Aware on macOS
/// (seen up with `fe80::` on the spike's Mac). Every `utun` is point-to-point
/// and goes out on that flag instead, Tailscale's included: an overlay does
/// not carry link-local traffic.
const NO_SERVICE: [&str; 3] = ["awdl", "llw", "nan"];

/// The interfaces a device may scope a dial to `ip` through, in the OS's
/// order, at most [`LINKLOCAL_TRIES`]: up and running, neither loopback nor
/// point-to-point, not one of [`NO_SERVICE`], and holding a link-local address
/// of the dial's family. With two links that both carry `169.254/16`, an
/// unscoped dial follows the routing table to the wrong one (measured, spike),
/// so IPv4 link-local goes through here too.
pub fn linklocal_candidates(ifaces: &[Iface], ip: IpAddr) -> Vec<&Iface> {
    ifaces
        .iter()
        .filter(|i| i.usable() && !i.loopback && !i.point_to_point)
        .filter(|i| !NO_SERVICE.iter().any(|p| i.name.strip_prefix(p).is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))))
        .filter(|i| i.addrs.iter().any(|a| a.ip.is_ipv4() == ip.is_ipv4() && row_of(a.ip).is_some_and(|r| r.class == Class::LinkLocal)))
        .take(LINKLOCAL_TRIES)
        .collect()
}

impl Class {
    /// The class's name on the wire and in the audit log.
    pub fn name(self) -> &'static str {
        match self {
            Class::Rfc1918 => "rfc1918",
            Class::Cgnat => "cgnat",
            Class::LinkLocal => "link-local",
            Class::UniqueLocal => "ula",
        }
    }
}

/// One address the host may turn remote access on at: what the enable
/// dialog offers (§4.1, "the dialog offers the interface's stable address").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub ip: IpAddr,
    pub prefix: u8,
    pub iface: String,
    pub class: Class,
    /// Overlay mode (a tunnel); LAN mode otherwise.
    pub point_to_point: bool,
    /// `100.64.0.0/10` on an interface that is not a tunnel ([`Bound::shared_space`]).
    pub shared_space: bool,
}

/// Every address a bind would take now, in the order a person should meet
/// them: a LAN address first, then shared space (Tailscale), unique-local and
/// link-local last, IPv4 before IPv6 inside a class, the OS's order otherwise.
/// [`at_bind`]'s rules decide, never a second copy of them: up and running,
/// not loopback, and [`check_held`] (the classifier, a stable address, a
/// subnet inside its row, not a network or broadcast address). Link-local is
/// left out where nothing could dial it: on a tunnel, which carries no
/// link-local traffic, and on the macOS interfaces with no service.
pub fn candidates(ifaces: &[Iface]) -> Vec<Candidate> {
    let no_service = |name: &str| NO_SERVICE.iter().any(|p| name.strip_prefix(p).is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit())));
    let mut out: Vec<Candidate> = ifaces
        .iter()
        .filter(|i| i.usable() && !i.loopback)
        .flat_map(|i| {
            i.addrs.iter().filter_map(move |a| {
                let class = check_held(a).ok()?;
                if class == Class::LinkLocal && (i.point_to_point || no_service(&i.name)) {
                    return None;
                }
                Some(Candidate {
                    ip: a.ip,
                    prefix: a.prefix,
                    iface: i.name.clone(),
                    class,
                    point_to_point: i.point_to_point,
                    shared_space: class == Class::Cgnat && !i.point_to_point,
                })
            })
        })
        .collect();
    let rank = |c: &Candidate| {
        let class = match c.class {
            Class::Rfc1918 => 0,
            Class::Cgnat => 1,
            Class::UniqueLocal => 2,
            Class::LinkLocal => 3,
        };
        (class, c.ip.is_ipv6())
    };
    out.sort_by_key(rank);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    /// §4.6 step 1: the first and last address of each allowed range, and the
    /// neighbour just outside it on both sides.
    #[test]
    fn each_row_admits_its_first_and_last_address_and_refuses_its_neighbours() {
        use Class::*;
        let table: &[(&str, Option<Class>)] = &[
            ("9.255.255.255", None),
            ("10.0.0.0", Some(Rfc1918)),
            ("10.255.255.255", Some(Rfc1918)),
            ("11.0.0.0", None),
            ("172.15.255.255", None),
            ("172.16.0.0", Some(Rfc1918)),
            ("172.31.255.255", Some(Rfc1918)),
            ("172.32.0.0", None),
            ("192.167.255.255", None),
            ("192.168.0.0", Some(Rfc1918)),
            ("192.168.255.255", Some(Rfc1918)),
            ("192.169.0.0", None),
            ("100.63.255.255", None),
            ("100.64.0.0", Some(Cgnat)),
            ("100.127.255.255", Some(Cgnat)),
            ("100.128.0.0", None),
            ("169.253.255.255", None),
            ("169.254.0.0", Some(LinkLocal)),
            ("169.254.255.255", Some(LinkLocal)),
            ("169.255.0.0", None),
            ("fbff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", None),
            ("fc00::", Some(UniqueLocal)),
            ("fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", Some(UniqueLocal)),
            ("fe00::", None),
            ("fe7f:ffff:ffff:ffff:ffff:ffff:ffff:ffff", None),
            ("fe80::", Some(LinkLocal)),
            ("febf:ffff:ffff:ffff:ffff:ffff:ffff:ffff", Some(LinkLocal)),
            ("fec0::", None),
        ];
        for (s, want) in table {
            assert_eq!(classify(ip(s)).ok(), *want, "{s}");
        }
    }

    /// Every refused kind in §4.1, each with the reason its message gives.
    #[test]
    fn every_refused_kind_is_not_private_with_its_own_reason() {
        use Embedding::*;
        use Refused::*;
        let table: &[(&str, Refused)] = &[
            ("127.0.0.1", Loopback),
            ("127.255.255.254", Loopback),
            ("::1", Loopback),
            ("0.0.0.0", Unspecified),
            ("::", Unspecified),
            ("224.0.0.251", Multicast),
            ("239.255.255.255", Multicast),
            ("ff02::fb", Multicast),
            ("255.255.255.255", Broadcast),
            ("192.0.2.1", Documentation),
            ("198.51.100.7", Documentation),
            ("203.0.113.9", Documentation),
            ("2001:db8::1", Documentation),
            ("198.18.0.1", Benchmarking),
            ("198.19.255.255", Benchmarking),
            ("0.1.2.3", Reserved),
            ("240.0.0.1", Reserved),
            ("192.0.0.8", Reserved),
            ("2001::1", Reserved),
            ("fec0::1", SiteLocal),
            ("::ffff:10.0.0.1", Embedded(Mapped, Ipv4Addr::new(10, 0, 0, 1))),
            ("::ffff:127.0.0.1", Embedded(Mapped, Ipv4Addr::new(127, 0, 0, 1))),
            ("::ffff:a00:1", Embedded(Mapped, Ipv4Addr::new(10, 0, 0, 1))),
            ("::10.0.0.1", Embedded(Compatible, Ipv4Addr::new(10, 0, 0, 1))),
            ("::192.168.1.5", Embedded(Compatible, Ipv4Addr::new(192, 168, 1, 5))),
            ("64:ff9b::a00:1", Embedded(Nat64, Ipv4Addr::new(10, 0, 0, 1))),
            ("64:ff9b::7f00:1", Embedded(Nat64, Ipv4Addr::new(127, 0, 0, 1))),
            ("2002:c0a8:105::1", Embedded(SixToFour, Ipv4Addr::new(192, 168, 1, 5))),
            ("8.8.8.8", Public),
            ("1.1.1.1", Public),
            ("172.32.0.1", Public),
            ("2606:4700::1111", Public),
            ("64:ff9b:1::a00:1", Public),
        ];
        for (s, why) in table {
            let e = classify(ip(s)).expect_err(s);
            assert_eq!(e.why, *why, "{s}");
            assert!(e.to_string().starts_with("not-private: "), "{e}");
        }
        let m = classify(ip("::ffff:192.168.1.5")).unwrap_err().to_string();
        assert!(m.contains("192.168.1.5") && m.contains("IPv4-mapped"), "{m}");
    }

    /// The decoder the device-fetch rule shares: each form, and the two
    /// all-zero-prefix addresses that are NOT the compatible form.
    #[test]
    fn embedded_v4_decodes_the_four_forms_and_nothing_else() {
        let v6 = |s: &str| s.parse::<Ipv6Addr>().unwrap();
        let v4 = Ipv4Addr::new(192, 168, 1, 5);
        assert_eq!(embedded_v4(&v6("::ffff:192.168.1.5")), Some((Embedding::Mapped, v4)));
        assert_eq!(embedded_v4(&v6("::192.168.1.5")), Some((Embedding::Compatible, v4)));
        assert_eq!(embedded_v4(&v6("64:ff9b::192.168.1.5")), Some((Embedding::Nat64, v4)));
        assert_eq!(embedded_v4(&v6("2002:c0a8:105:1::1")), Some((Embedding::SixToFour, v4)));
        assert_eq!(embedded_v4(&v6("::")), None);
        assert_eq!(embedded_v4(&v6("::1")), None);
        assert_eq!(embedded_v4(&v6("::0.0.0.2")), Some((Embedding::Compatible, Ipv4Addr::new(0, 0, 0, 2))));
        assert_eq!(embedded_v4(&v6("2001::c0a8:105")), None, "Teredo is not decoded");
        assert_eq!(embedded_v4(&v6("fd7a:115c:a1e0::1")), None);
        assert_eq!(embedded_v4(&v6("64:ff9b:0:0:1::c0a8:105")), None, "outside the /96");
    }

    /// The spellings std refuses and a resolver would take (spike §5.1), and
    /// the zone rules: refused, never repaired.
    #[test]
    fn the_bind_parser_refuses_odd_spellings_and_misplaced_zones() {
        for s in ["127.1", "0x7f.0.0.1", "010.0.0.1", "10.0.0.256", " 10.0.0.1", "10.0.0.1 ", "", "10.0.0.1/24", "[fd00::1]", "10.0.0.01"] {
            assert_eq!(parse_bind(s, "en0").map_err(|e| e.code()), Err("bad-address"), "{s:?}");
        }
        let zone = |a: &str, i: &str| parse_bind(a, i).map_err(|e| e.code());
        assert_eq!(zone("fe80::1%en1", "en0"), Err("bad-zone"), "a zone naming another interface");
        assert_eq!(zone("fe80::1%1", "en0"), Err("bad-zone"), "a numeric zone");
        assert_eq!(zone("fe80::1%", "en0"), Err("bad-zone"), "an empty zone");
        assert_eq!(zone("fe80::1%en0%en0", "en0"), Err("bad-zone"));
        assert_eq!(zone("192.168.1.5%en0", "en0"), Err("bad-zone"), "IPv4 has no zone");
        assert_eq!(zone("fd00::1%en0", "en0"), Err("bad-zone"), "only link-local takes one");
        assert_eq!(zone("192.168.1.5", ""), Err("bad-zone"), "--iface is required");
        assert_eq!(zone("192.168.1.5", "en0%1"), Err("bad-zone"));
        assert_eq!(zone("192.168.1.5", "-x"), Err("bad-zone"));
        assert_eq!(zone("8.8.8.8", "en0"), Err("not-private"));
        assert_eq!(zone("::ffff:192.168.1.5", "en0"), Err("not-private"));
        assert_eq!(zone("0.0.0.0", "en0"), Err("not-private"));
        assert_eq!(zone("127.0.0.1", "lo0"), Err("not-private"));
        let ok = parse_bind("fe80::1c04:1%en0", "en0").unwrap();
        assert_eq!((ok.ip, ok.class), (ip("fe80::1c04:1"), Class::LinkLocal));
        let ok = parse_bind("fe80::1c04:1", "en0").unwrap();
        assert_eq!(ok.iface, "en0", "a link-local bind without a zone takes --iface's");
        assert_eq!(parse_bind("192.168.1.5", "en0").unwrap().class, Class::Rfc1918);
    }

    #[test]
    fn the_dial_parser_takes_a_host_and_a_port_and_refuses_any_zone() {
        let ok = |s: &str| parse_dial(s).unwrap();
        assert_eq!(ok("192.168.1.5:7443"), DialAddr { ip: ip("192.168.1.5"), port: 7443, class: Class::Rfc1918 });
        assert_eq!(ok("[fd7a:115c:a1e0::1]:7443").class, Class::UniqueLocal);
        assert!(ok("[fe80::1]:7000").needs_scope());
        assert!(ok("169.254.10.1:7000").needs_scope());
        assert!(!ok("100.96.1.1:1").needs_scope());
        assert_eq!(ok("10.0.0.1:65535").port, 65535);
        let code = |s: &str| parse_dial(s).map_err(|e| e.code());
        for s in ["[fe80::1%1]:7000", "[fe80::1%25en0]:7000", "[fe80::1%en0]:7000", "10.0.0.1%en0:80", "[fe80::1]:70%30"] {
            assert_eq!(code(s), Err("bad-zone"), "{s}");
        }
        for s in ["10.0.0.1", "10.0.0.1:", "10.0.0.1:+80", "10.0.0.1:080", "10.0.0.1:0", "10.0.0.1:65536", "10.0.0.1:80 ", "[fd00::1]", "[fd00::1]80", "10.0.0.1:123456"] {
            assert_eq!(code(s), Err("bad-port"), "{s}");
        }
        for s in ["127.1:80", "010.0.0.1:80", "fd00::1:80", "[10.0.0.1]:80", "[fd00::1:80", "host.local:80", ":80", " 10.0.0.1:80"] {
            assert_eq!(code(s), Err("bad-address"), "{s}");
        }
        for s in ["8.8.8.8:443", "127.0.0.1:7443", "[::1]:7443", "[::ffff:10.0.0.1]:80", "0.0.0.0:80", "[2001:db8::1]:80"] {
            assert_eq!(code(s), Err("not-private"), "{s}");
        }
    }

    #[test]
    fn an_address_code_has_exactly_its_two_keys() {
        let pin = "ab".repeat(32);
        let c = parse_addr_code(&format!("worktrees-addr:v1?addr=192.168.1.5:7443&pin={pin}")).unwrap();
        assert_eq!(c.addr.ip, ip("192.168.1.5"));
        assert_eq!(c.pin, [0xab; 32]);
        let swapped = parse_addr_code(&format!("worktrees-addr:v1?pin={pin}&addr=[fd00::5]:7443")).unwrap();
        assert_eq!(swapped.addr.port, 7443);
        let code = |s: String| parse_addr_code(&s).map_err(|e| e.code());
        assert_eq!(code(format!("worktrees-addr:v2?addr=10.0.0.1:1&pin={pin}")), Err("bad-code"));
        assert_eq!(code(format!("WORKTREES-ADDR:v1?addr=10.0.0.1:1&pin={pin}")), Err("bad-code"));
        assert_eq!(code(format!("worktrees-addr:v1?addr=10.0.0.1:1&pin={pin}&x=1")), Err("bad-code"));
        assert_eq!(code(format!("worktrees-addr:v1?addr=10.0.0.1:1&addr=10.0.0.2:1&pin={pin}")), Err("bad-code"));
        assert_eq!(code("worktrees-addr:v1?addr=10.0.0.1:1".into()), Err("bad-code"));
        assert_eq!(code(format!("worktrees-addr:v1?pin={pin}")), Err("bad-code"));
        assert_eq!(code(format!("worktrees-addr:v1?addr=10.0.0.1:1&pin={}", "AB".repeat(32))), Err("bad-code"));
        assert_eq!(code(format!("worktrees-addr:v1?addr=10.0.0.1:1&pin={}", "ab".repeat(31))), Err("bad-code"));
        assert_eq!(code(format!("worktrees-addr:v1?addr=10.0.0.1:1&pin={pin}&")), Err("bad-code"));
        assert_eq!(code(format!("worktrees-addr:v1?addr=[fe80::1%251]:1&pin={pin}")), Err("bad-zone"));
        assert_eq!(code(format!("worktrees-addr:v1?addr=8.8.8.8:1&pin={pin}")), Err("not-private"));
    }

    /// The host builds what a device parses: the same two keys, IPv6 in
    /// brackets, never a zone — and nothing the parser would refuse.
    #[test]
    fn an_address_code_the_host_builds_is_one_the_device_reads() {
        let pin = "ab".repeat(32);
        let c = addr_code(ip("192.168.1.5"), 7443, &pin).unwrap();
        assert_eq!(c, format!("worktrees-addr:v1?addr=192.168.1.5:7443&pin={pin}"));
        assert_eq!(parse_addr_code(&c).unwrap().addr.port, 7443);
        let v6 = addr_code(ip("fe80::1"), 7443, &pin).unwrap();
        assert_eq!(v6, format!("worktrees-addr:v1?addr=[fe80::1]:7443&pin={pin}"));
        assert_eq!(parse_addr_code(&v6).unwrap().addr.ip, ip("fe80::1"));
        assert_eq!(addr_code(ip("8.8.8.8"), 7443, &pin).map_err(|e| e.code()), Err("not-private"));
        assert_eq!(addr_code(ip("10.0.0.1"), 7443, "AB").map_err(|e| e.code()), Err("bad-code"));
    }

    fn iface(name: &str, ptp: bool, addrs: &[(&str, u8)]) -> Iface {
        Iface {
            name: name.into(),
            index: 7,
            up: true,
            running: true,
            loopback: name.starts_with("lo"),
            point_to_point: ptp,
            addrs: addrs.iter().map(|(a, p)| IfAddr { ip: ip(a), prefix: *p, temporary: false, deprecated: false }).collect(),
        }
    }

    #[test]
    fn the_enable_dialog_offers_only_what_a_bind_would_take() {
        let mut en0 = iface("en0", false, &[("fe80::1", 64), ("fd00::5", 64), ("2001:db8::5", 64), ("192.168.1.5", 24)]);
        en0.addrs.push(IfAddr { ip: ip("fd00::77"), prefix: 64, temporary: true, deprecated: false });
        en0.addrs.push(IfAddr { ip: ip("fd00::78"), prefix: 64, temporary: false, deprecated: true });
        let mut en3 = iface("en3", false, &[("192.168.9.2", 24)]);
        en3.running = false;
        let ifs = [
            iface("lo0", false, &[("127.0.0.1", 8), ("fe80::1", 64)]),
            en0,
            iface("utun4", true, &[("fe80::2", 64), ("100.101.102.103", 32), ("fd7a:115c:a1e0::1", 128)]),
            iface("awdl0", false, &[("fe80::3", 64)]),
            iface("en7", false, &[("100.70.0.9", 16), ("169.254.7.7", 16)]),
            iface("en9", false, &[("10.0.0.0", 24), ("10.1.0.5", 7)]),
            en3,
            iface("en5", false, &[("8.8.8.8", 24)]),
        ];
        let c = candidates(&ifs);
        let got: Vec<(String, &str)> = c.iter().map(|c| (c.ip.to_string(), c.iface.as_str())).collect();
        let want = [
            ("192.168.1.5", "en0"),
            ("100.101.102.103", "utun4"),
            ("100.70.0.9", "en7"),
            ("fd00::5", "en0"),
            ("fd7a:115c:a1e0::1", "utun4"),
            ("169.254.7.7", "en7"),
            ("fe80::1", "en0"),
        ];
        assert_eq!(got, want.map(|(a, i)| (a.to_string(), i)));
        let by = |a: &str| c.iter().find(|c| c.ip == ip(a)).unwrap();
        assert_eq!((by("100.101.102.103").point_to_point, by("100.101.102.103").shared_space), (true, false));
        assert_eq!((by("100.70.0.9").point_to_point, by("100.70.0.9").shared_space), (false, true));
        assert_eq!((by("192.168.1.5").class, by("192.168.1.5").prefix), (Class::Rfc1918, 24));
        // Each one is an address `at_bind` takes, on the interface it names.
        for c in &c {
            let b = at_bind(&parse_bind(&c.ip.to_string(), &c.iface).unwrap(), &ifs).unwrap();
            assert_eq!((b.class, b.point_to_point, b.shared_space), (c.class, c.point_to_point, c.shared_space), "{}", c.ip);
        }
    }

    #[test]
    fn a_subnet_must_lie_inside_its_own_row() {
        assert_eq!(subnet_row(ip("10.0.0.5"), 8).unwrap().class, Class::Rfc1918);
        assert_eq!(subnet_row(ip("10.0.0.5"), 7).map_err(|e| e.code()), Err("subnet-too-wide"));
        assert_eq!(subnet_row(ip("172.16.4.5"), 11).map_err(|e| e.code()), Err("subnet-too-wide"));
        assert_eq!(subnet_row(ip("100.96.1.1"), 9).map_err(|e| e.code()), Err("subnet-too-wide"));
        assert!(subnet_row(ip("100.96.1.1"), 32).is_ok());
        assert_eq!(subnet_row(ip("fd7a:115c:a1e0::1"), 6).map_err(|e| e.code()), Err("subnet-too-wide"));
        assert!(subnet_row(ip("fd7a:115c:a1e0::1"), 48).is_ok());
        assert_eq!(subnet_row(ip("10.0.0.5"), 33).map_err(|e| e.code()), Err("bad-address"));
        let e = subnet_row(ip("10.0.0.5"), 7).unwrap_err().to_string();
        assert!(e.contains("10.0.0.0/7") && e.contains("10.0.0.0/8"), "{e}");
    }

    #[test]
    fn at_bind_needs_the_interface_to_hold_the_address_now() {
        let en0 = iface("en0", false, &[("192.168.1.5", 24), ("fd00::5", 64)]);
        let want = |a: &str, i: &str| parse_bind(a, i).unwrap();
        let b = at_bind(&want("192.168.1.5", "en0"), std::slice::from_ref(&en0)).unwrap();
        assert_eq!((b.prefix, b.index, b.point_to_point, b.shared_space), (24, 7, false, false));
        let code = |a: &str, i: &str, ifs: &[Iface]| at_bind(&want(a, i), ifs).map_err(|e| e.code());
        assert_eq!(code("192.168.1.6", "en0", std::slice::from_ref(&en0)), Err("not-held"));
        assert_eq!(code("192.168.1.5", "en1", std::slice::from_ref(&en0)), Err("not-held"));
        let mut down = en0.clone();
        down.running = false;
        assert_eq!(code("192.168.1.5", "en0", &[down]), Err("not-held"), "Linux keeps a down interface's addresses");
        let mut temp = en0.clone();
        temp.addrs[1].temporary = true;
        assert_eq!(code("fd00::5", "en0", std::slice::from_ref(&temp)), Err("unstable-address"));
        temp.addrs[1].temporary = false;
        temp.addrs[1].deprecated = true;
        assert_eq!(code("fd00::5", "en0", &[temp]), Err("unstable-address"));
        let wide = iface("en0", false, &[("10.0.0.5", 7)]);
        assert_eq!(code("10.0.0.5", "en0", &[wide]), Err("subnet-too-wide"));
        let edges = iface("en0", false, &[("192.168.1.0", 24), ("192.168.1.255", 24), ("10.9.0.0", 31), ("10.9.0.7", 32)]);
        assert_eq!(code("192.168.1.0", "en0", std::slice::from_ref(&edges)), Err("not-private"), "the network address");
        assert_eq!(code("192.168.1.255", "en0", std::slice::from_ref(&edges)), Err("not-private"), "the broadcast");
        assert!(at_bind(&want("10.9.0.0", "en0"), std::slice::from_ref(&edges)).is_ok(), "a /31 has no broadcast (RFC 3021)");
        assert!(at_bind(&want("10.9.0.7", "en0"), &[edges]).is_ok());
    }

    #[test]
    fn cgnat_off_a_point_to_point_interface_is_shared_space() {
        let ts = iface("utun6", true, &[("100.96.164.69", 32)]);
        let lan = iface("en0", false, &[("100.70.0.9", 16)]);
        let b = at_bind(&parse_bind("100.96.164.69", "utun6").unwrap(), &[ts]).unwrap();
        assert!(b.point_to_point && !b.shared_space);
        let b = at_bind(&parse_bind("100.70.0.9", "en0").unwrap(), &[lan]).unwrap();
        assert!(!b.point_to_point && b.shared_space);
    }

    /// The interfaces on the spike's Mac (`ifconfig -a`, 2026-10-22): only
    /// en0 carries a service. awdl0/llw0 are up with `fe80::`, nan0 is down,
    /// every utun is point-to-point, lo0 is loopback.
    #[test]
    fn a_link_local_dial_skips_interfaces_that_carry_no_service() {
        let mut nan0 = iface("nan0", false, &[("fe80::4", 64)]);
        nan0.up = false;
        let ifs = vec![
            iface("lo0", false, &[("127.0.0.1", 8), ("fe80::1", 64)]),
            iface("en0", false, &[("192.168.0.40", 24), ("fe80::1c04:1", 64)]),
            iface("awdl0", false, &[("fe80::2", 64)]),
            iface("llw0", false, &[("fe80::3", 64)]),
            nan0,
            iface("utun0", true, &[("fe80::5", 64)]),
            iface("utun6", true, &[("100.96.164.69", 32), ("fe80::6", 64)]),
            iface("bridge0", false, &[("169.254.7.7", 16)]),
            iface("en7", false, &[("fe80::7", 64)]),
            iface("awdlx", false, &[("fe80::8", 64)]),
        ];
        let names = |v: Vec<&Iface>| v.iter().map(|i| i.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(linklocal_candidates(&ifs, ip("fe80::99"))), ["en0", "en7", "awdlx"]);
        assert_eq!(names(linklocal_candidates(&ifs, ip("169.254.1.1"))), ["bridge0"]);
        let many: Vec<Iface> = (0..6).map(|n| iface(&format!("en{n}"), false, &[("fe80::1", 64)])).collect();
        assert_eq!(linklocal_candidates(&many, ip("fe80::99")).len(), LINKLOCAL_TRIES);
    }
}
