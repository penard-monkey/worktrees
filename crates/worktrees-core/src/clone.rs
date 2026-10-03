//! Clone a repository from a URL into `<parent>/<name>` — the one
//! implementation behind `worktrees clone` and the app's "Clone from URL…".
//!
//! What this module owns, and why each is here rather than in a caller:
//!
//! - **What a URL is** (`parse_source`): any git URL is accepted; only the
//!   `owner/repo` shorthand is GitHub-specific. The frontend mirrors the name
//!   derivation for its live preview, and `app/scripts/clone-check.mjs` runs that
//!   mirror over `NAME_CASES` below so the two cannot drift.
//! - **Where it lands** (`plan_target`): an existing target is refused, never
//!   written into and never suffixed — a silent `repo-2` would hide that you
//!   already have the repo.
//! - **That nothing waits on a prompt nobody can see** (`clone_env`): the app's
//!   git has no terminal, so a credential or host-key question would hang the
//!   dialog forever. The user's own ssh command/config, agent and keychain, and
//!   their git credential helpers, all still apply — only the INTERACTIVE
//!   fallbacks are switched off.
//! - **Cancel, and the cleanup that may remove only what this clone made**
//!   (`clone_repo`): the target is claimed with `create_dir` — which fails if it
//!   exists — so "the directory this clone created" is true by construction, and
//!   that is the only path ever removed.
//!
//! What it deliberately does NOT do (ADR 0001): run anything the repository
//! declares. `git clone` transfers no hooks; submodules are not recursed (each
//! is a URL the repo supplies, and another place a prompt could hide), and the
//! outcome says when there are some.

use serde::Serialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// A parsed, validated clone source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CloneSource {
    /// What `git clone` is handed (the shorthand already expanded).
    pub url: String,
    /// The folder name the URL implies — the default for `<parent>/<name>`.
    pub name: String,
}

/// Transports accepted with `scheme://`. Everything else — and every
/// `<helper>::<address>` remote-helper form, `ext::` above all — is refused:
/// a URL is pasted from somewhere else, and `ext::` is a command line.
const SCHEMES: &[&str] = &["https", "http", "ssh", "git", "file", "git+ssh", "ssh+git"];

/// `(input, Some(name))` for an accepted source, `(input, None)` for a refused
/// one. The single table both the Rust tests and `clone-check.mjs` read — keep
/// it one `("…", …),` row per line, double-quoted, no escapes.
pub const NAME_CASES: &[(&str, Option<&str>)] = &[
    ("https://github.com/owner/repo", Some("repo")),
    ("https://github.com/owner/repo.git", Some("repo")),
    ("https://github.com/owner/repo/", Some("repo")),
    ("https://github.com/owner/repo.git/", Some("repo")),
    ("https://gitlab.com/group/sub/proj.git", Some("proj")),
    ("https://github.com/owner/repo?tab=readme", Some("repo")),
    ("https://github.com/owner/repo#readme", Some("repo")),
    ("http://host.local/x/y.git", Some("y")),
    ("git@github.com:owner/repo.git", Some("repo")),
    ("git@github.com:repo.git", Some("repo")),
    ("host:path/to/thing", Some("thing")),
    ("ssh://git@host:2222/owner/repo.git", Some("repo")),
    ("ssh://host/~/repo", Some("repo")),
    ("git://host/repo.git", Some("repo")),
    ("file:///srv/git/repo.git", Some("repo")),
    ("file:///srv/git/repo/.git", Some("repo")),
    ("owner/repo", Some("repo")),
    ("owner/repo.git", Some("repo")),
    ("  owner/repo  ", Some("repo")),
    ("", None),
    ("-oProxyCommand=x", None),
    ("ext::sh -c touch% /tmp/pwned", None),
    ("fd::17", None),
    ("ftp://host/repo.git", None),
    ("https://github.com/", None),
    ("https://github.com/owner/.hidden", None),
    ("/Users/me/repo", None),
    ("./repo", None),
    ("repo", None),
    ("owner/repo/extra", None),
    ("https://github.com/owner/re po", None),
];

