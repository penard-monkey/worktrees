//! One request over a remote socket, for `worktrees remote call` and the CI
//! witness: handshake v1 written out by hand, as
//! `crates/worktrees-cli/tests/serve_bin.rs::connect` writes it, keyed with
//! the session's exporter instead of serve.json's token.
//!
//! ```text
//! → {"hello": cn}                          cn: 32 random bytes, hex
//! ← {"challenge": sn}
//! → {"auth": HMAC(K, "worktrees-serve/v1/client\n" ‖ cn ‖ "\n" ‖ sn)}
//! ← {"ok": true, "version", "proof": HMAC(K, "…/server\n" ‖ cn ‖ "\n" ‖ sn), "device": {…}}
//! → {"id": 1, "cmd", "args"}               only after the proof verifies
//! ← {"id": 1, "ok"} | {"id": 1, "err"}     channel and event frames on the way are skipped
//! ```
//!
//! K is the UTF-8 of [`Pipe::session_token`]. Over pinned mutual TLS the
//! proofs add nothing the TLS session has not already shown, and claim
//! nothing more: they are there so the remote socket speaks the loopback
//! protocol byte for byte, and the page's `ipcws.ts` is its one client.
//! Nothing is sent before the server's proof verifies all the same.

use std::time::Duration;

use hmac::{KeyInit, Mac};
use serde_json::{json, Value};
use sha2::Sha256;

use crate::hosts::HostRecord;
use crate::key::DeviceKey;
use crate::pipe::{self, Inbound, Pipe, PipeRx, PipeTx};

/// Handshake v1's labels (serve's `policy::CLIENT_LABEL` / `SERVER_LABEL`; a
/// test in worktrees-cli holds them equal).
pub const CLIENT_LABEL: &str = "worktrees-serve/v1/client\n";
pub const SERVER_LABEL: &str = "worktrees-serve/v1/server\n";

type HmacSha256 = hmac::Hmac<Sha256>;

fn mac(key: &str, label: &str, cn: &str, sn: &str) -> HmacSha256 {
    let mut m = HmacSha256::new_from_slice(key.as_bytes()).expect("HMAC takes a key of any length");
    m.update(label.as_bytes());
    m.update(cn.as_bytes());
    m.update(b"\n");
    m.update(sn.as_bytes());
    m
}

fn is_hex32(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or(0)).collect()
}

/// How a call failed.
#[derive(Debug)]
pub enum CallError {
    /// The socket did not open.
    Pipe(pipe::PipeError),
    /// The server did not complete handshake v1: nothing was sent to it.
    Handshake(String),
    /// The server runs another version.
    Version { theirs: String, ours: String },
    /// The socket ended before the answer.
    Closed(String),
    TimedOut,
    /// The command's own refusal, exactly as serve sent it.
    Refused(Value),
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::Pipe(e) => write!(f, "{e}"),
            CallError::Handshake(m) => write!(f, "the host did not complete the handshake ({m}); no request was sent"),
            CallError::Version { theirs, ours } => {
                write!(f, "version mismatch: the host runs v{theirs}, this client is v{ours}; refusing to drive it")
            }
            CallError::Closed(m) => write!(f, "{m}"),
            CallError::TimedOut => write!(f, "no answer in time"),
            CallError::Refused(v) => write!(f, "{}", crate::printable(&v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))),
        }
    }
}

/// A socket that has passed handshake v1.
pub struct Session {
    pub tx: PipeTx,
    pub rx: PipeRx,
    /// The server's `ok` frame: its version and, on the remote listener, the
    /// device's own record (`device`).
    pub welcome: Value,
}

async fn next(rx: &mut PipeRx) -> Result<Value, CallError> {
    match rx.recv().await {
        Inbound::Frame(t) => serde_json::from_str(&t).map_err(|_| CallError::Handshake("an unreadable frame".into())),
        Inbound::Closed(m) => Err(CallError::Closed(m)),
    }
}

/// Open the socket and run handshake v1 over it. `version` is this client's;
/// a server that proves itself but runs another is refused.
pub async fn connect(rec: &HostRecord, key: &DeviceKey, version: &str, wait: Duration) -> Result<Session, CallError> {
    handshake(pipe::open(rec, key, wait).await.map_err(CallError::Pipe)?, version, wait).await
}

