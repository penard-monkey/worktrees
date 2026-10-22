//! How the host's git fetches for whoever asked: the ONE place a caller's
//! class becomes the flags and environment a network `git` runs with
//! (Server Edition Phase 2, gap 17; §3.4 and O6).
//!
//! The console fetches exactly as it always did. That covers the host's own
//! user at the host, the CLI and the engine's own work, and
//! [`Policy::Console`] adds nothing. A paired device can make the host dial
//! (`clone_project`, `skill_preview_git`), and it must not borrow the host's
//! network position: loopback services, the LAN, a cloud metadata address.
//! Checking the URL's host first (serve's resolver, slice 1) is not enough on
//! its own, because git decides AGAIN where to connect:
//!
//! - it resolves the name itself, so DNS can answer differently at fetch time
//!   (rebinding);
//! - it follows an HTTP redirect on the initial request (the default,
//!   `http.followRedirects=initial`), so a public URL can send it to
//!   `http://127.0.0.1/…`;
//! - over protocol v2 it takes a `packfile-uris` section the server sends
//!   UNASKED and runs `git http-fetch` on each URI (`fetch-pack.c`,
//!   `do_fetch_pack_v2`), and v2's bundle URIs are another server-chosen URL;
//! - the host's own git config moves the fetch. A `url.<base>.insteadOf`
//!   rewrite does, including one under `includeIf "gitdir:…"`, which applies
//!   during a clone into a matching directory (measured). So do a per-URL
//!   `http.<url>.curloptResolve` or `.proxy`, and a per-URL
//!   `http.<url>.followRedirects=true` BEATS a generic `-c` (measured, git
//!   2.54). Git's own environment (`GIT_SSL_NO_VERIFY`) overrides any `-c`;
//! - a proxy resolves the name itself, so a pin means nothing through one.
//!
//! So [`Pin::resolve`] resolves the host ONCE, here, and refuses unless EVERY
//! address it got is global ([`host_local`]). [`Policy::git`] then hands git
//! exactly those addresses and leaves it nothing that could move the fetch:
//!
//! - no redirects;
//! - no proxy;
//! - protocol v1;
//! - TLS verified;
//! - only the device's schemes.
//!
//! Each `http.*` key is set twice, generic and keyed to the exact URL, so no
//! per-URL entry in the host's config is more specific than ours. The URL
//! also maps to itself, so no shorter `insteadOf` prefix can rewrite it. An
//! EXACT one is a tie, which git gives to the rule it read first (the
//! host's), so [`Pin::resolve`] first asks git where the URL goes, under the
//! same mapping, and refuses if the answer is not the URL itself.
//!
//! **A device fetches over https, and only https.**
//!
//! - ssh (`ssh://`, `git@host:path`, `git+ssh://`): ssh resolves the name
//!   itself. A pin (`-o HostName=`) would still leave the user's ssh config
//!   choosing its `Host` and `Match` blocks (`ProxyCommand`, `ProxyJump`,
//!   `Match exec`, `ForwardAgent`) by the name the DEVICE chose. ssh would
//!   also offer every key in the host's agent to whichever server answers,
//!   and that tells a stranger's server who the user is. Each could be closed
//!   one at a time; refusing is the smaller thing to get right, and a device's
//!   clone is a forge URL, which https serves.
//! - `git://`: git dials it itself, not curl, so there is nothing to pin. It
//!   is cleartext and unauthenticated besides.
//! - `http://`: cleartext, so whoever is on the path answers for the pinned
//!   address. Nothing a device needs is served only that way.
//! - `file://`, a local path, `ext::` and every remote helper: the host's
//!   disk, or a command line.
//!
//! https also AUTHENTICATES the address it dials. With `http.sslVerify`
//! forced on, a pinned address whose server cannot show the named host's
//! certificate fails the handshake before any request is sent.
//!
//! **What a device's clone keeps:** `http.followRedirects=false`, in the new
//! repository's own config. That clone registers a project whose `origin` the
//! device chose, and every later fetch of it would otherwise follow a redirect
//! from there: `fetch_origin`, a new place's base, the auto-fetch, the
//! console's own. Those later fetches are NOT pinned, on purpose. A registered
//! project's origin is the project's destination, not the device's, and a LAN
//! remote of the user's own is legitimate. The one way a device chooses an
//! origin is this clone, whose origin is https.
//!
//! What is left (ROADMAP): an `includeIf`-conditional `insteadOf` whose value
//! is EXACTLY the device's URL, for a clone into a directory its condition
//! matches. git gives the tie to the host's rule, and the pre-flight question
//! runs outside any repository, so it cannot see the rule.

use std::ffi::{OsStr, OsString};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::process::{Command, Stdio};

/// The URL schemes a device may make the host fetch over (module doc).
pub const DEVICE_SCHEMES: &[&str] = &["https"];

/// The oldest git that reads `http.curloptResolve` (2.37). An older one
/// IGNORES the key, so the pin would be silently absent.
const PINNING_GIT: (u32, u32) = (2, 37);

/// The environment that can carry a fetch somewhere else or turn TLS
/// verification off, and that outranks any `-c`.
const STRIPPED_ENV: &[&str] =
    &["http_proxy", "HTTP_PROXY", "https_proxy", "HTTPS_PROXY", "all_proxy", "ALL_PROXY", "GIT_PROXY_COMMAND", "GIT_SSL_NO_VERIFY"];

/// git's transport names, each one allowed or refused explicitly: a
/// per-protocol `protocol.<x>.allow` in the host's config outranks a generic
/// `protocol.allow`.
const TRANSPORTS: &[&str] = &["file", "git", "ssh", "ext", "http", "https"];

/// Who asked the host to fetch: the one input the policy keys on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Fetcher {
    /// The host's own user, at the host: the console, the CLI, the engine.
    #[default]
    Console,
    /// A paired device, with the pins serve's resolver made for THIS request:
    /// the very addresses it checked.
    Device(Vec<Pin>),
}

impl Fetcher {
    pub fn is_console(&self) -> bool {
        matches!(self, Fetcher::Console)
    }

    /// How git fetches `url`, spelled exactly as git will be handed it. THE
    /// place a caller's class decides how git dials. A device's fetch that the
    /// resolver did not pin is resolved and checked here instead, never let
    /// through unpinned. That covers a row that fetches without declaring
    /// `remote_url`, and a body that hands git another spelling.
    pub fn policy(&self, url: &str) -> Result<Policy, String> {
        match self {
            Fetcher::Console => Ok(Policy::Console),
            Fetcher::Device(pins) => match pins.iter().find(|p| p.url == url) {
                Some(p) => Ok(Policy::Pinned(p.clone())),
                None => Pin::resolve(url).map(Policy::Pinned),
            },
        }
    }

