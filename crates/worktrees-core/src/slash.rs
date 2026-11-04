//! The slash commands a session has, and the one rule for a command line
//! typed into it (`docs/proposals/chat-slash-commands.md`).
//!
//! **Built-ins come from the harness's binary, never from files**, so they are
//! TABLES measured per version: Claude 2.1.318's `/help` → Commands, codex-cli
//! 0.160.1's `/` popup (fixtures `claude-screen/2.1.318/slash/help-commands-
//! pages.txt`, `codex-inline/0.160.1/slash/menu-pages.txt`). A session on
//! another version gets the nearest table, and the answer says which.
//!
//! **Claude's own commands are files**, read the way Claude names them:
//! `commands/**.md` (a sub-folder is a namespace, `sub/x.md` is `/sub:x`) and
//! `skills/<name>/SKILL.md`, in `~/.claude` and in the place's `.claude`; the
//! account's synced skills; and every plugin the session runs. A PLACE's
//! folders are the repository's, so a link there is never followed, at any
//! component (a planted `.claude/commands/x.md -> ~/.ssh/id_ed25519` would
//! otherwise send a key's first line to a phone as a description). MCP prompts
//! are not listed: only the running Claude knows them (the proposal, §1.5).
//!
//! **Tiers.** `Sensitive` is account, login, permissions, settings writers,
//! MCP, hooks, plugins, directories, memory files, remote, cloud and
//! background work, and data that leaves the machine — and every custom
//! command whose file runs a shell line (`` !`…` ``) or grants tools
//! (`allowed-tools`), whatever its source. A device needs `admin` for these;
//! the desktop confirms them. `confirm` is the desktop's words for a command
//! that replaces or ends the conversation.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// Where a command comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    BuiltIn,
    User,
    Project,
    Plugin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Normal,
    Sensitive,
}

/// One measured built-in: its name (no `/`), the description the harness
/// shows, its tier and why, the desktop's confirm, and whether the chat lets
/// it take an argument (`/model` does not: owner's decision, 2026-11-04).
#[derive(Clone, Copy, Debug)]
pub struct Builtin {
    pub name: &'static str,
    pub description: &'static str,
    pub tier: Tier,
    pub why: Option<&'static str>,
    pub confirm: Option<&'static str>,
    pub args: bool,
}

const fn b(name: &'static str, description: &'static str, tier: Tier, why: Option<&'static str>, confirm: Option<&'static str>, args: bool) -> Builtin {
    Builtin { name, description, tier, why, confirm, args }
}

/// A command as a client is told about it (`agent_commands`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Command {
    pub name: String,
    pub description: String,
    pub source: Source,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin: Option<String>,
    pub tier: Tier,
    /// Why it is sensitive, in a few words.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// The desktop's two-click words: every sensitive command has them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirm: Option<String>,
    /// The argument hint its file gives (`argument-hint`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Whether the chat lets it take an argument.
    pub args: bool,
}

impl Command {
    fn builtin(b: &Builtin) -> Command {
        Command {
            name: b.name.into(),
            description: b.description.into(),
            source: Source::BuiltIn,
            plugin: None,
            tier: b.tier,
            why: b.why.map(Into::into),
            confirm: b.confirm.map(Into::into).or_else(|| (b.tier == Tier::Sensitive).then(|| sensitive_words(b.why))),
            hint: None,
            args: b.args,
        }
    }
}

fn sensitive_words(why: Option<&str>) -> String {
    match why {
        Some(w) => format!("A sensitive command ({w})."),
        None => "A sensitive command.".into(),
    }
}

/// The built-ins of one harness, as measured on one version.
pub struct Table {
    pub harness: &'static str,
    pub version: &'static str,
    pub commands: &'static [Builtin],
}

pub const TABLES: &[Table] = &[
    Table { harness: "claude", version: "2.1.318", commands: CLAUDE_2_1_318 },
    Table { harness: "codex", version: "0.160.1", commands: CODEX_0_160_1 },
];

/// The table measured on the newest version at or below `version`, else the
/// oldest; the only table, today, per harness. `None` for a harness with no
/// table (pi, kimi: the chat covers Claude and Codex).
pub fn table(harness: &str, version: Option<&str>) -> Option<&'static Table> {
    let mut mine: Vec<&Table> = TABLES.iter().filter(|t| t.harness == harness).collect();
    mine.sort_by_key(|t| version_key(t.version));
    let want = version.map(version_key);
    match want {
        Some(v) => mine.iter().rev().find(|t| version_key(t.version) <= v).or(mine.first()).copied(),
        None => mine.last().copied(),
    }
}

