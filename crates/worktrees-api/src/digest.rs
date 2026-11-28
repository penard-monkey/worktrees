//! serve's `digest` row and `worktrees digest`: the "where you left off"
//! list (`worktrees_core::digest`), with the two facts that live in this
//! crate handed down to it: the pull-request cache (`project::PR_CACHE`,
//! never a fetch from here) and the unsent drafts (`ops::scan_drafts`, one
//! `tmux` call). It reads and never writes, and starts nothing.

use std::collections::HashMap;

use worktrees_core::digest::{self, Answer, Extras, PrSource};
use worktrees_core::{registry, sysclock, tmux};

use crate::Ctx;

/// What a caller other than the host's console is told. The digest names
/// every registered project's places, and the first version is the console's
/// alone (plan, "Contract: the row").
pub const ERR_DIGEST_CONSOLE: &str = "not on this device: the digest lists every project's places, and only the host's own console may ask for it";

/// The row: `since` (epoch seconds), `limit` (default 5), `offset`.
pub fn digest(ctx: &Ctx, since: Option<i64>, limit: Option<u32>, offset: Option<u32>) -> Result<Answer, String> {
    if !ctx.is_console() {
        return Err(ERR_DIGEST_CONSOLE.to_string());
    }
    answer(sysclock::now_epoch(), since, limit, offset)
}

/// The digest at `now`, from the user's registry, this process's PR cache and
/// the live panes. The CLI's answer too: a process with no cache gives no
/// pull-request reason, which is said by the absence and never fetched for.
pub fn answer(now: i64, since: Option<i64>, limit: Option<u32>, offset: Option<u32>) -> Result<Answer, String> {
    let reg = registry::read_strict_at(&registry::path()).map_err(|e| format!("the list of projects could not be read: {e}"))?;
    let panes = tmux::PaneList::fetch();
    let extras = Extras { prs: cached_prs(), drafts: drafts(panes.as_ref()) };
    let limit = limit.map_or(digest::LIMIT_DEFAULT, |n| n as usize);
    Ok(digest::digest_in(&reg, now, panes.as_ref(), &extras, since, limit, offset.unwrap_or(0) as usize))
}

/// Every project's last good pull-request list, as the cache holds it.
fn cached_prs() -> HashMap<String, PrSource> {
    let entries = crate::project::PR_CACHE.entries.lock().unwrap_or_else(|e| e.into_inner());
    entries
        .iter()
        .filter_map(|(repo, e)| e.snap.as_ref().map(|(_, snap)| (repo.clone(), PrSource { push_owner: e.push_owner.clone(), snap: snap.clone() })))
        .collect()
}

/// Unsent text by place directory. Never logged, never shown: the digest
/// only says that there is some.
fn drafts(panes: Option<&tmux::PaneList>) -> HashMap<String, String> {
    let Some(panes) = panes else { return HashMap::new() };
    crate::ops::scan_drafts(panes).unwrap_or_default().into_iter().map(|d| (d.path, d.text)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use worktrees_core::authz::{Capability, Caps, Device, DeviceId, PrincipalId};
    use worktrees_core::store::{By, LaunchedBy};

    /// A paired device holding every capability is refused in words, before
    /// anything is read.
    #[test]
    fn the_digest_is_the_consoles_alone() {
        let dir = std::env::temp_dir().join(format!("wt-digest-console-{}", std::process::id()));
        let ctx = Ctx::new(dir, std::sync::Arc::new(crate::tests::Recorder::default()));
        let caps = Caps::of(&[Capability::Read, Capability::Write, Capability::Answer, Capability::Instruct, Capability::Admin]);
        let by = By { principal: PrincipalId::new("owner"), device: Device::Paired(DeviceId::new("phone")) };
        let phone = ctx.granted(caps, false).acting_for(LaunchedBy { by, cross_project: None });
        assert!(!phone.is_console());
        assert_eq!(digest(&phone, None, None, None).unwrap_err(), ERR_DIGEST_CONSOLE);
    }
}
