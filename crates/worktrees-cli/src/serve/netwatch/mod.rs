//! The remote listener's address rules over time: what serve does when the
//! address it bound leaves, comes back, or moves (Server Edition Phase 2,
//! `docs/proposals/server-edition-phase2.md` §4.1 "When the address changes",
//! §11.2 N4, §12.3, §12.4 T2/T6).
//!
//! Pure: [`Watch::poll`] takes the time and a [`Snapshot`] of the host's
//! interfaces and routers ([`read::snapshot`]) and RETURNS what to do as
//! [`Action`]s; it binds nothing and closes nothing. Slice 6 proper runs it on
//! the host poll (3 s) and performs the actions. Nothing in the kernel does
//! this for serve: on Linux 6.12 a listener and its connections survive their
//! address's removal, a re-added address is served by the OLD listener, and an
//! address moved to another interface keeps being served there (measured,
//! network spike 2026-10-21). So the poll checks that the address is on the
//! interface it was bound on, by name AND index (a device-bound socket is
//! stranded on a dead index when the interface is re-created), and a resume
//! is a NEW socket after the old one is closed — the old one still holds the
//! port.
//!
//! **The fingerprint** says what "the same network" means ([`Fingerprint`]).
//! For a point-to-point interface it is the address, its prefix and the flag,
//! and NOT the name: macOS numbers `utun`s in creation order, so Tailscale's
//! can be `utun3` today and `utun5` after a reboot. For anything else it is
//! the interface, the subnet and the interface's OWN router with its
//! link-layer address, where readable — not "the" default route, which a VPN
//! or an exit node takes over. **An absent router or link-layer address is
//! "not known yet" and never a change** (§12.4 T6): on Linux the neighbour
//! entry is missing before the first packet to the gateway and after a flush,
//! and an alert that fires on a quiet network teaches the click that matters.
//! Only a present, different address is a change. What "not known yet" does
//! depend on what serve is doing: a live bind keeps serving through it, and a
//! paused one WAITS for it before it resumes or moves, rather than resuming on
//! a network it cannot yet tell from a café's (both silent: no dialog, no
//! alert).
//!
//! **The other way round, a field the RECORD lacks vouches for nothing.** An
//! enable on an idle Linux host records no router link-layer address, and a
//! café that also hands out 192.168.1.5 with its router at .1 would otherwise
//! match it. Where neither side has the field (a routerless cable) it is the
//! same; where the network now shows one the record does not hold, nothing
//! resumes or moves by itself and a bind serve resumed on its own pauses —
//! the host is asked once ([`Question::CannotConfirm`]). The record is
//! completed ([`Action::Learn`]) only on the bind the host itself just chose
//! with `remote_enable`, before anything has paused it: never after a resume,
//! a move or a restart, which would write whatever network serve happened to
//! be on into "home". (Found by an automated review of the first cut, which
//! resumed on such a café and learned its router.)
//!
//! **The transitions** (§4.1):
//! - the address leaves its interface, the interface goes down, or is
//!   re-created → [`Action::Pause`];
//! - the same address returns with the same fingerprint → [`Action::Resume`],
//!   no dialog;
//! - LAN only: the address is gone and the interface holds a new one in the
//!   recorded subnet, the fingerprint the same → [`Action::Move`], at most
//!   [`MOVES_PER_HOUR`] in any hour; a further one pauses and asks
//!   ([`Question::TooManyMoves`], with an alert) and the watch then moves no
//!   more until the host chooses or the recorded address returns (§12.3 R1);
//! - anything else (another subnet, the address on another interface, a
//!   network whose fingerprint differs) → paused, and [`Action::Ask`]. When
//!   ONLY the gateway's link-layer address changed, the question is its own,
//!   [`Question::GatewayChanged`], with an alert (§12.3 R2).
//! - an address missing at startup is the paused state, said once.
//!
//! Wired by slice 6 proper; until then only the tests below call it.
#![allow(dead_code)]

pub mod read;

use std::collections::VecDeque;
use std::fmt;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use worktrees_core::netclass::{self, Bound, Class};
use worktrees_core::netif::{network, IfAddr, Iface};

/// A link-layer address (Ethernet's six bytes, usually).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct LinkAddr(pub Vec<u8>);

impl LinkAddr {
    /// `d8:44:89:a:b:c` — colon-separated groups of one or two hex digits,
    /// as both `scutil` and `ndp` print them. Anything else is `None`.
    pub fn parse(s: &str) -> Option<LinkAddr> {
        let mut v = Vec::new();
        for g in s.split(':') {
            if g.is_empty() || g.len() > 2 || !g.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            v.push(u8::from_str_radix(g, 16).ok()?);
        }
        (2..=20).contains(&v.len()).then_some(LinkAddr(v))
    }
}

impl fmt::Display for LinkAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = self.0.iter().map(|b| format!("{b:02x}")).collect();
        f.write_str(&parts.join(":"))
    }
}

impl fmt::Debug for LinkAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// An interface's own default router, and its link-layer address where the
/// platform lets an unprivileged process read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Router {
    pub ip: IpAddr,
    pub lladdr: Option<LinkAddr>,
}

/// One interface's router for one family (the family is the router's).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IfRouter {
    pub iface: String,
    pub router: Router,
}

/// What one poll reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub ifaces: Vec<Iface>,
    pub routers: Vec<IfRouter>,
}

impl Snapshot {
    fn iface(&self, name: &str) -> Option<&Iface> {
        self.ifaces.iter().find(|i| i.name == name)
    }

    fn router(&self, iface: &str, v4: bool) -> Option<&Router> {
        self.routers.iter().find(|r| r.iface == iface && r.router.ip.is_ipv4() == v4).map(|r| &r.router)
    }

    /// What `addr` on `iface` looks like as a fingerprint right now.
    fn fingerprint(&self, iface: &Iface, addr: &IfAddr) -> Fingerprint {
        if iface.point_to_point {
            Fingerprint::PointToPoint { addr: addr.ip, prefix: addr.prefix }
        } else {
            Fingerprint::Lan {
                iface: iface.name.clone(),
                net: network(addr.ip, addr.prefix),
                prefix: addr.prefix,
                router: self.router(&iface.name, addr.ip.is_ipv4()).cloned(),
            }
        }
    }
}

