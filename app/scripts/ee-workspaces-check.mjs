// "Show Everything Else in Workspaces" (Settings → Layout), the owner's
// decision of 2026-11-25: "I want All projects to hold all projects. The
// workspaces should not hold 'everything else' at all. I want to only show
// defined workspaces." A per-user setting, ON by default; OFF draws no
// Everything Else group in the Workspaces view, projects included, in every
// case: with no named workspace the view is EMPTY (the owner: an empty view
// over the group), with one line, New workspace, and a way to All projects.
// A hidden project must not hide what needs the person: its waiting or
// unread places mark the switch's "All projects" side.
//
//   node app/scripts/ee-workspaces-check.mjs     # exits non-zero on failure
//
// It loads the REAL settings.ts as a data: module (restore-default-check's
// way), so the default and loadSettings's answers are evaluated, not grepped.
// The nav's wiring is read from App.tsx; `ee-workspaces-browser.mjs` drives
// what it draws.
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
const K = "nav_ee_in_workspaces";
ok(S.DEFAULTS[K] === true, `DEFAULTS.${K} is true`);
ok(!("nav_ee_header" in S.DEFAULTS), "the first cut's nav_ee_header is gone (it never left this branch)");
for (const [blob, want, what] of [
  [null, true, "a fresh install"],
  [{}, true, "an empty blob"],
  [{ settings_rev: 5 }, true, "a blob from before the key"],
  [{ settings_rev: 5, [K]: false }, false, "a stored false"],
  [{ settings_rev: 5, [K]: true }, true, "a stored true"],
  [{ settings_rev: 5, [K]: "no" }, true, "a junk string: the default"],
  [{ settings_rev: 5, [K]: 0 }, true, "a junk number: the default"],
  [{ settings_rev: 5, [K]: null }, true, "a null: the default"],
]) {
  stored = blob;
  const s = await S.loadSettings();
  ok(s[K] === want, `loadSettings: ${what} → ${want}`);
}
stored = { settings_rev: 5, [K]: false };
S.saveSettings(await S.loadSettings());
await new Promise((r) => setTimeout(r, 300));
ok(saved.length > 0 && saved.at(-1)[K] === false, "a stored false is written back false by the next whole-blob save");

// ── 2. the Settings row ──
const layout = read("../src/pages/layout.tsx");
const at0 = layout.indexOf('<Sec id="ee-workspaces">');
const sec = at0 < 0 ? "" : layout.slice(at0, layout.indexOf("</Sec>", at0));
ok(sec.length > 0, 'pages/layout.tsx renders <Sec id="ee-workspaces">');
ok(/type="checkbox" checked=\{settings\.nav_ee_in_workspaces\}/.test(sec), `…a checkbox that reads ${K}`);
ok(/onChange\(\{ nav_ee_in_workspaces: e\.currentTarget\.checked \}\)/.test(sec), "…and writes it");
ok(/Show Everything Else in Workspaces/.test(sec), "…labelled \"Show Everything Else in Workspaces\"");
ok(/All projects/.test(sec), "…and its hint says where those projects are then");
const map = read("../src/settingsMap.ts");
ok(/\{ id: "ee-workspaces", page: "layout",[^}]*keys: \["nav_ee_in_workspaces"\] \}/.test(map), `settingsMap.ts: the ee-workspaces section on Layout owns ${K}`);

// ── 3. the nav: App.tsx ──
const app = read("../src/App.tsx");
ok(!/nav_ee_header/.test(app) && !/" bare"/.test(app), "App.tsx: nothing of the header-only cut is left");
ok(/const eeHidden = !settings\.nav_ee_in_workspaces;/.test(app),
  "eeHidden: the setting alone decides, with or without a named workspace");
const nav = app.slice(app.indexOf("const workspacesNav = () =>"), app.indexOf("const ee = everythingElse(wsList"));
ok(/data-testid="ws-empty"/.test(nav) && /data-testid="ws-new-first"/.test(nav), "the empty view keeps its line and the New workspace button");
ok(/\{eeHidden && \(\s*<button className="ws-new-btn" data-testid="ws-empty-all"[^>]*onClick=\{\(\) => setNavView\("projects"\)\}/.test(nav),
  "…and, with the group hidden, a button to All projects (the switch's own setter)");
const at = app.indexOf("const ee = everythingElse(wsList");
const block = at < 0 ? "" : app.slice(at, app.indexOf("})()}", at));
ok(/if \(!ee \|\| eeHidden\) return null;/.test(block), "the Workspaces view draws no group, projects included, when it is hidden");
ok(/const open = !settings\.ws_collapsed\?\.\[ee\.id\];/.test(block), "shown, its fold is the stored one, as before");
ok(/<EverythingElseHeader w=\{ee\}/.test(block) && !/eeHeader/.test(block), "shown, the header is drawn as before");
const css = read("../src/App.css");
ok(!/\.bare\b/.test(css), "App.css: no .bare styling is left");

// ── 4. the switch's mark: what a hidden project needs ──
ok(/const eeNeeds: Rollup = eeHidden && navView === "workspaces" \? hiddenNeeds\(/.test(app),
  "eeNeeds is computed only while the group is hidden and the Workspaces view shows");
ok(/needs=\{eeNeeds\}/.test(app), "…and handed to the switch");
const sw = app.slice(app.indexOf("function NavViewSwitch("), app.indexOf("function WsNameField("));
ok(/v === "projects" && needs && <span className=\{"wsc-dot " \+ needs\} data-testid="nav-all-needs"/.test(sw),
  "the switch draws it on the All projects side");
// hiddenNeeds: waiting first, then unread work; busy needs nobody.
const hn = app.match(/const hiddenNeeds = \(places: readonly Place\[\]\): Rollup =>[^;]*;/)?.[0] ?? "";
ok(hn.length > 0, "hiddenNeeds is one expression in App.tsx");
const fn = hn ? new Function("activityOf", "unreadOf", `${hn.replace(/: readonly Place\[\]/, "").replace(/: Rollup/, "")} return hiddenNeeds;`) : null;
if (fn) {
  const f = fn((p) => p.a, (p) => !!p.u);
  ok(f([{ a: "" }, { a: "waiting" }, { u: 1 }]) === "waiting", "hiddenNeeds: a waiting place wins");
  ok(f([{ a: "busy" }, { a: "", u: 1 }]) === "done", "…then unread work");
  ok(f([{ a: "busy" }, { a: "" }]) === "", "…and a busy one alone needs nobody");
  ok(f([]) === "", "…nothing, nothing");
}
const wc = read("../src/WorkspaceChat.css");
ok(/\.wsc-dot\.done \{[^}]*var\(--done\)/.test(wc), "WorkspaceChat.css: a .wsc-dot.done for unread work");

// ── 5. a project taken out of its last workspace leaves the view: the Undo says where it went ──
const rm = app.slice(app.indexOf("const wsRemoveProject = "), app.indexOf("const wsSetOrder = "));
ok(/const gone = !settings\.nav_ee_in_workspaces && !l\.workspaces\.some/.test(rm) && /now only in All projects/.test(rm),
  "wsRemoveProject: with the group hidden, a project in no workspace any more is said to be in All projects");

console.log(bad ? `\n${bad} failed` : "\nall passed");
process.exit(bad ? 1 : 0);
