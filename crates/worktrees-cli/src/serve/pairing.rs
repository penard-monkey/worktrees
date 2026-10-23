//! The pairing window (Phase 2 §2.2–§2.4): how a device becomes one.
//!
//! **Only the host opens a window** (§11 Q5): `pair_begin` is a console row.
//! The host chooses the scope and the expiry FIRST (§2.2 step 1), and only
//! then does a window exist: a pairing listener on its own port, a one-use
//! 128-bit secret, and the link a QR carries (`worktrees_core::pairuri`).
//! One window at a time; opening a second closes the first.
//!
//! **One attempt.** The first connection is the attempt, and ANY failure
//! closes the window with no retry: a TLS handshake that fails or outlasts
//! its 5 s, a frame that is malformed or too long, a proof that does not
//! verify, a second connection, a frame after the first, the device leaving,
//! or a deadline — 120 s from opening to the frame, 60 s from the frame to
//! the host's Allow or Deny, each running out as a Deny (§2.4). A failure is
//! answered with silence; a closed window's listener is gone, so a device
//! that finds it used is refused at connect and says so in its own words.
//!
//! **The proof** (§2.2 step 4). Over TLS 1.3 pinned to the server identity
//! (the device's [`tls::PinnedServer`]), with ANY client certificate
//! ([`tls::AnyKey`], whose signature check makes the device prove it holds
//! the key it enrolls), the device sends one line:
//!
//! ```text
//! {"pair": {"name": …, "platform": …, "stepup_spki": <base64 DER> | null}, "proof": <64 hex>}
//! proof = HMAC-SHA256(secret, "worktrees-pair/v1\n" ‖ exporter ‖ device_spki_sha256
//!                             ‖ stepup_spki_sha256 ‖ name)
//! ```
//!
//! `exporter` is 32 bytes of RFC 8446 §7.5 under `EXPORTER-worktrees-pair-v1`
//! with no context; the two hashes are raw 32-byte SHA-256s, the step-up one
//! 32 zero bytes when the device has no step-up key (a desktop); `name` is its
//! UTF-8, last, so the concatenation is unambiguous. Bound to the session and
//! to the keys: it cannot be relayed into another session or replayed for
//! other keys. Checked in constant time.
//!
//! **The comparison code** (step 6): six digits, the first four bytes of
//! `HMAC-SHA256(secret, "worktrees-pair/v1/sas\n" ‖ exporter)` as a big-endian
//! integer, high bit cleared, modulo 10⁶. Both screens show it; the host's
//! Allow is the consent act, and only it writes the record.
//!
//! Then one line back: `{"pending": true}` once the proof verifies (the
//! device shows its code and waits), and finally
//! `{"paired": {device_id, principal_id, host_name, remote_addr, expiry,
//! discovery_id}}` or `{"denied": "denied" | "timeout" | "closed" | "refused"}`.
//!
//! **Nothing here binds off loopback in slice 5** (§10): a window opens only
//! where `Bind` says, and only slice 6's `remote_enable` sets it in serve.
//! Until then `pair_begin` refuses, and the tests open windows on 127.0.0.1.

use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future::{select, Either};
use hmac::{KeyInit, Mac};
use serde::Serialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{oneshot, Notify};
use worktrees_core::authz::{DeviceId, Grant, PrincipalId};
use worktrees_core::pairuri::{self, PairUri};

use super::devices::{hex, is_pin, name_ok, DeviceEvent, DeviceRegistry, Enrollment, Expiry, Platform, MAX_DEVICES};
use super::identity::Identity;
use super::remote::peer::Rule;
use super::tls;

/// A window with no attempt closes after this (§2.4).
pub const OPEN_FOR: Duration = Duration::from_secs(120);
/// From the device's frame to the host's Allow or Deny (§2.4).
pub const DECIDE_FOR: Duration = Duration::from_secs(60);
/// The TLS handshake's deadline (§6).
pub const TLS_DEADLINE: Duration = Duration::from_secs(5);
/// The longest frame a device may send, newline included.
pub const MAX_FRAME: usize = 4096;

pub const PROOF_LABEL: &[u8] = b"worktrees-pair/v1\n";
pub const SAS_LABEL: &[u8] = b"worktrees-pair/v1/sas\n";