/// "The same network", as recorded in `serve-remote.json` at enable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fingerprint {
    /// An overlay: the overlay assigns the address, and it is this node's
    /// identity there. No interface name (see the module doc).
    PointToPoint { addr: IpAddr, prefix: u8 },
    /// Anything else: the interface, the subnet (its network and length, not
    /// the host's own address in it), and the interface's own router.
    Lan { iface: String, net: IpAddr, prefix: u8, router: Option<Router> },
}

/// How a fingerprint seen now compares with the recorded one.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Match {
    Same,
    /// Something recorded is not readable right now (a router, or its
    /// link-layer address). Never a change.
    Unknown,
    /// The network shows a router, or its link-layer address, that the
    /// record does not hold, so the record cannot vouch for it.
    Unrecorded,
    /// Same interface, same subnet, same router address: only the router's
    /// link-layer address differs. What ARP spoofing of the gateway looks like.
    GatewayChanged { router: IpAddr, was: LinkAddr, now: LinkAddr },
    Different,
}

fn compare(rec: &Fingerprint, now: &Fingerprint) -> Match {
    use Fingerprint::*;
    match (rec, now) {
        (PointToPoint { addr: a, prefix: p }, PointToPoint { addr: b, prefix: q }) => {
            if a == b && p == q {
                Match::Same
            } else {
                Match::Different
            }
        }
        (Lan { iface: i, net: n, prefix: p, router: r }, Lan { iface: j, net: m, prefix: q, router: s }) => {
            if i != j || n != m || p != q {
                return Match::Different;
            }
            match (r, s) {
                (None, None) => Match::Same,
                (None, Some(_)) => Match::Unrecorded,
                (Some(_), None) => Match::Unknown,
                (Some(r), Some(s)) if r.ip != s.ip => Match::Different,
                (Some(r), Some(s)) => match (&r.lladdr, &s.lladdr) {
                    (Some(a), Some(b)) if a != b => Match::GatewayChanged { router: r.ip, was: a.clone(), now: b.clone() },
                    (Some(_), None) => Match::Unknown,
                    (None, Some(_)) => Match::Unrecorded,
                    _ => Match::Same,
                },
            }
        }
        _ => Match::Different,
    }
}

impl Fingerprint {
    /// The recorded fingerprint with what it lacked filled in from `now`, when
    /// `now` matches it and knows more: a router where none was recorded, or
    /// the router's link-layer address.
    fn completed_by(&self, now: &Fingerprint) -> Option<Fingerprint> {
        let (Fingerprint::Lan { iface, net, prefix, router: rec }, Fingerprint::Lan { router: Some(cur), .. }) = (self, now) else {
            return None;
        };
        let fuller = match rec {
            None => cur.clone(),
            Some(r) if r.ip == cur.ip && r.lladdr.is_none() && cur.lladdr.is_some() => cur.clone(),
            Some(_) => return None,
        };
        Some(Fingerprint::Lan { iface: iface.clone(), net: *net, prefix: *prefix, router: Some(fuller) })
    }
}

/// What serve-remote.json records about the bind (`{addr, iface, network}`;
/// the port is the listener's own business).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub addr: IpAddr,
    /// The interface at enable. For an overlay this is informational: the
    /// address finds its interface, whatever it is called now.
    pub iface: String,
    pub fingerprint: Fingerprint,
}

impl Record {
    /// At enable, from [`netclass::at_bind`]'s answer and the same poll's
    /// snapshot.
    pub fn enable(bound: &Bound, snap: &Snapshot) -> Record {
        let addr = IfAddr { ip: bound.ip, prefix: bound.prefix, temporary: false, deprecated: false };
        let iface = Iface {
            name: bound.iface.clone(),
            index: bound.index,
            up: true,
            running: true,
            loopback: false,
            point_to_point: bound.point_to_point,
            addrs: vec![addr],
        };
        Record { addr: bound.ip, iface: bound.iface.clone(), fingerprint: snap.fingerprint(&iface, &addr) }
    }
}

/// A live bind: the address on an interface, by name and index. The socket
/// is bound to the device (`SO_BINDTODEVICE`, `IP_BOUND_IF`), which pins the
/// index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub ip: IpAddr,
    pub iface: String,
    pub index: u32,
}

/// Why the listener paused, for the audit record and the host's banner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Why {
    /// "Remote access paused: 192.168.1.5 is no longer on en0" — a lease
    /// moved, Wi-Fi dropped, the overlay went down, the machine slept.
    AddressLeft { addr: IpAddr, iface: String },
    /// The interface was re-created under the listener (its index changed).
    InterfaceReplaced { iface: String, was: u32, now: u32 },
    /// The address is still here, but the network around it is not the
    /// recorded one.
    NetworkChanged { addr: IpAddr, iface: String },
    /// The address is here and the network not confirmed: its router, or the
    /// router's link-layer address, is not readable yet, or the network shows
    /// one the record does not hold.
    Unconfirmed { addr: IpAddr, iface: String },
}

/// A question for the host. The dialog's answers are "Use this address"
/// (`remote_enable` again, which re-runs the classifier and records a new
/// fingerprint) and "Keep it off".
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Question {
    /// "This Mac is now 192.168.4.20 on en0, on a different network."
    DifferentNetwork { addr: IpAddr, iface: String },
    /// The recorded address is on another interface now.
    OtherInterface { addr: IpAddr, iface: String },
    /// "The router's hardware address changed. This can mean a different
    /// network, or someone on this one pretending to be the router."
    GatewayChanged { iface: String, router: IpAddr, was: LinkAddr, now: LinkAddr },
    /// A move past [`MOVES_PER_HOUR`]: a rogue DHCP server can make the
    /// listener flap, dropping every remote connection each time.
    TooManyMoves { from: IpAddr, to: IpAddr },
    /// "This Mac is on 192.168.1.5 on en0, but its network could not be
    /// confirmed when remote access was turned on." The record lacks the
    /// router, or its hardware address, that the network now shows.
    CannotConfirm { addr: IpAddr, iface: String },
}