/// Parse what the user pasted. `Err` is the sentence the dialog shows.
pub fn parse_source(input: &str) -> Result<CloneSource, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("paste a repository URL".into());
    }
    if s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("a URL cannot contain spaces".into());
    }
    // A leading '-' would be read by git as an option. `--` is passed as well;
    // this is the message, that is the guarantee.
    if s.starts_with('-') {
        return Err("a URL cannot start with '-'".into());
    }
    let url = if let Some(i) = s.find("://") {
        let scheme = &s[..i];
        if !SCHEMES.iter().any(|k| k.eq_ignore_ascii_case(scheme)) {
            return Err(format!("'{scheme}://' URLs are not supported — use https://, ssh:// or git@host:path"));
        }
        s.to_string()
    } else if s.contains("::") {
        return Err("remote-helper URLs (`helper::address`) are not supported".into());
    } else if is_scp_like(s) {
        s.to_string()
    } else if is_shorthand(s) {
        let repo = s.strip_suffix(".git").unwrap_or(s);
        format!("https://github.com/{repo}.git")
    } else if s.starts_with('/') || s.starts_with('.') || s.starts_with('~') {
        return Err("that is a folder on this Mac — use “Add existing…” for it".into());
    } else {
        return Err("not a git URL — paste https://…, git@host:owner/repo.git, or owner/repo".into());
    };
    let name = derive_name(&url).ok_or("cannot tell the repository's name from that URL")?;
    valid_dir_name(&name)?;
    Ok(CloneSource { url, name })
}

/// git's own rule for the scp form: a ':' that comes before any '/', and a
/// non-empty host in front of it.
fn is_scp_like(s: &str) -> bool {
    match s.find(':') {
        Some(c) if c > 0 => !s[..c].contains('/') && c + 1 < s.len(),
        _ => false,
    }
}

fn is_shorthand(s: &str) -> bool {
    let mut parts = s.split('/');
    let (Some(o), Some(r), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let ok = |p: &str| {
        !p.is_empty() && !p.starts_with('.') && p.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    };
    ok(o) && ok(r)
}

/// The last path segment, minus `.git` — and the segment before it when the
/// last one IS `.git` (a non-bare repo's path), which is what `git clone`
/// itself would name the folder.
fn derive_name(url: &str) -> Option<String> {
    let mut path = url;
    if let Some(i) = path.find("://") {
        path = &path[i + 3..];
        // drop the authority: everything up to the first '/'
        path = path.find('/').map_or("", |j| &path[j..]);
    } else if let Some(c) = path.find(':') {
        path = &path[c + 1..];
    }
    let path = path.split(['?', '#']).next().unwrap_or("");
    let mut segs: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if segs.last() == Some(&".git") {
        segs.pop();
    }
    let last = *segs.last()?;
    let name = last.strip_suffix(".git").unwrap_or(last);
    (!name.is_empty()).then(|| name.to_string())
}

/// A folder name that becomes ONE path component under the parent. Same rules
/// as the app's "New project" name, for the same reason: it may not steer the
/// path, and nobody pasting a URL means "make it hidden".
pub fn valid_dir_name(n: &str) -> Result<(), String> {
    if n.is_empty() {
        return Err("the folder needs a name".into());
    }
    if n == "." || n == ".." {
        return Err("'.' and '..' are not names".into());
    }
    if n.starts_with('.') {
        return Err("a name starting with '.' would make a hidden folder".into());
    }
    if n.contains('/') {
        return Err("a folder name cannot contain '/'".into());
    }
    if n.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("a folder name cannot contain spaces or control characters".into());
    }
    Ok(())
}

/// A LEADING `~` (alone or before `/`) → `$HOME`; `~user` stays literal.
pub fn expand_home(p: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    if home.is_empty() {
        return PathBuf::from(p);
    }
    if p == "~" {
        return PathBuf::from(home);
    }
    match p.strip_prefix("~/") {
        Some(rest) => PathBuf::from(home).join(rest),
        None => PathBuf::from(p),
    }
}

/// `<parent>/<name>`, or the reason it cannot be the target. Checked again at
/// claim time by `create_dir` — this is the readable answer, that is the race-free one.
pub fn plan_target(parent: &str, name: &str) -> Result<PathBuf, CloneError> {
    let invalid = |m: String| CloneError { kind: CloneErrorKind::Invalid, message: m };
    let name = name.trim();
    valid_dir_name(name).map_err(invalid)?;
    let parent = parent.trim();
    if parent.is_empty() {
        return Err(invalid("choose the folder to clone into".into()));
    }
    let base = expand_home(parent);
    // Relative would resolve against the PROCESS's cwd — `/` for a GUI app.
    if !base.is_absolute() {
        return Err(invalid(format!("the folder must be an absolute path (or start with ~) — got '{parent}'")));
    }
    let target = base.join(name);
    if target.symlink_metadata().is_ok() {
        return Err(exists_error(&target));
    }
    Ok(target)
}

