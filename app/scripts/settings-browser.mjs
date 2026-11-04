// Optional browser gate for the Settings regroup and its modes
// (docs/proposals/settings-reorg.md), in Chromium AND WebKit (the app is
// WKWebView; the harness is Chrome — AGENTS.md).
//
//   VITE_MOCK=1 vite --port 1458 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=/tmp/pw/node_modules/playwright/index.mjs \
//     SETTINGS_MOCK_URL=http://localhost:1458 node app/scripts/settings-browser.mjs
// SETTINGS_SCREENSHOTS=<dir> saves a page per mode and engine.
//
// What it holds, and why each needs a browser (settingsmap-check.mjs holds
// the same rules on the map itself; this is what the running sheet DRAWS):
//   - a fresh window opens in Beginner, lists six groups, hides the Advanced
//     pages, and says how many settings Advanced would add — and the switch
//     persists the mode in the blob;
//   - a setting changed in Expert stays drawn in Beginner, with its tier and
//     why (rule 2);
//   - an offer's deep link into a section the mode hides reveals it, with the
//     switch beside it (§4.3);
//   - the confirms: Full access asks first and its arm expires; Reset arms,
//     expires, and resets only its scope;
//   - a remote window: computer pages name the host, a console-only control is
//     drawn disabled with why, an admin-only read is a GrantNote, Restore
//     window is not drawn, and the settings file is this machine's;
//   - every button on every page is REACHABLE (elementFromPoint at its
//     centre), and the new words — group headers, scope sentences, chips, the
//     footer — read at ≥ 4.5:1 in all six themes (composited backgrounds).
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
import fs from "node:fs";
const BASE = process.env.SETTINGS_MOCK_URL ?? "http://localhost:1458";
const SHOTS = process.env.SETTINGS_SCREENSHOTS;
const THEMES = ["tokyo-night", "tokyo-day", "catppuccin-mocha", "catppuccin-latte", "nord", "gruvbox-dark"];

let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };

async function openSheet(page, query = "") {
  await page.goto(`${BASE}/${query}`);
  await page.waitForSelector("[data-track=settings]");
  await page.click("[data-track=settings]");
  await page.waitForSelector(".settings-body .page-title");
}
const mode = (page) => page.evaluate(() => document.querySelector("[data-testid=settings-mode] button.on")?.dataset.mode ?? null);
const setMode = async (page, m) => { await page.click(`[data-testid=settings-mode] [data-mode=${m}]`); await page.waitForTimeout(150); };
const go = async (page, cat) => { await page.click(`.settings-cat[data-cat=${cat}]`); await page.waitForSelector(`.settings-body[data-page=${cat}]`); await page.waitForTimeout(250); };
const listed = (page) => page.evaluate(() => [...document.querySelectorAll(".settings-cat")].map((b) => b.dataset.cat));
const drawn = (page) => page.evaluate(() => [...document.querySelectorAll(".settings-body [data-sec]")].map((s) => s.dataset.sec));
const blob = (page, key = "wt-mock-ui-state") => page.evaluate((k) => JSON.parse(sessionStorage.getItem(k) ?? "null"), key);

/** Every button in the body: reachable at its centre. */
const unreachable = (page) => page.evaluate(() => [...document.querySelectorAll(".settings-body button")].filter((b) => {
  if (b.closest("details:not([open])")) return false;
  b.scrollIntoView({ block: "center" });
  const q = b.getBoundingClientRect();
  if (q.width === 0 || q.height === 0) return false;
  const hit = document.elementFromPoint(q.x + q.width / 2, q.y + q.height / 2);
  return !(hit && (b === hit || b.contains(hit)));
}).map((b) => b.textContent.trim().slice(0, 40)));

