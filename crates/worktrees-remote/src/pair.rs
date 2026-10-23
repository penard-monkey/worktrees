//! Pairing, the device's side (Phase 2 §2.2–§2.4).
//!
//! 1. Read the link strictly, with §4.1's address rule (`pairuri::dial_rule`,
//!    which is `netclass::parse_dial`), BEFORE anything is dialled: a link is
//!    untrusted input, and a public address in one is a phishing attempt. A
//!    refused link is answered with the parser's own words, and no socket
//!    exists yet.
//! 2. Make this pairing's key (§2.2 step 3 — one key per pairing).
//! 3. Dial the pairing port with TLS 1.3 pinned to the link's SPKI, presenting
//!    a self-signed certificate over the new key.
//! 4. Send one line:
//!    `{"pair": {"name", "platform", "stepup_spki": null}, "proof": <64 hex>}`,
//!    `proof = HMAC-SHA256(secret, "worktrees-pair/v1\n" ‖ exporter ‖
//!    sha256(device spki) ‖ (sha256(step-up spki) | 32 zero bytes) ‖ name)`,
//!    `exporter` being 32 bytes of the session's RFC 8446 §7.5 exporter under
//!    `EXPORTER-worktrees-pair-v1`: bound to this session and these keys.
//! 5. On `{"pending": true}`, show the comparison code — the first four bytes
//!    of `HMAC-SHA256(secret, "worktrees-pair/v1/sas\n" ‖ exporter)`,
//!    big-endian, high bit cleared, mod 10⁶ — and wait for the host.
//! 6. `{"paired": {…}}` is kept (`hosts`), with its key; `{"denied": why}` is
//!    said in words that point at the host (§2.2 step 6).
//!
//! A desktop has no presence-bound key, so it sends no step-up key (`null`);
//! serve refuses a desktop that sends one, and an iPhone that does not.
//!
//! [`proof_ok`] and [`comparison_code`] are serve's as well: its pairing
//! listener imports them, so the two ends compute the proof and the code with
//! one implementation, and serve's end-to-end test restates both from §2.2.

use std::net::SocketAddr;
use std::time::Duration;

use hmac::{KeyInit, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use worktrees_core::pairuri::{self, PairUri};

use crate::hosts::{new_host_id, HostRecord, HostStore};
use crate::key::DeviceKey;
use crate::tls;

pub const PROOF_LABEL: &[u8] = b"worktrees-pair/v1\n";
pub const SAS_LABEL: &[u8] = b"worktrees-pair/v1/sas\n";

/// The longest line either side sends, newline included (serve's
/// `pairing::MAX_FRAME`).
pub const MAX_LINE: usize = 4096;

type HmacSha256 = hmac::Hmac<Sha256>;

fn unhex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

fn mac_proof(secret: &[u8; 16], exporter: &[u8; 32], device_pin: &str, stepup_pin: Option<&str>, name: &str) -> Option<HmacSha256> {
    let d = unhex32(device_pin)?;
    let s = match stepup_pin {
        Some(p) => unhex32(p)?,
        None => [0u8; 32],
    };
    let mut m = HmacSha256::new_from_slice(secret).expect("HMAC takes a key of any length");
    m.update(PROOF_LABEL);
    m.update(exporter);
    m.update(&d);
    m.update(&s);
    m.update(name.as_bytes());
    Some(m)
}

/// The proof a device sends (§2.2 step 4). `None` for a pin that is not 64
/// hex characters.
pub fn proof(secret: &[u8; 16], exporter: &[u8; 32], device_pin: &str, stepup_pin: Option<&str>, name: &str) -> Option<[u8; 32]> {
    Some(mac_proof(secret, exporter, device_pin, stepup_pin, name)?.finalize().into_bytes().into())
}

/// The proof the device must have sent, checked in constant time (§2.2 step
/// 5). serve's pairing listener calls this.
pub fn proof_ok(secret: &[u8; 16], exporter: &[u8; 32], device_pin: &str, stepup_pin: Option<&str>, name: &str, proof: &[u8; 32]) -> bool {
    mac_proof(secret, exporter, device_pin, stepup_pin, name).is_some_and(|m| m.verify_slice(proof).is_ok())
}

/// The six-digit comparison code both screens show (§2.2 step 6).
pub fn comparison_code(secret: &[u8; 16], exporter: &[u8; 32]) -> String {
    let mut m = HmacSha256::new_from_slice(secret).expect("HMAC takes a key of any length");
    m.update(SAS_LABEL);
    m.update(exporter);
    let t = m.finalize().into_bytes();
    let n = u32::from_be_bytes([t[0], t[1], t[2], t[3]]) & 0x7fff_ffff;
    format!("{:06}", n % 1_000_000)
}

/// What kind of machine this device is, as the pairing frame names it. A
/// desktop: its drive-class acts rest on one factor (§2.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Macos,
    Linux,
}