/// `2.1.318` as numbers to compare; a part that is not a number is 0.
fn version_key(v: &str) -> (u64, u64, u64) {
    let mut it = v.trim().split(['.', '-', ' ']).map(|p| p.parse::<u64>().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

/// The session's commands: the built-ins, then `custom` (a name a built-in
/// has, or one listed earlier, is dropped: the first wins), sorted by name.
pub fn list(table: &Table, custom: Vec<Command>) -> Vec<Command> {
    let mut out: Vec<Command> = table.commands.iter().map(Command::builtin).collect();
    for c in custom {
        if !out.iter().any(|o| o.name == c.name) {
            out.push(c);
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// `agent_command` with a line that is no command: `!` is Claude's shell.
pub const ERR_SHELL: &str = "a line starting with '!' runs a shell command in the agent's terminal: it is not a slash command, and is refused";
/// … and `#` writes Claude's memory.
pub const ERR_MEMORY: &str = "a line starting with '#' writes to the agent's memory files: it is not a slash command, and is refused";
/// The head of the refusal for a name the session does not list.
pub const ERR_UNKNOWN: &str = "not a command this session has:";
/// The head of the refusal for an argument the chat does not send.
pub const ERR_NO_ARGS: &str = "takes no argument from the chat";

/// The command `name` names (with or without its `/`) in `list`. `!` and `#`
/// are refused as what they are; anything else unlisted as unknown.
pub fn resolve<'a>(list: &'a [Command], name: &str) -> Result<&'a Command, String> {
    let n = name.trim();
    if n.starts_with('!') {
        return Err(ERR_SHELL.into());
    }
    if n.starts_with('#') {
        return Err(ERR_MEMORY.into());
    }
    let n = n.strip_prefix('/').unwrap_or(n);
    if !valid_name(n) {
        return Err(format!("{ERR_UNKNOWN} /{}", n.chars().take(64).collect::<String>()));
    }
    list.iter().find(|c| c.name == n).ok_or_else(|| format!("{ERR_UNKNOWN} /{n}"))
}

/// A command name as both harnesses draw them: letters, digits, `_ . : -`,
/// starting with a letter or digit, at most 100 bytes.
fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 100
        && n.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'))
}

/// The line typed for `cmd` with `args`: `/<name>` or `/<name> <args>`. The
/// arguments are ONE line under every rule of `harness::typed_line_ok` (no
/// control or invisible character, not starting with `/ @ ! #`), the whole
/// line within `harness::LINE_MAX`, and no `@` anywhere: Claude opens its
/// file menu on `@`, and the Enter that runs the command would pick a file.
pub fn line(cmd: &Command, args: &str) -> Result<String, String> {
    let args = args.trim();
    if args.is_empty() {
        return Ok(format!("/{}", cmd.name));
    }
    if !cmd.args {
        return Err(format!("/{} {ERR_NO_ARGS}: pick from what it opens instead", cmd.name));
    }
    crate::harness::typed_line_ok(args).map_err(|e| format!("its argument: {}", e.trim_start_matches("text ")))?;
    if args.contains('@') {
        return Err("its argument may not contain '@': Claude would open its file menu, and Enter would pick a file".into());
    }
    let line = format!("/{} {args}", cmd.name);
    if line.len() > crate::harness::LINE_MAX {
        return Err(format!("the command line is {} bytes; at most {} are typed", line.len(), crate::harness::LINE_MAX));
    }
    Ok(line)
}

// ── Claude's own commands, from files ──────────────────────────────────────

/// Where a Claude session's own commands live: the home its `~/.claude` is
/// in, the place, and the `--plugin-dir`s it was started with.
pub struct ClaudeSources<'a> {
    pub home: &'a Path,
    pub place: &'a Path,
    pub plugin_dirs: &'a [PathBuf],
}

/// The most files read per folder, and the bytes read per file.
const MAX_FILES: usize = 300;
const MAX_BYTES: u64 = 32 * 1024;

/// Every command and skill the session's files give it, in Claude's order of
/// precedence for a clash (the user's, the place's, synced, plugins').
pub fn claude_custom(src: &ClaudeSources) -> Vec<Command> {
    let mut out = Vec::new();
    let user = src.home.join(".claude");
    // The user's first: a personal skill outranks a repository's of the same
    // name (AGENTS.md, `close-out`).
    commands_in(&user.join("commands"), Source::User, None, true, &mut out);
    skills_in(&user.join("skills"), Source::User, None, true, &mut out);
    // The place's: the repository's files, read with no link followed.
    if let Some(dir) = crate::safepath::safe_dir_under(src.place, ".claude/commands") {
        commands_in(&dir, Source::Project, None, false, &mut out);
    }
    if let Some(dir) = crate::safepath::safe_dir_under(src.place, ".claude/skills") {
        skills_in(&dir, Source::Project, None, false, &mut out);
    }
    synced_in(&user.join("skills").join("synced"), &mut out);
    let given = src.plugin_dirs.iter().map(|d| (d.clone(), None));
    for (dir, key_name) in plugins(&user).into_iter().map(|(d, n)| (d, Some(n))).chain(given) {
        let name = plugin_name(&dir, key_name);
        commands_in(&dir.join("commands"), Source::Plugin, Some(&name), true, &mut out);
        skills_in(&dir.join("skills"), Source::Plugin, Some(&name), true, &mut out);
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|c| seen.insert(c.name.clone()));
    out
}

/// The installed plugins `~/.claude/settings.json` enables: each one's
/// install path (`installed_plugins.json`) and the name its `name@market` key
/// gives it — the install folder is its VERSION.
fn plugins(user: &Path) -> Vec<(PathBuf, String)> {
    let read = |p: PathBuf| std::fs::read_to_string(p).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
    let (Some(settings), Some(installed)) = (read(user.join("settings.json")), read(user.join("plugins/installed_plugins.json"))) else {
        return Vec::new();
    };
    let Some(enabled) = settings.get("enabledPlugins").and_then(|e| e.as_object()) else { return Vec::new() };
    enabled
        .iter()
        .filter(|(_, on)| on.as_bool() == Some(true))
        .filter_map(|(key, _)| {
            let path = installed.pointer(&format!("/plugins/{}", key.replace('~', "~0").replace('/', "~1")))?.get(0)?.get("installPath")?.as_str()?;
            Some((PathBuf::from(path), key.split('@').next().unwrap_or(key).to_string()))
        })
        .collect()
}

/// A plugin's name: its manifest's, else its settings key's, else its
/// folder's.
fn plugin_name(dir: &Path, key_name: Option<String>) -> String {
    std::fs::read_to_string(dir.join(".claude-plugin/plugin.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("name")?.as_str().map(str::to_string))
        .filter(|n| valid_name(n))
        .or(key_name.filter(|n| valid_name(n)))
        .unwrap_or_else(|| dir.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default())
}

/// The `.md` files under `dir`, as commands: `a/b.md` is `a:b`. `follow` is
/// false for a repository's folder, where no link is followed.
fn commands_in(dir: &Path, source: Source, plugin: Option<&str>, follow: bool, out: &mut Vec<Command>) {
    let mut files = Vec::new();
    walk(dir, &mut Vec::new(), follow, 0, &mut files);
    for (parts, path) in files {
        let mut name = parts.join(":");
        if let Some(p) = plugin {
            name = format!("{p}:{name}");
        }
        if let Some(c) = from_file(&path, name, source, plugin, follow) {
            out.push(c);
        }
    }
}

fn walk(dir: &Path, at: &mut Vec<String>, follow: bool, depth: usize, files: &mut Vec<(Vec<String>, PathBuf)>) {
    if depth > 3 || files.len() >= MAX_FILES {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if files.len() >= MAX_FILES {
            return;
        }
        let path = e.path();
        let Ok(md) = (if follow { std::fs::metadata(&path) } else { std::fs::symlink_metadata(&path) }) else { continue };
        let file = e.file_name().to_string_lossy().into_owned();
        if md.is_dir() && valid_name(&file) {
            at.push(file);
            walk(&path, at, follow, depth + 1, files);
            at.pop();
        } else if md.is_file() {
            if let Some(stem) = file.strip_suffix(".md").filter(|s| valid_name(s)) {
                let mut parts = at.clone();
                parts.push(stem.to_string());
                files.push((parts, path));
            }
        }
    }
}

/// `<dir>/<name>/SKILL.md`, as commands.
fn skills_in(dir: &Path, source: Source, plugin: Option<&str>, follow: bool, out: &mut Vec<Command>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().take(MAX_FILES).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let path = e.path();
        let ok = |p: &Path, dir: bool| match if follow { std::fs::metadata(p) } else { std::fs::symlink_metadata(p) } {
            Ok(md) => if dir { md.is_dir() } else { md.is_file() },
            Err(_) => false,
        };
        let folder = e.file_name().to_string_lossy().into_owned();
        if folder == "synced" || !valid_name(&folder) || !ok(&path, true) || !ok(&path.join("SKILL.md"), false) {
            continue;
        }
        let Some(mut c) = from_file(&path.join("SKILL.md"), folder.clone(), source, plugin, follow) else { continue };
        let head = read_head(&path.join("SKILL.md"), follow).map(|(t, _)| t).unwrap_or_default();
        let own = front(&head).map.get("name").cloned().filter(|n| valid_name(n));
        let base = own.unwrap_or(folder);
        c.name = match plugin {
            Some(p) => format!("{p}:{base}"),
            None => base,
        };
        out.push(c);
    }
}

/// The account's synced skills (`skills/synced/<bucket>/manifest.json`),
/// which Claude lists as `/anthropic-skills:<name>` (2.1.318: the ones whose
/// `creatorType` is `anthropic` or whose `source` starts `anthropic`; others
/// were not seen and are not guessed at).
fn synced_in(dir: &Path, out: &mut Vec<Command>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten().take(20) {
        let Ok(t) = std::fs::read_to_string(e.path().join("manifest.json")) else { continue };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) else { continue };
        for s in v.get("skills").and_then(|s| s.as_array()).into_iter().flatten().take(MAX_FILES) {
            let field = |k: &str| s.get(k).and_then(|v| v.as_str()).unwrap_or("");
            let anthropic = field("creatorType") == "anthropic" || field("source").starts_with("anthropic");
            let Some(name) = s.get("name").and_then(|n| n.as_str()).filter(|_| anthropic) else { continue };
            if !valid_name(name) {
                continue;
            }
            let description = one_line(s.get("description").and_then(|d| d.as_str()).unwrap_or(""));
            out.push(Command {
                name: format!("anthropic-skills:{name}"),
                description,
                source: Source::User,
                plugin: None,
                tier: Tier::Normal,
                why: None,
                confirm: None,
                hint: None,
                args: true,
            });
        }
    }
}

/// The head of a command's file (`MAX_BYTES`), with no link followed unless
/// `follow`, and whether the file goes on past it.
fn read_head(path: &Path, follow: bool) -> Option<(String, bool)> {
    use std::io::Read;
    let md = if follow { std::fs::metadata(path) } else { std::fs::symlink_metadata(path) }.ok()?;
    if !md.is_file() {
        return None;
    }
    let mut buf = Vec::new();
    std::fs::File::open(path).ok()?.take(MAX_BYTES + 1).read_to_end(&mut buf).ok()?;
    let cut = buf.len() as u64 > MAX_BYTES;
    buf.truncate(MAX_BYTES as usize);
    Some((String::from_utf8_lossy(&buf).into_owned(), cut))
}

/// A command from its file: the front matter's `description` (else the
/// body's first line) and `argument-hint`, and its tier from what the file
/// can do. A skill marked `user-invocable: false` is no slash command.
fn from_file(path: &Path, name: String, source: Source, plugin: Option<&str>, follow: bool) -> Option<Command> {
    if !valid_name(&name) {
        return None;
    }
    let (text, cut) = read_head(path, follow)?;
    let f = front(&text);
    let fm = &f.map;
    if fm.get("user-invocable").map(String::as_str) == Some("false") {
        return None;
    }
    let body = body(&text);
    let description = fm.get("description").map(|d| one_line(d)).filter(|d| !d.is_empty()).unwrap_or_else(|| {
        one_line(body.lines().map(|l| l.trim().trim_start_matches('#').trim()).find(|l| !l.is_empty()).unwrap_or(""))
    });
    let why = sensitive_why(&f, body, cut);
    let tier = if why.is_some() { Tier::Sensitive } else { Tier::Normal };
    let confirm = why.as_ref().map(|w| format!("A sensitive command: {w}."));
    Some(Command {
        name,
        description,
        source,
        plugin: plugin.map(str::to_string),
        tier,
        why,
        confirm,
        hint: fm.get("argument-hint").map(|h| one_line(h)).filter(|h| !h.is_empty()),
        args: true,
    })
}

/// Tools an `allowed-tools` line may grant and stay in the normal tier:
/// they read, and change nothing.
const READ_ONLY_TOOLS: &[&str] = &["Read", "Grep", "Glob", "LS", "NotebookRead", "TodoWrite"];

/// `allowed-tools` (`Read Grep Glob`, `Bash(git *), Edit`) names a tool that
/// is not a reader. Every word that starts a tool's name counts; a pattern in
/// parentheses belongs to the tool before it.
fn grants_more_than_reading(tools: &str) -> bool {
    let mut depth = 0usize;
    let mut word = String::new();
    let mut names = Vec::new();
    for c in tools.chars().chain(std::iter::once(' ')) {
        match c {
            '(' => {
                depth += 1;
                if !word.is_empty() {
                    names.push(std::mem::take(&mut word));
                }
            }
            ')' => depth = depth.saturating_sub(1),
            _ if depth > 0 => {}
            c if c.is_whitespace() || c == ',' || c == '[' || c == ']' || c == '"' || c == '\'' => {
                if !word.is_empty() {
                    names.push(std::mem::take(&mut word));
                }
            }
            c => word.push(c),
        }
    }
    names.iter().any(|n| !READ_ONLY_TOOLS.contains(&n.as_str()))
}

/// A file's YAML front matter, as far as this reader can tell: `key: value`
/// lines between the first two `---` rows (a quoted value unquoted, a folded
/// `>`/`|` one joined), a key's indented `- item` rows as a list, and
/// whatever it could not read as `unread`. Not a YAML parser: anything it
/// does not understand makes the command SENSITIVE ([`sensitive_why`]),
/// never normal.
#[derive(Default)]
struct Front {
    map: std::collections::HashMap<String, String>,
    lists: std::collections::HashMap<String, Vec<String>>,
    /// Keys whose value is neither a scalar nor a list of scalars, and rows
    /// that are no `key: value`.
    unread: Vec<String>,
    /// It opened with `---` and never closed.
    open: bool,
}

fn front(text: &str) -> Front {
    let mut f = Front::default();
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return f;
    }
    let all: Vec<&str> = lines.collect();
    let Some(close) = all.iter().position(|l| l.trim() == "---") else {
        f.open = true;
        return f;
    };
    let rows = &all[..close];
    let indented = |r: &str| r.starts_with([' ', '\t']);
    let mut i = 0;
    while i < rows.len() {
        let row = rows[i];
        i += 1;
        if row.trim().is_empty() || row.trim_start().starts_with('#') {
            continue;
        }
        let Some((k, v)) = row.split_once(':').filter(|_| !indented(row)) else {
            f.unread.push(row.trim().chars().take(40).collect());
            continue;
        };
        let k = k.trim().to_string();
        let mut v = v.trim().to_string();
        let mut block = Vec::new();
        while i < rows.len() && (indented(rows[i]) || rows[i].trim().is_empty()) {
            if !rows[i].trim().is_empty() {
                block.push(rows[i].trim());
            }
            i += 1;
        }
        if v == ">" || v == "|" || v == ">-" || v == "|-" {
            v = block.join(" ");
        } else if v.is_empty() && !block.is_empty() {
            match block.iter().map(|b| b.strip_prefix("- ").map(str::trim)).collect::<Option<Vec<_>>>() {
                Some(items) => {
                    f.lists.insert(k.clone(), items.into_iter().map(str::to_string).collect());
                }
                None => f.unread.push(k.clone()),
            }
            continue;
        } else if !block.is_empty() {
            f.unread.push(k.clone());
            continue;
        } else if v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'') {
            v = v[1..v.len() - 1].replace("''", "'");
        } else if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
            v = v[1..v.len() - 1].replace("\\\"", "\"");
        }
        f.map.insert(k, v);
    }
    f
}

