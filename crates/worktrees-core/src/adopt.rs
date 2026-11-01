//! Adopting a moved project: what `add_project` and `worktrees projects add`
//! do for a repository whose folder was renamed (or moved) outside worktrees.
//!
//! A folder rename breaks two things, and neither stops anything running.
//!
//! * **The session names.** A place's session is `<prefix>-<slug>`, and the
//!   prefix comes from the main root's folder name (`project::resolve_prefix`),
//!   so every place's canonical name changes with the folder while its live
//!   sessions keep the old one. Adoption by pane cwd (`Project::
//!   live_session_in`) finds a place's own session anyway, and the agent rows
//!   use it. It cannot find the provider sidecars, though: codex, pi and kimi
//!   look theirs up by the CANONICAL name (`<canonical>~agent~<id>`), so a
//!   moved place's Codex is invisible until its session is renamed. So the
//!   sessions are renamed, as gap 8's legacy sweep renames
//!   (`Project::rename_legacy_sessions`), and under a rule strict enough that
//!   only a session provably this repo's place ever moves ([`moved_renames`]).
//! * **The git links.** A linked worktree's `.git` file names the common dir
//!   by its absolute path, and the common dir's `worktrees/<id>/gitdir` names
//!   the worktree's `.git` the same way. A rename of the folder that holds
//!   both breaks both, for every worktree that existed before it, and `git`
//!   in that place stops working. `git worktree repair <paths>`, run in the
//!   main checkout, re-establishes both directions; it runs only when a link
//!   points at a path that no longer exists ([`broken_links`]).
//!
//! A pane attached to a renamed session stays attached (tmux attaches the
//! session, not the name). A Claude in it keeps the `--name` it was launched
//! with, which is what Claude's own messaging addresses and what
//! `place_status` reports; its probe file keeps naming the old session too,
//! which is why the agent rows find a lane's probe by its pane id first
//! (`worktrees_api::agents::lane_probe`).

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::project::session_name_under;
use crate::{tmux, Project};

/// One session `adopt` renamed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Renamed {
    pub from: String,
    pub to: String,
}

/// What [`adopt`] did. Empty when there was nothing to do.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct Adoption {
    /// Places whose git links `git worktree repair` re-established.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub repaired: Vec<String>,
    /// `git worktree repair`'s complaint, when it failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repair_error: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub renamed: Vec<Renamed>,
}

impl Adoption {
    pub fn is_empty(&self) -> bool {
        self.repaired.is_empty() && self.repair_error.is_none() && self.renamed.is_empty()
    }
}

/// Repair `p`'s broken worktree links, then rename the live sessions that are
/// provably its places' to the names its prefix renders now. Best-effort and
/// idempotent: a second run finds nothing broken and nothing to rename.
pub fn adopt(p: &Project) -> Adoption {
    let mut out = Adoption::default();
    let dirs: Vec<String> = p.place_index().into_iter().filter(|r| !r.is_main).map(|r| r.path).collect();
    let broken = broken_links(Path::new(&p.git_common), &dirs);
    match repair(&p.main_root, &broken, git_repair) {
        Ok(()) => out.repaired = broken,
        Err(e) => out.repair_error = Some(e),
    }
    if let Some(panes) = tmux::PaneList::fetch() {
        for (from, to) in planned(p, &panes) {
            if tmux::rename_session(&from, &to) {
                out.renamed.push(Renamed { from, to });
            }
        }
    }
    out
}

/// The renames [`adopt`] would make now, over the snapshot `panes`: what
/// `doctor` checks a drifted session against, so its remedy names the command
/// only for a session that command would move.
pub fn planned(p: &Project, panes: &tmux::PaneList) -> Vec<(String, String)> {
    let places: Vec<(String, String)> = p.place_index().into_iter().map(|r| (r.slug, r.path)).collect();
    let roots: Vec<String> = crate::registry::read_lenient().projects.into_iter().map(|e| e.root).collect();
    moved_renames(&p.prefix, p.wt_root_dir(), &places, panes, &other_prefixes(&roots, &p.main_root))
}

