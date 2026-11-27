// Run against VITE_MOCK=1 vite --port 1478 --strictPort --force.
import assert from "node:assert/strict";
import { join } from "node:path";

const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const base = process.env.QUIET_CHROME_URL ?? "http://127.0.0.1:1478";
const baseline = process.env.QUIET_CHROME_BASELINE;
const shots = process.env.QUIET_CHROME_SHOTS;
const stateKey = "wt-mock-ui-state";
const query = "?strays=2&activity=0";
const billing = 'li.row[data-slug="billing-refactor"]';
const hittable = (locator) => locator.evaluate((el) => {
  const r = el.getBoundingClientRect();
  return el.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
});
const painted = (locator) => locator.evaluate((el) => {
  for (let node = el; node; node = node.parentElement) {
    const style = getComputedStyle(node);
    if (style.opacity === "0" || style.visibility === "hidden") return false;
  }
  return true;
});

async function boot(browser, url, { width = 300, touch = false } = {}) {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 }, hasTouch: touch, isMobile: touch });
  page.setDefaultTimeout(6000);
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.addInitScript(({ key, settings }) => {
    if (!sessionStorage.getItem(key)) sessionStorage.setItem(key, JSON.stringify(settings));
  }, {
    key: stateKey,
    settings: {
      nav_pinned: true, nav_view: "projects", nav_width: width, settings_rev: 5, beta_group_chat: true,
    },
  });
  await page.goto(url + query);
  return page;
}

const snapshot = (page) => page.evaluate(() => {
  const controls = [
    ".nav-head .grip", ".nav-head .attn", ".nav-head .menu-wrap button", ".nav-head [title='focus search']",
    ".project-h .pcount", ".project-h .mini", ".project .init-banner", ".project-notice",
    ".row[data-slug] .g", ".row[data-slug] .row-branch", ".row[data-slug] .row-enter", ".row[data-slug] .row-age",
    ".topbar .slug", ".topbar .branch", ".topbar .live-badge", ".topbar .pr-chip", ".topbar .status-cluster .s",
    ".topbar .remote-link", ".topbar [data-track='place.pin']", ".topbar [title='more actions']",
  ];
  function visible(el) {
    if (!el.getClientRects().length) return false;
    for (let node = el; node; node = node.parentElement) {
      const style = getComputedStyle(node);
      if (style.display === "none" || style.opacity === "0" || style.visibility === "hidden") return false;
    }
    return true;
  }
  return Object.fromEntries(controls.map((selector) => [selector, [...document.querySelectorAll(selector)].map(visible)]));
});