/// The front-matter keys a command or skill may set and stay in the normal
/// tier: what it is called, what it says, how it is offered. Anything else
/// (`allowed-tools` aside, judged by its tools), `hooks`, `model`, `context`,
/// `agent` and keys this host has never seen alike, makes it sensitive.
const SAFE_KEYS: &[&str] = &["name", "description", "argument-hint", "user-invocable", "disable-model-invocation", "when_to_use", "when-to-use", "license", "version"];

/// Why a command's file makes it sensitive, or `None`: deny by default, so a
/// file this reader cannot see whole or understand is sensitive.
fn sensitive_why(f: &Front, body: &str, cut: bool) -> Option<String> {
    if cut {
        return Some(format!("its file is longer than {} KiB, more than this host reads", MAX_BYTES / 1024));
    }
    if f.open {
        return Some("its front matter never closes".into());
    }
    if body.contains("!`") || body.lines().any(|l| l.trim_start().starts_with("```!")) {
        return Some("its file runs a shell command before the model sees it".into());
    }
    if let Some(k) = f.unread.first() {
        return Some(format!("its front matter sets {k}, which this host cannot read"));
    }
    let tools: Option<Vec<String>> = match (f.map.get("allowed-tools"), f.lists.get("allowed-tools")) {
        (Some(v), _) => Some(vec![v.clone()]),
        (None, Some(items)) => Some(items.clone()),
        (None, None) => None,
    };
    if let Some(t) = tools {
        if t.iter().any(|v| v.trim().is_empty() || grants_more_than_reading(v)) {
            return Some("its file grants the model tools that act (allowed-tools)".into());
        }
    }
    let mut keys: Vec<&String> = f.map.keys().chain(f.lists.keys()).filter(|k| *k != "allowed-tools" && !SAFE_KEYS.contains(&k.as_str())).collect();
    keys.sort();
    keys.first().map(|k| format!("its front matter sets {k}"))
}