    /// `self` for a caller that is also `other`. Narrowing only: a device
    /// stays a device, so a `Ctx` cannot be turned back into the console's.
    pub fn narrow(&self, other: Fetcher) -> Fetcher {
        match (self, other) {
            (Fetcher::Console, o) => o,
            (Fetcher::Device(mine), Fetcher::Console) => Fetcher::Device(mine.clone()),
            (Fetcher::Device(mine), Fetcher::Device(mut theirs)) => {
                theirs.extend(mine.iter().cloned());
                Fetcher::Device(theirs)
            }
        }
    }
}

/// How one network `git` runs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Policy {
    /// Today's git, unchanged.
    #[default]
    Console,
    /// A device's fetch: dialled at the checked addresses and nowhere else.
    Pinned(Pin),
}

impl Policy {
    /// `git <pre…> <the policy's -c…> <sub…>`, with `env` and then the
    /// policy's environment over it. The policy has the last word on both, so
    /// a call site's own `-c` (`pre`) or variable cannot loosen it. A device's
    /// `clone` also keeps `http.followRedirects=false` in the new repository
    /// (module doc). The caller adds operands after `sub` and its stdio.
    pub fn git<P, S>(&self, pre: P, sub: S, env: &[(&str, String)]) -> Command
    where
        P: IntoIterator,
        P::Item: AsRef<OsStr>,
        S: IntoIterator,
        S::Item: AsRef<OsStr>,
    {
        let mut cmd = crate::childenv::command("git");
        cmd.args(pre);
        if let Policy::Pinned(pin) = self {
            for kv in pin.config() {
                cmd.arg("-c").arg(kv);
            }
        }
        let mut sub = sub.into_iter().map(|s| s.as_ref().to_owned());
        if let Some(first) = sub.next() {
            let keeps = matches!(self, Policy::Pinned(_)) && first == OsStr::new("clone");
            cmd.arg(first);
            if keeps {
                cmd.args(["--config", "http.followRedirects=false"]);
            }
        }
        cmd.args(sub.collect::<Vec<OsString>>());
        for (k, v) in env {
            cmd.env(k, v);
        }
        if let Policy::Pinned(pin) = self {
            for k in STRIPPED_ENV {
                cmd.env_remove(k);
            }
            cmd.env("GIT_ALLOW_PROTOCOL", pin.allow.join(":"));
            cmd.env("GIT_TERMINAL_PROMPT", "0");
        }
        cmd
    }
}

/// A device's fetch, resolved once and checked: what git is handed and where
/// it may connect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pin {
    /// The URL exactly as git is handed it.
    pub url: String,
    /// Its host, as the URL spells it (an IPv6 literal without brackets).
    pub host: String,
    pub port: u16,
    /// Every address `host` resolved to, each one checked. Empty when the
    /// host IS an address, which curl does not resolve.
    pub addrs: Vec<IpAddr>,
    /// The schemes git may use, as `GIT_ALLOW_PROTOCOL` spells them.
    allow: Vec<String>,
}

/// What [`Pin::resolve_with`] checks with. In use, [`Rules::device`]; the
/// tests stand a loopback server in for a public one and a scripted resolver
/// in for DNS.
pub struct Rules<'a> {
    /// The schemes a fetch may use.
    pub schemes: &'a [&'a str],
    /// Name → addresses: the one lookup.
    pub resolve: &'a dyn Fn(&str, u16) -> Result<Vec<IpAddr>, String>,
    /// Whether the host may dial an address for the caller.
    pub admit: &'a dyn Fn(&IpAddr) -> bool,
    /// The environment the fetch's git runs with, for the pre-flight `git`
    /// (a test's own git config; nothing in use).
    pub git_env: &'a [(&'a str, String)],
}

impl Rules<'static> {
    /// A paired device: https, the system resolver, global addresses only.
    pub fn device() -> Rules<'static> {
        Rules { schemes: DEVICE_SCHEMES, resolve: &system_resolve, admit: &global, git_env: &[] }
    }
}

impl Pin {
    /// [`Pin::resolve_with`] a device's rules.
    pub fn resolve(url: &str) -> Result<Pin, String> {
        Pin::resolve_with(url, &Rules::device())
    }

    /// Check `url` for a fetch under `rules` and resolve its host ONCE. `Err`
    /// is why it may not be fetched. Every refusal that needs no process
    /// comes first; then git's version and where git would send the URL;
    /// then the one lookup.
    pub fn resolve_with(url: &str, rules: &Rules) -> Result<Pin, String> {
        let shown = crate::clone::redact_url(url);
        let (host, port) = parse(url, rules.schemes)?;
        let lower = host.to_ascii_lowercase();
        if lower == "localhost" || lower.ends_with(".localhost") {
            return Err(format!("{host} is this host"));
        }
        let literal = literal_ip(&host)?;
        if let Some(ip) = literal {
            if !(rules.admit)(&ip) {
                return Err(format!("{ip} is an address only the host itself can reach"));
            }
        }
        pinning_git()?;
        let to = dial(url, rules.git_env)?;
        if to != url {
            return Err(format!(
                "this host's git config rewrites {shown} to {} (url.<base>.insteadOf), so a device may not fetch it",
                crate::clone::redact_url(&to)
            ));
        }
        let addrs = match literal {
            Some(_) => Vec::new(),
            None => {
                let addrs = (rules.resolve)(&host, port)?;
                if addrs.is_empty() {
                    return Err(format!("{host} resolves to nothing"));
                }
                if let Some(ip) = addrs.iter().find(|ip| !(rules.admit)(ip)) {
                    return Err(format!("{host} is {ip}, which only the host itself can reach"));
                }
                addrs
            }
        };
        let allow = rules.schemes.iter().map(|s| s.to_string()).collect();
        Ok(Pin { url: url.to_string(), host, port, addrs, allow })
    }

    /// `host:port:addr[,addr…]` for `http.curloptResolve`, or `None` for an
    /// address host.
    fn resolve_entry(&self) -> Option<String> {
        if self.addrs.is_empty() {
            return None;
        }
        let addrs: Vec<String> = self
            .addrs
            .iter()
            .map(|a| match a {
                IpAddr::V4(v) => v.to_string(),
                IpAddr::V6(v) => format!("[{v}]"),
            })
            .collect();
        Some(format!("{}:{}:{}", self.host, self.port, addrs.join(",")))
    }

