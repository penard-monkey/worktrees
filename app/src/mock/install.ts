// Browser design harness: install a fake `window.__TAURI_INTERNALS__` so the real
// App.tsx runs in a plain browser (Vite) with a mocked, STATEFUL backend + rich
// fixtures. Loaded only when VITE_MOCK=1 (see main.tsx). Never bundled in prod.
//
// The command names here MUST track the real Tauri handlers (lib.rs). Unknown
// commands resolve to null + a console.warn, so new commands never hard-crash the
// harness during a redesign.

import { initialWorkspace, sessionName, type Place, type Workspace } from "./fixtures";

let ws: Workspace = initialWorkspace();

const clone = <T>(x: T): T => JSON.parse(JSON.stringify(x));
const now = () => Math.floor(Date.now() / 1000);

function findProject(root: string) {
  return ws.projects.find((p) => p.root === root);
}
function editPlace(repo: string, slug: string, fn: (p: Place) => void) {
  const pv = findProject(repo);
  const pl = pv?.snapshot?.places.find((p) => p.slug === slug);
  if (pl) fn(pl);
}
function reconcile(pl: Place) {
  const life = pl.declared?.lifecycle;
  if (life === "archived" || life === "abandoned") pl.lifecycle_effective = life;
  else if (life === "saved") pl.lifecycle_effective = "saved";
  else if (pl.tmux_session.up) pl.lifecycle_effective = "active";
  else if (pl.declared?.last_opened_epoch && now() - pl.declared.last_opened_epoch < 7 * 86400)
    pl.lifecycle_effective = "idle";
  else pl.lifecycle_effective = "closed";
}

let dialogCount = 0;
let mockCliVersion: string | null = "0.1.0"; // bumped by update_cli
// tmux is there unless the harness is asked to take it away (?notmux); see the
// tmux_check case for the two shapes.
const mockTmuxStuck = location.search.includes("notmux=stuck");
let mockTmux = !location.search.includes("notmux");
const MOCK_SETTINGS_KEY = "wt-mock-ui-state"; // sessionStorage: see get_settings

// ── project config / doctor / init (the Project sheet) ──────────────────────
// STATEFUL, like mockCliVersion: relink clears the drift findings (so the badge
// and the nav glyph both go away), and init_write makes the config exist (so the
// banner + the suggestion section retire). Without that the sheet's whole point
// — a repair you can see land — is unexercisable headlessly.
const CDV_ROOT = "/Users/demo/workspace/casadelvalle/casa-del-valle-monorepo";
const WT_ROOT = "/Users/demo/workspace/worktrees";

type MockCfg = {
  path: string;
  exists: boolean;
  files: { path: string; mode: string }[];
  ports: { stride: number; max_slots: number; base: [string, number][] } | null;
  compose: { files: string[]; project: string } | null;
  error: string | null;
  warnings: string[];
};

// ── AI profiles + skill store ────────────────────────────────────────────────
// Stateful, like mockConfigs: a write here is visible to the next read, so the
// editor can be driven end-to-end headlessly (Playwright) without a backend.
type MockProfile = {
  id: string; name: string; rules?: string; skills?: string[];
  inherit_global_skills?: boolean; inherit_global_mcp?: boolean; worktrees_mcp?: boolean;
  model?: string | null; updated_epoch?: number;
};
const mockProfiles: Record<string, MockProfile> = {
  work: {
    id: "work",
    name: "Work",
    // Realistic multi-line rules: this is the field the feature exists for, and
    // a one-liner makes the editor look like it takes one.
    rules:
      "Be succinct — no preamble, no restating the question.\n" +
      "Prefer small diffs. Explain a design choice only when it is not obvious.\n" +
      "When you change a public API, say so in the first line.\n" +
      "Never commit without running the gates.",
    skills: ["demo-skill"],
    updated_epoch: 1000,
  },
};
let mockDefaultProfile: string | null = "work";
const mockAssignments: Record<string, string> = {};
// Which profiles have ever run — drives the "needs sign-in" tag. `work` has,
// so the tag is only visible on a profile you create in the harness.
const mockLaunched = new Set<string>(["work"]);
type MockSkill = {
  name: string; description: string; capabilities: string[];
  source?: { kind: "local"; path: string } | { kind: "git"; url: string; rev: string; sha: string };
};
const mockSkills: Record<string, MockSkill> = {
  "demo-skill": {
    name: "demo-skill",
    description: "A demo skill, installed from a folder.",
    capabilities: [],
    source: { kind: "local", path: "/tmp/demo-skill" },
  },
  "risky-skill": {
    name: "risky-skill",
    description: "Pre-authorises tools — the review case.",
    capabilities: ["pre-authorises tools: Bash(rm:*)"],
    source: { kind: "git", url: "https://example.com/skills.git", rev: "", sha: "abc123def456" },
  },
};

