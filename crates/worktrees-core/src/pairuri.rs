//! The pairing link a QR carries (Server Edition Phase 2 §2.2 step 2, §2.3):
//!
//! ```text
//! worktrees-pair:v1?addr=<address>:<port>&pin=<spki sha256>&secret=<128-bit>&host=<name>&exp=<epoch>
//! ```
//!
//! Built by serve when the host opens a pairing window; parsed by a DEVICE
//! before it dials anything, which is why it lives here, in core, where the
//! app's shim reaches it too: the host and the device read one format with
//! one parser. A QR is untrusted input — a public address in one is a
//! phishing attempt — so the parser is strict: exactly these five
//! parameters, each once; nothing unknown; an `exp` already past refused; a
//! value it cannot read refused, never repaired.
//!
//! Spellings, which §2.2 leaves open and this module fixes: `pin` is the
//! SHA-256 of the server's SubjectPublicKeyInfo in lowercase hex (64), the
//! spelling `devices.json` and the audit log use; `secret` is 128 bits in
//! RFC 4648 base32, lowercase and unpadded (26); `host` is percent-encoded
//! UTF-8 under the display-name rule ([`display_name_ok`]); `exp` is unix
//! seconds.
//!
//! **The address is not parsed here.** §4.1's address rules — `IpAddr`
//! never `SocketAddr` or a resolver, no `%` zone, the private-class
//! allowlist — are `netclass`'s, one implementation shared by serve and the
//! shim (Phase 2 slice 6a). [`parse`] hands the raw `addr` value to the
//! [`AddrRule`] its caller passes, and refuses the link if that does.

use std::net::{IpAddr, SocketAddr};

/// The scheme and version, everything before `?`.
pub const SCHEME: &str = "worktrees-pair:v1";

/// The longest link [`parse`] reads. A real one is under 220 bytes.
pub const MAX_LEN: usize = 1024;

/// The longest display name, in characters.
pub const NAME_MAX: usize = 64;

/// A pairing link, read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairUri {
    /// Where the pairing listener is: an address literal and a port, never a
    /// name, so a device classifies what it will dial before it dials.
    pub addr: SocketAddr,
    /// The server's pin: SHA-256 of its SPKI, lowercase hex.
    pub pin: String,
    /// The window's one-use secret.
    pub secret: [u8; 16],
    /// The host's display name.
    pub host: String,
    /// When the window closes, unix seconds.
    pub exp: i64,
}

/// The `addr` parameter's reader: §4.1's rules, from `netclass` (slice 6a).
/// It gets the value exactly as the link carried it (percent-decoding
/// does not apply: an address literal needs none, and a `%` must reach the
/// rule that refuses zones).
pub type AddrRule<'a> = &'a dyn Fn(&str) -> Result<SocketAddr, String>;

/// The link for `u`. A scoped IPv6 address is refused: a QR carries no zone
/// (§4.1), since the host's interface means nothing on the device.
pub fn build(u: &PairUri) -> Result<String, String> {
    if let SocketAddr::V6(a) = u.addr {
        if a.scope_id() != 0 {
            return Err("a pairing link carries no zone".into());
        }
    }
    if !is_pin(&u.pin) {
        return Err("a pin is 64 lowercase hex characters".into());
    }
    display_name_ok(&u.host)?;
    let addr = match u.addr.ip() {
        IpAddr::V4(ip) => format!("{ip}:{}", u.addr.port()),
        IpAddr::V6(ip) => format!("[{ip}]:{}", u.addr.port()),
    };
    Ok(format!("{SCHEME}?addr={addr}&pin={}&secret={}&host={}&exp={}", u.pin, base32(&u.secret), pct_encode(&u.host), u.exp))
}