fn exists_error(target: &Path) -> CloneError {
    CloneError {
        kind: CloneErrorKind::Exists,
        message: if target.join(".git").exists() {
            format!("{} already exists and is a git repo — add it with “Add existing…” instead.", target.display())
        } else {
            format!("{} already exists — choose another folder or name.", target.display())
        },
    }
}

/// Why a clone failed, as a category the UI can word and a test can assert.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CloneErrorKind {
    /// The source or destination was refused before git ran.
    Invalid,
    /// The target already exists.
    Exists,
    /// git asked for credentials it may not prompt for, or they were refused.
    Auth,
    /// The remote answered that there is no such repository.
    NotFound,
    /// ssh does not know the host yet (`BatchMode` will not ask).
    HostKey,
    /// The host could not be reached.
    Network,
    /// The user cancelled.
    Cancelled,
    /// Anything else — the message carries git's own words.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CloneError {
    pub kind: CloneErrorKind,
    pub message: String,
}

impl std::fmt::Display for CloneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// git's stderr → a kind. Order matters: GitHub answers a missing OR private
/// https repo with a credential request, which with prompts off is "could not
/// read Username" — that is an auth failure, and the message says it may also
/// mean "does not exist", because GitHub will not say which.
pub fn classify(stderr: &str) -> CloneErrorKind {
    let s = stderr.to_ascii_lowercase();
    let any = |ps: &[&str]| ps.iter().any(|p| s.contains(p));
    if any(&["host key verification failed", "no matching host key", "remote host identification has changed"]) {
        CloneErrorKind::HostKey
    } else if any(&[
        "authentication failed",
        "could not read username",
        "could not read password",
        "terminal prompts disabled",
        "permission denied (publickey",
        "permission denied, please try again",
        "invalid username or password",
        "returned error: 401",
        "returned error: 403",
    ]) {
        CloneErrorKind::Auth
    } else if any(&[
        "repository not found",
        "does not appear to be a git repository",
        "returned error: 404",
        "not found",
        "does not exist",
    ]) {
        CloneErrorKind::NotFound
    } else if any(&[
        "could not resolve host",
        "could not resolve hostname",
        "connection refused",
        "connection timed out",
        "operation timed out",
        "network is unreachable",
        "no route to host",
        "failed to connect",
        "connection reset",
        "could not connect",
        "unable to access",
        "ssl",
    ]) {
        CloneErrorKind::Network
    } else {
        CloneErrorKind::Other
    }
}

/// The headline for a kind, then git's own last `fatal:`/`error:` line — the
/// headline is what to DO, git's line is the evidence.
fn failure_message(kind: CloneErrorKind, url: &str, stderr: &str) -> String {
    let head = match kind {
        CloneErrorKind::Auth => format!(
            "Could not authenticate to {url}. The repository is private (or does not exist), and git has no credentials for it that work without asking — set up an ssh key or a credential helper, then try again."
        ),
        CloneErrorKind::NotFound => format!("No repository at {url} — check the URL."),
        CloneErrorKind::HostKey => format!(
            "This Mac has not trusted that host's ssh key yet. Connect once from a terminal (for example `ssh -T git@{}`) and accept it, then try again.",
            host_of(url).unwrap_or("<host>")
        ),
        CloneErrorKind::Network => format!("Could not reach the host for {url} — check the network and the address."),
        CloneErrorKind::Cancelled => "Clone cancelled.".into(),
        _ => format!("git clone {url} failed."),
    };
    let evidence = stderr
        .lines()
        .map(str::trim)
        .rfind(|l| l.starts_with("fatal:") || l.starts_with("error:") || l.starts_with("ERROR:"))
        .or_else(|| stderr.lines().map(str::trim).rfind(|l| !l.is_empty()));
    match evidence {
        Some(e) => format!("{head}\n{e}"),
        None => head,
    }
}

fn host_of(url: &str) -> Option<&str> {
    let rest = match url.find("://") {
        Some(i) => &url[i + 3..],
        None => url.split(':').next()?,
    };
    let auth = rest.split('/').next()?;
    let host = auth.rsplit('@').next()?;
    Some(host.split(':').next().unwrap_or(host)).filter(|h| !h.is_empty())
}

/// The ssh command git runs, with the two options that keep it from waiting on
/// a human. `base` is the USER's own choice (`$GIT_SSH_COMMAND`, else
/// `core.sshCommand`, else `$GIT_SSH`, else `ssh`), so their ssh config, agent
/// and keychain still apply — this appends, it never replaces.
pub fn ssh_command(base: &str) -> String {
    format!("{base} -o BatchMode=yes -o ConnectTimeout=20")
}