const mockConfigs: Record<string, MockCfg> = {
  [CDV_ROOT]: {
    path: `${CDV_ROOT}/.worktrees.toml`,
    exists: true,
    files: [
      { path: ".env", mode: "link" },
      { path: "apps/mobile/google-services.json", mode: "link" },
      { path: "apps/backoffice/.env.local", mode: "copy" },
    ],
    ports: {
      stride: 100,
      max_slots: 50,
      base: [["API", 3001], ["BACKOFFICE", 3000], ["LS", 4566], ["PG", 5432], ["WEBSITE", 3002]],
    },
    // Two files, in docker's order: the base declares the named volumes, the
    // worktree override goes last (projcfg::Compose::files).
    compose: { files: ["docker-compose.yml", "docker-compose.worktree.yml"], project: "{prefix}-wt-{slug}" },
    error: null,
    warnings: ["future_thing = 1 — unknown key, ignored"],
  },
  [WT_ROOT]: {
    path: `${WT_ROOT}/.worktrees.toml`,
    exists: false,
    files: [],
    ports: null,
    compose: null,
    error: null,
    warnings: [],
  },
};

type MockFinding = {
  severity: "info" | "warn" | "error";
  code: string;
  place: string | null;
  path: string | null;
  message: string;
};
// Findings relink/provision are able to clear, per project root.
let mockFileFindings: Record<string, MockFinding[]> = {
  [CDV_ROOT]: [
    {
      severity: "error", code: "not-linked", place: "messaging", path: "apps/mobile/google-services.json",
      message: "apps/mobile/google-services.json is not linked here — the Android build will have no FCM sender id",
    },
    {
      // Verbatim from materialize::plan_one — including the fix text, which is
      // what makes it obvious whether the app can act on this finding at all.
      severity: "warn", code: "copy-stale", place: "billing-refactor", path: "apps/backoffice/.env.local",
      message:
        "copy `apps/backoffice/.env.local` differs from main and main is NEWER — " +
        "re-seed with: worktrees relink billing-refactor --force",
    },
  ],
};
let mockPortFindings: Record<string, MockFinding[]> = {
  [CDV_ROOT]: [
    {
      severity: "error", code: "no-slot", place: "kitchen-sink", path: ".worktree.env",
      message: "no `.worktree.env` here — scripts that key off that file will treat this as the MAIN checkout. Fix: worktrees provision kitchen-sink",
    },
    {
      severity: "info", code: "port-busy", place: "messaging", path: null,
      message: "API=3101 is already bound — expected while this place's stack is up",
    },
  ],
};

const SUGGESTED_TOML = `# .worktrees.toml — generated by 'worktrees init'. Commit it.

[[file]]
path = ".env"
# mode defaults to "link" — ONE source of truth; a per-worktree copy drifts.

[[file]]
path = "app/.env.local"          # credential: fails SILENTLY when missing
`;
const mockSuggestions: Record<string, any> = {
  [WT_ROOT]: {
    path: `${WT_ROOT}/.worktrees.toml`,
    exists: false,
    qualifies: true,
    files: [{ path: ".env", credential: false }, { path: "app/.env.local", credential: true }],
    credentials: 1,
    ports: false,
    compose: false,
    stale_places: ["feat-redesign"],
    truncated: false,
    toml: SUGGESTED_TOML,
    hash: "a1b2c3d4e5f60718",
  },
};

// ── virtual FS for the dock's Files tab ──────────────────────────────────────
// Deterministic + lazily materialized from whatever worktree path the tree
// asks for, so ANY fixture place works headlessly. Dirs seed their children on
// first listing; files seed content so read/write round-trips.
type MockEntry = { name: string; path: string; is_dir: boolean };
const fsChildren = new Map<string, MockEntry[]>();
const fsFiles = new Map<string, { content: string; binary: boolean; mtime: number }>();
// dock shell sidecars, keyed "repo|slug" → set of 1-based tab indices
const shellSidecars = new Map<string, Set<number>>();
// exited-but-kept shells (mirrors the real registry's try_wait liveness) — the
// restore path must see them dead, not just the transient shell:exit event
const deadShells = new Map<string, Set<number>>();
let shellGen = 0; // attach generation counter (real backend: per-shell)
const sidecarKey = (repo: string, slug: string) => `${repo}|${slug}`;

function seedDir(dir: string) {
  if (fsChildren.has(dir)) return fsChildren.get(dir)!;
  const base = dir.split("/").pop() || dir;
  const mk = (name: string, is_dir: boolean): MockEntry => ({ name, path: `${dir}/${name}`, is_dir });
  let entries: MockEntry[];
  if (base === "src") entries = [mk("App.tsx", false), mk("main.rs", false), mk("lib.rs", false)];
  else if (base === "crates") entries = [mk("worktrees-core", true), mk("worktrees-cli", true)];
  else entries = [mk("src", true), mk("crates", true), mk("README.md", false), mk("Cargo.toml", false), mk(".gitignore", false)];
  fsChildren.set(dir, entries);
  for (const e of entries) {
    if (!e.is_dir && !fsFiles.has(e.path)) {
      fsFiles.set(e.path, { content: `// ${e.name}\n// mock content — ${e.path}\n\nfn main() {\n    println!("hello from the harness");\n}\n`, binary: false, mtime: Date.now() });
    }
  }
  return entries;
}

