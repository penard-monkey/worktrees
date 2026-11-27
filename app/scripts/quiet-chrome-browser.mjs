// Run against VITE_MOCK=1 vite --port 1478 --strictPort --force.
import assert from "node:assert/strict";

const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const base = process.env.QUIET_CHROME_URL ?? "http://127.0.0.1:1478";
const stateKey = "wt-mock-ui-state";
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
    await page.goto(base);
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
  } catch (error) {
    failed = true;
    console.error(`FAIL ${name}`, error);
  } finally {
    await browser.close();
  }
}
if (failed) process.exitCode = 1;
