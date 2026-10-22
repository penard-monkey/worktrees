//! The one way this crate hands an argv to a core `cmd_*` parser.
//!
//! **R3: a value a client sent is never read as a flag** (`worktrees_core::argv`
//! has the why, and the git half of it). Over `worktrees serve` every string a
//! command takes is a client's, so the rule cannot live in the habits of each
//! call site — every one of them used to push client strings bare, and
//! `relink`'s `slug: "--all"` relinked every place in the project. It lives in
//! [`Argv`]'s shape instead:
//!
//! - our own flags are `&'static str`, so a client `String` cannot be one;
//! - a flag's value is glued on as ONE element, `--name=<v>`, which no parser
//!   reads back as a flag whatever `<v>` holds;
//! - every positional goes after a `--`, which every parser reached here
//!   honours.
//!
//! [`Argv::call`] is the only door: `no_core_parser_is_called_except_through_argv`
//! fails on any other call to a `cmd_*` in this crate. Validation
//! (`worktrees_core::argv::{safe_arg, branch_arg}`) is the second layer and runs
//! FIRST — `--` decides how the parser reads a value, the validator decides
//! whether the user is told why it was refused.

use worktrees_core::{Project, Ui};

/// An argv under construction for a core `cmd_*` parser. See the module docs.
#[derive(Default)]
pub struct Argv {
    flags: Vec<String>,
    pos: Vec<String>,
}

impl Argv {
    pub fn new() -> Self {
        Self::default()
    }

    /// One of OUR flags. `'static` on purpose: a client's string cannot be one.
    pub fn flag(mut self, f: &'static str) -> Self {
        self.flags.push(f.into());
        self
    }

    pub fn flag_if(self, on: bool, f: &'static str) -> Self {
        if on {
            self.flag(f)
        } else {
            self
        }
    }

    /// A value-taking flag with its value glued on: `--name=<v>`, one element.
    pub fn value(mut self, f: &'static str, v: &str) -> Self {
        self.flags.push(format!("{f}={v}"));
        self
    }

    pub fn value_opt(self, f: &'static str, v: Option<&str>) -> Self {
        match v {
            Some(v) => self.value(f, v),
            None => self,
        }
    }

    /// A positional — the slot client values go in. Always after the `--`.
    pub fn pos(mut self, v: impl Into<String>) -> Self {
        self.pos.push(v.into());
        self
    }

    pub fn pos_opt(self, v: Option<String>) -> Self {
        match v {
            Some(v) => self.pos(v),
            None => self,
        }
    }

    /// Flags, then `--` and the positionals. No `--` when there are none: a
    /// parser that takes no positionals at all (`init`) has no reason to know
    /// the terminator.
    fn into_vec(self) -> Vec<String> {
        let mut v = self.flags;
        if !self.pos.is_empty() {
            v.push("--".into());
            v.extend(self.pos);
        }
        v
    }

    /// Hand the argv to a core parser. Takes the parser as a fn ITEM
    /// (`ops::cmd_new`), never a call, which is what the drift check keys on.
    pub fn call(self, p: &Project, ui: &mut dyn Ui, cmd: fn(&Project, &mut dyn Ui, &[String]) -> i32) -> i32 {
        cmd(p, ui, &self.into_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn flags_come_first_values_are_glued_and_positionals_follow_a_double_dash() {
        let v = Argv::new()
            .flag("-y")
            .flag_if(false, "--force")
            .value("--name", "--no-fetch")
            .value_opt("--model", None)
            .pos("--all")
            .pos_opt(Some("-r".into()))
            .into_vec();
        assert_eq!(v, ["-y", "--name=--no-fetch", "--", "--all", "-r"]);
        assert_eq!(Argv::new().flag("-y").into_vec(), ["-y"], "no positionals, no terminator");
    }

    /// The door is the only door. Every `cmd_*` this crate reaches goes through
    /// `Argv::call`, so a direct call — the shape every one of them had before
    /// R3 — fails here, and so does holding the fn anywhere but in a `.call(`.
    /// The drift-check shape AGENTS.md asks for: test modules are sliced off (a
    /// fixture may call anything), comments are not code, and a second `struct
    /// Argv` fails rather than leaving the check to guess which one is real.
    #[test]
    fn no_core_parser_is_called_except_through_argv() {
        let mut files = Vec::new();
        let mut stack = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(&dir).unwrap() {
                let path = e.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|x| x == "rs") {
                    files.push(path);
                }
            }
        }
        assert!(files.len() > 10, "found the sources: {files:?}");

        let (mut bad, mut doors) = (Vec::new(), Vec::new());
        for path in &files {
            let text = std::fs::read_to_string(path).unwrap();
            // The test MODULE, not the first `#[cfg(test)]`: docserver.rs and
            // viewer.rs carry test-only items in the middle of live code.
            let live = text.split("#[cfg(test)]\nmod tests").next().unwrap();
            let live = live.split("#[cfg(test)]\npub(crate) mod tests").next().unwrap();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            for (i, line) in live.lines().enumerate() {
                let code = line.split("//").next().unwrap();
                if code.contains("struct Argv") {
                    doors.push(format!("{name}:{}", i + 1));
                }
                let mut rest = code;
                while let Some(at) = rest.find("cmd_") {
                    let before = &rest[..at];
                    let tail = &rest[at + 4..];
                    let id_len = tail.find(|c: char| !(c.is_ascii_lowercase() || c == '_')).unwrap_or(tail.len());
                    let boundary = !before.ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_');
                    if boundary && id_len > 0 {
                        let called = tail[id_len..].trim_start().starts_with('(');
                        let through_door = before.contains(".call(");
                        if called || !through_door {
                            bad.push(format!("{name}:{}: {}", i + 1, line.trim()));
                        }
                    }
                    rest = &tail[id_len..];
                }
            }
        }
        assert_eq!(doors.len(), 1, "exactly one Argv, or the check is guessing: {doors:?}");
        assert!(bad.is_empty(), "a core parser reached other than through Argv::call:\n{}", bad.join("\n"));
    }
}
