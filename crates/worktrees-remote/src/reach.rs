//! Reaching a paired host: at the address the device kept, and when nothing
//! answers there, on the LAN (Phase 2 §4.1, §4.7). The CLI's `call` and the
//! shim's `remote_connect` both open through [`open`]; an address code goes
//! through [`adopt_code`].
//!
//! **Discovery chooses where to try, never whom to believe.** A discovered
//! address — or one from an address code, which anyone can print because the
//! pin is public — is only a candidate. It is dialled with the pin the
//! device already holds, and kept only once the WHOLE open succeeds: the
//! server proved the pinned key, accepted this device's, and upgraded. Then
//! the record moves, under the store's lock ([`HostStore::readdress`]). A
//! candidate that answers with another key costs one failed attempt, is
//! logged, and changes nothing: in TLS 1.3 it never sees this device's
//! certificate, and no SNI names the host to it.
//!
//! **When to look.** Only when the host is not at the kept address: nothing
//! answers there ([`PipeError::Connect`]: refused, unroutable, silent), or a
//! machine that is not the pin does ([`PipeError::NotThePin`]: the old
//! address handed to another machine, maybe another worktrees host — it saw
//! no certificate of this device's). The PINNED host refusing this device's
//! key is not a move: the device is no longer welcome there (revoked,
//! disabled, expired), and finding the host elsewhere would change nothing,
//! so no one goes looking. A round is bounded ([`ROUND`]; §4.7's ceiling is 30 s), tries at most
//! [`MAX_CANDIDATES`] distinct candidates, never the same one twice, and
//! never runs in the background: the next round is the caller's next retry
//! (the app's `reconnect.ts` backs off between them).
//!
//! Every candidate passes `netclass` before any dial ([`discover::candidate`]).

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use worktrees_core::netclass::AddrCode;
use worktrees_core::netif::Iface;

use crate::discover::{self, Browser, Skip};
use crate::hosts::{HostRecord, HostStore};
use crate::key::DeviceKey;
use crate::pipe::{self, Pipe, PipeError};

/// How long one round may look on the LAN and try what it finds.
pub const ROUND: Duration = Duration::from_secs(10);

/// How long one candidate may take, TLS and upgrade included: it is on this
/// device's own link.
pub const CANDIDATE_WAIT: Duration = Duration::from_secs(3);

/// The most distinct candidates one round dials (§4.7).
pub const MAX_CANDIDATES: usize = 4;

/// What [`open`] looks with.
#[derive(Clone)]
pub struct Env {
    pub browser: Arc<dyn Browser>,
    /// This device's interfaces, as they are at the start of a round.
    pub interfaces: Arc<dyn Fn() -> std::io::Result<Vec<Iface>> + Send + Sync>,
    /// Where a judged address is actually dialled: the address itself, except
    /// in tests, whose hosts listen on the loopback a record may not hold.
    pub route: Arc<dyn Fn(SocketAddr) -> SocketAddr + Send + Sync>,
    pub round: Duration,
    pub candidate_wait: Duration,
}

impl Env {
    /// The OS's browser and interfaces, every address dialled as itself.
    pub fn platform() -> Env {
        Env {
            browser: discover::platform(),
            interfaces: Arc::new(worktrees_core::netif::interfaces),
            route: Arc::new(|a| a),
            round: ROUND,
            candidate_wait: CANDIDATE_WAIT,
        }
    }
}

/// An open pipe, and the record it was opened for — moved, if it was found
/// somewhere new.
pub struct Reached {
    pub pipe: Pipe,
    pub rec: HostRecord,
    /// The address the record held before, when the host was found elsewhere.
    pub moved_from: Option<String>,
}

/// What the LAN said, when the kept address did not answer.
#[derive(Debug)]
pub enum Lan {
    /// The host gave this device no discovery id when it paired.
    NoId,
    /// This machine cannot browse.
    Unavailable(String),
    /// Looked for a round: each candidate tried, and how it failed.
    Looked(Vec<(SocketAddr, PipeError)>),
}

