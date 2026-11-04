// Settings → Updates → Agent CLIs: the older sessions under a tool's row and
// their two restarts (harnessrestart.ts), in Chromium and WebKit.
// Run only against VITE_MOCK=1 on port 1456; the restart rows are then mocks.
//   PLAYWRIGHT_MODULE=…/playwright/index.mjs node app/scripts/harness-restart-browser.mjs
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");

const BASE = process.env.HARNESS_MOCK_URL ?? "http://localhost:1456";
let failures = 0;
const ok = (condition, label) => {
  console.log(`${condition ? "ok  " : "FAIL"} ${label}`);
  if (!condition) failures++;
};

/** Open Settings → Updates from the update bubble. */
async function openUpdates(page, query) {
  await page.goto(`${BASE}/${query}`);
  const bubble = page.locator("[data-testid=update-bubble]");
  await bubble.waitFor({ timeout: 12_000 });
  await bubble.locator(".enter-btn").click();
  await page.locator("[data-testid=harness-updates]").waitFor();
}

/** The element's text colour against its first opaque background, as a ratio. */
async function contrast(locator) {
  return locator.evaluate((el) => {
    const rgb = (s) => {
      const m = s.match(/rgba?\(([^)]+)\)/);
      if (m) { const [r, g, b, a = 1] = m[1].split(/[ ,/]+/).filter(Boolean).map(Number); return [r, g, b, a]; }
      const c = s.match(/color\(srgb ([^)]+)\)/);
      if (c) { const [r, g, b, a = 1] = c[1].split(/[ /]+/).filter(Boolean).map(Number); return [r * 255, g * 255, b * 255, a]; }
      return [0, 0, 0, 0];
    };
    let bg = [0, 0, 0, 0];
    const layers = [];
    for (let n = el; n; n = n.parentElement) {
      const c = rgb(getComputedStyle(n).backgroundColor);
      if (c[3] > 0) layers.push(c);
      if (c[3] >= 1) break;
    }
    bg = layers.pop() ?? [255, 255, 255, 1];
    for (const l of layers.reverse()) bg = [0, 1, 2].map((i) => l[i] * l[3] + bg[i] * (1 - l[3])).concat(1);
    const fg = rgb(getComputedStyle(el).color);
    const lum = ([r, g, b]) => {
      const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; };
      return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
    };
    const [a, b] = [lum(fg), lum(bg)].sort((x, y) => y - x);
    return (a + 0.05) / (b + 0.05);
  });
}

/** A button is clickable where it is drawn (AGENTS.md: a rect is not reachability). */
const reachable = (locator) => locator.evaluate((el) => {
  const r = el.getBoundingClientRect();
  const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
  return !!hit && (hit === el || el.contains(hit));
});

