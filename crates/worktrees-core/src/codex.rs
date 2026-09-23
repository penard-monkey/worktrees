//! Small, read-only Codex session check for deciding whether `resume --last`
//! has a conversation in this exact worktree. Codex owns these files.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

fn sessions_dir() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".codex"))
        .join("sessions")
}

/// Rollout files live under `sessions/<year>/<month>/<day>/`. Read only the
/// first JSONL record (`session_meta`) of each file, never transcript content.
pub fn session_present(cwd: &str) -> bool {
    fn walk(dir: &Path, depth: u8, cwd: &str) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else { return false };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && depth < 3 {
                if walk(&path, depth + 1, cwd) { return true; }
            } else if depth == 3 && path.extension().is_some_and(|x| x == "jsonl") {
                let Ok(file) = std::fs::File::open(path) else { continue };
                let mut first = String::new();
                if BufReader::new(file).read_line(&mut first).is_ok() {
                    let meta = serde_json::from_str::<serde_json::Value>(&first).ok();
                    if meta.as_ref().and_then(|v| v.pointer("/payload/cwd")).and_then(|v| v.as_str()) == Some(cwd) {
                        return true;
                    }
                }
            }
        }
        false
    }
    walk(&sessions_dir(), 0, cwd)
}
