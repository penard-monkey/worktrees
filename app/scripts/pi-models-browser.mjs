// Optional browser gate for Settings → pi → Models (docs/proposals/
// pi-models-config.md), in Chromium AND WebKit, against the mock:
//
//   VITE_MOCK=1 vite --port 1447 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=<dir>/node_modules/playwright/index.mjs \
//     PIMODELS_MOCK_URL=http://localhost:1447 node app/scripts/pi-models-browser.mjs
// PIMODELS_SCREENSHOTS=<dir> saves the page per engine.
//
// What it holds: pi's hosts and the models they serve undeclared are drawn;
// adding a host, overriding one of pi's, declaring a served model and removing
// one (two clicks) all land and the default-model picker follows without a
// reload; a dead host's dot and pi's own load error are said; every button is
// REACHABLE (elementFromPoint at its centre); nothing overflows at 480px.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const BASE = process.env.PIMODELS_MOCK_URL ?? "http://localhost:1447";
const SHOTS = process.env.PIMODELS_SCREENSHOTS;

let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };

async function openPi(page, query = "") {
  await page.goto(`${BASE}/${query}`);
  await page.waitForSelector("[data-track=settings]");
  await page.click("[data-track=settings]");
  await page.waitForSelector(".settings-body .page-title");
  await page.click(".settings-cat[data-cat=pi]");
  await page.waitForSelector(".settings-body[data-page=pi] [data-testid=pi-host]");
  await page.waitForTimeout(200);
}
const hosts = (page) => page.$$eval("[data-testid=pi-host]", (els) => els.map((e) => ({
  origin: e.dataset.origin,
  name: e.querySelector(".pi-host-head b")?.textContent,
  tag: e.querySelector(".pi-host-tag")?.textContent,
  models: [...e.querySelectorAll("[data-testid=pi-host-model] code")].map((c) => c.textContent),
  undeclared: [...e.querySelectorAll("[data-testid=pi-served-undeclared] code")].map((c) => c.textContent),
  unusable: [...e.querySelectorAll("[data-testid=pi-served-unusable]")].map((r) => [r.querySelector("code")?.textContent, !!r.querySelector("button")]),
  bad: !!e.querySelector(".pi-host-head .pi-dot.bad"),
})));
const host = async (page, origin, name) => (await hosts(page)).find((h) => h.origin === origin && h.name === name);
const inHost = (origin, name, sel) => `[data-testid=pi-host][data-origin=${origin}]:has(.pi-host-head b:text-is("${name}")) ${sel}`;
const options = (page) => page.$$eval("[data-testid=pi-default-model] option", (os) => os.map((o) => o.value));
const unreachable = (page) => page.evaluate(() => [...document.querySelectorAll("[data-testid=pi-models] button")].filter((b) => {
  b.scrollIntoView({ block: "center" });
  const r = b.getBoundingClientRect();
  const hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
  return !(hit === b || b.contains(hit));
}).map((b) => b.textContent));