/// The environment a clone runs under, as pairs to set. `GIT_TERMINAL_PROMPT=0`
/// stops git's own username/password prompt; credential HELPERS (the macOS
/// keychain) are not prompts and still run.
pub fn clone_env(ssh_base: &str) -> Vec<(&'static str, String)> {
    vec![("GIT_TERMINAL_PROMPT", "0".into()), ("GIT_SSH_COMMAND", ssh_command(ssh_base))]
}

/// The user's ssh command, resolved the way git would before we append to it.
pub fn user_ssh_base() -> String {
    if let Ok(c) = std::env::var("GIT_SSH_COMMAND") {
        if !c.trim().is_empty() {
            return c;
        }
    }
    // Asked with no repository in play, so a parent folder that happens to sit
    // inside another repo cannot lend its config to this clone.
    let cfg = Command::new("git")
        .args(["config", "--get", "core.sshCommand"])
        .current_dir(std::env::temp_dir())
        .env("GIT_CEILING_DIRECTORIES", std::env::temp_dir())
        .stdin(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());
    if let Some(c) = cfg {
        return c;
    }
    match std::env::var("GIT_SSH") {
        Ok(p) if !p.trim().is_empty() => format!("'{}'", p.replace('\'', "'\\''")),
        _ => "ssh".into(),
    }
}

/// One progress sample: git's phase ("Receiving objects"), its percentage,
/// and the rest of its line ("1.20 MiB | 2.00 MiB/s").
#[derive(Debug, Default, Clone, PartialEq, Serialize)]
pub struct CloneProgress {
    pub phase: Option<String>,
    pub percent: Option<u8>,
    pub detail: Option<String>,
}

/// Parse one `\r`/`\n`-delimited stderr segment. `None` for a line that is not
/// a progress line ("Cloning into 'x'...", warnings).
pub fn parse_progress(line: &str) -> Option<CloneProgress> {
    let line = line.trim();
    let line = line.strip_prefix("remote:").map_or(line, str::trim);
    let (phase, rest) = line.split_once(':')?;
    let rest = rest.trim();
    let pct_end = rest.find('%')?;
    let percent: u8 = rest[..pct_end].trim().parse().ok()?;
    if percent > 100 || phase.is_empty() || phase.len() > 40 {
        return None;
    }
    // "45% (450/1000), 1.20 MiB | 2.00 MiB/s" → "1.20 MiB | 2.00 MiB/s";
    // "100% (3/3), done." → None.
    let detail = rest[pct_end + 1..]
        .split_once("),")
        .map(|(_, d)| d.trim().trim_end_matches('.').trim())
        .filter(|d| !d.is_empty() && *d != "done")
        .map(str::to_string);
    Some(CloneProgress { phase: Some(phase.trim().to_string()), percent: Some(percent), detail })
}

/// What a finished clone reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CloneOutcome {
    pub dir: String,
    /// `.gitmodules` exists — submodules were NOT fetched (see module doc).
    pub has_submodules: bool,
}

/// How to run git — overridable so tests can stand in a hanging ssh.
#[derive(Default)]
pub struct CloneOpts {
    /// `None` resolves the user's (`user_ssh_base`).
    pub ssh_base: Option<String>,
}

const PROGRESS_MIN_GAP: Duration = Duration::from_millis(66);
/// stderr kept for classification — enough for every fatal line, bounded so a
/// chatty remote cannot grow it without limit.
const STDERR_KEEP: usize = 64 * 1024;

/// Clone `src` into `target` (from `plan_target`). Creates missing parents,
/// claims `target` with `create_dir`, runs `git clone --progress -- <url> <target>`,
/// streams progress, and on ANY failure — cancel included — removes `target`,
/// which this call created and nothing else did.
pub fn clone_repo(
    src: &CloneSource,
    target: &Path,
    opts: &CloneOpts,
    cancel: &AtomicBool,
    mut progress: Option<&mut dyn FnMut(CloneProgress)>,
) -> Result<CloneOutcome, CloneError> {
    let other = |m: String| CloneError { kind: CloneErrorKind::Other, message: m };
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| other(format!("cannot create {}: {e}", parent.display())))?;
    }
    // The claim. From here on `target` is ours, and only ours to remove.
    match std::fs::create_dir(target) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Err(exists_error(target)),
        Err(e) => return Err(other(format!("cannot create {}: {e}", target.display()))),
    }
    let result = run_clone(src, target, opts, cancel, &mut progress);
    if result.is_err() {
        // Best effort, and said when it fails: a half-written clone left behind
        // is the next attempt's "already exists".
        if let Err(e) = std::fs::remove_dir_all(target) {
            if target.exists() {
                return result.map_err(|mut err| {
                    err.message.push_str(&format!("\n(could not remove the partial clone at {}: {e})", target.display()));
                    err
                });
            }
        }
    }
    result
}

