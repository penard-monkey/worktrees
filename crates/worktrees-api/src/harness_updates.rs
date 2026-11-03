//! Host-global, fixed-source updates for the four agent CLIs.
//!
//! A path is an update target only when it proves which installer owns it.
//! An editor-bundled or otherwise unknown executable is shown, never changed.
use std::path::{Path, PathBuf};
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use worktrees_core::childenv;
use worktrees_core::proc::run_deadline;

use crate::ops::CmdResult;

const CACHE_AGE: Duration = Duration::from_secs(180);
const CLAUDE_NPM: &str = "https://registry.npmjs.org/@anthropic-ai%2fclaude-code/latest";
const CODEX_NPM: &str = "https://registry.npmjs.org/@openai%2fcodex/latest";
const KIMI_NPM: &str = "https://registry.npmjs.org/@moonshot-ai%2fkimi-code/latest";
const PI_NPM: &str = "https://registry.npmjs.org/@earendil-works%2fpi-coding-agent/latest";
const CODEX_CASK_API: &str = "https://formulae.brew.sh/api/cask/codex.json";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Tool { Claude, Codex, Kimi, Pi }

impl Tool {
    const ALL: [Self; 4] = [Self::Claude, Self::Codex, Self::Kimi, Self::Pi];
    fn name(self) -> &'static str {
        match self { Self::Claude => "claude", Self::Codex => "codex", Self::Kimi => "kimi", Self::Pi => "pi" }
    }
    fn index(self) -> usize {
        match self { Self::Claude => 0, Self::Codex => 1, Self::Kimi => 2, Self::Pi => 3 }
    }
    fn package(self) -> &'static str {
        match self {
            Self::Claude => "@anthropic-ai/claude-code",
            Self::Codex => "@openai/codex",
            Self::Kimi => "@moonshot-ai/kimi-code",
            Self::Pi => "@earendil-works/pi-coding-agent",
        }
    }
    fn npm_url(self) -> &'static str {
        match self { Self::Claude => CLAUDE_NPM, Self::Codex => CODEX_NPM, Self::Kimi => KIMI_NPM, Self::Pi => PI_NPM }
    }
    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.name() == s)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Method { Native, Npm, Homebrew, Managed, LegacyPython, Unknown, NotInstalled }

impl Method {
    fn name(self) -> &'static str {
        match self {
            Self::Native => "native", Self::Npm => "npm", Self::Homebrew => "homebrew",
            Self::Managed => "managed", Self::LegacyPython => "legacy-python",
            Self::Unknown => "unknown", Self::NotInstalled => "not-installed",
        }
    }
    fn command(self, tool: Tool) -> Option<&'static str> {
        match (tool, self) {
            (Tool::Claude, Self::Native) => Some("claude update"),
            (Tool::Claude, Self::Npm) => Some("npm install -g @anthropic-ai/claude-code@latest"),
            (Tool::Codex, Self::Npm) => Some("npm install -g @openai/codex@latest"),
            (Tool::Codex, Self::Homebrew) => Some("brew upgrade --cask codex"),
            (Tool::Kimi, Self::Native) => Some("kimi upgrade -y"),
            (Tool::Kimi, Self::Npm) => Some("npm install -g @moonshot-ai/kimi-code@latest"),
            (Tool::Pi, Self::Managed) => Some("pi update"),
            (Tool::Pi, Self::Npm) => Some("npm install -g --ignore-scripts @earendil-works/pi-coding-agent@latest"),
            _ => None,
        }
    }
}

/// What the update row answers when the install has no fixed command here.
const NO_FIXED_UPDATE: &str =
    "Worktrees can't update this install itself; follow the steps shown for it in Settings → Updates";

/// Where an install Worktrees will not update came from, judged from its
/// paths alone. It only picks which steps a person is shown, never a command
/// the host runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Origin { Override, Editor, PiCustomUpdate, Homebrew, OtherNode, Other }

/// Folders under HOME where editors keep extensions, each version in its own
/// folder (`publisher.name-<version>`).
const EDITOR_EXTENSIONS: [&str; 5] = [
    ".vscode/extensions", ".vscode-insiders/extensions", ".vscode-server/extensions",
    ".cursor/extensions", ".windsurf/extensions",
];

/// One thing a person does by hand. Every string is fixed in this file, never
/// a path or anything read from the machine: a paired device can read it.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct ManualStep {
    pub text: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<&'static str>,
}

/// Why Worktrees leaves an install alone, and how to update it instead.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Manual {
    pub why: &'static str,
    pub steps: &'static [ManualStep],
}