type HmacSha256 = hmac::Hmac<Sha256>;

/// What the host chose before any QR appeared (§2.2 step 1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Offer {
    pub grant: Grant,
    pub expiry: Expiry,
}

/// Where windows open: an address of the remote listener's, on which the
/// pairing listener takes its own port, and the remote listener's address the
/// device will use afterwards. Set by slice 6's `remote_enable`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bind {
    pub ip: IpAddr,
    pub remote: SocketAddr,
    /// The interface the pairing socket is tied to, as the remote listener's
    /// is (0: untied — the tests' loopback windows).
    pub index: u32,
    /// §4.1's peer rule, as on the remote listener: a peer outside it is
    /// dropped unanswered and is NOT the attempt, so it cannot burn the
    /// window. `None` only for the tests' loopback windows.
    pub rule: Option<super::remote::peer::Rule>,
}

/// A device's request, waiting for the host (§2.2 step 6). `code` is for the
/// host's screen, never for a log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Request {
    pub name: String,
    pub platform: Platform,
    pub code: String,
    pub peer: String,
    pub grant: Grant,
    pub expiry: Expiry,
    #[serde(skip)]
    spki_sha256: String,
    #[serde(skip)]
    stepup_spki_sha256: Option<String>,
}

/// How a window ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Ended {
    /// Allowed, and the device enrolled.
    Paired,
    /// The host said no.
    Denied,
    /// No attempt (or no frame) before the window's deadline.
    Expired,
    /// No Allow or Deny within 60 s of the frame.
    Undecided,
    /// The host closed it.
    Cancelled,
    /// The host opened another.
    Superseded,
    /// Allowed, but the record could not be written (the device cap, a key
    /// already enrolled).
    Refused,
    /// The TLS handshake failed or outlasted its deadline.
    BadTls,
    /// The frame did not parse, or was too long.
    Malformed,
    /// The proof did not verify.
    BadProof,
    /// A second connection reached the listener.
    SecondConnection,
    /// The device sent a frame after the first.
    FrameAfterFirst,
    /// The device closed its connection before the host decided.
    DeviceLeft,
}

/// The window as the console sees it (`pair_status`, and the events).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Status {
    None,
    Open { uri: String, expires_at: i64 },
    Attempt { peer: String, expires_at: i64 },
    Pending { request: Request, decide_by: i64 },
    Closed { ended: Ended, device: Option<DeviceId> },
}

enum Decision {
    Paired(Value),
    Denied,
    Refused,
}

enum Phase {
    Open,
    Attempt { peer: String },
    Pending { request: Request, decide_by: i64, tx: Option<oneshot::Sender<Decision>> },
}

struct Window {
    id: u64,
    offer: Offer,
    uri: String,
    expires_at: i64,
    stop: Arc<Notify>,
    why: Arc<Mutex<Option<Ended>>>,
    phase: Phase,
}

impl Window {
    /// Close it from outside: say why, then wake whatever it waits on.
    fn stop(&self, why: Ended) {
        self.why.lock().unwrap().get_or_insert(why);
        self.stop.notify_one();
    }
}

/// The pairing windows of one serve.
pub struct Pairing {
    dir: Option<PathBuf>,
    devices: Arc<DeviceRegistry>,
    host: String,
    rt: Option<tokio::runtime::Handle>,
    bind: Mutex<Option<Bind>>,
    cur: Mutex<Option<Window>>,
    last: Mutex<Status>,
    next: AtomicU64,
    open_for: Duration,
    decide_for: Duration,
}

impl Pairing {
    /// Windows for the config dir `dir` (`None`: never opens one). Built
    /// inside the runtime its windows will run on.
    pub fn new(dir: Option<PathBuf>, devices: Arc<DeviceRegistry>) -> Pairing {
        Pairing::with_times(dir, devices, OPEN_FOR, DECIDE_FOR)
    }

