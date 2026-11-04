// Guards "notify while the window is hidden" (settings-reorg §6 item 9).
//
//   node app/scripts/notify-check.mjs     # exits non-zero on failure
//
// A notification that fires twice, or for something already on screen, is
// worse than none — and the cases that do it (a reload, the snapshot serve
// sends a new socket, a remote window's reconnect) are exactly the ones no
// other suite runs. So the REAL notify.ts is evaluated against them, and the
// wiring is held: App feeds it from the two session events and sends only
// through notifyhost.ts, and only the Settings button asks the system for
// permission (an act — never a load).
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const read = (rel) => fs.readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");
let bad = 0;
const fail = (m) => { console.error(`FAIL ${m}`); bad++; };
const ok = (m) => console.log(`ok   ${m}`);

const src = read("../src/notify.ts");
if (/^import /m.test(src)) fail("notify.ts imports something — it must load alone, as a data: module");
const js = (await transformWithEsbuild(src, "notify.ts", { loader: "ts", format: "esm" })).code;
const N = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));
const eq = (got, want, what) => (JSON.stringify(got) === JSON.stringify(want) ? ok(what) : fail(`${what}: got ${JSON.stringify(got)}, want ${JSON.stringify(want)}`));

{
  const n = N.newNotifier();
  const t0 = 1_000_000;
  N.onVisibility(n, true, t0);
  eq(N.onWaitingSnapshot(n, ["a"], t0, "waiting", true), [], "the first snapshot is a baseline: what already waits is not news");
  eq(N.onWaitingSnapshot(n, ["a", "b"], t0 + 1, "waiting", true), ["b"], "a place that STARTS waiting while hidden is announced");
  eq(N.onWaitingSnapshot(n, ["a", "b"], t0 + 2, "waiting", true), [], "a replayed snapshot (a reconnect, a new socket) announces nothing");
  eq(N.onWaitingSnapshot(n, ["a"], t0 + 3, "waiting", true), [], "leaving the set announces nothing");
  eq(N.onWaitingSnapshot(n, ["a", "b"], t0 + 4, "waiting", true), [], "a set that flickers does not announce the same place twice");
  eq(N.onWaitingSnapshot(n, ["a"], t0 + N.RENOTIFY_MS + 5, "waiting", true), [], "…");
  eq(N.onWaitingSnapshot(n, ["a", "b"], t0 + N.RENOTIFY_MS + 6, "waiting", true), ["b"], "after RENOTIFY_MS a new wait is news again");
  eq(N.onWaitingSnapshot(n, ["a", "b", "c"], t0 + N.RENOTIFY_MS + 7, "waiting", false), [], "nothing while the window is visible");
  eq(N.onWaitingSnapshot(n, ["a", "b", "c", "d"], t0 + N.RENOTIFY_MS + 8, "off", true), [], "nothing in Off");
}
{
  const n = N.newNotifier();
  const t0 = 5_000_000;
  N.onWaitingSnapshot(n, [], t0, "both", false);
  N.onVisibility(n, true, t0);
  eq(N.onDone(n, "p", t0 + 1000, "both", true), false, "a finish a moment after hiding is not news (a glance away)");
  eq(N.onDone(n, "p", t0 + N.DONE_AFTER_HIDDEN_MS, "waiting", true), false, "finishes are announced only in Both");
  eq(N.onDone(n, "p", t0 + N.DONE_AFTER_HIDDEN_MS, "both", true), true, "a finish after the window has been away is announced");
  eq(N.onDone(n, "p", t0 + N.DONE_AFTER_HIDDEN_MS + 10, "both", true), false, "…once");
  N.onVisibility(n, false, t0 + N.DONE_AFTER_HIDDEN_MS + 20);
  N.onVisibility(n, true, t0 + N.DONE_AFTER_HIDDEN_MS + 30);
  eq(N.onDone(n, "q", t0 + N.DONE_AFTER_HIDDEN_MS + 40, "both", true), false, "showing the window restarts the clock");
  eq(N.onDone(n, "q", t0 + N.DONE_AFTER_HIDDEN_MS + 50, "both", false), false, "nothing while visible");
}
// The setting's three words are the module's.
{
  const settings = read("../src/settings.ts");
  const typed = /notify_hidden: "off" \| "waiting" \| "both";/.test(settings);
  if (!typed || N.NOTIFY_MODES.join() !== "off,waiting,both") fail("settings.ts's notify_hidden is not notify.ts's NOTIFY_MODES");
  else ok("the setting offers exactly notify.ts's modes");
}
// The wiring.
{
  const app = read("../src/App.tsx");
  const busy = app.slice(app.indexOf('listen<{ busy: string[]; waiting: string[]'), app.indexOf("return () =>", app.indexOf('listen<{ busy: string[]; waiting: string[]')));
  const done = app.slice(app.indexOf('listen<{ path: string; epoch: number }>("sessions:done"'), app.indexOf("return () =>", app.indexOf('listen<{ path: string; epoch: number }>("sessions:done"')));
  if (!/onWaitingSnapshot\(notifier\.current, e\.payload\.waiting/.test(busy)) fail("App's sessions:busy listener does not feed notify.ts");
  else ok("sessions:busy feeds notify.ts");
  if (!/onDone\(notifier\.current, e\.payload\.path/.test(done)) fail("App's sessions:done listener does not feed notify.ts");
  else ok("sessions:done feeds notify.ts");
  const srcDir = fileURLToPath(new URL("../src/", import.meta.url));
  const walk = (d) => fs.readdirSync(d, { withFileTypes: true }).flatMap((e) => e.isDirectory() ? (e.name === "mock" ? [] : walk(d + e.name + "/")) : /\.tsx?$/.test(e.name) ? [d + e.name] : []);
  const files = walk(srcDir);
  const plugin = files.filter((f) => fs.readFileSync(f, "utf8").includes("@tauri-apps/plugin-notification")).map((f) => f.slice(srcDir.length));
  if (plugin.join() !== "notifyhost.ts") fail(`the notification plugin is imported outside notifyhost.ts (${plugin.join(", ")})`);
  else ok("only notifyhost.ts imports the plugin");
  const askers = files.filter((f) => /askNotifications\(/.test(fs.readFileSync(f, "utf8"))).map((f) => f.slice(srcDir.length));
  if (askers.filter((f) => f !== "notifyhost.ts").join() !== "pages/agents.tsx") fail(`permission is asked for outside Settings → Agents (${askers.join(", ")})`);
  else if (!/onClick=\{\(\) => void askNotifications\(\)/.test(read("../src/pages/agents.tsx"))) fail("Settings → Agents asks for permission other than from its button");
  else ok("permission is asked for only by the Settings button");
}

// The plugin is wired on the Rust side too: a page that calls it in an app
// built without it, or without the capability, fails silently (AGENTS.md: a
// missing permission rejects the invoke with nothing on screen).
{
  const cargo = read("../src-tauri/Cargo.toml");
  const lib = read("../src-tauri/src/lib.rs");
  const caps = JSON.parse(read("../src-tauri/capabilities/default.json"));
  if (!/^tauri-plugin-notification = "2"$/m.test(cargo)) fail("app/src-tauri/Cargo.toml does not depend on tauri-plugin-notification");
  else if (!/\.plugin\(tauri_plugin_notification::init\(\)\)/.test(lib)) fail("the shim does not register the notification plugin");
  else if (!caps.permissions.includes("notification:default")) fail("capabilities/default.json does not grant notification:default");
  else ok("the plugin is a dependency, registered, and granted");
}

console.log(bad ? `\n${bad} failure(s)` : "\nall good");
process.exit(bad ? 1 : 0);