const fn say(text: &'static str) -> ManualStep { ManualStep { text, command: None } }
const fn run(text: &'static str, command: &'static str) -> ManualStep { ManualStep { text, command: Some(command) } }

const OTHER_NODE: &str = "This copy belongs to a different Node.js (another nvm version, pnpm, yarn or bun) \
    than the npm Worktrees finds, so Worktrees leaves it alone.";
const OTHER_NODE_TAIL: ManualStep = say("With pnpm, yarn or bun, use that tool's global upgrade instead.");

const CLAUDE_EDITOR: Manual = Manual {
    why: "This Claude Code came with an editor extension, which keeps its own copy.",
    steps: &[
        run("Install Claude Code for the terminal:", "curl -fsSL https://claude.ai/install.sh | bash"),
        say("Then press Check for updates. Worktrees can update that copy itself."),
    ],
};
const CLAUDE_BREW: Manual = Manual {
    why: "Homebrew installed this Claude Code, so Homebrew updates it.",
    steps: &[run("In a terminal, run:", "brew upgrade --cask claude-code")],
};
const CLAUDE_NODE: Manual = Manual {
    why: OTHER_NODE,
    steps: &[run("Switch to that Node.js, then run:", "npm install -g @anthropic-ai/claude-code@latest"), OTHER_NODE_TAIL],
};
const CLAUDE_OTHER: Manual = Manual {
    why: "Worktrees can't tell how this Claude Code was installed.",
    steps: &[
        run("Let Claude Code update itself:", "claude update"),
        run("If that fails, reinstall it with Anthropic's installer:", "curl -fsSL https://claude.ai/install.sh | bash"),
    ],
};
const CODEX_EDITOR: Manual = Manual {
    why: "This Codex came with the Codex extension in your editor. Each extension update lands in a new \
        folder while PATH keeps naming this one, so updating the extension won't change it.",
    steps: &[
        run("Install the standalone Codex CLI:", "npm install -g @openai/codex"),
        run("Or with Homebrew:", "brew install --cask codex"),
        run("In a new terminal, check that codex now resolves outside the extension's folder:", "which codex"),
        say("If it still points into the extension, put npm's or Homebrew's bin directory before it in PATH. \
            Then press Check for updates: Worktrees can update the standalone copy itself."),
    ],
};
const CODEX_BREW: Manual = Manual {
    why: "Homebrew installed this Codex, but not as the linked codex cask Worktrees updates.",
    steps: &[run("In a terminal, run:", "brew upgrade --cask codex")],
};
const CODEX_NODE: Manual = Manual {
    why: OTHER_NODE,
    steps: &[run("Switch to that Node.js, then run:", "npm install -g @openai/codex@latest"), OTHER_NODE_TAIL],
};
const CODEX_OTHER: Manual = Manual {
    why: "Worktrees can't tell how this Codex was installed, for example a binary downloaded from GitHub.",
    steps: &[
        run("Installed with npm:", "npm install -g @openai/codex@latest"),
        run("Installed with Homebrew:", "brew upgrade --cask codex"),
        say("Downloaded from GitHub: replace it with the latest release from github.com/openai/codex/releases."),
    ],
};
const KIMI_LEGACY: Manual = Manual {
    why: "This is the old Python kimi-cli, which Kimi Code replaces.",
    steps: &[
        run("Install Kimi Code:", "curl -fsSL https://code.kimi.com/kimi-code/install.sh | bash"),
        run("Remove the old kimi-cli so it stops shadowing the new one (if uv installed it):", "uv tool uninstall kimi-cli"),
        run("In a new terminal, bring your config, MCP servers and sessions across:", "kimi migrate"),
    ],
};
const KIMI_OVERRIDE: Manual = Manual {
    why: "WORKTREES_KIMI_BIN points Worktrees at this Kimi, so updating it is up to you.",
    steps: &[say("Update the copy it names, or unset WORKTREES_KIMI_BIN to use your installed Kimi Code.")],
};
const KIMI_NODE: Manual = Manual {
    why: OTHER_NODE,
    steps: &[run("Switch to that Node.js, then run:", "npm install -g @moonshot-ai/kimi-code@latest"), OTHER_NODE_TAIL],
};
const KIMI_OTHER: Manual = Manual {
    why: "Worktrees can't tell how this Kimi Code was installed, for example the install script with a custom KIMI_INSTALL_DIR.",
    steps: &[
        run("Let Kimi Code update itself:", "kimi upgrade"),
        run("Installed with npm:", "npm install -g @moonshot-ai/kimi-code@latest"),
    ],
};
const PI_CUSTOM: Manual = Manual {
    why: "Your Pi settings or environment change what pi update runs, so Worktrees won't run it for you.",
    steps: &[run("In a terminal, run:", "pi update")],
};
const PI_OVERRIDE: Manual = Manual {
    why: "WORKTREES_PI_BIN points Worktrees at this Pi, so updating it is up to you.",
    steps: &[say("Update the copy it names, or unset WORKTREES_PI_BIN to use your installed Pi.")],
};
const PI_NODE: Manual = Manual {
    why: OTHER_NODE,
    steps: &[run("Switch to that Node.js, then run:", "npm install -g @earendil-works/pi-coding-agent@latest"), OTHER_NODE_TAIL],
};
const PI_OTHER: Manual = Manual {
    why: "Worktrees can't tell how this Pi was installed.",
    steps: &[
        run("Installed with Pi's installer:", "pi update"),
        run("Installed with npm:", "npm install -g @earendil-works/pi-coding-agent@latest"),
    ],
};

/// The steps for an install that has no Update button. None when Worktrees
/// can update it, or when there is nothing installed to update.
fn manual(tool: Tool, method: Method, origin: Origin) -> Option<Manual> {
    if method == Method::NotInstalled || method.command(tool).is_some() { return None; }
    Some(match (tool, method, origin) {
        (Tool::Kimi, Method::LegacyPython, _) => KIMI_LEGACY,
        (Tool::Kimi, _, Origin::Override) => KIMI_OVERRIDE,
        (Tool::Pi, _, Origin::Override) => PI_OVERRIDE,
        (Tool::Pi, _, Origin::PiCustomUpdate) => PI_CUSTOM,
        (Tool::Claude, _, Origin::Editor) => CLAUDE_EDITOR,
        (Tool::Codex, _, Origin::Editor) => CODEX_EDITOR,
        (Tool::Claude, _, Origin::Homebrew) => CLAUDE_BREW,
        (Tool::Codex, _, Origin::Homebrew) => CODEX_BREW,
        (Tool::Claude, _, Origin::OtherNode) => CLAUDE_NODE,
        (Tool::Codex, _, Origin::OtherNode) => CODEX_NODE,
        (Tool::Kimi, _, Origin::OtherNode) => KIMI_NODE,
        (Tool::Pi, _, Origin::OtherNode) => PI_NODE,
        (Tool::Claude, _, _) => CLAUDE_OTHER,
        (Tool::Codex, _, _) => CODEX_OTHER,
        (Tool::Kimi, _, _) => KIMI_OTHER,
        (Tool::Pi, _, _) => PI_OTHER,
    })
}

#[derive(Clone, Serialize)]
pub struct HarnessUpdate {
    pub tool: &'static str,
    pub installed_version: Option<String>,
    pub latest_version: Option<String>,
    pub install_method: &'static str,
    pub update_available: bool,
    pub update_command: Option<&'static str>,
    /// How a person updates an install Worktrees won't touch. Absent when the
    /// row has an Update button or the tool is not installed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manual: Option<Manual>,
}

struct Probe {
    home: PathBuf,
    npm_prefix: Option<PathBuf>,
    brew_prefix: Option<PathBuf>,
    brew_codex: bool,
}

fn read_command(program: &str, args: &[&str], secs: u64) -> Option<String> {
    let mut c = childenv::command(program);
    c.args(args);
    if let Some(home) = std::env::var_os("HOME") { c.current_dir(home); }
    let out = run_deadline(c, secs).ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

impl Probe {
    fn current() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let npm_prefix = read_command("npm", &["prefix", "-g"], 4).filter(|v| v.starts_with('/')).map(PathBuf::from);
        let brew_prefix = read_command("brew", &["--prefix"], 4).filter(|v| v.starts_with('/')).map(PathBuf::from);
        let brew_codex = read_command("brew", &["list", "--cask", "--versions", "codex"], 5)
            .is_some_and(|s| s.starts_with("codex "));
        Self { home, npm_prefix, brew_prefix, brew_codex }
    }
}

fn find_bin_in(name: &str, path: &std::ffi::OsStr) -> Option<PathBuf> {
    std::env::split_paths(path).map(|d| d.join(name)).find(|p| p.is_file())
}

fn find_bin(name: &str) -> Option<PathBuf> {
    find_bin_in(name, &std::env::var_os("PATH")?)
}
fn find_tool_bin(tool: Tool) -> Option<PathBuf> {
    match tool {
        Tool::Kimi => worktrees_core::kimi::kimi_bin(),
        Tool::Pi => worktrees_core::pimodels::pi_bin(),
        _ => find_bin(tool.name()),
    }
}

fn under(path: &Path, parent: &Path) -> bool { path.strip_prefix(parent).is_ok() }
fn canonical(path: &Path) -> PathBuf { std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned()) }
fn legacy_kimi(path: &Path) -> bool {
    let Ok(mut f) = std::fs::File::open(path) else { return false };
    let mut head = [0u8; 4096];
    let Ok(n) = f.read(&mut head) else { return false };
    String::from_utf8_lossy(&head[..n]).contains("kimi_cli")
}
fn pi_update_is_fixed(home: &Path) -> bool {
    // Pi can replace `pi update` with arbitrary argv via user settings or an
    // environment variable. A remote admin must not trigger that override.
    for key in ["PI_SELF_UPDATE_COMMAND", "PI_MANAGED_INSTALL_ROOT", "PI_CODING_AGENT_DIR", "PI_LEGACY_INSTALL"] {
        if std::env::var_os(key).is_some() { return false; }
    }
    let settings = home.join(".pi/agent/settings.json");
    let raw = match std::fs::read_to_string(settings) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return true,
        Err(_) => return false,
    };
    serde_json::from_str::<serde_json::Value>(&raw).ok()
        .is_some_and(|v| v.get("selfUpdateCommand").is_none_or(serde_json::Value::is_null))
}
fn pi_managed_launcher(path: &Path) -> bool {
    let Ok(mut f) = std::fs::File::open(path) else { return false };
    let mut head = [0u8; 4096];
    let Ok(n) = f.read(&mut head) else { return false };
    let text = String::from_utf8_lossy(&head[..n]);
    text.starts_with("#!/bin/sh")
        && text.contains("pi_current_file=$pi_agent_dir/install/current-version")
        && text.contains("PI_MANAGED_INSTALL_ROOT=$pi_agent_dir/install")
        && text.contains("exec \"$pi_release_bin\" \"$@\"")
}

