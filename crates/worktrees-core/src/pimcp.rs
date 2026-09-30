//! pi's user-scope MCP setup (pi-harness §4.3). READ `<agent dir>/mcp.json`
//! for status; let `pi mcp add` / `pi mcp remove` make every change to the
//! file pi owns — the `~/.claude.json` rule, and `codexmcp.rs`'s shape.
//!
//! Three rules that are easy to lose:
//! - Detection never runs `pi mcp list`: it CONNECTS to every server the user
//!   has (0 tools outside a repo, 22 inside one — it answers "what does this
//!   serve from here", not "is it installed") and exits 1 if any of them,
//!   ours or not, is down.
//! - Never `-l`: that writes the place's `.pi/mcp.json`, i.e. into the repo.
//! - `pi mcp add` resolves `PI_CODING_AGENT_DIR` from ITS environment, so it
//!   is handed the same directory `pi::agent_dir()` read, or status and
//!   install could be about two different files.

use std::path::{Path, PathBuf};
use serde::Serialize;

/// The server's name in `mcpServers`; pi names its tools `mcp__worktrees__*`.
pub const SERVER_KEY: &str = "worktrees";
/// Decided 2026-09-29 (pi-harness Q9): tools declared like built-ins.
pub const EXPOSURE: &str = "direct";
/// What the server is told about its client (see `cmd_mcp`).
pub const PROVIDER_ENV: &str = "WORKTREES_MCP_PROVIDER=pi";

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Entry {
    pub command: String,
    pub args: Vec<String>,
    pub ours: bool,
    pub command_ok: bool,
    pub mutations: bool,
    /// As written; `None` is pi's default, `codemode`.
    pub exposure: Option<String>,
    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Status {
    /// `installed` · `read-only` · `disabled` · `stale` · `foreign` ·
    /// `unreadable` · `absent` · `cli-missing` · `pi-missing`.
    pub state: &'static str,
    pub pi_bin: Option<String>,
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
    crate::pi::agent_dir().join("mcp.json")
}

/// `Err` when the file exists and is not an `mcp.json` pi could read either.
pub fn parse_entry(text: &str) -> Result<Option<Entry>, String> {
    let root: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let Some(server) = root.get("mcpServers").and_then(|m| m.get(SERVER_KEY)) else {
        return Ok(None);
    };
    let command = server.get("command").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let args: Vec<String> = server
        .get("args")
        .and_then(|v| v.as_array())
        .map(|xs| xs.iter().filter_map(|x| x.as_str()).map(str::to_string).collect())
        .unwrap_or_default();
    let ours = command.rsplit('/').next() == Some("worktrees") && args.first().map(String::as_str) == Some("mcp");
    let command_ok = if command.contains('/') {
        crate::profile::is_exec(Path::new(&command))
    } else {
        crate::profile::bin_on_path(&command).is_some()
    };
    Ok(Some(Entry {
        mutations: args.iter().any(|s| s == "--mutations"),
        exposure: server.get("exposure").and_then(|v| v.as_str()).map(str::to_string),
        enabled: server.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
        command,
        args,
        ours,
        command_ok,
    }))
}

/// The state, from what was read and what is on PATH.
pub fn state_of(entry: Option<&Entry>, unreadable: bool, pi: bool, worktrees: bool) -> &'static str {
    match entry {
        _ if unreadable => "unreadable",
        Some(e) if !e.ours => "foreign",
        Some(e) if !e.command_ok => "stale",
        Some(e) if !e.enabled => "disabled",
        Some(e) if !e.mutations => "read-only",
        Some(_) => "installed",
        None if !pi => "pi-missing",
        None if !worktrees => "cli-missing",
        None => "absent",
    }
}