    /// [`Pairing::new`] with other deadlines — the tests'.
    pub fn with_times(dir: Option<PathBuf>, devices: Arc<DeviceRegistry>, open_for: Duration, decide_for: Duration) -> Pairing {
        Pairing {
            dir,
            devices,
            host: host_name(),
            rt: tokio::runtime::Handle::try_current().ok(),
            bind: Mutex::new(None),
            cur: Mutex::new(None),
            last: Mutex::new(Status::None),
            next: AtomicU64::new(1),
            open_for,
            decide_for,
        }
    }

    /// Where windows open. `None` (the default, and all of slice 5 in serve)
    /// refuses every `pair_begin`, and closes any window that is open.
    pub fn set_bind(&self, bind: Option<Bind>) {
        if bind.is_none() {
            self.cancel_with(Ended::Cancelled);
        }
        *self.bind.lock().unwrap() = bind;
    }

    /// The window now, or how the last one ended.
    pub fn status(&self) -> Status {
        let cur = self.cur.lock().unwrap();
        match cur.as_ref() {
            None => self.last.lock().unwrap().clone(),
            Some(w) => match &w.phase {
                Phase::Open => Status::Open { uri: w.uri.clone(), expires_at: w.expires_at },
                Phase::Attempt { peer } => Status::Attempt { peer: peer.clone(), expires_at: w.expires_at },
                Phase::Pending { request, decide_by, .. } => Status::Pending { request: request.clone(), decide_by: *decide_by },
            },
        }
    }

    /// Open a window for `offer` (§2.2 step 2): the link for the QR, its
    /// deadline, and the pin. Refused past the device cap BEFORE anything
    /// opens (§2.4), and while remote access is off.
    pub fn begin(self: &Arc<Self>, offer: Offer, now: i64) -> Result<Value, String> {
        let bind = (*self.bind.lock().unwrap()).ok_or(
            "remote access is off: a device pairs over the remote listener's address, so turn remote access on first",
        )?;
        let (Some(dir), Some(rt)) = (self.dir.clone(), self.rt.clone()) else {
            return Err("this serve cannot open a pairing window".into());
        };
        let enrolled = self.devices.snapshot().ok_or_else(|| self.devices.readable().unwrap_err())?.enrolled();
        if enrolled >= MAX_DEVICES {
            return Err(format!("{MAX_DEVICES} devices are enrolled; revoke one before pairing another"));
        }
        if let Expiry::Until(at) = offer.expiry {
            if at <= now {
                return Err("the expiry date has already passed".into());
            }
        }
        let id = Identity::ensure(&dir)?;
        let cfg = tls::server_config(&id, Arc::new(tls::AnyKey))?;
        let std_listener = match bind.index {
            0 => std::net::TcpListener::bind(SocketAddr::new(bind.ip, 0)).and_then(|l| l.set_nonblocking(true).map(|()| l)),
            i => super::remote::sock::listen(bind.ip, 0, i).map(|(l, _)| l),
        }
        .map_err(|e| format!("the pairing listener: {e}"))?;
        // Zone-less: a link carries none (`pairuri::build`); a device scopes a
        // link-local dial to an interface of its own (§4.1).
        let addr = SocketAddr::new(bind.ip, std_listener.local_addr().map_err(|e| e.to_string())?.port());
        let listener = {
            let _in = rt.enter();
            tokio::net::TcpListener::from_std(std_listener).map_err(|e| e.to_string())?
        };
        let mut secret = [0u8; 16];
        getrandom::fill(&mut secret).expect("the OS refused entropy");
        // Rounded UP to the second: a link must never read as expired while
        // its window is still open.
        let expires_at = now + self.open_for.as_millis().div_ceil(1000) as i64;
        let uri = pairuri::build(&PairUri { addr, pin: id.pin().to_string(), secret, host: self.host.clone(), exp: expires_at })?;
        let wid = self.next.fetch_add(1, Ordering::SeqCst);
        let stop = Arc::new(Notify::new());
        let why = Arc::new(Mutex::new(None));
        {
            let mut cur = self.cur.lock().unwrap();
            if let Some(old) = cur.take() {
                old.stop(Ended::Superseded);
            }
            *cur = Some(Window { id: wid, offer: offer.clone(), uri: uri.clone(), expires_at, stop: stop.clone(), why: why.clone(), phase: Phase::Open });
        }
        self.devices.announce(DeviceEvent::PairingOpened {
            for_principal: PrincipalId::owner(),
            expires_at,
            grant: offer.grant.clone(),
            expiry: offer.expiry,
        });
        let deadline = tokio::time::Instant::now() + self.open_for;
        let me = self.clone();
        rt.spawn(async move {
            let (ended, device) = me.attempt(wid, listener, bind.rule, cfg, secret, deadline, stop, why).await;
            me.finish(wid, ended, device);
        });
        Ok(json!({ "uri": uri, "expires_at": expires_at, "pin": id.pin(), "host": self.host }))
    }

