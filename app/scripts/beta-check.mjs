// Holds the beta features (app/src/beta.ts) to their rules.
//
//  1. One feature, one flag, one checkbox (the owner, 2026-11-14): every
//     entry of BETA_FEATURES has its own `beta_<name>` key, default off and
//     normalised, its own helper, and its own checkbox, which Settings →
//     General draws FROM the table. The Settings map owns exactly the table's
//     keys. Nothing but the helper reads a key.
//  2. Ideas (#632) is the one beta today: with its flag OFF the main window
//     shows no lightbulb in the rail and takes no ⌘⇧I, turning it off closes
//     the Ideas window, and the shim's launch rule reads the same key.
//  3. Named workspaces LEFT the beta on 2026-11-14: no workspace surface sits
//     behind a beta helper any more (the switch, the menus, the list's fetch,
//     the view), and the words from before "All projects" existed are gone.
//
//   node app/scripts/beta-check.mjs     # exits non-zero on failure
//
// Like the other *-check.mjs it slices the REAL sources: App.tsx's gates
// (each found by the element it guards, then walked to the brace that
// encloses it), and the real beta.ts / settings.ts loaded as data: modules,
// so the defaults and the helpers' answers are evaluated, not grepped.
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

// ── 1. the table and the helpers, evaluated ──
const betaSrc = read("../src/beta.ts");
ok(!/^import (?!type )/m.test(betaSrc), "beta.ts has no runtime import (it loads alone)");
const B = await load(betaSrc, "beta.ts");
const F = B.BETA_FEATURES ?? [];
ok(Array.isArray(F) && F.length >= 1, `BETA_FEATURES lists the beta features (${F.length})`);
const keys = F.map((f) => f.key);
ok(new Set(keys).size === keys.length, "…each with its OWN key: no two features share a flag");
ok(new Set(F.map((f) => f.testId)).size === F.length, "…and its own checkbox test id");
for (const f of F) {
  ok(/^beta_[a-z][a-z_]*$/.test(f.key) && typeof f.label === "string" && f.label.length > 0 && typeof f.what === "string" && f.what.length > 20,
    `${f.key}: a beta_<name> key, a label and a line on what it adds`);
  // Its own helper: a function of exactly that key, answering only a real true.
  const helper = Object.entries(B).find(([, v]) => typeof v === "function" && new RegExp(`\\bs\\.${f.key} === true\\b`).test(String(v)));
  ok(!!helper, `${f.key}: has its own helper in beta.ts`);
  if (helper) {
    const [name, fn] = helper;
    ok(fn({ [f.key]: true }) === true, `${name}: on when ${f.key} is true`);
    for (const v of [false, undefined, "yes", 1, null]) ok(fn({ [f.key]: v }) === false, `${name}: off for ${JSON.stringify(v)}`);
  }
}
ok(keys.includes("beta_ideas") && F.find((f) => f.key === "beta_ideas")?.label === "Ideas",
  "Ideas is a beta feature of its own, labelled Ideas (no longer \"Ideas and workspaces\")");

// ── 2. every key: default off, normalised, owned by the Settings section ──
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
for (const k of keys) {
  ok(S.DEFAULTS[k] === false, `DEFAULTS.${k} is false`);
  for (const [blob, want, what] of [
    [null, false, "a fresh install"],
    [{ settings_rev: 5 }, false, "a blob from before the key"],
    [{ settings_rev: 5, [k]: "true" }, false, "a junk value"],
    [{ settings_rev: 5, [k]: true }, true, "a stored true"],
  ]) {
    stored = blob;
    const s = await S.loadSettings();
    ok(s[k] === want, `loadSettings ${k}: ${what} → ${want}`);
  }
}
const map = read("../src/settingsMap.ts");
const sec = map.match(/^\s*\{ id: "beta",[^\n]*\},$/m)?.[0] ?? "";
const mapKeys = (sec.match(/keys: \[([^\]]*)\]/)?.[1] ?? "").match(/"[a-z_]+"/g)?.map((x) => x.slice(1, -1)) ?? [];
ok(/page: "general"/.test(sec) && /tier: "advanced"/.test(sec), "settingsMap: the Beta features section is on General, Advanced (and Expert)");
ok(JSON.stringify([...mapKeys].sort()) === JSON.stringify([...keys].sort()),
  `settingsMap: the section owns exactly the table's keys (${mapKeys.join(", ")} vs ${keys.join(", ")})`);