impl Question {
    /// Raises a host alert (§5.3) as well as the dialog.
    pub fn alerts(&self) -> bool {
        matches!(self, Question::GatewayChanged { .. } | Question::TooManyMoves { .. })
    }
}

/// What the poll asks the listener to do. The executor performs each in
/// order; the audit vocabulary is the proposal's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// The first bind of this process, on the recorded address.
    Start(Binding),
    /// Close the remote listener, every remote connection and any pairing
    /// window; free the mDNS registration (Avahi re-announces by itself when
    /// the address returns otherwise, §12.4 T2). Audit `remote_paused` with
    /// the reason. `starting`: serve came up paused, logged once per episode.
    Pause { why: Why, starting: bool },
    /// Bind a NEW socket on the binding (the old one, if any, is closed
    /// first). Audit `remote_resumed`. Any open question is withdrawn.
    Resume(Binding),
    /// Close the old listener if one is open, bind `to`, close any pairing
    /// window (its QR names the old address), withdraw and re-register the
    /// advert; audit `bind moved` with both addresses and persist `to.ip` as
    /// the recorded address. Any open question is withdrawn.
    Move { from: IpAddr, to: Binding },
    /// Show the question; raise an alert when [`Question::alerts`].
    Ask(Question),
    /// The open question no longer describes anything; take it down.
    Retract,
    /// Persist this fingerprint: the recorded one, completed on the bind the
    /// host just confirmed (module doc).
    Learn(Fingerprint),
}

/// §11.2 N4 / §12.3 R1: automatic moves in any rolling hour.
pub const MOVES_PER_HOUR: usize = 3;
pub const MOVE_WINDOW: Duration = Duration::from_secs(3600);

#[derive(Clone, Debug, PartialEq, Eq)]
enum State {
    /// Before the first poll.
    Starting,
    Listening(Binding),
    /// `asked`: the open question, so it is asked once and not every poll.
    Paused { asked: Option<Question> },
}

/// What the snapshot says about the recorded bind.
#[derive(Debug)]
enum Seen {
    /// The recorded address on its interface (LAN: by name; overlay: on
    /// whichever point-to-point interface holds it).
    Held(Binding, Match),
    /// LAN: the address is gone from its interface, which holds this one in
    /// the recorded subnet.
    Candidate(Binding, Match),
    /// Something, but not ours.
    Foreign(Question),
    Nothing,
}

/// One remote bind's watch. Built at startup, and again by `remote_enable`;
/// the kill switch drops it.
#[derive(Debug)]
pub struct Watch {
    record: Record,
    state: State,
    /// When each automatic move happened, inside the window.
    moves: VecDeque<Instant>,
    /// A move was refused for the limit: none happen automatically until the
    /// host chooses (a new watch) or the recorded address returns.
    held_for_host: bool,
    /// Bound where the host just chose, and nothing has paused or moved it
    /// since: the only state in which the record may be completed.
    confirmed: bool,
}

impl Watch {
    /// `remote_enable`: the host chose this bind just now.
    pub fn enabled(record: Record) -> Watch {
        Watch { record, state: State::Starting, moves: VecDeque::new(), held_for_host: false, confirmed: true }
    }

    /// Serve starting on the bind `serve-remote.json` records: on whatever
    /// network the machine is on now, which the host has not looked at.
    pub fn restored(record: Record) -> Watch {
        Watch { record, state: State::Starting, moves: VecDeque::new(), held_for_host: false, confirmed: false }
    }

    /// The record as it stands: a move changes its address, a learn its
    /// fingerprint.
    pub fn record(&self) -> &Record {
        &self.record
    }

    pub fn poll(&mut self, now: Instant, snap: &Snapshot) -> Vec<Action> {
        let seen = self.see(snap);
        let mut out = Vec::new();
        match seen {
            Seen::Held(b, Match::Same) => self.listen(b, &mut out),
            Seen::Held(b, Match::Unknown) => match &self.state {
                State::Listening(cur) if *cur == b => {}
                State::Listening(cur) => {
                    let why = Why::InterfaceReplaced { iface: cur.iface.clone(), was: cur.index, now: b.index };
                    self.pause(why, &mut out)
                }
                _ => self.pause(Why::Unconfirmed { addr: b.ip, iface: b.iface.clone() }, &mut out),
            },
            Seen::Held(b, Match::Unrecorded) => {
                let on_the_hosts_bind = self.confirmed
                    && match &self.state {
                        State::Starting => true,
                        State::Listening(cur) => *cur == b,
                        State::Paused { .. } => false,
                    };
                if on_the_hosts_bind {
                    self.listen(b, &mut out);
                    self.learn(snap, &mut out);
                } else {
                    let q = Question::CannotConfirm { addr: b.ip, iface: b.iface.clone() };
                    self.ask(Why::Unconfirmed { addr: b.ip, iface: b.iface }, q, &mut out)
                }
            }
            Seen::Held(b, Match::GatewayChanged { router, was, now }) => {
                let q = Question::GatewayChanged { iface: b.iface.clone(), router, was, now };
                self.ask(Why::NetworkChanged { addr: b.ip, iface: b.iface }, q, &mut out)
            }
            Seen::Held(b, Match::Different) => {
                let q = Question::DifferentNetwork { addr: b.ip, iface: b.iface.clone() };
                self.ask(Why::NetworkChanged { addr: b.ip, iface: b.iface }, q, &mut out)
            }
            Seen::Candidate(b, m) => {
                let left = self.left();
                match m {
                    Match::Same => self.try_move(now, b, &mut out),
                    Match::Unknown => self.pause(left, &mut out),
                    Match::Unrecorded => {
                        let q = Question::CannotConfirm { addr: b.ip, iface: b.iface };
                        self.ask(left, q, &mut out)
                    }
                    Match::GatewayChanged { router, was, now } => {
                        let q = Question::GatewayChanged { iface: b.iface, router, was, now };
                        self.ask(left, q, &mut out)
                    }
                    Match::Different => {
                        let q = Question::DifferentNetwork { addr: b.ip, iface: b.iface };
                        self.ask(left, q, &mut out)
                    }
                }
            }
            Seen::Foreign(q) => {
                let left = self.left();
                self.ask(left, q, &mut out)
            }
            Seen::Nothing => {
                let left = self.left();
                self.pause(left, &mut out)
            }
        }
        out
    }

