// Browser proof for the shared dock. IDEAS_SCREENSHOTS saves the review images.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const base = process.env.IDEAS_MOCK_URL ?? "http://localhost:1466";
const shots = process.env.IDEAS_SCREENSHOTS;
const choice = (page, name) => page.locator(`[data-workspace-panel=${name}]`);
const dock = (page) => page.getByTestId("ws-dock");
const chat = (page) => page.getByTestId("ws-chat-col");
const width = async (locator) => (await locator.boundingBox()).width;
const panel = (page, name) => page.locator(`.ws-dock[data-panel=${name}]:visible`).waitFor();
const check = (ok, words) => { if (!ok) throw new Error(words); };

async function boot(browser, nav) {
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  page.setDefaultTimeout(6500);
  await page.addInitScript((navWidth) => sessionStorage.setItem("wt-mock-ui-state", JSON.stringify({
    beta_ideas: true, beta_workspace_chats: true, nav_pinned: true, nav_width: navWidth,
    nav_view: "workspaces", restore_last: false, places_side: "left", dock_side: "right", settings_rev: 5,
  })), nav);
  await page.goto(`${base}/?ideas=seed&workspaces=1&folding=0&proposed=1`);
  await page.getByTestId("ws-name").filter({ hasText: "Everything" }).first().click();
  await page.getByTestId("ws-frame-name").waitFor();
  await page.locator(".main [data-testid=ws-overview]").waitFor();
  const rail = page.locator(".rail-right button[data-dock-tab]");
  const buttons = await rail.evaluateAll((items) => items.map((button) => ({
    name: button.dataset.dockTab, disabled: button.disabled,
    box: button.getBoundingClientRect().toJSON(),
  })));
  check(buttons.map((button) => button.name).join() === "overview,memory,plan,ideas,chats,terminal", "workspace rail has all six choices in order");
  check(buttons.every(({ disabled, box }) => !disabled && box.width >= 30 && box.height >= 30), "workspace rail choices are live and have usable hit areas");
  await page.getByTestId("ws-ov-link-idea").click();
  await page.locator("[data-testid=ws-ov-idea-pick-row][data-doc='01JC8Z4K3M9QX2T7VB00000901']").click();
  await page.getByTestId("ws-ov-idea").waitFor();
  return page;
}

const contents = {
  overview: ".wsf-ov",
  memory: "[data-testid=main-doc]",
  plan: "[data-testid=main-doc]",
  ideas: "[data-testid=main-doc]",
  chats: "[data-testid=ws-chats-tab]",
  terminal: ".termtabs",
};