const themes = ["tokyo-night", "tokyo-day", "catppuccin-mocha", "catppuccin-latte", "nord", "gruvbox-dark"];
async function checkChips(page) {
  const ratios = await page.locator(".project-notice summary").evaluateAll((chips, themes) => {
    const root = document.documentElement;
    const original = root.dataset.theme;
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = 1;
    const ctx = canvas.getContext("2d");
    const luminance = () => {
      const rgb = [...ctx.getImageData(0, 0, 1, 1).data].slice(0, 3);
      const linear = rgb.map((v) => v / 255).map((v) => v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
      return linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722;
    };
    const fill = (color) => { ctx.fillStyle = color; ctx.fillRect(0, 0, 1, 1); };
    const ratios = Object.fromEntries(themes.map((theme) => {
      root.dataset.theme = theme;
      return [theme, Math.min(...chips.map((el) => {
        const layers = [];
        for (let node = el; node; node = node.parentElement) layers.unshift(getComputedStyle(node).backgroundColor);
        fill("#fff");
        layers.forEach(fill);
        const background = luminance();
        fill(getComputedStyle(el).color);
        const foreground = luminance();
        return (Math.max(background, foreground) + 0.05) / (Math.min(background, foreground) + 0.05);
      }))];
    }));
    if (original === undefined) delete root.dataset.theme;
    else root.dataset.theme = original;
    return ratios;
  }, themes);
  for (const [theme, ratio] of Object.entries(ratios)) assert.ok(ratio >= 4.5, `${theme} chip contrast is ${ratio.toFixed(2)}:1`);
  return Object.entries(ratios).map(([theme, ratio]) => `${theme} ${ratio.toFixed(1)}:1`).join(", ");
}

async function checkWidths(browser, name) {
  for (const width of [300, 220]) {
    const page = await boot(browser, base, { width });
    const row = page.locator(billing).first();
    const hoverRow = page.locator('li.row[data-slug="catalog-import"]').first();
    const header = page.locator(".topbar");
    const chip = page.locator(".project-notice summary").first();
    const shot = async (part, target = page) => {
      if (shots && name === "chromium") await target.screenshot({ path: join(shots, `${width}-${part}.png`) });
    };
    await row.click();
    await header.getByTestId("topbar-remote").waitFor();
    await chip.waitFor();
    await page.mouse.move(700, 500);
    await page.evaluate(() => document.activeElement?.blur());
    assert.equal(Math.round((await page.locator(".nav").boundingBox()).width), width);
    assert.ok((await row.locator(".row-name").boundingBox()).width >= 100, `${width}px leaves room for the lane name`);
    assert.ok((await page.locator(".project-h .pname").first().boundingBox()).width >= 100, `${width}px leaves room for the project name`);
    assert.ok(await hittable(row.locator(".row-name")));
    assert.ok(await hittable(chip));
    if (width === 300) console.log(`ok   ${name} chip contrast ${await checkChips(page)}`);
    await shot("place-rest");
    await shot("header-rest", header);
    await hoverRow.hover();
    assert.ok(await hittable(hoverRow.locator(".row-branch")));
    await shot("row-hover");
    await header.hover();
    assert.ok(await hittable(header.getByTestId("topbar-remote")));
    await shot("header-hover", header);
    await page.mouse.move(700, 500);
    await shot("chips", page.locator(".nav"));
    await page.getByRole("button", { name: "Show all", exact: true }).click();
    await page.mouse.move(700, 500);
    await page.evaluate(() => document.activeElement?.blur());
    await shot("show-all");

    if (baseline) {
      const previous = await boot(browser, baseline, { width });
      await previous.locator(billing).first().click();
      await previous.getByTestId("topbar-remote").waitFor();
      await previous.locator(".project .init-banner").last().waitFor();
      await previous.mouse.move(700, 500);
      await previous.evaluate(() => document.activeElement?.blur());
      assert.deepEqual(await snapshot(page), await snapshot(previous), `Show all matches the pre-change chrome at ${width}px`);
      await page.locator(".project-h").first().hover();
      await previous.locator(".project-h").first().hover();
      await Promise.all([page, previous].map((p) => p.locator(".project-h").first().evaluate((el) => {
        el.getAnimations({ subtree: true }).forEach((a) => { if (a.effect.getTiming().iterations !== Infinity) a.finish(); });
      })));
      assert.deepEqual(await snapshot(page), await snapshot(previous), `Show all matches pre-change project hover at ${width}px`);
      await previous.close();
    }
    await page.close();
  }
  const touch = await boot(browser, base, { width: 220, touch: true });
  const touchRow = touch.locator(billing).first();
  await touchRow.tap();
  await touch.getByTestId("topbar-remote").waitFor();
  assert.ok(await hittable(touch.locator(".nav-head").getByTitle("sort places", { exact: true })));
  assert.ok(await hittable(touch.getByTestId("topbar-remote")));
  await touch.locator(".project-notice summary").first().tap();
  assert.equal(await touch.locator(".project-notice[open]").count(), 1);
  await touch.close();
  console.log(`ok   ${name} 300px, 220px and touch reachability${baseline ? ", with Show all matched to the base" : ""}`);
}
let failed = false;

for (const [name, engine] of Object.entries({ chromium, webkit })) {
  const browser = await engine.launch();
  try {
    const page = await boot(browser, base);
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
    await page.locator('[data-track="settings"]').click();
    await page.locator('.settings-cat[data-cat="layout"]').click();
    const choice = page.getByRole("checkbox", { name: "Show all", exact: true });
    assert.equal(await choice.isChecked(), false);
    await choice.check();
    assert.equal(await toggle.getAttribute("aria-pressed"), "true");
    await choice.uncheck();
    await page.keyboard.press("Escape");
    assert.equal(await toggle.getAttribute("aria-pressed"), "false");
    console.log(`ok   ${name} Show all starts off, works by pointer and keyboard, persists, and shares the Layout setting`);

    for (const [label, action] of [["2 outside .worktrees/", "Adopt…"], ["Not configured", "Review…"]]) {
      const chip = page.locator(".project-notice").filter({ hasText: label });
      const summary = chip.locator("summary");
      const button = chip.getByRole("button", { name: action });
      await summary.waitFor();
      assert.equal(await button.isVisible(), false, `${label} starts folded`);
      const projectName = chip.locator("xpath=..").locator(".project-h .pname");
      await projectName.click();
      assert.equal(await hittable(summary), true, `${label} is visible even with the project collapsed`);
      await summary.focus();
      await page.keyboard.press("Space");
      await button.waitFor();
      await summary.click();
      assert.equal(await button.isVisible(), false, `${label} folds again`);
      await projectName.click();
    }
    console.log(`ok   ${name} both warning chips disclose their full cards`);

    const row = page.locator(billing).first();
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
    const unread = page.locator('li.row[data-slug="messaging"] .status-dot.unread');
    await page.evaluate(() => {
      const row = document.querySelector('li.row[data-slug="messaging"]');
      const root = row.closest("[data-project-root]").dataset.projectRoot;
      window.__mock.finishTask(root + "/.worktrees/messaging");
    });
    await unread.scrollIntoViewIfNeeded();
    assert.equal(await hittable(unread), true, "an unselected finished row keeps its unread ring");
    console.log(`ok   ${name} row details reveal without movement on hover, focus, and selection`);

    const head = page.locator(".nav-head");
    const sort = head.getByTitle("sort places", { exact: true });
    const grip = head.locator('.grip[data-grip="places"]');
    const project = row.locator("xpath=ancestor::div[contains(@class,'project')][1]").locator(".project-h");
    const sync = project.getByTitle("Sync", { exact: true });
    for (const [parent, control] of [[head, sort], [head, grip], [project, sync]]) {
      await page.mouse.move(700, 50);
      await page.evaluate(() => document.activeElement?.blur());
      assert.equal(await hittable(control), false);
      await parent.hover();
      await control.evaluate((el) => el.getAnimations().forEach((a) => a.finish()));
      assert.equal(await hittable(control), true);
      await page.mouse.move(700, 50);
      await control.focus();
      assert.equal(await hittable(control), true);
    }
    console.log(`ok   ${name} nav and project controls reveal and are hit-testable on hover and focus`);

    const header = page.locator(".topbar");
    const remote = header.getByTestId("topbar-remote");
    const device = header.getByTestId("launched-by");
    const status = header.locator(".status-cluster .ab");
    await remote.waitFor();
    await page.mouse.move(700, 500);
    await page.evaluate(() => document.activeElement?.blur());
    assert.equal(await header.getByTitle("more actions").count(), 0);
    assert.equal(await header.locator('[data-track="place.pin"]').count(), 0);
    for (const detail of [remote, device, status]) {
      assert.equal(await painted(detail), false);
      assert.equal(await hittable(detail), false);
    }
    assert.equal(await painted(header.locator(".live-badge").first()), true);
    assert.equal(await painted(header.getByTestId("topbar-pr")), true);
    assert.equal(await page.getByText("Codex needs your answer", { exact: true }).isVisible(), true);
    const placeName = await header.locator(".slug").boundingBox();
    await header.hover();
    for (const detail of [remote, device, status]) assert.equal(await painted(detail), true);
    assert.deepEqual(await header.locator(".slug").boundingBox(), placeName);
    await page.mouse.move(700, 500);
    await remote.focus();
    assert.equal(await hittable(remote), true);
    assert.equal(await painted(device), true);

    await row.focus();
    await page.keyboard.press("Shift+F10");
    await page.getByTestId("ctx-switch-agent").click();
    await page.getByRole("dialog", { name: "Switch agent" }).waitFor();
    await page.keyboard.press("Escape");
    const other = page.locator('li.row[data-slug="catalog-import"]').first();
    await other.click({ button: "right" });
    await page.getByTestId("ctx-switch-agent").click();
    assert.match(await page.getByRole("dialog", { name: "Switch agent" }).innerText(), /catalog-import/);
    assert.equal(await header.locator(".slug").innerText(), "billing-refactor");
    await page.keyboard.press("Escape");
    await row.click({ button: "right" });
    await page.getByTestId("ctx-copy-path").click();
    await page.waitForFunction(() => window.__mock.copies().at(-1)?.endsWith("/billing-refactor"));
    await row.click({ button: "right" });
    await page.getByTestId("ctx-status").click();
    await page.locator(".status-sheet").getByText("Status · billing-refactor").waitFor();
    await page.keyboard.press("Escape");
    await row.click({ button: "right" });
    await page.getByTestId("ctx-share").click();
    await page.getByTestId("share-dialog").waitFor();
    await page.keyboard.press("Escape");
    await row.click({ button: "right" });
    await page.getByTestId("ctx-remove").click();
    await page.getByRole("dialog", { name: /Remove worktree/ }).waitFor();
    await page.keyboard.press("Escape");
    await row.click({ button: "right" });
    await page.getByTestId("ctx-rename").click();
    await page.getByTestId("row-name-input").fill("Billing refactor");
    await page.keyboard.press("Enter");
    await row.click({ button: "right" });
    await page.getByTestId("ctx-clear-name").click();
    await row.getByText("billing-refactor", { exact: true }).waitFor();

    await page.keyboard.press("Meta+k");
    await page.getByRole("dialog", { name: "Quick switch" }).locator("input").fill("copy path");
    await page.locator('[data-key="place-action|copy-path"]').click();
    await page.waitForFunction(() => window.__mock.copies().filter((p) => p.endsWith("/billing-refactor")).length === 2);

    await toggle.click();
    await page.mouse.move(700, 500);
    await page.evaluate(() => document.activeElement?.blur());
    for (const detail of [remote, device, status]) assert.equal(await painted(detail), true);
    await header.getByTitle("more actions").waitFor();
    await header.locator('[data-track="place.pin"]').waitFor();
    assert.equal(await page.locator(".project-notice").count(), 0);
    assert.equal(await page.locator(".project .init-banner").count(), 2);
    await row.click({ button: "right" });
    await page.locator('.ctxmenu [data-track="nav.row.pin"]').click();
    await row.getByText("★ billing-refactor", { exact: true }).waitFor();
    await other.click({ button: "right" });
    await page.locator('.ctxmenu [data-track="nav.row.close_session"]').click();
    await other.locator(".g-agent").waitFor({ state: "detached" });
    console.log(`ok   ${name} place header reveals and row menu actions work, and Show all restores the header and cards`);
    await page.close();
    await checkWidths(browser, name);
  } catch (error) {
    failed = true;
    console.error(`FAIL ${name}`, error);
  } finally {
    await browser.close();
  }
}
if (failed) process.exitCode = 1;
