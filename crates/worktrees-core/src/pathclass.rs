//! What a path in a project IS, for the question "who may read or write it"
//! (Phase 2 proposal §7.1, R7).
//!
//! One classifier, so the write guard, the reads, the tree and the docs index
//! cannot each grow their own idea of which files steer an agent. It answers
//! with a `Class` and nothing else: which CAPABILITY a class needs belongs to
//! the dispatch table's resolver, which owns that type. The one answer that
//! needs no capability at all is here, because it applies to every caller,
//! the desktop included: `Forbidden` is never written
//! (`Class::refuses_every_caller`).
//!
//! **Matching is on the canonical path, folded the way APFS folds.** A
//! `notes.md -> AGENTS.md` link is classed as what it writes, which is why the
//! callers canonicalise first (`classify_canonical`). And the fold is not only
//! ASCII: measured on APFS (macOS 27), a file created as `.claude/ſettings.json`
//! (U+017F) is what `.claude/settings.json` opens, and `canonicalize` reports
//! it under the `ſ` spelling, so an ASCII-only match would class the hook file
//! as plain text. Those two letters (`ſ` → `s`, U+212A KELVIN → `k`) are the
//! only non-ASCII characters whose case folding lands on ASCII.
//!
//! **Every rule matches at any depth**, except the two that name worktrees' own
//! files at a place root. An agent launched in `apps/web` treats that directory
//! as its project, so `apps/web/.claude/settings.json` runs hooks for it as
//! surely as the root one does; and a `.git` anywhere is a repository whose
//! config git will honour (`projcfg::RelPath` refuses any `.git` component for
//! the same reason).
//!
//! **What this does not claim** (§7.4): every file an agent reads can steer it.
//! The classes separate what EXECUTES with nobody choosing to run it, which a
//! device may write only if it can already run code, from the files agents are
//! built to obey, which need `instruct`. Everything else is plain.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// The class of one path, ordered from the least to the most restricted, so
/// that the strictest of several answers is their `max`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Class {
    /// Everything else. Reading needs `read`, writing `write`.
    Plain,
    /// Text an agent is built to obey: `AGENTS.md` and its kin, the agent
    /// config directories, `.planning/`, `.worktrees.toml`. Writing needs
    /// `instruct`.
    Instruction,
    /// Runs with nobody choosing to run it: hooks, MCP server lists,
    /// extensions. Writing needs `stream` — a device that can already run code.
    Executes,
    /// `.env`, `.env.*` and the files a project declares in `[[file]]`. Reading
    /// AND writing need `stream`: a device with a shell can `cat` them anyway.
    Secret,
    /// git's own directory and worktrees' own state. Never written through the
    /// engine by ANY caller, and never read by a device.
    Forbidden,
}

/// Which half of the question is being asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Access {
    Read,
    Write,
}

impl Class {
    pub fn as_str(self) -> &'static str {
        match self {
            Class::Plain => "plain",
            Class::Instruction => "instruction",
            Class::Executes => "executes",
            Class::Secret => "secret",
            Class::Forbidden => "forbidden",
        }
    }

    /// No caller may do `access` here, loopback included. Only a write to a
    /// `Forbidden` path: `core.fsmonitor` in `.git/config` runs at the next git
    /// spawn, and the engine's own poll spawns git every few seconds, so such a
    /// write is code execution with nobody in the loop. A terminal can still
    /// edit `.git/config`; the person at the desk has one.
    pub fn refuses_every_caller(self, access: Access) -> bool {
        self == Class::Forbidden && access == Access::Write
    }

    /// No paired DEVICE may do `access` here, whatever it holds. Reads of
    /// `Forbidden` paths stay open to the loopback caller, which is the person
    /// at the desk.
    pub fn refuses_devices(self, _access: Access) -> bool {
        self == Class::Forbidden
    }
}

/// The fold matching is done in: ASCII lowercase, plus the two non-ASCII
/// letters APFS folds onto ASCII (see the module note).
fn fold(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{17F}' => 's',
            '\u{212A}' => 'k',
            c => c.to_ascii_lowercase(),
        })
        .collect()
}

