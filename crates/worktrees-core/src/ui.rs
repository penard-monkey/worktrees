//! Output + prompt surface for ops, so the same core logic serves the CLI
//! (prints with the bash glyphs/colors, prompts on stdin) and the app (captures
//! messages, auto-answers). Message formatting here is a parity target — it
//! mirrors bash `info`/`warn`/`error`/`header` and the `read -r -p` prompts.

use crate::diag::Severity;
use crate::render::{CYAN, GREEN, NC, RED, YELLOW};
use std::io::Write;

pub trait Ui {
    fn info(&mut self, msg: &str);
    fn warn(&mut self, msg: &str);
    fn error(&mut self, msg: &str);
    fn header(&mut self, msg: &str);
    /// A pre-formatted line (may already contain color/indent), like bash `echo`.
    fn plain(&mut self, msg: &str);
    /// `read -r -p "<prompt>"`: true only for exactly `y`/`Y`; EOF → false.
    fn confirm(&mut self, prompt: &str) -> bool;
}

/// The CLI's terminal UI — byte-parity with the bash helpers.
pub struct CliUi;

impl Ui for CliUi {
    fn info(&mut self, msg: &str) {
        println!("{GREEN}▸{NC} {msg}");
    }
    fn warn(&mut self, msg: &str) {
        println!("{YELLOW}▸{NC} {msg}");
    }
    fn error(&mut self, msg: &str) {
        eprintln!("{RED}✗{NC} {msg}");
    }
    fn header(&mut self, msg: &str) {
        println!("\n{CYAN}═══ {msg} ═══{NC}");
    }
    fn plain(&mut self, msg: &str) {
        println!("{msg}");
    }
    fn confirm(&mut self, prompt: &str) -> bool {
        // read -p writes the prompt to stderr, then reads a line from stdin.
        eprint!("{prompt}");
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) | Err(_) => false, // EOF / error → abort
            Ok(_) => {
                let a = line.trim_end_matches(['\n', '\r']);
                a == "y" || a == "Y"
            }
        }
    }
}

/// Captures op output for programmatic callers (the Tauri app), instead of
/// printing. Confirms are declined — callers run non-interactive ops (`-y`),
/// and any unexpected prompt safely aborts rather than proceeding. The collected
/// lines are plain text (ops pass plain messages; ANSI is a CliUi concern).
///
/// ⚠ `lines` used to be the ONLY record, which erased severity at capture: a
/// warning on a SUCCESSFUL op (`code == 0`) vanished entirely, because the app's
/// `run_op` only surfaces output on failure. A `doctor` whose warnings the app
/// drops does not solve a silent-failure bug (proposal §7), so every message is
/// now recorded with the severity it was emitted at. `lines` stays a plain
/// `Vec<String>` in the same order for every existing caller.
#[derive(Default)]
pub struct CaptureUi {
    pub lines: Vec<String>,
    /// `(severity, message)` for the SAME messages, same order as `lines`.
    pub events: Vec<(Severity, String)>,
    pub errored: bool,
}

impl CaptureUi {
    fn push(&mut self, sev: Severity, msg: &str) {
        self.lines.push(msg.to_string());
        self.events.push((sev, msg.to_string()));
    }
    /// Messages emitted at `Warn` or above — what a caller must show even when
    /// the op returned 0.
    pub fn warnings(&self) -> Vec<String> {
        self.events
            .iter()
            .filter(|(s, _)| *s >= Severity::Warn)
            .map(|(_, m)| m.clone())
            .collect()
    }
}

impl Ui for CaptureUi {
    fn info(&mut self, msg: &str) {
        self.push(Severity::Info, msg);
    }
    fn warn(&mut self, msg: &str) {
        self.push(Severity::Warn, msg);
    }
    fn error(&mut self, msg: &str) {
        self.errored = true;
        self.push(Severity::Error, msg);
    }
    fn header(&mut self, msg: &str) {
        self.push(Severity::Info, msg);
    }
    fn plain(&mut self, msg: &str) {
        self.push(Severity::Info, msg);
    }
    fn confirm(&mut self, _prompt: &str) -> bool {
        false
    }
}

/// Helpers for the color constants used when building pre-formatted `plain` lines.
pub mod fmt {
    use crate::render::{CYAN, NC, YELLOW};
    pub fn cyan(s: &str) -> String {
        format!("{CYAN}{s}{NC}")
    }
    pub fn yellow(s: &str) -> String {
        format!("{YELLOW}{s}{NC}")
    }
}