/// The prefixes of the OTHER registered projects whose root is on disk: the
/// names a session of theirs would carry, which this repo must never take.
///
/// A registered root that no longer exists protects nothing. That is the
/// moved folder itself when the new root is added before the old one is
/// removed, and its sessions are exactly the ones to adopt. It is safe because
/// [`moved_renames`] moves a session only when EVERY pane of it is in one of
/// this repo's places: a project whose root is only temporarily absent (an
/// unmounted volume, an iCloud folder not downloaded) has its sessions' panes
/// under its own path, never under this one.
pub fn other_prefixes(roots: &[String], own_root: &str) -> Vec<String> {
    roots
        .iter()
        .filter(|r| r.as_str() != own_root && Path::new(r).is_dir())
        .map(|r| crate::project::resolve_prefix(r).0)
        .collect()
}

/// Which live sessions to rename, as `(old, new)`. PURE.
///
/// A session moves only when ALL of these hold — the adoption rule:
///
/// 1. its name is `<X>-<slug>` for the slug of the place its panes are in
///    (`.` → `-`, as every session name is spelled), or that name with a
///    provider sidecar suffix this build knows (`~agent~<id>`) or a dock-shell
///    one (`~term`, `~term~<n>`);
/// 2. EVERY pane of the session is in that one place (for `(main)`, not under
///    `.worktrees/`);
/// 3. `<X>` is not another registered project's prefix (`others`, from
///    [`other_prefixes`]: only roots that exist);
/// 4. the name it would get (`<prefix>-<slug>` plus the same suffix) is not
///    already taken, by a live session or by an earlier rename in this pass.
///
/// So a session someone named by hand is never renamed, nor one with a pane
/// anywhere else, nor one of another project's, nor one whose new name a
/// session already holds — each stays as it is, still found by pane cwd.
pub fn moved_renames(
    prefix: &str,
    wt_root: &str,
    places: &[(String, String)],
    panes: &tmux::PaneList,
    others: &[String],
) -> Vec<(String, String)> {
    let mut live: Vec<&str> = Vec::new();
    for pane in panes.panes() {
        if !pane.session.is_empty() && !live.contains(&pane.session.as_str()) {
            live.push(&pane.session);
        }
    }
    let mut out: Vec<(String, String)> = Vec::new();
    for name in &live {
        let Some((base, suffix)) = split_sidecar(name) else { continue };
        let Some((slug, _)) = place_of(panes.session_paths(name), places, wt_root) else { continue };
        let own = session_name_under(prefix, slug);
        if base == own {
            continue;
        }
        let tail = session_name_under("", slug);
        let Some(x) = base.strip_suffix(tail.as_str()).filter(|x| !x.is_empty()) else { continue };
        if others.iter().any(|o| o.replace('.', "-") == x) {
            continue;
        }
        let target = format!("{own}{suffix}");
        if live.contains(&target.as_str()) || out.iter().any(|(_, t)| *t == target) {
            continue;
        }
        out.push((name.to_string(), target));
    }
    out
}

/// `name` as `(base, suffix)`: a known provider sidecar's `~agent~<id>` or a
/// dock shell's `~term`/`~term~<n>` split off, else the whole name. `None`
/// for any other `~` name — a sidecar of a harness this build does not know,
/// which is not this build's to move.
fn split_sidecar(name: &str) -> Option<(&str, &str)> {
    let Some(at) = name.find('~') else { return Some((name, "")) };
    let (base, suffix) = name.split_at(at);
    let known = crate::provider::PROVIDERS.iter().any(|p| suffix == p.sidecar_suffix)
        || tmux::shell_sidecar_index(base, name).is_some();
    known.then_some((base, suffix))
}

