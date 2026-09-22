//! Is worktrees' own MCP server wired into the user's `claude`, and if not, wire
//! it in — the engine behind `worktrees mcp --status|--install` and the app's
//! Home nudge / Settings → Claude panel.
//!
//! # The one rule: we READ claude's config, we never WRITE it
//!
//! User-scope MCP servers live in `~/.claude.json` under `mcpServers` (the same
//! key `profile::global_mcp_servers` already reads for the inherit toggle). That
//! file is claude's own state — a couple of hundred KB of onboarding flags,
//! caches, tips history and per-project conversation history — and Claude Code
//! rewrites it whole, continuously, from every running session. A
//! read-modify-write from here would lose whatever a live session wrote in
//! between, silently, and it would be SOMEONE ELSE'S data that went missing.
//!
//! So: detection parses the file (cheap, no subprocess), and installation shells
//! out to `claude mcp add`, which is claude's own supported entry point into its
//! own file. This is the same discipline the app already keeps for
//! `ui-state.json` — the side that does not own a whole-blob file never writes
//! into it — pointed one level outward.
//!
//! # Why detection does not ask `claude mcp list`
//!
//! Because it lies, through no fault of its own. `claude mcp list`/`get`
//! health-checks each server by LAUNCHING it in the current working directory,
//! and `worktrees mcp` legitimately serves nothing outside a git repo. Measured:
//!
//! ```text
//! $ cd /tmp      && claude mcp get worktrees   ✘ Failed to connect: CONNECTION_CLOSED
//! $ cd <a repo>  && claude mcp get worktrees   ✔ Connected
//! ```
//!
//! Both are the same, correct install. A status panel built on that verdict
//! would show a red ✘ on a working setup depending on where the app happened to
//! be standing. It also costs ~1.1s per invocation, which rules it out of
//! anything that runs more than once.
//!
//! # Why only USER scope is ever offered
//!
//! `claude mcp add` can write three scopes. We detect all of them and write
//! exactly one:
//!
//! - **user** (`~/.claude.json` `mcpServers`) — what we install. One server,
//!   every repo: the server pins its project from `CLAUDE_PROJECT_DIR`/cwd at
//!   startup (`mcp.rs`), so per-repo copies buy nothing.
//! - **local** (`~/.claude.json` `projects[<path>].mcpServers`) — detected so we
//!   do not nag someone who already has it, never written: it is invisible from
//!   every other checkout of the same repo.
//! - **project** (`<repo>/.mcp.json`) — detected, and **never** written. A
//!   committed `.mcp.json` is a cloned repository naming a program for claude to
//!   spawn, which is precisely the boundary ADR 0001 draws. That it would be
//!   convenient here is the argument the ADR exists to refuse.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The key we install under. Also the key we look for — but never the only
/// evidence we accept, see `Entry::ours`.
pub const SERVER_KEY: &str = "worktrees";

/// `claude mcp add` is a node program with a cold start; every shell-out here is
/// deadline-guarded so a wedged claude cannot freeze a Tauri command.
const ADD_TIMEOUT_SECS: u64 = 60;

// ── what we found ────────────────────────────────────────────────────────────

/// One `mcpServers` entry, judged.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Entry {
    /// The program claude would spawn.
    pub command: String,
    pub args: Vec<String>,
    /// `--mutations` is present, i.e. create/close/remove are exposed.
    pub mutations: bool,
    /// The command resolves to an executable file RIGHT NOW. False is the
    /// interesting case: a stanza left behind by a binary that moved fails at
    /// every session start and says nothing, and no amount of re-installing the
    /// CLI repairs it, because the stale path is in claude's config, not ours.
    pub command_ok: bool,
    /// This is OUR server, not somebody else's that happens to be keyed
    /// `worktrees`. Judged on what it RUNS (a `worktrees` binary with an `mcp`
    /// argument), never on the key — a foreign entry under our key must be
    /// reported and left alone, not overwritten.
    pub ours: bool,
}