/// Read a link strictly, before anything is dialled (§2.2 step 3). `now` is
/// the device's clock; a link whose `exp` is not after it is refused.
pub fn parse(s: &str, now: i64, addr_rule: AddrRule<'_>) -> Result<PairUri, String> {
    if s.len() > MAX_LEN {
        return Err("not a pairing link: too long".into());
    }
    let query = s.strip_prefix(SCHEME).and_then(|r| r.strip_prefix('?')).ok_or("not a worktrees pairing link (v1)")?;
    let (mut addr, mut pin, mut secret, mut host, mut exp) = (None, None, None, None, None);
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').ok_or_else(|| format!("pairing link: {pair:?} is not key=value"))?;
        let slot = match k {
            "addr" => &mut addr,
            "pin" => &mut pin,
            "secret" => &mut secret,
            "host" => &mut host,
            "exp" => &mut exp,
            _ => return Err(format!("pairing link: unknown parameter {k:?}")),
        };
        if slot.replace(v).is_some() {
            return Err(format!("pairing link: {k} appears twice"));
        }
    }
    fn need<'a>(v: Option<&'a str>, k: &str) -> Result<&'a str, String> {
        v.filter(|v| !v.is_empty()).ok_or_else(|| format!("pairing link: no {k}"))
    }
    let addr = addr_rule(need(addr, "addr")?)?;
    let pin = need(pin, "pin")?;
    if !is_pin(pin) {
        return Err("pairing link: the pin is not 64 lowercase hex characters".into());
    }
    let secret: [u8; 16] = base32_decode(need(secret, "secret")?)
        .and_then(|b| b.try_into().ok())
        .ok_or("pairing link: the secret is not 128 bits of lowercase base32")?;
    let host = pct_decode(need(host, "host")?).ok_or("pairing link: the host name is not percent-encoded UTF-8")?;
    display_name_ok(&host).map_err(|e| format!("pairing link: {e}"))?;
    let exp = need(exp, "exp")?;
    let exp: i64 = exp.parse().ok().filter(|_| exp.bytes().all(|c| c.is_ascii_digit())).ok_or("pairing link: exp is not a time")?;
    if exp <= now {
        return Err("this pairing link has expired; open a new pairing window on the host".into());
    }
    Ok(PairUri { addr, pin: pin.to_string(), secret, host, exp })
}

