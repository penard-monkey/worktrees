// The one provider chosen for the nav mark and the agent cell. A canonical
// tmux shell can exist alongside Codex; it is not a Claude agent signal.
import assert from "node:assert/strict";
import fs from "node:fs";
import { transformWithEsbuild } from "vite";

const src = fs.readFileSync(new URL("../src/harness.ts", import.meta.url), "utf8");
const js = (await transformWithEsbuild(src, "harness.ts", { loader: "ts", format: "esm" })).code;
const { liveHarness } = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

const sessions = (claude, codex) => ({ claude: { up: claude }, codex: { up: codex } });
for (const [caseName, agents, want] of [
  ["Codex with a leftover canonical shell", sessions(false, true), "codex"],
  ["Claude running", sessions(true, false), "claude"],
  ["both agents running", sessions(true, true), "claude"],
  ["no agent running", sessions(false, false), null],
]) {
  assert.equal(liveHarness(agents), want, caseName);
  console.log(`ok — ${caseName}`);
}
assert.equal(liveHarness(null), null, "no snapshot yet");
console.log("provider-choice-check: all good");
