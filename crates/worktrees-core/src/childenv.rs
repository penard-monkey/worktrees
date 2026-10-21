//! What a child we start FOR THE USER must not inherit from an AppImage launch.
//!
//! An AppImage starts the app through its AppRun — AppImageKit's classic one,
//! which tauri's linuxdeploy bundles — and linuxdeploy-plugin-gtk's hook, and
//! between them they put the AppDir, a mount that exists only while the app
//! runs (`$APPDIR`), at the FRONT of `PATH`, `LD_LIBRARY_PATH`,
//! `XDG_DATA_DIRS`, `GI_TYPELIB_PATH`, `PYTHONPATH`, `PERLLIB`,
//! `QT_PLUGIN_PATH` and more, and point single values into it (`GTK_PATH`,
//! `GDK_PIXBUF_MODULE_FILE`, …). The app needs them, and so do WebKit's own
//! child processes, which find the bundled GTK stack through them — so they
//! stay in THIS process. But everything we spawn inherits them too: a tmux
//! server the app brings up hands them to every pane, and the user's git, ssh
//! and python then load the AppImage's bundled libraries. (Measured: the
//! environment AppRun hands the app, dumped from a booted AppImage.)
//!
//! So EVERY process this workspace starts is made by [`command`] — tmux and
//! its panes, shells, editors, git, gh, ssh, curl, agents — and a pty's
//! `CommandBuilder` gets [`edits`] and [`start_dir`] (`term.rs`). Nothing we
//! spawn wants the AppImage's libraries (the processes that do are WebKit's,
//! and GLib starts those), and enumerating "user-facing" spawns is how two
//! login shells were first missed — so it is all of them, and
//! `app/scripts/childenv-check.mjs` fails on a bare `Command::new` outside
//! this file and tests. Outside an AppImage — macOS, a .deb, a dev build —
//! there are no edits at all.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// One change to a child's environment: set the variable to `Some(value)`, or
/// remove it (`None`).
pub type Edit = (OsString, Option<OsString>);

/// The AppDir this process runs from, when it is an AppImage launch.
pub fn appdir() -> Option<&'static Path> {
    static APPDIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    APPDIR
        .get_or_init(|| {
            let exe = std::env::current_exe().ok()?;
            appdir_from(std::env::var_os("APPDIR").as_deref(), &exe)
        })
        .as_deref()
}

/// The seam behind [`appdir`]: `$APPDIR` counts only when it is absolute, not
/// `/`, and contains this executable. The last test is what keeps a .deb or a
/// dev build a no-op when some profile merely exports an `APPDIR`.
pub fn appdir_from(var: Option<&OsStr>, exe: &Path) -> Option<PathBuf> {
    let d = PathBuf::from(var?);
    (d.is_absolute() && d.parent().is_some() && exe.starts_with(&d)).then_some(d)
}

/// What an AppImage launch sets that is not a path into the AppDir, so the
/// path rule cannot see it: the runtime's markers, and the two values
/// linuxdeploy-plugin-gtk's hook writes over whatever the user had —
/// `GDK_BACKEND=x11` and `GTK_THEME=Adwaita:<light|dark>`, which in a pane
/// would force every GTK program onto X11 and Adwaita. Matched by the hook's
/// own VALUE, so a backend or theme the user chose is left alone. `APPIMAGE`
/// matters most: a tauri app started from a pane reads it to decide that it is
/// an AppImage, and its updater would then rewrite ours.
fn launch_only(key: &OsStr, value: &OsStr) -> bool {
    match key.to_str() {
        Some("APPIMAGE" | "APPDIR" | "ARGV0" | "OWD") => true,
        Some("GDK_BACKEND") => value == "x11",
        Some("GTK_THEME") => value.to_str().is_some_and(|v| v.starts_with("Adwaita:")),
        _ => false,
    }
}

