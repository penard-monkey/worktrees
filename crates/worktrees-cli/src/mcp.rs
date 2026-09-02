//! `worktrees mcp` — an MCP server over stdio, so a Claude session running in a
//! worktree can drive the worktree layer as tools instead of reinventing it with
//! raw git.
//!
//! # Why this is hand-rolled
//!
//! The obvious move was the official `rmcp` crate. Measured, on this branch:
//! the CLI's dependency tree is **32 crates**; adding `rmcp` (with `server`,
//! `transport-io`, `macros`, `schemars`) resolves to **124**, and drags `tokio`
//! into a binary that is synchronous on purpose. rmcp also shipped a breaking
//! 3.0 in mid-2026, i.e. roughly a migration a year, and the CLI ships on four
//! release targets.
//!
//! What it would buy is a newline-delimited JSON-RPC 2.0 loop. `serde_json` is
//! already a dependency; the protocol fits in this file. That trade is the same
//! one the workspace already makes explicitly elsewhere — `worktrees-core`'s
//! Cargo.toml refuses `git2`/`gix` and `clap` for smaller reasons than this.
//!
//! The wire format was read out of rmcp's own source rather than guessed:
//! newline-delimited JSON, `initialize` → `{protocolVersion, capabilities,
//! serverInfo}`, `tools/list` → `{tools:[{name, description, inputSchema,
//! annotations}]}`, `tools/call` → `{content:[{type:"text",text}], isError}`.
//!
//! # Safety shape
//!
//! - **The repo is pinned at startup.** Every tool operates on the project the
//!   server was launched in. No tool takes a repo path, so a model cannot walk
//!   the server into another checkout.
//! - **Mutating tools are opt-in.** Without `--mutations` the server exposes
//!   reads and note/pin/lifecycle metadata only. The materializer writes the
//!   flag into a profile's `mcp.json`, so enabling it is a profile decision the
//!   user makes once, not something a session can grant itself.
//! - **Destructive tools additionally require `confirm: true`** in the call
//!   arguments, and say so when it is missing. `CaptureUi::can_confirm()` is
//!   false, so core's own guards report `EXIT_NEEDS_CONFIRM` rather than reading
//!   silence as consent.
//! - **stdout carries protocol only.** Anything human-facing goes to stderr, or
//!   the transport is corrupt.

use std::io::{BufRead, Read, Write};

use worktrees_core::{ops, store, ui::CaptureUi, Project};

/// Pick the protocol version to answer `initialize` with: echo what the client
/// asked for when we know it, else our newest. Free function so the test
/// exercises THIS, rather than a copy of the rule.
fn negotiate(asked: &str) -> &str {
    if SUPPORTED.contains(&asked) {
        asked
    } else {
        LATEST
    }
}

/// Versions this server understands. We echo the client's choice when we know
/// it, else answer with our newest — the handshake rule from the spec.
///
/// Deliberately does NOT include 2024-11-05 or 2025-03-26. Those permit JSON-RPC
/// BATCHES (an array of messages), which this reader does not parse — a batch
/// would be classified as a notification and dropped, hanging a conforming
/// client forever. Batching was removed in 2025-06-18, so advertising only
/// 2025-06-18 and later makes the reader's shape and the negotiated contract
/// agree.
const SUPPORTED: &[&str] = &["2025-11-25", "2025-06-18"];
const LATEST: &str = "2025-11-25";

const EXIT_NEEDS_CONFIRM: i32 = 3;

struct Server {
    project: Project,
    mutations: bool,
}

pub fn cmd_mcp(args: &[String]) -> i32 {
    let mutations = args.iter().any(|a| a == "--mutations");
    // Pin the project once, here. `CLAUDE_PROJECT_DIR` is what claude exports for
    // the session's root; fall back to the process cwd.
    let root = std::env::var("CLAUDE_PROJECT_DIR")
        .ok()
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::current_dir().ok());
    let Some(root) = root else {
        eprintln!("worktrees mcp: cannot determine a working directory");
        return 1;
    };
    let project = match Project::discover(&root) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("worktrees mcp: {}", e.msg);
            return e.code;
        }
    };
    let mut server = Server { project, mutations };

    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    // Bounded: `lines()` grows a String until it finds a newline, so a client
    // that never sends one would drive allocation until the process dies.
    const MAX_LINE: u64 = 8 * 1024 * 1024;
    for line in std::io::BufReader::new(stdin.lock().take(MAX_LINE)).lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("worktrees mcp: stdin: {e}");
                return 1;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let Some(resp) = server.handle_line(&line) else {
            continue; // a notification: no reply, by protocol
        };
        if writeln!(out, "{resp}").is_err() || out.flush().is_err() {
            return 0; // client hung up
        }
    }
    0
}