/// `pi mcp add`'s argv after the program, for `worktrees` at `wt`.
pub fn add_args(wt: &str, mutations: bool) -> Vec<String> {
    let mut a: Vec<String> = ["mcp", "add", SERVER_KEY, "--env", PROVIDER_ENV, "--exposure", EXPOSURE, "--", wt, "mcp"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    if mutations {
        a.push("--mutations".into());
    }
    a
}

pub fn status() -> Status {
    let path = config_path();
    let (entry, unreadable) = match std::fs::read_to_string(&path) {
        Ok(text) => match parse_entry(&text) {
            Ok(e) => (e, false),
            Err(_) => (None, true),
        },
        Err(_) => (None, false),
    };
    let pi_bin = crate::pimodels::pi_bin().map(|p| p.to_string_lossy().into_owned());
    let worktrees_bin = crate::profile::worktrees_bin().map(|p| p.to_string_lossy().into_owned());
    let state = state_of(entry.as_ref(), unreadable, pi_bin.is_some(), worktrees_bin.is_some());
    let command = pi_bin.as_ref().zip(worktrees_bin.as_ref()).map(|(pi, wt)| {
        let mut words = vec![crate::profile::shell_quote(pi)];
        words.extend(add_args(wt, true).iter().map(|w| crate::profile::shell_quote(w)));
        words.join(" ")
    });
    Status { state, pi_bin, worktrees_bin, entry, command, config_path: path.to_string_lossy().into_owned() }
}

fn run(pi: &str, args: &[String]) -> Result<(bool, String), String> {
    let mut cmd = std::process::Command::new(pi);
    cmd.args(args).env("PI_CODING_AGENT_DIR", crate::pi::agent_dir()).env("PI_OFFLINE", "1");
    let out = crate::proc::run_deadline(cmd, 60).map_err(|e| format!("pi mcp: {e}"))?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok((out.status.success(), text))
}

/// Refusals shared by install and uninstall: neither touches a file it cannot
/// read or an entry that is not ours.
fn guard(s: &Status) -> Result<(), String> {
    match s.state {
        "foreign" => Err(format!("a `{SERVER_KEY}` MCP server that is not ours is in {}; leaving it untouched", s.config_path)),
        "unreadable" => Err(format!("{} is not valid JSON; fix it (pi cannot read it either) and retry", s.config_path)),
        _ => Ok(()),
    }
}

pub fn install(mutations: bool) -> Result<Outcome, String> {
    let before = status();
    guard(&before)?;
    let pi = before.pi_bin.ok_or("pi is not installed")?;
    let wt = before.worktrees_bin.ok_or("worktrees is not on PATH")?;
    // `add` replaces an entry of the same name, so an existing one of ours
    // (read-only, stale, disabled) needs no `remove` first.
    let (ok, output) = run(&pi, &add_args(&wt, mutations))?;
    let after = status();
    let landed = after.entry.as_ref().is_some_and(|e| e.ours && e.enabled && e.mutations == mutations);
    Ok(Outcome { ok: ok && landed, output, status: after })
}

pub fn uninstall() -> Result<Outcome, String> {
    let before = status();
    guard(&before)?;
    if before.entry.is_none() {
        return Err("no worktrees MCP server is installed in pi".into());
    }
    let pi = before.pi_bin.ok_or("pi is not installed")?;
    let (ok, output) = run(&pi, &["mcp".into(), "remove".into(), SERVER_KEY.into()])?;
    let after = status();
    Ok(Outcome { ok: ok && after.entry.is_none(), output, status: after })
}

#[cfg(test)]
mod tests {
    use super::*;

    const OURS: &str = r#"{"mcpServers":{"worktrees":{"command":"/bin/sh","args":["mcp","--mutations"],"env":{"WORKTREES_MCP_PROVIDER":"pi"},"exposure":"direct"}}}"#;

    /// What `pi mcp add` wrote on 0.99.1 (§13.1), with the command pointed at
    /// an executable that exists everywhere; basename is what makes it ours.
    #[test]
    fn reads_the_entry_pi_writes() {
        let e = parse_entry(OURS).unwrap().unwrap();
        assert!(!e.ours, "/bin/sh is not worktrees");
        assert!(e.command_ok && e.mutations && e.enabled);
        assert_eq!(e.exposure.as_deref(), Some("direct"));
        let text = OURS.replace("/bin/sh", "/opt/x/worktrees");
        let e = parse_entry(&text).unwrap().unwrap();
        assert!(e.ours && !e.command_ok);
        assert_eq!(parse_entry(r#"{"mcpServers":{}}"#).unwrap(), None);
        assert_eq!(parse_entry("{}").unwrap(), None);
        assert!(parse_entry("{ nope").is_err());
        let off = text.replace(r#""exposure":"direct""#, r#""enabled":false"#);
        let e = parse_entry(&off).unwrap().unwrap();
        assert!(!e.enabled && e.exposure.is_none());
    }

    #[test]
    fn states_in_priority_order() {
        let e = |ours, ok, enabled, mutations| Entry {
            command: "worktrees".into(), args: vec![], ours, command_ok: ok, mutations, exposure: None, enabled,
        };
        assert_eq!(state_of(Some(&e(true, true, true, true)), false, true, true), "installed");
        assert_eq!(state_of(Some(&e(true, true, true, false)), false, true, true), "read-only");
        assert_eq!(state_of(Some(&e(true, true, false, true)), false, true, true), "disabled");
        assert_eq!(state_of(Some(&e(true, false, false, true)), false, true, true), "stale");
        assert_eq!(state_of(Some(&e(false, true, true, true)), false, true, true), "foreign");
        assert_eq!(state_of(None, true, true, true), "unreadable");
        assert_eq!(state_of(None, false, false, true), "pi-missing");
        assert_eq!(state_of(None, false, true, false), "cli-missing");
        assert_eq!(state_of(None, false, true, true), "absent");
        // An entry of ours still reads as installed with pi gone: the file is
        // the truth about the install, and pi-missing is about installing.
        assert_eq!(state_of(Some(&e(true, true, true, true)), false, false, true), "installed");
    }

    /// The decided install line (pi-harness Q9/Q10): user scope (no `-l`),
    /// direct exposure, the provider env, and the server's own flag.
    #[test]
    fn add_args_are_the_decided_line() {
        assert_eq!(
            add_args("/x/worktrees", true).join(" "),
            "mcp add worktrees --env WORKTREES_MCP_PROVIDER=pi --exposure direct -- /x/worktrees mcp --mutations"
        );
        assert!(!add_args("/x/worktrees", false).contains(&"--mutations".to_string()));
        assert!(!add_args("/x/worktrees", true).iter().any(|a| a == "-l" || a == "--local"));
    }
}