/// A file's text after its front matter.
fn body(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("---") else { return text };
    match rest.find("\n---") {
        Some(at) => rest[at + 4..].trim_start_matches(['-']),
        None => text,
    }
}

/// One line of display text: no control or invisible character, at most 300
/// characters.
fn one_line(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| if c == '\n' || c == '\t' { ' ' } else { c })
        .filter(|c| !c.is_control() && !crate::harness::invisible(*c))
        .collect();
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() > 300 {
        let mut cut: String = t.chars().take(299).collect();
        cut.push('…');
        cut
    } else {
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wt-slash-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn claude() -> Vec<Command> {
        list(table("claude", Some("2.1.318")).unwrap(), Vec::new())
    }

    /// The measured lists, whole: Claude 2.1.318's 108 default commands and
    /// Codex 0.160.1's 53, each with its description, `/model` taking no
    /// argument from the chat, `/clear` and `/new` confirmed, and the
    /// account and settings commands sensitive.
    #[test]
    fn the_tables_are_the_measured_lists() {
        assert_eq!(CLAUDE_2_1_318.len(), 108);
        assert_eq!(CODEX_0_160_1.len(), 53);
        let c = claude();
        let get = |n: &str| c.iter().find(|x| x.name == n).unwrap_or_else(|| panic!("{n}"));
        assert_eq!(get("model").description, "Set the AI model for Claude Code", "no live `(currently …)`");
        assert!(!get("model").args && get("compact").args);
        assert!(get("clear").confirm.as_deref().is_some_and(|w| w.contains("new conversation")) && get("clear").tier == Tier::Normal);
        for n in ["login", "logout", "permissions", "config", "mcp", "hooks", "plugin", "add-dir", "memory", "effort"] {
            assert_eq!((n, get(n).tier), (n, Tier::Sensitive));
            assert!(get(n).confirm.is_some(), "{n}: the desktop confirms every sensitive command");
        }
        for n in ["status", "usage", "context", "compact", "help"] {
            assert_eq!((n, get(n).tier, get(n).confirm.is_some()), (n, Tier::Normal, false));
        }
        let x = list(table("codex", None).unwrap(), Vec::new());
        let get = |n: &str| x.iter().find(|c| c.name == n).unwrap_or_else(|| panic!("{n}"));
        assert!(get("new").confirm.is_some() && get("clear").confirm.is_some());
        assert_eq!(get("permissions").tier, Tier::Sensitive);
        assert!(!get("model").args);
        // A newer version gets the newest table at or below it; pi none.
        assert_eq!(table("claude", Some("2.1.400")).unwrap().version, "2.1.318");
        assert_eq!(table("claude", Some("2.1.200")).unwrap().version, "2.1.318", "the oldest when none is older");
        assert!(table("pi", None).is_none());
    }

    /// A name is the session's exactly; `!` and `#` are refused as what they
    /// are, never looked up.
    #[test]
    fn a_name_is_one_the_session_lists() {
        let c = claude();
        assert_eq!(resolve(&c, "/status").unwrap().name, "status");
        assert_eq!(resolve(&c, "status").unwrap().name, "status");
        assert_eq!(resolve(&c, "!ls"), Err(ERR_SHELL.to_string()));
        assert_eq!(resolve(&c, "#remember this"), Err(ERR_MEMORY.to_string()));
        for bad in ["/nope", "/stat", "/sta tus", "/status/x", "/../x", "", "/"] {
            assert!(resolve(&c, bad).unwrap_err().starts_with(ERR_UNKNOWN), "{bad:?}");
        }
    }

    /// The arguments are one line under `typed_line_ok`'s rules, with no `@`
    /// (Claude's file menu would take Enter), and `/model` takes none.
    #[test]
    fn the_line_is_the_name_and_one_clean_argument() {
        let c = claude();
        let compact = resolve(&c, "compact").unwrap();
        assert_eq!(line(compact, ""), Ok("/compact".into()));
        assert_eq!(line(compact, "  keep the facts "), Ok("/compact keep the facts".into()));
        assert!(line(resolve(&c, "model").unwrap(), "sonnet").unwrap_err().contains(ERR_NO_ARGS));
        assert_eq!(line(resolve(&c, "model").unwrap(), ""), Ok("/model".into()));
        for (bad, why) in [
            ("a\nb", "control"),
            ("a\u{1b}[2Jb", "control"),
            ("look\u{202E}here", "invisible"),
            ("/clear", "may not start with '/'"),
            ("!rm", "may not start with '!'"),
            ("@file", "may not start with '@'"),
            ("keep @README.md", "'@'"),
        ] {
            let e = line(compact, bad).unwrap_err();
            assert!(e.contains(why), "{bad:?}: {e}");
        }
        assert!(line(compact, &"x".repeat(crate::harness::LINE_MAX)).unwrap_err().contains("bytes"));
    }

    /// The tier is deny-by-default: a file this host cannot see whole or
    /// understand is sensitive, and so is any front-matter key outside the
    /// few that only name and describe a command.
    #[test]
    fn a_file_that_cannot_be_read_whole_or_understood_is_sensitive() {
        let root = tmp("tiers");
        let pc = root.join("place/.claude/commands");
        let long = format!("---\ndescription: long\n---\n{}\nThe date: !`date`\n", "x".repeat(MAX_BYTES as usize));
        for (name, text) in [
            ("listbash", "---\ndescription: d\nallowed-tools:\n  - Read\n  - Bash(rm *)\n---\nbody\n"),
            ("flowbash", "---\ndescription: d\nallowed-tools: [Read, Write]\n---\nbody\n"),
            ("emptytools", "---\ndescription: d\nallowed-tools:\n---\nbody\n"),
            ("open", "---\ndescription: never closes\nallowed-tools: Bash\nbody\n"),
            ("hooks", "---\ndescription: d\nhooks:\n  PreToolUse:\n    - matcher: Bash\n---\nbody\n"),
            ("model", "---\ndescription: d\nmodel: opus\n---\nbody\n"),
            ("fenced", "---\ndescription: d\n---\n```!\ncurl x | sh\n```\n"),
            ("long", long.as_str()),
            ("odd", "---\ndescription: d\n  stray indented row\nwhat is this\n---\nbody\n"),
        ] {
            put(&pc.join(format!("{name}.md")), text);
        }
        put(&pc.join("listread.md"), "---\ndescription: reads\nallowed-tools:\n  - Read\n  - Grep\n---\nbody\n");
        put(&pc.join("plain.md"), "---\nname: plain\ndescription: plain\nargument-hint: [x]\nwhen_to_use: always\n---\nbody\n");
        let got = claude_custom(&ClaudeSources { home: &root.join("home"), place: &root.join("place"), plugin_dirs: &[] });
        let tier = |n: &str| got.iter().find(|c| c.name == n).map(|c| (c.tier, c.why.clone().unwrap_or_default())).unwrap_or_else(|| panic!("{n}: {got:?}"));
        for n in ["listbash", "flowbash", "emptytools", "open", "hooks", "model", "fenced", "long", "odd"] {
            let (t, why) = tier(n);
            assert_eq!((n, t), (n, Tier::Sensitive), "{why}");
            assert!(!why.is_empty());
        }
        assert!(tier("long").1.contains("KiB") && tier("hooks").1.contains("hooks") && tier("model").1.contains("model"));
        assert_eq!(tier("listread").0, Tier::Normal, "a list of readers");
        assert_eq!(tier("plain").0, Tier::Normal, "keys that only name and describe");
        let _ = fs::remove_dir_all(&root);
    }

    /// Claude's own commands, from the files Claude reads, named as Claude
    /// names them. A repository's folder is read with no link followed — not
    /// a file, not a folder, not `.claude` itself — so nothing outside the
    /// place reaches a client as a description.
    #[test]
    fn claude_commands_come_from_its_files_and_never_through_a_repos_link() {
        let root = tmp("files");
        let (home, place, outside) = (root.join("home"), root.join("place"), root.join("outside"));
        put(&outside.join("secret.md"), "---\ndescription: SECRET-KEY-MARKER\n---\nSECRET-BODY-MARKER\n");
        put(&outside.join("dir/inner.md"), "SECRET-DIR-MARKER\n");
        let pc = place.join(".claude/commands");
        put(&pc.join("hello.md"), "---\ndescription: Say hello (scratch)\nargument-hint: [name]\n---\nSay hello to $ARGUMENTS.\n");
        put(&pc.join("sub/nested.md"), "# Nested one\nSay nested.\n");
        put(&pc.join("preexec.md"), "---\ndescription: 'It''s a pre-exec'\n---\nThe date is: !`date +%Y`\n");
        put(&pc.join("tools.md"), "---\ndescription: grants\nallowed-tools: Read, Bash(git *)\n---\nDo it.\n");
        put(&pc.join("reader.md"), "---\ndescription: reads\nallowed-tools: Read Grep Glob\n---\nLook.\n");
        std::os::unix::fs::symlink(outside.join("secret.md"), pc.join("leak.md")).unwrap();
        std::os::unix::fs::symlink(outside.join("dir"), pc.join("linked")).unwrap();
        put(&place.join(".claude/skills/s1/SKILL.md"), "---\nname: scratch-skill\ndescription: >\n  A folded\n  description\n---\nbody\n");
        put(&place.join(".claude/skills/quiet/SKILL.md"), "---\ndescription: hidden\nuser-invocable: false\n---\n");
        put(&home.join(".claude/commands/mine.md"), "---\ndescription: my own\n---\n");
        put(&home.join(".claude/skills/close-out/SKILL.md"), "---\nname: close-out\ndescription: Close out\n---\n");
        put(&place.join(".claude/skills/close-out/SKILL.md"), "---\nname: close-out\ndescription: The repo's copy\n---\n");
        put(&home.join(".claude/skills/synced/b1/manifest.json"), r#"{"skills":[{"name":"docs","description":"Docs\nskill","creatorType":"anthropic"},{"name":"pdf","description":"PDFs","source":"anthropic"},{"name":"mine","description":"x","creatorType":"user"}]}"#);
        let plug = root.join("plug");
        put(&plug.join(".claude-plugin/plugin.json"), r#"{"name":"worktrees"}"#);
        put(&plug.join("skills/worktrees/SKILL.md"), "---\nname: worktrees\ndescription: Use BEFORE any branch work\n---\n");
        let inst = root.join("inst");
        put(&inst.join("commands/review.md"), "---\ndescription: plugin command\n---\n");
        put(&home.join(".claude/settings.json"), r#"{"enabledPlugins":{"rev@market":true,"off@market":false}}"#);
        put(&home.join(".claude/plugins/installed_plugins.json"), &format!(r#"{{"plugins":{{"rev@market":[{{"installPath":"{}"}}]}}}}"#, inst.display()));
        let dirs = vec![plug.clone()];
        let got = claude_custom(&ClaudeSources { home: &home, place: &place, plugin_dirs: &dirs });
        let all = serde_json::to_string(&got).unwrap();
        for secret in ["SECRET-KEY-MARKER", "SECRET-BODY-MARKER", "SECRET-DIR-MARKER", "\"leak\"", "linked:"] {
            assert!(!all.contains(secret), "{secret}: {all}");
        }
        let get = |n: &str| got.iter().find(|c| c.name == n).unwrap_or_else(|| panic!("{n}: {all}"));
        assert_eq!((get("hello").source, get("hello").description.as_str(), get("hello").hint.as_deref()), (Source::Project, "Say hello (scratch)", Some("[name]")));
        assert_eq!(get("sub:nested").description, "Nested one", "a sub-folder is a namespace; no front matter: the first line");
        assert_eq!((get("preexec").tier, get("preexec").description.as_str()), (Tier::Sensitive, "It's a pre-exec"));
        assert!(get("preexec").why.as_deref().unwrap().contains("shell"));
        assert!(get("tools").why.as_deref().unwrap().contains("allowed-tools") && get("tools").confirm.is_some());
        assert_eq!(get("reader").tier, Tier::Normal, "granting only readers is not sensitive");
        assert_eq!((get("scratch-skill").source, get("scratch-skill").description.as_str()), (Source::Project, "A folded description"));
        assert!(!all.contains("\"quiet\"") && !all.contains("hidden"), "user-invocable: false is no command");
        assert_eq!((get("mine").source, get("close-out").source), (Source::User, Source::User));
        assert_eq!(get("close-out").description, "Close out", "a personal skill outranks the repository's");
        assert_eq!(get("anthropic-skills:docs").description, "Docs skill");
        assert_eq!(get("anthropic-skills:pdf").description, "PDFs", "marked by its source");
        assert!(!all.contains("anthropic-skills:mine"), "only what was seen is named");
        assert_eq!((get("worktrees:worktrees").source, get("worktrees:worktrees").plugin.as_deref()), (Source::Plugin, Some("worktrees")));
        assert_eq!(get("rev:review").plugin.as_deref(), Some("rev"), "an enabled plugin, by its folder's name");
        assert_eq!(got.iter().filter(|c| c.tier == Tier::Sensitive).count(), 2);
        // `.claude` itself a link out of the place: nothing of the place's.
        let place2 = root.join("place2");
        fs::create_dir_all(&place2).unwrap();
        std::os::unix::fs::symlink(place.join(".claude"), place2.join(".claude")).unwrap();
        let got2 = claude_custom(&ClaudeSources { home: &home, place: &place2, plugin_dirs: &[] });
        assert!(got2.iter().all(|c| c.source != Source::Project), "{got2:?}");
        // A custom command never shadows a built-in.
        let mut clash = got.clone();
        clash.push(Command { name: "status".into(), description: "MINE".into(), ..get("mine").clone() });
        let l = list(table("claude", None).unwrap(), clash);
        assert_eq!(l.iter().filter(|c| c.name == "status").count(), 1);
        assert_eq!(l.iter().find(|c| c.name == "status").unwrap().source, Source::BuiltIn);
        let _ = fs::remove_dir_all(&root);
    }
}

// ── the measured tables ─────────────────────────────────────────────────────

use Tier::{Normal, Sensitive};

pub const CLAUDE_2_1_318: &[Builtin] = &[
    b("add-dir", "Add a new working directory", Sensitive, Some("directories"), None, true),
    b("advisor", "Let Claude consult a stronger model at key moments", Sensitive, Some("settings"), None, true),
    b("artifacts", "Browse your published and shared artifacts", Normal, None, None, true),
    b("auto-mode-setup", "Teach auto mode about your environment, plus optional rule tweaks", Sensitive, Some("permissions"), None, true),
    b("autocompact", "Set how full the context gets before auto-summarizing", Sensitive, Some("settings"), None, true),
    b("autofix-pr", "Monitor and autofix any issues with the current PR", Sensitive, Some("remote"), None, true),
    b("background", "Send this session to the background and free the terminal", Sensitive, Some("remote"), None, true),
    b("batch", "Plan a large change; background agents each open a PR", Sensitive, Some("remote"), None, true),
    b("branch", "Create a branch of the current conversation at this point", Normal, None, None, true),
    b("btw", "Ask a quick side question without interrupting the main conversation", Normal, None, None, true),
    b("bug", "Report a bug or share your conversation", Sensitive, Some("sends data"), None, true),
    b("cd", "Move this session to a new working directory", Sensitive, Some("directories"), None, true),
    b("chrome", "Open Claude in Chrome settings", Sensitive, Some("settings"), None, true),
    b("claude-api", "Build and debug apps that use the Claude API", Normal, None, None, true),
    b("claude-in-chrome", "Let Claude browse and interact with pages in your Chrome", Normal, None, None, true),
    b("clear", "Start a new session with empty context; previous session stays on disk (resumable with /resume)", Normal, None, Some("Starts a new conversation (the old one stays resumable)."), true),
    b("code-review", "Review the current diff or a PR for bugs and cleanups", Normal, None, None, true),
    b("color", "Set the prompt bar color for this session", Normal, None, None, true),
    b("compact", "Free up context by summarizing the conversation so far", Normal, None, None, true),
    b("config", "Open settings", Sensitive, Some("settings"), None, true),
    b("context", "Visualize current context usage as a colored grid", Normal, None, None, true),
    b("copy", "Copy Claude's last response to clipboard (or /copy N for the Nth-latest)", Normal, None, None, true),
    b("dataviz", "Chart and dashboard design guidance", Normal, None, None, true),
    b("debug", "Turn on debug logging and investigate problems", Normal, None, None, true),
    b("deep-research", "Deep research harness — fan-out web searches, fetch sources, adversarially verify claims, synthesize a cited …", Normal, None, None, true),
    b("design", "Make a new Design artifact from a brief", Normal, None, None, true),
    b("design-login", "Authorize design-system access for /design-sync with your claude.ai account", Sensitive, Some("account"), None, true),
    b("design-sync", "Push your design system components to claude.ai/design", Normal, None, None, true),
    b("desktop", "Continue the current session in Claude Desktop", Sensitive, Some("remote"), None, true),
    b("diff", "Toggle the diff panel showing uncommitted changes", Normal, None, None, true),
    b("doctor", "Health-check your setup and fix issues: installation, unused extensions, duplicated or bloated memory files, …", Sensitive, Some("settings"), None, true),
    b("effort", "Set effort level for model usage", Sensitive, Some("settings"), None, true),
    b("exit", "Exit the CLI", Normal, None, Some("Ends Claude; the pane goes back to a shell."), true),
    b("export", "Export the current conversation to a file or clipboard", Normal, None, None, true),
    b("fast", "Toggle fast mode (Opus 5.5)", Sensitive, Some("settings"), None, true),
    b("feedback", "Send feedback to Anthropic or report a bug", Sensitive, Some("sends data"), None, true),
    b("fewer-permission-prompts", "Pre-approve safe read-only commands based on your usage", Sensitive, Some("permissions"), None, true),
    b("focus", "Toggle focus view: just your prompt, summary, and response", Normal, None, None, true),
    b("fork", "Copy this conversation into a new background session and keep working here", Sensitive, Some("remote"), None, true),
    b("goal", "Set a goal Claude checks before stopping", Normal, None, None, true),
    b("help", "Show help and available commands", Normal, None, None, true),
    b("hooks", "View hook configurations for tool events", Sensitive, Some("hooks"), None, true),
    b("ide", "Manage IDE integrations and show status", Normal, None, None, true),
    b("import", "Import config from another AI coding agent", Sensitive, Some("settings"), None, true),
    b("init", "Initialize a new CLAUDE.md file with codebase documentation", Normal, None, None, true),
    b("insights", "Generate a report analyzing your Claude Code sessions", Normal, None, None, true),
    b("install-github-app", "Set up Claude GitHub Actions for a repository", Sensitive, Some("account"), None, true),
    b("install-slack-app", "Install the Claude Slack app", Sensitive, Some("account"), None, true),
    b("keybindings", "Open your keyboard shortcuts file", Sensitive, Some("settings"), None, true),
    b("list-agents", "List subagents, teammates, and other Claude sessions you can message", Normal, None, None, true),
    b("login", "Sign in with your Anthropic account", Sensitive, Some("account"), None, true),
    b("logout", "Sign out from your Anthropic account", Sensitive, Some("account"), None, true),
    b("loop", "Repeat a prompt or command on an interval (e.g. /loop 5m /foo)", Normal, None, None, true),
    b("mcp", "Manage MCP servers", Sensitive, Some("MCP"), None, true),
    b("memory", "Edit CLAUDE.md files and memory settings", Sensitive, Some("memory files"), None, true),
    b("mobile", "Show QR code to download the Claude mobile app", Normal, None, None, true),
    b("model", "Set the AI model for Claude Code", Normal, None, None, false),
    b("output-style", "List output styles or switch to one", Sensitive, Some("settings"), None, true),
    b("passes", "Share a free week of Claude Code with friends and earn usage credits", Sensitive, Some("account"), None, true),
    b("permissions", "Manage allow and deny tool permission rules", Sensitive, Some("permissions"), None, true),
    b("plan", "Enable plan mode or view the current session plan", Normal, None, None, true),
    b("plugin", "Manage Claude Code plugins", Sensitive, Some("plugins"), None, true),
    b("plugin-authoring", "Write a mod: a live pane, view or behaviour change in Claude Code", Normal, None, None, true),
    b("powerup", "Discover Claude Code features through quick interactive lessons", Normal, None, None, true),
    b("privacy-settings", "View and update your privacy settings", Sensitive, Some("account"), None, true),
    b("radio", "Listen to Claude FM lo-fi radio", Normal, None, None, true),
    b("rate-limit-options", "Manage usage limits and upgrade options", Sensitive, Some("account"), None, true),
    b("recap", "Generate a one-line session recap now", Normal, None, None, true),
    b("release-notes", "View release notes", Normal, None, None, true),
    b("reload-plugins", "Activate pending plugin changes in the current session", Sensitive, Some("plugins"), None, true),
    b("reload-skills", "Pick up skills added or changed on disk during this session", Normal, None, None, true),
    b("remote-control", "Control this session from your phone or claude.ai/code", Sensitive, Some("remote"), None, true),
    b("remote-env", "Choose the default environment for cloud agents", Sensitive, Some("remote"), None, true),
    b("rename", "Rename the current conversation", Normal, None, None, true),
    b("restart", "Restart Claude Code and keep this session", Normal, None, Some("Restarts Claude."), true),
    b("resume", "Resume a previous conversation", Normal, None, Some("Switches this session to another conversation."), true),
    b("rewind", "Restore the code and/or conversation to a previous point", Normal, None, Some("Can restore the code and the conversation to an earlier point."), true),
    b("run", "Launch this project’s app to see your change working", Normal, None, None, true),
    b("run-skill-generator", "Create a skill that knows how to run this project’s app", Normal, None, None, true),
    b("sandbox", "◯ sandbox disabled (⏎ to configure)", Sensitive, Some("permissions"), None, true),
    b("schedule", "Create and manage routines: cloud agents on a schedule", Sensitive, Some("remote"), None, true),
    b("scroll-speed", "Adjust mouse wheel scroll speed", Sensitive, Some("settings"), None, true),
    b("security-review", "Complete a security review of the pending changes on the current branch", Normal, None, None, true),
    b("send-to-project", "Send this session's work to a Claude Project", Sensitive, Some("sends data"), None, true),
    b("simplify", "Clean up the changed code without changing behavior", Normal, None, None, true),
    b("skill-doctor", "Show which loaded skills are unused and costing context", Normal, None, None, true),
    b("skills", "List available skills", Normal, None, None, true),
    b("slides", "Make a new Slides deck artifact from a brief", Normal, None, None, true),
    b("status", "Show Claude Code status including version, model, account, API connectivity, and tool statuses", Normal, None, None, true),
    b("statusline", "Set up Claude Code's status line UI", Sensitive, Some("settings"), None, true),
    b("stickers", "Order Claude Code stickers", Normal, None, None, true),
    b("subtask", "Send a subagent off with your full context; its result comes back here", Normal, None, None, true),
    b("tasks", "View and manage everything running in the background", Normal, None, None, true),
    b("team-onboarding", "Help teammates ramp on Claude Code with a guide from your usage", Normal, None, None, true),
    b("teleport", "Send this session to the cloud, or resume one from claude.ai", Sensitive, Some("remote"), None, true),
    b("terminal-setup", "Install Shift+Enter key binding for newlines", Sensitive, Some("settings"), None, true),
    b("theme", "Change the theme", Sensitive, Some("settings"), None, true),
    b("tui", "Set the terminal UI renderer (default | fullscreen)", Sensitive, Some("settings"), None, true),
    b("ultrareview", "Start a cloud agent that finds and verifies bugs in your branch (~5-15 min, $5-$25 USD) · Runs in a cloud ses…", Sensitive, Some("billed"), None, true),
    b("update-config", "Change settings: hooks, permissions, environment variables", Sensitive, Some("settings"), None, true),
    b("upgrade", "Upgrade your plan for higher rate limits and more Opus", Sensitive, Some("account"), None, true),
    b("usage", "Show session cost, plan usage, and activity stats", Normal, None, None, true),
    b("usage-credits", "Configure usage credits or request them from your admin when you hit a limit", Sensitive, Some("account"), None, true),
    b("verify", "Try your change and report if it works", Normal, None, None, true),
    b("voice", "Toggle voice mode", Normal, None, None, true),
    b("web-setup", "Set up cloud sessions with your GitHub account", Sensitive, Some("account"), None, true),
    b("workflow-authoring", "Load the reference for writing Workflow tool scripts", Normal, None, None, true),
    b("workflows", "Browse running and completed workflows", Normal, None, None, true),
];

pub const CODEX_0_160_1: &[Builtin] = &[
    b("agents", "open the agent command center", Normal, None, None, true),
    b("app", "continue this session in the Desktop app", Sensitive, Some("remote"), None, true),
    b("approve", "approve one retry of a recent auto-review denial", Sensitive, Some("permissions"), None, true),
    b("archive", "archive this session", Normal, None, Some("Archives this session."), true),
    b("cd", "change the current working directory", Sensitive, Some("directories"), None, true),
    b("clear", "clear the terminal and start a new chat", Normal, None, Some("Clears the terminal and starts a new conversation."), true),
    b("compact", "summarize conversation to prevent hitting the context limit", Normal, None, None, true),
    b("copy", "copy the last response or part of it", Normal, None, None, true),
    b("daemon", "Manage the local background server", Sensitive, Some("remote"), None, true),
    b("delete", "permanently delete this session", Sensitive, Some("deletes data"), Some("Permanently deletes this session."), true),
    b("diff", "show git diff (including untracked files)", Normal, None, None, true),
    b("exit", "exit Codex", Normal, None, Some("Ends Codex; the pane goes back to a shell."), true),
    b("experimental", "toggle experimental features", Sensitive, Some("settings"), None, true),
    b("export", "export the conversation as markdown", Normal, None, None, true),
    b("fast", "1.5x speed", Sensitive, Some("settings"), None, true),
    b("feedback", "send logs to maintainers", Sensitive, Some("sends data"), None, true),
    b("fork", "fork the current chat", Normal, None, None, true),
    b("goal", "set or view the goal for a long-running task", Normal, None, None, true),
    b("hooks", "view and manage lifecycle hooks", Sensitive, Some("hooks"), None, true),
    b("ide", "include current selection, open files, and other context from your IDE", Normal, None, None, true),
    b("import", "import setup, this project, and recent chats from Claude Code", Sensitive, Some("settings"), None, true),
    b("init", "create an AGENTS.md file with instructions for Codex", Normal, None, None, true),
    b("keymap", "remap TUI shortcuts", Sensitive, Some("settings"), None, true),
    b("logout", "log out of Codex", Sensitive, Some("account"), None, true),
    b("mcp", "list configured MCP tools; use /mcp verbose for details", Sensitive, Some("MCP"), None, true),
    b("memories", "configure memory use and generation", Sensitive, Some("settings"), None, true),
    b("mention", "mention a file", Normal, None, None, true),
    b("model", "choose what model and reasoning effort to use", Normal, None, None, false),
    b("new", "start a new chat during a conversation", Normal, None, Some("Starts a new conversation."), true),
    b("permissions", "choose what Codex is allowed to do", Sensitive, Some("permissions"), None, true),
    b("pets", "choose or hide the terminal pet", Sensitive, Some("settings"), None, true),
    b("plan", "switch to Plan mode", Normal, None, None, true),
    b("plugins", "browse plugins", Sensitive, Some("plugins"), None, true),
    b("ps", "list background terminals", Normal, None, None, true),
    b("pwd", "show the current working directory", Normal, None, None, true),
    b("raw", "toggle raw scrollback mode for copy-friendly terminal selection", Normal, None, None, true),
    b("recap", "summarize the current conversation now", Normal, None, None, true),
    b("rename", "rename the current thread", Normal, None, None, true),
    b("resume", "resume a saved chat", Normal, None, Some("Switches this session to another conversation."), true),
    b("review", "review my current changes and find issues", Normal, None, None, true),
    b("side", "start a side conversation in an ephemeral fork", Normal, None, None, true),
    b("skills", "use skills to improve how Codex performs specific tasks", Normal, None, None, true),
    b("status", "show current session configuration and token usage", Normal, None, None, true),
    b("statusline", "configure which items appear in the status line", Sensitive, Some("settings"), None, true),
    b("stop", "stop all background terminals", Normal, None, Some("Stops every background terminal."), true),
    b("subagents", "switch between this session's subagents", Normal, None, None, true),
    b("theme", "choose a syntax highlighting theme", Sensitive, Some("settings"), None, true),
    b("title", "configure which items appear in the terminal title", Sensitive, Some("settings"), None, true),
    b("tui", "choose the TUI mode for the next launch", Sensitive, Some("settings"), None, true),
    b("usage", "view account usage or use a usage limit reset", Normal, None, None, true),
    b("vim", "toggle Vim mode for the composer", Normal, None, None, true),
    b("warnings", "view retained warnings and diagnostic details", Normal, None, None, true),
    b("worktree", "start or continue a conversation in a new worktree", Sensitive, Some("remote"), None, true),
];