impl Entry {
    /// Parse one `mcpServers` value.
    ///
    /// An http/sse entry has no `command` to judge and is FOREIGN by
    /// construction — reported, never overwritten. It deliberately does not
    /// return `None`: absent and "somebody else's server is sitting on our key"
    /// are opposite answers, and collapsing the second into the first makes the
    /// install button try to `claude mcp add` over a name that already exists,
    /// which fails with claude's own error after the user has clicked.
    ///
    /// `None` is reserved for a value that is not an object at all.
    fn parse(v: &serde_json::Value) -> Option<Entry> {
        if !v.is_object() {
            return None;
        }
        let Some(command) = v.get("command").and_then(|c| c.as_str()).map(str::to_string) else {
            let url = v.get("url").and_then(|u| u.as_str()).unwrap_or("(no command)");
            return Some(Entry {
                command: url.to_string(),
                args: Vec::new(),
                mutations: false,
                command_ok: false,
                ours: false,
            });
        };
        let args: Vec<String> = v
            .get("args")
            .and_then(|a| a.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let base = command.rsplit('/').next().unwrap_or(&command);
        let ours = base == "worktrees" && args.iter().any(|a| a == "mcp");
        Some(Entry {
            mutations: args.iter().any(|a| a == "--mutations"),
            command_ok: crate::profile::is_exec(Path::new(&command)),
            ours,
            command,
            args,
        })
    }
}

/// The key OUR server is registered under in a parsed `~/.claude.json`, if it is
/// there at all.
///
/// Judged by `Entry::ours` — what the stanza RUNS — so a user who
/// `claude mcp add`ed it as `wt` is found, and a foreign server squatting on
/// our key is not. Exposed because the nav drag needs the same answer from the
/// other direction: `mention::server_name_for` has to name the server in a
/// token, and a second implementation of "is this ours" would be free to drift
/// from the one the install button trusts.
pub fn our_key(user_claude_json: &serde_json::Value) -> Option<String> {
    user_claude_json
        .get("mcpServers")?
        .as_object()?
        .iter()
        .find(|(_, v)| Entry::parse(v).is_some_and(|e| e.ours))
        .map(|(k, _)| k.clone())
}

/// Where a server was found. Ordered by how much we are willing to do about it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    User,
    Local,
    Project,
    /// An AI profile's own `mcp.json` (`profile::materialize`), which is not in
    /// claude's config at all — we hand it to claude with `--mcp-config`.
    Profile,
}

/// The verdict. One word for the UI to switch on, so the copy for each case
/// lives in exactly one place on each side.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    /// The configured AI command is not claude, so none of this applies. Say
    /// nothing, anywhere.
    NotApplicable,
    /// Installed, ours, resolvable, mutating. The finished state.
    Installed,
    /// Installed and working, but read-only — an orchestrator cannot create or
    /// close a place with it.
    ReadOnly,
    /// Installed, ours, and the binary it names is GONE. Repair, not install.
    Stale,
    /// Something else is sitting under our key. Report it; touch nothing.
    Foreign,
    /// Not in user scope, but present somewhere that already covers this
    /// machine (local/project/profile). Nothing to do, and nothing to nag about.
    Elsewhere,
    /// Nothing anywhere, and there IS a `worktrees` binary to point at.
    Absent,
    /// Nothing anywhere, and no CLI to point a stanza at. The remedy is the
    /// installer, not this — the app's Updates section already owns that
    /// sentence, so this state exists to keep the MCP nudge quiet rather than to
    /// raise a second competing banner.
    CliMissing,
}

