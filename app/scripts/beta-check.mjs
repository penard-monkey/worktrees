// Holds the Ideas and workspaces beta (app/src/beta.ts) to its rule: with the
// flag OFF the main window shows nothing Ideas (#632) or workspaces (#633)
// added — no lightbulb in the rail, no ⌘⇧I, no All projects | Workspaces
// switch, no workspace menu item, no `workspaces_list` fetch, and the remove
// wording from before "All projects" existed — and with it ON each is there.
// Every gate asks ONE helper, and the shim's launch rule reads the same key.
//
//   node app/scripts/beta-check.mjs     # exits non-zero on failure
//
// Like the other *-check.mjs it slices the REAL sources: App.tsx's gates
// (each found by the element it guards, then walked to the brace that
// encloses it), and the real beta.ts / settings.ts loaded as data: modules,
// so the default and the helper's answer are evaluated, not grepped.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const here = (rel) => fileURLToPath(new URL(rel, import.meta.url));
const read = (rel) => fs.readFileSync(here(rel), "utf8");
let bad = 0;
const ok = (cond, m) => { if (cond) console.log(`ok   ${m}`); else { console.log(`not ok  ${m}`); bad++; } };
const load = async (src, name) => {
  const js = (await transformWithEsbuild(src, name, { loader: "ts", format: "esm" })).code;
  return import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));
};

// ── 1. the helper, evaluated ──
const betaSrc = read("../src/beta.ts");
ok(!/^import (?!type )/m.test(betaSrc), "beta.ts has no runtime import (it loads alone)");
const B = await load(betaSrc, "beta.ts");
ok(B.ideasBeta({ beta_ideas: true }) === true, "ideasBeta: on when the key is true");
for (const v of [false, undefined, "yes", 1, null]) ok(B.ideasBeta({ beta_ideas: v }) === false, `ideasBeta: off for ${JSON.stringify(v)}`);

// ── 2. the key: default off, normalised, owned by the Settings section ──
let stored = null;
globalThis.__betaInvoke = (cmd) => Promise.resolve(cmd === "get_settings" ? structuredClone(stored) : null);
const settingsTs = read("../src/settings.ts");
const inline = (rel) => read(rel).replace(/^export /gm, "") + "\n";
const S = await load(
  inline("../src/afterglow.ts") + inline("../src/harness.ts") +
  settingsTs
    .replace(/^import \{[^}]*\} from "\.\/afterglow";$/m, "")
    .replace(/^import \{[^}]*\} from "\.\/harness";$/m, "")
    .replace(/^import \{ invoke \}.*$/m, "const invoke = (cmd, a) => globalThis.__betaInvoke(cmd, a);"),
  "settings.ts");
ok(S.DEFAULTS.beta_ideas === false, "DEFAULTS.beta_ideas is false");
for (const [blob, want, what] of [
  [null, false, "a fresh install"],
  [{ settings_rev: 5 }, false, "a blob from before the key"],
  [{ settings_rev: 5, beta_ideas: "true" }, false, "a junk value"],
  [{ settings_rev: 5, beta_ideas: true }, true, "a stored true"],
]) {
  stored = blob;
  const s = await S.loadSettings();
  ok(s.beta_ideas === want, `loadSettings: ${what} → ${want}`);
}
const map = read("../src/settingsMap.ts");
const sec = map.match(/^\s*\{ id: "beta",[^\n]*\},$/m)?.[0] ?? "";
ok(/page: "general"/.test(sec) && /tier: "advanced"/.test(sec) && /keys: \["beta_ideas"\]/.test(sec),
  "settingsMap: the Beta features section is on General, Advanced (and Expert), owning beta_ideas");
ok(/label: "Beta features"/.test(sec) && /"beta"/.test(sec) && /"workspaces"/.test(sec) && /"ideas"/.test(sec),
  "settingsMap: it is labelled Beta features and found by beta / ideas / workspaces");