/// SHA-256 in lowercase hex.
pub fn is_pin(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

/// A name another screen will show (a device's name in the host's dialog, the
/// host's on the device; §2.2 step 4): 1 to 64 characters, no control
/// characters, and nothing invisible or direction-changing that could make
/// one name pass for another — the bidirectional overrides, isolates and
/// marks, zero-width characters, the line and paragraph separators, the soft
/// hyphen. Refused, never stripped.
pub fn display_name_ok(name: &str) -> Result<(), String> {
    let n = name.chars().count();
    if n == 0 || name.trim().is_empty() {
        return Err("a name cannot be empty".into());
    }
    if n > NAME_MAX {
        return Err(format!("a name is at most {NAME_MAX} characters"));
    }
    let invisible = |c: char| {
        c.is_control()
            || matches!(c, '\u{00AD}' | '\u{061C}' | '\u{180E}' | '\u{FEFF}')
            || ('\u{200B}'..='\u{200F}').contains(&c)
            || ('\u{2028}'..='\u{202E}').contains(&c)
            || ('\u{2060}'..='\u{2069}').contains(&c)
    };
    if name.chars().any(invisible) {
        return Err("a name cannot hold control, invisible or direction-changing characters".into());
    }
    Ok(())
}

const B32: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// RFC 4648 base32, lowercase, unpadded.
pub fn base32(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &b in bytes {
        acc = (acc << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(B32[((acc >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(B32[((acc << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// The inverse of [`base32`], and only of it: lowercase, unpadded, a length
/// a whole number of bytes encodes to, and unused trailing bits zero — so
/// each value has exactly one spelling.
pub fn base32_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 5 / 8);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = B32.iter().position(|x| *x == c)? as u32;
        acc = (acc << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
        acc &= (1 << bits) - 1;
    }
    (bits < 5 && acc == 0 && base32(&out) == s).then_some(out)
}

fn unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')
}

fn pct_encode(s: &str) -> String {
    s.bytes().map(|b| if unreserved(b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// Percent-decoding, strictly: `%` and two hex digits, nothing reserved left
/// bare, and the result UTF-8.
fn pct_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(h, 16).ok().filter(|_| h.bytes().all(|c| c.is_ascii_hexdigit()))?);
            i += 3;
        } else if unreserved(b[i]) {
            out.push(b[i]);
            i += 1;
        } else {
            return None;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    /// A stand-in for netclass's rule, for these tests only: an address
    /// literal and a port, nothing else. The real one adds §4.1's classes.
    fn literal(v: &str) -> Result<SocketAddr, String> {
        if v.contains('%') {
            return Err("a zone".into());
        }
        let (h, p) = v.rsplit_once(':').ok_or("no port")?;
        let ip: IpAddr = h.strip_prefix('[').and_then(|h| h.strip_suffix(']')).unwrap_or(h).parse().map_err(|_| "not an address")?;
        Ok(SocketAddr::new(ip, p.parse().map_err(|_| "not a port")?))
    }

    fn sample() -> PairUri {
        PairUri { addr: "100.101.7.3:7461".parse().unwrap(), pin: "ab".repeat(32), secret: [7; 16], host: "Bret's Mac mini".into(), exp: NOW + 120 }
    }

    #[test]
    fn a_link_round_trips_and_names_only_what_section_2_2_lists() {
        let s = build(&sample()).unwrap();
        assert!(s.starts_with("worktrees-pair:v1?addr=100.101.7.3:7461&pin="), "{s}");
        assert!(s.contains("&host=Bret%27s%20Mac%20mini&exp="), "{s}");
        assert_eq!(parse(&s, NOW, &literal).unwrap(), sample());
        let v6 = PairUri { addr: "[fd7a:115c:a1e0::1]:7461".parse().unwrap(), ..sample() };
        let s6 = build(&v6).unwrap();
        assert!(s6.contains("addr=[fd7a:115c:a1e0::1]:7461&"), "{s6}");
        assert_eq!(parse(&s6, NOW, &literal).unwrap(), v6);
        let scoped = PairUri { addr: "[fe80::1%4]:7461".parse().unwrap(), ..sample() };
        assert!(build(&scoped).is_err(), "no zone ever goes into a link");
    }

    /// §2.2 step 3, O13: the device refuses an unknown parameter, a repeated
    /// one, an expired `exp`, and a value it cannot read — never repaired.
    #[test]
    fn a_link_is_read_strictly() {
        let good = build(&sample()).unwrap();
        let swap = |k: &str, v: &str| {
            let (head, q) = good.split_once('?').unwrap();
            let q: Vec<String> = q.split('&').map(|p| if p.starts_with(&format!("{k}=")) { format!("{k}={v}") } else { p.to_string() }).collect();
            format!("{head}?{}", q.join("&"))
        };
        let bad = [
            format!("{good}&extra=1"),
            format!("{good}&pin={}", "ab".repeat(32)),
            good.replace("worktrees-pair:v1", "worktrees-pair:v2"),
            good.replace("?addr=", "?ADDR="),
            swap("exp", &NOW.to_string()),
            swap("exp", "+1800000200"),
            swap("exp", ""),
            swap("pin", &"AB".repeat(32)),
            swap("pin", &"ab".repeat(31)),
            swap("secret", "aaaaaaaaaaaaaaaaaaaaaaaaab"),
            swap("secret", &base32(&[1; 17])),
            swap("secret", "AAAAAAAAAAAAAAAAAAAAAAAAAA"),
            swap("host", "Bret%E2%80%AEenohP"),
            swap("host", "a%2"),
            swap("host", "bad host"),
            swap("host", "%FF"),
            swap("addr", "[fe80::1%254]:7461"),
            swap("addr", "example.com:7461"),
            format!("{good}{}", "x".repeat(MAX_LEN)),
            "worktrees-pair:v1".into(),
        ];
        for b in bad {
            assert!(parse(&b, NOW, &literal).is_err(), "{b}");
        }
        let e = parse(&swap("exp", "1000"), NOW, &literal).unwrap_err();
        assert!(e.contains("expired"), "{e}");
    }

    /// The address is the rule's to judge, not this parser's: whatever the
    /// caller's rule refuses (netclass will refuse a public address, §4.1),
    /// the link is refused with its words.
    #[test]
    fn the_address_rule_decides_the_address() {
        let s = build(&sample()).unwrap();
        let public = |_: &str| -> Result<SocketAddr, String> { Err("not-private".into()) };
        assert_eq!(parse(&s, NOW, &public).unwrap_err(), "not-private");
        let seen = std::cell::RefCell::new(String::new());
        let spy = |v: &str| {
            *seen.borrow_mut() = v.to_string();
            literal(v)
        };
        parse(&s, NOW, &spy).unwrap();
        assert_eq!(*seen.borrow(), "100.101.7.3:7461", "the raw value, as carried");
    }

    #[test]
    fn base32_has_one_spelling_per_value() {
        for (i, o) in [("", ""), ("f", "my"), ("fo", "mzxq"), ("foo", "mzxw6"), ("foob", "mzxw6yq"), ("fooba", "mzxw6ytb"), ("foobar", "mzxw6ytboi")] {
            assert_eq!(base32(i.as_bytes()), o);
            assert_eq!(base32_decode(o).unwrap(), i.as_bytes());
        }
        for bad in ["MY", "mz", "m", "my=", "m1", "mzxw6ytbo"] {
            assert!(base32_decode(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn a_name_cannot_pass_itself_off_as_another() {
        for ok in ["Bret's iPhone", "Mac mini (office)", "ラップトップ", &"x".repeat(64)] {
            assert!(display_name_ok(ok).is_ok(), "{ok:?}");
        }
        for bad in [
            "", "   ", &"x".repeat(65), "Phone\n", "Phone\u{7}", "Phone\u{85}", "Bret\u{202E}enohP", "Bret\u{2066}x\u{2069}",
            "i\u{200B}Phone", "Phone\u{200F}", "Phone\u{2028}", "\u{FEFF}Phone", "Pho\u{00AD}ne", "\u{061C}Phone",
        ] {
            assert!(display_name_ok(bad).is_err(), "{bad:?}");
        }
    }
}
