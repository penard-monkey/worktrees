// Guards the boundary around the Ideas companion window's bundle.
//
//   node app/scripts/companion-boundary-check.mjs    # from the repo root; exits non-zero on failure
//
// The companion (app/companion/, vite.companion.config.ts → app/dist/companion/)
// is where the libraries ideas need will live — CodeMirror 6 in phase 1
// (docs/proposals/idea-board-phases.md §4.3) — and the app's own bundle must
// never gain a byte of them. Same shape and same reason as
// viewer-boundary-check.mjs: "it is isolated" is otherwise a claim about a vite
// config, and a single import in a shared file falsifies it silently.
//
// Assertions:
//   1. Nothing reachable from the APP's entry imports a companion file, or a
//      library only the companion may use (COMPANION_ONLY).
//   2. The companion reaches the backend through the one door (app/src/ipc.ts)
//      and imports no @tauri-apps/api/core or /event of its own; it opens its
//      engine door reconnect-only (`neverStart`), and takes the port and token
//      from engine_connect alone.
//   3. Two builds, two outDirs: the app's config names no companion input,
//      the companion's builds into dist/companion with its own root and base,
//      and `pnpm build` runs the app's (which empties dist/) first.
//   4. The window the shim opens is that page, and its capability is its own.
//   5. If a build exists: the app's assets hold no companion-only library, the
//      companion's page is there, and no built file carries the dev-only drag
//      probe or the dev token client.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const APP = fileURLToPath(new URL("..", import.meta.url));
let failed = 0;
const fail = (m) => { failed++; console.log(`not ok — ${m}`); };
const ok = (m) => console.log(`ok — ${m}`);
const check = (c, m) => (c ? ok(m) : fail(m));
const read = (rel) => fs.readFileSync(path.join(APP, rel), "utf8");
const exists = (rel) => fs.existsSync(path.join(APP, rel));
const code = (s) => s.replace(/\/\*[\s\S]*?\*\/|\/\/.*/g, "");

/** Packages only the companion's bundle may reach. CodeMirror is phase 1's;
 *  it is listed now so the line exists before the dependency does. */
const COMPANION_ONLY = ["codemirror", "@codemirror", "@lezer", "@uiw"];

function resolveRel(fromFile, spec) {
  const base = path.resolve(path.dirname(fromFile), spec);
  const tries = [base, `${base}.ts`, `${base}.tsx`, `${base}.js`, `${base}.jsx`, path.join(base, "index.ts"), path.join(base, "index.tsx")];
  return tries.find((p) => fs.existsSync(p) && fs.statSync(p).isFile()) ?? null;
}

/** The import graph from `entry`, following relative imports (static, export
 *  from, dynamic): the files reached and the bare packages they name. */
function graph(entry) {
  const files = new Set();
  const pkgs = new Set();
  const queue = [entry];
  const RE = /(?:^|\n)\s*(?:import|export)[\s\S]{0,400}?from\s*["']([^"']+)["']|(?:^|[^.\w])import\s*\(\s*["']([^"']+)["']\s*\)|(?:^|\n)\s*import\s+["']([^"']+)["']/g;
  while (queue.length) {
    const file = queue.pop();
    if (!file || files.has(file)) continue;
    files.add(file);
    if (!/\.(t|j)sx?$/.test(file)) continue;
    for (const m of fs.readFileSync(file, "utf8").matchAll(RE)) {
      const spec = (m[1] ?? m[2] ?? m[3] ?? "").split("?")[0];
      if (!spec) continue;
      if (spec.startsWith(".") || spec.startsWith("/")) {
        const next = resolveRel(file, spec);
        if (next) queue.push(next);
      } else pkgs.add(spec.startsWith("@") ? spec.split("/").slice(0, 2).join("/") : spec.split("/")[0]);
    }
  }
  return { files, pkgs };
}
const only = (pkg) => COMPANION_ONLY.some((c) => pkg === c || pkg.startsWith(`${c}/`));
const inCompanion = (f) => f.startsWith(path.join(APP, "companion") + path.sep);

// ── 1. the app's graph reaches nothing of the companion's ───────────────────
{
  const g = graph(path.join(APP, "src/main.tsx"));
  check(g.files.size > 50, `the app's graph was walked (${g.files.size} files) — a walk of nothing would pass everything`);
  const pkgs = [...g.pkgs].filter(only);
  check(pkgs.length === 0, `no companion-only library (${COMPANION_ONLY.join(", ")}) is reachable from app/src/main.tsx${pkgs.length ? `: ${pkgs.join(", ")}` : ""}`);
  const files = [...g.files].filter(inCompanion).map((f) => path.relative(APP, f));
  check(files.length === 0, `the app imports nothing from app/companion/ — the companion may import app/src, never the other way round${files.length ? `: ${files.join(", ")}` : ""}`);
}

