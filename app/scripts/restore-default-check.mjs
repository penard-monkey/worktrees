// "Restore last place on launch" is ON by default (the owner, 2026-11-25), and
// a value a blob already holds is kept.
//
// ui-state.json is saved WHOLE (`{...DEFAULTS, ...raw}`), so every blob a build
// since #45 (2026-07-27) has saved carries an explicit `restore_last`: false
// for whoever never touched it, exactly as for whoever turned it off. The two
// cannot be told apart on disk, so the new default reaches a fresh install and
// a blob without the key, and a stored value of either kind is kept. No marker
// flips existing installs (as `chat_default_v1` did for chat_view): that would
// also turn it on for those who chose off.
//
//   node app/scripts/restore-default-check.mjs     # exits non-zero on failure
//
// It loads the REAL settings.ts as a data: module (beta-check.mjs's way), so
// the default and loadSettings's answers are evaluated, not grepped.
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
globalThis.__restoreInvoke = (cmd, a) => {
  if (cmd === "set_settings") saved.push(structuredClone(a.settings));
  return Promise.resolve(cmd === "get_settings" ? structuredClone(stored) : null);
};
const inline = (rel) => read(rel).replace(/^export /gm, "") + "\n";
const S = await load(
  inline("../src/afterglow.ts") + inline("../src/harness.ts") +
  read("../src/settings.ts")
    .replace(/^import \{[^}]*\} from "\.\/afterglow";$/m, "")
    .replace(/^import \{[^}]*\} from "\.\/harness";$/m, "")
    .replace(/^import \{ invoke \}.*$/m, "const invoke = (cmd, a) => globalThis.__restoreInvoke(cmd, a);"),
  "settings.ts");

ok(S.DEFAULTS.restore_last === true, "DEFAULTS.restore_last is true");
for (const [blob, want, what] of [
  [null, true, "a fresh install"],
  [{}, true, "an empty blob"],
  [{ settings_rev: 5 }, true, "a blob from before the key"],
  [{ settings_rev: 5, restore_last: false }, false, "a stored false (never touched, or turned off: the same bytes)"],
  [{ settings_rev: 5, restore_last: true }, true, "a stored true"],
  [{ settings_rev: 5, restore_last: "no" }, true, "a junk value: the default"],
]) {
  stored = blob;
  const s = await S.loadSettings();
  ok(s.restore_last === want, `loadSettings: ${what} → ${want}`);
}

// A stored false survives the whole-blob save: load, save, the blob still says false.
stored = { settings_rev: 5, restore_last: false };
const s = await S.loadSettings();
S.saveSettings(s);
await new Promise((r) => setTimeout(r, 300));
ok(saved.length > 0 && saved.at(-1).restore_last === false, "a stored false is written back false by the next save");

// The Settings row says what it does with the new default.
const general = read("../src/pages/general.tsx");
ok(/checked=\{settings\.restore_last\}/.test(general), "pages/general.tsx: the checkbox reads restore_last");

console.log(bad ? `\n${bad} failed` : "\nall passed");
process.exit(bad ? 1 : 0);
