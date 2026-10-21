//! The app's own metadata: its log and its changelog.
//!
//! `get_changelog` keeps `include_str!("../../../CHANGELOG.md")` working
//! unchanged — `crates/worktrees-api/src/` is the same three levels below the
//! repo root that `app/src-tauri/src/` was. `CARGO_PKG_VERSION` likewise
//! resolves to the workspace version, which AGENTS.md names as the ONE
//! version source, so both the app crate and this one answer with it.

/// Where the log lives, for Settings → Logs.
#[derive(serde::Serialize)]
pub struct LogInfo {
    pub dir: String,
    pub file: String,
}

pub fn log_info() -> LogInfo {
    let (dir, file) = crate::log::paths();
    LogInfo { dir, file }
}

/// Frontend errors land in the same file, tagged `ui:`.
///
/// The message is a CLIENT's string, so it is cleaned here, in the body,
/// rather than in a sink: whichever host runs it writes the same line (since
/// the app became serve's client, that is serve, into `serve.log`). See
/// `ui_line`.
pub fn log_event(level: &str, msg: String) {
    let lv = match level {
        "error" | "warn" | "info" => level,
        _ => "info",
    };
    crate::log::log(lv, &format!("ui: {}", ui_line(&msg)));
}

/// The most of a client's message one log line keeps, in bytes.
const UI_LINE_MAX: usize = 4000;

/// A client's message as ONE line of the log: every character that could end
/// a line or make one read as something else is written as an escape, then
/// the result is cut to `UI_LINE_MAX` on a character boundary.
///
/// OWASP L1: the message kept its newlines, and an authenticated client wrote
/// a whole `[1] serve: host-admin …` line with a forged timestamp into
/// serve.log. Escaped rather than dropped, so a stack trace stays readable and
/// an attempt stays visible. The set is the C0/C1 controls (CR, LF, ESC and
/// NUL among them; C1 holds NEL), Unicode's own line and paragraph
/// separators, and the bidi embeddings, overrides and isolates, which reorder
/// a line on screen without changing its bytes.
///
/// Cut AFTER the escaping, because an escape is longer than the character it
/// stands for, and on a boundary, because `String::truncate` panics off one:
/// 3999 ASCII bytes and an `é` took the handler down before this.
fn ui_line(msg: &str) -> String {
    let mut out = String::with_capacity(msg.len().min(UI_LINE_MAX));
    for c in msg.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\x{:02x}", c as u32)),
            '\u{2028}' | '\u{2029}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => {
                out.push_str(&format!("\\u{{{:04x}}}", c as u32))
            }
            c => out.push(c),
        }
        if out.len() > UI_LINE_MAX {
            break;
        }
    }
    out.truncate(out.floor_char_boundary(UI_LINE_MAX));
    out
}

/// The CHANGELOG ships inside the binary — the "What's new" sheet renders the
/// sections between the last-seen and current versions with zero network.
#[derive(serde::Serialize)]
pub struct ChangelogInfo {
    pub version: String,
    pub changelog: String,
}

pub fn get_changelog() -> ChangelogInfo {
    ChangelogInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        changelog: include_str!("../../../CHANGELOG.md").to_string(),
    }
}

pub fn log_tail(lines: Option<usize>) -> String {
    crate::log::tail(lines.unwrap_or(200).min(2000))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A client's message is ONE line of the log. The OWASP review sent
    /// `log_event` a newline and a whole forged `[1] serve: host-admin …` line
    /// with a 2026-01-01 timestamp behind it, and serve.log printed it as a
    /// line of its own.
    #[test]
    fn a_ui_message_is_one_line_with_no_control_characters() {
        let forged = "boom\n2026-01-01T00:00:00Z [info] [1] serve: host-admin forged\r\x1b[2K\0tail";
        let out = ui_line(forged);
        assert_eq!(out.lines().count(), 1, "{out:?}");
        assert!(!out.chars().any(|c| c.is_control()), "{out:?}");
        // Kept legible, not dropped: what the client sent is still readable.
        assert!(out.contains("serve: host-admin forged") && out.ends_with("tail"), "{out:?}");
    }

    /// Unicode's own line terminators end a line in a viewer as surely as LF
    /// does, and the bidi overrides make a line READ as something its bytes
    /// are not.
    #[test]
    fn unicode_line_separators_and_bidi_overrides_are_escaped_too() {
        let out = ui_line("a\u{2028}b\u{2029}c\u{85}d\u{202e}e\u{2066}f");
        assert!(!out.contains(['\u{2028}', '\u{2029}', '\u{85}', '\u{202e}', '\u{2066}']), "{out:?}");
        assert!(out.starts_with('a') && out.ends_with('f'), "{out:?}");
    }

    /// The cap is applied AFTER cleaning (an escape is longer than what it
    /// replaces) and never splits a character: `String::truncate` panics off
    /// a char boundary, so 3999 ASCII bytes and an `é` took the handler down.
    #[test]
    fn the_cap_holds_after_cleaning_and_lands_on_a_char_boundary() {
        let out = ui_line(&format!("{}é", "a".repeat(3999)));
        assert_eq!(out, "a".repeat(3999));
        let out = ui_line(&"\n".repeat(3000));
        assert!(out.len() <= UI_LINE_MAX, "{}", out.len());
        assert!(!out.contains('\n'));
    }
}
