//! The loopback rule set, as pure functions — which is where the tests live.
//!
//! Everything here answers "may this connection proceed?" from strings, so
//! `conn.rs` is the only place that touches a socket and this is the only
//! place that holds an opinion. Each refusal in `conn.rs` closes the socket
//! with NOTHING written: no status, no body, no reason, no capability hint —
//! the documentation server's "nothing says otherwise" discipline, applied
//! one notch harder because there is no page here that a human is waiting on.

use hmac::{KeyInit, Mac};
use worktrees_api::docserver;

/// The origins every planned client is one of: the desktop app's WebView on
/// macOS and Linux (`tauri://localhost`) and on Windows and Android
/// (`http://tauri.localhost`). wry 0.57's `with_https_scheme` doc names the
/// split: "the `<scheme>://localhost` protocols used on macOS and Linux".
/// A browser-hosted dev frontend adds its own with `--allow-origin`, which is a
/// FLAG and never a config key, so a dev entry cannot ship in a file.
pub const BUILTIN_ORIGINS: &[&str] = &["tauri://localhost", "http://tauri.localhost"];

/// How long the WHOLE handshake may take, from the upgrade to the server's
/// `ok` — not each frame: two quick steps that add up to more than this are
/// refused like a silent client is. A client that has opened a socket and
/// says nothing holds a task and a file descriptor; this is what ends it.
pub const AUTH_DEADLINE: std::time::Duration = std::time::Duration::from_secs(3);

/// Is this `Host` the loopback interface, on OUR port? Exactly the documentation
/// server's rule, reused rather than restated: on plaintext loopback this is
/// what stops DNS rebinding — a page on `evil.example` that re-resolves its own
/// name to `127.0.0.1` is same-origin with this server as far as the browser
/// is concerned, and the `Host` header is the one thing it cannot be scripted
/// into forging. Nothing else stands in the way.
pub fn host_ok(host: Option<&str>, port: u16) -> bool {
    docserver::host_ok(host, port)
}

/// Is this `Origin` one we serve?
///
/// **This deliberately diverges from `docserver::origin_ok`, which allows a
/// MISSING `Origin`.** The docs server's own page fetches same-origin, and a
/// same-origin GET sends no `Origin` at all, so `None` is its common case.
/// Nothing like that exists here: every planned client is a WebView or a
/// browser, and browsers always send `Origin` on a WebSocket handshake. So a
/// missing `Origin` is not "one of ours being quiet", it is a non-browser
/// client — curl, a script, a port scanner — and it is refused. Do not "align"
/// the two functions; they answer different questions about different pages.
///
/// Exact string match, case-sensitive: an origin is serialized by the browser
/// in one canonical form, and a comparison looser than that is a comparison
/// with cases nobody has enumerated. No CORS header is ever sent in reply to
/// anything — a refused origin learns nothing, an allowed one needs nothing.
pub fn origin_ok(origin: Option<&str>, allowed: &[String]) -> bool {
    let Some(o) = origin else { return false };
    BUILTIN_ORIGINS.contains(&o) || allowed.iter().any(|a| a == o)
}

// ── the remote rule set (Phase 2 §4.5) ───────────────────────────────────────

/// Is this `Host` the remote listener's own address, on its port — exactly
/// as `SocketAddr` prints it, with no zone (`192.168.1.5:7443`,
/// `[fe80::1]:7443`)? A device dials an address literal, never a name
/// (§2.2), so there is one right answer and no case or spelling to fold.
/// Over TLS this is not the DNS-rebinding guard loopback's `host_ok` is (no
/// page can reach a pinned mutual-TLS socket); it is the same "one spelling"
/// rule, so a request never names a host this listener is not.
pub fn remote_host_ok(host: Option<&str>, bound: std::net::SocketAddr) -> bool {
    host.is_some_and(|h| h == bound.to_string())
}

/// A remote upgrade carries NO `Origin` (decided by (main), 2026-10-22).
///
/// **This deliberately diverges from `origin_ok`, which REFUSES a missing
/// `Origin`** — different pages, different questions, as `origin_ok` says of
/// the docs server. On loopback every client is a page, a page always sends
/// `Origin`, and its absence means curl or a port scanner. On the remote
/// listener every client is a NATIVE socket (the shim's or the CLI's,
/// rustls, holding a device key a page could never hold, §4.4), and a native
/// socket sends no `Origin`; a browser could not even complete the TLS
/// handshake, which demands an enrolled client certificate. So an `Origin`
/// here is the one thing out of place: a browser-shaped request has no
/// business on this socket, and no browser origin is ever served, with or
/// without `--allow-origin`, which this listener never reads. No CORS header,
/// ever.
pub fn remote_origin_ok(origin: Option<&str>) -> bool {
    origin.is_none()
}

