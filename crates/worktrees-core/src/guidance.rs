//! Agent guidance: teaching every agent the PLACE paradigm, not just the tools
//! (docs/proposals/agent-guidance.md).
//!
//! Three tiers, all text this BINARY ships — never read from a repository
//! (ADR 0001: nothing a cloned repo contains becomes argv or a prompt it did
//! not already reach through its own AGENTS.md):
//!
//! - [`HEAD`], the rule, in every managed session through the MCP server's
//!   `initialize` instructions (phase 1, `mcp.rs`);
//! - the `worktrees` skill ([`SKILL_MD`]) and the rule ([`rules_text`]),
//!   delivered PER LAUNCH through flags worktrees already builds (phase 2):
//!   Claude `--plugin-dir`, pi `--skill` + `--append-system-prompt`, Codex
//!   `-c developer_instructions` — the last only when the user has none of
//!   their own, because `-c` replaces the key;
//! - an optional Claude PreToolUse guard in the same plugin, OFF by default.
//!
//! Nothing here writes a harness's config. The per-launch files live in a
//! worktrees-owned data dir named by a hash of their content, so a running
//! session's plugin path never changes under it, and the settings live in a
//! worktrees-owned file every launcher reads — the app, the CLI and an MCP
//! `create_worktree` (which runs in the CLI, where an app-memory override
//! would never reach).

use crate::project::Project;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The rule (agent-guidance §3.1). It has to stand alone in 247 chars of ONE
/// line: Codex turns server instructions into a tool-namespace description and,
/// when the tools are deferred, shows only its first line cut to 250 chars, the
/// last 3 of them its own "..." (`MAX_NAMESPACE_DESCRIPTION_CHARS`,
/// 0.157–0.159). "(main)" and not "a checkout you did not create": a lane
/// moving its OWN place between branches is the paradigm, not a breach of it.
pub const HEAD: &str = "Managed by worktrees: every branch lives in its own PLACE (a git worktree \
under .worktrees/ plus a tmux session). Do branch work in a place: create_worktree or `worktrees new \
<branch>`. Never `git worktree add`; never switch branches in (main).";

/// The `worktrees` skill (tier b). Repo-agnostic: a repository's own
/// AGENTS.md still carries its specifics, and the skill says so.
pub const SKILL_MD: &str = include_str!("guidance/SKILL.md");

/// What agents are told, as a version a USER would care about: bumped when the
/// guidance changes enough to show the after-update offer again (the app's
/// `agent-guidance` offer fingerprints on it). A wording fix does not bump it.
/// 3: the "Across projects" section (cross-project P1b).
/// 4: start agents only through a launch, and why; what `unknown` means.
pub const VERSION: u32 = 4;

/// The rule plus where the how-to lives, for the channels that take a prompt
/// rather than a skill (pi's `--append-system-prompt`, Codex's
/// `developer_instructions`). One line, so it survives any first-line cut.
pub fn rules_text() -> String {
    format!(
        "{HEAD} For the how-to (handing work to another agent, messaging between places, finishing \
         and releasing from a place), use the worktrees skill if you have it, or run `worktrees guide`."
    )
}

// ── which repos ────────────────────────────────────────────────────────────

/// Whether `p` is worktrees-managed, so the rule applies to it (decision,
/// 2026-09-30). The cheapest signals visible from here: the repo is in the
/// user's project registry (`registry.rs`, one small file read), a
/// `.worktrees.toml` at the main root (one stat), or a REGISTERED place under
/// `.worktrees/` (one `git worktree list`). A plain directory there is not
/// enough.
pub fn is_managed(p: &Project) -> bool {
    // A repo the user REGISTERED is managed — registering is the act that
    // says so (cross-project §2.2). One small file read.
    crate::registry::read_lenient().by_root(&p.main_root).is_some()
        || Path::new(&p.main_root).join(".worktrees.toml").is_file()
        || p.place_index().iter().any(|pl| !pl.is_main && pl.registered)
}

/// Where a directory sits in a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Where {
    Main,
    Lane { slug: String, branch: Option<String> },
    /// Under `.worktrees/`, but not a worktree git registers.
    Unregistered { slug: String },
    /// A worktree git registers for this repo that is not a place.
    Stray,
    /// In the repo but in no place (`.worktrees/` itself).
    Unplaced,
}

/// Where `path` sits in `p`. A stray is checked FIRST: the place lookup would
/// answer `(main)` for a stray that lies inside the main checkout, because it
/// lies under the main root.
pub fn where_is(p: &Project, path: &Path) -> Where {
    let canon = |q: &Path| std::fs::canonicalize(q).unwrap_or_else(|_| q.to_path_buf());
    let here = canon(path);
    if p.stray_worktrees().iter().any(|s| here.starts_with(canon(Path::new(&s.path)))) {
        return Where::Stray;
    }
    let found = p.place_index().into_iter().filter(|pl| here.starts_with(&pl.path)).max_by_key(|pl| pl.path.len());
    match found {
        Some(pl) if pl.is_main && here.starts_with(p.wt_root_dir()) => Where::Unplaced,
        Some(pl) if pl.is_main => Where::Main,
        Some(pl) if !pl.registered => Where::Unregistered { slug: pl.slug },
        Some(pl) => Where::Lane { slug: pl.slug, branch: pl.branch },
        None => Where::Unplaced,
    }
}

// ── settings ───────────────────────────────────────────────────────────────

/// The user's choices. `enabled` turns per-launch delivery on (default) or
/// off; `guard` adds the PreToolUse hook to Claude's plugin (default off,
/// decision Q2). The MCP instructions are not covered: they are the floor.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub guard: bool,
}

fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Settings { enabled: true, guard: false }
    }
}

/// `~/.config/worktrees/agent-guidance.json` — worktrees-owned, written only
/// by [`save_settings`] (the app's toggle). Unknown keys are not kept: the
/// file holds two booleans and nothing reads it but this module.
pub fn settings_path() -> PathBuf {
    crate::profile::config_root_pub().join("agent-guidance.json")
}

/// The file, then `$WORKTREES_AGENT_GUIDANCE` (`off` / `on`) over `enabled`.
/// An unreadable or malformed file reads as the defaults: guidance on, guard
/// off — the guard never turns itself on.
pub fn settings() -> Settings {
    let file = std::fs::read_to_string(settings_path()).ok();
    settings_from(file.as_deref(), std::env::var("WORKTREES_AGENT_GUIDANCE").ok().as_deref())
}

pub fn settings_from(file: Option<&str>, env: Option<&str>) -> Settings {
    let mut s = file.and_then(|t| serde_json::from_str::<Settings>(t).ok()).unwrap_or_default();
    match env.map(str::trim) {
        Some("off") => s.enabled = false,
        Some("on") => s.enabled = true,
        _ => {}
    }
    s
}

pub fn save_settings(s: &Settings) -> Result<(), String> {
    let path = settings_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let body = serde_json::to_string_pretty(s).map_err(|e| e.to_string())? + "\n";
    let tmp = path.with_extension(format!("json.tmp-{}", std::process::id()));
    std::fs::write(&tmp, body).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
}

// ── the user's edit of the skill ────────────────────────────────────────────

/// Where the user's own copy of the skill lives:
/// `~/.config/worktrees/guidance/` — the user tier, beside the settings file.
/// Never a repository (ADR 0001: a clone must not supply agent instructions
/// this way) and never `ui-state.json` (the frontend writes that whole).
///
/// Two files. `SKILL.md` is the user's text, plain markdown so it can be
/// edited in any editor. `SKILL.base.json` is the SHIPPED text that edit was
/// based on — the text itself, not only its hash, because the binary that
/// later notices the default moved no longer contains the old one, and the
/// three-way compare ("what changed in the default since you forked it") needs
/// it. Only the skill is editable: [`HEAD`] is also the MCP server's
/// instructions, must stay one line inside Codex's 247-char cut, and is what
/// [`guard_bin`] recognises a guard-capable CLI by.
pub fn edit_dir() -> PathBuf {
    crate::profile::config_root_pub().join("guidance")
}

