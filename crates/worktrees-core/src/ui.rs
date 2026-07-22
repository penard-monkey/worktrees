//! Output + prompt surface for ops, so the same core logic serves the CLI
//! (prints with the bash glyphs/colors, prompts on stdin) and the app (captures
//! messages, auto-answers). Message formatting here is a parity target — it
//! mirrors bash `info`/`warn`/`error`/`header` and the `read -r -p` prompts.

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
#[derive(Default)]
pub struct CaptureUi {
    pub lines: Vec<String>,
    pub errored: bool,
}

impl Ui for CaptureUi {
    fn info(&mut self, msg: &str) {
        self.lines.push(msg.to_string());
    }
    fn warn(&mut self, msg: &str) {
        self.lines.push(msg.to_string());
    }
    fn error(&mut self, msg: &str) {
        self.errored = true;
        self.lines.push(msg.to_string());
    }
    fn header(&mut self, msg: &str) {
        self.lines.push(msg.to_string());
    }
    fn plain(&mut self, msg: &str) {
        self.lines.push(msg.to_string());
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
