//! A paired device's side of `worktrees serve`'s remote listener (Server
//! Edition Phase 2, `docs/proposals/server-edition-phase2.md` §2.2–§2.4,
//! §4.2–§4.4): pairing by link, the device key, the hosts it keeps, and the
//! pinned mutual-TLS WebSocket as a PIPE of text frames.
//!
//! The pipe parses nothing. The protocol over it is the loopback one,
//! handshake v1 included — keyed with the TLS session's exporter instead of
//! serve.json's token ([`tls::EXPORTER_SESSION`]) — so the page's
//! `app/src/ipcws.ts` stays its one client; [`call`] is the small Rust copy
//! `worktrees remote call` and the CI witness use.
//!
//! No tauri, no tmux, no engine: the desktop shim, the CLI and the iOS app (a
//! separate repository) all depend on this crate.

pub mod call;

/// Text a HOST chose — a close reason, an error, an event's name — made safe
/// to print: control characters and the invisible or direction-changing ones
/// (`pairuri::display_name_ok`'s set) become `\u{…}` escapes, so a host cannot
/// write terminal escape sequences into the device's screen, and it is cut at
/// 300 characters.
pub fn printable(s: &str) -> String {
    let odd = |c: char| {
        c.is_control()
            || matches!(c, '\u{00AD}' | '\u{061C}' | '\u{180E}' | '\u{FEFF}')
            || ('\u{200B}'..='\u{200F}').contains(&c)
            || ('\u{2028}'..='\u{202E}').contains(&c)
            || ('\u{2060}'..='\u{2069}').contains(&c)
    };
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i == 300 {
            out.push('…');
            break;
        }
        if odd(c) {
            out.push_str(&c.escape_unicode().to_string());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn host_text_cannot_drive_the_terminal() {
        let s = super::printable("revoked\u{1b}]0;pwned\u{7}\u{1b}[2J\nnext\u{202E}line");
        assert!(!s.chars().any(|c| c.is_control() || c == '\u{202E}'), "{s:?}");
        assert!(s.starts_with("revoked\\u{1b}]0;pwned"), "{s}");
        assert_eq!(super::printable(&"x".repeat(400)).chars().count(), 301);
    }
}
pub mod dial;
pub mod hosts;
pub mod key;
pub mod pair;
pub mod pipe;
pub mod tls;
