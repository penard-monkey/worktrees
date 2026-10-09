//! The plan hook and the Plan tab must name the same plan
//! (`docs/proposals/owned-planning.md` §7 item 4).
//!
//! Both entry points of ONE binary: `plan resolve --json` is `summarize_with`,
//! the function behind the tab and MCP; `plan hook session` is what Claude
//! injects. For every scenario they must agree on `plan_rel` — including
//! agreeing that there is none. valleos's `plan-context.sh` and cdv #684's port
//! disagreed with the tab on exactly these shapes (a linked `.planning/<name>`
//! injected while the tab refused it).
//!
//! The scenarios are cdv's four (main on `orchestrator`; a lane on its own
//! topic beside the copied `orchestrator/`; a root-layout lane beside it; a
//! lane with neither), plus a symlinked `.planning`, a `../` pointer and
//! `.active_plan = evil` with `.planning/evil -> elsewhere`, which must be
//! `invalid_pointer` and print no path at all.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_worktrees")
}

struct Sandbox {
    dir: PathBuf,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Sandbox {
    fn new() -> Sandbox {
        let dir = std::env::temp_dir().join(format!("wt-plan-parity-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("home")).unwrap();
        let dir = std::fs::canonicalize(&dir).unwrap();
        Sandbox { dir }
    }
    fn home(&self) -> PathBuf {
        self.dir.join("home")
    }
    fn run(&self, cwd: &Path, args: &[&str], stdin: &str) -> String {
        use std::io::Write;
        let mut c = Command::new(bin());
        c.args(args)
            .current_dir(cwd)
            .env("HOME", self.home())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_DATA_HOME")
            .env_remove("XDG_STATE_HOME")
            .env_remove("CLAUDE_PROJECT_DIR")
            .env_remove("TMUX")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = c.spawn().unwrap();
        child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into_owned()
    }
    fn git(&self, cwd: &Path, args: &[&str]) {
        let st = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("HOME", self.home())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(st.success(), "git {args:?}");
    }
}

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
}

/// `(how_resolved, plan_rel)` from the tab's side.
fn tab(sb: &Sandbox, place: &Path) -> (String, Option<String>) {
    let v: serde_json::Value = serde_json::from_str(&sb.run(place, &["plan", "resolve", "--json"], "")).unwrap();
    assert_eq!(v["level"], "full", "the scenario must run owned: {v}");
    assert_eq!(v["version"], env!("CARGO_PKG_VERSION"), "resolve --json stamps the version");
    (v["how_resolved"].as_str().unwrap_or("null").to_string(), v["plan_rel"].as_str().map(String::from))
}

/// `plan_rel` from the hook's side: the backticked path on its `Plan:` line.
fn hook(sb: &Sandbox, place: &Path) -> (String, Option<String>) {
    let input = format!("{{\"session_id\":\"parity\",\"cwd\":{}}}", serde_json::json!(place.to_string_lossy()));
    let out = sb.run(place, &["plan", "hook", "session"], &input);
    let rel = out
        .lines()
        .find_map(|l| l.strip_prefix("Plan: `"))
        .and_then(|r| r.split_once('`'))
        .map(|(rel, _)| rel.to_string());
    (out, rel)
}

#[test]
fn the_hook_and_the_tab_name_the_same_plan_in_every_scenario() {
    use std::os::unix::fs::symlink;
    let sb = Sandbox::new();
    let main = sb.dir.join("repo");
    std::fs::create_dir_all(&main).unwrap();
    sb.git(&main, &["init", "-q", "-b", "main"]);
    sb.git(&main, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "--allow-empty", "-m", "init"]);
    sb.run(&main, &["projects", "add"], "");
    sb.run(&main, &["plan", "level", "full"], "");
    let out = sb.dir.join("elsewhere");
    write(&out, "task_plan.md", "# outside — must never be read\n");
    write(&out, ".active_plan", "x\n");

    let lane = |name: &str| -> PathBuf {
        let p = main.join(".worktrees").join(name);
        std::fs::create_dir_all(&p).unwrap();
        p
    };
    let orchestrator = |p: &Path| write(p, ".planning/orchestrator/task_plan.md", "# goals copy\n");

    // 1. main on its orchestrator plan
    write(&main, ".planning/.active_plan", "orchestrator\n");
    orchestrator(&main);
    // 2. a lane on its own topic, beside the copied orchestrator dir
    let own = lane("own-topic");
    orchestrator(&own);
    write(&own, ".planning/own-topic/task_plan.md", "# lane plan\n### Phase 1: go\n- [ ] x\n");
    write(&own, ".planning/.active_plan", "own-topic\n");
    // 3. a root-layout lane beside it (no pointer)
    let rootlane = lane("root-layout");
    orchestrator(&rootlane);
    write(&rootlane, "task_plan.md", "# root plan\n");
    // 4. a lane with neither (the orchestrator copy must NOT be guessed)
    let bare = lane("neither");
    orchestrator(&bare);
    // 5. a symlinked .planning
    let linked = lane("linked-planning");
    symlink(&out, linked.join(".planning")).unwrap();
    // 6. a `../` pointer, with a root plan to fall back to
    let dotdot = lane("dotdot");
    write(&dotdot, ".planning/.active_plan", "../x\n");
    write(&dotdot, "task_plan.md", "# root fallback\n");
    // 7. `.active_plan = evil`, `.planning/evil -> elsewhere`, nothing to fall back to
    let evil = lane("evil");
    write(&evil, ".planning/.active_plan", "evil\n");
    symlink(&out, evil.join(".planning/evil")).unwrap();
    // 8. a valid pointer whose plan is not written yet
    let pending = lane("pending");
    write(&pending, ".planning/.active_plan", "pending\n");
    std::fs::create_dir_all(pending.join(".planning/pending")).unwrap();

    let want: Vec<(&Path, &str, Option<&str>)> = vec![
        (&main, "active_plan", Some(".planning/orchestrator/task_plan.md")),
        (&own, "active_plan", Some(".planning/own-topic/task_plan.md")),
        (&rootlane, "root", Some("task_plan.md")),
        (&bare, "null", None),
        (&linked, "invalid_pointer", None),
        (&dotdot, "invalid_pointer", Some("task_plan.md")),
        (&evil, "invalid_pointer", None),
        (&pending, "pending", None),
    ];
    for (place, how, rel) in want {
        let (t_how, t_rel) = tab(&sb, place);
        let (h_out, h_rel) = hook(&sb, place);
        let name = place.file_name().unwrap().to_string_lossy();
        assert_eq!((t_how.as_str(), t_rel.as_deref()), (how, rel), "{name}: the tab");
        assert_eq!(h_rel, t_rel, "{name}: the hook named a different plan than the tab:\n{h_out}");
        assert!(!h_out.contains("outside"), "{name}: read through a link:\n{h_out}");
        if how == "invalid_pointer" {
            assert!(h_out.contains("`.active_plan` is not usable"), "{name}:\n{h_out}");
            assert!(!h_out.contains("evil") && !h_out.contains("../x"), "{name}: echoed the pointer:\n{h_out}");
            assert!(!h_out.contains("goes in"), "{name}: told the agent where to write:\n{h_out}");
        }
        if how == "pending" {
            assert!(h_out.contains("goes in `.planning/pending/`"), "{name}:\n{h_out}");
        }
    }

    // and the moment planning is off, the hook goes silent — no relaunch
    sb.run(&main, &["plan", "level", "off"], "");
    let (h_out, _) = hook(&sb, &own);
    assert_eq!(h_out, "", "the hook still spoke with planning off");
}
