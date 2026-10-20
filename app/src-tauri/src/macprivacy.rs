//! macOS privacy (TCC) denials, named rather than left as `os error 1`.
//!
//! # The problem this solves
//!
//! A user whose repos live in `~/Documents` (or on iCloud Drive, or a USB
//! disk) gets a macOS prompt the first time this app reads them. Press Deny by
//! accident and **macOS never asks again** — there is no API to re-raise the
//! prompt, and the only lever is clearing or changing the recorded decision.
//! What the app got afterwards was `Operation not permitted (os error 1)`,
//! rendered as a raw line over an empty tree, which reads as a broken app
//! rather than as a permission the user can give back.
//!
//! # Why a SEPARATE command rather than a richer error type
//!
//! `list_dir`, `changed_files` and the project scan all answer
//! `Result<_, String>`, and their callers do `String(e)`. Widening those to a
//! structured error would touch every call site to fix one rare branch. So the
//! frontend asks *after* a failure — `privacy_denial(path)` — and that call is
//! the ONLY place that decides whether a failure is a privacy denial. The
//! frontend never pattern-matches an errno or an English message, which is the
//! mirror that would drift (and `os error 1`'s text is localised).
//!
//! # Why the probe re-reads instead of taking the caller's `io::Error`
//!
//! Same reason: the error has already been flattened to a `String` by the time
//! anything decides to ask. Re-reading costs one syscall on a path that just
//! failed, and it is honest — if access was restored in between (the user
//! fixed it in System Settings and the grant applied), the probe says `None`
//! and no card appears.
//!
//! # What it cannot do
//!
//! Nothing here can make macOS prompt again. `tccutil reset` and the Settings
//! toggle are the only two levers, plus a third that is specific to this app:
//! its designated requirement is a bare `cdhash` (ad-hoc signed, no Team ID),
//! so TCC keys the decision to that exact binary and **the next release
//! prompts fresh**. That is the same property AGENTS.md records as a nuisance
//! — grants never surviving a rebuild — working in the user's favour.
//!
//! # Off macOS it answers `None`, always
//!
//! The paths are only half of a denial. On Linux an unreadable `~/Documents`
//! refuses with EACCES and matches `Area::Documents` exactly as a TCC denial
//! would, and the card it produced named macOS, linked a System Settings pane
//! and offered a `tccutil` line — none of which exist there. Elsewhere the
//! failure is an ordinary permission problem the raw error already describes,
//! so this module is inert rather than wrong. The platform is a PARAMETER of
//! `denial_at`, not a `cfg` inside it, so both answers are tested on every OS:
//! a `#[cfg(not(target_os = "macos"))]` test would never run on the machine
//! this app is developed on.

use std::path::{Path, PathBuf};

/// A protected area, in the vocabulary of the pane that unlocks it.
///
/// `service` is the `tccutil reset` service name, `pane` the System Settings
/// deep link. Both are DATA on this enum rather than a `match` at the call
/// site, so a new area cannot be added with one of them forgotten.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Area {
    Desktop,
    Documents,
    Downloads,
    ICloud,
    Removable,
    Network,
    AppData,
    /// Readable-in-principle but refused — no specific pane names it, so the
    /// card falls back to Full Disk Access.
    Other,
}

impl Area {
    pub fn label(self) -> &'static str {
        match self {
            Area::Desktop => "your Desktop folder",
            Area::Documents => "your Documents folder",
            Area::Downloads => "your Downloads folder",
            Area::ICloud => "iCloud Drive",
            Area::Removable => "a removable volume",
            Area::Network => "a network volume",
            Area::AppData => "another app's data",
            Area::Other => "this folder",
        }
    }

    /// `tccutil reset <service> <bundle id>`.
    pub fn service(self) -> &'static str {
        match self {
            Area::Desktop => "SystemPolicyDesktopFolder",
            Area::Documents => "SystemPolicyDocumentsFolder",
            Area::Downloads => "SystemPolicyDownloadsFolder",
            // iCloud and everything under `CloudStorage` sit behind the same
            // consent as a local folder's parent would; there is no iCloud-only
            // service, and Full Disk Access is the one that always covers it.
            Area::ICloud => "SystemPolicyAllFiles",
            Area::Removable => "SystemPolicyRemovableVolumes",
            Area::Network => "SystemPolicyNetworkVolumes",
            Area::AppData => "SystemPolicyAppData",
            Area::Other => "SystemPolicyAllFiles",
        }
    }

    /// The System Settings pane. `x-apple.systempreferences:` survived the
    /// System Settings rewrite; the anchors are what moved, so these are the
    /// post-Ventura ones.
    pub fn pane(self) -> &'static str {
        match self {
            Area::Desktop | Area::Documents | Area::Downloads | Area::ICloud | Area::Other => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_FilesAndFolders"
            }
            Area::Removable => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_RemovableVolume"
            }
            Area::Network => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_NetworkVolume"
            }
            Area::AppData => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_AppBundles"
            }
        }
    }
}

