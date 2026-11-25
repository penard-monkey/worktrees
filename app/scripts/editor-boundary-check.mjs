// Guards two boundaries: CodeMirror's (one module, loaded on first use) and the
// Ideas companion window's (its own build, its own rights).
//
//   node app/scripts/editor-boundary-check.mjs    # from the repo root; exits non-zero on failure
//
// CodeMirror 6 is the document editor in BOTH windows (AGENTS.md, the third
// admitted library; docs/proposals/workspace-chats.md §4.5). The rule is not
// "never in app/src" any more: it is ONE module, app/src/doc/editor.ts, reached
// only through loadeditor.ts's dynamic import with destructured names, so it
// is a chunk fetched when a document is first opened and the main window's
// start does not carry it. "It is lazy" is otherwise a claim about a bundler,
// and one static import in a shared file falsifies it silently.
//
// Assertions:
//   1. The library: only editor.ts names an editor package, in any form, in
//      app/src, app/companion or app/viewer. editor.ts itself is imported once,
//      by `const { mountEditor } = await import("./editor")` in loadeditor.ts:
//      no static import, no namespace, no second door. Both windows reach it
//      (the positive control), and app/src imports nothing of app/companion's.
//   2. The companion reaches the backend through the one door (app/src/ipc.ts)
//      and imports no @tauri-apps/api/core or /event of its own; it opens its
//      engine door reconnect-only (`neverStart`), and takes the port and token
//      from engine_connect alone.
//   3. Two builds, two outDirs: the app's config names no companion input,
//      the companion's builds into dist/companion with its own root and base,
//      and `pnpm build` runs the app's (which empties dist/) first.
//   4. The window the shim opens is that page, and its capability is its own.
//   5. If a build exists: exactly one of the app's files carries CodeMirror, it
//      is in no page and no chunk statically imports it (so neither the entry
//      chunk nor App's does), and something imports it dynamically. The
//      companion's page is there with its editor, and no built file carries
//      the dev-only drag probe or the dev token client.
//
// APP_ROOT=<copy of app/> checks another tree, so a mutation can be shown red
// without touching this one.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const APP = process.env.APP_ROOT ? path.resolve(process.env.APP_ROOT) + path.sep : fileURLToPath(new URL("..", import.meta.url));
let failed = 0;
const fail = (m) => { failed++; console.log(`not ok — ${m}`); };
const ok = (m) => console.log(`ok — ${m}`);
const check = (c, m) => (c ? ok(m) : fail(m));
const read = (rel) => fs.readFileSync(path.join(APP, rel), "utf8");
const exists = (rel) => fs.existsSync(path.join(APP, rel));
const code = (s) => s.replace(/\/\*[\s\S]*?\*\/|\/\/.*/g, "");

/** CodeMirror, its parser, and what they alone pull in. */
const EDITOR_LIBS = ["codemirror", "@codemirror", "@lezer", "@uiw", "@marijn", "style-mod", "crelt", "w3c-keyname"];
const EDITOR = path.join(APP, "src/doc/editor.ts");
const DOOR = path.join(APP, "src/doc/loadeditor.ts");

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
const only = (pkg) => EDITOR_LIBS.some((c) => pkg === c || pkg.startsWith(`${c}/`));
const inCompanion = (f) => f.startsWith(path.join(APP, "companion") + path.sep);

// ── 1. the library: one module, one lazy door ───────────────────────────────
/** Every module specifier in a file, however it is named: `from "x"`,
 *  `import "x"`, `import("x")`, `require("x")`. Comments are blanked, so prose
 *  that names a package is not an import. */