impl Platform {
    pub fn this() -> Platform {
        if cfg!(target_os = "macos") {
            Platform::Macos
        } else {
            Platform::Linux
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Platform::Macos => "macos",
            Platform::Linux => "linux",
        }
    }
}

/// Why the host said no (§2.2: `{"denied": …}`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Denied {
    /// The host pressed Deny.
    Denied,
    /// Nobody pressed Allow or Deny within the host's 60 s.
    Timeout,
    /// The window closed while the request waited: cancelled on the host,
    /// replaced by another, or a second device reached it.
    Closed,
    /// Allowed, but the host could not enroll this device.
    Refused,
}

impl Denied {
    fn parse(s: &str) -> Option<Denied> {
        Some(match s {
            "denied" => Denied::Denied,
            "timeout" => Denied::Timeout,
            "closed" => Denied::Closed,
            "refused" => Denied::Refused,
            _ => return None,
        })
    }
}

/// How a pairing failed, each in words that say what to do.
#[derive(Debug)]
pub enum PairError {
    /// The link was refused before anything was dialled: the parser's words.
    Link(String),
    /// This device's name would not pass the host's dialog.
    Name(String),
    /// Nothing listens at the link's address any more: its one attempt was
    /// used, or its window closed.
    WindowGone { host: String, addr: SocketAddr },
    /// The address did not answer, or could not be reached.
    Unreachable { host: String, addr: SocketAddr, why: String },
    /// The machine at the address does not hold the link's key.
    NotThePin { host: String, addr: SocketAddr },
    Tls { host: String, why: String },
    /// The host closed the window with nothing said: it refused the attempt.
    Silent { host: String },
    Denied { host: String, why: Denied },
    Protocol { host: String, why: String },
    Store(String),
}

impl std::fmt::Display for PairError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PairError::Link(m) => write!(f, "{m}"),
            PairError::Name(m) => write!(f, "this device's name: {m}"),
            PairError::WindowGone { host, addr } => write!(
                f,
                "nothing answered at {addr}: someone else used this pairing code, or its window has closed. \
                 Tap Deny on {host} if it is asking, then open a new pairing window there"
            ),
            PairError::Unreachable { host, addr, why } => {
                write!(f, "could not reach {host} at {addr} ({why}); is this machine on {host}'s network?")
            }
            PairError::NotThePin { host, addr } => write!(
                f,
                "the machine at {addr} is not {host}: its key does not match the pairing link's pin, so this \
                 device sent it nothing. Check the link came from {host}'s own screen"
            ),
            PairError::Tls { host, why } => write!(f, "the secure connection to {host} failed: {why}"),
            PairError::Silent { host } => write!(
                f,
                "{host} closed the pairing window without an answer: it refused this attempt. Open a new pairing \
                 window there and try again"
            ),
            PairError::Denied { host, why } => match why {
                Denied::Denied => write!(f, "{host} said no: Deny was pressed there"),
                Denied::Timeout => write!(f, "nobody answered on {host} in time; open a new pairing window there and try again"),
                Denied::Closed => write!(
                    f,
                    "the pairing window on {host} closed before anyone answered: it was cancelled, replaced, or \
                     someone else used this pairing code too. Tap Deny on {host} if it is still asking, then open a \
                     new window"
                ),
                Denied::Refused => write!(
                    f,
                    "{host} allowed this device but could not enroll it (it may have as many devices as it takes); \
                     check its Devices list"
                ),
            },
            PairError::Protocol { host, why } => write!(f, "{host} answered something this client does not read: {why}"),
            PairError::Store(m) => write!(f, "paired, but this device could not keep the pairing: {m}"),
        }
    }
}

/// How long each step may take. The host's own deadlines are 120 s for the
/// frame and 60 s for its decision (§2.4); a device waits a little longer than
/// each, so the host's answer, not the device's clock, decides.
#[derive(Clone, Copy, Debug)]
pub struct Waits {
    pub connect: Duration,
    pub pending: Duration,
    pub decision: Duration,
}