    /// Close the window, if one is open (`pair_cancel`). A device waiting on
    /// it is told `closed`.
    pub fn cancel(&self) -> bool {
        self.cancel_with(Ended::Cancelled)
    }

    fn cancel_with(&self, why: Ended) -> bool {
        match self.cur.lock().unwrap().as_ref() {
            Some(w) => {
                w.stop(why);
                true
            }
            None => false,
        }
    }

    /// The host's Allow or Deny for the request that is waiting (§2.2 step 6).
    /// Allow writes the record — the consent act — and the device gets its
    /// answer; the record is returned. Deny closes the window.
    pub fn decide(&self, allow: bool, now: i64) -> Result<Value, String> {
        let mut cur = self.cur.lock().unwrap();
        let none = || "no pairing request is waiting".to_string();
        let w = cur.as_mut().ok_or_else(none)?;
        let offer = w.offer.clone();
        let Phase::Pending { request, tx, .. } = &mut w.phase else { return Err(none()) };
        let tx = tx.take().ok_or("this request has already been decided")?;
        self.devices.announce(DeviceEvent::PairingDecided { allow });
        if !allow {
            let _ = tx.send(Decision::Denied);
            return Ok(Value::Null);
        }
        let bind = (*self.bind.lock().unwrap()).ok_or("remote access was turned off")?;
        let discovery = self.dir.as_deref().map(Identity::ensure).transpose()?.map(|i| i.discovery_id().to_string());
        let e = Enrollment {
            principal: PrincipalId::owner(),
            name: request.name.clone(),
            platform: request.platform,
            spki_sha256: request.spki_sha256.clone(),
            stepup_spki_sha256: request.stepup_spki_sha256.clone(),
            grant: offer.grant,
            expiry: offer.expiry,
            addr: Some(request.peer.clone()),
        };
        match self.devices.enroll(e, now) {
            Ok(rec) => {
                let answer = json!({
                    "device_id": rec.id, "principal_id": rec.principal, "host_name": self.host,
                    "remote_addr": bind.remote.to_string(), "expiry": rec.expiry, "discovery_id": discovery,
                });
                let _ = tx.send(Decision::Paired(answer));
                serde_json::to_value(rec).map_err(|e| e.to_string())
            }
            Err(e) => {
                let _ = tx.send(Decision::Refused);
                Err(e)
            }
        }
    }

    /// One window's attempt, start to end.
    #[allow(clippy::too_many_arguments)]
    async fn attempt(
        &self,
        wid: u64,
        listener: tokio::net::TcpListener,
        rule: Option<Rule>,
        cfg: Arc<rustls::ServerConfig>,
        secret: [u8; 16],
        deadline: tokio::time::Instant,
        stop: Arc<Notify>,
        why: Arc<Mutex<Option<Ended>>>,
    ) -> (Ended, Option<DeviceId>) {
        let stopped = |or: Ended| why.lock().unwrap().unwrap_or(or);
        // A peer outside §4.1's rule is dropped unanswered, and is not the
        // attempt: a stranger cannot burn the window.
        let admits = move |p: &SocketAddr| rule.is_none_or(|r| r.admits(p.ip()));
        // 1. The first connection is the attempt.
        let (tcp, peer) = loop {
            match within(&stop, deadline, listener.accept()).await {
                Some(Ok(c)) if admits(&c.1) => break c,
                Some(Ok(_)) | Some(Err(_)) => continue,
                None => return (stopped(Ended::Expired), None),
            }
        };
        let peer = peer.ip().to_string();
        self.set_phase(wid, Phase::Attempt { peer: peer.clone() });
        self.devices.announce(DeviceEvent::PairingAttempt { for_principal: PrincipalId::owner(), peer: peer.clone() });
        // 2. Any further connection ends the window. The listener lives in
        // this watcher, so ending it closes the port.
        let watcher = {
            let (stop, why) = (stop.clone(), why.clone());
            tokio::spawn(async move {
                loop {
                    match listener.accept().await {
                        Ok((_, p)) if !admits(&p) => continue,
                        Ok(_) => {
                            why.lock().unwrap().get_or_insert(Ended::SecondConnection);
                            stop.notify_one();
                        }
                        Err(_) => {}
                    }
                    break;
                }
            })
        };
        let out = self.handshake_and_wait(wid, tcp, &peer, cfg, secret, deadline, &stop, &stopped).await;
        watcher.abort();
        let _ = watcher.await;
        out
    }