impl State {
    /// May the passive nudge appear for this state?
    ///
    /// **Only `Absent`.** `Stale` deliberately does not nudge — it warns, on its
    /// own line with its own dismissal, because a user who dismissed "install
    /// this" has not agreed to be silent about "the thing you installed is
    /// broken". That is the `init_dismissed` lesson (a dismissal keyed to one
    /// suggestion must not swallow a different one) applied here.
    pub fn nudgeable(self) -> bool {
        self == State::Absent
    }
    /// Is there an action the Settings panel should offer?
    pub fn actionable(self) -> bool {
        matches!(self, State::Absent | State::Stale | State::ReadOnly)
    }
}

/// Everything the CLI prints and the app renders. Every nullable field is an
/// explicit `Option` that is NOT skipped, following `model.rs` — a consumer sees
/// the same key set every time.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Status {
    pub state: State,
    /// The resolved `ai_cmd` (`config::resolve_ai_cmd`), so the UI can say WHY
    /// when the answer is `not-applicable`.
    pub ai_cmd: String,
    /// Absolute path of the `claude` binary, when one is on PATH. `None` means
    /// we cannot run the install for them and must show the command instead.
    pub claude_bin: Option<String>,
    /// Absolute path of the `worktrees` binary a fresh stanza would name.
    pub worktrees_bin: Option<String>,
    /// The user-scope entry, when there is one under our key.
    pub user: Option<Entry>,
    /// Other scopes that already carry the server, in the order listed above.
    /// Carried so the UI can explain an `elsewhere` verdict rather than just
    /// asserting it.
    pub found_in: Vec<Scope>,
    /// The exact command that would be run — shown next to a copy button, and
    /// the whole answer when `claude_bin` is `None`. Always present when a
    /// `worktrees` binary was found, even in states where we would not run it.
    pub command: Option<String>,
    /// `~/.claude.json`, for a reveal button.
    pub config_path: String,
}

// ── reading ──────────────────────────────────────────────────────────────────

/// `~/.claude.json`.
pub fn claude_json_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".claude.json")
}

/// Parsed `~/.claude.json`, or `null` if it is missing or malformed.
///
/// Lenient on purpose, exactly like `profile::read_lenient`: a claude config we
/// cannot parse means "we do not know", and the honest answer to that is to
/// offer the install and let `claude mcp add` — which CAN parse it — be the one
/// that refuses.
fn read_claude_json() -> serde_json::Value {
    std::fs::read_to_string(claude_json_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::Value::Null)
}

/// Judge one `mcpServers` map for our key.
fn entry_in(map: Option<&serde_json::Value>) -> Option<Entry> {
    map?.get("mcpServers")?.get(SERVER_KEY).and_then(Entry::parse)
}

/// Does the profile that would actually RUN already expose the server?
///
/// A profiled launch gets its own `mcp.json` (`profile.rs`), so such a user is
/// covered without ever touching claude's config — and, since a profile that
/// does not inherit the global adds `--strict-mcp-config`, a user-scope install
/// would not even reach that pane.
///
/// Deliberately the EFFECTIVE profile, not "any profile that has the box
/// ticked". A profile the user made once and never assigned says nothing about
/// the sessions they actually launch, and counting it would silence the offer
/// for someone who is not covered at all. With no repo in hand the question
/// collapses to the global default, which is the only profile that could apply
/// everywhere.
fn effective_profile_exposes_it(repo: Option<&str>) -> bool {
    let ps = crate::profile::read_lenient();
    let known: Vec<String> = ps.profiles.keys().cloned().collect();
    let id = crate::profile::resolve_profile_id_from(
        std::env::var("WORKTREES_PROFILE").ok().as_deref(),
        repo.and_then(|r| ps.assignments.get(r)).map(|s| s.as_str()),
        ps.default_id.as_deref(),
        &known,
    );
    id.and_then(|id| ps.profiles.get(&id)).is_some_and(|p| p.worktrees_mcp)
}