async function prove(page, engine, nav) {
  const snap = async (key) => { if (shots) await page.screenshot({ path: `${shots}/${engine}-nav${nav}-${key}.png` }); };
  await choice(page, "overview").click();
  await panel(page, "overview");
  check(await page.locator(".main [data-testid=ws-overview]").isVisible(), "overview remains in the main pane with no chat");
  await page.getByTestId("ws-chat-row").filter({ hasText: "Who can see what" }).first().click();
  await chat(page).waitFor();
  await dock(page).locator(contents.overview).waitFor();
  const start = await width(dock(page));
  const grip = await dock(page).locator(".dock-resizer").boundingBox();
  await page.mouse.move(grip.x + 1, grip.y + 100);
  await page.mouse.down();
  await page.mouse.move(grip.x - 39, grip.y + 100);
  await page.mouse.up();
  const shared = await width(dock(page));
  check(Math.abs(shared - start - 40) < 2, "the shared dock resizes through its existing grip");

  const names = Object.keys(contents);
  for (const name of names) {
    if (await choice(page, name).getAttribute("aria-pressed") !== "true") await choice(page, name).click();
    await panel(page, name);
    await dock(page).locator(contents[name]).first().waitFor();
    check(await chat(page).isVisible() && Math.abs(await width(dock(page)) - shared) < 2, `${name} stays beside the chat at the shared width`);
    await snap(name);
    await page.waitForTimeout(650); // Beyond the navigation ring's coalescing window.
  }
  await page.getByTestId("nav-back").filter({ visible: true }).first().click();
  await panel(page, "chats");
  await page.getByTestId("nav-forward").filter({ visible: true }).first().click();
  await panel(page, "terminal");

  const wide = await width(chat(page));
  await page.keyboard.press("Meta+j");
  check(!await dock(page).isVisible() && await width(chat(page)) > wide, "⌘J hides the dock and gives the chat more room");
  await page.keyboard.press("Meta+j");
  await panel(page, "terminal");
  const shell = await dock(page).locator(".termtabs").elementHandle();
  await choice(page, "overview").click();
  await panel(page, "overview");
  await choice(page, "terminal").click();
  await panel(page, "terminal");
  check(await shell.evaluate((el) => el.isConnected && el === document.querySelector(".ws-dock .termtabs")), "switching panels keeps the workspace shell mounted");
  await page.getByTestId("ws-dock-close").click();
  check(!await dock(page).isVisible() && await width(chat(page)) > wide, "× hides the dock and gives the chat more room");

  await page.getByTestId("ws-chat-view-terminal").click();
  await page.getByTestId("ws-chat-term").waitFor();
  await page.evaluate(() => {
    const ws = document.querySelector("[data-testid=ws-frame]").dataset.ws;
    const conv = document.querySelector("[data-testid=ws-chat-col]").dataset.chat;
    window.__mock.wsFold(ws, conv, "folding");
  });
  await chat(page).getByRole("status").filter({ hasText: "Folding into memory" }).waitFor();
  await page.getByTestId("ws-chat-view-chat").click();
  check(!await page.getByTestId("ws-chat-term").count(), "the header switches between the chat and its terminal");
  check(!await page.getByTestId("ws-chat-readsb").count() && !await page.getByTestId("ws-chat-close").count(), "reads and Close are out of the header");
  await page.getByTestId("ws-chat-menu").click();
  check(await page.getByTestId("ws-chat-close").isVisible(), "Close is in the title menu");
  check(await page.getByTestId("ws-chat-no-mcp").isVisible() && await page.getByRole("menuitem", { name: "Set up worktrees tools" }).isVisible(), "tools status stays visible and the title menu has setup");
  await page.getByTestId("ws-chat-readsb").click();
  await page.getByTestId("ws-chat-reads").waitFor();
  await page.keyboard.press("Escape");

  await page.locator("[data-testid=ws-group] li.row[data-slug]").first().click();
  await page.locator(".rail-right [data-dock-tab=files]:not([disabled])").click();
  const laneDock = page.locator(".dock:not(.ws-dock):visible");
  await laneDock.waitFor();
  const laneChoices = await page.locator(".rail-right [data-dock-tab]").evaluateAll((buttons) => buttons.map((button) => button.dataset.dockTab));
  check(laneChoices.slice(0, 4).join() === "files,terminal,docs,plan" && !laneChoices.some((key) => ["overview", "memory", "ideas", "chats"].includes(key)), "lanes retain their rail choices");
  check(Math.abs(await width(laneDock) - shared) < 2, "an unconfigured lane inherits the last chosen dock width");
  await snap("lane-files");
}

let failures = 0;
for (const [name, engine] of Object.entries({ chromium, webkit })) {
  const browser = await engine.launch();
  try {
    for (const nav of [300, 220]) {
      const page = await boot(browser, nav);
      await prove(page, name, nav);
      await page.close();
      console.log(`ok ${name}: shared shell at a ${nav}px sidebar`);
    }
  } catch (error) {
    failures++;
    console.error(`FAIL ${name}: ${error.message}\n${error.stack}`);
  } finally {
    await browser.close();
  }
}
process.exitCode = failures ? 1 : 0;