impl Server {
    /// One JSON-RPC message in, at most one out.
    fn handle_line(&mut self, line: &str) -> Option<String> {
        let msg: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            // Parse errors get an id-less error object; there is no id to echo.
            Err(e) => return Some(err_obj(serde_json::Value::Null, -32700, &format!("parse error: {e}"))),
        };
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(|m| m.as_str());
        let params = msg.get("params").cloned().unwrap_or(serde_json::json!({}));

        // No id = notification. Never answer one, even to complain.
        let Some(id) = id else { return None };

        // A request with no `method` is malformed, which is -32600 — distinct
        // from a method we simply do not implement.
        let Some(method) = method else {
            return Some(err_obj(id, -32600, "invalid request: no method"));
        };
        let result = match method {
            "initialize" => Ok(self.initialize(&params)),
            "ping" => Ok(serde_json::json!({})),
            "tools/list" => Ok(serde_json::json!({ "tools": self.tools() })),
            "tools/call" => self.call(&params),
            other => {
                return Some(err_obj(id, -32601, &format!("method not found: {other}")));
            }
        };
        Some(match result {
            Ok(r) => serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": r }).to_string(),
            Err(e) => err_obj(id, -32602, &e),
        })
    }

    fn initialize(&self, params: &serde_json::Value) -> serde_json::Value {
        let asked = params.get("protocolVersion").and_then(|v| v.as_str()).unwrap_or(LATEST);
        let version = negotiate(asked);
        serde_json::json!({
            "protocolVersion": version,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "worktrees", "version": env!("CARGO_PKG_VERSION") },
            "instructions": format!(
                "Worktree management for the repository at {}. One git worktree per branch, \
                 one tmux session per worktree. Use list_places to see the current state. \
                 {}",
                self.project.main_root,
                if self.mutations {
                    "Mutating tools are enabled; destructive ones need confirm: true."
                } else {
                    "This server is read-only apart from note/pin/lifecycle metadata."
                }
            ),
        })
    }

    fn tools(&self) -> Vec<serde_json::Value> {
        let mut t = vec![
            tool(
                "list_places",
                "List every place (worktree) in this repository with its branch, git state, \
                 tmux session status and declared lifecycle. Start here.",
                serde_json::json!({ "type": "object", "properties": {}, "additionalProperties": false }),
                true,
                false,
            ),
            tool(
                "place_status",
                "Details for one place: branch, dirty state, divergence from the base ref, \
                 tmux session, note and lifecycle.",
                serde_json::json!({
                    "type": "object",
                    "properties": { "slug": { "type": "string", "description": "Place slug, or (main)." } },
                    "required": ["slug"],
                    "additionalProperties": false
                }),
                true,
                false,
            ),
            tool(
                "doctor",
                "Report drift between .worktrees.toml and what is actually on disk for this repo.",
                serde_json::json!({ "type": "object", "properties": {}, "additionalProperties": false }),
                true,
                false,
            ),
            tool(
                "set_note",
                "Set (or clear) the human note on a place. Metadata only — nothing on disk changes.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "slug": { "type": "string" },
                        "note": { "type": "string", "description": "Empty string clears it." }
                    },
                    "required": ["slug", "note"],
                    "additionalProperties": false
                }),
                false,
                false,
            ),
            tool(
                "set_pin",
                "Pin or unpin a place so it sorts to the top. Metadata only.",
                serde_json::json!({
                    "type": "object",
                    "properties": { "slug": { "type": "string" }, "pinned": { "type": "boolean" } },
                    "required": ["slug", "pinned"],
                    "additionalProperties": false
                }),
                false,
                false,
            ),
            tool(
                "set_lifecycle",
                "Set a place's declared lifecycle: closed, saved, archived or abandoned. \
                 Metadata only — this does not touch the worktree or its branch.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "slug": { "type": "string" },
                        "lifecycle": { "type": "string", "enum": ["closed", "saved", "archived", "abandoned"] }
                    },
                    "required": ["slug", "lifecycle"],
                    "additionalProperties": false
                }),
                false,
                false,
            ),
        ];
        if self.mutations {
            t.push(tool(
                "create_worktree",
                "Create a worktree for a branch (creating the branch off base if needed) and \
                 open its tmux session: a single pane running the AI, named after the \
                 session so other sessions can message it. Pass `brief` to hand the agent \
                 its task: it is written to .planning/brief.md in the worktree and claude \
                 opens on it.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "branch": { "type": "string" },
                        "base": { "type": "string", "description": "Base ref for a new branch. Optional." },
                        "brief": { "type": "string", "description": "The agent's task, as markdown. Written to .planning/brief.md; claude is launched on it. Optional." },
                        "spare": { "type": "boolean", "description": "Also open a spare shell pane (where deps install). Default false." }
                    },
                    "required": ["branch"],
                    "additionalProperties": false
                }),
                false,
                false,
            ));
            t.push(tool(
                "close_session",
                "End a place's tmux session. The worktree, branch and files all stay.",
                serde_json::json!({
                    "type": "object",
                    "properties": { "slug": { "type": "string" } },
                    "required": ["slug"],
                    "additionalProperties": false
                }),
                false,
                false,
            ));
            t.push(tool(
                "remove_worktree",
                "DESTRUCTIVE. Remove a worktree directory and its tmux session. Requires \
                 confirm: true. Uncommitted work in that worktree is lost.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "slug": { "type": "string" },
                        "confirm": { "type": "boolean", "description": "Must be true. Refused otherwise." }
                    },
                    "required": ["slug", "confirm"],
                    "additionalProperties": false
                }),
                false,
                true,
            ));
        }
        t
    }

    fn call(&mut self, params: &serde_json::Value) -> Result<serde_json::Value, String> {
        // These two ARE protocol errors — the request is malformed, as opposed to
        // a tool that ran and failed (which is `isError: true` in a result).
        let Some(name) = params.get("name").and_then(|v| v.as_str()) else {
            return Err("params.name is required and must be a string".into());
        };
        let a = match params.get("arguments") {
            None | Some(serde_json::Value::Null) => serde_json::json!({}),
            Some(v) if v.is_object() => v.clone(),
            Some(_) => return Err("params.arguments must be an object".into()),
        };
        let s = |k: &str| a.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();

        // A tool the server did not advertise must not be callable, or
        // `--mutations` would be advisory rather than a gate.
        let Some(advertised) = self.tools().into_iter().find(|t| t["name"] == name) else {
            let hint = if !self.mutations {
                " (this server was started without --mutations)"
            } else {
                ""
            };
            return Ok(text_err(&format!("unknown tool: {name}{hint}")));
        };

        // Guard A on this surface. The CLI refuses every mutating command inside
        // a tree that arrived on a sync hub (one choke point in main.rs); this
        // server never passes that point, and a model has no way to tell that the
        // checkout it was launched in is another machine's mirror — where a
        // `worktree prune` can unregister the WRONG repo's worktrees and every
        // write is overwritten by the next pull.
        //
        // Keyed off the tool's own `readOnlyHint` rather than a second list of
        // names: a list would drift, and the annotation is already the answer to
        // "does this write?". Reads stay allowed — they are how you find out what
        // the tree is.
        if advertised["annotations"]["readOnlyHint"] != serde_json::json!(true) {
            if let Some(msg) =
                worktrees_core::sync::hub_copy_refusal(std::path::Path::new(&self.project.main_root))
            {
                return Ok(text_err(&msg));
            }
        }

        match name {
            "list_places" => {
                let ls = self.project.ls();
                Ok(text_ok(&serde_json::to_string_pretty(&ls).unwrap_or_default()))
            }
            "place_status" => {
                let slug = match safe_arg(&s("slug"), "slug") {
                    Ok(v) => v,
                    Err(e) => return Ok(text_err(&e)),
                };
                let ls = self.project.ls();
                match ls.places.iter().find(|p| p.slug == slug) {
                    Some(p) => {
                        // The place, plus WHO is working in it: the claude
                        // session(s) whose cwd is this worktree, most active
                        // first, each with the name another session messages
                        // it by. `agent_state` is the one-word answer to "is
                        // anyone on this?" — `none` when the pane has no claude.
                        let mut v = serde_json::to_value(p).unwrap_or_default();
                        let agents = worktrees_core::agent::agents_at(&worktrees_core::agent::live_probes(), &p.path);
                        v["agent_state"] = serde_json::json!(agents.first().map(|a| a.state.as_str()).unwrap_or("none"));
                        v["agents"] = serde_json::json!(agents);
                        Ok(text_ok(&serde_json::to_string_pretty(&v).unwrap_or_default()))
                    }
                    None => Ok(text_err(&format!("no such place: {slug}"))),
                }
            }
            "doctor" => Ok(self.run_op(|p, ui| ops::cmd_doctor(p, ui, &[]))),
            "set_note" => {
                let slug = match self.known_slug(&s("slug")) {
                    Ok(v) => v,
                    Err(e) => return Ok(text_err(&e)),
                };
                let note = s("note");
                self.meta(&slug, |d| {
                    d.note = if note.is_empty() { None } else { Some(note.clone()) }
                })
            }
            "set_pin" => {
                let slug = match self.known_slug(&s("slug")) {
                    Ok(v) => v,
                    Err(e) => return Ok(text_err(&e)),
                };
                // Strict: a non-bool used to mean "unpin", so a typo silently did
                // the opposite of what was asked.
                let Some(pinned) = a.get("pinned").and_then(|v| v.as_bool()) else {
                    return Ok(text_err("pinned must be true or false"));
                };
                self.meta(&slug, |d| d.pinned = Some(pinned))
            }
            "set_lifecycle" => {
                let slug = match self.known_slug(&s("slug")) {
                    Ok(v) => v,
                    Err(e) => return Ok(text_err(&e)),
                };
                let life = s("lifecycle");
                if !["closed", "saved", "archived", "abandoned"].contains(&life.as_str()) {
                    return Ok(text_err(&format!("invalid lifecycle: {life}")));
                }
                self.meta(&slug, |d| d.lifecycle = Some(life.clone()))
            }
            "create_worktree" => {
                let branch = match safe_arg(&s("branch"), "branch") {
                    Ok(v) => v,
                    Err(e) => return Ok(text_err(&e)),
                };
                let raw_base = s("base");
                let mut args = vec![branch, "--no-attach".to_string()];
                if !raw_base.trim().is_empty() {
                    match safe_arg(&raw_base, "base") {
                        Ok(b) => args.insert(1, b),
                        Err(e) => return Ok(text_err(&e)),
                    }
                }
                // Single pane unless asked: an agent's place has no one at the
                // keyboard to use a spare shell, and the pane it would take is
                // width claude reads by. Typed strictly, like set_pin's bool.
                match a.get("spare") {
                    None | Some(serde_json::Value::Null) | Some(serde_json::Value::Bool(false)) => {
                        args.push("--no-spare".to_string())
                    }
                    Some(serde_json::Value::Bool(true)) => {}
                    Some(_) => return Ok(text_err("spare must be true or false")),
                }
                // The brief is free text and never a flag: it rides as the value
                // of `--brief`, which core's parser consumes whole — so it needs
                // no `safe_arg`, only to be a string.
                match a.get("brief") {
                    None | Some(serde_json::Value::Null) => {}
                    Some(serde_json::Value::String(b)) if b.trim().is_empty() => {
                        return Ok(text_err("brief is empty — leave it out, or say what the agent is to do"))
                    }
                    Some(serde_json::Value::String(b)) => {
                        args.push("--brief".to_string());
                        args.push(b.clone());
                    }
                    Some(_) => return Ok(text_err("brief must be a string")),
                }
                Ok(self.run_op(move |p, ui| ops::cmd_new(p, ui, &args)))
            }
            "close_session" => {
                let slug = match self.known_slug(&s("slug")) {
                    Ok(v) => v,
                    Err(e) => return Ok(text_err(&e)),
                };
                let args = vec![slug];
                Ok(self.run_op(move |p, ui| ops::cmd_close(p, ui, &args)))
            }
            "remove_worktree" => {
                if a.get("confirm").and_then(|v| v.as_bool()) != Some(true) {
                    // Stated as a refusal with the reason, not a silent no-op:
                    // the model has to be told what it failed to provide.
                    return Ok(text_err(
                        "remove_worktree is destructive and needs confirm: true. \
                         Ask the user before retrying — uncommitted work is lost.",
                    ));
                }
                let slug = match self.known_slug(&s("slug")) {
                    Ok(v) => v,
                    Err(e) => return Ok(text_err(&e)),
                };
                let args = vec![slug, "-y".to_string()];
                Ok(self.run_op(move |p, ui| ops::cmd_rm(p, ui, &args)))
            }
            other => Ok(text_err(&format!("unknown tool: {other}"))),
        }
    }

    /// A slug that is flag-safe AND names a place that actually exists.
    ///
    /// The existence check is not pedantry: `store::edit` creates the entry it is
    /// given, so a typo used to leave a ghost record in the declared-state file
    /// for a place that never existed.
    fn known_slug(&self, slug: &str) -> Result<String, String> {
        let slug = safe_arg(slug, "slug")?;
        if self.project.ls().places.iter().any(|p| p.slug == slug) {
            Ok(slug)
        } else {
            Err(format!("no such place: {slug}"))
        }
    }

    fn meta<F: FnOnce(&mut store::Declared)>(&self, slug: &str, f: F) -> Result<serde_json::Value, String> {
        if slug.is_empty() {
            return Ok(text_err("slug is required"));
        }
        match store::edit(&self.project.main_root, slug, f) {
            Ok(()) => Ok(text_ok("ok")),
            Err(e) => Ok(text_err(&e)),
        }
    }

    /// Run a core op with a capturing Ui and report what it said.
    ///
    /// `CaptureUi::can_confirm()` is false, so an op that would have prompted
    /// returns EXIT_NEEDS_CONFIRM instead of treating an unanswered prompt as a
    /// decline — that distinction is what keeps a guarded operation guarded.
    fn run_op<F: FnOnce(&Project, &mut CaptureUi) -> i32>(&self, f: F) -> serde_json::Value {
        let mut ui = CaptureUi::default();
        let rc = f(&self.project, &mut ui);
        let body = ui.lines.join("\n");
        if rc == 0 {
            text_ok(if body.is_empty() { "ok" } else { &body })
        } else if rc == EXIT_NEEDS_CONFIRM {
            text_err(&format!(
                "{body}\n\nThis operation stopped to ask for confirmation. Relay the question to \
                 the user and retry only with their answer."
            ))
        } else {
            text_err(&format!("{body}\n(exit {rc})"))
        }
    }
}

