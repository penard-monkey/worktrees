//! The documentation server — one loopback HTTP server, N places as N routes.
//!
//! `docs.rs` decides which documents a place has, `derive.rs` decides what each
//! one looks like once a viewer gets hold of it, and `viewer.rs` writes the
//! derived tree. This is what serves it, and it is the only part of the feature
//! that holds a port — so it is the only part that can leak a client's signed
//! agreement to a web page, and almost every rule below is a refusal.
//!
//! **Why this module exists at all.** The previous implementation handed the
//! tree to `mo`, a third-party viewer, which does not validate the `Host`
//! header (proposal §13.2). A loopback bind keeps other MACHINES out; it does
//! not keep out the browser on THIS one, which runs code from strangers. A page
//! on `evil.com` re-resolves its own domain to `127.0.0.1`, is then same-origin
//! with the server, and reads every document in every place — no CORS check
//! applies, because from the browser's point of view nothing is cross-origin.
//! A proxy in front does not fix it (§13.3: the page reaches the real port
//! directly). One line of server code does, and it is the line below.
//!
//! **Why hyper and not a hand-rolled parser.** The `docs-transport` findings
//! recommend `std` alone, and their own §8.2 says the recommendation flips
//! wherever hyper is already compiled in — it is, by way of tauri, so this adds
//! `httpdate` to the lock and nothing else. The argument matters more than the
//! dependency: the `mo` failure was a missing POLICY check, not a parser bug,
//! and answering "a third party's HTTP server had a header bug" with "so we
//! will write our own HTTP server" is a strange move. What is written here is
//! routing and policy, which is where the bug actually lived.
//!
//! The refusals, each with the attack it answers:
//!
//! 1. **`Host` must name the loopback interface.** The browser sends the
//!    authority it believes it is talking to and cannot be scripted into lying,
//!    so this is what stops DNS rebinding. Anything else is `403`.
//! 2. **An `Origin` that is not ours is `403`, and no CORS header is ever
//!    sent.** Our own page fetches same-origin, which sends no `Origin` at all;
//!    `null` — every `file://` page and every sandboxed iframe on the web —
//!    is refused with the rest.
//! 3. **A per-launch path token.** Defence in depth behind the `Host` check,
//!    and what makes N places behind one port safe to enumerate: a wrong token
//!    is a flat `404` that does not confirm the route shape. It is in the PATH,
//!    never a cookie — cookies on `http://127.0.0.1` are not port-scoped, so
//!    every other loopback server on this machine would share the jar.
//! 4. **Nothing outside the generated tree.** Percent-decoded ONCE (decoding
//!    twice is the classic traversal bug), every component checked, then
//!    canonicalised and required to still be under the tree.
//!    `symlink_metadata`, never `metadata`, for the same
//!    `docs/logo.png -> ~/.ssh/id_rsa` that `docs.rs` refuses.
//! 5. **`GET` only, no request body, no keep-alive.** One request per
//!    connection means no pipelining and therefore no desync between what we
//!    parsed and what a later read would parse; the whole request-smuggling
//!    family is refused at the door rather than parsed correctly.
//! 6. **`Referrer-Policy: no-referrer` on every response.** Easy to miss and it
//!    matters: the token is in the URL, so without it, following an external
//!    link out of a document hands that site the capability in `Referer`.

use std::net::{Ipv4Addr, SocketAddr, TcpListener as StdListener};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::header::{HeaderValue, CACHE_CONTROL, CONTENT_TYPE, ETAG, HOST, IF_NONE_MATCH, ORIGIN};
use hyper::{Method, Request, Response, StatusCode};

use crate::viewer::{Group, DOC_MAX_BYTES, PLACE_KEY_MAX};

/// How long one connection gets, start to finish. A loopback client that has
/// opened a socket and then says nothing holds a task and a file descriptor;
/// this is the only thing that ever ends such a connection, because there is no
/// keep-alive timeout to do it (there is no keep-alive).
const CONN_DEADLINE: std::time::Duration = std::time::Duration::from_secs(20);

/// How long the accept loop waits after a failed `accept()`. Long enough that a
/// persistent error (the process is out of file descriptors) costs 20 wakeups a
/// second rather than a pegged core, short enough that the one dropped
/// connection behind a transient one is not felt.
const ACCEPT_RETRY: std::time::Duration = std::time::Duration::from_millis(50);

/// The content type served for each extension in `viewer::ASSET_EXTS`. Paired
/// with the allow-list rather than sniffed: `X-Content-Type-Options: nosniff`
/// is only worth sending if we are sure of the type we send with it.
const ASSET_TYPES: [(&str, &str); 8] = [
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("avif", "image/avif"),
    ("bmp", "image/bmp"),
    // Only safe because of `ASSET_CSP` and `nosniff` below — see
    // `viewer::ASSET_EXTS`, which carries the long version. An SVG served
    // without them is a document that runs script.
    ("svg", "image/svg+xml"),
];

/// The page's Content-Security-Policy.
///
/// `default-src 'none'` and then only what the shell needs: its own bundle, its
/// own fetches, and the images the derived tree carries. No `'unsafe-eval'`, no
/// third-party origin, nothing that phones home — proposal §4.3's "zero
/// outbound calls" as a header rather than as a promise. `style-src` allows
/// inline because a diagram renderer injects a `<style>` element and cannot be
/// made not to.
const SHELL_CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
     img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self'; \
     frame-ancestors 'none'; base-uri 'none'; form-action 'none'";

/// What an asset is served with. §6.4 of the findings: owning the server is
/// what turns "SVG can never be served" into a header — a document served with
/// `default-src 'none'` cannot run script even when navigated to directly.
/// Sent on every asset, not only the ones that could carry script, because an
/// allow-list that grows is the thing that forgets.
/// Re-admitting `svg` to `viewer::ASSET_EXTS` and sending this header are ONE
/// decision, so the allow-list's own test reads this constant — see
/// `viewer::tests::an_svg_is_copied_only_because_the_response_makes_it_inert`.
pub(crate) const ASSET_CSP: &str = "default-src 'none'";

// ── the running server ───────────────────────────────────────────────────────

/// A running server. Held by `viewer::Viewer`, one per app launch at most.
pub struct Handle {
    pub port: u16,
    pub token: String,
    /// The accept loop. `is_finished()` is this module's `Child::try_wait` —
    /// the same rule `Shells` keeps and the one `viewer.rs` kept for `mo`:
    /// liveness is asked at the point of use, never by a timer.
    task: tokio::task::JoinHandle<()>,
}

impl Handle {
    /// Is the accept loop still running? Asked on every open, so a server that
    /// died takes its slot with it and the next open starts a fresh one.
    pub fn alive(&self) -> bool {
        !self.task.is_finished()
    }

    /// Stop accepting and drop the listener, which releases the port.
    ///
    /// In-flight connections are their own tasks and are not aborted: each is
    /// one request with `Connection: close` on it, so each ends on its own
    /// within `CONN_DEADLINE`. Called from `RunEvent::Exit`, where the process
    /// is about to go anyway, and from the respawn path, where the only cost of
    /// a straggler is that it finishes answering a request we already made.
    pub fn stop(self) {
        self.task.abort();
    }

    /// Stop it WITHOUT consuming the handle, so a test can leave a dead server
    /// in the slot and watch the tick refuse to resurrect it. Not a production
    /// path: the only legitimate way to end a server is `stop`, which takes the
    /// slot with it.
    #[cfg(test)]
    pub fn abort_for_test(&self) {
        self.task.abort();
    }
}

/// Everything a request is answered from. Cloned into each connection task, so
/// every field is cheap to clone.
#[derive(Clone)]
struct Ctx {
    token: Arc<str>,
    port: u16,
    /// The places the server may answer about. Shared with `viewer`, which
    /// replaces entries on the tick.
    ///
    /// **Locked briefly and never across IO.** A request clones the handful of
    /// fields it needs and drops the guard before it touches the disk; the only
    /// long hold is `viewer::refresh`'s own derive, which a request may wait
    /// out. At one poll per second per tab that is a wait nobody can observe,
    /// and the alternative — a second copy of the registry — is a drift bug.
    places: Arc<Mutex<Vec<Group>>>,
    /// The browser bundle on disk, if there is one. Read per request rather
    /// than cached: it is built by a separate worktree and replaced under a
    /// running app during development, and a cache would serve the old one.
    bundle: Option<PathBuf>,
}