/** Contrast of each matched element's text against its composited background. */
const contrast = (page, sel) => page.evaluate((sel) => {
  const parse = (c) => {
    let m = /^rgba?\(([^)]+)\)/.exec(c);
    if (m) { const p = m[1].split(/[ ,/]+/).filter(Boolean).map(Number); return [p[0], p[1], p[2], p[3] ?? 1]; }
    m = /^color\(srgb ([^)]+)\)/.exec(c);
    if (m) { const p = m[1].split(/[ /]+/).filter(Boolean).map(Number); return [p[0] * 255, p[1] * 255, p[2] * 255, p[3] ?? 1]; }
    return [0, 0, 0, 0];
  };
  const over = (top, under) => { const a = top[3]; return [0, 1, 2].map((i) => top[i] * a + under[i] * (1 - a)).concat(1); };
  const bgOf = (el) => {
    const stack = [];
    for (let e = el; e; e = e.parentElement) { const c = parse(getComputedStyle(e).backgroundColor); if (c[3] > 0) stack.push(c); if (c[3] >= 1) break; }
    let acc = [255, 255, 255, 1];
    for (const c of stack.reverse()) acc = over(c, acc);
    return acc;
  };
  const lum = (c) => { const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]); };
  return [...document.querySelectorAll(sel)].filter((e) => e.textContent.trim()).map((e) => {
    const bg = bgOf(e);
    const fg = over(parse(getComputedStyle(e).color), bg);
    const [a, b] = [lum(fg), lum(bg)].sort((x, y) => y - x);
    return { text: e.textContent.trim().slice(0, 30), ratio: (a + 0.05) / (b + 0.05) };
  });
}, sel);