const general = read("../src/pages/general.tsx");
const secJsx = general.slice(general.indexOf('<Sec id="beta">'));
ok(secJsx.length > 0 && /checked=\{settings\.beta_ideas\}/.test(secJsx) && /onChange\(\{ beta_ideas: e\.currentTarget\.checked \}\)/.test(secJsx),
  "pages/general.tsx: the switch reads and writes beta_ideas");

// ── 3. one helper: nothing outside it reads the key ──
const srcFiles = (dir) => fs.readdirSync(here(dir), { withFileTypes: true }).flatMap((e) =>
  e.isDirectory() ? srcFiles(`${dir}/${e.name}`) : /\.(tsx?|mts)$/.test(e.name) ? [`${dir}/${e.name}`] : []);
const ALLOWED = new Set(["../src/beta.ts", "../src/settings.ts", "../src/settingsMap.ts", "../src/pages/general.tsx", "../src/mock/install.ts"]);
const readers = srcFiles("../src").filter((f) => !ALLOWED.has(f) && /beta_ideas/.test(read(f)));
ok(readers.length === 0, `only beta.ts answers "is the beta on" (the key is read nowhere else: ${readers.join(", ") || "none"})`);

// ── 4. App.tsx: every surface behind `ideasOn`, and `ideasOn` is the helper ──
const app = read("../src/App.tsx");
const code = app.replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, " ")).replace(/\/\/[^\n]*/g, (m) => " ".repeat(m.length));
ok(/import \{ ideasBeta \} from "\.\/beta";/.test(app) && /const ideasOn = ideasBeta\(settings\);/.test(code), "App.tsx: ideasOn = ideasBeta(settings)");

/** The `{ideasOn && … (` or `{ideasOn && <x … />}` regions, start to the
 *  matching close. Comments are blanked first, so a paren in one cannot
 *  unbalance the walk. */