    fn see(&self, snap: &Snapshot) -> Seen {
        let rec = &self.record;
        let binding = |i: &Iface, ip: IpAddr| Binding { ip, iface: i.name.clone(), index: i.index };
        if let Fingerprint::PointToPoint { .. } = rec.fingerprint {
            return match snap.ifaces.iter().filter(|i| i.usable()).find_map(|i| i.holds(rec.addr).map(|a| (i, a))) {
                Some((i, a)) => Seen::Held(binding(i, rec.addr), compare(&rec.fingerprint, &snap.fingerprint(i, a))),
                None => Seen::Nothing,
            };
        }
        let elsewhere = || {
            snap.ifaces
                .iter()
                .filter(|i| i.usable() && i.name != rec.iface)
                .find(|i| i.holds(rec.addr).is_some())
                .map(|i| Seen::Foreign(Question::OtherInterface { addr: rec.addr, iface: i.name.clone() }))
        };
        let Some(iface) = snap.iface(&rec.iface).filter(|i| i.usable()) else {
            return elsewhere().unwrap_or(Seen::Nothing);
        };
        if let Some(a) = iface.holds(rec.addr) {
            return Seen::Held(binding(iface, rec.addr), compare(&rec.fingerprint, &snap.fingerprint(iface, a)));
        }
        // Addresses of the same kind as the recorded one (family, and
        // link-local or not), stable ones only: a Wi-Fi that fell back to a
        // self-assigned 169.254 address is not "a different network" to ask
        // about, and neither is a temporary address that rotates by itself.
        let kind = |a: &&IfAddr| {
            a.ip.is_ipv4() == rec.addr.is_ipv4()
                && is_link_local(a.ip) == is_link_local(rec.addr)
                && !a.temporary
                && !a.deprecated
        };
        let Fingerprint::Lan { net, prefix, .. } = &rec.fingerprint else { unreachable!() };
        let candidate = iface
            .addrs
            .iter()
            .filter(kind)
            .find(|a| a.prefix == *prefix && network(a.ip, a.prefix) == *net && netclass::check_held(a).is_ok());
        if let Some(a) = candidate {
            return Seen::Candidate(binding(iface, a.ip), compare(&rec.fingerprint, &snap.fingerprint(iface, a)));
        }
        if let Some(found) = elsewhere() {
            return found;
        }
        match iface.addrs.iter().find(kind) {
            Some(a) => Seen::Foreign(Question::DifferentNetwork { addr: a.ip, iface: iface.name.clone() }),
            None => Seen::Nothing,
        }
    }

    /// The pause reason when the recorded address is not where it was.
    fn left(&self) -> Why {
        let iface = match &self.state {
            State::Listening(b) => b.iface.clone(),
            _ => self.record.iface.clone(),
        };
        Why::AddressLeft { addr: self.record.addr, iface }
    }

    fn listen(&mut self, b: Binding, out: &mut Vec<Action>) {
        match &self.state {
            State::Listening(cur) if *cur == b => {}
            State::Listening(cur) => {
                let why = Why::InterfaceReplaced { iface: cur.iface.clone(), was: cur.index, now: b.index };
                out.push(Action::Pause { why, starting: false });
                out.push(Action::Resume(b.clone()));
                self.confirmed = false;
            }
            State::Starting => out.push(Action::Start(b.clone())),
            State::Paused { .. } => out.push(Action::Resume(b.clone())),
        }
        self.held_for_host = false;
        self.state = State::Listening(b);
    }

    fn learn(&mut self, snap: &Snapshot, out: &mut Vec<Action>) {
        let State::Listening(b) = &self.state else { return };
        let (Some(iface), true) = (snap.iface(&b.iface), matches!(self.record.fingerprint, Fingerprint::Lan { .. })) else {
            return;
        };
        let Some(a) = iface.holds(b.ip) else { return };
        if let Some(fp) = self.record.fingerprint.completed_by(&snap.fingerprint(iface, a)) {
            self.record.fingerprint = fp.clone();
            out.push(Action::Learn(fp));
        }
    }

    fn try_move(&mut self, now: Instant, to: Binding, out: &mut Vec<Action>) {
        while self.moves.front().is_some_and(|t| now.duration_since(*t) >= MOVE_WINDOW) {
            self.moves.pop_front();
        }
        if self.held_for_host || self.moves.len() >= MOVES_PER_HOUR {
            self.held_for_host = true;
            let left = self.left();
            self.ask(left, Question::TooManyMoves { from: self.record.addr, to: to.ip }, out);
            return;
        }
        self.moves.push_back(now);
        out.push(Action::Move { from: self.record.addr, to: to.clone() });
        self.record.addr = to.ip;
        self.state = State::Listening(to);
        self.confirmed = false;
    }

    fn pause(&mut self, why: Why, out: &mut Vec<Action>) {
        self.confirmed = false;
        match &self.state {
            State::Listening(_) => out.push(Action::Pause { why, starting: false }),
            State::Starting => out.push(Action::Pause { why, starting: true }),
            State::Paused { asked: Some(_) } => out.push(Action::Retract),
            State::Paused { asked: None } => {}
        }
        self.state = State::Paused { asked: None };
    }

    fn ask(&mut self, why: Why, q: Question, out: &mut Vec<Action>) {
        self.confirmed = false;
        match &self.state {
            State::Listening(_) => out.push(Action::Pause { why, starting: false }),
            State::Starting => out.push(Action::Pause { why, starting: true }),
            State::Paused { asked } if asked.as_ref() == Some(&q) => return,
            State::Paused { .. } => {}
        }
        out.push(Action::Ask(q.clone()));
        self.state = State::Paused { asked: Some(q) };
    }
}

