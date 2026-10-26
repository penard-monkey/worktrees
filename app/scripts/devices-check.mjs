// Checks app/src/devices.ts — the words and shapes of the host app's
// remote-access UI — and the mirrors it keeps of serve's Rust: the presets
// (`worktrees serve pair --preset`, client.rs), the capability names
// (authz.rs), the expiry bounds and the device cap (devices.rs), how a pairing
// window ends (pairing.rs `Ended`) and the platforms. A mirror that drifts
// leaves a page that is confidently wrong while every test of either side
// passes (AGENTS.md: "a frontend that mirrors a core DECISION needs a drift
// check"). Also §1.1's label rule, the QR drawing, the alert wording's one
// rule (serve decides what is an alert, the page never adds one), and that a
// window driving a paired host shows these panels greyed with their reason
// and asks that host for none of the console's rows.
//
//   node app/scripts/devices-check.mjs     # exits non-zero on failure
//   DEVICES_TS=<path> node app/scripts/devices-check.mjs   # check another copy
//   SRC_DevicesPanel_tsx=<path> …   # another copy of any app/src file read here
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const here = (rel) => fileURLToPath(new URL(rel, import.meta.url));
const read = (p) => fs.readFileSync(p, "utf8");
/** An app/src file, or the copy `SRC_<its name, . as _>` names. */
const src = (file) => read(process.env[`SRC_${file.replace(/\W/g, "_")}`] ?? here(`../src/${file}`));
const SRC = process.env.DEVICES_TS ?? here("../src/devices.ts");
const js = (await transformWithEsbuild(read(SRC), "devices.ts", { loader: "ts", format: "esm" })).code;
const D = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

let failed = 0;
let passed = 0;
const eq = (name, got, want) => {
  const g = JSON.stringify(got), w = JSON.stringify(want);
  if (g === w) { passed++; return; }
  failed++;
  console.log(`not ok — ${name}\n     got: ${g}\n    want: ${w}`);
};
/** The Rust source with its `mod tests` sliced off (AGENTS.md: cut at the
 *  `#[cfg(test)]` that heads `mod tests`, never the first one). */
const rust = (rel) => {
  const s = read(here(rel));
  const m = /\n#\[cfg\(test\)\]\s*\nmod tests\b/.exec(s);
  return m ? s.slice(0, m.index) : s;
};
const client = rust("../../crates/worktrees-cli/src/serve/client.rs");
const devices = rust("../../crates/worktrees-cli/src/serve/devices.rs");
const pairing = rust("../../crates/worktrees-cli/src/serve/pairing.rs");
const authz = rust("../../crates/worktrees-core/src/authz.rs");