impl Default for Waits {
    fn default() -> Self {
        Waits { connect: Duration::from_secs(5), pending: Duration::from_secs(15), decision: Duration::from_secs(75) }
    }
}

/// What happens on the way, for the screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PairEvent {
    /// The link was read; dialling the host.
    Dialling { host: String, addr: SocketAddr },
    /// The host took the proof: show this code and wait for the host's answer.
    Code(String),
}

/// Pair with the host `link` names and keep the pairing in `store`.
pub async fn pair(store: &HostStore, link: &str, name: &str, now: i64, on_event: &mut (dyn FnMut(PairEvent) + Send)) -> Result<HostRecord, PairError> {
    pair_with(store, link, name, Platform::this(), now, &pairuri::dial_rule, Waits::default(), on_event).await
}

/// [`pair`], with the address rule and the waits given: the tests' entry,
/// since a real rule refuses the loopback their hosts listen on.
#[allow(clippy::too_many_arguments)]
pub async fn pair_with(
    store: &HostStore,
    link: &str,
    name: &str,
    platform: Platform,
    now: i64,
    rule: &(dyn Fn(&str) -> Result<SocketAddr, String> + Sync),
    waits: Waits,
    on_event: &mut (dyn FnMut(PairEvent) + Send),
) -> Result<HostRecord, PairError> {
    // 1. Nothing is dialled before the link — its address above all — is read.
    let u: PairUri = pairuri::parse(link, now, rule).map_err(PairError::Link)?;
    pairuri::display_name_ok(name).map_err(PairError::Name)?;
    let host = u.host.clone();
    // 2. This pairing's key.
    let key = DeviceKey::generate().map_err(PairError::Store)?;
    let cfg = key.client_config(&u.pin).map_err(|why| PairError::Tls { host: host.clone(), why })?;
    on_event(PairEvent::Dialling { host: host.clone(), addr: u.addr });
    // 3. Pinned TLS to the pairing port.
    let tcp = crate::dial::tcp(u.addr, waits.connect).await.map_err(|e| match e.kind() {
        std::io::ErrorKind::ConnectionRefused => PairError::WindowGone { host: host.clone(), addr: u.addr },
        _ => PairError::Unreachable { host: host.clone(), addr: u.addr, why: e.to_string() },
    })?;
    let connector = tokio_rustls::TlsConnector::from(cfg);
    let server = rustls::pki_types::ServerName::IpAddress(u.addr.ip().into());
    let tls = match tokio::time::timeout(waits.connect, connector.connect(server, tcp)).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) if tls::is_pin_mismatch(&e) => return Err(PairError::NotThePin { host, addr: u.addr }),
        Ok(Err(e)) => return Err(PairError::Tls { host, why: e.to_string() }),
        Err(_) => return Err(PairError::Tls { host, why: "the handshake did not finish in time".into() }),
    };
    let exporter = tls::client_exporter(tls.get_ref().1, tls::EXPORTER_PAIR).map_err(|why| PairError::Tls { host: host.clone(), why })?;
    // 4. The one frame.
    let p = proof(&u.secret, &exporter, key.pin(), None, name).ok_or_else(|| PairError::Store("a pin that is not hex".into()))?;
    let line = json!({"pair": {"name": name, "platform": platform.as_str(), "stepup_spki": null}, "proof": tls::hex(&p)}).to_string();
    let mut s = BufReader::new(tls);
    let sent = async {
        s.get_mut().write_all(format!("{line}\n").as_bytes()).await?;
        s.get_mut().flush().await
    };
    // A host that already closed the window may reset the write: the read
    // below says so in the host's terms.
    let _ = sent.await;
    // 5. The host took the proof (or closed with nothing said).
    match read_line(&mut s, waits.pending).await {
        Some(v) if v == json!({"pending": true}) => {}
        Some(v) => return Err(answer(&host, v).err().unwrap_or(PairError::Protocol { host: host.clone(), why: "an answer before the host had decided".into() })),
        None => return Err(PairError::Silent { host }),
    }
    on_event(PairEvent::Code(comparison_code(&u.secret, &exporter)));
    // 6. The host's answer.
    let v = read_line(&mut s, waits.decision).await.ok_or_else(|| PairError::Denied { host: host.clone(), why: Denied::Timeout })?;
    let rec = answer(&host, v)?;
    let rec = HostRecord { pin: u.pin.clone(), paired_at: now, ..rec };
    store.add(&rec, &key).map_err(PairError::Store)?;
    Ok(rec)
}