/// The edits for a child spawned NOW. Not cached: `PATH` changes after
/// startup (`fixup_gui_path`).
pub fn edits() -> Vec<Edit> {
    match appdir() {
        Some(d) => edits_in(std::env::vars_os(), d),
        None => Vec::new(),
    }
}

/// The rule, over any environment. Every variable is read as a `:` list, and
/// one with an entry inside the AppDir loses those entries; whatever is left
/// keeps its order, and a variable left with nothing is removed rather than
/// emptied. Empty entries go too, in the lists edited here and nowhere else:
/// AppRun builds `<its dirs>:<old value>`, so an unset original leaves a
/// trailing empty entry, which in `PATH` or `LD_LIBRARY_PATH` means "the
/// current directory". A single value inside the AppDir is a one-entry list,
/// and goes the same way. Generic on purpose: a variable a future AppRun adds
/// is covered without a list to keep up to date — and nothing of the user's
/// can point into a mount made at launch.
pub fn edits_in<I, K, V>(vars: I, appdir: &Path) -> Vec<Edit>
where
    I: IntoIterator<Item = (K, V)>,
    K: Into<OsString>,
    V: Into<OsString>,
{
    let mut out = Vec::new();
    for (k, v) in vars {
        let (k, v) = (k.into(), v.into());
        if launch_only(&k, &v) {
            out.push((k, None));
            continue;
        }
        let parts: Vec<PathBuf> = std::env::split_paths(&v).collect();
        if !parts.iter().any(|p| p.starts_with(appdir)) {
            continue;
        }
        let keep: Vec<PathBuf> = parts
            .into_iter()
            .filter(|p| !p.as_os_str().is_empty() && !p.starts_with(appdir))
            .collect();
        // join_paths fails only on an entry holding the separator, and these
        // came out of split_paths — but if it ever did, unset beats leaking.
        let joined = if keep.is_empty() { None } else { std::env::join_paths(keep).ok() };
        out.push((k, joined));
    }
    out
}

/// Where a child should start when its caller does not say. AppRun `chdir`s to
/// `$APPDIR/usr` before it starts the app (the `PWD` it leaves behind says so,
/// measured), and a child that inherits that — a tmux server above all, which
/// outlives the app — keeps a directory inside the AppImage's mount as its
/// own. `None` unless the cwd is inside the AppDir, so always `None` outside
/// an AppImage.
pub fn start_dir() -> Option<PathBuf> {
    let d = appdir()?;
    start_dir_in(&std::env::current_dir().ok()?, d, std::env::var_os("HOME").as_deref())
}

/// The seam behind [`start_dir`]: `$HOME`, or `/` when that is unusable too.
pub fn start_dir_in(cwd: &Path, appdir: &Path, home: Option<&OsStr>) -> Option<PathBuf> {
    if !cwd.starts_with(appdir) {
        return None;
    }
    let home = home.map(PathBuf::from).filter(|h| h.is_absolute() && !h.starts_with(appdir));
    Some(home.unwrap_or_else(|| PathBuf::from("/")))
}

/// `Command::new(program)` with [`scrub`] already applied: how this workspace
/// starts a process. The call site's own `env`/`current_dir` come after, so
/// they still win.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut c = Command::new(program);
    scrub(&mut c);
    c
}

/// [`edits`] and [`start_dir`], applied to a std `Command`; returned for
/// chaining. For a `Command` made elsewhere (a test's, or one handed to
/// `TmuxServer::configure`); [`command`] already does this.
pub fn scrub(cmd: &mut Command) -> &mut Command {
    if let Some(d) = start_dir() {
        cmd.current_dir(d);
    }
    apply(cmd, &edits())
}