/// Names that are instructions wherever they sit. The first three are §7.1's;
/// `CLAUDE.local.md` and `AGENTS.override.md` are the per-checkout variants
/// Claude Code and Codex also read, and `.cursorrules` is Cursor's older form
/// of `.cursor/`.
const INSTRUCTION_NAMES: [&str; 7] =
    ["agents.md", "claude.md", "gemini.md", "claude.local.md", "agents.override.md", ".cursorrules", ".worktrees.toml"];

/// Directories whose whole contents are instructions, unless a rule below
/// raises a path inside them to `Executes`.
const INSTRUCTION_DIRS: [&str; 6] = [".planning", ".claude", ".agents", ".codex", ".pi", ".cursor"];

/// worktrees' declared state at a project or place root (`store.rs`). Its
/// setters own it; a raw write would bypass every rule they enforce.
const PLACES_FILE: &str = ".worktrees.places.json";

/// The tool's own tree under a project root (`Project::wt_root`).
const WT_DIR: &str = ".worktrees";

/// The class of `rel`, a path relative to the project root `project_root`,
/// with the project's `[[file]]` declarations read from its `.worktrees.toml`.
///
/// `rel` should be the CANONICAL relative path; `classify_canonical` makes one.
/// A config that is absent, switched off or does not parse declares nothing,
/// and the built-in rules still apply.
pub fn classify(project_root: &Path, rel: &str) -> Class {
    classify_with(rel, &declared_files(project_root))
}

/// The `[[file]]` paths `project_root`'s config declares, as written there.
/// Separate from `classify` so a caller classing a whole directory reads the
/// config once.
pub fn declared_files(project_root: &Path) -> Vec<String> {
    match crate::projcfg::load(project_root) {
        Ok((Some(cfg), _)) => cfg.files.iter().map(|f| f.path.as_str().to_string()).collect(),
        _ => Vec::new(),
    }
}

/// `classify` with the declarations supplied. PURE: no filesystem.
///
/// A path under `.worktrees/<slug>/` is classed by the part below the place,
/// because a place is a checkout of the same project: its `AGENTS.md` is an
/// instruction file and its `[[file]]` destinations are secrets, exactly as
/// they are at the root. `.worktrees` itself and anything directly in it are
/// the tool's.
///
/// A path that is not relative (absolute, or carrying `..`) cannot be classed
/// and answers `Forbidden`: failing closed is the only safe reading of a path
/// whose meaning depends on where it is resolved.
pub fn classify_with(rel: &str, declared: &[String]) -> Class {
    if rel.starts_with('/') || rel.contains('\0') {
        return Class::Forbidden;
    }
    let parts: Vec<String> = rel.split('/').filter(|c| !c.is_empty() && *c != ".").map(fold).collect();
    if parts.iter().any(|c| c == "..") {
        return Class::Forbidden;
    }
    if parts.first().map(String::as_str) == Some(WT_DIR) {
        if parts.len() < 3 {
            return Class::Forbidden;
        }
        return classify_place(&parts[2..], declared);
    }
    classify_place(&parts, declared)
}

/// The rules for a path relative to a place root (or the project root, which
/// is the `(main)` place). `parts` are already folded.
fn classify_place(parts: &[String], declared: &[String]) -> Class {
    let mut class = Class::Plain;
    let mut raise = |c: Class| class = class.max(c);

    if parts.iter().any(|c| c == ".git") {
        raise(Class::Forbidden);
    }
    match parts.first().map(String::as_str) {
        // A `.worktrees` inside a place is nobody's tree but the tool's either
        // (`projcfg::RelPath` refuses it as a first component too).
        Some(WT_DIR) => raise(Class::Forbidden),
        Some(PLACES_FILE) if parts.len() == 1 => raise(Class::Forbidden),
        _ => {}
    }

    if let Some(name) = parts.last() {
        if name == ".env" || name.starts_with(".env.") {
            raise(Class::Secret);
        }
        if name == ".mcp.json" {
            raise(Class::Executes);
        }
        if INSTRUCTION_NAMES.contains(&name.as_str()) {
            raise(Class::Instruction);
        }
    }
    let joined = parts.join("/");
    for d in declared {
        let d = fold(d.trim_end_matches('/'));
        if !d.is_empty() && (joined == d || joined.starts_with(&format!("{d}/"))) {
            raise(Class::Secret);
        }
    }

    for (i, c) in parts.iter().enumerate() {
        let rest = &parts[i + 1..];
        let first = rest.first().map(String::as_str);
        match c.as_str() {
            ".claude" => match first {
                Some("hooks") => raise(Class::Executes),
                Some("settings.json" | "settings.local.json") if rest.len() == 1 => raise(Class::Executes),
                _ => raise(Class::Instruction),
            },
            ".pi" if first == Some("extensions") => raise(Class::Executes),
            ".codex" if first == Some("config.toml") && rest.len() == 1 => raise(Class::Executes),
            ".kimi-code" => raise(Class::Executes),
            ".github" if first == Some("copilot-instructions.md") && rest.len() == 1 => raise(Class::Instruction),
            _ => {}
        }
        if INSTRUCTION_DIRS.contains(&c.as_str()) {
            raise(Class::Instruction);
        }
    }
    class
}