/// The host's last line: a record to keep, or a refusal in its words.
fn answer(host: &str, v: Value) -> Result<HostRecord, PairError> {
    let proto = |why: &str| PairError::Protocol { host: host.to_string(), why: why.to_string() };
    if let Some(d) = v.get("denied") {
        let why = d.as_str().and_then(Denied::parse).ok_or_else(|| proto("an unknown refusal"))?;
        return Err(PairError::Denied { host: host.to_string(), why });
    }
    let p = v.get("paired").and_then(Value::as_object).ok_or_else(|| proto("neither paired nor denied"))?;
    let text = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string).ok_or_else(|| proto(&format!("no {k}")));
    let remote = text("remote_addr")?;
    worktrees_core::netclass::parse_dial(&remote).map_err(|e| proto(&format!("its remote address: {e}")))?;
    let rec = HostRecord {
        id: new_host_id(),
        name: text("host_name")?,
        addr: remote,
        pin: String::new(),
        discovery_id: match p.get("discovery_id") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(_) => return Err(proto("its discovery id is not a string")),
        },
        device_id: text("device_id")?,
        principal_id: text("principal_id")?,
        expiry: p.get("expiry").cloned().unwrap_or(Value::Null),
        paired_at: 0,
    };
    // Checked with a stand-in pin: the real one is the link's, set by the caller.
    HostRecord { pin: "0".repeat(64), ..rec.clone() }.check().map_err(|e| proto(&e))?;
    Ok(rec)
}

/// One line of JSON within `wait`, at most [`MAX_LINE`] bytes; `None` for
/// silence, the end, or anything that is not one JSON line.
async fn read_line<S: tokio::io::AsyncRead + Unpin>(s: &mut BufReader<S>, wait: Duration) -> Option<Value> {
    let mut buf = Vec::with_capacity(256);
    let read = async {
        loop {
            let avail = s.fill_buf().await.ok()?;
            if avail.is_empty() {
                return None;
            }
            match avail.iter().position(|b| *b == b'\n') {
                Some(i) => {
                    buf.extend_from_slice(&avail[..i]);
                    s.consume(i + 1);
                    return Some(());
                }
                None => {
                    let n = avail.len();
                    buf.extend_from_slice(avail);
                    s.consume(n);
                }
            }
            if buf.len() >= MAX_LINE {
                return None;
            }
        }
    };
    tokio::time::timeout(wait, read).await.ok()??;
    serde_json::from_slice(&buf).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §2.2's two formulas, restated from the text rather than from the code
    /// above: a proof made one way verifies the other, and the code is
    /// deterministic per (secret, exporter).
    #[test]
    fn the_proof_and_the_code_follow_section_2_2() {
        let (secret, ex) = ([7u8; 16], [9u8; 32]);
        let dev = "11".repeat(32);
        let p = proof(&secret, &ex, &dev, None, "Laptop").unwrap();
        let mut m = HmacSha256::new_from_slice(&secret).unwrap();
        m.update(b"worktrees-pair/v1\n");
        m.update(&ex);
        m.update(&[0x11; 32]);
        m.update(&[0; 32]);
        m.update(b"Laptop");
        assert_eq!(p.to_vec(), m.finalize().into_bytes().to_vec());
        assert!(proof_ok(&secret, &ex, &dev, None, "Laptop", &p));
        assert!(!proof_ok(&secret, &ex, &dev, None, "Laptop2", &p), "the name is bound");
        assert!(!proof_ok(&secret, &[8; 32], &dev, None, "Laptop", &p), "the session is bound");
        assert!(!proof_ok(&secret, &ex, &"22".repeat(32), None, "Laptop", &p), "the key is bound");
        assert!(!proof_ok(&secret, &ex, &dev, Some(&"33".repeat(32)), "Laptop", &p), "the step-up key is bound");
        assert!(!proof_ok(&[6; 16], &ex, &dev, None, "Laptop", &p), "the secret is the key");
        let code = comparison_code(&secret, &ex);
        assert_eq!(code.len(), 6);
        assert!(code.bytes().all(|b| b.is_ascii_digit()));
        assert_eq!(code, comparison_code(&secret, &ex));
        assert_ne!(code, comparison_code(&secret, &[8; 32]));
    }
}