// ── mirrors ────────────────────────────────────────────────────────────────
for (const p of ["observe", "operate", "drive"]) {
  const m = new RegExp(`"${p}" => Ok\\(&\\[([^\\]]*)\\]\\)`).exec(client);
  const want = m ? [...m[1].matchAll(/"([a-z]+)"/g)].map((x) => x[1]) : null;
  eq(`preset ${p} is client.rs's`, [...D.PRESETS[p]], want);
}
const capNames = (() => {
  const body = authz.slice(authz.indexOf("pub fn name(self)"));
  return [...body.slice(0, body.indexOf("}\n")).matchAll(/Capability::\w+ => "([a-z]+)"/g)].map((m) => m[1]);
})();
eq("CAPS are authz::Capability's names, in its order", [...D.CAPS], capNames);
const constOf = (src, name) => Number(new RegExp(`pub const ${name}: \\w+ = (\\d+);`).exec(src)?.[1]);
eq("DEFAULT_IDLE_DAYS is devices.rs's", D.DEFAULT_IDLE_DAYS, constOf(devices, "DEFAULT_IDLE_DAYS"));
eq("MAX_IDLE_DAYS is devices.rs's", D.MAX_IDLE_DAYS, constOf(devices, "MAX_IDLE_DAYS"));
const panel = src("DevicesPanel.tsx");
eq("MAX_DEVICES is devices.rs's", D.MAX_DEVICES, constOf(devices, "MAX_DEVICES"));
const snake = (s) => s.replace(/[A-Z]/g, (c, i) => (i ? "_" : "") + c.toLowerCase());
const variants = (src, name) => {
  const start = src.indexOf(`pub enum ${name} {`);
  const body = src.slice(start, src.indexOf("\n}", start));
  return [...body.matchAll(/^\s{4}([A-Z]\w*)\s*[,{(]/gm)].map((m) => m[1]);
};
eq("ENDED_WORDS has every pairing::Ended, and nothing else",
  Object.keys(D.ENDED_WORDS).sort(), variants(pairing, "Ended").map(snake).sort());
eq("PLATFORM_LABEL has every devices::Platform", Object.keys(D.PLATFORM_LABEL).sort(),
  variants(devices, "Platform").map((v) => v.toLowerCase()).sort());

// ── presets and the §1.1 label ─────────────────────────────────────────────
eq("presetOf observe", D.presetOf(["read"]), "observe");
eq("presetOf drive, any order, admin aside", D.presetOf(["stream", "admin", "read", "write", "instruct", "answer"]), "drive");
eq("presetOf custom", D.presetOf(["read", "stream"]), "custom");
eq("capsWords drive + admin", D.capsWords(["read", "write", "answer", "instruct", "stream", "admin"]), "Drive + host admin");
eq("capsWords custom", D.capsWords(["stream", "read"]), "read, stream");
const runs = (caps, unprompted) => D.runsCommands(caps, unprompted);
eq("§1.1: Observe never runs commands", [runs(["read"], false), runs(["read"], true)], [false, false]);
eq("§1.1: Operate does not (write alone)", [runs(["read", "write"], false), runs(["read", "write"], true)], [false, false]);
eq("§1.1: stream always does", runs(["read", "stream"], false), true);
eq("§1.1: answer with write does, answer alone does not", [runs(["answer", "write"], false), runs(["answer"], false)], [true, false]);
eq("§1.1: answer with instruct does", runs(["answer", "instruct"], false), true);
eq("§1.1: instruct does only while agents act unprompted", [runs(["read", "instruct"], false), runs(["read", "instruct"], true)], [false, true]);
eq("§1.1: Drive does", runs([...D.PRESETS.drive], false), true);
eq("the label names the host", D.runsCommandsLabel("Bret-MBP"), "can run commands on Bret-MBP as you");

// ── expiry ─────────────────────────────────────────────────────────────────
const until = D.untilEndOf("2027-01-12");
eq("an until is serve's one spelling", /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$/.test(until ?? ""), true);
eq("an until is the END of the local day", new Date(until).getHours() * 100 + new Date(until).getMinutes(), 2359);
eq("dayOf inverts untilEndOf", D.dayOf(until), "2027-01-12");
eq("a malformed day is no until", D.untilEndOf("12/01/2027"), null);
eq("expiry words", [D.expiryWords({ idle_days: 30 }), D.expiryWords({ idle_days: 1 }), D.expiryWords("never")],
  ["disabled after 30 days unused", "disabled after 1 day unused", "never expires"]);

// ── pairing ────────────────────────────────────────────────────────────────
eq("the code is two groups of three", D.codeSpaced("048213"), "048 213");
eq("a code that is not six digits is shown as given", D.codeSpaced("12"), "12");
eq("seconds left round up, never below zero", [D.secondsLeft(100, 98_500), D.secondsLeft(100, 120_000)], [2, 0]);

/** Rasterize qrPath's output back into modules: every `M x y h w v1 h-w z`. */
const raster = (size, d) => {
  const m = new Array(size * size).fill("0");
  for (const [, x, y, w] of d.matchAll(/M(\d+) (\d+)h(\d+)v1h-\d+z/g)) for (let i = 0; i < +w; i++) m[+y * size + +x + i] = "1";
  return m.join("");
};
const size = 21;
let seed = 7;
const modules = Array.from({ length: size * size }, () => ((seed = (seed * 1103515245 + 12345) % 2 ** 31) >> 16) & 1 ? "1" : "0").join("");
const d = D.qrPath(size, modules);
eq("qrPath draws exactly the dark modules", raster(size, d), modules);
eq("qrPath merges a row's run into one rectangle", D.qrPath(3, "111000010"), "M0 0h3v1h-3zM1 2h1v1h-1z");
eq("qrPath refuses a matrix of the wrong length", D.qrPath(3, "1"), "");

// ── alerts: worded here, decided by serve ──────────────────────────────────
// The audit log is §5.3's one alert channel (`audit:alert`): every alert it
// emits is a notice, and nothing else is.
const label = (id) => ({ d_1: "Studio Mac" })[id] ?? id;
const sub = { principal: "owner", device: "d_1" };
const device = (event, extra = {}) => D.alertOf({ alert: "device", event, subject: sub, ...extra }, 1);
eq("a device alert is the Devices category's", device("enrolled").cat, "devices");
eq("auth failures are Remote access's", D.alertOf({ alert: "auth_failures", count: 20 }, 1).cat, "remote");
eq("each alert has its own key", D.alertOf({ alert: "x" }, 5).key !== D.alertOf({ alert: "x" }, 6).key, true);
eq("a pairing names the device by the list's name", D.alertText(device("enrolled"), label), "Studio Mac was paired with this computer.");
eq("an unknown id is named by its id", D.alertText(D.alertOf({ alert: "device", event: "enrolled", subject: { principal: "owner", device: "d_9" } }, 1), label), "d_9 was paired with this computer.");
eq("a new address class names the device", D.alertText(device("new_address_class"), label), "Studio Mac connected from a different kind of network than before.");
eq("a knock from a revoked device says so", D.alertText(device("connect_refused", { reason: "revoked" }), label), "Studio Mac tried to connect and was refused: it is revoked.");
eq("an expiry says why", D.alertText(device("disabled", { reason: "idle" }), label), "Studio Mac was disabled: its idle expiry ran out.");
eq("auth failures count", D.alertText(D.alertOf({ alert: "auth_failures", count: 20 }, 1), label).startsWith("20 connection attempts failed authentication"), true);
// The store raises notices from `audit:alert` alone: its `devices:changed`
// handler re-reads, and never pushes one.
const store = src("remoteStore.ts");
const dc = store.slice(store.indexOf('listen<Record<string, unknown>>("devices:changed"'), store.indexOf('listen<Record<string, unknown>>("audit:alert"'));
eq("the devices:changed handler is found", dc.length > 0, true);
eq("devices:changed never raises a notice", /\bpush\(|alertOf\(/.test(dc), false);
eq("audit:alert does", /push\(alertOf\(/.test(store.slice(store.indexOf('listen<Record<string, unknown>>("audit:alert"'))), true);

// ── a window that drives a paired host (Phase 2 §4.4) ──────────────────────
// It is not that host's console. Each panel's FIRST return is GrantNote's
// reason, before any read or control, so a remote window asks for no console
// row and shows none enabled; the store reads no status and hears no console
// event there; the corner says nothing about an engine the window does not
// drive; and every console row this UI calls is one ipc.ts's remote door
// refuses in words (engine-route-check holds CONSOLE_ROWS to dispatch.rs).
const CONSOLE = [...(/export const CONSOLE_ROWS: readonly string\[\] = \[([^\]]*)\]/.exec(src("ipc.ts"))?.[1] ?? "").matchAll(/"([a-z_0-9]+)"/g)].map((m) => m[1]);
eq("ipc.ts CONSOLE_ROWS is found", CONSOLE.length > 30, true);
const fnBody = (text, name) => {
  const i = text.indexOf(`export function ${name}(`);
  return i < 0 ? "" : text.slice(i, text.indexOf("\n}\n", i));
};
const READS = /\binvoke\b|\buse(?:Effect|Projects|Unprompted|Now)\(|<button|<input/;
for (const [file, fn, row] of [["RemoteAccessPanel.tsx", "RemoteAccessSection", "remote_status"], ["DevicesPanel.tsx", "DevicesSection", "pair_begin"]]) {
  const body = fnBody(src(file), fn);
  const gate = new RegExp(`if \\(notGranted\\("${row}"\\) \\|\\| notConsole\\(r\\)\\) \\{\\s*return <GrantNote cmd="${row}"[^>]*\\brefused />`).exec(body);
  eq(`${fn} is found`, body.length > 0, true);
  eq(`${fn}: a remote window gets GrantNote's reason for ${row}, and serve's console-only the same note`, !!gate, true);
  eq(`${fn}: the gate is its first return`, gate ? body.indexOf("return") === gate.index + gate[0].indexOf("return") : false, true);
  eq(`${fn}: nothing reads or renders a control before the gate`, gate ? READS.test(body.slice(0, gate.index)) : true, false);
}
const pair = src("PairSheet.tsx");
eq("the pair sheet's begin is greyed with the remote status and notGranted's reason",
  /const problem = notGranted\("pair_begin"\) \?\? remoteProblem\?\.message \?\? scopeProblem\(scope\);/.test(pair) && /disabled=\{busy \|\| !!problem\} title=\{problem \?\? undefined\} onClick=\{begin\}/.test(pair), true);
const watch = fnBody(store, "watchRemote");
const stop = watch.indexOf('if (notGranted("remote_status")) return');
eq("watchRemote is found", watch.length > 0, true);
eq("watchRemote stops for a remote window before any console read or listen", stop >= 0 && stop < watch.indexOf("readStatus()") && stop < watch.indexOf("listen<"), true);
eq("…and lists devices only where the grant reaches devices_list",
  [...watch.matchAll(/readDevices\(\)/g)].length === 1 && /if \(!notGranted\("devices_list"\)\) void readDevices\(\);/.test(watch.slice(0, Math.max(0, stop))), true);
const layer = fnBody(src("EngineGate.tsx"), "RemoteLayer");
eq("RemoteLayer shows neither the notice nor an alert on a remote window",
  /const onConsole = !notGranted\("remote_status"\);/.test(layer) && /const on = onConsole && /.test(layer) && /const shown = on \|\| \(onConsole && alerts\.length > 0\);/.test(layer), true);
// DeviceRow calls its rows through `act("…", args)`, which adds the id.
const invoked = new Set(["RemoteAccessPanel.tsx", "DevicesPanel.tsx", "PairSheet.tsx", "remoteStore.ts"]
  .flatMap((f) => [...src(f).matchAll(/\b(?:invoke(?:<[^>()]*(?:<[^>]*>[^>()]*)*>)?|act)\(\s*"([a-z_0-9]+)"/g)].map((m) => m[1])));
eq("the rows the remote-access UI calls are found", invoked.size >= 15, true);
// Not console rows, on purpose: `devices_list` is `[admin]` (a device with
// admin lists its own principal's), `device_revoke` is `Own` (a device may
// revoke itself).
const OTHERS = ["devices_list", "device_revoke"];
eq("every row the remote-access UI calls is a console row, bar the two that are not",
  [...invoked].filter((c) => !CONSOLE.includes(c) && !OTHERS.includes(c)), []);
eq("…and that includes the two this UI added", ["remote_candidates", "device_rename"].every((c) => invoked.has(c) && CONSOLE.includes(c)), true);

// ── a record, in words ─────────────────────────────────────────────────────
const rec = {
  id: "d_1", principal: "owner", name: "Phone", platform: "ios", spki_sha256: "", stepup_spki_sha256: "x", stepup_required: false,
  caps: ["read", "stream"], projects: "*", expiry: "never", enrolled_at: 0, last_seen: null, last_addr: null, disabled: null, revoked_at: null,
};
eq("step-up off on an iPhone is said, for as long as it stays off", D.stepupWords(rec, false)?.startsWith("Step-up OFF"), true);
eq("a desktop that runs commands rests on one factor", D.stepupWords({ ...rec, platform: "macos", stepup_spki_sha256: null }, false)?.startsWith("No presence key"), true);
eq("a desktop that only reads says nothing", D.stepupWords({ ...rec, platform: "macos", caps: ["read"] }, false), null);
eq("states", [D.deviceState(rec).kind, D.deviceState({ ...rec, disabled: { at: 1, reason: "idle" } }).kind, D.deviceState({ ...rec, revoked_at: 1 }).kind],
  ["live", "disabled", "revoked"]);

if (failed) {
  console.log(`\n${failed} failed, ${passed} passed`);
  process.exit(1);
}
console.log(`ok ${passed} devices.ts checks`);