    /// The `-c` values, in order. Later wins for git, so nothing the call
    /// site put in `pre` can undo them.
    fn config(&self) -> Vec<String> {
        let u = &self.url;
        let mut c = vec!["protocol.allow=never".to_string()];
        for t in TRANSPORTS {
            let on = self.allow.iter().any(|a| a == t);
            c.push(format!("protocol.{t}.allow={}", if on { "always" } else { "never" }));
        }
        // v1: no `packfile-uris`, no bundle URIs (module doc).
        c.push("protocol.version=1".into());
        // A `remote.<name>.proxy` outranks every `http.proxy`; the name is
        // pinned so that the one below is the remote's.
        c.push("clone.defaultRemoteName=origin".into());
        c.push("remote.origin.proxy=".into());
        c.push(format!("url.{u}.insteadOf={u}"));
        let entry = self.resolve_entry();
        for scope in ["http".to_string(), format!("http.{u}")] {
            c.push(format!("{scope}.sslVerify=true"));
            c.push(format!("{scope}.followRedirects=false"));
            c.push(format!("{scope}.proxy="));
            // An empty value resets the list, the host's entries with it.
            c.push(format!("{scope}.curloptResolve="));
            if let Some(e) = &entry {
                c.push(format!("{scope}.curloptResolve={e}"));
            }
        }
        c
    }
}

/// `scheme://[userinfo@]host[:port]/…` → (host, port), for a scheme in
/// `schemes`. Strict on purpose: a host read differently here than by curl
/// is the hole, so anything ambiguous is refused rather than guessed at.
fn parse(url: &str, schemes: &[&str]) -> Result<(String, u16), String> {
    let only = || format!("a device may fetch over {} only", schemes.join(", "));
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("a URL cannot contain spaces or control characters".into());
    }
    let Some((scheme, rest)) = url.split_once("://") else {
        if url.contains("::") {
            return Err("a remote helper (`helper::address`) runs a command".into());
        }
        if url.starts_with(['/', '.', '~']) {
            return Err("a device may not fetch a local path from the host".into());
        }
        if url.contains(':') && !url[..url.find(':').unwrap_or(0)].contains('/') {
            return Err(format!("{}, not ssh (`host:path`)", only()));
        }
        return Err(format!("{} — not a URL: {url:?}", only()));
    };
    if !schemes.contains(&scheme) {
        return Err(format!("{}, not {scheme}://", only()));
    }
    // `-c key=value` splits at the FIRST '=', and the URL is part of a key.
    if url.contains('=') {
        return Err("a device's URL may not contain '='".into());
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.matches('@').count() > 1 {
        return Err("more than one '@' in the host part".into());
    }
    let hostport = authority.rsplit('@').next().unwrap_or("");
    let (host, port) = match hostport.strip_prefix('[') {
        Some(r) => {
            let (h, after) = r.split_once(']').ok_or("an unclosed '[' in the host")?;
            (h, after)
        }
        None => match hostport.split_once(':') {
            Some((h, _)) => (h, &hostport[h.len()..]),
            None => (hostport, ""),
        },
    };
    let port = match port {
        "" => match scheme {
            "https" => 443,
            "http" => 80,
            _ => return Err(format!("no default port for {scheme}://")),
        },
        p => p
            .strip_prefix(':')
            .and_then(|n| n.parse::<u16>().ok())
            .filter(|n| *n != 0)
            .ok_or_else(|| format!("not a port: {p:?}"))?,
    };
    if host.is_empty() {
        return Err(format!("no host in {:?}", crate::clone::redact_url(url)));
    }
    if hostport.starts_with('[') {
        host.parse::<Ipv6Addr>().map_err(|_| format!("not an IPv6 address: [{host}]"))?;
    } else if !host.chars().all(|c| c.is_ascii_alphanumeric() || "-._".contains(c)) {
        return Err(format!("not a host name: {host:?}"));
    }
    Ok((host.to_string(), port))
}

/// The host as an address, parsed strictly (`std::net`, never `inet_aton`).
/// A host that ENDS in a number must be an address, written plainly:
/// `127.1`, `0x7f.0.0.1` and `2130706433` are how an address hides from a
/// check that reads it differently than the resolver will (§12, S7).
fn literal_ip(host: &str) -> Result<Option<IpAddr>, String> {
    if let Ok(v6) = host.parse::<Ipv6Addr>() {
        return Ok(Some(IpAddr::V6(v6)));
    }
    if let Ok(v4) = host.parse::<Ipv4Addr>() {
        return Ok(Some(IpAddr::V4(v4)));
    }
    let last = host.trim_end_matches('.').rsplit('.').next().unwrap_or("");
    let numeric = !last.is_empty()
        && (last.chars().all(|c| c.is_ascii_digit())
            || last.strip_prefix("0x").or_else(|| last.strip_prefix("0X")).is_some_and(|h| h.chars().all(|c| c.is_ascii_hexdigit())));
    if numeric {
        return Err(format!("{host} is an address in a spelling this host does not accept"));
    }
    Ok(None)
}