// ── 2. the companion's graph: the one door, reconnect-only ──────────────────
{
  const entry = path.join(APP, "companion/main.tsx");
  if (!fs.existsSync(entry)) fail("app/companion/main.tsx not found — the check has nothing to guard");
  else {
    const g = graph(entry);
    check([...g.files].some((f) => f === path.join(APP, "src/ipc.ts")), "the companion reaches the backend through app/src/ipc.ts, the one door");
    const own = [...g.files].filter(inCompanion).filter((f) => /\.(t|j)sx?$/.test(f));
    const doors = own.filter((f) => /["']@tauri-apps\/api\/(core|event)["']/.test(fs.readFileSync(f, "utf8")));
    check(own.length >= 2 && doors.length === 0, `no companion file imports @tauri-apps/api/core or /event itself (${own.length} files)${doors.length ? `: ${doors.map((f) => path.relative(APP, f)).join(", ")}` : ""}`);
    const main = code(read("companion/main.tsx"));
    check(/engineDoor\(\s*tauriDoor,[\s\S]*?retryEnv\(\),\s*\{ neverStart: true \},?\s*\)/.test(main), "the companion opens its engine door with `{ neverStart: true }`: the main window alone starts serve");
    check((main.match(/engine_connect/g) ?? []).length === 1 && /\{ reason, start, manual, onPhase: onPhaseCh \}/.test(main),
      "engine_connect is called once, from the one dial, with the try the policy made (start included)");
    check(/connectSocket\(\{ \.\.\.socketDeps\(onLost\), port: ep\.port, token: ep\.token \}\)/.test(main) && !/localStorage|sessionStorage|console\.(log|info)\(.*token/.test(main),
      "the port and token go from engine_connect's answer straight to connectSocket, stored and logged nowhere");
    check(!/bootSocket/.test(main) && /version: __WT_VERSION__/.test(main), "the companion never names the dev token client, and hands its socket __WT_VERSION__");
    check(/document\.addEventListener\("visibilitychange", \(\) => door\.visibility\(\)\);/.test(main), "a hidden companion defers its retries (it hears visibilitychange)");
  }
}

// ── 3. two builds, two outputs ──────────────────────────────────────────────
{
  const appCfg = code(read("vite.config.ts"));
  check(!/companion/.test(appCfg), "the app's vite.config.ts names no companion input — a second input is a shared chunk waiting to happen");
  if (!exists("vite.companion.config.ts")) fail("vite.companion.config.ts not found — the companion has no build of its own");
  else {
    const c = code(read("vite.companion.config.ts"));
    check(/root:\s*here\("companion"\)/.test(c) && /outDir:\s*here\("dist\/companion"\)/.test(c) && /base:\s*"\/companion\/"/.test(c),
      "the companion builds from app/companion into app/dist/companion, with its assets under /companion/");
    check(!/rollupOptions[\s\S]*input/.test(c) && !/\.\.\/src\/main|src\/main\.tsx/.test(c), "the companion's build has no second input, and never the app's entry");
    check(/__WT_VERSION__: JSON\.stringify\(workspaceVersion\(\)\)/.test(c), "the companion's version comes from the same workspaceVersion() as the app's");
  }
  const build = JSON.parse(read("package.json")).scripts?.build ?? "";
  const a = build.indexOf("vite build"), b = build.indexOf("vite build --config vite.companion.config.ts");
  check(a >= 0 && b > a && build.indexOf("vite build", a + 1) === b, `\`pnpm build\` builds the app (which empties dist/) and THEN the companion (${JSON.stringify(build)})`);
}

// ── 4. the window and its rights ────────────────────────────────────────────
{
  const rs = read("src-tauri/src/companion.rs");
  check(/const PAGE: &str = "companion\/index\.html";/.test(rs) && exists("companion/index.html"), "the shim opens companion/index.html, the page this build makes");
  check(/pub const LABEL: &str = "companion";/.test(rs) && /\.disable_drag_drop_handler\(\)/.test(rs), "the window is labelled `companion` and, like main, leaves drag and drop to the page");
  const cap = JSON.parse(read("src-tauri/capabilities/companion.json"));
  const main = JSON.parse(read("src-tauri/capabilities/default.json"));
  check(JSON.stringify(cap.windows) === JSON.stringify(["companion"]), `capabilities/companion.json names the companion alone (${JSON.stringify(cap.windows)})`);
  check(JSON.stringify(main.windows) === JSON.stringify(["main"]), `capabilities/default.json still names main alone (${JSON.stringify(main.windows)})`);
  const ids = (c) => c.permissions.map((p) => (typeof p === "string" ? p : p.identifier));
  const borrowed = ids(cap).filter((p) => p !== "core:default" && ids(main).includes(p));
  check(borrowed.length === 0, `the companion holds none of main's plugin permissions${borrowed.length ? `: ${borrowed.join(", ")}` : ""} (${ids(cap).join(", ")})`);
}

// ── 5. the build, when there is one ─────────────────────────────────────────
{
  if (!exists("dist")) ok("app/dist absent — the build assertions are skipped on this run (build with `pnpm build` to exercise them)");
  else {
    const all = [];
    const walk = (d) => { for (const e of fs.readdirSync(d, { withFileTypes: true })) { const p = path.join(d, e.name); if (e.isDirectory()) walk(p); else if (/\.(js|mjs|css|html)$/.test(e.name)) all.push(p); } };
    walk(path.join(APP, "dist"));
    const appFiles = all.filter((f) => !f.startsWith(path.join(APP, "dist/companion") + path.sep));
    const leak = appFiles.filter((f) => /@codemirror|codemirror|@lezer/i.test(fs.readFileSync(f, "utf8")));
    check(leak.length === 0, `the app's built files carry no companion-only library${leak.length ? `: ${leak.map((f) => path.relative(APP, f)).join(", ")}` : ""}`);
    check(exists("dist/companion/index.html"), "the build has the companion's page at dist/companion/index.html");
    const probe = all.filter((f) => /x-worktrees-dragprobe|drag me \(/.test(fs.readFileSync(f, "utf8")));
    check(probe.length === 0, `no built file carries the dev-only drag probe${probe.length ? `: ${probe.map((f) => path.relative(APP, f)).join(", ")}` : ""}`);
    const dev = all.filter((f) => f.startsWith(path.join(APP, "dist/companion")) && /\/__worktrees\/serve|the dev server could not say/.test(fs.readFileSync(f, "utf8")));
    check(dev.length === 0, "the companion's build carries no dev token client");
  }
}

console.log(failed ? `\ncompanion-boundary-check: ${failed} failure(s)` : "\ncompanion-boundary-check: all good");
process.exit(failed ? 1 : 0);