/// What the UI needs to explain the denial and offer a way out.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Denial {
    pub area: Area,
    /// "your Documents folder" — reads inside a sentence.
    pub label: String,
    /// The path that was refused.
    pub path: String,
    pub pane: String,
    /// The whole command, ready to paste. Built here so the UI never
    /// assembles a shell line out of parts.
    pub reset_command: String,
}

/// Which protected area `path` is in, or `None` when it is somewhere macOS
/// does not gate.
///
/// Purely positional — it asks where the path IS, never whether it is
/// readable. `denial_at` is what pairs it with an actual refusal.
pub fn area_of(path: &Path, home: &Path) -> Option<Area> {
    let under = |rel: &str| path.starts_with(home.join(rel));
    // Volumes first: a removable disk is not under $HOME, and a home that
    // itself lives on an external disk would otherwise match nothing.
    if path.starts_with("/Volumes/") {
        return Some(Area::Removable);
    }
    if path.starts_with("/net/") || path.starts_with("/Network/") {
        return Some(Area::Network);
    }
    // iCloud and the File Provider mounts (Dropbox, OneDrive, Google Drive)
    // BEFORE the generic Library check below, which would otherwise swallow
    // them as app data.
    if under("Library/Mobile Documents") || under("Library/CloudStorage") {
        return Some(Area::ICloud);
    }
    if under("Library/Application Support")
        || under("Library/Containers")
        || under("Library/Group Containers")
    {
        return Some(Area::AppData);
    }
    if under("Desktop") {
        return Some(Area::Desktop);
    }
    if under("Documents") {
        return Some(Area::Documents);
    }
    if under("Downloads") {
        return Some(Area::Downloads);
    }
    None
}

/// Is `e` the kernel refusing on TCC's behalf?
///
/// TCC denials arrive as **EPERM (1)**, not the EACCES (13) a mode bit would
/// give — and Rust maps both to `PermissionDenied`, so the kind alone cannot
/// tell a privacy denial from an ordinary unreadable directory. Both are
/// accepted here because both are a permission the user can restore, and
/// because a file-mode problem inside a protected area is indistinguishable
/// from the outside anyway.
fn is_denied(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(1) | Some(13))
        || e.kind() == std::io::ErrorKind::PermissionDenied
}

/// The denial for `path`, if reading it right now is refused AND it sits
/// somewhere macOS gates.
///
/// `None` covers every ordinary case: the path reads fine, it is missing, or
/// it is refused somewhere TCC has no say (a `chmod 000` directory in a
/// project, say) — none of which a privacy card should claim to fix.
///
/// `is_macos` is the platform, and when it is false the answer is `None`
/// before anything is read: there is no TCC to have refused (see the module
/// note). Callers pass `cfg!(target_os = "macos")`.
pub fn denial_at(path: &Path, home: &Path, bundle_id: &str, is_macos: bool) -> Option<Denial> {
    if !is_macos {
        return None;
    }
    let refused = match std::fs::read_dir(path) {
        Ok(_) => false,
        Err(e) => is_denied(&e),
    };
    // A FILE is legitimate here too — the Files tab opens one — so fall back
    // to a metadata read before concluding anything.
    let refused = refused
        && match std::fs::metadata(path) {
            Ok(_) => true,
            Err(e) => is_denied(&e),
        };
    if !refused {
        return None;
    }
    let area = area_of(path, home)?;
    Some(Denial {
        area,
        label: area.label().to_string(),
        path: path.to_string_lossy().to_string(),
        pane: area.pane().to_string(),
        reset_command: format!("tccutil reset {} {bundle_id}", area.service()),
    })
}