/// Why [`open`] failed.
#[derive(Debug)]
pub enum ReachError {
    /// The pinned host answered at the kept address and the open failed
    /// there — it refused this device's key, the TLS or the upgrade failed —
    /// or the record cannot be dialled: nothing was looked for.
    Pipe(PipeError),
    /// The host was not at the kept address (nothing answered, or another
    /// key did), nor on the LAN.
    NotFound { host: String, addr: String, first: String, lan: Lan },
}

impl ReachError {
    /// The first sentence a person reads: where the host was looked for, and
    /// the way back. For [`ReachError::NotFound`], ends with the address code.
    pub fn headline(&self) -> String {
        match self {
            ReachError::Pipe(e) => e.to_string(),
            ReachError::NotFound { host, addr, first, lan } => {
                let code = format!("On {host}, `worktrees serve remote status` shows its address code.");
                match lan {
                    Lan::Looked(_) => format!("Not found at {addr}, and not on the LAN. {code}"),
                    Lan::NoId => format!(
                        "Not found at {addr} ({first}), and {host} gave this machine no discovery id when it paired, so it cannot be looked for on the LAN. {code}"
                    ),
                    Lan::Unavailable(why) => format!("Not found at {addr} ({first}), and not looked for on the LAN: {why}. {code}"),
                }
            }
        }
    }

    /// What each attempt came to, after a round on the LAN; empty otherwise.
    pub fn details(&self) -> String {
        let ReachError::NotFound { host, addr, first, lan: Lan::Looked(tried) } = self else { return String::new() };
        let mut out = format!("(At {addr}: {first}.");
        for (a, e) in tried {
            match e {
                PipeError::NotThePin => out.push_str(&format!(" An advert for it at {a} was answered with another key: not {host}, so nothing was sent.")),
                e => out.push_str(&format!(" An advert for it at {a}: {e}.")),
            }
        }
        out.push(')');
        out
    }
}

impl std::fmt::Display for ReachError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.details() {
            d if d.is_empty() => write!(f, "{}", self.headline()),
            d => write!(f, "{} {d}", self.headline()),
        }
    }
}