/// Bind a loopback port and start serving. Must be called inside a tokio
/// runtime; `open_docs_viewer` is an `async fn`, so it is.
///
/// **Bound with `std` and handed over.** The listener exists before this
/// returns, so the port in the URL is a port something is already listening on
/// — there is no window in which the frontend has a URL and the server has not
/// started, which is the whole class of "started but never listened" failure
/// the previous implementation needed a three-second polling deadline for.
pub fn start(
    places: Arc<Mutex<Vec<Group>>>,
    bundle: Option<PathBuf>,
) -> Result<Handle, String> {
    // 127.0.0.1 only, as a rule and not a default: these documents carry a
    // client's signed agreement (§4.3), and a docs server reachable from the
    // LAN is a data leak with a nice font. Port 0 — the OS picks, and we hold
    // the socket, so nothing can take it between the pick and the bind.
    let std_listener = StdListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .map_err(|e| format!("the documentation server could not bind a loopback port: {e}"))?;
    let port = std_listener.local_addr().map_err(|e| e.to_string())?.port();
    std_listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let listener = tokio::net::TcpListener::from_std(std_listener).map_err(|e| e.to_string())?;

    let token = new_token();
    let ctx = Ctx { token: Arc::from(token.as_str()), port, places, bundle };

    let task = tokio::spawn(async move {
        // Said once per run of failures, not once per failure: `EMFILE` does
        // not clear on its own and a line every 50 ms would be the log.
        let mut said = false;
        loop {
            let (stream, peer) = match listener.accept().await {
                Ok(v) => {
                    said = false;
                    v
                }
                Err(e) => {
                    // **An accept error is per connection — except when it is
                    // not.** A client that hung up between SYN and accept must
                    // not take the port down, so this continues; but tokio
                    // clears readiness only on `WouldBlock`, so a process-wide
                    // `EMFILE`/`ENFILE` makes the next `accept()` return `Err`
                    // immediately and this loop pegs a worker until some other
                    // task happens to close a descriptor — with nothing logged
                    // and `alive()` still reporting a healthy server. The sleep
                    // is hyper's `sleep_on_errors` shape: it costs a dropped
                    // connection nothing (there is no connection) and turns a
                    // spin into a retry.
                    if !said {
                        said = true;
                        crate::applog("error", &format!("docs server: accept failed: {e}"));
                    }
                    tokio::time::sleep(ACCEPT_RETRY).await;
                    continue;
                }
            };
            // The bind already guarantees this; asserted anyway, because it is
            // one comparison and the cost of being wrong is the whole feature.
            if !peer.ip().is_loopback() {
                continue;
            }
            let ctx = ctx.clone();
            tokio::spawn(async move {
                let io = hyper_util::rt::TokioIo::new(stream);
                let svc = hyper::service::service_fn(move |req: Request<hyper::body::Incoming>| {
                    let ctx = ctx.clone();
                    async move { Ok::<_, std::convert::Infallible>(respond(&ctx, &req)) }
                });
                let conn = hyper::server::conn::http1::Builder::new()
                    // No keep-alive: one request per connection, so there is no
                    // pipelining and therefore nothing for a smuggled second
                    // request to be pipelined into. hyper sends
                    // `Connection: close` for us and closes after the response.
                    .keep_alive(false)
                    .serve_connection(io, svc);
                let _ = tokio::time::timeout(CONN_DEADLINE, conn).await;
            });
        }
    });
    Ok(Handle { port, token, task })
}