/// `$HOME`, or `/` when it is unset — which makes every `under()` above false
/// and leaves only the volume checks, rather than matching `/Desktop`.
pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/Users/t";
    fn home() -> PathBuf {
        PathBuf::from(HOME)
    }

    #[test]
    fn each_protected_area_is_recognised_by_path() {
        for (p, want) in [
            ("/Users/t/Desktop/repo", Area::Desktop),
            ("/Users/t/Documents/work/repo", Area::Documents),
            ("/Users/t/Downloads/x", Area::Downloads),
            ("/Users/t/Library/Mobile Documents/com~apple~CloudDocs/r", Area::ICloud),
            ("/Users/t/Library/CloudStorage/Dropbox/r", Area::ICloud),
            ("/Users/t/Library/Application Support/Code", Area::AppData),
            ("/Users/t/Library/Containers/com.other.app", Area::AppData),
            ("/Volumes/SSD/repo", Area::Removable),
            ("/net/fileserver/repo", Area::Network),
        ] {
            assert_eq!(area_of(Path::new(p), &home()), Some(want), "{p}");
        }
    }

    #[test]
    fn an_ordinary_path_is_not_a_protected_area() {
        // The common case by far — a repo in ~/Projects or /tmp. A card here
        // would blame macOS for an error it had nothing to do with.
        for p in ["/Users/t/Projects/repo", "/tmp/x", "/Users/t/.claude", "/opt/src"] {
            assert_eq!(area_of(Path::new(p), &home()), None, "{p}");
        }
    }

    #[test]
    fn cloud_storage_is_not_mistaken_for_app_data() {
        // Both live under ~/Library, and `CloudStorage` is where every File
        // Provider mount (Dropbox, OneDrive, Google Drive) lands. Ordering the
        // Library checks the other way round files a user's Dropbox repo under
        // "another app's data" and sends them to the wrong Settings pane.
        let p = Path::new("/Users/t/Library/CloudStorage/GoogleDrive-a@b.c/repo");
        assert_eq!(area_of(p, &home()), Some(Area::ICloud));
        assert_ne!(area_of(p, &home()), Some(Area::AppData));
    }

    #[test]
    fn a_home_on_an_external_disk_still_reports_the_volume() {
        // `/Volumes` is checked first precisely for this: with $HOME on an
        // external disk every `under()` matches, and the user would be sent to
        // Files and Folders for a disk that needs Removable Volumes.
        let ext = PathBuf::from("/Volumes/Ext/Users/t");
        assert_eq!(area_of(Path::new("/Volumes/Ext/Users/t/Documents/r"), &ext), Some(Area::Removable));
    }

    #[test]
    fn every_area_carries_a_service_and_a_pane() {
        // The pair is data on the enum so neither can be forgotten; this is
        // what makes that true rather than merely intended.
        for a in [
            Area::Desktop, Area::Documents, Area::Downloads, Area::ICloud,
            Area::Removable, Area::Network, Area::AppData, Area::Other,
        ] {
            assert!(!a.service().is_empty(), "{a:?} has no tccutil service");
            assert!(a.pane().starts_with("x-apple.systempreferences:"), "{a:?} pane: {}", a.pane());
            assert!(!a.label().is_empty(), "{a:?} has no label");
        }
    }

    #[test]
    fn every_pane_is_one_the_opener_may_open() {
        // `opener:default` scopes `open_url` to http(s), mailto and tel, so a
        // pane the capability file does not name is a Settings button that
        // rejects with `ForbiddenUrl` — in the real app only, since the mock's
        // `openUrl` has no scope. The plugin matches each `url` as a glob; the
        // entries here are all `<prefix>*`, and a fancier one should be read by
        // hand rather than guessed at.
        let caps: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        let prefixes: Vec<&str> = caps["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["identifier"] == "opener:allow-open-url")
            .flat_map(|p| p["allow"].as_array().into_iter().flatten())
            .filter_map(|e| e["url"].as_str())
            .map(|u| {
                let pre = u.strip_suffix('*').unwrap_or_else(|| panic!("not a <prefix>* glob: {u}"));
                assert!(!pre.contains(['*', '?', '[']), "not a <prefix>* glob: {u}");
                pre
            })
            .collect();
        for a in [
            Area::Desktop, Area::Documents, Area::Downloads, Area::ICloud,
            Area::Removable, Area::Network, Area::AppData, Area::Other,
        ] {
            assert!(
                prefixes.iter().any(|pre| a.pane().starts_with(pre)),
                "{a:?}: capabilities/default.json does not let the opener open {}",
                a.pane()
            );
        }
    }

    #[test]
    fn a_readable_directory_is_never_a_denial() {
        let d = std::env::temp_dir();
        assert!(denial_at(&d, &home(), "net.example.app", true).is_none());
    }

    #[test]
    fn a_missing_path_is_not_a_denial() {
        // NotFound, not PermissionDenied — and a card offering to unlock a
        // folder that does not exist is worse than the raw error.
        let p = std::env::temp_dir().join("wt-macprivacy-absent-xyz");
        let _ = std::fs::remove_dir_all(&p);
        assert!(denial_at(&p, &home(), "net.example.app", true).is_none());
    }

    #[test]
    fn a_refused_path_outside_a_protected_area_is_not_claimed() {
        // A `chmod 000` directory in a project refuses exactly like a TCC
        // denial does. It is not one, and Settings cannot fix it.
        let d = std::env::temp_dir().join(format!("wt-macprivacy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o000)).unwrap();
            // Refused, but `area_of` says None, so no card.
            assert!(denial_at(&d, &home(), "net.example.app", true).is_none());
            std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    /// `<fake home>/Documents/repo`, unreadable: a denial's exact shape, made
    /// with a mode bit so it exists on every OS. Drop restores the mode first,
    /// since `remove_dir_all` cannot descend a `000` directory.
    #[cfg(unix)]
    struct RefusedDocs {
        home: PathBuf,
        path: PathBuf,
    }

    #[cfg(unix)]
    impl RefusedDocs {
        fn new(tag: &str) -> Self {
            use std::os::unix::fs::PermissionsExt;
            let home = std::env::temp_dir().join(format!("wt-macprivacy-{tag}-{}", std::process::id()));
            let path = home.join("Documents/repo");
            let _ = std::fs::remove_dir_all(&home);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
            // Without a refusal both directions below pass for nothing — the
            // macOS one fails, but the off-macOS one would assert `None` of a
            // path that reads fine. Root reads a `000` directory.
            assert!(std::fs::read_dir(&path).is_err(), "fixture {path:?} must be unreadable (running as root?)");
            assert_eq!(area_of(&path, &home), Some(Area::Documents));
            RefusedDocs { home, path }
        }
    }

    #[cfg(unix)]
    impl Drop for RefusedDocs {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o755));
            let _ = std::fs::remove_dir_all(&self.home);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_refused_protected_path_is_a_denial_on_macos() {
        let f = RefusedDocs::new("mac");
        let d = denial_at(&f.path, &f.home, "net.example.app", true).expect("a denial");
        assert_eq!(d.area, Area::Documents);
        assert_eq!(d.path, f.path.to_string_lossy());
        assert_eq!(d.reset_command, "tccutil reset SystemPolicyDocumentsFolder net.example.app");
    }

    #[cfg(unix)]
    #[test]
    fn off_macos_a_refused_protected_path_is_not_a_denial() {
        // The same refused `~/Documents` on Linux: no TCC refused it, and a
        // card naming macOS, System Settings and `tccutil` would be wrong on
        // every line. The raw error stands.
        let f = RefusedDocs::new("other");
        assert!(denial_at(&f.path, &f.home, "net.example.app", false).is_none());
    }

    #[test]
    fn the_reset_command_is_whole_and_names_the_right_service() {
        let d = Denial {
            area: Area::Documents,
            label: Area::Documents.label().into(),
            path: "/Users/t/Documents/r".into(),
            pane: Area::Documents.pane().into(),
            reset_command: format!("tccutil reset {} {}", Area::Documents.service(), "net.casadelvalle.worktrees"),
        };
        assert_eq!(d.reset_command, "tccutil reset SystemPolicyDocumentsFolder net.casadelvalle.worktrees");
    }
}