for (const [name, engine] of [["chromium", chromium], ["webkit", webkit]]) {
  const browser = await engine.launch();
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  await openPi(page, "?pihosts=undeclared");
  let lms = await host(page, "pi", "lm-studio");
  ok(lms?.tag === "pi's config" && lms.models.join() === "qwen3.6-27b", `[${name}] pi's lm-studio is drawn read-only with its model`);
  ok(lms?.undeclared.join() === "qwen3-coder-next,gemma-4-12b", `[${name}] …and the two models it serves undeclared`);
  ok(await page.locator(inHost("pi", "lm-studio", "[data-testid=pi-declare]")).count() === 0, `[${name}] …with no Declare on pi's own host`);
  ok(JSON.stringify(lms?.unusable) === JSON.stringify([["qwen3-coder-next@q4_k_m", false]]), `[${name}] a served id pi's ids cannot carry is listed with no button`);
  ok((await host(page, "pi", "lm-dead"))?.bad === true, `[${name}] a dead host's dot is the bad one`);
  ok(await page.locator("[data-testid=pi-models] .hint", { hasText: "exist only for pi started by worktrees" }).count() === 1,
    `[${name}] the page says the hosts are worktrees' only, and the resume trap`);

  // Add a host.
  await page.fill("[aria-label='host name']", "box");
  await page.fill("[aria-label='base URL']", "http://127.0.0.1:1234/v1/");
  await page.click("[data-testid=pi-host-add]");
  await page.waitForSelector(inHost("worktrees", "box", ".pi-host-tag"));
  const box = await host(page, "worktrees", "box");
  ok(box?.tag === "worktrees" && box.models.length === 0, `[${name}] an added host is worktrees', with no models yet`);

  // Override pi's lm-studio, declare a served model, and the picker follows.
  await page.click(inHost("pi", "lm-studio", "[data-testid=pi-host-override]"));
  await page.waitForSelector(inHost("worktrees", "lm-studio", "[data-testid=pi-declare]"));
  lms = await host(page, "worktrees", "lm-studio");
  ok(lms?.tag === "overridden in worktrees" && lms.models.join() === "qwen3.6-27b", `[${name}] an override starts as a copy of pi's models`);
  ok((await host(page, "pi", "lm-studio"))?.tag === "pi's config · replaced by worktrees", `[${name}] …and pi's own row says it is replaced`);
  await page.click(inHost("worktrees", "lm-studio", "[data-testid=pi-served-undeclared]:has(code:text-is('qwen3-coder-next')) [data-testid=pi-declare]"));
  await page.waitForFunction(() => [...document.querySelectorAll("[data-testid=pi-default-model] option")].some((o) => o.value === "lm-studio/qwen3-coder-next"), null, { timeout: 3000 }).catch(() => {});
  lms = await host(page, "worktrees", "lm-studio");
  ok(lms?.models.join() === "qwen3.6-27b,qwen3-coder-next" && lms.undeclared.join() === "gemma-4-12b", `[${name}] Declare moves a served model into the host`);
  ok(JSON.stringify(lms?.unusable) === JSON.stringify([["qwen3-coder-next@q4_k_m", false]]), `[${name}] …and on our override the unusable id still has no Declare`);
  ok((await options(page)).includes("lm-studio/qwen3-coder-next"), `[${name}] …and the default-model picker offers it, no reload`);

  ok((await unreachable(page)).length === 0, `[${name}] every button is reachable: ${JSON.stringify(await unreachable(page))}`);
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/pi-models-${name}.png`, fullPage: true });

  // Remove a model: two clicks, the first only arms.
  const rm = inHost("worktrees", "lm-studio", "[data-testid=pi-host-model]:has(code:text-is('qwen3-coder-next')) [data-testid=pi-model-remove]");
  await page.click(rm);
  ok((await host(page, "worktrees", "lm-studio"))?.models.length === 2, `[${name}] one click on Remove only arms`);
  await page.click(rm);
  await page.waitForTimeout(300);
  ok((await host(page, "worktrees", "lm-studio"))?.models.join() === "qwen3.6-27b", `[${name}] the second removes it`);

  // Remove the override: pi's own host is back.
  const ro = inHost("worktrees", "lm-studio", "[data-testid=pi-host-remove]");
  ok((await page.textContent(ro))?.trim() === "Remove override", `[${name}] an override's remove says "Remove override"`);
  await page.click(ro);
  await page.click(ro);
  await page.waitForTimeout(300);
  ok(!(await host(page, "worktrees", "lm-studio")) && (await host(page, "pi", "lm-studio"))?.tag === "pi's config", `[${name}] Remove override gives the host back to pi's config`);

  // A refused add is said, not swallowed.
  await page.fill("[aria-label='host name']", "-bad");
  await page.click("[data-testid=pi-host-add]");
  await page.waitForTimeout(300);
  ok(!(await host(page, "worktrees", "-bad")), `[${name}] a bad id is refused`);

  // Narrow window: nothing sticks out of the settings body.
  await page.setViewportSize({ width: 480, height: 900 });
  await page.waitForTimeout(200);
  const over = await page.evaluate(() => {
    const body = document.querySelector(".settings-body").getBoundingClientRect();
    return [...document.querySelectorAll("[data-testid=pi-models] *")].filter((e) => e.getBoundingClientRect().right > body.right + 1).map((e) => e.className || e.tagName).slice(0, 5);
  });
  ok(over.length === 0, `[${name}] nothing overflows at 480px: ${JSON.stringify(over)}`);
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/pi-models-${name}-480.png`, fullPage: true });

  // pi refusing its own models.json is its own state.
  await page.setViewportSize({ width: 1280, height: 900 });
  await openPi(page, "?pihosts=broken");
  ok(await page.locator("[data-testid=pi-load-error]", { hasText: "must not have fewer than 1 characters" }).count() === 1, `[${name}] pi's load error is said, with pi's reason`);
  await browser.close();
}

if (failed) { console.log(`pi-models-browser: ${failed} failure(s)`); process.exit(1); }
console.log("pi-models-browser: all good");