ok(/label: "Beta features"/.test(sec) && /"beta"/.test(sec) && /"ideas"/.test(sec), "settingsMap: it is labelled Beta features and found by beta / ideas");
const general = read("../src/pages/general.tsx");
const secJsx = general.slice(general.indexOf('<Sec id="beta">'), general.indexOf("</Sec>", general.indexOf('<Sec id="beta">')));
ok(/import \{ BETA_FEATURES \} from "\.\.\/beta";/.test(general) && /BETA_FEATURES\.map\(\(f\) =>/.test(secJsx),
  "pages/general.tsx: one checkbox per feature, drawn from BETA_FEATURES");
ok(/checked=\{settings\[f\.key\] === true\}/.test(secJsx) && /onChange\(\{ \[f\.key\]: e\.currentTarget\.checked \}\)/.test(secJsx),
  "…each reads and writes its own feature's key");
ok(!/beta_[a-z]/.test(secJsx), "…and no checkbox names a key by hand");

// ── 3. one helper per key: nothing outside it reads a key ──
const srcFiles = (dir) => fs.readdirSync(here(dir), { withFileTypes: true }).flatMap((e) =>
  e.isDirectory() ? srcFiles(`${dir}/${e.name}`) : /\.(tsx?|mts)$/.test(e.name) ? [`${dir}/${e.name}`] : []);
const ALLOWED = new Set(["../src/beta.ts", "../src/settings.ts", "../src/settingsMap.ts", "../src/mock/install.ts"]);
for (const k of keys) {
  const readers = srcFiles("../src").filter((f) => !ALLOWED.has(f) && new RegExp(`\\b${k}\\b`).test(read(f)));
  ok(readers.length === 0, `only beta.ts answers "is ${k} on" (read nowhere else: ${readers.join(", ") || "none"})`);
}

// ── 4. App.tsx: Ideas behind `ideasOn`, workspaces behind nothing ──
const app = read("../src/App.tsx");
const code = app.replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, " ")).replace(/\/\/[^\n]*/g, (m) => " ".repeat(m.length));
ok(/import \{[^}]*\bideasBeta\b[^}]*\} from "\.\/beta";/.test(app) && /const ideasOn = ideasBeta\(settings\);/.test(code), "App.tsx: ideasOn = ideasBeta(settings)");

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
const at = (needle) => [...code.matchAll(new RegExp(needle.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "g"))].map((m) => m.index);
const gated = (needle) => { const a = at(needle); return a.length > 0 && a.every((p) => regions.some(([x, y]) => p > x && p < y)); };
const ungated = (needle) => { const a = at(needle); return a.length > 0 && a.every((p) => !regions.some(([x, y]) => p > x && p < y)); };
// Ideas: still the beta's.
ok(gated('data-testid="ideas-rail"'), "the rail's lightbulb renders only with Ideas on");
const chord = code.match(/if \([^\n]*e\.code === "KeyI"\)\) \{/)?.[0] ?? "";
ok(/^if \(keyRef\.current\.ideasOn && /.test(chord), "⌘⇧I is taken only with Ideas on");
ok(/keyRef\.current = \{[\s\S]*?\bideasOn\b[\s\S]*?\};/.test(code), "…and keyRef carries ideasOn");
ok((code.match(/openIdeas\(\)|onClick=\{openIdeas\}/g) ?? []).length === 2, "openIdeas has exactly its two callers, the chord and the rail");
ok(/if \(was === true && !ideasOn\) \{\s*invoke\("close_companion"\)/.test(code), "turning Ideas off closes the Ideas window (close_companion)");
// Workspaces: out of the beta (2026-11-14).
ok(ungated("<NavViewSwitch"), "the Workspaces | All projects switch is behind no beta");
ok(ungated('testId="ctx-ws-add"') && ungated('data-testid="ctx-ws-remove"'), "a project's Add to workspace and Remove from workspace are behind no beta");
ok(/const navView: NavView = wsList && settings\.nav_view === "workspaces"/.test(code), "the view follows nav_view once the list has loaded, whatever the beta says");
ok(/navView === "workspaces" \? workspacesNav\(\)/.test(code), "the Workspaces view is drawn through navView");
const eff = code.match(/useEffect\(\(\) => \{\s*let alive = true;\s*invoke<WsList>\("workspaces_list"\)[\s\S]*?\}, \[wsRegKey\]\);/)?.[0] ?? "";
ok(eff.length > 0, "workspaces_list is fetched with no beta in its way (its effect keys on the registry alone)");
ok(!/ideasOn[^\n;]*\bwsList\b|\bwsList\b[^\n;]*ideasOn/.test(code), "no line ties the workspace list to ideasOn");
// The words from before "All projects" existed are gone.
ok(/: "Remove from All projects"\}/.test(code) && !/ideasOn \? "Remove from All projects"/.test(code), "the project menu always says Remove from All projects");
ok(/"click again to remove from All projects" : "remove project"/.test(code) && !/"click again to remove from workspace"/.test(code), "the ✕'s armed tooltip always says remove from All projects");

// ── 5. the shim and the route tables (Ideas) ──
const lib = read("../src-tauri/src/lib.rs");
ok(/dev_open \|\| companion::reopens_at_launch\(&config_dir, /.test(lib), "lib.rs: the companion reopens at launch through companion::reopens_at_launch");
ok(/async fn close_companion\(/.test(lib) && /^\s+close_companion,$/m.test(lib), "lib.rs: close_companion is a shim command, registered");
const comp = read("../src-tauri/src/companion.rs");
ok(/pub const BETA: &str = "beta_ideas";/.test(comp), "companion.rs reads the same key");
ok(/"close_companion",/.test(read("../src/ipc.ts").match(/export const SHIM_COMMANDS[\s\S]*?\];/)?.[0] ?? ""), "ipc.ts: close_companion is in SHIM_COMMANDS");
const mock = read("../src/mock/install.ts");
ok(/case "close_companion":/.test(mock), "the mock answers close_companion");
ok(/get\("beta"\)\?\.split\(","\)\.includes\("ideas"\)/.test(mock) && /beta_ideas: true/.test(mock), "the mock: off by default, ?beta=ideas turns Ideas on");

if (bad) { console.log(`\n${bad} failed`); process.exit(1); }
console.log("\nbeta-check: all ok");
