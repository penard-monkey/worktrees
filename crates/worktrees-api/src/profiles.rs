//! AI profiles + the skill store.
//!
//! Thin wrappers: core owns profiles.json and the skill store (the CLI reads
//! the same files), so these must never keep their own copy of that state.
//!
//! These were `async fn` as Tauri commands, because AGENTS.md requires it —
//! a sync handler runs on the main thread and freezes the UI. The rule is
//! about the HANDLER, not the work: these bodies are blocking filesystem and
//! git calls with nothing to await. They are sync here and the Tauri wrapper
//! stays `async`, which keeps the body off the main thread exactly as before
//! and lets the server decide its own blocking strategy.

use std::path::Path;

pub fn profiles_info(repo: String) -> Result<serde_json::Value, String> {
    // Filesystem probes per profile (`ever_launched`) live here rather than in
    // `snapshot`, because this is called on sheet-open and that is on the 3s poll.
    serde_json::to_value(worktrees_core::profile::info_for(&repo)).map_err(|e| e.to_string())
}

pub fn save_profile(profile: serde_json::Value) -> Result<String, String> {
    let p: worktrees_core::profile::Profile =
        serde_json::from_value(profile).map_err(|e| format!("invalid profile: {e}"))?;
    worktrees_core::profile::save(p).inspect_err(|e| crate::log::log("error", &format!("save_profile: {e}")))
}

pub fn new_profile_id(name: String) -> Result<String, String> {
    let taken: Vec<String> = worktrees_core::profile::read_lenient().profiles.keys().cloned().collect();
    Ok(worktrees_core::profile::new_id_from(&name, &taken))
}

/// What deleting a profile actually costs, reported rather than done silently.
#[derive(serde::Serialize)]
pub struct ProfileRemoval {
    /// The materialized dir is LEFT ON DISK — it holds the session transcripts.
    pub dir: Option<String>,
    /// The keychain item's service name, when it was recorded — so the message
    /// can name what to look for instead of sending the user hunting.
    pub keychain_service: Option<String>,
    /// A keychain item may remain. worktrees never touches credentials (that is
    /// the invariant that made this whole feature safe), so it cannot delete one
    /// either — the UI tells the user where it is instead of pretending.
    pub keychain_hint: bool,
}

pub fn delete_profile(id: String) -> Result<ProfileRemoval, String> {
    let dir = worktrees_core::profile::profile_dir(&id).map(|d| d.to_string_lossy().into_owned());
    let launched = worktrees_core::profile::ever_launched(&id);
    let keychain_service = worktrees_core::profile::remove(&id)
        .inspect_err(|e| crate::log::log("error", &format!("delete_profile: {e}")))?;
    Ok(ProfileRemoval { dir, keychain_service, keychain_hint: launched })
}

pub fn set_project_profile(repo: String, id: Option<String>) -> Result<(), String> {
    worktrees_core::profile::assign(&repo, id.as_deref())
        .inspect_err(|e| crate::log::log("error", &format!("set_project_profile: {e}")))
}

pub fn set_default_profile(id: Option<String>) -> Result<(), String> {
    worktrees_core::profile::set_default(id.as_deref())
        .inspect_err(|e| crate::log::log("error", &format!("set_default_profile: {e}")))
}

pub fn skills_list() -> Result<serde_json::Value, String> {
    serde_json::to_value(worktrees_core::skillstore::list()).map_err(|e| e.to_string())
}

/// Read a candidate skill directory WITHOUT installing it — the review step.
pub fn skill_inspect(path: String) -> Result<serde_json::Value, String> {
    let i = worktrees_core::skillstore::inspect(Path::new(&path))?;
    Ok(serde_json::json!({
        "name": i.name, "description": i.description,
        "capabilities": i.capabilities, "files": i.files, "bytes": i.bytes,
        "skill_md": i.skill_md,
    }))
}

pub fn skill_install_local(path: String) -> Result<serde_json::Value, String> {
    let e = worktrees_core::skillstore::install_local(Path::new(&path))
        .inspect_err(|e| crate::log::log("error", &format!("skill_install_local: {e}")))?;
    serde_json::to_value(e).map_err(|e| e.to_string())
}

/// Clone, inspect, discard. Installs nothing — see `skill_install_git`.
pub fn skill_preview_git(url: String, rev: Option<String>) -> Result<serde_json::Value, String> {
    let p = worktrees_core::skillstore::preview_git(&url, rev.as_deref().unwrap_or(""))
        .inspect_err(|e| crate::log::log("warn", &format!("skill_preview_git: {e}")))?;
    serde_json::to_value(p).map_err(|e| e.to_string())
}

/// Install at the sha the user reviewed. Refuses if the branch moved.
pub fn skill_install_git(
    url: String,
    rev: Option<String>,
    sha: String,
    name: String,
) -> Result<serde_json::Value, String> {
    let e = worktrees_core::skillstore::install_git_pinned(
        &url,
        rev.as_deref().unwrap_or(""),
        &sha,
        &name,
    )
    .inspect_err(|e| crate::log::log("error", &format!("skill_install_git: {e}")))?;
    serde_json::to_value(e).map_err(|e| e.to_string())
}

pub fn skill_remove(name: String) -> Result<Vec<String>, String> {
    worktrees_core::skillstore::remove(&name)
        .inspect_err(|e| crate::log::log("error", &format!("skill_remove: {e}")))
}