/// A per-launch, unguessable path token: 16 bytes of the OS CSPRNG as hex.
///
/// Regenerated every launch, so a URL that escaped into a bookmark or a chat
/// window stops working when the app is restarted — it is a capability, and a
/// capability that outlives the process it belongs to is a password.
fn new_token() -> String {
    let mut b = [0u8; 16];
    // A failure here is the OS refusing entropy, which is not a state this
    // process can serve documents in. The fallback is deliberately NOT a clock
    // and a pid: a token anyone can reconstruct is not a token, and a server
    // that comes up with a guessable one is worse than one that does not come
    // up at all.
    getrandom::fill(&mut b).expect("the OS refused entropy for the documentation server's token");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// ── policy: the pure parts, which is where the tests live ────────────────────

/// Is this `Host` one of the authorities we are willing to be?
///
/// **This is the gate**, and it is the thing `mo` did not have. Only the
/// loopback interface, by literal or by the one name that cannot be pointed
/// anywhere else by a remote party, and only on OUR port — a request naming a
/// different port is not a request to this server however loopback it looks.
///
/// A missing `Host` is a refusal too. HTTP/1.1 requires one; "I could not tell"
/// and "it is ours" must never collapse into the same branch, because only one
/// of them is safe.
pub fn host_ok(host: Option<&str>, port: u16) -> bool {
    let Some(h) = host else { return false };
    if h.is_empty() || h.len() > 260 || h.contains('@') {
        return false;
    }
    let (name, given_port) = if let Some(rest) = h.strip_prefix('[') {
        // An IPv6 literal's colons are not the port's, and what follows the
        // `]` is a port or nothing — `[::1].evil.example` is the bracketed
        // spelling of the dotted-suffix trick.
        match rest.split_once(']') {
            Some((n, "")) => (n, None),
            Some((n, tail)) => match tail.strip_prefix(':') {
                Some(p) => (n, Some(p)),
                None => return false,
            },
            None => return false,
        }
    } else {
        match h.split_once(':') {
            Some((n, p)) => (n, Some(p)),
            None => (h, None),
        }
    };
    if let Some(p) = given_port {
        if p.parse::<u16>().map(|p| p != port).unwrap_or(true) {
            return false;
        }
    }
    is_loopback_name(name)
}

/// `localhost`, any literal in `127.0.0.0/8`, or `::1`. Nothing else, and in
/// particular nothing that merely ends in one of them.
fn is_loopback_name(name: &str) -> bool {
    if name.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if name == "::1" || name == "0:0:0:0:0:0:0:1" {
        return true;
    }
    match name.parse::<std::net::IpAddr>() {
        Ok(ip) => ip.is_loopback(),
        Err(_) => false,
    }
}

/// An `Origin` header we are willing to answer.
///
/// Our own page fetches same-origin, which sends no `Origin` at all, so the
/// common case is `None`. Anything present must be exactly an origin we could
/// have emitted; `null` — which is what every `file://` page and every
/// sandboxed iframe sends — is refused with the rest, and no
/// `Access-Control-Allow-Origin` is ever sent in reply to anything.
pub fn origin_ok(origin: Option<&str>, port: u16) -> bool {
    let Some(o) = origin else { return true };
    let Some(rest) = o.strip_prefix("http://") else { return false };
    if rest.contains('/') {
        return false;
    }
    host_ok(Some(rest), port) && rest.contains(':')
}

/// Compare the token without an early return, so the comparison does not
/// describe the token by how long it took. Cheap, and the alternative is an
/// argument about whether this particular oracle is exploitable.
pub fn token_ok(given: &str, want: &str) -> bool {
    if given.len() != want.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in given.bytes().zip(want.bytes()) {
        diff |= a ^ b;
    }
    diff == 0
}

/// A place's route segment: what `viewer::place_key` builds, checked again on
/// the way in. A segment that is not this shape cannot name a place we
/// registered, so it is `404` before anything is looked up.
pub fn place_key_ok(k: &str) -> bool {
    !k.is_empty()
        && k.len() <= PLACE_KEY_MAX
        && k.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

/// The five routes of the contract, and everything else.
#[derive(Debug, PartialEq)]
pub enum Route<'a> {
    /// `GET /<token>/p/<place>/` — the viewer shell.
    Shell(&'a str),
    /// `GET /<token>/p/<place>` — the same page, one slash short.
    ///
    /// Not an alias: it is a REDIRECT, because the shell names its bundle and
    /// its endpoints relatively. Without the trailing slash the browser
    /// resolves them one segment too high (`/<token>/p/viewer.js`), and the
    /// page loads as a blank document with a 404 in the console — a failure
    /// that reads as a broken build rather than as a missing character.
    ShellRedirect(&'a str),
    /// `GET /<token>/p/<place>/doc?path=<rel>`
    Doc(&'a str),
    /// `GET /<token>/p/<place>/index`
    Index(&'a str),
    /// `GET /<token>/p/<place>/asset/<rel>` — `rel` still percent-encoded.
    Asset(&'a str, &'a str),
    /// `GET /<token>/viewer.js`
    Bundle,
    NotFound,
}

/// Parse a request path. A wrong or missing token is a flat `NotFound` — the
/// same answer as a route that does not exist, so probing cannot tell the two
/// apart and cannot learn the shape of the routes behind the token.
pub fn route<'a>(path: &'a str, token: &str) -> Route<'a> {
    let Some(rest) = path.strip_prefix('/') else { return Route::NotFound };
    let (given, rest) = rest.split_once('/').unwrap_or((rest, ""));
    if !token_ok(given, token) {
        return Route::NotFound;
    }
    if rest == "viewer.js" {
        return Route::Bundle;
    }
    let Some(rest) = rest.strip_prefix("p/") else { return Route::NotFound };
    // The split is what tells `…/p/<place>` from `…/p/<place>/`; both reach
    // the same page and only one of them can resolve a relative URL.
    let Some((place, tail)) = rest.split_once('/') else {
        return if place_key_ok(rest) { Route::ShellRedirect(rest) } else { Route::NotFound };
    };
    if !place_key_ok(place) {
        return Route::NotFound;
    }
    match tail {
        "" => Route::Shell(place),
        "doc" => Route::Doc(place),
        "index" => Route::Index(place),
        t => match t.strip_prefix("asset/") {
            Some(rel) if !rel.is_empty() => Route::Asset(place, rel),
            _ => Route::NotFound,
        },
    }
}

/// Percent-decode, **once**.
///
/// Once is the whole point. Decoding twice is the classic traversal bug: the
/// caller checks for `..`, the second pass turns `%2e%2e` into `..` behind the
/// check, and the path leaves the tree. `None` for a malformed escape or for
/// bytes that are not UTF-8 — a filename we cannot spell is a filename we do
/// not serve.
pub fn pct_decode_once(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            if i + 2 >= b.len() {
                return None;
            }
            let h = hexval(b[i + 1])?;
            let l = hexval(b[i + 2])?;
            out.push(h * 16 + l);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn hexval(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Resolve a decoded relative path inside `root`, or refuse it.
///
/// Two layers, and the second is the one that holds. The first is arithmetic on
/// a string — no empty, `.` or `..` component, nothing absolute, no backslash
/// (a Windows separator that this platform would treat as an ordinary
/// character in a name), no drive letter. A string cannot show a symlink, so
/// the second layer stats the candidate with `symlink_metadata` (never
/// `metadata`: a link is refused, not followed) and then canonicalises it,
/// which resolves every parent component, and requires the result to still be
/// under the canonical root.
pub fn safe_under(root: &Path, rel: &str) -> Option<PathBuf> {
    if rel.is_empty() || rel.len() > 1024 || rel.contains('\0') || rel.contains('\\') {
        return None;
    }
    if rel.starts_with('/') || rel.as_bytes().get(1) == Some(&b':') {
        return None;
    }
    for part in rel.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return None;
        }
    }
    let cand = root.join(rel);
    let md = std::fs::symlink_metadata(&cand).ok()?;
    if !md.is_file() {
        return None;
    }
    let canon = std::fs::canonicalize(&cand).ok()?;
    // Canonical on BOTH sides: `starts_with` compares components, so a root
    // that still contains a symlink (`/tmp` is `/private/tmp` here) would fail
    // to match its own files and refuse everything.
    let canon_root = std::fs::canonicalize(root).ok()?;
    canon.starts_with(&canon_root).then_some(canon)
}

/// One value of a query string, percent-decoded once. `+` is NOT read as a
/// space: this is a path, and a file legitimately called `a+b.md` must not
/// become `a b.md`.
pub fn query_get(query: Option<&str>, key: &str) -> Option<String> {
    for pair in query.unwrap_or("").split('&') {
        let (k, v) = pair.split_once('=')?;
        if k == key {
            return pct_decode_once(v);
        }
    }
    None
}

/// Percent-encode a path going into the shell's **fragment** — the deep link
/// this module emits into a mermaid `click` directive and hands to `openUrl`.
///
/// Conservative on purpose: everything but the unreserved set and `/` is
/// escaped, which is `encodeURIComponent` per segment and then joined, so the
/// page's own `decodeURIComponent` gives the path back exactly. Escaping MORE
/// than `encodeURIComponent` does is not caution here, it is correctness —
/// `!'()*` round-trip either way, but a literal `#` in a filename would
/// otherwise end the fragment at the wrong place and a literal `?` would be
/// read as the page's `?h=` anchor separator. Both are legal filenames.
pub fn route_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ── answering ────────────────────────────────────────────────────────────────

/// Build a response with the headers every answer carries.
fn base(status: StatusCode, ctype: &str, body: Vec<u8>) -> Response<Full<Bytes>> {
    let mut r = Response::builder()
        .status(status)
        .header(CONTENT_TYPE, ctype)
        // The token is in the URL. Without this, following an external link out
        // of a document hands that site the capability in `Referer`.
        .header("Referrer-Policy", "no-referrer")
        .header("X-Content-Type-Options", "nosniff")
        .header("X-Frame-Options", "DENY")
        // Store, but revalidate: the page's own `If-None-Match` is how a change
        // is noticed, and a document that is served from a cache without asking
        // is a stale document, which is the one thing this feature exists to
        // stop.
        .header(CACHE_CONTROL, "no-cache");
    if ctype.starts_with("text/html") {
        r = r.header("Content-Security-Policy", SHELL_CSP);
    }
    r.body(Full::new(Bytes::from(body))).expect("a response with static headers")
}

/// A permanent redirect that keeps the method and the query.
fn redirect(to: &str) -> Response<Full<Bytes>> {
    let mut r = text(StatusCode::PERMANENT_REDIRECT, "this page lives one slash further on");
    if let Ok(v) = HeaderValue::from_str(to) {
        r.headers_mut().insert(hyper::header::LOCATION, v);
    }
    r
}

fn text(status: StatusCode, msg: &str) -> Response<Full<Bytes>> {
    base(status, "text/plain; charset=utf-8", format!("{msg}\n").into_bytes())
}

/// One request, start to finish.
fn respond(ctx: &Ctx, req: &Request<hyper::body::Incoming>) -> Response<Full<Bytes>> {
    // ── the refusals, before anything is looked up ──────────────────────────
    //
    // `Host` first, because it is the gate: a request that lies about who it is
    // talking to must not reach a route, a token comparison or a filesystem
    // call, so that none of those can be a side channel for a page that should
    // have been refused at the door.
    let headers = req.headers();
    if headers.get_all(HOST).iter().count() > 1 {
        // Two `Host` headers is not an ambiguity to resolve — it is a request
        // built to be read differently by two readers.
        return text(StatusCode::BAD_REQUEST, "duplicate Host header");
    }
    let host = headers.get(HOST).and_then(|v| v.to_str().ok());
    if !host_ok(host, ctx.port) {
        return text(
            StatusCode::FORBIDDEN,
            "this server answers only to the loopback interface it is bound to",
        );
    }
    // A header that is PRESENT and unreadable is not an absent one. `to_str`
    // fails on any byte above 7-bit ASCII, which a `HeaderValue` happily
    // carries, and `and_then(…ok())` folded that into `None` — which
    // `origin_ok` accepts, because our own same-origin page sends no `Origin`
    // at all. The same shape on `Host` one branch up is a refusal, and its
    // docstring says why: "I could not tell" and "it is ours" may not collapse
    // into one answer when only one of them is safe.
    let origin = match headers.get(ORIGIN).map(|v| v.to_str()) {
        None => None,
        Some(Ok(o)) => Some(o),
        Some(Err(_)) => {
            return text(StatusCode::FORBIDDEN, "cross-origin requests are not served")
        }
    };
    if !origin_ok(origin, ctx.port) {
        return text(StatusCode::FORBIDDEN, "cross-origin requests are not served");
    }
    if req.method() != Method::GET {
        return text(StatusCode::METHOD_NOT_ALLOWED, "GET only");
    }
    // No body, on any method. `Transfer-Encoding` on a GET is the other half of
    // the smuggling pair with `Content-Length`, and hyper will already have
    // refused the two together — this refuses either, alone, on a request that
    // has no business carrying one.
    let has_body = headers.contains_key(hyper::header::TRANSFER_ENCODING)
        || headers
            .get(hyper::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .map(|n| n > 0)
            .unwrap_or(false);
    if has_body {
        return text(StatusCode::BAD_REQUEST, "this server reads no request body");
    }

    let path = req.uri().path();
    match route(path, &ctx.token) {
        Route::NotFound => text(StatusCode::NOT_FOUND, "no such route"),
        Route::Bundle => bundle(ctx),
        Route::ShellRedirect(place) => {
            let q = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
            redirect(&format!("/{}/p/{place}/{q}", ctx.token))
        }
        Route::Shell(place) => with_place(ctx, place, |_| shell(place)),
        Route::Index(place) => with_place(ctx, place, |p| index(req, &p)),
        Route::Doc(place) => {
            let want = query_get(req.uri().query(), "path");
            with_place(ctx, place, |p| doc(req, &p, want.as_deref()))
        }
        Route::Asset(place, rel) => with_place(ctx, place, |p| asset(&p, rel)),
    }
}

/// What one request needs to know about a place, cloned out from under the lock
/// so that no filesystem call happens while it is held.
struct Snapshot {
    tree: PathBuf,
    root: PathBuf,
    etag: String,
    meta: Arc<str>,
    index: Arc<str>,
    /// The last derive of this place failed — see `viewer::Group::broken`.
    broken: bool,
}

/// Resolve the route's place, or answer `410 Gone`.
///
/// **`410`, not `404`**, and the difference is the whole reason the contract
/// names it: a place that has been removed is a page that must say so rather
/// than retry forever. `404` reads as "wrong URL" and a polling page would keep
/// asking; `410` is final, and the tab can tell the reader the place is gone.
/// A place whose tree or whose worktree has left the disk is the same state as
/// one that was never registered — `remove_place` drops the registration, and
/// this catches the crash that could not.
fn with_place(
    ctx: &Ctx,
    key: &str,
    f: impl FnOnce(Snapshot) -> Response<Full<Bytes>>,
) -> Response<Full<Bytes>> {
    let snap = {
        let Ok(places) = ctx.places.lock() else {
            return text(StatusCode::INTERNAL_SERVER_ERROR, "the place registry is poisoned");
        };
        places.iter().find(|g| g.key == key).map(|g| Snapshot {
            tree: g.tree.clone(),
            root: g.root.clone(),
            etag: g.etag.clone(),
            meta: g.meta.clone(),
            index: g.index.clone(),
            broken: g.broken,
        })
    };
    let Some(snap) = snap else {
        return text(StatusCode::GONE, "this place is no longer being served");
    };
    if !snap.tree.is_dir() || !snap.root.is_dir() {
        return text(StatusCode::GONE, "this place is no longer on disk");
    }
    // After the two `410`s, deliberately: a place that has been removed says so
    // in the terms the page already handles, and a failed derive on a place
    // that no longer exists is the removal, not a fault.
    //
    // **`503`, because the tree is a MIXTURE.** A derive rewrites every
    // document; one that died part way through left some new and some old,
    // under an `etag` and an `index` that describe the last good pass — so
    // serving it answers `304` for documents that HAVE changed and renders half
    // of one version beside half of another, with a header claiming both are
    // current. The tick retries until one succeeds (`viewer::refresh` rule 3),
    // which is what makes "ask again" the true answer.
    if snap.broken {
        return text(
            StatusCode::SERVICE_UNAVAILABLE,
            "this place's documents could not be re-derived; what is on disk is part old and \
             part new, so nothing is served until the next attempt succeeds",
        );
    }
    f(snap)
}

/// `304` when the client already has this exact body, otherwise `None`.
///
/// The `ETag` incorporates the place's stat-only fingerprint
/// (`docs::fingerprint_with`, folded with its images), so it moves exactly when
/// the documents do — and a `304` is answered without touching the disk at all,
/// which is what makes a one-second poll cost nothing (measured at 0.149 ms and
/// 410 bytes per conditional request).
fn not_modified(req: &Request<hyper::body::Incoming>, etag: &str) -> Option<Response<Full<Bytes>>> {
    let inm = req.headers().get(IF_NONE_MATCH)?.to_str().ok()?;
    // A list, because a browser may send several and `*` means "any".
    let hit = inm == "*" || inm.split(',').any(|t| t.trim().trim_start_matches("W/") == etag);
    if !hit {
        return None;
    }
    let mut r = base(StatusCode::NOT_MODIFIED, "application/json", Vec::new());
    r.headers_mut().insert(ETAG, HeaderValue::from_str(etag).ok()?);
    // A 304 carries no body and no content type; hyper will not send one for an
    // empty `Full`, but the header would be a lie either way.
    r.headers_mut().remove(CONTENT_TYPE);
    Some(r)
}

fn with_etag(mut r: Response<Full<Bytes>>, etag: &str) -> Response<Full<Bytes>> {
    if let Ok(v) = HeaderValue::from_str(etag) {
        r.headers_mut().insert(ETAG, v);
    }
    r
}

/// One place's document list, for navigation and search.
fn index(req: &Request<hyper::body::Incoming>, p: &Snapshot) -> Response<Full<Bytes>> {
    let etag = etag_for(&p.etag, "index", "");
    if let Some(r) = not_modified(req, &etag) {
        return r;
    }
    with_etag(base(StatusCode::OK, "application/json", p.index.as_bytes().to_vec()), &etag)
}

/// One document, as blocks.
fn doc(
    req: &Request<hyper::body::Incoming>,
    p: &Snapshot,
    want: Option<&str>,
) -> Response<Full<Bytes>> {
    let Some(rel) = want else {
        return text(StatusCode::BAD_REQUEST, "doc needs a ?path=");
    };
    // `.md` and nothing else. The tree holds derived documents and copied
    // images; this route serves the documents, `asset` serves the images, and
    // neither is a general file server for the other's files.
    if !rel.to_ascii_lowercase().ends_with(".md") {
        return text(StatusCode::NOT_FOUND, "no such document");
    }
    let Some(file) = safe_under(&p.tree, rel) else {
        return text(StatusCode::NOT_FOUND, "no such document");
    };
    let etag = etag_for(&p.etag, "doc", rel);
    if let Some(r) = not_modified(req, &etag) {
        return r;
    }
    let Ok(text_) = read_capped(&file, DOC_MAX_BYTES) else {
        return text(StatusCode::NOT_FOUND, "that document could not be read");
    };
    let blocks: Vec<serde_json::Value> = worktrees_core::derive::blocks(&text_)
        .into_iter()
        .map(|b| serde_json::json!({ "id": b.id, "md": b.md }))
        .collect();
    let body = format!(
        "{{\"meta\":{},\"blocks\":{}}}",
        p.meta,
        serde_json::to_string(&blocks).unwrap_or_else(|_| "[]".into())
    );
    with_etag(base(StatusCode::OK, "application/json", body.into_bytes()), &etag)
}

/// One image out of the derived tree.
fn asset(p: &Snapshot, rel_encoded: &str) -> Response<Full<Bytes>> {
    let Some(rel) = pct_decode_once(rel_encoded) else {
        return text(StatusCode::NOT_FOUND, "no such asset");
    };
    let ext = rel.rsplit('/').next().unwrap_or("").rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    let Some(ctype) = ext.as_deref().and_then(|e| ASSET_TYPES.iter().find(|(x, _)| *x == e)).map(|(_, t)| *t)
    else {
        return text(StatusCode::NOT_FOUND, "no such asset");
    };
    let Some(file) = safe_under(&p.tree, &rel) else {
        return text(StatusCode::NOT_FOUND, "no such asset");
    };
    let Ok(bytes) = std::fs::read(&file) else {
        return text(StatusCode::NOT_FOUND, "that asset could not be read");
    };
    let mut r = base(StatusCode::OK, ctype, bytes);
    // An image served by URL is a document the browser can be navigated to.
    // `sandbox` and `default-src 'none'` are what make that harmless whatever
    // the bytes turn out to be.
    r.headers_mut().insert("Content-Security-Policy", HeaderValue::from_static(ASSET_CSP));
    r
}

/// The browser bundle, straight off disk.
fn bundle(ctx: &Ctx) -> Response<Full<Bytes>> {
    let Some(path) = ctx.bundle.as_deref() else {
        return text(
            StatusCode::NOT_FOUND,
            "this build carries no documentation viewer bundle (set WORKTREES_VIEWER_JS to one)",
        );
    };
    match std::fs::read(path) {
        Ok(b) => base(StatusCode::OK, "text/javascript; charset=utf-8", b),
        Err(e) => text(StatusCode::NOT_FOUND, &format!("the viewer bundle could not be read: {e}")),
    }
}

/// The element the bundle mounts on — `app/viewer/main.tsx`'s
/// `document.getElementById(…)`.
///
/// It is a constant with a test behind it because the two halves of this
/// feature were built in separate worktrees and disagreed about it: the server
/// emitted `app`, the bundle looked for `root`, and the result was a **blank
/// page with no console error** — a missing mount point is not an exception, so
/// nothing anywhere said a word. Every route answered 200 and the whole thing
/// was silently dead. `the_shell_mounts_where_the_bundle_looks` reads the id
/// back out of `main.tsx` rather than repeating it here, so the two cannot
/// drift apart again without going red.
const MOUNT_ID: &str = "root";

/// What the mount point says until the bundle replaces it.
///
/// **It states the failure, not the success.** The text used to be "loading the
/// documents viewer…", which is indistinguishable from the one state worth
/// naming: a build with no `viewer.js`, where the script 404s, no exception is
/// raised anywhere and that placeholder is the whole of the page, forever.
/// React's `createRoot(...).render(...)` clears the container's children on its
/// first commit, so on every working build this is on screen for one loopback
/// round-trip and then gone.
///
/// **Why not `onerror` on the `<script>`, which is the obvious fix.** It is an
/// inline event handler, and `SHELL_CSP` is `script-src 'self'` with no
/// `'unsafe-inline'` — the browser would refuse to run it. A handler that never
/// fires is the same silent failure it was added to remove, and buying it back
/// with `'unsafe-hashes'` would weaken the one header standing between a
/// document's markup and this origin. Static text needs no script at all.
/// `viewer::open` is the other half: since it refuses an open outright when
/// there is no bundle on disk, the only way to see this is a bundle that
/// vanished or broke under a running app.
const FALLBACK: &str = "Loading the documents viewer… If this line stays, viewer.js did not \
     load: this build may carry no browser bundle (see the app log, and Settings \u{2192} \
     Diagnostics).";

/// The shell: a small HTML file that loads the bundle and gets out of the way.
///
/// It carries no data of its own — no facts, no document, not even the place's
/// name in prose — because everything it could carry would be a second copy of
/// something the page fetches live, and a second copy is a copy that can be
/// stale. The place key is here only so the bundle need not parse its own URL.
fn shell(place: &str) -> Response<Full<Bytes>> {
    let p = esc(place);
    let html = format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n\
         <meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <meta name=\"referrer\" content=\"no-referrer\">\n\
         <meta name=\"worktrees-place\" content=\"{p}\">\n\
         <title>{p} — docs</title>\n\
         </head>\n<body>\n<div id=\"{MOUNT_ID}\">{FALLBACK}</div>\n\
         <script src=\"../../viewer.js\" defer></script>\n\
         </body>\n</html>\n"
    );
    base(StatusCode::OK, "text/html; charset=utf-8", html.into_bytes())
}

/// The five characters that can end an attribute or start a tag. The place key
/// is already `[a-z0-9-]` by `place_key_ok`, so this escapes nothing in
/// practice — it is here so that a future route with a less constrained segment
/// cannot make this function the hole.
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Read a file, refusing one over the cap rather than truncating it. A
/// truncated document renders as a document, which is the silent wrongness this
/// whole feature exists to remove.
fn read_capped(path: &Path, cap: u64) -> std::io::Result<String> {
    use std::io::Read;
    let f = std::fs::File::open(path)?;
    let mut buf = Vec::new();
    f.take(cap + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > cap {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "over the size cap"));
    }
    String::from_utf8(buf).map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "not UTF-8"))
}

