//! What a `cmd_*` parser may be handed by something that is not a person at a
//! shell, and what keeps it data.
//!
//! **R3: a value a client sent is never read as a flag.** Every `cmd_*` parser
//! tells a flag from a positional by its leading `-`, so a branch spelled
//! `--force` IS `--force` unless something ends the options first. There are two
//! layers, and the second is not a nicety:
//!
//! 1. **Structure.** `--` ends a parser's options ([`split_options`]; every
//!    parser a client can reach honours it), and a caller holding a client value
//!    puts it after one. That only moves the value one hop: past our parser it
//!    lands in a git argv, and git permutes options exactly the same way —
//!    `git fetch origin --upload-pack=<cmd>` RUNS `<cmd>`. So every git call that
//!    carries such a value puts its own `--` in front of it too.
//! 2. **Validation** at the boundary that knows the value came from a client
//!    ([`safe_arg`], [`branch_arg`]). A branch, a base or a place never
//!    legitimately starts with `-`, and refusing one is what turns "it was read
//!    as data" into a sentence a UI can show.

/// Split a `cmd_*` argv at its first `--`: the options before it, and the
/// arguments after it, each of which is a positional however it is spelled. The
/// `--` is in neither half. With no `--` everything is in the first half and the
/// parser sorts it by the leading `-`, exactly as before.
pub fn split_options(args: &[String]) -> (&[String], &[String]) {
    match args.iter().position(|a| a == "--") {
        Some(i) => (&args[..i], &args[i + 1..]),
        None => (args, &[]),
    }
}

/// Refuse a client-supplied value a parser — ours or git's — would read as a
/// FLAG, or one no branch, revision or place name can hold. Trimmed on the way
/// through.
///
/// Before `--` existed this was the only defence, and it was found the hard
/// way at the MCP boundary: `base: "--ai=touch /tmp/x"` did not create a
/// worktree off a branch called that, it set the AI command, which
/// `ops::launch` interpolates into `sh -ic '<ai_cmd>; …'` — a tool advertised
/// as "create a worktree" was arbitrary code execution. The command layer the
/// app and `worktrees serve` call (`worktrees-api`) had the same shape and was
/// saved only by pushing its own `--ai` AFTER the client's values.
pub fn safe_arg(v: &str, what: &str) -> Result<String, String> {
    let t = v.trim();
    if t.is_empty() {
        return Err(format!("{what} is required"));
    }
    if t.starts_with('-') {
        return Err(format!(
            "{what} may not begin with '-' (it would be read as a command-line flag)"
        ));
    }
    // Belt and braces: a git ref cannot contain these, and neither can a slug.
    if t.contains(['\n', '\r', '\0']) || t.contains("..") {
        return Err(format!("{what} contains characters that are not allowed"));
    }
    Ok(t.to_string())
}

/// A client-named BRANCH: [`safe_arg`], then git's own rule for a branch name —
/// applied to the name core will USE as well as the one written, because `new`
/// and `switch` strip one `origin/` first: `origin/-x` passes every check above
/// and becomes `-x`, and `origin/HEAD` becomes `HEAD`.
pub fn branch_arg(v: &str) -> Result<String, String> {
    let t = safe_arg(v, "branch")?;
    valid_branch(&t)?;
    valid_branch(t.strip_prefix("origin/").unwrap_or(&t))?;
    Ok(t)
}

