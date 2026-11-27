// Run against VITE_MOCK=1 vite --port 1478 --strictPort --force.
import assert from "node:assert/strict";

const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const base = process.env.QUIET_CHROME_URL ?? "http://127.0.0.1:1478";
const stateKey = "wt-mock-ui-state";
const hittable = (locator) => locator.evaluate((el) => {
  const r = el.getBoundingClientRect();
  return el.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
});
let failed = false;

for (const [name, engine] of Object.entries({ chromium, webkit })) {
  const browser = await engine.launch();
  try {
    const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
    page.setDefaultTimeout(4500);
    await page.addInitScript((key) => {
      if (!sessionStorage.getItem(key)) {
        sessionStorage.setItem(key, JSON.stringify({ nav_pinned: true, nav_view: "projects", settings_rev: 5 }));
      }
    }, stateKey);
    await page.goto(`${base}/?strays=2&activity=0`);
    const toggle = page.getByRole("button", { name: "Show all", exact: true });
    await toggle.waitFor();
    assert.equal(await toggle.getAttribute("aria-pressed"), "false", "existing settings default to quiet");
    await toggle.click();
    assert.equal(await toggle.getAttribute("aria-pressed"), "true");
    await page.waitForFunction((key) => JSON.parse(sessionStorage.getItem(key)).quiet_chrome === false, stateKey);
    await page.reload();
    await toggle.waitFor();
    await page.waitForFunction(() => document.querySelector('[data-track="nav.show_all"]')?.getAttribute("aria-pressed") === "true");
    await toggle.focus();
    await page.keyboard.press("Space");
    assert.equal(await toggle.getAttribute("aria-pressed"), "false");
    await page.waitForFunction((key) => JSON.parse(sessionStorage.getItem(key)).quiet_chrome === true, stateKey);
    console.log(`ok   ${name} Show all starts off, works by pointer and keyboard, and persists`);

    for (const [label, action] of [["2 outside .worktrees/", "Adopt…"], ["Not configured", "Review…"]]) {
      const chip = page.locator(".project-notice").filter({ hasText: label });
      const summary = chip.locator("summary");
      const button = chip.getByRole("button", { name: action });
      await summary.waitFor();
      assert.equal(await button.isVisible(), false, `${label} starts folded`);
      await summary.click();
      await button.waitFor();
      await summary.click();
      assert.equal(await button.isVisible(), false, `${label} folds again`);
    }
    console.log(`ok   ${name} both warning chips disclose their full cards`);

    const row = page.locator('li.row[data-slug="billing-refactor"]').first();
    const branch = row.locator(".row-branch");
    const age = row.locator(".row-age");
    await row.scrollIntoViewIfNeeded();
    await page.mouse.move(700, 50);
    assert.equal(await branch.isVisible(), false);
    assert.equal(await hittable(branch), false);
    const before = await row.boundingBox();
    await row.hover();
    assert.equal(await hittable(branch), true);
    assert.equal(await age.isVisible(), true);
    assert.deepEqual(await row.boundingBox(), before);
    await page.mouse.move(700, 50);
    await row.focus();
    assert.equal(await hittable(branch), true);
    await row.click();
    await page.mouse.move(700, 50);
    await row.evaluate((el) => el.blur());
    assert.equal(await hittable(branch), true);
    assert.equal(await age.isVisible(), true);
    const waiting = page.locator('li.row[data-slug="feat-redesign"] .status-dot.waiting');
    await waiting.scrollIntoViewIfNeeded();
    assert.equal(await hittable(waiting), true, "an unselected waiting row keeps its dot");
    console.log(`ok   ${name} row details reveal without movement on hover, focus, and selection`);
  } catch (error) {
    failed = true;
    console.error(`FAIL ${name}`, error);
  } finally {
    await browser.close();
  }
}
if (failed) process.exitCode = 1;