for (const [engine, type] of Object.entries({ chromium, webkit })) {
  const browser = await type.launch({ headless: true });

  // ── the list, the confirms, an inactive restart with one failure ─────────
  let page = await browser.newPage({ viewport: { width: 1120, height: 800 } });
  await openUpdates(page, "?harnessupdates=1&harnessrestart=1&restartfail=fix-login");
  const panel = page.locator("[data-testid=harness-restart-claude]");
  await panel.waitFor({ timeout: 8_000 });
  const text = await panel.textContent();
  ok(text.includes("5 Claude sessions run older versions (2.1.314, 2.1.316); 2.1.317 is installed."), `${engine}: the summary counts older sessions and names versions`);
  ok(text.includes("2 more Claude sessions outside Worktrees"), `${engine}: outside sessions are listed for restart by hand`);
  ok(text.includes("1 of them can't be resumed exactly"), `${engine}: an unresumable session is said, not offered`);
  const inactive = panel.locator("[data-testid=restart-inactive-claude]");
  const force = panel.locator("[data-testid=restart-force-claude]");
  ok((await inactive.textContent()) === "Restart 2 inactive", `${engine}: the inactive button counts idle sessions`);
  ok((await force.textContent()) === "Force restart all 4", `${engine}: the force button counts every restartable one`);
  ok(await reachable(inactive) && await reachable(force), `${engine}: both buttons are reachable where drawn`);
  ok(await contrast(panel.locator(".harness-restart-summary")) >= 4.5, `${engine}: the summary reads at ≥ 4.5:1`);
  ok(await page.locator("[data-testid=harness-restart-codex]").textContent().then((t) => t.includes("1 Codex session started before Codex 0.166.0 was installed.")),
    `${engine}: an older Codex with no known version says when it started`);

  await force.click();
  const confirm = panel.locator("[data-testid=restart-confirm-claude]");
  ok((await confirm.textContent()).includes("interrupts 1 session mid-turn and cancels 1 open question")
    && (await confirm.textContent()).includes("the (main) agent of worktrees and orfis"), `${engine}: the force confirm warns and names the (main)s`);
  ok((await force.textContent()) === "Confirm force restart", `${engine}: the armed force button asks for the second click`);
  ok(await contrast(confirm) >= 4.5, `${engine}: the force warning's words read at ≥ 4.5:1 (the accent is a fill)`);
  // Arming the other button disarms this one: one arm at a time.
  await inactive.click();
  ok((await confirm.textContent()).startsWith("This restarts 2 open Claude sessions. Each resumes its conversation."), `${engine}: the inactive confirm`);
  ok((await force.textContent()) === "Force restart all 4", `${engine}: arming one disarms the other`);
  await inactive.click();
  const results = panel.locator("[data-testid=restart-results-claude]");
  await results.waitFor();
  const r = await results.textContent();
  ok(r.includes("1 restarted · 3 skipped · 1 failed"), `${engine}: the tally (got ${JSON.stringify(r.slice(0, 40))})`);
  ok(r.includes("api · (main): restarted, now 2.1.317"), `${engine}: a restarted session names its new version`);
  ok(r.includes("worktrees · fix-login: failed — tmux respawn-pane exited 1"), `${engine}: a failed restart says why`);
  ok(r.includes("worktrees · (main): skipped — it is busy") && r.includes("orfis · (main): skipped — it is waiting on a prompt"), `${engine}: active sessions are skipped without force`);
  await page.waitForFunction(() => document.querySelector("[data-testid=restart-inactive-claude]")?.textContent === "Restart 1 inactive");
  ok(true, `${engine}: the list is read again after a restart`);
  await page.close();

  // ── force, slow ───────────────────────────────────────────────────────────
  page = await browser.newPage({ viewport: { width: 1120, height: 800 } });
  await openUpdates(page, "?harnessupdates=1&harnessrestart=1&restartslow=1");
  const p2 = page.locator("[data-testid=harness-restart-claude]");
  await p2.waitFor({ timeout: 8_000 });
  const f2 = p2.locator("[data-testid=restart-force-claude]");
  await f2.click();
  await f2.click();
  await page.waitForFunction(() => document.querySelector("[data-testid=restart-force-claude]")?.textContent === "Restarting…");
  ok(await p2.locator("[data-testid=restart-inactive-claude]").isDisabled(), `${engine}: a slow restart disables both buttons while it runs`);
  await p2.locator("[data-testid=restart-results-claude]").waitFor({ timeout: 8_000 });
  const r2 = await p2.locator("[data-testid=restart-results-claude]").textContent();
  ok(r2.includes("4 restarted · 1 skipped"), `${engine}: force restarts busy and waiting sessions too`);
  await page.waitForFunction(() => !document.querySelector("[data-testid=restart-force-claude]") && !document.querySelector("[data-testid=restart-inactive-claude]"));
  ok(true, `${engine}: with nothing restartable left, the buttons go`);

  // An expired arm does not fire: wait it out, then one click only arms.
  await page.close();
  page = await browser.newPage({ viewport: { width: 1120, height: 800 } });
  await openUpdates(page, "?harnessupdates=1&harnessrestart=1");
  const p3 = page.locator("[data-testid=harness-restart-claude]");
  await p3.waitFor({ timeout: 8_000 });
  const i3 = p3.locator("[data-testid=restart-inactive-claude]");
  await i3.click();
  await page.waitForTimeout(4_300);
  ok((await i3.textContent()) === "Restart 2 inactive", `${engine}: the arm expires`);
  await i3.click();
  ok((await i3.textContent()) === "Confirm restart" && await p3.locator("[data-testid=restart-results-claude]").count() === 0,
    `${engine}: after expiry one click only arms again`);
  await page.close();

  // ── nothing older: nothing shown ──────────────────────────────────────────
  page = await browser.newPage({ viewport: { width: 1120, height: 800 } });
  await openUpdates(page, "?harnessupdates=1");
  await page.waitForTimeout(500);
  ok(await page.locator("[data-testid=harness-restart-claude]").count() === 0, `${engine}: no older sessions, no panel`);
  await page.close();
  await browser.close();
}

if (failures) { console.error(`${failures} harness restart browser failure(s)`); process.exit(1); }
console.log("harness-restart-browser: all passed");