type Args = Record<string, any>;
async function mockInvoke(cmd: string, args: Args = {}): Promise<unknown> {
  switch (cmd) {
    case "list_workspace":
      return clone(ws);
    case "list_places":
      return clone(findProject(args.repo)?.snapshot ?? null);

    case "plugin:dialog|open": {
      // simulate a native folder pick → a fresh project path
      dialogCount += 1;
      return `/Users/demo/workspace/picked-${dialogCount}`;
    }
    case "add_project": {
      const root: string = args.dir;
      if (!findProject(root)) {
        const name = root.split("/").pop() || root;
        ws.projects.push({
          root, ok: true, error: null,
          snapshot: {
            repo: root, prefix: name,
            places: [{
              slug: "(main)", path: root, is_main: true, registered: true,
              branch: "main", detached: false, dirty: false, dirty_files: 0,
              ahead: 0, behind: 0, last_commit_subject: "initial commit",
              last_commit_epoch: now() - 86400,
              tmux_session: { name: sessionName(name, "(main)"), up: false },
              claude_session_present: false, declared: null, lifecycle_effective: "closed",
            }],
          },
        });
      }
      return clone(ws);
    }
    case "remove_project":
      ws.projects = ws.projects.filter((p) => p.root !== args.root);
      return clone(ws);

    case "set_lifecycle":
      editPlace(args.repo, args.slug, (p) => {
        p.declared = { ...(p.declared ?? {}), lifecycle: args.label };
        reconcile(p);
      });
      return null;
    case "set_pin":
      editPlace(args.repo, args.slug, (p) => {
        p.declared = { ...(p.declared ?? {}), pinned: args.on };
      });
      return null;
    case "set_note":
      editPlace(args.repo, args.slug, (p) => {
        p.declared = { ...(p.declared ?? {}), note: args.note || undefined };
      });
      return null;
    case "touch_place":
      editPlace(args.repo, args.slug, (p) => {
        p.declared = { ...(p.declared ?? {}), last_opened_epoch: now() };
        reconcile(p);
      });
      return null;

    case "new_place": {
      const pv = findProject(args.repo);
      const slug = (args.name || args.branch).replace(/\//g, "-");
      if (pv?.snapshot && !pv.snapshot.places.find((p) => p.slug === slug)) {
        pv.snapshot.places.push({
          slug, path: `${args.repo}/.worktrees/${slug}`, is_main: false, registered: true,
          branch: args.branch, detached: false, dirty: false, dirty_files: 0,
          ahead: 0, behind: 0, last_commit_subject: "wip", last_commit_epoch: now(),
          tmux_session: { name: sessionName(pv.snapshot.prefix, slug), up: true },
          claude_session_present: true,
          declared: { last_opened_epoch: now() }, lifecycle_effective: "active",
        });
      }
      // Return the computed slug (mirrors core's new_place contract) so the
      // frontend selects the right place headlessly.
      return { ok: true, code: 0, output: `Created worktree ${slug}`, slug };
    }
    case "open_place": {
      console.info("[mock] open_place", args); // includes args.fresh so headless tests can assert the flag
      editPlace(args.repo, args.slug, (p) => {
        p.tmux_session.up = true;
        p.claude_session_present = true;
        p.declared = { ...(p.declared ?? {}), last_opened_epoch: now() };
        reconcile(p);
      });
      return { ok: true, code: 0, output: `Opened ${args.slug}` };
    }
    case "close_place": {
      const pv = findProject(args.repo);
      const pl = pv?.snapshot?.places.find((p) => p.slug === args.slug);
      if (!pv?.snapshot || !pl) return { ok: false, code: 1, output: `No worktree '${args.slug}'` };
      // Mirror ops::close_one. A session whose name this tool did NOT write was
      // adopted by pane cwd — someone's own session, or one left under a
      // previous prefix — and kill-session takes the whole thing, every window.
      // So core stops with EXIT_NEEDS_CONFIRM (4) and names it; `yes` is the
      // user's word, collected by the frontend's two-click arm. A canonical
      // name is never a question.
      const canonical = sessionName(pv.snapshot.prefix, args.slug as string);
      const live = pl.tmux_session.name;
      // `session` is the name the frontend's arm displayed. Consent is bound to
      // it: if it is not what is live now, core kills nothing and asks again
      // naming the session that IS there.
      if (args.session && args.session !== live) {
        return {
          ok: false, code: 4, needs_confirm: live,
          output: `${args.session} is no longer the session in ${pl.path} — ${live} is.`,
        };
      }
      if (live !== canonical && !args.yes) {
        return {
          ok: false, code: 4, needs_confirm: live,
          output: `tmux ${live} was not opened under this repo's name (${canonical}) — adopted because a pane is cwd'd in ${pl.path}.`,
        };
      }
      editPlace(args.repo, args.slug, (p) => {
        // The adopted session is gone, so the name falls back to canonical —
        // what core's snapshot reports for a place with nothing live.
        p.tmux_session = { name: canonical, up: false };
        reconcile(p);
      });
      return { ok: true, code: 0, output: `closed tmux ${live} — worktree kept.` };
    }
    case "github_url":
      return `https://github.com/demo/${(args.repo as string).split("/").pop()}/tree/mock-branch`;
    case "fetch_origin":
      console.info("[mock] fetch_origin:", args); // no ahead/behind state to model in the harness
      return null;
    case "set_fetch_interval":
      console.info("[mock] set_fetch_interval:", args);
      return null;
    case "open_editor":
      console.info("[mock] open_editor:", args);
      return null;
    case "open_terminal":
      console.info("[mock] open_terminal:", args);
      return null;
    case "plugin:opener|open_url":
    case "plugin:opener|reveal_item_in_dir":
    case "plugin:opener|open_path":
      console.info("[mock] opener:", cmd, args);
      return null;

    // updater/process — no signed updates in the harness; check() sees "current"
    case "plugin:updater|check":
      console.info("[mock] updater check");
      return null;
    case "plugin:process|restart":
      console.info("[mock] relaunch requested");
      return null;

    // app log — mirror to the browser console in the harness
    case "log_event":
      console.info(`[mock applog ${args.level}]`, args.msg);
      return null;
    case "log_info":
      return { dir: "/Users/demo/Library/Logs/net.casadelvalle.worktrees", file: "/Users/demo/Library/Logs/net.casadelvalle.worktrees/app.log" };
    case "log_tail":
      return "2026-07-25 20:00:01Z [info] startup v0.2.1 PATH=/usr/bin:...\n2026-07-25 20:00:09Z [info] open messaging fresh=false ok repo=/Users/demo/workspace/cdv\n2026-07-25 20:01:12Z [warn] close api rc=1 repo=/Users/demo/workspace/cdv: no live session";

    // tmux presence → the missing-tmux banner. STATEFUL like mockCliVersion, so
    // the whole flow is exercisable headlessly: `?notmux` starts without tmux and
    // a Re-check (refresh:true, i.e. the backend re-resolving PATH) FINDS it —
    // banner retires, places refresh. `?notmux=stuck` never finds it, which is
    // the only way to see the "still not found" feedback.
    case "tmux_check":
      if (args.refresh && !mockTmuxStuck) mockTmux = true;
      return mockTmux;

    case "switch_place":
      editPlace(args.repo, args.slug, (p) => { p.branch = args.branch; });
      return { ok: true, code: 0, output: `Switched ${args.slug} → ${args.branch}` };
    // Branch names the status-bar combobox offers. Real backend unions local
    // heads with origin-only ones; here a fixed set plus whatever branches the
    // fixture places are actually on, so switching around stays coherent.
    case "list_branches": {
      const pv = findProject(args.repo);
      const onPlaces = (pv?.snapshot?.places ?? []).map((p) => p.branch).filter(Boolean) as string[];
      const branches = [...new Set([
        "main", "develop", "release/2026.07",
        "feat/messaging-sse", "feat/billing-v2", "feat/ui-redesign",
        "fix/backoffice-bug-fixes", "chore/deps-bump",
        ...onPlaces,
      ])].sort();
      const cur = pv?.snapshot?.places.find((p) => p.slug === args.slug)?.branch ?? "main";
      return { branches, current: cur, default_base: "main" };
    }
    case "remove_place": {
      // Mirror the real backend (ops.rs remove_one): refuse a DIRTY worktree
      // unless --force, WITHOUT deleting. This finally makes the error-banner
      // path reachable headlessly (several fixtures are dirty:true).
      console.info("[mock] remove_place:", args); // logs del_branch/force flags
      const pv = findProject(args.repo);
      const pl = pv?.snapshot?.places.find((p) => p.slug === args.slug);
      if (pl?.dirty && !args.force) {
        return {
          ok: false,
          code: 1,
          output:
            `Worktree '${args.slug}' has uncommitted changes:\n` +
            `  M …\n` +
            `Refusing to remove. Commit/stash, or pass --force.`,
        };
      }
      // del_branch is state-invisible here (no branch objects modeled) — the
      // place is removed either way; the flag is logged above.
      if (pv?.snapshot) pv.snapshot.places = pv.snapshot.places.filter((p) => p.slug !== args.slug);
      return { ok: true, code: 0, output: `Removed ${args.slug}` };
    }

    case "term_open": {
      // best-effort: render a canned banner into the xterm via the Channel
      const banner =
        "\x1b[38;5;110m worktrees \x1b[0m mock terminal — design harness\r\n" +
        "\x1b[90m(real tmux attach only in the Tauri app)\x1b[0m\r\n\r\n" +
        `\x1b[32m➜\x1b[0m  \x1b[36m${args.session}\x1b[0m $ \x1b[5m▌\x1b[0m\r\n`;
      const ch = args.onBytes;
      setTimeout(() => {
        try { ch?.onmessage?.(new TextEncoder().encode(banner).buffer); } catch { /* ignore */ }
      }, 40);
      return 1;
    }
    case "term_write":
    case "term_resize":
    case "term_close":
      return null;

    // ── dock Files tab (virtual FS) + Terminal shells ──
    case "list_dir":
      return seedDir(args.path as string);
    case "read_file": {
      const f = fsFiles.get(args.path as string);
      return f
        ? { content: f.content, truncated: false, binary: f.binary, mtime: f.mtime }
        : { content: `// ${args.path}\n`, truncated: false, binary: false, mtime: 0 };
    }
    case "write_file": {
      const path = args.path as string;
      const prev = fsFiles.get(path);
      // compare-and-swap: mirror the backend's stale-clobber guard
      if (args.expectedMtime != null && prev && prev.mtime !== args.expectedMtime) {
        throw new Error("file changed on disk since you opened it — reload to see the latest");
      }
      fsFiles.set(path, { content: args.content as string, binary: prev?.binary ?? false, mtime: Date.now() });
      return null;
    }
    // Dock shells are PTYs the backend OWNS (no tmux). The mock models the
    // registry the same way — spawn-or-reattach keyed by repo+slug+index — so a
    // tab flip re-renders the banner exactly where the real one replays its ring.
    case "shell_open": {
      const i = (args.index as number) ?? 1;
      const set = shellSidecars.get(sidecarKey(args.repo, args.slug)) ?? new Set<number>();
      set.add(i); shellSidecars.set(sidecarKey(args.repo, args.slug), set);
      const banner =
        "\x1b[38;5;110m worktrees \x1b[0m mock shell — design harness\r\n" +
        "\x1b[90m(a real login shell only in the Tauri app)\x1b[0m\r\n\r\n" +
        `\x1b[32m➜\x1b[0m  \x1b[36m${args.slug} sh ${i}\x1b[0m $ \x1b[5m▌\x1b[0m\r\n`;
      const ch = args.onBytes;
      setTimeout(() => {
        try { ch?.onmessage?.(new TextEncoder().encode(banner).buffer); } catch { /* ignore */ }
      }, 40);
      deadShells.get(sidecarKey(args.repo, args.slug))?.delete(i); // reattach of a restarted tab
      return ++shellGen; // attach generation — see shell_detach in lib.rs
    }
    case "shell_write":
    case "shell_resize":
    case "shell_detach": // detach keeps the shell alive — nothing to model
      return null;
    case "list_shell_sessions": {
      const deadSet = deadShells.get(sidecarKey(args.repo, args.slug)) ?? new Set<number>();
      return [...(shellSidecars.get(sidecarKey(args.repo, args.slug)) ?? new Set<number>())]
        .sort((a, b) => a - b)
        .map((index) => ({ index, dead: deadSet.has(index) }));
    }
    case "close_shell_session": {
      shellSidecars.get(sidecarKey(args.repo, args.slug))?.delete(args.index as number);
      deadShells.get(sidecarKey(args.repo, args.slug))?.delete(args.index as number);
      return null;
    }

    // update check — stateful: update_cli bumps the fake CLI so the badge-clear
    // and button-disappear transitions are exercisable in the harness
    case "check_update":
      return {
        app_version: "0.2.1",
        cli_version: mockCliVersion,
        cli_path: mockCliVersion ? "/Users/demo/.local/bin/worktrees" : null,
        latest: "v0.2.1",
      };
    case "update_cli": {
      mockCliVersion = (args.tag as string).replace(/^v/, "");
      return {
        ok: true, code: 0,
        output: `worktrees installer\n→ ${args.tag}: darwin/arm64 prebuilt\n✓ checksum verified\n✓ installed to ~/.local/bin/worktrees\nworktrees ${mockCliVersion}`,
      };
    }

    // AI command config (read-only, Phase 1). exists:false exercises the
    // reveal-parent fallback chain in SettingsSheet.
    case "get_ai_config":
      return { ai_cmd: "claude", ai_resume_arg: "-r", path: "/Users/demo/.config/worktrees/config", exists: false };

    // ── Project sheet: config view + the four verbs (lib.rs) ──
    case "project_config_read":
      return clone(mockConfigs[args.repo] ?? {
        path: `${args.repo}/.worktrees.toml`,
        exists: false, files: [], ports: null, compose: null, error: null, warnings: [],
      });
    case "doctor": {
      const root = args.repo as string;
      // The guard exit, modelled: cmd_doctor returns 1 BEFORE emitting any JSON
      // when the config does not parse, so the app gets `findings: []` with an
      // error — which is "nothing was measured", not "nothing is wrong". Any
      // consumer that reads findings without checking this is broken, and the
      // harness is where that shows up.
      const cfgErr = mockConfigs[root]?.error;
      if (cfgErr) {
        return { code: 1, schema_version: 1, findings: [], error: cfgErr };
      }
      const all = [...(mockFileFindings[root] ?? []), ...(mockPortFindings[root] ?? [])];
      const findings = args.slug ? all.filter((f) => f.place === args.slug) : all;
      return {
        code: findings.some((f) => f.severity === "error") ? 2 : 0,
        schema_version: 1,
        findings: clone(findings),
        error: null,
      };
    }
    case "relink": {
      // Stateful, and faithful to materialize::plan_one — which is the WHOLE
      // point of the harness. A plain relink NEVER touches an existing regular
      // file: a `copy-stale` and a `shadowed` both SURVIVE it, the run still
      // exits 0, and the warning rides out in `warnings` (run_op splits the Warn
      // lines out). Only `--force` moves the file aside as .bak and rewrites it.
      //
      // A mock that cleared the stale copy on a plain relink would demo a repair
      // the real backend does not perform — the badge would clear here and
      // persist against the real thing.
      const root = args.repo as string;
      const force = !!args.force;
      const before = mockFileFindings[root] ?? [];
      const survives = (f: MockFinding) => !force && (f.code === "copy-stale" || f.code === "shadowed");
      const kept = before.filter(survives);
      const repaired = before.filter((f) => !survives(f));
      mockFileFindings = { ...mockFileFindings, [root]: kept };
      const lines = [
        `═══ Relinking messaging ═══`,
        ...repaired.map((f) =>
          f.code === "copy-stale" || f.code === "shadowed"
            ? `▸ re-copied ${f.path ?? ""} (previous content kept as ${(f.path ?? "file").split("/").pop()}.bak)`
            : `▸ linked ${f.path ?? ""} -> ${root}/${f.path ?? ""}`,
        ),
        ...kept.map((f) => `! ${f.message}`),
      ];
      return {
        ok: true, code: 0,
        // Warn-severity lines only, exactly like CaptureUi::warnings().
        warnings: kept.filter((f) => f.severity === "warn").map((f) => f.message),
        output: lines.join("\n"),
      };
    }
    case "provision": {
      const root = args.repo as string;
      const kept = (mockPortFindings[root] ?? []).filter((f) => f.code !== "no-slot");
      mockPortFindings = { ...mockPortFindings, [root]: kept };
      return { ok: true, code: 0, warnings: [], output: "═══ Provisioning kitchen-sink ═══\n▸ slot 3 — wrote .worktree.env (5 ports)" };
    }
    case "init_suggest":
      return clone(mockSuggestions[args.repo] ?? {
        path: `${args.repo}/.worktrees.toml`,
        exists: !!mockConfigs[args.repo]?.exists,
        qualifies: false, files: [], credentials: 0, ports: false, compose: false,
        stale_places: [], truncated: false, toml: "", hash: "0000000000000000",
      });
    case "init_write": {
      // Stateful: the config now exists, so the banner and the suggestion
      // section retire on the next probe.
      const root = args.repo as string;
      mockConfigs[root] = {
        path: `${root}/.worktrees.toml`, exists: true,
        files: [{ path: ".env", mode: "link" }, { path: "app/.env.local", mode: "link" }],
        ports: null, compose: null, error: null, warnings: [],
      };
      if (mockSuggestions[root]) mockSuggestions[root] = { ...mockSuggestions[root], exists: true, qualifies: false };
      return { ok: true, code: 0, warnings: [], output: `▸ wrote ${root}/.worktrees.toml\n▸ Commit it — this is project structure, like docker-compose.yml.` };
    }

    // Copy diagnostics — canned block (the real command probes the login shell,
    // git/tmux, and tails app.log; none of that exists in the harness).
    case "diagnostics":
      return [
        "worktrees diagnostics",
        "=====================",
        "app version : 0.2.4",
        "cli version : 0.2.4",
        "cli path    : /Users/demo/.local/bin/worktrees",
        "",
        "PATH        : /opt/homebrew/bin:/usr/bin:/bin",
        "git         : git version 2.45.0 @ /opt/homebrew/bin/git",
        "tmux        : tmux 3.5a @ /opt/homebrew/bin/tmux",
        "",
        "core config",
        "-----------",
        "ai_cmd        : claude",
        "ai_resume_arg : -r",
        "config file   : /Users/demo/.config/worktrees/config (absent)",
        "",
        "log (last 200 lines)",
        "--------------------",
        "2026-07-25 20:00:01Z [info] startup v0.2.4 PATH=/usr/bin:...",
      ].join("\n");

    case "get_changelog":
      return {
        version: "0.2.2",
        changelog:
          "# Changelog\n\n## [Unreleased]\n\n## [0.2.2] - 2026-07-26\n\n### Added\n- Nav tier show/hide, sort modes (last-used / A–Z / manual drag), release\n  notes on update — hard-wrapped like the real CHANGELOG to exercise\n  bullet unwrapping.\n\n### Fixed\n- Multi-client size clamp left stale cells in the embedded terminal;\n  sessions now use `window-size latest` + `aggressive-resize`.\n\n## [0.2.1] - 2026-07-25\n\n### Added\n- App self-update.\n",
      };

    case "settings_info":
      return { dir: "/Users/demo/Library/Application Support/net.casadelvalle.worktrees", file: "/Users/demo/Library/Application Support/net.casadelvalle.worktrees/ui-state.json" };

    // settings — persisted in sessionStorage (per tab), so state that is only
    // meaningful ACROSS a reload is exercisable: an init banner dismissed by
    // content hash must stay dismissed. `?whatsnew` still forces a stale
    // last_seen_version → the What's-new sheet.
    case "get_settings": {
      if (location.search.includes("whatsnew")) return { last_seen_version: "0.2.0" };
      try {
        const raw = sessionStorage.getItem(MOCK_SETTINGS_KEY);
        return raw ? JSON.parse(raw) : null;
      } catch {
        return null;
      }
    }
    case "set_settings":
      try { sessionStorage.setItem(MOCK_SETTINGS_KEY, JSON.stringify(args.settings)); } catch { /* private mode */ }
      return null;

    // event system — handlers are registered and fired via emitEvent below
    case "plugin:event|listen": {
      (eventListeners[args.event] ??= new Set()).add(args.handler);
      return args.handler;
    }
    case "plugin:event|unlisten": {
      for (const s of Object.values(eventListeners)) s.delete(args.eventId);
      return null;
    }

    case "profiles_info": {
      const repo = String(args?.repo ?? "");
      const assigned = mockAssignments[repo] ?? null;
      const effective = assigned ?? mockDefaultProfile;
      return {
        profiles: Object.values(mockProfiles).map((p) => ({
          ...p,
          ever_launched: mockLaunched.has(p.id),
          dir: `/mock/data/worktrees/profiles/${p.id}`,
        })),
        default_id: mockDefaultProfile,
        assigned_id: assigned,
        effective_id: effective,
        // Flips the cold-conversation warning on for one fixture repo, so the
        // copy is reachable by clicking rather than only in theory.
        repo_has_unprofiled_history: repo === CDV_ROOT,
        env_override: null,
      };
    }
    case "new_profile_id": {
      const base = String(args?.name ?? "profile").toLowerCase().replace(/[^a-z0-9_-]/g, "-").replace(/^-+/, "") || "profile";
      let id = base;
      for (let n = 2; mockProfiles[id] || id === "none"; n++) id = `${base}-${n}`;
      return id;
    }
    case "save_profile": {
      const p = args?.profile as MockProfile;
      // Bump like the backend does — this is what the stale badge compares.
      mockProfiles[p.id] = { ...p, updated_epoch: (mockProfiles[p.id]?.updated_epoch ?? 0) + 1 };
      return p.id;
    }
    case "delete_profile": {
      const id = String(args?.id ?? "");
      const launched = mockLaunched.has(id);
      delete mockProfiles[id];
      if (mockDefaultProfile === id) mockDefaultProfile = null;
      for (const k of Object.keys(mockAssignments)) if (mockAssignments[k] === id) delete mockAssignments[k];
      return {
        dir: `/mock/data/worktrees/profiles/${id}`,
        // Mirrors the real handler: core returns the recorded service name so the
        // caller can name what it left behind.
        keychain_service: launched ? `Claude Code-credentials-${id.slice(0, 8)}` : null,
        keychain_hint: launched,
      };
    }
    case "set_project_profile": {
      const repo = String(args?.repo ?? "");
      const id = args?.id as string | null;
      if (id) mockAssignments[repo] = id;
      else delete mockAssignments[repo];
      return null;
    }
    case "set_default_profile":
      mockDefaultProfile = (args?.id as string | null) ?? null;
      return null;
    case "skills_list":
      return Object.values(mockSkills);
    case "skill_inspect":
      return { name: "demo-skill", description: "A demo skill.", capabilities: [], files: 1, bytes: 200, skill_md: "---\nname: demo-skill\n---\nbody\n" };
    case "skill_install_local": {
      const path = String(args?.path ?? "/tmp/added-skill");
      const name = path.split("/").filter(Boolean).pop() || "added-skill";
      mockSkills[name] = { name, description: "Installed from a folder.", capabilities: [], source: { kind: "local", path } };
      return mockSkills[name];
    }
    case "skill_preview_git": {
      // The review step: capabilities are shown BEFORE anything installs.
      return {
        url: String(args?.url ?? ""),
        rev: "",
        sha: "deadbeefcafe1234",
        skills: [{
          name: "fetched-skill",
          description: "Fetched from a repository.",
          capabilities: ["pre-authorises tools: Bash(rm:*)"],
          skill_md: "---\nname: fetched-skill\ndescription: Fetched from a repository.\nallowed-tools: Bash(rm:*)\n---\nbody\n",
        }],
      };
    }
    case "skill_install_git": {
      const name = String(args?.name ?? "fetched-skill");
      mockSkills[name] = {
        name, description: "Fetched from a repository.",
        capabilities: ["pre-authorises tools: Bash(rm:*)"],
        source: { kind: "git", url: String(args?.url ?? ""), rev: "", sha: String(args?.sha ?? "") },
      };
      return mockSkills[name];
    }
    case "skill_remove": {
      const name = String(args?.name ?? "");
      delete mockSkills[name];
      // Same contract as core: report the profiles that still list it.
      return Object.values(mockProfiles).filter((p) => (p.skills ?? []).includes(name)).map((p) => p.name);
    }

    default:
      console.warn("[mock] unhandled command:", cmd, args);
      return null;
  }
}

// Minimal internals surface the @tauri-apps/api v2 uses.
let cbId = 0;
const callbacks: Record<number, (v: unknown) => void> = {};
const eventListeners: Record<string, Set<number>> = {};
function emitEvent(event: string, payload: unknown) {
  for (const id of eventListeners[event] ?? []) callbacks[id]?.({ event, id, payload });
}
(window as any).__TAURI_INTERNALS__ = {
  invoke: (cmd: string, args?: Args) => mockInvoke(cmd, args),
  transformCallback: (cb: (v: unknown) => void) => {
    const id = ++cbId;
    callbacks[id] = cb;
    return id;
  },
  convertFileSrc: (p: string) => p,
  metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
};

// simulated Claude working state — cycles the sessions:busy push (lib.rs poll
// thread's event) so the busy (green blink) + waiting (amber static) dots and the
// project rollup badge are exercisable in the harness. Payload is now { busy,
// waiting } keyed by WORKTREE PATH (== place.path in fixtures.ts, which builds
// `${root}/.worktrees/${slug}`), matching lib.rs claude_activity's probe cwds.
const CDV = "/Users/demo/workspace/casadelvalle/casa-del-valle-monorepo/.worktrees";
const WT = "/Users/demo/workspace/worktrees/.worktrees";
const ACTIVITY_CYCLE: { busy: string[]; waiting: string[] }[] = [
  // both states visible: one place working, another needs input
  { busy: [`${CDV}/billing-refactor`], waiting: [`${WT}/feat-redesign`] },
  // working shifts, nothing waiting
  { busy: [`${CDV}/messaging`], waiting: [] },
  // only a waiting session (amber, no green)
  { busy: [], waiting: [`${CDV}/kitchen-sink`] },
  // idle — no dots at all
  { busy: [], waiting: [] },
];
let actIdx = 0;
setTimeout(() => emitEvent("sessions:busy", ACTIVITY_CYCLE[0]), 400);
setInterval(() => {
  actIdx = (actIdx + 1) % ACTIVITY_CYCLE.length;
  emitEvent("sessions:busy", ACTIVITY_CYCLE[actIdx]);
}, 5000);

// Harness-only controls for states a user cannot reach by clicking. An
// unreadable `.worktrees.toml` is the highest-consequence state in the Project
// sheet (doctor stops running, every op refuses) and there is no button that
// produces it — so drive it from the console / Playwright:
//
//   __mock.breakConfig(root?)   // config stops parsing; doctor exits on a guard
//   __mock.fixConfig(root?)     // back to parsing
//
// `root` defaults to the cdv fixture. Returns the root it touched. Both fire a
// places:changed so the nav re-pulls immediately instead of waiting on the sweep.
const healthyConfigs: Record<string, MockCfg> = {};
(window as any).__mock = {
  /** Fire the backend's shell:exit — the only way to reach the dock's
   * "process exited / Restart shell" state headlessly, since the mock has no
   * real PTY to die. */
  exitShell(repo: string, slug: string, index = 1) {
    const set = deadShells.get(sidecarKey(repo, slug)) ?? new Set<number>();
    set.add(index); deadShells.set(sidecarKey(repo, slug), set);
    emitEvent("shell:exit", { repo, slug, index });
    return { repo, slug, index };
  },
  breakConfig(root: string = CDV_ROOT, msg?: string) {
    const cfg = mockConfigs[root];
    if (!cfg) return null;
    healthyConfigs[root] ??= clone(cfg);
    // projcfg::load returning Err leaves ProjectConfigView with NOTHING but the
    // error (lib.rs) — a config that doesn't parse has no files, no ports and no
    // compose to report, only the reason. Modelling it as "structure + an error"
    // would let the sheet look half-fine.
    mockConfigs[root] = {
      path: cfg.path,
      exists: true,
      files: [], ports: null, compose: null, warnings: [],
      error: msg ?? ".worktrees.toml:7: expected `=` after key `path` — the file does not parse",
    };
    emitEvent("places:changed", {});
    return root;
  },
  fixConfig(root: string = CDV_ROOT) {
    if (!healthyConfigs[root]) return null;
    mockConfigs[root] = clone(healthyConfigs[root]);
    emitEvent("places:changed", {});
    return root;
  },
};

console.info("[mock] Tauri backend mocked — design harness active (window.__mock: breakConfig/fixConfig/exitShell)");