/// Where git sends `url` under the policy's own `insteadOf` mapping, asked
/// with no repository in play: the URL itself, unless the host's config has
/// an exact rule for it.
fn dial(url: &str, env: &[(&str, String)]) -> Result<String, String> {
    let tmp = std::env::temp_dir();
    let mut cmd = crate::childenv::command("git");
    cmd.arg("-c")
        .arg(format!("url.{url}.insteadOf={url}"))
        .args(["ls-remote", "--get-url", "--", url])
        .current_dir(&tmp)
        .env("GIT_CEILING_DIRECTORIES", &tmp);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = crate::proc::run_deadline(cmd, 10).map_err(|e| format!("git failed to start: {e}"))?;
    if !out.status.success() {
        return Err(format!("git cannot read that URL: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_end_matches(['\n', '\r']).to_string())
}

/// `Ok` when this host's git reads `http.curloptResolve`. A yes is kept for
/// the process; a no is asked again, so an upgrade needs no restart.
fn pinning_git() -> Result<(), String> {
    use std::sync::atomic::{AtomicBool, Ordering};
    static OK: AtomicBool = AtomicBool::new(false);
    if OK.load(Ordering::Relaxed) {
        return Ok(());
    }
    let out = crate::childenv::command("git")
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git failed to start: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    match git_version(&text) {
        Some(v) if v >= PINNING_GIT => {
            OK.store(true, Ordering::Relaxed);
            Ok(())
        }
        _ => Err(format!(
            "this host's git ({}) cannot pin a fetch to an address (that needs git {}.{}), so a device may not make it fetch",
            text.trim(),
            PINNING_GIT.0,
            PINNING_GIT.1
        )),
    }
}

/// `git version 2.54.0 (Apple Git-157)` → `(2, 54)`.
fn git_version(text: &str) -> Option<(u32, u32)> {
    let v = text.trim().strip_prefix("git version ")?;
    let mut it = v.split(|c: char| !c.is_ascii_digit());
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

fn system_resolve(host: &str, port: u16) -> Result<Vec<IpAddr>, String> {
    use std::net::ToSocketAddrs;
    let mut out: Vec<IpAddr> = Vec::new();
    for a in (host, port).to_socket_addrs().map_err(|e| format!("cannot resolve {host}: {e}"))? {
        if !out.contains(&a.ip()) {
            out.push(a.ip());
        }
    }
    Ok(out)
}

/// An address the host may dial for a device.
pub fn global(ip: &IpAddr) -> bool {
    !host_local(ip)
}

/// An address a device must not make the host dial for it: anything that is
/// not plainly GLOBAL unicast. Fail-closed by construction. The list is of
/// what is refused, but every block a host can hide behind is on it, and an
/// IPv4 address embedded in IPv6 (mapped, compatible, NAT64 `64:ff9b::/96`,
/// 6to4 `2002::/16`) is judged as the IPv4 address it carries, decoded by
/// [`crate::netclass::embedded_v4`], the one decoder the remote listener's
/// classifier reads them with too. Teredo (`2001::/32`) obfuscates its
/// embedded address, so it is refused whole.
pub fn host_local(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            o[0] == 0                                   // 0.0.0.0/8, "this network"
                || o[0] == 10                           // RFC 1918
                || o[0] == 127                          // loopback
                || (o[0] == 100 && (o[1] & 0xc0) == 64) // CGNAT 100.64.0.0/10
                || (o[0] == 169 && o[1] == 254)         // link-local
                || (o[0] == 172 && (o[1] & 0xf0) == 16) // RFC 1918
                || (o[0] == 192 && o[1] == 168)         // RFC 1918
                || (o[0] == 192 && o[1] == 0 && (o[2] == 0 || o[2] == 2)) // IETF protocol, TEST-NET-1
                || (o[0] == 198 && (o[1] & 0xfe) == 18) // benchmarking 198.18.0.0/15
                || (o[0] == 198 && o[1] == 51 && o[2] == 100) // TEST-NET-2
                || (o[0] == 203 && o[1] == 0 && o[2] == 113)  // TEST-NET-3
                || o[0] >= 224                          // multicast, reserved, broadcast
        }
        IpAddr::V6(v) => {
            // Before the decoder: `::` and `::1` are not the compatible form.
            if v.is_loopback() || v.is_unspecified() {
                return true;
            }
            if let Some((_, v4)) = crate::netclass::embedded_v4(v) {
                return host_local(&IpAddr::V4(v4));
            }
            let s = v.segments();
            (s[0] & 0xe000) != 0x2000                   // not global unicast 2000::/3
                || (s[0] == 0x2001 && s[1] == 0)        // Teredo 2001::/32
                || (s[0] == 0x2001 && s[1] == 0x0db8)   // documentation 2001:db8::/32
                || (s[0] & 0xfe00) == 0xfc00            // ULA (outside 2000::/3 anyway)
                || (s[0] & 0xffc0) == 0xfe80            // link-local
                || (s[0] & 0xffc0) == 0xfec0            // site-local
                || (s[0] & 0xff00) == 0xff00            // multicast
        }
    }
}

/// A loopback HTTP server for the fetch tests: no external network, every
/// request recorded, each one answered by a redirect, a 404, or
/// `git http-backend` (smart HTTP, so a shallow clone works).
#[cfg(test)]
pub(crate) mod testnet {
    use std::io::{Read, Write};
    use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[derive(Clone, Debug, Default)]
    pub struct Hit {
        pub path: String,
        /// The `Host` header.
        pub host: String,
        /// The `Git-Protocol` header ("" when absent).
        pub protocol: String,
    }

    #[derive(Clone)]
    pub enum Answer {
        /// `302` to this prefix + the request's own path.
        Redirect(String),
        /// `404`.
        Missing,
        /// `git http-backend` over this project root.
        Git(PathBuf),
    }

    pub struct Server {
        pub addr: SocketAddr,
        hits: Arc<Mutex<Vec<Hit>>>,
        stop: Arc<AtomicBool>,
    }

    impl Server {
        /// A server on `ip:port` (`0` = any free port), or `None` when the
        /// address cannot be bound here (no IPv6 loopback, say).
        pub fn start(ip: IpAddr, port: u16, answer: Answer) -> Option<Server> {
            let listener = TcpListener::bind(SocketAddr::new(ip, port)).ok()?;
            listener.set_nonblocking(true).ok()?;
            let addr = listener.local_addr().ok()?;
            let hits = Arc::new(Mutex::new(Vec::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let (h, s) = (hits.clone(), stop.clone());
            std::thread::spawn(move || {
                while !s.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((conn, _)) => {
                            let (h, a) = (h.clone(), answer.clone());
                            std::thread::spawn(move || {
                                let _ = handle(conn, &h, &a);
                            });
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(5)),
                    }
                }
            });
            Some(Server { addr, hits, stop })
        }

        pub fn port(&self) -> u16 {
            self.addr.port()
        }

        pub fn hits(&self) -> Vec<Hit> {
            self.hits.lock().unwrap().clone()
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
        }
    }

    fn handle(mut conn: TcpStream, hits: &Mutex<Vec<Hit>>, answer: &Answer) -> std::io::Result<()> {
        // An accepted socket inherits the listener's O_NONBLOCK on BSD.
        conn.set_nonblocking(false)?;
        conn.set_read_timeout(Some(Duration::from_secs(10)))?;
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        let head_end = loop {
            let n = conn.read(&mut chunk)?;
            if n == 0 {
                return Ok(());
            }
            buf.extend_from_slice(&chunk[..n]);
            if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break i + 4;
            }
            if buf.len() > 64 * 1024 {
                return Ok(());
            }
        };
        let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
        let mut lines = head.split("\r\n");
        let mut first = lines.next().unwrap_or("").split(' ');
        let method = first.next().unwrap_or("").to_string();
        let target = first.next().unwrap_or("").to_string();
        let header = |name: &str| {
            head.split("\r\n")
                .skip(1)
                .find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_string()))
                .unwrap_or_default()
        };
        let mut body = buf[head_end..].to_vec();
        if header("transfer-encoding").eq_ignore_ascii_case("chunked") {
            body = read_chunked(&mut conn, body)?;
        } else {
            let len: usize = header("content-length").parse().unwrap_or(0);
            while body.len() < len {
                let n = conn.read(&mut chunk)?;
                if n == 0 {
                    break;
                }
                body.extend_from_slice(&chunk[..n]);
            }
        }
        hits.lock().unwrap().push(Hit { path: target.clone(), host: header("host"), protocol: header("git-protocol") });
        let resp = match answer {
            Answer::Redirect(to) => {
                format!("HTTP/1.1 302 Found\r\nLocation: {to}{target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes()
            }
            Answer::Missing => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
            Answer::Git(root) => backend(root, &method, &target, &header, body),
        };
        conn.write_all(&resp)?;
        conn.flush()
    }

    fn read_chunked(conn: &mut TcpStream, mut raw: Vec<u8>) -> std::io::Result<Vec<u8>> {
        let mut chunk = [0u8; 4096];
        while !raw.windows(5).any(|w| w == b"0\r\n\r\n") {
            let n = conn.read(&mut chunk)?;
            if n == 0 {
                break;
            }
            raw.extend_from_slice(&chunk[..n]);
        }
        let mut out = Vec::new();
        let mut i = 0;
        while let Some(eol) = raw[i..].windows(2).position(|w| w == b"\r\n") {
            let size = usize::from_str_radix(String::from_utf8_lossy(&raw[i..i + eol]).trim(), 16).unwrap_or(0);
            i += eol + 2;
            if size == 0 || i + size > raw.len() {
                break;
            }
            out.extend_from_slice(&raw[i..i + size]);
            i += size + 2;
        }
        Ok(out)
    }

    /// One request through `git http-backend` (CGI), bounded in time.
    fn backend(root: &Path, method: &str, target: &str, header: &dyn Fn(&str) -> String, body: Vec<u8>) -> Vec<u8> {
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let mut cmd = crate::childenv::command("git");
        cmd.arg("http-backend")
            .env("GIT_PROJECT_ROOT", root)
            .env("GIT_HTTP_EXPORT_ALL", "1")
            .env("PATH_INFO", path)
            .env("QUERY_STRING", query)
            .env("REQUEST_METHOD", method)
            .env("CONTENT_TYPE", header("content-type"))
            .env("CONTENT_LENGTH", body.len().to_string())
            .env("REMOTE_ADDR", "127.0.0.1")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("HOME", root);
        for (cgi, h) in [("HTTP_CONTENT_ENCODING", "content-encoding"), ("HTTP_GIT_PROTOCOL", "git-protocol")] {
            let v = header(h);
            if !v.is_empty() {
                cmd.env(cgi, v);
            }
        }
        let Ok(out) = crate::proc::run_deadline_stdin(cmd, body, 20) else {
            return b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec();
        };
        let o = out.stdout;
        let (split, skip) = match o.windows(4).position(|w| w == b"\r\n\r\n") {
            Some(i) => (i, 4),
            None => (o.windows(2).position(|w| w == b"\n\n").unwrap_or(o.len()), 2),
        };
        let cgi_head = String::from_utf8_lossy(&o[..split]).to_string();
        let content = o.get(split + skip..).unwrap_or_default();
        let mut status = "200 OK".to_string();
        let mut resp = String::new();
        for l in cgi_head.lines() {
            match l.split_once(':') {
                Some((k, v)) if k.eq_ignore_ascii_case("status") => status = v.trim().to_string(),
                Some(_) => resp.push_str(&format!("{}\r\n", l.trim_end())),
                None => {}
            }
        }
        let mut bytes =
            format!("HTTP/1.1 {status}\r\n{resp}Content-Length: {}\r\nConnection: close\r\n\r\n", content.len()).into_bytes();
        bytes.extend_from_slice(content);
        bytes
    }

    /// An environment for the test's git that reads none of the user's:
    /// `global` is the only git config, `dir` is HOME.
    pub fn hermetic(dir: &Path, global: &Path) -> Vec<(&'static str, String)> {
        let d = dir.to_string_lossy().into_owned();
        vec![
            ("HOME", d.clone()),
            ("XDG_CONFIG_HOME", d),
            ("GIT_CONFIG_NOSYSTEM", "1".into()),
            ("GIT_CONFIG_GLOBAL", global.to_string_lossy().into_owned()),
            ("GIT_TERMINAL_PROMPT", "0".into()),
        ]
    }

    /// A bare repository `<root>/r.git` with one commit and a skill, for
    /// `http-backend` to serve.
    pub fn bare_repo(root: &Path) -> PathBuf {
        let git = |cwd: &Path, args: &[&str]| {
            let out = crate::childenv::command("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "init.defaultBranch=main"])
                .args(args)
                .current_dir(cwd)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        let work = root.join("work");
        std::fs::create_dir_all(&work).unwrap();
        git(&work, &["init", "-q"]);
        std::fs::create_dir_all(work.join("pinned")).unwrap();
        std::fs::write(work.join("pinned/SKILL.md"), "---\nname: pinned\ndescription: a test skill\n---\nbody\n").unwrap();
        git(&work, &["add", "."]);
        git(&work, &["commit", "-q", "-m", "first"]);
        let bare = root.join("r.git");
        git(root, &["clone", "-q", "--bare", work.to_str().unwrap(), bare.to_str().unwrap()]);
        bare
    }
}

#[cfg(test)]
mod tests {
    use super::testnet::{hermetic, Answer, Server};
    use super::*;
    use std::cell::Cell;
    use std::path::{Path, PathBuf};

    const LOOP4: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
    const LOOP6: IpAddr = IpAddr::V6(Ipv6Addr::LOCALHOST);

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wt-fetchpolicy-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }

    /// `dir/gitconfig` holding `text`: the host's config, as the test needs it.
    fn host_config(dir: &Path, text: &str) -> PathBuf {
        let p = dir.join("gitconfig");
        std::fs::write(&p, text).unwrap();
        p
    }

    fn no_dns(host: &str, _: u16) -> Result<Vec<IpAddr>, String> {
        panic!("an address host was looked up: {host}")
    }

    fn loopback(ip: &IpAddr) -> bool {
        ip.is_loopback()
    }

    /// Rules that stand a loopback server in for a public one: plain http
    /// (no TLS server here), loopback admitted, the test's git config.
    fn seam<'a>(resolve: &'a dyn Fn(&str, u16) -> Result<Vec<IpAddr>, String>, env: &'a [(&'a str, String)]) -> Rules<'a> {
        Rules { schemes: &["http"], resolve, admit: &loopback, git_env: env }
    }

    /// `git ls-remote <url>` under `policy`, bounded.
    fn ls_remote(policy: &Policy, url: &str, env: &[(&str, String)]) -> std::process::Output {
        let mut cmd = policy.git(Vec::<&str>::new(), ["ls-remote", "--", url], env);
        cmd.current_dir(std::env::temp_dir());
        crate::proc::run_deadline(cmd, 30).expect("git ran")
    }

    /// (main)'s ask: every scheme, one by one. https is the only one a device
    /// gets, and the refusals need no process and no lookup at all.
    #[test]
    fn a_device_fetches_over_https_and_nothing_else() {
        let d = scratch("schemes");
        let env = hermetic(&d, &host_config(&d, ""));
        let rules = Rules { schemes: DEVICE_SCHEMES, resolve: &no_dns, admit: &global, git_env: &env };
        let pin = Pin::resolve_with("https://140.82.112.3/o/r.git", &rules).unwrap();
        assert_eq!((pin.host.as_str(), pin.port, pin.addrs.len()), ("140.82.112.3", 443, 0));
        assert!(Pin::resolve_with("https://[2606:4700:4700::1111]:8443/o/r.git", &rules).is_ok(), "a global v6 address");
        for url in [
            "http://140.82.112.3/o/r.git",
            "ssh://git@140.82.112.3/o/r.git",
            "git@140.82.112.3:o/r.git",
            "140.82.112.3:o/r.git",
            "git+ssh://git@140.82.112.3/o/r.git",
            "ssh+git://git@140.82.112.3/o/r.git",
            "git://140.82.112.3/o/r.git",
            "file:///etc",
            "FILE:///etc",
            "/etc",
            "./x",
            "~/x",
            "ext::sh -c touch% /tmp/pwned",
            "fd::17",
            "HTTPS://140.82.112.3/o/r.git",
            "ftp://140.82.112.3/o/r.git",
        ] {
            let e = Pin::resolve_with(url, &Rules::device()).unwrap_err();
            assert!(!e.is_empty(), "{url}");
        }
        let e = Pin::resolve_with("git@github.com:o/r.git", &Rules::device()).unwrap_err();
        assert!(e.contains("https only") && e.contains("ssh"), "{e}");
    }

    /// The address check, and the spellings an address hides behind. None of
    /// these reaches a lookup (the resolver here panics) or a process.
    #[test]
    fn a_device_may_not_name_the_hosts_own_network() {
        let rules = Rules { schemes: DEVICE_SCHEMES, resolve: &no_dns, admit: &global, git_env: &[] };
        for url in [
            "https://localhost/x",
            "https://a.localhost/x",
            "https://127.0.0.1/x",
            "https://[::1]/x",
            "https://10.0.0.2/x",
            "https://192.168.1.4/x",
            "https://100.64.1.1/x",
            "https://169.254.169.254/latest",
            "https://[::ffff:127.0.0.1]/x",
            "https://[fd00::1]/x",
            "https://0.0.0.0/x",
            "https://127.1/x",
            "https://0x7f.0.0.1/x",
            "https://2130706433/x",
            "https://example.0x10/x",
            "https://[fe80::1%25en0]/x",
            "https://user@evil@140.82.112.3/x",
            "https://ex\\ample.com/x",
            "https://140.82.112.3/o/r.git?tab=readme",
            "https://140.82.112.3:0/x",
            "https://140.82.112.3:99999/x",
        ] {
            assert!(Pin::resolve_with(url, &rules).is_err(), "{url}");
        }
        // `-c key=value` splits at the first '=': refused with that reason,
        // not left to git's own parser.
        let e = Pin::resolve_with("https://140.82.112.3/o/r.git?tab=readme", &rules).unwrap_err();
        assert!(e.contains("'='"), "{e}");
    }

    /// Resolve once, check every address: one non-global answer among
    /// global ones refuses the fetch, and nothing is looked up twice.
    #[test]
    fn every_address_a_name_resolves_to_is_checked_and_it_is_asked_once() {
        let d = scratch("every");
        let env = hermetic(&d, &host_config(&d, ""));
        let calls = Cell::new(0);
        let answer: Cell<Vec<IpAddr>> = Cell::new(Vec::new());
        let resolve = |_: &str, _: u16| -> Result<Vec<IpAddr>, String> {
            calls.set(calls.get() + 1);
            Ok(answer.take())
        };
        let rules = Rules { schemes: DEVICE_SCHEMES, resolve: &resolve, admit: &global, git_env: &env };
        answer.set(vec!["140.82.112.3".parse().unwrap(), "127.0.0.1".parse().unwrap()]);
        let e = Pin::resolve_with("https://forge.example/o/r.git", &rules).unwrap_err();
        assert!(e.contains("127.0.0.1"), "{e}");
        answer.set(vec![]);
        assert!(Pin::resolve_with("https://forge.example/o/r.git", &rules).is_err(), "no address is no fetch");
        answer.set(vec!["140.82.112.3".parse().unwrap(), "2606:4700::1".parse().unwrap()]);
        let pin = Pin::resolve_with("https://forge.example/o/r.git", &rules).unwrap();
        assert_eq!(calls.get(), 3, "one lookup per resolution");
        assert_eq!(pin.resolve_entry().unwrap(), "forge.example:443:140.82.112.3,[2606:4700::1]");
    }

    /// The brief's first case. For a device, a local server is refused
    /// before git runs; and with the server standing in for a PUBLIC one, its
    /// 302 to the host's loopback is not followed, even when the host's own
    /// config turns redirects on for exactly that URL.
    #[test]
    fn a_redirect_to_the_hosts_own_network_is_not_followed() {
        let d = scratch("redirect");
        let witness = Server::start(LOOP4, 0, Answer::Missing).unwrap();
        let to = format!("http://127.0.0.1:{}", witness.port());
        let public = Server::start(LOOP4, 0, Answer::Redirect(to)).unwrap();
        let url = format!("http://127.0.0.1:{}/r.git", public.port());

        assert!(Pin::resolve(&url).is_err());
        assert!(public.hits().is_empty(), "refused before any request");

        for (tag, cfg) in [("plain", String::new()), ("hostile", format!("[http \"{url}\"]\n\tfollowRedirects = true\n"))] {
            let env = hermetic(&d, &host_config(&d, &cfg));
            let pin = Pin::resolve_with(&url, &seam(&no_dns, &env)).unwrap();
            let out = ls_remote(&Policy::Pinned(pin), &url, &env);
            let err = String::from_utf8_lossy(&out.stderr);
            assert!(!out.status.success(), "{tag}: the fetch went through: {err}");
            assert!(err.contains("302"), "{tag}: {err}");
            assert!(!public.hits().is_empty(), "{tag}: the public server was asked");
            assert!(witness.hits().is_empty(), "{tag}: the redirect was followed: {:?}", witness.hits());
        }
        // The control: today's git follows it, which is what made this a hole.
        let env = hermetic(&d, &host_config(&d, ""));
        let _ = ls_remote(&Policy::Console, &url, &env);
        assert!(!witness.hits().is_empty(), "the console's git no longer follows a first redirect: re-check the control");
    }

    /// The brief's second case: a name that answers differently the second
    /// time. Our lookup is the first; git's own is simulated by a
    /// `curloptResolve` in the host's config, generic and then per-URL, which
    /// sends it to a second address on the same port. The pin wins, and the
    /// scripted resolver is never asked again.
    #[test]
    fn the_pin_wins_over_a_second_lookup() {
        let d = scratch("rebind");
        let public = Server::start(LOOP4, 0, Answer::Missing).unwrap();
        let port = public.port();
        // The second answer: ::1 on the same port. A server there when this
        // host has IPv6 loopback, so a dial to it is SEEN, not just refused.
        let other = Server::start(LOOP6, port, Answer::Missing);
        let url = format!("http://rebind.test:{port}/r.git");
        let calls = Cell::new(0);
        let resolve = |host: &str, _: u16| -> Result<Vec<IpAddr>, String> {
            assert_eq!(host, "rebind.test");
            calls.set(calls.get() + 1);
            Ok(if calls.get() == 1 { vec![LOOP4] } else { vec![LOOP6] })
        };
        for (tag, cfg) in [
            ("generic", format!("[http]\n\tcurloptResolve = rebind.test:{port}:[::1]\n")),
            ("per-url", format!("[http \"{url}\"]\n\tcurloptResolve = rebind.test:{port}:[::1]\n")),
        ] {
            calls.set(0);
            let env = hermetic(&d, &host_config(&d, &cfg));
            let pin = Pin::resolve_with(&url, &seam(&resolve, &env)).unwrap();
            assert_eq!(pin.addrs, vec![LOOP4], "{tag}");
            let before = public.hits().len();
            let out = ls_remote(&Policy::Pinned(pin), &url, &env);
            let hits = public.hits();
            assert!(hits.len() > before, "{tag}: the pinned address was never dialled: {}", String::from_utf8_lossy(&out.stderr));
            assert_eq!(hits.last().unwrap().host, format!("rebind.test:{port}"), "{tag}");
            assert_eq!(calls.get(), 1, "{tag}: resolved more than once");
            if let Some(o) = &other {
                assert!(o.hits().is_empty(), "{tag}: git's own lookup won: {:?}", o.hits());
            }
        }
    }

    /// A proxy resolves the name itself, so it would carry the fetch past the
    /// pin: one from the environment the call site passes, and one the host's
    /// config sets for exactly that URL.
    #[test]
    fn a_proxy_cannot_carry_the_fetch_elsewhere() {
        let d = scratch("proxy");
        let public = Server::start(LOOP4, 0, Answer::Missing).unwrap();
        let proxy = Server::start(LOOP4, 0, Answer::Missing).unwrap();
        let via = format!("http://127.0.0.1:{}", proxy.port());
        let url = format!("http://pinned.test:{}/r.git", public.port());
        let one = |_: &str, _: u16| -> Result<Vec<IpAddr>, String> { Ok(vec![LOOP4]) };
        for (tag, cfg, var) in [
            ("env", String::new(), true),
            ("per-url", format!("[http \"{url}\"]\n\tproxy = {via}\n"), false),
            ("remote", format!("[remote \"origin\"]\n\tproxy = {via}\n"), false),
        ] {
            let mut env = hermetic(&d, &host_config(&d, &cfg));
            if var {
                env.push(("http_proxy", via.clone()));
                env.push(("ALL_PROXY", via.clone()));
            }
            let policy = Policy::Pinned(Pin::resolve_with(&url, &seam(&one, &env)).unwrap());
            // A clone names its remote `origin`, whose `remote.origin.proxy`
            // outranks every `http.proxy`; `ls-remote` has no named remote,
            // so it is the `http.*` keys alone that hold there.
            let before = public.hits().len();
            let mut cmd = policy.git(Vec::<&str>::new(), ["clone", "--", url.as_str()], &env);
            cmd.arg(d.join(format!("c-{tag}"))).current_dir(&d);
            let _ = crate::proc::run_deadline(cmd, 30).unwrap();
            assert!(public.hits().len() > before, "{tag}: clone: the pinned server was not asked");
            let before = public.hits().len();
            let _ = ls_remote(&policy, &url, &env);
            assert!(public.hits().len() > before, "{tag}: ls-remote: the pinned server was not asked");
            assert!(proxy.hits().is_empty(), "{tag}: the fetch went through the proxy: {:?}", proxy.hits());
        }
    }

    /// The host's `insteadOf` rules: a shorter prefix is outranked by the
    /// URL's own mapping, and an exact one (a tie, the host's) is refused
    /// before any request.
    #[test]
    fn the_hosts_rewrites_cannot_move_the_fetch() {
        let d = scratch("rewrite");
        let public = Server::start(LOOP4, 0, Answer::Missing).unwrap();
        let elsewhere = Server::start(LOOP4, 0, Answer::Missing).unwrap();
        let url = format!("http://127.0.0.1:{}/r.git", public.port());
        let dest = format!("http://127.0.0.1:{}/", elsewhere.port());

        let prefix = format!("[url \"{dest}\"]\n\tinsteadOf = http://127.0.0.1:{}/\n", public.port());
        let env = hermetic(&d, &host_config(&d, &prefix));
        let pin = Pin::resolve_with(&url, &seam(&no_dns, &env)).unwrap();
        let _ = ls_remote(&Policy::Pinned(pin), &url, &env);
        assert!(!public.hits().is_empty(), "the URL itself was not fetched");
        assert!(elsewhere.hits().is_empty(), "a prefix rewrite moved the fetch");

        let exact = format!("[url \"{dest}r.git\"]\n\tinsteadOf = {url}\n");
        let env = hermetic(&d, &host_config(&d, &exact));
        let e = Pin::resolve_with(&url, &seam(&no_dns, &env)).unwrap_err();
        assert!(e.contains("insteadOf"), "{e}");
        assert!(elsewhere.hits().is_empty());
    }

    /// v2 lets the server name other URLs (packfile-uris, which git takes
    /// unasked, and bundle URIs); a device's fetch never speaks it.
    #[test]
    fn a_device_fetch_speaks_protocol_v1() {
        let d = scratch("v1");
        let public = Server::start(LOOP4, 0, Answer::Missing).unwrap();
        let url = format!("http://127.0.0.1:{}/r.git", public.port());
        let env = hermetic(&d, &host_config(&d, "[protocol]\n\tversion = 2\n"));
        let pin = Pin::resolve_with(&url, &seam(&no_dns, &env)).unwrap();
        let _ = ls_remote(&Policy::Pinned(pin), &url, &env);
        let hits = public.hits();
        assert!(!hits.is_empty());
        assert!(hits.iter().all(|h| !h.protocol.contains("version=2")), "{hits:?}");
    }

    /// The positive case, end to end: a pinned clone of a real repository
    /// over smart HTTP works, lands where the pin says, and keeps
    /// `http.followRedirects=false` in the new repository.
    #[test]
    fn a_pinned_clone_works_and_keeps_redirects_off() {
        let d = scratch("clone");
        super::testnet::bare_repo(&d);
        let server = Server::start(LOOP4, 0, Answer::Git(d.clone())).unwrap();
        let url = format!("http://forge.test:{}/r.git", server.port());
        let one = |_: &str, _: u16| -> Result<Vec<IpAddr>, String> { Ok(vec![LOOP4]) };
        let env = hermetic(&d, &host_config(&d, ""));
        let pin = Pin::resolve_with(&url, &seam(&one, &env)).unwrap();
        let target = d.join("cloned");
        let mut cmd = Policy::Pinned(pin).git(Vec::<&str>::new(), ["clone", "--", url.as_str()], &env);
        cmd.arg(&target).current_dir(&d);
        let out = crate::proc::run_deadline(cmd, 30).unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert!(target.join("pinned/SKILL.md").is_file());
        let hits = server.hits();
        assert!(hits.iter().all(|h| h.host == format!("forge.test:{}", server.port())), "{hits:?}");
        assert!(hits.iter().any(|h| h.path.ends_with("/git-upload-pack")), "not smart HTTP: {hits:?}");
        let kept = crate::childenv::command("git")
            .args(["config", "--local", "--get", "http.followRedirects"])
            .current_dir(&target)
            .envs(env.iter().map(|(k, v)| (*k, v.as_str())))
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&kept.stdout).trim(), "false");
    }

    /// The console's git is today's: nothing added to argv, nothing to the
    /// environment, whatever the URL (a local file, a loopback address).
    #[test]
    fn the_console_runs_todays_git() {
        for url in ["file:///srv/r.git", "http://127.0.0.1/x", "git@localhost:x"] {
            assert_eq!(Fetcher::Console.policy(url).unwrap(), Policy::Console);
        }
        let env = [("GIT_TERMINAL_PROMPT", "0".to_string()), ("http_proxy", "http://p:1".to_string())];
        let cmd = Policy::Console.git(["-c", "protocol.file.allow=always"], ["clone", "--", "file:///srv/r.git"], &env);
        let args: Vec<_> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args, ["-c", "protocol.file.allow=always", "clone", "--", "file:///srv/r.git"]);
        let envs: Vec<_> = cmd
            .get_envs()
            .filter(|(k, _)| env.iter().any(|(e, _)| OsStr::new(e) == *k) || OsStr::new("GIT_ALLOW_PROTOCOL") == *k)
            .map(|(k, v)| (k.to_string_lossy().into_owned(), v.map(|v| v.to_string_lossy().into_owned())))
            .collect();
        assert_eq!(
            envs,
            [("GIT_TERMINAL_PROMPT".to_string(), Some("0".to_string())), ("http_proxy".to_string(), Some("http://p:1".to_string()))]
        );
    }

    /// The policy has the last word: a call site's own `-c` before it, and
    /// its own variables, cannot loosen a device's fetch.
    #[test]
    fn a_call_site_cannot_loosen_a_device_fetch() {
        let pin = Pin {
            url: "https://forge.example/o/r.git".into(),
            host: "forge.example".into(),
            port: 443,
            addrs: vec!["140.82.112.3".parse().unwrap()],
            allow: vec!["https".into()],
        };
        let env = [("GIT_ALLOW_PROTOCOL", "https:file".to_string()), ("GIT_SSL_NO_VERIFY", "1".to_string())];
        let cmd = Policy::Pinned(pin).git(["-c", "protocol.file.allow=always"], ["clone", "--", "https://forge.example/o/r.git"], &env);
        let args: Vec<_> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        let at = |s: &str| args.iter().position(|a| a == s).unwrap_or_else(|| panic!("no {s} in {args:?}"));
        assert!(at("protocol.file.allow=never") > at("protocol.file.allow=always"));
        assert!(at("protocol.file.allow=never") < at("clone"));
        assert_eq!(&args[at("clone") + 1..at("clone") + 3], ["--config", "http.followRedirects=false"]);
        for kv in [
            "http.https://forge.example/o/r.git.followRedirects=false",
            "http.https://forge.example/o/r.git.curloptResolve=forge.example:443:140.82.112.3",
            "url.https://forge.example/o/r.git.insteadOf=https://forge.example/o/r.git",
            "protocol.version=1",
        ] {
            at(kv);
        }
        let envs: std::collections::HashMap<_, _> =
            cmd.get_envs().map(|(k, v)| (k.to_string_lossy().into_owned(), v.map(|v| v.to_string_lossy().into_owned()))).collect();
        assert_eq!(envs["GIT_ALLOW_PROTOCOL"], Some("https".into()));
        assert_eq!(envs["GIT_SSL_NO_VERIFY"], None, "removed, not passed");
    }

    /// A device stays a device, and its pins are found by the URL git is
    /// handed; any other URL is resolved and checked afresh.
    #[test]
    fn a_device_fetcher_narrows_and_never_lets_an_unpinned_url_through() {
        let pin = Pin {
            url: "https://forge.example/o/r.git".into(),
            host: "forge.example".into(),
            port: 443,
            addrs: vec!["140.82.112.3".parse().unwrap()],
            allow: vec!["https".into()],
        };
        let dev = Fetcher::Device(vec![pin.clone()]);
        assert_eq!(dev.narrow(Fetcher::Console), dev);
        assert_eq!(Fetcher::Console.narrow(dev.clone()), dev);
        assert_eq!(dev.policy("https://forge.example/o/r.git").unwrap(), Policy::Pinned(pin));
        assert!(dev.policy("file:///etc").is_err());
        assert!(dev.policy("https://127.0.0.1/o/r.git").is_err());
    }

    #[test]
    fn git_versions_parse() {
        assert_eq!(git_version("git version 2.54.0 (Apple Git-157)\n"), Some((2, 54)));
        assert_eq!(git_version("git version 2.37.0"), Some((2, 37)));
        assert!(git_version("git version 2.34.1").unwrap() < PINNING_GIT);
        assert_eq!(git_version("nonsense"), None);
    }

    /// The classifier, moved here from serve's resolver with its cases.
    #[test]
    fn host_local_refuses_every_block_a_host_can_hide_behind() {
        for ip in [
            "0.1.2.3", "10.0.0.1", "127.0.0.1", "100.64.1.1", "169.254.169.254", "172.16.0.1", "192.168.1.1", "192.0.2.1",
            "198.18.0.1", "198.51.100.1", "203.0.113.1", "224.0.0.1", "255.255.255.255", "::1", "::", "::ffff:127.0.0.1",
            "::127.0.0.1", "64:ff9b::7f00:1", "2002:7f00:1::1", "2001::1", "2001:db8::1", "fd00::1", "fe80::1", "fec0::1",
            "ff02::1",
        ] {
            assert!(host_local(&ip.parse().unwrap()), "{ip}");
        }
        for ip in ["140.82.112.3", "1.1.1.1", "2606:4700:4700::1111", "::ffff:8.8.8.8", "64:ff9b::808:808"] {
            assert!(!host_local(&ip.parse().unwrap()), "{ip}");
        }
    }

    /// An IPv4 address written in IPv6 is judged as the one it carries,
    /// through the decoder the remote listener's classifier shares: local
    /// inside is refused, global inside is not. The global half is what shows
    /// the decoder runs at all — every form but 6to4 sits outside 2000::/3,
    /// so without it the local half passes anyway.
    #[test]
    fn an_embedded_ipv4_address_is_judged_as_the_one_it_carries() {
        for (local, global) in [
            ("::ffff:10.0.0.1", "::ffff:8.8.8.8"),
            ("::10.0.0.1", "::8.8.8.8"),
            ("64:ff9b::10.0.0.1", "64:ff9b::8.8.8.8"),
            ("2002:a00:1::1", "2002:808:808::1"),
        ] {
            assert!(host_local(&local.parse().unwrap()), "{local}");
            assert!(!host_local(&global.parse().unwrap()), "{global}");
        }
    }
}