/// Whether `name` may be a branch: `git check-ref-format --branch`, applied to
/// the name AS WRITTEN. git's own command first expands `@{-1}` and friends
/// inside a repository; a client names a branch, it does not ask for "the one
/// before", so nothing is expanded here.
///
/// That is `check_refname_format("refs/heads/<name>")` plus the two rules
/// `strbuf_check_branch_ref` adds — no leading `-`, and not `HEAD`. A mirror of
/// git, so it is pinned to git: a test runs the real `git check-ref-format`
/// over a corpus and fails on any name the two disagree about.
pub fn valid_branch(name: &str) -> Result<(), String> {
    let bad = |why: &str| Err(format!("'{name}' is not a valid branch name: {why}"));
    if name.is_empty() {
        return bad("it is empty");
    }
    if name.starts_with('-') {
        return bad("it begins with '-'");
    }
    if name == "HEAD" {
        return bad("HEAD is not a branch");
    }
    if let Some(c) = name.chars().find(|&c| c.is_ascii_control() || " ~^:?*[\\".contains(c)) {
        return bad(&format!("{c:?} is not allowed"));
    }
    if name.contains("..") {
        return bad("'..' is not allowed");
    }
    if name.contains("@{") {
        return bad("'@{' is not allowed");
    }
    if name.ends_with('.') {
        return bad("it ends with '.'");
    }
    // Empty components cover a leading or trailing '/' and '//'.
    for part in name.split('/') {
        if part.is_empty() {
            return bad("it has an empty '/'-separated part");
        }
        if part.starts_with('.') {
            return bad("a '/'-separated part begins with '.'");
        }
        if part.ends_with(".lock") {
            return bad("a '/'-separated part ends with '.lock'");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_first_double_dash_ends_the_options_and_is_in_neither_half() {
        let a = v(&["--force", "--", "--all", "--", "x"]);
        let (opts, rest) = split_options(&a);
        assert_eq!(opts, &v(&["--force"])[..]);
        assert_eq!(rest, &v(&["--all", "--", "x"])[..], "a second `--` is a positional");
        let a = v(&["--force", "x"]);
        assert_eq!(split_options(&a), (&a[..], &[][..]));
    }

    #[test]
    fn a_client_value_that_could_become_a_flag_is_refused() {
        // THE finding this boundary exists for: core's arg parsers consume
        // anything flag-shaped AS a flag, and `--ai=<cmd>` reaches
        // `ops::launch`, which interpolates it into `sh -ic '<cmd>; …'`.
        for bad in ["--ai=touch /tmp/x", "-r", "--name=..", "--no-tmux"] {
            assert!(safe_arg(bad, "base").is_err(), "{bad:?} must be refused");
        }
        for bad in ["", "   ", "a\nb", "x..y"] {
            assert!(safe_arg(bad, "branch").is_err(), "{bad:?} must be refused");
        }
        assert_eq!(safe_arg(" feat-x ", "branch").unwrap(), "feat-x");
        assert_eq!(branch_arg(" feat/x ").unwrap(), "feat/x");
        assert!(branch_arg("feat x").unwrap_err().contains("not a valid branch name"));
        // The name core uses, after `strip_origin`.
        assert!(branch_arg("origin/-x").is_err());
        assert!(branch_arg("origin/HEAD").is_err());
        assert_eq!(branch_arg("origin/feat").unwrap(), "origin/feat");
        // A NUL cannot travel through argv, so the oracle below cannot ask git
        // about one; refused here on its own.
        assert!(valid_branch("a\u{0}b").is_err());
    }

    /// The mirror against the thing it mirrors. Run outside any repository so
    /// git expands nothing, and fail on the first name the two disagree about.
    #[test]
    fn valid_branch_agrees_with_git_check_ref_format() {
        let corpus = [
            "feat", "feat/foo", "release-1.2.3", "a.b", "a/-b", "a-", "é", "é/ü", "(main)", "a,b", "a;b",
            "a$b", "a'b", "a\"b", "a!b", "a#b", "a%b", "a&b", "a(b)", "a+b", "a=b", "a`b", "a|b", "a<b>",
            "a]b", "a{b}", "a@b", "@", "lock", "a.lockx", "a.lock.b", "refs/heads/x", "head", "HEAD/x",
            "x/HEAD", "-", "--", "-x", "--force", "HEAD", "a b", "a\tb", "a\u{1}b", "a\u{7f}b", "~", "a~1",
            "a^", "a:b", "a?", "a*", "a[b", "a\\b", "a..b", "..", ".", ".a", "a/.b", "a.", "a/", "/a",
            "a//b", "/", "a@{b", "@{-1}", "a.lock", "a/b.lock", "a.lock/b", ".lock", "a/b/c",
        ];
        let dir = std::env::temp_dir().join(format!("wt-argv-oracle-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in corpus {
            let git = std::process::Command::new("git")
                .args(["check-ref-format", "--branch", name])
                .current_dir(&dir)
                .env("GIT_CEILING_DIRECTORIES", &dir)
                .output()
                .expect("git");
            assert_eq!(valid_branch(name).is_ok(), git.status.success(), "{name:?}: {:?}", valid_branch(name));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
