// Guards the Plan limits settings against the core gate they configure.
//
//   node app/scripts/quota-check.mjs        # exits non-zero on failure
//
// WHY THIS EXISTS. Two things in the frontend mirror `worktrees_core::quota`,
// and both can drift with every test green:
//
//   1. THE RANGE. `QUOTA_PCTS` (QuotaPanel.tsx) offers thresholds; core accepts
//      `PCT_MIN..=PCT_MAX` and REFUSES the write otherwise. An option outside
//      the range is a menu entry that fails when chosen.
//
//   2. THE REMEDY. Every refusal says where to change the setting
//      (`HOW_TO_CHANGE`: "Settings → Behavior → Plan limits"). A message's
//      remedy is a claim (AGENTS.md); renaming the section or moving it to
//      another category would leave every refusal pointing at nothing. So the
//      path is checked against what SettingsSheet actually renders.
//
// Plus the plumbing a new command needs: lib.rs registers both commands and
// the mock answers both, or the harness cannot drive the panel.
import fs from "node:fs";
import { fileURLToPath } from "node:url";

const read = (rel) => fs.readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");

let bad = 0;
const fail = (m) => { console.error(`FAIL ${m}`); bad++; };
const ok = (m) => console.log(`ok   ${m}`);

const quota = read("../../crates/worktrees-core/src/quota.rs").split("#[cfg(test)]\nmod tests")[0];
const panel = read("../src/QuotaPanel.tsx");
const sheet = read("../src/SettingsSheet.tsx");
const lib = read("../src-tauri/src/lib.rs");
const mock = read("../src/mock/install.ts");

// ── 1. the range ─────────────────────────────────────────────────────────────
const num = (name) => {
  const m = [...quota.matchAll(new RegExp(`pub const ${name}: u8 = (\\d+);`, "g"))];
  if (m.length !== 1) { fail(`quota.rs: expected exactly one ${name}, found ${m.length}`); return NaN; }
  return +m[0][1];
};
const [min, max] = [num("PCT_MIN"), num("PCT_MAX")];
const pm = [...panel.matchAll(/export const QUOTA_PCTS = \[([^\]]*)\]/g)];
if (pm.length !== 1) fail(`QuotaPanel.tsx: expected exactly one QUOTA_PCTS, found ${pm.length}`);
else {
  const pcts = pm[0][1].split(",").map((s) => +s.trim());
  const out = pcts.filter((n) => !Number.isInteger(n) || n < min || n > max);
  if (out.length) fail(`QUOTA_PCTS has values outside ${min}..=${max}: ${out.join(", ")}`);
  else ok(`QUOTA_PCTS (${pcts.length} values) inside quota::PCT_MIN..=PCT_MAX (${min}..=${max})`);
}

// ── 2. the remedy names a section that exists ───────────────────────────────
const how = quota.match(/pub const HOW_TO_CHANGE: &str = "([^"]*)/);
const path = how?.[1].match(/Settings → ([^→]+?) → ([^,\\]+?)\s*[,\\]/);
if (!path) fail("quota.rs: HOW_TO_CHANGE no longer names a `Settings → <category> → <section>` path");
else {
  const [cat, section] = [path[1].trim(), path[2].trim()];
  const catId = [...sheet.matchAll(/\{ id: "([a-z-]+)", label: "([^"]+)" \}/g)].find((m) => m[2] === cat)?.[1];
  if (!catId) fail(`HOW_TO_CHANGE names category "${cat}", which SettingsSheet's CATS does not have`);
  else {
    // The section must be rendered INSIDE that category's block.
    const block = sheet.split(`{cat === "${catId}" && <>`)[1]?.split("</>}")[0] ?? "";
    if (!block.includes("<QuotaSection")) fail(`<QuotaSection> is not rendered under cat === "${catId}" (${cat})`);
    else if (!panel.includes(`<label>${section}</label>`)) fail(`QuotaPanel's heading is not "${section}", which HOW_TO_CHANGE promises`);
    else ok(`HOW_TO_CHANGE's "Settings → ${cat} → ${section}" is where QuotaSection renders`);
  }
}

// ── 3. plumbing ─────────────────────────────────────────────────────────────
for (const cmd of ["quota_settings", "set_quota_settings"]) {
  if (!new RegExp(`async fn ${cmd}\\(`).test(lib)) fail(`lib.rs has no command ${cmd}`);
  else if (!new RegExp(`^\\s+${cmd},$`, "m").test(lib)) fail(`lib.rs does not register ${cmd} in invoke_handler`);
  else if (!mock.includes(`case "${cmd}":`)) fail(`mock/install.ts does not answer ${cmd}`);
  else ok(`${cmd}: defined, registered, mocked`);
}

if (bad) { console.error(`\n${bad} failure(s)`); process.exit(1); }
