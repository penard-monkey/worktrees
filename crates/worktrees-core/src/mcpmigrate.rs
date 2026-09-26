//! Copy user-scope Claude MCP definitions through Codex's CLI, never by editing its config.
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Copy,
    CopyNeedsLogin,
    CopyLiteralEnv,
    Exists,
    Differs,
    Unsupported,
}
impl Status {
    pub fn copyable(&self) -> bool {
        matches!(
            self,
            Self::Copy | Self::CopyNeedsLogin | Self::CopyLiteralEnv
        )
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Row {
    pub name: String,
    pub transport: String,
    pub status: Status,
    pub reason: String,
    /// Arguments after the executable. Passed directly to Command, never a shell.
    #[serde(skip)]
    pub argv: Vec<String>,
}

/// Pure, deterministic planning over the two parsed user configuration files.
pub fn plan(claude: &Value, codex: &toml::Value) -> Vec<Row> {
    let Some(servers) = claude.get("mcpServers").and_then(Value::as_object) else {
        return vec![];
    };
    let target = serde_json::to_value(codex).unwrap_or(Value::Null);
    servers
        .iter()
        .filter(|(name, _)| name.as_str() != "worktrees")
        .map(|(name, entry)| {
            let transport = entry
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or(if entry.get("url").is_some() {
                    "http"
                } else {
                    "stdio"
                })
                .to_owned();
            let mut row = Row {
                name: name.clone(),
                transport,
                status: Status::Unsupported,
                reason: String::new(),
                argv: vec![],
            };
            match translate(&row, entry) {
                Err(reason) => row.reason = reason,
                Ok((argv, expected, login, note)) => {
                    if let Some(existing) = target.get("mcp_servers").and_then(|m| m.get(name)) {
                        let same = equivalent(existing, &expected);
                        row.status = if same {
                            Status::Exists
                        } else {
                            Status::Differs
                        };
                        row.reason = if same {
                            "Already in Codex."
                        } else {
                            "A different entry already exists in Codex; it will not be overwritten."
                        }
                        .into();
                    } else {
                        row.status = if login {
                            Status::CopyNeedsLogin
                        } else if !literal_env_keys(entry).is_empty() {
                            Status::CopyLiteralEnv
                        } else {
                            Status::Copy
                        };
                        row.argv = argv;
                        row.reason = note;
                    }
                }
            }
            row
        })
        .collect()
}

fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && !s.contains('\0'))
        .ok_or_else(|| format!("Missing or invalid {key}."))
}
fn allowed(v: &Value, keys: &[&str]) -> Result<(), String> {
    let obj = v.as_object().ok_or("Expected a server object.")?;
    if obj.keys().any(|k| !keys.contains(&k.as_str())) {
        return Err("This entry has settings that codex mcp add cannot preserve.".into());
    }
    Ok(())
}
fn env_name(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .enumerate()
            .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
}
/// Only an entire ${VAR} reference avoids the literal-value warning. Mixed
/// strings and empty values still get explicit disclosure without their contents.
fn literal_env_keys(entry: &Value) -> Vec<&str> {
    entry
        .get("env")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| {
            let reference = value
                .as_str()
                .and_then(|s| s.strip_prefix("${"))
                .and_then(|s| s.strip_suffix('}'))
                .is_some_and(env_name);
            (!reference).then_some(key.as_str())
        })
        .collect()
}