const EDIT_FILE: &str = "SKILL.md";
const BASE_FILE: &str = "SKILL.base.json";
/// Far above any real skill (the shipped one is ~10 KB); a paste of something
/// else is refused rather than handed to every agent.
const EDIT_MAX: usize = 256 * 1024;

/// The shipped text an edit was based on.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Base {
    /// [`VERSION`] when the edit was saved.
    pub version: u32,
    /// [`text_hash`] of `text`. Checked on read: a record whose hash does not
    /// match its own text is treated as missing.
    pub hash: String,
    pub text: String,
}

/// The user's edit, as it stands on disk.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct SkillEdit {
    /// What the file holds, even when it is unusable (so it can be fixed).
    pub text: String,
    /// The default it was based on; `None` when that record is missing or
    /// damaged, which counts as stale — nobody can say it is current.
    pub base: Option<Base>,
    /// The shipped skill has changed since this edit was based on it: the
    /// user should compare before agents keep getting their older fork.
    pub stale: bool,
    /// Why agents get the default instead of this text, when they do.
    pub invalid: Option<String>,
}

/// A content hash for change detection — [`fnv1a`], hex. Not a security
/// boundary: it answers "did the default change", nothing more.
pub fn text_hash(text: &str) -> String {
    format!("{:016x}", fnv1a(&[(String::new(), text.to_string())]))
}

/// Whether `text` can stand in for the skill: frontmatter naming it
/// `worktrees` (the rule tells agents to use "the worktrees skill", and
/// Claude/pi load a skill by its frontmatter) with a description, which is
/// what makes an agent load it at all.
pub fn validate_skill(text: &str) -> Result<(), String> {
    if text.len() > EDIT_MAX {
        return Err(format!("it is {} KB; the limit is {} KB", text.len() / 1024, EDIT_MAX / 1024));
    }
    let body = text.strip_prefix("---\n").ok_or("it must start with a `---` frontmatter line")?;
    let end = body.find("\n---").ok_or("its frontmatter has no closing `---` line")?;
    let front = &body[..end];
    if !front.lines().any(|l| l.trim_end() == "name: worktrees") {
        return Err("its frontmatter must keep `name: worktrees`".into());
    }
    if !front.lines().any(|l| l.strip_prefix("description:").is_some_and(|d| !d.trim().is_empty())) {
        return Err("its frontmatter needs a `description:` (it is what makes an agent load the skill)".into());
    }
    // [`merge_texts`]' own markers: a merge saved with a conflict still in it
    // would hand every agent both versions and a row of `<<<<<<<`.
    if text.lines().any(|l| l.starts_with("<<<<<<< your edit") || l.starts_with(">>>>>>> the new default")) {
        return Err("it still has merge conflict markers (`<<<<<<< your edit`) — keep one side of each".into());
    }
    Ok(())
}

/// The edit in `dir`, judged against `shipped`. `None` when there is none.
pub fn read_edit_in(dir: &Path, shipped: &str) -> Option<SkillEdit> {
    let bytes = match std::fs::read(dir.join(EDIT_FILE)) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            return Some(SkillEdit { text: String::new(), base: None, stale: true, invalid: Some(format!("it could not be read: {e}")) })
        }
    };
    let (text, mut invalid) = match String::from_utf8(bytes) {
        Ok(t) => (t, None),
        Err(e) => (String::from_utf8_lossy(e.as_bytes()).into_owned(), Some("it is not UTF-8 text".to_string())),
    };
    if invalid.is_none() {
        invalid = validate_skill(&text).err();
    }
    let base = std::fs::read_to_string(dir.join(BASE_FILE))
        .ok()
        .and_then(|t| serde_json::from_str::<Base>(&t).ok())
        .filter(|b| b.hash == text_hash(&b.text));
    let stale = base.as_ref().is_none_or(|b| b.hash != text_hash(shipped));
    Some(SkillEdit { text, base, stale, invalid })
}

pub fn read_edit() -> Option<SkillEdit> {
    read_edit_in(&edit_dir(), SKILL_MD)
}

/// What agents get as the skill: a usable edit, else the shipped text.
pub fn effective_skill_in(dir: &Path, shipped: &str) -> String {
    match read_edit_in(dir, shipped) {
        Some(e) if e.invalid.is_none() => e.text,
        _ => shipped.to_string(),
    }
}

pub fn effective_skill() -> String {
    effective_skill_in(&edit_dir(), SKILL_MD)
}

/// Save `text` as the user's skill, based on `shipped` as of [`VERSION`] — or,
/// with `None` or a text equal to `shipped`, go back to the default (both
/// files removed, so "edited" can never mean "identical to the default").
/// Saving the edit you already have is how "keep mine" after an update
/// re-bases it on the new default.
pub fn save_edit_in(dir: &Path, shipped: &str, text: Option<&str>) -> Result<(), String> {
    let remove = |name: &str| match std::fs::remove_file(dir.join(name)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{}: {e}", dir.join(name).display())),
        _ => Ok(()),
    };
    let text = match text {
        Some(t) if t.trim_end() != shipped.trim_end() => t,
        // The edit goes first: an edit without its base reads as stale, never
        // as current, so a failure between the two cannot hide an update.
        _ => return remove(EDIT_FILE).and_then(|()| remove(BASE_FILE)),
    };
    validate_skill(text).map_err(|e| format!("not saved: {e}"))?;
    let mut text = text.to_string();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let base = Base { version: VERSION, hash: text_hash(shipped), text: shipped.to_string() };
    let base = serde_json::to_string_pretty(&base).map_err(|e| e.to_string())? + "\n";
    write_atomic(&dir.join(BASE_FILE), &base)?;
    write_atomic(&dir.join(EDIT_FILE), &text)
}

pub fn save_edit(text: Option<&str>) -> Result<(), String> {
    save_edit_in(&edit_dir(), SKILL_MD, text)
}

fn write_atomic(path: &Path, body: &str) -> Result<(), String> {
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&tmp, body).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// A scratch directory holding `files`, removed on drop — for the two git
/// commands below, which compare FILES.
struct Scratch(PathBuf);