/// Reject a model-supplied value that a core arg parser would read as a FLAG.
///
/// This is the boundary that matters. `ops::cmd_new` and friends parse their own
/// argv: anything matching a flag pattern is consumed AS a flag and never fills
/// a positional slot. So `base: "--ai=touch /tmp/x"` does not create a worktree
/// based on a branch called that — it sets the AI command, which
/// `ops::launch` interpolates into `sh -ic '<ai_cmd>; …'`. That is a shell
/// command line, so a tool advertised as "create a worktree" became arbitrary
/// code execution.
///
/// It has to be caught HERE: this is the only layer that knows these strings
/// came from a model rather than from a person typing a command. `--` would be
/// the tidier fix but core's parsers have no end-of-options case today.
fn safe_arg(v: &str, what: &str) -> Result<String, String> {
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

fn tool(
    name: &str,
    description: &str,
    input_schema: serde_json::Value,
    read_only: bool,
    destructive: bool,
) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
        // Hints, not enforcement — the gate is `tools()` not advertising a tool
        // the server was not started to expose. They exist so a client can warn.
        "annotations": {
            "readOnlyHint": read_only,
            "destructiveHint": destructive,
            "idempotentHint": !destructive && !read_only,
            "openWorldHint": false
        }
    })
}

fn text_ok(body: &str) -> serde_json::Value {
    serde_json::json!({ "content": [{ "type": "text", "text": body }], "isError": false })
}

