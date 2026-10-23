//! The remote socket, as a pipe of text frames (Phase 2 §4.4).
//!
//! TCP to the kept address, TLS 1.3 pinned to the kept SPKI with this
//! pairing's device key presented — asked to sign through `sign::Keys`, never
//! read — then a WebSocket upgrade:
//!
//! ```text
//! GET / HTTP/1.1
//! Host: <address literal>:<port>      ([v6]:port for IPv6)
//! (no Origin: the client is native, and §4.5 refuses every browser origin)
//! ```
//!
//! From there it moves text frames in and out and parses NONE of them: the
//! protocol — handshake v1, requests, channels, events — is the page's
//! (`app/src/ipcws.ts`), or [`crate::call`]'s for the CLI. What it hands its
//! owner besides the frames is the session's handshake-v1 key
//! ([`Pipe::session_token`]: the hex of 32 bytes of the TLS exporter under
//! `EXPORTER-worktrees-remote-v1`), which works only inside this TLS session,
//! and, when the socket ends, why: a host that revokes or disables the device
//! closes with 1008 and says which. On a device that holds a step-up key it
//! also hands over [`Pipe::stepup`], which signs step-up proofs for THIS
//! session only.

use std::time::Duration;

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use crate::hosts::HostRecord;
use crate::sign::{Keys, SignError};
use crate::stepup::StepUp;
use crate::tls;

type Ws = WebSocketStream<TlsStream<TcpStream>>;

/// How opening the socket failed.
#[derive(Debug)]
pub enum PipeError {
    /// The kept record cannot be dialled as it stands.
    Record(String),
    /// TCP did not connect.
    Connect(String),
    /// This device's own key did not sign: cancelled on the device, unable to
    /// sign (locked, gone), or a signature that does not verify. Before the
    /// dial when it was the certificate's signature, inside the handshake
    /// when it was the handshake's; either way the host received no
    /// signature from it.
    Sign(SignError),
    /// The machine at the address does not hold the pinned key: nothing of
    /// this device's was sent to it.
    NotThePin,
    /// The TLS handshake failed for another reason — for one, the host no
    /// longer accepts this device's key (revoked, disabled, expired, or never
    /// enrolled): TLS 1.3 tells the client only at its first read.
    Tls(String),
    /// The HTTP upgrade was refused (the host closes with nothing said).
    Upgrade(String),
}

impl std::fmt::Display for PipeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PipeError::Record(m) => write!(f, "{m}"),
            PipeError::Connect(m) => write!(f, "could not connect: {m}"),
            PipeError::Sign(e) => write!(f, "this device's key did not sign the connection: {e}"),
            PipeError::NotThePin => write!(f, "the machine at that address is not the paired host: its key does not match the pin, so nothing was sent to it"),
            PipeError::Tls(m) => write!(f, "the secure connection failed: {m}"),
            PipeError::Upgrade(m) => {
                write!(f, "the host refused the connection after TLS ({m}); if it no longer knows this device, pair again")
            }
        }
    }
}

impl std::error::Error for PipeError {}

/// An open remote socket.
pub struct Pipe {
    ws: Ws,
    token: String,
    stepup: Option<StepUp>,
}

/// One thing read from the socket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inbound {
    Frame(String),
    /// The socket is over; why, in words.
    Closed(String),
}

/// Open the socket to `rec`'s host with this pairing's `keys`, all of it
/// within `wait`. The record is read again first, its address by the dial
/// rule.
pub async fn open(rec: &HostRecord, keys: &Keys, wait: Duration) -> Result<Pipe, PipeError> {
    rec.check().map_err(PipeError::Record)?;
    let d = rec.socket_addr().map_err(PipeError::Record)?;
    open_at(std::net::SocketAddr::new(d.ip, d.port), &rec.pin, keys, wait).await
}