/// Pure ownership rule. `path` is the PATH entry; `real` is its canonical
/// target. The npm prefix is the one the current npm would modify.
fn method(tool: Tool, path: &Path, real: &Path, p: &Probe) -> Method {
    if (tool == Tool::Kimi && std::env::var_os("WORKTREES_KIMI_BIN").is_some())
        || (tool == Tool::Pi && std::env::var_os("WORKTREES_PI_BIN").is_some()) {
        // Development overrides are not an install method, even if they
        // happen to point at a familiar directory.
        return Method::Unknown;
    }
    let native_claude = p.home.join(".local/bin/claude");
    let claude_versions = canonical(&p.home.join(".local/share/claude/versions"));
    if tool == Tool::Claude && path == native_claude && under(real, &claude_versions)
        && real.parent() == Some(claude_versions.as_path())
        && real.file_name().and_then(|n| n.to_str()).and_then(numeric).is_some() {
        return Method::Native;
    }
    if tool == Tool::Kimi && path == p.home.join(".kimi-code/bin/kimi")
        && std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file()) {
        return Method::Native;
    }
    if tool == Tool::Pi {
        let root = p.home.join(".pi/agent/install");
        let launcher = p.home.join(".pi/agent/bin/pi");
        let marker = root.join("managed-install.json");
        let valid = std::fs::read_to_string(marker).ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .is_some_and(|v| v["kind"] == "pi-managed-install" && v["schemaVersion"] == 1 && v["layout"] == "releases-v1");
        let version_file = std::fs::read_to_string(root.join("current-version")).ok()
            .is_some_and(|s| numeric(s.trim()).is_some());
        if valid && version_file && pi_update_is_fixed(&p.home)
            && real == canonical(&launcher) && pi_managed_launcher(real) {
            return Method::Managed;
        }
    }
    if let Some(prefix) = &p.npm_prefix {
        let bin = prefix.join("bin").join(tool.name());
        let pkg = prefix.join("lib/node_modules").join(tool.package());
        if path == bin && under(real, &canonical(&pkg)) && path != real {
            return Method::Npm;
        }
    }
    if tool == Tool::Codex && p.brew_codex {
        if let Some(prefix) = &p.brew_prefix {
            if path == prefix.join("bin/codex") && under(real, &canonical(&prefix.join("Caskroom/codex"))) {
                return Method::Homebrew;
            }
        }
    }
    if tool == Tool::Kimi && legacy_kimi(real) {
        return Method::LegacyPython;
    }
    Method::Unknown
}

