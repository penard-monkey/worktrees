//! Which editors are installed, so Settings → Commands can offer a pick list
//! instead of a free-text field whose default (`code`) may name nothing.
//!
//! Cheap on purpose: a `stat` per known bundle name in the two Applications
//! directories (plus JetBrains Toolbox's own folder), and a walk of `PATH` for
//! known launchers. No Spotlight, no LaunchServices query — this runs every
//! time the Commands page opens. `PATH` is the process's, which
//! `fixup_gui_path` has already replaced with the login shell's by the time
//! any command runs, so a Homebrew `code` or `zed` is found from a GUI launch.
//!
//! An APP entry's command is `open -a "<name>"`: it works for every bundle,
//! with or without a CLI shim, and `open_editor` appends the path as `"$0"`.
//! A CLI entry is offered beside it only where the launcher is actually on
//! PATH, because that is the form that opens a FOLDER as a project window
//! (`code <dir>`, `idea <dir>`) rather than handing it to Finder's notion of
//! the app.

use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Editor {
    pub label: String,
    /// What gets stored in `editor_cmd` verbatim.
    pub cmd: String,
    /// `"app"` or `"cli"`.
    pub kind: &'static str,
}

/// `(label, bundle names to look for, CLI launchers)`. The first bundle name
/// found wins, so the label is the product's, not whichever edition is there.
const KNOWN: &[(&str, &[&str], &[&str])] = &[
    ("Visual Studio Code", &["Visual Studio Code", "Visual Studio Code - Insiders"], &["code", "code-insiders"]),
    ("Cursor", &["Cursor"], &["cursor"]),
    ("Zed", &["Zed", "Zed Preview"], &["zed"]),
    ("Windsurf", &["Windsurf"], &["windsurf"]),
    ("Sublime Text", &["Sublime Text"], &["subl"]),
    ("Nova", &["Nova"], &["nova"]),
    ("BBEdit", &["BBEdit"], &["bbedit"]),
    ("TextMate", &["TextMate"], &["mate"]),
    ("Xcode", &["Xcode"], &[]),
    (
        "IntelliJ IDEA",
        &["IntelliJ IDEA", "IntelliJ IDEA Ultimate", "IntelliJ IDEA CE", "IntelliJ IDEA Community Edition"],
        &["idea"],
    ),
    ("WebStorm", &["WebStorm"], &["webstorm"]),
    ("PyCharm", &["PyCharm", "PyCharm Professional Edition", "PyCharm CE", "PyCharm Community Edition"], &["pycharm"]),
    ("GoLand", &["GoLand"], &["goland"]),
    ("RustRover", &["RustRover"], &["rustrover"]),
    ("CLion", &["CLion"], &["clion"]),
    ("PhpStorm", &["PhpStorm"], &["phpstorm"]),
    ("RubyMine", &["RubyMine"], &["rubymine"]),
    ("Fleet", &["Fleet"], &["fleet"]),
];

/// The directories an app bundle is looked for in, in order.
pub fn app_dirs(home: &Path) -> Vec<PathBuf> {
    vec![
        PathBuf::from("/Applications"),
        home.join("Applications"),
        // Toolbox's older layout; its newer one installs straight into
        // ~/Applications, which the line above already covers.
        home.join("Applications/JetBrains Toolbox"),
    ]
}

/// Installed editors on this machine.
pub fn detect() -> Vec<Editor> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    detect_in(&app_dirs(&home), &std::env::var("PATH").unwrap_or_default())
}

/// The seam: `dirs` stands in for the Applications folders and `path_var` for
/// `$PATH`, so a test can point both at a temp tree.
pub fn detect_in(dirs: &[PathBuf], path_var: &str) -> Vec<Editor> {
    let mut apps = Vec::new();
    let mut clis = Vec::new();
    for (label, bundles, launchers) in KNOWN {
        let found = bundles
            .iter()
            .find(|b| dirs.iter().any(|d| d.join(format!("{b}.app")).is_dir()));
        if let Some(b) = found {
            apps.push(Editor { label: (*label).into(), cmd: format!("open -a \"{b}\""), kind: "app" });
        }
        // One CLI entry per product: `code` and `code-insiders` are the same
        // choice to a person picking from a list.
        if let Some(l) = launchers.iter().find(|l| on_path(l, path_var)) {
            clis.push(Editor { label: format!("{label} ({l} command)"), cmd: (*l).into(), kind: "cli" });
        }
    }
    apps.extend(clis);
    apps
}

/// Is `name` an executable regular file in some `PATH` entry?
fn on_path(name: &str, path_var: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path_var.split(':').filter(|d| !d.is_empty()).any(|d| {
        // metadata follows links — Homebrew's `code` is a symlink into the app
        std::fs::metadata(Path::new(d).join(name))
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wt-editors-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn exe(p: &Path, mode: u32) {
        std::fs::write(p, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    /// Bundles in either Applications folder (and Toolbox's) become `open -a`
    /// entries, named by the PRODUCT even when an edition variant is what is
    /// installed; launchers on PATH become CLI entries after every app.
    #[test]
    fn finds_bundles_and_path_launchers() {
        let t = tmp("find");
        let (sys, user, tb, bin) = (t.join("A"), t.join("UA"), t.join("UA/JetBrains Toolbox"), t.join("bin"));
        for d in [&sys, &user, &tb, &bin] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::create_dir(sys.join("Visual Studio Code.app")).unwrap();
        std::fs::create_dir(user.join("Zed.app")).unwrap();
        std::fs::create_dir(tb.join("IntelliJ IDEA CE.app")).unwrap();
        exe(&bin.join("code"), 0o755);
        exe(&bin.join("subl"), 0o755);
        let got = detect_in(&[sys, user, tb], &format!("/nonexistent:{}", bin.display()));
        let flat: Vec<(String, String, &str)> = got.into_iter().map(|e| (e.label, e.cmd, e.kind)).collect();
        assert_eq!(
            flat,
            vec![
                ("Visual Studio Code".into(), "open -a \"Visual Studio Code\"".into(), "app"),
                ("Zed".into(), "open -a \"Zed\"".into(), "app"),
                ("IntelliJ IDEA".into(), "open -a \"IntelliJ IDEA CE\"".into(), "app"),
                ("Visual Studio Code (code command)".into(), "code".into(), "cli"),
                ("Sublime Text (subl command)".into(), "subl".into(), "cli"),
            ]
        );
        let _ = std::fs::remove_dir_all(&t);
    }

    /// Nothing installed is an empty list, not an error — and a launcher that
    /// is not executable, or is a directory, is not a launcher. A FILE named
    /// like a bundle is not a bundle either.
    #[test]
    fn ignores_what_cannot_launch() {
        let t = tmp("none");
        let (apps, bin) = (t.join("A"), t.join("bin"));
        std::fs::create_dir_all(&apps).unwrap();
        std::fs::create_dir_all(bin.join("zed")).unwrap();
        exe(&bin.join("code"), 0o644);
        std::fs::write(apps.join("Cursor.app"), "").unwrap();
        assert_eq!(detect_in(&[apps], &bin.display().to_string()), vec![]);
        assert_eq!(detect_in(&[], ""), vec![]);
        let _ = std::fs::remove_dir_all(&t);
    }
}