/// What an upgrade request must look like, read off the request by `conn.rs`.
/// Every field is a header value as received (`None` = absent); a header that
/// appeared twice or could not be read as a string never reaches here, because
/// `conn.rs` refuses those on sight.
pub struct Handshake<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub query: Option<&'a str>,
    pub upgrade: Option<&'a str>,
    pub connection: Option<&'a str>,
    pub version: Option<&'a str>,
    pub key: Option<&'a str>,
}

/// Is this a WebSocket upgrade for the one endpoint there is? RFC 6455 §4.2.1,
/// plus two choices of our own: the path is exactly `/` with no query (the
/// token never travels at all — see handshake v1 below — and refusing a query
/// outright means nothing a client might put there is ever parsed), and the
/// `Connection` and `Upgrade` tokens are matched as the comma-lists they are
/// (`Connection: keep-alive, Upgrade` is what some browsers send).
pub fn upgrade_ok(h: &Handshake) -> bool {
    if h.method != "GET" || h.path != "/" || h.query.is_some() {
        return false;
    }
    let has_token = |v: Option<&str>, want: &str| {
        v.is_some_and(|v| v.split(',').any(|t| t.trim().eq_ignore_ascii_case(want)))
    };
    has_token(h.upgrade, "websocket")
        && has_token(h.connection, "upgrade")
        && h.version.is_some_and(|v| v.trim() == "13")
        && h.key.is_some_and(|k| !k.trim().is_empty() && k.len() <= 64)
}

// ── handshake v1 ─────────────────────────────────────────────────────────────
//
// The spec is (main)'s `handshake-v1.md` (2026-10-18), and it fixes OWASP
// finding H1: under v0 the client's FIRST frame was the token, so whatever
// held the port after serve died received it. Now:
//
//   C→S {"hello": cn}      S→C {"challenge": sn}
//   C→S {"auth": HMAC(K, "worktrees-serve/v1/client\n" + cn + "\n" + sn)}
//   S→C {"ok": true, "version": …, "proof": HMAC(K, "worktrees-serve/v1/server\n" + cn + "\n" + sn)}
//
// K is serve.json's token as UTF-8; cn, sn and both proofs are 32 bytes as 64
// LOWERCASE hex. The token is never in a frame, either way. The server's only
// output to an unproven peer is a random challenge: it computes its own proof
// only after the client's has verified, so a stand-in for a client learns
// nothing it could take elsewhere. Different labels per direction stop either
// proof being reflected back as the other; `sn`, fresh per connection, stops
// a recorded proof being replayed. Every refusal is a closed socket with
// nothing written, as before (D6), and `AUTH_DEADLINE` covers all of it.

/// The client's direction, spelled exactly as the spec spells it.
pub const CLIENT_LABEL: &str = "worktrees-serve/v1/client\n";
/// The server's direction.
pub const SERVER_LABEL: &str = "worktrees-serve/v1/server\n";