/// Pure origin rule, consulted only for an install `method` left without a
/// fixed command: which by-hand steps fit where this executable lives.
fn origin(tool: Tool, path: &Path, real: &Path, p: &Probe) -> Origin {
    if (tool == Tool::Kimi && std::env::var_os("WORKTREES_KIMI_BIN").is_some())
        || (tool == Tool::Pi && std::env::var_os("WORKTREES_PI_BIN").is_some()) {
        return Origin::Override;
    }
    if EDITOR_EXTENSIONS.iter().any(|d| {
        let dir = p.home.join(d);
        under(path, &dir) || under(real, &dir) || under(real, &canonical(&dir))
    }) {
        return Origin::Editor;
    }
    if tool == Tool::Pi && !pi_update_is_fixed(&p.home)
        && real == canonical(&p.home.join(".pi/agent/bin/pi")) && pi_managed_launcher(real) {
        return Origin::PiCustomUpdate;
    }
    if let Some(prefix) = &p.brew_prefix {
        if ["Caskroom", "Cellar"].iter().any(|d| under(real, &canonical(&prefix.join(d)))) {
            return Origin::Homebrew;
        }
    }
    let package = Path::new("node_modules").join(tool.package());
    let parts: Vec<_> = real.components().collect();
    let want: Vec<_> = package.components().collect();
    if parts.windows(want.len()).any(|w| w == want.as_slice()) {
        return Origin::OtherNode;
    }
    Origin::Other
}

