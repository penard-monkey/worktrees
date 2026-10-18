//! The loopback rule set, as pure functions — which is where the tests live.
//!
//! Everything here answers "may this connection proceed?" from strings, so
//! `conn.rs` is the only place that touches a socket and this is the only
//! place that holds an opinion. Each refusal in `conn.rs` closes the socket
//! with NOTHING written: no status, no body, no reason, no capability hint —
//! the documentation server's "nothing says otherwise" discipline, applied
//! one notch harder because there is no page here that a human is waiting on.

use worktrees_api::docserver;

/// The origins every planned client is one of: the desktop app's WebView on
/// macOS (`tauri://localhost`) and on Windows/Linux (`http://tauri.localhost`).
/// A browser-hosted dev frontend adds its own with `--allow-origin`, which is a
/// FLAG and never a config key, so a dev entry cannot ship in a file.
pub const BUILTIN_ORIGINS: &[&str] = &["tauri://localhost", "http://tauri.localhost"];

/// How long a connection has to produce its auth frame once the upgrade is
/// done. A client that has opened a socket and says nothing holds a task and
/// a file descriptor; this is what ends it.
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
/// token travels in the first frame, never in the URL, and refusing a query
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

/// Is this first frame `{"auth": "<token>"}` and nothing else?
///
/// Exactly one key, exactly `auth`, exactly a string: a frame that carries
/// anything more is a client that has started talking before it was let in,
/// and that is refused with the wrong tokens. The comparison itself is the
/// docs server's constant-time one.
pub fn auth_ok(frame: &str, token: &str) -> bool {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(frame) else { return false };
    let Some(obj) = v.as_object() else { return false };
    if obj.len() != 1 {
        return false;
    }
    let Some(given) = obj.get("auth").and_then(|a| a.as_str()) else { return false };
    docserver::token_ok(given, token)
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

    #[test]
    fn the_first_frame_is_exactly_the_auth_object_or_it_is_nothing() {
        let t = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert!(auth_ok(&format!("{{\"auth\":\"{t}\"}}"), t));
        assert!(auth_ok(&format!("{{ \"auth\" : \"{t}\" }}"), t), "whitespace is JSON's business");
        for bad in [
            format!("{{\"auth\":\"{}\"}}", &t[..63]),
            format!("{{\"auth\":\"{}x\"}}", t),
            format!("{{\"auth\":\"{t}\",\"cmd\":\"list_workspace\"}}"),
            format!("{{\"token\":\"{t}\"}}"),
            format!("[\"{t}\"]"),
            format!("\"{t}\""),
            format!("{{\"id\":1,\"cmd\":\"list_workspace\",\"args\":{{}}}}"),
            "{\"auth\":null}".to_string(),
            "{}".to_string(),
            String::new(),
            "not json".to_string(),
        ] {
            assert!(!auth_ok(&bad, t), "{bad:?} must not authenticate");
        }
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