fn translate(row: &Row, entry: &Value) -> Result<(Vec<String>, Value, bool, String), String> {
    if row.name.is_empty()
        || !row
            .name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        || row.name.starts_with('-')
    {
        return Err("Codex requires a server name using letters, numbers, underscores or hyphens, without a leading hyphen.".into());
    }
    if entry.get("type").is_some_and(|v| !v.is_string()) {
        return Err("Transport type must be a string.".into());
    }
    let mut argv = vec!["mcp".into(), "add".into(), row.name.clone()];
    let mut expected = json!({});
    let mut login = false;
    let mut note = "Ready to copy.".to_owned();
    match row.transport.as_str() {
        "stdio" => {
            allowed(entry, &["type", "command", "args", "env"])?;
            expected["command"] = text(entry, "command")?.into();
            let args = match entry.get("args") {
                None => vec![],
                Some(v) => v
                    .as_array()
                    .ok_or("args must be an array of strings.")?
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .filter(|s| !s.contains('\0'))
                            .map(str::to_owned)
                            .ok_or("args must be strings.")
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            };
            let mut env = serde_json::Map::new();
            if let Some(vars) = entry.get("env") {
                for (key, value) in vars
                    .as_object()
                    .ok_or("env must be an object of strings.")?
                {
                    if !env_name(key) {
                        return Err("Invalid environment variable name.".into());
                    }
                    let value = value
                        .as_str()
                        .filter(|s| !s.contains('\0'))
                        .ok_or("Environment values must be strings.")?;
                    argv.extend(["--env".into(), format!("{key}={value}")]);
                    env.insert(key.clone(), value.into());
                }
            }
            expected["args"] = json!(args);
            expected["env"] = env.into();
            argv.extend(["--".into(), text(entry, "command")?.into()]);
            argv.extend(args);
        }
        "http" => {
            allowed(entry, &["type", "url", "headers", "oauth"])?;
            let url = text(entry, "url")?;
            if !url.starts_with("https://") && !url.starts_with("http://") {
                return Err("Expected an HTTP or HTTPS URL.".into());
            }
            expected["url"] = url.into();
            argv.extend(["--url".into(), url.into()]);
            if let Some(headers) = entry.get("headers") {
                let headers = headers.as_object().ok_or("headers must be an object.")?;
                if headers.len() > 1
                    || headers
                        .keys()
                        .any(|k| !k.eq_ignore_ascii_case("authorization"))
                {
                    return Err("Custom HTTP headers require config-only http_headers; codex mcp add cannot copy them.".into());
                }
                if let Some(value) = headers.values().next() {
                    let var = value
                        .as_str()
                        .and_then(|s| s.strip_prefix("Bearer ${"))
                        .and_then(|s| s.strip_suffix('}'))
                        .filter(|s| env_name(s));
                    let var = var.ok_or("Move the bearer token into an environment variable and use Authorization: Bearer ${VAR}; literal tokens and other authentication headers cannot be copied.")?;
                    expected["bearer_token_env_var"] = var.into();
                    argv.extend(["--bearer-token-env-var".into(), var.into()]);
                }
            }
            if let Some(oauth) = entry.get("oauth") {
                if expected.get("bearer_token_env_var").is_some() {
                    return Err("Combined bearer and OAuth authentication cannot be copied.".into());
                }
                allowed(oauth, &["clientId", "resource", "clientRegistration"])?;
                login = true;
                for (key, flag) in [
                    ("clientId", "--oauth-client-id"),
                    ("resource", "--oauth-resource"),
                    ("clientRegistration", "--oauth-client-registration"),
                ] {
                    if oauth.get(key).is_none() {
                        continue;
                    }
                    let value = text(oauth, key)?;
                    if key == "clientRegistration" && !["auto", "cimd", "dcr"].contains(&value) {
                        return Err("Unknown OAuth client registration strategy.".into());
                    }
                    argv.extend([flag.into(), value.into()]);
                    match key {
                        "clientId" => expected["oauth"] = json!({"client_id":value}),
                        "resource" => expected["oauth_resource"] = value.into(),
                        _ => {} // Codex only uses registration strategy for immediate login.
                    }
                }
                note = format!("Sign-in needed: codex mcp login {}.", row.name);
                if oauth.get("clientRegistration").is_some() {
                    note.push_str(" OAuth registration strategy is used only by the immediate login attempt, not saved by Codex.");
                }
            } else if expected.get("bearer_token_env_var").is_none() {
                note = format!("Ready to copy. If this server requires OAuth, run codex mcp login {} afterwards.", row.name);
            }
        }
        "sse" => {
            return Err("SSE cannot be copied: Codex supports streamable HTTP, not SSE.".into())
        }
        _ => return Err("Unsupported transport.".into()),
    }
    for key in literal_env_keys(entry) {
        note.push_str(&format!(" {key} is a literal value; it will be written into Codex config and briefly visible in the process list while copying. Select this server explicitly to copy it."));
    }
    if argv.iter().any(|s| s.contains("${")) {
        note.push_str(" ${VAR} references are passed literally, never expanded; check the server's environment in Codex.");
    }
    Ok((argv, expected, login, note))
}

