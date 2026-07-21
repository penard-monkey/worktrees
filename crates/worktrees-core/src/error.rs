//! Error type that carries an exit code, so the CLI reproduces the bash CLI's
//! codes exactly (0 ok · 1 guard/usage · 3 target-not-found).

use std::fmt;

#[derive(Debug, Clone)]
pub struct WtError {
    pub msg: String,
    pub code: i32,
}

impl WtError {
    pub fn new(msg: impl Into<String>) -> Self {
        WtError { msg: msg.into(), code: 1 }
    }
    pub fn with_code(msg: impl Into<String>, code: i32) -> Self {
        WtError { msg: msg.into(), code }
    }
    /// Target (worktree/branch) not found — exit 3, so the UI can tell "gone"
    /// from "broke".
    pub fn not_found(msg: impl Into<String>) -> Self {
        WtError { msg: msg.into(), code: 3 }
    }
}

impl fmt::Display for WtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.msg)
    }
}

impl std::error::Error for WtError {}

impl From<std::io::Error> for WtError {
    fn from(e: std::io::Error) -> Self {
        WtError::new(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, WtError>;