impl Scratch {
    fn new(files: &[(&str, &str)]) -> Result<Scratch, String> {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("wt-guidance-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let s = Scratch(dir);
        for (name, body) in files {
            std::fs::write(s.0.join(name), body).map_err(|e| format!("{name}: {e}"))?;
        }
        Ok(s)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `old` → `new` as git's unified diff at full context — the shape the app's
/// diff viewer already parses (`diff.ts`). Empty when they are the same.
/// `--no-ext-diff`/`--no-color`, because a user's git config can route `diff`
/// through an external tool or force colour, and either would hand the parser
/// something that is not a patch.
pub fn diff_texts(old: &str, new: &str) -> Result<String, String> {
    let s = Scratch::new(&[("old", old), ("new", new)])?;
    let mut cmd = std::process::Command::new("git");
    cmd.args(["diff", "--no-index", "--no-ext-diff", "--no-color", "--unified=1000000", "--", "old", "new"]).current_dir(&s.0);
    let out = crate::proc::run_deadline(cmd, 10).map_err(|e| format!("git diff: {e}"))?;
    match out.status.code() {
        Some(0) => Ok(String::new()),
        Some(1) => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        _ => Err(format!("git diff: {}", String::from_utf8_lossy(&out.stderr).trim())),
    }
}

/// A three-way merge of the user's edit with the new default, from the default
/// it was based on (`git merge-file`): the edit's changes replayed onto the new
/// text, with conflict markers where both changed the same lines. A STARTING
/// POINT for the editor, never saved by itself.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Merged {
    pub text: String,
    pub conflicts: u32,
}

pub fn merge_texts(yours: &str, base: &str, new: &str) -> Result<Merged, String> {
    let s = Scratch::new(&[("yours", yours), ("base", base), ("new", new)])?;
    let mut cmd = std::process::Command::new("git");
    cmd.args(["merge-file", "-p", "-L", "your edit", "-L", "the default you edited", "-L", "the new default", "yours", "base", "new"])
        .current_dir(&s.0);
    let out = crate::proc::run_deadline(cmd, 10).map_err(|e| format!("git merge-file: {e}"))?;
    // The exit status is the number of conflicts (capped at 127); anything
    // above that, or a signal, is git failing.
    match out.status.code() {
        Some(n @ 0..=127) => Ok(Merged { text: String::from_utf8_lossy(&out.stdout).into_owned(), conflicts: n as u32 }),
        _ => Err(format!("git merge-file: {}", String::from_utf8_lossy(&out.stderr).trim())),
    }
}

/// [`merge_texts`] for the edit on disk. An error when there is nothing to
/// merge: no edit, or no record of what it was based on.
pub fn merge_edit() -> Result<Merged, String> {
    let e = read_edit().ok_or("there is no edited skill to merge")?;
    let b = e.base.ok_or("the record of which default your edit was based on is missing, so there is no three-way merge")?;
    merge_texts(&e.text, &b.text, SKILL_MD)
}

// ── per-launch files ────────────────────────────────────────────────────────

/// The materialised per-launch files, in one directory named by a hash of
/// their content (`$XDG_DATA_HOME/worktrees/agent/<hash>/`). Content-addressed
/// so a running session's `--plugin-dir` never changes under it: a new text,
/// or a moved worktrees binary in the guard's hook, is a new directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Materialized {
    pub dir: PathBuf,
    /// pi `--skill`: a directory holding `SKILL.md`.
    pub skill: PathBuf,
    /// pi `--append-system-prompt`: [`rules_text`].
    pub rules: PathBuf,
    /// Claude `--plugin-dir`: the skill, no hooks.
    pub claude_plugin: PathBuf,
    /// Claude `--plugin-dir` with the PreToolUse guard. `None` when no
    /// `worktrees` binary could be found for the hook to run.
    pub claude_guard_plugin: Option<PathBuf>,
}

/// `$XDG_DATA_HOME/worktrees` (the skill store's parent).
pub fn data_root() -> PathBuf {
    let store = crate::profile::skills_store_root();
    store.parent().map(Path::to_path_buf).unwrap_or(store)
}

/// The per-launch files for the skill agents get now ([`effective_skill`]):
/// an edit is a new directory, so a session already running keeps the text it
/// launched with.
pub fn materialize() -> Result<Materialized, String> {
    materialize_in(&data_root(), guard_bin().as_deref(), &effective_skill())
}

/// The `worktrees` the guard's hook would run — only when it HAS the guard. An
/// older CLI on PATH (the app links core in-process, so this is not the app)
/// would answer every Bash call with an error; `guide --rules` exists exactly
/// when `guard` does, and answers from any directory.
pub fn guard_bin() -> Option<PathBuf> {
    let bin = crate::profile::worktrees_bin()?;
    let mut cmd = std::process::Command::new(&bin);
    cmd.args(["guide", "--rules"]);
    let out = crate::proc::run_deadline(cmd, 5).ok()?;
    (out.status.success() && String::from_utf8_lossy(&out.stdout).starts_with("Managed by worktrees:")).then_some(bin)
}

pub fn materialize_in(data_root: &Path, bin: Option<&Path>, skill: &str) -> Result<Materialized, String> {
    let plugin_json = serde_json::to_string_pretty(&serde_json::json!({
        "name": "worktrees",
        "description": "How to work in a worktrees-managed repository: places, lanes, briefs, messaging.",
        "version": env!("CARGO_PKG_VERSION"),
    }))
    .map_err(|e| e.to_string())?;
    let mut files: Vec<(String, String)> = vec![
        ("skills/worktrees/SKILL.md".into(), skill.into()),
        ("rules.md".into(), format!("{}\n", rules_text())),
        ("claude/.claude-plugin/plugin.json".into(), plugin_json.clone()),
        ("claude/skills/worktrees/SKILL.md".into(), skill.into()),
    ];
    if let Some(bin) = bin {
        let command = format!("{} guard pretooluse", crate::profile::shell_quote(&bin.to_string_lossy()));
        let hooks = serde_json::to_string_pretty(&serde_json::json!({
            "hooks": { "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": command }] }] }
        }))
        .map_err(|e| e.to_string())?;
        files.push(("claude-guard/.claude-plugin/plugin.json".into(), plugin_json));
        files.push(("claude-guard/skills/worktrees/SKILL.md".into(), skill.into()));
        files.push(("claude-guard/hooks/hooks.json".into(), hooks));
    }
    let dir = data_root.join("agent").join(format!("{:016x}", fnv1a(&files)));
    if !dir.join(".complete").is_file() {
        // Build beside it and rename into place, so a reader never sees half a
        // plugin; losing the race to a parallel launch is fine — same content.
        let tmp = data_root.join("agent").join(format!(".tmp-{}-{}", std::process::id(), fnv1a(&files)));
        let _ = std::fs::remove_dir_all(&tmp);
        for (rel, body) in &files {
            let f = tmp.join(rel);
            if let Some(parent) = f.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            std::fs::write(&f, body).map_err(|e| format!("{}: {e}", f.display()))?;
        }
        std::fs::write(tmp.join(".complete"), "").map_err(|e| e.to_string())?;
        if std::fs::rename(&tmp, &dir).is_err() {
            let _ = std::fs::remove_dir_all(&tmp);
            if !dir.join(".complete").is_file() {
                return Err(format!("could not write {}", dir.display()));
            }
        }
    }
    Ok(Materialized {
        skill: dir.join("skills/worktrees"),
        rules: dir.join("rules.md"),
        claude_plugin: dir.join("claude"),
        claude_guard_plugin: bin.map(|_| dir.join("claude-guard")),
        dir,
    })
}