/// `hex32`: 32 bytes as 64 lowercase hex characters — the spec's ONE spelling.
/// Upper case is refused rather than folded: a comparison looser than the
/// format is a comparison with cases nobody has enumerated.
pub fn is_hex32(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The value of a handshake frame that must be exactly `{<key>: <hex32>}`:
/// one key, that key, a hex32 string. A frame carrying anything more is a
/// client talking before it was let in.
fn hex32_field(frame: &str, key: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(frame).ok()?;
    let obj = v.as_object()?;
    if obj.len() != 1 {
        return None;
    }
    let s = obj.get(key)?.as_str()?;
    is_hex32(s).then(|| s.to_string())
}

/// Frame 1, `{"hello": cn}`: the client's nonce, or `None` (refuse).
pub fn hello(frame: &str) -> Option<String> {
    hex32_field(frame, "hello")
}

/// Frame 3, `{"auth": client_proof}`: the proof, or `None` (refuse).
pub fn client_proof(frame: &str) -> Option<String> {
    hex32_field(frame, "auth")
}

type HmacSha256 = hmac::Hmac<sha2::Sha256>;

/// `HMAC(K, label + cn + "\n" + sn)`, not yet finalized.
fn mac(token: &str, label: &str, cn: &str, sn: &str) -> HmacSha256 {
    let mut m = HmacSha256::new_from_slice(token.as_bytes()).expect("HMAC takes a key of any length");
    m.update(label.as_bytes());
    m.update(cn.as_bytes());
    m.update(b"\n");
    m.update(sn.as_bytes());
    m
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    if !is_hex32(s) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// Does `proof` show the client holds `token`, for THIS connection's nonces?
/// `verify_slice` is a constant-time compare.
pub fn client_proof_ok(token: &str, cn: &str, sn: &str, proof: &str) -> bool {
    let Some(tag) = unhex32(proof) else { return false };
    mac(token, CLIENT_LABEL, cn, sn).verify_slice(&tag).is_ok()
}

/// The client's proof, as a console client of this serve computes it
/// (`client.rs`, the CLI's `serve call|remote|pair`).
pub fn client_proof_for(token: &str, cn: &str, sn: &str) -> String {
    mac(token, CLIENT_LABEL, cn, sn).finalize().into_bytes().iter().map(|x| format!("{x:02x}")).collect()
}

/// The server's proof, for the `ok` reply. Call it only after
/// `client_proof_ok` — the server proves nothing to an unproven peer.
pub fn server_proof(token: &str, cn: &str, sn: &str) -> String {
    mac(token, SERVER_LABEL, cn, sn).finalize().into_bytes().iter().map(|x| format!("{x:02x}")).collect()
}

/// Is this a value `--allow-origin` may take? A serialized origin is
/// `scheme://host[:port]` — no path, no trailing slash, no userinfo, no
/// spaces. Refusing the malformed ones at the flag means a typo is an error at
/// launch rather than an allow-list entry that silently matches nothing.
pub fn allow_origin_flag_ok(s: &str) -> Result<(), String> {
    let bad = |why: &str| Err(format!("--allow-origin {s:?}: {why} (want scheme://host[:port])"));
    let Some((scheme, rest)) = s.split_once("://") else { return bad("no scheme") };
    if scheme.is_empty() || !scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) {
        return bad("bad scheme");
    }
    if rest.is_empty() {
        return bad("no host");
    }
    if rest.contains('/') || rest.contains('?') || rest.contains('#') {
        return bad("an origin has no path");
    }
    if rest.contains('@') || rest.chars().any(char::is_whitespace) {
        return bad("not an origin");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORT: u16 = 41_237;

    /// The one rule that stops DNS rebinding, inherited from the docs server:
    /// loopback by literal or by the one name that cannot be pointed elsewhere
    /// by a remote party, and only on our port.
    #[test]
    fn only_the_loopback_interface_on_our_port_may_be_the_host() {
        for good in [format!("127.0.0.1:{PORT}"), format!("localhost:{PORT}"), format!("[::1]:{PORT}"), "127.0.0.1".into(), "localhost".into()] {
            assert!(host_ok(Some(&good), PORT), "{good} must be served");
        }
        for bad in ["attacker.example", "127.0.0.1.attacker.example", &format!("127.0.0.1:{}", PORT + 1), "10.0.0.5", &format!("localhost.evil:{PORT}"), ""] {
            assert!(!host_ok(Some(bad), PORT), "{bad} must be refused");
        }
        assert!(!host_ok(None, PORT), "a missing Host is a refusal, not an absence");
    }

    /// The divergence from the docs server, pinned: `None` is REFUSED here.
    /// If this test ever has to change, read `origin_ok`'s doc first.
    #[test]
    fn a_missing_origin_is_refused_and_only_the_allow_list_is_served() {
        let dev = vec!["http://localhost:1420".to_string()];
        assert!(!origin_ok(None, &dev), "a WebSocket handshake with no Origin is not a browser's");
        assert!(!origin_ok(None, &[]));
        assert!(origin_ok(Some("tauri://localhost"), &[]));
        assert!(origin_ok(Some("http://tauri.localhost"), &[]));
        assert!(origin_ok(Some("http://localhost:1420"), &dev));
        for bad in [
            "http://localhost:1420/",     // a path is not an origin
            "http://localhost:1421",      // the port is part of it
            "https://localhost:1420",     // and so is the scheme
            "null",                       // every file:// page and sandboxed iframe
            "http://127.0.0.1:1420",      // a different spelling is a different origin
            "TAURI://LOCALHOST",          // exact match, by design
            "",
        ] {
            assert!(!origin_ok(Some(bad), &dev), "{bad:?} must be refused");
        }
        // The docs server's rule, for contrast — this is the line nobody may
        // "align" the two across.
        assert!(docserver::origin_ok(None, PORT), "the docs server allows a missing Origin; this server does not");
    }

    #[test]
    fn only_a_websocket_upgrade_of_the_root_is_an_upgrade() {
        let ok = Handshake {
            method: "GET", path: "/", query: None,
            upgrade: Some("websocket"), connection: Some("Upgrade"),
            version: Some("13"), key: Some("dGhlIHNhbXBsZSBub25jZQ=="),
        };
        assert!(upgrade_ok(&ok));
        // What browsers actually send, in their various spellings.
        assert!(upgrade_ok(&Handshake { connection: Some("keep-alive, Upgrade"), upgrade: Some("WebSocket"), ..ok }));
        assert!(!upgrade_ok(&Handshake { method: "POST", ..ok }));
        assert!(!upgrade_ok(&Handshake { path: "/ws", ..ok }));
        assert!(!upgrade_ok(&Handshake { query: Some("token=abc"), ..ok }), "nothing in a query is ever read");
        assert!(!upgrade_ok(&Handshake { upgrade: None, ..ok }), "a plain GET of / is not served either");
        assert!(!upgrade_ok(&Handshake { connection: Some("keep-alive"), ..ok }));
        assert!(!upgrade_ok(&Handshake { version: Some("8"), ..ok }));
        assert!(!upgrade_ok(&Handshake { key: Some(""), ..ok }));
        assert!(!upgrade_ok(&Handshake { key: None, ..ok }));
    }

    const N: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// Each handshake frame is exactly one key with a hex32 value, or it is
    /// nothing — and the hello is not an auth, nor the auth a hello.
    #[test]
    fn a_handshake_frame_is_exactly_one_hex32_key_or_it_is_nothing() {
        assert_eq!(hello(&format!("{{\"hello\":\"{N}\"}}")).as_deref(), Some(N));
        assert_eq!(hello(&format!("{{ \"hello\" : \"{N}\" }}")).as_deref(), Some(N), "whitespace is JSON's business");
        assert_eq!(client_proof(&format!("{{\"auth\":\"{N}\"}}")).as_deref(), Some(N));
        assert_eq!(hello(&format!("{{\"auth\":\"{N}\"}}")), None, "an auth is not a hello");
        assert_eq!(client_proof(&format!("{{\"hello\":\"{N}\"}}")), None, "a hello is not an auth");
        for bad in [
            format!("{{\"hello\":\"{}\"}}", &N[..63]),
            format!("{{\"hello\":\"{N}0\"}}"),
            format!("{{\"hello\":\"{}\"}}", N.to_uppercase()),
            format!("{{\"hello\":\"{}g\"}}", &N[..63]),
            format!("{{\"hello\":\"{N}\",\"cmd\":\"list_workspace\"}}"),
            format!("[\"{N}\"]"),
            format!("\"{N}\""),
            "{\"hello\":null}".to_string(),
            "{\"hello\":7}".to_string(),
            "{}".to_string(),
            String::new(),
            "not json".to_string(),
        ] {
            assert_eq!(hello(&bad), None, "{bad:?} is not a hello");
        }
    }

    /// The two directions are tagged apart: the server's proof is not the
    /// client's, so neither can be reflected as the other, and a proof is
    /// bound to both nonces and the token.
    #[test]
    fn a_proof_is_bound_to_its_direction_its_nonces_and_the_token() {
        let (t, cn, sn) = ("a".repeat(64), N.to_string(), "b".repeat(64));
        let server = server_proof(&t, &cn, &sn);
        assert!(is_hex32(&server), "{server}");
        let client = {
            let m = mac(&t, CLIENT_LABEL, &cn, &sn).finalize().into_bytes();
            m.iter().map(|x| format!("{x:02x}")).collect::<String>()
        };
        assert!(client_proof_ok(&t, &cn, &sn, &client));
        assert_ne!(client, server, "the directions are labelled apart");
        assert!(!client_proof_ok(&t, &cn, &sn, &server), "a reflected server proof is not a client proof");
        assert!(!client_proof_ok(&t, &sn, &cn, &client), "cn and sn are ordered");
        assert!(!client_proof_ok(&t, &cn, &"c".repeat(64), &client), "another challenge, another proof");
        assert!(!client_proof_ok(&"d".repeat(64), &cn, &sn, &client), "another token, another proof");
        assert!(!client_proof_ok(&t, &cn, &sn, &client.to_uppercase()), "one spelling");
        assert!(!client_proof_ok(&t, &cn, &sn, &client[..62]), "the whole tag");
    }

    #[test]
    fn the_allow_origin_flag_takes_origins_and_nothing_else() {
        for good in ["http://localhost:1420", "http://127.0.0.1:5173", "tauri://localhost", "https://dev.example"] {
            assert!(allow_origin_flag_ok(good).is_ok(), "{good}");
        }
        for bad in ["localhost:1420", "http://localhost:1420/", "http://", "http://a b", "http://u@h", "http://h?x", "://h"] {
            assert!(allow_origin_flag_ok(bad).is_err(), "{bad} should be refused at the flag");
        }
    }
}