/// The one place every path is in, or `None` when any path is in none of
/// them or two paths are in different ones. `(main)` holds what is under the
/// main root but not under `.worktrees/`.
fn place_of<'a, 'p>(
    paths: impl Iterator<Item = &'p str>,
    places: &'a [(String, String)],
    wt_root: &str,
) -> Option<&'a (String, String)> {
    let under = |p: &str, d: &str| p == d || p.starts_with(&format!("{d}/"));
    let mut found: Option<&'a (String, String)> = None;
    for path in paths {
        let here = places.iter().find(|(slug, dir)| under(path, dir) && !(slug == "(main)" && under(path, wt_root)))?;
        match found {
            Some(f) if f != here => return None,
            _ => found = Some(here),
        }
    }
    found
}

/// The places among `dirs` whose git links point at a path that no longer
/// exists: the `.git` file's `gitdir:`, or the common dir's
/// `worktrees/<id>/gitdir` it leads to. A directory with no `.git` FILE (not a
/// linked worktree, or a stale directory) has nothing to repair.
pub fn broken_links(git_common: &Path, dirs: &[String]) -> Vec<String> {
    dirs.iter().filter(|d| link_broken(git_common, Path::new(d))).cloned().collect()
}

fn link_broken(git_common: &Path, dir: &Path) -> bool {
    let Some(admin) = gitdir_of(dir) else { return false };
    if !admin.exists() {
        return true;
    }
    // The admin side, by the id the link names; under the common dir it must
    // be, so a link into some other repository is not followed.
    let Some(id) = admin.file_name() else { return false };
    let back = git_common.join("worktrees").join(id).join("gitdir");
    match std::fs::read_to_string(&back) {
        Ok(t) => !resolve(back.parent().unwrap_or(git_common), t.trim()).exists(),
        Err(_) => false,
    }
}

/// The `gitdir:` a linked worktree's `.git` FILE names, made absolute.
fn gitdir_of(dir: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(dir.join(".git")).ok()?;
    let line = text.lines().next()?.strip_prefix("gitdir:")?.trim();
    (!line.is_empty()).then(|| resolve(dir, line))
}

/// `p` as written (git writes relative links under `worktree.useRelativePaths`)
/// against `base`.
fn resolve(base: &Path, p: &str) -> PathBuf {
    let p = Path::new(p);
    if p.is_absolute() { p.to_path_buf() } else { base.join(p) }
}

/// Run `git worktree repair` over `broken` from the main checkout, and only
/// when there is something broken: `run` is never called otherwise.
pub fn repair(main_root: &str, broken: &[String], run: impl FnOnce(&str, &[String]) -> Result<(), String>) -> Result<(), String> {
    if broken.is_empty() {
        return Ok(());
    }
    run(main_root, broken)
}