/// FNV-1a over every (path, content): stable across Rust versions, which
/// `DefaultHasher` does not promise, and a directory name must be.
fn fnv1a(files: &[(String, String)]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (a, b) in files {
        for byte in a.bytes().chain([0u8]).chain(b.bytes()).chain([0u8]) {
            h ^= u64::from(byte);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

// ── Codex: does the user have their own developer_instructions? ─────────────

/// The tag-wrapped blocks Codex itself puts in its first `developer` message
/// (observed with `codex debug prompt-input`, 0.159.0). A user's own
/// `developer_instructions` arrive as plain text AHEAD of them.
const CODEX_OWN_BLOCKS: &[&str] = &[
    "<skills_instructions>",
    "<permissions instructions>",
    "<collaboration_mode>",
    "<apps_instructions>",
    "<environment_context>",
];

/// From `codex debug prompt-input`'s JSON: `Some(true)` when the first
/// developer message opens with text Codex did not write itself — the user's
/// own `developer_instructions` — `Some(false)` when it opens with one of
/// Codex's own blocks, `None` when the shape is not one we know. A caller
/// treats `None` like `Some(true)`: an unfamiliar Codex means "do not
/// override", never "clobber".
pub fn codex_has_own_instructions(prompt_input: &str) -> Option<bool> {
    let v: serde_json::Value = serde_json::from_str(prompt_input).ok()?;
    let first = v.as_array()?.iter().find(|m| m["role"] == "developer")?;
    let text = first["content"].as_array()?.iter().find_map(|c| c["text"].as_str())?.trim_start();
    if CODEX_OWN_BLOCKS.iter().any(|b| text.starts_with(b)) {
        Some(false)
    } else if text.starts_with('<') {
        None
    } else {
        Some(true)
    }
}

/// Run `codex debug prompt-input` in `dir` and read the answer. It renders the
/// prompt locally and makes no model call — but it STARTS every configured MCP
/// server (measured: a marker server was launched, ~1 s against ~0.12 s with
/// none), so every server we can see configured is switched off for the probe
/// (`-c mcp_servers.<name>.enabled=false`; `mcp_servers={}` does not work), and
/// the probe runs only at a Codex LAUNCH, cached ([`codex_own_for_launch`]) —
/// never from a status screen, `doctor`, or a poll. In the PLACE, so a repo's
/// own `.codex/config.toml` counts when Codex trusts the project. Every failure
/// is `None`.
pub fn probe_codex(dir: &str) -> Option<bool> {
    let bin = crate::profile::codex_bin()?;
    let mut cmd = std::process::Command::new(bin);
    cmd.args(["debug", "prompt-input"]).current_dir(dir);
    for name in codex_mcp_server_names(dir) {
        cmd.arg("-c").arg(format!("mcp_servers.{}.enabled=false", toml_key(&name)));
    }
    let out = crate::proc::run_deadline(cmd, 10).ok()?;
    if !out.status.success() {
        return None;
    }
    codex_has_own_instructions(&String::from_utf8_lossy(&out.stdout))
}

/// `$CODEX_HOME` (default `~/.codex`).
fn codex_home() -> PathBuf {
    std::env::var("CODEX_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".codex"))
}

/// The MCP server names in Codex's user config and the place's project config
/// — READ only (the `~/.claude.json` rule applies to every harness's files).
fn codex_mcp_server_names(dir: &str) -> Vec<String> {
    let mut names = Vec::new();
    for f in [codex_home().join("config.toml"), Path::new(dir).join(".codex/config.toml")] {
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        let Ok(v) = text.parse::<toml::Table>() else { continue };
        if let Some(servers) = v.get("mcp_servers").and_then(|s| s.as_table()) {
            names.extend(servers.keys().filter(|k| !names.contains(*k)).cloned().collect::<Vec<_>>());
        }
    }
    names
}

/// A TOML dotted-key segment: bare when it can be, quoted otherwise.
fn toml_key(k: &str) -> String {
    if !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        k.to_string()
    } else {
        format!("\"{}\"", k.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// What a cached Codex answer depended on: the place, and the modification
/// times of everything the probe read that a user edits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexKey {
    pub place: String,
    pub codex_cfg: Option<i64>,
    pub place_cfg: Option<i64>,
    pub bin: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexEntry {
    pub key: CodexKey,
    pub own: Option<bool>,
    pub at: i64,
}

fn mtime(p: &Path) -> Option<i64> {
    let m = std::fs::metadata(p).ok()?.modified().ok()?;
    m.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs() as i64)
}

fn codex_key(place: &str) -> CodexKey {
    CodexKey {
        place: place.to_string(),
        codex_cfg: mtime(&codex_home().join("config.toml")),
        place_cfg: mtime(&Path::new(place).join(".codex/config.toml")),
        bin: crate::profile::codex_bin().as_deref().and_then(mtime),
    }
}

/// The cached answer for exactly this key, or `None` when it must be probed.
pub fn cached(entries: &[CodexEntry], key: &CodexKey) -> Option<Option<bool>> {
    entries.iter().find(|e| &e.key == key).map(|e| e.own)
}

/// `entries` with `e` replacing whatever its place had.
pub fn store(mut entries: Vec<CodexEntry>, e: CodexEntry) -> Vec<CodexEntry> {
    entries.retain(|x| x.key.place != e.key.place);
    entries.push(e);
    entries
}

pub fn newest(entries: &[CodexEntry]) -> Option<&CodexEntry> {
    entries.iter().max_by_key(|e| e.at)
}

/// `$XDG_STATE_HOME/worktrees/codex-instructions.json` — machine-generated,
/// safely deletable.
fn codex_cache_path() -> Option<PathBuf> {
    crate::init::state_dir().map(|d| d.join("codex-instructions.json"))
}

pub fn codex_cache() -> Vec<CodexEntry> {
    codex_cache_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// The answer a Codex launch in `place` acts on: cached while nothing it
/// depended on has changed, probed (and stored) otherwise.
pub fn codex_own_for_launch(place: &str) -> Option<bool> {
    let key = codex_key(place);
    let entries = codex_cache();
    if let Some(own) = cached(&entries, &key) {
        return own;
    }
    let own = probe_codex(place);
    if let Some(path) = codex_cache_path() {
        let next = store(entries, CodexEntry { key, own, at: crate::sysclock::now_epoch() });
        if let (Some(dir), Ok(body)) = (path.parent(), serde_json::to_string_pretty(&next)) {
            let _ = std::fs::create_dir_all(dir);
            let tmp = path.with_extension(format!("json.tmp-{}", std::process::id()));
            if std::fs::write(&tmp, body).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }
    own
}

// ── what a launch gets ──────────────────────────────────────────────────────

/// One harness's per-launch delivery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum Delivery {
    /// Delivered: these words go after the executable, already shell-quoted.
    On { flags: Vec<String> },
    /// The user turned guidance off.
    Off,
    /// Not delivered, and why (Codex with the user's own instructions, an
    /// unreadable Codex, files that could not be written).
    Skipped { reason: String },
    /// Codex, before any launch has asked it: decided at its next launch.
    Unchecked,
}

/// The words for one launch of `harness` (`provider::*.id`). `codex_own` is
/// [`codex_has_own_instructions`]'s answer, and only Codex reads it.
pub fn delivery(harness: &str, m: &Materialized, s: &Settings, codex_own: Option<bool>) -> Delivery {
    use crate::profile::shell_quote;
    let q = |p: &Path| shell_quote(&p.to_string_lossy());
    let known = [crate::provider::CLAUDE.id, crate::provider::CODEX.id, crate::provider::PI.id];
    if !known.contains(&harness) {
        return Delivery::Skipped { reason: "not a harness worktrees knows".into() };
    }
    if !s.enabled {
        return Delivery::Off;
    }
    if harness == crate::provider::CLAUDE.id {
        let plugin = if s.guard { m.claude_guard_plugin.as_deref().unwrap_or(&m.claude_plugin) } else { &m.claude_plugin };
        Delivery::On { flags: vec!["--plugin-dir".into(), q(plugin)] }
    } else if harness == crate::provider::PI.id {
        Delivery::On { flags: vec!["--skill".into(), q(&m.skill), "--append-system-prompt".into(), q(&m.rules)] }
    } else {
        match codex_own {
            Some(false) => {
                let toml = format!("\"{}\"", rules_text().replace('\\', "\\\\").replace('"', "\\\""));
                Delivery::On { flags: vec!["-c".into(), shell_quote(&format!("developer_instructions={toml}"))] }
            }
            Some(true) => Delivery::Skipped {
                reason: "you set your own developer_instructions, and -c would replace them".into(),
            },
            None => Delivery::Skipped { reason: "could not tell whether you set your own developer_instructions".into() },
        }
    }
}

// ── the guard ──────────────────────────────────────────────────────────────

/// Why a Bash command is refused, or `None`. `is_main(dir)` answers whether a
/// directory is the project's `(main)`. Deliberately narrow — it parses only
/// what it must, and anything it cannot read is allowed:
///
/// - `git worktree add` is refused anywhere;
/// - `git checkout -b|-B` and `git switch -c|-C|--create|--force-create` are
///   refused only where the command runs in `(main)`. Moving your OWN place
///   to another branch is the paradigm.
///
/// The directory a git command runs in follows `cd <dir>` segments and
/// `git -C <dir>`, relative to `cwd`.
pub fn guard_verdict(command: &str, cwd: &Path, is_main: &dyn Fn(&Path) -> bool) -> Option<String> {
    let mut dir = cwd.to_path_buf();
    for seg in segments(command) {
        let words = words(&seg);
        let Some(first) = words.first() else { continue };
        if first == "cd" {
            if let Some(to) = words.get(1) {
                dir = dir.join(to);
            }
            continue;
        }
        if first != "git" && !first.ends_with("/git") {
            continue;
        }
        // Global options, some of which take a value.
        let mut at = dir.clone();
        let mut i = 1;
        while i < words.len() && words[i].starts_with('-') {
            match words[i].as_str() {
                "-C" => {
                    if let Some(to) = words.get(i + 1) {
                        at = at.join(to);
                    }
                    i += 2;
                }
                "-c" | "--git-dir" | "--work-tree" | "--namespace" => i += 2,
                _ => i += 1,
            }
        }
        let sub = words.get(i).map(String::as_str);
        let rest = words.get(i + 1..).unwrap_or(&[]);
        let has = |flags: &[&str]| rest.iter().any(|w| flags.contains(&w.as_str()));
        match sub {
            Some("worktree") if rest.first().map(String::as_str) == Some("add") => {
                return Some(refusal("`git worktree add` makes a worktree worktrees does not manage: it gets no session, and outside .worktrees/ no tool or app can see it."));
            }
            Some("checkout") if has(&["-b", "-B"]) && is_main(&normalize(&at)) => {
                return Some(refusal("a new branch in (main) moves the base checkout off its branch."));
            }
            Some("switch") if has(&["-c", "-C", "--create", "--force-create"]) && is_main(&normalize(&at)) => {
                return Some(refusal("a new branch in (main) moves the base checkout off its branch."));
            }
            _ => {}
        }
    }
    None
}

fn refusal(why: &str) -> String {
    format!(
        "Refused by the worktrees guard: {why} This repository is managed by worktrees. Create a place \
         with the create_worktree tool (or `worktrees new <branch>`) and work in its directory. The guard \
         is optional: Settings → Agent guidance turns it off."
    )
}

/// `a/./b/../c` → `a/c`, lexically. The guard compares paths it built by
/// joining `cd` targets, which are never canonicalised (the target may not
/// even exist yet).
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Split on `;`, `&&`, `||`, `|` and newlines, outside quotes.
fn segments(command: &str) -> Vec<String> {
    let (mut out, mut cur, mut quote) = (Vec::new(), String::new(), None::<char>);
    let mut it = command.chars().peekable();
    while let Some(c) = it.next() {
        match (quote, c) {
            (Some(q), c) if c == q => {
                quote = None;
                cur.push(c);
            }
            (Some(_), c) => cur.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                cur.push(c);
            }
            (None, ';' | '\n') => out.push(std::mem::take(&mut cur)),
            (None, '&') if it.peek() == Some(&'&') => {
                it.next();
                out.push(std::mem::take(&mut cur));
            }
            (None, '|') => {
                if it.peek() == Some(&'|') {
                    it.next();
                }
                out.push(std::mem::take(&mut cur));
            }
            (None, c) => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// Whitespace-separated words with quotes removed. A quoted string is ONE
/// word, so `echo 'git worktree add'` is `echo` and a string, never git.
fn words(seg: &str) -> Vec<String> {
    let (mut out, mut cur, mut quote, mut any) = (Vec::new(), String::new(), None::<char>, false);
    for c in seg.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                any = true;
            }
            (None, c) if c.is_whitespace() => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            (None, c) => {
                cur.push(c);
                any = true;
            }
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

// ── status, and the two commands ───────────────────────────────────────────

/// One harness, as Settings → Agent guidance and `worktrees guide --status`
/// show it.
#[derive(Debug, Clone, Serialize)]
pub struct HarnessStatus {
    pub id: &'static str,
    pub label: &'static str,
    pub installed: bool,
    #[serde(flatten)]
    pub delivery: Delivery,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    /// [`VERSION`].
    pub version: u32,
    pub settings: Settings,
    /// Whether the guard can be delivered: it needs a `worktrees` CLI that has
    /// it ([`guard_bin`]). Asked for but unavailable, Claude gets the plain
    /// plugin.
    pub guard_available: bool,
    pub settings_path: String,
    /// The materialised directory, when it could be written.
    pub dir: Option<String>,
    pub error: Option<String>,
    pub harnesses: Vec<HarnessStatus>,
    /// What every per-launch session gets as its skill, verbatim: the user's
    /// edit when there is a usable one, else the default.
    pub skill: String,
    /// The skill this build ships.
    pub skill_default: &'static str,
    /// [`text_hash`] of `skill_default` — what an edit's base is compared
    /// with, and the app's `agent-guidance-changed` offer fingerprint.
    pub skill_hash: String,
    /// The user's edit, when there is one ([`read_edit`]).
    pub skill_edit: Option<SkillEdit>,
    pub edit_path: String,
    /// What pi and Codex get as their rule, verbatim.
    pub rules: String,
}

/// The machine-level picture. It never probes Codex (that starts its MCP
/// servers): Codex's line is the answer its most recent launch acted on, or
/// [`Delivery::Unchecked`] before any.
pub fn status() -> Status {
    let s = settings();
    let (m, error) = match materialize() {
        Ok(m) => (Some(m), None),
        Err(e) => (None, Some(e)),
    };
    let last = codex_cache();
    let harnesses = [crate::provider::CLAUDE, crate::provider::CODEX, crate::provider::PI]
        .into_iter()
        .map(|pv| {
            let installed = crate::harness::by_id(pv.id).is_some_and(|a| a.installed().is_ok());
            let delivery = match &m {
                None => Delivery::Skipped { reason: error.clone().unwrap_or_default() },
                Some(m) if pv.id == crate::provider::CODEX.id && s.enabled => match newest(&last) {
                    None => Delivery::Unchecked,
                    Some(e) => delivery(pv.id, m, &s, e.own),
                },
                Some(m) => delivery(pv.id, m, &s, None),
            };
            HarnessStatus { id: pv.id, label: pv.label, installed, delivery }
        })
        .collect();
    Status {
        version: VERSION,
        guard_available: m.as_ref().is_some_and(|m| m.claude_guard_plugin.is_some()),
        settings: s,
        settings_path: settings_path().to_string_lossy().into_owned(),
        dir: m.map(|m| m.dir.to_string_lossy().into_owned()),
        error,
        harnesses,
        skill: effective_skill(),
        skill_default: SKILL_MD,
        skill_hash: text_hash(SKILL_MD),
        skill_edit: read_edit(),
        edit_path: edit_dir().join(EDIT_FILE).to_string_lossy().into_owned(),
        rules: rules_text(),
    }
}

/// `worktrees guide [--status [--json]] [--rules] [--default]` — what agents
/// are told: the skill they get (the user's edit when there is a usable one),
/// `--default` the one this build ships. Works anywhere: none of it is a
/// question about the cwd's repository.
pub fn cmd_guide(args: &[String]) -> i32 {
    let has = |f: &str| args.iter().any(|a| a == f);
    if let Some(bad) = args.iter().find(|a| !["--status", "--json", "--rules", "--default"].contains(&a.as_str())) {
        eprintln!("worktrees guide: unknown argument '{bad}' (expected --status, --json, --rules, --default)");
        return 1;
    }
    if has("--rules") {
        println!("{}", rules_text());
        return 0;
    }
    if has("--default") {
        print!("{SKILL_MD}");
        return 0;
    }
    if !has("--status") {
        print!("{}", effective_skill());
        return 0;
    }
    let st = status();
    if has("--json") {
        println!("{}", serde_json::to_string(&st).unwrap_or_default());
        return 0;
    }
    println!(
        "Agent guidance: per-launch delivery {}, guard {}{} ({})",
        if st.settings.enabled { "on" } else { "off" },
        if st.settings.guard { "on" } else { "off" },
        if st.settings.guard && !st.guard_available { " but unavailable: no worktrees CLI with `guard` on PATH" } else { "" },
        st.settings_path
    );
    if let Some(e) = &st.error {
        println!("  files could not be written: {e}");
    }
    println!("  skill:  {}", skill_line(st.skill_edit.as_ref(), &st.edit_path));
    for h in &st.harnesses {
        let what = match &h.delivery {
            Delivery::On { flags } => format!("delivered ({})", flags.first().map(String::as_str).unwrap_or("")),
            Delivery::Off => "off".into(),
            Delivery::Skipped { reason } => format!("skipped: {reason}"),
            Delivery::Unchecked => "decided at its next launch".into(),
        };
        println!("  {:<7} {}{}", h.label, what, if h.installed { "" } else { " (not installed)" });
    }
    0
}

/// The status screen's one line about whose skill agents get.
fn skill_line(edit: Option<&SkillEdit>, path: &str) -> String {
    match edit {
        None => "the default".into(),
        Some(SkillEdit { invalid: Some(why), .. }) => format!("the default — your edit at {path} is not used: {why}"),
        Some(SkillEdit { stale: true, base: Some(b), .. }) => format!(
            "your edit ({path}); the default has changed since you edited v{} — compare in Settings → Agent guidance, or `worktrees guide --default`",
            b.version
        ),
        Some(SkillEdit { stale: true, .. }) => {
            format!("your edit ({path}); which default it was based on is unknown — compare with `worktrees guide --default`")
        }
        Some(_) => format!("your edit ({path})"),
    }
}

/// `worktrees guard pretooluse` — the Claude PreToolUse hook (§5.2). Reads
/// the hook's JSON on stdin and prints a deny decision, or nothing. Every
/// failure ALLOWS: a guard that breaks the session is worse than no guard.
pub fn cmd_guard(args: &[String]) -> i32 {
    use std::io::Read;
    if args.first().map(String::as_str) != Some("pretooluse") {
        eprintln!("worktrees guard: expected `pretooluse`");
        return 0;
    }
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return 0;
    }
    if let Some(out) = pretooluse(&input) {
        println!("{out}");
    }
    0
}

/// The hook's decision for one stdin payload, as the JSON to print.
pub fn pretooluse(input: &str) -> Option<String> {
    pretooluse_with(input, &settings(), &|d| Project::discover(d).ok())
}

/// [`pretooluse`] with its two inputs injected. The settings are read on EVERY
/// call, so turning the guard off takes effect on the next command — the hook
/// lives in the launched plugin, which only a relaunch would change, and the
/// refusal tells the model the toggle exists. A command that names none of
/// `worktree` / `checkout` / `switch` returns before any repository lookup:
/// this runs on every Bash call.
pub fn pretooluse_with(input: &str, s: &Settings, discover: &dyn Fn(&Path) -> Option<Project>) -> Option<String> {
    if !s.guard {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(input).ok()?;
    if v["tool_name"] != "Bash" {
        return None;
    }
    let command = v["tool_input"]["command"].as_str()?;
    if !["worktree", "checkout", "switch"].iter().any(|w| command.contains(w)) {
        return None;
    }
    let cwd = v["cwd"].as_str().map(PathBuf::from).or_else(|| std::env::current_dir().ok())?;
    let p = discover(&cwd)?;
    if !is_managed(&p) {
        return None;
    }
    let reason = guard_verdict(command, &cwd, &|d: &Path| where_is(&p, d) == Where::Main)?;
    Some(
        serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": reason,
            }
        })
        .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_head_is_the_rule_in_one_short_line() {
        assert!(HEAD.chars().count() <= 247, "{}", HEAD.chars().count());
        assert!(!HEAD.contains('\n'));
        let r = rules_text();
        assert!(r.starts_with(HEAD) && !r.contains('\n'), "{r}");
        assert!(r.contains("worktrees guide"), "{r}");
    }

    #[test]
    fn the_skill_has_frontmatter_a_trigger_and_no_repo_specifics() {
        assert!(SKILL_MD.starts_with("---\nname: worktrees\ndescription: Use BEFORE any branch work"), "{}", &SKILL_MD[..80]);
        for banned in ["casadelvalle", "/Users/", "bug-fixes", "close-out.md"] {
            assert!(!SKILL_MD.contains(banned), "repo- or machine-specific text in the skill: {banned}");
        }
    }

    #[test]
    fn settings_default_on_with_the_guard_off_and_env_overrides_enabled() {
        assert_eq!(settings_from(None, None), Settings { enabled: true, guard: false });
        assert_eq!(settings_from(Some("not json"), None), Settings { enabled: true, guard: false });
        assert_eq!(settings_from(Some(r#"{"guard": true}"#), None), Settings { enabled: true, guard: true });
        assert_eq!(settings_from(Some(r#"{"enabled": false, "guard": true}"#), None), Settings { enabled: false, guard: true });
        assert_eq!(settings_from(None, Some("off")).enabled, false);
        assert_eq!(settings_from(Some(r#"{"enabled": false}"#), Some("on")).enabled, true);
        // An unknown env value changes nothing.
        assert_eq!(settings_from(Some(r#"{"enabled": false}"#), Some("maybe")).enabled, false);
    }

    fn main_is(root: &'static str) -> impl Fn(&Path) -> bool {
        move |d: &Path| d == Path::new(root)
    }

    #[test]
    fn the_guard_refuses_a_raw_worktree_add_anywhere() {
        let m = main_is("/r");
        for c in [
            "git worktree add ../x -b fix",
            "cd /tmp && git worktree add y",
            "git -C /r worktree add /tmp/z main",
            "git -c core.x=1 worktree   add q",
            "echo hi; git worktree add q",
        ] {
            assert!(guard_verdict(c, Path::new("/r/.worktrees/lane"), &m).is_some(), "{c}");
        }
        for c in ["git worktree list", "git worktree remove x", "echo 'git worktree add'", "git log --grep worktree"] {
            assert!(guard_verdict(c, Path::new("/r"), &m).is_none(), "{c}");
        }
    }

    #[test]
    fn the_guard_refuses_a_new_branch_only_in_main() {
        let m = main_is("/r");
        for c in ["git checkout -b fix", "git switch -c fix", "git switch --create fix", "git checkout -B fix main"] {
            assert!(guard_verdict(c, Path::new("/r"), &m).is_some(), "in (main): {c}");
            assert!(guard_verdict(c, Path::new("/r/.worktrees/lane"), &m).is_none(), "in a lane: {c}");
        }
        // The directory follows `cd` and `-C`.
        assert!(guard_verdict("cd /r && git checkout -b x", Path::new("/r/.worktrees/lane"), &m).is_some());
        assert!(guard_verdict("git -C /r switch -c x", Path::new("/r/.worktrees/lane"), &m).is_some());
        assert!(guard_verdict("cd .worktrees/lane && git checkout -b x", Path::new("/r"), &m).is_none());
        assert!(guard_verdict("git -C .worktrees/lane checkout -b x", Path::new("/r"), &m).is_none());
        // Plain checkouts and switches are not new branches.
        for c in ["git checkout main", "git switch main", "git checkout -- file.txt"] {
            assert!(guard_verdict(c, Path::new("/r"), &m).is_none(), "{c}");
        }
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wt-guidance-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn materialising_writes_a_content_named_dir_once() {
        let root = tmp("mat");
        let m = materialize_in(&root, Some(Path::new("/opt/wt bin/worktrees")), SKILL_MD).expect("materialised");
        assert!(m.dir.starts_with(root.join("agent")), "{}", m.dir.display());
        assert_eq!(std::fs::read_to_string(m.skill.join("SKILL.md")).unwrap(), SKILL_MD);
        assert_eq!(std::fs::read_to_string(&m.rules).unwrap().trim_end(), rules_text());
        assert_eq!(std::fs::read_to_string(m.claude_plugin.join("skills/worktrees/SKILL.md")).unwrap(), SKILL_MD);
        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(m.claude_plugin.join(".claude-plugin/plugin.json")).unwrap()).unwrap();
        assert_eq!(manifest["name"], "worktrees");
        assert!(!m.claude_plugin.join("hooks").exists(), "the plain plugin carries no hook");
        let guard = m.claude_guard_plugin.clone().expect("a guard variant when the binary is known");
        let hooks = std::fs::read_to_string(guard.join("hooks/hooks.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&hooks).unwrap();
        let cmd = v["hooks"]["PreToolUse"][0]["hooks"][0]["command"].as_str().unwrap();
        assert_eq!(cmd, "'/opt/wt bin/worktrees' guard pretooluse", "the binary path is shell-quoted");
        assert_eq!(v["hooks"]["PreToolUse"][0]["matcher"], "Bash");

        // Same content, same directory; a different hook binary, another one.
        assert_eq!(materialize_in(&root, Some(Path::new("/opt/wt bin/worktrees")), SKILL_MD).unwrap(), m);
        let other = materialize_in(&root, Some(Path::new("/elsewhere/worktrees")), SKILL_MD).unwrap();
        assert_ne!(other.dir, m.dir);
        // No binary: no guard variant, and the rest unchanged in shape.
        assert!(materialize_in(&root, None, SKILL_MD).unwrap().claude_guard_plugin.is_none());
        // An edited skill is a new directory — a running session's plugin
        // keeps the text it launched with — and every copy carries the edit.
        let edited = SKILL_MD.replace("# Working in", "# Edited: working in");
        let e = materialize_in(&root, Some(Path::new("/opt/wt bin/worktrees")), &edited).unwrap();
        assert_ne!(e.dir, m.dir);
        for f in [e.skill.join("SKILL.md"), e.claude_plugin.join("skills/worktrees/SKILL.md"), e.claude_guard_plugin.unwrap().join("skills/worktrees/SKILL.md")] {
            assert_eq!(std::fs::read_to_string(&f).unwrap(), edited, "{}", f.display());
        }
        assert_eq!(std::fs::read_to_string(m.skill.join("SKILL.md")).unwrap(), SKILL_MD, "the old directory is untouched");
        let _ = std::fs::remove_dir_all(&root);
    }

    const OLD: &str = "---\nname: worktrees\ndescription: Use it.\n---\n\n# Places\n\nOne.\nTwo.\nThree.\n";
    const NEW: &str = "---\nname: worktrees\ndescription: Use it.\n---\n\n# Places\n\nOne.\nTwo, improved.\nThree.\n";

    #[test]
    fn no_edit_means_the_shipped_skill() {
        let dir = tmp("noedit");
        assert_eq!(read_edit_in(&dir, NEW), None);
        assert_eq!(effective_skill_in(&dir, NEW), NEW);
    }

    #[test]
    fn an_edit_wins_and_records_the_default_it_was_based_on() {
        let dir = tmp("edit");
        let mine = OLD.replace("One.", "One, my way.");
        save_edit_in(&dir, OLD, Some(&mine)).unwrap();
        assert_eq!(effective_skill_in(&dir, OLD), mine);
        let e = read_edit_in(&dir, OLD).unwrap();
        assert_eq!(e.base, Some(Base { version: VERSION, hash: text_hash(OLD), text: OLD.into() }));
        assert!(!e.stale && e.invalid.is_none(), "{e:?}");
        // The binary updates and ships NEW: still the user's text, now stale,
        // with the OLD default kept for the three-way compare.
        let e = read_edit_in(&dir, NEW).unwrap();
        assert!(e.stale, "a changed default must be noticed");
        assert_eq!(e.base.unwrap().text, OLD);
        assert_eq!(effective_skill_in(&dir, NEW), mine, "agents keep the user's text until they choose");
        // "Keep mine": saving the same text again re-bases it on the new default.
        save_edit_in(&dir, NEW, Some(&mine)).unwrap();
        let e = read_edit_in(&dir, NEW).unwrap();
        assert!(!e.stale);
        assert_eq!(e.base.unwrap().hash, text_hash(NEW));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn saving_the_default_or_nothing_is_a_reset() {
        let dir = tmp("reset");
        save_edit_in(&dir, OLD, Some(&OLD.replace("Two.", "2."))).unwrap();
        save_edit_in(&dir, OLD, None).unwrap();
        assert_eq!(read_edit_in(&dir, OLD), None);
        assert!(!dir.join(BASE_FILE).exists(), "a reset leaves no base behind");
        save_edit_in(&dir, OLD, Some(&OLD.replace("Two.", "2."))).unwrap();
        // The default itself, give or take a trailing newline, is not an edit.
        save_edit_in(&dir, OLD, Some(OLD.trim_end())).unwrap();
        assert_eq!(read_edit_in(&dir, OLD), None, "an edit identical to the default must not read as modified");
        // Resetting with nothing there is fine.
        save_edit_in(&dir, OLD, None).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unusable_edit_falls_back_to_the_default_and_says_why() {
        let dir = tmp("bad");
        std::fs::create_dir_all(&dir).unwrap();
        for (body, why) in [
            ("no frontmatter at all\n", "frontmatter"),
            ("---\nname: other\ndescription: x\n---\nbody\n", "name: worktrees"),
            ("---\nname: worktrees\n---\nbody\n", "description"),
            ("---\nname: worktrees\ndescription: x\nbody, never closed\n", "closing"),
        ] {
            std::fs::write(dir.join(EDIT_FILE), body).unwrap();
            let e = read_edit_in(&dir, NEW).unwrap();
            assert!(e.invalid.as_deref().is_some_and(|w| w.contains(why)), "{body:?}: {e:?}");
            assert_eq!(e.text, body, "the broken text is still shown, so it can be fixed");
            assert_eq!(effective_skill_in(&dir, NEW), NEW, "{body:?}");
            // And it is never SAVED in the first place.
            assert!(save_edit_in(&dir, NEW, Some(body)).is_err(), "{body:?}");
        }
        std::fs::write(dir.join(EDIT_FILE), [0xff, 0xfe, b'x']).unwrap();
        assert!(read_edit_in(&dir, NEW).unwrap().invalid.is_some());
        assert_eq!(effective_skill_in(&dir, NEW), NEW);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_or_damaged_base_reads_as_stale() {
        let dir = tmp("nobase");
        let mine = NEW.replace("One.", "Uno.");
        save_edit_in(&dir, NEW, Some(&mine)).unwrap();
        std::fs::remove_file(dir.join(BASE_FILE)).unwrap();
        let e = read_edit_in(&dir, NEW).unwrap();
        assert!(e.stale && e.base.is_none() && e.invalid.is_none(), "{e:?}");
        assert_eq!(effective_skill_in(&dir, NEW), mine, "a lost base does not discard the edit");
        // A record whose hash does not match its own text is not believed.
        let forged = Base { version: VERSION, hash: text_hash(NEW), text: OLD.into() };
        std::fs::write(dir.join(BASE_FILE), serde_json::to_string(&forged).unwrap()).unwrap();
        assert!(read_edit_in(&dir, NEW).unwrap().base.is_none());
        std::fs::write(dir.join(BASE_FILE), "{not json").unwrap();
        assert!(read_edit_in(&dir, NEW).unwrap().stale);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_shipped_skill_is_a_valid_edit() {
        // The editor starts from the default; it must pass its own check.
        validate_skill(SKILL_MD).unwrap();
    }

    #[test]
    fn diff_and_merge_go_through_git() {
        assert_eq!(diff_texts(OLD, OLD).unwrap(), "");
        let patch = diff_texts(OLD, NEW).unwrap();
        assert!(patch.contains("\n-Two.\n+Two, improved.\n"), "{patch}");
        // Full context: every line of both sides is in the patch.
        assert!(patch.contains("\n # Places\n") && patch.contains("\n Three.\n"), "{patch}");
        // The user's change and the default's change touch different lines.
        let mine = OLD.replace("Use it.", "Use it well.");
        let m = merge_texts(&mine, OLD, NEW).unwrap();
        assert_eq!(m, Merged { text: NEW.replace("Use it.", "Use it well."), conflicts: 0 });
        // Both changed the same line: markers, counted.
        let clash = OLD.replace("Two.", "Two, mine.");
        let m = merge_texts(&clash, OLD, NEW).unwrap();
        assert_eq!(m.conflicts, 1);
        assert!(m.text.contains("<<<<<<< your edit") && m.text.contains(">>>>>>> the new default"), "{}", m.text);
        // A merge with a conflict left in it is not a skill.
        assert!(validate_skill(&m.text).is_err_and(|e| e.contains("conflict")), "{}", m.text);
    }

    /// Shapes observed with `codex debug prompt-input` on 0.159.0, trimmed to
    /// what the parser reads.
    fn prompt_input(first_dev_texts: &[&str]) -> String {
        let content: Vec<_> = first_dev_texts.iter().map(|t| serde_json::json!({ "type": "input_text", "text": t })).collect();
        serde_json::json!([
            { "type": "message", "role": "developer", "content": content },
            { "type": "message", "role": "developer", "content": [{ "type": "input_text", "text": "<other>" }] },
            { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "# AGENTS.md" }] },
        ])
        .to_string()
    }

    #[test]
    fn codex_detection_reads_the_first_developer_text() {
        let none = prompt_input(&["<skills_instructions>\n## Skills", "<permissions instructions>\nFilesystem", "<collaboration_mode># C"]);
        assert_eq!(codex_has_own_instructions(&none), Some(false));
        let own = prompt_input(&["Always answer in French.", "<skills_instructions>\n## Skills"]);
        assert_eq!(codex_has_own_instructions(&own), Some(true));
        // Unknown shapes are None — which the caller treats as "do not override".
        assert_eq!(codex_has_own_instructions("not json"), None);
        assert_eq!(codex_has_own_instructions("[]"), None);
        assert_eq!(codex_has_own_instructions(&prompt_input(&["<some_future_block>"])), None);
    }

    fn fake_m() -> Materialized {
        Materialized {
            dir: PathBuf::from("/d"),
            skill: PathBuf::from("/d/skills/worktrees"),
            rules: PathBuf::from("/d/rules.md"),
            claude_plugin: PathBuf::from("/d/claude"),
            claude_guard_plugin: Some(PathBuf::from("/d/claude-guard")),
        }
    }

    #[test]
    fn each_harness_gets_its_own_flags() {
        let m = fake_m();
        let on = Settings::default();
        let guarded = Settings { enabled: true, guard: true };
        assert_eq!(delivery("claude", &m, &on, None), Delivery::On { flags: vec!["--plugin-dir".into(), "'/d/claude'".into()] });
        assert_eq!(delivery("claude", &m, &guarded, None), Delivery::On { flags: vec!["--plugin-dir".into(), "'/d/claude-guard'".into()] });
        assert_eq!(
            delivery("pi", &m, &on, None),
            Delivery::On { flags: vec!["--skill".into(), "'/d/skills/worktrees'".into(), "--append-system-prompt".into(), "'/d/rules.md'".into()] }
        );
        match delivery("codex", &m, &on, Some(false)) {
            Delivery::On { flags } => {
                assert_eq!(flags.len(), 2);
                assert_eq!(flags[0], "-c");
                assert!(flags[1].starts_with("'developer_instructions=\"Managed by worktrees:"), "{}", flags[1]);
                assert!(flags[1].contains("worktrees guide"), "{}", flags[1]);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(delivery("codex", &m, &on, Some(true)), Delivery::Skipped { .. }));
        assert!(matches!(delivery("codex", &m, &on, None), Delivery::Skipped { .. }));
        let off = Settings { enabled: false, guard: true };
        for h in ["claude", "pi", "codex"] {
            assert_eq!(delivery(h, &m, &off, Some(false)), Delivery::Off, "{h}");
        }
        // A guard asked for but impossible: the plain plugin, never nothing.
        let no_guard = Materialized { claude_guard_plugin: None, ..fake_m() };
        assert_eq!(delivery("claude", &no_guard, &guarded, None), Delivery::On { flags: vec!["--plugin-dir".into(), "'/d/claude'".into()] });
        // Anything else (a custom --ai tool) gets nothing.
        assert_eq!(delivery("aider", &m, &on, None), Delivery::Skipped { reason: "not a harness worktrees knows".into() });
    }

    #[test]
    fn delivery_knows_every_registered_harness() {
        // A new harness in provider::PROVIDERS must be given a guidance channel
        // here (or a deliberate Skipped) — docs/adding-a-harness.md says so.
        let m = fake_m();
        for p in crate::provider::PROVIDERS {
            assert_ne!(
                delivery(p.id, &m, &Settings::default(), Some(false)),
                Delivery::Skipped { reason: "not a harness worktrees knows".into() },
                "{} has no agent-guidance channel",
                p.id
            );
        }
    }

    #[test]
    fn the_codex_cache_answers_only_while_nothing_it_read_has_changed() {
        let key = CodexKey { place: "/r/.worktrees/a".into(), codex_cfg: Some(10), place_cfg: None, bin: Some(5) };
        let entries = vec![CodexEntry { key: key.clone(), own: Some(true), at: 100 }];
        assert_eq!(cached(&entries, &key), Some(Some(true)));
        let touched = CodexKey { codex_cfg: Some(11), ..key.clone() };
        assert_eq!(cached(&entries, &touched), None, "an edited ~/.codex/config.toml must re-probe");
        let repo_cfg = CodexKey { place_cfg: Some(1), ..key.clone() };
        assert_eq!(cached(&entries, &repo_cfg), None, "a new .codex/config.toml in the place must re-probe");
        let other = CodexKey { place: "/r".into(), ..key.clone() };
        assert_eq!(cached(&entries, &other), None);
        // Storing replaces the place's old entry rather than growing forever.
        let next = store(entries, CodexEntry { key: touched.clone(), own: Some(false), at: 200 });
        assert_eq!(next.len(), 1);
        assert_eq!(cached(&next, &touched), Some(Some(false)));
        assert_eq!(newest(&next).map(|e| e.own), Some(Some(false)));
    }

    #[test]
    fn the_guard_is_live_and_cheap() {
        // Off in settings: allowed, without ever looking at a repository —
        // the toggle takes effect on the next command, not the next launch.
        let input = r#"{"tool_name":"Bash","tool_input":{"command":"git worktree add x"},"cwd":"/nonexistent"}"#;
        let off = Settings { enabled: true, guard: false };
        assert_eq!(pretooluse_with(input, &off, &|_| panic!("discovered a repo with the guard off")), None);
        // On, but a command that cannot be refused: no repository lookup either.
        let on = Settings { enabled: true, guard: true };
        let ls = r#"{"tool_name":"Bash","tool_input":{"command":"ls -la"},"cwd":"/nonexistent"}"#;
        assert_eq!(pretooluse_with(ls, &on, &|_| panic!("discovered a repo for `ls`")), None);
    }

    #[test]
    fn a_refusal_names_the_way_out() {
        let r = guard_verdict("git worktree add x", Path::new("/r"), &main_is("/r")).unwrap();
        assert!(r.contains("create_worktree") && r.contains("worktrees new"), "{r}");
    }
}