    #[allow(clippy::too_many_arguments)]
    async fn handshake_and_wait(
        &self,
        wid: u64,
        tcp: tokio::net::TcpStream,
        peer: &str,
        cfg: Arc<rustls::ServerConfig>,
        secret: [u8; 16],
        deadline: tokio::time::Instant,
        stop: &Notify,
        stopped: &(dyn Fn(Ended) -> Ended + Sync),
    ) -> (Ended, Option<DeviceId>) {
        let tls_by = (tokio::time::Instant::now() + TLS_DEADLINE).min(deadline);
        let mut s = match within(stop, tls_by, tokio_rustls::TlsAcceptor::from(cfg).accept(tcp)).await {
            Some(Ok(s)) => s,
            Some(Err(_)) => return (Ended::BadTls, None),
            None => return (stopped(Ended::BadTls), None),
        };
        let (exporter, device_pin) = {
            let conn = s.get_ref().1;
            let ex = tls::exporter(conn, tls::EXPORTER_PAIR);
            let pin = conn.peer_certificates().and_then(|c| c.first()).and_then(tls::pin_of_cert);
            match (ex, pin) {
                (Ok(ex), Some(pin)) => (ex, pin),
                _ => return (Ended::BadTls, None),
            }
        };
        // 3. One frame, before the window's deadline.
        let line = match within(stop, deadline, read_frame(&mut s)).await {
            Some(Ok(l)) => l,
            Some(Err(Ended::DeviceLeft)) => return (stopped(Ended::DeviceLeft), None),
            Some(Err(e)) => return (e, None),
            None => return (stopped(Ended::Expired), None),
        };
        let Ok(frame) = parse_frame(&line) else { return (Ended::Malformed, None) };
        let stepup_pin = frame.stepup_spki.as_deref().map(|d| hex(&Sha256::digest(d)));
        if !proof_ok(&secret, &exporter, &device_pin, stepup_pin.as_deref(), &frame.name, &frame.proof) {
            return (Ended::BadProof, None);
        }
        // 4. The host decides.
        let code = comparison_code(&secret, &exporter);
        let decide_by = worktrees_core::sysclock::now_epoch() + self.decide_for.as_secs() as i64;
        let (tx, mut rx) = oneshot::channel();
        let request = {
            let mut cur = self.cur.lock().unwrap();
            let Some(w) = cur.as_mut().filter(|w| w.id == wid) else { return (stopped(Ended::Superseded), None) };
            let request = Request {
                name: frame.name.clone(),
                platform: frame.platform,
                code: code.clone(),
                peer: peer.to_string(),
                grant: w.offer.grant.clone(),
                expiry: w.offer.expiry,
                spki_sha256: device_pin,
                stepup_spki_sha256: stepup_pin,
            };
            w.phase = Phase::Pending { request: request.clone(), decide_by, tx: Some(tx) };
            request
        };
        self.devices.announce(DeviceEvent::PairingPending { name: request.name.clone(), platform: request.platform, code: request.code.clone(), peer: request.peer.clone() });
        if write_line(&mut s, &json!({ "pending": true })).await.is_err() {
            return self.give_up(wid, &mut rx, Ended::DeviceLeft).await;
        }
        let decide_until = tokio::time::Instant::now() + self.decide_for;
        let mut one = [0u8; 1];
        let decided = {
            let read = std::pin::pin!(s.read(&mut one));
            let got = std::pin::pin!(&mut rx);
            match within(stop, decide_until, select(got, read)).await {
                Some(Either::Left((d, _))) => Ok(d),
                Some(Either::Right((Ok(0) | Err(_), _))) => Err(Ended::DeviceLeft),
                Some(Either::Right((Ok(_), _))) => Err(Ended::FrameAfterFirst),
                None => Err(stopped(Ended::Undecided)),
            }
        };
        let decided = match decided {
            Ok(d) => d,
            // Taken back under the lock: if the host's decision is already in
            // flight, it wins, and the device is told it.
            Err(e) => match self.take_back(wid) {
                true => {
                    if matches!(e, Ended::Undecided | Ended::Cancelled | Ended::Superseded | Ended::SecondConnection) {
                        let word = if e == Ended::Undecided { "timeout" } else { "closed" };
                        let _ = write_line(&mut s, &json!({ "denied": word })).await;
                    }
                    return (e, None);
                }
                false => rx.await,
            },
        };
        match decided {
            Ok(Decision::Paired(answer)) => {
                let device = answer["device_id"].as_str().map(DeviceId::new);
                let _ = write_line(&mut s, &json!({ "paired": answer })).await;
                (Ended::Paired, device)
            }
            Ok(Decision::Denied) => {
                let _ = write_line(&mut s, &json!({ "denied": "denied" })).await;
                (Ended::Denied, None)
            }
            Ok(Decision::Refused) => {
                let _ = write_line(&mut s, &json!({ "denied": "refused" })).await;
                (Ended::Refused, None)
            }
            Err(_) => (Ended::Undecided, None),
        }
    }