/// The real runner: `git -C <main> worktree repair -- <paths>`, which, run in
/// the main checkout with each linked worktree's NEW path, re-establishes the
/// links in both directions. The paths are this repo's own `.worktrees/`
/// entries; the `--` keeps any of them from reading as a flag regardless.
pub fn git_repair(main_root: &str, dirs: &[String]) -> Result<(), String> {
    let mut args = vec!["worktree", "repair", "--"];
    args.extend(dirs.iter().map(String::as_str));
    let o = crate::git::git(main_root, &args).map_err(|e| e.to_string())?;
    if o.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
        Err(if err.is_empty() { format!("git worktree repair exited {}", o.status.code().unwrap_or(-1)) } else { err })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn places(root: &str, lanes: &[&str]) -> Vec<(String, String)> {
        std::iter::once(("(main)".to_string(), root.to_string()))
            .chain(lanes.iter().map(|l| (l.to_string(), format!("{root}/.worktrees/{l}"))))
            .collect()
    }

    fn panes(rows: &[(&str, &str)]) -> tmux::PaneList {
        tmux::PaneList::from_rows(rows.iter().map(|(s, p)| (s.to_string(), p.to_string(), "2.1.309".to_string())).collect())
    }

    const ROOT: &str = "/u/new";
    const WT: &str = "/u/new/.worktrees";

    /// The reported flow: the folder `old` became `new`. Its (main) and its
    /// lane, both still `old-…`, move to `new-…`, sidecars and dock shells
    /// with them — and a dotted slug is spelled the way session names are.
    #[test]
    fn a_moved_projects_sessions_move_to_the_names_its_folder_renders() {
        let pl = places(ROOT, &["lane", "v1.2"]);
        let live = panes(&[
            ("old-(main)", ROOT),
            ("old-(main)", "/u/new/src"),
            ("old-lane", "/u/new/.worktrees/lane"),
            ("old-lane~agent~codex", "/u/new/.worktrees/lane"),
            ("old-lane~term~2", "/u/new/.worktrees/lane/sub"),
            ("old-v1-2", "/u/new/.worktrees/v1.2"),
        ]);
        let got = moved_renames("new", WT, &pl, &live, &[]);
        let want: Vec<(String, String)> = [
            ("old-(main)", "new-(main)"),
            ("old-lane", "new-lane"),
            ("old-lane~agent~codex", "new-lane~agent~codex"),
            ("old-lane~term~2", "new-lane~term~2"),
            ("old-v1-2", "new-v1-2"),
        ]
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        assert_eq!(got, want);
        // Already canonical: nothing to do, which is what makes it idempotent.
        let done = panes(&[("new-(main)", ROOT), ("new-lane", "/u/new/.worktrees/lane")]);
        assert!(moved_renames("new", WT, &pl, &done, &[]).is_empty());
    }

    /// Each part of the rule refuses on its own.
    #[test]
    fn the_adoption_rule_refuses_what_it_cannot_prove_is_this_repos() {
        let pl = places(ROOT, &["lane"]);
        let none = |rows: &[(&str, &str)], others: &[&str]| {
            let others: Vec<String> = others.iter().map(|s| s.to_string()).collect();
            let got = moved_renames("new", WT, &pl, &panes(rows), &others);
            assert!(got.is_empty(), "{rows:?} with others {others:?} renamed {got:?}");
        };
        // 1. A session someone named by hand, in the place.
        none(&[("scratch", ROOT)], &[]);
        none(&[("my-work", "/u/new/.worktrees/lane")], &[]);
        // ...named for ANOTHER place than the one its panes are in.
        none(&[("old-lane", ROOT)], &[]);
        none(&[("old-(main)", "/u/new/.worktrees/lane")], &[]);
        // ...or the bare slug, with no prefix at all.
        none(&[("-lane", "/u/new/.worktrees/lane")], &[]);
        // ...or a sidecar of a harness this build does not know.
        none(&[("old-lane~agent~opencode", "/u/new/.worktrees/lane")], &[]);
        // 2. A pane outside the place, or in two places.
        none(&[("old-(main)", ROOT), ("old-(main)", "/elsewhere")], &[]);
        none(&[("old-(main)", ROOT), ("old-(main)", "/u/new/.worktrees/lane")], &[]);
        none(&[("old-(main)", "/u/new/.worktrees")], &[]);
        // 3. X is another registered project's prefix.
        none(&[("old-(main)", ROOT)], &["old"]);
        none(&[("old-lane~agent~codex", "/u/new/.worktrees/lane")], &["zzz", "old"]);
        // 4. The canonical name is already taken.
        none(&[("old-(main)", ROOT), ("new-(main)", "/elsewhere")], &[]);
        none(&[("old-lane~term", "/u/new/.worktrees/lane"), ("new-lane~term", "/x")], &[]);
        // ...including by an earlier rename in the same pass: one wins.
        let two = moved_renames("new", WT, &pl, &panes(&[("old-(main)", ROOT), ("older-(main)", ROOT)]), &[]);
        assert_eq!(two, vec![("old-(main)".to_string(), "new-(main)".to_string())]);
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wt-adopt-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::canonicalize(&d).unwrap()
    }

    /// A registered root that is gone from disk does not protect its prefix;
    /// one that exists still does (the moved folder vs. a live project).
    #[test]
    fn only_a_registered_root_on_disk_protects_its_prefix() {
        let base = tmp("prefixes");
        let gone = base.join("gone/old").to_string_lossy().into_owned();
        let there = base.join("there/old");
        std::fs::create_dir_all(&there).unwrap();
        let there = there.to_string_lossy().into_owned();
        let own = ROOT.to_string();
        let pl = places(ROOT, &[]);
        let live = panes(&[("old-(main)", ROOT)]);

        let others = other_prefixes(&[gone.clone(), own.clone()], &own);
        assert!(others.is_empty(), "{others:?}");
        assert_eq!(moved_renames("new", WT, &pl, &live, &others).len(), 1, "an absent root does not block adoption");

        let others = other_prefixes(&[gone, there], &own);
        assert_eq!(others, ["old"]);
        assert!(moved_renames("new", WT, &pl, &live, &others).is_empty(), "a root that exists still does");
        let _ = std::fs::remove_dir_all(&base);
    }

    fn git(dir: &Path, args: &[&str]) {
        let o = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    }

    /// `git worktree repair` runs only when a link is broken, and then mends
    /// both directions: the folder holding the main checkout and its
    /// `.worktrees/` was renamed, as the user's was.
    #[test]
    fn git_worktree_repair_runs_only_when_a_link_is_broken() {
        let base = tmp("repair");
        let old = base.join("old");
        std::fs::create_dir_all(&old).unwrap();
        git(&old, &["init", "-q"]);
        git(&old, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "--allow-empty", "-m", "init"]);
        git(&old, &["worktree", "add", "-q", ".worktrees/lane", "-b", "lane"]);
        std::fs::create_dir_all(old.join(".worktrees/stale")).unwrap(); // no .git: not a worktree

        let dirs = |root: &Path| vec![root.join(".worktrees/lane").to_string_lossy().into_owned(), root.join(".worktrees/stale").to_string_lossy().into_owned()];
        let common = |root: &Path| root.join(".git");
        let never = |_: &str, d: &[String]| -> Result<(), String> { panic!("repair ran with nothing broken: {d:?}") };

        assert!(broken_links(&common(&old), &dirs(&old)).is_empty(), "nothing is broken before the move");
        repair(&old.to_string_lossy(), &broken_links(&common(&old), &dirs(&old)), never).unwrap();

        let new = base.join("new");
        std::fs::rename(&old, &new).unwrap();
        let broken = broken_links(&common(&new), &dirs(&new));
        assert_eq!(broken, [new.join(".worktrees/lane").to_string_lossy().into_owned()], "the lane, and not the stale dir");
        let lane = new.join(".worktrees/lane");
        assert!(!std::process::Command::new("git").arg("-C").arg(&lane).args(["status"]).output().unwrap().status.success(), "git is broken in the lane");

        repair(&new.to_string_lossy(), &broken, git_repair).unwrap();
        assert!(broken_links(&common(&new), &dirs(&new)).is_empty(), "both directions mended");
        git(&lane, &["status", "-s"]);
        let list = String::from_utf8(std::process::Command::new("git").arg("-C").arg(&new).args(["worktree", "list", "--porcelain"]).output().unwrap().stdout).unwrap();
        assert!(list.contains(&format!("worktree {}", lane.display())), "{list}");
        // And now there is nothing to do.
        repair(&new.to_string_lossy(), &broken_links(&common(&new), &dirs(&new)), never).unwrap();

        // Only the ADMIN side broken (its gitdir rewritten to a dead path):
        // still found, by the id the place's own link names.
        std::fs::write(new.join(".git/worktrees/lane/gitdir"), "/no/such/place/.git\n").unwrap();
        assert_eq!(broken_links(&common(&new), &dirs(&new)).len(), 1);
        let _ = std::fs::remove_dir_all(&base);
    }
}