/// First dotted numeric version in a CLI's version line. Never display raw
/// output from a tool here: a broken executable might print credentials.
fn version(text: &str) -> Option<String> {
    for token in text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '+')) {
        let v = token.strip_prefix('v').unwrap_or(token);
        let core = v.split(['-', '+']).next().unwrap_or(v);
        let parts: Vec<_> = core.split('.').collect();
        if parts.len() >= 2 && parts.len() <= 4 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())) {
            return Some(v.to_owned());
        }
    }
    None
}

fn numeric(v: &str) -> Option<Vec<u64>> {
    let core = v.trim_start_matches('v').split(['-', '+']).next()?;
    let parts: Vec<u64> = core.split('.').map(str::parse).collect::<Result<_, _>>().ok()?;
    (parts.len() >= 2 && parts.len() <= 4).then_some(parts)
}

fn newer(latest: &str, installed: &str) -> bool {
    match (numeric(latest), numeric(installed)) {
        (Some(mut a), Some(mut b)) => {
            a.resize(4, 0); b.resize(4, 0);
            a > b || (a == b && installed.contains('-') && !latest.contains('-'))
        }
        _ => false,
    }
}

fn fetch_version_from(url: &str, key: &str, proto: &str) -> Option<String> {
    let mut c = childenv::command("curl");
    c.args(["-q", "-fsS", "--max-time", "6", "--connect-timeout", "3", "--max-filesize", "131072",
        "--noproxy", "*", "--proto", proto, url]);
    let out = run_deadline(c, 8).ok()?;
    if !out.status.success() { return None; }
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    value.get(key).and_then(|v| v.as_str()).and_then(version)
}

fn fetch_version(url: &str, key: &str) -> Option<String> {
    fetch_version_from(url, key, "=https")
}

fn installed_at(tool: Tool, path: PathBuf, p: &Probe) -> (Option<PathBuf>, Option<String>, Method) {
    let real = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
    let method = method(tool, &path, &real, p);
    let mut c = childenv::command(&path);
    c.arg("--version");
    c.current_dir(&p.home);
    if tool == Tool::Kimi { c.env("KIMI_CODE_NO_AUTO_UPDATE", "1"); }
    if tool == Tool::Pi { c.env("PI_SKIP_VERSION_CHECK", "1"); }
    let installed_version = run_deadline(c, 5).ok().filter(|o| o.status.success())
        .and_then(|o| version(&String::from_utf8_lossy(&o.stdout)));
    (Some(path), installed_version, method)
}

fn installed(tool: Tool, p: &Probe) -> (Option<PathBuf>, Option<String>, Method) {
    let Some(path) = find_tool_bin(tool) else { return (None, None, Method::NotInstalled) };
    installed_at(tool, path, p)
}

fn check_one(tool: Tool, p: &Probe) -> HarnessUpdate {
    let (path, installed_version, method) = installed(tool, p);
    let manual = path.as_deref().and_then(|path| manual(tool, method, origin(tool, path, &canonical(path), p)));
    let latest_version = if method == Method::LegacyPython {
        None
    } else if tool == Tool::Codex && method == Method::Homebrew {
        fetch_version(CODEX_CASK_API, "version")
    } else {
        fetch_version(tool.npm_url(), "version")
    };
    let update_available = match (&latest_version, &installed_version) {
        (Some(l), Some(i)) => newer(l, i),
        _ => false,
    };
    HarnessUpdate {
        tool: tool.name(), installed_version, latest_version,
        install_method: method.name(), update_available,
        update_command: method.command(tool),
        manual,
    }
}

static CACHE: Mutex<Option<(Instant, Vec<HarnessUpdate>)>> = Mutex::new(None);
static UPDATING: [AtomicBool; 4] = [const { AtomicBool::new(false) }; 4];

pub fn check_all() -> Vec<HarnessUpdate> {
    // The lock is held across the fetch: a manual check cannot start a second
    // set of four probes while the hourly one is in flight.
    let mut cache = CACHE.lock().unwrap();
    if let Some((at, rows)) = &*cache {
        if at.elapsed() < CACHE_AGE { return rows.clone(); }
    }
    let p = Probe::current();
    let rows = std::thread::scope(|s| {
        let probe = &p;
        let jobs: Vec<_> = Tool::ALL.into_iter().map(|tool| s.spawn(move || check_one(tool, probe))).collect();
        jobs.into_iter().map(|j| j.join().unwrap()).collect::<Vec<_>>()
    });
    *cache = Some((Instant::now(), rows.clone()));
    rows
}

struct Running(Tool);
impl Drop for Running {
    fn drop(&mut self) { UPDATING[self.0.index()].store(false, Ordering::Release); }
}
fn acquire(tool: Tool) -> Result<Running, String> {
    UPDATING[tool.index()].compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| format!("{} is already updating", tool.name()))?;
    Ok(Running(tool))
}

