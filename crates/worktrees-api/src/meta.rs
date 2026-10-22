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
pub fn log_event(level: &str, msg: String) {
    let lv = match level {
        "error" | "warn" | "info" => level,
        _ => "info",
    };
    let mut m = msg;
    m.truncate(4000);
    crate::log::log(lv, &format!("ui: {m}"));
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