/// The full verdict. `repo` is the project in focus, when there is one — it is
/// what makes the local and project scopes checkable; pass `None` from a
/// context with no repo (the app's Home screen) and those two are simply not
/// consulted.
pub fn status(repo: Option<&str>) -> Status {
    let ai_cmd = crate::config::resolve_ai_cmd(None);
    let wt = crate::profile::worktrees_bin().map(|p| p.to_string_lossy().into_owned());
    let command = wt.as_ref().map(|b| command_line(b, true));
    let mut st = Status {
        state: State::Absent,
        claude_bin: crate::profile::bin_on_path("claude").map(|p| p.to_string_lossy().into_owned()),
        worktrees_bin: wt,
        user: None,
        found_in: Vec::new(),
        command,
        config_path: claude_json_path().to_string_lossy().into_owned(),
        ai_cmd: ai_cmd.clone(),
    };

    let cj = read_claude_json();
    st.user = entry_in(Some(&cj));
    if st.user.is_some() {
        st.found_in.push(Scope::User);
    }
    if let Some(repo) = repo {
        if entry_in(cj.get("projects").and_then(|p| p.get(repo))).is_some() {
            st.found_in.push(Scope::Local);
        }
        let dot = Path::new(repo).join(".mcp.json");
        let parsed = std::fs::read_to_string(&dot).ok().and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
        if entry_in(parsed.as_ref()).is_some() {
            st.found_in.push(Scope::Project);
        }
    }
    if effective_profile_exposes_it(repo) {
        st.found_in.push(Scope::Profile);
    }

    // Order matters, and it is not the obvious one: a BROKEN user entry must
    // out-rank "no CLI installed", because re-installing the CLI does not fix a
    // stale path in claude's config and telling that user to go to Updates sends
    // them somewhere that cannot help.
    st.state = match &st.user {
        Some(e) if !e.ours => State::Foreign,
        Some(e) if !e.command_ok => State::Stale,
        Some(e) if !e.mutations => State::ReadOnly,
        Some(_) => State::Installed,
        None if !st.found_in.is_empty() => State::Elsewhere,
        None if st.worktrees_bin.is_none() => State::CliMissing,
        None => State::Absent,
    };
    st
}

/// The `claude mcp add` line, rendered for a human to read or paste. This is the
/// single source of the command: the installer below runs the same argv, so the
/// text on screen cannot drift from what the button does.
pub fn command_line(worktrees_bin: &str, mutations: bool) -> String {
    let tail = if mutations { " --mutations" } else { "" };
    format!("claude mcp add -s user {SERVER_KEY} -- {worktrees_bin} mcp{tail}")
}

// ── writing (via claude, never directly) ─────────────────────────────────────

/// What an install attempt did. A `CmdResult`-shaped answer rather than a bare
/// `Result` because the output is worth showing either way — `claude mcp add`
/// explains its own refusals better than we could paraphrase them.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Outcome {
    pub ok: bool,
    /// Combined stdout+stderr of every command run, in order.
    pub output: String,
    /// The status as re-read AFTER the attempt. The real success signal: the
    /// same shape as `update_cli`'s "the resolved CLI now reports the new
    /// version" check, and for the same reason — an installer that exits 0
    /// without having changed anything is the failure that looks like success.
    pub status: Status,
}

