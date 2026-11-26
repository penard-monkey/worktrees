// "Show the Everything Else header" (Settings → Layout), the owner's decision
// of 2026-11-25: a per-user setting, ON by default, and OFF hides the HEADER
// only. The projects under it stay in the Workspaces view, below the named
// workspaces, with no title, no card and no count.
//
//   node app/scripts/ee-header-check.mjs     # exits non-zero on failure
//
// It loads the REAL settings.ts as a data: module (restore-default-check's
// way), so the default and loadSettings's answers are evaluated, not grepped.
// The nav's wiring is read from App.tsx; `ee-header-browser.mjs` drives what
// it draws.
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

let stored = null;
const saved = [];
globalThis.__eeInvoke = (cmd, a) => {
  if (cmd === "set_settings") saved.push(structuredClone(a.settings));
  return Promise.resolve(cmd === "get_settings" ? structuredClone(stored) : null);
};
const inline = (rel) => read(rel).replace(/^export /gm, "") + "\n";
const S = await load(
  inline("../src/afterglow.ts") + inline("../src/harness.ts") +
  read("../src/settings.ts")
    .replace(/^import \{[^}]*\} from "\.\/afterglow";$/m, "")
    .replace(/^import \{[^}]*\} from "\.\/harness";$/m, "")
    .replace(/^import \{ invoke \}.*$/m, "const invoke = (cmd, a) => globalThis.__eeInvoke(cmd, a);"),
  "settings.ts");

// ── 1. the key: default on, a stored value kept, junk is the default ──
ok(S.DEFAULTS.nav_ee_header === true, "DEFAULTS.nav_ee_header is true");
for (const [blob, want, what] of [
  [null, true, "a fresh install"],
  [{}, true, "an empty blob"],
  [{ settings_rev: 5 }, true, "a blob from before the key"],
  [{ settings_rev: 5, nav_ee_header: false }, false, "a stored false"],
  [{ settings_rev: 5, nav_ee_header: true }, true, "a stored true"],
  [{ settings_rev: 5, nav_ee_header: "no" }, true, "a junk string: the default"],
  [{ settings_rev: 5, nav_ee_header: 0 }, true, "a junk number: the default"],
  [{ settings_rev: 5, nav_ee_header: null }, true, "a null: the default"],
]) {
  stored = blob;
  const s = await S.loadSettings();
  ok(s.nav_ee_header === want, `loadSettings: ${what} → ${want}`);
}
stored = { settings_rev: 5, nav_ee_header: false };
S.saveSettings(await S.loadSettings());
await new Promise((r) => setTimeout(r, 300));
ok(saved.length > 0 && saved.at(-1).nav_ee_header === false, "a stored false is written back false by the next whole-blob save");

// ── 2. the Settings row ──
const layout = read("../src/pages/layout.tsx");
const sec = layout.slice(layout.indexOf('<Sec id="ee-header">'), layout.indexOf("</Sec>", layout.indexOf('<Sec id="ee-header">')));
ok(sec.length > 0, 'pages/layout.tsx renders <Sec id="ee-header">');
ok(/type="checkbox" checked=\{settings\.nav_ee_header\}/.test(sec), "…a checkbox that reads nav_ee_header");
ok(/onChange\(\{ nav_ee_header: e\.currentTarget\.checked \}\)/.test(sec), "…and writes it");
ok(/Show the Everything Else header/.test(sec), "…labelled \"Show the Everything Else header\"");
const map = read("../src/settingsMap.ts");
ok(/\{ id: "ee-header", page: "layout",[^}]*keys: \["nav_ee_header"\] \}/.test(map), "settingsMap.ts: the ee-header section on Layout owns nav_ee_header");

// ── 3. the nav: App.tsx ──
const app = read("../src/App.tsx");
const at = app.indexOf("everythingElse(wsList");
const block = at < 0 ? "" : app.slice(at, app.indexOf("})()}", at));
ok(/const eeHeader = settings\.nav_ee_header \|\| list\.length === 0;/.test(block),
  "the header shows when the setting is on, or when there is no named workspace (that look never changes)");
ok(/const open = !eeHeader \|\| !settings\.ws_collapsed\?\.\[ee\.id\];/.test(block),
  "with the header hidden the group is open whatever its stored fold (no header, no way to unfold)");
ok(/eeHeader && <EverythingElseHeader /.test(block), "the header is drawn only under eeHeader");
ok(/className=\{"ws-group ee" \+ \(eeHeader \? "" : " bare"\)\}/.test(block), "…and the card's chrome drops with it (.bare)");
ok(/data-ee /.test(block) && !/data-ws-id/.test(block), "the group stays no drop target in both positions (no data-ws-id)");
ok(/ProjectNode\(\{ pv, inWs: ee\.id \}\)/.test(block), "its projects keep their Everything Else fold keys and the All projects menu");
const css = read("../src/App.css");
ok(/\.ws-group\.ee\.bare \{[^}]*border: 0;[^}]*background: transparent;/.test(css), "App.css: .ws-group.ee.bare has no border or card background");
ok(/\.ws-group\.ee\.bare > \.ws-kids \{[^}]*border-top: 0;/.test(css), "…no rule under a title that is not there");
ok(/\.ws-group\.ee\.bare \.project-h \{ top: 0; \}/.test(css), "…and its projects' headers stick at the top, with no title to sit under");

console.log(bad ? `\n${bad} failed` : "\nall passed");
process.exit(bad ? 1 : 0);
