// The words next to Pair a device come from the real status, including a
// listener paused after the address left. Keep the sheet and the way to
// Remote access wired to the same rule.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const source = (name) => fs.readFileSync(fileURLToPath(new URL(`../src/${name}`, import.meta.url)), "utf8");
const js = (await transformWithEsbuild(source("devices.ts"), "devices.ts", { loader: "ts", format: "esm" })).code;
const D = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));
let failed = 0;
const ok = (condition, name) => {
  console.log(`${condition ? "ok" : "not ok"} — ${name}`);
  if (!condition) failed++;
};

const paused = { state: "paused", address: "100.96.164.69", iface: "utun6", paused: "100.96.164.69 is no longer on utun6" };
const block = (status, full = false, err = null) => D.pairingBlock?.(status, full, err);
const p = block(paused);
ok(p?.kind === "paused" && p.message.includes("100.96.164.69 is no longer on utun6") && p.message.includes("Choose another address"), "paused reason names the lost address and recovery");
ok(block({ state: "off" })?.message.includes("turn it on"), "off still says to turn remote access on");
ok(block({ state: "starting", address: "192.168.1.5", iface: "en0" })?.message.includes("starting"), "starting is described truthfully");
ok(block({ state: "error", error: "bind failed" })?.message.includes("bind failed"), "error is described truthfully");
ok(block({ state: "unreadable", error: "bad JSON" })?.message.includes("bad JSON"), "unreadable is described truthfully");
ok(block(null, false, "socket closed")?.message.includes("socket closed"), "status read failure is named");
ok(block({ state: "listening" }) === null, "listening allows pairing");
ok(block({ state: "listening" }, true)?.message.includes("16 devices"), "device cap still blocks pairing");

const devices = source("DevicesPanel.tsx");
const sheet = source("PairSheet.tsx");
const settings = source("SettingsSheet.tsx");
const css = source("RemoteAccess.css");
ok(devices.includes("pairingBlock(r.status, full, r.statusErr)"), "Devices uses the status rule");
ok(devices.includes("onRemote={onRemote}") && devices.includes("data-testid=\"pair-open-remote\""), "Devices offers a direct Remote access action");
ok(sheet.includes("pairingBlock(r.status") && sheet.includes('data-testid={remoteProblem.kind === "paused" ? "pair-paused"'), "an open sheet sees a later pause");
ok(settings.includes("<DevicesSection onRemote={() => setCat(\"remote\")} />"), "the action opens Remote access");
ok(/\.ra \.enter-btn:disabled\s*\{[^}]*opacity:\s*0?\.\d+[^}]*cursor:\s*default/.test(css), "a disabled Pair a device looks disabled (.enter-btn has no disabled style of its own)");

if (failed) process.exit(1);