/// Install (or repair, or upgrade) the user-scope server.
///
/// Repair and upgrade are the same act as install: `claude mcp add` refuses a
/// name that already exists, so an existing entry is REMOVED first. That is safe
/// for exactly the states this is reachable from — `Absent` has nothing to
/// remove, and `Stale`/`ReadOnly` have an entry we wrote and are replacing.
/// `Foreign` is refused here rather than in the UI, because this is also the
/// CLI's entry point: we do not delete a server we did not write.
pub fn install(repo: Option<&str>, mutations: bool) -> Result<Outcome, String> {
    let before = status(repo);
    match before.state {
        State::NotApplicable => {
            return Err(format!(
                "the AI command is `{}`, not claude — nothing to install into",
                before.ai_cmd
            ))
        }
        State::Foreign => {
            return Err(format!(
                "an MCP server named `{SERVER_KEY}` already exists and is not ours \
                 (it runs `{}`). Remove or rename it first — this will not overwrite it.",
                before.user.as_ref().map(|e| e.command.as_str()).unwrap_or("?")
            ))
        }
        _ => {}
    }
    let Some(claude) = before.claude_bin.clone() else {
        return Err("`claude` is not on PATH — run the command shown by hand".into());
    };
    let Some(wt) = before.worktrees_bin.clone() else {
        return Err("no `worktrees` binary found to point the server at — install the CLI first".into());
    };

    let mut output = String::new();
    // Replace rather than merge: `claude mcp add` errors on an existing name,
    // and the removal is scoped to `-s user`, so a local/project entry the user
    // made themselves is untouched.
    if before.user.is_some() {
        let out = run(&claude, &["mcp", "remove", SERVER_KEY, "-s", "user"])?;
        output.push_str(&out.1);
    }
    let mut args: Vec<&str> = vec!["mcp", "add", "-s", "user", SERVER_KEY, "--", &wt, "mcp"];
    if mutations {
        args.push("--mutations");
    }
    let (ok, text) = run(&claude, &args)?;
    output.push_str(&text);

    // Trust the re-read, not the exit code.
    let after = status(repo);
    let landed = matches!(after.state, State::Installed | State::ReadOnly)
        && after.user.as_ref().map(|e| e.mutations) == Some(mutations);
    if ok && !landed {
        output.push_str(
            "\n! `claude mcp add` exited 0 but the server is not in ~/.claude.json as expected.",
        );
    }
    Ok(Outcome { ok: ok && landed, output, status: after })
}

/// Remove the user-scope server. Offered in Settings so the panel is not a
/// one-way door — and refused for a foreign entry for the same reason `install`
/// refuses one.
pub fn uninstall(repo: Option<&str>) -> Result<Outcome, String> {
    let before = status(repo);
    if before.user.is_none() {
        return Err("no user-scope `worktrees` MCP server to remove".into());
    }
    if before.state == State::Foreign {
        return Err(format!(
            "the MCP server named `{SERVER_KEY}` is not ours — remove it with claude directly"
        ));
    }
    let Some(claude) = before.claude_bin.clone() else {
        return Err("`claude` is not on PATH — run `claude mcp remove worktrees -s user` by hand".into());
    };
    let (ok, output) = run(&claude, &["mcp", "remove", SERVER_KEY, "-s", "user"])?;
    let after = status(repo);
    Ok(Outcome { ok: ok && after.user.is_none(), output, status: after })
}