const regions = [];
for (const m of code.matchAll(/\{ideasOn && /g)) {
  let depth = 0, i = m.index;
  for (; i < code.length; i++) {
    const c = code[i];
    if (c === "{") depth++;
    else if (c === "}" && --depth === 0) break;
  }
  regions.push([m.index, i]);
}
const gated = (needle) => {
  const at = [...code.matchAll(new RegExp(needle.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "g"))].map((m) => m.index);
  return at.length > 0 && at.every((p) => regions.some(([a, b]) => p > a && p < b));
};
ok(gated('data-testid="ideas-rail"'), "the rail's lightbulb renders only with the beta on");
ok(gated("<NavViewSwitch"), "the All projects | Workspaces switch renders only with the beta on");
ok(gated('data-testid="ctx-ws-add"'), "a project's Add to workspace renders only with the beta on");
ok(gated('data-testid="ctx-ws-remove"'), "a project's Remove from workspace renders only with the beta on");
// The Workspaces view itself, and the two menus only it opens.
ok(/const navView: NavView = ideasOn && wsList && settings\.nav_view === "workspaces"/.test(code),
  "the nav is All projects with the beta off, whatever nav_view holds (and nav_view is not rewritten)");
ok(!/updateSettings\(\{ nav_view: "projects"/.test(code), "nothing resets nav_view when the beta is off");
ok(/navView === "workspaces" \? workspacesNav\(\)/.test(code), "the Workspaces view is drawn only through navView");
const setsWsCtx = [...code.matchAll(/setCtx\(\{ kind: "(ws|ws-add)"/g)].map((m) => m.index);
ok(setsWsCtx.length >= 2, `the workspace menus are opened (${setsWsCtx.length} sites)`);
const wsNavAt = code.indexOf("const workspacesNav = ");
ok(setsWsCtx.every((p) => regions.some(([a, b]) => p > a && p < b) || (p > code.indexOf("const wsMenu = ") && p < code.indexOf("const startNewWorkspace"))),
  "the workspace menus open only from a gated item or from the Workspaces view's own headers");
ok(wsNavAt > 0 && /onMenu=\{\(e\) => wsMenu\(e, w\.id\)\}/.test(code.slice(wsNavAt)), "…wsMenu is called from the Workspaces view");
// The list: not fetched while off.
const eff = code.match(/useEffect\(\(\) => \{\s*if \(!ideasOn\) \{ setWsList\(null\); return; \}[\s\S]*?\}, \[wsRegKey, ideasOn\]\);/)?.[0] ?? "";
ok(/invoke<WsList>\("workspaces_list"\)/.test(eff), "workspaces_list is fetched only with the beta on, and dropped when it goes off");
const firstList = code.indexOf('"workspaces_list"');
ok(firstList > 0 && eff.length > 0 && firstList === code.indexOf(eff) + eff.indexOf('"workspaces_list"'),
  "…and that effect is the list's first fetch (the other is a reorder's retry, reachable only with it on)");
// ⌘⇧I.
const chord = code.match(/if \([^\n]*e\.code === "KeyI"\)\) \{/)?.[0] ?? "";
ok(/^if \(keyRef\.current\.ideasOn && /.test(chord), "⌘⇧I is taken only with the beta on");
ok(/keyRef\.current = \{[\s\S]*?\bideasOn\b[\s\S]*?\};/.test(code), "…and keyRef carries ideasOn");
ok((code.match(/openIdeas\(\)|onClick=\{openIdeas\}/g) ?? []).length === 2, "openIdeas has exactly its two callers, the chord and the rail");
// The words. "All projects" is on screen only with the beta on.
const visible = [...code.matchAll(/"[^"\n]*[Aa]ll projects[^"\n]*"/g)].map((m) => ({ s: m[0], at: m.index }));
const allowedAllProjects = (v) =>
  regions.some(([a, b]) => v.at > a && v.at < b) ||
  /^"(click again to remove|Remove) from All projects"$/.test(v.s) && /ideasOn \? \($|ideasOn \? $/.test(code.slice(v.at - 12, v.at)) ||
  // the ✕ under a workspace: only in the Workspaces view (ProjectNode's inWs)
  v.s === '"take it out of this workspace (it stays in All projects)"';
const loose = visible.filter((v) => !allowedAllProjects(v));
ok(visible.length >= 2 && loose.length === 0, `"All projects" is shown only with the beta on (loose: ${loose.map((v) => v.s).join(", ") || "none"})`);
ok(/ideasOn \? "Remove from All projects" : "Remove from workspace"/.test(code), "the project menu says Remove from workspace with the beta off");
ok(/ideasOn \? "click again to remove from All projects" : "click again to remove from workspace"/.test(code), "the ✕'s armed tooltip says remove from workspace with the beta off");
// Turning it off closes the Ideas window, on a change seen this session only.
ok(/if \(was === true && !ideasOn\) \{\s*invoke\("close_companion"\)/.test(code), "turning the beta off closes the Ideas window (close_companion)");

// ── 5. the shim and the route tables ──
const lib = read("../src-tauri/src/lib.rs");
ok(/dev_open \|\| companion::reopens_at_launch\(&config_dir, /.test(lib), "lib.rs: the companion reopens at launch through companion::reopens_at_launch");
ok(/async fn close_companion\(/.test(lib) && /^\s+close_companion,$/m.test(lib), "lib.rs: close_companion is a shim command, registered");
const comp = read("../src-tauri/src/companion.rs");
ok(/pub const BETA: &str = "beta_ideas";/.test(comp), "companion.rs reads the same key");
ok(/"close_companion",/.test(read("../src/ipc.ts").match(/export const SHIM_COMMANDS[\s\S]*?\];/)?.[0] ?? ""), "ipc.ts: close_companion is in SHIM_COMMANDS");
const mock = read("../src/mock/install.ts");
ok(/case "close_companion":/.test(mock), "the mock answers close_companion");
ok(/get\("beta"\)\?\.split\(","\)\.includes\("ideas"\)/.test(mock) && /beta_ideas: true/.test(mock), "the mock: off by default, ?beta=ideas turns it on");

if (bad) { console.log(`\n${bad} failed`); process.exit(1); }
console.log("\nbeta-check: all ok");