async function run(engine, name) {
  const browser = await engine.launch();
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 860 } });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `[${name}] page error: ${e.message}`));

  // ── a fresh window: Beginner ─────────────────────────────────────────────
  await openSheet(page);
  ok(await mode(page) === "beginner", `[${name}] a fresh window opens Settings in Beginner`);
  const groups = await page.evaluate(() => [...document.querySelectorAll(".settings-group-h")].map((h) => h.textContent));
  ok(groups.join("|") === "Workspace|Agents|Agent policy|Projects|Remote|System", `[${name}] six groups: ${groups.join(", ")}`);
  const beg = await listed(page);
  ok(!beg.includes("reference") && !beg.includes("guidance"), `[${name}] Beginner lists no Advanced page that holds nothing changed (${beg.join(", ")})`);
  // A page above the mode is listed only for a section that is drawn anyway,
  // and such a section says why (rule 2: the mock has a profile).
  if (beg.includes("profiles")) {
    await go(page, "profiles");
    ok(await page.locator("[data-testid=why-profiles]").count() === 1, `[${name}] Profiles is listed in Beginner only because it holds a profile, and says so`);
    await go(page, "general");
  }
  ok(beg.includes("policy") && beg.includes("general") && beg.includes("shortcuts"), `[${name}] Beginner lists General, Agent policy, Shortcuts`);
  const gen = await drawn(page);
  ok(gen.includes("restore-last") && gen.includes("editor") && gen.includes("pull-requests") && !gen.includes("terminal-command") && !gen.includes("auto-fetch"),
    `[${name}] General in Beginner: ${gen.join(", ")}`);
  const more = await page.locator("[data-testid=page-more]").textContent().catch(() => null);
  ok(/^\d+ more settings? in Advanced ›$/.test(more ?? ""), `[${name}] General says what Advanced adds: "${more}"`);
  ok(await page.locator("[data-testid=page-scope]").textContent() === "These settings are this window's.", `[${name}] General says whose its settings are`);
  if (SHOTS) await page.locator(".settings-modal").screenshot({ path: `${SHOTS}/${name}-beginner-general.png` });

  // the footer switches the mode in place, and the blob keeps it
  await page.click("[data-testid=page-more]");
  await page.waitForTimeout(300);
  ok(await mode(page) === "advanced", `[${name}] "N more settings in Advanced ›" switches to Advanced`);
  await page.waitForTimeout(400);
  ok((await blob(page))?.settings_mode === "advanced", `[${name}] the mode is saved in the window's blob`);
  ok((await listed(page)).includes("reference"), `[${name}] Advanced lists Reference panels`);
  ok((await drawn(page)).includes("terminal-command"), `[${name}] Advanced draws Terminal command`);

  // ── rule 2: changed in Expert, still drawn in Beginner ───────────────────
  await setMode(page, "expert");
  await go(page, "appearance");
  await page.locator(".sec[data-sec=density] .seg button", { hasText: "compact" }).click();
  await page.waitForTimeout(200);
  await setMode(page, "beginner");
  const why = await page.locator("[data-testid=why-density]").textContent().catch(() => null);
  ok(!!why && why.includes("Advanced setting") && why.includes("not at its default"), `[${name}] a changed Advanced setting stays drawn in Beginner: "${why}"`);
  ok(!(await drawn(page)).includes("ui-font"), `[${name}] …while an unchanged one stays hidden`);
  if (SHOTS) await page.locator(".settings-modal").screenshot({ path: `${SHOTS}/${name}-beginner-changed.png` });

  // ── confirms: Full access asks, and its arm expires ──────────────────────
  await go(page, "policy");
  const pol = await drawn(page);
  ok(pol.includes("codex-permissions") && pol.includes("main-guard") && pol.includes("plan-limits"), `[${name}] Agent policy in Beginner: ${pol.join(", ")}`);
  // Every section drawn ABOVE the mode says why (rules 2 and 3).
  const unexplained = await page.evaluate(() => [...document.querySelectorAll(".settings-body .sec[data-tier]")]
    .filter((s) => s.dataset.tier !== "beginner" && !s.querySelector(".sec-why")).map((s) => s.dataset.sec));
  ok(unexplained.length === 0, `[${name}] every Advanced/Expert section drawn in Beginner says why${unexplained.length ? ` — not: ${unexplained.join(", ")}` : ""}`);
  const onMode = () => page.evaluate(() => document.querySelector("[data-testid=codex-permissions] button.on")?.dataset.testid);
  const before = await onMode();
  await page.click("[data-testid=codex-permissions-full]");
  await page.waitForTimeout(150);
  ok(await page.locator("[data-testid=codex-full-confirm]").count() === 1 && await onMode() === before, `[${name}] Full access asks first, and changes nothing yet`);
  await page.waitForTimeout(4300);
  ok(await page.locator("[data-testid=codex-full-confirm]").count() === 0, `[${name}] …and the arm expires after 4 s`);
  await page.evaluate(() => { const b = document.querySelector("[data-testid=codex-permissions-full]"); b.click(); });
  await page.waitForTimeout(100);
  await page.evaluate(() => { document.querySelector("[data-testid=codex-permissions-full]").click(); });
  await page.waitForTimeout(400);
  ok(await onMode() === "codex-permissions-full", `[${name}] two clicks set Full access`);
  await page.click("[data-testid=codex-permissions-auto-review]");
  await page.waitForTimeout(300);
  ok(await onMode() === "codex-permissions-auto-review", `[${name}] going back to Auto-review needs no confirm`);
  if (SHOTS) await page.locator(".settings-modal").screenshot({ path: `${SHOTS}/${name}-beginner-policy.png` });

  // ── reset: the default scope takes the look, not a silenced suggestion ───
  await page.keyboard.press("Escape");
  await page.waitForTimeout(200);
  // silence one offer, from the offers list
  await page.click("[data-track=offers]");
  await page.waitForSelector(".wn-offer");
  await page.locator(".wn-offer", { hasText: "Let Worktrees keep your agents' plans" }).locator(".mcp-dismiss").click();
  await page.waitForTimeout(600);
  const silenced = Object.keys((await blob(page))?.offers_dismissed ?? {});
  ok(silenced.includes("planning"), `[${name}] an offer silenced (${silenced.join(", ")})`);

  // ── a deep link into a section the mode hides ────────────────────────────
  await page.locator(".wn-offer", { hasText: "Let agents see your other projects" }).locator(".enter-btn").click();
  await page.waitForSelector(".settings-body[data-page=policy]");
  await page.waitForTimeout(400);
  ok(await mode(page) === "beginner", `[${name}] the link does not change the mode`);
  ok((await drawn(page)).includes("cross-project"), `[${name}] an offer's link into an Advanced section reveals it in Beginner`);
  ok(await page.locator("[data-testid=reveal-switch-cross-project]").count() === 1, `[${name}] …with the switch to Advanced beside it`);
  ok(await page.evaluate(() => document.querySelector(".settings-cat.on")?.dataset.cat) === "policy", `[${name}] …on the page it moved to (Agent policy)`);
  if (SHOTS) await page.locator(".settings-modal").screenshot({ path: `${SHOTS}/${name}-beginner-revealed.png` });
  await page.click("[data-testid=reveal-switch-cross-project]");
  await page.waitForTimeout(200);
  ok(await mode(page) === "advanced", `[${name}] the reveal's switch moves to Advanced`);

  await go(page, "data");
  await page.click("[data-testid=reset-go]");
  ok((await page.locator("[data-testid=reset-go]").textContent()) === "Confirm reset?", `[${name}] Reset arms first`);
  await page.waitForTimeout(4300);
  ok((await page.locator("[data-testid=reset-go]").textContent()) !== "Confirm reset?", `[${name}] …and its arm expires`);
  await page.evaluate(() => { document.querySelector("[data-testid=reset-go]").click(); });
  await page.waitForTimeout(80);
  await page.evaluate(() => { document.querySelector("[data-testid=reset-go]").click(); });
  await page.waitForTimeout(500);
  const after = await blob(page);
  ok(after?.density === "comfortable", `[${name}] Reset (Appearance & layout) put density back`);
  ok(Object.keys(after?.offers_dismissed ?? {}).includes("planning"), `[${name}] …and kept the silenced suggestion`);
  ok(after?.settings_mode === "advanced", `[${name}] …and kept the mode`);

  // ── every page in Expert: buttons reachable, nothing throws ──────────────
  await setMode(page, "expert");
  for (const cat of await listed(page)) {
    await go(page, cat);
    const lost = await unreachable(page);
    ok(lost.length === 0, `[${name}] ${cat}: every button is reachable${lost.length ? ` — not: ${lost.join(" | ")}` : ""}`);
  }

  // ── contrast of the new words, in every theme ────────────────────────────
  await go(page, "general");
  for (const theme of THEMES) {
    await page.evaluate((t) => { document.documentElement.dataset.theme = t; }, theme);
    await page.waitForTimeout(60);
    const rows = [
      ...await contrast(page, ".settings-group-h"), ...await contrast(page, ".page-scope"),
      ...await contrast(page, ".sec-chip"), ...await contrast(page, ".page-title"),
    ];
    const low = rows.filter((r) => r.ratio < 4.5);
    ok(low.length === 0, `[${name}] ${theme}: group headers, scope, chips ≥ 4.5:1${low.length ? ` — ${low.map((r) => `"${r.text}" ${r.ratio.toFixed(2)}`).join(", ")}` : ` (min ${Math.min(...rows.map((r) => r.ratio)).toFixed(2)})`}`);
  }
  await ctx.close();

  // ── presets (§5): status, apply, the marker, back to preset, Compare ───────
  const pctx = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const pp = await pctx.newPage();
  pp.on("pageerror", (e) => ok(false, `[${name}] preset page error: ${e.message}`));
  await openSheet(pp);
  await go(pp, "policy");
  await pp.waitForSelector("[data-testid=preset-seg] button.on");
  const onPreset = () => pp.evaluate(() => document.querySelector("[data-testid=preset-seg] button.on")?.dataset.preset);
  const codexOn = () => pp.evaluate(() => document.querySelector("[data-testid=codex-permissions] button.on")?.dataset.testid);
  ok(await onPreset() === "balanced", `[${name}] a fresh host reads as Balanced (today's defaults)`);
  ok(await pp.locator("[data-testid=preset-modified]").count() === 0, `[${name}] …unmodified`);
  await pp.click("[data-testid=preset-compare]");
  const cols = await pp.evaluate(() => [...document.querySelectorAll("[data-testid=preset-table] thead th")].map((t) => [t.textContent, t.className]));
  ok(cols.map((c) => c[0]).join("|") === "|Cautious|Balanced|Autonomous" && cols[2][1] === "cur", `[${name}] Compare shows all three, Balanced highlighted`);
  await pp.click("[data-preset=cautious]");
  await pp.waitForTimeout(500);
  ok(await onPreset() === "cautious", `[${name}] Cautious applies in one click`);
  ok(await codexOn() === "codex-permissions-ask", `[${name}] …and the section below re-reads what it wrote (Codex: Ask)`);
  await pp.click("[data-testid=codex-permissions-auto-review]");
  await pp.waitForTimeout(500);
  await go(pp, "general"); await go(pp, "policy");
  await pp.waitForSelector("[data-testid=preset-modified]");
  const marker = await pp.locator("[data-testid=preset-modified]").textContent();
  ok(marker.includes("Cautious (modified:") && marker.includes("Codex permissions"), `[${name}] a changed value marks the preset modified: "${marker.slice(0, 60)}"`);
  ok(await pp.locator("[data-testid=preset-back-codex_access]").count() === 1, `[${name}] …and its control offers the way back`);
  await pp.click("[data-testid=preset-back-codex_access] button");
  await pp.waitForTimeout(500);
  ok(await codexOn() === "codex-permissions-ask" && await pp.locator("[data-testid=preset-modified]").count() === 0, `[${name}] back to preset restores that one value and clears the marker`);
  await pp.click("[data-preset=autonomous]");
  await pp.waitForTimeout(150);
  ok(await pp.locator("[data-testid=preset-full-confirm]").count() === 1 && await onPreset() === "cautious", `[${name}] Autonomous asks first, because it sets Full access`);
  await pp.click("[data-preset=autonomous]");
  await pp.waitForTimeout(500);
  ok(await onPreset() === "autonomous" && await codexOn() === "codex-permissions-full", `[${name}] …and a second click applies it`);
  if (SHOTS) await pp.locator(".settings-modal").screenshot({ path: `${SHOTS}/${name}-preset.png` });

  // ── Limits (§6 item 1) ───────────────────────────────────────────────────
  await setMode(pp, "advanced");
  await pp.locator("[data-testid=limit-cap-all]").scrollIntoViewIfNeeded();
  const capBefore = await pp.locator("[data-testid=limit-cap-all]").textContent();
  await pp.click("[data-testid=limit-inc-all]");
  await pp.waitForTimeout(300);
  ok(Number(await pp.locator("[data-testid=limit-cap-all]").textContent()) === Number(capBefore) + 1, `[${name}] Limits: + raises the live-agent cap (${capBefore} → ${await pp.locator("[data-testid=limit-cap-all]").textContent()})`);
  ok(await pp.locator("[data-testid=limit-reset-all]").count() === 1, `[${name}] …and offers the way back to the machine default`);
  await pp.click("[data-testid=limit-reset-all]");
  await pp.waitForTimeout(300);
  ok(await pp.locator("[data-testid=limit-reset-all]").count() === 0, `[${name}] …which removes the key`);
  await pctx.close();

  // ── Notifications (§6 item 9): drawn in Beginner; permission only on a click
  {
    const nctx = await browser.newContext({ viewport: { width: 1280, height: 900 } });
    const np = await nctx.newPage();
    await openSheet(np);
    await go(np, "agents");
    const on = await np.evaluate(() => document.querySelector("[data-testid=notify-mode] button.on")?.dataset.mode);
    ok(on === "waiting", `[${name}] Notifications is a Beginner section, on "when an agent waits" by default (${on})`);
    ok(await np.locator("[data-testid=notify-allow]").count() === 1, `[${name}] …and offers to ask the system, rather than asking on load`);
    await np.click("[data-testid=notify-allow]");
    await np.waitForTimeout(200);
    ok(await np.locator("[data-testid=notify-allow]").count() === 0, `[${name}] …and the button goes once allowed`);
    ok((await np.evaluate(() => window.__mock.notifications().length)) === 0, `[${name}] nothing was sent while the window is visible`);
    await nctx.close();
  }

  // ── "set by the environment" (§6 item 4) ─────────────────────────────────
  const ectx = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const ep = await ectx.newPage();
  await openSheet(ep, "?env=codex,pi,guidance");
  await setMode(ep, "expert");
  await go(ep, "policy");
  await ep.waitForSelector(".sec[data-sec=codex-permissions] [data-testid=env-origin]");
  const codexEnv = await ep.locator(".sec[data-sec=codex-permissions] [data-testid=env-origin]").textContent();
  ok(codexEnv.includes("WORKTREES_CODEX_PERMISSIONS=full") && codexEnv.includes("wins over it from then on"), `[${name}] Codex: the environment is named, and a choice here is said to win`);
  ok(await ep.evaluate(() => !document.querySelector("[data-testid=codex-permissions-ask]").disabled), `[${name}] …so the control stays usable`);
  ok(await ep.locator(".sec[data-sec=pi-trust] [data-testid=env-origin]").count() === 1, `[${name}] pi trust names its variable too`);
  await ep.click("[data-testid=codex-permissions-ask]");
  await ep.waitForTimeout(400);
  ok(await ep.evaluate(() => document.querySelector(".sec[data-sec=codex-permissions] [data-testid=env-origin]")?.dataset.source) === "host", `[${name}] after a choice, the host's own setting is said to win over the variable`);
  await go(ep, "guidance");
  const gEnv = await ep.locator(".sec[data-sec=agent-guidance] [data-testid=env-origin]").textContent();
  ok(gEnv.includes("WORKTREES_AGENT_GUIDANCE=off") && gEnv.includes("wins over this switch"), `[${name}] Guidance: the variable wins over the switch, and says so`);
  ok(await ep.evaluate(() => document.querySelector("[data-testid=guidance-enabled]").disabled), `[${name}] …and the switch it outranks is disabled`);
  await ectx.close();

  // ── a remote window ──────────────────────────────────────────────────────
  const rctx = await browser.newContext({ viewport: { width: 1280, height: 860 } });
  const rp = await rctx.newPage();
  rp.on("pageerror", (e) => ok(false, `[${name}] remote page error: ${e.message}`));
  await openSheet(rp, "?remote=noadmin");
  ok(await mode(rp) === "beginner", `[${name}] a remote window keeps its own mode (fresh device blob: Beginner)`);
  await setMode(rp, "expert");
  await rp.waitForTimeout(500);
  // The harness's `?remote` marks the window remote without the shim's door,
  // so its blob is the harness's one key; the real door routes it to
  // remote-ui-state.json (ipc.ts REMOTE_SETTINGS).
  ok(((await blob(rp, "wt-mock-ui-state:remote")) ?? (await blob(rp)))?.settings_mode === "expert", `[${name}] …and saves it in the window's blob`);
  ok(!(await drawn(rp)).includes("restore-window"), `[${name}] Restore window is not drawn on a remote window`);
  await go(rp, "policy");
  const scope = await rp.locator("[data-testid=page-scope]").textContent();
  ok(scope.includes("Studio"), `[${name}] a computer page names the host: "${scope}"`);
  const codex = await rp.evaluate(() => {
    const s = document.querySelector(".sec[data-sec=codex-permissions]");
    return { disabled: s?.disabled, line: !!s?.querySelector("[data-testid=grant-line]"), btn: s?.querySelector("button")?.matches(":disabled") };
  });
  ok(codex.disabled && codex.line && codex.btn, `[${name}] Codex permissions: read-only, controls disabled, and says why`);
  const presetRemote = await rp.evaluate(() => {
    const s = document.querySelector(".sec[data-sec=policy-preset]");
    return { disabled: s?.disabled, line: !!s?.querySelector("[data-testid=grant-line]"), named: !!s?.querySelector("[data-testid=preset-seg] button.on") };
  });
  ok(presetRemote.disabled && presetRemote.line && presetRemote.named, `[${name}] the preset is read on a remote window, and applied only at the host`);
  await go(rp, "profiles");
  ok(await rp.locator(".sec[data-sec=profiles] [data-testid=grant-note]").count() === 1, `[${name}] Profiles (an admin read) is a GrantNote for a device without admin`);
  await go(rp, "general");
  ok(await rp.locator(".sec[data-sec=auto-fetch] [data-testid=grant-line]").count() === 1, `[${name}] Auto-fetch (admin) says why it cannot be changed here`);
  await go(rp, "data");
  const file = await rp.locator("[data-testid=settings-file-path]").textContent();
  ok(file.includes("remote-ui-state.json"), `[${name}] the settings file is this machine's: "${file}"`);
  if (SHOTS) await rp.locator(".settings-modal").screenshot({ path: `${SHOTS}/${name}-remote-data.png` });
  await rctx.close();
  await browser.close();
}

if (SHOTS) fs.mkdirSync(SHOTS, { recursive: true });
await run(chromium, "chromium");
await run(webkit, "webkit");
console.log(failed ? `\n${failed} failure(s)` : "\nall good");
process.exit(failed ? 1 : 0);