    /// The device failed while its request waited: take the decision back
    /// from the host, unless it is already being made, in which case wait
    /// for it.
    async fn give_up(&self, wid: u64, rx: &mut oneshot::Receiver<Decision>, why: Ended) -> (Ended, Option<DeviceId>) {
        if self.take_back(wid) {
            return (why, None);
        }
        match rx.await {
            Ok(Decision::Paired(a)) => (Ended::Paired, a["device_id"].as_str().map(DeviceId::new)),
            _ => (why, None),
        }
    }

    /// Withdraw the waiting request so the host can no longer decide it.
    /// `false` when the host already took it (its decision is in flight).
    fn take_back(&self, wid: u64) -> bool {
        let mut cur = self.cur.lock().unwrap();
        match cur.as_mut().filter(|w| w.id == wid).map(|w| &mut w.phase) {
            Some(Phase::Pending { tx, .. }) => tx.take().is_some(),
            _ => true,
        }
    }

    fn set_phase(&self, wid: u64, phase: Phase) {
        if let Some(w) = self.cur.lock().unwrap().as_mut().filter(|w| w.id == wid) {
            w.phase = phase;
        }
    }

    /// The window is over: forget it (unless another replaced it) and say how.
    fn finish(&self, wid: u64, ended: Ended, device: Option<DeviceId>) {
        {
            let mut cur = self.cur.lock().unwrap();
            if cur.as_ref().is_some_and(|w| w.id == wid) {
                *cur = None;
                *self.last.lock().unwrap() = Status::Closed { ended, device: device.clone() };
            }
        }
        self.devices.announce(DeviceEvent::PairingClosed { ended, device });
    }
}

/// `f`, unless `stop` is notified or `until` passes first.
async fn within<T>(stop: &Notify, until: tokio::time::Instant, f: impl Future<Output = T>) -> Option<T> {
    let f = std::pin::pin!(f);
    let s = std::pin::pin!(stop.notified());
    let t = std::pin::pin!(tokio::time::sleep_until(until));
    match select(f, select(s, t)).await {
        Either::Left((v, _)) => Some(v),
        Either::Right(_) => None,
    }
}

