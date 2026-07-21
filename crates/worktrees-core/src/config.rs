//! Config + naming resolution, faithful to the bash CLI.
//! Precedence: flag > env > user config (`~/.config/worktrees/config`,
//! respecting `$XDG_CONFIG_HOME`) > default. The config is parsed as data,
//! never executed.

use std::path::{Path, PathBuf};

/// tmux/name-safe prefix: lowercase, then every byte not in `[a-z0-9_-]` → `-`
/// (per-byte, NOT run-collapsing — matches `tr -c 'a-z0-9_-' '-'`).
pub fn sanitize_prefix(s: &str) -> String {
    s.chars()
        .map(|c| {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Last matching `key = value` line (inline ` #…` stripped, trailing ws trimmed),
/// or `None`. Mirrors the bash `cfg_get` sed pipeline.
pub fn cfg_get(cfg_path: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(cfg_path).ok()?;
    let mut found = None;
    for line in text.lines() {
        if let Some(v) = parse_kv_line(line.trim_start(), key) {
            found = Some(v);
        }
    }
    found
}

/// `key <ws>* = <ws>* value` (line already left-trimmed) → the value, with an
/// inline ` #…` comment and trailing whitespace removed.
fn parse_kv_line(line: &str, key: &str) -> Option<String> {
    let rest = line.strip_prefix(key)?.trim_start();
    let rest = rest.strip_prefix('=')?;
    let mut val = rest.trim_start().to_string();
    if let Some(idx) = find_inline_comment(&val) {
        val.truncate(idx);
    }
    Some(val.trim_end().to_string())
}

/// Index of a ` #` / `\t#` inline comment (whitespace immediately before `#`).
fn find_inline_comment(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    (1..b.len()).find(|&i| b[i] == b'#' && (b[i - 1] == b' ' || b[i - 1] == b'\t')).map(|i| i - 1)
}

pub fn config_path() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_default();
            format!("{home}/.config")
        });
    PathBuf::from(base).join("worktrees").join("config")
}

/// AI pane command: flag > `$WORKTREES_AI_CMD` > `$WORKTREES_CLAUDE_CMD` (deprecated)
/// > `ai_cmd` config > default `claude`. `none` → empty (plain shell). Pure form
/// for testing.
pub fn resolve_ai_cmd_from(
    flag: Option<&str>,
    env_ai: Option<&str>,
    env_claude: Option<&str>,
    cfg: Option<&str>,
) -> String {
    let v = flag
        .filter(|s| !s.is_empty())
        .or(env_ai.filter(|s| !s.is_empty()))
        .or(env_claude.filter(|s| !s.is_empty()))
        .or(cfg.filter(|s| !s.is_empty()))
        .unwrap_or("claude");
    if v == "none" {
        String::new()
    } else {
        v.to_string()
    }
}

/// Resume arg (`-r` appends it): `$WORKTREES_AI_RESUME_ARG` > `ai_resume_arg` > `-r`.
pub fn resolve_ai_resume_arg_from(env: Option<&str>, cfg: Option<&str>) -> String {
    env.filter(|s| !s.is_empty())
        .or(cfg.filter(|s| !s.is_empty()))
        .unwrap_or("-r")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn sanitize_matches_bash_tr() {
        assert_eq!(sanitize_prefix("My.Repo!"), "my-repo-");
        assert_eq!(sanitize_prefix("feat/Foo"), "feat-foo");
        assert_eq!(sanitize_prefix("a b"), "a-b");
        assert_eq!(sanitize_prefix("ok-name_1"), "ok-name_1");
    }

    #[test]
    fn cfg_last_match_wins_and_strips_inline_comment() {
        let dir = std::env::temp_dir().join(format!("wtcfgtest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config");
        let mut f = std::fs::File::create(&p).unwrap();
        writeln!(f, "# comment").unwrap();
        writeln!(f, "ai_cmd = first").unwrap();
        writeln!(f, "ai_cmd = codex   # inline note").unwrap();
        writeln!(f, "prefix=teamx").unwrap();
        drop(f);
        assert_eq!(cfg_get(&p, "ai_cmd").as_deref(), Some("codex"));
        assert_eq!(cfg_get(&p, "prefix").as_deref(), Some("teamx"));
        assert_eq!(cfg_get(&p, "missing"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ai_cmd_precedence() {
        assert_eq!(resolve_ai_cmd_from(Some("flagcmd"), Some("envai"), None, Some("cfg")), "flagcmd");
        assert_eq!(resolve_ai_cmd_from(None, Some("envai"), Some("claudeenv"), Some("cfg")), "envai");
        assert_eq!(resolve_ai_cmd_from(None, None, Some("claudeenv"), Some("cfg")), "claudeenv");
        assert_eq!(resolve_ai_cmd_from(None, None, None, Some("cfg")), "cfg");
        assert_eq!(resolve_ai_cmd_from(None, None, None, None), "claude");
        assert_eq!(resolve_ai_cmd_from(Some("none"), None, None, None), "");
    }

    #[test]
    fn resume_arg_precedence() {
        assert_eq!(resolve_ai_resume_arg_from(Some("resume"), Some("--cont")), "resume");
        assert_eq!(resolve_ai_resume_arg_from(None, Some("--cont")), "--cont");
        assert_eq!(resolve_ai_resume_arg_from(None, None), "-r");
    }
}