/// The `ETag` for one resource of one place.
///
/// The place's digest is the whole of the change signal — it is stat-only over
/// every document and every image the last derive knew about, which is what
/// makes it cheap enough to compute on a three-second tick. The route and the
/// document's own path are folded in so that two resources of one place cannot
/// share a tag.
///
/// The cost, stated: the digest is PER PLACE, so editing one document changes
/// every document's tag in that place and a tab reading a different one pays
/// one extra full response. The alternative is hashing each file's bytes on
/// every conditional request, which is the disk read that the `304` exists to
/// avoid.
pub fn etag_for(place_digest: &str, kind: &str, rel: &str) -> String {
    format!("\"{place_digest}-{kind}-{:016x}\"", fnv(rel.as_bytes()))
}

fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};

    const PORT: u16 = 6275;

    // ── the policy, without a socket ────────────────────────────────────────

    /// **The gate, as one assertion.** Everything else in this module is
    /// reachable only past this test's subject: a server that answers a request
    /// claiming to be somewhere else is a server any web page can read, and the
    /// shape it must never take is "it looked fine, so we carried on".
    #[test]
    fn only_the_loopback_interface_may_claim_to_be_talking_to_us() {
        for good in [
            "127.0.0.1:6275",
            "127.0.0.1",
            "localhost:6275",
            "LOCALHOST:6275",
            "[::1]:6275",
            "[::1]",
            // 127.0.0.0/8 is all loopback, and a machine may be configured to
            // use any of it.
            "127.5.5.5:6275",
        ] {
            assert!(host_ok(Some(good), PORT), "{good} must be served");
        }
        for bad in [
            // The whole of §13.2: a page on evil.com whose DNS has been rebound
            // to 127.0.0.1 sends THIS, and cannot be scripted into sending
            // anything else.
            "evil.example.com",
            "evil.example.com:6275",
            // A host that merely begins or ends with the literal.
            "127.0.0.1.evil.example",
            "not-127.0.0.1",
            "[::1].evil.example",
            // Userinfo: the loopback literal is the USER here and the browser
            // connects to evil.example.
            "127.0.0.1@evil.example",
            // Our own name on somebody else's port is not our server.
            "127.0.0.1:6276",
            "localhost:80",
            // 10.0.0.1 is reachable from the LAN. So is 0.0.0.0.
            "10.0.0.1:6275",
            "0.0.0.0:6275",
            "[::]:6275",
            "",
            // A port that is not a number at all.
            "127.0.0.1:abc",
        ] {
            assert!(!host_ok(Some(bad), PORT), "{bad} must be refused");
        }
        // Absent is refused too. "I could not tell" and "it is ours" must never
        // collapse into the same branch — only one of them is safe.
        assert!(!host_ok(None, PORT));
    }

    /// Our own page fetches same-origin and sends no `Origin` at all. Anything
    /// that sends one is, by construction, not us — including `null`, which is
    /// what every `file://` page and every sandboxed iframe on the web sends,
    /// and which is therefore the one value that would let an attacker in while
    /// looking like an exception for our own tooling.
    #[test]
    fn any_origin_that_is_not_ours_is_refused() {
        assert!(origin_ok(None, PORT), "a same-origin fetch sends no Origin");
        assert!(origin_ok(Some("http://127.0.0.1:6275"), PORT));
        assert!(origin_ok(Some("http://localhost:6275"), PORT));
        for bad in [
            "null",
            "https://evil.example.com",
            "http://evil.example.com:6275",
            "http://127.0.0.1:6276",
            // No port: not an origin we could have emitted.
            "http://127.0.0.1",
            "http://127.0.0.1:6275/x",
            "file://",
            "",
        ] {
            assert!(!origin_ok(Some(bad), PORT), "{bad} must be refused");
        }
    }

    /// A wrong token and a route that does not exist must be the SAME answer,
    /// or probing tells an attacker which half they got right.
    #[test]
    fn a_wrong_token_is_indistinguishable_from_a_wrong_route() {
        let t = "0123456789abcdef";
        assert_eq!(route("/nope/p/a/index", t), Route::NotFound);
        assert_eq!(route("/p/a/index", t), Route::NotFound);
        assert_eq!(route("//p/a/index", t), Route::NotFound);
        assert_eq!(route("/0123456789abcde/p/a/index", t), Route::NotFound, "a prefix is not the token");
        assert_eq!(route("/0123456789abcdefX/p/a/index", t), Route::NotFound);
        assert_eq!(route("", t), Route::NotFound);
    }

    /// The five routes of the contract, and the sixth that only redirects.
    #[test]
    fn the_contracts_routes_are_the_only_ones_that_exist() {
        let t = "tok";
        assert_eq!(route("/tok/p/place/", t), Route::Shell("place"));
        assert_eq!(route("/tok/p/place", t), Route::ShellRedirect("place"));
        assert_eq!(route("/tok/p/place/doc", t), Route::Doc("place"));
        assert_eq!(route("/tok/p/place/index", t), Route::Index("place"));
        assert_eq!(route("/tok/p/place/asset/docs/a.png", t), Route::Asset("place", "docs/a.png"));
        assert_eq!(route("/tok/viewer.js", t), Route::Bundle);
        // And nothing else, including the shapes that look like they ought to
        // work.
        for miss in [
            "/tok/",
            "/tok/p/",
            "/tok/p/place/doc/",
            "/tok/p/place/asset/",
            "/tok/p/place/other",
            "/tok/p/PLACE/index",
            "/tok/p/../index",
            "/tok/viewer.js/x",
        ] {
            assert_eq!(route(miss, t), Route::NotFound, "{miss}");
        }
    }

    /// Decoded ONCE. Decoding twice is the classic traversal bug: the caller
    /// checks for `..`, a second pass turns `%2e%2e` into `..` behind the
    /// check, and the path leaves the tree.
    #[test]
    fn a_path_is_percent_decoded_exactly_once() {
        assert_eq!(pct_decode_once("a%2Fb").as_deref(), Some("a/b"));
        assert_eq!(pct_decode_once("%2e%2e%2fetc").as_deref(), Some("../etc"));
        // Double-encoded: one pass yields the literal `%2e%2e`, which is a
        // filename and not a traversal. A second pass would yield `..`.
        assert_eq!(pct_decode_once("%252e%252e%252f").as_deref(), Some("%2e%2e%2f"));
        assert_eq!(pct_decode_once("plain.md").as_deref(), Some("plain.md"));
        // Malformed, and bytes that are not a name we can spell.
        assert_eq!(pct_decode_once("%"), None);
        assert_eq!(pct_decode_once("%2"), None);
        assert_eq!(pct_decode_once("%zz"), None);
        assert_eq!(pct_decode_once("%ff%fe"), None);
    }

    /// Nothing outside the generated tree, whatever shape the request takes.
    #[test]
    fn nothing_outside_the_tree_resolves() {
        let root = std::env::temp_dir().join(format!("wt-docsrv-safe-{}", std::process::id()));
        let out = std::env::temp_dir().join(format!("wt-docsrv-out-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(root.join("docs/a.md"), "# a\n").unwrap();
        std::fs::write(out.join("secret.md"), "not ours\n").unwrap();
        std::os::unix::fs::symlink(out.join("secret.md"), root.join("docs/link.md")).unwrap();
        // A link pointing INSIDE the tree. The containment check below cannot
        // refuse this one — it resolves to a file that really is under the root
        // — so it is the case that says whether `symlink_metadata` is load
        // bearing or decoration. Nothing we write into this tree is a symlink,
        // so anything that is one arrived by another route.
        std::os::unix::fs::symlink(root.join("docs/a.md"), root.join("docs/inside.md")).unwrap();

        assert!(safe_under(&root, "docs/a.md").is_some(), "the one real document did not resolve");
        for bad in [
            "",
            "/etc/passwd",
            "../secret.md",
            "docs/../../secret.md",
            "docs//a.md",
            "./docs/a.md",
            "docs/a.md/",
            // A symlink out of the tree, refused twice over: as a symlink,
            // and by the containment check behind it.
            "docs/link.md",
            // A symlink INTO the tree, which only `symlink_metadata` refuses.
            "docs/inside.md",
            // A directory is not a document.
            "docs",
            "a\\b.md",
            "C:/x.md",
            "docs/nope.md",
        ] {
            assert!(safe_under(&root, bad).is_none(), "{bad:?} resolved");
        }
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&out);
    }

    // ── the same policy, over a real socket ─────────────────────────────────

    /// A place on disk with one document and one image, and a server in front
    /// of it. Real sockets: the pure tests above are the policy, and this is
    /// whether the policy is WIRED — delete the `host_ok` call from `respond`
    /// and every pure test still passes.
    struct Live {
        h: Handle,
        tree: PathBuf,
        /// The same `Vec` the server reads, so a test can change what is
        /// registered under a running server — which is the only way to reach
        /// the states `viewer::refresh` puts a place into.
        places: Arc<Mutex<Vec<Group>>>,
    }

    /// Stops the server however the test ends.
    ///
    /// A `#[test]` that panics before its own teardown leaves a REAL server
    /// holding a REAL port on the machine of whoever ran it — a test suite
    /// reproducing, in miniature, the exact failure the lifecycle rules exist
    /// to prevent. It happened once in this lineage, on port 53381, and was
    /// found by hand.
    impl Drop for Live {
        fn drop(&mut self) {
            self.h.abort_for_test();
            let _ = std::fs::remove_dir_all(&self.tree);
        }
    }

    fn live(name: &str) -> Live {
        let tree = std::env::temp_dir().join(format!("wt-docsrv-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tree);
        let place = tree.join("one");
        std::fs::create_dir_all(place.join("docs")).unwrap();
        std::fs::write(place.join("README.md"), "# Read me\n\nfirst\n\nsecond\n").unwrap();
        std::fs::write(place.join("docs/a.png"), crate::viewer::tests::PNG).unwrap();
        // Something outside the place but inside the temp dir, so a traversal
        // that escaped by one segment would have a real file to find.
        std::fs::write(tree.join("outside.md"), "not ours\n").unwrap();

        let places = Arc::new(Mutex::new(vec![crate::viewer::test_group(
            "one",
            &place,
            &place,
            "{\"place\":\"one\",\"behind\":25}",
            "{\"meta\":{\"place\":\"one\"},\"entries\":[{\"path\":\"README.md\",\"title\":\"Read me\",\"group\":\"\"}]}",
        )]));
        let _g = crate::viewer::tests::rt().enter();
        let h = start(places.clone(), None).unwrap();
        Live { h, tree, places }
    }

    /// One raw request, byte for byte as given. Raw sockets rather than `curl`
    /// deliberately: **curl silently collapses duplicate `Host` headers** and
    /// sends only one, which made the first run of that case a false pass in
    /// the findings' own gate (§6.1). A test that cannot express the request
    /// cannot test the refusal.
    fn raw(port: u16, req: &str) -> String {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let mut s = std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(5))
            .expect("the server is listening");
        s.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
        s.write_all(req.as_bytes()).unwrap();
        s.flush().unwrap();
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out);
        String::from_utf8_lossy(&out).into_owned()
    }

    fn get(port: u16, path: &str, host: &str, extra: &str) -> String {
        raw(port, &format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n{extra}Connection: close\r\n\r\n"))
    }

    fn status(resp: &str) -> u16 {
        resp.lines()
            .next()
            .and_then(|l| l.split(' ').nth(1))
            .and_then(|c| c.parse().ok())
            .unwrap_or(0)
    }

    fn header(resp: &str, name: &str) -> Option<String> {
        resp.split("\r\n\r\n").next()?.lines().skip(1).find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case(name).then(|| v.trim().to_string())
        })
    }

    fn body(resp: &str) -> String {
        resp.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default()
    }

    /// **BOTH directions, which is the whole of the CI gate.** A server that
    /// answered `403` to everything would pass a one-sided check while serving
    /// nothing at all — the same class of "the test passes and the feature is
    /// broken" this repo keeps finding — so the loopback half is as load-bearing
    /// as the forged one.
    #[test]
    fn a_forged_host_is_refused_and_a_loopback_one_is_served() {
        let l = live("gate");
        let (p, t) = (l.h.port, l.h.token.clone());
        let u = format!("/{t}/p/one/index");

        let foreign = get(p, &u, "worktrees-viewer-probe.invalid", "");
        assert_eq!(status(&foreign), 403, "{foreign}");
        // And it is a refusal, not a redirect or an empty 403 with the
        // documents in it.
        assert!(!body(&foreign).contains("README"), "{foreign}");

        let loopback = get(p, &u, &format!("127.0.0.1:{p}"), "");
        assert_eq!(status(&loopback), 200, "{loopback}");
        assert!(body(&loopback).contains("README.md"), "{loopback}");

    }

    /// Two `Host` headers is not an ambiguity to resolve — it is a request built
    /// to be read one way by one reader and another way by the next.
    #[test]
    fn a_duplicate_host_header_is_refused_rather_than_resolved() {
        let l = live("duphost");
        let (p, t) = (l.h.port, l.h.token.clone());
        let resp = raw(
            p,
            &format!(
                "GET /{t}/p/one/index HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nHost: evil.example\r\nConnection: close\r\n\r\n"
            ),
        );
        assert!(status(&resp) == 400 || status(&resp) == 403, "{resp}");
        assert!(!body(&resp).contains("README"), "{resp}");
    }

    /// No CORS header is ever sent, to anyone — a cross-origin request is
    /// refused rather than answered with a policy that says "not you".
    #[test]
    fn a_cross_origin_request_is_refused_and_nothing_says_otherwise() {
        let l = live("origin");
        let (p, t) = (l.h.port, l.h.token.clone());
        let host = format!("127.0.0.1:{p}");
        for o in ["https://evil.example", "null", &format!("http://evil.example:{p}")] {
            let resp = get(p, &format!("/{t}/p/one/index"), &host, &format!("Origin: {o}\r\n"));
            assert_eq!(status(&resp), 403, "{o}: {resp}");
        }
        let ok = get(p, &format!("/{t}/p/one/index"), &host, "");
        assert_eq!(status(&ok), 200);
        for r in [&ok] {
            assert!(header(r, "access-control-allow-origin").is_none(), "a CORS header was sent: {r}");
        }
    }

    /// A wrong token, a missing one, and a traversal are all the same flat
    /// `404`, and none of them reaches a file.
    /// **A present `Origin` we cannot read is a refusal, not an absence.**
    /// `to_str()` fails on any byte above 7-bit ASCII, and the header used to
    /// be read as `None` — which `origin_ok` accepts. The same shape on `Host`
    /// is a `403` whose docstring says "I could not tell" and "it is ours" must
    /// never collapse into one branch; this is that rule, applied to the header
    /// beside it.
    #[test]
    fn an_origin_we_cannot_read_is_refused_rather_than_ignored() {
        let l = live("origin-unreadable");
        let (p, t) = (l.h.port, l.h.token.clone());
        let host = format!("127.0.0.1:{p}");
        // Legal in a `HeaderValue` (obs-text), refused by `to_str`.
        let bad = get(p, &format!("/{t}/p/one/index"), &host, "Origin: http://\u{e9}.example\r\n");
        assert_eq!(status(&bad), 403, "an unreadable Origin was served:\n{bad}");
        // And the readable, correct one is still served — both directions, or
        // a server that refuses everything passes half of this.
        let ours = get(p, &format!("/{t}/p/one/index"), &host, &format!("Origin: http://{host}\r\n"));
        assert_eq!(status(&ours), 200, "{ours}");
    }

    #[test]
    fn a_missing_token_or_a_traversal_reaches_nothing() {
        let l = live("token");
        let (p, t) = (l.h.port, l.h.token.clone());
        let host = format!("127.0.0.1:{p}");
        for path in [
            "/p/one/index".to_string(),
            "/deadbeef/p/one/index".to_string(),
            format!("/{t}/p/one/doc?path=../outside.md"),
            format!("/{t}/p/one/doc?path=%2e%2e%2foutside.md"),
            format!("/{t}/p/one/doc?path=%252e%252e%252foutside.md"),
            format!("/{t}/p/one/doc?path=/etc/passwd"),
            format!("/{t}/p/one/asset/../outside.md"),
            format!("/{t}/p/one/doc?path=docs/../../outside.md"),
        ] {
            let resp = get(p, &path, &host, "");
            assert_eq!(status(&resp), 404, "{path}: {resp}");
            assert!(!body(&resp).contains("not ours"), "{path} served a file outside the tree");
        }
    }

    /// `GET` only, and no request body on anything.
    #[test]
    fn only_get_is_served_and_no_body_is_read() {
        let l = live("method");
        let (p, t) = (l.h.port, l.h.token.clone());
        let post = raw(
            p,
            &format!(
                "POST /{t}/p/one/index HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nContent-Length: 4\r\nConnection: close\r\n\r\nabcd"
            ),
        );
        assert_eq!(status(&post), 405, "{post}");
        let bodied = raw(
            p,
            &format!(
                "GET /{t}/p/one/index HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nContent-Length: 4\r\nConnection: close\r\n\r\nabcd"
            ),
        );
        assert_eq!(status(&bodied), 400, "{bodied}");
    }

    /// The conditional GET the whole transport rests on: the page sends what it
    /// has, and a place that has not moved answers `304` with no body at all.
    #[test]
    fn an_unchanged_document_is_answered_304_with_nothing_in_it() {
        let l = live("etag");
        let (p, t) = (l.h.port, l.h.token.clone());
        let host = format!("127.0.0.1:{p}");
        let first = get(p, &format!("/{t}/p/one/doc?path=README.md"), &host, "");
        assert_eq!(status(&first), 200, "{first}");
        let etag = header(&first, "etag").expect("a doc must carry an ETag");
        let v: serde_json::Value = serde_json::from_str(&body(&first)).unwrap();
        assert_eq!(v["meta"]["behind"], 25, "the meta was not spliced in");
        let blocks = v["blocks"].as_array().unwrap();
        assert!(blocks.len() >= 2, "{v}");
        assert!(blocks[0]["id"].as_str().unwrap().starts_with('b'), "{v}");

        let again = get(
            p,
            &format!("/{t}/p/one/doc?path=README.md"),
            &host,
            &format!("If-None-Match: {etag}\r\n"),
        );
        assert_eq!(status(&again), 304, "{again}");
        assert_eq!(body(&again), "", "a 304 carried a body");

        // The index too, and with a tag of its own — two resources of one place
        // may not share one.
        let idx = get(p, &format!("/{t}/p/one/index"), &host, "");
        assert_ne!(header(&idx, "etag").unwrap(), etag);
    }

    /// A place that is no longer served answers `410 Gone`, not `404`. The
    /// difference is the contract's, and it is the difference between a page
    /// that can say "this place was removed" and a page that polls a hole
    /// forever.
    #[test]
    fn a_place_that_is_gone_says_so_rather_than_404() {
        let l = live("gone");
        let (p, t) = (l.h.port, l.h.token.clone());
        let host = format!("127.0.0.1:{p}");
        let resp = get(p, &format!("/{t}/p/two/index"), &host, "");
        assert_eq!(status(&resp), 410, "{resp}");
        // …and a key that could never have been a place is still a 404, so the
        // two answers keep meaning different things.
        let resp = get(p, &format!("/{t}/p/NOT_A_KEY/index"), &host, "");
        assert_eq!(status(&resp), 404, "{resp}");
    }

    /// An asset states its own type and forbids everything else.
    ///
    /// These two headers are what let `svg` back onto `viewer::ASSET_EXTS`
    /// after `mo` forced it off: a navigated SVG under `default-src 'none'`
    /// can neither run its inline script nor fetch anything, and `nosniff`
    /// stops the type being second-guessed. Remove either and an SVG in any
    /// cloned repository is live again.
    /// **A place whose last derive failed is not served at all.** The derive
    /// rewrites every document, so one that dies part way through leaves the
    /// tree a mixture of two versions under an `etag` that still describes the
    /// old one — a polling tab is answered `304` for documents that HAVE
    /// changed, and a fresh load reads half of each with a header claiming both
    /// are current. `503` says "ask again", which is true: the tick retries
    /// until one succeeds.
    #[test]
    fn a_place_whose_derive_failed_is_not_served_as_if_it_had_not() {
        let l = live("mixture");
        let (p, t) = (l.h.port, l.h.token.clone());
        let host = format!("127.0.0.1:{p}");
        let ok = get(p, &format!("/{t}/p/one/index"), &host, "");
        assert_eq!(status(&ok), 200, "{ok}");

        l.places.lock().unwrap()[0].broken = true;
        for route in ["index", "doc?path=README.md", "", "asset/docs/a.png"] {
            let r = get(p, &format!("/{t}/p/one/{route}"), &host, "");
            assert_eq!(status(&r), 503, "/{route} served a mixture:\n{r}");
        }
    }

    #[test]
    fn an_asset_states_its_type_and_forbids_everything_else() {
        let l = live("asset");
        let (p, t) = (l.h.port, l.h.token.clone());
        let host = format!("127.0.0.1:{p}");
        let resp = get(p, &format!("/{t}/p/one/asset/docs/a.png"), &host, "");
        assert_eq!(status(&resp), 200, "{resp}");
        assert_eq!(header(&resp, "content-type").as_deref(), Some("image/png"));
        assert_eq!(header(&resp, "x-content-type-options").as_deref(), Some("nosniff"));
        assert_eq!(header(&resp, "content-security-policy").as_deref(), Some("default-src 'none'"));
        assert_eq!(header(&resp, "referrer-policy").as_deref(), Some("no-referrer"));
    }

    /// The token is in the URL, so a document's outbound link must not carry it
    /// to the site it points at — and one request per connection is what keeps
    /// the whole request-smuggling family out.
    #[test]
    fn every_response_withholds_the_referrer_and_closes_the_connection() {
        let l = live("headers");
        let (p, t) = (l.h.port, l.h.token.clone());
        let host = format!("127.0.0.1:{p}");
        for path in [format!("/{t}/p/one/"), format!("/{t}/p/one/index"), "/nope".to_string()] {
            let resp = get(p, &path, &host, "");
            assert_eq!(header(&resp, "referrer-policy").as_deref(), Some("no-referrer"), "{path}");
            assert_eq!(header(&resp, "connection").as_deref(), Some("close"), "{path}");
            assert_eq!(header(&resp, "x-content-type-options").as_deref(), Some("nosniff"), "{path}");
        }
    }

    /// The Rust deep link must be the shape the page's router reads.
    ///
    /// Three copies of "a document is `#/<rel>`" exist — `viewer::place_url`
    /// here, `routeHash` in `app/viewer/Viewer.tsx`, and `parseRoute` beside
    /// it — and nothing tied them together until this. That gap already cost
    /// one silent defect: `place_url` emitted `?path=<rel>`, the page routes on
    /// `location.hash` and ignores the query entirely, so the Docs tab's
    /// per-row action and every mermaid drill-down opened the right PLACE at
    /// the wrong DOCUMENT, with both halves passing their own tests. Same
    /// shape as the mount-id bug: the contract named the routes and never said
    /// how a document is named in a URL.
    ///
    /// So this reads the prefix out of `routeHash` rather than restating it —
    /// the drift-check shape this repo uses for `dnd.ts::predictTier`.
    #[test]
    fn the_deep_link_is_the_fragment_the_page_routes_on() {
        let src = std::fs::read_to_string(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../viewer/Viewer.tsx"),
        )
        .expect("app/viewer/Viewer.tsx — the page's router; renamed?");
        let body = src
            .split_once("export function routeHash")
            .map(|(_, rest)| rest)
            .expect("routeHash is gone from Viewer.tsx — find what names a document now");
        assert!(
            body.contains("`#/${p}`"),
            "routeHash no longer builds `#/<path>`; place_url would emit a URL the page cannot route:\n{}",
            body.lines().take(8).collect::<Vec<_>>().join("\n"),
        );

        let url = crate::viewer::place_url(1234, "tok", "place", Some("docs/a.md"));
        assert_eq!(url, "http://127.0.0.1:1234/tok/p/place/#/docs/a.md", "{url}");
        assert!(
            !url.contains('?'),
            "a query is not read by the page's router — that was the bug: {url}",
        );
    }

    /// The shell must mount where the bundle actually looks.
    ///
    /// This is the test whose absence let the two halves of this feature ship
    /// past each other: the server emitted `<div id="app">`, `main.tsx` called
    /// `getElementById("root")`, and every route answered 200 while the page
    /// rendered **nothing at all** — no exception, no console message, because
    /// a mount point that is not there is not an error, it is an absence. Two
    /// worktrees each measured their own half green.
    ///
    /// So it reads the id back out of `main.tsx` instead of restating it. A
    /// literal here would be a third copy of the same fact and would pass
    /// happily while the bundle moved underneath it — the drift-check shape
    /// this repo already uses for `dnd.ts::predictTier` and the usage tokens.
    #[test]
    fn the_shell_mounts_where_the_bundle_looks() {
        let src = std::fs::read_to_string(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../viewer/main.tsx"),
        )
        .expect("app/viewer/main.tsx — the bundle's entry point; renamed?");
        let want = src
            .split_once("getElementById(\"")
            .and_then(|(_, rest)| rest.split_once('"'))
            .map(|(id, _)| id.to_string())
            .expect("main.tsx no longer mounts with getElementById — find what it does now");
        assert_eq!(
            MOUNT_ID, want,
            "the shell mounts #{MOUNT_ID} and the bundle looks for #{want}: the page would render nothing, silently",
        );

        let l = live("mount");
        let (p, t) = (l.h.port, l.h.token.clone());
        let host = format!("127.0.0.1:{p}");
        let body = body(&get(p, &format!("/{t}/p/one/"), &host, ""));
        assert!(
            body.contains(&format!("id=\"{want}\"")),
            "the served shell carries no #{want} for the bundle to mount on:\n{body}",
        );
    }

    /// **The mount point states the failure, and it cannot be an `onerror`.**
    /// A build with no `viewer.js` used to leave "loading the documents
    /// viewer…" on screen forever: the script 404s, no exception is raised, and
    /// nothing anywhere says a word. The obvious fix — `onerror` on the
    /// `<script>` — is an INLINE HANDLER, and this page's CSP is
    /// `script-src 'self'` with no `'unsafe-inline'`, so the browser would
    /// refuse to run it: a handler that never fires is the same silence it was
    /// added to remove. So the two halves are asserted together, the way the
    /// SVG allow-list and its header are: static text that React replaces on
    /// mount, and a `script-src` that still forbids inline script.
    #[test]
    fn the_mount_point_says_so_when_the_bundle_never_arrives() {
        let l = live("fallback");
        let (p, t) = (l.h.port, l.h.token.clone());
        let host = format!("127.0.0.1:{p}");
        let body = body(&get(p, &format!("/{t}/p/one/"), &host, ""));
        assert!(
            body.contains("viewer.js did not"),
            "the mount point does not say what it means when it stays:\n{body}",
        );
        assert!(!body.contains("onerror"), "an inline handler this page's CSP would refuse:\n{body}");
        let script = SHELL_CSP
            .split(';')
            .map(str::trim)
            .find(|d| d.starts_with("script-src"))
            .expect("script-src left SHELL_CSP");
        assert!(
            !script.contains("unsafe-inline") && !script.contains("unsafe-hashes"),
            "inline script is allowed again — then an onerror handler is the better fallback: {script}",
        );
    }

    /// The shell loads the bundle and nothing else, and it is only reachable at
    /// the slashed form — the unslashed one redirects, because the bundle is
    /// named relatively and one segment too high is a blank page with a 404 in
    /// the console.
    #[test]
    fn the_shell_redirects_to_the_form_its_own_links_resolve_from() {
        let l = live("shell");
        let (p, t) = (l.h.port, l.h.token.clone());
        let host = format!("127.0.0.1:{p}");

        let bare = get(p, &format!("/{t}/p/one?path=docs/a.md"), &host, "");
        assert_eq!(status(&bare), 308, "{bare}");
        assert_eq!(
            header(&bare, "location").as_deref(),
            Some(format!("/{t}/p/one/?path=docs/a.md").as_str()),
            "the query was dropped on the way through the redirect"
        );

        let shell = get(p, &format!("/{t}/p/one/"), &host, "");
        assert_eq!(status(&shell), 200, "{shell}");
        assert!(body(&shell).contains("src=\"../../viewer.js\""), "{}", body(&shell));
        assert!(header(&shell, "content-security-policy").unwrap().contains("default-src 'none'"));
        assert!(header(&shell, "content-security-policy").unwrap().contains("frame-ancestors 'none'"));
        // No bundle on disk in this fixture, so the route says so rather than
        // serving an empty file the page would fail to parse.
        let js = get(p, &format!("/{t}/viewer.js"), &host, "");
        assert_eq!(status(&js), 404, "{js}");
    }

    /// This machine's own non-loopback IPv4 address, or `None` if it has only
    /// `lo`. `std` cannot enumerate interfaces, so the address is obtained the
    /// portable way: a UDP socket "connected" to an unroutable TEST-NET address
    /// sends no packet, but the kernel picks a source address for the route and
    /// `local_addr` reads it back. Measured to answer on both CI platforms.
    fn own_lan_addr() -> Option<std::net::Ipv4Addr> {
        let s = std::net::UdpSocket::bind((std::net::Ipv4Addr::UNSPECIFIED, 0)).ok()?;
        s.connect((std::net::Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
        match s.local_addr().ok()?.ip() {
            std::net::IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => Some(ip),
            _ => None,
        }
    }

    /// Bound to loopback, and to nothing else. The one-line version of §4.3:
    /// these documents carry a client's signed agreement, and a docs server
    /// reachable from the LAN is a data leak with a nice font.
    ///
    /// **The witness is a connection, because a bind is not portable.** The
    /// first version of this test bound `0.0.0.0:<our port>` and asserted it
    /// succeeded — true of a loopback-only listener on macOS, where it was
    /// measured, and false on Linux, which refuses a wildcard bind over ANY
    /// holder of that port. It passed here and went red in CI for a server that
    /// was bound correctly. Its mirror image (bind our own LAN address) fails
    /// the other way round: it discriminates on Linux and is satisfied by a
    /// wildcard listener on macOS. Measured, all four cases, on both:
    ///
    /// | witness bind | macOS | Linux |
    /// | --- | --- | --- |
    /// | `0.0.0.0:P`  | tells them apart | `EADDRINUSE` either way |
    /// | `<lan ip>:P` | succeeds either way | tells them apart |
    ///
    /// So there is no bind that means the same thing twice, and the rule above
    /// does not talk about binds anyway — it talks about who can REACH this
    /// server. A connect asks that directly and answers identically on both
    /// platforms: refused via the LAN address while loopback-only, accepted via
    /// it the moment the bind widens.
    ///
    /// The loopback connect is not scenery. Without it a dead server passes —
    /// every connect fails, including the one that must — and this test's whole
    /// job is to notice a server listening somewhere it should not be.
    #[test]
    fn the_server_is_bound_to_loopback_and_its_token_is_fresh_each_time() {
        let a = live("bind-a");
        let b = live("bind-b");
        assert_ne!(a.h.token, b.h.token, "two launches shared a token");
        assert_eq!(a.h.token.len(), 32);
        assert!(a.h.token.chars().all(|c| c.is_ascii_hexdigit()));

        let wait = std::time::Duration::from_secs(5);
        let via_lo = SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, a.h.port));
        assert!(
            std::net::TcpStream::connect_timeout(&via_lo, wait).is_ok(),
            "the server did not answer on loopback, so the refusal below would prove nothing"
        );

        match own_lan_addr() {
            // A firewall that DROPs rather than refuses shows up as a timeout,
            // which is also an `Err` — the fail-safe direction.
            Some(ip) => {
                let via_lan = SocketAddr::from((ip, a.h.port));
                assert!(
                    std::net::TcpStream::connect_timeout(&via_lan, wait).is_err(),
                    "{via_lan} answered, so the server is reachable off loopback"
                );
            }
            None => eprintln!("no non-loopback IPv4 on this host; the LAN half did not run"),
        }
    }

    /// The allow-list and the content types are two tables that have to agree:
    /// an extension copied into the tree with no type here is a `404` on an
    /// image that is sitting right there, and a type here with no allow-list
    /// entry is a route onto a file that is never copied.
    #[test]
    fn the_asset_allow_list_and_the_content_types_are_the_same_set() {
        let mut a: Vec<&str> = crate::viewer::ASSET_EXTS.to_vec();
        let mut b: Vec<&str> = ASSET_TYPES.iter().map(|(e, _)| *e).collect();
        a.sort_unstable();
        b.sort_unstable();
        assert_eq!(a, b);
    }
}