fn equivalent(existing: &Value, expected: &Value) -> bool {
    fn normalized(v: &Value) -> Value {
        let mut v = v.clone();
        if let Some(m) = v.as_object_mut() {
            if m.get("enabled") == Some(&json!(true)) {
                m.remove("enabled");
            }
            for key in [
                "args",
                "env",
                "http_headers",
                "env_http_headers",
                "env_vars",
            ] {
                if m.get(key).is_some_and(|v| {
                    v.as_array().is_some_and(Vec::is_empty)
                        || v.as_object().is_some_and(serde_json::Map::is_empty)
                }) {
                    m.remove(key);
                }
            }
            // Codex generates this callback URL when adding a custom OAuth client.
            if let Some(oauth) = m.get_mut("oauth").and_then(Value::as_object_mut) {
                oauth.remove("callback_url");
            }
        }
        v
    }
    normalized(existing) == normalized(expected)
}

fn read_codex() -> Result<toml::Value, String> {
    let path = crate::codexmcp::config_path();
    match std::fs::read_to_string(&path) {
        Ok(s) => toml::from_str(&s)
            .map_err(|_| "Cannot parse Codex configuration; migration was not attempted.".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok(toml::Value::Table(Default::default()))
        }
        Err(_) => Err("Cannot read Codex configuration; migration was not attempted.".into()),
    }
}

pub fn migration_plan() -> Result<Vec<Row>, String> {
    let source = crate::mcpsetup::read_claude_json_checked()?;
    if !source.is_null() && !source.is_object() {
        return Err("Claude configuration must be an object.".into());
    }
    if source.get("mcpServers").is_some_and(|v| !v.is_object()) {
        return Err("Claude mcpServers must be an object.".into());
    }
    let target = read_codex()?;
    if target.get("mcp_servers").is_some_and(|v| !v.is_table()) {
        return Err("Codex mcp_servers must be a table.".into());
    }
    Ok(plan(&source, &target))
}

#[derive(Debug, Serialize)]
pub struct Outcome {
    pub name: String,
    pub ok: bool,
    pub output: String,
    pub needs_login: bool,
}

/// Keep a stable sibling inode: Codex replaces config.toml when it writes.
/// File owns the advisory lock until drop; never unlink the lock file.
fn lock_migration() -> Result<std::fs::File, String> {
    use std::os::fd::AsRawFd;
    let config = crate::codexmcp::config_path();
    let parent = config
        .parent()
        .ok_or("Codex configuration has no parent directory.")?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("Cannot create Codex configuration directory: {e}"))?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(config.with_file_name("config.toml.worktrees-migrate.lock"))
        .map_err(|e| format!("Cannot open Codex migration lock: {e}"))?;
    loop {
        // SAFETY: file owns a live descriptor and stays alive through recheck,
        // add, and verification in both the CLI and app's shared apply path.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
            return Ok(file);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(format!("Cannot acquire Codex migration lock: {error}"));
        }
    }
}