/// The class of an already-CANONICAL absolute path, or `None` when no root in
/// `roots` (canonical too) contains it.
///
/// The strictest answer over EVERY root that contains it, not the nearest
/// one's: registered projects can nest, and only some rules are anchored at a
/// root (`.worktrees/`, the places file, `[[file]]`). And a `.git` component
/// ANYWHERE in the absolute path is `Forbidden` even above the root, so that a
/// root registered inside a `.git` directory cannot make `config` a plain file
/// — the same test `files::classify_symlink` applies to a link's target.
pub fn classify_canonical(roots: &[PathBuf], canon: &Path) -> Option<Class> {
    let mut out: Option<Class> = None;
    for r in roots {
        let Ok(rel) = canon.strip_prefix(r) else { continue };
        let c = classify(r, &rel.to_string_lossy());
        out = Some(out.map_or(c, |o| o.max(c)));
    }
    let out = out?;
    if has_git_component(canon) {
        return Some(Class::Forbidden);
    }
    Some(out)
}

/// Does any component of `p` fold to `.git`? The question `classify_canonical`
/// asks of the whole absolute path, for callers that must ask it of a path
/// under no root at all (a project about to be created).
pub fn has_git_component(p: &Path) -> bool {
    p.components().any(|c| fold(&c.as_os_str().to_string_lossy()) == ".git")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(rel: &str) -> Class {
        classify_with(rel, &[])
    }

    /// §7.1's table, row by row, as built. Each path is checked at the root
    /// AND inside a place, because a place is a checkout of the same project
    /// and an agent there reads the same files.
    #[test]
    fn the_class_table_as_built() {
        let table: &[(&str, Class)] = &[
            // forbidden
            (".git", Class::Forbidden),
            (".git/config", Class::Forbidden),
            (".git/hooks/pre-commit", Class::Forbidden),
            (".GIT/CONFIG", Class::Forbidden),
            ("vendor/lib/.git/config", Class::Forbidden),
            (".worktrees.places.json", Class::Forbidden),
            // executes
            (".claude/settings.json", Class::Executes),
            (".claude/settings.local.json", Class::Executes),
            (".claude/hooks", Class::Executes),
            (".claude/hooks/pre-tool.sh", Class::Executes),
            (".mcp.json", Class::Executes),
            (".pi/extensions/x.ts", Class::Executes),
            (".codex/config.toml", Class::Executes),
            (".kimi-code/AGENTS.md", Class::Executes),
            (".kimi-code/mcp.json", Class::Executes),
            ("apps/web/.claude/settings.json", Class::Executes),
            ("apps/web/.mcp.json", Class::Executes),
            // instruction
            ("AGENTS.md", Class::Instruction),
            ("CLAUDE.md", Class::Instruction),
            ("GEMINI.md", Class::Instruction),
            ("crates/core/AGENTS.md", Class::Instruction),
            ("CLAUDE.local.md", Class::Instruction),
            ("AGENTS.override.md", Class::Instruction),
            (".cursorrules", Class::Instruction),
            (".planning/brief.md", Class::Instruction),
            (".planning/x/task_plan.md", Class::Instruction),
            (".claude/skills/close-out/SKILL.md", Class::Instruction),
            (".claude/settings.json.bak", Class::Instruction),
            (".claude/commands/x.md", Class::Instruction),
            (".agents/skills/x/SKILL.md", Class::Instruction),
            (".codex/prompts/x.md", Class::Instruction),
            (".pi/settings.json", Class::Instruction),
            (".cursor/rules/x.mdc", Class::Instruction),
            (".github/copilot-instructions.md", Class::Instruction),
            (".worktrees.toml", Class::Instruction),
            // secret
            (".env", Class::Secret),
            (".env.local", Class::Secret),
            ("apps/api/.env.production", Class::Secret),
            (".planning/.env", Class::Secret),
            // plain
            ("README.md", Class::Plain),
            ("src/main.rs", Class::Plain),
            (".github/workflows/ci.yml", Class::Plain),
            (".envrc", Class::Plain),
            ("environment.md", Class::Plain),
            (".gitignore", Class::Plain),
            (".gitmodules", Class::Plain),
            ("docs/agents.md.txt", Class::Plain),
            ("", Class::Plain),
        ];
        for (rel, want) in table {
            assert_eq!(c(rel), *want, "{rel}");
            if !rel.is_empty() {
                let in_place = format!(".worktrees/feat-x/{rel}");
                assert_eq!(c(&in_place), *want, "{in_place}");
            }
        }
    }

    /// The tool's own tree is forbidden; what is inside a place in it is not.
    #[test]
    fn the_worktrees_directory_is_the_tools_and_a_place_inside_it_is_a_checkout() {
        assert_eq!(c(".worktrees"), Class::Forbidden);
        assert_eq!(c(".worktrees/feat-x"), Class::Forbidden, "the place directory itself");
        assert_eq!(c(".worktrees/stray.txt"), Class::Forbidden);
        assert_eq!(c(".worktrees/feat-x/.git"), Class::Forbidden, "a place's gitfile");
        assert_eq!(c(".worktrees/feat-x/.worktrees.places.json"), Class::Forbidden);
        assert_eq!(c(".worktrees/feat-x/.worktrees/y/z"), Class::Forbidden, "a tool tree nested in a place");
        assert_eq!(c(".Worktrees/feat-x/src/a.rs"), Class::Plain);
        assert_eq!(c("docs/.worktrees.places.json"), Class::Plain, "only the root copy is the store");
    }

    /// The fold is APFS's for the letters that matter: a hook file spelled
    /// with U+017F is the hook file.
    #[test]
    fn matching_folds_case_and_the_two_letters_apfs_folds_onto_ascii() {
        assert_eq!(c(".CLAUDE/Settings.JSON"), Class::Executes);
        assert_eq!(c(".claude/\u{17F}ettings.json"), Class::Executes);
        assert_eq!(c(".\u{212A}imi-code/x"), Class::Executes);
        assert_eq!(c(".mcp.j\u{17F}on"), Class::Executes);
        assert_eq!(c("Agents.MD"), Class::Instruction);
        assert_eq!(c(".worktree\u{17F}/x"), Class::Forbidden);
    }

    #[test]
    fn a_declared_file_is_a_secret_at_the_root_and_in_every_place() {
        let d = vec!["config/master.key".to_string(), "secrets/".to_string(), ".claude/settings.local.json".to_string()];
        assert_eq!(classify_with("config/master.key", &d), Class::Secret);
        assert_eq!(classify_with(".worktrees/a/config/master.key", &d), Class::Secret);
        assert_eq!(classify_with("Config/Master.key", &d), Class::Secret);
        assert_eq!(classify_with("secrets/prod/db.json", &d), Class::Secret, "a declared directory");
        assert_eq!(classify_with("secrets-not/x", &d), Class::Plain, "a prefix is a COMPONENT prefix");
        assert_eq!(classify_with("config/master.key.example", &d), Class::Plain);
        // Two classes at once: the stricter wins, on both axes (§7.1).
        assert_eq!(classify_with(".claude/settings.local.json", &d), Class::Secret);
    }

    #[test]
    fn a_path_that_cannot_be_classed_fails_closed() {
        assert_eq!(c("/etc/passwd"), Class::Forbidden);
        assert_eq!(c("src/../.git/config"), Class::Forbidden);
        assert_eq!(c("../x"), Class::Forbidden);
        assert_eq!(c("a\0b"), Class::Forbidden);
        assert_eq!(c("./src//a.rs"), Class::Plain, "`.` and empty components are just separators");
    }

    #[test]
    fn only_a_forbidden_write_is_refused_to_every_caller() {
        for k in [Class::Plain, Class::Instruction, Class::Executes, Class::Secret, Class::Forbidden] {
            assert!(!k.refuses_every_caller(Access::Read), "{k:?} read");
            assert_eq!(k.refuses_every_caller(Access::Write), k == Class::Forbidden, "{k:?} write");
            assert_eq!(k.refuses_devices(Access::Read), k == Class::Forbidden, "{k:?} device read");
        }
    }

    /// `[[file]]` paths are the other door into a checkout (materialize writes
    /// them), and `projcfg::RelPath` guards it with its own name rule. It must
    /// refuse every path this module calls `Forbidden`, or a cloned repo's
    /// config reaches what the Files tab cannot.
    #[test]
    fn projcfg_refuses_every_forbidden_path_a_config_could_declare() {
        for rel in [
            ".git/config",
            ".GIT/hooks/pre-commit",
            "sub/.git/config",
            ".worktrees.places.json",
            ".Worktrees.Places.JSON",
            ".worktrees/x",
            ".worktrees/feat-x/.git",
        ] {
            assert_eq!(c(rel), Class::Forbidden, "{rel}");
            assert!(crate::projcfg::RelPath::parse(rel).is_err(), "projcfg would let a config declare {rel}");
        }
    }

    /// The absolute path decides as well as the relative one: a root
    /// registered inside a `.git` directory does not make `config` plain.
    #[test]
    fn the_strictest_root_wins_and_a_git_component_above_the_root_counts() {
        let base = std::env::temp_dir().join(format!("wt-pathclass-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("repo/.git")).unwrap();
        std::fs::create_dir_all(base.join("repo/vendor/lib")).unwrap();
        let base = std::fs::canonicalize(&base).unwrap();
        let repo = base.join("repo");
        let inner = repo.join("vendor/lib");
        let gitdir = repo.join(".git");

        assert_eq!(classify_canonical(&[repo.clone()], &repo.join("src/a.rs")), Some(Class::Plain));
        assert_eq!(classify_canonical(&[repo.clone()], &base.join("elsewhere")), None, "under no root");
        // Only the root registered inside .git: relative to it, `config` is plain.
        assert_eq!(classify(&gitdir, "config"), Class::Plain);
        assert_eq!(classify_canonical(&[gitdir.clone()], &gitdir.join("config")), Some(Class::Forbidden));
        // Nested roots: the inner root's places file is its store, and only a
        // file three levels down to the outer root.
        let nested = inner.join(".worktrees.places.json");
        assert_eq!(classify(&inner, ".worktrees.places.json"), Class::Forbidden);
        assert_eq!(classify(&repo, "vendor/lib/.worktrees.places.json"), Class::Plain);
        assert_eq!(classify_canonical(&[repo.clone(), inner.clone()], &nested), Some(Class::Forbidden));
        assert_eq!(classify_canonical(&[inner.clone(), repo.clone()], &nested), Some(Class::Forbidden), "order-free");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// `classify` reads the project's own config for its `[[file]]` list.
    #[test]
    fn classify_reads_the_declared_files_from_the_projects_config() {
        let base = std::env::temp_dir().join(format!("wt-pathclass-cfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join(".worktrees.toml"), "[[file]]\npath = \"config/master.key\"\n").unwrap();
        assert_eq!(classify(&base, "config/master.key"), Class::Secret);
        assert_eq!(classify(&base, ".worktrees/a/config/master.key"), Class::Secret);
        assert_eq!(classify(&base, "config/other.key"), Class::Plain);
        let _ = std::fs::remove_dir_all(&base);
    }
}