function specs(file) {
  const text = fs.readFileSync(file, "utf8").replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:"'`])\/\/.*$/gm, "$1");
  return [...text.matchAll(/\bfrom\s*["'`]([^"'`]+)["'`]|\bimport\s*["'`]([^"'`]+)["'`]|\b(?:import|require|glob)\s*\(\s*["'`]([^"'`]+)["'`]/g)]
    .map((m) => ({ spec: (m[1] ?? m[2] ?? m[3]).split("?")[0], at: m.index, text }));
}
const sources = (dir) => (fs.existsSync(dir) ? fs.readdirSync(dir, { withFileTypes: true }) : []).flatMap((e) => {
  const p = path.join(dir, e.name);
  return e.isDirectory() ? sources(p) : /\.(ts|tsx|js|jsx|mjs|mts)$/.test(e.name) ? [p] : [];
});
{
  const all = ["src", "companion", "viewer"].flatMap((d) => sources(path.join(APP, d)));
  const rel = (f) => path.relative(APP, f);
  check(all.length > 100, `every source file was read (${all.length}) — a read of nothing would pass everything`);
  const users = all.filter((f) => specs(f).some(({ spec }) => only(spec)));
  check(users.length === 1 && users[0] === EDITOR, `one module imports the editor's libraries (${EDITOR_LIBS.join(", ")}): ${users.map(rel).join(", ") || "none"}`);
  const doors = all.flatMap((f) => specs(f).filter(({ spec }) => spec.startsWith(".") && resolveRel(f, spec) === EDITOR).map((x) => ({ f, ...x })));
  check(doors.length === 1 && doors[0].f === DOOR, `editor.ts is imported once, by loadeditor.ts: ${doors.map((d) => rel(d.f)).join(", ") || "nowhere"}`);
  // Destructured: a namespace (`const ed = await import(…)`, or `.then((m) =>`)
  // keeps every export alive and defeats tree-shaking.
  check(/\bconst \{ mountEditor \} = await import\("\.\/editor"\);/.test(code(fs.existsSync(DOOR) ? fs.readFileSync(DOOR, "utf8") : "")),
    "…and by a dynamic import with destructured names");
  const g = graph(path.join(APP, "src/main.tsx"));
  check(g.files.size > 50, `the app's graph was walked (${g.files.size} files)`);
  check(g.files.has(EDITOR) && g.pkgs.has("@codemirror/view"), "the main window reaches the editor (through that door): the rule guards something");
  const files = [...g.files].filter(inCompanion).map(rel);
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
    check(g.files.has(EDITOR), "the companion's editor is the same module as the main window's");
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
  // open-url is in both, each with its own scope: main's adds System Settings,
  // the companion's is web pages alone, so it is held whole below.
  const OPEN_URL = "opener:allow-open-url";
  const borrowed = ids(cap).filter((p) => p !== "core:default" && p !== OPEN_URL && ids(main).includes(p));
  check(borrowed.length === 0, `the companion holds none of main's plugin permissions${borrowed.length ? `: ${borrowed.join(", ")}` : ""} (${ids(cap).join(", ")})`);
  // Exactly what the window uses: Show in Finder's reveal, and a link in a
  // conversation (http and https), with no opener set (opener:default would
  // also open mailto, tel and paths).
  const WANT = ["core:default", "opener:allow-reveal-item-in-dir", { identifier: OPEN_URL, allow: [{ url: "http://*" }, { url: "https://*" }] }];
  check(JSON.stringify(cap.permissions) === JSON.stringify(WANT), `the companion's rights are exactly ${ids({ permissions: WANT }).join(" + ")} (http and https only): ${JSON.stringify(cap.permissions)}`);
  // A link never takes the window: the page cancels every anchor's click
  // before it renders anything, and the window goes nowhere but its own page
  // (WebKit's menu has an Open Link that raises no DOM event).
  const entry = code(read("companion/main.tsx"));
  check(/guardLinks\(document\);/.test(entry) && entry.indexOf("guardLinks(document);") < entry.indexOf("createRoot("), "the page guards its links before it renders");
  check(/\.on_navigation\(move \|url\| \{\s*let here = stays\(url, dev\.as_ref\(\)\);/.test(rs) && /\.on_new_window\(move \|url, _\| \{[\s\S]*?NewWindowResponse::Deny/.test(rs),
    "the window navigates only where `stays` says, and opens no second one");
}

// ── 5. the build, when there is one ─────────────────────────────────────────
{
  if (!exists("dist")) ok("app/dist absent — the build assertions are skipped on this run (build with `pnpm build` to exercise them)");
  else {
    const all = [];
    const walk = (d) => { for (const e of fs.readdirSync(d, { withFileTypes: true })) { const p = path.join(d, e.name); if (e.isDirectory()) walk(p); else if (/\.(js|mjs|css|html)$/.test(e.name)) all.push(p); } };
    walk(path.join(APP, "dist"));
    const appFiles = all.filter((f) => !f.startsWith(path.join(APP, "dist/companion") + path.sep));
    // Package names may not survive minification; CodeMirror's own class
    // names do (the companion's, below, is the same grep finding them).
    const has = (f) => /@codemirror|@lezer|cm-content|cm-scroller/i.test(fs.readFileSync(f, "utf8"));
    const carriers = appFiles.filter(has);
    const name = (f) => path.relative(APP, f);
    check(carriers.length === 1 && carriers[0].endsWith(".js"), `exactly one of the app's built files carries CodeMirror: ${carriers.map(name).join(", ") || "none"}`);
    // A static import would fold it INTO App's chunk, which is itself one file,
    // imported dynamically by the entry: every other line here passes on that.
    const pane = appFiles.filter((f) => f.endsWith(".js") && /main-doc-float/.test(fs.readFileSync(f, "utf8")));
    check(pane.length === 1 && pane[0] !== carriers[0], `and that file is not the one with the pane's own code (${pane.map(name).join(", ") || "found nowhere"})`);
    const chunk = carriers[0] && path.basename(carriers[0]);
    const lit = chunk?.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") ?? "(?!)";
    const pages = appFiles.filter((f) => f.endsWith(".html") && new RegExp(lit).test(fs.readFileSync(f, "utf8")));
    check(pages.length === 0, `no page names it: it is neither the entry chunk nor preloaded${pages.length ? `: ${pages.map(name).join(", ")}` : ""}`);
    const js = appFiles.filter((f) => f.endsWith(".js") && f !== carriers[0]);
    const hard = js.filter((f) => new RegExp(`(?:\\bfrom|\\bimport)\\s*["'][^"']*${lit}["']`).test(fs.readFileSync(f, "utf8")));
    check(hard.length === 0, `no chunk statically imports it, the entry's and App's included${hard.length ? `: ${hard.map(name).join(", ")}` : ""}`);
    const soft = js.filter((f) => new RegExp(`\\bimport\\(\\s*["'][^"']*${lit}["']`).test(fs.readFileSync(f, "utf8")));
    check(soft.length >= 1, `and it is imported dynamically (${soft.map(name).join(", ") || "nowhere"})`);
    const kb = (f) => `${(fs.statSync(f).size / 1000).toFixed(1)} kB`;
    if (carriers.length === 1) ok(`sizes: ${chunk} ${kb(carriers[0])}; ${js.filter((f) => /\/(index|App)-/.test(f)).map((f) => `${path.basename(f)} ${kb(f)}`).join(", ")}`);
    check(exists("dist/companion/index.html"), "the build has the companion's page at dist/companion/index.html");
    const cmp = all.filter((f) => f.startsWith(path.join(APP, "dist/companion") + path.sep));
    check(cmp.some((f) => /cm-content|cm-scroller/.test(fs.readFileSync(f, "utf8"))), "the companion's build carries the editor (the leak test above greps for something that exists)");
    const probe = all.filter((f) => /x-worktrees-dragprobe|drag me \(/.test(fs.readFileSync(f, "utf8")));
    check(probe.length === 0, `no built file carries the dev-only drag probe${probe.length ? `: ${probe.map((f) => path.relative(APP, f)).join(", ")}` : ""}`);
    const dev = all.filter((f) => f.startsWith(path.join(APP, "dist/companion")) && /\/__worktrees\/serve|the dev server could not say/.test(fs.readFileSync(f, "utf8")));
    check(dev.length === 0, "the companion's build carries no dev token client");
  }
}

console.log(failed ? `\neditor-boundary-check: ${failed} failure(s)` : "\neditor-boundary-check: all good");
process.exit(failed ? 1 : 0);
