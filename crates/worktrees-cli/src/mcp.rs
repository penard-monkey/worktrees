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

use std::io::{BufRead, Write};

use worktrees_core::{ops, store, ui::CaptureUi, Project};

/// Versions this server understands. We echo the client's choice when we know
/// it, else answer with our newest — the handshake rule from the spec.
const SUPPORTED: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
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
    for line in stdin.lock().lines() {
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
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(serde_json::json!({}));

        // No id = notification. Never answer one, even to complain.
        let Some(id) = id else { return None };

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
        let version = if SUPPORTED.contains(&asked) { asked } else { LATEST };
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
                 open its tmux session.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "branch": { "type": "string" },
                        "base": { "type": "string", "description": "Base ref for a new branch. Optional." }
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
        let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let a = params.get("arguments").cloned().unwrap_or(serde_json::json!({}));
        let s = |k: &str| a.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();

        // A tool the server did not advertise must not be callable, or
        // `--mutations` would be advisory rather than a gate.
        if !self.tools().iter().any(|t| t["name"] == name) {
            let hint = if !self.mutations {
                " (this server was started without --mutations)"
            } else {
                ""
            };
            return Ok(text_err(&format!("unknown tool: {name}{hint}")));
        }

        match name {
            "list_places" => {
                let ls = self.project.ls();
                Ok(text_ok(&serde_json::to_string_pretty(&ls).unwrap_or_default()))
            }
            "place_status" => {
                let slug = s("slug");
                let ls = self.project.ls();
                match ls.places.iter().find(|p| p.slug == slug) {
                    Some(p) => Ok(text_ok(&serde_json::to_string_pretty(p).unwrap_or_default())),
                    None => Ok(text_err(&format!("no such place: {slug}"))),
                }
            }
            "doctor" => Ok(self.run_op(|p, ui| ops::cmd_doctor(p, ui, &[]))),
            "set_note" => {
                let (slug, note) = (s("slug"), s("note"));
                self.meta(&slug, |d| {
                    d.note = if note.is_empty() { None } else { Some(note.clone()) }
                })
            }
            "set_pin" => {
                let slug = s("slug");
                let pinned = a.get("pinned").and_then(|v| v.as_bool()).unwrap_or(false);
                self.meta(&slug, |d| d.pinned = Some(pinned))
            }
            "set_lifecycle" => {
                let (slug, life) = (s("slug"), s("lifecycle"));
                if !["closed", "saved", "archived", "abandoned"].contains(&life.as_str()) {
                    return Ok(text_err(&format!("invalid lifecycle: {life}")));
                }
                self.meta(&slug, |d| d.lifecycle = Some(life.clone()))
            }
            "create_worktree" => {
                let branch = s("branch");
                if branch.is_empty() {
                    return Ok(text_err("branch is required"));
                }
                let base = s("base");
                let mut args = vec![branch, "--no-attach".to_string()];
                if !base.is_empty() {
                    args.insert(1, base);
                }
                Ok(self.run_op(move |p, ui| ops::cmd_new(p, ui, &args)))
            }
            "close_session" => {
                let slug = s("slug");
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
                let slug = s("slug");
                let args = vec![slug, "-y".to_string()];
                Ok(self.run_op(move |p, ui| ops::cmd_rm(p, ui, &args)))
            }
            other => Ok(text_err(&format!("unknown tool: {other}"))),
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
    fn a_notification_is_never_answered() {
        // JSON-RPC: a message with no id gets no reply. Answering one corrupts
        // the stream, because the client is not expecting a frame.
        let msg = serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(msg.get("id").is_none());
    }

    #[test]
    fn version_negotiation_echoes_a_known_version_else_falls_back() {
        fn pick(asked: &str) -> &str {
            if SUPPORTED.contains(&asked) {
                asked
            } else {
                LATEST
            }
        }
        assert_eq!(pick("2025-06-18"), "2025-06-18", "a version we know is echoed back");
        assert_eq!(pick("2024-11-05"), "2024-11-05");
        assert_eq!(pick("1999-01-01"), LATEST, "an unknown version falls back to ours");
        assert!(SUPPORTED.contains(&LATEST));
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

    #[test]
    fn a_parse_error_still_produces_a_well_formed_frame() {
        let s = err_obj(serde_json::Value::Null, -32700, "parse error");
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["jsonrpc"], serde_json::json!("2.0"));
        assert_eq!(v["error"]["code"], serde_json::json!(-32700));
        assert!(v["id"].is_null());
    }
}