pub fn update(tool_name: String) -> Result<CmdResult, String> {
    let tool = Tool::parse(&tool_name).ok_or("unknown harness")?;
    let _running = acquire(tool)?;
    let p = Probe::current();
    let (path, _before, method) = installed(tool, &p);
    let path = path.ok_or("harness is not installed")?;
    let label = method.command(tool).ok_or(NO_FIXED_UPDATE)?;
    let mut c = match method {
        Method::Native | Method::Managed => childenv::command(&path),
        Method::Npm => childenv::command("npm"),
        Method::Homebrew => childenv::command("brew"),
        _ => return Err(NO_FIXED_UPDATE.into()),
    };
    // A serve started from a checkout must not let that checkout's npm config
    // or Pi project settings influence a host-global update.
    c.current_dir(&p.home);
    match (tool, method) {
        (Tool::Claude, Method::Native) => { c.arg("update"); }
        (Tool::Kimi, Method::Native) => { c.args(["upgrade", "-y"]); }
        (Tool::Pi, Method::Managed) => { c.arg("update"); }
        (Tool::Codex, Method::Homebrew) => { c.args(["upgrade", "--cask", "codex"]); }
        (Tool::Pi, Method::Npm) => { c.args(["install", "-g", "--ignore-scripts", "@earendil-works/pi-coding-agent@latest"]); }
        (_, Method::Npm) => { c.args(["install", "-g", &format!("{}@latest", tool.package())]); }
        _ => return Err(NO_FIXED_UPDATE.into()),
    }
    let out = run_deadline(c, 180).map_err(|e| format!("{label}: {e}"))?;
    *CACHE.lock().unwrap() = None;
    let (new_path, after, new_method) = installed(tool, &p);
    let mut output = String::from_utf8_lossy(&out.stdout).to_string();
    output.push_str(&String::from_utf8_lossy(&out.stderr));
    output.truncate(output.floor_char_boundary(output.len().min(16_384)));
    if let Some(v) = after.as_deref() { output.push_str(&format!("\n{} now reports {v}", tool.name())); }
    let verified = new_path.as_deref() == Some(path.as_path()) && new_method == method && after.is_some();
    if !verified { output.push_str("\nCould not verify the updated executable."); }
    let ok = out.status.success() && verified;
    Ok(CmdResult { ok, code: out.status.code().unwrap_or(-1), output, slug: None, needs_confirm: None, warnings: Vec::new() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::unix::fs::symlink;

    fn script(path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // The writer is a reaped process, not an open fd inherited by another
        // test's fork (the same rule as core's fsx::exec_fixture).
        let mut sh = childenv::command("/bin/sh")
            .args(["-c", "cat > \"$1\" && chmod 0755 \"$1\"", "sh"])
            .arg(path).stdin(std::process::Stdio::piped()).spawn().unwrap();
        sh.stdin.take().unwrap().write_all(body.as_bytes()).unwrap();
        assert!(sh.wait().unwrap().success());
    }

    #[test]
    fn installer_ownership_and_versions_use_the_resolved_binary() {
        let scratch = worktrees_core::fsx::PrivateDir::new(&std::env::temp_dir(), "harness-updates").unwrap();
        let home = scratch.path();
        let npm = home.join("npm");
        let brew = home.join("brew");
        let p = Probe { home: home.to_owned(), npm_prefix: Some(npm.clone()), brew_prefix: Some(brew.clone()), brew_codex: true };

        let cv = home.join(".local/share/claude/versions/2.1.317");
        script(&cv, "#!/bin/sh\necho '2.1.317 (Claude Code)'\n");
        let cb = home.join(".local/bin/claude");
        std::fs::create_dir_all(cb.parent().unwrap()).unwrap();
        symlink(&cv, &cb).unwrap();
        let path = std::env::join_paths([cb.parent().unwrap()]).unwrap();
        assert_eq!(find_bin_in("claude", &path).as_deref(), Some(cb.as_path()));
        let (_, v, m) = installed_at(Tool::Claude, cb, &p);
        assert_eq!(v.as_deref(), Some("2.1.317"));
        assert_eq!(m, Method::Native);

        let codex_real = npm.join("lib/node_modules/@openai/codex/bin/codex");
        script(&codex_real, "#!/bin/sh\necho 'codex-cli 0.160.1'\n");
        let codex_bin = npm.join("bin/codex");
        std::fs::create_dir_all(codex_bin.parent().unwrap()).unwrap();
        symlink(&codex_real, &codex_bin).unwrap();
        let (_, v, m) = installed_at(Tool::Codex, codex_bin, &p);
        assert_eq!(v.as_deref(), Some("0.160.1"));
        assert_eq!(m, Method::Npm);

        let cask_real = brew.join("Caskroom/codex/0.166.0/codex");
        script(&cask_real, "#!/bin/sh\necho 'codex-cli 0.166.0'\n");
        let cask_bin = brew.join("bin/codex");
        std::fs::create_dir_all(cask_bin.parent().unwrap()).unwrap();
        symlink(&cask_real, &cask_bin).unwrap();
        let cask_target = canonical(&cask_bin);
        assert_eq!(method(Tool::Codex, &cask_bin, &cask_target, &p), Method::Homebrew);
        assert_eq!(method(Tool::Codex, &home.join(".vscode/extensions/codex"), &cask_target, &p), Method::Unknown);

        let kimi = home.join(".kimi-code/bin/kimi");
        script(&kimi, "#!/bin/sh\necho 'kimi 0.43.1'\n");
        assert_eq!(installed_at(Tool::Kimi, kimi, &p).2, Method::Native);
        let legacy = home.join("uv/bin/kimi");
        script(&legacy, "#!/bin/sh\n# kimi_cli\necho 'kimi 1.52.0'\n");
        assert_eq!(installed_at(Tool::Kimi, legacy, &p).2, Method::LegacyPython);

        let pi_root = home.join(".pi/agent/install");
        std::fs::create_dir_all(&pi_root).unwrap();
        std::fs::write(pi_root.join("managed-install.json"), r#"{"kind":"pi-managed-install","schemaVersion":1,"layout":"releases-v1"}"#).unwrap();
        std::fs::write(pi_root.join("current-version"), "0.99.1\n").unwrap();
        let pi_real = home.join(".pi/agent/bin/pi");
        script(&pi_real, "#!/bin/sh\n# pi_current_file=$pi_agent_dir/install/current-version\n# PI_MANAGED_INSTALL_ROOT=$pi_agent_dir/install\n# exec \"$pi_release_bin\" \"$@\"\necho 'pi 0.99.1'\n");
        let pi_bin = home.join(".local/bin/pi");
        symlink(&pi_real, &pi_bin).unwrap();
        assert_eq!(installed_at(Tool::Pi, pi_bin.clone(), &p).2, Method::Managed);
        std::fs::write(home.join(".pi/agent/settings.json"), r#"{"selfUpdateCommand":["sh","-c","echo override"]}"#).unwrap();
        assert_eq!(installed_at(Tool::Pi, pi_bin, &p).2, Method::Unknown);
    }

    #[test]
    fn an_install_without_an_update_button_gets_steps_for_where_it_came_from() {
        let scratch = worktrees_core::fsx::PrivateDir::new(&std::env::temp_dir(), "harness-manual").unwrap();
        let home = scratch.path();
        let brew = home.join("brew");
        let p = Probe { home: home.to_owned(), npm_prefix: Some(home.join("npm")), brew_prefix: Some(brew.clone()), brew_codex: false };
        let steps_for = |tool, path: &Path| {
            let real = canonical(path);
            manual(tool, method(tool, path, &real, &p), origin(tool, path, &real, &p))
        };
        let commands = |m: Option<Manual>| m.expect("steps").steps.iter().filter_map(|s| s.command).collect::<Vec<_>>();

        // This host's Codex: the editor extension's own binary, on PATH by its
        // versioned folder. Updating the extension writes a NEW folder.
        let ext = home.join(".vscode/extensions/openai.chatgpt-26.930.61225-darwin-arm64/bin/macos-aarch64/codex");
        script(&ext, "#!/bin/sh\necho 'codex-cli 0.160.1'\n");
        let c = commands(steps_for(Tool::Codex, &ext));
        assert_eq!(c.first(), Some(&"npm install -g @openai/codex"));
        assert!(c.contains(&"which codex"), "{c:?}");

        // Homebrew holds it, but not as the linked, registered codex cask.
        let cask = brew.join("Caskroom/codex/0.166.0/codex");
        script(&cask, "#!/bin/sh\necho 'codex-cli 0.166.0'\n");
        assert_eq!(commands(steps_for(Tool::Codex, &cask)), ["brew upgrade --cask codex"]);
        let claude_cask = brew.join("Caskroom/claude-code/2.1.311/claude");
        script(&claude_cask, "#!/bin/sh\necho '2.1.311 (Claude Code)'\n");
        assert_eq!(commands(steps_for(Tool::Claude, &claude_cask)), ["brew upgrade --cask claude-code"]);

        // A global package of another Node than the npm on PATH.
        let other = home.join(".nvm/versions/node/v20.0.0");
        let pi_js = other.join("lib/node_modules/@earendil-works/pi-coding-agent/dist/cli.js");
        script(&pi_js, "#!/bin/sh\necho 'pi 0.99.1'\n");
        let pi_link = other.join("bin/pi");
        std::fs::create_dir_all(pi_link.parent().unwrap()).unwrap();
        symlink(&pi_js, &pi_link).unwrap();
        assert_eq!(steps_for(Tool::Pi, &pi_link).unwrap().why, OTHER_NODE);

        // The archived Python kimi-cli: install Kimi Code, then migrate.
        let legacy = home.join("uv/bin/kimi");
        script(&legacy, "#!/bin/sh\n# kimi_cli\necho 'kimi 1.52.0'\n");
        let c = commands(steps_for(Tool::Kimi, &legacy));
        assert!(c[0].contains("code.kimi.com/kimi-code/install.sh") && c.contains(&"kimi migrate"), "{c:?}");

        // Pi's own launcher whose update a setting replaced: run it by hand.
        let pi_root = home.join(".pi/agent/install");
        std::fs::create_dir_all(&pi_root).unwrap();
        std::fs::write(pi_root.join("managed-install.json"), r#"{"kind":"pi-managed-install","schemaVersion":1,"layout":"releases-v1"}"#).unwrap();
        std::fs::write(pi_root.join("current-version"), "0.99.1\n").unwrap();
        let launcher = home.join(".pi/agent/bin/pi");
        script(&launcher, "#!/bin/sh\n# pi_current_file=$pi_agent_dir/install/current-version\n# PI_MANAGED_INSTALL_ROOT=$pi_agent_dir/install\n# exec \"$pi_release_bin\" \"$@\"\necho 'pi 0.99.1'\n");
        std::fs::write(home.join(".pi/agent/settings.json"), r#"{"selfUpdateCommand":["sh","-c","echo override"]}"#).unwrap();
        assert_eq!(steps_for(Tool::Pi, &launcher).unwrap().why, PI_CUSTOM.why);
        assert_eq!(commands(steps_for(Tool::Pi, &launcher)), ["pi update"]);

        // Nothing to go on: the tool's own updater first.
        let lone = home.join("opt/claude");
        script(&lone, "#!/bin/sh\necho '2.1.300 (Claude Code)'\n");
        assert_eq!(commands(steps_for(Tool::Claude, &lone)), ["claude update", "curl -fsSL https://claude.ai/install.sh | bash"]);

        // A row with an Update button, or nothing installed, has no steps;
        // every other row has at least one, and serializes without paths.
        let origins = [Origin::Override, Origin::Editor, Origin::PiCustomUpdate, Origin::Homebrew, Origin::OtherNode, Origin::Other];
        for tool in Tool::ALL {
            for o in origins {
                for m in [Method::Native, Method::Npm, Method::Homebrew, Method::Managed, Method::LegacyPython, Method::Unknown, Method::NotInstalled] {
                    let steps = manual(tool, m, o);
                    assert_eq!(steps.is_none(), m == Method::NotInstalled || m.command(tool).is_some(), "{tool:?} {m:?} {o:?}");
                    if let Some(s) = steps { assert!(!s.steps.is_empty()); }
                }
            }
        }
        let row = |manual| serde_json::to_value(HarnessUpdate {
            tool: "codex", installed_version: Some("0.160.1".into()), latest_version: None,
            install_method: "unknown", update_available: false, update_command: None, manual,
        }).unwrap();
        assert!(row(None).get("manual").is_none());
        let json = row(Some(CODEX_EDITOR));
        assert_eq!(json["manual"]["steps"][0]["command"], "npm install -g @openai/codex");
        assert!(json["manual"]["steps"][3].get("command").is_none());
        assert!(!json.to_string().contains(&*home.to_string_lossy()));
    }

    #[test]
    fn latest_version_comes_from_a_fake_registry() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request).unwrap();
            let body = r#"{"version":"0.166.0"}"#;
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        });
        let url = format!("http://{addr}/@openai%2fcodex/latest");
        assert_eq!(fetch_version_from(&url, "version", "=http"), Some("0.166.0".into()));
        server.join().unwrap();
    }

    #[test]
    fn version_lines_and_comparison() {
        assert_eq!(version("2.1.317 (Claude Code)"), Some("2.1.317".into()));
        assert_eq!(version("codex-cli 0.160.1"), Some("0.160.1".into()));
        assert_eq!(version("kimi 1.52.0"), Some("1.52.0".into()));
        assert_eq!(version("kimi 0.71"), Some("0.71".into()));
        assert_eq!(version("pi v0.99.1"), Some("0.99.1".into()));
        assert!(newer("0.166.0", "0.160.1"));
        assert!(!newer("0.99.1", "0.99.1"));
        assert!(newer("0.99.1", "0.99.1-rc.1"));
        assert!(!newer("garbage", "0.1.0"));
    }

    #[test]
    fn unknown_tool_and_concurrent_update_are_refused() {
        assert!(update("other".into()).unwrap_err().contains("unknown harness"));
        let first = acquire(Tool::Pi).unwrap();
        assert!(acquire(Tool::Pi).err().unwrap().contains("already updating"));
        drop(first);
        assert!(acquire(Tool::Pi).is_ok());
    }
}