/// One deadline-guarded shell-out. Returns (success, stdout+stderr).
///
/// `claude` is a node program with a cold start measured at ~1.1s even for a
/// read; an unguarded wait here is a Tauri command that never returns. The pipes
/// are DRAINED on their own threads rather than after the wait — a child that
/// fills a 64K pipe buffer blocks forever on the write, so a `wait()`-then-read
/// shape deadlocks on exactly the chatty failure we most want to report.
fn run(claude: &str, args: &[&str]) -> Result<(bool, String), String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let ctx = || format!("running `claude {}`", args.join(" "));
    let mut child = Command::new(claude)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{}: {e}", ctx()))?;
    let drain = |mut s: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut b = Vec::new();
            if let Some(s) = s.as_mut() {
                let _ = s.read_to_end(&mut b);
            }
            b
        })
    };
    let ho = drain(child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
    let he = drain(child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(ADD_TIMEOUT_SECS);
    let status = loop {
        match child.try_wait().map_err(|e| format!("{}: {e}", ctx()))? {
            Some(st) => break st,
            None if std::time::Instant::now() > deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{}: timed out after {ADD_TIMEOUT_SECS}s", ctx()));
            }
            None => std::thread::sleep(std::time::Duration::from_millis(80)),
        }
    };
    let mut text = String::from_utf8_lossy(&ho.join().unwrap_or_default()).to_string();
    text.push_str(&String::from_utf8_lossy(&he.join().unwrap_or_default()));
    if !text.ends_with('\n') {
        text.push('\n');
    }
    Ok((status.success(), text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ent(cmd: &str, args: &[&str]) -> Entry {
        Entry::parse(&serde_json::json!({ "command": cmd, "args": args })).unwrap()
    }

    #[test]
    fn ours_is_judged_on_what_it_runs_not_on_the_key() {
        // The key is `worktrees` in every one of these; only the first two are
        // ours. A foreign entry under our key is the case that must never be
        // silently overwritten.
        assert!(ent("/usr/local/bin/worktrees", &["mcp"]).ours);
        assert!(ent("/Users/x/.local/bin/worktrees", &["mcp", "--mutations"]).ours);
        assert!(!ent("/usr/local/bin/worktrees", &["serve"]).ours); // right binary, wrong verb
        assert!(!ent("npx", &["some-other-mcp"]).ours);
        // An http entry carries no `command` at all, and must still be seen —
        // "somebody else holds our key" is the opposite of "nothing is there".
        let http = Entry::parse(&serde_json::json!({ "type": "http", "url": "http://x/mcp" })).unwrap();
        assert!(!http.ours);
        assert_eq!(http.command, "http://x/mcp");
        // Only a non-object is nothing.
        assert!(Entry::parse(&serde_json::json!("nonsense")).is_none());
    }

    #[test]
    fn mutations_is_read_off_the_args() {
        assert!(!ent("/bin/worktrees", &["mcp"]).mutations);
        assert!(ent("/bin/worktrees", &["mcp", "--mutations"]).mutations);
    }

    #[test]
    fn command_ok_is_a_live_filesystem_fact() {
        // The whole point of the `stale` state: the stanza parses fine and names
        // a path that is not there.
        assert!(!ent("/nonexistent/worktrees", &["mcp"]).command_ok);
        assert!(ent("/bin/sh", &["mcp"]).command_ok || cfg!(not(unix)));
    }

    #[test]
    fn only_absent_nudges() {
        // Stale warns on its own line and must not be silenced by a dismissal of
        // the install nudge — see `State::nudgeable`.
        assert!(State::Absent.nudgeable());
        for s in [State::Stale, State::ReadOnly, State::Installed, State::Foreign, State::Elsewhere, State::CliMissing, State::NotApplicable] {
            assert!(!s.nudgeable(), "{s:?} must not nudge");
        }
        assert!(State::Stale.actionable());
        assert!(State::ReadOnly.actionable());
        assert!(!State::Installed.actionable());
        assert!(!State::CliMissing.actionable());
    }

    #[test]
    fn the_rendered_command_is_the_one_we_would_run() {
        // The text on screen and the argv in `install` must not drift; this pins
        // the rendering, and `install` builds the same list.
        assert_eq!(
            command_line("/Users/x/.local/bin/worktrees", true),
            "claude mcp add -s user worktrees -- /Users/x/.local/bin/worktrees mcp --mutations"
        );
        assert_eq!(
            command_line("/opt/homebrew/bin/worktrees", false),
            "claude mcp add -s user worktrees -- /opt/homebrew/bin/worktrees mcp"
        );
    }

    #[test]
    fn states_serialize_as_kebab_slugs() {
        for (s, w) in [
            (State::NotApplicable, "not-applicable"),
            (State::Installed, "installed"),
            (State::ReadOnly, "read-only"),
            (State::Stale, "stale"),
            (State::Foreign, "foreign"),
            (State::Elsewhere, "elsewhere"),
            (State::Absent, "absent"),
            (State::CliMissing, "cli-missing"),
        ] {
            assert_eq!(serde_json::to_string(&s).unwrap(), format!("\"{w}\""));
        }
        assert_eq!(serde_json::to_string(&Scope::User).unwrap(), "\"user\"");
    }
}