/// [`open`], to an address the caller has already judged — the tests', whose
/// hosts listen on the loopback the dial rule refuses.
pub async fn open_at(addr: std::net::SocketAddr, pin: &str, keys: &Keys, wait: Duration) -> Result<Pipe, PipeError> {
    // The certificate is signed here, before anything is dialled: a key that
    // cannot sign never reaches the host.
    let certified = tls::certified_key(keys.device.clone()).map_err(PipeError::Sign)?;
    let cfg = tls::client_config(pin, certified).map_err(PipeError::Tls)?;
    let work = async {
        let tcp = crate::dial::tcp(addr, wait).await.map_err(|e| PipeError::Connect(e.to_string()))?;
        let server = rustls::pki_types::ServerName::IpAddress(addr.ip().into());
        let tls = tokio_rustls::TlsConnector::from(cfg).connect(server, tcp).await.map_err(|e| {
            if tls::is_pin_mismatch(&e) {
                PipeError::NotThePin
            } else if let Some(why) = tls::sign_error(&e) {
                PipeError::Sign(why.clone())
            } else {
                PipeError::Tls(e.to_string())
            }
        })?;
        let ex = tls::client_exporter(tls.get_ref().1, tls::EXPORTER_SESSION).map_err(PipeError::Tls)?;
        let stepup = match &keys.stepup {
            Some(k) => Some(StepUp::new(k.clone(), tls::client_exporter(tls.get_ref().1, tls::EXPORTER_STEPUP).map_err(PipeError::Tls)?)),
            None => None,
        };
        // Host is the listener's rule exactly: `SocketAddr`'s Display of the
        // address dialled (`192.168.0.40:53455`, `[fe80::1]:7443` — zone-less,
        // lowercase, compressed), set here rather than left to how tungstenite
        // derives it from the URI (which drops a default port). No Origin: the
        // listener refuses any.
        let mut req = format!("ws://{addr}/").into_client_request().map_err(|e| PipeError::Upgrade(e.to_string()))?;
        let host = addr.to_string().parse().map_err(|e: tokio_tungstenite::tungstenite::http::header::InvalidHeaderValue| PipeError::Upgrade(e.to_string()))?;
        req.headers_mut().insert("Host", host);
        let (ws, _) = tokio_tungstenite::client_async(req, tls).await.map_err(|e| upgrade_error(&e))?;
        Ok(Pipe { ws, token: tls::hex(&ex), stepup })
    };
    tokio::time::timeout(wait, work).await.map_err(|_| PipeError::Connect(format!("{addr} did not answer within {}s", wait.as_secs())))?
}

/// TLS 1.3 settles the client's certificate after the client thinks the
/// handshake is done, so a refused key shows up at the upgrade's first read,
/// as a TLS alert.
fn upgrade_error(e: &tokio_tungstenite::tungstenite::Error) -> PipeError {
    use tokio_tungstenite::tungstenite::Error as E;
    match e {
        E::Io(io) => match io.get_ref().and_then(|i| i.downcast_ref::<rustls::Error>()) {
            Some(rustls::Error::AlertReceived(a)) => {
                PipeError::Tls(format!("the host refused this device's key ({a:?}): it was revoked, disabled or expired there, or never enrolled"))
            }
            Some(t) => PipeError::Tls(t.to_string()),
            None => PipeError::Upgrade(io.to_string()),
        },
        other => PipeError::Upgrade(other.to_string()),
    }
}

impl Pipe {
    /// The session's handshake-v1 key: lowercase hex of the TLS exporter
    /// under `EXPORTER-worktrees-remote-v1`. Usable only inside this session.
    pub fn session_token(&self) -> &str {
        &self.token
    }

    /// This session's step-up (§2.5): proofs signed by the pairing's step-up
    /// key over this session's exporter. `None` on a device with no step-up
    /// key (a desktop). Take it before [`Self::split`] and keep it with the
    /// connection: a proof it makes is good on this session and no other.
    pub fn stepup(&self) -> Option<StepUp> {
        self.stepup.clone()
    }

    pub fn split(self) -> (PipeTx, PipeRx) {
        let (tx, rx) = self.ws.split();
        (PipeTx(tx), PipeRx(rx))
    }
}

/// The writing half.
pub struct PipeTx(SplitSink<Ws, Message>);

impl PipeTx {
    pub async fn send(&mut self, text: String) -> Result<(), String> {
        self.0.send(Message::text(text)).await.map_err(|e| e.to_string())
    }

    /// Close the socket, politely.
    pub async fn close(mut self) {
        let _ = self.0.send(Message::Close(None)).await;
        let _ = self.0.close().await;
    }
}

/// The reading half.
pub struct PipeRx(SplitStream<Ws>);

impl PipeRx {
    /// The next text frame, or why the socket ended. Pings are answered by
    /// tungstenite as it reads; a binary frame ends the socket (the protocol
    /// is text only).
    pub async fn recv(&mut self) -> Inbound {
        loop {
            match self.0.next().await {
                Some(Ok(Message::Text(t))) => return Inbound::Frame(t.to_string()),
                Some(Ok(Message::Close(c))) => {
                    return Inbound::Closed(match c {
                        Some(f) if !f.reason.is_empty() => format!("the host closed the connection: {}", crate::printable(&f.reason)),
                        _ => "the host closed the connection".into(),
                    })
                }
                Some(Ok(Message::Binary(_))) => return Inbound::Closed("the host sent a binary frame; the protocol is text only".into()),
                Some(Ok(_)) => continue,
                Some(Err(e)) => return Inbound::Closed(format!("the connection failed: {e}")),
                None => return Inbound::Closed("the connection ended".into()),
            }
        }
    }
}