fn text_err(body: &str) -> serde_json::Value {
    serde_json::json!({ "content": [{ "type": "text", "text": body }], "isError": true })
}

fn err_obj(id: serde_json::Value, code: i64, message: &str) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The protocol-shaping helpers are testable without a repo; the tool bodies
    /// need one, and get exercised end to end from bats instead.
    #[test]
    fn version_negotiation_echoes_a_known_version_else_falls_back() {
        // Calls the real function, not a copy of the rule — the previous version
        // of this test re-implemented negotiation locally and would have stayed
        // green if `initialize` stopped consulting SUPPORTED at all.
        assert_eq!(negotiate("2025-06-18"), "2025-06-18", "a known version is echoed back");
        assert_eq!(negotiate("1999-01-01"), LATEST, "an unknown one falls back to ours");
        assert_eq!(negotiate("2026-07-28"), LATEST, "a NEWER one falls back too");
        assert!(SUPPORTED.contains(&LATEST));
    }

    #[test]
    fn batching_versions_are_not_advertised() {
        // 2024-11-05 and 2025-03-26 permit JSON-RPC batches, which this reader
        // does not parse: a batch is an array, `get("id")` returns None, and the
        // whole thing would be dropped as a notification — hanging the client.
        assert!(!SUPPORTED.contains(&"2024-11-05"));
        assert!(!SUPPORTED.contains(&"2025-03-26"));
    }

    #[test]
    fn a_model_supplied_value_cannot_become_a_command_line_flag() {
        // THE finding this boundary exists for: core's arg parsers consume
        // anything flag-shaped AS a flag, and `--ai=<cmd>` reaches
        // `ops::launch`, which interpolates it into `sh -ic '<cmd>; …'`. A tool
        // advertised as "create a worktree" was arbitrary code execution.
        for bad in ["--ai=touch /tmp/x", "-r", "--name=..", "--no-tmux"] {
            assert!(safe_arg(bad, "base").is_err(), "{bad:?} must be refused");
        }
        for bad in ["", "   ", "a\nb", "x..y"] {
            assert!(safe_arg(bad, "branch").is_err(), "{bad:?} must be refused");
        }
        assert_eq!(safe_arg(" feat-x ", "branch").unwrap(), "feat-x");
    }

    #[test]
    fn results_carry_the_error_flag_the_client_reads() {
        assert_eq!(text_ok("x")["isError"], serde_json::json!(false));
        assert_eq!(text_err("x")["isError"], serde_json::json!(true));
        assert_eq!(text_ok("x")["content"][0]["type"], serde_json::json!("text"));
    }

    #[test]
    fn destructive_tools_are_annotated_as_such() {
        let t = tool("x", "d", serde_json::json!({}), false, true);
        assert_eq!(t["annotations"]["destructiveHint"], serde_json::json!(true));
        assert_eq!(t["annotations"]["readOnlyHint"], serde_json::json!(false));
        let r = tool("y", "d", serde_json::json!({}), true, false);
        assert_eq!(r["annotations"]["readOnlyHint"], serde_json::json!(true));
    }

    /// Guard A on the MCP surface. The CLI refuses every mutating command inside
    /// a tree that rode in on a sync hub, at one dispatch choke point in main.rs;
    /// `worktrees mcp` never passes that point, and a model driving this server
    /// has no way to know which tree it is standing in.
    ///
    /// The loop is deliberate: it derives the mutating set from the server's OWN
    /// `readOnlyHint` annotations, so a tool added later is covered without
    /// editing this test — and cannot be added as "mutating but unguarded".
    #[test]
    fn a_mutating_tool_is_refused_inside_a_hub_copy() {
        use serde_json::json;
        // What a push leaves on the hub: <hub>/proj/.worktrees-sync.toml naming
        // the OTHER machine's root, and the tree itself at <hub>/proj/proj.
        let base = std::env::temp_dir().join(format!("wt-mcp-hubcopy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("proj");
        std::fs::create_dir_all(&root).unwrap();
        let init = std::process::Command::new("git")
            .args(["init", "-q"])
            .arg(&root)
            .status()
            .expect("git init");
        assert!(init.success());
        std::fs::write(
            base.join(worktrees_core::sync::MANIFEST),
            "schema = 1\nname = \"proj\"\nlocal_root = \"/Users/dp/work/proj\"\nhost = \"othermac\"\n",
        )
        .unwrap();

        let project = Project::discover(&root).expect("a git repo");
        let mut server = Server { project, mutations: true };

        // Reading is how you find out WHAT this tree is — never refused.
        let r = server.call(&json!({ "name": "list_places", "arguments": {} })).unwrap();
        assert_eq!(r["isError"], json!(false));

        let mutating: Vec<String> = server
            .tools()
            .iter()
            .filter(|t| t["annotations"]["readOnlyHint"] != json!(true))
            .map(|t| t["name"].as_str().unwrap_or_default().to_string())
            .collect();
        assert!(mutating.len() >= 6, "expected the whole mutating set, got {mutating:?}");
        for name in mutating {
            let r = server
                .call(&json!({
                    "name": name,
                    "arguments": {
                        "slug": "(main)", "note": "x", "pinned": true,
                        "lifecycle": "closed", "branch": "guard-test", "confirm": true
                    }
                }))
                .unwrap();
            let text = r["content"][0]["text"].as_str().unwrap_or_default();
            assert_eq!(r["isError"], json!(true), "{name} ran in a hub copy: {text}");
            assert!(
                text.contains("hub copy") && text.contains("/Users/dp/work/proj"),
                "{name} was refused for the wrong reason: {text}"
            );
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_parse_error_still_produces_a_well_formed_frame() {
        let s = err_obj(serde_json::Value::Null, -32700, "parse error");
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["jsonrpc"], serde_json::json!("2.0"));
        assert_eq!(v["error"]["code"], serde_json::json!(-32700));
        assert!(v["id"].is_null());
    }
}