fn is_link_local(ip: IpAddr) -> bool {
    netclass::classify(ip) == Ok(Class::LinkLocal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use worktrees_core::netclass::{at_bind, parse_bind};

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn mac(s: &str) -> LinkAddr {
        LinkAddr::parse(s).unwrap()
    }

    /// `"192.168.1.5/24"`, or `"fd00::9/64 temporary"`.
    fn addr(s: &str) -> IfAddr {
        let (a, flag) = s.split_once(' ').unwrap_or((s, ""));
        let (a, p) = a.split_once('/').unwrap();
        IfAddr { ip: ip(a), prefix: p.parse().unwrap(), temporary: flag == "temporary", deprecated: flag == "deprecated" }
    }

    fn iface(name: &str, index: u32, ptp: bool, addrs: &[&str]) -> Iface {
        Iface { name: name.into(), index, up: true, running: true, loopback: false, point_to_point: ptp, addrs: addrs.iter().map(|a| addr(a)).collect() }
    }

    fn lan(name: &str, index: u32, addrs: &[&str]) -> Iface {
        iface(name, index, false, addrs)
    }

    fn snap(ifaces: Vec<Iface>, routers: &[(&str, &str, Option<&str>)]) -> Snapshot {
        let routers = routers
            .iter()
            .map(|(i, r, l)| IfRouter { iface: i.to_string(), router: Router { ip: ip(r), lladdr: l.map(mac) } })
            .collect();
        Snapshot { ifaces, routers }
    }

    const GW: &str = "aa:bb:cc:00:00:01";

    /// Home: en0 (index 14) on 192.168.1.0/24, router .1 with a known lladdr.
    fn home(addrs: &[&str], lladdr: Option<&str>) -> Snapshot {
        snap(vec![lan("en0", 14, addrs)], &[("en0", "192.168.1.1", lladdr)])
    }

    /// The enable path, for real: parse, the bind-time check, the record.
    fn enabled(addr: &str, iface: &str, s: &Snapshot) -> Watch {
        let bound = at_bind(&parse_bind(addr, iface).unwrap(), &s.ifaces).unwrap();
        Watch::enabled(Record::enable(&bound, s))
    }

    fn b(ip_: &str, iface: &str, index: u32) -> Binding {
        Binding { ip: ip(ip_), iface: iface.into(), index }
    }

    fn left(a: &str, i: &str) -> Why {
        Why::AddressLeft { addr: ip(a), iface: i.into() }
    }

    fn pause(why: Why) -> Action {
        Action::Pause { why, starting: false }
    }

    fn mins(t0: Instant, m: u64) -> Instant {
        t0 + Duration::from_secs(m * 60)
    }

    #[test]
    fn the_first_poll_starts_on_the_recorded_address() {
        let s = home(&["192.168.1.5/24"], Some(GW));
        let mut w = enabled("192.168.1.5", "en0", &s);
        assert_eq!(w.poll(Instant::now(), &s), [Action::Start(b("192.168.1.5", "en0", 14))]);
        assert_eq!(w.poll(Instant::now(), &s), []);
    }

    /// §4.1: "An address missing at startup is the paused state … serve logs
    /// the state once per episode."
    #[test]
    fn an_address_missing_at_startup_pauses_once_per_episode() {
        let t = Instant::now();
        let mut w = enabled("192.168.1.5", "en0", &home(&["192.168.1.5/24"], Some(GW)));
        let gone = home(&[], Some(GW));
        assert_eq!(w.poll(t, &gone), [Action::Pause { why: left("192.168.1.5", "en0"), starting: true }]);
        assert_eq!(w.poll(t, &gone), []);
        assert_eq!(w.poll(t, &gone), []);
        assert_eq!(w.poll(t, &home(&["192.168.1.5/24"], Some(GW))), [Action::Resume(b("192.168.1.5", "en0", 14))]);
    }

    /// The spike measured Linux serving a re-added address with the OLD
    /// listener; a resume is a NEW socket, and the pause comes first.
    #[test]
    fn the_address_leaving_pauses_and_its_return_resumes() {
        let t = Instant::now();
        let s = home(&["192.168.1.5/24"], Some(GW));
        let mut w = enabled("192.168.1.5", "en0", &s);
        w.poll(t, &s);
        assert_eq!(w.poll(t, &home(&[], Some(GW))), [pause(left("192.168.1.5", "en0"))]);
        assert_eq!(w.poll(t, &home(&[], None)), []);
        assert_eq!(w.poll(t, &s), [Action::Resume(b("192.168.1.5", "en0", 14))]);
    }

    /// Linux keeps the addresses of an interface that is down.
    #[test]
    fn an_interface_that_is_down_counts_as_the_address_leaving() {
        let t = Instant::now();
        let s = home(&["192.168.1.5/24"], Some(GW));
        let mut w = enabled("192.168.1.5", "en0", &s);
        w.poll(t, &s);
        let mut down = s.clone();
        down.ifaces[0].running = false;
        assert_eq!(w.poll(t, &down), [pause(left("192.168.1.5", "en0"))]);
        let mut gone = s.clone();
        gone.ifaces.clear();
        assert_eq!(w.poll(t, &gone), []);
        assert_eq!(w.poll(t, &s), [Action::Resume(b("192.168.1.5", "en0", 14))]);
    }

    /// The spike: `SO_BINDTODEVICE` pins the index, and a listener on a
    /// re-created interface of the same name is stranded on the dead one.
    #[test]
    fn a_re_created_interface_pauses_and_resumes_on_its_new_index() {
        let t = Instant::now();
        let s = home(&["192.168.1.5/24"], Some(GW));
        let mut w = enabled("192.168.1.5", "en0", &s);
        w.poll(t, &s);
        let mut again = s.clone();
        again.ifaces[0].index = 15;
        assert_eq!(
            w.poll(t, &again),
            [pause(Why::InterfaceReplaced { iface: "en0".into(), was: 14, now: 15 }), Action::Resume(b("192.168.1.5", "en0", 15))]
        );
        assert_eq!(w.poll(t, &again), []);
    }

    /// The spike: on Linux a listener keeps serving its address after it
    /// moves to ANOTHER interface. The watch does not follow it there.
    #[test]
    fn the_address_on_another_interface_pauses_and_asks() {
        let t = Instant::now();
        let s = snap(vec![lan("en0", 14, &["192.168.1.5/24"]), lan("en1", 16, &[])], &[("en0", "192.168.1.1", Some(GW))]);
        let mut w = enabled("192.168.1.5", "en0", &s);
        w.poll(t, &s);
        let moved = snap(vec![lan("en0", 14, &[]), lan("en1", 16, &["192.168.1.5/24"])], &[("en1", "192.168.1.1", Some(GW))]);
        assert_eq!(
            w.poll(t, &moved),
            [pause(left("192.168.1.5", "en0")), Action::Ask(Question::OtherInterface { addr: ip("192.168.1.5"), iface: "en1".into() })]
        );
    }

    /// §11.2 N4: a moved DHCP lease — same interface, same subnet, same
    /// gateway — rebinds by itself, and the record follows.
    #[test]
    fn a_new_address_on_the_same_network_moves_the_bind() {
        let t = Instant::now();
        let s = home(&["192.168.1.5/24"], Some(GW));
        let mut w = enabled("192.168.1.5", "en0", &s);
        w.poll(t, &s);
        let moved = home(&["192.168.1.77/24"], Some(GW));
        assert_eq!(w.poll(t, &moved), [Action::Move { from: ip("192.168.1.5"), to: b("192.168.1.77", "en0", 14) }]);
        assert_eq!(w.record().addr, ip("192.168.1.77"));
        assert_eq!(w.poll(t, &moved), []);
        assert_eq!(w.poll(t, &home(&[], Some(GW))), [pause(left("192.168.1.77", "en0"))]);
        assert_eq!(w.poll(t, &moved), [Action::Resume(b("192.168.1.77", "en0", 14))]);
    }

    /// §12.3 R1: three automatic moves in any hour; the fourth pauses and
    /// asks, with an alert, and nothing moves again by itself — not even
    /// once the hour is up — until the recorded address returns.
    #[test]
    fn the_fourth_move_inside_an_hour_pauses_and_asks_with_an_alert() {
        let t0 = Instant::now();
        let s = home(&["192.168.1.5/24"], Some(GW));
        let mut w = enabled("192.168.1.5", "en0", &s);
        w.poll(t0, &s);
        let at = |n: u8| home(&[&format!("192.168.1.{n}/24")], Some(GW));
        for (m, n) in [(1, 10), (10, 11), (20, 12)] {
            assert!(matches!(w.poll(mins(t0, m), &at(n))[..], [Action::Move { .. }]), "move {n}");
        }
        let fourth = w.poll(mins(t0, 30), &at(13));
        let q = Question::TooManyMoves { from: ip("192.168.1.12"), to: ip("192.168.1.13") };
        assert_eq!(fourth, [pause(left("192.168.1.12", "en0")), Action::Ask(q.clone())]);
        assert!(q.alerts());
        assert_eq!(w.poll(mins(t0, 31), &at(13)), [], "asked once");
        let later = w.poll(mins(t0, 120), &at(14));
        assert_eq!(later, [Action::Ask(Question::TooManyMoves { from: ip("192.168.1.12"), to: ip("192.168.1.14") })], "held for the host");
        assert_eq!(w.poll(mins(t0, 121), &at(12)), [Action::Resume(b("192.168.1.12", "en0", 14))]);
        assert!(matches!(w.poll(mins(t0, 122), &at(15))[..], [Action::Move { .. }]), "the window has emptied");
    }

    #[test]
    fn moves_outside_the_rolling_hour_do_not_count() {
        let t0 = Instant::now();
        let s = home(&["192.168.1.5/24"], Some(GW));
        let mut w = enabled("192.168.1.5", "en0", &s);
        w.poll(t0, &s);
        let at = |n: u8| home(&[&format!("192.168.1.{n}/24")], Some(GW));
        // At 60 the move at 0 has left the window; at 71, the one at 10.
        for (m, n) in [(0, 10), (10, 11), (20, 12), (60, 13), (71, 14)] {
            assert!(matches!(w.poll(mins(t0, m), &at(n))[..], [Action::Move { .. }]), "move at {m} min");
        }
        // At 75 the window holds 20, 60 and 71.
        assert!(matches!(w.poll(mins(t0, 75), &at(15))[..], [_, Action::Ask(Question::TooManyMoves { .. })]));
    }

    /// §12.3 R2: only the gateway's link-layer address changed — what ARP
    /// spoofing of the gateway looks like — gets its own words and an alert.
    #[test]
    fn a_changed_gateway_lladdr_pauses_and_asks_in_its_own_words() {
        let t = Instant::now();
        let s = home(&["192.168.1.5/24"], Some(GW));
        let mut w = enabled("192.168.1.5", "en0", &s);
        w.poll(t, &s);
        let spoofed = home(&["192.168.1.5/24"], Some("66:66:66:00:00:01"));
        let q = Question::GatewayChanged { iface: "en0".into(), router: ip("192.168.1.1"), was: mac(GW), now: mac("66:66:66:00:00:01") };
        assert_eq!(
            w.poll(t, &spoofed),
            [pause(Why::NetworkChanged { addr: ip("192.168.1.5"), iface: "en0".into() }), Action::Ask(q.clone())]
        );
        assert!(q.alerts());
        assert_eq!(w.poll(t, &spoofed), []);
        assert_eq!(w.poll(t, &s), [Action::Resume(b("192.168.1.5", "en0", 14))]);
        let moved_spoofed = home(&["192.168.1.9/24"], Some("66:66:66:00:00:01"));
        assert_eq!(w.poll(t, &moved_spoofed), [pause(left("192.168.1.5", "en0")), Action::Ask(q)], "no automatic move under a changed gateway");
    }

    /// §12.4 T6: an absent entry is "not known yet" and never a change. A
    /// live bind keeps serving through it; a paused one waits, silently.
    #[test]
    fn an_absent_gateway_lladdr_is_never_a_change() {
        let t = Instant::now();
        let s = home(&["192.168.1.5/24"], Some(GW));
        let mut w = enabled("192.168.1.5", "en0", &s);
        w.poll(t, &s);
        assert_eq!(w.poll(t, &home(&["192.168.1.5/24"], None)), [], "flushed neighbour table");
        assert_eq!(w.poll(t, &snap(vec![lan("en0", 14, &["192.168.1.5/24"])], &[])), [], "no default route yet");
        assert_eq!(w.poll(t, &home(&[], None)), [pause(left("192.168.1.5", "en0"))]);
        assert_eq!(w.poll(t, &home(&["192.168.1.5/24"], None)), [], "back, but not confirmed: wait, no dialog");
        assert_eq!(w.poll(t, &home(&["192.168.1.9/24"], None)), [], "a move waits too");
        assert_eq!(w.poll(t, &home(&["192.168.1.9/24"], Some(GW))), [Action::Move { from: ip("192.168.1.5"), to: b("192.168.1.9", "en0", 14) }]);
    }

    #[test]
    fn an_unconfirmed_network_at_startup_waits_for_its_gateway() {
        let t = Instant::now();
        let mut w = Watch::restored(enabled("192.168.1.5", "en0", &home(&["192.168.1.5/24"], Some(GW))).record().clone());
        let quiet = home(&["192.168.1.5/24"], None);
        let why = Why::Unconfirmed { addr: ip("192.168.1.5"), iface: "en0".into() };
        assert_eq!(w.poll(t, &quiet), [Action::Pause { why, starting: true }]);
        assert_eq!(w.poll(t, &quiet), []);
        assert_eq!(w.poll(t, &home(&["192.168.1.5/24"], Some(GW))), [Action::Resume(b("192.168.1.5", "en0", 14))]);
    }

    /// Enabled while the neighbour entry was missing (an idle Linux host):
    /// the first reading on the same network completes the fingerprint, and
    /// from then on a different gateway is caught.
    #[test]
    fn a_fingerprint_recorded_without_the_gateway_is_completed_by_the_first_reading() {
        let t = Instant::now();
        let mut w = enabled("192.168.1.5", "en0", &home(&["192.168.1.5/24"], None));
        let s = home(&["192.168.1.5/24"], Some(GW));
        let full = Fingerprint::Lan {
            iface: "en0".into(),
            net: ip("192.168.1.0"),
            prefix: 24,
            router: Some(Router { ip: ip("192.168.1.1"), lladdr: Some(mac(GW)) }),
        };
        assert_eq!(w.poll(t, &s), [Action::Start(b("192.168.1.5", "en0", 14)), Action::Learn(full.clone())]);
        assert_eq!(w.record().fingerprint, full);
        assert_eq!(w.poll(t, &s), []);
        assert!(matches!(&w.poll(t, &home(&["192.168.1.5/24"], Some("66:66:66:00:00:01")))[..], [_, Action::Ask(Question::GatewayChanged { .. })]));

        let mut w = enabled("192.168.1.5", "en0", &snap(vec![lan("en0", 14, &["192.168.1.5/24"])], &[]));
        assert!(matches!(&w.poll(t, &s)[..], [Action::Start(_), Action::Learn(_)]), "no router at all at enable");
    }

    /// A record that lacks the router's hardware address cannot vouch for
    /// one (found by an automated review of 558705f). A café that also hands
    /// out 192.168.1.5, with its router at .1 like most home routers, must
    /// not be resumed on, moved to or learned as home. Only the bind the host
    /// just confirmed may complete the record.
    #[test]
    fn a_record_cannot_vouch_for_a_router_it_does_not_hold() {
        let t = Instant::now();
        let quiet = home(&["192.168.1.5/24"], None);
        let mut w = enabled("192.168.1.5", "en0", &quiet);
        let incomplete = w.record().clone();
        assert_eq!(w.poll(t, &quiet), [Action::Start(b("192.168.1.5", "en0", 14))]);
        assert_eq!(w.poll(t, &home(&[], None)), [pause(left("192.168.1.5", "en0"))]);
        let cafe = home(&["192.168.1.5/24"], Some("12:34:56:78:9a:bc"));
        let q = Question::CannotConfirm { addr: ip("192.168.1.5"), iface: "en0".into() };
        assert_eq!(w.poll(t, &cafe), [Action::Ask(q.clone())], "no resume, no learn");
        assert!(!q.alerts());
        assert_eq!(w.record(), &incomplete);
        assert_eq!(w.poll(t, &quiet), [Action::Resume(b("192.168.1.5", "en0", 14))], "neither side knows: the same");
        let unconfirmed = Why::Unconfirmed { addr: ip("192.168.1.5"), iface: "en0".into() };
        assert_eq!(w.poll(t, &cafe), [pause(unconfirmed.clone()), Action::Ask(q.clone())], "a resumed bind is not the host's");
        assert_eq!(w.record(), &incomplete);
        let moved = home(&["192.168.1.9/24"], Some("12:34:56:78:9a:bc"));
        assert_eq!(w.poll(t, &moved), [Action::Ask(Question::CannotConfirm { addr: ip("192.168.1.9"), iface: "en0".into() })], "no move");
        let mut r = Watch::restored(incomplete.clone());
        assert_eq!(r.poll(t, &cafe), [Action::Pause { why: unconfirmed, starting: true }, Action::Ask(q)], "a restart is not the host's either");
        assert_eq!(r.record(), &incomplete);

        // A move that matched only because neither side knew the lladdr is
        // not the host's bind either.
        let mut m = enabled("192.168.1.5", "en0", &quiet);
        m.poll(t, &quiet);
        assert!(matches!(m.poll(t, &home(&["192.168.1.9/24"], None))[..], [Action::Move { .. }]));
        let after = m.record().clone();
        let unconfirmed9 = Why::Unconfirmed { addr: ip("192.168.1.9"), iface: "en0".into() };
        let q9 = Question::CannotConfirm { addr: ip("192.168.1.9"), iface: "en0".into() };
        assert_eq!(m.poll(t, &moved), [pause(unconfirmed9), Action::Ask(q9)], "no learning after a move");
        assert_eq!(m.record(), &after);

        // No router at all in the record: a link without one resumes, and a
        // router showing up later is one the record cannot vouch for.
        let cable = snap(vec![lan("bridge0", 20, &["169.254.7.7/16"])], &[]);
        let mut c = enabled("169.254.7.7", "bridge0", &cable);
        c.poll(t, &cable);
        c.poll(t, &snap(vec![lan("bridge0", 20, &[])], &[]));
        assert_eq!(c.poll(t, &cable), [Action::Resume(b("169.254.7.7", "bridge0", 20))], "a routerless link resumes");
        let routed = snap(vec![lan("bridge0", 20, &["169.254.7.7/16"])], &[("bridge0", "169.254.0.1", None)]);
        assert_eq!(
            c.poll(t, &routed),
            [
                pause(Why::Unconfirmed { addr: ip("169.254.7.7"), iface: "bridge0".into() }),
                Action::Ask(Question::CannotConfirm { addr: ip("169.254.7.7"), iface: "bridge0".into() })
            ]
        );
    }

    /// A café that also hands out 192.168.1.x: everything matches but the
    /// router. And an address in another subnet on the interface.
    #[test]
    fn a_network_that_is_not_ours_pauses_and_asks_once() {
        let t = Instant::now();
        let s = home(&["192.168.1.5/24"], Some(GW));
        let mut w = enabled("192.168.1.5", "en0", &s);
        w.poll(t, &s);
        let cafe = snap(vec![lan("en0", 14, &["192.168.1.5/24"])], &[("en0", "192.168.1.254", Some("12:34:56:78:9a:bc"))]);
        let q = Question::DifferentNetwork { addr: ip("192.168.1.5"), iface: "en0".into() };
        assert_eq!(w.poll(t, &cafe), [pause(Why::NetworkChanged { addr: ip("192.168.1.5"), iface: "en0".into() }), Action::Ask(q.clone())]);
        assert!(!q.alerts());
        assert_eq!(w.poll(t, &cafe), []);
        let other = |a: &str| snap(vec![lan("en0", 14, &[a])], &[("en0", "192.168.4.1", Some("12:34:56:78:9a:bc"))]);
        assert_eq!(w.poll(t, &other("192.168.4.20/24")), [Action::Ask(Question::DifferentNetwork { addr: ip("192.168.4.20"), iface: "en0".into() })]);
        assert_eq!(w.poll(t, &other("192.168.4.20/24")), []);
        assert_eq!(w.poll(t, &home(&[], Some(GW))), [Action::Retract]);
        assert_eq!(w.poll(t, &home(&[], Some(GW))), []);
        assert_eq!(w.poll(t, &s), [Action::Resume(b("192.168.1.5", "en0", 14))]);
    }

    /// Overlay mode: the fingerprint is the address, so the name may change
    /// (`utun6` → `utun7`) and a new address is never a move.
    #[test]
    fn an_overlay_follows_its_address_across_renumbering_and_never_moves() {
        let t = Instant::now();
        let ts = |name: &str, index: u32, a: &[&str]| snap(vec![lan("en0", 14, &["192.168.1.5/24"]), iface(name, index, true, a)], &[]);
        let s = ts("utun6", 25, &["100.96.164.69/32"]);
        let mut w = enabled("100.96.164.69", "utun6", &s);
        assert_eq!(w.record().fingerprint, Fingerprint::PointToPoint { addr: ip("100.96.164.69"), prefix: 32 });
        assert_eq!(w.poll(t, &s), [Action::Start(b("100.96.164.69", "utun6", 25))]);
        let renumbered = ts("utun7", 27, &["100.96.164.69/32"]);
        assert_eq!(
            w.poll(t, &renumbered),
            [pause(Why::InterfaceReplaced { iface: "utun6".into(), was: 25, now: 27 }), Action::Resume(b("100.96.164.69", "utun7", 27))]
        );
        assert_eq!(w.poll(t, &ts("utun7", 27, &["100.96.9.9/32"])), [pause(left("100.96.164.69", "utun7"))], "no move, no question");
        assert_eq!(w.poll(t, &ts("utun7", 27, &[])), []);
        assert_eq!(w.poll(t, &renumbered), [Action::Resume(b("100.96.164.69", "utun7", 27))]);
        let on_a_lan = snap(vec![lan("en5", 30, &["100.96.164.69/32"])], &[]);
        assert_eq!(
            w.poll(t, &on_a_lan),
            [
                pause(Why::NetworkChanged { addr: ip("100.96.164.69"), iface: "en5".into() }),
                Action::Ask(Question::DifferentNetwork { addr: ip("100.96.164.69"), iface: "en5".into() })
            ]
        );
    }

    /// A move re-runs the address rules on the new address: stable, of the
    /// same kind, in the recorded subnet. A temporary IPv6 address, or a
    /// self-assigned 169.254 one after a DHCP failure, is neither a move nor
    /// a question.
    #[test]
    fn only_a_stable_address_of_the_same_kind_is_a_move_or_a_question() {
        let t = Instant::now();
        let v6 = |a: &[&str]| snap(vec![lan("en0", 14, a)], &[("en0", "fe80::1", None)]);
        let s = v6(&["fd00::5/64", "fe80::5/64"]);
        let mut w = enabled("fd00::5", "en0", &s);
        w.poll(t, &s);
        assert_eq!(w.poll(t, &v6(&["fd00::9/64 temporary", "fd00::a/64 deprecated", "fe80::5/64"])), [pause(left("fd00::5", "en0"))]);
        assert_eq!(w.poll(t, &v6(&["fd00::7/64", "fe80::5/64"])), [Action::Move { from: ip("fd00::5"), to: b("fd00::7", "en0", 14) }]);

        let s = home(&["192.168.1.5/24"], Some(GW));
        let mut w = enabled("192.168.1.5", "en0", &s);
        w.poll(t, &s);
        assert_eq!(w.poll(t, &home(&["169.254.3.4/16"], None)), [pause(left("192.168.1.5", "en0"))]);
    }

    #[test]
    fn a_link_addr_parses_as_scutil_and_ndp_print_it() {
        assert_eq!(mac("d8:44:89:a:b:c").to_string(), "d8:44:89:0a:0b:0c");
        assert_eq!(LinkAddr::parse("2:0:0:0:0:0").unwrap().0, [2, 0, 0, 0, 0, 0]);
        for bad in ["", "d8", "d8::89:0a:0b:0c", "d8:44:89:0a:0b:0g", "d8:44:89:0a:0b:0c0", "(none)"] {
            assert_eq!(LinkAddr::parse(bad), None, "{bad:?}");
        }
    }
}
