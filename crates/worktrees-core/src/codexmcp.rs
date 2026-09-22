//! Codex's user-scope MCP setup. Read its TOML for status; let `codex mcp`
//! make every change to the file it owns.

use std::path::{Path, PathBuf};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Entry {
    pub command: String,
    pub args: Vec<String>,
    pub ours: bool,
    pub command_ok: bool,
    pub mutations: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Status {
    pub state: &'static str,
    pub codex_bin: Option<String>,
    pub worktrees_bin: Option<String>,
    pub entry: Option<Entry>,
    pub command: Option<String>,
    pub config_path: String,
}

#[derive(Serialize)]
pub struct Outcome {
    pub ok: bool,
    pub output: String,
    pub status: Status,
}

pub fn config_path() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".codex"))
        .join("config.toml")
}

fn registered_entry(path: &Path) -> Option<Entry> {
    let text = std::fs::read_to_string(path).ok()?;
    let root: toml::Value = toml::from_str(&text).ok()?;
    let server = root.get("mcp_servers")?.get("worktrees")?;
    let command = server.get("command").and_then(toml::Value::as_str).unwrap_or("").to_string();
    let args: Vec<String> = server.get("args").and_then(toml::Value::as_array)
        .map(|xs| xs.iter().filter_map(toml::Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let ours = command.rsplit('/').next() == Some("worktrees") && args.iter().any(|s| s == "mcp");
    let command_ok = if command.contains('/') {
        crate::profile::is_exec(Path::new(&command))
    } else {
        crate::profile::bin_on_path(&command).is_some()
    };
    let mutations = args.iter().any(|s| s == "--mutations");
    Some(Entry { command, args, ours, command_ok, mutations })
}

pub fn status() -> Status {
    let path = config_path();
    let entry = registered_entry(&path);
    let codex_bin = crate::profile::bin_on_path("codex").map(|p| p.to_string_lossy().into_owned());
    let worktrees_bin = crate::profile::worktrees_bin().map(|p| p.to_string_lossy().into_owned());
    let state = match &entry {
        Some(e) if !e.ours => "foreign",
        Some(e) if !e.command_ok => "stale",
        Some(e) if !e.mutations => "read-only",
        Some(_) => "installed",
        None if worktrees_bin.is_none() => "cli-missing",
        None => "absent",
    };
    let command = worktrees_bin.as_ref().map(|wt| format!("codex mcp add worktrees --env WORKTREES_MCP_PROVIDER=codex -- {} mcp --mutations", crate::profile::shell_quote(wt)));
    Status { state, codex_bin, worktrees_bin, entry, command, config_path: path.to_string_lossy().into_owned() }
}

fn run(codex: &str, args: &[&str]) -> Result<(bool, String), String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let mut child = Command::new(codex).args(args).stdin(Stdio::null())
        .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()
        .map_err(|e| format!("codex mcp {}: {e}", args.join(" ")))?;
    let drain = |mut stream: Option<Box<dyn Read + Send>>| std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(s) = stream.as_mut() { let _ = s.read_to_end(&mut bytes); }
        bytes
    });
    let stdout = drain(child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
    let stderr = drain(child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let exit = loop {
        if let Some(exit) = child.try_wait().map_err(|e| e.to_string())? { break exit; }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("codex mcp command timed out after 60 seconds".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let mut text = String::from_utf8_lossy(&stdout.join().unwrap_or_default()).into_owned();
    text.push_str(&String::from_utf8_lossy(&stderr.join().unwrap_or_default()));
    Ok((exit.success(), text))
}

pub fn install(mutations: bool) -> Result<Outcome, String> {
    let before = status();
    if before.state == "foreign" { return Err("a foreign MCP server is registered as worktrees in Codex; leaving it untouched".into()); }
    let codex = before.codex_bin.ok_or("codex is not on PATH")?;
    let wt = before.worktrees_bin.ok_or("worktrees is not on PATH")?;
    let mut output = String::new();
    if before.entry.is_some() {
        let (ok, text) = run(&codex, &["mcp", "remove", "worktrees"])?;
        output.push_str(&text);
        if !ok { return Err(format!("codex mcp remove failed: {output}")); }
    }
    let mut args = vec!["mcp", "add", "worktrees", "--env", "WORKTREES_MCP_PROVIDER=codex", "--", &wt, "mcp"];
    if mutations { args.push("--mutations"); }
    let (ok, text) = run(&codex, &args)?;
    output.push_str(&text);
    let after = status();
    let landed = after.entry.as_ref().is_some_and(|e| e.ours && e.mutations == mutations);
    Ok(Outcome { ok: ok && landed, output, status: after })
}

pub fn uninstall() -> Result<Outcome, String> {
    let before = status();
    if before.state == "foreign" { return Err("a foreign MCP server is registered as worktrees in Codex; leaving it untouched".into()); }
    if before.entry.is_none() { return Err("no Worktrees MCP server is installed in Codex".into()); }
    let codex = before.codex_bin.ok_or("codex is not on PATH")?;
    let (ok, output) = run(&codex, &["mcp", "remove", "worktrees"])?;
    let after = status();
    Ok(Outcome { ok: ok && after.entry.is_none(), output, status: after })
}