/// Re-plan before EACH command, skip existing names, and verify the persisted
/// definition afterwards. This cannot serialize independent external Codex writers.
pub fn apply(names: &[String]) -> Result<Vec<Outcome>, String> {
    let codex = crate::profile::codex_bin().ok_or("Codex CLI was not found on PATH.")?;
    // Serialize threads as well as cooperating CLI/app processes. External
    // tools that do not acquire our flock remain outside our control.
    static APPLY: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = APPLY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _file_guard = lock_migration()?;
    let mut outcomes = vec![];
    for name in names {
        let mut result = Outcome {
            name: name.clone(),
            ok: false,
            output: String::new(),
            needs_login: false,
        };
        let attempt = || -> Result<(bool, bool, String), String> {
            let row = migration_plan()?
                .into_iter()
                .find(|r| &r.name == name)
                .ok_or("Server is not in Claude's user-scope migration plan.")?;
            if !row.status.copyable() {
                return Ok((false, false, row.reason));
            }
            let login = row.status == Status::CopyNeedsLogin;
            let args: Vec<&str> = row.argv.iter().map(String::as_str).collect();
            let (ok, _) = crate::codexmcp::run(&codex.to_string_lossy(), &args)?;
            // CLI output may echo credentials in args/env. Return a bounded, safe
            // result instead of exposing raw subprocess output in UI logs.
            let landed = migration_plan()?
                .iter()
                .any(|r| &r.name == name && r.status == Status::Exists);
            let message = if landed && ok {
                "Copied to Codex."
            } else if landed {
                "Configuration copied, but the Codex command did not complete successfully; check sign-in."
            } else {
                "Codex did not save the expected configuration. Check the server definition and Codex CLI."
            };
            Ok((ok && landed, login && landed, message.into()))
        };
        match attempt() {
            Ok((ok, login, output)) => {
                result.ok = ok;
                result.needs_login = login;
                result.output = output;
            }
            Err(e) => result.output = e,
        }
        outcomes.push(result);
    }
    Ok(outcomes)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rows(source: Value, target: &str) -> Vec<Row> {
        plan(&source, &toml::from_str(target).unwrap())
    }
    fn row(entry: Value) -> Row {
        rows(json!({"mcpServers":{"demo":entry}}), "").remove(0)
    }
    #[test]
    fn serialized_plan_never_exposes_execution_arguments() {
        let r = row(
            json!({"command":"echo","args":["private-argument"],"env":{"OPENAI_API_KEY":"sk-example-secret"}}),
        );
        let serialized = serde_json::to_string(&r).unwrap();
        assert!(!serialized.contains("argv"));
        assert!(!serialized.contains("sk-example-secret"));
        assert!(!serialized.contains("private-argument"));
        assert!(r
            .argv
            .iter()
            .any(|a| a == "OPENAI_API_KEY=sk-example-secret"));
    }
    #[test]
    fn literal_env_values_are_copyable_with_explicit_warning() {
        for value in ["sk-example-secret", "", "prefix-${TOKEN}"] {
            let r = row(json!({"command":"echo","env":{"KEY":value,"REFERENCE":"${TOKEN}"}}));
            assert_eq!(serde_json::to_value(&r.status).unwrap(), "copy_literal_env");
            assert!(r.status.copyable());
            assert!(r.reason.contains("KEY is a literal value"));
            assert!(r.reason.contains("written into Codex config"));
            assert!(r.reason.contains("process list"));
            assert!(!r.reason.contains("REFERENCE is a literal value"));
        }
    }
    #[test]
    fn untranslatable_existing_entry_keeps_unsupported_reason() {
        let r = rows(
            json!({"mcpServers":{"demo":{"type":"sse","url":"https://example.com"}}}),
            "[mcp_servers.demo]\nurl='https://example.com'\n",
        );
        assert_eq!(r[0].status, Status::Unsupported);
        assert!(r[0].reason.contains("SSE cannot be copied"));
        assert!(r[0].argv.is_empty());
    }
    #[test]
    fn stdio() {
        let r =
            row(json!({"command":"node","args":["server.js","two words"],"env":{"KEY":"value"}}));
        assert_eq!(r.status, Status::CopyLiteralEnv);
        assert_eq!(
            r.argv,
            [
                "mcp",
                "add",
                "demo",
                "--env",
                "KEY=value",
                "--",
                "node",
                "server.js",
                "two words"
            ]
        );
    }
    #[test]
    fn http() {
        let r = row(json!({"type":"http","url":"https://example.com/mcp"}));
        assert_eq!(r.status, Status::Copy);
        assert_eq!(
            r.argv,
            ["mcp", "add", "demo", "--url", "https://example.com/mcp"]
        );
    }
    #[test]
    fn bearer_variable() {
        let r = row(
            json!({"type":"http","url":"https://example.com/mcp","headers":{"Authorization":"Bearer ${TOKEN}"}}),
        );
        assert_eq!(r.status, Status::Copy);
        assert_eq!(
            r.argv,
            [
                "mcp",
                "add",
                "demo",
                "--url",
                "https://example.com/mcp",
                "--bearer-token-env-var",
                "TOKEN"
            ]
        );
    }
    #[test]
    fn literal_bearer_refused_without_exposing_token() {
        let r = row(
            json!({"type":"http","url":"https://example.com/mcp","headers":{"Authorization":"Bearer secret-value"}}),
        );
        assert_eq!(r.status, Status::Unsupported);
        assert!(r.reason.contains("environment variable"));
        assert!(!serde_json::to_string(&r).unwrap().contains("secret-value"));
        assert!(r.argv.is_empty());
    }
    #[test]
    fn other_headers_refused() {
        for headers in [
            json!({"X-Key":"value"}),
            json!({"Authorization":"Bearer ${TOKEN}","X-Key":"value"}),
        ] {
            let r = row(json!({"type":"http","url":"https://example.com/mcp","headers":headers}));
            assert_eq!(r.status, Status::Unsupported);
            assert!(r.argv.is_empty());
        }
    }
    #[test]
    fn oauth_requires_login() {
        let r = row(
            json!({"type":"http","url":"https://example.com/mcp","oauth":{"clientId":"client","resource":"resource","clientRegistration":"dcr"}}),
        );
        assert_eq!(r.status, Status::CopyNeedsLogin);
        assert_eq!(
            r.argv,
            [
                "mcp",
                "add",
                "demo",
                "--url",
                "https://example.com/mcp",
                "--oauth-client-id",
                "client",
                "--oauth-resource",
                "resource",
                "--oauth-client-registration",
                "dcr"
            ]
        );
        assert!(r.reason.contains("codex mcp login demo"));
    }
    #[test]
    fn sse_refused() {
        assert_eq!(
            row(json!({"type":"sse","url":"https://example.com"})).status,
            Status::Unsupported
        );
    }
    #[test]
    fn interpolation_stays_literal() {
        let r = row(json!({"command":"echo","args":["${PATH}"],"env":{"KEY":"${SECRET}"}}));
        assert_eq!(r.status, Status::Copy);
        assert!(r.argv.contains(&"${PATH}".into()));
        assert!(r.argv.contains(&"KEY=${SECRET}".into()));
        assert!(r.reason.contains("literal"));
    }
    #[test]
    fn existing_identical_and_different() {
        let c = json!({"mcpServers":{"same":{"command":"echo"},"different":{"command":"echo"},"http":{"type":"http","url":"https://example.com"}}});
        let r = rows(c, "[mcp_servers.same]\ncommand='echo'\nargs=[]\n[mcp_servers.different]\ncommand='other'\n[mcp_servers.http]\nurl='https://example.com'\n");
        assert_eq!(
            r.iter().find(|r| r.name == "same").unwrap().status,
            Status::Exists
        );
        assert_eq!(
            r.iter().find(|r| r.name == "different").unwrap().status,
            Status::Differs
        );
        assert_eq!(
            r.iter().find(|r| r.name == "http").unwrap().status,
            Status::Exists
        );
        assert!(r.iter().all(|r| r.argv.is_empty()));
    }
    #[test]
    fn existing_auth_env_and_disabled_differences() {
        for extra in [
            "enabled=false",
            "bearer_token_env_var='TOKEN'",
            "http_headers={X='y'}",
        ] {
            let r = rows(
                json!({"mcpServers":{"demo":{"type":"http","url":"https://example.com"}}}),
                &format!("[mcp_servers.demo]\nurl='https://example.com'\n{extra}"),
            );
            assert_eq!(r[0].status, Status::Differs);
        }
    }
    #[test]
    fn worktrees_and_project_scope_excluded() {
        let r = rows(
            json!({"mcpServers":{"worktrees":{"command":"worktrees"},"demo":{"command":"echo"}},"projects":{"/repo":{"mcpServers":{"local":{"command":"echo"}}}}}),
            "",
        );
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].name, "demo");
    }
    #[test]
    fn invalid_transport_type_is_not_defaulted() {
        assert_eq!(
            row(json!({"type":5,"command":"echo"})).status,
            Status::Unsupported
        );
    }
    #[test]
    fn malformed_or_lossy_entries_refused() {
        for e in [
            json!(null),
            json!({"command":""}),
            json!({"command":"echo","args":[1]}),
            json!({"command":"echo","env":{"A":1}}),
            json!({"command":"echo","cwd":"/tmp"}),
            json!({"type":"http","url":"https://example.com","oauth":{"clientSecret":"secret"}}),
        ] {
            let r = row(e);
            assert_eq!(r.status, Status::Unsupported);
            assert!(r.argv.is_empty());
        }
    }
}