/// Open the pipe to `rec`'s host: its kept address, then — only if the host
/// is not there (nothing answered, or another key did) — one round on the
/// LAN. A candidate found there is kept in
/// `store` once its pinned open succeeds. `wait` bounds the kept address's
/// attempt; [`Env::round`] the rest. `log` hears every candidate that was
/// not dialled for a reason worth saying, and every one that failed.
pub async fn open(store: &HostStore, rec: &HostRecord, key: &DeviceKey, env: &Env, wait: Duration, log: &mut (dyn FnMut(&str) + Send)) -> Result<Reached, ReachError> {
    rec.check().map_err(|e| ReachError::Pipe(PipeError::Record(e)))?;
    let kept = rec.socket_addr().map_err(|e| ReachError::Pipe(PipeError::Record(e)))?;
    let kept = SocketAddr::new(kept.ip, kept.port);
    let first = match pipe::open_at((env.route)(kept), &rec.pin, key, wait).await {
        Ok(pipe) => return Ok(Reached { pipe, rec: rec.clone(), moved_from: None }),
        Err(PipeError::Connect(m)) => m,
        Err(PipeError::NotThePin) => "the machine there answered with another key".into(),
        Err(e) => return Err(ReachError::Pipe(e)),
    };
    let not_found = |lan| ReachError::NotFound { host: rec.name.clone(), addr: rec.addr.clone(), first: first.clone(), lan };
    let Some(id) = rec.discovery_id.as_deref() else {
        return Err(not_found(Lan::NoId));
    };
    let ifaces = (env.interfaces)().map_err(|e| not_found(Lan::Unavailable(format!("this machine's interfaces could not be read ({e})"))))?;
    let mut rx = env.browser.browse(env.round).map_err(|why| not_found(Lan::Unavailable(why)))?;
    log(&format!("{}: not at {} ({first}); looking for it on the LAN", rec.name, rec.addr));
    let mut tried: Vec<(SocketAddr, PipeError)> = Vec::new();
    let mut seen: HashSet<SocketAddr> = HashSet::from([kept]);
    let look = async {
        while let Some(s) = rx.recv().await {
            let c = match discover::candidate(&s, id, &ifaces) {
                Ok(c) => c,
                Err(Skip::NotOurs) => continue,
                Err(skip) => {
                    log(&format!("{}: {skip}", rec.name));
                    continue;
                }
            };
            if !seen.insert(c.addr) {
                continue;
            }
            if tried.len() == MAX_CANDIDATES {
                log(&format!("{}: an advert at {} is past the {MAX_CANDIDATES} candidates a round tries; not dialled", rec.name, c.addr));
                continue;
            }
            if c.class == worktrees_core::netclass::Class::LinkLocal {
                log(&format!("{}: an advert at {} is link-local, which this client cannot dial yet; not dialled", rec.name, c.addr));
                continue;
            }
            match pipe::open_at((env.route)(c.addr), &rec.pin, key, env.candidate_wait).await {
                Ok(pipe) => return Some((c.addr, pipe)),
                Err(e) => {
                    log(&format!("{}: an advert for it at {}: {e}; the record is unchanged", rec.name, c.addr));
                    tried.push((c.addr, e));
                }
            }
        }
        None
    };
    let found = tokio::time::timeout(env.round, look).await.ok().flatten();
    let Some((at, pipe)) = found else {
        return Err(not_found(Lan::Looked(tried)));
    };
    let addr = at.to_string();
    let rec = match store.readdress(&rec.id, &rec.pin, &addr) {
        Ok(moved) => moved,
        Err(e) => {
            // The pipe is pinned and open; only the record could not follow.
            log(&format!("{}: found at {addr}, but the new address could not be kept: {e}", rec.name));
            HostRecord { addr: addr.clone(), ..rec.clone() }
        }
    };
    log(&format!("{}: found on the LAN at {addr} (was {}); the record follows", rec.name, kept));
    Ok(Reached { pipe, rec, moved_from: Some(kept.to_string()) })
}

/// An address code as a person pasted it: trimmed, then read by
/// `netclass::parse_addr_code`, whose refusals (a public address, a zone)
/// come before anything is dialled.
pub fn parse_code(code: &str) -> Result<AddrCode, String> {
    worktrees_core::netclass::parse_addr_code(code.trim()).map_err(|e| format!("not an address code: {e}"))
}

/// Follow an address code (§4.1: `worktrees-addr:v1?addr=…&pin=…`) for
/// `rec`'s host: a code for another pin is refused before anything is
/// dialled — or before this device's key is read — and the address is kept
/// only once a pinned open to it succeeds. The old address stays until then,
/// so a forged code costs one failed attempt and nothing else.
pub async fn adopt_code(store: &HostStore, rec: &HostRecord, c: &AddrCode, env: &Env, wait: Duration) -> Result<HostRecord, String> {
    if crate::tls::hex(&c.pin) != rec.pin {
        return Err(format!("that address code is for another host: its pin is not {}'s. Nothing was dialled.", rec.name));
    }
    let key = store.key_for(rec)?;
    if c.addr.needs_scope() {
        return Err(format!("{} is link-local, which this client cannot dial yet; nothing was dialled", c.addr.ip));
    }
    let at = SocketAddr::new(c.addr.ip, c.addr.port);
    let p = pipe::open_at((env.route)(at), &rec.pin, &key, wait)
        .await
        .map_err(|e| format!("{at}: {e}. {}'s address is unchanged ({}).", rec.name, rec.addr))?;
    let (tx, _rx) = p.split();
    tx.close().await;
    store.readdress(&rec.id, &rec.pin, &at.to_string())
}