/// The seam behind [`scrub`].
pub fn apply<'a>(cmd: &'a mut Command, edits: &[Edit]) -> &'a mut Command {
    for (k, v) in edits {
        match v {
            Some(v) => cmd.env(k, v),
            None => cmd.env_remove(k),
        };
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const APPDIR: &str = "/tmp/.mount_worktrQx7Kp2";

    /// The environment AppRun handed the app in a booted AppImage (dumped by
    /// wrapping the main binary, scratch HOME; the mount renamed to APPDIR),
    /// after `fixup_gui_path` put the login shell's PATH in front — plus the
    /// user's own variables, which the rule must not touch.
    fn launched() -> Vec<(String, String)> {
        let a = APPDIR;
        [
            ("PATH", format!("/usr/local/bin:/usr/bin:/home/u/.local/bin:{a}/usr/bin/:{a}/usr/sbin/:{a}/usr/games/:{a}/bin/:{a}/sbin/:/usr/local/sbin:/usr/sbin:/bin")),
            ("LD_LIBRARY_PATH", format!("{a}/usr/lib/:{a}/usr/lib/i386-linux-gnu/:{a}/usr/lib/x86_64-linux-gnu/:{a}/usr/lib32/:{a}/usr/lib64/:{a}/lib/:{a}/lib/i386-linux-gnu/:{a}/lib/x86_64-linux-gnu/:{a}/lib32/:{a}/lib64/:")),
            ("XDG_DATA_DIRS", format!("{a}/usr/share/:{a}/usr/share:/usr/share:")),
            ("GSETTINGS_SCHEMA_DIR", format!("{a}/usr/share/glib-2.0/schemas/:{a}//usr/share/glib-2.0/schemas")),
            ("GI_TYPELIB_PATH", format!("{a}//usr/lib/girepository-1.0")),
            ("GIO_EXTRA_MODULES", format!("{a}/usr/lib/aarch64-linux-gnu/gio/modules")),
            ("GDK_PIXBUF_MODULE_FILE", format!("{a}//usr/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache")),
            ("GTK_DATA_PREFIX", a.to_string()),
            ("GTK_EXE_PREFIX", format!("{a}//usr")),
            ("GTK_IM_MODULE_FILE", format!("{a}//usr/lib/gtk-3.0/3.0.0/immodules.cache")),
            ("GTK_PATH", format!("{a}//usr/lib/gtk-3.0")),
            ("GST_PLUGIN_SYSTEM_PATH", format!("{a}/usr/lib/gstreamer:")),
            ("GST_PLUGIN_SYSTEM_PATH_1_0", format!("{a}/usr/lib/gstreamer-1.0:")),
            ("PERLLIB", format!("{a}/usr/share/perl5/:{a}/usr/lib/perl5/:")),
            ("PYTHONPATH", format!("{a}/usr/share/pyshared/:/home/u/py")),
            ("QT_PLUGIN_PATH", format!("{a}/usr/lib/qt4/plugins/:{a}/usr/lib/qt5/plugins/:")),
            ("APPDIR", a.to_string()),
            ("APPIMAGE", "/home/u/Applications/worktrees.AppImage".to_string()),
            ("ARGV0", "/home/u/Applications/worktrees.AppImage".to_string()),
            ("OWD", "/home/u".to_string()),
            ("GDK_BACKEND", "x11".to_string()),
            ("GTK_THEME", "Adwaita:light".to_string()),
            // The user's, and not ours to touch.
            ("HOME", "/home/u".to_string()),
            ("SHELL", "/bin/bash".to_string()),
            ("DISPLAY", ":0".to_string()),
            ("LANG", "C.UTF-8".to_string()),
            ("BROWSER", "firefox:https://example.com/a:b".to_string()),
            ("MANPATH", "/usr/local/man::/usr/share/man".to_string()),
            ("NEAR_MISS", format!("{a}x/usr/bin:usr/lib")),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
    }

    /// The child's environment after the edits.
    fn child(env: Vec<(String, String)>, appdir: &str) -> BTreeMap<String, String> {
        let edits = edits_in(env.clone(), Path::new(appdir));
        let mut m: BTreeMap<String, String> = env.into_iter().collect();
        for (k, v) in edits {
            let k = k.into_string().unwrap();
            match v {
                Some(v) => m.insert(k, v.into_string().unwrap()),
                None => m.remove(&k),
            };
        }
        m
    }

    #[test]
    fn appdir_entries_leave_every_list_and_the_users_keep_their_order() {
        let m = child(launched(), APPDIR);
        assert_eq!(m["PATH"], "/usr/local/bin:/usr/bin:/home/u/.local/bin:/usr/local/sbin:/usr/sbin:/bin");
        assert_eq!(m["XDG_DATA_DIRS"], "/usr/share");
        assert_eq!(m["PYTHONPATH"], "/home/u/py");
        // By path COMPONENT, as the rule reads it: NEAR_MISS's `…Kp2x/usr/bin`
        // contains the AppDir's name as a string and is rightly kept.
        for (k, v) in &m {
            assert!(
                !std::env::split_paths(v).any(|p| p.starts_with(APPDIR)),
                "{k}={v} still has an entry inside the AppDir"
            );
        }
    }

    /// AppRun builds `<its dirs>:<old value>`, so an unset original leaves a
    /// trailing EMPTY entry — which in PATH or LD_LIBRARY_PATH means "the
    /// current directory". A list with nothing of the user's left goes.
    #[test]
    fn a_list_left_with_nothing_of_the_users_is_unset_not_emptied() {
        let m = child(launched(), APPDIR);
        for k in [
            "LD_LIBRARY_PATH",
            "GSETTINGS_SCHEMA_DIR",
            "GI_TYPELIB_PATH",
            "GIO_EXTRA_MODULES",
            "GST_PLUGIN_SYSTEM_PATH",
            "GST_PLUGIN_SYSTEM_PATH_1_0",
            "PERLLIB",
            "QT_PLUGIN_PATH",
        ] {
            assert!(!m.contains_key(k), "{k} survived as {:?}", m.get(k));
        }
    }

    #[test]
    fn single_values_inside_the_appdir_are_unset() {
        let m = child(launched(), APPDIR);
        for k in ["APPDIR", "GTK_DATA_PREFIX", "GTK_EXE_PREFIX", "GTK_PATH", "GTK_IM_MODULE_FILE", "GDK_PIXBUF_MODULE_FILE"] {
            assert!(!m.contains_key(k), "{k} survived as {:?}", m.get(k));
        }
    }

    /// Not paths into the AppDir, so the path rule cannot see them. APPIMAGE
    /// matters most: a tauri app started from a pane reads it to decide that
    /// IT is the AppImage, and its updater would then rewrite ours.
    #[test]
    fn the_launch_markers_and_the_gtk_hooks_overrides_are_unset() {
        let m = child(launched(), APPDIR);
        for k in ["APPIMAGE", "ARGV0", "OWD", "GDK_BACKEND", "GTK_THEME"] {
            assert!(!m.contains_key(k), "{k} survived as {:?}", m.get(k));
        }
        // …but only the hook's own values: one the user chose is theirs.
        let mine = vec![("GDK_BACKEND".to_string(), "wayland".to_string()), ("GTK_THEME".to_string(), "Nordic".to_string())];
        assert!(edits_in(mine, Path::new(APPDIR)).is_empty());
    }

    #[test]
    fn what_never_names_the_appdir_is_not_touched() {
        let m = child(launched(), APPDIR);
        assert_eq!(m["HOME"], "/home/u");
        assert_eq!(m["DISPLAY"], ":0");
        assert_eq!(m["BROWSER"], "firefox:https://example.com/a:b");
        // an empty entry the USER wrote stays: only lists we edit are tidied
        assert_eq!(m["MANPATH"], "/usr/local/man::/usr/share/man");
        // a sibling whose NAME starts like the AppDir is not inside it
        assert_eq!(m["NEAR_MISS"], format!("{APPDIR}x/usr/bin:usr/lib"));
        let edited: Vec<String> = edits_in(launched(), Path::new(APPDIR))
            .into_iter()
            .map(|(k, _)| k.into_string().unwrap())
            .collect();
        for k in ["HOME", "SHELL", "DISPLAY", "LANG", "BROWSER", "MANPATH", "NEAR_MISS"] {
            assert!(!edited.iter().any(|e| e == k), "{k} was edited");
        }
    }

    /// macOS, a .deb, a dev build: no AppDir, so no edits — including when an
    /// `APPDIR` is merely exported by someone's profile, which is why the
    /// executable has to be INSIDE it.
    #[test]
    fn outside_an_appimage_there_is_no_appdir_and_no_edits() {
        let exe = Path::new("/tmp/.mount_worktrQx7Kp2/usr/bin/worktrees-app");
        assert_eq!(appdir_from(Some(OsStr::new(APPDIR)), exe), Some(PathBuf::from(APPDIR)));
        assert_eq!(appdir_from(None, exe), None);
        assert_eq!(appdir_from(Some(OsStr::new("")), exe), None);
        assert_eq!(appdir_from(Some(OsStr::new("/")), exe), None);
        assert_eq!(appdir_from(Some(OsStr::new("tmp/.mount_worktrQx7Kp2")), exe), None);
        let deb = Path::new("/usr/bin/worktrees-app");
        assert_eq!(appdir_from(Some(OsStr::new(APPDIR)), deb), None);
        let mac = Path::new("/Applications/worktrees.app/Contents/MacOS/app");
        assert_eq!(appdir_from(Some(OsStr::new(APPDIR)), mac), None);
        // and this test process (target/…/deps) is not an AppImage either
        assert_eq!(appdir(), None);
        assert!(edits().is_empty());
    }

    #[test]
    fn a_child_never_starts_inside_the_appdir() {
        let a = Path::new(APPDIR);
        let usr = Path::new("/tmp/.mount_worktrQx7Kp2/usr");
        assert_eq!(start_dir_in(usr, a, Some(OsStr::new("/home/u"))), Some(PathBuf::from("/home/u")));
        // a HOME that is no better falls back to /
        assert_eq!(start_dir_in(usr, a, None), Some(PathBuf::from("/")));
        assert_eq!(start_dir_in(usr, a, Some(OsStr::new("relative"))), Some(PathBuf::from("/")));
        assert_eq!(start_dir_in(usr, a, Some(OsStr::new("/tmp/.mount_worktrQx7Kp2"))), Some(PathBuf::from("/")));
        // a cwd anywhere else is the caller's business
        assert_eq!(start_dir_in(Path::new("/home/u/repo"), a, Some(OsStr::new("/home/u"))), None);
        assert_eq!(start_dir_in(Path::new("/tmp/.mount_worktrQx7Kp2x"), a, Some(OsStr::new("/home/u"))), None);
        // and this test process is no AppImage
        assert_eq!(start_dir(), None);
    }

    #[test]
    fn apply_sets_and_removes_on_a_command() {
        let mut c = Command::new("true");
        apply(
            &mut c,
            &[("PATH".into(), Some("/usr/bin:/bin".into())), ("LD_LIBRARY_PATH".into(), None)],
        );
        let got: BTreeMap<_, _> = c.get_envs().map(|(k, v)| (k.to_owned(), v.map(OsStr::to_owned))).collect();
        assert_eq!(got.get(OsStr::new("PATH")), Some(&Some(OsString::from("/usr/bin:/bin"))));
        assert_eq!(got.get(OsStr::new("LD_LIBRARY_PATH")), Some(&None));
        // no edits, no change: the no-op that macOS and a .deb get
        let mut c = Command::new("true");
        apply(&mut c, &[]);
        assert_eq!(c.get_envs().count(), 0);
    }
}