/// Handshake v1 over an open pipe, keyed with its session token.
pub async fn handshake(p: Pipe, version: &str, wait: Duration) -> Result<Session, CallError> {
    let token = p.session_token().to_string();
    let (mut tx, mut rx) = p.split();
    let shake = async {
        let mut b = [0u8; 32];
        getrandom::fill(&mut b).expect("the OS refused entropy");
        let cn = crate::tls::hex(&b);
        tx.send(json!({ "hello": cn }).to_string()).await.map_err(CallError::Closed)?;
        let c = next(&mut rx).await?;
        let sn = c.get("challenge").and_then(Value::as_str).filter(|s| is_hex32(s)).ok_or_else(|| CallError::Handshake(format!("no challenge: {c}")))?.to_string();
        let auth = crate::tls::hex(&mac(&token, CLIENT_LABEL, &cn, &sn).finalize().into_bytes());
        tx.send(json!({ "auth": auth }).to_string()).await.map_err(CallError::Closed)?;
        let ok = next(&mut rx).await?;
        let proof = ok.get("proof").and_then(Value::as_str).filter(|s| is_hex32(s)).ok_or_else(|| CallError::Handshake("no proof".into()))?;
        if ok.get("ok") != Some(&Value::Bool(true)) || mac(&token, SERVER_LABEL, &cn, &sn).verify_slice(&unhex(proof)).is_err() {
            return Err(CallError::Handshake("its proof does not verify".into()));
        }
        let theirs = ok.get("version").and_then(Value::as_str).unwrap_or("?");
        if theirs != version {
            return Err(CallError::Version { theirs: crate::printable(theirs), ours: version.to_string() });
        }
        Ok(ok)
    };
    let welcome = tokio::time::timeout(wait, shake).await.map_err(|_| CallError::TimedOut)??;
    Ok(Session { tx, rx, welcome })
}

impl Session {
    /// One request, and its answer: `Ok(ok)` or `Err(Refused(err))`. Frames
    /// for channels and events on the way are skipped.
    pub async fn request(&mut self, id: u64, cmd: &str, args: Value, wait: Duration) -> Result<Value, CallError> {
        self.tx.send(json!({ "id": id, "cmd": cmd, "args": args }).to_string()).await.map_err(CallError::Closed)?;
        let answer = async {
            loop {
                let f = match self.rx.recv().await {
                    Inbound::Frame(t) => t,
                    Inbound::Closed(m) => return Err(CallError::Closed(m)),
                };
                let Ok(v) = serde_json::from_str::<Value>(&f) else { continue };
                if v.get("id") != Some(&json!(id)) {
                    continue;
                }
                if let Some(e) = v.get("err") {
                    return Err(CallError::Refused(e.clone()));
                }
                return Ok(v.get("ok").cloned().unwrap_or(Value::Null));
            }
        };
        tokio::time::timeout(wait, answer).await.map_err(|_| CallError::TimedOut)?
    }

    /// Keep reading for `wait` after a request — the frames of a channel it
    /// opened (a terminal's bytes), events — handing each to `seen`. `Some`
    /// with the host's words if the socket ends first (a revoke says so).
    pub async fn follow(&mut self, wait: Duration, seen: &mut (dyn FnMut(&Value) + Send)) -> Option<String> {
        let read = async {
            loop {
                match self.rx.recv().await {
                    Inbound::Frame(t) => {
                        if let Ok(v) = serde_json::from_str::<Value>(&t) {
                            seen(&v);
                        }
                    }
                    Inbound::Closed(why) => return why,
                }
            }
        };
        tokio::time::timeout(wait, read).await.ok()
    }

    pub async fn close(self) {
        self.tx.close().await;
    }
}

/// Connect, run one request, close.
pub async fn call(rec: &HostRecord, key: &DeviceKey, version: &str, cmd: &str, args: Value, wait: Duration) -> Result<(Value, Value), CallError> {
    let mut s = connect(rec, key, version, wait).await?;
    let out = s.request(1, cmd, args, wait).await;
    let welcome = s.welcome.clone();
    s.close().await;
    out.map(|v| (welcome, v))
}
