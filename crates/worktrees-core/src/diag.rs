//! Structured findings — the `doctor` channel (proposal §7).
//!
//! Deliberately NOT `WtError`: that type is a single fatal error carrying an
//! exit code, and widening it would make every op's error path pay for a
//! diagnostic report it never produces. A `Report` is many non-fatal facts; a
//! `WtError` is one fatal one. They stay separate types.
//!
//! Serialization follows `model.rs`: explicit field order, and every nullable
//! field is `Option<T>` that is NOT skipped, so `None` emits `null` rather than
//! vanishing — a consumer reading `doctor --json` sees the same key set every
//! time.

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

/// Exit code when any finding is an `Error` (§7). `0` clean / `1` usage-or-guard
/// (`WtError`'s existing convention) / `2` findings present — `1` is reserved so
/// CI can tell "doctor broke" from "doctor found problems".
pub const EXIT_FINDINGS: i32 = 2;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warn,
    Error,
}

/// The CLOSED set of finding slugs. An enum rather than free strings so a typo
/// is a compile error and the JSON vocabulary can't drift between the CLI and
/// the app; serializes as the kebab slug the proposal names.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Code {
    // §7 — file/link drift
    /// A declared source file is absent from the main checkout.
    MissingSource,
    /// A declared file is not materialized in this place at all. ⚠ NOT in §7's
    /// slug list, and it has to be: "the worktree is simply missing the file" is
    /// the exact silent failure §1.2 describes, and none of the listed slugs
    /// names it (`missing-source` is about MAIN, not about the place).
    NotLinked,
    /// A Layer B (§4) violation: the source's parent, or the source itself,
    /// resolves outside the main checkout; the source is not a regular file or is
    /// over the copy cap; or a destination ancestor is a symlink we did not
    /// create. ⚠ Also not in §7's slug list — which has no code for "the config
    /// pointed somewhere it may not point", the whole point of Layer B.
    UnsafePath,
    /// A real file sits where a declared link belongs — report, never overwrite.
    Shadowed,
    /// A link exists but its target is gone.
    DanglingLink,
    /// Declared `link` but found a copy (or the reverse).
    WrongMode,
    /// A declared path is not gitignored, so `git add -A` would commit it.
    NotGitignored,
    /// A copy differs from its source and the SOURCE is newer.
    CopyStale,
    // §6 — port slots
    /// Two places claim the same `WORKTREE_SLOT`.
    SlotConflict,
    /// No free slot in `1..=max_slots`.
    NoSlot,
    // §5 — config policy
    /// A key `.worktrees.toml` does not understand; ignored, same forward-compat
    /// discipline as `store.rs`'s `#[serde(flatten)] extra`.
    UnknownKey,
    /// A key that parses today but is not honored yet (`[project] prefix`, §12).
    DeferredKey,
    /// `.worktree-prefix` and `[project] prefix` disagree.
    PrefixMismatch,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Finding {
    pub severity: Severity,
    pub code: Code,
    /// Slug of the place this is about, or `null` for a project-wide finding.
    pub place: Option<String>,
    /// Repo-relative path this is about, or `null`.
    pub path: Option<String>,
    pub message: String,
}

impl Finding {
    pub fn new(severity: Severity, code: Code, message: impl Into<String>) -> Self {
        Finding { severity, code, place: None, path: None, message: message.into() }
    }
    pub fn info(code: Code, message: impl Into<String>) -> Self {
        Finding::new(Severity::Info, code, message)
    }
    pub fn warn(code: Code, message: impl Into<String>) -> Self {
        Finding::new(Severity::Warn, code, message)
    }
    pub fn error(code: Code, message: impl Into<String>) -> Self {
        Finding::new(Severity::Error, code, message)
    }
    /// Builder: attach the place slug.
    pub fn at_place(mut self, place: impl Into<String>) -> Self {
        self.place = Some(place.into());
        self
    }
    /// Builder: attach the repo-relative path.
    pub fn at_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Report {
    pub schema_version: u32,
    pub findings: Vec<Finding>,
}

impl Default for Report {
    fn default() -> Self {
        Report { schema_version: SCHEMA_VERSION, findings: Vec::new() }
    }
}

impl Report {
    pub fn new(findings: Vec<Finding>) -> Self {
        Report { schema_version: SCHEMA_VERSION, findings }
    }
    pub fn push(&mut self, f: Finding) {
        self.findings.push(f);
    }
    pub fn is_empty(&self) -> bool {
        self.findings.is_empty()
    }
    /// Any `Error` → `2`, otherwise `0`. Warnings never fail a run on their own
    /// (§7: drift is the expected steady state); `1` is never returned here
    /// because it belongs to `WtError`.
    pub fn exit_code(&self) -> i32 {
        if self.findings.iter().any(|f| f.severity == Severity::Error) {
            EXIT_FINDINGS
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_is_two_only_when_an_error_is_present() {
        assert_eq!(Report::default().exit_code(), 0);
        let warns = Report::new(vec![
            Finding::info(Code::CopyStale, "drifted"),
            Finding::warn(Code::UnknownKey, "unknown key"),
        ]);
        assert_eq!(warns.exit_code(), 0);
        let errs = Report::new(vec![
            Finding::warn(Code::UnknownKey, "unknown key"),
            Finding::error(Code::Shadowed, "shadowed"),
        ]);
        assert_eq!(errs.exit_code(), 2);
    }

    #[test]
    fn codes_serialize_as_the_kebab_slugs_the_proposal_names() {
        let f = Finding::error(Code::NotGitignored, "m").at_place("feat").at_path("apps/.env");
        let j = serde_json::to_string(&Report::new(vec![f])).unwrap();
        assert_eq!(
            j,
            r#"{"schema_version":1,"findings":[{"severity":"error","code":"not-gitignored","place":"feat","path":"apps/.env","message":"m"}]}"#
        );
        for (c, s) in [
            (Code::MissingSource, "missing-source"),
            (Code::NotLinked, "not-linked"),
            (Code::UnsafePath, "unsafe-path"),
            (Code::Shadowed, "shadowed"),
            (Code::DanglingLink, "dangling-link"),
            (Code::WrongMode, "wrong-mode"),
            (Code::NotGitignored, "not-gitignored"),
            (Code::SlotConflict, "slot-conflict"),
            (Code::NoSlot, "no-slot"),
            (Code::CopyStale, "copy-stale"),
            (Code::UnknownKey, "unknown-key"),
            (Code::DeferredKey, "deferred-key"),
            (Code::PrefixMismatch, "prefix-mismatch"),
        ] {
            assert_eq!(serde_json::to_string(&c).unwrap(), format!("\"{s}\""));
        }
    }

    #[test]
    fn nullable_fields_serialize_as_explicit_null() {
        // model.rs's rule: no `skip_serializing_if`, so the key set is stable.
        let j = serde_json::to_string(&Finding::warn(Code::UnknownKey, "m")).unwrap();
        assert_eq!(j, r#"{"severity":"warn","code":"unknown-key","place":null,"path":null,"message":"m"}"#);
    }
}