/// One line, at most [`MAX_FRAME`] bytes with its newline. Nothing past the
/// limit is kept, whether or not a newline is in it yet.
async fn read_frame<S: tokio::io::AsyncRead + Unpin>(s: &mut S) -> Result<String, Ended> {
    let mut buf = Vec::with_capacity(512);
    let mut chunk = [0u8; 512];
    loop {
        let n = s.read(&mut chunk).await.map_err(|_| Ended::DeviceLeft)?;
        if n == 0 {
            return Err(Ended::DeviceLeft);
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > MAX_FRAME {
            return Err(Ended::Malformed);
        }
        if let Some(i) = buf.iter().position(|b| *b == b'\n') {
            // Anything after the newline is a second frame.
            if i + 1 != buf.len() {
                return Err(Ended::FrameAfterFirst);
            }
            buf.truncate(i);
            return String::from_utf8(buf).map_err(|_| Ended::Malformed);
        }
    }
}

async fn write_line<S: tokio::io::AsyncWrite + Unpin>(s: &mut S, v: &Value) -> std::io::Result<()> {
    let mut b = serde_json::to_vec(v).map_err(std::io::Error::other)?;
    b.push(b'\n');
    s.write_all(&b).await?;
    s.flush().await
}

/// The device's frame, read strictly (§2.2 step 4): exactly these keys, a
/// name the dialog can show, a known platform, and a step-up key exactly
/// when the platform has one — an iPhone or iPad enrolls one, a desktop has
/// none — which must be a P-256 key, the only kind a step-up verifies.
#[derive(Debug)]
pub struct Frame {
    pub name: String,
    pub platform: Platform,
    pub stepup_spki: Option<Vec<u8>>,
    pub proof: [u8; 32],
}

pub fn parse_frame(line: &str) -> Result<Frame, String> {
    let v: Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
    let top = only(&v, &["pair", "proof"])?;
    let pair = only(&top["pair"], &["name", "platform", "stepup_spki"])?;
    let name = pair["name"].as_str().ok_or("name")?.to_string();
    name_ok(&name)?;
    let platform = pair["platform"].as_str().and_then(Platform::parse).ok_or("platform")?;
    let stepup_spki = match &pair["stepup_spki"] {
        Value::Null => None,
        Value::String(b) => Some(super::stepup::b64_decode(b).ok_or("stepup_spki is not base64")?),
        _ => return Err("stepup_spki".into()),
    };
    if platform.has_presence_key() != stepup_spki.is_some() {
        return Err("a step-up key comes with an iPhone or iPad, and only with one".into());
    }
    if let Some(d) = &stepup_spki {
        super::stepup::p256_point(d).ok_or("the step-up key is not a P-256 key")?;
    }
    let proof = top["proof"].as_str().filter(|p| is_pin(p)).and_then(unhex32).ok_or("proof")?;
    Ok(Frame { name, platform, stepup_spki, proof })
}

/// `v` is an object with exactly `keys`.
fn only<'a>(v: &'a Value, keys: &[&str]) -> Result<&'a Map<String, Value>, String> {
    let o = v.as_object().ok_or("not an object")?;
    if o.len() != keys.len() || !keys.iter().all(|k| o.contains_key(*k)) {
        return Err(format!("expected exactly {keys:?}"));
    }
    Ok(o)
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

/// The proof the device must have sent, checked in constant time (§2.2 step
/// 5).
pub fn proof_ok(secret: &[u8; 16], exporter: &[u8; 32], device_pin: &str, stepup_pin: Option<&str>, name: &str, proof: &[u8; 32]) -> bool {
    let (Some(d), Some(s)) = (unhex32(device_pin), stepup_pin.map(unhex32).unwrap_or(Some([0u8; 32]))) else { return false };
    let mut m = HmacSha256::new_from_slice(secret).expect("HMAC takes a key of any length");
    m.update(PROOF_LABEL);
    m.update(exporter);
    m.update(&d);
    m.update(&s);
    m.update(name.as_bytes());
    m.verify_slice(proof).is_ok()
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

/// This host's name, for the link and the device's screen: the system host
/// name without `.local`, or a constant when it would not pass the
/// display-name rule.
fn host_name() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: a stack buffer and its length.
    let raw = if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0 {
        String::from_utf8_lossy(&buf[..buf.iter().position(|b| *b == 0).unwrap_or(buf.len())]).into_owned()
    } else {
        String::new()
    };
    let name = raw.strip_suffix(".local").unwrap_or(&raw).to_string();
    if name_ok(&name).is_ok() {
        name
    } else {
        "worktrees host".to_string()
    }
}