fn run_clone(
    src: &CloneSource,
    target: &Path,
    opts: &CloneOpts,
    cancel: &AtomicBool,
    progress: &mut Option<&mut dyn FnMut(CloneProgress)>,
) -> Result<CloneOutcome, CloneError> {
    use std::os::unix::process::CommandExt;
    let base = opts.ssh_base.clone().unwrap_or_else(user_ssh_base);
    let mut cmd = Command::new("git");
    cmd.args(["clone", "--progress", "--", &src.url])
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        // Its own process group, so a cancel reaches git-remote-https, ssh and
        // index-pack too — killing git alone leaves them holding the pipe.
        .process_group(0);
    for (k, v) in clone_env(&base) {
        cmd.env(k, v);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| CloneError { kind: CloneErrorKind::Other, message: format!("git failed to start: {e}") })?;
    let pgid = child.id() as i32;
    let mut errpipe = child.stderr.take().expect("stderr piped");
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let reader = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match errpipe.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let mut stderr: Vec<u8> = Vec::new();
    let mut partial = String::new();
    let mut last_sent: Option<Instant> = None;
    let mut pending: Option<CloneProgress> = None;
    let mut cancelled = false;
    let mut kill_deadline: Option<Instant> = None;
    loop {
        if !cancelled && cancel.load(Ordering::SeqCst) {
            cancelled = true;
            // SAFETY: plain syscall on a pgid we created; a stale one is ESRCH.
            unsafe {
                libc::kill(-pgid, libc::SIGKILL);
            }
            kill_deadline = Some(Instant::now() + Duration::from_secs(5));
        }
        if kill_deadline.is_some_and(|d| Instant::now() > d) {
            // Something outside the group still holds the pipe; stop waiting.
            break;
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(chunk) => {
                if stderr.len() < STDERR_KEEP {
                    stderr.extend_from_slice(&chunk);
                }
                partial.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(i) = partial.find(['\r', '\n']) {
                    let seg: String = partial.drain(..=i).collect();
                    if let Some(p) = parse_progress(&seg) {
                        pending = Some(p);
                    }
                }
                if let (Some(cb), Some(p)) = (progress.as_mut(), pending.as_ref()) {
                    if last_sent.is_none_or(|t| t.elapsed() >= PROGRESS_MIN_GAP) {
                        cb(p.clone());
                        pending = None;
                        last_sent = Some(Instant::now());
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    if let (Some(cb), Some(p)) = (progress.as_mut(), pending.take()) {
        cb(p);
    }
    let status = child.wait().map_err(|e| CloneError { kind: CloneErrorKind::Other, message: e.to_string() })?;
    if !cancelled {
        let _ = reader.join();
    }
    let stderr = String::from_utf8_lossy(&stderr);
    if cancelled {
        return Err(CloneError { kind: CloneErrorKind::Cancelled, message: "Clone cancelled.".into() });
    }
    if !status.success() {
        let kind = classify(&stderr);
        return Err(CloneError { kind, message: failure_message(kind, &src.url, &stderr) });
    }
    Ok(CloneOutcome {
        dir: target.to_string_lossy().into_owned(),
        has_submodules: target.join(".gitmodules").exists(),
    })
}

/// Ctrl-C during `worktrees clone`. git runs in its OWN process group (so a
/// cancel can reach its helpers), which also means the terminal's SIGINT no
/// longer reaches it — this flag is how the keypress becomes a cancel, and why
/// an interrupted CLI clone cleans up exactly like the app's Cancel button.
static CLI_CANCEL: AtomicBool = AtomicBool::new(false);

extern "C" fn on_sigint(_: libc::c_int) {
    CLI_CANCEL.store(true, Ordering::SeqCst);
}

/// `worktrees clone <url> [--into <dir>] [--name <folder>]` — clone into
/// `<dir>/<folder>` (default: the current directory and the URL's name) and
/// register the result as a project, as the app's "Clone from URL…" does.
/// Runs ahead of the git guard: there is no repository to stand in yet.
pub fn cmd_clone(ui: &mut dyn crate::ui::Ui, args: &[String]) -> i32 {
    const USAGE: &str = "usage: worktrees clone <url> [--into <dir>] [--name <folder>]";
    let mut url: Option<&str> = None;
    let mut into: Option<String> = None;
    let mut name: Option<String> = None;
    let mut it = args.iter();
    let mut opts_done = false;
    while let Some(a) = it.next() {
        if opts_done {
            if url.is_some() {
                ui.error(USAGE);
                return 2;
            }
            url = Some(a);
            continue;
        }
        match a.as_str() {
            "--" => opts_done = true,
            "-h" | "--help" => {
                ui.plain(USAGE);
                return 0;
            }
            "--into" => match it.next() {
                Some(v) => into = Some(v.clone()),
                None => {
                    ui.error(USAGE);
                    return 2;
                }
            },
            "--name" => match it.next() {
                Some(v) => name = Some(v.clone()),
                None => {
                    ui.error(USAGE);
                    return 2;
                }
            },
            s if s.starts_with("--") => {
                ui.error(&format!("unknown option {s}\n{USAGE}"));
                return 2;
            }
            s if url.is_none() => url = Some(s),
            _ => {
                ui.error(USAGE);
                return 2;
            }
        }
    }
    let Some(url) = url else {
        ui.error(USAGE);
        return 2;
    };
    let src = match parse_source(url) {
        Ok(s) => s,
        Err(e) => {
            ui.error(&e);
            return 1;
        }
    };
    let parent = match into {
        Some(d) => d,
        None => match std::env::current_dir() {
            Ok(d) => d.to_string_lossy().into_owned(),
            Err(e) => {
                ui.error(&e.to_string());
                return 1;
            }
        },
    };
    let folder = name.unwrap_or_else(|| src.name.clone());
    let target = match plan_target(&parent, &folder) {
        Ok(t) => t,
        Err(e) => {
            ui.error(&e.message);
            return 1;
        }
    };
    ui.info(&format!("cloning {} → {}", src.url, target.display()));
    // SAFETY: installs a handler that only stores to an atomic.
    unsafe {
        libc::signal(libc::SIGINT, on_sigint as *const () as libc::sighandler_t);
    }
    // SAFETY: isatty on a constant fd.
    let tty = unsafe { libc::isatty(2) } == 1;
    let mut drew = false;
    let mut sink = |p: CloneProgress| {
        if !tty {
            return;
        }
        let line = match (&p.phase, p.percent) {
            (Some(ph), Some(pc)) => match &p.detail {
                Some(d) => format!("{ph}: {pc}% · {d}"),
                None => format!("{ph}: {pc}%"),
            },
            _ => return,
        };
        eprint!("\r\x1b[2K{line}");
        drew = true;
    };
    let res = clone_repo(&src, &target, &CloneOpts::default(), &CLI_CANCEL, Some(&mut sink));
    // SAFETY: restores the default disposition.
    unsafe {
        libc::signal(libc::SIGINT, libc::SIG_DFL);
    }
    if drew {
        eprintln!();
    }
    let out = match res {
        Ok(o) => o,
        Err(e) => {
            ui.error(&e.message);
            return if e.kind == CloneErrorKind::Cancelled { 130 } else { 1 };
        }
    };
    let root = match crate::Project::discover(Path::new(&out.dir)) {
        Ok(p) => p.main_root,
        Err(e) => {
            ui.error(&format!("cloned to {}, but it is not readable as a project: {}", out.dir, e.msg));
            return 1;
        }
    };
    match crate::registry::add(&root) {
        Ok(e) => ui.info(&format!("registered '{}': {}", e.name, e.root)),
        Err(e) => {
            ui.error(&format!("cloned to {}, but registering it failed: {e}", out.dir));
            return 1;
        }
    }
    if out.has_submodules {
        ui.warn(&format!(
            "this repository has submodules, which were not fetched — run `git -C {} submodule update --init --recursive` if you need them",
            out.dir
        ));
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wtclone-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }

    fn git(cwd: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "init.defaultBranch=trunk"])
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap();
        assert!(ok.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&ok.stderr));
    }

    /// A bare repo with one commit on `trunk` — a real remote, no network.
    fn remote(root: &Path) -> PathBuf {
        let work = root.join("work");
        std::fs::create_dir_all(&work).unwrap();
        git(&work, &["init", "-q"]);
        std::fs::write(work.join("README.md"), "hi\n").unwrap();
        git(&work, &["add", "."]);
        git(&work, &["commit", "-q", "-m", "first"]);
        let bare = root.join("remote.git");
        git(root, &["clone", "-q", "--bare", work.to_str().unwrap(), bare.to_str().unwrap()]);
        bare
    }

    #[test]
    fn name_cases_parse_as_the_table_says() {
        for (input, want) in NAME_CASES {
            let got = parse_source(input).ok().map(|s| s.name);
            assert_eq!(got.as_deref(), *want, "input {input:?}");
        }
    }

    #[test]
    fn shorthand_expands_to_github_https_and_urls_pass_through() {
        assert_eq!(parse_source("owner/repo").unwrap().url, "https://github.com/owner/repo.git");
        assert_eq!(parse_source("owner/repo.git").unwrap().url, "https://github.com/owner/repo.git");
        let u = "git@gitlab.com:g/p.git";
        assert_eq!(parse_source(u).unwrap().url, u);
    }

    #[test]
    fn target_refuses_existing_relative_and_bad_names() {
        let d = tmp("plan");
        std::fs::create_dir_all(d.join("taken/.git")).unwrap();
        std::fs::create_dir_all(d.join("plain")).unwrap();
        let p = d.to_str().unwrap();
        assert_eq!(plan_target(p, "fresh").unwrap(), d.join("fresh"));
        let e = plan_target(p, "taken").unwrap_err();
        assert_eq!(e.kind, CloneErrorKind::Exists);
        assert!(e.message.contains("Add existing"), "{}", e.message);
        assert_eq!(plan_target(p, "plain").unwrap_err().kind, CloneErrorKind::Exists);
        assert_eq!(plan_target("rel/dir", "x").unwrap_err().kind, CloneErrorKind::Invalid);
        assert_eq!(plan_target(p, ".hidden").unwrap_err().kind, CloneErrorKind::Invalid);
        assert_eq!(plan_target(p, "a/b").unwrap_err().kind, CloneErrorKind::Invalid);
        assert_eq!(plan_target("", "x").unwrap_err().kind, CloneErrorKind::Invalid);
    }

    #[test]
    fn classify_maps_gits_words_to_kinds() {
        let cases = [
            ("fatal: could not read Username for 'https://github.com': terminal prompts disabled", CloneErrorKind::Auth),
            ("git@github.com: Permission denied (publickey).\nfatal: Could not read from remote repository.", CloneErrorKind::Auth),
            ("remote: Invalid username or password.\nfatal: Authentication failed for 'https://x/'", CloneErrorKind::Auth),
            ("ERROR: Repository not found.\nfatal: Could not read from remote repository.", CloneErrorKind::NotFound),
            ("fatal: '/nope' does not appear to be a git repository", CloneErrorKind::NotFound),
            ("fatal: repository 'https://gitlab.com/x/y.git/' not found", CloneErrorKind::NotFound),
            ("Host key verification failed.\nfatal: Could not read from remote repository.", CloneErrorKind::HostKey),
            ("ssh: Could not resolve hostname nohost: nodename nor servname provided", CloneErrorKind::Network),
            ("fatal: unable to access 'https://x/': Could not resolve host: x", CloneErrorKind::Network),
            ("ssh: connect to host h port 22: Connection refused", CloneErrorKind::Network),
            ("fatal: something odd", CloneErrorKind::Other),
        ];
        for (s, k) in cases {
            assert_eq!(classify(s), k, "{s}");
        }
    }

    #[test]
    fn progress_lines_parse_and_noise_does_not() {
        let p = parse_progress("Receiving objects:  45% (450/1000), 1.20 MiB | 2.00 MiB/s").unwrap();
        assert_eq!(p.phase.as_deref(), Some("Receiving objects"));
        assert_eq!(p.percent, Some(45));
        assert_eq!(p.detail.as_deref(), Some("1.20 MiB | 2.00 MiB/s"));
        let p = parse_progress("remote: Counting objects: 100% (5/5), done.").unwrap();
        assert_eq!((p.phase.as_deref(), p.percent, p.detail), (Some("Counting objects"), Some(100), None));
        assert!(parse_progress("Cloning into 'x'...").is_none());
        assert!(parse_progress("remote: Enumerating objects: 5, done.").is_none());
        assert!(parse_progress("warning: You appear to have cloned an empty repository.").is_none());
    }

    #[test]
    fn env_turns_off_prompts_and_keeps_the_users_ssh() {
        let env = clone_env("ssh -F ~/.ssh/work_config");
        assert!(env.contains(&("GIT_TERMINAL_PROMPT", "0".to_string())));
        let ssh = &env.iter().find(|(k, _)| *k == "GIT_SSH_COMMAND").unwrap().1;
        assert!(ssh.starts_with("ssh -F ~/.ssh/work_config "), "{ssh}");
        assert!(ssh.contains("-o BatchMode=yes"), "{ssh}");
    }

    #[test]
    fn a_file_url_clones_for_real_and_reports_progress() {
        let d = tmp("real");
        let bare = remote(&d);
        let src = parse_source(&format!("file://{}", bare.display())).unwrap();
        assert_eq!(src.name, "remote");
        let target = plan_target(d.join("projects/nested").to_str().unwrap(), &src.name).unwrap();
        let mut samples = Vec::new();
        let mut sink = |p: CloneProgress| samples.push(p);
        let out = clone_repo(&src, &target, &CloneOpts::default(), &AtomicBool::new(false), Some(&mut sink)).unwrap();
        assert_eq!(out.dir, target.to_string_lossy());
        assert!(!out.has_submodules);
        assert!(target.join("README.md").exists());
        assert!(!samples.is_empty(), "no progress at all");
        let head = Command::new("git").args(["-C", &out.dir, "branch", "--show-current"]).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&head.stdout).trim(), "trunk", "remote HEAD is checked out");
    }

    #[test]
    fn a_failed_clone_removes_only_the_dir_it_created() {
        let d = tmp("fail");
        let parent = d.join("parent");
        std::fs::create_dir_all(&parent).unwrap();
        std::fs::write(parent.join("keep.txt"), "mine").unwrap();
        let src = parse_source(&format!("file://{}/missing.git", d.display())).unwrap();
        let target = plan_target(parent.to_str().unwrap(), &src.name).unwrap();
        let e = clone_repo(&src, &target, &CloneOpts::default(), &AtomicBool::new(false), None).unwrap_err();
        assert_eq!(e.kind, CloneErrorKind::NotFound, "{}", e.message);
        assert!(!target.exists(), "the claimed dir is cleaned up");
        assert!(parent.join("keep.txt").exists(), "the parent and its contents are not");
    }

    #[test]
    fn a_target_that_appears_after_planning_is_never_written_or_removed() {
        let d = tmp("race");
        let bare = remote(&d);
        let src = parse_source(&format!("file://{}", bare.display())).unwrap();
        let target = plan_target(d.to_str().unwrap(), "late").unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("theirs.txt"), "x").unwrap();
        let e = clone_repo(&src, &target, &CloneOpts::default(), &AtomicBool::new(false), None).unwrap_err();
        assert_eq!(e.kind, CloneErrorKind::Exists);
        assert!(target.join("theirs.txt").exists());
    }

    #[test]
    fn cancel_kills_the_whole_group_and_removes_the_partial_clone() {
        let d = tmp("cancel");
        let pidfile = d.join("ssh.pid");
        // A "remote" whose ssh never answers: the shape of a hang. It records
        // its pid so the test can prove the GRANDCHILD died, not just git.
        let fake = d.join("fake-ssh");
        std::fs::write(&fake, format!("#!/bin/sh\necho $$ > '{}'\nexec sleep 60\n", pidfile.display())).unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let src = parse_source("ssh://example.invalid/owner/slow.git").unwrap();
        let target = plan_target(d.to_str().unwrap(), &src.name).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let c2 = cancel.clone();
        let pf = pidfile.clone();
        std::thread::spawn(move || {
            for _ in 0..100 {
                if pf.exists() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            c2.store(true, Ordering::SeqCst);
        });
        let opts = CloneOpts { ssh_base: Some(fake.to_string_lossy().into_owned()) };
        let t0 = Instant::now();
        let e = clone_repo(&src, &target, &opts, &cancel, None).unwrap_err();
        assert_eq!(e.kind, CloneErrorKind::Cancelled, "{}", e.message);
        assert!(t0.elapsed() < Duration::from_secs(15), "cancel took {:?}", t0.elapsed());
        assert!(!target.exists(), "partial clone removed");
        let pid: i32 = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
        std::thread::sleep(Duration::from_millis(200));
        // SAFETY: signal 0 only probes for existence.
        let alive = unsafe { libc::kill(pid, 0) } == 0;
        assert!(!alive, "the fake ssh (git's grandchild) survived the cancel");
    }
}
